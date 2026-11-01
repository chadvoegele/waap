use std::io;
use std::time::Instant;

use super::super::commands::CommandHandle;
use super::protocol::{build_rejection, build_response, parse_request, Request, Response};
use super::socket::SocketConnection;
use super::{ACK_TIMEOUT, IO_TIMEOUT};

pub(in crate::agent) struct ControlRequestHandler<'a> {
    run: &'a CommandHandle,
}

impl<'a> ControlRequestHandler<'a> {
    pub(in crate::agent) fn new(run: &'a CommandHandle) -> Self {
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

    pub(in crate::agent) async fn serve(&self, mut connection: SocketConnection) -> io::Result<()> {
        let response = match connection.read_frame(IO_TIMEOUT).await {
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
    #[tokio::test(flavor = "current_thread")]
    async fn unsupported_backend_rejection_is_forwarded() {
        let request = Request::steer("steer");
        let id = request.request_id.clone();
        let handle = CommandHandle::Unsupported("claude");
        let response = ControlRequestHandler::new(&handle).handle(request).await;
        assert_eq!(response.request_id.as_deref(), Some(id.as_str()));
        assert_eq!(response.accepted, Some(false));
        assert_eq!(response.error.unwrap().code, "not_implemented");
    }
}
