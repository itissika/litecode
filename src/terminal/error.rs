//! TerminalHub error surface.

use std::fmt;

/// Stable error codes for Hub / WS / tools.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalError {
    SessionNotFound(String),
    Spawn(String),
    Io(String),
    Closed(String),
    /// Foreground `exec_once` hit wall-clock timeout after kill. `partial_output`
    /// is whatever was drained from the PTY before/after kill (may be empty).
    Timeout {
        partial_output: String,
    },
    /// Foreground `exec_once` cancelled. Same partial-output contract as Timeout.
    Cancelled {
        partial_output: String,
    },
}

impl fmt::Display for TerminalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SessionNotFound(id) => write!(f, "terminal session '{id}' not found"),
            Self::Spawn(msg) => write!(f, "terminal spawn failed: {msg}"),
            Self::Io(msg) => write!(f, "terminal I/O error: {msg}"),
            Self::Closed(id) => write!(f, "terminal session '{id}' is closed"),
            Self::Timeout { .. } => write!(f, "terminal command timed out"),
            Self::Cancelled { .. } => write!(f, "terminal command cancelled"),
        }
    }
}

impl std::error::Error for TerminalError {}

pub type TerminalResult<T> = Result<T, TerminalError>;
