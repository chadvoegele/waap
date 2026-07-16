use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use super::backend::{AgentSystemBackend, RunOutcome, StartContext};
use crate::agent::{
    agent_report_json, load_agent_report, print_agent_report_human, read_agent_record,
    transition_agent_status, write_agent_record, AgentMetadata, AgentReport, AgentStatus,
    AgentSystem,
};
use crate::cli::OutputFormat;
use crate::git::{commit_paths, create_worktree, remove_worktree};

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

pub(crate) fn run_agent(
    waap_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
    system: &AgentSystem,
) -> io::Result<ExitCode> {
    require_ready_agent(waap_root, agent_id)?;
    let mut backend = system.backend()?;
    run_agent_with_backend(waap_root, output_format, agent_id, system, backend.as_mut())
}

fn run_agent_with_backend(
    waap_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
    system: &AgentSystem,
    backend: &mut dyn AgentSystemBackend,
) -> io::Result<ExitCode> {
    let (mut metadata, body) = read_agent_record(waap_root, agent_id)?;
    metadata.system = Some(system.clone());

    if let Err(error) = mark_running(waap_root, output_format, agent_id, &mut metadata, &body) {
        let is_running = read_agent_record(waap_root, agent_id)
            .map(|(metadata, _)| metadata.status == AgentStatus::Running.as_str())
            .unwrap_or(false);
        return Err(if is_running {
            persist_failed_after_error(waap_root, output_format, agent_id, error)
        } else {
            error
        });
    }
    let result = run_started_agent(waap_root, output_format, agent_id, system, backend);

    match result {
        Ok(RunOutcome::Completed) => {
            if let Err(error) = mark_completed(waap_root, output_format, agent_id) {
                return Err(persist_failed_after_error(
                    waap_root,
                    output_format,
                    agent_id,
                    error,
                ));
            }
            Ok(ExitCode::SUCCESS)
        }
        Ok(RunOutcome::Failed(code)) => {
            mark_failed(waap_root, output_format, agent_id)?;
            Ok(code)
        }
        Err(error) => Err(persist_failed_after_error(
            waap_root,
            output_format,
            agent_id,
            error,
        )),
    }
}

fn run_started_agent(
    waap_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
    system: &AgentSystem,
    backend: &mut dyn AgentSystemBackend,
) -> io::Result<RunOutcome> {
    let mut worktree = AgentWorktree::create(waap_root, agent_id)?;
    let repository_root = waap_root.canonicalize()?;
    let prompt = build_agent_goal(system, &repository_root, agent_id, worktree.dir());
    let run_result = backend
        .start(StartContext {
            agent_id,
            prompt: &prompt,
            repository_root: &repository_root,
            worktree_dir: worktree.dir(),
        })
        .and_then(|started| {
            update_agent_session(
                waap_root,
                output_format,
                agent_id,
                &started.session_id,
                system.clone(),
            )?;
            started.handle.wait()
        });
    let cleanup_result = worktree.cleanup();
    collapse_errors(run_result, cleanup_result)
}

fn build_agent_goal(
    system: &AgentSystem,
    repository_root: &Path,
    agent_id: &str,
    worktree_dir: &Path,
) -> String {
    let instruction_path = repository_root.join(format!(".waap/agents/{agent_id}/agent.md"));
    match system {
        AgentSystem::Opencode => format!(
            "Use the agent worktree at {} for all work. To integrate, run `git -C {} merge --ff-only {}`. Complete when instructions in {} are satisfied",
            worktree_dir.display(),
            repository_root.display(),
            agent_id,
            instruction_path.display(),
        ),
        AgentSystem::Claude | AgentSystem::Codex => {
            format!("Complete when instructions in /.waap/agents/{agent_id}/agent.md are satisfied")
        }
    }
}

fn require_ready_agent(waap_root: &Path, agent_id: &str) -> io::Result<()> {
    let (metadata, _) = read_agent_record(waap_root, agent_id)?;
    let current = AgentStatus::parse(&metadata.status).expect("validated agent status");
    current.validate_transition(AgentStatus::Running)
}

fn agent_worktree_dir(agent_id: &str) -> PathBuf {
    Path::new("worktrees").join(agent_id)
}

struct AgentWorktree {
    waap_root: PathBuf,
    relative_path: PathBuf,
    worktree_dir: PathBuf,
    cleanup_pending: bool,
}

impl AgentWorktree {
    // Call only after committing the running state so the branch includes it.
    fn create(waap_root: &Path, agent_id: &str) -> io::Result<Self> {
        let relative_path = agent_worktree_dir(agent_id);
        let worktree_dir = create_worktree(waap_root, agent_id, &relative_path)?;
        Ok(Self {
            waap_root: waap_root.to_path_buf(),
            relative_path,
            worktree_dir,
            cleanup_pending: true,
        })
    }

    fn dir(&self) -> &Path {
        &self.worktree_dir
    }

