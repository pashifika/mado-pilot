//! Revision-bound quality and resource qualification for ADRs 0038–0044.

use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use mado_pilot_backend_onnx::benchmark_instrumentation::install_native_run_gate;
use mado_pilot_backend_onnx::{
    OnnxBackendFault, OnnxBackendObservations, OnnxOcrBackend, OnnxOcrProfile, RUNTIME_PROFILE_ID,
};
use mado_pilot_capture::{Frame, PixelFormat};
use mado_pilot_core::{
    CancellationToken, ClipPolicy, CoordinateSpace, OperationContext, PixelExtent, PixelRect, Rect,
    Status,
};
use mado_pilot_ocr::{
    OcrBackend, OcrBackendDescriptor, OcrRecognizer, OcrRegion, OcrRequest, OcrResult, OcrZone,
    OcrZoneScanRequest, OcrZoneScanResult,
};
use mado_pilot_testkit::bench_harness::{
    self, Accounting, PHASE3_1_BOUNDED_OCR_HEAP_LIMIT_BYTES,
    PHASE3_1_BOUNDED_OCR_MAX_DETECTOR_TENSOR_BYTES, Plan, Sample,
};
#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
use mado_pilot_testkit::bench_harness::{
    PHASE3_1_APPLE_BOUNDED_OCR_CLOSE_LIMIT, PHASE3_1_APPLE_BOUNDED_OCR_COLD_LOAD_LIMIT,
    PHASE3_1_APPLE_BOUNDED_OCR_LATENCY_BUDGETS, PHASE3_1_APPLE_BOUNDED_OCR_REOPEN_CLOSE_LIMIT,
    PHASE3_1_APPLE_BOUNDED_OCR_RESIDENT_LIMIT_BYTES, PHASE3_1_APPLE_GROUPED_OCR_CANCELLATION_LIMIT,
    PHASE3_1_APPLE_GROUPED_OCR_LATENCY_BUDGETS, PHASE3_1_APPLE_GROUPED_OCR_RETAINED_RESULT_LIMIT,
};
#[cfg(all(target_arch = "x86_64", target_os = "windows", target_env = "msvc"))]
use mado_pilot_testkit::bench_harness::{
    PHASE3_1_WINDOWS_BOUNDED_OCR_CLOSE_LIMIT, PHASE3_1_WINDOWS_BOUNDED_OCR_COLD_LOAD_LIMIT,
    PHASE3_1_WINDOWS_BOUNDED_OCR_LATENCY_BUDGETS, PHASE3_1_WINDOWS_BOUNDED_OCR_REOPEN_CLOSE_LIMIT,
    PHASE3_1_WINDOWS_BOUNDED_OCR_RESIDENT_LIMIT_BYTES,
    PHASE3_1_WINDOWS_GROUPED_OCR_CANCELLATION_LIMIT, PHASE3_1_WINDOWS_GROUPED_OCR_LATENCY_BUDGETS,
    PHASE3_1_WINDOWS_GROUPED_OCR_RETAINED_RESULT_LIMIT,
};
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
const SOURCE_TREE_ENV: &str = "MADO_PILOT_BOUNDED_SOURCE_TREE";
const HEAP_GROWTH_LIMIT: i64 = 4_096;
const DETECTOR_SHA256: &str = "d2a7720d45a54257208b1e13e36a8479894cb74155a5efe29462512d42f49da9";
const RECOGNIZER_SHA256: &str = "6f327246b50388f3c176ae304bd95767ea6dc0c9ae92153ef8cbe210b3c14884";
const VOCABULARY_SHA256: &str = "f7aa897ca828a4c7c9e2739c30f9161a33306d532f020bcdb91dcfb664a5507e";
const GROUPED_RESULT_BYTES_LIMIT: usize = 5_242_880;
const ACTIVE_CANCELLATION_BOUND: Duration = Duration::from_millis(250);
const ACTIVE_CANCELLATION_GATE_BOUND: Duration = Duration::from_secs(2);

#[derive(Debug, Deserialize)]
struct FixtureManifest {
    images: Vec<FixtureImage>,
}

#[derive(Debug, Deserialize)]
struct FixtureImage {
    sha256: String,
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

#[derive(Debug, Clone, Copy)]
enum WorkloadKind {
    Hud4k,
    MenuWide,
    StatusExtremeWide,
    HudReference,
    HudOdd,
    TooltipDense,
    MissionBoundary,
    Blank4k,
}

impl WorkloadKind {
    const ALL: [Self; 8] = [
        Self::Hud4k,
        Self::MenuWide,
        Self::StatusExtremeWide,
        Self::HudReference,
        Self::HudOdd,
        Self::TooltipDense,
        Self::MissionBoundary,
        Self::Blank4k,
    ];
}

#[derive(Debug, Clone, Copy)]
enum ZoneWorkloadKind {
    OneFull,
    ThreeSparse,
    EightDistinct,
    DenseUnique,
    Empty4k,
}

impl ZoneWorkloadKind {
    const ALL: [Self; 5] = [
        Self::OneFull,
        Self::ThreeSparse,
        Self::EightDistinct,
        Self::DenseUnique,
        Self::Empty4k,
    ];
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

#[derive(Debug, Clone)]
struct ZoneWorkloadSpec {
    name: &'static str,
    frame: Frame,
    zones: Arc<[OcrZone]>,
    effective_zones: Arc<[PixelRect]>,
    source_envelope: PixelRect,
    expected: Arc<[ExpectedRegion]>,
    expected_groups: Arc<[Arc<[usize]>]>,
    expected_ignored_candidates: usize,
}

impl ZoneWorkloadSpec {
    fn mapped_bytes(&self) -> u64 {
        u64::from(self.source_envelope.width()) * u64::from(self.source_envelope.height()) * 4
    }

