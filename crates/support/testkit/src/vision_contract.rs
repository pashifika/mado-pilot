//! The checks every matching backend must pass, whatever it is.
//!
//! These are the rules that do not depend on what an image contains: identity,
//! ownership, the three successful-empty outcomes, correlation, and the
//! operation contract. A backend cannot satisfy them by being good at matching
//! and cannot fail them for lack of a fixture, which is what makes them
//! runnable against the controlled double and against OpenCV unchanged.
//!
//! What is deliberately *not* here is whether a template is found where it
//! should be. That needs image fixtures and a known answer, so it belongs to
//! each backend's own algorithm tests rather than to a shared suite.
//!
//! Every check takes its own operation context and a caller-supplied backend,
//! so a suite run cannot hang: nothing here waits for anything.

use std::any::Any;
use std::sync::Arc;

use mado_pilot_capture::{Frame, FrameDescriptor, PixelFormat};
use mado_pilot_core::{
    CancellationToken, ClipPolicy, CoordinateSpace, GeometryRevision, IdentityIssuer,
    MonotonicInstant, OperationContext, PixelExtent, Rect, Status, StreamCursor, TransformSnapshot,
};
use mado_pilot_vision::{
    BackendId, MatchBackend, MatchDefaults, MatchOptions, MatchRequest, Matcher, PreparedTemplate,
    RegionSelection, TemplateEncoding, TemplateId, TemplatePayload, TemplateSource,
    TemplateSourceRequest,
};

/// A payload no real backend produces, for the check that a foreign prepared
/// template is refused on its identity before anything touches its state.
#[derive(Debug)]
struct ForeignPayload;

