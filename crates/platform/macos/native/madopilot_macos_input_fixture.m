/*
 * MadoPilot macOS input fixture.
 *
 * Objective-C with Automatic Reference Counting. Compiled into an archive the
 * production Adapter does not link, so nothing here can reach a released
 * artifact.
 *
 * # Why AppKit and OpenGL are not imported
 *
 * For the reason the production shim does not import ScreenCaptureKit: an import
 * creates a load command, and this repository's linkage rule is that the Adapter
 * package declares exactly the frameworks it needs at load. The fixture opens
 * AppKit from its absolute system location and sends the handful of selectors it
 * needs, declared below without a framework header. Its opt-in game-like mode
 * separately opens OpenGL from its absolute system location and resolves every
 * required function before creating an OpenGL-backed content view.
 *
 * # What it deliberately does not do
 *
 * It never retains, prints, or forwards the characters of an observed event. It
 * counts UTF-16 units and reports the count. Its default window content remains
 * one AppKit background colour. The opt-in game-like content view and benchmark
 * modes use only the same deterministic approved colours or bounded sizes, so a
 * captured frame still contains nothing from the user's desktop.
 */

#import <CoreGraphics/CoreGraphics.h>
#import <Foundation/Foundation.h>

#include <dlfcn.h>
#include <dispatch/dispatch.h>
#include <math.h>
#include <stdatomic.h>
#include <string.h>
#include <unistd.h>

#include "madopilot_macos_input_fixture.h"

#if !__has_feature(objc_arc)
#error "the MadoPilot macOS input fixture requires Automatic Reference Counting"
#endif

static void mp_fixture_reset_state(void);

#define MP_FIXTURE_BEGIN @try {
#define MP_FIXTURE_END                                                                             \
    }                                                                                              \
    @catch (NSException * exception) {                                                             \
        (void)exception;                                                                           \
        return MP_FIXTURE_NATIVE_EXCEPTION;                                                        \
    }                                                                                              \
    @catch (...) {                                                                                 \
        return MP_FIXTURE_NATIVE_EXCEPTION;                                                        \
    }                                                                                              \
    @finally {                                                                                     \
        mp_fixture_reset_state();                                                                  \
    }

/* NSApplicationActivationPolicyRegular. */
static const NSInteger MPFixtureActivationRegular = 0;
/* NSWindowStyleMaskTitled | Closable | Miniaturizable. */
static const NSUInteger MPFixtureWindowStyle = 1 | 2 | 4;
/* NSBackingStoreBuffered. */
static const NSUInteger MPFixtureBackingBuffered = 2;
/* NSWindowTabbingModeDisallowed. The fixture owns independent, never-tabbed windows. */
static const NSInteger MPFixtureWindowTabbingDisallowed = 2;
/* NSEventMaskAny. */
static const unsigned long long MPFixtureEventMaskAny = ~0ull;
/* GL_COLOR_BUFFER_BIT, without importing OpenGL and creating a load command. */
static const uint32_t MPFixtureGLColorBufferBit = 0x00004000u;
/* Lets AppKit attach a newly ordered NSOpenGLView before its first clear. */
static const NSTimeInterval MPFixtureOpenGLDrawableWait = 0.01;
/* Fixed ceiling for the public NSScreen snapshot used by one topology command. */
enum { MPFixtureMaxScreens = 16 };
/* NSBoxCustom, NSNoBorder, and NSNoTitle without importing AppKit. */
static const NSUInteger MPFixtureBoxCustom = 4;
static const NSUInteger MPFixtureNoBorder = 0;
static const NSUInteger MPFixtureNoTitle = 0;
/* `watch-marker-v1`: a bounded asymmetric three-by-two cell pattern. */
static const CGFloat MPFixtureWatchMarkerX = 64.0;
static const CGFloat MPFixtureWatchMarkerTop = 48.0;
static const CGFloat MPFixtureWatchMarkerCell = 24.0;
static const uint32_t MPFixtureWatchMarkerPrimary = 0x00f26b38u;
static const uint32_t MPFixtureWatchMarkerSecondary = 0x002dd4bfu;

/* Qualification V2's token grid is disjoint from `watch-marker-v1`. */
static const CGFloat MPFixtureWatchTokenX = 176.0;
static const CGFloat MPFixtureWatchTokenTop = 48.0;
static const CGFloat MPFixtureWatchTokenCell = 8.0;
enum {
    MPFixtureWatchTokenGridWidth = 10,
    MPFixtureWatchTokenGridHeight = 9,
    MPFixtureWatchTokenCellCount =
        MPFixtureWatchTokenGridWidth * MPFixtureWatchTokenGridHeight,
};
_Static_assert(MPFixtureWatchTokenCellCount == 90,
               "qualification visual-token layout must remain 10x9");

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
- (void)finishLaunching;
- (void)activateIgnoringOtherApps:(BOOL)ignore;
- (void)run;
- (void)terminate:(id)sender;
@end

@protocol MPFixtureWorkspaceClass <NSObject>
+ (id)sharedWorkspace;
@end

@protocol MPFixtureWorkspace <NSObject>
@property(readonly) id frontmostApplication;
@end

@protocol MPFixtureRunningApplicationClass <NSObject>
+ (id)currentApplication;
@end

@protocol MPFixtureRunningApplication <NSObject>
- (BOOL)activateWithOptions:(NSUInteger)options;
@property(readonly) pid_t processIdentifier;
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
- (void)setContentView:(id)view;
- (id)contentView;
- (void)setReleasedWhenClosed:(BOOL)released;
- (void)setTabbingMode:(NSInteger)mode;
- (void)center;
- (void)makeKeyAndOrderFront:(id)sender;
- (void)orderFrontRegardless;
- (void)orderOut:(id)sender;
- (void)miniaturize:(id)sender;
- (void)deminiaturize:(id)sender;
- (void)setContentSize:(CGSize)size;
- (void)displayIfNeeded;
- (CGRect)frame;
- (void)setFrameOrigin:(CGPoint)origin;
- (void)close;
- (NSInteger)windowNumber;
@end

@protocol MPFixtureView <NSObject>
- (CGRect)frame;
- (void)setFrame:(CGRect)frame;
- (void)addSubview:(id)view;
- (void)removeFromSuperview;
- (void)setHidden:(BOOL)hidden;
@end

@protocol MPFixtureBox <MPFixtureView>
- (instancetype)initWithFrame:(CGRect)frame;
- (void)setBoxType:(NSUInteger)box_type;
- (void)setBorderType:(NSUInteger)border_type;
- (void)setFillColor:(id)color;
- (void)setTitlePosition:(NSUInteger)title_position;
- (void)setTransparent:(BOOL)transparent;
- (void)setContentViewMargins:(CGSize)margins;
@end

@protocol MPFixtureOpenGLPixelFormat <NSObject>
- (instancetype)initWithAttributes:(const uint32_t *)attributes;
@end

@protocol MPFixtureOpenGLContext <NSObject>
- (void)makeCurrentContext;
- (void)update;
- (void)flushBuffer;
@end

@protocol MPFixtureOpenGLView <NSObject>
- (instancetype)initWithFrame:(CGRect)frame pixelFormat:(id)pixel_format;
- (void)prepareOpenGL;
- (id<MPFixtureOpenGLContext>)openGLContext;
@end

@protocol MPFixtureScreenClass <NSObject>
+ (NSArray *)screens;
@end

@protocol MPFixtureScreen <NSObject>
- (CGRect)frame;
@end

@protocol MPFixtureEvent <NSObject>
@property(readonly) NSUInteger type;
@property(readonly) CGEventRef CGEvent;
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

typedef void (*MPFixtureGLClearColor)(float red, float green, float blue, float alpha);
typedef void (*MPFixtureGLClear)(uint32_t mask);
typedef void (*MPFixtureGLFinish)(void);
typedef uint32_t (*MPFixtureGLGetError)(void);

typedef struct MPFixtureOpenGLSymbols {
    void *handle;
    MPFixtureGLClearColor clear_color;
    MPFixtureGLClear clear;
    MPFixtureGLFinish finish;
    MPFixtureGLGetError get_error;
} MPFixtureOpenGLSymbols;

static MPFixtureOpenGLSymbols mp_fixture_opengl = {0};

static bool mp_fixture_load_opengl_path(const char *path, MPFixtureOpenGLSymbols *out_symbols) {
    MPFixtureOpenGLSymbols loaded = {0};
    loaded.handle = dlopen(path, RTLD_NOW | RTLD_LOCAL);
    if (loaded.handle == NULL) {
        return false;
    }
    loaded.clear_color = (MPFixtureGLClearColor)dlsym(loaded.handle, "glClearColor");
    loaded.clear = (MPFixtureGLClear)dlsym(loaded.handle, "glClear");
    loaded.finish = (MPFixtureGLFinish)dlsym(loaded.handle, "glFinish");
    loaded.get_error = (MPFixtureGLGetError)dlsym(loaded.handle, "glGetError");
    if (loaded.clear_color == NULL || loaded.clear == NULL || loaded.finish == NULL ||
        loaded.get_error == NULL) {
        (void)dlclose(loaded.handle);
        return false;
    }
    *out_symbols = loaded;
    return true;
}

static bool mp_fixture_load_opengl(void) {
    if (mp_fixture_opengl.handle != NULL) {
        return true;
    }
    MPFixtureOpenGLSymbols loaded = {0};
    if (!mp_fixture_load_opengl_path(
            "/System/Library/Frameworks/OpenGL.framework/Versions/A/OpenGL", &loaded) &&
        !mp_fixture_load_opengl_path("/System/Library/Frameworks/OpenGL.framework/OpenGL",
                                     &loaded)) {
        return false;
    }
    mp_fixture_opengl = loaded;
    return true;
}

