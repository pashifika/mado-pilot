//! The measurement scaffolding every in-process benchmark target shares.
//!
//! The deterministic Rust workflow, C boundary, and diagnostic-overhead
//! benchmarks all emit the profile format `docs/performance.md` defines. They
//! measure different things and must emit the *same* shape, because a committed
//! profile is read by whoever compares two runs and a second copy of the printer
//! is a second thing to keep in step with the format document.
//!
//! What is here is everything that is not a workload: the sampling loop, the
//! allocation accounting, the host arguments, the report, and the hard budgets.
//! What each benchmark keeps for itself is its fixtures, its workloads, and its
//! oracles.
//!
//! # Which budgets are enforced here
//!
//! A `hard` budget is a structural property that holds on any host, so the
//! harness enforces it: [`enforce_hard_budgets`] is called by every in-process
//! benchmark target on both of the paths they run. An `absolute` or `relative`
//! budget is a per-target regression ceiling measured on named hardware, so
//! only a run on that hardware can evaluate it; those stay with the operator and
//! committed profile for the matching release target.
//!
//! # Why the counters live in a library
//!
//! A `#[global_allocator]` is per binary, so each benchmark declares its own
//! static. [`Accounting`] is the implementation they all point at, and the
//! counters are its statics, which is what lets [`measure`] read them without
//! any benchmark passing them in.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// The target triple the benchmark runs on, when it is one this project
/// releases.
///
/// Selected rather than detected. `std::env::consts` can report the
/// architecture and the operating system but not the vendor or the ABI, and a
/// triple assembled from the parts that are available would be a guess printed
/// where a measurement condition belongs. A budget is valid only for the target
/// in its profile, so the wrong string here is worse than no string.
pub const RELEASE_TARGET: &str = if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
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
/// a different platform API on each release target, it moves with allocator and
/// operating-system behaviour that no MadoPilot change can affect, and on a
/// workload this small the noise is larger than the signal. Live heap bytes are
/// portable, are the same computation on both targets, and answer the question
/// a bounded-memory gate actually asks: does a repeated operation give back
/// what it took. The three measures this feeds are named separately in the
/// measure vocabulary so that neither reading is mistaken for the other.
///
/// A benchmark installs it with:
///
/// ```ignore
/// #[global_allocator]
/// static ALLOCATOR: mado_pilot_testkit::bench_harness::Accounting =
///     mado_pilot_testkit::bench_harness::Accounting;
/// ```
#[derive(Debug)]
pub struct Accounting;

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

/// Live heap bytes now.
fn live() -> usize {
    LIVE.load(Ordering::Relaxed)
}

// --- Running -----------------------------------------------------------------

/// How many iterations a run discards and how many it keeps.
#[derive(Debug, Clone, Copy)]
pub struct Plan {
    warmup: usize,
    samples: usize,
}

impl Plan {
    /// Builds an explicit plan for a workload set whose contract uses a
    /// different sample schedule from the Phase 1 default.
    ///
    /// # Panics
    ///
    /// Panics when `samples` is zero, because a run with no retained sample
    /// cannot produce a percentile or exercise a correctness oracle.
    #[must_use]
    pub fn new(warmup: usize, samples: usize) -> Self {
        assert!(samples > 0, "a benchmark plan retains at least one sample");
        Self { warmup, samples }
    }

    /// Enough samples for the oracles, not enough for a percentile.
    #[must_use]
    pub const fn smoke() -> Self {
        Self {
            warmup: 1,
            samples: 3,
        }
    }

    /// A full timing run.
    #[must_use]
    pub const fn full() -> Self {
        Self {
            warmup: 20,
            samples: 200,
        }
    }

    /// Returns the plan a run's arguments ask for.
    ///
    /// `cargo bench` passes `--bench`; `cargo test --all-targets` does not, and
    /// wants the oracles rather than the timings.
    #[must_use]
    pub fn from(arguments: &[String]) -> Self {
        if arguments.iter().any(|argument| argument == "--bench") {
            Self::full()
        } else {
            Self::smoke()
        }
    }

    /// How many samples one workload retains.
    #[must_use]
    pub const fn samples(self) -> usize {
        self.samples
    }

    /// How many iterations a run discards before retaining samples.
    #[must_use]
    pub const fn warmup(self) -> usize {
        self.warmup
    }
}

/// What one iteration of a workload reports.
#[derive(Debug)]
pub struct Sample {
    elapsed: Duration,
    correct: bool,
    mapped: u64,
    peak_resident: Option<u64>,
    stale: Option<(u64, u64)>,
}

impl Sample {
    /// A sample from a workload that maps frame bytes.
    #[must_use]
    pub const fn new(elapsed: Duration, correct: bool, mapped: u64) -> Self {
        Self {
            elapsed,
            correct,
            mapped,
            peak_resident: None,
            stale: None,
        }
    }

