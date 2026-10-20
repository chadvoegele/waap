use std::collections::VecDeque;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::Duration;

use super::*;

type SteeringResult = Result<(), SteeringError>;

pub(super) struct SteeringMailbox {
    pub(super) active: AtomicBool,
    sender: SyncSender<SteerRequest>,
    receiver: Mutex<Option<Receiver<SteerRequest>>>,
}

#[derive(Default)]
struct Delivery {
    dispatched: bool,
    cancelled: bool,
}

pub(super) struct SteerRequest {
    prompt: String,
    deadline: Instant,
    delivery: Arc<Mutex<Delivery>>,
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
    pub(super) fn new() -> Self {
        let (sender, receiver) = mpsc::sync_channel(1);
        Self {
            active: AtomicBool::new(false),
            sender,
            receiver: Mutex::new(Some(receiver)),
        }
    }

    pub(super) fn take_receiver(&self) -> io::Result<Receiver<SteerRequest>> {
        self.receiver
            .lock()
            .map_err(|_| io::Error::other("Codex steering lock poisoned"))?
            .take()
            .ok_or_else(|| io::Error::other("Codex run already waited"))
    }

    pub(super) fn steer(&self, prompt: &str, deadline: Instant) -> SteeringResult {
        if !self.active.load(Ordering::Acquire) {
            return Err(reject(
                "no_active_turn",
                "Codex has no active turn to steer",
            ));
        }
        if Instant::now() >= deadline {
            return Err(reject(
                "timeout",
                "Steering request expired before Codex dispatch",
            ));
        }
        let delivery = Arc::new(Mutex::new(Delivery::default()));
        let (reply, replies) = mpsc::sync_channel(1);
        let request = SteerRequest {
            prompt: prompt.to_string(),
            deadline,
            delivery: Arc::clone(&delivery),
            reply: Some(reply),
        };
        self.sender.try_send(request).map_err(|error| match error {
            mpsc::TrySendError::Full(_) => reject("busy", "Codex steering dispatch is busy"),
            mpsc::TrySendError::Disconnected(_) => reject(
                "agent_exited",
                "Codex protocol owner exited before steering dispatch",
            ),
        })?;
        match replies.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(result) => result,
            Err(_) => {
                let mut delivery = delivery.lock().unwrap();
                delivery.cancelled = true;
                if delivery.dispatched {
                    Err(unknown(
                        "acknowledgement was not received before the deadline",
                    ))
                } else {
                    Err(reject(
                        "timeout",
                        "Steering request expired before Codex dispatch",
                    ))
                }
            }
        }
    }
}

impl SteerRequest {
    fn respond(&mut self, result: SteeringResult) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(result);
        }
    }

    fn expire(&mut self, now: Instant) {
        if now < self.deadline || self.reply.is_none() {
            return;
        }
        let dispatched = {
            let mut delivery = self.delivery.lock().unwrap();
            delivery.cancelled = true;
            delivery.dispatched
        };
        self.respond(Err(if dispatched {
            unknown("acknowledgement deadline expired")
        } else {
            reject("timeout", "Steering request expired before Codex dispatch")
        }));
    }
}

impl Drop for SteerRequest {
    fn drop(&mut self) {
        let dispatched = self.delivery.lock().unwrap().dispatched;
        self.respond(Err(if dispatched {
            unknown("the active turn or protocol connection ended before acknowledgement")
        } else {
            reject("agent_exited", "Codex exited before steering dispatch")
        }));
    }
}

pub(super) fn set_nonblocking(pipe: &impl AsRawFd) -> io::Result<()> {
    // These descriptors remain owned by the app-server client throughout the call.
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
}

struct LiveTurn<'a> {
    thread_id: &'a str,
    active: &'a AtomicBool,
    start_id: i64,
    turn_id: Option<String>,
    early_completion: Option<(String, io::Result<TurnStatus>)>,
    pending: Option<PendingSteer>,
    writes: VecDeque<PendingWrite>,
    interrupt_id: Option<i64>,
}

impl Drop for LiveTurn<'_> {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
    }
}

impl<R: BufRead, W: Write, O: Write> CodexClient<R, W, O> {
    pub(super) fn run_turn(
        &mut self,
        thread_id: &str,
        prompt: &str,
        interrupt: &AtomicBool,
        active: &AtomicBool,
        commands: Receiver<SteerRequest>,
    ) -> io::Result<TurnStatus> {
        let mut turn = LiveTurn {
            thread_id,
            active,
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
                pending.request.expire(Instant::now());
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
        request.expire(Instant::now());
        if request.reply.is_none() {
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
        turn.pending = Some(PendingSteer { id, request });
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
                pending.request.expire(Instant::now());
            }
            let write = turn.writes.front_mut().unwrap();
            let delivery = turn
                .pending
                .as_ref()
                .filter(|pending| pending.id == write.id)
                .map(|pending| Arc::clone(&pending.request.delivery));
            let mut delivery = delivery.as_ref().map(|delivery| delivery.lock().unwrap());
            if write.offset == 0 && delivery.as_ref().is_some_and(|state| state.cancelled) {
                drop(delivery);
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
                    if let Some(delivery) = &mut delivery {
                        delivery.dispatched = true;
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
            pending.request.expire(Instant::now());
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
        turn.active.store(true, Ordering::Release);
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
mod tests;
