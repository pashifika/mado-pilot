/*
 * MadoPilot macOS native shim: the one internal C-callable boundary.
 *
 * This surface is internal to `mado-pilot-platform-macos`. It is not a public
 * MadoPilot ABI, is not installed, and is not covered by the C ABI compatibility
 * policy in docs/c-abi.md. Its rules come from
 * docs/adr/0012-macos-shim-language-and-containment.md:
 *
 *   - opaque handles, size-versioned request and report structures, a status
 *     return on every entry point, and output values written through validated
 *     pointers;
 *   - no Objective-C, Core Foundation, Core Video, or ScreenCaptureKit type
 *     appears here, so no native type reaches a Rust seam;
 *   - every entry point and callback trampoline contains native exceptions;
 *   - frames handed to a callback are borrowed for the duration of that call;
 *   - callback admission is fenced by disable-and-drain before a caller may
 *     release the state it registered;
 *   - close is idempotent and releasing a handle is a separate operation.
 */

#ifndef MADOPILOT_MACOS_SHIM_H
#define MADOPILOT_MACOS_SHIM_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* The version of this internal surface. Rust asserts it at load. */
#define MP_SHIM_ABI_VERSION 19u

/* The largest extent, budget, and default wait the shim will accept or apply. */
#define MP_SHIM_MAX_PIXEL_EXTENT 32768u
#define MP_SHIM_MAX_DETACHED_BUDGET 256u
#define MP_SHIM_DEFAULT_TIMEOUT_NANOS 1000000000ull
#define MP_SHIM_MAX_NATIVE_WAIT_NANOS 2000000000ull

/*
 * The largest surface the shim will accept, in bytes.
 *
 * The per-axis limit above is what bounds the conversions; it does not bound the
 * allocation, and the two are far apart. A target at the axis limit on both sides is
 * 32768 x 32768 BGRA, which is four gibibytes for a single surface, and a session
 * holds a producer queue and a detached budget of them.
 *
 * An 8K display at BGRA8 is 132,710,400 bytes. This is the next power of two above
 * it, so every target a host can really present passes and no single surface reaches
 * a gibibyte. Bounding a whole session rather than one surface would mean expressing
 * the detached budget in bytes, which is a public queue-policy question and belongs
 * to the `G-013` budgets with the bound it would redefine.
 */
#define MP_SHIM_MAX_SURFACE_BYTES 268435456u

/* Finite bound for signed coordinates in the global logical desktop plane. */
#define MP_SHIM_MAX_DESKTOP_COORDINATE 1000000000.0

/*
 * Session-scoped test seams.
 *
 * A caller may ask a session to raise a native exception at one of the boundary
 * positions ADR 0012 measured, which is how the containment and failure-path
 * ownership cases become testable. Nothing in the product sets them, and a session
 * that leaves them zero behaves as if they did not exist.
 *
 * The completion seams cover callback trampolines the framework invokes after the
 * registering entry point has returned. A catch around that entry point cannot
 * contain a later exception, so each completion owns its own containment and owed
 * gate settlement.
 */
#define MP_SHIM_RAISE_AT_START 1u
#define MP_SHIM_RAISE_BEFORE_CALLBACK 2u
#define MP_SHIM_RAISE_AFTER_CALLBACK 4u
#define MP_SHIM_RAISE_AT_TEARDOWN 8u
#define MP_SHIM_RAISE_IN_START_COMPLETION 16u
/* Rust-only companion seam: the Rust trampoline panics before processing. */
#define MP_SHIM_PANIC_IN_RUST_CALLBACK 32u
#define MP_SHIM_RAISE_IN_STOP_COMPLETION 64u
/* Rust-only companion seam: one callback outlives the default fence wait. */
#define MP_SHIM_DELAY_IN_RUST_CALLBACK 128u
/* Native allocation-failure seams; zero in every product request. */
#define MP_SHIM_FAIL_START_SEMAPHORE_ALLOCATION 256u
#define MP_SHIM_FAIL_START_HOLD_ALLOCATION 512u
#define MP_SHIM_FAIL_RECONFIGURE_SEMAPHORE_ALLOCATION 1024u

/* Status returned by every entry point. Zero is success. */
typedef uint32_t mp_shim_status;

#define MP_SHIM_OK 0u
/* A pointer, length, structure size, or enumerated value failed validation. */
#define MP_SHIM_INVALID_ARGUMENT 1u
/* This host does not offer the capability at all. */
#define MP_SHIM_UNSUPPORTED 2u
/* The operating system refused for want of authorization. */
#define MP_SHIM_PERMISSION_DENIED 3u
/* The window or display existed when discovered and does not now. */
#define MP_SHIM_TARGET_LOST 4u
/* The platform reported a failure none of the above explains. */
#define MP_SHIM_PLATFORM_FAILURE 5u
/* A native exception was contained at this boundary. */
#define MP_SHIM_NATIVE_EXCEPTION 6u
/* The session stopped accepting work. */
#define MP_SHIM_CLOSED 7u
/* A bounded native wait reached the caller's budget. */
#define MP_SHIM_TIMED_OUT 8u
/* Every unit of the session's detached-storage budget is leased. */
#define MP_SHIM_BUDGET_EXHAUSTED 9u
/* A pooled buffer could not be filled because the producer surface moved. */
#define MP_SHIM_FRAME_INCOMPLETE 10u
/* The user stopped the stream through a system control. Nothing failed. */
#define MP_SHIM_STOPPED_BY_USER 11u
/* The operating system ended the stream without naming a cause. */
#define MP_SHIM_STOPPED_BY_SYSTEM 12u
/* Authoritative target geometry changed after event preparation. */
#define MP_SHIM_GEOMETRY_CHANGED 13u
/* A caller-selected focus predicate was false at the final authority gate. */
#define MP_SHIM_FOCUS_REQUIRED 14u

/* What a non-prompting authorization probe established. */
#define MP_SHIM_PERMISSION_GRANTED 0u
#define MP_SHIM_PERMISSION_NOT_GRANTED 1u
#define MP_SHIM_PERMISSION_UNAVAILABLE 2u
#define MP_SHIM_PERMISSION_UNKNOWN 3u

/* The bundle-launch context a permission answer was read in. */
#define MP_SHIM_LAUNCH_UNKNOWN 0u
/* A main bundle with an identifier: the context Apple grants per application. */
#define MP_SHIM_LAUNCH_BUNDLED 1u
/* A bare executable, whose grant follows the launching process instead. */
#define MP_SHIM_LAUNCH_UNBUNDLED 2u

/* The independently verified signature mode of the running code. */
#define MP_SHIM_SIGNATURE_PLATFORM_FAILURE 0u
#define MP_SHIM_SIGNATURE_UNSIGNED 1u
#define MP_SHIM_SIGNATURE_INVALID 2u
/* A structurally valid signature sealed without a certificate identity. */
#define MP_SHIM_SIGNATURE_AD_HOC 3u
/* A structurally valid signature backed by a certificate identity. */
#define MP_SHIM_SIGNATURE_CERTIFICATE_BACKED 4u

/* Finite UTF-8 bound for a signing identifier returned to a deliberate reporter. */
#define MP_SHIM_MAX_SIGNING_IDENTIFIER 255u
/* Maximum public Security.framework unique code identity length. */
#define MP_SHIM_EXECUTABLE_IDENTITY_CAPACITY 32u

