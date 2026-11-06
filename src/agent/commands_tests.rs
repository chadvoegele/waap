use super::*;
use futures_util::FutureExt;
use std::time::Duration;
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(2)
}

#[tokio::test(flavor = "current_thread")]
async fn stop_is_bounded_correlated_and_expiration_is_not_acceptance() {
    let (dispatcher, mut requests) = CommandDispatcher::create_channel();
    let (result, ()) = tokio::join!(dispatcher.stop(deadline()), async {
        let Command::Stop(mut request) = requests.recv().await.unwrap() else {
            panic!("expected stop")
        };
        assert!(dispatcher.stop(deadline()).now_or_never().is_none());
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
    let (dispatcher, mut requests) = CommandDispatcher::create_channel();
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
    let (dispatcher, mut requests) = CommandDispatcher::create_channel();
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
    let (dispatcher, mut requests) = CommandDispatcher::create_channel();
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
    let (dispatcher, receiver) = CommandDispatcher::create_channel();
    let (result, ()) = tokio::join!(dispatcher.steer("queued", deadline()), async {
        tokio::task::yield_now().await;
        drop(receiver);
    });
    assert!(matches!(
        result,
        Err(CommandError::AcceptanceUnknown { .. })
    ));
}
