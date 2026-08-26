//! Explicit native-runtime conformance and accepted-fixture smoke coverage.

use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(any(target_os = "macos", feature = "coreml-provider"))]
use crate::OnnxProviderFallbackReason;
use crate::{OnnxBackendFault, OnnxBackendObservations, OnnxOcrBackend, OnnxOcrProfile};
#[cfg(any(
    target_os = "macos",
    feature = "coreml-provider",
    all(
        target_os = "windows",
        target_arch = "x86_64",
        target_env = "msvc",
        feature = "cuda-provider"
    )
))]
use crate::{OnnxExecutionProvider, OnnxExecutionProviderPolicy};
use mado_pilot_capture::PixelFormat;
use mado_pilot_core::{
    CancellationToken, ClipPolicy, CoordinateSpace, OperationContext, PixelExtent, Rect, Status,
};
use mado_pilot_ocr::{
    OcrBackend, OcrModelIdentity, OcrRecognizer, OcrRegion, OcrRequest, OcrZone,
    OcrZoneScanRequest, OcrZoneScanResult,
};
use mado_pilot_testkit::{
    ManualClock,
    bench_harness::{self, Plan, Sample},
    ocr_contract, vision_contract,
};
use opencv::core::{Mat, MatTraitConst, MatTraitConstManual};
use opencv::imgcodecs::{IMREAD_COLOR, imread};
use opencv::imgproc::{COLOR_BGR2BGRA, cvt_color_def};

const RUNTIME_ENV: &str = "MADO_PILOT_ONNX_RUNTIME";
const MODEL_ROOT_ENV: &str = "MADO_PILOT_G004_MODEL_ROOT";
#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
    target_env = "msvc",
    feature = "cuda-provider"
))]
const CUDA_PROVIDER_ROOT_ENV: &str = "MADO_PILOT_CUDA_PROVIDER_ROOT";

const HUD_TEXT: [&str; 8] = [
    "魔導士",
    "Lv.42",
    "HP1234/5678",
    "MP98%",
    "クエスト",
    "[A-7]",
    "次へ>",
    "READY!",
];
const HUD_QUADS: [[(f64, f64); 4]; 8] = [
    [(58.0, 61.0), (200.0, 61.0), (200.0, 112.0), (58.0, 112.0)],
    [(708.0, 67.0), (809.0, 67.0), (809.0, 109.0), (708.0, 109.0)],
    [(59.0, 176.0), (315.0, 176.0), (315.0, 216.0), (59.0, 216.0)],
    [
        (721.0, 179.0),
        (853.0, 179.0),
        (853.0, 214.0),
        (721.0, 214.0),
    ],
    [(60.0, 292.0), (228.0, 294.0), (227.0, 336.0), (59.0, 334.0)],
    [
        (737.0, 293.0),
        (819.0, 295.0),
        (818.0, 339.0),
        (736.0, 337.0),
    ],
    [(54.0, 413.0), (166.0, 413.0), (166.0, 459.0), (54.0, 459.0)],
    [
        (695.0, 420.0),
        (811.0, 420.0),
        (811.0, 453.0),
        (695.0, 453.0),
    ],
];
const HUD_EIGHT_ZONES: [(f64, f64, f64, f64); 8] = [
    (40.0, 45.0, 230.0, 120.0),
    (680.0, 45.0, 880.0, 120.0),
    (40.0, 150.0, 350.0, 235.0),
    (680.0, 150.0, 900.0, 235.0),
    (40.0, 270.0, 260.0, 350.0),
    (700.0, 270.0, 880.0, 350.0),
    (40.0, 395.0, 190.0, 475.0),
    (680.0, 395.0, 880.0, 475.0),
];
const HUD_THREE_ZONES: [(f64, f64, f64, f64); 3] = [
    (40.0, 45.0, 350.0, 235.0),
    (680.0, 45.0, 900.0, 235.0),
    (40.0, 270.0, 260.0, 475.0),
];
const HUD_POINT_TOLERANCE: f64 = 0.0;
const NATIVE_TERMINATION_BOUND: Duration = Duration::from_millis(250);
const NATIVE_GATE_BOUND: Duration = Duration::from_secs(2);

