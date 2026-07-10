use std::io;
use std::path::Path;

use serde_json::json;

use super::backend::{AbortContext, BackendRegistry, BackendResolver};
use crate::agent::get::load_agent_report;
use crate::agent::{
    agent_report_json, print_agent_report_human, read_agent_record, write_agent_record,
    AgentReport, AgentStatus,
};
use crate::cli::OutputFormat;
use crate::git::commit_paths;
use crate::record::{list_record_ids, WaapRecordKind};

#[derive(Debug)]
pub(crate) struct AgentStopReport {
    pub(crate) stopped_agents: Vec<AgentReport>,
    pub(crate) commit: Option<String>,
}

pub(crate) fn print_agent_stop_report(output_format: &OutputFormat, report: &AgentStopReport) {
    match output_format {
        OutputFormat::Json => println!("{}", agent_stop_json(report)),
        OutputFormat::HumanReadable => {
            for agent in &report.stopped_agents {
                print_agent_report_human("Stopped agent", agent);
            }
            if let Some(commit) = &report.commit {
                println!("Commit: {commit}");
            }
        }
    }
}

fn agent_stop_json(report: &AgentStopReport) -> serde_json::Value {
    json!({
        "stopped_agents": report.stopped_agents.iter().map(agent_report_json).collect::<Vec<_>>(),
        "commit": report.commit,
    })
}

pub(crate) fn stop_agents_with_systems(
    waap_root: &Path,
    agent_id: Option<&str>,
) -> io::Result<AgentStopReport> {
    let mut backends = BackendRegistry::default();
    let stopped_agents = stop_agents(waap_root, agent_id, &mut backends)?;

    let commit = if stopped_agents.is_empty() {
        None
    } else {
        let paths: Vec<&Path> = stopped_agents
            .iter()
            .map(|report| report.path.as_path())
            .collect();
        let ids: Vec<&str> = stopped_agents
            .iter()
            .map(|report| report.agent_id.as_str())
            .collect();
        Some(
            commit_paths(
                waap_root,
                &paths,
                &format!("waap agent stop {}", ids.join(" ")),
            )
            .map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("failed to commit waap state change: {error}"),
                )
            })?,
        )
    };

    Ok(AgentStopReport {
        stopped_agents,
        commit,
    })
}

pub(super) fn stop_agents(
    waap_root: &Path,
    agent_id: Option<&str>,
    backends: &mut dyn BackendResolver,
) -> io::Result<Vec<AgentReport>> {
    match agent_id {
        Some(agent_id) => stop_agent_if_running(waap_root, agent_id, backends)
            .map(|report| report.into_iter().collect::<Vec<AgentReport>>()),
        None => {
            let mut reports = Vec::new();
            for agent_id in list_record_ids(waap_root, WaapRecordKind::Agent)? {
                if let Some(report) = stop_agent_if_running(waap_root, &agent_id, backends)? {
                    reports.push(report);
                }
            }
            Ok(reports)
        }
    }
}

