use std::time::Instant;
use tokio::sync::{mpsc, oneshot};

use super::backend::SteeringError;

pub(super) type SteeringResult = Result<(), SteeringError>;

pub(super) enum Command {
    Steer(SteerRequest),
}

pub(super) struct SteerRequest {
    pub(super) message: String,
    pub(super) deadline: Instant,
    reply: Option<oneshot::Sender<SteeringResult>>,
}

impl SteerRequest {
    pub(super) fn new(
        message: String,
        deadline: Instant,
    ) -> (Self, oneshot::Receiver<SteeringResult>) {
        let (reply, replies) = oneshot::channel();
        (
            Self {
                message,
                deadline,
                reply: Some(reply),
            },
            replies,
        )
    }

    pub(super) fn has_reply_sender(&self) -> bool {
        self.reply.is_some()
    }

    pub(super) fn respond(&mut self, result: SteeringResult) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(result);
        }
    }
}

#[derive(Clone)]
pub(super) enum CommandDispatcher {
    Channel(mpsc::Sender<Command>),
    Unsupported(&'static str),
}

impl CommandDispatcher {
    pub(super) fn channel() -> (Self, mpsc::Receiver<Command>) {
        let (sender, receiver) = mpsc::channel(1);
        (Self::Channel(sender), receiver)
    }

    pub(super) async fn steer(&self, prompt: &str, deadline: Instant) -> SteeringResult {
        let sender = match self {
            Self::Channel(sender) => sender,
            Self::Unsupported(backend) => return Err(SteeringError::not_implemented(backend)),
        };
        if Instant::now() >= deadline {
            return Err(SteeringError::Rejected {
                code: "timeout",
                message: "Steering request expired before dispatch".into(),
            });
        }
        let (request, replies) = SteerRequest::new(prompt.to_string(), deadline);
        sender
            .try_send(Command::Steer(request))
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => SteeringError::Rejected {
                    code: "busy",
                    message: "Steering dispatch is busy".into(),
                },
                mpsc::error::TrySendError::Closed(_) => SteeringError::Rejected {
                    code: "agent_exited",
                    message: "Backend exited before steering dispatch".into(),
                },
            })?;
        match tokio::time::timeout_at(deadline.into(), replies).await {
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
    use std::time::Duration;
    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(2)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn owned_commands_wake_actor_and_correlate_replies() {
        let (dispatcher, mut requests) = CommandDispatcher::channel();
        let expires = deadline();
        let (result, ()) = tokio::join!(dispatcher.steer("Unicode 雪", expires), async {
            let Command::Steer(mut request) = requests.recv().await.unwrap();
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
        let CommandDispatcher::Channel(sender) = &dispatcher else {
            unreachable!()
        };
        let (request, _reply) = SteerRequest::new("first".into(), deadline());
        sender.try_send(Command::Steer(request)).ok().unwrap();
        assert!(matches!(
            dispatcher.steer("second", deadline()).await,
            Err(SteeringError::Rejected { code: "busy", .. })
        ));
        let Command::Steer(request) = requests.recv().await.unwrap();
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
        let Command::Steer(request) = requests.recv().await.unwrap();
        assert_eq!(request.message, "queued");
    }
}
