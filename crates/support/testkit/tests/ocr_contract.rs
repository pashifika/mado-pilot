//! Deterministic evidence for the platform-neutral OCR contract.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use mado_pilot_capture::{CpuPixels, Frame, FrameDescriptor, FrameStorage, PixelFormat};
use mado_pilot_core::{
    CancellationToken, ClipPolicy, Clock, CoordinateSpace, GeometryRevision, IdentityIssuer,
    MonotonicInstant, OperationContext, PixelExtent, PixelRect, Rect, Status, StreamCursor,
    TransformSnapshot,
};
use mado_pilot_ocr::{
    ACCEPTED_G004_NORMALIZATION_ID, BackendId, BackendVersion, DecoderId, LanguageProfileId,
    ModelComponentIdentity, ModelId, ModelVersion, NormalizationId, OcrBackend,
    OcrBackendDescriptor, OcrBackendIdentity, OcrModelComponent, OcrModelIdentity, OcrModelSource,
    OcrModelSourceRequest, OcrProfileMetadata, OcrRecognizer, OcrRegion, OcrRequest, OcrZone,
    OcrZoneScanRequest, OcrZoneScanResult, PreprocessingId, ProfileId,
};
use mado_pilot_testkit::{
    CompletionGate, ControlledOcr, ControlledProducer, ManualClock, OcrBehavior, ScriptedOcrCall,
    ScriptedOcrCandidate, ocr_contract,
};
use sha2::{Digest, Sha256};

type ScriptedOutput = (&'static [u8], [(f64, f64); 4], u32);

fn frame() -> Frame {
    ocr_contract::frame(PixelExtent::new(32, 24), PixelFormat::Bgra8, 0)
}

#[derive(Debug)]
struct PreflightOnlyStorage {
    descriptor: FrameDescriptor,
}

impl FrameStorage for PreflightOnlyStorage {
    fn descriptor(&self) -> FrameDescriptor {
        self.descriptor
    }

    fn cpu_pixels(&self) -> Option<Arc<CpuPixels>> {
        None
    }

    fn read_cpu(&self, _operation: &OperationContext) -> mado_pilot_core::Result<Arc<CpuPixels>> {
        panic!("mapping preflight must reject this storage before conversion")
    }
}

#[derive(Debug)]
struct CountingDeadlineClock {
    calls: AtomicUsize,
    expire_at: Option<usize>,
}

impl CountingDeadlineClock {
    fn new(expire_at: Option<usize>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            expire_at,
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::Acquire)
    }
}

