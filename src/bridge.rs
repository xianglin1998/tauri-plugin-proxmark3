//! Thin, safe wrapper around the C bridge (`shim/pm3_bridge.c`).
//!
//! The proxmark3 client is a process-wide singleton: its command parser, the
//! serial port and the print buffers are global state. Every entry point is
//! therefore serialised through one mutex, and callers run them on a blocking
//! thread so a slow (or stuck) device never blocks the Tauri runtime.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use crate::ansi;
use crate::error::Error;
use crate::models::{CommandOutput, SessionInfo};

// Status codes, must match shim/pm3_bridge.h
const PM3X_E_NOT_OPEN: c_int = -2;
const PM3X_E_DEVICE: c_int = -3;

/// C callback used to forward command output chunks while a command runs.
type ChunkCallback = extern "C" fn(*mut c_void, *const c_char, usize);

extern "C" {
    fn pm3x_set_home(dir: *const c_char);
    fn pm3x_open(port: *const c_char) -> c_int;
    fn pm3x_close() -> c_int;
    fn pm3x_is_open() -> c_int;
    fn pm3x_is_offline() -> c_int;
    fn pm3x_port() -> *const c_char;
    fn pm3x_version() -> *const c_char;
    fn pm3x_execute(command: *const c_char, out: *mut *mut c_char) -> c_int;
    fn pm3x_execute_stream(
        command: *const c_char,
        callback: Option<ChunkCallback>,
        userdata: *mut c_void,
    ) -> c_int;
    fn pm3x_send_input(data: *const c_char) -> c_int;
    fn pm3x_free(ptr: *mut c_char);
    fn pm3x_last_error() -> *const c_char;
}

static PM3_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    // A panic while a command was running leaves the C side in an unknown
    // state, but poisoning the mutex forever would break the whole plugin.
    PM3_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn cstring(value: &str) -> Result<CString, Error> {
    CString::new(value).map_err(|_| Error::NulByte)
}

unsafe fn cstr_to_string(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    CStr::from_ptr(ptr).to_string_lossy().into_owned()
}

/// Last error reported by the bridge, if any.
pub fn last_error() -> String {
    unsafe { cstr_to_string(pm3x_last_error()) }
}

/// Overrides the `HOME` directory used by the client for `~/.proxmark3`.
pub fn set_home(dir: &Path) {
    if let Ok(value) = CString::new(dir.to_string_lossy().as_bytes()) {
        unsafe { pm3x_set_home(value.as_ptr()) };
    }
}

/// Current session state.
pub fn status() -> SessionInfo {
    let _guard = lock();
    let open = unsafe { pm3x_is_open() } != 0;
    let offline = unsafe { pm3x_is_offline() } != 0;
    let port = if open && !offline {
        unsafe { cstr_to_string(pm3x_port()) }
    } else {
        String::new()
    };
    SessionInfo {
        open,
        offline,
        port,
        version: unsafe { cstr_to_string(pm3x_version()) },
    }
}

/// Client version baked into the library.
pub fn version() -> String {
    unsafe { cstr_to_string(pm3x_version()) }
}

/// Writes bytes to the client stdin.  This does not take the command mutex:
/// the whole point is to feed a command that is currently holding it.
pub fn send_input(data: &str) -> Result<(), Error> {
    let data_c = cstring(data)?;
    let status = unsafe { pm3x_send_input(data_c.as_ptr()) };
    if status != 0 {
        let detail = last_error();
        return Err(Error::client(if detail.is_empty() {
            "cannot write to the client stdin".to_string()
        } else {
            detail
        }));
    }
    Ok(())
}

/// Opens a session.
///
/// `port` may be:
///   * `None` – offline session, or the first device found by the client;
///   * `"/dev/ttyACM0"`, `"COM3"` – a local serial port;
///   * `"tcp:host:port"`, `"udp:host:port"` – a network bridge;
///   * `"bt:..."` – Bluetooth (Linux/BlueZ builds only).
pub fn connect(port: Option<&str>, home: Option<&Path>) -> Result<SessionInfo, Error> {
    if let Some(home) = home {
        set_home(home);
    }
    let port_c = match port {
        Some(port) if !port.is_empty() => Some(cstring(port)?),
        _ => None,
    };

    let _guard = lock();
    let status = unsafe {
        pm3x_open(match &port_c {
            Some(value) => value.as_ptr(),
            None => std::ptr::null(),
        })
    };

    if status == PM3X_E_DEVICE {
        return Err(Error::client(last_error()));
    }
    if status != 0 {
        return Err(Error::client(format!(
            "cannot open the Proxmark3 session (status {status}): {}",
            last_error()
        )));
    }

    Ok(status_unlocked())
}

/// Closes the session, if any.
pub fn disconnect() -> Result<SessionInfo, Error> {
    let _guard = lock();
    let status = unsafe { pm3x_close() };
    if status != 0 {
        return Err(Error::client(last_error()));
    }
    Ok(status_unlocked())
}

