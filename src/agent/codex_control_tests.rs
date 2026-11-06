use std::os::unix::net::UnixStream;

use super::super::commands::CommandDispatcher;
use super::*;

#[test]
fn interrupt_rejection_preserves_rpc_code_without_disclosing_backend_text() {
    for code in [-32601, -32603, -32000] {
        let error = parse_interrupt_response(&json!({
            "error": {"code": code, "message": "private prompt or diagnostic"}
        }))
        .unwrap_err();
        assert!(matches!(
            error,
            CommandError::Rejected {
                code: "backend_rejected",
                ..
            }
        ));
        let diagnostic = error.to_string();
        assert!(diagnostic.contains(&format!("RPC error code {code}")));
        assert!(diagnostic.contains("verify that the turn is active"));
        assert!(!diagnostic.contains("private prompt or diagnostic"));
    }
    for response in [
        json!({"error": {"code": "invalid", "message": "private"}}),
        json!({"error": {"code": -32603}}),
        json!({"error": {"code": -32603, "message": "private"}, "result": {}}),
    ] {
        assert!(matches!(
            parse_interrupt_response(&response),
            Err(CommandError::AcceptanceUnknown { .. })
        ));
    }
}

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

impl AsyncWrite for CapturedOutput {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        std::task::Poll::Ready(Write::write(&mut *self, bytes))
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

fn create_test_dispatcher() -> (CommandDispatcher, Receiver<Command>) {
    CommandDispatcher::create_channel()
}

struct TestMailbox {
    dispatcher: CommandDispatcher,
    finished: AtomicBool,
    output: CapturedOutput,
}

impl TestMailbox {
    fn stop(&self) -> CommandResult {
        crate::agent::runtime::block_on(self.dispatcher.stop(deadline()))
    }

