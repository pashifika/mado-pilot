//! Replay configuration and source failures.

use std::fmt;

use mado_pilot_core::Status;

/// A replay source rule that could not be satisfied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ReplayFault {
    /// The manifest file is missing or could not be read.
    ManifestUnreadable,
    /// The manifest is not valid JSON, or carries a field this build rejects.
    ManifestMalformed,
    /// The manifest declares a schema version this build does not support.
    UnsupportedSchemaVersion,
    /// A frame names a pixel format outside the supported set.
    UnsupportedFormatName,
    /// A frame names a continuity outside the supported set.
    UnsupportedContinuityName,
    /// A frame's extent, format, and stride do not form a valid descriptor.
    FrameDescriptorInvalid,
    /// A frame's pixel file is not the length its descriptor requires.
    FrameBytesMismatch,
    /// A pixel file is missing or could not be read.
    PixelsUnreadable,
    /// A declared pixel path is absolute, rooted, traverses, or is not a regular
    /// file.
    UnsafePixelPath,
    /// A declared target placement is not representable.
    PlacementInvalid,
    /// A target declares no frames.
    EmptySequence,
    /// Two targets share a name.
    DuplicateTargetName,
}

impl ReplayFault {
    /// Returns the public status this fault reports as.
    ///
    /// Everything here is a property of the configured source rather than of the
    /// caller's request, so all of it reports as a capture failure. A caller who
    /// pointed at a broken replay directory has not made an invalid API call.
    #[must_use]
    pub const fn status(self) -> Status {
        Status::CaptureFailed
    }

    pub(crate) const fn detail(self) -> &'static str {
        match self {
            ReplayFault::ManifestUnreadable => "replay manifest could not be read",
            ReplayFault::ManifestMalformed => "replay manifest is malformed",
            ReplayFault::UnsupportedSchemaVersion => {
                "replay manifest declares an unsupported schema version"
            }
            ReplayFault::UnsupportedFormatName => "replay frame names an unsupported pixel format",
            ReplayFault::UnsupportedContinuityName => {
                "replay frame names an unsupported continuity"
            }
            ReplayFault::FrameDescriptorInvalid => "replay frame descriptor is invalid",
            ReplayFault::FrameBytesMismatch => {
                "replay frame pixels do not match the declared descriptor"
            }
            ReplayFault::PixelsUnreadable => "replay frame pixels could not be read",
            ReplayFault::UnsafePixelPath => "replay frame declares an unsafe pixel path",
            ReplayFault::PlacementInvalid => "replay frame declares an invalid target placement",
            ReplayFault::EmptySequence => "replay target declares no frames",
            ReplayFault::DuplicateTargetName => "replay source declares duplicate target names",
        }
    }
}

impl fmt::Display for ReplayFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.detail())
    }
}

impl std::error::Error for ReplayFault {}

#[cfg(test)]
mod tests {
    use super::ReplayFault;
    use mado_pilot_core::{Error, Status};

    #[test]
    fn every_source_fault_reports_as_a_capture_failure() {
        for fault in [
            ReplayFault::ManifestUnreadable,
            ReplayFault::ManifestMalformed,
            ReplayFault::UnsafePixelPath,
            ReplayFault::EmptySequence,
        ] {
            let error: Error = fault.into();
            assert_eq!(error.status(), Status::CaptureFailed, "{fault:?}");
            assert!(!error.detail().is_empty());
        }
    }
}
