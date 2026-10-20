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

    fn start(&mut self, mailbox: &SteeringMailbox) {
        let request = self.read();
        assert_eq!(request["method"], METHOD_TURN_START);
        assert_eq!(request["params"]["input"][0]["text"], "initial");
        self.send(json!({"id":request["id"],"result":{"turn":{"id":"tu"}}}));
        wait_until(|| mailbox.active.load(Ordering::Acquire));
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
    action: impl FnOnce(&mut Server, &SteeringMailbox, &AtomicBool, &CapturedOutput),
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
    let mailbox = SteeringMailbox::new();
    let interrupt = AtomicBool::new(false);
    let result = thread::scope(|scope| {
        let commands = mailbox.take_receiver().unwrap();
        let worker =
            scope.spawn(|| client.run_turn("th", "initial", &interrupt, &mailbox.active, commands));
        let mut server = Server(BufReader::new(server));
        action(&mut server, &mailbox, &interrupt, &output);
        drop(server);
        worker.join().unwrap()
    });
    assert!(!mailbox.active.load(Ordering::Acquire));
    assert_rejected(
        mailbox.steer("after completion", deadline()),
        "no_active_turn",
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
            // A second request must reuse this same turn and have a distinct RPC ID.
            let second = scope.spawn(|| mailbox.steer("second", deadline()));
            let next = server.read();
            assert_eq!(next["method"], METHOD_TURN_STEER);
            assert_ne!(next["id"], request["id"]);
            assert_eq!(next["params"]["expectedTurnId"], "tu");
            server.accept(&next["id"]);
            assert_eq!(second.join().unwrap(), Ok(()));
        });
        server.complete("completed");
        wait_until(|| !mailbox.active.load(Ordering::Acquire));
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
                let steering = scope.spawn(|| mailbox.steer("secret prompt echoed", deadline()));
                let request = server.read();
                assert_eq!(request["method"], METHOD_TURN_STEER);
                server
                    .send(json!({"id":request["id"],"error":{"code":rpc_code,"message":message}}));
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
            wait_until(|| !mailbox.active.load(Ordering::Acquire));
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
            wait_until(|| !mailbox.active.load(Ordering::Acquire));
        });
        assert_eq!(status.unwrap(), TurnStatus::Interrupted);
    }
}

#[test]
fn uncertain_delivery_keeps_correlation_until_late_reply_without_retry() {
    let (status, _) = with_run(|server, mailbox, _, output| {
        server.start(mailbox);
        thread::scope(|scope| {
            let steering = scope
                .spawn(|| mailbox.steer("expires", Instant::now() + Duration::from_millis(250)));
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
        wait_until(|| !mailbox.active.load(Ordering::Acquire));
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
            wait_until(|| !mailbox.active.load(Ordering::Acquire));
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
        wait_until(|| !mailbox.active.load(Ordering::Acquire));
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
            wait_until(|| !mailbox.active.load(Ordering::Acquire));
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
        let active = AtomicBool::new(true);
        let (reply, replies) = mpsc::sync_channel(1);
        let delivery = Arc::new(Mutex::new(Delivery::default()));
        let expires = deadline();
        let request = SteerRequest {
            prompt: "prompt".to_string(),
            deadline: expires,
            delivery: Arc::clone(&delivery),
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
            active: &active,
            start_id: 0,
            turn_id: Some("tu".to_string()),
            early_completion: None,
            pending: None,
            writes: VecDeque::new(),
            interrupt_id: None,
        };
        let id = client.enqueue(&mut turn, METHOD_TURN_STEER, json!({"input":"prompt"}));
        turn.pending = Some(PendingSteer { id, request });
        client.flush_requests(&mut turn).unwrap();
        assert_eq!(delivery.lock().unwrap().dispatched, allowance > 0);
        turn.pending.as_mut().unwrap().request.expire(expires);
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
fn timed_out_mailbox_request_is_cancelled_before_owner_dispatch() {
    let mailbox = SteeringMailbox::new();
    mailbox.active.store(true, Ordering::Release);
    let commands = mailbox.take_receiver().unwrap();
    assert_rejected(
        mailbox.steer("queued", Instant::now() + Duration::from_millis(10)),
        "timeout",
    );
    let request = commands.recv().unwrap();
    let state = request.delivery.lock().unwrap();
    assert!(state.cancelled);
    assert!(!state.dispatched);
}
