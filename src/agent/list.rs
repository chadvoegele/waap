use std::io;
use std::path::Path;

use serde_json::json;

use crate::agent::get::load_agent_report;
use crate::agent::{AgentReport, AgentStatus};
use crate::cli::OutputFormat;
use crate::record::{list_record_ids, WaapRecordKind};

pub(crate) fn print_agent_list(output_format: &OutputFormat, reports: &[AgentReport]) {
    let agent_ids: Vec<&str> = reports
        .iter()
        .map(|report| report.agent_id.as_str())
        .collect();
    match output_format {
        OutputFormat::Json => println!("{}", agent_list_json(reports)),
        OutputFormat::HumanReadable => {
            for agent_id in agent_ids {
                println!("{agent_id}");
            }
        }
    }
}

pub(crate) fn agent_list_json(reports: &[AgentReport]) -> serde_json::Value {
    json!(reports
        .iter()
        .map(|report| report.agent_id.as_str())
        .collect::<Vec<_>>())
}

pub(crate) fn list_agents(
    repo_root: &Path,
    status: Option<&AgentStatus>,
) -> io::Result<Vec<AgentReport>> {
    let mut reports = Vec::new();
    for agent_id in list_record_ids(repo_root, WaapRecordKind::Agent)? {
        let report = load_agent_report(repo_root, &agent_id)?;
        if status.is_none_or(|status| report.metadata.status == status.as_str()) {
            reports.push(report);
        }
    }

    Ok(reports)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};

    use serde_json::json;
    use tempfile::tempdir;

    use super::{agent_list_json, list_agents};
    use crate::agent::{AgentMetadata, AgentReport, AgentStatus};

    #[test]
    fn agent_list_returns_sorted_agent_ids() {
        let dir = tempdir().unwrap();
        write_agent(dir.path(), "aa-ffffffff", "completed");
        write_agent(dir.path(), "aa-00000001", "ready");
        write_agent(dir.path(), "aa-10000000", "running");

        let reports = list_agents(dir.path(), None).unwrap();

        assert_eq!(
            agent_ids(&reports),
            vec!["aa-00000001", "aa-10000000", "aa-ffffffff"]
        );
    }

    #[test]
    fn agent_list_filters_by_status() {
        let dir = tempdir().unwrap();
        write_agent(dir.path(), "aa-00000001", "ready");
        write_agent(dir.path(), "aa-00000002", "completed");
        write_agent(dir.path(), "aa-00000003", "completed");

        let reports = list_agents(dir.path(), Some(&AgentStatus::Completed)).unwrap();

        assert_eq!(agent_ids(&reports), vec!["aa-00000002", "aa-00000003"]);
    }

    #[test]
    fn agent_list_handles_empty_agent_directories() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap/agents")).unwrap();

        let reports = list_agents(dir.path(), None).unwrap();

        assert!(reports.is_empty());
    }

    #[test]
    fn agent_list_handles_missing_agent_directories() {
        let dir = tempdir().unwrap();

        let reports = list_agents(dir.path(), None).unwrap();

        assert!(reports.is_empty());
    }

    #[test]
    fn agent_list_rejects_non_directory_entries() {
        let dir = tempdir().unwrap();
        write_file(&dir.path().join(".waap/agents/not-a-directory"), "oops");

        let error = list_agents(dir.path(), None).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("must be an agent directory"));
    }

    #[test]
    fn agent_list_validates_agent_frontmatter() {
        let dir = tempdir().unwrap();
        write_file(
            &dir.path().join(".waap/agents/aa-3881fda0/agent.md"),
            "+++
creation_date = 2026-06-18T15:00:34Z
role = \"developer\"
status = \"pending\"
+++
",
        );

        let error = list_agents(dir.path(), None).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("status has invalid value"));
    }

    #[test]
    fn agent_list_json_has_expected_shape() {
        let reports = vec![
            AgentReport {
                agent_id: "aa-00000001".to_string(),
                path: PathBuf::from(".waap/agents/aa-00000001/agent.md"),
                metadata: AgentMetadata {
                    creation_date: "2026-06-18T15:00:34Z".to_string(),
                    status: "ready".to_string(),
                    session_id: None,
                    system: None,
                },
                file_size: 123,
            },
            AgentReport {
                agent_id: "aa-00000002".to_string(),
                path: PathBuf::from(".waap/agents/aa-00000002/agent.md"),
                metadata: AgentMetadata {
                    creation_date: "2026-06-18T15:00:34Z".to_string(),
                    status: "completed".to_string(),
                    session_id: Some("ses_123".to_string()),
                    system: None,
                },
                file_size: 456,
            },
        ];

        assert_eq!(
            agent_list_json(&reports),
            json!(["aa-00000001", "aa-00000002"])
        );
    }

    fn agent_ids(reports: &[AgentReport]) -> Vec<&str> {
        reports
            .iter()
            .map(|report| report.agent_id.as_str())
            .collect()
    }

    fn write_agent(repo_root: &Path, agent_id: &str, status: &str) {
        write_file(
            &repo_root.join(format!(".waap/agents/{agent_id}/agent.md")),
            &format!(
                "+++\ncreation_date = 2026-06-18T15:00:34Z\nrole = \"developer\"\nstatus = \"{status}\"\n+++\n\n# Purpose\n"
            ),
        );
    }

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
}
