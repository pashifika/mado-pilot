//! Stable public outcome statuses and the structured error that carries them.
//!
//! A caller distinguishes failures by matching [`Status`], never by reading
//! [`Error::detail`]. Detail text is diagnostic: it may be reworded, and it is
//! not part of any compatibility promise.

use std::borrow::Cow;
use std::fmt;

/// Programmatic failure category shared by every MadoPilot package.
///
/// The variants separate the distinctions a caller must be able to act on: bad
/// input, an unsupported request, the two terminal interruptions, lifecycle
/// loss, a limit, a failure attributable to one responsibility, and an internal
/// defect.
///
/// This enum is `#[non_exhaustive]`: later phases add responsibilities, and a
/// caller must keep a fallback arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Status {
    /// The request was malformed, out of range, or not accepted by the
    /// receiving engine.
    InvalidArgument,
    /// The request was well formed, but the capability or conversion it needs
    /// is not available.
    Unsupported,
    /// The operation's cancellation token was set before the result committed.
    Cancelled,
    /// The operation's absolute deadline passed before the result committed.
    DeadlineExceeded,
    /// The session, stream, or handle was closed.
    Closed,
    /// The capture target no longer exists.
    TargetLost,
    /// A documented resource ceiling would have been exceeded.
    LimitExceeded,
    /// Capture failed for a reason the capture responsibility owns.
    CaptureFailed,
    /// An asset package failed validation.
    AssetInvalid,
    /// A vision backend failed or was unavailable.
    VisionFailed,
    /// An invariant this implementation is responsible for did not hold.
    Internal,
}

impl Status {
    /// Returns a stable lowercase slug, for logs and for the C ABI mapping.
    ///
    /// The slug is part of the diagnostic surface, not the matching surface:
    /// match on the variant rather than on this string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Status::InvalidArgument => "invalid_argument",
            Status::Unsupported => "unsupported",
            Status::Cancelled => "cancelled",
            Status::DeadlineExceeded => "deadline_exceeded",
            Status::Closed => "closed",
            Status::TargetLost => "target_lost",
            Status::LimitExceeded => "limit_exceeded",
            Status::CaptureFailed => "capture_failed",
            Status::AssetInvalid => "asset_invalid",
            Status::VisionFailed => "vision_failed",
            Status::Internal => "internal",
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A public failure: one [`Status`] and diagnostic detail.
///
/// The detail must never carry captured pixels, recognized text, credentials,
/// file content, or any other sensitive payload. It exists so a human can tell
/// two failures of the same status apart, and callers never parse it.
///
/// There is deliberately no error `source` chain. A source would let a platform
/// or backend type escape through a platform-neutral contract, which is exactly
/// what this package exists to prevent; an adapter translates its own failure
/// into a status and a detail string instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    status: Status,
    detail: Cow<'static, str>,
}

impl Error {
    /// Builds an error with `status` and diagnostic `detail`.
    #[must_use]
    pub fn new(status: Status, detail: impl Into<Cow<'static, str>>) -> Self {
        Self {
            status,
            detail: detail.into(),
        }
    }

    /// Returns the programmatic status a caller matches on.
    #[must_use]
    pub const fn status(&self) -> Status {
        self.status
    }

    /// Returns the diagnostic detail.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.detail.is_empty() {
            formatter.write_str(self.status.as_str())
        } else {
            write!(formatter, "{}: {}", self.status.as_str(), self.detail)
        }
    }
}

impl std::error::Error for Error {}

/// Result specialized to the shared [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::{Error, Status};

    #[test]
    fn status_matching_does_not_depend_on_detail_text() {
        let first = Error::new(
            Status::Unsupported,
            "conversion has no authoritative mapping",
        );
        let second = Error::new(Status::Unsupported, "backend is not loaded");

        assert_eq!(first.status(), second.status());
        assert_ne!(first.detail(), second.detail());
    }

    #[test]
    fn display_includes_the_status_slug() {
        let error = Error::new(Status::DeadlineExceeded, "expired while hashing");

        assert_eq!(
            error.to_string(),
            "deadline_exceeded: expired while hashing"
        );
    }

    #[test]
    fn display_omits_an_empty_detail_separator() {
        assert_eq!(Error::new(Status::Closed, "").to_string(), "closed");
    }

    #[test]
    fn every_status_slug_is_distinct() {
        let statuses = [
            Status::InvalidArgument,
            Status::Unsupported,
            Status::Cancelled,
            Status::DeadlineExceeded,
            Status::Closed,
            Status::TargetLost,
            Status::LimitExceeded,
            Status::CaptureFailed,
            Status::AssetInvalid,
            Status::VisionFailed,
            Status::Internal,
        ];
        let mut slugs: Vec<&str> = statuses.iter().map(|status| status.as_str()).collect();
        slugs.sort_unstable();
        let total = slugs.len();
        slugs.dedup();

        assert_eq!(slugs.len(), total);
    }
}
