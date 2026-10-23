use std::io::{self, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};
use uuid::Uuid;

use super::protocol::MAX_FRAME_BYTES;
use super::{read_frame, ACK_TIMEOUT, IO_TIMEOUT};

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
    let mut stream = UnixStream::connect(path).map_err(|error| {
        io::Error::new(error.kind(), format!("cannot connect to control socket {}: {error}; agent may be starting, exited, or have a stale endpoint", path.display()))
    })?;
    exchange(
        &mut stream,
        &frame,
        &request_id,
        IO_TIMEOUT,
        ACK_TIMEOUT + IO_TIMEOUT,
    )
}

fn exchange(
    stream: &mut UnixStream,
    frame: &[u8],
    request_id: &str,
    write_timeout: Duration,
    ack_timeout: Duration,
) -> io::Result<()> {
    stream.set_write_timeout(Some(write_timeout))?;
    stream.set_read_timeout(Some(ack_timeout))?;
    stream.write_all(frame).map_err(|error| {
        unknown(&format!(
            "request write failed (agent may be busy or unavailable): {error}"
        ))
    })?;
    let response = read_frame(stream).map_err(|error| {
        unknown(&format!(
            "acknowledgement failed (agent may be busy or unavailable): {error}"
        ))
    })?;
    validate_response(&response, request_id)
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
    use std::io::{BufRead, BufReader, Read};
    use std::os::unix::net::UnixListener;
    use std::thread;

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
    fn missing_and_stale_endpoints_fail_without_reconnection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("control.sock");
        assert!(submit(&path, "data")
            .unwrap_err()
            .to_string()
            .contains("cannot connect"));
        drop(UnixListener::bind(&path).unwrap());
        assert!(submit(&path, "data")
            .unwrap_err()
            .to_string()
            .contains("stale endpoint"));
    }

    #[test]
    fn missing_acknowledgement_times_out_without_retry() {
        let (mut client, _server) = UnixStream::pair().unwrap();
        let error = exchange(
            &mut client,
            b"request\n",
            "id",
            IO_TIMEOUT,
            Duration::from_millis(20),
        )
        .unwrap_err();
        assert!(error.to_string().contains("acceptance is unknown"));
    }

    #[test]
    fn response_eof_truncation_and_oversize_are_uncertain() {
        for frame in [
            vec![],
            b"{}".to_vec(),
            vec![b'x'; MAX_FRAME_BYTES],
            vec![b'x'; MAX_FRAME_BYTES + 1],
        ] {
            let (mut client, mut server) = UnixStream::pair().unwrap();
            thread::scope(|scope| {
                scope.spawn(move || {
                    let mut request = String::new();
                    BufReader::new(server.try_clone().unwrap())
                        .read_line(&mut request)
                        .unwrap();
                    let _ = server.write_all(&frame);
                });
                assert!(
                    exchange(&mut client, b"request\n", "id", IO_TIMEOUT, ACK_TIMEOUT)
                        .unwrap_err()
                        .to_string()
                        .contains("acceptance is unknown")
                );
            });
        }
    }
}
