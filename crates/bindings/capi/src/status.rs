//! The C status vocabulary.
//!
//! One value per machine-readable outcome a caller branches on, plus the one
//! outcome that has no Rust counterpart because it is the boundary's own: a
//! contained panic. A caller never has to parse a message to decide what to do.
//!
//! Status values 0 through 12 are frozen by
//! `docs/adr/0007-phase-1-c-abi-freeze.md`. ABI 1.1 appends input status 13
//! under `docs/adr/0017-c-abi-1-1-native-input-prefix.md`. `Status` is
//! `#[non_exhaustive]`, so any later status reports as
//! `MADOPILOT_STATUS_INTERNAL` until an ABI minor gives it a value of its own.
//! Reusing the nearest existing status instead would tell a C caller something
//! specific and wrong.

use mado_pilot::Status;

/// A C status code.
///
/// Signed so that a future negative range stays available for a distinct
/// failure category, and fixed-width so the value does not depend on the enum
/// representation a C compiler chose.
pub type madopilot_status_t = i32;

/// The operation completed and every required output is populated.
pub const MADOPILOT_STATUS_OK: madopilot_status_t = 0;
/// The request was malformed, out of range, or named something unknown.
pub const MADOPILOT_STATUS_INVALID_ARGUMENT: madopilot_status_t = 1;
/// The request is well-formed but this build cannot satisfy it.
pub const MADOPILOT_STATUS_UNSUPPORTED: madopilot_status_t = 2;
/// The operation's cancellation token was set before the result committed.
pub const MADOPILOT_STATUS_CANCELLED: madopilot_status_t = 3;
/// The operation's absolute deadline passed before the result committed.
pub const MADOPILOT_STATUS_DEADLINE_EXCEEDED: madopilot_status_t = 4;
/// The session has entered closing and starts no further work.
pub const MADOPILOT_STATUS_CLOSED: madopilot_status_t = 5;
/// The capture target no longer exists.
pub const MADOPILOT_STATUS_TARGET_LOST: madopilot_status_t = 6;
/// A configured or implementation limit would have been exceeded.
pub const MADOPILOT_STATUS_LIMIT_EXCEEDED: madopilot_status_t = 7;
/// Capture could not produce the requested frame.
pub const MADOPILOT_STATUS_CAPTURE_FAILED: madopilot_status_t = 8;
/// An asset package broke one of the rules that make it trustworthy.
pub const MADOPILOT_STATUS_ASSET_INVALID: madopilot_status_t = 9;
/// The matching backend was unavailable or could not complete the search.
pub const MADOPILOT_STATUS_VISION_FAILED: madopilot_status_t = 10;
/// An invariant this library is responsible for did not hold.
pub const MADOPILOT_STATUS_INTERNAL: madopilot_status_t = 11;
/// A Rust panic was contained at the boundary.
///
/// The C ABI's own status, with no Rust counterpart. No unwind crossed into C,
/// every valid output is in its documented failure state, and handles unrelated
/// to the failed call remain usable.
pub const MADOPILOT_STATUS_INTERNAL_PANIC: madopilot_status_t = 12;
/// Input was refused before admission and no terminal receipt exists.
pub const MADOPILOT_STATUS_INPUT_FAILED: madopilot_status_t = 13;

/// The subsystem a failure came from.
///
/// A caller that reports failures to a human uses this to say *where*; a caller
/// that branches uses the status. Frozen with the rest; it is a second axis,
/// chosen at the call site rather than derived from the status.
pub type madopilot_error_category_t = i32;

/// The failure has no more specific category than its status.
pub const MADOPILOT_ERROR_CATEGORY_UNSPECIFIED: madopilot_error_category_t = 0;
/// The C boundary itself refused the call: a pointer, size, tag, or conversion.
pub const MADOPILOT_ERROR_CATEGORY_ABI: madopilot_error_category_t = 1;
/// The operation's deadline or cancellation ended the call.
pub const MADOPILOT_ERROR_CATEGORY_OPERATION: madopilot_error_category_t = 2;
/// Engine construction or configuration.
pub const MADOPILOT_ERROR_CATEGORY_ENGINE: madopilot_error_category_t = 3;
/// Target discovery, session lifecycle, frames, or mapping.
pub const MADOPILOT_ERROR_CATEGORY_CAPTURE: madopilot_error_category_t = 4;
/// Asset package loading or template resolution.
pub const MADOPILOT_ERROR_CATEGORY_ASSET: madopilot_error_category_t = 5;
/// Template preparation or matching.
pub const MADOPILOT_ERROR_CATEGORY_VISION: madopilot_error_category_t = 6;
/// Coordinate spaces, rectangles, and extents.
pub const MADOPILOT_ERROR_CATEGORY_GEOMETRY: madopilot_error_category_t = 7;
/// Non-prompting permission probes.
pub const MADOPILOT_ERROR_CATEGORY_PERMISSION: madopilot_error_category_t = 8;
/// Input admission or delivery.
pub const MADOPILOT_ERROR_CATEGORY_INPUT: madopilot_error_category_t = 9;

