use std::io;
use std::time::Instant;

use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

const MAX_RECORD_BYTES: usize = 64 * 1024 * 1024;

pub(super) struct PiClient<R, W, O> {
    reader: R,
    writer: Option<W>,
    output: O,
    buffer: Vec<u8>,
    next_id: u64,
}

impl<R: AsyncBufRead + Unpin, W: AsyncWrite + Unpin, O: AsyncWrite + Unpin> PiClient<R, W, O> {
    pub(super) fn new(reader: R, writer: W, output: O) -> Self {
        Self {
            reader,
            writer: Some(writer),
            output,
            buffer: Vec::new(),
            next_id: 1,
        }
    }

    pub(super) async fn send(
        &mut self,
        command: &str,
        fields: Value,
        deadline: Instant,
    ) -> io::Result<String> {
        let id = format!("waap-{}", self.next_id);
        self.next_id += 1;
        let mut value = fields
            .as_object()
            .expect("command fields must be an object")
            .clone();
        value.insert("id".into(), json!(id));
        value.insert("type".into(), json!(command));
        self.write(&Value::Object(value), deadline).await?;
        Ok(id)
    }

    async fn write(&mut self, value: &Value, deadline: Instant) -> io::Result<()> {
        let mut bytes = serde_json::to_vec(value)?;
        bytes.push(b'\n');
        tokio::time::timeout_at(deadline.into(), async {
            let writer = self.writer.as_mut().ok_or_else(|| {
                io::Error::new(io::ErrorKind::BrokenPipe, "Pi RPC stdin is closed")
            })?;
            writer.write_all(&bytes).await?;
            writer.flush().await
        })
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "Pi RPC write timed out; acceptance is unknown",
            )
        })?
    }

    pub(super) async fn receive(&mut self) -> io::Result<Value> {
        self.read_record().await?.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Pi RPC closed before acknowledgement or agent_settled",
            )
        })
    }

    pub(super) async fn drain_shutdown(&mut self) -> io::Result<()> {
        while self.read_record().await?.is_some() {}
        Ok(())
    }

    async fn read_record(&mut self) -> io::Result<Option<Value>> {
        loop {
            let available = self.reader.fill_buf().await?;
            if available.is_empty() {
                if self.buffer.is_empty() {
                    return Ok(None);
                }
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Pi RPC sent an unterminated JSONL record",
                ));
            }
            let newline = available.iter().position(|byte| *byte == b'\n');
            let count = newline.map_or(available.len(), |index| index + 1);
            if self.buffer.len() + count > MAX_RECORD_BYTES {
                return Err(build_protocol_error(
                    "Pi RPC record exceeds the 64 MiB native record limit",
                ));
            }
            self.buffer.extend_from_slice(&available[..count]);
            self.reader.consume(count);
            if newline.is_some() {
                break;
            }
        }
        let record = serde_json::from_slice(&self.buffer)
            .map_err(|error| build_protocol_error(format!("Pi RPC sent malformed JSON: {error}")));
        self.buffer.clear();
        let value: Value = record?;
        if !value.is_object() || value.get("type").and_then(Value::as_str).is_none() {
            return Err(build_protocol_error(
                "Pi RPC record must be an object with a type",
            ));
        }
        Ok(Some(value))
    }

    pub(super) async fn fetch_session_id(&mut self, deadline: Instant) -> io::Result<String> {
        tokio::time::timeout_at(deadline.into(), async {
            let id = self.send("get_state", json!({}), deadline).await?;
            loop {
                let record = self.receive().await?;
                if record["type"] == "response" && record["id"] == id {
                    validate_response(&record, "get_state")?.map_err(io::Error::other)?;
                    return record["data"]["sessionId"]
                        .as_str()
                        .filter(|id| !id.trim().is_empty())
                        .map(str::to_owned)
                        .ok_or_else(|| {
                            build_protocol_error("Pi get_state response is missing data.sessionId")
                        });
                }
                self.handle_event(&record, deadline).await?;
            }
        })
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "timed out waiting for Pi get_state response",
            )
        })?
    }

    pub(super) async fn handle_event(
        &mut self,
        record: &Value,
        deadline: Instant,
    ) -> io::Result<()> {
        match record["type"].as_str() {
            Some("extension_ui_request") => self.handle_ui(record, deadline).await,
            Some("message_update") if record["assistantMessageEvent"]["type"] == "text_delta" => {
                let text = record["assistantMessageEvent"]["delta"]
                    .as_str()
                    .ok_or_else(|| build_protocol_error("Pi text_delta event is missing delta"))?;
                self.output.write_all(text.as_bytes()).await?;
                self.output.flush().await
            }
            _ => Ok(()),
        }
    }

    async fn handle_ui(&mut self, record: &Value, deadline: Instant) -> io::Result<()> {
        match record["method"].as_str() {
            Some("select" | "confirm" | "input" | "editor") => {
                let id = record["id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| build_protocol_error("Pi extension dialog is missing id"))?;
                self.write(
                    &json!({"type":"extension_ui_response", "id":id, "cancelled":true}),
                    deadline,
                )
                .await
            }
            Some("notify" | "setStatus" | "setWidget" | "setTitle" | "set_editor_text") => Ok(()),
            method => Err(build_protocol_error(format!(
                "Pi extension UI request has unknown method {method:?}"
            ))),
        }
    }

    pub(super) fn close_stdin(&mut self) {
        drop(self.writer.take());
    }
}