    fn memberships(&self) -> usize {
        self.expected_groups.iter().map(|group| group.len()).sum()
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
struct ZoneResourceSignature {
    inference: ResourceSignature,
    zones: usize,
    unique_candidates: usize,
    memberships: usize,
    selected_candidates: u64,
    ignored_candidates: u64,
    backend_unique_candidates: u64,
    backend_memberships: u64,
    cleanup_completions: u64,
    normalized_text_bytes: usize,
    result_semantic_bytes: usize,
}

#[derive(Debug, Default)]
struct ZoneMeasurementState {
    confidence: Option<Vec<f64>>,
    resources: Option<ZoneResourceSignature>,
    calls: usize,
    failures: usize,
}

struct ZoneFixture {
    recognizer: OcrRecognizer,
    backend: Arc<OnnxOcrBackend>,
    spec: ZoneWorkloadSpec,
    state: Arc<Mutex<ZoneMeasurementState>>,
}

#[derive(Debug, Serialize)]
struct ZoneWorkloadReport {
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
    process_peak_resident_bytes_after_workload: Option<u64>,
    resources: Option<ZoneResourceSignature>,
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
    process_peak_resident_bytes_after_workload: Option<u64>,
    resources: Option<ResourceSignature>,
}

#[derive(Debug, Serialize)]
struct CancellationReport {
    status: String,
    elapsed_ms: f64,
    resources_unchanged: bool,
}

#[derive(Debug, Serialize)]
struct ActiveCancellationReport {
    status: String,
    cancellation_to_return_ms: f64,
    detector_runs: u64,
    cleanup_completions: u64,
}

#[derive(Debug, Serialize)]
struct RetainedResultReport {
    elapsed_ms: f64,
    unique_candidates: usize,
    memberships: usize,
    semantic_bytes: usize,
    resource_oracle_passed: bool,
    readable_after_close: bool,
}

#[derive(Debug, Serialize)]
struct ProfileReport {
    schema_version: u32,
    release_target: &'static str,
    host_id: String,
    source_revision: String,
    process_index: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_tree: Option<String>,
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
    integrated_zones: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    integrated_workload_order: Option<&'static str>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    active_cancellation: Option<ActiveCancellationReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retained_result: Option<RetainedResultReport>,
    workloads: Vec<WorkloadReport>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    zone_workloads: Vec<ZoneWorkloadReport>,
    passed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunMode {
    Smoke,
    Precursor,
    EnforceBudgets,
}

impl RunMode {
    fn from_arguments(arguments: &[String]) -> Self {
        let smoke_count = arguments
            .iter()
            .filter(|argument| argument.as_str() == "--smoke")
            .count();
        let precursor_count = arguments
            .iter()
            .filter(|argument| argument.as_str() == "--precursor")
            .count();
        let enforce_count = arguments
            .iter()
            .filter(|argument| argument.as_str() == "--enforce-budgets")
            .count();
        let smoke = smoke_count == 1;
        let precursor = precursor_count == 1;
        let enforce = enforce_count == 1;
        assert!(
            !arguments.iter().any(|argument| argument == "--qualify"),
            "use the explicit --enforce-budgets mode"
        );
        assert!(
            arguments.iter().all(|argument| {
                matches!(
                    argument.as_str(),
                    "--bench"
                        | "--smoke"
                        | "--precursor"
                        | "--enforce-budgets"
                        | "--integrated"
                        | "--zones-first"
                ) || matches!(
                    argument.strip_prefix("--profile="),
                    Some("native" | "bounded")
                )
            }),
            "unsupported bounded-detector benchmark argument"
        );
        assert!(
            smoke_count <= 1
                && precursor_count <= 1
                && enforce_count <= 1
                && smoke_count + precursor_count + enforce_count <= 1,
            "select one non-duplicated benchmark mode"
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

    const fn enforces_budgets(self) -> bool {
        matches!(self, Self::EnforceBudgets)
    }

    const fn requires_bound_identity(self) -> bool {
        !matches!(self, Self::Smoke)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IntegratedOrder {
    Disabled,
    SingularFirst,
    ZonesFirst,
}

impl IntegratedOrder {
    fn from_arguments(arguments: &[String]) -> Self {
        let integrated = arguments
            .iter()
            .filter(|argument| argument.as_str() == "--integrated")
            .count();
        let zones_first = arguments
            .iter()
            .filter(|argument| argument.as_str() == "--zones-first")
            .count();
        assert!(
            integrated <= 1 && zones_first <= 1,
            "select integrated workload options at most once"
        );
        assert!(
            zones_first == 0 || integrated == 1,
            "--zones-first requires --integrated"
        );
        match (integrated, zones_first) {
            (0, 0) => Self::Disabled,
            (1, 0) => Self::SingularFirst,
            (1, 1) => Self::ZonesFirst,
            _ => unreachable!("validated integrated workload options"),
        }
    }

    const fn enabled(self) -> bool {
        !matches!(self, Self::Disabled)
    }

    const fn zones_first(self) -> bool {
        matches!(self, Self::ZonesFirst)
    }

    const fn name(self) -> Option<&'static str> {
        match self {
            Self::Disabled => None,
            Self::SingularFirst => Some("singular-first"),
            Self::ZonesFirst => Some("zones-first"),
        }
    }
}

fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let mode = RunMode::from_arguments(&arguments);
    let integrated_order = IntegratedOrder::from_arguments(&arguments);
    let profile = selected_profile(&arguments);
    assert!(
        !integrated_order.enabled() || profile == OnnxOcrProfile::BoundedDetector,
        "integrated grouped rows require the bounded profile"
    );
    if [RUNTIME_ENV, MODEL_ROOT_ENV]
        .into_iter()
        .any(|variable| std::env::var_os(variable).is_none())
    {
        let explicit_smoke = arguments.iter().any(|argument| argument == "--smoke");
        let debug_harness_smoke = cfg!(debug_assertions)
            && (arguments.is_empty() || (arguments.len() == 1 && arguments[0] == "--bench"));
        assert!(
            mode == RunMode::Smoke && (explicit_smoke || debug_harness_smoke),
            "precursor and final enforcement require both reviewed MADO_PILOT_* paths"
        );
        eprintln!("bounded-detector benchmark skipped: set the two reviewed MADO_PILOT_* paths");
        return;
    }

    let runtime = required_path(RUNTIME_ENV);
    let model_root = required_path(MODEL_ROOT_ENV);
    let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/ocr/g-004");
    let manifest_path = fixture_root.join("fixture-manifest.json");
    let manifest_bytes = std::fs::read(&manifest_path).expect("read tracked OCR fixture manifest");
    let manifest: FixtureManifest =
        serde_json::from_slice(&manifest_bytes).expect("parse tracked OCR fixture manifest");

    let report = run_profile(
        profile,
        &model_root,
        &runtime,
        &fixture_root,
        &manifest,
        mode,
        integrated_order,
    );
    println!(
        "{}",
        serde_json::to_string(&report).expect("serialize bounded qualification report")
    );
    assert!(report.passed, "bounded detector qualification gate failed");
}

fn selected_profile(arguments: &[String]) -> OnnxOcrProfile {
    let mut selected = arguments
        .iter()
        .filter_map(|argument| argument.strip_prefix("--profile="));
    let profile = selected.next().unwrap_or("bounded");
    assert!(
        selected.next().is_none(),
        "select exactly one qualification profile"
    );
    match profile {
        "native" => OnnxOcrProfile::NativeG004,
        "bounded" => OnnxOcrProfile::BoundedDetector,
        other => panic!("unsupported qualification profile {other}"),
    }
}

fn run_profile(
    profile: OnnxOcrProfile,
    model_root: &Path,
    runtime: &Path,
    fixture_root: &Path,
    manifest: &FixtureManifest,
    mode: RunMode,
    integrated_order: IntegratedOrder,
) -> ProfileReport {
    let integrated = integrated_order.enabled();
    let plan = mode.plan();
    assert!(
        !mode.enforces_budgets() || profile == OnnxOcrProfile::BoundedDetector,
        "final budgets apply only to the bounded candidate"
    );
    if mode.requires_bound_identity() {
        require_release_target();
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
        if integrated {
            assert!(
                std::env::var_os(SOURCE_TREE_ENV).is_some(),
                "integrated qualification requires exact source tree identity"
            );
        }
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

    let mut zone_workload_reports = if integrated_order.zones_first() {
        measure_zone_workloads(&recognizer, &backend, fixture_root, manifest, plan)
    } else {
        Vec::new()
    };
    let mut workload_reports = Vec::with_capacity(WorkloadKind::ALL.len());
    for kind in WorkloadKind::ALL {
        workload_reports.push(measure_workload(
            recognizer.clone(),
            Arc::clone(&backend),
            build_workload(fixture_root, manifest, kind),
            profile,
            plan,
        ));
    }
    if integrated && !integrated_order.zones_first() {
        zone_workload_reports =
            measure_zone_workloads(&recognizer, &backend, fixture_root, manifest, plan);
    }

    let cancellation_spec = build_workload(fixture_root, manifest, WorkloadKind::HudReference);
    let cancellation = measure_cancelled(&recognizer, &backend, &cancellation_spec);
    let active_cancellation = integrated.then(|| {
        let spec = build_zone_workload(fixture_root, manifest, ZoneWorkloadKind::OneFull);
        measure_active_cancellation(&recognizer, &backend, &spec)
    });
    let retained_spec =
        integrated.then(|| build_zone_workload(fixture_root, manifest, ZoneWorkloadKind::OneFull));
    let (retained_owner, mut retained_report) = match retained_spec.as_ref() {
        Some(spec) => {
            let (owner, report) = measure_retained_result(&recognizer, &backend, spec);
            (Some(owner), Some(report))
        }
        None => (None, None),
    };
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
    if let (Some(owner), Some(spec), Some(report)) = (
        retained_owner.as_ref(),
        retained_spec.as_ref(),
        retained_report.as_mut(),
    ) {
        report.readable_after_close = retained_result_matches(spec, owner, &descriptor);
    }

    let reopen_started = Instant::now();
    let reopened = open_profile(profile, model_root, runtime, &operation)
        .expect("selected backend reopens after cleanup");
    reopened.close(&operation).expect("reopened backend closes");
    let reopen_close_ms = milliseconds(reopen_started.elapsed());
    let peak_resident_bytes = peak_resident_bytes();

    let identity = descriptor.model_identity();
    let mut report = ProfileReport {
        schema_version: if integrated { 4 } else { 3 },
        release_target: bench_harness::RELEASE_TARGET,
        host_id: environment_or(HOST_ENV, "unbound-smoke-host"),
        source_revision: environment_or(SOURCE_ENV, "unbound-smoke-source"),
        process_index: environment_or(PROCESS_ENV, "smoke"),
        source_tree: std::env::var(SOURCE_TREE_ENV).ok(),
        executable_sha256: digest_file(
            &std::env::current_exe().expect("qualification executable path"),
        ),
        runtime_sha256: digest_file(runtime),
        fixture_manifest_sha256: digest_file(&fixture_root.join("fixture-manifest.json")),
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
        integrated_zones: integrated,
        integrated_workload_order: integrated_order.name(),
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
        active_cancellation,
        retained_result: retained_report,
        workloads: workload_reports,
        zone_workloads: zone_workload_reports,
        passed: false,
    };
    report.passed = report_passes(mode, profile, &report)
        && (!mode.enforces_budgets() || target_budget_passes(&report));
    report
}

fn measure_zone_workloads(
    recognizer: &OcrRecognizer,
    backend: &Arc<OnnxOcrBackend>,
    fixture_root: &Path,
    manifest: &FixtureManifest,
    plan: Plan,
) -> Vec<ZoneWorkloadReport> {
    let mut reports = Vec::with_capacity(ZoneWorkloadKind::ALL.len());
    for kind in ZoneWorkloadKind::ALL {
        reports.push(measure_zone_workload(
            recognizer.clone(),
            Arc::clone(backend),
            build_zone_workload(fixture_root, manifest, kind),
            plan,
        ));
    }
    reports
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
        process_peak_resident_bytes_after_workload: workload.peak_resident_bytes(),
        resources: state.resources,
    }
}

fn measure_zone_workload(
    recognizer: OcrRecognizer,
    backend: Arc<OnnxOcrBackend>,
    spec: ZoneWorkloadSpec,
    plan: Plan,
) -> ZoneWorkloadReport {
    let state = Arc::new(Mutex::new(ZoneMeasurementState::default()));
    let fixture_state = Arc::clone(&state);
    let fixture_spec = spec.clone();
    let workload = bench_harness::measure(
        spec.name,
        "exact source/envelope/zones/text/geometry/order, one detector scan, unique recognition, compact memberships, and cleanup",
        plan,
        || ZoneFixture {
            recognizer,
            backend,
            spec: fixture_spec,
            state: fixture_state,
        },
        scan_zones,
    );
    let state = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    ZoneWorkloadReport {
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
        process_peak_resident_bytes_after_workload: workload.peak_resident_bytes(),
        resources: state.resources,
    }
}

fn scan_zones(fixture: &ZoneFixture) -> Sample {
    let descriptor = fixture.recognizer.descriptor();
    let before = fixture
        .backend
        .observations()
        .expect("integrated qualification observes an idle backend before inference");
    let started = Instant::now();
    let operation = OperationContext::new();
    let request = OcrZoneScanRequest::new(
        &fixture.spec.frame,
        descriptor.backend_identity(),
        descriptor.model_identity(),
        &fixture.spec.zones,
        CoordinateSpace::CapturePixels,
        &operation,
    )
    .expect("predeclared integrated zone request is valid");
    let result = fixture.recognizer.scan_zones(request);
    let elapsed = started.elapsed();
    let after = fixture
        .backend
        .observations()
        .expect("integrated qualification observes an idle backend after inference");
    let inference = observation_delta(before, after);
    let unique_candidates = result
        .as_ref()
        .map_or(0, |result| result.unique_candidates().len());
    let memberships = result.as_ref().map_or(0, |result| {
        (0..result.effective_zones().len())
            .map(|index| {
                result
                    .group(index)
                    .expect("effective zone has a group")
                    .len()
            })
            .sum()
    });
    let (normalized_text_bytes, result_semantic_bytes) = result
        .as_ref()
        .ok()
        .and_then(zone_result_storage)
        .unwrap_or((usize::MAX, usize::MAX));
    let resources = ZoneResourceSignature {
        inference,
        zones: fixture.spec.zones.len(),
        unique_candidates,
        memberships,
        selected_candidates: after
            .selected_candidates()
            .checked_sub(before.selected_candidates())
            .expect("selected candidate count is monotonic"),
        ignored_candidates: after
            .ignored_candidates()
            .checked_sub(before.ignored_candidates())
            .expect("ignored candidate count is monotonic"),
        backend_unique_candidates: after
            .unique_candidates()
            .checked_sub(before.unique_candidates())
            .expect("unique candidate count is monotonic"),
        backend_memberships: after
            .memberships()
            .checked_sub(before.memberships())
            .expect("membership count is monotonic"),
        cleanup_completions: after
            .cleanup_completions()
            .checked_sub(before.cleanup_completions())
            .expect("cleanup completion count is monotonic"),
        normalized_text_bytes,
        result_semantic_bytes,
    };
    let expected_recognizer_runs =
        u64::try_from(fixture.spec.expected.len().div_ceil(6)).expect("candidate count fits u64");
    let detector_plan_matches = inference.detector_width != 0
        && inference.detector_height != 0
        && inference.detector_width.is_multiple_of(32)
        && inference.detector_height.is_multiple_of(32)
        && inference.detector_width <= 1_312
        && inference.detector_height <= 736
        && inference.detector_tensor_bytes
            == u64::from(inference.detector_width) * u64::from(inference.detector_height) * 3 * 4
        && inference.detector_tensor_bytes <= PHASE3_1_BOUNDED_OCR_MAX_DETECTOR_TENSOR_BYTES;
    let expected_candidates =
        u64::try_from(fixture.spec.expected.len()).expect("candidate count fits u64");
    let expected_memberships =
        u64::try_from(fixture.spec.memberships()).expect("membership count fits u64");
    let expected_text_bytes = fixture
        .spec
        .expected
        .iter()
        .try_fold(0_usize, |total, region| {
            total.checked_add(region.text.len())
        });
    let resources_match = inference.mapped_bytes == fixture.spec.mapped_bytes()
        && inference.detector_resizes == 1
        && inference.detector_runs == 1
        && inference.recognizer_runs == expected_recognizer_runs
        && resources.zones == fixture.spec.effective_zones.len()
        && resources.unique_candidates == fixture.spec.expected.len()
        && resources.memberships == fixture.spec.memberships()
        && resources.selected_candidates == expected_candidates
        && resources.ignored_candidates
            == u64::try_from(fixture.spec.expected_ignored_candidates)
                .expect("ignored candidate count fits u64")
        && resources.backend_unique_candidates == expected_candidates
        && resources.backend_memberships == expected_memberships
        && resources.cleanup_completions == 1
        && expected_text_bytes == Some(resources.normalized_text_bytes)
        && resources.result_semantic_bytes <= GROUPED_RESULT_BYTES_LIMIT
        && detector_plan_matches;

    let mut state = fixture
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let result_matches = result
        .as_ref()
        .is_ok_and(|result| zone_quality_oracle(fixture, result, &mut state));
    let stable_resources = state.resources.is_none_or(|baseline| baseline == resources);
    state.resources.get_or_insert(resources);
    state.calls += 1;
    let correct = result_matches && resources_match && stable_resources;
    if !correct {
        state.failures += 1;
        let status = result.as_ref().err().map(|error| error.status());
        eprintln!(
            "integrated-zone-qualification-failure workload={} status={status:?} resources={resources:?}",
            fixture.spec.name
        );
    }
    drop(state);

    let sample = Sample::new(elapsed, correct, inference.mapped_bytes);
    match peak_resident_bytes() {
        Some(bytes) => sample.with_peak_resident_bytes(bytes),
        None => sample,
    }
}

fn zone_quality_oracle(
    fixture: &ZoneFixture,
    result: &OcrZoneScanResult,
    state: &mut ZoneMeasurementState,
) -> bool {
    if result.stamp() != fixture.spec.frame.stamp()
        || result.source_envelope() != fixture.spec.source_envelope
        || result.effective_zones() != fixture.spec.effective_zones.as_ref()
        || result.output_space() != CoordinateSpace::CapturePixels
        || result.backend() != &fixture.recognizer.descriptor()
        || result.unique_candidates().len() != fixture.spec.expected.len()
    {
        return false;
    }

    let extent = fixture.spec.frame.descriptor().extent();
    let mut confidences = Vec::with_capacity(result.unique_candidates().len());
    for (actual, expected) in result
        .unique_candidates()
        .iter()
        .zip(fixture.spec.expected.iter())
    {
        if !region_matches(actual, expected, extent) {
            return false;
        }
        confidences.push(actual.confidence().get());
    }
    for (group_index, expected_indexes) in fixture.spec.expected_groups.iter().enumerate() {
        let Some(group) = result.group(group_index) else {
            return false;
        };
        if group.len() != expected_indexes.len()
            || !group.iter().zip(expected_indexes.iter().copied()).all(
                |(actual, expected_index)| {
                    region_matches(actual, &fixture.spec.expected[expected_index], extent)
                },
            )
        {
            return false;
        }
    }
    match state.confidence.as_ref() {
        Some(baseline) => baseline == &confidences,
        None => {
            state.confidence = Some(confidences);
            true
        }
    }
}

fn region_matches(
    actual: &mado_pilot_ocr::RecognizedRegion,
    expected: &ExpectedRegion,
    extent: PixelExtent,
) -> bool {
    let confidence = actual.confidence().get();
    actual.text() == expected.text
        && geometry_matches(actual.geometry().points(), expected.quad, extent)
        && confidence.is_finite()
        && (0.0..=1.0).contains(&confidence)
}

fn zone_result_storage(result: &OcrZoneScanResult) -> Option<(usize, usize)> {
    let normalized_text_bytes = result
        .unique_candidates()
        .iter()
        .try_fold(0_usize, |total, region| {
            total.checked_add(region.text().len())
        })?;
    let memberships = (0..result.effective_zones().len()).try_fold(0_usize, |total, index| {
        total.checked_add(result.group(index)?.len())
    })?;
    let semantic_bytes = size_of::<OcrZoneScanResult>()
        .checked_add(
            result
                .effective_zones()
                .len()
                .checked_mul(size_of::<PixelRect>())?,
        )?
        .checked_add(
            result
                .unique_candidates()
                .len()
                .checked_mul(size_of::<mado_pilot_ocr::RecognizedRegion>())?,
        )?
        .checked_add(normalized_text_bytes)?
        .checked_add(memberships.checked_mul(size_of::<u16>())?)?;
    Some((normalized_text_bytes, semantic_bytes))
}

fn retained_result_matches(
    spec: &ZoneWorkloadSpec,
    result: &OcrZoneScanResult,
    descriptor: &OcrBackendDescriptor,
) -> bool {
    if result.stamp() != spec.frame.stamp()
        || result.source_envelope() != spec.source_envelope
        || result.effective_zones() != spec.effective_zones.as_ref()
        || result.output_space() != CoordinateSpace::CapturePixels
        || result.backend() != descriptor
        || result.unique_candidates().len() != spec.expected.len()
    {
        return false;
    }
    let extent = spec.frame.descriptor().extent();
    if !result
        .unique_candidates()
        .iter()
        .zip(spec.expected.iter())
        .all(|(actual, expected)| region_matches(actual, expected, extent))
    {
        return false;
    }
    spec.expected_groups
        .iter()
        .enumerate()
        .all(|(group_index, expected_indexes)| {
            result.group(group_index).is_some_and(|group| {
                group.len() == expected_indexes.len()
                    && group.iter().zip(expected_indexes.iter().copied()).all(
                        |(actual, expected_index)| {
                            region_matches(actual, &spec.expected[expected_index], extent)
                        },
                    )
            })
        })
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
        let status = result.as_ref().err().map(|error| error.status());
        eprintln!(
            "bounded-qualification-failure profile={} workload={} status={status:?} resources={resources:?}",
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
        && actual.into_iter().zip(expected).all(
            |((actual_x, actual_y), (expected_x, expected_y))| {
                (actual_x - expected_x).abs() / f64::from(extent.width()) <= 0.025
                    && (actual_y - expected_y).abs() / f64::from(extent.height()) <= 0.025
            },
        )
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

fn measure_active_cancellation(
    recognizer: &OcrRecognizer,
    backend: &Arc<OnnxOcrBackend>,
    spec: &ZoneWorkloadSpec,
) -> ActiveCancellationReport {
    let before = backend
        .observations()
        .expect("active cancellation starts from an idle backend");
    let token = CancellationToken::new();
    let worker_token = token.clone();
    let worker_recognizer = recognizer.clone();
    let worker_spec = spec.clone();
    let (sender, receiver) = mpsc::sync_channel(1);
    let gate = install_native_run_gate();
    let worker = thread::spawn(move || {
        let operation = OperationContext::new().with_cancellation(worker_token);
        let descriptor = worker_recognizer.descriptor();
        let request = OcrZoneScanRequest::new(
            &worker_spec.frame,
            descriptor.backend_identity(),
            descriptor.model_identity(),
            &worker_spec.zones,
            CoordinateSpace::CapturePixels,
            &operation,
        )
        .expect("active cancellation request is valid");
        let status = worker_recognizer
            .scan_zones(request)
            .err()
            .map(|error| error.status());
        sender
            .send(status)
            .expect("active cancellation receiver remains live");
    });

    assert!(
        gate.wait_until_admitted(ACTIVE_CANCELLATION_GATE_BOUND),
        "active cancellation native admission timed out"
    );
    gate.release();
    assert!(
        gate.wait_until_run_started(ACTIVE_CANCELLATION_GATE_BOUND),
        "active cancellation native run start timed out"
    );
    let cancellation_started = Instant::now();
    token.cancel();
    assert!(
        gate.wait_until_termination_issued(ACTIVE_CANCELLATION_BOUND),
        "active cancellation native termination timed out"
    );
    let remaining = ACTIVE_CANCELLATION_BOUND.saturating_sub(cancellation_started.elapsed());
    let status = receiver
        .recv_timeout(remaining)
        .expect("active cancellation returns within the hard bound")
        .expect("active cancellation cannot publish a successful result");
    let cancellation_to_return_ms = milliseconds(cancellation_started.elapsed());
    worker.join().expect("active cancellation worker joins");
    drop(gate);
    let after = backend
        .observations()
        .expect("active cancellation leaves the backend observable");
    ActiveCancellationReport {
        status: format!("{status:?}"),
        cancellation_to_return_ms,
        detector_runs: after
            .detector_runs()
            .checked_sub(before.detector_runs())
            .expect("active cancellation detector count is monotonic"),
        cleanup_completions: after
            .cleanup_completions()
            .checked_sub(before.cleanup_completions())
            .expect("active cancellation cleanup count is monotonic"),
    }
}

fn measure_retained_result(
    recognizer: &OcrRecognizer,
    backend: &OnnxOcrBackend,
    spec: &ZoneWorkloadSpec,
) -> (OcrZoneScanResult, RetainedResultReport) {
    let descriptor = recognizer.descriptor();
    let before = backend
        .observations()
        .expect("retained-result workload starts from an idle backend");
    let operation = OperationContext::new();
    let started = Instant::now();
    let request = OcrZoneScanRequest::new(
        &spec.frame,
        descriptor.backend_identity(),
        descriptor.model_identity(),
        &spec.zones,
        CoordinateSpace::CapturePixels,
        &operation,
    )
    .expect("retained-result request is valid");
    let result = recognizer
        .scan_zones(request)
        .expect("retained-result workload succeeds");
    let elapsed_ms = milliseconds(started.elapsed());
    let after = backend
        .observations()
        .expect("retained-result workload leaves the backend idle");
    let inference = observation_delta(before, after);
    let memberships = (0..result.effective_zones().len())
        .map(|index| {
            result
                .group(index)
                .expect("retained-result effective zone has a group")
                .len()
        })
        .sum();
    let semantic_bytes = zone_result_storage(&result)
        .map(|(_, bytes)| bytes)
        .unwrap_or(usize::MAX);
    let expected_candidates =
        u64::try_from(spec.expected.len()).expect("retained candidate count fits u64");
    let expected_memberships =
        u64::try_from(spec.memberships()).expect("retained membership count fits u64");
    let resource_oracle_passed = retained_result_matches(spec, &result, &descriptor)
        && inference.mapped_bytes == spec.mapped_bytes()
        && inference.detector_resizes == 1
        && inference.detector_runs == 1
        && inference.recognizer_runs
            == u64::try_from(spec.expected.len().div_ceil(6))
                .expect("retained recognizer count fits u64")
        && after
            .selected_candidates()
            .checked_sub(before.selected_candidates())
            == Some(expected_candidates)
        && after
            .ignored_candidates()
            .checked_sub(before.ignored_candidates())
            == Some(
                u64::try_from(spec.expected_ignored_candidates)
                    .expect("retained ignored count fits u64"),
            )
        && after
            .unique_candidates()
            .checked_sub(before.unique_candidates())
            == Some(expected_candidates)
        && after.memberships().checked_sub(before.memberships()) == Some(expected_memberships)
        && after
            .cleanup_completions()
            .checked_sub(before.cleanup_completions())
            == Some(1)
        && semantic_bytes <= GROUPED_RESULT_BYTES_LIMIT;
    let report = RetainedResultReport {
        elapsed_ms,
        unique_candidates: result.unique_candidates().len(),
        memberships,
        semantic_bytes,
        resource_oracle_passed,
        readable_after_close: false,
    };
    (result, report)
}

fn report_passes(mode: RunMode, profile: OnnxOcrProfile, report: &ProfileReport) -> bool {
    report.max_concurrent_inferences == 1
        && report.session_pairs == 1
        && report.sessions == 2
        && report.zone_workloads.len()
            == if report.integrated_zones {
                ZoneWorkloadKind::ALL.len()
            } else {
                0
            }
        && (mode == RunMode::Smoke || report.peak_resident_bytes.is_some())
        && report.cancellation.status == format!("{:?}", Status::Cancelled)
        && report.cancellation.resources_unchanged
        && if report.integrated_zones {
            report
                .active_cancellation
                .as_ref()
                .is_some_and(|cancellation| {
                    cancellation.status == format!("{:?}", Status::Cancelled)
                        && cancellation.cancellation_to_return_ms
                            <= milliseconds(ACTIVE_CANCELLATION_BOUND)
                        && cancellation.detector_runs == 1
                        && cancellation.cleanup_completions == 1
                })
                && report.retained_result.as_ref().is_some_and(|retained| {
                    retained.unique_candidates == 8
                        && retained.memberships == 8
                        && retained.semantic_bytes <= GROUPED_RESULT_BYTES_LIMIT
                        && retained.resource_oracle_passed
                        && retained.readable_after_close
                })
        } else {
            report.active_cancellation.is_none() && report.retained_result.is_none()
        }
        && report.workloads.iter().all(|workload| {
            workload.incorrect_retained == 0
                && workload.all_call_failures == 0
                && workload.growth_bytes <= HEAP_GROWTH_LIMIT
                && (profile != OnnxOcrProfile::BoundedDetector
                    || workload.peak_allocated_bytes <= PHASE3_1_BOUNDED_OCR_HEAP_LIMIT_BYTES)
                && (mode == RunMode::Smoke
                    || workload
                        .process_peak_resident_bytes_after_workload
                        .is_some())
                && workload.resources.is_some()
        })
        && report.zone_workloads.iter().all(|workload| {
            workload.incorrect_retained == 0
                && workload.all_call_failures == 0
                && workload.growth_bytes <= HEAP_GROWTH_LIMIT
                && workload.peak_allocated_bytes <= PHASE3_1_BOUNDED_OCR_HEAP_LIMIT_BYTES
                && (mode == RunMode::Smoke
                    || workload
                        .process_peak_resident_bytes_after_workload
                        .is_some())
                && workload.resources.is_some()
        })
}

struct TargetBudgets {
    latency: &'static [bench_harness::LatencyBudget],
    grouped_latency: &'static [bench_harness::LatencyBudget],
    cold_load: Duration,
    close: Duration,
    reopen_close: Duration,
    resident_bytes: u64,
    active_cancellation: Duration,
    retained_result: Duration,
}

fn report_within_target_budgets(report: &ProfileReport, budgets: &TargetBudgets) -> bool {
    report.cold_open_ms <= milliseconds(budgets.cold_load)
        && report.first_close_ms <= milliseconds(budgets.close)
        && report.reopen_close_ms <= milliseconds(budgets.reopen_close)
        && report
            .peak_resident_bytes
            .is_some_and(|bytes| bytes <= budgets.resident_bytes)
        && report.max_detector_width == Some(1_312)
        && report.max_detector_height == Some(736)
        && report.max_detector_tensor_bytes == PHASE3_1_BOUNDED_OCR_MAX_DETECTOR_TENSOR_BYTES
        && report.workloads.len() == budgets.latency.len()
        && report.workloads.iter().all(|workload| {
            budgets
                .latency
                .iter()
                .filter(|budget| budget.workload() == workload.name)
                .count()
                == 1
        })
        && budgets.latency.iter().all(|budget| {
            let mut matching = report
                .workloads
                .iter()
                .filter(|workload| workload.name == budget.workload());
            let Some(workload) = matching.next() else {
                return false;
            };
            matching.next().is_none()
                && workload.p50_ms <= milliseconds(budget.p50())
                && workload.p95_ms <= milliseconds(budget.p95())
                && workload.maximum_ms <= milliseconds(budget.hard_max())
                && workload.peak_allocated_bytes <= PHASE3_1_BOUNDED_OCR_HEAP_LIMIT_BYTES
        })
        && if report.integrated_zones {
            report.zone_workloads.len() == budgets.grouped_latency.len()
                && report.zone_workloads.iter().all(|workload| {
                    budgets
                        .grouped_latency
                        .iter()
                        .filter(|budget| budget.workload() == workload.name)
                        .count()
                        == 1
                })
                && budgets.grouped_latency.iter().all(|budget| {
                    let mut matching = report
                        .zone_workloads
                        .iter()
                        .filter(|workload| workload.name == budget.workload());
                    let Some(workload) = matching.next() else {
                        return false;
                    };
                    matching.next().is_none()
                        && workload.p50_ms <= milliseconds(budget.p50())
                        && workload.p95_ms <= milliseconds(budget.p95())
                        && workload.maximum_ms <= milliseconds(budget.hard_max())
                        && workload.peak_allocated_bytes <= PHASE3_1_BOUNDED_OCR_HEAP_LIMIT_BYTES
                })
                && report
                    .active_cancellation
                    .as_ref()
                    .is_some_and(|cancellation| {
                        cancellation.cancellation_to_return_ms
                            <= milliseconds(budgets.active_cancellation)
                    })
                && report.retained_result.as_ref().is_some_and(|retained| {
                    retained.elapsed_ms <= milliseconds(budgets.retained_result)
                })
        } else {
            true
        }
}

#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
fn target_budget_passes(report: &ProfileReport) -> bool {
    report_within_target_budgets(
        report,
        &TargetBudgets {
            latency: &PHASE3_1_APPLE_BOUNDED_OCR_LATENCY_BUDGETS,
            cold_load: PHASE3_1_APPLE_BOUNDED_OCR_COLD_LOAD_LIMIT,
            close: PHASE3_1_APPLE_BOUNDED_OCR_CLOSE_LIMIT,
            reopen_close: PHASE3_1_APPLE_BOUNDED_OCR_REOPEN_CLOSE_LIMIT,
            resident_bytes: PHASE3_1_APPLE_BOUNDED_OCR_RESIDENT_LIMIT_BYTES,
            grouped_latency: &PHASE3_1_APPLE_GROUPED_OCR_LATENCY_BUDGETS,
            active_cancellation: PHASE3_1_APPLE_GROUPED_OCR_CANCELLATION_LIMIT,
            retained_result: PHASE3_1_APPLE_GROUPED_OCR_RETAINED_RESULT_LIMIT,
        },
    )
}

#[cfg(all(target_arch = "x86_64", target_os = "windows", target_env = "msvc"))]
fn target_budget_passes(report: &ProfileReport) -> bool {
    report_within_target_budgets(
        report,
        &TargetBudgets {
            latency: &PHASE3_1_WINDOWS_BOUNDED_OCR_LATENCY_BUDGETS,
            cold_load: PHASE3_1_WINDOWS_BOUNDED_OCR_COLD_LOAD_LIMIT,
            close: PHASE3_1_WINDOWS_BOUNDED_OCR_CLOSE_LIMIT,
            reopen_close: PHASE3_1_WINDOWS_BOUNDED_OCR_REOPEN_CLOSE_LIMIT,
            resident_bytes: PHASE3_1_WINDOWS_BOUNDED_OCR_RESIDENT_LIMIT_BYTES,
            grouped_latency: &PHASE3_1_WINDOWS_GROUPED_OCR_LATENCY_BUDGETS,
            active_cancellation: PHASE3_1_WINDOWS_GROUPED_OCR_CANCELLATION_LIMIT,
            retained_result: PHASE3_1_WINDOWS_GROUPED_OCR_RETAINED_RESULT_LIMIT,
        },
    )
}

#[cfg(not(any(
    all(target_arch = "aarch64", target_os = "macos"),
    all(target_arch = "x86_64", target_os = "windows", target_env = "msvc")
)))]
fn target_budget_passes(_report: &ProfileReport) -> bool {
    false
}

#[cfg(any(
    all(target_arch = "aarch64", target_os = "macos"),
    all(target_arch = "x86_64", target_os = "windows", target_env = "msvc")
))]
fn require_release_target() {}

#[cfg(not(any(
    all(target_arch = "aarch64", target_os = "macos"),
    all(target_arch = "x86_64", target_os = "windows", target_env = "msvc")
)))]
fn require_release_target() {
    panic!("precursor requires one release target");
}

