use std::io;
use std::path::Path;
use std::process::{ExitCode, ExitStatus};

use super::commands::Command;
#[cfg(test)]
use super::commands::CommandError;
use super::{claude::ClaudeBackend, codex::CodexBackend, opencode::OpencodeBackend};
use std::future::Future;
use tokio::sync::mpsc::Receiver;

#[derive(Debug, PartialEq)]
pub(super) enum RunOutcome {
    Completed,
    Aborted,
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

pub(super) struct AgentSessionRun {
    pub(super) session_id: String,
    pub(super) runner: Runner,
    receiver: Option<Receiver<Command>>,
}

impl AgentSessionRun {
    pub(super) fn new(session_id: String, runner: Runner, receiver: Receiver<Command>) -> Self {
        Self {
            session_id,
            runner,
            receiver: Some(receiver),
        }
    }

    pub(super) fn run(&mut self) -> impl Future<Output = io::Result<RunOutcome>> + '_ {
        let receiver = self
            .receiver
            .take()
            .expect("agent session can only run once");
        self.runner.run(receiver)
    }
}

pub(super) enum Runner {
    Claude(super::claude::ClaudeRunner),
    Codex(Box<super::codex::CodexRunner>),
    Opencode(super::opencode::OpencodeRunner),
    #[cfg(test)]
    Fake(fake::FakeRunner),
}

impl Runner {
    pub(super) async fn run(&mut self, receiver: Receiver<Command>) -> io::Result<RunOutcome> {
        match self {
            Self::Claude(runner) => runner.run(receiver).await,
            Self::Codex(runner) => runner.run(receiver).await,
            Self::Opencode(runner) => runner.run(receiver).await,
            #[cfg(test)]
            Self::Fake(runner) => runner.run(receiver).await,
        }
    }

    pub(super) async fn cleanup(&mut self) -> io::Result<()> {
        match self {
            Self::Claude(runner) => runner.cleanup().await,
            Self::Codex(runner) => runner.cleanup().await,
            Self::Opencode(_) => Ok(()),
            #[cfg(test)]
            Self::Fake(_) => Ok(()),
        }
    }
}

/// Acceptance leaves cleanup and terminal persistence to the live run owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AbortOutcome {
    StopAccepted,
    Stopped,
}

pub(super) struct AbortContext<'a> {
    pub(super) waap_root: &'a Path,
    pub(super) session_id: Option<&'a str>,
    pub(super) agent_id: &'a str,
    pub(super) control_root: &'a io::Result<crate::root::ControlRoot>,
}

pub(super) enum Backend {
    Claude(ClaudeBackend),
    Codex(CodexBackend),
    Opencode(OpencodeBackend),
    #[cfg(test)]
    Fake(fake::FakeBackend),
}
impl Backend {
    pub(super) async fn start(
        &mut self,
        context: StartContext<'_>,
        receiver: Receiver<Command>,
    ) -> io::Result<AgentSessionRun> {
        match self {
            Self::Claude(b) => b.start(context, receiver).await,
            Self::Codex(b) => b.start(context, receiver).await,
            Self::Opencode(b) => b.start(context, receiver).await,
            #[cfg(test)]
            Self::Fake(b) => b.start(context, receiver).await,
        }
    }
    pub(super) async fn abort(&mut self, context: AbortContext<'_>) -> io::Result<AbortOutcome> {
        match self {
            Self::Claude(b) => b.abort(context).await,
            Self::Codex(b) => b.abort(context).await,
            Self::Opencode(b) => b.abort(context).await,
            #[cfg(test)]
            Self::Fake(b) => b.abort(context).await,
        }
    }
}
#[cfg(test)]
impl Backend {
    pub(super) fn fake(&self) -> &fake::FakeBackend {
        let Self::Fake(backend) = self else {
            panic!("expected fake backend")
        };
        backend
    }

