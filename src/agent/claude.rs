use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcessCommand, ExitStatus, Stdio};

use uuid::Uuid;

use super::backend::{
    AbortContext, AgentSystemBackend, RunHandle, RunOutcome, StartContext, StartedRun,
};

pub(super) struct ClaudeBackend {
    config: ClaudeRunConfig,
}

impl ClaudeBackend {
    pub(super) fn from_env() -> Self {
        Self {
            config: claude_run_config_from_env(),
        }
    }
}

impl AgentSystemBackend for ClaudeBackend {
    fn start(&mut self, context: StartContext<'_>) -> io::Result<StartedRun> {
        let session_id = Uuid::new_v4().to_string();
        let command = build_claude_run_command(
            &self.config,
            context.agent_id,
            &session_id,
            context.worktree_dir,
            context.prompt,
        );
        Ok(StartedRun {
            session_id,
            handle: Box::new(ClaudeRun {
                child: spawn_claude_attached(&command)?,
            }),
        })
    }

    fn abort(&mut self, context: AbortContext<'_>) -> io::Result<()> {
        kill_claude_session(context.session_id)
    }
}

struct ClaudeRun {
    child: Child,
}

impl RunHandle for ClaudeRun {
    fn wait(mut self: Box<Self>) -> io::Result<RunOutcome> {
        Ok(RunOutcome::from_exit_status(self.child.wait()?))
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ClaudeRunConfig {
    model: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct ClaudeRunCommand {
    program: String,
    args: Vec<String>,
    working_dir: PathBuf,
}

fn claude_run_config_from_env() -> ClaudeRunConfig {
    ClaudeRunConfig {
        model: env::var("CLAUDE_MODEL")
            .ok()
            .filter(|model| !model.is_empty()),
    }
}

fn kill_claude_session(session_id: &str) -> io::Result<()> {
    let status = ProcessCommand::new("pkill")
        .arg("-TERM")
        .arg("-f")
        .arg(session_id)
        .status()?;
    map_pkill_status(status)
}

fn map_pkill_status(status: ExitStatus) -> io::Result<()> {
    match status.code() {
        // 0: a process was signalled. 1: no process matched (already exited).
        Some(0) | Some(1) => Ok(()),
        Some(code) => Err(io::Error::other(format!("pkill exited with status {code}"))),
        None => Err(io::Error::other("pkill terminated by signal")),
    }
}

/// Spawn Claude with output attached to this process and stdin disconnected.
fn spawn_claude_attached(command: &ClaudeRunCommand) -> io::Result<Child> {
    let mut process = ProcessCommand::new(&command.program);
    process
        .args(&command.args)
        .current_dir(&command.working_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
}

fn build_claude_run_command(
    config: &ClaudeRunConfig,
    _agent_id: &str,
    session_id: &str,
    worktree_dir: &Path,
    prompt: &str,
) -> ClaudeRunCommand {
    let mut args = vec![
        "-p".to_string(),
        "--session-id".to_string(),
        session_id.to_string(),
        "--output-format".to_string(),
        "json".to_string(),
        // allow git merge ff
        "--permission-mode".to_string(),
        "bypassPermissions".to_string(),
        // Disable the bash sandbox; its /dev/null dotfile mounts break the agent's `git worktree remove`.
        "--settings".to_string(),
        "{\"sandbox\":{\"enabled\":false}}".to_string(),
    ];
    if let Some(model) = &config.model {
        args.push("--model".to_string());
        args.push(model.clone());
    }
    args.push(prompt.to_string());

    ClaudeRunCommand {
        program: "claude".to_string(),
        args,
        working_dir: worktree_dir.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::ExitStatusExt;
    use std::path::PathBuf;

    use super::{
        build_claude_run_command, map_pkill_status, spawn_claude_attached, ClaudeRunCommand,
        ClaudeRunConfig,
    };

    #[test]
    fn spawn_claude_attached_returns_child_with_exit_code() {
        let command = ClaudeRunCommand {
            program: "sh".to_string(),
            args: vec!["-c".to_string(), "exit 5".to_string()],
            working_dir: std::env::temp_dir(),
        };

        let mut child = spawn_claude_attached(&command).unwrap();
        let status = child.wait().unwrap();

        assert_eq!(status.code(), Some(5));
    }

    #[test]
    fn spawn_claude_attached_connects_stdin_to_null() {
        let command = ClaudeRunCommand {
            program: "sh".to_string(),
            args: vec!["-c".to_string(), "read value; test $? -ne 0".to_string()],
            working_dir: std::env::temp_dir(),
        };

        let status = spawn_claude_attached(&command).unwrap().wait().unwrap();

        assert!(status.success());
    }

    #[test]
    fn claude_pkill_status_mapping_accepts_match_and_no_match() {
        for code in [0, 1] {
            assert!(map_pkill_status(std::process::ExitStatus::from_raw(code << 8)).is_ok());
        }
        assert_eq!(
            map_pkill_status(std::process::ExitStatus::from_raw(2 << 8))
                .unwrap_err()
                .to_string(),
            "pkill exited with status 2"
        );
        assert_eq!(
            map_pkill_status(std::process::ExitStatus::from_raw(9))
                .unwrap_err()
                .to_string(),
            "pkill terminated by signal"
        );
    }

    #[test]
    fn claude_run_command_matches_spec() {
        let config = test_claude_config(Some("opus"));

        let command = build_claude_run_command(
            &config,
            "aa-3881fda0",
            "11111111-2222-4333-8444-555555555555",
            PathBuf::from("/repo/with space").as_path(),
            "Complete when instructions in /.waap/agents/aa-3881fda0/agent.md are satisfied",
        );

        assert_eq!(
            command,
            ClaudeRunCommand {
                program: "claude".to_string(),
                args: vec![
                    "-p".to_string(),
                    "--session-id".to_string(),
                    "11111111-2222-4333-8444-555555555555".to_string(),
                    "--output-format".to_string(),
                    "json".to_string(),
                    "--permission-mode".to_string(),
                    "bypassPermissions".to_string(),
                    "--settings".to_string(),
                    "{\"sandbox\":{\"enabled\":false}}".to_string(),
                    "--model".to_string(),
                    "opus".to_string(),
                    "Complete when instructions in /.waap/agents/aa-3881fda0/agent.md are satisfied"
                        .to_string(),
                ],
                working_dir: PathBuf::from("/repo/with space"),
            }
        );
    }

    #[test]
    fn claude_run_command_omits_model_when_unset() {
        let config = test_claude_config(None);

        let command = build_claude_run_command(
            &config,
            "aa-3881fda0",
            "ses-uuid",
            PathBuf::from("/repo/with space").as_path(),
            "Complete when instructions in /.waap/agents/aa-3881fda0/agent.md are satisfied",
        );

        assert!(!command.args.iter().any(|arg| arg == "--model"));
        assert_eq!(
            command.args.last().map(String::as_str),
            Some("Complete when instructions in /.waap/agents/aa-3881fda0/agent.md are satisfied")
        );
    }

    fn test_claude_config(model: Option<&str>) -> ClaudeRunConfig {
        ClaudeRunConfig {
            model: model.map(str::to_string),
        }
    }
}
