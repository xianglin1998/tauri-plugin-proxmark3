//! End-to-end check of the native library: no Proxmark3 hardware required.
//!
//! The client supports an offline mode that keeps the whole command parser,
//! the data conversion commands and the help system available without a
//! device.  If these tests pass, the C client was compiled, linked and can be
//! driven from Rust on the current target.

use tauri_plugin_proxmark3::client;

#[test]
fn reports_client_version() {
    let version = client::version();
    assert!(!version.is_empty(), "the client must expose a version");
    assert!(
        version.contains("Iceman") || version.contains('/'),
        "unexpected version string: {version}"
    );
}

#[test]
fn offline_session_executes_commands() {
    let session = client::connect(None).expect("offline session");
    assert!(session.open);
    assert!(session.offline);
    assert!(session.port.is_empty());

    let status = client::status();
    assert!(status.open);

    let output = client::execute("hw version").expect("hw version");
    assert_eq!(output.status, 0);
    assert!(
        output.output.contains("Proxmark3") || output.output.contains("Client"),
        "unexpected output: {}",
        output.output
    );

    let help = client::execute("data -h").expect("data help");
    assert!(
        help.output.contains("help") || help.output.contains("Graph"),
        "help output was not captured: {}",
        help.output
    );

    client::disconnect().expect("disconnect");
    assert!(!client::status().open);
}

#[test]
fn unknown_command_does_not_fail_the_process() {
    client::connect(None).expect("offline session");
    let output = client::execute("this-command-does-not-exist").expect("unknown command");
    assert!(
        !output.output.is_empty(),
        "the client should print its help screen for unknown commands"
    );
}
