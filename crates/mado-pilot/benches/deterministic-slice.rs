//! Timing, memory, and mapped-byte measurements for the Phase 1 Rust workflow,
//! with a correctness oracle on every sample.
//!
//! Gate `G-013` in `docs/validation-gates.md` is what sets a budget, and
//! `docs/benchmarks/` is where the set ones live. What is here are the eight
//! workloads and the oracle each is checked against — because a latency number
//! whose output was never checked is a timing experiment rather than evidence,
//! which is the rule `docs/performance.md` states.
//!
//! The sampling loop, the allocation accounting, and the report belong to
//! `mado_pilot_testkit::bench_harness`, which the C boundary benchmark in
//! `mado-pilot-capi` also uses, so the two emit the same profile shape.
//!
//! Two modes, because the same oracles are worth running far more often than
//! the timings are:
//!
//! ```text
//! cargo test  --locked --workspace --all-targets            # oracles, three samples
//! cargo bench --locked --package mado-pilot --bench deterministic-slice -- \
//!     --hardware "..." --os-version "..."                   # full run, TOML report
//! ```

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use mado_pilot::replay::{ReplayFrame, ReplaySource, ReplayTarget};
use mado_pilot::{
    AssetPackage, ClipPolicy, ContentDigest, Continuity, CoordinateSpace, Engine, FindOutcome,
    FindRequest, Frame, FrameDescriptor, FrameRequest, MatchOptions, MemoryPackage,
    MonotonicInstant, OpenRequest, OperationContext, PackageSource, PixelFormat, PreparedTemplate,
    Rect, Session,
};
use mado_pilot_testkit::bench_harness::{Accounting, Benchmark, Plan, Profile, Sample, measure};
use mado_pilot_testkit::{bench_harness, match_fixtures};

#[global_allocator]
static ALLOCATOR: Accounting = Accounting;

/// The layout the replay source publishes.
const SOURCE_FORMAT: PixelFormat = PixelFormat::Rgba8;

/// Where the planted copies of `panel.patch` sit, and therefore what every
/// matching sample must find.
const PLANTED: [(i32, i32); 2] = [(20, 12), (60, 40)];

/// How far a score may sit from an exact correlation and still be one.
const TOLERANCE: f64 = 1e-5;

/// The region of interest the partial-mapping workload maps, as left, top,
/// right, and bottom: 48 by 32 inside the 96 by 64 frame, which is 6144 bytes
/// at four bytes per pixel. The committed profiles state that byte count as a
/// budget, so the extent is written here rather than left to be derived.
const ROI: (f64, f64, f64, f64) = (16.0, 8.0, 64.0, 40.0);

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let plan = Plan::from(&arguments);
    let (hardware, os_version) = Profile::host(&arguments);

    let workloads = [
        measure(
            "replay_open",
            "the session reports the source's own extent and pixel format",
            plan,
            Fixture::new,
            open_session,
        ),
        measure(
            "map_full_frame",
            "the mapping covers the whole frame and reports its exact identity",
            plan,
            Fixture::new,
            map_full_frame,
        ),
        measure(
            "map_region_of_interest",
            "the mapping covers the requested region and no more",
            plan,
            Fixture::new,
            map_region,
        ),
        measure(
            "load_package_directory",
            "the package declares its six tracked templates",
            plan,
            Fixture::new,
            load_directory,
        ),
        measure(
            "load_package_memory",
            "the committed package equals the one the same files commit as a directory",
            plan,
            Fixture::new,
            load_memory,
        ),
        measure(
            "load_package_archive",
            "the committed package equals the one the same files commit as a directory",
            plan,
            Fixture::new,
            load_archive,
        ),
        measure(
            PLANTED_ORACLE.0,
            PLANTED_ORACLE.1,
            plan,
            Fixture::new,
            prepare_and_match,
        ),
        measure(
            "match_warm",
            PLANTED_ORACLE.1,
            plan,
            Fixture::new,
            match_warm,
        ),
    ];

    if arguments.iter().any(|argument| argument == "--bench") {
        bench_harness::report(
            &Benchmark {
                id: "phase-1-deterministic-slice",
                workload: "the Phase 1 deterministic replay workflow, eight operations",
                phase: "1",
            },
            &Profile {
                fixture: "fixtures/assets/phase1-slice for matching, \
                          fixtures/assets/g-014/valid for loading, \
                          mado-pilot-testkit match_fixtures for the scene"
                    .to_owned(),
                fixture_sha256: fixture_digest().to_string(),
                benchmark_executable_sha256: None,
                hardware,
                os_version,
                deployment_target: None,
                build_profile: format!(
                    "cargo bench, default features, debug_assertions={}",
                    cfg!(debug_assertions)
                ),
                correctness_oracle: "every retained sample is checked; \
                                     each measurement states its own oracle",
                queue_policy: "none; every Phase 1 operation is synchronous \
                               and no work is queued",
                notes: None,
            },
            plan,
            &workloads,
        );
    } else {
        bench_harness::summarize("deterministic-slice", plan, &workloads);
    }

    // After the report, so a run that fails a gate still emits the numbers that
    // explain the failure.
    bench_harness::enforce_hard_budgets(&workloads);
    MAPPING_PATHS.assert_every_iteration_took_the_path_its_bytes_assume(plan);
}

