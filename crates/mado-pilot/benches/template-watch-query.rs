//! Exact-source deterministic replay/OpenCV and scheduler qualification profile.
//!
//! Smoke mode runs one warmup and three retained samples through every oracle.
//! A qualification process passes `--bench` and retains the frozen three
//! warmups and twenty samples without retry or exclusion.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mado_pilot::replay::{ReplayFrame, ReplaySource, ReplayTarget};
use mado_pilot::{
    ClipPolicy, ContentDigest, Continuity, FrameDescriptor, MatchOptions, MonotonicInstant,
    OpenRequest, OperationContext, PackageSource, PixelFormat, PixelRect, PreparedTemplate,
    RegionSelection, Session, TemplateOverload, TemplateQuery, TemplateQueryOutcome,
    TemplateQueryProgress, TemplateStability, TemplateTerminalOutcome, TemplateWatchRequest,
    TemplateWorkDisposition,
};
use mado_pilot_adapter_replay::ReplayProvider;
use mado_pilot_backend_opencv::OpenCvBackend;
use mado_pilot_runtime::{
    CaptureProvider, Engine, EngineWiring, IdentityIssuer, Matcher, PackageLoader,
};
use mado_pilot_testkit::bench_harness::{
    Accounting, Benchmark, Plan, Profile, QueryWorkMetrics, Sample, Workload, measure,
};
use mado_pilot_testkit::{
    Candidate, CompletionGate, ControlledCapture, ControlledMatcher, ManualClock, MatchBackend,
    ObservedMatcher, ScriptedMatchCall, bench_harness, match_fixtures,
};
#[cfg(all(
    target_arch = "x86_64",
    target_os = "windows",
    target_env = "msvc",
    target_vendor = "pc"
))]
use std::mem::size_of;
#[cfg(all(
    target_arch = "x86_64",
    target_os = "windows",
    target_env = "msvc",
    target_vendor = "pc"
))]
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
#[cfg(all(
    target_arch = "x86_64",
    target_os = "windows",
    target_env = "msvc",
    target_vendor = "pc"
))]
use windows::Win32::System::Threading::GetCurrentProcess;

#[global_allocator]
static ALLOCATOR: Accounting = Accounting;

const SOURCE_FORMAT: PixelFormat = PixelFormat::Rgba8;
const WAIT: Duration = Duration::from_secs(5);
const QUEUE_EXPIRY_ADVANCE: Duration = Duration::from_secs(31);
const FULL_MAPPED_BYTES: u64 = 96 * 64 * 4;
const CONTROLLED_MAPPED_BYTES: u64 = 32 * 24 * 4;
const ROI: (i32, i32, i32, i32) = (16, 8, 48, 32);
const ROI_MAPPED_BYTES: u64 = 32 * 24 * 4;
const FULL_ORIGINS: [(i32, i32); 2] = [(20, 12), (60, 40)];
const ROI_ORIGINS: [(i32, i32); 1] = [(20, 12)];

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let qualification = arguments.iter().any(|argument| argument == "--bench");
    let plan = if qualification {
        Plan::new(3, 20)
    } else {
        Plan::new(3, 3)
    };
    let (hardware, os_version) = Profile::host(&arguments);

    let workloads = [
        measure(
            "engine_session_startup",
            "one fixed replay source constructs the OpenCV engine and opens then closes one session",
            plan,
            || (),
            engine_session_startup,
        ),
        measure(
            "current_match",
            "one current replay frame produces the exact planted match set and source stamp",
            plan,
            || {
                ReplayFixture::new(
                    vec![match_fixtures::scene_pixels(SOURCE_FORMAT)],
                    TemplateStability::immediate(),
                    None,
                    0,
                    1,
                    1,
                    &FULL_ORIGINS,
                    FULL_MAPPED_BYTES,
                )
            },
            replay_watch,
        ),
        measure(
            "appearance_stable",
            "background then two present frames confirms only the second stable presence",
            plan,
            || (),
            appearance_stable,
        ),
        measure(
            "disappearance_reset",
            "a confirmed absence resets stability before two later presence confirmations",
            plan,
            || (),
            disappearance_reset,
        ),
        measure(
            "roi_match",
            "the exact capture-pixel ROI maps only itself and retains its source-qualified match",
            plan,
            || {
                ReplayFixture::new(
                    vec![match_fixtures::scene_pixels(SOURCE_FORMAT)],
                    TemplateStability::immediate(),
                    Some(
                        RegionSelection::pixels(roi_rect(), ClipPolicy::Reject)
                            .expect("representable qualification ROI"),
                    ),
                    0,
                    1,
                    1,
                    &ROI_ORIGINS,
                    ROI_MAPPED_BYTES,
                )
            },
            replay_watch,
        ),
        measure(
            "static_duration",
            "three OpenCV confirmations advance duration only after the manual clock advances",
            plan,
            || (),
            static_duration,
        ),
        measure(
            "coalesced_pair",
            "one gated immutable analysis completes two independently owned queries",
            plan,
            || (),
            coalesced_pair,
        ),
        measure(
            "saturation_latest_wins",
            "finite saturation retains the query, replaces pending frames, expires visibly, and publishes",
            plan,
            || (),
            saturation_latest_wins,
        ),
        measure(
            "two_session_fairness",
            "four eligible queries across two sessions enter in two bounded backend waves",
            plan,
            || (),
            two_session_fairness,
        ),
        measure(
            "cancel_in_flight",
            "query cancellation wins before a gated backend completion and discards the late match",
            plan,
            || (),
            cancel_in_flight,
        ),
        measure(
            "close_and_retain",
            "repeated close wakes pending work, retained results survive, and later publication progresses",
            plan,
            || (),
            close_and_retain,
        ),
    ];

    let opencv_runtime_version = OpenCvBackend::new()
        .expect("OpenCV development installation")
        .descriptor()
        .version()
        .to_owned();
    assert_eq!(
        opencv_runtime_version, "4.14.0",
        "qualification requires the frozen native OpenCV runtime"
    );

    if qualification {
        let source = required_identity(&arguments, "--source-revision", 40);
        let tree = required_identity(&arguments, "--source-tree", 40);
        let process = required_identity(&arguments, "--process-id", 1);
        let executable = required_identity(&arguments, "--executable-sha256", 64);
        bench_harness::report(
            &Benchmark {
                id: "phase-4-template-watch-query",
                workload: "deterministic Rust replay/OpenCV watcher plus controlled scheduler oracles",
                phase: "4",
            },
            &Profile {
                fixture: "fixtures/assets/phase1-slice and mado-pilot-testkit deterministic scenes"
                    .to_owned(),
                fixture_sha256: fixture_digest().to_string(),
                benchmark_executable_sha256: Some(executable),
                hardware,
                os_version,
                deployment_target: Some(bench_harness::RELEASE_TARGET.to_owned()),
                build_profile: format!(
                    "cargo build --locked --release --package mado-pilot --bench template-watch-query; debug_assertions={}",
                    cfg!(debug_assertions)
                ),
                correctness_oracle: "every retained sample checks exact source, match, state, observed work/lifecycle outcomes, ownership, and producer progress",
                queue_policy: "fixed scheduler descriptor: 256 engine queries, 16 active sessions, 64 queries per session, two analyses, one latest pending frame per query",
                notes: Some(format!(
                    "source commit {source}; tree {tree}; process {process}; OpenCV {opencv_runtime_version}; controlled-matcher rows are correctness/resource evidence only; startup latency has no numeric ceiling"
                )),
            },
            plan,
            &workloads,
        );
    } else {
        bench_harness::summarize("template-watch-query", plan, &workloads);
    }

    bench_harness::enforce_hard_budgets(&workloads);
    bench_harness::enforce_mapped_bytes(
        &workloads,
        &bench_harness::PHASE4_TEMPLATE_WATCH_MAPPED_BYTES_BUDGETS,
    );
    enforce_qualification_oracles(&workloads, plan);
    if arguments
        .iter()
        .any(|argument| argument == "--enforce-budgets")
    {
        enforce_target_budgets(&workloads);
    }
}

