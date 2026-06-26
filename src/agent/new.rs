use std::fs;
use std::io::{self, Read};
use std::path::Path;

use crate::agent::{
    agent_path, agent_report_json, available_agent_id, print_agent_report_human,
    write_agent_record, AgentMetadata, AgentReport, AgentRole,
};
use crate::cli::OutputFormat;
use crate::ids::current_toml_datetime;
use crate::record::WaapRecordKind;

pub(crate) fn print_created_agent_report(output_format: &OutputFormat, report: &AgentReport) {
    match output_format {
        OutputFormat::Json => println!("{}", agent_report_json(report)),
        OutputFormat::HumanReadable => print_agent_report_human("Created agent", report),
    }
}

pub(crate) fn create_agent(repo_root: &Path, role: &AgentRole) -> io::Result<AgentReport> {
    let mut markdown = String::new();
    io::stdin()
        .read_to_string(&mut markdown)
        .map_err(|error| io::Error::new(error.kind(), format!("failed to read stdin: {error}")))?;

    create_agent_with_markdown(repo_root, role, &markdown)
}

pub(crate) fn create_agent_with_markdown(
    repo_root: &Path,
    role: &AgentRole,
    markdown: &str,
) -> io::Result<AgentReport> {
    let agents_dir = WaapRecordKind::Agent.root_path(repo_root);
    let agent_id = available_agent_id(&agents_dir)?;

    let creation_date = current_toml_datetime();
    let metadata = AgentMetadata {
        creation_date: creation_date.clone(),
        role: role.as_str().to_string(),
        status: "ready".to_string(),
        session_id: None,
    };
    write_agent_record(repo_root, &agent_id, &metadata, &format!("\n{markdown}"))?;
    let path = agent_path(repo_root, &agent_id);
    let file_size = fs::metadata(&path)?.len();

    Ok(AgentReport {
        agent_id,
        path,
        creation_date,
        role: role.as_str().to_string(),
        status: "ready".to_string(),
        session_id: None,
        file_size,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::create_agent_with_markdown;
    use crate::agent::{is_agent_id, AgentRole};
    use crate::check::check_waap;

    #[test]
    fn create_agent_writes_frontmatter_and_stdin_content() {
        let dir = tempdir().unwrap();

        let report =
            create_agent_with_markdown(dir.path(), &AgentRole::Planner, "# Purpose\nPlan things\n")
                .unwrap();
        let contents = fs::read_to_string(&report.path).unwrap();

        assert!(is_agent_id(&report.agent_id));
        assert_eq!(report.role, "planner");
        assert_eq!(report.status, "ready");
        assert_eq!(report.session_id, None);
        assert_eq!(report.file_size, contents.len() as u64);
        assert!(contents.starts_with("+++\ncreation_date = "));
        assert!(contents
            .contains("\nrole = \"planner\"\nstatus = \"ready\"\n+++\n\n# Purpose\nPlan things\n"));
        assert!(check_waap(dir.path()).is_empty());
    }
}
