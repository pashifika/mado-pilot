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
use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const CHILD_PROCESS_POLL: Duration = Duration::from_millis(5);
const CHILD_PROCESS_TERMINATE_WAIT: Duration = Duration::from_secs(1);
const CHILD_PIPE_DRAIN_WAIT: Duration = Duration::from_millis(100);

/// Captured output from one benchmark child whose lifetime and output were
/// bounded by [`bounded_child_output`].
#[derive(Debug)]
pub struct BoundedChildOutput {
    /// The reaped process status, or `None` if the child could not be reaped.
    pub status: Option<ExitStatus>,
    /// The retained stdout prefix, never longer than the requested byte cap.
    pub stdout: Vec<u8>,
    /// The retained stderr prefix, never longer than the requested byte cap.
    pub stderr: Vec<u8>,
    /// Whether the child exited before its deadline and both streams completed
    /// without exceeding the byte cap.
    pub within_bounds: bool,
}

struct CappedPipe {
    bytes: Vec<u8>,
    overflowed: bool,
    complete: bool,
}

fn read_capped_pipe(mut pipe: impl Read, max_bytes: usize) -> CappedPipe {
    let mut bytes = Vec::with_capacity(max_bytes);
    let mut chunk = [0u8; 4_096];
    let mut overflowed = false;
    loop {
        match pipe.read(&mut chunk) {
            Ok(0) => {
                return CappedPipe {
                    bytes,
                    overflowed,
                    complete: true,
                };
            }
            Ok(count) => {
                let remaining = max_bytes.saturating_sub(bytes.len());
                let retained = remaining.min(count);
                bytes.extend_from_slice(&chunk[..retained]);
                overflowed |= retained != count;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => {
                return CappedPipe {
                    bytes,
                    overflowed,
                    complete: false,
                };
            }
        }
    }
}

fn wait_for_child_exit(child: &mut Child, deadline: Instant) -> Option<ExitStatus> {
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => thread::sleep(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(CHILD_PROCESS_POLL),
            ),
            Err(_) => return None,
        }
    }
    child.try_wait().ok().flatten()
}

