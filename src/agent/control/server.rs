use std::future::Future;
use std::io;

use futures_util::{stream::FuturesUnordered, FutureExt, StreamExt};
use tokio::sync::watch;

use super::socket::{AsyncSocketConnection, SocketListener};

// No application work queue: excess connections are closed without acceptance.
const MAX_HANDLERS: usize = 8;

pub(in crate::agent) struct ControlServer {
    listener: SocketListener,
    shutdown: watch::Sender<bool>,
}

impl ControlServer {
    pub(in crate::agent) fn new(listener: SocketListener) -> Self {
        let (shutdown, _) = watch::channel(false);
        Self { listener, shutdown }
    }

    pub(in crate::agent) async fn run<F: Future<Output = io::Result<()>>>(
        &self,
        handler: impl Fn(AsyncSocketConnection) -> F,
    ) {
        let listener = match self.listener.async_listener() {
            Ok(listener) => listener,
            Err(error) => {
                log::error!("control socket registration failed: {error}");
                return;
            }
        };
        let mut shutdown = self.shutdown.subscribe();
        let mut handlers = FuturesUnordered::new();
        while !*shutdown.borrow() {
            tokio::select! {
                biased;
                _ = shutdown.changed() => break,
                // Reap completed handlers before admitting new connections.
                result = handlers.next(), if !handlers.is_empty() => {
                    if matches!(result, Some(Err(_))) { log::error!("control handler panicked"); }
                },
                connection = listener.accept() => {
                    match connection {
                        Ok((stream, _)) if handlers.len() < MAX_HANDLERS => {
                            let handler = &handler;
                            handlers.push(std::panic::AssertUnwindSafe(async move {
                                handler(AsyncSocketConnection::new(stream)).await
                            }).catch_unwind());
                        }
                        Ok((stream, _)) => drop(stream),
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {},
                        Err(error) => {
                            log::error!("control socket accept failed: {error}");
                            break;
                        }
                    }
                }
            }
        }
        // These futures are owned here, not detached tasks. Dropping them cancels
        // pending reads, writes and reply waits and closes every active socket.
        drop(handlers);
    }

    pub(in crate::agent) fn shutdown(&self) {
        self.shutdown.send_replace(true);
    }

    pub(in crate::agent) fn close(&mut self) -> io::Result<()> {
        self.listener.close()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::control::socket::SocketPath;
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
    async fn idle_shutdown_wakes_accept_and_early_shutdown_is_retained() {
        for early in [false, true] {
            let (_repo, _runtime, server, path) = fixture();
            let dispatched = std::cell::Cell::new(false);
            let _queued = if early {
                let stream = UnixStream::connect(&path).await.unwrap();
                server.shutdown();
                Some(stream)
            } else {
                None
            };
            tokio::time::timeout(Duration::from_secs(1), async {
                tokio::join!(
                    server.run(|_| async {
                        dispatched.set(true);
                        Ok(())
                    }),
                    async {
                        tokio::task::yield_now().await;
                        server.shutdown();
                    }
                );
            })
            .await
            .unwrap();
            assert!(!dispatched.get());
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn slow_clients_do_not_serialize_handlers_overload_closes_and_shutdown_cancels() {
        let (_repo, _runtime, server, path) = fixture();
        let (entered, mut entries) = mpsc::unbounded_channel();
        let (finished, mut finishes) = mpsc::unbounded_channel();
        tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(
                server.run(|mut connection| {
                    let entered = &entered;
                    let finished = finished.clone();
                    async move {
                        // Dropped even when shutdown cancels this future mid-request.
                        struct Cleanup(mpsc::UnboundedSender<()>);
                        impl Drop for Cleanup {
                            fn drop(&mut self) {
                                self.0.send(()).unwrap();
                            }
                        }
                        let _cleanup = Cleanup(finished);
                        entered.send(std::thread::current().id()).unwrap();
                        let frame = connection.read_frame().await?;
                        connection.write_frame(&frame[..frame.len() - 1]).await?;
                        // Keep the fast handler admitted while checking overload.
                        connection.read_frame().await?;
                        Ok(())
                    }
                }),
                async {
                    let owner = std::thread::current().id();
                    let _slow = UnixStream::connect(&path).await.unwrap();
                    assert_eq!(entries.recv().await.unwrap(), owner);
                    let mut fast = UnixStream::connect(&path).await.unwrap();
                    assert_eq!(entries.recv().await.unwrap(), owner);
                    fast.write_all(b"fast\n").await.unwrap();
                    let mut response = [0; 5];
                    fast.read_exact(&mut response).await.unwrap();
                    assert_eq!(&response, b"fast\n");
                    let mut slow = Vec::new();
                    for _ in 2..MAX_HANDLERS {
                        slow.push(UnixStream::connect(&path).await.unwrap());
                        assert_eq!(entries.recv().await.unwrap(), owner);
                    }
                    let mut excess = UnixStream::connect(&path).await.unwrap();
                    assert_eq!(excess.read(&mut [0]).await.unwrap(), 0);
                    server.shutdown();
                }
            );
        })
        .await
        .unwrap();
        for _ in 0..MAX_HANDLERS {
            finishes.try_recv().unwrap();
        }
        assert!(finishes.try_recv().is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn serves_raw_frames_and_reclaims_completed_handler_slots() {
        let (_repo, _runtime, mut server, path) = fixture();
        tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(
                server.run(|mut connection| async move {
                    assert_eq!(connection.read_frame().await?, b"\xffraw\n");
                    connection.write_frame(b"\xfereply").await
                }),
                async {
                    for _ in 0..MAX_HANDLERS * 2 {
                        let stream = UnixStream::connect(&path).await.unwrap();
                        let mut client = AsyncSocketConnection::new(stream);
                        client.write_frame(b"\xffraw").await.unwrap();
                        assert_eq!(client.read_frame().await.unwrap(), b"\xfereply\n");
                    }
                    server.shutdown();
                }
            );
        })
        .await
        .unwrap();
        server.close().unwrap();
        assert!(!path.exists());
    }
}
