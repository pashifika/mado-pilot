//! The rules every backend's output is put through.
//!
//! Region resolution, thresholding, score validation, canonical ordering,
//! overlap suppression, and the result limit happen here, once, for every
//! backend. Two adapters cannot disagree about what a match *is* if neither of
//! them decides it — the same reason the capture package assigns frame identity
//! rather than letting each adapter do it.
//!
//! Three outcomes that look like failures are deliberately successes with no
//! matches: nothing scored high enough, the template is larger than the region,
//! and a clip-permitted region that misses the frame entirely. A caller asked a
//! well-formed question and the answer is "not there".

use std::sync::Arc;

use mado_pilot_capture::{CpuMapping, Frame, FrameView};
use mado_pilot_core::{
    ClipPolicy, Error, GeometryFault, Operation, OperationContext, PixelExtent, PixelRect, Result,
    TransformSnapshot,
};

use crate::backend::{
    BackendDescriptor, BackendRequest, Candidate, MatchBackend, candidate_bounds,
};
use crate::fault::VisionFault;
use crate::prepared::{PreparedTemplate, PreparedTemplateInstance};
use crate::request::{MatchOptions, MatchRequest, RegionSelection, Suppression};
use crate::result::{Match, MatchResult};
use crate::template::TemplateSource;

/// One exact resolved and mapped matching input.
///
/// This is an internal composition seam for runtimes that must inspect the same
/// mapped bytes before deciding whether backend analysis is required. It owns
/// the source frame and mapping, so a later capture publication cannot change
/// either. It exposes no backend payload.
#[doc(hidden)]
#[derive(Debug)]
pub struct MappedMatch {
    frame: Frame,
    transform: TransformSnapshot,
    searched: PixelRect,
    descriptor: BackendDescriptor,
    template: PreparedTemplateInstance,
    options: MatchOptions,
    pixels: Option<CpuMapping>,
}

impl MappedMatch {
    /// Returns the exact source frame.
    #[doc(hidden)]
    #[must_use]
    pub const fn frame(&self) -> &Frame {
        &self.frame
    }

    /// Returns the resolved capture-pixel region.
    #[doc(hidden)]
    #[must_use]
    pub const fn searched(&self) -> PixelRect {
        self.searched
    }

    /// Returns the backend descriptor used to map the region.
    #[doc(hidden)]
    #[must_use]
    pub const fn descriptor(&self) -> &BackendDescriptor {
        &self.descriptor
    }

    /// Returns the validated matching options.
    #[doc(hidden)]
    #[must_use]
    pub const fn options(&self) -> MatchOptions {
        self.options
    }

    /// Returns the mapped pixels, or `None` for a successful empty search.
    #[doc(hidden)]
    #[must_use]
    pub const fn pixels(&self) -> Option<&CpuMapping> {
        self.pixels.as_ref()
    }

    /// Reports whether `request` has the exact immutable analysis identity.
    #[doc(hidden)]
    #[must_use]
    pub fn is_equivalent_request(&self, request: &MatchRequest<'_>) -> bool {
        if request.frame().stamp() != self.frame.stamp()
            || request.template().backend().as_str() != self.descriptor.id()
            || !self
                .template
                .is_same(&request.template().diagnostic_instance())
            || request.options() != self.options
        {
            return false;
        }
        let Ok(region) = resolve_region(request.frame().transform(), request.selection()) else {
            return false;
        };
        let searched = match region {
            Some(region) => region,
            None => match empty_region() {
                Ok(region) => region,
                Err(_) => return false,
            },
        };
        let has_pixels = !searched.is_empty() && fits(request.template().extent(), searched);
        searched == self.searched && has_pixels == self.pixels.is_some()
    }
}

/// Applies the public matching rules over one backend.
///
/// Cloning shares the backend. A matcher holds no per-request state, so one can
/// serve any number of concurrent searches.
#[derive(Debug, Clone)]
pub struct Matcher {
    backend: Arc<dyn MatchBackend>,
    descriptor: BackendDescriptor,
}

impl Matcher {
    /// Builds a matcher over `backend`.
    #[must_use]
    pub fn new(backend: Arc<dyn MatchBackend>) -> Self {
        let descriptor = backend.descriptor();
        Self {
            backend,
            descriptor,
        }
    }

