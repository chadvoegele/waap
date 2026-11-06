use std::time::Duration;

pub(super) mod client;
mod protocol;
mod server;
mod socket;

pub(super) use server::ControlServer;
#[cfg(test)]
use socket::SocketConnection;
pub(super) use socket::{SocketListener, SocketPath};

// Connect, frame read, and frame write each have a five-second deadline.
// Dispatch allows 30 seconds for native acknowledgement; the client waits
// another five seconds for response delivery. Shutdown drains at most eight
// concurrent handlers, each bounded by read + dispatch + write (40 seconds).
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const ACK_TIMEOUT: Duration = Duration::from_secs(30);

#[cfg(test)]
#[path = "control_tests.rs"]
pub(in crate::agent) mod tests;
