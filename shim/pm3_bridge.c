//-----------------------------------------------------------------------------
// C bridge between the Rust plugin and the Proxmark3 client library (libpm3).
// See pm3_bridge.h for the contract of every entry point.
//-----------------------------------------------------------------------------
#include "pm3_bridge.h"

#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifdef _WIN32
#include <fcntl.h>
#include <io.h>
#else
#include <fcntl.h>
#include <unistd.h>
#endif

#include <pthread.h>

#include "comms.h"
#include "commonutil.h"
#include "pm3.h"
#include "preferences.h"
#include "proxmark3.h"
#include "ui.h"
#include "usart_defs.h"
#include "util_posix.h"

static bool g_initialized = false;
static bool g_opened = false;
static bool g_offline = false;
static int g_input_write_fd = -1;
static char g_last_error[512] = {0};

static void pm3x_set_error(const char *fmt, ...);

typedef struct {
    int fd;
    pm3x_chunk_callback callback;
    void *userdata;
} pm3x_reader_t;

static void *pm3x_reader_thread(void *arg) {
    pm3x_reader_t *reader = (pm3x_reader_t *)arg;
    char buffer[4096];

    for (;;) {
#ifdef _WIN32
        int read_len = _read(reader->fd, buffer, (unsigned int)sizeof(buffer));
#else
        ssize_t read_len = read(reader->fd, buffer, sizeof(buffer));
#endif
        if (read_len <= 0) {
            break;
        }
        if (reader->callback != NULL) {
            reader->callback(reader->userdata, buffer, (size_t)read_len);
        }
    }
    return NULL;
}

typedef struct {
    int saved_fd;
    FILE *file;
} pm3x_capture_t;

// Stdout is not guaranteed to be associated with a descriptor in a GUI
// process.  A Windows GUI binary started without a console can leave stdout
// with `_fileno(stdout) == -1`, in which case dup2() has no target to install
// the capture pipe on.  Giving stdout a valid null-device stream first makes
// both the temporary-file and pipe capture paths work.
static void pm3x_ensure_stdout(void) {
#ifdef _WIN32
    if (_fileno(stdout) < 0) {
        (void)freopen("NUL", "w", stdout);
    }
#else
    if (fileno(stdout) < 0) {
        (void)freopen("/dev/null", "w", stdout);
    }
#endif
}

// Point fd 1 at the null device first and return a distinct descriptor that
// can be used to restore the previous state afterwards.
static int pm3x_save_stdout(void) {
    pm3x_ensure_stdout();
#ifdef _WIN32
    int target = _fileno(stdout);
    int saved = _dup(target);
#else
    int target = fileno(stdout);
    int saved = dup(target);
#endif
    if (saved >= 0) {
        return saved;
    }

#ifdef _WIN32
    int fallback = _open("NUL", _O_WRONLY);
#else
    int fallback = open("/dev/null", O_WRONLY);
#endif
    if (fallback < 0) {
        return -1;
    }

    // The null device may have taken the just-closed descriptor itself.  Move
    // it aside so `saved` remains distinct from the descriptor being restored.
    if (fallback == target) {
#ifdef _WIN32
        int moved = _dup(fallback);
        _close(fallback);
#else
        int moved = dup(fallback);
        close(fallback);
#endif
        fallback = moved;
        if (fallback < 0) {
            return -1;
        }
    }

#ifdef _WIN32
    if (_dup2(fallback, target) != 0) {
        _close(fallback);
        return -1;
    }
#else
    if (dup2(fallback, target) < 0) {
        close(fallback);
        return -1;
    }
#endif
    return fallback;
}

static bool pm3x_capture_begin(pm3x_capture_t *capture) {
    capture->saved_fd = -1;
    capture->file = tmpfile();
    if (capture->file == NULL) {
        return false;
    }

    fflush(stdout);
    capture->saved_fd = pm3x_save_stdout();
#ifdef _WIN32
    if (capture->saved_fd < 0 || _dup2(_fileno(capture->file), _fileno(stdout)) != 0) {
        if (capture->saved_fd >= 0) {
            _close(capture->saved_fd);
        }
        fclose(capture->file);
        return false;
    }
#else
    if (capture->saved_fd < 0 || dup2(fileno(capture->file), fileno(stdout)) < 0) {
        if (capture->saved_fd >= 0) {
            close(capture->saved_fd);
        }
        fclose(capture->file);
        return false;
    }
#endif
    return true;
}