    /// Returns the backend's public identity.
    #[must_use]
    pub fn descriptor(&self) -> BackendDescriptor {
        self.descriptor.clone()
    }

    /// Reports whether two requests have one exact immutable analysis identity.
    #[doc(hidden)]
    #[must_use]
    pub fn requests_are_equivalent(
        &self,
        left: &MatchRequest<'_>,
        right: &MatchRequest<'_>,
    ) -> bool {
        let descriptor = &self.descriptor;
        if left.frame().stamp() != right.frame().stamp()
            || left.template().backend().as_str() != descriptor.id()
            || right.template().backend().as_str() != descriptor.id()
            || !left
                .template()
                .diagnostic_instance()
                .is_same(&right.template().diagnostic_instance())
            || left.options() != right.options()
        {
            return false;
        }
        let resolved = |request: &MatchRequest<'_>| {
            let region = resolve_region(request.frame().transform(), request.selection()).ok()?;
            let searched = match region {
                Some(region) => region,
                None => empty_region().ok()?,
            };
            Some((
                searched,
                !searched.is_empty() && fits(request.template().extent(), searched),
            ))
        };
        match (resolved(left), resolved(right)) {
            (Some(left), Some(right)) => left == right,
            _ => false,
        }
    }

    /// Compiles a template for this matcher's backend.
    ///
    /// # Errors
    ///
    /// Returns the backend's typed preparation failure, or the operation's
    /// terminal outcome when it is interrupted.
    pub fn prepare(
        &self,
        source: &TemplateSource,
        operation: &OperationContext,
    ) -> Result<PreparedTemplate> {
        Operation::admit(operation)?;
        self.backend.prepare(source, operation)
    }

    /// Searches one exact frame for one prepared template.
    ///
    /// The operation context is checked before admission, after mapping, after
    /// the backend returns, and immediately before the result commits, so a
    /// backend that finished after cancellation won cannot produce an
    /// observable result.
    ///
    /// # Errors
    ///
    /// Returns [`VisionFault::BackendMismatch`] for a template another backend
    /// prepared, a geometry error for a region that cannot be resolved under
    /// its policy, the backend's typed failure, or the operation's terminal
    /// outcome.
    pub fn find(
        &self,
        request: MatchRequest<'_>,
        operation: &OperationContext,
    ) -> Result<MatchResult> {
        let mapped = self.map_match(&request, operation)?;
        self.find_mapped(&mapped, request.template(), operation)
    }

    /// Resolves and maps one request without invoking the backend.
    ///
    /// Runtime orchestration uses this hidden seam to apply an accepted change
    /// policy to the exact bytes the backend would otherwise map again.
    #[doc(hidden)]
    pub fn map_match(
        &self,
        request: &MatchRequest<'_>,
        operation: &OperationContext,
    ) -> Result<MappedMatch> {
        let mut attempt = Operation::admit(operation)?;
        let descriptor = self.descriptor.clone();
        if request.template().backend().as_str() != descriptor.id() {
            return Err(VisionFault::BackendMismatch.into());
        }

        let frame = request.frame();
        let transform = *frame.transform();
        let searched = match resolve_region(&transform, request.selection())? {
            Some(region) => region,
            None => empty_region()?,
        };
        let pixels = if searched.is_empty() || !fits(request.template().extent(), searched) {
            None
        } else {
            let view = FrameView::new(frame.clone(), searched)?;
            let pixels = view.map(descriptor.format(), operation)?;
            attempt.checkpoint()?;
            Some(pixels)
        };
        let mapped = MappedMatch {
            frame: frame.clone(),
            transform,
            searched,
            descriptor,
            template: request.template().diagnostic_instance(),
            options: request.options(),
            pixels,
        };
        attempt.commit(mapped).map_err(Error::from)
    }

    /// Runs backend matching over one exact mapped request.
    ///
    /// The prepared-template instance and backend are revalidated so a mapped
    /// input cannot be paired with different compiled state.
    #[doc(hidden)]
    pub fn find_mapped(
        &self,
        mapped: &MappedMatch,
        template: &PreparedTemplate,
        operation: &OperationContext,
    ) -> Result<MatchResult> {
        let mut attempt = Operation::admit(operation)?;
        let descriptor = &self.descriptor;
        if template.backend().as_str() != descriptor.id()
            || descriptor != &mapped.descriptor
            || !mapped.template.is_same(&template.diagnostic_instance())
        {
            return Err(VisionFault::BackendMismatch.into());
        }

        let Some(pixels) = mapped.pixels.as_ref() else {
            return commit(
                attempt,
                &mapped.frame,
                &mapped.transform,
                mapped.searched,
                &mapped.descriptor,
                mapped.options,
                Vec::new(),
            );
        };
        let candidates = self.backend.find(
            &BackendRequest {
                template,
                pixels,
                region: mapped.searched,
                options: mapped.options,
            },
            operation,
        )?;
        attempt.checkpoint()?;
        let matches = normalize(candidates, template, mapped.searched, mapped.options)?;
        commit(
            attempt,
            &mapped.frame,
            &mapped.transform,
            mapped.searched,
            &mapped.descriptor,
            mapped.options,
            matches,
        )
    }
}

