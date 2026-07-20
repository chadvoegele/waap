use std::io;
use std::path::Path;

use crate::agent::get::load_agent_report;
use crate::agent::{
    agent_report_json, print_agent_report_human, read_agent_record, transition_agent_status,
    write_agent_record, AgentReport, AgentStatus,
};
use crate::cli::OutputFormat;
use crate::git::{commit_paths, Committed};

pub(crate) fn print_updated_agent_report(
    output_format: &OutputFormat,
    committed: &Committed<AgentReport>,
) {
    let report = &committed.value;
    match output_format {
        OutputFormat::Json => {
            let mut value = agent_report_json(report);
            value["commit"] = serde_json::json!(committed.commit);
            println!("{value}");
        }
        OutputFormat::HumanReadable => {
            print_agent_report_human("Updated agent", report);
            println!("Commit: {}", committed.commit);
        }
    }
}

pub(crate) fn update_agent(
    waap_root: &Path,
    agent_id: &str,
    set_status: Option<&AgentStatus>,
    set_session_id: Option<&str>,
) -> io::Result<Committed<AgentReport>> {
    let report = update_agent_record(waap_root, agent_id, set_status, set_session_id)?;
    let commit = commit_paths(
        waap_root,
        &[report.path.as_path()],
        &format!("waap agent update {}", report.agent_id),
    )
    .map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("failed to commit waap state change: {error}"),
        )
    })?;

    Ok(Committed {
        value: report,
        commit,
    })
}

