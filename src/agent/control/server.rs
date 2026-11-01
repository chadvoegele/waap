use super::super::commands::CommandDispatcher;
use super::handler::ControlRequestHandler;
use super::socket::{SocketConnection, SocketListener};
use futures_util::{stream::FuturesUnordered, FutureExt, StreamExt};
use std::io;

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

    pub(in crate::agent) async fn run(&self) -> io::Result<()> {
        let handler = ControlRequestHandler::new(self.dispatcher.clone());
        let mut handlers = FuturesUnordered::new();
        loop {
            tokio::select! {
                biased;
                result = handlers.next(), if !handlers.is_empty() => {
                    if matches!(result, Some(Err(_))) { log::error!("control handler panicked"); }
                },
                connection = self.listener.accept() => {
                    match connection {
                        Ok(stream) if handlers.len() < MAX_HANDLERS => {
                            handlers.push(std::panic::AssertUnwindSafe(
                                handler.handle(SocketConnection::new(stream))
                            ).catch_unwind());
                        }
                        Ok(stream) => drop(stream),
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {},
                        Err(error) => return Err(error),
                    }
                }
            }
        }
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
    use crate::agent::control::socket::SocketPath;
    use crate::agent::control::IO_TIMEOUT;
    use crate::root::find_control_root_in;
    use crate::test_git::init_repo_with_commit;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;

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
                    let Command::Steer(mut command) = commands.recv().await.unwrap();
                    assert_eq!(command.message, request.prompt);
                    command.respond(Ok(()));
                    let response = read_response(&mut fast).await;
                    assert_eq!(response.request_id, Some(request.request_id));
                    assert_eq!(response.accepted, Some(true));
                    for index in 1..MAX_HANDLERS {
                        let (request, client) = send_request(&path, &format!("held {index}")).await;
                        let Command::Steer(command) = commands.recv().await.unwrap();
                        assert_eq!(command.message, request.prompt);
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
                        let Command::Steer(command) = commands.recv().await.unwrap();
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
                    let Command::Steer(mut command) = commands.recv().await.unwrap();
                    assert_eq!(command.message, request.prompt);
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
                        let Command::Steer(mut command) = commands.recv().await.unwrap();
                        assert_eq!(command.message, request.prompt);
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
