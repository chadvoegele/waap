use std::io;
use std::path::Path;
use std::process::{ExitCode, ExitStatus};

use super::commands::Command;
use super::{claude::ClaudeBackend, codex::CodexBackend, opencode::OpencodeBackend};
use std::future::Future;
use tokio::sync::mpsc::Receiver;

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

pub(in crate::agent) async fn run_unsupported<F>(
    wait: F,
    receiver: Receiver<Command>,
    backend: &'static str,
) -> io::Result<RunOutcome>
where
    F: Future<Output = io::Result<RunOutcome>>,
{
    tokio::pin!(wait);
    let mut receiver = receiver;
    let mut channel_open = true;
    loop {
        tokio::select! {
            result = &mut wait => return result,
            request = receiver.recv(), if channel_open => match request {
                Some(Command::Steer(mut request)) => request.respond(Err(SteeringError::not_implemented(backend))),
                None => channel_open = false,
            }
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
            Self::Claude(runner) => run_unsupported(runner.run(), receiver, "claude").await,
            Self::Codex(runner) => runner.run(receiver).await,
            Self::Opencode(runner) => run_unsupported(runner.run(), receiver, "opencode").await,
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

pub(super) trait AgentSystemBackend {
    async fn start(
        &mut self,
        context: StartContext<'_>,
        receiver: Receiver<Command>,
    ) -> io::Result<AgentSessionRun>;
    async fn abort(&mut self, context: AbortContext<'_>) -> io::Result<()>;
}

pub(super) enum Backend {
    Claude(ClaudeBackend),
    Codex(CodexBackend),
    Opencode(OpencodeBackend),
}
impl AgentSystemBackend for Backend {
    async fn start(
        &mut self,
        context: StartContext<'_>,
        receiver: Receiver<Command>,
    ) -> io::Result<AgentSessionRun> {
        match self {
            Self::Claude(b) => b.start(context, receiver).await,
            Self::Codex(b) => b.start(context, receiver).await,
            Self::Opencode(b) => b.start(context, receiver).await,
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
        pub(crate) steer_calls: Arc<Mutex<Vec<(String, Instant, std::thread::ThreadId)>>>,
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
    pub(in crate::agent) struct FakeRunner {
        pub(in crate::agent) outcome: Option<RunOutcome>,
        pub(in crate::agent) error: Option<String>,
        pub(in crate::agent) action: Option<Box<dyn FnOnce() -> io::Result<()> + Send>>,
        pub(in crate::agent) wait_calls: Arc<AtomicUsize>,
        pub(in crate::agent) steer_calls: Arc<Mutex<Vec<(String, Instant, std::thread::ThreadId)>>>,
        pub(in crate::agent) steer_error: Option<SteeringError>,
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
                    Some(Command::Steer(mut request)) = commands.recv() => {
                        self.steer_calls.lock().unwrap().push((request.message.clone(), request.deadline, std::thread::current().id()));
                        request.respond(self.steer_error.clone().map_or(Ok(()), Err));
                    }
                }
            }
        }
    }
    impl AgentSystemBackend for FakeBackend {
        async fn start(
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

#[cfg(test)]
mod execution_tests {
    use super::*;
    use crate::agent::commands::CommandDispatcher;
    use futures_util::FutureExt;
    use std::time::{Duration, Instant};

    fn session(receiver: Receiver<Command>) -> AgentSessionRun {
        AgentSessionRun::new(
            "test".into(),
            Runner::Fake(fake::FakeRunner {
                outcome: Some(RunOutcome::Completed),
                error: None,
                action: None,
                wait_calls: Default::default(),
                steer_calls: Default::default(),
                steer_error: None,
            }),
            receiver,
        )
    }

    #[tokio::test(flavor = "current_thread")]
    async fn completed_session_releases_receiver_while_retaining_runner() {
        let (dispatcher, receiver) = CommandDispatcher::channel();
        let mut session = session(receiver);
        assert_eq!(session.run().await.unwrap(), RunOutcome::Completed);
        assert!(session.receiver.is_none());
        assert!(matches!(
            dispatcher
                .steer("completed", Instant::now() + Duration::from_secs(1))
                .await,
            Err(SteeringError::Rejected {
                code: "agent_exited",
                ..
            })
        ));
        session.runner.cleanup().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancelled_session_releases_queued_replies_before_or_after_polling() {
        for polled in [false, true] {
            let (dispatcher, receiver) = CommandDispatcher::channel();
            let mut session = session(receiver);
            let (release, wait) = std::sync::mpsc::channel();
            let Runner::Fake(runner) = &mut session.runner else {
                unreachable!()
            };
            runner.action = Some(Box::new(move || {
                wait.recv_timeout(Duration::from_secs(3))
                    .map_err(io::Error::other)
            }));
            let mut execution = Box::pin(session.run());
            if polled {
                assert!(execution.as_mut().now_or_never().is_none());
            }
            let steering = dispatcher.steer("queued", Instant::now() + Duration::from_secs(30));
            tokio::pin!(steering);
            assert!(steering.as_mut().now_or_never().is_none());
            drop(execution);
            let _ = release.send(());
            assert!(session.receiver.is_none());
            assert!(matches!(
                tokio::time::timeout(Duration::from_millis(100), steering)
                    .await
                    .unwrap(),
                Err(SteeringError::AcceptanceUnknown { .. })
            ));
            assert!(matches!(
                dispatcher
                    .steer("cancelled", Instant::now() + Duration::from_secs(1))
                    .await,
                Err(SteeringError::Rejected {
                    code: "agent_exited",
                    ..
                })
            ));
            session.runner.cleanup().await.unwrap();
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn completed_execution_closes_command_transport() {
        let (dispatcher, receiver) = CommandDispatcher::channel();
        assert_eq!(
            run_unsupported(async { Ok(RunOutcome::Completed) }, receiver, "claude")
                .await
                .unwrap(),
            RunOutcome::Completed
        );
        assert!(matches!(
            dispatcher
                .steer("after completion", Instant::now() + Duration::from_secs(1))
                .await,
            Err(SteeringError::Rejected {
                code: "agent_exited",
                ..
            })
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancelling_execution_drops_queued_replies_without_waiting_for_deadline() {
        for polled in [false, true] {
            let (dispatcher, receiver) = CommandDispatcher::channel();
            let mut execution = Box::pin(run_unsupported(
                std::future::pending(),
                receiver,
                "opencode",
            ));
            if polled {
                assert!(execution.as_mut().now_or_never().is_none());
            }
            let steering = dispatcher.steer("queued", Instant::now() + Duration::from_secs(30));
            tokio::pin!(steering);
            assert!(steering.as_mut().now_or_never().is_none());
            drop(execution);
            assert!(matches!(
                tokio::time::timeout(Duration::from_millis(100), steering)
                    .await
                    .unwrap(),
                Err(SteeringError::AcceptanceUnknown { .. })
            ));
            assert!(matches!(
                dispatcher
                    .steer(
                        "after cancellation",
                        Instant::now() + Duration::from_secs(1)
                    )
                    .await,
                Err(SteeringError::Rejected {
                    code: "agent_exited",
                    ..
                })
            ));
        }
    }
}