fn engine_session_startup(_: &()) -> Sample {
    let allocation_baseline = bench_harness::live_allocated_bytes();
    let started = Instant::now();
    let engine = mado_pilot::replay_engine(replay_source(vec![match_fixtures::scene_pixels(
        SOURCE_FORMAT,
    )]))
    .expect("constructed OpenCV replay engine");
    let operation = bounded_operation();
    let targets = engine
        .discover(&operation)
        .expect("discovered startup target");
    let target = targets[0].id();
    let session = engine
        .open(target, &OpenRequest::new(), &operation)
        .expect("opened startup session");
    let elapsed = started.elapsed();
    let opened_correct = session.target() == target && targets.len() == 1;
    session.close(&operation).expect("closed startup session");
    drop(session);
    drop(targets);
    drop(engine);
    let cleanup_correct = wait_for_live_allocations(allocation_baseline.saturating_add(
        usize::try_from(bench_harness::GROWTH_LIMIT_BYTES).expect("positive growth limit"),
    ));

    finish_sample(
        elapsed,
        opened_correct && cleanup_correct,
        0,
        Some(QueryWorkMetrics::default()),
    )
}

#[derive(Debug)]
struct ReplayFixture {
    engine: Engine,
    observed: Arc<ObservedMatcher>,
    target: mado_pilot::TargetId,
    template: PreparedTemplate,
    stability: TemplateStability,
    region: Option<RegionSelection>,
    expected_sequence: u64,
    expected_observations: u32,
    expected_backend_runs: u64,
    expected_origins: &'static [(i32, i32)],
    mapped_bytes: u64,
    publications: u64,
}

impl ReplayFixture {
    #[allow(clippy::too_many_arguments)]
    fn new(
        frames: Vec<Vec<u8>>,
        stability: TemplateStability,
        region: Option<RegionSelection>,
        expected_sequence: u64,
        expected_observations: u32,
        expected_backend_runs: u64,
        expected_origins: &'static [(i32, i32)],
        mapped_bytes: u64,
    ) -> Self {
        let publications = u64::try_from(frames.len()).expect("small replay sequence");
        let source = replay_source(frames);
        let issuer = Arc::new(IdentityIssuer::new());
        let engine_id = issuer.engine();
        let capture = ReplayProvider::new(issuer, source).expect("replay provider");
        let observed = Arc::new(ObservedMatcher::new(Arc::new(
            OpenCvBackend::new().expect("OpenCV replay backend"),
        )));
        let engine = Engine::new(EngineWiring {
            engine: engine_id,
            capture: Arc::new(capture),
            matcher: Matcher::new(Arc::clone(&observed) as Arc<dyn MatchBackend>),
            loader: PackageLoader::new(),
            ocr: None,
            input: None,
            permission: None,
        })
        .expect("observed OpenCV replay engine");
        let operation = OperationContext::new();
        let target = engine.discover(&operation).expect("discovered")[0].id();
        let package = engine
            .load_package(&PackageSource::directory(package_root()), &operation)
            .expect("tracked package loads");
        let template = engine
            .prepare_from_package(&package, "panel.patch", &operation)
            .expect("prepared template");
        Self {
            engine,
            observed,
            target,
            template,
            stability,
            region,
            expected_sequence,
            expected_observations,
            expected_backend_runs,
            expected_origins,
            mapped_bytes,
            publications,
        }
    }
}

fn replay_watch(fixture: &ReplayFixture) -> Sample {
    let allocation_baseline = bench_harness::live_allocated_bytes();
    let operation = bounded_operation();
    let session = fixture
        .engine
        .open(fixture.target, &OpenRequest::new(), &operation)
        .expect("opened replay session");
    let mut request = TemplateWatchRequest::new(
        fixture.template.clone(),
        MatchOptions::from_defaults(fixture.template.defaults()),
        bounded_operation(),
    )
    .with_stability(fixture.stability);
    if let Some(region) = fixture.region {
        request = request.with_region(region);
    }

    let backend_runs_before = fixture.observed.find_count();
    let backend_completions_before = fixture.observed.completion_count();
    let started = Instant::now();
    let query = session
        .start_template_watch(request)
        .expect("started replay query");
    let outcome = query
        .wait(&bounded_operation())
        .expect("replay query completed");
    let elapsed = started.elapsed();
    let matched = match outcome.as_ref() {
        TemplateTerminalOutcome::Matched(result) => Some(result),
        _ => None,
    };
    let result_correct = matched.is_some_and(|result| {
        result.frame().stamp().sequence().value() == fixture.expected_sequence
            && result.confirmed_observations() == fixture.expected_observations
            && result.result().searched()
                == fixture.region.map_or_else(full_scene_rect, |_| roi_rect())
            && exact_origins(result.result(), fixture.expected_origins)
    });
    let result_mapped_bytes = matched.and_then(|result| mapped_bytes(result.result().searched()));
    let observed_mapped_bytes =
        u64::try_from(fixture.observed.consistent_mapped_bytes().unwrap_or(0))
            .expect("mapped bytes fit");
    let observed_backend_runs = u64::try_from(
        fixture
            .observed
            .find_count()
            .checked_sub(backend_runs_before)
            .expect("backend run counter is monotonic"),
    )
    .expect("backend count fits");
    let observed_backend_completions = u64::try_from(
        fixture
            .observed
            .completion_count()
            .checked_sub(backend_completions_before)
            .expect("backend completion counter is monotonic"),
    )
    .expect("backend count fits");
    let metrics = Some(observed_query_metrics(
        observed_backend_runs,
        observed_backend_completions,
        outcome.as_ref(),
        fixture.publications,
    ));
    let work_correct = metrics.is_some_and(|work| {
        observed_backend_runs == fixture.expected_backend_runs
            && observed_backend_completions == fixture.expected_backend_runs
            && work.backend_runs == observed_backend_runs
            && work.query_completions == 1
            && work.query_failures == 0
            && work.producer_publications == fixture.publications
            && work.admitted == observed_backend_runs
            && work.skipped_change == 0
            && work.deferred_rate == 0
            && work.coalesced == 0
            && work.superseded == 0
            && work.rejected == 0
            && work.queue_expired == 0
            && work.completed == observed_backend_completions
            && work.failed == 0
    });
    let mapped_correct = result_mapped_bytes == Some(fixture.mapped_bytes)
        && observed_mapped_bytes == fixture.mapped_bytes;
    session.close(&operation).expect("closed replay session");
    drop(outcome);
    drop(query);
    drop(session);
    let cleanup_correct = wait_for_live_allocations(allocation_baseline.saturating_add(
        usize::try_from(bench_harness::GROWTH_LIMIT_BYTES).expect("positive growth limit"),
    ));

    finish_sample(
        elapsed,
        result_correct && work_correct && mapped_correct && cleanup_correct,
        observed_mapped_bytes,
        metrics,
    )
}

#[derive(Debug)]
struct OpenCvControlledFixture {
    core: ControlledCore,
    observed: Arc<ObservedMatcher>,
    template: PreparedTemplate,
    options: MatchOptions,
    scene: Vec<u8>,
    background: Vec<u8>,
}

impl OpenCvControlledFixture {
    fn new() -> Self {
        let observed = Arc::new(ObservedMatcher::new(Arc::new(
            OpenCvBackend::new().expect("OpenCV 4 development installation"),
        )));
        let core = ControlledCore::new(
            Arc::clone(&observed) as Arc<dyn MatchBackend>,
            match_fixtures::SCENE,
        );
        let operation = OperationContext::new();
        let template = core
            .engine
            .prepare_template(&match_fixtures::planted_template("duration"), &operation)
            .expect("prepared OpenCV template");
        let options = MatchOptions::from_defaults(template.defaults());
        Self {
            core,
            observed,
            template,
            options,
            scene: match_fixtures::scene_pixels(SOURCE_FORMAT),
            background: match_fixtures::background_pixels(SOURCE_FORMAT),
        }
    }
}

