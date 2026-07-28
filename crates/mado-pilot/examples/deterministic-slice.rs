//! The complete deterministic Phase 1 workflow, end to end.
//!
//! Configure replay capture and require the OpenCV CPU backend, discover a
//! target, open it, take one frame, view and map it, load an asset package from
//! disk, prepare two templates, search that exact frame for both, and close.
//!
//! ```text
//! cargo run --locked --package mado-pilot --example deterministic-slice
//! ```
//!
//! The inputs are deterministic in two different ways, on purpose. The scene is
//! generated from integer arithmetic by `mado-pilot-testkit`, because its
//! construction parameters say everything a tracked 24 KiB of raw pixels would:
//! a patch planted at a stated offset is found at that offset. The asset
//! package is tracked on disk, because what package loading has to be shown
//! doing — reading a manifest, normalizing paths, verifying content digests —
//! is only exercised by real bytes.
//!
//! Every name this program uses is provisional until gate `G-009` is resolved.
//! Reviewing them is part of what the example is for.

use std::path::PathBuf;

use mado_pilot::replay::{ReplayFrame, ReplaySource, ReplayTarget};
use mado_pilot::{
    ClipPolicy, Continuity, CoordinateSpace, FindOutcome, FindRequest, FrameDescriptor,
    FrameRequest, MatchOptions, MonotonicInstant, OpenRequest, OperationContext, PackageSource,
    PixelFormat, REQUIRED_BACKEND, Rect, Status,
};
use mado_pilot_testkit::match_fixtures;

/// The pixel layout the replay source publishes.
///
/// Deliberately not the backend's own layout: the matcher maps a searched
/// region into whatever the backend declared, and a workflow that only worked
/// when the two already agreed would be hiding that step.
const SOURCE_FORMAT: PixelFormat = PixelFormat::Rgba8;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Configure replay capture and require the OpenCV CPU backend. An
    //    unusable OpenCV fails here, with no engine left behind and nothing
    //    else silently substituted for it.
    let engine = mado_pilot::replay_engine(scene_source()?)?;
    let backend = engine.backend();
    println!("backend: {backend} required {REQUIRED_BACKEND}");
    println!("backend pixel format: {}", backend.format());

    // Every operation that can block carries a deadline and cancellation. This
    // one has neither, which is a statement rather than an omission: replay
    // publishes synchronously and this program has nothing to cancel.
    let operation = OperationContext::new();

    // 2. Discover targets and open one.
    let targets = engine.discover(&operation)?;
    for target in &targets {
        println!(
            "target: {} {} {}",
            target.name(),
            target.extent(),
            target.format()
        );
    }
    let target = targets
        .first()
        .ok_or("the replay source declares a target")?;
    let session = engine.open(target.id(), &OpenRequest::new(), &operation)?;
    println!("session: stream {:?}", session.stream());

    // 3. Take one frame and hold it. Everything below searches this exact
    //    frame, not whatever the session publishes later.
    let frame = session.frame(&FrameRequest::latest(), &operation)?;
    let stamp = frame.stamp();
    println!(
        "frame: epoch {} sequence {} geometry {} {}",
        stamp.epoch().value(),
        stamp.sequence().value(),
        stamp.geometry().value(),
        frame.descriptor()
    );

    // 4. Map it. A mapping is CPU-readable bytes the caller owns, and it stays
    //    readable after the session is gone.
    let whole = frame.map(SOURCE_FORMAT, &operation)?;
    println!(
        "mapped whole frame: {} bytes, shared with the frame: {}",
        whole.bytes().len(),
        whole.is_shared()
    );

    // A view is a region of one exact frame. Mapping it copies only that
    // region and reports the same complete source identity.
    let corner = frame.view(
        Rect::new(CoordinateSpace::CapturePixels, 0.0, 0.0, 48.0, 32.0)?,
        ClipPolicy::Reject,
    )?;
    let mapped_corner = corner.map(SOURCE_FORMAT, &operation)?;
    println!(
        "mapped view {}: {} bytes, same source frame: {}",
        mapped_corner.region(),
        mapped_corner.bytes().len(),
        mapped_corner.stamp() == stamp
    );

    // 5. Load the asset package and prepare its templates.
    let package = engine.load_package(&PackageSource::directory(package_root()), &operation)?;
    println!(
        "package: {} {} under {}, {} templates",
        package.manifest().package_id(),
        package.manifest().package_version(),
        package.manifest().license(),
        package.template_count()
    );

    let present = engine.prepare_template(&package, "panel.patch", &operation)?;
    let absent = engine.prepare_template(&package, "panel.absent", &operation)?;

    // 6. Search that exact frame. Three searches, three different answers.
    let found = session.find_template(
        &FindRequest::exact(
            &frame,
            &present,
            MatchOptions::from_defaults(present.defaults()),
        ),
        &operation,
    )?;
    report("whole frame, template present", &found);

    let in_view = session.find_template(
        &FindRequest::view(
            &corner,
            &present,
            MatchOptions::from_defaults(present.defaults()),
        )?,
        &operation,
    )?;
    report("corner view, template present", &in_view);

    // Nothing found is a successful answer to a well-formed question, not a
    // failure, and it still carries the complete source correlation.
    let missing = session.find_template(
        &FindRequest::exact(
            &frame,
            &absent,
            MatchOptions::from_defaults(absent.defaults()),
        ),
        &operation,
    )?;
    report("whole frame, template absent", &missing);
    if missing.result().is_empty() {
        println!("  the absent template is not on this frame");
    }

    // 7. Close. What the caller owns survives it.
    session.close(&operation)?;
    session.close(&operation)?;
    println!("closed: {}", session.is_closed());

    let after_close = session
        .frame(&FrameRequest::latest(), &operation)
        .expect_err("a closed session publishes nothing further");
    println!("after close: {}", after_close.status());
    assert_eq!(after_close.status(), Status::Closed);

    println!(
        "mapping still readable after close: {} bytes",
        whole.bytes().len()
    );
    println!(
        "result still correlated after close: sequence {}",
        found.result().stamp().sequence().value()
    );

    Ok(())
}

/// Prints one outcome, including what it is correlated with.
fn report(what: &str, outcome: &FindOutcome) {
    let result = outcome.result();
    println!(
        "{what}: {} match(es) in {} by {}",
        result.matches().len(),
        result.searched(),
        result.backend()
    );
    for found in result.matches() {
        println!(
            "  {} at {} score {:.6}",
            found.template(),
            found.bounds(),
            found.score()
        );
    }
}

/// Builds a one-frame replay source from the tracked matching scene.
fn scene_source() -> Result<ReplaySource, Box<dyn std::error::Error>> {
    let descriptor = FrameDescriptor::packed(match_fixtures::SCENE, SOURCE_FORMAT)?;
    let frame = ReplayFrame::new(
        descriptor,
        MonotonicInstant::ORIGIN,
        Continuity::Continuous,
        None,
        match_fixtures::scene_pixels(SOURCE_FORMAT).into_boxed_slice(),
    )?;

    Ok(ReplaySource::from_targets(vec![ReplayTarget::new(
        "panel",
        vec![frame],
    )?])?)
}

/// Returns the tracked example package directory.
fn package_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/assets/phase1-slice")
}
