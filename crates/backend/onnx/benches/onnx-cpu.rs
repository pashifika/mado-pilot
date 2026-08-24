//! Focused cold-load, warm-inference, allocation-growth, and cleanup observations.

#[cfg(windows)]
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
#[cfg(any(all(target_arch = "aarch64", target_os = "macos"), windows))]
use std::time::Duration;
use std::time::Instant;

use mado_pilot_backend_onnx::{OnnxBackendFacts, OnnxBackendObservations, OnnxOcrBackend};
use mado_pilot_capture::{Frame, PixelFormat};
use mado_pilot_core::{
    ClipPolicy, CoordinateSpace, OperationContext, PixelExtent, PixelRect, Rect,
};
use mado_pilot_ocr::{OcrBackend, OcrRecognizer, OcrRegion, OcrRequest, OcrResult};
use mado_pilot_testkit::bench_harness::{self, Accounting, Plan, Sample};
#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
use mado_pilot_testkit::bench_harness::{
    PHASE3_APPLE_OCR_CLOSE_LIMIT, PHASE3_APPLE_OCR_COLD_LOAD_LIMIT,
    PHASE3_APPLE_OCR_HEAP_LIMIT_BYTES, PHASE3_APPLE_OCR_LATENCY_BUDGETS,
    PHASE3_APPLE_OCR_REOPEN_CLOSE_LIMIT, PHASE3_APPLE_OCR_RESIDENT_LIMIT_BYTES,
};
#[cfg(any(all(target_arch = "aarch64", target_os = "macos"), windows))]
use mado_pilot_testkit::bench_harness::{
    PHASE3_OCR_EMPTY_MAPPED_BYTES, PHASE3_OCR_FULL_MAPPED_BYTES, PHASE3_OCR_MAX_OUTPUT_BYTES,
    PHASE3_OCR_MAX_TENSOR_BYTES, PHASE3_OCR_REGION_MAPPED_BYTES,
};
#[cfg(windows)]
use mado_pilot_testkit::bench_harness::{
    PHASE3_WINDOWS_OCR_CLOSE_LIMIT, PHASE3_WINDOWS_OCR_COLD_LOAD_LIMIT,
    PHASE3_WINDOWS_OCR_HEAP_LIMIT_BYTES, PHASE3_WINDOWS_OCR_LATENCY_BUDGETS,
    PHASE3_WINDOWS_OCR_REOPEN_CLOSE_LIMIT, PHASE3_WINDOWS_OCR_RESIDENT_LIMIT_BYTES,
};
use mado_pilot_testkit::vision_contract;
use opencv::core::{Mat, MatTraitConst, MatTraitConstManual};
use opencv::imgcodecs::{IMREAD_COLOR, imread};
use opencv::imgproc::{COLOR_BGR2BGRA, cvt_color_def};
#[cfg(windows)]
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
#[cfg(windows)]
use windows::Win32::System::Threading::GetCurrentProcess;

#[global_allocator]
static ACCOUNTING: Accounting = Accounting;

const RUNTIME_ENV: &str = "MADO_PILOT_ONNX_RUNTIME";
const MODEL_ROOT_ENV: &str = "MADO_PILOT_G004_MODEL_ROOT";
const FULL_NAME: &str = "onnx_cpu_hud_full";
const REGION_NAME: &str = "onnx_cpu_hud_region";
const EMPTY_NAME: &str = "onnx_cpu_blank";

struct ExpectedRegion {
    text: &'static str,
    quad: [(f64, f64); 4],
}

static HUD_EXPECTED: [ExpectedRegion; 8] = [
    ExpectedRegion {
        text: "魔導士",
        quad: [(53.0, 60.0), (203.0, 60.0), (203.0, 112.0), (53.0, 112.0)],
    },
    ExpectedRegion {
        text: "Lv.42",
        quad: [(705.0, 67.0), (811.0, 67.0), (811.0, 107.0), (705.0, 107.0)],
    },
    ExpectedRegion {
        text: "HP1234/5678",
        quad: [(53.0, 174.0), (319.0, 174.0), (319.0, 222.0), (53.0, 222.0)],
    },
    ExpectedRegion {
        text: "MP98%",
        quad: [
            (717.0, 180.0),
            (855.0, 180.0),
            (855.0, 214.0),
            (717.0, 214.0),
        ],
    },
    ExpectedRegion {
        text: "クエスト",
        quad: [(53.0, 291.0), (235.0, 291.0), (235.0, 335.0), (53.0, 335.0)],
    },
    ExpectedRegion {
        text: "[A-7]",
        quad: [
            (735.0, 297.0),
            (820.0, 297.0),
            (820.0, 338.0),
            (735.0, 338.0),
        ],
    },
    ExpectedRegion {
        text: "次へ>",
        quad: [(53.0, 412.0), (166.0, 412.0), (166.0, 458.0), (53.0, 458.0)],
    },
    ExpectedRegion {
        text: "READY!",
        quad: [
            (693.0, 421.0),
            (813.0, 421.0),
            (813.0, 453.0),
            (693.0, 453.0),
        ],
    },
];

