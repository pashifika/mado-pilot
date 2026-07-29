//! The size-versioned structures and the enumerated values they carry.
//!
//! # The structure rule
//!
//! Every extensible structure begins with `uint32_t struct_size`, immediately
//! followed by a second 32-bit field so that no implicit padding is introduced
//! between them. That second field is `flags` where the structure has optional
//! or presence-bearing bits and a meaningful discriminant where it has one.
//!
//! A caller sets `struct_size` to `sizeof` the structure as its own header
//! declares it. The library reads only the fields that size covers, applies the
//! documented default to every omitted optional field, and ignores trailing
//! bytes it does not recognize. A size below the documented mandatory prefix is
//! invalid argument, and nothing beyond `struct_size` is read even to check it.
//!
//! A size describes a prefix, so it also has to end where a prefix can end. A
//! `struct_size` that stops inside a field is invalid argument, because that
//! field would be neither covered nor omitted; an element of a caller-declared
//! array whose `struct_size` is above the array's element stride is invalid
//! argument, because the two declarations describe different extents; and a
//! presence bit set for a field the declared size does not reach is invalid
//! argument, because the field the bit names would carry the omitted-field
//! default under the caller's own claim that it was supplied. Every size a
//! released header declares satisfies all three.
//!
//! For an output structure the same size is a promise in the other direction:
//! the library writes only within it, and a caller that supplied an older prefix
//! gets the fields that prefix covers and no others.
//!
//! # Two structures that are not versioned
//!
//! [`madopilot_pixel_rect_t`] and the two views in [`crate::view`] are
//! primitives rather than records. They appear inside other structures, so
//! growing one would move every field after it; a later phase that needs more
//! introduces a different type instead.
//!
//! **Every layout, offset, and numeric value in this module is frozen** for
//! ABI major 1 by `docs/adr/0007-phase-1-c-abi-freeze.md`. The measured totals
//! are in that record and the per-field report is in `docs/evidence/c-abi/`.

use mado_pilot::{
    AssetFaultKind, ClipPolicy, Continuity, CoordinateSpace, LoadStage, PixelFormat, Suppression,
};

use crate::error::Fault;
use crate::operation::madopilot_cancellation_t;
use crate::view::{madopilot_bytes_t, madopilot_str_t};

/// A coordinate space identifier.
pub type madopilot_space_t = i32;

/// Discrete pixels of the captured frame, origin at its top-left.
pub const MADOPILOT_SPACE_CAPTURE_PIXELS: madopilot_space_t = 0;
/// The captured frame's extent normalized to `0.0..=1.0`.
pub const MADOPILOT_SPACE_FRAME_NORMALIZED: madopilot_space_t = 1;
/// The target's logical extent normalized to `0.0..=1.0`.
pub const MADOPILOT_SPACE_TARGET_NORMALIZED: madopilot_space_t = 2;
/// The target's logical points.
pub const MADOPILOT_SPACE_TARGET_LOGICAL: madopilot_space_t = 3;
/// Desktop logical points.
pub const MADOPILOT_SPACE_DESKTOP_LOGICAL: madopilot_space_t = 4;

/// A pixel layout identifier.
pub type madopilot_pixel_format_t = i32;

/// Four bytes per pixel, red, green, blue, alpha.
pub const MADOPILOT_PIXEL_FORMAT_RGBA8: madopilot_pixel_format_t = 0;
/// Four bytes per pixel, blue, green, red, alpha.
pub const MADOPILOT_PIXEL_FORMAT_BGRA8: madopilot_pixel_format_t = 1;

/// What to do with a region that leaves the frame.
pub type madopilot_clip_policy_t = i32;

/// Fail when any part of the region falls outside. The default.
pub const MADOPILOT_CLIP_POLICY_REJECT: madopilot_clip_policy_t = 0;
/// Keep the overlapping part, failing only when nothing overlaps.
pub const MADOPILOT_CLIP_POLICY_CLIP: madopilot_clip_policy_t = 1;

/// Whether a replay frame continues the previous one.
pub type madopilot_continuity_t = i32;

/// The frame continues the previous one and may be compared with it.
pub const MADOPILOT_CONTINUITY_CONTINUOUS: madopilot_continuity_t = 0;
/// The frame begins a new epoch and must not be compared with the previous one.
pub const MADOPILOT_CONTINUITY_DISCONTINUOUS: madopilot_continuity_t = 1;

/// How overlapping candidates are reduced.
pub type madopilot_suppression_t = i32;

/// Drop a candidate that overlaps a canonically earlier surviving one. The
/// default.
pub const MADOPILOT_SUPPRESSION_DROP_OVERLAPPING: madopilot_suppression_t = 0;
/// Report every candidate that passed the threshold.
pub const MADOPILOT_SUPPRESSION_KEEP_ALL: madopilot_suppression_t = 1;

/// Which deterministic source an engine captures from.
pub type madopilot_source_kind_t = i32;

/// Replay frames supplied as raw pixels in caller memory.
pub const MADOPILOT_SOURCE_REPLAY_MEMORY: madopilot_source_kind_t = 0;
/// A replay directory containing a manifest and its raw frame files.
pub const MADOPILOT_SOURCE_REPLAY_DIRECTORY: madopilot_source_kind_t = 1;

