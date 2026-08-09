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
#define MP_SHIM_TARGET_MAGIC 0x4d505354u

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

static CFTypeRef mp_shim_string_symbol(void *handle, const char *name);

#pragma mark - Public code-signing inspection loaded without an eager framework

/*
 * Security.framework is opened from an absolute system path for the same reason
 * ScreenCaptureKit, AppKit, and HIToolbox are: reporting execution context must
 * not add an eager load command to every binary that links this Adapter. These
 * are the exact public SecCode signatures in the qualified SDK, expressed with
 * opaque Core Foundation types so importing Security headers is unnecessary.
 */
typedef OSStatus (*MPShimSecCodeCopySelf)(uint32_t flags, CFTypeRef *out_code);
typedef OSStatus (*MPShimSecCodeCheckValidity)(CFTypeRef code, uint32_t flags,
                                               CFTypeRef requirement);
typedef OSStatus (*MPShimSecCodeCopySigningInformation)(CFTypeRef code, uint32_t flags,
                                                        CFDictionaryRef *out_information);

typedef struct {
    bool loaded;
    MPShimSecCodeCopySelf copy_self;
    MPShimSecCodeCheckValidity check_validity;
    MPShimSecCodeCopySigningInformation copy_signing_information;
    CFStringRef flags_key;
    CFStringRef identifier_key;
} MPShimCodeSigningApi;

static MPShimCodeSigningApi mp_shim_code_signing_api;
static pthread_once_t mp_shim_code_signing_once = PTHREAD_ONCE_INIT;

static void mp_shim_load_code_signing_api(void) {
    void *handle = dlopen("/System/Library/Frameworks/Security.framework/Versions/A/Security",
                          RTLD_LAZY | RTLD_LOCAL);
    if (handle == NULL) {
        handle = dlopen("/System/Library/Frameworks/Security.framework/Security",
                        RTLD_LAZY | RTLD_LOCAL);
    }
    if (handle == NULL) {
        return;
    }

    MPShimCodeSigningApi loaded;
    memset(&loaded, 0, sizeof(loaded));
    loaded.copy_self = (MPShimSecCodeCopySelf)dlsym(handle, "SecCodeCopySelf");
    loaded.check_validity =
        (MPShimSecCodeCheckValidity)dlsym(handle, "SecCodeCheckValidity");
    loaded.copy_signing_information = (MPShimSecCodeCopySigningInformation)dlsym(
        handle, "SecCodeCopySigningInformation");
    loaded.flags_key = (CFStringRef)mp_shim_string_symbol(handle, "kSecCodeInfoFlags");
    loaded.identifier_key =
        (CFStringRef)mp_shim_string_symbol(handle, "kSecCodeInfoIdentifier");
    loaded.loaded = loaded.copy_self != NULL && loaded.check_validity != NULL &&
                    loaded.copy_signing_information != NULL && loaded.flags_key != NULL &&
                    loaded.identifier_key != NULL;
    if (loaded.loaded) {
        mp_shim_code_signing_api = loaded;
    }
}

static const MPShimCodeSigningApi *mp_shim_signing_api(void) {
    pthread_once(&mp_shim_code_signing_once, mp_shim_load_code_signing_api);
    return mp_shim_code_signing_api.loaded ? &mp_shim_code_signing_api : NULL;
}

/*
 * Exact public Security.framework OSStatus values from CSCommon.h in the
 * qualified SDK. Only statuses that affirmatively describe invalid code are
 * classified as invalid; unreadable, internal, and unknown failures remain
 * platform failures rather than overclaiming what Security established.
 */
static bool mp_shim_signature_status_is_invalid(OSStatus status) {
    switch (status) {
    case -67063: /* errSecCSGuestInvalid */
    case -67061: /* errSecCSSignatureFailed */
    case -67059: /* errSecCSSignatureUnsupported */
    case -67058: /* errSecCSBadDictionaryFormat */
    case -67057: /* errSecCSResourcesNotSealed */
    case -67056: /* errSecCSResourcesNotFound */
    case -67055: /* errSecCSResourcesInvalid */
    case -67054: /* errSecCSBadResource */
    case -67053: /* errSecCSResourceRulesInvalid */
    case -67052: /* errSecCSReqInvalid */
    case -67051: /* errSecCSReqUnsupported */
    case -67050: /* errSecCSReqFailed */
    case -67049: /* errSecCSBadObjectFormat */
    case -67047: /* errSecCSHostReject */
    case -67045: /* errSecCSSignatureInvalid */
    case -67034: /* errSecCSStaticCodeChanged */
    case -67030: /* errSecCSInfoPlistFailed */
    case -67029: /* errSecCSNoMainExecutable */
    case -67028: /* errSecCSBadBundleFormat */
    case -67023: /* errSecCSResourceDirectoryFailed */
    case -67022: /* errSecCSUnsignedNestedCode */
    case -67021: /* errSecCSBadNestedCode */
    case -67010: /* errSecCSBadMainExecutable */
    case -67007: /* errSecCSWeakResourceEnvelope */
    case -67003: /* errSecCSInvalidSymlink */
    case -67000: /* errSecCSUnsupportedDigestAlgorithm */
    case -66999: /* errSecCSInvalidAssociatedFileData */
    case -66998: /* errSecCSInvalidTeamIdentifier */
    case -66997: /* errSecCSBadTeamIdentifier */
    case -66996: /* errSecCSSignatureUntrusted */
    case -66994: /* errSecCSInvalidEntitlements */
    case -66993: /* errSecCSInvalidRuntimeVersion */
    case -66992: /* errSecCSRevokedNotarization */
        return true;
    default:
        return false;
    }
}

static uint32_t mp_shim_classify_signature(OSStatus signing_info_status,
                                           OSStatus validity_status, bool has_identifier,
                                           uint32_t signature_flags) {
    if (validity_status != 0) {
        /* errSecCSUnsigned is authoritative only when the signing-information
         * dictionary independently agrees that no identifier exists. */
        if (validity_status == -67062) {
            if (signing_info_status != 0) {
                return MP_SHIM_SIGNATURE_PLATFORM_FAILURE;
            }
            return has_identifier ? MP_SHIM_SIGNATURE_INVALID : MP_SHIM_SIGNATURE_UNSIGNED;
        }
        return mp_shim_signature_status_is_invalid(validity_status)
                   ? MP_SHIM_SIGNATURE_INVALID
                   : MP_SHIM_SIGNATURE_PLATFORM_FAILURE;
    }
    if (signing_info_status != 0 || !has_identifier) {
        /* Successful validity with no identifier is contradictory, not proof
         * that the code is unsigned. */
        return MP_SHIM_SIGNATURE_PLATFORM_FAILURE;
    }
    return (signature_flags & 0x0002u) != 0 ? MP_SHIM_SIGNATURE_AD_HOC
                                            : MP_SHIM_SIGNATURE_CERTIFICATE_BACKED;
}

static bool mp_shim_copy_signing_identifier(CFStringRef identifier, uint8_t *out_identifier,
                                            size_t *out_identifier_len) {
    if (identifier == NULL || CFGetTypeID(identifier) != CFStringGetTypeID()) {
        return false;
    }
    CFIndex characters = CFStringGetLength(identifier);
    CFIndex used = 0;
    CFIndex converted = CFStringGetBytes(
        identifier, CFRangeMake(0, characters), kCFStringEncodingUTF8, 0, false, out_identifier,
        (CFIndex)MP_SHIM_MAX_SIGNING_IDENTIFIER, &used);
    if (characters <= 0 || converted != characters || used <= 0 ||
        used > (CFIndex)MP_SHIM_MAX_SIGNING_IDENTIFIER) {
        return false;
    }
    out_identifier[used] = 0;
    *out_identifier_len = (size_t)used;
    return true;
}

static void mp_shim_read_signature_context(uint32_t *out_signature, uint8_t *out_identifier,
                                           size_t *out_identifier_len) {
    const MPShimCodeSigningApi *api = mp_shim_signing_api();
    if (api == NULL) {
        return;
    }

    CFTypeRef code = NULL;
    CFDictionaryRef information = NULL;
    @try {
        OSStatus copy_self_status = api->copy_self(0, &code);
        if (copy_self_status != 0 || code == NULL) {
            return;
        }

        /* The qualified SDK requires successful validity before treating
         * signing information as complete. An unsigned result is corroborated
         * below by the documented absence of an identifier. */
        OSStatus validity_status = api->check_validity(code, 0, NULL);
        if (validity_status != 0 && validity_status != -67062) {
            *out_signature = mp_shim_classify_signature(0, validity_status, false, 0);
            return;
        }
        OSStatus information_status = api->copy_signing_information(code, 0, &information);
        if (information_status != 0 || information == NULL) {
            *out_signature =
                mp_shim_classify_signature(information_status, validity_status, false, 0);
            return;
        }

        CFTypeRef identifier_value =
            CFDictionaryGetValue(information, api->identifier_key);
        bool has_identifier = identifier_value != NULL;
        if (validity_status != 0 || !has_identifier) {
            *out_signature =
                mp_shim_classify_signature(0, validity_status, has_identifier, 0);
            return;
        }

        uint32_t signature_flags = 0;
        CFTypeRef flags_value = CFDictionaryGetValue(information, api->flags_key);
        int64_t flags = 0;
        if (flags_value == NULL || CFGetTypeID(flags_value) != CFNumberGetTypeID() ||
            !CFNumberGetValue((CFNumberRef)flags_value, kCFNumberSInt64Type, &flags) || flags < 0 ||
            flags > UINT32_MAX) {
            return;
        }
        signature_flags = (uint32_t)flags;

        *out_signature =
            mp_shim_classify_signature(0, validity_status, true, signature_flags);
        if (*out_signature != MP_SHIM_SIGNATURE_AD_HOC &&
            *out_signature != MP_SHIM_SIGNATURE_CERTIFICATE_BACKED) {
            return;
        }
        if (!mp_shim_copy_signing_identifier((CFStringRef)identifier_value, out_identifier,
                                             out_identifier_len)) {
            *out_signature = MP_SHIM_SIGNATURE_PLATFORM_FAILURE;
            *out_identifier_len = 0;
            out_identifier[0] = 0;
        }
    } @finally {
        if (information != NULL) {
            CFRelease(information);
        }
        if (code != NULL) {
            CFRelease(code);
        }
    }
}

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
@property(nonnull, nonatomic, readonly) NSArray *includedDisplays;
@property(nonnull, nonatomic, readonly) NSArray *includedWindows;
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
    CFTypeRef key_screen_rect;
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
 * not already use, since discovery already reads display bounds.
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
    loaded.key_screen_rect = mp_shim_string_symbol(handle, "SCStreamFrameInfoScreenRect");
    loaded.error_domain = mp_shim_string_symbol(handle, "SCStreamErrorDomain");

    loaded.loaded = loaded.shareable_content != Nil && loaded.stream != Nil &&
                    loaded.stream_configuration != Nil && loaded.content_filter != Nil &&
                    loaded.key_status != NULL && loaded.key_content_rect != NULL &&
                    loaded.key_screen_rect != NULL && loaded.key_scale_factor != NULL &&
                    loaded.error_domain != NULL;
    if (loaded.loaded) {
        mp_shim_framework = loaded;
    }
}

