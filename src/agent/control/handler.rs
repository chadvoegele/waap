use std::io;
use std::time::Instant;

use super::super::backend::RunHandle;
use super::protocol::{build_rejection, build_response, parse_request, Request, Response};
use super::socket::AsyncSocketConnection;
use super::ACK_TIMEOUT;

pub(in crate::agent) struct ControlRequestHandler<'a> {
    run: &'a dyn RunHandle,
}

impl<'a> ControlRequestHandler<'a> {
    pub(in crate::agent) fn new(run: &'a dyn RunHandle) -> Self {
        Self { run }
    }

    async fn handle(&self, request: Request) -> Response {
        build_response(
            &request.request_id,
            self.run
                .steer(&request.prompt, Instant::now() + ACK_TIMEOUT)
                .await,
        )
    }

    pub(in crate::agent) async fn serve(
        &self,
        mut connection: AsyncSocketConnection,
    ) -> io::Result<()> {
        let response = match connection.read_frame().await {
            Ok(frame) => match parse_request(&frame) {
                Ok(request) => self.handle(request).await,
                Err(response) => response,
            },
            Err(error) if error.kind() == io::ErrorKind::InvalidData => build_rejection(
                None,
                "request_too_large",
                "Request exceeds 64 KiB including newline.",
            ),
            Err(error) => return Err(error),
        };
        connection
            .write_frame(&serde_json::to_vec(&response)?)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::backend::{RunOutcome, SteeringError};

    struct RejectingRun;

    impl RunHandle for RejectingRun {
        fn wait(&self) -> crate::agent::backend::BackendFuture<'_, io::Result<RunOutcome>> {
            panic!("handler must not wait for the backend")
        }

        fn steer<'a>(
            &'a self,
            _: &'a str,
            _: Instant,
        ) -> crate::agent::backend::BackendFuture<'a, Result<(), SteeringError>> {
            Box::pin(async {
                Err(SteeringError::Rejected {
                    code: "no_active_turn",
                    message: "No active turn".to_string(),
                })
            })
        }
    }

    #[test]
    fn backend_rejection_is_forwarded_without_waiting_for_run_completion() {
        let request = Request::steer("steer");
        let id = request.request_id.clone();
        let response = crate::agent::runtime::block_on(
            ControlRequestHandler::new(&RejectingRun).handle(request),
        );
        assert_eq!(response.request_id.as_deref(), Some(id.as_str()));
        assert_eq!(response.accepted, Some(false));
        assert_eq!(response.error.unwrap().code, "no_active_turn");
    }
}
