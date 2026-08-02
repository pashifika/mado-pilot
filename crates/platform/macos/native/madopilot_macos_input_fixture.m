/*
 * MadoPilot macOS input fixture.
 *
 * Objective-C with Automatic Reference Counting. Compiled into an archive the
 * production Adapter does not link, so nothing here can reach a released
 * artifact.
 *
 * # Why AppKit is not imported
 *
 * For the reason the production shim does not import ScreenCaptureKit: an import
 * creates a load command, and this repository's linkage rule is that the Adapter
 * package declares exactly the frameworks it needs at load. The fixture opens
 * AppKit from its absolute system location and sends the handful of selectors it
 * needs, declared below without a framework header.
 *
 * # What it deliberately does not do
 *
 * It never retains, prints, or forwards the characters of an observed event. It
 * counts UTF-16 units and reports the count. Its window content is one fixed
 * colour, so a captured frame of it contains nothing from the user's desktop.
 */

#import <CoreGraphics/CoreGraphics.h>
#import <Foundation/Foundation.h>

#include <dlfcn.h>

#include "madopilot_macos_input_fixture.h"

#if !__has_feature(objc_arc)
#error "the MadoPilot macOS input fixture requires Automatic Reference Counting"
#endif

#define MP_FIXTURE_BEGIN @try {
#define MP_FIXTURE_END                                                                             \
    }                                                                                              \
    @catch (NSException * exception) {                                                             \
        (void)exception;                                                                           \
        return MP_FIXTURE_NATIVE_EXCEPTION;                                                        \
    }                                                                                              \
    @catch (...) {                                                                                 \
        return MP_FIXTURE_NATIVE_EXCEPTION;                                                        \
    }

/* NSApplicationActivationPolicyRegular. */
static const NSInteger MPFixtureActivationRegular = 0;
/* NSWindowStyleMaskTitled | Closable | Miniaturizable. */
static const NSUInteger MPFixtureWindowStyle = 1 | 2 | 4;
/* NSBackingStoreBuffered. */
static const NSUInteger MPFixtureBackingBuffered = 2;
/* NSEventMaskAny. */
static const unsigned long long MPFixtureEventMaskAny = ~0ull;

/* AppKit event types this fixture classifies. */
static const NSUInteger MPFixtureLeftMouseDown = 1;
static const NSUInteger MPFixtureLeftMouseUp = 2;
static const NSUInteger MPFixtureRightMouseDown = 3;
static const NSUInteger MPFixtureRightMouseUp = 4;
static const NSUInteger MPFixtureMouseMoved = 5;
static const NSUInteger MPFixtureLeftMouseDragged = 6;
static const NSUInteger MPFixtureRightMouseDragged = 7;
static const NSUInteger MPFixtureKeyDown = 10;
static const NSUInteger MPFixtureKeyUp = 11;
static const NSUInteger MPFixtureFlagsChanged = 12;
static const NSUInteger MPFixtureScrollWheel = 22;
static const NSUInteger MPFixtureOtherMouseDown = 25;
static const NSUInteger MPFixtureOtherMouseUp = 26;
static const NSUInteger MPFixtureOtherMouseDragged = 27;

@protocol MPFixtureApplicationClass <NSObject>
+ (id)sharedApplication;
@end

@protocol MPFixtureApplication <NSObject>
- (BOOL)setActivationPolicy:(NSInteger)policy;
- (void)activateIgnoringOtherApps:(BOOL)ignore;
- (void)run;
@end

@protocol MPFixtureColorClass <NSObject>
+ (id)colorWithSRGBRed:(CGFloat)red green:(CGFloat)green blue:(CGFloat)blue alpha:(CGFloat)alpha;
@end

@protocol MPFixtureWindow <NSObject>
- (instancetype)initWithContentRect:(CGRect)contentRect
                          styleMask:(NSUInteger)style
                            backing:(NSUInteger)backing
                              defer:(BOOL)defer;
- (void)setTitle:(NSString *)title;
- (void)setBackgroundColor:(id)color;
- (void)setReleasedWhenClosed:(BOOL)released;
- (void)center;
- (void)makeKeyAndOrderFront:(id)sender;
- (NSInteger)windowNumber;
@end

@protocol MPFixtureEvent <NSObject>
@property(readonly) NSUInteger type;
@property(readonly, copy) NSString *characters;
@end

@protocol MPFixtureEventClass <NSObject>
+ (id)addLocalMonitorForEventsMatchingMask:(unsigned long long)mask
                                   handler:(id (^)(id event))handler;
@end

static bool mp_fixture_load_appkit(void) {
    static void *handle = NULL;
    if (handle != NULL) {
        return true;
    }
    handle = dlopen("/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit",
                    RTLD_LAZY | RTLD_LOCAL);
    if (handle == NULL) {
        handle = dlopen("/System/Library/Frameworks/AppKit.framework/AppKit", RTLD_LAZY | RTLD_LOCAL);
    }
    return handle != NULL;
}

