//! Minimal Tauri application that depends on `tauri-plugin-proxmark3`.
//!
//! It is intentionally tiny: the point is to prove that the plugin can be
//! linked into a real Tauri app on every platform (Linux, macOS, Windows,
//! Android, iOS) and that its runtime API works on desktop.

/// Returns the Proxmark3 client version compiled into the plugin.
#[tauri::command]
fn pm3_version() -> String {
    tauri_plugin_proxmark3::client::version()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_proxmark3::init())
        .invoke_handler(tauri::generate_handler![pm3_version])
        .setup(|_app| {
            let version = tauri_plugin_proxmark3::client::version();
            println!("[minimal-app] proxmark3 client version: {version}");

            #[cfg(desktop)]
            {
                // Exercise the plugin end to end (offline mode, no hardware).
                match tauri_plugin_proxmark3::client::execute("hw version") {
                    Ok(output) => println!(
                        "[minimal-app] `hw version` returned {} bytes (status {})",
                        output.output.len(),
                        output.status
                    ),
                    Err(error) => eprintln!("[minimal-app] `hw version` failed: {error}"),
                }
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the minimal Tauri application");
}
