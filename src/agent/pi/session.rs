use std::future::Future;
use std::io;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use futures_util::FutureExt;
use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncWrite};
use tokio::sync::mpsc::Receiver;

use super::rpc::{invalid_protocol, validate_response, PiClient};
use crate::agent::backend::RunOutcome;
use crate::agent::commands::{Command, CommandError, CommandRequest};

struct PendingSteer {
    id: String,
    request: CommandRequest,
}

enum StopPhase {
    Waiting,
    Clearing(String),
    Aborting(String),
}

struct PendingStop {
    phase: StopPhase,
    deadline: Instant,
    request: Option<CommandRequest>,
}

struct Session {
    prompt_id: Option<String>,
    prompt_deadline: Instant,
    steer: Option<PendingSteer>,
    stop: Option<PendingStop>,
    settled: bool,
    latest_stop_reason: Option<String>,
}

impl Session {
    fn deadline(&self) -> Option<Instant> {
        self.prompt_id
            .as_ref()
            .map(|_| self.prompt_deadline)
            .into_iter()
            .chain(self.steer.as_ref().map(|steer| steer.request.deadline))
            .chain(self.stop.as_ref().map(|stop| stop.deadline))
            .min()
    }

    fn outcome(&self) -> io::Result<RunOutcome> {
        match self.latest_stop_reason.as_deref() {
            Some("stop") => Ok(RunOutcome::Completed),
            Some("aborted") => Ok(RunOutcome::Aborted),
            Some("error" | "length" | "toolUse") => Ok(RunOutcome::Failed(ExitCode::FAILURE)),
            reason => Err(invalid_protocol(format!(
                "Pi settled with missing or unknown assistant stopReason: {reason:?}"
            ))),
        }
    }

    fn observe(&mut self, record: &Value) -> io::Result<()> {
        match record["type"].as_str() {
            Some("agent_start") => self.settled = false,
            Some("agent_settled") => self.settled = true,
            Some("message_end" | "turn_end") if record["message"]["role"] == "assistant" => {
                self.latest_stop_reason = Some(
                    record["message"]["stopReason"]
                        .as_str()
                        .ok_or_else(|| {
                            invalid_protocol("Pi assistant message is missing stopReason")
                        })?
                        .into(),
                );
            }
            _ => {}
        }
        Ok(())
    }

    fn ready_to_finish(&self) -> bool {
        self.settled && self.prompt_id.is_none() && self.steer.is_none() && self.stop.is_none()
    }
}

