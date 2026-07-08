use std::io;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, ExitStatus};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use super::claude::{build_claude_run_command, claude_run_config_from_env, spawn_claude_attached};
use super::codex::{codex_run_config_from_env, spawn_codex_app_server, TurnStatus};
use super::opencode::{
    build_opencode_run_command, create_opencode_session, opencode_run_config_from_env,
    spawn_opencode_attached, OpencodeRunConfig,
};
use crate::agent::{
    agent_report_json, load_agent_report, print_agent_report_human, read_agent_record,
    write_agent_record, AgentMetadata, AgentReport, AgentSystem,
};
use crate::cli::OutputFormat;
use crate::git::{commit_paths, create_worktree, remove_worktree};
use uuid::Uuid;

fn print_run_agent_report(
    output_format: &OutputFormat,
    header: &str,
    report: &AgentReport,
    commit: &str,
) {
    match output_format {
        OutputFormat::Json => {
            let mut value = agent_report_json(report);
            value["commit"] = serde_json::json!(commit);
            println!("{value}");
        }
        OutputFormat::HumanReadable => {
            print_agent_report_human(header, report);
            println!("Commit: {commit}");
        }
    }
}

/// Map a finished process's exit status to a CLI exit code so `waap agent run`
/// exits with the same code as the system it ran. Processes terminated by a
/// signal have no exit code; report a generic failure for those.
fn exit_code_from_status(status: ExitStatus) -> ExitCode {
    ExitCode::from(status.code().unwrap_or(1) as u8)
}

pub(crate) fn run_agent(
    waap_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
    system: &AgentSystem,
) -> io::Result<ExitCode> {
    match system {
        AgentSystem::Opencode => run_agent_opencode(waap_root, output_format, agent_id),
        AgentSystem::Claude => run_agent_claude(waap_root, output_format, agent_id),
        AgentSystem::Codex => run_agent_codex(waap_root, output_format, agent_id),
    }
}

fn agent_worktree_dir(agent_id: &str) -> PathBuf {
    Path::new("worktrees").join(agent_id)
}

/// Own the agent worktree lifecycle around a system run.
///
/// `prepare` runs first, *before* the worktree is cut. It commits the agent's `running` status to
/// `main` so the worktree branch is created from that commit and carries it, keeping history linear
/// (see the worktree-base reordering in the ticket). A fresh worktree is then prepared before `run`
/// is invoked and is always removed afterwards, even when `run` returns an error or the system exits
/// non-zero. The worktree path is passed to `run` so the selected system is launched inside it. The
/// run result is propagated only after cleanup so a failing run cannot leave a stale worktree behind.
///
/// Generic over the run closure's outcome `R`: opencode/claude return an `ExitStatus`, while codex
/// returns a `TurnStatus` (its completion is derived from the turn status, not a process exit code).
fn run_in_agent_worktree<P, F, R>(
    waap_root: &Path,
    agent_id: &str,
    prepare: P,
    run: F,
) -> io::Result<R>
where
    P: FnOnce() -> io::Result<()>,
    F: FnOnce(&Path) -> io::Result<R>,
{
    prepare()?;
    let relative_path = agent_worktree_dir(agent_id);
    let worktree = create_worktree(waap_root, agent_id, &relative_path)?;
    let run_result = run(&worktree);
    let cleanup_result = remove_worktree(waap_root, &relative_path);
    let outcome = run_result?;
    cleanup_result?;
    Ok(outcome)
}

/// Create and run an OpenCode session inside the canonical agent worktree.
///
/// OpenCode persists the directory supplied at session creation and gives it precedence over the
/// later run request's `--dir`. Setting the config root once, before both operations, keeps the
/// persisted session directory and run command aligned. Session creation remains inside the
/// worktree lifecycle so every subsequent error still triggers cleanup.
fn run_opencode_in_agent_worktree<P, S, U, L>(
    waap_root: &Path,
    agent_id: &str,
    mut config: OpencodeRunConfig,
    prepare: P,
    create_session: S,
    update_session: U,
    launch: L,
) -> io::Result<ExitStatus>
where
    P: FnOnce() -> io::Result<()>,
    S: FnOnce(&OpencodeRunConfig) -> io::Result<String>,
    U: FnOnce(&str) -> io::Result<()>,
    L: FnOnce(&OpencodeRunConfig, &str) -> io::Result<ExitStatus>,
{
    run_in_agent_worktree(waap_root, agent_id, prepare, |worktree| {
        config.waap_root = worktree.to_path_buf();
        let session_id = create_session(&config)?;
        update_session(&session_id)?;
        launch(&config, &session_id)
    })
}

