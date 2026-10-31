//! Bounded async request/reply transport. Deadlines bound the caller's wait;
//! cancellation and operation acceptance semantics belong to the backend.
use std::time::Instant;
use tokio::sync::{mpsc, oneshot};

pub(super) struct Client<T, R> {
    sender: mpsc::Sender<Call<T, R>>,
}

pub(super) struct Call<T, R> {
    pub(super) message: T,
    pub(super) deadline: Instant,
    reply: Option<oneshot::Sender<R>>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum CallError {
    QueueFull,
    ReceiverClosed,
    Timeout,
    ReplyClosed,
}

pub(super) fn channel<T, R>(capacity: usize) -> (Client<T, R>, mpsc::Receiver<Call<T, R>>) {
    let (sender, receiver) = mpsc::channel(capacity);
    (Client { sender }, receiver)
}

impl<T, R> Client<T, R> {
    pub(super) async fn call(&self, message: T, deadline: Instant) -> Result<R, CallError> {
        let (call, replies) = Call::new(message, deadline);
        self.sender.try_send(call).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => CallError::QueueFull,
            mpsc::error::TrySendError::Closed(_) => CallError::ReceiverClosed,
        })?;
        tokio::time::timeout_at(deadline.into(), replies)
            .await
            .map_err(|_| CallError::Timeout)?
            .map_err(|_| CallError::ReplyClosed)
    }
}

impl<T, R> Call<T, R> {
    pub(super) fn new(message: T, deadline: Instant) -> (Self, oneshot::Receiver<R>) {
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

    pub(super) fn respond(&mut self, response: R) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(response);
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
    async fn passes_owned_messages_deadlines_and_replies() {
        let (client, mut requests) = channel::<String, usize>(1);
        let expires = deadline();
        let (result, ()) = tokio::join!(client.call("Unicode 雪".to_string(), expires), async {
            let mut call = requests.recv().await.unwrap();
            assert_eq!(call.message, "Unicode 雪");
            assert_eq!(call.deadline, expires);
            call.respond(call.message.len());
        });
        assert_eq!(result, Ok("Unicode 雪".len()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn timeout_does_not_cancel_an_enqueued_message() {
        let (client, mut requests) = channel::<&str, ()>(1);
        assert_eq!(
            client.call("queued", Instant::now()).await,
            Err(CallError::Timeout)
        );
        let mut call = requests.recv().await.unwrap();
        assert_eq!(call.message, "queued");
        call.respond(());
        assert!(!call.has_reply_sender());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn full_queue_does_not_enqueue_another_message() {
        let (client, mut requests) = channel::<&str, ()>(1);
        let (call, _replies) = Call::new("first", deadline());
        assert!(client.sender.try_send(call).is_ok());
        assert_eq!(
            client.call("second", deadline()).await,
            Err(CallError::QueueFull)
        );
        assert_eq!(requests.recv().await.unwrap().message, "first");
        assert!(requests.try_recv().is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn distinguishes_request_receiver_exit_from_missing_reply() {
        let (client, mut requests) = channel::<(), ()>(1);
        let (result, ()) = tokio::join!(client.call((), deadline()), async {
            drop(requests.recv().await.unwrap());
        });
        assert_eq!(result, Err(CallError::ReplyClosed));
        drop(requests);
        assert_eq!(
            client.call((), deadline()).await,
            Err(CallError::ReceiverClosed)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn responds_at_most_once() {
        let (mut call, mut replies) = Call::<(), usize>::new((), deadline());
        assert!(call.has_reply_sender());
        call.respond(1);
        call.respond(2);
        assert!(!call.has_reply_sender());
        assert_eq!(replies.try_recv(), Ok(1));
        assert_eq!(
            replies.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        );
    }
}