#[test]
#[ignore = "requires explicit reviewed ONNX Runtime and G-004 model paths"]
fn accepted_runtime_passes_contract_and_hud_oracle() {
    let runtime = required_path(RUNTIME_ENV);
    let model_root = required_path(MODEL_ROOT_ENV);
    let operation = OperationContext::new();

    for (identity, profile) in [
        (
            OcrModelIdentity::accepted_g004(),
            OnnxOcrProfile::NativeG004,
        ),
        (
            OcrModelIdentity::accepted_bounded_detector(),
            OnnxOcrProfile::BoundedDetector,
        ),
    ] {
        let opened = open_profile(profile, &model_root, &runtime, &operation)
            .expect("accepted backend opens");
        assert_eq!(opened.descriptor().model_identity(), &identity);
        assert_eq!(opened.facts().profile(), profile);
        let backend: Arc<dyn OcrBackend> = Arc::new(opened);
        ocr_contract::run(&backend);
        drop(backend);

        let opened = open_profile(profile, &model_root, &runtime, &operation)
            .expect("backend reopens after close");
        assert_eq!(opened.descriptor().model_identity(), &identity);
        assert_eq!(opened.facts().profile(), profile);
        let backend: Arc<dyn OcrBackend> = Arc::new(opened);
        let first = recognize_hud(Arc::clone(&backend), &operation);
        assert_hud(&first);
        for _ in 1..3 {
            let repeated = recognize_hud(Arc::clone(&backend), &operation);
            assert_hud(&repeated);
            assert_confidence_stable(&first, &repeated);
        }

        let cancellation_gate = crate::inference::test_hook::install();
        let token = CancellationToken::new();
        let worker_token = token.clone();
        let worker_backend = Arc::clone(&backend);
        let (cancelled_sender, cancelled_receiver) = mpsc::sync_channel(1);
        let cancelled = thread::spawn(move || {
            let context = OperationContext::new().with_cancellation(worker_token);
            let status = recognize_hud_result(worker_backend, &context)
                .expect_err("cancelled inference cannot publish")
                .status();
            cancelled_sender
                .send(status)
                .expect("cancellation result receiver remains live");
        });
        assert!(
            cancellation_gate.wait_until_admitted(NATIVE_GATE_BOUND),
            "native run admission timed out"
        );
        cancellation_gate.release();
        assert!(
            cancellation_gate.wait_until_run_started(NATIVE_GATE_BOUND),
            "native run start timed out"
        );
        let cancellation_started = Instant::now();
        token.cancel();
        assert!(
            cancellation_gate.wait_until_termination_issued(NATIVE_TERMINATION_BOUND),
            "native termination was not issued within the explicit test bound"
        );
        let remaining = NATIVE_TERMINATION_BOUND.saturating_sub(cancellation_started.elapsed());
        assert_eq!(
            cancelled_receiver
                .recv_timeout(remaining)
                .expect("cancelled native run returns within the explicit test bound"),
            Status::Cancelled
        );
        cancelled.join().expect("cancellation worker joins");
        drop(cancellation_gate);
        let after_cancellation = recognize_hud(Arc::clone(&backend), &operation);
        assert_hud(&after_cancellation);
        assert_confidence_stable(&first, &after_cancellation);

        let close_gate = crate::inference::test_hook::install();
        let worker_backend = Arc::clone(&backend);
        let (in_flight_sender, in_flight_receiver) = mpsc::sync_channel(1);
        let in_flight = thread::spawn(move || {
            in_flight_sender
                .send(recognize_hud(worker_backend, &OperationContext::new()))
                .expect("in-flight result receiver remains live");
        });
        assert!(
            close_gate.wait_until_admitted(NATIVE_GATE_BOUND),
            "close-race admission timed out"
        );
        assert_eq!(
            backend
                .close(&operation)
                .expect_err("close does not tear down admitted work")
                .status(),
            Status::LimitExceeded
        );
        close_gate.release();
        let raced = in_flight_receiver
            .recv_timeout(NATIVE_GATE_BOUND)
            .expect("in-flight run returns within the native test bound");
        in_flight.join().expect("inference worker joins");
        drop(close_gate);
        assert_hud(&raced);
        assert_confidence_stable(&first, &raced);
        backend.close(&operation).expect("native backend closes");
        backend
            .close(&operation)
            .expect("repeated close is idempotent");
        if profile == OnnxOcrProfile::BoundedDetector {
            drop(backend);
            let grouped = Arc::new(
                open_profile(profile, &model_root, &runtime, &operation)
                    .expect("bounded grouped backend reopens"),
            );
            let retained = assert_grouped_hud_contract(&grouped, &operation);
            grouped.close(&operation).expect("grouped backend closes");
            grouped
                .close(&operation)
                .expect("grouped backend repeated close is idempotent");
            assert_eq!(
                retained.group(0).unwrap().get(0).unwrap().text(),
                HUD_TEXT[0]
            );
        }
    }
}

