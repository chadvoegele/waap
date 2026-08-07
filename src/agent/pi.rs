use std::env;
use std::ffi::OsString;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command as ProcessCommand, ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use clap::ValueEnum;
use serde_json::{json, Map, Value as JsonValue};

use super::backend::{
    signal_agent_run, AbortContext, AgentSystemBackend, RunHandle, RunOutcome, StartContext,
    StartedRun,
};
use super::{AgentRunOptions, ReasoningEffort};

const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const SCRUBBED_PI_ENV: [&str; 5] = [
    "PI_SESSION_ID",
    "PI_SESSION_FILE",
    "PI_PROVIDER",
    "PI_MODEL",
    "PI_REASONING_LEVEL",
];

#[derive(Default)]
pub(super) struct PiBackend {
    config: PiRunConfig,
}

impl PiBackend {
    pub(super) fn from_env(options: &AgentRunOptions) -> io::Result<Self> {
        Ok(Self {
            config: PiRunConfig::from_env(options)?,
        })
    }
}

impl AgentSystemBackend for PiBackend {
    fn start(&mut self, context: StartContext<'_>) -> io::Result<StartedRun> {
        let interrupt = Arc::new(AtomicBool::new(false));
        signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&interrupt)).map_err(
            |error| io::Error::other(format!("failed to install SIGTERM handler: {error}")),
        )?;
        start_pi_run(&self.config, context, interrupt, Box::new(io::stdout()))
    }

    fn abort(&mut self, context: AbortContext<'_>) -> io::Result<()> {
        signal_agent_run(context.agent_id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PiRunConfig {
    executable: OsString,
    model: Option<String>,
    reasoning_effort: Option<ReasoningEffort>,
    command_timeout: Duration,
}

impl Default for PiRunConfig {
    fn default() -> Self {
        Self {
            executable: OsString::from("pi"),
            model: None,
            reasoning_effort: None,
            command_timeout: DEFAULT_COMMAND_TIMEOUT,
        }
    }
}

impl PiRunConfig {
    fn from_env(options: &AgentRunOptions) -> io::Result<Self> {
        let executable = env::var_os("WAAP_PI_BIN").unwrap_or_else(|| OsString::from("pi"));
        if executable.is_empty() {
            return Err(invalid_input("WAAP_PI_BIN must not be empty"));
        }

        let model = match &options.model {
            Some(model) => Some(model.clone()),
            None => optional_non_empty_env("WAAP_PI_MODEL")?,
        };
        let reasoning_effort = match options.reasoning_effort {
            Some(effort) => Some(validate_pi_effort(effort, "--reasoning-effort")?),
            None => optional_pi_effort_env()?,
        };

        Ok(Self {
            executable,
            model,
            reasoning_effort,
            command_timeout: DEFAULT_COMMAND_TIMEOUT,
        })
    }
}

fn optional_non_empty_env(name: &str) -> io::Result<Option<String>> {
    match env::var(name) {
        Ok(value) if value.trim().is_empty() => {
            Err(invalid_input(format!("{name} must not be empty")))
        }
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => {
            Err(invalid_input(format!("{name} must be valid UTF-8")))
        }
    }
}

fn optional_pi_effort_env() -> io::Result<Option<ReasoningEffort>> {
    let Some(value) = optional_non_empty_env("WAAP_PI_REASONING_EFFORT")? else {
        return Ok(None);
    };
    let effort = ReasoningEffort::parse(&value).ok_or_else(|| {
        invalid_input(format!(
            "invalid WAAP_PI_REASONING_EFFORT {value:?}; accepted values: {}",
            pi_effort_labels().join(", ")
        ))
    })?;
    validate_pi_effort(effort, "WAAP_PI_REASONING_EFFORT").map(Some)
}

fn validate_pi_effort(effort: ReasoningEffort, source: &str) -> io::Result<ReasoningEffort> {
    if effort == ReasoningEffort::Ultra {
        return Err(invalid_input(format!(
            "{source} does not support ultra for Pi; accepted values: {}",
            pi_effort_labels().join(", ")
        )));
    }
    Ok(effort)
}

fn pi_effort_labels() -> Vec<&'static str> {
    ReasoningEffort::value_variants()
        .iter()
        .copied()
        .filter(|effort| *effort != ReasoningEffort::Ultra)
        .map(ReasoningEffort::as_str)
        .collect()
}

fn pi_thinking(effort: ReasoningEffort) -> &'static str {
    match effort {
        ReasoningEffort::None => "off",
        _ => effort.as_str(),
    }
}

fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn pi_command(config: &PiRunConfig, agent_id: &str, worktree_dir: &Path) -> ProcessCommand {
    let mut command = ProcessCommand::new(&config.executable);
    command
        .arg("--mode")
        .arg("rpc")
        .arg("--approve")
        .arg("--name")
        .arg(format!("waap {agent_id}"))
        .current_dir(worktree_dir);
    if let Some(model) = &config.model {
        command.arg("--model").arg(model);
    }
    if let Some(effort) = config.reasoning_effort {
        command.arg("--thinking").arg(pi_thinking(effort));
    }
    for name in SCRUBBED_PI_ENV {
        command.env_remove(name);
    }
    command
}

fn start_pi_run(
    config: &PiRunConfig,
    context: StartContext<'_>,
    interrupt: Arc<AtomicBool>,
    output: Box<dyn Write + Send>,
) -> io::Result<StartedRun> {
    let mut client = PiRpcClient::spawn(config, context.agent_id, context.worktree_dir, output)?;
    let mut startup_state = PiState::default();
    let response = client.command("get_state", &[], &mut startup_state, None)?;
    let session_id = response
        .pointer("/data/sessionId")
        .and_then(JsonValue::as_str)
        .filter(|session_id| !session_id.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| invalid_protocol("Pi get_state response is missing data.sessionId"))?;

    Ok(StartedRun {
        session_id,
        handle: Box::new(PiRun {
            client,
            interrupt,
            prompt: context.prompt.to_string(),
        }),
    })
}

pub(super) struct PiRun {
    client: PiRpcClient,
    interrupt: Arc<AtomicBool>,
    prompt: String,
}

impl RunHandle for PiRun {
    fn wait(mut self: Box<Self>) -> io::Result<RunOutcome> {
        let mut state = PiState::default();
        self.client.command(
            "prompt",
            &[("message", JsonValue::String(self.prompt.clone()))],
            &mut state,
            Some(&self.interrupt),
        )?;

        while !state.settled || state.abort_request_id.is_some() {
            self.client.maybe_abort(&self.interrupt, &mut state)?;
            match self.client.receive(POLL_INTERVAL)? {
                Some(record) => self.client.process_record(record, &mut state)?,
                None if state.owner_interrupted => {
                    self.client.finish(false)?;
                    return Ok(RunOutcome::Aborted);
                }
                None => return Err(self.client.eof_error("the agent settled")),
            }
        }

        let outcome = state.outcome()?;
        self.client.finish(true)?;
        Ok(outcome)
    }
}

#[derive(Default)]
struct PiState {
    latest_stop_reason: Option<String>,
    settled: bool,
    owner_interrupted: bool,
    abort_request_id: Option<String>,
    abort_deadline: Option<Instant>,
}

impl PiState {
    fn outcome(&self) -> io::Result<RunOutcome> {
        if self.owner_interrupted {
            return Ok(RunOutcome::Aborted);
        }
        match self.latest_stop_reason.as_deref() {
            Some("stop") => Ok(RunOutcome::Completed),
            Some("error" | "length" | "toolUse") => Ok(RunOutcome::Failed(ExitCode::FAILURE)),
            Some("aborted") => Ok(RunOutcome::Aborted),
            Some(reason) => Err(invalid_protocol(format!(
                "Pi assistant message has unknown stopReason {reason:?}"
            ))),
            None => Err(invalid_protocol(
                "Pi agent settled without an assistant result",
            )),
        }
    }
}

enum ReaderMessage {
    Record(JsonValue),
    Eof,
    Error(io::ErrorKind, String),
}

pub(super) struct PiRpcClient {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    receiver: Receiver<ReaderMessage>,
    reader_thread: Option<JoinHandle<()>>,
    output: Box<dyn Write + Send>,
    next_id: u64,
    command_timeout: Duration,
}

