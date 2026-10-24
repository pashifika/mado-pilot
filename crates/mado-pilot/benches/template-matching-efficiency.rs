//! Capture-free, oracle-checked public matching and production score selection.
//!
//! `template-matching-timing` builds the normal mode: system allocator, no work
//! counters. `template-matching-benchmark` builds the instrumented mode with
//! counted Rust allocation and backend/mapping work. Protocol4 adds explicit
//! sampling plans and retained-loop CPU; historical artifacts keep their readers.

#[cfg(not(feature = "template-matching-benchmark"))]
const _: () = assert!(
    !mado_pilot_backend_opencv::benchmark_selection::INSTRUMENTATION_ENABLED,
    "normal template-matching timing excludes matching and mapping instrumentation"
);

#[cfg(feature = "template-matching-benchmark")]
use std::alloc::{GlobalAlloc, Layout};
use std::io::Write;
use std::sync::Barrier;
#[cfg(feature = "template-matching-benchmark")]
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use mado_pilot::replay::{ReplayFrame, ReplaySource, ReplayTarget};
use mado_pilot::{
    ClipPolicy, Continuity, CpuMapping, Frame, FrameDescriptor, FrameRequest, MatchOptions,
    MatchResult, MonotonicInstant, OpenRequest, OperationContext, PixelFormat, PixelRect,
    PreparedTemplate, RegionSelection,
};
use mado_pilot_backend_opencv::OpenCvBackend;
#[cfg(feature = "template-matching-benchmark")]
use mado_pilot_backend_opencv::benchmark_instrumentation as backend;
use mado_pilot_backend_opencv::benchmark_selection::{
    SelectionRequest, parallel_threads, select_scores,
};
use mado_pilot_runtime::Matcher;
#[cfg(feature = "template-matching-benchmark")]
use mado_pilot_testkit::bench_harness::{
    Accounting, live_allocated_bytes, peak_allocated_bytes, reset_allocation_peak,
};
use mado_pilot_testkit::{MatchBackend, MatchRequest};
use serde::Serialize;
use serde_json::{Value, json};

#[path = "support/process_cpu.rs"]
mod cpu;
#[path = "support/template_efficiency_fixtures.rs"]
mod fixtures;
#[path = "support/template_efficiency_formats.rs"]
mod formats;
#[path = "support/template_efficiency_queries.rs"]
mod queries;
#[path = "support/ocr_text_watch_resources.rs"]
mod resources;
#[path = "support/template_efficiency_runtime.rs"]
mod runtime;
#[path = "support/template_efficiency_session_fixtures.rs"]
mod session_fixtures;
#[path = "support/template_efficiency_sessions.rs"]
mod sessions;
#[cfg(feature = "template-matching-benchmark")]
#[path = "support/template_efficiency_watch.rs"]
mod watch;
use fixtures::{Kind, PATCH, ROI, SCORE_TOLERANCE, WORKLOADS, Workload};

const INSTRUMENTED: bool = cfg!(feature = "template-matching-benchmark");
const TIMING_BOUNDARY_PROTOCOL: &str = "mado-component-timing-v1";
const SAMPLING_PROTOCOL: &str = "mado-component-sampling-v1";
const RETAINED_LOOP_CPU_PROTOCOL: &str = "mado-component-retained-loop-cpu-v1";
const MAX_WARMUPS: usize = 1_000;
const MAX_SAMPLES: usize = 40_000;
const BUILD_MODE: &str = if INSTRUMENTED {
    "instrumented"
} else {
    "normal"
};

/// Metric keys observed only by the instrumented build; null in normal mode.
const INSTRUMENTED_METRICS: [&str; 26] = [
    "allocations",
    "heap_before_bytes",
    "heap_after_bytes",
    "heap_after_drop_bytes",
    "heap_steady_before_bytes",
    "heap_steady_after_bytes",
    "peak_heap_bytes",
    "peak_heap_scope",
    "score_copied_bytes",
    "score_offsets_scanned",
    "bgr_conversions",
    "bgr_converted_bytes",
    "bgr_retained_bytes",
    "bgr_retained_peak_bytes",
    "bgr_retained_after_drop_bytes",
    "bgr_evicted_bytes",
    "bgr_evicted_peak_bytes",
    "bgr_evicted_after_drop_bytes",
    "mapping_copies",
    "mapping_copied_bytes",
    "logical_mapped_bytes",
    "backend_find_calls",
    "backend_find_failures",
    "peak_active_finds",
    "peak_active_finds_scope",
    "final_owner_correct",
];

const UNAVAILABLE_REASON: &str = "normal build compiles no Rust allocation accounting or \
    backend/mapping work counters: listed metrics, including the final_owner_correct \
    owner/cache oracle, are unobserved; listed workloads' oracles are those counters";

/// Rows whose oracle is backend/mapping work counters; instrumented builds only.
fn counter_oracle(row: Workload) -> bool {
    matches!(
        row.kind,
        Kind::RoiFormatTransition | Kind::WatchAnalysisAlways | Kind::WatchExactRgba
    )
}

#[cfg(feature = "template-matching-benchmark")]
#[global_allocator]
static ALLOCATOR: ObservedAccounting = ObservedAccounting;
#[cfg(feature = "template-matching-benchmark")]
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "template-matching-benchmark")]
struct ObservedAccounting;