    fn steer(&self, prompt: &str, deadline: Instant) -> CommandResult {
        crate::agent::runtime::block_on(self.dispatcher.steer(prompt, deadline))
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
    action: impl FnOnce(&mut Server, &TestMailbox, &tokio::sync::watch::Sender<bool>, &CapturedOutput),
) -> (io::Result<TurnStatus>, String) {
    let (stream, server) = UnixStream::pair().unwrap();
    stream.set_nonblocking(true).unwrap();
    server
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let output = CapturedOutput::default();
    let (dispatcher, command_receiver) = create_test_dispatcher();
    let mailbox = TestMailbox {
        dispatcher,
        finished: AtomicBool::new(false),
        output: output.clone(),
    };
    let (shutdown_signal, mut shutdown_signals) = tokio::sync::watch::channel(false);
    let result = thread::scope(|scope| {
        let worker = scope.spawn(|| {
            let mut command_receiver = command_receiver;
            let result = crate::agent::runtime::block_on(async {
                let (reader, writer) = tokio::net::UnixStream::from_std(stream)
                    .unwrap()
                    .into_split();
                let mut client = CodexClient {
                    reader,
                    writer,
                    out: output.clone(),
                    next_id: 0,
                    model: None,
                    reasoning_effort: None,
                };
                client
                    .run_turn(
                        "th",
                        "initial",
                        async {
                            shutdown_signals.changed().await.unwrap();
                        },
                        &mut command_receiver,
                    )
                    .await
            });
            drop(command_receiver);
            mailbox.finished.store(true, Ordering::Release);
            result
        });
        let mut server = Server(BufReader::new(server));
        action(&mut server, &mailbox, &shutdown_signal, &output);
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

fn assert_rejected(result: CommandResult, code: &str) {
    assert!(
        matches!(&result, Err(CommandError::Rejected { code: actual, .. }) if *actual == code),
        "{result:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn acknowledgement_deadline_expires_without_output_or_client_timeout() {
    let (stream, server) = tokio::io::duplex(4096);
    let (reader, writer) = tokio::io::split(stream);
    let output = CapturedOutput::default();
    let mut client = CodexClient {
        reader,
        writer,
        out: output.clone(),
        next_id: 0,
        model: None,
        reasoning_effort: None,
    };
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    let (result, ()) = tokio::join!(
        client.run_turn("th", "initial", std::future::pending(), &mut receiver),
        async {
            use tokio::io::AsyncBufReadExt;
            let mut server = tokio::io::BufReader::new(server);
            let mut line = String::new();
            server.read_line(&mut line).await.unwrap();
            let start: JsonValue = serde_json::from_str(&line).unwrap();
            let ack = json!({"id": start["id"], "result": {"turn": {"id": "tu"}}});
            let ready = json!({"method": METHOD_AGENT_MESSAGE_DELTA, "params": {"threadId": "th", "turnId": "tu", "delta": "ready"}});
            server
                .get_mut()
                .write_all(format!("{ack}\n{ready}\n").as_bytes())
                .await
                .unwrap();
            while output.0.lock().unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
            let (request, reply_receiver) =
                CommandRequest::new(Instant::now() + Duration::from_millis(100));
            sender
                .send(Command::Steer {
                    prompt: "expires".into(),
                    request,
                })
                .await
                .unwrap();
            line.clear();
            server.read_line(&mut line).await.unwrap();
            assert_eq!(
                serde_json::from_str::<JsonValue>(&line).unwrap()["method"],
                METHOD_TURN_STEER
            );
            assert!(matches!(
                tokio::time::timeout(Duration::from_secs(2), reply_receiver)
                    .await
                    .unwrap()
                    .unwrap(),
                Err(CommandError::AcceptanceUnknown { .. })
            ));
            let complete = json!({"method":METHOD_TURN_COMPLETED,"params":{"threadId":"th","turn":{"id":"tu","status":"completed"}}});
            server
                .get_mut()
                .write_all(format!("{complete}\n").as_bytes())
                .await
                .unwrap();
            // Avoid racing completion against EOF when the mock server drops its stream.
            tokio::task::yield_now().await;
        }
    );
    assert_eq!(result.unwrap(), TurnStatus::Completed);
}

#[test]
fn backpressured_writes_resume_on_writability_without_server_output() {
    let prompt = "x".repeat(2 * 1024 * 1024);
    with_run(|server, mailbox, _, _| {
        server.start(mailbox);
        thread::scope(|scope| {
            let steering = scope.spawn(|| mailbox.steer(&prompt, deadline()));
            let request = server.read();
            assert_eq!(request["params"]["input"][0]["text"], prompt);
            server.accept(&request["id"]);
            assert_eq!(steering.join().unwrap(), Ok(()));
        });
        server.complete("completed");
    })
    .0
    .unwrap();
}

#[test]
fn competing_steering_is_busy_without_waiting_for_acknowledgement() {
    with_run(|server, mailbox, _, _| {
        server.start(mailbox);
        thread::scope(|scope| {
            let first = scope.spawn(|| mailbox.steer("first", deadline()));
            let request = server.read();
            assert_eq!(request["method"], METHOD_TURN_STEER);
            assert_rejected(mailbox.steer("second", deadline()), "busy");
            assert!(!first.is_finished());
            server.accept(&request["id"]);
            assert_eq!(first.join().unwrap(), Ok(()));
        });
        server.complete("completed");
    })
    .0
    .unwrap();
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

#[tokio::test(flavor = "current_thread")]
async fn stop_deadlines_distinguish_unsent_intent_from_lost_native_ack() {
    for active in [false, true] {
        let (stream, server) = tokio::io::duplex(4096);
        let (reader, writer) = tokio::io::split(stream);
        let mut client = CodexClient {
            reader,
            writer,
            out: Vec::new(),
            next_id: 0,
            model: None,
            reasoning_effort: None,
        };
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let (result, ()) = tokio::join!(
            client.run_turn("th", "initial", std::future::pending(), &mut receiver),
            async {
                let mut server = tokio::io::BufReader::new(server);
                let mut line = String::new();
                server.read_line(&mut line).await.unwrap();
                if active {
                    let start: JsonValue = serde_json::from_str(&line).unwrap();
                    server
                        .get_mut()
                        .write_all(
                            format!(
                                "{}\n",
                                json!({"id":start["id"],"result":{"turn":{"id":"tu"}}})
                            )
                            .as_bytes(),
                        )
                        .await
                        .unwrap();
                }
                let (request, reply) =
                    CommandRequest::new(Instant::now() + Duration::from_millis(50));
                sender.send(Command::Stop(request)).await.unwrap();
                if active {
                    line.clear();
                    server.read_line(&mut line).await.unwrap();
                    assert_eq!(
                        serde_json::from_str::<JsonValue>(&line).unwrap()["method"],
                        METHOD_TURN_INTERRUPT
                    );
                }
                let result = reply.await.unwrap();
                if active {
                    assert!(matches!(
                        result,
                        Err(CommandError::AcceptanceUnknown { .. })
                    ));
                } else {
                    assert_rejected(result, "timeout");
                }
            }
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn accepted_stop_bounds_completion_without_retracting_acceptance() {
    let mut client = CodexClient {
        reader: io::Cursor::new(Vec::<u8>::new()),
        writer: Vec::<u8>::new(),
        out: Vec::<u8>::new(),
        next_id: 2,
        model: None,
        reasoning_effort: None,
    };
    let mut turn = TurnState {
        turn_id: Some("tu".into()),
        interrupt_rpc_id: Some(1),
        ..TurnState::new("th", 0)
    };
    let (request, reply) = CommandRequest::new(deadline());
    CodexClient::<(), (), Vec<u8>>::dispatch_stop(&mut turn, Some(request));
    let before_ack = Instant::now();
    assert!(client
        .process_message(&mut turn, &json!({"id": 1, "result": {}}))
        .await
        .unwrap()
        .is_none());
    assert_eq!(reply.await.unwrap(), Ok(()));
    let stop = turn.pending_stop.as_mut().unwrap();
    assert!(stop.acknowledged);
    assert!(stop.deadline >= before_ack + STOP_TIMEOUT);
    assert!(stop.deadline <= Instant::now() + STOP_TIMEOUT);
    assert!(stop.check_deadline().is_ok());
    stop.deadline = Instant::now();
    let error = stop.check_deadline().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(error.to_string().contains("accepted interruption"));
    assert!(!stop.request.as_ref().unwrap().has_reply_sender());
}

#[tokio::test(flavor = "current_thread")]
async fn stop_preserves_a_partially_written_steer_frame() {
    use futures_util::FutureExt;
    let mut client = CodexClient {
        reader: io::Cursor::new(Vec::<u8>::new()),
        writer: LimitedWriter {
            bytes: Vec::new(),
            allowance: 7,
        },
        out: Vec::new(),
        next_id: 1,
        model: None,
        reasoning_effort: None,
    };
    let mut turn = TurnState {
        turn_id: Some("tu".into()),
        interrupt_rpc_id: None,
        ..TurnState::new("th", 0)
    };
    let (steer, _) = CommandRequest::new(deadline());
    client.dispatch_steering(&mut turn, "prompt".into(), steer);
    assert!(client.flush_requests(&mut turn).now_or_never().is_none());
    let (stop, mut reply) = CommandRequest::new(deadline());
    CodexClient::<io::Cursor<Vec<u8>>, LimitedWriter, Vec<u8>>::dispatch_stop(
        &mut turn,
        Some(stop),
    );
    let id = client.enqueue_request(
        &mut turn,
        METHOD_TURN_INTERRUPT,
        json!({"threadId":"th","turnId":"tu"}),
    );
    turn.interrupt_rpc_id = Some(id);
    client.writer.allowance = usize::MAX;
    client.flush_requests(&mut turn).await.unwrap();
    let wire = String::from_utf8(client.writer.bytes.clone()).unwrap();
    let frames: Vec<JsonValue> = wire
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0]["method"], METHOD_TURN_STEER);
    assert_eq!(frames[1]["method"], METHOD_TURN_INTERRUPT);
    assert!(reply.try_recv().is_err());
    client
        .process_message(&mut turn, &json!({"id":id,"result":{}}))
        .await
        .unwrap();
    assert_eq!(reply.try_recv().unwrap(), Ok(()));
}

#[test]
fn stop_bypasses_pending_steer_and_waits_for_native_ack_even_after_completion() {
    for complete_first in [false, true] {
        let (status, _) = with_run(|server, mailbox, _, _| {
            server.start(mailbox);
            thread::scope(|scope| {
                let steering = scope.spawn(|| mailbox.steer("steer", deadline()));
                let steer = server.read();
                let stopping = scope.spawn(|| mailbox.stop());
                let stop = server.read();
                assert_eq!(stop["method"], METHOD_TURN_INTERRUPT);
                assert_eq!(stop["params"], json!({"threadId":"th","turnId":"tu"}));
                assert_ne!(stop["id"], steer["id"]);
                assert!(!stopping.is_finished(), "enqueue is not acceptance");
                assert_rejected(mailbox.stop(), "busy");
                server.accept(&steer["id"]);
                assert_eq!(steering.join().unwrap(), Ok(()));
                if complete_first {
                    server.complete("interrupted");
                }
                server.send(json!({"id":stop["id"].as_i64().unwrap() + 100,"result":{}}));
                assert!(
                    !stopping.is_finished(),
                    "uncorrelated response is not acceptance"
                );
                server.send(json!({"id":stop["id"],"result":{}}));
                assert_eq!(stopping.join().unwrap(), Ok(()));
                if !complete_first {
                    assert_eq!(
                        mailbox.stop(),
                        Ok(()),
                        "duplicate after acknowledgement shares acceptance"
                    );
                    server.complete("interrupted");
                }
            });
            wait_until(|| mailbox.finished.load(Ordering::Acquire));
        });
        assert_eq!(status.unwrap(), TurnStatus::Aborted);
    }
}

#[test]
fn pre_turn_stop_retains_intent_without_starting_another_turn() {
    let (status, _) = with_run(|server, mailbox, _, _| {
        let start = server.read();
        assert_eq!(start["method"], METHOD_TURN_START);
        thread::scope(|scope| {
            let (enqueued, queued) = std::sync::mpsc::channel();
            let stopping = scope.spawn(move || {
                crate::agent::runtime::block_on(async {
                    use futures_util::FutureExt;
                    let stop = mailbox.dispatcher.stop(deadline());
                    tokio::pin!(stop);
                    assert!(stop.as_mut().now_or_never().is_none());
                    enqueued.send(()).unwrap();
                    stop.await
                })
            });
            queued.recv().unwrap();
            // Steering rejection proves the owner retained the pre-turn intent.
            wait_until(|| {
                matches!(
                    mailbox.steer("probe", deadline()),
                    Err(CommandError::Rejected {
                        code: "stopping",
                        ..
                    })
                )
            });
            assert!(!stopping.is_finished());
            server.send(json!({"id":start["id"],"result":{"turn":{"id":"tu"}}}));
            let stop = server.read();
            assert_eq!(stop["method"], METHOD_TURN_INTERRUPT);
            server.send(json!({"id":stop["id"],"result":{}}));
            assert_eq!(stopping.join().unwrap(), Ok(()));
            server.complete("interrupted");
        });
        wait_until(|| mailbox.finished.load(Ordering::Acquire));
    });
    assert_eq!(status.unwrap(), TurnStatus::Aborted);
}

#[test]
fn acknowledged_stop_does_not_relabel_natural_completion() {
    let (status, _) = with_run(|server, mailbox, _, _| {
        server.start(mailbox);
        thread::scope(|scope| {
            let stopping = scope.spawn(|| mailbox.stop());
            let stop = server.read();
            server.send(json!({"id":stop["id"],"result":{}}));
            assert_eq!(stopping.join().unwrap(), Ok(()));
            server.complete("completed");
        });
        wait_until(|| mailbox.finished.load(Ordering::Acquire));
    });
    assert_eq!(status.unwrap(), TurnStatus::Completed);
}

#[test]
fn stop_rejection_malformed_ack_and_transport_loss_do_not_abort_successfully() {
    for response in [
        Some(json!({"error":{"code":-32603,"message":"private diagnostic"}})),
        Some(json!({"result":null})),
        None,
    ] {
        let (status, _) = with_run(|server, mailbox, _, _| {
            server.start(mailbox);
            thread::scope(|scope| {
                let stopping = scope.spawn(|| mailbox.stop());
                let stop = server.read();
                if let Some(mut response) = response {
                    response["id"] = stop["id"].clone();
                    server.send(response);
                } else {
                    server
                        .0
                        .get_mut()
                        .shutdown(std::net::Shutdown::Both)
                        .unwrap();
                }
                assert!(stopping.join().unwrap().is_err());
            });
            wait_until(|| mailbox.finished.load(Ordering::Acquire));
        });
        assert!(status.is_err());
    }
}

#[test]
fn interrupt_is_sent_without_waiting_for_steering_or_interrupt_ack() {
    for complete_first in [false, true] {
        let (status, _) = with_run(|server, mailbox, shutdown_signal, _| {
            server.start(mailbox);
            thread::scope(|scope| {
                let steering = scope.spawn(|| mailbox.steer("steer", deadline()));
                let request = server.read();
                shutdown_signal.send(true).unwrap();
                let stop = server.read();
                assert_eq!(stop["method"], METHOD_TURN_INTERRUPT);
                assert_eq!(stop["params"], json!({"threadId":"th","turnId":"tu"}));
                assert_ne!(stop["id"], request["id"]);
                if complete_first {
                    server.complete("interrupted");
                    server.send(json!({"id":stop["id"],"result":{}}));
                    assert!(matches!(
                        steering.join().unwrap(),
                        Err(CommandError::AcceptanceUnknown { .. })
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
        assert_eq!(status.unwrap(), TurnStatus::Aborted);
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
                Err(CommandError::AcceptanceUnknown { .. })
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
                Err(CommandError::AcceptanceUnknown { .. })
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
                    Err(CommandError::AcceptanceUnknown { .. })
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

impl AsyncWrite for LimitedWriter {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        match Write::write(&mut *self, bytes) {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => std::task::Poll::Pending,
            result => std::task::Poll::Ready(result),
        }
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

#[tokio::test(flavor = "current_thread")]
async fn unsent_timeout_is_definite_and_partial_write_timeout_preserves_framing() {
    use futures_util::FutureExt;
    for allowance in [0, 7] {
        let expires = deadline();
        let (request, mut reply_receiver) = CommandRequest::new(expires);
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
        let mut turn = TurnState {
            turn_id: Some("tu".to_string()),
            interrupt_rpc_id: None,
            ..TurnState::new("th", 0)
        };
        let id = client.enqueue_request(&mut turn, METHOD_TURN_STEER, json!({"input":"prompt"}));
        turn.pending_steer = Some(PendingSteer {
            id,
            request,
            write_started: false,
        });
        assert!(client.flush_requests(&mut turn).now_or_never().is_none());
        assert_eq!(
            turn.pending_steer.as_ref().unwrap().write_started,
            allowance > 0
        );
        turn.pending_steer.as_mut().unwrap().expire(expires);
        let result = reply_receiver.try_recv().unwrap();
        if allowance == 0 {
            assert_rejected(result, "timeout");
        } else {
            assert!(matches!(
                result,
                Err(CommandError::AcceptanceUnknown { .. })
            ));
        }
        client.writer.allowance = usize::MAX;
        client.flush_requests(&mut turn).await.unwrap();
        if allowance == 0 {
            assert!(client.writer.bytes.is_empty());
            assert!(turn.pending_steer.is_none());
        } else {
            let wire: JsonValue = serde_json::from_slice(&client.writer.bytes).unwrap();
            assert_eq!(wire["method"], METHOD_TURN_STEER);
            assert!(
                turn.pending_steer.is_some(),
                "keep the correlation until a late acknowledgement"
            );
        }
        assert!(turn.pending_writes.is_empty());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn caller_timeout_is_unknown_and_owner_discards_expired_queued_request() {
    let (dispatcher, mut command_receiver) = create_test_dispatcher();
    assert!(matches!(
        dispatcher
            .steer("queued", Instant::now() + Duration::from_millis(10))
            .await,
        Err(CommandError::AcceptanceUnknown { .. })
    ));
    let Command::Steer { prompt, request } = command_receiver.recv().await.unwrap() else {
        panic!("expected steering command")
    };
    assert!(Instant::now() >= request.deadline);
    let mut client = CodexClient {
        reader: io::Cursor::new(Vec::<u8>::new()),
        writer: Vec::new(),
        out: Vec::new(),
        next_id: 1,
        model: None,
        reasoning_effort: None,
    };
    let mut turn = TurnState {
        turn_id: Some("tu".to_string()),
        interrupt_rpc_id: None,
        ..TurnState::new("th", 0)
    };
    client.dispatch_steering(&mut turn, prompt, request);
    client.flush_requests(&mut turn).await.unwrap();
    assert!(client.writer.is_empty());
    assert!(turn.pending_steer.is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn runner_exit_without_a_reply_is_unknown_and_later_calls_are_rejected() {
    let (dispatcher, mut command_receiver) = create_test_dispatcher();
    let (result, ()) = tokio::join!(dispatcher.steer("queued", deadline()), async {
        drop(command_receiver.recv().await.unwrap());
    });
    assert!(matches!(
        result,
        Err(CommandError::AcceptanceUnknown { .. })
    ));
    drop(command_receiver);
    assert_rejected(
        dispatcher.steer("after exit", deadline()).await,
        "agent_exited",
    );
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
                    matches!(error, CommandError::AcceptanceUnknown { .. }),
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

#[test]
fn only_acknowledged_interruption_maps_to_aborted() {
    let mut turn = TurnState::new("th", 0);
    for acknowledged in [false, true] {
        turn.pending_stop = Some(PendingStop {
            request: None,
            deadline: deadline(),
            write_started: true,
            acknowledged,
        });
        assert_eq!(
            turn.resolve_status(TurnStatus::Completed),
            TurnStatus::Completed
        );
        assert_eq!(
            turn.resolve_status(TurnStatus::Interrupted),
            if acknowledged {
                TurnStatus::Aborted
            } else {
                TurnStatus::Interrupted
            }
        );
    }
    turn.pending_stop = None;
    assert_eq!(
        turn.resolve_status(TurnStatus::Interrupted),
        TurnStatus::Interrupted
    );
}

#[test]
fn compute_deadline_ignores_replied_steering_but_retains_stop_cleanup_deadline() {
    let mut turn = TurnState::new("th", 0);
    assert_eq!(turn.compute_deadline(), None);
    let steer_deadline = deadline();
    let stop_deadline = steer_deadline + Duration::from_secs(1);
    let (request, _reply) = CommandRequest::new(steer_deadline);
    turn.pending_steer = Some(PendingSteer {
        id: 1,
        request,
        write_started: false,
    });
    turn.pending_stop = Some(PendingStop {
        request: None,
        deadline: stop_deadline,
        write_started: true,
        acknowledged: true,
    });
    assert_eq!(turn.compute_deadline(), Some(steer_deadline));
    turn.pending_steer.as_mut().unwrap().request.respond(Ok(()));
    assert_eq!(turn.compute_deadline(), Some(stop_deadline));
    turn.pending_steer = None;
    assert_eq!(turn.compute_deadline(), Some(stop_deadline));
}
