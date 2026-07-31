/*
 * MadoPilot macOS native shim.
 *
 * Objective-C with Automatic Reference Counting, compiled with
 * -fobjc-arc-exceptions. Both are requirements of
 * docs/adr/0012-macos-shim-language-and-containment.md rather than preferences:
 * without the exception flag, ARC emits no release on an unwind edge, so an
 * exception raised where a failing stream start would raise one leaves the
 * native object the session had already retained alive.
 *
 * # Why ScreenCaptureKit is not imported
 *
 * Every framework this file imports exists on every macOS version the project
 * could select as its minimum. ScreenCaptureKit does not: it arrived in 12.3, and
 * a translation unit that names its classes or its exported constants makes the
 * binary carry a load command for it, which makes an older host fail to load
 * instead of reporting a status. Linking it weakly would fix that at the final
 * link, but Cargo does not propagate a dependency's `rustc-link-arg` to the
 * binary that consumes the dependency, so a build script here cannot own that
 * flag. This file therefore loads the framework from its absolute system
 * location, resolves its classes by name and its exported attachment keys by
 * symbol, and gates every use behind an availability check. The observable
 * property ADR 0012 asked for is unchanged: an unsupported host reports
 * MP_SHIM_UNSUPPORTED and the library still loads.
 *
 * # Why discovery preflights authorization
 *
 * The framework's own shareable-content query presents the system Screen
 * Recording dialog when the process has no decision yet. MadoPilot presents no
 * permission UI, so every entry point that would reach that query preflights
 * with the non-prompting Core Graphics check and refuses with
 * MP_SHIM_PERMISSION_DENIED instead.
 */

#import <ApplicationServices/ApplicationServices.h>
#import <CoreGraphics/CoreGraphics.h>
#import <CoreMedia/CoreMedia.h>
#import <CoreVideo/CoreVideo.h>
#import <Foundation/Foundation.h>

#include <dlfcn.h>
#include <mach/mach_time.h>
#include <math.h>
#include <pthread.h>
#include <stdatomic.h>
#include <string.h>
#include <time.h>

#include "madopilot_macos_shim.h"

#if !__has_feature(objc_arc)
#error "the MadoPilot macOS shim requires Automatic Reference Counting"
#endif
/*
 * Clang exposes no feature macro for -fobjc-arc-exceptions, so the build script
 * defines this alongside the flag and this check keeps the two in one review.
 * The behavioural guard is separate and stronger: the ownership tests assert
 * that a contained failure leaves no native object alive, which is exactly what
 * stops holding once the flag is dropped.
 */
#ifndef MP_SHIM_ARC_EXCEPTIONS
#error "the MadoPilot macOS shim requires -fobjc-arc-exceptions (ADR 0012)"
#endif

/* Bounds on what a caller may ask for, so a request cannot size the producer. */
#define MP_SHIM_MIN_QUEUE_DEPTH 3u
#define MP_SHIM_MAX_QUEUE_DEPTH 8u

#define MP_SHIM_SESSION_MAGIC 0x4d505353u
#define MP_SHIM_FRAME_MAGIC 0x4d505346u
#define MP_SHIM_INVENTORY_MAGIC 0x4d505349u

/*
 * Every entry point and callback trampoline wraps its body in these. Both
 * branches are load-bearing: @throw accepts any object, so a thrown value that
 * is not an NSException reaches the catch-all rather than escaping.
 */
#define MP_SHIM_BEGIN @try {
#define MP_SHIM_END                                                                                \
    }                                                                                              \
    @catch (NSException * exception) {                                                             \
        (void)exception;                                                                           \
        return MP_SHIM_NATIVE_EXCEPTION;                                                           \
    }                                                                                              \
    @catch (...) {                                                                                 \
        return MP_SHIM_NATIVE_EXCEPTION;                                                           \
    }

static mach_timebase_info_data_t mp_shim_timebase_data;
static pthread_once_t mp_shim_timebase_once = PTHREAD_ONCE_INIT;

static void mp_shim_read_timebase(void) {
    if (mach_timebase_info(&mp_shim_timebase_data) != KERN_SUCCESS) {
        mp_shim_timebase_data.numer = 0;
        mp_shim_timebase_data.denom = 0;
    }
}

static mach_timebase_info_data_t mp_shim_timebase(void) {
    pthread_once(&mp_shim_timebase_once, mp_shim_read_timebase);
    return mp_shim_timebase_data;
}

/* Converts a mach absolute time reading into nanoseconds.
 *
 * Both the shim's own clock reading and the framework's per-frame display time
 * arrive in mach absolute units, which are not nanoseconds on Apple silicon, so
 * they are converted in one place and every timestamp leaving this shim is in
 * nanoseconds. Returns 0 when the timebase is unreadable; each caller reports
 * that as a platform failure or as an absent timestamp. */
static uint64_t mp_shim_nanos_from_ticks(uint64_t ticks) {
    mach_timebase_info_data_t timebase = mp_shim_timebase();
    if (timebase.denom == 0) {
        return 0;
    }
    /* The intermediate product is widened because a nanosecond conversion of a
     * host uptime overflows 64 bits on a timebase whose numerator is large. */
    unsigned __int128 nanos = (unsigned __int128)ticks * timebase.numer / timebase.denom;
    return nanos > UINT64_MAX ? UINT64_MAX : (uint64_t)nanos;
}

/* Count of native objects this shim owns, for the ADR 0012 ownership tests. */
static atomic_ullong mp_shim_owned_objects = 0;

static void mp_shim_note_owned(void) { atomic_fetch_add(&mp_shim_owned_objects, 1u); }

static void mp_shim_note_released(void) { atomic_fetch_sub(&mp_shim_owned_objects, 1u); }

#pragma mark - Selectors the shim sends, declared without a framework header

/*
 * These protocols mirror the selectors this shim sends to ScreenCaptureKit
 * objects. They are named for this shim rather than for the framework on
 * purpose: adopting the framework's own protocol names would define them twice
 * once the framework is loaded, and importing its header would create the link
 * dependency the comment at the top of this file exists to avoid.
 */

@protocol MPShimShareableContentClass <NSObject>
- (void)getShareableContentExcludingDesktopWindows:(BOOL)excludeDesktopWindows
                               onScreenWindowsOnly:(BOOL)onScreenWindowsOnly
                                 completionHandler:(void (^)(id content, NSError *error))handler;
@end

@protocol MPShimShareableContent <NSObject>
@property(readonly, copy) NSArray *windows;
@property(readonly, copy) NSArray *displays;
@end

@protocol MPShimRunningApplication <NSObject>
@property(readonly) pid_t processID;
@property(readonly, copy) NSString *applicationName;
@end

@protocol MPShimWindow <NSObject>
@property(readonly) uint32_t windowID;
@property(readonly) CGRect frame;
@property(readonly, copy) NSString *title;
@property(readonly) id owningApplication;
@property(readonly, getter=isOnScreen) BOOL onScreen;
@property(readonly) NSInteger windowLayer;
@end

@protocol MPShimDisplay <NSObject>
@property(readonly) CGDirectDisplayID displayID;
@property(readonly) CGRect frame;
@end

@protocol MPShimContentFilterInit <NSObject>
- (instancetype)initWithDesktopIndependentWindow:(id)window;
- (instancetype)initWithDisplay:(id)display excludingWindows:(NSArray *)excluded;
@end

@protocol MPShimStreamConfiguration <NSObject>
@property(nonatomic, assign) size_t width;
@property(nonatomic, assign) size_t height;
@property(nonatomic, assign) OSType pixelFormat;
@property(nonatomic, assign) BOOL showsCursor;
@property(nonatomic, assign) NSInteger queueDepth;
@property(nonatomic, assign) BOOL scalesToFit;
@end

@protocol MPShimStream <NSObject>
- (instancetype)initWithFilter:(id)filter configuration:(id)configuration delegate:(id)delegate;
- (BOOL)addStreamOutput:(id)output
                   type:(NSInteger)type
     sampleHandlerQueue:(dispatch_queue_t)queue
                  error:(NSError **)error;
- (BOOL)removeStreamOutput:(id)output type:(NSInteger)type error:(NSError **)error;
- (void)startCaptureWithCompletionHandler:(void (^)(NSError *error))handler;
- (void)stopCaptureWithCompletionHandler:(void (^)(NSError *error))handler;
- (void)updateConfiguration:(id)configuration completionHandler:(void (^)(NSError *error))handler;
@end

/* SCStreamOutputType.screen */
static const NSInteger MPShimStreamOutputTypeScreen = 0;
/* SCFrameStatus.complete */
static const NSInteger MPShimFrameStatusComplete = 0;
/*
 * `SCStreamErrorCode` values, by value for the reason the framework is not
 * imported. Every one is transcribed from `SCError.h` in the SDK this builds
 * against — they must not be recalled from memory, because a wrong value here
 * reports a deliberate stop as a failure or a failure as a lost target, and
 * nothing in the type system catches it. The `mp_shim_error_status` tests pin the
 * mapping so a future edit has to disagree with an assertion rather than with a
 * comment.
 */
static const NSInteger MPShimErrorUserDeclined = -3801;
static const NSInteger MPShimErrorFailedToStart = -3802;
static const NSInteger MPShimErrorAttemptToStartStreamState = -3807;
static const NSInteger MPShimErrorAttemptToStopStreamState = -3808;
static const NSInteger MPShimErrorNoWindowList = -3813;
static const NSInteger MPShimErrorNoDisplayList = -3814;
static const NSInteger MPShimErrorNoCaptureSource = -3815;
static const NSInteger MPShimErrorUserStopped = -3817;
static const NSInteger MPShimErrorSystemStoppedStream = -3821;

#pragma mark - Controlled framework loading

typedef struct MPShimFramework {
    Class shareable_content;
    Class stream;
    Class stream_configuration;
    Class content_filter;
    CFTypeRef key_status;
    CFTypeRef key_display_time;
    CFTypeRef key_scale_factor;
    CFTypeRef key_content_scale;
    CFTypeRef key_content_rect;
    CFTypeRef error_domain;
    bool loaded;
} MPShimFramework;

static MPShimFramework mp_shim_framework;
static pthread_once_t mp_shim_framework_once = PTHREAD_ONCE_INIT;

static CFTypeRef mp_shim_string_symbol(void *handle, const char *name) {
    void *const *slot = dlsym(handle, name);
    return slot == NULL ? NULL : (CFTypeRef)*slot;
}

/*
 * Establishes this process's Core Graphics window-server connection.
 *
 * The capture framework's shareable-content query requires it and does not check:
 * in a process that has made no earlier Core Graphics window or display call, the
 * query fails the `CGS_REQUIRE_INIT` assertion and aborts. An abort is not an
 * exception, so no `@catch` on either side of this boundary can contain it — which
 * makes satisfying the precondition the only available answer, and this shim the
 * place that owns it. `CGMainDisplayID` is what establishes the connection, and it
 * was the smallest call measured to do so; it takes no capability the Adapter does
 * not already use, since frame-time placement reads display bounds anyway.
 */
