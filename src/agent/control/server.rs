use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use super::socket::{SocketConnection, SocketListener};

pub(in crate::agent) struct ControlServer {
    listener: SocketListener,
    shutdown: AtomicBool,
}

impl ControlServer {
    pub(in crate::agent) fn new(listener: SocketListener) -> Self {
        Self {
            listener,
            shutdown: AtomicBool::new(false),
        }
    }

    pub(in crate::agent) fn run(
        &self,
        handler: impl Fn(SocketConnection, &AtomicBool) -> io::Result<()>,
    ) {
        while !self.shutdown.load(Ordering::Acquire) {
            match self.listener.accept() {
                Ok(connection) => {
                    // One request at a time. Never log peer-controlled transport errors.
                    let _ = handler(connection, &self.shutdown);
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
    }

    pub(in crate::agent) fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
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

    #[test]
    fn serves_raw_frames_without_waap_or_an_agent() {
        let repo = tempfile::tempdir().unwrap();
        init_repo_with_commit(repo.path());
        let runtime = tempfile::tempdir().unwrap();
        std::fs::set_permissions(runtime.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let root = find_control_root_in(repo.path(), Some(runtime.path().as_os_str())).unwrap();
        let socket_path = SocketPath::for_agent(&root, "aa-raw").unwrap();
        let path = socket_path.as_path().to_path_buf();
        let mut server = ControlServer::new(SocketListener::bind(socket_path).unwrap());
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