fn run_agent_opencode(
    waap_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
) -> io::Result<ExitCode> {
    let config = opencode_run_config_from_env(waap_root)?;

    let (mut metadata, body) = read_agent_record(waap_root, agent_id)?;
    metadata.system = Some(AgentSystem::Opencode);

    let status = run_opencode_in_agent_worktree(
        waap_root,
        agent_id,
        config,
        || mark_running(waap_root, output_format, agent_id, &mut metadata, &body),
        create_opencode_session,
        |session_id| {
            update_agent_session(
                waap_root,
                output_format,
                agent_id,
                session_id,
                AgentSystem::Opencode,
            )
        },
        |config, session_id| {
            let command = build_opencode_run_command(config, agent_id, session_id);
            let mut child = spawn_opencode_attached(&command)?;
            child.wait()
        },
    )?;
    finalize_agent_run(waap_root, output_format, agent_id, status)
}

fn run_agent_claude(
    waap_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
) -> io::Result<ExitCode> {
    let mut config = claude_run_config_from_env(waap_root)?;
    let session_id = Uuid::new_v4().to_string();

    let (mut metadata, body) = read_agent_record(waap_root, agent_id)?;
    metadata.session_id = Some(session_id.clone());
    metadata.system = Some(AgentSystem::Claude);

    let status = run_in_agent_worktree(
        waap_root,
        agent_id,
        || mark_running(waap_root, output_format, agent_id, &mut metadata, &body),
        |worktree| {
            // Launch claude in the prepared worktree rather than the repository root.
            config.waap_root = worktree.to_path_buf();
            let command = build_claude_run_command(&config, agent_id, &session_id);
            let mut child = spawn_claude_attached(&command)?;
            child.wait()
        },
    )?;
    finalize_agent_run(waap_root, output_format, agent_id, status)
}

/// Drive a `codex` run via the `codex app-server --stdio` JSON-RPC client (see
/// /specs/codex-agent-system.md §3). Structurally mirrors `run_agent_opencode`, but codex's
/// `session_id` (its authentic `ThreadId`) is unknown until `thread/start` returns inside the
/// worktree, so it is persisted and committed mid-run by `update_agent_session` rather than ahead of
/// the worktree. Completion is derived from the turn status, not a process exit code.
fn run_agent_codex(
    waap_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
) -> io::Result<ExitCode> {
    let mut config = codex_run_config_from_env(waap_root)?;

    let (mut metadata, body) = read_agent_record(waap_root, agent_id)?;
    metadata.system = Some(AgentSystem::Codex);
    // session_id (the ThreadId) is unknown until thread/start returns inside the worktree.

    // Install a SIGTERM handler that flips this flag; `waap agent stop` signals this process (R) and
    // `pump_until_turn_completed` observes the flag to issue a graceful `turn/interrupt` before
    // unwinding (see /specs/codex-agent-system.md §5). The interrupted turn yields a non-`Completed`
    // status, so `finalize_codex_run` leaves the agent `running` and does not overwrite the `aborted`
    // status `stop` wrote to the record.
    let interrupt = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&interrupt))
        .map_err(|error| io::Error::other(format!("failed to install SIGTERM handler: {error}")))?;

    let status = run_in_agent_worktree(
        waap_root,
        agent_id,
        || mark_running(waap_root, output_format, agent_id, &mut metadata, &body),
        |worktree| {
            // Spawn the app-server in the prepared worktree and pass that cwd to thread/start so the
            // model's tools operate inside the worktree.
            config.waap_root = worktree.to_path_buf();
            let mut client = spawn_codex_app_server(&config)?;
            client.initialize()?;
            let thread_id = client.thread_start(worktree)?;

            // Persist the authentic ThreadId as session_id, then commit (one extra commit on `main`).
            update_agent_session(
                waap_root,
                output_format,
                agent_id,
                &thread_id,
                AgentSystem::Codex,
            )?;

            let prompt = format!(
                "Complete when instructions in /.waap/agents/{agent_id}/agent.md are satisfied"
            );
            let turn_id = client.turn_start(&thread_id, &prompt)?;
            client.pump_until_turn_completed(&thread_id, &turn_id, &interrupt)
        },
    )?;
    finalize_codex_run(waap_root, output_format, agent_id, status)
}