#[cfg(feature = "template-matching-benchmark")]
// SAFETY: pointers, layouts and byte accounting pass through to Accounting.
// The additional atomic counts successful allocation/reallocation calls only.
unsafe impl GlobalAlloc for ObservedAccounting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's allocation contract passes through unchanged.
        let pointer = unsafe { Accounting.alloc(layout) };
        if !pointer.is_null() {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the caller supplied Accounting's pointer and original layout.
        unsafe { Accounting.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller's reallocation contract passes through unchanged.
        let pointer = unsafe { Accounting.realloc(pointer, layout, new_size) };
        if !pointer.is_null() {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        pointer
    }
}

#[derive(Clone, Copy)]
struct Memory {
    rss: Option<u64>,
    peak_rss: Option<u64>,
    private: Option<u64>,
    footprint: Option<u64>,
    #[cfg(feature = "template-matching-benchmark")]
    heap: u64,
}

impl Memory {
    fn observe() -> Self {
        let resident = resources::resident();
        Self {
            rss: resident.current,
            peak_rss: resident.peak,
            private: resident.private,
            footprint: resident.footprint,
            #[cfg(feature = "template-matching-benchmark")]
            heap: live_allocated_bytes() as u64,
        }
    }
}

#[derive(Serialize)]
struct Sample {
    elapsed_ns: u64,
    cpu_ns: Option<u64>,
    correct: bool,
    error: Option<String>,
}

struct ClockStart {
    wall: Instant,
    cpu: Option<u64>,
}

impl ClockStart {
    fn now() -> Self {
        let cpu = cpu::process_cpu_ns();
        Self {
            wall: Instant::now(),
            cpu,
        }
    }

    fn finish(self) -> (u64, Option<u64>) {
        let elapsed = self.wall.elapsed();
        let cpu = cpu::process_cpu_ns().and_then(|now| now.checked_sub(self.cpu?));
        (nanos(elapsed), cpu)
    }
}

fn nanos(value: Duration) -> u64 {
    u64::try_from(value.as_nanos()).expect("bounded benchmark duration fits u64 nanoseconds")
}

fn operation() -> OperationContext {
    OperationContext::new()
        .with_timeout(Duration::from_secs(5))
        .expect("bounded matching operation")
}

fn matcher() -> Result<Matcher, String> {
    OpenCvBackend::new()
        .map(|backend| Matcher::new(std::sync::Arc::new(backend)))
        .map_err(|error| error.to_string())
}

fn replay_frame(row: Workload, pixels: Vec<u8>, format: PixelFormat) -> Result<Frame, String> {
    let descriptor =
        FrameDescriptor::packed(row.extent(), format).map_err(|error| error.to_string())?;
    replay_frame_with_descriptor(descriptor, pixels)
}

fn replay_frame_with_descriptor(
    descriptor: FrameDescriptor,
    pixels: Vec<u8>,
) -> Result<Frame, String> {
    let replay = ReplayFrame::new(
        descriptor,
        MonotonicInstant::ORIGIN,
        Continuity::Continuous,
        None,
        pixels.into_boxed_slice(),
    )
    .map_err(|error| error.to_string())?;
    let target = ReplayTarget::new("template-efficiency", vec![replay])
        .map_err(|error| error.to_string())?;
    let source = ReplaySource::from_targets(vec![target]).map_err(|error| error.to_string())?;
    let engine = mado_pilot::replay_engine(source).map_err(|error| error.to_string())?;
    let operation = operation();
    let targets = engine
        .discover(&operation)
        .map_err(|error| error.to_string())?;
    let target = targets.first().ok_or("replay target missing")?;
    let session = engine
        .open(target.id(), &OpenRequest::new(), &operation)
        .map_err(|error| error.to_string())?;
    let frame = session
        .acquire_frame(&FrameRequest::latest(), &operation)
        .map_err(|error| error.to_string())?;
    session
        .close(&operation)
        .map_err(|error| error.to_string())?;
    Ok(frame)
}

fn region(row: Workload) -> PixelRect {
    if row.roi {
        PixelRect::new(ROI.0, ROI.1, ROI.2, ROI.3).expect("fixed ROI")
    } else {
        PixelRect::new(
            0,
            0,
            i32::try_from(row.width).expect("bounded width"),
            i32::try_from(row.height).expect("bounded height"),
        )
        .expect("fixed frame extent")
    }
}

fn options(template: &PreparedTemplate) -> MatchOptions {
    MatchOptions::from_defaults(template.defaults())
}

fn find(
    matcher: &Matcher,
    frame: &Frame,
    template: &PreparedTemplate,
    searched: PixelRect,
    operation: &OperationContext,
) -> mado_pilot::Result<MatchResult> {
    matcher.find(
        MatchRequest::new(
            frame,
            RegionSelection::pixels(searched, ClipPolicy::Reject)?,
            template,
            options(template),
        ),
        operation,
    )
}

fn check(
    result: &MatchResult,
    frame: &Frame,
    template: &PreparedTemplate,
    searched: PixelRect,
    variant: u32,
    index: usize,
    row: Workload,
) -> bool {
    let (x, y) = fixtures::origin(row.extent(), index, variant);
    let bounds = PixelRect::new(
        x,
        y,
        x + i32::try_from(PATCH.width()).expect("patch width"),
        y + i32::try_from(PATCH.height()).expect("patch height"),
    )
    .expect("planted bounds");
    result.stamp() == frame.stamp()
        && result.transform() == frame.transform()
        && result.searched() == searched
        && result.options() == options(template)
        && result.backend().id() == template.backend().as_str()
        && result.matches().len() == 1
        && result.matches().len() <= result.options().max_results() as usize
        && result.matches().iter().all(|found| {
            found.template() == template.id()
                && found.bounds() == bounds
                && found.score().is_finite()
                && (found.score() - 1.0).abs() <= SCORE_TOLERANCE
        })
}

struct SearchFixture {
    row: Workload,
    pixels: Vec<u8>,
    templates: Vec<PreparedTemplate>,
    matcher: Option<Matcher>,
    frames: Vec<Frame>,
    mapping_owner: Option<CpuMapping>,
}

impl SearchFixture {
    fn new(row: Workload) -> Result<Self, String> {
        let compiler = matcher()?;
        let count = match row.kind {
            Kind::ColdBatch | Kind::LiveOwnerBatch | Kind::WarmBatch => 8,
            Kind::Concurrent => 2,
            _ => 1,
        };
        let templates = fixtures::templates()
            .iter()
            .take(count)
            .map(|source| {
                compiler
                    .prepare(source, &operation())
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            row,
            pixels: fixtures::scene(row.extent(), 0, PixelFormat::Rgba8),
            templates,
            matcher: None,
            frames: Vec::with_capacity(3),
            mapping_owner: None,
        })
    }

    fn prepare_iteration(&mut self) -> Result<(), String> {
        if matches!(self.row.kind, Kind::WarmBatch) && self.matcher.is_some() {
            return Ok(());
        }
        self.mapping_owner = None;
        self.frames.clear();
        self.matcher = Some(matcher()?);
        self.frames.push(replay_frame(
            self.row,
            self.pixels.clone(),
            PixelFormat::Rgba8,
        )?);
        match self.row.kind {
            Kind::DifferentFrame | Kind::EqualStamp | Kind::Eviction => {
                let shifted = fixtures::scene(self.row.extent(), 1, PixelFormat::Rgba8);
                let second = if matches!(self.row.kind, Kind::EqualStamp) {
                    let first = &self.frames[0];
                    Frame::new(
                        first.stamp(),
                        MonotonicInstant::ORIGIN,
                        first.descriptor(),
                        *first.transform(),
                        shifted.into_boxed_slice(),
                    )
                    .map_err(|error| error.to_string())?
                } else {
                    replay_frame(self.row, shifted, PixelFormat::Rgba8)?
                };
                self.frames.push(second);
                if matches!(self.row.kind, Kind::Eviction) {
                    self.frames.push(replay_frame(
                        self.row,
                        fixtures::scene(self.row.extent(), 2, PixelFormat::Rgba8),
                        PixelFormat::Rgba8,
                    )?);
                }
            }
            _ => {}
        }
        if matches!(self.row.kind, Kind::LiveOwnerBatch | Kind::ExpiredMapping) {
            let view = mado_pilot::FrameView::new(self.frames[0].clone(), region(self.row))
                .map_err(|error| error.to_string())?;
            let owner = view
                .map(
                    selected_format(
                        self.matcher.as_ref().expect("created matcher"),
                        &self.frames[0],
                    ),
                    &operation(),
                )
                .map_err(|error| error.to_string())?;
            if matches!(self.row.kind, Kind::LiveOwnerBatch) {
                self.mapping_owner = Some(owner);
            }
        }
        if matches!(self.row.kind, Kind::WarmBatch) {
            let warm = self.run_sequential();
            if !warm.correct {
                return Err(warm.error.unwrap_or_else(|| "prewarm oracle failed".into()));
            }
        }
        Ok(())
    }

    fn run(&self) -> Sample {
        if matches!(self.row.kind, Kind::Concurrent) {
            self.run_concurrent()
        } else {
            self.run_sequential()
        }
    }

    fn requests(&self) -> ([(usize, usize, u32, PixelRect); 8], usize) {
        let searched = region(self.row);
        let mut requests = [(0, 0, 0, searched); 8];
        let count = match self.row.kind {
            Kind::ColdBatch | Kind::LiveOwnerBatch | Kind::WarmBatch => {
                for (index, request) in requests.iter_mut().enumerate() {
                    request.1 = index;
                }
                8
            }
            Kind::DifferentFrame | Kind::EqualStamp => {
                requests[1] = (1, 0, 1, searched);
                2
            }
            Kind::ExpiredMapping => 2,
            Kind::Eviction => {
                requests[1] = (1, 0, 1, searched);
                requests[2] = (2, 0, 2, searched);
                4
            }
            _ => 1,
        };
        (requests, count)
    }

    fn run_sequential(&self) -> Sample {
        let matcher = self.matcher.as_ref().expect("iteration prepared");
        let (requests, count) = self.requests();
        let requests = &requests[..count];
        let operation = operation();
        let mut results: [Option<mado_pilot::Result<MatchResult>>; 8] =
            std::array::from_fn(|_| None);
        let started = ClockStart::now();
        for (slot, &(frame, template, _, searched)) in results.iter_mut().zip(requests) {
            *slot = Some(find(
                matcher,
                &self.frames[frame],
                &self.templates[template],
                searched,
                &operation,
            ));
        }
        let (elapsed_ns, cpu_ns) = started.finish();
        let mut correct = true;
        let mut error = None;
        for (result, &(frame, template, variant, searched)) in results.iter().zip(requests) {
            match result.as_ref().expect("one result per request") {
                Ok(result) => {
                    correct &= check(
                        result,
                        &self.frames[frame],
                        &self.templates[template],
                        searched,
                        variant,
                        template,
                        self.row,
                    )
                }
                Err(failure) => {
                    correct = false;
                    error.get_or_insert_with(|| failure.to_string());
                }
            }
        }
        if let Some(owner) = &self.mapping_owner {
            correct &= owner.stamp() == self.frames[0].stamp()
                && owner.region() == region(self.row)
                && !owner.is_shared()
                && owner.descriptor().extent().width() == 320
                && owner.descriptor().extent().height() == 180;
        }
        Sample {
            elapsed_ns,
            cpu_ns,
            correct,
            error,
        }
    }

    fn run_concurrent(&self) -> Sample {
        let matcher = self.matcher.as_ref().expect("iteration prepared");
        let frame = &self.frames[0];
        let searched = region(self.row);
        let ready = Barrier::new(3);
        let go = Barrier::new(3);
        let worker = |index| {
            let operation = operation();
            ready.wait();
            go.wait();
            let start = Instant::now();
            let result = find(matcher, frame, &self.templates[index], searched, &operation);
            (start, Instant::now(), result)
        };
        std::thread::scope(|scope| {
            let first = scope.spawn(|| worker(0));
            let second = scope.spawn(|| worker(1));
            ready.wait();
            let started = ClockStart::now();
            go.wait();
            let results = [first.join(), second.join()];
            let (elapsed_ns, cpu_ns) = started.finish();
            let mut windows = [None; 2];
            let mut correct = true;
            let mut error = None;
            for (index, result) in results.into_iter().enumerate() {
                match result {
                    Ok((start, end, result)) => {
                        windows[index] = Some((start, end));
                        match result {
                            Ok(result) => {
                                correct &= check(
                                    &result,
                                    frame,
                                    &self.templates[index],
                                    searched,
                                    0,
                                    index,
                                    self.row,
                                );
                            }
                            Err(failure) => {
                                correct = false;
                                error.get_or_insert_with(|| failure.to_string());
                            }
                        }
                    }
                    Err(_) => {
                        correct = false;
                        error.get_or_insert_with(|| "concurrent worker panicked".into());
                    }
                }
            }
            let overlap = matches!(windows, [Some((a, b)), Some((c, d))] if a.max(c) < b.min(d));
            if !overlap {
                correct = false;
                error.get_or_insert_with(|| "public Matcher calls did not overlap".into());
            }
            Sample {
                elapsed_ns,
                cpu_ns,
                correct,
                error,
            }
        })
    }
}

struct SelectionFixture {
    request: SelectionRequest,
    source: Vec<f32>,
    scratch: Vec<f32>,
    expected: Vec<(usize, usize, f64)>,
}

impl SelectionFixture {
    fn new(row: Workload) -> Self {
        let request = row.selection();
        let source = fixtures::score_map(row);
        let expected = fixtures::score_oracle(&source, request);
        Self {
            request,
            scratch: source.clone(),
            source,
            expected,
        }
    }

    fn run(&mut self) -> Sample {
        self.scratch.copy_from_slice(&self.source);
        let operation = operation();
        let started = ClockStart::now();
        let result = select_scores(&mut self.scratch, self.request, &operation);
        let (elapsed_ns, cpu_ns) = started.finish();
        let (correct, error) = match result {
            Ok(observed) => (
                observed.iter().len() <= self.request.candidate_budget
                    && observed.iter().eq(self.expected.iter().copied()),
                None,
            ),
            Err(error) => (false, Some(mado_pilot::Error::from(error).to_string())),
        };
        Sample {
            elapsed_ns,
            cpu_ns,
            correct,
            error,
        }
    }
}

fn selected_format(matcher: &Matcher, frame: &Frame) -> PixelFormat {
    let source = frame.descriptor().format();
    if matcher.descriptor().accepts_format(source) {
        source
    } else {
        matcher.descriptor().format()
    }
}

/// One counter-oracle contract iteration and its optional format observation.
#[cfg(feature = "template-matching-benchmark")]
fn contract_sample(row: Workload) -> (Sample, Option<Value>) {
    let started = ClockStart::now();
    if matches!(row.kind, Kind::RoiFormatTransition) {
        let result = formats::run(row);
        let (elapsed_ns, cpu_ns) = started.finish();
        let correct = result.as_ref().is_ok_and(|value| value["correct"] == true);
        let (observation, error) = match result {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error)),
        };
        let sample = Sample {
            elapsed_ns,
            cpu_ns,
            correct,
            error,
        };
        (sample, observation)
    } else {
        let result = watch::run(matches!(row.kind, Kind::WatchAnalysisAlways));
        let (elapsed_ns, cpu_ns) = started.finish();
        let sample = Sample {
            elapsed_ns,
            cpu_ns,
            correct: result.as_ref().is_ok_and(|correct| *correct),
            error: result.err(),
        };
        (sample, None)
    }
}

