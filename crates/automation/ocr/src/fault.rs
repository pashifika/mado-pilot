//! Typed OCR contract failures.

use std::fmt;

use mado_pilot_core::{Error, Status};

/// An OCR rule that could not be satisfied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OcrFault {
    /// A model, profile, or backend identifier was not bounded and canonical.
    InvalidIdentifier,
    /// Profile metadata is incomplete or carries an invalid bounded value.
    InvalidProfileMetadata,
    /// A declaration using the accepted G-004 model or profile ID does not match its authority.
    AcceptedProfileMismatch,
    /// The selected profile uses result-normalization semantics this build does not implement.
    UnsupportedProfile,
    /// A model detector or recognizer component was empty.
    EmptyModelComponent,
    /// A model component exceeded the reviewed 64 MiB ceiling.
    ModelComponentAboveCeiling,
    /// A model component did not have its declared byte length.
    ModelLengthMismatch,
    /// A model component did not have its declared SHA-256 digest.
    ModelDigestMismatch,
    /// The requested backend identity differs from the selected backend.
    BackendMismatch,
    /// The requested model identity differs from the selected backend's model.
    ModelMismatch,
    /// The requested profile identity differs from the selected backend's profile.
    ProfileMismatch,
    /// The backend could not be initialized or is not available.
    BackendUnavailable,
    /// The backend failed after accepting OCR work.
    BackendFailed,
    /// The backend emitted more candidates than the accepted profile permits.
    BackendCandidateCountAboveCeiling,
    /// The backend emitted text that was not valid UTF-8.
    BackendTextNotUtf8,
    /// Backend text exceeded a bounded input or normalized-output length.
    BackendTextAboveCeiling,
    /// The backend emitted non-finite, degenerate, non-convex, or out-of-region geometry.
    BackendGeometryInvalid,
    /// The backend emitted confidence outside the profile's finite range.
    BackendConfidenceOutOfRange,
    /// Two backend candidates used the same detector order.
    BackendOrderDuplicate,
}

impl OcrFault {
    /// Returns the public status this fault reports as.
    #[must_use]
    pub const fn status(self) -> Status {
        match self {
            Self::InvalidIdentifier
            | Self::InvalidProfileMetadata
            | Self::AcceptedProfileMismatch
            | Self::EmptyModelComponent
            | Self::ModelComponentAboveCeiling
            | Self::ModelLengthMismatch
            | Self::ModelDigestMismatch
            | Self::BackendMismatch
            | Self::ModelMismatch
            | Self::ProfileMismatch => Status::InvalidArgument,
            Self::UnsupportedProfile => Status::Unsupported,
            Self::BackendUnavailable
            | Self::BackendFailed
            | Self::BackendCandidateCountAboveCeiling
            | Self::BackendTextNotUtf8
            | Self::BackendTextAboveCeiling
            | Self::BackendGeometryInvalid
            | Self::BackendConfidenceOutOfRange
            | Self::BackendOrderDuplicate => Status::VisionFailed,
        }
    }

    const fn detail(self) -> &'static str {
        match self {
            Self::InvalidIdentifier => {
                "OCR identifier is empty, non-canonical, or above its byte ceiling"
            }
            Self::InvalidProfileMetadata => "OCR profile metadata is incomplete or invalid",
            Self::AcceptedProfileMismatch => {
                "OCR metadata does not match the accepted G-004 profile identity"
            }
            Self::UnsupportedProfile => "OCR profile normalization is not supported",
            Self::EmptyModelComponent => "OCR model component carries no bytes",
            Self::ModelComponentAboveCeiling => "OCR model component exceeds the byte ceiling",
            Self::ModelLengthMismatch => "OCR model component length does not match its identity",
            Self::ModelDigestMismatch => "OCR model component digest does not match its identity",
            Self::BackendMismatch => "OCR request selected a different backend",
            Self::ModelMismatch => "OCR request selected a different model",
            Self::ProfileMismatch => "OCR request selected a different profile",
            Self::BackendUnavailable => "OCR backend is not available",
            Self::BackendFailed => "OCR backend failed",
            Self::BackendCandidateCountAboveCeiling => "OCR backend emitted too many candidates",
            Self::BackendTextNotUtf8 => "OCR backend emitted malformed UTF-8 text",
            Self::BackendTextAboveCeiling => "OCR backend text exceeds the byte ceiling",
            Self::BackendGeometryInvalid => "OCR backend emitted invalid candidate geometry",
            Self::BackendConfidenceOutOfRange => {
                "OCR backend emitted confidence outside zero through one"
            }
            Self::BackendOrderDuplicate => "OCR backend emitted duplicate detector order",
        }
    }
}

impl fmt::Display for OcrFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.detail())
    }
}

impl std::error::Error for OcrFault {}

impl From<OcrFault> for Error {
    fn from(fault: OcrFault) -> Self {
        Self::new(fault.status(), fault.detail())
    }
}

#[cfg(test)]
mod tests {
    use mado_pilot_core::Status;

    use super::OcrFault;

    #[test]
    fn caller_and_backend_faults_keep_distinct_status_classes() {
        assert_eq!(OcrFault::ModelMismatch.status(), Status::InvalidArgument);
        assert_eq!(
            OcrFault::BackendGeometryInvalid.status(),
            Status::VisionFailed
        );
    }
}
