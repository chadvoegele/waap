use std::collections::VecDeque;
use std::env;
use std::io;
#[cfg(test)]
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::Stdio;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex;
#[cfg(test)]
use std::thread;
#[cfg(test)]
use std::time::Duration;
use std::time::Instant;
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt,
};
use tokio::process::{ChildStdin, ChildStdout, Command as ProcessCommand};
use tokio::sync::mpsc::Receiver;

use super::commands::{Command, SteerRequest, SteeringResult};

use serde_json::{json, Value as JsonValue};

use super::backend::{
    AbortContext, AgentSessionRun, RunOutcome, Runner, StartContext, SteeringError,
};
use super::process::AgentProcess;
use super::{AgentRunOptions, ReasoningEffort};

#[derive(Default)]
pub(super) struct CodexBackend {
    config: CodexRunConfig,
}

impl CodexBackend {
    pub(super) fn from_env(options: &AgentRunOptions) -> io::Result<Self> {
        Ok(Self {
            config: get_codex_run_config(options)?,
        })
    }

    pub(super) async fn start(
        &mut self,
        context: StartContext<'_>,
        receiver: Receiver<Command>,
    ) -> io::Result<AgentSessionRun> {
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut server = spawn_codex_app_server(&self.config, context.worktree_dir)?;
        let startup = tokio::select! {
            result = async {
                server.client.initialize().await?;
                server.client.thread_start(context.worktree_dir).await
            } => result,
            _ = interrupt.recv() => Err(io::Error::new(io::ErrorKind::Interrupted, "Codex startup interrupted")),
        };
        let thread_id = match startup {
            Ok(thread_id) => thread_id,
            Err(error) => {
                return super::worktree::collapse_errors(
                    Err(error),
                    server.process.terminate().await,
                )
            }
        };
        Ok(AgentSessionRun::new(
            thread_id.clone(),
            Runner::Codex(Box::new(CodexRunner {
                server,
                interrupt,
                thread_id,
                prompt: context.prompt.to_string(),
            })),
            receiver,
        ))
    }
    pub(super) async fn abort(&mut self, context: AbortContext<'_>) -> io::Result<()> {
        signal_codex_run(context.agent_id).await
    }
}

struct CodexAppServer {
    client: CodexClient<tokio::io::BufReader<ChildStdout>, ChildStdin, tokio::io::Stdout>,
    process: AgentProcess,
}
pub(super) struct CodexRunner {
    server: CodexAppServer,
    interrupt: tokio::signal::unix::Signal,
    thread_id: String,
    prompt: String,
}
impl CodexRunner {
    pub(super) async fn run(
        &mut self,
        mut steering_receiver: Receiver<Command>,
    ) -> io::Result<RunOutcome> {
        let interrupt = &mut self.interrupt;
        let status = self
            .server
            .client
            .run_turn(
                &self.thread_id,
                &self.prompt,
                async {
                    interrupt.recv().await;
                },
                &mut steering_receiver,
            )
            .await;
        if status?.is_success() {
            Ok(RunOutcome::Completed)
        } else {
            Ok(RunOutcome::Failed(std::process::ExitCode::FAILURE))
        }
    }
    pub(super) async fn cleanup(&mut self) -> io::Result<()> {
        self.server.process.terminate().await
    }
}

const METHOD_INITIALIZE: &str = "initialize";
const METHOD_INITIALIZED: &str = "initialized";
const METHOD_THREAD_START: &str = "thread/start";
const METHOD_TURN_START: &str = "turn/start";
const METHOD_TURN_STEER: &str = "turn/steer";
const METHOD_TURN_INTERRUPT: &str = "turn/interrupt";
const METHOD_AGENT_MESSAGE_DELTA: &str = "item/agentMessage/delta";
const METHOD_TURN_COMPLETED: &str = "turn/completed";

const APPROVAL_POLICY_NEVER: &str = "never";
const SANDBOX_DANGER_FULL_ACCESS: &str = "danger-full-access";

#[derive(Debug, Default, PartialEq, Eq)]
struct CodexRunConfig {
    model: Option<String>,
    reasoning_effort: Option<ReasoningEffort>,
}

fn get_codex_reasoning_effort_from_env() -> io::Result<Option<ReasoningEffort>> {
    match env::var("CODEX_REASONING_EFFORT") {
        Ok(value) => ReasoningEffort::parse(&value).map(Some).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "invalid CODEX_REASONING_EFFORT {value:?}; accepted values: {}",
                    ReasoningEffort::labels().join(", ")
                ),
            )
        }),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "invalid CODEX_REASONING_EFFORT; accepted values: {}",
                ReasoningEffort::labels().join(", ")
            ),
        )),
    }
}