/// Where an asset package is read from.
pub type madopilot_package_source_kind_t = i32;

/// A directory laid out as a package.
pub const MADOPILOT_PACKAGE_SOURCE_DIRECTORY: madopilot_package_source_kind_t = 0;
/// An archive file on disk.
pub const MADOPILOT_PACKAGE_SOURCE_ARCHIVE_FILE: madopilot_package_source_kind_t = 1;
/// An archive already in caller memory.
pub const MADOPILOT_PACKAGE_SOURCE_ARCHIVE_BYTES: madopilot_package_source_kind_t = 2;

/// Which rule an asset package broke.
///
/// This is the detail that a single status cannot carry. Package loading is the
/// one Phase 1 operation whose failures a caller may reasonably want to tell
/// apart by more than their category — a bad content hash and an unsafe entry
/// path are both `MADOPILOT_STATUS_ASSET_INVALID` and are not the same problem.
pub type madopilot_asset_fault_t = i32;

/// This build has no name for the rule that was broken.
pub const MADOPILOT_ASSET_FAULT_UNKNOWN: madopilot_asset_fault_t = 0;
/// A caller asked for a limit above the implementation ceiling.
pub const MADOPILOT_ASSET_FAULT_LIMIT_ABOVE_CEILING: madopilot_asset_fault_t = 1;
/// The source could not be measured, opened, or read.
pub const MADOPILOT_ASSET_FAULT_SOURCE_UNREADABLE: madopilot_asset_fault_t = 2;
/// The source changed while it was being read.
pub const MADOPILOT_ASSET_FAULT_SOURCE_CHANGED: madopilot_asset_fault_t = 3;
/// The archive's structure is malformed or self-inconsistent.
pub const MADOPILOT_ASSET_FAULT_MALFORMED_ARCHIVE: madopilot_asset_fault_t = 4;
/// An entry uses a compression method the contract does not accept.
pub const MADOPILOT_ASSET_FAULT_UNSUPPORTED_COMPRESSION_METHOD: madopilot_asset_fault_t = 5;
/// An entry is encrypted.
pub const MADOPILOT_ASSET_FAULT_ENCRYPTED_ENTRY: madopilot_asset_fault_t = 6;
/// A count, byte total, or expansion ratio would exceed its limit.
pub const MADOPILOT_ASSET_FAULT_ARCHIVE_LIMIT: madopilot_asset_fault_t = 7;
/// An entry name is absolute, rooted, traversing, or otherwise unsafe.
pub const MADOPILOT_ASSET_FAULT_UNSAFE_PATH: madopilot_asset_fault_t = 8;
/// Two entries normalize to the same package path.
pub const MADOPILOT_ASSET_FAULT_DUPLICATE_PATH: madopilot_asset_fault_t = 9;
/// An entry is a directory, link, device, or other non-regular type.
pub const MADOPILOT_ASSET_FAULT_UNSUPPORTED_ENTRY_TYPE: madopilot_asset_fault_t = 10;
/// An entry produced a different number of bytes than it declared.
pub const MADOPILOT_ASSET_FAULT_DECLARED_SIZE_MISMATCH: madopilot_asset_fault_t = 11;
/// The package contains no manifest.
pub const MADOPILOT_ASSET_FAULT_MISSING_MANIFEST: madopilot_asset_fault_t = 12;
/// The manifest is not strict UTF-8 JSON matching the typed schema.
pub const MADOPILOT_ASSET_FAULT_MALFORMED_MANIFEST: madopilot_asset_fault_t = 13;
/// The manifest omits its required schema version.
pub const MADOPILOT_ASSET_FAULT_MISSING_SCHEMA_VERSION: madopilot_asset_fault_t = 14;
/// The manifest declares a schema version this build does not implement.
pub const MADOPILOT_ASSET_FAULT_UNSUPPORTED_SCHEMA_VERSION: madopilot_asset_fault_t = 15;
/// Two manifest entries claim the same identity.
pub const MADOPILOT_ASSET_FAULT_DUPLICATE_IDENTITY: madopilot_asset_fault_t = 16;
/// The manifest references an entry the source does not contain.
pub const MADOPILOT_ASSET_FAULT_MISSING_ENTRY: madopilot_asset_fault_t = 17;
/// A committed package was asked for a template it does not contain.
pub const MADOPILOT_ASSET_FAULT_UNKNOWN_TEMPLATE: madopilot_asset_fault_t = 18;
/// The manifest requires content this loader will not fetch.
pub const MADOPILOT_ASSET_FAULT_UNSUPPORTED_SOURCE: madopilot_asset_fault_t = 19;
/// A template's declared extent, coordinates, or defaults are not acceptable.
pub const MADOPILOT_ASSET_FAULT_INVALID_TEMPLATE_METADATA: madopilot_asset_fault_t = 20;
/// A template declares its geometry in an unsupported coordinate space.
pub const MADOPILOT_ASSET_FAULT_UNSUPPORTED_TEMPLATE_SPACE: madopilot_asset_fault_t = 21;
/// The manifest declares a hash algorithm this build does not implement.
pub const MADOPILOT_ASSET_FAULT_UNSUPPORTED_HASH_ALGORITHM: madopilot_asset_fault_t = 22;
/// A declared hash value is not a well-formed digest.
pub const MADOPILOT_ASSET_FAULT_MALFORMED_HASH: madopilot_asset_fault_t = 23;
/// An entry's computed hash differs from the one the manifest declared.
pub const MADOPILOT_ASSET_FAULT_HASH_MISMATCH: madopilot_asset_fault_t = 24;
/// A template's content bytes are not an encoding this build accepts.
pub const MADOPILOT_ASSET_FAULT_UNSUPPORTED_CONTENT_ENCODING: madopilot_asset_fault_t = 25;
/// A size computation would have overflowed.
pub const MADOPILOT_ASSET_FAULT_ARITHMETIC_OVERFLOW: madopilot_asset_fault_t = 26;
/// The cancellation token was set before the package committed.
pub const MADOPILOT_ASSET_FAULT_CANCELLED: madopilot_asset_fault_t = 27;
/// The deadline passed before the package committed.
pub const MADOPILOT_ASSET_FAULT_DEADLINE_EXCEEDED: madopilot_asset_fault_t = 28;

