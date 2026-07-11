use std::env;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::blocking::{Client, Response};
use serde_json::{json, Value as JsonValue};

use super::backend::{
    AbortContext, AgentSystemBackend, RunHandle, RunOutcome, StartContext, StartedRun,
};

pub(super) struct OpencodeBackend {
    config: OpencodeRunConfig,
}

impl OpencodeBackend {
    pub(super) fn from_env() -> io::Result<Self> {
        Ok(Self {
            config: opencode_run_config_from_env()?,
        })
    }
}

impl AgentSystemBackend for OpencodeBackend {
    fn start(&mut self, context: StartContext<'_>) -> io::Result<StartedRun> {
        let repository_root = opencode_repository_root(context.repository_root)?;
        let client = opencode_client()?;
        let session_id = create_opencode_session(&client, &self.config, &repository_root)?;

        // Open the repository-wide stream before submitting work so a fast agent cannot finish
        // before the monitor is listening for its session events.
        let events = subscribe_opencode_events(&client, &self.config, &repository_root)?;
        submit_opencode_prompt(
            &client,
            &self.config,
            &session_id,
            &repository_root,
            context.prompt,
        )?;

        Ok(StartedRun {
            session_id: session_id.clone(),
            handle: Box::new(OpencodeRun { events, session_id }),
        })
    }

    fn abort(&mut self, context: AbortContext<'_>) -> io::Result<()> {
        let repository_root = opencode_repository_root(context.waap_root)?;
        abort_opencode_session(&self.config, context.session_id, &repository_root)
    }
}

struct OpencodeRun {
    events: Response,
    session_id: String,
}

impl RunHandle for OpencodeRun {
    fn wait(self: Box<Self>) -> io::Result<RunOutcome> {
        let stdout = io::stdout();
        monitor_opencode_events(
            BufReader::new(self.events),
            &self.session_id,
            &mut stdout.lock(),
        )
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
    let Some((provider_id, model_id)) = model.split_once('/') else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "OPENCODE_SERVER_MODEL must use provider/model format",
        ));
    };
    if provider_id.is_empty() || model_id.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "OPENCODE_SERVER_MODEL must use provider/model format",
        ));
    }
    Ok(OpencodeModel {
        provider_id: provider_id.to_string(),
        model_id: model_id.to_string(),
    })
}

fn opencode_client() -> io::Result<Client> {
    Client::builder()
        .timeout(None)
        .build()
        .map_err(opencode_http_error)
}

