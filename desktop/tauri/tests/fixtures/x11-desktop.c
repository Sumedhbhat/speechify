/* External X11 boundary responses in an isolated clipboard-worker process. */
#define _GNU_SOURCE
#include <X11/Xlib.h>
#include <dlfcn.h>
#include <stdbool.h>
#include <stdlib.h>
#include <string.h>

static void *self_handle, *fixes_handle;
static bool grabbed, startup_event, claimed;
static int pending_event, event_base;
static void *(*lookup)(void *, const char *);
static void *(*open_library)(const char *, int);
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