/* What kind of desktop object a target is. */
#define MP_SHIM_TARGET_WINDOW 0u
#define MP_SHIM_TARGET_DISPLAY 1u

/* `mp_shim_target_info.flags`: process-directed input passed snapshot admission. */
#define MP_SHIM_TARGET_INFO_PROCESS_DIRECTED 1u

/* The only pixel layout this shim publishes. */
#define MP_SHIM_PIXEL_BGRA8 0u

/* `screen_*` in `mp_shim_frame_info` contains a validated per-frame rectangle. */
#define MP_SHIM_FRAME_INFO_SCREEN_RECT 1u
/* A same-sample, bounded producer-capacity recommendation is present. */
#define MP_SHIM_FRAME_INFO_SURFACE_RECOMMENDATION (1u << 1)

/* Opaque handles. Each has a complete lifecycle below. */
typedef struct mp_shim_inventory mp_shim_inventory;
typedef struct mp_shim_target mp_shim_target;
typedef struct mp_shim_session mp_shim_session;
typedef struct mp_shim_frame mp_shim_frame;
typedef struct mp_shim_process_event_source mp_shim_process_event_source;
typedef struct mp_shim_prepared_input mp_shim_prepared_input;
typedef struct mp_shim_fixture_application mp_shim_fixture_application;

/*
 * One discovered window or display.
 *
 * Geometry is reported in the global point space both Core Graphics window
 * bounds and display bounds use: a top-left origin on the main display, with
 * signed coordinates for anything above or to the left of it.
 */
typedef struct mp_shim_target_info {
    /* Set by the caller to sizeof(mp_shim_target_info) it was compiled against. */
    uint32_t struct_size;
    uint32_t kind;
    /* CGWindowID or CGDirectDisplayID. Opaque to Rust apart from equality. */
    uint64_t native_id;
    /* Owning process, or zero for a display. */
    int64_t owner_process;
    uint32_t pixel_width;
    uint32_t pixel_height;
    double logical_x;
    double logical_y;
    double logical_width;
    double logical_height;
    /* Capture pixels per point for the target's display. */
    double backing_scale;
    /* Byte length of the UTF-8 name available from mp_shim_inventory_name. */
    uint32_t name_len;
    uint32_t flags;
} mp_shim_target_info;

/* The layout and frame-time geometry of one produced frame. */
typedef struct mp_shim_frame_info {
    uint32_t struct_size;
    uint32_t pixel_format;
    /* Extent of the valid content, in capture pixels. */
    uint32_t content_width;
    uint32_t content_height;
    /* Extent of the producer surface the content sits in, in capture pixels. */
    uint32_t surface_width;
    uint32_t surface_height;
    /* Which optional frame-attached reports below passed validation. */
    uint32_t flags;
    /* Declared rather than left to padding, so the layout is the same on both
     * sides of the boundary without either side inferring it. */
    uint32_t reserved;
    /* Producer timestamp, in nanoseconds on the host's monotonic clock — the same
     * clock and unit `mp_shim_monotonic_nanos` reports, so the two are directly
     * comparable. The framework supplies mach absolute units; the shim converts. */
    uint64_t display_time_nanos;
    /* Capture pixels per point for this frame. */
    double scale_factor;
    /* Raw display backing pixels per point for final live-window comparison.
     * Unlike `scale_factor`, this excludes SCStreamFrameInfoContentScale. */
    double backing_scale;
    /* Content origin within the surface, in capture pixels. */
    double content_origin_x;
    double content_origin_y;
    /* Onscreen content rectangle from SCStreamFrameInfoScreenRect, in the
     * framework's logical coordinate plane. This is authoritative only when
     * MP_SHIM_FRAME_INFO_SCREEN_RECT is set. */
    double screen_x;
    double screen_y;
    double screen_width;
    double screen_height;
    /* Source-resolution producer capacity derived from this sample's validated
     * screenRect and raw SCStreamFrameInfoScaleFactor. It has no publication
     * geometry authority and is valid only with the matching flag. */
    uint32_t recommended_surface_width;
    uint32_t recommended_surface_height;
} mp_shim_frame_info;

/* What a session is being opened for. */
typedef struct mp_shim_open_request {
    uint32_t struct_size;
    uint32_t kind;
    uint64_t native_id;
    /*
     * The owning process recorded for this window, or zero for a display.
     *
     * macOS recycles window numbers, so neither number nor process names an
     * incarnation. Capture consumes the retained selection below. Input uses
     * these values only to narrow a fresh search whose logical SCWindow must
     * equal the retained object. Zero is valid only for a display.
     */
    int64_t owner_process;
    /*
     * The retained SCContentFilter constructed transactionally from the candidate
     * in its originating inventory snapshot. Capture open consumes it directly;
     * input compares its SCWindow with a fresh bounded shareable-content snapshot.
     */
    const mp_shim_target *target;
    uint32_t pixel_width;
    uint32_t pixel_height;
    /* Producer queue depth. Bounded by the shim to a reviewed range. */
    uint32_t queue_depth;
    /* How many detached buffers the session may lease at once. */
    uint32_t detached_budget;
    /* Test-only completion delays. Zero in the product. */
    uint64_t testing_start_delay_nanos;
    uint64_t testing_stop_delay_nanos;
    /* Zero in the product. See the MP_SHIM_RAISE_* test seams above. */
    uint32_t testing_raise_sites;
    bool shows_cursor;
    uint8_t reserved[3];
    /* Passed unchanged to both callbacks. Never dereferenced by the shim. */
    void *callback_context;
    /*
     * Invoked on the session's sample queue with a borrowed frame. The frame is
     * valid only for the duration of the call; retaining it is
     * mp_shim_frame_detach and nothing else. The callback must not let a Rust
     * panic escape.
     */
    mp_shim_status (*frame_callback)(void *context, mp_shim_frame *borrowed,
                                    const mp_shim_frame_info *info);
    /*
     * Invoked only after frame_callback returned success and every remaining
     * throwing native frame step completed. It commits or safely ignores the
     * frame staged by frame_callback and must contain Rust panics.
     */
    mp_shim_status (*frame_commit_callback)(void *context);
    /* Invoked once if the producer stops for a reason of its own. */
    void (*stopped_callback)(void *context, mp_shim_status status);
} mp_shim_open_request;

/* Returns MP_SHIM_ABI_VERSION as the linked shim was compiled with it. */
uint32_t mp_shim_abi_version(void);

/*
 * Reports the sizes the linked shim compiled its structures to.
 *
 * Rust mirrors these declarations by hand, so a test asserts the two agree
 * rather than trusting that they do.
 */
mp_shim_status mp_shim_struct_sizes(uint32_t *out_target_info, uint32_t *out_frame_info,
                                    uint32_t *out_open_request,
                                    uint32_t *out_process_authority,
                                    uint32_t *out_process_post_request,
                                    uint32_t *out_process_post_report);

/*
 * Reports offsets for every process-post pointer/count field whose placement
 * changed or was renamed in ABI 9. Rust compares these with its hand-written
 * mirrors before exposing any native capability.
 */
mp_shim_status mp_shim_process_struct_offsets(
    uint32_t *out_authority_target_match_count, uint32_t *out_request_target,
    uint32_t *out_request_event_source, uint32_t *out_request_timeout_nanos,
    uint32_t *out_report_target_match_count, uint32_t *out_report_invoked_native_units);

/*
 * Reports whether this host offers the capture capability at all.
 *
 * Loads the capture framework from its absolute system location and checks the
 * operating-system version gate. Never prompts and never presents UI.
 */
