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
#define MP_SHIM_ABI_VERSION 1u

/* The largest extent, budget, and default wait the shim will accept or apply. */
#define MP_SHIM_MAX_PIXEL_EXTENT 32768u
#define MP_SHIM_MAX_DETACHED_BUDGET 256u
#define MP_SHIM_DEFAULT_TIMEOUT_NANOS 1000000000ull

/*
 * Session-scoped test seams.
 *
 * A caller may ask a session to raise a native exception at one of the four
 * boundary positions ADR 0012 measured, which is how the containment and
 * failure-path ownership cases become testable. Nothing in the product sets
 * them, and a session that leaves them zero behaves as if they did not exist.
 */
#define MP_SHIM_RAISE_AT_START 1u
#define MP_SHIM_RAISE_BEFORE_CALLBACK 2u
#define MP_SHIM_RAISE_AFTER_CALLBACK 4u
#define MP_SHIM_RAISE_AT_TEARDOWN 8u

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

/* What a non-prompting authorization probe established. */
#define MP_SHIM_PERMISSION_GRANTED 0u
#define MP_SHIM_PERMISSION_NOT_GRANTED 1u
#define MP_SHIM_PERMISSION_UNAVAILABLE 2u
#define MP_SHIM_PERMISSION_UNKNOWN 3u

/* The signing and launch context a permission answer was read in. */
#define MP_SHIM_CONTEXT_UNKNOWN 0u
/* A main bundle with an identifier: the context Apple grants per application. */
#define MP_SHIM_CONTEXT_BUNDLED 1u
/* A bare executable, whose grant follows the launching process instead. */
#define MP_SHIM_CONTEXT_UNBUNDLED 2u

/* What kind of desktop object a target is. */
#define MP_SHIM_TARGET_WINDOW 0u
#define MP_SHIM_TARGET_DISPLAY 1u

/* The only pixel layout this shim publishes. */
#define MP_SHIM_PIXEL_BGRA8 0u

/* Opaque handles. Each has a complete lifecycle below. */
typedef struct mp_shim_inventory mp_shim_inventory;
typedef struct mp_shim_session mp_shim_session;
typedef struct mp_shim_frame mp_shim_frame;

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
    uint32_t reserved;
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
    /* Declared rather than left to padding, so the layout is the same on both
     * sides of the boundary without either side inferring it. */
    uint32_t reserved[2];
    /* Producer timestamp, in nanoseconds on the host's monotonic clock — the same
     * clock and unit `mp_shim_monotonic_nanos` reports, so the two are directly
     * comparable. The framework supplies mach absolute units; the shim converts. */
    uint64_t display_time_nanos;
    /* Capture pixels per point for this frame. */
    double scale_factor;
    /* Content origin within the surface, in capture pixels. */
    double content_origin_x;
    double content_origin_y;
} mp_shim_frame_info;

/* What a session is being opened for. */
typedef struct mp_shim_open_request {
    uint32_t struct_size;
    uint32_t kind;
    uint64_t native_id;
    uint32_t pixel_width;
    uint32_t pixel_height;
    /* Producer queue depth. Bounded by the shim to a reviewed range. */
    uint32_t queue_depth;
    /* How many detached buffers the session may lease at once. */
    uint32_t detached_budget;
    /* Bounds the one native asynchronous query this open performs. */
    uint64_t timeout_nanos;
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
                                    uint32_t *out_open_request);

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

/* Reports the signing and launch context the probes above were read in. */
mp_shim_status mp_shim_launch_context(uint32_t *out_context);

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

/* Releases the inventory and every view borrowed from it. Accepts NULL. */
void mp_shim_inventory_release(mp_shim_inventory *inventory);

/*
 * Reads the target's current placement, for use at frame time.
 *
 * Writes four doubles into `out_frame`: origin x, origin y, width, height, in
 * global points. Writes the backing scale of the display holding the target into
 * `out_scale`, which is what an Adapter needs to ask for a producer surface at the
 * target's native resolution after it has moved. Returns MP_SHIM_TARGET_LOST when
 * the target is no longer present.
 */
mp_shim_status mp_shim_current_placement(uint32_t kind, uint64_t native_id, double *out_frame,
                                        double *out_scale);

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
 * Stops the producer, removes the stream output, and releases native state.
 *
 * Idempotent, and completes its release even when it reports a failure.
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

#ifdef __cplusplus
}
#endif

#endif /* MADOPILOT_MACOS_SHIM_H */
