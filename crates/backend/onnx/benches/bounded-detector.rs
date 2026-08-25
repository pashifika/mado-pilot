//! Revision-bound quality and resource qualification for ADR 0038.

#[cfg(windows)]
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mado_pilot_backend_onnx::{
    OnnxBackendObservations, OnnxOcrBackend, OnnxOcrProfile, RUNTIME_PROFILE_ID,
};
use mado_pilot_capture::{Frame, PixelFormat};
use mado_pilot_core::{
    CancellationToken, ClipPolicy, CoordinateSpace, OperationContext, PixelExtent, PixelRect, Rect,
    Status,
};
use mado_pilot_ocr::{OcrBackend, OcrRecognizer, OcrRegion, OcrRequest, OcrResult};
use mado_pilot_testkit::bench_harness::{self, Accounting, Plan, Sample};
use mado_pilot_testkit::vision_contract;
use opencv::core::{Mat, MatTraitConst, MatTraitConstManual, Size};
use opencv::imgcodecs::{IMREAD_COLOR, imread};
use opencv::imgproc::{COLOR_BGR2BGRA, INTER_LINEAR, INTER_NEAREST_EXACT, cvt_color_def, resize};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(windows)]
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
#[cfg(windows)]
use windows::Win32::System::Threading::GetCurrentProcess;

#[global_allocator]
static ACCOUNTING: Accounting = Accounting;

const RUNTIME_ENV: &str = "MADO_PILOT_ONNX_RUNTIME";
const MODEL_ROOT_ENV: &str = "MADO_PILOT_G004_MODEL_ROOT";
const SOURCE_ENV: &str = "MADO_PILOT_BOUNDED_SOURCE_REVISION";
const HOST_ENV: &str = "MADO_PILOT_BOUNDED_HOST_ID";
const PROCESS_ENV: &str = "MADO_PILOT_BOUNDED_PROCESS_INDEX";
const HEAP_GROWTH_LIMIT: i64 = 4_096;
const DETECTOR_SHA256: &str = "d2a7720d45a54257208b1e13e36a8479894cb74155a5efe29462512d42f49da9";
const RECOGNIZER_SHA256: &str = "6f327246b50388f3c176ae304bd95767ea6dc0c9ae92153ef8cbe210b3c14884";
const VOCABULARY_SHA256: &str = "f7aa897ca828a4c7c9e2739c30f9161a33306d532f020bcdb91dcfb664a5507e";

#[derive(Debug, Deserialize)]
struct FixtureManifest {
    images: Vec<FixtureImage>,
}

#[derive(Debug, Deserialize)]
struct FixtureImage {
    file: String,
    width: u32,
    height: u32,
    regions: Vec<FixtureRegion>,
}

#[derive(Debug, Deserialize)]
struct FixtureRegion {
    text_nfc: String,
    source_relative_quad: [[f64; 2]; 4],
}

#[derive(Debug, Clone)]
struct ExpectedRegion {
    text: String,
    quad: [(f64, f64); 4],
}

#[derive(Debug, Clone, Copy)]
struct DetectorDimensions {
    width: u32,
    height: u32,
}

#[derive(Debug, Clone)]
struct WorkloadSpec {
    name: &'static str,
    frame: Frame,
    region: OcrRegion,
    effective_region: PixelRect,
    expected: Arc<[ExpectedRegion]>,
    native_detector: DetectorDimensions,
    bounded_detector: DetectorDimensions,
}

impl WorkloadSpec {
    fn detector_dimensions(&self, profile: OnnxOcrProfile) -> DetectorDimensions {
        match profile {
            OnnxOcrProfile::NativeG004 => self.native_detector,
            OnnxOcrProfile::BoundedDetector => self.bounded_detector,
        }
    }

