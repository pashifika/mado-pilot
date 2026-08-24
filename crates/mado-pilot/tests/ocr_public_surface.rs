//! OCR vocabulary and explicit fake-backend wiring through the public facade only.

use std::sync::Arc;

use mado_pilot::replay::{ReplayFrame, ReplaySource, ReplayTarget};
use mado_pilot::{
    ClipPolicy, Continuity, CoordinateSpace, DefaultOcrConfig, FrameDescriptor, FrameRequest,
    MonotonicInstant, OcrBackend, OcrRegion, OcrRequest, OpenRequest, OperationContext,
    PixelExtent, PixelFormat, PixelRect, Rect, ReplayEngineRequest, Status,
};
use mado_pilot_testkit::{ControlledOcr, ScriptedOcrCandidate};

#[test]
fn replay_facade_exposes_explicit_source_correlated_ocr() {
    let descriptor = FrameDescriptor::packed(PixelExtent::new(16, 12), PixelFormat::Rgba8)
        .expect("valid replay descriptor");
    let source = ReplaySource::from_targets(vec![
        ReplayTarget::new(
            "ocr-panel",
            vec![
                ReplayFrame::new(
                    descriptor,
                    MonotonicInstant::ORIGIN,
                    Continuity::Continuous,
                    None,
                    vec![0x40; descriptor.byte_len()].into_boxed_slice(),
                )
                .expect("valid replay frame"),
            ],
        )
        .expect("valid replay target"),
    ])
    .expect("valid replay source");
    let backend = Arc::new(ControlledOcr::new(PixelFormat::Rgba8).with_candidates(vec![
        ScriptedOcrCandidate::new(
            "  魔導士 A-7  ".as_bytes(),
            [(1.0, 1.0), (14.0, 1.0), (14.0, 6.0), (1.0, 6.0)],
            0.91,
            0,
        ),
    ]));
    let selected = backend.descriptor();
    let request = ReplayEngineRequest::new(source)
        .with_ocr_backend(Arc::clone(&backend) as Arc<dyn OcrBackend>);
    assert_eq!(
        request
            .ocr_backend()
            .expect("explicit backend remains inspectable")
            .descriptor(),
        selected
    );

    let engine =
        mado_pilot::replay_engine(request).expect("OpenCV and replay wiring are available");
    assert_eq!(engine.ocr_backend(), Some(selected.clone()));
    let operation = OperationContext::new();
    let target = engine.discover(&operation).expect("discovered")[0].id();
    let session = engine
        .open(target, &OpenRequest::new(), &operation)
        .expect("opened");
    let frame = session
        .acquire_frame(&FrameRequest::latest(), &operation)
        .expect("acquired exact replay frame");

    let result = session
        .recognize(OcrRequest::new(
            &frame,
            selected.backend_identity(),
            selected.model_identity(),
            OcrRegion::FullFrame,
            CoordinateSpace::CapturePixels,
            &operation,
        ))
        .expect("recognized through facade");

    assert_eq!(result.stamp(), frame.stamp());
    assert_eq!(result.transform(), frame.transform());
    assert_eq!(result.regions()[0].text(), "魔導士 A-7");
    assert_eq!(result.backend(), &selected);
}

#[test]
fn default_ocr_refuses_a_missing_controlled_runtime_without_ambient_fallback() {
    let descriptor = FrameDescriptor::packed(PixelExtent::new(8, 8), PixelFormat::Bgra8)
        .expect("valid replay descriptor");
    let frame = ReplayFrame::new(
        descriptor,
        MonotonicInstant::ORIGIN,
        Continuity::Continuous,
        None,
        vec![0; descriptor.byte_len()].into_boxed_slice(),
    )
    .expect("valid replay frame");
    let source = ReplaySource::from_targets(vec![
        ReplayTarget::new("blank", vec![frame]).expect("valid replay target"),
    ])
    .expect("valid replay source");
    let runtime_name = if cfg!(windows) {
        "onnxruntime.dll"
    } else {
        "libonnxruntime.1.29.0.dylib"
    };
    let runtime = std::env::temp_dir()
        .join(format!(
            "mado-pilot-default-ocr-missing-{}",
            std::process::id()
        ))
        .join(runtime_name);
    let config = DefaultOcrConfig::new(std::env::temp_dir(), &runtime);

    let error = mado_pilot::replay_engine_with_default_ocr(
        ReplayEngineRequest::new(source),
        &config,
        &OperationContext::new(),
    )
    .expect_err("a missing controlled runtime cannot construct an engine");

    assert_eq!(error.status(), Status::Unsupported);
    assert_eq!(error.detail(), "controlled ONNX runtime is unavailable");
    assert!(
        !error.detail().contains(runtime.to_string_lossy().as_ref()),
        "host paths stay private"
    );
}

