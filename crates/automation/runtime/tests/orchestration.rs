//! What the engine decides that no contract below it can.

mod support;

use std::sync::Arc;
use std::time::Duration;

use mado_pilot_runtime::{
    CancellationToken, Continuity, FindRequest, FrameRequest, MatchOptions, OpenRequest,
    OperationContext, PackageSource, PixelFormat, PreparedTemplate, Status,
};
use mado_pilot_testkit::{Behavior, ControlledMatcher, ManualClock, match_fixtures};
use mado_pilot_vision::Candidate;

use support::Harness;

/// Opens one session on a harness that has published one frame.
fn opened(harness: &Harness, operation: &OperationContext) -> mado_pilot_runtime::Session {
    let targets = harness.engine.discover(operation).expect("discovered");
    let session = harness
        .engine
        .open(targets[0].id(), &OpenRequest::new(), operation)
        .expect("opened");
    harness
        .capture
        .publish(0x11, Continuity::Continuous)
        .expect("published");
    session
}

/// Prepares a template through the engine's own backend.
fn prepared(harness: &Harness, operation: &OperationContext) -> PreparedTemplate {
    harness
        .engine
        .prepare(&match_fixtures::planted_template("patch"), operation)
        .expect("prepared")
}

fn options(template: &PreparedTemplate) -> MatchOptions {
    MatchOptions::from_defaults(template.defaults())
}

#[test]
fn an_engine_reports_the_backend_that_will_produce_every_score() {
    let harness = Harness::silent();

    let descriptor = harness.engine.backend();

    assert_eq!(descriptor.id(), "controlled");
    assert_eq!(descriptor.format(), PixelFormat::Rgba8);
}

#[test]
fn a_latest_search_is_correlated_with_the_frame_it_acquired() {
    let harness = Harness::silent();
    let operation = OperationContext::new();
    let session = opened(&harness, &operation);
    let template = prepared(&harness, &operation);

    let outcome = session
        .find_template(
            &FindRequest::latest(&template, options(&template)),
            &operation,
        )
        .expect("searched");

    assert_eq!(outcome.target(), session.target());
    assert_eq!(outcome.result().stamp(), outcome.frame().stamp());
    assert_eq!(outcome.frame().stamp().stream(), session.stream());
    assert_eq!(
        outcome.result().transform(),
        outcome.frame().transform(),
        "the envelope carries the searched frame's own transform"
    );
}

#[test]
fn a_later_publication_does_not_replace_the_frame_a_request_named() {
    let harness = Harness::silent();
    let operation = OperationContext::new();
    let session = opened(&harness, &operation);
    let template = prepared(&harness, &operation);
    let held = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("got the first frame");

    // A newer frame arrives before the search runs. The request named an exact
    // frame, so the answer is about that frame and not about this one.
    harness
        .capture
        .publish(0x22, Continuity::Continuous)
        .expect("published");

    let outcome = session
        .find_template(
            &FindRequest::exact(&held, &template, options(&template)),
            &operation,
        )
        .expect("searched");

    assert_eq!(outcome.result().stamp(), held.stamp());
    assert_eq!(outcome.frame().stamp(), held.stamp());
    assert_eq!(outcome.frame().stamp().sequence().value(), 0);
}

#[test]
fn a_latest_search_after_a_later_publication_uses_the_later_frame() {
    let harness = Harness::silent();
    let operation = OperationContext::new();
    let session = opened(&harness, &operation);
    let template = prepared(&harness, &operation);
    let held = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("got the first frame");
    harness
        .capture
        .publish(0x22, Continuity::Continuous)
        .expect("published");

    let outcome = session
        .find_template(
            &FindRequest::latest(&template, options(&template)),
            &operation,
        )
        .expect("searched");

    assert_ne!(outcome.result().stamp(), held.stamp());
    assert_eq!(outcome.frame().stamp().sequence().value(), 1);
}

