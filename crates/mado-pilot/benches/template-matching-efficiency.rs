//! Capture-free, oracle-checked public matching and production score selection.

use std::alloc::{GlobalAlloc, Layout};
use std::sync::Barrier;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use mado_pilot::replay::{ReplayFrame, ReplaySource, ReplayTarget};
use mado_pilot::{
    ClipPolicy, Continuity, CpuMapping, Frame, FrameDescriptor, FrameRequest, MatchOptions,
    MatchResult, MonotonicInstant, OpenRequest, OperationContext, PixelFormat, PixelRect,
    PreparedTemplate, RegionSelection,
};
use mado_pilot_backend_opencv::{OpenCvBackend, benchmark_instrumentation as backend};
use mado_pilot_runtime::Matcher;
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
#[path = "support/ocr_text_watch_resources.rs"]
mod resources;
#[path = "support/template_efficiency_watch.rs"]
mod watch;
use fixtures::{Kind, PATCH, ROI, SCORE_TOLERANCE, WORKLOADS, Workload};

#[global_allocator]
static ALLOCATOR: ObservedAccounting = ObservedAccounting;
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

struct ObservedAccounting;

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

fn matcher() -> Result<Matcher, String> {
    OpenCvBackend::new()
        .map(|backend| Matcher::new(std::sync::Arc::new(backend)))
        .map_err(|error| error.to_string())
}

