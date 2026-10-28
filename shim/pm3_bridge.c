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
#include <io.h>
#endif

#if defined(__APPLE__)
#include <TargetConditionals.h>
#endif

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
static char g_last_error[512] = {0};

#if defined(__APPLE__) && TARGET_OS_IPHONE
// iOS forbids spawning processes.  Both the client's `!` shell escape and
// Lua's os.execute() are routed here by the build (see cmake/CMakeLists.txt).
int pm3x_platform_system(const char *command) {
    (void)command;
    return -1;
}
#endif

typedef struct {
    int saved_fd;
    FILE *file;
} pm3x_capture_t;

static bool pm3x_capture_begin(pm3x_capture_t *capture) {
    capture->saved_fd = -1;
    capture->file = tmpfile();
    if (capture->file == NULL) {
        return false;
    }

    fflush(stdout);
#ifdef _WIN32
    capture->saved_fd = _dup(_fileno(stdout));
    if (capture->saved_fd < 0 || _dup2(_fileno(capture->file), _fileno(stdout)) != 0) {
        if (capture->saved_fd >= 0) {
            _close(capture->saved_fd);
        }
        fclose(capture->file);
        return false;
    }
#else
    capture->saved_fd = dup(fileno(stdout));
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

void pm3x_free(char *ptr) {
    free(ptr);
}

const char *pm3x_last_error(void) {
    return g_last_error;
}
