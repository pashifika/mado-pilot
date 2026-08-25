//! Platform- and executor-neutral OCR backend seam.

use std::fmt;

use mado_pilot_capture::{CpuMapping, PixelFormat};
use mado_pilot_core::{OperationContext, PixelExtent, PixelRect, Result};

use crate::fault::OcrFault;
use crate::model::{BackendId, BackendVersion, ModelId, OcrModelIdentity, ProfileId};
use crate::request::MAX_OCR_ZONES;

/// One borrowed backend candidate before contract validation and normalization.
///
/// Borrowed text lets an adapter submit a decoder-buffer view without allocating
/// one `String` or `Arc` per candidate. Geometry is relative to the effective
/// source region's origin and ordered around the recognized quadrilateral.
#[derive(Clone, Copy, PartialEq)]
pub struct BackendCandidate<'a> {
    text: &'a [u8],
    quadrilateral: [(f64, f64); 4],
    confidence: f64,
    detector_order: u32,
}

impl<'a> BackendCandidate<'a> {
    /// Builds an untrusted borrowed backend candidate.
    #[must_use]
    pub const fn new(
        text: &'a [u8],
        quadrilateral: [(f64, f64); 4],
        confidence: f64,
        detector_order: u32,
    ) -> Self {
        Self {
            text,
            quadrilateral,
            confidence,
            detector_order,
        }
    }

    /// Returns the raw text bytes.
    #[must_use]
    pub const fn text(&self) -> &'a [u8] {
        self.text
    }

    /// Returns candidate points relative to the effective source region.
    #[must_use]
    pub const fn quadrilateral(&self) -> [(f64, f64); 4] {
        self.quadrilateral
    }

    /// Returns the backend confidence observation.
    #[must_use]
    pub const fn confidence(&self) -> f64 {
        self.confidence
    }

    /// Returns the backend's stable detector order.
    #[must_use]
    pub const fn detector_order(&self) -> u32 {
        self.detector_order
    }
}

impl fmt::Debug for BackendCandidate<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackendCandidate")
            .field("text_bytes", &self.text.len())
            .field("quadrilateral", &self.quadrilateral)
            .field("confidence", &self.confidence)
            .field("detector_order", &self.detector_order)
            .finish()
    }
}

/// Bounded destination for untrusted backend candidates.
///
/// A backend must propagate the first `push` failure and stop producing output.
/// The recognizer independently latches that failure, so ignoring it cannot make
/// a partial result observable.
pub trait OcrCandidateSink {
    /// Validates and retains one candidate within the request's hard ceilings.
    ///
    /// # Errors
    ///
    /// Returns a malformed-output or interruption error. No candidate beyond a
    /// returned error may be submitted.
    fn push(&mut self, candidate: BackendCandidate<'_>) -> Result<()>;
}
/// Borrowed caller-order interest rectangles relative to one mapped envelope.
#[derive(Debug, Clone, Copy)]
pub struct BackendInterests<'a> {
    zones: &'a [PixelRect],
}

impl<'a> BackendInterests<'a> {
    pub(crate) fn new(zones: &'a [PixelRect]) -> Result<Self> {
        if !(1..=MAX_OCR_ZONES).contains(&zones.len()) {
            return Err(OcrFault::ZoneCountOutOfRange.into());
        }
        Ok(Self { zones })
    }

    /// Returns relative half-open interest rectangles in caller order.
    #[must_use]
    pub const fn zones(self) -> &'a [PixelRect] {
        self.zones
    }
}

/// Computes exact caller-zone membership for one envelope-relative candidate.
///
/// The centroid is the arithmetic mean of the four finite points. Membership
/// uses half-open rectangle edges without epsilon, proximity, or intersection
/// heuristics.
///
/// # Errors
///
/// Returns [`OcrFault::BackendGeometryInvalid`] when a point or centroid is
/// non-finite or a point lies outside `source_extent`.
pub fn candidate_interest_membership(
    quadrilateral: [(f64, f64); 4],
    source_extent: PixelExtent,
    interests: BackendInterests<'_>,
) -> Result<u8> {
    let width = f64::from(source_extent.width());
    let height = f64::from(source_extent.height());
    for (x, y) in quadrilateral {
        if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 || x > width || y > height {
            return Err(OcrFault::BackendGeometryInvalid.into());
        }
    }
    // Opposite vertices stay paired under cyclic rotation and reversal. Averaging
    // the two diagonal midpoints therefore makes boundary membership independent
    // of which valid vertex a backend lists first.
    let diagonal_0 = (
        (quadrilateral[0].0 + quadrilateral[2].0) * 0.5,
        (quadrilateral[0].1 + quadrilateral[2].1) * 0.5,
    );
    let diagonal_1 = (
        (quadrilateral[1].0 + quadrilateral[3].0) * 0.5,
        (quadrilateral[1].1 + quadrilateral[3].1) * 0.5,
    );
    let centroid = (
        (diagonal_0.0 + diagonal_1.0) * 0.5,
        (diagonal_0.1 + diagonal_1.1) * 0.5,
    );
    if !centroid.0.is_finite() || !centroid.1.is_finite() {
        return Err(OcrFault::BackendGeometryInvalid.into());
    }

    let mut membership = 0_u8;
    for (index, zone) in interests.zones().iter().enumerate() {
        if f64::from(zone.left()) <= centroid.0
            && centroid.0 < f64::from(zone.right())
            && f64::from(zone.top()) <= centroid.1
            && centroid.1 < f64::from(zone.bottom())
        {
            membership |= 1_u8 << index;
        }
    }
    Ok(membership)
}

