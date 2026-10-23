use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::backend::{RunHandle, RunOutcome};
use super::worktree::collapse_errors;
use crate::root::ControlRoot;

pub(super) mod client;
mod endpoint;
mod protocol;

use endpoint::prepare_parent;
pub(super) use endpoint::socket_path;
use protocol::{parse_request, rejection, response, MAX_FRAME_BYTES};

const IO_TIMEOUT: Duration = Duration::from_secs(5);
pub(super) const ACK_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) struct ControlServer {
    listener: Option<UnixListener>,
    path: PathBuf,
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
        };
        fs::set_permissions(&server.path, fs::Permissions::from_mode(0o600))?;
        server.listener.as_ref().unwrap().set_nonblocking(true)?;
        Ok(server)
    }

    pub(super) fn wait(mut self, handle: &dyn RunHandle) -> io::Result<RunOutcome> {
        let completed = AtomicBool::new(false);
        let outcome = thread::scope(|scope| {
            let worker = thread::Builder::new()
                .name("waap-control".to_string())
                .spawn_scoped(scope, || {
                    let listener = self.listener.as_ref().unwrap();
                    while !completed.load(Ordering::Acquire) {
                        match listener.accept() {
                            Ok((stream, _)) => {
                                // One request at a time; socket/backend timeouts bound stalls.
                                // Do not log transport errors that may contain peer data.
                                let _ = serve_connection(stream, handle, &completed);
                            }
                            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                                thread::park_timeout(Duration::from_millis(10));
                            }
                            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                            Err(error) => {
                                log::error!("control socket accept failed: {error}");
                                break;
                            }
                        }
                    }
                })?;
            // The backend runs on its owner thread, independently of control I/O.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle.wait()))
                .unwrap_or_else(|_| Err(io::Error::other("agent wait panicked")));
            completed.store(true, Ordering::Release);
            worker.thread().unpark();
            if worker.join().is_err() {
                log::error!("control server panicked");
            }
            outcome
        });
        let cleanup = self.remove_socket();
        collapse_errors(outcome, cleanup)
    }

    fn remove_socket(&mut self) -> io::Result<()> {
        self.listener.take();
        fs::remove_file(&self.path)
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        if self.listener.is_some() {
            if let Err(error) = self.remove_socket() {
                log::error!(
                    "failed to remove control socket {}: {error}",
                    self.path.display()
                );
            }
        }
    }
}

fn serve_connection(
    mut stream: UnixStream,
    handle: &dyn RunHandle,
    completed: &AtomicBool,
) -> io::Result<()> {
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let value = match read_frame(&mut stream) {
        Ok(frame) => match parse_request(&frame) {
            Ok(request) => {
                if completed.load(Ordering::Acquire) {
                    rejection(
                        Some(&request.request_id),
                        "agent_exited",
                        "Agent exited before steering dispatch.",
                    )
                } else {
                    response(
                        &request.request_id,
                        handle.steer(&request.prompt, Instant::now() + ACK_TIMEOUT),
                    )
                }
            }
            Err(value) => value,
        },
        Err(error) if error.kind() == io::ErrorKind::InvalidData => rejection(
            None,
            "request_too_large",
            "Request exceeds 64 KiB including newline.",
        ),
        Err(error) => return Err(error),
    };
    write_response(&mut stream, &value)
}

fn write_response(stream: &mut UnixStream, value: &Value) -> io::Result<()> {
    let mut frame = serde_json::to_vec(value)?;
    frame.push(b'\n');
    stream.write_all(&frame)
}

// Bound allocation before reading. A limit-sized frame must include its newline.
fn read_frame(stream: &mut UnixStream) -> io::Result<Vec<u8>> {
    let mut frame = Vec::new();
    BufReader::new(stream.take(MAX_FRAME_BYTES as u64)).read_until(b'\n', &mut frame)?;
    if frame.last() != Some(&b'\n') {
        let kind = if frame.len() == MAX_FRAME_BYTES {
            io::ErrorKind::InvalidData
        } else {
            io::ErrorKind::UnexpectedEof
        };
        return Err(io::Error::new(
            kind,
            "expected a complete NDJSON frame within 64 KiB",
        ));
    }
    Ok(frame)
}

#[cfg(test)]
pub(in crate::agent) mod tests {
    use super::super::backend::SteeringError;
    use super::*;
    use crate::root::find_control_root_in;
    use serde_json::json;
    use std::sync::{
        mpsc::{self, Receiver},
        Mutex,
    };
    use tempfile::{tempdir, TempDir};

    struct Fixture {
        repo: TempDir,
        runtime: TempDir,
        server: ControlServer,
    }

    impl Fixture {
        fn new() -> Self {
            let repo = tempdir().unwrap();
            std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(repo.path())
                .status()
                .unwrap();
            let runtime = tempdir().unwrap();
            fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
            let root = find_control_root_in(repo.path(), Some(runtime.path().as_os_str())).unwrap();
            let server = ControlServer::bind(&root, "aa-1").unwrap();
            Self {
                repo,
                runtime,
                server,
            }
        }
    }