/// Normal builds refuse counter-oracle rows instead of weakening their oracle.
#[cfg(not(feature = "template-matching-benchmark"))]
fn contract_sample(row: Workload) -> (Sample, Option<Value>) {
    let sample = Sample {
        elapsed_ns: 0,
        cpu_ns: None,
        correct: false,
        error: Some(format!("{} requires the instrumented build", row.id)),
    };
    (sample, None)
}

fn emit_timing_boundary(enabled: bool, id: &str, phase: &str) -> Result<(), String> {
    if !enabled {
        return Ok(());
    }
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(format!("invalid timing boundary workload ID: {id}"));
    }
    let mut buffer = [0_u8; 256];
    let unused = {
        let mut line = buffer.as_mut_slice();
        writeln!(
            line,
            "{TIMING_BOUNDARY_PROTOCOL}[{}]: {id} {phase}",
            std::process::id()
        )
        .map_err(|error| format!("failed to format timing boundary for {id} {phase}: {error}"))?;
        line.len()
    };
    let length = buffer.len() - unused;
    // One unbuffered write; never retry or complete a torn marker.
    let written = std::io::stderr()
        .write(&buffer[..length])
        .map_err(|error| format!("failed to write timing boundary for {id} {phase}: {error}"))?;
    if written != length {
        return Err(format!(
            "short timing boundary write for {id} {phase}: {written}/{length} bytes"
        ));
    }
    Ok(())
}

