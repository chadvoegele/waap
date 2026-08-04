use std::io;

use crate::agent::{
    agent_report_json, load_agent_report, print_agent_report_human, read_agent_record,
    transition_agent_status, write_agent_record, AgentMetadata, AgentReport, AgentStatus,
    AgentSystem,
};
use crate::cli::OutputFormat;
use crate::git::{StateStore, StateTransaction};

fn print_report(output_format: &OutputFormat, header: &str, report: &AgentReport, commit: &str) {
    match output_format {
        OutputFormat::Json => {
            let mut value = agent_report_json(report);
            value["commit"] = serde_json::json!(commit);
            println!("{value}");
        }
        OutputFormat::HumanReadable => {
            print_agent_report_human(header, report);
            println!("Commit: {commit}");
        }
    }
}

pub(super) fn mark_running(
    store: &StateStore,
    output_format: &OutputFormat,
    agent_id: &str,
    metadata: &mut AgentMetadata,
    body: &str,
) -> io::Result<()> {
    let mut transaction = StateTransaction::begin(store.clone())?;
    let state_root = transaction.state_root().to_path_buf();
    let path = crate::agent::agent_path(&state_root, agent_id);
    transaction.snapshot_path(&path)?;
    transition_agent_status(metadata, AgentStatus::Running)?;
    write_agent_record(&state_root, agent_id, metadata, body)?;

    let report = load_agent_report(&state_root, agent_id)?;
    let commit = transaction.commit(
        &[report.path.as_path()],
        &format!("waap agent run {agent_id}"),
    )?;
    print_report(output_format, "Running agent", &report, &commit);
    Ok(())
}

pub(super) fn update_session(
    store: &StateStore,
    output_format: &OutputFormat,
    agent_id: &str,
    session_id: &str,
    system: AgentSystem,
) -> io::Result<()> {
    let mut transaction = StateTransaction::begin(store.clone())?;
    let state_root = transaction.state_root().to_path_buf();
    let (mut metadata, body) = read_agent_record(&state_root, agent_id)?;
    if metadata.status != AgentStatus::Running.as_str() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("agent {agent_id} must be running to assign a session"),
        ));
    }
    if let Some(existing_session_id) = &metadata.session_id {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("agent {agent_id} already has session id {existing_session_id}"),
        ));
    }
    if let Some(existing_system) = &metadata.system {
        if existing_system != &system {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "agent {agent_id} system mismatch: expected {}, got {}",
                    existing_system.as_str(),
                    system.as_str()
                ),
            ));
        }
    }

    let header = format!("{} session", system.as_str());
    metadata.session_id = Some(session_id.to_string());
    metadata.system = Some(system.clone());
    let path = crate::agent::agent_path(&state_root, agent_id);
    transaction.snapshot_path(&path)?;
    write_agent_record(&state_root, agent_id, &metadata, &body)?;

    let report = load_agent_report(&state_root, agent_id)?;
    let commit = transaction.commit(
        &[report.path.as_path()],
        &format!("waap agent {} session {agent_id}", system.as_str()),
    )?;
    print_report(output_format, &header, &report, &commit);
    Ok(())
}

pub(super) fn mark_completed(
    store: &StateStore,
    output_format: &OutputFormat,
    agent_id: &str,
) -> io::Result<()> {
    transition_status(
        store,
        output_format,
        agent_id,
        AgentStatus::Completed,
        "Completed agent",
        &format!("waap agent completed {agent_id}"),
    )
}

pub(super) fn mark_failed(
    store: &StateStore,
    output_format: &OutputFormat,
    agent_id: &str,
) -> io::Result<()> {
    transition_status(
        store,
        output_format,
        agent_id,
        AgentStatus::Failed,
        "Failed agent",
        &format!("waap agent failed {agent_id}"),
    )
}

pub(super) fn transition_status(
    store: &StateStore,
    output_format: &OutputFormat,
    agent_id: &str,
    status: AgentStatus,
    header: &str,
    commit_message: &str,
) -> io::Result<()> {
    let mut transaction = StateTransaction::begin(store.clone())?;
    let state_root = transaction.state_root().to_path_buf();
    let (mut metadata, body) = read_agent_record(&state_root, agent_id)?;
    if metadata.status == status.as_str() {
        return Ok(());
    }
    transition_agent_status(&mut metadata, status)?;
    let path = crate::agent::agent_path(&state_root, agent_id);
    transaction.snapshot_path(&path)?;
    write_agent_record(&state_root, agent_id, &metadata, &body)?;
    let report = load_agent_report(&state_root, agent_id)?;
    let commit = transaction.commit(&[report.path.as_path()], commit_message)?;
    print_report(output_format, header, &report, &commit);
    Ok(())
}

pub(super) fn persist_failure(
    store: &StateStore,
    output_format: &OutputFormat,
    agent_id: &str,
    primary: io::Error,
) -> io::Error {
    match mark_failed(store, output_format, agent_id) {
        Ok(()) => primary,
        Err(persistence_error) => io::Error::new(
            primary.kind(),
            format!("{primary}; failed to persist agent failure state: {persistence_error}"),
        ),
    }
}
