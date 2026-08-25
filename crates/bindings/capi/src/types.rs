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
//! `struct_size` that stops inside a field of the structure this library
//! declares is invalid argument, because that field would be neither covered
//! nor omitted — a size at or above this library's own is a newer caller, whose
//! extra bytes are the trailing bytes above rather than a field this library
//! could place; an element of a caller-declared array whose `struct_size` is
//! above the array's element stride is invalid argument, because the two
//! declarations describe different extents; and a presence bit set for a field
//! the declared size does not reach is invalid argument, because the field the
//! bit names would carry the omitted-field default under the caller's own claim
//! that it was supplied. Every size a released header declares satisfies all
//! three.
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

use crate::capture::madopilot_frame_t;
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
/// Native Windows discovery, capture, and input through the facade.
pub const MADOPILOT_SOURCE_NATIVE_WINDOWS: madopilot_source_kind_t = 2;
/// Native macOS discovery, capture, permission, and input through the facade.
pub const MADOPILOT_SOURCE_NATIVE_MACOS: madopilot_source_kind_t = 3;

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
/// This is detail that a single status cannot carry — a bad content hash and an
/// unsafe entry path are both `MADOPILOT_STATUS_ASSET_INVALID` and are not the
/// same problem.
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
/// A committed package was asked for an OCR model it does not contain.
pub const MADOPILOT_ASSET_FAULT_UNKNOWN_OCR_MODEL: madopilot_asset_fault_t = 29;
/// An OCR model/profile declaration is incomplete, unbounded, or inconsistent.
pub const MADOPILOT_ASSET_FAULT_INVALID_OCR_MODEL_METADATA: madopilot_asset_fault_t = 30;
/// The package names OCR normalization semantics this build does not support.
pub const MADOPILOT_ASSET_FAULT_UNSUPPORTED_OCR_PROFILE: madopilot_asset_fault_t = 31;

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
/// A redacted-diagnostic category.
pub type madopilot_diagnostic_category_t = i32;

/// No diagnostic category was reported.
pub const MADOPILOT_DIAGNOSTIC_UNSPECIFIED: madopilot_diagnostic_category_t = 0;
/// Authorization was denied.
pub const MADOPILOT_DIAGNOSTIC_PERMISSION_DENIED: madopilot_diagnostic_category_t = 1;
/// Authorization could not be established without prompting.
pub const MADOPILOT_DIAGNOSTIC_PERMISSION_UNDETERMINED: madopilot_diagnostic_category_t = 2;
/// The capability is unavailable.
pub const MADOPILOT_DIAGNOSTIC_CAPABILITY_UNAVAILABLE: madopilot_diagnostic_category_t = 3;
/// The target no longer exists.
pub const MADOPILOT_DIAGNOSTIC_TARGET_LOST: madopilot_diagnostic_category_t = 4;
/// The platform reported another failure.
pub const MADOPILOT_DIAGNOSTIC_PLATFORM_FAILURE: madopilot_diagnostic_category_t = 5;
/// The request or Adapter configuration was inconsistent.
pub const MADOPILOT_DIAGNOSTIC_CONFIGURATION: madopilot_diagnostic_category_t = 6;

/// A sensitive capability whose authorization can be probed.
pub type madopilot_permission_kind_t = i32;

/// No permission kind was reported.
pub const MADOPILOT_PERMISSION_KIND_UNSPECIFIED: madopilot_permission_kind_t = 0;
/// Permission to read window and display contents.
pub const MADOPILOT_PERMISSION_KIND_SCREEN_CAPTURE: madopilot_permission_kind_t = 1;
/// Permission to deliver pointer, keyboard, and text input.
pub const MADOPILOT_PERMISSION_KIND_INPUT_CONTROL: madopilot_permission_kind_t = 2;

/// The result of a non-prompting permission probe.
pub type madopilot_permission_state_t = i32;

/// The probe could not establish a state without prompting.
pub const MADOPILOT_PERMISSION_STATE_UNKNOWN: madopilot_permission_state_t = 0;
/// The operating system authorizes the capability.
pub const MADOPILOT_PERMISSION_STATE_GRANTED: madopilot_permission_state_t = 1;
/// The operating system withholds the capability.
pub const MADOPILOT_PERMISSION_STATE_NOT_GRANTED: madopilot_permission_state_t = 2;
/// The platform or build has no corresponding global authorization.
pub const MADOPILOT_PERMISSION_STATE_UNAVAILABLE: madopilot_permission_state_t = 3;

/// The kind of desktop object a discovered target represents.
pub type madopilot_target_kind_t = i32;

/// The provider did not classify the target.
pub const MADOPILOT_TARGET_KIND_UNKNOWN: madopilot_target_kind_t = 0;
/// One application window.
pub const MADOPILOT_TARGET_KIND_WINDOW: madopilot_target_kind_t = 1;
/// One display.
pub const MADOPILOT_TARGET_KIND_DISPLAY: madopilot_target_kind_t = 2;

/// Whether a provider can attempt one operation.
pub type madopilot_capability_support_t = i32;

/// Support cannot be established without attempting the operation.
pub const MADOPILOT_CAPABILITY_UNKNOWN: madopilot_capability_support_t = 0;
/// The provider can attempt the operation.
pub const MADOPILOT_CAPABILITY_SUPPORTED: madopilot_capability_support_t = 1;
/// The provider cannot perform the operation.
pub const MADOPILOT_CAPABILITY_UNSUPPORTED: madopilot_capability_support_t = 2;

/// What an input event does, independently of delivery.
pub type madopilot_input_operation_kind_t = i32;

/// No input operation kind.
pub const MADOPILOT_INPUT_OPERATION_UNKNOWN: madopilot_input_operation_kind_t = 0;
/// Pointer movement, button, or scroll input.
pub const MADOPILOT_INPUT_OPERATION_POINTER: madopilot_input_operation_kind_t = 1;
/// Key press or release input.
pub const MADOPILOT_INPUT_OPERATION_KEYBOARD: madopilot_input_operation_kind_t = 2;
/// Text input.
pub const MADOPILOT_INPUT_OPERATION_TEXT: madopilot_input_operation_kind_t = 3;

/// How an input event is submitted.
pub type madopilot_input_delivery_t = i32;

/// No route was selected.
pub const MADOPILOT_INPUT_DELIVERY_NONE: madopilot_input_delivery_t = 0;
/// The operating system's focused system-input path.
pub const MADOPILOT_INPUT_DELIVERY_SYSTEM: madopilot_input_delivery_t = 1;
/// A message addressed to one exact selected window.
pub const MADOPILOT_INPUT_DELIVERY_WINDOW_MESSAGE: madopilot_input_delivery_t = 2;
/// An event addressed to the process that owns the selected target.
pub const MADOPILOT_INPUT_DELIVERY_PROCESS_DIRECTED: madopilot_input_delivery_t = 3;
/// Whether session open can proceed without requested input.
pub type madopilot_input_requirement_t = i32;

/// Open capture-only when requested input cannot be established.
pub const MADOPILOT_INPUT_OPTIONAL: madopilot_input_requirement_t = 0;
/// Fail open when requested input cannot be established.
pub const MADOPILOT_INPUT_REQUIRED: madopilot_input_requirement_t = 1;

/// What input delivery may do about focus.
pub type madopilot_focus_policy_t = i32;

/// Never change focus.
pub const MADOPILOT_FOCUS_PRESERVE: madopilot_focus_policy_t = 0;
/// Require the target already to be focused.
pub const MADOPILOT_FOCUS_REQUIRE_FOCUSED: madopilot_focus_policy_t = 1;
/// Activate the target only when the selected mechanism requires it.
pub const MADOPILOT_FOCUS_ACTIVATE_IF_REQUIRED: madopilot_focus_policy_t = 2;

/// How pointer coordinates resolve at delivery time.
pub type madopilot_geometry_policy_t = i32;

/// Resolve against authoritative current geometry.
pub const MADOPILOT_GEOMETRY_REPROJECT_CURRENT: madopilot_geometry_policy_t = 0;
/// Refuse when geometry changed since the source frame.
pub const MADOPILOT_GEOMETRY_REQUIRE_UNCHANGED: madopilot_geometry_policy_t = 1;
/// Resolve against the source frame's retained transform.
pub const MADOPILOT_GEOMETRY_USE_FRAME_SNAPSHOT: madopilot_geometry_policy_t = 2;

/// A pointer button.
pub type madopilot_pointer_button_t = i32;