mp_shim_status mp_shim_capture_available(void);

/* Reads the Screen Recording authorization without requesting it. */
mp_shim_status mp_shim_probe_screen_capture(uint32_t *out_state);

/* Reads the Accessibility authorization without requesting it. */
mp_shim_status mp_shim_probe_accessibility(uint32_t *out_state);

/*
 * Reports the separate bundle-launch and signature contexts the probes above
 * were read in.
 *
 * Signature inspection dynamically loads the public Security.framework
 * SecCode API. A missing API or an unreadable result is represented by
 * MP_SHIM_SIGNATURE_PLATFORM_FAILURE while preserving the independently read
 * launch axis. A signing identifier is returned only for a valid ad-hoc or
 * certificate-backed signature. `identifier_capacity` must be at least
 * MP_SHIM_MAX_SIGNING_IDENTIFIER + 1; the returned bytes are UTF-8 and are also
 * NUL-terminated for native callers, while `out_identifier_len` excludes the
 * terminator.
 */
mp_shim_status mp_shim_execution_context(uint32_t *out_launch, uint32_t *out_signature,
                                         uint8_t *out_identifier, size_t identifier_capacity,
                                         size_t *out_identifier_len);

/*
 * Returns the validity-first Security.framework unique identity for one
 * canonical executable path or one kernel-issued audit-token process lifetime.
 */
mp_shim_status mp_shim_executable_identity_for_path(
    const uint8_t *path, size_t path_len, uint8_t *out_identity,
    size_t identity_capacity, size_t *out_identity_len);
mp_shim_status mp_shim_executable_identity_for_audit_token(
    const uint32_t *audit_token, size_t audit_token_count,
    uint8_t *out_identity, size_t identity_capacity,
    size_t *out_identity_len);
mp_shim_status mp_shim_executable_identity_for_process(
    uint32_t process_id, uint8_t *out_identity, size_t identity_capacity,
    size_t *out_identity_len);

/*
 * Classifies one capture-framework error code as this shim maps it.
 *
 * Exposed so the mapping table can be asserted per code rather than reached only
 * through a stream that has to be made to fail. Codes outside the framework's own
 * error domain are not classified here; the entry points that receive an error
 * object check the domain first.
 */
mp_shim_status mp_shim_classify_stream_error(int64_t code);

/*
 * Reads the host clock the producer timestamps frames on, in nanoseconds.
 *
 * The Adapter calibrates this against its own monotonic clock once, so a frame's
 * producer timestamp becomes a project timestamp without consulting either clock
 * again per frame.
 */
mp_shim_status mp_shim_monotonic_nanos(uint64_t *out_nanos);

/*
 * Reports how many native objects this shim owns process-wide.
 *
 * This exists for the ownership cases ADR 0012 requires: a contained failure
 * must leave none of the objects it had taken alive.
 */
mp_shim_status mp_shim_live_objects(uint64_t *out_live);

/*
 * Deterministic test seam for the production exactly-once terminal helper.
 * Calls terminalization twice; `stopped_callback` must observe only `first`.
 */
mp_shim_status mp_shim_testing_terminalize_twice(
    void *context, void (*stopped_callback)(void *context, mp_shim_status status),
    mp_shim_status first, mp_shim_status second);

/* Exercises stream timestamp range checks and stop-callback containment. */
uint64_t mp_shim_testing_seconds_to_nanos(double seconds);
mp_shim_status mp_shim_testing_stop_callback_exception(
    mp_shim_status *out_terminal_status, uint32_t *out_terminal_calls,
    mp_shim_status *out_fence_status);

/*
 * Deterministic test seam for the resumable asynchronous start/stop gates.
 * Each first wait expires while the delayed completion remains pending; each
 * second wait resumes that same gate and observes its completion.
 */
mp_shim_status mp_shim_testing_gate_retries(
    uint64_t completion_delay_nanos, uint64_t first_wait_nanos, uint64_t second_wait_nanos,
    mp_shim_status *out_start_first, mp_shim_status *out_start_second,
    mp_shim_status *out_stop_first, mp_shim_status *out_stop_second);

/*
 * Runs the production stop-completion trampoline with an injected exception.
 * The returned status proves that the exception was translated and the stop gate
 * was settled instead of remaining pending.
 */
mp_shim_status mp_shim_testing_stop_completion_exception(mp_shim_status *out_status,
                                                          bool *out_started);

/*
 * Exercises every synchronization-object construction boundary used by a
 * session. A nonzero `fail_at` injects failure at that one-based pthread init
 * attempt; zero initializes and then destroys the complete set.
 */
#define MP_SHIM_TEST_SESSION_SYNC_INIT_STAGES 10u
mp_shim_status mp_shim_testing_session_sync_init(
    uint32_t fail_at, uint32_t *out_attempts, uint32_t *out_initialized,
    uint32_t *out_destroyed, uint32_t *out_success);

/*
 * Runs the exact resource factories used by asynchronous production paths with
 * deterministic allocation failure enabled.
 */
mp_shim_status mp_shim_testing_resource_allocation_failures(
    mp_shim_status *out_semaphore_status, mp_shim_status *out_session_hold_status);

/* Pure seam for the same-sample source-resolution surface recommendation. */
mp_shim_status mp_shim_testing_surface_recommendation(double logical_width,
                                                      double logical_height,
                                                      double display_scale,
                                                      uint32_t *out_width,
                                                      uint32_t *out_height);

/*
 * Proves target materialization keeps its capture filter and retained owner
 * when public AppKit process-lifetime metadata is unavailable.
 */
mp_shim_status mp_shim_testing_target_without_process_lifetime(
    uint32_t *out_capture_metadata_retained, uint32_t *out_process_metadata_retained);

/*
 * Proves activation uses one retained target and refuses a process-lifetime
 * replacement observed immediately after the activation attempt.
 */
mp_shim_status mp_shim_testing_input_activation_lifetime_loss(
    mp_shim_status *out_activation_status, uint32_t *out_validation_calls,
    uint32_t *out_activation_calls);

/* Scenarios for one-unit system-event exception containment. */
#define MP_SHIM_TEST_INPUT_SINGLE_CONFIGURE_EXCEPTION 0u
#define MP_SHIM_TEST_INPUT_SINGLE_POST_EXCEPTION 1u

/*
 * Runs the production one-unit ownership helper with a contained exception
 * before or after its irreversible posting threshold.
 */
mp_shim_status mp_shim_testing_input_single_event_failure(
    uint32_t scenario, mp_shim_status *out_delivery_status, size_t *out_configurations,
    size_t *out_posts, size_t *out_releases, size_t *out_posted);

/* Scenarios for balanced system text preparation and posting failures. */
#define MP_SHIM_TEST_INPUT_TEXT_SECOND_ALLOCATION_FAILURE 0u
#define MP_SHIM_TEST_INPUT_TEXT_CONFIGURE_EXCEPTION 1u
#define MP_SHIM_TEST_INPUT_TEXT_POST_EXCEPTION 2u

/*
 * Runs the production text-event ownership helper with the selected allocation
 * failure or contained configuration/post exception.
 */
mp_shim_status mp_shim_testing_input_text_failure(
    uint32_t scenario, mp_shim_status *out_delivery_status, size_t *out_allocations,
    size_t *out_configurations, size_t *out_posts, size_t *out_releases,
    size_t *out_posted);

