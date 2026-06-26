use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value as JsonValue};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct OpencodeRunConfig {
    pub(crate) server_url: String,
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) model: String,
    pub(crate) repo_root: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct OpencodeRunCommand {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
}

pub(crate) fn opencode_run_config_from_env(repo_root: &Path) -> io::Result<OpencodeRunConfig> {
    Ok(OpencodeRunConfig {
        server_url: required_env("OPENCODE_SERVER_URL")?,
        username: required_env("OPENCODE_SERVER_USERNAME")?,
        password: required_env("OPENCODE_SERVER_PASSWORD")?,
        model: required_env("OPENCODE_SERVER_MODEL")?,
        repo_root: repo_root.canonicalize()?,
    })
}

pub(crate) fn required_env(name: &str) -> io::Result<String> {
    env::var(name).map_err(|_| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("{name} environment variable is required"),
        )
    })
}

pub(crate) fn run_opencode_detached(command: &OpencodeRunCommand) -> io::Result<()> {
    ProcessCommand::new(&command.program)
        .args(&command.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

pub(crate) fn create_opencode_session(config: &OpencodeRunConfig) -> io::Result<String> {
    let response: JsonValue = reqwest::blocking::Client::new()
        .post(opencode_url(config, "/session"))
        .basic_auth(&config.username, Some(&config.password))
        .query(&[("directory", config.repo_root.display().to_string())])
        .json(&create_session_payload())
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
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

pub(crate) fn wait_for_opencode_session_status(
    config: &OpencodeRunConfig,
    session_id: &str,
) -> io::Result<bool> {
    for _ in 0..10 {
        if opencode_session_has_status(config, session_id)? {
            return Ok(true);
        }
        thread::sleep(Duration::from_millis(200));
    }
    Ok(false)
}

pub(crate) fn opencode_session_has_status(
    config: &OpencodeRunConfig,
    session_id: &str,
) -> io::Result<bool> {
    let response: JsonValue = reqwest::blocking::Client::new()
        .get(opencode_url(config, "/session/status"))
        .basic_auth(&config.username, Some(&config.password))
        .query(&[("directory", config.repo_root.display().to_string())])
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(opencode_http_error)?
        .json()
        .map_err(opencode_http_error)?;

    Ok(session_status_type(&response, session_id).is_some())
}

pub(crate) fn abort_opencode_session(
    config: &OpencodeRunConfig,
    session_id: &str,
) -> io::Result<()> {
    reqwest::blocking::Client::new()
        .post(opencode_url(
            config,
            &format!("/session/{session_id}/abort"),
        ))
        .basic_auth(&config.username, Some(&config.password))
        .query(&[("directory", config.repo_root.display().to_string())])
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(opencode_http_error)?;

    Ok(())
}

pub(crate) fn session_status_type<'a>(
    response: &'a JsonValue,
    session_id: &str,
) -> Option<&'a str> {
    response
        .get(session_id)
        .and_then(|status| status.get("type"))
        .and_then(JsonValue::as_str)
}

pub(crate) fn opencode_http_error(error: reqwest::Error) -> io::Error {
    io::Error::other(format!("opencode HTTP request failed: {error}"))
}
pub(crate) fn create_session_payload() -> JsonValue {
    json!({
        "permission": [
            { "permission": "question", "action": "deny", "pattern": "*" },
            { "permission": "plan_enter", "action": "deny", "pattern": "*" },
            { "permission": "plan_exit", "action": "deny", "pattern": "*" },
        ]
    })
}

pub(crate) fn opencode_url(config: &OpencodeRunConfig, path: &str) -> String {
    format!("{}{}", config.server_url.trim_end_matches('/'), path)
}

pub(crate) fn build_opencode_run_command(
    config: &OpencodeRunConfig,
    agent_id: &str,
    session_id: &str,
) -> OpencodeRunCommand {
    OpencodeRunCommand {
        program: "opencode".to_string(),
        args: vec![
            "run".to_string(),
            "--attach".to_string(),
            config.server_url.clone(),
            "--session".to_string(),
            session_id.to_string(),
            "--model".to_string(),
            config.model.clone(),
            "--dir".to_string(),
            config.repo_root.display().to_string(),
            "--agent".to_string(),
            "build".to_string(),
            "--command".to_string(),
            "goal".to_string(),
            "--format".to_string(),
            "json".to_string(),
            format!(
                "Complete when instructions in /.waap/agents/{agent_id}/agent.md are satisfied"
            ),
        ],
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::{
        build_opencode_run_command, create_session_payload, opencode_url, session_status_type,
        OpencodeRunConfig,
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
    fn opencode_url_trims_trailing_slash() {
        let mut config = test_opencode_config();
        config.server_url = "https://opencode.example/".to_string();

        assert_eq!(
            opencode_url(&config, "/session"),
            "https://opencode.example/session"
        );
    }

    #[test]
    fn session_status_type_reads_status_map() {
        let response = json!({
            "ses_123": { "type": "busy" },
            "ses_456": { "type": "idle" },
        });

        assert_eq!(session_status_type(&response, "ses_123"), Some("busy"));
        assert_eq!(session_status_type(&response, "ses_456"), Some("idle"));
        assert_eq!(session_status_type(&response, "ses_missing"), None);
    }

    #[test]
    fn opencode_run_command_matches_spec() {
        let config = test_opencode_config();

        let command = build_opencode_run_command(&config, "aa-3881fda0", "ses_123");

        assert_eq!(command.program, "opencode");
        assert_eq!(
            command.args,
            vec![
                "run".to_string(),
                "--attach".to_string(),
                "https://opencode.example".to_string(),
                "--session".to_string(),
                "ses_123".to_string(),
                "--model".to_string(),
                "openai/gpt-5.5".to_string(),
                "--dir".to_string(),
                "/repo/with space".to_string(),
                "--agent".to_string(),
                "build".to_string(),
                "--command".to_string(),
                "goal".to_string(),
                "--format".to_string(),
                "json".to_string(),
                "Complete when instructions in /.waap/agents/aa-3881fda0/agent.md are satisfied"
                    .to_string(),
            ]
        );
    }

    fn test_opencode_config() -> OpencodeRunConfig {
        OpencodeRunConfig {
            server_url: "https://opencode.example".to_string(),
            username: "runner".to_string(),
            password: "secret".to_string(),
            model: "openai/gpt-5.5".to_string(),
            repo_root: PathBuf::from("/repo/with space"),
        }
    }
}