fn main() {
    if [RUNTIME_ENV, MODEL_ROOT_ENV]
        .into_iter()
        .any(|variable| std::env::var_os(variable).is_none())
    {
        eprintln!("onnx-cpu benchmark skipped: set the two reviewed MADO_PILOT_* paths");
        return;
    }
    let runtime = required_path(RUNTIME_ENV);
    let model_root = required_path(MODEL_ROOT_ENV);
    let operation = OperationContext::new();
    // `cargo test --all-targets` executes this custom benchmark in the debug
    // profile. It enforces host-independent oracles/growth only; target timing
    // belongs exclusively to an optimized qualification process.
    let smoke = cfg!(debug_assertions) || std::env::args().any(|argument| argument == "--smoke");
    let plan = if smoke {
        Plan::smoke()
    } else {
        Plan::new(3, 20)
    };

    let cold_started = Instant::now();
    let backend = Arc::new(
        OnnxOcrBackend::open_accepted(&model_root, &runtime, &operation)
            .expect("accepted default backend cold-opens"),
    );
    let cold_load_ms = cold_started.elapsed().as_secs_f64() * 1_000.0;
    let facts = backend.facts();
    let erased: Arc<dyn OcrBackend> = backend.clone();
    let recognizer = OcrRecognizer::new(erased);
    let hud = hud_frame();
    let blank = blank_frame();

    let full = bench_harness::measure(
        FULL_NAME,
        "exact HUD text, geometry, order, confidence, and source correlation",
        plan,
        || {
            Fixture::new(
                recognizer.clone(),
                Arc::clone(&backend),
                hud.clone(),
                OcrRegion::FullFrame,
                &HUD_EXPECTED,
                PixelRect::new(0, 0, 960, 540).expect("valid full HUD region"),
                960 * 540 * 4,
            )
        },
        recognize,
    );
    let region = bench_harness::measure(
        REGION_NAME,
        "exact bounded HUD text, geometry, confidence, and source correlation",
        plan,
        || {
            Fixture::new(
                recognizer.clone(),
                Arc::clone(&backend),
                hud.clone(),
                OcrRegion::Region {
                    rect: Rect::new(CoordinateSpace::CapturePixels, 40.0, 40.0, 220.0, 130.0)
                        .expect("valid bounded HUD region"),
                    policy: ClipPolicy::Reject,
                },
                &HUD_EXPECTED[..1],
                PixelRect::new(40, 40, 220, 130).expect("valid effective HUD region"),
                180 * 90 * 4,
            )
        },
        recognize,
    );
    let empty = bench_harness::measure(
        EMPTY_NAME,
        "empty result with exact source correlation",
        plan,
        || {
            Fixture::new(
                recognizer.clone(),
                Arc::clone(&backend),
                blank,
                OcrRegion::FullFrame,
                &[],
                PixelRect::new(0, 0, 64, 64).expect("valid full blank region"),
                64 * 64 * 4,
            )
        },
        recognize,
    );
    let workloads = [full, region, empty];

    let close_started = Instant::now();
    backend
        .close(&operation)
        .expect("backend closes after samples");
    let close_ms = close_started.elapsed().as_secs_f64() * 1_000.0;
    let mut reopen_close_ms = Vec::with_capacity(3);
    for _ in 0..3 {
        let started = Instant::now();
        let reopened = OnnxOcrBackend::open_accepted(&model_root, &runtime, &operation)
            .expect("accepted default session pair reopens");
        reopened.close(&operation).expect("reopened pair closes");
        reopen_close_ms.push(started.elapsed().as_secs_f64() * 1_000.0);
    }

    let peak_resident = peak_resident_bytes();
    print_observation(
        plan,
        cold_load_ms,
        close_ms,
        &reopen_close_ms,
        peak_resident,
        facts,
        &workloads,
    );
    bench_harness::enforce_hard_budgets(&workloads);
    if !smoke {
        enforce_target_budgets(
            cold_load_ms,
            close_ms,
            &reopen_close_ms,
            peak_resident,
            facts,
            &workloads,
        );
    }
}

