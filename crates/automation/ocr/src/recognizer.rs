//! Backend-independent OCR admission, validation, normalization, and commit.

use std::sync::Arc;

use mado_pilot_capture::FrameView;
use mado_pilot_core::{
    CoordinateSpace, Error, GeometryFault, Operation, OperationContext, PixelRect, Point, Result,
    TransformSnapshot,
};
use unicode_normalization::{IsNormalized, UnicodeNormalization, is_nfc_quick};

use crate::backend::{
    BackendCandidate, BackendRequest, OcrBackend, OcrBackendDescriptor, OcrCandidateSink,
};
use crate::fault::OcrFault;
use crate::request::{OcrRegion, OcrRequest};
use crate::result::{Confidence, OcrQuadrilateral, OcrResult, RecognizedRegion};

/// Maximum candidates fixed by the accepted G-004 detector profile.
pub const MAX_CANDIDATES: usize = 1_000;
/// Maximum UTF-8 bytes retained for one normalized region.
pub const MAX_TEXT_BYTES: usize = 4 * 1024;
/// Maximum raw text bytes a backend may submit for one candidate.
pub const MAX_BACKEND_TEXT_BYTES: usize = 16 * 1024;

/// Applies one platform-neutral OCR contract over a selected backend.
///
/// Cloning shares only the backend. No request or result state is stored here,
/// so concurrent and out-of-order completion cannot replace another result.
#[derive(Debug, Clone)]
pub struct OcrRecognizer {
    backend: Arc<dyn OcrBackend>,
}

impl OcrRecognizer {
    /// Builds a recognizer over `backend`.
    #[must_use]
    pub fn new(backend: Arc<dyn OcrBackend>) -> Self {
        Self { backend }
    }

    /// Returns the backend's exact identity.
    #[must_use]
    pub fn descriptor(&self) -> OcrBackendDescriptor {
        self.backend.descriptor()
    }

    /// Recognizes one exact immutable frame region.
    ///
    /// One operation context governs admission, pixel mapping, backend work, the
    /// synchronous caller wait, normalization, and final commit. Completion after
    /// deadline or cancellation is discarded rather than published.
    ///
    /// # Errors
    ///
    /// Returns typed identity, geometry, malformed-output, backend, deadline, or
    /// cancellation failures. No partial result is committed.
    pub fn recognize(&self, request: OcrRequest<'_>) -> Result<OcrResult> {
        let operation = request.operation();
        let mut attempt = Operation::admit(operation)?;
        let descriptor = self.backend.descriptor();
        validate_selection(&descriptor, &request)?;

        let frame = request.frame();
        let transform = *frame.transform();
        let effective_region = resolve_region(&transform, request.source_region())?;
        preflight_output(&transform, effective_region, request.output_space())?;

        let view = FrameView::new(frame.clone(), effective_region)?;
        let pixels = view.map(descriptor.format(), operation)?;
        attempt.checkpoint()?;

        let mut normalized = Normalizer::new(
            operation,
            effective_region,
            &transform,
            request.output_space(),
        );
        let backend_outcome = self.backend.recognize(
            &BackendRequest::new(&pixels, MAX_CANDIDATES, MAX_BACKEND_TEXT_BYTES),
            &mut normalized,
            operation,
        );
        attempt.checkpoint()?;
        backend_outcome?;
        let regions_outcome = normalized.finish();
        attempt.checkpoint()?;
        let regions = regions_outcome?;

        let result = OcrResult::new(
            frame.stamp(),
            transform,
            effective_region,
            request.output_space(),
            descriptor,
            Arc::from(regions),
        );
        attempt.commit(result).map_err(Error::from)
    }

    /// Closes backend resources through the caller's operation context.
    ///
    /// # Errors
    ///
    /// Returns the authoritative interruption ahead of a simultaneous backend
    /// close failure.
    pub fn close(&self, operation: &OperationContext) -> Result<()> {
        let mut attempt = Operation::admit(operation)?;
        let backend_outcome = self.backend.close(operation);
        attempt.checkpoint()?;
        backend_outcome?;
        attempt.commit(()).map_err(Error::from)
    }
}

