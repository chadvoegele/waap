use std::collections::VecDeque;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::Duration;

use super::*;

type SteeringResult = Result<(), SteeringError>;

pub(super) struct SteeringMailbox {
    sender: SyncSender<SteerRequest>,
}

pub(super) struct SteerRequest {
    prompt: String,
    deadline: Instant,
    reply: Option<SyncSender<SteeringResult>>,
}

fn reject(code: &'static str, message: &str) -> SteeringError {
    SteeringError::Rejected {
        code,
        message: message.to_string(),
    }
}

fn unknown(reason: &str) -> SteeringError {
    SteeringError::AcceptanceUnknown {
        message: format!(
            "Codex steering acceptance is unknown: {reason}; do not retry automatically"
        ),
    }
}

impl SteeringMailbox {
    pub(super) fn channel() -> (Self, Receiver<SteerRequest>) {
        let (sender, receiver) = mpsc::sync_channel(1);
        (Self { sender }, receiver)
    }

    pub(super) fn steer(&self, prompt: &str, deadline: Instant) -> SteeringResult {
        if Instant::now() >= deadline {
            return Err(reject(
                "timeout",
                "Steering request expired before Codex dispatch",
            ));
        }
        let (reply, replies) = mpsc::sync_channel(1);
        let request = SteerRequest {
            prompt: prompt.to_string(),
            deadline,
            reply: Some(reply),
        };
        self.sender.try_send(request).map_err(|error| match error {
            mpsc::TrySendError::Full(_) => reject("busy", "Codex steering dispatch is busy"),
            mpsc::TrySendError::Disconnected(_) => reject(
                "agent_exited",
                "Codex protocol owner exited before steering dispatch",
            ),
        })?;
        replies
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_else(|_| {
                Err(unknown(
                    "no reply received before the deadline or driver exit",
                ))
            })
    }
}

impl SteerRequest {
    fn respond(&mut self, result: SteeringResult) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(result);
        }
    }
}

pub(super) fn set_nonblocking(pipe: &impl AsRawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_GETFL) };
    if flags < 0
        || unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

struct PendingWrite {
    id: i64,
    bytes: Vec<u8>,
    offset: usize,
}

struct PendingSteer {
    id: i64,
    request: SteerRequest,
    dispatched: bool,
}

impl PendingSteer {
    fn expire(&mut self, now: Instant) {
        if now >= self.request.deadline && self.request.reply.is_some() {
            self.request.respond(Err(if self.dispatched {
                unknown("acknowledgement deadline expired")
            } else {
                reject("timeout", "Steering request expired before Codex dispatch")
            }));
        }
    }
}

struct LiveTurn<'a> {
    thread_id: &'a str,
    start_id: i64,
    turn_id: Option<String>,
    early_completion: Option<(String, io::Result<TurnStatus>)>,
    pending: Option<PendingSteer>,
    writes: VecDeque<PendingWrite>,
    interrupt_id: Option<i64>,
}