fn get_codex_run_config(options: &AgentRunOptions) -> io::Result<CodexRunConfig> {
    let model = options.model.clone().or_else(|| {
        env::var("CODEX_MODEL")
            .ok()
            .filter(|model| !model.is_empty())
    });
    let reasoning_effort = match options.reasoning_effort {
        Some(effort) => Some(effort),
        None => get_codex_reasoning_effort_from_env()?,
    };
    Ok(CodexRunConfig {
        model,
        reasoning_effort,
    })
}

async fn signal_codex_run(agent_id: &str) -> io::Result<()> {
    let mut command = ProcessCommand::new("pkill");
    command
        .arg("-TERM")
        .arg("-f")
        .arg(format!("agent run --agent-id {agent_id}"));
    map_pkill_status(command).await
}

async fn map_pkill_status(mut command: ProcessCommand) -> io::Result<()> {
    let status = command.status().await?;
    match status.code() {
        Some(0) | Some(1) => Ok(()),
        Some(code) => Err(io::Error::other(format!("pkill exited with status {code}"))),
        None => Err(io::Error::other("pkill terminated by signal")),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TurnStatus {
    Completed,
    Interrupted,
    Failed,
    InProgress,
}

impl TurnStatus {
    fn is_success(self) -> bool {
        matches!(self, TurnStatus::Completed)
    }

    fn from_wire(value: &str) -> Option<TurnStatus> {
        match value {
            "completed" => Some(TurnStatus::Completed),
            "interrupted" => Some(TurnStatus::Interrupted),
            "failed" => Some(TurnStatus::Failed),
            "inProgress" => Some(TurnStatus::InProgress),
            _ => None,
        }
    }
}

fn initialize_params() -> JsonValue {
    json!({
        "clientInfo": {
            "name": "waap",
            "version": env!("CARGO_PKG_VERSION"),
        }
    })
}

fn thread_start_params(cwd: &Path, model: Option<&str>) -> JsonValue {
    let mut params = json!({
        "cwd": cwd.display().to_string(),
        "approvalPolicy": APPROVAL_POLICY_NEVER,
        "sandbox": SANDBOX_DANGER_FULL_ACCESS,
    });
    if let Some(model) = model {
        params["model"] = JsonValue::String(model.to_string());
    }
    params
}

fn turn_start_params(
    thread_id: &str,
    prompt: &str,
    model: Option<&str>,
    reasoning_effort: Option<ReasoningEffort>,
) -> JsonValue {
    let mut params = json!({
        "threadId": thread_id,
        "input": [{ "type": "text", "text": prompt }],
    });
    if let Some(model) = model {
        params["model"] = JsonValue::String(model.to_string());
    }
    if let Some(effort) = reasoning_effort {
        params["effort"] = JsonValue::String(effort.as_str().to_string());
    }
    params
}

fn request_line(id: i64, method: &str, params: JsonValue) -> String {
    let mut line = serde_json::to_string(&json!({
        "id": id,
        "method": method,
        "params": params,
    }))
    .expect("request value serializes");
    line.push('\n');
    line
}

fn notification_line(method: &str) -> String {
    let mut line =
        serde_json::to_string(&json!({ "method": method })).expect("notification value serializes");
    line.push('\n');
    line
}

fn delta_for_turn<'a>(message: &'a JsonValue, thread_id: &str, turn_id: &str) -> Option<&'a str> {
    if message.get("method").and_then(JsonValue::as_str)? != METHOD_AGENT_MESSAGE_DELTA {
        return None;
    }
    let params = message.get("params")?;
    if params.get("threadId").and_then(JsonValue::as_str)? != thread_id {
        return None;
    }
    if params.get("turnId").and_then(JsonValue::as_str)? != turn_id {
        return None;
    }
    params.get("delta").and_then(JsonValue::as_str)
}

fn completed_status_for_turn(
    message: &JsonValue,
    thread_id: &str,
    turn_id: &str,
) -> Option<io::Result<TurnStatus>> {
    if message.get("method").and_then(JsonValue::as_str)? != METHOD_TURN_COMPLETED {
        return None;
    }
    let params = message.get("params")?;
    if params.get("threadId").and_then(JsonValue::as_str)? != thread_id {
        return None;
    }
    let turn = params.get("turn")?;
    if turn.get("id").and_then(JsonValue::as_str)? != turn_id {
        return None;
    }
    let status = turn.get("status").and_then(JsonValue::as_str);
    Some(status.and_then(TurnStatus::from_wire).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("codex turn/completed has unknown status: {status:?}"),
        )
    }))
}