fn open_profile(
    profile: OnnxOcrProfile,
    model_root: &Path,
    runtime: &Path,
    operation: &OperationContext,
) -> Result<OnnxOcrBackend, OnnxBackendFault> {
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
        OnnxOcrProfile::BoundedDetector => "bounded-detector-fit1312x736-then-tensor6291456b",
    }
}

fn build_workload(root: &Path, manifest: &FixtureManifest, kind: WorkloadKind) -> WorkloadSpec {
    match kind {
        WorkloadKind::Hud4k => {
            let hud = image(manifest, "hud.png");
            let hud_mat = load_bgra(&root.join("hud.png"), &hud.sha256);
            let hud_4k_mat = resize_bgra(&hud_mat, 3_840, 2_160, INTER_NEAREST_EXACT);
            full_workload(
                "bounded_hud_4k",
                frame_from_mat(&hud_4k_mat),
                expected_regions(hud, 4.0, 4.0, 0.0, 0.0),
                DetectorDimensions {
                    width: 3_840,
                    height: 2_176,
                },
                DetectorDimensions {
                    width: 960,
                    height: 512,
                },
            )
        }
        WorkloadKind::MenuWide => {
            let menu = image(manifest, "menu.png");
            let menu_mat = load_bgra(&root.join("menu.png"), &menu.sha256);
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
            )
        }
        WorkloadKind::StatusExtremeWide => {
            let status = image(manifest, "status.png");
            let status_mat = load_bgra(&root.join("status.png"), &status.sha256);
            let status_wide_mat = resize_bgra(&status_mat, 569, 320, INTER_LINEAR);
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
            )
        }
        WorkloadKind::HudReference => {
            let hud = image(manifest, "hud.png");
            let hud_mat = load_bgra(&root.join("hud.png"), &hud.sha256);
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
            )
        }
        WorkloadKind::HudOdd => {
            let hud = image(manifest, "hud.png");
            let hud_mat = load_bgra(&root.join("hud.png"), &hud.sha256);
            let hud_odd_mat = resize_bgra(&hud_mat, 1_001, 563, INTER_LINEAR);
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
            )
        }
        WorkloadKind::TooltipDense => {
            let tooltip = image(manifest, "tooltip-v3.png");
            let tooltip_mat = load_bgra(&root.join("tooltip-v3.png"), &tooltip.sha256);
            full_workload(
                "bounded_tooltip_dense",
                frame_from_mat(&tooltip_mat),
                expected_regions(tooltip, 1.0, 1.0, 0.0, 0.0),
                DetectorDimensions {
                    width: 1_472,
                    height: 736,
                },
                DetectorDimensions {
                    width: 1_024,
                    height: 480,
                },
            )
        }
        WorkloadKind::MissionBoundary => {
            let mission = image(manifest, "mission.png");
            let mission_mat = load_bgra(&root.join("mission.png"), &mission.sha256);
            let mission_region =
                PixelRect::new(877, 0, 1_440, 720).expect("mission qualification region");
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
            }
        }
        WorkloadKind::Blank4k => full_workload(
            "bounded_blank_4k",
            opaque_black_frame(3_840, 2_160),
            Vec::new(),
            DetectorDimensions {
                width: 3_840,
                height: 2_176,
            },
            DetectorDimensions {
                width: 960,
                height: 512,
            },
        ),
    }
}