impl<R: BufRead, W: Write, O: Write> CodexClient<R, W, O> {
    pub(super) fn run_turn(
        &mut self,
        thread_id: &str,
        prompt: &str,
        interrupt: &AtomicBool,
        commands: Receiver<SteerRequest>,
    ) -> io::Result<TurnStatus> {
        let mut turn = LiveTurn {
            thread_id,
            start_id: self.next_id,
            turn_id: None,
            early_completion: None,
            pending: None,
            writes: VecDeque::new(),
            interrupt_id: None,
        };
        self.enqueue(
            &mut turn,
            METHOD_TURN_START,
            turn_start_params(
                thread_id,
                prompt,
                self.model.as_deref(),
                self.reasoning_effort,
            ),
        );
        let mut buffer = Vec::new();
        loop {
            if let Some(pending) = &mut turn.pending {
                pending.expire(Instant::now());
            }
            if let Ok(request) = commands.try_recv() {
                self.dispatch_steering(&mut turn, request);
            }
            if turn.interrupt_id.is_none() && interrupt.load(Ordering::SeqCst) {
                if let Some(turn_id) = &turn.turn_id {
                    let params = json!({ "threadId": thread_id, "turnId": turn_id });
                    turn.interrupt_id =
                        Some(self.enqueue(&mut turn, METHOD_TURN_INTERRUPT, params));
                }
            }
            self.flush_requests(&mut turn)?;
            let mut chunk = [0; 8192];
            match self.reader.read(&mut chunk) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "codex app-server closed before the turn completed",
                    ))
                }
                Ok(count) => buffer.extend_from_slice(&chunk[..count]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
            if let Some(status) = self.process_frames(&mut turn, &mut buffer)? {
                return status;
            }
        }
    }

    fn process_frames(
        &mut self,
        turn: &mut LiveTurn<'_>,
        buffer: &mut Vec<u8>,
    ) -> io::Result<Option<io::Result<TurnStatus>>> {
        let mut consumed = 0;
        while let Some(end) = buffer[consumed..].iter().position(|byte| *byte == b'\n') {
            let end = consumed + end + 1;
            let line = &buffer[consumed..end];
            consumed = end;
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let message = serde_json::from_slice(line).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "codex app-server sent malformed JSON",
                )
            })?;
            if let Some(status) = self.process_message(turn, &message)? {
                return Ok(Some(status));
            }
        }
        buffer.drain(..consumed);
        Ok(None)
    }

    fn dispatch_steering(&mut self, turn: &mut LiveTurn<'_>, mut request: SteerRequest) {
        if Instant::now() >= request.deadline {
            request.respond(Err(reject(
                "timeout",
                "Steering request expired before Codex dispatch",
            )));
            return;
        }
        let Some(turn_id) = turn.turn_id.as_deref() else {
            request.respond(Err(reject(
                "no_active_turn",
                "Codex has no active turn to steer",
            )));
            return;
        };
        if turn.pending.is_some() {
            request.respond(Err(reject(
                "busy",
                "Codex is still awaiting an earlier steering acknowledgement",
            )));
            return;
        }
        let params = json!({
            "threadId": turn.thread_id,
            "expectedTurnId": turn_id,
            "input": [{ "type": "text", "text": request.prompt }],
        });
        let id = self.enqueue(turn, METHOD_TURN_STEER, params);
        turn.pending = Some(PendingSteer {
            id,
            request,
            dispatched: false,
        });
    }

    fn enqueue(&mut self, turn: &mut LiveTurn<'_>, method: &str, params: JsonValue) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        turn.writes.push_back(PendingWrite {
            id,
            bytes: request_line(id, method, params).into_bytes(),
            offset: 0,
        });
        id
    }

    fn flush_requests(&mut self, turn: &mut LiveTurn<'_>) -> io::Result<()> {
        while !turn.writes.is_empty() {
            if let Some(pending) = &mut turn.pending {
                pending.expire(Instant::now());
            }
            let write = turn.writes.front_mut().unwrap();
            let pending = turn
                .pending
                .as_mut()
                .filter(|pending| pending.id == write.id);
            if write.offset == 0
                && pending
                    .as_ref()
                    .is_some_and(|pending| pending.request.reply.is_none())
            {
                turn.writes.pop_front();
                turn.pending.take();
                continue;
            }
            match self.writer.write(&write.bytes[write.offset..]) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "Codex app-server input closed",
                    ))
                }
                Ok(count) => {
                    if let Some(pending) = pending {
                        pending.dispatched = true;
                    }
                    write.offset += count;
                    if write.offset == write.bytes.len() {
                        turn.writes.pop_front();
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn process_message(
        &mut self,
        turn: &mut LiveTurn<'_>,
        message: &JsonValue,
    ) -> io::Result<Option<io::Result<TurnStatus>>> {
        if message.get("method").is_some() {
            return self.process_notification(turn, message);
        }
        let Some(id) = message.get("id").and_then(JsonValue::as_i64) else {
            return Ok(None);
        };
        if id == turn.start_id {
            return self.accept_started_turn(turn, message);
        }
        if turn
            .pending
            .as_ref()
            .is_some_and(|pending| pending.id == id)
        {
            let mut pending = turn.pending.take().unwrap();
            pending.expire(Instant::now());
            pending
                .request
                .respond(steering_response(message, turn.turn_id.as_deref().unwrap()));
        } else if Some(id) == turn.interrupt_id && message.get("error").is_some() {
            return Err(io::Error::other("codex turn/interrupt rejected"));
        }
        Ok(None)
    }

    fn process_notification(
        &mut self,
        turn: &mut LiveTurn<'_>,
        message: &JsonValue,
    ) -> io::Result<Option<io::Result<TurnStatus>>> {
        if message.get("id").is_some() {
            return Ok(None);
        }
        if let Some(turn_id) = &turn.turn_id {
            if let Some(delta) = delta_for_turn(message, turn.thread_id, turn_id) {
                self.out.write_all(delta.as_bytes())?;
                self.out.flush()?;
            }
            return Ok(completed_status_for_turn(message, turn.thread_id, turn_id));
        }
        if message
            .pointer("/params/threadId")
            .and_then(JsonValue::as_str)
            != Some(turn.thread_id)
        {
            return Ok(None);
        }
        self.forward_delta(message)?;
        if let Some(id) = message
            .pointer("/params/turn/id")
            .and_then(JsonValue::as_str)
        {
            if let Some(status) = completed_status_for_turn(message, turn.thread_id, id) {
                turn.early_completion = Some((id.to_string(), status));
            }
        }
        Ok(None)
    }

    fn accept_started_turn(
        &self,
        turn: &mut LiveTurn<'_>,
        message: &JsonValue,
    ) -> io::Result<Option<io::Result<TurnStatus>>> {
        if let Some(error) = message.get("error") {
            return Err(io::Error::other(format!(
                "codex turn/start rejected (code {})",
                error["code"]
            )));
        }
        let turn_id = message
            .pointer("/result/turn/id")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "codex turn/start response is missing turn.id",
                )
            })?;
        turn.turn_id = Some(turn_id.to_string());
        if let Some((id, status)) = turn.early_completion.take() {
            if id == turn_id {
                return Ok(Some(status));
            }
        }
        Ok(None)
    }
}