    fn mapped_bytes(&self) -> u64 {
        u64::from(self.effective_region.width()) * u64::from(self.effective_region.height()) * 4
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
struct ResourceSignature {
    mapped_bytes: u64,
    detector_width: u32,
    detector_height: u32,
    detector_tensor_bytes: u64,
    detector_resizes: u64,
    detector_runs: u64,
    recognizer_runs: u64,
}

#[derive(Debug, Default)]
struct MeasurementState {
    confidence: Option<Vec<f64>>,
    resources: Option<ResourceSignature>,
    calls: usize,
    failures: usize,
}

struct Fixture {
    recognizer: OcrRecognizer,
    backend: Arc<OnnxOcrBackend>,
    spec: WorkloadSpec,
    profile: OnnxOcrProfile,
    state: Arc<Mutex<MeasurementState>>,
}

#[derive(Debug, Serialize)]
struct WorkloadReport {
    name: &'static str,
    p50_ms: f64,
    p95_ms: f64,
    maximum_ms: f64,
    incorrect_retained: usize,
    all_call_failures: usize,
    call_count: usize,
    growth_bytes: i64,
    peak_allocated_bytes: usize,
    mapped_bytes: u64,
    peak_resident_bytes: Option<u64>,
    resources: Option<ResourceSignature>,
}

#[derive(Debug, Serialize)]
struct CancellationReport {
    status: String,
    elapsed_ms: f64,
    resources_unchanged: bool,
}

#[derive(Debug, Serialize)]
struct ProfileReport {
    schema_version: u32,
    release_target: &'static str,
    host_id: String,
    source_revision: String,
    process_index: String,
    executable_sha256: String,
    runtime_sha256: String,
    fixture_manifest_sha256: String,
    detector_sha256: &'static str,
    recognizer_sha256: &'static str,
    vocabulary_sha256: &'static str,
    runtime_profile: &'static str,
    profile: &'static str,
    model_id: String,
    profile_id: String,
    preprocessing_id: String,
    mode: &'static str,
    warmup_iterations: usize,
    retained_samples: usize,
    cold_open_ms: f64,
    first_close_ms: f64,
    reopen_close_ms: f64,
    peak_resident_bytes: Option<u64>,
    max_detector_width: Option<u32>,
    max_detector_height: Option<u32>,
    max_detector_tensor_bytes: u64,
    max_concurrent_inferences: u32,
    session_pairs: u32,
    sessions: u32,
    producer_surface_copy: &'static str,
    cancellation: CancellationReport,
    workloads: Vec<WorkloadReport>,
    passed: bool,
}

#[derive(Debug, Clone, Copy)]
struct TargetLimits {
    nonempty_p50_ms: f64,
    nonempty_p95_ms: f64,
    nonempty_max_ms: f64,
    empty_p50_ms: f64,
    empty_p95_ms: f64,
    empty_max_ms: f64,
    cold_open_ms: f64,
    first_close_ms: f64,
    reopen_close_ms: f64,
    resident_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunMode {
    Smoke,
    Precursor,
    EnforceBudgets,
}

impl RunMode {
    fn from_arguments(arguments: &[String]) -> Self {
        let smoke = arguments.iter().any(|argument| argument == "--smoke");
        let precursor = arguments.iter().any(|argument| argument == "--precursor");
        let enforce = arguments.iter().any(|argument| argument == "--qualify");
        assert!(
            usize::from(smoke) + usize::from(precursor) + usize::from(enforce) <= 1,
            "select only one benchmark mode"
        );
        if cfg!(debug_assertions) {
            assert!(
                !precursor && !enforce,
                "debug benchmark execution is smoke-only"
            );
            return Self::Smoke;
        }
        if smoke {
            Self::Smoke
        } else if enforce {
            Self::EnforceBudgets
        } else {
            Self::Precursor
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Smoke => "smoke",
            Self::Precursor => "precursor",
            Self::EnforceBudgets => "enforce-budgets",
        }
    }

    fn plan(self) -> Plan {
        match self {
            Self::Smoke => Plan::smoke(),
            Self::Precursor | Self::EnforceBudgets => Plan::new(3, 20),
        }
    }

    const fn requires_bound_identity(self) -> bool {
        !matches!(self, Self::Smoke)
    }

    const fn enforces_budgets(self) -> bool {
        matches!(self, Self::EnforceBudgets)
    }
}

fn main() {
    if [RUNTIME_ENV, MODEL_ROOT_ENV]
        .into_iter()
        .any(|variable| std::env::var_os(variable).is_none())
    {
        eprintln!("bounded-detector benchmark skipped: set the two reviewed MADO_PILOT_* paths");
        return;
    }

    let arguments = std::env::args().collect::<Vec<_>>();
    let mode = RunMode::from_arguments(&arguments);
    let profile = selected_profile(&arguments);
    let runtime = required_path(RUNTIME_ENV);
    let model_root = required_path(MODEL_ROOT_ENV);
    let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/ocr/g-004");
    let manifest_path = fixture_root.join("fixture-manifest.json");
    let manifest_bytes = std::fs::read(&manifest_path).expect("read tracked OCR fixture manifest");
    let manifest: FixtureManifest =
        serde_json::from_slice(&manifest_bytes).expect("parse tracked OCR fixture manifest");
    let workloads = build_workloads(&fixture_root, &manifest);

    let report = run_profile(
        profile,
        &model_root,
        &runtime,
        &workloads,
        mode,
        hex_digest(&manifest_bytes),
    );
    println!(
        "{}",
        serde_json::to_string(&report).expect("serialize bounded qualification report")
    );
    assert!(report.passed, "bounded detector qualification gate failed");
}

fn selected_profile(arguments: &[String]) -> OnnxOcrProfile {
    let selected = arguments
        .iter()
        .find_map(|argument| argument.strip_prefix("--profile="))
        .unwrap_or("bounded");
    match selected {
        "native" => OnnxOcrProfile::NativeG004,
        "bounded" => OnnxOcrProfile::BoundedDetector,
        other => panic!("unsupported qualification profile {other}"),
    }
}

fn run_profile(
    profile: OnnxOcrProfile,
    model_root: &Path,
    runtime: &Path,
    specs: &[WorkloadSpec],
    mode: RunMode,
    fixture_manifest_sha256: String,
) -> ProfileReport {
    let plan = mode.plan();
    if mode.requires_bound_identity() {
        assert!(
            target_limits().is_some(),
            "qualification requires an approved release target"
        );
        assert!(
            std::env::var_os(SOURCE_ENV).is_some(),
            "qualification requires exact source identity"
        );
        assert!(
            std::env::var_os(HOST_ENV).is_some(),
            "qualification requires approved host identity"
        );
        assert!(
            std::env::var_os(PROCESS_ENV).is_some(),
            "qualification requires process index"
        );
    }

    let operation = OperationContext::new();
    let cold_started = Instant::now();
    let backend = Arc::new(
        open_profile(profile, model_root, runtime, &operation)
            .expect("selected backend cold-opens"),
    );
    let cold_open_ms = milliseconds(cold_started.elapsed());
    let facts = backend.facts();
    assert_eq!(facts.profile(), profile);
    let descriptor = backend.descriptor();
    let port: Arc<dyn OcrBackend> = backend.clone();
    let recognizer = OcrRecognizer::new(port);

    let mut workload_reports = Vec::with_capacity(specs.len());
    for spec in specs {
        workload_reports.push(measure_workload(
            recognizer.clone(),
            Arc::clone(&backend),
            spec.clone(),
            profile,
            plan,
        ));
    }

    let cancellation = measure_cancelled(&recognizer, &backend, &specs[0]);
    let final_observations = backend
        .observations()
        .expect("qualification observes an idle session pair");
    let close_started = Instant::now();
    backend.close(&operation).expect("selected backend closes");
    let first_close_ms = milliseconds(close_started.elapsed());
    backend
        .close(&operation)
        .expect("selected backend close is idempotent");
    drop(recognizer);
    drop(backend);

    let reopen_started = Instant::now();
    let reopened = open_profile(profile, model_root, runtime, &operation)
        .expect("selected backend reopens after cleanup");
    reopened.close(&operation).expect("reopened backend closes");
    let reopen_close_ms = milliseconds(reopen_started.elapsed());
    let peak_resident_bytes = peak_resident_bytes();

    let identity = descriptor.model_identity();
    let mut report = ProfileReport {
        schema_version: 2,
        release_target: bench_harness::RELEASE_TARGET,
        host_id: environment_or(HOST_ENV, "unbound-smoke-host"),
        source_revision: environment_or(SOURCE_ENV, "unbound-smoke-source"),
        process_index: environment_or(PROCESS_ENV, "smoke"),
        executable_sha256: digest_file(
            &std::env::current_exe().expect("qualification executable path"),
        ),
        runtime_sha256: digest_file(runtime),
        fixture_manifest_sha256,
        detector_sha256: DETECTOR_SHA256,
        recognizer_sha256: RECOGNIZER_SHA256,
        vocabulary_sha256: VOCABULARY_SHA256,
        runtime_profile: RUNTIME_PROFILE_ID,
        profile: profile_name(profile),
        model_id: identity.model().as_str().to_owned(),
        profile_id: identity.profile().as_str().to_owned(),
        preprocessing_id: identity
            .profile_metadata()
            .preprocessing()
            .as_str()
            .to_owned(),
        mode: mode.name(),
        warmup_iterations: plan.warmup(),
        retained_samples: plan.samples(),
        cold_open_ms,
        first_close_ms,
        reopen_close_ms,
        peak_resident_bytes,
        max_detector_width: facts.max_detector_width(),
        max_detector_height: facts.max_detector_height(),
        max_detector_tensor_bytes: facts.max_detector_tensor_bytes(),
        max_concurrent_inferences: facts.max_concurrent_inferences(),
        session_pairs: final_observations.session_pairs(),
        sessions: final_observations.sessions(),
        producer_surface_copy: "not applicable: immutable CPU replay frames own no producer surface",
        cancellation,
        workloads: workload_reports,
        passed: false,
    };
    report.passed = report_passes(&report, profile, mode.enforces_budgets());
    report
}

fn measure_workload(
    recognizer: OcrRecognizer,
    backend: Arc<OnnxOcrBackend>,
    spec: WorkloadSpec,
    profile: OnnxOcrProfile,
    plan: Plan,
) -> WorkloadReport {
    let state = Arc::new(Mutex::new(MeasurementState::default()));
    let fixture_state = Arc::clone(&state);
    let fixture_spec = spec.clone();
    let workload = bench_harness::measure(
        spec.name,
        "exact text/count/order, manifest geometry, confidence, source identity, detector plan, and cleanup",
        plan,
        || Fixture {
            recognizer,
            backend,
            spec: fixture_spec,
            profile,
            state: fixture_state,
        },
        recognize,
    );
    let state = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    WorkloadReport {
        name: workload.name(),
        p50_ms: workload.percentile(0.50),
        p95_ms: workload.percentile(0.95),
        maximum_ms: milliseconds(workload.max_elapsed()),
        incorrect_retained: workload.incorrect(),
        all_call_failures: state.failures,
        call_count: state.calls,
        growth_bytes: workload.growth_bytes(),
        peak_allocated_bytes: workload.peak_allocated_bytes(),
        mapped_bytes: workload.mapped_bytes_per_result(),
        peak_resident_bytes: workload.peak_resident_bytes(),
        resources: state.resources,
    }
}

fn recognize(fixture: &Fixture) -> Sample {
    let descriptor = fixture.recognizer.descriptor();
    let before = fixture
        .backend
        .observations()
        .expect("qualification observes an idle backend before inference");
    let started = Instant::now();
    let result = fixture.recognizer.recognize(OcrRequest::new(
        &fixture.spec.frame,
        descriptor.backend_identity(),
        descriptor.model_identity(),
        fixture.spec.region,
        CoordinateSpace::CapturePixels,
        &OperationContext::new(),
    ));
    let elapsed = started.elapsed();
    let after = fixture
        .backend
        .observations()
        .expect("qualification observes an idle backend after inference");
    let resources = observation_delta(before, after);
    let expected_dimensions = fixture.spec.detector_dimensions(fixture.profile);
    let expected_recognizer_runs = u64::try_from(fixture.spec.expected.len().div_ceil(6))
        .expect("fixture result count fits u64");
    let resources_match = resources
        == ResourceSignature {
            mapped_bytes: fixture.spec.mapped_bytes(),
            detector_width: expected_dimensions.width,
            detector_height: expected_dimensions.height,
            detector_tensor_bytes: u64::from(expected_dimensions.width)
                * u64::from(expected_dimensions.height)
                * 3
                * 4,
            detector_resizes: 1,
            detector_runs: 1,
            recognizer_runs: expected_recognizer_runs,
        };

    let mut state = fixture
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let result_matches = result
        .as_ref()
        .is_ok_and(|result| quality_oracle(fixture, result, &mut state));
    let stable_resources = state.resources.is_none_or(|baseline| baseline == resources);
    state.resources.get_or_insert(resources);
    state.calls += 1;
    let correct = result_matches && resources_match && stable_resources;
    if !correct {
        state.failures += 1;
        eprintln!(
            "bounded-qualification-failure profile={} workload={} result={result:#?} resources={resources:?}",
            profile_name(fixture.profile),
            fixture.spec.name
        );
    }
    drop(state);

    let sample = Sample::new(elapsed, correct, resources.mapped_bytes);
    match peak_resident_bytes() {
        Some(bytes) => sample.with_peak_resident_bytes(bytes),
        None => sample,
    }
}

fn quality_oracle(fixture: &Fixture, result: &OcrResult, state: &mut MeasurementState) -> bool {
    if result.stamp() != fixture.spec.frame.stamp()
        || result.effective_region() != fixture.spec.effective_region
        || result.output_space() != CoordinateSpace::CapturePixels
        || result.backend() != &fixture.recognizer.descriptor()
        || result.regions().len() != fixture.spec.expected.len()
    {
        return false;
    }

    let extent = fixture.spec.frame.descriptor().extent();
    let mut confidences = Vec::with_capacity(result.regions().len());
    for (actual, expected) in result.regions().iter().zip(fixture.spec.expected.iter()) {
        if actual.text() != expected.text
            || !geometry_matches(actual.geometry().points(), expected.quad, extent)
        {
            return false;
        }
        let confidence = actual.confidence().get();
        if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
            return false;
        }
        confidences.push(confidence);
    }
    match state.confidence.as_ref() {
        Some(baseline) => baseline == &confidences,
        None => {
            state.confidence = Some(confidences);
            true
        }
    }
}

fn geometry_matches(
    actual: [mado_pilot_core::Point; 4],
    expected: [(f64, f64); 4],
    extent: PixelExtent,
) -> bool {
    let actual = actual.map(|point| (point.x(), point.y()));
    let actual_bounds = bounds(actual);
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
    if !union.is_finite() || union <= 0.0 || intersection / union < 0.50 {
        return false;
    }
    let actual_center = (
        (actual_bounds.0 + actual_bounds.2) * 0.5,
        (actual_bounds.1 + actual_bounds.3) * 0.5,
    );
    let expected_center = (
        (expected_bounds.0 + expected_bounds.2) * 0.5,
        (expected_bounds.1 + expected_bounds.3) * 0.5,
    );
    (actual_center.0 - expected_center.0).abs() / f64::from(extent.width()) <= 0.025
        && (actual_center.1 - expected_center.1).abs() / f64::from(extent.height()) <= 0.025
}

fn bounds(points: [(f64, f64); 4]) -> (f64, f64, f64, f64) {
    points.into_iter().fold(
        (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ),
        |bounds, (x, y)| {
            (
                bounds.0.min(x),
                bounds.1.min(y),
                bounds.2.max(x),
                bounds.3.max(y),
            )
        },
    )
}

fn observation_delta(
    before: OnnxBackendObservations,
    after: OnnxBackendObservations,
) -> ResourceSignature {
    ResourceSignature {
        mapped_bytes: after
            .mapped_bytes()
            .checked_sub(before.mapped_bytes())
            .expect("mapped bytes are monotonic"),
        detector_width: after
            .latest_detector_width()
            .expect("successful preprocessing records detector width"),
        detector_height: after
            .latest_detector_height()
            .expect("successful preprocessing records detector height"),
        detector_tensor_bytes: after
            .detector_tensor_bytes()
            .checked_sub(before.detector_tensor_bytes())
            .expect("detector tensor bytes are monotonic"),
        detector_resizes: after
            .detector_resizes()
            .checked_sub(before.detector_resizes())
            .expect("detector resize count is monotonic"),
        detector_runs: after
            .detector_runs()
            .checked_sub(before.detector_runs())
            .expect("detector run count is monotonic"),
        recognizer_runs: after
            .recognizer_runs()
            .checked_sub(before.recognizer_runs())
            .expect("recognizer run count is monotonic"),
    }
}

fn measure_cancelled(
    recognizer: &OcrRecognizer,
    backend: &OnnxOcrBackend,
    spec: &WorkloadSpec,
) -> CancellationReport {
    let token = CancellationToken::new();
    token.cancel();
    let operation = OperationContext::new().with_cancellation(token);
    let descriptor = recognizer.descriptor();
    let before = backend
        .observations()
        .expect("cancellation observes idle backend before refusal");
    let started = Instant::now();
    let result = recognizer.recognize(OcrRequest::new(
        &spec.frame,
        descriptor.backend_identity(),
        descriptor.model_identity(),
        spec.region,
        CoordinateSpace::CapturePixels,
        &operation,
    ));
    let elapsed_ms = milliseconds(started.elapsed());
    let after = backend
        .observations()
        .expect("cancellation observes idle backend after refusal");
    let status = result
        .expect_err("pre-cancelled qualification call is refused")
        .status();
    CancellationReport {
        status: format!("{status:?}"),
        elapsed_ms,
        resources_unchanged: before == after,
    }
}

fn report_passes(report: &ProfileReport, profile: OnnxOcrProfile, enforce_budgets: bool) -> bool {
    let structural = report.max_concurrent_inferences == 1
        && report.session_pairs == 1
        && report.sessions == 2
        && report.cancellation.status == format!("{:?}", Status::Cancelled)
        && report.cancellation.resources_unchanged
        && report.workloads.iter().all(|workload| {
            workload.incorrect_retained == 0
                && workload.all_call_failures == 0
                && workload.growth_bytes <= HEAP_GROWTH_LIMIT
                && workload.resources.is_some()
        });
    if !structural || !enforce_budgets || profile == OnnxOcrProfile::NativeG004 {
        return structural;
    }

    let Some(limits) = target_limits() else {
        return false;
    };
    let timing = report.workloads.iter().all(|workload| {
        let limits = if workload.name == "bounded_blank_4k" {
            (
                limits.empty_p50_ms,
                limits.empty_p95_ms,
                limits.empty_max_ms,
            )
        } else {
            (
                limits.nonempty_p50_ms,
                limits.nonempty_p95_ms,
                limits.nonempty_max_ms,
            )
        };
        workload.p50_ms <= limits.0
            && workload.p95_ms <= limits.1
            && workload.maximum_ms <= limits.2
    });
    timing
        && report.cold_open_ms <= limits.cold_open_ms
        && report.first_close_ms <= limits.first_close_ms
        && report.reopen_close_ms <= limits.reopen_close_ms
        && report
            .peak_resident_bytes
            .is_some_and(|bytes| bytes <= limits.resident_bytes)
}

fn open_profile(
    profile: OnnxOcrProfile,
    model_root: &Path,
    runtime: &Path,
    operation: &OperationContext,
) -> Result<OnnxOcrBackend, mado_pilot_backend_onnx::OnnxBackendFault> {
    match profile {
        OnnxOcrProfile::NativeG004 => OnnxOcrBackend::open_accepted(model_root, runtime, operation),
        OnnxOcrProfile::BoundedDetector => {
            OnnxOcrBackend::open_bounded_detector(model_root, runtime, operation)
        }
    }
}

fn profile_name(profile: OnnxOcrProfile) -> &'static str {
    match profile {
        OnnxOcrProfile::NativeG004 => "native-g004",
        OnnxOcrProfile::BoundedDetector => "bounded-detector-1312x736",
    }
}

fn build_workloads(root: &Path, manifest: &FixtureManifest) -> Vec<WorkloadSpec> {
    let hud = image(manifest, "hud.png");
    let menu = image(manifest, "menu.png");
    let status = image(manifest, "status.png");
    let tooltip = image(manifest, "tooltip-v3.png");
    let mission = image(manifest, "mission.png");

    let hud_mat = load_bgra(&root.join("hud.png"));
    let menu_mat = load_bgra(&root.join("menu.png"));
    let status_mat = load_bgra(&root.join("status.png"));
    let tooltip_mat = load_bgra(&root.join("tooltip-v3.png"));
    let mission_mat = load_bgra(&root.join("mission.png"));

    let hud_4k_mat = resize_bgra(&hud_mat, 3_840, 2_160, INTER_NEAREST_EXACT);
    let hud_odd_mat = resize_bgra(&hud_mat, 1_001, 563, INTER_LINEAR);
    let status_wide_mat = resize_bgra(&status_mat, 569, 320, INTER_LINEAR);
    let mission_region = PixelRect::new(877, 0, 1_440, 720).expect("mission region");

    vec![
        full_workload(
            "bounded_hud_4k",
            frame_from_mat(&hud_4k_mat),
            expected_regions(hud, 4.0, 4.0, 0.0, 0.0),
            DetectorDimensions {
                width: 3_840,
                height: 2_176,
            },
            DetectorDimensions {
                width: 1_312,
                height: 736,
            },
        ),
        full_workload(
            "bounded_menu_wide",
            canvas_frame(&menu_mat, 2_000, 500, 640, 10),
            expected_regions(menu, 1.0, 1.0, 640.0, 10.0),
            DetectorDimensions {
                width: 2_944,
                height: 736,
            },
            DetectorDimensions {
                width: 1_312,
                height: 320,
            },
        ),
        full_workload(
            "bounded_status_extreme_wide",
            canvas_frame(&status_wide_mat, 2_560, 320, 995, 0),
            expected_regions(status, 569.0 / 640.0, 320.0 / 360.0, 995.0, 0.0),
            DetectorDimensions {
                width: 5_888,
                height: 736,
            },
            DetectorDimensions {
                width: 1_312,
                height: 160,
            },
        ),
        full_workload(
            "bounded_hud_reference",
            frame_from_mat(&hud_mat),
            expected_regions(hud, 1.0, 1.0, 0.0, 0.0),
            DetectorDimensions {
                width: 1_312,
                height: 736,
            },
            DetectorDimensions {
                width: 1_312,
                height: 736,
            },
        ),
        full_workload(
            "bounded_hud_odd",
            frame_from_mat(&hud_odd_mat),
            expected_regions(hud, 1_001.0 / 960.0, 563.0 / 540.0, 0.0, 0.0),
            DetectorDimensions {
                width: 1_312,
                height: 736,
            },
            DetectorDimensions {
                width: 1_312,
                height: 736,
            },
        ),
        full_workload(
            "bounded_tooltip_dense",
            frame_from_mat(&tooltip_mat),
            expected_regions(tooltip, 1.0, 1.0, 0.0, 0.0),
            DetectorDimensions {
                width: 1_472,
                height: 736,
            },
            DetectorDimensions {
                width: 1_312,
                height: 640,
            },
        ),
        WorkloadSpec {
            name: "bounded_mission_boundary",
            frame: frame_from_mat(&mission_mat),
            region: OcrRegion::Region {
                rect: Rect::new(CoordinateSpace::CapturePixels, 877.0, 0.0, 1_440.0, 720.0)
                    .expect("mission qualification region"),
                policy: ClipPolicy::Reject,
            },
            effective_region: mission_region,
            expected: expected_regions(mission, 1.0, 1.0, 0.0, 0.0)
                .into_iter()
                .filter(|region| region.quad.iter().all(|(x, _)| *x >= 877.0))
                .collect::<Vec<_>>()
                .into(),
            native_detector: DetectorDimensions {
                width: 736,
                height: 928,
            },
            bounded_detector: DetectorDimensions {
                width: 576,
                height: 736,
            },
        },
        full_workload(
            "bounded_blank_4k",
            opaque_black_frame(3_840, 2_160),
            Vec::new(),
            DetectorDimensions {
                width: 3_840,
                height: 2_176,
            },
            DetectorDimensions {
                width: 1_312,
                height: 736,
            },
        ),
    ]
}

fn full_workload(
    name: &'static str,
    frame: Frame,
    expected: Vec<ExpectedRegion>,
    native_detector: DetectorDimensions,
    bounded_detector: DetectorDimensions,
) -> WorkloadSpec {
    let extent = frame.descriptor().extent();
    WorkloadSpec {
        name,
        frame,
        region: OcrRegion::FullFrame,
        effective_region: PixelRect::new(
            0,
            0,
            i32::try_from(extent.width()).expect("full qualification width"),
            i32::try_from(extent.height()).expect("full qualification height"),
        )
        .expect("full qualification extent"),
        expected: expected.into(),
        native_detector,
        bounded_detector,
    }
}

fn image<'a>(manifest: &'a FixtureManifest, file: &str) -> &'a FixtureImage {
    manifest
        .images
        .iter()
        .find(|image| image.file == file)
        .unwrap_or_else(|| panic!("missing fixture manifest entry {file}"))
}