/// No pointer button.
pub const MADOPILOT_POINTER_BUTTON_UNKNOWN: madopilot_pointer_button_t = 0;
/// The user's primary button.
pub const MADOPILOT_POINTER_BUTTON_PRIMARY: madopilot_pointer_button_t = 1;
/// The user's secondary button.
pub const MADOPILOT_POINTER_BUTTON_SECONDARY: madopilot_pointer_button_t = 2;
/// The middle button or wheel click.
pub const MADOPILOT_POINTER_BUTTON_MIDDLE: madopilot_pointer_button_t = 3;

/// A keyboard modifier.
pub type madopilot_modifier_t = i32;

/// No modifier.
pub const MADOPILOT_MODIFIER_UNKNOWN: madopilot_modifier_t = 0;
/// Shift.
pub const MADOPILOT_MODIFIER_SHIFT: madopilot_modifier_t = 1;
/// Control.
pub const MADOPILOT_MODIFIER_CONTROL: madopilot_modifier_t = 2;
/// Alt or Option.
pub const MADOPILOT_MODIFIER_ALT: madopilot_modifier_t = 3;
/// Command or Windows.
pub const MADOPILOT_MODIFIER_META: madopilot_modifier_t = 4;

/// A logical key kind. Character, function, and modifier keys use
/// `madopilot_input_event_t.key_value`.
pub type madopilot_key_t = i32;

/// No key.
pub const MADOPILOT_KEY_UNKNOWN: madopilot_key_t = 0;
/// One Unicode scalar in `key_value`.
pub const MADOPILOT_KEY_CHARACTER: madopilot_key_t = 1;
/// Function key 1 through 24 in `key_value`.
pub const MADOPILOT_KEY_FUNCTION: madopilot_key_t = 2;
/// A [`madopilot_modifier_t`] in `key_value`.
pub const MADOPILOT_KEY_MODIFIER: madopilot_key_t = 3;
/// Return or Enter.
pub const MADOPILOT_KEY_ENTER: madopilot_key_t = 4;
/// Tab.
pub const MADOPILOT_KEY_TAB: madopilot_key_t = 5;
/// Backspace.
pub const MADOPILOT_KEY_BACKSPACE: madopilot_key_t = 6;
/// Forward delete.
pub const MADOPILOT_KEY_DELETE: madopilot_key_t = 7;
/// Escape.
pub const MADOPILOT_KEY_ESCAPE: madopilot_key_t = 8;
/// Space.
pub const MADOPILOT_KEY_SPACE: madopilot_key_t = 9;
/// Arrow up.
pub const MADOPILOT_KEY_ARROW_UP: madopilot_key_t = 10;
/// Arrow down.
pub const MADOPILOT_KEY_ARROW_DOWN: madopilot_key_t = 11;
/// Arrow left.
pub const MADOPILOT_KEY_ARROW_LEFT: madopilot_key_t = 12;
/// Arrow right.
pub const MADOPILOT_KEY_ARROW_RIGHT: madopilot_key_t = 13;
/// Home.
pub const MADOPILOT_KEY_HOME: madopilot_key_t = 14;
/// End.
pub const MADOPILOT_KEY_END: madopilot_key_t = 15;
/// Page up.
pub const MADOPILOT_KEY_PAGE_UP: madopilot_key_t = 16;
/// Page down.
pub const MADOPILOT_KEY_PAGE_DOWN: madopilot_key_t = 17;

/// One input event variant.
pub type madopilot_input_event_kind_t = i32;

/// No event.
pub const MADOPILOT_INPUT_EVENT_UNKNOWN: madopilot_input_event_kind_t = 0;
/// Move the pointer.
pub const MADOPILOT_INPUT_EVENT_POINTER_MOVE: madopilot_input_event_kind_t = 1;
/// Press a pointer button.
pub const MADOPILOT_INPUT_EVENT_POINTER_PRESS: madopilot_input_event_kind_t = 2;
/// Release a pointer button.
pub const MADOPILOT_INPUT_EVENT_POINTER_RELEASE: madopilot_input_event_kind_t = 3;
/// Scroll the pointer wheel.
pub const MADOPILOT_INPUT_EVENT_POINTER_SCROLL: madopilot_input_event_kind_t = 4;
/// Press a key.
pub const MADOPILOT_INPUT_EVENT_KEY_PRESS: madopilot_input_event_kind_t = 5;
/// Release a key.
pub const MADOPILOT_INPUT_EVENT_KEY_RELEASE: madopilot_input_event_kind_t = 6;
/// Enter text.
pub const MADOPILOT_INPUT_EVENT_TEXT: madopilot_input_event_kind_t = 7;
/// Wait before the next event.
pub const MADOPILOT_INPUT_EVENT_DELAY: madopilot_input_event_kind_t = 8;

/// The ABI 1.2 ceiling on events in one sequence.
///
/// A returned input descriptor may advertise a lower target-specific ceiling.
pub const MADOPILOT_INPUT_MAX_EVENTS: u32 = 256;
/// The most Unicode scalar values one text event may contain.
pub const MADOPILOT_INPUT_MAX_TEXT_CHARS: u32 = 4_096;
/// The most UTF-8 bytes a text event within the character ceiling can occupy.
pub const MADOPILOT_INPUT_MAX_TEXT_UTF8_BYTES: u32 = 16_384;
/// The longest delay event, in nanoseconds.
pub const MADOPILOT_INPUT_MAX_DELAY_NANOS: u64 = 5_000_000_000;
/// The maximum absolute value of either scroll component.
pub const MADOPILOT_INPUT_MAX_SCROLL_NOTCHES: i32 = 120;
/// The first accepted function-key number.
pub const MADOPILOT_INPUT_MIN_FUNCTION_KEY: u32 = 1;
/// The last accepted function-key number.
pub const MADOPILOT_INPUT_MAX_FUNCTION_KEY: u32 = 24;
/// The most release events an explicit cleanup budget may request.
pub const MADOPILOT_INPUT_MAX_CLEANUP_EVENTS: u32 = 256;
/// The longest explicit cleanup budget, in nanoseconds.
pub const MADOPILOT_INPUT_MAX_CLEANUP_NANOS: u64 = 250_000_000;

/// How far an admitted sequence got.
pub type madopilot_sequence_outcome_t = i32;

/// No event or partial native representation may have had an effect.
pub const MADOPILOT_SEQUENCE_UNEXECUTED: madopilot_sequence_outcome_t = 0;
/// Every logical event reached the selected route's submission threshold.
pub const MADOPILOT_SEQUENCE_COMPLETE: madopilot_sequence_outcome_t = 1;
/// Some input may have native effect and then the sequence stopped.
pub const MADOPILOT_SEQUENCE_PARTIAL: madopilot_sequence_outcome_t = 2;

/// What became of state a stopped sequence had pressed.
pub type madopilot_cleanup_state_t = i32;

/// The sequence held nothing when it stopped.
pub const MADOPILOT_CLEANUP_NOT_NEEDED: madopilot_cleanup_state_t = 0;
/// Every owned pressed state was released.
pub const MADOPILOT_CLEANUP_COMPLETE: madopilot_cleanup_state_t = 1;
/// A cleanup release was attempted and failed.
pub const MADOPILOT_CLEANUP_INCOMPLETE: madopilot_cleanup_state_t = 2;
/// Cleanup reached its own bound with releases still owed.
pub const MADOPILOT_CLEANUP_EXHAUSTED: madopilot_cleanup_state_t = 3;

/// What native object or subsystem a route addresses.
pub type madopilot_input_address_scope_t = i32;

/// No address scope is present.
pub const MADOPILOT_INPUT_ADDRESS_NONE: madopilot_input_address_scope_t = 0;
/// The system input stream and whichever target is focused.
pub const MADOPILOT_INPUT_ADDRESS_FOCUSED_SYSTEM: madopilot_input_address_scope_t = 1;
/// One exact selected window.
pub const MADOPILOT_INPUT_ADDRESS_EXACT_WINDOW: madopilot_input_address_scope_t = 2;
/// The process that owns the selected target.
pub const MADOPILOT_INPUT_ADDRESS_OWNING_PROCESS: madopilot_input_address_scope_t = 3;

/// The strongest native submission fact a route can report.
pub type madopilot_submission_evidence_t = i32;

