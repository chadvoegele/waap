use futures_util::TryStreamExt;
use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc::Receiver;

use reqwest::{Client, Response};
use serde_json::{json, Value as JsonValue};

use super::backend::{
    AbortContext, AbortOutcome, AgentSessionRun, RunOutcome, Runner, StartContext,
};
use super::commands::Command;

#[derive(Default)]
pub(super) struct OpencodeBackend {
    config: Option<OpencodeRunConfig>,
}

impl OpencodeBackend {
    pub(super) fn from_env() -> io::Result<Self> {
        Ok(Self {
            config: Some(opencode_run_config_from_env()?),
        })
    }

    fn config(&mut self) -> io::Result<&OpencodeRunConfig> {
        if self.config.is_none() {
            self.config = Some(opencode_run_config_from_env()?);
        }
        Ok(self.config.as_ref().expect("configuration initialized"))
    }

    pub(super) async fn start(
        &mut self,
        context: StartContext<'_>,
        receiver: Receiver<Command>,
    ) -> io::Result<AgentSessionRun> {
        let config = self.config()?;
        let repository_root = opencode_repository_root(context.repository_root)?;
        let client = opencode_client()?;
        let session_id = create_opencode_session(&client, config, &repository_root).await?;
        // Subscribe first so a fast agent cannot finish before monitoring begins.
        let events = subscribe_opencode_events(&client, config, &repository_root).await?;
        submit_opencode_prompt(
            &client,
            config,
            &session_id,
            &repository_root,
            context.prompt,
        )
        .await?;
        Ok(AgentSessionRun::new(
            session_id.clone(),
            Runner::Opencode(OpencodeRunner {
                events: Some(events),
                session_id,
            }),
            receiver,
        ))
    }
    pub(super) async fn abort(&mut self, context: AbortContext<'_>) -> io::Result<AbortOutcome> {
        if let Some(session_id) = context.session_id {
            let config = self.config()?;
            let root = opencode_repository_root(context.waap_root)?;
            abort_opencode_session(config, session_id, &root).await?;
        }
        Ok(AbortOutcome::Stopped)
    }
}

pub(super) struct OpencodeRunner {
    events: Option<Response>,
    session_id: String,
}
impl OpencodeRunner {
    pub(super) async fn run(&mut self, receiver: Receiver<Command>) -> io::Result<RunOutcome> {
        let events = self
            .events
            .take()
            .ok_or_else(|| io::Error::other("OpenCode run already waited"))?;
        let stream = events.bytes_stream().map_err(io::Error::other);
        let reader = tokio::io::BufReader::new(tokio_util::io::StreamReader::new(stream));
        super::unsupported_commands::await_with_unsupported_commands(
            monitor_opencode_events(reader, &self.session_id, &mut tokio::io::stdout()),
            receiver,
            "opencode",
        )
        .await
    }
}

fn opencode_repository_root(repository_root: &Path) -> io::Result<PathBuf> {
    repository_root.canonicalize()
}

#[derive(Debug, PartialEq, Eq)]
struct OpencodeRunConfig {
    server_url: String,
    username: String,
    password: String,
    model: OpencodeModel,
}

#[derive(Debug, PartialEq, Eq)]
struct OpencodeModel {
    provider_id: String,
    model_id: String,
    variant: Option<String>,
}

#[cfg(test)]
impl OpencodeRunConfig {
    fn for_test() -> Self {
        Self {
            server_url: "https://opencode.example".to_string(),
            username: "runner".to_string(),
            password: "secret".to_string(),
            model: parse_opencode_model("openai/gpt-5.5").unwrap(),
        }
    }
}

fn opencode_run_config_from_env() -> io::Result<OpencodeRunConfig> {
    Ok(OpencodeRunConfig {
        server_url: required_env("OPENCODE_SERVER_URL")?,
        username: required_env("OPENCODE_SERVER_USERNAME")?,
        password: required_env("OPENCODE_SERVER_PASSWORD")?,
        model: parse_opencode_model(&required_env("OPENCODE_SERVER_MODEL")?)?,
    })
}

fn required_env(name: &str) -> io::Result<String> {
    env::var(name).map_err(|_| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("{name} environment variable is required"),
        )
    })
}

fn parse_opencode_model(model: &str) -> io::Result<OpencodeModel> {
    const VARIANTS: [&str; 7] = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];

    let Some((provider_id, model_id)) = model.split_once('/') else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "OPENCODE_SERVER_MODEL must use provider/model or provider/model/variant format",
        ));
    };
    if provider_id.trim().is_empty()
        || provider_id.trim() != provider_id
        || model_id.trim().is_empty()
        || model_id.trim() != model_id
        || model_id
            .split('/')
            .any(|segment| segment.trim().is_empty() || segment.trim() != segment)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "OPENCODE_SERVER_MODEL must use non-empty provider/model or provider/model/variant segments",
        ));
    }

    let (model_id, variant) = model_id
        .rsplit_once('/')
        .filter(|(_, variant)| VARIANTS.contains(variant))
        .map_or((model_id, None), |(model_id, variant)| {
            (model_id, Some(variant.to_string()))
        });
    Ok(OpencodeModel {
        provider_id: provider_id.to_string(),
        model_id: model_id.to_string(),
        variant,
    })
}