/* Scenarios for the prepared system-event final native fence. */
#define MP_SHIM_TEST_PREPARED_INPUT_SUCCESS 0u
#define MP_SHIM_TEST_PREPARED_INPUT_CANCELLED 1u
#define MP_SHIM_TEST_PREPARED_INPUT_DEADLINE 2u
#define MP_SHIM_TEST_PREPARED_INPUT_POST_EXCEPTION 3u

mp_shim_status mp_shim_testing_prepared_input_gate(
    uint32_t scenario, mp_shim_status *out_delivery_status,
    uint32_t *out_native_effect_may_have_occurred, uint64_t *out_post_calls,
    size_t *out_next_index);

/*
 * Classifies one required Accessibility read without consulting another
 * process. Scenarios cover success, disabled API, missing-value variants, and
 * an incomplete response in that order.
 */
mp_shim_status mp_shim_testing_required_ax_error_status(uint32_t scenario);

/* Deterministic scenarios for the complete foreground/pointer environment sample. */
#define MP_SHIM_TEST_INPUT_ENVIRONMENT_STABLE_IDENTITY 0u
#define MP_SHIM_TEST_INPUT_ENVIRONMENT_PID_CHANGE 1u
#define MP_SHIM_TEST_INPUT_ENVIRONMENT_LAUNCH_TIME_CHANGE 2u
#define MP_SHIM_TEST_INPUT_ENVIRONMENT_APPLICATION_CHANGE 3u
#define MP_SHIM_TEST_INPUT_ENVIRONMENT_POINTER_FAILURE 4u
#define MP_SHIM_TEST_INPUT_ENVIRONMENT_SECOND_LIFETIME_FAILURE 5u

/*
 * Runs the production sampling sequence through injected operations. A failed
 * sample leaves its process, launch time, and pointer coordinates zero. The
 * operation trace appends nibbles 1 (frontmost), 2 (lifetime), and 3 (pointer).
 */
mp_shim_status mp_shim_testing_input_environment(
    uint32_t scenario, mp_shim_status *out_sampling_status, int64_t *out_process,
    double *out_process_launch_time, double *out_pointer_x, double *out_pointer_y,
    uint32_t *out_frontmost_calls, uint32_t *out_lifetime_calls,
    uint32_t *out_pointer_calls, uint32_t *out_operation_trace);

/*
 * Raises from the native event-source release operation and proves the opaque
 * wrapper still completes its ownership cleanup without crossing the C boundary.
 */
mp_shim_status mp_shim_testing_process_event_source_release_exception(
    uint32_t *out_release_calls, uint32_t *out_cleanup_completed);

/* Deterministic event-source factory failures: native source, then wrapper. */
#define MP_SHIM_TEST_PROCESS_EVENT_SOURCE_NATIVE_FAILURE 0u
#define MP_SHIM_TEST_PROCESS_EVENT_SOURCE_WRAPPER_FAILURE 1u

mp_shim_status mp_shim_testing_process_event_source_allocation_failure(
    uint32_t scenario, mp_shim_status *out_creation_status,
    uint32_t *out_source_is_null, uint32_t *out_create_calls,
    uint32_t *out_allocation_calls, uint32_t *out_release_calls);

/*
 * Raises while releasing one retained target object and proves every later
 * object and the opaque wrapper still complete their ownership cleanup.
 */
mp_shim_status mp_shim_testing_target_release_exception(
    uint32_t raise_slot, uint32_t *out_release_calls, uint32_t *out_cleanup_completed);

/* Scenarios for the deterministic process-post state-machine seam. */
#define MP_SHIM_TEST_PROCESS_SUCCESS 0u
#define MP_SHIM_TEST_PROCESS_PERMISSION_DENIED 1u
#define MP_SHIM_TEST_PROCESS_TARGET_LOST 2u
#define MP_SHIM_TEST_PROCESS_WINDOW_UNAVAILABLE 3u
#define MP_SHIM_TEST_PROCESS_INVALID_EVENT 4u
#define MP_SHIM_TEST_PROCESS_NATIVE_EXCEPTION 5u
#define MP_SHIM_TEST_PROCESS_API_UNAVAILABLE 6u
#define MP_SHIM_TEST_PROCESS_REVOKED_AFTER_FIRST 7u
#define MP_SHIM_TEST_PROCESS_GEOMETRY_CHANGED 8u
#define MP_SHIM_TEST_PROCESS_TARGET_LOST_AFTER_FIRST 9u
#define MP_SHIM_TEST_PROCESS_LIFETIME_LOST_BEFORE_POST 10u
#define MP_SHIM_TEST_PROCESS_INTERRUPTED_BEFORE_POST 11u
#define MP_SHIM_TEST_PROCESS_CONSTRUCTION_FAILED 12u
#define MP_SHIM_TEST_PROCESS_INTERRUPTED_AFTER_FIRST 13u
#define MP_SHIM_TEST_PROCESS_INTERRUPTED_AFTER_PREPARE 14u
#define MP_SHIM_TEST_PROCESS_RELEASE_WINDOW_UNAVAILABLE 15u
#define MP_SHIM_TEST_PROCESS_TARGET_LOST_AFTER_PREPARE 16u
#define MP_SHIM_TEST_PROCESS_REVOKED_AFTER_PREPARE 17u
#define MP_SHIM_TEST_PROCESS_LIFETIME_LOST_AFTER_PREPARE 18u
#define MP_SHIM_TEST_PROCESS_GEOMETRY_CHANGED_AFTER_PREPARE 19u
#define MP_SHIM_TEST_PROCESS_POST_EXCEPTION 20u
#define MP_SHIM_TEST_PROCESS_FOCUS_REFUSED 21u
#define MP_SHIM_TEST_PROCESS_FOCUS_LOST_AFTER_PREPARE 22u
#define MP_SHIM_TEST_PROCESS_FOCUS_UNAVAILABLE 23u
#define MP_SHIM_TEST_PROCESS_FOCUS_REQUIRED_SUCCESS 24u
#define MP_SHIM_TEST_PROCESS_GEOMETRY_RESTORED_BEFORE_COMMIT 25u
#define MP_SHIM_TEST_PROCESS_FRACTIONAL_GEOMETRY_NORMALIZED 26u
#define MP_SHIM_TEST_PROCESS_DEADLINE_AFTER_PREPARE 27u
#define MP_SHIM_TEST_PROCESS_TARGET_LOST_AFTER_FOCUS 28u
#define MP_SHIM_TEST_PROCESS_INTERRUPTION_INVALIDATES_LIFETIME 29u
#define MP_SHIM_TEST_PROCESS_CANCELLED_DURING_LIFETIME 30u
#define MP_SHIM_TEST_PROCESS_NATIVE_BUDGET_AFTER_AUTHORITY 31u
#define MP_SHIM_TEST_PROCESS_NATIVE_BUDGET_AFTER_LIFETIME 32u
#define MP_SHIM_TEST_PROCESS_RELEASE_EXCEPTION 33u
#define MP_SHIM_TEST_PROCESS_FOCUS_LOST_DURING_AUTHORITY 34u
#define MP_SHIM_TEST_PROCESS_GEOMETRY_CHANGED_DURING_FOCUS 35u
#define MP_SHIM_TEST_PROCESS_GEOMETRY_MOVED_WITHOUT_REQUIRE_UNCHANGED 36u

