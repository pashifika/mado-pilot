//! Deterministic evidence for the platform-neutral OCR contract.

use std::sync::Arc;
use std::time::Duration;

use mado_pilot_capture::{Frame, FrameDescriptor, PixelFormat};
use mado_pilot_core::{
    CancellationToken, ClipPolicy, CoordinateSpace, GeometryRevision, IdentityIssuer,
    MonotonicInstant, OperationContext, PixelExtent, Rect, Status, StreamCursor, TransformSnapshot,
};
use mado_pilot_ocr::{
    ACCEPTED_G004_NORMALIZATION_ID, BackendId, BackendVersion, DecoderId, LanguageProfileId,
    ModelComponentIdentity, ModelId, ModelVersion, NormalizationId, OcrBackend,
    OcrBackendDescriptor, OcrBackendIdentity, OcrModelComponent, OcrModelIdentity, OcrModelSource,
    OcrModelSourceRequest, OcrProfileMetadata, OcrRecognizer, OcrRegion, OcrRequest,
    PreprocessingId, ProfileId,
};
use mado_pilot_testkit::{
    CompletionGate, ControlledOcr, ControlledProducer, ManualClock, OcrBehavior, ScriptedOcrCall,
    ScriptedOcrCandidate, ocr_contract,
};
use sha2::{Digest, Sha256};

fn frame() -> Frame {
    ocr_contract::frame(PixelExtent::new(32, 24), PixelFormat::Bgra8, 0)
}

fn same_stream_frames() -> (Frame, Frame) {
    let descriptor = FrameDescriptor::packed(PixelExtent::new(32, 24), PixelFormat::Bgra8).unwrap();
    let issuer = IdentityIssuer::new();
    let mut cursor = StreamCursor::new(issuer.issue_stream().unwrap());
    let old_stamp = cursor.publish(GeometryRevision::FIRST).unwrap();
    let new_stamp = cursor.publish(GeometryRevision::FIRST).unwrap();
    let make = |stamp| {
        Frame::new(
            stamp,
            MonotonicInstant::ORIGIN,
            descriptor,
            TransformSnapshot::frame_only(GeometryRevision::FIRST, descriptor.extent()),
            vec![0; descriptor.byte_len()].into_boxed_slice(),
        )
        .unwrap()
    };
    (make(old_stamp), make(new_stamp))
}

fn candidate(text: &[u8], order: u32) -> ScriptedOcrCandidate {
    candidate_with(
        text,
        [(1.0, 1.0), (12.0, 1.0), (12.0, 8.0), (1.0, 8.0)],
        0.987_654,
        order,
    )
}

fn candidate_with(
    text: &[u8],
    quadrilateral: [(f64, f64); 4],
    confidence: f64,
    order: u32,
) -> ScriptedOcrCandidate {
    ScriptedOcrCandidate::new(Arc::<[u8]>::from(text), quadrilateral, confidence, order)
}

fn recognize(
    recognizer: &OcrRecognizer,
    frame: &Frame,
    context: &OperationContext,
) -> mado_pilot_core::Result<mado_pilot_ocr::OcrResult> {
    let descriptor = recognizer.descriptor();
    recognizer.recognize(OcrRequest::new(
        frame,
        descriptor.backend_identity(),
        descriptor.model_identity(),
        OcrRegion::FullFrame,
        CoordinateSpace::CapturePixels,
        context,
    ))
}

fn component_identity(bytes: &[u8]) -> ModelComponentIdentity {
    ModelComponentIdentity::new(
        u64::try_from(bytes.len()).unwrap(),
        Sha256::digest(bytes).into(),
    )
    .unwrap()
}