static void mp_shim_connect_window_server(void) { (void)CGMainDisplayID(); }

static void mp_shim_load_framework(void) {
    mp_shim_connect_window_server();

    /*
     * An absolute system path, never a bare name: a bare name would let the
     * dynamic loader's ambient search decide which library answers, which is the
     * unrestricted search this project's packaging rules reject.
     */
    void *handle = dlopen("/System/Library/Frameworks/ScreenCaptureKit.framework/"
                          "Versions/A/ScreenCaptureKit",
                          RTLD_LAZY | RTLD_LOCAL);
    if (handle == NULL) {
        handle = dlopen("/System/Library/Frameworks/ScreenCaptureKit.framework/ScreenCaptureKit",
                        RTLD_LAZY | RTLD_LOCAL);
    }
    if (handle == NULL) {
        return;
    }

    MPShimFramework loaded;
    memset(&loaded, 0, sizeof(loaded));
    loaded.shareable_content = NSClassFromString(@"SCShareableContent");
    loaded.stream = NSClassFromString(@"SCStream");
    loaded.stream_configuration = NSClassFromString(@"SCStreamConfiguration");
    loaded.content_filter = NSClassFromString(@"SCContentFilter");
    loaded.key_status = mp_shim_string_symbol(handle, "SCStreamFrameInfoStatus");
    loaded.key_display_time = mp_shim_string_symbol(handle, "SCStreamFrameInfoDisplayTime");
    loaded.key_scale_factor = mp_shim_string_symbol(handle, "SCStreamFrameInfoScaleFactor");
    loaded.key_content_scale = mp_shim_string_symbol(handle, "SCStreamFrameInfoContentScale");
    loaded.key_content_rect = mp_shim_string_symbol(handle, "SCStreamFrameInfoContentRect");
    loaded.error_domain = mp_shim_string_symbol(handle, "SCStreamErrorDomain");

    loaded.loaded = loaded.shareable_content != Nil && loaded.stream != Nil &&
                    loaded.stream_configuration != Nil && loaded.content_filter != Nil &&
                    loaded.key_status != NULL && loaded.key_content_rect != NULL &&
                    loaded.key_scale_factor != NULL && loaded.error_domain != NULL;
    if (loaded.loaded) {
        mp_shim_framework = loaded;
    }
}

static const MPShimFramework *mp_shim_capture_framework(void) {
    if (@available(macOS 12.3, *)) {
        pthread_once(&mp_shim_framework_once, mp_shim_load_framework);
        return mp_shim_framework.loaded ? &mp_shim_framework : NULL;
    }
    return NULL;
}

/* Reads the existing Screen Recording decision. The requesting variant, which
 * would present the system dialog, is deliberately never called. */
static bool mp_shim_screen_capture_preflight(void) {
    if (@available(macOS 10.15, *)) {
        return CGPreflightScreenCaptureAccess();
    }
    return false;
}

#pragma mark - Bounded native waits

static mp_shim_status mp_shim_wait(dispatch_semaphore_t semaphore, uint64_t timeout_nanos) {
    if (timeout_nanos == 0) {
        return MP_SHIM_TIMED_OUT;
    }
    int64_t interval = timeout_nanos > (uint64_t)INT64_MAX ? INT64_MAX : (int64_t)timeout_nanos;
    dispatch_time_t deadline = dispatch_time(DISPATCH_TIME_NOW, interval);
    return dispatch_semaphore_wait(semaphore, deadline) == 0 ? MP_SHIM_OK : MP_SHIM_TIMED_OUT;
}

/*
 * Classifies one `SCStreamErrorDomain` code.
 *
 * Split out from the error object so the table can be asserted directly. The
 * domain is checked by the caller: a matching negative code from an unrelated
 * domain is not a stream outcome.
 */
static mp_shim_status mp_shim_stream_error_status(NSInteger code) {
    if (code == MPShimErrorUserDeclined) {
        return MP_SHIM_PERMISSION_DENIED;
    }
    if (code == MPShimErrorUserStopped) {
        /* The user stopped the stream through a system control. Nothing failed,
         * and reporting a failure would send a caller looking for one. */
        return MP_SHIM_STOPPED_BY_USER;
    }
    if (code == MPShimErrorSystemStoppedStream) {
        /* The system ended the stream without naming a cause. The Adapter decides
         * what to report by re-reading authorization rather than by guessing. */
        return MP_SHIM_STOPPED_BY_SYSTEM;
    }
    if (code == MPShimErrorAttemptToStartStreamState ||
        code == MPShimErrorAttemptToStopStreamState) {
        /* Our own call found the stream already in the state it asked for. That is
         * what makes an idempotent close idempotent, not a failure to report. */
        return MP_SHIM_CLOSED;
    }
    if (code == MPShimErrorNoCaptureSource || code == MPShimErrorNoWindowList ||
        code == MPShimErrorNoDisplayList) {
        /* The thing being captured is no longer listable, which is target loss
         * rather than an unexplained platform failure. */
        return MP_SHIM_TARGET_LOST;
    }
    if (code == MPShimErrorFailedToStart) {
        return MP_SHIM_PLATFORM_FAILURE;
    }
    return MP_SHIM_PLATFORM_FAILURE;
}

static mp_shim_status mp_shim_error_status(NSError *error) {
    if (error == nil) {
        return MP_SHIM_PLATFORM_FAILURE;
    }
    const MPShimFramework *framework = mp_shim_capture_framework();
    NSString *domain = framework == NULL ? nil : (__bridge NSString *)framework->error_domain;
    if (domain == nil || ![error.domain isEqualToString:domain]) {
        /* Not this framework's error, so its code space does not apply. */
        return MP_SHIM_PLATFORM_FAILURE;
    }
    return mp_shim_stream_error_status(error.code);
}

#pragma mark - Callback admission

typedef struct MPShimAdmission {
    pthread_mutex_t mutex;
    pthread_cond_t drained;
    bool accepting;
    bool fenced;
    uint32_t active;
} MPShimAdmission;

static void mp_shim_admission_init(MPShimAdmission *admission) {
    pthread_mutex_init(&admission->mutex, NULL);
    pthread_cond_init(&admission->drained, NULL);
    admission->accepting = true;
    admission->fenced = false;
    admission->active = 0;
}

static void mp_shim_admission_destroy(MPShimAdmission *admission) {
    pthread_cond_destroy(&admission->drained);
    pthread_mutex_destroy(&admission->mutex);
}

static bool mp_shim_admission_enter(MPShimAdmission *admission) {
    pthread_mutex_lock(&admission->mutex);
    bool admitted = admission->accepting && !admission->fenced;
    if (admitted) {
        admission->active += 1;
    }
    pthread_mutex_unlock(&admission->mutex);
    return admitted;
}

/*
 * Admits the producer's one terminal report.
 *
 * A second door rather than the one above, because the report's own first act is
 * to stop admitting frames: a gate that `accepting == false` closes would refuse
 * the very callback that closes it, which is how the stop report came to sit
 * outside the fence entirely. `fenced` is still honoured, and that is the whole
 * rule — after a successful fence the caller may have released the state this
 * report would reach, so a stop arriving then is dropped rather than delivered.
 */
static bool mp_shim_admission_enter_final(MPShimAdmission *admission) {
    pthread_mutex_lock(&admission->mutex);
    bool admitted = !admission->fenced;
    if (admitted) {
        admission->active += 1;
    }
    pthread_mutex_unlock(&admission->mutex);
    return admitted;
}

static void mp_shim_admission_leave(MPShimAdmission *admission) {
    pthread_mutex_lock(&admission->mutex);
    if (admission->active > 0) {
        admission->active -= 1;
    }
    pthread_cond_broadcast(&admission->drained);
    pthread_mutex_unlock(&admission->mutex);
}

static void mp_shim_admission_stop(MPShimAdmission *admission) {
    pthread_mutex_lock(&admission->mutex);
    admission->accepting = false;
    pthread_cond_broadcast(&admission->drained);
    pthread_mutex_unlock(&admission->mutex);
}

/*
 * Waits out the caller's budget on the host's monotonic clock.
 *
 * The budget arrives from a monotonic domain, so it must not be turned into a
 * CLOCK_REALTIME deadline: stepping wall time backward would push that deadline
 * further away and a close documented as bounded would overrun by the adjustment. The
 * relative wait is Darwin's, which this shim is anyway, and the remaining time is
 * recomputed each turn so a spurious wakeup cannot restart the whole budget.
 */
static mp_shim_status mp_shim_admission_fence(MPShimAdmission *admission, uint64_t timeout_nanos) {
    uint64_t started = mp_shim_nanos_from_ticks(mach_absolute_time());
    uint64_t deadline = started > UINT64_MAX - timeout_nanos ? UINT64_MAX : started + timeout_nanos;

    pthread_mutex_lock(&admission->mutex);
    admission->accepting = false;
    mp_shim_status status = MP_SHIM_OK;
    while (admission->active > 0) {
        uint64_t now = mp_shim_nanos_from_ticks(mach_absolute_time());
        if (now >= deadline) {
            status = MP_SHIM_TIMED_OUT;
            break;
        }
        uint64_t left = deadline - now;
        struct timespec relative;
        relative.tv_sec = (time_t)(left / 1000000000ull);
        relative.tv_nsec = (long)(left % 1000000000ull);
        if (pthread_cond_timedwait_relative_np(&admission->drained, &admission->mutex,
                                               &relative) != 0) {
            status = admission->active > 0 ? MP_SHIM_TIMED_OUT : MP_SHIM_OK;
            break;
        }
    }
    if (status == MP_SHIM_OK) {
        /* Only now may the caller release the state it registered. */
        admission->fenced = true;
    }
    pthread_mutex_unlock(&admission->mutex);
    return status;
}

#pragma mark - Handles

struct mp_shim_inventory {
    uint32_t magic;
    CFTypeRef entries; /* NSArray<MPShimInventoryEntry *> */
};

/*
 * One capture session.
 *
 * # Lifetime
 *
 * Three parties can dereference this allocation and none of them is its owner in
 * any order the others can predict: a detached frame returning its lease, the
 * stream output object delivering a sample or a stop, and the Rust handle that
 * opened it. So the allocation is counted rather than owned. Every party that can
 * dereference it holds a reference for as long as it can, and
 * `mp_shim_session_abandon` runs when the last of them lets go. Releasing the
 * handle without closing it therefore frees nothing on its own, which is why
 * `mp_shim_session_release` closes first.
 *
 * Close is also what breaks the one ownership cycle here: `output` retains the
 * stream output object, and that object holds a reference to this session for its
 * whole lifetime. A path that released a session handle without closing it would
 * leak both.
 */