fn expected_regions(
    image: &FixtureImage,
    scale_x: f64,
    scale_y: f64,
    offset_x: f64,
    offset_y: f64,
) -> Vec<ExpectedRegion> {
    image
        .regions
        .iter()
        .map(|region| ExpectedRegion {
            text: region.text_nfc.clone(),
            quad: region.source_relative_quad.map(|[x, y]| {
                (
                    x * f64::from(image.width) * scale_x + offset_x,
                    y * f64::from(image.height) * scale_y + offset_y,
                )
            }),
        })
        .collect()
}

fn load_bgra(path: &Path) -> Mat {
    let bgr = imread(path.to_str().expect("UTF-8 fixture path"), IMREAD_COLOR)
        .expect("decode tracked fixture");
    let mut bgra = Mat::default();
    cvt_color_def(&bgr, &mut bgra, COLOR_BGR2BGRA).expect("convert fixture to BGRA");
    assert!(bgra.is_continuous());
    bgra
}

fn resize_bgra(source: &Mat, width: i32, height: i32, interpolation: i32) -> Mat {
    let mut resized = Mat::default();
    resize(
        source,
        &mut resized,
        Size::new(width, height),
        0.0,
        0.0,
        interpolation,
    )
    .expect("derive qualification fixture");
    assert!(resized.is_continuous());
    resized
}

