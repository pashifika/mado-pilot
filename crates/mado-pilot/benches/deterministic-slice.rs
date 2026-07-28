//! Timing harness for the six Phase 1 workloads, with a correctness oracle on
//! every sample.
//!
//! This sets **no numeric budget**. Gate `G-013` in `docs/validation-gates.md`
//! is what sets one, and it needs measurements from both release targets that
//! do not exist yet. What exists here is the harness that produces them, and
//! the oracle each workload is checked against — because a latency number whose
//! output was never checked is a timing experiment rather than evidence, which
//! is the rule `docs/performance.md` states.
//!
//! Two modes, because the same oracles are worth running far more often than
//! the timings are:
//!
//! ```text
//! cargo test  --locked --workspace --all-targets            # oracles, three samples
//! cargo bench --locked --package mado-pilot -- --label "..."  # full run, TOML report
//! ```
//!
//! The label is the operator's to supply and nothing here guesses it. A CPU
//! model this program detected would be a guess recorded as a measurement
//! condition.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use mado_pilot::replay::{ReplayFrame, ReplaySource, ReplayTarget};
use mado_pilot::{
    ClipPolicy, Continuity, CoordinateSpace, Engine, FindOutcome, FindRequest, Frame,
    FrameDescriptor, FrameRequest, MatchOptions, MonotonicInstant, OpenRequest, OperationContext,
    PackageSource, PixelFormat, PreparedTemplate, Rect, Session,
};
use mado_pilot_testkit::match_fixtures;

/// The layout the replay source publishes.
const SOURCE_FORMAT: PixelFormat = PixelFormat::Rgba8;

/// Where the planted copies of `panel.patch` sit, and therefore what every
/// matching sample must find.
const PLANTED: [(i32, i32); 2] = [(20, 12), (60, 40)];

/// How far a score may sit from an exact correlation and still be one.
const TOLERANCE: f64 = 1e-5;

/// The region of interest the partial-mapping workload maps.
const ROI: (f64, f64, f64, f64) = (16.0, 8.0, 64.0, 40.0);

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let full = arguments.iter().any(|argument| argument == "--bench");
    let label = label(&arguments).unwrap_or_else(|| "unlabelled".to_owned());
    let plan = if full { Plan::full() } else { Plan::smoke() };

    let workloads = [
        measure("replay_open", plan, open_session),
        measure("map_full_frame", plan, map_full_frame),
        measure("map_region_of_interest", plan, map_region),
        measure("load_package", plan, load_package),
        measure("prepare_and_match_cold", plan, prepare_and_match),
        measure("match_warm", plan, match_warm),
    ];

    let failures: usize = workloads.iter().map(|workload| workload.incorrect).sum();
    if full {
        report(&label, plan, &workloads);
    } else {
        println!(
            "deterministic-slice: {} workloads, {} samples each, {failures} oracle failure(s)",
            workloads.len(),
            plan.samples
        );
    }

    assert_eq!(
        failures, 0,
        "a workload produced an output its oracle rejected"
    );
}

/// How many iterations a run discards and how many it keeps.
#[derive(Debug, Clone, Copy)]
struct Plan {
    warmup: usize,
    samples: usize,
}

impl Plan {
    /// Enough samples for the oracles, not enough for a percentile.
    const fn smoke() -> Self {
        Self {
            warmup: 1,
            samples: 3,
        }
    }

    const fn full() -> Self {
        Self {
            warmup: 20,
            samples: 200,
        }
    }
}

/// One workload's samples and how many of them failed their oracle.
#[derive(Debug)]
struct Workload {
    name: &'static str,
    oracle: &'static str,
    elapsed: Vec<Duration>,
    incorrect: usize,
}

impl Workload {
    /// Returns the `percentile`-th sample, in milliseconds.
    fn percentile(&self, percentile: f64) -> f64 {
        let mut sorted = self.elapsed.clone();
        sorted.sort_unstable();
        let last = sorted.len().saturating_sub(1);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "an index into a sample vector whose length is far below the f64 mantissa"
        )]
        let index = ((last as f64) * percentile).round() as usize;
        sorted.get(index).copied().unwrap_or_default().as_secs_f64() * 1_000.0
    }
}

/// What one iteration of a workload reports.
#[derive(Debug)]
struct Sample {
    elapsed: Duration,
    correct: bool,
}

