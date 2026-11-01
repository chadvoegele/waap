use std::io;
use std::path::Path;
use std::process::{ExitCode, ExitStatus};

use super::commands::CommandHandle;
use super::{claude::ClaudeBackend, codex::CodexBackend, opencode::OpencodeBackend};

#[derive(Debug, PartialEq)]
pub(super) enum RunOutcome {
    Completed,
    Failed(ExitCode),
}

impl RunOutcome {
    pub(super) fn from_exit_status(status: ExitStatus) -> Self {
        if status.success() {
            Self::Completed
        } else {
            Self::Failed(ExitCode::from(status.code().unwrap_or(1) as u8))
        }
    }
}

pub(super) struct StartContext<'a> {
    pub(super) agent_id: &'a str,
    pub(super) prompt: &'a str,
    pub(super) repository_root: &'a Path,
    pub(super) worktree_dir: &'a Path,
}

pub(super) struct StartedRun {
    pub(super) session_id: String,
    pub(super) handle: CommandHandle,
    pub(super) driver: Run,
}

// The actor owns its resources. Its command handle never borrows protocol state.
pub(super) enum Run {
    Claude(super::claude::ClaudeRun),
    Codex(Box<super::codex::CodexRun>),
    Opencode(super::opencode::OpencodeRun),
    #[cfg(test)]
    Fake(fake::FakeRun),
}

impl Run {
    pub(super) async fn run(&mut self) -> io::Result<RunOutcome> {
        match self {
            Self::Claude(run) => run.run().await,
            Self::Codex(run) => run.run().await,
            Self::Opencode(run) => run.run().await,
            #[cfg(test)]
            Self::Fake(run) => run.run().await,
        }
    }

    pub(super) async fn cleanup(&mut self) -> io::Result<()> {
        match self {
            Self::Claude(run) => run.cleanup().await,
            Self::Codex(run) => run.cleanup().await,
            Self::Opencode(_) => Ok(()),
            #[cfg(test)]
            Self::Fake(_) => Ok(()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SteeringError {
    Rejected { code: &'static str, message: String },
    AcceptanceUnknown { message: String },
}

impl SteeringError {
    pub(super) fn not_implemented(backend: &str) -> Self {
        Self::Rejected {
            code: "not_implemented",
            message: format!("steering not implemented for backend {backend}"),
        }
    }
    pub(super) fn acceptance_unknown(reason: &str) -> Self {
        Self::AcceptanceUnknown {
            message: format!(
                "Steering acceptance is unknown: {reason}; do not retry automatically"
            ),
        }
    }
}

impl std::fmt::Display for SteeringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected { message, .. } | Self::AcceptanceUnknown { message } => {
                f.write_str(message)
            }
        }
    }
}
impl std::error::Error for SteeringError {}

pub(super) struct AbortContext<'a> {
    pub(super) waap_root: &'a Path,
    pub(super) agent_id: &'a str,
    pub(super) session_id: &'a str,
}

// Static async dispatch keeps backend tests injectable without boxed futures.
pub(super) trait AgentSystemBackend {
    async fn start(&mut self, context: StartContext<'_>) -> io::Result<StartedRun>;
    async fn abort(&mut self, context: AbortContext<'_>) -> io::Result<()>;
}

pub(super) enum Backend {
    Claude(ClaudeBackend),
    Codex(CodexBackend),
    Opencode(OpencodeBackend),
}
impl AgentSystemBackend for Backend {
    async fn start(&mut self, context: StartContext<'_>) -> io::Result<StartedRun> {
        match self {
            Self::Claude(b) => b.start(context).await,
            Self::Codex(b) => b.start(context).await,
            Self::Opencode(b) => b.start(context).await,
        }
    }
    async fn abort(&mut self, context: AbortContext<'_>) -> io::Result<()> {
        match self {
            Self::Claude(b) => b.abort(context).await,
            Self::Codex(b) => b.abort(context).await,
            Self::Opencode(b) => b.abort(context).await,
        }
    }
}
#[cfg(test)]
impl Backend {
    pub(super) fn type_name(&self) -> &'static str {
        match self {
            Self::Claude(_) => std::any::type_name::<ClaudeBackend>(),
            Self::Codex(_) => std::any::type_name::<CodexBackend>(),
            Self::Opencode(_) => std::any::type_name::<OpencodeBackend>(),
        }
    }
}

#[cfg(test)]
pub(super) mod fake {
    use super::*;
    use crate::agent::commands::Command;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;
    use tokio::sync::mpsc;

    #[derive(Debug, PartialEq, Eq)]
    pub(crate) struct StartCall {
        pub(crate) agent_id: String,
        pub(crate) prompt: String,
        pub(crate) repository_root: PathBuf,
        pub(crate) worktree_dir: PathBuf,
        pub(crate) worktree_existed: bool,
    }
    #[derive(Debug, PartialEq, Eq)]
    pub(crate) struct AbortCall {
        pub(crate) waap_root: PathBuf,
        pub(crate) agent_id: String,
        pub(crate) session_id: String,
    }

