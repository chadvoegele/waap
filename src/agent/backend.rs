use std::io;
use std::path::Path;
use std::process::{ExitCode, ExitStatus};

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
    pub(super) worktree_dir: &'a Path,
}

pub(super) struct StartedRun {
    pub(super) session_id: String,
    pub(super) handle: Box<dyn RunHandle>,
}

pub(super) trait RunHandle {
    fn wait(self: Box<Self>) -> io::Result<RunOutcome>;
}

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
    use std::cell::Cell;
    use std::path::PathBuf;
    use std::rc::Rc;

    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    pub(crate) struct StartCall {
        pub(crate) agent_id: String,
        pub(crate) prompt: String,
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
        pub(crate) abort_error: Option<String>,
        pub(crate) start_calls: Vec<StartCall>,
        pub(crate) wait_calls: Rc<Cell<usize>>,
        pub(crate) abort_calls: Vec<AbortCall>,
    }

    impl Default for FakeBackend {
        fn default() -> Self {
            Self {
                session_id: "ses_fake".to_string(),
                outcome: Some(RunOutcome::Completed),
                start_error: None,
                wait_error: None,
                abort_error: None,
                start_calls: Vec::new(),
                wait_calls: Rc::new(Cell::new(0)),
                abort_calls: Vec::new(),
            }
        }
    }

    struct FakeRun {
        outcome: RunOutcome,
        error: Option<String>,
        wait_calls: Rc<Cell<usize>>,
    }

    impl RunHandle for FakeRun {
        fn wait(self: Box<Self>) -> io::Result<RunOutcome> {
            self.wait_calls.set(self.wait_calls.get() + 1);
            if let Some(error) = self.error {
                Err(io::Error::other(error))
            } else {
                Ok(self.outcome)
            }
        }
    }

    impl AgentSystemBackend for FakeBackend {
        fn start(&mut self, context: StartContext<'_>) -> io::Result<StartedRun> {
            self.start_calls.push(StartCall {
                agent_id: context.agent_id.to_string(),
                prompt: context.prompt.to_string(),
                worktree_dir: context.worktree_dir.to_path_buf(),
                worktree_existed: context.worktree_dir.is_dir(),
            });
            if let Some(error) = self.start_error.take() {
                return Err(io::Error::other(error));
            }
            Ok(StartedRun {
                session_id: self.session_id.clone(),
                handle: Box::new(FakeRun {
                    outcome: self.outcome.take().unwrap_or(RunOutcome::Completed),
                    error: self.wait_error.take(),
                    wait_calls: Rc::clone(&self.wait_calls),
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
