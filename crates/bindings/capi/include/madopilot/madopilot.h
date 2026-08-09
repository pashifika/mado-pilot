/*
 * MadoPilot C ABI — ABI 1.2.
 *
 * MadoPilot is a headless visual automation runtime. This header is the whole
 * C contract: one exported symbol, an immutable function table reached through
 * it, opaque handles with complete retain and release lifecycles, and
 * size-versioned structures with documented mandatory prefixes.
 *
 * ============================================================================
 * ABI 1.2. COMPLETE RELEASED ABI 1.0 PREFIX FROZEN.
 *
 * Every ABI 1.0 numeric value, structure prefix, field offset, and
 * function-table position is frozen for ABI major 1 by
 * docs/adr/0007-phase-1-c-abi-freeze.md. ABI 1.2 replaces the unreleased ABI
 * 1.1 draft after that boundary with explicit input-submission evidence and
 * bounded caller-owned diagnostics. ABI 1.1 is intentionally unsupported.
 * Within this major, ABI 1.2 and later append only.
 *
 * Use the smaller of your sizeof and the returned table's struct_size to decide
 * which members exist. crates/bindings/capi/tests/abi-compat/ keeps every
 * released header as a compatibility fixture against every later library.
 * ============================================================================
 *
 * Requires C99 or later, or C++11 or later. Both release targets are 64-bit,
 * so there is one calling convention and no convention macro.
 *
 * See docs/c-abi.md for the ownership rules, the structure-prefix rules, the
 * status table, and how to build against this library.
 */

#ifndef MADOPILOT_MADOPILOT_H
#define MADOPILOT_MADOPILOT_H

#include <stddef.h>
#include <stdint.h>

#if defined(_WIN32)
#  if defined(MADOPILOT_BUILDING)
#    define MADOPILOT_EXPORT __declspec(dllexport)
#  else
#    define MADOPILOT_EXPORT __declspec(dllimport)
#  endif
#else
#  define MADOPILOT_EXPORT __attribute__((visibility("default")))
#endif

