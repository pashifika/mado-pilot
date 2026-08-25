//! OCR vocabulary and explicit fake-backend wiring through the public facade only.

use std::sync::Arc;
use std::time::Duration;

use mado_pilot::replay::{ReplayFrame, ReplaySource, ReplayTarget};
use mado_pilot::{
    CancellationToken, ClipPolicy, Continuity, CoordinateSpace, DefaultOcrConfig, FrameDescriptor,
    FrameRequest, MonotonicInstant, OcrBackend, OcrProfile, OcrProfileConfig, OcrRegion,
    OcrRequest, OcrZone, OcrZoneScanRequest, OpenRequest, OperationContext, PixelExtent,
    PixelFormat, PixelRect, Rect, ReplayEngineRequest, Status,
};
use mado_pilot_testkit::{ControlledOcr, ManualClock, ScriptedOcrCandidate};

fn blank_source(extent: PixelExtent, format: PixelFormat) -> ReplaySource {
    let descriptor = FrameDescriptor::packed(extent, format).expect("valid replay descriptor");
    let frame = ReplayFrame::new(
        descriptor,
        MonotonicInstant::ORIGIN,
        Continuity::Continuous,
        None,
        vec![0; descriptor.byte_len()].into_boxed_slice(),
    )
    .expect("valid replay frame");
    ReplaySource::from_targets(vec![
        ReplayTarget::new("blank", vec![frame]).expect("valid replay target"),
    ])
    .expect("valid replay source")
}

fn zone(left: f64, top: f64, right: f64, bottom: f64) -> OcrZone {
    OcrZone::new(
        Rect::new(CoordinateSpace::CapturePixels, left, top, right, bottom).expect("valid zone"),
        ClipPolicy::Reject,
    )
}

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
    let backend_lifetime = Arc::downgrade(&backend);
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
    let one = [zone(0.0, 0.0, 16.0, 12.0)];
    let three = [
        zone(0.0, 0.0, 8.0, 8.0),
        zone(8.0, 0.0, 16.0, 8.0),
        zone(0.0, 8.0, 16.0, 12.0),
    ];
    let eight = [
        zone(0.0, 0.0, 4.0, 6.0),
        zone(4.0, 0.0, 8.0, 6.0),
        zone(8.0, 0.0, 12.0, 6.0),
        zone(12.0, 0.0, 16.0, 6.0),
        zone(0.0, 6.0, 4.0, 12.0),
        zone(4.0, 6.0, 8.0, 12.0),
        zone(8.0, 6.0, 12.0, 12.0),
        zone(12.0, 6.0, 16.0, 12.0),
    ];
    let overlap = [zone(0.0, 0.0, 10.0, 8.0), zone(4.0, 0.0, 16.0, 8.0)];
    let scan = |zones: &[OcrZone], operation: &OperationContext| {
        session
            .scan_ocr_zones(
                OcrZoneScanRequest::new(
                    &frame,
                    selected.backend_identity(),
                    selected.model_identity(),
                    zones,
                    CoordinateSpace::CapturePixels,
                    operation,
                )
                .expect("bounded zone count"),
            )
            .expect("grouped facade scan succeeds")
    };
    let one_result = scan(&one, &operation);
    let three_result = scan(&three, &operation);
    let eight_result = scan(&eight, &operation);
    let overlap_result = scan(&overlap, &operation);

    assert_eq!(one_result.group(0).expect("group").len(), 1);
    assert_eq!(three_result.group(0).expect("group").len(), 1);
    assert!(three_result.group(1).expect("empty group").is_empty());
    assert!(three_result.group(2).expect("empty group").is_empty());
    assert_eq!(eight_result.effective_zones().len(), 8);
    assert_eq!(eight_result.group(1).expect("group").len(), 1);
    assert_eq!(overlap_result.unique_candidates().len(), 1);
    let left = overlap_result
        .group(0)
        .expect("left group")
        .get(0)
        .expect("left membership");
    let right = overlap_result
        .group(1)
        .expect("right group")
        .get(0)
        .expect("right membership");
    assert!(
        std::ptr::eq(left, right),
        "shared membership owns one candidate"
    );

    let zero: [OcrZone; 0] = [];
    let zero_error = OcrZoneScanRequest::new(
        &frame,
        selected.backend_identity(),
        selected.model_identity(),
        &zero,
        CoordinateSpace::CapturePixels,
        &operation,
    )
    .expect_err("zero zones are invalid");
    assert_eq!(zero_error.status(), Status::InvalidArgument);
    let nine = [zone(0.0, 0.0, 16.0, 12.0); 9];
    let nine_error = OcrZoneScanRequest::new(
        &frame,
        selected.backend_identity(),
        selected.model_identity(),
        &nine,
        CoordinateSpace::CapturePixels,
        &operation,
    )
    .expect_err("nine zones are invalid");
    assert_eq!(nine_error.status(), Status::InvalidArgument);

    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let cancelled_operation = OperationContext::new().with_cancellation(cancellation);
    let cancelled = session
        .scan_ocr_zones(
            OcrZoneScanRequest::new(
                &frame,
                selected.backend_identity(),
                selected.model_identity(),
                &one,
                CoordinateSpace::CapturePixels,
                &cancelled_operation,
            )
            .expect("one zone"),
        )
        .expect_err("cancellation wins before backend work");
    assert_eq!(cancelled.status(), Status::Cancelled);

    session.close(&operation).expect("closed");
    session
        .close(&operation)
        .expect("repeated close is idempotent");
    drop(frame);
    drop(session);
    drop(engine);
    drop(backend);
    assert!(backend_lifetime.upgrade().is_none());
    assert_eq!(
        overlap_result
            .group(0)
            .expect("retained group")
            .get(0)
            .expect("retained candidate")
            .text(),
        "魔導士 A-7"
    );
}

