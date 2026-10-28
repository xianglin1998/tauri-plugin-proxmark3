//! Proxmark3 client plugin for Tauri v2.
//!
//! The plugin links the upstream Proxmark3 client (the `pm3` command line
//! client) as a static library and exposes it as Tauri commands:
//!
//! ```ignore
//! use tauri_plugin_proxmark3::Proxmark3Ext;
//!
//! tauri::Builder::default()
//!     .plugin(tauri_plugin_proxmark3::init())
//!     .run(tauri::generate_context!())
//!     .expect("error while running tauri application");
//! ```
//!
//! Then, from JavaScript:
//!
//! ```ignore
//! import { connect, execute } from 'tauri-plugin-proxmark3';
//!
//! await connect();                       // offline mode, or auto-detect a device
//! await connect('tcp:192.168.1.10:18888'); // or a specific transport
//! const { output } = await execute('hw version');
//! ```
//!
//! On Android and iOS the client is cross compiled with the platform toolchain.
//! Desktop serial ports (`/dev/ttyACM0`, `COM3`, ...), TCP/UDP bridges
//! (`tcp:host:port`, `udp:host:port`) and offline mode work on every platform;
//! native Bluetooth (`bt:...`) is only available on Linux builds that ship
//! BlueZ, because that is a build-time option of the upstream client.

mod ansi;
mod bridge;
mod commands;
mod error;
mod models;

pub use error::Error;
pub use models::{CommandOutput, SessionInfo};

/// Rust API for programs that do not go through the Tauri command layer.
///
/// The proxmark3 client is a process-wide singleton, so every call is
/// serialised internally and may be used from any thread.
pub mod client {
    use crate::bridge;
    use crate::error::Error;
    use crate::models::{CommandOutput, SessionInfo};
    use crate::home_dir;

    /// Opens a session; see [`crate::commands::connect`] for the port syntax.
    pub fn connect(port: Option<&str>) -> Result<SessionInfo, Error> {
        bridge::connect(port, home_dir().map(|dir| dir.as_path()))
    }

    /// Closes the current session.
    pub fn disconnect() -> Result<SessionInfo, Error> {
        bridge::disconnect()
    }

    /// Executes one client command and returns its captured output.
    pub fn execute(command: &str) -> Result<CommandOutput, Error> {
        bridge::execute(command, home_dir().map(|dir| dir.as_path()))
    }

    /// Executes one client command while forwarding its output to `on_chunk`
    /// as it is produced (streaming).
    pub fn execute_stream<F>(command: &str, on_chunk: F) -> Result<CommandOutput, Error>
    where
        F: FnMut(&str) + Send,
    {
        bridge::execute_stream(command, home_dir().map(|dir| dir.as_path()), on_chunk)
    }

    /// Current session state.
    pub fn status() -> SessionInfo {
        bridge::status()
    }

    /// Client version string compiled into the library.
    pub fn version() -> String {
        bridge::version()
    }
}

use std::path::PathBuf;
use std::sync::OnceLock;

use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime};

/// Base directory the Proxmark3 client may use for its `~/.proxmark3` files.
/// Only set when the platform has no usable `HOME` (Windows) or when the app is
/// sandboxed (Android/iOS).
static HOME: OnceLock<PathBuf> = OnceLock::new();

pub(crate) fn home_dir() -> Option<&'static PathBuf> {
    HOME.get()
}

/// Initialises the plugin.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("proxmark3")
        .invoke_handler(tauri::generate_handler![
            commands::connect,
            commands::disconnect,
            commands::execute,
            commands::execute_stream,
            commands::status,
            commands::version,
        ])
        .setup(|app, _api| {
            let sandboxed = cfg!(any(target_os = "android", target_os = "ios"));
            let no_home = std::env::var_os("HOME").is_none();
            if sandboxed || no_home {
                if let Ok(dir) = app.path().app_data_dir() {
                    let _ = std::fs::create_dir_all(dir.join(".proxmark3"));
                    let _ = HOME.set(dir);
                }
            }
            Ok(())
        })
        .build()
}
