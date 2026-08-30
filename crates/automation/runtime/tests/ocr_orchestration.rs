//! One-shot OCR decisions owned by runtime rather than the OCR contract alone.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use mado_pilot_runtime::{
    ActivityTag, CaptureProvider, ClipPolicy, Continuity, CoordinateSpace, DiagnosticDrain,
    DiagnosticKind, DiagnosticOperationKind, DiagnosticOptions, DiagnosticPayload, Engine,
    EngineOptions, EngineWiring, Frame, FrameRequest, IdentityIssuer, Matcher, OcrBackend,
    OcrBackendDescriptor, OcrDiagnosticOutcome, OcrDiagnosticProfile, OcrModelIdentity,
    OcrRecognizer, OcrRegion, OcrRequest, OcrZone, OcrZoneScanRequest, OpenRequest,
    OperationContext, PackageLoader, PixelExtent, PixelFormat, Rect, Session, Status,
};
use mado_pilot_testkit::{
    CompletionGate, ControlledCapture, ControlledMatcher, ControlledOcr, ManualClock,
    ScriptedOcrCandidate,
};
use mado_pilot_vision::MatchBackend;

const EXTENT: PixelExtent = PixelExtent::new(32, 24);

fn candidate(text: &'static [u8]) -> ScriptedOcrCandidate {
    ScriptedOcrCandidate::new(
        text,
        [(1.0, 2.0), (12.0, 2.0), (12.0, 8.0), (1.0, 8.0)],
        0.937_216,
        0,
    )
}

fn wired(ocr: Option<Arc<ControlledOcr>>) -> (Engine, Arc<ControlledCapture>) {
    wired_with_options(ocr, EngineOptions::new())
}

fn wired_with_options(
    ocr: Option<Arc<ControlledOcr>>,
    options: EngineOptions,
) -> (Engine, Arc<ControlledCapture>) {
    let issuer = Arc::new(IdentityIssuer::new());
    let capture = Arc::new(
        ControlledCapture::new(Arc::clone(&issuer), EXTENT, PixelFormat::Rgba8)
            .expect("valid controlled capture"),
    );
    let matcher = Arc::new(ControlledMatcher::new(PixelFormat::Rgba8));
    let recognizer = ocr.map(|backend| OcrRecognizer::new(backend as Arc<dyn OcrBackend>));
    let engine = Engine::new_with_options(
        EngineWiring {
            engine: issuer.engine(),
            capture: Arc::clone(&capture) as Arc<dyn CaptureProvider>,
            matcher: Matcher::new(matcher as Arc<dyn MatchBackend>),
            loader: PackageLoader::new(),
            ocr: recognizer,
            input: None,
            permission: None,
        },
        options,
    )
    .expect("capture-only wiring is valid");
    (engine, capture)
}

fn opened_with_frame(
    engine: &Engine,
    capture: &ControlledCapture,
    operation: &OperationContext,
) -> (Session, Frame) {
    let target = engine.discover(operation).expect("discovered")[0].id();
    let session = engine
        .open(target, &OpenRequest::new(), operation)
        .expect("opened");
    capture
        .publish(0x31, Continuity::Continuous)
        .expect("published");
    let frame = session
        .acquire_frame(&FrameRequest::latest(), operation)
        .expect("acquired");
    (session, frame)
}

fn request<'a>(
    frame: &'a Frame,
    descriptor: &'a OcrBackendDescriptor,
    operation: &'a OperationContext,
) -> OcrRequest<'a> {
    OcrRequest::new(
        frame,
        descriptor.backend_identity(),
        descriptor.model_identity(),
        OcrRegion::FullFrame,
        CoordinateSpace::CapturePixels,
        operation,
    )
}

