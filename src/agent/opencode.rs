use std::env;
use std::io;
use std::path::Path;
use std::process::{Child, Command as ProcessCommand, Stdio};

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
        let session_id = create_opencode_session(&self.config, context.worktree_dir)?;
        let command = build_opencode_run_command(
            &self.config,
            context.agent_id,
            &session_id,
            context.worktree_dir,
            context.prompt,
        );
        Ok(StartedRun {
            session_id,
            handle: Box::new(OpencodeRun {
                child: spawn_opencode_attached(&command)?,
            }),
        })
    }

    fn abort(&mut self, context: AbortContext<'_>) -> io::Result<()> {
        let worktree_dir = opencode_worktree_dir(context.waap_root, context.agent_id)?;
        abort_opencode_session(&self.config, context.session_id, &worktree_dir)
    }
}

struct OpencodeRun {
    child: Child,
}

impl RunHandle for OpencodeRun {
    fn wait(mut self: Box<Self>) -> io::Result<RunOutcome> {
        Ok(RunOutcome::from_exit_status(self.child.wait()?))
    }
}

fn opencode_worktree_dir(waap_root: &Path, agent_id: &str) -> io::Result<std::path::PathBuf> {
    Ok(waap_root.canonicalize()?.join("worktrees").join(agent_id))
}

#[derive(Debug, PartialEq, Eq)]
struct OpencodeRunConfig {
    server_url: String,
    username: String,
    password: String,
    model: String,
}

#[cfg(test)]
impl OpencodeRunConfig {
    fn for_test() -> Self {
        Self {
            server_url: "https://opencode.example".to_string(),
            username: "runner".to_string(),
            password: "secret".to_string(),
            model: "openai/gpt-5.5".to_string(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct OpencodeRunCommand {
    program: String,
    args: Vec<String>,
}

fn opencode_run_config_from_env() -> io::Result<OpencodeRunConfig> {
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

fn spawn_opencode_attached(command: &OpencodeRunCommand) -> io::Result<Child> {
    let mut process = ProcessCommand::new(&command.program);
    process
        .args(&command.args)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
}

fn create_opencode_session(config: &OpencodeRunConfig, worktree_dir: &Path) -> io::Result<String> {
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

fn abort_opencode_session(
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

fn build_opencode_run_command(
    config: &OpencodeRunConfig,
    _agent_id: &str,
    session_id: &str,
    worktree_dir: &Path,
    prompt: &str,
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
            prompt.to_string(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::{
        build_opencode_run_command, create_session_payload, opencode_directory_query, opencode_url,
        opencode_worktree_dir, spawn_opencode_attached, OpencodeRunCommand, OpencodeRunConfig,
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
    fn opencode_abort_derives_canonical_agent_worktree_directory() {
        let dir = tempfile::tempdir().unwrap();

        assert_eq!(
            opencode_worktree_dir(dir.path(), "aa-3881fda0").unwrap(),
            dir.path()
                .canonicalize()
                .unwrap()
                .join("worktrees/aa-3881fda0")
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

        let command = build_opencode_run_command(
            &config,
            "aa-3881fda0",
            "ses_123",
            &worktree_dir,
            "Complete when instructions in /.waap/agents/aa-3881fda0/agent.md are satisfied",
        );

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

    fn test_opencode_config() -> OpencodeRunConfig {
        OpencodeRunConfig::for_test()
    }
}