#[cfg(feature = "coreml-provider")]
#[test]
#[ignore = "requires explicit reviewed ONNX Runtime and G-004 model paths"]
fn rejected_coreml_preference_reports_fresh_cpu_fallback() {
    let runtime = required_path(RUNTIME_ENV);
    let model_root = required_path(MODEL_ROOT_ENV);
    let operation = OperationContext::new();
    let backend = Arc::new(
        OnnxOcrBackend::open_accepted_with_provider_policy(
            &model_root,
            &runtime,
            OnnxExecutionProviderPolicy::PreferCoreMl,
            &operation,
        )
        .expect("rejected preferred CoreML constructs a fresh CPU pair"),
    );
    let facts = backend.facts();
    assert_eq!(facts.provider(), OnnxExecutionProvider::Cpu);
    assert_eq!(
        facts.requested_provider_policy(),
        OnnxExecutionProviderPolicy::PreferCoreMl
    );
    assert!(facts.initialization_fell_back());
    assert_eq!(
        facts.fallback_reason(),
        Some(OnnxProviderFallbackReason::QualificationRejected)
    );
    let result = recognize_hud(backend.clone(), &operation);
    assert_hud(&result);
    backend
        .close(&operation)
        .expect("fallback CPU backend closes");
}
#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
    target_env = "msvc",
    feature = "cuda-provider"
))]
#[test]
#[ignore = "requires explicit reviewed ONNX Runtime, CUDA provider, and G-004 model paths"]
fn cuda_provider_opens_without_cpu_initialization_fallback() {
    let runtime = required_path(RUNTIME_ENV);
    let provider_root = required_path(CUDA_PROVIDER_ROOT_ENV);
    let model_root = required_path(MODEL_ROOT_ENV);
    let operation = OperationContext::new();
    let backend = Arc::new(
        OnnxOcrBackend::open_accepted_with_provider_config(
            &model_root,
            &runtime,
            OnnxExecutionProviderPolicy::RequireCuda,
            Some(&provider_root),
            &operation,
        )
        .expect("controlled CUDA provider opens"),
    );
    let facts = backend.facts();
    assert_eq!(facts.provider(), OnnxExecutionProvider::Cuda);
    assert_eq!(
        facts.requested_provider_policy(),
        OnnxExecutionProviderPolicy::RequireCuda
    );
    assert!(!facts.initialization_fell_back());
    assert_eq!(facts.fallback_reason(), None);
    let result = recognize_hud(backend.clone(), &operation);
    assert_hud_with_tolerance(&result, 960.0 * 0.025, 540.0 * 0.025);
    assert_provider_survives_cancelled_inference(
        &backend,
        OnnxExecutionProvider::Cuda,
        &operation,
        960.0 * 0.025,
        540.0 * 0.025,
    );
    backend.close(&operation).expect("CUDA backend closes");
}

