use super::socket::{SocketConnection, SocketListener};
use futures_util::{stream::FuturesUnordered, FutureExt, StreamExt};
use std::future::Future;
use std::io;

const MAX_HANDLERS: usize = 8;

/// Owns all connection futures. Dropping `run` cancels them and closes their
/// sockets; no shutdown messages, detached tasks, or worker joins are needed.
pub(in crate::agent) struct ControlServer {
    listener: SocketListener,
}

impl ControlServer {
    pub(in crate::agent) fn new(listener: SocketListener) -> Self {
        Self { listener }
    }

    pub(in crate::agent) async fn run<F: Future<Output = io::Result<()>>>(
        &self,
        handler: impl Fn(SocketConnection) -> F,
    ) -> io::Result<()> {
        let listener = self.listener.async_listener()?;
        let mut handlers = FuturesUnordered::new();
        loop {
            tokio::select! {
                biased;
                result = handlers.next(), if !handlers.is_empty() => {
                    if matches!(result, Some(Err(_))) { log::error!("control handler panicked"); }
                },
                connection = listener.accept() => {
                    match connection {
                        Ok((stream, _)) if handlers.len() < MAX_HANDLERS => {
                            let handler = &handler;
                            handlers.push(std::panic::AssertUnwindSafe(async move {
                                handler(SocketConnection::new(stream)).await
                            }).catch_unwind());
                        }
                        // No application work queue: overload is a closed connection,
                        // never an acceptance or an implicit retry.
                        Ok((stream, _)) => drop(stream),
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
    use crate::agent::control::socket::SocketPath;
    use crate::agent::control::IO_TIMEOUT;
    use crate::root::find_control_root_in;
    use crate::test_git::init_repo_with_commit;
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;
    use tokio::sync::mpsc;

    fn fixture() -> (
        tempfile::TempDir,
        tempfile::TempDir,
        ControlServer,
        std::path::PathBuf,
    ) {
        let repo = tempfile::tempdir().unwrap();
        init_repo_with_commit(repo.path());
        let runtime = tempfile::tempdir().unwrap();
        std::fs::set_permissions(runtime.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let root = find_control_root_in(repo.path(), Some(runtime.path().as_os_str())).unwrap();
        let socket_path = SocketPath::for_agent(&root, "aa-concurrent").unwrap();
        let path = socket_path.as_path().to_path_buf();
        let server = ControlServer::new(SocketListener::bind(socket_path).unwrap());
        (repo, runtime, server, path)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancellation_of_idle_or_unpolled_server_needs_no_notification() {
        for polled in [false, true] {
            let (_repo, _runtime, mut server, path) = fixture();
            let future = server.run(|_| async { panic!("unexpected connection") });
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
        let (_repo, _runtime, server, path) = fixture();
        let (entered, mut entries) = mpsc::unbounded_channel();
        let (finished, mut finishes) = mpsc::unbounded_channel();
        tokio::time::timeout(Duration::from_secs(2), async {
            tokio::select! {
                result = server.run(|mut connection| {
                    let entered = &entered;
                    let finished = finished.clone();
                    async move {
                        struct Cleanup(mpsc::UnboundedSender<()>);
                        impl Drop for Cleanup { fn drop(&mut self) { self.0.send(()).unwrap(); } }
                        let _cleanup = Cleanup(finished);
                        entered.send(std::thread::current().id()).unwrap();
                        let frame = connection.read_frame(IO_TIMEOUT).await?;
                        connection.write_frame(&frame[..frame.len() - 1]).await?;
                        connection.read_frame(IO_TIMEOUT).await?;
                        Ok(())
                    }
                }) => panic!("server exited: {result:?}"),
                _ = async {
                    let owner = std::thread::current().id();
                    let _slow = UnixStream::connect(&path).await.unwrap();
                    assert_eq!(entries.recv().await.unwrap(), owner);
                    let mut fast = UnixStream::connect(&path).await.unwrap();
                    assert_eq!(entries.recv().await.unwrap(), owner);
                    fast.write_all(b"fast\n").await.unwrap();
                    let mut response = [0; 5]; fast.read_exact(&mut response).await.unwrap();
                    assert_eq!(&response, b"fast\n");
                    let mut slow = Vec::new();
                    for _ in 2..MAX_HANDLERS {
                        slow.push(UnixStream::connect(&path).await.unwrap());
                        assert_eq!(entries.recv().await.unwrap(), owner);
                    }
                    let mut excess = UnixStream::connect(&path).await.unwrap();
                    assert_eq!(excess.read(&mut [0]).await.unwrap(), 0);
                } => {}
            }
        })
        .await
        .unwrap();
        for _ in 0..MAX_HANDLERS {
            finishes.try_recv().unwrap();
        }
        assert!(finishes.try_recv().is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn serves_raw_frames_and_reclaims_completed_slots() {
        let (_repo, _runtime, mut server, path) = fixture();
        tokio::time::timeout(Duration::from_secs(2), async {
            tokio::select! {
                result = server.run(|mut connection| async move {
                    assert_eq!(connection.read_frame(IO_TIMEOUT).await?, b"\xffraw\n");
                    connection.write_frame(b"\xfereply").await
                }) => panic!("server exited: {result:?}"),
                _ = async {
                    for _ in 0..MAX_HANDLERS * 2 {
                        let stream = UnixStream::connect(&path).await.unwrap();
                        let mut client = SocketConnection::new(stream);
                        client.write_frame(b"\xffraw").await.unwrap();
                        assert_eq!(client.read_frame(IO_TIMEOUT).await.unwrap(), b"\xfereply\n");
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
