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
}

impl VisionFault {
    /// Returns the public status this fault reports as.
    #[must_use]
    pub const fn status(self) -> Status {
        match self {
            VisionFault::EmptyTemplateId
            | VisionFault::EmptyTemplateContent
            | VisionFault::EmptyTemplateExtent
            | VisionFault::InvalidMatchScore
            | VisionFault::InvalidMatchResultLimit => Status::InvalidArgument,
            VisionFault::UnsupportedTemplateSpace => Status::Unsupported,
        }
    }

    const fn detail(self) -> &'static str {
        match self {
            VisionFault::EmptyTemplateId => "template identity is empty",
            VisionFault::EmptyTemplateContent => "template carries no content bytes",
            VisionFault::EmptyTemplateExtent => "template extent has a zero dimension",
            VisionFault::UnsupportedTemplateSpace => {
                "template geometry must be expressed in capture pixels"
            }
            VisionFault::InvalidMatchScore => "minimum match score is outside 0.0..=1.0",
            VisionFault::InvalidMatchResultLimit => "match result limit is zero",
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
