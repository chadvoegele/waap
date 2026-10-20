use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::backend::{RunHandle, RunOutcome, SteeringError};
use super::worktree::collapse_errors;

pub(super) mod client;
mod endpoint;
mod protocol;

use endpoint::prepare_parent;
pub(super) use endpoint::socket_path;
use protocol::{parse_request, rejection, response, Request, MAX_FRAME_BYTES};

const MAX_CONNECTIONS: usize = 32;
const MAX_PENDING: usize = 16;
const FRAME_TIMEOUT: Duration = Duration::from_secs(5);
pub(super) const ACK_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) struct ControlServer {
    listener: Option<UnixListener>,
    path: PathBuf,
    connections: BTreeMap<u64, Connection>,
    next_connection: u64,
    pending: VecDeque<Pending>,
    dispatch: Option<Dispatch>,
}

struct Pending {
    connection: u64,
    request: Request,
    deadline: Instant,
}

struct Dispatch {
    pending: Pending,
    responded: bool,
}

struct DispatchResult {
    result: Result<(), SteeringError>,
    completed_at: Instant,
    called_backend: bool,
}

struct Connection {
    stream: UnixStream,
    stage: Stage,
}

enum Stage {
    Reading {
        buffer: Vec<u8>,
        deadline: Instant,
    },
    Waiting,
    Replying {
        buffer: Vec<u8>,
        offset: usize,
        deadline: Instant,
    },
    Closed,
}

impl ControlServer {
    pub(super) fn bind(
        repository_root: &Path,
        agent_id: &str,
        runtime_dir: Option<&OsStr>,
    ) -> io::Result<Self> {
        let path = socket_path(repository_root, agent_id, runtime_dir)?;
        prepare_parent(&path, Path::new(runtime_dir.unwrap()))?;
        let listener = UnixListener::bind(&path).map_err(|error| {
            io::Error::new(error.kind(), format!("failed to bind control socket {}: {error}; existing endpoints are not automatically removed", path.display()))
        })?;
        let server = Self {
            listener: Some(listener),
            path,
            connections: BTreeMap::new(),
            next_connection: 0,
            pending: VecDeque::new(),
            dispatch: None,
        };
        fs::set_permissions(&server.path, fs::Permissions::from_mode(0o600))?;
        server.listener.as_ref().unwrap().set_nonblocking(true)?;
        Ok(server)
    }

    pub(super) fn wait(mut self, handle: &dyn RunHandle) -> io::Result<RunOutcome> {
        let completed = AtomicBool::new(false);
        thread::scope(|scope| {
            let (commands, command_rx) = mpsc::sync_channel::<(String, Instant)>(1);
            let (replies, reply_rx) = mpsc::channel();
            let completed = &completed;
            let worker = thread::Builder::new()
                .name("waap-steer".to_string())
                .spawn_scoped(scope, move || {
                    while let Ok((prompt, deadline)) = command_rx.recv() {
                        let rejection = if completed.load(Ordering::Acquire) {
                            Some(("agent_exited", "Agent exited before steering dispatch."))
                        } else if Instant::now() >= deadline {
                            Some(("timeout", "Steering request expired before dispatch."))
                        } else {
                            None
                        };
                        let called_backend = rejection.is_none();
                        let result = match rejection {
                            Some((code, message)) => Err(SteeringError::Rejected {
                                code,
                                message: message.to_string(),
                            }),
                            None => handle.steer(&prompt, deadline),
                        };
                        let _ = replies.send(DispatchResult {
                            result,
                            completed_at: Instant::now(),
                            called_backend,
                        });
                    }
                })?;
            let outcome = super::run::wait_for_run(handle, completed, |_| {
                self.service(&commands, &reply_rx, Instant::now());
            });
            self.receive_reply(&reply_rx, Instant::now());
            let cleanup = self.remove_socket();
            self.finish(Instant::now());
            drop(commands);
            if worker.join().is_err() {
                log::error!("steering dispatch worker panicked");
            }
            collapse_errors(outcome, cleanup)
        })
    }