impl<R: AsyncBufRead + Unpin, W: AsyncWrite + Unpin, O: AsyncWrite + Unpin> PiClient<R, W, O> {
    pub(super) async fn run_session<F: Future<Output = ()>>(
        &mut self,
        prompt: &str,
        commands: &mut Receiver<Command>,
        interrupt: F,
        timeout: Duration,
    ) -> io::Result<RunOutcome> {
        let prompt_deadline = Instant::now() + timeout;
        tokio::pin!(interrupt);
        let mut interrupted = interrupt.as_mut().now_or_never().is_some();
        let mut session = self
            .prepare_session(prompt, commands, interrupted, prompt_deadline)
            .await?;
        let cancelled_before_prompt = session.stop.is_some();
        loop {
            self.start_stop(&mut session).await?;
            if cancelled_before_prompt && session.stop.is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Pi cancelled before prompt submission",
                ));
            }
            if session.ready_to_finish() {
                return session.outcome();
            }
            let expires = async {
                match session.deadline() {
                    Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                _ = expires => {
                    if session.stop.as_ref().is_some_and(|stop| matches!(stop.phase, StopPhase::Waiting) && Instant::now() >= stop.deadline) {
                        let mut stop = session.stop.take().unwrap();
                        if let Some(request) = &mut stop.request {
                            request.respond(Err(CommandError::Rejected { code: "timeout", message: "Stop expired before Pi dispatch".into() }));
                            continue;
                        }
                    }
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "Pi command acknowledgement timed out; control acceptance is unknown; do not retry automatically"));
                },
                _ = &mut interrupt, if !interrupted => {
                    interrupted = true;
                    if session.stop.is_none() {
                        session.stop = Some(PendingStop { phase: StopPhase::Waiting, deadline: Instant::now() + timeout, request: None });
                    }
                }
                record = self.receive() => {
                    let record = record?;
                    if self.handle_response(&record, &mut session, interrupted).await? {
                        return Ok(RunOutcome::Aborted);
                    }
                    session.observe(&record)?;
                    self.handle_event(&record, Instant::now() + timeout).await?;
                }
                Some(command) = commands.recv() => self.handle_command(command, &mut session).await?,
            }
        }
    }

    async fn prepare_session(
        &mut self,
        prompt: &str,
        commands: &mut Receiver<Command>,
        interrupted: bool,
        prompt_deadline: Instant,
    ) -> io::Result<Session> {
        let mut session = Session {
            prompt_id: None,
            prompt_deadline,
            steer: None,
            stop: None,
            settled: false,
            latest_stop_reason: None,
        };
        if interrupted {
            session.stop = Some(PendingStop {
                phase: StopPhase::Waiting,
                deadline: prompt_deadline,
                request: None,
            });
        }
        while let Ok(command) = commands.try_recv() {
            match command {
                Command::Stop(request) if session.stop.is_none() => {
                    if Instant::now() < request.deadline {
                        session.stop = Some(PendingStop {
                            phase: StopPhase::Waiting,
                            deadline: request.deadline,
                            request: Some(request),
                        });
                    } else {
                        let mut request = request;
                        request.respond(Err(CommandError::Rejected {
                            code: "timeout",
                            message: "Stop expired before Pi dispatch".into(),
                        }));
                    }
                }
                Command::Steer { mut request, .. } | Command::Stop(mut request) => {
                    request.respond(Err(CommandError::Rejected {
                        code: "busy",
                        message: "Pi is starting or stopping".into(),
                    }));
                }
            }
        }
        if session.stop.is_none() {
            session.prompt_id = Some(
                self.send("prompt", json!({"message":prompt}), prompt_deadline)
                    .await?,
            );
        }
        Ok(session)
    }

    async fn handle_command(&mut self, command: Command, session: &mut Session) -> io::Result<()> {
        let (mut request, prompt) = match command {
            Command::Steer { prompt, request } => (request, Some(prompt)),
            Command::Stop(request) => (request, None),
        };
        let rejection = if Instant::now() >= request.deadline {
            Some(("timeout", "Control request expired before Pi dispatch"))
        } else if session.stop.is_some() {
            Some(("stopping", "Pi stop is already pending"))
        } else if session.settled {
            Some((
                "agent_exited",
                "Pi already settled; inspect agent status with waap agent get",
            ))
        } else if prompt.is_some() && (session.prompt_id.is_some() || session.steer.is_some()) {
            Some(("busy", "Pi prompt or steering acknowledgement is pending"))
        } else {
            None
        };
        if let Some((code, message)) = rejection {
            request.respond(Err(CommandError::Rejected {
                code,
                message: message.into(),
            }));
            return Ok(());
        }
        if let Some(prompt) = prompt {
            let id = self
                .send("steer", json!({"message":prompt}), request.deadline)
                .await?;
            session.steer = Some(PendingSteer { id, request });
        } else {
            session.stop = Some(PendingStop {
                phase: StopPhase::Waiting,
                deadline: request.deadline,
                request: Some(request),
            });
        }
        Ok(())
    }

    async fn start_stop(&mut self, session: &mut Session) -> io::Result<()> {
        let Some(stop) = &mut session.stop else {
            return Ok(());
        };
        if !matches!(stop.phase, StopPhase::Waiting)
            || session.prompt_id.is_some()
            || session.steer.is_some()
        {
            return Ok(());
        }
        if Instant::now() >= stop.deadline {
            if let Some(request) = &mut stop.request {
                request.respond(Err(CommandError::Rejected {
                    code: "timeout",
                    message: "Stop expired before Pi dispatch".into(),
                }));
            }
            session.stop = None;
            return Ok(());
        }
        let id = self.send("clear_queue", json!({}), stop.deadline).await?;
        stop.phase = StopPhase::Clearing(id);
        Ok(())
    }

    async fn handle_response(
        &mut self,
        record: &Value,
        session: &mut Session,
        interrupted: bool,
    ) -> io::Result<bool> {
        if record["type"] != "response" {
            return Ok(false);
        }
        let Some(id) = record["id"].as_str() else {
            return Ok(false);
        };
        if session.prompt_id.as_deref() == Some(id) {
            validate_response(record, "prompt")?.map_err(io::Error::other)?;
            session.prompt_id = None;
            match record["data"]["disposition"].as_str() {
                Some("started" | "queued") => {}
                Some("handled") => {
                    return Err(invalid_protocol(
                        "Pi initial prompt was handled without starting an agent run",
                    ))
                }
                disposition => {
                    return Err(invalid_protocol(format!(
                        "Pi initial prompt has missing or unknown disposition: {disposition:?}"
                    )))
                }
            }
            return Ok(false);
        }
        if session.steer.as_ref().is_some_and(|steer| steer.id == id) {
            let result = control_result(record, "steer")?;
            if result.is_ok()
                && !matches!(
                    record["data"]["disposition"].as_str(),
                    Some("queued" | "handled")
                )
            {
                return Err(invalid_protocol(
                    "Pi steer response has missing or unknown disposition",
                ));
            }
            session.steer.take().unwrap().request.respond(result);
            return Ok(false);
        }
        let Some(stop) = &mut session.stop else {
            return Ok(false);
        };
        let command = match &stop.phase {
            StopPhase::Clearing(expected) if expected == id => "clear_queue",
            StopPhase::Aborting(expected) if expected == id => "abort",
            _ => return Ok(false),
        };
        let result = control_result(record, command)?;
        if result.is_err() || command == "abort" {
            let accepted = result.is_ok();
            if let Some(request) = &mut stop.request {
                request.respond(result);
            }
            if !accepted && (interrupted || stop.request.is_none()) {
                return Err(io::Error::other("Pi rejected owner interruption"));
            }
            session.stop = None;
            return Ok(accepted);
        }
        let id = self.send("abort", json!({}), stop.deadline).await?;
        stop.phase = StopPhase::Aborting(id);
        Ok(false)
    }
}

