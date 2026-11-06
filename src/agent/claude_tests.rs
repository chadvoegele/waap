use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;

use super::{
    build_claude_run_command, map_pkill_status, spawn_claude_attached, ClaudeRunCommand,
    ClaudeRunConfig,
};

use crate::agent::backend::RunOutcome;
use crate::agent::commands::CommandDispatcher;
use crate::agent::commands::CommandError;
use futures_util::FutureExt;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;

fn pending_runner() -> (super::ClaudeRunner, tokio::process::ChildStdin) {
    let mut command = tokio::process::Command::new("sh");
    command
        .args(["-c", "read release; exit 7"])
        .stdin(Stdio::piped());
    let mut child = super::AgentProcess::spawn(&mut command).unwrap();
    let release = child.take_stdin().unwrap();
    (super::ClaudeRunner { child }, release)
}

#[tokio::test(flavor = "current_thread")]
async fn concrete_runner_rejects_steering_then_finishes_the_same_process_wait() {
    let (mut runner, mut release) = pending_runner();
    let (dispatcher, receiver) = CommandDispatcher::create_channel();
    let mut execution = Box::pin(runner.run(receiver));
    assert!(execution.as_mut().now_or_never().is_none());
    let ((), result) = tokio::join!(
        async {
            assert_eq!(
                dispatcher
                    .steer("pending", Instant::now() + Duration::from_secs(2))
                    .await,
                Err(CommandError::not_implemented("claude", "steering"))
            );
            release.write_all(b"finish\n").await.unwrap();
        },
        tokio::time::timeout(Duration::from_secs(2), &mut execution)
    );
    assert_eq!(
        result.unwrap().unwrap(),
        RunOutcome::Failed(std::process::ExitCode::from(7))
    );
    drop(execution);
    assert_eq!(runner.child.wait().await.unwrap().code(), Some(7));
    runner.cleanup().await.unwrap();
    assert!(matches!(
        dispatcher
            .steer("completed", Instant::now() + Duration::from_secs(2))
            .await,
        Err(CommandError::Rejected {
            code: "agent_exited",
            ..
        })
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn concrete_runner_completion_releases_queued_replies() {
    let (mut runner, mut release) = pending_runner();
    release.write_all(b"finish\n").await.unwrap();
    assert_eq!(runner.child.wait().await.unwrap().code(), Some(7));
    let (dispatcher, receiver) = CommandDispatcher::create_channel();
    let steering = dispatcher.steer("queued", Instant::now() + Duration::from_secs(30));
    tokio::pin!(steering);
    assert!(steering.as_mut().now_or_never().is_none());
    assert_eq!(
        runner.run(receiver).await.unwrap(),
        RunOutcome::Failed(std::process::ExitCode::from(7))
    );
    let result = tokio::time::timeout(Duration::from_millis(100), steering)
        .await
        .unwrap();
    assert!(
        matches!(result, Err(CommandError::AcceptanceUnknown { .. }))
            || result == Err(CommandError::not_implemented("claude", "steering"))
    );
    runner.cleanup().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn concrete_runner_cancellation_releases_queued_replies_before_or_after_polling() {
    for polled in [false, true] {
        let (mut runner, _release) = pending_runner();
        let (dispatcher, receiver) = CommandDispatcher::create_channel();
        let mut execution = Box::pin(runner.run(receiver));
        if polled {
            assert!(execution.as_mut().now_or_never().is_none());
        }
        let steering = dispatcher.steer("queued", Instant::now() + Duration::from_secs(30));
        tokio::pin!(steering);
        assert!(steering.as_mut().now_or_never().is_none());
        drop(execution);
        assert!(matches!(
            tokio::time::timeout(Duration::from_millis(100), steering)
                .await
                .unwrap(),
            Err(CommandError::AcceptanceUnknown { .. })
        ));
        assert!(matches!(
            dispatcher
                .steer("cancelled", Instant::now() + Duration::from_secs(2))
                .await,
            Err(CommandError::Rejected {
                code: "agent_exited",
                ..
            })
        ));
        runner.cleanup().await.unwrap();
        assert!(!runner.child.wait().await.unwrap().success());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn unsupported_steering_does_not_need_to_wait_for_the_process() {
    let command = ClaudeRunCommand {
        program: "sh".into(),
        args: vec!["-c".into(), "while :; do :; done".into()],
        working_dir: std::env::temp_dir(),
    };
    let mut runner = super::super::backend::Runner::Claude(super::ClaudeRunner {
        child: spawn_claude_attached(&command).unwrap(),
    });
    crate::agent::control::tests::assert_backend_socket_rejection(&mut runner, "claude").await;
    runner.cleanup().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_claude_attached_returns_child_with_exit_code() {
    let command = ClaudeRunCommand {
        program: "sh".to_string(),
        args: vec!["-c".to_string(), "exit 5".to_string()],
        working_dir: std::env::temp_dir(),
    };

    let mut child = spawn_claude_attached(&command).unwrap();
    let status = child.wait().await.unwrap();

    assert_eq!(status.code(), Some(5));
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_claude_attached_connects_stdin_to_null() {
    let command = ClaudeRunCommand {
        program: "sh".to_string(),
        args: vec!["-c".to_string(), "read value; test $? -ne 0".to_string()],
        working_dir: std::env::temp_dir(),
    };

    let status = spawn_claude_attached(&command)
        .unwrap()
        .wait()
        .await
        .unwrap();

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
        "Complete when instructions in /agents/aa-3881fda0/agent.md are satisfied",
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
                "Complete when instructions in /agents/aa-3881fda0/agent.md are satisfied"
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
        "Complete when instructions in /agents/aa-3881fda0/agent.md are satisfied",
    );

    assert!(!command.args.iter().any(|arg| arg == "--model"));
    assert_eq!(
        command.args.last().map(String::as_str),
        Some("Complete when instructions in /agents/aa-3881fda0/agent.md are satisfied")
    );
}

fn test_claude_config(model: Option<&str>) -> ClaudeRunConfig {
    ClaudeRunConfig {
        model: model.map(str::to_string),
    }
}
