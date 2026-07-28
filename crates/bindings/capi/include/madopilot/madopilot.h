/*
 * MadoPilot C ABI — Phase 1 prefix.
 *
 * MadoPilot is a headless visual automation runtime. This header is the whole
 * C contract: one exported symbol, an immutable function table reached through
 * it, opaque handles with a complete retain and release lifecycle, and
 * size-versioned structures with documented mandatory prefixes.
 *
 * ============================================================================
 * NOTHING HERE IS STABLE YET.
 *
 * Every status value, structure layout, field offset, and function-table
 * position in this header is PROVISIONAL. Gate `G-010` in
 * docs/validation-gates.md freezes them after the evidence this phase produces
 * has been reviewed, and until it does they may change without an ABI major
 * bump. Recompile against the header you link with; do not hard-code a number
 * you read here once.
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

/* The ABI minor version this header declares. A later minor only appends. */
#define MADOPILOT_ABI_MINOR 0u

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
     * call remain usable. */
    MADOPILOT_STATUS_INTERNAL_PANIC = 12
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
    MADOPILOT_ERROR_CATEGORY_GEOMETRY = 7
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

/* Which deterministic source an engine captures from. */
typedef int32_t madopilot_source_kind_t;

enum {
    MADOPILOT_SOURCE_REPLAY_MEMORY = 0,
    MADOPILOT_SOURCE_REPLAY_DIRECTORY = 1
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
 * This is the detail a single status cannot carry. Package loading is the one
 * Phase 1 operation whose failures a caller may reasonably want to tell apart
 * by more than their category: a bad content hash and an unsafe entry path are
 * both MADOPILOT_STATUS_ASSET_INVALID and are not the same problem. */
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

/* ---------------------------------------------------------------------------
 * Flags
 * ------------------------------------------------------------------------ */

/* madopilot_operation_t.deadline_nanos carries an absolute deadline. Without
 * it the operation has no deadline, which is not the same as a very large one:
 * zero nanoseconds is the domain origin and a valid instant. */
#define MADOPILOT_OPERATION_HAS_DEADLINE 0x1u

/* madopilot_open_request_t: which format fields are set. */
#define MADOPILOT_OPEN_HAS_REQUIRED_FORMAT 0x1u
#define MADOPILOT_OPEN_HAS_PREFERRED_FORMAT 0x2u

/* madopilot_map_request_t.region is set; the whole frame is mapped without it. */
#define MADOPILOT_MAP_HAS_REGION 0x1u

/* madopilot_find_request_t.region is set; the whole frame is searched
 * without it. */
#define MADOPILOT_FIND_HAS_REGION 0x1u

/* madopilot_match_options_t: which fields override the template's defaults. */
#define MADOPILOT_MATCH_HAS_MIN_SCORE 0x1u
#define MADOPILOT_MATCH_HAS_MAX_RESULTS 0x2u
#define MADOPILOT_MATCH_HAS_SUPPRESSION 0x4u

/* The mapped bytes are shared with the frame rather than copied out of it. */
#define MADOPILOT_IMAGE_SHARED 0x1u

/* The target reports a placement, so target and desktop spaces convert. */
#define MADOPILOT_TARGET_SUPPORTS_PLACEMENT 0x1u

/* madopilot_error_detail_t: which optional fields the library populated. */
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
 * `*_release` drops one; both accept null as a no-op, so a cleanup path can
 * release whatever it has without knowing how far construction got. Every other
 * operation rejects null, because "do nothing" is not an answer to a question
 * that has one.
 *
 * A handle passed to a call must stay retained for the whole call. Releasing
 * the last reference concurrently with a call that has not retained one of its
 * own is outside this contract.
 *
 * Const access is safe from several threads at once while each keeps a live
 * reference.
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
 * For an output structure the same size is a promise in the other direction:
 * the library writes only within it, and writes back the number of bytes it
 * actually populated. A caller built against a newer header therefore learns
 * how much of what it knows is really there.
 * ------------------------------------------------------------------------ */

/* A half-open rectangle: [left, right) x [top, bottom). Not versioned. */
typedef struct madopilot_pixel_rect_t {
    madopilot_space_t space;
    int32_t left;
    int32_t top;
    int32_t right;
    int32_t bottom;
} madopilot_pixel_rect_t;

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
    uint32_t flags; /* MADOPILOT_OPERATION_HAS_DEADLINE */
    uint64_t deadline_nanos;
    const madopilot_cancellation_t* cancellation; /* Borrowed, may be null. */
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
    uint32_t flags; /* MADOPILOT_TARGET_SUPPORTS_PLACEMENT */
    uint32_t width;
    uint32_t height;
    madopilot_pixel_format_t format;
    /* A bit set: bit (1 << space) is set when that space converts. */
    int32_t coordinate_spaces;
    madopilot_str_t name;
    madopilot_str_t provider;
} madopilot_target_t;

/* What an open session accepted. Mandatory prefix: the whole structure. */
typedef struct madopilot_session_info_t {
    uint32_t struct_size;
    uint32_t flags; /* No bits defined; the library writes zero. */
    uint64_t stream; /* Same domain as madopilot_frame_stamp_t.stream. */
    uint32_t width;
    uint32_t height;
    madopilot_pixel_format_t format;
    int32_t coordinate_spaces;
} madopilot_session_info_t;

/* How to open a session. Mandatory prefix: through flags.
 *
 * Without either format bit the adapter's own layout is accepted. */
typedef struct madopilot_open_request_t {
    uint32_t struct_size;
    uint32_t flags; /* MADOPILOT_OPEN_HAS_{REQUIRED,PREFERRED}_FORMAT */
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
    madopilot_pixel_rect_t region;
} madopilot_map_request_t;

/* The thresholds one search runs under. Mandatory prefix: through flags.
 *
 * Every omitted field defaults to the prepared template's own declared default.
 * Also used as an output, where the library sets every presence bit because
 * every field was in effect. */
typedef struct madopilot_match_options_t {
    uint32_t struct_size;
    uint32_t flags; /* MADOPILOT_MATCH_HAS_* */
    double min_score;
    uint32_t max_results;
    madopilot_suppression_t suppression;
} madopilot_match_options_t;

/* One template search. Mandatory prefix: through tmpl.
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

/* Where an asset package is read from. Mandatory prefix: through path. */
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
 * An entry that takes `out_error` may be passed null there, and then reports
 * the status only. A returned error is owned by the caller and is released with
 * error_release.
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
    madopilot_status_t (*template_prepare)(
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

    madopilot_status_t (*session_frame)(const madopilot_session_t* session,
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
    /* The options the search actually ran under, not the ones requested. */
    madopilot_status_t (*result_options)(
        const madopilot_result_t* result,
        madopilot_match_options_t* out_options);
    /* An index at or beyond the count is invalid argument and leaves the output
     * in its failure state. */
    madopilot_status_t (*result_match)(const madopilot_result_t* result,
                                       size_t index,
                                       madopilot_match_t* out_match);
} madopilot_api_t;

/* The table's mandatory prefix: everything through status_text.
 *
 * A caller that knows less than this cannot report what it loaded and cannot
 * build a deadline, so negotiation refuses it rather than handing back a table
 * it could not use. */
#define MADOPILOT_API_SIZE_INFORMATION \
    (offsetof(madopilot_api_t, status_text) + sizeof(void*))

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
