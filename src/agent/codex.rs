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
use std::time::Duration;
use std::time::Instant;
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt,
};
use tokio::process::{ChildStdin, ChildStdout, Command as ProcessCommand};
use tokio::sync::mpsc::Receiver;

use super::commands::{Command, CommandError, CommandRequest, CommandResult};

use serde_json::{json, Value as JsonValue};

use super::backend::{
    AbortContext, AbortOutcome, AgentSessionRun, RunOutcome, Runner, StartContext,
};
use super::process::AgentProcess;
use super::{AgentRunOptions, ReasoningEffort};

#[derive(Default)]
pub(super) struct CodexBackend {
    config: CodexRunConfig,
}

impl CodexBackend {
    pub(super) async fn abort(&mut self, context: AbortContext<'_>) -> io::Result<AbortOutcome> {
        use super::control::{client::ControlClient, SocketPath};
        let root = context
            .control_root
            .as_ref()
            .map_err(|error| io::Error::new(error.kind(), error.to_string()))?;
        let path = SocketPath::for_agent(root, context.agent_id)?;
        async { ControlClient::connect(path.as_path()).await?.stop().await }
            .await
            .map_err(|error| {
                io::Error::new(error.kind(), format!("agent {}: {error}", context.agent_id))
            })?;
        Ok(AbortOutcome::StopAccepted)
    }

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
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        let mut server = spawn_codex_app_server(&self.config, context.worktree_dir)?;
        let startup = tokio::select! {
            result = async {
                server.client.initialize().await?;
                server.client.thread_start(context.worktree_dir).await
            } => result,
            _ = sigint.recv() => Err(io::Error::new(io::ErrorKind::Interrupted, "Codex startup interrupted")),
            _ = sigterm.recv() => Err(io::Error::new(io::ErrorKind::Interrupted, "Codex startup interrupted")),
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
                sigterm,
                sigint,
                thread_id,
                prompt: context.prompt.to_string(),
            })),
            receiver,
        ))
    }
}

struct CodexAppServer {
    client: CodexClient<tokio::io::BufReader<ChildStdout>, ChildStdin, tokio::io::Stdout>,
    process: AgentProcess,
}
pub(super) struct CodexRunner {
    server: CodexAppServer,
    sigterm: tokio::signal::unix::Signal,
    sigint: tokio::signal::unix::Signal,
    thread_id: String,
    prompt: String,
}
impl CodexRunner {
    pub(super) async fn run(
        &mut self,
        mut command_receiver: Receiver<Command>,
    ) -> io::Result<RunOutcome> {
        let sigterm = &mut self.sigterm;
        let sigint = &mut self.sigint;
        let status = self
            .server
            .client
            .run_turn(
                &self.thread_id,
                &self.prompt,
                async {
                    tokio::select! { _ = sigterm.recv() => {}, _ = sigint.recv() => {} }
                },
                &mut command_receiver,
            )
            .await;
        let status = status?;
        if status == TurnStatus::Aborted {
            Ok(RunOutcome::Aborted)
        } else if status.is_success() {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TurnStatus {
    Completed,
    Interrupted,
    Aborted,
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

fn reject(code: &'static str, message: &str) -> CommandError {
    CommandError::Rejected {
        code,
        message: message.to_string(),
    }
}

struct PendingWrite {
    id: i64,
    bytes: Vec<u8>,
    offset: usize,
}

struct PendingSteer {
    id: i64,
    request: CommandRequest,
    write_started: bool,
}

impl PendingSteer {
    fn expire(&mut self, now: Instant) {
        if now >= self.request.deadline && self.request.has_reply_sender() {
            self.request.respond(Err(if self.write_started {
                CommandError::acceptance_unknown("acknowledgement deadline expired")
            } else {
                reject("timeout", "Steering request expired before Codex dispatch")
            }));
        }
    }
}

// The 30-second deadline bounds pre-turn intent, native acknowledgement, and
// (after acknowledgement) interrupted completion. No retries are performed.
const STOP_TIMEOUT: Duration = Duration::from_secs(30);

struct PendingStop {
    request: Option<CommandRequest>,
    deadline: Instant,
    write_started: bool,
    acknowledged: bool,
}

impl PendingStop {
    fn check_deadline(&mut self) -> io::Result<()> {
        if Instant::now() < self.deadline {
            return Ok(());
        }
        if self.acknowledged {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Codex accepted interruption but did not complete within 30 seconds; inspect waap agent get and runner logs; do not retry automatically",
            ));
        }
        let error = if self.write_started {
            CommandError::acceptance_unknown("interrupt acknowledgement deadline expired")
        } else {
            reject(
                "timeout",
                "Stop expired before native interruption could be sent",
            )
        };
        if let Some(request) = &mut self.request {
            request.respond(Err(error));
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "Codex interruption deadline expired",
        ))
    }
}

struct TurnState<'a> {
    thread_id: &'a str,
    start_request_id: i64,
    turn_id: Option<String>,
    early_completion: Option<(String, io::Result<TurnStatus>)>,
    pending_steer: Option<PendingSteer>,
    pending_writes: VecDeque<PendingWrite>,
    interrupt_rpc_id: Option<i64>,
    pending_stop: Option<PendingStop>,
    completion: Option<io::Result<TurnStatus>>,
}

