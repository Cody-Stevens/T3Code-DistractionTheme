use std::{fmt, io};

/// Shared failures. Context strings must be static labels, never tokens or response bodies.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Filesystem or stream failure; callers must not attach secret contents.
    Io(io::Error),
    /// Invalid dimensions, feature arrays, command, or other caller input.
    InvalidInput(&'static str),
    /// An external executable failed; output is deliberately excluded.
    ExternalTool {
        /// Executable label, without command-line arguments.
        program: &'static str,
        /// Exit code, or `None` when terminated without a code.
        code: Option<i32>,
    },
    /// HTTP status failure, without headers or response body.
    Http {
        /// HTTP status code.
        status: u16,
    },
    /// Transport or wire-format failure with non-sensitive static context.
    Protocol(&'static str),
    /// An interface is defined but its phase-two implementation is pending.
    NotImplemented(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::InvalidInput(message) => write!(f, "invalid input: {message}"),
            Self::ExternalTool { program, code } => write!(f, "{program} failed (exit {code:?})"),
            Self::Http { status } => write!(f, "HTTP {status}"),
            Self::Protocol(message) => write!(f, "protocol error: {message}"),
            Self::NotImplemented(component) => write!(f, "{component} is not implemented yet"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

/// Result type shared by crate boundaries.
pub type Result<T> = std::result::Result<T, Error>;