impl PiRpcClient {
    fn spawn(
        config: &PiRunConfig,
        agent_id: &str,
        worktree_dir: &Path,
        output: Box<dyn Write + Send>,
    ) -> io::Result<Self> {
        let mut command = pi_command(config, agent_id, worktree_dir);
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("Pi RPC stdin is unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("Pi RPC stdout is unavailable"))?;
        let (sender, receiver) = mpsc::channel();
        let reader_thread = thread::spawn(move || read_records(stdout, sender));

        Ok(Self {
            child: Some(child),
            stdin: Some(stdin),
            receiver,
            reader_thread: Some(reader_thread),
            output,
            next_id: 1,
            command_timeout: config.command_timeout,
        })
    }

    fn command(
        &mut self,
        command: &str,
        fields: &[(&str, JsonValue)],
        state: &mut PiState,
        interrupt: Option<&AtomicBool>,
    ) -> io::Result<JsonValue> {
        let id = self.send_command(command, fields)?;
        let deadline = Instant::now() + self.command_timeout;
        loop {
            if let Some(interrupt) = interrupt {
                self.maybe_abort(interrupt, state)?;
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("timed out waiting for Pi {command} response"),
                ));
            }
            let timeout = POLL_INTERVAL.min(deadline.saturating_duration_since(now));
            let Some(record) = self.receive(timeout)? else {
                return Err(self.eof_error(&format!("the {command} response")));
            };

            if record.get("type").and_then(JsonValue::as_str) == Some("response")
                && record.get("id").and_then(JsonValue::as_str) == Some(id.as_str())
            {
                return validate_response(record, command);
            }
            self.process_record(record, state)?;
        }
    }

    fn send_command(&mut self, command: &str, fields: &[(&str, JsonValue)]) -> io::Result<String> {
        let id = format!("waap-{}", self.next_id);
        self.next_id += 1;
        let mut value = Map::new();
        value.insert("id".to_string(), JsonValue::String(id.clone()));
        value.insert("type".to_string(), JsonValue::String(command.to_string()));
        for (name, field) in fields {
            value.insert((*name).to_string(), field.clone());
        }
        self.write_value(&JsonValue::Object(value))?;
        Ok(id)
    }

    fn write_value(&mut self, value: &JsonValue) -> io::Result<()> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "Pi RPC stdin is closed"))?;
        serde_json::to_writer(&mut *stdin, value).map_err(io::Error::other)?;
        stdin.write_all(b"\n")?;
        stdin.flush()
    }

    fn maybe_abort(&mut self, interrupt: &AtomicBool, state: &mut PiState) -> io::Result<()> {
        if state
            .abort_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "timed out waiting for Pi abort response",
            ));
        }
        if interrupt.load(Ordering::SeqCst) && !state.owner_interrupted {
            state.owner_interrupted = true;
            state.abort_request_id = Some(self.send_command("abort", &[])?);
            state.abort_deadline = Some(Instant::now() + self.command_timeout);
        }
        Ok(())
    }

    fn process_record(&mut self, record: JsonValue, state: &mut PiState) -> io::Result<()> {
        let record_type = record.get("type").and_then(JsonValue::as_str);
        if record_type == Some("response") {
            if record.get("id").and_then(JsonValue::as_str) == state.abort_request_id.as_deref() {
                validate_response(record, "abort")?;
                state.abort_request_id = None;
                state.abort_deadline = None;
            }
            return Ok(());
        }

        match record_type {
            Some("extension_ui_request") => self.handle_ui_request(&record),
            Some("message_update") => {
                if record
                    .pointer("/assistantMessageEvent/type")
                    .and_then(JsonValue::as_str)
                    == Some("text_delta")
                {
                    let delta = record
                        .pointer("/assistantMessageEvent/delta")
                        .and_then(JsonValue::as_str)
                        .ok_or_else(|| invalid_protocol("Pi text_delta event is missing delta"))?;
                    self.output.write_all(delta.as_bytes())?;
                    self.output.flush()?;
                }
                Ok(())
            }
            Some("message_end") => update_assistant_result(record.get("message"), state),
            Some("turn_end") => update_assistant_result(record.get("message"), state),
            Some("agent_settled") => {
                state.settled = true;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn handle_ui_request(&mut self, record: &JsonValue) -> io::Result<()> {
        let method = record
            .get("method")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| invalid_protocol("Pi extension UI request is missing method"))?;
        match method {
            "select" | "confirm" | "input" | "editor" => {
                let id = record
                    .get("id")
                    .and_then(JsonValue::as_str)
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| invalid_protocol("Pi extension dialog is missing id"))?;
                self.write_value(&json!({
                    "type": "extension_ui_response",
                    "id": id,
                    "cancelled": true,
                }))
            }
            "notify" | "setStatus" | "setWidget" | "setTitle" | "set_editor_text" => Ok(()),
            _ => Err(invalid_protocol(format!(
                "Pi extension UI request has unknown method {method:?}"
            ))),
        }
    }

    fn receive(&mut self, timeout: Duration) -> io::Result<Option<JsonValue>> {
        match self.receiver.recv_timeout(timeout) {
            Ok(ReaderMessage::Record(record)) => Ok(Some(record)),
            Ok(ReaderMessage::Eof) => Ok(None),
            Ok(ReaderMessage::Error(kind, message)) => {
                let status = self
                    .child
                    .as_mut()
                    .and_then(|child| child.try_wait().ok().flatten())
                    .map(|status| status.to_string())
                    .unwrap_or_else(|| "unavailable".to_string());
                Err(io::Error::new(
                    kind,
                    format!("{message}; process status: {status}"),
                ))
            }
            Err(RecvTimeoutError::Timeout) => {
                if self
                    .child
                    .as_mut()
                    .is_some_and(|child| child.try_wait().is_ok_and(|status| status.is_some()))
                {
                    Ok(None)
                } else {
                    Ok(Some(JsonValue::Null))
                }
            }
            Err(RecvTimeoutError::Disconnected) => Ok(None),
        }
    }

    fn eof_error(&mut self, expected: &str) -> io::Error {
        let status = self
            .child
            .as_mut()
            .and_then(|child| child.try_wait().ok().flatten())
            .map(|status| status.to_string())
            .unwrap_or_else(|| "unavailable".to_string());
        io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!("Pi RPC closed before {expected}; process status: {status}"),
        )
    }

    fn finish(&mut self, require_success: bool) -> io::Result<()> {
        self.stdin.take();
        let Some(mut child) = self.child.take() else {
            return Ok(());
        };
        let deadline = Instant::now() + self.command_timeout;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() >= deadline {
                child.kill()?;
                let status = child.wait()?;
                self.join_reader();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("Pi RPC did not exit after stdin closed; killed with {status}"),
                ));
            }
            thread::sleep(Duration::from_millis(10));
        };
        self.join_reader();
        if require_success && !status.success() {
            return Err(io::Error::other(format!(
                "Pi RPC exited with status {status}"
            )));
        }
        Ok(())
    }

    fn join_reader(&mut self) {
        if let Some(reader_thread) = self.reader_thread.take() {
            let _ = reader_thread.join();
        }
    }
}

