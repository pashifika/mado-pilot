//! Focused cold-load, warm-inference, allocation-growth, and cleanup observations.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use mado_pilot_backend_onnx::{OnnxBackendFacts, OnnxOcrBackend};
use mado_pilot_capture::{Frame, PixelFormat};
use mado_pilot_core::{CoordinateSpace, OperationContext, PixelExtent};
use mado_pilot_ocr::{
    OcrBackend, OcrModelComponent, OcrModelIdentity, OcrModelSource, OcrModelSourceRequest,
    OcrRecognizer, OcrRegion, OcrRequest,
};
use mado_pilot_testkit::bench_harness::{self, Accounting, Plan, Sample};
use mado_pilot_testkit::vision_contract;
use opencv::core::{Mat, MatTraitConst, MatTraitConstManual};
use opencv::imgcodecs::{IMREAD_COLOR, imread};
use opencv::imgproc::{COLOR_BGR2BGRA, cvt_color_def};

#[global_allocator]
static ACCOUNTING: Accounting = Accounting;

const RUNTIME_ENV: &str = "MADO_PILOT_ONNX_RUNTIME";
const DETECTOR_ENV: &str = "MADO_PILOT_ONNX_DETECTOR";
const RECOGNIZER_ENV: &str = "MADO_PILOT_ONNX_RECOGNIZER";
const EXPECTED: [&str; 8] = [
    "魔導士",
    "Lv.42",
    "HP1234/5678",
    "MP98%",
    "クエスト",
    "[A-7]",
    "次へ>",
    "READY!",
];

fn main() {
    if [RUNTIME_ENV, DETECTOR_ENV, RECOGNIZER_ENV]
        .into_iter()
        .any(|variable| std::env::var_os(variable).is_none())
    {
        eprintln!("onnx-cpu benchmark skipped: set the three reviewed MADO_PILOT_ONNX_* paths");
        return;
    }
    let runtime = required_path(RUNTIME_ENV);
    let source = source();
    let operation = OperationContext::new();

    let cold_started = Instant::now();
    let backend = Arc::new(
        OnnxOcrBackend::open(source.clone(), &runtime, &operation)
            .expect("accepted backend cold-opens"),
    );
    let cold_load_ms = cold_started.elapsed().as_secs_f64() * 1_000.0;
    let facts = backend.facts();
    let erased: Arc<dyn OcrBackend> = backend.clone();
    let fixture = Fixture {
        recognizer: OcrRecognizer::new(erased),
        frame: hud_frame(),
        operation: operation.clone(),
    };
    let warm = bench_harness::measure(
        "onnx_cpu_hud_warm",
        "exact accepted HUD region text and order",
        Plan::new(3, 10),
        || fixture,
        recognize_hud,
    );

    let close_started = Instant::now();
    backend
        .close(&operation)
        .expect("backend closes after samples");
    let close_ms = close_started.elapsed().as_secs_f64() * 1_000.0;
    let mut reopen_close_ms = Vec::with_capacity(3);
    for _ in 0..3 {
        let started = Instant::now();
        let reopened = OnnxOcrBackend::open(source.clone(), &runtime, &operation)
            .expect("session pair reopens");
        reopened.close(&operation).expect("reopened pair closes");
        reopen_close_ms.push(started.elapsed().as_secs_f64() * 1_000.0);
    }

    print_observation(cold_load_ms, close_ms, &reopen_close_ms, facts, &warm);
}

struct Fixture {
    recognizer: OcrRecognizer,
    frame: Frame,
    operation: OperationContext,
}

fn recognize_hud(fixture: &Fixture) -> Sample {
    let descriptor = fixture.recognizer.descriptor();
    let started = Instant::now();
    let result = fixture.recognizer.recognize(OcrRequest::new(
        &fixture.frame,
        descriptor.backend_identity(),
        descriptor.model_identity(),
        OcrRegion::FullFrame,
        CoordinateSpace::CapturePixels,
        &fixture.operation,
    ));
    let elapsed = started.elapsed();
    let correct = result.is_ok_and(|result| {
        result.regions().len() == EXPECTED.len()
            && result
                .regions()
                .iter()
                .zip(EXPECTED)
                .all(|(region, expected)| region.text() == expected)
    });
    Sample::new(elapsed, correct, 960 * 540 * 4)
}

fn print_observation(
    cold_load_ms: f64,
    close_ms: f64,
    reopen_close_ms: &[f64],
    facts: OnnxBackendFacts,
    warm: &bench_harness::Workload,
) {
    let reopen_close_mean_ms = reopen_close_ms.iter().sum::<f64>() / reopen_close_ms.len() as f64;
    println!(
        concat!(
            "onnx-cpu-observation ",
            "cold_load_ms={cold_load_ms:.3} ",
            "warm_p50_ms={warm_p50_ms:.3} ",
            "warm_p95_ms={warm_p95_ms:.3} ",
            "warm_max_ms={warm_max_ms:.3} ",
            "incorrect={incorrect} mapped_bytes={mapped_bytes} ",
            "rust_peak_allocated_bytes={peak_bytes} rust_growth_bytes={growth_bytes} ",
            "close_ms={close_ms:.3} reopen_close_mean_ms={reopen_close_mean_ms:.3} ",
            "max_tensor_bytes={max_tensor_bytes} max_output_bytes={max_output_bytes} ",
            "max_concurrency={max_concurrency} recognition_batch={recognition_batch}"
        ),
        cold_load_ms = cold_load_ms,
        close_ms = close_ms,
        reopen_close_mean_ms = reopen_close_mean_ms,
        warm_p50_ms = warm.percentile(0.50),
        warm_p95_ms = warm.percentile(0.95),
        warm_max_ms = warm.max_elapsed().as_secs_f64() * 1_000.0,
        incorrect = warm.incorrect(),
        mapped_bytes = warm.mapped_bytes_per_result(),
        peak_bytes = warm.peak_allocated_bytes(),
        growth_bytes = warm.growth_bytes(),
        max_tensor_bytes = facts.max_tensor_bytes(),
        max_output_bytes = facts.max_output_bytes(),
        max_concurrency = facts.max_concurrent_inferences(),
        recognition_batch = facts.recognition_batch(),
    );
}

fn source() -> OcrModelSource {
    let identity = OcrModelIdentity::accepted_g004();
    let detector: Arc<[u8]> = std::fs::read(required_path(DETECTOR_ENV))
        .expect("read accepted detector")
        .into();
    let recognizer: Arc<[u8]> = std::fs::read(required_path(RECOGNIZER_ENV))
        .expect("read accepted recognizer")
        .into();
    OcrModelSource::new(OcrModelSourceRequest {
        detector: OcrModelComponent::new(Arc::clone(&detector), identity.detector())
            .expect("accepted detector identity"),
        recognizer: OcrModelComponent::new(Arc::clone(&recognizer), identity.recognizer())
            .expect("accepted recognizer identity"),
        identity,
    })
    .expect("accepted source")
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

fn required_path(variable: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(variable).expect("benchmark path is configured"))
        .canonicalize()
        .expect("benchmark path is canonicalizable")
}
