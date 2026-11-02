use std::time::Duration;

pub(super) mod client;
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
    use super::socket::MAX_FRAME_BYTES;
    use super::*;
    use crate::agent::backend::{AgentSessionRun, RunOutcome, Runner, SteeringError};
    use crate::agent::commands::CommandDispatcher;
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
        listener: SocketListener,
        path: PathBuf,
    }

    impl Fixture {
        async fn wait<F: Fn(&str, Instant) -> Result<(), SteeringError>>(
            self,
            runner: &FakeRunner<F>,
        ) -> io::Result<RunOutcome> {
            let completion = Arc::clone(&runner.completion);
            let steer_calls = Arc::new(Mutex::new(Vec::new()));
            let observed = Arc::clone(&steer_calls);
            let action = Box::new(move || {
                completion
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(10))
                    .map(|_| ())
                    .map_err(io::Error::other)
            });
            let (dispatcher, receiver) = CommandDispatcher::channel();
            let session = AgentSessionRun::new(
                "test".into(),
                Runner::Fake(crate::agent::backend::fake::FakeRunner {
                    outcome: Some(RunOutcome::Completed),
                    error: None,
                    action: Some(action),
                    wait_calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                    steer_calls: steer_calls.clone(),
                    steer_error: None,
                }),
                receiver,
            );
            let mut app = crate::agent::run::AgentApp::new(
                session,
                ControlServer::new(self.listener, dispatcher),
            );
            let result = app.run().await;
            for (message, deadline, owner) in observed.lock().unwrap().iter() {
                assert_eq!(*owner, thread::current().id());
                (runner.steer)(message, *deadline).map_err(io::Error::other)?;
            }
            result
        }

        async fn new() -> Self {
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
            let listener = bind_listener(&root, "aa-1").unwrap();
            Self {
                repo,
                runtime,
                listener,
                path,
            }
        }
    }

    struct FakeRunner<F> {
        steer: F,
        completion: Arc<Mutex<Receiver<()>>>,
    }

    fn bind_listener(root: &ControlRoot, agent_id: &str) -> io::Result<SocketListener> {
        SocketListener::bind(SocketPath::for_agent(root, agent_id)?)
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

    #[tokio::test(flavor = "current_thread")]
    async fn socket_preserves_prompt_and_correlates_acknowledgement() {
        let fixture = Fixture::new().await;
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
            .wait(&FakeRunner {
                completion: Arc::new(Mutex::new(completion)),
                steer: |received: &str, deadline| {
                    assert_eq!(received, prompt);
                    assert!(deadline > Instant::now());
                    Ok(())
                },
            })
            .await
            .unwrap();
        assert_eq!(
            client.join().unwrap(),
            json!({"version":1,"request_id":uuid::Uuid::nil().to_string(),"accepted":true})
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn control_failure_does_not_restart_or_abandon_the_backend_future() {
        let mut fixture = Fixture::new().await;
        fixture.listener.close().unwrap();
        let (complete, completion) = mpsc::channel();
        let calls = Arc::new(Mutex::new(0usize));
        let observed_calls = Arc::clone(&calls);
        let wait_action = Box::new(move || {
            *observed_calls.lock().unwrap() += 1;
            completion
                .recv_timeout(Duration::from_secs(3))
                .map(|_| ())
                .map_err(io::Error::other)
        });
        let (dispatcher, receiver) = CommandDispatcher::channel();
        let session = AgentSessionRun::new(
            "test".into(),
            Runner::Fake(crate::agent::backend::fake::FakeRunner {
                outcome: Some(RunOutcome::Completed),
                error: None,
                action: Some(wait_action),
                wait_calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                steer_calls: Arc::new(Mutex::new(Vec::new())),
                steer_error: None,
            }),
            receiver,
        );
        let mut app = crate::agent::run::AgentApp::new(
            session,
            ControlServer::new(fixture.listener, dispatcher),
        );
        let (outcome, ()) = tokio::join!(app.run(), async {
            tokio::task::yield_now().await;
            complete.send(()).unwrap();
        });
        assert_eq!(outcome.unwrap(), RunOutcome::Completed);
        assert_eq!(*calls.lock().unwrap(), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn command_actor_and_control_are_polled_on_the_owner_thread() {
        let fixture = Fixture::new().await;
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
            .wait(&FakeRunner {
                completion: Arc::new(Mutex::new(completion)),
                steer: |_: &str, _| {
                    assert_eq!(thread::current().id(), owner);
                    Ok(())
                },
            })
            .await
            .unwrap();
        client.join().unwrap();
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

    #[tokio::test(flavor = "current_thread")]
    async fn completion_closes_an_idle_connection_and_removes_endpoint() {
        let fixture = Fixture::new().await;
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
            .wait(&FakeRunner {
                completion: Arc::new(Mutex::new(completion)),
                steer: |_: &str, _| panic!("unexpected dispatch"),
            })
            .await
            .unwrap();
        client.join().unwrap();
        assert!(!path.exists());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn endpoint_is_private_and_stale_socket_is_not_replaced() {
        let fixture = Fixture::new().await;
        assert_eq!(
            fs::metadata(&fixture.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let root = find_control_root_in(
            fixture.repo.path(),
            Some(fixture.runtime.path().as_os_str()),
        )
        .unwrap();
        let error = bind_listener(&root, "aa-1").err().unwrap();
        assert!(error
            .to_string()
            .contains("existing endpoints are not automatically removed"));
        assert!(fixture.path.exists());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn bind_rejects_insecure_or_symlinked_runtime_descendants() {
        let fixture = Fixture::new().await;
        let parent = fixture.path.parent().unwrap();
        fs::set_permissions(parent, fs::Permissions::from_mode(0o755)).unwrap();
        let root = find_control_root_in(
            fixture.repo.path(),
            Some(fixture.runtime.path().as_os_str()),
        )
        .unwrap();
        assert!(bind_listener(&root, "aa-2").is_err());
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
        assert!(bind_listener(&root, "aa-2").is_err());
    }

    pub(in crate::agent) async fn assert_backend_socket_rejection(
        runner: &mut Runner,
        backend: &'static str,
    ) {
        let fixture = Fixture::new().await;
        let path = fixture.path.clone();
        let complete = tokio::sync::Notify::new();
        let (dispatcher, receiver) = CommandDispatcher::channel();
        let mut server = ControlServer::new(fixture.listener, dispatcher);
        let (outcome, reply) = tokio::join!(
            async {
                let unsupported = runner.run(receiver);
                tokio::select! {
                    result = server.run() => result.map(|_| RunOutcome::Completed),
                    result = unsupported => panic!("backend exited before rejection: {result:?}"),
                    _ = complete.notified() => { server.close()?; Ok(RunOutcome::Completed) },
                }
            },
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