/* Process-post request and capture-only target-shape validation scenarios. */
#define MP_SHIM_TEST_PROCESS_VALIDATE_NULL_REQUEST 0u
#define MP_SHIM_TEST_PROCESS_VALIDATE_REQUEST_PREFIX 1u
#define MP_SHIM_TEST_PROCESS_VALIDATE_REPORT_PREFIX 2u
#define MP_SHIM_TEST_PROCESS_VALIDATE_TARGET_NULL 3u
#define MP_SHIM_TEST_PROCESS_VALIDATE_TARGET_MAGIC 4u
#define MP_SHIM_TEST_PROCESS_VALIDATE_TARGET_KIND 5u
#define MP_SHIM_TEST_PROCESS_VALIDATE_TARGET_NATIVE_ID 6u
#define MP_SHIM_TEST_PROCESS_VALIDATE_TARGET_PROCESS 7u
#define MP_SHIM_TEST_PROCESS_VALIDATE_TARGET_FILTER 8u
#define MP_SHIM_TEST_PROCESS_VALIDATE_TARGET_OWNER 9u
#define MP_SHIM_TEST_PROCESS_VALIDATE_TARGET_LIFETIME 10u
#define MP_SHIM_TEST_PROCESS_VALIDATE_TARGET_LAUNCH 11u
#define MP_SHIM_TEST_PROCESS_VALIDATE_SOURCE_NULL 12u
#define MP_SHIM_TEST_PROCESS_VALIDATE_SOURCE_MAGIC 13u
#define MP_SHIM_TEST_PROCESS_VALIDATE_SOURCE_VALUE 14u
#define MP_SHIM_TEST_PROCESS_VALIDATE_INTERRUPTION_CONTEXT 15u
#define MP_SHIM_TEST_PROCESS_VALIDATE_INTERRUPTION_CALLBACK 16u
#define MP_SHIM_TEST_PROCESS_VALIDATE_TIMEOUT 17u
#define MP_SHIM_TEST_PROCESS_VALIDATE_FLAGS 18u
#define MP_SHIM_TEST_PROCESS_VALIDATE_GEOMETRY_POLICY 19u
#define MP_SHIM_TEST_PROCESS_VALIDATE_RESERVED 20u
#define MP_SHIM_TEST_PROCESS_VALIDATE_GEOMETRY_BOUNDS 21u
#define MP_SHIM_TEST_PROCESS_VALIDATE_POINTER_COORDINATE 22u
#define MP_SHIM_TEST_PROCESS_VALIDATE_POINTER_ACTION 23u
#define MP_SHIM_TEST_PROCESS_VALIDATE_POINTER_BUTTON 24u
#define MP_SHIM_TEST_PROCESS_VALIDATE_POINTER_CLICK 25u
#define MP_SHIM_TEST_PROCESS_VALIDATE_SCROLL_ZERO 26u
#define MP_SHIM_TEST_PROCESS_VALIDATE_SCROLL_RANGE 27u
#define MP_SHIM_TEST_PROCESS_VALIDATE_KEY_CODE 28u
#define MP_SHIM_TEST_PROCESS_VALIDATE_KEY_GEOMETRY 29u
#define MP_SHIM_TEST_PROCESS_VALIDATE_TEXT_POINTER 30u
#define MP_SHIM_TEST_PROCESS_VALIDATE_TEXT_COUNT 31u
#define MP_SHIM_TEST_PROCESS_VALIDATE_TEXT_UTF16 32u
#define MP_SHIM_TEST_PROCESS_VALIDATE_EVENT_KIND 33u
#define MP_SHIM_TEST_PROCESS_VALIDATE_PURPOSE 34u
#define MP_SHIM_TEST_PROCESS_VALIDATE_OUTPUT_NULL 35u
#define MP_SHIM_TEST_PROCESS_VALIDATE_SCROLL_COORDINATE 36u
#define MP_SHIM_TEST_PROCESS_VALIDATE_FOCUS_REQUIREMENT 37u
#define MP_SHIM_TEST_PROCESS_VALIDATE_RELEASE_FOCUS 38u
#define MP_SHIM_TEST_PROCESS_VALIDATE_CANCELLATION_CONTEXT 39u
#define MP_SHIM_TEST_PROCESS_VALIDATE_CANCELLATION_CALLBACK 40u

/*
 * Runs the production process-post state machine with deterministic authority,
 * preflight, event, and release callbacks. `out_delivery_status` is the state
 * machine result; this function itself reports only seam argument failures.
 */
mp_shim_status mp_shim_testing_process_post(
    uint32_t scenario, mp_shim_status *out_delivery_status, uint64_t *out_invoked_native_units,
    uint32_t *out_native_effect_may_have_occurred, uint32_t *out_target_match_count,
    uint32_t *out_focus_result, uint64_t *out_authority_calls, uint64_t *out_preflight_calls,
    uint64_t *out_lifetime_calls, uint64_t *out_focus_calls, uint64_t *out_prepare_calls,
    uint64_t *out_post_calls, uint64_t *out_release_calls, uint64_t *out_checkpoint_calls,
    uint64_t *out_cancellation_calls);

/*
 * Runs one invalid request through the public native entry point. A valid report
 * prefix is reset before every request-side failure.
 */
mp_shim_status mp_shim_testing_validate_process_post(
    uint32_t scenario, mp_shim_status *out_delivery_status,
    uint32_t *out_target_match_count, uint64_t *out_invoked_native_units,
    uint32_t *out_native_effect_may_have_occurred);

/* Scenarios for retained process/window identity and owning-process authority. */
#define MP_SHIM_TEST_AUTHORITY_SUCCESS 0u
#define MP_SHIM_TEST_AUTHORITY_PROCESS_REPLACED 1u
#define MP_SHIM_TEST_AUTHORITY_PROCESS_RESTARTED 2u
#define MP_SHIM_TEST_AUTHORITY_PROCESS_TERMINATED 3u
#define MP_SHIM_TEST_AUTHORITY_WINDOW_REPLACED 4u
#define MP_SHIM_TEST_AUTHORITY_EXTRA_WINDOW 5u
#define MP_SHIM_TEST_AUTHORITY_MINIMIZED 6u
#define MP_SHIM_TEST_AUTHORITY_OWNER_REPLACED 7u
#define MP_SHIM_TEST_AUTHORITY_WINDOW_MISSING 8u
#define MP_SHIM_TEST_AUTHORITY_AUXILIARY_WINDOW 9u
#define MP_SHIM_TEST_AUTHORITY_DUPLICATE_WINDOW 10u

/* Deterministically evaluates retained-window and owning-process identity. */
mp_shim_status mp_shim_testing_process_authority_rules(
    uint32_t scenario, mp_shim_status *out_authority_status,
    uint32_t *out_target_match_count);

/* Deterministically exercises the production signature-state classifier. */
mp_shim_status mp_shim_testing_classify_signature(int32_t signing_info_status,
                                                  int32_t validity_status,
                                                  bool has_identifier, uint32_t signature_flags,
                                                  uint32_t *out_signature);

/*
 * Enumerates the currently shareable windows and displays.
 *
 * `timeout_nanos` bounds the one native asynchronous query this performs. The
 * query is not interruptible once started, so the caller passes a slice of its
 * own remaining budget. Presents no picker.
 */
mp_shim_status mp_shim_inventory_acquire(uint64_t timeout_nanos, mp_shim_inventory **out);

/* Returns how many targets the inventory holds. */
mp_shim_status mp_shim_inventory_count(const mp_shim_inventory *inventory, size_t *out_count);