struct Fixture {
    recognizer: OcrRecognizer,
    backend: Arc<OnnxOcrBackend>,
    frame: Frame,
    region: OcrRegion,
    expected: &'static [ExpectedRegion],
    effective_region: PixelRect,
    mapped_bytes: u64,
    confidence: Mutex<Option<Vec<f64>>>,
}

impl Fixture {
    fn new(
        recognizer: OcrRecognizer,
        backend: Arc<OnnxOcrBackend>,
        frame: Frame,
        region: OcrRegion,
        expected: &'static [ExpectedRegion],
        effective_region: PixelRect,
        mapped_bytes: u64,
    ) -> Self {
        Self {
            recognizer,
            backend,
            frame,
            region,
            expected,
            effective_region,
            mapped_bytes,
            confidence: Mutex::new(None),
        }
    }
}

fn recognize(fixture: &Fixture) -> Sample {
    let descriptor = fixture.recognizer.descriptor();
    let before = fixture
        .backend
        .observations()
        .expect("benchmark observes an idle open backend");
    let started = Instant::now();
    let result = fixture.recognizer.recognize(OcrRequest::new(
        &fixture.frame,
        descriptor.backend_identity(),
        descriptor.model_identity(),
        fixture.region,
        CoordinateSpace::CapturePixels,
        &OperationContext::new(),
    ));
    let elapsed = started.elapsed();
    let after = fixture
        .backend
        .observations()
        .expect("completed inference returns the session pair");
    let resources = observation_delta(before, after);
    let expected_recognizer_runs = u64::try_from(fixture.expected.len().div_ceil(6))
        .expect("the fixture candidate count fits u64");
    let correct = result.as_ref().is_ok_and(|result| oracle(fixture, result))
        && resources.mapped_bytes == fixture.mapped_bytes
        && resources.detector_runs == 1
        && resources.recognizer_runs == expected_recognizer_runs
        && after.session_pairs() == 1
        && after.sessions() == 2;
    if !correct {
        eprintln!(
            "OCR benchmark rejected result/resources: result={result:#?} resources={resources:?}"
        );
    }
    assert!(correct, "OCR benchmark correctness/resource oracle failed");
    Sample::new(elapsed, correct, resources.mapped_bytes)
}

#[derive(Debug)]
struct ObservationDelta {
    mapped_bytes: u64,
    detector_runs: u64,
    recognizer_runs: u64,
}

fn observation_delta(
    before: OnnxBackendObservations,
    after: OnnxBackendObservations,
) -> ObservationDelta {
    ObservationDelta {
        mapped_bytes: after
            .mapped_bytes()
            .checked_sub(before.mapped_bytes())
            .expect("mapped-byte observations are monotonic"),
        detector_runs: after
            .detector_runs()
            .checked_sub(before.detector_runs())
            .expect("detector-run observations are monotonic"),
        recognizer_runs: after
            .recognizer_runs()
            .checked_sub(before.recognizer_runs())
            .expect("recognizer-run observations are monotonic"),
    }
}

fn oracle(fixture: &Fixture, result: &OcrResult) -> bool {
    if result.stamp() != fixture.frame.stamp()
        || result.effective_region() != fixture.effective_region
        || result.output_space() != CoordinateSpace::CapturePixels
        || result.backend() != &fixture.recognizer.descriptor()
        || result.regions().len() != fixture.expected.len()
    {
        return false;
    }
    let mut confidences = Vec::with_capacity(result.regions().len());
    for (region, expected) in result.regions().iter().zip(fixture.expected) {
        if region.text() != expected.text {
            return false;
        }
        if !geometry_matches(region.geometry().points(), expected.quad) {
            return false;
        }
        let confidence = region.confidence().get();
        if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
            return false;
        }
        confidences.push(confidence);
    }
    let mut baseline = fixture
        .confidence
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match baseline.as_ref() {
        Some(baseline) => baseline == &confidences,
        None => {
            *baseline = Some(confidences);
            true
        }
    }
}