fn observe(
    row: Workload,
    smoke: bool,
    timing_boundaries: bool,
    sampling: Option<Sampling>,
) -> Result<Value, String> {
    emit_timing_boundary(timing_boundaries, row.id, "row-start")?;
    let (warmups, count) =
        sampling.map_or_else(|| row.plan(smoke), |plan| (plan.warmups, plan.samples));
    let mut warmup_samples = Vec::with_capacity(warmups);
    let mut samples = Vec::with_capacity(count);
    let before = Memory::observe();
    #[cfg(feature = "template-matching-benchmark")]
    let (allocations_before, work_before, backend_before) = {
        reset_allocation_peak();
        (
            ALLOCATIONS.load(Ordering::Relaxed),
            backend::work_snapshot(),
            backend::snapshot(),
        )
    };
    let mut steady_before = before;
    let mut setup_error = None;
    let mut selection = matches!(row.kind, Kind::Selection).then(|| SelectionFixture::new(row));
    let is_contract = counter_oracle(row);
    let is_session = matches!(row.kind, Kind::Session(_));
    let mut format_observations =
        Vec::with_capacity(if matches!(row.kind, Kind::RoiFormatTransition) {
            warmups + count
        } else {
            0
        });
    let mut unit_observations = Vec::with_capacity(if is_session { warmups + count } else { 0 });
    let mut session = if is_session {
        match sessions::Fixture::new(row) {
            Ok(fixture) => Some(fixture),
            Err(error) => {
                setup_error = Some(error);
                None
            }
        }
    } else {
        None
    };
    let mut search = if selection.is_none() && !is_contract && !is_session {
        match SearchFixture::new(row) {
            Ok(fixture) => Some(fixture),
            Err(error) => {
                setup_error = Some(error);
                None
            }
        }
    } else {
        None
    };
    let mut loop_cpu_start = None;
    let mut loop_cpu_end = None;
    if setup_error.is_none() {
        for iteration in 0..warmups + count {
            if iteration == warmups {
                emit_timing_boundary(timing_boundaries, row.id, "samples-begin")?;
                loop_cpu_start = cpu::process_cpu_ns();
            }
            let sample = if let Some(fixture) = &mut selection {
                fixture.run()
            } else if let Some(fixture) = &mut session {
                match fixture.prepare_iteration() {
                    Ok(()) => {
                        let (sample, observation) = fixture.run();
                        unit_observations.push(observation);
                        sample
                    }
                    Err(error) => {
                        unit_observations.push(json!({"correct": false, "error": error}));
                        Sample {
                            elapsed_ns: 0,
                            cpu_ns: None,
                            correct: false,
                            error: Some(error),
                        }
                    }
                }
            } else if is_contract {
                let (sample, observation) = contract_sample(row);
                format_observations.extend(observation);
                sample
            } else {
                let fixture = search.as_mut().expect("search fixture created");
                match fixture.prepare_iteration() {
                    Ok(()) => fixture.run(),
                    Err(error) => Sample {
                        elapsed_ns: 0,
                        cpu_ns: None,
                        correct: false,
                        error: Some(error),
                    },
                }
            };
            if iteration < warmups {
                warmup_samples.push(sample);
            } else {
                samples.push(sample);
            }
            if iteration + 1 == warmups {
                if let Some(fixture) = &mut session {
                    fixture.checkpoint("after-warmups");
                }
                steady_before = Memory::observe();
            }
        }
        loop_cpu_end = cpu::process_cpu_ns();
        emit_timing_boundary(timing_boundaries, row.id, "samples-end")?;
    }
    if let Some(fixture) = &mut session {
        fixture.checkpoint("after-samples");
    }
    let after = Memory::observe();
    #[cfg(feature = "template-matching-benchmark")]
    let (work_after, activity) = (
        backend::work_snapshot(),
        backend::snapshot().checked_delta(backend_before),
    );
    let mapping_owner_live = search
        .as_ref()
        .is_some_and(|fixture| fixture.mapping_owner.is_some())
        || session
            .as_ref()
            .is_some_and(sessions::Fixture::mapping_owner_live);
    let mut owner_error = None;
    let owner_observations = match session.as_mut().map(sessions::Fixture::finish) {
        Some(Ok(observations)) => observations,
        Some(Err(error)) => {
            owner_error = Some(error);
            Value::Null
        }
        None => Value::Null,
    };
    let owner_failures = owner_observations.as_array().map_or(0, |points| {
        points
            .iter()
            .filter(|point| !point["error"].is_null() || point["owner_correct"] == false)
            .count()
    });
    drop(selection.take());
    drop(search.take());
    drop(session.take());
    let after_drop = Memory::observe();
    #[cfg(feature = "template-matching-benchmark")]
    let (heap_peak, allocations, final_work) = (
        peak_allocated_bytes() as u64,
        ALLOCATIONS
            .load(Ordering::Relaxed)
            .saturating_sub(allocations_before),
        backend::work_snapshot(),
    );
    let cpu_ns = samples
        .iter()
        .try_fold(0_u64, |sum, sample| sum.checked_add(sample.cpu_ns?));
    let cpu_missing = warmup_samples
        .iter()
        .chain(&samples)
        .any(|sample| sample.cpu_ns.is_none());
    let native_missing = [
        before.rss,
        before.peak_rss,
        steady_before.rss,
        after.rss,
        after.peak_rss,
        after_drop.rss,
        after_drop.peak_rss,
    ]
    .iter()
    .any(Option::is_none);
    #[cfg(feature = "template-matching-benchmark")]
    let (lifecycle_correct, counted) = {
        let independent = matches!(
            row.kind,
            Kind::Session(session_fixtures::Case::LifecycleIndependent)
        );
        let cache_limit = 33_554_432 * if independent { 2 } else { 1 };
        let lifecycle_correct = final_work.bgr_retained_bytes == work_before.bgr_retained_bytes
            && final_work.bgr_evicted_bytes == work_before.bgr_evicted_bytes
            && work_after.bgr_retained_bytes <= cache_limit
            && work_after.bgr_retained_peak_bytes <= cache_limit
            && activity.is_some_and(|value| {
                value.active_finds == 0
                    && (!matches!(row.kind, Kind::Concurrent) || value.peak_active_finds >= 2)
            });
        let counted = json!({
            "allocations": allocations,
            "heap_before_bytes": before.heap, "heap_after_bytes": after.heap,
            "heap_after_drop_bytes": after_drop.heap, "peak_heap_bytes": heap_peak,
            "heap_steady_before_bytes": steady_before.heap, "heap_steady_after_bytes": after.heap,
            "peak_heap_scope": "row-including-setup-and-warmup",
            "score_copied_bytes": work_after.score_copied_bytes.saturating_sub(work_before.score_copied_bytes),
            "score_offsets_scanned": work_after.score_offsets_scanned.saturating_sub(work_before.score_offsets_scanned),
            "bgr_conversions": work_after.bgr_conversions.saturating_sub(work_before.bgr_conversions),
            "bgr_converted_bytes": work_after.bgr_converted_bytes.saturating_sub(work_before.bgr_converted_bytes),
            "bgr_retained_bytes": work_after.bgr_retained_bytes,
            "bgr_retained_peak_bytes": work_after.bgr_retained_peak_bytes,
            "bgr_retained_after_drop_bytes": final_work.bgr_retained_bytes,
            "bgr_evicted_bytes": work_after.bgr_evicted_bytes,
            "bgr_evicted_peak_bytes": work_after.bgr_evicted_peak_bytes,
            "bgr_evicted_after_drop_bytes": final_work.bgr_evicted_bytes,
            "mapping_copies": work_after.mapping_copies.saturating_sub(work_before.mapping_copies),
            "mapping_copied_bytes": work_after.mapping_copied_bytes.saturating_sub(work_before.mapping_copied_bytes),
            "logical_mapped_bytes": activity.map(|value| value.mapped_bytes),
            "backend_find_calls": activity.map(|value| value.find_calls),
            "backend_find_failures": activity.map(|value| value.find_failures),
            "peak_active_finds": activity.map(|value| value.peak_active_finds),
            "peak_active_finds_scope": "process-high-water",
            "final_owner_correct": lifecycle_correct,
        });
        (Some(lifecycle_correct), counted)
    };
    // No counter source exists; the owner/cache oracle stays unevaluated.
    #[cfg(not(feature = "template-matching-benchmark"))]
    let (lifecycle_correct, counted) = (
        None::<bool>,
        Value::Object(
            INSTRUMENTED_METRICS
                .iter()
                .map(|&key| (key.to_owned(), Value::Null))
                .collect(),
        ),
    );
    assert!(
        counted.as_object().is_some_and(|object| {
            object.len() == INSTRUMENTED_METRICS.len()
                && INSTRUMENTED_METRICS
                    .iter()
                    .all(|&key| object.contains_key(key))
        }),
        "instrumented metric keys match the declared normal-mode null set"
    );
    let oracle_failures = warmup_samples
        .iter()
        .chain(&samples)
        .filter(|sample| !sample.correct)
        .count()
        + usize::from(setup_error.is_some())
        + usize::from(lifecycle_correct == Some(false))
        + usize::from(owner_error.is_some())
        + owner_failures;
    let mut report = row.manifest(smoke);
    let object = report.as_object_mut().expect("manifest object");
    object.insert("warmups".into(), json!(warmups));
    object.insert("sample_count".into(), json!(count));
    let loop_cpu_ns = loop_cpu_start
        .zip(loop_cpu_end)
        .and_then(|(start, end)| end.checked_sub(start));
    object.insert(
        "retained_loop_cpu".into(),
        json!({
            "protocol": RETAINED_LOOP_CPU_PROTOCOL,
            "source": if cfg!(target_os = "macos") {
                "getrusage-RUSAGE_SELF"
            } else if cfg!(windows) {
                "GetProcessTimes-current-process"
            } else {
                "unsupported"
            },
            "source_unit_ns": if cfg!(windows) { Some(100) } else if cfg!(target_os = "macos") { Some(1000) } else { None },
            "start_ns": loop_cpu_start, "end_ns": loop_cpu_end, "cpu_ns": loop_cpu_ns,
            "sample_count": count,
            "reason": loop_cpu_ns.is_none().then_some("retained-loop process CPU unavailable or nonmonotonic"),
            "scope": "current-process-retained-loop-including-preparation-oracles-and-storage",
        }),
    );
    object.insert("samples".into(), json!(samples));
    object.insert("warmup_samples".into(), json!(warmup_samples));
    object.insert("oracle_failures".into(), json!(oracle_failures));
    object.insert("setup_error".into(), json!(setup_error));
    if matches!(row.kind, Kind::RoiFormatTransition) {
        object.insert("format_observations".into(), json!(format_observations));
    }
    if is_session {
        object.insert("unit_observations".into(), json!(unit_observations));
        object.insert("owner_observations".into(), owner_observations);
        object.insert("owner_error".into(), json!(owner_error));
    }
    let mut metrics = json!({
        "iterations": warmups + count, "cpu_ns": cpu_ns,
        "cpu_reason": if cpu_missing { Some("process CPU observation unavailable or nonmonotonic") } else { None },
        "native_resource_reason": if native_missing { Some("self-process resident/high-water observation unavailable") } else { None },
        "peak_rss_bytes": after_drop.peak_rss, "rss_before_bytes": before.rss,
        "rss_after_bytes": after.rss, "rss_after_drop_bytes": after_drop.rss,
        "rss_steady_before_bytes": steady_before.rss, "rss_steady_after_bytes": after.rss,
        "private_after_bytes": after.private, "footprint_after_bytes": after.footprint,
        "private_reason": if after.private.is_none() { Some("not exposed by this target resource helper") } else { None },
        "footprint_reason": if after.footprint.is_none() { Some("not exposed by this target resource helper") } else { None },
        "native_allocation_scope": if INSTRUMENTED {
            "cache-payload gauges and process RSS, not total native allocation tracking"
        } else {
            "process RSS only; no cache-payload gauges or native allocation tracking"
        },
        "peak_rss_scope": "process-high-water",
        "mapping_owner_live_before_drop": mapping_owner_live,
    });
    if let (Some(metrics), Value::Object(counted)) = (metrics.as_object_mut(), counted) {
        metrics.extend(counted);
    }
    object.insert("metrics".into(), metrics);
    Ok(report)
}