fn validate_selection(descriptor: &OcrBackendDescriptor, request: &OcrRequest<'_>) -> Result<()> {
    if request.backend() != descriptor.backend_identity() {
        return Err(OcrFault::BackendMismatch.into());
    }
    let requested = request.model_identity();
    let selected = descriptor.model_identity();
    if requested.model() != selected.model()
        || requested.version() != selected.version()
        || requested.detector() != selected.detector()
        || requested.recognizer() != selected.recognizer()
    {
        return Err(OcrFault::ModelMismatch.into());
    }
    if requested.profile() != selected.profile()
        || requested.profile_metadata() != selected.profile_metadata()
    {
        return Err(OcrFault::ProfileMismatch.into());
    }
    if selected.profile_metadata().normalization().as_str()
        != crate::model::ACCEPTED_G004_NORMALIZATION_ID
    {
        return Err(OcrFault::UnsupportedProfile.into());
    }
    Ok(())
}

fn resolve_region(transform: &TransformSnapshot, selection: OcrRegion) -> Result<PixelRect> {
    match selection {
        OcrRegion::FullFrame => transform.frame_bounds().map_err(Error::from),
        OcrRegion::Region { rect, policy } => {
            rect.require_non_empty()?;
            transform
                .resolve_capture_pixels(rect, policy)
                .map_err(Error::from)
        }
    }
}

fn preflight_output(
    transform: &TransformSnapshot,
    region: PixelRect,
    output_space: CoordinateSpace,
) -> Result<()> {
    if !transform.supports(output_space) {
        return Err(GeometryFault::ConversionUnsupported.into());
    }
    let origin = Point::new(
        CoordinateSpace::CapturePixels,
        f64::from(region.left()),
        f64::from(region.top()),
    )?;
    transform.convert_point(origin, output_space)?;
    Ok(())
}

#[derive(Debug)]
struct NormalizedCandidate {
    detector_order: u32,
    region: Option<RecognizedRegion>,
}

#[derive(Debug)]
struct Normalizer<'a> {
    operation: &'a OperationContext,
    region: PixelRect,
    transform: &'a TransformSnapshot,
    output_space: CoordinateSpace,
    submitted: usize,
    candidates: Vec<NormalizedCandidate>,
    fault: Option<Error>,
}

impl<'a> Normalizer<'a> {
    const fn new(
        operation: &'a OperationContext,
        region: PixelRect,
        transform: &'a TransformSnapshot,
        output_space: CoordinateSpace,
    ) -> Self {
        Self {
            operation,
            region,
            transform,
            output_space,
            submitted: 0,
            candidates: Vec::new(),
            fault: None,
        }
    }

    fn process(&mut self, candidate: BackendCandidate<'_>) -> Result<()> {
        if let Some(interruption) = self.operation.interruption() {
            return Err(interruption.into());
        }
        if self.submitted >= MAX_CANDIDATES {
            return Err(OcrFault::BackendCandidateCountAboveCeiling.into());
        }
        self.submitted += 1;

        let confidence = normalize_confidence(candidate.confidence())?;
        let geometry = normalize_geometry(
            candidate.quadrilateral(),
            self.region,
            self.transform,
            self.output_space,
        )?;
        let text = normalize_text(candidate.text())?;
        let region = if text.is_empty() {
            None
        } else {
            Some(RecognizedRegion::new(text, geometry, confidence))
        };
        self.candidates.push(NormalizedCandidate {
            detector_order: candidate.detector_order(),
            region,
        });
        Ok(())
    }

    fn finish(mut self) -> Result<Vec<RecognizedRegion>> {
        if let Some(fault) = self.fault {
            return Err(fault);
        }
        self.candidates
            .sort_by_key(|candidate| candidate.detector_order);
        if self
            .candidates
            .windows(2)
            .any(|pair| pair[0].detector_order == pair[1].detector_order)
        {
            return Err(OcrFault::BackendOrderDuplicate.into());
        }
        Ok(self
            .candidates
            .into_iter()
            .filter_map(|candidate| candidate.region)
            .collect())
    }
}

impl OcrCandidateSink for Normalizer<'_> {
    fn push(&mut self, candidate: BackendCandidate<'_>) -> Result<()> {
        if let Some(fault) = &self.fault {
            return Err(fault.clone());
        }
        match self.process(candidate) {
            Ok(()) => Ok(()),
            Err(fault) => {
                self.fault = Some(fault.clone());
                Err(fault)
            }
        }
    }
}