fn opencode_client() -> io::Result<Client> {
    Client::builder().build().map_err(opencode_http_error)
}

async fn create_opencode_session(
    client: &Client,
    config: &OpencodeRunConfig,
    repository_root: &Path,
) -> io::Result<String> {
    let response: JsonValue = client
        .post(opencode_url(config, "/session"))
        .basic_auth(&config.username, Some(&config.password))
        .query(&opencode_directory_query(repository_root))
        .json(&create_session_payload())
        .send()
        .await
        .and_then(Response::error_for_status)
        .map_err(opencode_http_error)?
        .json()
        .await
        .map_err(opencode_http_error)?;

    response
        .get("id")
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "opencode session create response is missing id",
            )
        })
}

async fn subscribe_opencode_events(
    client: &Client,
    config: &OpencodeRunConfig,
    repository_root: &Path,
) -> io::Result<Response> {
    client
        .get(opencode_url(config, "/event"))
        .basic_auth(&config.username, Some(&config.password))
        .query(&opencode_directory_query(repository_root))
        .header(reqwest::header::ACCEPT, "text/event-stream")
        .send()
        .await
        .and_then(Response::error_for_status)
        .map_err(opencode_http_error)
}

async fn submit_opencode_prompt(
    client: &Client,
    config: &OpencodeRunConfig,
    session_id: &str,
    repository_root: &Path,
    prompt: &str,
) -> io::Result<()> {
    client
        .post(opencode_url(
            config,
            &format!("/session/{session_id}/prompt_async"),
        ))
        .basic_auth(&config.username, Some(&config.password))
        .query(&opencode_directory_query(repository_root))
        .json(&prompt_payload(config, prompt))
        .send()
        .await
        .and_then(Response::error_for_status)
        .map_err(opencode_http_error)?;
    Ok(())
}

async fn abort_opencode_session(
    config: &OpencodeRunConfig,
    session_id: &str,
    repository_root: &Path,
) -> io::Result<()> {
    opencode_client()?
        .post(opencode_url(
            config,
            &format!("/session/{session_id}/abort"),
        ))
        .basic_auth(&config.username, Some(&config.password))
        .query(&opencode_directory_query(repository_root))
        .send()
        .await
        .and_then(Response::error_for_status)
        .map_err(opencode_http_error)?;
    Ok(())
}

fn opencode_directory_query(repository_root: &Path) -> [(&'static str, String); 1] {
    [("directory", repository_root.display().to_string())]
}

fn opencode_http_error(error: reqwest::Error) -> io::Error {
    io::Error::other(format!("opencode HTTP request failed: {error}"))
}

fn create_session_payload() -> JsonValue {
    json!({
        "permission": [
            { "permission": "question", "action": "deny", "pattern": "*" },
            { "permission": "plan_enter", "action": "deny", "pattern": "*" },
            { "permission": "plan_exit", "action": "deny", "pattern": "*" },
        ]
    })
}

fn prompt_payload(config: &OpencodeRunConfig, prompt: &str) -> JsonValue {
    let mut payload = json!({
        "model": {
            "providerID": config.model.provider_id,
            "modelID": config.model.model_id,
        },
        "agent": "build",
        "parts": [{ "type": "text", "text": prompt }],
    });
    if let Some(variant) = &config.model.variant {
        payload["variant"] = JsonValue::String(variant.clone());
    }
    payload
}

fn opencode_url(config: &OpencodeRunConfig, path: &str) -> String {
    format!("{}{}", config.server_url.trim_end_matches('/'), path)
}

async fn monitor_opencode_events<R: AsyncBufRead + Unpin, W: AsyncWrite + Unpin>(
    mut reader: R,
    session_id: &str,
    output: &mut W,
) -> io::Result<RunOutcome> {
    let mut data_lines = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "opencode event stream ended before the session became idle",
            ));
        }

        let line = line.strip_suffix('\n').unwrap_or(&line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            let mut rendered = Vec::new();
            let outcome = handle_sse_event(&data_lines.join("\n"), session_id, &mut rendered)?;
            output.write_all(&rendered).await?;
            output.flush().await?;
            if let Some(outcome) = outcome {
                return Ok(outcome);
            }
            data_lines.clear();
            continue;
        }
        if line.starts_with(':') {
            continue;
        }

        let Some((field, value)) = line.split_once(':') else {
            return Err(invalid_event("malformed opencode SSE field"));
        };
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "data" => data_lines.push(value.to_string()),
            "event" | "id" | "retry" => {}
            _ => return Err(invalid_event("malformed opencode SSE field")),
        }
    }
}

