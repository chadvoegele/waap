use std::io;
use std::path::Path;

use serde_json::json;

use super::backend::{AbortContext, AbortOutcome, Backend};
use crate::agent::get::load_agent_report;
use crate::agent::{
    agent_report_json, print_agent_report_human, read_agent_record, transition_agent_status,
    write_agent_record, AgentReport, AgentStatus,
};
use crate::cli::OutputFormat;
use crate::git::commit_paths;
use crate::record::{list_record_ids, WaapRecordKind};
use crate::root::ControlRoot;

#[derive(Debug)]
pub(crate) struct AgentStopReport {
    stopped_agents: Vec<AgentStop>,
    pub(crate) commit: Option<String>,
}

#[derive(Debug)]
struct AgentStop {
    report: AgentReport,
    outcome: AbortOutcome,
}

impl AgentStop {
    fn header(&self) -> &'static str {
        match self.outcome {
            AbortOutcome::StopAccepted => "Stop accepted for agent",
            AbortOutcome::Stopped => "Stopped agent",
        }
    }
}

pub(crate) fn print_agent_stop_report(output_format: &OutputFormat, report: &AgentStopReport) {
    match output_format {
        OutputFormat::Json => println!("{}", agent_stop_json(report)),
        OutputFormat::HumanReadable => {
            for agent in &report.stopped_agents {
                print_agent_report_human(agent.header(), &agent.report);
            }
            if let Some(commit) = &report.commit {
                println!("Commit: {commit}");
            }
        }
    }
}

fn agent_stop_json(report: &AgentStopReport) -> serde_json::Value {
    json!({
        "stopped_agents": report.stopped_agents.iter().map(|agent| agent_report_json(&agent.report)).collect::<Vec<_>>(),
        "commit": report.commit,
    })
}

pub(crate) fn stop_agents_with_systems(
    waap_root: &Path,
    control_root: io::Result<ControlRoot>,
    agent_id: Option<&str>,
) -> io::Result<AgentStopReport> {
    let selected = select_agents_to_stop(waap_root, agent_id)?;
    let mut stopped_agents = Vec::new();
    let mut commit = None;
    for report in selected {
        if report.metadata.status == AgentStatus::Running.as_str() {
            let mut backend = report
                .metadata
                .system
                .clone()
                .unwrap_or_default()
                .backend(None)?;
            let (stopped, state_commit) =
                stop_agent_session(waap_root, &control_root, &report, &mut backend)?;
            commit = state_commit.or(commit);
            stopped_agents.push(stopped);
            continue;
        }
        let (stopped, state_commit) = persist_stopped_agent(waap_root, &report)?;
        commit = state_commit.or(commit);
        stopped_agents.push(AgentStop {
            report: stopped,
            outcome: AbortOutcome::Stopped,
        });
    }
    Ok(AgentStopReport {
        stopped_agents,
        commit,
    })
}

fn select_agents_to_stop(waap_root: &Path, agent_id: Option<&str>) -> io::Result<Vec<AgentReport>> {
    let _lock = crate::state::StateLock::acquire(waap_root)?;
    match agent_id {
        Some(id) => {
            let report = load_agent_report(waap_root, id)?;
            AgentStatus::parse(&report.metadata.status)
                .expect("validated agent status")
                .validate_transition(AgentStatus::Aborted)?;
            Ok(vec![report])
        }
        None => Ok(list_record_ids(waap_root, WaapRecordKind::Agent)?
            .iter()
            .map(|id| load_agent_report(waap_root, id))
            .collect::<io::Result<Vec<_>>>()?
            .into_iter()
            .filter(|report| report.metadata.status == AgentStatus::Running.as_str())
            .collect()),
    }
}

fn stop_agent_session(
    waap_root: &Path,
    control_root: &io::Result<ControlRoot>,
    report: &AgentReport,
    backend: &mut Backend,
) -> io::Result<(AgentStop, Option<String>)> {
    let outcome = abort_agent_session(waap_root, control_root, report, backend)?;
    let (report, commit) = match outcome {
        AbortOutcome::StopAccepted => (load_agent_report(waap_root, &report.agent_id)?, None),
        AbortOutcome::Stopped => persist_stopped_agent(waap_root, report)?,
    };
    Ok((AgentStop { report, outcome }, commit))
}