static char *pm3x_capture_end(pm3x_capture_t *capture) {
    fflush(stdout);
#ifdef _WIN32
    _dup2(capture->saved_fd, _fileno(stdout));
    _close(capture->saved_fd);
#else
    dup2(capture->saved_fd, fileno(stdout));
    close(capture->saved_fd);
#endif
    fflush(stdout);

    char *buffer = NULL;
    if (fseek(capture->file, 0, SEEK_END) == 0) {
        long size = ftell(capture->file);
        if (size >= 0 && fseek(capture->file, 0, SEEK_SET) == 0) {
            buffer = (char *)malloc((size_t)size + 1);
            if (buffer != NULL) {
                size_t read = fread(buffer, 1, (size_t)size, capture->file);
                buffer[read] = '\0';
            }
        }
    }
    fclose(capture->file);
    return buffer;
}

static void pm3x_ensure_stdin(void) {
    if (g_input_write_fd >= 0) {
        return;
    }

#ifdef _WIN32
    if (_isatty(_fileno(stdin))) {
        return;
    }
    if (_fileno(stdin) < 0) {
        (void)freopen("NUL", "r", stdin);
    }
    int fds[2];
    if (_pipe(fds, 4096, _O_BINARY) != 0) {
        return;
    }
    _dup2(fds[0], _fileno(stdin));
    _close(fds[0]);
    g_input_write_fd = fds[1];
#else
    if (isatty(STDIN_FILENO)) {
        return;
    }
    if (fileno(stdin) < 0) {
        (void)freopen("/dev/null", "r", stdin);
    }
    int fds[2];
    if (pipe(fds) != 0) {
        return;
    }
    dup2(fds[0], fileno(stdin));
    close(fds[0]);
    g_input_write_fd = fds[1];
#endif
}

int pm3x_send_input(const char *data) {
    if (data == NULL) {
        pm3x_set_error("input is NULL");
        return PM3X_E_INVALID_ARG;
    }
    if (g_input_write_fd < 0) {
        pm3x_set_error("the client stdin is not available");
        return PM3X_E_INTERNAL;
    }

    size_t length = strlen(data);
    size_t offset = 0;
    while (offset < length) {
#ifdef _WIN32
        int written = _write(g_input_write_fd, data + offset, (unsigned int)(length - offset));
#else
        ssize_t written = write(g_input_write_fd, data + offset, length - offset);
#endif
        if (written <= 0) {
            pm3x_set_error("cannot write to the client stdin");
            return PM3X_E_INTERNAL;
        }
        offset += (size_t)written;
    }
    return PM3X_OK;
}

static void pm3x_clear_error(void) {
    g_last_error[0] = '\0';
}

static void pm3x_set_error(const char *fmt, ...) {
    va_list args;
    va_start(args, fmt);
    vsnprintf(g_last_error, sizeof(g_last_error), fmt, args);
    va_end(args);
}

void pm3x_set_home(const char *dir) {
    if (dir == NULL || dir[0] == '\0') {
        return;
    }
#ifdef _WIN32
    _putenv_s("HOME", dir);
#else
    setenv("HOME", dir, 1);
#endif
}

// The client's command dispatcher is single threaded: a command may wait for
// the device, so callers must serialise access (the Rust side does that with a
// mutex).
static void pm3x_ensure_init(void) {
    if (g_initialized) {
        return;
    }
    pm3_init();
    preferences_load();
    g_initialized = true;
}

int pm3x_open(const char *port) {
    pm3x_clear_error();
    pm3x_ensure_stdin();

    if (g_opened) {
        return PM3X_OK;
    }

    pm3x_ensure_init();

    const char *requested = (port != NULL && port[0] != '\0') ? port : NULL;

    g_session.pm3_present = false;
    g_session.current_device = NULL;

    if (requested == NULL) {
        // No port requested: do not probe the USB buses, just run offline.
        g_offline = true;
        g_opened = true;
        return PM3X_OK;
    }

    OpenProxmark(&g_session.current_device, requested, false, 20, false, USART_BAUD_RATE);

    if (!g_session.pm3_present) {
        pm3x_set_error("no Proxmark3 found on '%s'", requested);
        g_session.current_device = NULL;
        return PM3X_E_DEVICE;
    }

    int status = TestProxmark(g_session.current_device);
    if (status != PM3_SUCCESS) {
        pm3x_set_error("cannot communicate with the Proxmark3 on '%s' (status %d)", requested, status);
        CloseProxmark(g_session.current_device);
        g_session.pm3_present = false;
        g_session.current_device = NULL;
        return PM3X_E_DEVICE;
    }

    g_offline = false;
    g_opened = true;
    return PM3X_OK;
}