/// How far package loading had got when it refused.
pub type madopilot_asset_stage_t = i32;

/// This build has no name for the stage that refused.
pub const MADOPILOT_ASSET_STAGE_UNKNOWN: madopilot_asset_stage_t = 0;
/// Limits were rejected before any source was touched.
pub const MADOPILOT_ASSET_STAGE_CONFIGURATION: madopilot_asset_stage_t = 1;
/// The source was measured, opened, and enumerated.
pub const MADOPILOT_ASSET_STAGE_SOURCE: madopilot_asset_stage_t = 2;
/// An archive's recorded entry count was read from its trailer.
pub const MADOPILOT_ASSET_STAGE_DIRECTORY_PRE_PARSE: madopilot_asset_stage_t = 3;
/// An archive's central directory was materialized.
pub const MADOPILOT_ASSET_STAGE_DIRECTORY_OPEN: madopilot_asset_stage_t = 4;
/// Entry names, types, and declared sizes were checked.
pub const MADOPILOT_ASSET_STAGE_ENTRY_METADATA: madopilot_asset_stage_t = 5;
/// The manifest was read under its byte cap and parsed.
pub const MADOPILOT_ASSET_STAGE_MANIFEST: madopilot_asset_stage_t = 6;
/// Referenced entries were streamed, size-checked, and hashed.
pub const MADOPILOT_ASSET_STAGE_EXPANSION: madopilot_asset_stage_t = 7;
/// Every check had passed and the package was being committed.
pub const MADOPILOT_ASSET_STAGE_COMMIT: madopilot_asset_stage_t = 8;

/// `madopilot_operation_t.deadline_nanos` carries an absolute deadline.
///
/// Without it the operation has no deadline, which is not the same as a very
/// large one: zero nanoseconds is the domain origin and a valid instant.
pub const MADOPILOT_OPERATION_HAS_DEADLINE: u32 = 1 << 0;

/// `madopilot_open_request_t.required_format` is set.
pub const MADOPILOT_OPEN_HAS_REQUIRED_FORMAT: u32 = 1 << 0;
/// `madopilot_open_request_t.preferred_format` is set.
pub const MADOPILOT_OPEN_HAS_PREFERRED_FORMAT: u32 = 1 << 1;

/// `madopilot_map_request_t.region` is set; the whole frame is mapped without it.
pub const MADOPILOT_MAP_HAS_REGION: u32 = 1 << 0;

/// `madopilot_find_request_t.region` is set; the whole frame is searched without it.
pub const MADOPILOT_FIND_HAS_REGION: u32 = 1 << 0;

/// `madopilot_match_options_t.min_score` is set.
pub const MADOPILOT_MATCH_HAS_MIN_SCORE: u32 = 1 << 0;
/// `madopilot_match_options_t.max_results` is set.
pub const MADOPILOT_MATCH_HAS_MAX_RESULTS: u32 = 1 << 1;
/// `madopilot_match_options_t.suppression` is set.
pub const MADOPILOT_MATCH_HAS_SUPPRESSION: u32 = 1 << 2;

/// The mapped bytes are shared with the frame rather than copied out of it.
pub const MADOPILOT_IMAGE_SHARED: u32 = 1 << 0;

/// The target reports a placement, so target and desktop spaces are convertible.
pub const MADOPILOT_TARGET_SUPPORTS_PLACEMENT: u32 = 1 << 0;

/// `madopilot_error_detail_t` carries `asset_fault` and `asset_stage`.
pub const MADOPILOT_ERROR_HAS_ASSET_DETAIL: u32 = 1 << 0;
/// `madopilot_error_detail_t.backend` names the backend that failed.
pub const MADOPILOT_ERROR_HAS_BACKEND: u32 = 1 << 1;

