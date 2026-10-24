#include <errno.h>
#include <stdlib.h>
#include <string.h>
int fsync(int fd) {
    (void) fd;
    static int calls;
    const char *mode = getenv("ROMACS_FILEIO_FSYNC_FAULT");
    if (mode && strcmp(mode, "eintr") == 0 && calls++ == 0) {
        errno = EINTR;
    } else if (mode && strcmp(mode, "einval") == 0) {
        errno = EINVAL;
    } else {
        errno = EIO;
    }
    return -1;
}