fn appearance_stable(_: &()) -> Sample {
    let allocation_baseline = bench_harness::live_allocated_bytes();
    let fixture = OpenCvControlledFixture::new();
    let session = fixture.core.open();
    let request = TemplateWatchRequest::new(
        fixture.template.clone(),
        fixture.options,
        OperationContext::new(),
    )
    .with_stability(TemplateStability::consecutive(2).expect("valid stability"));

    let started = Instant::now();
    let query = session
        .start_template_watch(request)
        .expect("started appearance query");
    fixture
        .core
        .capture
        .publish_pixels(&fixture.background, Continuity::Continuous)
        .expect("published absent frame");
    let absent = wait_progress(&query, |progress| {
        progress.confirmed_observations() == 0
            && progress.work().get(TemplateWorkDisposition::Completed) == 1
    });
    fixture
        .core
        .capture
        .publish_pixels(&fixture.scene, Continuity::Continuous)
        .expect("published first presence");
    let first_presence = wait_progress(&query, |progress| {
        progress.confirmed_observations() == 1
            && progress.work().get(TemplateWorkDisposition::Completed) == 2
    });
    fixture
        .core
        .capture
        .publish_pixels(&fixture.scene, Continuity::Continuous)
        .expect("published second presence");
    let outcome = query
        .wait(&bounded_operation())
        .expect("appearance query completed");
    let elapsed = started.elapsed();
    let result_correct = absent
        .last_frame()
        .is_some_and(|stamp| stamp.sequence().value() == 0)
        && first_presence
            .last_frame()
            .is_some_and(|stamp| stamp.sequence().value() == 1)
        && matches!(
            outcome.as_ref(),
            TemplateTerminalOutcome::Matched(result)
                if result.frame().stamp().sequence().value() == 2
                    && result.confirmed_observations() == 2
                    && result.result().searched() == full_scene_rect()
                    && exact_origins(result.result(), &FULL_ORIGINS)
        );
    let result_mapped_bytes = match outcome.as_ref() {
        TemplateTerminalOutcome::Matched(result) => mapped_bytes(result.result().searched()),
        _ => None,
    };
    let observed_mapped_bytes =
        u64::try_from(fixture.observed.consistent_mapped_bytes().unwrap_or(0))
            .expect("mapped bytes fit");
    let metrics = Some(observed_query_metrics(
        u64::try_from(fixture.observed.find_count()).expect("backend count fits"),
        u64::try_from(fixture.observed.completion_count()).expect("backend count fits"),
        outcome.as_ref(),
        3,
    ));
    let work_correct = metrics == Some(completed_query_metrics(3, 3));
    let mapped_correct = result_mapped_bytes == Some(FULL_MAPPED_BYTES)
        && observed_mapped_bytes == FULL_MAPPED_BYTES;
    session.close(&bounded_operation()).expect("closed session");

    drop(outcome);
    drop(query);
    drop(session);
    drop(fixture);
    let cleanup_correct = wait_for_live_allocations(allocation_baseline.saturating_add(
        usize::try_from(bench_harness::GROWTH_LIMIT_BYTES).expect("positive growth limit"),
    ));

    finish_sample(
        elapsed,
        result_correct && work_correct && mapped_correct && cleanup_correct,
        observed_mapped_bytes,
        metrics,
    )
}

fn disappearance_reset(_: &()) -> Sample {
    let allocation_baseline = bench_harness::live_allocated_bytes();
    let fixture = OpenCvControlledFixture::new();
    let session = fixture.core.open();
    let request = TemplateWatchRequest::new(
        fixture.template.clone(),
        fixture.options,
        OperationContext::new(),
    )
    .with_stability(TemplateStability::consecutive(2).expect("valid stability"));

    let started = Instant::now();
    let query = session
        .start_template_watch(request)
        .expect("started disappearance query");
    fixture
        .core
        .capture
        .publish_pixels(&fixture.scene, Continuity::Continuous)
        .expect("published initial presence");
    let initial_presence = wait_progress(&query, |progress| {
        progress.confirmed_observations() == 1
            && progress.work().get(TemplateWorkDisposition::Completed) == 1
    });
    fixture
        .core
        .capture
        .publish_pixels(&fixture.background, Continuity::Continuous)
        .expect("published disappearance");
    let disappeared = wait_progress(&query, |progress| {
        progress.confirmed_observations() == 0
            && progress.work().get(TemplateWorkDisposition::Completed) == 2
    });
    fixture
        .core
        .capture
        .publish_pixels(&fixture.scene, Continuity::Continuous)
        .expect("published first reappearance");
    let first_reappearance = wait_progress(&query, |progress| {
        progress.confirmed_observations() == 1
            && progress.work().get(TemplateWorkDisposition::Completed) == 3
    });
    fixture
        .core
        .capture
        .publish_pixels(&fixture.scene, Continuity::Continuous)
        .expect("published second reappearance");
    let outcome = query
        .wait(&bounded_operation())
        .expect("disappearance query completed");
    let elapsed = started.elapsed();
    let result_correct = initial_presence
        .last_frame()
        .is_some_and(|stamp| stamp.sequence().value() == 0)
        && disappeared
            .last_frame()
            .is_some_and(|stamp| stamp.sequence().value() == 1)
        && first_reappearance
            .last_frame()
            .is_some_and(|stamp| stamp.sequence().value() == 2)
        && matches!(
            outcome.as_ref(),
            TemplateTerminalOutcome::Matched(result)
                if result.frame().stamp().sequence().value() == 3
                    && result.confirmed_observations() == 2
                    && result.result().searched() == full_scene_rect()
                    && exact_origins(result.result(), &FULL_ORIGINS)
        );
    let result_mapped_bytes = match outcome.as_ref() {
        TemplateTerminalOutcome::Matched(result) => mapped_bytes(result.result().searched()),
        _ => None,
    };
    let observed_mapped_bytes =
        u64::try_from(fixture.observed.consistent_mapped_bytes().unwrap_or(0))
            .expect("mapped bytes fit");
    let metrics = Some(observed_query_metrics(
        u64::try_from(fixture.observed.find_count()).expect("backend count fits"),
        u64::try_from(fixture.observed.completion_count()).expect("backend count fits"),
        outcome.as_ref(),
        4,
    ));
    let work_correct = metrics == Some(completed_query_metrics(4, 4));
    let mapped_correct = result_mapped_bytes == Some(FULL_MAPPED_BYTES)
        && observed_mapped_bytes == FULL_MAPPED_BYTES;
    session.close(&bounded_operation()).expect("closed session");

    drop(outcome);
    drop(query);
    drop(session);
    drop(fixture);
    let cleanup_correct = wait_for_live_allocations(allocation_baseline.saturating_add(
        usize::try_from(bench_harness::GROWTH_LIMIT_BYTES).expect("positive growth limit"),
    ));

    finish_sample(
        elapsed,
        result_correct && work_correct && mapped_correct && cleanup_correct,
        observed_mapped_bytes,
        metrics,
    )
}

fn static_duration(_: &()) -> Sample {
    let allocation_baseline = bench_harness::live_allocated_bytes();
    let fixture = OpenCvControlledFixture::new();
    let session = fixture.core.open();
    let clock = Arc::new(ManualClock::new());
    let request = TemplateWatchRequest::new(
        fixture.template.clone(),
        fixture.options,
        OperationContext::new().with_clock(clock.clone()),
    )
    .with_stability(TemplateStability::duration(Duration::from_secs(1)).expect("valid duration"));

    let started = Instant::now();
    let query = session
        .start_template_watch(request)
        .expect("started duration query");
    fixture
        .core
        .capture
        .publish_pixels(&fixture.scene, Continuity::Continuous)
        .expect("published first confirmation");
    let first = wait_progress(&query, |progress| progress.confirmed_observations() == 1);
    fixture
        .core
        .capture
        .publish_pixels(&fixture.scene, Continuity::Continuous)
        .expect("published same-time confirmation");
    let second = wait_progress(&query, |progress| progress.confirmed_observations() == 2);
    clock.advance(Duration::from_secs(1));
    fixture
        .core
        .capture
        .publish_pixels(&fixture.scene, Continuity::Continuous)
        .expect("published duration confirmation");
    let outcome = query
        .wait(&bounded_operation())
        .expect("duration query completed");
    let elapsed = started.elapsed();
    let result_correct = matches!(
        outcome.as_ref(),
        TemplateTerminalOutcome::Matched(result)
            if result.frame().stamp().sequence().value() == 2
                && result.confirmed_observations() == 3
                && result.confirmed_duration() == Duration::from_secs(1)
                && result.result().searched() == full_scene_rect()
                && exact_origins(result.result(), &FULL_ORIGINS)
    ) && first.confirmed_duration() == Duration::ZERO
        && second.confirmed_duration() == Duration::ZERO;
    let result_mapped_bytes = match outcome.as_ref() {
        TemplateTerminalOutcome::Matched(result) => mapped_bytes(result.result().searched()),
        _ => None,
    };
    let observed_mapped_bytes =
        u64::try_from(fixture.observed.consistent_mapped_bytes().unwrap_or(0))
            .expect("mapped bytes fit");
    let metrics = Some(observed_query_metrics(
        u64::try_from(fixture.observed.find_count()).expect("backend count fits"),
        u64::try_from(fixture.observed.completion_count()).expect("backend count fits"),
        outcome.as_ref(),
        3,
    ));
    let work_correct = metrics == Some(completed_query_metrics(3, 3));
    let mapped_correct = result_mapped_bytes == Some(FULL_MAPPED_BYTES)
        && observed_mapped_bytes == FULL_MAPPED_BYTES;
    session.close(&bounded_operation()).expect("closed session");

    drop(outcome);
    drop(query);
    drop(session);
    drop(fixture);
    drop(clock);
    let cleanup_correct = wait_for_live_allocations(allocation_baseline.saturating_add(
        usize::try_from(bench_harness::GROWTH_LIMIT_BYTES).expect("positive growth limit"),
    ));

    finish_sample(
        elapsed,
        result_correct && work_correct && mapped_correct && cleanup_correct,
        observed_mapped_bytes,
        metrics,
    )
}