int pm3x_close(void) {
    pm3x_clear_error();

    if (!g_opened) {
        return PM3X_OK;
    }

    if (g_session.pm3_present && g_session.current_device != NULL) {
        clearCommandBuffer();
        SendCommandNG(CMD_QUIT_SESSION, NULL, 0);
        msleep(100); // let the command leave before tearing the link down
        CloseProxmark(g_session.current_device);
        g_session.pm3_present = false;
    }

    g_session.current_device = NULL;
    free_grabber();
    g_opened = false;
    g_offline = false;
    g_initialized = false;
    return PM3X_OK;
}

int pm3x_is_open(void) {
    return g_opened ? 1 : 0;
}

int pm3x_is_offline(void) {
    return (g_opened && g_offline) ? 1 : 0;
}

const char *pm3x_port(void) {
    if (!g_opened || g_offline || g_session.current_device == NULL || g_session.current_device->g_conn == NULL) {
        return "";
    }
    const char *name = g_session.current_device->g_conn->serial_port_name;
    return name != NULL ? name : "";
}

const char *pm3x_version(void) {
    return g_version_information.gitversion;
}

int pm3x_execute(const char *command, char **out) {
    if (out != NULL) {
        *out = NULL;
    }
    pm3x_clear_error();

    if (command == NULL) {
        pm3x_set_error("command is NULL");
        return PM3X_E_INVALID_ARG;
    }
    if (!g_opened) {
        pm3x_set_error("no Proxmark3 session is open");
        return PM3X_E_NOT_OPEN;
    }

    // The client prints command output with PrintAndLogEx(), but some paths
    // (notably every help screen via CmdsHelp()) deliberately bypass its
    // capture buffer.  Redirecting stdout for the duration of the command is
    // the only way to collect everything, including direct printf() calls.
    pm3x_capture_t capture;
    if (!pm3x_capture_begin(&capture)) {
        pm3x_set_error("cannot redirect stdout to capture command output");
        return PM3X_E_INTERNAL;
    }

    int status = pm3_console(g_session.current_device, command, false, false);

    char *captured = pm3x_capture_end(&capture);
    if (captured == NULL) {
        pm3x_set_error("out of memory while capturing command output");
        return PM3X_E_INTERNAL;
    }

    if (out != NULL) {
        *out = captured;
    } else {
        free(captured);
    }

    return status;
}

int pm3x_execute_stream(const char *command, pm3x_chunk_callback callback, void *userdata) {
    pm3x_clear_error();

    if (command == NULL) {
        pm3x_set_error("command is NULL");
        return PM3X_E_INVALID_ARG;
    }
    if (!g_opened) {
        pm3x_set_error("no Proxmark3 session is open");
        return PM3X_E_NOT_OPEN;
    }

    int fds[2];
#ifdef _WIN32
    if (_pipe(fds, 65536, _O_BINARY) != 0) {
#else
    if (pipe(fds) != 0) {
#endif
        pm3x_set_error("cannot create the output pipe");
        return PM3X_E_INTERNAL;
    }

    pm3x_reader_t reader = {fds[0], callback, userdata};
    pthread_t thread;
    bool threaded = pthread_create(&thread, NULL, pm3x_reader_thread, &reader) == 0;
    if (!threaded) {
#ifdef _WIN32
        _close(fds[0]);
        _close(fds[1]);
#else
        close(fds[0]);
        close(fds[1]);
#endif
        pm3x_set_error("cannot start the output reader thread");
        return PM3X_E_INTERNAL;
    }

    fflush(stdout);
    int saved = pm3x_save_stdout();
#ifdef _WIN32
    if (saved < 0 || _dup2(fds[1], _fileno(stdout)) != 0) {
        if (saved >= 0) {
            _close(saved);
        }
        _close(fds[1]);
        pthread_join(thread, NULL);
        _close(fds[0]);
        pm3x_set_error("cannot redirect stdout");
        return PM3X_E_INTERNAL;
    }
#else
    if (saved < 0 || dup2(fds[1], fileno(stdout)) < 0) {
        if (saved >= 0) {
            close(saved);
        }
        close(fds[1]);
        pthread_join(thread, NULL);
        close(fds[0]);
        pm3x_set_error("cannot redirect stdout");
        return PM3X_E_INTERNAL;
    }
#endif

    int status = pm3_console(g_session.current_device, command, false, false);
    fflush(stdout);

#ifdef _WIN32
    _dup2(saved, _fileno(stdout));
    _close(saved);
    _close(fds[1]);
#else
    dup2(saved, fileno(stdout));
    close(saved);
    close(fds[1]);
#endif

    // Closing the write end is what lets the reader thread see EOF.
    pthread_join(thread, NULL);
#ifdef _WIN32
    _close(fds[0]);
#else
    close(fds[0]);
#endif
    fflush(stdout);

    return status;
}

void pm3x_free(char *ptr) {
    free(ptr);
}

const char *pm3x_last_error(void) {
    return g_last_error;
}
