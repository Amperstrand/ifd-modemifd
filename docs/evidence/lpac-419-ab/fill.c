#define _GNU_SOURCE
#include <dlfcn.h>
#include <stddef.h>
#include <string.h>
static void *(*real_malloc)(size_t);
void *malloc(size_t n) {
    if (!real_malloc) real_malloc = dlsym(RTLD_NEXT, "malloc");
    void *p = real_malloc(n);
    if (p) memset(p, 0x11, n);
    return p;
}