#[test]
fn recognition_uses_the_exact_retained_frame_named_by_the_request() {
    let ocr = Arc::new(
        ControlledOcr::new(PixelFormat::Rgba8).with_candidates(vec![candidate(b"  retained  ")]),
    );
    let descriptor = ocr.descriptor();
    let (engine, capture) = wired(Some(Arc::clone(&ocr)));
    let operation = OperationContext::new();
    let (session, retained) = opened_with_frame(&engine, &capture, &operation);

    capture
        .publish(0x52, Continuity::Continuous)
        .expect("published a newer frame");
    let result = session
        .recognize(request(&retained, &descriptor, &operation))
        .expect("recognized the retained frame");

    assert_eq!(result.stamp(), retained.stamp());
    assert_eq!(result.stamp().sequence().value(), 0);
    assert_eq!(result.regions()[0].text(), "retained");
    assert_eq!(result.backend(), &descriptor);
    assert_eq!(engine.ocr_backend(), Some(descriptor));
}

#[test]
fn zone_scan_uses_the_exact_frame_and_outlives_every_parent() {
    let ocr = Arc::new(
        ControlledOcr::new(PixelFormat::Rgba8).with_candidates(vec![candidate(b"  grouped  ")]),
    );
    let backend_lifetime = Arc::downgrade(&ocr);
    let descriptor = ocr.descriptor();
    let (engine, capture) = wired(Some(Arc::clone(&ocr)));
    let operation = OperationContext::new();
    let (session, retained) = opened_with_frame(&engine, &capture, &operation);
    capture
        .publish(0x52, Continuity::Continuous)
        .expect("published a newer frame");
    let zones = [OcrZone::new(
        Rect::new(CoordinateSpace::CapturePixels, 0.0, 0.0, 16.0, 12.0).expect("valid zone"),
        ClipPolicy::Reject,
    )];

    let result = session
        .scan_ocr_zones(
            OcrZoneScanRequest::new(
                &retained,
                descriptor.backend_identity(),
                descriptor.model_identity(),
                &zones,
                CoordinateSpace::CapturePixels,
                &operation,
            )
            .expect("one zone is valid"),
        )
        .expect("scanned the retained frame");

    assert_eq!(result.stamp(), retained.stamp());
    assert_eq!(result.stamp().sequence().value(), 0);
    assert_eq!(result.source_envelope().left(), 0);
    assert_eq!(result.source_envelope().right(), 16);
    assert_eq!(result.group(0).expect("group").len(), 1);
    assert_eq!(
        result
            .group(0)
            .expect("group")
            .get(0)
            .expect("candidate")
            .text(),
        "grouped"
    );

    drop(retained);
    session.close(&operation).expect("closed");
    drop(session);
    drop(engine);
    drop(capture);
    drop(ocr);

    assert!(backend_lifetime.upgrade().is_none());
    assert_eq!(result.group(0).expect("group").len(), 1);
    assert_eq!(result.backend(), &descriptor);
}

#[test]
fn foreign_stream_and_missing_configuration_are_refused_before_backend_work() {
    let ocr = Arc::new(ControlledOcr::new(PixelFormat::Rgba8));
    let descriptor = ocr.descriptor();
    let operation = OperationContext::new();

    let (engine, capture) = wired(Some(Arc::clone(&ocr)));
    let (session, _) = opened_with_frame(&engine, &capture, &operation);
    let target = engine.discover(&operation).expect("discovered")[0].id();
    let other = engine
        .open(target, &OpenRequest::new(), &operation)
        .expect("opened another stream");
    capture
        .publish(0x74, Continuity::Continuous)
        .expect("published to both streams");
    let foreign = other
        .acquire_frame(&FrameRequest::latest(), &operation)
        .expect("acquired foreign frame");
    let foreign_error = session
        .recognize(request(&foreign, &descriptor, &operation))
        .expect_err("another stream is never substituted");
    assert_eq!(foreign_error.status(), Status::InvalidArgument);
    assert_eq!(ocr.recognition_count(), 0);

    let (unconfigured, unconfigured_capture) = wired(None);
    let (unconfigured_session, frame) =
        opened_with_frame(&unconfigured, &unconfigured_capture, &operation);
    let unavailable = unconfigured_session
        .recognize(request(&frame, &descriptor, &operation))
        .expect_err("no default OCR backend exists");
    assert_eq!(unavailable.status(), Status::VisionFailed);
    assert_eq!(ocr.recognition_count(), 0);
}

