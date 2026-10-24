#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

int close(int fd) {
    int (*real_close)(int) = dlsym(RTLD_NEXT, "close");
    const char *target = getenv("ROMACS_FILEIO_CLOSE_TARGET");
    char descriptor[64];
    char path[PATH_MAX + 1];
    snprintf(descriptor, sizeof descriptor, "/proc/self/fd/%d", fd);
    ssize_t length = readlink(descriptor, path, PATH_MAX);
    int matched = 0;
    if (length >= 0) {
        path[length] = '\0';
        matched = target && strcmp(path, target) == 0;
    }
    int result = real_close(fd);
    if (matched && result == 0) {
        errno = EIO;
        return -1;
    }
    return result;
}