/// Resolves a selection into the region to search.
///
/// `Ok(None)` means a clip-permitted region that does not overlap the frame at
/// all, which is a successful search of nothing rather than a bad request. The
/// same miss under a rejecting policy stays an error, because a caller that
/// said "reject" asked to be told.
fn resolve_region(
    transform: &TransformSnapshot,
    selection: RegionSelection,
) -> Result<Option<PixelRect>> {
    match selection {
        RegionSelection::FullFrame => Ok(Some(transform.frame_bounds()?)),
        RegionSelection::Region { rect, policy } => {
            rect.require_non_empty()?;
            match transform.resolve_capture_pixels(rect, policy) {
                Ok(region) => Ok(Some(region)),
                Err(GeometryFault::OutsideExtent) if policy == ClipPolicy::Clip => Ok(None),
                Err(fault) => Err(fault.into()),
            }
        }
    }
}

/// A degenerate region at the frame's origin, for a search that had nothing to
/// look at. A result must still report *some* searched region, and an empty one
/// at the origin says "nothing" without implying a location.
fn empty_region() -> Result<PixelRect> {
    PixelRect::new(0, 0, 0, 0).map_err(Error::from)
}

fn fits(template: PixelExtent, region: PixelRect) -> bool {
    template.width() <= region.width() && template.height() <= region.height()
}

/// Turns raw candidates into the canonical public collection.
fn normalize(
    candidates: Vec<Candidate>,
    template: &PreparedTemplate,
    region: PixelRect,
    options: MatchOptions,
) -> Result<Vec<Match>> {
    let mut kept: Vec<Match> = Vec::with_capacity(candidates.len());

    for candidate in candidates {
        let score = candidate.score();
        if !score.is_finite() || !(0.0..=1.0).contains(&score) {
            return Err(VisionFault::BackendScoreOutOfRange.into());
        }
        let bounds = candidate_bounds(region, candidate, template.extent())
            .ok_or(VisionFault::BackendCandidateOutsideRegion)?;
        if score < options.min_score() {
            continue;
        }
        kept.push(Match::new(template.id().clone(), bounds, score));
    }

    kept.sort_by(Match::canonical_order);

    if options.suppression() == Suppression::DropOverlapping {
        let mut survivors: Vec<Match> = Vec::with_capacity(kept.len());
        for found in kept {
            let overlaps = survivors.iter().any(|survivor| {
                survivor
                    .bounds()
                    .intersect(found.bounds())
                    .is_some_and(|shared| !shared.is_empty())
            });
            if !overlaps {
                survivors.push(found);
            }
        }
        kept = survivors;
    }

    kept.truncate(usize::try_from(options.max_results()).unwrap_or(usize::MAX));
    Ok(kept)
}

fn commit(
    attempt: Operation<'_>,
    frame: &Frame,
    transform: &TransformSnapshot,
    searched: PixelRect,
    descriptor: &BackendDescriptor,
    options: MatchOptions,
    matches: Vec<Match>,
) -> Result<MatchResult> {
    let result = MatchResult::new(
        frame.stamp(),
        *transform,
        searched,
        descriptor.clone(),
        options,
        matches,
    );
    attempt.commit(result).map_err(Error::from)
}

