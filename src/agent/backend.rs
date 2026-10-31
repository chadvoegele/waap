use std::io;
use std::path::Path;
use std::process::{ExitCode, ExitStatus};
use std::time::Instant;

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
    pub(super) handle: Box<dyn RunHandle>,
}

pub(super) type BackendFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + 'a>>;

// Futures stay on the runner's current-thread runtime. Blocking backends must
// offload their waits rather than block the shared I/O executor.
pub(super) trait RunHandle: Sync {
    fn wait(&self) -> BackendFuture<'_, io::Result<RunOutcome>>;

    fn steer<'a>(
        &'a self,
        prompt: &'a str,
        deadline: Instant,
    ) -> BackendFuture<'a, Result<(), SteeringError>>;
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
}

impl std::fmt::Display for SteeringError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected { message, .. } | Self::AcceptanceUnknown { message } => {
                formatter.write_str(message)
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

pub(super) trait AgentSystemBackend {
    fn start(&mut self, context: StartContext<'_>) -> io::Result<StartedRun>;

    fn abort(&mut self, context: AbortContext<'_>) -> io::Result<()>;

    #[cfg(test)]
    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}

#[cfg(test)]
pub(super) mod fake {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;

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
                session_id: "ses_fake".to_string(),
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

    struct FakeRun {
        state: Mutex<Option<FakeRunState>>,
        wait_calls: Arc<AtomicUsize>,
        steer_calls: Arc<Mutex<Vec<(String, Instant)>>>,
        steer_error: Option<SteeringError>,
    }

    struct FakeRunState {
        outcome: RunOutcome,
        error: Option<String>,
        action: Option<Box<dyn FnOnce() -> io::Result<()> + Send>>,
    }

    impl RunHandle for FakeRun {
        fn wait(&self) -> BackendFuture<'_, io::Result<RunOutcome>> {
            Box::pin(async move {
                self.wait_calls.fetch_add(1, Ordering::SeqCst);
                let state = self.state.lock().unwrap().take().unwrap();
                tokio::task::spawn_blocking(move || {
                    if let Some(action) = state.action {
                        action()?;
                    }
                    if let Some(error) = state.error {
                        Err(io::Error::other(error))
                    } else {
                        Ok(state.outcome)
                    }
                })
                .await
                .unwrap_or_else(|error| {
                    if error.is_panic() {
                        std::panic::resume_unwind(error.into_panic());
                    }
                    Err(io::Error::other(error))
                })
            })
        }

        fn steer<'a>(
            &'a self,
            prompt: &'a str,
            deadline: Instant,
        ) -> BackendFuture<'a, Result<(), SteeringError>> {
            Box::pin(async move {
                self.steer_calls
                    .lock()
                    .unwrap()
                    .push((prompt.to_string(), deadline));
                match &self.steer_error {
                    Some(error) => Err(error.clone()),
                    None => Ok(()),
                }
            })
        }
    }

    impl AgentSystemBackend for FakeBackend {
        fn start(&mut self, context: StartContext<'_>) -> io::Result<StartedRun> {
            self.start_calls.push(StartCall {
                agent_id: context.agent_id.to_string(),
                prompt: context.prompt.to_string(),
                repository_root: context.repository_root.to_path_buf(),
                worktree_dir: context.worktree_dir.to_path_buf(),
                worktree_existed: context.worktree_dir.is_dir(),
            });
            if let Some(error) = self.start_error.take() {
                return Err(io::Error::other(error));
            }
            Ok(StartedRun {
                session_id: self.session_id.clone(),
                handle: Box::new(FakeRun {
                    state: Mutex::new(Some(FakeRunState {
                        outcome: self.outcome.take().unwrap_or(RunOutcome::Completed),
                        error: self.wait_error.take(),
                        action: self.wait_action.take(),
                    })),
                    wait_calls: Arc::clone(&self.wait_calls),
                    steer_calls: Arc::clone(&self.steer_calls),
                    steer_error: self.steer_error.clone(),
                }),
            })
        }

        fn abort(&mut self, context: AbortContext<'_>) -> io::Result<()> {
            self.abort_calls.push(AbortCall {
                waap_root: context.waap_root.to_path_buf(),
                agent_id: context.agent_id.to_string(),
                session_id: context.session_id.to_string(),
            });
            if let Some(error) = self.abort_error.take() {
                Err(io::Error::other(error))
            } else {
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::ExitStatusExt;

    use super::*;

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
