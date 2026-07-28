//! The wired workflow, against the adapters a host actually gets.
//!
//! The orchestration rules themselves are covered against controlled doubles in
//! `mado-pilot-runtime`. What is only checkable here is the wiring: that the
//! required backend is the one that runs, that the replay adapter and the
//! OpenCV adapter agree about pixels through the mapping the matcher inserts
//! between them, and that a caller reaches all of it through this package
//! without naming a contract package.

use std::path::PathBuf;

use mado_pilot::replay::{ReplayFrame, ReplaySource, ReplayTarget};
use mado_pilot::{
    ClipPolicy, Continuity, CoordinateSpace, Engine, FindRequest, Frame, FrameDescriptor,
    FrameRequest, MatchOptions, MonotonicInstant, OpenRequest, OperationContext, PackageSource,
    PixelFormat, PreparedTemplate, REQUIRED_BACKEND, Rect, Session, Status,
};
use mado_pilot_testkit::{match_fixtures, png};

/// The layout the replay source publishes, which is not the backend's own.
const SOURCE_FORMAT: PixelFormat = PixelFormat::Rgba8;

/// Where the planted copies of `panel.patch` sit in the scene.
const PLANTED: [(i32, i32); 2] = [(20, 12), (60, 40)];

/// How far two scores may differ and still be the same answer.
///
/// OpenCV normalizes through integral images, so a score carries rounding from
/// arithmetic outside its own window; see
/// `docs/adr/0003-opencv-matching-profile-and-public-score.md`.
const TOLERANCE: f64 = 1e-5;

fn scene_source(frames: usize) -> ReplaySource {
    let descriptor =
        FrameDescriptor::packed(match_fixtures::SCENE, SOURCE_FORMAT).expect("a valid descriptor");
    let pixels = match_fixtures::scene_pixels(SOURCE_FORMAT);
    let replayed = (0..frames)
        .map(|_| {
            ReplayFrame::new(
                descriptor,
                MonotonicInstant::ORIGIN,
                Continuity::Continuous,
                None,
                pixels.clone().into_boxed_slice(),
            )
            .expect("a valid replay frame")
        })
        .collect();

    ReplaySource::from_targets(vec![
        ReplayTarget::new("panel", replayed).expect("a valid target"),
    ])
    .expect("a valid source")
}

fn engine(frames: usize) -> Engine {
    mado_pilot::replay_engine(scene_source(frames)).expect("an OpenCV 4 development installation")
}

fn opened(engine: &Engine, operation: &OperationContext) -> Session {
    let targets = engine.discover(operation).expect("discovered");
    engine
        .open(targets[0].id(), &OpenRequest::new(), operation)
        .expect("opened")
}

fn package_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/assets/phase1-slice")
}

fn prepared(engine: &Engine, id: &str, operation: &OperationContext) -> PreparedTemplate {
    let package = engine
        .load_package(&PackageSource::directory(package_root()), operation)
        .expect("the tracked example package loads");
    engine
        .prepare_template(&package, id, operation)
        .expect("prepared")
}

fn options(template: &PreparedTemplate) -> MatchOptions {
    MatchOptions::from_defaults(template.defaults())
}

fn origins(result: &mado_pilot::MatchResult) -> Vec<(i32, i32)> {
    result
        .matches()
        .iter()
        .map(|found| (found.bounds().left(), found.bounds().top()))
        .collect()
}

#[test]
fn the_engine_requires_and_reports_the_opencv_cpu_backend() {
    let engine = engine(1);

    let descriptor = engine.backend();

    assert_eq!(descriptor.id(), REQUIRED_BACKEND);
    assert!(
        descriptor.version().starts_with("4."),
        "the adapter accepts OpenCV 4 only, and reports which one it linked: {}",
        descriptor.version()
    );
}

#[test]
fn the_tracked_package_still_holds_the_templates_the_scene_was_generated_with() {
    // `panel.patch` is only findable because its bytes are the testkit
    // generator's patch. Asserting it here turns a generator change into a
    // failure instead of into every later result quietly emptying.
    let operation = OperationContext::new();
    let package = engine(1)
        .load_package(&PackageSource::directory(package_root()), &operation)
        .expect("loaded");

    let resolved = package.resolve_template("panel.patch").expect("declared");

    assert_eq!(resolved.extent(), match_fixtures::PATCH);
    assert_eq!(
        resolved.content().to_vec(),
        png::encode_rgb(
            match_fixtures::PATCH.width(),
            match_fixtures::PATCH.height(),
            &match_fixtures::patch_rgb(),
        ),
        "the tracked template no longer matches the scene generator"
    );
}

#[test]
fn the_complete_workflow_finds_the_planted_template_in_the_mapped_frame() {
    let engine = engine(1);
    let operation = OperationContext::new();
    let session = opened(&engine, &operation);
    let frame = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("a published frame");
    let mapping = frame.map(SOURCE_FORMAT, &operation).expect("mapped");
    let template = prepared(&engine, "panel.patch", &operation);

    let outcome = session
        .find_template(
            &FindRequest::exact(&frame, &template, options(&template)),
            &operation,
        )
        .expect("searched");

    assert_eq!(mapping.stamp(), frame.stamp());
    assert_eq!(outcome.target(), session.target());
    assert_eq!(outcome.result().stamp(), frame.stamp());
    assert_eq!(outcome.result().backend().id(), REQUIRED_BACKEND);
    assert_eq!(origins(outcome.result()), PLANTED);
    for found in outcome.result().matches() {
        assert!(
            (found.score() - 1.0).abs() <= TOLERANCE,
            "an exact copy correlates to one within tolerance: {}",
            found.score()
        );
    }
    session.close(&operation).expect("closed");
}