const PROTOCOL_VERSION: u8 = 4;

fn workload_rows() -> impl Iterator<Item = Workload> {
    WORKLOADS
        .iter()
        .chain(&session_fixtures::WORKLOADS)
        .copied()
}

#[derive(Clone, Copy)]
struct Sampling {
    warmups: usize,
    samples: usize,
}

struct Arguments {
    list: bool,
    smoke: bool,
    timing_boundaries: bool,
    workload: Option<String>,
    sampling: Option<Sampling>,
}

fn arguments() -> Result<Arguments, String> {
    let mut parsed = Arguments {
        list: false,
        smoke: false,
        timing_boundaries: false,
        workload: None,
        sampling: None,
    };
    let mut protocol_given = false;
    let mut warmups = None;
    let mut samples = None;
    let mut values = std::env::args().skip(1);
    while let Some(value) = values.next() {
        match value.as_str() {
            "--list" => parsed.list = true,
            "--smoke" => parsed.smoke = true,
            "--timing-boundaries" => parsed.timing_boundaries = true,
            "--bench" => {}
            "--workload" => {
                if parsed.workload.is_some() {
                    return Err("--workload supplied twice".into());
                }
                parsed.workload = Some(values.next().ok_or("--workload requires an ID")?);
            }
            "--warmups" | "--samples" => {
                let (destination, maximum) = if value == "--warmups" {
                    (&mut warmups, MAX_WARMUPS)
                } else {
                    (&mut samples, MAX_SAMPLES)
                };
                if destination.is_some() {
                    return Err(format!("{value} supplied twice"));
                }
                let count = values
                    .next()
                    .ok_or_else(|| format!("{value} requires a count"))?
                    .parse::<usize>()
                    .map_err(|_| format!("{value} requires an integer in 1..={maximum}"))?;
                if !(1..=maximum).contains(&count) {
                    return Err(format!("{value} requires an integer in 1..={maximum}"));
                }
                *destination = Some(count);
            }
            "--protocol-version" => {
                if protocol_given {
                    return Err("--protocol-version supplied twice".into());
                }
                protocol_given = true;
                if values.next().as_deref() != Some("4") {
                    return Err("--protocol-version requires 4; historical artifacts retain their original protocol".into());
                }
            }
            _ => return Err(format!("unsupported argument: {value}")),
        }
    }
    parsed.sampling = match (warmups, samples) {
        (None, None) => None,
        (Some(warmups), Some(samples)) => Some(Sampling { warmups, samples }),
        _ => return Err("--warmups and --samples must be supplied together".into()),
    };
    if parsed.sampling.is_some() && (parsed.smoke || parsed.list || parsed.workload.is_none()) {
        return Err(
            "custom sampling requires one statistical --workload without --list or --smoke".into(),
        );
    }
    if let Some(id) = &parsed.workload {
        let row = workload_rows()
            .find(|row| row.id == id)
            .ok_or_else(|| format!("unknown workload: {id}"))?;
        if !INSTRUMENTED && counter_oracle(row) {
            return Err(format!("{id} requires the instrumented build"));
        }
        if parsed.sampling.is_some() && !row.statistical {
            return Err(format!("{id} keeps its fixed contract sampling plan"));
        }
    }
    Ok(parsed)
}

