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

/// How an input event reaches its target.
pub type madopilot_input_delivery_t = i32;

/// No delivery mechanism was selected.
pub const MADOPILOT_INPUT_DELIVERY_NONE: madopilot_input_delivery_t = 0;
/// The operating system's system-input path.
pub const MADOPILOT_INPUT_DELIVERY_SYSTEM: madopilot_input_delivery_t = 1;
/// Delivery addressed to the target without activating it.
pub const MADOPILOT_INPUT_DELIVERY_BACKGROUND_TARGET: madopilot_input_delivery_t = 2;

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

/// How far an admitted sequence got.
pub type madopilot_sequence_outcome_t = i32;

/// No event was delivered.
pub const MADOPILOT_SEQUENCE_UNEXECUTED: madopilot_sequence_outcome_t = 0;
/// Every event was delivered.
pub const MADOPILOT_SEQUENCE_COMPLETE: madopilot_sequence_outcome_t = 1;
/// Some input may have reached the target before the sequence stopped.
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
/// The requested operation and delivery combination is unavailable.
pub const MADOPILOT_INPUT_FAULT_UNSUPPORTED_COMBINATION: madopilot_input_fault_t = 5;
/// The delivery plan is empty or repeats a mechanism.
pub const MADOPILOT_INPUT_FAULT_INVALID_DELIVERY_PLAN: madopilot_input_fault_t = 6;
/// Every permitted delivery mechanism was unavailable.
pub const MADOPILOT_INPUT_FAULT_DELIVERY_UNAVAILABLE: madopilot_input_fault_t = 7;
/// The sequence or one of its events exceeds a bound.
pub const MADOPILOT_INPUT_FAULT_SEQUENCE_OUT_OF_BOUNDS: madopilot_input_fault_t = 8;
/// The target does not accept the pointer coordinate space.
pub const MADOPILOT_INPUT_FAULT_UNSUPPORTED_COORDINATE: madopilot_input_fault_t = 9;
/// The geometry policy requires a source frame.
pub const MADOPILOT_INPUT_FAULT_MISSING_COORDINATE_SOURCE: madopilot_input_fault_t = 10;
/// Target geometry changed since the source frame.
pub const MADOPILOT_INPUT_FAULT_GEOMETRY_CHANGED: madopilot_input_fault_t = 11;
/// Delivery needs focus that policy withholds.
pub const MADOPILOT_INPUT_FAULT_FOCUS_REQUIRED: madopilot_input_fault_t = 12;
/// The operating system refused focus.
pub const MADOPILOT_INPUT_FAULT_FOCUS_REFUSED: madopilot_input_fault_t = 13;
/// Input control is not authorized.
pub const MADOPILOT_INPUT_FAULT_NOT_AUTHORIZED: madopilot_input_fault_t = 14;
/// Operating-system policy refused delivery.
pub const MADOPILOT_INPUT_FAULT_POLICY_REFUSED: madopilot_input_fault_t = 15;
/// The input controller is closed.
pub const MADOPILOT_INPUT_FAULT_CONTROLLER_CLOSED: madopilot_input_fault_t = 16;
/// The operation was cancelled.
pub const MADOPILOT_INPUT_FAULT_CANCELLED: madopilot_input_fault_t = 17;
/// The operation deadline passed.
pub const MADOPILOT_INPUT_FAULT_DEADLINE_EXCEEDED: madopilot_input_fault_t = 18;
/// The platform reported another delivery failure.
pub const MADOPILOT_INPUT_FAULT_DELIVERY_FAILED: madopilot_input_fault_t = 19;

