//! Paired public Rust-facade / negotiated C-table template-query observations.
//!
//! Replay/OpenCV setup and completion are outside every caller allocation and
//! latency window. This is not native capture, backend latency, dynamic loading,
//! foreign malloc, RSS, or a qualification against historical watcher ceilings.
//! The process-wide `Accounting` heap readings include fixture/setup and worker
//! activity; the additional allocation-call counters cover only this caller.
//!
//! A one-frame absent replay ends its idle query when the source is exhausted.
//! Pending fixtures therefore use two frames, `AnalysisAlways`, and a second
//! analysis held rate-ineligible beyond the finite query lifetime. Setup observes
//! the first no-match completion and the deferred second frame. Every subsequent
//! observation must preserve that entire pending snapshot, not invent progress.
//!
//! The first-projection pair uses public session close to commit `SessionClosed`
//! before the first terminal poll, without first materializing a C result. Matched
//! projections are exercised separately after a bounded setup wait. No sleep is
//! used to guess that matching or terminal publication has completed.
//!
//! ```text
//! cargo test --locked --package mado-pilot-capi --bench template-watch-boundary
//! cargo bench --locked --package mado-pilot-capi --bench template-watch-boundary -- \
//!     --hardware "..." --os-version "..."
//! ```
//! Use a dedicated `CARGO_TARGET_DIR` if another campaign has pinned this
//! package's binaries. No latency/heap ceiling is accepted by this harness.

use std::alloc::{GlobalAlloc, Layout};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use mado_pilot::ContentDigest;
use mado_pilot_testkit::bench_harness::{Accounting, Benchmark, Plan, Profile, Sample, Workload};
use mado_pilot_testkit::{bench_harness, match_fixtures};

#[path = "template_watch_boundary/flows.rs"]
mod flows;

use flows::{CFlow, RustFlow};

const BATCH: usize = 32;
const SETUP_WAIT: Duration = Duration::from_secs(5);
const QUERY_LIFETIME: Duration = Duration::from_secs(60);
const PENDING_INTERVAL: Duration = Duration::from_secs(3_600);

#[derive(Clone, Copy, Default)]
struct AllocationCalls {
    alloc: u64,
    alloc_zeroed: u64,
    realloc: u64,
    dealloc: u64,
}

impl AllocationCalls {
    fn allocations(self) -> u64 {
        self.alloc + self.alloc_zeroed + self.realloc
    }
}

thread_local! {
    // Const initialization and Copy-only cells avoid TLS initialization or
    // destructor allocations inside GlobalAlloc. None means outside a window.
    static CALLER_ALLOCATIONS: Cell<Option<AllocationCalls>> = const { Cell::new(None) };
}

struct CallerAccounting;

fn count_call(update: impl FnOnce(&mut AllocationCalls)) {
    let _ = CALLER_ALLOCATIONS.try_with(|cell| {
        if let Some(mut counts) = cell.get() {
            update(&mut counts);
            cell.set(Some(counts));
        }
    });
}

