//! Deterministic one-shot OCR through the public Rust facade.

use std::path::PathBuf;
use std::sync::Arc;

use mado_pilot::replay::{ReplayFrame, ReplaySource, ReplayTarget};
use mado_pilot::{
    ContentDigest, Continuity, CoordinateSpace, DecoderId, FrameDescriptor, FrameRequest,
    LanguageProfileId, ModelComponentIdentity, ModelId, ModelVersion, MonotonicInstant,
    NormalizationId, OcrBackend, OcrBackendDescriptor, OcrBackendId, OcrBackendIdentity,
    OcrBackendVersion, OcrModelIdentity, OcrProfileMetadata, OcrRegion, OcrRequest, OpenRequest,
    OperationContext, PackageSource, PixelExtent, PixelFormat, PreprocessingId, ProfileId,
    ReplayEngineRequest,
};
use mado_pilot_testkit::{ControlledOcr, ScriptedOcrCandidate};

const MODEL_ID: &str = "fixture-ocr-model";
const BACKEND_ID: &str = "fixture-ocr-backend";
const BACKEND_VERSION: &str = "1";
const DETECTOR: &[u8] =
    include_bytes!("../../../fixtures/assets/ocr-public-surface/models/detector.onnx");
const RECOGNIZER: &[u8] =
    include_bytes!("../../../fixtures/assets/ocr-public-surface/models/recognizer.onnx");

fn descriptor() -> OcrBackendDescriptor {
    let detector = ContentDigest::of(DETECTOR);
    let recognizer = ContentDigest::of(RECOGNIZER);
    let model = OcrModelIdentity::new(
        ModelId::new(MODEL_ID).expect("fixture model id"),
        ModelVersion::new("1").expect("fixture model version"),
        ProfileId::new("fixture-ocr-profile").expect("fixture profile"),
        ModelComponentIdentity::new(DETECTOR.len() as u64, *detector.as_bytes())
            .expect("fixture detector identity"),
        ModelComponentIdentity::new(RECOGNIZER.len() as u64, *recognizer.as_bytes())
            .expect("fixture recognizer identity"),
        OcrProfileMetadata::new(
            LanguageProfileId::new("fixture-language").expect("fixture language"),
            PreprocessingId::new("fixture-preprocessing").expect("fixture preprocessing"),
            DecoderId::new("fixture-decoder").expect("fixture decoder"),
            NormalizationId::new(mado_pilot::ACCEPTED_G004_NORMALIZATION_ID)
                .expect("accepted normalization"),
            1,
            [3; 32],
        )
        .expect("fixture profile metadata"),
    )
    .expect("fixture model identity");
    OcrBackendDescriptor::new(
        OcrBackendIdentity::new(
            OcrBackendId::new(BACKEND_ID).expect("fixture backend id"),
            OcrBackendVersion::new(BACKEND_VERSION).expect("fixture backend version"),
        ),
        model,
        PixelFormat::Rgba8,
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let frame_descriptor = FrameDescriptor::packed(PixelExtent::new(32, 24), PixelFormat::Rgba8)?;
    let replay = ReplayFrame::new(
        frame_descriptor,
        MonotonicInstant::ORIGIN,
        Continuity::Continuous,
        None,
        vec![0x40; frame_descriptor.byte_len()].into_boxed_slice(),
    )?;
    let source = ReplaySource::from_targets(vec![ReplayTarget::new("ocr-panel", vec![replay])?])?;
    let selected = descriptor();
    let backend = Arc::new(
        ControlledOcr::new(PixelFormat::Rgba8)
            .with_descriptor(selected.clone())
            .with_candidates(vec![ScriptedOcrCandidate::new(
                "  魔導士 A-7  ".as_bytes(),
                [(1.0, 2.0), (20.0, 2.0), (20.0, 9.0), (1.0, 9.0)],
                0.91,
                0,
            )]),
    );
    let engine = mado_pilot::replay_engine(
        ReplayEngineRequest::new(source)
            .with_ocr_backend(Arc::clone(&backend) as Arc<dyn OcrBackend>),
    )?;
    let operation = OperationContext::new();
    let target = engine.discover(&operation)?[0].id();
    let session = engine.open(target, &OpenRequest::new(), &operation)?;
    let frame = session.acquire_frame(&FrameRequest::latest(), &operation)?;
    let package = engine.load_package(
        &PackageSource::directory(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/assets/ocr-public-surface"),
        ),
        &operation,
    )?;
    let model = package.resolve_ocr_model(MODEL_ID)?;
    let result = session.recognize(OcrRequest::new(
        &frame,
        selected.backend_identity(),
        model.identity(),
        OcrRegion::FullFrame,
        CoordinateSpace::CapturePixels,
        &operation,
    ))?;

    session.close(&operation)?;
    drop(frame);
    drop(session);
    drop(model);
    drop(package);
    drop(engine);
    drop(backend);

    println!(
        "ocr: sequence={} text={} confidence={:.5}",
        result.stamp().sequence().value(),
        result.regions()[0].text(),
        result.regions()[0].confidence().get(),
    );
    Ok(())
}