#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
    target_env = "msvc",
    feature = "cuda-provider"
))]
fn assert_provider_survives_cancelled_inference(
    backend: &Arc<OnnxOcrBackend>,
    expected: OnnxExecutionProvider,
    operation: &OperationContext,
    x_tolerance: f64,
    y_tolerance: f64,
) {
    let gate = crate::inference::test_hook::install();
    let token = CancellationToken::new();
    let worker_token = token.clone();
    let worker_backend: Arc<dyn OcrBackend> = backend.clone();
    let (sender, receiver) = mpsc::sync_channel(1);
    let cancelled = thread::spawn(move || {
        let context = OperationContext::new().with_cancellation(worker_token);
        sender
            .send(
                recognize_hud_result(worker_backend, &context)
                    .expect_err("cancelled provider inference cannot publish")
                    .status(),
            )
            .expect("provider cancellation receiver remains live");
    });
    assert!(
        gate.wait_until_admitted(NATIVE_GATE_BOUND),
        "provider run admission timed out"
    );
    gate.release();
    assert!(
        gate.wait_until_run_started(NATIVE_GATE_BOUND),
        "provider run start timed out"
    );
    token.cancel();
    assert!(
        gate.wait_until_termination_issued(NATIVE_TERMINATION_BOUND),
        "provider termination was not issued within the explicit bound"
    );
    assert_eq!(
        receiver
            .recv_timeout(NATIVE_TERMINATION_BOUND)
            .expect("cancelled provider inference returns"),
        Status::Cancelled
    );
    cancelled
        .join()
        .expect("provider cancellation worker joins");
    drop(gate);

    assert_eq!(backend.facts().provider(), expected);
    assert!(!backend.facts().initialization_fell_back());
    let after = recognize_hud(backend.clone(), operation);
    assert_hud_with_tolerance(&after, x_tolerance, y_tolerance);
    assert_eq!(backend.facts().provider(), expected);
}

#[cfg(all(target_os = "macos", not(feature = "coreml-provider")))]
#[test]
#[ignore = "requires explicit reviewed ONNX Runtime and G-004 model paths"]
fn preferred_coreml_without_build_capability_falls_back_to_cpu() {
    let runtime = required_path(RUNTIME_ENV);
    let model_root = required_path(MODEL_ROOT_ENV);
    let operation = OperationContext::new();
    let backend = Arc::new(
        OnnxOcrBackend::open_accepted_with_provider_policy(
            &model_root,
            &runtime,
            OnnxExecutionProviderPolicy::PreferCoreMl,
            &operation,
        )
        .expect("preferred CoreML falls back to a fresh CPU pair"),
    );
    let facts = backend.facts();
    assert_eq!(facts.provider(), OnnxExecutionProvider::Cpu);
    assert!(facts.initialization_fell_back());
    assert_eq!(
        facts.fallback_reason(),
        Some(OnnxProviderFallbackReason::BuildCapabilityUnavailable)
    );
    let result = recognize_hud(backend.clone(), &operation);
    assert_hud(&result);
    backend
        .close(&operation)
        .expect("fallback CPU backend closes");
}

#[cfg(all(target_os = "macos", not(feature = "coreml-provider")))]
#[test]
#[ignore = "requires explicit reviewed ONNX Runtime and G-004 model paths"]
fn required_coreml_without_build_capability_publishes_nothing() {
    let runtime = required_path(RUNTIME_ENV);
    let model_root = required_path(MODEL_ROOT_ENV);
    let operation = OperationContext::new();
    let fault = OnnxOcrBackend::open_accepted_with_provider_policy(
        &model_root,
        &runtime,
        OnnxExecutionProviderPolicy::RequireCoreMl,
        &operation,
    )
    .expect_err("required CoreML cannot fall back");
    assert_eq!(fault.fault(), OnnxBackendFault::ProviderUnavailable);
}