fn geometry_matches(actual: [mado_pilot_core::Point; 4], expected: [(f64, f64); 4]) -> bool {
    let actual_bounds = bounds(actual.map(|point| (point.x(), point.y())));
    let expected_bounds = bounds(expected);
    let intersection_width =
        (actual_bounds.2.min(expected_bounds.2) - actual_bounds.0.max(expected_bounds.0)).max(0.0);
    let intersection_height =
        (actual_bounds.3.min(expected_bounds.3) - actual_bounds.1.max(expected_bounds.1)).max(0.0);
    let intersection = intersection_width * intersection_height;
    let actual_area = (actual_bounds.2 - actual_bounds.0) * (actual_bounds.3 - actual_bounds.1);
    let expected_area =
        (expected_bounds.2 - expected_bounds.0) * (expected_bounds.3 - expected_bounds.1);
    let union = actual_area + expected_area - intersection;
    let iou = intersection / union;
    let center_delta_x =
        ((actual_bounds.0 + actual_bounds.2) - (expected_bounds.0 + expected_bounds.2)).abs()
            / (2.0 * 960.0);
    let center_delta_y =
        ((actual_bounds.1 + actual_bounds.3) - (expected_bounds.1 + expected_bounds.3)).abs()
            / (2.0 * 540.0);
    let points_within_profile = actual.into_iter().zip(expected).all(|(actual, expected)| {
        (actual.x() - expected.0).abs() / 960.0 <= 0.025
            && (actual.y() - expected.1).abs() / 540.0 <= 0.025
    });

    iou >= 0.5 && center_delta_x <= 0.025 && center_delta_y <= 0.025 && points_within_profile
}

fn bounds(points: [(f64, f64); 4]) -> (f64, f64, f64, f64) {
    points.into_iter().fold(
        (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ),
        |(left, top, right, bottom), (x, y)| (left.min(x), top.min(y), right.max(x), bottom.max(y)),
    )
}

fn print_observation(
    plan: Plan,
    cold_load_ms: f64,
    close_ms: f64,
    reopen_close_ms: &[f64],
    peak_resident: Option<u64>,
    facts: OnnxBackendFacts,
    workloads: &[bench_harness::Workload],
) {
    println!(
        concat!(
            "onnx-cpu-startup ",
            "cold_load_ms={cold_load_ms:.3} close_ms={close_ms:.3} ",
            "reopen_close_p95_ms={reopen_close_p95_ms:.3} ",
            "reopen_close_max_ms={reopen_close_max_ms:.3} ",
            "configured_session_pairs=1 configured_sessions=2 ",
            "max_concurrency={max_concurrency} max_tensor_bytes={max_tensor_bytes} ",
            "max_output_bytes={max_output_bytes} recognition_batch={recognition_batch}"
        ),
        cold_load_ms = cold_load_ms,
        close_ms = close_ms,
        reopen_close_p95_ms = percentile(reopen_close_ms, 0.95),
        reopen_close_max_ms = percentile(reopen_close_ms, 1.0),
        max_concurrency = facts.max_concurrent_inferences(),
        max_tensor_bytes = facts.max_tensor_bytes(),
        max_output_bytes = facts.max_output_bytes(),
        recognition_batch = facts.recognition_batch(),
    );
    match peak_resident {
        Some(bytes) => println!("onnx-cpu-resident peak_resident_bytes={bytes}"),
        None => println!("onnx-cpu-resident peak_resident_bytes=unavailable"),
    }
    for workload in workloads {
        let (detector_tensors, recognizer_tensors, result_regions) = match workload.name() {
            FULL_NAME => (1, 2, HUD_EXPECTED.len()),
            REGION_NAME => (1, 1, 1),
            EMPTY_NAME => (1, 0, 0),
            name => panic!("unrecognized OCR workload {name}"),
        };
        println!(
            concat!(
                "onnx-cpu-workload name={name} warmups={warmups} samples={samples} ",
                "p50_ms={p50_ms:.3} p95_ms={p95_ms:.3} max_ms={max_ms:.3} ",
                "incorrect={incorrect} mapped_bytes={mapped_bytes} ",
                "producer_copy_bytes=not_applicable ",
                "rust_peak_allocated_bytes={peak_bytes} rust_growth_bytes={growth_bytes} ",
                "detector_tensor_runs={detector_tensors} ",
                "recognizer_tensor_runs={recognizer_tensors} result_regions={result_regions}"
            ),
            name = workload.name(),
            warmups = plan.warmup(),
            samples = plan.samples(),
            p50_ms = workload.percentile(0.50),
            p95_ms = workload.percentile(0.95),
            max_ms = workload.max_elapsed().as_secs_f64() * 1_000.0,
            incorrect = workload.incorrect(),
            mapped_bytes = workload.mapped_bytes_per_result(),
            peak_bytes = workload.peak_allocated_bytes(),
            growth_bytes = workload.growth_bytes(),
            detector_tensors = detector_tensors,
            recognizer_tensors = recognizer_tensors,
            result_regions = result_regions,
        );
    }
}