    pub(crate) struct FakeBackend {
        pub(crate) session_id: String,
        pub(crate) outcome: Option<RunOutcome>,
        pub(crate) start_error: Option<String>,
        pub(crate) wait_error: Option<String>,
        pub(crate) wait_action: Option<Box<dyn FnOnce() -> io::Result<()> + Send>>,
        pub(crate) abort_error: Option<String>,
        pub(crate) start_calls: Vec<StartCall>,
        pub(crate) wait_calls: Arc<AtomicUsize>,
        pub(crate) steer_calls: Arc<Mutex<Vec<(String, Instant)>>>,
        pub(crate) steer_error: Option<SteeringError>,
        pub(crate) abort_calls: Vec<AbortCall>,
    }
    impl Default for FakeBackend {
        fn default() -> Self {
            Self {
                session_id: "ses_fake".into(),
                outcome: Some(RunOutcome::Completed),
                start_error: None,
                wait_error: None,
                wait_action: None,
                abort_error: None,
                start_calls: Vec::new(),
                wait_calls: Arc::new(AtomicUsize::new(0)),
                steer_calls: Arc::new(Mutex::new(Vec::new())),
                steer_error: None,
                abort_calls: Vec::new(),
            }
        }
    }
    pub(in crate::agent) struct FakeRun {
        outcome: Option<RunOutcome>,
        error: Option<String>,
        action: Option<Box<dyn FnOnce() -> io::Result<()> + Send>>,
        wait_calls: Arc<AtomicUsize>,
        steer_calls: Arc<Mutex<Vec<(String, Instant)>>>,
        steer_error: Option<SteeringError>,
        commands: mpsc::Receiver<Command>,
    }
    impl FakeRun {
        pub(super) async fn run(&mut self) -> io::Result<RunOutcome> {
            self.wait_calls.fetch_add(1, Ordering::SeqCst);
            let action = self.action.take();
            let mut wait = tokio::task::spawn_blocking(move || {
                if let Some(action) = action {
                    action()?;
                }
                Ok::<_, io::Error>(())
            });
            loop {
                tokio::select! {
                    result = &mut wait => {
                        match result {
                            Ok(result) => result?,
                            Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
                            Err(error) => return Err(io::Error::other(error)),
                        }
                        return if let Some(error) = self.error.take() { Err(io::Error::other(error)) } else { Ok(self.outcome.take().unwrap()) };
                    }
                    Some(Command::Steer(mut request)) = self.commands.recv() => {
                        self.steer_calls.lock().unwrap().push((request.message.clone(), request.deadline));
                        request.respond(self.steer_error.clone().map_or(Ok(()), Err));
                    }
                }
            }
        }
    }
    impl AgentSystemBackend for FakeBackend {
        async fn start(&mut self, context: StartContext<'_>) -> io::Result<StartedRun> {
            self.start_calls.push(StartCall {
                agent_id: context.agent_id.into(),
                prompt: context.prompt.into(),
                repository_root: context.repository_root.into(),
                worktree_dir: context.worktree_dir.into(),
                worktree_existed: context.worktree_dir.is_dir(),
            });
            if let Some(error) = self.start_error.take() {
                return Err(io::Error::other(error));
            }
            let (handle, commands) = CommandHandle::channel();
            Ok(StartedRun {
                session_id: self.session_id.clone(),
                handle,
                driver: Run::Fake(FakeRun {
                    outcome: self.outcome.take(),
                    error: self.wait_error.take(),
                    action: self.wait_action.take(),
                    wait_calls: Arc::clone(&self.wait_calls),
                    steer_calls: Arc::clone(&self.steer_calls),
                    steer_error: self.steer_error.clone(),
                    commands,
                }),
            })
        }
        async fn abort(&mut self, context: AbortContext<'_>) -> io::Result<()> {
            self.abort_calls.push(AbortCall {
                waap_root: context.waap_root.into(),
                agent_id: context.agent_id.into(),
                session_id: context.session_id.into(),
            });
            self.abort_error
                .as_ref()
                .map_or(Ok(()), |error| Err(io::Error::other(error.clone())))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    #[test]
    fn exit_status_maps_success_and_exact_failure_code() {
        assert_eq!(
            RunOutcome::from_exit_status(ExitStatus::from_raw(0)),
            RunOutcome::Completed
        );
        assert_eq!(
            RunOutcome::from_exit_status(ExitStatus::from_raw(7 << 8)),
            RunOutcome::Failed(ExitCode::from(7))
        );
        assert_eq!(
            RunOutcome::from_exit_status(ExitStatus::from_raw(9)),
            RunOutcome::Failed(ExitCode::from(1))
        );
    }
}
