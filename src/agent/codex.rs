use std::env;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::process::{ChildStdin, ChildStdout, Command as ProcessCommand, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{json, Value as JsonValue};

const METHOD_INITIALIZE: &str = "initialize";
const METHOD_INITIALIZED: &str = "initialized";
const METHOD_THREAD_START: &str = "thread/start";
const METHOD_TURN_START: &str = "turn/start";
const METHOD_TURN_INTERRUPT: &str = "turn/interrupt";
const METHOD_AGENT_MESSAGE_DELTA: &str = "item/agentMessage/delta";
const METHOD_TURN_COMPLETED: &str = "turn/completed";

const APPROVAL_POLICY_NEVER: &str = "never";
const SANDBOX_DANGER_FULL_ACCESS: &str = "danger-full-access";

#[derive(Debug, PartialEq, Eq)]
pub(super) struct CodexRunConfig {
    model: Option<String>,
}

pub(super) fn codex_run_config_from_env() -> CodexRunConfig {
    CodexRunConfig {
        model: env::var("CODEX_MODEL")
            .ok()
            .filter(|model| !model.is_empty()),
    }
}

pub(super) fn signal_codex_run(agent_id: &str) -> io::Result<()> {
    let mut command = ProcessCommand::new("pkill");
    command
        .arg("-TERM")
        .arg("-f")
        .arg(format!("agent run --agent-id {agent_id}"));
    map_pkill_status(command)
}

fn map_pkill_status(mut command: ProcessCommand) -> io::Result<()> {
    let status = command.status()?;
    match status.code() {
        Some(0) | Some(1) => Ok(()),
        Some(code) => Err(io::Error::other(format!("pkill exited with status {code}"))),
        None => Err(io::Error::other("pkill terminated by signal")),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TurnStatus {
    Completed,
    Interrupted,
    Failed,
    InProgress,
}

impl TurnStatus {
    pub(super) fn is_success(self) -> bool {
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

fn turn_start_params(thread_id: &str, prompt: &str, model: Option<&str>) -> JsonValue {
    let mut params = json!({
        "threadId": thread_id,
        "input": [{ "type": "text", "text": prompt }],
    });
    if let Some(model) = model {
        params["model"] = JsonValue::String(model.to_string());
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

pub(super) struct CodexClient<R, W, O> {
    reader: R,
    writer: W,
    out: O,
    next_id: i64,
    model: Option<String>,
}

pub(super) fn spawn_codex_app_server(
    config: &CodexRunConfig,
    worktree_dir: &Path,
) -> io::Result<CodexClient<BufReader<ChildStdout>, ChildStdin, io::Stdout>> {
    let mut command = codex_app_server_command(worktree_dir);
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;

    let writer = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("codex app-server stdin is unavailable"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("codex app-server stdout is unavailable"))?;

    Ok(CodexClient {
        reader: BufReader::new(stdout),
        writer,
        out: io::stdout(),
        next_id: 0,
        model: config.model.clone(),
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

impl<R: BufRead, W: Write, O: Write> CodexClient<R, W, O> {
    fn write_line(&mut self, line: &str) -> io::Result<()> {
        self.writer.write_all(line.as_bytes())?;
        self.writer.flush()
    }

    fn read_message(&mut self) -> io::Result<Option<JsonValue>> {
        let mut line = String::new();
        loop {
            line.clear();
            if self.reader.read_line(&mut line)? == 0 {
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

    fn send_request(&mut self, method: &str, params: JsonValue) -> io::Result<JsonValue> {
        let id = self.next_id;
        self.next_id += 1;
        self.write_line(&request_line(id, method, params))?;

        loop {
            let message = self.read_message()?.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("codex app-server closed before responding to request {id}"),
                )
            })?;

            if message.get("method").is_some() {
                if message.get("id").is_none() {
                    self.forward_delta(&message)?;
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

    fn forward_delta(&mut self, message: &JsonValue) -> io::Result<()> {
        if message.get("method").and_then(JsonValue::as_str) == Some(METHOD_AGENT_MESSAGE_DELTA) {
            if let Some(delta) = message.pointer("/params/delta").and_then(JsonValue::as_str) {
                self.out.write_all(delta.as_bytes())?;
                self.out.flush()?;
            }
        }
        Ok(())
    }

    pub(super) fn initialize(&mut self) -> io::Result<()> {
        self.send_request(METHOD_INITIALIZE, initialize_params())?;
        self.write_line(&notification_line(METHOD_INITIALIZED))
    }

    pub(super) fn thread_start(&mut self, cwd: &Path) -> io::Result<String> {
        let params = thread_start_params(cwd, self.model.as_deref());
        let result = self.send_request(METHOD_THREAD_START, params)?;
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

    pub(super) fn turn_start(&mut self, thread_id: &str, prompt: &str) -> io::Result<String> {
        let params = turn_start_params(thread_id, prompt, self.model.as_deref());
        let result = self.send_request(METHOD_TURN_START, params)?;
        result
            .pointer("/turn/id")
            .and_then(JsonValue::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "codex turn/start response is missing turn.id",
                )
            })
    }

    fn turn_interrupt(&mut self, thread_id: &str, turn_id: &str) -> io::Result<()> {
        self.send_request(
            METHOD_TURN_INTERRUPT,
            json!({ "threadId": thread_id, "turnId": turn_id }),
        )?;
        Ok(())
    }

    pub(super) fn pump_until_turn_completed(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        interrupt: &AtomicBool,
    ) -> io::Result<TurnStatus> {
        let mut interrupt_sent = false;
        loop {
            if !interrupt_sent && interrupt.load(Ordering::SeqCst) {
                self.turn_interrupt(thread_id, turn_id)?;
                interrupt_sent = true;
            }

            let message = self.read_message()?.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "codex app-server closed before the turn completed",
                )
            })?;

            if message.get("id").is_some() || message.get("method").is_none() {
                continue;
            }

            if let Some(delta) = delta_for_turn(&message, thread_id, turn_id) {
                self.out.write_all(delta.as_bytes())?;
                self.out.flush()?;
                continue;
            }
            if let Some(status) = completed_status_for_turn(&message, thread_id, turn_id) {
                return status;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::path::PathBuf;

    use super::*;

    fn parse(line: &str) -> JsonValue {
        serde_json::from_str(line).expect("line is JSON")
    }

    fn client_over(input: &str) -> CodexClient<Cursor<Vec<u8>>, Vec<u8>, Vec<u8>> {
        CodexClient {
            reader: Cursor::new(input.as_bytes().to_vec()),
            writer: Vec::new(),
            out: Vec::new(),
            next_id: 0,
            model: None,
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
    fn thread_start_params_encode_never_approvals_and_full_access() {
        let worktree_dir = PathBuf::from("/repo/with space");
        let params = thread_start_params(&worktree_dir, Some("o3"));
        let command = codex_app_server_command(&worktree_dir);

        assert_eq!(params["approvalPolicy"], json!("never"));
        assert_eq!(params["sandbox"], json!("danger-full-access"));
        assert_eq!(params["cwd"], json!("/repo/with space"));
        assert_eq!(params["model"], json!("o3"));
        assert_eq!(command.get_program(), "codex");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["app-server", "--stdio"]
        );
        assert_eq!(command.get_current_dir(), Some(worktree_dir.as_path()));
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
        let params = turn_start_params("th_1", "do the thing", Some("o3"));

        assert_eq!(params["threadId"], json!("th_1"));
        assert_eq!(params["input"][0]["type"], json!("text"));
        assert_eq!(params["input"][0]["text"], json!("do the thing"));
        assert_eq!(params["model"], json!("o3"));
    }

    #[test]
    fn turn_start_params_omit_model_when_unset() {
        let params = turn_start_params("th_1", "prompt", None);

        assert!(params.get("model").is_none());
        assert_eq!(params["threadId"], json!("th_1"));
    }

    #[test]
    fn thread_start_extracts_thread_id_skipping_notifications() {
        let input = concat!(
            "{\"method\":\"thread/started\",\"params\":{\"thread\":{\"id\":\"th_x\"}}}\n",
            "{\"id\":0,\"result\":{\"thread\":{\"id\":\"th_abc\"}}}\n",
        );
        let mut client = client_over(input);

        let thread_id = client.thread_start(&PathBuf::from("/repo")).unwrap();

        assert_eq!(thread_id, "th_abc");
        let request = parse(&String::from_utf8(client.writer.clone()).unwrap());
        assert_eq!(request["method"], json!("thread/start"));
    }

    #[test]
    fn turn_start_extracts_turn_id() {
        let input = "{\"id\":0,\"result\":{\"turn\":{\"id\":\"tu_99\"}}}\n";
        let mut client = client_over(input);

        let turn_id = client.turn_start("th_1", "go").unwrap();

        assert_eq!(turn_id, "tu_99");
        let request = parse(&String::from_utf8(client.writer.clone()).unwrap());
        assert_eq!(request["method"], json!("turn/start"));
        assert_eq!(request["params"]["input"][0]["text"], json!("go"));
    }

    #[test]
    fn pump_returns_status_for_each_turn_completed_value() {
        for (wire, expected) in [
            ("completed", TurnStatus::Completed),
            ("failed", TurnStatus::Failed),
            ("interrupted", TurnStatus::Interrupted),
            ("inProgress", TurnStatus::InProgress),
        ] {
            let input = format!(
                "{{\"method\":\"turn/completed\",\"params\":{{\"threadId\":\"th\",\"turn\":{{\"id\":\"tu\",\"status\":\"{wire}\"}}}}}}\n"
            );
            let mut client = client_over(&input);

            let status = client
                .pump_until_turn_completed("th", "tu", &AtomicBool::new(false))
                .unwrap();

            assert_eq!(status, expected, "wire status {wire}");
        }
    }

    #[test]
    fn pump_rejects_noncanonical_turn_status() {
        for wire in ["Completed", "Interrupted", "Failed", "InProgress"] {
            let input = format!(
                "{{\"method\":\"turn/completed\",\"params\":{{\"threadId\":\"th\",\"turn\":{{\"id\":\"tu\",\"status\":\"{wire}\"}}}}}}\n"
            );
            let mut client = client_over(&input);

            let error = client
                .pump_until_turn_completed("th", "tu", &AtomicBool::new(false))
                .expect_err("noncanonical status must error");

            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        }
    }

    #[test]
    fn pump_forwards_and_concatenates_agent_message_deltas() {
        let input = concat!(
            "{\"method\":\"item/agentMessage/delta\",\"params\":{\"threadId\":\"th\",\"turnId\":\"tu\",\"itemId\":\"it\",\"delta\":\"Hello, \"}}\n",
            "{\"method\":\"item/agentMessage/delta\",\"params\":{\"threadId\":\"th\",\"turnId\":\"tu\",\"itemId\":\"it\",\"delta\":\"world\"}}\n",
            "{\"method\":\"turn/completed\",\"params\":{\"threadId\":\"th\",\"turn\":{\"id\":\"tu\",\"status\":\"completed\"}}}\n",
        );
        let mut client = client_over(input);

        let status = client
            .pump_until_turn_completed("th", "tu", &AtomicBool::new(false))
            .unwrap();

        assert_eq!(status, TurnStatus::Completed);
        assert_eq!(
            String::from_utf8(client.out.clone()).unwrap(),
            "Hello, world"
        );
    }

    #[test]
    fn pump_errors_on_eof_before_turn_completed() {
        let input = "{\"method\":\"item/agentMessage/delta\",\"params\":{\"threadId\":\"th\",\"turnId\":\"tu\",\"itemId\":\"it\",\"delta\":\"x\"}}\n";
        let mut client = client_over(input);

        let error = client
            .pump_until_turn_completed("th", "tu", &AtomicBool::new(false))
            .expect_err("EOF before turn/completed must error");

        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn is_success_only_for_completed() {
        assert!(TurnStatus::Completed.is_success());
        assert!(!TurnStatus::Failed.is_success());
        assert!(!TurnStatus::Interrupted.is_success());
        assert!(!TurnStatus::InProgress.is_success());
    }

    #[test]
    fn pump_interrupts_when_flag_is_set_and_returns_interrupted_status() {
        let input = concat!(
            "{\"id\":0,\"result\":{}}\n",
            "{\"method\":\"turn/completed\",\"params\":{\"threadId\":\"th\",\"turn\":{\"id\":\"tu\",\"status\":\"interrupted\"}}}\n",
        );
        let mut client = client_over(input);
        let interrupt = AtomicBool::new(true);

        let status = client
            .pump_until_turn_completed("th", "tu", &interrupt)
            .unwrap();

        assert_eq!(status, TurnStatus::Interrupted);
        let request = parse(&String::from_utf8(client.writer.clone()).unwrap());
        assert_eq!(request["method"], json!("turn/interrupt"));
        assert_eq!(request["params"]["threadId"], json!("th"));
        assert_eq!(request["params"]["turnId"], json!("tu"));
    }

    #[test]
    fn pump_does_not_interrupt_when_flag_is_unset() {
        let input = "{\"method\":\"turn/completed\",\"params\":{\"threadId\":\"th\",\"turn\":{\"id\":\"tu\",\"status\":\"completed\"}}}\n";
        let mut client = client_over(input);

        let status = client
            .pump_until_turn_completed("th", "tu", &AtomicBool::new(false))
            .unwrap();

        assert_eq!(status, TurnStatus::Completed);
        assert!(client.writer.is_empty(), "no request should be written");
    }

    #[test]
    fn signal_status_maps_zero_and_one_to_ok_and_other_to_err() {
        for code in [0, 1] {
            let mut command = ProcessCommand::new("sh");
            command.arg("-c").arg(format!("exit {code}"));
            assert!(
                map_pkill_status(command).is_ok(),
                "exit {code} should be Ok"
            );
        }

        let mut command = ProcessCommand::new("sh");
        command.arg("-c").arg("exit 2");
        let error = map_pkill_status(command).expect_err("exit 2 should be Err");
        assert!(error.to_string().contains("status 2"));
    }
}