    struct Run<F> {
        steer: F,
        completion: Mutex<Receiver<()>>,
    }

    impl<F: Fn(&str, Instant) -> Result<(), SteeringError> + Sync> RunHandle for Run<F> {
        fn wait(&self) -> io::Result<RunOutcome> {
            self.completion
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
            Ok(RunOutcome::Completed)
        }
        fn steer(&self, prompt: &str, deadline: Instant) -> Result<(), SteeringError> {
            (self.steer)(prompt, deadline)
        }
    }

    fn frame(prompt: &str) -> Vec<u8> {
        let mut bytes = serde_json::to_vec(&json!({"version":1,"request_id":uuid::Uuid::nil().to_string(),"method":"steer","prompt":prompt})).unwrap();
        bytes.push(b'\n');
        bytes
    }

    fn connect(path: &std::path::Path) -> UnixStream {
        let stream = UnixStream::connect(path).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
    }

    fn read_response(mut stream: UnixStream) -> Value {
        serde_json::from_slice(&read_frame(&mut stream).unwrap()).unwrap()
    }

    #[test]
    fn socket_preserves_prompt_and_correlates_acknowledgement() {
        let fixture = Fixture::new();
        let path = fixture.server.path.clone();
        let (complete, completion) = mpsc::channel();
        let prompt = "  Unicode 雪\n\"quoted\" '$HOME' `echo data`  ";
        let client = thread::spawn(move || {
            let mut stream = connect(&path);
            stream.write_all(&frame(prompt)).unwrap();
            let result = read_response(stream);
            complete.send(()).unwrap();
            result
        });
        fixture
            .server
            .wait(&Run {
                completion: Mutex::new(completion),
                steer: |received: &str, deadline| {
                    assert_eq!(received, prompt);
                    assert!(deadline > Instant::now());
                    Ok(())
                },
            })
            .unwrap();
        assert_eq!(
            client.join().unwrap(),
            json!({"version":1,"request_id":uuid::Uuid::nil().to_string(),"accepted":true})
        );
    }

