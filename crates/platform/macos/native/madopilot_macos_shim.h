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
#define MP_SHIM_ABI_VERSION 3u

/* The largest extent, budget, and default wait the shim will accept or apply. */
#define MP_SHIM_MAX_PIXEL_EXTENT 32768u
#define MP_SHIM_MAX_DETACHED_BUDGET 256u
#define MP_SHIM_DEFAULT_TIMEOUT_NANOS 1000000000ull

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

/* `screen_*` in `mp_shim_frame_info` contains a validated per-frame rectangle. */
#define MP_SHIM_FRAME_INFO_SCREEN_RECT 1u
/* A same-sample, bounded producer-capacity recommendation is present. */
#define MP_SHIM_FRAME_INFO_SURFACE_RECOMMENDATION (1u << 1)

/* Opaque handles. Each has a complete lifecycle below. */
typedef struct mp_shim_inventory mp_shim_inventory;
typedef struct mp_shim_target mp_shim_target;
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
     * macOS recycles window numbers, so the number alone does not name an
     * incarnation. The retained selection below is authoritative; this value is
     * metadata that must agree with it. Zero is valid only for a display.
     */
    int64_t owner_process;
    /*
     * The retained SCContentFilter constructed transactionally from the candidate
     * in its originating inventory snapshot. Open consumes it directly; numeric
     * metadata is never used to resolve another object.
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
 * Deterministic test seam for the production exactly-once terminal helper.
 * Calls terminalization twice; `stopped_callback` must observe only `first`.
 */
mp_shim_status mp_shim_testing_terminalize_twice(
    void *context, void (*stopped_callback)(void *context, mp_shim_status status),
    mp_shim_status first, mp_shim_status second);

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

/* Pure seam for the same-sample source-resolution surface recommendation. */
mp_shim_status mp_shim_testing_surface_recommendation(double logical_width,
                                                      double logical_height,
                                                      double display_scale,
                                                      uint32_t *out_width,
                                                      uint32_t *out_height);

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
 * Every entry point below posts or observes Core Graphics events. None of them
 * requests Accessibility, presents permission UI, or consults ScreenCaptureKit;
 * the caller preflights authorization and re-checks it before each irreversible
 * event, because macOS silently discards a synthesized event from an untrusted
 * process rather than failing the post.
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
 * Reports the frontmost on-screen window and the process that owns it.
 *
 * Reads the window server's own front-to-back order and returns the first entry
 * in the ordinary window layer. Window names are not read, so this needs no
 * Screen Recording authorization. Returns MP_SHIM_TARGET_LOST when the desktop
 * presents no ordinary window at all.
 */
mp_shim_status mp_shim_input_frontmost_window(uint64_t *out_window_id, int64_t *out_owner_pid);

/*
 * Reports one target's current on-screen rectangle in the global point space.
 *
 * `kind` is MP_SHIM_TARGET_WINDOW or MP_SHIM_TARGET_DISPLAY. `owner_process` is
 * validated for a window and ignored for a display. Returns MP_SHIM_TARGET_LOST
 * when the object is absent or no longer owned by that process, which is also how
 * a caller establishes liveness.
 */
mp_shim_status mp_shim_input_target_bounds(uint32_t kind, uint64_t native_id, int64_t owner_process,
                                           double *out_x, double *out_y, double *out_width,
                                           double *out_height, double *out_scale);

/* Reads the pointer location in the same global point space. */
mp_shim_status mp_shim_input_pointer_location(double *out_x, double *out_y);

/*
 * Activates the application owning `owner_process`, without presenting UI.
 *
 * AppKit is loaded from its absolute system location on first use rather than
 * linked, for the reason ScreenCaptureKit is: a headless library must not carry a
 * load command for the desktop UI framework. A host that cannot provide it
 * reports MP_SHIM_UNSUPPORTED. This activates an application and never claims to
 * raise one particular window; the caller re-reads the frontmost window and
 * decides.
 */
mp_shim_status mp_shim_input_activate_owner(int64_t owner_process);

/*
 * Resolves one Unicode scalar to a key code on the active keyboard layout.
 *
 * Returns MP_SHIM_UNSUPPORTED when the layout produces the character only with
 * modifiers, or does not produce it at all. A caller that wants such a character
 * sends explicit modifier events or posts it as text.
 */
mp_shim_status mp_shim_input_resolve_character(uint32_t scalar, uint16_t *out_key_code);

/*
 * Posts one pointer event at `x`, `y` in the global point space.
 *
 * `button` is ignored for a move. `click_state` is the click count a press or
 * release carries and is ignored for a move.
 */
mp_shim_status mp_shim_input_post_pointer(uint32_t action, uint32_t button, uint64_t click_state,
                                          double x, double y, uint32_t flags);

/* Posts one line-unit scroll. Positive `vertical` scrolls the content down. */
mp_shim_status mp_shim_input_post_scroll(int32_t horizontal, int32_t vertical, uint32_t flags);

/* Posts one key event for a hardware key code. */
mp_shim_status mp_shim_input_post_key(uint16_t key_code, bool down, uint32_t flags);

/*
 * Posts `count` UTF-16 units as text rather than as key codes.
 *
 * `count` must not exceed MP_SHIM_INPUT_MAX_TEXT_CHUNK. `*out_posted` receives
 * how many units were posted, which is the whole chunk on success and the count
 * already delivered when a post could not be created. A caller reports a nonzero
 * partial count as native effect it cannot take back.
 */
mp_shim_status mp_shim_input_post_text(const uint16_t *units, size_t count, uint32_t flags,
                                       size_t *out_posted);

#ifdef __cplusplus
}
#endif

#endif /* MADOPILOT_MACOS_SHIM_H */
