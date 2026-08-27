//! Waits for stable template presence over replay capture without frame polling.
//!
//! ```text
//! cargo run --locked --package mado-pilot --example template-watch
//! ```

use std::path::PathBuf;
use std::time::Duration;

use mado_pilot::replay::{ReplayFrame, ReplaySource, ReplayTarget};

use mado_pilot::{
    Continuity, FrameDescriptor, MatchOptions, MonotonicInstant, OpenRequest, OperationContext,
    PackageSource, PixelFormat, TemplateStability, TemplateTerminalOutcome, TemplateWatchRequest,
};
use mado_pilot_testkit::match_fixtures;

const SOURCE_FORMAT: PixelFormat = PixelFormat::Rgba8;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine = mado_pilot::replay_engine(scene_source()?)?;
    let setup = OperationContext::new().with_timeout(Duration::from_secs(5))?;
    let target = engine
        .discover(&setup)?
        .into_iter()
        .next()
        .ok_or("the replay source declares a target")?;
    let session = engine.open(target.id(), &OpenRequest::new(), &setup)?;
    let package = engine.load_package(&PackageSource::directory(package_root()), &setup)?;
    let template = engine.prepare_from_package(&package, "panel.patch", &setup)?;

    // Query lifetime and this caller's blocking wait have independent authority.
    // Cancelling or timing out `wait` would not cancel `query`.
    let query_operation = OperationContext::new().with_timeout(Duration::from_secs(5))?;
    let query = session.start_template_watch(
        TemplateWatchRequest::new(
            template.clone(),
            MatchOptions::from_defaults(template.defaults()),
            query_operation,
        )
        .with_stability(TemplateStability::consecutive(2)?),
    )?;
    let wait_operation = OperationContext::new().with_timeout(Duration::from_secs(5))?;
    let outcome = query.wait(&wait_operation)?;

    let TemplateTerminalOutcome::Matched(result) = outcome.as_ref() else {
        return Err(format!("template watcher ended without a match: {outcome:?}").into());
    };
    let stamp = result.frame().stamp();
    println!(
        "stable template {}: {} match(es), stream {:?}, epoch {}, sequence {}, geometry {}",
        result.template(),
        result.result().matches().len(),
        stamp.stream(),
        stamp.epoch().value(),
        stamp.sequence().value(),
        stamp.geometry().value(),
    );
    println!(
        "confirmed observations: {}, searched: {}, backend: {}",
        result.confirmed_observations(),
        result.result().searched(),
        result.result().backend(),
    );

    session.close(&setup)?;
    Ok(())
}

fn scene_source() -> Result<ReplaySource, Box<dyn std::error::Error>> {
    let descriptor = FrameDescriptor::packed(match_fixtures::SCENE, SOURCE_FORMAT)?;
    let pixels = match_fixtures::scene_pixels(SOURCE_FORMAT);
    let first = ReplayFrame::new(
        descriptor,
        MonotonicInstant::ORIGIN,
        Continuity::Continuous,
        None,
        pixels.clone().into_boxed_slice(),
    )?;
    let second = ReplayFrame::new(
        descriptor,
        MonotonicInstant::from_origin(Duration::from_millis(16)),
        Continuity::Continuous,
        None,
        pixels.into_boxed_slice(),
    )?;
    Ok(ReplaySource::from_targets(vec![ReplayTarget::new(
        "stable-panel",
        vec![first, second],
    )?])?)
}

fn package_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/assets/phase1-slice")
}
