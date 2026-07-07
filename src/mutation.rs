use std::io;

#[derive(Debug)]
pub(crate) struct Committed<T> {
    pub(crate) value: T,
    pub(crate) commit: String,
}

#[derive(Debug)]
pub(crate) enum MutationError {
    Operation(io::Error),
    Commit(io::Error),
}

impl From<io::Error> for MutationError {
    fn from(error: io::Error) -> Self {
        Self::Operation(error)
    }
}

pub(crate) type MutationResult<T> = Result<T, MutationError>;