    fn cleanup(&mut self) -> io::Result<()> {
        if !self.cleanup_pending {
            return Ok(());
        }
        remove_worktree(&self.waap_root, &self.relative_path)?;
        self.cleanup_pending = false;
        Ok(())
    }
}

fn collapse_errors<T>(run_result: io::Result<T>, cleanup_result: io::Result<()>) -> io::Result<T> {
    match (run_result, cleanup_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
        (Err(run_error), Ok(())) => Err(run_error),
        (Err(run_error), Err(cleanup_error)) => Err(io::Error::new(
            run_error.kind(),
            format!("{run_error}; worktree cleanup also failed: {cleanup_error}"),
        )),
    }
}

impl Drop for AgentWorktree {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            log::error!(
                "failed to clean up agent worktree {}: {error}",
                self.worktree_dir.display()
            );
        }
    }
}

fn mark_running(
    waap_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
    metadata: &mut AgentMetadata,
    body: &str,
) -> io::Result<()> {
    transition_agent_status(metadata, AgentStatus::Running)?;
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

fn update_agent_session(
    waap_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
    session_id: &str,
    system: AgentSystem,
) -> io::Result<()> {
    let (mut metadata, body) = read_agent_record(waap_root, agent_id)?;
    if metadata.status != AgentStatus::Running.as_str() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("agent {agent_id} must be running to assign a session"),
        ));
    }
    if let Some(existing_session_id) = &metadata.session_id {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("agent {agent_id} already has session id {existing_session_id}"),
        ));
    }
    if let Some(existing_system) = &metadata.system {
        if existing_system != &system {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "agent {agent_id} system mismatch: expected {}, got {}",
                    existing_system.as_str(),
                    system.as_str()
                ),
            ));
        }
    }

    let header = format!("{} session", system.as_str());
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

fn mark_completed(
    waap_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
) -> io::Result<()> {
    transition_and_commit_status(
        waap_root,
        output_format,
        agent_id,
        AgentStatus::Completed,
        "Completed agent",
        &format!("waap agent completed {agent_id}"),
    )
}

fn mark_failed(waap_root: &Path, output_format: &OutputFormat, agent_id: &str) -> io::Result<()> {
    transition_and_commit_status(
        waap_root,
        output_format,
        agent_id,
        AgentStatus::Failed,
        "Failed agent",
        &format!("waap agent failed {agent_id}"),
    )
}

fn transition_and_commit_status(
    waap_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
    status: AgentStatus,
    header: &str,
    commit_message: &str,
) -> io::Result<()> {
    let (mut metadata, body) = read_agent_record(waap_root, agent_id)?;
    if metadata.status == status.as_str() {
        return Ok(());
    }
    let previous_metadata = metadata.clone();
    transition_agent_status(&mut metadata, status)?;
    let persistence_result = (|| {
        write_agent_record(waap_root, agent_id, &metadata, &body)?;
        let report = load_agent_report(waap_root, agent_id)?;
        let commit = commit_paths(waap_root, &[report.path.as_path()], commit_message)?;
        Ok((report, commit))
    })();
    let (report, commit) = match persistence_result {
        Ok(persisted) => persisted,
        Err(primary) => {
            return match write_agent_record(waap_root, agent_id, &previous_metadata, &body) {
                Ok(()) => Err(primary),
                Err(rollback_error) => Err(io::Error::new(
                    primary.kind(),
                    format!("{primary}; failed to restore previous agent status: {rollback_error}"),
                )),
            };
        }
    };
    print_run_agent_report(output_format, header, &report, &commit);
    Ok(())
}