/// Which path each mapping iteration took, counted as it took it.
///
/// `mapped_bytes_per_result` is `mapping.bytes().len()` for both mapping
/// workloads, and that number is the same whether the mapping shared the
/// frame's storage or copied it — so no budget on it can see a mapping that
/// stopped sharing. What can see it is the decision itself: a full-frame
/// mapping in the source format shares the frame's storage, and a region
/// mapping owns its packed copy.
///
/// Every iteration is counted, warmup included, because a mapping that shared
/// on its first call and copied afterwards satisfies a check made once and is
/// exactly the regression a per-result byte count cannot see either.
///
/// Counted here rather than folded into a sample's `correct` flag: that flag is
/// what each profile's recorded `correctness_oracle` string describes and what
/// its `result_correctness` counts, and this is a property of the path rather
/// than of a result. Asserted once after the report, so it fails the benchmark
/// on both the `cargo bench` and the `cargo test` path, which is the
/// enforcement a hard budget gets.
///
/// Each count is taken after the sample's own `elapsed` reading and allocates
/// nothing, so no latency percentile and no allocation number can move. It does
/// land inside `iteration_span`, which the harness times across the whole
/// sample loop, so two relaxed atomic increments per iteration are charged
/// against the sub-microsecond `iteration_span_ms` ceiling both target profiles
/// state for `map_full_frame`. That charge is expected to be nanoseconds but is
/// measured on neither release target; read a miss on that ceiling with it in
/// mind.
static MAPPING_PATHS: MappingPaths = MappingPaths::new();

/// The tally [`MAPPING_PATHS`] keeps.
#[derive(Debug)]
struct MappingPaths {
    full_frames: AtomicU64,
    /// Whole-frame mappings that copied, which is the regression.
    copied_full_frames: AtomicU64,
    regions: AtomicU64,
    /// Region mappings that shared, which is the other one.
    shared_regions: AtomicU64,
}

impl MappingPaths {
    const fn new() -> Self {
        Self {
            full_frames: AtomicU64::new(0),
            copied_full_frames: AtomicU64::new(0),
            regions: AtomicU64::new(0),
            shared_regions: AtomicU64::new(0),
        }
    }

