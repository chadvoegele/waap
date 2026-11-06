use std::io;
use std::path::Path;

use super::protocol::{Request, Response};
use super::socket::SocketConnection;
use super::{ACK_TIMEOUT, IO_TIMEOUT};

pub(in crate::agent) struct ControlClient {
    connection: SocketConnection,
}

impl ControlClient {
    pub(in crate::agent) async fn connect(path: &Path) -> io::Result<Self> {
        let connection = SocketConnection::connect(path).await?;
        Ok(Self { connection })
    }

    pub(in crate::agent) async fn steer(&mut self, prompt: &str) -> io::Result<()> {
        self.exchange(Request::steer(prompt)).await
    }

    pub(in crate::agent) async fn stop(&mut self) -> io::Result<()> {
        self.exchange(Request::stop()).await
    }

    async fn exchange(&mut self, request: Request) -> io::Result<()> {
        let payload = serde_json::to_vec(&request)?;
        self.connection
            .write_frame(&payload)
            .await
            .map_err(|error| {
                if error.kind() == io::ErrorKind::InvalidInput {
                    error
                } else {
                    build_acceptance_unknown_error(&format!(
                        "request write failed (agent may be busy or unavailable): {error}"
                    ))
                }
            })?;
        let frame = self
            .connection
            .read_frame(ACK_TIMEOUT + IO_TIMEOUT)
            .await
            .map_err(|error| {
                build_acceptance_unknown_error(&format!(
                    "acknowledgement failed (agent may be busy or unavailable): {error}"
                ))
            })?;
        validate_response(&frame, &request.request_id)
    }
}

fn validate_response(frame: &[u8], request_id: &str) -> io::Result<()> {
    let invalid = || build_acceptance_unknown_error("invalid or uncorrelated control response");
    let response: Response = serde_json::from_slice(frame).map_err(|_| invalid())?;
    if response.version != super::protocol::PROTOCOL_VERSION
        || response.request_id.as_deref() != Some(request_id)
    {
        return Err(invalid());
    }
    match (response.accepted, response.error) {
        (Some(true), None) => Ok(()),
        (accepted, Some(error)) if !error.code.is_empty() && !error.message.is_empty() => {
            match (accepted, error.code.as_str()) {
                (Some(false), code) if code != "acceptance_unknown" => {
                    Err(io::Error::other(format!("{code}: {}", error.message)))
                }
                (None, "acceptance_unknown") => Err(build_acceptance_unknown_error(&error.message)),
                _ => Err(invalid()),
            }
        }
        _ => Err(invalid()),
    }
}

fn build_acceptance_unknown_error(reason: &str) -> io::Error {
    io::Error::other(format!(
        "acceptance is unknown: {reason}; no automatic retry was attempted"
    ))
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