#[test]
fn backend_completion_after_the_deadline_commits_no_result() {
    let clock = Arc::new(ManualClock::new());
    let ocr = Arc::new(
        ControlledOcr::new(PixelFormat::Rgba8)
            .with_candidates(vec![candidate(b"late")])
            .with_latency(Arc::clone(&clock), Duration::from_millis(50)),
    );
    let descriptor = ocr.descriptor();
    let (engine, capture) = wired(Some(Arc::clone(&ocr)));
    let operation = OperationContext::new()
        .with_clock(clock)
        .with_timeout(Duration::from_millis(10))
        .expect("representable deadline");
    let (session, frame) = opened_with_frame(&engine, &capture, &operation);

    let error = session
        .recognize(request(&frame, &descriptor, &operation))
        .expect_err("late backend data is discarded");

    assert_eq!(error.status(), Status::DeadlineExceeded);
    assert_eq!(ocr.recognition_count(), 1);
}

#[test]
fn close_wins_the_final_gate_without_waiting_for_the_backend() {
    let gate = Arc::new(CompletionGate::new());
    let ocr = Arc::new(
        ControlledOcr::new(PixelFormat::Rgba8)
            .with_candidates(vec![candidate(b"closed")])
            .with_completion_gate(Arc::clone(&gate)),
    );
    let descriptor = ocr.descriptor();
    let (engine, capture) = wired(Some(Arc::clone(&ocr)));
    let _gate_release = gate.release_guard();
    let operation = OperationContext::new();
    let (session, frame) = opened_with_frame(&engine, &capture, &operation);
    let session = Arc::new(session);

    let worker = {
        let session = Arc::clone(&session);
        thread::spawn(move || {
            let operation = OperationContext::new();
            session.recognize(request(&frame, &descriptor, &operation))
        })
    };
    assert!(gate.wait_until_entered(Duration::from_secs(1)));
    session
        .close(&OperationContext::new())
        .expect("close does not wait for OCR backend work");
    gate.release();

    let error = worker
        .join()
        .expect("recognition thread did not panic")
        .expect_err("close is authoritative before final result commit");
    assert_eq!(error.status(), Status::Closed);
    assert_eq!(ocr.recognition_count(), 1);
}

#[test]
fn retained_result_outlives_frame_session_engine_and_backend() {
    let ocr = Arc::new(
        ControlledOcr::new(PixelFormat::Rgba8)
            .with_candidates(vec![candidate("  e\u{301}  ".as_bytes())]),
    );
    let backend_lifetime = Arc::downgrade(&ocr);
    let descriptor = ocr.descriptor();
    let (engine, capture) = wired(Some(Arc::clone(&ocr)));
    let operation = OperationContext::new();
    let (session, frame) = opened_with_frame(&engine, &capture, &operation);
    let result = session
        .recognize(request(&frame, &descriptor, &operation))
        .expect("recognized");

    drop(frame);
    session.close(&operation).expect("closed");
    drop(session);
    drop(engine);
    drop(capture);
    drop(ocr);

    assert!(backend_lifetime.upgrade().is_none());
    assert_eq!(result.regions()[0].text(), "é");
    assert_eq!(result.stamp().sequence().value(), 0);
    assert_eq!(result.backend(), &descriptor);
}

fn accepted_backend() -> Arc<ControlledOcr> {
    let backend = ControlledOcr::new(PixelFormat::Rgba8);
    let identity = backend.descriptor().backend_identity().clone();
    Arc::new(
        backend
            .with_descriptor(OcrBackendDescriptor::new(
                identity,
                OcrModelIdentity::accepted_g004(),
                PixelFormat::Rgba8,
            ))
            .with_candidates(vec![candidate(b"caller-secret-recognized-text")]),
    )
}

fn accepted_bounded_backend() -> Arc<ControlledOcr> {
    let backend = ControlledOcr::new(PixelFormat::Rgba8);
    let identity = backend.descriptor().backend_identity().clone();
    Arc::new(
        backend
            .with_descriptor(OcrBackendDescriptor::new(
                identity,
                OcrModelIdentity::accepted_bounded_detector(),
                PixelFormat::Rgba8,
            ))
            .with_candidates(vec![candidate(b"caller-secret-grouped-text")]),
    )
}