/// A half-open rectangle in a named coordinate space.
///
/// `[left, right) x [top, bottom)`. Not size-versioned: it is a primitive that
/// other structures embed by value.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct madopilot_pixel_rect_t {
    /// The coordinate space the four edges are measured in.
    pub space: madopilot_space_t,
    /// The inclusive left edge.
    pub left: i32,
    /// The inclusive top edge.
    pub top: i32,
    /// The exclusive right edge.
    pub right: i32,
    /// The exclusive bottom edge.
    pub bottom: i32,
}

impl madopilot_pixel_rect_t {
    /// The failure state: an empty rectangle in capture pixels.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            space: MADOPILOT_SPACE_CAPTURE_PIXELS,
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        }
    }
}

/// What the loaded library is, and what it negotiated.
///
/// Mandatory prefix: through `table_size`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_build_info_t {
    /// `sizeof(madopilot_build_info_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// No bits are defined; the library writes zero.
    pub flags: u32,
    /// The ABI major version this library implements.
    pub abi_major: u32,
    /// The ABI minor version this library implements.
    pub abi_minor: u32,
    /// `sizeof` the library's own function table.
    pub table_size: u32,
    /// Padding to the natural alignment of the views below. Written as zero.
    pub reserved: u32,
    /// The library's package version. Borrowed from static storage.
    pub library_version: madopilot_str_t,
    /// The matching backend this build requires. Borrowed from static storage.
    pub required_backend: madopilot_str_t,
}

/// A deadline and a cancellation token, supplied by the caller.
///
/// Mandatory prefix: through `flags`. Omitting the rest means no deadline and
/// no cancellation.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_operation_t {
    /// `sizeof(madopilot_operation_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// [`MADOPILOT_OPERATION_HAS_DEADLINE`].
    pub flags: u32,
    /// An absolute deadline, in nanoseconds since the library clock's origin.
    pub deadline_nanos: u64,
    /// A borrowed cancellation handle, or null. The caller keeps it retained.
    pub cancellation: *const madopilot_cancellation_t,
}

/// The complete public identity of one published frame.
///
/// Mandatory prefix: the whole structure. Identity is not optional; a caller
/// that cannot store all four fields cannot correlate a result at all.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct madopilot_frame_stamp_t {
    /// `sizeof(madopilot_frame_stamp_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// No bits are defined; the library writes zero.
    pub flags: u32,
    /// The stream that published the frame. Never reused while the library is
    /// loaded, and minted by this boundary rather than read from the Rust
    /// identity, which has no fixed-width projection.
    pub stream: u64,
    /// The stream continuity generation.
    pub epoch: u64,
    /// The frame's position within that epoch.
    pub sequence: u64,
    /// The geometry generation that was authoritative for the frame.
    pub geometry: u64,
}

impl madopilot_frame_stamp_t {
    /// The failure state.
    #[must_use]
    pub const fn cleared(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            stream: 0,
            epoch: 0,
            sequence: 0,
            geometry: 0,
        }
    }
}

/// A frame's pixel geometry.
///
/// Mandatory prefix: through `space`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_frame_info_t {
    /// `sizeof(madopilot_frame_info_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// No bits are defined; the library writes zero.
    pub flags: u32,
    /// The frame width in pixels.
    pub width: u32,
    /// The frame height in pixels.
    pub height: u32,
    /// The frame's pixel layout.
    pub format: madopilot_pixel_format_t,
    /// The coordinate space of `bounds`.
    pub space: madopilot_space_t,
    /// The distance between the starts of two rows, in bytes.
    pub stride: u64,
    /// The frame's own bounds.
    pub bounds: madopilot_pixel_rect_t,
}

/// A completed CPU mapping's descriptor and its borrowed bytes.
///
/// Mandatory prefix: through `bytes`. `bytes` remains readable while the mapping
/// handle that produced this descriptor is retained, and becomes invalid at that
/// handle's final release.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_image_t {
    /// `sizeof(madopilot_image_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// [`MADOPILOT_IMAGE_SHARED`].
    pub flags: u32,
    /// The mapped width in pixels.
    pub width: u32,
    /// The mapped height in pixels.
    pub height: u32,
    /// The mapped pixel layout.
    pub format: madopilot_pixel_format_t,
    /// The coordinate space of `region`.
    pub space: madopilot_space_t,
    /// The distance between the starts of two rows, in bytes.
    pub stride: u64,
    /// The mapped bytes, borrowed from the mapping handle.
    pub bytes: madopilot_bytes_t,
    /// The region of the source frame this mapping covers.
    pub region: madopilot_pixel_rect_t,
}

/// One discovered capture target.
///
/// Mandatory prefix: through `coordinate_spaces`. `name` and `provider` are
/// borrowed from the target-list handle and become invalid at its final release.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_target_t {
    /// `sizeof(madopilot_target_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// [`MADOPILOT_TARGET_SUPPORTS_PLACEMENT`].
    pub flags: u32,
    /// The target width in pixels.
    pub width: u32,
    /// The target height in pixels.
    pub height: u32,
    /// The pixel layout the target publishes.
    pub format: madopilot_pixel_format_t,
    /// A bit set: bit `1 << space` is set when that coordinate space converts.
    pub coordinate_spaces: i32,
    /// The target's title.
    pub name: madopilot_str_t,
    /// The provider that discovered it.
    pub provider: madopilot_str_t,
}

