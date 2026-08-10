//! The wired workflow, against the adapters a host actually gets.
//!
//! The orchestration rules themselves are covered against controlled doubles in
//! `mado-pilot-runtime`. What is only checkable here is the wiring: that the
//! required backend is the one that runs, that the replay adapter and the
//! OpenCV adapter agree about pixels through the mapping the matcher inserts
//! between them, and that a caller reaches all of it through this package
//! without naming a contract package.

use std::path::{Path, PathBuf};

use mado_pilot::replay::{ReplayFrame, ReplaySource, ReplayTarget};
use mado_pilot::{
    ActivityTag, AssetLimits, ClipPolicy, ContentDigest, Continuity, CoordinateSpace,
    DiagnosticDrain, DiagnosticKind, DiagnosticOptions, DiagnosticPayload, Engine, FindRequest,
    Frame, FrameDescriptor, FrameRequest, MatchOptions, MonotonicInstant, OpenRequest,
    OperationContext, PackageSource, PixelFormat, PreparedTemplate, REQUIRED_BACKEND, Rect,
    ReplayEngineRequest, Session, Status,
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
        .prepare_from_package(&package, id, operation)
        .expect("prepared")
}

fn options(template: &PreparedTemplate) -> MatchOptions {
    MatchOptions::from_defaults(template.defaults())
}

/// Returns where the matches were found, in a canonical order of this test's own.
///
/// Sorted, because the order the result reports is not one a test may assert
/// against. Two byte-identical copies correlate at one to within the tolerance,
/// so which of them the result puts first rests on a difference smaller than the
/// tolerance — and that difference is a property of the host's OpenCV build, not
/// of the workflow. The adapter's own algorithm tests already compare this way.
fn origins(result: &mado_pilot::MatchResult) -> Vec<(i32, i32)> {
    let mut origins: Vec<(i32, i32)> = result
        .matches()
        .iter()
        .map(|found| (found.bounds().left(), found.bounds().top()))
        .collect();
    origins.sort_unstable_by_key(|&(left, top)| (top, left));

    origins
}

/// Returns the planted origins in the same canonical order as [`origins`].
fn planted_origins() -> Vec<(i32, i32)> {
    let mut expected = PLANTED.to_vec();
    expected.sort_unstable_by_key(|&(left, top)| (top, left));

    expected
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
        .acquire_frame(&FrameRequest::latest(), &operation)
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
    assert_eq!(origins(outcome.result()), planted_origins());
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
        .acquire_frame(&FrameRequest::latest(), &operation)
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
        .acquire_frame(&FrameRequest::latest(), &operation)
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
        .acquire_frame(&FrameRequest::latest(), &operation)
        .expect("a published frame");
    let second = session
        .acquire_frame(&FrameRequest::newer_than(first.stamp()), &operation)
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
        .acquire_frame(&FrameRequest::latest(), &operation)
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
            .acquire_frame(&FrameRequest::latest(), &operation)
            .expect_err("closed")
            .status(),
        Status::Closed
    );
}

#[test]
fn diagnostics_are_off_by_default_and_enabled_as_one_owned_bounded_stream() {
    let default = engine(1);
    assert!(default.take_diagnostic_reader().is_none());

    let diagnostics = DiagnosticOptions::normal(16).expect("bounded capacity");
    let engine = mado_pilot::replay_engine(
        ReplayEngineRequest::new(scene_source(1)).with_diagnostics(diagnostics),
    )
    .expect("an OpenCV 4 development installation");
    let reader = engine
        .take_diagnostic_reader()
        .expect("enabled diagnostics have one reader");
    assert!(
        engine.take_diagnostic_reader().is_none(),
        "reader ownership is unique"
    );
    assert!(matches!(reader.drain(), DiagnosticDrain::OpenEmpty));

    let tag = ActivityTag::new(0x2a).expect("nonzero");
    let operation = OperationContext::new().with_activity_tag(tag);
    let session = opened(&engine, &operation);
    let frame = session
        .acquire_frame(&FrameRequest::latest(), &operation)
        .expect("captured");
    let template = prepared(&engine, "panel.patch", &operation);
    session
        .find_template(
            &FindRequest::exact(&frame, &template, options(&template)),
            &operation,
        )
        .expect("searched");
    session.close(&operation).expect("closed");
    drop(session);
    drop(engine);

    let batch = match reader.drain() {
        DiagnosticDrain::Batch(batch) => batch,
        other => panic!("expected retained diagnostics, got {other:?}"),
    };
    assert_eq!(
        batch
            .records()
            .iter()
            .map(|record| record.kind())
            .collect::<Vec<_>>(),
        vec![
            DiagnosticKind::Lifecycle,
            DiagnosticKind::Search,
            DiagnosticKind::Lifecycle,
        ]
    );
    assert!(
        batch
            .records()
            .iter()
            .all(|record| record.activity() == Some(tag))
    );
    assert!(
        batch
            .records()
            .windows(2)
            .all(|records| records[0].sequence() < records[1].sequence())
    );
    assert!(batch.losses().is_empty());

    let search = batch
        .records()
        .iter()
        .find_map(|record| match record.payload() {
            DiagnosticPayload::Search(search) => Some(search),
            _ => None,
        })
        .expect("search summary");
    assert_eq!(search.frame, Some(frame.stamp()));
    assert_ne!(search.template.get(), 0);
    assert!(matches!(reader.drain(), DiagnosticDrain::EndOfStream));
}

