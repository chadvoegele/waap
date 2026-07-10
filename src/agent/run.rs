use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use super::backend::{BackendRegistry, BackendResolver, RunContext, RunOutcome};
use crate::agent::{
    agent_report_json, load_agent_report, print_agent_report_human, read_agent_record,
    write_agent_record, AgentMetadata, AgentReport, AgentSystem,
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
    let mut backends = BackendRegistry::default();
    run_agent_with_backends(waap_root, output_format, agent_id, system, &mut backends)
}

fn run_agent_with_backends(
    waap_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
    system: &AgentSystem,
    backends: &mut dyn BackendResolver,
) -> io::Result<ExitCode> {
    let (metadata, _) = read_agent_record(waap_root, agent_id)?;
    if metadata.status == "running" {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("agent {agent_id} is already running"),
        ));
    }

    let backend = backends.resolve(system)?;
    let (mut metadata, body) = read_agent_record(waap_root, agent_id)?;
    let preparation = backend.prepare_run()?;
    metadata.system = Some(system.clone());
    if let Some(initial_session_id) = &preparation.initial_session_id {
        metadata.session_id = Some(initial_session_id.clone());
    }

    mark_running(waap_root, output_format, agent_id, &mut metadata, &body)?;
    let mut worktree = AgentWorktree::create(waap_root, agent_id)?;
    let prompt =
        format!("Complete when instructions in /.waap/agents/{agent_id}/agent.md are satisfied");
    let mut publish_session = |session_id: &str| {
        update_agent_session(
            waap_root,
            output_format,
            agent_id,
            session_id,
            system.clone(),
        )
    };
    let run_result = backend.run(RunContext {
        agent_id,
        prompt: &prompt,
        initial_session_id: preparation.initial_session_id.as_deref(),
        worktree_dir: worktree.dir(),
        publish_session: &mut publish_session,
    });
    let cleanup_result = worktree.cleanup();
    let outcome = collapse_errors(run_result, cleanup_result)?;

    match outcome {
        RunOutcome::Completed => {
            mark_completed(waap_root, output_format, agent_id)?;
            Ok(ExitCode::SUCCESS)
        }
        RunOutcome::Failed(code) => Ok(code),
    }
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
    metadata.status = "running".to_string();

    // Re-read immediately before writing so concurrent attempts cannot both start this agent.
    let (current, _) = read_agent_record(waap_root, agent_id)?;
    if current.status == "running" {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("agent {agent_id} is already running"),
        ));
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

