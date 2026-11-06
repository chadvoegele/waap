use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use tokio::process::Command as ProcessCommand;
use tokio::sync::mpsc::Receiver;

use super::process::AgentProcess;
use uuid::Uuid;

use super::backend::{
    AbortContext, AbortOutcome, AgentSessionRun, RunOutcome, Runner, StartContext,
};
use super::commands::Command;

pub(super) struct ClaudeBackend {
    config: ClaudeRunConfig,
}

impl ClaudeBackend {
    pub(super) fn from_env() -> Self {
        Self {
            config: claude_run_config_from_env(),
        }
    }

    pub(super) async fn start(
        &mut self,
        context: StartContext<'_>,
        receiver: Receiver<Command>,
    ) -> io::Result<AgentSessionRun> {
        let session_id = Uuid::new_v4().to_string();
        let command = build_claude_run_command(
            &self.config,
            context.agent_id,
            &session_id,
            context.worktree_dir,
            context.prompt,
        );
        Ok(AgentSessionRun::new(
            session_id,
            Runner::Claude(ClaudeRunner {
                child: spawn_claude_attached(&command)?,
            }),
            receiver,
        ))
    }
    pub(super) async fn abort(&mut self, context: AbortContext<'_>) -> io::Result<AbortOutcome> {
        if let Some(session_id) = context.session_id {
            kill_claude_session(session_id).await?;
        }
        Ok(AbortOutcome::Stopped)
    }
}

pub(super) struct ClaudeRunner {
    child: AgentProcess,
}
impl ClaudeRunner {
    pub(super) async fn run(&mut self, receiver: Receiver<Command>) -> io::Result<RunOutcome> {
        super::unsupported_commands::await_with_unsupported_commands(
            async { Ok(RunOutcome::from_exit_status(self.child.wait().await?)) },
            receiver,
            "claude",
        )
        .await
    }
    pub(super) async fn cleanup(&mut self) -> io::Result<()> {
        self.child.terminate().await
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

async fn kill_claude_session(session_id: &str) -> io::Result<()> {
    let status = ProcessCommand::new("pkill")
        .arg("-TERM")
        .arg("-f")
        .arg(session_id)
        .status()
        .await?;
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
fn spawn_claude_attached(command: &ClaudeRunCommand) -> io::Result<AgentProcess> {
    let mut process = ProcessCommand::new(&command.program);
    process
        .args(&command.args)
        .current_dir(&command.working_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    AgentProcess::spawn(&mut process)
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
#[path = "claude_tests.rs"]
mod tests;