/* Writes the entry at `index`. `out_info->struct_size` is validated first. */
mp_shim_status mp_shim_inventory_entry(const mp_shim_inventory *inventory, size_t index,
                                       mp_shim_target_info *out_info);

/*
 * Borrows the UTF-8 descriptive name of the entry at `index`.
 *
 * The bytes stay valid until the inventory handle is released. An empty name is
 * reported as a zero length with a non-null pointer.
 */
mp_shim_status mp_shim_inventory_name(const mp_shim_inventory *inventory, size_t index,
                                      const uint8_t **out_bytes, size_t *out_len);

/*
 * Constructs and retains an exact SCContentFilter from the target at `index`.
 * The caller owns the returned opaque selection handle.
 */
mp_shim_status mp_shim_inventory_target(const mp_shim_inventory *inventory, size_t index,
                                        mp_shim_target **out);

/* Releases the inventory and every view borrowed from it. Accepts NULL. */
void mp_shim_inventory_release(mp_shim_inventory *inventory);

/* Releases one retained selection handle. Accepts NULL. */
void mp_shim_target_release(mp_shim_target *target);

/*
 * Creates a session and registers its callbacks. Does not start the producer.
 *
 * On success the caller owns the handle and must release it with
 * mp_shim_session_release after mp_shim_session_close.
 */
mp_shim_status mp_shim_session_open(const mp_shim_open_request *request, mp_shim_session **out);

/* Starts the producer. `timeout_nanos` bounds the native start. */
mp_shim_status mp_shim_session_start(mp_shim_session *session, uint64_t timeout_nanos);

/*
 * Applies a new producer surface size and retires the detached pool generation.
 *
 * Leases a caller still holds keep their own buffers; only reuse is retired.
 */
mp_shim_status mp_shim_session_reconfigure(mp_shim_session *session, uint32_t pixel_width,
                                           uint32_t pixel_height, uint64_t timeout_nanos);

/* Stops admitting callbacks. Idempotent, never blocks. */
mp_shim_status mp_shim_session_disable_callbacks(mp_shim_session *session);

/*
 * Returns only when no callback is in flight.
 *
 * After a successful fence the caller may release the state it registered. A
 * fence that reaches `timeout_nanos` reports MP_SHIM_TIMED_OUT and leaves a
 * state a later fence can finish. Repeated fences and a fence after close both
 * succeed.
 */
mp_shim_status mp_shim_session_fence(mp_shim_session *session, uint64_t timeout_nanos);

/*
 * Joins a pending start, removes the stream output, stops the producer, fences
 * admitted callbacks, and releases native state in resumable phases.
 *
 * A timeout preserves the current phase for a later call. Idempotent, and
 * completes its release even when it reports a non-timeout failure.
 */
mp_shim_status mp_shim_session_close(mp_shim_session *session, uint64_t timeout_nanos);

/* Releases the handle. Separate from close. Accepts NULL. */
void mp_shim_session_release(mp_shim_session *session);

/* Reports how many detached buffers the session currently has leased. */
mp_shim_status mp_shim_session_leased(const mp_shim_session *session, uint64_t *out_leased);

/*
 * Reports how many native objects the session still owns.
 *
 * This exists for the ownership tests ADR 0012 requires: a contained failure
 * must leave none of them alive.
 */
mp_shim_status mp_shim_session_live_objects(const mp_shim_session *session, uint64_t *out_live);

/*
 * Copies the borrowed frame's content into a session-owned buffer.
 *
 * The result is independent of the producer surface, so retaining it cannot
 * stall capture. Returns MP_SHIM_BUDGET_EXHAUSTED without blocking when every
 * unit of the session's budget is leased.
 */
mp_shim_status mp_shim_frame_detach(mp_shim_frame *borrowed, mp_shim_frame **out);

/* Releases a detached frame and returns its budget unit. Accepts NULL. */
void mp_shim_frame_release(mp_shim_frame *frame);

/*
 * Copies the frame's content into caller-owned bytes at an exact row stride.
 *
 * `destination_stride` must be at least the frame's packed row length, and
 * `capacity` at least `destination_stride * content_height`.
 */
mp_shim_status mp_shim_frame_copy_out(const mp_shim_frame *frame, uint8_t *destination,
                                      size_t capacity, uint64_t destination_stride);

/*
 * Input delivery.
 *
 * System event preparation and posting request no Accessibility access, present
 * no permission UI, and never consult ScreenCaptureKit. Process-directed posting
 * likewise never prompts, but performs bounded fresh ScreenCaptureKit authority
 * reads before each irreversible event. Authorization is non-promptingly
 * preflighted and rechecked because macOS silently discards a synthesized event
 * from an untrusted process rather than failing the post.
 *
 * The logical key vocabulary is deliberately absent. Rust owns the fixed
 * hardware key codes, which do not vary with the active layout, and this surface
 * resolves only the one thing that does: a printable character.
 */

/* Pointer buttons, in the order the platform-neutral contract declares them. */
#define MP_SHIM_INPUT_BUTTON_PRIMARY 0u
#define MP_SHIM_INPUT_BUTTON_SECONDARY 1u
#define MP_SHIM_INPUT_BUTTON_MIDDLE 2u
/* No button is involved. Valid only for a move, which it makes a plain move. */
#define MP_SHIM_INPUT_BUTTON_NONE 0xFFFFFFFFu

/* What one pointer post does. */
#define MP_SHIM_INPUT_POINTER_MOVE 0u
#define MP_SHIM_INPUT_POINTER_PRESS 1u
#define MP_SHIM_INPUT_POINTER_RELEASE 2u

/* The highest click count a press or release may declare. */
#define MP_SHIM_INPUT_MAX_CLICK_STATE 3u

/*
 * The largest line count one scroll post may carry on either axis.
 *
 * Equal to the platform-neutral notch bound, so a request the contract accepts is
 * never rejected here and a value beyond it is refused before it reaches the
 * window server.
 */
#define MP_SHIM_INPUT_MAX_SCROLL_LINES 120

/*
 * Modifier state applied to one posted event.
 *
 * Declared here rather than passing CGEventFlags so no Core Graphics value
 * reaches a Rust seam. The caller sends the modifiers its own sequence is
 * holding; the shim sets exactly those and never merges the user's live state.
 */
#define MP_SHIM_INPUT_FLAG_SHIFT 1u
#define MP_SHIM_INPUT_FLAG_CONTROL (1u << 1)
#define MP_SHIM_INPUT_FLAG_ALT (1u << 2)
#define MP_SHIM_INPUT_FLAG_META (1u << 3)

/*
 * The most UTF-16 units one posted text event carries.
 *
 * CGEventKeyboardSetUnicodeString accepts a longer string, and the window server
 * drops the tail of one that is much longer. Posting in bounded chunks keeps the
 * count the caller is told about equal to the count that was posted.
 */
#define MP_SHIM_INPUT_MAX_TEXT_CHUNK 16u

/*
 * Process-directed posting remains process-scoped even though `target` retains
 * one exact ScreenCaptureKit window. The retained window and owning-process
 * lifetime authorize the current PID; neither PID nor window number selects a
 * replacement.
 */
#define MP_SHIM_PROCESS_EVENT_POINTER 0u
#define MP_SHIM_PROCESS_EVENT_SCROLL 1u
#define MP_SHIM_PROCESS_EVENT_KEY 2u
#define MP_SHIM_PROCESS_EVENT_TEXT 3u