impl Drop for PiRpcClient {
    fn drop(&mut self) {
        if let Err(error) = self.finish(false) {
            log::error!("failed to clean up Pi RPC child: {error}");
        }
    }
}

fn validate_response(response: JsonValue, expected_command: &str) -> io::Result<JsonValue> {
    let command = response.get("command").and_then(JsonValue::as_str);
    if command != Some(expected_command) {
        return Err(invalid_protocol(format!(
            "Pi response command mismatch: expected {expected_command:?}, got {command:?}"
        )));
    }
    match response.get("success").and_then(JsonValue::as_bool) {
        Some(true) => Ok(response),
        Some(false) => Err(io::Error::other(format!(
            "Pi {expected_command} command failed: {}",
            response
                .get("error")
                .and_then(JsonValue::as_str)
                .unwrap_or("unspecified error")
        ))),
        None => Err(invalid_protocol(format!(
            "Pi {expected_command} response is missing success"
        ))),
    }
}

fn update_assistant_result(message: Option<&JsonValue>, state: &mut PiState) -> io::Result<()> {
    let Some(message) = message else {
        return Err(invalid_protocol("Pi message event is missing message"));
    };
    if message.get("role").and_then(JsonValue::as_str) != Some("assistant") {
        return Ok(());
    }
    let stop_reason = message
        .get("stopReason")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| invalid_protocol("Pi assistant message is missing stopReason"))?;
    state.latest_stop_reason = Some(stop_reason.to_string());
    Ok(())
}

