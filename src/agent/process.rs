use std::io;
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus};

/// Owns a local backend process until it is reaped, including failed startup.
pub(super) struct AgentProcess {
    child: Child,
    reaped: bool,
}

impl AgentProcess {
    pub(super) fn spawn(command: &mut Command) -> io::Result<Self> {
        let child = command.spawn().map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "failed to start {}: {error}",
                    command.get_program().to_string_lossy()
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

    pub(super) fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = self.child.wait()?;
        self.reaped = true;
        Ok(status)
    }
}

impl Drop for AgentProcess {
    fn drop(&mut self) {
        if self.reaped {
            return;
        }
        if let Err(error) = self.child.kill() {
            log::error!(
                "failed to terminate agent process {}: {error}",
                self.child.id()
            );
        }
        if let Err(error) = self.wait() {
            log::error!("failed to reap agent process {}: {error}", self.child.id());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "linux")]
    fn dropping_unfinished_process_kills_and_reaps_it() {
        let mut command = Command::new("sh");
        command.args(["-c", "while :; do :; done"]);
        let process = AgentProcess::spawn(&mut command).unwrap();
        let pid = process.child.id();
        drop(process);
        // A reaped process has neither a running PID nor a zombie entry.
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    }

    #[test]
    fn wait_preserves_exit_status_and_disarms_cleanup() {
        let mut command = Command::new("sh");
        command.args(["-c", "exit 7"]);
        let mut process = AgentProcess::spawn(&mut command).unwrap();
        assert_eq!(process.wait().unwrap().code(), Some(7));
        assert!(process.reaped);
    }

    #[test]
    fn missing_pipes_return_errors_and_leave_process_owned() {
        let mut command = Command::new("sh");
        command.args(["-c", "while :; do :; done"]);
        let mut process = AgentProcess::spawn(&mut command).unwrap();
        assert!(process.take_stdin().is_err());
        assert!(process.take_stdout().is_err());
    }
}
