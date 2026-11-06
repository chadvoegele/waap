use super::*;
use crate::agent::commands::CommandDispatcher;
use futures_util::FutureExt;
use std::time::{Duration, Instant};

fn session(receiver: Receiver<Command>) -> AgentSessionRun {
    AgentSessionRun::new(
        "test".into(),
        Runner::Fake(fake::FakeRunner {
            outcome: Some(RunOutcome::Completed),
            error: None,
            action: None,
            wait_calls: Default::default(),
            steer_calls: Default::default(),
            steer_error: None,
        }),
        receiver,
    )
}

#[tokio::test(flavor = "current_thread")]
async fn completed_session_releases_receiver_while_retaining_runner() {
    let (dispatcher, receiver) = CommandDispatcher::create_channel();
    let mut session = session(receiver);
    assert_eq!(session.run().await.unwrap(), RunOutcome::Completed);
    assert!(session.receiver.is_none());
    assert!(matches!(
        dispatcher
            .steer("completed", Instant::now() + Duration::from_secs(1))
            .await,
        Err(CommandError::Rejected {
            code: "agent_exited",
            ..
        })
    ));
    session.runner.cleanup().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_session_releases_queued_replies_before_or_after_polling() {
    for polled in [false, true] {
        let (dispatcher, receiver) = CommandDispatcher::create_channel();
        let mut session = session(receiver);
        let (release, wait) = std::sync::mpsc::channel();
        let Runner::Fake(runner) = &mut session.runner else {
            unreachable!()
        };
        runner.action = Some(Box::new(move || {
            wait.recv_timeout(Duration::from_secs(3))
                .map_err(io::Error::other)
        }));
        let mut execution = Box::pin(session.run());
        if polled {
            assert!(execution.as_mut().now_or_never().is_none());
        }
        let steering = dispatcher.steer("queued", Instant::now() + Duration::from_secs(30));
        tokio::pin!(steering);
        assert!(steering.as_mut().now_or_never().is_none());
        drop(execution);
        let _ = release.send(());
        assert!(session.receiver.is_none());
        assert!(matches!(
            tokio::time::timeout(Duration::from_millis(100), steering)
                .await
                .unwrap(),
            Err(CommandError::AcceptanceUnknown { .. })
        ));
        assert!(matches!(
            dispatcher
                .steer("cancelled", Instant::now() + Duration::from_secs(1))
                .await,
            Err(CommandError::Rejected {
                code: "agent_exited",
                ..
            })
        ));
        session.runner.cleanup().await.unwrap();
    }
}
