use std::io;

pub(super) fn new() -> io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
}

#[cfg(test)]
pub(super) fn block_on<F: std::future::Future>(future: F) -> F::Output {
    new().unwrap().block_on(future)
}