// SAFETY: pointers, layouts, return values, and allocation ownership are passed
// unchanged to Accounting. The const TLS counters never allocate or affect them.
unsafe impl GlobalAlloc for CallerAccounting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_call(|counts| counts.alloc += 1);
        // SAFETY: forwards the caller's GlobalAlloc contract unchanged.
        unsafe { Accounting.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count_call(|counts| counts.alloc_zeroed += 1);
        // SAFETY: forwards the caller's GlobalAlloc contract unchanged.
        unsafe { Accounting.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count_call(|counts| counts.realloc += 1);
        // SAFETY: forwards the caller's GlobalAlloc contract unchanged.
        unsafe { Accounting.realloc(pointer, layout, size) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        count_call(|counts| counts.dealloc += 1);
        // SAFETY: forwards the caller's GlobalAlloc contract unchanged.
        unsafe { Accounting.dealloc(pointer, layout) };
    }
}

#[global_allocator]
static ALLOCATOR: CallerAccounting = CallerAccounting;

struct CallerWindow;

impl Drop for CallerWindow {
    fn drop(&mut self) {
        // Also disable counting on oracle panic, before panic-output bookkeeping.
        CALLER_ALLOCATIONS.with(|cell| cell.set(None));
    }
}

struct Observation {
    elapsed: Duration,
    calls: AllocationCalls,
    live_before: usize,
    live_after: usize,
    readable_view: Option<usize>,
}

fn timed<T>(work: impl FnOnce() -> T) -> (T, Observation) {
    let live_before = bench_harness::live_allocated_bytes();
    CALLER_ALLOCATIONS.with(|cell| {
        assert!(cell.get().is_none(), "caller windows must not nest");
        cell.set(Some(AllocationCalls::default()));
    });
    let window = CallerWindow;
    let started = Instant::now();
    let value = std::hint::black_box(work());
    let elapsed = started.elapsed();
    let calls = CALLER_ALLOCATIONS.with(|cell| cell.get().expect("active caller window"));
    drop(window);
    let live_after = bench_harness::live_allocated_bytes();
    (
        value,
        Observation {
            elapsed,
            calls,
            live_before,
            live_after,
            readable_view: None,
        },
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Case {
    PendingPoll,
    FirstTerminal,
    TerminalPoll,
    TerminalRead,
    WaitCancelled,
    QueryClone,
    ResultClone,
    RetainedFrame,
}

impl Case {
    const ALL: [Self; 8] = [
        Self::PendingPoll,
        Self::FirstTerminal,
        Self::TerminalPoll,
        Self::TerminalRead,
        Self::WaitCancelled,
        Self::QueryClone,
        Self::ResultClone,
        Self::RetainedFrame,
    ];

    fn pending(self) -> bool {
        matches!(
            self,
            Self::PendingPoll | Self::FirstTerminal | Self::WaitCancelled | Self::QueryClone
        )
    }

    fn names(self) -> [&'static str; 2] {
        match self {
            Self::PendingPoll => ["pending_poll_rust", "pending_poll_c"],
            Self::FirstTerminal => [
                "first_closed_terminal_poll_rust",
                "first_closed_terminal_poll_c",
            ],
            Self::TerminalPoll => ["retained_terminal_poll_rust", "retained_terminal_poll_c"],
            Self::TerminalRead => [
                "terminal_info_match_read_rust",
                "terminal_info_match_read_c",
            ],
            Self::WaitCancelled => ["caller_wait_cancelled_rust", "caller_wait_cancelled_c"],
            Self::QueryClone => ["query_clone_release_rust", "query_clone_release_c"],
            Self::ResultClone => ["result_clone_release_rust", "result_clone_release_c"],
            Self::RetainedFrame => [
                "exact_frame_after_parents_rust",
                "exact_frame_after_parents_c",
            ],
        }
    }

    fn oracle(self) -> &'static str {
        match self {
            Self::PendingPoll => {
                "the settled no-match query remains pending with identical identity, stability, depths and all nine work counts"
            }
            Self::FirstTerminal => {
                "the first terminal poll after successful public session close returns SessionClosed, not a match or caller failure"
            }
            Self::TerminalPoll => {
                "each terminal poll preserves the retained matched winner and both planted matches without accumulating projections"
            }
            Self::TerminalRead => {
                "every info and both match reads preserve exact source, target, geometry, template, effective options and planted bounds/scores"
            }
            Self::WaitCancelled => {
                "a separately pre-cancelled bounded caller wait reports Cancelled and leaves the complete pending query observation unchanged"
            }
            Self::QueryClone => {
                "dropping a non-final shared query reference leaves the original query and its complete pending observation unchanged"
            }
            Self::ResultClone => {
                "a shared result clone is readable, its release leaves the retained matched owner unchanged, and neither copies match storage"
            }
            Self::RetainedFrame => {
                "after query, session, engine, template and package teardown a retained result yields its exact RGBA frame, shared mapping and all original bytes"
            }
        }
    }

    fn repetitions(self) -> usize {
        if self == Self::FirstTerminal {
            1
        } else {
            BATCH
        }
    }

    fn requires_zero_allocations(self) -> bool {
        matches!(
            self,
            Self::PendingPoll | Self::TerminalPoll | Self::TerminalRead
        )
    }
}

struct Metrics {
    warmup: usize,
    iterations: Cell<usize>,
    observations: RefCell<Vec<Observation>>,
    fixture_baseline: Cell<usize>,
    ending_live: Cell<usize>,
}

impl Metrics {
    fn new(plan: Plan) -> Self {
        Self {
            warmup: plan.warmup(),
            iterations: Cell::new(0),
            observations: RefCell::new(Vec::with_capacity(plan.samples())),
            fixture_baseline: Cell::new(0),
            ending_live: Cell::new(0),
        }
    }

    fn record(&self, observation: Observation, correct: bool) -> Sample {
        let elapsed = observation.elapsed;
        let iteration = self.iterations.get();
        self.iterations.set(iteration + 1);
        // All sample storage was reserved before the fixture/heap baseline. This
        // update is outside both the caller window and its allocation counters.
        if iteration >= self.warmup {
            self.observations.borrow_mut().push(observation);
        } else {
            assert!(correct, "warmup failed its consumer oracle");
        }
        self.ending_live.set(bench_harness::live_allocated_bytes());
        // Sample has no unknown-mapping representation. Its mapped slot is not
        // evidence here and is deliberately never emitted by our reporter.
        Sample::unmapped(elapsed, correct)
    }
}

#[expect(
    clippy::large_enum_variant,
    reason = "fixture state stays stack-owned without adding measured setup allocations"
)]
enum Ready {
    Rust(RustFlow),
    C(CFlow),
    FirstRust,
    FirstC,
}