fn recognize_hud(
    backend: Arc<dyn OcrBackend>,
    operation: &OperationContext,
) -> mado_pilot_ocr::OcrResult {
    recognize_hud_result(backend, operation).expect("accepted HUD fixture recognizes")
}

fn recognize_hud_result(
    backend: Arc<dyn OcrBackend>,
    operation: &OperationContext,
) -> mado_pilot_core::Result<mado_pilot_ocr::OcrResult> {
    let recognizer = OcrRecognizer::new(backend);
    let descriptor = recognizer.descriptor();
    let frame = hud_frame();
    recognizer.recognize(OcrRequest::new(
        &frame,
        descriptor.backend_identity(),
        descriptor.model_identity(),
        OcrRegion::FullFrame,
        CoordinateSpace::CapturePixels,
        operation,
    ))
}

fn assert_grouped_hud_contract(
    backend: &Arc<OnnxOcrBackend>,
    operation: &OperationContext,
) -> OcrZoneScanResult {
    let singular_port: Arc<dyn OcrBackend> = backend.clone();
    let singular = recognize_hud(singular_port, operation);

    let full = [ocr_zone((0.0, 0.0, 960.0, 540.0))];
    let before = backend
        .observations()
        .expect("grouped observations available");
    let one = scan_hud_zones(backend, &full, operation).expect("one-zone scan succeeds");
    let after = backend
        .observations()
        .expect("grouped observations available");
    assert_eq!(one.unique_candidates(), singular.regions());
    assert_eq!(
        one.group(0).unwrap().iter().collect::<Vec<_>>(),
        singular.regions().iter().collect::<Vec<_>>()
    );
    assert_grouped_observation_delta(before, after, 960, 540, 8, 0, 8, 2);

    let overlap = [full[0]; 2];
    let before = after;
    let overlap_result =
        scan_hud_zones(backend, &overlap, operation).expect("overlap scan succeeds safely");
    let after = backend
        .observations()
        .expect("grouped observations available");
    for index in 0..HUD_TEXT.len() {
        assert!(std::ptr::eq(
            overlap_result.group(0).unwrap().get(index).unwrap(),
            overlap_result.group(1).unwrap().get(index).unwrap(),
        ));
    }
    assert_grouped_observation_delta(before, after, 960, 540, 8, 0, 16, 2);

    let three = HUD_THREE_ZONES.map(ocr_zone);
    let before = after;
    let three_result =
        scan_hud_zones(backend, &three, operation).expect("three-zone scan succeeds");
    let after = backend
        .observations()
        .expect("grouped observations available");
    assert_group_texts(
        &three_result,
        &[
            &[HUD_TEXT[0], HUD_TEXT[2]],
            &[HUD_TEXT[1], HUD_TEXT[3]],
            &[HUD_TEXT[4], HUD_TEXT[6]],
        ],
    );
    assert_grouped_observation_delta(before, after, 860, 430, 6, 2, 6, 1);

    let eight = HUD_EIGHT_ZONES.map(ocr_zone);
    let before = after;
    let eight_result =
        scan_hud_zones(backend, &eight, operation).expect("eight-zone scan succeeds");
    let after = backend
        .observations()
        .expect("grouped observations available");
    let expected = HUD_TEXT.map(|text| [text]);
    let expected = expected.iter().map(|group| &group[..]).collect::<Vec<_>>();
    assert_group_texts(&eight_result, &expected);
    assert_grouped_observation_delta(before, after, 860, 430, 8, 0, 8, 2);

    let cancellation_gate = crate::inference::test_hook::install();
    let token = CancellationToken::new();
    let worker_token = token.clone();
    let worker_backend = backend.clone();
    let worker_zones = eight;
    let (cancelled_sender, cancelled_receiver) = mpsc::sync_channel(1);
    let cancelled = thread::spawn(move || {
        let context = OperationContext::new().with_cancellation(worker_token);
        let status = scan_hud_zones(&worker_backend, &worker_zones, &context)
            .expect_err("cancelled grouped inference cannot publish")
            .status();
        cancelled_sender
            .send(status)
            .expect("grouped cancellation receiver remains live");
    });
    assert!(
        cancellation_gate.wait_until_admitted(NATIVE_GATE_BOUND),
        "grouped native run admission timed out"
    );
    cancellation_gate.release();
    assert!(
        cancellation_gate.wait_until_run_started(NATIVE_GATE_BOUND),
        "grouped native run start timed out"
    );
    let cancellation_started = Instant::now();
    token.cancel();
    assert!(
        cancellation_gate.wait_until_termination_issued(NATIVE_TERMINATION_BOUND),
        "grouped native termination was not issued within the explicit test bound"
    );
    let remaining = NATIVE_TERMINATION_BOUND.saturating_sub(cancellation_started.elapsed());
    assert_eq!(
        cancelled_receiver
            .recv_timeout(remaining)
            .expect("cancelled grouped run returns within the explicit test bound"),
        Status::Cancelled
    );
    cancelled.join().expect("grouped cancellation worker joins");
    drop(cancellation_gate);
    let cancelled_observations = backend
        .observations()
        .expect("observations after cancellation");
    assert_eq!(
        cancelled_observations.cleanup_completions() - after.cleanup_completions(),
        1
    );

    let deadline_gate = crate::inference::test_hook::install();
    let deadline_clock = Arc::new(ManualClock::new());
    let worker_clock = deadline_clock.clone();
    let worker_backend = backend.clone();
    let worker_zones = eight;
    let (deadline_sender, deadline_receiver) = mpsc::sync_channel(1);
    let deadline = thread::spawn(move || {
        let context = OperationContext::new()
            .with_clock(worker_clock)
            .with_timeout(Duration::from_millis(10))
            .unwrap();
        let status = scan_hud_zones(&worker_backend, &worker_zones, &context)
            .expect_err("expired grouped inference cannot publish")
            .status();
        deadline_sender
            .send(status)
            .expect("grouped deadline receiver remains live");
    });
    assert!(
        deadline_gate.wait_until_admitted(NATIVE_GATE_BOUND),
        "grouped deadline admission timed out"
    );
    deadline_clock.advance(Duration::from_millis(10));
    assert!(
        deadline_gate.wait_until_termination_issued(NATIVE_GATE_BOUND),
        "grouped deadline termination timed out"
    );
    deadline_gate.release();
    assert_eq!(
        deadline_receiver
            .recv_timeout(NATIVE_GATE_BOUND)
            .expect("deadline grouped run returns"),
        Status::DeadlineExceeded
    );
    deadline.join().expect("grouped deadline worker joins");
    drop(deadline_gate);

    let close_gate = crate::inference::test_hook::install();
    let worker_backend = backend.clone();
    let worker_zones = eight;
    let (raced_sender, raced_receiver) = mpsc::sync_channel(1);
    let raced = thread::spawn(move || {
        raced_sender
            .send(scan_hud_zones(
                &worker_backend,
                &worker_zones,
                &OperationContext::new(),
            ))
            .expect("grouped close-race receiver remains live");
    });
    assert!(
        close_gate.wait_until_admitted(NATIVE_GATE_BOUND),
        "grouped close-race admission timed out"
    );
    assert_eq!(
        backend
            .close(operation)
            .expect_err("grouped close does not tear down admitted work")
            .status(),
        Status::LimitExceeded
    );
    close_gate.release();
    let raced_result = raced_receiver
        .recv_timeout(NATIVE_GATE_BOUND)
        .expect("grouped close-race run returns")
        .expect("admitted grouped run completes");
    raced.join().expect("grouped close-race worker joins");
    drop(close_gate);
    assert_group_texts(&raced_result, &expected);

    let recovered = scan_hud_zones(backend, &three, operation).expect("grouped backend recovers");
    assert_group_texts(
        &recovered,
        &[
            &[HUD_TEXT[0], HUD_TEXT[2]],
            &[HUD_TEXT[1], HUD_TEXT[3]],
            &[HUD_TEXT[4], HUD_TEXT[6]],
        ],
    );

    let workload = bench_harness::measure(
        "ocr_zone_grouped_scan_allocation_growth",
        "three caller-order groups remain exact and release per-call Rust storage",
        Plan::new(2, 10),
        || GroupedAllocationFixture {
            backend: backend.clone(),
            frame: hud_frame(),
            zones: three,
        },
        measure_grouped_allocation,
    );
    assert_eq!(workload.incorrect(), 0);
    assert!(workload.growth_bytes() <= 4_096);
    assert_eq!(
        workload.mapped_bytes_per_result(),
        u64::from(860_u32) * u64::from(430_u32) * 4
    );
    recovered
}