/// Runs one client command and returns everything it printed.
pub fn execute(command: &str, home: Option<&Path>) -> Result<CommandOutput, Error> {
    let started = Instant::now();

    // Opening an offline session is cheap and lets `execute()` work without an
    // explicit `connect()` call.
    if unsafe { pm3x_is_open() } == 0 {
        connect(None, home)?;
    }

    let command_c = cstring(command)?;
    let mut raw: *mut c_char = std::ptr::null_mut();

    let _guard = lock();
    let status = unsafe { pm3x_execute(command_c.as_ptr(), &mut raw) };

    let output = if raw.is_null() {
        String::new()
    } else {
        let text = unsafe { cstr_to_string(raw) };
        unsafe { pm3x_free(raw) };
        text
    };

    if status == PM3X_E_NOT_OPEN {
        return Err(Error::NotOpen);
    }
    if status < 0 {
        let detail = last_error();
        if detail.is_empty() {
            return Err(Error::client(format!(
                "command failed with status {status}"
            )));
        }
        return Err(Error::client(detail));
    }

    Ok(CommandOutput {
        status,
        output: ansi::sanitize(&output),
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

fn status_unlocked() -> SessionInfo {
    let open = unsafe { pm3x_is_open() } != 0;
    let offline = unsafe { pm3x_is_offline() } != 0;
    let port = if open && !offline {
        unsafe { cstr_to_string(pm3x_port()) }
    } else {
        String::new()
    };
    SessionInfo {
        open,
        offline,
        port,
        version: unsafe { cstr_to_string(pm3x_version()) },
    }
}

/// Runs one client command while forwarding its output to `on_chunk` as it is
/// State shared with the C reader thread for the duration of one command.
struct StreamState<F> {
    on_chunk: F,
    pending: Vec<u8>,
    collected: Vec<u8>,
}

/// Trampoline invoked by the C reader thread for every chunk of output.
///
/// Safety: `userdata` always points to the `StreamState` owned by
/// [`execute_stream`], which outlives the call because the C side joins the
/// reader thread before returning.
extern "C" fn stream_trampoline<F>(userdata: *mut c_void, data: *const c_char, len: usize)
where
    F: FnMut(&str),
{
    if userdata.is_null() || data.is_null() || len == 0 {
        return;
    }
    // SAFETY: see the function contract above.
    let state = unsafe { &mut *(userdata as *mut StreamState<F>) };
    // SAFETY: the C side guarantees `data` is valid for `len` bytes.
    let bytes = unsafe { std::slice::from_raw_parts(data as *const u8, len) };

    state.collected.extend_from_slice(bytes);
    state.pending.extend_from_slice(bytes);

    // Only forward complete UTF-8 sequences; a multi-byte character may
    // straddle two reads.
    let valid = match std::str::from_utf8(&state.pending) {
        Ok(_) => state.pending.len(),
        Err(error) => error.valid_up_to(),
    };
    if valid > 0 {
        let text = String::from_utf8_lossy(&state.pending[..valid]).into_owned();
        (state.on_chunk)(&text);
        state.pending.drain(..valid);
    }
}

/// Runs one client command while forwarding its output to `on_chunk` as it is
/// produced, and returns the complete output when the command finishes.
///
/// The command runs on the calling thread (use a blocking thread); `on_chunk`
/// is invoked from the bridge's reader thread while the command is running.
pub fn execute_stream<F>(
    command: &str,
    home: Option<&Path>,
    on_chunk: F,
) -> Result<CommandOutput, Error>
where
    F: FnMut(&str) + Send,
{
    let started = Instant::now();

    // Opening an offline session is cheap and lets `execute_stream()` work
    // without an explicit `connect()` call.
    if unsafe { pm3x_is_open() } == 0 {
        connect(None, home)?;
    }

    let command_c = cstring(command)?;
    let mut state = StreamState {
        on_chunk,
        pending: Vec::new(),
        collected: Vec::new(),
    };

    let status = {
        let _guard = lock();
        unsafe {
            pm3x_execute_stream(
                command_c.as_ptr(),
                Some(stream_trampoline::<F>),
                &mut state as *mut StreamState<F> as *mut c_void,
            )
        }
    };

    if !state.pending.is_empty() {
        let text = String::from_utf8_lossy(&state.pending).into_owned();
        (state.on_chunk)(&text);
    }

    if status == PM3X_E_NOT_OPEN {
        return Err(Error::NotOpen);
    }
    if status < 0 {
        let detail = last_error();
        return Err(Error::client(if detail.is_empty() {
            format!("command failed with status {status}")
        } else {
            detail
        }));
    }

    let output = String::from_utf8_lossy(&state.collected).into_owned();
    Ok(CommandOutput {
        status,
        output: ansi::sanitize(&output),
        duration_ms: started.elapsed().as_millis() as u64,
    })
}
