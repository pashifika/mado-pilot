//! The OpenCV CPU matching backend.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use mado_pilot_core::{Error, OperationContext, PixelExtent, Result};
use mado_pilot_vision::{
    BackendDescriptor, BackendId, BackendRequest, Candidate, MatchBackend, PreparedTemplate,
    TemplateEncoding, TemplateSource, VisionFault,
};
use opencv::core::{Mat, MatTraitConst, MatTraitConstManual};
use opencv::imgproc::{TM_CCOEFF_NORMED, match_template_def};

use crate::candidates::{PeakSearch, peaks};
use crate::image::{self, ImageFault, REQUIRED_FORMAT};

/// The backend's stable public identifier.
///
/// Exposed as a constant so a caller composing a required-backend policy can
/// name this backend before it has been constructed — an unavailable backend has
/// no instance to ask.
pub const BACKEND_ID: &str = "opencv-cpu";

/// The OpenCV major version this adapter was verified against.
const SUPPORTED_MAJOR: i32 = 4;

/// A template decoded into OpenCV's own representation.
///
/// The matrix never leaves this type, and this type never leaves the adapter: it
/// travels inside a [`PreparedTemplate`]'s opaque payload, which the matcher
/// attributes by [`BackendId`] before anything downcasts it.
struct TemplateMatrix {
    image: Mat,
    extent: PixelExtent,
}

impl mado_pilot_vision::TemplatePayload for TemplateMatrix {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl fmt::Debug for TemplateMatrix {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The decoded matrix is pixels, and ordinary diagnostics exclude those.
        formatter
            .debug_struct("TemplateMatrix")
            .field("extent", &self.extent)
            .finish_non_exhaustive()
    }
}

/// Template matching on the CPU through OpenCV's `matchTemplate`.
///
/// The Phase 1 profile is normalized correlation coefficient over three-channel
/// BGR, with the negative half of the correlation range clamped to "no match".
/// Both the profile and the clamp are decisions with recorded reasons; see
/// `docs/adr/0003-opencv-matching-profile-and-public-score.md`.
///
/// # What it decides, and what it does not
///
/// This adapter decodes a template, converts a searched region, correlates the
/// two, and extracts a bounded canonical candidate prefix. It mirrors the
/// request's suppression only to avoid materializing a dense correlation map;
/// the public matcher remains authoritative and reapplies score validation,
/// thresholding, canonical ordering, suppression, and the result limit.
///
/// # Availability
///
/// [`OpenCvBackend::new`] probes the linked library's runtime version and refuses
/// a major version this adapter was not verified against. It cannot report a
/// *missing* library as a status: OpenCV is linked dynamically at load time, so
/// an absent library stops the process before any MadoPilot code runs. Turning
/// that into an actionable status needs deferred loading, which belongs with gate
/// `G-007`'s controlled library search paths.
#[derive(Debug, Clone)]
pub struct OpenCvBackend {
    descriptor: BackendDescriptor,
}

impl OpenCvBackend {
    /// Builds the backend after confirming the linked OpenCV is usable.
    ///
    /// The version the probe reads becomes the descriptor's version, so a score
    /// a result carries is attributable to the library that produced it rather
    /// than to whatever OpenCV the headers described at build time.
    ///
    /// # Errors
    ///
    /// Returns [`VisionFault::BackendUnavailable`] when the runtime version
    /// cannot be read or is not a version this adapter supports.
    pub fn new() -> Result<Self> {
        let version = opencv::core::get_version_string()
            .map_err(|_| Error::from(VisionFault::BackendUnavailable))?;
        let version = accept_version(opencv::core::get_version_major(), &version)?;

        Ok(Self {
            descriptor: BackendDescriptor::new(BACKEND_ID, version, REQUIRED_FORMAT),
        })
    }
}

impl MatchBackend for OpenCvBackend {
    fn descriptor(&self) -> BackendDescriptor {
        self.descriptor.clone()
    }

    fn prepare(
        &self,
        source: &TemplateSource,
        operation: &OperationContext,
    ) -> Result<PreparedTemplate> {
        checkpoint(operation)?;

        match source.encoding() {
            TemplateEncoding::Png => {}
            // `TemplateEncoding` is `#[non_exhaustive]`, and a later encoding is
            // unsupported here until this adapter is shown to decode it.
            _ => return Err(VisionFault::UnsupportedTemplateEncoding.into()),
        }

        let image = image::decode_to_bgr(source.content())
            .map_err(|_| Error::from(VisionFault::TemplatePreparationFailed))?;
        let extent = image::extent_of(&image)
            .map_err(|_| Error::from(VisionFault::TemplatePreparationFailed))?;

        // A template whose bytes decode to other dimensions than its metadata
        // declares would put every match it produced at the wrong extent, so the
        // disagreement is reported instead of resolved in the adapter's favour.
        if extent != source.extent() {
            return Err(VisionFault::TemplatePreparationFailed.into());
        }
        checkpoint(operation)?;

        Ok(PreparedTemplate::new(
            BackendId::new(BACKEND_ID),
            source,
            Arc::new(TemplateMatrix { image, extent }),
        ))
    }