/// Everything a backend needs for one recognition call.
#[derive(Debug)]
pub struct BackendRequest<'a> {
    pixels: &'a CpuMapping,
    interests: Option<BackendInterests<'a>>,
    max_candidates: usize,
    max_text_bytes: usize,
}

impl<'a> BackendRequest<'a> {
    pub(crate) const fn new(
        pixels: &'a CpuMapping,
        interests: Option<BackendInterests<'a>>,
        max_candidates: usize,
        max_text_bytes: usize,
    ) -> Self {
        Self {
            interests,
            pixels,
            max_candidates,
            max_text_bytes,
        }
    }

    /// Returns effective source-region pixels in the declared backend format.
    #[must_use]
    pub const fn pixels(&self) -> &'a CpuMapping {
        self.pixels
    }

    /// Returns the authoritative effective region carried by the mapping.
    #[must_use]
    pub const fn region(&self) -> PixelRect {
        self.pixels.region()
    }
    /// Returns optional caller-order interest rectangles relative to the mapping.
    #[must_use]
    pub const fn interests(&self) -> Option<BackendInterests<'a>> {
        self.interests
    }

    /// Returns the hard maximum candidate count.
    #[must_use]
    pub const fn max_candidates(&self) -> usize {
        self.max_candidates
    }

    /// Returns the hard maximum raw UTF-8 byte count per candidate.
    #[must_use]
    pub const fn max_text_bytes(&self) -> usize {
        self.max_text_bytes
    }
}

/// Stable backend implementation identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OcrBackendIdentity {
    id: BackendId,
    version: BackendVersion,
}

impl OcrBackendIdentity {
    /// Builds an exact backend identity.
    #[must_use]
    pub const fn new(id: BackendId, version: BackendVersion) -> Self {
        Self { id, version }
    }

    /// Returns the stable backend identifier.
    #[must_use]
    pub const fn id(&self) -> &BackendId {
        &self.id
    }

    /// Returns the bounded implementation version.
    #[must_use]
    pub const fn version(&self) -> &BackendVersion {
        &self.version
    }
}

impl fmt::Display for OcrBackendIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} {}", self.id, self.version)
    }
}

/// Exact backend, model/profile, and pixel-format identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrBackendDescriptor {
    backend: OcrBackendIdentity,
    model: OcrModelIdentity,
    format: PixelFormat,
}

impl OcrBackendDescriptor {
    /// Builds an OCR backend descriptor from already validated identities.
    #[must_use]
    pub const fn new(
        backend: OcrBackendIdentity,
        model: OcrModelIdentity,
        format: PixelFormat,
    ) -> Self {
        Self {
            backend,
            model,
            format,
        }
    }

    /// Returns the exact backend implementation identity.
    #[must_use]
    pub const fn backend_identity(&self) -> &OcrBackendIdentity {
        &self.backend
    }

    /// Returns the stable backend identifier.
    #[must_use]
    pub const fn id(&self) -> &BackendId {
        self.backend.id()
    }

    /// Returns the bounded backend implementation version.
    #[must_use]
    pub const fn version(&self) -> &BackendVersion {
        self.backend.version()
    }

    /// Returns complete model, component, and profile identity.
    #[must_use]
    pub const fn model_identity(&self) -> &OcrModelIdentity {
        &self.model
    }

    /// Returns the stable model identifier.
    #[must_use]
    pub const fn model(&self) -> &ModelId {
        self.model.model()
    }

    /// Returns the exact profile identifier.
    #[must_use]
    pub const fn profile(&self) -> &ProfileId {
        self.model.profile()
    }

    /// Returns the pixel format required by the backend.
    #[must_use]
    pub const fn format(&self) -> PixelFormat {
        self.format
    }
}

impl fmt::Display for OcrBackendDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} (model {} {}, profile {})",
            self.backend,
            self.model.model(),
            self.model.version(),
            self.model.profile()
        )
    }
}

