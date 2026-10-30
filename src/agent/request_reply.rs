//! Bounded inter-thread request/reply transport. Deadlines bound the caller's
//! wait; operation handling, cancellation, and error semantics belong to callers.

use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::Instant;

pub(super) struct Client<T, R> {
    sender: SyncSender<Call<T, R>>,
}

pub(super) struct Call<T, R> {
    pub(super) message: T,
    pub(super) deadline: Instant,
    reply: Option<SyncSender<R>>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum CallError {
    QueueFull,
    ReceiverClosed,
    Timeout,
    ReplyClosed,
}

pub(super) fn channel<T, R>(capacity: usize) -> (Client<T, R>, Receiver<Call<T, R>>) {
    let (sender, receiver) = mpsc::sync_channel(capacity);
    (Client { sender }, receiver)
}

impl<T, R> Client<T, R> {
    pub(super) fn call(&self, message: T, deadline: Instant) -> Result<R, CallError> {
        let (call, replies) = Call::new(message, deadline);
        self.sender.try_send(call).map_err(|error| match error {
            mpsc::TrySendError::Full(_) => CallError::QueueFull,
            mpsc::TrySendError::Disconnected(_) => CallError::ReceiverClosed,
        })?;
        replies
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => CallError::Timeout,
                mpsc::RecvTimeoutError::Disconnected => CallError::ReplyClosed,
            })
    }
}

impl<T, R> Call<T, R> {
    pub(super) fn new(message: T, deadline: Instant) -> (Self, Receiver<R>) {
        let (reply, replies) = mpsc::sync_channel(1);
        (
            Self {
                message,
                deadline,
                reply: Some(reply),
            },
            replies,
        )
    }

    // This tracks whether a response was attempted, not whether the caller is still waiting.
    pub(super) fn reply_pending(&self) -> bool {
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
    use std::thread;
    use std::time::Duration;

    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(2)
    }

    #[test]
    fn passes_owned_messages_deadlines_and_replies() {
        let (client, requests) = channel::<String, usize>(1);
        let expires = deadline();
        thread::scope(|scope| {
            let caller = scope.spawn(|| client.call("Unicode 雪".to_string(), expires));
            let mut call = requests.recv_timeout(Duration::from_secs(1)).unwrap();
            assert_eq!(call.message, "Unicode 雪");
            assert_eq!(call.deadline, expires);
            call.respond(call.message.len());
            assert_eq!(caller.join().unwrap(), Ok("Unicode 雪".len()));
        });
    }

    #[test]
    fn timeout_does_not_cancel_an_enqueued_message() {
        let (client, requests) = channel::<&str, ()>(1);
        assert_eq!(
            client.call("queued", Instant::now()),
            Err(CallError::Timeout)
        );
        let mut call = requests.recv().unwrap();
        assert_eq!(call.message, "queued");
        call.respond(());
        assert!(!call.reply_pending());
    }

    #[test]
    fn full_queue_does_not_enqueue_another_message() {
        let (client, requests) = channel::<&str, ()>(1);
        let (call, _replies) = Call::new("first", deadline());
        assert!(client.sender.try_send(call).is_ok());
        assert_eq!(client.call("second", deadline()), Err(CallError::QueueFull));
        assert_eq!(requests.recv().unwrap().message, "first");
        assert!(requests.try_recv().is_err());
    }

    #[test]
    fn distinguishes_request_receiver_exit_from_missing_reply() {
        let (client, requests) = channel::<(), ()>(1);
        thread::scope(|scope| {
            let caller = scope.spawn(|| client.call((), deadline()));
            drop(requests.recv_timeout(Duration::from_secs(1)).unwrap());
            assert_eq!(caller.join().unwrap(), Err(CallError::ReplyClosed));
        });
        drop(requests);
        assert_eq!(client.call((), deadline()), Err(CallError::ReceiverClosed));
    }

    #[test]
    fn responds_at_most_once() {
        let (mut call, replies) = Call::<(), usize>::new((), deadline());
        assert!(call.reply_pending());
        call.respond(1);
        call.respond(2);
        assert!(!call.reply_pending());
        assert_eq!(replies.recv().unwrap(), 1);
        assert_eq!(replies.try_recv(), Err(mpsc::TryRecvError::Disconnected));
    }
}