#[test]
#[ignore = "requires explicit reviewed ONNX Runtime and G-004 model root"]
fn default_ocr_profile_runs_full_and_bounded_replay_regions() {
    let model_root =
        std::env::var_os("MADO_PILOT_G004_MODEL_ROOT").expect("model root is explicitly set");
    let runtime =
        std::env::var_os("MADO_PILOT_ONNX_RUNTIME").expect("runtime path is explicitly set");
    let config = DefaultOcrConfig::new(model_root, runtime);
    let extent = PixelExtent::new(64, 64);
    let descriptor =
        FrameDescriptor::packed(extent, PixelFormat::Bgra8).expect("valid blank descriptor");
    let frame = ReplayFrame::new(
        descriptor,
        MonotonicInstant::ORIGIN,
        Continuity::Continuous,
        None,
        vec![0; descriptor.byte_len()].into_boxed_slice(),
    )
    .expect("valid blank frame");
    let source = ReplaySource::from_targets(vec![
        ReplayTarget::new("blank", vec![frame]).expect("valid replay target"),
    ])
    .expect("valid replay source");
    let operation = OperationContext::new();
    let engine = mado_pilot::replay_engine_with_default_ocr(
        ReplayEngineRequest::new(source),
        &config,
        &operation,
    )
    .expect("accepted controlled prerequisites construct the default");
    let selected = engine.ocr_backend().expect("default OCR is observable");
    assert_eq!(selected.id().as_str(), mado_pilot::DEFAULT_OCR_BACKEND_ID);
    assert_eq!(
        selected.version().as_str(),
        mado_pilot::DEFAULT_OCR_BACKEND_VERSION
    );
    assert_eq!(
        selected.model().as_str(),
        mado_pilot::ACCEPTED_G004_MODEL_ID
    );
    assert_eq!(
        selected.profile().as_str(),
        mado_pilot::ACCEPTED_G004_PROFILE_ID
    );

    let target = engine.discover(&operation).expect("discovered")[0].id();
    let session = engine
        .open(target, &OpenRequest::new(), &operation)
        .expect("opened");
    let frame = session
        .acquire_frame(&FrameRequest::latest(), &operation)
        .expect("acquired exact replay frame");
    let full = session
        .recognize(OcrRequest::new(
            &frame,
            selected.backend_identity(),
            selected.model_identity(),
            OcrRegion::FullFrame,
            CoordinateSpace::CapturePixels,
            &operation,
        ))
        .expect("blank full frame recognizes");
    let bounded_region = Rect::new(CoordinateSpace::CapturePixels, 8.0, 8.0, 40.0, 40.0)
        .expect("valid bounded region");
    let bounded = session
        .recognize(OcrRequest::new(
            &frame,
            selected.backend_identity(),
            selected.model_identity(),
            OcrRegion::Region {
                rect: bounded_region,
                policy: ClipPolicy::Reject,
            },
            CoordinateSpace::CapturePixels,
            &operation,
        ))
        .expect("blank bounded region recognizes");

    assert!(full.regions().is_empty());
    assert!(bounded.regions().is_empty());
    assert_eq!(full.stamp(), frame.stamp());
    assert_eq!(bounded.stamp(), frame.stamp());
    assert_eq!(
        bounded.effective_region(),
        PixelRect::new(8, 8, 40, 40).expect("valid effective region")
    );
    session.close(&operation).expect("first close succeeds");
    session
        .close(&operation)
        .expect("repeated close is idempotent");
    assert_eq!(full.backend(), &selected);
    assert_eq!(bounded.backend(), &selected);
}