#[test]
fn a_frame_another_session_published_is_refused_before_the_backend() {
    let harness = Harness::silent();
    let operation = OperationContext::new();
    let session = opened(&harness, &operation);
    let targets = harness.engine.discover(&operation).expect("discovered");
    let other = harness
        .engine
        .open(targets[0].id(), &OpenRequest::new(), &operation)
        .expect("a second session on the same target");
    harness
        .capture
        .publish(0x33, Continuity::Continuous)
        .expect("published");
    let template = prepared(&harness, &operation);
    let foreign = other
        .frame(&FrameRequest::latest(), &operation)
        .expect("the other session's frame");

    let error = session
        .find_template(
            &FindRequest::exact(&foreign, &template, options(&template)),
            &operation,
        )
        .expect_err("a frame from another stream is not this session's to report on");

    assert_eq!(error.status(), Status::InvalidArgument);
    assert_eq!(
        harness.matcher.find_count(),
        0,
        "the refusal must happen before any backend work"
    );
}

#[test]
fn a_target_another_engine_issued_cannot_be_opened() {
    let harness = Harness::silent();
    let other = Harness::silent();
    let operation = OperationContext::new();
    let foreign = other.engine.discover(&operation).expect("discovered")[0].id();

    let error = harness
        .engine
        .open(foreign, &OpenRequest::new(), &operation)
        .expect_err("another engine's target is not this engine's to open");

    assert_eq!(error.status(), Status::InvalidArgument);
}

#[test]
fn an_already_cancelled_search_never_reaches_the_backend() {
    let harness = Harness::silent();
    let operation = OperationContext::new();
    let session = opened(&harness, &operation);
    let template = prepared(&harness, &operation);
    let token = CancellationToken::new();
    token.cancel();
    let cancelled = OperationContext::new().with_cancellation(token);

    let error = session
        .find_template(
            &FindRequest::latest(&template, options(&template)),
            &cancelled,
        )
        .expect_err("cancelled");

    assert_eq!(error.status(), Status::Cancelled);
    assert_eq!(harness.matcher.find_count(), 0);
}

#[test]
fn a_backend_that_answers_after_cancellation_produces_no_outcome() {
    let token = CancellationToken::new();
    let harness = Harness::new(
        ControlledMatcher::new(PixelFormat::Rgba8)
            .with_candidates(vec![Candidate::new(2, 3, 0.99)])
            .cancelling(token.clone()),
    );
    let operation = OperationContext::new().with_cancellation(token);
    let session = opened(&harness, &operation);
    let template = prepared(&harness, &operation);

    let error = session
        .find_template(
            &FindRequest::latest(&template, options(&template)),
            &operation,
        )
        .expect_err("a late answer is not an outcome");

    assert_eq!(error.status(), Status::Cancelled);
    assert_eq!(
        harness.matcher.find_count(),
        1,
        "the backend ran to the end; what it produced was discarded"
    );
}

#[test]
fn a_deadline_that_passes_during_the_backend_is_not_rescued_by_its_answer() {
    let clock = Arc::new(ManualClock::new());
    let harness = Harness::new(
        ControlledMatcher::new(PixelFormat::Rgba8)
            .with_candidates(vec![Candidate::new(1, 1, 0.95)])
            .with_latency(Arc::clone(&clock), Duration::from_millis(50)),
    );
    let operation = OperationContext::new()
        .with_clock(clock)
        .with_timeout(Duration::from_millis(10))
        .expect("representable");
    let session = opened(&harness, &operation);
    let template = prepared(&harness, &operation);

    let error = session
        .find_template(
            &FindRequest::latest(&template, options(&template)),
            &operation,
        )
        .expect_err("the deadline passed while the backend worked");

    assert_eq!(error.status(), Status::DeadlineExceeded);
    assert_eq!(harness.matcher.find_count(), 1);
}

#[test]
fn a_search_that_finds_nothing_is_a_success_with_the_full_correlation() {
    let harness = Harness::silent();
    let operation = OperationContext::new();
    let session = opened(&harness, &operation);
    let template = prepared(&harness, &operation);

    let outcome = session
        .find_template(
            &FindRequest::latest(&template, options(&template)),
            &operation,
        )
        .expect("nothing found is still an answer");

    assert!(outcome.result().is_empty());
    assert_eq!(outcome.result().stamp(), outcome.frame().stamp());
    assert_eq!(outcome.result().backend().id(), "controlled");
}