fn persist_failed_after_error(
    waap_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
    primary: io::Error,
) -> io::Error {
    match mark_failed(waap_root, output_format, agent_id) {
        Ok(()) => primary,
        Err(persistence_error) => io::Error::new(
            primary.kind(),
            format!("{primary}; failed to persist agent failure state: {persistence_error}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::ExitCode;

    use serde_json::json;
    use tempfile::tempdir;

    use super::{
        agent_worktree_dir, build_agent_goal, collapse_errors, mark_completed, mark_failed,
        mark_running, persist_failed_after_error, require_ready_agent, run_agent,
        run_agent_with_backend, transition_and_commit_status, update_agent_session, AgentWorktree,
    };
    use crate::agent::backend::{fake::FakeBackend, RunOutcome};
    use crate::agent::{
        agent_report_json, read_agent_record, transition_agent_status, write_agent_record,
        AgentMetadata, AgentReport, AgentStatus, AgentSystem,
    };
    use crate::cli::OutputFormat;
    use crate::git::{create_worktree, remove_worktree};
    use crate::test_git::{init_repo_with_commit, run as git};

    #[test]
    fn agent_worktree_creates_and_explicitly_removes_agent_directory() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());

        let mut worktree = AgentWorktree::create(dir.path(), "aa-00000001").unwrap();
        assert!(worktree.dir().is_dir());
        assert!(worktree.dir().ends_with("worktrees/aa-00000001"));

        worktree.cleanup().unwrap();
        assert!(!dir.path().join("worktrees/aa-00000001").exists());
    }

    #[test]
    fn agent_worktree_drop_cleans_up_after_early_error() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());

        fn fail_after_create(waap_root: &Path) -> std::io::Result<()> {
            let worktree = AgentWorktree::create(waap_root, "aa-00000001")?;
            assert!(worktree.dir().is_dir());
            Err(std::io::Error::other("launch failed"))
        }

        let error = fail_after_create(dir.path()).unwrap_err();
        assert_eq!(error.to_string(), "launch failed");
        assert!(!dir.path().join("worktrees/aa-00000001").exists());
    }

    #[test]
    fn collapse_errors_returns_cleanup_error_after_successful_run() {
        let error =
            collapse_errors(Ok(17), Err(std::io::Error::other("cleanup failed"))).unwrap_err();

        assert_eq!(error.to_string(), "cleanup failed");
    }

    #[test]
    fn collapse_errors_returns_run_value_when_both_succeed() {
        assert_eq!(collapse_errors(Ok(17), Ok(())).unwrap(), 17);
    }

    #[test]
    fn collapse_errors_returns_run_error_when_cleanup_succeeds() {
        let error = collapse_errors::<()>(
            Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionAborted,
                "run failed",
            )),
            Ok(()),
        )
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::ConnectionAborted);
        assert_eq!(error.to_string(), "run failed");
    }

    #[test]
    fn collapse_errors_preserves_run_error_and_cleanup_diagnostics() {
        let error = collapse_errors(
            Err::<(), _>(std::io::Error::new(
                std::io::ErrorKind::ConnectionAborted,
                "run failed",
            )),
            Err(std::io::Error::other("cleanup failed")),
        )
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::ConnectionAborted);
        assert!(error.to_string().starts_with("run failed;"));
        assert!(error
            .to_string()
            .contains("worktree cleanup also failed: cleanup failed"));
    }

    #[test]
    fn agent_worktree_cuts_branch_from_running_commit() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let seed = git(dir.path(), &["rev-parse", "HEAD"]);

        fs::write(dir.path().join("running.txt"), "running\n").unwrap();
        git(dir.path(), &["add", "running.txt"]);
        git(
            dir.path(),
            &["commit", "-q", "-m", "waap agent run aa-00000001"],
        );
        let mut worktree = AgentWorktree::create(dir.path(), "aa-00000001").unwrap();
        assert!(worktree.dir().join("running.txt").exists());
        let head_at_run = git(worktree.dir(), &["rev-parse", "HEAD"]);

        let run_commit = git(dir.path(), &["rev-parse", "HEAD"]);
        assert_ne!(run_commit, seed);
        assert_eq!(head_at_run, run_commit);
        let parents = git(dir.path(), &["rev-list", "--merges", "HEAD"]);
        assert!(parents.is_empty(), "unexpected merge commits: {parents}");
        worktree.cleanup().unwrap();
    }

    #[test]
    fn agent_branch_rebase_and_ff_merge_keeps_main_linear() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());

        fs::write(dir.path().join("running.txt"), "running\n").unwrap();
        git(dir.path(), &["add", "running.txt"]);
        git(
            dir.path(),
            &["commit", "-q", "-m", "waap agent run aa-00000001"],
        );
        let relative_path = agent_worktree_dir("aa-00000001");
        let worktree = create_worktree(dir.path(), "aa-00000001", &relative_path).unwrap();

        fs::write(worktree.join("feature.txt"), "feature\n").unwrap();
        git(&worktree, &["add", "feature.txt"]);
        git(&worktree, &["commit", "-q", "-m", "feature aa-00000001"]);

        fs::write(dir.path().join("other.txt"), "other\n").unwrap();
        git(dir.path(), &["add", "other.txt"]);
        git(dir.path(), &["commit", "-q", "-m", "other agent"]);

        git(&worktree, &["rebase", "main"]);
        git(dir.path(), &["merge", "--ff-only", "aa-00000001"]);

        let merges = git(dir.path(), &["rev-list", "--merges", "HEAD"]);
        assert!(merges.is_empty(), "unexpected merge commits: {merges}");
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
    fn require_ready_agent_rejects_running_status() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "running");

        let error = require_ready_agent(dir.path(), agent_id).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(
            error.to_string(),
            "invalid agent status transition: running -> running"
        );
    }

    #[test]
    fn production_run_rejects_running_before_loading_opencode_environment() {
        let _lock = crate::agent::OPENCODE_ENV_LOCK.lock().unwrap();
        let names = [
            "OPENCODE_SERVER_URL",
            "OPENCODE_SERVER_USERNAME",
            "OPENCODE_SERVER_PASSWORD",
            "OPENCODE_SERVER_MODEL",
        ];
        let previous = names.map(std::env::var_os);
        for name in names {
            std::env::remove_var(name);
        }
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "running");

        let error = run_agent(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            &AgentSystem::Opencode,
        )
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        for (name, value) in names.into_iter().zip(previous) {
            if let Some(value) = value {
                std::env::set_var(name, value);
            }
        }
    }

    #[test]
    fn backend_construction_error_leaves_ready_agent_unchanged() {
        let _lock = crate::agent::OPENCODE_ENV_LOCK.lock().unwrap();
        let names = [
            "OPENCODE_SERVER_URL",
            "OPENCODE_SERVER_USERNAME",
            "OPENCODE_SERVER_PASSWORD",
            "OPENCODE_SERVER_MODEL",
        ];
        let previous = names.map(std::env::var_os);
        for name in names {
            std::env::remove_var(name);
        }
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "ready");

        let error = run_agent(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            &AgentSystem::Opencode,
        )
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        assert_eq!(
            read_agent_record(dir.path(), agent_id).unwrap().0.status,
            "ready"
        );
        for (name, value) in names.into_iter().zip(previous) {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }

    #[test]
    fn run_agent_passes_start_context_after_worktree_creation() {
        for system in [
            AgentSystem::Opencode,
            AgentSystem::Claude,
            AgentSystem::Codex,
        ] {
            let dir = tempdir().unwrap();
            init_repo_with_commit(dir.path());
            let agent_id = "aa-00000001";
            seed_agent_record(dir.path(), agent_id, "ready");
            let mut backend = FakeBackend {
                session_id: "ses_started".to_string(),
                ..FakeBackend::default()
            };

            let code = run_agent_with_backend(
                dir.path(),
                &OutputFormat::Json,
                agent_id,
                &system,
                &mut backend,
            )
            .unwrap();

            assert_eq!(code, ExitCode::SUCCESS);
            assert_eq!(backend.start_calls.len(), 1);
            assert_eq!(backend.wait_calls.get(), 1);
            let call = &backend.start_calls[0];
            assert_eq!(call.agent_id, agent_id);
            assert_eq!(
                call.prompt,
                build_agent_goal(&system, dir.path(), agent_id, &call.worktree_dir)
            );
            assert_eq!(call.repository_root, dir.path().canonicalize().unwrap());
            assert!(call.worktree_dir.ends_with("worktrees/aa-00000001"));
            assert!(call.worktree_existed);
            assert!(!call.worktree_dir.exists());

            let (metadata, _) = read_agent_record(dir.path(), agent_id).unwrap();
            assert_eq!(metadata.status, "completed");
            assert_eq!(metadata.system, Some(system));
            assert_eq!(metadata.session_id.as_deref(), Some("ses_started"));
            let running_record = git(
                dir.path(),
                &["show", "HEAD~2:.waap/agents/aa-00000001/agent.md"],
            );
            assert!(running_record.contains("status = \"running\""));
            assert!(!running_record.contains("session_id ="));
            let started_record = git(
                dir.path(),
                &["show", "HEAD~1:.waap/agents/aa-00000001/agent.md"],
            );
            assert!(started_record.contains("session_id = \"ses_started\""));
        }
    }

    #[test]
    fn opencode_goal_allows_final_integration_from_canonical_checkout() {
        let repository_root = PathBuf::from("/repository");
        let worktree_dir = repository_root.join("worktrees/aa-00000001");

        assert_eq!(
            build_agent_goal(
                &AgentSystem::Opencode,
                &repository_root,
                "aa-00000001",
                &worktree_dir,
            ),
            "Use the agent worktree at /repository/worktrees/aa-00000001 for all work. To integrate, run `git -C /repository merge --ff-only aa-00000001`. Complete when instructions in /repository/.waap/agents/aa-00000001/agent.md are satisfied"
        );
    }

    #[test]
    fn claude_and_codex_goals_remain_unchanged() {
        let repository_root = PathBuf::from("/repository");
        let worktree_dir = repository_root.join("worktrees/aa-00000001");

        for system in [AgentSystem::Claude, AgentSystem::Codex] {
            assert_eq!(
                build_agent_goal(&system, &repository_root, "aa-00000001", &worktree_dir),
                "Complete when instructions in /.waap/agents/aa-00000001/agent.md are satisfied"
            );
        }
    }

    #[test]
    fn run_agent_persists_started_session_before_wait_completion() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "ready");
        let mut backend = FakeBackend {
            session_id: "th_authentic".to_string(),
            ..FakeBackend::default()
        };

        let code = run_agent_with_backend(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            &AgentSystem::Codex,
            &mut backend,
        )
        .unwrap();

        assert_eq!(code, ExitCode::SUCCESS);
        assert_eq!(backend.start_calls.len(), 1);
        assert_eq!(backend.wait_calls.get(), 1);
        let (metadata, _) = read_agent_record(dir.path(), agent_id).unwrap();
        assert_eq!(metadata.session_id.as_deref(), Some("th_authentic"));
        let running_record = git(
            dir.path(),
            &["show", "HEAD~2:.waap/agents/aa-00000001/agent.md"],
        );
        assert!(running_record.contains("status = \"running\""));
        assert!(!running_record.contains("session_id ="));
        let subjects = git(dir.path(), &["log", "-3", "--format=%s"]);
        assert_eq!(
            subjects.lines().collect::<Vec<_>>(),
            [
                "waap agent completed aa-00000001",
                "waap agent codex session aa-00000001",
                "waap agent run aa-00000001"
            ]
        );
    }

    #[test]
    fn run_agent_failed_outcome_cleans_worktree_and_persists_failed_status() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "ready");
        let mut backend = FakeBackend {
            outcome: Some(RunOutcome::Failed(ExitCode::from(7))),
            session_id: "claude-session".to_string(),
            ..FakeBackend::default()
        };

        let code = run_agent_with_backend(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            &AgentSystem::Claude,
            &mut backend,
        )
        .unwrap();

        assert_eq!(code, ExitCode::from(7));
        let metadata = read_agent_record(dir.path(), agent_id).unwrap().0;
        assert_eq!(metadata.status, "failed");
        assert_eq!(metadata.session_id.as_deref(), Some("claude-session"));
        assert_eq!(backend.wait_calls.get(), 1);
        assert!(!dir.path().join(agent_worktree_dir(agent_id)).exists());
        assert_eq!(
            git(dir.path(), &["log", "-1", "--format=%s"]),
            "waap agent failed aa-00000001"
        );
    }

    #[test]
    fn successful_run_accepts_agent_persisted_completed_status() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "ready");
        let root = dir.path().to_path_buf();
        let mut backend = FakeBackend {
            wait_action: Some(Box::new(move || {
                persist_agent_status(&root, agent_id, AgentStatus::Completed)
            })),
            ..FakeBackend::default()
        };

        let code = run_agent_with_backend(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            &AgentSystem::Opencode,
            &mut backend,
        )
        .unwrap();

        assert_eq!(code, ExitCode::SUCCESS);
        assert_eq!(
            read_agent_record(dir.path(), agent_id).unwrap().0.status,
            "completed"
        );
        assert_eq!(
            git(dir.path(), &["log", "-1", "--format=%s"]),
            "agent persisted completed"
        );
    }

    #[test]
    fn failed_run_preserves_exit_code_when_agent_persisted_failed_status() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "ready");
        let root = dir.path().to_path_buf();
        let mut backend = FakeBackend {
            outcome: Some(RunOutcome::Failed(ExitCode::from(7))),
            wait_action: Some(Box::new(move || {
                persist_agent_status(&root, agent_id, AgentStatus::Failed)
            })),
            ..FakeBackend::default()
        };

        let code = run_agent_with_backend(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            &AgentSystem::Opencode,
            &mut backend,
        )
        .unwrap();

        assert_eq!(code, ExitCode::from(7));
        assert_eq!(
            read_agent_record(dir.path(), agent_id).unwrap().0.status,
            "failed"
        );
        assert_eq!(
            git(dir.path(), &["log", "-1", "--format=%s"]),
            "agent persisted failed"
        );
    }

    #[test]
    fn run_agent_backend_error_cleans_worktree_and_propagates() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "ready");
        let mut backend = FakeBackend {
            start_error: Some("launch failed".to_string()),
            ..FakeBackend::default()
        };

        let error = run_agent_with_backend(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            &AgentSystem::Opencode,
            &mut backend,
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "launch failed");
        assert_eq!(backend.start_calls.len(), 1);
        assert_eq!(backend.wait_calls.get(), 0);
        assert!(!dir.path().join(agent_worktree_dir(agent_id)).exists());
        assert_eq!(
            read_agent_record(dir.path(), agent_id).unwrap().0.status,
            "failed"
        );
    }

    #[test]
    fn run_agent_worktree_creation_error_persists_failed_status() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "ready");
        git(dir.path(), &["branch", agent_id]);
        let mut backend = FakeBackend::default();

        let error = run_agent_with_backend(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            &AgentSystem::Opencode,
            &mut backend,
        )
        .unwrap_err();

        assert!(error.to_string().contains("git worktree failed"));
        assert_eq!(backend.start_calls.len(), 0);
        assert_eq!(
            read_agent_record(dir.path(), agent_id).unwrap().0.status,
            "failed"
        );
    }

    #[test]
    fn run_agent_session_persistence_error_persists_failed_status() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "ready");
        let (mut metadata, body) = read_agent_record(dir.path(), agent_id).unwrap();
        metadata.session_id = Some("ses_existing".to_string());
        write_agent_record(dir.path(), agent_id, &metadata, &body).unwrap();
        git(dir.path(), &["add", "-A"]);
        git(dir.path(), &["commit", "-q", "-m", "seed session"]);
        let mut backend = FakeBackend::default();

        let error = run_agent_with_backend(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            &AgentSystem::Opencode,
            &mut backend,
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("already has session id ses_existing"));
        let metadata = read_agent_record(dir.path(), agent_id).unwrap().0;
        assert_eq!(metadata.status, "failed");
        assert_eq!(metadata.session_id.as_deref(), Some("ses_existing"));
        assert_eq!(backend.wait_calls.get(), 0);
    }

    #[test]
    fn run_agent_cleanup_error_persists_failed_status() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "ready");
        let git_admin = dir.path().join(".git/worktrees").join(agent_id);
        let mut backend = FakeBackend {
            wait_action: Some(Box::new(move || {
                fs::remove_dir_all(&git_admin)?;
                Ok(())
            })),
            ..FakeBackend::default()
        };

        let error = run_agent_with_backend(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            &AgentSystem::Opencode,
            &mut backend,
        )
        .unwrap_err();

        assert!(error.to_string().contains("git worktree failed"));
        assert_eq!(
            read_agent_record(dir.path(), agent_id).unwrap().0.status,
            "failed"
        );
    }

    #[test]
    fn run_agent_wait_error_occurs_after_session_persistence_and_cleans_worktree() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "ready");
        let mut backend = FakeBackend {
            session_id: "ses_started".to_string(),
            wait_error: Some("wait failed".to_string()),
            ..FakeBackend::default()
        };

        let error = run_agent_with_backend(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            &AgentSystem::Opencode,
            &mut backend,
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "wait failed");
        assert_eq!(backend.wait_calls.get(), 1);
        let metadata = read_agent_record(dir.path(), agent_id).unwrap().0;
        assert_eq!(metadata.status, "failed");
        assert_eq!(metadata.session_id.as_deref(), Some("ses_started"));
        assert!(!dir.path().join(agent_worktree_dir(agent_id)).exists());
    }

    #[test]
    fn run_agent_preserves_runner_error_when_agent_persisted_failed_status() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "ready");
        let root = dir.path().to_path_buf();
        let mut backend = FakeBackend {
            wait_error: Some("wait failed".to_string()),
            wait_action: Some(Box::new(move || {
                persist_agent_status(&root, agent_id, AgentStatus::Failed)
            })),
            ..FakeBackend::default()
        };

        let error = run_agent_with_backend(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            &AgentSystem::Opencode,
            &mut backend,
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "wait failed");
        assert_eq!(
            read_agent_record(dir.path(), agent_id).unwrap().0.status,
            "failed"
        );
        assert_eq!(
            git(dir.path(), &["log", "-1", "--format=%s"]),
            "agent persisted failed"
        );
    }

    #[test]
    fn run_agent_preserves_wait_error_when_failure_persistence_is_rejected() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "ready");
        let root = dir.path().to_path_buf();
        let mut backend = FakeBackend {
            wait_error: Some("wait failed".to_string()),
            wait_action: Some(Box::new(move || abort_agent(&root, agent_id))),
            ..FakeBackend::default()
        };

        let error = run_agent_with_backend(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            &AgentSystem::Opencode,
            &mut backend,
        )
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::Other);
        assert!(error.to_string().starts_with("wait failed;"));
        assert!(error
            .to_string()
            .contains("failed to persist agent failure state"));
        assert_eq!(
            read_agent_record(dir.path(), agent_id).unwrap().0.status,
            "aborted"
        );
    }

    #[test]
    fn successful_run_does_not_overwrite_concurrent_abort() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "ready");
        let root = dir.path().to_path_buf();
        let mut backend = FakeBackend {
            wait_action: Some(Box::new(move || abort_agent(&root, agent_id))),
            ..FakeBackend::default()
        };

        let error = run_agent_with_backend(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            &AgentSystem::Opencode,
            &mut backend,
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .starts_with("invalid agent status transition: aborted -> completed;"));
        assert_eq!(
            read_agent_record(dir.path(), agent_id).unwrap().0.status,
            "aborted"
        );
    }

    #[test]
    fn completion_persistence_error_rolls_back_then_persists_failed_status() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "running");

        let primary = transition_and_commit_status(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            AgentStatus::Completed,
            "Completed agent",
            "invalid\0commit message",
        )
        .unwrap_err();
        assert_eq!(
            read_agent_record(dir.path(), agent_id).unwrap().0.status,
            "running"
        );

        let error = persist_failed_after_error(dir.path(), &OutputFormat::Json, agent_id, primary);

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("nul byte"));
        assert_eq!(
            read_agent_record(dir.path(), agent_id).unwrap().0.status,
            "failed"
        );
        assert_eq!(
            git(dir.path(), &["log", "-1", "--format=%s"]),
            format!("waap agent failed {agent_id}")
        );
    }

    #[test]
    fn run_agent_rejects_every_terminal_status_without_starting_backend() {
        for status in ["completed", "failed", "aborted"] {
            let dir = tempdir().unwrap();
            init_repo_with_commit(dir.path());
            let agent_id = "aa-00000001";
            seed_agent_record(dir.path(), agent_id, status);
            let mut backend = FakeBackend::default();

            let error = run_agent_with_backend(
                dir.path(),
                &OutputFormat::Json,
                agent_id,
                &AgentSystem::Opencode,
                &mut backend,
            )
            .unwrap_err();

            assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
            assert!(error.to_string().contains(&format!("{status} -> running")));
            assert!(backend.start_calls.is_empty());
            assert_eq!(
                read_agent_record(dir.path(), agent_id).unwrap().0.status,
                status
            );
        }
    }

    #[test]
    fn mark_running_persists_and_commits_running_transition() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "ready");
        let (mut metadata, body) = read_agent_record(dir.path(), agent_id).unwrap();

        mark_running(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            &mut metadata,
            &body,
        )
        .unwrap();

        assert_eq!(metadata.status, "running");
        assert_eq!(
            read_agent_record(dir.path(), agent_id).unwrap().0.status,
            "running"
        );
        assert_eq!(
            git(dir.path(), &["log", "-1", "--format=%s"]),
            format!("waap agent run {agent_id}")
        );
    }

    #[test]
    fn generated_session_ids_are_visible_on_main_during_runs() {
        for (system, session_id) in [
            (AgentSystem::Opencode, "ses_live"),
            (AgentSystem::Codex, "th_live"),
        ] {
            let dir = tempdir().unwrap();
            init_repo_with_commit(dir.path());
            let agent_id = "aa-00000001";
            seed_agent_record(dir.path(), agent_id, "ready");
            let (mut metadata, body) = read_agent_record(dir.path(), agent_id).unwrap();
            metadata.system = Some(system.clone());

            mark_running(
                dir.path(),
                &OutputFormat::Json,
                agent_id,
                &mut metadata,
                &body,
            )
            .unwrap();
            let mut worktree = AgentWorktree::create(dir.path(), agent_id).unwrap();
            update_agent_session(
                dir.path(),
                &OutputFormat::Json,
                agent_id,
                session_id,
                system.clone(),
            )
            .unwrap();

            let (main_metadata, _) = read_agent_record(dir.path(), agent_id).unwrap();
            assert_eq!(main_metadata.status, "running");
            assert_eq!(main_metadata.system, Some(system.clone()));
            assert_eq!(main_metadata.session_id.as_deref(), Some(session_id));

            let (branch_metadata, _) = read_agent_record(worktree.dir(), agent_id).unwrap();
            assert_eq!(branch_metadata.status, "running");
            assert_eq!(branch_metadata.system, Some(system));
            assert_eq!(branch_metadata.session_id, None);

            worktree.cleanup().unwrap();
        }
    }

    #[test]
    fn mark_completed_updates_status_and_commits() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        let path = seed_agent_record(dir.path(), agent_id, "running");

        mark_completed(dir.path(), &OutputFormat::Json, agent_id).unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("status = \"completed\"\n"));
        let subject = git(dir.path(), &["log", "-1", "--format=%s"]);
        assert_eq!(subject, format!("waap agent completed {agent_id}"));
        let merges = git(dir.path(), &["rev-list", "--merges", "HEAD"]);
        assert!(merges.is_empty(), "unexpected merge commits: {merges}");
    }

    #[test]
    fn mark_completed_is_idempotent_without_writing_or_committing() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        let path = seed_agent_record(dir.path(), agent_id, "completed");
        let head_before = git(dir.path(), &["rev-parse", "HEAD"]);

        let contents_before = fs::read_to_string(&path).unwrap();

        mark_completed(dir.path(), &OutputFormat::Json, agent_id).unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), contents_before);
        let head_after = git(dir.path(), &["rev-parse", "HEAD"]);
        assert_eq!(head_before, head_after);
    }

    #[test]
    fn mark_failed_is_idempotent_without_writing_or_committing() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        let path = seed_agent_record(dir.path(), agent_id, "failed");
        let contents_before = fs::read_to_string(&path).unwrap();
        let head_before = git(dir.path(), &["rev-parse", "HEAD"]);

        mark_failed(dir.path(), &OutputFormat::Json, agent_id).unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), contents_before);
        assert_eq!(git(dir.path(), &["rev-parse", "HEAD"]), head_before);
    }

    #[test]
    fn runner_terminal_persistence_rejects_conflicting_statuses() {
        for (current, desired) in [
            (AgentStatus::Completed, AgentStatus::Failed),
            (AgentStatus::Failed, AgentStatus::Completed),
            (AgentStatus::Aborted, AgentStatus::Completed),
            (AgentStatus::Aborted, AgentStatus::Failed),
        ] {
            let dir = tempdir().unwrap();
            init_repo_with_commit(dir.path());
            let agent_id = "aa-00000001";
            seed_agent_record(dir.path(), agent_id, current.as_str());
            let head_before = git(dir.path(), &["rev-parse", "HEAD"]);

            let error = match desired {
                AgentStatus::Completed => {
                    mark_completed(dir.path(), &OutputFormat::Json, agent_id).unwrap_err()
                }
                AgentStatus::Failed => {
                    mark_failed(dir.path(), &OutputFormat::Json, agent_id).unwrap_err()
                }
                _ => unreachable!(),
            };

            assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
            assert_eq!(
                error.to_string(),
                format!(
                    "invalid agent status transition: {} -> {}",
                    current.as_str(),
                    desired.as_str()
                )
            );
            assert_eq!(
                read_agent_record(dir.path(), agent_id).unwrap().0.status,
                current.as_str()
            );
            assert_eq!(git(dir.path(), &["rev-parse", "HEAD"]), head_before);
        }
    }

    #[test]
    fn mark_completed_does_not_change_ticket_status() {
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

        mark_completed(dir.path(), &OutputFormat::Json, agent_id).unwrap();

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
    fn update_agent_session_writes_codex_session_id_and_system_and_commits() {
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
    fn update_agent_session_rejects_non_running_agent() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "ready");

        let error = update_agent_session(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            "th_abc123",
            AgentSystem::Codex,
        )
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("must be running"));
        assert_eq!(
            read_agent_record(dir.path(), agent_id).unwrap().0.status,
            "ready"
        );
    }

    #[test]
    fn update_agent_session_rejects_any_existing_session_id_without_committing() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "running");

        update_agent_session(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            "th_abc123",
            AgentSystem::Codex,
        )
        .unwrap();
        let head_after_first = git(dir.path(), &["rev-parse", "HEAD"]);

        for session_id in ["th_abc123", "th_different"] {
            let error = update_agent_session(
                dir.path(),
                &OutputFormat::Json,
                agent_id,
                session_id,
                AgentSystem::Codex,
            )
            .unwrap_err();

            assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
            assert_eq!(
                error.to_string(),
                format!("agent {agent_id} already has session id th_abc123")
            );
            assert_eq!(git(dir.path(), &["rev-parse", "HEAD"]), head_after_first);
        }
    }

    #[test]
    fn update_agent_session_rejects_conflicting_system_without_committing() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "running");
        let (mut metadata, body) = read_agent_record(dir.path(), agent_id).unwrap();
        metadata.system = Some(AgentSystem::Claude);
        crate::agent::write_agent_record(dir.path(), agent_id, &metadata, &body).unwrap();
        git(dir.path(), &["add", "-A"]);
        git(dir.path(), &["commit", "-q", "-m", "seed agent system"]);
        let head_before = git(dir.path(), &["rev-parse", "HEAD"]);

        let error = update_agent_session(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            "th_abc123",
            AgentSystem::Codex,
        )
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(
            error.to_string(),
            format!("agent {agent_id} system mismatch: expected claude, got codex")
        );
        assert_eq!(git(dir.path(), &["rev-parse", "HEAD"]), head_before);
        let (metadata, _) = read_agent_record(dir.path(), agent_id).unwrap();
        assert_eq!(metadata.session_id, None);
        assert_eq!(metadata.system, Some(AgentSystem::Claude));
    }

    #[test]
    fn update_agent_session_accepts_matching_system_without_session_id() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "running");
        let (mut metadata, body) = read_agent_record(dir.path(), agent_id).unwrap();
        metadata.system = Some(AgentSystem::Codex);
        crate::agent::write_agent_record(dir.path(), agent_id, &metadata, &body).unwrap();
        git(dir.path(), &["add", "-A"]);
        git(dir.path(), &["commit", "-q", "-m", "seed agent system"]);

        update_agent_session(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            "th_abc123",
            AgentSystem::Codex,
        )
        .unwrap();

        let (metadata, _) = read_agent_record(dir.path(), agent_id).unwrap();
        assert_eq!(metadata.session_id.as_deref(), Some("th_abc123"));
        assert_eq!(metadata.system, Some(AgentSystem::Codex));
        assert_eq!(
            git(dir.path(), &["log", "-1", "--format=%s"]),
            format!("waap agent codex session {agent_id}")
        );
    }

    fn abort_agent(root: &Path, agent_id: &str) -> std::io::Result<()> {
        let (mut metadata, body) = read_agent_record(root, agent_id)?;
        transition_agent_status(&mut metadata, AgentStatus::Aborted)?;
        write_agent_record(root, agent_id, &metadata, &body)?;
        git(root, &["add", "-A"]);
        git(root, &["commit", "-q", "-m", "abort agent"]);
        Ok(())
    }

    fn persist_agent_status(
        root: &Path,
        agent_id: &str,
        status: AgentStatus,
    ) -> std::io::Result<()> {
        let (mut metadata, body) = read_agent_record(root, agent_id)?;
        transition_agent_status(&mut metadata, status)?;
        write_agent_record(root, agent_id, &metadata, &body)?;
        git(root, &["add", "-A"]);
        git(
            root,
            &[
                "commit",
                "-q",
                "-m",
                &format!("agent persisted {}", status.as_str()),
            ],
        );
        Ok(())
    }
}
