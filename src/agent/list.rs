use std::io;
use std::path::Path;

use serde_json::json;

use crate::agent::get::load_agent_report;
use crate::agent::{AgentReport, AgentStatus};
use crate::cli::OutputFormat;
use crate::record::{list_record_ids, WaapRecordKind};

const AGENT_ID_HEADER: &str = "Agent ID";
const STATUS_HEADER: &str = "Status";

pub(crate) fn print_agent_list(output_format: &OutputFormat, reports: &[AgentReport]) {
    match output_format {
        OutputFormat::Json => println!("{}", agent_list_json(reports)),
        OutputFormat::HumanReadable => {
            for line in agent_list_human_lines(reports) {
                println!("{line}");
            }
        }
    }
}

fn agent_list_human_lines(reports: &[AgentReport]) -> Vec<String> {
    if reports.is_empty() {
        return Vec::new();
    }

    let id_width = reports
        .iter()
        .map(|report| report.agent_id.len())
        .max()
        .unwrap_or(0)
        .max(AGENT_ID_HEADER.len());

    let id_separator = "-".repeat(AGENT_ID_HEADER.len());
    let status_separator = "-".repeat(STATUS_HEADER.len());

    let mut lines = Vec::with_capacity(reports.len() + 2);
    lines.push(format!("{AGENT_ID_HEADER:id_width$}  {STATUS_HEADER}"));
    lines.push(format!("{id_separator:id_width$}  {status_separator}"));
    lines.extend(reports.iter().map(|report| {
        let id = &report.agent_id;
        let status = &report.metadata.status;
        format!("{id:id_width$}  {status}")
    }));

    lines
}

fn agent_list_json(reports: &[AgentReport]) -> serde_json::Value {
    json!(reports
        .iter()
        .map(|report| json!({
            "agent_id": report.agent_id.as_str(),
            "status": report.metadata.status.as_str(),
        }))
        .collect::<Vec<_>>())
}

pub(crate) fn list_agents(
    waap_root: &Path,
    status: Option<&AgentStatus>,
) -> io::Result<Vec<AgentReport>> {
    let mut reports = Vec::new();
    for agent_id in list_record_ids(waap_root, WaapRecordKind::Agent)? {
        let report = load_agent_report(waap_root, &agent_id)?;
        if status.is_none_or(|status| report.metadata.status == status.as_str()) {
            reports.push(report);
        }
    }

    reports.sort_by(|a, b| a.metadata.creation_date.cmp(&b.metadata.creation_date));

    Ok(reports)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};

    use serde_json::json;
    use tempfile::tempdir;

    use super::{agent_list_human_lines, agent_list_json, list_agents};
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
    fn agent_list_orders_by_creation_date_not_id() {
        let dir = tempdir().unwrap();
        write_agent_with_creation_date(dir.path(), "aa-ffffffff", "ready", "2026-06-20T00:00:00Z");
        write_agent_with_creation_date(dir.path(), "aa-00000001", "ready", "2026-06-18T00:00:00Z");
        write_agent_with_creation_date(dir.path(), "aa-10000000", "ready", "2026-06-19T00:00:00Z");

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
    fn agent_list_filters_failed_status() {
        let dir = tempdir().unwrap();
        write_agent(dir.path(), "aa-00000001", "failed");
        write_agent(dir.path(), "aa-00000002", "completed");

        let reports = list_agents(dir.path(), Some(&AgentStatus::Failed)).unwrap();

        assert_eq!(agent_ids(&reports), vec!["aa-00000001"]);
        assert_eq!(reports[0].metadata.status, "failed");
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
                    name: None,
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
                    name: None,
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
            json!([
                {"agent_id": "aa-00000001", "status": "ready"},
                {"agent_id": "aa-00000002", "status": "completed"},
            ])
        );
    }

    #[test]
    fn agent_list_human_lines_includes_heading_and_status() {
        let reports = vec![
            report("aa-00000001", "ready"),
            report("aa-000000002", "completed"),
        ];

        let lines = agent_list_human_lines(&reports);

        assert_eq!(
            lines,
            vec![
                "Agent ID      Status".to_string(),
                "--------      ------".to_string(),
                "aa-00000001   ready".to_string(),
                "aa-000000002  completed".to_string(),
            ]
        );
    }

    #[test]
    fn agent_list_human_lines_aligns_using_header_width_when_ids_are_short() {
        let reports = vec![report("aa-1", "ready")];

        let lines = agent_list_human_lines(&reports);

        assert_eq!(
            lines,
            vec![
                "Agent ID  Status".to_string(),
                "--------  ------".to_string(),
                "aa-1      ready".to_string()
            ]
        );
    }

    #[test]
    fn agent_list_human_lines_empty_when_no_entries() {
        let lines = agent_list_human_lines(&[]);

        assert!(lines.is_empty());
    }

    fn report(agent_id: &str, status: &str) -> AgentReport {
        AgentReport {
            agent_id: agent_id.to_string(),
            path: PathBuf::from(format!(".waap/agents/{agent_id}/agent.md")),
            metadata: AgentMetadata {
                name: None,
                creation_date: "2026-06-18T15:00:34Z".to_string(),
                status: status.to_string(),
                session_id: None,
                system: None,
            },
            file_size: 0,
        }
    }

    fn agent_ids(reports: &[AgentReport]) -> Vec<&str> {
        reports
            .iter()
            .map(|report| report.agent_id.as_str())
            .collect()
    }

    fn write_agent(waap_root: &Path, agent_id: &str, status: &str) {
        write_agent_with_creation_date(waap_root, agent_id, status, "2026-06-18T15:00:34Z");
    }

    fn write_agent_with_creation_date(
        waap_root: &Path,
        agent_id: &str,
        status: &str,
        creation_date: &str,
    ) {
        write_file(
            &waap_root.join(format!(".waap/agents/{agent_id}/agent.md")),
            &format!(
                "+++\ncreation_date = {creation_date}\nrole = \"developer\"\nstatus = \"{status}\"\n+++\n\n# Purpose\n"
            ),
        );
    }

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
}
