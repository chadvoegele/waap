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
    pub(super) fn create_channel() -> (Self, mpsc::Receiver<Command>) {
        const QUEUE_CAPACITY: usize = 1;
        let (sender, receiver) = mpsc::channel(QUEUE_CAPACITY);
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
                    message: "Control dispatch is busy; wait for the pending request to finish before sending a new request".into(),
                },
                mpsc::error::TrySendError::Closed(_) => CommandError::Rejected {
                    code: "agent_exited",
                    message: "Backend exited before control dispatch; inspect agent status with waap agent get".into(),
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
#[path = "commands_tests.rs"]
mod tests;
