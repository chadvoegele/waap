use super::super::commands::CommandDispatcher;
use super::protocol::{build_rejection, build_response, parse_request};
use super::socket::{SocketConnection, SocketListener};
use super::{ACK_TIMEOUT, IO_TIMEOUT};
use futures_util::{stream::FuturesUnordered, FutureExt, StreamExt};
use std::io;
use std::time::Instant;

const MAX_HANDLERS: usize = 8;

pub(in crate::agent) struct ControlServer {
    listener: SocketListener,
    dispatcher: CommandDispatcher,
}

impl ControlServer {
    pub(in crate::agent) fn new(listener: SocketListener, dispatcher: CommandDispatcher) -> Self {
        Self {
            listener,
            dispatcher,
        }
    }

    #[cfg(test)]
    pub(in crate::agent) async fn run(&self) -> io::Result<()> {
        self.run_until(std::future::pending()).await
    }

    pub(in crate::agent) async fn run_until(
        &self,
        shutdown: impl std::future::Future<Output = ()>,
    ) -> io::Result<()> {
        tokio::pin!(shutdown);
        let mut handlers = FuturesUnordered::new();
        loop {
            tokio::select! {
                biased;
                result = handlers.next(), if !handlers.is_empty() => {
                    if matches!(result, Some(Err(_))) { log::error!("control handler panicked"); }
                },
                _ = &mut shutdown => break,
                connection = self.listener.accept() => {
                    match connection {
                        Ok(stream) if handlers.len() < MAX_HANDLERS => {
                            handlers.push(std::panic::AssertUnwindSafe(
                                self.handle(SocketConnection::new(stream))
                            ).catch_unwind());
                        }
                        Ok(stream) => drop(stream),
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {},
                        Err(error) => return Err(error),
                    }
                }
            }
        }
        // Each handler already has bounded read, dispatch, and write deadlines.
        while handlers.next().await.is_some() {}
        Ok(())
    }

    async fn handle(&self, mut connection: SocketConnection) -> io::Result<()> {
        let response = match connection.read_frame(IO_TIMEOUT).await {
            Ok(frame) => match parse_request(&frame) {
                Ok(request) => {
                    let deadline = Instant::now() + ACK_TIMEOUT;
                    let steering_result = if request.method == "stop" {
                        self.dispatcher.stop(deadline).await
                    } else {
                        self.dispatcher
                            .steer(request.prompt.as_deref().unwrap(), deadline)
                            .await
                    };
                    build_response(&request.request_id, steering_result)
                }
                Err(response) => response,
            },
            Err(error) if error.kind() == io::ErrorKind::InvalidData => build_rejection(
                None,
                "request_too_large",
                "Request exceeds 64 KiB including newline.",
            ),
            Err(error) => return Err(error),
        };
        connection
            .write_frame(&serde_json::to_vec(&response)?)
            .await
    }