struct CodexClient<R, W, O> {
    reader: R,
    writer: W,
    out: O,
    next_id: i64,
    model: Option<String>,
    reasoning_effort: Option<ReasoningEffort>,
}

fn spawn_codex_app_server(
    config: &CodexRunConfig,
    worktree_dir: &Path,
) -> io::Result<CodexAppServer> {
    let mut command = codex_app_server_command(worktree_dir);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut process = AgentProcess::spawn(&mut command)?;
    let writer = process.take_stdin()?;
    let stdout = process.take_stdout()?;
    Ok(CodexAppServer {
        client: CodexClient {
            reader: tokio::io::BufReader::new(stdout),
            writer,
            out: tokio::io::stdout(),
            next_id: 0,
            model: config.model.clone(),
            reasoning_effort: config.reasoning_effort,
        },
        process,
    })
}

fn codex_app_server_command(worktree_dir: &Path) -> ProcessCommand {
    let mut command = ProcessCommand::new("codex");
    command
        .arg("app-server")
        .arg("--stdio")
        .current_dir(worktree_dir);
    command
}

impl<R: AsyncBufRead + Unpin, W: AsyncWrite + Unpin, O: AsyncWrite + Unpin> CodexClient<R, W, O> {
    async fn write_line(&mut self, line: &str) -> io::Result<()> {
        self.writer.write_all(line.as_bytes()).await?;
        self.writer.flush().await
    }

    async fn read_message(&mut self) -> io::Result<Option<JsonValue>> {
        let mut line = String::new();
        loop {
            line.clear();
            if self.reader.read_line(&mut line).await? == 0 {
                return Ok(None);
            }
            if line.trim().is_empty() {
                continue;
            }
            let message: JsonValue = serde_json::from_str(&line).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("codex app-server sent malformed JSON: {error}"),
                )
            })?;
            return Ok(Some(message));
        }
    }

    async fn send_request(&mut self, method: &str, params: JsonValue) -> io::Result<JsonValue> {
        let id = self.next_id;
        self.next_id += 1;
        self.write_line(&request_line(id, method, params)).await?;

        loop {
            let message = self.read_message().await?.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("codex app-server closed before responding to request {id}"),
                )
            })?;

            if message.get("method").is_some() {
                if message.get("id").is_none() {
                    self.forward_delta(&message).await?;
                }
                continue;
            }

            if message.get("id").and_then(JsonValue::as_i64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                return Err(io::Error::other(format!(
                    "codex app-server returned an error for request {id}: {error}"
                )));
            }
            return Ok(message.get("result").cloned().unwrap_or(JsonValue::Null));
        }
    }

    async fn initialize(&mut self) -> io::Result<()> {
        self.send_request(METHOD_INITIALIZE, initialize_params())
            .await?;
        self.write_line(&notification_line(METHOD_INITIALIZED))
            .await
    }

    async fn thread_start(&mut self, cwd: &Path) -> io::Result<String> {
        let params = thread_start_params(cwd, self.model.as_deref());
        let result = self.send_request(METHOD_THREAD_START, params).await?;
        result
            .pointer("/thread/id")
            .and_then(JsonValue::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "codex thread/start response is missing thread.id",
                )
            })
    }
}

fn reject(code: &'static str, message: &str) -> SteeringError {
    SteeringError::Rejected {
        code,
        message: message.to_string(),
    }
}

fn build_acceptance_unknown_error(reason: &str) -> SteeringError {
    SteeringError::AcceptanceUnknown {
        message: format!(
            "Codex steering acceptance is unknown: {reason}; do not retry automatically"
        ),
    }
}

struct PendingWrite {
    id: i64,
    bytes: Vec<u8>,
    offset: usize,
}

struct PendingSteer {
    id: i64,
    request: SteerRequest,
    write_started: bool,
}

impl PendingSteer {
    fn expire(&mut self, now: Instant) {
        if now >= self.request.deadline && self.request.has_reply_sender() {
            self.request.respond(Err(if self.write_started {
                build_acceptance_unknown_error("acknowledgement deadline expired")
            } else {
                reject("timeout", "Steering request expired before Codex dispatch")
            }));
        }
    }
}

