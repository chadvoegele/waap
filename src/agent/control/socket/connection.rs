use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use super::MAX_FRAME_BYTES;

pub(in crate::agent) struct SocketConnection {
    stream: UnixStream,
}

impl SocketConnection {
    pub(in crate::agent) fn new(stream: UnixStream) -> Self {
        Self { stream }
    }

    pub(in crate::agent) fn connect(path: &Path) -> io::Result<Self> {
        UnixStream::connect(path).map(Self::new).map_err(|error| {
            io::Error::new(error.kind(), format!("cannot connect to control socket {}: {error}; agent may be starting, exited, or have a stale endpoint", path.display()))
        })
    }

    pub(in crate::agent) fn set_timeouts(&self, write: Duration, read: Duration) -> io::Result<()> {
        self.stream.set_write_timeout(Some(write))?;
        self.stream.set_read_timeout(Some(read))
    }

    pub(in crate::agent) fn read_frame(&mut self) -> io::Result<Vec<u8>> {
        // Each connection carries one exchange; bound allocation before reading.
        let mut frame = Vec::new();
        BufReader::new((&mut self.stream).take(MAX_FRAME_BYTES as u64))
            .read_until(b'\n', &mut frame)?;
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

    pub(in crate::agent) fn write_frame(&mut self, payload: &[u8]) -> io::Result<()> {
        if payload.len() >= MAX_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "request_too_large: control frame exceeds 64 KiB including JSON and newline",
            ));
        }
        self.stream.write_all(payload)?;
        self.stream.write_all(b"\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_sets_timeouts() {
        let (stream, _peer) = UnixStream::pair().unwrap();
        let connection = SocketConnection::new(stream);
        let write = Duration::from_secs(5);
        let read = Duration::from_secs(35);
        connection.set_timeouts(write, read).unwrap();
        assert_eq!(connection.stream.write_timeout().unwrap(), Some(write));
        assert_eq!(connection.stream.read_timeout().unwrap(), Some(read));
    }
}