struct mp_shim_session {
    uint32_t magic;
    /* One for the Rust handle at open, plus one per party listed above. */
    atomic_uint refs;
    MPShimAdmission admission;

    uint32_t kind;
    uint64_t native_id;
    uint32_t testing_raise_sites;

    /*
     * Native ownership. Each slot is retained exactly once and released by
     * mp_shim_session_close, which is idempotent.
     *
     * `native_mutex` guards the slots themselves rather than the objects in them.
     * Reading one and retaining it are two instructions, and a close landing
     * between them retained a freed object; the mutex makes the pair atomic. It is
     * never held across a release, a framework message, or a callback.
     */
    pthread_mutex_t native_mutex;
    CFTypeRef stream;
    CFTypeRef configuration;
    CFTypeRef filter;
    CFTypeRef output;
    CFTypeRef queue;

    pthread_mutex_t pool_mutex;
    CVPixelBufferPoolRef pool;
    uint32_t pool_width;
    uint32_t pool_height;
    uint32_t detached_budget;
    uint32_t leased;

    void *callback_context;
    mp_shim_status (*frame_callback)(void *, mp_shim_frame *, const mp_shim_frame_info *);
    void (*stopped_callback)(void *, mp_shim_status);

    atomic_bool output_added;
    atomic_bool started;
    atomic_bool closed;
    atomic_bool stop_reported;
};

struct mp_shim_frame {
    uint32_t magic;
    bool owns_buffer;
    CVPixelBufferRef buffer;
    mp_shim_frame_info info;
    /* Counted for the frame's whole life. A detached frame outliving its session
     * still returns its lease through this pointer. */
    struct mp_shim_session *session;
};

@interface MPShimInventoryEntry : NSObject
@property(nonatomic, assign) mp_shim_target_info info;
@property(nonatomic, copy) NSData *name;
@end

@implementation MPShimInventoryEntry
@end

#pragma mark - Session reference count

/*
 * Runs when the last reference goes. Reached only through mp_shim_session_unref,
 * which is the only place this allocation is freed.
 *
 * Every native object the session owned is released by mp_shim_session_close, so
 * what this destroys is the bookkeeping: the synchronization objects, the magic
 * that makes a stale handle detectable, and the allocation.
 */
static void mp_shim_session_abandon(struct mp_shim_session *session) {
    mp_shim_admission_destroy(&session->admission);
    pthread_mutex_destroy(&session->native_mutex);
    pthread_mutex_destroy(&session->pool_mutex);
    session->magic = 0;
    free(session);
}

static void mp_shim_session_retain(struct mp_shim_session *session) {
    atomic_fetch_add(&session->refs, 1u);
}

static void mp_shim_session_unref(struct mp_shim_session *session) {
    if (atomic_fetch_sub(&session->refs, 1u) == 1u) {
        mp_shim_session_abandon(session);
    }
}

/*
 * One counted hold on a session, dropped when ARC releases this object.
 *
 * It exists so that a block can hold a session reference without the code creating
 * the block having to know whether the block was ever accepted. A raise from the
 * message that would have taken the block destroys the stack block and releases this
 * hold with it; an accepted block keeps it until the block itself is released. Either
 * way the reference is dropped exactly once, and ARC is what guarantees that rather
 * than an assumption about the framework.
 *
 * Pairing a bare retain with an unref inside the block cannot do this. The raise path
 * would have to guess whether the block is going to run, and both guesses are wrong in
 * one direction: unref and the block may still run against a freed session, or do not
 * and the reference is stranded for the life of the process.
 */
@interface MPShimSessionHold : NSObject
- (instancetype)initWithSession:(struct mp_shim_session *)session;
@end

@implementation MPShimSessionHold {
    struct mp_shim_session *_session;
}

- (instancetype)initWithSession:(struct mp_shim_session *)session {
    self = [super init];
    if (self != nil) {
        mp_shim_session_retain(session);
        _session = session;
    }
    return self;
}

- (void)dealloc {
    if (_session != NULL) {
        mp_shim_session_unref(_session);
        _session = NULL;
    }
}

@end

#pragma mark - Geometry helpers

static double mp_shim_display_backing_scale(CGDirectDisplayID display) {
    CGDisplayModeRef mode = CGDisplayCopyDisplayMode(display);
    if (mode == NULL) {
        return 1.0;
    }
    size_t points = CGDisplayModeGetWidth(mode);
    size_t pixels = CGDisplayModeGetPixelWidth(mode);
    CGDisplayModeRelease(mode);
    if (points == 0 || pixels == 0) {
        return 1.0;
    }
    return (double)pixels / (double)points;
}

static bool mp_shim_display_is_active(CGDirectDisplayID display) {
    uint32_t count = 0;
    if (CGGetActiveDisplayList(0, NULL, &count) != kCGErrorSuccess || count == 0) {
        return false;
    }
    CGDirectDisplayID *displays = calloc(count, sizeof(CGDirectDisplayID));
    if (displays == NULL) {
        return false;
    }
    bool found = false;
    if (CGGetActiveDisplayList(count, displays, &count) == kCGErrorSuccess) {
        for (uint32_t index = 0; index < count; index += 1) {
            if (displays[index] == display) {
                found = true;
                break;
            }
        }
    }
    free(displays);
    return found;
}

/* Returns the backing scale of the display holding the greater part of `frame`. */
static double mp_shim_scale_for_frame(CGRect frame) {
    uint32_t count = 0;
    CGDirectDisplayID displays[16];
    if (CGGetDisplaysWithRect(frame, 16, displays, &count) != kCGErrorSuccess || count == 0) {
        return mp_shim_display_backing_scale(CGMainDisplayID());
    }
    double best_area = -1.0;
    double best_scale = 1.0;
    for (uint32_t index = 0; index < count; index += 1) {
        CGRect bounds = CGDisplayBounds(displays[index]);
        CGRect shared = CGRectIntersection(bounds, frame);
        double area = CGRectIsNull(shared) ? 0.0 : shared.size.width * shared.size.height;
        if (area > best_area) {
            best_area = area;
            best_scale = mp_shim_display_backing_scale(displays[index]);
        }
    }
    return best_scale;
}

static bool mp_shim_window_frame(CGWindowID window, CGRect *out_frame) {
/*
 * The shareable-content query is asynchronous and would be wrong inside a
 * producer callback, which is where frame-time placement is read. This
 * synchronous Core Graphics query answers the same question. It is soft
 * deprecated in favour of the capture framework, and is kept because the
 * replacement is unavailable on the older hosts this adapter still supports;
 * only window bounds are read, which needs no authorization.
 */
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
    CFArrayRef list = CGWindowListCopyWindowInfo(kCGWindowListOptionIncludingWindow, window);
#pragma clang diagnostic pop
    if (list == NULL) {
        return false;
    }
    bool resolved = false;
    if (CFArrayGetCount(list) > 0) {
        CFDictionaryRef entry = (CFDictionaryRef)CFArrayGetValueAtIndex(list, 0);
        CFDictionaryRef bounds = (CFDictionaryRef)CFDictionaryGetValue(entry, kCGWindowBounds);
        if (bounds != NULL) {
            resolved = CGRectMakeWithDictionaryRepresentation(bounds, out_frame);
        }
    }
    CFRelease(list);
    return resolved;
}

/*
 * Reports whether a surface of this extent is within the byte ceiling.
 *
 * Both axes are needed at once, which is why this is separate from the per-axis
 * check: the product is the quantity that decides what gets allocated, and the axes
 * bound only what the conversions can express. The multiplication is widened because
 * the point of the check is that the 32-bit product overflows.
 */
static bool mp_shim_surface_within_limit(uint32_t width, uint32_t height) {
    uint64_t bytes = (uint64_t)width * (uint64_t)height * 4ull;
    return bytes <= (uint64_t)MP_SHIM_MAX_SURFACE_BYTES;
}

static uint32_t mp_shim_pixels_from_points(double points, double scale) {
    double pixels = points * scale;
    if (!isfinite(pixels) || pixels < 1.0) {
        return 0;
    }
    double rounded = round(pixels);
    if (rounded > (double)MP_SHIM_MAX_PIXEL_EXTENT) {
        return 0;
    }
    return (uint32_t)rounded;
}

#pragma mark - Version, availability, and authorization

uint32_t mp_shim_abi_version(void) { return MP_SHIM_ABI_VERSION; }

mp_shim_status mp_shim_struct_sizes(uint32_t *out_target_info, uint32_t *out_frame_info,
                                   uint32_t *out_open_request) {
    if (out_target_info == NULL || out_frame_info == NULL || out_open_request == NULL) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    *out_target_info = (uint32_t)sizeof(mp_shim_target_info);
    *out_frame_info = (uint32_t)sizeof(mp_shim_frame_info);
    *out_open_request = (uint32_t)sizeof(mp_shim_open_request);
    return MP_SHIM_OK;
}

mp_shim_status mp_shim_capture_available(void) {
    MP_SHIM_BEGIN
    return mp_shim_capture_framework() == NULL ? MP_SHIM_UNSUPPORTED : MP_SHIM_OK;
    MP_SHIM_END
}

mp_shim_status mp_shim_probe_screen_capture(uint32_t *out_state) {
    if (out_state == NULL) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    MP_SHIM_BEGIN
    if (@available(macOS 10.15, *)) {
        *out_state = mp_shim_screen_capture_preflight() ? MP_SHIM_PERMISSION_GRANTED
                                                       : MP_SHIM_PERMISSION_NOT_GRANTED;
    } else {
        *out_state = MP_SHIM_PERMISSION_UNAVAILABLE;
    }
    return MP_SHIM_OK;
    MP_SHIM_END
}

mp_shim_status mp_shim_probe_accessibility(uint32_t *out_state) {
    if (out_state == NULL) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    MP_SHIM_BEGIN
    /* The options-taking variant can be asked to prompt. This one cannot. */
    *out_state = AXIsProcessTrusted() ? MP_SHIM_PERMISSION_GRANTED : MP_SHIM_PERMISSION_NOT_GRANTED;
    return MP_SHIM_OK;
    MP_SHIM_END
}

mp_shim_status mp_shim_launch_context(uint32_t *out_context) {
    if (out_context == NULL) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    MP_SHIM_BEGIN
    NSString *identifier = [NSBundle mainBundle].bundleIdentifier;
    *out_context = identifier.length > 0 ? MP_SHIM_CONTEXT_BUNDLED : MP_SHIM_CONTEXT_UNBUNDLED;
    return MP_SHIM_OK;
    MP_SHIM_END
}

mp_shim_status mp_shim_classify_stream_error(int64_t code) {
    return mp_shim_stream_error_status((NSInteger)code);
}

mp_shim_status mp_shim_monotonic_nanos(uint64_t *out_nanos) {
    if (out_nanos == NULL) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    if (mp_shim_timebase().denom == 0) {
        return MP_SHIM_PLATFORM_FAILURE;
    }
    *out_nanos = mp_shim_nanos_from_ticks(mach_absolute_time());
    return MP_SHIM_OK;
}

