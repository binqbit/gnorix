use std::{error::Error as StdError, fmt};

pub type Result<T> = std::result::Result<T, Error>;

/// Invalid transport data or a failure from the shared authorization pipeline.
#[derive(Debug)]
pub enum Error {
    InvalidInput(&'static str),
    Core(gnorix_core::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(reason) => write!(f, "invalid NEAR authorization: {reason}"),
            Self::Core(error) => write!(f, "Gnorix authorization: {error}"),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Core(error) => Some(error),
            _ => None,
        }
    }
}

impl From<gnorix_core::Error> for Error {
    fn from(error: gnorix_core::Error) -> Self {
        Self::Core(error)
    }
}