fn build_zone_workload(
    root: &Path,
    manifest: &FixtureManifest,
    kind: ZoneWorkloadKind,
) -> ZoneWorkloadSpec {
    match kind {
        ZoneWorkloadKind::OneFull => {
            let base = build_workload(root, manifest, WorkloadKind::HudReference);
            assert_eq!(base.expected.len(), 8, "HUD oracle count");
            zone_workload(
                "zone_one_full",
                base.frame,
                &[(0, 0, 960, 540)],
                base.expected.iter().cloned().collect(),
                vec![(0..8).collect()],
                0,
            )
        }
        ZoneWorkloadKind::ThreeSparse => {
            let base = build_workload(root, manifest, WorkloadKind::HudReference);
            assert_eq!(base.expected.len(), 8, "HUD oracle count");
            let expected = [0, 1, 2, 3, 4, 6]
                .map(|index| base.expected[index].clone())
                .into_iter()
                .collect();
            zone_workload(
                "zone_three_sparse",
                base.frame,
                &[(40, 45, 350, 235), (680, 45, 900, 235), (40, 270, 260, 475)],
                expected,
                vec![vec![0, 2], vec![1, 3], vec![4, 5]],
                2,
            )
        }
        ZoneWorkloadKind::EightDistinct => {
            let base = build_workload(root, manifest, WorkloadKind::HudReference);
            assert_eq!(base.expected.len(), 8, "HUD oracle count");
            zone_workload(
                "zone_eight_distinct",
                base.frame,
                &[
                    (40, 45, 230, 120),
                    (680, 45, 880, 120),
                    (40, 150, 350, 235),
                    (680, 150, 900, 235),
                    (40, 270, 260, 350),
                    (700, 270, 880, 350),
                    (40, 395, 190, 475),
                    (680, 395, 880, 475),
                ],
                base.expected.iter().cloned().collect(),
                (0..8).map(|index| vec![index]).collect(),
                0,
            )
        }
        ZoneWorkloadKind::DenseUnique => {
            let base = build_workload(root, manifest, WorkloadKind::TooltipDense);
            assert_eq!(base.expected.len(), 11, "dense tooltip oracle count");
            zone_workload(
                "zone_dense_unique",
                base.frame,
                &[(0, 0, 1_440, 720)],
                base.expected.iter().cloned().collect(),
                vec![(0..11).collect()],
                0,
            )
        }
        ZoneWorkloadKind::Empty4k => {
            let base = build_workload(root, manifest, WorkloadKind::Blank4k);
            zone_workload(
                "zone_empty_4k",
                base.frame,
                &[(0, 0, 3_840, 2_160)],
                Vec::new(),
                vec![Vec::new()],
                0,
            )
        }
    }
}