fn ocr_zone((left, top, right, bottom): (f64, f64, f64, f64)) -> OcrZone {
    OcrZone::new(
        Rect::new(CoordinateSpace::CapturePixels, left, top, right, bottom).unwrap(),
        ClipPolicy::Reject,
    )
}

fn scan_hud_zones(
    backend: &Arc<OnnxOcrBackend>,
    zones: &[OcrZone],
    operation: &OperationContext,
) -> mado_pilot_core::Result<OcrZoneScanResult> {
    let frame = hud_frame();
    scan_frame_zones(backend, &frame, zones, operation)
}

fn scan_frame_zones(
    backend: &Arc<OnnxOcrBackend>,
    frame: &mado_pilot_capture::Frame,
    zones: &[OcrZone],
    operation: &OperationContext,
) -> mado_pilot_core::Result<OcrZoneScanResult> {
    let port: Arc<dyn OcrBackend> = backend.clone();
    let recognizer = OcrRecognizer::new(port);
    let descriptor = recognizer.descriptor();
    recognizer.scan_zones(OcrZoneScanRequest::new(
        frame,
        descriptor.backend_identity(),
        descriptor.model_identity(),
        zones,
        CoordinateSpace::CapturePixels,
        operation,
    )?)
}

#[derive(Debug)]
struct GroupedAllocationFixture {
    backend: Arc<OnnxOcrBackend>,
    frame: mado_pilot_capture::Frame,
    zones: [OcrZone; 3],
}