/// Mark the agent as running, commit the state change to `main`, and report it.
///
/// This runs as the worktree `prepare` step, *before* the worktree is cut, so the worktree branch
/// descends from the `running` commit (keeping history linear). The commit always lands on `main`
/// (`waap_root`) rather than the worktree branch so `waap agent list --status running` sees it.
/// System-created session ids are added to `main` after the worktree is available.
fn mark_running(
    waap_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
    metadata: &mut AgentMetadata,
    body: &str,
) -> io::Result<()> {
    metadata.status = "running".to_string();

    // Read the record on `main` immediately before deciding so a concurrent merge that already set
    // `running` is observed; an already-`running` record is a no-op write+commit, so skip it (still
    // report the agent state).
    let (current, _) = read_agent_record(waap_root, agent_id)?;
    if current.status == "running" {
        let report = load_agent_report(waap_root, agent_id)?;
        print_run_agent_report(output_format, "Running agent", &report, "");
        return Ok(());
    }

    write_agent_record(waap_root, agent_id, metadata, body)?;

    let report = load_agent_report(waap_root, agent_id)?;
    let commit = commit_paths(
        waap_root,
        &[report.path.as_path()],
        &format!("waap agent run {agent_id}"),
    )?;
    print_run_agent_report(output_format, "Running agent", &report, &commit);
    Ok(())
}

/// Persist a system-created session id on `main` and report it.
///
/// OpenCode and Codex only return authentic session ids after their sessions are created inside the
/// worktree. This adds a commit on `main` before either system starts agent work, keeping
/// `agent list` and `agent stop` connected to the live session.
fn update_agent_session(
    waap_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
    session_id: &str,
    system: AgentSystem,
) -> io::Result<()> {
    let (mut metadata, body) = read_agent_record(waap_root, agent_id)?;
    // Read from `main` immediately before deciding; if the session id and system already match the
    // target values the write+commit is a no-op, so skip it (still report the agent state).
    let header = format!("{} session", system.as_str());
    if metadata.session_id.as_deref() == Some(session_id)
        && metadata.system.as_ref() == Some(&system)
    {
        let report = load_agent_report(waap_root, agent_id)?;
        print_run_agent_report(output_format, &header, &report, "");
        return Ok(());
    }
    metadata.session_id = Some(session_id.to_string());
    metadata.system = Some(system.clone());
    write_agent_record(waap_root, agent_id, &metadata, &body)?;

    let report = load_agent_report(waap_root, agent_id)?;
    let commit = commit_paths(
        waap_root,
        &[report.path.as_path()],
        &format!("waap agent {} session {agent_id}", system.as_str()),
    )?;
    print_run_agent_report(output_format, &header, &report, &commit);
    Ok(())
}

/// After a successful system run, mark the agent `completed` and commit that state to `main`.
///
/// `waap agent run` derives the terminal status from the system process so completion no longer
/// depends on the agent self-reporting (which was unreliable). By the time the process exits the
/// agent has already merged its branch into `main`, so the agent record on `main` still carries the
/// `running` status. Re-read the record from `main`, flip the status to `completed`, and commit it on
/// top of the merged work so the completion lands cleanly and history stays linear.
///
/// Only a zero exit reaches here. A non-zero exit deliberately leaves the agent `running` so the
/// failure stays visible (see `run_agent_*`). This sets only the AGENT status; the ticket status is
/// the agent's responsibility and is never touched here.
fn mark_completed(
    waap_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
) -> io::Result<()> {
    let (mut metadata, body) = read_agent_record(waap_root, agent_id)?;
    // Read from `main` immediately before deciding so a concurrent merge that already set `completed`
    // (e.g. the codex agent self-marked it) is observed; an already-`completed` record is a no-op
    // write+commit, so skip it (still report the agent state) and avoid a redundant commit.
    if metadata.status == "completed" {
        let report = load_agent_report(waap_root, agent_id)?;
        print_run_agent_report(output_format, "Completed agent", &report, "");
        return Ok(());
    }
    metadata.status = "completed".to_string();
    write_agent_record(waap_root, agent_id, &metadata, &body)?;

    let report = load_agent_report(waap_root, agent_id)?;
    let commit = commit_paths(
        waap_root,
        &[report.path.as_path()],
        &format!("waap agent completed {agent_id}"),
    )?;
    print_run_agent_report(output_format, "Completed agent", &report, &commit);
    Ok(())
}

/// Derive the agent's terminal status from the finished system process and return the CLI exit code.
///
/// On a zero exit the agent is marked `completed` and that state is committed to `main`. A non-zero
/// exit is left `running` so the failure stays visible. The CLI exit code always mirrors the system.
fn finalize_agent_run(
    waap_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
    status: ExitStatus,
) -> io::Result<ExitCode> {
    finalize_run(waap_root, output_format, agent_id, status.success())?;
    Ok(exit_code_from_status(status))
}