fn handle_sse_event<W: Write>(
    data: &str,
    session_id: &str,
    output: &mut W,
) -> io::Result<Option<RunOutcome>> {
    if data.is_empty() {
        return Ok(None);
    }
    let event: JsonValue = serde_json::from_str(data)
        .map_err(|error| invalid_event(format!("malformed opencode SSE JSON: {error}")))?;
    match event.get("type").and_then(JsonValue::as_str) {
        Some("message.part.updated") => handle_message_part_event(&event, session_id, output),
        Some("session.status") => handle_session_status_event(&event, session_id),
        Some("session.error") => handle_session_error_event(&event, session_id, output),
        _ => Ok(None),
    }
}

fn handle_message_part_event<W: Write>(
    event: &JsonValue,
    session_id: &str,
    output: &mut W,
) -> io::Result<Option<RunOutcome>> {
    let properties = required_object(event.get("properties"), "message.part.updated properties")?;
    let part = required_object(properties.get("part"), "message.part.updated part")?;
    if required_string(part, "sessionID", "message.part.updated part")? != session_id {
        return Ok(None);
    }

    let part_type = required_string(part, "type", "message.part.updated part")?;
    match part_type {
        "tool" => {
            let state = required_object(part.get("state"), "tool part state")?;
            let status = required_string(state, "status", "tool part state")?;
            if matches!(status, "completed" | "error") {
                emit_opencode_output("tool_use", session_id, "part", part, output)?;
            }
        }
        "step-start" => emit_opencode_output("step_start", session_id, "part", part, output)?,
        "step-finish" => emit_opencode_output("step_finish", session_id, "part", part, output)?,
        "text" if text_part_completed(part) => {
            emit_opencode_output("text", session_id, "part", part, output)?;
        }
        _ => {}
    }
    Ok(None)
}

fn handle_session_status_event(
    event: &JsonValue,
    session_id: &str,
) -> io::Result<Option<RunOutcome>> {
    let properties = required_object(event.get("properties"), "session.status properties")?;
    if required_string(properties, "sessionID", "session.status properties")? != session_id {
        return Ok(None);
    }
    let status = required_object(properties.get("status"), "session.status status")?;
    if required_string(status, "type", "session.status status")? == "idle" {
        Ok(Some(RunOutcome::Completed))
    } else {
        Ok(None)
    }
}

fn handle_session_error_event<W: Write>(
    event: &JsonValue,
    session_id: &str,
    output: &mut W,
) -> io::Result<Option<RunOutcome>> {
    let properties = required_object(event.get("properties"), "session.error properties")?;
    if required_string(properties, "sessionID", "session.error properties")? != session_id {
        return Ok(None);
    }
    let error = required_object(properties.get("error"), "session.error error")?;
    emit_opencode_output("error", session_id, "error", error, output)?;
    Ok(Some(RunOutcome::Failed(std::process::ExitCode::from(1))))
}

fn required_object<'a>(
    value: Option<&'a JsonValue>,
    context: &str,
) -> io::Result<&'a serde_json::Map<String, JsonValue>> {
    value
        .and_then(JsonValue::as_object)
        .ok_or_else(|| invalid_event(format!("malformed opencode event: missing {context}")))
}

fn required_string<'a>(
    value: &'a serde_json::Map<String, JsonValue>,
    key: &str,
    context: &str,
) -> io::Result<&'a str> {
    value
        .get(key)
        .and_then(JsonValue::as_str)
        .ok_or_else(|| invalid_event(format!("malformed opencode event: missing {context}.{key}")))
}

fn text_part_completed(part: &serde_json::Map<String, JsonValue>) -> bool {
    part.get("time")
        .and_then(JsonValue::as_object)
        .and_then(|time| time.get("end"))
        .is_some_and(|end| !end.is_null())
}

fn emit_opencode_output<W: Write>(
    event_type: &str,
    session_id: &str,
    key: &str,
    value: &serde_json::Map<String, JsonValue>,
    output: &mut W,
) -> io::Result<()> {
    let mut event = serde_json::Map::new();
    event.insert(
        "type".to_string(),
        JsonValue::String(event_type.to_string()),
    );
    event.insert(
        "timestamp".to_string(),
        JsonValue::from(timestamp_millis()?),
    );
    event.insert(
        "sessionID".to_string(),
        JsonValue::String(session_id.to_string()),
    );
    event.insert(key.to_string(), JsonValue::Object(value.clone()));
    serde_json::to_writer(&mut *output, &JsonValue::Object(event))
        .map_err(|error| io::Error::other(format!("failed to write opencode output: {error}")))?;
    output.write_all(b"\n")?;
    output.flush()
}

fn timestamp_millis() -> io::Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            io::Error::other(format!("system clock is before the Unix epoch: {error}"))
        })?
        .as_millis()
        .try_into()
        .map_err(|_| io::Error::other("Unix epoch milliseconds exceed u64"))
}

fn invalid_event(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
#[path = "opencode_tests.rs"]
mod tests;
