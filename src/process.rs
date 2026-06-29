use std::io::{self, Read, Write};
use std::process::{Command as ProcessCommand, ExitStatus, Stdio};
use std::thread;

/// Run `command` in the foreground, forwarding the child's stdout and stderr to
/// this process's stdout and stderr, and return the child's exit status.
///
/// `on_started` runs once the child has been spawned (used to mark the agent
/// running). The child's stdin is connected to /dev/null so it can be launched
/// with background tools such as `nohup` or `setsid` without a controlling
/// terminal; output is governed by normal shell redirection of this process's
/// stdout/stderr.
pub(crate) fn run_forwarding<F>(
    command: &mut ProcessCommand,
    on_started: F,
) -> io::Result<ExitStatus>
where
    F: FnOnce() -> io::Result<()>,
{
    forward(command, &mut io::stdout(), &mut io::stderr(), on_started)
}

fn forward<O, E, F>(
    command: &mut ProcessCommand,
    out: &mut O,
    err: &mut E,
    on_started: F,
) -> io::Result<ExitStatus>
where
    O: Write,
    E: Write + Send,
    F: FnOnce() -> io::Result<()>,
{
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    on_started()?;

    let mut child_out = child.stdout.take().expect("stdout is piped");
    let mut child_err = child.stderr.take().expect("stderr is piped");

    thread::scope(|scope| -> io::Result<()> {
        let err_handle = scope.spawn(move || copy(&mut child_err, err));
        copy(&mut child_out, out)?;
        err_handle
            .join()
            .expect("stderr forwarding thread panicked")
    })?;

    child.wait()
}

fn copy<R: Read, W: Write>(reader: &mut R, writer: &mut W) -> io::Result<()> {
    let mut buf = [0u8; 8192];
    loop {
        let read = reader.read(&mut buf)?;
        if read == 0 {
            return Ok(());
        }
        writer.write_all(&buf[..read])?;
        writer.flush()?;
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command as ProcessCommand;

    use super::forward;

    #[test]
    fn forward_copies_stdout_and_stderr_to_writers() {
        let mut command = ProcessCommand::new("sh");
        command.args(["-c", "printf to-out; printf to-err 1>&2"]);

        let mut out = Vec::new();
        let mut err = Vec::new();
        let status = forward(&mut command, &mut out, &mut err, || Ok(())).unwrap();

        assert!(status.success());
        assert_eq!(out, b"to-out");
        assert_eq!(err, b"to-err");
    }

    #[test]
    fn forward_propagates_exit_code() {
        let mut command = ProcessCommand::new("sh");
        command.args(["-c", "exit 7"]);

        let mut out = Vec::new();
        let mut err = Vec::new();
        let status = forward(&mut command, &mut out, &mut err, || Ok(())).unwrap();

        assert_eq!(status.code(), Some(7));
    }

    #[test]
    fn forward_runs_on_started_after_spawn() {
        let mut command = ProcessCommand::new("sh");
        command.args(["-c", "exit 0"]);

        let mut started = false;
        let mut out = Vec::new();
        let mut err = Vec::new();
        forward(&mut command, &mut out, &mut err, || {
            started = true;
            Ok(())
        })
        .unwrap();

        assert!(started);
    }

    #[test]
    fn forward_closes_stdin_so_background_processes_do_not_block() {
        // `cat` with no arguments reads stdin until EOF. With stdin connected to
        // /dev/null it sees EOF immediately, which is what lets the process run
        // detached in the background under nohup/setsid without a terminal.
        let mut command = ProcessCommand::new("sh");
        command.args(["-c", "cat; printf done"]);

        let mut out = Vec::new();
        let mut err = Vec::new();
        let status = forward(&mut command, &mut out, &mut err, || Ok(())).unwrap();

        assert!(status.success());
        assert_eq!(out, b"done");
    }
}