fn frame_from_mat(image: &Mat) -> Frame {
    let width = u32::try_from(image.cols()).expect("fixture width");
    let height = u32::try_from(image.rows()).expect("fixture height");
    vision_contract::frame_with_pixels(
        PixelExtent::new(width, height),
        PixelFormat::Bgra8,
        image.data_bytes().expect("fixture BGRA bytes").to_vec(),
    )
}

fn canvas_frame(source: &Mat, width: u32, height: u32, left: u32, top: u32) -> Frame {
    let source_width = u32::try_from(source.cols()).expect("canvas source width");
    let source_height = u32::try_from(source.rows()).expect("canvas source height");
    assert!(left + source_width <= width && top + source_height <= height);
    let row_bytes = usize::try_from(width).unwrap() * 4;
    let source_row_bytes = usize::try_from(source_width).unwrap() * 4;
    let mut pixels = vec![0_u8; row_bytes * usize::try_from(height).unwrap()];
    for pixel in pixels.chunks_exact_mut(4) {
        pixel[3] = 255;
    }
    let source_bytes = source.data_bytes().expect("canvas source bytes");
    for row in 0..usize::try_from(source_height).unwrap() {
        let source_start = row * source_row_bytes;
        let destination_start =
            (usize::try_from(top).unwrap() + row) * row_bytes + usize::try_from(left).unwrap() * 4;
        pixels[destination_start..destination_start + source_row_bytes]
            .copy_from_slice(&source_bytes[source_start..source_start + source_row_bytes]);
    }
    vision_contract::frame_with_pixels(PixelExtent::new(width, height), PixelFormat::Bgra8, pixels)
}