struct TurnState<'a> {
    thread_id: &'a str,
    start_request_id: i64,
    turn_id: Option<String>,
    early_completion: Option<(String, io::Result<TurnStatus>)>,
    pending_steer: Option<PendingSteer>,
    pending_writes: VecDeque<PendingWrite>,
    interrupt_request_id: Option<i64>,
}

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin, O: AsyncWrite + Unpin> CodexClient<R, W, O> {
    async fn run_turn(
        &mut self,
        thread_id: &str,
        prompt: &str,
        interrupt: impl std::future::Future<Output = ()>,
        steering_receiver: &mut Receiver<Command>,
    ) -> io::Result<TurnStatus> {
        let mut turn = TurnState {
            thread_id,
            start_request_id: self.next_id,
            turn_id: None,
            early_completion: None,
            pending_steer: None,
            pending_writes: VecDeque::new(),
            interrupt_request_id: None,
        };
        self.enqueue_request(
            &mut turn,
            METHOD_TURN_START,
            turn_start_params(
                thread_id,
                prompt,
                self.model.as_deref(),
                self.reasoning_effort,
            ),
        );
        tokio::pin!(interrupt);
        let mut interrupted = false;
        let mut channel_open = true;
        let mut chunk = [0; 8192];
        let mut buffer = Vec::new();
        loop {
            if let Some(status) = self.process_frames(&mut turn, &mut buffer).await? {
                return status;
            }
            self.prepare_writes(&mut turn);
            if interrupted && turn.interrupt_request_id.is_none() {
                if let Some(turn_id) = &turn.turn_id {
                    let params = json!({ "threadId": thread_id, "turnId": turn_id });
                    turn.interrupt_request_id =
                        Some(self.enqueue_request(&mut turn, METHOD_TURN_INTERRUPT, params));
                }
            }
            let deadline = turn
                .pending_steer
                .as_ref()
                .filter(|pending| pending.request.has_reply_sender())
                .map(|pending| pending.request.deadline);
            let bytes = turn
                .pending_writes
                .front()
                .map_or(&[][..], |write| &write.bytes[write.offset..]);
            tokio::select! {
                result = self.reader.read(&mut chunk) => {
                    match result {
                        Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "codex app-server closed before the turn completed")),
                        Ok(count) => buffer.extend_from_slice(&chunk[..count]),
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {},
                        Err(error) => return Err(error),
                    }
                }
                result = self.writer.write(bytes), if !bytes.is_empty() => {
                    match result {
                        Ok(count) => Self::record_write(&mut turn, count)?,
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {},
                        Err(error) => return Err(error),
                    }
                }
                request = steering_receiver.recv(), if channel_open => {
                    match request {
                        Some(Command::Steer(request)) => self.dispatch_steering(&mut turn, request),
                        None => channel_open = false,
                    }
                }
                _ = &mut interrupt, if !interrupted => interrupted = true,
                _ = wait_for_deadline(deadline) => {}
            }
        }
    }
}

async fn wait_for_deadline(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
        None => std::future::pending::<()>().await,
    }
}

impl<R, W, O: AsyncWrite + Unpin> CodexClient<R, W, O> {
    async fn forward_delta(&mut self, message: &JsonValue) -> io::Result<()> {
        if message.get("method").and_then(JsonValue::as_str) == Some(METHOD_AGENT_MESSAGE_DELTA) {
            if let Some(delta) = message.pointer("/params/delta").and_then(JsonValue::as_str) {
                self.out.write_all(delta.as_bytes()).await?;
                self.out.flush().await?;
            }
        }
        Ok(())
    }