/// Runs `workload` through its warmup and its samples.
fn measure(name: &'static str, plan: Plan, workload: fn(&Fixture) -> Sample) -> Workload {
    let fixture = Fixture::new();
    for _ in 0..plan.warmup {
        workload(&fixture);
    }

    let mut elapsed = Vec::with_capacity(plan.samples);
    let mut incorrect = 0;
    for _ in 0..plan.samples {
        let sample = workload(&fixture);
        if !sample.correct {
            incorrect += 1;
        }
        elapsed.push(sample.elapsed);
    }

    Workload {
        name,
        oracle: oracle(name),
        elapsed,
        incorrect,
    }
}

/// Returns what each workload's output is checked against.
fn oracle(name: &str) -> &'static str {
    match name {
        "replay_open" => "the session reports the source's own extent and pixel format",
        "map_full_frame" => "the mapping covers the whole frame and reports its exact identity",
        "map_region_of_interest" => "the mapping covers the requested region and no more",
        "load_package" => "the package declares both tracked templates",
        _ => {
            "the two planted copies are found at their planted offsets, each scoring 1.0 within 1e-5"
        }
    }
}

/// Everything a workload needs that is not what it measures.
#[derive(Debug)]
struct Fixture {
    engine: Engine,
    operation: OperationContext,
    session: Session,
    template: PreparedTemplate,
}

impl Fixture {
    fn new() -> Self {
        let engine = mado_pilot::replay_engine(scene_source())
            .expect("an OpenCV 4 development installation");
        let operation = OperationContext::new();
        let targets = engine.discover(&operation).expect("discovered");
        let session = engine
            .open(targets[0].id(), &OpenRequest::new(), &operation)
            .expect("opened");
        let package = engine
            .load_package(&PackageSource::directory(package_root()), &operation)
            .expect("the tracked example package loads");
        let template = engine
            .prepare_from_package(&package, "panel.patch", &operation)
            .expect("prepared");

        Self {
            engine,
            operation,
            session,
            template,
        }
    }

    fn frame(&self) -> Frame {
        self.session
            .acquire_frame(&FrameRequest::latest(), &self.operation)
            .expect("a published frame")
    }
}

fn open_session(fixture: &Fixture) -> Sample {
    let started = Instant::now();
    let targets = fixture
        .engine
        .discover(&fixture.operation)
        .expect("discovered");
    let session = fixture
        .engine
        .open(targets[0].id(), &OpenRequest::new(), &fixture.operation)
        .expect("opened");
    let elapsed = started.elapsed();

    let correct = session.description().extent() == match_fixtures::SCENE
        && session.description().format() == SOURCE_FORMAT;
    session.close(&fixture.operation).expect("closed");

    Sample { elapsed, correct }
}

fn map_full_frame(fixture: &Fixture) -> Sample {
    let frame = fixture.frame();
    let started = Instant::now();
    let mapping = frame
        .map(SOURCE_FORMAT, &fixture.operation)
        .expect("mapped");
    let elapsed = started.elapsed();

    Sample {
        elapsed,
        correct: mapping.stamp() == frame.stamp()
            && mapping.bytes().len() == frame.descriptor().byte_len(),
    }
}

fn map_region(fixture: &Fixture) -> Sample {
    let frame = fixture.frame();
    let region = frame
        .view(
            Rect::new(CoordinateSpace::CapturePixels, ROI.0, ROI.1, ROI.2, ROI.3)
                .expect("a valid region"),
            ClipPolicy::Reject,
        )
        .expect("inside the frame");
    let started = Instant::now();
    let mapping = region
        .map(SOURCE_FORMAT, &fixture.operation)
        .expect("mapped");
    let elapsed = started.elapsed();

    let expected = u64::from(region.region().width())
        * u64::from(region.region().height())
        * u64::from(SOURCE_FORMAT.bytes_per_pixel());
    Sample {
        elapsed,
        correct: mapping.region() == region.region()
            && u64::try_from(mapping.bytes().len()).is_ok_and(|mapped| mapped == expected),
    }
}

fn load_package(fixture: &Fixture) -> Sample {
    let started = Instant::now();
    let package = fixture
        .engine
        .load_package(
            &PackageSource::directory(package_root()),
            &fixture.operation,
        )
        .expect("loaded");
    let elapsed = started.elapsed();

    Sample {
        elapsed,
        correct: package.template_count() == 2 && package.resolve_template("panel.patch").is_ok(),
    }
}

fn prepare_and_match(fixture: &Fixture) -> Sample {
    let frame = fixture.frame();
    let package = fixture
        .engine
        .load_package(
            &PackageSource::directory(package_root()),
            &fixture.operation,
        )
        .expect("loaded");

    // Cold means the template is compiled inside the measured window, which is
    // what a caller pays the first time it looks for something.
    let started = Instant::now();
    let template = fixture
        .engine
        .prepare_from_package(&package, "panel.patch", &fixture.operation)
        .expect("prepared");
    let outcome = fixture
        .session
        .find_template(
            &FindRequest::exact(
                &frame,
                &template,
                MatchOptions::from_defaults(template.defaults()),
            ),
            &fixture.operation,
        )
        .expect("searched");
    let elapsed = started.elapsed();

    Sample {
        elapsed,
        correct: planted(&outcome),
    }
}

