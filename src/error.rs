use std::io;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum NdsError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Session already exists: {0}")]
    SessionAlreadyExists(String),

    #[error("PTY error: {0}")]
    PtyError(String),

    #[error("Fork error: {0}")]
    ForkError(String),

    #[error("Socket error: {0}")]
    SocketError(String),

    #[error("JSON serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Invalid session ID: {0}")]
    InvalidSessionId(String),

    #[error("Session is already attached")]
    SessionAlreadyAttached,

    #[error("Failed to create session directory: {0}")]
    DirectoryCreationError(String),

    #[error("Signal error: {0}")]
    SignalError(String),

    #[error("Terminal error: {0}")]
    TerminalError(String),

    #[error("Process error: {0}")]
    ProcessError(String),
}

pub type Result<T> = std::result::Result<T, NdsError>;

impl From<nix::Error> for NdsError {
    fn from(err: nix::Error) -> Self {
        match err {
            nix::Error::EPERM => NdsError::PermissionDenied(err.to_string()),
            _ => NdsError::PtyError(err.to_string()),
        }
    }
}
