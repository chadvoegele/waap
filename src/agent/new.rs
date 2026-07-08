use std::fs;
use std::io::{self, Read};
use std::path::Path;

use crate::agent::{
    agent_path, agent_report_json, available_agent_id, print_agent_report_human,
    write_agent_record, AgentMetadata, AgentReport,
};
use crate::cli::OutputFormat;
use crate::git::{commit_paths, Committed};
use crate::record::{require_initialized_project, WaapRecordKind};
use crate::toml::current_toml_datetime;

pub(crate) fn print_created_agent_report(
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
            print_agent_report_human("Created agent", report);
            println!("Commit: {}", committed.commit);
        }
    }
}

pub(crate) fn create_agent(
    waap_root: &Path,
    name: Option<&str>,
) -> io::Result<Committed<AgentReport>> {
    let mut markdown = String::new();
    io::stdin()
        .read_to_string(&mut markdown)
        .map_err(|error| io::Error::new(error.kind(), format!("failed to read stdin: {error}")))?;

    let report = create_agent_with_markdown(waap_root, name, &markdown)?;
    let commit = commit_paths(
        waap_root,
        &[report.path.as_path()],
        &format!("waap agent new {}", report.agent_id),
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

pub(crate) fn create_agent_with_markdown(
    waap_root: &Path,
    name: Option<&str>,
    markdown: &str,
) -> io::Result<AgentReport> {
    require_initialized_project(waap_root)?;

    let agents_dir = WaapRecordKind::Agent.root_path(waap_root);
    let agent_id = available_agent_id(&agents_dir, name)?;

    let metadata = AgentMetadata {
        name: name.map(str::to_string),
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
    fn create_agent_slugifies_name() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap")).unwrap();

        let report = create_agent_with_markdown(
            dir.path(),
            Some("Custom Agent_123"),
            "# Purpose\nPlan things\n",
        )
        .unwrap();

        assert_eq!(report.agent_id, "aa-custom-agent123");
        assert_eq!(report.metadata.name.as_deref(), Some("Custom Agent_123"));
        assert!(report
            .path
            .ends_with(".waap/agents/aa-custom-agent123/agent.md"));
        assert!(check_waap(dir.path()).is_empty());
    }

    #[test]
    fn create_agent_name_conflict_appends_hex_suffix() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap/agents/aa-custom-agent")).unwrap();

        let report =
            create_agent_with_markdown(dir.path(), Some("Custom Agent"), "# Purpose\n").unwrap();

        assert!(report.agent_id.starts_with("aa-custom-agent-"));
        assert_eq!(report.agent_id.len(), "aa-custom-agent-".len() + 4);
    }

    #[test]
    fn create_agent_without_name_uses_random_hex_id() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap")).unwrap();

        let report = create_agent_with_markdown(dir.path(), None, "# Purpose\n").unwrap();
        let suffix = report.agent_id.strip_prefix("aa-").unwrap();

        assert_eq!(suffix.len(), 8);
        assert!(suffix.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(report.metadata.name, None);
    }
}
