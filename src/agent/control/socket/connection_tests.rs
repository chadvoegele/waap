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
