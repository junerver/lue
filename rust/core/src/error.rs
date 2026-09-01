//! Error type shared by the core library and its bindings.

#[derive(Debug, thiserror::Error)]
pub enum LueError {
    /// Filesystem/IO failure (maps to Python's OSError).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// Invalid input or stale state (maps to Python's ValueError).
    #[error("{0}")]
    Invalid(String),
    /// Byte decoding failure (maps to Python's UnicodeDecodeError).
    #[error("decode error: {0}")]
    Decode(String),
}
