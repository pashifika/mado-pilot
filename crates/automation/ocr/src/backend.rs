//! Platform- and executor-neutral OCR backend seam.

use std::fmt;

use mado_pilot_capture::{CpuMapping, PixelFormat};
use mado_pilot_core::{OperationContext, PixelRect, Result};

use crate::model::{BackendId, BackendVersion, ModelId, OcrModelIdentity, ProfileId};

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

/// Everything a backend needs for one recognition call.
#[derive(Debug)]
pub struct BackendRequest<'a> {
    pixels: &'a CpuMapping,
    max_candidates: usize,
    max_text_bytes: usize,
}

impl<'a> BackendRequest<'a> {
    pub(crate) const fn new(
        pixels: &'a CpuMapping,
        max_candidates: usize,
        max_text_bytes: usize,
    ) -> Self {
        Self {
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
    use super::BackendCandidate;

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
}