/* Whether one post is ordinary input or a bounded sequence-owned release. */
#define MP_SHIM_PROCESS_POST_INPUT 0u
#define MP_SHIM_PROCESS_POST_RELEASE 1u

/* Whether the final native gate must match the geometry prepared by Rust. */
#define MP_SHIM_PROCESS_GEOMETRY_AUTHORITY_ONLY 0u
#define MP_SHIM_PROCESS_GEOMETRY_REQUIRE_CURRENT 1u

/*
 * Whether the final native gate must also observe the exact retained-window
 * focus predicate. The route itself imposes no focus requirement; only a
 * caller that selected it sets this, and a sequence-owned release never does.
 */
#define MP_SHIM_PROCESS_FOCUS_NONE 0u
#define MP_SHIM_PROCESS_FOCUS_REQUIRE_FOCUSED 1u

/* Privacy-safe facts from the final per-unit process-directed gate. */
#define MP_SHIM_PROCESS_AUTHORIZATION_UNKNOWN 0u
#define MP_SHIM_PROCESS_AUTHORIZATION_GRANTED 1u
#define MP_SHIM_PROCESS_AUTHORIZATION_NOT_GRANTED 2u
#define MP_SHIM_PROCESS_AUTHORIZATION_UNAVAILABLE 3u
#define MP_SHIM_PROCESS_GEOMETRY_NOT_APPLICABLE 0u
#define MP_SHIM_PROCESS_GEOMETRY_NOT_EVALUATED 1u
#define MP_SHIM_PROCESS_GEOMETRY_PASSED 2u
#define MP_SHIM_PROCESS_GEOMETRY_CHANGED 3u
#define MP_SHIM_PROCESS_FOCUS_NOT_APPLICABLE 0u
#define MP_SHIM_PROCESS_FOCUS_NOT_EVALUATED 1u
#define MP_SHIM_PROCESS_FOCUS_PASSED 2u
#define MP_SHIM_PROCESS_FOCUS_REFUSED 3u
#define MP_SHIM_PROCESS_FOCUS_UNAVAILABLE 4u

/*
 * One fresh retained-window/process-authority observation.
 *
 * `target_match_count` is zero or one and records whether the exact retained
 * logical window passed current admission. Other windows owned by the process
 * are neither counted nor used to refuse process-scoped delivery.
 * Geometry is written only on success.
 */
typedef struct mp_shim_process_authority_report {
    uint32_t struct_size;
    uint32_t target_match_count;
    double logical_x;
    double logical_y;
    double logical_width;
    double logical_height;
    double backing_scale;
} mp_shim_process_authority_report;

/*
 * One adapter-bounded process-directed native event.
 *
 * Fields not selected by `event_kind` are ignored after their containing
 * structure and reserved bytes are validated. Text is a borrowed pointer-length
 * view valid only for the call. `timeout_nanos` is an adapter-owned maximum for
 * each native observation, not an operation-deadline decision.
 * `focus_requirement` is the caller's focus predicate, not a route requirement,
 * and a release purpose must leave it `MP_SHIM_PROCESS_FOCUS_NONE`. No native
 * framework type crosses this boundary.
 */
typedef struct mp_shim_process_post_request {
    uint32_t struct_size;
    uint32_t event_kind;
    const mp_shim_target *target;
    const mp_shim_process_event_source *event_source;
    uint64_t timeout_nanos;
    uint32_t flags;
    uint32_t geometry_check;
    uint32_t purpose;
    uint32_t action;
    uint32_t button;
    uint64_t click_state;
    double x;
    double y;
    int32_t horizontal;
    int32_t vertical;
    uint16_t key_code;
    bool key_down;
    uint8_t focus_requirement;
    uint8_t reserved[4];
    const uint16_t *text_units;
    size_t text_unit_count;
    double expected_x;
    double expected_y;
    double expected_width;
    double expected_height;
    double expected_scale;
    /*
     * The synchronous operation checkpoint runs before every mutable final gate
     * and writes the caller clock's current remaining slice. The cancellation
     * callback runs after final authority and reads only adapter-owned atomic
     * state. Both callback/context pairs remain valid for this call, contain
     * their own failures, and are never retained.
     */
    void *interruption_context;
    mp_shim_status (*interruption_callback)(void *context, uint64_t *out_wait_nanos);
    void *cancellation_context;
    mp_shim_status (*cancellation_callback)(void *context);
} mp_shim_process_post_request;

/*
 * Result facts written even when a later native unit fails.
 *
 * For ordinary input, `target_match_count` records only the retained-window
 * match and never inventories unrelated same-process windows. For a sequence-
 * owned release it is zero because visibility/window admission is deliberately
 * not consulted. `invoked_native_units` counts returned `CGEventPostToPid`
 * calls, not logical events, queue admission, target observation, consumption,
 * or visual effect. `native_effect_may_have_occurred` becomes one immediately
 * before a post call; it closes unsafe fallback and drives bounded cleanup but
 * is not evidence that the call returned or produced an effect.
 * `focus_result` distinguishes an observed unfocused target from a focus
 * predicate that could not be observed at all, and stays
 * `MP_SHIM_PROCESS_FOCUS_NOT_APPLICABLE` when the caller required no focus.
 */
typedef struct mp_shim_process_post_report {
    uint32_t struct_size;
    uint32_t target_match_count;
    uint64_t invoked_native_units;
    uint32_t authorization;
    uint32_t geometry_result;
    uint32_t focus_result;
    uint32_t native_effect_may_have_occurred;
} mp_shim_process_post_report;

/*
 * Reads the two public non-prompting authorization observations used by the
 * qualification evidence. Production process posting uses the post-event
 * preflight as its authorization truth; Accessibility remains diagnostic only.
 */
mp_shim_status mp_shim_process_authorization(uint32_t *out_post_event_access,
                                             uint32_t *out_accessibility);

/*
 * Revalidates the retained logical window, process birth token, current PID,
 * current geometry, and post-event access. Other same-process windows do not
 * affect this process-scoped authority.
 */
mp_shim_status mp_shim_process_authority(const mp_shim_target *target,
                                         uint64_t timeout_nanos,
                                         mp_shim_process_authority_report *out_authority);

/*
 * Creates one isolated `CGEventSourceStatePrivate` source for a selected
 * process-directed sequence. The caller owns it until release; release accepts
 * NULL. Every event and sequence-owned cleanup release passes the same source.
 * A nonzero activity tag is copied to the documented event-source user-data
 * field as observational, non-control-flow metadata.
 */
mp_shim_status mp_shim_process_event_source_create(
    uint64_t activity_tag, mp_shim_process_event_source **out_source);
void mp_shim_process_event_source_release(mp_shim_process_event_source *source);

/*
 * Repeats retained-window/process authority and authorization, creates every
 * balanced native event from `request->event_source` before posting, performs a
 * final post-event preflight, evaluates the caller's focus predicate when one
 * was selected, and invokes `CGEventPostToPid`. It never activates the process
 * or reads/moves the cursor.
 */
mp_shim_status mp_shim_process_post(const mp_shim_process_post_request *request,
                                    mp_shim_process_post_report *out_report);

