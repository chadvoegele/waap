use std::io;
use std::path::Path;
use std::process::{ExitCode, ExitStatus};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::agent::{
    agent_report_json, load_agent_report, print_agent_report_human, read_agent_record,
    write_agent_record, AgentMetadata, AgentReport, AgentSystem,
};
use crate::claude::{build_claude_run_command, claude_run_config_from_env, run_claude_attached};
use crate::cli::OutputFormat;
use crate::codex::{codex_run_config_from_env, spawn_codex_app_server, TurnStatus};
use crate::git::{commit_paths, create_agent_worktree, remove_agent_worktree};
use crate::opencode::{
    build_opencode_run_command, create_opencode_session, opencode_run_config_from_env,
    run_opencode_attached,
};
use uuid::Uuid;

pub(crate) fn print_run_agent_report(
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
    repo_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
    system: &AgentSystem,
) -> io::Result<ExitCode> {
    match system {
        AgentSystem::Opencode => run_agent_opencode(repo_root, output_format, agent_id),
        AgentSystem::Claude => run_agent_claude(repo_root, output_format, agent_id),
        AgentSystem::Codex => run_agent_codex(repo_root, output_format, agent_id),
    }
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
    repo_root: &Path,
    agent_id: &str,
    prepare: P,
    run: F,
) -> io::Result<R>
where
    P: FnOnce() -> io::Result<()>,
    F: FnOnce(&Path) -> io::Result<R>,
{
    prepare()?;
    let worktree = create_agent_worktree(repo_root, agent_id)?;
    let run_result = run(&worktree);
    let cleanup_result = remove_agent_worktree(repo_root, agent_id);
    let outcome = run_result?;
    cleanup_result?;
    Ok(outcome)
}

fn run_agent_opencode(
    repo_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
) -> io::Result<ExitCode> {
    let mut config = opencode_run_config_from_env(repo_root)?;

    // Create the session before cutting the worktree so the session id can be committed with the
    // `running` status in a single commit on `main`, ahead of the worktree.
    let session_id = create_opencode_session(&config)?;
    let (mut metadata, body) = read_agent_record(repo_root, agent_id)?;
    metadata.session_id = Some(session_id.clone());
    metadata.system = Some(AgentSystem::Opencode);

    let status = run_in_agent_worktree(
        repo_root,
        agent_id,
        || mark_running(repo_root, output_format, agent_id, &mut metadata, &body),
        |worktree| {
            // Launch opencode against the prepared worktree rather than the repository root.
            config.repo_root = worktree.to_path_buf();
            let command = build_opencode_run_command(&config, agent_id, &session_id);
            run_opencode_attached(&command, || Ok(()))
        },
    )?;
    finalize_agent_run(repo_root, output_format, agent_id, status)
}

fn run_agent_claude(
    repo_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
) -> io::Result<ExitCode> {
    let mut config = claude_run_config_from_env(repo_root)?;
    let session_id = Uuid::new_v4().to_string();

    let (mut metadata, body) = read_agent_record(repo_root, agent_id)?;
    metadata.session_id = Some(session_id.clone());
    metadata.system = Some(AgentSystem::Claude);

    let status = run_in_agent_worktree(
        repo_root,
        agent_id,
        || mark_running(repo_root, output_format, agent_id, &mut metadata, &body),
        |worktree| {
            // Launch claude in the prepared worktree rather than the repository root.
            config.repo_root = worktree.to_path_buf();
            let command = build_claude_run_command(&config, agent_id, &session_id);
            run_claude_attached(&command, || Ok(()))
        },
    )?;
    finalize_agent_run(repo_root, output_format, agent_id, status)
}

/// Drive a `codex` run via the `codex app-server --stdio` JSON-RPC client (see
/// /specs/codex-agent-system.md §3). Structurally mirrors `run_agent_opencode`, but codex's
/// `session_id` (its authentic `ThreadId`) is unknown until `thread/start` returns inside the
/// worktree, so it is persisted and committed mid-run by `update_codex_session` rather than ahead of
/// the worktree. Completion is derived from the turn status, not a process exit code.
fn run_agent_codex(
    repo_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
) -> io::Result<ExitCode> {
    let mut config = codex_run_config_from_env(repo_root)?;

    let (mut metadata, body) = read_agent_record(repo_root, agent_id)?;
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
        repo_root,
        agent_id,
        || mark_running(repo_root, output_format, agent_id, &mut metadata, &body),
        |worktree| {
            // Spawn the app-server in the prepared worktree and pass that cwd to thread/start so the
            // model's tools operate inside the worktree.
            config.repo_root = worktree.to_path_buf();
            let mut client = spawn_codex_app_server(&config)?;
            client.initialize()?;
            let thread_id = client.thread_start(worktree)?;

            // Persist the authentic ThreadId as session_id, then commit (one extra commit on `main`).
            update_codex_session(repo_root, output_format, agent_id, &thread_id)?;

            let prompt = format!(
                "Complete when instructions in /.waap/agents/{agent_id}/agent.md are satisfied"
            );
            let turn_id = client.turn_start(&thread_id, &prompt)?;
            client.pump_until_turn_completed(&thread_id, &turn_id, &interrupt)
        },
    )?;
    finalize_codex_run(repo_root, output_format, agent_id, status)
}