fn update_agent_record(
    waap_root: &Path,
    agent_id: &str,
    set_status: Option<&AgentStatus>,
    set_session_id: Option<&str>,
) -> io::Result<AgentReport> {
    if set_status.is_none() && set_session_id.is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "at least one of --set-status or --set-session-id is required",
        ));
    }

    let (mut metadata, body) = read_agent_record(waap_root, agent_id)?;
    if metadata.status == AgentStatus::Running.as_str() && set_status == Some(&AgentStatus::Aborted)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "agent {agent_id} is running; use `waap agent stop --agent-id {agent_id}` to abort it"
            ),
        ));
    }
    if set_session_id.is_some()
        && (metadata.status != AgentStatus::Running.as_str() || set_status.is_some())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("agent {agent_id} must remain running to assign a session"),
        ));
    }
    if set_session_id.is_some() {
        if let Some(existing_session_id) = &metadata.session_id {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("agent {agent_id} already has session id {existing_session_id}"),
            ));
        }
    }
    if let Some(status) = set_status {
        transition_agent_status(&mut metadata, *status)?;
    }
    if let Some(session_id) = set_session_id {
        metadata.session_id = Some(session_id.to_string());
    }
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

    use super::update_agent_record;
    use crate::agent::{agent_report_json, AgentMetadata, AgentReport, AgentStatus};

    #[test]
    fn agent_update_requires_at_least_one_update_field() {
        let dir = tempdir().unwrap();
        write_file(
            &dir.path().join(".waap/agents/aa-3881fda0/agent.md"),
            "+++\ncreation_date = 2026-06-18T15:00:34Z\nrole = \"developer\"\nstatus = \"ready\"\n+++\n\n# Purpose\n",
        );

        let error = update_agent_record(dir.path(), "aa-3881fda0", None, None).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("at least one"));
    }

    #[test]
    fn agent_update_reports_missing_agents() {
        let dir = tempdir().unwrap();

        let error =
            update_agent_record(dir.path(), "aa-3881fda0", Some(&AgentStatus::Running), None)
                .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(error
            .to_string()
            .contains(".waap/agents/aa-3881fda0/agent.md"));
    }

    #[test]
    fn agent_update_valid_transition_preserves_frontmatter_and_body() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".waap/agents/aa-3881fda0/agent.md");
        write_file(
            &path,
            "+++\ncreation_date = 2026-06-18T15:00:34Z\nrole = \"planner\"\nstatus = \"running\"\n+++\n\n# Purpose\nDo work\n",
        );

        let report = update_agent_record(
            dir.path(),
            "aa-3881fda0",
            Some(&AgentStatus::Completed),
            None,
        )
        .unwrap();
        let contents = fs::read_to_string(&path).unwrap();

        assert_eq!(report.agent_id, "aa-3881fda0");
        assert_eq!(report.metadata.creation_date, "2026-06-18T15:00:34Z");
        assert_eq!(report.metadata.status, "completed");
        assert_eq!(report.metadata.session_id, None);
        assert_eq!(report.file_size, contents.len() as u64);
        assert!(contents.contains("creation_date = 2026-06-18T15:00:34Z\n"));
        assert!(!contents.contains("role ="));
        assert!(contents.contains("status = \"completed\"\n"));
        assert!(contents.contains("status = \"completed\"\n+++\n\n# Purpose\nDo work\n"));
    }

    #[test]
    fn agent_update_rejects_aborting_running_agent_without_modifying_record() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".waap/agents/aa-3881fda0/agent.md");
        let original = "+++\ncreation_date = 2026-06-18T15:00:34Z\nstatus = \"running\"\nsession_id = \"ses_123\"\n+++\n\n# Purpose\n";
        write_file(&path, original);

        let error =
            update_agent_record(dir.path(), "aa-3881fda0", Some(&AgentStatus::Aborted), None)
                .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error
            .to_string()
            .contains("waap agent stop --agent-id aa-3881fda0"));
        assert_eq!(fs::read_to_string(path).unwrap(), original);
    }

    #[test]
    fn agent_update_allows_aborting_ready_agent() {
        let dir = tempdir().unwrap();
        write_file(
            &dir.path().join(".waap/agents/aa-3881fda0/agent.md"),
            "+++\ncreation_date = 2026-06-18T15:00:34Z\nstatus = \"ready\"\n+++\n\n# Purpose\n",
        );

        let report =
            update_agent_record(dir.path(), "aa-3881fda0", Some(&AgentStatus::Aborted), None)
                .unwrap();

        assert_eq!(report.metadata.status, "aborted");
    }

    #[test]
    fn agent_update_rejects_existing_session_id() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".waap/agents/aa-3881fda0/agent.md");
        write_file(
            &path,
            "+++\ncreation_date = 2026-06-18T15:00:34Z\nrole = \"developer\"\nstatus = \"running\"\nsession_id = \"ses_old\"\n+++\n\n# Purpose\n",
        );

        let error =
            update_agent_record(dir.path(), "aa-3881fda0", None, Some("ses_new")).unwrap_err();
        let contents = fs::read_to_string(&path).unwrap();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(error.to_string().contains("already has session id ses_old"));
        assert!(contents.contains("session_id = \"ses_old\"\n"));
    }

    #[test]
    fn agent_update_rejects_invalid_same_and_terminal_transitions() {
        for (current, next) in [
            ("ready", AgentStatus::Ready),
            ("ready", AgentStatus::Completed),
            ("running", AgentStatus::Running),
            ("completed", AgentStatus::Failed),
            ("failed", AgentStatus::Aborted),
            ("aborted", AgentStatus::Running),
        ] {
            let dir = tempdir().unwrap();
            let path = dir.path().join(".waap/agents/aa-3881fda0/agent.md");
            write_file(
                &path,
                &format!(
                    "+++\ncreation_date = 2026-06-18T15:00:34Z\nstatus = \"{current}\"\n+++\n\n# Purpose\n"
                ),
            );

            let error =
                update_agent_record(dir.path(), "aa-3881fda0", Some(&next), None).unwrap_err();

            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            assert_eq!(
                fs::read_to_string(path).unwrap(),
                format!(
                    "+++\ncreation_date = 2026-06-18T15:00:34Z\nstatus = \"{current}\"\n+++\n\n# Purpose\n"
                )
            );
        }
    }

    #[test]
    fn agent_update_assigns_session_only_to_running_agent_without_session() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".waap/agents/aa-3881fda0/agent.md");
        write_file(
            &path,
            "+++\ncreation_date = 2026-06-18T15:00:34Z\nstatus = \"running\"\n+++\n\n# Purpose\n",
        );

        let report = update_agent_record(dir.path(), "aa-3881fda0", None, Some("ses_new")).unwrap();

        assert_eq!(report.metadata.status, "running");
        assert_eq!(report.metadata.session_id.as_deref(), Some("ses_new"));
    }

    #[test]
    fn agent_update_rejects_session_assignment_when_status_changes() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".waap/agents/aa-3881fda0/agent.md");
        write_file(
            &path,
            "+++\ncreation_date = 2026-06-18T15:00:34Z\nstatus = \"running\"\n+++\n\n# Purpose\n",
        );

        let error = update_agent_record(
            dir.path(),
            "aa-3881fda0",
            Some(&AgentStatus::Completed),
            Some("ses_new"),
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("must remain running"));
        assert!(!fs::read_to_string(path).unwrap().contains("session_id"));
    }

    #[test]
    fn agent_update_json_output_includes_updated_metadata() {
        let report = AgentReport {
            agent_id: "aa-3881fda0".to_string(),
            path: PathBuf::from(".waap/agents/aa-3881fda0/agent.md"),
            metadata: AgentMetadata {
                name: None,
                creation_date: "2026-06-18T15:00:34Z".to_string(),
                status: "completed".to_string(),
                session_id: Some("ses_123".to_string()),
                system: None,
            },
            file_size: 789,
        };

        assert_eq!(
            agent_report_json(&report),
            json!({
                "agent_id": "aa-3881fda0",
                "path": ".waap/agents/aa-3881fda0/agent.md",
                "metadata": {
                    "name": null,
                    "creation_date": "2026-06-18T15:00:34Z",
                    "status": "completed",
                    "session_id": "ses_123",
                },
                "file_size": 789,
            })
        );
    }

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
}
