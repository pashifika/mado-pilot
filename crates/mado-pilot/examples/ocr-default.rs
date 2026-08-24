//! Integrated default ONNX CPU OCR through the public Rust facade.

use std::io;
use std::path::PathBuf;

use mado_pilot::replay::{ReplayFrame, ReplaySource, ReplayTarget};
use mado_pilot::{
    ClipPolicy, Continuity, CoordinateSpace, DefaultOcrConfig, FrameDescriptor, FrameRequest,
    MonotonicInstant, OcrRegion, OcrRequest, OpenRequest, OperationContext, PixelExtent,
    PixelFormat, PixelRect, Rect, ReplayEngineRequest,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = DefaultOcrConfig::new(
        controlled_path("MADO_PILOT_G004_MODEL_ROOT")?,
        controlled_path("MADO_PILOT_ONNX_RUNTIME")?,
    );
    let extent = PixelExtent::new(64, 64);
    let descriptor = FrameDescriptor::packed(extent, PixelFormat::Bgra8)?;
    let replay = ReplayFrame::new(
        descriptor,
        MonotonicInstant::ORIGIN,
        Continuity::Continuous,
        None,
        vec![0; descriptor.byte_len()].into_boxed_slice(),
    )?;
    let source =
        ReplaySource::from_targets(vec![ReplayTarget::new("default-ocr-blank", vec![replay])?])?;
    let operation = OperationContext::new();
    let engine = mado_pilot::replay_engine_with_default_ocr(
        ReplayEngineRequest::new(source),
        &config,
        &operation,
    )?;
    let selected = engine
        .ocr_backend()
        .expect("the default constructor always configures OCR");
    let target = engine.discover(&operation)?[0].id();
    let session = engine.open(target, &OpenRequest::new(), &operation)?;
    let frame = session.acquire_frame(&FrameRequest::latest(), &operation)?;

    let full = session.recognize(OcrRequest::new(
        &frame,
        selected.backend_identity(),
        selected.model_identity(),
        OcrRegion::FullFrame,
        CoordinateSpace::CapturePixels,
        &operation,
    ))?;
    let bounded = session.recognize(OcrRequest::new(
        &frame,
        selected.backend_identity(),
        selected.model_identity(),
        OcrRegion::Region {
            rect: Rect::new(CoordinateSpace::CapturePixels, 8.0, 8.0, 40.0, 40.0)?,
            policy: ClipPolicy::Reject,
        },
        CoordinateSpace::CapturePixels,
        &operation,
    ))?;

    assert!(full.is_empty());
    assert!(bounded.is_empty());
    assert_eq!(full.stamp(), frame.stamp());
    assert_eq!(bounded.stamp(), frame.stamp());
    assert_eq!(
        bounded.effective_region(),
        PixelRect::new(8, 8, 40, 40).expect("valid fixture bounds")
    );
    session.close(&operation)?;
    session.close(&operation)?;

    println!(
        "default-ocr: backend={} model={} full={} region={}",
        selected.id().as_str(),
        selected.model().as_str(),
        full.regions().len(),
        bounded.regions().len(),
    );
    Ok(())
}

fn controlled_path(variable: &str) -> io::Result<PathBuf> {
    let value = std::env::var_os(variable).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("{variable} must name a reviewed default OCR prerequisite"),
        )
    })?;
    PathBuf::from(value).canonicalize()
}
