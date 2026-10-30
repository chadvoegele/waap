use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use super::super::backend::RunHandle;
use super::protocol::{build_rejection, build_response, parse_request, Request, Response};
use super::socket::SocketConnection;
use super::{ACK_TIMEOUT, IO_TIMEOUT};

pub(in crate::agent) struct ControlRequestHandler<'a> {
    run: &'a dyn RunHandle,
}

impl<'a> ControlRequestHandler<'a> {
    pub(in crate::agent) fn new(run: &'a dyn RunHandle) -> Self {
        Self { run }
    }

    fn handle(&self, request: Request, completed: bool) -> Response {
        if completed {
            build_rejection(
                Some(&request.request_id),
                "agent_exited",
                "Agent exited before steering dispatch.",
            )
        } else {
            build_response(
                &request.request_id,
                self.run
                    .steer(&request.prompt, Instant::now() + ACK_TIMEOUT),
            )
        }
    }

    pub(in crate::agent) fn serve(
        &self,
        mut connection: SocketConnection,
        completed: &AtomicBool,
    ) -> io::Result<()> {
        connection.set_timeouts(IO_TIMEOUT, IO_TIMEOUT)?;
        let response = match connection.read_frame() {
            Ok(frame) => match parse_request(&frame) {
                Ok(request) => self.handle(request, completed.load(Ordering::Acquire)),
                Err(response) => response,
            },
            Err(error) if error.kind() == io::ErrorKind::InvalidData => build_rejection(
                None,
                "request_too_large",
                "Request exceeds 64 KiB including newline.",
            ),
            Err(error) => return Err(error),
        };
        connection.write_frame(&serde_json::to_vec(&response)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::backend::{RunOutcome, SteeringError};

    struct ExitedRun;

    impl RunHandle for ExitedRun {
        fn wait(&self) -> io::Result<RunOutcome> {
            panic!("handler must not wait for the backend")
        }

        fn steer(&self, _: &str, _: Instant) -> Result<(), SteeringError> {
            panic!("handler must not steer an exited backend")
        }
    }

    #[test]
    fn completed_run_is_rejected_without_backend_dispatch() {
        let request = Request::steer("do not dispatch");
        let id = request.request_id.clone();
        let response = ControlRequestHandler::new(&ExitedRun).handle(request, true);
        assert_eq!(response.request_id.as_deref(), Some(id.as_str()));
        assert_eq!(response.accepted, Some(false));
        assert_eq!(response.error.unwrap().code, "agent_exited");
    }
}
