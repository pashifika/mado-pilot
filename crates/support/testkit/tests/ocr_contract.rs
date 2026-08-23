//! Deterministic evidence for the platform-neutral OCR contract.

use std::sync::Arc;
use std::time::Duration;

use mado_pilot_capture::{Frame, PixelFormat};
use mado_pilot_core::{
    CancellationToken, ClipPolicy, CoordinateSpace, GeometryRevision, IdentityIssuer,
    MonotonicInstant, OperationContext, PixelExtent, Rect, Status, StreamCursor, TransformSnapshot,
};
use mado_pilot_ocr::{
    BackendCandidate, ModelComponentIdentity, OcrBackend, OcrModelSource, OcrModelSourceRequest,
    OcrRecognizer, OcrRegion, OcrRequest,
};
use mado_pilot_testkit::{
    CompletionGate, ControlledOcr, ControlledProducer, ManualClock, OcrBehavior, ocr_contract,
};
use sha2::{Digest, Sha256};

fn frame() -> Frame {
    ocr_contract::frame(PixelExtent::new(32, 24), PixelFormat::Bgra8, 0)
}

fn candidate(text: &[u8], order: u32) -> BackendCandidate {
    BackendCandidate::new(
        Arc::from(text),
        [(1.0, 1.0), (12.0, 1.0), (12.0, 8.0), (1.0, 8.0)],
        0.987_654,
        order,
    )
}

fn recognize(
    backend: Arc<dyn OcrBackend>,
    frame: &Frame,
    context: &OperationContext,
) -> mado_pilot_core::Result<mado_pilot_ocr::OcrResult> {
    let recognizer = OcrRecognizer::new(backend);
    let descriptor = recognizer.descriptor();
    recognizer.recognize(OcrRequest::new(
        frame,
        descriptor.id(),
        descriptor.model(),
        descriptor.profile(),
        OcrRegion::FullFrame,
        CoordinateSpace::CapturePixels,
        context,
    ))
}

#[test]
fn controlled_backend_passes_the_shared_contract_suite() {
    let backend: Arc<dyn OcrBackend> = Arc::new(ControlledOcr::new(PixelFormat::Bgra8));
    ocr_contract::run(&backend);
}

#[test]
fn success_empty_normalization_order_and_clipped_region_are_deterministic() {
    let backend: Arc<dyn OcrBackend> =
        Arc::new(ControlledOcr::new(PixelFormat::Bgra8).with_candidates(vec![
            candidate(b"second", 2),
            candidate("  e\u{301}  ".as_bytes(), 1),
            candidate(b" \t\n ", 0),
        ]));
    let source = frame();
    let result = recognize(backend, &source, &OperationContext::new()).unwrap();

    assert_eq!(
        result
            .regions()
            .iter()
            .map(|region| region.text())
            .collect::<Vec<_>>(),
        ["é", "second"]
    );
    assert_eq!(result.regions()[0].confidence().get(), 0.987_65);
    assert_eq!(result.stamp(), source.stamp());
    assert_eq!(result.output_space(), CoordinateSpace::CapturePixels);

    let backend = Arc::new(ControlledOcr::new(PixelFormat::Bgra8));
    let port: Arc<dyn OcrBackend> = backend.clone();
    let recognizer = OcrRecognizer::new(port);
    let descriptor = recognizer.descriptor();
    let outside = Rect::new(CoordinateSpace::CapturePixels, 40.0, 40.0, 50.0, 50.0).unwrap();
    let context = OperationContext::new();
    let error = recognizer
        .recognize(OcrRequest::new(
            &source,
            descriptor.id(),
            descriptor.model(),
            descriptor.profile(),
            OcrRegion::Region {
                rect: outside,
                policy: ClipPolicy::Clip,
            },
            CoordinateSpace::CapturePixels,
            &context,
        ))
        .unwrap_err();
    assert_eq!(error.status(), Status::InvalidArgument);
    assert_eq!(backend.recognition_count(), 0);
}