fn steering_response(message: &JsonValue, expected_turn: &str) -> SteeringResult {
    if let Some(error) = message.get("error") {
        let (Some(code), Some(diagnostic)) = (
            error.get("code").and_then(JsonValue::as_i64),
            error.get("message").and_then(JsonValue::as_str),
        ) else {
            return Err(unknown("the server returned an invalid turn/steer error"));
        };
        if message.get("result").is_some() {
            return Err(unknown("the server returned both a result and an error"));
        }
        let missing = code == -32601
            || (code == -32600
                && (diagnostic.starts_with("Invalid request: unknown variant `turn/steer`")
                    || diagnostic.starts_with("unknown variant `turn/steer`")));
        return Err(if missing {
            reject("unsupported_backend_version", "Codex app-server does not support turn/steer; upgrade Codex to a version with native turn/steer support")
        } else {
            // The server's message can echo submitted input; keep routine diagnostics prompt-free.
            reject("backend_rejected", &format!("Codex rejected turn/steer (RPC error code {code}); verify that the turn is still active"))
        });
    }
    if message
        .pointer("/result/turnId")
        .and_then(JsonValue::as_str)
        != Some(expected_turn)
    {
        return Err(unknown(
            "the server returned an invalid turn/steer acknowledgement",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;

    use super::*;

    type TestClient = CodexClient<BufReader<UnixStream>, UnixStream, CapturedOutput>;

    #[derive(Clone, Default)]
    struct CapturedOutput(Arc<Mutex<Vec<u8>>>);

    impl Write for CapturedOutput {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct TestMailbox {
        steering: SteeringMailbox,
        finished: AtomicBool,
        output: CapturedOutput,
    }

    impl TestMailbox {
        fn steer(&self, prompt: &str, deadline: Instant) -> SteeringResult {
            self.steering.steer(prompt, deadline)
        }
    }

    struct Server(BufReader<UnixStream>);

    impl Server {
        fn read(&mut self) -> JsonValue {
            let mut line = String::new();
            self.0.read_line(&mut line).unwrap();
            serde_json::from_str(&line).unwrap()
        }

        fn send(&mut self, message: JsonValue) {
            writeln!(self.0.get_mut(), "{message}").unwrap();
        }

        fn start(&mut self, mailbox: &TestMailbox) {
            let request = self.read();
            assert_eq!(request["method"], METHOD_TURN_START);
            assert_eq!(request["params"]["input"][0]["text"], "initial");
            self.send(json!({"id":request["id"],"result":{"turn":{"id":"tu"}}}));
            // Synchronize the test through a normal output event, not production shared state.
            self.send(json!({"method":METHOD_AGENT_MESSAGE_DELTA,"params":{
                "threadId":"th","turnId":"tu","delta":"__ready__"}}));
            wait_until(|| mailbox.output.0.lock().unwrap().ends_with(b"__ready__"));
            mailbox.output.0.lock().unwrap().clear();
        }

        fn complete(&mut self, status: &str) {
            self.send(json!({"method":METHOD_TURN_COMPLETED,"params":{
                "threadId":"th","turn":{"id":"tu","status":status}}}));
        }

        fn accept(&mut self, id: &JsonValue) {
            self.send(json!({"id":id,"result":{"turnId":"tu"}}));
        }
    }

    fn wait_until(mut predicate: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !predicate() {
            assert!(Instant::now() < deadline, "condition did not become true");
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn with_run(
        action: impl FnOnce(&mut Server, &TestMailbox, &AtomicBool, &CapturedOutput),
    ) -> (io::Result<TurnStatus>, String) {
        let (stream, server) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        server
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let output = CapturedOutput::default();
        let mut client = TestClient {
            reader: BufReader::new(stream.try_clone().unwrap()),
            writer: stream,
            out: output.clone(),
            next_id: 0,
            model: None,
            reasoning_effort: None,
        };
        let (steering, commands) = SteeringMailbox::channel();
        let mailbox = TestMailbox {
            steering,
            finished: AtomicBool::new(false),
            output: output.clone(),
        };
        let interrupt = AtomicBool::new(false);
        let result = thread::scope(|scope| {
            let worker = scope.spawn(|| {
                let result = client.run_turn("th", "initial", &interrupt, commands);
                mailbox.finished.store(true, Ordering::Release);
                result
            });
            let mut server = Server(BufReader::new(server));
            action(&mut server, &mailbox, &interrupt, &output);
            drop(server);
            worker.join().unwrap()
        });
        assert!(mailbox.finished.load(Ordering::Acquire));
        assert_rejected(
            mailbox.steer("after completion", deadline()),
            "agent_exited",
        );
        let output = String::from_utf8(output.0.lock().unwrap().clone()).unwrap();
        (result, output)
    }

    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(2)
    }

    fn assert_rejected(result: SteeringResult, code: &str) {
        assert!(
            matches!(&result, Err(SteeringError::Rejected { code: actual, .. }) if *actual == code),
            "{result:?}"
        );
    }

    #[test]
    fn native_steering_preserves_payload_and_correlates_replies_amid_output() {
        let prompt = "  steer\nUnicode λ 🦀 'quotes' \" $(touch /never) ; & ";
        let (status, output) = with_run(|server, mailbox, _, _| {
            assert_rejected(mailbox.steer(prompt, deadline()), "no_active_turn");
            server.start(mailbox);
            thread::scope(|scope| {
                let steering = scope.spawn(|| mailbox.steer(prompt, deadline()));
                let request = server.read();
                assert_eq!(request["method"], METHOD_TURN_STEER);
                assert_eq!(
                    request["params"],
                    json!({"threadId":"th","expectedTurnId":"tu",
                    "input":[{"type":"text","text":prompt}]})
                );
                assert_ne!(request["id"], 0);
                server.send(json!({"id":999,"result":{"turnId":"wrong"}}));
                server.send(json!({"method":"unrelated","params":{}}));
                server.send(json!({"method":METHOD_AGENT_MESSAGE_DELTA,"id":99,
                    "params":{"threadId":"th","turnId":"tu","delta":"server request"}}));
                for (thread, turn, text) in [
                    ("other", "tu", "bad"),
                    ("th", "other", "bad"),
                    ("th", "tu", "hello"),
                ] {
                    server.send(json!({"method":METHOD_AGENT_MESSAGE_DELTA,"params":{
                        "threadId":thread,"turnId":turn,"delta":text}}));
                }
                assert!(
                    !steering.is_finished(),
                    "notifications cannot acknowledge steering"
                );
                server.accept(&request["id"]);
                assert_eq!(steering.join().unwrap(), Ok(()));
                let second = scope.spawn(|| mailbox.steer("second", deadline()));
                let next = server.read();
                assert_eq!(next["method"], METHOD_TURN_STEER);
                assert_ne!(next["id"], request["id"]);
                assert_eq!(next["params"]["expectedTurnId"], "tu");
                server.accept(&next["id"]);
                assert_eq!(second.join().unwrap(), Ok(()));
            });
            server.complete("completed");
            wait_until(|| mailbox.finished.load(Ordering::Acquire));
        });
        assert_eq!(status.unwrap(), TurnStatus::Completed);
        assert_eq!(output, "hello");
    }

    #[test]
    fn native_rejection_and_missing_method_do_not_restart_or_create_turns() {
        for (rpc_code, message, expected) in [
            (-32603, "secret prompt echoed", "backend_rejected"),
            (-32601, "Method not found", "unsupported_backend_version"),
            (
                -32600,
                "Invalid request: unknown variant `turn/steer`, expected thread/start",
                "unsupported_backend_version",
            ),
            (-32600, "invalid input", "backend_rejected"),
            (
                -32600,
                "Invalid request: unknown variant `image`, expected turn/steer",
                "backend_rejected",
            ),
        ] {
            let (status, _) = with_run(|server, mailbox, _, _| {
                server.start(mailbox);
                thread::scope(|scope| {
                    let steering =
                        scope.spawn(|| mailbox.steer("secret prompt echoed", deadline()));
                    let request = server.read();
                    assert_eq!(request["method"], METHOD_TURN_STEER);
                    server.send(
                        json!({"id":request["id"],"error":{"code":rpc_code,"message":message}}),
                    );
                    let result = steering.join().unwrap();
                    assert!(!result
                        .as_ref()
                        .unwrap_err()
                        .to_string()
                        .contains("secret prompt"));
                    if expected == "unsupported_backend_version" {
                        assert!(result
                            .as_ref()
                            .unwrap_err()
                            .to_string()
                            .contains("upgrade Codex"));
                    }
                    assert_rejected(result, expected);
                    let next = scope.spawn(|| mailbox.steer("another", deadline()));
                    let request = server.read();
                    assert_eq!(request["method"], METHOD_TURN_STEER);
                    server.accept(&request["id"]);
                    assert_eq!(next.join().unwrap(), Ok(()));
                });
                server.complete("completed");
                wait_until(|| mailbox.finished.load(Ordering::Acquire));
            });
            assert_eq!(status.unwrap(), TurnStatus::Completed);
        }
    }

    #[test]
    fn startup_ack_does_not_drop_output_or_completion_notifications() {
        let (status, output) = with_run(|server, mailbox, _, _| {
            let request = server.read();
            assert_eq!(request["method"], METHOD_TURN_START);
            server.send(json!({"method":METHOD_AGENT_MESSAGE_DELTA,"params":{
                "threadId":"th","turnId":"tu","delta":"early"}}));
            server.complete("completed");
            assert_rejected(mailbox.steer("startup", deadline()), "no_active_turn");
            server.send(json!({"id":request["id"],"result":{"turn":{"id":"tu"}}}));
        });
        assert_eq!(status.unwrap(), TurnStatus::Completed);
        assert_eq!(output, "early");
    }

    #[test]
    fn interrupt_is_sent_without_waiting_for_steering_or_interrupt_ack() {
        for complete_first in [false, true] {
            let (status, _) = with_run(|server, mailbox, interrupt, _| {
                server.start(mailbox);
                thread::scope(|scope| {
                    let steering = scope.spawn(|| mailbox.steer("steer", deadline()));
                    let request = server.read();
                    interrupt.store(true, Ordering::SeqCst);
                    let stop = server.read();
                    assert_eq!(stop["method"], METHOD_TURN_INTERRUPT);
                    assert_eq!(stop["params"], json!({"threadId":"th","turnId":"tu"}));
                    assert_ne!(stop["id"], request["id"]);
                    if complete_first {
                        server.complete("interrupted");
                        assert!(matches!(
                            steering.join().unwrap(),
                            Err(SteeringError::AcceptanceUnknown { .. })
                        ));
                    } else {
                        server.send(json!({"id":stop["id"],"result":{}}));
                        assert!(!steering.is_finished());
                        server.accept(&request["id"]);
                        assert_eq!(steering.join().unwrap(), Ok(()));
                        server.complete("interrupted");
                    }
                });
                wait_until(|| mailbox.finished.load(Ordering::Acquire));
            });
            assert_eq!(status.unwrap(), TurnStatus::Interrupted);
        }
    }

    #[test]
    fn uncertain_delivery_keeps_correlation_until_late_reply_without_retry() {
        let (status, _) = with_run(|server, mailbox, _, output| {
            server.start(mailbox);
            thread::scope(|scope| {
                let steering = scope.spawn(|| {
                    mailbox.steer("expires", Instant::now() + Duration::from_millis(250))
                });
                let request = server.read();
                assert!(matches!(
                    steering.join().unwrap(),
                    Err(SteeringError::AcceptanceUnknown { .. })
                ));
                assert_rejected(mailbox.steer("blocked", deadline()), "busy");
                server.accept(&request["id"]);
                server.send(json!({"method":METHOD_AGENT_MESSAGE_DELTA,"params":{
                    "threadId":"th","turnId":"tu","delta":"late reply processed"}}));
                wait_until(|| !output.0.lock().unwrap().is_empty());
                let next = scope.spawn(|| mailbox.steer("next", deadline()));
                let second = server.read();
                assert_eq!(second["method"], METHOD_TURN_STEER);
                assert_eq!(second["params"]["input"][0]["text"], "next");
                assert_ne!(second["id"], request["id"]);
                server.accept(&request["id"]);
                assert!(!next.is_finished());
                server.accept(&second["id"]);
                assert_eq!(next.join().unwrap(), Ok(()));
            });
            server.complete("completed");
            wait_until(|| mailbox.finished.load(Ordering::Acquire));
        });
        assert_eq!(status.unwrap(), TurnStatus::Completed);
    }

    #[test]
    fn eof_after_dispatch_reports_unknown_and_preserves_run_failure() {
        let (status, _) = with_run(|server, mailbox, _, _| {
            server.start(mailbox);
            thread::scope(|scope| {
                let steering = scope.spawn(|| mailbox.steer("steer", deadline()));
                assert_eq!(server.read()["method"], METHOD_TURN_STEER);
                server
                    .0
                    .get_mut()
                    .shutdown(std::net::Shutdown::Write)
                    .unwrap();
                assert!(matches!(
                    steering.join().unwrap(),
                    Err(SteeringError::AcceptanceUnknown { .. })
                ));
            });
        });
        assert_eq!(status.unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn invalid_acknowledgement_is_uncertain_and_expired_requests_never_dispatch() {
        for response in [json!({}), json!({"turnId":"other"})] {
            let (status, _) = with_run(|server, mailbox, _, _| {
                server.start(mailbox);
                assert_rejected(mailbox.steer("expired", Instant::now()), "timeout");
                thread::scope(|scope| {
                    let steering = scope.spawn(|| mailbox.steer("steer", deadline()));
                    let request = server.read();
                    server.send(json!({"id":request["id"],"result":response}));
                    assert!(matches!(
                        steering.join().unwrap(),
                        Err(SteeringError::AcceptanceUnknown { .. })
                    ));
                });
                server.complete("completed");
                wait_until(|| mailbox.finished.load(Ordering::Acquire));
            });
            assert_eq!(status.unwrap(), TurnStatus::Completed);
        }
    }

    #[test]
    fn fragmented_notifications_do_not_block_steering_or_corrupt_utf8() {
        let (status, output) = with_run(|server, mailbox, _, _| {
            server.start(mailbox);
            let frame = format!(
                "{}\n",
                json!({"method":METHOD_AGENT_MESSAGE_DELTA,"params":{
                "threadId":"th","turnId":"tu","delta":"λ"}})
            );
            let split = frame
                .as_bytes()
                .iter()
                .position(|byte| *byte == 0xce)
                .unwrap()
                + 1;
            server
                .0
                .get_mut()
                .write_all(&frame.as_bytes()[..split])
                .unwrap();
            thread::scope(|scope| {
                let steering = scope.spawn(|| mailbox.steer("fragment", deadline()));
                let request = server.read();
                assert_eq!(request["method"], METHOD_TURN_STEER);
                server
                    .0
                    .get_mut()
                    .write_all(&frame.as_bytes()[split..])
                    .unwrap();
                server.accept(&request["id"]);
                assert_eq!(steering.join().unwrap(), Ok(()));
            });
            server.complete("completed");
            wait_until(|| mailbox.finished.load(Ordering::Acquire));
        });
        assert_eq!(status.unwrap(), TurnStatus::Completed);
        assert_eq!(output, "λ");
    }

    #[test]
    fn completion_status_validation_and_startup_errors_are_preserved() {
        for (wire, expected) in [
            ("completed", Some(TurnStatus::Completed)),
            ("interrupted", Some(TurnStatus::Interrupted)),
            ("failed", Some(TurnStatus::Failed)),
            ("inProgress", Some(TurnStatus::InProgress)),
            ("Completed", None),
            ("unknown", None),
        ] {
            let (status, _) = with_run(|server, mailbox, _, _| {
                server.start(mailbox);
                server.complete(wire);
                wait_until(|| mailbox.finished.load(Ordering::Acquire));
            });
            match expected {
                Some(expected) => assert_eq!(status.unwrap(), expected),
                None => assert_eq!(status.unwrap_err().kind(), io::ErrorKind::InvalidData),
            }
        }
        for response in [
            json!({"error":{"code":-32603,"message":"rejected"}}),
            json!({"result":{}}),
        ] {
            let (status, _) = with_run(|server, _, _, _| {
                let request = server.read();
                let mut response = response;
                response["id"] = request["id"].clone();
                server.send(response);
            });
            assert!(status.is_err());
        }
        assert!(TurnStatus::Completed.is_success());
        for status in [
            TurnStatus::Failed,
            TurnStatus::Interrupted,
            TurnStatus::InProgress,
        ] {
            assert!(!status.is_success());
        }
    }

    struct LimitedWriter {
        bytes: Vec<u8>,
        allowance: usize,
    }

    impl Write for LimitedWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.allowance == 0 {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            let count = bytes.len().min(self.allowance);
            self.bytes.extend_from_slice(&bytes[..count]);
            self.allowance -= count;
            Ok(count)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn unsent_timeout_is_definite_and_partial_write_timeout_preserves_framing() {
        for allowance in [0, 7] {
            let (reply, replies) = mpsc::sync_channel(1);
            let expires = deadline();
            let request = SteerRequest {
                prompt: "prompt".to_string(),
                deadline: expires,
                reply: Some(reply),
            };
            let mut client = CodexClient {
                reader: io::Cursor::new(Vec::<u8>::new()),
                writer: LimitedWriter {
                    bytes: Vec::new(),
                    allowance,
                },
                out: Vec::new(),
                next_id: 1,
                model: None,
                reasoning_effort: None,
            };
            let mut turn = LiveTurn {
                thread_id: "th",
                start_id: 0,
                turn_id: Some("tu".to_string()),
                early_completion: None,
                pending: None,
                writes: VecDeque::new(),
                interrupt_id: None,
            };
            let id = client.enqueue(&mut turn, METHOD_TURN_STEER, json!({"input":"prompt"}));
            turn.pending = Some(PendingSteer {
                id,
                request,
                dispatched: false,
            });
            client.flush_requests(&mut turn).unwrap();
            assert_eq!(turn.pending.as_ref().unwrap().dispatched, allowance > 0);
            turn.pending.as_mut().unwrap().expire(expires);
            let result = replies.recv().unwrap();
            if allowance == 0 {
                assert_rejected(result, "timeout");
            } else {
                assert!(matches!(
                    result,
                    Err(SteeringError::AcceptanceUnknown { .. })
                ));
            }
            client.writer.allowance = usize::MAX;
            client.flush_requests(&mut turn).unwrap();
            if allowance == 0 {
                assert!(client.writer.bytes.is_empty());
                assert!(turn.pending.is_none());
            } else {
                let wire: JsonValue = serde_json::from_slice(&client.writer.bytes).unwrap();
                assert_eq!(wire["method"], METHOD_TURN_STEER);
                assert!(
                    turn.pending.is_some(),
                    "keep the correlation until a late acknowledgement"
                );
            }
            assert!(turn.writes.is_empty());
        }
    }

    #[test]
    fn caller_timeout_is_unknown_and_owner_discards_expired_queued_request() {
        let (mailbox, commands) = SteeringMailbox::channel();
        assert!(matches!(
            mailbox.steer("queued", Instant::now() + Duration::from_millis(10)),
            Err(SteeringError::AcceptanceUnknown { .. })
        ));
        let request = commands.recv().unwrap();
        assert!(Instant::now() >= request.deadline);
        let mut client = CodexClient {
            reader: io::Cursor::new(Vec::<u8>::new()),
            writer: Vec::new(),
            out: Vec::new(),
            next_id: 1,
            model: None,
            reasoning_effort: None,
        };
        let mut turn = LiveTurn {
            thread_id: "th",
            start_id: 0,
            turn_id: Some("tu".to_string()),
            early_completion: None,
            pending: None,
            writes: VecDeque::new(),
            interrupt_id: None,
        };
        client.dispatch_steering(&mut turn, request);
        client.flush_requests(&mut turn).unwrap();
        assert!(client.writer.is_empty());
        assert!(turn.pending.is_none());
    }

    #[test]
    fn driver_exit_without_a_reply_is_unknown_and_later_calls_are_rejected() {
        let (mailbox, commands) = SteeringMailbox::channel();
        thread::scope(|scope| {
            let caller = scope.spawn(|| mailbox.steer("queued", deadline()));
            let request = commands.recv_timeout(Duration::from_secs(1)).unwrap();
            drop(request);
            assert!(matches!(
                caller.join().unwrap(),
                Err(SteeringError::AcceptanceUnknown { .. })
            ));
        });
        drop(commands);
        assert_rejected(mailbox.steer("after exit", deadline()), "agent_exited");
    }

    #[test]
    fn full_mailbox_rejects_without_queueing_another_request() {
        let (mailbox, commands) = SteeringMailbox::channel();
        let (reply, _replies) = mpsc::sync_channel(1);
        mailbox
            .sender
            .try_send(SteerRequest {
                prompt: "first".to_string(),
                deadline: deadline(),
                reply: Some(reply),
            })
            .unwrap();
        assert_rejected(mailbox.steer("second", deadline()), "busy");
        assert_eq!(commands.recv().unwrap().prompt, "first");
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn malformed_native_errors_leave_acceptance_unknown() {
        for response in [
            json!({"error": null}),
            json!({"error": {}}),
            json!({"error": {"code": -32603}}),
            json!({"error": {"message": "secret prompt"}}),
            json!({"error": {"code": "invalid", "message": "secret prompt"}}),
            json!({"error": {"code": -32603, "message": "secret prompt"},
                "result": {"turnId": "tu"}}),
        ] {
            let (status, _) = with_run(|server, mailbox, _, _| {
                server.start(mailbox);
                thread::scope(|scope| {
                    let steering = scope.spawn(|| mailbox.steer("secret prompt", deadline()));
                    let request = server.read();
                    let mut response = response;
                    response["id"] = request["id"].clone();
                    server.send(response);
                    let error = steering.join().unwrap().unwrap_err();
                    assert!(
                        matches!(error, SteeringError::AcceptanceUnknown { .. }),
                        "{error:?}"
                    );
                    assert!(!error.to_string().contains("secret prompt"));
                });
                server.complete("completed");
                wait_until(|| mailbox.finished.load(Ordering::Acquire));
            });
            assert_eq!(status.unwrap(), TurnStatus::Completed);
        }
    }
}