/// Runs one benchmark child with finite time and per-stream output bounds.
///
/// A child still running at `wait` is terminated and given one bounded second
/// to be reaped. Output beyond `max_output_bytes` is drained so the child cannot
/// deadlock on a full pipe, but it is not retained and makes
/// [`BoundedChildOutput::within_bounds`] false.
pub fn bounded_child_output(
    command: &mut Command,
    wait: Duration,
    max_output_bytes: usize,
) -> BoundedChildOutput {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let Ok(mut child) = command.spawn() else {
        return BoundedChildOutput {
            status: None,
            stdout: Vec::new(),
            stderr: Vec::new(),
            within_bounds: false,
        };
    };
    let stdout = child
        .stdout
        .take()
        .expect("a piped benchmark child must expose stdout");
    let stderr = child
        .stderr
        .take()
        .expect("a piped benchmark child must expose stderr");
    let stdout_reader = thread::spawn(move || read_capped_pipe(stdout, max_output_bytes));
    let stderr_reader = thread::spawn(move || read_capped_pipe(stderr, max_output_bytes));

    let deadline = Instant::now() + wait;
    let mut status = wait_for_child_exit(&mut child, deadline);
    let exited_in_time = status.is_some();
    if !exited_in_time {
        let _killed = child.kill();
        status = wait_for_child_exit(&mut child, Instant::now() + CHILD_PROCESS_TERMINATE_WAIT);
    }

    let drain_deadline = Instant::now() + CHILD_PIPE_DRAIN_WAIT;
    while (!stdout_reader.is_finished() || !stderr_reader.is_finished())
        && Instant::now() < drain_deadline
    {
        thread::sleep(
            drain_deadline
                .saturating_duration_since(Instant::now())
                .min(CHILD_PROCESS_POLL),
        );
    }
    let readers_finished = stdout_reader.is_finished() && stderr_reader.is_finished();
    let stdout = readers_finished
        .then(|| stdout_reader.join().ok())
        .flatten();
    let stderr = readers_finished
        .then(|| stderr_reader.join().ok())
        .flatten();
    let within_bounds = exited_in_time
        && status.is_some()
        && stdout
            .as_ref()
            .is_some_and(|pipe| pipe.complete && !pipe.overflowed)
        && stderr
            .as_ref()
            .is_some_and(|pipe| pipe.complete && !pipe.overflowed);
    BoundedChildOutput {
        status,
        stdout: stdout.map_or_else(Vec::new, |pipe| pipe.bytes),
        stderr: stderr.map_or_else(Vec::new, |pipe| pipe.bytes),
        within_bounds,
    }
}

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
        let Some(last) = sorted.len().checked_sub(1) else {
            return 0.0;
        };
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the clamped nearest rank indexes this small in-memory sample vector"
        )]
        let index = ((percentile.clamp(0.0, 1.0) * sorted.len() as f64)
            .ceil()
            .max(1.0) as usize
            - 1)
        .min(last);
        sorted[index].as_secs_f64() * 1_000.0
    }
    /// Returns the slowest retained sample.
    ///
    /// This is the hard scenario-bound observation. Percentiles can hide one
    /// path that exceeded its absolute deadline, so qualification profiles use
    /// both.
    #[must_use]
    pub fn max_elapsed(&self) -> Duration {
        self.elapsed.iter().copied().max().unwrap_or_default()
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
    /// High-water live Rust heap bytes attributable to this workload.
    #[must_use]
    pub const fn peak_allocated_bytes(&self) -> usize {
        self.peak_bytes
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
        println!(
            "latency_max_ms = {:.6}",
            workload.max_elapsed().as_secs_f64() * 1_000.0
        );
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

/// One frozen latency gate for a named workload.
///
/// These values are fixed before a native qualification run. They are kept out
/// of [`measure`] because most profiles establish host-specific ceilings only
/// after measurement; callers opt in only when a pre-measurement plan already
/// fixed all three bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LatencyBudget {
    workload: &'static str,
    p50: Duration,
    p95: Duration,
    hard_max: Duration,
}

impl LatencyBudget {
    /// Builds one pre-measurement latency gate.
    #[must_use]
    pub const fn new(
        workload: &'static str,
        p50: Duration,
        p95: Duration,
        hard_max: Duration,
    ) -> Self {
        Self {
            workload,
            p50,
            p95,
            hard_max,
        }
    }

    /// Returns the workload name this gate applies to.
    #[must_use]
    pub const fn workload(self) -> &'static str {
        self.workload
    }

    /// Returns the frozen p50 ceiling.
    #[must_use]
    pub const fn p50(self) -> Duration {
        self.p50
    }

    /// Returns the frozen p95 ceiling.
    #[must_use]
    pub const fn p95(self) -> Duration {
        self.p95
    }

    /// Returns the frozen per-scenario maximum.
    #[must_use]
    pub const fn hard_max(self) -> Duration {
        self.hard_max
    }
}

/// Phase 2.2 controlled-capture latency ceilings frozen before qualification.
pub const PHASE2_2_CAPTURE_LATENCY_BUDGETS: [LatencyBudget; 2] = [
    LatencyBudget::new(
        "fixture_command_acknowledgement",
        Duration::from_millis(50),
        Duration::from_millis(100),
        Duration::from_millis(500),
    ),
    LatencyBudget::new(
        "controlled_stimulus_to_frame",
        Duration::from_millis(300),
        Duration::from_millis(750),
        Duration::from_secs(2),
    ),
];

/// Phase 2.2 controlled-transition latency ceilings frozen before qualification.
pub const PHASE2_2_TRANSITION_LATENCY_BUDGETS: [LatencyBudget; 1] = [LatencyBudget::new(
    "close_drain",
    Duration::from_millis(100),
    Duration::from_millis(250),
    Duration::from_secs(1),
)];