#[derive(Debug)]
struct ControlledCore {
    engine: Engine,
    capture: Arc<ControlledCapture>,
    target: mado_pilot::TargetId,
}

impl ControlledCore {
    fn new(backend: Arc<dyn MatchBackend>, extent: mado_pilot::PixelExtent) -> Self {
        let issuer = Arc::new(IdentityIssuer::new());
        let capture = Arc::new(
            ControlledCapture::new(Arc::clone(&issuer), extent, SOURCE_FORMAT)
                .expect("controlled capture"),
        );
        let engine = Engine::new(EngineWiring {
            engine: issuer.engine(),
            capture: Arc::clone(&capture) as Arc<dyn CaptureProvider>,
            matcher: Matcher::new(backend),
            loader: PackageLoader::new(),
            ocr: None,
            input: None,
            permission: None,
        })
        .expect("controlled engine");
        let target = engine
            .discover(&OperationContext::new())
            .expect("discovered")[0]
            .id();
        Self {
            engine,
            capture,
            target,
        }
    }

    fn open(&self) -> Session {
        self.engine
            .open(self.target, &OpenRequest::new(), &bounded_operation())
            .expect("opened controlled session")
    }

    fn prepare(&self, id: &str) -> PreparedTemplate {
        self.engine
            .prepare_template(
                &match_fixtures::planted_template(id),
                &OperationContext::new(),
            )
            .expect("prepared controlled template")
    }
}

#[derive(Debug)]
struct ControlledRun {
    core: ControlledCore,
    matcher: Arc<ControlledMatcher>,
}

impl ControlledRun {
    fn new(matcher: ControlledMatcher) -> Self {
        let matcher = Arc::new(matcher);
        let core = ControlledCore::new(
            Arc::clone(&matcher) as Arc<dyn MatchBackend>,
            mado_pilot::PixelExtent::new(32, 24),
        );
        Self { core, matcher }
    }
}

fn coalesced_pair(_: &()) -> Sample {
    let allocation_baseline = bench_harness::live_allocated_bytes();
    let gate = Arc::new(CompletionGate::new());
    let run = ControlledRun::new(
        ControlledMatcher::new(SOURCE_FORMAT)
            .with_candidates(vec![Candidate::new(1, 1, 0.99)])
            .with_completion_gate(Arc::clone(&gate)),
    );
    let session = run.core.open();
    let template = run.core.prepare("coalesced");
    let options = MatchOptions::from_defaults(template.defaults());

    let started = Instant::now();
    let first = start_query(&session, template.clone(), options, OperationContext::new());
    let second = start_query(&session, template, options, OperationContext::new());
    run.core
        .capture
        .publish(0x40, Continuity::Continuous)
        .expect("published shared frame");
    let entered = gate.wait_until_entered(WAIT);
    gate.release();
    let first_outcome = first.wait(&bounded_operation()).expect("first completed");
    let second_outcome = second.wait(&bounded_operation()).expect("second completed");
    let elapsed = started.elapsed();
    let backend_runs = u64::try_from(run.matcher.find_count()).expect("count fits");
    let query_completions = u64::from(matches!(
        first_outcome.as_ref(),
        TemplateTerminalOutcome::Matched(_)
    ))
    .saturating_add(u64::from(matches!(
        second_outcome.as_ref(),
        TemplateTerminalOutcome::Matched(_)
    )));
    let metrics = Some(QueryWorkMetrics {
        backend_runs,
        query_completions,
        producer_publications: 1,
        admitted: backend_runs,
        coalesced: query_completions.saturating_sub(backend_runs),
        completed: query_completions,
        ..QueryWorkMetrics::default()
    });
    let work_correct = backend_runs == 1 && query_completions == 2;
    let observed_mapped_bytes =
        u64::try_from(run.matcher.consistent_mapped_bytes().unwrap_or(0)).expect("bytes fit");
    let mapped_correct = observed_mapped_bytes == CONTROLLED_MAPPED_BYTES;
    let outcomes_correct = first.id() != second.id()
        && matches!(first_outcome.as_ref(), TemplateTerminalOutcome::Matched(_))
        && matches!(second_outcome.as_ref(), TemplateTerminalOutcome::Matched(_));
    session.close(&bounded_operation()).expect("closed session");

    drop(first_outcome);
    drop(second_outcome);
    drop(first);
    drop(second);
    drop(session);
    drop(run);
    drop(gate);
    let cleanup_correct = wait_for_live_allocations(allocation_baseline.saturating_add(
        usize::try_from(bench_harness::GROWTH_LIMIT_BYTES).expect("positive growth limit"),
    ));

    finish_sample(
        elapsed,
        entered && outcomes_correct && work_correct && mapped_correct && cleanup_correct,
        observed_mapped_bytes,
        metrics,
    )
}

