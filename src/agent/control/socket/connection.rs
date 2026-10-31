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

/// Server-side connection. Timeouts bound whole frames, including slow-drip clients.
pub(in crate::agent) struct AsyncSocketConnection {
    stream: tokio::net::UnixStream,
}

impl AsyncSocketConnection {
    pub(in crate::agent) fn new(stream: tokio::net::UnixStream) -> Self {
        Self { stream }
    }

    pub(in crate::agent) async fn read_frame(&mut self) -> io::Result<Vec<u8>> {
        use tokio::io::{AsyncBufReadExt, AsyncReadExt};
        tokio::time::timeout(crate::agent::control::IO_TIMEOUT, async {
            let mut frame = Vec::new();
            tokio::io::BufReader::new((&mut self.stream).take(MAX_FRAME_BYTES as u64))
                .read_until(b'\n', &mut frame)
                .await?;
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
        })
        .await
        .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?
    }

    pub(in crate::agent) async fn write_frame(&mut self, payload: &[u8]) -> io::Result<()> {
        use tokio::io::AsyncWriteExt;
        if payload.len() >= MAX_FRAME_BYTES {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        tokio::time::timeout(crate::agent::control::IO_TIMEOUT, async {
            self.stream.write_all(payload).await?;
            self.stream.write_all(b"\n").await
        })
        .await
        .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn async_frame_deadline_is_not_extended_by_slow_drip_activity() {
        use tokio::io::AsyncWriteExt;
        let (stream, mut peer) = tokio::net::UnixStream::pair().unwrap();
        let mut connection = AsyncSocketConnection::new(stream);
        let started = tokio::time::Instant::now();
        let (result, ()) = tokio::join!(
            async {
                let result = connection.read_frame().await;
                assert_eq!(started.elapsed(), crate::agent::control::IO_TIMEOUT);
                result
            },
            async {
                peer.write_all(b"a").await.unwrap();
                tokio::time::sleep(crate::agent::control::IO_TIMEOUT / 2).await;
                peer.write_all(b"b").await.unwrap();
                tokio::time::sleep(crate::agent::control::IO_TIMEOUT).await;
            }
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
    }

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