    fn service(
        &mut self,
        commands: &SyncSender<(String, Instant)>,
        replies: &Receiver<DispatchResult>,
        now: Instant,
    ) {
        self.receive_reply(replies, now);
        self.expire_requests(now);
        self.accept_connections(now);
        self.read_requests(now);
        self.dispatch_next(commands, Instant::now());
        self.flush_replies(now);
    }

    fn accept_connections(&mut self, now: Instant) {
        // Bound work per tick even if a local caller continually opens connections.
        for _ in 0..MAX_CONNECTIONS {
            let stream = match self.listener.as_ref().unwrap().accept() {
                Ok((stream, _)) => stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    log::error!("control socket accept failed: {error}");
                    break;
                }
            };
            if self.connections.len() == MAX_CONNECTIONS {
                continue;
            }
            if stream.set_nonblocking(true).is_err() {
                continue;
            }
            self.connections.insert(
                self.next_connection,
                Connection {
                    stream,
                    stage: Stage::Reading {
                        buffer: Vec::new(),
                        deadline: now + FRAME_TIMEOUT,
                    },
                },
            );
            self.next_connection += 1;
        }
    }

    fn read_requests(&mut self, now: Instant) {
        for (&id, connection) in &mut self.connections {
            let Some((received_at, result)) = connection.read_frame(now) else {
                continue;
            };
            let request = match result {
                Ok(request) => request,
                Err(error) => {
                    connection.reply(error, now);
                    continue;
                }
            };
            let capacity = MAX_PENDING + usize::from(self.dispatch.is_none());
            if self.pending.len() >= capacity {
                connection.reply(
                    rejection(Some(&request.request_id), "busy", "Steering queue is full."),
                    now,
                );
                continue;
            }
            self.pending.push_back(Pending {
                connection: id,
                request,
                deadline: received_at + ACK_TIMEOUT,
            });
        }
    }

    fn receive_reply(&mut self, replies: &Receiver<DispatchResult>, now: Instant) {
        let reply = match replies.try_recv() {
            Ok(reply) => Some(reply),
            Err(TryRecvError::Disconnected) => None,
            Err(TryRecvError::Empty) => return,
        };
        let Some(dispatch) = self.dispatch.take() else {
            return;
        };
        if dispatch.responded {
            return;
        }
        let result = match reply {
            Some(reply)
                if !reply.called_backend || reply.completed_at < dispatch.pending.deadline =>
            {
                reply.result
            }
            _ => Err(acceptance_unknown()),
        };
        self.reply_to(
            &dispatch.pending,
            response(&dispatch.pending.request.request_id, result),
            now,
        );
    }

    fn expire_requests(&mut self, now: Instant) {
        let mut remaining = VecDeque::new();
        while let Some(pending) = self.pending.pop_front() {
            if now >= pending.deadline {
                self.reject_expired(&pending, now);
            } else {
                remaining.push_back(pending);
            }
        }
        self.pending = remaining;
        if let Some(dispatch) = &mut self.dispatch {
            if !dispatch.responded && now >= dispatch.pending.deadline {
                if let Some(connection) = self.connections.get_mut(&dispatch.pending.connection) {
                    connection.reply(
                        response(
                            &dispatch.pending.request.request_id,
                            Err(acceptance_unknown()),
                        ),
                        now,
                    );
                }
                dispatch.responded = true;
            }
        }
    }

    fn dispatch_next(&mut self, commands: &SyncSender<(String, Instant)>, now: Instant) {
        if self.dispatch.is_some() {
            return;
        }
        while let Some(pending) = self.pending.pop_front() {
            if now >= pending.deadline {
                self.reject_expired(&pending, now);
                continue;
            }
            if commands
                .try_send((pending.request.prompt.clone(), pending.deadline))
                .is_err()
            {
                self.reply_to(
                    &pending,
                    rejection(
                        Some(&pending.request.request_id),
                        "agent_exited",
                        "Steering dispatcher is unavailable.",
                    ),
                    now,
                );
                continue;
            }
            self.dispatch = Some(Dispatch {
                pending,
                responded: false,
            });
            break;
        }
    }

    fn reject_expired(&mut self, pending: &Pending, now: Instant) {
        self.reply_to(
            pending,
            rejection(
                Some(&pending.request.request_id),
                "timeout",
                "Steering request expired before dispatch.",
            ),
            now,
        );
    }

    fn reply_to(&mut self, pending: &Pending, value: Value, now: Instant) {
        if let Some(connection) = self.connections.get_mut(&pending.connection) {
            connection.reply(value, now);
        }
    }

    fn flush_replies(&mut self, now: Instant) {
        for connection in self.connections.values_mut() {
            connection.flush(now);
        }
        self.connections
            .retain(|_, connection| !matches!(connection.stage, Stage::Closed));
    }

    fn finish(&mut self, now: Instant) {
        self.expire_requests(now);
        while let Some(pending) = self.pending.pop_front() {
            self.reply_to(
                &pending,
                rejection(
                    Some(&pending.request.request_id),
                    "agent_exited",
                    "Agent exited before steering dispatch.",
                ),
                now,
            );
        }
        if let Some(dispatch) = self.dispatch.take() {
            if !dispatch.responded {
                self.reply_to(
                    &dispatch.pending,
                    response(
                        &dispatch.pending.request.request_id,
                        Err(acceptance_unknown()),
                    ),
                    now,
                );
            }
        }
        self.flush_replies(now);
        self.connections.clear();
    }

    fn remove_socket(&mut self) -> io::Result<()> {
        if self.listener.take().is_some() {
            fs::remove_file(&self.path)?;
        }
        Ok(())
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        if let Err(error) = self.remove_socket() {
            log::error!(
                "failed to remove control socket {}: {error}",
                self.path.display()
            );
        }
    }
}