const fn phase2_2_process_latency_budgets(event_p95: Duration) -> [LatencyBudget; 5] {
    [
        LatencyBudget::new(
            "discovery_open_retained_authority",
            Duration::from_millis(350),
            Duration::from_millis(750),
            Duration::from_secs(2),
        ),
        LatencyBudget::new(
            "event_authority_preflight_post",
            event_p95,
            event_p95,
            Duration::from_secs(2),
        ),
        LatencyBudget::new(
            "release_cleanup",
            Duration::from_millis(100),
            Duration::from_millis(250),
            Duration::from_millis(250),
        ),
        LatencyBudget::new(
            "session_close",
            Duration::from_millis(100),
            Duration::from_millis(250),
            Duration::from_secs(1),
        ),
        LatencyBudget::new(
            "fixture_controller_close",
            Duration::from_millis(100),
            Duration::from_millis(250),
            Duration::from_secs(1),
        ),
    ]
}

/// Phase 2.2 AppKit process-directed latency ceilings frozen before qualification.
pub const PHASE2_2_PROCESS_APPKIT_LATENCY_BUDGETS: [LatencyBudget; 5] =
    phase2_2_process_latency_budgets(Duration::from_micros(106_340));

/// Phase 2.2 controlled game-like process-directed latency ceilings.
pub const PHASE2_2_PROCESS_GAME_LIKE_LATENCY_BUDGETS: [LatencyBudget; 5] =
    phase2_2_process_latency_budgets(Duration::from_micros(112_180));

/// Phase 2.2 process-diagnostic latency ceilings frozen before qualification.
pub const PHASE2_2_PROCESS_DIAGNOSTIC_LATENCY_BUDGETS: [LatencyBudget; 4] = [
    LatencyBudget::new(
        "event_diagnostics_off",
        Duration::from_millis(300),
        Duration::from_millis(750),
        Duration::from_secs(2),
    ),
    LatencyBudget::new(
        "event_diagnostics_normal",
        Duration::from_millis(300),
        Duration::from_millis(750),
        Duration::from_secs(2),
    ),
    LatencyBudget::new(
        "event_diagnostics_debug",
        Duration::from_millis(300),
        Duration::from_millis(750),
        Duration::from_secs(2),
    ),
    LatencyBudget::new(
        "event_diagnostic_overflow",
        Duration::from_millis(300),
        Duration::from_millis(750),
        Duration::from_secs(2),
    ),
];

/// Frozen Phase 2.2 live-Rust-heap ceiling for every process-directed workload.
pub const PHASE2_2_PROCESS_HEAP_LIMIT_BYTES: usize = 16 * 1024 * 1024;

/// Enforces frozen p50, p95, and per-scenario latency ceilings.
///
/// A missing or duplicated workload is a harness error rather than a skipped
/// gate. The hard maximum is checked independently because a passing percentile
/// must never conceal one operation that escaped its scenario bound.
///
/// # Panics
///
/// Panics when a budget is malformed, names anything other than one measured
/// workload, or when a retained measurement exceeds any ceiling.
pub fn enforce_latency_budgets(workloads: &[Workload], budgets: &[LatencyBudget]) {
    for (index, budget) in budgets.iter().enumerate() {
        assert!(
            budget.p50 <= budget.p95 && budget.p95 <= budget.hard_max,
            "latency budget for {} must satisfy p50 <= p95 <= hard maximum",
            budget.workload
        );
        assert!(
            budgets[..index]
                .iter()
                .all(|earlier| earlier.workload != budget.workload),
            "latency budget for {} is duplicated",
            budget.workload
        );
        let mut matching = workloads
            .iter()
            .filter(|workload| workload.name() == budget.workload);
        let workload = matching.next().unwrap_or_else(|| {
            panic!(
                "latency budget names unmeasured workload {}",
                budget.workload
            )
        });
        assert!(
            matching.next().is_none(),
            "measured workload {} is duplicated",
            budget.workload
        );

        let p50 = workload.percentile(0.50);
        let p95 = workload.percentile(0.95);
        let hard_max = workload.max_elapsed();
        assert!(
            p50 <= budget.p50.as_secs_f64() * 1_000.0,
            "{} exceeded frozen p50 latency ceiling: {p50:.6} ms > {:.6} ms",
            budget.workload,
            budget.p50.as_secs_f64() * 1_000.0
        );
        assert!(
            p95 <= budget.p95.as_secs_f64() * 1_000.0,
            "{} exceeded frozen p95 latency ceiling: {p95:.6} ms > {:.6} ms",
            budget.workload,
            budget.p95.as_secs_f64() * 1_000.0
        );
        assert!(
            hard_max <= budget.hard_max,
            "{} exceeded frozen hard scenario bound: {:.6} ms > {:.6} ms",
            budget.workload,
            hard_max.as_secs_f64() * 1_000.0,
            budget.hard_max.as_secs_f64() * 1_000.0
        );
    }
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

/// Classification of one line emitted by a native benchmark fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefixedLineMatch {
    /// The line is unrelated to the observation family and may be skipped.
    Irrelevant,
    /// The line is the exact observation the current sample expects.
    Expected,
    /// The line belongs to the observation family but names a different outcome.
    Unexpected,
}