#[test]
fn a_view_scopes_the_search_to_its_own_region_of_its_own_frame() {
    let engine = engine(1);
    let operation = OperationContext::new();
    let session = opened(&engine, &operation);
    let frame = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("a published frame");
    let corner = frame
        .view(
            Rect::new(CoordinateSpace::CapturePixels, 0.0, 0.0, 48.0, 32.0).expect("valid"),
            ClipPolicy::Reject,
        )
        .expect("inside the frame");
    let template = prepared(&engine, "panel.patch", &operation);

    let outcome = session
        .find_template(
            &FindRequest::view(&corner, &template, options(&template)).expect("representable"),
            &operation,
        )
        .expect("searched");

    assert_eq!(outcome.result().searched(), corner.region());
    assert_eq!(
        origins(outcome.result()),
        vec![PLANTED[0]],
        "bounds stay in full-frame capture pixels, and the second copy is outside the view"
    );
}

#[test]
fn a_template_the_frame_does_not_contain_is_a_success_with_no_matches() {
    let engine = engine(1);
    let operation = OperationContext::new();
    let session = opened(&engine, &operation);
    let frame = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("a published frame");
    let template = prepared(&engine, "panel.absent", &operation);

    let outcome = session
        .find_template(
            &FindRequest::exact(&frame, &template, options(&template)),
            &operation,
        )
        .expect("nothing found is an answer, not a failure");

    assert!(outcome.result().is_empty());
    assert_eq!(outcome.result().stamp(), frame.stamp());
    assert_eq!(outcome.result().searched(), frame.bounds().expect("valid"));
}

#[test]
fn a_repeated_workflow_produces_the_same_identities_ordering_and_geometry() {
    let first = run_once();
    let second = run_once();

    assert_eq!(
        first.identities, second.identities,
        "frame identity progression"
    );
    assert_eq!(first.origins, second.origins, "match bounds and ordering");
    for (left, right) in first.scores.iter().zip(&second.scores) {
        assert!(
            (left - right).abs() <= TOLERANCE,
            "scores differ by more than the documented tolerance: {left} vs {right}"
        );
    }
}

/// What one repetition of the workflow is compared on.
///
/// Scores are held separately from everything else because they are the one
/// part compared against a tolerance rather than for equality.
#[derive(Debug)]
struct Run {
    identities: Vec<(u64, u64, u64)>,
    origins: Vec<(i32, i32)>,
    scores: Vec<f64>,
}

/// Runs one whole workflow and returns what determinism is asserted over.
fn run_once() -> Run {
    let engine = engine(2);
    let operation = OperationContext::new();
    let session = opened(&engine, &operation);
    let template = prepared(&engine, "panel.patch", &operation);

    let first = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("a published frame");
    let second = session
        .frame(&FrameRequest::newer_than(first.stamp()), &operation)
        .expect("the replay sequence advances when a consumer asks");
    let outcome = session
        .find_template(
            &FindRequest::exact(&second, &template, options(&template)),
            &operation,
        )
        .expect("searched");
    session.close(&operation).expect("closed");

    Run {
        identities: [&first, &second].map(stamp_values).to_vec(),
        origins: origins(outcome.result()),
        scores: outcome
            .result()
            .matches()
            .iter()
            .map(|found| found.score())
            .collect(),
    }
}

fn stamp_values(frame: &Frame) -> (u64, u64, u64) {
    let stamp = frame.stamp();
    (
        stamp.epoch().value(),
        stamp.sequence().value(),
        stamp.geometry().value(),
    )
}

#[test]
fn a_mapping_and_an_outcome_stay_valid_after_the_session_closes() {
    let engine = engine(1);
    let operation = OperationContext::new();
    let session = opened(&engine, &operation);
    let frame = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("a published frame");
    let template = prepared(&engine, "panel.patch", &operation);
    let mapping = frame.map(SOURCE_FORMAT, &operation).expect("mapped");
    let outcome = session
        .find_template(
            &FindRequest::exact(&frame, &template, options(&template)),
            &operation,
        )
        .expect("searched");
    let expected = mapping.bytes().to_vec();
    let matches = outcome.result().matches().to_vec();

    drop(frame);
    session.close(&operation).expect("closed");
    session.close(&operation).expect("closing again is a no-op");

    assert!(session.is_closed());
    assert_eq!(mapping.bytes(), expected.as_slice());
    assert_eq!(outcome.result().matches(), matches.as_slice());
    assert_eq!(
        session
            .frame(&FrameRequest::latest(), &operation)
            .expect_err("closed")
            .status(),
        Status::Closed
    );
}

#[test]
fn an_unknown_template_identity_is_refused_by_the_package_it_was_asked_of() {
    let engine = engine(1);
    let operation = OperationContext::new();
    let package = engine
        .load_package(&PackageSource::directory(package_root()), &operation)
        .expect("loaded");

    let error = engine
        .prepare_template(&package, "panel.nothing", &operation)
        .expect_err("the package declares no such template");

    assert_eq!(error.status(), Status::InvalidArgument);
}

#[test]
fn a_package_source_that_is_not_a_package_reports_which_rule_it_broke() {
    let engine = engine(1);
    let operation = OperationContext::new();

    let fault = engine
        .load_package(
            &PackageSource::directory(package_root().join("templates")),
            &operation,
        )
        .expect_err("a directory of templates is not a package");

    assert_eq!(fault.kind(), mado_pilot::AssetFaultKind::MissingManifest);
    assert_eq!(fault.stage(), mado_pilot::LoadStage::Manifest);
    assert_eq!(fault.status(), Status::AssetInvalid);
}