/// What an open session accepted.
///
/// Mandatory prefix: the whole structure.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_session_info_t {
    /// `sizeof(madopilot_session_info_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// No bits are defined; the library writes zero.
    pub flags: u32,
    /// The stream this session publishes to, in the same domain as
    /// [`madopilot_frame_stamp_t::stream`].
    pub stream: u64,
    /// The accepted width in pixels.
    pub width: u32,
    /// The accepted height in pixels.
    pub height: u32,
    /// The accepted pixel layout.
    pub format: madopilot_pixel_format_t,
    /// A bit set: bit `1 << space` is set when that coordinate space converts.
    pub coordinate_spaces: i32,
}

/// How a session should be opened.
///
/// Mandatory prefix: through `flags`. Without either format bit the adapter's
/// own layout is accepted.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_open_request_t {
    /// `sizeof(madopilot_open_request_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// [`MADOPILOT_OPEN_HAS_REQUIRED_FORMAT`], [`MADOPILOT_OPEN_HAS_PREFERRED_FORMAT`].
    pub flags: u32,
    /// A layout the session must publish, or the open fails.
    pub required_format: madopilot_pixel_format_t,
    /// A layout the session should publish if it can.
    pub preferred_format: madopilot_pixel_format_t,
}

/// How a frame should be mapped to CPU-readable bytes.
///
/// Mandatory prefix: through `format`. Omitting `clip_policy` rejects a region
/// that leaves the frame; omitting the region maps the whole frame.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_map_request_t {
    /// `sizeof(madopilot_map_request_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// [`MADOPILOT_MAP_HAS_REGION`].
    pub flags: u32,
    /// The pixel layout the mapped bytes must be in.
    pub format: madopilot_pixel_format_t,
    /// What to do with a region that leaves the frame.
    pub clip_policy: madopilot_clip_policy_t,
    /// The region to map, when [`MADOPILOT_MAP_HAS_REGION`] is set.
    pub region: madopilot_pixel_rect_t,
}

/// The thresholds and limits one search runs under.
///
/// Mandatory prefix: through `flags`. Every omitted field defaults to the
/// prepared template's own declared default.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_match_options_t {
    /// `sizeof(madopilot_match_options_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// [`MADOPILOT_MATCH_HAS_MIN_SCORE`], [`MADOPILOT_MATCH_HAS_MAX_RESULTS`],
    /// [`MADOPILOT_MATCH_HAS_SUPPRESSION`].
    pub flags: u32,
    /// The lowest score that qualifies as a match.
    pub min_score: f64,
    /// The most matches to report.
    pub max_results: u32,
    /// How overlapping candidates are reduced.
    pub suppression: madopilot_suppression_t,
}

impl madopilot_match_options_t {
    /// The failure state, and the "everything defaulted" input.
    #[must_use]
    pub const fn cleared(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            min_score: 0.0,
            max_results: 0,
            suppression: MADOPILOT_SUPPRESSION_DROP_OVERLAPPING,
        }
    }
}

/// One template search against one session.
///
/// Mandatory prefix: through `tmpl`. The field is not called `template` because
/// the C++ wrapper includes this header and that word is a keyword there.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_find_request_t {
    /// `sizeof(madopilot_find_request_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// [`MADOPILOT_FIND_HAS_REGION`].
    pub flags: u32,
    /// The exact frame to search, or null for the session's latest frame.
    pub frame: *const crate::capture::madopilot_frame_t,
    /// The prepared template to search for. Required.
    pub tmpl: *const crate::assets::madopilot_template_t,
    /// The options to search under, or null for the template's own defaults.
    pub options: *const madopilot_match_options_t,
    /// The region to search, when [`MADOPILOT_FIND_HAS_REGION`] is set.
    pub region: madopilot_pixel_rect_t,
    /// What to do with a region that leaves the frame.
    pub clip_policy: madopilot_clip_policy_t,
}

/// One match within a result.
///
/// Mandatory prefix: through `bounds`. `template_id` is borrowed from the result
/// handle and becomes invalid at its final release.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_match_t {
    /// `sizeof(madopilot_match_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// No bits are defined; the library writes zero.
    pub flags: u32,
    /// The match score, in `0.0..=1.0`.
    pub score: f64,
    /// The identity of the template that matched.
    pub template_id: madopilot_str_t,
    /// Where it matched.
    pub bounds: madopilot_pixel_rect_t,
}

/// What one completed search produced.
///
/// Mandatory prefix: through `searched`. Both backend views are borrowed from
/// the result handle.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_result_info_t {
    /// `sizeof(madopilot_result_info_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// No bits are defined; the library writes zero.
    pub flags: u32,
    /// How many matches the result contains. Zero is a successful answer.
    pub match_count: u64,
    /// The backend that actually produced the result.
    pub backend_id: madopilot_str_t,
    /// That backend's version.
    pub backend_version: madopilot_str_t,
    /// The region of the source frame that was searched.
    pub searched: madopilot_pixel_rect_t,
}

