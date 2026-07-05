use std::fs;
use std::io::{self, Read};
use std::path::Path;

use crate::agent::{
    agent_path, agent_report_json, available_agent_id, print_agent_report_human,
    write_agent_record, AgentMetadata, AgentReport,
};
use crate::cli::OutputFormat;
use crate::ids::current_toml_datetime;
use crate::record::{require_initialized_project, WaapRecordKind};

pub(crate) fn print_created_agent_report(
    output_format: &OutputFormat,
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
            print_agent_report_human("Created agent", report);
            println!("Commit: {commit}");
        }
    }
}

pub(crate) fn create_agent(waap_root: &Path, agent_id: Option<&str>) -> io::Result<AgentReport> {
    let mut markdown = String::new();
    io::stdin()
        .read_to_string(&mut markdown)
        .map_err(|error| io::Error::new(error.kind(), format!("failed to read stdin: {error}")))?;

    create_agent_with_markdown(waap_root, agent_id, &markdown)
}

pub(crate) fn create_agent_with_markdown(
    waap_root: &Path,
    agent_id: Option<&str>,
    markdown: &str,
) -> io::Result<AgentReport> {
    require_initialized_project(waap_root)?;

    let agents_dir = WaapRecordKind::Agent.root_path(waap_root);
    let agent_id = match agent_id {
        Some(agent_id) => {
            validate_custom_agent_id(agent_id)?;
            if agents_dir.join(agent_id).exists() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("agent id {agent_id:?} already exists"),
                ));
            }
            agent_id.to_string()
        }
        None => available_agent_id(&agents_dir)?,
    };

    let metadata = AgentMetadata {
        creation_date: current_toml_datetime(),
        status: "ready".to_string(),
        session_id: None,
        system: None,
    };
    write_agent_record(waap_root, &agent_id, &metadata, &format!("\n{markdown}"))?;
    let path = agent_path(waap_root, &agent_id);
    let file_size = fs::metadata(&path)?.len();

    Ok(AgentReport {
        agent_id,
        path,
        metadata,
        file_size,
    })
}

fn validate_custom_agent_id(agent_id: &str) -> io::Result<()> {
    if crate::agent::is_agent_id(agent_id) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "agent id {agent_id:?} must be non-empty, fewer than 64 characters, and contain only lowercase ASCII letters, digits, hyphen, or underscore"
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::create_agent_with_markdown;
    use crate::agent::is_agent_id;
    use crate::check::check_waap;

    #[test]
    fn create_agent_writes_frontmatter_and_stdin_content() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap")).unwrap();

        let report =
            create_agent_with_markdown(dir.path(), None, "# Purpose\nPlan things\n").unwrap();
        let contents = fs::read_to_string(&report.path).unwrap();

        assert!(is_agent_id(&report.agent_id));
        assert_eq!(report.metadata.status, "ready");
        assert_eq!(report.metadata.session_id, None);
        assert_eq!(report.file_size, contents.len() as u64);
        assert!(contents.starts_with("+++\ncreation_date = "));
        assert!(!contents.contains("role ="));
        assert!(contents.contains("\nstatus = \"ready\"\n+++\n\n# Purpose\nPlan things\n"));
        assert!(check_waap(dir.path()).is_empty());
    }

    #[test]
    fn create_agent_errors_when_project_not_initialized() {
        let dir = tempdir().unwrap();

        let err = create_agent_with_markdown(dir.path(), None, "# Purpose\n").unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        assert!(err.to_string().contains("waap init"));
    }

    #[test]
    fn create_agent_uses_valid_custom_agent_id() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap")).unwrap();

        let report = create_agent_with_markdown(
            dir.path(),
            Some("custom-agent_123"),
            "# Purpose\nPlan things\n",
        )
        .unwrap();

        assert_eq!(report.agent_id, "custom-agent_123");
        assert!(report
            .path
            .ends_with(".waap/agents/custom-agent_123/agent.md"));
        assert!(check_waap(dir.path()).is_empty());
    }

    #[test]
    fn create_agent_rejects_invalid_custom_agent_id() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap")).unwrap();

        for agent_id in ["", "Upper", "has space", "has/slash", &"a".repeat(64)] {
            let err =
                create_agent_with_markdown(dir.path(), Some(agent_id), "# Purpose\n").unwrap_err();

            assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        }
    }

    #[test]
    fn create_agent_rejects_duplicate_custom_agent_id() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap/agents/custom-id")).unwrap();

        let err =
            create_agent_with_markdown(dir.path(), Some("custom-id"), "# Purpose\n").unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(!dir.path().join(".waap/agents/custom-id/agent.md").exists());
    }
}