    #[test]
    fn control_calls_are_serial_and_a_client_timeout_does_not_cancel_delivery() {
        let fixture = Fixture::new();
        let path = fixture.server.path.clone();
        let (entered, dispatched) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let resume = Mutex::new(resume);
        let (complete, completion) = mpsc::channel();
        let calls = Mutex::new(Vec::new());
        let client = thread::spawn(move || {
            let mut first = connect(&path);
            first.write_all(&frame("first")).unwrap();
            dispatched.recv_timeout(Duration::from_secs(3)).unwrap();
            let mut second = connect(&path);
            second.write_all(&frame("second")).unwrap();
            second
                .set_read_timeout(Some(Duration::from_millis(20)))
                .unwrap();
            let error = read_frame(&mut second).unwrap_err();
            assert!(matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ));
            // Timing out does not cancel a request already sent to the socket.
            second
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            release.send(()).unwrap();
            assert_eq!(read_response(first)["accepted"], true);
            assert_eq!(read_response(second)["accepted"], true);
            complete.send(()).unwrap();
        });
        fixture
            .server
            .wait(&Run {
                completion: Mutex::new(completion),
                steer: |prompt: &str, _| {
                    calls.lock().unwrap().push(prompt.to_string());
                    if prompt == "first" {
                        entered.send(()).unwrap();
                        resume
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(3))
                            .unwrap();
                    } else {
                        assert_eq!(prompt, "second");
                    }
                    Ok(())
                },
            })
            .unwrap();
        client.join().unwrap();
        assert_eq!(*calls.lock().unwrap(), ["first", "second"]);
    }

    #[test]
    fn backend_wait_stays_on_its_owner_thread_and_control_runs_in_background() {
        struct OwnerRun {
            owner: thread::ThreadId,
            completion: Mutex<Receiver<()>>,
        }
        impl RunHandle for OwnerRun {
            fn wait(&self) -> io::Result<RunOutcome> {
                assert_eq!(thread::current().id(), self.owner);
                self.completion
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(3))
                    .unwrap();
                Ok(RunOutcome::Completed)
            }
            fn steer(&self, _: &str, _: Instant) -> Result<(), SteeringError> {
                assert_ne!(thread::current().id(), self.owner);
                assert_eq!(thread::current().name(), Some("waap-control"));
                Ok(())
            }
        }
        let fixture = Fixture::new();
        let path = fixture.server.path.clone();
        let (complete, completion) = mpsc::channel();
        let client = thread::spawn(move || {
            let mut stream = connect(&path);
            stream.write_all(&frame("steer")).unwrap();
            assert_eq!(read_response(stream)["accepted"], true);
            complete.send(()).unwrap();
        });
        fixture
            .server
            .wait(&OwnerRun {
                owner: thread::current().id(),
                completion: Mutex::new(completion),
            })
            .unwrap();
        client.join().unwrap();
    }

    #[test]
    fn socket_rejects_invalid_schemas_without_backend_dispatch_or_prompt_disclosure() {
        let secret = "private prompt";
        let valid: Value = serde_json::from_slice(&frame(secret)).unwrap();
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
        cases.push((vec![b'x'; MAX_FRAME_BYTES], "request_too_large", false));
        for (bytes, code, trusted_id) in cases {
            let (mut client, server) = UnixStream::pair().unwrap();
            let (_complete, completion) = mpsc::channel();
            let run = Run {
                completion: Mutex::new(completion),
                steer: |_: &str, _| panic!("invalid request dispatched"),
            };
            thread::scope(|scope| {
                scope.spawn(|| serve_connection(server, &run, &AtomicBool::new(false)).unwrap());
                client.write_all(&bytes).unwrap();
                let value = read_response(client);
                assert_eq!(value["accepted"], false);
                assert_eq!(value["error"]["code"], code);
                assert_eq!(
                    value["request_id"],
                    if trusted_id {
                        valid["request_id"].clone()
                    } else {
                        Value::Null
                    }
                );
                assert!(!value.to_string().contains(secret));
            });
        }
    }

    #[test]
    fn frame_limit_includes_newline_and_reads_only_one_frame() {
        for size in [MAX_FRAME_BYTES - 1, MAX_FRAME_BYTES, MAX_FRAME_BYTES + 1] {
            let (mut client, mut server) = UnixStream::pair().unwrap();
            let mut bytes = vec![b'x'; size - 1];
            bytes.push(b'\n');
            bytes.extend_from_slice(b"second\n");
            thread::scope(|scope| {
                scope.spawn(|| client.write_all(&bytes).unwrap());
                let result = read_frame(&mut server);
                if size <= MAX_FRAME_BYTES {
                    assert_eq!(result.unwrap().len(), size);
                } else {
                    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
                }
            });
        }
    }

    #[test]
    fn incomplete_frame_has_an_inactivity_timeout() {
        let (_client, mut server) = UnixStream::pair().unwrap();
        server
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let error = read_frame(&mut server).unwrap_err();
        assert!(matches!(
            error.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
        ));
    }

    #[test]
    fn completion_closes_an_idle_connection_and_removes_endpoint() {
        let fixture = Fixture::new();
        let path = fixture.server.path.clone();
        let (complete, completion) = mpsc::channel();
        let client = thread::spawn(move || {
            let mut stream = connect(&path);
            stream
                .set_read_timeout(Some(Duration::from_secs(6)))
                .unwrap();
            complete.send(()).unwrap();
            match stream.read(&mut [0]) {
                Ok(0) => {}
                Err(error) if error.kind() == io::ErrorKind::ConnectionReset => {}
                result => panic!("expected closed connection, got {result:?}"),
            }
        });
        let path = fixture.server.path.clone();
        fixture
            .server
            .wait(&Run {
                completion: Mutex::new(completion),
                steer: |_: &str, _| panic!("unexpected dispatch"),
            })
            .unwrap();
        client.join().unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn endpoint_is_private_and_stale_socket_is_not_replaced() {
        let fixture = Fixture::new();
        assert_eq!(
            fs::metadata(&fixture.server.path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let root = find_control_root_in(
            fixture.repo.path(),
            Some(fixture.runtime.path().as_os_str()),
        )
        .unwrap();
        let error = ControlServer::bind(&root, "aa-1").err().unwrap();
        assert!(error
            .to_string()
            .contains("existing endpoints are not automatically removed"));
        assert!(fixture.server.path.exists());
    }

    #[test]
    fn bind_rejects_insecure_or_symlinked_runtime_descendants() {
        let fixture = Fixture::new();
        let parent = fixture.server.path.parent().unwrap();
        fs::set_permissions(parent, fs::Permissions::from_mode(0o755)).unwrap();
        let root = find_control_root_in(
            fixture.repo.path(),
            Some(fixture.runtime.path().as_os_str()),
        )
        .unwrap();
        assert!(ControlServer::bind(&root, "aa-2").is_err());
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).unwrap();
        let runtime = tempdir().unwrap();
        fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(
            fixture.runtime.path().join("waap"),
            runtime.path().join("waap"),
        )
        .unwrap();
        let root =
            find_control_root_in(fixture.repo.path(), Some(runtime.path().as_os_str())).unwrap();
        assert!(ControlServer::bind(&root, "aa-2").is_err());
    }

    pub(in crate::agent) fn assert_backend_socket_rejection(
        steer: impl Fn(&str, Instant) -> Result<(), SteeringError> + Sync,
        backend: &str,
    ) {
        let fixture = Fixture::new();
        let path = fixture.server.path.clone();
        let (complete, completion) = mpsc::channel();
        let client = thread::spawn(move || {
            let mut stream = connect(&path);
            stream.write_all(&frame("steer")).unwrap();
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
        assert_eq!(value["accepted"], false);
        assert_eq!(value["error"]["code"], "not_implemented");
        assert_eq!(
            value["error"]["message"],
            format!("steering not implemented for backend {backend}")
        );
    }
}
