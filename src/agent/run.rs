use std::io;
use std::path::Path;
use std::process::ExitCode;

use super::backend::{AgentSessionRun, Backend, RunOutcome, StartContext};
use super::commands::CommandDispatcher;
use super::control::{ControlServer, SocketListener, SocketPath};
use super::worktree::{collapse_errors, AgentWorktree};
use crate::agent::{
    agent_report_json, load_agent_report, print_agent_report_human, read_agent_record,
    transition_agent_status, write_agent_record, AgentMetadata, AgentReport, AgentRunOptions,
    AgentStatus, AgentSystem,
};
use crate::cli::OutputFormat;
use crate::git::commit_paths;
use crate::root::ControlRoot;

fn print_run_agent_report(
    output_format: &OutputFormat,
    header: &str,
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
            print_agent_report_human(header, report);
            println!("Commit: {commit}");
        }
    }
}

pub(crate) fn run_agent(
    repository_root: &Path,
    waap_root: &Path,
    control_root: io::Result<ControlRoot>,
    output_format: &OutputFormat,
    agent_id: &str,
    system: &AgentSystem,
    options: &AgentRunOptions,
) -> io::Result<ExitCode> {
    require_ready_agent(waap_root, agent_id)?;
    let mut backend = system.backend(Some(options))?;
    run_agent_with_backend(
        repository_root,
        waap_root,
        control_root,
        output_format,
        agent_id,
        system,
        &mut backend,
    )
}

fn run_agent_with_backend(
    repository_root: &Path,
    waap_root: &Path,
    control_root: io::Result<ControlRoot>,
    output_format: &OutputFormat,
    agent_id: &str,
    system: &AgentSystem,
    backend: &mut Backend,
) -> io::Result<ExitCode> {
    {
        let _lock = crate::state::StateLock::acquire(waap_root)?;
        let (mut metadata, body) = read_agent_record(waap_root, agent_id)?;
        let current = AgentStatus::parse(&metadata.status).expect("validated agent status");
        current.validate_transition(AgentStatus::Running)?;
        let previous_metadata = metadata.clone();
        metadata.system = Some(system.clone());
        if let Err(primary) = mark_running(waap_root, output_format, agent_id, &mut metadata, &body)
        {
            return match write_agent_record(waap_root, agent_id, &previous_metadata, &body) {
                Ok(()) => Err(primary),
                Err(rollback_error) => Err(io::Error::new(
                    primary.kind(),
                    format!("{primary}; failed to restore ready agent record: {rollback_error}"),
                )),
            };
        }
    }
    let result = run_agent_app(
        repository_root,
        waap_root,
        control_root,
        output_format,
        agent_id,
        system,
        backend,
    );

    match result {
        Ok(RunOutcome::Completed) => {
            if let Err(error) = mark_completed(waap_root, output_format, agent_id) {
                return Err(persist_failed_after_error(
                    waap_root,
                    output_format,
                    agent_id,
                    error,
                ));
            }
            Ok(ExitCode::SUCCESS)
        }
        Ok(RunOutcome::Aborted) => {
            transition_and_commit_status(
                waap_root,
                output_format,
                agent_id,
                AgentStatus::Aborted,
                "Aborted agent",
                &format!("waap agent aborted {agent_id}"),
            )?;
            Ok(ExitCode::SUCCESS)
        }
        Ok(RunOutcome::Failed(code)) => {
            mark_failed(waap_root, output_format, agent_id)?;
            Ok(code)
        }
        Err(error) => Err(persist_failed_after_error(
            waap_root,
            output_format,
            agent_id,
            error,
        )),
    }
}

fn run_agent_app(
    repository_root: &Path,
    waap_root: &Path,
    control_root: io::Result<ControlRoot>,
    output_format: &OutputFormat,
    agent_id: &str,
    system: &AgentSystem,
    backend: &mut Backend,
) -> io::Result<RunOutcome> {
    let repository_root = repository_root.canonicalize()?;
    let control_root = control_root?;
    let socket_path = SocketPath::for_agent(&control_root, agent_id)?;
    let runtime = super::runtime::new()?;
    let mut worktree = AgentWorktree::create(&repository_root, agent_id)?;
    let prompt = build_agent_goal(waap_root, agent_id);
    let run_result = runtime.block_on(async {
        let (dispatcher, receiver) = CommandDispatcher::create_channel();
        let mut session_run = backend
            .start(
                StartContext {
                    agent_id,
                    prompt: &prompt,
                    repository_root: &repository_root,
                    worktree_dir: worktree.dir(),
                },
                receiver,
            )
            .await?;
        let listener_result = (|| {
            update_agent_session(
                waap_root,
                output_format,
                agent_id,
                &session_run.session_id,
                system.clone(),
            )?;
            SocketListener::bind(socket_path)
        })();
        match listener_result {
            Ok(listener) => {
                AgentApp::new(session_run, ControlServer::new(listener, dispatcher))
                    .run()
                    .await
            }
            Err(error) => collapse_errors(Err(error), session_run.runner.cleanup().await),
        }
    });
    let cleanup_result = worktree.cleanup();
    collapse_errors(run_result, cleanup_result)
}