/*
 * Reports whether the exact retained window is the active application's focused
 * window.
 *
 * A fresh shareable-content snapshot must contain a logical SCWindow equal to
 * the retained object and supplies current geometry first. Public Accessibility
 * attributes then report the active application, its focused window, and its
 * window list. Focus is true only when exactly one Accessibility window has the
 * freshly verified window's unchanged position and size and that element is
 * focused. Missing or internally inconsistent required attributes return a
 * failure status; a complete unequal or ambiguous observation writes false. No
 * title, private Accessibility identifier, or window-raising action is used.
 *
 * `timeout_nanos` bounds the shareable-content and Accessibility observations.
 */
mp_shim_status mp_shim_input_target_focused(const mp_shim_target *target,
                                            uint64_t timeout_nanos, bool *out_focused);

/*
 * Reports the retained selection's current on-screen rectangle in the global
 * point space.
 *
 * Window bounds come from a fresh shareable-content snapshot whose logical
 * `SCWindow` must equal the object retained by the discovery filter. PID and
 * window number are validation metadata only, so a same-process replacement
 * that recycles them cannot satisfy the old public target identity. Displays
 * are checked against the active display list.
 *
 * `timeout_nanos` bounds the fresh window observation.
 */
mp_shim_status mp_shim_input_target_bounds(const mp_shim_target *target,
                                           uint64_t timeout_nanos, double *out_x, double *out_y,
                                           double *out_width, double *out_height,
                                           double *out_scale);

/* Reads the pointer location in the same global point space. */
mp_shim_status mp_shim_input_pointer_location(double *out_x, double *out_y);

/* Synchronously reads the public system foreground process without prompting. */
mp_shim_status mp_shim_input_frontmost_process(uint32_t *out_process);

/*
 * Snapshots the public foreground-process lifetime and physical cursor without
 * prompting. The PID and public process-lifetime object are sampled before the
 * pointer and confirmed afterward. Only the same PID, application object, and
 * launch time publish a sample; every output remains zero on failure.
 */
mp_shim_status mp_shim_input_environment(int64_t *out_process,
                                         double *out_process_launch_time,
                                         double *out_pointer_x,
                                         double *out_pointer_y);

/*
 * Private qualification-fixture launcher. AppKit is loaded from its absolute
 * system path; the returned handle owns the exact NSRunningApplication instance.
 * A submitted application that cannot be returned remains retained through a
 * bounded graceful-then-force termination sequence and an exact-object lifecycle
 * observation, including when completion arrives after the caller's wait expires.
 * If that bounded attempt cannot verify exit, delayed reaper ownership persists
 * across bounded retries until the exact application is observed terminated.
 */
mp_shim_status mp_shim_fixture_application_launch(
    const char *bundle_path, const char *const *arguments,
    size_t argument_count, mp_shim_fixture_application **out_application,
    uint32_t *out_process_id);
mp_shim_status mp_shim_fixture_application_is_live(
    const mp_shim_fixture_application *application, uint32_t *out_live);
mp_shim_status mp_shim_fixture_application_terminate(
    mp_shim_fixture_application *application, uint32_t force);
/*
 * Releases the opaque owner only after the exact application is observed
 * terminated or retained reaper ownership has accepted the handoff.
 */
void mp_shim_fixture_application_release(
    mp_shim_fixture_application *application);

/* Deterministic scenarios for the production-shaped workspace launch helper. */
#define MP_SHIM_TEST_FIXTURE_SEMAPHORE_ALLOCATION_FAILURE 0u
#define MP_SHIM_TEST_FIXTURE_COMPLETION_EXCEPTION 1u
#define MP_SHIM_TEST_FIXTURE_LATE_COMPLETION 2u
#define MP_SHIM_TEST_FIXTURE_SUCCESSFUL_RELEASE 3u
#define MP_SHIM_TEST_FIXTURE_VALIDATION_FAILURE 4u
#define MP_SHIM_TEST_FIXTURE_HANDLE_ALLOCATION_FAILURE 5u
#define MP_SHIM_TEST_FIXTURE_REAPER_HANDOFF 6u
#define MP_SHIM_TEST_FIXTURE_RELEASE_REAPER_HANDOFF 7u

/*
 * Exercises fixture launch submission, asynchronous completion containment,
 * abandoned-application termination, and opaque-handle release without starting
 * a process. Live counts are readings from `mp_shim_live_objects`.
 */
mp_shim_status mp_shim_testing_fixture_application_launch(
    uint32_t scenario, mp_shim_status *out_launch_status,
    uint32_t *out_submission_calls,
    uint32_t *out_graceful_termination_calls,
    uint32_t *out_force_termination_calls, uint32_t *out_terminated,
    uint64_t *out_live_during_handle,
    uint64_t *out_live_after_release);

/*
 * Activates the application retained by `target`, without presenting UI.
 *
 * The target's retained public process lifetime is revalidated before and after
 * activation. A recycled numeric PID can therefore never select a replacement.
 * This activates an application and never claims to raise one particular
 * window; the caller re-reads focus and decides.
 */
mp_shim_status mp_shim_input_activate_owner(const mp_shim_target *target);

/*
 * Resolves one Unicode scalar to a key code on the active keyboard layout.
 *
 * Returns MP_SHIM_UNSUPPORTED when the layout produces the character only with
 * modifiers, or does not produce it at all. A caller that wants such a character
 * sends explicit modifier events or posts it as text.
 */
mp_shim_status mp_shim_input_resolve_character(uint32_t scalar, uint16_t *out_key_code);

/*
 * Fully prepares system-delivery events before any final mutable target gate.
 * Pointer coordinates use the global point space; `button` is ignored for a
 * move and `click_state` is the click count for a press or release.
 *
 * On success the caller owns `*out_prepared`; release it exactly once.
 */
mp_shim_status mp_shim_input_prepare_pointer(uint32_t action, uint32_t button,
                                             uint64_t click_state, double x, double y,
                                             uint32_t flags,
                                             mp_shim_prepared_input **out_prepared);
mp_shim_status mp_shim_input_prepare_scroll(int32_t horizontal, int32_t vertical, double x,
                                            double y, uint32_t flags,
                                            mp_shim_prepared_input **out_prepared);
mp_shim_status mp_shim_input_prepare_key(uint16_t key_code, bool down, uint32_t flags,
                                         mp_shim_prepared_input **out_prepared);
mp_shim_status mp_shim_input_prepare_text(const uint16_t *units, size_t count, uint32_t flags,
                                          mp_shim_prepared_input **out_prepared);

/* Reports the ordered native event count owned by a prepared input handle. */
mp_shim_status mp_shim_input_prepared_count(const mp_shim_prepared_input *prepared,
                                            size_t *out_count);

/*
 * Posts exactly the next prepared event. The deadline uses the same monotonic
 * nanosecond domain as `mp_shim_monotonic_nanos`. The cancellation callback must
 * read only adapter-owned atomic state; it and its context are borrowed for this
 * synchronous call and are never retained. `out_native_effect_may_have_occurred`
 * advances immediately before entering the void Core Graphics post.
 */
mp_shim_status mp_shim_input_post_prepared(
    mp_shim_prepared_input *prepared, size_t index, uint64_t deadline_nanos,
    void *cancellation_context,
    mp_shim_status (*cancellation_callback)(void *context),
    uint32_t *out_native_effect_may_have_occurred);

/* Releases every Core Graphics object still owned by the prepared handle. */
mp_shim_status mp_shim_input_prepared_release(mp_shim_prepared_input *prepared);

#ifdef __cplusplus
}
#endif

#endif /* MADOPILOT_MACOS_SHIM_H */
