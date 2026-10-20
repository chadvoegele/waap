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
mod tests;