/// Pointer/system capability pair.
pub const MADOPILOT_INPUT_PAIR_POINTER_SYSTEM: u64 = 1 << 0;
/// Pointer/background capability pair.
pub const MADOPILOT_INPUT_PAIR_POINTER_BACKGROUND: u64 = 1 << 1;
/// Keyboard/system capability pair.
pub const MADOPILOT_INPUT_PAIR_KEYBOARD_SYSTEM: u64 = 1 << 2;
/// Keyboard/background capability pair.
pub const MADOPILOT_INPUT_PAIR_KEYBOARD_BACKGROUND: u64 = 1 << 3;
/// Text/system capability pair.
pub const MADOPILOT_INPUT_PAIR_TEXT_SYSTEM: u64 = 1 << 4;
/// Text/background capability pair.
pub const MADOPILOT_INPUT_PAIR_TEXT_BACKGROUND: u64 = 1 << 5;
/// Every input capability pair ABI 1.1 knows.
pub const MADOPILOT_INPUT_PAIRS_ALL: u64 = (1 << 6) - 1;

/// System delivery requires focus.
pub const MADOPILOT_INPUT_FOCUS_SYSTEM: u32 = 1 << 0;
/// Background delivery requires focus.
pub const MADOPILOT_INPUT_FOCUS_BACKGROUND: u32 = 1 << 1;
/// Every focus-requirement bit ABI 1.1 knows.
pub const MADOPILOT_INPUT_FOCUS_ALL: u32 = (1 << 2) - 1;

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
/// `madopilot_target_t.kind` is populated.
pub const MADOPILOT_TARGET_HAS_KIND: u32 = 1 << 1;
/// `madopilot_target_t.capture_permission` is populated.
pub const MADOPILOT_TARGET_HAS_CAPTURE_PERMISSION: u32 = 1 << 2;

/// The engine can deliver input to at least some targets.
pub const MADOPILOT_ENGINE_DELIVERS_INPUT: u32 = 1 << 0;
/// The engine can run non-prompting permission probes.
pub const MADOPILOT_ENGINE_READS_PERMISSIONS: u32 = 1 << 1;

/// `madopilot_permission_t` carries a redacted diagnostic.
pub const MADOPILOT_PERMISSION_HAS_DIAGNOSTIC: u32 = 1 << 0;
/// `madopilot_permission_t` carries a platform code and namespace.
pub const MADOPILOT_PERMISSION_HAS_PLATFORM_CODE: u32 = 1 << 1;

/// `madopilot_target_capability_t.kind` is populated.
pub const MADOPILOT_TARGET_CAPABILITY_HAS_KIND: u32 = 1 << 0;
/// `madopilot_target_capability_t.capture_permission` is populated.
pub const MADOPILOT_TARGET_CAPABILITY_HAS_CAPTURE_PERMISSION: u32 = 1 << 1;
/// `madopilot_target_capability_t.input_permission` is populated.
pub const MADOPILOT_TARGET_CAPABILITY_HAS_INPUT_PERMISSION: u32 = 1 << 2;

/// `madopilot_input_descriptor_t.permission` is populated.
pub const MADOPILOT_INPUT_DESCRIPTOR_HAS_PERMISSION: u32 = 1 << 0;

/// `madopilot_input_request_t` supplies explicit cleanup bounds.
pub const MADOPILOT_INPUT_REQUEST_HAS_CLEANUP_BUDGET: u32 = 1 << 0;

/// `madopilot_input_receipt_t.target` is populated.
pub const MADOPILOT_INPUT_RECEIPT_HAS_TARGET: u32 = 1 << 0;
/// `madopilot_input_receipt_t.delivery` is populated.
pub const MADOPILOT_INPUT_RECEIPT_HAS_DELIVERY: u32 = 1 << 1;
/// `madopilot_input_receipt_t.last_completed` is populated.
pub const MADOPILOT_INPUT_RECEIPT_HAS_LAST_COMPLETED: u32 = 1 << 2;
/// `madopilot_input_receipt_t.failure` is populated.
pub const MADOPILOT_INPUT_RECEIPT_HAS_FAILURE: u32 = 1 << 3;
/// The selected delivery differed from the first requested delivery.
pub const MADOPILOT_INPUT_RECEIPT_USED_FALLBACK: u32 = 1 << 4;

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
/// Capability flags that apply to the whole engine.
///
/// Mandatory prefix: the whole structure.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_engine_capabilities_t {
    /// `sizeof(madopilot_engine_capabilities_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// [`MADOPILOT_ENGINE_DELIVERS_INPUT`] and
    /// [`MADOPILOT_ENGINE_READS_PERMISSIONS`].
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