mp_shim_status mp_shim_live_objects(uint64_t *out_live) {
    if (out_live == NULL) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    *out_live = (uint64_t)atomic_load(&mp_shim_owned_objects);
    return MP_SHIM_OK;
}

#pragma mark - Inventory

static MPShimInventoryEntry *mp_shim_window_entry(id<MPShimWindow> window) {
    CGRect frame = window.frame;
    if (CGRectIsNull(frame) || frame.size.width < 1.0 || frame.size.height < 1.0) {
        return nil;
    }
    double scale = mp_shim_scale_for_frame(frame);
    uint32_t pixel_width = mp_shim_pixels_from_points(frame.size.width, scale);
    uint32_t pixel_height = mp_shim_pixels_from_points(frame.size.height, scale);
    if (pixel_width == 0 || pixel_height == 0 ||
        !mp_shim_surface_within_limit(pixel_width, pixel_height)) {
        /* Refused at discovery rather than at open: a target this size cannot be
         * captured, and listing it would offer the caller something that fails. */
        return nil;
    }

    id owner = window.owningApplication;
    NSString *title = window.title;
    NSString *name = title.length > 0 ? title : nil;
    if (name == nil && owner != nil) {
        name = ((id<MPShimRunningApplication>)owner).applicationName;
    }
    NSData *encoded = [(name == nil ? @"" : name) dataUsingEncoding:NSUTF8StringEncoding];

    mp_shim_target_info info;
    memset(&info, 0, sizeof(info));
    info.struct_size = (uint32_t)sizeof(info);
    info.kind = MP_SHIM_TARGET_WINDOW;
    info.native_id = window.windowID;
    info.owner_process = owner == nil ? 0 : ((id<MPShimRunningApplication>)owner).processID;
    info.pixel_width = pixel_width;
    info.pixel_height = pixel_height;
    info.logical_x = frame.origin.x;
    info.logical_y = frame.origin.y;
    info.logical_width = frame.size.width;
    info.logical_height = frame.size.height;
    info.backing_scale = scale;
    info.name_len = (uint32_t)encoded.length;

    MPShimInventoryEntry *entry = [MPShimInventoryEntry new];
    entry.info = info;
    entry.name = encoded;
    return entry;
}

static MPShimInventoryEntry *mp_shim_display_entry(id<MPShimDisplay> display) {
    CGDirectDisplayID identifier = display.displayID;
    CGRect frame = CGDisplayBounds(identifier);
    if (CGRectIsNull(frame) || frame.size.width < 1.0 || frame.size.height < 1.0) {
        return nil;
    }
    double scale = mp_shim_display_backing_scale(identifier);
    uint32_t pixel_width = mp_shim_pixels_from_points(frame.size.width, scale);
    uint32_t pixel_height = mp_shim_pixels_from_points(frame.size.height, scale);
    if (pixel_width == 0 || pixel_height == 0 ||
        !mp_shim_surface_within_limit(pixel_width, pixel_height)) {
        /* Refused at discovery rather than at open: a target this size cannot be
         * captured, and listing it would offer the caller something that fails. */
        return nil;
    }

    NSData *encoded = [[NSString stringWithFormat:@"Display %u", identifier]
        dataUsingEncoding:NSUTF8StringEncoding];

    mp_shim_target_info info;
    memset(&info, 0, sizeof(info));
    info.struct_size = (uint32_t)sizeof(info);
    info.kind = MP_SHIM_TARGET_DISPLAY;
    info.native_id = identifier;
    info.owner_process = 0;
    info.pixel_width = pixel_width;
    info.pixel_height = pixel_height;
    info.logical_x = frame.origin.x;
    info.logical_y = frame.origin.y;
    info.logical_width = frame.size.width;
    info.logical_height = frame.size.height;
    info.backing_scale = scale;
    info.name_len = (uint32_t)encoded.length;

    MPShimInventoryEntry *entry = [MPShimInventoryEntry new];
    entry.info = info;
    entry.name = encoded;
    return entry;
}

/* Performs the one native asynchronous query, bounded by the caller's budget. */
static id mp_shim_shareable_content(const MPShimFramework *framework, uint64_t timeout_nanos,
                                    mp_shim_status *out_status) {
    __block id content = nil;
    __block NSError *failure = nil;
    dispatch_semaphore_t ready = dispatch_semaphore_create(0);
    id<MPShimShareableContentClass> shareable =
        (id<MPShimShareableContentClass>)framework->shareable_content;
    [shareable getShareableContentExcludingDesktopWindows:YES
                                     onScreenWindowsOnly:YES
                                       completionHandler:^(id result, NSError *error) {
                                         content = result;
                                         failure = error;
                                         dispatch_semaphore_signal(ready);
                                       }];
    mp_shim_status waited = mp_shim_wait(ready, timeout_nanos);
    if (waited != MP_SHIM_OK) {
        *out_status = waited;
        return nil;
    }
    if (content == nil) {
        *out_status = mp_shim_error_status(failure);
        return nil;
    }
    *out_status = MP_SHIM_OK;
    return content;
}

mp_shim_status mp_shim_inventory_acquire(uint64_t timeout_nanos, mp_shim_inventory **out) {
    if (out == NULL) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    *out = NULL;
    MP_SHIM_BEGIN
    const MPShimFramework *framework = mp_shim_capture_framework();
    if (framework == NULL) {
        return MP_SHIM_UNSUPPORTED;
    }
    if (!mp_shim_screen_capture_preflight()) {
        /* The framework query would present the system dialog here. */
        return MP_SHIM_PERMISSION_DENIED;
    }

    mp_shim_status queried = MP_SHIM_PLATFORM_FAILURE;
    id content = mp_shim_shareable_content(framework, timeout_nanos, &queried);
    if (content == nil) {
        return queried;
    }

    NSMutableArray<MPShimInventoryEntry *> *entries = [NSMutableArray array];
    id<MPShimShareableContent> shareable_content = (id<MPShimShareableContent>)content;
    for (id window in shareable_content.windows) {
        @autoreleasepool {
            id<MPShimWindow> typed = (id<MPShimWindow>)window;
            if (!typed.onScreen || typed.windowLayer != 0) {
                continue;
            }
            MPShimInventoryEntry *entry = mp_shim_window_entry(typed);
            if (entry != nil) {
                [entries addObject:entry];
            }
        }
    }
    for (id display in shareable_content.displays) {
        @autoreleasepool {
            MPShimInventoryEntry *entry = mp_shim_display_entry((id<MPShimDisplay>)display);
            if (entry != nil) {
                [entries addObject:entry];
            }
        }
    }

    struct mp_shim_inventory *inventory = calloc(1, sizeof(struct mp_shim_inventory));
    if (inventory == NULL) {
        return MP_SHIM_PLATFORM_FAILURE;
    }
    inventory->magic = MP_SHIM_INVENTORY_MAGIC;
    inventory->entries = CFBridgingRetain([entries copy]);
    mp_shim_note_owned();
    *out = inventory;
    return MP_SHIM_OK;
    MP_SHIM_END
}

static MPShimInventoryEntry *mp_shim_inventory_at(const mp_shim_inventory *inventory,
                                                  size_t index) {
    if (inventory == NULL || inventory->magic != MP_SHIM_INVENTORY_MAGIC ||
        inventory->entries == NULL) {
        return nil;
    }
    NSArray *entries = (__bridge NSArray *)inventory->entries;
    if (index >= entries.count) {
        return nil;
    }
    return entries[index];
}

mp_shim_status mp_shim_inventory_count(const mp_shim_inventory *inventory, size_t *out_count) {
    if (out_count == NULL || inventory == NULL || inventory->magic != MP_SHIM_INVENTORY_MAGIC ||
        inventory->entries == NULL) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    MP_SHIM_BEGIN
    *out_count = ((__bridge NSArray *)inventory->entries).count;
    return MP_SHIM_OK;
    MP_SHIM_END
}

mp_shim_status mp_shim_inventory_entry(const mp_shim_inventory *inventory, size_t index,
                                       mp_shim_target_info *out_info) {
    if (out_info == NULL || out_info->struct_size < sizeof(mp_shim_target_info)) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    MP_SHIM_BEGIN
    MPShimInventoryEntry *entry = mp_shim_inventory_at(inventory, index);
    if (entry == nil) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    mp_shim_target_info info = entry.info;
    info.struct_size = (uint32_t)sizeof(mp_shim_target_info);
    memcpy(out_info, &info, sizeof(mp_shim_target_info));
    return MP_SHIM_OK;
    MP_SHIM_END
}

mp_shim_status mp_shim_inventory_name(const mp_shim_inventory *inventory, size_t index,
                                      const uint8_t **out_bytes, size_t *out_len) {
    if (out_bytes == NULL || out_len == NULL) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    MP_SHIM_BEGIN
    MPShimInventoryEntry *entry = mp_shim_inventory_at(inventory, index);
    if (entry == nil) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    NSData *name = entry.name;
    *out_bytes = name.length == 0 ? (const uint8_t *)"" : (const uint8_t *)name.bytes;
    *out_len = name.length;
    return MP_SHIM_OK;
    MP_SHIM_END
}

void mp_shim_inventory_release(mp_shim_inventory *inventory) {
    if (inventory == NULL || inventory->magic != MP_SHIM_INVENTORY_MAGIC) {
        return;
    }
    @try {
        if (inventory->entries != NULL) {
            CFRelease(inventory->entries);
            inventory->entries = NULL;
            mp_shim_note_released();
        }
        inventory->magic = 0;
    } @catch (NSException *exception) {
        (void)exception;
    } @catch (...) {
    }
    free(inventory);
}

