use std::time::Instant;
use tokio::sync::{mpsc, oneshot};

use super::backend::SteeringError;

pub(super) type SteeringResult = Result<(), SteeringError>;

pub(super) enum Command {
    Steer(CommandRequest),
    Stop(CommandRequest),
}

pub(super) struct CommandRequest {
    pub(super) message: String,
    pub(super) deadline: Instant,
    reply_sender: Option<oneshot::Sender<SteeringResult>>,
}

impl CommandRequest {
    pub(super) fn new(
        message: String,
        deadline: Instant,
    ) -> (Self, oneshot::Receiver<SteeringResult>) {
        let (reply_sender, reply_receiver) = oneshot::channel();
        (
            Self {
                message,
                deadline,
                reply_sender: Some(reply_sender),
            },
            reply_receiver,
        )
    }

    pub(super) fn has_reply_sender(&self) -> bool {
        self.reply_sender.is_some()
    }

    pub(super) fn respond(&mut self, result: SteeringResult) {
        if let Some(reply_sender) = self.reply_sender.take() {
            let _ = reply_sender.send(result);
        }
    }
}

#[derive(Clone)]
pub(super) struct CommandDispatcher {
    sender: mpsc::Sender<Command>,
}

impl CommandDispatcher {
    pub(super) fn channel() -> (Self, mpsc::Receiver<Command>) {
        let (sender, receiver) = mpsc::channel(1);
        (Self { sender }, receiver)
    }

    pub(super) async fn steer(&self, prompt: &str, deadline: Instant) -> SteeringResult {
        self.dispatch(prompt, deadline, false).await
    }

    pub(super) async fn stop(&self, deadline: Instant) -> SteeringResult {
        self.dispatch("", deadline, true).await
    }

    async fn dispatch(&self, prompt: &str, deadline: Instant, stop: bool) -> SteeringResult {
        if Instant::now() >= deadline {
            return Err(SteeringError::Rejected {
                code: "timeout",
                message: "Control request expired before dispatch".into(),
            });
        }
        let (request, reply_receiver) = CommandRequest::new(prompt.to_string(), deadline);
        self.sender
            .try_send(if stop {
                Command::Stop(request)
            } else {
                Command::Steer(request)
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => SteeringError::Rejected {
                    code: "busy",
                    message: "Control dispatch is busy".into(),
                },
                mpsc::error::TrySendError::Closed(_) => SteeringError::Rejected {
                    code: "agent_exited",
                    message: "Backend exited before control dispatch".into(),
                },
            })?;
        match tokio::time::timeout_at(deadline.into(), reply_receiver).await {
            Ok(Ok(result)) => result,
            _ => Err(SteeringError::acceptance_unknown(
                "no reply received before the deadline or driver exit",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::FutureExt;
    use std::time::Duration;
    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(2)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stop_is_bounded_correlated_and_expiration_is_not_acceptance() {
        let (dispatcher, mut requests) = CommandDispatcher::channel();
        let (result, ()) = tokio::join!(dispatcher.stop(deadline()), async {
            let Command::Stop(mut request) = requests.recv().await.unwrap() else {
                panic!("expected stop")
            };
            assert!(request.message.is_empty());
            assert!(dispatcher.stop(deadline()).now_or_never().is_none());
            // The second request fills the bounded queue until this actor consumes it.
            assert!(matches!(
                dispatcher.stop(deadline()).await,
                Err(SteeringError::Rejected { code: "busy", .. })
            ));
            request.respond(Ok(()));
        });
        assert_eq!(result, Ok(()));
        drop(requests.recv().await.unwrap());
        assert!(matches!(
            dispatcher.stop(Instant::now()).await,
            Err(SteeringError::Rejected {
                code: "timeout",
                ..
            })
        ));
        let (result, ()) = tokio::join!(dispatcher.stop(deadline()), async {
            drop(requests.recv().await.unwrap());
        });
        assert!(matches!(
            result,
            Err(SteeringError::AcceptanceUnknown { .. })
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn owned_commands_wake_actor_and_correlate_replies() {
        let (dispatcher, mut requests) = CommandDispatcher::channel();
        let expires = deadline();
        let (result, ()) = tokio::join!(dispatcher.steer("Unicode 雪", expires), async {
            let Command::Steer(mut request) = requests.recv().await.unwrap() else {
                panic!("expected steering command")
            };
            assert_eq!(request.message, "Unicode 雪");
            assert_eq!(request.deadline, expires);
            request.respond(Ok(()));
            request.respond(Err(SteeringError::not_implemented("ignored")));
            assert!(!request.has_reply_sender());
        });
        assert_eq!(result, Ok(()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn queue_is_bounded_and_runner_exit_is_distinguished_from_uncertain_delivery() {
        let (dispatcher, mut requests) = CommandDispatcher::channel();
        let sender = dispatcher.sender.clone();
        let (request, _reply_receiver) = CommandRequest::new("first".into(), deadline());
        sender.try_send(Command::Steer(request)).ok().unwrap();
        assert!(matches!(
            dispatcher.steer("second", deadline()).await,
            Err(SteeringError::Rejected { code: "busy", .. })
        ));
        let Command::Steer(request) = requests.recv().await.unwrap() else {
            panic!("expected steering command")
        };
        assert_eq!(request.message, "first");
        assert!(requests.try_recv().is_err());
        let (result, ()) = tokio::join!(dispatcher.steer("no reply", deadline()), async {
            drop(requests.recv().await.unwrap());
        });
        assert!(matches!(
            result,
            Err(SteeringError::AcceptanceUnknown { .. })
        ));
        drop(requests);
        assert!(matches!(
            dispatcher.steer("after exit", deadline()).await,
            Err(SteeringError::Rejected {
                code: "agent_exited",
                ..
            })
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn caller_timeout_does_not_retract_enqueued_command() {
        let (dispatcher, mut requests) = CommandDispatcher::channel();
        assert!(matches!(
            dispatcher
                .steer("queued", Instant::now() + Duration::from_millis(1))
                .await,
            Err(SteeringError::AcceptanceUnknown { .. })
        ));
        let Command::Steer(request) = requests.recv().await.unwrap() else {
            panic!("expected steering command")
        };
        assert_eq!(request.message, "queued");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dropping_receiver_releases_queued_reply_senders() {
        let (dispatcher, receiver) = CommandDispatcher::channel();
        let (result, ()) = tokio::join!(dispatcher.steer("queued", deadline()), async {
            tokio::task::yield_now().await;
            drop(receiver);
        });
        assert!(matches!(
            result,
            Err(SteeringError::AcceptanceUnknown { .. })
        ));
    }
}