/// Capability data for one target.
///
/// Mandatory prefix: through `capture`. The target identity is in the same
/// engine-local domain as `madopilot_target_t.target`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_target_capability_t {
    /// `sizeof(madopilot_target_capability_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// `MADOPILOT_TARGET_CAPABILITY_HAS_*` presence bits.
    pub flags: u32,
    /// Engine-local target identity.
    pub target: u64,
    /// Target kind, when present.
    pub kind: madopilot_target_kind_t,
    /// Whether capture can be attempted.
    pub capture: madopilot_capability_support_t,
    /// Permission required for capture, when present.
    pub capture_permission: madopilot_permission_kind_t,
    /// Reserved; written as zero.
    pub reserved: u32,
    /// Supported operation/delivery pairs.
    pub input_pairs: u64,
    /// A `MADOPILOT_INPUT_FOCUS_*` bit for each delivery that requires focus.
    pub focus_required: u32,
    /// A bit set: bit `1 << space` is set for accepted pointer spaces.
    pub pointer_spaces: u32,
    /// Permission required for input, when present.
    pub input_permission: madopilot_permission_kind_t,
    /// Reserved; written as zero.
    pub reserved2: u32,
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
    /// Operation/delivery pairs that must be accepted.
    pub required_pairs: u64,
    /// Additional operation/delivery pairs to request.
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
    /// [`MADOPILOT_INPUT_DESCRIPTOR_HAS_PERMISSION`].
    pub flags: u32,
    /// Engine-local target identity, or zero for an engine-wide descriptor.
    pub target: u64,
    /// Accepted operation/delivery pairs.
    pub pairs: u64,
    /// A `MADOPILOT_INPUT_FOCUS_*` bit for each delivery that requires focus.
    pub focus_required: u32,
    /// A bit set: bit `1 << space` is set for accepted pointer spaces.
    pub pointer_spaces: u32,
    /// Permission required for input, when present.
    pub permission: madopilot_permission_kind_t,
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
    /// Ordered caller-owned array of requested delivery mechanisms.
    pub deliveries: *const madopilot_input_delivery_t,
    /// Number of delivery mechanisms.
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

/// The one terminal outcome of an admitted input sequence.
///
/// Mandatory prefix: the whole structure. Presence flags distinguish optional
/// scalar values from valid zero values.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_input_receipt_t {
    /// `sizeof(madopilot_input_receipt_t)` as the caller's header declares it.
    pub struct_size: u32,
    /// `MADOPILOT_INPUT_RECEIPT_HAS_*` and
    /// [`MADOPILOT_INPUT_RECEIPT_USED_FALLBACK`].
    pub flags: u32,
    /// Engine-local target identity, when present.
    pub target: u64,
    /// Complete, partial, or unexecuted.
    pub outcome: madopilot_sequence_outcome_t,
    /// Selected delivery mechanism, when present.
    pub delivery: madopilot_input_delivery_t,
    /// Number of delivery mechanisms attempted.
    pub attempted_count: u32,
    /// First attempted delivery, or `MADOPILOT_INPUT_DELIVERY_NONE`.
    pub attempted_first: madopilot_input_delivery_t,
    /// Second attempted delivery, or `MADOPILOT_INPUT_DELIVERY_NONE`.
    pub attempted_second: madopilot_input_delivery_t,
    /// Number of complete logical events delivered.
    pub delivered: u32,
    /// Last complete event index, when present.
    pub last_completed: u32,
    /// Typed input fault, when present.
    pub failure: madopilot_input_fault_t,
    /// Cleanup outcome.
    pub cleanup: madopilot_cleanup_state_t,
    /// Number of cleanup releases completed.
    pub cleanup_released: u32,
    /// Number of cleanup releases still owed.
    pub cleanup_owed: u32,
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
    /// A boundary identity copied from the discovery snapshot.
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
