//! Backend-independent OCR admission, validation, normalization, and commit.

use std::{fmt, mem::size_of, sync::Arc};

use mado_pilot_capture::{CpuMapping, Frame, FrameView, PixelFormat};
use mado_pilot_core::{
    CoordinateSpace, Error, GeometryFault, Operation, OperationContext, PixelRect, Point, Result,
    TransformSnapshot,
};

use crate::backend::{
    BackendCandidate, BackendInterests, BackendRequest, OcrBackend, OcrBackendDescriptor,
    OcrBackendIdentity, OcrCandidateSink, candidate_interest_membership,
};
use crate::fault::OcrFault;
use crate::model::OcrModelIdentity;
use crate::normalization::normalize_backend;
use crate::request::{MAX_OCR_ZONES, OcrRegion, OcrRequest, OcrZone, OcrZoneScanRequest};
use crate::result::{Confidence, OcrQuadrilateral, OcrResult, OcrZoneScanResult, RecognizedRegion};

/// Maximum candidates fixed by the accepted G-004 detector profile.
pub const MAX_CANDIDATES: usize = 1_000;
/// Maximum UTF-8 bytes retained for one normalized region.
pub const MAX_TEXT_BYTES: usize = 4 * 1024;
/// Maximum raw text bytes a backend may submit for one candidate.
pub const MAX_BACKEND_TEXT_BYTES: usize = 16 * 1024;
const MAX_ZONE_MEMBERSHIPS: usize = 8_000;
const MAX_AGGREGATE_RAW_TEXT_BYTES: usize = 16_384_000;
const MAX_AGGREGATE_NORMALIZED_TEXT_BYTES: usize = 4_096_000;
const MAX_MAPPING_BYTES: usize = 268_435_456;
const MAX_TEMPORARY_GROUPED_BYTES: usize = 262_144;
const MAX_MEMBERSHIP_INDEX_BYTES: usize = 16_000;
const MAX_GROUPED_RESULT_BYTES: usize = 5_242_880;

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

    /// Returns immutable execution-provider initialization facts when available.
    #[must_use]
    pub fn provider_descriptor(&self) -> Option<crate::OcrProviderDescriptor> {
        self.backend.provider_descriptor()
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
        prepare(self, request)?.execute()
    }

    /// Scans one through eight zones through one shared source envelope.
    ///
    /// One operation context governs identity validation, zone resolution,
    /// mapping, backend work, grouping, and final all-or-nothing commit.
    ///
    /// # Errors
    ///
    /// Returns typed identity, geometry, limit, malformed-output, backend,
    /// deadline, or cancellation failures. No partial group is committed.
    pub fn scan_zones(&self, request: OcrZoneScanRequest<'_>) -> Result<OcrZoneScanResult> {
        if !(1..=MAX_OCR_ZONES).contains(&request.zones().len()) {
            return Err(OcrFault::ZoneCountOutOfRange.into());
        }

        let operation = request.operation();
        let mut attempt = Operation::admit(operation)?;
        let descriptor = self.backend.descriptor();
        validate_selection(&descriptor, request.backend(), request.model_identity())?;

        let frame = request.frame();
        let transform = *frame.transform();
        let resolved = resolve_zones(&transform, request.zones())?;
        preflight_output(&transform, resolved.envelope, request.output_space())?;
        preflight_mapping_bytes(frame, resolved.envelope, descriptor.format())?;

        let view = FrameView::new(frame.clone(), resolved.envelope)?;
        let pixels = view.map(descriptor.format(), operation)?;
        enforce_mapping_ceiling(pixels.bytes().len())?;
        attempt.checkpoint()?;

        let interests = BackendInterests::new(&resolved.relative)?;
        let mut normalized = GroupedNormalizer::new(
            operation,
            resolved.envelope,
            &transform,
            request.output_space(),
            interests,
        );
        let backend_outcome = self.backend.recognize(
            &BackendRequest::new(
                &pixels,
                Some(interests),
                MAX_CANDIDATES,
                MAX_BACKEND_TEXT_BYTES,
            ),
            &mut normalized,
            operation,
        );
        attempt.checkpoint()?;
        backend_outcome?;
        let grouped_outcome = normalized.finish();
        attempt.checkpoint()?;
        let grouped = grouped_outcome?;

        let result = OcrZoneScanResult::new(
            frame.stamp(),
            transform,
            resolved.envelope,
            Arc::from(resolved.effective),
            request.output_space(),
            descriptor,
            Arc::from(grouped.candidates),
            Arc::from(grouped.membership_indexes),
            grouped.group_offsets,
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

/// One validated, mapped OCR operation bound to its original source and authority.
///
/// Construction is restricted to [`prepare`]; execution consumes this value and
/// accepts no replacement request, context, frame, or backend.
#[doc(hidden)]
pub struct PreparedOcr {
    view: FrameView,
    pixels: CpuMapping,
    output_space: CoordinateSpace,
    descriptor: OcrBackendDescriptor,
    backend: Arc<dyn OcrBackend>,
    operation: OperationContext,
}

impl fmt::Debug for PreparedOcr {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedOcr")
            .field("stamp", &self.view.stamp())
            .field("effective_region", &self.view.region())
            .field("output_space", &self.output_space)
            .field("backend", self.descriptor.backend_identity())
            .field("model", self.descriptor.model())
            .field("profile", self.descriptor.profile())
            .field("mapped_bytes", &self.pixels.bytes().len())
            .finish()
    }
}

impl PreparedOcr {
    /// Returns the original immutable source frame.
    #[must_use]
    pub const fn frame(&self) -> &Frame {
        self.view.frame()
    }

    /// Returns the exact mapping that execution will consume.
    #[must_use]
    pub const fn pixels(&self) -> &CpuMapping {
        &self.pixels
    }

    /// Returns the resolved capture-pixel source region.
    #[must_use]
    pub const fn effective_region(&self) -> PixelRect {
        self.view.region()
    }

    /// Returns the requested output coordinate space.
    #[must_use]
    pub const fn output_space(&self) -> CoordinateSpace {
        self.output_space
    }

    /// Returns the identity used to validate and map this operation.
    #[must_use]
    pub const fn descriptor(&self) -> &OcrBackendDescriptor {
        &self.descriptor
    }

    /// Executes and commits this exact operation without mapping again.
    ///
    /// # Errors
    ///
    /// Returns the original authority's interruption ahead of a simultaneous
    /// backend or malformed-output failure. No partial result is committed.
    pub fn execute(self) -> Result<OcrResult> {
        let operation = &self.operation;
        let mut attempt = Operation::admit(operation)?;
        let transform = *self.view.transform();
        let effective_region = self.view.region();
        let mut normalized =
            Normalizer::new(operation, effective_region, &transform, self.output_space);
        let backend_outcome = self.backend.recognize(
            &BackendRequest::new(&self.pixels, None, MAX_CANDIDATES, MAX_BACKEND_TEXT_BYTES),
            &mut normalized,
            operation,
        );
        attempt.checkpoint()?;
        backend_outcome?;
        let regions_outcome = normalized.finish();
        attempt.checkpoint()?;
        let regions = regions_outcome?;
        let result = OcrResult::new(
            self.view.stamp(),
            transform,
            effective_region,
            self.output_space,
            self.descriptor,
            Arc::from(regions),
        );
        attempt.commit(result).map_err(Error::from)
    }
}

/// Validates and maps one exact singular OCR request without running its backend.
///
/// The watch-only source/mapping ceiling is a runtime admission policy, not a
/// new restriction on one-shot recognition.
///
/// # Errors
///
/// Preserves singular operation, selection, geometry, and mapping error order.
#[doc(hidden)]
pub fn prepare(recognizer: &OcrRecognizer, request: OcrRequest<'_>) -> Result<PreparedOcr> {
    let operation = request.operation();
    let mut attempt = Operation::admit(operation)?;
    let descriptor = recognizer.backend.descriptor();
    validate_selection(&descriptor, request.backend(), request.model_identity())?;
    let frame = request.frame();
    let transform = frame.transform();
    let effective_region = resolve_region(transform, request.source_region())?;
    preflight_output(transform, effective_region, request.output_space())?;
    let view = FrameView::new(frame.clone(), effective_region)?;
    let pixels = view.map(descriptor.format(), operation)?;
    attempt.checkpoint()?;
    Ok(PreparedOcr {
        view,
        pixels,
        output_space: request.output_space(),
        descriptor,
        backend: Arc::clone(&recognizer.backend),
        operation: operation.clone(),
    })
}

fn validate_selection(
    descriptor: &OcrBackendDescriptor,
    backend: &OcrBackendIdentity,
    requested: &OcrModelIdentity,
) -> Result<()> {
    if backend != descriptor.backend_identity() {
        return Err(OcrFault::BackendMismatch.into());
    }
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

#[derive(Debug)]
struct ResolvedZones {
    effective: Vec<PixelRect>,
    envelope: PixelRect,
    relative: Vec<PixelRect>,
}

fn resolve_zones(transform: &TransformSnapshot, zones: &[OcrZone]) -> Result<ResolvedZones> {
    if !(1..=MAX_OCR_ZONES).contains(&zones.len()) {
        return Err(OcrFault::ZoneCountOutOfRange.into());
    }

    let mut effective = Vec::with_capacity(zones.len());
    for zone in zones {
        zone.rect().require_non_empty()?;
        effective.push(
            transform
                .resolve_capture_pixels(zone.rect(), zone.policy())
                .map_err(Error::from)?,
        );
    }

    let first = effective[0];
    let mut left = first.left();
    let mut top = first.top();
    let mut right = first.right();
    let mut bottom = first.bottom();
    for zone in &effective[1..] {
        left = left.min(zone.left());
        top = top.min(zone.top());
        right = right.max(zone.right());
        bottom = bottom.max(zone.bottom());
    }
    let envelope = PixelRect::new(left, top, right, bottom).map_err(Error::from)?;

    let mut relative = Vec::with_capacity(effective.len());
    for zone in &effective {
        relative.push(
            PixelRect::new(
                zone.left() - envelope.left(),
                zone.top() - envelope.top(),
                zone.right() - envelope.left(),
                zone.bottom() - envelope.top(),
            )
            .map_err(Error::from)?,
        );
    }

    Ok(ResolvedZones {
        effective,
        envelope,
        relative,
    })
}

fn preflight_mapping_bytes(frame: &Frame, region: PixelRect, format: PixelFormat) -> Result<()> {
    let source = frame.descriptor();
    // Native storage conversion may materialize the complete source before
    // cropping this view, so the source descriptor is part of the preflight.
    enforce_mapping_ceiling(source.byte_len())?;
    let bytes = if region == frame.bounds()? && source.format() == format {
        source.byte_len()
    } else {
        usize::try_from(region.width())
            .ok()
            .and_then(|width| {
                usize::try_from(region.height())
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| {
                usize::try_from(format.bytes_per_pixel())
                    .ok()
                    .and_then(|bytes| pixels.checked_mul(bytes))
            })
            .ok_or_else(|| Error::from(OcrFault::MappingAboveCeiling))?
    };
    enforce_mapping_ceiling(bytes)
}

fn enforce_mapping_ceiling(bytes: usize) -> Result<()> {
    if bytes > MAX_MAPPING_BYTES {
        return Err(OcrFault::MappingAboveCeiling.into());
    }
    Ok(())
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
        let text = normalize_backend(candidate.text(), self.operation)?;
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

#[derive(Debug)]
struct GroupedNormalizedCandidate {
    detector_order: u32,
    membership: u8,
    region: Option<RecognizedRegion>,
}

#[derive(Debug)]
struct GroupedNormalization {
    candidates: Vec<RecognizedRegion>,
    membership_indexes: Vec<u16>,
    group_offsets: [u16; 9],
}

const _: () = {
    assert!(MAX_CANDIDATES * MAX_OCR_ZONES <= MAX_ZONE_MEMBERSHIPS);
    assert!(MAX_CANDIDATES * MAX_BACKEND_TEXT_BYTES <= MAX_AGGREGATE_RAW_TEXT_BYTES);
    assert!(MAX_CANDIDATES * MAX_TEXT_BYTES <= MAX_AGGREGATE_NORMALIZED_TEXT_BYTES);
    assert!(
        MAX_CANDIDATES
            * (size_of::<GroupedNormalizedCandidate>()
                + size_of::<RecognizedRegion>()
                + size_of::<u8>())
            <= MAX_TEMPORARY_GROUPED_BYTES
    );
    assert!(MAX_ZONE_MEMBERSHIPS * size_of::<u16>() <= MAX_MEMBERSHIP_INDEX_BYTES);
    assert!(
        size_of::<OcrZoneScanResult>()
            + MAX_OCR_ZONES * size_of::<PixelRect>()
            + MAX_CANDIDATES * size_of::<RecognizedRegion>()
            + MAX_AGGREGATE_NORMALIZED_TEXT_BYTES
            + MAX_MEMBERSHIP_INDEX_BYTES
            <= MAX_GROUPED_RESULT_BYTES
    );
};

#[derive(Debug)]
struct GroupedNormalizer<'a> {
    operation: &'a OperationContext,
    envelope: PixelRect,
    transform: &'a TransformSnapshot,
    output_space: CoordinateSpace,
    interests: BackendInterests<'a>,
    submitted: usize,
    candidates: Vec<GroupedNormalizedCandidate>,
    fault: Option<Error>,
}

impl<'a> GroupedNormalizer<'a> {
    const fn new(
        operation: &'a OperationContext,
        envelope: PixelRect,
        transform: &'a TransformSnapshot,
        output_space: CoordinateSpace,
        interests: BackendInterests<'a>,
    ) -> Self {
        Self {
            operation,
            envelope,
            transform,
            output_space,
            interests,
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

        let membership = candidate_interest_membership(
            candidate.quadrilateral(),
            self.envelope.extent(),
            self.interests,
        )?;
        let confidence = normalize_confidence(candidate.confidence())?;
        let geometry = normalize_geometry(
            candidate.quadrilateral(),
            self.envelope,
            self.transform,
            self.output_space,
        )?;
        let text = normalize_backend(candidate.text(), self.operation)?;

        let region = if membership == 0 || text.is_empty() {
            None
        } else {
            Some(RecognizedRegion::new(text, geometry, confidence))
        };
        self.submitted += 1;
        self.candidates.push(GroupedNormalizedCandidate {
            detector_order: candidate.detector_order(),
            membership,
            region,
        });
        Ok(())
    }

    fn finish(mut self) -> Result<GroupedNormalization> {
        if let Some(fault) = self.fault {
            return Err(fault);
        }
        if let Some(interruption) = self.operation.interruption() {
            return Err(interruption.into());
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

        let mut candidates = Vec::new();
        let mut memberships = Vec::new();
        for candidate in self.candidates {
            if let Some(region) = candidate.region {
                candidates.push(region);
                memberships.push(candidate.membership);
            }
        }

        let membership_count = memberships.iter().try_fold(0_usize, |count, membership| {
            count.checked_add(membership.count_ones() as usize)
        });
        let membership_count = membership_count
            .filter(|&count| count <= MAX_ZONE_MEMBERSHIPS)
            .ok_or_else(|| Error::from(OcrFault::ZoneMembershipCountAboveCeiling))?;
        let mut membership_indexes = Vec::with_capacity(membership_count);
        let mut group_offsets = [0_u16; 9];
        for group in 0..self.interests.zones().len() {
            if let Some(interruption) = self.operation.interruption() {
                return Err(interruption.into());
            }
            let group_bit = 1_u8 << group;
            for (candidate, membership) in memberships.iter().copied().enumerate() {
                if membership & group_bit != 0 {
                    membership_indexes.push(
                        u16::try_from(candidate)
                            .map_err(|_| Error::from(OcrFault::ZoneMembershipCountAboveCeiling))?,
                    );
                }
            }
            group_offsets[group + 1] = u16::try_from(membership_indexes.len())
                .map_err(|_| Error::from(OcrFault::ZoneMembershipCountAboveCeiling))?;
        }
        let final_offset = u16::try_from(membership_indexes.len())
            .map_err(|_| Error::from(OcrFault::ZoneMembershipCountAboveCeiling))?;
        group_offsets[(self.interests.zones().len() + 1)..].fill(final_offset);

        Ok(GroupedNormalization {
            candidates,
            membership_indexes,
            group_offsets,
        })
    }
}

impl OcrCandidateSink for GroupedNormalizer<'_> {
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
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::{
        BackendCandidate, BackendRequest, MAX_BACKEND_TEXT_BYTES, MAX_MAPPING_BYTES,
        MAX_TEXT_BYTES, OcrBackend, OcrBackendDescriptor, OcrBackendIdentity, OcrCandidateSink,
        OcrRecognizer, OcrRegion, OcrRequest, enforce_mapping_ceiling, normalize_backend,
        normalize_confidence, prepare, validate_relative_quad,
    };
    use crate::{BackendId, BackendVersion, OcrFault, OcrModelIdentity};
    use mado_pilot_capture::{CpuPixels, Frame, FrameDescriptor, FrameStorage, PixelFormat};
    use mado_pilot_core::{
        CancellationToken, ClipPolicy, Clock, CoordinateSpace, GeometryRevision, IdentityIssuer,
        MonotonicInstant, OperationContext, PixelExtent, PixelRect, Rect, Result, Status,
        StreamCursor, TransformSnapshot,
    };

    #[test]
    fn text_is_nfc_normalized_trimmed_and_confidence_is_rounded() {
        let text = normalize_backend("  e\u{301}  ".as_bytes(), &OperationContext::new()).unwrap();
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
        let error = normalize_backend(&[0xff], &OperationContext::new()).unwrap_err();
        assert_eq!(error.status(), Status::VisionFailed);
        let region = PixelRect::new(0, 0, 20, 20).unwrap();
        let error =
            validate_relative_quad([(0.0, 0.0), (10.0, 0.0), (5.0, 5.0), (0.0, 10.0)], region)
                .unwrap_err();
        assert_eq!(error.status(), OcrFault::BackendGeometryInvalid.status());
    }

    #[test]
    fn backend_text_keeps_raw_limits_and_empty_output_separate_from_literal_admission() {
        let context = OperationContext::new();
        let maximum_edges = vec![b' '; MAX_BACKEND_TEXT_BYTES];
        assert!(
            normalize_backend(&maximum_edges, &context)
                .unwrap()
                .is_empty()
        );
        let oversized_edges = vec![b' '; MAX_BACKEND_TEXT_BYTES + 1];
        assert_eq!(
            normalize_backend(&oversized_edges, &context)
                .unwrap_err()
                .status(),
            Status::VisionFailed
        );
        let exact = "e\u{301}".repeat(MAX_TEXT_BYTES / 2);
        assert_eq!(
            &*normalize_backend(exact.as_bytes(), &context).unwrap(),
            "é".repeat(MAX_TEXT_BYTES / 2)
        );
        assert_eq!(
            normalize_backend(format!("{exact}x").as_bytes(), &context)
                .unwrap_err()
                .status(),
            Status::VisionFailed
        );
    }

    #[test]
    fn mapping_ceiling_accepts_exactly_the_limit_and_refuses_one_more_byte() {
        assert!(enforce_mapping_ceiling(MAX_MAPPING_BYTES).is_ok());
        assert_eq!(
            enforce_mapping_ceiling(MAX_MAPPING_BYTES + 1)
                .unwrap_err()
                .status(),
            Status::LimitExceeded
        );
    }

    #[derive(Debug)]
    struct CountedStorage {
        descriptor: FrameDescriptor,
        pixels: Arc<CpuPixels>,
        conversions: Arc<AtomicUsize>,
    }

    impl FrameStorage for CountedStorage {
        fn descriptor(&self) -> FrameDescriptor {
            self.descriptor
        }

        fn cpu_pixels(&self) -> Option<Arc<CpuPixels>> {
            None
        }

        fn read_cpu(&self, _operation: &OperationContext) -> Result<Arc<CpuPixels>> {
            self.conversions.fetch_add(1, Ordering::AcqRel);
            Ok(Arc::clone(&self.pixels))
        }
    }

    #[derive(Debug)]
    struct PixelBackend {
        descriptor: OcrBackendDescriptor,
        calls: Arc<AtomicUsize>,
    }

    impl OcrBackend for PixelBackend {
        fn descriptor(&self) -> OcrBackendDescriptor {
            self.descriptor.clone()
        }

        fn recognize(
            &self,
            request: &BackendRequest<'_>,
            output: &mut dyn OcrCandidateSink,
            _operation: &OperationContext,
        ) -> Result<()> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            let text: &[u8] = if request.pixels().bytes()[0] == 9 {
                b"frame-nine"
            } else {
                b"another-frame"
            };
            output.push(BackendCandidate::new(
                text,
                [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)],
                1.0,
                0,
            ))
        }

        fn close(&self, _operation: &OperationContext) -> Result<()> {
            Ok(())
        }
    }

    fn prepared_fixture() -> (Frame, Arc<AtomicUsize>, Arc<AtomicUsize>, OcrRecognizer) {
        let descriptor =
            FrameDescriptor::packed(PixelExtent::new(4, 3), PixelFormat::Bgra8).unwrap();
        let conversions = Arc::new(AtomicUsize::new(0));
        let storage = Arc::new(CountedStorage {
            descriptor,
            pixels: Arc::new(CpuPixels::new(
                vec![9; descriptor.byte_len()].into_boxed_slice(),
            )),
            conversions: Arc::clone(&conversions),
        });
        let mut cursor = StreamCursor::new(IdentityIssuer::new().issue_stream().unwrap());
        let stamp = cursor.publish(GeometryRevision::FIRST).unwrap();
        let frame = Frame::from_storage(
            stamp,
            MonotonicInstant::ORIGIN,
            TransformSnapshot::frame_only(stamp.geometry(), descriptor.extent()),
            storage,
        )
        .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let backend = Arc::new(PixelBackend {
            descriptor: OcrBackendDescriptor::new(
                OcrBackendIdentity::new(
                    BackendId::new("prepared-ocr-fixture").unwrap(),
                    BackendVersion::new("1").unwrap(),
                ),
                OcrModelIdentity::accepted_bounded_detector(),
                PixelFormat::Bgra8,
            ),
            calls: Arc::clone(&calls),
        });
        (frame, conversions, calls, OcrRecognizer::new(backend))
    }

    #[test]
    fn prepared_execution_keeps_original_source_backend_and_mapping_after_owner_drop() {
        let (frame, conversions, calls, recognizer) = prepared_fixture();
        let stamp = frame.stamp();
        let descriptor = recognizer.descriptor();
        let context = OperationContext::new();
        let work = prepare(
            &recognizer,
            OcrRequest::new(
                &frame,
                descriptor.backend_identity(),
                descriptor.model_identity(),
                OcrRegion::Region {
                    rect: Rect::new(CoordinateSpace::CapturePixels, 1.0, 1.0, 3.0, 3.0).unwrap(),
                    policy: ClipPolicy::Reject,
                },
                CoordinateSpace::CapturePixels,
                &context,
            ),
        )
        .unwrap();
        let retained_mapping = work.pixels().clone();
        drop(frame);
        drop(recognizer);
        drop(context);

        let result = work.execute().unwrap();
        assert_eq!(result.stamp(), stamp);
        assert_eq!(result.backend(), &descriptor);
        assert_eq!(
            result.effective_region(),
            PixelRect::new(1, 1, 3, 3).unwrap()
        );
        assert_eq!(result.regions()[0].text(), "frame-nine");
        assert_eq!(result.regions()[0].geometry().points()[0].x(), 1.0);
        assert_eq!(result.regions()[0].geometry().points()[0].y(), 1.0);
        assert_eq!(retained_mapping.bytes(), &[9; 16]);
        assert_eq!(conversions.load(Ordering::Acquire), 1);
        assert_eq!(calls.load(Ordering::Acquire), 1);
    }

    #[derive(Debug)]
    struct PreparedClock(AtomicUsize);

    impl Clock for PreparedClock {
        fn now(&self) -> MonotonicInstant {
            MonotonicInstant::from_origin(Duration::from_secs(
                u64::try_from(self.0.load(Ordering::Acquire)).unwrap(),
            ))
        }
    }

    #[test]
    fn prepared_execution_keeps_original_interruption_authority_before_backend_entry() {
        for cancelled in [false, true] {
            let (frame, conversions, calls, recognizer) = prepared_fixture();
            let descriptor = recognizer.descriptor();
            let clock = Arc::new(PreparedClock(AtomicUsize::new(0)));
            let token = CancellationToken::new();
            let context = OperationContext::new()
                .with_clock(clock.clone())
                .with_cancellation(token.clone())
                .with_deadline(MonotonicInstant::from_origin(Duration::from_secs(1)));
            let work = prepare(
                &recognizer,
                OcrRequest::new(
                    &frame,
                    descriptor.backend_identity(),
                    descriptor.model_identity(),
                    OcrRegion::FullFrame,
                    CoordinateSpace::CapturePixels,
                    &context,
                ),
            )
            .unwrap();
            clock.0.store(1, Ordering::Release);
            if cancelled {
                token.cancel();
            }
            drop(context);
            assert_eq!(
                work.execute().unwrap_err().status(),
                if cancelled {
                    Status::Cancelled
                } else {
                    Status::DeadlineExceeded
                }
            );
            assert_eq!(calls.load(Ordering::Acquire), 0);
            assert_eq!(conversions.load(Ordering::Acquire), 1);
        }
    }
}