fn saturation_latest_wins(_: &()) -> Sample {
    let allocation_baseline = bench_harness::live_allocated_bytes();
    let started = Instant::now();

    let latest_first_gate = Arc::new(CompletionGate::new());
    let latest_second_gate = Arc::new(CompletionGate::new());
    let latest_final_gate = Arc::new(CompletionGate::new());
    let latest_run = ControlledRun::new(
        ControlledMatcher::new(SOURCE_FORMAT).with_calls([
            ScriptedMatchCall::new(Vec::new()).with_completion_gate(Arc::clone(&latest_first_gate)),
            ScriptedMatchCall::new(Vec::new())
                .with_completion_gate(Arc::clone(&latest_second_gate)),
            ScriptedMatchCall::new(vec![Candidate::new(1, 1, 0.99)])
                .with_completion_gate(Arc::clone(&latest_final_gate)),
        ]),
    );
    let latest_session = latest_run.core.open();
    let latest_template = latest_run.core.prepare("latest-wins");
    let latest = start_query(
        &latest_session,
        latest_template.clone(),
        MatchOptions::from_defaults(latest_template.defaults()),
        OperationContext::new(),
    );
    latest_run
        .core
        .capture
        .publish(0x40, Continuity::Continuous)
        .expect("published first in-flight frame");
    let latest_first_entered = latest_first_gate.wait_until_entered(WAIT);
    latest_run
        .core
        .capture
        .publish(0x41, Continuity::Continuous)
        .expect("published second in-flight frame");
    let latest_second_entered = latest_second_gate.wait_until_entered(WAIT);
    latest_run
        .core
        .capture
        .publish(0x42, Continuity::Continuous)
        .expect("published pending frame");
    let _ = wait_progress(&latest, |progress| progress.pending_count() == 1);
    latest_run
        .core
        .capture
        .publish(0x43, Continuity::Continuous)
        .expect("published latest replacement");
    let latest_replaced = wait_progress(&latest, |progress| {
        progress.pending_count() == 1
            && progress.work().get(TemplateWorkDisposition::Superseded) == 1
    });
    latest_first_gate.release();
    let _ = wait_progress(&latest, |progress| {
        progress.work().get(TemplateWorkDisposition::Superseded) == 2
    });
    let latest_final_entered = latest_final_gate.wait_until_entered(WAIT);
    latest_second_gate.release();
    let latest_ready = wait_progress(&latest, |progress| {
        progress.pending_count() == 0
            && progress.work().get(TemplateWorkDisposition::Admitted) == 3
            && progress.work().get(TemplateWorkDisposition::Completed) == 0
            && progress.work().get(TemplateWorkDisposition::Superseded) == 3
    });
    latest_final_gate.release();
    let latest_outcome = latest
        .wait(&bounded_operation())
        .expect("latest frame completed");
    let latest_terminal_matches = u64::from(matches!(
        latest_outcome.as_ref(),
        TemplateTerminalOutcome::Matched(_)
    ));
    let latest_work = latest_ready.work();
    let latest_backend_runs = u64::try_from(latest_run.matcher.find_count()).expect("count fits");
    let latest_metrics = Some(QueryWorkMetrics {
        backend_runs: latest_backend_runs,
        query_completions: latest_terminal_matches,
        stale_discards: 2,
        producer_publications: 4,
        admitted: latest_work.get(TemplateWorkDisposition::Admitted),
        superseded: latest_work.get(TemplateWorkDisposition::Superseded),
        completed: latest_work
            .get(TemplateWorkDisposition::Completed)
            .saturating_add(latest_terminal_matches),
        ..QueryWorkMetrics::default()
    });
    let latest_correct = latest_first_entered
        && latest_second_entered
        && latest_final_entered
        && latest_replaced.pending_count() == 1
        && matches!(
            latest_outcome.as_ref(),
            TemplateTerminalOutcome::Matched(result)
                if result.frame().stamp().sequence().value() == 3
        )
        && latest_metrics.is_some_and(|work| {
            work.backend_runs == 3
                && work.admitted == 3
                && work.completed == 1
                && work.superseded == 3
                && work.stale_discards == 2
                && work.query_completions == 1
        });
    latest_session
        .close(&bounded_operation())
        .expect("closed latest-wins session");

    let first_gate = Arc::new(CompletionGate::new());
    let second_gate = Arc::new(CompletionGate::new());
    let found = vec![Candidate::new(1, 1, 0.99)];
    let saturation_run = ControlledRun::new(ControlledMatcher::new(SOURCE_FORMAT).with_calls([
        ScriptedMatchCall::new(found.clone()).with_completion_gate(Arc::clone(&first_gate)),
        ScriptedMatchCall::new(found.clone()).with_completion_gate(Arc::clone(&second_gate)),
        ScriptedMatchCall::new(found),
    ]));
    let saturation_session = saturation_run.core.open();
    let first_template = saturation_run.core.prepare("saturation-first");
    let second_template = saturation_run.core.prepare("saturation-second");
    let first = start_query(
        &saturation_session,
        first_template.clone(),
        MatchOptions::from_defaults(first_template.defaults()),
        OperationContext::new(),
    );
    let second = start_query(
        &saturation_session,
        second_template.clone(),
        MatchOptions::from_defaults(second_template.defaults()),
        OperationContext::new(),
    );
    saturation_run
        .core
        .capture
        .publish(0x40, Continuity::Continuous)
        .expect("published blocker frame");
    let blockers_entered =
        first_gate.wait_until_entered(WAIT) && second_gate.wait_until_entered(WAIT);
    let clock = Arc::new(ManualClock::new());
    let expiring_template = saturation_run.core.prepare("saturation-expiring");
    let expiring = start_query(
        &saturation_session,
        expiring_template.clone(),
        MatchOptions::from_defaults(expiring_template.defaults()),
        OperationContext::new().with_clock(clock.clone()),
    );
    let _ = wait_progress(&expiring, TemplateQueryProgress::is_pending);
    clock.advance(QUEUE_EXPIRY_ADVANCE);
    let expired = expiring
        .wait(&bounded_operation())
        .expect("expiry is a terminal outcome");
    first_gate.release();
    second_gate.release();
    let first_outcome = first
        .wait(&bounded_operation())
        .expect("first blocker completed");
    let second_outcome = second
        .wait(&bounded_operation())
        .expect("second blocker completed");
    let first_matched = u64::from(matches!(
        first_outcome.as_ref(),
        TemplateTerminalOutcome::Matched(_)
    ));
    let second_matched = u64::from(matches!(
        second_outcome.as_ref(),
        TemplateTerminalOutcome::Matched(_)
    ));
    let expired_count = u64::from(matches!(
        expired.as_ref(),
        TemplateTerminalOutcome::Overloaded(TemplateOverload::QueueExpired)
    ));
    let saturation_backend_runs =
        u64::try_from(saturation_run.matcher.find_count()).expect("count fits");
    let saturation_metrics = Some(QueryWorkMetrics {
        backend_runs: saturation_backend_runs,
        query_completions: first_matched
            .saturating_add(second_matched)
            .saturating_add(expired_count),
        producer_publications: 1,
        admitted: saturation_backend_runs,
        queue_expired: expired_count,
        completed: first_matched.saturating_add(second_matched),
        ..QueryWorkMetrics::default()
    });
    let saturation_correct = blockers_entered
        && first_matched == 1
        && second_matched == 1
        && expired_count == 1
        && saturation_backend_runs == 2;
    saturation_session
        .close(&bounded_operation())
        .expect("closed saturation session");

    let metrics = latest_metrics
        .zip(saturation_metrics)
        .map(|(latest, saturation)| latest.saturating_add(saturation));
    let work_correct = metrics.is_some_and(|work| {
        work.backend_runs == 5
            && work.admitted == 5
            && work.completed == 3
            && work.queue_expired == 1
            && work.superseded == 3
            && work.stale_discards == 2
            && work.query_completions == 4
            && work.query_failures == 0
            && work.producer_publications == 5
    });
    let latest_mapped = u64::try_from(latest_run.matcher.consistent_mapped_bytes().unwrap_or(0))
        .expect("bytes fit");
    let saturation_mapped = u64::try_from(
        saturation_run
            .matcher
            .consistent_mapped_bytes()
            .unwrap_or(0),
    )
    .expect("bytes fit");
    let mapped_correct =
        latest_mapped == CONTROLLED_MAPPED_BYTES && saturation_mapped == CONTROLLED_MAPPED_BYTES;

    let elapsed = started.elapsed();
    drop(latest_outcome);
    drop(latest);
    drop(latest_session);
    drop(latest_run);
    drop(latest_first_gate);
    drop(latest_second_gate);
    drop(latest_final_gate);
    drop(expired);
    drop(first_outcome);
    drop(second_outcome);
    drop(expiring);
    drop(first);
    drop(second);
    drop(saturation_session);
    drop(saturation_run);
    drop(clock);
    drop(first_gate);
    drop(second_gate);
    let cleanup_correct = wait_for_live_allocations(allocation_baseline.saturating_add(
        usize::try_from(bench_harness::GROWTH_LIMIT_BYTES).expect("positive growth limit"),
    ));

    finish_sample(
        elapsed,
        latest_correct && saturation_correct && work_correct && mapped_correct && cleanup_correct,
        latest_mapped,
        metrics,
    )
}

