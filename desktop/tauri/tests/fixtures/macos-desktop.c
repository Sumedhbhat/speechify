/* Controlled Quartz/Accessibility responses in a disposable child process.
 * Real pasteboard and CoreFoundation objects remain native. No input is posted
 * to other applications, and no TCC permission database is modified. */
#include <ApplicationServices/ApplicationServices.h>
#include <mach-o/dyld-interposing.h>
#include <stdatomic.h>
#include <stdint.h>
#include <unistd.h>

enum {
    DENY_ACCESSIBILITY = 1, DENY_INPUT = 2, NO_FOCUS = 4,
    BAD_PID = 8, ZERO_PID = 16, NO_EVENT = 32, NULL_FOCUS = 64
};
static _Atomic uint32_t mode;
static _Atomic bool keys[128];
static unsigned event_count;
static CGKeyCode event_keys[16];
static bool event_down[16];
static CGEventFlags event_flags[16];

void fixture_reset(uint32_t value) {
    atomic_store(&mode, value);
    for (unsigned i = 0; i < 128; i++) atomic_store(&keys[i], false);
    event_count = 0;
}
void fixture_key(uint16_t key, bool down) {
    if (key < 128) atomic_store(&keys[key], down);
}
unsigned fixture_event_count(void) { return event_count; }
uint16_t fixture_event_key(unsigned index) { return event_keys[index]; }
bool fixture_event_down(unsigned index) { return event_down[index]; }
uint64_t fixture_event_flags(unsigned index) { return event_flags[index]; }

static bool trusted(void) { return !(atomic_load(&mode) & DENY_ACCESSIBILITY); }
static bool listening(void) { return !(atomic_load(&mode) & DENY_INPUT); }
static bool held(CGEventSourceStateID state, CGKeyCode key) {
    (void)state;
    return key < 128 && atomic_load(&keys[key]);
}
static AXError focused(AXUIElementRef element, CFStringRef attribute, CFTypeRef *value) {
    (void)element;
    (void)attribute;
    *value = NULL;
    if (atomic_load(&mode) & NO_FOCUS) return kAXErrorCannotComplete;
    if (!(atomic_load(&mode) & NULL_FOCUS)) *value = AXUIElementCreateApplication(getpid());
    return kAXErrorSuccess;
}
static AXError process(AXUIElementRef element, pid_t *pid) {
    (void)element;
    if (atomic_load(&mode) & BAD_PID) return kAXErrorCannotComplete;
    *pid = atomic_load(&mode) & ZERO_PID ? 0 : getpid();
    return kAXErrorSuccess;
}
static CGEventRef keyboard(CGEventSourceRef source, CGKeyCode key, bool down) {
    if (atomic_load(&mode) & NO_EVENT) return NULL;
    /* dyld does not interpose a reference from the interposing image itself. */
    return CGEventCreateKeyboardEvent(source, key, down);
}
static void post(CGEventTapLocation tap, CGEventRef event) {
    (void)tap;
    if (event_count < 16) {
        event_keys[event_count] = (CGKeyCode)CGEventGetIntegerValueField(event, kCGKeyboardEventKeycode);
        event_down[event_count] = CGEventGetType(event) == kCGEventKeyDown;
        event_flags[event_count] = CGEventGetFlags(event);
        event_count++;
    }
}
DYLD_INTERPOSE(trusted, AXIsProcessTrusted)
DYLD_INTERPOSE(listening, CGPreflightListenEventAccess)
DYLD_INTERPOSE(held, CGEventSourceKeyState)
DYLD_INTERPOSE(focused, AXUIElementCopyAttributeValue)
DYLD_INTERPOSE(process, AXUIElementGetPid)
DYLD_INTERPOSE(keyboard, CGEventCreateKeyboardEvent)
DYLD_INTERPOSE(post, CGEventPost)