/// No submission evidence is present.
pub const MADOPILOT_SUBMISSION_EVIDENCE_NONE: madopilot_submission_evidence_t = 0;
/// A posting API was invoked and returned without a submission result.
pub const MADOPILOT_SUBMISSION_EVIDENCE_INVOCATION_ONLY: madopilot_submission_evidence_t = 1;
/// The system input mechanism reported complete insertion.
pub const MADOPILOT_SUBMISSION_EVIDENCE_SYSTEM_INPUT_ADMISSION: madopilot_submission_evidence_t = 2;
/// The selected target queue accepted the native representation.
pub const MADOPILOT_SUBMISSION_EVIDENCE_TARGET_QUEUE_ADMISSION: madopilot_submission_evidence_t = 3;
/// A documented target-specific protocol acknowledged the logical event.
pub const MADOPILOT_SUBMISSION_EVIDENCE_TARGET_PROTOCOL_ACKNOWLEDGEMENT:
    madopilot_submission_evidence_t = 4;

/// Why an admitted sequence stopped, or why input was refused.
pub type madopilot_input_fault_t = i32;

/// No input fault.
pub const MADOPILOT_INPUT_FAULT_NONE: madopilot_input_fault_t = 0;
/// The target belongs to another engine or provider.
pub const MADOPILOT_INPUT_FAULT_FOREIGN_TARGET: madopilot_input_fault_t = 1;
/// The provider knows no such target.
pub const MADOPILOT_INPUT_FAULT_UNKNOWN_TARGET: madopilot_input_fault_t = 2;
/// The target no longer exists.
pub const MADOPILOT_INPUT_FAULT_TARGET_LOST: madopilot_input_fault_t = 3;
/// Capture and input providers do not match.
pub const MADOPILOT_INPUT_FAULT_PROVIDER_MISMATCH: madopilot_input_fault_t = 4;
/// The requested operation and route pair is unsupported.
pub const MADOPILOT_INPUT_FAULT_UNSUPPORTED_COMBINATION: madopilot_input_fault_t = 5;
/// The route plan is empty or repeats a route.
pub const MADOPILOT_INPUT_FAULT_INVALID_ROUTE_PLAN: madopilot_input_fault_t = 6;
/// Every caller-allowed route refused before native effect.
pub const MADOPILOT_INPUT_FAULT_ROUTE_UNAVAILABLE: madopilot_input_fault_t = 7;
/// The sequence or one of its events exceeds a bound.
pub const MADOPILOT_INPUT_FAULT_SEQUENCE_OUT_OF_BOUNDS: madopilot_input_fault_t = 8;
/// The target does not accept the pointer coordinate space.
pub const MADOPILOT_INPUT_FAULT_UNSUPPORTED_COORDINATE: madopilot_input_fault_t = 9;
/// The geometry policy requires a source frame.
pub const MADOPILOT_INPUT_FAULT_MISSING_COORDINATE_SOURCE: madopilot_input_fault_t = 10;
/// Target geometry changed since the source frame.
pub const MADOPILOT_INPUT_FAULT_GEOMETRY_CHANGED: madopilot_input_fault_t = 11;
/// Submission needs focus that policy withholds.
pub const MADOPILOT_INPUT_FAULT_FOCUS_REQUIRED: madopilot_input_fault_t = 12;
/// The operating system refused focus.
pub const MADOPILOT_INPUT_FAULT_FOCUS_REFUSED: madopilot_input_fault_t = 13;
/// Input control is not authorized.
pub const MADOPILOT_INPUT_FAULT_NOT_AUTHORIZED: madopilot_input_fault_t = 14;
/// Operating-system policy refused submission.
pub const MADOPILOT_INPUT_FAULT_POLICY_REFUSED: madopilot_input_fault_t = 15;
/// The input controller is closed.
pub const MADOPILOT_INPUT_FAULT_CONTROLLER_CLOSED: madopilot_input_fault_t = 16;
/// The operation was cancelled.
pub const MADOPILOT_INPUT_FAULT_CANCELLED: madopilot_input_fault_t = 17;
/// The operation deadline passed.
pub const MADOPILOT_INPUT_FAULT_DEADLINE_EXCEEDED: madopilot_input_fault_t = 18;
/// The platform reported another native submission failure.
pub const MADOPILOT_INPUT_FAULT_SUBMISSION_FAILED: madopilot_input_fault_t = 19;

/// Pointer/system capability pair.
pub const MADOPILOT_INPUT_PAIR_POINTER_SYSTEM: u64 = 1 << 0;
/// Pointer/window-message capability pair.
pub const MADOPILOT_INPUT_PAIR_POINTER_WINDOW_MESSAGE: u64 = 1 << 1;
/// Pointer/process-directed capability pair.
pub const MADOPILOT_INPUT_PAIR_POINTER_PROCESS_DIRECTED: u64 = 1 << 2;
/// Keyboard/system capability pair.
pub const MADOPILOT_INPUT_PAIR_KEYBOARD_SYSTEM: u64 = 1 << 3;
/// Keyboard/window-message capability pair.
pub const MADOPILOT_INPUT_PAIR_KEYBOARD_WINDOW_MESSAGE: u64 = 1 << 4;
/// Keyboard/process-directed capability pair.
pub const MADOPILOT_INPUT_PAIR_KEYBOARD_PROCESS_DIRECTED: u64 = 1 << 5;
/// Text/system capability pair.
pub const MADOPILOT_INPUT_PAIR_TEXT_SYSTEM: u64 = 1 << 6;
/// Text/window-message capability pair.
pub const MADOPILOT_INPUT_PAIR_TEXT_WINDOW_MESSAGE: u64 = 1 << 7;
/// Text/process-directed capability pair.
pub const MADOPILOT_INPUT_PAIR_TEXT_PROCESS_DIRECTED: u64 = 1 << 8;
/// Every input capability pair ABI 1.2 knows.
pub const MADOPILOT_INPUT_PAIRS_ALL: u64 = (1 << 9) - 1;
/// Diagnostic detail retained by an engine.
pub type madopilot_diagnostic_level_t = i32;
/// Diagnostics are allocation-free and disabled.
pub const MADOPILOT_DIAGNOSTIC_LEVEL_OFF: madopilot_diagnostic_level_t = 0;
/// Terminal public-operation summaries are retained.
pub const MADOPILOT_DIAGNOSTIC_LEVEL_NORMAL: madopilot_diagnostic_level_t = 1;
/// Normal summaries and bounded decision detail are retained.
pub const MADOPILOT_DIAGNOSTIC_LEVEL_DEBUG: madopilot_diagnostic_level_t = 2;

/// The observable result of one diagnostic drain.
pub type madopilot_diagnostic_drain_state_t = i32;
/// An owned batch was returned.
pub const MADOPILOT_DIAGNOSTIC_DRAIN_BATCH: madopilot_diagnostic_drain_state_t = 1;
/// No records or losses exist and production remains open.
pub const MADOPILOT_DIAGNOSTIC_DRAIN_OPEN_EMPTY: madopilot_diagnostic_drain_state_t = 2;
/// No records or losses exist and production is sealed.
pub const MADOPILOT_DIAGNOSTIC_DRAIN_END_OF_STREAM: madopilot_diagnostic_drain_state_t = 3;

/// A stable diagnostic payload category.
pub type madopilot_diagnostic_kind_t = i32;
/// An operation was admitted for observation.
pub const MADOPILOT_DIAGNOSTIC_KIND_OPERATION_STARTED: madopilot_diagnostic_kind_t = 1;
/// A published frame was observed.
pub const MADOPILOT_DIAGNOSTIC_KIND_FRAME: madopilot_diagnostic_kind_t = 2;
/// A frame mapping reached a terminal result.
pub const MADOPILOT_DIAGNOSTIC_KIND_MAPPING: madopilot_diagnostic_kind_t = 3;
/// A template search reached a terminal result.
pub const MADOPILOT_DIAGNOSTIC_KIND_SEARCH: madopilot_diagnostic_kind_t = 4;
/// An input submission reached a terminal receipt.
pub const MADOPILOT_DIAGNOSTIC_KIND_INPUT: madopilot_diagnostic_kind_t = 5;
/// One input delivery route was attempted.
pub const MADOPILOT_DIAGNOSTIC_KIND_ROUTE_ATTEMPT: madopilot_diagnostic_kind_t = 6;
/// A session lifecycle transition was observed.
pub const MADOPILOT_DIAGNOSTIC_KIND_LIFECYCLE: madopilot_diagnostic_kind_t = 7;
/// A non-prompting permission probe reached a terminal result.
pub const MADOPILOT_DIAGNOSTIC_KIND_PERMISSION: madopilot_diagnostic_kind_t = 8;
/// One OCR recognition reached a terminal result.
pub const MADOPILOT_DIAGNOSTIC_KIND_OCR: madopilot_diagnostic_kind_t = 9;