fn measure_grouped_allocation(fixture: &GroupedAllocationFixture) -> Sample {
    let started = Instant::now();
    let result = scan_frame_zones(
        &fixture.backend,
        &fixture.frame,
        &fixture.zones,
        &OperationContext::new(),
    );
    let elapsed = started.elapsed();
    let correct = result.as_ref().is_ok_and(|result| {
        group_matches(result, 0, &[HUD_TEXT[0], HUD_TEXT[2]])
            && group_matches(result, 1, &[HUD_TEXT[1], HUD_TEXT[3]])
            && group_matches(result, 2, &[HUD_TEXT[4], HUD_TEXT[6]])
    });
    Sample::new(
        elapsed,
        correct,
        u64::from(860_u32) * u64::from(430_u32) * 4,
    )
}

fn group_matches(actual: &OcrZoneScanResult, group: usize, expected: &[&str]) -> bool {
    actual.group(group).is_some_and(|actual| {
        actual.len() == expected.len()
            && actual
                .iter()
                .zip(expected)
                .all(|(actual, expected)| actual.text() == *expected)
    })
}

fn assert_group_texts(actual: &OcrZoneScanResult, expected: &[&[&str]]) {
    assert_eq!(actual.effective_zones().len(), expected.len());
    for (group, expected) in expected.iter().enumerate() {
        assert_eq!(
            actual
                .group(group)
                .unwrap()
                .iter()
                .map(|region| region.text())
                .collect::<Vec<_>>(),
            *expected
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn assert_grouped_observation_delta(
    before: OnnxBackendObservations,
    after: OnnxBackendObservations,
    mapping_width: u32,
    mapping_height: u32,
    selected: u64,
    ignored: u64,
    memberships: u64,
    recognizer_runs: u64,
) {
    assert_eq!(after.mapping_calls() - before.mapping_calls(), 1);
    assert_eq!(after.latest_mapping_width(), Some(mapping_width));
    assert_eq!(after.latest_mapping_height(), Some(mapping_height));
    assert_eq!(
        after.mapped_bytes() - before.mapped_bytes(),
        u64::from(mapping_width) * u64::from(mapping_height) * 4
    );
    assert_eq!(after.detector_resizes() - before.detector_resizes(), 1);
    assert_eq!(after.detector_runs() - before.detector_runs(), 1);
    assert_eq!(
        after.recognizer_runs() - before.recognizer_runs(),
        recognizer_runs
    );
    assert_eq!(
        after.selected_candidates() - before.selected_candidates(),
        selected
    );
    assert_eq!(
        after.ignored_candidates() - before.ignored_candidates(),
        ignored
    );
    assert_eq!(
        after.unique_candidates() - before.unique_candidates(),
        selected
    );
    assert_eq!(after.memberships() - before.memberships(), memberships);
    assert_eq!(
        after.cleanup_completions() - before.cleanup_completions(),
        1
    );
    assert_eq!(after.session_pairs(), 1);
    assert_eq!(after.sessions(), 2);
}

fn assert_hud(actual: &mado_pilot_ocr::OcrResult) {
    assert_hud_with_tolerance(actual, HUD_POINT_TOLERANCE, HUD_POINT_TOLERANCE);
}

fn assert_hud_with_tolerance(
    actual: &mado_pilot_ocr::OcrResult,
    x_tolerance: f64,
    y_tolerance: f64,
) {
    assert_eq!(actual.regions().len(), HUD_TEXT.len());
    for (index, region) in actual.regions().iter().enumerate() {
        assert_eq!(region.text(), HUD_TEXT[index]);
        let points = region.geometry().points();
        for (point, (expected_x, expected_y)) in points.into_iter().zip(HUD_QUADS[index]) {
            assert!(
                (point.x() - expected_x).abs() <= x_tolerance
                    && (point.y() - expected_y).abs() <= y_tolerance,
                "geometry drift at detector order {index}: ({}, {}) versus ({expected_x}, {expected_y})",
                point.x(),
                point.y()
            );
        }
        let confidence = region.confidence().get();
        assert!(
            confidence.is_finite() && (0.0..=1.0).contains(&confidence),
            "invalid confidence at detector order {index}"
        );
    }
}

fn assert_confidence_stable(
    baseline: &mado_pilot_ocr::OcrResult,
    repeated: &mado_pilot_ocr::OcrResult,
) {
    let baseline = baseline
        .regions()
        .iter()
        .map(|region| region.confidence().get());
    let repeated = repeated
        .regions()
        .iter()
        .map(|region| region.confidence().get());
    assert!(baseline.eq(repeated), "same-host confidence drift");
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

fn hud_frame() -> mado_pilot_capture::Frame {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/ocr/g-004/hud.png");
    let bgr = imread(path.to_str().expect("UTF-8 fixture path"), IMREAD_COLOR)
        .expect("decode tracked HUD fixture");
    let mut bgra = Mat::default();
    cvt_color_def(&bgr, &mut bgra, COLOR_BGR2BGRA).expect("convert fixture to BGRA");
    assert!(bgra.is_continuous());
    let width = u32::try_from(bgra.cols()).expect("fixture width");
    let height = u32::try_from(bgra.rows()).expect("fixture height");
    let pixels = bgra.data_bytes().expect("fixture bytes").to_vec();
    vision_contract::frame_with_pixels(PixelExtent::new(width, height), PixelFormat::Bgra8, pixels)
}

fn required_path(variable: &str) -> PathBuf {
    let path = PathBuf::from(std::env::var_os(variable).expect("native test path is configured"));
    path.canonicalize()
        .expect("native test path is canonicalizable")
}