fn match_warm(fixture: &Fixture) -> Sample {
    let frame = fixture.frame();
    let options = MatchOptions::from_defaults(fixture.template.defaults());

    let started = Instant::now();
    let outcome = fixture
        .session
        .find_template(
            &FindRequest::exact(&frame, &fixture.template, options),
            &fixture.operation,
        )
        .expect("searched");
    let elapsed = started.elapsed();

    Sample {
        elapsed,
        correct: planted(&outcome),
    }
}

/// Reports whether an outcome found exactly the planted copies.
///
/// Compared as a set. Two byte-identical copies correlate at one to within the
/// tolerance, so which of them the result puts first rests on a difference
/// smaller than the tolerance, and an oracle that asserted an order would be
/// asserting the host's rounding rather than the workload's correctness.
fn planted(outcome: &FindOutcome) -> bool {
    let result = outcome.result();
    let mut origins: Vec<(i32, i32)> = result
        .matches()
        .iter()
        .map(|found| (found.bounds().left(), found.bounds().top()))
        .collect();
    origins.sort_unstable_by_key(|&(left, top)| (top, left));

    let mut expected = PLANTED.to_vec();
    expected.sort_unstable_by_key(|&(left, top)| (top, left));

    result.stamp() == outcome.frame().stamp()
        && origins == expected
        && result
            .matches()
            .iter()
            .all(|found| (found.score() - 1.0).abs() <= TOLERANCE)
}

/// Prints a profile-shaped report with no budget in it.
fn report(label: &str, plan: Plan, workloads: &[Workload]) {
    println!("format_version = 1");
    println!();
    println!("[benchmark]");
    println!("id = \"phase-1-deterministic-slice\"");
    println!("workload = \"the Phase 1 deterministic replay workflow, six operations\"");
    println!("phase = \"1\"");
    println!("status = \"harness-output\"");
    println!("normative = false");
    println!("budgets_set = false");
    println!("# G-013 sets numeric budgets; this run records measurements only.");
    println!();
    println!("[profile]");
    println!(
        "fixture = \"mado-pilot-testkit match_fixtures scene, fixtures/assets/phase1-slice package\""
    );
    // Arch and operating system are what the program can know. The exact target
    // triple, the CPU model, and the operating-system build are the operator's
    // to state when this output is turned into a tracked profile.
    println!("arch = \"{}\"", std::env::consts::ARCH);
    println!("os = \"{}\"", std::env::consts::OS);
    println!("label = \"{}\"", escape(label));
    println!("build_profile = \"cargo bench, default features\"");
    println!("warmup_iterations = {}", plan.warmup);
    println!("sample_count = {}", plan.samples);
    println!(
        "queue_policy = \"none; every Phase 1 operation is synchronous and no work is queued\""
    );
    println!();

    for workload in workloads {
        println!("[[measurement]]");
        println!("workload = \"{}\"", workload.name);
        println!("correctness_oracle = \"{}\"", workload.oracle);
        println!("result_correctness = {}", workload.incorrect);
        println!("latency_p50_ms = {:.6}", workload.percentile(0.50));
        println!("latency_p95_ms = {:.6}", workload.percentile(0.95));
        println!();
    }
}

/// Returns the `--label` argument, when one was supplied.
fn label(arguments: &[String]) -> Option<String> {
    let mut iterator = arguments.iter();
    while let Some(argument) = iterator.next() {
        if argument == "--label" {
            return iterator.next().cloned();
        }
        if let Some(value) = argument.strip_prefix("--label=") {
            return Some(value.to_owned());
        }
    }
    None
}

fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn scene_source() -> ReplaySource {
    let descriptor =
        FrameDescriptor::packed(match_fixtures::SCENE, SOURCE_FORMAT).expect("a valid descriptor");
    let pixels = match_fixtures::scene_pixels(SOURCE_FORMAT);
    let frame = ReplayFrame::new(
        descriptor,
        MonotonicInstant::ORIGIN,
        Continuity::Continuous,
        None,
        pixels.into_boxed_slice(),
    )
    .expect("a valid replay frame");

    ReplaySource::from_targets(vec![
        ReplayTarget::new("panel", vec![frame]).expect("a valid target"),
    ])
    .expect("a valid source")
}

fn package_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/assets/phase1-slice")
}
