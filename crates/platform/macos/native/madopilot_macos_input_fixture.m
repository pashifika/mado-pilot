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
 * counts UTF-16 units and reports the count. Its default window content is one
 * fixed colour; opt-in benchmark modes alternate only deterministic colours or
 * sizes, so a captured frame still contains nothing from the user's desktop.
 */

#import <CoreGraphics/CoreGraphics.h>
#import <Foundation/Foundation.h>

#include <dlfcn.h>
#include <dispatch/dispatch.h>

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
/* Length-only selectors for the combined benchmark behavior. */
static const uint32_t MPFixtureAnimateTextUnits = 1;
static const uint32_t MPFixtureResizeTextUnits = 2;
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
- (void)setContentSize:(CGSize)size;
- (void)close;
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

/*
 * The process has one fixture window at a time. A strong process-owned slot is
 * required for replacement mode: the delayed block must release the destroyed
 * window before it creates the same-process successor, and the successor must
 * remain alive after that block returns.
 */
static __strong id<MPFixtureWindow> mp_fixture_window = nil;

static id mp_fixture_color(Class color_class, uint32_t fill) {
    return [(id<MPFixtureColorClass>)color_class
        colorWithSRGBRed:(CGFloat)((fill >> 16) & 0xFFu) / 255.0
                   green:(CGFloat)((fill >> 8) & 0xFFu) / 255.0
                    blue:(CGFloat)(fill & 0xFFu) / 255.0
                   alpha:1.0];
}

static id<MPFixtureWindow> mp_fixture_create_window(Class window_class, Class color_class,
                                                    NSString *title, uint32_t fill, double width,
                                                    double height) {
    id<MPFixtureWindow> window = [[(id)window_class alloc]
        initWithContentRect:CGRectMake(0.0, 0.0, width, height)
                  styleMask:MPFixtureWindowStyle
                    backing:MPFixtureBackingBuffered
                      defer:NO];
    if (window == nil) {
        return nil;
    }

    id color = mp_fixture_color(color_class, fill);
    if (color == nil) {
        return nil;
    }
    [window setReleasedWhenClosed:NO];
    [window setTitle:title];
    [window setBackgroundColor:color];
    [window center];
    [window makeKeyAndOrderFront:nil];
    return window;
}

uint32_t mp_fixture_run(const char *title, uint32_t fill, uint32_t replacement_fill,
                        uint32_t behavior, uint32_t replacement_delay_ms, double width,
                        double height, uint32_t launch_context, uint32_t signature_mode,
                        const uint8_t *signing_identifier, size_t signing_identifier_len,
                        void *context,
                        void (*ready)(void *context, uint64_t window_number,
                                      uint32_t launch_context, uint32_t signature_mode,
                                      const uint8_t *signing_identifier,
                                      size_t signing_identifier_len),
                        void (*replaced)(void *context, uint32_t status,
                                         uint64_t old_window_number,
                                         uint64_t new_window_number),
                        void (*sink)(void *context, uint32_t kind, uint32_t text_units)) {
    const uint32_t behavior_mask = MP_FIXTURE_BEHAVIOR_ANIMATE_ON_KEY_DOWN |
                                   MP_FIXTURE_BEHAVIOR_RESIZE_ON_KEY_DOWN;
    if (title == NULL || ready == NULL || replaced == NULL || sink == NULL || !(width >= 64.0) ||
        !(height >= 64.0) || !(width <= 4096.0) || !(height <= 4096.0) ||
        (behavior & ~behavior_mask) != 0u || replacement_delay_ms > 60000u ||
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
    [application activateIgnoringOtherApps:YES];

    mp_fixture_window =
        mp_fixture_create_window(window_class, color_class, window_title, fill, width, height);
    if (mp_fixture_window == nil) {
        return MP_FIXTURE_PLATFORM_FAILURE;
    }

    __block bool alternate_fill = false;
    __block bool alternate_size = false;
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
                                                   NSString *characters = observed.characters;
                                                   units = (uint32_t)characters.length;
                                               }
                                               sink(context, kind, units);
                                               bool animates =
                                                   (behavior &
                                                    MP_FIXTURE_BEHAVIOR_ANIMATE_ON_KEY_DOWN) != 0u;
                                               bool resizes =
                                                   (behavior &
                                                    MP_FIXTURE_BEHAVIOR_RESIZE_ON_KEY_DOWN) != 0u;
                                               bool animation_event =
                                                   animates &&
                                                   kind == MP_FIXTURE_EVENT_KEY_DOWN &&
                                                   (!resizes ||
                                                    units == MPFixtureAnimateTextUnits);
                                               if (animation_event) {
                                                   alternate_fill = !alternate_fill;
                                                   uint32_t benchmark_fill =
                                                       alternate_fill ? replacement_fill : fill;
                                                   id color = mp_fixture_color(color_class,
                                                                               benchmark_fill);
                                                   if (color != nil) {
                                                       [mp_fixture_window
                                                           setBackgroundColor:color];
                                                   }
                                               }
                                               bool resize_event =
                                                   resizes &&
                                                   kind == MP_FIXTURE_EVENT_KEY_DOWN &&
                                                   (!animates ||
                                                    units == MPFixtureResizeTextUnits);
                                               if (resize_event) {
                                                   alternate_size = !alternate_size;
                                                   CGSize size =
                                                       alternate_size
                                                           ? CGSizeMake(width + 180.0,
                                                                        height + 120.0)
                                                           : CGSizeMake(width, height);
                                                   [mp_fixture_window setContentSize:size];
                                               }
                                           }
                                       } @catch (...) {
                                       }
                                       return event;
                                     }];
    if (monitor == nil) {
        mp_fixture_window = nil;
        return MP_FIXTURE_PLATFORM_FAILURE;
    }

    ready(context, (uint64_t)[mp_fixture_window windowNumber], launch_context, signature_mode,
          signing_identifier, signing_identifier_len);

    if (replacement_delay_ms > 0) {
        dispatch_time_t replacement_time =
            dispatch_time(DISPATCH_TIME_NOW, (int64_t)replacement_delay_ms * NSEC_PER_MSEC);
        dispatch_after(replacement_time, dispatch_get_main_queue(), ^{
          uint64_t old_window_number = 0;
          @try {
              id<MPFixtureWindow> old_window = mp_fixture_window;
              if (old_window == nil) {
                  replaced(context, MP_FIXTURE_PLATFORM_FAILURE, 0, 0);
                  return;
              }
              old_window_number = (uint64_t)[old_window windowNumber];
              [old_window close];
              mp_fixture_window = nil;
              [application activateIgnoringOtherApps:YES];

              id<MPFixtureWindow> replacement =
                  mp_fixture_create_window(window_class, color_class, window_title,
                                           replacement_fill, width, height);
              if (replacement == nil) {
                  replaced(context, MP_FIXTURE_PLATFORM_FAILURE, old_window_number, 0);
                  return;
              }
              mp_fixture_window = replacement;
              replaced(context, MP_FIXTURE_OK, old_window_number,
                       (uint64_t)[replacement windowNumber]);
          } @catch (NSException *exception) {
              (void)exception;
              replaced(context, MP_FIXTURE_NATIVE_EXCEPTION, old_window_number, 0);
          } @catch (...) {
              replaced(context, MP_FIXTURE_NATIVE_EXCEPTION, old_window_number, 0);
          }
        });
    }

    [application run];
    mp_fixture_window = nil;
    return MP_FIXTURE_OK;
    MP_FIXTURE_END
}
