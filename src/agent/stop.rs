use std::io;
use std::path::Path;

use serde_json::json;

use crate::agent::get::load_agent_report;
use crate::agent::{
    agent_report_json, print_agent_report_human, read_agent_record, write_agent_record,
    AgentReport, AgentStatus, AgentSystem,
};
use crate::claude::kill_claude_session;
use crate::cli::OutputFormat;
use crate::codex::signal_codex_run;
use crate::opencode::{abort_opencode_session, opencode_run_config_from_env};
use crate::record::{list_record_ids, WaapRecordKind};

pub(crate) fn print_agent_stop_report(
    output_format: &OutputFormat,
    reports: &[AgentReport],
    commit: Option<&str>,
) {
    match output_format {
        OutputFormat::Json => println!("{}", agent_stop_json(reports, commit)),
        OutputFormat::HumanReadable => {
            for report in reports {
                print_agent_report_human("Stopped agent", report);
            }
            if let Some(commit) = commit {
                println!("Commit: {commit}");
            }
        }
    }
}

pub(crate) fn agent_stop_json(reports: &[AgentReport], commit: Option<&str>) -> serde_json::Value {
    json!({
        "stopped_agents": reports.iter().map(agent_report_json).collect::<Vec<_>>(),
        "commit": commit,
    })
}

pub(crate) fn stop_agents_with_systems(
    waap_root: &Path,
    agent_id: Option<&str>,
) -> io::Result<Vec<AgentReport>> {
    let mut config = None;
    // The abort closure receives both `agent_id` and `session_id`: claude/opencode key their stop on
    // the `session_id`, while codex keys on the `agent_id` because `turn/interrupt` requires the live
    // JSON-RPC connection held only by the running `waap agent run` process, which is signalled by its
    // unique argv (see /specs/codex-agent-system.md §5).
    stop_agents(
        waap_root,
        agent_id,
        |system, agent_id, session_id| match system {
            AgentSystem::Opencode => {
                if config.is_none() {
                    config = Some(opencode_run_config_from_env(waap_root)?);
                }
                abort_opencode_session(config.as_ref().expect("config initialized"), session_id)
            }
            AgentSystem::Claude => kill_claude_session(session_id),
            AgentSystem::Codex => signal_codex_run(agent_id),
        },
    )
}