#[cfg(test)]
mod tests {
    use std::any::Any;
    use std::sync::Arc;

    use mado_pilot_capture::{Frame, FrameDescriptor, PixelFormat};
    use mado_pilot_core::{
        ClipPolicy, CoordinateSpace, Error, GeometryFault, GeometryRevision, IdentityIssuer,
        MonotonicInstant, OperationContext, PixelExtent, PixelRect, Rect, Result, Status,
        StreamCursor, TransformSnapshot,
    };

    use super::{Matcher, fits, resolve_region};
    use crate::backend::{BackendDescriptor, BackendRequest, MatchBackend, TemplatePayload};
    use crate::prepared::{BackendId, PreparedTemplate};
    use crate::request::{MatchOptions, MatchRequest, RegionSelection};
    use crate::template::{
        MatchDefaults, TemplateEncoding, TemplateId, TemplateSource, TemplateSourceRequest,
    };

    const BACKEND_ID: &str = "matcher-contract";

    #[derive(Debug)]
    struct Payload;

    impl TemplatePayload for Payload {
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    #[derive(Debug)]
    struct NoSearchBackend;

    impl MatchBackend for NoSearchBackend {
        fn descriptor(&self) -> BackendDescriptor {
            BackendDescriptor::new(BACKEND_ID, "1", PixelFormat::Rgba8)
        }

        fn prepare(
            &self,
            source: &TemplateSource,
            _operation: &OperationContext,
        ) -> Result<PreparedTemplate> {
            Ok(PreparedTemplate::new(
                BackendId::new(BACKEND_ID),
                source,
                Arc::new(Payload),
            ))
        }

        fn find(
            &self,
            _request: &BackendRequest<'_>,
            _operation: &OperationContext,
        ) -> Result<Vec<crate::backend::Candidate>> {
            panic!("a search with no pixels must not reach the backend")
        }
    }

    fn frame() -> Frame {
        let extent = PixelExtent::new(64, 64);
        let format = PixelFormat::Rgba8;
        let descriptor = FrameDescriptor::packed(extent, format).expect("valid descriptor");
        let issuer = IdentityIssuer::new();
        let mut cursor =
            StreamCursor::new(issuer.issue_stream().expect("an engine can issue a stream"));
        let stamp = cursor
            .publish(GeometryRevision::FIRST)
            .expect("the first frame publishes");

        Frame::new(
            stamp,
            MonotonicInstant::ORIGIN,
            descriptor,
            TransformSnapshot::frame_only(GeometryRevision::FIRST, extent),
            vec![0; descriptor.byte_len()].into_boxed_slice(),
        )
        .expect("a consistent frame")
    }

    fn template(extent: PixelExtent) -> TemplateSource {
        TemplateSource::new(TemplateSourceRequest {
            id: TemplateId::new("t").expect("non-empty"),
            encoding: TemplateEncoding::Png,
            extent,
            space: CoordinateSpace::CapturePixels,
            defaults: MatchDefaults::new(0.5, 8).expect("valid defaults"),
            content: Arc::from([0x89].as_slice()),
        })
        .expect("a valid template source")
    }

    fn options() -> MatchOptions {
        MatchOptions::from_defaults(MatchDefaults::new(0.5, 8).expect("valid defaults"))
    }

    fn search(
        selection: RegionSelection,
        template_extent: PixelExtent,
    ) -> Result<crate::result::MatchResult> {
        let matcher = Matcher::new(Arc::new(NoSearchBackend));
        let prepared = matcher.prepare(&template(template_extent), &OperationContext::new())?;
        let frame = frame();

        matcher.find(
            MatchRequest::new(&frame, selection, &prepared, options()),
            &OperationContext::new(),
        )
    }

    fn snapshot() -> TransformSnapshot {
        TransformSnapshot::frame_only(GeometryRevision::FIRST, PixelExtent::new(200, 100))
    }

    fn region(space: CoordinateSpace, values: (f64, f64, f64, f64)) -> Rect {
        Rect::new(space, values.0, values.1, values.2, values.3).expect("valid")
    }