impl Clock for CountingDeadlineClock {
    fn now(&self) -> MonotonicInstant {
        let call = self.calls.fetch_add(1, Ordering::AcqRel) + 1;
        if self.expire_at.is_some_and(|expire_at| call >= expire_at) {
            MonotonicInstant::ORIGIN
                .checked_add(Duration::from_secs(1))
                .unwrap()
        } else {
            MonotonicInstant::ORIGIN
        }
    }
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

fn zone(left: f64, top: f64, right: f64, bottom: f64, policy: ClipPolicy) -> OcrZone {
    OcrZone::new(
        Rect::new(CoordinateSpace::CapturePixels, left, top, right, bottom).unwrap(),
        policy,
    )
}

fn scan_zones(
    recognizer: &OcrRecognizer,
    frame: &Frame,
    zones: &[OcrZone],
    context: &OperationContext,
) -> mado_pilot_core::Result<OcrZoneScanResult> {
    let descriptor = recognizer.descriptor();
    recognizer.scan_zones(OcrZoneScanRequest::new(
        frame,
        descriptor.backend_identity(),
        descriptor.model_identity(),
        zones,
        CoordinateSpace::CapturePixels,
        context,
    )?)
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
fn zone_scan_request_is_bounded_borrowed_and_fully_explicit() {
    let backend = Arc::new(ControlledOcr::new(PixelFormat::Bgra8));
    let recognizer = OcrRecognizer::new(backend.clone());
    let descriptor = recognizer.descriptor();
    let source = frame();
    let context = OperationContext::new();
    let zones = [zone(1.0, 2.0, 10.0, 12.0, ClipPolicy::Clip)];

    let request = OcrZoneScanRequest::new(
        &source,
        descriptor.backend_identity(),
        descriptor.model_identity(),
        &zones,
        CoordinateSpace::CapturePixels,
        &context,
    )
    .unwrap();

    assert!(std::ptr::eq(request.frame(), &source));
    assert_eq!(request.backend(), descriptor.backend_identity());
    assert_eq!(request.model_identity(), descriptor.model_identity());
    assert_eq!(request.model(), descriptor.model());
    assert_eq!(request.profile(), descriptor.profile());
    assert_eq!(request.zones(), zones);
    assert_eq!(request.zones()[0].rect(), zones[0].rect());
    assert_eq!(request.zones()[0].policy(), ClipPolicy::Clip);
    assert_eq!(request.output_space(), CoordinateSpace::CapturePixels);
    assert!(std::ptr::eq(request.operation(), &context));

    assert_eq!(
        OcrZoneScanRequest::new(
            &source,
            descriptor.backend_identity(),
            descriptor.model_identity(),
            &[],
            CoordinateSpace::CapturePixels,
            &context,
        )
        .unwrap_err()
        .status(),
        Status::InvalidArgument
    );
    let nine = [zones[0]; 9];
    assert_eq!(
        OcrZoneScanRequest::new(
            &source,
            descriptor.backend_identity(),
            descriptor.model_identity(),
            &nine,
            CoordinateSpace::CapturePixels,
            &context,
        )
        .unwrap_err()
        .status(),
        Status::InvalidArgument
    );
    assert_eq!(backend.recognition_count(), 0);
}

#[test]
fn grouped_debug_omits_model_fingerprints_and_recognized_text() {
    let model = OcrModelIdentity::new(
        ModelId::new("debug-sentinel-model").unwrap(),
        ModelVersion::new("1").unwrap(),
        ProfileId::new("debug-sentinel-profile").unwrap(),
        ModelComponentIdentity::new(1, [165; 32]).unwrap(),
        ModelComponentIdentity::new(1, [182; 32]).unwrap(),
        OcrProfileMetadata::new(
            LanguageProfileId::new("debug-language").unwrap(),
            PreprocessingId::new("debug-preprocessing").unwrap(),
            DecoderId::new("debug-decoder").unwrap(),
            NormalizationId::new(ACCEPTED_G004_NORMALIZATION_ID).unwrap(),
            1,
            [199; 32],
        )
        .unwrap(),
    )
    .unwrap();
    let backend_identity = OcrBackendIdentity::new(
        BackendId::new("debug-backend").unwrap(),
        BackendVersion::new("1").unwrap(),
    );
    let backend = Arc::new(
        ControlledOcr::new(PixelFormat::Bgra8)
            .with_descriptor(OcrBackendDescriptor::new(
                backend_identity,
                model,
                PixelFormat::Bgra8,
            ))
            .with_candidates(vec![candidate(b"private-grouped-text", 0)]),
    );
    let recognizer = OcrRecognizer::new(backend);
    let descriptor = recognizer.descriptor();
    let source = frame();
    let context = OperationContext::new();
    let zones = [zone(0.0, 0.0, 32.0, 24.0, ClipPolicy::Reject)];
    let request = OcrZoneScanRequest::new(
        &source,
        descriptor.backend_identity(),
        descriptor.model_identity(),
        &zones,
        CoordinateSpace::CapturePixels,
        &context,
    )
    .unwrap();

    let request_debug = format!("{request:?}");
    for digest in ["165, 165, 165", "182, 182, 182", "199, 199, 199"] {
        assert!(!request_debug.contains(digest));
    }
    assert!(request_debug.contains("debug-sentinel-model"));
    assert!(request_debug.contains("debug-sentinel-profile"));

    let result = recognizer.scan_zones(request).unwrap();
    let result_debug = format!("{result:?}");
    for sensitive in [
        "165, 165, 165",
        "182, 182, 182",
        "199, 199, 199",
        "private-grouped-text",
    ] {
        assert!(!result_debug.contains(sensitive));
    }
    assert!(result_debug.contains("debug-sentinel-model"));
    assert!(result_debug.contains("debug-sentinel-profile"));
}

#[test]
fn grouped_scan_maps_one_envelope_and_owns_unique_ordered_candidates() {
    let zones = [
        zone(0.0, 0.0, 8.0, 8.0, ClipPolicy::Reject),
        zone(12.0, 0.0, 20.0, 8.0, ClipPolicy::Reject),
        zone(0.0, 12.0, 8.0, 20.0, ClipPolicy::Reject),
    ];
    let backend = Arc::new(ControlledOcr::new(PixelFormat::Bgra8).with_candidates(vec![
        candidate_with(
            b"third",
            [(1.0, 13.0), (5.0, 13.0), (5.0, 17.0), (1.0, 17.0)],
            0.7,
            2,
        ),
        candidate_with(
            b"outside",
            [(9.0, 9.0), (11.0, 9.0), (11.0, 11.0), (9.0, 11.0)],
            0.6,
            3,
        ),
        candidate_with(
            b"first",
            [(1.0, 1.0), (5.0, 1.0), (5.0, 5.0), (1.0, 5.0)],
            0.9,
            0,
        ),
    ]));
    let recognizer = OcrRecognizer::new(backend.clone());
    let source = frame();
    let result = scan_zones(&recognizer, &source, &zones, &OperationContext::new()).unwrap();

    assert_eq!(backend.recognition_count(), 1);
    assert_eq!(
        backend.last_region(),
        Some(PixelRect::new(0, 0, 20, 20).unwrap())
    );
    assert_eq!(
        backend.last_interests().unwrap(),
        vec![
            PixelRect::new(0, 0, 8, 8).unwrap(),
            PixelRect::new(12, 0, 20, 8).unwrap(),
            PixelRect::new(0, 12, 8, 20).unwrap(),
        ]
    );
    assert_eq!(result.stamp(), source.stamp());
    assert_eq!(result.transform(), source.transform());
    assert_eq!(
        result.source_envelope(),
        PixelRect::new(0, 0, 20, 20).unwrap()
    );
    assert_eq!(result.effective_zones(), backend.last_interests().unwrap());
    assert_eq!(result.unique_candidates().len(), 2);
    assert_eq!(result.unique_candidates()[0].text(), "first");
    assert_eq!(result.unique_candidates()[1].text(), "third");
    assert_eq!(result.group(0).unwrap().len(), 1);
    assert_eq!(result.group(0).unwrap().get(0).unwrap().text(), "first");
    assert!(result.group(1).unwrap().is_empty());
    assert_eq!(
        result
            .group(2)
            .unwrap()
            .iter()
            .map(|region| region.text())
            .collect::<Vec<_>>(),
        vec!["third"]
    );
    assert!(result.group(3).is_none());
}

#[test]
fn qualified_eight_zone_fixture_and_caller_reordering_are_deterministic() {
    let zones = [
        zone(4.0, 2.0, 44.0, 12.0, ClipPolicy::Reject),
        zone(52.0, 2.0, 92.0, 12.0, ClipPolicy::Reject),
        zone(4.0, 15.0, 44.0, 25.0, ClipPolicy::Reject),
        zone(52.0, 15.0, 92.0, 25.0, ClipPolicy::Reject),
        zone(4.0, 28.0, 44.0, 38.0, ClipPolicy::Reject),
        zone(52.0, 28.0, 92.0, 38.0, ClipPolicy::Reject),
        zone(4.0, 41.0, 44.0, 51.0, ClipPolicy::Reject),
        zone(52.0, 41.0, 92.0, 51.0, ClipPolicy::Reject),
    ];
    let outputs: [ScriptedOutput; 8] = [
        (
            b"left-name",
            [(1.0, 1.0), (8.0, 1.0), (8.0, 5.0), (1.0, 5.0)],
            0,
        ),
        (
            b"right-level",
            [(49.0, 1.0), (56.0, 1.0), (56.0, 5.0), (49.0, 5.0)],
            1,
        ),
        (
            b"left-health",
            [(1.0, 14.0), (8.0, 14.0), (8.0, 18.0), (1.0, 18.0)],
            2,
        ),
        (
            b"right-mana",
            [(49.0, 14.0), (56.0, 14.0), (56.0, 18.0), (49.0, 18.0)],
            3,
        ),
        (
            b"left-quest",
            [(1.0, 27.0), (8.0, 27.0), (8.0, 31.0), (1.0, 31.0)],
            4,
        ),
        (
            b"right-code",
            [(49.0, 27.0), (56.0, 27.0), (56.0, 31.0), (49.0, 31.0)],
            5,
        ),
        (
            b"left-next",
            [(1.0, 40.0), (8.0, 40.0), (8.0, 44.0), (1.0, 44.0)],
            6,
        ),
        (
            b"right-ready",
            [(49.0, 40.0), (56.0, 40.0), (56.0, 44.0), (49.0, 44.0)],
            7,
        ),
    ];
    let candidates = outputs
        .iter()
        .rev()
        .map(|(text, quadrilateral, order)| candidate_with(text, *quadrilateral, 0.9, *order))
        .collect();
    let backend = Arc::new(ControlledOcr::new(PixelFormat::Bgra8).with_candidates(candidates));
    let recognizer = OcrRecognizer::new(backend.clone());
    let source = ocr_contract::frame(PixelExtent::new(96, 54), PixelFormat::Bgra8, 0);

    let result = scan_zones(&recognizer, &source, &zones, &OperationContext::new()).unwrap();
    assert_eq!(
        result.source_envelope(),
        PixelRect::new(4, 2, 92, 51).unwrap()
    );
    for (group, (expected, _, _)) in outputs.iter().enumerate() {
        assert_eq!(
            result.group(group).unwrap().get(0).unwrap().text(),
            std::str::from_utf8(expected).unwrap()
        );
    }

    let reordered = [zones[7], zones[0], zones[3]];
    let result = scan_zones(&recognizer, &source, &reordered, &OperationContext::new()).unwrap();
    assert_eq!(
        (0..3)
            .map(|group| result.group(group).unwrap().get(0).unwrap().text())
            .collect::<Vec<_>>(),
        ["right-ready", "left-name", "right-mana"]
    );
    assert_eq!(backend.recognition_count(), 2);
}

#[test]
fn one_zone_grouped_output_equals_singular_output() {
    let candidates = vec![
        candidate_with(
            b"second",
            [(10.0, 2.0), (16.0, 2.0), (16.0, 6.0), (10.0, 6.0)],
            0.75,
            1,
        ),
        candidate_with(
            b"first",
            [(1.0, 1.0), (6.0, 1.0), (6.0, 5.0), (1.0, 5.0)],
            0.875,
            0,
        ),
    ];
    let backend = Arc::new(ControlledOcr::new(PixelFormat::Bgra8).with_candidates(candidates));
    let recognizer = OcrRecognizer::new(backend);
    let source = frame();
    let singular = recognize(&recognizer, &source, &OperationContext::new()).unwrap();
    let zones = [zone(0.0, 0.0, 32.0, 24.0, ClipPolicy::Reject)];
    let grouped = scan_zones(&recognizer, &source, &zones, &OperationContext::new()).unwrap();

    assert_eq!(grouped.source_envelope(), singular.effective_region());
    assert_eq!(grouped.backend(), singular.backend());
    assert_eq!(grouped.unique_candidates(), singular.regions());
    assert_eq!(
        grouped.group(0).unwrap().iter().collect::<Vec<_>>(),
        singular.regions().iter().collect::<Vec<_>>()
    );
}

#[test]
fn honored_and_ignored_interests_commit_the_same_groups() {
    let candidates = vec![
        candidate_with(
            b"in-zone",
            [(1.0, 1.0), (5.0, 1.0), (5.0, 5.0), (1.0, 5.0)],
            0.9,
            0,
        ),
        candidate_with(
            b"outside",
            [(9.0, 1.0), (13.0, 1.0), (13.0, 5.0), (9.0, 5.0)],
            0.8,
            1,
        ),
    ];
    let ignored =
        Arc::new(ControlledOcr::new(PixelFormat::Bgra8).with_candidates(candidates.clone()));
    let honored = Arc::new(
        ControlledOcr::new(PixelFormat::Bgra8)
            .with_candidates(candidates)
            .honoring_interests(),
    );
    let source = frame();
    let zones = [
        zone(0.0, 0.0, 8.0, 8.0, ClipPolicy::Reject),
        zone(16.0, 0.0, 20.0, 8.0, ClipPolicy::Reject),
    ];

    let ignored_result = scan_zones(
        &OcrRecognizer::new(ignored.clone()),
        &source,
        &zones,
        &OperationContext::new(),
    )
    .unwrap();
    let honored_result = scan_zones(
        &OcrRecognizer::new(honored.clone()),
        &source,
        &zones,
        &OperationContext::new(),
    )
    .unwrap();

    assert_eq!(ignored_result, honored_result);
    assert_eq!(ignored.last_selected_candidates(), 2);
    assert_eq!(ignored.last_ignored_candidates(), 0);
    assert_eq!(honored.last_selected_candidates(), 1);
    assert_eq!(honored.last_ignored_candidates(), 1);
}

#[test]
fn controlled_backend_resets_latest_candidate_counts_after_failed_call() {
    let candidates = vec![
        candidate_with(
            b"in-zone",
            [(1.0, 1.0), (5.0, 1.0), (5.0, 5.0), (1.0, 5.0)],
            0.9,
            0,
        ),
        candidate_with(
            b"outside",
            [(9.0, 1.0), (13.0, 1.0), (13.0, 5.0), (9.0, 5.0)],
            0.8,
            1,
        ),
    ];
    let backend = Arc::new(
        ControlledOcr::new(PixelFormat::Bgra8)
            .with_calls(vec![
                ScriptedOcrCall::new(candidates),
                ScriptedOcrCall::new(Vec::new()).with_behavior(OcrBehavior::Unavailable),
            ])
            .honoring_interests(),
    );
    let recognizer = OcrRecognizer::new(backend.clone());
    let source = frame();
    let zones = [
        zone(0.0, 0.0, 8.0, 8.0, ClipPolicy::Reject),
        zone(16.0, 0.0, 20.0, 8.0, ClipPolicy::Reject),
    ];

    scan_zones(&recognizer, &source, &zones, &OperationContext::new()).unwrap();
    assert_eq!(backend.last_selected_candidates(), 1);
    assert_eq!(backend.last_ignored_candidates(), 1);

    assert_eq!(
        scan_zones(&recognizer, &source, &zones, &OperationContext::new())
            .unwrap_err()
            .status(),
        Status::VisionFailed
    );
    assert_eq!(backend.last_selected_candidates(), 0);
    assert_eq!(backend.last_ignored_candidates(), 0);
}

#[test]
fn grouped_scan_clips_every_zone_and_refuses_any_empty_effective_zone_before_backend() {
    let backend = Arc::new(ControlledOcr::new(PixelFormat::Bgra8));
    let recognizer = OcrRecognizer::new(backend.clone());
    let source = frame();
    let clipped = [
        zone(-4.0, -3.0, 4.0, 5.0, ClipPolicy::Clip),
        zone(28.0, 20.0, 40.0, 30.0, ClipPolicy::Clip),
    ];

    let result = scan_zones(&recognizer, &source, &clipped, &OperationContext::new()).unwrap();
    assert_eq!(
        result.effective_zones(),
        [
            PixelRect::new(0, 0, 4, 5).unwrap(),
            PixelRect::new(28, 20, 32, 24).unwrap(),
        ]
    );
    assert_eq!(
        result.source_envelope(),
        PixelRect::new(0, 0, 32, 24).unwrap()
    );
    assert_eq!(backend.recognition_count(), 1);

    let outside = [zone(40.0, 30.0, 50.0, 40.0, ClipPolicy::Clip)];
    assert_eq!(
        scan_zones(&recognizer, &source, &outside, &OperationContext::new())
            .unwrap_err()
            .status(),
        Status::InvalidArgument
    );
    assert_eq!(backend.recognition_count(), 1);
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
        PixelRect::new(8, 6, 24, 18).unwrap()
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
        Some(PixelRect::new(8, 6, 24, 18).unwrap())
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
    let effective = PixelRect::new(25, 18, 32, 24).unwrap();
    assert_eq!(clipped_backend.last_region(), Some(effective));
    assert_eq!(clipped.effective_region(), effective);
    assert_eq!(
        clipped.regions()[0].geometry().points()[0],
        mado_pilot_core::Point::new(CoordinateSpace::CapturePixels, 26.0, 19.0).unwrap()
    );
}

#[test]
fn grouped_scan_accepts_exact_aggregate_bounds_and_refuses_the_next_candidate() {
    let maximum_text: Arc<[u8]> = Arc::from(vec![b'a'; mado_pilot_ocr::MAX_TEXT_BYTES]);
    let candidates = (0..mado_pilot_ocr::MAX_CANDIDATES)
        .map(|order| {
            ScriptedOcrCandidate::new(
                Arc::clone(&maximum_text),
                [(1.0, 1.0), (12.0, 1.0), (12.0, 8.0), (1.0, 8.0)],
                0.5,
                u32::try_from(order).unwrap(),
            )
        })
        .collect();
    let backend = Arc::new(ControlledOcr::new(PixelFormat::Bgra8).with_candidates(candidates));
    let recognizer = OcrRecognizer::new(backend);
    let source = frame();
    let zones = [zone(0.0, 0.0, 32.0, 24.0, ClipPolicy::Reject); 8];
    let result = scan_zones(&recognizer, &source, &zones, &OperationContext::new()).unwrap();

    assert_eq!(
        result.unique_candidates().len(),
        mado_pilot_ocr::MAX_CANDIDATES
    );
    for group in 0..8 {
        assert_eq!(
            result.group(group).unwrap().len(),
            mado_pilot_ocr::MAX_CANDIDATES
        );
    }
    assert!(std::ptr::eq(
        result.group(0).unwrap().get(0).unwrap(),
        result.group(7).unwrap().get(0).unwrap(),
    ));

    let maximum_raw_text: Arc<[u8]> = Arc::from(vec![b' '; mado_pilot_ocr::MAX_BACKEND_TEXT_BYTES]);
    let raw_boundary = (0..mado_pilot_ocr::MAX_CANDIDATES)
        .map(|order| {
            ScriptedOcrCandidate::new(
                Arc::clone(&maximum_raw_text),
                [(1.0, 1.0), (12.0, 1.0), (12.0, 8.0), (1.0, 8.0)],
                0.5,
                u32::try_from(order).unwrap(),
            )
        })
        .collect();
    let backend: Arc<dyn OcrBackend> =
        Arc::new(ControlledOcr::new(PixelFormat::Bgra8).with_candidates(raw_boundary));
    let empty = scan_zones(
        &OcrRecognizer::new(backend),
        &source,
        &zones[..1],
        &OperationContext::new(),
    )
    .unwrap();
    assert!(empty.is_empty());

    let too_many = (0..=mado_pilot_ocr::MAX_CANDIDATES)
        .map(|order| candidate(b"x", u32::try_from(order).unwrap()))
        .collect();
    let backend: Arc<dyn OcrBackend> =
        Arc::new(ControlledOcr::new(PixelFormat::Bgra8).with_candidates(too_many));
    assert_eq!(
        scan_zones(
            &OcrRecognizer::new(backend),
            &source,
            &zones[..1],
            &OperationContext::new(),
        )
        .unwrap_err()
        .status(),
        Status::VisionFailed
    );
}

#[test]
fn grouped_scan_rejects_oversized_source_before_mapping_or_backend_work() {
    const MAPPING_LIMIT_BYTES: usize = 256 * 1024 * 1024;

    let descriptor = FrameDescriptor::new(
        PixelExtent::new(1, 1),
        PixelFormat::Bgra8,
        MAPPING_LIMIT_BYTES + 1,
    )
    .unwrap();
    let storage: Arc<dyn FrameStorage> = Arc::new(PreflightOnlyStorage { descriptor });
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
    let backend = Arc::new(ControlledOcr::new(PixelFormat::Bgra8));
    let port: Arc<dyn OcrBackend> = backend.clone();
    let recognizer = OcrRecognizer::new(port);
    let zones = [zone(0.0, 0.0, 1.0, 1.0, ClipPolicy::Reject)];

    assert_eq!(
        scan_zones(&recognizer, &source, &zones, &OperationContext::new())
            .unwrap_err()
            .status(),
        Status::LimitExceeded
    );
    assert_eq!(backend.recognition_count(), 0);
}

#[test]
fn malformed_grouped_candidates_fail_atomically() {
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
    let source = frame();
    let zones = [zone(0.0, 0.0, 32.0, 24.0, ClipPolicy::Reject)];
    for candidates in cases {
        let backend: Arc<dyn OcrBackend> =
            Arc::new(ControlledOcr::new(PixelFormat::Bgra8).with_candidates(candidates));
        assert_eq!(
            scan_zones(
                &OcrRecognizer::new(backend),
                &source,
                &zones,
                &OperationContext::new(),
            )
            .unwrap_err()
            .status(),
            Status::VisionFailed
        );
    }
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
fn grouped_interruption_stages_never_commit_partial_output() {
    let source = frame();
    let zones = [zone(0.0, 0.0, 32.0, 24.0, ClipPolicy::Reject)];

    let token = CancellationToken::new();
    token.cancel();
    let backend = Arc::new(ControlledOcr::new(PixelFormat::Bgra8));
    assert_eq!(
        scan_zones(
            &OcrRecognizer::new(backend.clone()),
            &source,
            &zones,
            &OperationContext::new().with_cancellation(token),
        )
        .unwrap_err()
        .status(),
        Status::Cancelled
    );
    assert_eq!(backend.recognition_count(), 0);

    let clock = Arc::new(ManualClock::new());
    let context = OperationContext::new()
        .with_clock(clock.clone())
        .with_timeout(Duration::from_millis(5))
        .unwrap();
    clock.advance(Duration::from_millis(5));
    assert_eq!(
        scan_zones(
            &OcrRecognizer::new(backend.clone()),
            &source,
            &zones,
            &context,
        )
        .unwrap_err()
        .status(),
        Status::DeadlineExceeded
    );
    assert_eq!(backend.recognition_count(), 0);

    let token = CancellationToken::new();
    let backend: Arc<dyn OcrBackend> = Arc::new(
        ControlledOcr::new(PixelFormat::Bgra8)
            .with_candidates(vec![candidate(b"after-mapping", 0)])
            .cancelling(token.clone()),
    );
    assert_eq!(
        scan_zones(
            &OcrRecognizer::new(backend),
            &source,
            &zones,
            &OperationContext::new().with_cancellation(token),
        )
        .unwrap_err()
        .status(),
        Status::Cancelled
    );

    let token = CancellationToken::new();
    let backend: Arc<dyn OcrBackend> = Arc::new(
        ControlledOcr::new(PixelFormat::Bgra8)
            .with_candidates(vec![candidate(b"first", 0), candidate(b"second", 1)])
            .cancelling_after_candidates(token.clone(), 1),
    );
    assert_eq!(
        scan_zones(
            &OcrRecognizer::new(backend),
            &source,
            &zones,
            &OperationContext::new().with_cancellation(token),
        )
        .unwrap_err()
        .status(),
        Status::Cancelled
    );

    let token = CancellationToken::new();
    let backend: Arc<dyn OcrBackend> = Arc::new(
        ControlledOcr::new(PixelFormat::Bgra8)
            .with_candidates(vec![candidate(b"returned", 0)])
            .cancelling_after_output(token.clone()),
    );
    assert_eq!(
        scan_zones(
            &OcrRecognizer::new(backend),
            &source,
            &zones,
            &OperationContext::new().with_cancellation(token),
        )
        .unwrap_err()
        .status(),
        Status::Cancelled
    );

    let clock = Arc::new(ManualClock::new());
    let backend: Arc<dyn OcrBackend> = Arc::new(
        ControlledOcr::new(PixelFormat::Bgra8)
            .with_candidates(vec![candidate(b"late", 0)])
            .with_latency(clock.clone(), Duration::from_millis(10)),
    );
    let context = OperationContext::new()
        .with_clock(clock)
        .with_timeout(Duration::from_millis(5))
        .unwrap();
    assert_eq!(
        scan_zones(&OcrRecognizer::new(backend), &source, &zones, &context)
            .unwrap_err()
            .status(),
        Status::DeadlineExceeded
    );
}

#[test]
fn grouped_final_commit_rechecks_deadline_without_a_production_test_hook() {
    const FINAL_COMMIT_CLOCK_READ: usize = 11;

    let backend = Arc::new(
        ControlledOcr::new(PixelFormat::Bgra8).with_candidates(vec![candidate(b"complete", 0)]),
    );
    let recognizer = OcrRecognizer::new(backend.clone());
    let source = frame();
    let zones = [zone(0.0, 0.0, 32.0, 24.0, ClipPolicy::Reject)];
    let deadline = MonotonicInstant::ORIGIN
        .checked_add(Duration::from_secs(1))
        .unwrap();

    let baseline_clock = Arc::new(CountingDeadlineClock::new(None));
    let baseline_context = OperationContext::new()
        .with_clock(baseline_clock.clone())
        .with_deadline(deadline);
    scan_zones(&recognizer, &source, &zones, &baseline_context).unwrap();
    // Scan admission; mapping admission/commit; mapping checkpoint; backend and
    // sink checks; backend checkpoint; finish and group checks; finish
    // checkpoint; then the final commit read.
    assert_eq!(baseline_clock.calls(), FINAL_COMMIT_CLOCK_READ);

    let expiring_clock = Arc::new(CountingDeadlineClock::new(Some(FINAL_COMMIT_CLOCK_READ)));
    let expiring_context = OperationContext::new()
        .with_clock(expiring_clock.clone())
        .with_deadline(deadline);
    assert_eq!(
        scan_zones(&recognizer, &source, &zones, &expiring_context)
            .unwrap_err()
            .status(),
        Status::DeadlineExceeded
    );
    assert_eq!(expiring_clock.calls(), FINAL_COMMIT_CLOCK_READ);
    assert_eq!(backend.recognition_count(), 2);
}

#[test]
fn grouped_late_completion_and_close_race_preserve_terminal_outcomes() {
    let gate = Arc::new(CompletionGate::new());
    let backend = Arc::new(
        ControlledOcr::new(PixelFormat::Bgra8)
            .with_candidates(vec![candidate(b"late", 0)])
            .with_completion_gate(gate.clone()),
    );
    let recognizer = OcrRecognizer::new(backend.clone());
    let token = CancellationToken::new();
    let worker_recognizer = recognizer.clone();
    let worker_token = token.clone();
    let worker = std::thread::spawn(move || {
        let source = frame();
        let zones = [zone(0.0, 0.0, 32.0, 24.0, ClipPolicy::Reject)];
        scan_zones(
            &worker_recognizer,
            &source,
            &zones,
            &OperationContext::new().with_cancellation(worker_token),
        )
    });
    assert!(gate.wait_until_entered(Duration::from_secs(1)));

    recognizer.close(&OperationContext::new()).unwrap();
    recognizer.close(&OperationContext::new()).unwrap();
    token.cancel();
    gate.release();

    assert_eq!(
        worker.join().unwrap().unwrap_err().status(),
        Status::Cancelled
    );
    assert_eq!(backend.recognition_count(), 1);
    assert_eq!(backend.close_count(), 2);
}

#[test]
fn overlap_layouts_keep_exact_coordinates_and_shared_candidate_ownership() {
    let source = frame();
    let cases = [
        (
            vec![
                zone(0.0, 0.0, 16.0, 16.0, ClipPolicy::Reject),
                zone(0.0, 0.0, 16.0, 16.0, ClipPolicy::Reject),
            ],
            candidate_with(
                b"duplicate",
                [(2.0, 2.0), (6.0, 2.0), (6.0, 6.0), (2.0, 6.0)],
                0.9,
                0,
            ),
            vec![true, true],
        ),
        (
            vec![
                zone(0.0, 0.0, 16.0, 16.0, ClipPolicy::Reject),
                zone(1.0, 0.0, 17.0, 16.0, ClipPolicy::Reject),
            ],
            candidate_with(
                b"near",
                [(2.0, 2.0), (6.0, 2.0), (6.0, 6.0), (2.0, 6.0)],
                0.9,
                0,
            ),
            vec![true, true],
        ),
        (
            vec![
                zone(0.0, 0.0, 8.0, 8.0, ClipPolicy::Reject),
                zone(8.0, 0.0, 16.0, 8.0, ClipPolicy::Reject),
            ],
            candidate_with(
                b"adjacent",
                [(7.0, 1.0), (9.0, 1.0), (9.0, 3.0), (7.0, 3.0)],
                0.9,
                0,
            ),
            vec![false, true],
        ),
        (
            vec![
                zone(0.0, 0.0, 9.0, 8.0, ClipPolicy::Reject),
                zone(8.0, 0.0, 16.0, 8.0, ClipPolicy::Reject),
            ],
            candidate_with(
                b"overlap",
                [(7.5, 1.0), (9.5, 1.0), (9.5, 3.0), (7.5, 3.0)],
                0.9,
                0,
            ),
            vec![true, true],
        ),
    ];

    for (zones, candidate, expected_memberships) in cases {
        let expected_zones = zones
            .iter()
            .map(|zone| {
                source
                    .transform()
                    .resolve_capture_pixels(zone.rect(), zone.policy())
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let backend: Arc<dyn OcrBackend> =
            Arc::new(ControlledOcr::new(PixelFormat::Bgra8).with_candidates(vec![candidate]));
        let result = scan_zones(
            &OcrRecognizer::new(backend),
            &source,
            &zones,
            &OperationContext::new(),
        )
        .unwrap();
        assert_eq!(result.effective_zones(), expected_zones);
        assert_eq!(result.unique_candidates().len(), 1);
        for (group, expected) in expected_memberships.into_iter().enumerate() {
            assert_eq!(!result.group(group).unwrap().is_empty(), expected);
        }
        let references = (0..zones.len())
            .filter_map(|group| result.group(group).unwrap().get(0))
            .collect::<Vec<_>>();
        assert!(
            references
                .windows(2)
                .all(|pair| std::ptr::eq(pair[0], pair[1]))
        );
    }

    let zones = vec![zone(0.0, 0.0, 16.0, 16.0, ClipPolicy::Reject); 8];
    let backend: Arc<dyn OcrBackend> = Arc::new(
        ControlledOcr::new(PixelFormat::Bgra8).with_candidates(vec![candidate(b"complete", 0)]),
    );
    let result = scan_zones(
        &OcrRecognizer::new(backend),
        &source,
        &zones,
        &OperationContext::new(),
    )
    .unwrap();
    assert_eq!(result.effective_zones().len(), 8);
    assert_eq!(result.unique_candidates().len(), 1);
    assert!((0..8).all(|group| result.group(group).unwrap().len() == 1));
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
fn out_of_order_calls_keep_results_and_latest_observations_isolated() {
    let old_gate = Arc::new(CompletionGate::new());
    let new_gate = Arc::new(CompletionGate::new());
    let backend = Arc::new(ControlledOcr::new(PixelFormat::Bgra8).with_calls(vec![
            ScriptedOcrCall::new(vec![candidate(b"old", 0), candidate(b"old-second", 1)])
                .with_completion_gate(Arc::clone(&old_gate)),
            ScriptedOcrCall::new(vec![candidate(b"new", 0)])
                .with_completion_gate(Arc::clone(&new_gate)),
        ]));
    let port: Arc<dyn OcrBackend> = backend.clone();
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
    assert_eq!(older.regions().len(), 2);
    assert_eq!(newer.stamp(), new_stamp);
    assert_eq!(newer.regions()[0].text(), "new");
    assert_eq!(backend.last_selected_candidates(), 1);
    assert_eq!(backend.last_ignored_candidates(), 0);
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
fn retained_grouped_result_uses_one_mapping_and_releases_backend_frame_and_script_storage() {
    let raw_text: Arc<[u8]> = Arc::from(&b"private-grouped-text"[..]);
    let backend = Arc::new(ControlledOcr::new(PixelFormat::Bgra8).with_candidates(vec![
        ScriptedOcrCandidate::new(
            Arc::clone(&raw_text),
            [(1.0, 1.0), (12.0, 1.0), (12.0, 8.0), (1.0, 8.0)],
            0.5,
            0,
        ),
    ]));
    let port: Arc<dyn OcrBackend> = backend.clone();
    let recognizer = OcrRecognizer::new(port);
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
    let zones = [zone(0.0, 0.0, 16.0, 12.0, ClipPolicy::Reject)];

    let result = scan_zones(&recognizer, &source, &zones, &OperationContext::new()).unwrap();
    assert_eq!(producer.conversions(), 1);
    assert_eq!(backend.recognition_count(), 1);
    assert_eq!(producer.producer_slots_free(), producer.pool());
    assert_eq!(producer.detached_slots_free(), 0);

    drop(source);
    drop(recognizer);
    drop(backend);
    assert_eq!(producer.detached_slots_free(), producer.detached_budget());
    assert_eq!(Arc::strong_count(&raw_text), 1);
    assert_eq!(
        result.group(0).unwrap().get(0).unwrap().text(),
        "private-grouped-text"
    );
    let debug = format!("{result:?}");
    assert!(!debug.contains("private-grouped-text"));
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
