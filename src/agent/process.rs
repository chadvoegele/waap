use std::io;
use std::process::ExitStatus;
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

pub(super) struct AgentProcess {
    child: Child,
    reaped: bool,
}

impl AgentProcess {
    pub(super) fn spawn(command: &mut Command) -> io::Result<Self> {
        let child = command.kill_on_drop(true).spawn().map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "failed to start {}: {error}",
                    command.as_std().get_program().to_string_lossy()
                ),
            )
        })?;
        Ok(Self {
            child,
            reaped: false,
        })
    }
    pub(super) fn take_stdin(&mut self) -> io::Result<ChildStdin> {
        self.child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("agent process stdin is unavailable"))
    }
    pub(super) fn take_stdout(&mut self) -> io::Result<ChildStdout> {
        self.child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("agent process stdout is unavailable"))
    }
    pub(super) async fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = self.child.wait().await?;
        self.reaped = true;
        Ok(status)
    }
    pub(super) async fn terminate(&mut self) -> io::Result<()> {
        if self.reaped {
            return Ok(());
        }
        if self.child.try_wait()?.is_none() {
            self.child.start_kill()?;
        }
        self.wait().await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(flavor = "current_thread")]
    #[cfg(target_os = "linux")]
    async fn explicit_cleanup_kills_and_reaps_and_is_idempotent() {
        let mut command = Command::new("sh");
        command.args(["-c", "while :; do :; done"]);
        let mut process = AgentProcess::spawn(&mut command).unwrap();
        let pid = process.child.id().unwrap();
        process.terminate().await.unwrap();
        process.terminate().await.unwrap();
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    }
    #[tokio::test(flavor = "current_thread")]
    #[cfg(target_os = "linux")]
    async fn cancellation_fallback_kills_and_tokio_reaps() {
        let mut command = Command::new("sh");
        command.args(["-c", "while :; do :; done"]);
        let process = AgentProcess::spawn(&mut command).unwrap();
        let path = std::path::PathBuf::from(format!("/proc/{}", process.child.id().unwrap()));
        drop(process);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while path.exists() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
    #[tokio::test(flavor = "current_thread")]
    async fn wait_preserves_exit_status_and_disarms_cleanup() {
        let mut command = Command::new("sh");
        command.args(["-c", "exit 7"]);
        let mut process = AgentProcess::spawn(&mut command).unwrap();
        assert_eq!(process.wait().await.unwrap().code(), Some(7));
        process.terminate().await.unwrap();
        assert!(process.reaped);
    }
    #[tokio::test(flavor = "current_thread")]
    async fn missing_pipes_leave_process_owned_for_cleanup() {
        let mut command = Command::new("sh");
        command.args(["-c", "while :; do :; done"]);
        let mut process = AgentProcess::spawn(&mut command).unwrap();
        assert!(process.take_stdin().is_err());
        assert!(process.take_stdout().is_err());
        process.terminate().await.unwrap();
    }
}
