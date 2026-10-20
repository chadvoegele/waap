use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::backend::{RunHandle, RunOutcome, SteeringError};
use super::worktree::collapse_errors;
use crate::root::ControlRoot;

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
    pub(super) fn bind(control_root: &ControlRoot, agent_id: &str) -> io::Result<Self> {
        let path = socket_path(control_root, agent_id)?;
        prepare_parent(&path, control_root)?;
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
pub(super) mod tests {
    use std::io::{BufRead, BufReader};
    use std::sync::Mutex;

    use serde_json::json;
    use tempfile::TempDir;
    use uuid::Uuid;

    use super::*;
    use crate::agent::backend::{fake::FakeBackend, AgentSystemBackend, StartContext};
    use crate::root::find_control_root_in;
    use crate::test_git::init_repo_with_commit;

    struct Fixture {
        server: ControlServer,
        runtime: TempDir,
        repo: TempDir,
        commands: SyncSender<(String, Instant)>,
        command_rx: Receiver<(String, Instant)>,
        replies: mpsc::Sender<DispatchResult>,
        reply_rx: Receiver<DispatchResult>,
    }

    impl Fixture {
        fn new() -> Self {
            let runtime = tempfile::tempdir().unwrap();
            fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
            let repo = tempfile::tempdir().unwrap();
            init_repo_with_commit(repo.path());
            let server = ControlServer::bind(
                &find_control_root_in(repo.path(), Some(runtime.path().as_os_str())).unwrap(),
                "aa-1",
            )
            .unwrap();
            let (commands, command_rx) = mpsc::sync_channel(1);
            let (replies, reply_rx) = mpsc::channel();
            Self {
                server,
                runtime,
                repo,
                commands,
                command_rx,
                replies,
                reply_rx,
            }
        }

        fn tick(&mut self) {
            self.server
                .service(&self.commands, &self.reply_rx, Instant::now());
        }

        fn connect(&self, bytes: &[u8]) -> UnixStream {
            let mut client = UnixStream::connect(&self.server.path).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            client.write_all(bytes).unwrap();
            client
        }

        fn request(&self, index: u128, prompt: &str) -> UnixStream {
            self.connect(&frame(index, prompt))
        }

        fn acknowledge(&mut self, result: Result<(), SteeringError>) {
            self.replies
                .send(DispatchResult {
                    result,
                    completed_at: Instant::now(),
                    called_backend: true,
                })
                .unwrap();
            self.tick();
        }
    }

    fn request(index: u128, prompt: &str) -> Value {
        json!({"version": 1, "request_id": Uuid::from_u128(index).to_string(), "method": "steer", "prompt": prompt})
    }

    fn frame(index: u128, prompt: &str) -> Vec<u8> {
        let mut bytes = serde_json::to_vec(&request(index, prompt)).unwrap();
        bytes.push(b'\n');
        bytes
    }

    fn read_response(client: UnixStream) -> Value {
        let mut reader = BufReader::new(client);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(line.ends_with('\n'), "expected a complete response: {line}");
        let result = serde_json::from_str(&line).unwrap();
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => {}
            Err(error) if error.kind() == io::ErrorKind::ConnectionReset => {}
            result => panic!("expected connection closure after one response: {result:?}"),
        }
        result
    }

    fn assert_error(value: &Value, accepted: Value, code: &str) {
        assert_eq!(value["version"], 1);
        assert_eq!(value["accepted"], accepted);
        assert_eq!(value["error"]["code"], code);
    }

    #[test]
    fn socket_rejects_invalid_schemas_without_backend_dispatch_or_prompt_disclosure() {
        let mut fixture = Fixture::new();
        let secret = "private prompt";
        let valid = request(1, secret);
        let mut cases = vec![
            (b"not JSON\n".to_vec(), "invalid_request", false),
            (b"\xff\n".to_vec(), "invalid_request", false),
            (b"[]\n".to_vec(), "invalid_request", false),
        ];
        for (key, value, code, trusted_id) in [
            ("version", json!(2), "unsupported_version", true),
            ("version", json!("1"), "invalid_request", true),
            ("method", json!("start"), "unknown_method", true),
            ("method", json!(1), "invalid_request", true),
            ("request_id", json!("untrusted"), "invalid_request", false),
            ("request_id", Value::Null, "invalid_request", false),
            ("prompt", json!(" \n\t\u{2003}"), "invalid_request", true),
            ("prompt", json!(7), "invalid_request", true),
            ("threadId", json!("not allowed"), "invalid_request", true),
        ] {
            let mut value_to_send = valid.clone();
            value_to_send[key] = value;
            let mut bytes = serde_json::to_vec(&value_to_send).unwrap();
            bytes.push(b'\n');
            cases.push((bytes, code, trusted_id));
        }
        for key in ["version", "method", "prompt", "request_id"] {
            let mut value = valid.clone();
            value.as_object_mut().unwrap().remove(key);
            let mut bytes = serde_json::to_vec(&value).unwrap();
            bytes.push(b'\n');
            cases.push((bytes, "invalid_request", key != "request_id"));
        }
        for (bytes, code, trusted_id) in cases {
            let client = fixture.connect(&bytes);
            fixture.tick();
            let value = read_response(client);
            assert_error(&value, json!(false), code);
            assert_eq!(
                value["request_id"],
                if trusted_id {
                    valid["request_id"].clone()
                } else {
                    Value::Null
                }
            );
            assert!(!value.to_string().contains(secret));
            assert!(fixture.command_rx.try_recv().is_err());
        }
    }

    #[test]
    fn real_socket_preserves_prompt_and_correlates_backend_results() {
        for error in [
            None,
            Some(SteeringError::Rejected {
                code: "no_active_turn",
                message: "No active turn.".to_string(),
            }),
            Some(SteeringError::Rejected {
                code: "backend_rejected",
                message: "Rejected.".to_string(),
            }),
            Some(SteeringError::AcceptanceUnknown {
                message: "Steering acceptance is unknown.".to_string(),
            }),
        ] {
            let fixture = Fixture::new();
            let path = fixture.server.path.clone();
            let endpoint = path.clone();
            let (release, wait) = mpsc::channel();
            let mut backend = FakeBackend {
                steer_error: error.clone(),
                wait_action: Some(Box::new(move || {
                    wait.recv_timeout(Duration::from_secs(5)).unwrap();
                    Ok(())
                })),
                ..FakeBackend::default()
            };
            let prompt = "  steer\nλ, quotes '\"; $() `data`  ";
            let started = backend
                .start(StartContext {
                    agent_id: "aa-1",
                    prompt: "original",
                    repository_root: fixture.repo.path(),
                    worktree_dir: fixture.repo.path(),
                })
                .unwrap();
            let client = thread::spawn(move || {
                let mut stream = UnixStream::connect(&path).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let bytes = frame(21, prompt);
                stream.write_all(&bytes[..20]).unwrap();
                stream.write_all(&bytes[20..]).unwrap();
                let result = read_response(stream);
                release.send(()).unwrap();
                result
            });
            assert_eq!(
                fixture.server.wait(started.handle.as_ref()).unwrap(),
                RunOutcome::Completed
            );
            let actual = client.join().unwrap();
            assert_eq!(
                actual,
                response(&Uuid::from_u128(21).to_string(), error.map_or(Ok(()), Err))
            );
            assert_eq!(backend.steer_calls.lock().unwrap().len(), 1);
            assert_eq!(backend.steer_calls.lock().unwrap()[0].0, prompt);
            assert!(!endpoint.exists());
        }
    }

    #[test]
    fn completion_progresses_during_dispatch_and_rejects_queued_requests() {
        struct BlockingRun {
            completion: Mutex<Receiver<()>>,
            entered: mpsc::Sender<String>,
            release: Mutex<Receiver<()>>,
        }
        impl RunHandle for BlockingRun {
            fn wait(&self) -> io::Result<RunOutcome> {
                self.completion
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap();
                Ok(RunOutcome::Completed)
            }

            fn steer(&self, prompt: &str, _deadline: Instant) -> Result<(), SteeringError> {
                self.entered.send(prompt.to_string()).unwrap();
                self.release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap();
                Ok(())
            }
        }
        let fixture = Fixture::new();
        let path = fixture.server.path.clone();
        let (complete, completion) = mpsc::channel();
        let (entered, calls) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let run = BlockingRun {
            completion: Mutex::new(completion),
            entered,
            release: Mutex::new(blocked),
        };
        let first = fixture.request(1, "blocked");
        thread::scope(|scope| {
            let owner = scope.spawn(|| fixture.server.wait(&run));
            assert_eq!(
                calls.recv_timeout(Duration::from_secs(3)).unwrap(),
                "blocked"
            );
            let connect = |index| {
                let mut stream = UnixStream::connect(&path).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                stream.write_all(&frame(index, "pending")).unwrap();
                stream
            };
            let pending: Vec<_> = (2..=17).map(connect).collect();
            let overflow = connect(18);
            assert_error(&read_response(overflow), json!(false), "busy");
            complete.send(()).unwrap();
            assert_error(&read_response(first), Value::Null, "acceptance_unknown");
            for client in pending {
                assert_error(&read_response(client), json!(false), "agent_exited");
            }
            assert!(
                !path.exists(),
                "endpoint is removed before joining late dispatch"
            );
            assert!(
                calls.try_recv().is_err(),
                "queued requests did not dispatch"
            );
            release.send(()).unwrap();
            assert_eq!(owner.join().unwrap().unwrap(), RunOutcome::Completed);
        });
    }

    #[test]
    fn fifo_allows_one_dispatch_and_sixteen_pending_then_rejects_overflow() {
        let mut fixture = Fixture::new();
        let first = fixture.request(1, "first");
        fixture.tick();
        let first_call = fixture.command_rx.try_recv().unwrap();
        assert_eq!(first_call.0, "first");
        let mut pending = Vec::new();
        for index in 2..=17 {
            pending.push((index, fixture.request(index, &index.to_string())));
            fixture.tick();
        }
        assert_eq!(fixture.server.pending.len(), MAX_PENDING);
        let overflow = fixture.request(18, "overflow");
        fixture.tick();
        let value = read_response(overflow);
        assert_error(&value, json!(false), "busy");
        assert_eq!(value["request_id"], Uuid::from_u128(18).to_string());
        assert!(fixture.command_rx.try_recv().is_err());
        fixture.acknowledge(Ok(()));
        assert_eq!(
            read_response(first)["request_id"],
            Uuid::from_u128(1).to_string()
        );
        for (index, client) in pending {
            let (prompt, deadline) = fixture.command_rx.try_recv().unwrap();
            assert_eq!(prompt, index.to_string());
            assert!(deadline >= first_call.1);
            fixture.acknowledge(Ok(()));
            assert_eq!(
                read_response(client),
                response(&Uuid::from_u128(index).to_string(), Ok(()))
            );
        }
        assert!(fixture.server.dispatch.is_none());
        assert!(fixture.server.pending.is_empty());
    }

    #[test]
    fn fragmented_utf8_enqueues_only_complete_frames_in_server_receipt_order() {
        let mut fixture = Fixture::new();
        let prompt = "雪\nλ 'quoted' $(data)";
        let bytes = frame(1, prompt);
        let split = bytes.iter().position(|byte| *byte == 0xe9).unwrap() + 1;
        let mut slow = fixture.connect(&bytes[..split]);
        fixture.tick();
        assert!(fixture.server.pending.is_empty());
        assert!(fixture.command_rx.try_recv().is_err());
        let fast = fixture.request(2, "complete first");
        fixture.tick();
        assert_eq!(fixture.command_rx.try_recv().unwrap().0, "complete first");
        slow.write_all(&bytes[split..bytes.len() - 1]).unwrap();
        fixture.tick();
        assert!(
            fixture.server.pending.is_empty(),
            "newline terminates a frame"
        );
        slow.write_all(b"\n").unwrap();
        fixture.tick();
        assert_eq!(fixture.server.pending.len(), 1);
        fixture.acknowledge(Ok(()));
        assert_eq!(
            read_response(fast)["request_id"],
            Uuid::from_u128(2).to_string()
        );
        assert_eq!(fixture.command_rx.try_recv().unwrap().0, prompt);
        fixture.acknowledge(Ok(()));
        assert_eq!(
            read_response(slow)["request_id"],
            Uuid::from_u128(1).to_string()
        );
        assert!(fixture.command_rx.try_recv().is_err());
    }

    #[test]
    fn frame_limit_includes_the_newline_and_connection_serves_only_one_request() {
        let mut fixture = Fixture::new();
        let overhead = frame(1, "").len();
        let prompt = "x".repeat(MAX_FRAME_BYTES - overhead);
        let bytes = frame(1, &prompt);
        assert_eq!(bytes.len(), MAX_FRAME_BYTES);
        let client = fixture.connect(&bytes);
        fixture.tick();
        assert_eq!(fixture.command_rx.try_recv().unwrap().0, prompt);
        fixture.acknowledge(Ok(()));
        assert_eq!(read_response(client)["accepted"], true);
        let oversized = fixture.connect(&frame(2, &format!("{prompt}x")));
        fixture.tick();
        assert_error(&read_response(oversized), json!(false), "request_too_large");
        assert!(fixture.command_rx.try_recv().is_err());
        let mut two_frames = frame(3, "one");
        two_frames.extend(frame(4, "two"));
        let client = fixture.connect(&two_frames);
        fixture.tick();
        assert_eq!(fixture.command_rx.try_recv().unwrap().0, "one");
        fixture.acknowledge(Ok(()));
        assert_eq!(
            read_response(client)["request_id"],
            Uuid::from_u128(3).to_string()
        );
        assert!(fixture.command_rx.try_recv().is_err());
    }

    #[test]
    fn connection_cap_and_absolute_incomplete_frame_deadline() {
        let mut fixture = Fixture::new();
        let mut clients: Vec<_> = (0..MAX_CONNECTIONS)
            .map(|_| fixture.connect(b"{"))
            .collect();
        fixture.tick();
        assert_eq!(fixture.server.connections.len(), MAX_CONNECTIONS);
        let mut excess = fixture.connect(b"");
        fixture.tick();
        let mut byte = [0];
        assert_eq!(excess.read(&mut byte).unwrap(), 0);
        assert_eq!(fixture.server.connections.len(), MAX_CONNECTIONS);
        let deadline = fixture
            .server
            .connections
            .values()
            .map(|connection| {
                let Stage::Reading { deadline, .. } = connection.stage else {
                    panic!("reading")
                };
                deadline
            })
            .max()
            .unwrap();
        clients[0].write_all(b" ").unwrap();
        fixture
            .server
            .read_requests(deadline - Duration::from_millis(1));
        fixture
            .server
            .flush_replies(deadline - Duration::from_millis(1));
        assert_eq!(fixture.server.connections.len(), MAX_CONNECTIONS);
        fixture.server.read_requests(deadline);
        fixture.server.flush_replies(deadline);
        assert!(fixture.server.connections.is_empty());
        for mut client in clients {
            assert_eq!(client.read(&mut byte).unwrap(), 0);
        }
        assert!(fixture.command_rx.try_recv().is_err());
        assert_eq!(FRAME_TIMEOUT, Duration::from_secs(5));
    }

    #[test]
    fn queued_expiry_is_rejection_and_late_ack_cannot_dispatch_concurrently_or_misroute() {
        let mut fixture = Fixture::new();
        let first = fixture.request(1, "first");
        fixture.tick();
        let (_, dispatch_deadline) = fixture.command_rx.try_recv().unwrap();
        let second = fixture.request(2, "expires queued");
        fixture.tick();
        let queued_deadline = fixture.server.pending.front().unwrap().deadline;
        assert!(queued_deadline >= dispatch_deadline);
        assert_eq!(ACK_TIMEOUT, Duration::from_secs(30));
        let now = queued_deadline;
        fixture.server.expire_requests(now);
        fixture.server.flush_replies(now);
        assert_error(&read_response(first), Value::Null, "acceptance_unknown");
        assert_error(&read_response(second), json!(false), "timeout");
        assert!(fixture.server.pending.is_empty());
        assert!(fixture.server.dispatch.as_ref().unwrap().responded);
        let third = fixture.request(3, "next");
        fixture.tick();
        assert!(
            fixture.command_rx.try_recv().is_err(),
            "late acknowledgement still owns dispatch slot"
        );
        fixture
            .replies
            .send(DispatchResult {
                result: Ok(()),
                completed_at: now,
                called_backend: true,
            })
            .unwrap();
        fixture.tick();
        assert_eq!(fixture.command_rx.try_recv().unwrap().0, "next");
        fixture.acknowledge(Err(SteeringError::Rejected {
            code: "backend_rejected",
            message: "third only".to_string(),
        }));
        let value = read_response(third);
        assert_eq!(value["request_id"], Uuid::from_u128(3).to_string());
        assert_error(&value, json!(false), "backend_rejected");
    }

    #[test]
    fn dispatch_rechecks_deadline_and_never_resets_queued_deadline() {
        let mut fixture = Fixture::new();
        let first = fixture.request(1, "first");
        fixture.tick();
        fixture.command_rx.try_recv().unwrap();
        let second = fixture.request(2, "second");
        fixture.tick();
        let deadline = fixture.server.pending.front().unwrap().deadline;
        fixture.acknowledge(Ok(()));
        read_response(first);
        assert_eq!(fixture.command_rx.try_recv().unwrap().1, deadline);
        fixture.acknowledge(Ok(()));
        read_response(second);

        let third = fixture.request(3, "expired");
        fixture.server.accept_connections(Instant::now());
        fixture.server.read_requests(Instant::now());
        let deadline = fixture.server.pending.front().unwrap().deadline;
        fixture.server.dispatch_next(&fixture.commands, deadline);
        fixture.server.flush_replies(deadline);
        assert_error(&read_response(third), json!(false), "timeout");
        assert!(fixture.command_rx.try_recv().is_err());
    }

    #[test]
    fn acknowledgement_time_determines_acceptance_even_when_owner_polls_after_deadline() {
        for timely in [true, false] {
            let mut fixture = Fixture::new();
            let client = fixture.request(1, "steer");
            fixture.tick();
            fixture.command_rx.try_recv().unwrap();
            let deadline = fixture.server.dispatch.as_ref().unwrap().pending.deadline;
            fixture
                .replies
                .send(DispatchResult {
                    result: Ok(()),
                    completed_at: if timely {
                        deadline - Duration::from_nanos(1)
                    } else {
                        deadline
                    },
                    called_backend: true,
                })
                .unwrap();
            fixture.server.service(
                &fixture.commands,
                &fixture.reply_rx,
                deadline + Duration::from_millis(1),
            );
            let value = read_response(client);
            if timely {
                assert_eq!(value["accepted"], true);
            } else {
                assert_error(&value, Value::Null, "acceptance_unknown");
            }
        }
    }

    #[test]
    fn same_agent_id_in_two_repositories_uses_independent_sockets_and_dispatch() {
        let mut first = Fixture::new();
        let mut second = Fixture::new();
        second.server = ControlServer::bind(
            &find_control_root_in(second.repo.path(), Some(first.runtime.path().as_os_str()))
                .unwrap(),
            "aa-1",
        )
        .unwrap();
        assert_ne!(first.server.path, second.server.path);
        let client_a = first.request(1, "repository a");
        let client_b = second.request(1, "repository b");
        first.tick();
        second.tick();
        assert_eq!(first.command_rx.try_recv().unwrap().0, "repository a");
        assert_eq!(second.command_rx.try_recv().unwrap().0, "repository b");
        second.acknowledge(Err(SteeringError::not_implemented("claude")));
        first.acknowledge(Ok(()));
        assert_eq!(read_response(client_a)["accepted"], true);
        assert_error(&read_response(client_b), json!(false), "not_implemented");
    }

    #[test]
    fn completion_rejects_pending_and_cleans_endpoint_without_repairing_stale_files() {
        let mut fixture = Fixture::new();
        let path = fixture.server.path.clone();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        let first = fixture.request(1, "dispatched");
        fixture.tick();
        fixture.command_rx.try_recv().unwrap();
        let second = fixture.request(2, "queued");
        fixture.tick();
        fixture.server.finish(Instant::now());
        fixture.server.remove_socket().unwrap();
        assert_error(&read_response(first), Value::Null, "acceptance_unknown");
        assert_error(&read_response(second), json!(false), "agent_exited");
        assert!(!path.exists());
        assert!(UnixStream::connect(&path).is_err());
        let stale = UnixListener::bind(&path).unwrap();
        drop(stale);
        assert!(UnixStream::connect(&path).is_err());
        let error = ControlServer::bind(
            &find_control_root_in(
                fixture.repo.path(),
                Some(fixture.runtime.path().as_os_str()),
            )
            .unwrap(),
            "aa-1",
        )
        .err()
        .unwrap();
        assert!(error
            .to_string()
            .contains("existing endpoints are not automatically removed"));
        assert!(path.exists());
    }

    #[test]
    fn bind_rejects_insecure_or_symlinked_runtime_descendants() {
        let fixture = Fixture::new();
        let parent = fixture.server.path.parent().unwrap();
        fs::set_permissions(parent, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(ControlServer::bind(
            &find_control_root_in(
                fixture.repo.path(),
                Some(fixture.runtime.path().as_os_str())
            )
            .unwrap(),
            "aa-2"
        )
        .is_err());
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).unwrap();
        let runtime = tempfile::tempdir().unwrap();
        fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(
            fixture.runtime.path().join("waap"),
            runtime.path().join("waap"),
        )
        .unwrap();
        assert!(ControlServer::bind(
            &find_control_root_in(fixture.repo.path(), Some(runtime.path().as_os_str())).unwrap(),
            "aa-2"
        )
        .is_err());
    }

    pub(in crate::agent) fn assert_backend_socket_rejection(
        steer: impl Fn(&str, Instant) -> Result<(), SteeringError> + Sync,
        backend: &str,
    ) {
        struct Run<F> {
            steer: F,
            completion: Mutex<Receiver<()>>,
        }
        impl<F: Fn(&str, Instant) -> Result<(), SteeringError> + Sync> RunHandle for Run<F> {
            fn wait(&self) -> io::Result<RunOutcome> {
                self.completion
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap();
                Ok(RunOutcome::Completed)
            }
            fn steer(&self, prompt: &str, deadline: Instant) -> Result<(), SteeringError> {
                (self.steer)(prompt, deadline)
            }
        }
        let fixture = Fixture::new();
        let path = fixture.server.path.clone();
        let (complete, completion) = mpsc::channel();
        let client = thread::spawn(move || {
            let mut stream = UnixStream::connect(path).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            stream.write_all(&frame(1, "steer")).unwrap();
            let result = read_response(stream);
            complete.send(()).unwrap();
            result
        });
        fixture
            .server
            .wait(&Run {
                steer,
                completion: Mutex::new(completion),
            })
            .unwrap();
        let value = client.join().unwrap();
        assert_error(&value, json!(false), "not_implemented");
        assert_eq!(
            value["error"]["message"],
            format!("steering not implemented for backend {backend}")
        );
    }
}