#[test]
fn an_unknown_template_identity_is_refused_by_the_package_it_was_asked_of() {
    let engine = engine(1);
    let operation = OperationContext::new();
    let package = engine
        .load_package(&PackageSource::directory(package_root()), &operation)
        .expect("loaded");

    let error = engine
        .prepare_from_package(&package, "panel.nothing", &operation)
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

#[test]
fn a_host_can_tighten_the_limits_every_package_it_loads_is_held_to() {
    // The one knob that bounds what an untrusted package may allocate. Before
    // `ReplayEngineRequest` the facade always wired the defaults, so a host
    // could read `Engine::limits` and could not change it.
    let tightened = AssetLimits::default()
        .with_max_entry_count(1)
        .expect("below the implementation ceiling");
    let engine =
        mado_pilot::replay_engine(ReplayEngineRequest::new(scene_source(1)).with_limits(tightened))
            .expect("an OpenCV 4 development installation");
    let operation = OperationContext::new();

    assert_eq!(engine.limits().max_entry_count(), 1);

    // The tracked package has a manifest and two templates, so one entry is not
    // enough and the tightened ceiling is what refuses it.
    let fault = engine
        .load_package(&PackageSource::directory(package_root()), &operation)
        .expect_err("three entries against a ceiling of one");

    assert_eq!(fault.kind(), mado_pilot::AssetFaultKind::ArchiveLimit);
    assert_eq!(fault.status(), Status::LimitExceeded);

    // The default path is unchanged, and is what every other test here uses.
    let default = mado_pilot::replay_engine(scene_source(1)).expect("a default engine");
    assert_eq!(default.limits(), AssetLimits::default());
    assert!(
        default
            .load_package(&PackageSource::directory(package_root()), &operation)
            .is_ok()
    );
}

#[test]
fn every_tracked_slice_fixture_still_hashes_to_its_recorded_checksum() {
    // A benchmark profile's `fixture_sha256` is only evidence if the fixture it
    // names cannot change underneath it. This is the same rule
    // `mado-pilot-assets` applies to the `G-014` set, applied to the package
    // the example, the facade suite, and the benchmark all load.
    let root = package_root();
    let sums = std::fs::read_to_string(root.join("SHA256SUMS")).expect("readable");
    let mut pinned: Vec<String> = Vec::new();

    for line in sums.lines().filter(|line| !line.trim().is_empty()) {
        let (expected, relative) = line
            .split_once("  ")
            .unwrap_or_else(|| panic!("unexpected checksum line: {line}"));
        let relative = relative.trim().trim_start_matches("./");
        let bytes = std::fs::read(root.join(relative))
            .unwrap_or_else(|_| panic!("SHA256SUMS names a missing file: {relative}"));

        assert_eq!(
            ContentDigest::of(&bytes).to_string(),
            expected,
            "{relative} no longer matches the checksum the measurements were taken against"
        );
        pinned.push(relative.to_owned());
    }

    // A fixture added without a checksum would be a file no measurement was
    // ever taken against.
    pinned.sort();
    let mut present = fixture_files(&root, &root);
    present.sort();

    assert_eq!(
        pinned, present,
        "every fixture must be pinned, and every pin must name a fixture"
    );
}

/// Returns every pinned fixture file below `directory`, relative to `root`.
///
/// `SHA256SUMS` cannot pin itself, and the README describes the package rather
/// than being part of it.
fn fixture_files(root: &Path, directory: &Path) -> Vec<String> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(directory).expect("a readable fixture directory") {
        let path = entry.expect("a readable directory entry").path();
        if path.is_dir() {
            found.extend(fixture_files(root, &path));
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .expect("every fixture is below the root")
            .to_str()
            .expect("fixture paths are UTF-8")
            .replace('\\', "/");
        if relative != "SHA256SUMS" && relative != "README.md" {
            found.push(relative);
        }
    }
    found
}