    fn find(
        &self,
        request: &BackendRequest<'_>,
        operation: &OperationContext,
    ) -> Result<Vec<Candidate>> {
        checkpoint(operation)?;

        let template = request
            .template
            .payload()
            .as_any()
            .downcast_ref::<TemplateMatrix>()
            .ok_or(VisionFault::BackendMismatch)?;
        if template.extent != request.template.extent() {
            // The prepared template's public extent bounds every candidate the
            // matcher accepts, so it must be the extent that was correlated.
            return Err(VisionFault::BackendFailed.into());
        }

        let region = image::region_to_bgr(request.pixels).map_err(backend_failure)?;
        checkpoint(operation)?;

        let mut scores = Mat::default();
        match_template_def(&region, &template.image, &mut scores, TM_CCOEFF_NORMED)
            .map_err(|_| Error::from(VisionFault::BackendFailed))?;
        checkpoint(operation)?;

        let width = usize::try_from(scores.cols()).map_err(|_| unrepresentable())?;
        let height = usize::try_from(scores.rows()).map_err(|_| unrepresentable())?;
        let values = scores
            .data_typed::<f32>()
            .map_err(|_| Error::from(VisionFault::BackendFailed))?;

        let search = PeakSearch {
            width,
            height,
            template_width: usize::try_from(template.extent.width())
                .map_err(|_| unrepresentable())?,
            template_height: usize::try_from(template.extent.height())
                .map_err(|_| unrepresentable())?,
            min_score: request.options.min_score(),
            candidate_budget: usize::try_from(request.options.max_results()).unwrap_or(usize::MAX),
            suppression: request.options.suppression(),
        };

        // Extraction scans the score map once per candidate and the budget above
        // is the caller's own `u32` result limit, so this is the one stage of a
        // search whose length the caller sets. It reads the context as it goes
        // rather than at its end.
        peaks(values, search, operation)?
            .into_iter()
            .map(|peak| {
                let left = i32::try_from(peak.left).map_err(|_| unrepresentable())?;
                let top = i32::try_from(peak.top).map_err(|_| unrepresentable())?;
                Ok(Candidate::new(left, top, peak.score))
            })
            .collect()
    }
}

/// Decides whether a runtime OpenCV version is one this adapter supports.
///
/// Phase 1 accepts major version four because that is what both release targets
/// were verified against. A different major version reports unavailable rather
/// than matching with behaviour no fixture has covered.
fn accept_version(major: i32, version: &str) -> Result<Arc<str>> {
    if major != SUPPORTED_MAJOR || version.trim().is_empty() {
        return Err(VisionFault::BackendUnavailable.into());
    }

    Ok(Arc::from(version.trim()))
}

/// Returns the operation's terminal outcome when cancellation or the deadline
/// has already won.
///
/// OpenCV's own calls are uninterruptible, so the adapter checks between them:
/// before decoding, before converting, before correlating, and before extracting
/// candidates. A call that finishes after an outcome has been decided still
/// returns here, and the check that follows it discards its output.
///
/// Candidate extraction is the exception that checks *within* itself, because it
/// is MadoPilot's own arithmetic rather than a library call and its length is set
/// by the caller's result limit. See `candidates::peaks`.
fn checkpoint(operation: &OperationContext) -> Result<()> {
    match operation.interruption() {
        Some(interruption) => Err(interruption.into()),
        None => Ok(()),
    }
}

/// Reports an image-boundary failure as a search failure.
fn backend_failure(_fault: ImageFault) -> Error {
    // The specific fault is deliberately not carried into the public error: the
    // public matching contract is a category and a reason, and a backend's own
    // diagnostic text is non-normative data no caller should branch on.
    VisionFault::BackendFailed.into()
}

fn unrepresentable() -> Error {
    VisionFault::BackendFailed.into()
}

#[cfg(test)]
mod tests {
    use super::{BACKEND_ID, OpenCvBackend, SUPPORTED_MAJOR, accept_version};
    use mado_pilot_core::Status;
    use mado_pilot_vision::MatchBackend;

    #[test]
    fn a_supported_version_becomes_the_descriptors_version() {
        let version = accept_version(SUPPORTED_MAJOR, "4.14.0 ").expect("supported");

        assert_eq!(&*version, "4.14.0");
    }

    #[test]
    fn an_unsupported_major_version_reports_the_backend_unavailable() {
        for major in [0, 3, 5] {
            let error = accept_version(major, "9.9.9").expect_err("unsupported");

            assert_eq!(error.status(), Status::VisionFailed);
        }
    }

    #[test]
    fn a_version_the_library_will_not_name_reports_the_backend_unavailable() {
        assert!(accept_version(SUPPORTED_MAJOR, "   ").is_err());
    }

    #[test]
    fn the_linked_library_reports_a_supported_version() {
        let backend = OpenCvBackend::new().expect("the linked OpenCV is supported");
        let descriptor = backend.descriptor();

        assert_eq!(descriptor.id(), BACKEND_ID);
        assert!(
            descriptor.version().starts_with("4."),
            "the descriptor records the runtime version"
        );
    }
}