#ifdef __cplusplus
extern "C" {
#endif

/* ---------------------------------------------------------------------------
 * Versions
 * ------------------------------------------------------------------------ */

/* The ABI major version this header declares. A different major is a different
 * library: the release loader names carry it. */
#define MADOPILOT_ABI_MAJOR 1u

/* The ABI minor version this header declares. ABI 1.1 was never released. */
#define MADOPILOT_ABI_MINOR 2u

/* ---------------------------------------------------------------------------
 * Status
 * ------------------------------------------------------------------------ */

/* A status code. Signed and fixed-width, so it does not depend on the
 * representation a C compiler chose for an enum. */
typedef int32_t madopilot_status_t;

enum {
    /* The operation completed and every required output is populated. */
    MADOPILOT_STATUS_OK = 0,
    /* The request was malformed, out of range, or named something unknown. */
    MADOPILOT_STATUS_INVALID_ARGUMENT = 1,
    /* The request is well-formed but this build cannot satisfy it. */
    MADOPILOT_STATUS_UNSUPPORTED = 2,
    /* The cancellation token was set before the result committed. */
    MADOPILOT_STATUS_CANCELLED = 3,
    /* The absolute deadline passed before the result committed. */
    MADOPILOT_STATUS_DEADLINE_EXCEEDED = 4,
    /* The session has entered closing and starts no further work. */
    MADOPILOT_STATUS_CLOSED = 5,
    /* The capture target no longer exists. */
    MADOPILOT_STATUS_TARGET_LOST = 6,
    /* A configured or implementation limit would have been exceeded. */
    MADOPILOT_STATUS_LIMIT_EXCEEDED = 7,
    /* Capture could not produce the requested frame. */
    MADOPILOT_STATUS_CAPTURE_FAILED = 8,
    /* An asset package broke one of the rules that make it trustworthy. */
    MADOPILOT_STATUS_ASSET_INVALID = 9,
    /* The matching backend was unavailable or could not complete the search. */
    MADOPILOT_STATUS_VISION_FAILED = 10,
    /* An invariant the library is responsible for did not hold. */
    MADOPILOT_STATUS_INTERNAL = 11,
    /* A panic was contained at the boundary. No unwind crossed into C, every
     * valid output is in its failure state, and handles unrelated to the failed
     * call remain usable. For a side-effecting call, the failure state does not
     * prove that no native effect occurred. */
    MADOPILOT_STATUS_INTERNAL_PANIC = 12,
    /* Input was refused before admission and no terminal receipt exists. */
    MADOPILOT_STATUS_INPUT_FAILED = 13
};

/* The subsystem a failure came from. Diagnostic; branch on the status. */
typedef int32_t madopilot_error_category_t;

enum {
    MADOPILOT_ERROR_CATEGORY_UNSPECIFIED = 0,
    /* The C boundary refused the call: a pointer, size, tag, or conversion. */
    MADOPILOT_ERROR_CATEGORY_ABI = 1,
    /* The operation's deadline or cancellation ended the call. */
    MADOPILOT_ERROR_CATEGORY_OPERATION = 2,
    /* Engine construction or configuration. */
    MADOPILOT_ERROR_CATEGORY_ENGINE = 3,
    /* Discovery, session lifecycle, frames, or mapping. */
    MADOPILOT_ERROR_CATEGORY_CAPTURE = 4,
    /* Asset package loading or template resolution. */
    MADOPILOT_ERROR_CATEGORY_ASSET = 5,
    /* Template preparation or matching. */
    MADOPILOT_ERROR_CATEGORY_VISION = 6,
    /* Coordinate spaces, rectangles, and extents. */
    MADOPILOT_ERROR_CATEGORY_GEOMETRY = 7,
    /* Non-prompting permission probes. */
    MADOPILOT_ERROR_CATEGORY_PERMISSION = 8,
    /* Input admission or delivery. */
    MADOPILOT_ERROR_CATEGORY_INPUT = 9
};

/* ---------------------------------------------------------------------------
 * Enumerated values
 * ------------------------------------------------------------------------ */

/* A coordinate space. Every public rectangle names the space it is measured
 * in, because a rectangle without one places input somewhere nobody asked. */
typedef int32_t madopilot_space_t;

enum {
    MADOPILOT_SPACE_CAPTURE_PIXELS = 0,
    MADOPILOT_SPACE_FRAME_NORMALIZED = 1,
    MADOPILOT_SPACE_TARGET_NORMALIZED = 2,
    MADOPILOT_SPACE_TARGET_LOGICAL = 3,
    MADOPILOT_SPACE_DESKTOP_LOGICAL = 4
};

/* A pixel layout. Both are four bytes per pixel. */
typedef int32_t madopilot_pixel_format_t;

enum {
    MADOPILOT_PIXEL_FORMAT_RGBA8 = 0,
    MADOPILOT_PIXEL_FORMAT_BGRA8 = 1
};

/* What to do with a region that leaves the frame. */
typedef int32_t madopilot_clip_policy_t;

enum {
    /* Fail when any part falls outside. The default. */
    MADOPILOT_CLIP_POLICY_REJECT = 0,
    /* Keep the overlapping part, failing only when nothing overlaps. */
    MADOPILOT_CLIP_POLICY_CLIP = 1
};

/* Whether a supplied replay frame continues the previous one. */
typedef int32_t madopilot_continuity_t;

enum {
    MADOPILOT_CONTINUITY_CONTINUOUS = 0,
    MADOPILOT_CONTINUITY_DISCONTINUOUS = 1
};

/* How overlapping candidates are reduced. */
typedef int32_t madopilot_suppression_t;

enum {
    /* Drop a candidate overlapping a canonically earlier survivor. Default. */
    MADOPILOT_SUPPRESSION_DROP_OVERLAPPING = 0,
    /* Report every candidate that passed the threshold. */
    MADOPILOT_SUPPRESSION_KEEP_ALL = 1
};

/* Which source an engine captures from. */
typedef int32_t madopilot_source_kind_t;

enum {
    MADOPILOT_SOURCE_REPLAY_MEMORY = 0,
    MADOPILOT_SOURCE_REPLAY_DIRECTORY = 1,
    MADOPILOT_SOURCE_NATIVE_WINDOWS = 2,
    MADOPILOT_SOURCE_NATIVE_MACOS = 3
};

/* Where an asset package is read from. */
typedef int32_t madopilot_package_source_kind_t;

enum {
    MADOPILOT_PACKAGE_SOURCE_DIRECTORY = 0,
    MADOPILOT_PACKAGE_SOURCE_ARCHIVE_FILE = 1,
    MADOPILOT_PACKAGE_SOURCE_ARCHIVE_BYTES = 2
};

/* Which rule an asset package broke.
 *
 * This is detail a single status cannot carry: a bad content hash and an unsafe
 * entry path are both MADOPILOT_STATUS_ASSET_INVALID and are not the same
 * problem. */
typedef int32_t madopilot_asset_fault_t;

enum {
    MADOPILOT_ASSET_FAULT_UNKNOWN = 0,
    MADOPILOT_ASSET_FAULT_LIMIT_ABOVE_CEILING = 1,
    MADOPILOT_ASSET_FAULT_SOURCE_UNREADABLE = 2,
    MADOPILOT_ASSET_FAULT_SOURCE_CHANGED = 3,
    MADOPILOT_ASSET_FAULT_MALFORMED_ARCHIVE = 4,
    MADOPILOT_ASSET_FAULT_UNSUPPORTED_COMPRESSION_METHOD = 5,
    MADOPILOT_ASSET_FAULT_ENCRYPTED_ENTRY = 6,
    MADOPILOT_ASSET_FAULT_ARCHIVE_LIMIT = 7,
    MADOPILOT_ASSET_FAULT_UNSAFE_PATH = 8,
    MADOPILOT_ASSET_FAULT_DUPLICATE_PATH = 9,
    MADOPILOT_ASSET_FAULT_UNSUPPORTED_ENTRY_TYPE = 10,
    MADOPILOT_ASSET_FAULT_DECLARED_SIZE_MISMATCH = 11,
    MADOPILOT_ASSET_FAULT_MISSING_MANIFEST = 12,
    MADOPILOT_ASSET_FAULT_MALFORMED_MANIFEST = 13,
    MADOPILOT_ASSET_FAULT_MISSING_SCHEMA_VERSION = 14,
    MADOPILOT_ASSET_FAULT_UNSUPPORTED_SCHEMA_VERSION = 15,
    MADOPILOT_ASSET_FAULT_DUPLICATE_IDENTITY = 16,
    MADOPILOT_ASSET_FAULT_MISSING_ENTRY = 17,
    MADOPILOT_ASSET_FAULT_UNKNOWN_TEMPLATE = 18,
    MADOPILOT_ASSET_FAULT_UNSUPPORTED_SOURCE = 19,
    MADOPILOT_ASSET_FAULT_INVALID_TEMPLATE_METADATA = 20,
    MADOPILOT_ASSET_FAULT_UNSUPPORTED_TEMPLATE_SPACE = 21,
    MADOPILOT_ASSET_FAULT_UNSUPPORTED_HASH_ALGORITHM = 22,
    MADOPILOT_ASSET_FAULT_MALFORMED_HASH = 23,
    MADOPILOT_ASSET_FAULT_HASH_MISMATCH = 24,
    MADOPILOT_ASSET_FAULT_UNSUPPORTED_CONTENT_ENCODING = 25,
    MADOPILOT_ASSET_FAULT_ARITHMETIC_OVERFLOW = 26,
    MADOPILOT_ASSET_FAULT_CANCELLED = 27,
    MADOPILOT_ASSET_FAULT_DEADLINE_EXCEEDED = 28
};

/* How far package loading had got when it refused. */
typedef int32_t madopilot_asset_stage_t;

enum {
    MADOPILOT_ASSET_STAGE_UNKNOWN = 0,
    MADOPILOT_ASSET_STAGE_CONFIGURATION = 1,
    MADOPILOT_ASSET_STAGE_SOURCE = 2,
    MADOPILOT_ASSET_STAGE_DIRECTORY_PRE_PARSE = 3,
    MADOPILOT_ASSET_STAGE_DIRECTORY_OPEN = 4,
    MADOPILOT_ASSET_STAGE_ENTRY_METADATA = 5,
    MADOPILOT_ASSET_STAGE_MANIFEST = 6,
    MADOPILOT_ASSET_STAGE_EXPANSION = 7,
    MADOPILOT_ASSET_STAGE_COMMIT = 8
};

/* A redacted-diagnostic category. */
typedef int32_t madopilot_diagnostic_category_t;

enum {
    MADOPILOT_DIAGNOSTIC_UNSPECIFIED = 0,
    MADOPILOT_DIAGNOSTIC_PERMISSION_DENIED = 1,
    MADOPILOT_DIAGNOSTIC_PERMISSION_UNDETERMINED = 2,
    MADOPILOT_DIAGNOSTIC_CAPABILITY_UNAVAILABLE = 3,
    MADOPILOT_DIAGNOSTIC_TARGET_LOST = 4,
    MADOPILOT_DIAGNOSTIC_PLATFORM_FAILURE = 5,
    MADOPILOT_DIAGNOSTIC_CONFIGURATION = 6
};

/* A sensitive capability whose authorization can be probed. */
typedef int32_t madopilot_permission_kind_t;

enum {
    MADOPILOT_PERMISSION_KIND_UNSPECIFIED = 0,
    MADOPILOT_PERMISSION_KIND_SCREEN_CAPTURE = 1,
    MADOPILOT_PERMISSION_KIND_INPUT_CONTROL = 2
};

/* The result of a non-prompting permission probe. */
typedef int32_t madopilot_permission_state_t;

enum {
    MADOPILOT_PERMISSION_STATE_UNKNOWN = 0,
    MADOPILOT_PERMISSION_STATE_GRANTED = 1,
    MADOPILOT_PERMISSION_STATE_NOT_GRANTED = 2,
    MADOPILOT_PERMISSION_STATE_UNAVAILABLE = 3
};

/* The kind of desktop object a discovered target represents. */
typedef int32_t madopilot_target_kind_t;

enum {
    MADOPILOT_TARGET_KIND_UNKNOWN = 0,
    MADOPILOT_TARGET_KIND_WINDOW = 1,
    MADOPILOT_TARGET_KIND_DISPLAY = 2
};

/* Whether a provider can attempt one operation. */
typedef int32_t madopilot_capability_support_t;

enum {
    MADOPILOT_CAPABILITY_UNKNOWN = 0,
    MADOPILOT_CAPABILITY_SUPPORTED = 1,
    MADOPILOT_CAPABILITY_UNSUPPORTED = 2
};

/* What an input event does, independently of delivery. */
typedef int32_t madopilot_input_operation_kind_t;

enum {
    MADOPILOT_INPUT_OPERATION_UNKNOWN = 0,
    MADOPILOT_INPUT_OPERATION_POINTER = 1,
    MADOPILOT_INPUT_OPERATION_KEYBOARD = 2,
    MADOPILOT_INPUT_OPERATION_TEXT = 3
};

/* How an input event is submitted. */
typedef int32_t madopilot_input_delivery_t;

enum {
    MADOPILOT_INPUT_DELIVERY_NONE = 0,
    MADOPILOT_INPUT_DELIVERY_SYSTEM = 1,
    MADOPILOT_INPUT_DELIVERY_WINDOW_MESSAGE = 2,
    MADOPILOT_INPUT_DELIVERY_PROCESS_DIRECTED = 3
};

/* Whether session open can proceed without requested input. */
typedef int32_t madopilot_input_requirement_t;

enum {
    MADOPILOT_INPUT_OPTIONAL = 0,
    MADOPILOT_INPUT_REQUIRED = 1
};

/* What input delivery may do about focus. */
typedef int32_t madopilot_focus_policy_t;

enum {
    MADOPILOT_FOCUS_PRESERVE = 0,
    MADOPILOT_FOCUS_REQUIRE_FOCUSED = 1,
    MADOPILOT_FOCUS_ACTIVATE_IF_REQUIRED = 2
};

/* How pointer coordinates resolve at delivery time. */
typedef int32_t madopilot_geometry_policy_t;

enum {
    MADOPILOT_GEOMETRY_REPROJECT_CURRENT = 0,
    MADOPILOT_GEOMETRY_REQUIRE_UNCHANGED = 1,
    MADOPILOT_GEOMETRY_USE_FRAME_SNAPSHOT = 2
};

/* A pointer button. */
typedef int32_t madopilot_pointer_button_t;

enum {
    MADOPILOT_POINTER_BUTTON_UNKNOWN = 0,
    MADOPILOT_POINTER_BUTTON_PRIMARY = 1,
    MADOPILOT_POINTER_BUTTON_SECONDARY = 2,
    MADOPILOT_POINTER_BUTTON_MIDDLE = 3
};

/* A keyboard modifier. */
typedef int32_t madopilot_modifier_t;

enum {
    MADOPILOT_MODIFIER_UNKNOWN = 0,
    MADOPILOT_MODIFIER_SHIFT = 1,
    MADOPILOT_MODIFIER_CONTROL = 2,
    MADOPILOT_MODIFIER_ALT = 3,
    MADOPILOT_MODIFIER_META = 4
};

/* A logical key kind. Character, function, and modifier keys use key_value. */
typedef int32_t madopilot_key_t;

enum {
    MADOPILOT_KEY_UNKNOWN = 0,
    MADOPILOT_KEY_CHARACTER = 1,
    MADOPILOT_KEY_FUNCTION = 2,
    MADOPILOT_KEY_MODIFIER = 3,
    MADOPILOT_KEY_ENTER = 4,
    MADOPILOT_KEY_TAB = 5,
    MADOPILOT_KEY_BACKSPACE = 6,
    MADOPILOT_KEY_DELETE = 7,
    MADOPILOT_KEY_ESCAPE = 8,
    MADOPILOT_KEY_SPACE = 9,
    MADOPILOT_KEY_ARROW_UP = 10,
    MADOPILOT_KEY_ARROW_DOWN = 11,
    MADOPILOT_KEY_ARROW_LEFT = 12,
    MADOPILOT_KEY_ARROW_RIGHT = 13,
    MADOPILOT_KEY_HOME = 14,
    MADOPILOT_KEY_END = 15,
    MADOPILOT_KEY_PAGE_UP = 16,
    MADOPILOT_KEY_PAGE_DOWN = 17
};

/* One input event variant. */
typedef int32_t madopilot_input_event_kind_t;

enum {
    MADOPILOT_INPUT_EVENT_UNKNOWN = 0,
    MADOPILOT_INPUT_EVENT_POINTER_MOVE = 1,
    MADOPILOT_INPUT_EVENT_POINTER_PRESS = 2,
    MADOPILOT_INPUT_EVENT_POINTER_RELEASE = 3,
    MADOPILOT_INPUT_EVENT_POINTER_SCROLL = 4,
    MADOPILOT_INPUT_EVENT_KEY_PRESS = 5,
    MADOPILOT_INPUT_EVENT_KEY_RELEASE = 6,
    MADOPILOT_INPUT_EVENT_TEXT = 7,
    MADOPILOT_INPUT_EVENT_DELAY = 8
};

/* Fixed ABI 1.2 input ceilings. A descriptor's max_events may be lower. */
#define MADOPILOT_INPUT_MAX_EVENTS 256u
#define MADOPILOT_INPUT_MAX_TEXT_CHARS 4096u
#define MADOPILOT_INPUT_MAX_TEXT_UTF8_BYTES 16384u
#define MADOPILOT_INPUT_MAX_DELAY_NANOS UINT64_C(5000000000)
#define MADOPILOT_INPUT_MAX_SCROLL_NOTCHES 120
#define MADOPILOT_INPUT_MIN_FUNCTION_KEY 1u
#define MADOPILOT_INPUT_MAX_FUNCTION_KEY 24u
#define MADOPILOT_INPUT_MAX_CLEANUP_EVENTS 256u
#define MADOPILOT_INPUT_MAX_CLEANUP_NANOS UINT64_C(250000000)

/* How far an admitted sequence got. */
typedef int32_t madopilot_sequence_outcome_t;

enum {
    MADOPILOT_SEQUENCE_UNEXECUTED = 0,
    MADOPILOT_SEQUENCE_COMPLETE = 1,
    MADOPILOT_SEQUENCE_PARTIAL = 2
};

/* What became of state a stopped sequence had pressed. */
typedef int32_t madopilot_cleanup_state_t;

enum {
    MADOPILOT_CLEANUP_NOT_NEEDED = 0,
    MADOPILOT_CLEANUP_COMPLETE = 1,
    MADOPILOT_CLEANUP_INCOMPLETE = 2,
    MADOPILOT_CLEANUP_EXHAUSTED = 3
};

/* What native object or subsystem a route addresses. */
typedef int32_t madopilot_input_address_scope_t;

enum {
    MADOPILOT_INPUT_ADDRESS_NONE = 0,
    MADOPILOT_INPUT_ADDRESS_FOCUSED_SYSTEM = 1,
    MADOPILOT_INPUT_ADDRESS_EXACT_WINDOW = 2,
    MADOPILOT_INPUT_ADDRESS_OWNING_PROCESS = 3
};

/* The strongest native submission fact a route can report. */
typedef int32_t madopilot_submission_evidence_t;

enum {
    MADOPILOT_SUBMISSION_EVIDENCE_NONE = 0,
    MADOPILOT_SUBMISSION_EVIDENCE_INVOCATION_ONLY = 1,
    MADOPILOT_SUBMISSION_EVIDENCE_SYSTEM_INPUT_ADMISSION = 2,
    MADOPILOT_SUBMISSION_EVIDENCE_TARGET_QUEUE_ADMISSION = 3,
    MADOPILOT_SUBMISSION_EVIDENCE_TARGET_PROTOCOL_ACKNOWLEDGEMENT = 4
};

/* Why an admitted sequence stopped, or why input was refused. */
typedef int32_t madopilot_input_fault_t;

enum {
    MADOPILOT_INPUT_FAULT_NONE = 0,
    MADOPILOT_INPUT_FAULT_FOREIGN_TARGET = 1,
    MADOPILOT_INPUT_FAULT_UNKNOWN_TARGET = 2,
    MADOPILOT_INPUT_FAULT_TARGET_LOST = 3,
    MADOPILOT_INPUT_FAULT_PROVIDER_MISMATCH = 4,
    MADOPILOT_INPUT_FAULT_UNSUPPORTED_COMBINATION = 5,
    MADOPILOT_INPUT_FAULT_INVALID_ROUTE_PLAN = 6,
    MADOPILOT_INPUT_FAULT_ROUTE_UNAVAILABLE = 7,
    MADOPILOT_INPUT_FAULT_SEQUENCE_OUT_OF_BOUNDS = 8,
    MADOPILOT_INPUT_FAULT_UNSUPPORTED_COORDINATE = 9,
    MADOPILOT_INPUT_FAULT_MISSING_COORDINATE_SOURCE = 10,
    MADOPILOT_INPUT_FAULT_GEOMETRY_CHANGED = 11,
    MADOPILOT_INPUT_FAULT_FOCUS_REQUIRED = 12,
    MADOPILOT_INPUT_FAULT_FOCUS_REFUSED = 13,
    MADOPILOT_INPUT_FAULT_NOT_AUTHORIZED = 14,
    MADOPILOT_INPUT_FAULT_POLICY_REFUSED = 15,
    MADOPILOT_INPUT_FAULT_CONTROLLER_CLOSED = 16,
    MADOPILOT_INPUT_FAULT_CANCELLED = 17,
    MADOPILOT_INPUT_FAULT_DEADLINE_EXCEEDED = 18,
    MADOPILOT_INPUT_FAULT_SUBMISSION_FAILED = 19
};

/* Input capability pair masks. */
#define MADOPILOT_INPUT_PAIR_POINTER_SYSTEM UINT64_C(0x001)
#define MADOPILOT_INPUT_PAIR_POINTER_WINDOW_MESSAGE UINT64_C(0x002)
#define MADOPILOT_INPUT_PAIR_POINTER_PROCESS_DIRECTED UINT64_C(0x004)
#define MADOPILOT_INPUT_PAIR_KEYBOARD_SYSTEM UINT64_C(0x008)
#define MADOPILOT_INPUT_PAIR_KEYBOARD_WINDOW_MESSAGE UINT64_C(0x010)
#define MADOPILOT_INPUT_PAIR_KEYBOARD_PROCESS_DIRECTED UINT64_C(0x020)
#define MADOPILOT_INPUT_PAIR_TEXT_SYSTEM UINT64_C(0x040)
#define MADOPILOT_INPUT_PAIR_TEXT_WINDOW_MESSAGE UINT64_C(0x080)
#define MADOPILOT_INPUT_PAIR_TEXT_PROCESS_DIRECTED UINT64_C(0x100)
#define MADOPILOT_INPUT_PAIRS_ALL UINT64_C(0x1ff)

/* Diagnostic configuration and drain outcomes. */
typedef int32_t madopilot_diagnostic_level_t;
enum {
    MADOPILOT_DIAGNOSTIC_LEVEL_OFF = 0,
    MADOPILOT_DIAGNOSTIC_LEVEL_NORMAL = 1,
    MADOPILOT_DIAGNOSTIC_LEVEL_DEBUG = 2
};

typedef int32_t madopilot_diagnostic_drain_state_t;
enum {
    MADOPILOT_DIAGNOSTIC_DRAIN_BATCH = 1,
    MADOPILOT_DIAGNOSTIC_DRAIN_OPEN_EMPTY = 2,
    MADOPILOT_DIAGNOSTIC_DRAIN_END_OF_STREAM = 3
};

/* Stable diagnostic payload categories. */
typedef int32_t madopilot_diagnostic_kind_t;
enum {
    MADOPILOT_DIAGNOSTIC_KIND_OPERATION_STARTED = 1,
    MADOPILOT_DIAGNOSTIC_KIND_FRAME = 2,
    MADOPILOT_DIAGNOSTIC_KIND_MAPPING = 3,
    MADOPILOT_DIAGNOSTIC_KIND_SEARCH = 4,
    MADOPILOT_DIAGNOSTIC_KIND_INPUT = 5,
    MADOPILOT_DIAGNOSTIC_KIND_ROUTE_ATTEMPT = 6,
    MADOPILOT_DIAGNOSTIC_KIND_LIFECYCLE = 7,
    MADOPILOT_DIAGNOSTIC_KIND_PERMISSION = 8
};

typedef int32_t madopilot_diagnostic_operation_kind_t;
enum {
    MADOPILOT_DIAGNOSTIC_OPERATION_DISCOVERY = 1,
    MADOPILOT_DIAGNOSTIC_OPERATION_INPUT_DESCRIPTION = 2,
    MADOPILOT_DIAGNOSTIC_OPERATION_PERMISSION = 3,
    MADOPILOT_DIAGNOSTIC_OPERATION_SESSION_OPEN = 4,
    MADOPILOT_DIAGNOSTIC_OPERATION_FRAME_ACQUIRE = 5,
    MADOPILOT_DIAGNOSTIC_OPERATION_MAPPING = 6,
    MADOPILOT_DIAGNOSTIC_OPERATION_TEMPLATE_PREPARATION = 7,
    MADOPILOT_DIAGNOSTIC_OPERATION_SEARCH = 8,
    MADOPILOT_DIAGNOSTIC_OPERATION_INPUT_SUBMISSION = 9,
    MADOPILOT_DIAGNOSTIC_OPERATION_SESSION_CLOSE = 10
};

typedef int32_t madopilot_search_diagnostic_outcome_t;
enum {
    MADOPILOT_SEARCH_DIAGNOSTIC_MATCHED = 1,
    MADOPILOT_SEARCH_DIAGNOSTIC_NO_MATCH = 2,
    MADOPILOT_SEARCH_DIAGNOSTIC_FAILED = 3
};

typedef int32_t madopilot_lifecycle_t;
enum {
    MADOPILOT_LIFECYCLE_OPEN = 1,
    MADOPILOT_LIFECYCLE_CLOSING = 2,
    MADOPILOT_LIFECYCLE_CLOSED = 3
};

/* ---------------------------------------------------------------------------
 * Flags
 * ------------------------------------------------------------------------ */

/* madopilot_operation_t optional fields. */
#define MADOPILOT_OPERATION_HAS_DEADLINE 0x1u
#define MADOPILOT_OPERATION_HAS_ACTIVITY_TAG 0x2u

/* madopilot_open_request_t: which optional fields are set. */
#define MADOPILOT_OPEN_HAS_REQUIRED_FORMAT 0x1u
#define MADOPILOT_OPEN_HAS_PREFERRED_FORMAT 0x2u

/* madopilot_map_request_t.region is set; the whole frame is mapped without it. */
#define MADOPILOT_MAP_HAS_REGION 0x1u

/* madopilot_find_request_t.region is set. */
#define MADOPILOT_FIND_HAS_REGION 0x1u

/* madopilot_match_options_t overrides. */
#define MADOPILOT_MATCH_HAS_MIN_SCORE 0x1u
#define MADOPILOT_MATCH_HAS_MAX_RESULTS 0x2u
#define MADOPILOT_MATCH_HAS_SUPPRESSION 0x4u

#define MADOPILOT_IMAGE_SHARED 0x1u

/* madopilot_target_t capability and presence bits. */
#define MADOPILOT_TARGET_SUPPORTS_PLACEMENT 0x1u
#define MADOPILOT_TARGET_HAS_KIND 0x2u
#define MADOPILOT_TARGET_HAS_CAPTURE_PERMISSION 0x4u

/* Engine-wide capability bits. */
#define MADOPILOT_ENGINE_DELIVERS_INPUT 0x1u
#define MADOPILOT_ENGINE_READS_PERMISSIONS 0x2u

/* madopilot_permission_t presence bits. */
#define MADOPILOT_PERMISSION_HAS_DIAGNOSTIC 0x1u
#define MADOPILOT_PERMISSION_HAS_PLATFORM_CODE 0x2u

/* madopilot_input_capability_t presence bits. */
#define MADOPILOT_INPUT_CAPABILITY_HAS_PERMISSION 0x1u
#define MADOPILOT_INPUT_CAPABILITY_HAS_EVIDENCE 0x2u

#define MADOPILOT_INPUT_REQUEST_HAS_CLEANUP_BUDGET 0x1u

/* madopilot_input_receipt_info_t presence and outcome bits. */
#define MADOPILOT_INPUT_RECEIPT_HAS_SELECTED_ROUTE 0x01u
#define MADOPILOT_INPUT_RECEIPT_HAS_LAST_SUBMITTED 0x02u
#define MADOPILOT_INPUT_RECEIPT_HAS_EVIDENCE 0x04u
#define MADOPILOT_INPUT_RECEIPT_HAS_FAULT 0x08u
#define MADOPILOT_INPUT_RECEIPT_PARTIAL_NATIVE_EFFECT 0x10u
#define MADOPILOT_INPUT_RECEIPT_USED_FALLBACK 0x20u

/* madopilot_input_attempt_t presence and outcome bits. */
#define MADOPILOT_INPUT_ATTEMPT_HAS_LAST_SUBMITTED 0x1u
#define MADOPILOT_INPUT_ATTEMPT_HAS_EVIDENCE 0x2u
#define MADOPILOT_INPUT_ATTEMPT_HAS_FAULT 0x4u
#define MADOPILOT_INPUT_ATTEMPT_PARTIAL_NATIVE_EFFECT 0x8u

/* madopilot_diagnostic_record_t presence bits. */
#define MADOPILOT_DIAGNOSTIC_RECORD_HAS_ACTIVITY 0x001u
#define MADOPILOT_DIAGNOSTIC_RECORD_HAS_TARGET 0x002u
#define MADOPILOT_DIAGNOSTIC_RECORD_HAS_FRAME 0x004u
#define MADOPILOT_DIAGNOSTIC_RECORD_HAS_TEMPLATE 0x008u
#define MADOPILOT_DIAGNOSTIC_RECORD_HAS_SOURCE_SPACE 0x010u
#define MADOPILOT_DIAGNOSTIC_RECORD_HAS_DESTINATION_SPACE 0x020u
#define MADOPILOT_DIAGNOSTIC_RECORD_HAS_REGION_SPACE 0x040u
#define MADOPILOT_DIAGNOSTIC_RECORD_HAS_ROUTE 0x080u
#define MADOPILOT_DIAGNOSTIC_RECORD_HAS_ADDRESS_SCOPE 0x100u
#define MADOPILOT_DIAGNOSTIC_RECORD_HAS_EVIDENCE 0x200u
#define MADOPILOT_DIAGNOSTIC_RECORD_HAS_INPUT_FAULT 0x400u
#define MADOPILOT_DIAGNOSTIC_RECORD_HAS_STATUS 0x800u
#define MADOPILOT_DIAGNOSTIC_RECORD_HAS_PERMISSION_STATE 0x1000u

/* madopilot_error_detail_t optional fields. */
#define MADOPILOT_ERROR_HAS_ASSET_DETAIL 0x1u
#define MADOPILOT_ERROR_HAS_BACKEND 0x2u

/* ---------------------------------------------------------------------------
 * Borrowed views
 *
 * Nothing is NUL-terminated. A length the caller states is a length the library
 * can validate; a terminator it must search for is not.
 *
 * `data` may be null only when `len` is zero. A null pointer with a nonzero
 * length is rejected before the pointer is read.
 *
 * These two carry no struct_size. They are the boundary's primitives rather
 * than extensible records: they appear inside other structures, so growing one
 * would move every field after it.
 * ------------------------------------------------------------------------ */

typedef struct madopilot_str_t {
    const char* data; /* UTF-8, not NUL-terminated. */
    size_t len;
} madopilot_str_t;

typedef struct madopilot_bytes_t {
    const uint8_t* data;
    size_t len;
} madopilot_bytes_t;

/* ---------------------------------------------------------------------------
 * Opaque handles
 *
 * Every handle is reference counted. `*_retain` adds one owned reference and
 * `*_release` drops one; both accept null as a no-op and return
 * MADOPILOT_STATUS_OK for it, so a cleanup path can release whatever it has
 * without knowing how far construction got, and without special-casing the
 * status. Every other operation rejects null, because "do nothing" is not an
 * answer to a question that has one.
 *
 * A handle passed to a call must stay retained for the whole call. Releasing
 * the last reference concurrently with a call that has not retained one of its
 * own is outside this contract.
 *
 * Const access is safe from several threads at once while each keeps a live
 * reference.
 *
 * The four entries that take no handle — madopilot_get_api, describe_build,
 * clock_now, and status_text — are safe to call from any thread at any time,
 * concurrently with each other and with any other entry, and are safe to call
 * again from inside a call to any of them. They read immutable state or the
 * platform clock and own nothing that another call could be using.
 * ------------------------------------------------------------------------ */

typedef struct madopilot_cancellation_t madopilot_cancellation_t;
typedef struct madopilot_error_t madopilot_error_t;
typedef struct madopilot_engine_t madopilot_engine_t;
typedef struct madopilot_target_list_t madopilot_target_list_t;
typedef struct madopilot_package_t madopilot_package_t;
typedef struct madopilot_template_t madopilot_template_t;
typedef struct madopilot_session_t madopilot_session_t;
typedef struct madopilot_frame_t madopilot_frame_t;
typedef struct madopilot_mapping_t madopilot_mapping_t;
typedef struct madopilot_result_t madopilot_result_t;
typedef struct madopilot_input_receipt_t madopilot_input_receipt_t;
typedef struct madopilot_diagnostic_reader_t madopilot_diagnostic_reader_t;
typedef struct madopilot_diagnostic_batch_t madopilot_diagnostic_batch_t;

/* ---------------------------------------------------------------------------
 * Structures
 *
 * Every extensible structure begins with `uint32_t struct_size`, immediately
 * followed by a second 32-bit field so that no implicit padding is introduced
 * between them.
 *
 * A caller sets struct_size to sizeof the structure as THIS header declares it.
 * The library reads only the fields that size covers, applies the documented
 * default to every omitted optional field, and ignores trailing bytes it does
 * not recognize. A size below the documented mandatory prefix is
 * MADOPILOT_STATUS_INVALID_ARGUMENT, and nothing past struct_size is read even
 * to check it.
 *
 * A size describes a prefix, so it also has to end where a prefix can end.
 * Three further sizes are MADOPILOT_STATUS_INVALID_ARGUMENT for that reason,
 * each refused rather than adjusted to something nearby: a struct_size that
 * stops inside a field of the structure the library declares, because the field
 * would be neither supplied nor omitted — a size at or above the library's own
 * comes from a newer header, so its extra bytes are the trailing bytes above
 * rather than a field that library could place; an element of a caller-declared
 * array whose struct_size is above that array's element stride, because the two
 * declarations describe different extents; and a struct_size that does not
 * reach a field whose presence bit is set, because the field the bit names would
 * carry the omitted-field default under the caller's own claim that it was
 * supplied. Every size this header declares satisfies all three.
 *
 * For an output structure the same size is a promise in the other direction:
 * the library writes only within it, and writes back the number of bytes it
 * actually populated. A caller built against a newer header therefore learns
 * how much of what it knows is really there.
 * ------------------------------------------------------------------------ */

/* A half-open rectangle: [left, right) x [top, bottom). Not versioned.
 *
 * `space` is read in one direction and written in the other. On a rectangle the
 * library WRITES it names whichever space the library measured that rectangle
 * in. On a rectangle a caller SUPPLIES — the region fields of
 * madopilot_map_request_t and madopilot_find_request_t — this table accepts
 * MADOPILOT_SPACE_CAPTURE_PIXELS only, and any other space is
 * MADOPILOT_STATUS_INVALID_ARGUMENT. The ABI has no general
 * coordinate-conversion entry, so a caller converts before it asks. */
typedef struct madopilot_pixel_rect_t {
    madopilot_space_t space;
    int32_t left;
    int32_t top;
    int32_t right;
    int32_t bottom;
} madopilot_pixel_rect_t;

/* Capability flags that apply to the whole engine. Mandatory prefix: whole. */
typedef struct madopilot_engine_capabilities_t {
    uint32_t struct_size;
    uint32_t flags; /* MADOPILOT_ENGINE_{DELIVERS_INPUT,READS_PERMISSIONS} */
} madopilot_engine_capabilities_t;
/* Engine-wide diagnostic configuration. Mandatory prefix: whole. Off requires
 * zero capacity; enabled levels accept capacities from 1 through 65,536. */
typedef struct madopilot_engine_options_t {
    uint32_t struct_size;
    uint32_t flags; /* No bits defined; the caller sets zero. */
    madopilot_diagnostic_level_t diagnostic_level;
    uint32_t diagnostic_capacity;
} madopilot_engine_options_t;

/* Summary of one immutable owned diagnostic batch. */
typedef struct madopilot_diagnostic_batch_info_t {
    uint32_t struct_size;
    uint32_t flags; /* No bits defined; written as zero. */
    uint64_t record_count;
    uint64_t discarded_normal;
    uint64_t discarded_debug;
} madopilot_diagnostic_batch_info_t;

/* One non-prompting permission-probe result. Mandatory prefix: through state.
 * Borrowed strings remain valid while the engine is retained. */
typedef struct madopilot_permission_t {
    uint32_t struct_size;
    uint32_t flags; /* MADOPILOT_PERMISSION_HAS_* */
    madopilot_permission_kind_t kind;
    madopilot_permission_state_t state;
    madopilot_diagnostic_category_t diagnostic_category;
    uint32_t reserved; /* Written as zero. */
    int64_t platform_code;
    madopilot_str_t platform_namespace;
    madopilot_str_t context;
} madopilot_permission_t;

/* Capability data for one operation/route pair on one target.
 * Mandatory prefix: through support. */
typedef struct madopilot_input_capability_t {
    uint32_t struct_size;
    uint32_t flags; /* MADOPILOT_INPUT_CAPABILITY_HAS_* */
    uint64_t target;
    madopilot_input_operation_kind_t operation;
    madopilot_input_delivery_t delivery;
    madopilot_capability_support_t support;
    madopilot_input_address_scope_t address_scope;
    madopilot_permission_kind_t permission;
    madopilot_submission_evidence_t evidence;
    int32_t focus_required;
    uint32_t pointer_spaces; /* Bit (1 << space) for accepted pointer spaces. */
    uint32_t reserved;
} madopilot_input_capability_t;

/* Input requested while opening capture. Mandatory prefix: through requirement. */
typedef struct madopilot_input_open_request_t {
    uint32_t struct_size;
    uint32_t flags; /* No bits defined; the caller sets zero. */
    madopilot_input_requirement_t requirement;
    uint32_t reserved;
    uint64_t required_pairs;
    uint64_t preferred_pairs;
} madopilot_input_open_request_t;

/* What input an engine or open session accepts. Mandatory prefix: whole. */
typedef struct madopilot_input_descriptor_t {
    uint32_t struct_size;
    uint32_t flags; /* No bits defined; written as zero. */
    uint64_t target;
    uint64_t known_pairs;
    uint64_t supported_pairs;
    uint64_t unknown_pairs;
    uint32_t pointer_spaces;
    uint32_t max_events;
} madopilot_input_descriptor_t;

/* One bounded input event. The mandatory prefix varies by kind. */
typedef struct madopilot_input_event_t {
    uint32_t struct_size;
    madopilot_input_event_kind_t kind;
    madopilot_space_t space;
    madopilot_pointer_button_t button;
    madopilot_key_t key;
    uint32_t key_value;
    double x;
    double y;
    int32_t horizontal;
    int32_t vertical;
    madopilot_str_t text;
    uint64_t delay_nanos;
} madopilot_input_event_t;

/* One bounded input sequence and explicit route plan. Mandatory prefix:
 * through source_frame. Arrays and event text are borrowed for the call. */
typedef struct madopilot_input_request_t {
    uint32_t struct_size;
    uint32_t flags; /* MADOPILOT_INPUT_REQUEST_HAS_CLEANUP_BUDGET */
    const madopilot_input_event_t* events;
    size_t event_count;
    size_t event_stride;
    const madopilot_input_delivery_t* deliveries;
    size_t delivery_count;
    madopilot_focus_policy_t focus_policy;
    madopilot_geometry_policy_t geometry_policy;
    const madopilot_frame_t* source_frame;
    uint32_t cleanup_max_events;
    uint32_t reserved;
    uint64_t cleanup_timeout_nanos;
} madopilot_input_request_t;

/* Fixed fields of one immutable owned receipt. Route attempts are indexed
 * separately from the receipt handle. Mandatory prefix: whole. */
typedef struct madopilot_input_receipt_info_t {
    uint32_t struct_size;
    uint32_t flags; /* MADOPILOT_INPUT_RECEIPT_* */
    uint64_t target;
    madopilot_sequence_outcome_t outcome;
    madopilot_input_delivery_t selected_route;
    madopilot_input_address_scope_t address_scope;
    uint32_t attempt_count;
    uint32_t submitted;
    uint32_t last_submitted;
    madopilot_submission_evidence_t evidence;
    madopilot_input_fault_t fault;
    madopilot_cleanup_state_t cleanup;
    uint32_t cleanup_released;
    uint32_t cleanup_owed;
} madopilot_input_receipt_info_t;

/* One immutable route attempt borrowed from its receipt. */
typedef struct madopilot_input_attempt_t {
    uint32_t struct_size;
    uint32_t flags; /* MADOPILOT_INPUT_ATTEMPT_* */
    madopilot_input_delivery_t route;
    madopilot_input_address_scope_t address_scope;
    madopilot_sequence_outcome_t outcome;
    uint32_t submitted;
    uint32_t last_submitted;
    madopilot_submission_evidence_t evidence;
    madopilot_input_fault_t fault;
    uint32_t reserved;
} madopilot_input_attempt_t;

/* What the loaded library is. Mandatory prefix: through table_size. */
typedef struct madopilot_build_info_t {
    uint32_t struct_size;
    uint32_t flags; /* No bits defined; the library writes zero. */
    uint32_t abi_major;
    uint32_t abi_minor;
    uint32_t table_size; /* sizeof the library's own function table. */
    uint32_t reserved;   /* Alignment padding. Written as zero. */
    madopilot_str_t library_version;   /* Static; valid while loaded. */
    madopilot_str_t required_backend;  /* Static; valid while loaded. */
} madopilot_build_info_t;

/* A deadline and a cancellation token. Mandatory prefix: through flags.
 *
 * The deadline is an ABSOLUTE instant in the library's monotonic domain, in
 * nanoseconds since an origin fixed for the life of the loaded library. Read
 * the current instant with clock_now and add to it. It is not a wall clock and
 * must not be presented as one. */
typedef struct madopilot_operation_t {
    uint32_t struct_size;
    uint32_t flags; /* MADOPILOT_OPERATION_HAS_* */
    uint64_t deadline_nanos;
    const madopilot_cancellation_t* cancellation; /* Borrowed, may be null. */
    uint64_t activity_tag; /* Opaque nonzero diagnostic correlation tag. */
} madopilot_operation_t;

/* The complete public identity of one published frame.
 *
 * Mandatory prefix: the whole structure. Identity is not optional; a caller
 * that cannot store all four fields cannot correlate a result at all.
 *
 * `stream` is unique for the life of the loaded library and is never reused. */
typedef struct madopilot_frame_stamp_t {
    uint32_t struct_size;
    uint32_t flags; /* No bits defined; the library writes zero. */
    uint64_t stream;
    uint64_t epoch;
    uint64_t sequence;
    uint64_t geometry;
} madopilot_frame_stamp_t;
/* One privacy-reviewed immutable diagnostic record. Presence flags distinguish
 * optional scalar values from valid zero values. Mandatory prefix: whole. */
typedef struct madopilot_diagnostic_record_t {
    uint32_t struct_size;
    uint32_t flags; /* MADOPILOT_DIAGNOSTIC_RECORD_HAS_* */
    uint64_t sequence;
    uint64_t timestamp_nanos;
    uint64_t operation_id;
    uint64_t activity_tag;
    madopilot_diagnostic_level_t level;
    madopilot_diagnostic_kind_t kind;
    madopilot_diagnostic_operation_kind_t operation;
    madopilot_status_t status;
    uint64_t target;
    madopilot_frame_stamp_t frame;
    uint64_t template_identity;
    madopilot_space_t source_space;
    madopilot_space_t destination_space;
    madopilot_space_t region_space;
    madopilot_input_delivery_t route;
    madopilot_input_address_scope_t address_scope;
    madopilot_submission_evidence_t evidence;
    madopilot_input_fault_t input_fault;
    madopilot_sequence_outcome_t input_outcome;
    madopilot_cleanup_state_t cleanup;
    madopilot_permission_kind_t permission_kind;
    madopilot_permission_state_t permission_state;
    madopilot_lifecycle_t lifecycle;
    madopilot_search_diagnostic_outcome_t search_outcome;
    uint32_t input_operations;
    int32_t partial_native_effect;
    int32_t used_fallback;
    uint32_t reserved;
    uint64_t requested;
    uint64_t submitted;
    uint64_t result_count;
    uint64_t cleanup_released;
    uint64_t cleanup_owed;
} madopilot_diagnostic_record_t;


/* A frame's pixel geometry. Mandatory prefix: through space. */
typedef struct madopilot_frame_info_t {
    uint32_t struct_size;
    uint32_t flags; /* No bits defined; the library writes zero. */
    uint32_t width;
    uint32_t height;
    madopilot_pixel_format_t format;
    madopilot_space_t space; /* The space `bounds` is measured in. */
    uint64_t stride;
    madopilot_pixel_rect_t bounds;
} madopilot_frame_info_t;

/* A completed mapping's descriptor and its borrowed bytes.
 *
 * Mandatory prefix: through bytes. `bytes` stays readable while the mapping
 * handle is retained and becomes invalid at its final release. */
typedef struct madopilot_image_t {
    uint32_t struct_size;
    uint32_t flags; /* MADOPILOT_IMAGE_SHARED */
    uint32_t width;
    uint32_t height;
    madopilot_pixel_format_t format;
    madopilot_space_t space; /* The space `region` is measured in. */
    uint64_t stride;
    madopilot_bytes_t bytes; /* Borrowed from the mapping handle. */
    madopilot_pixel_rect_t region;
} madopilot_image_t;

/* One discovered capture target. Mandatory prefix: through coordinate_spaces.
 *
 * `name` and `provider` are borrowed from the target-list handle. */
typedef struct madopilot_target_t {
    uint32_t struct_size;
    uint32_t flags; /* MADOPILOT_TARGET_{SUPPORTS_PLACEMENT,HAS_*} */
    uint32_t width;
    uint32_t height;
    madopilot_pixel_format_t format;
    /* A bit set: bit (1 << space) is set when that space converts. */
    int32_t coordinate_spaces;
    madopilot_str_t name;
    madopilot_str_t provider;
    uint64_t target; /* Engine-local target identity. */
    madopilot_target_kind_t kind;
    madopilot_capability_support_t capture;
    madopilot_permission_kind_t capture_permission;
    uint32_t reserved; /* Written as zero. */
} madopilot_target_t;

/* What an open session accepted. Mandatory prefix: through coordinate_spaces.
 * ABI 1.2 appends target, accepts_input, and reserved without moving that prefix. */
typedef struct madopilot_session_info_t {
    uint32_t struct_size;
    uint32_t flags; /* No bits defined; the library writes zero. */
    uint64_t stream; /* Same domain as madopilot_frame_stamp_t.stream. */
    uint32_t width;
    uint32_t height;
    madopilot_pixel_format_t format;
    /* A bit set: bit (1 << space) is set when that coordinate space converts.
     * Read as a madopilot_space_t it gives a plausible wrong answer: the value
     * 1 is both "capture pixels converts" and SPACE_FRAME_NORMALIZED. */
    int32_t coordinate_spaces;
    uint64_t target; /* Boundary identity copied from discovery. */
    int32_t accepts_input; /* One when input was established, otherwise zero. */
    uint32_t reserved; /* Written as zero. */
} madopilot_session_info_t;

/* How to open a capture session. Mandatory prefix: through flags.
 *
 * Without either format bit the adapter's own layout is accepted. Input policy
 * is supplied separately to the ABI 1.2 session_open_with_input entry. */
typedef struct madopilot_open_request_t {
    uint32_t struct_size;
    uint32_t flags; /* MADOPILOT_OPEN_HAS_* */
    madopilot_pixel_format_t required_format;
    madopilot_pixel_format_t preferred_format;
} madopilot_open_request_t;

/* How to map a frame. Mandatory prefix: through format.
 *
 * Omitting clip_policy rejects a region that leaves the frame; omitting the
 * region maps the whole frame. */
typedef struct madopilot_map_request_t {
    uint32_t struct_size;
    uint32_t flags; /* MADOPILOT_MAP_HAS_REGION */
    madopilot_pixel_format_t format;
    madopilot_clip_policy_t clip_policy;
    /* MADOPILOT_SPACE_CAPTURE_PIXELS only; any other space is
     * MADOPILOT_STATUS_INVALID_ARGUMENT. */
    madopilot_pixel_rect_t region;
} madopilot_map_request_t;

/* The thresholds one search runs under.
 *
 * Mandatory prefix as an INPUT: through flags. Every omitted field defaults to
 * the prepared template's own declared default, so a structure that sets no
 * presence bit is the documented way to ask for exactly those defaults.
 *
 * Mandatory prefix as an OUTPUT: the whole structure. This is the only
 * structure the table uses in both directions, and the only one whose two
 * prefixes differ. result_options reports the thresholds the search really ran
 * under, where every field was in effect and the library sets every presence
 * bit; a shorter report would drop one of them silently. So pass
 * sizeof(madopilot_match_options_t) there — eight bytes is a valid input size
 * and is MADOPILOT_STATUS_INVALID_ARGUMENT as an output size. */
typedef struct madopilot_match_options_t {
    uint32_t struct_size;
    uint32_t flags; /* MADOPILOT_MATCH_HAS_* */
    double min_score;
    uint32_t max_results;
    madopilot_suppression_t suppression;
} madopilot_match_options_t;

/* One template search. Mandatory prefix: through tmpl.
 *
 * Omitting clip_policy rejects a region that leaves the frame, the same default
 * madopilot_map_request_t states for its own field. Only a struct_size of 52
 * through 55 omits it, which no released header declares, but the two fields
 * are identical and stating the default for one and not the other invites the
 * reader to think they differ.
 *
 * The field is `tmpl` rather than `template` because the C++ wrapper includes
 * this header and that word is a keyword there. */
typedef struct madopilot_find_request_t {
    uint32_t struct_size;
    uint32_t flags; /* MADOPILOT_FIND_HAS_REGION */
    /* The exact frame to search, or null for the session's latest frame.
     * These are different questions: "what is on screen now" and "the frame I
     * mapped a moment ago" diverge as soon as a second frame is published. */
    const madopilot_frame_t* frame;
    const madopilot_template_t* tmpl;         /* Required. */
    const madopilot_match_options_t* options; /* Null: template defaults. */
    /* MADOPILOT_SPACE_CAPTURE_PIXELS only; any other space is
     * MADOPILOT_STATUS_INVALID_ARGUMENT. */
    madopilot_pixel_rect_t region;
    madopilot_clip_policy_t clip_policy;
} madopilot_find_request_t;

/* One match. Mandatory prefix: the whole structure.
 *
 * `template_id` is borrowed from the result handle. */
typedef struct madopilot_match_t {
    uint32_t struct_size;
    uint32_t flags; /* No bits defined; the library writes zero. */
    double score;
    madopilot_str_t template_id;
    madopilot_pixel_rect_t bounds;
} madopilot_match_t;

/* What one completed search produced. Mandatory prefix: the whole structure.
 *
 * Both backend views are borrowed from the result handle. A match_count of zero
 * is a successful answer, not a failure. */
typedef struct madopilot_result_info_t {
    uint32_t struct_size;
    uint32_t flags; /* No bits defined; the library writes zero. */
    uint64_t match_count;
    madopilot_str_t backend_id;
    madopilot_str_t backend_version;
    madopilot_pixel_rect_t searched;
} madopilot_result_info_t;

/* What a loaded package declares. Mandatory prefix: the whole structure.
 *
 * Every view is borrowed from the package handle. */
typedef struct madopilot_package_info_t {
    uint32_t struct_size;
    uint32_t flags; /* No bits defined; the library writes zero. */
    uint64_t template_count;
    madopilot_str_t package_id;
    madopilot_str_t package_version;
    madopilot_str_t license;
} madopilot_package_info_t;

/* What a prepared template is. Mandatory prefix: the whole structure.
 *
 * Both views are borrowed from the template handle. */
typedef struct madopilot_template_info_t {
    uint32_t struct_size;
    uint32_t flags; /* No bits defined; the library writes zero. */
    uint32_t width;
    uint32_t height;
    double min_score;
    madopilot_str_t id;
    madopilot_str_t backend;
    uint32_t max_results;
    madopilot_space_t space;
} madopilot_template_info_t;

/* A failure, in structured form. Mandatory prefix: through category.
 *
 * `message` and `backend` are borrowed from the error handle and become invalid
 * at its final release: copy anything you still need first. The message is
 * redacted diagnostic text and never contains captured pixels or recognized
 * text; it is never required for control flow. */
typedef struct madopilot_error_detail_t {
    uint32_t struct_size;
    uint32_t flags; /* MADOPILOT_ERROR_HAS_ASSET_DETAIL, _HAS_BACKEND */
    madopilot_status_t status;
    madopilot_error_category_t category;
    madopilot_asset_fault_t asset_fault; /* Valid with _HAS_ASSET_DETAIL. */
    madopilot_asset_stage_t asset_stage; /* Valid with _HAS_ASSET_DETAIL. */
    madopilot_str_t message;
    madopilot_str_t backend; /* Valid with _HAS_BACKEND. */
} madopilot_error_detail_t;

/* One replay frame supplied as raw pixels. Mandatory prefix: through pixels.
 *
 * Omitting captured_at_nanos places the frame at the clock origin; omitting
 * stride means packed rows. The pixels are copied during engine construction,
 * so the caller may release its own storage as soon as the call returns. */
typedef struct madopilot_replay_frame_t {
    uint32_t struct_size;
    uint32_t flags; /* No bits defined; set zero. */
    uint32_t width;
    uint32_t height;
    madopilot_pixel_format_t format;
    madopilot_continuity_t continuity;
    madopilot_bytes_t pixels;
    uint64_t captured_at_nanos;
    uint64_t stride;
} madopilot_replay_frame_t;

/* Where an engine's frames come from. Mandatory prefix: through frame_stride.
 *
 * `kind` selects which of the remaining fields are read; the others are ignored
 * entirely. frame_stride is sizeof(madopilot_replay_frame_t) as THIS header
 * declares it, and is the stride between the elements of `frames`. It is a
 * separate field because a caller built against an older header has smaller
 * elements, and the library cannot guess the spacing of an array it did not
 * declare. */
typedef struct madopilot_source_t {
    uint32_t struct_size;
    madopilot_source_kind_t kind;
    madopilot_str_t directory; /* MADOPILOT_SOURCE_REPLAY_DIRECTORY */
    const madopilot_replay_frame_t* frames; /* MADOPILOT_SOURCE_REPLAY_MEMORY */
    size_t frame_count;
    size_t frame_stride;
    madopilot_str_t target_name; /* Empty for a default. */
} madopilot_source_t;

/* Where an asset package is read from. Mandatory prefix: through path.
 *
 * ARCHIVE_BYTES is read where it is: the library makes no whole-archive copy,
 * and the view has to be readable and unchanged for the duration of the call.
 * A package the call returns owns everything it kept, so the caller may release
 * or overwrite the archive the moment the call comes back. The declared length
 * answers to the engine's configured source-byte ceiling, so a length above it
 * reports MADOPILOT_STATUS_LIMIT_EXCEEDED with the archive-limit asset fault at
 * the source stage, before the view behind it is read at all. The load answers
 * to the operation, so a cancellation or a deadline that lands before the
 * package is published reports that outcome and publishes nothing. A malformed
 * view — a null pointer carrying a length — is
 * MADOPILOT_STATUS_INVALID_ARGUMENT with MADOPILOT_ERROR_CATEGORY_ABI, whatever
 * the length says. */
typedef struct madopilot_package_source_t {
    uint32_t struct_size;
    madopilot_package_source_kind_t kind;
    madopilot_str_t path;      /* DIRECTORY, ARCHIVE_FILE */
    madopilot_bytes_t archive; /* ARCHIVE_BYTES */
} madopilot_package_source_t;

/* ---------------------------------------------------------------------------
 * The function table
 *
 * Within an ABI major a member's position is permanent. Later phases append;
 * nothing is reordered, removed, repurposed, or reserved as a null slot for
 * work that does not exist yet.
 *
 * Every member returns madopilot_status_t and reports values only through
 * output parameters, which are initialized to their documented failure state
 * before the request is validated: owned handle outputs to null, structures
 * through their failure prefix, scalars to zero. On failure they stay that way.
 *
 * Every member catches a panic before it can cross into C.
 *
 * Every pointer parameter is required unless its declaration says otherwise,
 * and a null one is MADOPILOT_STATUS_INVALID_ARGUMENT. That covers the request,
 * source, and operation structures as well as the handles: passing a null
 * `operation` to mean "no deadline" is refused, and the way to say that is a
 * madopilot_operation_t whose `flags` set no bit. An absent operation structure
 * and one that declares nothing are different requests, and only the second
 * says which header the caller was built against.
 *
 * Beyond the retain and release entries and the empty-view rule above, null is
 * accepted in four places, each documented where it is declared: `out_error`
 * below, madopilot_operation_t.cancellation, madopilot_find_request_t.frame,
 * and madopilot_find_request_t.options.
 *
 * Every structure a caller passes in, every view one of those structures
 * carries, and every view passed directly as an argument — a template identity,
 * a target name — must be readable for the duration of the call and must not be
 * modified during it, whether the library reads it once or reads it throughout.
 * The library never retains a caller's memory past the call that received it:
 * anything it must keep, it copies or converts into storage of its own. This is
 * the input counterpart of the output rule above, where a view the library owns
 * stays valid while its handle is retained.
 *
 * An entry that takes `out_error` may be passed null there, and then reports
 * the status only. A returned error is owned by the caller and is released with
 * error_release.
 *
 * That includes a fault about an output argument. A null or misaligned output is
 * an invalid argument like any other, and MADOPILOT_STATUS_INVALID_ARGUMENT does
 * not say which of an entry's pointers was wrong, so an entry whose `out_error`
 * passed validation reports through it — after clearing it — which output it
 * refused. A call whose `out_error` is itself the refused output gets the status
 * alone.
 * ------------------------------------------------------------------------ */

typedef struct madopilot_api_t {
    uint32_t struct_size; /* sizeof this table as the library declares it. */
    uint32_t abi_major;
    uint32_t abi_minor;
    uint32_t reserved; /* Alignment padding. Zero. */

    /* --- Information -------------------------------------------------- */

    madopilot_status_t (*describe_build)(madopilot_build_info_t* out_info);

    /* The current instant in the library's monotonic domain. Add to it to build
     * the absolute deadline an operation structure carries. */
    madopilot_status_t (*clock_now)(uint64_t* out_nanos);

    /* A stable lowercase slug for a status, borrowed from static storage. */
    madopilot_status_t (*status_text)(madopilot_status_t status,
                                      madopilot_str_t* out_text);

    /* --- Cancellation --------------------------------------------------- */

    madopilot_status_t (*cancellation_create)(
        madopilot_cancellation_t** out_cancellation);
    madopilot_status_t (*cancellation_retain)(
        const madopilot_cancellation_t* cancellation);
    madopilot_status_t (*cancellation_release)(
        madopilot_cancellation_t* cancellation);
    madopilot_status_t (*cancellation_cancel)(
        const madopilot_cancellation_t* cancellation);
    madopilot_status_t (*cancellation_is_cancelled)(
        const madopilot_cancellation_t* cancellation, int32_t* out_cancelled);

    /* --- Errors --------------------------------------------------------- */

    madopilot_status_t (*error_retain)(const madopilot_error_t* error);
    madopilot_status_t (*error_release)(madopilot_error_t* error);
    madopilot_status_t (*error_describe)(const madopilot_error_t* error,
                                         madopilot_error_detail_t* out_detail);

    /* --- Engine --------------------------------------------------------- */

    madopilot_status_t (*engine_create)(const madopilot_source_t* source,
                                        const madopilot_operation_t* operation,
                                        madopilot_engine_t** out_engine,
                                        madopilot_error_t** out_error);
    madopilot_status_t (*engine_retain)(const madopilot_engine_t* engine);
    madopilot_status_t (*engine_release)(madopilot_engine_t* engine);

    /* --- Assets and templates ------------------------------------------- */

    madopilot_status_t (*package_load)(
        const madopilot_engine_t* engine,
        const madopilot_package_source_t* source,
        const madopilot_operation_t* operation,
        madopilot_package_t** out_package,
        madopilot_error_t** out_error);
    madopilot_status_t (*package_retain)(const madopilot_package_t* package);
    madopilot_status_t (*package_release)(madopilot_package_t* package);
    madopilot_status_t (*package_describe)(const madopilot_package_t* package,
                                           madopilot_package_info_t* out_info);
    madopilot_status_t (*package_template_id)(
        const madopilot_package_t* package, size_t index,
        madopilot_str_t* out_id);
    madopilot_status_t (*template_prepare_from_package)(
        const madopilot_engine_t* engine, const madopilot_package_t* package,
        madopilot_str_t id, const madopilot_operation_t* operation,
        madopilot_template_t** out_template, madopilot_error_t** out_error);
    madopilot_status_t (*template_retain)(const madopilot_template_t* tmpl);
    madopilot_status_t (*template_release)(madopilot_template_t* tmpl);
    madopilot_status_t (*template_describe)(
        const madopilot_template_t* tmpl, madopilot_template_info_t* out_info);

    /* --- Discovery ------------------------------------------------------ */

    madopilot_status_t (*engine_discover)(
        const madopilot_engine_t* engine,
        const madopilot_operation_t* operation,
        madopilot_target_list_t** out_targets, madopilot_error_t** out_error);
    madopilot_status_t (*target_list_retain)(
        const madopilot_target_list_t* targets);
    madopilot_status_t (*target_list_release)(madopilot_target_list_t* targets);
    madopilot_status_t (*target_list_count)(
        const madopilot_target_list_t* targets, size_t* out_count);
    /* An index at or beyond the count is invalid argument and leaves the output
     * in its failure state. */
    madopilot_status_t (*target_list_get)(
        const madopilot_target_list_t* targets, size_t index,
        madopilot_target_t* out_target);

    /* --- Session -------------------------------------------------------- */

    /* Copies the target identity, so the list may be released immediately. */
    madopilot_status_t (*session_open)(
        const madopilot_engine_t* engine,
        const madopilot_target_list_t* targets, size_t index,
        const madopilot_open_request_t* request,
        const madopilot_operation_t* operation,
        madopilot_session_t** out_session, madopilot_error_t** out_error);
    madopilot_status_t (*session_retain)(const madopilot_session_t* session);
    /* Releasing a session does not close it, and does not invalidate frames,
     * mappings, or results the caller still holds. */
    madopilot_status_t (*session_release)(madopilot_session_t* session);
    madopilot_status_t (*session_describe)(const madopilot_session_t* session,
                                           madopilot_session_info_t* out_info);
    /* Idempotent. Work starting after close returns MADOPILOT_STATUS_CLOSED. */
    madopilot_status_t (*session_close)(const madopilot_session_t* session,
                                        const madopilot_operation_t* operation,
                                        madopilot_error_t** out_error);
    madopilot_status_t (*session_is_closed)(const madopilot_session_t* session,
                                            int32_t* out_closed);

    /* --- Frames and mapping --------------------------------------------- */

    madopilot_status_t (*session_acquire_frame)(const madopilot_session_t* session,
                                        const madopilot_operation_t* operation,
                                        madopilot_frame_t** out_frame,
                                        madopilot_error_t** out_error);
    madopilot_status_t (*frame_retain)(const madopilot_frame_t* frame);
    madopilot_status_t (*frame_release)(madopilot_frame_t* frame);
    madopilot_status_t (*frame_stamp)(const madopilot_frame_t* frame,
                                      madopilot_frame_stamp_t* out_stamp);
    madopilot_status_t (*frame_describe)(const madopilot_frame_t* frame,
                                         madopilot_frame_info_t* out_info);
    madopilot_status_t (*frame_map)(const madopilot_frame_t* frame,
                                    const madopilot_map_request_t* request,
                                    const madopilot_operation_t* operation,
                                    madopilot_mapping_t** out_mapping,
                                    madopilot_error_t** out_error);
    madopilot_status_t (*mapping_retain)(const madopilot_mapping_t* mapping);
    /* At the final release every byte view borrowed from this mapping becomes
     * invalid and the retained storage is released exactly once. */
    madopilot_status_t (*mapping_release)(madopilot_mapping_t* mapping);
    madopilot_status_t (*mapping_describe)(const madopilot_mapping_t* mapping,
                                           madopilot_image_t* out_image);
    madopilot_status_t (*mapping_stamp)(const madopilot_mapping_t* mapping,
                                        madopilot_frame_stamp_t* out_stamp);

    /* --- Matching and results ------------------------------------------- */

    /* A completed search with no qualifying match succeeds with a count of
     * zero. It is an answer, not a failure. */
    madopilot_status_t (*session_find)(const madopilot_session_t* session,
                                       const madopilot_find_request_t* request,
                                       const madopilot_operation_t* operation,
                                       madopilot_result_t** out_result,
                                       madopilot_error_t** out_error);
    madopilot_status_t (*result_retain)(const madopilot_result_t* result);
    madopilot_status_t (*result_release)(madopilot_result_t* result);
    madopilot_status_t (*result_describe)(const madopilot_result_t* result,
                                          madopilot_result_info_t* out_info);
    /* The complete identity of the frame that was searched. It stays answerable
     * after the session, template, package, and engine are gone: the result
     * owns the frame it searched. */
    madopilot_status_t (*result_stamp)(const madopilot_result_t* result,
                                       madopilot_frame_stamp_t* out_stamp);
    /* The options the search actually ran under, not the ones requested.
     *
     * madopilot_match_options_t's mandatory prefix HERE is the whole structure,
     * not the eight bytes it accepts as an input. */
    madopilot_status_t (*result_options)(
        const madopilot_result_t* result,
        madopilot_match_options_t* out_options);
    /* An index at or beyond the count is invalid argument and leaves the output
     * in its failure state. */
    madopilot_status_t (*result_match)(const madopilot_result_t* result,
                                       size_t index,
                                       madopilot_match_t* out_match);

    /* --- ABI 1.2 input and bounded diagnostic suffix -------------------- */

    madopilot_status_t (*engine_create_with_options)(
        const madopilot_source_t* source,
        const madopilot_engine_options_t* options,
        const madopilot_operation_t* operation,
        madopilot_engine_t** out_engine,
        madopilot_error_t** out_error);
    madopilot_status_t (*engine_capabilities)(
        const madopilot_engine_t* engine,
        madopilot_engine_capabilities_t* out_capabilities);
    madopilot_status_t (*engine_permission)(
        const madopilot_engine_t* engine,
        madopilot_permission_kind_t kind,
        const madopilot_operation_t* operation,
        madopilot_permission_t* out_permission,
        madopilot_error_t** out_error);
    madopilot_status_t (*target_list_input_capability)(
        const madopilot_target_list_t* targets,
        size_t index,
        madopilot_input_operation_kind_t operation,
        madopilot_input_delivery_t delivery,
        madopilot_input_capability_t* out_capability);
    madopilot_status_t (*engine_input_descriptor)(
        const madopilot_engine_t* engine,
        const madopilot_target_list_t* targets,
        size_t index,
        const madopilot_operation_t* operation,
        madopilot_input_descriptor_t* out_descriptor,
        madopilot_error_t** out_error);
    madopilot_status_t (*session_open_with_input)(
        const madopilot_engine_t* engine,
        const madopilot_target_list_t* targets,
        size_t index,
        const madopilot_open_request_t* request,
        const madopilot_input_open_request_t* input_request,
        const madopilot_operation_t* operation,
        madopilot_session_t** out_session,
        madopilot_error_t** out_error);
    madopilot_status_t (*session_input_descriptor)(
        const madopilot_session_t* session,
        madopilot_input_descriptor_t* out_descriptor);
    madopilot_status_t (*session_send_input)(
        const madopilot_session_t* session,
        const madopilot_input_request_t* request,
        const madopilot_operation_t* operation,
        madopilot_input_receipt_t** out_receipt,
        madopilot_error_t** out_error);
    madopilot_status_t (*input_receipt_retain)(
        const madopilot_input_receipt_t* receipt);
    madopilot_status_t (*input_receipt_release)(
        madopilot_input_receipt_t* receipt);
    madopilot_status_t (*input_receipt_info)(
        const madopilot_input_receipt_t* receipt,
        madopilot_input_receipt_info_t* out_info);
    madopilot_status_t (*input_receipt_attempt_count)(
        const madopilot_input_receipt_t* receipt,
        size_t* out_count);
    madopilot_status_t (*input_receipt_attempt_at)(
        const madopilot_input_receipt_t* receipt,
        size_t index,
        madopilot_input_attempt_t* out_attempt);
    madopilot_status_t (*engine_take_diagnostic_reader)(
        const madopilot_engine_t* engine,
        madopilot_diagnostic_reader_t** out_reader);
    madopilot_status_t (*diagnostic_reader_retain)(
        const madopilot_diagnostic_reader_t* reader);
    madopilot_status_t (*diagnostic_reader_release)(
        madopilot_diagnostic_reader_t* reader);
    madopilot_status_t (*diagnostic_reader_drain)(
        const madopilot_diagnostic_reader_t* reader,
        madopilot_diagnostic_drain_state_t* out_state,
        madopilot_diagnostic_batch_t** out_batch);
    madopilot_status_t (*diagnostic_batch_retain)(
        const madopilot_diagnostic_batch_t* batch);
    madopilot_status_t (*diagnostic_batch_release)(
        madopilot_diagnostic_batch_t* batch);
    madopilot_status_t (*diagnostic_batch_info)(
        const madopilot_diagnostic_batch_t* batch,
        madopilot_diagnostic_batch_info_t* out_info);
    madopilot_status_t (*diagnostic_batch_record_at)(
        const madopilot_diagnostic_batch_t* batch,
        size_t index,
        madopilot_diagnostic_record_t* out_record);
} madopilot_api_t;

/* The table's mandatory prefix: everything through status_text.
 *
 * A caller that knows less than this cannot report what it loaded and cannot
 * build a deadline, so negotiation refuses it rather than handing back a table
 * it could not use.
 *
 * Written as the offset of the member *after* the prefix, which is what a
 * prefix size is: where the next member begins. Adding sizeof(void*) to
 * status_text's own offset would have assumed a function pointer is the size of
 * an object pointer, and that no padding separates the two members. This
 * assumes neither, and it is the same number on both release targets. */
#define MADOPILOT_API_SIZE_INFORMATION offsetof(madopilot_api_t, cancellation_create)

/* Complete released ABI 1.0 prefix and additive ABI 1.2 entry extents. */
#define MADOPILOT_API_SIZE_ABI_1_0 \
    offsetof(madopilot_api_t, engine_create_with_options)
#define MADOPILOT_API_SIZE_ENGINE_CREATE_WITH_OPTIONS \
    offsetof(madopilot_api_t, engine_capabilities)
#define MADOPILOT_API_SIZE_ENGINE_CAPABILITIES \
    offsetof(madopilot_api_t, engine_permission)
#define MADOPILOT_API_SIZE_ENGINE_PERMISSION \
    offsetof(madopilot_api_t, target_list_input_capability)
#define MADOPILOT_API_SIZE_TARGET_LIST_INPUT_CAPABILITY \
    offsetof(madopilot_api_t, engine_input_descriptor)
#define MADOPILOT_API_SIZE_ENGINE_INPUT_DESCRIPTOR \
    offsetof(madopilot_api_t, session_open_with_input)
#define MADOPILOT_API_SIZE_SESSION_OPEN_WITH_INPUT \
    offsetof(madopilot_api_t, session_input_descriptor)
#define MADOPILOT_API_SIZE_SESSION_INPUT_DESCRIPTOR \
    offsetof(madopilot_api_t, session_send_input)
#define MADOPILOT_API_SIZE_SESSION_SEND_INPUT \
    offsetof(madopilot_api_t, input_receipt_retain)
#define MADOPILOT_API_SIZE_INPUT_RECEIPT_RETAIN \
    offsetof(madopilot_api_t, input_receipt_release)
#define MADOPILOT_API_SIZE_INPUT_RECEIPT_RELEASE \
    offsetof(madopilot_api_t, input_receipt_info)
#define MADOPILOT_API_SIZE_INPUT_RECEIPT_INFO \
    offsetof(madopilot_api_t, input_receipt_attempt_count)
#define MADOPILOT_API_SIZE_INPUT_RECEIPT_ATTEMPT_COUNT \
    offsetof(madopilot_api_t, input_receipt_attempt_at)
#define MADOPILOT_API_SIZE_INPUT_RECEIPT_ATTEMPT_AT \
    offsetof(madopilot_api_t, engine_take_diagnostic_reader)
#define MADOPILOT_API_SIZE_ENGINE_TAKE_DIAGNOSTIC_READER \
    offsetof(madopilot_api_t, diagnostic_reader_retain)
#define MADOPILOT_API_SIZE_DIAGNOSTIC_READER_RETAIN \
    offsetof(madopilot_api_t, diagnostic_reader_release)
#define MADOPILOT_API_SIZE_DIAGNOSTIC_READER_RELEASE \
    offsetof(madopilot_api_t, diagnostic_reader_drain)
#define MADOPILOT_API_SIZE_DIAGNOSTIC_READER_DRAIN \
    offsetof(madopilot_api_t, diagnostic_batch_retain)
#define MADOPILOT_API_SIZE_DIAGNOSTIC_BATCH_RETAIN \
    offsetof(madopilot_api_t, diagnostic_batch_release)
#define MADOPILOT_API_SIZE_DIAGNOSTIC_BATCH_RELEASE \
    offsetof(madopilot_api_t, diagnostic_batch_info)
#define MADOPILOT_API_SIZE_DIAGNOSTIC_BATCH_INFO \
    offsetof(madopilot_api_t, diagnostic_batch_record_at)
#define MADOPILOT_API_SIZE_DIAGNOSTIC_BATCH_RECORD_AT sizeof(madopilot_api_t)

/* ---------------------------------------------------------------------------
 * The one exported symbol
 * ------------------------------------------------------------------------ */

/*
 * Negotiates the ABI and returns the library's immutable function table.
 *
 * `abi_major`          MADOPILOT_ABI_MAJOR as this header declares it. A
 *                      different major is refused; it is a different library.
 * `min_abi_minor`      the oldest minor the caller can work with.
 * `caller_struct_size` sizeof(madopilot_api_t) as this header declares it, or a
 *                      smaller size ending at an earlier member. It must be at
 *                      least MADOPILOT_API_SIZE_INFORMATION.
 * `out_api`            receives the table, or null on failure. The table is
 *                      owned by the library, valid while it is loaded, and is
 *                      never released.
 *
 * Returns MADOPILOT_STATUS_OK, MADOPILOT_STATUS_UNSUPPORTED for an ABI this
 * library does not implement, or MADOPILOT_STATUS_INVALID_ARGUMENT for a null
 * output or a size below the mandatory prefix.
 *
 * Use the smaller of your sizeof and the returned table's struct_size to decide
 * which members exist: a library older than your header has fewer.
 */
MADOPILOT_EXPORT madopilot_status_t madopilot_get_api(
    uint32_t abi_major, uint32_t min_abi_minor, size_t caller_struct_size,
    const madopilot_api_t** out_api);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* MADOPILOT_MADOPILOT_H */