/* Classifies one AppKit event, or reports that this fixture does not record it. */
static bool mp_fixture_classify(NSUInteger type, uint32_t *out_kind) {
    if (type == MPFixtureMouseMoved || type == MPFixtureLeftMouseDragged ||
        type == MPFixtureRightMouseDragged || type == MPFixtureOtherMouseDragged) {
        *out_kind = MP_FIXTURE_EVENT_POINTER_MOVE;
        return true;
    }
    if (type == MPFixtureLeftMouseDown || type == MPFixtureRightMouseDown ||
        type == MPFixtureOtherMouseDown) {
        *out_kind = MP_FIXTURE_EVENT_POINTER_PRESS;
        return true;
    }
    if (type == MPFixtureLeftMouseUp || type == MPFixtureRightMouseUp ||
        type == MPFixtureOtherMouseUp) {
        *out_kind = MP_FIXTURE_EVENT_POINTER_RELEASE;
        return true;
    }
    if (type == MPFixtureScrollWheel) {
        *out_kind = MP_FIXTURE_EVENT_POINTER_SCROLL;
        return true;
    }
    if (type == MPFixtureKeyDown) {
        *out_kind = MP_FIXTURE_EVENT_KEY_DOWN;
        return true;
    }
    if (type == MPFixtureKeyUp) {
        *out_kind = MP_FIXTURE_EVENT_KEY_UP;
        return true;
    }
    if (type == MPFixtureFlagsChanged) {
        *out_kind = MP_FIXTURE_EVENT_FLAGS_CHANGED;
        return true;
    }
    return false;
}

uint32_t mp_fixture_run(const char *title, uint32_t fill, double width, double height,
                        uint32_t launch_context, uint32_t signature_mode,
                        const uint8_t *signing_identifier, size_t signing_identifier_len,
                        void *context,
                        void (*ready)(void *context, uint64_t window_number,
                                      uint32_t launch_context, uint32_t signature_mode,
                                      const uint8_t *signing_identifier,
                                      size_t signing_identifier_len),
                        void (*sink)(void *context, uint32_t kind, uint32_t text_units)) {
    if (title == NULL || ready == NULL || sink == NULL || !(width >= 64.0) ||
        !(height >= 64.0) || !(width <= 4096.0) || !(height <= 4096.0) ||
        (signing_identifier_len > 0 && signing_identifier == NULL)) {
        return MP_FIXTURE_INVALID_ARGUMENT;
    }
    MP_FIXTURE_BEGIN
    if (!mp_fixture_load_appkit()) {
        return MP_FIXTURE_UNSUPPORTED;
    }
    Class application_class = NSClassFromString(@"NSApplication");
    Class window_class = NSClassFromString(@"NSWindow");
    Class color_class = NSClassFromString(@"NSColor");
    Class event_class = NSClassFromString(@"NSEvent");
    if (application_class == Nil || window_class == Nil || color_class == Nil ||
        event_class == Nil) {
        return MP_FIXTURE_UNSUPPORTED;
    }

    NSString *window_title = [NSString stringWithUTF8String:title];
    if (window_title == nil) {
        return MP_FIXTURE_INVALID_ARGUMENT;
    }

    id<MPFixtureApplication> application =
        [(id<MPFixtureApplicationClass>)application_class sharedApplication];
    if (application == nil) {
        return MP_FIXTURE_PLATFORM_FAILURE;
    }
    (void)[application setActivationPolicy:MPFixtureActivationRegular];

    id<MPFixtureWindow> window = [[(id)window_class alloc]
        initWithContentRect:CGRectMake(0.0, 0.0, width, height)
                  styleMask:MPFixtureWindowStyle
                    backing:MPFixtureBackingBuffered
                      defer:NO];
    if (window == nil) {
        return MP_FIXTURE_PLATFORM_FAILURE;
    }
    /* One fixed colour and nothing else, so a frame captured from this window
     * carries no desktop content and is byte-comparable between runs. */
    id color = [(id<MPFixtureColorClass>)color_class
        colorWithSRGBRed:(CGFloat)((fill >> 16) & 0xFFu) / 255.0
                   green:(CGFloat)((fill >> 8) & 0xFFu) / 255.0
                    blue:(CGFloat)(fill & 0xFFu) / 255.0
                   alpha:1.0];
    [window setReleasedWhenClosed:NO];
    [window setTitle:window_title];
    [window setBackgroundColor:color];
    [window center];
    [window makeKeyAndOrderFront:nil];
    [application activateIgnoringOtherApps:YES];

    id monitor = [(id<MPFixtureEventClass>)event_class
        addLocalMonitorForEventsMatchingMask:MPFixtureEventMaskAny
                                     handler:^id(id event) {
                                       @try {
                                           uint32_t kind = 0;
                                           id<MPFixtureEvent> observed = (id<MPFixtureEvent>)event;
                                           if (mp_fixture_classify(observed.type, &kind)) {
                                               uint32_t units = 0;
                                               if (kind == MP_FIXTURE_EVENT_KEY_DOWN ||
                                                   kind == MP_FIXTURE_EVENT_KEY_UP) {
                                                   /* Length only. The characters
                                                    * themselves are never read out
                                                    * of this block. */
                                                   NSString *characters =
                                                       observed.characters;
                                                   units = (uint32_t)characters.length;
                                               }
                                               sink(context, kind, units);
                                           }
                                       } @catch (...) {
                                       }
                                       return event;
                                     }];
    if (monitor == nil) {
        return MP_FIXTURE_PLATFORM_FAILURE;
    }

    ready(context, (uint64_t)[window windowNumber], launch_context, signature_mode,
          signing_identifier, signing_identifier_len);
    [application run];
    return MP_FIXTURE_OK;
    MP_FIXTURE_END
}