/// What a loaded asset package declares about itself.
///
/// Mandatory prefix: the whole structure. Every view is borrowed from the
/// package handle.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_package_info_t {
    /// `sizeof(madopilot_package_info_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// No bits are defined; the library writes zero.
    pub flags: u32,
    /// How many templates the package declares.
    pub template_count: u64,
    /// The package identity.
    pub package_id: madopilot_str_t,
    /// The package version.
    pub package_version: madopilot_str_t,
    /// The license the package content is offered under.
    pub license: madopilot_str_t,
}

/// What a prepared template is.
///
/// Mandatory prefix: the whole structure. Both views are borrowed from the
/// template handle.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_template_info_t {
    /// `sizeof(madopilot_template_info_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// No bits are defined; the library writes zero.
    pub flags: u32,
    /// The template width in pixels.
    pub width: u32,
    /// The template height in pixels.
    pub height: u32,
    /// The template's own default lowest qualifying score.
    pub min_score: f64,
    /// The template identity.
    pub id: madopilot_str_t,
    /// The backend the template was compiled for.
    pub backend: madopilot_str_t,
    /// The template's own default result limit.
    pub max_results: u32,
    /// The coordinate space the template's geometry is declared in.
    pub space: madopilot_space_t,
}

/// A failure, in structured form.
///
/// Mandatory prefix: through `category`. `message` and `backend` are borrowed
/// from the error handle and become invalid at its final release.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_error_detail_t {
    /// `sizeof(madopilot_error_detail_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// [`MADOPILOT_ERROR_HAS_ASSET_DETAIL`], [`MADOPILOT_ERROR_HAS_BACKEND`].
    pub flags: u32,
    /// The same status the failing call returned.
    pub status: crate::status::madopilot_status_t,
    /// The subsystem the failure came from.
    pub category: crate::status::madopilot_error_category_t,
    /// Which rule an asset package broke, when the flag is set.
    pub asset_fault: madopilot_asset_fault_t,
    /// How far package loading had got, when the flag is set.
    pub asset_stage: madopilot_asset_stage_t,
    /// A redacted diagnostic message. Never required for control flow, and
    /// never contains captured pixels or recognized text.
    pub message: madopilot_str_t,
    /// The backend that failed, when the flag is set.
    pub backend: madopilot_str_t,
}

/// One replay frame supplied as raw pixels in caller memory.
///
/// Mandatory prefix: through `pixels`. Omitting `captured_at_nanos` places the
/// frame at the clock domain's origin, and omitting `stride` means packed rows.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_replay_frame_t {
    /// `sizeof(madopilot_replay_frame_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// No bits are defined; the caller sets zero.
    pub flags: u32,
    /// The frame width in pixels.
    pub width: u32,
    /// The frame height in pixels.
    pub height: u32,
    /// The pixel layout of `pixels`.
    pub format: madopilot_pixel_format_t,
    /// Whether this frame continues the previous one.
    pub continuity: madopilot_continuity_t,
    /// The pixels. Copied during engine construction; the caller may release
    /// its own storage as soon as the call returns.
    pub pixels: madopilot_bytes_t,
    /// When the frame was captured, in nanoseconds since the clock origin.
    pub captured_at_nanos: u64,
    /// The distance between the starts of two rows, or zero for packed rows.
    pub stride: u64,
}

/// Where an engine's frames come from.
///
/// Mandatory prefix: through `frame_stride`. `kind` selects which of the
/// remaining fields are read; the others are ignored entirely.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_source_t {
    /// `sizeof(madopilot_source_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// Which source this is, and therefore which fields below are active.
    pub kind: madopilot_source_kind_t,
    /// The replay directory, for [`MADOPILOT_SOURCE_REPLAY_DIRECTORY`].
    pub directory: madopilot_str_t,
    /// The frames, for [`MADOPILOT_SOURCE_REPLAY_MEMORY`].
    pub frames: *const madopilot_replay_frame_t,
    /// How many frames `frames` points at.
    pub frame_count: usize,
    /// `sizeof(madopilot_replay_frame_t)` as the caller's header declares it,
    /// which is the stride between the elements of `frames`.
    pub frame_stride: usize,
    /// The name to publish the memory target under, or empty for a default.
    pub target_name: madopilot_str_t,
}

/// Where an asset package is read from.
///
/// Mandatory prefix: through `path`. `kind` selects which field is active.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_package_source_t {
    /// `sizeof(madopilot_package_source_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// Which source this is, and therefore which field below is active.
    pub kind: madopilot_package_source_kind_t,
    /// The directory or archive path, for the two path-bearing kinds.
    pub path: madopilot_str_t,
    /// The archive bytes, for [`MADOPILOT_PACKAGE_SOURCE_ARCHIVE_BYTES`].
    pub archive: madopilot_bytes_t,
}

