use std::io;
use std::path::Path;

use super::protocol::{Request, Response};
use super::socket::SocketConnection;
use super::{ACK_TIMEOUT, IO_TIMEOUT};

pub(in crate::agent) struct ControlClient {
    connection: SocketConnection,
}

impl ControlClient {
    pub(in crate::agent) fn connect(path: &Path) -> io::Result<Self> {
        let connection = SocketConnection::connect(path)?;
        connection.set_timeouts(IO_TIMEOUT, ACK_TIMEOUT + IO_TIMEOUT)?;
        Ok(Self { connection })
    }

    pub(in crate::agent) fn steer(&mut self, prompt: &str) -> io::Result<()> {
        self.exchange(Request::steer(prompt))
    }

    fn exchange(&mut self, request: Request) -> io::Result<()> {
        let payload = serde_json::to_vec(&request)?;
        self.connection.write_frame(&payload).map_err(|error| {
            if error.kind() == io::ErrorKind::InvalidInput {
                error
            } else {
                build_acceptance_unknown_error(&format!(
                    "request write failed (agent may be busy or unavailable): {error}"
                ))
            }
        })?;
        let frame = self.connection.read_frame().map_err(|error| {
            build_acceptance_unknown_error(&format!(
                "acknowledgement failed (agent may be busy or unavailable): {error}"
            ))
        })?;
        validate_response(&frame, &request.request_id)
    }
}

fn validate_response(frame: &[u8], request_id: &str) -> io::Result<()> {
    let invalid = || build_acceptance_unknown_error("invalid or uncorrelated control response");
    let response: Response = serde_json::from_slice(frame).map_err(|_| invalid())?;
    if response.version != 1 || response.request_id.as_deref() != Some(request_id) {
        return Err(invalid());
    }
    match (response.accepted, response.error) {
        (Some(true), None) => Ok(()),
        (accepted, Some(error)) if !error.code.is_empty() && !error.message.is_empty() => {
            match (accepted, error.code.as_str()) {
                (Some(false), code) if code != "acceptance_unknown" => {
                    Err(io::Error::other(format!("{code}: {}", error.message)))
                }
                (None, "acceptance_unknown") => Err(build_acceptance_unknown_error(&error.message)),
                _ => Err(invalid()),
            }
        }
        _ => Err(invalid()),
    }
}

fn build_acceptance_unknown_error(reason: &str) -> io::Error {
    io::Error::other(format!(
        "acceptance is unknown: {reason}; no automatic retry was attempted"
    ))
}

#[cfg(test)]
mod tests {
    use super::super::socket::MAX_FRAME_BYTES;
    use super::*;
    use serde_json::{json, Value};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::thread;
    use std::time::Duration;
    use uuid::Uuid;

    fn submit(path: &Path, prompt: &str) -> io::Result<()> {
        ControlClient::connect(path)?.steer(prompt)
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
        for error in [
            json!({"code": 7, "message": "rejected"}),
            json!({"code": "busy", "message": null}),
            json!({"code": "busy"}),
            json!({"code": "busy", "message": "rejected", "extra": true}),
            json!({"code": "", "message": "rejected"}),
            json!({"code": "busy", "message": ""}),
        ] {
            let mut response = rejected.clone();
            response["error"] = error;
            invalid.push(response);
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
        let (mut oversized, _) = listener.accept().unwrap();
        assert_eq!(
            oversized.read(&mut [0]).unwrap(),
            0,
            "oversized request must not be written"
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
        let error = submit(&path, "data").unwrap_err();
        assert!(error.to_string().contains("stale endpoint"), "{error}");
    }

    #[test]
    fn missing_acknowledgement_times_out_without_retry() {
        let (client, _server) = UnixStream::pair().unwrap();
        let mut socket = ControlClient {
            connection: SocketConnection::new(client),
        };
        socket
            .connection
            .set_timeouts(IO_TIMEOUT, Duration::from_millis(20))
            .unwrap();
        let error = socket.exchange(Request::steer("request")).unwrap_err();
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
            let (client, mut server) = UnixStream::pair().unwrap();
            let mut socket = ControlClient {
                connection: SocketConnection::new(client),
            };
            socket
                .connection
                .set_timeouts(IO_TIMEOUT, ACK_TIMEOUT)
                .unwrap();
            thread::scope(|scope| {
                scope.spawn(move || {
                    let mut request = String::new();
                    BufReader::new(server.try_clone().unwrap())
                        .read_line(&mut request)
                        .unwrap();
                    let _ = server.write_all(&frame);
                });
                assert!(socket
                    .exchange(Request::steer("request"))
                    .unwrap_err()
                    .to_string()
                    .contains("acceptance is unknown"));
            });
        }
    }
}
