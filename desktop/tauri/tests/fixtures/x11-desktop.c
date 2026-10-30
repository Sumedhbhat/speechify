/* External X11 boundary responses in an isolated clipboard-worker process. */
#define _GNU_SOURCE
#include <X11/Xlib.h>
#include <dlfcn.h>
#include <stdbool.h>
#include <stdatomic.h>
#include <stdlib.h>
#include <string.h>

static void *self_handle, *fixes_handle;
static bool grabbed, startup_event, claimed;
static int pending_event, event_base;
typedef void *(*lookup_fn)(void *, const char *);
typedef void *(*open_library_fn)(const char *, int);
static _Atomic(lookup_fn) lookup;
static _Atomic(open_library_fn) open_library;
static bool scenario(const char *name) {
    const char *value = getenv("SPEECHIFY_TEST_X11_SCENARIO");
    return value && !strcmp(value, name);
}
static void initialize(void) {
    if (!lookup) lookup = dlvsym(RTLD_NEXT, "dlsym", "GLIBC_2.2.5");
    if (!open_library) open_library = lookup(RTLD_NEXT, "dlopen");
}
void *dlopen(const char *name, int flags) {
    initialize();
    const char *path = getenv("SPEECHIFY_TEST_X11_SHIM");
    if (path && name && (!strcmp(name, "libX11.so.6") || !strcmp(name, "libXfixes.so.3"))) {
        if (!strcmp(name, "libXfixes.so.3")) fixes_handle = open_library(name, flags);
        self_handle = open_library(path, flags);
        return self_handle;
    }
    return open_library(name, flags);
}
void *dlsym(void *handle, const char *name) {
    initialize();
    if (self_handle && handle == self_handle && scenario("unavailable-event-api") && !strcmp(name, "XChangeProperty"))
        return lookup(handle, "speechify_fixture_missing_x11_entry");
    return lookup(handle, name);
}
#define REAL(name, type) initialize(); type original = (type)lookup(RTLD_NEXT, #name)
/* C typedefs keep the exact native signatures visible to the compiler. */
typedef Window (*owner_fn)(Display *, Atom);
Window XGetSelectionOwner(Display *display, Atom atom) {
    REAL(XGetSelectionOwner, owner_fn);
    if (grabbed && scenario("changed-before-grab")) return None;
    if (grabbed && claimed && scenario("owner-disappeared-before-restore")) return None;
    return original(display, atom);
}
typedef int (*set_owner_fn)(Display *, Atom, Window, Time);
int XSetSelectionOwner(Display *display, Atom atom, Window owner, Time time) {
    REAL(XSetSelectionOwner, set_owner_fn);
    if (scenario("ownership-refused")) return 0;
    claimed = true;
    return original(display, atom, owner, time);
}
typedef int (*display_fn)(Display *);
int XGrabServer(Display *display) {
    REAL(XGrabServer, display_fn);
    int result = original(display);
    grabbed = true;
    if (scenario("queued-copy-notification")) pending_event = event_base;
    return result;
}
int XUngrabServer(Display *display) {
    REAL(XUngrabServer, display_fn);
    grabbed = false;
    return original(display);
}
typedef int (*sync_fn)(Display *, Bool);
int XSync(Display *display, Bool discard) {
    REAL(XSync, sync_fn);
    int result = original(display, discard);
    if (!startup_event && scenario("startup-event")) {
        startup_event = true;
        pending_event = Expose;
    }
    return result;
}
int XPending(Display *display) {
    REAL(XPending, display_fn);
    return pending_event ? 1 : original(display);
}
typedef int (*next_fn)(Display *, XEvent *);
int XNextEvent(Display *display, XEvent *event) {
    REAL(XNextEvent, next_fn);
    if (!pending_event) return original(display, event);
    memset(event, 0, sizeof(*event));
    event->type = pending_event;
    pending_event = 0;
    return 0;
}
typedef Bool (*query_fn)(Display *, int *, int *);
Bool XFixesQueryExtension(Display *display, int *event, int *error) {
    query_fn original = (query_fn)lookup(fixes_handle, "XFixesQueryExtension");
    Bool result = original(display, event, error);
    event_base = *event;
    return scenario("no-selection-notifications") ? False : result;
}
typedef void (*select_fn)(Display *, Window, Atom, unsigned long);
void XFixesSelectSelectionInput(Display *display, Window window, Atom selection, unsigned long mask) {
    select_fn original = (select_fn)lookup(fixes_handle, "XFixesSelectSelectionInput");
    original(display, window, selection, mask);
}

/* Delay one completed IO operation so cancellation can overlap a late result. */
#include <fcntl.h>
#include <stdio.h>
#include <sys/stat.h>
#include <unistd.h>
static atomic_bool delayed_io;
static void io_gate(void) {
    const char *gate = getenv("SPEECHIFY_TEST_IO_GATE");
    const char *release = getenv("SPEECHIFY_TEST_IO_RELEASE");
    if (!gate || !release || atomic_exchange(&delayed_io, true)) return;
    int file = open(gate, O_WRONLY | O_CREAT | O_TRUNC, 0600);
    if (file >= 0) close(file);
    for (unsigned attempt = 0; attempt < 10000 && access(release, F_OK) != 0; attempt++) usleep(1000);
}
typedef ssize_t (*read_fn)(int, void *, size_t);
ssize_t read(int fd, void *buffer, size_t length) {
    REAL(read, read_fn);
    ssize_t bytes = original(fd, buffer, length);
    if (fd > 2 && bytes > 0 && scenario("delay-transcript-read") && memmem(buffer, (size_t)bytes, "\"text\"", 6)) {
        struct stat state;
        if (fstat(fd, &state) == 0 && S_ISFIFO(state.st_mode)) io_gate();
    }
    return bytes;
}
typedef ssize_t (*write_fn)(int, const void *, size_t);
ssize_t write(int fd, const void *buffer, size_t length) {
    REAL(write, write_fn);
    if (fd > 2 && scenario("delay-inference-log") && memmem(buffer, length, "inference", 9)) {
        char link[64], path[4096];
        snprintf(link, sizeof(link), "/proc/self/fd/%d", fd);
        ssize_t size = readlink(link, path, sizeof(path) - 1);
        if (size > 0) {
            path[size] = 0;
            if (strstr(path, "/logs/timings.jsonl")) io_gate();
        }
    }
    return original(fd, buffer, length);
}