static const MPShimFramework *mp_shim_capture_framework(void) {
    /* Version one needs SCStreamFrameInfoScreenRect for every published frame.
     * The qualified Apple Silicon environment and declared implementation floor
     * are macOS 26.5.2 and SDK 26.5; earlier hosts are outside the support contract. */
    if (@available(macOS 26.5.2, *)) {
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

static void mp_shim_testing_delay(uint64_t delay_nanos) {
    if (delay_nanos == 0) {
        return;
    }
    struct timespec delay;
    delay.tv_sec = (time_t)(delay_nanos / 1000000000ull);
    delay.tv_nsec = (long)(delay_nanos % 1000000000ull);
    (void)nanosleep(&delay, NULL);
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

#pragma mark - In-flight capture start

/*
 * Whether a capture start is still to settle, so that teardown can join it.
 *
 * A start can outlive the wait its own caller gave it, and the outcome then arrives
 * with nobody left to report it to: open has already returned a failure and, once the
 * fence succeeds, the Adapter state a stopped callback would reach is gone. Teardown
 * waiting for the start is what puts that outcome back where a caller can see it —
 * close reads the settled result and reports it through its own status.
 *
 * A timed-out close leaves `pending` intact. A later close resumes this same wait;
 * the completion owns the session through its counted hold, so no orphaning shortcut
 * is needed and no late successful start escapes teardown.
 */
typedef struct MPShimStartGate {
    pthread_mutex_t mutex;
    pthread_cond_t settled;
    bool pending;
} MPShimStartGate;

static void mp_shim_start_gate_init(MPShimStartGate *gate) {
    pthread_mutex_init(&gate->mutex, NULL);
    pthread_cond_init(&gate->settled, NULL);
    gate->pending = false;
}

static void mp_shim_start_gate_destroy(MPShimStartGate *gate) {
    pthread_cond_destroy(&gate->settled);
    pthread_mutex_destroy(&gate->mutex);
}

static void mp_shim_start_gate_begin(MPShimStartGate *gate) {
    pthread_mutex_lock(&gate->mutex);
    gate->pending = true;
    pthread_mutex_unlock(&gate->mutex);
}

static void mp_shim_start_gate_end(MPShimStartGate *gate) {
    pthread_mutex_lock(&gate->mutex);
    gate->pending = false;
    pthread_cond_broadcast(&gate->settled);
    pthread_mutex_unlock(&gate->mutex);
}

/*
 * Waits out `timeout_nanos` for a start to settle. Returns immediately when none is in
 * flight, which is every close that did not race one.
 *
 * The wait is relative and on the monotonic clock, and the remaining time is recomputed
 * each turn, for the same reasons the admission fence above is written that way.
 */
static mp_shim_status mp_shim_start_gate_wait(MPShimStartGate *gate, uint64_t timeout_nanos) {
    uint64_t began = mp_shim_nanos_from_ticks(mach_absolute_time());
    uint64_t deadline = began > UINT64_MAX - timeout_nanos ? UINT64_MAX : began + timeout_nanos;

    pthread_mutex_lock(&gate->mutex);
    mp_shim_status status = MP_SHIM_OK;
    while (gate->pending) {
        uint64_t now = mp_shim_nanos_from_ticks(mach_absolute_time());
        if (now >= deadline) {
            status = MP_SHIM_TIMED_OUT;
            break;
        }
        uint64_t left = deadline - now;
        struct timespec relative;
        relative.tv_sec = (time_t)(left / 1000000000ull);
        relative.tv_nsec = (long)(left % 1000000000ull);
        if (pthread_cond_timedwait_relative_np(&gate->settled, &gate->mutex, &relative) != 0) {
            status = gate->pending ? MP_SHIM_TIMED_OUT : MP_SHIM_OK;
            break;
        }
    }
    pthread_mutex_unlock(&gate->mutex);
    return status;
}

typedef struct MPShimStopGate {
    pthread_mutex_t mutex;
    pthread_cond_t settled;
    bool pending;
    mp_shim_status result;
} MPShimStopGate;

static void mp_shim_stop_gate_init(MPShimStopGate *gate) {
    pthread_mutex_init(&gate->mutex, NULL);
    pthread_cond_init(&gate->settled, NULL);
    gate->pending = false;
    gate->result = MP_SHIM_OK;
}

static void mp_shim_stop_gate_destroy(MPShimStopGate *gate) {
    pthread_cond_destroy(&gate->settled);
    pthread_mutex_destroy(&gate->mutex);
}

static void mp_shim_stop_gate_begin(MPShimStopGate *gate) {
    pthread_mutex_lock(&gate->mutex);
    gate->pending = true;
    gate->result = MP_SHIM_OK;
    pthread_mutex_unlock(&gate->mutex);
}

static void mp_shim_stop_gate_end(MPShimStopGate *gate, mp_shim_status result) {
    pthread_mutex_lock(&gate->mutex);
    if (gate->pending) {
        gate->result = result;
        gate->pending = false;
        pthread_cond_broadcast(&gate->settled);
    }
    pthread_mutex_unlock(&gate->mutex);
}

static bool mp_shim_stop_gate_pending(MPShimStopGate *gate) {
    pthread_mutex_lock(&gate->mutex);
    bool pending = gate->pending;
    pthread_mutex_unlock(&gate->mutex);
    return pending;
}

static mp_shim_status mp_shim_stop_gate_wait(MPShimStopGate *gate, uint64_t timeout_nanos) {
    uint64_t began = mp_shim_nanos_from_ticks(mach_absolute_time());
    uint64_t deadline = began > UINT64_MAX - timeout_nanos ? UINT64_MAX : began + timeout_nanos;

    pthread_mutex_lock(&gate->mutex);
    while (gate->pending) {
        uint64_t now = mp_shim_nanos_from_ticks(mach_absolute_time());
        if (now >= deadline) {
            pthread_mutex_unlock(&gate->mutex);
            return MP_SHIM_TIMED_OUT;
        }
        uint64_t left = deadline - now;
        struct timespec relative;
        relative.tv_sec = (time_t)(left / 1000000000ull);
        relative.tv_nsec = (long)(left % 1000000000ull);
        if (pthread_cond_timedwait_relative_np(&gate->settled, &gate->mutex, &relative) != 0 &&
            gate->pending) {
            pthread_mutex_unlock(&gate->mutex);
            return MP_SHIM_TIMED_OUT;
        }
    }
    mp_shim_status result = gate->result;
    pthread_mutex_unlock(&gate->mutex);
    return result;
}

/*
 * The asynchronous stop completion's complete no-throw boundary.
 *
 * `gate` and `started` belong to a session retained by the completion block. The
 * gate is settled exactly once from @finally, including when delay, error
 * translation, or the deliberate regression seam raises.
 */
static void mp_shim_complete_stop(MPShimStopGate *gate, atomic_bool *started, NSError *error,
                                  uint64_t delay_nanos, bool raise_for_test) {
    mp_shim_status status = MP_SHIM_NATIVE_EXCEPTION;
    @try {
        mp_shim_testing_delay(delay_nanos);
        status = error == nil ? MP_SHIM_OK : mp_shim_error_status(error);
        if (raise_for_test) {
            [NSException raise:@"MPShimInjectedFailure" format:@"stop completion"];
        }
    } @catch (NSException *exception) {
        (void)exception;
        status = MP_SHIM_NATIVE_EXCEPTION;
    } @catch (...) {
        status = MP_SHIM_NATIVE_EXCEPTION;
    } @finally {
        atomic_store(started, false);
        mp_shim_stop_gate_end(gate, status);
    }
}

mp_shim_status mp_shim_testing_gate_retries(
    uint64_t completion_delay_nanos, uint64_t first_wait_nanos, uint64_t second_wait_nanos,
    mp_shim_status *out_start_first, mp_shim_status *out_start_second,
    mp_shim_status *out_stop_first, mp_shim_status *out_stop_second) {
    if (completion_delay_nanos == 0 || out_start_first == NULL || out_start_second == NULL ||
        out_stop_first == NULL || out_stop_second == NULL) {
        return MP_SHIM_INVALID_ARGUMENT;
    }

    MPShimStartGate start;
    mp_shim_start_gate_init(&start);
    mp_shim_start_gate_begin(&start);
    MPShimStartGate *start_ptr = &start;
    dispatch_group_t start_group = dispatch_group_create();
    dispatch_group_enter(start_group);
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_UTILITY, 0), ^{
      mp_shim_testing_delay(completion_delay_nanos);
      mp_shim_start_gate_end(start_ptr);
      dispatch_group_leave(start_group);
    });
    *out_start_first = mp_shim_start_gate_wait(&start, first_wait_nanos);
    *out_start_second = mp_shim_start_gate_wait(&start, second_wait_nanos);
    dispatch_group_wait(start_group, DISPATCH_TIME_FOREVER);
    mp_shim_start_gate_destroy(&start);

    MPShimStopGate stop;
    mp_shim_stop_gate_init(&stop);
    mp_shim_stop_gate_begin(&stop);
    MPShimStopGate *stop_ptr = &stop;
    dispatch_group_t stop_group = dispatch_group_create();
    dispatch_group_enter(stop_group);
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_UTILITY, 0), ^{
      mp_shim_testing_delay(completion_delay_nanos);
      mp_shim_stop_gate_end(stop_ptr, MP_SHIM_OK);
      dispatch_group_leave(stop_group);
    });
    *out_stop_first = mp_shim_stop_gate_wait(&stop, first_wait_nanos);
    *out_stop_second = mp_shim_stop_gate_wait(&stop, second_wait_nanos);
    dispatch_group_wait(stop_group, DISPATCH_TIME_FOREVER);
    mp_shim_stop_gate_destroy(&stop);
    return MP_SHIM_OK;
}

mp_shim_status mp_shim_testing_stop_completion_exception(mp_shim_status *out_status,
                                                         bool *out_started) {
    if (out_status == NULL || out_started == NULL) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    MPShimStopGate gate;
    atomic_bool started;
    atomic_init(&started, true);
    mp_shim_stop_gate_init(&gate);
    mp_shim_stop_gate_begin(&gate);
    mp_shim_complete_stop(&gate, &started, nil, 0, true);
    /* A later duplicate settlement cannot replace the first contained result. */
    mp_shim_stop_gate_end(&gate, MP_SHIM_OK);
    *out_status = mp_shim_stop_gate_wait(&gate, MP_SHIM_DEFAULT_TIMEOUT_NANOS);
    *out_started = atomic_load(&started);
    mp_shim_stop_gate_destroy(&gate);
    return MP_SHIM_OK;
}

#pragma mark - Handles

struct mp_shim_inventory {
    uint32_t magic;
    CFTypeRef entries; /* NSArray<MPShimInventoryEntry *> */
};

/*
 * One exact capture filter constructed from a discovery snapshot.
 *
 * `native_id` and `owner_process` are repeated here only to validate metadata at
 * the boundary. `filter` is the authority and open performs no later lookup.
 */
struct mp_shim_target {
    uint32_t magic;
    uint32_t kind;
    uint64_t native_id;
    int64_t owner_process;
    CFTypeRef filter; /* SCContentFilter */
};

#define MP_SHIM_CLOSE_START 0u
#define MP_SHIM_CLOSE_OUTPUT 1u
#define MP_SHIM_CLOSE_STOP 2u
#define MP_SHIM_CLOSE_FENCE 3u
#define MP_SHIM_CLOSE_RELEASE 4u
#define MP_SHIM_CLOSE_COMPLETE 5u

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
    /* Lets teardown join a capture start that outlived its own caller's wait. */
    MPShimStartGate start_gate;
    /* Preserves one asynchronous stop across close retries. */
    MPShimStopGate stop_gate;
    /* Serializes claims on the resumable close phases without spanning waits. */
    pthread_mutex_t close_mutex;
    pthread_cond_t close_idle;
    bool close_active;
    uint32_t close_phase;
    mp_shim_status close_error;
    bool close_error_reported;

    uint32_t kind;
    uint64_t native_id;
    uint32_t testing_raise_sites;
    uint64_t testing_start_delay_nanos;
    uint64_t testing_stop_delay_nanos;

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
    mp_shim_status (*frame_commit_callback)(void *);
    void (*stopped_callback)(void *, mp_shim_status);

    atomic_bool output_added;
    atomic_bool started;
    atomic_bool closing;
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