    /// A sample from a workload that maps nothing.
    #[must_use]
    pub const fn unmapped(elapsed: Duration, correct: bool) -> Self {
        Self::new(elapsed, correct, 0)
    }

    /// Associates an observable stale/drop count with this sample.
    ///
    /// `total` is the number of producer publications represented by the
    /// sample, including the one returned to the consumer. The ratio is
    /// therefore `stale / total`, never a count detached from its denominator.
    #[must_use]
    pub const fn with_stale_work(mut self, stale: u64, total: u64) -> Self {
        self.stale = Some((stale, total));
        self
    }

    /// Associates the measured child process's peak resident set with this sample.
    ///
    /// This is separate from the Rust global-allocator counters because a
    /// separately linked C or C++ process has its own allocator and address
    /// space. The child reports this value from its native process API after it
    /// has released the flow's owned handles.
    #[must_use]
    pub const fn with_peak_resident_bytes(mut self, bytes: u64) -> Self {
        self.peak_resident = Some(bytes);
        self
    }
}

/// One workload's samples, and what they cost besides time.
#[derive(Debug)]
pub struct Workload {
    name: &'static str,
    oracle: &'static str,
    elapsed: Vec<Duration>,
    incorrect: usize,
    stale: u64,
    scheduled: u64,
    mapped: u64,
    iteration_span: Duration,
    peak_bytes: usize,
    steady_bytes: usize,
    peak_resident_bytes: Option<u64>,
    growth_bytes: i64,
}

impl Workload {
    /// How many retained samples failed their oracle.
    #[must_use]
    pub const fn incorrect(&self) -> usize {
        self.incorrect
    }