/// A public operation observed by diagnostics.
pub type madopilot_diagnostic_operation_kind_t = i32;
/// Target discovery.
pub const MADOPILOT_DIAGNOSTIC_OPERATION_DISCOVERY: madopilot_diagnostic_operation_kind_t = 1;
/// Input capability description.
pub const MADOPILOT_DIAGNOSTIC_OPERATION_INPUT_DESCRIPTION: madopilot_diagnostic_operation_kind_t =
    2;
/// Non-prompting permission probing.
pub const MADOPILOT_DIAGNOSTIC_OPERATION_PERMISSION: madopilot_diagnostic_operation_kind_t = 3;
/// Capture-session opening.
pub const MADOPILOT_DIAGNOSTIC_OPERATION_SESSION_OPEN: madopilot_diagnostic_operation_kind_t = 4;
/// Frame acquisition.
pub const MADOPILOT_DIAGNOSTIC_OPERATION_FRAME_ACQUIRE: madopilot_diagnostic_operation_kind_t = 5;
/// Frame mapping.
pub const MADOPILOT_DIAGNOSTIC_OPERATION_MAPPING: madopilot_diagnostic_operation_kind_t = 6;
/// Template preparation.
pub const MADOPILOT_DIAGNOSTIC_OPERATION_TEMPLATE_PREPARATION:
    madopilot_diagnostic_operation_kind_t = 7;
/// Template search.
pub const MADOPILOT_DIAGNOSTIC_OPERATION_SEARCH: madopilot_diagnostic_operation_kind_t = 8;
/// Input submission.
pub const MADOPILOT_DIAGNOSTIC_OPERATION_INPUT_SUBMISSION: madopilot_diagnostic_operation_kind_t =
    9;
/// Capture-session closing.
pub const MADOPILOT_DIAGNOSTIC_OPERATION_SESSION_CLOSE: madopilot_diagnostic_operation_kind_t = 10;
/// One-shot OCR recognition.
pub const MADOPILOT_DIAGNOSTIC_OPERATION_OCR_RECOGNITION: madopilot_diagnostic_operation_kind_t =
    11;

/// A terminal template-search result in diagnostics.
pub type madopilot_search_diagnostic_outcome_t = i32;
/// The search produced at least one match.
pub const MADOPILOT_SEARCH_DIAGNOSTIC_MATCHED: madopilot_search_diagnostic_outcome_t = 1;
/// The search completed with no match.
pub const MADOPILOT_SEARCH_DIAGNOSTIC_NO_MATCH: madopilot_search_diagnostic_outcome_t = 2;
/// The search failed before producing a result.
pub const MADOPILOT_SEARCH_DIAGNOSTIC_FAILED: madopilot_search_diagnostic_outcome_t = 3;

/// Explicit non-default product OCR profile selection.
pub type madopilot_ocr_profile_kind_t = i32;
/// ADR 0040/0041 bounded-detector profile.
pub const MADOPILOT_OCR_PROFILE_BOUNDED_DETECTOR: madopilot_ocr_profile_kind_t = 1;

/// Accepted public OCR profile classification in diagnostics.
pub type madopilot_ocr_diagnostic_profile_t = i32;
/// No accepted public profile claim is made.
pub const MADOPILOT_OCR_DIAGNOSTIC_PROFILE_UNSPECIFIED: madopilot_ocr_diagnostic_profile_t = 0;
/// Accepted G-004 RapidOCR PP-OCRv4 detector / PP-OCRv6 recognizer profile.
pub const MADOPILOT_OCR_DIAGNOSTIC_PROFILE_G004: madopilot_ocr_diagnostic_profile_t = 1;
/// Accepted ADR 0040/0041 bounded-detector profile.
pub const MADOPILOT_OCR_DIAGNOSTIC_PROFILE_BOUNDED: madopilot_ocr_diagnostic_profile_t = 2;

/// Typed terminal OCR outcome in diagnostics.
pub type madopilot_ocr_diagnostic_outcome_t = i32;
/// No OCR outcome is present.
pub const MADOPILOT_OCR_DIAGNOSTIC_OUTCOME_UNSPECIFIED: madopilot_ocr_diagnostic_outcome_t = 0;
/// One or more normalized regions committed.
pub const MADOPILOT_OCR_DIAGNOSTIC_OUTCOME_RECOGNIZED: madopilot_ocr_diagnostic_outcome_t = 1;
/// Recognition committed with no non-empty normalized text.
pub const MADOPILOT_OCR_DIAGNOSTIC_OUTCOME_EMPTY: madopilot_ocr_diagnostic_outcome_t = 2;
/// Recognition failed with the record's typed status.
pub const MADOPILOT_OCR_DIAGNOSTIC_OUTCOME_FAILED: madopilot_ocr_diagnostic_outcome_t = 3;

/// A lifecycle state in diagnostics.
pub type madopilot_lifecycle_t = i32;
/// The session is open.
pub const MADOPILOT_LIFECYCLE_OPEN: madopilot_lifecycle_t = 1;
/// Session close has begun.
pub const MADOPILOT_LIFECYCLE_CLOSING: madopilot_lifecycle_t = 2;
/// The session is closed.
pub const MADOPILOT_LIFECYCLE_CLOSED: madopilot_lifecycle_t = 3;

/// `madopilot_operation_t.deadline_nanos` carries an absolute deadline.
///
/// Without it the operation has no deadline, which is not the same as a very
/// large one: zero nanoseconds is the domain origin and a valid instant.
pub const MADOPILOT_OPERATION_HAS_DEADLINE: u32 = 1 << 0;
/// `madopilot_operation_t.activity_tag` carries an opaque diagnostic correlation value.
pub const MADOPILOT_OPERATION_HAS_ACTIVITY_TAG: u32 = 1 << 1;

/// `madopilot_open_request_t.required_format` is set.
pub const MADOPILOT_OPEN_HAS_REQUIRED_FORMAT: u32 = 1 << 0;
/// `madopilot_open_request_t.preferred_format` is set.
pub const MADOPILOT_OPEN_HAS_PREFERRED_FORMAT: u32 = 1 << 1;

/// `madopilot_map_request_t.region` is set; the whole frame is mapped without it.
pub const MADOPILOT_MAP_HAS_REGION: u32 = 1 << 0;

/// `madopilot_find_request_t.region` is set; the whole frame is searched without it.
pub const MADOPILOT_FIND_HAS_REGION: u32 = 1 << 0;
/// `madopilot_ocr_request_t.region` is set; the whole frame is recognized without it.
pub const MADOPILOT_OCR_HAS_REGION: u32 = 1 << 0;

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
/// `madopilot_target_t.kind` is populated.
pub const MADOPILOT_TARGET_HAS_KIND: u32 = 1 << 1;
/// `madopilot_target_t.capture_permission` is populated.
pub const MADOPILOT_TARGET_HAS_CAPTURE_PERMISSION: u32 = 1 << 2;

/// The engine can deliver input to at least some targets.
pub const MADOPILOT_ENGINE_DELIVERS_INPUT: u32 = 1 << 0;
/// The engine can run non-prompting permission probes.
pub const MADOPILOT_ENGINE_READS_PERMISSIONS: u32 = 1 << 1;
/// The engine has one configured OCR backend/model profile.
pub const MADOPILOT_ENGINE_HAS_OCR: u32 = 1 << 2;

/// `madopilot_permission_t` carries a redacted diagnostic.
pub const MADOPILOT_PERMISSION_HAS_DIAGNOSTIC: u32 = 1 << 0;
/// `madopilot_permission_t` carries a platform code and namespace.
pub const MADOPILOT_PERMISSION_HAS_PLATFORM_CODE: u32 = 1 << 1;

/// `madopilot_input_capability_t.permission` is populated.
pub const MADOPILOT_INPUT_CAPABILITY_HAS_PERMISSION: u32 = 1 << 0;
/// `madopilot_input_capability_t.evidence` is populated.
pub const MADOPILOT_INPUT_CAPABILITY_HAS_EVIDENCE: u32 = 1 << 1;

/// `madopilot_input_request_t` supplies explicit cleanup bounds.
pub const MADOPILOT_INPUT_REQUEST_HAS_CLEANUP_BUDGET: u32 = 1 << 0;