fn abort_agent_session(
    waap_root: &Path,
    control_root: &io::Result<ControlRoot>,
    report: &AgentReport,
    backend: &mut Backend,
) -> io::Result<AbortOutcome> {
    super::runtime::new()?.block_on(backend.abort(AbortContext {
        waap_root,
        session_id: report.metadata.session_id.as_deref(),
        agent_id: &report.agent_id,
        control_root,
    }))
}

fn persist_stopped_agent(
    waap_root: &Path,
    expected: &AgentReport,
) -> io::Result<(AgentReport, Option<String>)> {
    let _lock = crate::state::StateLock::acquire(waap_root)?;
    let id = &expected.agent_id;
    let current = load_agent_report(waap_root, id)?;
    if current.metadata != expected.metadata {
        return Ok((current, None));
    }
    let stopped = mark_agent_aborted(waap_root, id)?;
    let commit = commit_paths(
        waap_root,
        &[stopped.path.as_path()],
        &format!("waap agent stop {id}"),
    )
    .map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("failed to commit waap state change: {error}"),
        )
    })?;
    Ok((stopped, Some(commit)))
}

#[cfg(test)]
fn stop_agents(waap_root: &Path, agent_id: Option<&str>) -> io::Result<Vec<AgentReport>> {
    stop_agents_with_systems(
        waap_root,
        Err(io::Error::other("control root required for Codex")),
        agent_id,
    )
    .map(|report| {
        report
            .stopped_agents
            .into_iter()
            .map(|agent| agent.report)
            .collect()
    })
}

#[cfg(test)]
fn stop_agent_with_backend(
    waap_root: &Path,
    agent_id: &str,
    backend: &mut Backend,
) -> io::Result<Option<AgentReport>> {
    let report = load_agent_report(waap_root, agent_id)?;
    if report.metadata.status != AgentStatus::Running.as_str() {
        return Ok(None);
    }

    stop_agent_session(
        waap_root,
        &Err(io::Error::other("no control root")),
        &report,
        backend,
    )
    .map(|(agent, _)| Some(agent.report))
}

