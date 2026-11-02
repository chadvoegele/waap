mod connection;
mod listener;
mod path;

pub(super) use connection::SocketConnection;
pub(in crate::agent) use listener::SocketListener;
pub(in crate::agent) use path::SocketPath;

pub(super) const MAX_FRAME_BYTES: usize = 64 * 1024;