/// Derive the agent's terminal status from a finished codex turn and return the CLI exit code.
///
/// codex completion is keyed on the `turn/completed` status rather than a process exit code (see
/// /specs/codex-agent-system.md §3 "Completion"): only `TurnStatus::Completed` is a success — the
/// agent is marked `completed`, committed to `main`, and exit 0 is returned. `Failed`/`Interrupted`/
/// `InProgress` leave the agent `running` so the failure (or graceful interrupt) stays visible and a
/// non-zero `ExitCode` is returned. Shares the mark/commit core with `finalize_agent_run`.
fn finalize_codex_run(
    waap_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
    status: TurnStatus,
) -> io::Result<ExitCode> {
    let success = status.is_success();
    finalize_run(waap_root, output_format, agent_id, success)?;
    Ok(if success {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// Shared completion core for `finalize_agent_run`/`finalize_codex_run`: on success mark the agent
/// `completed` and commit it to `main`; on failure do nothing so the agent stays `running` and the
/// failure remains visible. Never touches the ticket status.
fn finalize_run(
    waap_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
    success: bool,
) -> io::Result<()> {
    if success {
        mark_completed(waap_root, output_format, agent_id)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::fs;
    use std::os::unix::process::ExitStatusExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, ExitStatus};

    use serde_json::json;
    use tempfile::tempdir;

    use super::{
        agent_worktree_dir, finalize_agent_run, finalize_codex_run, mark_running,
        run_in_agent_worktree, run_opencode_in_agent_worktree, update_agent_session,
    };
    use crate::agent::codex::TurnStatus;
    use crate::agent::opencode::OpencodeRunConfig;
    use crate::agent::stop::stop_agents;
    use crate::agent::{
        agent_report_json, read_agent_record, AgentMetadata, AgentReport, AgentSystem,
    };
    use crate::cli::OutputFormat;
    use crate::git::{create_worktree, remove_worktree};
    use crate::test_git::{init_repo_with_commit, run as git};

    #[test]
    fn run_in_agent_worktree_launches_in_worktree_and_cleans_up_on_success() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());

        let mut launch_dir: Option<PathBuf> = None;
        let status = run_in_agent_worktree(
            dir.path(),
            "aa-00000001",
            || Ok(()),
            |worktree| {
                // The system is launched in the prepared worktree, which exists during the run.
                assert!(worktree.is_dir());
                launch_dir = Some(worktree.to_path_buf());
                Ok(ExitStatus::from_raw(0))
            },
        )
        .unwrap();

        assert_eq!(status.code(), Some(0));
        // The launch directory is the prepared worktree for this agent.
        assert!(launch_dir.unwrap().ends_with("worktrees/aa-00000001"));
        // The worktree is cleaned up after a successful run.
        assert!(!dir.path().join("worktrees/aa-00000001").exists());
    }

    #[test]
    fn run_in_agent_worktree_cleans_up_after_nonzero_exit() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());

        let status = run_in_agent_worktree(
            dir.path(),
            "aa-00000001",
            || Ok(()),
            |worktree| {
                assert!(worktree.is_dir());
                Ok(ExitStatus::from_raw(7 << 8))
            },
        )
        .unwrap();

        assert_eq!(status.code(), Some(7));
        assert!(!dir.path().join("worktrees/aa-00000001").exists());
    }

    #[test]
    fn run_in_agent_worktree_cleans_up_after_run_error() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());

        let error = run_in_agent_worktree(
            dir.path(),
            "aa-00000001",
            || Ok(()),
            |worktree| {
                // Simulate a failure mid-run (e.g. the system process could not be launched).
                assert!(worktree.is_dir());
                Err::<ExitStatus, _>(std::io::Error::other("boom"))
            },
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "boom");
        // Cleanup still runs when the run fails.
        assert!(!dir.path().join("worktrees/aa-00000001").exists());
    }

    #[test]
    fn opencode_session_and_run_use_canonical_worktree() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let session_directory = RefCell::new(None);
        let run_directory = RefCell::new(None);
        let effective_cwd = RefCell::new(None);

        let status = run_opencode_in_agent_worktree(
            dir.path(),
            "aa-00000001",
            OpencodeRunConfig::for_test(dir.path().to_path_buf()),
            || Ok(()),
            |config| {
                session_directory.replace(Some(config.waap_root.clone()));
                Ok("ses_worktree".to_string())
            },
            |_| Ok(()),
            |config, session_id| {
                assert_eq!(session_id, "ses_worktree");
                run_directory.replace(Some(config.waap_root.clone()));

                // Model OpenCode's routing rule: the persisted session directory wins over the
                // request's `--dir`. A pwd-equivalent command must still resolve to the worktree.
                let persisted = session_directory.borrow().clone().unwrap();
                let output = Command::new("pwd").current_dir(persisted).output().unwrap();
                assert!(output.status.success());
                effective_cwd.replace(Some(
                    PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
                        .canonicalize()
                        .unwrap(),
                ));
                Ok(ExitStatus::from_raw(0))
            },
        )
        .unwrap();

        let session_directory = session_directory.into_inner().unwrap();
        assert_eq!(status.code(), Some(0));
        assert_eq!(run_directory.into_inner().unwrap(), session_directory);
        assert_eq!(effective_cwd.into_inner().unwrap(), session_directory);
        assert!(session_directory.ends_with("worktrees/aa-00000001"));
        assert_ne!(session_directory, dir.path().canonicalize().unwrap());
        assert!(!dir.path().join("worktrees/aa-00000001").exists());
    }

    #[test]
    fn opencode_worktree_is_cleaned_up_after_session_creation_error() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let launch_called = Cell::new(false);

        let error = run_opencode_in_agent_worktree(
            dir.path(),
            "aa-00000001",
            OpencodeRunConfig::for_test(dir.path().to_path_buf()),
            || Ok(()),
            |_| Err(std::io::Error::other("session failed")),
            |_| Ok(()),
            |_, _| {
                launch_called.set(true);
                Ok(ExitStatus::from_raw(0))
            },
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "session failed");
        assert!(!launch_called.get());
        assert!(!dir.path().join("worktrees/aa-00000001").exists());
    }

    #[test]
    fn opencode_worktree_is_cleaned_up_after_launch_error() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());

        let error = run_opencode_in_agent_worktree(
            dir.path(),
            "aa-00000001",
            OpencodeRunConfig::for_test(dir.path().to_path_buf()),
            || Ok(()),
            |_| Ok("ses_worktree".to_string()),
            |_| Ok(()),
            |_, _| Err(std::io::Error::other("launch failed")),
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "launch failed");
        assert!(!dir.path().join("worktrees/aa-00000001").exists());
    }

    #[test]
    fn opencode_worktree_is_cleaned_up_after_nonzero_exit() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());

        let status = run_opencode_in_agent_worktree(
            dir.path(),
            "aa-00000001",
            OpencodeRunConfig::for_test(dir.path().to_path_buf()),
            || Ok(()),
            |_| Ok("ses_worktree".to_string()),
            |_| Ok(()),
            |_, _| Ok(ExitStatus::from_raw(7 << 8)),
        )
        .unwrap();

        assert_eq!(status.code(), Some(7));
        assert!(!dir.path().join("worktrees/aa-00000001").exists());
    }

    #[test]
    fn run_in_agent_worktree_cuts_branch_from_prepare_commit() {
        // The `prepare` step commits the run-status to `main` before the worktree is cut, so the
        // worktree branch must descend from that commit and carry it (acceptance criteria 1 & 3).
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let seed = git(dir.path(), &["rev-parse", "HEAD"]);

        let mut head_at_run = None;
        run_in_agent_worktree(
            dir.path(),
            "aa-00000001",
            || {
                // Simulate `mark_running` committing the run-status to `main`.
                fs::write(dir.path().join("running.txt"), "running\n").unwrap();
                git(dir.path(), &["add", "running.txt"]);
                git(
                    dir.path(),
                    &["commit", "-q", "-m", "waap agent run aa-00000001"],
                );
                Ok(())
            },
            |worktree| {
                // The worktree branch was cut after the run-status commit, so it contains the file.
                assert!(worktree.join("running.txt").exists());
                head_at_run = Some(git(worktree, &["rev-parse", "HEAD"]));
                Ok(ExitStatus::from_raw(0))
            },
        )
        .unwrap();

        let run_commit = git(dir.path(), &["rev-parse", "HEAD"]);
        // The run-status commit descends from the seed and the worktree branch was cut from it.
        assert_ne!(run_commit, seed);
        assert_eq!(head_at_run.unwrap(), run_commit);
        // History up to the run-status commit is linear: each commit has at most one parent.
        let parents = git(dir.path(), &["rev-list", "--merges", "HEAD"]);
        assert!(parents.is_empty(), "unexpected merge commits: {parents}");
    }

    #[test]
    fn agent_branch_rebase_and_ff_merge_keeps_main_linear() {
        // End-to-end of the ordering + the agent's rebase/`--ff-only` merge step: even when `main`
        // advances during the run, history stays linear with no merge bubble (acceptance criteria
        // 1, 4 & 6).
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());

        // The run commits the run-status to `main`, then cuts the worktree branch from it.
        fs::write(dir.path().join("running.txt"), "running\n").unwrap();
        git(dir.path(), &["add", "running.txt"]);
        git(
            dir.path(),
            &["commit", "-q", "-m", "waap agent run aa-00000001"],
        );
        let relative_path = agent_worktree_dir("aa-00000001");
        let worktree = create_worktree(dir.path(), "aa-00000001", &relative_path).unwrap();

        // The agent commits its work on its branch.
        fs::write(worktree.join("feature.txt"), "feature\n").unwrap();
        git(&worktree, &["add", "feature.txt"]);
        git(&worktree, &["commit", "-q", "-m", "feature aa-00000001"]);

        // Meanwhile another agent advances `main`.
        fs::write(dir.path().join("other.txt"), "other\n").unwrap();
        git(dir.path(), &["add", "other.txt"]);
        git(dir.path(), &["commit", "-q", "-m", "other agent"]);

        // The agent rebases onto the current `main` and fast-forward merges back.
        git(&worktree, &["rebase", "main"]);
        git(dir.path(), &["merge", "--ff-only", "aa-00000001"]);

        // No merge commits: `git log --graph` would show a straight line.
        let merges = git(dir.path(), &["rev-list", "--merges", "HEAD"]);
        assert!(merges.is_empty(), "unexpected merge commits: {merges}");
        // Both the other agent's work and this agent's work are present on the linear `main`.
        assert!(dir.path().join("other.txt").exists());
        assert!(dir.path().join("feature.txt").exists());

        remove_worktree(dir.path(), &relative_path).unwrap();
    }

    #[test]
    fn run_report_json_includes_running_status_and_session_id() {
        let report = AgentReport {
            agent_id: "aa-3881fda0".to_string(),
            path: std::path::PathBuf::from(".waap/agents/aa-3881fda0/agent.md"),
            metadata: AgentMetadata {
                name: None,
                creation_date: "2026-06-18T15:00:34Z".to_string(),
                status: "running".to_string(),
                session_id: Some("ses_abc123".to_string()),
                system: None,
            },
            file_size: 512,
        };

        let json = agent_report_json(&report);

        assert_eq!(json["metadata"]["status"], "running");
        assert_eq!(json["metadata"]["session_id"], "ses_abc123");
        assert_eq!(
            json,
            json!({
                "agent_id": "aa-3881fda0",
                "path": ".waap/agents/aa-3881fda0/agent.md",
                "metadata": {
                    "name": null,
                    "creation_date": "2026-06-18T15:00:34Z",
                    "status": "running",
                    "session_id": "ses_abc123",
                },
                "file_size": 512,
            })
        );
    }

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    /// Seed a committed agent record on `main` with the given status so `finalize_agent_run` has a
    /// record to read and update.
    fn seed_agent_record(root: &Path, agent_id: &str, status: &str) -> PathBuf {
        let path = root.join(format!(".waap/agents/{agent_id}/agent.md"));
        write_file(
            &path,
            &format!(
                "+++\ncreation_date = 2026-06-18T15:00:34Z\nstatus = \"{status}\"\n+++\n\n# Purpose\nDo work\n"
            ),
        );
        git(root, &["add", "-A"]);
        git(root, &["commit", "-q", "-m", "seed agent"]);
        path
    }

    #[test]
    fn opencode_session_is_visible_and_stoppable_during_run() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "ready");
        let (mut metadata, body) = read_agent_record(dir.path(), agent_id).unwrap();
        metadata.system = Some(AgentSystem::Opencode);
        let abort_called = Cell::new(false);

        run_opencode_in_agent_worktree(
            dir.path(),
            agent_id,
            OpencodeRunConfig::for_test(dir.path().to_path_buf()),
            || {
                mark_running(
                    dir.path(),
                    &OutputFormat::Json,
                    agent_id,
                    &mut metadata,
                    &body,
                )
            },
            |_| Ok("ses_live".to_string()),
            |session_id| {
                update_agent_session(
                    dir.path(),
                    &OutputFormat::Json,
                    agent_id,
                    session_id,
                    AgentSystem::Opencode,
                )
            },
            |config, session_id| {
                let (main_metadata, _) = read_agent_record(dir.path(), agent_id).unwrap();
                assert_eq!(main_metadata.status, "running");
                assert_eq!(main_metadata.system, Some(AgentSystem::Opencode));
                assert_eq!(main_metadata.session_id.as_deref(), Some(session_id));

                // The branch was cut from the running-state commit before the later session-id
                // commit on main, so it carries both the status and selected system.
                let (branch_metadata, _) = read_agent_record(&config.waap_root, agent_id).unwrap();
                assert_eq!(branch_metadata.status, "running");
                assert_eq!(branch_metadata.system, Some(AgentSystem::Opencode));

                let stopped = stop_agents(
                    dir.path(),
                    Some(agent_id),
                    |system, stopped_agent_id, stopped_session_id| {
                        assert_eq!(system, &AgentSystem::Opencode);
                        assert_eq!(stopped_agent_id, agent_id);
                        assert_eq!(stopped_session_id, session_id);
                        abort_called.set(true);
                        Ok(())
                    },
                )
                .unwrap();
                assert_eq!(stopped.len(), 1);
                Ok(ExitStatus::from_raw(0))
            },
        )
        .unwrap();

        assert!(abort_called.get());
        assert!(!dir.path().join("worktrees/aa-00000001").exists());
    }

    #[test]
    fn finalize_agent_run_marks_completed_on_zero_exit() {
        // Acceptance criteria 1 & 3: a successful run leaves the agent `completed` on `main` via a
        // commit that lands on top of the agent's merged work.
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        let path = seed_agent_record(dir.path(), agent_id, "running");

        finalize_agent_run(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            ExitStatus::from_raw(0),
        )
        .unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("status = \"completed\"\n"));
        // The completion is committed on `main` and history stays linear.
        let subject = git(dir.path(), &["log", "-1", "--format=%s"]);
        assert_eq!(subject, format!("waap agent completed {agent_id}"));
        let merges = git(dir.path(), &["rev-list", "--merges", "HEAD"]);
        assert!(merges.is_empty(), "unexpected merge commits: {merges}");
    }

    #[test]
    fn finalize_agent_run_skips_commit_when_already_completed() {
        // Acceptance criteria 1: an already-`completed` record is a no-op — no write and no new
        // commit — yet still completes successfully (e.g. the agent self-marked completed before the
        // process exit reached here).
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        let path = seed_agent_record(dir.path(), agent_id, "completed");
        let head_before = git(dir.path(), &["rev-parse", "HEAD"]);

        finalize_agent_run(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            ExitStatus::from_raw(0),
        )
        .unwrap();

        // Still completed, but no redundant commit was made.
        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("status = \"completed\"\n"));
        let head_after = git(dir.path(), &["rev-parse", "HEAD"]);
        assert_eq!(head_before, head_after);
    }

    #[test]
    fn finalize_agent_run_leaves_running_on_nonzero_exit() {
        // Acceptance criteria 2: a non-zero exit does not mark the agent completed and commits
        // nothing; the agent stays `running` so the failure is visible.
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        let path = seed_agent_record(dir.path(), agent_id, "running");
        let head_before = git(dir.path(), &["rev-parse", "HEAD"]);

        finalize_agent_run(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            ExitStatus::from_raw(7 << 8),
        )
        .unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("status = \"running\"\n"));
        // No commit was made for a failed run.
        let head_after = git(dir.path(), &["rev-parse", "HEAD"]);
        assert_eq!(head_before, head_after);
    }

    #[test]
    fn finalize_agent_run_does_not_change_ticket_status() {
        // Acceptance criteria 5: marking the agent completed never touches the ticket status.
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "running");

        let ticket_path = dir.path().join(".waap/tickets/tt-some-ticket/ticket.md");
        write_file(
            &ticket_path,
            "+++\ntitle = \"Some ticket\"\ncreation_date = 2026-06-18T15:00:34Z\nstatus = \"in-progress\"\n+++\n\n# Problem\nstuff\n",
        );
        let ticket_before = fs::read_to_string(&ticket_path).unwrap();

        finalize_agent_run(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            ExitStatus::from_raw(0),
        )
        .unwrap();

        let ticket_after = fs::read_to_string(&ticket_path).unwrap();
        assert_eq!(ticket_before, ticket_after);
    }

    #[test]
    fn run_agent_claude_updates_status_and_session_id_in_frontmatter() {
        use crate::agent::{load_agent_report, read_agent_record, AgentSystem};

        let dir = tempdir().unwrap();
        let agent_id = "aa-3881fda0";
        let path = dir.path().join(".waap/agents/aa-3881fda0/agent.md");
        write_file(
            &path,
            "+++\ncreation_date = 2026-06-18T15:00:34Z\nrole = \"developer\"\nstatus = \"ready\"\n+++\n\n# Purpose\nDo work\n",
        );

        // Simulate the run_agent_claude path: read once, mutate, write, then derive report.
        let (mut metadata, body) = read_agent_record(dir.path(), agent_id).unwrap();
        let session_id = "ses_test123".to_string();
        metadata.session_id = Some(session_id.clone());
        metadata.system = Some(AgentSystem::Claude);
        metadata.status = "running".to_string();
        crate::agent::write_agent_record(dir.path(), agent_id, &metadata, &body).unwrap();

        let report = load_agent_report(dir.path(), agent_id).unwrap();

        assert_eq!(report.metadata.status, "running");
        assert_eq!(report.metadata.session_id.as_deref(), Some("ses_test123"));

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("status = \"running\"\n"));
        assert!(contents.contains("session_id = \"ses_test123\"\n"));
        assert!(contents.contains("system = \"claude\"\n"));
        assert!(contents.contains("# Purpose\nDo work\n"));
    }

    #[test]
    fn run_in_agent_worktree_propagates_non_exit_status_outcome() {
        // The helper is generic over the run closure's outcome: codex returns a `TurnStatus`, not an
        // `ExitStatus`. Exercise a non-`ExitStatus` return and confirm cleanup still happens.
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());

        let outcome = run_in_agent_worktree(
            dir.path(),
            "aa-00000001",
            || Ok(()),
            |worktree| {
                assert!(worktree.is_dir());
                Ok(TurnStatus::Completed)
            },
        )
        .unwrap();

        assert_eq!(outcome, TurnStatus::Completed);
        assert!(!dir.path().join("worktrees/aa-00000001").exists());
    }

    #[test]
    fn finalize_codex_run_marks_completed_on_completed_status() {
        // Completion (§3): only `TurnStatus::Completed` marks the agent `completed` on `main` via a
        // commit, and returns success.
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        let path = seed_agent_record(dir.path(), agent_id, "running");

        finalize_codex_run(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            TurnStatus::Completed,
        )
        .unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("status = \"completed\"\n"));
        let subject = git(dir.path(), &["log", "-1", "--format=%s"]);
        assert_eq!(subject, format!("waap agent completed {agent_id}"));
        let merges = git(dir.path(), &["rev-list", "--merges", "HEAD"]);
        assert!(merges.is_empty(), "unexpected merge commits: {merges}");
    }

    #[test]
    fn finalize_codex_run_leaves_running_on_non_completed_status() {
        // A `Failed`/`Interrupted`/`InProgress` turn leaves the agent `running` and commits nothing,
        // so the failure (or graceful interrupt) stays visible.
        for status in [
            TurnStatus::Failed,
            TurnStatus::Interrupted,
            TurnStatus::InProgress,
        ] {
            let dir = tempdir().unwrap();
            init_repo_with_commit(dir.path());
            let agent_id = "aa-00000001";
            let path = seed_agent_record(dir.path(), agent_id, "running");
            let head_before = git(dir.path(), &["rev-parse", "HEAD"]);

            finalize_codex_run(dir.path(), &OutputFormat::Json, agent_id, status).unwrap();

            let contents = fs::read_to_string(&path).unwrap();
            assert!(
                contents.contains("status = \"running\"\n"),
                "status {status:?} should leave agent running"
            );
            let head_after = git(dir.path(), &["rev-parse", "HEAD"]);
            assert_eq!(head_before, head_after, "status {status:?} made a commit");
        }
    }

    #[test]
    fn finalize_codex_run_does_not_change_ticket_status() {
        // Marking the agent completed never touches the ticket status (mirrors the claude/opencode
        // finalize behavior).
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "running");

        let ticket_path = dir.path().join(".waap/tickets/tt-some-ticket/ticket.md");
        write_file(
            &ticket_path,
            "+++\ntitle = \"Some ticket\"\ncreation_date = 2026-06-18T15:00:34Z\nstatus = \"in-progress\"\n+++\n\n# Problem\nstuff\n",
        );
        let ticket_before = fs::read_to_string(&ticket_path).unwrap();

        finalize_codex_run(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            TurnStatus::Completed,
        )
        .unwrap();

        let ticket_after = fs::read_to_string(&ticket_path).unwrap();
        assert_eq!(ticket_before, ticket_after);
    }

    #[test]
    fn update_agent_session_writes_codex_session_id_and_system_and_commits() {
        // `update_agent_session` persists the authentic ThreadId as `session_id`, sets the system
        // to `codex`, and commits the record to `main` with a codex-session subject.
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        let path = seed_agent_record(dir.path(), agent_id, "running");

        update_agent_session(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            "th_abc123",
            AgentSystem::Codex,
        )
        .unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("status = \"running\"\n"));
        assert!(contents.contains("session_id = \"th_abc123\"\n"));
        assert!(contents.contains("system = \"codex\"\n"));

        let subject = git(dir.path(), &["log", "-1", "--format=%s"]);
        assert_eq!(subject, format!("waap agent codex session {agent_id}"));
        let merges = git(dir.path(), &["rev-list", "--merges", "HEAD"]);
        assert!(merges.is_empty(), "unexpected merge commits: {merges}");
    }

    #[test]
    fn update_agent_session_skips_commit_when_already_set() {
        // Acceptance criteria 2: when the session id and system already match the target, the
        // write+commit is skipped (no new commit) while the call still succeeds.
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "running");

        // First call writes the session id and system and commits once.
        update_agent_session(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            "th_abc123",
            AgentSystem::Codex,
        )
        .unwrap();
        let head_after_first = git(dir.path(), &["rev-parse", "HEAD"]);

        // Second call with the same target is a no-op: no new commit.
        update_agent_session(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            "th_abc123",
            AgentSystem::Codex,
        )
        .unwrap();
        let head_after_second = git(dir.path(), &["rev-parse", "HEAD"]);

        assert_eq!(head_after_first, head_after_second);
    }
}