fn opaque_black_frame(width: u32, height: u32) -> Frame {
    let mut pixels =
        vec![0_u8; usize::try_from(width).unwrap() * usize::try_from(height).unwrap() * 4];
    for pixel in pixels.chunks_exact_mut(4) {
        pixel[3] = 255;
    }
    vision_contract::frame_with_pixels(PixelExtent::new(width, height), PixelFormat::Bgra8, pixels)
}

fn required_path(variable: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(variable).expect("qualification path is configured"))
        .canonicalize()
        .expect("qualification path is canonicalizable")
}

fn environment_or(variable: &str, fallback: &str) -> String {
    std::env::var(variable).unwrap_or_else(|_| fallback.to_owned())
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn hex_digest(bytes: &[u8]) -> String {
    encode_hex(Sha256::digest(bytes).as_ref())
}

fn digest_file(path: &Path) -> String {
    use std::io::Read;

    let mut file = std::fs::File::open(path).expect("open qualification identity file");
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .expect("hash qualification identity file");
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    encode_hex(digest.finalize().as_ref())
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
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

#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
fn target_limits() -> Option<TargetLimits> {
    Some(TargetLimits {
        nonempty_p50_ms: 600.0,
        nonempty_p95_ms: 750.0,
        nonempty_max_ms: 900.0,
        empty_p50_ms: 175.0,
        empty_p95_ms: 210.0,
        empty_max_ms: 300.0,
        cold_open_ms: 175.0,
        first_close_ms: 2.0,
        reopen_close_ms: 100.0,
        resident_bytes: 768 * 1024 * 1024,
    })
}

#[cfg(all(target_arch = "x86_64", target_os = "windows", target_env = "msvc"))]
fn target_limits() -> Option<TargetLimits> {
    Some(TargetLimits {
        nonempty_p50_ms: 900.0,
        nonempty_p95_ms: 1_000.0,
        nonempty_max_ms: 1_200.0,
        empty_p50_ms: 350.0,
        empty_p95_ms: 425.0,
        empty_max_ms: 500.0,
        cold_open_ms: 250.0,
        first_close_ms: 10.0,
        reopen_close_ms: 225.0,
        resident_bytes: 320 * 1024 * 1024,
    })
}

#[cfg(not(any(
    all(target_arch = "aarch64", target_os = "macos"),
    all(target_arch = "x86_64", target_os = "windows", target_env = "msvc")
)))]
fn target_limits() -> Option<TargetLimits> {
    None
}