    pub(in crate::agent) fn close(&mut self) -> io::Result<()> {
        self.listener.close()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::backend::SteeringError;
    use crate::agent::commands::Command;
    use crate::agent::control::protocol::{Request, Response};
    use crate::agent::control::socket::{SocketPath, MAX_FRAME_BYTES};
    use crate::root::find_control_root_in;
    use crate::test_git::init_repo_with_commit;
    use serde_json::{json, Value};
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;

    #[tokio::test(flavor = "current_thread")]
    async fn completion_drains_correlated_stop_response_before_closing_connection() {
        let (_repo, _runtime, control, path, mut commands) = fixture().await;
        let (shutdown, stopped) = tokio::sync::oneshot::channel();
        let (server, client, ()) = tokio::join!(
            control.run_until(async {
                let _ = stopped.await;
            }),
            async {
                let mut client = crate::agent::control::client::ControlClient::connect(&path)
                    .await
                    .unwrap();
                client.stop().await
            },
            async {
                let Command::Stop(mut request) = commands.recv().await.unwrap() else {
                    panic!("expected stop")
                };
                request.respond(Ok(()));
                shutdown.send(()).unwrap();
            }
        );
        server.unwrap();
        client.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn unsupported_backend_rejection_is_forwarded() {
        let request = Request::steer("steer");
        let id = request.request_id.clone();
        let (_repo, _runtime, control, _path, mut commands) = fixture().await;
        let (server, client) = tokio::net::UnixStream::pair().unwrap();
        let mut client = SocketConnection::new(client);
        let (result, response, ()) = tokio::join!(
            control.handle(SocketConnection::new(server)),
            async {
                client
                    .write_frame(&serde_json::to_vec(&request).unwrap())
                    .await
                    .unwrap();
                serde_json::from_slice::<Response>(&client.read_frame(IO_TIMEOUT).await.unwrap())
                    .unwrap()
            },
            async {
                let crate::agent::commands::Command::Steer(mut request) =
                    commands.recv().await.unwrap()
                else {
                    panic!("expected steering command")
                };
                request.respond(Err(crate::agent::backend::SteeringError::not_implemented(
                    "claude",
                )));
            }
        );
        result.unwrap();
        assert_eq!(response.request_id.as_deref(), Some(id.as_str()));
        assert_eq!(response.accepted, Some(false));
        assert_eq!(response.error.unwrap().code, "not_implemented");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn socket_rejects_invalid_schemas_without_backend_dispatch_or_prompt_disclosure() {
        let secret = "private prompt";
        let valid = serde_json::to_value(Request::steer(secret)).unwrap();
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
        let (_repo, _runtime, control, _path, mut commands) = fixture().await;
        for (bytes, code, trusted_id) in cases {
            let (server, mut client) = UnixStream::pair().unwrap();
            let (result, value) =
                tokio::join!(control.handle(SocketConnection::new(server)), async {
                    client.write_all(&bytes).await.unwrap();
                    serde_json::from_slice::<Value>(
                        &SocketConnection::new(client)
                            .read_frame(IO_TIMEOUT)
                            .await
                            .unwrap(),
                    )
                    .unwrap()
                });
            result.unwrap();
            assert!(commands.try_recv().is_err());
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
        }
    }

    async fn fixture() -> (
        tempfile::TempDir,
        tempfile::TempDir,
        ControlServer,
        std::path::PathBuf,
        tokio::sync::mpsc::Receiver<Command>,
    ) {
        let repo = tempfile::tempdir().unwrap();
        init_repo_with_commit(repo.path());
        let runtime = tempfile::tempdir().unwrap();
        std::fs::set_permissions(runtime.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let root = find_control_root_in(repo.path(), Some(runtime.path().as_os_str())).unwrap();
        let socket_path = SocketPath::for_agent(&root, "aa-concurrent").unwrap();
        let path = socket_path.as_path().to_path_buf();
        let (dispatcher, commands) = CommandDispatcher::channel();
        let server = ControlServer::new(SocketListener::bind(socket_path).unwrap(), dispatcher);
        (repo, runtime, server, path, commands)
    }

    async fn send_request(path: &Path, prompt: &str) -> (Request, SocketConnection) {
        let request = Request::steer(prompt);
        let mut client = SocketConnection::new(UnixStream::connect(path).await.unwrap());
        client
            .write_frame(&serde_json::to_vec(&request).unwrap())
            .await
            .unwrap();
        (request, client)
    }

    async fn read_response(client: &mut SocketConnection) -> Response {
        serde_json::from_slice(&client.read_frame(IO_TIMEOUT).await.unwrap()).unwrap()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancellation_of_idle_or_unpolled_server_needs_no_notification() {
        for polled in [false, true] {
            let (_repo, _runtime, mut server, path, _commands) = fixture().await;
            let future = server.run();
            if polled {
                assert!(future.now_or_never().is_none());
            } else {
                drop(future);
            }
            server.close().unwrap();
            assert!(!path.exists());
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn slow_clients_are_concurrent_overload_closes_and_cancellation_drops_handlers() {
        let (_repo, _runtime, mut server, path, mut commands) = fixture().await;
        let mut clients = Vec::new();
        let mut pending = Vec::new();
        tokio::time::timeout(Duration::from_secs(2), async {
            tokio::select! {
                result = server.run() => panic!("server exited: {result:?}"),
                _ = async {
                    let mut slow = UnixStream::connect(&path).await.unwrap();
                    slow.write_all(b"{").await.unwrap();
                    clients.push(SocketConnection::new(slow));
                    let (request, mut fast) = send_request(&path, "fast").await;
                    let Command::Steer(mut command) = commands.recv().await.unwrap() else { panic!("expected steering command") };
                    assert_eq!(Some(command.message.as_str()), request.prompt.as_deref());
                    command.respond(Ok(()));
                    let response = read_response(&mut fast).await;
                    assert_eq!(response.request_id, Some(request.request_id));
                    assert_eq!(response.accepted, Some(true));
                    for index in 1..MAX_HANDLERS {
                        let (request, client) = send_request(&path, &format!("held {index}")).await;
                        let Command::Steer(command) = commands.recv().await.unwrap() else { panic!("expected steering command") };
                        assert_eq!(Some(command.message.as_str()), request.prompt.as_deref());
                        pending.push(command);
                        clients.push(client);
                    }
                    let mut excess = UnixStream::connect(&path).await.unwrap();
                    assert_eq!(excess.read(&mut [0]).await.unwrap(), 0);
                    assert!(commands.try_recv().is_err());
                } => {}
            }
        })
        .await
        .unwrap();
        for command in &mut pending {
            command.respond(Ok(()));
        }
        for mut client in clients {
            assert_eq!(
                client.read_frame(IO_TIMEOUT).await.unwrap_err().kind(),
                io::ErrorKind::UnexpectedEof
            );
        }
        server.close().unwrap();
        assert!(!path.exists());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn completed_handlers_are_reaped_before_admitting_backlogged_connections() {
        let (_repo, _runtime, mut server, path, mut commands) = fixture().await;
        tokio::time::timeout(Duration::from_secs(2), async {
            let future = server.run();
            tokio::pin!(future);
            let mut clients = Vec::new();
            let mut pending = Vec::new();
            tokio::select! {
                result = &mut future => panic!("server exited: {result:?}"),
                _ = async {
                    for _ in 0..MAX_HANDLERS {
                        let (_, client) = send_request(&path, "held").await;
                        let Command::Steer(command) = commands.recv().await.unwrap() else { panic!("expected steering command") };
                        clients.push(client);
                        pending.push(command);
                    }
                } => {}
            }
            let (request, mut backlogged) = send_request(&path, "backlogged").await;
            pending[0].respond(Ok(()));
            tokio::select! {
                result = &mut future => panic!("server exited: {result:?}"),
                _ = async {
                    let Command::Steer(mut command) = commands.recv().await.unwrap() else { panic!("expected steering command") };
                    assert_eq!(Some(command.message.as_str()), request.prompt.as_deref());
                    command.respond(Ok(()));
                    let response = read_response(&mut backlogged).await;
                    assert_eq!(response.request_id, Some(request.request_id));
                    assert_eq!(response.accepted, Some(true));
                } => {}
            }
        })
        .await
        .unwrap();
        server.close().unwrap();
        assert!(!path.exists());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn serves_one_protocol_request_and_reclaims_completed_slots() {
        let (_repo, _runtime, mut server, path, mut commands) = fixture().await;
        tokio::time::timeout(Duration::from_secs(2), async {
            tokio::select! {
                result = server.run() => panic!("server exited: {result:?}"),
                _ = async {
                    for index in 0..MAX_HANDLERS * 2 {
                        let (request, mut client) = send_request(&path, &format!("request {index}")).await;
                        let Command::Steer(mut command) = commands.recv().await.unwrap() else { panic!("expected steering command") };
                        assert_eq!(Some(command.message.as_str()), request.prompt.as_deref());
                        let (reply, accepted, code) = match index % 3 {
                            0 => (Ok(()), Some(true), None),
                            1 => (
                                Err(SteeringError::Rejected {
                                    code: "busy",
                                    message: "pending acknowledgement".into(),
                                }),
                                Some(false),
                                Some("busy"),
                            ),
                            _ => (
                                Err(SteeringError::acceptance_unknown("lost acknowledgement")),
                                None,
                                Some("acceptance_unknown"),
                            ),
                        };
                        command.respond(reply);
                        let response = read_response(&mut client).await;
                        assert_eq!(response.version, 1);
                        assert_eq!(response.request_id, Some(request.request_id));
                        assert_eq!(response.accepted, accepted);
                        assert_eq!(response.error.as_ref().map(|error| error.code.as_str()), code);
                        assert_eq!(
                            client.read_frame(IO_TIMEOUT).await.unwrap_err().kind(),
                            io::ErrorKind::UnexpectedEof
                        );
                        assert!(commands.try_recv().is_err());
                    }
                } => {}
            }
        })
        .await
        .unwrap();
        server.close().unwrap();
        assert!(!path.exists());
    }
}
