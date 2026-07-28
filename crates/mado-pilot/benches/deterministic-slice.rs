//! Timing, memory, and mapped-byte harness for the Phase 1 workloads, with a
//! correctness oracle on every sample.
//!
//! Gate `G-013` in `docs/validation-gates.md` is what sets a budget, and
//! `docs/benchmarks/` is where a set one lives. What is here is the harness that
//! produces the evidence and the oracle each workload is checked against —
//! because a latency number whose output was never checked is a timing
//! experiment rather than evidence, which is the rule `docs/performance.md`
//! states.
//!
//! Two modes, because the same oracles are worth running far more often than
//! the timings are:
//!
//! ```text
//! cargo test  --locked --workspace --all-targets            # oracles, three samples
//! cargo bench --locked --package mado-pilot --bench deterministic-slice -- \
//!     --hardware "..." --os-version "..."                   # full run, TOML report
//! ```
//!
//! The host is the operator's to state and nothing here guesses it. A CPU model
//! this program detected would be a guess recorded as a measurement condition.
//! The release target is the one exception: it is not detected but selected by
//! `cfg`, and anything that is not one of the two declared release targets says
//! so rather than being assembled from parts.

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use mado_pilot::replay::{ReplayFrame, ReplaySource, ReplayTarget};
use mado_pilot::{
    AssetPackage, ClipPolicy, ContentDigest, Continuity, CoordinateSpace, Engine, FindOutcome,
    FindRequest, Frame, FrameDescriptor, FrameRequest, MatchOptions, MemoryPackage,
    MonotonicInstant, OpenRequest, OperationContext, PackageSource, PixelFormat, PreparedTemplate,
    Rect, Session,
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

/// The target triple this build runs on, when it is one this project releases.
///
/// Selected rather than detected. `std::env::consts` can report the
/// architecture and the operating system but not the vendor or the ABI, and a
/// triple assembled from the parts that are available would be a guess printed
/// where a measurement condition belongs. A budget is valid only for the target
/// in its profile, so the wrong string here is worse than no string.
const RELEASE_TARGET: &str = if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
    "aarch64-apple-darwin"
} else if cfg!(all(
    target_arch = "x86_64",
    target_os = "windows",
    target_env = "msvc"
)) {
    "x86_64-pc-windows-msvc"
} else {
    "not a declared release target"
};

// --- Allocation accounting ---------------------------------------------------

/// Live heap bytes, and the high-water mark since it was last reset.
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

/// The system allocator, counting what it hands out and takes back.
///
/// Resident memory is what `docs/performance.md` names for `peak_memory` and
/// `steady_memory`, and it is the wrong instrument here. It is measured through
/// a different platform API on each release target, it moves with allocator
/// and operating-system behaviour that no MadoPilot change can affect, and on a
/// workload this small the noise is larger than the signal. Live heap bytes are
/// portable, are the same computation on both targets, and answer the question
/// a bounded-memory gate actually asks: does a repeated operation give back
/// what it took. The three measures this feeds are named separately in the
/// measure vocabulary so that neither reading is mistaken for the other.
struct Accounting;

// SAFETY: every method forwards to the system allocator with the layout it was
// given and returns exactly what it returned. The counters are plain relaxed
// arithmetic on the side and never influence which pointer is produced.
unsafe impl GlobalAlloc for Accounting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's contract for `alloc` is passed through unchanged.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            record(layout.size(), 0);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        record(0, layout.size());
        // SAFETY: as above.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: as above.
        let moved = unsafe { System.realloc(pointer, layout, new_size) };
        if !moved.is_null() {
            record(new_size, layout.size());
        }
        moved
    }
}

/// Applies one allocation and one release to the counters.
fn record(gained: usize, lost: usize) {
    let before = LIVE.fetch_add(gained, Ordering::Relaxed) + gained;
    LIVE.fetch_sub(lost, Ordering::Relaxed);

    // A peak that another thread raised higher stays; this only ever lifts it.
    PEAK.fetch_max(before, Ordering::Relaxed);
}

#[global_allocator]
static ALLOCATOR: Accounting = Accounting;

/// Live bytes now.
fn live() -> usize {
    LIVE.load(Ordering::Relaxed)
}

/// Restarts the high-water mark at the current live total.
fn reset_peak() {
    PEAK.store(live(), Ordering::Relaxed);
}

fn peak() -> usize {
    PEAK.load(Ordering::Relaxed)
}

