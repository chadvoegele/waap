use super::*;
use crate::agent::commands::CommandDispatcher;
use std::time::{Duration, Instant};

#[tokio::test(flavor = "current_thread")]
async fn socket_stop_is_explicitly_rejected_by_unsupported_backends() {
    for backend in ["claude", "opencode"] {
        let (dispatcher, receiver) = CommandDispatcher::create_channel();
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
                assert_eq!(result, Err(CommandError::not_implemented(backend, "stop")));
                finish.send(()).unwrap();
            }
        );
        assert_eq!(outcome.unwrap(), RunOutcome::Completed);
    }
}
