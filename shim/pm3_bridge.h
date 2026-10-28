//-----------------------------------------------------------------------------
// C bridge between the Rust plugin and the Proxmark3 client library (libpm3).
//
// The upstream client exposes a small C API in client/include/pm3.h
// (pm3_open / pm3_console / pm3_grabbed_output_get).  That API is convenient
// but calls exit() when an explicitly requested port cannot be opened, which is
// unacceptable inside a Tauri application.  This bridge re-implements the few
// needed entry points on top of the client internals so every failure is
// reported as a status code instead of terminating the process.
//
// This file is compiled by the plugin build script (see build.rs) and links
// against the static proxmark3 client library built from the submodule.
//-----------------------------------------------------------------------------
#ifndef PM3X_BRIDGE_H
#define PM3X_BRIDGE_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

// Status codes returned by the bridge.  Positive values are the proxmark3
// client status codes (PM3_SUCCESS == 0, PM3_* error codes).
#define PM3X_OK 0
#define PM3X_E_INVALID_ARG (-1)
#define PM3X_E_NOT_OPEN (-2)
#define PM3X_E_DEVICE (-3)
#define PM3X_E_INTERNAL (-4)

// Overrides $HOME before the client library initialises its user directory.
// Must be called before pm3x_open().  Passing NULL is a no-op.
void pm3x_set_home(const char *dir);

// Opens a connection.
//   port == NULL or ""  -> connect to the first Proxmark3 found; if none is
//                          present the client runs in offline mode, which is
//                          still useful (help, data conversion commands, ...).
//   port == "tcp:host:port" / "udp:..." / "bt:..." / "/dev/ttyACM0" / "COM3"
//                       -> connect to that transport, fail if unavailable.
// Returns PM3X_OK on success, a negative PM3X_E_* code otherwise.
int pm3x_open(const char *port);

// Closes the connection (no-op when not open).
int pm3x_close(void);

// 1 when a session is open (offline sessions included).
int pm3x_is_open(void);

// 1 when the session was opened without a device (offline mode).
int pm3x_is_offline(void);

// Name of the transport used by the current session, "" when offline.
const char *pm3x_port(void);

// Version string baked into the client library at build time.
const char *pm3x_version(void);

// Executes one client command (same syntax as the proxmark3 command line) and
// captures everything the client printed while it ran.
//   *out is a malloc'ed, NUL terminated UTF-8 string, release with pm3x_free().
// Returns the client command status (>= 0) or a negative PM3X_E_* code.
int pm3x_execute(const char *command, char **out);

// Same as pm3x_execute(), but the output is forwarded to `callback` while the
// command runs, in chunks, so the caller can stream it.  The callback runs on
// a reader thread owned by the bridge; it is joined before this function
// returns.  `userdata` is passed through untouched.
// Returns the client command status (>= 0) or a negative PM3X_E_* code.
typedef void (*pm3x_chunk_callback)(void *userdata, const char *data, size_t len);
int pm3x_execute_stream(const char *command, pm3x_chunk_callback callback, void *userdata);

// Releases a string returned through pm3x_execute().
void pm3x_free(char *ptr);

// Last human readable error message produced by the bridge ("" when none).
const char *pm3x_last_error(void);

#ifdef __cplusplus
}
#endif

#endif // PM3X_BRIDGE_H