mp_shim_status mp_shim_current_placement(uint32_t kind, uint64_t native_id, double *out_frame,
                                        double *out_scale) {
    if (out_frame == NULL || out_scale == NULL) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    MP_SHIM_BEGIN
    CGRect frame = CGRectNull;
    double scale = 1.0;
    if (kind == MP_SHIM_TARGET_WINDOW) {
        if (native_id > UINT32_MAX || !mp_shim_window_frame((CGWindowID)native_id, &frame)) {
            return MP_SHIM_TARGET_LOST;
        }
        scale = mp_shim_scale_for_frame(frame);
    } else if (kind == MP_SHIM_TARGET_DISPLAY) {
        if (native_id > UINT32_MAX || !mp_shim_display_is_active((CGDirectDisplayID)native_id)) {
            return MP_SHIM_TARGET_LOST;
        }
        frame = CGDisplayBounds((CGDirectDisplayID)native_id);
        scale = mp_shim_display_backing_scale((CGDirectDisplayID)native_id);
    } else {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    if (CGRectIsNull(frame) || frame.size.width < 1.0 || frame.size.height < 1.0) {
        return MP_SHIM_TARGET_LOST;
    }
    out_frame[0] = frame.origin.x;
    out_frame[1] = frame.origin.y;
    out_frame[2] = frame.size.width;
    out_frame[3] = frame.size.height;
    *out_scale = scale;
    return MP_SHIM_OK;
    MP_SHIM_END
}

#pragma mark - Detached buffer pool

static void mp_shim_pool_release_locked(struct mp_shim_session *session) {
    if (session->pool != NULL) {
        CVPixelBufferPoolRelease(session->pool);
        session->pool = NULL;
        session->pool_width = 0;
        session->pool_height = 0;
        mp_shim_note_released();
    }
}

static mp_shim_status mp_shim_pool_create_locked(struct mp_shim_session *session, uint32_t width,
                                                uint32_t height) {
    mp_shim_pool_release_locked(session);
    NSDictionary *pixel_attributes = @{
        (__bridge NSString *)kCVPixelBufferPixelFormatTypeKey : @(kCVPixelFormatType_32BGRA),
        (__bridge NSString *)kCVPixelBufferWidthKey : @(width),
        (__bridge NSString *)kCVPixelBufferHeightKey : @(height),
        (__bridge NSString *)kCVPixelBufferBytesPerRowAlignmentKey : @(16),
    };
    NSDictionary *pool_attributes =
        @{(__bridge NSString *)kCVPixelBufferPoolMinimumBufferCountKey : @(0)};
    CVPixelBufferPoolRef pool = NULL;
    CVReturn created =
        CVPixelBufferPoolCreate(kCFAllocatorDefault, (__bridge CFDictionaryRef)pool_attributes,
                                (__bridge CFDictionaryRef)pixel_attributes, &pool);
    if (created != kCVReturnSuccess || pool == NULL) {
        return MP_SHIM_PLATFORM_FAILURE;
    }
    session->pool = pool;
    session->pool_width = width;
    session->pool_height = height;
    mp_shim_note_owned();
    return MP_SHIM_OK;
}

/*
 * Acquires without waiting. MP_SHIM_BUDGET_EXHAUSTED is the finite-pressure
 * outcome a producer callback must be able to observe without blocking, and the
 * lock is tried rather than taken for the same reason.
 */
static mp_shim_status mp_shim_pool_acquire(struct mp_shim_session *session, uint32_t width,
                                           uint32_t height, CVPixelBufferRef *out_buffer) {
    *out_buffer = NULL;
    /*
     * The allocation site bounds its own request. Every extent reaching here is derived
     * from framework metadata, and the ceilings applied at open and reconfiguration
     * bound what this Adapter *asked* for rather than what it was handed — so a
     * detached copy sized from a frame is not covered by either of them.
     */
    if (width == 0 || height == 0 || width > MP_SHIM_MAX_PIXEL_EXTENT ||
        height > MP_SHIM_MAX_PIXEL_EXTENT || !mp_shim_surface_within_limit(width, height)) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    if (pthread_mutex_trylock(&session->pool_mutex) != 0) {
        return MP_SHIM_BUDGET_EXHAUSTED;
    }
    mp_shim_status status = MP_SHIM_OK;
    /*
     * The unlock is in @finally because the body allocates Objective-C objects, and
     * Foundation raises under memory pressure. An exception unwinding past a plain
     * unlock statement leaves this mutex held for the life of the process: the next
     * mp_shim_pool_return blocks on it forever, and close destroys a locked mutex.
     */
    @try {
        if (session->leased >= session->detached_budget) {
            status = MP_SHIM_BUDGET_EXHAUSTED;
        } else if (session->pool == NULL || session->pool_width != width ||
                   session->pool_height != height) {
            status = mp_shim_pool_create_locked(session, width, height);
        }
        if (status == MP_SHIM_OK) {
            NSDictionary *auxiliary = @{
                (__bridge NSString *)
                kCVPixelBufferPoolAllocationThresholdKey : @(session->detached_budget)
            };
            CVPixelBufferRef buffer = NULL;
            CVReturn created = CVPixelBufferPoolCreatePixelBufferWithAuxAttributes(
                kCFAllocatorDefault, session->pool, (__bridge CFDictionaryRef)auxiliary, &buffer);
            if (created == kCVReturnWouldExceedAllocationThreshold) {
                status = MP_SHIM_BUDGET_EXHAUSTED;
            } else if (created != kCVReturnSuccess || buffer == NULL) {
                status = MP_SHIM_PLATFORM_FAILURE;
            } else {
                session->leased += 1;
                mp_shim_note_owned();
                *out_buffer = buffer;
            }
        }
    } @finally {
        pthread_mutex_unlock(&session->pool_mutex);
    }
    return status;
}

static void mp_shim_pool_return(struct mp_shim_session *session, CVPixelBufferRef buffer) {
    CVPixelBufferRelease(buffer);
    mp_shim_note_released();
    pthread_mutex_lock(&session->pool_mutex);
    if (session->leased > 0) {
        session->leased -= 1;
    }
    pthread_mutex_unlock(&session->pool_mutex);
}

#pragma mark - Frame copies

static void mp_shim_copy_rows(const uint8_t *source, size_t source_stride, uint8_t *destination,
                              size_t destination_stride, size_t row_bytes, size_t rows) {
    for (size_t row = 0; row < rows; row += 1) {
        memcpy(destination + row * destination_stride, source + row * source_stride, row_bytes);
    }
}

mp_shim_status mp_shim_frame_detach(mp_shim_frame *borrowed, mp_shim_frame **out) {
    if (out == NULL) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    *out = NULL;
    if (borrowed == NULL || borrowed->magic != MP_SHIM_FRAME_MAGIC || borrowed->owns_buffer ||
        borrowed->session == NULL || borrowed->buffer == NULL) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    MP_SHIM_BEGIN
    struct mp_shim_session *session = borrowed->session;
    uint32_t width = borrowed->info.content_width;
    uint32_t height = borrowed->info.content_height;
    CVPixelBufferRef destination = NULL;
    mp_shim_status acquired = mp_shim_pool_acquire(session, width, height, &destination);
    if (acquired != MP_SHIM_OK) {
        return acquired;
    }

    mp_shim_status status = MP_SHIM_OK;
    CVPixelBufferRef source = borrowed->buffer;
    if (CVPixelBufferLockBaseAddress(source, kCVPixelBufferLock_ReadOnly) != kCVReturnSuccess) {
        status = MP_SHIM_FRAME_INCOMPLETE;
    } else {
        @try {
            if (CVPixelBufferLockBaseAddress(destination, 0) != kCVReturnSuccess) {
                status = MP_SHIM_PLATFORM_FAILURE;
            } else {
                @try {
                    size_t source_stride = CVPixelBufferGetBytesPerRow(source);
                    size_t destination_stride = CVPixelBufferGetBytesPerRow(destination);
                    size_t row_bytes = (size_t)width * 4;
                    size_t origin_x = (size_t)borrowed->info.content_origin_x;
                    size_t origin_y = (size_t)borrowed->info.content_origin_y;
                    const uint8_t *source_base = CVPixelBufferGetBaseAddress(source);
                    uint8_t *destination_base = CVPixelBufferGetBaseAddress(destination);
                    size_t source_height = CVPixelBufferGetHeight(source);
                    if (source_base == NULL || destination_base == NULL ||
                        destination_stride < row_bytes || origin_y + height > source_height ||
                        (origin_x + width) * 4 > source_stride) {
                        status = MP_SHIM_FRAME_INCOMPLETE;
                    } else {
                        mp_shim_copy_rows(source_base + origin_y * source_stride + origin_x * 4,
                                          source_stride, destination_base, destination_stride,
                                          row_bytes, height);
                    }
                } @finally {
                    CVPixelBufferUnlockBaseAddress(destination, 0);
                }
            }
        } @finally {
            CVPixelBufferUnlockBaseAddress(source, kCVPixelBufferLock_ReadOnly);
        }
    }

    if (status != MP_SHIM_OK) {
        mp_shim_pool_return(session, destination);
        return status;
    }

    struct mp_shim_frame *frame = calloc(1, sizeof(struct mp_shim_frame));
    if (frame == NULL) {
        mp_shim_pool_return(session, destination);
        return MP_SHIM_PLATFORM_FAILURE;
    }
    frame->magic = MP_SHIM_FRAME_MAGIC;
    frame->owns_buffer = true;
    frame->buffer = destination;
    frame->info = borrowed->info;
    /* The detached copy holds the content at its own origin. */
    frame->info.content_origin_x = 0.0;
    frame->info.content_origin_y = 0.0;
    frame->info.surface_width = width;
    frame->info.surface_height = height;
    /* The lease outlives the caller's session handle if the caller keeps the
     * frame, and returning it reads the session, so the frame keeps it alive. */
    mp_shim_session_retain(session);
    frame->session = session;
    *out = frame;
    return MP_SHIM_OK;
    MP_SHIM_END
}

void mp_shim_frame_release(mp_shim_frame *frame) {
    if (frame == NULL || frame->magic != MP_SHIM_FRAME_MAGIC || !frame->owns_buffer) {
        return;
    }
    struct mp_shim_session *session = frame->session;
    @try {
        mp_shim_pool_return(session, frame->buffer);
    } @catch (NSException *exception) {
        (void)exception;
    } @catch (...) {
    }
    frame->buffer = NULL;
    frame->session = NULL;
    frame->magic = 0;
    free(frame);
    /* Last, because the line above is the final read of session state and this may
     * be the reference the session was waiting on. */
    mp_shim_session_unref(session);
}

mp_shim_status mp_shim_frame_copy_out(const mp_shim_frame *frame, uint8_t *destination,
                                      size_t capacity, uint64_t destination_stride) {
    if (frame == NULL || frame->magic != MP_SHIM_FRAME_MAGIC || destination == NULL) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    size_t rows = frame->info.content_height;
    size_t row_bytes = (size_t)frame->info.content_width * 4;
    if (rows == 0 || row_bytes == 0 || destination_stride < (uint64_t)row_bytes) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    size_t stride = (size_t)destination_stride;
    if ((uint64_t)stride != destination_stride || stride > SIZE_MAX / rows ||
        stride * rows > capacity) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    MP_SHIM_BEGIN
    if (CVPixelBufferLockBaseAddress(frame->buffer, kCVPixelBufferLock_ReadOnly) !=
        kCVReturnSuccess) {
        return MP_SHIM_PLATFORM_FAILURE;
    }
    mp_shim_status status = MP_SHIM_OK;
    @try {
        size_t source_stride = CVPixelBufferGetBytesPerRow(frame->buffer);
        const uint8_t *source = CVPixelBufferGetBaseAddress(frame->buffer);
        if (source == NULL || source_stride < row_bytes ||
            rows > CVPixelBufferGetHeight(frame->buffer)) {
            status = MP_SHIM_PLATFORM_FAILURE;
        } else {
            mp_shim_copy_rows(source, source_stride, destination, stride, row_bytes, rows);
        }
    } @finally {
        /* The unlock is owed even when the copy raised, so it happens here. */
        CVPixelBufferUnlockBaseAddress(frame->buffer, kCVPixelBufferLock_ReadOnly);
    }
    return status;
    MP_SHIM_END
}

#pragma mark - Stream output

/*
 * The stream's output and its delegate, which are one object here.
 *
 * It holds a counted reference to its session from the moment it adopts one until
 * it is deallocated, and nothing ever clears the pointer. That is deliberate, and
 * the obvious alternative is what this replaces: close used to null the pointer
 * after the fence, which protects nothing, because a callback that has already
 * loaded it into a local still holds the address. Keeping the reference makes the
 * property structural instead — while this object is alive, its session is — so
 * the callbacks below dereference it without further synchronization, and what
 * stops a late callback from doing work is admission, which is what admission is
 * for.
 */
@interface MPShimStreamOutput : NSObject
- (void)adoptSession:(struct mp_shim_session *)session;
@end

@implementation MPShimStreamOutput {
    struct mp_shim_session *_session;
}

- (void)adoptSession:(struct mp_shim_session *)session {
    mp_shim_session_retain(session);
    _session = session;
}

- (void)dealloc {
    if (_session != NULL) {
        mp_shim_session_unref(_session);
        _session = NULL;
    }
}

- (void)stream:(id)stream
    didOutputSampleBuffer:(CMSampleBufferRef)sampleBuffer
                   ofType:(NSInteger)type {
    (void)stream;
    if (type != MPShimStreamOutputTypeScreen) {
        return;
    }
    struct mp_shim_session *session = _session;
    if (session == NULL || session->magic != MP_SHIM_SESSION_MAGIC) {
        return;
    }
    if (!mp_shim_admission_enter(&session->admission)) {
        return;
    }
    /* Each work item pools its own temporaries: without this the pool does not
     * drain between items and the live temporary count grows with the run. */
    @autoreleasepool {
        @try {
            [self deliver:sampleBuffer session:session];
        } @catch (NSException *exception) {
            (void)exception;
        } @catch (...) {
        } @finally {
            /* Decremented here so a thrown exception cannot strand the fence. */
            mp_shim_admission_leave(&session->admission);
        }
    }
}

- (void)deliver:(CMSampleBufferRef)sampleBuffer session:(struct mp_shim_session *)session {
    const MPShimFramework *framework = mp_shim_capture_framework();
    if (framework == NULL || sampleBuffer == NULL || !CMSampleBufferIsValid(sampleBuffer)) {
        return;
    }
    CFArrayRef attachments = CMSampleBufferGetSampleAttachmentsArray(sampleBuffer, false);
    if (attachments == NULL || CFArrayGetCount(attachments) == 0) {
        return;
    }
    NSDictionary *attachment =
        (__bridge NSDictionary *)(CFDictionaryRef)CFArrayGetValueAtIndex(attachments, 0);
    NSNumber *status = attachment[(__bridge NSString *)framework->key_status];
    if (status == nil || status.integerValue != MPShimFrameStatusComplete) {
        return;
    }

    CVImageBufferRef image = CMSampleBufferGetImageBuffer(sampleBuffer);
    if (image == NULL || CVPixelBufferGetPixelFormatType(image) != kCVPixelFormatType_32BGRA) {
        return;
    }

    CGRect content = CGRectNull;
    NSDictionary *rect = attachment[(__bridge NSString *)framework->key_content_rect];
    if (rect == nil ||
        !CGRectMakeWithDictionaryRepresentation((__bridge CFDictionaryRef)rect, &content)) {
        return;
    }
    NSNumber *scale_factor = attachment[(__bridge NSString *)framework->key_scale_factor];
    NSNumber *content_scale = framework->key_content_scale == NULL
                                  ? nil
                                  : attachment[(__bridge NSString *)framework->key_content_scale];
    double factor = scale_factor == nil ? 1.0 : scale_factor.doubleValue;
    double scale = content_scale == nil ? 1.0 : content_scale.doubleValue;
    if (!isfinite(factor) || factor <= 0.0 || !isfinite(scale) || scale <= 0.0) {
        return;
    }

    size_t surface_width = CVPixelBufferGetWidth(image);
    size_t surface_height = CVPixelBufferGetHeight(image);
    /*
     * The surface is framework metadata like everything else validated here, and it is
     * what the content extent is checked against below — so leaving it unbounded left
     * the bound on the content meaningless. Reported rather than assumed, and a frame
     * that fails it is dropped like any other implausible one.
     */
    if (surface_width > MP_SHIM_MAX_PIXEL_EXTENT || surface_height > MP_SHIM_MAX_PIXEL_EXTENT ||
        !mp_shim_surface_within_limit((uint32_t)surface_width, (uint32_t)surface_height)) {
        return;
    }
    double pixel_width = content.size.width * factor;
    double pixel_height = content.size.height * factor;
    /*
     * Every bound is checked as a double, before any conversion. A double outside the
     * destination's range converts with undefined behaviour, and NaN defeats an
     * ordering test rather than failing it — `origin_x < 0.0` is false for NaN, so a
     * malformed origin used to reach `(size_t)origin_x` unchecked. The framework is
     * the source of these values, which makes them untrusted input like any other.
     */
    if (!isfinite(pixel_width) || !isfinite(pixel_height) || pixel_width < 1.0 ||
        pixel_height < 1.0 || pixel_width > (double)MP_SHIM_MAX_PIXEL_EXTENT ||
        pixel_height > (double)MP_SHIM_MAX_PIXEL_EXTENT) {
        return;
    }
    double origin_x = floor(content.origin.x * factor);
    double origin_y = floor(content.origin.y * factor);
    if (!isfinite(origin_x) || !isfinite(origin_y) || origin_x < 0.0 || origin_y < 0.0 ||
        origin_x > (double)MP_SHIM_MAX_PIXEL_EXTENT || origin_y > (double)MP_SHIM_MAX_PIXEL_EXTENT) {
        return;
    }
    uint32_t content_width = (uint32_t)floor(pixel_width);
    uint32_t content_height = (uint32_t)floor(pixel_height);
    if ((size_t)origin_x + content_width > surface_width ||
        (size_t)origin_y + content_height > surface_height) {
        return;
    }

    uint64_t display_time = 0;
    NSNumber *native_time = framework->key_display_time == NULL
                                ? nil
                                : attachment[(__bridge NSString *)framework->key_display_time];
    if (native_time != nil) {
        /* The framework reports this in mach absolute units, not nanoseconds.
         * Unconverted it is the same clock the shim reads but roughly forty times
         * too small, which put it a host uptime behind the stream's calibration
         * anchor: every frame's public timestamp collapsed onto the clock origin,
         * and the gate that holds back frames produced before an observed move
         * never opened again. */
        display_time = mp_shim_nanos_from_ticks(native_time.unsignedLongLongValue);
    } else {
        /* The presentation timestamp is already in nanoseconds, and measured
         * equal to the converted display time above on this framework. */
        CMTime presentation = CMSampleBufferGetPresentationTimeStamp(sampleBuffer);
        if (CMTIME_IS_NUMERIC(presentation)) {
            double seconds = CMTimeGetSeconds(presentation);
            display_time = seconds > 0.0 ? (uint64_t)(seconds * 1e9) : 0;
        }
    }

    mp_shim_frame_info info;
    memset(&info, 0, sizeof(info));
    info.struct_size = (uint32_t)sizeof(info);
    info.pixel_format = MP_SHIM_PIXEL_BGRA8;
    info.content_width = content_width;
    info.content_height = content_height;
    info.surface_width = (uint32_t)surface_width;
    info.surface_height = (uint32_t)surface_height;
    info.display_time_nanos = display_time;
    info.scale_factor = factor * scale;
    info.content_origin_x = origin_x;
    info.content_origin_y = origin_y;

    struct mp_shim_frame borrowed;
    memset(&borrowed, 0, sizeof(borrowed));
    borrowed.magic = MP_SHIM_FRAME_MAGIC;
    borrowed.owns_buffer = false;
    borrowed.buffer = image;
    borrowed.info = info;
    borrowed.session = session;

    if ((session->testing_raise_sites & MP_SHIM_RAISE_BEFORE_CALLBACK) != 0) {
        [NSException raise:@"MPShimInjectedFailure" format:@"before frame callback"];
    }
    if (session->frame_callback != NULL) {
        /* No shim lock is held here: invoking a host callback under one is how a
         * deadlock between the producer and a consumer is built. */
        (void)session->frame_callback(session->callback_context, &borrowed, &info);
    }
    /* The borrow ends with the call, so the handle stops being usable here. */
    borrowed.magic = 0;
    borrowed.buffer = NULL;
    if ((session->testing_raise_sites & MP_SHIM_RAISE_AFTER_CALLBACK) != 0) {
        [NSException raise:@"MPShimInjectedFailure" format:@"after frame callback returned"];
    }
}

- (void)stream:(id)stream didStopWithError:(NSError *)error {
    (void)stream;
    struct mp_shim_session *session = _session;
    if (session == NULL || session->magic != MP_SHIM_SESSION_MAGIC) {
        return;
    }
    /*
     * Inside the fence, through the door that exists for a terminal report. Without
     * it the fence could observe no callback in flight and return while this one was
     * still reporting, after which the caller reclaims the state `callback_context`
     * points at — which is the one thing a successful fence promises cannot happen.
     * A refusal here means the fence already succeeded, so there is no caller state
     * left to report to and the stop is dropped rather than delivered.
     */
    if (!mp_shim_admission_enter_final(&session->admission)) {
        return;
    }
    @try {
        /* The producer has ended. Admission stops before the report, so no frame is
         * admitted after the stop the caller is about to observe. */
        mp_shim_admission_stop(&session->admission);
        bool expected = false;
        if (atomic_compare_exchange_strong(&session->stop_reported, &expected, true) &&
            session->stopped_callback != NULL) {
            @try {
                session->stopped_callback(session->callback_context, mp_shim_error_status(error));
            } @catch (NSException *exception) {
                (void)exception;
            } @catch (...) {
            }
        }
    } @finally {
        /* Decremented here so a thrown exception cannot strand the fence. */
        mp_shim_admission_leave(&session->admission);
    }
}

@end

#pragma mark - Session lifecycle

static bool mp_shim_session_valid(const struct mp_shim_session *session) {
    return session != NULL && session->magic == MP_SHIM_SESSION_MAGIC;
}

/*
 * Finds the window or display the request names, in a snapshot of its own.
 *
 * A window is matched on its owner as well as its number. The number alone does not
 * name an incarnation, because macOS recycles it: an application that closes a window
 * and opens another can be handed the same number, and matching on the number alone
 * then captures a window the caller never asked for — another application's, if the
 * number crossed processes. The caller records the owner for exactly this reason and
 * cannot enforce it here, because this open queries the shareable content again
 * rather than reusing what the caller validated.
 */
static id mp_shim_find_native_target(const MPShimFramework *framework, uint32_t kind,
                                     uint64_t native_id, int64_t owner_process,
                                     uint64_t timeout_nanos, mp_shim_status *out_status) {
    id content = mp_shim_shareable_content(framework, timeout_nanos, out_status);
    if (content == nil) {
        return nil;
    }
    id<MPShimShareableContent> shareable_content = (id<MPShimShareableContent>)content;
    if (kind == MP_SHIM_TARGET_WINDOW) {
        for (id window in shareable_content.windows) {
            id<MPShimWindow> typed = (id<MPShimWindow>)window;
            if (typed.windowID != (uint32_t)native_id) {
                continue;
            }
            id owner = typed.owningApplication;
            int64_t owned_by =
                owner == nil ? 0 : (int64_t)((id<MPShimRunningApplication>)owner).processID;
            if (owned_by == owner_process) {
                *out_status = MP_SHIM_OK;
                return window;
            }
            /* The number matched and the owner did not, so this is a different
             * incarnation rather than the target that was asked for. */
            *out_status = MP_SHIM_TARGET_LOST;
            return nil;
        }
    } else {
        for (id display in shareable_content.displays) {
            if (((id<MPShimDisplay>)display).displayID == (CGDirectDisplayID)native_id) {
                *out_status = MP_SHIM_OK;
                return display;
            }
        }
    }
    *out_status = MP_SHIM_TARGET_LOST;
    return nil;
}

/*
 * Reads one native slot and retains what it holds, as one step.
 *
 * The two halves are what needed joining. ARC does retain the strong local, so the
 * object cannot be deallocated once this returns — but a close landing between the
 * read and that retain retains an object it has already released. Holding
 * `native_mutex` across the pair is the whole fix; the lock is released before the
 * caller sends the object anything.
 */
static id mp_shim_session_copy_slot(struct mp_shim_session *session, CFTypeRef *slot) {
    id native = nil;
    pthread_mutex_lock(&session->native_mutex);
    if (*slot != NULL) {
        /* Assigning a __bridge cast to a strong local is the retain. */
        native = (__bridge id)*slot;
    }
    pthread_mutex_unlock(&session->native_mutex);
    return native;
}

static id<MPShimStream> mp_shim_session_copy_stream(struct mp_shim_session *session) {
    return (id<MPShimStream>)mp_shim_session_copy_slot(session, &session->stream);
}

static id<MPShimStreamConfiguration> mp_shim_session_copy_configuration(
    struct mp_shim_session *session) {
    return (id<MPShimStreamConfiguration>)mp_shim_session_copy_slot(session,
                                                                    &session->configuration);
}

mp_shim_status mp_shim_session_open(const mp_shim_open_request *request, mp_shim_session **out) {
    if (out == NULL) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    *out = NULL;
    if (request == NULL || request->struct_size < sizeof(mp_shim_open_request) ||
        request->frame_callback == NULL || request->pixel_width == 0 ||
        request->pixel_height == 0 || request->pixel_width > MP_SHIM_MAX_PIXEL_EXTENT ||
        request->pixel_height > MP_SHIM_MAX_PIXEL_EXTENT ||
        !mp_shim_surface_within_limit(request->pixel_width, request->pixel_height) ||
        request->detached_budget == 0 ||
        request->detached_budget > MP_SHIM_MAX_DETACHED_BUDGET ||
        (request->kind != MP_SHIM_TARGET_WINDOW && request->kind != MP_SHIM_TARGET_DISPLAY)) {
        return MP_SHIM_INVALID_ARGUMENT;
    }

    MP_SHIM_BEGIN
    const MPShimFramework *framework = mp_shim_capture_framework();
    if (framework == NULL) {
        return MP_SHIM_UNSUPPORTED;
    }
    if (!mp_shim_screen_capture_preflight()) {
        return MP_SHIM_PERMISSION_DENIED;
    }

    /*
     * Everything fallible happens while these strong locals own the native
     * objects, so an exception unwinding out of this scope releases every one of
     * them. Ownership moves into the session struct only after the last failure
     * point, which is what makes the failure path leak nothing.
     */
    mp_shim_status located = MP_SHIM_PLATFORM_FAILURE;
    id target = mp_shim_find_native_target(framework, request->kind, request->native_id,
                                           request->owner_process, request->timeout_nanos, &located);
    if (target == nil) {
        return located;
    }

    id<MPShimContentFilterInit> filter = nil;
    if (request->kind == MP_SHIM_TARGET_WINDOW) {
        filter = [(id<MPShimContentFilterInit>)[framework->content_filter alloc]
            initWithDesktopIndependentWindow:target];
    } else {
        filter = [(id<MPShimContentFilterInit>)[framework->content_filter alloc]
              initWithDisplay:target
             excludingWindows:@[]];
    }
    if (filter == nil) {
        return MP_SHIM_PLATFORM_FAILURE;
    }

    id<MPShimStreamConfiguration> configuration = [[framework->stream_configuration alloc] init];
    if (configuration == nil) {
        return MP_SHIM_PLATFORM_FAILURE;
    }
    uint32_t depth = request->queue_depth;
    if (depth < MP_SHIM_MIN_QUEUE_DEPTH) {
        depth = MP_SHIM_MIN_QUEUE_DEPTH;
    } else if (depth > MP_SHIM_MAX_QUEUE_DEPTH) {
        depth = MP_SHIM_MAX_QUEUE_DEPTH;
    }
    configuration.width = request->pixel_width;
    configuration.height = request->pixel_height;
    configuration.pixelFormat = kCVPixelFormatType_32BGRA;
    configuration.showsCursor = request->shows_cursor;
    configuration.queueDepth = (NSInteger)depth;
    configuration.scalesToFit = NO;

    MPShimStreamOutput *output = [MPShimStreamOutput new];
    id<MPShimStream> stream = [(id<MPShimStream>)[framework->stream alloc]
        initWithFilter:filter
         configuration:configuration
              delegate:output];
    if (stream == nil) {
        return MP_SHIM_PLATFORM_FAILURE;
    }

    dispatch_queue_attr_t attributes = dispatch_queue_attr_make_with_qos_class(
        DISPATCH_QUEUE_SERIAL, QOS_CLASS_USER_INITIATED, 0);
    dispatch_queue_t queue =
        dispatch_queue_create("io.github.pashifika.mado-pilot.capture", attributes);
    if (queue == nil) {
        return MP_SHIM_PLATFORM_FAILURE;
    }

    struct mp_shim_session *session = calloc(1, sizeof(struct mp_shim_session));
    if (session == NULL) {
        return MP_SHIM_PLATFORM_FAILURE;
    }
    session->magic = MP_SHIM_SESSION_MAGIC;
    /* The Rust handle's reference. Every other holder adds its own. */
    atomic_store(&session->refs, 1u);
    mp_shim_admission_init(&session->admission);
    pthread_mutex_init(&session->native_mutex, NULL);
    pthread_mutex_init(&session->pool_mutex, NULL);
    session->kind = request->kind;
    session->native_id = request->native_id;
    session->detached_budget = request->detached_budget;
    session->callback_context = request->callback_context;
    session->frame_callback = request->frame_callback;
    session->stopped_callback = request->stopped_callback;
    session->testing_raise_sites = request->testing_raise_sites;
    atomic_store(&session->output_added, false);
    atomic_store(&session->started, false);
    atomic_store(&session->closed, false);
    atomic_store(&session->stop_reported, false);

    NSError *output_error = nil;
    BOOL added = NO;
    /*
     * The session is a raw allocation at this point, so it is not covered by the ARC
     * locals that make every other failure here release what it owns. A raise from
     * `addStreamOutput` would unwind straight to MP_SHIM_END and leak the allocation
     * with its initialized mutexes and admission, so the raise is caught here, the
     * one reference is dropped, and the exception continues to the boundary handler.
     */
    @try {
        added = [stream addStreamOutput:output
                                  type:MPShimStreamOutputTypeScreen
                    sampleHandlerQueue:queue
                                 error:&output_error];
    } @catch (...) {
        mp_shim_session_unref(session);
        @throw;
    }
    if (!added) {
        mp_shim_session_unref(session);
        return mp_shim_error_status(output_error);
    }
    atomic_store(&session->output_added, true);
    /*
     * Adopted after the registration rather than before it, so that the two failures
     * above have exactly one reference to drop. Nothing is delivered in between: the
     * framework produces no sample until the capture starts, and reports no stop for
     * a stream that never did.
     */
    [output adoptSession:session];

    /* The last failure point has passed. Take ownership. */
    session->stream = CFBridgingRetain(stream);
    mp_shim_note_owned();
    session->configuration = CFBridgingRetain(configuration);
    mp_shim_note_owned();
    session->filter = CFBridgingRetain(filter);
    mp_shim_note_owned();
    session->output = CFBridgingRetain(output);
    mp_shim_note_owned();
    session->queue = CFBridgingRetain(queue);
    mp_shim_note_owned();
    *out = session;
    return MP_SHIM_OK;
    MP_SHIM_END
}

mp_shim_status mp_shim_session_start(mp_shim_session *session, uint64_t timeout_nanos) {
    if (!mp_shim_session_valid(session)) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    if (atomic_load(&session->closed)) {
        return MP_SHIM_CLOSED;
    }
    MP_SHIM_BEGIN
    id<MPShimStream> stream = mp_shim_session_copy_stream(session);
    if (stream == nil) {
        return MP_SHIM_CLOSED;
    }
    __block NSError *failure = nil;
    dispatch_semaphore_t ready = dispatch_semaphore_create(0);
    /*
     * The completion block settles the start rather than reporting it, because a
     * start can outlive the wait below. When that happened, `started` stayed false
     * forever, close skipped its stop on that condition, and a late success left
     * screen capture running with nothing tracking it. So the block records the
     * outcome, and if it succeeded after teardown had already run, it stops the
     * producer itself. Both it and close may end up stopping the stream, and the
     * second stop reports that it was already in that state, which close treats as
     * the success it is.
     */
    MPShimSessionHold *hold = [[MPShimSessionHold alloc] initWithSession:session];
    [stream startCaptureWithCompletionHandler:^(NSError *error) {
      /* Captured so the session outlives this block, however the block ends and
       * whether or not the message below ever accepted it. */
      (void)hold;
      /*
       * A callback trampoline, so it contains its own exceptions the way every other
       * one in this file does. The stop below is a framework message and can raise,
       * and an exception leaving this block unwinds into the framework with no handler
       * anywhere above it — which is an abort rather than a status. The signal is owed
       * whatever happens, so it runs in @finally; a waiter that never received it
       * would block to its own timeout for no reason.
       */
      @try {
          failure = error;
          if (error == nil) {
              atomic_store(&session->started, true);
              if (atomic_load(&session->closed)) {
                  [stream stopCaptureWithCompletionHandler:^(NSError *stopped) {
                    (void)stopped;
                  }];
                  atomic_store(&session->started, false);
              }
          }
          /* Last, so that the raise costs the outcome nothing and what it exercises is
           * the containment alone — the position the real stop message occupies. */
          if ((session->testing_raise_sites & MP_SHIM_RAISE_IN_START_COMPLETION) != 0) {
              [NSException raise:@"MPShimInjectedFailure" format:@"start completion"];
          }
      } @catch (NSException *exception) {
          (void)exception;
      } @catch (...) {
      } @finally {
          dispatch_semaphore_signal(ready);
      }
    }];
    mp_shim_status waited = mp_shim_wait(ready, timeout_nanos);
    if (waited != MP_SHIM_OK) {
        return waited;
    }
    if (failure != nil) {
        return mp_shim_error_status(failure);
    }
    /* `started` is the block's to set, so that one writer owns it whether or not the
     * wait above was the thing that observed the outcome. */
    if ((session->testing_raise_sites & MP_SHIM_RAISE_AT_START) != 0) {
        [NSException raise:@"MPShimInjectedFailure" format:@"session start"];
    }
    return MP_SHIM_OK;
    MP_SHIM_END
}

mp_shim_status mp_shim_session_reconfigure(mp_shim_session *session, uint32_t pixel_width,
                                           uint32_t pixel_height, uint64_t timeout_nanos) {
    if (!mp_shim_session_valid(session) || pixel_width == 0 || pixel_height == 0 ||
        pixel_width > MP_SHIM_MAX_PIXEL_EXTENT || pixel_height > MP_SHIM_MAX_PIXEL_EXTENT ||
        !mp_shim_surface_within_limit(pixel_width, pixel_height)) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    if (atomic_load(&session->closed)) {
        return MP_SHIM_CLOSED;
    }
    MP_SHIM_BEGIN
    id<MPShimStream> stream = mp_shim_session_copy_stream(session);
    id<MPShimStreamConfiguration> configuration = mp_shim_session_copy_configuration(session);
    if (stream == nil || configuration == nil) {
        return MP_SHIM_CLOSED;
    }
    configuration.width = pixel_width;
    configuration.height = pixel_height;

    __block NSError *failure = nil;
    dispatch_semaphore_t ready = dispatch_semaphore_create(0);
    [stream updateConfiguration:configuration
             completionHandler:^(NSError *error) {
               failure = error;
               dispatch_semaphore_signal(ready);
             }];
    mp_shim_status waited = mp_shim_wait(ready, timeout_nanos);
    if (waited != MP_SHIM_OK) {
        return waited;
    }
    if (failure != nil) {
        return mp_shim_error_status(failure);
    }

    /* Retire only reuse. A lease a caller still holds keeps its own buffer. */
    pthread_mutex_lock(&session->pool_mutex);
    mp_shim_pool_release_locked(session);
    pthread_mutex_unlock(&session->pool_mutex);
    return MP_SHIM_OK;
    MP_SHIM_END
}

mp_shim_status mp_shim_session_disable_callbacks(mp_shim_session *session) {
    if (!mp_shim_session_valid(session)) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    mp_shim_admission_stop(&session->admission);
    return MP_SHIM_OK;
}

mp_shim_status mp_shim_session_fence(mp_shim_session *session, uint64_t timeout_nanos) {
    if (!mp_shim_session_valid(session)) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    return mp_shim_admission_fence(&session->admission, timeout_nanos);
}

mp_shim_status mp_shim_session_close(mp_shim_session *session, uint64_t timeout_nanos) {
    if (!mp_shim_session_valid(session)) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    bool expected = false;
    if (!atomic_compare_exchange_strong(&session->closed, &expected, true)) {
        /* Idempotent: a later close finds the release already done. */
        return MP_SHIM_OK;
    }

    mp_shim_status reported = MP_SHIM_OK;
    @try {
        mp_shim_admission_stop(&session->admission);
        id<MPShimStream> stream = mp_shim_session_copy_stream(session);
        id output = mp_shim_session_copy_slot(session, &session->output);

        if (stream != nil && atomic_load(&session->output_added)) {
            NSError *removed = nil;
            if (![stream removeStreamOutput:output
                                      type:MPShimStreamOutputTypeScreen
                                     error:&removed]) {
                reported = mp_shim_error_status(removed);
            }
            atomic_store(&session->output_added, false);
        }

        if (stream != nil && atomic_load(&session->started)) {
            __block NSError *failure = nil;
            dispatch_semaphore_t ready = dispatch_semaphore_create(0);
            [stream stopCaptureWithCompletionHandler:^(NSError *error) {
              failure = error;
              dispatch_semaphore_signal(ready);
            }];
            mp_shim_status waited = mp_shim_wait(ready, timeout_nanos);
            if (waited != MP_SHIM_OK) {
                reported = waited;
            } else if (failure != nil) {
                mp_shim_status status = mp_shim_error_status(failure);
                /* A producer that has already stopped — because it was already in
                 * that state, or because the user or the system ended it first — is
                 * not a failure of the close the caller asked for. */
                if (status != MP_SHIM_CLOSED && status != MP_SHIM_STOPPED_BY_USER &&
                    status != MP_SHIM_STOPPED_BY_SYSTEM) {
                    reported = status;
                }
            }
            atomic_store(&session->started, false);
        }

        mp_shim_status fenced = mp_shim_admission_fence(&session->admission, timeout_nanos);
        if (fenced != MP_SHIM_OK && reported == MP_SHIM_OK) {
            reported = fenced;
        }
        /*
         * The output object's session pointer is deliberately left alone. Clearing it
         * here is what this used to do, and it protected nothing: a callback that has
         * already read the pointer holds the address whatever this writes afterwards.
         * The object holds a counted reference instead, so the fence above is what
         * makes it safe for the caller to release its own state, and the release
         * below is what lets the object — and with it that reference — go.
         */

        if ((session->testing_raise_sites & MP_SHIM_RAISE_AT_TEARDOWN) != 0) {
            [NSException raise:@"MPShimInjectedFailure" format:@"teardown"];
        }
    } @catch (NSException *exception) {
        (void)exception;
        reported = MP_SHIM_NATIVE_EXCEPTION;
    } @catch (...) {
        reported = MP_SHIM_NATIVE_EXCEPTION;
    } @finally {
        /*
         * Release runs here so a cleanup failure is reported without costing the
         * cleanup. The order is the stream, then the objects it was built from,
         * then the queue that delivered to it, then the detached pool.
         *
         * The slots are emptied under `native_mutex` and released after it, never
         * under it. Two reasons, and the second is the sharper one: a read-and-retain
         * elsewhere must not see a slot whose object this has already released, and
         * releasing the output object here runs its `dealloc`, which drops the
         * session reference it holds — so holding the mutex across that release would
         * be holding a mutex that the last reference destroys.
         *
         * Its own handlers, because an exception raised inside a @finally is not
         * caught by the @catch pair belonging to the same @try — it leaves the
         * function. This entry point opens a bare @try rather than MP_SHIM_BEGIN, so
         * nothing outside would catch it either, and a raise while releasing a bridged
         * framework object would cross the boundary that ADR 0012 rule 1 exists to
         * close.
         */
        @try {
            CFTypeRef released[5];
            pthread_mutex_lock(&session->native_mutex);
            released[0] = session->stream;
            released[1] = session->output;
            released[2] = session->configuration;
            released[3] = session->filter;
            released[4] = session->queue;
            session->stream = NULL;
            session->output = NULL;
            session->configuration = NULL;
            session->filter = NULL;
            session->queue = NULL;
            pthread_mutex_unlock(&session->native_mutex);

            for (size_t slot = 0; slot < 5; slot += 1) {
                if (released[slot] == NULL) {
                    continue;
                }
                /*
                 * Each release carries its own handler. One shared handler let a raise
                 * from the first release skip the four after it and the pool below —
                 * and because the slots are emptied above, nothing could reach those
                 * objects again afterwards, so the ownership cycle with the stream
                 * output would never be broken and no later close could retry.
                 *
                 * The count is noted only where the release returned. A release that
                 * raised may have left the object alive, and reporting it as gone
                 * would be the ownership scenarios agreeing with a leak.
                 */
                @try {
                    CFRelease(released[slot]);
                    mp_shim_note_released();
                } @catch (NSException *exception) {
                    (void)exception;
                    reported = MP_SHIM_NATIVE_EXCEPTION;
                } @catch (...) {
                    reported = MP_SHIM_NATIVE_EXCEPTION;
                }
            }

            pthread_mutex_lock(&session->pool_mutex);
            /* Unlocked in @finally for the reason mp_shim_pool_acquire is. */
            @try {
                mp_shim_pool_release_locked(session);
            } @finally {
                pthread_mutex_unlock(&session->pool_mutex);
            }
        } @catch (NSException *exception) {
            (void)exception;
            reported = MP_SHIM_NATIVE_EXCEPTION;
        } @catch (...) {
            reported = MP_SHIM_NATIVE_EXCEPTION;
        }
    }
    return reported;
}

void mp_shim_session_release(mp_shim_session *session) {
    if (!mp_shim_session_valid(session)) {
        return;
    }
    /*
     * Closing is not a courtesy here. It releases the stream output object, and that
     * object holds the reference that completes an ownership cycle with this session,
     * so a release that skipped the close would leak both rather than freeing either.
     */
    if (!atomic_load(&session->closed)) {
        (void)mp_shim_session_close(session, MP_SHIM_DEFAULT_TIMEOUT_NANOS);
    }
    /* Drops the handle's reference. A frame the caller still holds, or a callback
     * still in flight, keeps the allocation alive past this point. */
    mp_shim_session_unref(session);
}

mp_shim_status mp_shim_session_leased(const mp_shim_session *session, uint64_t *out_leased) {
    if (!mp_shim_session_valid(session) || out_leased == NULL) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    struct mp_shim_session *owner = (struct mp_shim_session *)session;
    pthread_mutex_lock(&owner->pool_mutex);
    *out_leased = owner->leased;
    pthread_mutex_unlock(&owner->pool_mutex);
    return MP_SHIM_OK;
}

mp_shim_status mp_shim_session_live_objects(const mp_shim_session *session, uint64_t *out_live) {
    if (!mp_shim_session_valid(session) || out_live == NULL) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    uint64_t live = 0;
    live += session->stream == NULL ? 0 : 1;
    live += session->configuration == NULL ? 0 : 1;
    live += session->filter == NULL ? 0 : 1;
    live += session->output == NULL ? 0 : 1;
    live += session->queue == NULL ? 0 : 1;
    live += session->pool == NULL ? 0 : 1;
    *out_live = live;
    return MP_SHIM_OK;
}
