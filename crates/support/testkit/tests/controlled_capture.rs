//! The controlled capture double against the contract, and against the paths a
//! finite replay sequence cannot reach.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use mado_pilot_capture::{CaptureProvider, Continuity, FrameRequest, OpenRequest, PixelFormat};
use mado_pilot_core::{CancellationToken, IdentityIssuer, OperationContext, PixelExtent, Status};
use mado_pilot_testkit::{ControlledCapture, capture_contract};

/// Every wait in this file carries a deadline, so a contract regression fails
/// the run instead of hanging it.
fn bounded() -> OperationContext {
    OperationContext::new()
        .with_timeout(Duration::from_secs(10))
        .expect("representable")
}

fn provider() -> Arc<ControlledCapture> {
    Arc::new(
        ControlledCapture::new(
            Arc::new(IdentityIssuer::new()),
            PixelExtent::new(8, 6),
            PixelFormat::Rgba8,
        )
        .expect("built"),
    )
}

#[test]
fn the_double_is_provider_qualified_and_refuses_foreign_targets() {
    let capture = provider();

    capture_contract::discovery_is_provider_qualified(capture.as_ref());
    capture_contract::a_foreign_target_is_refused(capture.as_ref());
}

#[test]
fn a_session_that_exists_before_its_first_frame_still_starts_at_epoch_zero() {
    let capture = provider();
    let operation = bounded();
    let targets = capture.discover(&operation).expect("discovered");
    // Open first, then publish. This is the state a finite replay sequence never
    // occupies, and it is why the shared suite's whole-provider entry point does
    // not apply to this double: that entry point opens its own session, which
    // would have no frames.
    let session = capture
        .open(targets[0].id(), &OpenRequest::new(), &operation)
        .expect("opened");
    capture
        .publish(0x11, Continuity::Continuous)
        .expect("published");

    let frame = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("frame");

    assert_eq!(frame.stamp().epoch().value(), 0);
    assert_eq!(frame.stamp().sequence().value(), 0);
    assert_eq!(frame.stamp().stream(), session.description().stream());
    assert_eq!(
        frame.transform().frame_extent(),
        frame.descriptor().extent()
    );
}

#[test]
fn a_waiting_request_is_satisfied_by_a_later_publication() {
    let capture = provider();
    let operation = bounded();
    let targets = capture.discover(&operation).expect("discovered");
    let session = capture
        .open(targets[0].id(), &OpenRequest::new(), &operation)
        .expect("opened");

    let publisher = Arc::clone(&capture);
    let handle = thread::spawn(move || {
        thread::sleep(Duration::from_millis(10));
        publisher
            .publish(0x22, Continuity::Continuous)
            .expect("published");
    });

    let frame = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("woken by the publication");
    handle.join().expect("publisher finished");

    assert_eq!(frame.stamp().sequence().value(), 0);
    assert!(
        frame
            .map(PixelFormat::Rgba8, &operation)
            .expect("mapped")
            .bytes()
            .iter()
            .all(|byte| *byte == 0x22)
    );
}

#[test]
fn a_wait_cancelled_mid_flight_does_not_become_a_success() {
    let capture = provider();
    let operation = bounded();
    let targets = capture.discover(&operation).expect("discovered");
    let session = capture
        .open(targets[0].id(), &OpenRequest::new(), &operation)
        .expect("opened");
    let token = CancellationToken::new();
    let waiting = bounded().with_cancellation(token.clone());

    let publisher = Arc::clone(&capture);
    let handle = thread::spawn(move || {
        thread::sleep(Duration::from_millis(10));
        token.cancel();
        thread::sleep(Duration::from_millis(10));
        publisher
            .publish(0x33, Continuity::Continuous)
            .expect("published");
    });

    let error = session
        .frame(&FrameRequest::latest(), &waiting)
        .expect_err("cancelled while waiting");
    handle.join().expect("publisher finished");

    assert_eq!(error.status(), Status::Cancelled);
}

#[test]
fn a_wait_with_no_publication_ends_at_its_deadline() {
    let capture = provider();
    let operation = bounded();
    let targets = capture.discover(&operation).expect("discovered");
    let session = capture
        .open(targets[0].id(), &OpenRequest::new(), &operation)
        .expect("opened");
    let bounded = OperationContext::new()
        .with_timeout(Duration::from_millis(20))
        .expect("representable");

    let error = session
        .frame(&FrameRequest::latest(), &bounded)
        .expect_err("nothing was published");

    assert_eq!(error.status(), Status::DeadlineExceeded);
}

#[test]
fn closing_under_a_waiter_ends_the_wait_and_the_session() {
    let capture = provider();
    let operation = bounded();
    let targets = capture.discover(&operation).expect("discovered");
    let session = capture
        .open(targets[0].id(), &OpenRequest::new(), &operation)
        .expect("opened");

    let closing = Arc::clone(&session);
    let handle = thread::spawn(move || {
        thread::sleep(Duration::from_millis(10));
        closing
            .close(&bounded())
            .expect("close finishes once the waiter unwinds");
    });

    let error = session
        .frame(&FrameRequest::latest(), &operation)
        .expect_err("the session closed under the wait");
    handle.join().expect("closer finished");

    assert_eq!(error.status(), Status::Closed);
    assert!(session.is_closed());
    assert_eq!(
        capture
            .publish(0x44, Continuity::Continuous)
            .expect_err("a closed session publishes nothing")
            .status(),
        Status::Closed
    );
}

#[test]
fn a_reshaped_publication_starts_a_later_epoch() {
    let capture = provider();
    let operation = bounded();
    let targets = capture.discover(&operation).expect("discovered");
    let session = capture
        .open(targets[0].id(), &OpenRequest::new(), &operation)
        .expect("opened");
    capture
        .publish(0x55, Continuity::Continuous)
        .expect("published");
    let first = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("frame");

    capture
        .publish_reshaped(PixelExtent::new(16, 12), 0x66)
        .expect("published");
    let reshaped = session
        .frame(&FrameRequest::newer_than(first.stamp()), &operation)
        .expect("frame");

    assert_eq!(reshaped.stamp().epoch().value(), 1);
    assert_eq!(reshaped.stamp().sequence().value(), 0);
    assert_eq!(reshaped.descriptor().extent(), PixelExtent::new(16, 12));

    // The frame taken before the reshape is untouched by it.
    assert_eq!(first.descriptor().extent(), PixelExtent::new(8, 6));
    assert_eq!(first.stamp().epoch().value(), 0);

    session.close(&operation).expect("close succeeds");
    assert_eq!(session.description().stream(), first.stamp().stream());
    assert_eq!(
        capture.discover(&operation).expect("discovered").len(),
        1,
        "closing a session does not change what the provider offers"
    );
}
