use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;

use super::{SocketConnection, SocketPath};

pub(in crate::agent) struct SocketListener {
    listener: Option<UnixListener>,
    path: SocketPath,
}

impl SocketListener {
    pub(in crate::agent) fn bind(path: SocketPath) -> io::Result<Self> {
        path.prepare_parent()?;
        let listener = UnixListener::bind(path.as_path()).map_err(|error| {
            io::Error::new(error.kind(), format!("failed to bind control socket {}: {error}; existing endpoints are not automatically removed", path.as_path().display()))
        })?;
        let listener = Self {
            listener: Some(listener),
            path,
        };
        fs::set_permissions(listener.path.as_path(), fs::Permissions::from_mode(0o600))?;
        listener.listener.as_ref().unwrap().set_nonblocking(true)?;
        Ok(listener)
    }

    pub(in crate::agent) fn accept(&self) -> io::Result<SocketConnection> {
        let (stream, _) = self.listener.as_ref().unwrap().accept()?;
        Ok(SocketConnection::new(stream))
    }

    pub(in crate::agent) fn close(&mut self) -> io::Result<()> {
        if self.listener.take().is_some() {
            fs::remove_file(self.path.as_path())?;
        }
        Ok(())
    }
}

impl Drop for SocketListener {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            log::error!(
                "failed to remove control socket {}: {error}",
                self.path.as_path().display()
            );
        }
    }
}
