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
        UnixStream::connect(path).await.map(Self::new).map_err(|error| {
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
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn exchanges_raw_frames_without_utf8_conversion() {
        let (client, server) = UnixStream::pair().unwrap();
        let mut client = SocketConnection::new(client);
        let mut server = SocketConnection::new(server);
        let ((), ()) = tokio::join!(
            async {
                client.write_frame(b"\xffraw").await.unwrap();
                assert_eq!(client.read_frame(IO_TIMEOUT).await.unwrap(), b"\xfereply\n");
            },
            async {
                assert_eq!(server.read_frame(IO_TIMEOUT).await.unwrap(), b"\xffraw\n");
                server.write_frame(b"\xfereply").await.unwrap();
            }
        );
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn frame_deadline_is_not_extended_by_slow_drip_activity() {
        use tokio::io::AsyncWriteExt;
        let (stream, mut peer) = tokio::net::UnixStream::pair().unwrap();
        let mut connection = SocketConnection::new(stream);
        let started = tokio::time::Instant::now();
        let (result, ()) = tokio::join!(
            async {
                let result = connection.read_frame(IO_TIMEOUT).await;
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
}
