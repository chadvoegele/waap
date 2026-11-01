use std::time::Duration;

pub(super) mod client;
mod handler;
mod protocol;
mod server;
mod socket;

pub(super) use server::ControlServer;
#[cfg(test)]
use socket::SocketConnection;
pub(super) use socket::{SocketListener, SocketPath};

const IO_TIMEOUT: Duration = Duration::from_secs(5);
const ACK_TIMEOUT: Duration = Duration::from_secs(30);

#[cfg(test)]
pub(in crate::agent) mod tests {
    use super::handler::ControlRequestHandler;
    use super::socket::MAX_FRAME_BYTES;
    use super::*;
    use crate::agent::backend::{RunOutcome, SteeringError};
    use crate::agent::commands::{Command, CommandHandle};
    use crate::root::{find_control_root_in, ControlRoot};
    use serde_json::{json, Value};
    use std::fs;
    use std::io::{self, Read, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;
    use std::sync::{
        mpsc::{self, Receiver},
        Arc, Mutex,
    };
    use std::thread;
    use std::time::Instant;
    use tempfile::{tempdir, TempDir};

    struct Fixture {
        repo: TempDir,
        runtime: TempDir,
        server: ControlServer,
        path: PathBuf,
    }

    impl Fixture {
        fn wait<F: Fn(&str, Instant) -> Result<(), SteeringError>>(
            self,
            run: &Run<F>,
        ) -> io::Result<RunOutcome> {
            crate::agent::runtime::block_on(async {
                let (handle, mut commands) = CommandHandle::channel();
                let completion = Arc::clone(&run.completion);
                let wait = async {
                    let mut completion = tokio::task::spawn_blocking(move || {
                        completion
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(10))
                            .unwrap();
                    });
                    loop {
                        tokio::select! {
                            result = &mut completion => { result.unwrap(); return Ok(RunOutcome::Completed); }
                            Some(Command::Steer(mut request)) = commands.recv() => {
                                request.respond((run.steer)(&request.message, request.deadline));
                            }
                        }
                    }
                };
                crate::agent::run::wait_with_control_async(self.server, wait, &handle).await
            })
        }

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
            let path = SocketPath::for_agent(&root, "aa-1")
                .unwrap()
                .as_path()
                .to_path_buf();
            let server = bind_server(&root, "aa-1").unwrap();
            Self {
                repo,
                runtime,
                server,
                path,
            }
        }
    }

    struct Run<F> {
        steer: F,
        completion: Arc<Mutex<Receiver<()>>>,
    }

    fn bind_server(root: &ControlRoot, agent_id: &str) -> io::Result<ControlServer> {
        Ok(ControlServer::new(SocketListener::bind(
            SocketPath::for_agent(root, agent_id)?,
        )?))
    }

    fn read_frame(stream: &mut UnixStream) -> io::Result<Vec<u8>> {
        let stream = stream.try_clone()?;
        stream.set_nonblocking(true)?;
        crate::agent::runtime::block_on(async {
            SocketConnection::new(tokio::net::UnixStream::from_std(stream)?)
                .read_frame(IO_TIMEOUT)
                .await
        })
    }

    fn serve_connection(
        stream: UnixStream,
        run: &Run<impl Fn(&str, Instant) -> Result<(), SteeringError>>,
    ) -> io::Result<()> {
        stream.set_nonblocking(true)?;
        crate::agent::runtime::block_on(async {
            let (handle, mut commands) = CommandHandle::channel();
            let handler = ControlRequestHandler::new(&handle);
            tokio::select! {
                result = handler.serve(SocketConnection::new(tokio::net::UnixStream::from_std(stream)?)) => result,
                Some(Command::Steer(request)) = commands.recv() => { (run.steer)(&request.message, request.deadline).unwrap(); panic!("unexpected dispatch"); }
            }
        })
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
        let path = fixture.path.clone();
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
            .wait(&Run {
                completion: Arc::new(Mutex::new(completion)),
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

    #[tokio::test(flavor = "current_thread")]
    async fn delayed_ack_does_not_serialize_handlers_and_completion_cancels_reply_wait() {
        let fixture = Fixture::new();
        let path = fixture.path.clone();
        let (handle, mut commands) = CommandHandle::channel();
        let entered = tokio::sync::Notify::new();
        let release = tokio::sync::Notify::new();
        let complete = tokio::sync::Notify::new();
        let driver = async {
            let actor = async {
                let Some(Command::Steer(mut first)) = commands.recv().await else {
                    panic!()
                };
                entered.notify_one();
                let Some(Command::Steer(mut second)) = commands.recv().await else {
                    panic!()
                };
                second.respond(Ok(()));
                release.notified().await;
                first.respond(Ok(()));
                let Some(Command::Steer(_held)) = commands.recv().await else {
                    panic!()
                };
                entered.notify_one();
                std::future::pending::<()>().await;
            };
            tokio::select! { _ = complete.notified() => Ok(RunOutcome::Completed), _ = actor => panic!() }
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            let (outcome, ()) = tokio::join!(
                crate::agent::run::wait_with_control_async(fixture.server, driver, &handle),
                async {
                    let mut first = SocketConnection::new(
                        tokio::net::UnixStream::connect(&path).await.unwrap(),
                    );
                    let bytes = frame("first");
                    first.write_frame(&bytes[..bytes.len() - 1]).await.unwrap();
                    entered.notified().await;
                    let mut second = SocketConnection::new(
                        tokio::net::UnixStream::connect(&path).await.unwrap(),
                    );
                    let bytes = frame("second");
                    second.write_frame(&bytes[..bytes.len() - 1]).await.unwrap();
                    let reply: Value =
                        serde_json::from_slice(&second.read_frame(IO_TIMEOUT).await.unwrap())
                            .unwrap();
                    assert_eq!(reply["accepted"], true);
                    release.notify_one();
                    let reply: Value =
                        serde_json::from_slice(&first.read_frame(IO_TIMEOUT).await.unwrap())
                            .unwrap();
                    assert_eq!(reply["accepted"], true);
                    let mut held = SocketConnection::new(
                        tokio::net::UnixStream::connect(&path).await.unwrap(),
                    );
                    let bytes = frame("held");
                    held.write_frame(&bytes[..bytes.len() - 1]).await.unwrap();
                    entered.notified().await;
                    complete.notify_one();
                    assert_eq!(
                        held.read_frame(IO_TIMEOUT).await.unwrap_err().kind(),
                        io::ErrorKind::UnexpectedEof
                    );
                }
            );
            assert_eq!(outcome.unwrap(), RunOutcome::Completed);
        })
        .await
        .unwrap();
        assert!(!path.exists());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn control_failure_does_not_restart_or_abandon_the_backend_future() {
        let mut fixture = Fixture::new();
        fixture.server.close().unwrap();
        let calls = std::cell::Cell::new(0);
        let (complete, completion) = tokio::sync::oneshot::channel();
        let handle = CommandHandle::Unsupported("test");
        let wait = async {
            calls.set(calls.get() + 1);
            completion.await.unwrap();
            Ok(RunOutcome::Completed)
        };
        let (outcome, ()) = tokio::join!(
            crate::agent::run::wait_with_control_async(fixture.server, wait, &handle),
            async {
                tokio::task::yield_now().await;
                complete.send(()).unwrap();
            }
        );
        assert_eq!(outcome.unwrap(), RunOutcome::Completed);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn command_actor_and_control_are_polled_on_the_owner_thread() {
        let fixture = Fixture::new();
        let path = fixture.path.clone();
        let owner = thread::current().id();
        let (complete, completion) = mpsc::channel();
        let client = thread::spawn(move || {
            let mut stream = connect(&path);
            stream.write_all(&frame("steer")).unwrap();
            assert_eq!(read_response(stream)["accepted"], true);
            complete.send(()).unwrap();
        });
        fixture
            .wait(&Run {
                completion: Arc::new(Mutex::new(completion)),
                steer: |_: &str, _| {
                    assert_eq!(thread::current().id(), owner);
                    Ok(())
                },
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
                completion: Arc::new(Mutex::new(completion)),
                steer: |_: &str, _| panic!("invalid request dispatched"),
            };
            thread::scope(|scope| {
                scope.spawn(|| serve_connection(server, &run).unwrap());
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
    fn completion_closes_an_idle_connection_and_removes_endpoint() {
        let fixture = Fixture::new();
        let path = fixture.path.clone();
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
        let path = fixture.path.clone();
        fixture
            .wait(&Run {
                completion: Arc::new(Mutex::new(completion)),
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
            fs::metadata(&fixture.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let root = find_control_root_in(
            fixture.repo.path(),
            Some(fixture.runtime.path().as_os_str()),
        )
        .unwrap();
        let error = bind_server(&root, "aa-1").err().unwrap();
        assert!(error
            .to_string()
            .contains("existing endpoints are not automatically removed"));
        assert!(fixture.path.exists());
    }

    #[test]
    fn bind_rejects_insecure_or_symlinked_runtime_descendants() {
        let fixture = Fixture::new();
        let parent = fixture.path.parent().unwrap();
        fs::set_permissions(parent, fs::Permissions::from_mode(0o755)).unwrap();
        let root = find_control_root_in(
            fixture.repo.path(),
            Some(fixture.runtime.path().as_os_str()),
        )
        .unwrap();
        assert!(bind_server(&root, "aa-2").is_err());
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
        assert!(bind_server(&root, "aa-2").is_err());
    }

    pub(in crate::agent) async fn assert_backend_socket_rejection(
        handle: &CommandHandle,
        backend: &str,
    ) {
        let fixture = Fixture::new();
        let path = fixture.path.clone();
        let complete = tokio::sync::Notify::new();
        let (outcome, reply) = tokio::join!(
            crate::agent::run::wait_with_control_async(
                fixture.server,
                async {
                    complete.notified().await;
                    Ok(RunOutcome::Completed)
                },
                handle
            ),
            async {
                let mut connection =
                    SocketConnection::new(tokio::net::UnixStream::connect(&path).await.unwrap());
                let bytes = frame("steer");
                connection
                    .write_frame(&bytes[..bytes.len() - 1])
                    .await
                    .unwrap();
                let reply: Value =
                    serde_json::from_slice(&connection.read_frame(IO_TIMEOUT).await.unwrap())
                        .unwrap();
                complete.notify_one();
                reply
            }
        );
        outcome.unwrap();
        assert_eq!(reply["accepted"], false);
        assert_eq!(reply["error"]["code"], "not_implemented");
        assert_eq!(
            reply["error"]["message"],
            format!("steering not implemented for backend {backend}")
        );
    }
}