fn two_session_fairness(_: &()) -> Sample {
    let allocation_baseline = bench_harness::live_allocated_bytes();
    let blocker_gates: Vec<_> = (0..2).map(|_| Arc::new(CompletionGate::new())).collect();
    let fairness_gates: Vec<_> = (0..4).map(|_| Arc::new(CompletionGate::new())).collect();
    let blocker_calls = blocker_gates.iter().enumerate().map(|(index, gate)| {
        ScriptedMatchCall::new(vec![Candidate::new(
            i32::try_from(index + 10).expect("small blocker coordinate"),
            1,
            0.99,
        )])
        .with_completion_gate(Arc::clone(gate))
    });
    let fairness_calls = fairness_gates.iter().enumerate().map(|(index, gate)| {
        ScriptedMatchCall::new(vec![Candidate::new(
            i32::try_from(index + 1).expect("small fairness coordinate"),
            1,
            0.99,
        )])
        .with_completion_gate(Arc::clone(gate))
    });
    let run = ControlledRun::new(
        ControlledMatcher::new(SOURCE_FORMAT).with_calls(blocker_calls.chain(fairness_calls)),
    );

    let blocker_session = run.core.open();
    let blockers: Vec<_> = (0..2)
        .map(|index| {
            let template = run.core.prepare(&format!("fairness-blocker-{index}"));
            start_query(
                &blocker_session,
                template.clone(),
                MatchOptions::from_defaults(template.defaults()),
                OperationContext::new(),
            )
        })
        .collect();
    run.core
        .capture
        .publish(0x30, Continuity::Continuous)
        .expect("published blocker frame");
    let blockers_entered = blocker_gates
        .iter()
        .all(|gate| gate.wait_until_entered(WAIT));
    let blocker_outcomes: [Arc<TemplateTerminalOutcome>; 2] =
        std::array::from_fn(|index| blockers[index].cancel());
    let blockers_cancelled = blocker_outcomes
        .iter()
        .all(|outcome| matches!(outcome.as_ref(), TemplateTerminalOutcome::Cancelled));

    let first_session = run.core.open();
    let second_session = run.core.open();
    let prepared: Vec<_> = (0..4)
        .map(|index| run.core.prepare(&format!("fairness-{index}")))
        .collect();
    let started = Instant::now();
    let queries = [
        start_query(
            &first_session,
            prepared[0].clone(),
            MatchOptions::from_defaults(prepared[0].defaults()),
            OperationContext::new(),
        ),
        start_query(
            &first_session,
            prepared[1].clone(),
            MatchOptions::from_defaults(prepared[1].defaults()),
            OperationContext::new(),
        ),
        start_query(
            &second_session,
            prepared[2].clone(),
            MatchOptions::from_defaults(prepared[2].defaults()),
            OperationContext::new(),
        ),
        start_query(
            &second_session,
            prepared[3].clone(),
            MatchOptions::from_defaults(prepared[3].defaults()),
            OperationContext::new(),
        ),
    ];
    run.core
        .capture
        .publish(0x40, Continuity::Continuous)
        .expect("published to both fairness sessions");
    let all_eligible = queries.iter().all(|query| {
        wait_progress(query, |progress| progress.pending_count() == 1).pending_count() == 1
    });

    for gate in &blocker_gates {
        gate.release();
    }
    let first_wave = fairness_gates[0].wait_until_entered(WAIT)
        && fairness_gates[1].wait_until_entered(WAIT)
        && run.matcher.find_count() == 4;
    fairness_gates[0].release();
    let third_entered = fairness_gates[2].wait_until_entered(WAIT) && run.matcher.find_count() == 5;
    fairness_gates[1].release();
    let fourth_entered =
        fairness_gates[3].wait_until_entered(WAIT) && run.matcher.find_count() == 6;
    fairness_gates[2].release();
    fairness_gates[3].release();

    let blockers_completed = blocker_gates
        .iter()
        .all(|gate| gate.wait_until_completed(WAIT));
    let fairness_outcomes = std::array::from_fn(|index| {
        queries[index]
            .wait(&bounded_operation())
            .expect("fairness query completed")
    });
    let mut fairness_origins = fairness_outcomes.each_ref().map(|outcome| {
        let TemplateTerminalOutcome::Matched(result) = outcome.as_ref() else {
            return -1;
        };
        result.result().matches()[0].bounds().left()
    });
    let first_wave_identity = fairness_origins[..2]
        .iter()
        .filter(|left| matches!(**left, 1 | 2))
        .count()
        == 1
        && fairness_origins[2..]
            .iter()
            .filter(|left| matches!(**left, 1 | 2))
            .count()
            == 1;
    fairness_origins.sort_unstable();
    let outcomes_correct = blockers_cancelled && fairness_origins == [1, 2, 3, 4];
    let fairness_query_completions = u64::try_from(
        blocker_outcomes.len() + fairness_origins.iter().filter(|left| **left > 0).count(),
    )
    .expect("query count fits");
    let fairness_backend_runs = u64::try_from(run.matcher.find_count()).expect("count fits");
    let fairness_backend_completions =
        u64::try_from(run.matcher.completion_count()).expect("count fits");
    let fairness_metrics = Some(QueryWorkMetrics {
        backend_runs: fairness_backend_runs,
        query_completions: fairness_query_completions,
        producer_publications: 2,
        admitted: fairness_backend_runs,
        superseded: 2,
        completed: 4,
        ..QueryWorkMetrics::default()
    });
    let fairness_work_correct = blockers_completed
        && fairness_backend_runs == 6
        && fairness_backend_completions == 6
        && fairness_query_completions == 6;
    blocker_session
        .close(&bounded_operation())
        .expect("closed blocker session");
    first_session
        .close(&bounded_operation())
        .expect("closed first session");
    second_session
        .close(&bounded_operation())
        .expect("closed second session");

    let older = Arc::new(CompletionGate::new());
    let newer = Arc::new(CompletionGate::new());
    let found = vec![Candidate::new(1, 1, 0.99)];
    let stale_run = ControlledRun::new(ControlledMatcher::new(SOURCE_FORMAT).with_calls([
        ScriptedMatchCall::new(found.clone()).with_completion_gate(Arc::clone(&older)),
        ScriptedMatchCall::new(found).with_completion_gate(Arc::clone(&newer)),
    ]));
    let stale_session = stale_run.core.open();
    let stale_template = stale_run.core.prepare("stale-generation");
    let stale_query = start_query(
        &stale_session,
        stale_template.clone(),
        MatchOptions::from_defaults(stale_template.defaults()),
        OperationContext::new(),
    );
    stale_run
        .core
        .capture
        .publish(0x40, Continuity::Continuous)
        .expect("published older generation");
    let older_entered = older.wait_until_entered(WAIT);
    stale_run
        .core
        .capture
        .publish(0x41, Continuity::Continuous)
        .expect("published newer generation");
    let newer_entered = newer.wait_until_entered(WAIT);
    newer.release();
    let newer_outcome = stale_query
        .wait(&bounded_operation())
        .expect("newer generation committed");
    older.release();
    let older_completed = older.wait_until_completed(WAIT);
    let retained = match stale_query.poll() {
        TemplateQueryOutcome::Terminal(outcome) => Some(outcome),
        TemplateQueryOutcome::Pending(_) => None,
    };
    let stale_backend_runs = u64::try_from(stale_run.matcher.find_count()).expect("count fits");
    let stale_terminal_matches = u64::from(matches!(
        newer_outcome.as_ref(),
        TemplateTerminalOutcome::Matched(_)
    ));
    let stale_metrics = Some(QueryWorkMetrics {
        backend_runs: stale_backend_runs,
        query_completions: stale_terminal_matches,
        stale_discards: u64::from(older_entered && newer_entered && older_completed),
        producer_publications: 2,
        admitted: stale_backend_runs,
        superseded: stale_terminal_matches,
        completed: stale_terminal_matches,
        ..QueryWorkMetrics::default()
    });
    let stale_correct = matches!(
        newer_outcome.as_ref(),
        TemplateTerminalOutcome::Matched(result)
            if result.frame().stamp().sequence().value() == 1
    ) && retained
        .as_ref()
        .is_some_and(|retained| Arc::ptr_eq(&newer_outcome, retained))
        && stale_backend_runs == 2
        && stale_metrics.is_some_and(|work| work.stale_discards == 1);
    stale_session
        .close(&bounded_operation())
        .expect("closed stale session");

    let metrics = fairness_metrics
        .zip(stale_metrics)
        .map(|(fairness, stale)| fairness.saturating_add(stale));
    let work_correct = metrics.is_some_and(|work| {
        work.backend_runs == 8
            && work.admitted == 8
            && work.completed == 5
            && work.stale_discards == 1
            && work.superseded == 3
            && work.query_completions == 7
            && work.query_failures == 0
            && work.producer_publications == 4
    });
    let fairness_mapped =
        u64::try_from(run.matcher.consistent_mapped_bytes().unwrap_or(0)).expect("bytes fit");
    let stale_mapped =
        u64::try_from(stale_run.matcher.consistent_mapped_bytes().unwrap_or(0)).expect("bytes fit");
    let mapped_correct =
        fairness_mapped == CONTROLLED_MAPPED_BYTES && stale_mapped == CONTROLLED_MAPPED_BYTES;

    let elapsed = started.elapsed();
    drop(retained);
    drop(newer_outcome);
    drop(fairness_outcomes);
    drop(blocker_outcomes);
    drop(stale_query);
    drop(stale_session);
    drop(stale_run);
    drop(older);
    drop(newer);
    drop(queries);
    drop(blockers);
    drop(prepared);
    drop(blocker_session);
    drop(first_session);
    drop(second_session);
    drop(run);
    drop(blocker_gates);
    drop(fairness_gates);
    let cleanup_correct = wait_for_live_allocations(allocation_baseline.saturating_add(
        usize::try_from(bench_harness::GROWTH_LIMIT_BYTES).expect("positive growth limit"),
    ));

    finish_sample(
        elapsed,
        blockers_entered
            && all_eligible
            && first_wave
            && first_wave_identity
            && third_entered
            && fourth_entered
            && outcomes_correct
            && fairness_work_correct
            && stale_correct
            && work_correct
            && mapped_correct
            && cleanup_correct,
        fairness_mapped,
        metrics,
    )
}