/// Resolves a C pixel-format value.
pub(crate) fn pixel_format(value: madopilot_pixel_format_t) -> Result<PixelFormat, Fault> {
    match value {
        MADOPILOT_PIXEL_FORMAT_RGBA8 => Ok(PixelFormat::Rgba8),
        MADOPILOT_PIXEL_FORMAT_BGRA8 => Ok(PixelFormat::Bgra8),
        other => Err(Fault::abi(format!("unrecognized pixel format {other}"))),
    }
}

/// Projects a pixel format onto its C value.
///
/// [`PixelFormat`] is `#[non_exhaustive]`. A format this build has no number for
/// cannot be reported honestly, so it is refused rather than aliased onto one of
/// the two that exist.
pub(crate) fn pixel_format_code(format: PixelFormat) -> Result<madopilot_pixel_format_t, Fault> {
    match format {
        PixelFormat::Rgba8 => Ok(MADOPILOT_PIXEL_FORMAT_RGBA8),
        PixelFormat::Bgra8 => Ok(MADOPILOT_PIXEL_FORMAT_BGRA8),
        other => Err(Fault::internal(format!(
            "this ABI major has no value for pixel format {other}"
        ))),
    }
}

/// Resolves a C coordinate-space value.
pub(crate) fn space(value: madopilot_space_t) -> Result<CoordinateSpace, Fault> {
    match value {
        MADOPILOT_SPACE_CAPTURE_PIXELS => Ok(CoordinateSpace::CapturePixels),
        MADOPILOT_SPACE_FRAME_NORMALIZED => Ok(CoordinateSpace::FrameNormalized),
        MADOPILOT_SPACE_TARGET_NORMALIZED => Ok(CoordinateSpace::TargetNormalized),
        MADOPILOT_SPACE_TARGET_LOGICAL => Ok(CoordinateSpace::TargetLogical),
        MADOPILOT_SPACE_DESKTOP_LOGICAL => Ok(CoordinateSpace::DesktopLogical),
        other => Err(Fault::abi(format!("unrecognized coordinate space {other}"))),
    }
}

/// Projects a coordinate space onto its C value.
pub(crate) const fn space_code(value: CoordinateSpace) -> madopilot_space_t {
    match value {
        CoordinateSpace::CapturePixels => MADOPILOT_SPACE_CAPTURE_PIXELS,
        CoordinateSpace::FrameNormalized => MADOPILOT_SPACE_FRAME_NORMALIZED,
        CoordinateSpace::TargetNormalized => MADOPILOT_SPACE_TARGET_NORMALIZED,
        CoordinateSpace::TargetLogical => MADOPILOT_SPACE_TARGET_LOGICAL,
        CoordinateSpace::DesktopLogical => MADOPILOT_SPACE_DESKTOP_LOGICAL,
        // `CoordinateSpace` is `#[non_exhaustive]`. A later space has no number
        // yet, and reporting it as capture pixels would be a confident lie about
        // where a rectangle is.
        _ => -1,
    }
}

/// Resolves a C clip-policy value.
pub(crate) fn clip_policy(value: madopilot_clip_policy_t) -> Result<ClipPolicy, Fault> {
    match value {
        MADOPILOT_CLIP_POLICY_REJECT => Ok(ClipPolicy::Reject),
        MADOPILOT_CLIP_POLICY_CLIP => Ok(ClipPolicy::Clip),
        other => Err(Fault::abi(format!("unrecognized clip policy {other}"))),
    }
}

/// Resolves a C continuity value.
pub(crate) fn continuity(value: madopilot_continuity_t) -> Result<Continuity, Fault> {
    match value {
        MADOPILOT_CONTINUITY_CONTINUOUS => Ok(Continuity::Continuous),
        MADOPILOT_CONTINUITY_DISCONTINUOUS => Ok(Continuity::Discontinuous),
        other => Err(Fault::abi(format!("unrecognized continuity {other}"))),
    }
}

/// Resolves a C suppression value.
pub(crate) fn suppression(value: madopilot_suppression_t) -> Result<Suppression, Fault> {
    match value {
        MADOPILOT_SUPPRESSION_DROP_OVERLAPPING => Ok(Suppression::DropOverlapping),
        MADOPILOT_SUPPRESSION_KEEP_ALL => Ok(Suppression::KeepAll),
        other => Err(Fault::abi(format!("unrecognized suppression {other}"))),
    }
}

/// Projects a suppression policy onto its C value.
pub(crate) const fn suppression_code(value: Suppression) -> madopilot_suppression_t {
    match value {
        Suppression::DropOverlapping => MADOPILOT_SUPPRESSION_DROP_OVERLAPPING,
        Suppression::KeepAll => MADOPILOT_SUPPRESSION_KEEP_ALL,
        // `Suppression` is `#[non_exhaustive]`; a later policy has no number.
        _ => -1,
    }
}

