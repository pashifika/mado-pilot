//! Explicit bounded OCR profile and grouped zones through the public Rust facade.

use std::io;
use std::path::PathBuf;

use mado_pilot::replay::{ReplayFrame, ReplaySource, ReplayTarget};
use mado_pilot::{
    ClipPolicy, Continuity, CoordinateSpace, FrameDescriptor, FrameRequest, MonotonicInstant,
    OcrProfile, OcrProfileConfig, OcrZone, OcrZoneScanRequest, OpenRequest, OperationContext,
    PixelExtent, PixelFormat, Rect, ReplayEngineRequest,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = OcrProfileConfig::new(
        OcrProfile::BoundedDetector,
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
    let source = ReplaySource::from_targets(vec![ReplayTarget::new(
        "bounded-profile-blank",
        vec![replay],
    )?])?;
    let operation = OperationContext::new();
    let engine = mado_pilot::replay_engine_with_ocr_profile(
        ReplayEngineRequest::new(source),
        &config,
        &operation,
    )?;
    let selected = engine
        .ocr_backend()
        .expect("the profile constructor always configures OCR");
    assert_eq!(
        selected.profile().as_str(),
        mado_pilot::ACCEPTED_BOUNDED_PROFILE_ID
    );
    let target = engine.discover(&operation)?[0].id();
    let session = engine.open(target, &OpenRequest::new(), &operation)?;
    let frame = session.acquire_frame(&FrameRequest::latest(), &operation)?;
    let zones = [
        zone(0.0, 0.0, 24.0, 24.0)?,
        zone(40.0, 0.0, 64.0, 24.0)?,
        zone(0.0, 40.0, 24.0, 64.0)?,
    ];
    let result = session.scan_ocr_zones(OcrZoneScanRequest::new(
        &frame,
        selected.backend_identity(),
        selected.model_identity(),
        &zones,
        CoordinateSpace::CapturePixels,
        &operation,
    )?)?;

    drop(frame);
    session.close(&operation)?;
    drop(session);
    drop(engine);

    for index in 0..zones.len() {
        for region in result
            .group(index)
            .expect("the result preserves every zone")
            .iter()
        {
            println!("{}", region.text());
        }
    }
    drop(result);
    Ok(())
}

fn zone(left: f64, top: f64, right: f64, bottom: f64) -> mado_pilot::Result<OcrZone> {
    Ok(OcrZone::new(
        Rect::new(CoordinateSpace::CapturePixels, left, top, right, bottom)?,
        ClipPolicy::Reject,
    ))
}

fn controlled_path(variable: &str) -> io::Result<PathBuf> {
    let value = std::env::var_os(variable).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("{variable} must name a reviewed OCR prerequisite"),
        )
    })?;
    PathBuf::from(value).canonicalize()
}