/// Mark the agent as running, commit the state change to `main`, and report it.
///
/// This runs as the worktree `prepare` step, *before* the worktree is cut, so the worktree branch
/// descends from the `running` commit (keeping history linear). The commit always lands on `main`
/// (`repo_root`) rather than the worktree branch so `waap agent list --status running` and
/// `waap agent stop` see the running status and session id from the main worktree during the run.
fn mark_running(
    repo_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
    metadata: &mut AgentMetadata,
    body: &str,
) -> io::Result<()> {
    metadata.status = "running".to_string();
    write_agent_record(repo_root, agent_id, metadata, body)?;

    let report = load_agent_report(repo_root, agent_id)?;
    let commit = commit_paths(
        repo_root,
        &[report.path.as_path()],
        &format!("waap agent run {agent_id}"),
    )?;
    print_run_agent_report(output_format, "Running agent", &report, &commit);
    Ok(())
}

/// Persist codex's authentic `ThreadId` as the agent's `session_id`, commit it to `main`, and report
/// it. Unlike opencode/claude, codex's session id is only known after `thread/start` returns inside
/// the worktree, so this runs mid-run and adds one extra commit on `main` (mirrors `mark_running`'s
/// write+`commit_paths` pattern). `metadata.system` is set to `codex` so the record on `main` carries
/// the system alongside the session id.
fn update_codex_session(
    repo_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
    thread_id: &str,
) -> io::Result<()> {
    let (mut metadata, body) = read_agent_record(repo_root, agent_id)?;
    metadata.session_id = Some(thread_id.to_string());
    metadata.system = Some(AgentSystem::Codex);
    write_agent_record(repo_root, agent_id, &metadata, &body)?;

    let report = load_agent_report(repo_root, agent_id)?;
    let commit = commit_paths(
        repo_root,
        &[report.path.as_path()],
        &format!("waap agent codex session {agent_id}"),
    )?;
    print_run_agent_report(output_format, "Codex session", &report, &commit);
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
    repo_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
) -> io::Result<()> {
    let (mut metadata, body) = read_agent_record(repo_root, agent_id)?;
    metadata.status = "completed".to_string();
    write_agent_record(repo_root, agent_id, &metadata, &body)?;

    let report = load_agent_report(repo_root, agent_id)?;
    let commit = commit_paths(
        repo_root,
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
    repo_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
    status: ExitStatus,
) -> io::Result<ExitCode> {
    finalize_run(repo_root, output_format, agent_id, status.success())?;
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
    repo_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
    status: TurnStatus,
) -> io::Result<ExitCode> {
    let success = status.is_success();
    finalize_run(repo_root, output_format, agent_id, success)?;
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
    repo_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
    success: bool,
) -> io::Result<()> {
    if success {
        mark_completed(repo_root, output_format, agent_id)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::process::ExitStatusExt;
    use std::path::{Path, PathBuf};
    use std::process::ExitStatus;

    use serde_json::json;
    use tempfile::tempdir;

    use super::{
        finalize_agent_run, finalize_codex_run, run_in_agent_worktree, update_codex_session,
    };
    use crate::agent::{agent_report_json, AgentMetadata, AgentReport};
    use crate::cli::OutputFormat;
    use crate::codex::TurnStatus;
    use crate::git::{create_agent_worktree, remove_agent_worktree};

    fn git(root: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn init_repo_with_commit(root: &Path) {
        git(root, &["init", "-q"]);
        git(root, &["config", "user.name", "Test"]);
        git(root, &["config", "user.email", "test@example.com"]);
        fs::write(root.join("README.md"), "seed\n").unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-q", "-m", "seed"]);
    }

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
        let worktree = create_agent_worktree(dir.path(), "aa-00000001").unwrap();

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

        remove_agent_worktree(dir.path(), "aa-00000001").unwrap();
    }

    #[test]
    fn run_report_json_includes_running_status_and_session_id() {
        let report = AgentReport {
            agent_id: "aa-3881fda0".to_string(),
            path: std::path::PathBuf::from(".waap/agents/aa-3881fda0/agent.md"),
            metadata: AgentMetadata {
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
    fn update_codex_session_writes_session_id_and_system_and_commits() {
        // `update_codex_session` persists the authentic ThreadId as `session_id`, sets the system to
        // `codex`, and commits the record to `main` with a codex-session subject.
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        let path = seed_agent_record(dir.path(), agent_id, "running");

        update_codex_session(dir.path(), &OutputFormat::Json, agent_id, "th_abc123").unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("status = \"running\"\n"));
        assert!(contents.contains("session_id = \"th_abc123\"\n"));
        assert!(contents.contains("system = \"codex\"\n"));

        let subject = git(dir.path(), &["log", "-1", "--format=%s"]);
        assert_eq!(subject, format!("waap agent codex session {agent_id}"));
        let merges = git(dir.path(), &["rev-list", "--merges", "HEAD"]);
        assert!(merges.is_empty(), "unexpected merge commits: {merges}");
    }
}
