//! Explicit native-runtime conformance and accepted-fixture smoke coverage.

use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::{OnnxBackendFault, OnnxOcrBackend, OnnxOcrProfile};
use mado_pilot_capture::PixelFormat;
use mado_pilot_core::{CancellationToken, CoordinateSpace, OperationContext, PixelExtent, Status};
use mado_pilot_ocr::{OcrBackend, OcrModelIdentity, OcrRecognizer, OcrRegion, OcrRequest};
use mado_pilot_testkit::{ocr_contract, vision_contract};
use opencv::core::{Mat, MatTraitConst, MatTraitConstManual};
use opencv::imgcodecs::{IMREAD_COLOR, imread};
use opencv::imgproc::{COLOR_BGR2BGRA, cvt_color_def};

const RUNTIME_ENV: &str = "MADO_PILOT_ONNX_RUNTIME";
const MODEL_ROOT_ENV: &str = "MADO_PILOT_G004_MODEL_ROOT";

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
    }
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

fn assert_hud(actual: &mado_pilot_ocr::OcrResult) {
    assert_eq!(actual.regions().len(), HUD_TEXT.len());
    for (index, region) in actual.regions().iter().enumerate() {
        assert_eq!(region.text(), HUD_TEXT[index]);
        let points = region.geometry().points();
        for (point, (expected_x, expected_y)) in points.into_iter().zip(HUD_QUADS[index]) {
            assert!(
                (point.x() - expected_x).abs() <= HUD_POINT_TOLERANCE
                    && (point.y() - expected_y).abs() <= HUD_POINT_TOLERANCE,
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