impl TemplatePayload for ForeignPayload {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Builds a frame of `extent` filled with `fill`, in `format`.
///
/// # Panics
///
/// Panics when the extent and format cannot describe a frame, which is a
/// mistake in the calling test rather than a condition to handle.
#[must_use]
pub fn frame(extent: PixelExtent, format: PixelFormat, fill: u8) -> Frame {
    let count = |value: u32| usize::try_from(value).expect("a test frame stays small");
    let bytes = count(extent.width()) * count(extent.height()) * count(format.bytes_per_pixel());

    frame_with_pixels(extent, format, vec![fill; bytes])
}

/// Builds a frame of `extent` from packed `pixels`, in `format`.
///
/// The frame is given a fresh stream, its first epoch, and the first geometry
/// revision, so a test that needs a frame with real content does not have to
/// reproduce the identity plumbing `Frame::new` cross-checks.
///
/// # Panics
///
/// Panics when `pixels` is not exactly the packed length `extent` and `format`
/// require, which is a mistake in the calling test rather than a condition to
/// handle.
#[must_use]
pub fn frame_with_pixels(extent: PixelExtent, format: PixelFormat, pixels: Vec<u8>) -> Frame {
    let issuer = IdentityIssuer::new();
    let mut cursor =
        StreamCursor::new(issuer.issue_stream().expect("an engine can issue a stream"));
    let stamp = cursor
        .publish(GeometryRevision::FIRST)
        .expect("the first frame publishes");
    let stride = usize::try_from(extent.width() * format.bytes_per_pixel())
        .expect("a test frame stays small");
    let descriptor = FrameDescriptor::new(extent, format, stride).expect("a valid descriptor");
    assert_eq!(
        pixels.len(),
        descriptor.byte_len(),
        "a frame's pixels must match its descriptor"
    );

    Frame::new(
        stamp,
        MonotonicInstant::ORIGIN,
        descriptor,
        TransformSnapshot::frame_only(GeometryRevision::FIRST, extent),
        pixels.into_boxed_slice(),
    )
    .expect("a consistent frame")
}

/// Builds a template source of `extent` under `id`.
///
/// The content is a real encoded image rather than a signature, because a
/// backend that decodes template bytes has to be given bytes that decode. Its
/// colour is arbitrary and no check here depends on it: what a backend finds
/// where needs an image fixture with a known answer, which belongs to that
/// backend's own algorithm tests.
///
/// # Panics
///
/// Panics for an identity or extent a template cannot have.
#[must_use]
pub fn template(id: &str, extent: PixelExtent) -> TemplateSource {
    TemplateSource::new(TemplateSourceRequest {
        id: TemplateId::new(id).expect("a non-empty identity"),
        encoding: TemplateEncoding::Png,
        extent,
        space: CoordinateSpace::CapturePixels,
        defaults: MatchDefaults::new(0.5, 8).expect("valid defaults"),
        content: Arc::from(
            crate::png::solid_rgb(extent.width(), extent.height(), [0x40, 0x80, 0xc0]).as_slice(),
        ),
    })
    .expect("a valid template source")
}

fn options() -> MatchOptions {
    MatchOptions::from_defaults(MatchDefaults::new(0.5, 8).expect("valid"))
}

/// A backend's public identity is stable and complete.
///
/// # Panics
///
/// Panics when the descriptor is incomplete or changes between reads.
pub fn descriptor_is_stable(backend: Arc<dyn MatchBackend>) {
    let matcher = Matcher::new(backend);
    let first = matcher.descriptor();
    let second = matcher.descriptor();

    assert_eq!(first, second, "a backend's identity must not change");
    assert!(!first.id().is_empty(), "a backend must identify itself");
    assert!(
        !first.version().is_empty(),
        "a score must be attributable to a version"
    );
}

/// Preparation produces a template belonging to this backend.
///
/// # Panics
///
/// Panics when preparation fails or the prepared template is misattributed.
pub fn preparation_belongs_to_its_backend(backend: Arc<dyn MatchBackend>) {
    let matcher = Matcher::new(backend);
    let descriptor = matcher.descriptor();
    let prepared = matcher
        .prepare(
            &template("t", PixelExtent::new(8, 8)),
            &OperationContext::new(),
        )
        .expect("a valid template prepares");

    assert_eq!(prepared.backend().as_str(), descriptor.id());
    assert_eq!(prepared.id().as_str(), "t");
    assert_eq!(prepared.extent(), PixelExtent::new(8, 8));
}

/// A template another backend prepared is refused before any search runs.
///
/// # Panics
///
/// Panics when the foreign template is accepted or refused with the wrong
/// status.
pub fn a_foreign_prepared_template_is_refused(backend: Arc<dyn MatchBackend>) {
    let matcher = Matcher::new(backend);
    let source = template("t", PixelExtent::new(8, 8));

    // The identity is what must be checked, and it must be checked before the
    // payload is touched: this payload would panic nothing and match nothing,
    // because no backend can downcast it.
    let foreign = PreparedTemplate::new(
        BackendId::new("some-other-backend"),
        &source,
        Arc::new(ForeignPayload),
    );
    let image = frame(PixelExtent::new(64, 64), matcher.descriptor().format(), 0);

    let error = matcher
        .find(
            MatchRequest::new(&image, RegionSelection::FullFrame, &foreign, options()),
            &OperationContext::new(),
        )
        .expect_err("a foreign prepared template is not searchable");

    assert_eq!(error.status(), Status::InvalidArgument);
}

/// A search that begins already cancelled never reaches the backend.
///
/// # Panics
///
/// Panics when the search succeeds or reports the wrong status.
pub fn an_already_cancelled_search_is_refused(backend: Arc<dyn MatchBackend>) {
    let matcher = Matcher::new(backend);
    let prepared = matcher
        .prepare(
            &template("t", PixelExtent::new(8, 8)),
            &OperationContext::new(),
        )
        .expect("prepares");
    let image = frame(PixelExtent::new(64, 64), matcher.descriptor().format(), 0);

    let token = CancellationToken::new();
    token.cancel();
    let context = OperationContext::new().with_cancellation(token);

    let error = matcher
        .find(
            MatchRequest::new(&image, RegionSelection::FullFrame, &prepared, options()),
            &context,
        )
        .expect_err("a cancelled search commits nothing");

    assert_eq!(error.status(), Status::Cancelled);
}

/// A template larger than the search region is a successful empty result.
///
/// # Panics
///
/// Panics when the search fails or reports matches.
pub fn a_template_larger_than_the_region_finds_nothing(backend: Arc<dyn MatchBackend>) {
    let matcher = Matcher::new(backend);
    let prepared = matcher
        .prepare(
            &template("large", PixelExtent::new(128, 128)),
            &OperationContext::new(),
        )
        .expect("prepares");
    let image = frame(PixelExtent::new(64, 64), matcher.descriptor().format(), 0);

    let result = matcher
        .find(
            MatchRequest::new(&image, RegionSelection::FullFrame, &prepared, options()),
            &OperationContext::new(),
        )
        .expect("a well-formed question with the answer 'not there'");

    assert!(result.is_empty());
    assert_eq!(result.stamp(), image.stamp());
}

/// A clip-permitted region that misses the frame is a successful empty result.
///
/// # Panics
///
/// Panics when the search fails or reports matches.
pub fn a_region_that_misses_the_frame_finds_nothing(backend: Arc<dyn MatchBackend>) {
    let matcher = Matcher::new(backend);
    let prepared = matcher
        .prepare(
            &template("t", PixelExtent::new(8, 8)),
            &OperationContext::new(),
        )
        .expect("prepares");
    let image = frame(PixelExtent::new(64, 64), matcher.descriptor().format(), 0);
    let outside = Rect::new(CoordinateSpace::CapturePixels, 500.0, 500.0, 600.0, 600.0)
        .expect("a valid rectangle");

    let result = matcher
        .find(
            MatchRequest::new(
                &image,
                RegionSelection::Region {
                    rect: outside,
                    policy: ClipPolicy::Clip,
                },
                &prepared,
                options(),
            ),
            &OperationContext::new(),
        )
        .expect("clipping to nothing searched nothing");

    assert!(result.is_empty());
    assert_eq!(result.stamp(), image.stamp());
}

/// A result carries the exact source frame's identity and its own transform.
///
/// # Panics
///
/// Panics when correlation is missing or substituted.
pub fn a_result_correlates_with_its_exact_source(backend: Arc<dyn MatchBackend>) {
    let matcher = Matcher::new(backend);
    let prepared = matcher
        .prepare(
            &template("t", PixelExtent::new(8, 8)),
            &OperationContext::new(),
        )
        .expect("prepares");
    let image = frame(PixelExtent::new(64, 48), matcher.descriptor().format(), 0);

    let result = matcher
        .find(
            MatchRequest::new(&image, RegionSelection::FullFrame, &prepared, options()),
            &OperationContext::new(),
        )
        .expect("searches");

    assert_eq!(result.stamp(), image.stamp());
    assert_eq!(result.transform(), image.transform());
    assert_eq!(
        result.searched(),
        image.bounds().expect("a frame has bounds"),
        "a full-frame search reports the whole half-open extent"
    );
    assert_eq!(result.backend().id(), matcher.descriptor().id());
}

/// Runs every backend-independent check against `backend`.
///
/// # Panics
///
/// Panics on the first check the backend does not satisfy.
pub fn run(backend: &Arc<dyn MatchBackend>) {
    descriptor_is_stable(Arc::clone(backend));
    preparation_belongs_to_its_backend(Arc::clone(backend));
    a_foreign_prepared_template_is_refused(Arc::clone(backend));
    an_already_cancelled_search_is_refused(Arc::clone(backend));
    a_template_larger_than_the_region_finds_nothing(Arc::clone(backend));
    a_region_that_misses_the_frame_finds_nothing(Arc::clone(backend));
    a_result_correlates_with_its_exact_source(Arc::clone(backend));
}