fn update_agent_session(
    waap_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
    session_id: &str,
    system: AgentSystem,
) -> io::Result<()> {
    let (mut metadata, body) = read_agent_record(waap_root, agent_id)?;
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
    let (mut metadata, body) = read_agent_record(waap_root, agent_id)?;
    if metadata.status == "completed" {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("agent {agent_id} is already completed"),
        ));
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::ExitCode;

    use serde_json::json;
    use tempfile::tempdir;

    use super::{
        agent_worktree_dir, collapse_errors, mark_completed, mark_running, run_agent_with_backends,
        update_agent_session, AgentWorktree,
    };
    use crate::agent::backend::{fake::FakeResolver, RunOutcome};
    use crate::agent::{
        agent_report_json, read_agent_record, AgentMetadata, AgentReport, AgentSystem,
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
    fn run_agent_rejects_running_before_backend_resolution() {
        for system in [
            AgentSystem::Opencode,
            AgentSystem::Claude,
            AgentSystem::Codex,
        ] {
            let dir = tempdir().unwrap();
            init_repo_with_commit(dir.path());
            let agent_id = "aa-00000001";
            seed_agent_record(dir.path(), agent_id, "running");
            let head_before = git(dir.path(), &["rev-parse", "HEAD"]);
            let mut resolver = FakeResolver::default();

            let error = run_agent_with_backends(
                dir.path(),
                &OutputFormat::Json,
                agent_id,
                &system,
                &mut resolver,
            )
            .unwrap_err();

            assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
            assert_eq!(
                error.to_string(),
                format!("agent {agent_id} is already running")
            );
            assert_eq!(git(dir.path(), &["rev-parse", "HEAD"]), head_before);
            assert!(!dir.path().join(agent_worktree_dir(agent_id)).exists());
            assert!(resolver.resolved.is_empty());
        }
    }

    #[test]
    fn run_agent_passes_lifecycle_context_and_initial_session_to_selected_backend() {
        for system in [
            AgentSystem::Opencode,
            AgentSystem::Claude,
            AgentSystem::Codex,
        ] {
            let dir = tempdir().unwrap();
            init_repo_with_commit(dir.path());
            let agent_id = "aa-00000001";
            seed_agent_record(dir.path(), agent_id, "ready");
            let mut resolver = FakeResolver::default();
            resolver.backend_mut(&system).initial_session_id = Some("ses_initial".to_string());

            let code = run_agent_with_backends(
                dir.path(),
                &OutputFormat::Json,
                agent_id,
                &system,
                &mut resolver,
            )
            .unwrap();

            assert_eq!(code, ExitCode::SUCCESS);
            assert_eq!(resolver.resolved, vec![system.clone()]);
            let backend = resolver.backend_mut(&system);
            assert_eq!(backend.prepare_calls, 1);
            assert_eq!(backend.run_calls.len(), 1);
            let call = &backend.run_calls[0];
            assert_eq!(call.agent_id, agent_id);
            assert_eq!(
                call.prompt,
                "Complete when instructions in /.waap/agents/aa-00000001/agent.md are satisfied"
            );
            assert_eq!(call.initial_session_id.as_deref(), Some("ses_initial"));
            assert!(call.worktree_dir.ends_with("worktrees/aa-00000001"));
            assert!(!call.worktree_dir.exists());

            let (metadata, _) = read_agent_record(dir.path(), agent_id).unwrap();
            assert_eq!(metadata.status, "completed");
            assert_eq!(metadata.system, Some(system));
            assert_eq!(metadata.session_id.as_deref(), Some("ses_initial"));
            let running_record = git(
                dir.path(),
                &["show", "HEAD~1:.waap/agents/aa-00000001/agent.md"],
            );
            assert!(running_record.contains("status = \"running\""));
            assert!(running_record.contains("session_id = \"ses_initial\""));
        }
    }

    #[test]
    fn run_agent_publishes_authentic_session_before_completion() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "ready");
        let mut resolver = FakeResolver::default();
        resolver.codex.late_session_id = Some("th_authentic".to_string());

        let code = run_agent_with_backends(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            &AgentSystem::Codex,
            &mut resolver,
        )
        .unwrap();

        assert_eq!(code, ExitCode::SUCCESS);
        assert_eq!(resolver.codex.run_calls[0].initial_session_id, None);
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
    fn run_agent_failed_outcome_cleans_worktree_and_preserves_running_status() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "ready");
        let mut resolver = FakeResolver::default();
        resolver.claude.outcome = Some(RunOutcome::Failed(ExitCode::from(7)));

        let code = run_agent_with_backends(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            &AgentSystem::Claude,
            &mut resolver,
        )
        .unwrap();

        assert_eq!(code, ExitCode::from(7));
        assert_eq!(
            read_agent_record(dir.path(), agent_id).unwrap().0.status,
            "running"
        );
        assert!(!dir.path().join(agent_worktree_dir(agent_id)).exists());
        assert_eq!(
            git(dir.path(), &["log", "-1", "--format=%s"]),
            "waap agent run aa-00000001"
        );
    }

    #[test]
    fn run_agent_backend_error_cleans_worktree_and_propagates() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "ready");
        let mut resolver = FakeResolver::default();
        resolver.opencode.run_error = Some("launch failed".to_string());

        let error = run_agent_with_backends(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            &AgentSystem::Opencode,
            &mut resolver,
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "launch failed");
        assert!(!dir.path().join(agent_worktree_dir(agent_id)).exists());
        assert_eq!(
            read_agent_record(dir.path(), agent_id).unwrap().0.status,
            "running"
        );
    }

    #[test]
    fn run_agent_resolution_and_preparation_errors_happen_before_running() {
        for prepare_fails in [false, true] {
            let dir = tempdir().unwrap();
            init_repo_with_commit(dir.path());
            let agent_id = "aa-00000001";
            seed_agent_record(dir.path(), agent_id, "ready");
            let head_before = git(dir.path(), &["rev-parse", "HEAD"]);
            let mut resolver = FakeResolver::default();
            if prepare_fails {
                resolver.claude.prepare_error = Some("preparation failed".to_string());
            } else {
                resolver.resolve_error = Some(AgentSystem::Claude);
            }

            let error = run_agent_with_backends(
                dir.path(),
                &OutputFormat::Json,
                agent_id,
                &AgentSystem::Claude,
                &mut resolver,
            )
            .unwrap_err();

            let expected = if prepare_fails {
                "preparation failed"
            } else {
                "backend resolution failed"
            };
            assert_eq!(error.to_string(), expected);
            assert_eq!(git(dir.path(), &["rev-parse", "HEAD"]), head_before);
            assert_eq!(
                read_agent_record(dir.path(), agent_id).unwrap().0.status,
                "ready"
            );
            assert!(!dir.path().join(agent_worktree_dir(agent_id)).exists());
        }
    }

    #[test]
    fn mark_running_rejects_a_concurrent_running_transition() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        seed_agent_record(dir.path(), agent_id, "running");
        let (mut metadata, body) = read_agent_record(dir.path(), agent_id).unwrap();
        let head_before = git(dir.path(), &["rev-parse", "HEAD"]);

        let error = mark_running(
            dir.path(),
            &OutputFormat::Json,
            agent_id,
            &mut metadata,
            &body,
        )
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            format!("agent {agent_id} is already running")
        );
        assert_eq!(git(dir.path(), &["rev-parse", "HEAD"]), head_before);
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
    fn mark_completed_rejects_already_completed() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let agent_id = "aa-00000001";
        let path = seed_agent_record(dir.path(), agent_id, "completed");
        let head_before = git(dir.path(), &["rev-parse", "HEAD"]);

        let error = mark_completed(dir.path(), &OutputFormat::Json, agent_id).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(
            error.to_string(),
            format!("agent {agent_id} is already completed")
        );
        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("status = \"completed\"\n"));
        let head_after = git(dir.path(), &["rev-parse", "HEAD"]);
        assert_eq!(head_before, head_after);
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
}