fn zone_workload(
    name: &'static str,
    frame: Frame,
    rectangles: &[(i32, i32, i32, i32)],
    expected: Vec<ExpectedRegion>,
    expected_groups: Vec<Vec<usize>>,
    expected_ignored_candidates: usize,
) -> ZoneWorkloadSpec {
    assert_eq!(
        rectangles.len(),
        expected_groups.len(),
        "every zone has one explicit group oracle"
    );
    let effective_zones = rectangles
        .iter()
        .map(|&(left, top, right, bottom)| {
            PixelRect::new(left, top, right, bottom).expect("predeclared zone is valid")
        })
        .collect::<Vec<_>>();
    let source_envelope = effective_zones
        .iter()
        .copied()
        .reduce(|left, right| {
            PixelRect::new(
                left.left().min(right.left()),
                left.top().min(right.top()),
                left.right().max(right.right()),
                left.bottom().max(right.bottom()),
            )
            .expect("zone envelope is valid")
        })
        .expect("integrated workload has at least one zone");
    let zones = rectangles
        .iter()
        .map(|&(left, top, right, bottom)| {
            OcrZone::new(
                Rect::new(
                    CoordinateSpace::CapturePixels,
                    f64::from(left),
                    f64::from(top),
                    f64::from(right),
                    f64::from(bottom),
                )
                .expect("predeclared zone rectangle is valid"),
                ClipPolicy::Reject,
            )
        })
        .collect::<Vec<_>>();
    assert!(
        expected_groups
            .iter()
            .flatten()
            .all(|&index| index < expected.len()),
        "group oracle indexes one unique candidate"
    );
    ZoneWorkloadSpec {
        name,
        frame,
        zones: zones.into(),
        effective_zones: effective_zones.into(),
        source_envelope,
        expected: expected.into(),
        expected_groups: expected_groups
            .into_iter()
            .map(Arc::<[usize]>::from)
            .collect::<Vec<_>>()
            .into(),
        expected_ignored_candidates,
    }
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

fn load_bgra(path: &Path, expected_sha256: &str) -> Mat {
    assert_eq!(
        digest_file(path),
        expected_sha256,
        "tracked fixture bytes drifted from their manifest"
    );
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