#[cfg(any(all(target_arch = "aarch64", target_os = "macos"), windows))]
struct TargetBudgets {
    target: &'static str,
    label: &'static str,
    resident_name: &'static str,
    latency: &'static [bench_harness::LatencyBudget],
    cold_load: Duration,
    close: Duration,
    reopen_close: Duration,
    heap_bytes: usize,
    resident_bytes: u64,
}

#[cfg(any(all(target_arch = "aarch64", target_os = "macos"), windows))]
fn enforce_target_budget_set(
    budgets: TargetBudgets,
    cold_load_ms: f64,
    close_ms: f64,
    reopen_close_ms: &[f64],
    peak_resident: Option<u64>,
    facts: OnnxBackendFacts,
    workloads: &[bench_harness::Workload],
) {
    bench_harness::enforce_latency_budgets(workloads, budgets.latency);
    assert!(
        cold_load_ms <= budgets.cold_load.as_secs_f64() * 1_000.0,
        "cold default OCR startup exceeded the {} ceiling",
        budgets.label
    );
    assert!(
        close_ms <= budgets.close.as_secs_f64() * 1_000.0,
        "OCR close exceeded the {} ceiling",
        budgets.label
    );
    assert!(
        percentile(reopen_close_ms, 1.0) <= budgets.reopen_close.as_secs_f64() * 1_000.0,
        "OCR reopen-close exceeded the {} ceiling",
        budgets.label
    );
    assert_eq!(facts.max_concurrent_inferences(), 1);
    assert_eq!(facts.max_tensor_bytes(), PHASE3_OCR_MAX_TENSOR_BYTES);
    assert_eq!(facts.max_output_bytes(), PHASE3_OCR_MAX_OUTPUT_BYTES);
    assert_eq!(facts.recognition_batch(), 6);
    bench_harness::nonzero_at_most(budgets.resident_name, peak_resident, budgets.resident_bytes);

    for workload in workloads {
        assert!(
            workload.peak_allocated_bytes() <= budgets.heap_bytes,
            "{} exceeded the {} live-Rust-heap ceiling",
            workload.name(),
            budgets.label
        );
        let expected_mapped = match workload.name() {
            FULL_NAME => PHASE3_OCR_FULL_MAPPED_BYTES,
            REGION_NAME => PHASE3_OCR_REGION_MAPPED_BYTES,
            EMPTY_NAME => PHASE3_OCR_EMPTY_MAPPED_BYTES,
            name => panic!("unrecognized OCR workload {name}"),
        };
        assert_eq!(
            workload.mapped_bytes_per_result(),
            expected_mapped,
            "{} changed its exact mapped-byte cost",
            workload.name()
        );
    }
    println!(
        "onnx-cpu-target-budgets target={} status=passed",
        budgets.target
    );
}

#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
fn enforce_target_budgets(
    cold_load_ms: f64,
    close_ms: f64,
    reopen_close_ms: &[f64],
    peak_resident: Option<u64>,
    facts: OnnxBackendFacts,
    workloads: &[bench_harness::Workload],
) {
    enforce_target_budget_set(
        TargetBudgets {
            target: "aarch64-apple-darwin",
            label: "Apple Silicon",
            resident_name: "Apple Silicon OCR peak resident bytes",
            latency: &PHASE3_APPLE_OCR_LATENCY_BUDGETS,
            cold_load: PHASE3_APPLE_OCR_COLD_LOAD_LIMIT,
            close: PHASE3_APPLE_OCR_CLOSE_LIMIT,
            reopen_close: PHASE3_APPLE_OCR_REOPEN_CLOSE_LIMIT,
            heap_bytes: PHASE3_APPLE_OCR_HEAP_LIMIT_BYTES,
            resident_bytes: PHASE3_APPLE_OCR_RESIDENT_LIMIT_BYTES,
        },
        cold_load_ms,
        close_ms,
        reopen_close_ms,
        peak_resident,
        facts,
        workloads,
    );
}

