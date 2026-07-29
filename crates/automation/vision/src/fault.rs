//! Vision rules that could not be satisfied.

use std::fmt;

use mado_pilot_core::{Error, Status};

/// A vision rule that could not be satisfied.
///
/// Typed so a test — and a caller — can assert which rule was broken rather than
/// reading a message. It converts into the public [`Error`] at the package
/// boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum VisionFault {
    /// A template identity was empty, so two templates could not be told apart.
    EmptyTemplateId,
    /// A template carried no content bytes.
    EmptyTemplateContent,
    /// A template declared a zero width or height.
    EmptyTemplateExtent,
    /// A template declared more pixels than [`TemplateSource::MAX_PIXELS`].
    ///
    /// A declared extent is metadata, and a backend allocates a decoded image
    /// from it, so an extent no capture could produce is refused where it is
    /// declared rather than where it would be decoded. Reported as an invalid
    /// argument, like the zero extent above it: both are the same field carrying
    /// a value this build does not accept.
    ///
    /// [`TemplateSource::MAX_PIXELS`]: crate::TemplateSource::MAX_PIXELS
    TemplateExtentAboveCeiling,
    /// A template declared its geometry in a coordinate space this version does
    /// not accept. A template is a patch of captured pixels, so version one
    /// accepts [`CapturePixels`] only.
    ///
    /// [`CapturePixels`]: mado_pilot_core::CoordinateSpace::CapturePixels
    UnsupportedTemplateSpace,
    /// A minimum match score was not a finite value inside `0.0..=1.0`.
    InvalidMatchScore,
    /// A match result limit was zero, which asks for no results at all.
    InvalidMatchResultLimit,
    /// No backend in this build can decode the template's content encoding.
    UnsupportedTemplateEncoding,
    /// The backend could not compile the template into its own representation.
    TemplatePreparationFailed,
    /// A prepared template was submitted to a backend that did not prepare it.
    ///
    /// Compiled template state is backend-private, so using it elsewhere is a
    /// caller mistake caught before any native value is touched.
    BackendMismatch,
    /// The backend could not be loaded or initialized.
    BackendUnavailable,
    /// The backend failed while executing a request it had accepted.
    BackendFailed,
    /// The backend reported a score that is not a finite value inside
    /// `0.0..=1.0`.
    ///
    /// This is a defect in the backend rather than a property of the request:
    /// the public score range is the contract every backend is normalized to.
    BackendScoreOutOfRange,
    /// The backend reported a candidate that does not lie inside the search
    /// region it was given.
    BackendCandidateOutsideRegion,
}

impl VisionFault {
    /// Returns the public status this fault reports as.
    #[must_use]
    pub const fn status(self) -> Status {
        match self {
            VisionFault::EmptyTemplateId
            | VisionFault::EmptyTemplateContent
            | VisionFault::EmptyTemplateExtent
            | VisionFault::TemplateExtentAboveCeiling
            | VisionFault::InvalidMatchScore
            | VisionFault::InvalidMatchResultLimit
            | VisionFault::BackendMismatch => Status::InvalidArgument,
            VisionFault::UnsupportedTemplateSpace | VisionFault::UnsupportedTemplateEncoding => {
                Status::Unsupported
            }
            VisionFault::TemplatePreparationFailed
            | VisionFault::BackendUnavailable
            | VisionFault::BackendFailed
            | VisionFault::BackendScoreOutOfRange
            | VisionFault::BackendCandidateOutsideRegion => Status::VisionFailed,
        }
    }

    const fn detail(self) -> &'static str {
        match self {
            VisionFault::EmptyTemplateId => "template identity is empty",
            VisionFault::EmptyTemplateContent => "template carries no content bytes",
            VisionFault::EmptyTemplateExtent => "template extent has a zero dimension",
            VisionFault::TemplateExtentAboveCeiling => {
                "template extent is above the implementation pixel ceiling"
            }
            VisionFault::UnsupportedTemplateSpace => {
                "template geometry must be expressed in capture pixels"
            }
            VisionFault::InvalidMatchScore => "minimum match score is outside 0.0..=1.0",
            VisionFault::InvalidMatchResultLimit => "match result limit is zero",
            VisionFault::UnsupportedTemplateEncoding => {
                "template content encoding is not supported by this backend"
            }
            VisionFault::TemplatePreparationFailed => "backend could not prepare the template",
            VisionFault::BackendMismatch => "prepared template belongs to another backend",
            VisionFault::BackendUnavailable => "matching backend is not available",
            VisionFault::BackendFailed => "matching backend failed",
            VisionFault::BackendScoreOutOfRange => "backend reported a score outside 0.0..=1.0",
            VisionFault::BackendCandidateOutsideRegion => {
                "backend reported a candidate outside the searched region"
            }
        }
    }
}

impl fmt::Display for VisionFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.detail())
    }
}

impl std::error::Error for VisionFault {}

impl From<VisionFault> for Error {
    fn from(fault: VisionFault) -> Self {
        Error::new(fault.status(), fault.detail())
    }
}

#[cfg(test)]
mod tests {
    use super::VisionFault;
    use mado_pilot_core::{Error, Status};

    #[test]
    fn faults_map_to_public_statuses() {
        assert_eq!(
            VisionFault::EmptyTemplateId.status(),
            Status::InvalidArgument
        );
        assert_eq!(
            VisionFault::UnsupportedTemplateSpace.status(),
            Status::Unsupported
        );
    }

    #[test]
    fn a_fault_converts_into_the_public_error() {
        let error: Error = VisionFault::InvalidMatchScore.into();

        assert_eq!(error.status(), Status::InvalidArgument);
        assert!(!error.detail().is_empty());
    }
}
