use std::time::Instant;
use tokio::sync::{mpsc, oneshot};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum CommandError {
    Rejected { code: &'static str, message: String },
    AcceptanceUnknown { message: String },
}

impl CommandError {
    pub(super) fn not_implemented(backend: &str, method: &str) -> Self {
        Self::Rejected {
            code: "not_implemented",
            message: format!("{method} not implemented for backend {backend}"),
        }
    }
    pub(super) fn acceptance_unknown(reason: &str) -> Self {
        Self::AcceptanceUnknown {
            message: format!("Control acceptance is unknown: {reason}; do not retry automatically"),
        }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected { message, .. } | Self::AcceptanceUnknown { message } => {
                f.write_str(message)
            }
        }
    }
}
impl std::error::Error for CommandError {}

pub(super) type CommandResult = Result<(), CommandError>;

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Operation {
    Steer { prompt: String },
    Stop {},
}

pub(super) enum Command {
    Steer {
        prompt: String,
        request: CommandRequest,
    },
    Stop(CommandRequest),
}

pub(super) struct CommandRequest {
    pub(super) deadline: Instant,
    reply_sender: Option<oneshot::Sender<CommandResult>>,
}

impl CommandRequest {
    pub(super) fn new(deadline: Instant) -> (Self, oneshot::Receiver<CommandResult>) {
        let (reply_sender, reply_receiver) = oneshot::channel();
        (
            Self {
                deadline,
                reply_sender: Some(reply_sender),
            },
            reply_receiver,
        )
    }

    pub(super) fn has_reply_sender(&self) -> bool {
        self.reply_sender.is_some()
    }

    pub(super) fn respond(&mut self, result: CommandResult) {
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

    pub(super) async fn steer(&self, prompt: &str, deadline: Instant) -> CommandResult {
        self.dispatch(
            Operation::Steer {
                prompt: prompt.into(),
            },
            deadline,
        )
        .await
    }

    pub(super) async fn stop(&self, deadline: Instant) -> CommandResult {
        self.dispatch(Operation::Stop {}, deadline).await
    }

    async fn dispatch(&self, operation: Operation, deadline: Instant) -> CommandResult {
        if Instant::now() >= deadline {
            return Err(CommandError::Rejected {
                code: "timeout",
                message: "Control request expired before dispatch".into(),
            });
        }
        let (request, reply_receiver) = CommandRequest::new(deadline);
        self.sender
            .try_send(match operation {
                Operation::Stop {} => Command::Stop(request),
                Operation::Steer { prompt } => Command::Steer { prompt, request },
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => CommandError::Rejected {
                    code: "busy",
                    message: "Control dispatch is busy".into(),
                },
                mpsc::error::TrySendError::Closed(_) => CommandError::Rejected {
                    code: "agent_exited",
                    message: "Backend exited before control dispatch".into(),
                },
            })?;
        match tokio::time::timeout_at(deadline.into(), reply_receiver).await {
            Ok(Ok(result)) => result,
            _ => Err(CommandError::acceptance_unknown(
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
            assert!(dispatcher.stop(deadline()).now_or_never().is_none());
            // The second request fills the bounded queue until this actor consumes it.
            assert!(matches!(
                dispatcher.stop(deadline()).await,
                Err(CommandError::Rejected { code: "busy", .. })
            ));
            request.respond(Ok(()));
        });
        assert_eq!(result, Ok(()));
        drop(requests.recv().await.unwrap());
        assert!(matches!(
            dispatcher.stop(Instant::now()).await,
            Err(CommandError::Rejected {
                code: "timeout",
                ..
            })
        ));
        let (result, ()) = tokio::join!(dispatcher.stop(deadline()), async {
            drop(requests.recv().await.unwrap());
        });
        assert!(matches!(
            result,
            Err(CommandError::AcceptanceUnknown { .. })
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn owned_commands_wake_actor_and_correlate_replies() {
        let (dispatcher, mut requests) = CommandDispatcher::channel();
        let expires = deadline();
        let (result, ()) = tokio::join!(dispatcher.steer("Unicode 雪", expires), async {
            let Command::Steer {
                prompt,
                mut request,
            } = requests.recv().await.unwrap()
            else {
                panic!("expected steering command")
            };
            assert_eq!(prompt, "Unicode 雪");
            assert_eq!(request.deadline, expires);
            request.respond(Ok(()));
            request.respond(Err(CommandError::not_implemented("ignored", "steering")));
            assert!(!request.has_reply_sender());
        });
        assert_eq!(result, Ok(()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn queue_is_bounded_and_runner_exit_is_distinguished_from_uncertain_delivery() {
        let (dispatcher, mut requests) = CommandDispatcher::channel();
        let sender = dispatcher.sender.clone();
        let (request, _reply_receiver) = CommandRequest::new(deadline());
        sender
            .try_send(Command::Steer {
                prompt: "first".into(),
                request,
            })
            .ok()
            .unwrap();
        assert!(matches!(
            dispatcher.steer("second", deadline()).await,
            Err(CommandError::Rejected { code: "busy", .. })
        ));
        let Command::Steer { prompt, .. } = requests.recv().await.unwrap() else {
            panic!("expected steering command")
        };
        assert_eq!(prompt, "first");
        assert!(requests.try_recv().is_err());
        let (result, ()) = tokio::join!(dispatcher.steer("no reply", deadline()), async {
            drop(requests.recv().await.unwrap());
        });
        assert!(matches!(
            result,
            Err(CommandError::AcceptanceUnknown { .. })
        ));
        drop(requests);
        assert!(matches!(
            dispatcher.steer("after exit", deadline()).await,
            Err(CommandError::Rejected {
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
            Err(CommandError::AcceptanceUnknown { .. })
        ));
        let Command::Steer { prompt, .. } = requests.recv().await.unwrap() else {
            panic!("expected steering command")
        };
        assert_eq!(prompt, "queued");
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
            Err(CommandError::AcceptanceUnknown { .. })
        ));
    }
}
