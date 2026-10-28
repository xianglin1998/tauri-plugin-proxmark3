//! Error type shared by the plugin commands and the native bridge.

use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The Proxmark3 client could not be initialised or refused the operation.
    #[error("{0}")]
    Client(String),

    /// No session is open.
    #[error("no Proxmark3 session is open, call connect() first")]
    NotOpen,

    /// A NUL byte was found in a string handed to the C library.
    #[error("string contains a NUL byte")]
    NulByte,

    /// A blocking task failed to complete.
    #[error("background task failed: {0}")]
    Join(String),

    /// Filesystem/session directory problem.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Error {
    pub(crate) fn client(message: impl Into<String>) -> Self {
        Self::Client(message.into())
    }
}

impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
