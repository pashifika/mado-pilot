//! Capture-free OpenCV watcher authority and owner-lifetime oracle.

use std::sync::Arc;
use std::time::{Duration, Instant};

use mado_pilot::{
    ChangeDetectionPolicy, ClipPolicy, Continuity, MatchDefaults, MatchOptions, OpenRequest,
    OperationContext, PixelFormat, PixelRect, RegionSelection, Suppression, TemplateQuery,
    TemplateQueryOutcome, TemplateQueryProgress, TemplateStability, TemplateTerminalOutcome,
    TemplateWatchRequest, TemplateWorkDisposition,
};
use mado_pilot_backend_opencv::{OpenCvBackend, benchmark_instrumentation};
use mado_pilot_runtime::{
    CaptureProvider, Engine, EngineWiring, IdentityIssuer, Matcher, PackageLoader,
};
use mado_pilot_testkit::{ControlledCapture, match_fixtures};

const WAIT: Duration = Duration::from_secs(5);
const FORMAT: PixelFormat = PixelFormat::Rgba8;
const CACHE_LIMIT: u64 = 33_554_432;

pub(super) fn run(analysis_always: bool) -> Result<bool, String> {
    let before = benchmark_instrumentation::snapshot();
    let retained_before = benchmark_instrumentation::work_snapshot().bgr_retained_bytes;
    let issuer = Arc::new(IdentityIssuer::new());
    let capture = Arc::new(
        ControlledCapture::new(Arc::clone(&issuer), match_fixtures::SCENE, FORMAT)
            .map_err(|error| error.to_string())?,
    );
    let engine = Engine::new(EngineWiring {
        engine: issuer.engine(),
        capture: Arc::clone(&capture) as Arc<dyn CaptureProvider>,
        matcher: Matcher::new(Arc::new(
            OpenCvBackend::new().map_err(|error| error.to_string())?,
        )),
        loader: PackageLoader::new(),
        ocr: None,
        input: None,
        permission: None,
    })
    .map_err(|error| error.to_string())?;
    let target = engine
        .discover(&operation())
        .map_err(|error| error.to_string())?
        .first()
        .ok_or("controlled replay target missing")?
        .id();
    let session = engine
        .open(target, &OpenRequest::new(), &operation())
        .map_err(|error| error.to_string())?;
    let present = engine
        .prepare_template(
            &match_fixtures::planted_template("watch-present"),
            &operation(),
        )
        .map_err(|error| error.to_string())?;
    let absent = engine
        .prepare_template(
            &match_fixtures::absent_template("watch-absent"),
            &operation(),
        )
        .map_err(|error| error.to_string())?;
    let policy = if analysis_always {
        ChangeDetectionPolicy::AnalysisAlways
    } else {
        ChangeDetectionPolicy::ExactRgba
    };
    let options = MatchOptions::from_defaults(MatchDefaults::new(0.95, 1).expect("match defaults"))
        .with_suppression(Suppression::DropOverlapping);
    let roi = PixelRect::new(16, 8, 48, 32).expect("fixed ROI");
    let region = RegionSelection::pixels(roi, ClipPolicy::Reject).expect("ROI selection");
    let query = session
        .start_template_watch(
            TemplateWatchRequest::new(present, options, operation())
                .with_region(region)
                .with_change_policy(policy)
                .with_stability(TemplateStability::consecutive(2).expect("two confirmations")),
        )
        .map_err(|error| error.to_string())?;
    capture
        .publish_pixels(
            &match_fixtures::background_pixels(FORMAT),
            Continuity::Continuous,
        )
        .map_err(|error| error.to_string())?;
    let nonmatch = progress(&query, 1, 0)?;
    let scene = match_fixtures::scene_pixels(FORMAT);
    capture
        .publish_pixels(&scene, Continuity::Continuous)
        .map_err(|error| error.to_string())?;
    let first = progress(&query, 2, 1)?;

    // This distinct template maps the same current frame after the first analysis.
    let other = session
        .start_template_watch(
            TemplateWatchRequest::new(absent, options, operation())
                .with_region(region)
                .with_change_policy(policy),
        )
        .map_err(|error| error.to_string())?;
    let other_progress = progress(&other, 1, 0)?;
    let cancelled = other.cancel();
    let cancellation_correct = matches!(cancelled.as_ref(), TemplateTerminalOutcome::Cancelled)
        && matches!(other.poll(), TemplateQueryOutcome::Terminal(outcome)
            if matches!(outcome.as_ref(), TemplateTerminalOutcome::Cancelled));
    capture
        .publish_pixels(&scene, Continuity::Continuous)
        .map_err(|error| error.to_string())?;
    let outcome = query
        .wait(&operation())
        .map_err(|error| error.to_string())?;
    session
        .close(&operation())
        .map_err(|error| error.to_string())?;
    session
        .close(&operation())
        .map_err(|error| error.to_string())?;

    let retained = benchmark_instrumentation::work_snapshot().bgr_retained_bytes;
    let result_correct = match outcome.as_ref() {
        TemplateTerminalOutcome::Matched(result) => {
            let matches = result.result().matches();
            result.frame().stamp().sequence().value() == 2
                && result.result().stamp() == result.frame().stamp()
                && result.result().searched() == roi
                && result.confirmed_observations() == 2
                && matches.len() == 1
                && matches[0].bounds().left() == 20
                && matches[0].bounds().top() == 12
                && (matches[0].score() - 1.0).abs() <= 1e-5
                && result
                    .frame()
                    .map(FORMAT, &operation())
                    .map_err(|error| error.to_string())?
                    .bytes()
                    == scene
        }
        _ => false,
    };
    let source_correct = nonmatch
        .last_frame()
        .is_some_and(|stamp| stamp.sequence().value() == 0)
        && first
            .last_frame()
            .is_some_and(|stamp| stamp.sequence().value() == 1)
        && other_progress.last_frame() == first.last_frame();
    let dispositions_correct = [nonmatch, first, other_progress].iter().all(|progress| {
        [
            TemplateWorkDisposition::SkippedChange,
            TemplateWorkDisposition::Coalesced,
            TemplateWorkDisposition::Failed,
            TemplateWorkDisposition::Rejected,
            TemplateWorkDisposition::QueueExpired,
        ]
        .iter()
        .all(|&kind| progress.work().get(kind) == 0)
    });
    let after = benchmark_instrumentation::snapshot();
    let work_correct = after.checked_delta(before).is_some_and(|work| {
        work.find_calls == 4 && work.find_completions == 4 && work.find_failures == 0
    });
    drop(outcome);
    drop(cancelled);
    drop(other);
    drop(query);
    drop(session);
    drop(engine);
    drop(capture);

    let deadline = Instant::now() + WAIT;
    while benchmark_instrumentation::work_snapshot().bgr_retained_bytes != retained_before
        && Instant::now() < deadline
    {
        std::thread::yield_now();
    }
    let released = benchmark_instrumentation::work_snapshot().bgr_retained_bytes == retained_before;
    Ok(source_correct
        && dispositions_correct
        && cancellation_correct
        && result_correct
        && work_correct
        && retained.saturating_sub(retained_before) <= CACHE_LIMIT
        && released)
}

fn progress(
    query: &TemplateQuery,
    completed: u64,
    confirmed: u32,
) -> Result<TemplateQueryProgress, String> {
    let deadline = Instant::now() + WAIT;
    loop {
        match query.poll() {
            TemplateQueryOutcome::Pending(progress)
                if progress.work().get(TemplateWorkDisposition::Completed) == completed
                    && progress.confirmed_observations() == confirmed =>
            {
                return Ok(progress);
            }
            TemplateQueryOutcome::Pending(_) if Instant::now() < deadline => {
                std::thread::yield_now();
            }
            outcome => {
                return Err(format!(
                    "watcher did not reach the expected public state: {outcome:?}"
                ));
            }
        }
    }
}

fn operation() -> OperationContext {
    OperationContext::new()
        .with_timeout(WAIT)
        .expect("bounded watcher operation")
}
