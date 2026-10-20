use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use uuid::Uuid;

use super::protocol::MAX_FRAME_BYTES;
use super::ACK_TIMEOUT;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);

pub(in crate::agent) fn submit(path: &Path, prompt: &str) -> io::Result<()> {
    let request_id = Uuid::new_v4().to_string();
    let mut frame = serde_json::to_vec(&json!({
        "version": 1, "request_id": request_id, "method": "steer", "prompt": prompt,
    }))?;
    frame.push(b'\n');
    if frame.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "request_too_large: steering request exceeds 64 KiB including JSON and newline",
        ));
    }
    let mut stream = connect(path, Instant::now() + CONNECT_TIMEOUT).map_err(|error| {
        io::Error::new(error.kind(), format!("cannot connect to control socket {}: {error}; agent may be starting, exited, or have a stale endpoint", path.display()))
    })?;
    exchange(&mut stream, &frame, &request_id, WRITE_TIMEOUT, ACK_TIMEOUT)
}

fn connect(path: &Path, deadline: Instant) -> io::Result<UnixStream> {
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    let bytes = path.as_os_str().as_bytes();
    if bytes.len() >= address.sun_path.len() || bytes.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid control socket path",
        ));
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (target, byte) in address.sun_path.iter_mut().zip(bytes) {
        *target = *byte as libc::c_char;
    }
    let fd = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let stream = unsafe { UnixStream::from_raw_fd(fd) };
    let result = unsafe {
        libc::connect(
            fd,
            &address as *const _ as *const libc::sockaddr,
            std::mem::size_of_val(&address) as libc::socklen_t,
        )
    };
    if result == 0 {
        return Ok(stream);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() != Some(libc::EINPROGRESS) {
        return Err(error);
    }
    wait_ready(&stream, libc::POLLOUT, deadline)?;
    if let Some(error) = stream.take_error()? {
        return Err(error);
    }
    Ok(stream)
}

fn exchange(
    stream: &mut UnixStream,
    frame: &[u8],
    request_id: &str,
    write_timeout: Duration,
    ack_timeout: Duration,
) -> io::Result<()> {
    write_frame(stream, frame, Instant::now() + write_timeout)
        .map_err(|error| unknown(&format!("request write failed: {error}")))?;
    let response = read_frame(stream, Instant::now() + ack_timeout)
        .map_err(|error| unknown(&format!("acknowledgement failed: {error}")))?;
    validate_response(&response, request_id)
}

fn write_frame(stream: &mut UnixStream, mut frame: &[u8], deadline: Instant) -> io::Result<()> {
    while !frame.is_empty() {
        wait_ready(stream, libc::POLLOUT, deadline)?;
        match stream.write(frame) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "control socket closed",
                ))
            }
            Ok(count) => frame = &frame[count..],
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn read_frame(stream: &mut UnixStream, deadline: Instant) -> io::Result<Vec<u8>> {
    let mut frame = Vec::new();
    loop {
        wait_ready(stream, libc::POLLIN, deadline)?;
        let mut buffer = [0; 4096];
        match stream.read(&mut buffer) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "control socket closed before a complete response",
                ))
            }
            Ok(count) => {
                frame.extend_from_slice(&buffer[..count]);
                if frame.len() > MAX_FRAME_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "response exceeds 64 KiB",
                    ));
                }
                if let Some(end) = frame.iter().position(|byte| *byte == b'\n') {
                    if end + 1 != frame.len() {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "expected one response frame",
                        ));
                    }
                    return Ok(frame);
                }
                if frame.len() == MAX_FRAME_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "unterminated response exceeds frame limit",
                    ));
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error),
        }
    }
}

fn wait_ready(stream: &UnixStream, events: libc::c_short, deadline: Instant) -> io::Result<()> {
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::TimedOut, "control socket deadline exceeded")
            })?;
        let mut descriptor = libc::pollfd {
            fd: stream.as_raw_fd(),
            events,
            revents: 0,
        };
        let milliseconds = remaining
            .as_millis()
            .saturating_add(1)
            .min(i32::MAX as u128) as i32;
        let ready = unsafe { libc::poll(&mut descriptor, 1, milliseconds) };
        if ready > 0 {
            return Ok(());
        }
        if ready < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
}

fn validate_response(frame: &[u8], request_id: &str) -> io::Result<()> {
    let invalid = || unknown("invalid or uncorrelated control response");
    let value: Value = serde_json::from_slice(frame).map_err(|_| invalid())?;
    let object = value.as_object().ok_or_else(invalid)?;
    if value["version"].as_u64() != Some(1) || value["request_id"].as_str() != Some(request_id) {
        return Err(invalid());
    }
    if value["accepted"] == true && object.len() == 3 && !object.contains_key("error") {
        return Ok(());
    }
    let error = value["error"].as_object().ok_or_else(invalid)?;
    let code = error
        .get("code")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(invalid)?;
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(invalid)?;
    if object.len() != 4 || error.len() != 2 || !object.contains_key("accepted") {
        return Err(invalid());
    }
    match (&value["accepted"], code) {
        (Value::Bool(false), code) if code != "acceptance_unknown" => {
            Err(io::Error::other(format!("{code}: {message}")))
        }
        (Value::Null, "acceptance_unknown") => Err(unknown(message)),
        _ => Err(invalid()),
    }
}

