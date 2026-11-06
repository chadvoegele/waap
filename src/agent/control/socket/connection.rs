use std::io;
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use super::MAX_FRAME_BYTES;
use crate::agent::control::IO_TIMEOUT;

pub(in crate::agent::control) struct SocketConnection {
    stream: UnixStream,
}

impl SocketConnection {
    pub(in crate::agent::control) fn new(stream: UnixStream) -> Self {
        Self { stream }
    }

    pub(in crate::agent::control) async fn connect(path: &Path) -> io::Result<Self> {
        tokio::time::timeout(IO_TIMEOUT, UnixStream::connect(path))
            .await
            .unwrap_or_else(|_| Err(io::Error::new(io::ErrorKind::TimedOut, "control connection deadline expired")))
            .map(Self::new).map_err(|error| {
            io::Error::new(error.kind(), format!("cannot connect to control socket {}: {error}; agent may be starting, exited, or have a stale endpoint", path.display()))
        })
    }

    pub(in crate::agent::control) async fn read_frame(
        &mut self,
        timeout: Duration,
    ) -> io::Result<Vec<u8>> {
        tokio::time::timeout(timeout, async {
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

    pub(in crate::agent::control) async fn write_frame(
        &mut self,
        payload: &[u8],
    ) -> io::Result<()> {
        if payload.len() >= MAX_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "request_too_large: control frame exceeds 64 KiB including JSON and newline",
            ));
        }
        tokio::time::timeout(IO_TIMEOUT, async {
            self.stream.write_all(payload).await?;
            self.stream.write_all(b"\n").await
        })
        .await
        .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?
    }
}

#[cfg(test)]
#[path = "connection_tests.rs"]
mod tests;