/// Projects a facade status onto its C code.
///
/// [`Status`] is `#[non_exhaustive]`, so a status this build has never seen
/// reports as [`MADOPILOT_STATUS_INTERNAL`] rather than as a number a caller
/// cannot look up. That is the honest answer: the library returned something
/// this ABI major has no vocabulary for.
///
/// `Status::InputFailed` gained its own value in ABI 1.1. Later unknown values
/// continue to use the fallback.
#[must_use]
pub(crate) fn code(status: Status) -> madopilot_status_t {
    match status {
        Status::InvalidArgument => MADOPILOT_STATUS_INVALID_ARGUMENT,
        Status::Unsupported => MADOPILOT_STATUS_UNSUPPORTED,
        Status::Cancelled => MADOPILOT_STATUS_CANCELLED,
        Status::DeadlineExceeded => MADOPILOT_STATUS_DEADLINE_EXCEEDED,
        Status::Closed => MADOPILOT_STATUS_CLOSED,
        Status::TargetLost => MADOPILOT_STATUS_TARGET_LOST,
        Status::LimitExceeded => MADOPILOT_STATUS_LIMIT_EXCEEDED,
        Status::CaptureFailed => MADOPILOT_STATUS_CAPTURE_FAILED,
        Status::AssetInvalid => MADOPILOT_STATUS_ASSET_INVALID,
        Status::VisionFailed => MADOPILOT_STATUS_VISION_FAILED,
        Status::InputFailed => MADOPILOT_STATUS_INPUT_FAILED,
        Status::Internal => MADOPILOT_STATUS_INTERNAL,
        _ => MADOPILOT_STATUS_INTERNAL,
    }
}

/// Returns a stable lowercase slug for `status`.
///
/// Diagnostic text, not a control-flow input: a caller branches on the number.
#[must_use]
pub(crate) const fn text(status: madopilot_status_t) -> &'static str {
    match status {
        MADOPILOT_STATUS_OK => "ok",
        MADOPILOT_STATUS_INVALID_ARGUMENT => "invalid_argument",
        MADOPILOT_STATUS_UNSUPPORTED => "unsupported",
        MADOPILOT_STATUS_CANCELLED => "cancelled",
        MADOPILOT_STATUS_DEADLINE_EXCEEDED => "deadline_exceeded",
        MADOPILOT_STATUS_CLOSED => "closed",
        MADOPILOT_STATUS_TARGET_LOST => "target_lost",
        MADOPILOT_STATUS_LIMIT_EXCEEDED => "limit_exceeded",
        MADOPILOT_STATUS_CAPTURE_FAILED => "capture_failed",
        MADOPILOT_STATUS_ASSET_INVALID => "asset_invalid",
        MADOPILOT_STATUS_VISION_FAILED => "vision_failed",
        MADOPILOT_STATUS_INTERNAL => "internal",
        MADOPILOT_STATUS_INTERNAL_PANIC => "internal_panic",
        MADOPILOT_STATUS_INPUT_FAILED => "input_failed",
        _ => "unrecognized",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_status_code_has_a_slug() {
        for status in MADOPILOT_STATUS_OK..=MADOPILOT_STATUS_INPUT_FAILED {
            assert_ne!(text(status), "unrecognized", "status {status} has no slug");
        }
    }

    #[test]
    fn an_unallocated_code_is_not_claimed() {
        assert_eq!(text(MADOPILOT_STATUS_INPUT_FAILED + 1), "unrecognized");
        assert_eq!(text(-1), "unrecognized");
    }

    #[test]
    fn every_facade_status_maps_to_its_own_code() {
        let mapped = [
            (Status::InvalidArgument, MADOPILOT_STATUS_INVALID_ARGUMENT),
            (Status::Unsupported, MADOPILOT_STATUS_UNSUPPORTED),
            (Status::Cancelled, MADOPILOT_STATUS_CANCELLED),
            (Status::DeadlineExceeded, MADOPILOT_STATUS_DEADLINE_EXCEEDED),
            (Status::Closed, MADOPILOT_STATUS_CLOSED),
            (Status::TargetLost, MADOPILOT_STATUS_TARGET_LOST),
            (Status::LimitExceeded, MADOPILOT_STATUS_LIMIT_EXCEEDED),
            (Status::CaptureFailed, MADOPILOT_STATUS_CAPTURE_FAILED),
            (Status::AssetInvalid, MADOPILOT_STATUS_ASSET_INVALID),
            (Status::VisionFailed, MADOPILOT_STATUS_VISION_FAILED),
            (Status::InputFailed, MADOPILOT_STATUS_INPUT_FAILED),
            (Status::Internal, MADOPILOT_STATUS_INTERNAL),
        ];

        for (status, expected) in mapped {
            assert_eq!(code(status), expected, "{status} maps to {expected}");
        }
    }

    #[test]
    fn input_failed_has_the_appended_abi_1_1_code() {
        assert_eq!(code(Status::InputFailed), MADOPILOT_STATUS_INPUT_FAILED);
    }
}
