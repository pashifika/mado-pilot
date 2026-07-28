//! Capture rules that could not be satisfied.

use std::fmt;

use mado_pilot_core::{Error, Status};

/// A capture rule that could not be satisfied.
///
/// Typed so a test — and a caller — can assert which rule was broken rather than
/// reading a message. It converts into the public [`Error`] at the package
/// boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CaptureFault {
    /// A frame or mapping descriptor is internally inconsistent: a row stride
    /// shorter than one row, or a byte length that does not follow from the
    /// extent and stride.
    InconsistentDescriptor,
    /// Pixel bytes do not match the length the descriptor requires.
    ByteLengthMismatch,
    /// The requested pixel format is not one this package can produce.
    UnsupportedFormat,
    /// The frame has no authoritative mapping for the requested coordinate space.
    UnsupportedCoordinate,
    /// A region fell outside the frame it was taken from.
    RegionOutsideFrame,
    /// The target identity was issued by another engine or another provider.
    ForeignTarget,
    /// No target with that identity is known to this provider.
    UnknownTarget,
    /// The session is closed or closing, so it accepts no new frame work.
    SessionClosed,
    /// A required capture option cannot be honored by this provider.
    UnsupportedOption,
    /// The configured source is malformed or unreadable.
    SourceInvalid,
    /// A frame request named a stamp from a different stream.
    ForeignStream,
    /// The stream produced no further frames and never will.
    StreamEnded,
}

impl CaptureFault {
    /// Returns the public status this fault reports as.
    #[must_use]
    pub const fn status(self) -> Status {
        match self {
            CaptureFault::InconsistentDescriptor
            | CaptureFault::ByteLengthMismatch
            | CaptureFault::RegionOutsideFrame
            | CaptureFault::ForeignTarget
            | CaptureFault::UnknownTarget
            | CaptureFault::ForeignStream => Status::InvalidArgument,
            CaptureFault::UnsupportedFormat
            | CaptureFault::UnsupportedCoordinate
            | CaptureFault::UnsupportedOption => Status::Unsupported,
            CaptureFault::SessionClosed | CaptureFault::StreamEnded => Status::Closed,
            CaptureFault::SourceInvalid => Status::CaptureFailed,
        }
    }

    const fn detail(self) -> &'static str {
        match self {
            CaptureFault::InconsistentDescriptor => "frame descriptor is internally inconsistent",
            CaptureFault::ByteLengthMismatch => "pixel bytes do not match the descriptor length",
            CaptureFault::UnsupportedFormat => "requested pixel format is not supported",
            CaptureFault::UnsupportedCoordinate => {
                "frame transform does not support the requested coordinate space"
            }
            CaptureFault::RegionOutsideFrame => "region falls outside its source frame",
            CaptureFault::ForeignTarget => "target identity was not issued by this provider",
            CaptureFault::UnknownTarget => "no such target",
            CaptureFault::SessionClosed => "session is closed",
            CaptureFault::UnsupportedOption => "required capture option is not supported",
            CaptureFault::SourceInvalid => "configured capture source is invalid",
            CaptureFault::ForeignStream => "frame stamp belongs to another stream",
            CaptureFault::StreamEnded => "stream published its final frame",
        }
    }
}

impl fmt::Display for CaptureFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.detail())
    }
}

impl std::error::Error for CaptureFault {}

impl From<CaptureFault> for Error {
    fn from(fault: CaptureFault) -> Self {
        Error::new(fault.status(), fault.detail())
    }
}

#[cfg(test)]
mod tests {
    use super::CaptureFault;
    use mado_pilot_core::{Error, Status};

    #[test]
    fn faults_map_to_public_statuses() {
        assert_eq!(
            CaptureFault::ByteLengthMismatch.status(),
            Status::InvalidArgument
        );
        assert_eq!(
            CaptureFault::UnsupportedFormat.status(),
            Status::Unsupported
        );
        assert_eq!(
            CaptureFault::UnsupportedCoordinate.status(),
            Status::Unsupported
        );
        assert_eq!(CaptureFault::SessionClosed.status(), Status::Closed);
        assert_eq!(CaptureFault::SourceInvalid.status(), Status::CaptureFailed);
    }

    #[test]
    fn a_fault_converts_into_the_public_error() {
        let error: Error = CaptureFault::RegionOutsideFrame.into();

        assert_eq!(error.status(), Status::InvalidArgument);
        assert!(!error.detail().is_empty());
    }
}