struct Fixture<'a> {
    case: Case,
    ready: Ready,
    metrics: &'a Metrics,
}

fn exercise(fixture: &Fixture<'_>) -> Sample {
    let (result, mut observation) = match &fixture.ready {
        Ready::Rust(flow) => timed(|| flow.exercise(fixture.case)),
        Ready::C(flow) => timed(|| flow.exercise(fixture.case)),
        Ready::FirstRust => {
            let mut flow = RustFlow::new(Case::FirstTerminal);
            flow.close_parents();
            let (outcome, observation) = timed(|| flow.first_poll());
            let correct = flow.closed_outcome(&outcome);
            drop(outcome);
            drop(flow);
            (
                flows::Exercise {
                    correct,
                    readable_view: None,
                },
                observation,
            )
        }
        Ready::FirstC => {
            let mut flow = CFlow::new(Case::FirstTerminal);
            flow.close_parents();
            let (outcome, observation) = timed(|| flow.first_poll());
            let correct = flow.closed_outcome(&outcome);
            drop(outcome);
            drop(flow);
            (
                flows::Exercise {
                    correct,
                    readable_view: None,
                },
                observation,
            )
        }
    };
    observation.readable_view = result.readable_view;
    fixture.metrics.record(observation, result.correct)
}

struct Measurement {
    case: Case,
    workload: Workload,
    metrics: Metrics,
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let plan = Plan::from(&arguments);
    let full = arguments.iter().any(|argument| argument == "--bench");
    let (hardware, os_version) = Profile::host(&arguments);
    if full {
        assert!(
            bench_harness::argument(&arguments, "--hardware")
                .is_some_and(|value| !value.trim().is_empty()),
            "--bench requires nonempty --hardware"
        );
        assert!(
            bench_harness::argument(&arguments, "--os-version")
                .is_some_and(|value| !value.trim().is_empty()),
            "--bench requires nonempty --os-version"
        );
    }
    let mut measurements = Vec::with_capacity(Case::ALL.len() * 2);
    for case in Case::ALL {
        for (side, name) in case.names().into_iter().enumerate() {
            let metrics = Metrics::new(plan);
            let workload = bench_harness::measure(
                name,
                case.oracle(),
                plan,
                || {
                    metrics
                        .fixture_baseline
                        .set(bench_harness::live_allocated_bytes());
                    let ready = match (case, side) {
                        (Case::FirstTerminal, 0) => Ready::FirstRust,
                        (Case::FirstTerminal, _) => Ready::FirstC,
                        (_, 0) => Ready::Rust(RustFlow::new(case)),
                        _ => Ready::C(CFlow::new(case)),
                    };
                    Fixture {
                        case,
                        ready,
                        metrics: &metrics,
                    }
                },
                exercise,
            );
            measurements.push(Measurement {
                case,
                workload,
                metrics,
            });
        }
    }