/// Declares observations with no source in this compiled mode.
fn unavailable() -> Value {
    if INSTRUMENTED {
        json!({"reason": null, "metrics": [], "workloads": []})
    } else {
        let workloads: Vec<_> = workload_rows()
            .filter(|row| counter_oracle(*row))
            .map(|row| row.id)
            .collect();
        json!({"reason": UNAVAILABLE_REASON, "metrics": INSTRUMENTED_METRICS, "workloads": workloads})
    }
}

/// Observes the native OpenCV thread setting without changing it.
fn thread_observation() -> Value {
    match parallel_threads() {
        Ok(observed) => {
            json!({"threads": observed.threads, "framework": observed.framework, "reason": null})
        }
        Err(reason) => json!({"threads": null, "framework": null, "reason": reason}),
    }
}

fn resolutions() -> (u64, Option<u64>) {
    let mut wall = u64::MAX;
    for _ in 0..1024 {
        let started = Instant::now();
        let mut elapsed = nanos(started.elapsed());
        while elapsed == 0 {
            std::hint::spin_loop();
            elapsed = nanos(started.elapsed());
        }
        wall = wall.min(elapsed);
    }
    let mut cpu = None;
    let deadline = Instant::now() + Duration::from_millis(100);
    let mut previous = cpu::process_cpu_ns();
    let mut transitions = 0;
    while Instant::now() < deadline && transitions < 8 {
        let current = cpu::process_cpu_ns();
        match (previous, current) {
            (Some(before), Some(after)) if after > before => {
                let delta = after - before;
                cpu = Some(cpu.map_or(delta, |minimum: u64| minimum.min(delta)));
                transitions += 1;
            }
            (_, None) => break,
            _ => {}
        }
        previous = current;
        std::hint::black_box(previous);
    }
    (wall, cpu)
}