uint32_t mp_fixture_test_unsupported_renderer(void) {
    MPFixtureOpenGLSymbols loaded = {0};
    bool unexpectedly_loaded = mp_fixture_load_opengl_path(
        "/System/Library/Frameworks/MadoPilotUnsupportedRenderer.framework/"
        "MadoPilotUnsupportedRenderer",
        &loaded);
    if (unexpectedly_loaded) {
        (void)dlclose(loaded.handle);
        return MP_FIXTURE_PLATFORM_FAILURE;
    }
    return MP_FIXTURE_UNSUPPORTED;
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

static uint64_t mp_fixture_fingerprint_u64(uint64_t state, uint64_t value) {
    const uint64_t prime = UINT64_C(0x00000100000001b3);
    for (uint32_t shift = 0; shift < 64; shift += 8) {
        state ^= (value >> shift) & UINT64_C(0xff);
        state *= prime;
    }
    return state;
}

static uint64_t mp_fixture_double_bits(double value) {
    uint64_t bits = 0;
    memcpy(&bits, &value, sizeof(bits));
    return bits;
}


static uint64_t mp_fixture_event_fingerprint(uint32_t kind, CGEventRef event,
                                             uint32_t *out_text_units) {
    const uint64_t offset = UINT64_C(0xcbf29ce484222325);
    const UniCharCount capacity = 256;
    UniChar text[256] = {0};
    UniCharCount text_units = 0;
    uint64_t button = 0;
    uint64_t click_state = 0;
    double x = 0.0;
    double y = 0.0;
    int64_t horizontal = 0;
    int64_t vertical = 0;
    uint64_t key_code = 0;
    if (kind == MP_FIXTURE_EVENT_POINTER_MOVE ||
        kind == MP_FIXTURE_EVENT_POINTER_PRESS ||
        kind == MP_FIXTURE_EVENT_POINTER_RELEASE) {
        CGPoint location = CGEventGetLocation(event);
        x = location.x;
        y = location.y;
        button = (uint64_t)CGEventGetIntegerValueField(
            event, kCGMouseEventButtonNumber);
        click_state = (uint64_t)CGEventGetIntegerValueField(
            event, kCGMouseEventClickState);
    } else if (kind == MP_FIXTURE_EVENT_POINTER_SCROLL) {
        CGPoint location = CGEventGetLocation(event);
        x = location.x;
        y = location.y;
        horizontal = CGEventGetIntegerValueField(
            event, kCGScrollWheelEventDeltaAxis2);
        vertical = CGEventGetIntegerValueField(
            event, kCGScrollWheelEventDeltaAxis1);
    } else {
        key_code = (uint64_t)CGEventGetIntegerValueField(
            event, kCGKeyboardEventKeycode);
        /*
         * A zero virtual key is the production text route and is also the
         * qualifying layout key. Other key rows are identified exactly by their
         * virtual key and flags; excluding their layout-derived characters keeps
         * this privacy-safe oracle deterministic.
         */
        if (key_code == 0) {
            CGEventKeyboardGetUnicodeString(event, capacity, &text_units, text);
        }
    }
    *out_text_units =
        text_units > UINT32_MAX ? UINT32_MAX : (uint32_t)text_units;
    uint64_t state = offset;
    const uint64_t fields[] = {
        (uint64_t)kind,
        (uint64_t)CGEventGetType(event),
        (uint64_t)CGEventGetFlags(event),
        button,
        click_state,
        mp_fixture_double_bits(x),
        mp_fixture_double_bits(y),
        (uint64_t)horizontal,
        (uint64_t)vertical,
        key_code,
        (uint64_t)text_units,
    };
    for (size_t index = 0; index < sizeof(fields) / sizeof(fields[0]); index += 1) {
        state = mp_fixture_fingerprint_u64(state, fields[index]);
    }
    UniCharCount bounded = text_units < capacity ? text_units : capacity;
    for (UniCharCount index = 0; index < bounded; index += 1) {
        state = mp_fixture_fingerprint_u64(state, (uint64_t)text[index]);
    }
    return state;
}


/*
 * The fixture has one primary window and at most one ordinary auxiliary
 * window. Every object below is owned by the fixture binary and touched only
 * on the AppKit main queue. The atomic flag is the sole cross-thread state: it
 * lets the stdin decoder refuse commands before ready or after termination
 * without racing an Objective-C object.
 */
typedef void (*MPFixtureControlledCallback)(void *context, uint64_t nonce,
                                            uint32_t command, uint32_t status,
                                            uint64_t before_window_number,
                                            uint64_t after_window_number);

static __strong id<MPFixtureWindow> mp_fixture_window = nil;
static __strong id<MPFixtureWindow> mp_fixture_auxiliary_window = nil;
static __strong id<MPFixtureApplication> mp_fixture_application = nil;
static __strong id<MPFixtureRunningApplication> mp_fixture_prior_application = nil;
static __strong id<MPFixtureRunningApplication> mp_fixture_current_application = nil;
static __strong NSString *mp_fixture_window_title = nil;
static Class mp_fixture_window_class = Nil;
static Class mp_fixture_color_class = Nil;
static Class mp_fixture_opengl_pixel_format_class = Nil;
static Class mp_fixture_opengl_view_class = Nil;
static Class mp_fixture_box_class = Nil;
static uint32_t mp_fixture_renderer = MP_FIXTURE_RENDERER_APPKIT_BACKGROUND;
static uint32_t mp_fixture_fill = 0;
static uint32_t mp_fixture_replacement_fill = 0;
static double mp_fixture_width = 0.0;
static double mp_fixture_height = 0.0;
static __strong id<MPFixtureBox> mp_fixture_marker_primary = nil;
static __strong id<MPFixtureBox> mp_fixture_marker_top_middle = nil;
static __strong id<MPFixtureBox> mp_fixture_marker_bottom_left = nil;
static __strong id<MPFixtureBox>
    mp_fixture_token_cells[MPFixtureWatchTokenCellCount] = {nil};
static bool mp_fixture_marker_visible = false;
static uint32_t mp_fixture_visual_token = 0;
static bool mp_fixture_alternate_fill = false;
static bool mp_fixture_moved = false;
static bool mp_fixture_resized = false;
static bool mp_fixture_offscreen = false;
static CGPoint mp_fixture_onscreen_origin = {0.0, 0.0};
static bool mp_fixture_placement_saved = false;
static CGPoint mp_fixture_saved_placement = {0.0, 0.0};
static bool mp_fixture_activate = false;
static void *mp_fixture_control_context = NULL;
static MPFixtureControlledCallback mp_fixture_controlled = NULL;
static _Atomic uint64_t mp_fixture_run_nonce = 0;
static uint64_t mp_fixture_last_nonce = 0;
static uint64_t mp_fixture_last_event_payload_tag = 0;
static _Atomic uint64_t mp_fixture_event_payload_tag = 0;
static uint32_t mp_fixture_last_command = 0;
static uint32_t mp_fixture_last_status = MP_FIXTURE_OK;
static uint64_t mp_fixture_last_before = 0;
static uint64_t mp_fixture_last_after = 0;
static atomic_bool mp_fixture_control_active = false;

static id mp_fixture_color(Class color_class, uint32_t fill) {
    return [(id<MPFixtureColorClass>)color_class
        colorWithSRGBRed:(CGFloat)((fill >> 16) & 0xFFu) / 255.0
                   green:(CGFloat)((fill >> 8) & 0xFFu) / 255.0
                    blue:(CGFloat)(fill & 0xFFu) / 255.0
                   alpha:1.0];
}

static id<MPFixtureBox> mp_fixture_create_visual_box(uint32_t fill) {
    if (mp_fixture_box_class == Nil || mp_fixture_color_class == Nil) {
        return nil;
    }
    id<MPFixtureBox> box =
        [[(id)mp_fixture_box_class alloc] initWithFrame:CGRectMake(0.0, 0.0, 1.0, 1.0)];
    id color = mp_fixture_color(mp_fixture_color_class, fill);
    if (box == nil || color == nil) {
        return nil;
    }
    [box setBoxType:MPFixtureBoxCustom];
    [box setBorderType:MPFixtureNoBorder];
    [box setFillColor:color];
    [box setTitlePosition:MPFixtureNoTitle];
    [box setTransparent:NO];
    [box setContentViewMargins:CGSizeMake(0.0, 0.0)];
    [box setHidden:YES];
    return box;
}

static uint8_t mp_fixture_visual_token_checksum(uint32_t token, bool visible) {
    uint32_t marker_salt = visible ? 0x15a53c6du : 0x0a5ac396u;
    uint32_t rotated_left = (token << 7) | (token >> 25);
    uint32_t rotated_right = (token >> 11) | (token << 21);
    uint32_t mixed = token ^ rotated_left ^ rotated_right ^ marker_salt;
    mixed ^= mixed >> 16;
    mixed ^= mixed >> 8;
    return (uint8_t)(mixed & 0x1fu);
}

static bool mp_fixture_visual_token_cell(uint32_t token, bool visible,
                                         size_t index) {
    static const bool top[MPFixtureWatchTokenGridWidth] = {
        true, false, true, true, false, false, true, false, false, true,
    };
    static const bool bottom[MPFixtureWatchTokenGridWidth] = {
        false, false, true, false, true, true, true, false, true, false,
    };
    if (index < MPFixtureWatchTokenGridWidth) {
        return top[index];
    }
    if (index >=
        MPFixtureWatchTokenCellCount - MPFixtureWatchTokenGridWidth) {
        return bottom[index -
                      (MPFixtureWatchTokenCellCount -
                       MPFixtureWatchTokenGridWidth)];
    }

    size_t payload = index - MPFixtureWatchTokenGridWidth;
    if (payload < 32) {
        return ((token >> payload) & 1u) != 0;
    }
    if (payload < 64) {
        return (((~token) >> (payload - 32)) & 1u) != 0;
    }
    if (payload == 64) {
        return visible;
    }
    size_t checksum_bit = payload - 65;
    return ((mp_fixture_visual_token_checksum(token, visible) >>
             checksum_bit) &
            1u) != 0;
}

static void mp_fixture_remove_visual_views(void) {
    for (size_t index = 0; index < MPFixtureWatchTokenCellCount; index += 1) {
        [mp_fixture_token_cells[index] removeFromSuperview];
        mp_fixture_token_cells[index] = nil;
    }
    [mp_fixture_marker_bottom_left removeFromSuperview];
    [mp_fixture_marker_top_middle removeFromSuperview];
    [mp_fixture_marker_primary removeFromSuperview];
    mp_fixture_marker_bottom_left = nil;
    mp_fixture_marker_top_middle = nil;
    mp_fixture_marker_primary = nil;
}

static bool mp_fixture_layout_visual_views(id<MPFixtureWindow> window,
                                           bool visible, uint32_t token) {
    id<MPFixtureView> content = (id<MPFixtureView>)[window contentView];
    if (window == nil || content == nil || mp_fixture_marker_primary == nil ||
        mp_fixture_marker_top_middle == nil ||
        mp_fixture_marker_bottom_left == nil) {
        return false;
    }
    for (size_t index = 0; index < MPFixtureWatchTokenCellCount; index += 1) {
        if (mp_fixture_token_cells[index] == nil) {
            return false;
        }
    }

    CGRect content_frame = [content frame];
    CGFloat marker_width = MPFixtureWatchMarkerCell * 3.0;
    CGFloat marker_height = MPFixtureWatchMarkerCell * 2.0;
    CGFloat marker_bottom =
        content_frame.size.height - MPFixtureWatchMarkerTop - marker_height;
    CGFloat token_width =
        MPFixtureWatchTokenCell * (CGFloat)MPFixtureWatchTokenGridWidth;
    CGFloat token_height =
        MPFixtureWatchTokenCell * (CGFloat)MPFixtureWatchTokenGridHeight;
    CGFloat token_bottom =
        content_frame.size.height - MPFixtureWatchTokenTop - token_height;
    if (!isfinite(content_frame.size.width) ||
        !isfinite(content_frame.size.height) || MPFixtureWatchMarkerX < 0.0 ||
        marker_bottom < 0.0 ||
        MPFixtureWatchMarkerX + marker_width > content_frame.size.width ||
        MPFixtureWatchTokenX < 0.0 || token_bottom < 0.0 ||
        MPFixtureWatchTokenX + token_width > content_frame.size.width) {
        return false;
    }

    [mp_fixture_marker_primary
        setFrame:CGRectMake(MPFixtureWatchMarkerX, marker_bottom, marker_width,
                            marker_height)];
    [mp_fixture_marker_top_middle
        setFrame:CGRectMake(MPFixtureWatchMarkerX + MPFixtureWatchMarkerCell,
                            marker_bottom + MPFixtureWatchMarkerCell,
                            MPFixtureWatchMarkerCell,
                            MPFixtureWatchMarkerCell)];
    [mp_fixture_marker_bottom_left
        setFrame:CGRectMake(MPFixtureWatchMarkerX, marker_bottom,
                            MPFixtureWatchMarkerCell,
                            MPFixtureWatchMarkerCell)];
    [mp_fixture_marker_primary setHidden:!visible];
    [mp_fixture_marker_top_middle setHidden:!visible];
    [mp_fixture_marker_bottom_left setHidden:!visible];

    id primary =
        mp_fixture_color(mp_fixture_color_class, MPFixtureWatchMarkerPrimary);
    id secondary =
        mp_fixture_color(mp_fixture_color_class, MPFixtureWatchMarkerSecondary);
    if (primary == nil || secondary == nil) {
        return false;
    }
    for (size_t index = 0; index < MPFixtureWatchTokenCellCount; index += 1) {
        size_t row = index / MPFixtureWatchTokenGridWidth;
        size_t column = index % MPFixtureWatchTokenGridWidth;
        CGFloat left =
            MPFixtureWatchTokenX + (CGFloat)column * MPFixtureWatchTokenCell;
        CGFloat bottom =
            content_frame.size.height - MPFixtureWatchTokenTop -
            ((CGFloat)row + 1.0) * MPFixtureWatchTokenCell;
        id<MPFixtureBox> cell = mp_fixture_token_cells[index];
        [cell setFrame:CGRectMake(left, bottom, MPFixtureWatchTokenCell,
                                  MPFixtureWatchTokenCell)];
        if (token != 0) {
            [cell setFillColor:mp_fixture_visual_token_cell(token, visible, index)
                                   ? primary
                                   : secondary];
        }
        [cell setHidden:token == 0];
    }
    [window displayIfNeeded];
    return true;
}

static bool mp_fixture_install_visual_views(id<MPFixtureWindow> window) {
    id<MPFixtureView> content = (id<MPFixtureView>)[window contentView];
    if (window == nil || content == nil) {
        return false;
    }
    mp_fixture_remove_visual_views();
    mp_fixture_marker_primary =
        mp_fixture_create_visual_box(MPFixtureWatchMarkerPrimary);
    mp_fixture_marker_top_middle =
        mp_fixture_create_visual_box(MPFixtureWatchMarkerSecondary);
    mp_fixture_marker_bottom_left =
        mp_fixture_create_visual_box(MPFixtureWatchMarkerSecondary);
    if (mp_fixture_marker_primary == nil ||
        mp_fixture_marker_top_middle == nil ||
        mp_fixture_marker_bottom_left == nil) {
        mp_fixture_remove_visual_views();
        return false;
    }
    [content addSubview:mp_fixture_marker_primary];
    [content addSubview:mp_fixture_marker_top_middle];
    [content addSubview:mp_fixture_marker_bottom_left];

    for (size_t index = 0; index < MPFixtureWatchTokenCellCount; index += 1) {
        id<MPFixtureBox> cell =
            mp_fixture_create_visual_box(MPFixtureWatchMarkerSecondary);
        if (cell == nil) {
            mp_fixture_remove_visual_views();
            return false;
        }
        mp_fixture_token_cells[index] = cell;
        [content addSubview:cell];
    }
    return mp_fixture_layout_visual_views(window, false, 0);
}

static bool mp_fixture_set_visual_state(id<MPFixtureWindow> window,
                                        bool visible, uint32_t token) {
    if (token == 0 ||
        !mp_fixture_layout_visual_views(window, visible, token)) {
        return false;
    }
    mp_fixture_marker_visible = visible;
    mp_fixture_visual_token = token;
    return true;
}

static bool mp_fixture_layout_current_visual(id<MPFixtureWindow> window) {
    return mp_fixture_layout_visual_views(
        window, mp_fixture_marker_visible, mp_fixture_visual_token);
}

static bool mp_fixture_install_opengl_content(id<MPFixtureWindow> window, double width,
                                              double height) {
    if (mp_fixture_opengl_pixel_format_class == Nil || mp_fixture_opengl_view_class == Nil ||
        mp_fixture_opengl.handle == NULL) {
        return false;
    }
    static const uint32_t attributes[] = {
        99u, 0x3200u, /* NSOpenGLPFAOpenGLProfile, NSOpenGLProfileVersion3_2Core. */
        8u,  24u,     /* NSOpenGLPFAColorSize. */
        11u, 8u,      /* NSOpenGLPFAAlphaSize. */
        5u,           /* NSOpenGLPFADoubleBuffer. */
        73u,          /* NSOpenGLPFAAccelerated. */
        0u,
    };
    id<MPFixtureOpenGLPixelFormat> pixel_format =
        [[(id)mp_fixture_opengl_pixel_format_class alloc] initWithAttributes:attributes];
    if (pixel_format == nil) {
        return false;
    }
    id<MPFixtureOpenGLView> view =
        [[(id)mp_fixture_opengl_view_class alloc]
            initWithFrame:CGRectMake(0.0, 0.0, width, height)
              pixelFormat:pixel_format];
    if (view == nil) {
        return false;
    }
    id<MPFixtureOpenGLContext> context = [view openGLContext];
    if (context == nil) {
        return false;
    }
    [window setContentView:view];
    [view prepareOpenGL];
    [context update];
    return true;
}

static bool mp_fixture_apply_fill(id<MPFixtureWindow> window, uint32_t fill) {
    if (window == nil) {
        return false;
    }
    if (mp_fixture_renderer == MP_FIXTURE_RENDERER_APPKIT_BACKGROUND) {
        id color = mp_fixture_color(mp_fixture_color_class, fill);
        if (color == nil) {
            return false;
        }
        [window setBackgroundColor:color];
        return true;
    }
    if (mp_fixture_renderer != MP_FIXTURE_RENDERER_OPENGL ||
        mp_fixture_opengl.clear_color == NULL || mp_fixture_opengl.clear == NULL ||
        mp_fixture_opengl.finish == NULL || mp_fixture_opengl.get_error == NULL) {
        return false;
    }
    id<MPFixtureOpenGLView> view = (id<MPFixtureOpenGLView>)[window contentView];
    id<MPFixtureOpenGLContext> context = [view openGLContext];
    if (view == nil || context == nil) {
        return false;
    }
    [context update];
    [context makeCurrentContext];
    if (mp_fixture_opengl.get_error() != 0u) {
        return false;
    }
    mp_fixture_opengl.clear_color((float)((fill >> 16) & 0xFFu) / 255.0f,
                                  (float)((fill >> 8) & 0xFFu) / 255.0f,
                                  (float)(fill & 0xFFu) / 255.0f, 1.0f);
    mp_fixture_opengl.clear(MPFixtureGLColorBufferBit);
    [context flushBuffer];
    mp_fixture_opengl.finish();
    return mp_fixture_opengl.get_error() == 0u;
}
static bool mp_fixture_prepare_opengl_content(id<MPFixtureWindow> window, uint32_t fill) {
    [window displayIfNeeded];
    [[NSRunLoop currentRunLoop]
        runUntilDate:[NSDate dateWithTimeIntervalSinceNow:MPFixtureOpenGLDrawableWait]];
    return mp_fixture_apply_fill(window, fill);
}

static id<MPFixtureWindow> mp_fixture_create_window(Class window_class, NSString *title,
                                                    uint32_t fill, double width, double height) {
    id<MPFixtureWindow> window = [[(id)window_class alloc]
        initWithContentRect:CGRectMake(0.0, 0.0, width, height)
                  styleMask:MPFixtureWindowStyle
                    backing:MPFixtureBackingBuffered
                      defer:NO];
    if (window == nil) {
        return nil;
    }
    [window setReleasedWhenClosed:NO];
    [window setTabbingMode:MPFixtureWindowTabbingDisallowed];
    [window setTitle:title];
    if (mp_fixture_renderer == MP_FIXTURE_RENDERER_OPENGL) {
        if (!mp_fixture_install_opengl_content(window, width, height)) {
            [window close];
            return nil;
        }
    } else if (!mp_fixture_apply_fill(window, fill)) {
        [window close];
        return nil;
    }
    [window center];
    if (mp_fixture_activate) {
        [window makeKeyAndOrderFront:nil];
    } else {
        [window orderFrontRegardless];
    }
    if (mp_fixture_renderer == MP_FIXTURE_RENDERER_OPENGL &&
        !mp_fixture_prepare_opengl_content(window, fill)) {
        [window close];
        return nil;
    }
    return window;
}

static id<MPFixtureWindow> mp_fixture_create_auxiliary_window(void) {
    if (mp_fixture_window == nil || mp_fixture_window_class == Nil ||
        mp_fixture_color_class == Nil || mp_fixture_window_title == nil) {
        return nil;
    }
    id<MPFixtureWindow> window = [[(id)mp_fixture_window_class alloc]
        initWithContentRect:CGRectMake(0.0, 0.0, 240.0, 160.0)
                  styleMask:MPFixtureWindowStyle
                    backing:MPFixtureBackingBuffered
                      defer:NO];
    if (window == nil) {
        return nil;
    }
    [window setReleasedWhenClosed:NO];
    [window setTabbingMode:MPFixtureWindowTabbingDisallowed];
    [window setTitle:[mp_fixture_window_title stringByAppendingString:@" Auxiliary"]];
    if (mp_fixture_renderer == MP_FIXTURE_RENDERER_OPENGL) {
        if (!mp_fixture_install_opengl_content(window, 240.0, 160.0)) {
            [window close];
            return nil;
        }
    } else if (!mp_fixture_apply_fill(window, mp_fixture_fill)) {
        [window close];
        return nil;
    }
    CGRect main_frame = [mp_fixture_window frame];
    [window setFrameOrigin:CGPointMake(main_frame.origin.x + 80.0, main_frame.origin.y + 80.0)];
    [window orderFrontRegardless];
    if (mp_fixture_renderer == MP_FIXTURE_RENDERER_OPENGL &&
        !mp_fixture_prepare_opengl_content(window, mp_fixture_fill)) {
        [window close];
        return nil;
    }
    return window;
}


static uint64_t mp_fixture_window_number(void) {
    return mp_fixture_window == nil ? 0 : (uint64_t)[mp_fixture_window windowNumber];
}

static uint32_t mp_fixture_current_fill(void) {
    return mp_fixture_alternate_fill ? mp_fixture_replacement_fill : mp_fixture_fill;
}

static bool mp_fixture_frame_is_valid(CGRect frame) {
    return isfinite(frame.origin.x) && isfinite(frame.origin.y) &&
           isfinite(frame.size.width) && isfinite(frame.size.height) &&
           frame.size.width > 0.0 && frame.size.height > 0.0;
}

static bool mp_fixture_frame_precedes(CGRect left, CGRect right) {
    if (left.origin.x != right.origin.x) {
        return left.origin.x < right.origin.x;
    }
    if (left.origin.y != right.origin.y) {
        return left.origin.y < right.origin.y;
    }
    if (left.size.width != right.size.width) {
        return left.size.width < right.size.width;
    }
    return left.size.height < right.size.height;
}

static double mp_fixture_intersection_area(CGRect left, CGRect right) {
    double width = fmin(CGRectGetMaxX(left), CGRectGetMaxX(right)) -
                   fmax(CGRectGetMinX(left), CGRectGetMinX(right));
    double height = fmin(CGRectGetMaxY(left), CGRectGetMaxY(right)) -
                    fmax(CGRectGetMinY(left), CGRectGetMinY(right));
    return width > 0.0 && height > 0.0 ? width * height : 0.0;
}

/*
 * Moves without ordering or activating the window. Sorting the bounded public
 * screen-frame snapshot makes the next display independent of enumeration order.
 */
static uint32_t mp_fixture_move_to_next_display(void) {
    if (mp_fixture_window == nil) {
        return MP_FIXTURE_PLATFORM_FAILURE;
    }
    Class screen_class = NSClassFromString(@"NSScreen");
    if (screen_class == Nil) {
        return MP_FIXTURE_UNSUPPORTED;
    }
    NSArray *screens = [(id<MPFixtureScreenClass>)screen_class screens];
    NSUInteger count = screens.count;
    if (count < 2) {
        return MP_FIXTURE_UNSUPPORTED;
    }
    if (count > MPFixtureMaxScreens) {
        return MP_FIXTURE_PLATFORM_FAILURE;
    }

    CGRect frames[MPFixtureMaxScreens];
    for (NSUInteger index = 0; index < count; index++) {
        CGRect frame = [(id<MPFixtureScreen>)[screens objectAtIndex:index] frame];
        if (!mp_fixture_frame_is_valid(frame)) {
            return MP_FIXTURE_PLATFORM_FAILURE;
        }
        frames[index] = frame;
    }
    for (NSUInteger index = 1; index < count; index++) {
        CGRect frame = frames[index];
        NSUInteger cursor = index;
        while (cursor > 0 && mp_fixture_frame_precedes(frame, frames[cursor - 1])) {
            frames[cursor] = frames[cursor - 1];
            cursor--;
        }
        frames[cursor] = frame;
    }

    CGRect window_frame = [mp_fixture_window frame];
    if (!mp_fixture_frame_is_valid(window_frame)) {
        return MP_FIXTURE_PLATFORM_FAILURE;
    }
    NSUInteger current = 0;
    double current_area = 0.0;
    for (NSUInteger index = 0; index < count; index++) {
        double area = mp_fixture_intersection_area(window_frame, frames[index]);
        if (area > current_area) {
            current = index;
            current_area = area;
        }
    }
    if (current_area == 0.0) {
        return MP_FIXTURE_PLATFORM_FAILURE;
    }

    CGRect source = frames[current];
    CGRect destination = frames[(current + 1) % count];
    double relative_y = window_frame.origin.y - source.origin.y;
    double max_y = fmax(destination.size.height - window_frame.size.height, 0.0);
    double destination_x = destination.origin.x;
    if (destination.origin.x < source.origin.x) {
        destination_x =
            CGRectGetMaxX(destination) - fmin(window_frame.size.width, destination.size.width);
    } else if (destination.origin.x == source.origin.x) {
        double relative_x = window_frame.origin.x - source.origin.x;
        double max_x = fmax(destination.size.width - window_frame.size.width, 0.0);
        destination_x = destination.origin.x + fmin(fmax(relative_x, 0.0), max_x);
    }
    CGPoint destination_origin =
        CGPointMake(destination_x,
                    destination.origin.y + fmin(fmax(relative_y, 0.0), max_y));
    if (!mp_fixture_placement_saved) {
        mp_fixture_saved_placement = window_frame.origin;
        mp_fixture_placement_saved = true;
    }
    [mp_fixture_window setFrameOrigin:destination_origin];
    if ((mp_fixture_renderer == MP_FIXTURE_RENDERER_OPENGL &&
         !mp_fixture_apply_fill(mp_fixture_window, mp_fixture_current_fill())) ||
        !mp_fixture_layout_current_visual(mp_fixture_window)) {
        return MP_FIXTURE_PLATFORM_FAILURE;
    }
    return MP_FIXTURE_OK;
}

static uint32_t mp_fixture_restore_placement(void) {
    if (mp_fixture_window == nil || !mp_fixture_placement_saved ||
        mp_fixture_offscreen) {
        return MP_FIXTURE_INVALID_ARGUMENT;
    }
    [mp_fixture_window setFrameOrigin:mp_fixture_saved_placement];
    if ((mp_fixture_renderer == MP_FIXTURE_RENDERER_OPENGL &&
         !mp_fixture_apply_fill(mp_fixture_window, mp_fixture_current_fill())) ||
        !mp_fixture_layout_current_visual(mp_fixture_window)) {
        return MP_FIXTURE_PLATFORM_FAILURE;
    }
    mp_fixture_placement_saved = false;
    mp_fixture_saved_placement = CGPointMake(0.0, 0.0);
    return MP_FIXTURE_OK;
}

/*
 * Moves wholly beyond the right edge of every current public display without
 * ordering or activating the window. The exact prior origin is restored by a
 * separate command so the target-loss row remains reversible.
 */
static uint32_t mp_fixture_move_offscreen(void) {
    if (mp_fixture_window == nil || mp_fixture_offscreen) {
        return MP_FIXTURE_INVALID_ARGUMENT;
    }
    Class screen_class = NSClassFromString(@"NSScreen");
    if (screen_class == Nil) {
        return MP_FIXTURE_UNSUPPORTED;
    }
    NSArray *screens = [(id<MPFixtureScreenClass>)screen_class screens];
    NSUInteger count = screens.count;
    if (count == 0 || count > MPFixtureMaxScreens) {
        return MP_FIXTURE_PLATFORM_FAILURE;
    }

    double display_max_x = -INFINITY;
    for (NSUInteger index = 0; index < count; index++) {
        CGRect frame = [(id<MPFixtureScreen>)[screens objectAtIndex:index] frame];
        if (!mp_fixture_frame_is_valid(frame)) {
            return MP_FIXTURE_PLATFORM_FAILURE;
        }
        display_max_x = fmax(display_max_x, CGRectGetMaxX(frame));
    }
    CGRect window_frame = [mp_fixture_window frame];
    if (!mp_fixture_frame_is_valid(window_frame)) {
        return MP_FIXTURE_PLATFORM_FAILURE;
    }
    CGPoint destination =
        CGPointMake(display_max_x + window_frame.size.width + 4096.0,
                    window_frame.origin.y);
    if (!isfinite(destination.x) || !isfinite(destination.y)) {
        return MP_FIXTURE_PLATFORM_FAILURE;
    }

    mp_fixture_onscreen_origin = window_frame.origin;
    [mp_fixture_window orderOut:nil];
    [mp_fixture_window setFrameOrigin:destination];
    if ((mp_fixture_renderer == MP_FIXTURE_RENDERER_OPENGL &&
         !mp_fixture_apply_fill(mp_fixture_window, mp_fixture_current_fill())) ||
        !mp_fixture_layout_current_visual(mp_fixture_window)) {
        [mp_fixture_window setFrameOrigin:mp_fixture_onscreen_origin];
        [mp_fixture_window orderFrontRegardless];
        return MP_FIXTURE_PLATFORM_FAILURE;
    }
    mp_fixture_offscreen = true;
    return MP_FIXTURE_OK;
}

static uint32_t mp_fixture_restore_onscreen(void) {
    if (mp_fixture_window == nil || !mp_fixture_offscreen) {
        return MP_FIXTURE_INVALID_ARGUMENT;
    }
    [mp_fixture_window setFrameOrigin:mp_fixture_onscreen_origin];
    [mp_fixture_window orderFrontRegardless];
    if ((mp_fixture_renderer == MP_FIXTURE_RENDERER_OPENGL &&
         !mp_fixture_apply_fill(mp_fixture_window, mp_fixture_current_fill())) ||
        !mp_fixture_layout_current_visual(mp_fixture_window)) {
        return MP_FIXTURE_PLATFORM_FAILURE;
    }
    mp_fixture_offscreen = false;
    return MP_FIXTURE_OK;
}

static void mp_fixture_reset_state(void) {
    atomic_store_explicit(&mp_fixture_control_active, false, memory_order_release);
    mp_fixture_remove_visual_views();
    mp_fixture_window = nil;
    mp_fixture_auxiliary_window = nil;
    mp_fixture_application = nil;
    mp_fixture_prior_application = nil;
    mp_fixture_current_application = nil;
    mp_fixture_window_title = nil;
    mp_fixture_window_class = Nil;
    mp_fixture_color_class = Nil;
    mp_fixture_opengl_pixel_format_class = Nil;
    mp_fixture_opengl_view_class = Nil;
    mp_fixture_box_class = Nil;
    mp_fixture_renderer = MP_FIXTURE_RENDERER_APPKIT_BACKGROUND;
    mp_fixture_fill = 0;
    mp_fixture_replacement_fill = 0;
    mp_fixture_width = 0.0;
    mp_fixture_height = 0.0;
    mp_fixture_marker_visible = false;
    mp_fixture_visual_token = 0;
    mp_fixture_alternate_fill = false;
    mp_fixture_moved = false;
    mp_fixture_resized = false;
    mp_fixture_offscreen = false;
    mp_fixture_onscreen_origin = CGPointMake(0.0, 0.0);
    mp_fixture_placement_saved = false;
    mp_fixture_saved_placement = CGPointMake(0.0, 0.0);
    mp_fixture_activate = false;
    mp_fixture_control_context = NULL;
    mp_fixture_controlled = NULL;
    atomic_store_explicit(&mp_fixture_run_nonce, 0, memory_order_relaxed);
    mp_fixture_last_nonce = 0;
    mp_fixture_last_command = 0;
    mp_fixture_last_event_payload_tag = 0;
    atomic_store_explicit(&mp_fixture_event_payload_tag, 0, memory_order_relaxed);
    mp_fixture_last_status = MP_FIXTURE_OK;
    mp_fixture_last_before = 0;
    mp_fixture_last_after = 0;
}

typedef struct MPFixtureControlTesting {
    uint32_t scenario;
    uint64_t last_nonce;
    uint32_t last_command;
    uint64_t last_event_payload_tag;
    uint32_t last_status;
    uint64_t last_before;
    uint64_t last_after;
    uint32_t completion_count;
    uint32_t completion_status;
    uint64_t completion_before;
    uint64_t completion_after;
    uint32_t termination_calls;
    uint32_t fail_closed_calls;
} MPFixtureControlTesting;

static const uint64_t MPFixtureTestingBeforeWindow = 41;
static const uint64_t MPFixtureTestingAfterWindow = 42;

static void mp_fixture_testing_raise_control(MPFixtureControlTesting *testing,
                                             uint32_t scenario) {
    if (testing != NULL && testing->scenario == scenario) {
        [NSException raise:@"MPFixtureInjectedFailure" format:@"fixture control"];
    }
}

static void mp_fixture_fail_closed(MPFixtureControlTesting *testing) {
    if (testing != NULL) {
        testing->fail_closed_calls += 1;
        return;
    }
    atomic_store_explicit(&mp_fixture_control_active, false, memory_order_release);
    _exit(1);
}

static uint64_t mp_fixture_control_window_number(MPFixtureControlTesting *testing,
                                                 uint32_t scenario,
                                                 uint64_t testing_value) {
    mp_fixture_testing_raise_control(testing, scenario);
    return testing == NULL ? mp_fixture_window_number() : testing_value;
}

static void mp_fixture_terminate_control(MPFixtureControlTesting *testing,
                                         uint32_t scenario) {
    if (testing != NULL) {
        testing->termination_calls += 1;
        mp_fixture_testing_raise_control(testing, scenario);
        return;
    }
    [mp_fixture_application terminate:nil];
}

/*
 * Completes one accepted command without any Objective-C message send. The Rust
 * callback contains its own panics; if it nevertheless raises, fail the private
 * fixture closed rather than retrying a possibly delivered completion.
 */
static void mp_fixture_complete_control(MPFixtureControlTesting *testing,
                                        uint64_t nonce, uint32_t command,
                                        uint64_t event_payload_tag, bool cache_result,
                                        uint32_t status, uint64_t before, uint64_t after) {
    if (cache_result) {
        if (testing == NULL) {
            mp_fixture_last_nonce = nonce;
            mp_fixture_last_command = command;
            mp_fixture_last_event_payload_tag = event_payload_tag;
            mp_fixture_last_status = status;
            mp_fixture_last_before = before;
            mp_fixture_last_after = after;
        } else {
            testing->last_nonce = nonce;
            testing->last_command = command;
            testing->last_event_payload_tag = event_payload_tag;
            testing->last_status = status;
            testing->last_before = before;
            testing->last_after = after;
        }
    }

    if (testing != NULL) {
        testing->completion_count += 1;
        testing->completion_status = status;
        testing->completion_before = before;
        testing->completion_after = after;
        return;
    }

    MPFixtureControlledCallback controlled = mp_fixture_controlled;
    void *context = mp_fixture_control_context;
    if (controlled == NULL) {
        mp_fixture_fail_closed(NULL);
    }
    @try {
        controlled(context, nonce, command, status, before, after);
    } @catch (NSException *exception) {
        (void)exception;
        mp_fixture_fail_closed(NULL);
    } @catch (...) {
        mp_fixture_fail_closed(NULL);
    }
}

static bool mp_fixture_valid_command(uint32_t command) {
    return command >= MP_FIXTURE_COMMAND_TRANSITION &&
           command <= MP_FIXTURE_COMMAND_RESTORE_PLACEMENT;
}

static void mp_fixture_run_control_block(uint64_t run_nonce, uint64_t nonce,
                                         uint32_t command, uint64_t event_payload_tag,
                                         MPFixtureControlTesting *testing) {
    bool completion_owed = false;
    bool completion_sent = false;
    bool cache_result = false;
    uint32_t status = MP_FIXTURE_OK;
    uint64_t before = 0;
    uint64_t after = 0;
    bool should_stop = false;

    @try {
        if (testing == NULL &&
            (!atomic_load_explicit(&mp_fixture_control_active, memory_order_acquire) ||
             mp_fixture_controlled == NULL ||
             run_nonce !=
                 atomic_load_explicit(&mp_fixture_run_nonce, memory_order_relaxed))) {
            return;
        }
        completion_owed = true;

        uint64_t last_nonce =
            testing == NULL ? mp_fixture_last_nonce : testing->last_nonce;
        uint32_t last_command =
            testing == NULL ? mp_fixture_last_command : testing->last_command;
        uint64_t last_event_payload_tag = testing == NULL
                                              ? mp_fixture_last_event_payload_tag
                                              : testing->last_event_payload_tag;
        if (nonce < last_nonce ||
            (nonce == last_nonce &&
             (command != last_command ||
              event_payload_tag != last_event_payload_tag))) {
            uint64_t current = mp_fixture_control_window_number(
                testing, MP_FIXTURE_TEST_CONTROL_PRE_WINDOW_EXCEPTION,
                MPFixtureTestingBeforeWindow);
            before = current;
            after = current;
            status = MP_FIXTURE_INVALID_ARGUMENT;
            mp_fixture_complete_control(testing, nonce, command, event_payload_tag,
                                        false, status, before, after);
            completion_sent = true;
            return;
        }
        if (nonce == last_nonce) {
            status =
                testing == NULL ? mp_fixture_last_status : testing->last_status;
            before =
                testing == NULL ? mp_fixture_last_before : testing->last_before;
            after =
                testing == NULL ? mp_fixture_last_after : testing->last_after;
            mp_fixture_complete_control(testing, nonce, command, event_payload_tag,
                                        false, status, before, after);
            completion_sent = true;
            return;
        }

        cache_result = true;
        before = mp_fixture_control_window_number(
            testing, MP_FIXTURE_TEST_CONTROL_PRE_WINDOW_EXCEPTION,
            MPFixtureTestingBeforeWindow);
        mp_fixture_testing_raise_control(
            testing, MP_FIXTURE_TEST_CONTROL_COMMAND_EXCEPTION);
        if (command == MP_FIXTURE_COMMAND_SET_VISUAL_ABSENT ||
            command == MP_FIXTURE_COMMAND_SET_VISUAL_VISIBLE) {
            bool visible = command == MP_FIXTURE_COMMAND_SET_VISUAL_VISIBLE;
            if (nonce > UINT32_MAX) {
                status = MP_FIXTURE_INVALID_ARGUMENT;
            } else if (mp_fixture_window == nil ||
                       !mp_fixture_set_visual_state(
                           mp_fixture_window, visible, (uint32_t)nonce)) {
                status = MP_FIXTURE_PLATFORM_FAILURE;
            }
        } else if (command == MP_FIXTURE_COMMAND_TRANSITION) {
            if (mp_fixture_window == nil) {
                status = MP_FIXTURE_PLATFORM_FAILURE;
            } else {
                bool alternate_fill = !mp_fixture_alternate_fill;
                uint32_t fill =
                    alternate_fill ? mp_fixture_replacement_fill : mp_fixture_fill;
                if (!mp_fixture_apply_fill(mp_fixture_window, fill) ||
                    !mp_fixture_layout_current_visual(mp_fixture_window)) {
                    status = MP_FIXTURE_PLATFORM_FAILURE;
                } else {
                    mp_fixture_alternate_fill = alternate_fill;
                }
            }
        } else if (command == MP_FIXTURE_COMMAND_REPLACE) {
            if (mp_fixture_window == nil || mp_fixture_window_class == Nil ||
                mp_fixture_color_class == Nil || mp_fixture_window_title == nil) {
                status = MP_FIXTURE_PLATFORM_FAILURE;
            } else {
                id<MPFixtureWindow> old_window = mp_fixture_window;
                mp_fixture_remove_visual_views();
                mp_fixture_marker_visible = false;
                mp_fixture_visual_token = 0;
                mp_fixture_placement_saved = false;
                mp_fixture_saved_placement = CGPointMake(0.0, 0.0);
                [old_window close];
                mp_fixture_window = nil;
                id<MPFixtureWindow> replacement =
                    mp_fixture_create_window(mp_fixture_window_class, mp_fixture_window_title,
                                             mp_fixture_replacement_fill, mp_fixture_width,
                                             mp_fixture_height);
                if (replacement == nil) {
                    status = MP_FIXTURE_PLATFORM_FAILURE;
                } else {
                    mp_fixture_window = replacement;
                    if (!mp_fixture_install_visual_views(replacement)) {
                        [replacement close];
                        mp_fixture_window = nil;
                        status = MP_FIXTURE_PLATFORM_FAILURE;
                    } else {
                        mp_fixture_alternate_fill = true;
                    }
                }
            }
        } else if (command == MP_FIXTURE_COMMAND_MINIMIZE) {
            if (mp_fixture_window == nil) {
                status = MP_FIXTURE_PLATFORM_FAILURE;
            } else {
                [mp_fixture_window miniaturize:nil];
            }
        } else if (command == MP_FIXTURE_COMMAND_RESTORE) {
            if (mp_fixture_window == nil) {
                status = MP_FIXTURE_PLATFORM_FAILURE;
            } else {
                [mp_fixture_window deminiaturize:nil];
                [mp_fixture_window orderFrontRegardless];
                if ((mp_fixture_renderer == MP_FIXTURE_RENDERER_OPENGL &&
                     !mp_fixture_apply_fill(mp_fixture_window,
                                            mp_fixture_current_fill())) ||
                    !mp_fixture_layout_current_visual(mp_fixture_window)) {
                    status = MP_FIXTURE_PLATFORM_FAILURE;
                }
            }
        } else if (command == MP_FIXTURE_COMMAND_YIELD_FOREGROUND) {
            if (mp_fixture_prior_application == nil ||
                ![mp_fixture_prior_application activateWithOptions:2u]) {
                status = MP_FIXTURE_PLATFORM_FAILURE;
            }
        } else if (command == MP_FIXTURE_COMMAND_MOVE) {
            if (mp_fixture_window == nil) {
                status = MP_FIXTURE_PLATFORM_FAILURE;
            } else {
                CGRect frame = [mp_fixture_window frame];
                double offset = mp_fixture_moved ? -48.0 : 48.0;
                [mp_fixture_window
                    setFrameOrigin:CGPointMake(frame.origin.x + offset,
                                              frame.origin.y + offset)];
                if ((mp_fixture_renderer == MP_FIXTURE_RENDERER_OPENGL &&
                     !mp_fixture_apply_fill(mp_fixture_window,
                                            mp_fixture_current_fill())) ||
                    !mp_fixture_layout_current_visual(mp_fixture_window)) {
                    status = MP_FIXTURE_PLATFORM_FAILURE;
                } else {
                    mp_fixture_moved = !mp_fixture_moved;
                }
            }
        } else if (command == MP_FIXTURE_COMMAND_RESIZE) {
            if (mp_fixture_window == nil) {
                status = MP_FIXTURE_PLATFORM_FAILURE;
            } else {
                CGSize size = mp_fixture_resized
                                  ? CGSizeMake(mp_fixture_width, mp_fixture_height)
                                  : CGSizeMake(mp_fixture_width + 48.0,
                                               mp_fixture_height + 32.0);
                [mp_fixture_window setContentSize:size];
                if ((mp_fixture_renderer == MP_FIXTURE_RENDERER_OPENGL &&
                     !mp_fixture_apply_fill(mp_fixture_window,
                                            mp_fixture_current_fill())) ||
                    !mp_fixture_layout_current_visual(mp_fixture_window)) {
                    status = MP_FIXTURE_PLATFORM_FAILURE;
                } else {
                    mp_fixture_resized = !mp_fixture_resized;
                }
            }
        } else if (command == MP_FIXTURE_COMMAND_OPEN_AUXILIARY) {
            if (mp_fixture_auxiliary_window != nil) {
                status = MP_FIXTURE_INVALID_ARGUMENT;
            } else {
                mp_fixture_auxiliary_window = mp_fixture_create_auxiliary_window();
                if (mp_fixture_auxiliary_window == nil) {
                    status = MP_FIXTURE_PLATFORM_FAILURE;
                }
            }
        } else if (command == MP_FIXTURE_COMMAND_CLOSE_AUXILIARY) {
            if (mp_fixture_auxiliary_window == nil) {
                status = MP_FIXTURE_INVALID_ARGUMENT;
            } else {
                [mp_fixture_auxiliary_window close];
                mp_fixture_auxiliary_window = nil;
            }
        } else if (command == MP_FIXTURE_COMMAND_CLOSE) {
            if (mp_fixture_window == nil) {
                status = MP_FIXTURE_PLATFORM_FAILURE;
            } else {
                mp_fixture_remove_visual_views();
                mp_fixture_marker_visible = false;
                mp_fixture_visual_token = 0;
                mp_fixture_placement_saved = false;
                mp_fixture_saved_placement = CGPointMake(0.0, 0.0);
                [mp_fixture_window close];
                mp_fixture_window = nil;
            }
        } else if (command == MP_FIXTURE_COMMAND_MOVE_TO_NEXT_DISPLAY) {
            status = mp_fixture_move_to_next_display();
        } else if (command == MP_FIXTURE_COMMAND_RESTORE_PLACEMENT) {
            status = mp_fixture_restore_placement();
        } else if (command == MP_FIXTURE_COMMAND_MOVE_OFFSCREEN) {
            status = mp_fixture_move_offscreen();
        } else if (command == MP_FIXTURE_COMMAND_RESTORE_ONSCREEN) {
            status = mp_fixture_restore_onscreen();
        } else if (command == MP_FIXTURE_COMMAND_RESET_EVENTS) {
            atomic_store_explicit(&mp_fixture_event_payload_tag, event_payload_tag,
                                  memory_order_release);
        } else if (command == MP_FIXTURE_COMMAND_PREPARE_LANGUAGE_FLOW) {
            if (mp_fixture_window == nil) {
                status = MP_FIXTURE_PLATFORM_FAILURE;
            } else {
                mp_fixture_alternate_fill = false;
                if (!mp_fixture_apply_fill(mp_fixture_window, mp_fixture_fill) ||
                    !mp_fixture_layout_current_visual(mp_fixture_window)) {
                    status = MP_FIXTURE_PLATFORM_FAILURE;
                } else {
                    atomic_store_explicit(&mp_fixture_event_payload_tag, event_payload_tag,
                                          memory_order_release);
                }
            }
        } else if (command == MP_FIXTURE_COMMAND_READ_EVENTS) {
            /* The Rust callback owns the bounded process-wide summary. */
        } else if (command == MP_FIXTURE_COMMAND_STOP) {
            should_stop = true;
        } else {
            status = MP_FIXTURE_INVALID_ARGUMENT;
        }

        after = mp_fixture_control_window_number(
            testing, MP_FIXTURE_TEST_CONTROL_FINAL_WINDOW_EXCEPTION,
            MPFixtureTestingAfterWindow);
        mp_fixture_complete_control(testing, nonce, command, event_payload_tag,
                                    cache_result, status, before, after);
        completion_sent = true;
        if (should_stop && status == MP_FIXTURE_OK &&
            (testing != NULL || mp_fixture_application != nil)) {
            mp_fixture_terminate_control(
                testing, MP_FIXTURE_TEST_CONTROL_STOP_TERMINATION_EXCEPTION);
        }
    } @catch (NSException *exception) {
        (void)exception;
        if (completion_owed) {
            if (completion_sent) {
                mp_fixture_fail_closed(testing);
            } else {
                mp_fixture_complete_control(
                    testing, nonce, command, event_payload_tag, cache_result,
                    MP_FIXTURE_NATIVE_EXCEPTION, before, after);
            }
        }
    } @catch (...) {
        if (completion_owed) {
            if (completion_sent) {
                mp_fixture_fail_closed(testing);
            } else {
                mp_fixture_complete_control(
                    testing, nonce, command, event_payload_tag, cache_result,
                    MP_FIXTURE_NATIVE_EXCEPTION, before, after);
            }
        }
    }
}

uint32_t mp_fixture_control(uint32_t version, uint64_t run_nonce,
                            uint64_t nonce, uint32_t command,
                            uint64_t event_payload_tag) {
    bool visual_command =
        command == MP_FIXTURE_COMMAND_SET_VISUAL_ABSENT ||
        command == MP_FIXTURE_COMMAND_SET_VISUAL_VISIBLE;
    if (version != MP_FIXTURE_CONTROL_VERSION || run_nonce == 0 || nonce == 0 ||
        !mp_fixture_valid_command(command) ||
        (visual_command && nonce > UINT32_MAX) ||
        (event_payload_tag != 0 && command != MP_FIXTURE_COMMAND_RESET_EVENTS &&
         command != MP_FIXTURE_COMMAND_PREPARE_LANGUAGE_FLOW)) {
        return MP_FIXTURE_INVALID_ARGUMENT;
    }
    if (!atomic_load_explicit(&mp_fixture_control_active, memory_order_acquire)) {
        return MP_FIXTURE_PLATFORM_FAILURE;
    }
    if (run_nonce !=
        atomic_load_explicit(&mp_fixture_run_nonce, memory_order_relaxed)) {
        return MP_FIXTURE_INVALID_ARGUMENT;
    }

    dispatch_async(dispatch_get_main_queue(), ^{
      mp_fixture_run_control_block(run_nonce, nonce, command, event_payload_tag, NULL);
    });
    return MP_FIXTURE_OK;
}

static void mp_fixture_run_control_closed_block(uint64_t run_nonce,
                                                MPFixtureControlTesting *testing) {
    @try {
        if (testing != NULL ||
            (atomic_load_explicit(&mp_fixture_control_active, memory_order_acquire) &&
             run_nonce ==
                 atomic_load_explicit(&mp_fixture_run_nonce, memory_order_relaxed) &&
             mp_fixture_application != nil)) {
            mp_fixture_terminate_control(
                testing, MP_FIXTURE_TEST_CONTROL_CLOSED_TERMINATION_EXCEPTION);
        }
    } @catch (NSException *exception) {
        (void)exception;
        mp_fixture_fail_closed(testing);
    } @catch (...) {
        mp_fixture_fail_closed(testing);
    }
}

uint32_t mp_fixture_control_closed(uint32_t version, uint64_t run_nonce) {
    if (version != MP_FIXTURE_CONTROL_VERSION || run_nonce == 0) {
        return MP_FIXTURE_INVALID_ARGUMENT;
    }
    if (!atomic_load_explicit(&mp_fixture_control_active, memory_order_acquire)) {
        return MP_FIXTURE_PLATFORM_FAILURE;
    }
    if (run_nonce !=
        atomic_load_explicit(&mp_fixture_run_nonce, memory_order_relaxed)) {
        return MP_FIXTURE_INVALID_ARGUMENT;
    }

    dispatch_async(dispatch_get_main_queue(), ^{
      mp_fixture_run_control_closed_block(run_nonce, NULL);
    });
    return MP_FIXTURE_OK;
}

uint32_t mp_fixture_test_control_containment(
    uint32_t scenario, uint32_t *out_completion_count,
    uint32_t *out_completion_status, uint64_t *out_completion_before,
    uint64_t *out_completion_after, uint32_t *out_cached_status,
    uint64_t *out_cached_before, uint64_t *out_cached_after,
    uint32_t *out_termination_calls, uint32_t *out_fail_closed_calls) {
    if (scenario > MP_FIXTURE_TEST_CONTROL_CLOSED_TERMINATION_EXCEPTION ||
        out_completion_count == NULL || out_completion_status == NULL ||
        out_completion_before == NULL || out_completion_after == NULL ||
        out_cached_status == NULL || out_cached_before == NULL ||
        out_cached_after == NULL || out_termination_calls == NULL ||
        out_fail_closed_calls == NULL) {
        return MP_FIXTURE_INVALID_ARGUMENT;
    }

    MPFixtureControlTesting testing = {.scenario = scenario};
    if (scenario == MP_FIXTURE_TEST_CONTROL_CLOSED_TERMINATION_EXCEPTION) {
        mp_fixture_run_control_closed_block(1, &testing);
    } else {
        uint32_t command =
            scenario == MP_FIXTURE_TEST_CONTROL_STOP_TERMINATION_EXCEPTION
                ? MP_FIXTURE_COMMAND_STOP
                : MP_FIXTURE_COMMAND_READ_EVENTS;
        mp_fixture_run_control_block(1, 1, command, 0, &testing);
    }

    *out_completion_count = testing.completion_count;
    *out_completion_status = testing.completion_status;
    *out_completion_before = testing.completion_before;
    *out_completion_after = testing.completion_after;
    *out_cached_status = testing.last_status;
    *out_cached_before = testing.last_before;
    *out_cached_after = testing.last_after;
    *out_termination_calls = testing.termination_calls;
    *out_fail_closed_calls = testing.fail_closed_calls;
    return MP_FIXTURE_OK;
}


uint32_t mp_fixture_run(const char *title, uint64_t run_nonce, uint32_t fill,
                        uint32_t replacement_fill, uint32_t behavior, uint32_t renderer,
                        uint32_t replacement_delay_ms, double width, double height,
                        uint32_t activate, uint32_t launch_context, uint32_t signature_mode,
                        const uint8_t *signing_identifier, size_t signing_identifier_len,
                        void *context,
                        void (*ready)(void *context, uint64_t window_number,
                                      uint64_t run_nonce, uint32_t renderer,
                                      uint32_t launch_context, uint32_t signature_mode,
                                      const uint8_t *signing_identifier,
                                      size_t signing_identifier_len),
                        void (*replaced)(void *context, uint32_t status,
                                         uint64_t old_window_number,
                                         uint64_t new_window_number),
                        void (*controlled)(void *context, uint64_t nonce,
                                           uint32_t command, uint32_t status,
                                           uint64_t before_window_number,
                                           uint64_t after_window_number),
                        void (*sink)(void *context, uint32_t kind, uint32_t text_units,
                                     uint64_t event_payload_tag,
                                     uint64_t payload_fingerprint)) {
    const uint32_t behavior_mask = MP_FIXTURE_BEHAVIOR_ANIMATE_ON_KEY_DOWN |
                                   MP_FIXTURE_BEHAVIOR_RESIZE_ON_KEY_DOWN |
                                   MP_FIXTURE_BEHAVIOR_TAGGED_INPUT_NO_VISUAL;
    if (title == NULL || run_nonce == 0 || ready == NULL || replaced == NULL ||
        controlled == NULL || sink == NULL ||
        atomic_load_explicit(&mp_fixture_control_active, memory_order_acquire) ||
        !(width >= 64.0) || !(height >= 64.0) || !(width <= 4096.0) ||
        !(height <= 4096.0) || (behavior & ~behavior_mask) != 0u ||
        renderer > MP_FIXTURE_RENDERER_OPENGL || activate > 1u ||
        replacement_delay_ms > 60000u ||
        (signing_identifier_len > 0 && signing_identifier == NULL)) {
        return MP_FIXTURE_INVALID_ARGUMENT;
    }
    mp_fixture_reset_state();
    MP_FIXTURE_BEGIN
    if (!mp_fixture_load_appkit()) {
        return MP_FIXTURE_UNSUPPORTED;
    }
    Class opengl_pixel_format_class = Nil;
    Class opengl_view_class = Nil;
    if (renderer == MP_FIXTURE_RENDERER_OPENGL) {
        if (!mp_fixture_load_opengl()) {
            return MP_FIXTURE_UNSUPPORTED;
        }
        opengl_pixel_format_class = NSClassFromString(@"NSOpenGLPixelFormat");
        opengl_view_class = NSClassFromString(@"NSOpenGLView");
        if (opengl_pixel_format_class == Nil || opengl_view_class == Nil) {
            return MP_FIXTURE_UNSUPPORTED;
        }
    }
    Class application_class = NSClassFromString(@"NSApplication");
    Class workspace_class = NSClassFromString(@"NSWorkspace");
    Class window_class = NSClassFromString(@"NSWindow");
    Class color_class = NSClassFromString(@"NSColor");
    Class box_class = NSClassFromString(@"NSBox");
    Class event_class = NSClassFromString(@"NSEvent");
    Class running_application_class = NSClassFromString(@"NSRunningApplication");
    if (application_class == Nil || workspace_class == Nil ||
        running_application_class == Nil || window_class == Nil ||
        color_class == Nil || box_class == Nil || event_class == Nil) {
        return MP_FIXTURE_UNSUPPORTED;
    }

    NSString *window_title = [NSString stringWithUTF8String:title];
    if (window_title == nil) {
        return MP_FIXTURE_INVALID_ARGUMENT;
    }
    id<MPFixtureWorkspace> workspace =
        [(id<MPFixtureWorkspaceClass>)workspace_class sharedWorkspace];
    id<MPFixtureRunningApplication> prior_application =
        (id<MPFixtureRunningApplication>)workspace.frontmostApplication;
    id<MPFixtureApplication> application =
        [(id<MPFixtureApplicationClass>)application_class sharedApplication];
    if (application == nil) {
        return MP_FIXTURE_PLATFORM_FAILURE;
    }
    (void)[application setActivationPolicy:MPFixtureActivationRegular];
    [application finishLaunching];
    id<MPFixtureRunningApplication> current_application =
        [(id<MPFixtureRunningApplicationClass>)running_application_class currentApplication];
    if (current_application == nil) {
        return MP_FIXTURE_PLATFORM_FAILURE;
    }
    if (activate != 0u) {
        [application activateIgnoringOtherApps:YES];
    }

    mp_fixture_application = application;
    mp_fixture_prior_application = prior_application;
    mp_fixture_current_application = current_application;
    mp_fixture_window_title = window_title;
    mp_fixture_window_class = window_class;
    mp_fixture_color_class = color_class;
    mp_fixture_box_class = box_class;
    mp_fixture_opengl_pixel_format_class = opengl_pixel_format_class;
    mp_fixture_opengl_view_class = opengl_view_class;
    mp_fixture_activate = activate != 0u;
    mp_fixture_renderer = renderer;
    mp_fixture_fill = fill;
    mp_fixture_replacement_fill = replacement_fill;
    mp_fixture_width = width;
    mp_fixture_height = height;
    mp_fixture_control_context = context;
    atomic_store_explicit(&mp_fixture_run_nonce, run_nonce, memory_order_relaxed);
    mp_fixture_controlled = controlled;
    mp_fixture_window =
        mp_fixture_create_window(window_class, window_title, fill, width, height);
    if (mp_fixture_window == nil ||
        !mp_fixture_install_visual_views(mp_fixture_window)) {
        [mp_fixture_window close];
        mp_fixture_window = nil;
        return MP_FIXTURE_PLATFORM_FAILURE;
    }
    if (activate == 0u &&
        (prior_application == nil || ![prior_application activateWithOptions:3u])) {
        return MP_FIXTURE_PLATFORM_FAILURE;
    }

    __block uint32_t visualized_correlation = 0;
    __block bool alternate_size = false;
    id monitor = [(id<MPFixtureEventClass>)event_class
        addLocalMonitorForEventsMatchingMask:MPFixtureEventMaskAny
                                     handler:^id(id event) {
                                       @try {
                                           uint32_t kind = 0;
                                           id<MPFixtureEvent> observed = (id<MPFixtureEvent>)event;
                                           if (mp_fixture_classify(observed.type, &kind)) {
                                               uint32_t units = 0;
                                               uint64_t expected_event_payload_tag =
                                                   atomic_load_explicit(
                                                       &mp_fixture_event_payload_tag,
                                                       memory_order_acquire);
                                               uint64_t observed_event_payload_tag = 0;
                                               uint64_t payload_fingerprint = 0;
                                               CGEventRef native_event = observed.CGEvent;
                                               if (native_event != NULL) {
                                                   observed_event_payload_tag =
                                                       (uint64_t)CGEventGetIntegerValueField(
                                                           native_event,
                                                           kCGEventSourceUserData);
                                                   payload_fingerprint =
                                                       mp_fixture_event_fingerprint(
                                                           kind, native_event, &units);
                                               }
                                               if (expected_event_payload_tag != 0 &&
                                                   (observed_event_payload_tag == 0 ||
                                                    observed_event_payload_tag !=
                                                        expected_event_payload_tag)) {
                                                   return event;
                                               }
                                               uint64_t event_payload_tag =
                                                   expected_event_payload_tag == 0
                                                       ? 0
                                                       : observed_event_payload_tag;
                                               sink(context, kind, units, event_payload_tag,
                                                    payload_fingerprint);
                                               uint32_t correlation =
                                                   (uint32_t)(event_payload_tag >> 32);
                                               bool tagged_visual_event =
                                                   (behavior &
                                                    MP_FIXTURE_BEHAVIOR_TAGGED_INPUT_NO_VISUAL) ==
                                                       0u &&
                                                   correlation != 0u &&
                                                   correlation != visualized_correlation;
                                               bool animates =
                                                   (behavior &
                                                    MP_FIXTURE_BEHAVIOR_ANIMATE_ON_KEY_DOWN) != 0u;
                                               bool resizes =
                                                   (behavior &
                                                    MP_FIXTURE_BEHAVIOR_RESIZE_ON_KEY_DOWN) != 0u;
                                               bool animation_event =
                                                   tagged_visual_event ||
                                                   (animates &&
                                                    kind == MP_FIXTURE_EVENT_KEY_DOWN &&
                                                    (!resizes ||
                                                     units == MPFixtureAnimateTextUnits));
                                               if (animation_event) {
                                                   /*
                                                    * A correlated qualification sequence toggles
                                                    * once per tag. Legacy untagged example flows
                                                    * latch the expected fill so a later key-down
                                                    * cannot erase their visual observation.
                                                    */
                                                   bool next_fill =
                                                       tagged_visual_event
                                                           ? !mp_fixture_alternate_fill
                                                           : true;
                                                   uint32_t benchmark_fill =
                                                       next_fill ? replacement_fill : fill;
                                                   if (mp_fixture_apply_fill(
                                                           mp_fixture_window,
                                                           benchmark_fill) &&
                                                       mp_fixture_layout_current_visual(
                                                           mp_fixture_window)) {
                                                       mp_fixture_alternate_fill = next_fill;
                                                       if (tagged_visual_event) {
                                                           visualized_correlation = correlation;
                                                       }
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
                                                   if ((mp_fixture_renderer ==
                                                            MP_FIXTURE_RENDERER_OPENGL &&
                                                        !mp_fixture_apply_fill(
                                                            mp_fixture_window,
                                                            mp_fixture_current_fill())) ||
                                                       !mp_fixture_layout_current_visual(
                                                           mp_fixture_window)) {
                                                       alternate_size = !alternate_size;
                                                   }
                                               }
                                           }
                                       } @catch (...) {
                                       }
                                       return event;
                                     }];
    if (monitor == nil) {
        mp_fixture_remove_visual_views();
        mp_fixture_window = nil;
        return MP_FIXTURE_PLATFORM_FAILURE;
    }

    atomic_store_explicit(&mp_fixture_control_active, true, memory_order_release);
    ready(context, (uint64_t)[mp_fixture_window windowNumber], run_nonce, renderer,
          launch_context, signature_mode, signing_identifier, signing_identifier_len);

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
              mp_fixture_remove_visual_views();
              mp_fixture_marker_visible = false;
              mp_fixture_visual_token = 0;
              [old_window close];
              mp_fixture_window = nil;

              id<MPFixtureWindow> replacement =
                  mp_fixture_create_window(window_class, window_title,
                                           replacement_fill, width, height);
              if (replacement == nil) {
                  replaced(context, MP_FIXTURE_PLATFORM_FAILURE, old_window_number, 0);
                  return;
              }
              mp_fixture_window = replacement;
              if (!mp_fixture_install_visual_views(replacement)) {
                  [replacement close];
                  mp_fixture_window = nil;
                  replaced(context, MP_FIXTURE_PLATFORM_FAILURE, old_window_number, 0);
                  return;
              }
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
    return MP_FIXTURE_OK;
    MP_FIXTURE_END
}