/// Classifies an exact fixture observation without discarding sibling outcomes.
///
/// Native benchmark readers may ignore readiness and control records, but once a
/// line belongs to the supplied observation family, any non-exact value is an
/// oracle failure rather than noise.
#[must_use]
pub fn classify_prefixed_line(
    line: &str,
    observation_prefix: &str,
    expected: &str,
) -> PrefixedLineMatch {
    if !line.starts_with(observation_prefix) {
        PrefixedLineMatch::Irrelevant
    } else if line == expected {
        PrefixedLineMatch::Expected
    } else {
        PrefixedLineMatch::Unexpected
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
    use super::{
        LatencyBudget, Plan, PrefixedLineMatch, Sample, Workload, bounded_child_output,
        classify_prefixed_line, enforce_latency_budgets, measure,
    };
    use std::io::Write;
    use std::process::Command;
    use std::time::{Duration, Instant};

    fn fixture() {}

    const CHILD_MODE: &str = "MADO_PILOT_TESTKIT_BOUNDED_CHILD_MODE";

    fn child_command(mode: &str) -> Command {
        let mut command =
            Command::new(std::env::current_exe().expect("the current test executable exists"));
        command
            .args(["bounded_child_fixture", "--nocapture"])
            .env(CHILD_MODE, mode);
        command
    }

    #[test]
    fn bounded_child_fixture() {
        match std::env::var(CHILD_MODE).as_deref() {
            Ok("success") => println!("bounded child completed"),
            Ok("overflow") => {
                let mut stdout = std::io::stdout().lock();
                stdout
                    .write_all(&vec![b'x'; 4_096])
                    .expect("the child writes its oversized output");
                stdout.flush().expect("the child flushes stdout");
            }
            Ok("timeout") => std::thread::sleep(Duration::from_secs(5)),
            _ => {}
        }
    }

    #[test]
    fn bounded_child_output_accepts_a_timely_finite_process() {
        let output = bounded_child_output(
            &mut child_command("success"),
            Duration::from_secs(1),
            16 * 1_024,
        );

        assert!(output.within_bounds);
        assert!(output.status.is_some_and(|status| status.success()));
        assert!(
            String::from_utf8(output.stdout)
                .expect("the child writes UTF-8")
                .contains("bounded child completed")
        );
    }

    #[test]
    fn bounded_child_output_reaps_a_process_after_timeout() {
        let started = Instant::now();
        let output = bounded_child_output(
            &mut child_command("timeout"),
            Duration::from_millis(25),
            16 * 1_024,
        );

        assert!(!output.within_bounds);
        assert!(
            output.status.is_some(),
            "the terminated child was not reaped"
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the timed-out child exceeded the bounded termination allowance"
        );
    }

    #[test]
    fn bounded_child_output_rejects_and_caps_oversized_stdout() {
        let output = bounded_child_output(
            &mut child_command("overflow"),
            Duration::from_secs(1),
            1_024,
        );

        assert!(!output.within_bounds);
        assert!(output.status.is_some_and(|status| status.success()));
        assert_eq!(output.stdout.len(), 1_024);
    }

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

    fn timed_workload(name: &'static str, elapsed: Vec<Duration>) -> Workload {
        Workload {
            name,
            oracle: "the synthetic timing sample is accepted",
            elapsed,
            incorrect: 0,
            stale: 0,
            scheduled: 0,
            mapped: 0,
            iteration_span: Duration::ZERO,
            peak_bytes: 0,
            steady_bytes: 0,
            peak_resident_bytes: None,
            growth_bytes: 0,
        }
    }

    #[test]
    fn frozen_latency_budgets_check_percentiles_and_the_slowest_sample() {
        let workloads = [timed_workload(
            "qualified",
            vec![
                Duration::from_millis(10),
                Duration::from_millis(20),
                Duration::from_millis(30),
            ],
        )];

        enforce_latency_budgets(
            &workloads,
            &[LatencyBudget::new(
                "qualified",
                Duration::from_millis(20),
                Duration::from_millis(30),
                Duration::from_millis(40),
            )],
        );
    }

    #[test]
    #[should_panic(expected = "exceeded frozen hard scenario bound")]
    fn one_slow_sample_cannot_hide_behind_a_passing_percentile() {
        let mut elapsed = vec![Duration::from_millis(1); 100];
        elapsed.push(Duration::from_millis(501));
        let workloads = [timed_workload("qualified", elapsed)];

        enforce_latency_budgets(
            &workloads,
            &[LatencyBudget::new(
                "qualified",
                Duration::from_millis(10),
                Duration::from_millis(10),
                Duration::from_millis(500),
            )],
        );
    }

    #[test]
    #[should_panic(expected = "exceeded frozen p95 latency ceiling")]
    fn a_percentile_ceiling_is_not_treated_as_only_a_hard_maximum() {
        let workloads = [timed_workload(
            "qualified",
            vec![
                Duration::from_millis(1),
                Duration::from_millis(2),
                Duration::from_millis(30),
            ],
        )];

        enforce_latency_budgets(
            &workloads,
            &[LatencyBudget::new(
                "qualified",
                Duration::from_millis(2),
                Duration::from_millis(20),
                Duration::from_millis(40),
            )],
        );
    }

    #[test]
    fn a_wrong_role_observation_is_not_skipped_before_the_expected_target() {
        assert_eq!(
            classify_prefixed_line(
                "control queue-block=ready",
                "observation role=",
                "observation role=target family=pointer-move units=1",
            ),
            PrefixedLineMatch::Irrelevant
        );
        assert_eq!(
            classify_prefixed_line(
                "observation role=sibling family=pointer-move units=1",
                "observation role=",
                "observation role=target family=pointer-move units=1",
            ),
            PrefixedLineMatch::Unexpected
        );
        assert_eq!(
            classify_prefixed_line(
                "observation role=target family=pointer-move units=1",
                "observation role=",
                "observation role=target family=pointer-move units=1",
            ),
            PrefixedLineMatch::Expected
        );
    }
    #[test]
    fn percentile_uses_nearest_rank_for_even_sample_counts() {
        let workload = Workload {
            name: "nearest-rank",
            oracle: "the selected order statistic is exact",
            elapsed: (1..=50).map(Duration::from_millis).collect(),
            incorrect: 0,
            stale: 0,
            scheduled: 0,
            mapped: 0,
            iteration_span: Duration::ZERO,
            peak_bytes: 0,
            steady_bytes: 0,
            peak_resident_bytes: None,
            growth_bytes: 0,
        };

        assert_eq!(workload.percentile(0.50), 25.0);
        assert_eq!(workload.percentile(0.95), 48.0);
        assert_eq!(workload.percentile(1.0), 50.0);
    }
}
