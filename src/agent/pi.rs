use std::io;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::signal::unix::{signal, Signal, SignalKind};
use tokio::sync::mpsc::Receiver;

use super::backend::{
    AbortContext, AbortOutcome, AgentSessionRun, RunOutcome, Runner, SessionOpen, StartContext,
};
use super::commands::Command;
use super::control::{client::ControlClient, SocketPath};
use super::process::AgentProcess;
use super::AgentRunOptions;
use config::{build_pi_command, build_version_command, PiRunConfig};
use resume::ExactSession;
use rpc::PiClient;

mod config;
mod resume;
mod rpc;
mod session;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Default)]
pub(super) struct PiBackend {
    config: PiRunConfig,
}

impl PiBackend {
    pub(super) fn load_from_env(options: &AgentRunOptions) -> io::Result<Self> {
        Ok(Self {
            config: PiRunConfig::load_from_env(options)?,
        })
    }

    pub(super) async fn abort(&mut self, context: AbortContext<'_>) -> io::Result<AbortOutcome> {
        let root = context
            .control_root
            .as_ref()
            .map_err(|error| io::Error::new(error.kind(), error.to_string()))?;
        let path = SocketPath::for_agent(root, context.agent_id)?;
        async { ControlClient::connect(path.as_path()).await?.stop().await }
            .await
            .map_err(|error| {
                io::Error::new(error.kind(), format!("agent {}: {error}", context.agent_id))
            })?;
        Ok(AbortOutcome::StopAccepted)
    }

    pub(super) async fn open(
        &mut self,
        intent: &SessionOpen,
        context: StartContext<'_>,
        receiver: Receiver<Command>,
    ) -> io::Result<AgentSessionRun> {
        let mut sigterm = signal(SignalKind::terminate())?;
        let mut sigint = signal(SignalKind::interrupt())?;
        let resume = match intent {
            SessionOpen::Fresh => None,
            SessionOpen::ResumeExact(id) => {
                Some(ExactSession::locate(id, context.repository_root)?)
            }
        };
        if resume.is_some() {
            self.verify_resume_version(context.repository_root, &mut sigterm, &mut sigint)
                .await?;
        }
        let mut command = build_pi_command(
            &self.config,
            context.agent_id,
            context.repository_root,
            resume.as_ref(),
        );
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut process = AgentProcess::spawn(&mut command)?;
        let startup = async {
            let mut client = PiClient::new(
                BufReader::new(process.take_stdout()?),
                process.take_stdin()?,
                tokio::io::stdout(),
            );
            let deadline = Instant::now() + COMMAND_TIMEOUT;
            let state = client.fetch_state(deadline).await?;
            if let Some(session) = &resume {
                session.verify(&state)?;
            }
            Ok::<_, io::Error>((client, state.session_id))
        };
        let started = tokio::select! {
            result = startup => result,
            _ = sigterm.recv() => Err(startup_interrupted()),
            _ = sigint.recv() => Err(startup_interrupted()),
        };
        let (client, session_id) = match started {
            Ok(started) => started,
            Err(error) => {
                return super::run::collapse_errors(Err(error), process.terminate().await)
            }
        };
        Ok(AgentSessionRun::new(
            session_id,
            Runner::Pi(Box::new(PiRunner {
                process,
                client,
                sigterm,
                sigint,
                prompt: context.prompt.into(),
            })),
            receiver,
        ))
    }

    async fn verify_resume_version(
        &self,
        checkout: &Path,
        sigterm: &mut Signal,
        sigint: &mut Signal,
    ) -> io::Result<()> {
        // get_state does not expose the effective tool cwd. Resume relies on
        // the verified native open and header-cwd behavior of this Pi release.
        let mut command = build_version_command(&self.config, checkout);
        command.stdout(Stdio::piped());
        let mut process = AgentProcess::spawn(&mut command)?;
        let probe = async {
            let mut version = Vec::new();
            process
                .take_stdout()?
                .take(64)
                .read_to_end(&mut version)
                .await?;
            if !matches!(version.as_slice(), b"1.5.2\n" | b"1.5.2\r\n" | b"1.5.2") {
                return Err(io::Error::new(io::ErrorKind::Unsupported,
                    "exact session resume requires verified Pi version 1.5.2; select it with WAAP_PI_BIN"));
            }
            if !process.wait().await?.success() {
                return Err(io::Error::other("Pi --version exited unsuccessfully"));
            }
            Ok(())
        };
        let result = tokio::select! {
            result = tokio::time::timeout(COMMAND_TIMEOUT, probe) => result.unwrap_or_else(|_| {
                Err(io::Error::new(io::ErrorKind::TimedOut, "timed out checking Pi --version"))
            }),
            _ = sigterm.recv() => Err(startup_interrupted()),
            _ = sigint.recv() => Err(startup_interrupted()),
        };
        super::run::collapse_errors(result, process.terminate().await)
    }
}

fn startup_interrupted() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Pi startup interrupted")
}

pub(super) struct PiRunner {
    process: AgentProcess,
    client: PiClient<BufReader<ChildStdout>, ChildStdin, tokio::io::Stdout>,
    sigterm: Signal,
    sigint: Signal,
    prompt: String,
}

impl PiRunner {
    pub(super) async fn run(&mut self, mut receiver: Receiver<Command>) -> io::Result<RunOutcome> {
        let sigterm = &mut self.sigterm;
        let sigint = &mut self.sigint;
        let outcome = self
            .client
            .run_session(
                &self.prompt,
                &mut receiver,
                async {
                    tokio::select! { _ = sigterm.recv() => {}, _ = sigint.recv() => {} }
                },
                COMMAND_TIMEOUT,
            )
            .await?;
        self.client.close_stdin();
        let status = tokio::time::timeout(COMMAND_TIMEOUT, async {
            tokio::select! {
                result = self.process.wait() => result,
                result = self.client.drain_shutdown() => {
                    result?;
                    self.process.wait().await
                }
            }
        })
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "Pi RPC did not exit after stdin closed",
            )
        })??;
        if !status.success() {
            return Err(io::Error::other(format!(
                "Pi RPC exited with status {status}"
            )));
        }
        Ok(outcome)
    }

    pub(super) async fn cleanup(&mut self) -> io::Result<()> {
        self.process.terminate().await
    }
}