pub(super) fn validate_response(record: &Value, command: &str) -> io::Result<Result<(), String>> {
    if record["type"] != "response" || record["command"] != command {
        return Err(build_protocol_error(format!(
            "Pi response command mismatch: expected {command}"
        )));
    }
    match record["success"].as_bool() {
        Some(true) => Ok(Ok(())),
        Some(false) => Ok(Err(record["error"]
            .as_str()
            .unwrap_or("unspecified error")
            .into())),
        None => Err(build_protocol_error(format!(
            "Pi {command} response is missing success"
        ))),
    }
}

pub(super) fn build_protocol_error(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{duplex, BufReader};

    #[tokio::test]
    async fn test_framing_preserves_partial_reads_crlf_unicode_and_rejects_bad_records() {
        let (input, mut peer) = duplex(1024);
        let mut client = PiClient::new(BufReader::new(input), Vec::new(), Vec::new());
        peer.write_all(b"{\"type\":").await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(1), client.receive())
                .await
                .is_err()
        );
        peer.write_all("\"event\",\"text\":\"a\u{2028}b\u{2029}c\"}\r\n".as_bytes())
            .await
            .unwrap();
        assert_eq!(
            client.receive().await.unwrap()["text"],
            "a\u{2028}b\u{2029}c"
        );
        for bad in ["\n", "invalid\n", "null\n", "{}\n"] {
            peer.write_all(bad.as_bytes()).await.unwrap();
            assert_eq!(
                client.receive().await.unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
        peer.write_all(b"{\"type\":\"partial\"}").await.unwrap();
        drop(peer);
        assert_eq!(
            client.receive().await.unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }

    #[tokio::test]
    async fn test_shutdown_validates_records_without_forwarding_text_or_replying_to_ui() {
        let input = concat!(
            "{\"type\":\"extension_ui_request\",\"method\":\"confirm\",\"id\":\"shutdown\"}\n",
            "{\"type\":\"message_update\",\"assistantMessageEvent\":{\"type\":\"text_delta\",\"delta\":\"shutdown text\"}}\n"
        );
        let mut client = PiClient::new(BufReader::new(input.as_bytes()), Vec::new(), Vec::new());
        client.close_stdin();
        client.drain_shutdown().await.unwrap();
        assert!(client.output.is_empty());

        for (input, expected) in [
            ("not-json\n", io::ErrorKind::InvalidData),
            ("{}\n", io::ErrorKind::InvalidData),
            ("{\"type\":\"partial\"}", io::ErrorKind::UnexpectedEof),
        ] {
            let mut client =
                PiClient::new(BufReader::new(input.as_bytes()), Vec::new(), Vec::new());
            client.close_stdin();
            assert_eq!(client.drain_shutdown().await.unwrap_err().kind(), expected);
        }
    }

    #[tokio::test]
    async fn test_dialogs_are_cancelled_notifications_ignored_and_only_text_is_forwarded() {
        let mut client = PiClient::new(BufReader::new(&b""[..]), Vec::new(), Vec::new());
        let deadline = Instant::now() + Duration::from_secs(1);
        for method in [
            "select",
            "confirm",
            "input",
            "editor",
            "notify",
            "setStatus",
            "setWidget",
            "setTitle",
            "set_editor_text",
        ] {
            client
                .handle_event(
                    &json!({"type":"extension_ui_request", "method":method, "id":method}),
                    deadline,
                )
                .await
                .unwrap();
        }
        assert_eq!(
            client
                .writer
                .as_ref()
                .unwrap()
                .split(|byte| *byte == b'\n')
                .filter(|line| !line.is_empty())
                .count(),
            4
        );
        for line in client
            .writer
            .as_ref()
            .unwrap()
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
        {
            assert_eq!(
                serde_json::from_slice::<Value>(line).unwrap()["cancelled"],
                true
            );
        }
        for kind in ["text_delta", "thinking_delta"] {
            client.handle_event(&json!({"type":"message_update", "assistantMessageEvent":{"type":kind,"delta":"snow 雪"}}), deadline).await.unwrap();
        }
        assert_eq!(String::from_utf8(client.output).unwrap(), "snow 雪");
    }

    #[tokio::test]
    async fn test_native_snapshots_exceed_control_frame_size_but_parser_memory_is_bounded() {
        let mut bytes =
            serde_json::to_vec(&json!({"type":"snapshot", "data":"x".repeat(70 * 1024)})).unwrap();
        bytes.push(b'\n');
        let mut client = PiClient::new(BufReader::new(bytes.as_slice()), Vec::new(), Vec::new());
        assert_eq!(
            client.receive().await.unwrap()["data"]
                .as_str()
                .unwrap()
                .len(),
            70 * 1024
        );
        let mut client = PiClient::new(
            BufReader::new(tokio::io::repeat(b'x')),
            Vec::new(),
            Vec::new(),
        );
        assert!(client
            .receive()
            .await
            .unwrap_err()
            .to_string()
            .contains("64 MiB"));
        assert_eq!(client.buffer.len(), MAX_RECORD_BYTES);
    }

    #[tokio::test]
    async fn test_startup_correlates_authentic_session_and_rejects_missing_session_rejection_eof_and_timeout(
    ) {
        let deadline = Instant::now() + Duration::from_secs(1);
        let input = concat!(
            "{\"type\":\"response\",\"id\":\"unrelated\",\"command\":\"get_state\",\"success\":true}\n",
            "{\"type\":\"response\",\"id\":\"waap-1\",\"command\":\"get_state\",\"success\":true,\"data\":{\"sessionId\":\"real-session\"}}\n"
        );
        let mut client = PiClient::new(BufReader::new(input.as_bytes()), Vec::new(), Vec::new());
        assert_eq!(
            client.fetch_session_id(deadline).await.unwrap(),
            "real-session"
        );
        let command: Value = serde_json::from_slice(client.writer.as_ref().unwrap()).unwrap();
        assert_eq!(command["type"], "get_state");
        for record in [
            json!({"type":"response", "id":"waap-1", "command":"get_state", "success":true, "data":{}}),
            json!({"type":"response", "id":"waap-1", "command":"get_state", "success":false, "error":"denied"}),
            json!({"type":"response", "id":"waap-1", "command":"prompt", "success":true}),
        ] {
            let bytes = format!("{record}\n");
            let mut client =
                PiClient::new(BufReader::new(bytes.as_bytes()), Vec::new(), Vec::new());
            assert!(client.fetch_session_id(deadline).await.is_err());
        }
        let mut client = PiClient::new(BufReader::new(&b""[..]), Vec::new(), Vec::new());
        assert_eq!(
            client.fetch_session_id(deadline).await.unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        let (input, _peer) = duplex(128);
        let mut client = PiClient::new(BufReader::new(input), Vec::new(), Vec::new());
        assert_eq!(
            client
                .fetch_session_id(Instant::now() + Duration::from_millis(10))
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::TimedOut
        );
    }

    #[test]
    fn test_responses_distinguish_rejection_from_invalid_acknowledgement() {
        assert!(validate_response(
            &json!({"type":"response","command":"steer","success":false,"error":"denied"}),
            "steer"
        )
        .unwrap()
        .is_err());
        for record in [
            json!({"type":"response","command":"abort","success":true}),
            json!({"type":"response","command":"steer"}),
        ] {
            assert!(validate_response(&record, "steer").is_err());
        }
    }
}