/// An OCR implementation over one already validated immutable model source.
pub trait OcrBackend: fmt::Debug + Send + Sync {
    /// Returns the backend's exact public identity and required pixel format.
    fn descriptor(&self) -> OcrBackendDescriptor;

    /// Recognizes text in `request.pixels()` and streams bounded candidates.
    ///
    /// The hard limits in `request` must be enforced before candidate collection
    /// or text allocation crosses them. Internal tensor, session, and decoder
    /// allocations remain backend-owned and require their own measured bounds.
    ///
    /// # Errors
    ///
    /// Returns a backend failure, sink refusal, or operation interruption. A
    /// backend must use the same absolute deadline and cancellation context and
    /// stop after the first sink error.
    fn recognize(
        &self,
        request: &BackendRequest<'_>,
        output: &mut dyn OcrCandidateSink,
        operation: &OperationContext,
    ) -> Result<()>;

    /// Closes backend resources idempotently.
    ///
    /// # Errors
    ///
    /// Returns a backend failure or operation interruption. Implementations may
    /// perform only bounded cleanup after interruption.
    fn close(&self, operation: &OperationContext) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::{BackendCandidate, BackendInterests, candidate_interest_membership};
    use mado_pilot_core::{PixelExtent, PixelRect, Status};

    #[test]
    fn candidate_debug_never_prints_recognized_text() {
        let candidate = BackendCandidate::new(
            b"private-screen-text",
            [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)],
            0.5,
            0,
        );

        let debug = format!("{candidate:?}");
        assert!(!debug.contains("private-screen-text"));
        assert!(debug.contains("text_bytes"));
    }

    #[test]
    fn interests_are_bounded_and_membership_uses_exact_half_open_edges() {
        let zones = [
            PixelRect::new(0, 0, 5, 5).unwrap(),
            PixelRect::new(5, 0, 10, 5).unwrap(),
        ];
        let interests = BackendInterests::new(&zones).unwrap();
        let extent = PixelExtent::new(10, 5);

        assert_eq!(
            candidate_interest_membership([(5.0, 2.0); 4], extent, interests).unwrap(),
            0b10
        );
        let immediately_left = f64::from_bits(5.0_f64.to_bits() - 1);
        assert_eq!(
            candidate_interest_membership([(immediately_left, 2.0); 4], extent, interests).unwrap(),
            0b01
        );
        assert_eq!(
            candidate_interest_membership([(10.0, 2.0); 4], extent, interests).unwrap(),
            0
        );
    }

    #[test]
    fn membership_is_invariant_to_cyclic_and_reversed_vertex_order() {
        let zones = [
            PixelRect::new(0, 0, 10, 5).unwrap(),
            PixelRect::new(10, 0, 20, 5).unwrap(),
        ];
        let interests = BackendInterests::new(&zones).unwrap();
        let extent = PixelExtent::new(20, 5);
        let points = [
            (1.000_000_000_000_406_6, 1.0),
            (18.999_999_999_984_155, 1.0),
            (19.000_000_000_000_345, 3.0),
            (1.000_000_000_015_094_4, 3.0),
        ];
        let equivalent_orders = [
            [points[0], points[1], points[2], points[3]],
            [points[1], points[2], points[3], points[0]],
            [points[2], points[3], points[0], points[1]],
            [points[3], points[0], points[1], points[2]],
            [points[0], points[3], points[2], points[1]],
            [points[3], points[2], points[1], points[0]],
            [points[2], points[1], points[0], points[3]],
            [points[1], points[0], points[3], points[2]],
        ];

        for quadrilateral in equivalent_orders {
            assert_eq!(
                candidate_interest_membership(quadrilateral, extent, interests).unwrap(),
                0b10
            );
        }
    }

    #[test]
    fn membership_rejects_non_finite_and_outside_envelope_points() {
        let zones = [PixelRect::new(0, 0, 10, 5).unwrap()];
        let interests = BackendInterests::new(&zones).unwrap();
        let extent = PixelExtent::new(10, 5);

        for points in [[(f64::NAN, 2.0); 4], [(10.1, 2.0); 4], [(-0.1, 2.0); 4]] {
            assert_eq!(
                candidate_interest_membership(points, extent, interests)
                    .unwrap_err()
                    .status(),
                Status::VisionFailed
            );
        }
    }

    #[test]
    fn interest_view_refuses_zero_and_nine_entries() {
        assert_eq!(
            BackendInterests::new(&[]).unwrap_err().status(),
            Status::InvalidArgument
        );
        let nine = [PixelRect::new(0, 0, 1, 1).unwrap(); 9];
        assert_eq!(
            BackendInterests::new(&nine).unwrap_err().status(),
            Status::InvalidArgument
        );
    }
}