// --- Running -----------------------------------------------------------------

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let full = arguments.iter().any(|argument| argument == "--bench");
    let host = Host::from(&arguments);
    let plan = if full { Plan::full() } else { Plan::smoke() };

    let workloads = [
        measure("replay_open", plan, open_session),
        measure("map_full_frame", plan, map_full_frame),
        measure("map_region_of_interest", plan, map_region),
        measure("load_package_directory", plan, load_directory),
        measure("load_package_memory", plan, load_memory),
        measure("load_package_archive", plan, load_archive),
        measure("prepare_and_match_cold", plan, prepare_and_match),
        measure("match_warm", plan, match_warm),
    ];

    let failures: usize = workloads.iter().map(|workload| workload.incorrect).sum();
    if full {
        report(&host, plan, &workloads);
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

/// What the operator says about the machine, because the program will not guess.
#[derive(Debug)]
struct Host {
    hardware: String,
    os_version: String,
}

impl Host {
    fn from(arguments: &[String]) -> Self {
        // `--label` predates the two specific arguments and named the host as
        // one string. It still fills the hardware field so the command recorded
        // in `docs/performance.md` keeps working.
        let label = argument(arguments, "--label");
        Self {
            hardware: argument(arguments, "--hardware")
                .or_else(|| label.clone())
                .unwrap_or_else(|| "unstated".to_owned()),
            os_version: argument(arguments, "--os-version")
                .unwrap_or_else(|| "unstated".to_owned()),
        }
    }
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

/// One workload's samples, and what they cost besides time.
#[derive(Debug)]
struct Workload {
    name: &'static str,
    oracle: &'static str,
    elapsed: Vec<Duration>,
    incorrect: usize,
    /// Frame bytes mapped into CPU memory per produced result. Zero for a
    /// workload that maps nothing.
    mapped: u64,
    /// One span covering the whole sampled run, divided by the sample count.
    iteration_span: Duration,
    /// The high-water mark of live heap bytes, above what was live before this
    /// workload's fixture was built.
    peak_bytes: usize,
    /// Live heap bytes at the end of sampling, above the same baseline, with the
    /// fixture still alive.
    steady_bytes: usize,
    /// Signed change in live heap bytes across the sampled run alone.
    growth_bytes: i64,
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

    /// Returns one sampled iteration's cost in milliseconds, from a single span.
    ///
    /// A per-iteration percentile disappears when the operation is faster than
    /// the host clock can express — on `x86_64-pc-windows-msvc`
    /// `map_full_frame` measures exactly zero, because a matching-format
    /// mapping is a reference-count increment. One clock read across hundreds
    /// of iterations recovers a number that granularity cannot swallow.
    ///
    /// It measures more than the operation does. Everything an iteration needs
    /// is inside the span: acquiring the frame, checking the oracle, dropping
    /// what the iteration produced. So it is an upper bound on the operation
    /// rather than a reading of it, and that is what makes it usable as a
    /// ceiling for a workload whose own fast path is too quick to time.
    fn iteration_span_ms(&self) -> f64 {
        self.iteration_span.as_secs_f64() * 1_000.0
    }
}

/// What one iteration of a workload reports.
#[derive(Debug)]
struct Sample {
    elapsed: Duration,
    correct: bool,
    /// Frame bytes this iteration mapped into CPU memory.
    mapped: u64,
}

impl Sample {
    /// A sample from a workload that maps nothing.
    const fn unmapped(elapsed: Duration, correct: bool) -> Self {
        Self {
            elapsed,
            correct,
            mapped: 0,
        }
    }
}

/// Runs `workload` through its warmup and its samples.
///
/// The three memory numbers are differences against two baselines rather than
/// absolute totals, because an absolute total would include every earlier
/// workload's retained samples and would grow down the report for a reason that
/// has nothing to do with the workload being measured. `before_fixture` is what
/// this workload's own footprint is measured against; `after_warmup` is what its
/// growth is measured against, so a one-time cost the first iterations paid is
/// not reported as a leak.
fn measure(name: &'static str, plan: Plan, workload: fn(&Fixture) -> Sample) -> Workload {
    // Allocated before the baseline is taken and never grown afterwards, so the
    // harness's own record of the run does not appear as the workload's memory.
    let mut elapsed = Vec::with_capacity(plan.samples);

    let before_fixture = live();
    let fixture = Fixture::new();
    for _ in 0..plan.warmup {
        workload(&fixture);
    }

    let after_warmup = live();
    reset_peak();

    let mut incorrect = 0;
    let mut mapped = 0;
    let span = Instant::now();
    for _ in 0..plan.samples {
        let sample = workload(&fixture);
        if !sample.correct {
            incorrect += 1;
        }
        mapped = sample.mapped;
        elapsed.push(sample.elapsed);
    }
    let span = span.elapsed();

    let ending = live();
    Workload {
        name,
        oracle: oracle(name),
        elapsed,
        incorrect,
        mapped,
        iteration_span: span / u32::try_from(plan.samples).unwrap_or(u32::MAX),
        peak_bytes: peak().saturating_sub(before_fixture),
        steady_bytes: ending.saturating_sub(before_fixture),
        growth_bytes: i64::try_from(ending).unwrap_or(i64::MAX)
            - i64::try_from(after_warmup).unwrap_or(i64::MAX),
    }
}

/// Returns what each workload's output is checked against.
fn oracle(name: &str) -> &'static str {
    match name {
        "replay_open" => "the session reports the source's own extent and pixel format",
        "map_full_frame" => "the mapping covers the whole frame and reports its exact identity",
        "map_region_of_interest" => "the mapping covers the requested region and no more",
        "load_package_directory" => "the package declares its six tracked templates",
        "load_package_memory" | "load_package_archive" => {
            "the committed package equals the one the same files commit as a directory"
        }
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
    /// The `G-014` tiny package as a directory, which the other two source
    /// kinds are checked against.
    tiny_directory: AssetPackage,
    /// The same files, described in caller-owned memory.
    tiny_memory: MemoryPackage,
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
        let tiny_directory = engine
            .load_package(&PackageSource::directory(tiny_directory()), &operation)
            .expect("the tracked G-014 tiny package loads");

        Self {
            engine,
            operation,
            session,
            template,
            tiny_directory,
            tiny_memory: tiny_memory_package(),
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

    Sample::unmapped(elapsed, correct)
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
        mapped: mapping.bytes().len() as u64,
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
        mapped: mapping.bytes().len() as u64,
    }
}

fn load_directory(fixture: &Fixture) -> Sample {
    let started = Instant::now();
    let package = fixture
        .engine
        .load_package(
            &PackageSource::directory(tiny_directory()),
            &fixture.operation,
        )
        .expect("loaded");
    let elapsed = started.elapsed();

    Sample::unmapped(
        elapsed,
        package.template_count() == 6 && package == fixture.tiny_directory,
    )
}

fn load_memory(fixture: &Fixture) -> Sample {
    // The description is built once, in the fixture: assembling it reads six
    // files from disk, which is the directory workload's cost and not this
    // one's. What is measured here is validating and committing a package whose
    // bytes the caller already holds.
    let source = PackageSource::memory(fixture.tiny_memory.clone());
    let started = Instant::now();
    let package = fixture
        .engine
        .load_package(&source, &fixture.operation)
        .expect("loaded");
    let elapsed = started.elapsed();

    Sample::unmapped(elapsed, package == fixture.tiny_directory)
}

fn load_archive(fixture: &Fixture) -> Sample {
    let started = Instant::now();
    let package = fixture
        .engine
        .load_package(
            &PackageSource::archive_file(tiny_archive()),
            &fixture.operation,
        )
        .expect("loaded");
    let elapsed = started.elapsed();

    Sample::unmapped(elapsed, package == fixture.tiny_directory)
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
        mapped: searched_bytes(fixture, &outcome),
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
        mapped: searched_bytes(fixture, &outcome),
    }
}

/// Returns the frame bytes one search mapped into CPU memory.
///
/// Derived rather than observed, from two things the result reports: the region
/// that was actually searched, after any clipping, and the pixel format the
/// backend requires. The matcher maps the searched region into that format
/// exactly once per search, so this is the rule it follows rather than an
/// estimate of it. A backend that mapped twice would break the rule, not this
/// arithmetic.
fn searched_bytes(fixture: &Fixture, outcome: &FindOutcome) -> u64 {
    let searched = outcome.result().searched();
    let format = fixture.engine.backend().format();

    u64::from(searched.width()) * u64::from(searched.height()) * u64::from(format.bytes_per_pixel())
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

// --- Reporting ---------------------------------------------------------------

/// Prints a profile-shaped report with no budget in it.
fn report(host: &Host, plan: Plan, workloads: &[Workload]) {
    println!("format_version = 1");
    println!();
    println!("[benchmark]");
    println!("id = \"phase-1-deterministic-slice\"");
    println!("workload = \"the Phase 1 deterministic replay workflow, eight operations\"");
    println!("phase = \"1\"");
    println!("status = \"harness-output\"");
    println!("normative = false");
    println!("budgets_set = false");
    println!("# A committed profile under docs/benchmarks/ carries the budgets.");
    println!();
    println!("[profile]");
    println!(
        "fixture = \"fixtures/assets/phase1-slice for matching, \
         fixtures/assets/g-014/valid for loading, \
         mado-pilot-testkit match_fixtures for the scene\""
    );
    println!("fixture_sha256 = \"{}\"", fixture_digest());
    println!("release_target = \"{RELEASE_TARGET}\"");
    println!("hardware = \"{}\"", escape(&host.hardware));
    println!("os_version = \"{}\"", escape(&host.os_version));
    println!("build_profile = \"{}\"", build_profile());
    println!("warmup_iterations = {}", plan.warmup);
    println!("sample_count = {}", plan.samples);
    println!(
        "correctness_oracle = \"every retained sample is checked; \
         each measurement states its own oracle\""
    );
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
        println!("iteration_span_ms = {:.6}", workload.iteration_span_ms());
        println!("mapped_bytes_per_result = {}", workload.mapped);
        println!("peak_allocated_bytes = {}", workload.peak_bytes);
        println!("steady_allocated_bytes = {}", workload.steady_bytes);
        println!("allocated_growth_bytes = {}", workload.growth_bytes);
        println!();
    }
}

/// Returns one digest covering every tracked fixture this harness reads.
///
/// Each fixture set is pinned by its own `SHA256SUMS`, so hashing those files
/// pins every file they list with one number. A file added to a fixture set and
/// left out of its `SHA256SUMS` is invisible here, which is the same hole the
/// checksum files have and is why they are generated from the tree.
fn fixture_digest() -> ContentDigest {
    let mut combined = Vec::new();
    for sums in [
        fixtures().join("assets/phase1-slice/SHA256SUMS"),
        fixtures().join("assets/g-014/SHA256SUMS"),
    ] {
        combined.extend_from_slice(&std::fs::read(&sums).unwrap_or_else(|error| {
            panic!(
                "{} is a tracked fixture checksum file: {error}",
                sums.display()
            )
        }));
    }

    ContentDigest::of(&combined)
}

/// Returns how this program was built, as far as it can know.
fn build_profile() -> String {
    format!(
        "cargo bench, default features, debug_assertions={}",
        cfg!(debug_assertions)
    )
}

/// Returns the value of a `--name value` or `--name=value` argument.
fn argument(arguments: &[String], name: &str) -> Option<String> {
    let mut iterator = arguments.iter();
    let prefix = format!("{name}=");
    while let Some(argument) = iterator.next() {
        if argument == name {
            return iterator.next().cloned();
        }
        if let Some(value) = argument.strip_prefix(&prefix) {
            return Some(value.to_owned());
        }
    }
    None
}

fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

// --- Fixtures ----------------------------------------------------------------

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

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn package_root() -> PathBuf {
    fixtures().join("assets/phase1-slice")
}

/// The `G-014` tiny package as a directory.
///
/// The loading workloads use it rather than the two-template slice package
/// because it is the only package tracked in more than one form, which is what
/// makes the three source kinds comparable: same bytes, same six templates,
/// three containers. Its equivalence across the three is asserted by
/// `mado-pilot-assets`, so a fixture that drifted would fail a test before it
/// reached a benchmark.
fn tiny_directory() -> PathBuf {
    fixtures().join("assets/g-014/valid/tiny-directory")
}

fn tiny_archive() -> PathBuf {
    fixtures().join("assets/g-014/valid/valid-tiny.zip")
}

/// Describes the tiny package's files in caller-owned memory.
fn tiny_memory_package() -> MemoryPackage {
    let root = tiny_directory();
    let mut package = MemoryPackage::new();
    for relative in tracked_files(&root, &root) {
        let bytes = std::fs::read(root.join(&relative)).expect("a readable fixture");
        package = package.with_entry(relative, bytes);
    }

    package
}

/// Lists every file under `directory`, as package-relative slash-separated
/// names.
fn tracked_files(root: &Path, directory: &Path) -> Vec<String> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(directory).expect("a readable fixture directory") {
        let path = entry.expect("a readable directory entry").path();
        if path.is_dir() {
            found.extend(tracked_files(root, &path));
            continue;
        }
        found.push(
            path.strip_prefix(root)
                .expect("a path below the root")
                .components()
                .map(|component| component.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/"),
        );
    }
    found.sort();

    found
}