    #[test]
    fn a_full_frame_selection_is_the_whole_half_open_extent() {
        let resolved = resolve_region(&snapshot(), RegionSelection::FullFrame)
            .expect("resolvable")
            .expect("present");

        assert_eq!(resolved, PixelRect::new(0, 0, 200, 100).expect("valid"));
    }

    #[test]
    fn an_in_bounds_region_resolves_to_itself() {
        let selection = RegionSelection::Region {
            rect: region(CoordinateSpace::CapturePixels, (10.0, 20.0, 60.0, 70.0)),
            policy: ClipPolicy::Reject,
        };
        let resolved = resolve_region(&snapshot(), selection)
            .expect("resolvable")
            .expect("present");

        assert_eq!(resolved, PixelRect::new(10, 20, 60, 70).expect("valid"));
    }

    #[test]
    fn a_normalized_region_resolves_through_the_frames_own_snapshot() {
        let selection = RegionSelection::Region {
            rect: region(CoordinateSpace::FrameNormalized, (0.0, 0.0, 0.5, 0.5)),
            policy: ClipPolicy::Reject,
        };
        let resolved = resolve_region(&snapshot(), selection)
            .expect("resolvable")
            .expect("present");

        assert_eq!(resolved, PixelRect::new(0, 0, 100, 50).expect("valid"));
    }

    #[test]
    fn a_partly_outside_region_clips_when_the_policy_permits_it() {
        let selection = RegionSelection::Region {
            rect: region(CoordinateSpace::CapturePixels, (150.0, 50.0, 400.0, 300.0)),
            policy: ClipPolicy::Clip,
        };
        let resolved = resolve_region(&snapshot(), selection)
            .expect("resolvable")
            .expect("present");

        assert_eq!(resolved, PixelRect::new(150, 50, 200, 100).expect("valid"));
    }

    #[test]
    fn a_region_that_misses_entirely_is_nothing_to_search_rather_than_a_bad_request() {
        let selection = RegionSelection::Region {
            rect: region(CoordinateSpace::CapturePixels, (500.0, 500.0, 600.0, 600.0)),
            policy: ClipPolicy::Clip,
        };

        assert_eq!(
            resolve_region(&snapshot(), selection).expect("resolvable"),
            None
        );
    }

    #[test]
    fn the_same_miss_under_a_rejecting_policy_stays_an_error() {
        let selection = RegionSelection::Region {
            rect: region(CoordinateSpace::CapturePixels, (500.0, 500.0, 600.0, 600.0)),
            policy: ClipPolicy::Reject,
        };

        assert!(resolve_region(&snapshot(), selection).is_err());
    }

    #[test]
    fn an_explicit_empty_region_is_invalid_geometry() {
        let empty = region(CoordinateSpace::CapturePixels, (10.0, 20.0, 10.0, 40.0));
        let error = search(
            RegionSelection::Region {
                rect: empty,
                policy: ClipPolicy::Clip,
            },
            PixelExtent::new(8, 8),
        )
        .expect_err("an explicit empty ROI is not a search of nothing");

        assert_eq!(error, Error::from(GeometryFault::EmptyRegion));
        assert_eq!(error.status(), Status::InvalidArgument);
    }

    #[test]
    fn a_clip_permitted_non_intersection_is_a_successful_empty_result() {
        let outside = region(CoordinateSpace::CapturePixels, (500.0, 500.0, 600.0, 600.0));
        let result = search(
            RegionSelection::Region {
                rect: outside,
                policy: ClipPolicy::Clip,
            },
            PixelExtent::new(8, 8),
        )
        .expect("clipping a non-empty ROI to no intersection searches nothing");

        assert!(result.is_empty());
    }

    #[test]
    fn a_template_larger_than_the_region_is_a_successful_empty_result() {
        let result = search(RegionSelection::FullFrame, PixelExtent::new(65, 64))
            .expect("a well-formed search can have no valid template placement");

        assert!(result.is_empty());
    }

    #[test]
    fn a_template_larger_than_the_region_does_not_fit() {
        let region = PixelRect::new(0, 0, 20, 20).expect("valid");

        assert!(fits(PixelExtent::new(20, 20), region));
        assert!(fits(PixelExtent::new(1, 1), region));
        assert!(!fits(PixelExtent::new(21, 20), region));
        assert!(!fits(PixelExtent::new(20, 21), region));
    }
}
