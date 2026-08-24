//! OCR vocabulary and explicit fake-backend wiring through the public facade only.

use std::sync::Arc;

use mado_pilot::replay::{ReplayFrame, ReplaySource, ReplayTarget};
use mado_pilot::{
    Continuity, CoordinateSpace, FrameDescriptor, FrameRequest, MonotonicInstant, OcrBackend,
    OcrRegion, OcrRequest, OpenRequest, OperationContext, PixelExtent, PixelFormat,
    ReplayEngineRequest,
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
