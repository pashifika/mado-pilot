//! Immutable, source-correlated OCR result values.

use std::fmt;
use std::sync::Arc;

use mado_pilot_core::{CoordinateSpace, FrameStamp, PixelRect, Point, TransformSnapshot};

use crate::backend::OcrBackendDescriptor;

/// A finite profile confidence observation in `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Confidence(f64);

impl Confidence {
    pub(crate) const fn new_validated(value: f64) -> Self {
        Self(value)
    }

    /// Returns the profile-defined confidence observation.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

/// Four ordered points around one recognized text region.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OcrQuadrilateral([Point; 4]);

impl OcrQuadrilateral {
    pub(crate) const fn new(points: [Point; 4]) -> Self {
        Self(points)
    }

    /// Returns the ordered points in the result's declared coordinate space.
    #[must_use]
    pub const fn points(self) -> [Point; 4] {
        self.0
    }
}

/// One normalized recognized text region.
#[derive(Clone, PartialEq)]
pub struct RecognizedRegion {
    text: Arc<str>,
    geometry: OcrQuadrilateral,
    confidence: Confidence,
}

impl RecognizedRegion {
    pub(crate) const fn new(
        text: Arc<str>,
        geometry: OcrQuadrilateral,
        confidence: Confidence,
    ) -> Self {
        Self {
            text,
            geometry,
            confidence,
        }
    }

    /// Returns NFC-normalized, Unicode-trimmed text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the recognized quadrilateral.
    #[must_use]
    pub const fn geometry(&self) -> OcrQuadrilateral {
        self.geometry
    }

    /// Returns the profile-defined confidence observation.
    #[must_use]
    pub const fn confidence(&self) -> Confidence {
        self.confidence
    }
}

impl fmt::Debug for RecognizedRegion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecognizedRegion")
            .field("text_bytes", &self.text.len())
            .field("geometry", &self.geometry)
            .field("confidence", &self.confidence)
            .finish()
    }
}

/// The immutable outcome of recognizing one exact source frame region.
#[derive(Debug, Clone, PartialEq)]
pub struct OcrResult {
    stamp: FrameStamp,
    transform: TransformSnapshot,
    effective_region: PixelRect,
    output_space: CoordinateSpace,
    backend: OcrBackendDescriptor,
    regions: Arc<[RecognizedRegion]>,
}

impl OcrResult {
    pub(crate) const fn new(
        stamp: FrameStamp,
        transform: TransformSnapshot,
        effective_region: PixelRect,
        output_space: CoordinateSpace,
        backend: OcrBackendDescriptor,
        regions: Arc<[RecognizedRegion]>,
    ) -> Self {
        Self {
            stamp,
            transform,
            effective_region,
            output_space,
            backend,
            regions,
        }
    }

    /// Returns stream, epoch, sequence, and geometry identity of the source frame.
    #[must_use]
    pub const fn stamp(&self) -> FrameStamp {
        self.stamp
    }

    /// Returns the source frame's immutable transform snapshot.
    #[must_use]
    pub const fn transform(&self) -> &TransformSnapshot {
        &self.transform
    }

    /// Returns the effective clipped source region in full-frame capture pixels.
    #[must_use]
    pub const fn effective_region(&self) -> PixelRect {
        self.effective_region
    }

    /// Returns the declared coordinate space of every recognized quadrilateral.
    #[must_use]
    pub const fn output_space(&self) -> CoordinateSpace {
        self.output_space
    }

    /// Returns the exact backend/model/profile identity that produced the result.
    #[must_use]
    pub const fn backend(&self) -> &OcrBackendDescriptor {
        &self.backend
    }

    /// Returns recognized regions in deterministic detector order.
    #[must_use]
    pub fn regions(&self) -> &[RecognizedRegion] {
        &self.regions
    }

    /// Reports whether recognition produced no non-empty normalized text.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }
}