fn test_identity(
    detector: ModelComponentIdentity,
    recognizer: ModelComponentIdentity,
    normalization: &str,
) -> OcrModelIdentity {
    OcrModelIdentity::new(
        ModelId::new("test-model").unwrap(),
        ModelVersion::new("1").unwrap(),
        ProfileId::new("test-profile").unwrap(),
        detector,
        recognizer,
        OcrProfileMetadata::new(
            LanguageProfileId::new("test-language").unwrap(),
            PreprocessingId::new("test-preprocessing").unwrap(),
            DecoderId::new("test-decoder").unwrap(),
            NormalizationId::new(normalization).unwrap(),
            1,
            [7; 32],
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn controlled_backend_passes_the_shared_contract_suite() {
    let backend: Arc<dyn OcrBackend> = Arc::new(ControlledOcr::new(PixelFormat::Bgra8));
    ocr_contract::run(&backend);
}

#[test]
fn bounded_profile_results_preserve_source_identity_geometry_order_and_output_space() {
    let model = OcrModelIdentity::accepted_bounded_detector();
    let backend_identity = OcrBackendIdentity::new(
        BackendId::new("controlled-bounded").unwrap(),
        BackendVersion::new("1").unwrap(),
    );
    let backend = Arc::new(
        ControlledOcr::new(PixelFormat::Bgra8)
            .with_descriptor(OcrBackendDescriptor::new(
                backend_identity,
                model.clone(),
                PixelFormat::Bgra8,
            ))
            .with_candidates(vec![
                candidate_with(
                    b"second",
                    [(4.0, 3.0), (10.0, 3.0), (10.0, 7.0), (4.0, 7.0)],
                    0.75,
                    1,
                ),
                candidate_with(
                    b"first",
                    [(1.0, 1.0), (6.0, 1.0), (6.0, 5.0), (1.0, 5.0)],
                    0.875,
                    0,
                ),
            ]),
    );
    let port: Arc<dyn OcrBackend> = backend.clone();
    let recognizer = OcrRecognizer::new(port);
    let descriptor = FrameDescriptor::packed(PixelExtent::new(32, 24), PixelFormat::Bgra8)
        .expect("bounded contract frame");
    let issuer = IdentityIssuer::new();
    let mut cursor = StreamCursor::new(issuer.issue_stream().unwrap());
    let stamp = cursor.publish(GeometryRevision::FIRST).unwrap();
    let source = Frame::new(
        stamp,
        MonotonicInstant::ORIGIN,
        descriptor,
        TransformSnapshot::with_target_extent(GeometryRevision::FIRST, descriptor.extent()),
        vec![0; descriptor.byte_len()].into_boxed_slice(),
    )
    .unwrap();
    let selected = recognizer.descriptor();
    let requested = Rect::new(CoordinateSpace::CapturePixels, 8.0, 6.0, 24.0, 18.0).unwrap();

    let result = recognizer
        .recognize(OcrRequest::new(
            &source,
            selected.backend_identity(),
            selected.model_identity(),
            OcrRegion::Region {
                rect: requested,
                policy: ClipPolicy::Clip,
            },
            CoordinateSpace::TargetNormalized,
            &OperationContext::new(),
        ))
        .unwrap();

    assert_eq!(result.stamp(), stamp);
    assert_eq!(
        result.effective_region(),
        mado_pilot_core::PixelRect::new(8, 6, 24, 18).unwrap()
    );
    assert_eq!(result.output_space(), CoordinateSpace::TargetNormalized);
    assert_eq!(
        result.backend().backend_identity(),
        selected.backend_identity()
    );
    assert_eq!(result.backend().model_identity(), &model);
    assert_eq!(
        result
            .regions()
            .iter()
            .map(|region| region.text())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
    assert_eq!(
        result.regions()[0].geometry().points()[0],
        mado_pilot_core::Point::new(CoordinateSpace::TargetNormalized, 9.0 / 32.0, 7.0 / 24.0,)
            .unwrap()
    );
    assert_eq!(
        backend.last_region(),
        Some(mado_pilot_core::PixelRect::new(8, 6, 24, 18).unwrap())
    );
}

#[test]
fn success_normalization_order_limits_and_nonempty_clipping_are_deterministic() {
    let backend = Arc::new(ControlledOcr::new(PixelFormat::Bgra8).with_candidates(vec![
        candidate(b"second", 2),
        candidate("  e\u{301}  ".as_bytes(), 1),
        candidate(b" \t\n ", 0),
    ]));
    let port: Arc<dyn OcrBackend> = backend.clone();
    let recognizer = OcrRecognizer::new(port);
    let source = frame();
    let result = recognize(&recognizer, &source, &OperationContext::new()).unwrap();

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
    assert_eq!(
        backend.last_max_candidates(),
        mado_pilot_ocr::MAX_CANDIDATES
    );
    assert_eq!(
        backend.last_max_text_bytes(),
        mado_pilot_ocr::MAX_BACKEND_TEXT_BYTES
    );

    let clipped_backend =
        Arc::new(
            ControlledOcr::new(PixelFormat::Bgra8).with_candidates(vec![candidate_with(
                b"clipped",
                [(1.0, 1.0), (6.0, 1.0), (6.0, 5.0), (1.0, 5.0)],
                0.5,
                0,
            )]),
        );
    let port: Arc<dyn OcrBackend> = clipped_backend.clone();
    let clipped_recognizer = OcrRecognizer::new(port);
    let descriptor = clipped_recognizer.descriptor();
    let requested = Rect::new(CoordinateSpace::CapturePixels, 25.0, 18.0, 40.0, 30.0).unwrap();
    let context = OperationContext::new();
    let clipped = clipped_recognizer
        .recognize(OcrRequest::new(
            &source,
            descriptor.backend_identity(),
            descriptor.model_identity(),
            OcrRegion::Region {
                rect: requested,
                policy: ClipPolicy::Clip,
            },
            CoordinateSpace::CapturePixels,
            &context,
        ))
        .unwrap();
    let effective = mado_pilot_core::PixelRect::new(25, 18, 32, 24).unwrap();
    assert_eq!(clipped_backend.last_region(), Some(effective));
    assert_eq!(clipped.effective_region(), effective);
    assert_eq!(
        clipped.regions()[0].geometry().points()[0],
        mado_pilot_core::Point::new(CoordinateSpace::CapturePixels, 26.0, 19.0).unwrap()
    );
}

#[test]
fn empty_clipped_and_malformed_outputs_commit_nothing() {
    let source = frame();
    let backend = Arc::new(ControlledOcr::new(PixelFormat::Bgra8));
    let port: Arc<dyn OcrBackend> = backend.clone();
    let recognizer = OcrRecognizer::new(port);
    let descriptor = recognizer.descriptor();
    let outside = Rect::new(CoordinateSpace::CapturePixels, 40.0, 40.0, 50.0, 50.0).unwrap();
    let context = OperationContext::new();
    assert_eq!(
        recognizer
            .recognize(OcrRequest::new(
                &source,
                descriptor.backend_identity(),
                descriptor.model_identity(),
                OcrRegion::Region {
                    rect: outside,
                    policy: ClipPolicy::Clip,
                },
                CoordinateSpace::CapturePixels,
                &context,
            ))
            .unwrap_err()
            .status(),
        Status::InvalidArgument
    );
    assert_eq!(backend.recognition_count(), 0);

    let over_backend_text = vec![b'a'; mado_pilot_ocr::MAX_BACKEND_TEXT_BYTES + 1];
    let over_retained_text = vec![b'a'; mado_pilot_ocr::MAX_TEXT_BYTES + 1];
    let cases = [
        vec![candidate(&over_backend_text, 0)],
        vec![candidate(&over_retained_text, 0)],
        vec![candidate(&[0xff], 0)],
        vec![candidate_with(
            b"bad geometry",
            [(1.0, 1.0), (40.0, 1.0), (40.0, 8.0), (1.0, 8.0)],
            0.5,
            0,
        )],
        vec![candidate_with(
            b"bad confidence",
            [(1.0, 1.0), (12.0, 1.0), (12.0, 8.0), (1.0, 8.0)],
            f64::NAN,
            0,
        )],
        vec![candidate(b"first", 0), candidate(b"duplicate", 0)],
    ];
    for candidates in cases {
        let backend: Arc<dyn OcrBackend> =
            Arc::new(ControlledOcr::new(PixelFormat::Bgra8).with_candidates(candidates));
        let recognizer = OcrRecognizer::new(backend);
        assert_eq!(
            recognize(&recognizer, &frame(), &OperationContext::new())
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
        recognize(
            &OcrRecognizer::new(backend),
            &frame(),
            &OperationContext::new()
        )
        .unwrap_err()
        .status(),
        Status::VisionFailed
    );

    let backend: Arc<dyn OcrBackend> = Arc::new(
        ControlledOcr::new(PixelFormat::Bgra8)
            .with_candidates(vec![candidate(b"must-not-commit", 0)])
            .recognizing(OcrBehavior::Fail),
    );
    assert_eq!(
        recognize(
            &OcrRecognizer::new(backend),
            &frame(),
            &OperationContext::new()
        )
        .unwrap_err()
        .status(),
        Status::VisionFailed
    );
}

#[test]
fn interruption_is_authoritative_over_backend_normalization_and_close_failures() {
    let clock = Arc::new(ManualClock::new());
    let backend: Arc<dyn OcrBackend> = Arc::new(
        ControlledOcr::new(PixelFormat::Bgra8)
            .with_latency(Arc::clone(&clock), Duration::from_millis(10))
            .recognizing(OcrBehavior::Fail),
    );
    let context = OperationContext::new()
        .with_clock(clock)
        .with_timeout(Duration::from_millis(5))
        .unwrap();
    assert_eq!(
        recognize(&OcrRecognizer::new(backend), &frame(), &context)
            .unwrap_err()
            .status(),
        Status::DeadlineExceeded
    );

    let token = CancellationToken::new();
    let backend: Arc<dyn OcrBackend> = Arc::new(
        ControlledOcr::new(PixelFormat::Bgra8)
            .with_candidates(vec![candidate(&[0xff], 0)])
            .cancelling(token.clone())
            .recognizing(OcrBehavior::Fail),
    );
    let context = OperationContext::new().with_cancellation(token);
    assert_eq!(
        recognize(&OcrRecognizer::new(backend), &frame(), &context)
            .unwrap_err()
            .status(),
        Status::Cancelled
    );

    let token = CancellationToken::new();
    let backend: Arc<dyn OcrBackend> = Arc::new(
        ControlledOcr::new(PixelFormat::Bgra8)
            .cancelling(token.clone())
            .recognizing(OcrBehavior::Fail),
    );
    let context = OperationContext::new().with_cancellation(token);
    assert_eq!(
        recognize(&OcrRecognizer::new(backend), &frame(), &context)
            .unwrap_err()
            .status(),
        Status::Cancelled
    );

    let token = CancellationToken::new();
    let backend = Arc::new(
        ControlledOcr::new(PixelFormat::Bgra8)
            .cancelling_close(token.clone())
            .closing(OcrBehavior::Fail),
    );
    let port: Arc<dyn OcrBackend> = backend.clone();
    assert_eq!(
        OcrRecognizer::new(port)
            .close(&OperationContext::new().with_cancellation(token))
            .unwrap_err()
            .status(),
        Status::Cancelled
    );
    assert_eq!(backend.close_count(), 1);
}

#[test]
fn out_of_order_calls_share_one_recognizer_without_result_replacement() {
    let old_gate = Arc::new(CompletionGate::new());
    let new_gate = Arc::new(CompletionGate::new());
    let backend = Arc::new(ControlledOcr::new(PixelFormat::Bgra8).with_calls(vec![
            ScriptedOcrCall::new(vec![candidate(b"old", 0)])
                .with_completion_gate(Arc::clone(&old_gate)),
            ScriptedOcrCall::new(vec![candidate(b"new", 0)])
                .with_completion_gate(Arc::clone(&new_gate)),
        ]));
    let port: Arc<dyn OcrBackend> = backend;
    let recognizer = OcrRecognizer::new(port);
    let (old_frame, new_frame) = same_stream_frames();
    let old_stamp = old_frame.stamp();
    let new_stamp = new_frame.stamp();

    let old_recognizer = recognizer.clone();
    let old = std::thread::spawn(move || {
        recognize(&old_recognizer, &old_frame, &OperationContext::new())
    });
    assert!(old_gate.wait_until_entered(Duration::from_secs(1)));
    let new_recognizer = recognizer.clone();
    let new = std::thread::spawn(move || {
        recognize(&new_recognizer, &new_frame, &OperationContext::new())
    });
    assert!(new_gate.wait_until_entered(Duration::from_secs(1)));

    new_gate.release();
    let newer = new.join().unwrap().unwrap();
    assert_eq!(newer.stamp(), new_stamp);
    assert_eq!(newer.regions()[0].text(), "new");

    old_gate.release();
    let older = old.join().unwrap().unwrap();
    assert_eq!(older.stamp(), old_stamp);
    assert_eq!(older.regions()[0].text(), "old");
    assert_eq!(newer.stamp(), new_stamp);
    assert_eq!(newer.regions()[0].text(), "new");
}

#[test]
fn retained_results_own_data_without_backend_frame_or_model_retention() {
    let raw_text: Arc<[u8]> = Arc::from(&b"private-owned-text"[..]);
    let backend = Arc::new(ControlledOcr::new(PixelFormat::Bgra8).with_candidates(vec![
        ScriptedOcrCandidate::new(
            Arc::clone(&raw_text),
            [(1.0, 1.0), (12.0, 1.0), (12.0, 8.0), (1.0, 8.0)],
            0.5,
            0,
        ),
    ]));
    let port: Arc<dyn OcrBackend> = backend.clone();
    let recognizer_port = OcrRecognizer::new(port);

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
    let result = recognize(&recognizer_port, &source, &OperationContext::new()).unwrap();
    assert_eq!(producer.producer_slots_free(), producer.pool());
    assert_eq!(producer.detached_slots_free(), 0);

    drop(source);
    drop(recognizer_port);
    drop(backend);
    assert_eq!(producer.detached_slots_free(), producer.detached_budget());
    assert_eq!(Arc::strong_count(&raw_text), 1);
    assert_eq!(result.regions()[0].text(), "private-owned-text");
    let debug = format!("{result:?}");
    assert!(!debug.contains("private-owned-text"));

    let detector: Arc<[u8]> = Arc::from(&b"private-detector"[..]);
    let recognizer: Arc<[u8]> = Arc::from(&b"private-recognizer"[..]);
    let detector_identity = component_identity(&detector);
    let recognizer_identity = component_identity(&recognizer);
    let source = OcrModelSource::new(OcrModelSourceRequest {
        identity: test_identity(
            detector_identity,
            recognizer_identity,
            ACCEPTED_G004_NORMALIZATION_ID,
        ),
        detector: OcrModelComponent::new(Arc::clone(&detector), detector_identity).unwrap(),
        recognizer: OcrModelComponent::new(Arc::clone(&recognizer), recognizer_identity).unwrap(),
    })
    .unwrap();
    let source_debug = format!("{source:?}");
    assert!(!source_debug.contains("private-detector"));
    assert!(!source_debug.contains("private-recognizer"));
    drop(source);
    assert_eq!(Arc::strong_count(&detector), 1);
    assert_eq!(Arc::strong_count(&recognizer), 1);
}

#[test]
fn unsupported_profile_and_backend_or_model_identity_mutations_fail_before_backend_work() {
    let backend = Arc::new(ControlledOcr::new(PixelFormat::Bgra8));
    let port: Arc<dyn OcrBackend> = backend.clone();
    let recognizer = OcrRecognizer::new(port);
    let descriptor = recognizer.descriptor();
    let selected = descriptor.model_identity();
    let wrong_model = OcrModelIdentity::new(
        ModelId::new("wrong-model").unwrap(),
        selected.version().clone(),
        selected.profile().clone(),
        selected.detector(),
        selected.recognizer(),
        selected.profile_metadata().clone(),
    )
    .unwrap();
    let source = frame();
    let context = OperationContext::new();
    assert_eq!(
        recognizer
            .recognize(OcrRequest::new(
                &source,
                descriptor.backend_identity(),
                &wrong_model,
                OcrRegion::FullFrame,
                CoordinateSpace::CapturePixels,
                &context,
            ))
            .unwrap_err()
            .status(),
        Status::InvalidArgument
    );
    assert_eq!(backend.recognition_count(), 0);

    let wrong_backend =
        OcrBackendIdentity::new(descriptor.id().clone(), BackendVersion::new("2").unwrap());
    assert_eq!(
        recognizer
            .recognize(OcrRequest::new(
                &source,
                &wrong_backend,
                descriptor.model_identity(),
                OcrRegion::FullFrame,
                CoordinateSpace::CapturePixels,
                &context,
            ))
            .unwrap_err()
            .status(),
        Status::InvalidArgument
    );
    assert_eq!(backend.recognition_count(), 0);

    let unsupported = OcrProfileMetadata::new(
        selected.profile_metadata().language_profile().clone(),
        selected.profile_metadata().preprocessing().clone(),
        selected.profile_metadata().decoder().clone(),
        NormalizationId::new("unsupported-normalization").unwrap(),
        selected.profile_metadata().vocabulary_entries(),
        selected.profile_metadata().vocabulary_sha256(),
    )
    .unwrap_err();
    assert_eq!(unsupported.status(), Status::Unsupported);
}