#[test]
fn malformed_backend_outputs_and_backend_failure_commit_nothing() {
    let cases = [
        vec![candidate(&[0xff], 0)],
        vec![BackendCandidate::new(
            Arc::from(&b"bad geometry"[..]),
            [(1.0, 1.0), (40.0, 1.0), (40.0, 8.0), (1.0, 8.0)],
            0.5,
            0,
        )],
        vec![BackendCandidate::new(
            Arc::from(&b"bad confidence"[..]),
            [(1.0, 1.0), (12.0, 1.0), (12.0, 8.0), (1.0, 8.0)],
            f64::NAN,
            0,
        )],
        vec![candidate(b"first", 0), candidate(b"duplicate", 0)],
    ];
    for candidates in cases {
        let backend: Arc<dyn OcrBackend> =
            Arc::new(ControlledOcr::new(PixelFormat::Bgra8).with_candidates(candidates));
        assert_eq!(
            recognize(backend, &frame(), &OperationContext::new())
                .unwrap_err()
                .status(),
            Status::VisionFailed
        );
    }

    let too_many = (0..=mado_pilot_ocr::MAX_CANDIDATES)
        .map(|order| {
            candidate(
                b"bounded",
                u32::try_from(order).expect("candidate ceiling fits u32"),
            )
        })
        .collect();
    let backend: Arc<dyn OcrBackend> =
        Arc::new(ControlledOcr::new(PixelFormat::Bgra8).with_candidates(too_many));
    assert_eq!(
        recognize(backend, &frame(), &OperationContext::new())
            .unwrap_err()
            .status(),
        Status::VisionFailed
    );

    let backend: Arc<dyn OcrBackend> =
        Arc::new(ControlledOcr::new(PixelFormat::Bgra8).recognizing(OcrBehavior::Fail));
    assert_eq!(
        recognize(backend, &frame(), &OperationContext::new())
            .unwrap_err()
            .status(),
        Status::VisionFailed
    );
}

#[test]
fn deadline_cancellation_and_close_use_the_same_operation_authority() {
    let clock = Arc::new(ManualClock::new());
    let backend: Arc<dyn OcrBackend> = Arc::new(
        ControlledOcr::new(PixelFormat::Bgra8)
            .with_candidates(vec![candidate(b"late", 0)])
            .with_latency(Arc::clone(&clock), Duration::from_millis(10)),
    );
    let context = OperationContext::new()
        .with_clock(clock)
        .with_timeout(Duration::from_millis(5))
        .unwrap();
    assert_eq!(
        recognize(backend, &frame(), &context).unwrap_err().status(),
        Status::DeadlineExceeded
    );

    let token = CancellationToken::new();
    let backend: Arc<dyn OcrBackend> = Arc::new(
        ControlledOcr::new(PixelFormat::Bgra8)
            .with_candidates(vec![candidate(b"cancelled", 0)])
            .cancelling(token.clone()),
    );
    let context = OperationContext::new().with_cancellation(token);
    assert_eq!(
        recognize(backend, &frame(), &context).unwrap_err().status(),
        Status::Cancelled
    );

    let backend = Arc::new(ControlledOcr::new(PixelFormat::Bgra8).closing(OcrBehavior::Fail));
    let port: Arc<dyn OcrBackend> = backend.clone();
    assert_eq!(
        OcrRecognizer::new(port)
            .close(&OperationContext::new())
            .unwrap_err()
            .status(),
        Status::VisionFailed
    );
    assert_eq!(backend.close_count(), 1);
}

#[test]
fn newer_completion_stays_authoritative_when_an_older_call_finishes_late() {
    let old_gate = Arc::new(CompletionGate::new());
    let new_gate = Arc::new(CompletionGate::new());
    let old_backend: Arc<dyn OcrBackend> = Arc::new(
        ControlledOcr::new(PixelFormat::Bgra8)
            .with_candidates(vec![candidate(b"old", 0)])
            .with_completion_gate(Arc::clone(&old_gate)),
    );
    let new_backend: Arc<dyn OcrBackend> = Arc::new(
        ControlledOcr::new(PixelFormat::Bgra8)
            .with_candidates(vec![candidate(b"new", 0)])
            .with_completion_gate(Arc::clone(&new_gate)),
    );
    let old_frame = frame();
    let old_stamp = old_frame.stamp();
    let new_frame = frame();
    let new_stamp = new_frame.stamp();
    let old_token = CancellationToken::new();
    let old_context = OperationContext::new().with_cancellation(old_token.clone());

    let old = std::thread::spawn(move || recognize(old_backend, &old_frame, &old_context));
    assert!(old_gate.wait_until_entered(Duration::from_secs(1)));
    let new =
        std::thread::spawn(move || recognize(new_backend, &new_frame, &OperationContext::new()));
    assert!(new_gate.wait_until_entered(Duration::from_secs(1)));

    new_gate.release();
    let newer = new.join().unwrap().unwrap();
    assert_eq!(newer.stamp(), new_stamp);
    assert_eq!(newer.regions()[0].text(), "new");

    old_token.cancel();
    old_gate.release();
    let older = old.join().unwrap().unwrap_err();
    assert_eq!(older.status(), Status::Cancelled);
    assert_ne!(newer.stamp(), old_stamp);
    assert_eq!(newer.regions()[0].text(), "new");
}