fn acceptance_unknown() -> SteeringError {
    SteeringError::AcceptanceUnknown {
        message: "Steering acceptance is unknown: backend acknowledgement is unavailable."
            .to_string(),
    }
}

impl Connection {
    fn read_frame(&mut self, now: Instant) -> Option<(Instant, Result<Request, Value>)> {
        let Stage::Reading { buffer, deadline } = &mut self.stage else {
            return None;
        };
        if now >= *deadline {
            self.stage = Stage::Closed;
            return None;
        }
        let mut chunk = [0; 4096];
        loop {
            let capacity = chunk.len().min(MAX_FRAME_BYTES - buffer.len());
            match self.stream.read(&mut chunk[..capacity]) {
                Ok(0) => {
                    self.stage = Stage::Closed;
                    return None;
                }
                Ok(count) => {
                    if let Some(end) = chunk[..count].iter().position(|byte| *byte == b'\n') {
                        let received_at = Instant::now();
                        buffer.extend_from_slice(&chunk[..=end]);
                        let result = parse_request(buffer);
                        self.stage = Stage::Waiting;
                        return Some((received_at, result));
                    }
                    buffer.extend_from_slice(&chunk[..count]);
                    if buffer.len() == MAX_FRAME_BYTES {
                        self.stage = Stage::Waiting;
                        return Some((
                            Instant::now(),
                            Err(rejection(
                                None,
                                "request_too_large",
                                "Request frame exceeds 64 KiB including newline.",
                            )),
                        ));
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return None,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    self.stage = Stage::Closed;
                    return None;
                }
            }
        }
    }

    fn reply(&mut self, value: Value, now: Instant) {
        let mut buffer = serde_json::to_vec(&value).expect("JSON response");
        buffer.push(b'\n');
        self.stage = Stage::Replying {
            buffer,
            offset: 0,
            deadline: now + FRAME_TIMEOUT,
        };
    }

    fn flush(&mut self, now: Instant) {
        let Stage::Replying {
            buffer,
            offset,
            deadline,
        } = &mut self.stage
        else {
            return;
        };
        if now >= *deadline {
            self.stage = Stage::Closed;
            return;
        }
        while *offset < buffer.len() {
            match self.stream.write(&buffer[*offset..]) {
                Ok(0) => break,
                Ok(count) => *offset += count,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        self.stage = Stage::Closed;
    }
}

#[cfg(test)]
pub(super) mod tests;