fn create_opencode_session(
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
        .and_then(Response::error_for_status)
        .map_err(opencode_http_error)?
        .json()
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

fn subscribe_opencode_events(
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
        .and_then(Response::error_for_status)
        .map_err(opencode_http_error)
}

fn submit_opencode_prompt(
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
        .and_then(Response::error_for_status)
        .map_err(opencode_http_error)?;
    Ok(())
}

fn abort_opencode_session(
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
    json!({
        "model": {
            "providerID": config.model.provider_id,
            "modelID": config.model.model_id,
        },
        "agent": "build",
        "parts": [{ "type": "text", "text": prompt }],
    })
}

fn opencode_url(config: &OpencodeRunConfig, path: &str) -> String {
    format!("{}{}", config.server_url.trim_end_matches('/'), path)
}

fn monitor_opencode_events<R: BufRead, W: Write>(
    mut reader: R,
    session_id: &str,
    output: &mut W,
) -> io::Result<RunOutcome> {
    let mut data_lines = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "opencode event stream ended before the session became idle",
            ));
        }

        let line = line.strip_suffix('\n').unwrap_or(&line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            if let Some(outcome) = handle_sse_event(&data_lines.join("\n"), session_id, output)? {
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
mod tests {
    use std::collections::BTreeMap;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::Path;
    use std::thread;

    use serde_json::json;
    use tempfile::tempdir;

    use super::{
        abort_opencode_session, create_session_payload, monitor_opencode_events,
        opencode_directory_query, opencode_repository_root, opencode_url, parse_opencode_model,
        prompt_payload, AgentSystemBackend, OpencodeBackend, OpencodeRunConfig, RunOutcome,
        StartContext,
    };

    #[test]
    fn opencode_create_session_payload_matches_api() {
        assert_eq!(
            create_session_payload(),
            json!({
                "permission": [
                    { "permission": "question", "action": "deny", "pattern": "*" },
                    { "permission": "plan_enter", "action": "deny", "pattern": "*" },
                    { "permission": "plan_exit", "action": "deny", "pattern": "*" },
                ]
            })
        );
    }

    #[test]
    fn opencode_model_parsing_preserves_nested_model_ids() {
        assert_eq!(
            parse_opencode_model("openai/gpt-5.5/reasoning").unwrap(),
            super::OpencodeModel {
                provider_id: "openai".to_string(),
                model_id: "gpt-5.5/reasoning".to_string(),
            }
        );
        for invalid in ["openai", "/gpt-5.5", "openai/"] {
            assert!(parse_opencode_model(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn opencode_prompt_payload_matches_async_api() {
        let config = OpencodeRunConfig::for_test();

        assert_eq!(
            prompt_payload(&config, "complete the work"),
            json!({
                "model": { "providerID": "openai", "modelID": "gpt-5.5" },
                "agent": "build",
                "parts": [{ "type": "text", "text": "complete the work" }],
            })
        );
    }

    #[test]
    fn opencode_url_trims_trailing_slash() {
        let mut config = OpencodeRunConfig::for_test();
        config.server_url = "https://opencode.example/".to_string();

        assert_eq!(
            opencode_url(&config, "/session"),
            "https://opencode.example/session"
        );
    }

    #[test]
    fn opencode_session_and_abort_use_canonical_repository_root() {
        let dir = tempdir().unwrap();
        let repository_root = opencode_repository_root(dir.path()).unwrap();

        assert_eq!(repository_root, dir.path().canonicalize().unwrap());
        assert_eq!(
            opencode_directory_query(&repository_root),
            [("directory", repository_root.display().to_string())]
        );
    }

    #[test]
    fn async_runner_subscribes_before_prompt_and_scopes_every_request() {
        let root = tempdir().unwrap();
        let repository_root = root.path().canonicalize().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let expected_root = repository_root.clone();
        let server = thread::spawn(move || {
            let mut create = listener.accept().unwrap().0;
            let request = read_request(&mut create);
            assert_request(&request, "POST", "/session", &expected_root);
            assert_eq!(request.body, create_session_payload());
            write_response(&mut create, "HTTP/1.1 200 OK", Some(r#"{"id":"ses_123"}"#));
            drop(create);

            let mut events = listener.accept().unwrap().0;
            let request = read_request(&mut events);
            assert_request(&request, "GET", "/event", &expected_root);
            events
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n: connected\n\n",
                )
                .unwrap();
            events.flush().unwrap();

            let mut prompt = listener.accept().unwrap().0;
            let request = read_request(&mut prompt);
            assert_request(
                &request,
                "POST",
                "/session/ses_123/prompt_async",
                &expected_root,
            );
            assert_eq!(
                request.body,
                json!({
                    "model": { "providerID": "openai", "modelID": "gpt-5.5" },
                    "agent": "build",
                    "parts": [{ "type": "text", "text": "complete the work" }],
                })
            );
            write_response(&mut prompt, "HTTP/1.1 204 No Content", None);
            drop(prompt);

            events
                .write_all(
                    b"data: {\"type\":\"session.status\",\"properties\":{\"sessionID\":\"ses_123\",\"status\":{\"type\":\"idle\"}}}\n\n",
                )
                .unwrap();
        });
        let mut backend = OpencodeBackend {
            config: OpencodeRunConfig {
                server_url: format!("http://{address}"),
                ..OpencodeRunConfig::for_test()
            },
        };

        let started = backend
            .start(StartContext {
                agent_id: "aa-00000001",
                prompt: "complete the work",
                repository_root: &repository_root,
                worktree_dir: root.path(),
            })
            .unwrap();

        assert_eq!(started.session_id, "ses_123");
        assert_eq!(started.handle.wait().unwrap(), RunOutcome::Completed);
        server.join().unwrap();
    }

    #[test]
    fn abort_request_is_authenticated_and_scoped_to_repository_root() {
        let root = tempdir().unwrap();
        let repository_root = root.path().canonicalize().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let expected_root = repository_root.clone();
        let server = thread::spawn(move || {
            let mut stream = listener.accept().unwrap().0;
            let request = read_request(&mut stream);
            assert_request(&request, "POST", "/session/ses_123/abort", &expected_root);
            write_response(&mut stream, "HTTP/1.1 204 No Content", None);
        });
        let config = OpencodeRunConfig {
            server_url: format!("http://{address}"),
            ..OpencodeRunConfig::for_test()
        };

        abort_opencode_session(&config, "ses_123", &repository_root).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn monitor_parses_sse_framing_and_emits_cli_compatible_json_lines() {
        let input = concat!(
            ": connected\n\n",
            "event: message.part.updated\n",
            "data: {\"type\":\"message.part.updated\",\n",
            "data: \"properties\":{\"part\":{\"sessionID\":\"ses_other\",\"type\":\"text\",\"time\":{\"end\":1}}}}\n\n",
            "data: {\"type\":\"message.part.updated\",\"properties\":{\"part\":{\"sessionID\":\"ses_123\",\"type\":\"tool\",\"state\":{\"status\":\"completed\"}}}}\n\n",
            "data: {\"type\":\"message.part.updated\",\"properties\":{\"part\":{\"sessionID\":\"ses_123\",\"type\":\"step-start\"}}}\n\n",
            "data: {\"type\":\"message.part.updated\",\"properties\":{\"part\":{\"sessionID\":\"ses_123\",\"type\":\"step-finish\"}}}\n\n",
            "data: {\"type\":\"message.part.updated\",\"properties\":{\"part\":{\"sessionID\":\"ses_123\",\"type\":\"text\",\"time\":{\"end\":1}}}}\n\n",
            "data: {\"type\":\"session.status\",\"properties\":{\"sessionID\":\"ses_other\",\"status\":{\"type\":\"idle\"}}}\n\n",
            "data: {\"type\":\"session.status\",\"properties\":{\"sessionID\":\"ses_123\",\"status\":{\"type\":\"idle\"}}}\n\n",
        );
        let mut output = Vec::new();

        assert_eq!(
            monitor_opencode_events(BufReader::new(input.as_bytes()), "ses_123", &mut output)
                .unwrap(),
            RunOutcome::Completed
        );
        let lines = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();

        assert_eq!(
            lines
                .iter()
                .map(|line| line["type"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["tool_use", "step_start", "step_finish", "text"]
        );
        assert!(lines.iter().all(|line| line["sessionID"] == "ses_123"));
        assert!(lines.iter().all(|line| line["timestamp"].is_u64()));
    }

    #[test]
    fn monitor_reports_matching_session_errors_as_failed_output() {
        let input = concat!(
            "data: {\"type\":\"session.error\",\"properties\":{\"sessionID\":\"ses_123\",\"error\":{\"name\":\"UnknownError\",\"data\":{\"message\":\"failed\"}}}}\n\n",
        );
        let mut output = Vec::new();

        assert_eq!(
            monitor_opencode_events(BufReader::new(input.as_bytes()), "ses_123", &mut output)
                .unwrap(),
            RunOutcome::Failed(std::process::ExitCode::from(1))
        );
        let event = serde_json::from_slice::<serde_json::Value>(&output).unwrap();
        assert_eq!(event["type"], "error");
        assert!(event["timestamp"].is_u64());
        assert_eq!(event["sessionID"], "ses_123");
        assert_eq!(
            event["error"],
            json!({ "name": "UnknownError", "data": { "message": "failed" } })
        );
    }

    #[test]
    fn monitor_rejects_malformed_events_and_premature_disconnects() {
        for input in [
            "data: not json\n\n",
            "data: {\"type\":\"session.status\",\"properties\":{\"sessionID\":\"ses_123\"}}\n\n",
            "data: {\"type\":\"session.status\",\"properties\":{\"sessionID\":\"ses_other\",\"status\":{\"type\":\"idle\"}}}\n\n",
            "bad field\n",
        ] {
            let error = monitor_opencode_events(BufReader::new(input.as_bytes()), "ses_123", &mut Vec::new())
                .unwrap_err();
            assert!(
                matches!(error.kind(), std::io::ErrorKind::InvalidData | std::io::ErrorKind::UnexpectedEof),
                "{}",
                error
            );
        }
    }

    #[test]
    fn rejected_event_subscription_fails_before_prompt_submission() {
        let root = tempdir().unwrap();
        let repository_root = root.path().canonicalize().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let expected_root = repository_root.clone();
        let server = thread::spawn(move || {
            let mut create = listener.accept().unwrap().0;
            let request = read_request(&mut create);
            assert_request(&request, "POST", "/session", &expected_root);
            write_response(&mut create, "HTTP/1.1 200 OK", Some(r#"{"id":"ses_123"}"#));
            drop(create);

            let mut events = listener.accept().unwrap().0;
            let request = read_request(&mut events);
            assert_request(&request, "GET", "/event", &expected_root);
            write_response(
                &mut events,
                "HTTP/1.1 503 Service Unavailable",
                Some("unavailable"),
            );
        });
        let mut backend = OpencodeBackend {
            config: OpencodeRunConfig {
                server_url: format!("http://{address}"),
                ..OpencodeRunConfig::for_test()
            },
        };

        let error = match backend.start(StartContext {
            agent_id: "aa-00000001",
            prompt: "complete the work",
            repository_root: &repository_root,
            worktree_dir: root.path(),
        }) {
            Ok(_) => panic!("event subscription unexpectedly succeeded"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("503"));
        server.join().unwrap();
    }

    struct HttpRequest {
        method: String,
        target: String,
        headers: BTreeMap<String, String>,
        body: serde_json::Value,
    }

    fn read_request(stream: &mut TcpStream) -> HttpRequest {
        let mut reader = BufReader::new(stream);
        let mut request_line = String::new();
        reader.read_line(&mut request_line).unwrap();
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap().to_string();
        let target = parts.next().unwrap().to_string();
        assert_eq!(parts.next(), Some("HTTP/1.1"));

        let mut headers = BTreeMap::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            let (name, value) = line.trim_end().split_once(':').unwrap();
            headers.insert(name.to_ascii_lowercase(), value.trim().to_string());
        }
        let content_length = headers
            .get("content-length")
            .map(|value| value.parse::<usize>().unwrap())
            .unwrap_or(0);
        let mut body = vec![0; content_length];
        reader.read_exact(&mut body).unwrap();

        HttpRequest {
            method,
            target,
            headers,
            body: if body.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::from_slice(&body).unwrap()
            },
        }
    }

    fn assert_request(request: &HttpRequest, method: &str, path: &str, repository_root: &Path) {
        assert_eq!(request.method, method);
        let (request_path, query) = request.target.split_once('?').unwrap();
        assert_eq!(request_path, path);
        assert_eq!(
            query_parameter(query, "directory"),
            Some(repository_root.display().to_string())
        );
        assert_eq!(
            request.headers.get("authorization"),
            Some(&"Basic cnVubmVyOnNlY3JldA==".to_string())
        );
    }

    fn query_parameter(query: &str, name: &str) -> Option<String> {
        query.split('&').find_map(|entry| {
            let (key, value) = entry.split_once('=')?;
            (key == name).then(|| percent_decode(value))
        })
    }

    fn percent_decode(value: &str) -> String {
        let mut decoded = Vec::new();
        let bytes = value.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            match bytes[index] {
                b'%' if index + 2 < bytes.len() => {
                    decoded.push(hex_value(bytes[index + 1]) * 16 + hex_value(bytes[index + 2]));
                    index += 3;
                }
                b'+' => {
                    decoded.push(b' ');
                    index += 1;
                }
                byte => {
                    decoded.push(byte);
                    index += 1;
                }
            }
        }
        String::from_utf8(decoded).unwrap()
    }

    fn hex_value(value: u8) -> u8 {
        match value {
            b'0'..=b'9' => value - b'0',
            b'a'..=b'f' => value - b'a' + 10,
            b'A'..=b'F' => value - b'A' + 10,
            _ => panic!("invalid percent encoding"),
        }
    }

    fn write_response(stream: &mut TcpStream, status: &str, body: Option<&str>) {
        let body = body.unwrap_or_default();
        write!(
            stream,
            "{status}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        stream.flush().unwrap();
    }
}