    async fn process_frames(
        &mut self,
        turn: &mut TurnState<'_>,
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
            if let Some(status) = self.process_message(turn, &message).await? {
                return Ok(Some(status));
            }
        }
        buffer.drain(..consumed);
        Ok(None)
    }

    fn dispatch_steering(&mut self, turn: &mut TurnState<'_>, mut request: SteerRequest) {
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
        if turn.pending_steer.is_some() {
            request.respond(Err(reject(
                "busy",
                "Codex is still awaiting an earlier steering acknowledgement",
            )));
            return;
        }
        let params = json!({
            "threadId": turn.thread_id,
            "expectedTurnId": turn_id,
            "input": [{ "type": "text", "text": request.message }],
        });
        let id = self.enqueue_request(turn, METHOD_TURN_STEER, params);
        turn.pending_steer = Some(PendingSteer {
            id,
            request,
            write_started: false,
        });
    }

    fn enqueue_request(
        &mut self,
        turn: &mut TurnState<'_>,
        method: &str,
        params: JsonValue,
    ) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        turn.pending_writes.push_back(PendingWrite {
            id,
            bytes: request_line(id, method, params).into_bytes(),
            offset: 0,
        });
        id
    }

    fn prepare_writes(&mut self, turn: &mut TurnState<'_>) {
        if let Some(pending) = &mut turn.pending_steer {
            pending.expire(Instant::now());
        }
        while let Some(write) = turn.pending_writes.front() {
            if write.offset == 0
                && turn.pending_steer.as_ref().is_some_and(|pending| {
                    pending.id == write.id && !pending.request.has_reply_sender()
                })
            {
                turn.pending_writes.pop_front();
                turn.pending_steer.take();
            } else {
                break;
            }
        }
    }

    fn record_write(turn: &mut TurnState<'_>, count: usize) -> io::Result<()> {
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "Codex app-server input closed",
            ));
        }
        let write = turn.pending_writes.front_mut().unwrap();
        if let Some(pending) = &mut turn.pending_steer {
            if pending.id == write.id {
                pending.write_started = true;
            }
        }
        write.offset += count;
        if write.offset == write.bytes.len() {
            turn.pending_writes.pop_front();
        }
        Ok(())
    }

    #[cfg(test)]
    async fn flush_requests(&mut self, turn: &mut TurnState<'_>) -> io::Result<()>
    where
        W: AsyncWrite + Unpin,
    {
        loop {
            self.prepare_writes(turn);
            let Some(write) = turn.pending_writes.front() else {
                return Ok(());
            };
            let count = self.writer.write(&write.bytes[write.offset..]).await?;
            Self::record_write(turn, count)?;
        }
    }

    async fn process_message(
        &mut self,
        turn: &mut TurnState<'_>,
        message: &JsonValue,
    ) -> io::Result<Option<io::Result<TurnStatus>>> {
        if message.get("method").is_some() {
            return self.process_notification(turn, message).await;
        }
        let Some(id) = message.get("id").and_then(JsonValue::as_i64) else {
            return Ok(None);
        };
        if id == turn.start_request_id {
            return self.accept_started_turn(turn, message);
        }
        if turn
            .pending_steer
            .as_ref()
            .is_some_and(|pending_steer| pending_steer.id == id)
        {
            let mut pending_steer = turn.pending_steer.take().unwrap();
            pending_steer.expire(Instant::now());
            pending_steer.request.respond(parse_steering_response(
                message,
                turn.turn_id.as_deref().unwrap(),
            ));
        } else if Some(id) == turn.interrupt_request_id && message.get("error").is_some() {
            return Err(io::Error::other("codex turn/interrupt rejected"));
        }
        Ok(None)
    }

    async fn process_notification(
        &mut self,
        turn: &mut TurnState<'_>,
        message: &JsonValue,
    ) -> io::Result<Option<io::Result<TurnStatus>>> {
        if message.get("id").is_some() {
            return Ok(None);
        }
        if let Some(turn_id) = &turn.turn_id {
            if let Some(delta) = delta_for_turn(message, turn.thread_id, turn_id) {
                self.out.write_all(delta.as_bytes()).await?;
                self.out.flush().await?;
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
        self.forward_delta(message).await?;
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
        turn: &mut TurnState<'_>,
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

fn parse_steering_response(message: &JsonValue, expected_turn: &str) -> SteeringResult {
    if let Some(error) = message.get("error") {
        let (Some(code), Some(diagnostic)) = (
            error.get("code").and_then(JsonValue::as_i64),
            error.get("message").and_then(JsonValue::as_str),
        ) else {
            return Err(build_acceptance_unknown_error(
                "the server returned an invalid turn/steer error",
            ));
        };
        if message.get("result").is_some() {
            return Err(build_acceptance_unknown_error(
                "the server returned both a result and an error",
            ));
        }
        let missing = code == -32601
            || (code == -32600
                && (diagnostic.starts_with("Invalid request: unknown variant `turn/steer`")
                    || diagnostic.starts_with("unknown variant `turn/steer`")));
        return Err(if missing {
            reject("unsupported_backend_version", "Codex app-server does not support turn/steer; upgrade Codex to a version with native turn/steer support")
        } else {
            reject("backend_rejected", &format!("Codex rejected turn/steer (RPC error code {code}); verify that the turn is still active"))
        });
    }
    if message
        .pointer("/result/turnId")
        .and_then(JsonValue::as_str)
        != Some(expected_turn)
    {
        return Err(build_acceptance_unknown_error(
            "the server returned an invalid turn/steer acknowledgement",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod steering_tests {
    use std::os::unix::net::UnixStream;

    use super::super::commands::CommandDispatcher;
    use super::*;

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
        CommandDispatcher::channel()
    }

    struct TestMailbox {
        dispatcher: CommandDispatcher,
        finished: AtomicBool,
        output: CapturedOutput,
    }

    impl TestMailbox {
        fn steer(&self, prompt: &str, deadline: Instant) -> SteeringResult {
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
        action: impl FnOnce(
            &mut Server,
            &TestMailbox,
            &tokio::sync::watch::Sender<bool>,
            &CapturedOutput,
        ),
    ) -> (io::Result<TurnStatus>, String) {
        let (stream, server) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        server
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let output = CapturedOutput::default();
        let (dispatcher, steering_receiver) = create_test_dispatcher();
        let mailbox = TestMailbox {
            dispatcher,
            finished: AtomicBool::new(false),
            output: output.clone(),
        };
        let (interrupt, mut interrupts) = tokio::sync::watch::channel(false);
        let result = thread::scope(|scope| {
            let worker = scope.spawn(|| {
                let mut steering_receiver = steering_receiver;
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
                                interrupts.changed().await.unwrap();
                            },
                            &mut steering_receiver,
                        )
                        .await
                });
                drop(steering_receiver);
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
                let (request, reply_receiver) = SteerRequest::new(
                    "expires".to_string(),
                    Instant::now() + Duration::from_millis(100),
                );
                sender.send(Command::Steer(request)).await.unwrap();
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
                    Err(SteeringError::AcceptanceUnknown { .. })
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
                    interrupt.send(true).unwrap();
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
            let (request, mut reply_receiver) = SteerRequest::new("prompt".to_string(), expires);
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
                thread_id: "th",
                start_request_id: 0,
                turn_id: Some("tu".to_string()),
                early_completion: None,
                pending_steer: None,
                pending_writes: VecDeque::new(),
                interrupt_request_id: None,
            };
            let id =
                client.enqueue_request(&mut turn, METHOD_TURN_STEER, json!({"input":"prompt"}));
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
                    Err(SteeringError::AcceptanceUnknown { .. })
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
        let (dispatcher, mut steering_receiver) = create_test_dispatcher();
        assert!(matches!(
            dispatcher
                .steer("queued", Instant::now() + Duration::from_millis(10))
                .await,
            Err(SteeringError::AcceptanceUnknown { .. })
        ));
        let Command::Steer(request) = steering_receiver.recv().await.unwrap();
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
            thread_id: "th",
            start_request_id: 0,
            turn_id: Some("tu".to_string()),
            early_completion: None,
            pending_steer: None,
            pending_writes: VecDeque::new(),
            interrupt_request_id: None,
        };
        client.dispatch_steering(&mut turn, request);
        client.flush_requests(&mut turn).await.unwrap();
        assert!(client.writer.is_empty());
        assert!(turn.pending_steer.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn runner_exit_without_a_reply_is_unknown_and_later_calls_are_rejected() {
        let (dispatcher, mut steering_receiver) = create_test_dispatcher();
        let (result, ()) = tokio::join!(dispatcher.steer("queued", deadline()), async {
            drop(steering_receiver.recv().await.unwrap());
        });
        assert!(matches!(
            result,
            Err(SteeringError::AcceptanceUnknown { .. })
        ));
        drop(steering_receiver);
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

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::path::PathBuf;

    use clap::ValueEnum;

    use super::*;
    use crate::agent::CODEX_ENV_LOCK;

    fn parse(line: &str) -> JsonValue {
        serde_json::from_str(line).expect("line is JSON")
    }

    fn client_over(
        input: &str,
    ) -> CodexClient<tokio::io::BufReader<Cursor<Vec<u8>>>, Vec<u8>, Vec<u8>> {
        CodexClient {
            reader: tokio::io::BufReader::new(Cursor::new(input.as_bytes().to_vec())),
            writer: Vec::new(),
            out: Vec::new(),
            next_id: 0,
            model: None,
            reasoning_effort: None,
        }
    }

    #[test]
    fn request_line_is_single_line_with_id_method_params_and_no_jsonrpc() {
        let line = request_line(7, METHOD_INITIALIZE, initialize_params());

        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);

        let value = parse(&line);
        assert_eq!(value["id"], json!(7));
        assert_eq!(value["method"], json!("initialize"));
        assert!(value.get("params").is_some());
        assert!(value.get("jsonrpc").is_none());
    }

    #[test]
    fn notification_line_has_no_id_and_no_jsonrpc() {
        let line = notification_line(METHOD_INITIALIZED);

        assert!(line.ends_with('\n'));
        let value = parse(&line);
        assert_eq!(value["method"], json!("initialized"));
        assert!(value.get("id").is_none());
        assert!(value.get("params").is_none());
        assert!(value.get("jsonrpc").is_none());
    }

    #[test]
    fn initialize_correlates_response_forwards_delta_and_notifies_server() {
        let input = concat!(
            "{\"id\":9,\"result\":{}}\n",
            "{\"method\":\"item/agentMessage/delta\",\"params\":{\"delta\":\"ready\"}}\n",
            "{\"id\":0,\"result\":{}}\n",
        );
        let mut client = client_over(input);

        crate::agent::runtime::block_on(client.initialize()).unwrap();

        assert_eq!(String::from_utf8(client.out.clone()).unwrap(), "ready");
        let lines = String::from_utf8(client.writer.clone()).unwrap();
        let mut lines = lines.lines();
        let request = parse(lines.next().unwrap());
        assert_eq!(request["id"], json!(0));
        assert_eq!(request["method"], json!("initialize"));
        assert!(request.get("params").is_some());
        let notification = parse(lines.next().unwrap());
        assert_eq!(notification, json!({ "method": "initialized" }));
        assert!(lines.next().is_none());
    }

    #[test]
    fn thread_start_params_encode_never_approvals_and_full_access() {
        let worktree_dir = PathBuf::from("/repo/with space");
        let params = thread_start_params(&worktree_dir, Some("o3"));
        let command = codex_app_server_command(&worktree_dir);

        assert_eq!(params["approvalPolicy"], json!("never"));
        assert_eq!(params["sandbox"], json!("danger-full-access"));
        assert_eq!(params["cwd"], json!("/repo/with space"));
        assert_eq!(params["model"], json!("o3"));
        assert!(params.get("effort").is_none());
        assert!(params.get("reasoningEffort").is_none());
        assert_eq!(command.as_std().get_program(), "codex");
        assert_eq!(
            command.as_std().get_args().collect::<Vec<_>>(),
            ["app-server", "--stdio"]
        );
        assert_eq!(
            command.as_std().get_current_dir(),
            Some(worktree_dir.as_path())
        );
    }

    #[test]
    fn thread_start_params_omit_model_when_unset() {
        let params = thread_start_params(&PathBuf::from("/repo"), None);

        assert!(params.get("model").is_none());
        assert_eq!(params["approvalPolicy"], json!("never"));
        assert_eq!(params["sandbox"], json!("danger-full-access"));
    }

    #[test]
    fn turn_start_params_use_camel_case_thread_id_and_carry_prompt() {
        let params = turn_start_params(
            "th_1",
            "do the thing",
            Some("gpt-5.4"),
            Some(ReasoningEffort::High),
        );

        assert_eq!(params["threadId"], json!("th_1"));
        assert_eq!(params["input"][0]["type"], json!("text"));
        assert_eq!(params["input"][0]["text"], json!("do the thing"));
        assert_eq!(params["model"], json!("gpt-5.4"));
        assert_eq!(params["effort"], json!("high"));
        assert!(params.get("reasoningEffort").is_none());
    }

    #[test]
    fn turn_start_params_omit_model_when_unset() {
        let params = turn_start_params("th_1", "prompt", None, None);

        assert!(params.get("model").is_none());
        assert!(params.get("effort").is_none());
        assert_eq!(params["threadId"], json!("th_1"));
    }

    #[test]
    fn turn_start_params_support_model_and_effort_independently() {
        let model_only = turn_start_params("th", "prompt", Some("gpt-5.4"), None);
        let effort_only = turn_start_params("th", "prompt", None, Some(ReasoningEffort::Ultra));

        assert_eq!(model_only["model"], json!("gpt-5.4"));
        assert!(model_only.get("effort").is_none());
        assert!(effort_only.get("model").is_none());
        assert_eq!(effort_only["effort"], json!("ultra"));
    }

    #[test]
    fn codex_config_uses_environment_fallbacks_and_cli_precedence() {
        let _lock = CODEX_ENV_LOCK.lock().unwrap();
        let previous_model = env::var_os("CODEX_MODEL");
        let previous_effort = env::var_os("CODEX_REASONING_EFFORT");
        env::set_var("CODEX_MODEL", "env-model");
        env::set_var("CODEX_REASONING_EFFORT", "low");

        let fallback = get_codex_run_config(&AgentRunOptions::default()).unwrap();
        let overrides = get_codex_run_config(&AgentRunOptions {
            model: Some("cli-model".to_string()),
            reasoning_effort: Some(ReasoningEffort::High),
        })
        .unwrap();
        env::set_var("CODEX_REASONING_EFFORT", "invalid-but-overridden");
        let invalid_override = get_codex_run_config(&AgentRunOptions {
            model: None,
            reasoning_effort: Some(ReasoningEffort::Max),
        })
        .unwrap();

        assert_eq!(fallback.model.as_deref(), Some("env-model"));
        assert_eq!(fallback.reasoning_effort, Some(ReasoningEffort::Low));
        assert_eq!(overrides.model.as_deref(), Some("cli-model"));
        assert_eq!(overrides.reasoning_effort, Some(ReasoningEffort::High));
        assert_eq!(
            invalid_override.reasoning_effort,
            Some(ReasoningEffort::Max)
        );

        match previous_model {
            Some(value) => env::set_var("CODEX_MODEL", value),
            None => env::remove_var("CODEX_MODEL"),
        }
        match previous_effort {
            Some(value) => env::set_var("CODEX_REASONING_EFFORT", value),
            None => env::remove_var("CODEX_REASONING_EFFORT"),
        }
    }

    #[test]
    fn codex_config_accepts_exact_efforts_and_rejects_invalid_environment_values() {
        let _lock = CODEX_ENV_LOCK.lock().unwrap();
        let previous_model = env::var_os("CODEX_MODEL");
        let previous_effort = env::var_os("CODEX_REASONING_EFFORT");
        env::remove_var("CODEX_MODEL");

        for expected in ReasoningEffort::value_variants() {
            env::set_var("CODEX_REASONING_EFFORT", expected.as_str());
            let config = get_codex_run_config(&AgentRunOptions::default()).unwrap();
            assert_eq!(config.reasoning_effort, Some(*expected));
        }
        for invalid in ["", "HIGH", "extreme"] {
            env::set_var("CODEX_REASONING_EFFORT", invalid);
            let error = get_codex_run_config(&AgentRunOptions::default()).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            let message = error.to_string();
            assert!(message.contains("CODEX_REASONING_EFFORT"));
            for accepted in ReasoningEffort::labels() {
                assert!(message.contains(accepted));
            }
        }

        env::remove_var("CODEX_REASONING_EFFORT");
        assert_eq!(
            get_codex_run_config(&AgentRunOptions::default()).unwrap(),
            CodexRunConfig::default()
        );

        match previous_model {
            Some(value) => env::set_var("CODEX_MODEL", value),
            None => env::remove_var("CODEX_MODEL"),
        }
        match previous_effort {
            Some(value) => env::set_var("CODEX_REASONING_EFFORT", value),
            None => env::remove_var("CODEX_REASONING_EFFORT"),
        }
    }

    #[test]
    fn thread_start_extracts_thread_id_skipping_notifications() {
        let input = concat!(
            "{\"method\":\"thread/started\",\"params\":{\"thread\":{\"id\":\"th_x\"}}}\n",
            "{\"id\":0,\"result\":{\"thread\":{\"id\":\"th_abc\"}}}\n",
        );
        let mut client = client_over(input);

        let thread_id =
            crate::agent::runtime::block_on(client.thread_start(&PathBuf::from("/repo"))).unwrap();

        assert_eq!(thread_id, "th_abc");
        let request = parse(&String::from_utf8(client.writer.clone()).unwrap());
        assert_eq!(request["method"], json!("thread/start"));
    }

    #[test]
    fn thread_start_propagates_server_error() {
        let input = "{\"id\":0,\"error\":{\"code\":-32603,\"message\":\"boom\"}}\n";
        let mut client = client_over(input);

        let error = crate::agent::runtime::block_on(client.thread_start(&PathBuf::from("/repo")))
            .expect_err("server error must propagate");

        assert!(error.to_string().contains("boom"));
    }

    #[test]
    fn signal_status_maps_zero_and_one_to_ok_and_other_to_err() {
        for code in [0, 1] {
            let mut command = ProcessCommand::new("sh");
            command.arg("-c").arg(format!("exit {code}"));
            assert!(
                crate::agent::runtime::block_on(map_pkill_status(command)).is_ok(),
                "exit {code} should be Ok"
            );
        }

        let mut command = ProcessCommand::new("sh");
        command.arg("-c").arg("exit 2");
        let error = crate::agent::runtime::block_on(map_pkill_status(command))
            .expect_err("exit 2 should be Err");
        assert!(error.to_string().contains("status 2"));
    }
}
