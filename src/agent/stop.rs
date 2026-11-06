use std::io;
use std::path::Path;

use serde_json::json;

use super::backend::{AbortContext, AbortOutcome, Backend};
use crate::agent::get::load_agent_report;
use crate::agent::{
    agent_report_json, print_agent_report_human, read_agent_record, transition_agent_status,
    write_agent_record, AgentReport, AgentStatus,
};
use crate::cli::OutputFormat;
use crate::git::commit_paths;
use crate::record::{list_record_ids, WaapRecordKind};
use crate::root::ControlRoot;

#[derive(Debug)]
pub(crate) struct AgentStopReport {
    agent_stops: Vec<AgentStop>,
    pub(crate) commit: Option<String>,
}

#[derive(Debug)]
struct AgentStop {
    report: AgentReport,
    outcome: AbortOutcome,
}

impl AgentStop {
    fn describe_outcome(&self) -> &'static str {
        match self.outcome {
            AbortOutcome::StopAccepted => "Stop accepted for agent",
            AbortOutcome::Stopped => "Stopped agent",
        }
    }
}

pub(crate) fn print_agent_stop_report(output_format: &OutputFormat, report: &AgentStopReport) {
    match output_format {
        OutputFormat::Json => println!("{}", agent_stop_json(report)),
        OutputFormat::HumanReadable => {
            for agent in &report.agent_stops {
                print_agent_report_human(agent.describe_outcome(), &agent.report);
            }
            if let Some(commit) = &report.commit {
                println!("Commit: {commit}");
            }
        }
    }
}

fn agent_stop_json(report: &AgentStopReport) -> serde_json::Value {
    json!({
        "stopped_agents": report.agent_stops.iter().map(|agent| agent_report_json(&agent.report)).collect::<Vec<_>>(),
        "commit": report.commit,
    })
}

pub(crate) fn stop_agents_with_systems(
    waap_root: &Path,
    control_root: io::Result<ControlRoot>,
    agent_id: Option<&str>,
) -> io::Result<AgentStopReport> {
    let selected = select_agents_to_stop(waap_root, agent_id)?;
    let mut agent_stops = Vec::new();
    let mut commit = None;
    for report in selected {
        if report.metadata.status == AgentStatus::Running.as_str() {
            let mut backend = report
                .metadata
                .system
                .clone()
                .unwrap_or_default()
                .backend(None)?;
            let (stopped, state_commit) =
                stop_agent_session(waap_root, &control_root, &report, &mut backend)?;
            commit = state_commit.or(commit);
            agent_stops.push(stopped);
            continue;
        }
        let (stopped, state_commit) = persist_stopped_agent(waap_root, &report)?;
        commit = state_commit.or(commit);
        agent_stops.push(AgentStop {
            report: stopped,
            outcome: AbortOutcome::Stopped,
        });
    }
    Ok(AgentStopReport {
        agent_stops,
        commit,
    })
}

fn select_agents_to_stop(waap_root: &Path, agent_id: Option<&str>) -> io::Result<Vec<AgentReport>> {
    let _lock = crate::state::StateLock::acquire(waap_root)?;
    match agent_id {
        Some(id) => {
            let report = load_agent_report(waap_root, id)?;
            AgentStatus::parse(&report.metadata.status)
                .expect("validated agent status")
                .validate_transition(AgentStatus::Aborted)?;
            Ok(vec![report])
        }
        None => Ok(list_record_ids(waap_root, WaapRecordKind::Agent)?
            .iter()
            .map(|id| load_agent_report(waap_root, id))
            .collect::<io::Result<Vec<_>>>()?
            .into_iter()
            .filter(|report| report.metadata.status == AgentStatus::Running.as_str())
            .collect()),
    }
}

fn stop_agent_session(
    waap_root: &Path,
    control_root: &io::Result<ControlRoot>,
    report: &AgentReport,
    backend: &mut Backend,
) -> io::Result<(AgentStop, Option<String>)> {
    let outcome = abort_agent_session(waap_root, control_root, report, backend)?;
    let (report, commit) = match outcome {
        AbortOutcome::StopAccepted => (load_agent_report(waap_root, &report.agent_id)?, None),
        AbortOutcome::Stopped => persist_stopped_agent(waap_root, report)?,
    };
    Ok((AgentStop { report, outcome }, commit))
}

fn abort_agent_session(
    waap_root: &Path,
    control_root: &io::Result<ControlRoot>,
    report: &AgentReport,
    backend: &mut Backend,
) -> io::Result<AbortOutcome> {
    super::runtime::new()?.block_on(backend.abort(AbortContext {
        waap_root,
        session_id: report.metadata.session_id.as_deref(),
        agent_id: &report.agent_id,
        control_root,
    }))
}

fn persist_stopped_agent(
    waap_root: &Path,
    expected: &AgentReport,
) -> io::Result<(AgentReport, Option<String>)> {
    let lock = crate::state::StateLock::acquire(waap_root)?;
    persist_stopped_agent_locked(waap_root, expected, &lock)
}

fn persist_stopped_agent_locked(
    waap_root: &Path,
    expected: &AgentReport,
    _lock: &crate::state::StateLock,
) -> io::Result<(AgentReport, Option<String>)> {
    let id = &expected.agent_id;
    let current = load_agent_report(waap_root, id)?;
    if current.metadata != expected.metadata {
        return Ok((current, None));
    }
    let stopped = mark_agent_aborted(waap_root, id)?;
    let commit = commit_paths(
        waap_root,
        &[stopped.path.as_path()],
        &format!("waap agent stop {id}"),
    )
    .map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("failed to commit waap state change: {error}"),
        )
    })?;
    Ok((stopped, Some(commit)))
}

#[cfg(test)]
fn stop_agents(waap_root: &Path, agent_id: Option<&str>) -> io::Result<Vec<AgentReport>> {
    stop_agents_with_systems(
        waap_root,
        Err(io::Error::other("control root required for Codex")),
        agent_id,
    )
    .map(|report| {
        report
            .agent_stops
            .into_iter()
            .map(|agent| agent.report)
            .collect()
    })
}

#[cfg(test)]
fn stop_agent_with_backend(
    waap_root: &Path,
    agent_id: &str,
    backend: &mut Backend,
) -> io::Result<Option<AgentReport>> {
    let report = load_agent_report(waap_root, agent_id)?;
    if report.metadata.status != AgentStatus::Running.as_str() {
        return Ok(None);
    }

    stop_agent_session(
        waap_root,
        &Err(io::Error::other("no control root")),
        &report,
        backend,
    )
    .map(|(agent, _)| Some(agent.report))
}

fn mark_agent_aborted(waap_root: &Path, agent_id: &str) -> io::Result<AgentReport> {
    let (mut metadata, body) = read_agent_record(waap_root, agent_id)?;
    transition_agent_status(&mut metadata, AgentStatus::Aborted)?;
    write_agent_record(waap_root, agent_id, &metadata, &body)?;

    load_agent_report(waap_root, agent_id)
}

#[cfg(test)]
#[path = "stop_tests.rs"]
mod tests;
