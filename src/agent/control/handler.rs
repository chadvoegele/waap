use std::io;
use std::time::Instant;

use super::super::commands::CommandDispatcher;
use super::protocol::{build_rejection, build_response, parse_request};
use super::socket::SocketConnection;
use super::{ACK_TIMEOUT, IO_TIMEOUT};

pub(super) struct ControlRequestHandler<'a> {
    dispatcher: &'a CommandDispatcher,
}

impl<'a> ControlRequestHandler<'a> {
    pub(super) fn new(dispatcher: &'a CommandDispatcher) -> Self {
        Self { dispatcher }
    }

    pub(super) async fn handle(&self, mut connection: SocketConnection) -> io::Result<()> {
        let response = match connection.read_frame(IO_TIMEOUT).await {
            Ok(frame) => match parse_request(&frame) {
                Ok(request) => build_response(
                    &request.request_id,
                    self.dispatcher
                        .steer(&request.prompt, Instant::now() + ACK_TIMEOUT)
                        .await,
                ),
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
    use super::super::protocol::{Request, Response};
    use super::*;
    #[tokio::test(flavor = "current_thread")]
    async fn unsupported_backend_rejection_is_forwarded() {
        let request = Request::steer("steer");
        let id = request.request_id.clone();
        let dispatcher = CommandDispatcher::Unsupported("claude");
        let handler = ControlRequestHandler::new(&dispatcher);
        let (server, client) = tokio::net::UnixStream::pair().unwrap();
        let mut client = SocketConnection::new(client);
        let (result, response) =
            tokio::join!(handler.handle(SocketConnection::new(server)), async {
                client
                    .write_frame(&serde_json::to_vec(&request).unwrap())
                    .await
                    .unwrap();
                serde_json::from_slice::<Response>(&client.read_frame(IO_TIMEOUT).await.unwrap())
                    .unwrap()
            });
        result.unwrap();
        assert_eq!(response.request_id.as_deref(), Some(id.as_str()));
        assert_eq!(response.accepted, Some(false));
        assert_eq!(response.error.unwrap().code, "not_implemented");
    }
}
