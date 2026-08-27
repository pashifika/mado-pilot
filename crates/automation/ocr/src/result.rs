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

/// Borrowed immutable view of one caller-order OCR zone group.
#[derive(Debug, Clone, Copy)]
pub struct OcrZoneGroup<'a> {
    candidates: &'a [RecognizedRegion],
    indexes: &'a [u16],
}

impl<'a> OcrZoneGroup<'a> {
    const fn new(candidates: &'a [RecognizedRegion], indexes: &'a [u16]) -> Self {
        Self {
            candidates,
            indexes,
        }
    }

    /// Returns the number of candidate memberships in this group.
    #[must_use]
    pub const fn len(self) -> usize {
        self.indexes.len()
    }

    /// Reports whether this group has no candidate memberships.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.indexes.is_empty()
    }

    /// Returns one candidate membership in global detector order.
    #[must_use]
    pub fn get(self, index: usize) -> Option<&'a RecognizedRegion> {
        let candidate = usize::from(*self.indexes.get(index)?);
        self.candidates.get(candidate)
    }

    /// Iterates candidate memberships in global detector order.
    pub fn iter(self) -> impl ExactSizeIterator<Item = &'a RecognizedRegion> + 'a {
        self.indexes
            .iter()
            .map(|&index| &self.candidates[usize::from(index)])
    }
}

/// The immutable grouped outcome of scanning one exact source frame.
#[derive(Clone, PartialEq)]
pub struct OcrZoneScanResult {
    stamp: FrameStamp,
    transform: TransformSnapshot,
    source_envelope: PixelRect,
    effective_zones: Arc<[PixelRect]>,
    output_space: CoordinateSpace,
    backend: OcrBackendDescriptor,
    candidates: Arc<[RecognizedRegion]>,
    membership_indexes: Arc<[u16]>,
    group_offsets: [u16; 9],
}

impl fmt::Debug for OcrZoneScanResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OcrZoneScanResult")
            .field("stamp", &self.stamp)
            .field("transform", &self.transform)
            .field("source_envelope", &self.source_envelope)
            .field("effective_zones", &self.effective_zones)
            .field("output_space", &self.output_space)
            .field("backend", self.backend.backend_identity())
            .field("model", &self.backend.model())
            .field("profile", &self.backend.profile())
            .field("unique_candidates", &self.candidates.len())
            .field("memberships", &self.membership_indexes.len())
            .finish()
    }
}

impl OcrZoneScanResult {
    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn new(
        stamp: FrameStamp,
        transform: TransformSnapshot,
        source_envelope: PixelRect,
        effective_zones: Arc<[PixelRect]>,
        output_space: CoordinateSpace,
        backend: OcrBackendDescriptor,
        candidates: Arc<[RecognizedRegion]>,
        membership_indexes: Arc<[u16]>,
        group_offsets: [u16; 9],
    ) -> Self {
        Self {
            stamp,
            transform,
            source_envelope,
            effective_zones,
            output_space,
            backend,
            candidates,
            membership_indexes,
            group_offsets,
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

    /// Returns the mapped source envelope in full-frame capture pixels.
    #[must_use]
    pub const fn source_envelope(&self) -> PixelRect {
        self.source_envelope
    }

    /// Returns effective clipped zones in caller order.
    #[must_use]
    pub fn effective_zones(&self) -> &[PixelRect] {
        &self.effective_zones
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

    /// Returns candidates owned once in global detector order.
    #[must_use]
    pub fn unique_candidates(&self) -> &[RecognizedRegion] {
        &self.candidates
    }

    /// Returns one caller-order group.
    #[must_use]
    pub fn group(&self, index: usize) -> Option<OcrZoneGroup<'_>> {
        if index >= self.effective_zones.len() {
            return None;
        }
        let start = usize::from(self.group_offsets[index]);
        let end = usize::from(self.group_offsets[index + 1]);
        Some(OcrZoneGroup::new(
            &self.candidates,
            &self.membership_indexes[start..end],
        ))
    }

    /// Reports whether every caller-order group is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.membership_indexes.is_empty()
    }
}