    if full {
        report(plan, &hardware, &os_version, &measurements);
    } else {
        for measurement in &measurements {
            bench_harness::summarize(
                "template-watch-boundary",
                plan,
                std::slice::from_ref(&measurement.workload),
            );
        }
    }
    // No inherited Phase 1 growth allowance or historical native ceiling.
    // Emit observations first so a failed structural gate retains its evidence.
    for measurement in &measurements {
        bench_harness::enforce_correctness(std::slice::from_ref(&measurement.workload));
        let observations = measurement.metrics.observations.borrow();
        assert_eq!(
            observations.len(),
            plan.samples(),
            "every retained window was recorded"
        );
        if measurement.case.requires_zero_allocations() {
            for (index, observation) in observations.iter().enumerate() {
                assert_eq!(
                    observation.calls.allocations(),
                    0,
                    "{} sample {index}: caller alloc + alloc_zeroed + realloc must be zero",
                    measurement.workload.name()
                );
            }
        }
    }
}

// The shared reporter cannot omit an unknown mapped-byte observation. Reuse its
// benchmark block, Plan, Profile host parsing, measurement and heap accounting,
// but emit only supported observations here rather than print a fictional zero.
fn report(plan: Plan, hardware: &str, os_version: &str, measurements: &[Measurement]) {
    println!("format_version = 1\n\n[benchmark]");
    for (key, value) in bench_harness::benchmark_block(&Benchmark {
        id: "phase-5-template-watch-boundary",
        workload: "paired public Rust-facade and negotiated C-table query observation costs",
        phase: "5",
    }) {
        println!("{key} = {value}");
    }
    println!("\n[profile]");
    text(
        "fixture",
        "fixtures/assets/phase1-slice; generated match_fixtures RGBA scene; two-frame rate-held absent and one-frame matched replay",
    );
    text("fixture_sha256", &fixture_digest().to_string());
    text(
        "scene_rgba_sha256",
        &ContentDigest::of(&match_fixtures::scene_pixels(
            mado_pilot::PixelFormat::Rgba8,
        ))
        .to_string(),
    );
    text("release_target", bench_harness::RELEASE_TARGET);
    text("hardware", hardware);
    text("os_version", os_version);
    text(
        "build_profile",
        &format!(
            "template-watch-boundary; debug_assertions={}; private-fixture={}; coreml-provider={}; cuda-provider={}; qualification-unsupported-api={}",
            cfg!(debug_assertions),
            cfg!(feature = "private-fixture"),
            cfg!(feature = "coreml-provider"),
            cfg!(feature = "cuda-provider"),
            cfg!(feature = "qualification-unsupported-api")
        ),
    );
    println!(
        "warmup_iterations = {}\nsample_count = {}",
        plan.warmup(),
        plan.samples()
    );
    text(
        "correctness_oracle",
        "every warmup and retained observation checks its consumer oracle; zero-allocation gates inspect allocation calls, not net bytes",
    );
    text(
        "queue_policy",
        "existing finite scheduler; pending fixture holds one rate-ineligible latest frame after one no-match completion; its fixed two-frame source cannot publish another frame; every pending snapshot is checked unchanged",
    );
    text(
        "latency_scope",
        "one batch of caller observations including fixed-value oracle comparisons and transient reference release; no replay/engine/package/template setup, matching completion wait, parent close, report storage or output",
    );
    text(
        "first_terminal_scope",
        "one first poll and its owned result publication only; SessionClosed is committed by public close before the window; result oracle and release are after the window; no matched-backend completion is timed",
    );
    text(
        "deadline_scope",
        "finite operation authority, not preemption of an uninterruptible backend or a hard wall-clock guarantee; poll, retain, release and value access use their public nonblocking/bounded lifecycle contracts",
    );
    text(
        "allocation_scope",
        "caller-thread Rust GlobalAlloc calls only; includes alloc, alloc_zeroed and every realloc (even a same-size or shrinking call); excludes other threads, foreign malloc and independently loaded libraries",
    );
    text(
        "heap_scope",
        "process-wide live Rust heap via existing Accounting, including fixture and concurrent workers; peak/growth use shared harness baselines; first-terminal rows also include per-sample untimed setup/teardown in heap and iteration_span",
    );
    text(
        "mapped_bytes_scope",
        "not instrumented: no mapped_bytes_per_result or extra-mapped-byte assertion; unchanged pending authority is observed, not a byte measurement; retained-frame rows report actual readable mapping length and check shared-storage flags in their oracle",
    );
    text(
        "qualification",
        "observations only; correctness and structural caller zero allocations enforced; target latency and heap budgets require separate acceptance; no native support or historical evidence promotion",
    );
    println!("setup_operation_timeout_nanos = {}", SETUP_WAIT.as_nanos());
    println!("query_lifetime_nanos = {}", QUERY_LIFETIME.as_nanos());
    println!(
        "pending_minimum_interval_nanos = {}",
        PENDING_INTERVAL.as_nanos()
    );
    for measurement in measurements {
        let workload = &measurement.workload;
        let metrics = &measurement.metrics;
        let observations = metrics.observations.borrow();
        println!("\n[[measurement]]");
        text("workload", workload.name());
        text("correctness_oracle", measurement.case.oracle());
        println!(
            "observations_per_sample = {}",
            measurement.case.repetitions()
        );
        println!("result_correctness = {}", workload.incorrect());
        println!("latency_p50_ms = {:.6}", workload.percentile(0.50));
        println!("latency_p95_ms = {:.6}", workload.percentile(0.95));
        println!(
            "latency_max_ms = {:.6}",
            workload.max_elapsed().as_secs_f64() * 1_000.0
        );
        println!("iteration_span_ms = {:.6}", workload.iteration_span_ms());
        println!("peak_allocated_bytes = {}", workload.peak_allocated_bytes());
        println!(
            "steady_allocated_bytes = {}",
            metrics
                .ending_live
                .get()
                .saturating_sub(metrics.fixture_baseline.get())
        );
        println!("allocated_growth_bytes = {}", workload.growth_bytes());
        println!(
            "caller_zero_allocation_gate = {}",
            measurement.case.requires_zero_allocations()
        );
        array(
            "latency_samples_nanos",
            observations.iter().map(|sample| sample.elapsed.as_nanos()),
        );
        array(
            "caller_allocation_calls",
            observations.iter().map(|sample| sample.calls.allocations()),
        );
        array(
            "caller_alloc_calls",
            observations.iter().map(|sample| sample.calls.alloc),
        );
        array(
            "caller_alloc_zeroed_calls",
            observations.iter().map(|sample| sample.calls.alloc_zeroed),
        );
        array(
            "caller_realloc_calls",
            observations.iter().map(|sample| sample.calls.realloc),
        );
        array(
            "caller_dealloc_calls",
            observations.iter().map(|sample| sample.calls.dealloc),
        );
        array(
            "process_live_bytes_before_window",
            observations.iter().map(|sample| sample.live_before),
        );
        array(
            "process_live_bytes_after_window",
            observations.iter().map(|sample| sample.live_after),
        );
        if measurement.case == Case::RetainedFrame {
            array(
                "readable_frame_view_bytes_per_sample",
                observations.iter().map(|sample| {
                    sample
                        .readable_view
                        .expect("retained-frame sample observed a real byte view")
                }),
            );
        }
    }
}

fn text(key: &str, value: &str) {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    println!("{key} = \"{escaped}\"");
}

fn array(key: &str, values: impl IntoIterator<Item = impl std::fmt::Display>) {
    print!("{key} = [");
    for (index, value) in values.into_iter().enumerate() {
        if index != 0 {
            print!(", ");
        }
        print!("{value}");
    }
    println!("]");
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures")
}

fn package_root() -> PathBuf {
    fixtures().join("assets/phase1-slice")
}

fn fixture_digest() -> ContentDigest {
    ContentDigest::of(
        &std::fs::read(package_root().join("SHA256SUMS")).expect("tracked fixture checksum file"),
    )
}
