use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, ExitStatus};

use crate::process::run_forwarding;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ClaudeRunConfig {
    pub(crate) model: Option<String>,
    pub(crate) waap_root: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ClaudeRunCommand {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    pub(crate) working_dir: PathBuf,
}

pub(crate) fn claude_run_config_from_env(waap_root: &Path) -> io::Result<ClaudeRunConfig> {
    Ok(ClaudeRunConfig {
        model: env::var("CLAUDE_MODEL")
            .ok()
            .filter(|model| !model.is_empty()),
        waap_root: waap_root.canonicalize()?,
    })
}

pub(crate) fn kill_claude_session(session_id: &str) -> io::Result<()> {
    let status = ProcessCommand::new("pkill")
        .arg("-TERM")
        .arg("-f")
        .arg(session_id)
        .status()?;
    match status.code() {
        // 0: a process was signalled. 1: no process matched (already exited).
        Some(0) | Some(1) => Ok(()),
        Some(code) => Err(io::Error::other(format!("pkill exited with status {code}"))),
        None => Err(io::Error::other("pkill terminated by signal")),
    }
}

/// Run the Claude system in the foreground, forwarding its stdout and stderr to
/// this process's stdout and stderr, and return its exit status. `on_started`
/// runs once the process has been launched.
pub(crate) fn run_claude_attached<F>(
    command: &ClaudeRunCommand,
    on_started: F,
) -> io::Result<ExitStatus>
where
    F: FnOnce() -> io::Result<()>,
{
    let mut process = ProcessCommand::new(&command.program);
    process
        .args(&command.args)
        .current_dir(&command.working_dir);
    run_forwarding(&mut process, on_started)
}

pub(crate) fn build_claude_run_command(
    config: &ClaudeRunConfig,
    agent_id: &str,
    session_id: &str,
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
    args.push(format!(
        "Complete when instructions in /.waap/agents/{agent_id}/agent.md are satisfied"
    ));

    ClaudeRunCommand {
        program: "claude".to_string(),
        args,
        working_dir: config.waap_root.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{build_claude_run_command, run_claude_attached, ClaudeRunCommand, ClaudeRunConfig};

    #[test]
    fn run_claude_attached_propagates_exit_code_and_marks_started() {
        let command = ClaudeRunCommand {
            program: "sh".to_string(),
            args: vec!["-c".to_string(), "exit 5".to_string()],
            working_dir: std::env::temp_dir(),
        };

        let mut started = false;
        let status = run_claude_attached(&command, || {
            started = true;
            Ok(())
        })
        .unwrap();

        assert!(started);
        assert_eq!(status.code(), Some(5));
    }

    #[test]
    fn claude_run_command_matches_spec() {
        let config = test_claude_config(Some("opus"));

        let command = build_claude_run_command(
            &config,
            "aa-3881fda0",
            "11111111-2222-4333-8444-555555555555",
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

        let command = build_claude_run_command(&config, "aa-3881fda0", "ses-uuid");

        assert!(!command.args.iter().any(|arg| arg == "--model"));
        assert_eq!(
            command.args.last().map(String::as_str),
            Some("Complete when instructions in /.waap/agents/aa-3881fda0/agent.md are satisfied")
        );
    }

    fn test_claude_config(model: Option<&str>) -> ClaudeRunConfig {
        ClaudeRunConfig {
            model: model.map(str::to_string),
            waap_root: PathBuf::from("/repo/with space"),
        }
    }
}