fn control_result(record: &Value, command: &str) -> io::Result<Result<(), CommandError>> {
    Ok(
        validate_response(record, command)?.map_err(|message| CommandError::Rejected {
            code: "backend_rejected",
            message: format!("Pi {command} rejected: {message}"),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::commands::CommandDispatcher;
    use tokio::io::{
        duplex, AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf,
    };

    type Peer = (BufReader<ReadHalf<DuplexStream>>, WriteHalf<DuplexStream>);

    type Driver = PiClient<BufReader<ReadHalf<DuplexStream>>, WriteHalf<DuplexStream>, Vec<u8>>;

    fn pair() -> (Driver, Peer) {
        let (driver, peer) = duplex(1024 * 128);
        let (read, write) = tokio::io::split(driver);
        let (peer_read, peer_write) = tokio::io::split(peer);
        (
            PiClient::new(BufReader::new(read), write, Vec::new()),
            (BufReader::new(peer_read), peer_write),
        )
    }

    async fn read(peer: &mut Peer, expected: &str) -> Value {
        let mut line = String::new();
        peer.0.read_line(&mut line).await.unwrap();
        let record: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(record["type"], expected);
        record
    }

    async fn emit(peer: &mut Peer, record: Value) {
        let mut bytes = serde_json::to_vec(&record).unwrap();
        bytes.push(b'\n');
        peer.1.write_all(&bytes).await.unwrap();
    }

    async fn reply(peer: &mut Peer, command: &Value, success: bool) {
        emit(peer, json!({"type":"response","id":command["id"],"command":command["type"],"success":success,"error":"denied", "data":{"disposition": if command["type"] == "prompt" {"started"} else {"queued"}}})).await;
    }

    async fn settle(peer: &mut Peer, reason: &str) {
        emit(
            peer,
            json!({"type":"message_end", "message":{"role":"assistant", "stopReason":reason}}),
        )
        .await;
        emit(peer, json!({"type":"agent_settled"})).await;
    }

    async fn accept_prompt(peer: &mut Peer) {
        let prompt = read(peer, "prompt").await;
        reply(peer, &prompt, true).await;
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(2)
    }

    #[tokio::test]
    async fn steer_preserves_text_correlates_and_waits_for_ack_in_both_settlement_orders() {
        for settle_first in [false, true] {
            let (mut client, mut peer) = pair();
            let (dispatcher, mut receiver) = CommandDispatcher::create_channel();
            let prompt = "  snow 雪\n\"quoted\" '$HOME' `echo hi` \u{2028}  ";
            let (outcome, ()) = tokio::join!(
                client.run_session(
                    "task",
                    &mut receiver,
                    std::future::pending(),
                    Duration::from_secs(2)
                ),
                async {
                    accept_prompt(&mut peer).await;
                    let steering = dispatcher.steer(prompt, deadline());
                    let (result, ()) = tokio::join!(steering, async {
                        let steer = read(&mut peer, "steer").await;
                        assert_eq!(steer["message"], prompt);
                        emit(&mut peer, json!({"type":"response","id":"unrelated","command":"steer","success":true})).await;
                        emit(&mut peer, json!({"type":"agent_end","willRetry":true})).await;
                        if settle_first {
                            settle(&mut peer, "stop").await;
                        }
                        reply(&mut peer, &steer, true).await;
                    });
                    assert_eq!(result, Ok(()));
                    if !settle_first {
                        settle(&mut peer, "stop").await;
                    }
                }
            );
            assert_eq!(outcome.unwrap(), RunOutcome::Completed);
        }
    }

    #[tokio::test]
    async fn stop_waits_for_slow_steer_preflight_clears_then_aborts_and_drains_settlement() {
        let (mut client, mut peer) = pair();
        let (dispatcher, mut receiver) = CommandDispatcher::create_channel();
        let (outcome, ()) = tokio::join!(
            client.run_session(
                "task",
                &mut receiver,
                std::future::pending(),
                Duration::from_secs(2)
            ),
            async {
                accept_prompt(&mut peer).await;
                let (steer_seen, wait_for_steer) = tokio::sync::oneshot::channel();
                let (steered, stopped, ()) = tokio::join!(
                    dispatcher.steer("change", deadline()),
                    async {
                        wait_for_steer.await.unwrap();
                        dispatcher.stop(deadline()).await
                    },
                    async {
                        let steer = read(&mut peer, "steer").await;
                        steer_seen.send(()).unwrap();
                        assert!(
                            tokio::time::timeout(Duration::from_millis(30), peer.0.fill_buf())
                                .await
                                .is_err()
                        );
                        reply(&mut peer, &steer, true).await;
                        let clear = read(&mut peer, "clear_queue").await;
                        reply(&mut peer, &clear, true).await;
                        let abort = read(&mut peer, "abort").await;
                        settle(&mut peer, "aborted").await;
                        tokio::task::yield_now().await;
                        reply(&mut peer, &abort, true).await;
                    }
                );
                assert_eq!(steered, Ok(()));
                assert_eq!(stopped, Ok(()));
            }
        );
        assert_eq!(outcome.unwrap(), RunOutcome::Aborted);
    }

    #[tokio::test]
    async fn pre_prompt_stop_and_already_ready_signal_suppress_prompt_submission() {
        for signal in [false, true] {
            let (mut client, mut peer) = pair();
            let (_dispatcher, mut receiver) = CommandDispatcher::create_channel();
            let (request, response) = CommandRequest::new(deadline());
            let response = if signal {
                drop(request);
                None
            } else {
                let (sender, channel) = tokio::sync::mpsc::channel(1);
                sender.try_send(Command::Stop(request)).ok().unwrap();
                receiver = channel;
                Some(response)
            };
            let interrupt = async {
                if !signal {
                    std::future::pending::<()>().await;
                }
            };
            let (outcome, ()) = tokio::join!(
                client.run_session(
                    "never sent",
                    &mut receiver,
                    interrupt,
                    Duration::from_secs(2)
                ),
                async {
                    let clear = read(&mut peer, "clear_queue").await;
                    reply(&mut peer, &clear, true).await;
                    let abort = read(&mut peer, "abort").await;
                    reply(&mut peer, &abort, true).await;
                }
            );
            assert_eq!(outcome.unwrap(), RunOutcome::Aborted);
            if let Some(response) = response {
                assert_eq!(response.await.unwrap(), Ok(()));
            }
        }
    }

    #[tokio::test]
    async fn handled_prompt_fails_without_waiting_even_if_extension_started_independent_work() {
        for extension_work in [false, true] {
            let (mut client, mut peer) = pair();
            let (_dispatcher, mut receiver) = CommandDispatcher::create_channel();
            let (outcome, ()) = tokio::join!(
                client.run_session(
                    "task",
                    &mut receiver,
                    std::future::pending(),
                    Duration::from_secs(2)
                ),
                async {
                    let prompt = read(&mut peer, "prompt").await;
                    if extension_work {
                        emit(&mut peer, json!({"type":"agent_start"})).await;
                    }
                    emit(&mut peer, json!({"type":"response","id":prompt["id"],"command":"prompt","success":true,"data":{"disposition":"handled"}})).await;
                }
            );
            assert!(outcome.unwrap_err().to_string().contains("handled"));
        }
    }

    #[tokio::test]
    async fn native_rejections_are_definite_and_do_not_abort_a_successful_run() {
        for operation in ["steer", "clear_queue", "abort"] {
            let (mut client, mut peer) = pair();
            let (dispatcher, mut receiver) = CommandDispatcher::create_channel();
            let (outcome, ()) = tokio::join!(
                client.run_session(
                    "task",
                    &mut receiver,
                    std::future::pending(),
                    Duration::from_secs(2)
                ),
                async {
                    accept_prompt(&mut peer).await;
                    let dispatch = async {
                        if operation == "steer" {
                            dispatcher.steer("rejected", deadline()).await
                        } else {
                            dispatcher.stop(deadline()).await
                        }
                    };
                    let (result, ()) = tokio::join!(dispatch, async {
                        let first = if operation == "steer" {
                            "steer"
                        } else {
                            "clear_queue"
                        };
                        let command = read(&mut peer, first).await;
                        if operation == "abort" {
                            reply(&mut peer, &command, true).await;
                            let abort = read(&mut peer, "abort").await;
                            reply(&mut peer, &abort, false).await;
                        } else {
                            reply(&mut peer, &command, false).await;
                        }
                    });
                    assert!(matches!(
                        result,
                        Err(CommandError::Rejected {
                            code: "backend_rejected",
                            ..
                        })
                    ));
                    settle(&mut peer, "stop").await;
                }
            );
            assert_eq!(outcome.unwrap(), RunOutcome::Completed);
        }
    }

    #[tokio::test]
    async fn lost_malformed_or_missing_ack_is_unknown_and_never_retried() {
        for failure in ["eof", "timeout", "malformed"] {
            let (mut client, mut peer) = pair();
            let (dispatcher, mut receiver) = CommandDispatcher::create_channel();
            let (outcome, ()) = tokio::join!(
                client.run_session(
                    "task",
                    &mut receiver,
                    std::future::pending(),
                    Duration::from_secs(2)
                ),
                async {
                    accept_prompt(&mut peer).await;
                    let (result, ()) = tokio::join!(
                        dispatcher.stop(Instant::now() + Duration::from_millis(50)),
                        async {
                            let clear = read(&mut peer, "clear_queue").await;
                            reply(&mut peer, &clear, true).await;
                            let abort = read(&mut peer, "abort").await;
                            settle(&mut peer, "aborted").await;
                            match failure {
                        "eof" => peer.1.shutdown().await.unwrap(),
                        "malformed" => emit(&mut peer, json!({"type":"response", "id":abort["id"],"command":"steer","success":true})).await,
                        _ => {},
                    }
                        }
                    );
                    assert!(matches!(
                        result,
                        Err(CommandError::AcceptanceUnknown { .. })
                    ));
                }
            );
            assert!(outcome.is_err());
        }
    }

    #[tokio::test]
    async fn blocked_partial_native_write_is_bounded_and_unknown() {
        let (driver, mut peer) = duplex(32);
        let (read, write) = tokio::io::split(driver);
        let mut client = PiClient::new(BufReader::new(read), write, Vec::new());
        let deadline = Instant::now() + Duration::from_millis(20);
        let error = client
            .send("steer", json!({"message":"x".repeat(1024)}), deadline)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(error.to_string().contains("unknown"));
        use tokio::io::AsyncReadExt;
        let mut bytes = [0; 32];
        peer.read_exact(&mut bytes).await.unwrap();
        assert!(!bytes.contains(&b'\n'));
        assert!(
            tokio::time::timeout(Duration::from_millis(10), peer.read(&mut bytes))
                .await
                .is_err()
        );
    }

    #[test]
    fn outcome_uses_final_assistant_reason_and_rejects_missing_or_unknown() {
        let mut session = Session {
            prompt_id: None,
            prompt_deadline: deadline(),
            steer: None,
            stop: None,
            settled: true,
            latest_stop_reason: None,
        };
        assert!(session.outcome().is_err());
        for (reason, expected) in [
            ("stop", RunOutcome::Completed),
            ("aborted", RunOutcome::Aborted),
            ("error", RunOutcome::Failed(ExitCode::FAILURE)),
            ("length", RunOutcome::Failed(ExitCode::FAILURE)),
            ("toolUse", RunOutcome::Failed(ExitCode::FAILURE)),
        ] {
            session.latest_stop_reason = Some(reason.into());
            assert_eq!(session.outcome().unwrap(), expected);
        }
        session.latest_stop_reason = Some("future".into());
        assert!(session.outcome().is_err());
    }
}