fn cancel_in_flight(_: &()) -> Sample {
    let allocation_baseline = bench_harness::live_allocated_bytes();
    let gate = Arc::new(CompletionGate::new());
    let run = ControlledRun::new(
        ControlledMatcher::new(SOURCE_FORMAT)
            .with_candidates(vec![Candidate::new(1, 1, 0.99)])
            .with_completion_gate(Arc::clone(&gate)),
    );
    let session = run.core.open();
    let template = run.core.prepare("cancel");

    let started = Instant::now();
    let query = start_query(
        &session,
        template.clone(),
        MatchOptions::from_defaults(template.defaults()),
        OperationContext::new(),
    );
    run.core
        .capture
        .publish(0x40, Continuity::Continuous)
        .expect("published cancel frame");
    let entered = gate.wait_until_entered(WAIT);
    let cancelled = query.cancel();
    gate.release();
    let completed = gate.wait_until_completed(WAIT);
    let elapsed = started.elapsed();
    let backend_runs = u64::try_from(run.matcher.find_count()).expect("count fits");
    let cancelled_count = u64::from(matches!(
        cancelled.as_ref(),
        TemplateTerminalOutcome::Cancelled
    ));
    let metrics = Some(QueryWorkMetrics {
        backend_runs,
        query_completions: cancelled_count,
        producer_publications: 1,
        admitted: backend_runs,
        superseded: cancelled_count,
        ..QueryWorkMetrics::default()
    });
    let work_correct = backend_runs == 1 && cancelled_count == 1;
    let observed_mapped_bytes =
        u64::try_from(run.matcher.consistent_mapped_bytes().unwrap_or(0)).expect("bytes fit");
    let mapped_correct = observed_mapped_bytes == CONTROLLED_MAPPED_BYTES;

    let outcome_correct = matches!(cancelled.as_ref(), TemplateTerminalOutcome::Cancelled)
        && matches!(query.poll(), TemplateQueryOutcome::Terminal(outcome) if matches!(outcome.as_ref(), TemplateTerminalOutcome::Cancelled));
    session.close(&bounded_operation()).expect("closed session");
    drop(cancelled);
    drop(query);
    drop(template);
    drop(session);
    drop(run);
    drop(gate);
    let cleanup_correct = wait_for_live_allocations(allocation_baseline.saturating_add(
        usize::try_from(bench_harness::GROWTH_LIMIT_BYTES).expect("positive growth limit"),
    ));

    finish_sample(
        elapsed,
        entered
            && completed
            && outcome_correct
            && work_correct
            && mapped_correct
            && cleanup_correct,
        observed_mapped_bytes,
        metrics,
    )
}

fn close_and_retain(_: &()) -> Sample {
    let allocation_baseline = bench_harness::live_allocated_bytes();
    let run = ControlledRun::new(
        ControlledMatcher::new(SOURCE_FORMAT).with_candidates(vec![Candidate::new(1, 1, 0.99)]),
    );
    let first_session = run.core.open();
    let template = run.core.prepare("retain");
    let options = MatchOptions::from_defaults(template.defaults());

    let started = Instant::now();
    let retained_query = start_query(
        &first_session,
        template.clone(),
        options,
        OperationContext::new(),
    );
    run.core
        .capture
        .publish(0x40, Continuity::Continuous)
        .expect("published retained result");
    let retained = retained_query
        .wait(&bounded_operation())
        .expect("retained query completed");
    first_session
        .close(&bounded_operation())
        .expect("first close");
    first_session
        .close(&bounded_operation())
        .expect("repeated close");

    let pending_session = run.core.open();
    let pending = start_query(
        &pending_session,
        template.clone(),
        options,
        OperationContext::new(),
    );
    pending_session
        .close(&bounded_operation())
        .expect("closed pending session");
    let pending_outcome = pending
        .wait(&bounded_operation())
        .expect("pending query woke");

    let progress_session = run.core.open();
    let progressed = start_query(
        &progress_session,
        template,
        options,
        OperationContext::new(),
    );
    let publication_correct = run
        .core
        .capture
        .publish(0x41, Continuity::Continuous)
        .is_err_and(|error| error.status() == mado_pilot::Status::Closed);
    let progressed_outcome = progressed
        .wait(&bounded_operation())
        .expect("later query completed");
    let elapsed = started.elapsed();
    progress_session
        .close(&bounded_operation())
        .expect("closed progress session");
    let retained_still_owned = matches!(
        retained.as_ref(),
        TemplateTerminalOutcome::Matched(result)
            if result.result().matches().len() == 1
    );
    let outcomes_correct = publication_correct
        && retained_still_owned
        && matches!(
            pending_outcome.as_ref(),
            TemplateTerminalOutcome::SessionClosed
        )
        && matches!(
            progressed_outcome.as_ref(),
            TemplateTerminalOutcome::Matched(_)
        );
    let backend_runs = u64::try_from(run.matcher.find_count()).expect("count fits");
    let query_completions = u64::from(retained_still_owned)
        .saturating_add(u64::from(matches!(
            pending_outcome.as_ref(),
            TemplateTerminalOutcome::SessionClosed
        )))
        .saturating_add(u64::from(matches!(
            progressed_outcome.as_ref(),
            TemplateTerminalOutcome::Matched(_)
        )));
    let metrics = Some(QueryWorkMetrics {
        backend_runs,
        query_completions,
        producer_publications: 2,
        admitted: backend_runs,
        completed: backend_runs,
        ..QueryWorkMetrics::default()
    });
    let work_correct = backend_runs == 2 && query_completions == 3;
    let observed_mapped_bytes =
        u64::try_from(run.matcher.consistent_mapped_bytes().unwrap_or(0)).expect("bytes fit");
    let mapped_correct = observed_mapped_bytes == CONTROLLED_MAPPED_BYTES;

    drop(retained);
    drop(pending_outcome);
    drop(progressed_outcome);
    drop(retained_query);
    drop(pending);
    drop(progressed);
    drop(first_session);
    drop(pending_session);
    drop(progress_session);
    drop(run);
    let cleanup_correct = wait_for_live_allocations(allocation_baseline.saturating_add(
        usize::try_from(bench_harness::GROWTH_LIMIT_BYTES).expect("positive growth limit"),
    ));

    finish_sample(
        elapsed,
        outcomes_correct && work_correct && mapped_correct && cleanup_correct,
        observed_mapped_bytes,
        metrics,
    )
}

fn start_query(
    session: &Session,
    template: PreparedTemplate,
    options: MatchOptions,
    operation: OperationContext,
) -> TemplateQuery {
    session
        .start_template_watch(TemplateWatchRequest::new(template, options, operation))
        .expect("started query")
}

fn wait_progress(
    query: &TemplateQuery,
    predicate: impl Fn(TemplateQueryProgress) -> bool,
) -> TemplateQueryProgress {
    let deadline = Instant::now() + WAIT;
    loop {
        match query.poll() {
            TemplateQueryOutcome::Pending(progress) if predicate(progress) => return progress,
            TemplateQueryOutcome::Pending(_) if Instant::now() < deadline => {
                std::thread::yield_now();
            }
            TemplateQueryOutcome::Pending(progress) => {
                panic!("query progress condition timed out: {progress:?}")
            }
            TemplateQueryOutcome::Terminal(outcome) => {
                panic!("query became terminal before expected progress: {outcome:?}")
            }
        }
    }
}

fn wait_for_live_allocations(limit: usize) -> bool {
    let deadline = Instant::now() + WAIT;
    while bench_harness::live_allocated_bytes() > limit && Instant::now() < deadline {
        std::thread::yield_now();
    }
    bench_harness::live_allocated_bytes() <= limit
}

fn finish_sample(
    elapsed: Duration,
    correct: bool,
    mapped: u64,
    metrics: Option<QueryWorkMetrics>,
) -> Sample {
    let resident = peak_resident_bytes();
    let supported_target = bench_harness::RELEASE_TARGET != "not a declared release target";
    let mut sample = Sample::new(
        elapsed,
        correct && metrics.is_some() && (!supported_target || resident.is_some()),
        mapped,
    );
    if let Some(metrics) = metrics {
        sample = sample.with_query_work(metrics);
    }
    if let Some(bytes) = resident {
        sample = sample.with_peak_resident_bytes(bytes);
    }
    sample
}

fn observed_query_metrics(
    backend_runs: u64,
    backend_completions: u64,
    outcome: &TemplateTerminalOutcome,
    producer_publications: u64,
) -> QueryWorkMetrics {
    let matched = u64::from(matches!(outcome, TemplateTerminalOutcome::Matched(_)));
    QueryWorkMetrics {
        backend_runs,
        query_completions: matched,
        query_failures: u64::from(matches!(outcome, TemplateTerminalOutcome::Failed(_))),
        producer_publications,
        admitted: backend_runs,
        completed: backend_completions.saturating_mul(matched),
        failed: backend_runs.saturating_sub(backend_completions),
        ..QueryWorkMetrics::default()
    }
}
fn completed_query_metrics(backend_runs: u64, producer_publications: u64) -> QueryWorkMetrics {
    QueryWorkMetrics {
        backend_runs,
        query_completions: 1,
        producer_publications,
        admitted: backend_runs,
        completed: backend_runs,
        ..QueryWorkMetrics::default()
    }
}

