use std::future::Future;
use std::io;
use tokio::sync::mpsc::Receiver;

use super::backend::RunOutcome;
use super::commands::Command;
use super::commands::CommandError;

pub(super) async fn await_with_unsupported_commands<F>(
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
                Some(Command::Steer { mut request, .. }) => request.respond(Err(CommandError::not_implemented(backend, "steering"))),
                Some(Command::Stop(mut request)) => request.respond(Err(CommandError::not_implemented(backend, "stop"))),
                None => channel_open = false,
            }
        }
    }
}

#[cfg(test)]
#[path = "unsupported_commands_tests.rs"]
mod tests;
