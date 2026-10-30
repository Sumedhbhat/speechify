/* Controlled Quartz/Accessibility responses in a disposable child process.
 * Real pasteboard and CoreFoundation objects remain native. No input is posted
 * to other applications, and no TCC permission database is modified. */
#include <ApplicationServices/ApplicationServices.h>
#include <stdatomic.h>
#include <stdint.h>
#include <unistd.h>

/* dyld's public __interpose ABI; newer SDKs omit dyld-interposing.h. */
#define DYLD_INTERPOSE(replacement, original) \
    __attribute__((used)) static const struct { const void *replace; const void *with; } \
    interpose_##original __attribute__((section("__DATA,__interpose"))) = \
        { (const void *)&replacement, (const void *)&original };

enum {
    DENY_ACCESSIBILITY = 1, DENY_INPUT = 2, NO_FOCUS = 4,
    BAD_PID = 8, ZERO_PID = 16, NO_EVENT = 32, NULL_FOCUS = 64
};
static _Atomic uint32_t mode;
static _Atomic bool keys[128];
static unsigned event_count;
static unsigned focus_reads;
static CGKeyCode event_keys[16];
static bool event_down[16];
static CGEventFlags event_flags[16];

void fixture_reset(uint32_t value) {
    atomic_store(&mode, value);
    for (unsigned i = 0; i < 128; i++) atomic_store(&keys[i], false);
    event_count = 0;
    focus_reads = 0;
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
    *pid = atomic_load(&mode) & ZERO_PID ? 0 : getpid() + ((atomic_load(&mode) & 128) && focus_reads++ > 0);
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
        event_down[event_count] = CGEventGetType(event) == kCGEventKeyDown
            || (CGEventGetType(event) == kCGEventFlagsChanged
                && (CGEventGetFlags(event) & kCGEventFlagMaskCommand) != 0);
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


/* Isolated pasteboard responses for unavailable promises, malformed types,
 * exhausted capacity, refused writes and concurrent-copy detection. */
#import <AppKit/AppKit.h>
#import <objc/runtime.h>
static uint32_t clipboard_mode;
static NSUInteger clipboard_reads, clipboard_generation, clipboard_writes, last_write_size;
static id fixture_pasteboard;
static id (*real_pasteboard)(id, SEL);
static BOOL (*real_set_data)(id, SEL, id, id);

@interface FixtureType : NSObject
- (const char *)UTF8String;
@end
@implementation FixtureType
- (const char *)UTF8String { return NULL; }
@end
@interface FixtureItem : NSObject
- (NSArray *)types;
- (NSData *)dataForType:(id)kind;
@end
@implementation FixtureItem
- (NSArray *)types {
    return clipboard_mode & 4 ? @[[[[FixtureType alloc] init] autorelease]] : @[@"public.utf8-plain-text"];
}
- (NSData *)dataForType:(id)kind {
    (void)kind;
    if (clipboard_mode & 1) return nil;
    if (clipboard_mode & 2) return [NSMutableData dataWithLength:33 * 1024 * 1024];
    if (clipboard_mode & 64) return [NSData data];
    return [@"previous data" dataUsingEncoding:NSUTF8StringEncoding];
}
@end
@interface FixturePasteboard : NSObject
- (NSUInteger)changeCount;
- (NSArray *)pasteboardItems;
- (NSUInteger)clearContents;
- (BOOL)writeObjects:(NSArray *)items;
@end
@implementation FixturePasteboard
- (NSUInteger)changeCount { return clipboard_generation + ((clipboard_mode & 32) ? ++clipboard_reads : 0); }
- (NSArray *)pasteboardItems { return @[[[[FixtureItem alloc] init] autorelease]]; }
- (NSUInteger)clearContents { return ++clipboard_generation; }
- (BOOL)writeObjects:(NSArray *)items {
    clipboard_writes++;
    last_write_size = 0;
    for (NSPasteboardItem *item in items) {
        for (NSString *kind in [item types]) last_write_size += [[item dataForType:kind] length];
    }
    return !(clipboard_mode & 8);
}
@end
static id pasteboard(id object, SEL selector) {
    if (clipboard_mode & 128) {
        if (!fixture_pasteboard) fixture_pasteboard = [[FixturePasteboard alloc] init];
        return fixture_pasteboard;
    }
    return real_pasteboard(object, selector);
}
static BOOL set_data(id object, SEL selector, id data, id kind) {
    return clipboard_mode & 16 ? NO : real_set_data(object, selector, data, kind);
}
__attribute__((constructor)) static void install_pasteboard_responses(void) {
    Method general = class_getClassMethod([NSPasteboard class], @selector(generalPasteboard));
    real_pasteboard = (id (*)(id, SEL))method_setImplementation(general, (IMP)pasteboard);
    Method setter = class_getInstanceMethod([NSPasteboardItem class], @selector(setData:forType:));
    real_set_data = (BOOL (*)(id, SEL, id, id))method_setImplementation(setter, (IMP)set_data);
}
void fixture_clipboard_mode(uint32_t value) {
    clipboard_mode = value;
    clipboard_reads = clipboard_generation = clipboard_writes = last_write_size = 0;
}
NSUInteger fixture_clipboard_writes(void) { return clipboard_writes; }
NSUInteger fixture_clipboard_bytes(void) { return last_write_size; }