fn normalize_text(raw: &[u8]) -> Result<Arc<str>> {
    if raw.len() > MAX_BACKEND_TEXT_BYTES {
        return Err(OcrFault::BackendTextAboveCeiling.into());
    }
    let text = std::str::from_utf8(raw).map_err(|_| Error::from(OcrFault::BackendTextNotUtf8))?;
    let owned;
    let nfc = if is_nfc_quick(text.chars()) == IsNormalized::Yes {
        text
    } else {
        owned = text.nfc().collect::<String>();
        &owned
    };
    let trimmed = nfc.trim();
    if trimmed.len() > MAX_TEXT_BYTES {
        return Err(OcrFault::BackendTextAboveCeiling.into());
    }
    Ok(Arc::from(trimmed))
}

fn normalize_confidence(value: f64) -> Result<Confidence> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(OcrFault::BackendConfidenceOutOfRange.into());
    }
    let rounded = (value * 100_000.0).round_ties_even() / 100_000.0;
    Ok(Confidence::new_validated(rounded))
}

fn normalize_geometry(
    points: [(f64, f64); 4],
    region: PixelRect,
    transform: &TransformSnapshot,
    output_space: CoordinateSpace,
) -> Result<OcrQuadrilateral> {
    validate_relative_quad(points, region)?;
    let mut converted = [Point::new(output_space, 0.0, 0.0)?; 4];
    for (destination, (x, y)) in converted.iter_mut().zip(points) {
        let capture_point = Point::new(
            CoordinateSpace::CapturePixels,
            f64::from(region.left()) + x,
            f64::from(region.top()) + y,
        )?;
        *destination = transform.convert_point(capture_point, output_space)?;
    }
    Ok(OcrQuadrilateral::new(converted))
}

fn validate_relative_quad(points: [(f64, f64); 4], region: PixelRect) -> Result<()> {
    let width = f64::from(region.width());
    let height = f64::from(region.height());
    if points.iter().any(|&(x, y)| {
        !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 || x > width || y > height
    }) {
        return Err(OcrFault::BackendGeometryInvalid.into());
    }

    let mut direction = 0.0_f64;
    for index in 0..4 {
        let a = points[index];
        let b = points[(index + 1) % 4];
        let c = points[(index + 2) % 4];
        let cross = (b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0);
        if !cross.is_finite() || cross.abs() <= f64::EPSILON {
            return Err(OcrFault::BackendGeometryInvalid.into());
        }
        if direction == 0.0 {
            direction = cross;
        } else if cross.is_sign_positive() != direction.is_sign_positive() {
            return Err(OcrFault::BackendGeometryInvalid.into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{normalize_confidence, normalize_text, validate_relative_quad};
    use crate::OcrFault;
    use mado_pilot_core::{PixelRect, Status};

    #[test]
    fn text_is_nfc_normalized_trimmed_and_confidence_is_rounded() {
        let text = normalize_text("  e\u{301}  ".as_bytes()).unwrap();
        let confidence = normalize_confidence(0.123_456).unwrap();

        assert_eq!(&*text, "é");
        assert_eq!(confidence.get(), 0.123_46);
    }

    #[test]
    fn confidence_halfway_values_round_to_even() {
        assert_eq!(normalize_confidence(0.000_005).unwrap().get(), 0.0);
        assert_eq!(normalize_confidence(0.000_025).unwrap().get(), 0.000_02);
        assert_eq!(normalize_confidence(0.123_445).unwrap().get(), 0.123_44);
    }

    #[test]
    fn malformed_utf8_and_non_convex_geometry_are_refused() {
        let error = normalize_text(&[0xff]).unwrap_err();
        assert_eq!(error.status(), Status::VisionFailed);
        let region = PixelRect::new(0, 0, 20, 20).unwrap();
        let error =
            validate_relative_quad([(0.0, 0.0), (10.0, 0.0), (5.0, 5.0), (0.0, 10.0)], region)
                .unwrap_err();
        assert_eq!(error.status(), OcrFault::BackendGeometryInvalid.status());
    }
}