/// `madopilot_input_receipt_info_t.selected_route` and `address_scope` are populated.
pub const MADOPILOT_INPUT_RECEIPT_HAS_SELECTED_ROUTE: u32 = 1 << 0;
/// `madopilot_input_receipt_info_t.last_submitted` is populated.
pub const MADOPILOT_INPUT_RECEIPT_HAS_LAST_SUBMITTED: u32 = 1 << 1;
/// `madopilot_input_receipt_info_t.evidence` is populated.
pub const MADOPILOT_INPUT_RECEIPT_HAS_EVIDENCE: u32 = 1 << 2;
/// `madopilot_input_receipt_info_t.fault` is populated.
pub const MADOPILOT_INPUT_RECEIPT_HAS_FAULT: u32 = 1 << 3;
/// The current incomplete logical event may have native effect.
pub const MADOPILOT_INPUT_RECEIPT_PARTIAL_NATIVE_EFFECT: u32 = 1 << 4;
/// The selected route followed at least one refused route.
pub const MADOPILOT_INPUT_RECEIPT_USED_FALLBACK: u32 = 1 << 5;

/// `madopilot_input_attempt_t.last_submitted` is populated.
pub const MADOPILOT_INPUT_ATTEMPT_HAS_LAST_SUBMITTED: u32 = 1 << 0;
/// `madopilot_input_attempt_t.evidence` is populated.
pub const MADOPILOT_INPUT_ATTEMPT_HAS_EVIDENCE: u32 = 1 << 1;
/// `madopilot_input_attempt_t.fault` is populated.
pub const MADOPILOT_INPUT_ATTEMPT_HAS_FAULT: u32 = 1 << 2;
/// The current incomplete logical event may have native effect.
pub const MADOPILOT_INPUT_ATTEMPT_PARTIAL_NATIVE_EFFECT: u32 = 1 << 3;

/// `madopilot_diagnostic_record_t.activity_tag` is populated.
pub const MADOPILOT_DIAGNOSTIC_RECORD_HAS_ACTIVITY: u32 = 1 << 0;
/// `madopilot_diagnostic_record_t.target` is populated.
pub const MADOPILOT_DIAGNOSTIC_RECORD_HAS_TARGET: u32 = 1 << 1;
/// `madopilot_diagnostic_record_t.frame` is populated.
pub const MADOPILOT_DIAGNOSTIC_RECORD_HAS_FRAME: u32 = 1 << 2;
/// `madopilot_diagnostic_record_t.template_identity` is populated.
pub const MADOPILOT_DIAGNOSTIC_RECORD_HAS_TEMPLATE: u32 = 1 << 3;
/// `madopilot_diagnostic_record_t.source_space` is populated.
pub const MADOPILOT_DIAGNOSTIC_RECORD_HAS_SOURCE_SPACE: u32 = 1 << 4;
/// `madopilot_diagnostic_record_t.destination_space` is populated.
pub const MADOPILOT_DIAGNOSTIC_RECORD_HAS_DESTINATION_SPACE: u32 = 1 << 5;
/// `madopilot_diagnostic_record_t.region` is populated.
pub const MADOPILOT_DIAGNOSTIC_RECORD_HAS_REGION: u32 = 1 << 6;
/// `madopilot_diagnostic_record_t.route` is populated.
pub const MADOPILOT_DIAGNOSTIC_RECORD_HAS_ROUTE: u32 = 1 << 7;
/// `madopilot_diagnostic_record_t.address_scope` is populated.
pub const MADOPILOT_DIAGNOSTIC_RECORD_HAS_ADDRESS_SCOPE: u32 = 1 << 8;
/// `madopilot_diagnostic_record_t.evidence` is populated.
pub const MADOPILOT_DIAGNOSTIC_RECORD_HAS_EVIDENCE: u32 = 1 << 9;
/// `madopilot_diagnostic_record_t.input_fault` is populated.
pub const MADOPILOT_DIAGNOSTIC_RECORD_HAS_INPUT_FAULT: u32 = 1 << 10;
/// `madopilot_diagnostic_record_t.status` is populated.
pub const MADOPILOT_DIAGNOSTIC_RECORD_HAS_STATUS: u32 = 1 << 11;
/// `madopilot_diagnostic_record_t.permission_state` is populated.
pub const MADOPILOT_DIAGNOSTIC_RECORD_HAS_PERMISSION_STATE: u32 = 1 << 12;
/// `madopilot_diagnostic_record_t.ocr_model_instance` is populated.
pub const MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_MODEL_INSTANCE: u32 = 1 << 13;
/// `madopilot_diagnostic_record_t.ocr_profile` is an accepted public profile.
pub const MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_PROFILE: u32 = 1 << 14;
/// `madopilot_diagnostic_record_t.ocr_requested_region` is populated.
pub const MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_REQUESTED_REGION: u32 = 1 << 15;
/// `madopilot_diagnostic_record_t.ocr_elapsed_nanos` is populated.
pub const MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_TIMING: u32 = 1 << 16;
/// `madopilot_diagnostic_record_t.ocr_source_pixels` is populated.
pub const MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_RESOURCES: u32 = 1 << 17;
/// `madopilot_diagnostic_record_t.ocr_source_envelope` is populated.
pub const MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_SOURCE_ENVELOPE: u32 = 1 << 18;
/// `madopilot_diagnostic_record_t.ocr_zone_count` is populated.
pub const MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_ZONE_COUNT: u32 = 1 << 19;
/// Unique-candidate and membership result counts are populated.
pub const MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_RESULT_COUNTS: u32 = 1 << 20;
/// `madopilot_diagnostic_record_t.ocr_result_bytes` is populated.
pub const MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_RESULT_BYTES: u32 = 1 << 21;
/// Exact request-scoped detector/recognizer work is populated.
pub const MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_BACKEND_WORK: u32 = 1 << 22;

/// `madopilot_error_detail_t` carries `asset_fault` and `asset_stage`.
pub const MADOPILOT_ERROR_HAS_ASSET_DETAIL: u32 = 1 << 0;
/// `madopilot_error_detail_t.backend` names the backend that failed.
pub const MADOPILOT_ERROR_HAS_BACKEND: u32 = 1 << 1;

/// One point in a declared coordinate space.
///
/// Not size-versioned: four points are embedded by value in one OCR region.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct madopilot_ocr_point_t {
    /// Horizontal coordinate.
    pub x: f64,
    /// Vertical coordinate.
    pub y: f64,
}

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
/// Capability flags that apply to the whole engine.
///
/// Mandatory prefix: the whole structure.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_engine_capabilities_t {
    /// `sizeof(madopilot_engine_capabilities_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// [`MADOPILOT_ENGINE_DELIVERS_INPUT`],
    /// [`MADOPILOT_ENGINE_READS_PERMISSIONS`], and [`MADOPILOT_ENGINE_HAS_OCR`].
    pub flags: u32,
}

/// One non-prompting permission-probe result.
///
/// Mandatory prefix: through `state`. Diagnostic fields are present only when
/// their corresponding flag is set. Borrowed strings remain valid while the
/// engine is retained.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_permission_t {
    /// `sizeof(madopilot_permission_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// [`MADOPILOT_PERMISSION_HAS_DIAGNOSTIC`] and
    /// [`MADOPILOT_PERMISSION_HAS_PLATFORM_CODE`].
    pub flags: u32,
    /// The sensitive capability that was probed.
    pub kind: madopilot_permission_kind_t,
    /// The result of the probe.
    pub state: madopilot_permission_state_t,
    /// Redacted diagnostic category, when present.
    pub diagnostic_category: madopilot_diagnostic_category_t,
    /// Reserved; written as zero.
    pub reserved: u32,
    /// Platform-specific numeric code, when present.
    pub platform_code: i64,
    /// Namespace for `platform_code`, when present.
    pub platform_namespace: madopilot_str_t,
    /// Redacted diagnostic context, when present.
    pub context: madopilot_str_t,
}

/// Capability data for one operation/route pair on one target.
///
/// Mandatory prefix: through `support`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_input_capability_t {
    /// `sizeof(madopilot_input_capability_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// `MADOPILOT_INPUT_CAPABILITY_HAS_*` presence bits.
    pub flags: u32,
    /// Engine-local target identity.
    pub target: u64,
    /// Input operation kind.
    pub operation: madopilot_input_operation_kind_t,
    /// Explicit delivery route.
    pub delivery: madopilot_input_delivery_t,
    /// Whether this pair can be attempted.
    pub support: madopilot_capability_support_t,
    /// The subsystem addressed by this route.
    pub address_scope: madopilot_input_address_scope_t,
    /// Permission required for this pair, when present.
    pub permission: madopilot_permission_kind_t,
    /// Strongest submission evidence the route can report, when present.
    pub evidence: madopilot_submission_evidence_t,
    /// Nonzero when focus is required.
    pub focus_required: i32,
    /// Bit `1 << space` is set for every accepted pointer coordinate space.
    pub pointer_spaces: u32,
    /// Reserved; written as zero.
    pub reserved: u32,
}