fn stop_agent_if_running(
    waap_root: &Path,
    agent_id: &str,
    backends: &mut dyn BackendResolver,
) -> io::Result<Option<AgentReport>> {
    let report = load_agent_report(waap_root, agent_id)?;
    if report.metadata.status != AgentStatus::Running.as_str() {
        return Ok(None);
    }

    let (mut metadata, body) = read_agent_record(waap_root, agent_id)?;
    if let Some(session_id) = &report.metadata.session_id {
        let system = metadata.system.clone().unwrap_or_default();
        backends.resolve(&system)?.abort(AbortContext {
            waap_root,
            agent_id,
            session_id,
        })?;
    }

    metadata.status = AgentStatus::Aborted.as_str().to_string();
    write_agent_record(waap_root, agent_id, &metadata, &body)?;

    load_agent_report(waap_root, agent_id).map(Some)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};

    use serde_json::json;
    use tempfile::tempdir;

    use super::{agent_stop_json, stop_agents, AgentStopReport};
    use crate::agent::backend::fake::FakeResolver;
    use crate::agent::get::load_agent_report;
    use crate::agent::{AgentMetadata, AgentReport, AgentSystem};

    #[test]
    fn agent_stop_stops_one_running_agent() {
        let dir = tempdir().unwrap();
        write_agent_with_session(dir.path(), "aa-3881fda0", "running", Some("ses_123"));
        write_agent(dir.path(), "aa-00000001", "running");

        let mut resolver = FakeResolver::default();
        let reports = stop_agents(dir.path(), Some("aa-3881fda0"), &mut resolver).unwrap();

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
        write_agent(dir.path(), "aa-00000003", "running");
        write_agent(dir.path(), "aa-00000001", "running");

        let mut resolver = FakeResolver::default();
        let reports = stop_agents(dir.path(), None, &mut resolver).unwrap();

        assert_eq!(agent_ids(&reports), vec!["aa-00000001", "aa-00000003"]);
        assert!(resolver.resolved.is_empty());
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
        write_agent(dir.path(), "aa-00000001", "ready");
        write_agent(dir.path(), "aa-00000002", "running");
        write_agent(dir.path(), "aa-00000003", "completed");
        write_agent(dir.path(), "aa-00000004", "aborted");

        let mut resolver = FakeResolver::default();
        let reports = stop_agents(dir.path(), None, &mut resolver).unwrap();

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
    }

    #[test]
    fn agent_stop_existing_non_running_agent_is_noop() {
        let dir = tempdir().unwrap();
        write_agent(dir.path(), "aa-3881fda0", "completed");

        let mut resolver = FakeResolver::default();
        let reports = stop_agents(dir.path(), Some("aa-3881fda0"), &mut resolver).unwrap();

        assert!(reports.is_empty());
        assert_eq!(
            load_agent_report(dir.path(), "aa-3881fda0")
                .unwrap()
                .metadata
                .status,
            "completed"
        );
    }

    #[test]
    fn agent_stop_reports_invalid_agent_id() {
        let dir = tempdir().unwrap();

        let mut resolver = FakeResolver::default();
        let error = stop_agents(dir.path(), Some("not an agent"), &mut resolver).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("not a valid agent id"));
    }

    #[test]
    fn agent_stop_reports_missing_agent() {
        let dir = tempdir().unwrap();

        let mut resolver = FakeResolver::default();
        let error = stop_agents(dir.path(), Some("aa-3881fda0"), &mut resolver).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(error
            .to_string()
            .contains(".waap/agents/aa-3881fda0/agent.md"));
    }

    #[test]
    fn agent_stop_aborts_opencode_sessions_for_running_agents() {
        let dir = tempdir().unwrap();
        write_agent_with_session(dir.path(), "aa-00000001", "running", Some("ses_123"));
        write_agent_with_session(dir.path(), "aa-00000002", "ready", Some("ses_ready"));
        let mut resolver = FakeResolver::default();

        let reports = stop_agents(dir.path(), None, &mut resolver).unwrap();

        assert_eq!(agent_ids(&reports), vec!["aa-00000001"]);
        assert_eq!(resolver.resolved, vec![AgentSystem::Opencode]);
        assert_eq!(resolver.opencode.abort_calls.len(), 1);
        let call = &resolver.opencode.abort_calls[0];
        assert_eq!(call.waap_root, dir.path());
        assert_eq!(call.agent_id, "aa-00000001");
        assert_eq!(call.session_id, "ses_123");
    }

    #[test]
    fn agent_stop_kills_claude_process_instead_of_opencode_abort() {
        let dir = tempdir().unwrap();
        write_claude_agent_with_session(dir.path(), "aa-00000001", "running", "ses_claude");
        let mut resolver = FakeResolver::default();

        let reports = stop_agents(dir.path(), None, &mut resolver).unwrap();

        assert_eq!(agent_ids(&reports), vec!["aa-00000001"]);
        assert_eq!(resolver.resolved, vec![AgentSystem::Claude]);
        assert!(resolver.opencode.abort_calls.is_empty());
        assert_eq!(resolver.claude.abort_calls[0].session_id, "ses_claude");
        assert_eq!(
            load_agent_report(dir.path(), "aa-00000001")
                .unwrap()
                .metadata
                .status,
            "aborted"
        );
    }

    #[test]
    fn agent_stop_mixed_systems_resolve_their_persisted_backends_in_order() {
        let dir = tempdir().unwrap();
        write_agent_with_session(dir.path(), "aa-00000001", "running", Some("ses_open"));
        write_claude_agent_with_session(dir.path(), "aa-00000002", "running", "ses_claude");
        write_codex_agent_with_session(dir.path(), "aa-00000003", "running", "th_codex");
        let mut resolver = FakeResolver::default();

        let reports = stop_agents(dir.path(), None, &mut resolver).unwrap();

        assert_eq!(
            resolver.resolved,
            vec![
                AgentSystem::Opencode,
                AgentSystem::Claude,
                AgentSystem::Codex
            ]
        );
        assert_eq!(
            agent_ids(&reports),
            vec!["aa-00000001", "aa-00000002", "aa-00000003"]
        );
    }

    #[test]
    fn agent_stop_resolver_failure_leaves_record_running() {
        let dir = tempdir().unwrap();
        write_claude_agent_with_session(dir.path(), "aa-00000001", "running", "ses_claude");
        let mut resolver = FakeResolver {
            resolve_error: Some(AgentSystem::Claude),
            ..FakeResolver::default()
        };

        let error = stop_agents(dir.path(), Some("aa-00000001"), &mut resolver).unwrap_err();

        assert_eq!(error.to_string(), "backend resolution failed");
        assert_eq!(
            load_agent_report(dir.path(), "aa-00000001")
                .unwrap()
                .metadata
                .status,
            "running"
        );
    }

    #[test]
    fn agent_stop_does_not_mark_aborted_when_claude_kill_fails() {
        let dir = tempdir().unwrap();
        write_claude_agent_with_session(dir.path(), "aa-00000001", "running", "ses_claude");
        let mut resolver = FakeResolver::default();
        resolver.claude.abort_error = Some("kill failed".to_string());

        let error = stop_agents(dir.path(), Some("aa-00000001"), &mut resolver).unwrap_err();

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
    fn agent_stop_signals_codex_run_with_agent_id_not_session_id() {
        let dir = tempdir().unwrap();
        write_codex_agent_with_session(dir.path(), "aa-00000001", "running", "th_codex");
        let mut resolver = FakeResolver::default();

        let reports = stop_agents(dir.path(), None, &mut resolver).unwrap();

        assert_eq!(agent_ids(&reports), vec!["aa-00000001"]);
        assert_eq!(resolver.resolved, vec![AgentSystem::Codex]);
        assert_eq!(resolver.codex.abort_calls[0].agent_id, "aa-00000001");
        assert_eq!(resolver.codex.abort_calls[0].session_id, "th_codex");
        assert_eq!(
            load_agent_report(dir.path(), "aa-00000001")
                .unwrap()
                .metadata
                .status,
            "aborted"
        );
    }

    #[test]
    fn agent_stop_does_not_mark_aborted_when_codex_signal_fails() {
        let dir = tempdir().unwrap();
        write_codex_agent_with_session(dir.path(), "aa-00000001", "running", "th_codex");
        let mut resolver = FakeResolver::default();
        resolver.codex.abort_error = Some("signal failed".to_string());

        let error = stop_agents(dir.path(), Some("aa-00000001"), &mut resolver).unwrap_err();

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
        write_agent_with_session(dir.path(), "aa-3881fda0", "running", Some("ses_123"));
        let mut resolver = FakeResolver::default();
        resolver.opencode.abort_error = Some("abort failed".to_string());

        let error = stop_agents(dir.path(), Some("aa-3881fda0"), &mut resolver).unwrap_err();

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
            path: PathBuf::from(".waap/agents/aa-3881fda0/agent.md"),
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
                stopped_agents: reports,
                commit: Some("abc123".to_string()),
            }),
            json!({
                "stopped_agents": [
                    {
                        "agent_id": "aa-3881fda0",
                        "path": ".waap/agents/aa-3881fda0/agent.md",
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
            &waap_root.join(format!(".waap/agents/{agent_id}/agent.md")),
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
            &waap_root.join(format!(".waap/agents/{agent_id}/agent.md")),
            &format!(
                "+++\ncreation_date = 2026-06-18T15:00:34Z\nrole = \"developer\"\nstatus = \"{status}\"\nsession_id = \"{session_id}\"\nsystem = \"claude\"\n+++\n\n# Purpose\n"
            ),
        );
    }

    fn write_codex_agent_with_session(
        waap_root: &Path,
        agent_id: &str,
        status: &str,
        session_id: &str,
    ) {
        write_file(
            &waap_root.join(format!(".waap/agents/{agent_id}/agent.md")),
            &format!(
                "+++\ncreation_date = 2026-06-18T15:00:34Z\nrole = \"developer\"\nstatus = \"{status}\"\nsession_id = \"{session_id}\"\nsystem = \"codex\"\n+++\n\n# Purpose\n"
            ),
        );
    }

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
}
