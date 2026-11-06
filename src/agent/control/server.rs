use super::super::commands::{CommandDispatcher, Operation};
use super::protocol::{build_rejection, build_response, parse_request};
use super::socket::{SocketConnection, SocketListener};
use super::{ACK_TIMEOUT, IO_TIMEOUT};
use futures_util::{stream::FuturesUnordered, FutureExt, StreamExt};
use std::io;
use std::time::Instant;

pub(in crate::agent) struct ControlServer {
    listener: SocketListener,
    dispatcher: CommandDispatcher,
}

impl ControlServer {
    pub(in crate::agent) fn new(listener: SocketListener, dispatcher: CommandDispatcher) -> Self {
        Self {
            listener,
            dispatcher,
        }
    }

    pub(in crate::agent) async fn run(
        &self,
        shutdown: impl std::future::Future<Output = ()>,
    ) -> io::Result<()> {
        const MAX_HANDLERS: usize = 8;
        tokio::pin!(shutdown);
        let mut handlers = FuturesUnordered::new();
        loop {
            tokio::select! {
                biased;
                result = handlers.next(), if !handlers.is_empty() => {
                    if matches!(result, Some(Err(_))) { log::error!("control handler panicked"); }
                },
                _ = &mut shutdown => break,
                connection = self.listener.accept() => {
                    match connection {
                        Ok(stream) if handlers.len() < MAX_HANDLERS => {
                            handlers.push(std::panic::AssertUnwindSafe(
                                self.handle(SocketConnection::new(stream))
                            ).catch_unwind());
                        }
                        Ok(stream) => drop(stream),
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {},
                        Err(error) => return Err(error),
                    }
                }
            }
        }
        while handlers.next().await.is_some() {}
        Ok(())
    }

    async fn handle(&self, mut connection: SocketConnection) -> io::Result<()> {
        let response = self.read_response(&mut connection).await?;
        connection
            .write_frame(&serde_json::to_vec(&response)?)
            .await
    }

    async fn read_response(
        &self,
        connection: &mut SocketConnection,
    ) -> io::Result<super::protocol::Response> {
        let frame = match connection.read_frame(IO_TIMEOUT).await {
            Ok(frame) => frame,
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                return Ok(build_rejection(
                    None,
                    "request_too_large",
                    "Request exceeds 64 KiB including newline; send a shorter request.",
                ))
            }
            Err(error) => return Err(error),
        };
        let request = match parse_request(&frame) {
            Ok(request) => request,
            Err(response) => return Ok(response),
        };
        let deadline = Instant::now() + ACK_TIMEOUT;
        let result = match request.operation {
            Operation::Steer { prompt } => self.dispatcher.steer(&prompt, deadline).await,
            Operation::Stop {} => self.dispatcher.stop(deadline).await,
        };
        Ok(build_response(&request.request_id, result))
    }

    pub(in crate::agent) fn close(&mut self) -> io::Result<()> {
        self.listener.close()
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