fn target() -> String {
    if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
        "aarch64-apple-darwin".into()
    } else if cfg!(all(
        target_arch = "x86_64",
        target_os = "windows",
        target_env = "msvc"
    )) {
        "x86_64-pc-windows-msvc".into()
    } else {
        format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
    }
}

fn apparatus() -> Value {
    let sources: [(&str, &[u8]); 11] = [
        (
            "crates/mado-pilot/benches/template-matching-efficiency.rs",
            include_bytes!("template-matching-efficiency.rs"),
        ),
        (
            "crates/mado-pilot/benches/support/template_efficiency_fixtures.rs",
            include_bytes!("support/template_efficiency_fixtures.rs"),
        ),
        (
            "crates/mado-pilot/benches/support/template_efficiency_session_fixtures.rs",
            include_bytes!("support/template_efficiency_session_fixtures.rs"),
        ),
        (
            "crates/mado-pilot/benches/support/template_efficiency_sessions.rs",
            include_bytes!("support/template_efficiency_sessions.rs"),
        ),
        (
            "crates/mado-pilot/benches/support/template_efficiency_queries.rs",
            include_bytes!("support/template_efficiency_queries.rs"),
        ),
        (
            "crates/mado-pilot/benches/support/template_efficiency_formats.rs",
            include_bytes!("support/template_efficiency_formats.rs"),
        ),
        (
            "crates/mado-pilot/benches/support/template_efficiency_watch.rs",
            include_bytes!("support/template_efficiency_watch.rs"),
        ),
        (
            "crates/mado-pilot/benches/support/template_efficiency_runtime.rs",
            include_bytes!("support/template_efficiency_runtime.rs"),
        ),
        (
            "crates/mado-pilot/benches/support/process_cpu.rs",
            include_bytes!("support/process_cpu.rs"),
        ),
        (
            "crates/mado-pilot/benches/support/ocr_text_watch_resources.rs",
            include_bytes!("support/ocr_text_watch_resources.rs"),
        ),
        (
            "crates/support/testkit/src/match_fixtures.rs",
            include_bytes!("../../support/testkit/src/match_fixtures.rs"),
        ),
    ];
    let source_files: serde_json::Map<_, _> = sources
        .into_iter()
        .map(|(path, bytes)| {
            (
                path.to_owned(),
                mado_pilot::ContentDigest::of(bytes).to_string().into(),
            )
        })
        .collect();
    json!({"accounting_contract": "prepared-cache-row-winner-v1", "source_files": source_files})
}

