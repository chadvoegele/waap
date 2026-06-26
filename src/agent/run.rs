use std::fs;
use std::io;
use std::path::Path;
use std::process::ExitCode;

use crate::agent::{
    agent_report_json, load_agent_report, print_agent_report_human, read_agent_record,
    write_agent_record, AgentReport,
};
use crate::cli::OutputFormat;
use crate::opencode::{
    build_opencode_run_command, create_opencode_session, opencode_run_config_from_env,
    run_opencode_detached, wait_for_opencode_session_status,
};

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
) -> io::Result<ExitCode> {
    let mut report = load_agent_report(repo_root, agent_id)?;
    let config = opencode_run_config_from_env(repo_root)?;
    let session_id = create_opencode_session(&config)?;

    let (mut metadata, body) = read_agent_record(repo_root, agent_id)?;
    metadata.session_id = Some(session_id.clone());
    write_agent_record(repo_root, agent_id, &metadata, &body)?;

    run_opencode_detached(&build_opencode_run_command(&config, agent_id, &session_id))?;
    if !wait_for_opencode_session_status(&config, &session_id)? {
        eprintln!("warning: opencode session {session_id} did not appear active within 2 seconds");
    }
    report.session_id = Some(session_id);
    report.file_size = fs::metadata(&report.path)?.len();
    print_agent_report(output_format, &report);
    Ok(ExitCode::SUCCESS)
}
