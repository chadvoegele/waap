use std::io;
use std::path::Path;
use std::process::ExitCode;

use crate::agent::{
    agent_report_json, load_agent_report, print_agent_report_human, read_agent_record,
    write_agent_record, AgentReport, AgentSystem,
};
use crate::claude::{build_claude_run_command, claude_run_config_from_env, run_claude_detached};
use crate::cli::OutputFormat;
use crate::opencode::{
    build_opencode_run_command, create_opencode_session, opencode_run_config_from_env,
    run_opencode_detached, wait_for_opencode_session_status,
};
use uuid::Uuid;

pub(crate) fn print_agent_report(output_format: &OutputFormat, report: &AgentReport) {
    match output_format {
        OutputFormat::Json => println!("{}", agent_report_json(report)),
        OutputFormat::HumanReadable => print_agent_report_human("Running agent", report),
    }
}

pub(crate) fn run_agent(
    repo_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
    system: &AgentSystem,
) -> io::Result<ExitCode> {
    match system {
        AgentSystem::Opencode => run_agent_opencode(repo_root, output_format, agent_id),
        AgentSystem::Claude => run_agent_claude(repo_root, output_format, agent_id),
    }
}

fn run_agent_opencode(
    repo_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
) -> io::Result<ExitCode> {
    let config = opencode_run_config_from_env(repo_root)?;
    let session_id = create_opencode_session(&config)?;

    let (mut metadata, body) = read_agent_record(repo_root, agent_id)?;
    metadata.session_id = Some(session_id.clone());
    metadata.system = Some(AgentSystem::Opencode);

    run_opencode_detached(&build_opencode_run_command(&config, agent_id, &session_id))?;
    if !wait_for_opencode_session_status(&config, &session_id)? {
        eprintln!("warning: opencode session {session_id} did not appear active within 2 seconds");
    }

    metadata.status = "running".to_string();
    write_agent_record(repo_root, agent_id, &metadata, &body)?;

    let report = load_agent_report(repo_root, agent_id)?;
    print_agent_report(output_format, &report);
    Ok(ExitCode::SUCCESS)
}

fn run_agent_claude(
    repo_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
) -> io::Result<ExitCode> {
    let config = claude_run_config_from_env(repo_root)?;
    let session_id = Uuid::new_v4().to_string();

    let (mut metadata, body) = read_agent_record(repo_root, agent_id)?;
    metadata.session_id = Some(session_id.clone());
    metadata.system = Some(AgentSystem::Claude);

    run_claude_detached(&build_claude_run_command(&config, agent_id, &session_id))?;

    metadata.status = "running".to_string();
    write_agent_record(repo_root, agent_id, &metadata, &body)?;

    let report = load_agent_report(repo_root, agent_id)?;
    print_agent_report(output_format, &report);
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use serde_json::json;
    use tempfile::tempdir;

    use crate::agent::{agent_report_json, AgentMetadata, AgentReport};

    #[test]
    fn run_report_json_includes_running_status_and_session_id() {
        let report = AgentReport {
            agent_id: "aa-3881fda0".to_string(),
            path: std::path::PathBuf::from(".waap/agents/aa-3881fda0/agent.md"),
            metadata: AgentMetadata {
                creation_date: "2026-06-18T15:00:34Z".to_string(),
                role: "developer".to_string(),
                status: "running".to_string(),
                session_id: Some("ses_abc123".to_string()),
                system: None,
            },
            file_size: 512,
        };

        let json = agent_report_json(&report);

        assert_eq!(json["metadata"]["status"], "running");
        assert_eq!(json["metadata"]["session_id"], "ses_abc123");
        assert_eq!(
            json,
            json!({
                "agent_id": "aa-3881fda0",
                "path": ".waap/agents/aa-3881fda0/agent.md",
                "metadata": {
                    "creation_date": "2026-06-18T15:00:34Z",
                    "role": "developer",
                    "status": "running",
                    "session_id": "ses_abc123",
                },
                "file_size": 512,
            })
        );
    }

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn run_agent_claude_updates_status_and_session_id_in_frontmatter() {
        use crate::agent::{load_agent_report, read_agent_record, AgentSystem};

        let dir = tempdir().unwrap();
        let agent_id = "aa-3881fda0";
        let path = dir.path().join(".waap/agents/aa-3881fda0/agent.md");
        write_file(
            &path,
            "+++\ncreation_date = 2026-06-18T15:00:34Z\nrole = \"developer\"\nstatus = \"ready\"\n+++\n\n# Purpose\nDo work\n",
        );

        // Simulate the run_agent_claude path: read once, mutate, write, then derive report.
        let (mut metadata, body) = read_agent_record(dir.path(), agent_id).unwrap();
        let session_id = "ses_test123".to_string();
        metadata.session_id = Some(session_id.clone());
        metadata.system = Some(AgentSystem::Claude);
        metadata.status = "running".to_string();
        crate::agent::write_agent_record(dir.path(), agent_id, &metadata, &body).unwrap();

        let report = load_agent_report(dir.path(), agent_id).unwrap();

        assert_eq!(report.metadata.status, "running");
        assert_eq!(report.metadata.session_id.as_deref(), Some("ses_test123"));

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("status = \"running\"\n"));
        assert!(contents.contains("session_id = \"ses_test123\"\n"));
        assert!(contents.contains("system = \"claude\"\n"));
        assert!(contents.contains("# Purpose\nDo work\n"));
    }
}