/* Claims exclusive phase advancement without holding the mutex across a wait. */
static mp_shim_status mp_shim_close_claim(struct mp_shim_session *session, uint64_t deadline,
                                          bool *out_complete) {
    *out_complete = false;
    pthread_mutex_lock(&session->close_mutex);
    while (session->close_active) {
        uint64_t now = mp_shim_nanos_from_ticks(mach_absolute_time());
        if (now >= deadline) {
            pthread_mutex_unlock(&session->close_mutex);
            return MP_SHIM_TIMED_OUT;
        }
        uint64_t left = deadline - now;
        struct timespec relative;
        relative.tv_sec = (time_t)(left / 1000000000ull);
        relative.tv_nsec = (long)(left % 1000000000ull);
        if (pthread_cond_timedwait_relative_np(&session->close_idle, &session->close_mutex,
                                               &relative) != 0 &&
            session->close_active) {
            pthread_mutex_unlock(&session->close_mutex);
            return MP_SHIM_TIMED_OUT;
        }
    }
    if (session->close_phase == MP_SHIM_CLOSE_COMPLETE) {
        *out_complete = true;
    } else {
        session->close_active = true;
    }
    pthread_mutex_unlock(&session->close_mutex);
    return MP_SHIM_OK;
}

static void mp_shim_close_release(struct mp_shim_session *session) {
    pthread_mutex_lock(&session->close_mutex);
    session->close_active = false;
    pthread_cond_broadcast(&session->close_idle);
    pthread_mutex_unlock(&session->close_mutex);
}

@interface MPShimInventoryEntry : NSObject
@property(nonatomic, assign) mp_shim_target_info info;
@property(nonatomic, copy) NSData *name;
@property(nonatomic, strong) id nativeTarget;
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
    mp_shim_start_gate_destroy(&session->start_gate);
    mp_shim_stop_gate_destroy(&session->stop_gate);
    pthread_cond_destroy(&session->close_idle);
    pthread_mutex_destroy(&session->close_mutex);
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

/* Derives prospective producer capacity from one sample's source-resolution facts. */
static bool mp_shim_recommended_surface(CGSize logical_size, double display_scale,
                                        uint32_t *out_width, uint32_t *out_height) {
    if (out_width == NULL || out_height == NULL || !isfinite(display_scale) ||
        display_scale < 1.0 || display_scale > 4.0) {
        return false;
    }
    uint32_t width = mp_shim_pixels_from_points(logical_size.width, display_scale);
    uint32_t height = mp_shim_pixels_from_points(logical_size.height, display_scale);
    if (width == 0 || height == 0 || !mp_shim_surface_within_limit(width, height)) {
        return false;
    }
    *out_width = width;
    *out_height = height;
    return true;
}