/// Input requested while opening a capture session.
///
/// Mandatory prefix: through `requirement`. Pair masks contain only
/// `MADOPILOT_INPUT_PAIR_*` bits.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_input_open_request_t {
    /// `sizeof(madopilot_input_open_request_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// No bits are defined; the caller sets zero.
    pub flags: u32,
    /// Whether capture-only open is acceptable.
    pub requirement: madopilot_input_requirement_t,
    /// Reserved; the caller sets zero.
    pub reserved: u32,
    /// Operation/route pairs that must be accepted.
    pub required_pairs: u64,
    /// Additional operation/route pairs to request.
    pub preferred_pairs: u64,
}

/// What input an engine or an open session accepts.
///
/// Mandatory prefix: the whole structure.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_input_descriptor_t {
    /// `sizeof(madopilot_input_descriptor_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// No bits are defined; written as zero.
    pub flags: u32,
    /// Engine-local target identity, or zero for an engine-wide descriptor.
    pub target: u64,
    /// Every pair for which support is known.
    pub known_pairs: u64,
    /// Known pairs whose support is `Supported`.
    pub supported_pairs: u64,
    /// Pairs whose support is `Unknown`.
    pub unknown_pairs: u64,
    /// Bit `1 << space` is set for every accepted pointer coordinate space.
    pub pointer_spaces: u32,
    /// Maximum events in one admitted sequence.
    pub max_events: u32,
}

/// One event in an input sequence.
///
/// Mandatory prefix varies by `kind`; fields not selected by `kind` are ignored.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_input_event_t {
    /// `sizeof(madopilot_input_event_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// Which event variant this record carries.
    pub kind: madopilot_input_event_kind_t,
    /// Pointer coordinate space.
    pub space: madopilot_space_t,
    /// Pointer button.
    pub button: madopilot_pointer_button_t,
    /// Key kind.
    pub key: madopilot_key_t,
    /// Unicode scalar, function-key number, or modifier value selected by `key`.
    pub key_value: u32,
    /// Pointer horizontal coordinate.
    pub x: f64,
    /// Pointer vertical coordinate.
    pub y: f64,
    /// Horizontal scroll amount.
    pub horizontal: i32,
    /// Vertical scroll amount.
    pub vertical: i32,
    /// Borrowed UTF-8 text for `MADOPILOT_INPUT_EVENT_TEXT`.
    pub text: madopilot_str_t,
    /// Delay before the next event.
    pub delay_nanos: u64,
}

/// One bounded input sequence and its delivery policy.
///
/// Mandatory prefix: through `source_frame`. Event text and both arrays are
/// borrowed for the call. The caller keeps `source_frame` retained for the call.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_input_request_t {
    /// `sizeof(madopilot_input_request_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// [`MADOPILOT_INPUT_REQUEST_HAS_CLEANUP_BUDGET`].
    pub flags: u32,
    /// First event in a strided caller-owned array.
    pub events: *const madopilot_input_event_t,
    /// Number of events.
    pub event_count: usize,
    /// Distance in bytes between event starts.
    pub event_stride: usize,
    /// Ordered caller-owned array of requested delivery routes.
    pub deliveries: *const madopilot_input_delivery_t,
    /// Number of delivery routes.
    pub delivery_count: usize,
    /// Focus policy for this sequence.
    pub focus_policy: madopilot_focus_policy_t,
    /// Geometry policy for pointer coordinates.
    pub geometry_policy: madopilot_geometry_policy_t,
    /// Source frame for geometry policies that require one, or null.
    pub source_frame: *const madopilot_frame_t,
    /// Maximum cleanup releases, when the cleanup-budget flag is set.
    pub cleanup_max_events: u32,
    /// Reserved; the caller sets zero.
    pub reserved: u32,
    /// Cleanup timeout, when the cleanup-budget flag is set.
    pub cleanup_timeout_nanos: u64,
}

/// Fixed fields of one immutable owned input receipt.
///
/// Mandatory prefix: the whole structure. Presence flags distinguish optional
/// scalar values from valid zero values. Route attempts are accessed separately.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_input_receipt_info_t {
    /// `sizeof(madopilot_input_receipt_info_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// `MADOPILOT_INPUT_RECEIPT_*` bits.
    pub flags: u32,
    /// Engine-local target identity.
    pub target: u64,
    /// Complete, partial, or unexecuted.
    pub outcome: madopilot_sequence_outcome_t,
    /// Selected route, when present.
    pub selected_route: madopilot_input_delivery_t,
    /// Address scope of `selected_route`, when present.
    pub address_scope: madopilot_input_address_scope_t,
    /// Number of immutable route attempts.
    pub attempt_count: u64,
    /// Number of complete logical events submitted.
    pub submitted: u64,
    /// Last complete event index submitted, when present.
    pub last_submitted: u64,
    /// Strongest submission evidence for the selected route, when present.
    pub evidence: madopilot_submission_evidence_t,
    /// Typed terminal input fault, when present.
    pub fault: madopilot_input_fault_t,
    /// Cleanup outcome.
    pub cleanup: madopilot_cleanup_state_t,
    /// Number of cleanup releases completed.
    pub cleanup_released: u64,
    /// Number of cleanup releases still owed.
    pub cleanup_owed: u64,
}

/// One immutable route attempt borrowed from an input receipt.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_input_attempt_t {
    /// `sizeof(madopilot_input_attempt_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// `MADOPILOT_INPUT_ATTEMPT_*` bits.
    pub flags: u32,
    /// Attempted route.
    pub route: madopilot_input_delivery_t,
    /// Address scope of `route`.
    pub address_scope: madopilot_input_address_scope_t,
    /// Complete, partial, or unexecuted.
    pub outcome: madopilot_sequence_outcome_t,
    /// Number of complete logical events submitted.
    pub submitted: u64,
    /// Last complete event index submitted, when present.
    pub last_submitted: u64,
    /// Strongest native submission evidence, when present.
    pub evidence: madopilot_submission_evidence_t,
    /// Typed route fault, when present.
    pub fault: madopilot_input_fault_t,
    /// Reserved; written as zero.
    pub reserved: u32,
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
    /// Default OCR backend identity. Borrowed from static storage.
    pub default_ocr_backend: madopilot_str_t,
    /// Exact default OCR backend version. Borrowed from static storage.
    pub default_ocr_backend_version: madopilot_str_t,
    /// Controlled runtime/provider profile. Borrowed from static storage.
    pub default_ocr_runtime_profile: madopilot_str_t,
    /// Accepted default OCR model identity. Borrowed from static storage.
    pub default_ocr_model: madopilot_str_t,
    /// Accepted default OCR model version. Borrowed from static storage.
    pub default_ocr_model_version: madopilot_str_t,
    /// Accepted default OCR profile identity. Borrowed from static storage.
    pub default_ocr_profile: madopilot_str_t,
    /// Accepted explicit bounded OCR model identity. Borrowed from static storage.
    pub bounded_ocr_model: madopilot_str_t,
    /// Accepted explicit bounded OCR model version. Borrowed from static storage.
    pub bounded_ocr_model_version: madopilot_str_t,
    /// Accepted explicit bounded OCR profile identity. Borrowed from static storage.
    pub bounded_ocr_profile: madopilot_str_t,
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
    /// Opaque nonzero diagnostic correlation tag, when its presence flag is set.
    pub activity_tag: u64,
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
/// Engine-wide diagnostic configuration.
///
/// Mandatory prefix: the whole structure. `Off` requires zero capacity; enabled
/// levels require a capacity from 1 through 65,536.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_engine_options_t {
    /// `sizeof(madopilot_engine_options_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// No bits are defined; the caller sets zero.
    pub flags: u32,
    /// Engine-wide diagnostic level.
    pub diagnostic_level: madopilot_diagnostic_level_t,
    /// Maximum retained records.
    pub diagnostic_capacity: u32,
}

/// Explicit controlled paths for integrated default OCR construction.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_default_ocr_options_t {
    /// `sizeof(madopilot_default_ocr_options_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// No bits are defined; the caller sets zero.
    pub flags: u32,
    /// Absolute root containing the fixed G-004 relative model paths.
    pub model_root: madopilot_str_t,
    /// Canonical absolute ONNX Runtime 1.29.0 file.
    pub runtime_path: madopilot_str_t,
}