impl<'a> TurnState<'a> {
    fn new(thread_id: &'a str, start_request_id: i64) -> Self {
        Self {
            thread_id,
            start_request_id,
            turn_id: None,
            early_completion: None,
            pending_steer: None,
            pending_writes: VecDeque::new(),
            interrupt_rpc_id: None,
            pending_stop: None,
            completion: None,
        }
    }

    fn compute_deadline(&self) -> Option<Instant> {
        self.pending_steer
            .as_ref()
            .filter(|pending| pending.request.has_reply_sender())
            .map(|pending| pending.request.deadline)
            .into_iter()
            .chain(self.pending_stop.as_ref().map(|stop| stop.deadline))
            .min()
    }

    fn resolve_status(&self, status: TurnStatus) -> TurnStatus {
        if status == TurnStatus::Interrupted
            && self
                .pending_stop
                .as_ref()
                .is_some_and(|stop| stop.acknowledged)
        {
            return TurnStatus::Aborted;
        }
        status
    }
}

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin, O: AsyncWrite + Unpin> CodexClient<R, W, O> {
    async fn run_turn(
        &mut self,
        thread_id: &str,
        prompt: &str,
        shutdown_signal: impl std::future::Future<Output = ()>,
        command_receiver: &mut Receiver<Command>,
    ) -> io::Result<TurnStatus> {
        let mut turn = TurnState::new(thread_id, self.next_id);
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
        tokio::pin!(shutdown_signal);
        let mut shutdown_signal_received = false;
        let mut channel_open = true;
        const READ_CHUNK_BYTES: usize = 8192;
        let mut chunk = [0; READ_CHUNK_BYTES];
        let mut buffer = Vec::new();
        loop {
            if let Some(stop) = &mut turn.pending_stop {
                stop.check_deadline()?;
            }
            if let Some(status) = self.process_frames(&mut turn, &mut buffer).await? {
                let status = status?;
                return Ok(turn.resolve_status(status));
            }
            self.prepare_writes(&mut turn);
            self.enqueue_interrupt(&mut turn);
            let deadline = turn.compute_deadline();
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
                request = command_receiver.recv(), if channel_open => {
                    match request {
                        Some(Command::Steer { prompt, request }) => self.dispatch_steering(&mut turn, prompt, request),
                        Some(Command::Stop(request)) => Self::dispatch_stop(&mut turn, Some(request)),
                        None => channel_open = false,
                    }
                }
                _ = &mut shutdown_signal, if !shutdown_signal_received => {
                    shutdown_signal_received = true;
                    Self::dispatch_stop(&mut turn, None);
                },
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
                if turn.interrupt_rpc_id.is_some()
                    && turn
                        .pending_stop
                        .as_ref()
                        .is_some_and(|stop| !stop.acknowledged)
                {
                    turn.completion = Some(status);
                    continue;
                }
                if turn.interrupt_rpc_id.is_none() {
                    if let Some(request) = turn
                        .pending_stop
                        .as_mut()
                        .and_then(|stop| stop.request.as_mut())
                    {
                        request.respond(Err(reject(
                            "no_active_turn",
                            "Codex turn completed before interruption",
                        )));
                    }
                }
                return Ok(Some(status));
            }
        }
        buffer.drain(..consumed);
        Ok(None)
    }

    fn enqueue_interrupt(&mut self, turn: &mut TurnState<'_>) {
        if turn.pending_stop.is_none() || turn.interrupt_rpc_id.is_some() {
            return;
        }
        let Some(turn_id) = &turn.turn_id else {
            return;
        };
        let params = json!({ "threadId": turn.thread_id, "turnId": turn_id });
        turn.interrupt_rpc_id = Some(self.enqueue_request(turn, METHOD_TURN_INTERRUPT, params));
    }

    fn dispatch_stop(turn: &mut TurnState<'_>, mut request: Option<CommandRequest>) {
        let now = Instant::now();
        if let Some(request) = &mut request {
            if now >= request.deadline {
                request.respond(Err(reject("timeout", "Stop expired before dispatch")));
                return;
            }
        }
        if let Some(pending) = &turn.pending_stop {
            if let Some(request) = &mut request {
                request.respond(if pending.acknowledged {
                    Ok(())
                } else {
                    Err(reject("busy", "Codex interruption is already pending"))
                });
            }
            return;
        }
        let deadline = request.as_ref().map_or(now + STOP_TIMEOUT, |request| {
            request.deadline.min(now + STOP_TIMEOUT)
        });
        turn.pending_stop = Some(PendingStop {
            request,
            deadline,
            write_started: false,
            acknowledged: false,
        });
    }

    fn dispatch_steering(
        &mut self,
        turn: &mut TurnState<'_>,
        prompt: String,
        mut request: CommandRequest,
    ) {
        if turn.pending_stop.is_some() {
            request.respond(Err(reject("stopping", "Codex interruption is pending")));
            return;
        }
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
            "input": [{ "type": "text", "text": prompt }],
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
        if Some(write.id) == turn.interrupt_rpc_id {
            turn.pending_stop.as_mut().unwrap().write_started = true;
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
        } else if Some(id) == turn.interrupt_rpc_id
            && turn
                .pending_stop
                .as_ref()
                .is_some_and(|stop| !stop.acknowledged)
        {
            let stop = turn.pending_stop.as_mut().unwrap();
            stop.check_deadline()?;
            let result = parse_interrupt_response(message);
            if let Some(request) = &mut stop.request {
                request.respond(result.clone());
            }
            result.map_err(io::Error::other)?;
            stop.acknowledged = true;
            stop.deadline = Instant::now() + STOP_TIMEOUT;
            if let Some(status) = turn.completion.take() {
                return Ok(Some(status));
            }
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

fn parse_interrupt_response(message: &JsonValue) -> CommandResult {
    if let Some(error) = message.get("error") {
        if let Some(code) = error.get("code").and_then(JsonValue::as_i64) {
            if error.get("message").and_then(JsonValue::as_str).is_some()
                && message.get("result").is_none()
            {
                return Err(reject(
                    "backend_rejected",
                    &format!("Codex rejected turn/interrupt (RPC error code {code}); verify that the turn is active"),
                ));
            }
        }
        return Err(CommandError::acceptance_unknown(
            "invalid turn/interrupt error",
        ));
    }
    if message
        .get("result")
        .and_then(JsonValue::as_object)
        .is_none_or(|result| !result.is_empty())
    {
        return Err(CommandError::acceptance_unknown(
            "invalid turn/interrupt acknowledgement",
        ));
    }
    Ok(())
}

fn parse_steering_response(message: &JsonValue, expected_turn: &str) -> CommandResult {
    if let Some(error) = message.get("error") {
        let (Some(code), Some(diagnostic)) = (
            error.get("code").and_then(JsonValue::as_i64),
            error.get("message").and_then(JsonValue::as_str),
        ) else {
            return Err(CommandError::acceptance_unknown(
                "the server returned an invalid turn/steer error",
            ));
        };
        if message.get("result").is_some() {
            return Err(CommandError::acceptance_unknown(
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
        return Err(CommandError::acceptance_unknown(
            "the server returned an invalid turn/steer acknowledgement",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "codex_control_tests.rs"]
mod control_tests;
#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