fn diagnostic_batch(
    reader: &mado_pilot_runtime::DiagnosticReader,
) -> mado_pilot_runtime::DiagnosticBatch {
    match reader.drain() {
        DiagnosticDrain::Batch(batch) => batch,
        other => panic!("expected diagnostic batch, got {other:?}"),
    }
}

#[test]
fn ocr_diagnostics_are_source_correlated_profile_typed_and_content_redacted() {
    let ocr = accepted_backend();
    let descriptor = ocr.descriptor();
    let (engine, capture) = wired_with_options(
        Some(Arc::clone(&ocr)),
        EngineOptions::new()
            .with_diagnostics(DiagnosticOptions::debug(16).expect("bounded diagnostics")),
    );
    let reader = engine.take_diagnostic_reader().expect("enabled reader");
    let operation = OperationContext::new()
        .with_activity_tag(ActivityTag::new(77).expect("nonzero activity tag"));
    let (session, frame) = opened_with_frame(&engine, &capture, &operation);
    let _ = reader.drain();

    let result = session
        .recognize(request(&frame, &descriptor, &operation))
        .expect("recognized");
    assert_eq!(result.regions()[0].text(), "caller-secret-recognized-text");
    let batch = diagnostic_batch(&reader);
    assert!(batch.losses().is_empty());
    let started = batch
        .records()
        .iter()
        .find(|record| {
            matches!(
                record.payload(),
                DiagnosticPayload::OperationStarted(value)
                    if value.operation == DiagnosticOperationKind::OcrRecognition
            )
        })
        .expect("OCR admission record");
    assert_eq!(started.activity(), ActivityTag::new(77));
    let terminal = batch
        .records()
        .iter()
        .find_map(|record| match record.payload() {
            DiagnosticPayload::Ocr(value) => Some((record, value)),
            _ => None,
        })
        .expect("OCR terminal record");
    assert_eq!(terminal.0.kind(), DiagnosticKind::Ocr);
    assert_eq!(terminal.0.activity(), ActivityTag::new(77));
    assert_eq!(terminal.1.model_instance.get(), 1);
    assert_eq!(terminal.1.profile, OcrDiagnosticProfile::AcceptedG004);
    assert_eq!(terminal.1.source, frame.stamp());
    assert_eq!(terminal.1.requested_region, None);
    assert_eq!(terminal.1.effective_region, Some(result.effective_region()));
    assert_eq!(terminal.1.source_envelope, None);
    assert_eq!(terminal.1.output_space, CoordinateSpace::CapturePixels);
    assert_eq!(terminal.1.outcome, OcrDiagnosticOutcome::Recognized);
    assert_eq!(terminal.1.result_count, 1);
    assert_eq!(terminal.1.zone_count, None);
    assert_eq!(terminal.1.unique_candidate_count, None);
    assert_eq!(terminal.1.membership_count, None);
    assert_eq!(terminal.1.result_bytes, None);
    assert_eq!(terminal.1.detector_runs, None);
    assert_eq!(terminal.1.detector_bytes, None);
    assert_eq!(terminal.1.recognizer_runs, None);
    assert_eq!(terminal.1.recognizer_bytes, None);
    assert_eq!(
        terminal.1.source_pixels,
        u64::from(EXTENT.width()) * u64::from(EXTENT.height())
    );
    let visible = format!("{:?}", terminal.1);
    for secret in [
        "caller-secret-recognized-text",
        descriptor.id().as_str(),
        descriptor.model().as_str(),
        descriptor.profile().as_str(),
    ] {
        assert!(!visible.contains(secret), "diagnostics exposed {secret}");
    }
}

