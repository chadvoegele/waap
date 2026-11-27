use std::fs;
use std::io;
use std::path::Path;

use crate::agent::{
    agent_path, agent_report_json, available_agent_id, print_agent_report_human,
    write_agent_record, AgentMetadata, AgentReport, AgentSystem,
};
use crate::cli::OutputFormat;
use crate::record::WaapRecordKind;
use crate::toml::current_toml_datetime;

pub(crate) fn print_created_agent_report(output_format: &OutputFormat, report: &AgentReport) {
    match output_format {
        OutputFormat::Json => println!("{}", agent_report_json(report)),
        OutputFormat::HumanReadable => print_agent_report_human("Created agent", report),
    }
}

pub(crate) fn create_agent(
    waap_root: &Path,
    name: Option<&str>,
    system: AgentSystem,
) -> io::Result<AgentReport> {
    let _lock = crate::state::StateLock::acquire(waap_root)?;
    let agents_dir = WaapRecordKind::Agent.root_path(waap_root);
    let agent_id = available_agent_id(&agents_dir, name)?;

    let metadata = AgentMetadata {
        name: name.map(str::to_string),
        creation_date: current_toml_datetime(),
        status: "ready".to_string(),
        session_id: None,
        system,
    };
    write_agent_record(waap_root, &agent_id, &metadata, "")?;
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

    use super::create_agent;
    use crate::agent::{is_agent_id, AgentSystem};
    use crate::check::check_waap;

    #[test]
    fn create_agent_writes_only_registration_metadata() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap")).unwrap();

        let report = create_agent(dir.path(), None, AgentSystem::default()).unwrap();
        let contents = fs::read_to_string(&report.path).unwrap();

        assert!(is_agent_id(&report.agent_id));
        assert_eq!(report.metadata.status, "ready");
        assert_eq!(report.metadata.session_id, None);
        assert_eq!(report.file_size, contents.len() as u64);
        assert!(contents.starts_with("+++\ncreation_date = "));
        assert!(contents.ends_with("\nstatus = \"ready\"\nsystem = \"pi\"\n+++\n"));
        assert!(!contents.contains("name ="));
        assert!(!contents.contains("session_id ="));
        assert!(check_waap(dir.path()).is_empty());
    }

    #[test]
    fn create_agent_slugifies_name() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap")).unwrap();

        let report =
            create_agent(dir.path(), Some("Custom Agent_123"), AgentSystem::Opencode).unwrap();
        assert_eq!(report.metadata.system, AgentSystem::Opencode);

        assert_eq!(report.agent_id, "aa-custom-agent123");
        assert_eq!(report.metadata.name.as_deref(), Some("Custom Agent_123"));
        assert!(report.path.ends_with("agents/aa-custom-agent123/agent.md"));
        assert!(check_waap(dir.path()).is_empty());
    }

    #[test]
    fn create_agent_name_conflict_appends_hex_suffix() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("agents/aa-custom-agent")).unwrap();

        let report = create_agent(dir.path(), Some("Custom Agent"), AgentSystem::Pi).unwrap();

        assert!(report.agent_id.starts_with("aa-custom-agent-"));
        assert_eq!(report.agent_id.len(), "aa-custom-agent-".len() + 4);
    }

    #[test]
    fn create_agent_caps_new_ids_without_invalidating_historical_records() {
        let dir = tempdir().unwrap();
        let name = "a".repeat(crate::ids::MAX_SLUG_LEN);
        let first = create_agent(dir.path(), Some(&name), AgentSystem::Pi).unwrap();
        let second = create_agent(dir.path(), Some(&name), AgentSystem::Pi).unwrap();
        let longer = create_agent(dir.path(), Some(&"b".repeat(63)), AgentSystem::Pi).unwrap();
        assert_eq!(first.agent_id, format!("aa-{name}"));
        for report in [&first, &second, &longer] {
            assert_eq!(
                report.agent_id.len(),
                "aa-".len() + crate::ids::MAX_SLUG_LEN
            );
        }
        let historical = format!("aa-{}", "c".repeat(63));
        let historical_path = dir.path().join("agents").join(&historical).join("agent.md");
        fs::create_dir_all(historical_path.parent().unwrap()).unwrap();
        fs::copy(&first.path, &historical_path).unwrap();
        assert!(crate::agent::load_agent_report(dir.path(), &historical).is_ok());
        assert!(check_waap(dir.path()).is_empty());
    }

    #[test]
    fn create_agent_without_name_uses_random_hex_id() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap")).unwrap();

        let report = create_agent(dir.path(), None, AgentSystem::Pi).unwrap();
        let suffix = report.agent_id.strip_prefix("aa-").unwrap();

        assert_eq!(suffix.len(), 8);
        assert!(suffix.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(report.metadata.name, None);
    }
}