#[test]
fn retained_results_own_data_without_backend_frame_or_model_retention() {
    let raw_text: Arc<[u8]> = Arc::from(&b"owned"[..]);
    let backend = Arc::new(ControlledOcr::new(PixelFormat::Bgra8).with_candidates(vec![
        BackendCandidate::new(
            Arc::clone(&raw_text),
            [(1.0, 1.0), (12.0, 1.0), (12.0, 8.0), (1.0, 8.0)],
            0.5,
            0,
        ),
    ]));
    let port: Arc<dyn OcrBackend> = backend.clone();

    let producer =
        ControlledProducer::new(PixelExtent::new(32, 24), PixelFormat::Bgra8, 1, 1).unwrap();
    let storage = producer.capture(0).unwrap();
    let descriptor = storage.descriptor();
    let issuer = IdentityIssuer::new();
    let mut cursor = StreamCursor::new(issuer.issue_stream().unwrap());
    let stamp = cursor.publish(GeometryRevision::FIRST).unwrap();
    let source = Frame::from_storage(
        stamp,
        MonotonicInstant::ORIGIN,
        TransformSnapshot::frame_only(GeometryRevision::FIRST, descriptor.extent()),
        storage,
    )
    .unwrap();
    let result = recognize(port, &source, &OperationContext::new()).unwrap();
    assert_eq!(producer.producer_slots_free(), producer.pool());
    assert_eq!(producer.detached_slots_free(), 0);

    drop(source);
    drop(backend);
    assert_eq!(producer.detached_slots_free(), producer.detached_budget());
    assert_eq!(Arc::strong_count(&raw_text), 1);
    assert_eq!(result.regions()[0].text(), "owned");

    let detector: Arc<[u8]> = Arc::from(&b"detector"[..]);
    let recognizer: Arc<[u8]> = Arc::from(&b"recognizer"[..]);
    let descriptor = result.backend().clone();
    let source = OcrModelSource::new(OcrModelSourceRequest {
        model: descriptor.model().clone(),
        profile: descriptor.profile().clone(),
        detector: Arc::clone(&detector),
        detector_identity: ModelComponentIdentity::new(
            detector.len() as u64,
            Sha256::digest(&detector).into(),
        ),
        recognizer: Arc::clone(&recognizer),
        recognizer_identity: ModelComponentIdentity::new(
            recognizer.len() as u64,
            Sha256::digest(&recognizer).into(),
        ),
    })
    .unwrap();
    drop(source);
    assert_eq!(Arc::strong_count(&detector), 1);
    assert_eq!(Arc::strong_count(&recognizer), 1);
    assert_eq!(result.backend(), &descriptor);
}

#[test]
fn model_and_source_identity_mutations_are_observable() {
    let backend = Arc::new(ControlledOcr::new(PixelFormat::Bgra8));
    let port: Arc<dyn OcrBackend> = backend.clone();
    let recognizer = OcrRecognizer::new(port);
    let descriptor = recognizer.descriptor();
    let wrong_model = mado_pilot_ocr::ModelId::new("wrong-model").unwrap();
    let source = frame();
    let context = OperationContext::new();
    let error = recognizer
        .recognize(OcrRequest::new(
            &source,
            descriptor.id(),
            &wrong_model,
            descriptor.profile(),
            OcrRegion::FullFrame,
            CoordinateSpace::CapturePixels,
            &context,
        ))
        .unwrap_err();

    assert_eq!(error.status(), Status::InvalidArgument);
    assert_eq!(backend.recognition_count(), 0);
}
