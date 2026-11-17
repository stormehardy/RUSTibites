//! Error type shared by the simulation core.
//!
//! The core is I/O free by design, so errors carry human-readable context
//! instead of foreign error types. The `bibites-io` crate wraps these errors
//! and adds file-format context.

use core::fmt;

/// Result alias used throughout RUSTibites.
pub type Result<T> = core::result::Result<T, Error>;

/// Everything that can go wrong inside the simulation core or its file-format
/// adapters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Underlying I/O failure (message only, so the core stays independent of
    /// platform error types).
    Io(String),
    /// Parsed data is malformed or violates an invariant.
    InvalidData(String),
    /// A file version, setting or feature is not supported.
    Unsupported(String),
    /// An internal invariant was violated; this is a bug.
    Internal(String),
}

impl Error {
    /// Creates an [`Error::InvalidData`] from anything displayable.
    pub fn invalid_data(message: impl Into<String>) -> Self {
        Self::InvalidData(message.into())
    }

    /// Creates an [`Error::Unsupported`] from anything displayable.
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::Unsupported(message.into())
    }

    /// Creates an [`Error::Internal`] from anything displayable.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => write!(f, "I/O error: {message}"),
            Self::InvalidData(message) => write!(f, "invalid data: {message}"),
            Self::Unsupported(message) => write!(f, "unsupported: {message}"),
            Self::Internal(message) => write!(f, "internal error: {message}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

impl From<String> for Error {
    fn from(message: String) -> Self {
        Self::InvalidData(message)
    }
}

impl From<&str> for Error {
    fn from(message: &str) -> Self {
        Self::InvalidData(message.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_includes_context() {
        let error = Error::invalid_data("gene 3 out of range");
        assert_eq!(error.to_string(), "invalid data: gene 3 out of range");
    }

    #[test]
    fn converts_io_errors() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let error: Error = io.into();
        assert!(matches!(error, Error::Io(_)));
    }
}