#[cfg(windows)]
fn enforce_target_budgets(
    cold_load_ms: f64,
    close_ms: f64,
    reopen_close_ms: &[f64],
    peak_resident: Option<u64>,
    facts: OnnxBackendFacts,
    workloads: &[bench_harness::Workload],
) {
    enforce_target_budget_set(
        TargetBudgets {
            target: "x86_64-pc-windows-msvc",
            label: "Windows",
            resident_name: "Windows OCR peak resident bytes",
            latency: &PHASE3_WINDOWS_OCR_LATENCY_BUDGETS,
            cold_load: PHASE3_WINDOWS_OCR_COLD_LOAD_LIMIT,
            close: PHASE3_WINDOWS_OCR_CLOSE_LIMIT,
            reopen_close: PHASE3_WINDOWS_OCR_REOPEN_CLOSE_LIMIT,
            heap_bytes: PHASE3_WINDOWS_OCR_HEAP_LIMIT_BYTES,
            resident_bytes: PHASE3_WINDOWS_OCR_RESIDENT_LIMIT_BYTES,
        },
        cold_load_ms,
        close_ms,
        reopen_close_ms,
        peak_resident,
        facts,
        workloads,
    );
}

#[cfg(not(any(all(target_arch = "aarch64", target_os = "macos"), windows)))]
fn enforce_target_budgets(
    _cold_load_ms: f64,
    _close_ms: f64,
    _reopen_close_ms: &[f64],
    _peak_resident: Option<u64>,
    _facts: OnnxBackendFacts,
    _workloads: &[bench_harness::Workload],
) {
    println!(
        "onnx-cpu-target-budgets target={} status=withheld",
        bench_harness::RELEASE_TARGET
    );
}

#[cfg(target_os = "macos")]
fn peak_resident_bytes() -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: `usage` is writable for the complete target `rusage` structure;
    // a zero return initializes it before `assume_init`.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: successful `getrusage` initialized the complete structure.
    let usage = unsafe { usage.assume_init() };
    u64::try_from(usage.ru_maxrss)
        .ok()
        .filter(|bytes| *bytes > 0)
}

#[cfg(windows)]
fn peak_resident_bytes() -> Option<u64> {
    let mut counters = PROCESS_MEMORY_COUNTERS::default();
    let bytes = u32::try_from(size_of::<PROCESS_MEMORY_COUNTERS>()).ok()?;
    counters.cb = bytes;
    // SAFETY: the pseudo handle names this process and `counters` is writable
    // for the complete native structure declared by `bytes`.
    unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &raw mut counters, bytes) }.ok()?;
    u64::try_from(counters.PeakWorkingSetSize)
        .ok()
        .filter(|bytes| *bytes > 0)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn peak_resident_bytes() -> Option<u64> {
    None
}

fn percentile(samples: &[f64], percentile: f64) -> f64 {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let last = sorted.len().checked_sub(1).expect("at least one sample");
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the clamped nearest rank indexes this three-sample vector"
    )]
    let index = ((percentile.clamp(0.0, 1.0) * sorted.len() as f64)
        .ceil()
        .max(1.0) as usize
        - 1)
    .min(last);
    sorted[index]
}

fn hud_frame() -> Frame {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/ocr/g-004/hud.png");
    let bgr = imread(path.to_str().expect("UTF-8 fixture path"), IMREAD_COLOR)
        .expect("decode tracked HUD fixture");
    let mut bgra = Mat::default();
    cvt_color_def(&bgr, &mut bgra, COLOR_BGR2BGRA).expect("convert fixture to BGRA");
    assert!(bgra.is_continuous());
    let width = u32::try_from(bgra.cols()).expect("fixture width");
    let height = u32::try_from(bgra.rows()).expect("fixture height");
    vision_contract::frame_with_pixels(
        PixelExtent::new(width, height),
        PixelFormat::Bgra8,
        bgra.data_bytes().expect("fixture bytes").to_vec(),
    )
}

fn blank_frame() -> Frame {
    vision_contract::frame_with_pixels(
        PixelExtent::new(64, 64),
        PixelFormat::Bgra8,
        vec![0; 64 * 64 * 4],
    )
}

fn required_path(variable: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(variable).expect("benchmark path is configured"))
        .canonicalize()
        .expect("benchmark path is canonicalizable")
}