/// Projects an asset fault kind onto its C value.
pub(crate) const fn asset_fault_code(kind: AssetFaultKind) -> madopilot_asset_fault_t {
    match kind {
        AssetFaultKind::LimitAboveCeiling => MADOPILOT_ASSET_FAULT_LIMIT_ABOVE_CEILING,
        AssetFaultKind::SourceUnreadable => MADOPILOT_ASSET_FAULT_SOURCE_UNREADABLE,
        AssetFaultKind::SourceChanged => MADOPILOT_ASSET_FAULT_SOURCE_CHANGED,
        AssetFaultKind::MalformedArchive => MADOPILOT_ASSET_FAULT_MALFORMED_ARCHIVE,
        AssetFaultKind::UnsupportedCompressionMethod => {
            MADOPILOT_ASSET_FAULT_UNSUPPORTED_COMPRESSION_METHOD
        }
        AssetFaultKind::EncryptedEntry => MADOPILOT_ASSET_FAULT_ENCRYPTED_ENTRY,
        AssetFaultKind::ArchiveLimit => MADOPILOT_ASSET_FAULT_ARCHIVE_LIMIT,
        AssetFaultKind::UnsafePath => MADOPILOT_ASSET_FAULT_UNSAFE_PATH,
        AssetFaultKind::DuplicatePath => MADOPILOT_ASSET_FAULT_DUPLICATE_PATH,
        AssetFaultKind::UnsupportedEntryType => MADOPILOT_ASSET_FAULT_UNSUPPORTED_ENTRY_TYPE,
        AssetFaultKind::DeclaredSizeMismatch => MADOPILOT_ASSET_FAULT_DECLARED_SIZE_MISMATCH,
        AssetFaultKind::MissingManifest => MADOPILOT_ASSET_FAULT_MISSING_MANIFEST,
        AssetFaultKind::MalformedManifest => MADOPILOT_ASSET_FAULT_MALFORMED_MANIFEST,
        AssetFaultKind::MissingSchemaVersion => MADOPILOT_ASSET_FAULT_MISSING_SCHEMA_VERSION,
        AssetFaultKind::UnsupportedSchemaVersion => {
            MADOPILOT_ASSET_FAULT_UNSUPPORTED_SCHEMA_VERSION
        }
        AssetFaultKind::DuplicateIdentity => MADOPILOT_ASSET_FAULT_DUPLICATE_IDENTITY,
        AssetFaultKind::MissingEntry => MADOPILOT_ASSET_FAULT_MISSING_ENTRY,
        AssetFaultKind::UnknownTemplate => MADOPILOT_ASSET_FAULT_UNKNOWN_TEMPLATE,
        AssetFaultKind::UnsupportedSource => MADOPILOT_ASSET_FAULT_UNSUPPORTED_SOURCE,
        AssetFaultKind::InvalidTemplateMetadata => MADOPILOT_ASSET_FAULT_INVALID_TEMPLATE_METADATA,
        AssetFaultKind::UnsupportedTemplateSpace => {
            MADOPILOT_ASSET_FAULT_UNSUPPORTED_TEMPLATE_SPACE
        }
        AssetFaultKind::UnsupportedHashAlgorithm => {
            MADOPILOT_ASSET_FAULT_UNSUPPORTED_HASH_ALGORITHM
        }
        AssetFaultKind::MalformedHash => MADOPILOT_ASSET_FAULT_MALFORMED_HASH,
        AssetFaultKind::HashMismatch => MADOPILOT_ASSET_FAULT_HASH_MISMATCH,
        AssetFaultKind::UnsupportedContentEncoding => {
            MADOPILOT_ASSET_FAULT_UNSUPPORTED_CONTENT_ENCODING
        }
        AssetFaultKind::ArithmeticOverflow => MADOPILOT_ASSET_FAULT_ARITHMETIC_OVERFLOW,
        AssetFaultKind::Cancelled => MADOPILOT_ASSET_FAULT_CANCELLED,
        AssetFaultKind::DeadlineExceeded => MADOPILOT_ASSET_FAULT_DEADLINE_EXCEEDED,
        // `AssetFaultKind` is `#[non_exhaustive]`; a later rule has no number.
        _ => MADOPILOT_ASSET_FAULT_UNKNOWN,
    }
}

/// Projects a load stage onto its C value.
pub(crate) const fn asset_stage_code(stage: LoadStage) -> madopilot_asset_stage_t {
    match stage {
        LoadStage::Configuration => MADOPILOT_ASSET_STAGE_CONFIGURATION,
        LoadStage::Source => MADOPILOT_ASSET_STAGE_SOURCE,
        LoadStage::DirectoryPreParse => MADOPILOT_ASSET_STAGE_DIRECTORY_PRE_PARSE,
        LoadStage::DirectoryOpen => MADOPILOT_ASSET_STAGE_DIRECTORY_OPEN,
        LoadStage::EntryMetadata => MADOPILOT_ASSET_STAGE_ENTRY_METADATA,
        LoadStage::Manifest => MADOPILOT_ASSET_STAGE_MANIFEST,
        LoadStage::Expansion => MADOPILOT_ASSET_STAGE_EXPANSION,
        LoadStage::Commit => MADOPILOT_ASSET_STAGE_COMMIT,
        // `LoadStage` is `#[non_exhaustive]`; a later stage has no number.
        _ => MADOPILOT_ASSET_STAGE_UNKNOWN,
    }
}
