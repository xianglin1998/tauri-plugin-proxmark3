//! Tauri commands exposed to the frontend (`plugin:proxmark3|...`).

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

use crate::error::Error;
use crate::models::{CommandOutput, SessionInfo};
use crate::{bridge, home_dir};

/// Payload of the `proxmark3://output` event emitted while a streaming command
/// runs.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputChunk {
    pub chunk: String,
}

/// Opens a Proxmark3 session.
///
/// * `port` omitted / `null` – offline session, or the first device the client
///   auto-detects;
/// * `"/dev/ttyACM0"`, `"COM3"` – a local serial port;
/// * `"tcp:host:port"`, `"udp:host:port"` – a network bridge;
/// * `"bt:AA:BB:..."` – Bluetooth (Linux builds with BlueZ only).
#[tauri::command]
pub async fn connect(port: Option<String>) -> Result<SessionInfo, Error> {
    let home = home_dir().cloned();
    tauri::async_runtime::spawn_blocking(move || bridge::connect(port.as_deref(), home.as_deref()))
        .await
        .map_err(|error| Error::Join(error.to_string()))?
}

/// Closes the current session.
#[tauri::command]
pub async fn disconnect() -> Result<SessionInfo, Error> {
    tauri::async_runtime::spawn_blocking(bridge::disconnect)
        .await
        .map_err(|error| Error::Join(error.to_string()))?
}

/// Executes one client command (the same syntax the `pm3` client accepts) and
/// returns its output. Opens an offline session on first use.
#[tauri::command]
pub async fn execute(command: String) -> Result<CommandOutput, Error> {
    let home = home_dir().cloned();
    tauri::async_runtime::spawn_blocking(move || bridge::execute(&command, home.as_deref()))
        .await
        .map_err(|error| Error::Join(error.to_string()))?
}

/// Executes one client command and forwards its output as it is produced
/// through the `proxmark3://output` event.  The final, complete output is also
/// returned.  Commands are serialised, so only one stream runs at a time.
#[tauri::command]
pub async fn execute_stream<R: Runtime>(
    app: AppHandle<R>,
    command: String,
) -> Result<CommandOutput, Error> {
    let home = home_dir().cloned();
    tauri::async_runtime::spawn_blocking(move || {
        let emitter = app.clone();
        bridge::execute_stream(&command, home.as_deref(), move |chunk| {
            let _ = emitter.emit("proxmark3://output", OutputChunk { chunk: chunk.to_string() });
        })
    })
    .await
    .map_err(|error| Error::Join(error.to_string()))?
}

/// Current session state.
#[tauri::command]
pub fn status() -> SessionInfo {
    bridge::status()
}

/// Client version string compiled into the library.
#[tauri::command]
pub fn version() -> String {
    bridge::version()
}

/// Writes bytes to the client stdin while a command is running.
#[tauri::command]
pub fn input(data: String) -> Result<(), Error> {
    bridge::send_input(&data)
}
