use crate::agent::readiness::{self, Wakeup};
use std::io;
use std::net::Shutdown;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
#[cfg(test)]
use std::time::Duration;

// No waiting queue. Excess connections are closed immediately (transport
// failure, never an acceptance); admitted requests retain their own deadlines.
const MAX_HANDLERS: usize = 8;

use super::socket::{SocketConnection, SocketListener};

pub(in crate::agent) struct ControlServer {
    listener: SocketListener,
    shutdown: AtomicBool,
    wakeup: Wakeup,
}

impl ControlServer {
    pub(in crate::agent) fn new(listener: SocketListener) -> io::Result<Self> {
        Ok(Self {
            listener,
            shutdown: AtomicBool::new(false),
            wakeup: Wakeup::new()?,
        })
    }

    pub(in crate::agent) fn run(
        &self,
        handler: impl Fn(SocketConnection, &AtomicBool) -> io::Result<()> + Sync,
    ) {
        thread::scope(|scope| {
            let mut workers: Vec<(
                thread::ScopedJoinHandle<'_, ()>,
                std::os::unix::net::UnixStream,
            )> = Vec::new();
            while !self.shutdown.load(Ordering::Acquire) {
                let mut fds = [
                    readiness::interest(self.listener.fd(), libc::POLLIN),
                    readiness::interest(self.wakeup.fd(), libc::POLLIN),
                ];
                if let Err(error) = readiness::wait(&mut fds, None) {
                    log::error!("control socket readiness failed: {error}");
                    break;
                }
                if self.shutdown.load(Ordering::Acquire) {
                    break;
                }
                loop {
                    if self.shutdown.load(Ordering::Acquire) {
                        break;
                    }
                    let connection = match self.listener.accept() {
                        Ok(connection) => connection,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        Err(error) => {
                            log::error!("control socket accept failed: {error}");
                            self.shutdown();
                            break;
                        }
                    };
                    let mut index = 0;
                    while index < workers.len() {
                        if workers[index].0.is_finished() {
                            let (worker, _) = workers.swap_remove(index);
                            if worker.join().is_err() {
                                log::error!("control handler panicked");
                            }
                        } else {
                            index += 1;
                        }
                    }
                    if workers.len() == MAX_HANDLERS {
                        // Close without reading an untrusted/possibly incomplete frame.
                        drop(connection);
                        continue;
                    }
                    let Ok(socket) = connection.shutdown_handle() else {
                        continue;
                    };
                    let handler = &handler;
                    match thread::Builder::new()
                        .name("waap-control-request".into())
                        .spawn_scoped(scope, move || {
                            // Never log peer-controlled transport errors.
                            let _ = handler(connection, &self.shutdown);
                        }) {
                        Ok(worker) => workers.push((worker, socket)),
                        Err(error) => log::error!("control handler spawn failed: {error}"),
                    }
                }
            }
            self.shutdown();
            // Wake blocked socket reads/writes before joining. Backend calls are
            // bounded by their acknowledgement deadline (or driver termination).
            for (_, socket) in &workers {
                let _ = socket.shutdown(Shutdown::Both);
            }
            for (worker, _) in workers {
                if worker.join().is_err() {
                    log::error!("control handler panicked");
                }
            }
        });
    }

    pub(in crate::agent) fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        self.wakeup.signal();
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
        let server = ControlServer::new(SocketListener::bind(socket_path).unwrap()).unwrap();
        (repo, runtime, server, path)
    }

    #[test]
    fn idle_shutdown_wakes_readiness_wait() {
        let (_repo, _runtime, server, _path) = fixture();
        thread::scope(|scope| {
            let worker = scope.spawn(|| server.run(|_, _| panic!("unexpected connection")));
            server.shutdown();
            worker.join().unwrap();
        });
    }

    #[test]
    fn slow_clients_are_concurrent_overload_is_closed_and_shutdown_joins_workers() {
        use std::io::Read;
        use std::sync::atomic::AtomicUsize;
        use std::sync::mpsc;
        let (_repo, _runtime, server, path) = fixture();
        let (entered, entries) = mpsc::channel();
        let (finished, finishes) = mpsc::channel();
        let count = AtomicUsize::new(0);
        thread::scope(|scope| {
            let worker = scope.spawn(|| {
                server.run(|mut connection, _| {
                    let index = count.fetch_add(1, Ordering::SeqCst);
                    entered.send(index).unwrap();
                    connection.set_timeouts(Duration::from_secs(2), Duration::from_secs(10))?;
                    let result = connection.read_frame();
                    if index == 1 {
                        assert_eq!(result.unwrap(), b"fast\n");
                        connection.write_frame(b"ok")?;
                        assert!(connection.read_frame().is_err());
                    } else {
                        assert!(result.is_err());
                    }
                    finished.send(()).unwrap();
                    Ok(())
                });
            });
            let _slow = SocketConnection::connect(&path).unwrap();
            assert_eq!(entries.recv_timeout(Duration::from_secs(2)).unwrap(), 0);
            let mut fast = SocketConnection::connect(&path).unwrap();
            fast.set_timeouts(Duration::from_secs(2), Duration::from_secs(2))
                .unwrap();
            fast.write_frame(b"fast").unwrap();
            assert_eq!(entries.recv_timeout(Duration::from_secs(2)).unwrap(), 1);
            assert_eq!(fast.read_frame().unwrap(), b"ok\n");
            let mut slow = Vec::new();
            for _ in 2..MAX_HANDLERS {
                slow.push(SocketConnection::connect(&path).unwrap());
                entries.recv_timeout(Duration::from_secs(2)).unwrap();
            }
            let mut excess = std::os::unix::net::UnixStream::connect(&path).unwrap();
            excess
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            assert_eq!(excess.read(&mut [0]).unwrap(), 0);
            assert_eq!(count.load(Ordering::SeqCst), MAX_HANDLERS);
            let shutdown_started = std::time::Instant::now();
            server.shutdown();
            worker.join().unwrap();
            assert!(shutdown_started.elapsed() < Duration::from_secs(2));
            assert_eq!(finishes.try_iter().count(), MAX_HANDLERS);
        });
    }

    #[test]
    fn serves_raw_frames_without_waap_or_an_agent() {
        let repo = tempfile::tempdir().unwrap();
        init_repo_with_commit(repo.path());
        let runtime = tempfile::tempdir().unwrap();
        std::fs::set_permissions(runtime.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let root = find_control_root_in(repo.path(), Some(runtime.path().as_os_str())).unwrap();
        let socket_path = SocketPath::for_agent(&root, "aa-raw").unwrap();
        let path = socket_path.as_path().to_path_buf();
        let mut server = ControlServer::new(SocketListener::bind(socket_path).unwrap()).unwrap();
        thread::scope(|scope| {
            let server = &server;
            let worker = scope.spawn(|| {
                server.run(|mut connection, _| {
                    connection.set_timeouts(Duration::from_secs(1), Duration::from_secs(1))?;
                    assert_eq!(connection.read_frame()?, b"\xffraw\n");
                    connection.write_frame(b"\xfereply")?;
                    server.shutdown();
                    Ok(())
                })
            });
            let mut client = SocketConnection::connect(&path).unwrap();
            client
                .set_timeouts(Duration::from_secs(1), Duration::from_secs(1))
                .unwrap();
            client.write_frame(b"\xffraw").unwrap();
            assert_eq!(client.read_frame().unwrap(), b"\xfereply\n");
            worker.join().unwrap();
        });
        server.close().unwrap();
        assert!(!path.exists());
    }
}