fn invalid_protocol(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn read_records(stdout: ChildStdout, sender: mpsc::Sender<ReaderMessage>) {
    let mut reader = BufReader::new(stdout);
    loop {
        match read_jsonl_record(&mut reader) {
            Ok(Some(record)) => {
                if sender.send(ReaderMessage::Record(record)).is_err() {
                    break;
                }
            }
            Ok(None) => {
                let _ = sender.send(ReaderMessage::Eof);
                break;
            }
            Err(error) => {
                let _ = sender.send(ReaderMessage::Error(error.kind(), error.to_string()));
                break;
            }
        }
    }
}

fn read_jsonl_record(reader: &mut impl BufRead) -> io::Result<Option<JsonValue>> {
    let mut bytes = Vec::new();
    if reader.read_until(b'\n', &mut bytes)? == 0 {
        return Ok(None);
    }
    if bytes.last() != Some(&b'\n') {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Pi RPC sent an unterminated JSONL record",
        ));
    }
    bytes.pop();
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    serde_json::from_slice(&bytes).map(Some).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Pi RPC sent malformed JSON: {error}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{Cursor, Read};
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use tempfile::tempdir;

    use super::*;
    use crate::agent::PI_ENV_LOCK;

    struct OneByteReader<R>(R);

    impl<R: Read> Read for OneByteReader<R> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let length = buffer.len().min(1);
            self.0.read(&mut buffer[..length])
        }
    }

    #[derive(Clone)]
    struct SharedOutput(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedOutput {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn fake_script(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("fake-pi");
        fs::write(&path, format!("#!/bin/sh\nset -eu\n{body}\n")).unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    fn fake_config(executable: PathBuf) -> PiRunConfig {
        PiRunConfig {
            executable: executable.into_os_string(),
            command_timeout: Duration::from_secs(2),
            ..PiRunConfig::default()
        }
    }

    fn context<'a>(dir: &'a Path, prompt: &'a str) -> StartContext<'a> {
        StartContext {
            agent_id: "aa-12345678",
            prompt,
            repository_root: dir,
            worktree_dir: dir,
        }
    }

    #[test]
    fn framing_splits_only_lf_strips_cr_and_preserves_unicode_separators() {
        let input = "{\"value\":\"a\u{2028}b\u{2029}c\"}\r\n{\"value\":2}\n";
        let mut reader = BufReader::new(OneByteReader(Cursor::new(input.as_bytes())));

        assert_eq!(
            read_jsonl_record(&mut reader).unwrap().unwrap()["value"],
            json!("a\u{2028}b\u{2029}c")
        );
        assert_eq!(
            read_jsonl_record(&mut reader).unwrap().unwrap()["value"],
            json!(2)
        );
        assert!(read_jsonl_record(&mut reader).unwrap().is_none());
    }

    #[test]
    fn framing_rejects_blank_malformed_and_unterminated_json() {
        for input in ["\n", "not-json\n"] {
            let mut reader = Cursor::new(input.as_bytes());
            assert_eq!(
                read_jsonl_record(&mut reader).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
        for input in ["{\"x\":", "{\"x\":1}"] {
            let mut reader = Cursor::new(input.as_bytes());
            assert_eq!(
                read_jsonl_record(&mut reader).unwrap_err().kind(),
                io::ErrorKind::UnexpectedEof
            );
        }
    }

    #[test]
    fn config_uses_cli_precedence_maps_none_and_rejects_ultra() {
        let _lock = PI_ENV_LOCK.lock().unwrap();
        let previous = [
            env::var_os("WAAP_PI_BIN"),
            env::var_os("WAAP_PI_MODEL"),
            env::var_os("WAAP_PI_REASONING_EFFORT"),
        ];
        env::set_var("WAAP_PI_BIN", "/tmp/pi-bin");
        env::set_var("WAAP_PI_MODEL", "env-model");
        env::set_var("WAAP_PI_REASONING_EFFORT", "low");

        let fallback = PiRunConfig::from_env(&AgentRunOptions::default()).unwrap();
        let overridden = PiRunConfig::from_env(&AgentRunOptions {
            model: Some("cli-model".to_string()),
            reasoning_effort: Some(ReasoningEffort::None),
        })
        .unwrap();
        assert_eq!(fallback.model.as_deref(), Some("env-model"));
        assert_eq!(fallback.executable, OsString::from("/tmp/pi-bin"));
        assert_eq!(fallback.reasoning_effort, Some(ReasoningEffort::Low));
        assert_eq!(overridden.model.as_deref(), Some("cli-model"));
        assert_eq!(pi_thinking(overridden.reasoning_effort.unwrap()), "off");
        for effort in ReasoningEffort::value_variants()
            .iter()
            .copied()
            .filter(|effort| !matches!(effort, ReasoningEffort::None | ReasoningEffort::Ultra))
        {
            assert_eq!(pi_thinking(effort), effort.as_str());
        }
        assert!(PiRunConfig::from_env(&AgentRunOptions {
            model: None,
            reasoning_effort: Some(ReasoningEffort::Ultra),
        })
        .unwrap_err()
        .to_string()
        .contains("does not support ultra"));

        for (name, value) in [
            ("WAAP_PI_BIN", previous[0].clone()),
            ("WAAP_PI_MODEL", previous[1].clone()),
            ("WAAP_PI_REASONING_EFFORT", previous[2].clone()),
        ] {
            match value {
                Some(value) => env::set_var(name, value),
                None => env::remove_var(name),
            }
        }
    }

    #[test]
    fn config_rejects_invalid_environment_values_unless_cli_overrides_them() {
        let _lock = PI_ENV_LOCK.lock().unwrap();
        let previous_model = env::var_os("WAAP_PI_MODEL");
        let previous_effort = env::var_os("WAAP_PI_REASONING_EFFORT");
        env::set_var("WAAP_PI_MODEL", "  ");
        assert!(PiRunConfig::from_env(&AgentRunOptions::default()).is_err());
        env::set_var("WAAP_PI_MODEL", "valid");
        env::set_var("WAAP_PI_REASONING_EFFORT", "off");
        assert!(PiRunConfig::from_env(&AgentRunOptions::default()).is_err());
        assert!(PiRunConfig::from_env(&AgentRunOptions {
            model: Some("cli".to_string()),
            reasoning_effort: Some(ReasoningEffort::High),
        })
        .is_ok());
        match previous_model {
            Some(value) => env::set_var("WAAP_PI_MODEL", value),
            None => env::remove_var("WAAP_PI_MODEL"),
        }
        match previous_effort {
            Some(value) => env::set_var("WAAP_PI_REASONING_EFFORT", value),
            None => env::remove_var("WAAP_PI_REASONING_EFFORT"),
        }
    }

    #[test]
    fn command_has_rpc_approval_session_name_options_cwd_and_scrubbed_metadata() {
        let config = PiRunConfig {
            model: Some("openai/gpt".to_string()),
            reasoning_effort: Some(ReasoningEffort::Max),
            ..PiRunConfig::default()
        };
        let dir = Path::new("/repo/with space");
        let command = pi_command(&config, "aa-12345678", dir);

        assert_eq!(command.get_program(), "pi");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                "--mode",
                "rpc",
                "--approve",
                "--name",
                "waap aa-12345678",
                "--model",
                "openai/gpt",
                "--thinking",
                "max",
            ]
        );
        assert_eq!(command.get_current_dir(), Some(dir));
        let removed = command
            .get_envs()
            .filter_map(|(name, value)| value.is_none().then_some(name.to_string_lossy()))
            .collect::<Vec<_>>();
        for name in SCRUBBED_PI_ENV {
            assert!(removed.iter().any(|removed| removed == name));
        }
    }

    #[test]
    fn session_precedes_prompt_interleaved_ui_is_cancelled_text_streams_and_child_is_reaped() {
        let _lock = PI_ENV_LOCK.lock().unwrap();
        let dir = tempdir().unwrap();
        let log = dir.path().join("commands.log");
        let pid_file = dir.path().join("pid");
        let env_file = dir.path().join("env");
        let inherited_names = [
            "PI_MODEL",
            "PI_SESSION_ID",
            "PI_CODING_AGENT_DIR",
            "HTTPS_PROXY",
        ];
        let inherited_previous = inherited_names.map(env::var_os);
        env::set_var("PI_MODEL", "parent-model");
        env::set_var("PI_SESSION_ID", "parent-session");
        env::set_var("PI_CODING_AGENT_DIR", "/tmp/pi-config");
        env::set_var("HTTPS_PROXY", "http://proxy.test");
        let script = fake_script(
            dir.path(),
            r#"
printf '%s' "$$" > "$WAAP_TEST_PID_FILE"
printf '%s|%s|%s|%s' "${PI_MODEL-unset}" "${PI_SESSION_ID-unset}" "${PI_CODING_AGENT_DIR-unset}" "${HTTPS_PROXY-unset}" > "$WAAP_TEST_ENV_FILE"
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$WAAP_TEST_LOG"
  id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"type":"get_state"'*)
      printf '%s\n' '{"type":"message_update","assistantMessageEvent":{"type":"thinking_delta","delta":"secret"}}'
      for pair in 'dialog-select select' 'dialog-confirm confirm' 'dialog-input input' 'dialog-editor editor'; do
        set -- $pair
        printf '{"type":"extension_ui_request","id":"%s","method":"%s"}\n' "$1" "$2"
        IFS= read -r reply
        printf '%s\n' "$reply" >> "$WAAP_TEST_LOG"
      done
      for method in notify setStatus setWidget setTitle set_editor_text; do
        printf '{"type":"extension_ui_request","id":"notice","method":"%s"}\n' "$method"
      done
      printf '{"type":"response","id":"%s","command":"get_state","success":true,"data":{"sessionId":"pi-session"}}\n' "$id"
      ;;
    *'"type":"prompt"'*)
      printf '{"type":"response","id":"%s","command":"prompt","success":true}\n' "$id"
      printf '%s\n' '{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"hello "}}'
      printf '%s\n' '{"type":"agent_end","messages":[],"willRetry":true}'
      printf '%s\n' '{"type":"auto_retry_end","success":true}'
      printf '%s\n' '{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"world"}}'
      printf '%s\n' '{"type":"message_end","message":{"role":"assistant","stopReason":"stop"}}'
      printf '%s\n' '{"type":"agent_settled"}'
      ;;
  esac
done
"#,
        );
        env::set_var("WAAP_TEST_LOG", &log);
        env::set_var("WAAP_TEST_PID_FILE", &pid_file);
        env::set_var("WAAP_TEST_ENV_FILE", &env_file);
        let output = Arc::new(Mutex::new(Vec::new()));
        let started = start_pi_run(
            &fake_config(script),
            context(dir.path(), "do it"),
            Arc::new(AtomicBool::new(false)),
            Box::new(SharedOutput(Arc::clone(&output))),
        )
        .unwrap();

        assert_eq!(started.session_id, "pi-session");
        let before_wait = fs::read_to_string(&log).unwrap();
        assert!(before_wait.contains("\"type\":\"get_state\""));
        assert!(!before_wait.contains("\"type\":\"prompt\""));
        let cancellations = before_wait
            .lines()
            .filter_map(|line| serde_json::from_str::<JsonValue>(line).ok())
            .filter(|value| {
                value.get("type").and_then(JsonValue::as_str) == Some("extension_ui_response")
                    && value.get("cancelled").and_then(JsonValue::as_bool) == Some(true)
            })
            .count();
        assert_eq!(cancellations, 4);
        assert_eq!(
            fs::read_to_string(&env_file).unwrap(),
            "unset|unset|/tmp/pi-config|http://proxy.test"
        );

        assert_eq!(started.handle.wait().unwrap(), RunOutcome::Completed);
        assert_eq!(&*output.lock().unwrap(), b"hello world");
        assert!(fs::read_to_string(&log)
            .unwrap()
            .contains("\"message\":\"do it\""));
        let pid = fs::read_to_string(pid_file).unwrap();
        assert!(!Path::new("/proc").join(pid).exists());
        env::remove_var("WAAP_TEST_LOG");
        env::remove_var("WAAP_TEST_PID_FILE");
        env::remove_var("WAAP_TEST_ENV_FILE");
        for (name, value) in inherited_names.into_iter().zip(inherited_previous) {
            match value {
                Some(value) => env::set_var(name, value),
                None => env::remove_var(name),
            }
        }
    }

    #[test]
    fn owner_interrupt_sends_exactly_one_abort_and_returns_aborted() {
        let _lock = PI_ENV_LOCK.lock().unwrap();
        let dir = tempdir().unwrap();
        let log = dir.path().join("commands.log");
        let script = fake_script(
            dir.path(),
            r#"
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$WAAP_TEST_LOG"
  id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"type":"get_state"'*)
      printf '{"type":"response","id":"%s","command":"get_state","success":true,"data":{"sessionId":"pi-abort"}}\n' "$id"
      ;;
    *'"type":"prompt"'*)
      printf '{"type":"response","id":"%s","command":"prompt","success":true}\n' "$id"
      ;;
    *'"type":"abort"'*)
      printf '{"type":"response","id":"%s","command":"abort","success":true}\n' "$id"
      printf '%s\n' '{"type":"message_end","message":{"role":"assistant","stopReason":"aborted"}}'
      printf '%s\n' '{"type":"agent_settled"}'
      ;;
  esac
done
"#,
        );
        env::set_var("WAAP_TEST_LOG", &log);
        let interrupt = Arc::new(AtomicBool::new(false));
        let started = start_pi_run(
            &fake_config(script),
            context(dir.path(), "stop"),
            Arc::clone(&interrupt),
            Box::new(io::sink()),
        )
        .unwrap();
        interrupt.store(true, Ordering::SeqCst);

        assert_eq!(started.handle.wait().unwrap(), RunOutcome::Aborted);
        let commands = fs::read_to_string(log).unwrap();
        assert_eq!(commands.matches("\"type\":\"abort\"").count(), 1);
        env::remove_var("WAAP_TEST_LOG");
    }

    #[test]
    fn prompt_rejection_malformed_json_timeout_and_eof_fail_closed() {
        for (body, expected_kind) in [
            (
                r#"read line
id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
printf '{"type":"response","id":"%s","command":"get_state","success":true,"data":{"sessionId":"s"}}\n' "$id"
read line
id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
printf '{"type":"response","id":"%s","command":"prompt","success":false,"error":"rejected"}\n' "$id""#,
                io::ErrorKind::Other,
            ),
            ("printf '%s\\n' 'not-json'", io::ErrorKind::InvalidData),
            ("exit 0", io::ErrorKind::UnexpectedEof),
        ] {
            let dir = tempdir().unwrap();
            let script = fake_script(dir.path(), body);
            let result = start_pi_run(
                &fake_config(script),
                context(dir.path(), "prompt"),
                Arc::new(AtomicBool::new(false)),
                Box::new(io::sink()),
            );
            match result {
                Ok(started) => assert_eq!(started.handle.wait().unwrap_err().kind(), expected_kind),
                Err(error) => assert_eq!(error.kind(), expected_kind),
            }
        }

        let dir = tempdir().unwrap();
        let script = fake_script(dir.path(), "sleep 1");
        let mut config = fake_config(script);
        config.command_timeout = Duration::from_millis(50);
        let error = match start_pi_run(
            &config,
            context(dir.path(), "prompt"),
            Arc::new(AtomicBool::new(false)),
            Box::new(io::sink()),
        ) {
            Ok(_) => panic!("silent Pi process should time out"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn missing_session_and_prompt_response_timeout_fail_closed() {
        let dir = tempdir().unwrap();
        let missing_session = fake_script(
            dir.path(),
            r#"read line
id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
printf '{"type":"response","id":"%s","command":"get_state","success":true,"data":{}}\n' "$id""#,
        );
        let error = match start_pi_run(
            &fake_config(missing_session),
            context(dir.path(), "prompt"),
            Arc::new(AtomicBool::new(false)),
            Box::new(io::sink()),
        ) {
            Ok(_) => panic!("missing session id must fail"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);

        let dir = tempdir().unwrap();
        let silent_prompt = fake_script(
            dir.path(),
            r#"read line
id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
printf '{"type":"response","id":"%s","command":"get_state","success":true,"data":{"sessionId":"s"}}\n' "$id"
read line
sleep 1"#,
        );
        let mut config = fake_config(silent_prompt);
        config.command_timeout = Duration::from_millis(50);
        let started = start_pi_run(
            &config,
            context(dir.path(), "prompt"),
            Arc::new(AtomicBool::new(false)),
            Box::new(io::sink()),
        )
        .unwrap();
        assert_eq!(
            started.handle.wait().unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
    }

    #[test]
    fn settled_outcome_maps_every_stop_reason_and_rejects_missing_or_unknown() {
        for (reason, expected) in [
            ("stop", RunOutcome::Completed),
            ("error", RunOutcome::Failed(ExitCode::FAILURE)),
            ("length", RunOutcome::Failed(ExitCode::FAILURE)),
            ("toolUse", RunOutcome::Failed(ExitCode::FAILURE)),
            ("aborted", RunOutcome::Aborted),
        ] {
            assert_eq!(
                PiState {
                    latest_stop_reason: Some(reason.to_string()),
                    settled: true,
                    ..PiState::default()
                }
                .outcome()
                .unwrap(),
                expected
            );
        }
        assert!(PiState::default().outcome().is_err());
        assert!(PiState {
            latest_stop_reason: Some("future".to_string()),
            ..PiState::default()
        }
        .outcome()
        .is_err());
        assert_eq!(
            PiState {
                latest_stop_reason: Some("stop".to_string()),
                owner_interrupted: true,
                ..PiState::default()
            }
            .outcome()
            .unwrap(),
            RunOutcome::Aborted
        );
    }

    #[test]
    fn dropping_started_run_closes_and_reaps_child_before_prompt() {
        let _lock = PI_ENV_LOCK.lock().unwrap();
        let dir = tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let script = fake_script(
            dir.path(),
            r#"
printf '%s' "$$" > "$WAAP_TEST_PID_FILE"
IFS= read -r line
id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
printf '{"type":"response","id":"%s","command":"get_state","success":true,"data":{"sessionId":"drop-me"}}\n' "$id"
while IFS= read -r line; do :; done
"#,
        );
        env::set_var("WAAP_TEST_PID_FILE", &pid_file);
        let started = start_pi_run(
            &fake_config(script),
            context(dir.path(), "never sent"),
            Arc::new(AtomicBool::new(false)),
            Box::new(io::sink()),
        )
        .unwrap();
        let pid = fs::read_to_string(&pid_file).unwrap();

        drop(started);

        assert!(!Path::new("/proc").join(pid).exists());
        env::remove_var("WAAP_TEST_PID_FILE");
    }
}
