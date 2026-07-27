//! The capture contract suite every capture adapter must pass.
//!
//! Written as assertions over the public traits rather than over any
//! implementation, so an adapter passes it for the same reasons a caller can
//! rely on it. Each check panics with a message naming the rule it enforces,
//! because a contract failure is a defect in the adapter and not a value to be
//! handled.

use mado_pilot_capture::{CaptureProvider, FrameRequest, OpenRequest, PixelFormat};
use mado_pilot_core::{CancellationToken, FrameOrder, IdentityIssuer, OperationContext, Status};

/// Runs every contract check against `provider`.
///
/// `provider` must offer at least one target and must be able to publish at
/// least one frame without help. An adapter that needs a test to drive
/// publication runs the individual checks instead.
///
/// # Panics
///
/// Panics when the adapter violates the capture contract.
pub fn run(provider: &dyn CaptureProvider) {
    discovery_is_provider_qualified(provider);
    a_foreign_target_is_refused(provider);
    the_first_frame_is_the_start_of_the_stream(provider);
    repeated_latest_requests_return_one_identity(provider);
    derived_outputs_report_the_exact_source_frame(provider);
    an_already_cancelled_request_is_refused(provider);
    close_is_idempotent_and_retained_outputs_survive_it(provider);
}

/// Every discovered target is qualified by the provider that found it.
///
/// # Panics
///
/// Panics when a description names another provider.
pub fn discovery_is_provider_qualified(provider: &dyn CaptureProvider) {
    let operation = OperationContext::new();
    let targets = provider.discover(&operation).expect("discovery succeeds");
    assert!(
        !targets.is_empty(),
        "a provider under contract test must offer a target"
    );
    for target in &targets {
        assert_eq!(
            target.provider(),
            provider.provider(),
            "a target description must be qualified by its own provider"
        );
        assert_eq!(target.id().provider(), provider.provider());
    }
}

/// A target identity from another engine is refused rather than matched by name.
///
/// # Panics
///
/// Panics when the adapter opens a session for a foreign identity.
pub fn a_foreign_target_is_refused(provider: &dyn CaptureProvider) {
    let operation = OperationContext::new();
    let other = IdentityIssuer::new();
    let foreign = other
        .issue_target(provider.provider())
        .expect("identity issued");

    let error = provider
        .open(foreign, &OpenRequest::new(), &operation)
        .expect_err("a foreign identity must not open a session");

    assert_eq!(
        error.status(),
        Status::InvalidArgument,
        "a foreign identity is an invalid argument, not a missing target"
    );
}

/// The first published frame starts the stream at epoch zero, sequence zero.
///
/// # Panics
///
/// Panics when the first frame is numbered differently or its descriptor and
/// pixels disagree.
pub fn the_first_frame_is_the_start_of_the_stream(provider: &dyn CaptureProvider) {
    let operation = OperationContext::new();
    let session = open_first(provider);
    let frame = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("a frame is available");

    assert_eq!(frame.stamp().epoch().value(), 0);
    assert_eq!(frame.stamp().sequence().value(), 0);
    assert_eq!(
        frame.stamp().stream(),
        session.description().stream(),
        "a frame must belong to the stream that published it"
    );
    assert_eq!(
        frame.transform().frame_extent(),
        frame.descriptor().extent(),
        "geometry and pixels must describe the same rectangle"
    );

    let mapping = frame
        .map(frame.descriptor().format(), &operation)
        .expect("a published frame maps");
    assert_eq!(mapping.bytes().len(), frame.descriptor().byte_len());
    session.close(&operation).expect("close succeeds");
}

/// Asking for the latest frame twice returns one identity, not two.
///
/// # Panics
///
/// Panics when the adapter renames a frame it has already published.
pub fn repeated_latest_requests_return_one_identity(provider: &dyn CaptureProvider) {
    let operation = OperationContext::new();
    let session = open_first(provider);

    let first = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("a frame is available");
    let second = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("a frame is available");

    assert_eq!(
        first.stamp(),
        second.stamp(),
        "a maintained frame must not be assigned a new identity per request"
    );
    assert_eq!(first.stamp().order(&second.stamp()), Ok(FrameOrder::Same));
    session.close(&operation).expect("close succeeds");
}

/// A view and a mapping report the frame they came from, not a newer one.
///
/// # Panics
///
/// Panics when a derived output loses or changes its source identity.
pub fn derived_outputs_report_the_exact_source_frame(provider: &dyn CaptureProvider) {
    let operation = OperationContext::new();
    let session = open_first(provider);
    let frame = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("a frame is available");
    let stamp = frame.stamp();

    let view = frame.full_view().expect("the whole frame is a valid view");
    assert_eq!(view.stamp(), stamp);

    let mapping = view
        .map(frame.descriptor().format(), &operation)
        .expect("a view maps");
    assert_eq!(mapping.stamp(), stamp);
    assert_eq!(mapping.transform().geometry(), stamp.geometry());

    session.close(&operation).expect("close succeeds");
}

/// A request whose token is already cancelled never produces a frame.
///
/// # Panics
///
/// Panics when the adapter admits work it should have refused.
pub fn an_already_cancelled_request_is_refused(provider: &dyn CaptureProvider) {
    let session = open_first(provider);
    let token = CancellationToken::new();
    token.cancel();
    let cancelled = OperationContext::new().with_cancellation(token);

    let error = session
        .frame(&FrameRequest::latest(), &cancelled)
        .expect_err("an already cancelled request must not return a frame");

    assert_eq!(error.status(), Status::Cancelled);
    session
        .close(&OperationContext::new())
        .expect("close succeeds");
}

/// Close can be repeated, and what the caller already holds survives it.
///
/// # Panics
///
/// Panics when a second close fails, when a closed session still serves frames,
/// or when a retained frame or mapping is disturbed by close.
pub fn close_is_idempotent_and_retained_outputs_survive_it(provider: &dyn CaptureProvider) {
    let operation = OperationContext::new();
    let session = open_first(provider);
    let frame = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("a frame is available");
    let format = frame.descriptor().format();
    let mapping = frame.map(format, &operation).expect("a frame maps");
    let expected: Vec<u8> = mapping.bytes().to_vec();
    let stamp = frame.stamp();

    session.close(&operation).expect("close succeeds");
    session
        .close(&operation)
        .expect("close is safe to repeat after it has succeeded");
    assert!(session.is_closed());

    let error = session
        .frame(&FrameRequest::latest(), &operation)
        .expect_err("a closed session serves no frames");
    assert_eq!(error.status(), Status::Closed);

    assert_eq!(
        frame.stamp(),
        stamp,
        "close must not disturb a retained frame"
    );
    assert_eq!(
        mapping.bytes(),
        expected.as_slice(),
        "close must not disturb a completed mapping"
    );
    assert!(
        frame.map(PixelFormat::Bgra8, &operation).is_ok(),
        "a retained frame supports frame-local work after its session closes"
    );
}

fn open_first(
    provider: &dyn CaptureProvider,
) -> std::sync::Arc<dyn mado_pilot_capture::CaptureSession> {
    let operation = OperationContext::new();
    let targets = provider.discover(&operation).expect("discovery succeeds");
    let target = targets.first().expect("at least one target");
    provider
        .open(target.id(), &OpenRequest::new(), &operation)
        .expect("the discovered target opens")
}
