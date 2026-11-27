use std::io;
use std::path::Path;

use crate::agent::get::load_agent_report;
use crate::agent::{
    agent_report_json, print_agent_report_human, read_agent_record, write_agent_record, AgentReport,
};
use crate::cli::OutputFormat;

pub(crate) fn print_updated_agent_report(output_format: &OutputFormat, report: &AgentReport) {
    match output_format {
        OutputFormat::Json => println!("{}", agent_report_json(report)),
        OutputFormat::HumanReadable => print_agent_report_human("Updated agent", report),
    }
}

pub(crate) fn update_agent(
    waap_root: &Path,
    agent_id: &str,
    name: &str,
) -> io::Result<AgentReport> {
    let _lock = crate::state::StateLock::acquire(waap_root)?;
    let (mut metadata, body) = read_agent_record(waap_root, agent_id)?;
    metadata.name = Some(name.to_owned());
    write_agent_record(waap_root, agent_id, &metadata, &body)?;
    load_agent_report(waap_root, agent_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentSystem;

    #[test]
    fn name_update_preserves_lifecycle_identity_and_body() {
        let dir = tempfile::tempdir().unwrap();
        for (status, session) in [
            ("ready", ""),
            ("running", "session_id = \"ses_new\"\n"),
            ("completed", ""),
            ("aborted", ""),
            ("failed", ""),
        ] {
            let id = "aa-test";
            let path = crate::agent::agent_path(dir.path(), id);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, format!("+++\ncreation_date = 2026-06-18T15:00:34Z\nstatus = \"{status}\"\nsystem = \"codex\"\n{session}+++\n\n# Existing body\n")).unwrap();
            let report = update_agent(dir.path(), id, "New name\n雪").unwrap();
            assert_eq!(report.agent_id, id);
            assert_eq!(report.metadata.name.as_deref(), Some("New name\n雪"));
            assert_eq!(report.metadata.status, status);
            assert_eq!(report.metadata.creation_date, "2026-06-18T15:00:34Z");
            assert_eq!(report.metadata.system, AgentSystem::Codex);
            assert_eq!(
                report.metadata.session_id.as_deref(),
                (status == "running").then_some("ses_new")
            );
            let contents = std::fs::read_to_string(path).unwrap();
            assert!(contents.ends_with("\n# Existing body\n"));
            assert_eq!(report.file_size, contents.len() as u64);
        }
    }

    #[test]
    fn update_reports_missing_agent() {
        let dir = tempfile::tempdir().unwrap();
        let error = update_agent(dir.path(), "aa-missing", "New name").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(error.to_string().contains("agents/aa-missing/agent.md"));
    }
}
