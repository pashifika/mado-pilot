//! Platform- and executor-neutral OCR backend seam.

use std::fmt;
use std::sync::Arc;

use mado_pilot_capture::{CpuMapping, PixelFormat};
use mado_pilot_core::{OperationContext, PixelRect, Result};

use crate::model::{BackendId, BackendVersion, ModelId, ProfileId};

/// One backend candidate before contract validation and normalization.
///
/// Text is raw bytes so a broken adapter cannot make malformed UTF-8
/// unrepresentable only in tests. Geometry is relative to the effective source
/// region's origin and ordered around the recognized quadrilateral.
#[derive(Debug, Clone, PartialEq)]
pub struct BackendCandidate {
    text: Arc<[u8]>,
    quadrilateral: [(f64, f64); 4],
    confidence: f64,
    detector_order: u32,
}

impl BackendCandidate {
    /// Builds an untrusted backend candidate.
    #[must_use]
    pub const fn new(
        text: Arc<[u8]>,
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
    pub fn text(&self) -> &[u8] {
        &self.text
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

/// Everything a backend needs for one recognition call.
#[derive(Debug)]
pub struct BackendRequest<'a> {
    /// Effective source-region pixels in the backend's declared format.
    pub pixels: &'a CpuMapping,
    /// Effective source region in full-frame capture pixels.
    pub region: PixelRect,
}

/// Exact backend, model, profile, and pixel-format identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrBackendDescriptor {
    id: BackendId,
    version: BackendVersion,
    model: ModelId,
    profile: ProfileId,
    format: PixelFormat,
}

impl OcrBackendDescriptor {
    /// Builds an OCR backend descriptor from already validated identities.
    #[must_use]
    pub const fn new(
        id: BackendId,
        version: BackendVersion,
        model: ModelId,
        profile: ProfileId,
        format: PixelFormat,
    ) -> Self {
        Self {
            id,
            version,
            model,
            profile,
            format,
        }
    }

    /// Returns the stable backend identifier.
    #[must_use]
    pub const fn id(&self) -> &BackendId {
        &self.id
    }

    /// Returns the bounded backend implementation version.
    #[must_use]
    pub const fn version(&self) -> &BackendVersion {
        &self.version
    }

    /// Returns the exact model identity loaded by the backend.
    #[must_use]
    pub const fn model(&self) -> &ModelId {
        &self.model
    }

    /// Returns the exact profile identity enforced by the backend and contract.
    #[must_use]
    pub const fn profile(&self) -> &ProfileId {
        &self.profile
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
            "{} {} (model {}, profile {})",
            self.id, self.version, self.model, self.profile
        )
    }
}

/// An OCR implementation over one already validated immutable model source.
pub trait OcrBackend: fmt::Debug + Send + Sync {
    /// Returns the backend's exact public identity and required pixel format.
    fn descriptor(&self) -> OcrBackendDescriptor;

    /// Recognizes text in `request.pixels`.
    ///
    /// Candidates remain untrusted until the OCR contract validates and commits
    /// them. An empty vector is successful recognition with no text.
    ///
    /// # Errors
    ///
    /// Returns a backend failure or the operation's terminal outcome. A backend
    /// must use the same absolute deadline and cancellation context it receives.
    fn recognize(
        &self,
        request: &BackendRequest<'_>,
        operation: &OperationContext,
    ) -> Result<Vec<BackendCandidate>>;

    /// Closes backend resources idempotently.
    ///
    /// # Errors
    ///
    /// Returns a backend failure or operation interruption. Implementations may
    /// perform only bounded cleanup after interruption.
    fn close(&self, operation: &OperationContext) -> Result<()>;
}
