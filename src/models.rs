//! Data exchanged with the frontend.

use serde::{Deserialize, Serialize};

/// State of the client session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    /// A session is open (offline sessions included).
    pub open: bool,
    /// The session runs without a device.
    pub offline: bool,
    /// Transport of the session (`""` when offline).
    pub port: String,
    /// Proxmark3 client version compiled into the library.
    pub version: String,
}

/// Result of one command execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandOutput {
    /// proxmark3 client status code (0 == success).
    pub status: i32,
    /// Everything the client printed while the command ran, with ANSI escape
    /// sequences and progress redraws removed.
    pub output: String,
    /// Wall clock duration of the call.
    pub duration_ms: u64,
}