fn run(arguments: Arguments) -> Result<Value, String> {
    let backend = OpenCvBackend::new().map_err(|error| error.to_string())?;
    let version = backend.descriptor().version().to_owned();
    drop(backend);
    let threads_before = thread_observation();
    let (timer_resolution_ns, cpu_timer_resolution_ns) = resolutions();
    let workloads: Vec<_> = workload_rows()
        .filter(|row| INSTRUMENTED || !counter_oracle(*row))
        .filter(|row| arguments.workload.as_deref().is_none_or(|id| row.id == id))
        .map(|row| {
            if arguments.list {
                Ok(row.manifest(arguments.smoke))
            } else {
                observe(
                    row,
                    arguments.smoke,
                    arguments.timing_boundaries,
                    arguments.sampling,
                )
            }
        })
        .collect::<Result<_, String>>()?;
    let threads_after = thread_observation();
    // Corpus and source hashing are outside every measured resource interval.
    let (fixture_sha256, fixture) = fixtures::manifest();
    let short = arguments.smoke
        || (!arguments.list && workloads.len() == 1 && workloads[0]["statistical"] == false);
    Ok(json!({
        "format_version": PROTOCOL_VERSION, "kind": if arguments.list { "manifest" } else { "observations" },
        "timing_boundary_protocol": TIMING_BOUNDARY_PROTOCOL,
        "sampling_protocol": SAMPLING_PROTOCOL,
        "sampling_limits": {
            "warmups_min": 1, "warmups_max": MAX_WARMUPS,
            "samples_min": 1, "samples_max": MAX_SAMPLES,
        },
        "retained_loop_cpu_protocol": RETAINED_LOOP_CPU_PROTOCOL,
        "timing_boundaries_enabled": arguments.timing_boundaries,
        "target": target(), "opencv_version": version, "fixture_sha256": fixture_sha256,
        "loaded_native_runtime": runtime::observe()?,
        "fixture": fixture, "apparatus": apparatus(),
        "warmups": arguments.sampling.map_or(if short { 1 } else { 20 }, |plan| plan.warmups),
        "sample_count": arguments.sampling.map_or(if short { 3 } else { 200 }, |plan| plan.samples),
        "timer_resolution_ns": timer_resolution_ns, "cpu_timer_resolution_ns": cpu_timer_resolution_ns,
        "timer_resolution_method": "minimum-observed-positive-Instant-delta",
        "cpu_resolution_method": "minimum-observed-positive-process-cpu-delta",
        "cpu_clock_unit_ns": if cfg!(windows) { Some(100) } else if cfg!(target_os = "macos") { Some(1000) } else { None },
        "cpu_timer_resolution_source": "bounded self-process CPU probe; observed deltas are not guaranteed accounting accuracy",
        "timer_resolution_source": "minimum positive Instant delta probe",
        "workloads": workloads, "build_mode": BUILD_MODE,
        "global_allocator": if INSTRUMENTED { "accounting-over-system" } else { "system" },
        "unavailable": unavailable(),
        "opencv_parallel_threads": {
            "method": "opencv-runtime-backend-and-thread-setting-v1",
            "before": threads_before, "after": threads_after,
            "changed": threads_before != threads_after,
        },
    }))
}

fn main() {
    let arguments = arguments();
    let timing_boundaries_enabled = arguments
        .as_ref()
        .is_ok_and(|arguments| arguments.timing_boundaries);
    let (report, failed) = match arguments.and_then(run) {
        Ok(report) => {
            let failed = report["workloads"].as_array().is_some_and(|rows| {
                rows.iter().any(|row| {
                    row["oracle_failures"]
                        .as_u64()
                        .is_some_and(|count| count != 0)
                        || (report["kind"] == "observations"
                            && row["retained_loop_cpu"]["cpu_ns"].as_u64().is_none())
                })
            });
            let threads = &report["opencv_parallel_threads"];
            let thread_failure = threads["changed"].as_bool() != Some(false)
                || ["before", "after"].iter().any(|point| {
                    let observed = &threads[*point];
                    !observed["reason"].is_null()
                        || observed["threads"].as_i64().is_none_or(|count| count <= 0)
                        || observed["framework"].as_str().is_none_or(str::is_empty)
                });
            (report, failed || thread_failure)
        }
        Err(error) => {
            let report = json!({"format_version": PROTOCOL_VERSION, "kind": "observations",
                "target": target(), "build_mode": BUILD_MODE, "error": error, "workloads": [],
                "timing_boundary_protocol": TIMING_BOUNDARY_PROTOCOL,
                "timing_boundaries_enabled": timing_boundaries_enabled});
            (report, true)
        }
    };
    println!(
        "{}",
        serde_json::to_string(&report).expect("finite benchmark JSON")
    );
    if failed {
        std::process::exit(1);
    }
}
