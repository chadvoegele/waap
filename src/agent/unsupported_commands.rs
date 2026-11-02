use std::future::Future;
use std::io;
use tokio::sync::mpsc::Receiver;

use super::backend::{RunOutcome, SteeringError};
use super::commands::Command;

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
                Some(Command::Steer(mut request) | Command::Stop(mut request)) => request.respond(Err(SteeringError::not_implemented(backend))),
                None => channel_open = false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::commands::CommandDispatcher;
    use std::time::{Duration, Instant};

    #[tokio::test(flavor = "current_thread")]
    async fn socket_stop_is_explicitly_rejected_by_unsupported_backends() {
        for backend in ["claude", "opencode"] {
            let (dispatcher, receiver) = CommandDispatcher::channel();
            let (finish, finished) = tokio::sync::oneshot::channel();
            let (outcome, ()) = tokio::join!(
                await_with_unsupported_commands(
                    async {
                        finished.await.unwrap();
                        Ok(RunOutcome::Completed)
                    },
                    receiver,
                    backend
                ),
                async {
                    let result = dispatcher
                        .stop(Instant::now() + Duration::from_secs(1))
                        .await;
                    assert_eq!(result, Err(SteeringError::not_implemented(backend)));
                    finish.send(()).unwrap();
                }
            );
            assert_eq!(outcome.unwrap(), RunOutcome::Completed);
        }
    }
}
