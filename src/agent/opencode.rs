use std::env;
use std::io;
use std::path::Path;
use std::process::{Child, Command as ProcessCommand, Stdio};

use serde_json::{json, Value as JsonValue};

#[derive(Debug, PartialEq, Eq)]
pub(super) struct OpencodeRunConfig {
    server_url: String,
    username: String,
    password: String,
    model: String,
}

#[cfg(test)]
impl OpencodeRunConfig {
    pub(super) fn for_test() -> Self {
        Self {
            server_url: "https://opencode.example".to_string(),
            username: "runner".to_string(),
            password: "secret".to_string(),
            model: "openai/gpt-5.5".to_string(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct OpencodeRunCommand {
    program: String,
    args: Vec<String>,
}

pub(super) fn opencode_run_config_from_env() -> io::Result<OpencodeRunConfig> {
    Ok(OpencodeRunConfig {
        server_url: required_env("OPENCODE_SERVER_URL")?,
        username: required_env("OPENCODE_SERVER_USERNAME")?,
        password: required_env("OPENCODE_SERVER_PASSWORD")?,
        model: required_env("OPENCODE_SERVER_MODEL")?,
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

pub(super) fn spawn_opencode_attached(command: &OpencodeRunCommand) -> io::Result<Child> {
    let mut process = ProcessCommand::new(&command.program);
    process
        .args(&command.args)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
}

pub(super) fn create_opencode_session(
    config: &OpencodeRunConfig,
    worktree_dir: &Path,
) -> io::Result<String> {
    let response: JsonValue = reqwest::blocking::Client::new()
        .post(opencode_url(config, "/session"))
        .basic_auth(&config.username, Some(&config.password))
        .query(&opencode_directory_query(worktree_dir))
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

pub(super) fn abort_opencode_session(
    config: &OpencodeRunConfig,
    session_id: &str,
    worktree_dir: &Path,
) -> io::Result<()> {
    reqwest::blocking::Client::new()
        .post(opencode_url(
            config,
            &format!("/session/{session_id}/abort"),
        ))
        .basic_auth(&config.username, Some(&config.password))
        .query(&opencode_directory_query(worktree_dir))
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(opencode_http_error)?;

    Ok(())
}

fn opencode_directory_query(worktree_dir: &Path) -> [(&'static str, String); 1] {
    [("directory", worktree_dir.display().to_string())]
}

fn opencode_http_error(error: reqwest::Error) -> io::Error {
    io::Error::other(format!("opencode HTTP request failed: {error}"))
}

fn opencode_directory(worktree_dir: &Path) -> String {
    worktree_dir.display().to_string()
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

fn opencode_url(config: &OpencodeRunConfig, path: &str) -> String {
    format!("{}{}", config.server_url.trim_end_matches('/'), path)
}

pub(super) fn build_opencode_run_command(
    config: &OpencodeRunConfig,
    agent_id: &str,
    session_id: &str,
    worktree_dir: &Path,
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
            worktree_dir.display().to_string(),
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
        build_opencode_run_command, create_session_payload, opencode_directory_query, opencode_url,
        spawn_opencode_attached, OpencodeRunCommand, OpencodeRunConfig,
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
    fn spawn_opencode_attached_returns_child_with_exit_code() {
        let command = OpencodeRunCommand {
            program: "sh".to_string(),
            args: vec!["-c".to_string(), "exit 3".to_string()],
        };

        let mut child = spawn_opencode_attached(&command).unwrap();
        let status = child.wait().unwrap();

        assert_eq!(status.code(), Some(3));
    }

    #[test]
    fn spawn_opencode_attached_connects_stdin_to_null() {
        let command = OpencodeRunCommand {
            program: "sh".to_string(),
            args: vec!["-c".to_string(), "read value; test $? -ne 0".to_string()],
        };

        let status = spawn_opencode_attached(&command).unwrap().wait().unwrap();

        assert!(status.success());
    }

    #[test]
    fn opencode_run_command_matches_spec() {
        let config = test_opencode_config();
        let worktree_dir = PathBuf::from("/repo/with space");

        let command = build_opencode_run_command(&config, "aa-3881fda0", "ses_123", &worktree_dir);

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
        assert_eq!(
            opencode_directory_query(&worktree_dir),
            [("directory", "/repo/with space".to_string())]
        );
    }

    #[test]
    fn opencode_session_and_run_use_worktree_directory() {
        let worktree_dir = PathBuf::from("/repo/worktrees/aa-3881fda0");
        let command = build_opencode_run_command(
            &test_opencode_config(),
            "aa-3881fda0",
            "ses_123",
            &worktree_dir,
        );
        let dir_index = command.args.iter().position(|arg| arg == "--dir").unwrap();

        assert_eq!(
            opencode_directory(&worktree_dir),
            worktree_dir.display().to_string()
        );
        assert_eq!(
            command.args[dir_index + 1],
            worktree_dir.display().to_string()
        );
    }

    fn test_opencode_config() -> OpencodeRunConfig {
        OpencodeRunConfig::for_test()
    }
}