    /// Returns the `percentile`-th sample, in milliseconds.
    #[must_use]
    pub fn percentile(&self, percentile: f64) -> f64 {
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
    /// the host clock can express — on `x86_64-pc-windows-msvc` a
    /// matching-format frame mapping measures exactly zero, because it is a
    /// reference-count increment. One clock read across hundreds of iterations
    /// recovers a number that granularity cannot swallow.
    ///
    /// It measures more than the operation does. Everything an iteration needs
    /// is inside the span: preparing its inputs, checking the oracle, dropping
    /// what it produced. So it is an upper bound on the operation rather than a
    /// reading of it, and that is what makes it usable as a ceiling for a
    /// workload whose own fast path is too quick to time.
    #[must_use]
    pub fn iteration_span_ms(&self) -> f64 {
        self.iteration_span.as_secs_f64() * 1_000.0
    }

    /// Live heap bytes this workload's samples did not give back.
    ///
    /// Signed, because a workload that ends below its post-warmup baseline has
    /// released more than it took and satisfies the requirement just as a
    /// workload that ended level does.
    #[must_use]
    pub const fn growth_bytes(&self) -> i64 {
        self.growth_bytes
    }

    /// Share of observed producer work skipped before a retained result.
    #[must_use]
    pub fn stale_work_ratio(&self) -> Option<f64> {
        (self.scheduled > 0).then(|| self.stale as f64 / self.scheduled as f64)
    }

    /// The workload's name, as the report files it under.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }
}

/// Runs `workload` through its warmup and its samples.
///
/// The three memory numbers are differences against two baselines rather than
/// absolute totals, because an absolute total would include every earlier
/// workload's retained samples and would grow down the report for a reason that
/// has nothing to do with the workload being measured. The fixture baseline is
/// what this workload's own footprint is measured against; the post-warmup one
/// is what its growth is measured against, so a one-time cost the first
/// iterations paid is not reported as a leak.
pub fn measure<F, M>(
    name: &'static str,
    oracle: &'static str,
    plan: Plan,
    make: M,
    workload: fn(&F) -> Sample,
) -> Workload
where
    M: FnOnce() -> F,
{
    // Allocated before the baseline is taken and never grown afterwards, so the
    // harness's own record of the run does not appear as the workload's memory.
    let mut elapsed = Vec::with_capacity(plan.samples);

    let before_fixture = live();
    let fixture = make();
    for _ in 0..plan.warmup {
        workload(&fixture);
    }

    let after_warmup = live();
    PEAK.store(after_warmup, Ordering::Relaxed);

    let mut incorrect = 0;
    let mut stale = 0u64;
    let mut scheduled = 0u64;
    let mut mapped = 0;
    let mut peak_resident_bytes: Option<u64> = None;
    let span = Instant::now();
    for _ in 0..plan.samples {
        let sample = workload(&fixture);
        if !sample.correct {
            incorrect += 1;
        }
        // The largest of the retained samples, not the last one: a change that
        // maps twice on every sampled iteration except the final one would
        // otherwise report the low number and satisfy its budget.
        if let Some((sample_stale, sample_total)) = sample.stale {
            stale = stale.saturating_add(sample_stale);
            scheduled = scheduled.saturating_add(sample_total);
        }
        mapped = mapped.max(sample.mapped);
        if let Some(sample_peak) = sample.peak_resident {
            peak_resident_bytes = Some(peak_resident_bytes.unwrap_or_default().max(sample_peak));
        }
        elapsed.push(sample.elapsed);
    }
    let span = span.elapsed();

    let ending = live();
    Workload {
        name,
        oracle,
        elapsed,
        incorrect,
        mapped,
        iteration_span: span / u32::try_from(plan.samples).unwrap_or(u32::MAX),
        peak_bytes: PEAK.load(Ordering::Relaxed).saturating_sub(before_fixture),
        steady_bytes: ending.saturating_sub(before_fixture),
        peak_resident_bytes,
        stale,
        scheduled,
        growth_bytes: i64::try_from(ending).unwrap_or(i64::MAX)
            - i64::try_from(after_warmup).unwrap_or(i64::MAX),
    }
}

// --- Reporting ---------------------------------------------------------------

/// What the run is, for the report's `[benchmark]` table.
#[derive(Debug)]
pub struct Benchmark {
    /// The identifier a committed profile is filed under.
    pub id: &'static str,
    /// One sentence naming what the set of workloads covers.
    pub workload: &'static str,
    /// The phase that introduced them.
    pub phase: &'static str,
}

/// The conditions that make a measurement reproducible.
///
/// `hardware` and `os_version` are the operator's to state and are read from
/// the command line: a CPU model the program detected would be a guess recorded
/// as a measurement condition.
#[derive(Debug)]
pub struct Profile {
    /// Tracked paths of everything the workloads read.
    pub fixture: String,
    /// One digest pinning all of it.
    pub fixture_sha256: String,
    /// The machine, as the operator stated it.
    pub hardware: String,
    /// Its operating-system version, as the operator stated it.
    pub os_version: String,
    /// The command profile and feature selection that produced the executable.
    pub build_profile: String,
    /// How every retained sample was checked.
    pub correctness_oracle: &'static str,
    /// The queue depth and drop policy in effect.
    pub queue_policy: &'static str,
    /// Optional target-specific conditions not represented by another field.
    pub notes: Option<String>,
}

impl Profile {
    /// Reads `--hardware` and `--os-version`, falling back to `--label`.
    #[must_use]
    pub fn host(arguments: &[String]) -> (String, String) {
        // `--label` predates the two specific arguments and named the host as
        // one string. It still fills the hardware field so an older recorded
        // command keeps working.
        let label = argument(arguments, "--label");
        (
            argument(arguments, "--hardware")
                .or(label)
                .unwrap_or_else(|| "unstated".to_owned()),
            argument(arguments, "--os-version").unwrap_or_else(|| "unstated".to_owned()),
        )
    }
}

/// The `[benchmark]` block a report opens with, as `key = value` lines.
///
/// Returned as a list rather than printed one line at a time so that the key
/// set is a value something can compare. A committed profile is this block with
/// `status` and `normative` answered differently and the budgets added, so a key
/// here that no profile carries — or one every profile carries and this omits —
/// is the two records drifting apart. `benchmark_block_drift.rs` is that
/// comparison.
#[must_use]
pub fn benchmark_block(benchmark: &Benchmark) -> Vec<(&'static str, String)> {
    vec![
        ("id", format!("\"{}\"", escape(benchmark.id))),
        ("workload", format!("\"{}\"", escape(benchmark.workload))),
        ("phase", format!("\"{}\"", escape(benchmark.phase))),
        // What this run is: harness output, which nothing gates on, carrying
        // measurements that are real readings rather than illustrations.
        ("status", "\"harness-output\"".to_owned()),
        ("normative", "false".to_owned()),
        ("measurements_recorded", "true".to_owned()),
    ]
}

/// Prints a profile-shaped report with no budget in it.
///
/// A committed file under `docs/benchmarks/` is this output with budgets added.
pub fn report(benchmark: &Benchmark, profile: &Profile, plan: Plan, workloads: &[Workload]) {
    println!("format_version = 1");
    println!();
    println!("[benchmark]");
    for (key, value) in benchmark_block(benchmark) {
        println!("{key} = {value}");
    }
    println!("# A committed profile under docs/benchmarks/ carries the budgets.");
    println!();
    println!("[profile]");
    println!("fixture = \"{}\"", escape(&profile.fixture));
    println!("fixture_sha256 = \"{}\"", escape(&profile.fixture_sha256));
    println!("release_target = \"{RELEASE_TARGET}\"");
    println!("hardware = \"{}\"", escape(&profile.hardware));
    println!("os_version = \"{}\"", escape(&profile.os_version));
    println!("build_profile = \"{}\"", escape(&profile.build_profile));
    println!("warmup_iterations = {}", plan.warmup);
    println!("sample_count = {}", plan.samples);
    println!("correctness_oracle = \"{}\"", profile.correctness_oracle);
    println!("queue_policy = \"{}\"", profile.queue_policy);
    if let Some(notes) = &profile.notes {
        println!("notes = \"{}\"", escape(notes));
    }
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
        if let Some(ratio) = workload.stale_work_ratio() {
            println!("stale_work_ratio = {ratio:.9}");
        }
        println!("peak_allocated_bytes = {}", workload.peak_bytes);
        println!("steady_allocated_bytes = {}", workload.steady_bytes);
        println!("allocated_growth_bytes = {}", workload.growth_bytes);
        if let Some(bytes) = workload.peak_resident_bytes {
            println!("peak_resident_bytes = {bytes}");
        }
        println!();
    }
}