fn unknown(reason: &str) -> io::Error {
    io::Error::other(format!(
        "acceptance is unknown: {reason}; no automatic retry was attempted"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixListener;
    use std::thread;

    fn pair() -> (UnixStream, UnixStream) {
        let (client, server) = UnixStream::pair().unwrap();
        client.set_nonblocking(true).unwrap();
        server
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        (client, server)
    }

    #[test]
    fn submit_preserves_prompt_and_uses_one_correlated_version_one_request() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("control.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let prompt = "  Unicode 雪\n\"quoted\" '$HOME' `echo data`  ";
        thread::scope(|scope| {
            let server = scope.spawn(|| {
                let (mut stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                let value: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(value.as_object().unwrap().len(), 4);
                assert_eq!(value["version"], 1);
                assert_eq!(value["method"], "steer");
                assert_eq!(value["prompt"], prompt);
                assert_eq!(
                    Uuid::parse_str(value["request_id"].as_str().unwrap())
                        .unwrap()
                        .get_version_num(),
                    4
                );
                writeln!(
                    stream,
                    "{}",
                    json!({"version": 1, "request_id": value["request_id"], "accepted": true})
                )
                .unwrap();
                let mut byte = [0];
                assert_eq!(stream.read(&mut byte).unwrap(), 0);
            });
            submit(&path, prompt).unwrap();
            server.join().unwrap();
        });
    }

    #[test]
    fn response_schema_correlation_and_acceptance_are_validated() {
        let id = Uuid::new_v4().to_string();
        let accepted = json!({"version": 1, "request_id": id, "accepted": true});
        validate_response(&serde_json::to_vec(&accepted).unwrap(), &id).unwrap();
        let rejected = json!({"version": 1, "request_id": id, "accepted": false,
            "error": {"code": "busy", "message": "Steering queue is full."}});
        let error = validate_response(&serde_json::to_vec(&rejected).unwrap(), &id)
            .unwrap_err()
            .to_string();
        assert!(error.contains("busy: Steering queue is full."));
        assert!(!error.contains("unknown"));
        let uncertain = json!({"version": 1, "request_id": id, "accepted": null,
            "error": {"code": "acceptance_unknown", "message": "Backend acknowledgement lost."}});
        assert!(
            validate_response(&serde_json::to_vec(&uncertain).unwrap(), &id)
                .unwrap_err()
                .to_string()
                .contains("acceptance is unknown")
        );
        let mut invalid = vec![json!(null), json!({}), rejected.clone(), uncertain.clone()];
        invalid[2]["error"]["code"] = json!("acceptance_unknown");
        invalid[3]["error"]["code"] = json!("timeout");
        for (key, value) in [
            ("version", json!(2)),
            ("request_id", json!(Uuid::new_v4().to_string())),
            ("request_id", Value::Null),
            ("accepted", json!("true")),
            ("accepted", json!(false)),
            ("accepted", Value::Null),
            ("error", json!({})),
            ("extra", json!(true)),
        ] {
            let mut value_with_error = accepted.clone();
            value_with_error[key] = value;
            invalid.push(value_with_error);
        }
        let mut missing = uncertain;
        missing.as_object_mut().unwrap().remove("accepted");
        invalid.push(missing);
        for value in invalid {
            assert!(
                validate_response(&serde_json::to_vec(&value).unwrap(), &id)
                    .unwrap_err()
                    .to_string()
                    .contains("acceptance is unknown"),
                "{value}"
            );
        }
        for frame in [b"bad JSON".as_slice(), b"\xff\n"] {
            assert!(validate_response(frame, &id)
                .unwrap_err()
                .to_string()
                .contains("unknown"));
        }
    }

    #[test]
    fn request_frame_limit_includes_json_and_newline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("control.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let overhead = serde_json::to_vec(&json!({"version": 1, "request_id": Uuid::new_v4().to_string(), "method": "steer", "prompt": ""})).unwrap().len() + 1;
        let prompt = "x".repeat(MAX_FRAME_BYTES - overhead);
        thread::scope(|scope| {
            let server = scope.spawn(|| {
                let (mut stream, _) = listener.accept().unwrap();
                let mut frame = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut frame)
                    .unwrap();
                assert_eq!(frame.len(), MAX_FRAME_BYTES);
                let value: Value = serde_json::from_str(&frame).unwrap();
                writeln!(
                    stream,
                    "{}",
                    json!({"version": 1, "request_id": value["request_id"], "accepted": true})
                )
                .unwrap();
            });
            submit(&path, &prompt).unwrap();
            server.join().unwrap();
        });
        listener.set_nonblocking(true).unwrap();
        assert!(submit(&path, &(prompt + "x"))
            .unwrap_err()
            .to_string()
            .contains("request_too_large"));
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn missing_stale_and_saturated_endpoints_fail_without_reconnection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("control.sock");
        assert!(submit(&path, "data")
            .unwrap_err()
            .to_string()
            .contains("cannot connect"));
        drop(UnixListener::bind(&path).unwrap());
        let start = Instant::now();
        assert!(submit(&path, "data")
            .unwrap_err()
            .to_string()
            .contains("stale endpoint"));
        assert!(start.elapsed() < CONNECT_TIMEOUT);
        std::fs::remove_file(&path).unwrap();
        let listener = UnixListener::bind(&path).unwrap();
        assert_eq!(unsafe { libc::listen(listener.as_raw_fd(), 0) }, 0);
        let _queued = connect(&path, Instant::now() + CONNECT_TIMEOUT).unwrap();
        let start = Instant::now();
        assert!(connect(&path, start + CONNECT_TIMEOUT).is_err());
        assert!(start.elapsed() < CONNECT_TIMEOUT);
    }

    #[test]
    fn acknowledgement_deadline_is_absolute_and_includes_server_queue_wait() {
        let (mut client, mut server) = pair();
        thread::scope(|scope| {
            scope.spawn(move || {
                let mut request = String::new();
                BufReader::new(server.try_clone().unwrap())
                    .read_line(&mut request)
                    .unwrap();
                for _ in 0..20 {
                    if server.write_all(b" ").is_err() {
                        break;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
            });
            let start = Instant::now();
            let error = exchange(
                &mut client,
                b"request\n",
                "id",
                WRITE_TIMEOUT,
                Duration::from_millis(35),
            )
            .unwrap_err();
            assert!(error.to_string().contains("acceptance is unknown"));
            assert!(error.to_string().contains("deadline exceeded"));
            assert!(start.elapsed() >= Duration::from_millis(35));
            assert!(start.elapsed() < Duration::from_secs(1));
        });
    }

    #[test]
    fn writes_to_nonreading_peer_have_a_deadline() {
        let (mut client, _server) = pair();
        let size: libc::c_int = 1024;
        assert_eq!(
            unsafe {
                libc::setsockopt(
                    client.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_SNDBUF,
                    &size as *const _ as *const libc::c_void,
                    std::mem::size_of_val(&size) as libc::socklen_t,
                )
            },
            0
        );
        let error = exchange(
            &mut client,
            &vec![b'x'; MAX_FRAME_BYTES],
            "id",
            Duration::from_millis(30),
            ACK_TIMEOUT,
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("acceptance is unknown: request write failed"));
        assert!(error.to_string().contains("deadline exceeded"));
    }

    #[test]
    fn response_eof_truncation_and_oversize_are_uncertain() {
        for frame in [
            vec![],
            b"{}".to_vec(),
            vec![b'x'; MAX_FRAME_BYTES],
            vec![b'x'; MAX_FRAME_BYTES + 1],
            b"{}\n{}\n".to_vec(),
        ] {
            let (mut client, mut server) = pair();
            thread::scope(|scope| {
                scope.spawn(move || {
                    let mut request = String::new();
                    BufReader::new(server.try_clone().unwrap())
                        .read_line(&mut request)
                        .unwrap();
                    let _ = server.write_all(&frame);
                });
                assert!(
                    exchange(&mut client, b"request\n", "id", WRITE_TIMEOUT, ACK_TIMEOUT)
                        .unwrap_err()
                        .to_string()
                        .contains("acceptance is unknown")
                );
            });
        }
    }

    #[test]
    fn expired_readiness_deadline_fails_even_if_socket_is_ready() {
        let (client, _server) = pair();
        assert_eq!(
            wait_ready(&client, libc::POLLOUT, Instant::now())
                .unwrap_err()
                .kind(),
            io::ErrorKind::TimedOut
        );
        assert_eq!(CONNECT_TIMEOUT, Duration::from_secs(2));
        assert_eq!(ACK_TIMEOUT, Duration::from_secs(30));
    }

    #[test]
    fn acknowledgement_budget_starts_after_complete_send() {
        let (mut client, mut server) = pair();
        let size: libc::c_int = 1024;
        assert_eq!(
            unsafe {
                libc::setsockopt(
                    client.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_SNDBUF,
                    &size as *const _ as *const libc::c_void,
                    std::mem::size_of_val(&size) as libc::socklen_t,
                )
            },
            0
        );
        thread::scope(|scope| {
            scope.spawn(move || {
                thread::sleep(Duration::from_millis(60));
                let mut frame = vec![0; MAX_FRAME_BYTES];
                server.read_exact(&mut frame).unwrap();
                writeln!(
                    server,
                    "{}",
                    json!({"version":1,"request_id":"id","accepted":true})
                )
                .unwrap();
            });
            let start = Instant::now();
            exchange(
                &mut client,
                &vec![b'x'; MAX_FRAME_BYTES],
                "id",
                WRITE_TIMEOUT,
                Duration::from_millis(30),
            )
            .unwrap();
            assert!(start.elapsed() >= Duration::from_millis(60));
        });
    }
}