pub(crate) fn stop_agents(
    waap_root: &Path,
    agent_id: Option<&str>,
    mut abort: impl FnMut(&AgentSystem, &str, &str) -> io::Result<()>,
) -> io::Result<Vec<AgentReport>> {
    match agent_id {
        Some(agent_id) => stop_agent_if_running(waap_root, agent_id, &mut abort)
            .map(|report| report.into_iter().collect::<Vec<AgentReport>>()),
        None => {
            let mut reports = Vec::new();
            for agent_id in list_record_ids(waap_root, WaapRecordKind::Agent)? {
                if let Some(report) = stop_agent_if_running(waap_root, &agent_id, &mut abort)? {
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
    abort: &mut impl FnMut(&AgentSystem, &str, &str) -> io::Result<()>,
) -> io::Result<Option<AgentReport>> {
    let report = load_agent_report(waap_root, agent_id)?;
    if report.metadata.status != AgentStatus::Running.as_str() {
        return Ok(None);
    }

    let (mut metadata, body) = read_agent_record(waap_root, agent_id)?;
    if let Some(session_id) = &report.metadata.session_id {
        let system = metadata.system.as_ref().unwrap_or(&AgentSystem::Opencode);
        abort(system, agent_id, session_id)?;
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

    use super::{agent_stop_json, stop_agents};
    use crate::agent::get::load_agent_report;
    use crate::agent::{AgentMetadata, AgentReport, AgentSystem};

    #[test]
    fn agent_stop_stops_one_running_agent() {
        let dir = tempdir().unwrap();
        write_agent_with_session(dir.path(), "aa-3881fda0", "running", Some("ses_123"));
        write_agent(dir.path(), "aa-00000001", "running");

        let reports = stop_agents(dir.path(), Some("aa-3881fda0"), noop_abort).unwrap();

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

        let reports = stop_agents(dir.path(), None, noop_abort).unwrap();

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
        write_agent(dir.path(), "aa-00000001", "ready");
        write_agent(dir.path(), "aa-00000002", "running");
        write_agent(dir.path(), "aa-00000003", "completed");
        write_agent(dir.path(), "aa-00000004", "aborted");

        let reports = stop_agents(dir.path(), None, noop_abort).unwrap();

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

        let reports = stop_agents(dir.path(), Some("aa-3881fda0"), noop_abort).unwrap();

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

        let error = stop_agents(dir.path(), Some("not-an-agent"), noop_abort).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("not a valid agent id"));
    }

    #[test]
    fn agent_stop_reports_missing_agent() {
        let dir = tempdir().unwrap();

        let error = stop_agents(dir.path(), Some("aa-3881fda0"), noop_abort).unwrap_err();

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
        let mut aborted = Vec::new();

        let reports = stop_agents(dir.path(), None, |_system, _agent_id, session_id| {
            aborted.push(session_id.to_string());
            Ok(())
        })
        .unwrap();

        assert_eq!(agent_ids(&reports), vec!["aa-00000001"]);
        assert_eq!(aborted, vec!["ses_123"]);
    }

    #[test]
    fn agent_stop_kills_claude_process_instead_of_opencode_abort() {
        let dir = tempdir().unwrap();
        write_claude_agent_with_session(dir.path(), "aa-00000001", "running", "ses_claude");
        let mut aborted = Vec::new();
        let mut killed = Vec::new();

        let reports = stop_agents(dir.path(), None, |system, _agent_id, session_id| {
            match system {
                AgentSystem::Opencode => aborted.push(session_id.to_string()),
                AgentSystem::Claude => killed.push(session_id.to_string()),
                AgentSystem::Codex => {}
            }
            Ok(())
        })
        .unwrap();

        assert_eq!(agent_ids(&reports), vec!["aa-00000001"]);
        assert!(aborted.is_empty());
        assert_eq!(killed, vec!["ses_claude"]);
        assert_eq!(
            load_agent_report(dir.path(), "aa-00000001")
                .unwrap()
                .metadata
                .status,
            "aborted"
        );
    }

    #[test]
    fn agent_stop_does_not_mark_aborted_when_claude_kill_fails() {
        let dir = tempdir().unwrap();
        write_claude_agent_with_session(dir.path(), "aa-00000001", "running", "ses_claude");

        let error = stop_agents(
            dir.path(),
            Some("aa-00000001"),
            |_system, _agent_id, _session_id| Err(io::Error::other("kill failed")),
        )
        .unwrap_err();

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
        let mut signalled = Vec::new();

        let reports = stop_agents(dir.path(), None, |system, agent_id, session_id| {
            match system {
                // The codex arm keys on the agent id, not the session id (§5).
                AgentSystem::Codex => signalled.push(agent_id.to_string()),
                AgentSystem::Claude | AgentSystem::Opencode => {
                    signalled.push(session_id.to_string())
                }
            }
            Ok(())
        })
        .unwrap();

        assert_eq!(agent_ids(&reports), vec!["aa-00000001"]);
        assert_eq!(signalled, vec!["aa-00000001"]);
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

        let error = stop_agents(
            dir.path(),
            Some("aa-00000001"),
            |_system, _agent_id, _session_id| Err(io::Error::other("signal failed")),
        )
        .unwrap_err();

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

        let error = stop_agents(
            dir.path(),
            Some("aa-3881fda0"),
            |_system, _agent_id, _session_id| Err(io::Error::other("abort failed")),
        )
        .unwrap_err();

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
                creation_date: "2026-06-18T15:00:34Z".to_string(),
                status: "aborted".to_string(),
                session_id: Some("ses_123".to_string()),
                system: None,
            },
            file_size: 456,
        }];

        assert_eq!(
            agent_stop_json(&reports, Some("abc123")),
            json!({
                "stopped_agents": [
                    {
                        "agent_id": "aa-3881fda0",
                        "path": ".waap/agents/aa-3881fda0/agent.md",
                        "metadata": {
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

    fn noop_abort(_: &AgentSystem, _: &str, _: &str) -> io::Result<()> {
        Ok(())
    }
}