/// Prints the short line a `cargo test` run reports instead of a profile.
pub fn summarize(name: &str, plan: Plan, workloads: &[Workload]) {
    let failures: usize = workloads.iter().map(Workload::incorrect).sum();
    println!(
        "{name}: {} workloads, {} samples each, {failures} oracle failure(s)",
        workloads.len(),
        plan.samples
    );
}

// --- Hard budgets --------------------------------------------------------------

/// The `kind = "hard"` predicates every committed profile states, enforced by
/// [`enforce_hard_budgets`].
///
/// Copied rather than parsed. Reading them out of the profiles would need a
/// TOML reader and an evaluator for the predicate expression, which is a
/// dependency and a small language for a set of two strings. What keeps the
/// copies honest is `tests/hard_budget_drift.rs`, which reads all four
/// committed profiles and fails when the hard predicates they state are not
/// exactly these.
pub const HARD_BUDGET_PREDICATES: [&str; 2] =
    ["result_correctness == 0", "allocated_growth_bytes <= 4096"];

/// The bound in the second predicate above, as the number it is compared with.
pub const GROWTH_LIMIT_BYTES: i64 = 4096;

/// Fails the run when a workload violates a hard budget.
///
/// Every in-process benchmark target calls this unconditionally, so the two
/// predicates are enforced on the `cargo bench` path and on the
/// `cargo test --all-targets` path that CI runs on both release targets. Call it
/// after the report, so a run that fails still emits the numbers that explain it.
///
/// Sensitivity differs between the two paths even though the gate does not. A
/// smoke run retains three samples ([`Plan::smoke`]), so a per-iteration leak
/// has three iterations to exceed one page rather than the two hundred a
/// `--bench` run gives it. A leak is still a leak on both paths; what belongs
/// to the `--bench` run alone is the claim that a leak of a few dozen bytes per
/// iteration is caught.
///
/// # Panics
///
/// Panics naming the workload, the predicate it violated, and the measurement
/// that violated it.
pub fn enforce_hard_budgets(workloads: &[Workload]) {
    let [_correctness, growth] = HARD_BUDGET_PREDICATES;
    enforce_correctness(workloads);

    for workload in workloads {
        assert!(
            workload.growth_bytes <= GROWTH_LIMIT_BYTES,
            "{}: {growth} — live heap grew {} bytes over {} retained samples, \
             so a repeated operation did not give back what it took",
            workload.name,
            workload.growth_bytes,
            workload.elapsed.len(),
        );
    }
}

/// Fails when any retained sample violates its workload oracle.
///
/// Native evidence uses this before its measured bounded-growth predicate has
/// been set from both release targets. Phase 1 calls [`enforce_hard_budgets`],
/// which applies this same rule and its established growth bound together.
///
/// # Panics
///
/// Panics naming the workload and oracle when a retained sample is incorrect.
pub fn enforce_correctness(workloads: &[Workload]) {
    let correctness = HARD_BUDGET_PREDICATES[0];
    for workload in workloads {
        assert!(
            workload.incorrect == 0,
            "{}: {correctness} — {} of {} retained samples produced an output \
             its oracle rejected ({})",
            workload.name,
            workload.incorrect,
            workload.elapsed.len(),
            workload.oracle,
        );
    }
}

/// Returns the value of a `--name value` or `--name=value` argument.
#[must_use]
pub fn argument(arguments: &[String], name: &str) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::{Plan, Sample, measure};
    use std::time::Duration;

    fn fixture() {}

    fn stale_sample(_: &()) -> Sample {
        Sample::unmapped(Duration::from_micros(1), true).with_stale_work(1, 4)
    }

    #[test]
    fn an_observable_stale_ratio_keeps_its_denominator() {
        let workload = measure(
            "stale",
            "one of four publications is skipped",
            Plan::new(0, 2),
            fixture,
            stale_sample,
        );

        assert_eq!(workload.stale_work_ratio(), Some(0.25));
    }
}
