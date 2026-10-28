//-----------------------------------------------------------------------------
// iOS has no system(3): the SDK declares it as unavailable, which turns any
// call into a compile error.  The Proxmark3 client uses it for the `!` shell
// escape (client/src/cmdparser.c) and Lua's os.execute() would use it too.
//
// This header is force-included (-include) on iOS.  It pulls in <stdlib.h>
// first, then shadows the name with a function-like macro, so the client
// compiles unchanged and the calls resolve to a stub that always fails, which
// is exactly the behaviour iOS can offer.
//-----------------------------------------------------------------------------
#ifndef PM3X_IOS_SYSTEM_H
#define PM3X_IOS_SYSTEM_H

#include <stdlib.h>

#ifdef __cplusplus
extern "C" {
#endif

static inline int pm3x_ios_no_process_spawn(const char *command) {
    (void)command;
    return -1;
}

#ifdef __cplusplus
}
#endif

#define system(command) pm3x_ios_no_process_spawn(command)

#endif // PM3X_IOS_SYSTEM_H