#[test]
fn grouped_ocr_diagnostics_are_aggregate_bounded_and_content_redacted() {
    let ocr = accepted_bounded_backend();
    let descriptor = ocr.descriptor();
    let (engine, capture) = wired_with_options(
        Some(Arc::clone(&ocr)),
        EngineOptions::new()
            .with_diagnostics(DiagnosticOptions::debug(16).expect("bounded diagnostics")),
    );
    let reader = engine.take_diagnostic_reader().expect("enabled reader");
    let operation = OperationContext::new();
    let (session, frame) = opened_with_frame(&engine, &capture, &operation);
    let _ = reader.drain();
    let zones = [
        OcrZone::new(
            Rect::new(CoordinateSpace::CapturePixels, 0.0, 0.0, 8.0, 12.0).expect("valid zone"),
            ClipPolicy::Reject,
        ),
        OcrZone::new(
            Rect::new(CoordinateSpace::CapturePixels, 24.0, 0.0, 32.0, 12.0).expect("valid zone"),
            ClipPolicy::Reject,
        ),
        OcrZone::new(
            Rect::new(CoordinateSpace::CapturePixels, 4.0, 0.0, 16.0, 12.0).expect("valid zone"),
            ClipPolicy::Reject,
        ),
    ];

    let result = session
        .scan_ocr_zones(
            OcrZoneScanRequest::new(
                &frame,
                descriptor.backend_identity(),
                descriptor.model_identity(),
                &zones,
                CoordinateSpace::CapturePixels,
                &operation,
            )
            .expect("three zones"),
        )
        .expect("grouped recognition");
    assert_eq!(
        result.unique_candidates()[0].text(),
        "caller-secret-grouped-text"
    );
    let batch = diagnostic_batch(&reader);
    assert!(batch.losses().is_empty());
    let terminal = batch
        .records()
        .iter()
        .find_map(|record| match record.payload() {
            DiagnosticPayload::Ocr(value) => Some(value),
            _ => None,
        })
        .expect("grouped OCR terminal");
    assert_eq!(terminal.profile, OcrDiagnosticProfile::BoundedDetector);
    assert_eq!(terminal.source, frame.stamp());
    assert_eq!(terminal.requested_region, None);
    assert_eq!(terminal.effective_region, None);
    assert_eq!(terminal.source_envelope, Some(result.source_envelope()));
    assert_eq!(terminal.zone_count, Some(3));
    assert_eq!(terminal.unique_candidate_count, Some(1));
    assert_eq!(terminal.membership_count, Some(2));
    assert_eq!(terminal.result_count, 2);
    assert_eq!(terminal.result_bytes, None);
    assert_eq!(terminal.detector_runs, None);
    assert_eq!(terminal.detector_bytes, None);
    assert_eq!(terminal.recognizer_runs, None);
    assert_eq!(terminal.recognizer_bytes, None);
    assert_eq!(terminal.source_pixels, 32 * 12);

    let visible = format!("{terminal:?}");
    for secret in [
        "caller-secret-grouped-text",
        descriptor.id().as_str(),
        descriptor.model().as_str(),
        descriptor.profile().as_str(),
    ] {
        assert!(!visible.contains(secret), "diagnostics exposed {secret}");
    }
}
#[test]
fn a_full_diagnostic_queue_never_changes_ocr_and_reports_exact_normal_loss() {
    let ocr = accepted_backend();
    let descriptor = ocr.descriptor();
    let (engine, capture) = wired_with_options(
        Some(Arc::clone(&ocr)),
        EngineOptions::new()
            .with_diagnostics(DiagnosticOptions::normal(1).expect("bounded diagnostics")),
    );
    let reader = engine.take_diagnostic_reader().expect("enabled reader");
    let operation = OperationContext::new();
    let (session, frame) = opened_with_frame(&engine, &capture, &operation);
    let _ = reader.drain();

    for _ in 0..3 {
        let result = session
            .recognize(request(&frame, &descriptor, &operation))
            .expect("diagnostic capacity cannot alter OCR");
        assert_eq!(result.regions()[0].text(), "caller-secret-recognized-text");
    }
    let batch = diagnostic_batch(&reader);
    assert_eq!(batch.len(), 1);
    assert_eq!(batch.losses().normal(), 2);
    assert_eq!(batch.losses().debug(), 0);
    assert!(matches!(
        batch.records()[0].payload(),
        DiagnosticPayload::Ocr(value)
            if value.outcome == OcrDiagnosticOutcome::Recognized && value.result_count == 1
    ));
}