#[test]
fn grouped_facade_discards_backend_completion_after_the_deadline() {
    let clock = Arc::new(ManualClock::new());
    let backend = Arc::new(
        ControlledOcr::new(PixelFormat::Rgba8)
            .with_candidates(vec![ScriptedOcrCandidate::new(
                b"late".as_slice(),
                [(1.0, 1.0), (14.0, 1.0), (14.0, 6.0), (1.0, 6.0)],
                0.91,
                0,
            )])
            .with_latency(Arc::clone(&clock), Duration::from_millis(50)),
    );
    let selected = backend.descriptor();
    let engine = mado_pilot::replay_engine(
        ReplayEngineRequest::new(blank_source(PixelExtent::new(16, 12), PixelFormat::Rgba8))
            .with_ocr_backend(Arc::clone(&backend) as Arc<dyn OcrBackend>),
    )
    .expect("replay wiring");
    let operation = OperationContext::new()
        .with_clock(clock)
        .with_timeout(Duration::from_millis(10))
        .expect("representable deadline");
    let target = engine.discover(&operation).expect("discovered")[0].id();
    let session = engine
        .open(target, &OpenRequest::new(), &operation)
        .expect("opened");
    let frame = session
        .acquire_frame(&FrameRequest::latest(), &operation)
        .expect("acquired");
    let zones = [zone(0.0, 0.0, 16.0, 12.0)];

    let error = session
        .scan_ocr_zones(
            OcrZoneScanRequest::new(
                &frame,
                selected.backend_identity(),
                selected.model_identity(),
                &zones,
                CoordinateSpace::CapturePixels,
                &operation,
            )
            .expect("one zone"),
        )
        .expect_err("late backend data cannot publish a grouped result");

    assert_eq!(error.status(), Status::DeadlineExceeded);
    assert_eq!(backend.recognition_count(), 1);
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
fn explicit_profile_is_closed_atomic_and_has_no_ambient_fallback() {
    let runtime_name = if cfg!(windows) {
        "onnxruntime.dll"
    } else {
        "libonnxruntime.1.29.0.dylib"
    };
    let runtime = std::env::temp_dir()
        .join(format!(
            "mado-pilot-profile-ocr-missing-{}",
            std::process::id()
        ))
        .join(runtime_name);
    let config = OcrProfileConfig::new(OcrProfile::BoundedDetector, std::env::temp_dir(), &runtime);
    assert_eq!(config.profile(), OcrProfile::BoundedDetector);
    assert_eq!(config.runtime_path(), runtime);

    let error = mado_pilot::replay_engine_with_ocr_profile(
        ReplayEngineRequest::new(blank_source(PixelExtent::new(8, 8), PixelFormat::Bgra8)),
        &config,
        &OperationContext::new(),
    )
    .expect_err("a missing controlled runtime cannot publish an engine");
    assert_eq!(error.status(), Status::Unsupported);
    assert_eq!(error.detail(), "controlled ONNX runtime is unavailable");
    assert!(!error.detail().contains(runtime.to_string_lossy().as_ref()));

    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let cancelled = mado_pilot::replay_engine_with_ocr_profile(
        ReplayEngineRequest::new(blank_source(PixelExtent::new(8, 8), PixelFormat::Bgra8)),
        &config,
        &OperationContext::new().with_cancellation(cancellation),
    )
    .expect_err("authoritative cancellation publishes no partial engine");
    assert_eq!(cancelled.status(), Status::Cancelled);
}

#[test]
#[ignore = "requires explicit reviewed ONNX Runtime and G-004 model root"]
fn default_ocr_profile_runs_full_and_bounded_replay_regions() {
    let model_root = std::path::PathBuf::from(
        std::env::var_os("MADO_PILOT_G004_MODEL_ROOT").expect("model root is explicitly set"),
    )
    .canonicalize()
    .expect("model root is canonicalizable");
    let runtime = std::path::PathBuf::from(
        std::env::var_os("MADO_PILOT_ONNX_RUNTIME").expect("runtime path is explicitly set"),
    )
    .canonicalize()
    .expect("runtime path is canonicalizable");
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

    drop(frame);
    drop(session);
    drop(engine);

    let profile_config = OcrProfileConfig::new(
        OcrProfile::BoundedDetector,
        config.model_root(),
        config.runtime_path(),
    );
    let profile_engine = mado_pilot::replay_engine_with_ocr_profile(
        ReplayEngineRequest::new(blank_source(extent, PixelFormat::Bgra8)),
        &profile_config,
        &operation,
    )
    .expect("accepted controlled prerequisites construct the bounded profile");
    let selected = profile_engine
        .ocr_backend()
        .expect("explicit OCR profile is observable");
    assert_eq!(
        selected.model().as_str(),
        mado_pilot::ACCEPTED_BOUNDED_MODEL_ID
    );
    assert_eq!(
        selected.profile().as_str(),
        mado_pilot::ACCEPTED_BOUNDED_PROFILE_ID
    );

    let target = profile_engine.discover(&operation).expect("discovered")[0].id();
    let profile_session = profile_engine
        .open(target, &OpenRequest::new(), &operation)
        .expect("opened");
    let profile_frame = profile_session
        .acquire_frame(&FrameRequest::latest(), &operation)
        .expect("acquired exact replay frame");
    let one = [zone(0.0, 0.0, 64.0, 64.0)];
    let three = [
        zone(0.0, 0.0, 32.0, 32.0),
        zone(32.0, 0.0, 64.0, 32.0),
        zone(0.0, 32.0, 64.0, 64.0),
    ];
    let eight = [
        zone(0.0, 0.0, 16.0, 32.0),
        zone(16.0, 0.0, 32.0, 32.0),
        zone(32.0, 0.0, 48.0, 32.0),
        zone(48.0, 0.0, 64.0, 32.0),
        zone(0.0, 32.0, 16.0, 64.0),
        zone(16.0, 32.0, 32.0, 64.0),
        zone(32.0, 32.0, 48.0, 64.0),
        zone(48.0, 32.0, 64.0, 64.0),
    ];
    let scan = |zones: &[OcrZone]| {
        profile_session
            .scan_ocr_zones(
                OcrZoneScanRequest::new(
                    &profile_frame,
                    selected.backend_identity(),
                    selected.model_identity(),
                    zones,
                    CoordinateSpace::CapturePixels,
                    &operation,
                )
                .expect("bounded zone count"),
            )
            .expect("blank grouped scan succeeds")
    };
    let one_result = scan(&one);
    let three_result = scan(&three);
    let eight_result = scan(&eight);
    let singular = profile_session
        .recognize(OcrRequest::new(
            &profile_frame,
            selected.backend_identity(),
            selected.model_identity(),
            OcrRegion::FullFrame,
            CoordinateSpace::CapturePixels,
            &operation,
        ))
        .expect("singular OCR remains available");

    assert_eq!(one_result.effective_zones().len(), 1);
    assert_eq!(three_result.effective_zones().len(), 3);
    assert_eq!(eight_result.effective_zones().len(), 8);
    assert!(one_result.group(0).expect("group").is_empty());
    assert!((0..3).all(|index| three_result.group(index).expect("group").is_empty()));
    assert!((0..8).all(|index| eight_result.group(index).expect("group").is_empty()));
    assert!(singular.regions().is_empty());

    profile_session.close(&operation).expect("closed");
    drop(profile_frame);
    drop(profile_session);
    drop(profile_engine);
    assert_eq!(eight_result.backend(), &selected);
    assert_eq!(
        eight_result.source_envelope(),
        PixelRect::new(0, 0, 64, 64).expect("valid envelope")
    );
}