/// Explicit controlled profile and paths for integrated OCR construction.
///
/// Mandatory prefix: the whole structure. Both string views are borrowed for
/// `engine_create_with_ocr_profile` only.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_ocr_profile_options_t {
    /// `sizeof(madopilot_ocr_profile_options_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// No bits are defined; the caller sets zero.
    pub flags: u32,
    /// Closed product-profile selection.
    pub kind: madopilot_ocr_profile_kind_t,
    /// Reserved; the caller sets zero.
    pub reserved: u32,
    /// Absolute root containing the selected profile's fixed model paths.
    pub model_root: madopilot_str_t,
    /// Canonical absolute ONNX Runtime 1.29.0 file.
    pub runtime_path: madopilot_str_t,
}

/// Exact OCR backend/model/profile identity selected by one engine.
///
/// Mandatory prefix: the whole structure. String views remain borrowed while
/// the engine is retained.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_ocr_engine_descriptor_t {
    /// `sizeof(madopilot_ocr_engine_descriptor_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// No bits are defined; written as zero.
    pub flags: u32,
    /// Selected OCR backend identifier.
    pub backend_id: madopilot_str_t,
    /// Selected OCR backend implementation version.
    pub backend_version: madopilot_str_t,
    /// Selected OCR model identifier.
    pub model_id: madopilot_str_t,
    /// Selected OCR model version.
    pub model_version: madopilot_str_t,
    /// Selected OCR profile identifier.
    pub profile_id: madopilot_str_t,
}

/// Summary of one immutable owned diagnostic batch.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_diagnostic_batch_info_t {
    /// `sizeof(madopilot_diagnostic_batch_info_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// No bits are defined; written as zero.
    pub flags: u32,
    /// Semantic record count.
    pub record_count: u64,
    /// Normal records discarded since the preceding committed batch.
    pub discarded_normal: u64,
    /// Debug records discarded since the preceding committed batch.
    pub discarded_debug: u64,
}

/// Exact caller-requested OCR geometry.
///
/// Not size-versioned: it is appended by value to the extensible diagnostic record.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct madopilot_ocr_requested_region_t {
    /// Coordinate space of all four edges.
    pub space: madopilot_space_t,
    /// Requested clipping policy.
    pub clip_policy: madopilot_clip_policy_t,
    /// Requested left edge.
    pub left: f64,
    /// Requested top edge.
    pub top: f64,
    /// Requested right edge.
    pub right: f64,
    /// Requested bottom edge.
    pub bottom: f64,
}

impl madopilot_ocr_requested_region_t {
    /// Failure state.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            space: MADOPILOT_SPACE_CAPTURE_PIXELS,
            clip_policy: MADOPILOT_CLIP_POLICY_REJECT,
            left: 0.0,
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
        }
    }
}

/// One privacy-reviewed immutable diagnostic record.
///
/// The mandatory released ABI 1.2 prefix ends through `cleanup_owed`; ABI 1.3
/// appends singular OCR and ABI 1.4 appends grouped aggregate fields. `kind`
/// selects payload fields, and presence flags distinguish optional scalar
/// values from valid zero values.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_diagnostic_record_t {
    /// `sizeof(Self)` as declared by the caller.
    pub struct_size: u32,
    /// `MADOPILOT_DIAGNOSTIC_RECORD_HAS_*` bits.
    pub flags: u32,
    /// Strict engine-local commit order.
    pub sequence: u64,
    /// Timestamp in the library monotonic-clock domain.
    pub timestamp_nanos: u64,
    /// Checked nonzero engine-local operation identity.
    pub operation_id: u64,
    /// Opaque caller correlation tag, when present.
    pub activity_tag: u64,
    /// Normal or debug.
    pub level: madopilot_diagnostic_level_t,
    /// Stable payload category.
    pub kind: madopilot_diagnostic_kind_t,
    /// Public operation kind for operation-started records.
    pub operation: madopilot_diagnostic_operation_kind_t,
    /// Public terminal status, when present.
    pub status: crate::status::madopilot_status_t,
    /// Nonzero engine-local target ordinal, when present.
    pub target: u64,
    /// Complete frame identity, when present.
    pub frame: madopilot_frame_stamp_t,
    /// Engine-issued template identity, when present.
    pub template_identity: u64,
    /// Mapping source coordinate space, when present.
    pub source_space: madopilot_space_t,
    /// Mapping destination coordinate space, when present.
    pub destination_space: madopilot_space_t,
    /// Exact searched region in capture pixels, when present.
    pub region: madopilot_pixel_rect_t,
    /// Route, when present.
    pub route: madopilot_input_delivery_t,
    /// Route address scope, when present.
    pub address_scope: madopilot_input_address_scope_t,
    /// Strongest submission evidence, when present.
    pub evidence: madopilot_submission_evidence_t,
    /// Typed input fault, when present.
    pub input_fault: madopilot_input_fault_t,
    /// Complete, partial, or unexecuted for input records.
    pub input_outcome: madopilot_sequence_outcome_t,
    /// Cleanup terminal state for input records.
    pub cleanup: madopilot_cleanup_state_t,
    /// Permission kind for permission records.
    pub permission_kind: madopilot_permission_kind_t,
    /// Permission state, when present.
    pub permission_state: madopilot_permission_state_t,
    /// Lifecycle state for lifecycle records.
    pub lifecycle: madopilot_lifecycle_t,
    /// Terminal search result for search records.
    pub search_outcome: madopilot_search_diagnostic_outcome_t,
    /// `MADOPILOT_INPUT_OPERATION_*` bits represented as `1 << (kind - 1)`.
    pub input_operations: u32,
    /// Nonzero when the current native unit may have partial effect.
    pub partial_native_effect: i32,
    /// Nonzero when an earlier route was refused.
    pub used_fallback: i32,
    /// Reserved; written as zero.
    pub reserved: u32,
    /// Requested logical event count.
    pub requested: u64,
    /// Complete logical events submitted.
    pub submitted: u64,
    /// Semantic search result count.
    pub result_count: u64,
    /// Cleanup releases completed.
    pub cleanup_released: u64,
    /// Cleanup releases owed when cleanup began.
    pub cleanup_owed: u64,
    /// Opaque library-issued OCR model-instance identity, when present.
    pub ocr_model_instance: u64,
    /// Accepted public OCR profile classification.
    pub ocr_profile: madopilot_ocr_diagnostic_profile_t,
    /// Typed terminal OCR outcome.
    pub ocr_outcome: madopilot_ocr_diagnostic_outcome_t,
    /// Exact caller-requested region, when present.
    pub ocr_requested_region: madopilot_ocr_requested_region_t,
    /// Caller-clock elapsed recognition duration in nanoseconds.
    pub ocr_elapsed_nanos: u64,
    /// Effective source-region pixel count.
    pub ocr_source_pixels: u64,
    /// Shared grouped source envelope in capture pixels, when present.
    pub ocr_source_envelope: madopilot_pixel_rect_t,
    /// Reserved; written as zero.
    pub ocr_grouped_reserved: u32,
    /// Caller zone count, when grouped counts are present.
    pub ocr_zone_count: u64,
    /// Unique retained candidate count, when grouped counts are present.
    pub ocr_unique_candidate_count: u64,
    /// Group-relative membership count, when grouped counts are present.
    pub ocr_membership_count: u64,
    /// Exact immutable result semantic bytes, when available.
    pub ocr_result_bytes: u64,
    /// Exact detector runs for this request, when available.
    pub ocr_detector_runs: u64,
    /// Exact recognizer runs for this request, when available.
    pub ocr_recognizer_runs: u64,
    /// Exact detector bytes for this request, when available.
    pub ocr_detector_bytes: u64,
    /// Exact recognizer bytes for this request, when available.
    pub ocr_recognizer_bytes: u64,
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
    /// [`MADOPILOT_TARGET_SUPPORTS_PLACEMENT`],
    /// [`MADOPILOT_TARGET_HAS_KIND`], and
    /// [`MADOPILOT_TARGET_HAS_CAPTURE_PERMISSION`].
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
    /// Engine-local target identity.
    pub target: u64,
    /// Target kind, when `MADOPILOT_TARGET_HAS_KIND` is set.
    pub kind: madopilot_target_kind_t,
    /// Whether capture can be attempted.
    pub capture: madopilot_capability_support_t,
    /// Required capture permission, when its presence flag is set.
    pub capture_permission: madopilot_permission_kind_t,
    /// Reserved; written as zero.
    pub reserved: u32,
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
    /// Nonzero engine-local target ordinal.
    pub target: u64,
    /// One when the session accepted input, zero for capture-only.
    pub accepts_input: i32,
    /// Reserved; written as zero.
    pub reserved: u32,
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
    /// [`MADOPILOT_OPEN_HAS_REQUIRED_FORMAT`] and
    /// [`MADOPILOT_OPEN_HAS_PREFERRED_FORMAT`].
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
    pub frame: *const madopilot_frame_t,
    /// The prepared template to search for. Required.
    pub tmpl: *const crate::assets::madopilot_template_t,
    /// The options to search under, or null for the template's own defaults.
    pub options: *const madopilot_match_options_t,
    /// The region to search, when [`MADOPILOT_FIND_HAS_REGION`] is set.
    pub region: madopilot_pixel_rect_t,
    /// What to do with a region that leaves the frame.
    pub clip_policy: madopilot_clip_policy_t,
}