fn replay_frame(row: Workload, pixels: Vec<u8>, format: PixelFormat) -> Result<Frame, String> {
    let descriptor =
        FrameDescriptor::packed(row.extent(), format).map_err(|error| error.to_string())?;
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
    let operation = OperationContext::new();
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
            Kind::Concurrent | Kind::RoiFormatTransition => 2,
            _ => 1,
        };
        let templates = fixtures::templates()
            .iter()
            .take(count)
            .map(|source| {
                compiler
                    .prepare(source, &OperationContext::new())
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
            Kind::RoiFormatTransition => self.frames.push(replay_frame(
                self.row,
                fixtures::scene(self.row.extent(), 0, PixelFormat::Bgra8),
                PixelFormat::Bgra8,
            )?),
            _ => {}
        }
        if matches!(self.row.kind, Kind::LiveOwnerBatch | Kind::ExpiredMapping) {
            let view = mado_pilot::FrameView::new(self.frames[0].clone(), region(self.row))
                .map_err(|error| error.to_string())?;
            let owner = view
                .map(
                    self.matcher
                        .as_ref()
                        .expect("created matcher")
                        .descriptor()
                        .format(),
                    &OperationContext::new(),
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
            Kind::RoiFormatTransition => {
                requests[1] = (
                    0,
                    1,
                    0,
                    PixelRect::new(72, 56, 376, 220).expect("alternate ROI"),
                );
                requests[2] = (
                    1,
                    0,
                    0,
                    PixelRect::new(0, 0, 1920, 1080).expect("full frame"),
                );
                3
            }
            _ => 1,
        };
        (requests, count)
    }

    fn run_sequential(&self) -> Sample {
        let matcher = self.matcher.as_ref().expect("iteration prepared");
        let (requests, count) = self.requests();
        let requests = &requests[..count];
        let operation = OperationContext::new();
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
            let operation = OperationContext::new();
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
    request: backend::SelectionRequest,
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
        let operation = OperationContext::new();
        let started = ClockStart::now();
        let result = backend::select_scores(&mut self.scratch, self.request, &operation);
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

fn observe(row: Workload, smoke: bool) -> Value {
    let (warmups, count) = row.plan(smoke);
    let mut warmup_samples = Vec::with_capacity(warmups);
    let mut samples = Vec::with_capacity(count);
    let before = Memory::observe();
    reset_allocation_peak();
    let allocations_before = ALLOCATIONS.load(Ordering::Relaxed);
    let work_before = backend::work_snapshot();
    let backend_before = backend::snapshot();
    let mut steady_before = before;
    let mut setup_error = None;
    let mut selection = matches!(row.kind, Kind::Selection).then(|| SelectionFixture::new(row));
    let is_watch = matches!(row.kind, Kind::WatchAnalysisAlways | Kind::WatchExactRgba);
    let mut search = if selection.is_none() && !is_watch {
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
    if setup_error.is_none() {
        for iteration in 0..warmups + count {
            let sample = if let Some(fixture) = &mut selection {
                fixture.run()
            } else if is_watch {
                let started = ClockStart::now();
                let result = watch::run(matches!(row.kind, Kind::WatchAnalysisAlways));
                let (elapsed_ns, cpu_ns) = started.finish();
                Sample {
                    elapsed_ns,
                    cpu_ns,
                    correct: result.as_ref().is_ok_and(|correct| *correct),
                    error: result.err(),
                }
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
                steady_before = Memory::observe();
            }
        }
    }
    let after = Memory::observe();
    let work_after = backend::work_snapshot();
    let activity = backend::snapshot().checked_delta(backend_before);
    let mapping_owner_live = search
        .as_ref()
        .is_some_and(|fixture| fixture.mapping_owner.is_some());
    drop(selection.take());
    drop(search.take());
    let after_drop = Memory::observe();
    let heap_peak = peak_allocated_bytes() as u64;
    let allocations = ALLOCATIONS
        .load(Ordering::Relaxed)
        .saturating_sub(allocations_before);
    let final_work = backend::work_snapshot();
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
    let lifecycle_correct = final_work.bgr_retained_bytes == work_before.bgr_retained_bytes
        && work_after.bgr_retained_bytes <= 33_554_432
        && activity.is_some_and(|value| {
            value.active_finds == 0
                && (!matches!(row.kind, Kind::Concurrent) || value.peak_active_finds >= 2)
        });
    let oracle_failures = warmup_samples
        .iter()
        .chain(&samples)
        .filter(|sample| !sample.correct)
        .count()
        + usize::from(setup_error.is_some())
        + usize::from(!lifecycle_correct);
    let mut report = row.manifest(smoke);
    let object = report.as_object_mut().expect("manifest object");
    object.insert("samples".into(), json!(samples));
    object.insert("warmup_samples".into(), json!(warmup_samples));
    object.insert("oracle_failures".into(), json!(oracle_failures));
    object.insert("setup_error".into(), json!(setup_error));
    let mut metrics = json!({
        "iterations": warmups + count, "cpu_ns": cpu_ns,
        "cpu_reason": if cpu_missing { Some("process CPU observation unavailable or nonmonotonic") } else { None },
        "native_resource_reason": if native_missing { Some("self-process resident/high-water observation unavailable") } else { None },
        "peak_rss_bytes": after_drop.peak_rss, "rss_before_bytes": before.rss,
        "rss_after_bytes": after.rss, "rss_after_drop_bytes": after_drop.rss,
        "rss_steady_before_bytes": steady_before.rss, "rss_steady_after_bytes": after.rss,
        "heap_before_bytes": before.heap, "heap_after_bytes": after.heap,
        "heap_after_drop_bytes": after_drop.heap, "peak_heap_bytes": heap_peak,
        "heap_steady_before_bytes": steady_before.heap, "heap_steady_after_bytes": after.heap,
        "private_after_bytes": after.private, "footprint_after_bytes": after.footprint,
        "private_reason": if after.private.is_none() { Some("not exposed by this target resource helper") } else { None },
        "footprint_reason": if after.footprint.is_none() { Some("not exposed by this target resource helper") } else { None },
        "native_allocation_scope": "cache-payload gauges and process RSS, not total native allocation tracking",
        "peak_rss_scope": "process-high-water", "peak_heap_scope": "row-including-setup-and-warmup",
        "allocations": allocations,
        "score_copied_bytes": work_after.score_copied_bytes.saturating_sub(work_before.score_copied_bytes),
        "score_offsets_scanned": work_after.score_offsets_scanned.saturating_sub(work_before.score_offsets_scanned),
        "bgr_conversions": work_after.bgr_conversions.saturating_sub(work_before.bgr_conversions),
        "bgr_converted_bytes": work_after.bgr_converted_bytes.saturating_sub(work_before.bgr_converted_bytes),
        "bgr_retained_bytes": work_after.bgr_retained_bytes,
        "bgr_retained_peak_bytes": work_after.bgr_retained_peak_bytes,
        "bgr_retained_after_drop_bytes": final_work.bgr_retained_bytes,
        "mapping_copies": work_after.mapping_copies.saturating_sub(work_before.mapping_copies),
        "mapping_copied_bytes": work_after.mapping_copied_bytes.saturating_sub(work_before.mapping_copied_bytes),
        "logical_mapped_bytes": activity.map(|value| value.mapped_bytes),
        "backend_find_calls": activity.map(|value| value.find_calls),
        "backend_find_failures": activity.map(|value| value.find_failures),
        "peak_active_finds": activity.map(|value| value.peak_active_finds),
        "peak_active_finds_scope": "process-high-water",
        "mapping_owner_live_before_drop": mapping_owner_live,
        "final_owner_correct": lifecycle_correct,
    });
    for (key, value) in [
        ("bgr_evicted_bytes", work_after.bgr_evicted_bytes),
        ("bgr_evicted_peak_bytes", work_after.bgr_evicted_peak_bytes),
        ("bgr_evicted_after_drop_bytes", final_work.bgr_evicted_bytes),
    ] {
        metrics[key] = Value::from(value);
    }
    object.insert("metrics".into(), metrics);
    report
}

struct Arguments {
    list: bool,
    smoke: bool,
    workload: Option<String>,
}

fn arguments() -> Result<Arguments, String> {
    let mut parsed = Arguments {
        list: false,
        smoke: false,
        workload: None,
    };
    let mut values = std::env::args().skip(1);
    while let Some(value) = values.next() {
        match value.as_str() {
            "--list" => parsed.list = true,
            "--smoke" => parsed.smoke = true,
            "--bench" => {}
            "--workload" => {
                if parsed.workload.is_some() {
                    return Err("--workload supplied twice".into());
                }
                parsed.workload = Some(values.next().ok_or("--workload requires an ID")?);
            }
            _ => return Err(format!("unsupported argument: {value}")),
        }
    }
    if parsed
        .workload
        .as_ref()
        .is_some_and(|id| !WORKLOADS.iter().any(|row| row.id == id))
    {
        return Err(format!(
            "unknown workload: {}",
            parsed.workload.as_deref().unwrap_or_default()
        ));
    }
    Ok(parsed)
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

fn run(arguments: Arguments) -> Result<Value, String> {
    let backend = OpenCvBackend::new().map_err(|error| error.to_string())?;
    let version = backend.descriptor().version().to_owned();
    drop(backend);
    let (timer_resolution_ns, cpu_timer_resolution_ns) = resolutions();
    let workloads: Vec<_> = WORKLOADS
        .iter()
        .filter(|row| arguments.workload.as_deref().is_none_or(|id| row.id == id))
        .map(|&row| {
            if arguments.list {
                row.manifest(arguments.smoke)
            } else {
                observe(row, arguments.smoke)
            }
        })
        .collect();
    // Corpus identity generation is outside every measured resource interval.
    let (fixture_sha256, fixture) = fixtures::manifest();
    Ok(json!({
        "format_version": 1, "kind": if arguments.list { "manifest" } else { "observations" },
        "target": target(), "opencv_version": version, "fixture_sha256": fixture_sha256,
        "fixture": fixture, "warmups": if arguments.smoke { 1 } else { 20 },
        "sample_count": if arguments.smoke { 3 } else { 200 },
        "timer_resolution_ns": timer_resolution_ns, "cpu_timer_resolution_ns": cpu_timer_resolution_ns,
        "timer_resolution_method": "minimum-observed-positive-Instant-delta",
        "cpu_resolution_method": "minimum-observed-positive-process-cpu-delta",
        "cpu_clock_unit_ns": if cfg!(windows) { Some(100) } else if cfg!(target_os = "macos") { Some(1000) } else { None },
        "cpu_timer_resolution_source": "bounded self-process CPU probe; observed deltas are not guaranteed accounting accuracy",
        "timer_resolution_source": "minimum positive Instant delta probe",
        "workloads": workloads,
    }))
}

fn main() {
    let (report, failed) = match arguments().and_then(run) {
        Ok(report) => {
            let failed = report["workloads"].as_array().is_some_and(|rows| {
                rows.iter().any(|row| {
                    row["oracle_failures"]
                        .as_u64()
                        .is_some_and(|count| count != 0)
                })
            });
            (report, failed)
        }
        Err(error) => (
            json!({"format_version": 1, "kind": "observations", "target": target(),
            "error": error, "workloads": []}),
            true,
        ),
    };
    println!(
        "{}",
        serde_json::to_string(&report).expect("finite benchmark JSON")
    );
    if failed {
        std::process::exit(1);
    }
}