fn mapped_bytes(region: PixelRect) -> Option<u64> {
    u64::from(region.width())
        .checked_mul(u64::from(region.height()))?
        .checked_mul(u64::from(SOURCE_FORMAT.bytes_per_pixel()))
}

fn exact_origins(result: &mado_pilot::MatchResult, expected: &[(i32, i32)]) -> bool {
    let mut observed: Vec<_> = result
        .matches()
        .iter()
        .map(|found| (found.bounds().left(), found.bounds().top()))
        .collect();
    observed.sort_unstable();
    let mut expected = expected.to_vec();
    expected.sort_unstable();
    observed == expected
        && result
            .matches()
            .iter()
            .all(|found| (found.score() - 1.0).abs() <= 1e-5)
}

fn replay_source(frames: Vec<Vec<u8>>) -> ReplaySource {
    let descriptor = FrameDescriptor::packed(match_fixtures::SCENE, SOURCE_FORMAT)
        .expect("valid replay descriptor");
    let frames: Vec<_> = frames
        .into_iter()
        .enumerate()
        .map(|(index, pixels)| {
            ReplayFrame::new(
                descriptor,
                MonotonicInstant::from_origin(Duration::from_millis(
                    u64::try_from(index).expect("small frame index") * 16,
                )),
                Continuity::Continuous,
                None,
                pixels.into_boxed_slice(),
            )
            .expect("valid replay frame")
        })
        .collect();
    ReplaySource::from_targets(vec![
        ReplayTarget::new("template-watch-qualification", frames).expect("valid replay target"),
    ])
    .expect("valid replay source")
}

fn full_scene_rect() -> PixelRect {
    PixelRect::new(
        0,
        0,
        i32::try_from(match_fixtures::SCENE.width()).expect("small scene width"),
        i32::try_from(match_fixtures::SCENE.height()).expect("small scene height"),
    )
    .expect("non-empty scene")
}

fn roi_rect() -> PixelRect {
    PixelRect::new(ROI.0, ROI.1, ROI.2, ROI.3).expect("valid qualification ROI")
}

fn bounded_operation() -> OperationContext {
    OperationContext::new()
        .with_timeout(WAIT)
        .expect("representable qualification timeout")
}

fn package_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/assets/phase1-slice")
}

fn fixture_digest() -> ContentDigest {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let mut combined = Vec::new();
    for sums in [
        fixtures.join("change-detection/g-005/SHA256SUMS"),
        fixtures.join("assets/phase1-slice/SHA256SUMS"),
    ] {
        combined.extend_from_slice(
            &std::fs::read(&sums)
                .unwrap_or_else(|error| panic!("tracked checksum manifest failed: {error}")),
        );
    }
    ContentDigest::of(&combined)
}

fn required_identity(arguments: &[String], name: &str, minimum_len: usize) -> String {
    let value = bench_harness::argument(arguments, name)
        .unwrap_or_else(|| panic!("qualification requires {name}"));
    assert!(
        value.len() >= minimum_len
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')),
        "qualification identity is not allowlisted"
    );
    value
}

fn enforce_qualification_oracles(workloads: &[Workload], plan: Plan) {
    for workload in workloads {
        assert_eq!(
            workload.incorrect(),
            0,
            "{} failed an oracle",
            workload.name()
        );
        assert!(
            workload.growth_bytes() <= bench_harness::GROWTH_LIMIT_BYTES,
            "{} exceeded retained growth",
            workload.name()
        );
        let work = workload
            .query_work()
            .unwrap_or_else(|| panic!("{} omitted query work", workload.name()));
        assert_eq!(
            work.producer_publications % u64::try_from(plan.samples()).expect("samples fit"),
            0,
            "{} lost producer accounting",
            workload.name()
        );
        assert_eq!(
            work.failed,
            0,
            "{} recorded backend failures",
            workload.name()
        );
        assert_eq!(
            work.query_failures,
            0,
            "{} recorded query failures",
            workload.name()
        );
        assert!(
            workload.peak_resident_bytes().is_some(),
            "{} omitted target-native peak resident memory",
            workload.name()
        );
    }
}

fn enforce_resource_budgets(workloads: &[Workload], heap_limit: usize, resident_limit: u64) {
    for workload in workloads {
        assert!(
            workload.peak_allocated_bytes() <= heap_limit,
            "{} exceeded the accepted live-Rust-heap ceiling: {} > {heap_limit}",
            workload.name(),
            workload.peak_allocated_bytes()
        );
        let resident = workload
            .peak_resident_bytes()
            .unwrap_or_else(|| panic!("{} omitted peak resident memory", workload.name()));
        assert!(
            resident <= resident_limit,
            "{} exceeded the accepted process peak-RSS ceiling: {resident} > {resident_limit}",
            workload.name()
        );
    }
}

#[cfg(all(target_arch = "aarch64", target_os = "macos", target_vendor = "apple"))]
fn enforce_target_budgets(workloads: &[Workload]) {
    bench_harness::enforce_latency_budgets(
        workloads,
        &bench_harness::PHASE4_APPLE_TEMPLATE_WATCH_LATENCY_BUDGETS,
    );
    enforce_resource_budgets(
        workloads,
        bench_harness::PHASE4_APPLE_TEMPLATE_WATCH_HEAP_LIMIT_BYTES,
        bench_harness::PHASE4_APPLE_TEMPLATE_WATCH_RESIDENT_LIMIT_BYTES,
    );
}

#[cfg(all(
    target_arch = "x86_64",
    target_os = "windows",
    target_env = "msvc",
    target_vendor = "pc"
))]
fn enforce_target_budgets(workloads: &[Workload]) {
    bench_harness::enforce_latency_budgets(
        workloads,
        &bench_harness::PHASE4_WINDOWS_REMEDIATED_TEMPLATE_WATCH_LATENCY_BUDGETS,
    );
    enforce_resource_budgets(
        workloads,
        bench_harness::PHASE4_WINDOWS_TEMPLATE_WATCH_HEAP_LIMIT_BYTES,
        bench_harness::PHASE4_WINDOWS_TEMPLATE_WATCH_RESIDENT_LIMIT_BYTES,
    );
}

#[cfg(not(any(
    all(target_arch = "aarch64", target_os = "macos", target_vendor = "apple"),
    all(
        target_arch = "x86_64",
        target_os = "windows",
        target_env = "msvc",
        target_vendor = "pc"
    )
)))]
fn enforce_target_budgets(_workloads: &[Workload]) {
    panic!("template-watch qualification budgets exist only for release targets");
}

#[cfg(all(target_arch = "aarch64", target_os = "macos", target_vendor = "apple"))]
fn peak_resident_bytes() -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: `usage` points to writable storage for one complete `rusage`, and
    // `RUSAGE_SELF` asks libc to initialize that storage for this process.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: successful `getrusage` initialized the complete `rusage` value.
    let usage = unsafe { usage.assume_init() };
    u64::try_from(usage.ru_maxrss)
        .ok()
        .filter(|bytes| *bytes > 0)
}

#[cfg(all(
    target_arch = "x86_64",
    target_os = "windows",
    target_env = "msvc",
    target_vendor = "pc"
))]
fn peak_resident_bytes() -> Option<u64> {
    let mut counters = PROCESS_MEMORY_COUNTERS::default();
    let bytes = u32::try_from(size_of::<PROCESS_MEMORY_COUNTERS>()).ok()?;
    // SAFETY: `GetCurrentProcess` returns the documented pseudo-handle for this
    // process, `counters` is writable for the complete structure declared by
    // `bytes`, and the call does not retain either pointer.
    unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &raw mut counters, bytes) }.ok()?;
    u64::try_from(counters.PeakWorkingSetSize)
        .ok()
        .filter(|bytes| *bytes > 0)
}

#[cfg(not(any(
    all(target_arch = "aarch64", target_os = "macos", target_vendor = "apple"),
    all(
        target_arch = "x86_64",
        target_os = "windows",
        target_env = "msvc",
        target_vendor = "pc"
    )
)))]
fn peak_resident_bytes() -> Option<u64> {
    None
}