pub(super) struct AgentApp {
    run: AgentSessionRun,
    control: ControlServer,
}

impl AgentApp {
    pub(super) fn new(run: AgentSessionRun, control: ControlServer) -> Self {
        Self { run, control }
    }

    pub(super) async fn run(&mut self) -> io::Result<RunOutcome> {
        use futures_util::FutureExt;
        let (shutdown, stopped) = tokio::sync::oneshot::channel();
        let (outcome, control) = tokio::join!(
            async {
                let result = std::panic::AssertUnwindSafe(self.run.run())
                    .catch_unwind()
                    .await;
                let _ = shutdown.send(());
                result.unwrap_or_else(|_| Err(io::Error::other("agent wait panicked")))
            },
            self.control.run(async {
                let _ = stopped.await;
            }),
        );
        if let Err(error) = control {
            log::error!("control server failed: {error}");
        }
        let outcome = collapse_errors(outcome, self.control.close());
        collapse_errors(outcome, self.cleanup().await)
    }

    async fn cleanup(&mut self) -> io::Result<()> {
        self.run.runner.cleanup().await
    }
}

fn build_agent_goal(waap_root: &Path, agent_id: &str) -> String {
    let instruction_path = waap_root.join(format!("agents/{agent_id}/agent.md"));
    format!(
        "Complete when instructions in {} are satisfied",
        instruction_path.display()
    )
}

fn require_ready_agent(waap_root: &Path, agent_id: &str) -> io::Result<()> {
    let (metadata, _) = read_agent_record(waap_root, agent_id)?;
    let current = AgentStatus::parse(&metadata.status).expect("validated agent status");
    current.validate_transition(AgentStatus::Running)
}

fn mark_running(
    waap_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
    metadata: &mut AgentMetadata,
    body: &str,
) -> io::Result<()> {
    transition_agent_status(metadata, AgentStatus::Running)?;
    write_agent_record(waap_root, agent_id, metadata, body)?;

    let report = load_agent_report(waap_root, agent_id)?;
    let commit = commit_paths(
        waap_root,
        &[report.path.as_path()],
        &format!("waap agent run {agent_id}"),
    )?;
    print_run_agent_report(output_format, "Running agent", &report, &commit);
    Ok(())
}

fn update_agent_session(
    waap_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
    session_id: &str,
    system: AgentSystem,
) -> io::Result<()> {
    let _lock = crate::state::StateLock::acquire(waap_root)?;
    let (mut metadata, body) = read_agent_record(waap_root, agent_id)?;
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
    write_agent_record(waap_root, agent_id, &metadata, &body)?;

    let report = load_agent_report(waap_root, agent_id)?;
    let commit = commit_paths(
        waap_root,
        &[report.path.as_path()],
        &format!("waap agent {} session {agent_id}", system.as_str()),
    )?;
    print_run_agent_report(output_format, &header, &report, &commit);
    Ok(())
}

fn mark_completed(
    waap_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
) -> io::Result<()> {
    transition_and_commit_status(
        waap_root,
        output_format,
        agent_id,
        AgentStatus::Completed,
        "Completed agent",
        &format!("waap agent completed {agent_id}"),
    )
}

fn mark_failed(waap_root: &Path, output_format: &OutputFormat, agent_id: &str) -> io::Result<()> {
    transition_and_commit_status(
        waap_root,
        output_format,
        agent_id,
        AgentStatus::Failed,
        "Failed agent",
        &format!("waap agent failed {agent_id}"),
    )
}

fn transition_and_commit_status(
    waap_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
    status: AgentStatus,
    header: &str,
    commit_message: &str,
) -> io::Result<()> {
    let _lock = crate::state::StateLock::acquire(waap_root)?;
    let (mut metadata, body) = read_agent_record(waap_root, agent_id)?;
    if metadata.status == status.as_str() {
        return Ok(());
    }
    let previous_metadata = metadata.clone();
    transition_agent_status(&mut metadata, status)?;
    let persistence_result = (|| {
        write_agent_record(waap_root, agent_id, &metadata, &body)?;
        let report = load_agent_report(waap_root, agent_id)?;
        let commit = commit_paths(waap_root, &[report.path.as_path()], commit_message)?;
        Ok((report, commit))
    })();
    let (report, commit) = match persistence_result {
        Ok(persisted) => persisted,
        Err(primary) => {
            return match write_agent_record(waap_root, agent_id, &previous_metadata, &body) {
                Ok(()) => Err(primary),
                Err(rollback_error) => Err(io::Error::new(
                    primary.kind(),
                    format!("{primary}; failed to restore previous agent status: {rollback_error}"),
                )),
            };
        }
    };
    print_run_agent_report(output_format, header, &report, &commit);
    Ok(())
}

fn persist_failed_after_error(
    waap_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
    primary: io::Error,
) -> io::Error {
    match mark_failed(waap_root, output_format, agent_id) {
        Ok(()) => primary,
        Err(persistence_error) => io::Error::new(
            primary.kind(),
            format!("{primary}; failed to persist agent failure state: {persistence_error}"),
        ),
    }
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
