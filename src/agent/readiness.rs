//! Level-triggered readiness and coalescing inter-thread wakeups.
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::time::Instant;

pub(super) struct Wakeup {
    reader: UnixStream,
    writer: UnixStream,
}

impl Wakeup {
    pub(super) fn new() -> io::Result<Self> {
        let (reader, writer) = UnixStream::pair()?;
        reader.set_nonblocking(true)?;
        writer.set_nonblocking(true)?;
        Ok(Self { reader, writer })
    }

    pub(super) fn signal(&self) {
        // A full socket already carries a notification. Enqueue state first.
        loop {
            match (&self.writer).write(&[1]) {
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                _ => break,
            }
        }
    }

    pub(super) fn drain(&self) -> io::Result<()> {
        let mut bytes = [0; 1024];
        loop {
            match (&self.reader).read(&mut bytes) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e),
            }
        }
    }

    pub(super) fn fd(&self) -> RawFd {
        self.reader.as_raw_fd()
    }

    pub(super) fn signal_writer(&self) -> io::Result<UnixStream> {
        self.writer.try_clone()
    }
}

pub(super) fn wait(fds: &mut [libc::pollfd], deadline: Option<Instant>) -> io::Result<()> {
    loop {
        let timeout = deadline.map_or(-1, |deadline| {
            let remaining = deadline.saturating_duration_since(Instant::now());
            remaining
                .as_millis()
                .saturating_add(u128::from(remaining.subsec_nanos() % 1_000_000 != 0))
                .min(i32::MAX as u128) as i32
        });
        // SAFETY: poll receives a valid mutable array for the duration of the call.
        let result = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) };
        if result >= 0 {
            if fds.iter().any(|fd| fd.revents & libc::POLLNVAL != 0) {
                return Err(io::Error::other("invalid readiness descriptor"));
            }
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

pub(super) fn interest(fd: RawFd, events: libc::c_short) -> libc::pollfd {
    libc::pollfd {
        fd,
        events,
        revents: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coalesced_notifications_and_rearming() {
        let wake = Wakeup::new().unwrap();
        for _ in 0..100_000 {
            wake.signal();
        }
        let mut fds = [interest(wake.fd(), libc::POLLIN)];
        wait(&mut fds, Some(Instant::now())).unwrap();
        assert_ne!(fds[0].revents & libc::POLLIN, 0);
        wake.drain().unwrap();
        wait(&mut fds, Some(Instant::now())).unwrap();
        assert_eq!(fds[0].revents, 0);
        wake.signal();
        wait(&mut fds, None).unwrap();
        assert_ne!(fds[0].revents & libc::POLLIN, 0);
    }
}