#[test]
fn an_outcome_outlives_the_session_the_frame_and_the_template() {
    let harness = Harness::new(
        ControlledMatcher::new(PixelFormat::Rgba8)
            .with_candidates(vec![Candidate::new(4, 5, 0.98)]),
    );
    let operation = OperationContext::new();
    let session = opened(&harness, &operation);
    let template = prepared(&harness, &operation);
    let outcome = session
        .find_template(
            &FindRequest::latest(&template, options(&template)),
            &operation,
        )
        .expect("searched");
    let expected = outcome.result().clone();

    drop(template);
    session.close(&operation).expect("closed");

    assert_eq!(outcome.result(), &expected);
    assert_eq!(outcome.result().matches().len(), 1);
    assert!(
        outcome
            .frame()
            .map(PixelFormat::Rgba8, &operation)
            .expect("the retained frame is still mappable")
            .bytes()
            .iter()
            .all(|byte| *byte == 0x11)
    );
}

#[test]
fn closing_twice_succeeds_and_a_closed_session_publishes_nothing_further() {
    let harness = Harness::silent();
    let operation = OperationContext::new();
    let session = opened(&harness, &operation);

    session.close(&operation).expect("closed");
    session.close(&operation).expect("closing again is a no-op");

    assert!(session.is_closed());
    assert_eq!(
        session
            .frame(&FrameRequest::latest(), &operation)
            .expect_err("closed")
            .status(),
        Status::Closed
    );
}

#[test]
fn a_search_on_a_closed_session_reports_closure_rather_than_a_stale_frame() {
    let harness = Harness::silent();
    let operation = OperationContext::new();
    let session = opened(&harness, &operation);
    let template = prepared(&harness, &operation);
    session.close(&operation).expect("closed");

    let error = session
        .find_template(
            &FindRequest::latest(&template, options(&template)),
            &operation,
        )
        .expect_err("a closed session has no current frame to search");

    assert_eq!(error.status(), Status::Closed);
    assert_eq!(harness.matcher.find_count(), 0);
}

#[test]
fn an_unknown_template_identity_is_refused_before_the_backend() {
    let harness = Harness::silent();
    let operation = OperationContext::new();
    let package = harness
        .engine
        .load_package(
            &PackageSource::directory(support::package_root()),
            &operation,
        )
        .expect("the tracked example package loads");

    let error = harness
        .engine
        .prepare_template(&package, "panel.absent-entirely", &operation)
        .expect_err("the package contains no such template");

    // A package that loaded is valid; asking it for something it never
    // declared is the caller's mistake, so this is an invalid argument rather
    // than an invalid package.
    assert_eq!(error.status(), Status::InvalidArgument);
    assert_eq!(
        harness.matcher.prepare_count(),
        0,
        "an identity the package lacks must not reach the backend"
    );
}

#[test]
fn a_packaged_template_is_prepared_for_the_engines_own_backend() {
    let harness = Harness::silent();
    let operation = OperationContext::new();
    let package = harness
        .engine
        .load_package(
            &PackageSource::directory(support::package_root()),
            &operation,
        )
        .expect("loaded");

    let template = harness
        .engine
        .prepare_template(&package, "panel.patch", &operation)
        .expect("prepared");

    assert_eq!(template.backend().as_str(), harness.engine.backend().id());
    assert_eq!(harness.matcher.prepare_count(), 1);
}

#[test]
fn a_backend_that_cannot_prepare_reports_a_vision_failure() {
    let harness =
        Harness::new(ControlledMatcher::new(PixelFormat::Rgba8).preparing(Behavior::Fail));
    let operation = OperationContext::new();

    let error = harness
        .engine
        .prepare(&match_fixtures::planted_template("patch"), &operation)
        .expect_err("the backend refused");

    assert_eq!(error.status(), Status::VisionFailed);
}