    pub(super) fn type_name(&self) -> &'static str {
        match self {
            Self::Claude(_) => std::any::type_name::<ClaudeBackend>(),
            Self::Codex(_) => std::any::type_name::<CodexBackend>(),
            Self::Opencode(_) => std::any::type_name::<OpencodeBackend>(),
            Self::Fake(_) => std::any::type_name::<fake::FakeBackend>(),
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
        pub(crate) session_id: Option<String>,
        pub(crate) agent_id: String,
    }

    pub(crate) struct FakeBackend {
        pub(crate) session_id: String,
        pub(crate) outcome: Option<RunOutcome>,
        pub(crate) start_error: Option<String>,
        pub(crate) wait_error: Option<String>,
        pub(crate) wait_action: Option<Box<dyn FnOnce() -> io::Result<()> + Send>>,
        pub(crate) abort_error: Option<String>,
        pub(crate) abort_outcome: AbortOutcome,
        pub(crate) start_calls: Vec<StartCall>,
        pub(crate) wait_calls: Arc<AtomicUsize>,
        pub(crate) steer_calls: Arc<Mutex<Vec<(String, Instant, std::thread::ThreadId)>>>,
        pub(crate) steer_error: Option<CommandError>,
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
                abort_outcome: AbortOutcome::Stopped,
                start_calls: Vec::new(),
                wait_calls: Arc::new(AtomicUsize::new(0)),
                steer_calls: Arc::new(Mutex::new(Vec::new())),
                steer_error: None,
                abort_calls: Vec::new(),
            }
        }
    }
    pub(in crate::agent) struct FakeRunner {
        pub(in crate::agent) outcome: Option<RunOutcome>,
        pub(in crate::agent) error: Option<String>,
        pub(in crate::agent) action: Option<Box<dyn FnOnce() -> io::Result<()> + Send>>,
        pub(in crate::agent) wait_calls: Arc<AtomicUsize>,
        pub(in crate::agent) steer_calls: Arc<Mutex<Vec<(String, Instant, std::thread::ThreadId)>>>,
        pub(in crate::agent) steer_error: Option<CommandError>,
    }
    impl FakeRunner {
        pub(super) async fn run(
            &mut self,
            mut commands: mpsc::Receiver<Command>,
        ) -> io::Result<RunOutcome> {
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
                    Some(command) = commands.recv() => {
                        match command {
                            Command::Steer { prompt, mut request } => {
                                self.steer_calls.lock().unwrap().push((prompt, request.deadline, std::thread::current().id()));
                                request.respond(self.steer_error.clone().map_or(Ok(()), Err));
                            }
                            Command::Stop(mut request) => request.respond(self.steer_error.clone().map_or(Ok(()), Err)),
                        }
                    }
                }
            }
        }
    }
    impl FakeBackend {
        pub(super) async fn start(
            &mut self,
            context: StartContext<'_>,
            receiver: Receiver<Command>,
        ) -> io::Result<AgentSessionRun> {
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
            Ok(AgentSessionRun::new(
                self.session_id.clone(),
                Runner::Fake(FakeRunner {
                    outcome: self.outcome.take(),
                    error: self.wait_error.take(),
                    action: self.wait_action.take(),
                    wait_calls: Arc::clone(&self.wait_calls),
                    steer_calls: Arc::clone(&self.steer_calls),
                    steer_error: self.steer_error.clone(),
                }),
                receiver,
            ))
        }
        pub(super) async fn abort(
            &mut self,
            context: AbortContext<'_>,
        ) -> io::Result<AbortOutcome> {
            self.abort_calls.push(AbortCall {
                waap_root: context.waap_root.into(),
                session_id: context.session_id.map(str::to_owned),
                agent_id: context.agent_id.into(),
            });
            self.abort_error
                .as_ref()
                .map_or(Ok(self.abort_outcome), |error| {
                    Err(io::Error::other(error.clone()))
                })
        }
    }
}

#[cfg(test)]
#[path = "backend_execution_tests.rs"]
mod execution_tests;
#[cfg(test)]
#[path = "backend_tests.rs"]
mod tests;