fn mark_agent_aborted(waap_root: &Path, agent_id: &str) -> io::Result<AgentReport> {
    let (mut metadata, body) = read_agent_record(waap_root, agent_id)?;
    transition_agent_status(&mut metadata, AgentStatus::Aborted)?;
    write_agent_record(waap_root, agent_id, &metadata, &body)?;

    load_agent_report(waap_root, agent_id)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};

    use serde_json::json;
    use tempfile::tempdir;

    use super::{agent_stop_json, stop_agent_with_backend, stop_agents, AgentStopReport};
    use crate::agent::backend::{fake::FakeBackend, Backend};
    use crate::agent::get::load_agent_report;
    use crate::agent::{AgentMetadata, AgentReport};

    #[test]
    fn stop_output_uses_explicit_outcome_for_every_system_and_status() {
        use crate::agent::backend::AbortOutcome;
        use crate::agent::AgentSystem;
        let dir = tempdir().unwrap();
        write_agent(dir.path(), "aa-test", "running");
        let mut report = load_agent_report(dir.path(), "aa-test").unwrap();
        for system in [
            None,
            Some(AgentSystem::Claude),
            Some(AgentSystem::Opencode),
            Some(AgentSystem::Codex),
        ] {
            report.metadata.system = system;
            for status in ["running", "aborted", "completed"] {
                report.metadata.status = status.to_string();
                for (outcome, header) in [
                    (AbortOutcome::StopAccepted, "Stop accepted for agent"),
                    (AbortOutcome::Stopped, "Stopped agent"),
                ] {
                    let mut output_report = load_agent_report(dir.path(), "aa-test").unwrap();
                    output_report.metadata = report.metadata.clone();
                    let stopped = super::AgentStop {
                        report: output_report,
                        outcome,
                    };
                    assert_eq!(stopped.header(), header);
                    let json = agent_stop_json(&AgentStopReport {
                        stopped_agents: vec![stopped],
                        commit: None,
                    });
                    assert_eq!(
                        json,
                        json!({
                            "stopped_agents": [crate::agent::agent_report_json(&report)],
                            "commit": null,
                        })
                    );
                }
            }
        }
    }

    #[test]
    fn running_agents_without_sessions_preserve_other_backends_local_stop() {
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
        for system in ["claude", "opencode"] {
            let dir = tempdir().unwrap();
            crate::test_git::init_repo_with_commit(dir.path());
            write_agent(dir.path(), "aa-test", "running");
            let path = dir.path().join("agents/aa-test/agent.md");
            let text = fs::read_to_string(&path).unwrap().replacen(
                "+++\n",
                &format!("+++\nsystem = \"{system}\"\n"),
                1,
            );
            fs::write(path, text).unwrap();
            // An invalid control root is irrelevant to these backends.
            let result = super::stop_agents_with_systems(
                dir.path(),
                Err(io::Error::other("invalid control root")),
                Some("aa-test"),
            )
            .unwrap();
            assert_eq!(
                result.stopped_agents[0].outcome,
                crate::agent::backend::AbortOutcome::Stopped
            );
            assert_eq!(result.stopped_agents[0].report.metadata.status, "aborted");
            assert!(result.commit.is_some());
        }
        for (name, value) in names.into_iter().zip(previous) {
            if let Some(value) = value {
                std::env::set_var(name, value);
            }
        }
    }

    #[test]
    fn abort_outcome_controls_persistence_independently_of_agent_system() {
        use crate::agent::backend::AbortOutcome;
        for outcome in [AbortOutcome::StopAccepted, AbortOutcome::Stopped] {
            let dir = tempdir().unwrap();
            crate::test_git::init_repo_with_commit(dir.path());
            write_agent_with_session(dir.path(), "aa-test", "running", Some("ses_test"));
            let expected = load_agent_report(dir.path(), "aa-test").unwrap();
            let before = fs::read(&expected.path).unwrap();
            let mut backend = Backend::Fake(FakeBackend {
                abort_outcome: outcome,
                ..FakeBackend::default()
            });
            let (report, commit) = super::stop_agent_session(
                dir.path(),
                &Err(io::Error::other("invalid control root")),
                &expected,
                &mut backend,
            )
            .unwrap();
            assert_eq!(backend.fake().abort_calls[0].agent_id, "aa-test");
            match outcome {
                AbortOutcome::StopAccepted => {
                    assert_eq!(report.report, expected);
                    assert!(commit.is_none());
                    assert_eq!(fs::read(&expected.path).unwrap(), before);
                }
                AbortOutcome::Stopped => {
                    assert_eq!(report.report.metadata.status, "aborted");
                    assert!(commit.is_some());
                }
            }
        }
    }

    #[test]
    fn codex_backend_abort_dispatches_stop_without_claiming_terminal_persistence() {
        use crate::agent::control::SocketPath;
        use crate::root::find_control_root_in;
        use std::io::{BufRead, BufReader, Read, Write};
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::net::UnixListener;

        for reply in ["accepted", "rejected", "uncertain", "malformed", "eof"] {
            let dir = tempdir().unwrap();
            let runtime = tempdir().unwrap();
            fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
            crate::test_git::init_repo_with_commit(dir.path());
            // No session ID: Codex addresses the owner by agent ID.
            write_agent(dir.path(), "aa-test", "running");
            let path = dir.path().join("agents/aa-test/agent.md");
            let text = fs::read_to_string(&path).unwrap().replacen(
                "+++\n",
                "+++\nsystem = \"codex\"\n",
                1,
            );
            fs::write(path, text).unwrap();
            let expected = load_agent_report(dir.path(), "aa-test").unwrap();
            let before = fs::read(&expected.path).unwrap();
            let root =
                Ok(find_control_root_in(dir.path(), Some(runtime.path().as_os_str())).unwrap());
            let path = SocketPath::for_agent(root.as_ref().unwrap(), "aa-test").unwrap();
            fs::create_dir_all(path.as_path().parent().unwrap()).unwrap();
            let listener = UnixListener::bind(path.as_path()).unwrap();
            listener.set_nonblocking(true).unwrap();
            std::thread::scope(|scope| {
                let server = scope.spawn(|| {
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                    let mut stream = loop {
                        match listener.accept() {
                            Ok((stream, _)) => break stream,
                            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                                assert!(std::time::Instant::now() < deadline, "backend did not connect");
                                std::thread::sleep(std::time::Duration::from_millis(5));
                            }
                            Err(error) => panic!("{error}"),
                        }
                    };
                    stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
                    let mut line = String::new();
                    BufReader::new(stream.try_clone().unwrap()).read_line(&mut line).unwrap();
                    let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                    assert_eq!(request["method"], "stop");
                    assert_eq!(request["version"], 1);
                    // The caller must release the state lock before any socket wait.
                    let _lock = crate::state::StateLock::acquire(dir.path()).unwrap();
                    let response = match reply {
                        "accepted" => json!({"version": 1, "request_id": request["request_id"], "accepted": true}),
                        "rejected" => json!({"version": 1, "request_id": request["request_id"], "accepted": false, "error": {"code": "busy", "message": "busy"}}),
                        "uncertain" => json!({"version": 1, "request_id": request["request_id"], "accepted": null, "error": {"code": "acceptance_unknown", "message": "lost acknowledgement"}}),
                        "malformed" => json!({}),
                        "eof" => return,
                        _ => unreachable!(),
                    };
                    writeln!(stream, "{response}").unwrap();
                    assert_eq!(stream.read(&mut [0]).unwrap(), 0, "no retry on this connection");
                });
                let result = super::stop_agents_with_systems(dir.path(), root, Some("aa-test"));
                if reply == "accepted" {
                    let report = result.unwrap();
                    assert_eq!(report.stopped_agents[0].report, expected);
                    assert_eq!(
                        report.stopped_agents[0].outcome,
                        crate::agent::backend::AbortOutcome::StopAccepted
                    );
                    assert!(report.commit.is_none());
                } else {
                    let error = result.unwrap_err().to_string();
                    assert!(error.contains("agent aa-test"), "{error}");
                    if reply != "rejected" {
                        assert!(error.contains("acceptance is unknown"), "{error}");
                        assert!(error.contains("no automatic retry"), "{error}");
                    }
                }
                server.join().unwrap();
            });
            assert_eq!(fs::read(&expected.path).unwrap(), before);
            assert_eq!(
                listener.accept().unwrap_err().kind(),
                io::ErrorKind::WouldBlock
            );
        }
    }

    #[test]
    fn agent_stop_stops_one_running_agent() {
        let dir = tempdir().unwrap();
        crate::test_git::init_repo_with_commit(dir.path());
        write_agent_with_session(dir.path(), "aa-3881fda0", "running", Some("ses_123"));
        write_agent(dir.path(), "aa-00000001", "running");

        let mut backend = Backend::Fake(FakeBackend::default());
        let reports = stop_agent_with_backend(dir.path(), "aa-3881fda0", &mut backend)
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>();

        assert_eq!(agent_ids(&reports), vec!["aa-3881fda0"]);
        assert_eq!(reports[0].metadata.status, "aborted");
        assert_eq!(reports[0].metadata.session_id.as_deref(), Some("ses_123"));
        assert_eq!(
            load_agent_report(dir.path(), "aa-3881fda0")
                .unwrap()
                .metadata
                .status,
            "aborted"
        );
        assert_eq!(
            load_agent_report(dir.path(), "aa-00000001")
                .unwrap()
                .metadata
                .status,
            "running"
        );
    }

    #[test]
    fn agent_stop_all_stops_all_running_agents() {
        let dir = tempdir().unwrap();
        crate::test_git::init_repo_with_commit(dir.path());
        write_agent(dir.path(), "aa-00000003", "running");
        write_agent(dir.path(), "aa-00000001", "running");

        let reports = stop_agents(dir.path(), None).unwrap();

        assert_eq!(agent_ids(&reports), vec!["aa-00000001", "aa-00000003"]);
        assert_eq!(
            load_agent_report(dir.path(), "aa-00000001")
                .unwrap()
                .metadata
                .status,
            "aborted"
        );
        assert_eq!(
            load_agent_report(dir.path(), "aa-00000003")
                .unwrap()
                .metadata
                .status,
            "aborted"
        );
    }

    #[test]
    fn agent_stop_filters_only_running_agents() {
        let dir = tempdir().unwrap();
        crate::test_git::init_repo_with_commit(dir.path());
        write_agent(dir.path(), "aa-00000001", "ready");
        write_agent(dir.path(), "aa-00000002", "running");
        write_agent(dir.path(), "aa-00000003", "completed");
        write_agent(dir.path(), "aa-00000004", "aborted");
        write_agent(dir.path(), "aa-00000005", "failed");

        let reports = stop_agents(dir.path(), None).unwrap();

        assert_eq!(agent_ids(&reports), vec!["aa-00000002"]);
        assert_eq!(
            load_agent_report(dir.path(), "aa-00000001")
                .unwrap()
                .metadata
                .status,
            "ready"
        );
        assert_eq!(
            load_agent_report(dir.path(), "aa-00000002")
                .unwrap()
                .metadata
                .status,
            "aborted"
        );
        assert_eq!(
            load_agent_report(dir.path(), "aa-00000003")
                .unwrap()
                .metadata
                .status,
            "completed"
        );
        assert_eq!(
            load_agent_report(dir.path(), "aa-00000004")
                .unwrap()
                .metadata
                .status,
            "aborted"
        );
        assert_eq!(
            load_agent_report(dir.path(), "aa-00000005")
                .unwrap()
                .metadata
                .status,
            "failed"
        );
    }

    #[test]
    fn agent_stop_rejects_explicit_terminal_agent() {
        let dir = tempdir().unwrap();
        crate::test_git::init_repo_with_commit(dir.path());
        write_agent(dir.path(), "aa-3881fda0", "completed");

        let error = stop_agents(dir.path(), Some("aa-3881fda0")).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            error.to_string(),
            "invalid agent status transition: completed -> aborted"
        );
        assert_eq!(
            load_agent_report(dir.path(), "aa-3881fda0")
                .unwrap()
                .metadata
                .status,
            "completed"
        );
    }

    #[test]
    fn agent_stop_explicitly_cancels_ready_agent() {
        let dir = tempdir().unwrap();
        crate::test_git::init_repo_with_commit(dir.path());
        write_agent(dir.path(), "aa-3881fda0", "ready");

        let reports = stop_agents(dir.path(), Some("aa-3881fda0")).unwrap();

        assert_eq!(agent_ids(&reports), vec!["aa-3881fda0"]);
        assert_eq!(reports[0].metadata.status, "aborted");
    }

    #[test]
    fn agent_stop_reports_invalid_agent_id() {
        let dir = tempdir().unwrap();
        crate::test_git::init_repo_with_commit(dir.path());

        let error = stop_agents(dir.path(), Some("not an agent")).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("not a valid agent id"));
    }

    #[test]
    fn agent_stop_reports_missing_agent() {
        let dir = tempdir().unwrap();
        crate::test_git::init_repo_with_commit(dir.path());

        let error = stop_agents(dir.path(), Some("aa-3881fda0")).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(error.to_string().contains("agents/aa-3881fda0/agent.md"));
    }

    #[test]
    fn agent_stop_aborts_opencode_sessions_for_running_agents() {
        let dir = tempdir().unwrap();
        crate::test_git::init_repo_with_commit(dir.path());
        write_agent_with_session(dir.path(), "aa-00000001", "running", Some("ses_123"));
        write_agent_with_session(dir.path(), "aa-00000002", "ready", Some("ses_ready"));
        let mut backend = Backend::Fake(FakeBackend::default());

        let reports = stop_agent_with_backend(dir.path(), "aa-00000001", &mut backend)
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>();

        assert_eq!(agent_ids(&reports), vec!["aa-00000001"]);
        assert_eq!(backend.fake().abort_calls.len(), 1);
        let call = &backend.fake().abort_calls[0];
        assert_eq!(call.waap_root, dir.path());
        assert_eq!(call.session_id.as_deref(), Some("ses_123"));
    }

    #[test]
    fn agent_stop_kills_claude_process_instead_of_opencode_abort() {
        let dir = tempdir().unwrap();
        crate::test_git::init_repo_with_commit(dir.path());
        write_claude_agent_with_session(dir.path(), "aa-00000001", "running", "ses_claude");
        let mut backend = Backend::Fake(FakeBackend::default());

        let reports = stop_agent_with_backend(dir.path(), "aa-00000001", &mut backend)
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>();

        assert_eq!(agent_ids(&reports), vec!["aa-00000001"]);
        assert_eq!(
            backend.fake().abort_calls[0].session_id.as_deref(),
            Some("ses_claude")
        );
        assert_eq!(
            load_agent_report(dir.path(), "aa-00000001")
                .unwrap()
                .metadata
                .status,
            "aborted"
        );
    }

    #[test]
    fn agent_stop_direct_backends_handle_mixed_system_sessions() {
        let dir = tempdir().unwrap();
        crate::test_git::init_repo_with_commit(dir.path());
        write_agent_with_session(dir.path(), "aa-00000001", "running", Some("ses_open"));
        write_claude_agent_with_session(dir.path(), "aa-00000002", "running", "ses_claude");
        let mut opencode = Backend::Fake(FakeBackend::default());
        let mut claude = Backend::Fake(FakeBackend::default());

        let reports = [
            stop_agent_with_backend(dir.path(), "aa-00000001", &mut opencode).unwrap(),
            stop_agent_with_backend(dir.path(), "aa-00000002", &mut claude).unwrap(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

        assert_eq!(agent_ids(&reports), vec!["aa-00000001", "aa-00000002"]);
        assert_eq!(
            opencode.fake().abort_calls[0].session_id.as_deref(),
            Some("ses_open")
        );
        assert_eq!(
            claude.fake().abort_calls[0].session_id.as_deref(),
            Some("ses_claude")
        );
    }

    #[test]
    fn agent_stop_does_not_mark_aborted_when_claude_kill_fails() {
        let dir = tempdir().unwrap();
        crate::test_git::init_repo_with_commit(dir.path());
        write_claude_agent_with_session(dir.path(), "aa-00000001", "running", "ses_claude");
        let mut backend = Backend::Fake(FakeBackend {
            abort_error: Some("kill failed".to_string()),
            ..FakeBackend::default()
        });

        let error = stop_agent_with_backend(dir.path(), "aa-00000001", &mut backend).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(
            load_agent_report(dir.path(), "aa-00000001")
                .unwrap()
                .metadata
                .status,
            "running"
        );
    }

    #[test]
    fn agent_stop_does_not_mark_aborted_when_opencode_abort_fails() {
        let dir = tempdir().unwrap();
        crate::test_git::init_repo_with_commit(dir.path());
        write_agent_with_session(dir.path(), "aa-3881fda0", "running", Some("ses_123"));
        let mut backend = Backend::Fake(FakeBackend {
            abort_error: Some("abort failed".to_string()),
            ..FakeBackend::default()
        });

        let error = stop_agent_with_backend(dir.path(), "aa-3881fda0", &mut backend).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(
            load_agent_report(dir.path(), "aa-3881fda0")
                .unwrap()
                .metadata
                .status,
            "running"
        );
    }

    #[test]
    fn agent_stop_json_has_expected_shape() {
        let reports = vec![AgentReport {
            agent_id: "aa-3881fda0".to_string(),
            path: PathBuf::from("agents/aa-3881fda0/agent.md"),
            metadata: AgentMetadata {
                name: None,
                creation_date: "2026-06-18T15:00:34Z".to_string(),
                status: "aborted".to_string(),
                session_id: Some("ses_123".to_string()),
                system: None,
            },
            file_size: 456,
        }];

        assert_eq!(
            agent_stop_json(&AgentStopReport {
                stopped_agents: reports
                    .into_iter()
                    .map(|report| super::AgentStop {
                        report,
                        outcome: crate::agent::backend::AbortOutcome::Stopped
                    })
                    .collect(),
                commit: Some("abc123".to_string()),
            }),
            json!({
                "stopped_agents": [
                    {
                        "agent_id": "aa-3881fda0",
                        "path": "agents/aa-3881fda0/agent.md",
                        "metadata": {
                            "name": null,
                            "creation_date": "2026-06-18T15:00:34Z",
                            "status": "aborted",
                            "session_id": "ses_123",
                        },
                        "file_size": 456,
                    }
                ],
                "commit": "abc123",
            })
        );
    }

    fn agent_ids(reports: &[AgentReport]) -> Vec<&str> {
        reports
            .iter()
            .map(|report| report.agent_id.as_str())
            .collect()
    }

    fn write_agent(waap_root: &Path, agent_id: &str, status: &str) {
        write_agent_with_session(waap_root, agent_id, status, None);
    }

    fn write_agent_with_session(
        waap_root: &Path,
        agent_id: &str,
        status: &str,
        session_id: Option<&str>,
    ) {
        let session_id = session_id
            .map(|session_id| format!("session_id = \"{session_id}\"\n"))
            .unwrap_or_default();
        write_file(
            &waap_root.join(format!("agents/{agent_id}/agent.md")),
            &format!(
                "+++\ncreation_date = 2026-06-18T15:00:34Z\nrole = \"developer\"\nstatus = \"{status}\"\n{session_id}+++\n\n# Purpose\n"
            ),
        );
    }

    fn write_claude_agent_with_session(
        waap_root: &Path,
        agent_id: &str,
        status: &str,
        session_id: &str,
    ) {
        write_file(
            &waap_root.join(format!("agents/{agent_id}/agent.md")),
            &format!(
                "+++\ncreation_date = 2026-06-18T15:00:34Z\nrole = \"developer\"\nstatus = \"{status}\"\nsession_id = \"{session_id}\"\nsystem = \"claude\"\n+++\n\n# Purpose\n"
            ),
        );
    }

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
}