/// One OCR operation against one exact retained frame.
///
/// Mandatory prefix: through `output_space`. The frame and package handles and
/// all string views are borrowed for the call. `model_id` resolves one complete
/// validated model/profile identity from `package`; backend ID and version must
/// equal the backend explicitly configured on the source session.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_ocr_request_t {
    /// `sizeof(madopilot_ocr_request_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// [`MADOPILOT_OCR_HAS_REGION`].
    pub flags: u32,
    /// The exact retained source frame. Required.
    pub frame: *const madopilot_frame_t,
    /// The validated package from which `model_id` is resolved. Required.
    pub package: *const crate::assets::madopilot_package_t,
    /// Stable package model identity. Required and non-empty.
    pub model_id: madopilot_str_t,
    /// Stable configured backend identity. Required and non-empty.
    pub backend_id: madopilot_str_t,
    /// Exact configured backend implementation version. Required and non-empty.
    pub backend_version: madopilot_str_t,
    /// Coordinate space of every returned quadrilateral.
    pub output_space: madopilot_space_t,
    /// What to do when the optional source region leaves the frame.
    pub clip_policy: madopilot_clip_policy_t,
    /// Optional source region selected by [`MADOPILOT_OCR_HAS_REGION`].
    pub region: madopilot_pixel_rect_t,
}

/// One caller-order capture-pixel OCR zone.
///
/// Mandatory prefix: the whole structure.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_ocr_zone_t {
    /// `sizeof(madopilot_ocr_zone_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// No bits are defined; the caller sets zero.
    pub flags: u32,
    /// Requested capture-pixel rectangle.
    pub region: madopilot_pixel_rect_t,
    /// Whether an out-of-frame rectangle is rejected or clipped.
    pub clip_policy: madopilot_clip_policy_t,
}

/// One grouped OCR operation against one exact retained frame.
///
/// Mandatory prefix: the whole structure. Every handle, string, and zone-array
/// element is borrowed for the synchronous call only.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_ocr_zone_scan_request_t {
    /// `sizeof(madopilot_ocr_zone_scan_request_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// No bits are defined; the caller sets zero.
    pub flags: u32,
    /// The exact retained source frame. Required.
    pub frame: *const madopilot_frame_t,
    /// Model package for an injected backend; null for an integrated profile.
    pub package: *const crate::assets::madopilot_package_t,
    /// Stable package or integrated model identity. Required and non-empty.
    pub model_id: madopilot_str_t,
    /// Stable configured backend identity. Required and non-empty.
    pub backend_id: madopilot_str_t,
    /// Exact configured backend implementation version. Required and non-empty.
    pub backend_version: madopilot_str_t,
    /// Coordinate space of every returned quadrilateral.
    pub output_space: madopilot_space_t,
    /// Reserved; the caller sets zero.
    pub reserved: u32,
    /// Caller-owned array of size-versioned zones.
    pub zones: *const madopilot_ocr_zone_t,
    /// Number of zone elements; must be one through eight.
    pub zone_count: usize,
    /// Byte stride between zone elements.
    pub zone_stride: usize,
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

/// Fixed description of one immutable OCR result.
///
/// Mandatory prefix: the whole structure. Every string view is borrowed from
/// the result handle and becomes invalid at its final release.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_ocr_result_info_t {
    /// `sizeof(madopilot_ocr_result_info_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// No bits are defined; the library writes zero.
    pub flags: u32,
    /// Complete identity of the exact source frame.
    pub source: madopilot_frame_stamp_t,
    /// Effective clipped source region in capture pixels.
    pub effective_region: madopilot_pixel_rect_t,
    /// Coordinate space of every returned quadrilateral.
    pub output_space: madopilot_space_t,
    /// Reserved; the library writes zero.
    pub reserved: u32,
    /// Number of immutable recognized regions.
    pub region_count: u64,
    /// Backend identity that produced the result.
    pub backend_id: madopilot_str_t,
    /// Backend implementation version that produced the result.
    pub backend_version: madopilot_str_t,
    /// Model identity that produced the result.
    pub model_id: madopilot_str_t,
    /// Exact model version that produced the result.
    pub model_version: madopilot_str_t,
    /// Accepted result profile identity.
    pub profile_id: madopilot_str_t,
}

/// Fixed description of one immutable grouped OCR result.
///
/// Mandatory prefix: the whole structure. Every string view is borrowed from
/// the result handle and becomes invalid at its final release.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_ocr_zone_scan_result_info_t {
    /// `sizeof(madopilot_ocr_zone_scan_result_info_t)` as the caller declares it.
    pub struct_size: u32,
    /// No bits are defined; the library writes zero.
    pub flags: u32,
    /// Complete identity of the exact source frame.
    pub source: madopilot_frame_stamp_t,
    /// Smallest mapped source envelope in capture pixels.
    pub source_envelope: madopilot_pixel_rect_t,
    /// Coordinate space of every returned quadrilateral.
    pub output_space: madopilot_space_t,
    /// Number of caller-order zone groups.
    pub zone_count: u64,
    /// Number of immutable candidate payloads stored once.
    pub unique_candidate_count: u64,
    /// Number of group-relative candidate memberships.
    pub membership_count: u64,
    /// Backend identity that produced the result.
    pub backend_id: madopilot_str_t,
    /// Backend implementation version that produced the result.
    pub backend_version: madopilot_str_t,
    /// Model identity that produced the result.
    pub model_id: madopilot_str_t,
    /// Exact model version that produced the result.
    pub model_version: madopilot_str_t,
    /// Accepted result profile identity.
    pub profile_id: madopilot_str_t,
}

/// Effective geometry and membership count of one caller-order zone.
///
/// Mandatory prefix: the whole structure.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_ocr_zone_result_t {
    /// `sizeof(madopilot_ocr_zone_result_t)` as the caller declares it.
    pub struct_size: u32,
    /// No bits are defined; the library writes zero.
    pub flags: u32,
    /// Effective clipped zone in capture pixels.
    pub effective_zone: madopilot_pixel_rect_t,
    /// Reserved; the library writes zero.
    pub reserved: u32,
    /// Number of immutable candidate memberships in this group.
    pub region_count: u64,
}

/// Geometry and confidence of one immutable recognized region.
///
/// Mandatory prefix: the whole structure. Text is read separately through
/// `ocr_result_text_at`, tying that borrowed view to the live result owner.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_ocr_region_t {
    /// `sizeof(madopilot_ocr_region_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// No bits are defined; the library writes zero.
    pub flags: u32,
    /// Profile-defined finite confidence in `0.0..=1.0`.
    pub confidence: f64,
    /// Ordered quadrilateral points in the result's declared coordinate space.
    pub points: [madopilot_ocr_point_t; 4],
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
        AssetFaultKind::UnknownOcrModel => MADOPILOT_ASSET_FAULT_UNKNOWN_OCR_MODEL,
        AssetFaultKind::UnsupportedSource => MADOPILOT_ASSET_FAULT_UNSUPPORTED_SOURCE,
        AssetFaultKind::InvalidTemplateMetadata => MADOPILOT_ASSET_FAULT_INVALID_TEMPLATE_METADATA,
        AssetFaultKind::InvalidOcrModelMetadata => MADOPILOT_ASSET_FAULT_INVALID_OCR_MODEL_METADATA,
        AssetFaultKind::UnsupportedOcrProfile => MADOPILOT_ASSET_FAULT_UNSUPPORTED_OCR_PROFILE,
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