mp_shim_status mp_shim_testing_surface_recommendation(double logical_width,
                                                       double logical_height,
                                                       double display_scale,
                                                       uint32_t *out_width,
                                                       uint32_t *out_height) {
    if (out_width == NULL || out_height == NULL) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    *out_width = 0;
    *out_height = 0;
    CGSize logical_size = CGSizeMake(logical_width, logical_height);
    return mp_shim_recommended_surface(logical_size, display_scale, out_width, out_height)
               ? MP_SHIM_OK
               : MP_SHIM_INVALID_ARGUMENT;
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

mp_shim_status mp_shim_execution_context(uint32_t *out_launch, uint32_t *out_signature,
                                         uint8_t *out_identifier, size_t identifier_capacity,
                                         size_t *out_identifier_len) {
    if (out_launch == NULL || out_signature == NULL || out_identifier == NULL ||
        out_identifier_len == NULL ||
        identifier_capacity < (size_t)MP_SHIM_MAX_SIGNING_IDENTIFIER + 1u) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    *out_launch = MP_SHIM_LAUNCH_UNKNOWN;
    *out_signature = MP_SHIM_SIGNATURE_PLATFORM_FAILURE;
    *out_identifier_len = 0;
    out_identifier[0] = 0;
    MP_SHIM_BEGIN
    NSString *identifier = [NSBundle mainBundle].bundleIdentifier;
    *out_launch = identifier.length > 0 ? MP_SHIM_LAUNCH_BUNDLED : MP_SHIM_LAUNCH_UNBUNDLED;
    mp_shim_read_signature_context(out_signature, out_identifier, out_identifier_len);
    return MP_SHIM_OK;
    MP_SHIM_END
}

mp_shim_status mp_shim_testing_classify_signature(int32_t signing_info_status,
                                                  int32_t validity_status,
                                                  bool has_identifier, uint32_t signature_flags,
                                                  uint32_t *out_signature) {
    if (out_signature == NULL) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    *out_signature = mp_shim_classify_signature((OSStatus)signing_info_status,
                                                (OSStatus)validity_status, has_identifier,
                                                signature_flags);
    return MP_SHIM_OK;
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

    /*
     * A window whose owner the framework does not name is not listed at all.
     *
     * `owningApplication` is optional, and macOS recycles window numbers. The
     * retained filter is the actual selection; the owner is descriptive metadata
     * repeated at open so a mismatched request and filter are rejected. A window
     * without that metadata is refused here rather than handed to a boundary whose
     * declared request shape cannot represent it.
     *
     * The cost was measured rather than assumed: on the verification host every
     * on-screen, layer-zero window the framework reported had a named owner. Those two
     * filters are why — a window at layer zero and on screen is an ordinary application
     * window. Zero is therefore a value no listed window carries, which is what lets an
     * open reject it outright.
     */
    id owner = window.owningApplication;
    if (owner == nil) {
        return nil;
    }
    NSString *title = window.title;
    NSString *name = title.length > 0 ? title : nil;
    if (name == nil) {
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
    entry.nativeTarget = window;
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
    entry.nativeTarget = display;
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

mp_shim_status mp_shim_inventory_target(const mp_shim_inventory *inventory, size_t index,
                                        mp_shim_target **out) {
    if (out == NULL) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    *out = NULL;
    MP_SHIM_BEGIN
    MPShimInventoryEntry *entry = mp_shim_inventory_at(inventory, index);
    if (entry == nil || entry.nativeTarget == nil) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    const MPShimFramework *framework = mp_shim_capture_framework();
    if (framework == NULL) {
        return MP_SHIM_UNSUPPORTED;
    }
    mp_shim_target_info info = entry.info;
    id<MPShimContentFilterInit> filter = nil;
    if (info.kind == MP_SHIM_TARGET_WINDOW) {
        filter = [(id<MPShimContentFilterInit>)[framework->content_filter alloc]
            initWithDesktopIndependentWindow:entry.nativeTarget];
    } else if (info.kind == MP_SHIM_TARGET_DISPLAY) {
        filter = [(id<MPShimContentFilterInit>)[framework->content_filter alloc]
              initWithDisplay:entry.nativeTarget
             excludingWindows:@[]];
    }
    if (filter == nil) {
        return MP_SHIM_PLATFORM_FAILURE;
    }
    struct mp_shim_target *target = calloc(1, sizeof(struct mp_shim_target));
    if (target == NULL) {
        return MP_SHIM_PLATFORM_FAILURE;
    }
    target->magic = MP_SHIM_TARGET_MAGIC;
    target->kind = info.kind;
    target->native_id = info.native_id;
    target->owner_process = info.owner_process;
    target->filter = CFBridgingRetain(filter);
    mp_shim_note_owned();
    *out = target;
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

void mp_shim_target_release(mp_shim_target *target) {
    if (target == NULL || target->magic != MP_SHIM_TARGET_MAGIC) {
        return;
    }
    @try {
        if (target->filter != NULL) {
            CFRelease(target->filter);
            target->filter = NULL;
            mp_shim_note_released();
        }
        target->magic = 0;
    } @catch (NSException *exception) {
        (void)exception;
    } @catch (...) {
    }
    free(target);
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
 * Stops frame admission and delivers one typed terminal report.
 *
 * The compare-and-exchange is the exactly-once decision shared by producer stop,
 * a contained native frame exception, and a non-OK Rust frame callback status.
 * No admission/native/pool mutex is held while the host callback runs. This helper
 * contains the complete terminal trampoline because it is also called from
 * `@finally`, where an exception would not be caught by the surrounding `@catch`.
 */
static void mp_shim_session_terminalize(struct mp_shim_session *session,
                                        mp_shim_status status) {
    @try {
        mp_shim_admission_stop(&session->admission);
        bool expected = false;
        if (atomic_compare_exchange_strong(&session->stop_reported, &expected, true) &&
            session->stopped_callback != NULL) {
            @try {
                session->stopped_callback(session->callback_context, status);
            } @catch (NSException *exception) {
                (void)exception;
            } @catch (...) {
            }
        }
    } @catch (NSException *exception) {
        (void)exception;
    } @catch (...) {
    }
}

mp_shim_status mp_shim_testing_terminalize_twice(
    void *context, void (*stopped_callback)(void *context, mp_shim_status status),
    mp_shim_status first, mp_shim_status second) {
    if (stopped_callback == NULL || first == MP_SHIM_OK || second == MP_SHIM_OK) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    struct mp_shim_session session = {0};
    mp_shim_admission_init(&session.admission);
    atomic_init(&session.stop_reported, false);
    session.callback_context = context;
    session.stopped_callback = stopped_callback;
    mp_shim_session_terminalize(&session, first);
    mp_shim_session_terminalize(&session, second);
    mp_shim_admission_destroy(&session.admission);
    return MP_SHIM_OK;
}

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
        mp_shim_status terminal = MP_SHIM_OK;
        @try {
            terminal = [self deliver:sampleBuffer session:session];
        } @catch (NSException *exception) {
            (void)exception;
            terminal = MP_SHIM_NATIVE_EXCEPTION;
        } @catch (...) {
            terminal = MP_SHIM_NATIVE_EXCEPTION;
        } @finally {
            @try {
                if (terminal != MP_SHIM_OK) {
                    mp_shim_session_terminalize(session, terminal);
                }
            } @finally {
                /* Decremented here so a thrown exception cannot strand the fence. */
                mp_shim_admission_leave(&session->admission);
            }
        }
    }
}

- (mp_shim_status)deliver:(CMSampleBufferRef)sampleBuffer
                   session:(struct mp_shim_session *)session {
    const MPShimFramework *framework = mp_shim_capture_framework();
    if (framework == NULL || sampleBuffer == NULL || !CMSampleBufferIsValid(sampleBuffer)) {
        return MP_SHIM_OK;
    }
    CFArrayRef attachments = CMSampleBufferGetSampleAttachmentsArray(sampleBuffer, false);
    if (attachments == NULL || CFArrayGetCount(attachments) == 0) {
        return MP_SHIM_OK;
    }
    NSDictionary *attachment =
        (__bridge NSDictionary *)(CFDictionaryRef)CFArrayGetValueAtIndex(attachments, 0);
    NSNumber *status = attachment[(__bridge NSString *)framework->key_status];
    if (status == nil || status.integerValue != MPShimFrameStatusComplete) {
        return MP_SHIM_OK;
    }

    CVImageBufferRef image = CMSampleBufferGetImageBuffer(sampleBuffer);
    if (image == NULL || CVPixelBufferGetPixelFormatType(image) != kCVPixelFormatType_32BGRA) {
        return MP_SHIM_OK;
    }

    CGRect content = CGRectNull;
    NSDictionary *rect = attachment[(__bridge NSString *)framework->key_content_rect];
    if (rect == nil ||
        !CGRectMakeWithDictionaryRepresentation((__bridge CFDictionaryRef)rect, &content)) {
        return MP_SHIM_OK;
    }
    NSNumber *scale_factor = attachment[(__bridge NSString *)framework->key_scale_factor];
    NSNumber *content_scale = framework->key_content_scale == NULL
                                  ? nil
                                  : attachment[(__bridge NSString *)framework->key_content_scale];
    double factor = scale_factor == nil ? 1.0 : scale_factor.doubleValue;
    double scale = content_scale == nil ? 1.0 : content_scale.doubleValue;
    double effective_scale = factor * scale;
    if (!isfinite(factor) || factor <= 0.0 || !isfinite(scale) || scale <= 0.0 ||
        !isfinite(effective_scale) || effective_scale <= 0.0) {
        return MP_SHIM_OK;
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
        return MP_SHIM_OK;
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
        return MP_SHIM_OK;
    }
    double origin_x = floor(content.origin.x * factor);
    double origin_y = floor(content.origin.y * factor);
    if (!isfinite(origin_x) || !isfinite(origin_y) || origin_x < 0.0 || origin_y < 0.0 ||
        origin_x > (double)MP_SHIM_MAX_PIXEL_EXTENT || origin_y > (double)MP_SHIM_MAX_PIXEL_EXTENT) {
        return MP_SHIM_OK;
    }
    uint32_t content_width = (uint32_t)floor(pixel_width);
    uint32_t content_height = (uint32_t)floor(pixel_height);
    if ((size_t)origin_x + content_width > surface_width ||
        (size_t)origin_y + content_height > surface_height) {
        return MP_SHIM_OK;
    }

    /* `screenRect` is the only placement attached to these exact pixels. A
     * shareable-content snapshot acquired later is deliberately not substituted.
     * Missing or contradictory metadata reaches the Rust callback with no valid
     * flag, where it advances observable drop accounting without publishing. */
    CGRect screen = CGRectNull;
    NSDictionary *screen_rect = attachment[(__bridge NSString *)framework->key_screen_rect];
    bool screen_valid = screen_rect != nil &&
                        CGRectMakeWithDictionaryRepresentation(
                            (__bridge CFDictionaryRef)screen_rect, &screen) &&
                        isfinite(screen.origin.x) && isfinite(screen.origin.y) &&
                        isfinite(screen.size.width) && isfinite(screen.size.height) &&
                        screen.size.width > 0.0 && screen.size.height > 0.0 &&
                        fabs(screen.origin.x) <= MP_SHIM_MAX_DESKTOP_COORDINATE &&
                        fabs(screen.origin.y) <= MP_SHIM_MAX_DESKTOP_COORDINATE &&
                        screen.size.width <= (double)MP_SHIM_MAX_PIXEL_EXTENT &&
                        screen.size.height <= (double)MP_SHIM_MAX_PIXEL_EXTENT;
    if (screen_valid) {
        double logical_width = (double)content_width / effective_scale;
        double logical_height = (double)content_height / effective_scale;
        /* Less than one logical point allows only the quantization already
         * introduced by the framework's pixel rectangle. A full point is a real
         * resize and must not be attached to these pixels. */
        screen_valid = fabs(logical_width - screen.size.width) < 1.0 &&
                       fabs(logical_height - screen.size.height) < 1.0;
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
         * and every frame's public time collapsed onto the clock origin. */
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
    info.scale_factor = effective_scale;
    info.content_origin_x = origin_x;
    info.content_origin_y = origin_y;
    if (screen_valid) {
        info.flags |= MP_SHIM_FRAME_INFO_SCREEN_RECT;
        info.screen_x = screen.origin.x;
        info.screen_y = screen.origin.y;
        info.screen_width = screen.size.width;
        info.screen_height = screen.size.height;
        if (session->kind == MP_SHIM_TARGET_WINDOW && scale_factor != nil) {
            uint32_t recommended_width = 0;
            uint32_t recommended_height = 0;
            if (mp_shim_recommended_surface(screen.size, factor, &recommended_width,
                                            &recommended_height)) {
                info.flags |= MP_SHIM_FRAME_INFO_SURFACE_RECOMMENDATION;
                info.recommended_surface_width = recommended_width;
                info.recommended_surface_height = recommended_height;
            }
        }
    }

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
    mp_shim_status callback_status = MP_SHIM_OK;
    if (session->frame_callback != NULL) {
        /* No shim lock is held here: invoking a host callback under one is how a
         * deadlock between the producer and a consumer is built. */
        callback_status = session->frame_callback(session->callback_context, &borrowed, &info);
    }
    /* The borrow ends with the call, so the handle stops being usable here. */
    borrowed.magic = 0;
    borrowed.buffer = NULL;
    if ((session->testing_raise_sites & MP_SHIM_RAISE_AFTER_CALLBACK) != 0) {
        [NSException raise:@"MPShimInjectedFailure" format:@"after frame callback returned"];
    }
    if (callback_status != MP_SHIM_OK) {
        return callback_status;
    }
    /*
     * The stage callback owns no publication authority. This is deliberately the
     * last fallible frame operation: a native exception above terminalizes the
     * session and lets the stopped callback discard the staged frame before any
     * waiter can observe it. The commit trampoline contains Rust panics and no
     * framework message follows a successful publication.
     */
    if (session->frame_commit_callback != NULL) {
        return session->frame_commit_callback(session->callback_context);
    }
    return MP_SHIM_INVALID_ARGUMENT;
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
        mp_shim_session_terminalize(session, mp_shim_error_status(error));
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
        request->frame_callback == NULL || request->frame_commit_callback == NULL ||
        request->target == NULL ||
        request->target->magic != MP_SHIM_TARGET_MAGIC || request->target->filter == NULL ||
        request->target->kind != request->kind ||
        request->target->native_id != request->native_id ||
        request->target->owner_process != request->owner_process || request->pixel_width == 0 ||
        request->pixel_height == 0 || request->pixel_width > MP_SHIM_MAX_PIXEL_EXTENT ||
        request->pixel_height > MP_SHIM_MAX_PIXEL_EXTENT ||
        !mp_shim_surface_within_limit(request->pixel_width, request->pixel_height) ||
        request->detached_budget == 0 ||
        request->detached_budget > MP_SHIM_MAX_DETACHED_BUDGET ||
        (request->kind != MP_SHIM_TARGET_WINDOW && request->kind != MP_SHIM_TARGET_DISPLAY) ||
        /*
         * A window request carries the owning process the caller resolved it against,
         * and discovery lists no window without one, so a non-positive value is either a
         * caller that invented an identity or one carrying a fingerprint from before
         * this rule. Refused rather than matched: zero used to match another window
         * whose owner was equally unknown, which is the recycled-number capture the
         * owner check exists to prevent.
         */
        (request->kind == MP_SHIM_TARGET_WINDOW && request->owner_process <= 0)) {
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
    /* The originating inventory already constructed this exact filter. No fresh
     * wrapper, numeric identifier, or process lookup is consulted here. */
    id<MPShimContentFilterInit> filter = (__bridge id)request->target->filter;

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
    mp_shim_start_gate_init(&session->start_gate);
    mp_shim_stop_gate_init(&session->stop_gate);
    pthread_mutex_init(&session->close_mutex, NULL);
    pthread_cond_init(&session->close_idle, NULL);
    pthread_mutex_init(&session->native_mutex, NULL);
    pthread_mutex_init(&session->pool_mutex, NULL);
    session->kind = request->kind;
    session->native_id = request->native_id;
    session->detached_budget = request->detached_budget;
    session->callback_context = request->callback_context;
    session->frame_callback = request->frame_callback;
    session->frame_commit_callback = request->frame_commit_callback;
    session->stopped_callback = request->stopped_callback;
    session->testing_raise_sites = request->testing_raise_sites;
    session->testing_start_delay_nanos = request->testing_start_delay_nanos;
    session->testing_stop_delay_nanos = request->testing_stop_delay_nanos;
    atomic_store(&session->output_added, false);
    atomic_store(&session->started, false);
    atomic_store(&session->closing, false);
    atomic_store(&session->closed, false);
    atomic_store(&session->stop_reported, false);
    session->close_active = false;
    session->close_phase = MP_SHIM_CLOSE_START;
    session->close_error = MP_SHIM_OK;
    session->close_error_reported = false;

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
    if (atomic_load(&session->closing) || atomic_load(&session->closed)) {
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
     * The completion records the start's outcome rather than reporting it, because a
     * start can outlive the wait below. When it did, `started` stayed false forever,
     * close skipped its stop on that condition, and a late success left screen capture
     * running with nothing tracking it. Close now joins an in-flight start through the
     * gate, so the outcome recorded here is read by a caller holding a status — and the
     * completion stops the producer itself only when close could not wait that long.
     */
    MPShimSessionHold *hold = [[MPShimSessionHold alloc] initWithSession:session];
    void (^completion)(NSError *) = ^(NSError *error) {
      /* Captured so the session outlives this block, however the block ends and
       * whether or not the message below ever accepted it. */
      (void)hold;
      /*
       * A callback trampoline, so it contains its own exceptions the way every other
       * one in this file does. The stop below is a framework message and can raise,
       * and an exception leaving this block unwinds into the framework with no handler
       * anywhere above it — which is an abort rather than a status. The signal and the
       * gate are owed whatever happens, so they are settled in @finally; a waiter that
       * never received either would block to its own timeout for no reason.
      */
      @try {
          mp_shim_testing_delay(session->testing_start_delay_nanos);
          failure = error;
          if (error == nil) {
              atomic_store(&session->started, true);
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
          mp_shim_start_gate_end(&session->start_gate);
          dispatch_semaphore_signal(ready);
      }
    };
    /* Declared in flight before the message, so a close racing this start joins it
     * rather than deciding the producer's fate without knowing the outcome. */
    mp_shim_start_gate_begin(&session->start_gate);
    @try {
        [stream startCaptureWithCompletionHandler:completion];
    } @catch (...) {
        /*
         * The message never accepted the block, so nothing else will settle the gate —
         * and every later close would wait out its whole budget for a start that never
         * began, reporting a timeout against it. Settling here is unconditional because
         * settling twice is harmless, which is the difference from the session reference
         * this block also holds: that one is counted, so a second drop would be a free.
         */
        mp_shim_start_gate_end(&session->start_gate);
        @throw;
    }
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
    if (atomic_load(&session->closing) || atomic_load(&session->closed)) {
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
    uint64_t began = mp_shim_nanos_from_ticks(mach_absolute_time());
    uint64_t deadline = began > UINT64_MAX - timeout_nanos ? UINT64_MAX : began + timeout_nanos;
    bool complete = false;
    mp_shim_status claimed = mp_shim_close_claim(session, deadline, &complete);
    if (claimed != MP_SHIM_OK) {
        return claimed;
    }
    if (complete) {
        return MP_SHIM_OK;
    }
    atomic_store(&session->closing, true);
    mp_shim_admission_stop(&session->admission);

    while (session->close_phase != MP_SHIM_CLOSE_COMPLETE) {
        uint64_t now = mp_shim_nanos_from_ticks(mach_absolute_time());
        uint64_t remaining = now >= deadline ? 0 : deadline - now;
        if (remaining == 0 && session->close_phase != MP_SHIM_CLOSE_RELEASE) {
            mp_shim_close_release(session);
            return MP_SHIM_TIMED_OUT;
        }

        if (session->close_phase == MP_SHIM_CLOSE_START) {
            mp_shim_status settled = mp_shim_start_gate_wait(&session->start_gate, remaining);
            if (settled == MP_SHIM_TIMED_OUT) {
                mp_shim_close_release(session);
                return settled;
            }
            if (settled != MP_SHIM_OK && session->close_error == MP_SHIM_OK) {
                session->close_error = settled;
            }
            session->close_phase = MP_SHIM_CLOSE_OUTPUT;
            continue;
        }

        id<MPShimStream> stream = mp_shim_session_copy_stream(session);
        if (session->close_phase == MP_SHIM_CLOSE_OUTPUT) {
            id output = mp_shim_session_copy_slot(session, &session->output);
            if (stream != nil && atomic_load(&session->output_added)) {
                @try {
                    NSError *removed = nil;
                    if (![stream removeStreamOutput:output
                                              type:MPShimStreamOutputTypeScreen
                                             error:&removed] &&
                        session->close_error == MP_SHIM_OK) {
                        session->close_error = mp_shim_error_status(removed);
                    }
                } @catch (NSException *exception) {
                    (void)exception;
                    if (session->close_error == MP_SHIM_OK) {
                        session->close_error = MP_SHIM_NATIVE_EXCEPTION;
                    }
                } @catch (...) {
                    if (session->close_error == MP_SHIM_OK) {
                        session->close_error = MP_SHIM_NATIVE_EXCEPTION;
                    }
                }
                atomic_store(&session->output_added, false);
            }
            session->close_phase = MP_SHIM_CLOSE_STOP;
            continue;
        }

        if (session->close_phase == MP_SHIM_CLOSE_STOP) {
            if (stream != nil && atomic_load(&session->started) &&
                !mp_shim_stop_gate_pending(&session->stop_gate)) {
                mp_shim_stop_gate_begin(&session->stop_gate);
                @try {
                    MPShimSessionHold *hold = [[MPShimSessionHold alloc] initWithSession:session];
                    if (hold == nil) {
                        mp_shim_stop_gate_end(&session->stop_gate, MP_SHIM_PLATFORM_FAILURE);
                    } else {
                        [stream stopCaptureWithCompletionHandler:^(NSError *error) {
                          (void)hold;
                          mp_shim_complete_stop(
                              &session->stop_gate, &session->started, error,
                              session->testing_stop_delay_nanos,
                              (session->testing_raise_sites &
                               MP_SHIM_RAISE_IN_STOP_COMPLETION) != 0);
                        }];
                    }
                } @catch (NSException *exception) {
                    (void)exception;
                    mp_shim_stop_gate_end(&session->stop_gate, MP_SHIM_NATIVE_EXCEPTION);
                } @catch (...) {
                    mp_shim_stop_gate_end(&session->stop_gate, MP_SHIM_NATIVE_EXCEPTION);
                }
            }
            mp_shim_status stopped = mp_shim_stop_gate_wait(&session->stop_gate, remaining);
            if (stopped == MP_SHIM_TIMED_OUT) {
                mp_shim_close_release(session);
                return stopped;
            }
            if (stopped != MP_SHIM_OK && stopped != MP_SHIM_CLOSED &&
                stopped != MP_SHIM_STOPPED_BY_USER && stopped != MP_SHIM_STOPPED_BY_SYSTEM &&
                session->close_error == MP_SHIM_OK) {
                session->close_error = stopped;
            }
            session->close_phase = MP_SHIM_CLOSE_FENCE;
            continue;
        }

        if (session->close_phase == MP_SHIM_CLOSE_FENCE) {
            mp_shim_status fenced = mp_shim_admission_fence(&session->admission, remaining);
            if (fenced == MP_SHIM_TIMED_OUT) {
                mp_shim_close_release(session);
                return fenced;
            }
            if (fenced != MP_SHIM_OK && session->close_error == MP_SHIM_OK) {
                session->close_error = fenced;
            }
            session->close_phase = MP_SHIM_CLOSE_RELEASE;
            continue;
        }

        /* Release is non-blocking and always runs to completion once entered. */
        if ((session->testing_raise_sites & MP_SHIM_RAISE_AT_TEARDOWN) != 0) {
            @try {
                [NSException raise:@"MPShimInjectedFailure" format:@"teardown"];
            } @catch (NSException *exception) {
                (void)exception;
                if (session->close_error == MP_SHIM_OK) {
                    session->close_error = MP_SHIM_NATIVE_EXCEPTION;
                }
            } @catch (...) {
                if (session->close_error == MP_SHIM_OK) {
                    session->close_error = MP_SHIM_NATIVE_EXCEPTION;
                }
            }
        }

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
            @try {
                CFRelease(released[slot]);
                mp_shim_note_released();
            } @catch (NSException *exception) {
                (void)exception;
                if (session->close_error == MP_SHIM_OK) {
                    session->close_error = MP_SHIM_NATIVE_EXCEPTION;
                }
            } @catch (...) {
                if (session->close_error == MP_SHIM_OK) {
                    session->close_error = MP_SHIM_NATIVE_EXCEPTION;
                }
            }
        }
        pthread_mutex_lock(&session->pool_mutex);
        @try {
            mp_shim_pool_release_locked(session);
        } @catch (NSException *exception) {
            (void)exception;
            if (session->close_error == MP_SHIM_OK) {
                session->close_error = MP_SHIM_NATIVE_EXCEPTION;
            }
        } @catch (...) {
            if (session->close_error == MP_SHIM_OK) {
                session->close_error = MP_SHIM_NATIVE_EXCEPTION;
            }
        } @finally {
            pthread_mutex_unlock(&session->pool_mutex);
        }
        session->close_phase = MP_SHIM_CLOSE_COMPLETE;
        atomic_store(&session->closed, true);
    }

    mp_shim_status reported = MP_SHIM_OK;
    if (!session->close_error_reported && session->close_error != MP_SHIM_OK) {
        session->close_error_reported = true;
        reported = session->close_error;
    }
    mp_shim_close_release(session);
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
        mp_shim_status closed = mp_shim_session_close(session, MP_SHIM_DEFAULT_TIMEOUT_NANOS);
        if (closed == MP_SHIM_TIMED_OUT) {
            /* A void release has no caller operation to wait under. Preserve the
             * incomplete phase in a bounded Drop quarantine instead of releasing
             * native state underneath its completion. */
            mp_shim_session_retain(session);
            dispatch_async(dispatch_get_global_queue(QOS_CLASS_UTILITY, 0), ^{
              while (mp_shim_session_close(session, MP_SHIM_DEFAULT_TIMEOUT_NANOS) ==
                     MP_SHIM_TIMED_OUT) {
              }
              mp_shim_session_unref(session);
            });
        }
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

#pragma mark - Input: the two frameworks it loads rather than links

/*
 * Neither AppKit nor HIToolbox is imported, for the reason at the top of this
 * file: a header import creates a load command, and this Adapter is a headless
 * library that must not depend on the desktop UI framework or on Carbon to load.
 * Both are opened from an absolute system path on first use and their entry
 * points resolved by symbol, so a host that cannot provide one reports
 * MP_SHIM_UNSUPPORTED for exactly the operation that needed it.
 */

typedef struct MPShimOpaqueInputSource *MPShimInputSourceRef;
typedef MPShimInputSourceRef (*MPShimCopyKeyboardLayoutSource)(void);
typedef void *(*MPShimInputSourceProperty)(MPShimInputSourceRef source, CFStringRef key);
typedef OSStatus (*MPShimKeyTranslate)(const void *layout, UInt16 key_code, UInt16 action,
                                       UInt32 modifier_state, UInt32 keyboard_type,
                                       OptionBits options, UInt32 *dead_key_state,
                                       UniCharCount capacity, UniCharCount *out_length,
                                       UniChar *out_units);

/* UCKeyAction.kUCKeyActionDown and the no-dead-keys translate option. */
static const UInt16 MPShimKeyActionDown = 0;
static const OptionBits MPShimKeyTranslateNoDeadKeys = 1u << 0;
/* How many UTF-16 units one key may legitimately produce on any layout. */
#define MP_SHIM_LAYOUT_UNIT_CAPACITY 8u
/* Hardware key codes are seven bits wide on every keyboard the layout describes. */
#define MP_SHIM_LAYOUT_KEY_CODES 128u
/* Upper bound for one active-display enumeration. Matches the geometry helper's. */
#define MP_SHIM_MAX_ACTIVE_DISPLAYS 16u

typedef struct MPShimKeyboardLayoutApi {
    bool loaded;
    MPShimCopyKeyboardLayoutSource copy_current;
    MPShimCopyKeyboardLayoutSource copy_ascii_capable;
    MPShimInputSourceProperty property;
    MPShimKeyTranslate translate;
    CFStringRef unicode_layout_key;
} MPShimKeyboardLayoutApi;

static MPShimKeyboardLayoutApi mp_shim_layout_api;
static pthread_once_t mp_shim_layout_once = PTHREAD_ONCE_INIT;

static void mp_shim_load_keyboard_layout_api(void) {
    void *handle = dlopen("/System/Library/Frameworks/Carbon.framework/Frameworks/"
                          "HIToolbox.framework/Versions/A/HIToolbox",
                          RTLD_LAZY | RTLD_LOCAL);
    if (handle == NULL) {
        handle = dlopen("/System/Library/Frameworks/Carbon.framework/Frameworks/"
                        "HIToolbox.framework/HIToolbox",
                        RTLD_LAZY | RTLD_LOCAL);
    }
    if (handle == NULL) {
        return;
    }

    MPShimKeyboardLayoutApi loaded;
    memset(&loaded, 0, sizeof(loaded));
    loaded.copy_current = (MPShimCopyKeyboardLayoutSource)dlsym(
        handle, "TISCopyCurrentKeyboardLayoutInputSource");
    loaded.copy_ascii_capable = (MPShimCopyKeyboardLayoutSource)dlsym(
        handle, "TISCopyCurrentASCIICapableKeyboardLayoutInputSource");
    loaded.property = (MPShimInputSourceProperty)dlsym(handle, "TISGetInputSourceProperty");
    loaded.translate = (MPShimKeyTranslate)dlsym(handle, "UCKeyTranslate");
    loaded.unicode_layout_key =
        (CFStringRef)mp_shim_string_symbol(handle, "kTISPropertyUnicodeKeyLayoutData");

    loaded.loaded = loaded.copy_current != NULL && loaded.property != NULL &&
                    loaded.translate != NULL && loaded.unicode_layout_key != NULL;
    if (loaded.loaded) {
        mp_shim_layout_api = loaded;
    }
}

static const MPShimKeyboardLayoutApi *mp_shim_keyboard_layout_api(void) {
    pthread_once(&mp_shim_layout_once, mp_shim_load_keyboard_layout_api);
    return mp_shim_layout_api.loaded ? &mp_shim_layout_api : NULL;
}

@protocol MPShimRunningApplicationClass <NSObject>
+ (id)runningApplicationWithProcessIdentifier:(pid_t)identifier;
@end

@protocol MPShimActivatableApplication <NSObject>
- (BOOL)activateWithOptions:(NSUInteger)options;
@end

/*
 * NSApplicationActivateAllWindows.
 *
 * NSApplicationActivateIgnoringOtherApps is deliberately absent: it asks macOS to
 * take focus away from whatever the user is doing, and this Adapter reports a
 * refusal instead of overriding the system's activation policy.
 */
static const NSUInteger MPShimActivateAllWindows = 1u << 0;

static Class mp_shim_running_application_class = Nil;
static pthread_once_t mp_shim_appkit_once = PTHREAD_ONCE_INIT;

static void mp_shim_load_appkit(void) {
    void *handle =
        dlopen("/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit", RTLD_LAZY | RTLD_LOCAL);
    if (handle == NULL) {
        handle = dlopen("/System/Library/Frameworks/AppKit.framework/AppKit", RTLD_LAZY | RTLD_LOCAL);
    }
    if (handle == NULL) {
        return;
    }
    mp_shim_running_application_class = NSClassFromString(@"NSRunningApplication");
}

static Class mp_shim_activation_class(void) {
    pthread_once(&mp_shim_appkit_once, mp_shim_load_appkit);
    return mp_shim_running_application_class;
}

#pragma mark - Input: window-server observations

/* Accessibility work is bounded even if a hostile application reports many windows. */
#define MP_SHIM_MAX_ACCESSIBILITY_WINDOWS 256

static mp_shim_status mp_shim_ax_error_status(AXError error, uint64_t deadline,
                                              bool *out_missing) {
    *out_missing = false;
    switch (error) {
    case kAXErrorSuccess:
        return MP_SHIM_OK;
    case kAXErrorAPIDisabled:
        return MP_SHIM_PERMISSION_DENIED;
    case kAXErrorAttributeUnsupported:
    case kAXErrorNoValue:
    case kAXErrorNotImplemented:
    case kAXErrorInvalidUIElement:
        *out_missing = true;
        return MP_SHIM_OK;
    case kAXErrorCannotComplete:
        return mp_shim_nanos_from_ticks(mach_absolute_time()) >= deadline ? MP_SHIM_TIMED_OUT
                                                                         : MP_SHIM_PLATFORM_FAILURE;
    default:
        return MP_SHIM_PLATFORM_FAILURE;
    }
}

/* Gives one Accessibility object no more than the observation's remaining budget. */
static mp_shim_status mp_shim_ax_prepare(AXUIElementRef element, uint64_t deadline) {
    uint64_t now = mp_shim_nanos_from_ticks(mach_absolute_time());
    if (element == NULL || now >= deadline) {
        return MP_SHIM_TIMED_OUT;
    }
    float seconds = (float)((double)(deadline - now) / 1000000000.0);
    if (!(seconds > 0.0f)) {
        return MP_SHIM_TIMED_OUT;
    }
    AXError error = AXUIElementSetMessagingTimeout(element, seconds);
    if (error == kAXErrorSuccess) {
        return MP_SHIM_OK;
    }
    if (error == kAXErrorAPIDisabled) {
        return MP_SHIM_PERMISSION_DENIED;
    }
    if (error == kAXErrorCannotComplete &&
        mp_shim_nanos_from_ticks(mach_absolute_time()) >= deadline) {
        return MP_SHIM_TIMED_OUT;
    }
    return MP_SHIM_PLATFORM_FAILURE;
}

/* Copies one public attribute and distinguishes an absent value from a failed query. */
static mp_shim_status mp_shim_ax_copy_attribute(AXUIElementRef element, CFStringRef attribute,
                                                uint64_t deadline, CFTypeRef *out_value,
                                                bool *out_available) {
    *out_value = NULL;
    *out_available = false;
    mp_shim_status status = mp_shim_ax_prepare(element, deadline);
    if (status != MP_SHIM_OK) {
        return status;
    }
    AXError error = AXUIElementCopyAttributeValue(element, attribute, out_value);
    bool missing = false;
    status = mp_shim_ax_error_status(error, deadline, &missing);
    if (status != MP_SHIM_OK || missing) {
        if (*out_value != NULL) {
            CFRelease(*out_value);
            *out_value = NULL;
        }
        return status;
    }
    if (*out_value == NULL) {
        return MP_SHIM_PLATFORM_FAILURE;
    }
    *out_available = true;
    return MP_SHIM_OK;
}

/* Reads one Accessibility window rectangle in the same top-left global plane as SCWindow. */
static mp_shim_status mp_shim_ax_window_rect(AXUIElementRef window, uint64_t deadline,
                                             CGRect *out_bounds, bool *out_available) {
    CFTypeRef position = NULL;
    CFTypeRef size = NULL;
    mp_shim_status status = MP_SHIM_OK;
    *out_bounds = CGRectNull;
    *out_available = false;
    do {
        bool position_available = false;
        status = mp_shim_ax_copy_attribute(window, kAXPositionAttribute, deadline, &position,
                                           &position_available);
        if (status != MP_SHIM_OK || !position_available) {
            break;
        }
        bool size_available = false;
        status =
            mp_shim_ax_copy_attribute(window, kAXSizeAttribute, deadline, &size, &size_available);
        if (status != MP_SHIM_OK || !size_available) {
            break;
        }
        if (CFGetTypeID(position) != AXValueGetTypeID() ||
            CFGetTypeID(size) != AXValueGetTypeID() ||
            AXValueGetType((AXValueRef)position) != kAXValueCGPointType ||
            AXValueGetType((AXValueRef)size) != kAXValueCGSizeType) {
            status = MP_SHIM_PLATFORM_FAILURE;
            break;
        }
        CGPoint origin = CGPointZero;
        CGSize extent = CGSizeZero;
        if (!AXValueGetValue((AXValueRef)position, kAXValueCGPointType, &origin) ||
            !AXValueGetValue((AXValueRef)size, kAXValueCGSizeType, &extent) ||
            !isfinite(origin.x) || !isfinite(origin.y) || !isfinite(extent.width) ||
            !isfinite(extent.height) || extent.width < 1.0 || extent.height < 1.0) {
            break;
        }
        *out_bounds = CGRectMake(origin.x, origin.y, extent.width, extent.height);
        *out_available = true;
    } while (false);
    if (size != NULL) {
        CFRelease(size);
    }
    if (position != NULL) {
        CFRelease(position);
    }
    return status;
}

static mp_shim_status mp_shim_retained_window_is_live(const struct mp_shim_target *target) {
    id<MPShimContentFilterInit> filter = (__bridge id<MPShimContentFilterInit>)target->filter;
    NSArray *windows = filter.includedWindows;
    if (windows.count != 1) {
        return MP_SHIM_TARGET_LOST;
    }
    id<MPShimWindow> window = (id<MPShimWindow>)windows.firstObject;
    id<MPShimRunningApplication> owner = window.owningApplication;
    if (window == nil || owner == nil || window.windowID != (CGWindowID)target->native_id ||
        owner.processID != (pid_t)target->owner_process || !window.isOnScreen) {
        return MP_SHIM_TARGET_LOST;
    }
    return MP_SHIM_OK;
}

/*
 * `SCWindow.frame` belongs to the shareable-content snapshot that selected the
 * target; it does not advance when that same live window moves or resizes.
 * Keep the retained filter as incarnation authority, but read the exact
 * PID-qualified window number from Core Graphics for its current rectangle.
 */
static mp_shim_status mp_shim_input_window_rect(const struct mp_shim_target *target,
                                                CGRect *out_bounds) {
    mp_shim_status status = mp_shim_retained_window_is_live(target);
    if (status != MP_SHIM_OK) {
        return status;
    }

    CFArrayRef copied = CGWindowListCopyWindowInfo(kCGWindowListOptionIncludingWindow,
                                                   (CGWindowID)target->native_id);
    NSArray *descriptions = CFBridgingRelease(copied);
    if (descriptions == nil || descriptions.count != 1) {
        return MP_SHIM_TARGET_LOST;
    }
    NSDictionary *description = descriptions.firstObject;
    NSNumber *window_number = description[(__bridge NSString *)kCGWindowNumber];
    NSNumber *owner_process = description[(__bridge NSString *)kCGWindowOwnerPID];
    NSNumber *on_screen = description[(__bridge NSString *)kCGWindowIsOnscreen];
    NSDictionary *bounds_dictionary = description[(__bridge NSString *)kCGWindowBounds];
    CGRect bounds = CGRectNull;
    if (![window_number isKindOfClass:[NSNumber class]] ||
        ![owner_process isKindOfClass:[NSNumber class]] ||
        ![on_screen isKindOfClass:[NSNumber class]] ||
        ![bounds_dictionary isKindOfClass:[NSDictionary class]]) {
        return MP_SHIM_PLATFORM_FAILURE;
    }
    if (window_number.unsignedIntValue != (CGWindowID)target->native_id ||
        owner_process.longLongValue != target->owner_process || !on_screen.boolValue) {
        return MP_SHIM_TARGET_LOST;
    }
    if (!CGRectMakeWithDictionaryRepresentation((__bridge CFDictionaryRef)bounds_dictionary,
                                                &bounds) ||
        CGRectIsNull(bounds) || bounds.size.width < 1.0 || bounds.size.height < 1.0) {
        return MP_SHIM_TARGET_LOST;
    }

    /*
     * Close the lookup race: a destroyed retained window cannot authorize a
     * recycled number that appeared while Core Graphics took its snapshot.
     */
    status = mp_shim_retained_window_is_live(target);
    if (status != MP_SHIM_OK) {
        return status;
    }
    *out_bounds = bounds;
    return MP_SHIM_OK;
}

mp_shim_status mp_shim_input_target_focused(const mp_shim_target *target,
                                            uint64_t timeout_nanos, bool *out_focused) {
    if (target == NULL || target->magic != MP_SHIM_TARGET_MAGIC || target->filter == NULL ||
        target->kind != MP_SHIM_TARGET_WINDOW || target->owner_process <= 0 ||
        timeout_nanos == 0 || out_focused == NULL) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    *out_focused = false;
    MP_SHIM_BEGIN
    if (!AXIsProcessTrusted()) {
        return MP_SHIM_PERMISSION_DENIED;
    }
    CGRect target_before = CGRectNull;
    mp_shim_status status = mp_shim_input_window_rect(target, &target_before);
    if (status != MP_SHIM_OK) {
        return status;
    }
    uint64_t began = mp_shim_nanos_from_ticks(mach_absolute_time());
    uint64_t deadline =
        began > UINT64_MAX - timeout_nanos ? UINT64_MAX : began + timeout_nanos;
    AXUIElementRef application = NULL;
    CFTypeRef frontmost = NULL;
    CFTypeRef focused = NULL;
    CFArrayRef windows = NULL;
    CFTypeRef frontmost_after = NULL;
    CFTypeRef focused_after = NULL;
    @try {
        do {
            application = AXUIElementCreateApplication((pid_t)target->owner_process);
            if (application == NULL) {
                status = MP_SHIM_PLATFORM_FAILURE;
                break;
            }

            bool available = false;
            status = mp_shim_ax_copy_attribute(application, kAXFrontmostAttribute, deadline,
                                               &frontmost, &available);
            if (status != MP_SHIM_OK || !available) {
                break;
            }
            if (CFGetTypeID(frontmost) != CFBooleanGetTypeID()) {
                status = MP_SHIM_PLATFORM_FAILURE;
                break;
            }
            if (!CFBooleanGetValue((CFBooleanRef)frontmost)) {
                break;
            }

            available = false;
            status = mp_shim_ax_copy_attribute(application, kAXFocusedWindowAttribute, deadline,
                                               &focused, &available);
            if (status != MP_SHIM_OK || !available) {
                break;
            }
            if (CFGetTypeID(focused) != AXUIElementGetTypeID()) {
                status = MP_SHIM_PLATFORM_FAILURE;
                break;
            }

            status = mp_shim_ax_prepare(application, deadline);
            if (status != MP_SHIM_OK) {
                break;
            }
            CFIndex window_count = 0;
            AXError count_error =
                AXUIElementGetAttributeValueCount(application, kAXWindowsAttribute, &window_count);
            bool missing = false;
            status = mp_shim_ax_error_status(count_error, deadline, &missing);
            if (status != MP_SHIM_OK || missing || window_count < 1 ||
                window_count > MP_SHIM_MAX_ACCESSIBILITY_WINDOWS) {
                break;
            }

            status = mp_shim_ax_prepare(application, deadline);
            if (status != MP_SHIM_OK) {
                break;
            }
            AXError windows_error = AXUIElementCopyAttributeValues(
                application, kAXWindowsAttribute, 0, window_count, &windows);
            missing = false;
            status = mp_shim_ax_error_status(windows_error, deadline, &missing);
            if (status != MP_SHIM_OK || missing || windows == NULL) {
                break;
            }
            CFIndex actual_count = CFArrayGetCount(windows);
            if (actual_count < 1 || actual_count > MP_SHIM_MAX_ACCESSIBILITY_WINDOWS) {
                break;
            }

            CFIndex geometry_matches = 0;
            bool matching_window_is_focused = false;
            bool complete_snapshot = true;
            for (CFIndex index = 0; index < actual_count; index += 1) {
                CFTypeRef value = CFArrayGetValueAtIndex(windows, index);
                if (value == NULL || CFGetTypeID(value) != AXUIElementGetTypeID()) {
                    status = MP_SHIM_PLATFORM_FAILURE;
                    complete_snapshot = false;
                    break;
                }
                CGRect bounds = CGRectNull;
                bool bounds_available = false;
                status = mp_shim_ax_window_rect((AXUIElementRef)value, deadline, &bounds,
                                                &bounds_available);
                if (status != MP_SHIM_OK || !bounds_available) {
                    complete_snapshot = false;
                    break;
                }
                if (CGRectEqualToRect(bounds, target_before)) {
                    geometry_matches += 1;
                    matching_window_is_focused = CFEqual(value, focused);
                }
            }
            if (status != MP_SHIM_OK || !complete_snapshot || geometry_matches != 1 ||
                !matching_window_is_focused) {
                break;
            }

            CGRect target_after = CGRectNull;
            status = mp_shim_input_window_rect(target, &target_after);
            if (status != MP_SHIM_OK) {
                break;
            }
            if (!CGRectEqualToRect(target_before, target_after)) {
                break;
            }

            available = false;
            status = mp_shim_ax_copy_attribute(application, kAXFrontmostAttribute, deadline,
                                               &frontmost_after, &available);
            if (status != MP_SHIM_OK || !available) {
                break;
            }
            if (CFGetTypeID(frontmost_after) != CFBooleanGetTypeID()) {
                status = MP_SHIM_PLATFORM_FAILURE;
                break;
            }
            if (!CFBooleanGetValue((CFBooleanRef)frontmost_after)) {
                break;
            }

            available = false;
            status = mp_shim_ax_copy_attribute(application, kAXFocusedWindowAttribute, deadline,
                                               &focused_after, &available);
            if (status != MP_SHIM_OK || !available) {
                break;
            }
            if (CFGetTypeID(focused_after) != AXUIElementGetTypeID()) {
                status = MP_SHIM_PLATFORM_FAILURE;
                break;
            }
            if (!CFEqual(focused, focused_after)) {
                break;
            }
            *out_focused = true;
        } while (false);
    } @finally {
        if (focused_after != NULL) {
            CFRelease(focused_after);
        }
        if (frontmost_after != NULL) {
            CFRelease(frontmost_after);
        }
        if (windows != NULL) {
            CFRelease(windows);
        }
        if (focused != NULL) {
            CFRelease(focused);
        }
        if (frontmost != NULL) {
            CFRelease(frontmost);
        }
        if (application != NULL) {
            CFRelease(application);
        }
    }
    return status;
    MP_SHIM_END
}

static mp_shim_status mp_shim_input_display_rect(const struct mp_shim_target *target,
                                                 CGRect *out_bounds) {
    id<MPShimContentFilterInit> filter = (__bridge id<MPShimContentFilterInit>)target->filter;
    NSArray *displays = filter.includedDisplays;
    if (displays.count != 1) {
        return MP_SHIM_TARGET_LOST;
    }
    id<MPShimDisplay> selected = (id<MPShimDisplay>)displays.firstObject;
    if (selected == nil || selected.displayID != (CGDirectDisplayID)target->native_id) {
        return MP_SHIM_TARGET_LOST;
    }
    mp_shim_connect_window_server();
    CGDirectDisplayID display = selected.displayID;
    uint32_t count = 0;
    CGDirectDisplayID active[MP_SHIM_MAX_ACTIVE_DISPLAYS];
    if (CGGetActiveDisplayList(MP_SHIM_MAX_ACTIVE_DISPLAYS, active, &count) != kCGErrorSuccess) {
        return MP_SHIM_PLATFORM_FAILURE;
    }
    for (uint32_t index = 0; index < count; index += 1) {
        if (active[index] != display) {
            continue;
        }
        CGRect bounds = CGDisplayBounds(display);
        if (CGRectIsNull(bounds) || bounds.size.width < 1.0 || bounds.size.height < 1.0) {
            return MP_SHIM_TARGET_LOST;
        }
        *out_bounds = bounds;
        return MP_SHIM_OK;
    }
    return MP_SHIM_TARGET_LOST;
}

mp_shim_status mp_shim_input_target_bounds(const mp_shim_target *target, double *out_x,
                                           double *out_y, double *out_width, double *out_height,
                                           double *out_scale) {
    if (target == NULL || target->magic != MP_SHIM_TARGET_MAGIC || target->filter == NULL ||
        out_x == NULL || out_y == NULL || out_width == NULL || out_height == NULL ||
        out_scale == NULL) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    *out_x = 0.0;
    *out_y = 0.0;
    *out_width = 0.0;
    *out_height = 0.0;
    *out_scale = 0.0;
    MP_SHIM_BEGIN
    CGRect bounds = CGRectNull;
    mp_shim_status status;
    double scale;
    if (target->kind == MP_SHIM_TARGET_WINDOW) {
        status = mp_shim_input_window_rect(target, &bounds);
        scale = status == MP_SHIM_OK ? mp_shim_scale_for_frame(bounds) : 0.0;
    } else if (target->kind == MP_SHIM_TARGET_DISPLAY) {
        status = mp_shim_input_display_rect(target, &bounds);
        scale = status == MP_SHIM_OK
                    ? mp_shim_display_backing_scale((CGDirectDisplayID)target->native_id)
                    : 0.0;
    } else {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    if (status != MP_SHIM_OK) {
        return status;
    }
    if (!isfinite(bounds.origin.x) || !isfinite(bounds.origin.y) ||
        !isfinite(bounds.size.width) || !isfinite(bounds.size.height) || !isfinite(scale) ||
        scale <= 0.0 || fabs(bounds.origin.x) > MP_SHIM_MAX_DESKTOP_COORDINATE ||
        fabs(bounds.origin.y) > MP_SHIM_MAX_DESKTOP_COORDINATE) {
        return MP_SHIM_PLATFORM_FAILURE;
    }
    *out_x = bounds.origin.x;
    *out_y = bounds.origin.y;
    *out_width = bounds.size.width;
    *out_height = bounds.size.height;
    *out_scale = scale;
    return MP_SHIM_OK;
    MP_SHIM_END
}

mp_shim_status mp_shim_input_pointer_location(double *out_x, double *out_y) {
    if (out_x == NULL || out_y == NULL) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    *out_x = 0.0;
    *out_y = 0.0;
    MP_SHIM_BEGIN
    CGEventRef reading = CGEventCreate(NULL);
    if (reading == NULL) {
        return MP_SHIM_PLATFORM_FAILURE;
    }
    CGPoint location = CGEventGetLocation(reading);
    CFRelease(reading);
    if (!isfinite(location.x) || !isfinite(location.y)) {
        return MP_SHIM_PLATFORM_FAILURE;
    }
    *out_x = location.x;
    *out_y = location.y;
    return MP_SHIM_OK;
    MP_SHIM_END
}

mp_shim_status mp_shim_input_activate_owner(int64_t owner_process) {
    if (owner_process <= 0 || owner_process > INT32_MAX) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    MP_SHIM_BEGIN
    Class class = mp_shim_activation_class();
    if (class == Nil) {
        return MP_SHIM_UNSUPPORTED;
    }
    id<MPShimRunningApplicationClass> factory = (id<MPShimRunningApplicationClass>)class;
    id running = [factory runningApplicationWithProcessIdentifier:(pid_t)owner_process];
    if (running == nil) {
        return MP_SHIM_TARGET_LOST;
    }
    id<MPShimActivatableApplication> application = (id<MPShimActivatableApplication>)running;
    /* macOS may decline under its own activation policy. A refusal is reported;
     * nothing here retries, elevates, or overrides the user's foreground app. */
    return [application activateWithOptions:MPShimActivateAllWindows] ? MP_SHIM_OK
                                                                      : MP_SHIM_PLATFORM_FAILURE;
    MP_SHIM_END
}

#pragma mark - Input: layout resolution

static const void *mp_shim_unicode_layout(const MPShimKeyboardLayoutApi *api,
                                          MPShimInputSourceRef *out_source) {
    MPShimInputSourceRef source = api->copy_current();
    void *data = source == NULL ? NULL : api->property(source, api->unicode_layout_key);
    if (data == NULL) {
        /* An input method rather than a keyboard layout is current, and it
         * publishes no key layout. The ASCII-capable layout the system keeps
         * beside it is what a key code actually means on this host. */
        if (source != NULL) {
            CFRelease(source);
            source = NULL;
        }
        if (api->copy_ascii_capable != NULL) {
            source = api->copy_ascii_capable();
            data = source == NULL ? NULL : api->property(source, api->unicode_layout_key);
        }
    }
    if (data == NULL || CFGetTypeID((CFTypeRef)data) != CFDataGetTypeID()) {
        if (source != NULL) {
            CFRelease(source);
        }
        return NULL;
    }
    *out_source = source;
    return CFDataGetBytePtr((CFDataRef)data);
}

static uint32_t mp_shim_keyboard_type(void) {
    CGEventSourceRef source = CGEventSourceCreate(kCGEventSourceStateHIDSystemState);
    if (source == NULL) {
        return 0;
    }
    uint32_t type = (uint32_t)CGEventSourceGetKeyboardType(source);
    CFRelease(source);
    return type;
}

mp_shim_status mp_shim_input_resolve_character(uint32_t scalar, uint16_t *out_key_code) {
    if (out_key_code == NULL || scalar > 0x10FFFFu || (scalar >= 0xD800u && scalar <= 0xDFFFu)) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    *out_key_code = 0;
    MP_SHIM_BEGIN
    const MPShimKeyboardLayoutApi *api = mp_shim_keyboard_layout_api();
    if (api == NULL) {
        return MP_SHIM_UNSUPPORTED;
    }

    UniChar wanted[2];
    UniCharCount wanted_length;
    if (scalar > 0xFFFFu) {
        uint32_t offset = scalar - 0x10000u;
        wanted[0] = (UniChar)(0xD800u + (offset >> 10));
        wanted[1] = (UniChar)(0xDC00u + (offset & 0x3FFu));
        wanted_length = 2;
    } else {
        wanted[0] = (UniChar)scalar;
        wanted_length = 1;
    }

    MPShimInputSourceRef source = NULL;
    const void *layout = mp_shim_unicode_layout(api, &source);
    if (layout == NULL) {
        return MP_SHIM_UNSUPPORTED;
    }
    uint32_t keyboard_type = mp_shim_keyboard_type();
    mp_shim_status status = MP_SHIM_UNSUPPORTED;
    for (uint32_t code = 0; code < MP_SHIM_LAYOUT_KEY_CODES; code += 1) {
        UInt32 dead_key_state = 0;
        UniCharCount produced = 0;
        UniChar units[MP_SHIM_LAYOUT_UNIT_CAPACITY];
        /* Modifier state zero is the whole rule: a character this layout produces
         * only with a modifier is not a key the caller can press, and reporting it
         * as one would deliver a different character. */
        OSStatus translated =
            api->translate(layout, (UInt16)code, MPShimKeyActionDown, 0, keyboard_type,
                           MPShimKeyTranslateNoDeadKeys, &dead_key_state,
                           MP_SHIM_LAYOUT_UNIT_CAPACITY, &produced, units);
        if (translated != 0 || produced != wanted_length) {
            continue;
        }
        if (memcmp(units, wanted, (size_t)wanted_length * sizeof(UniChar)) != 0) {
            continue;
        }
        *out_key_code = (uint16_t)code;
        status = MP_SHIM_OK;
        break;
    }
    if (source != NULL) {
        CFRelease(source);
    }
    return status;
    MP_SHIM_END
}

#pragma mark - Input: posting

static CGEventFlags mp_shim_input_event_flags(uint32_t flags) {
    CGEventFlags result = 0;
    if ((flags & MP_SHIM_INPUT_FLAG_SHIFT) != 0) {
        result |= kCGEventFlagMaskShift;
    }
    if ((flags & MP_SHIM_INPUT_FLAG_CONTROL) != 0) {
        result |= kCGEventFlagMaskControl;
    }
    if ((flags & MP_SHIM_INPUT_FLAG_ALT) != 0) {
        result |= kCGEventFlagMaskAlternate;
    }
    if ((flags & MP_SHIM_INPUT_FLAG_META) != 0) {
        result |= kCGEventFlagMaskCommand;
    }
    return result;
}

static bool mp_shim_input_mouse_button(uint32_t button, CGMouseButton *out_button) {
    switch (button) {
    case MP_SHIM_INPUT_BUTTON_PRIMARY:
        *out_button = kCGMouseButtonLeft;
        return true;
    case MP_SHIM_INPUT_BUTTON_SECONDARY:
        *out_button = kCGMouseButtonRight;
        return true;
    case MP_SHIM_INPUT_BUTTON_MIDDLE:
        *out_button = kCGMouseButtonCenter;
        return true;
    default:
        return false;
    }
}

static bool mp_shim_input_pointer_type(uint32_t action, uint32_t button, CGEventType *out_type) {
    if (action == MP_SHIM_INPUT_POINTER_MOVE) {
        /* A move while this sequence holds a button is a drag. Reporting it as a
         * plain move would leave every drag gesture inert. */
        switch (button) {
        case MP_SHIM_INPUT_BUTTON_NONE:
            *out_type = kCGEventMouseMoved;
            return true;
        case MP_SHIM_INPUT_BUTTON_PRIMARY:
            *out_type = kCGEventLeftMouseDragged;
            return true;
        case MP_SHIM_INPUT_BUTTON_SECONDARY:
            *out_type = kCGEventRightMouseDragged;
            return true;
        case MP_SHIM_INPUT_BUTTON_MIDDLE:
            *out_type = kCGEventOtherMouseDragged;
            return true;
        default:
            return false;
        }
    }
    bool pressed = action == MP_SHIM_INPUT_POINTER_PRESS;
    if (!pressed && action != MP_SHIM_INPUT_POINTER_RELEASE) {
        return false;
    }
    switch (button) {
    case MP_SHIM_INPUT_BUTTON_PRIMARY:
        *out_type = pressed ? kCGEventLeftMouseDown : kCGEventLeftMouseUp;
        return true;
    case MP_SHIM_INPUT_BUTTON_SECONDARY:
        *out_type = pressed ? kCGEventRightMouseDown : kCGEventRightMouseUp;
        return true;
    case MP_SHIM_INPUT_BUTTON_MIDDLE:
        *out_type = pressed ? kCGEventOtherMouseDown : kCGEventOtherMouseUp;
        return true;
    default:
        return false;
    }
}

mp_shim_status mp_shim_input_post_pointer(uint32_t action, uint32_t button, uint64_t click_state,
                                          double x, double y, uint32_t flags) {
    if (!isfinite(x) || !isfinite(y) || fabs(x) > MP_SHIM_MAX_DESKTOP_COORDINATE ||
        fabs(y) > MP_SHIM_MAX_DESKTOP_COORDINATE || click_state > MP_SHIM_INPUT_MAX_CLICK_STATE) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    MP_SHIM_BEGIN
    CGEventType type;
    if (!mp_shim_input_pointer_type(action, button, &type)) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    CGMouseButton mouse_button = kCGMouseButtonLeft;
    if (button != MP_SHIM_INPUT_BUTTON_NONE && !mp_shim_input_mouse_button(button, &mouse_button)) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    CGEventRef event = CGEventCreateMouseEvent(NULL, type, CGPointMake(x, y), mouse_button);
    if (event == NULL) {
        return MP_SHIM_PLATFORM_FAILURE;
    }
    if (action != MP_SHIM_INPUT_POINTER_MOVE) {
        CGEventSetIntegerValueField(event, kCGMouseEventClickState, (int64_t)click_state);
    }
    /* The flags are set rather than merged: a sequence delivers the modifiers it
     * pressed, and inheriting whatever the user is holding would change the
     * keystroke a caller asked for into a different one. */
    CGEventSetFlags(event, mp_shim_input_event_flags(flags));
    CGEventPost(kCGHIDEventTap, event);
    CFRelease(event);
    return MP_SHIM_OK;
    MP_SHIM_END
}

mp_shim_status mp_shim_input_post_scroll(int32_t horizontal, int32_t vertical, uint32_t flags) {
    if (horizontal == 0 && vertical == 0) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    if (horizontal < -MP_SHIM_INPUT_MAX_SCROLL_LINES ||
        horizontal > MP_SHIM_INPUT_MAX_SCROLL_LINES || vertical < -MP_SHIM_INPUT_MAX_SCROLL_LINES ||
        vertical > MP_SHIM_INPUT_MAX_SCROLL_LINES) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    MP_SHIM_BEGIN
    /* Core Graphics counts a positive vertical wheel value as scrolling the
     * content up and a positive horizontal value as scrolling it left. The
     * platform-neutral contract counts positive as down and right, so both axes
     * are negated exactly here rather than at each caller. */
    CGEventRef event = CGEventCreateScrollWheelEvent2(NULL, kCGScrollEventUnitLine, 2, -vertical,
                                                      -horizontal, 0);
    if (event == NULL) {
        return MP_SHIM_PLATFORM_FAILURE;
    }
    CGEventSetFlags(event, mp_shim_input_event_flags(flags));
    CGEventPost(kCGHIDEventTap, event);
    CFRelease(event);
    return MP_SHIM_OK;
    MP_SHIM_END
}

mp_shim_status mp_shim_input_post_key(uint16_t key_code, bool down, uint32_t flags) {
    if (key_code >= MP_SHIM_LAYOUT_KEY_CODES) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    MP_SHIM_BEGIN
    CGEventRef event = CGEventCreateKeyboardEvent(NULL, (CGKeyCode)key_code, down);
    if (event == NULL) {
        return MP_SHIM_PLATFORM_FAILURE;
    }
    CGEventSetFlags(event, mp_shim_input_event_flags(flags));
    CGEventPost(kCGHIDEventTap, event);
    CFRelease(event);
    return MP_SHIM_OK;
    MP_SHIM_END
}

typedef struct {
    CGEventRef (*create)(bool down, void *context);
    void (*configure)(CGEventRef event, const uint16_t *units, size_t count,
                      CGEventFlags flags, void *context);
    void (*post)(CGEventRef event, void *context);
    void (*release)(CGEventRef event, void *context);
    void *context;
} mp_shim_text_event_ops;

static CGEventRef mp_shim_text_event_create(bool down, void *context) {
    (void)context;
    return CGEventCreateKeyboardEvent(NULL, 0, down);
}

static void mp_shim_text_event_configure(CGEventRef event, const uint16_t *units, size_t count,
                                         CGEventFlags flags, void *context) {
    (void)context;
    CGEventKeyboardSetUnicodeString(event, (UniCharCount)count, (const UniChar *)units);
    CGEventSetFlags(event, flags);
}

static void mp_shim_text_event_post(CGEventRef event, void *context) {
    (void)context;
    CGEventPost(kCGHIDEventTap, event);
}

static void mp_shim_text_event_release(CGEventRef event, void *context) {
    (void)context;
    CFRelease(event);
}

static mp_shim_status mp_shim_input_post_text_with_ops(const uint16_t *units, size_t count,
                                                       uint32_t flags, size_t *out_posted,
                                                       const mp_shim_text_event_ops *ops) {
    CGEventRef down = NULL;
    CGEventRef up = NULL;
    @try {
        down = ops->create(true, ops->context);
        if (down == NULL) {
            return MP_SHIM_PLATFORM_FAILURE;
        }
        up = ops->create(false, ops->context);
        if (up == NULL) {
            return MP_SHIM_PLATFORM_FAILURE;
        }

        CGEventFlags event_flags = mp_shim_input_event_flags(flags);
        /* Prepare the whole balanced pair before either half can reach the
         * system. A second allocation or either configuration failure therefore
         * leaves `out_posted` at zero and posts nothing. */
        ops->configure(down, units, count, event_flags, ops->context);
        ops->configure(up, units, count, event_flags, ops->context);
        ops->post(down, ops->context);
        *out_posted = count;
        ops->post(up, ops->context);
        return MP_SHIM_OK;
    } @finally {
        /* `@finally` covers ordinary returns and a contained native exception;
         * every successfully allocated Core Foundation object is released. */
        if (up != NULL) {
            ops->release(up, ops->context);
        }
        if (down != NULL) {
            ops->release(down, ops->context);
        }
    }
}

mp_shim_status mp_shim_input_post_text(const uint16_t *units, size_t count, uint32_t flags,
                                       size_t *out_posted) {
    if (out_posted == NULL) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    *out_posted = 0;
    if (units == NULL || count == 0 || count > MP_SHIM_INPUT_MAX_TEXT_CHUNK) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    MP_SHIM_BEGIN
    /* A key code of zero with an attached string is how Core Graphics delivers
     * text rather than a key press, which is what the Text operation means. */
    const mp_shim_text_event_ops ops = {
        .create = mp_shim_text_event_create,
        .configure = mp_shim_text_event_configure,
        .post = mp_shim_text_event_post,
        .release = mp_shim_text_event_release,
        .context = NULL,
    };
    return mp_shim_input_post_text_with_ops(units, count, flags, out_posted, &ops);
    MP_SHIM_END
}

typedef struct {
    size_t allocations;
    size_t configurations;
    size_t posts;
    size_t releases;
} mp_shim_text_failure_probe;

static CGEventRef mp_shim_testing_text_create(bool down, void *context) {
    (void)down;
    mp_shim_text_failure_probe *probe = context;
    probe->allocations += 1;
    if (probe->allocations == 2) {
        return NULL;
    }
    return (CGEventRef)(uintptr_t)1;
}

static void mp_shim_testing_text_configure(CGEventRef event, const uint16_t *units, size_t count,
                                           CGEventFlags flags, void *context) {
    (void)event;
    (void)units;
    (void)count;
    (void)flags;
    mp_shim_text_failure_probe *probe = context;
    probe->configurations += 1;
}

static void mp_shim_testing_text_post(CGEventRef event, void *context) {
    (void)event;
    mp_shim_text_failure_probe *probe = context;
    probe->posts += 1;
}

static void mp_shim_testing_text_release(CGEventRef event, void *context) {
    (void)event;
    mp_shim_text_failure_probe *probe = context;
    probe->releases += 1;
}

mp_shim_status mp_shim_testing_input_text_second_allocation_failure(
    mp_shim_status *out_delivery_status, size_t *out_allocations, size_t *out_configurations,
    size_t *out_posts, size_t *out_releases, size_t *out_posted) {
    if (out_delivery_status == NULL || out_allocations == NULL || out_configurations == NULL ||
        out_posts == NULL || out_releases == NULL || out_posted == NULL) {
        return MP_SHIM_INVALID_ARGUMENT;
    }
    *out_delivery_status = MP_SHIM_PLATFORM_FAILURE;
    *out_allocations = 0;
    *out_configurations = 0;
    *out_posts = 0;
    *out_releases = 0;
    *out_posted = 0;
    MP_SHIM_BEGIN
    mp_shim_text_failure_probe probe = {0};
    const mp_shim_text_event_ops ops = {
        .create = mp_shim_testing_text_create,
        .configure = mp_shim_testing_text_configure,
        .post = mp_shim_testing_text_post,
        .release = mp_shim_testing_text_release,
        .context = &probe,
    };
    const uint16_t unit = (uint16_t)'x';
    size_t posted = 0;
    mp_shim_status delivery =
        mp_shim_input_post_text_with_ops(&unit, 1, 0, &posted, &ops);
    *out_delivery_status = delivery;
    *out_allocations = probe.allocations;
    *out_configurations = probe.configurations;
    *out_posts = probe.posts;
    *out_releases = probe.releases;
    *out_posted = posted;
    return MP_SHIM_OK;
    MP_SHIM_END
}