    /// Records how one whole-frame mapping in the source format came out.
    fn record_full_frame(&self, shared: bool) {
        self.full_frames.fetch_add(1, Ordering::Relaxed);
        if !shared {
            self.copied_full_frames.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Records how one region mapping came out.
    fn record_region(&self, shared: bool) {
        self.regions.fetch_add(1, Ordering::Relaxed);
        if shared {
            self.shared_regions.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Fails the run when any iteration took the other path.
    ///
    /// # Panics
    ///
    /// Panics when a whole-frame mapping copied, when a region mapping shared,
    /// or when a workload was observed fewer times than the run retained
    /// samples — the last so that a benchmark which stopped mapping cannot pass
    /// this by having nothing to report.
    fn assert_every_iteration_took_the_path_its_bytes_assume(&self, plan: Plan) {
        let full_frames = self.full_frames.load(Ordering::Relaxed);
        let copied = self.copied_full_frames.load(Ordering::Relaxed);
        let regions = self.regions.load(Ordering::Relaxed);
        let shared = self.shared_regions.load(Ordering::Relaxed);

        assert_eq!(
            copied, 0,
            "map_full_frame maps a whole frame in its own format, which shares \
             the frame's storage rather than copying it, but {copied} of \
             {full_frames} iterations copied; a copy costs the frame's bytes \
             per result and reports the same mapped_bytes_per_result as sharing"
        );
        assert_eq!(
            shared, 0,
            "map_region_of_interest maps a sub-rectangle, which owns a packed \
             copy, but {shared} of {regions} iterations shared; a shared \
             mapping is the whole frame's storage reported under a region's \
             byte count"
        );

        let retained = plan.samples() as u64;
        assert!(
            full_frames >= retained && regions >= retained,
            "each mapping workload runs once per warmup and retained \
             iteration, so at least {retained} of each were expected, but \
             {full_frames} whole-frame and {regions} region mappings were \
             observed; a count below that means this check passed because \
             nothing mapped"
        );
    }
}

/// The name and oracle the two matching workloads share.
const PLANTED_ORACLE: (&str, &str) = (
    "prepare_and_match_cold",
    "the two planted copies are found at their planted offsets, each scoring 1.0 within 1e-5",
);

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

    MAPPING_PATHS.record_full_frame(mapping.is_shared());
    Sample::new(
        elapsed,
        mapping.stamp() == frame.stamp() && mapping.bytes().len() == frame.descriptor().byte_len(),
        mapping.bytes().len() as u64,
    )
}

fn map_region(fixture: &Fixture) -> Sample {
    let frame = fixture.frame();
    let region = region_of_interest(&frame);
    let started = Instant::now();
    let mapping = region
        .map(SOURCE_FORMAT, &fixture.operation)
        .expect("mapped");
    let elapsed = started.elapsed();

    MAPPING_PATHS.record_region(mapping.is_shared());
    let expected = u64::from(region.region().width())
        * u64::from(region.region().height())
        * u64::from(SOURCE_FORMAT.bytes_per_pixel());
    Sample::new(
        elapsed,
        mapping.region() == region.region()
            && u64::try_from(mapping.bytes().len()).is_ok_and(|mapped| mapped == expected),
        mapping.bytes().len() as u64,
    )
}

/// Returns the view the region-mapping workload maps.
///
/// Built outside every measured window: what the workload times is the mapping,
/// not the arithmetic that describes the rectangle.
fn region_of_interest(frame: &Frame) -> mado_pilot::FrameView {
    frame
        .view(
            Rect::new(CoordinateSpace::CapturePixels, ROI.0, ROI.1, ROI.2, ROI.3)
                .expect("a valid region"),
            ClipPolicy::Reject,
        )
        .expect("inside the frame")
}

fn load_directory(fixture: &Fixture) -> Sample {
    // Built outside the window, as in every other workload here: naming a path
    // is the caller's arithmetic and reading the package is what is measured.
    let source = PackageSource::directory(tiny_directory());
    let started = Instant::now();
    let package = fixture
        .engine
        .load_package(&source, &fixture.operation)
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
    // As `load_directory`.
    let source = PackageSource::archive_file(tiny_archive());
    let started = Instant::now();
    let package = fixture
        .engine
        .load_package(&source, &fixture.operation)
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

    Sample::new(
        elapsed,
        planted(&outcome),
        searched_bytes(fixture, &outcome),
    )
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

    Sample::new(
        elapsed,
        planted(&outcome),
        searched_bytes(fixture, &outcome),
    )
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

// --- Fixtures ----------------------------------------------------------------

/// Returns one digest covering every tracked fixture this benchmark reads.
///
/// Each fixture set is pinned by its own `SHA256SUMS`, so hashing those files
/// pins every file they list with one number. A file added to a fixture set and
/// left out of its `SHA256SUMS` is invisible here, which is the same hole the
/// checksum files have and is why a test checks that every present file is
/// listed as well as that every listed file matches.
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
