//! Consumer-facing OCR watch authority, scheduling and retention regressions.

use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use mado_pilot_capture::{
    CaptureSession, CpuPixels, FrameStorage, Publication, StoragePublication, StreamState,
};
use mado_pilot_runtime::*;
use mado_pilot_testkit::{
    Candidate, CompletionGate, ControlledCapture, ControlledMatcher, ControlledOcr,
    ControlledProducer, ManualClock, ScriptedOcrCall, ScriptedOcrCandidate, match_fixtures,
};

const LIMIT: Duration = Duration::from_secs(3);
const EXTENT: PixelExtent = PixelExtent::new(32, 24);

fn bounded() -> OperationContext {
    OperationContext::new().with_timeout(LIMIT).unwrap()
}
fn region() -> Rect {
    Rect::new(CoordinateSpace::CapturePixels, 0.0, 0.0, 32.0, 24.0).unwrap()
}
fn candidate(text: &[u8], confidence: f64, order: u32) -> ScriptedOcrCandidate {
    ScriptedOcrCandidate::new(
        text,
        [(1.0, 1.0), (8.0, 1.0), (8.0, 5.0), (1.0, 5.0)],
        confidence,
        order,
    )
}
fn call(text: &[u8]) -> ScriptedOcrCall {
    ScriptedOcrCall::new(vec![candidate(text, 0.8, 0)])
}
fn cpu_descriptor() -> OcrBackendDescriptor {
    OcrBackendDescriptor::new(
        OcrBackendIdentity::new(
            OcrBackendId::new("onnxruntime-cpu").unwrap(),
            OcrBackendVersion::new(concat!(env!("CARGO_PKG_VERSION"), "+ort-1.29.0-api17"))
                .unwrap(),
        ),
        OcrModelIdentity::accepted_bounded_detector(),
        PixelFormat::Bgra8,
    )
}
fn cpu_provider() -> OcrProviderDescriptor {
    OcrProviderDescriptor::new(
        OcrExecutionProviderPolicy::Cpu,
        OcrExecutionProvider::Cpu,
        None,
        ProviderProfileId::new("onnxruntime-1.29.0-api17-cpu").unwrap(),
    )
}
fn backend() -> ControlledOcr {
    ControlledOcr::new(PixelFormat::Bgra8)
        .with_descriptor(cpu_descriptor())
        .with_provider_descriptor(cpu_provider())
}
fn request(text: &str, operation: OperationContext) -> OcrTextWatchRequest {
    configured(
        text,
        0.8,
        OcrTextStability::immediate(),
        ChangeDetectionPolicy::AnalysisAlways,
        operation,
    )
}
fn configured(
    text: &str,
    confidence: f64,
    stability: OcrTextStability,
    policy: ChangeDetectionPolicy,
    operation: OperationContext,
) -> OcrTextWatchRequest {
    OcrTextWatchRequest::new(
        region(),
        ClipPolicy::Reject,
        text,
        confidence,
        CoordinateSpace::CapturePixels,
        OcrTextAnalysisRate::from_minimum_interval(Duration::from_nanos(1)).unwrap(),
        stability,
        policy,
        operation,
    )
    .unwrap()
}

struct Harness {
    engine: Engine,
    capture: Arc<ControlledCapture>,
    matcher: Arc<ControlledMatcher>,
}
impl Harness {
    fn new(ocr: Arc<dyn OcrBackend>) -> Self {
        Self::with_options(
            Some(ocr),
            ControlledMatcher::new(PixelFormat::Rgba8),
            EngineOptions::new(),
        )
    }
    fn with_options(
        ocr: Option<Arc<dyn OcrBackend>>,
        matcher: ControlledMatcher,
        options: EngineOptions,
    ) -> Self {
        let issuer = Arc::new(IdentityIssuer::new());
        let capture =
            Arc::new(ControlledCapture::new(issuer.clone(), EXTENT, PixelFormat::Bgra8).unwrap());
        let matcher = Arc::new(matcher);
        let engine = Engine::new_with_options(
            EngineWiring {
                engine: issuer.engine(),
                capture: capture.clone(),
                matcher: Matcher::new(matcher.clone()),
                loader: PackageLoader::new(),
                ocr: ocr.map(OcrRecognizer::new),
                input: None,
                permission: None,
            },
            options,
        )
        .unwrap();
        Self {
            engine,
            capture,
            matcher,
        }
    }
    fn open(&self) -> Session {
        self.engine
            .open(self.capture.target(), &OpenRequest::new(), &bounded())
            .unwrap()
    }
    fn publish(&self, fill: u8) {
        self.capture.publish(fill, Continuity::Continuous).unwrap();
    }
    fn template(&self, session: &Session, operation: OperationContext) -> TemplateQuery {
        let template = self
            .engine
            .prepare_template(&match_fixtures::planted_template("mixed"), &bounded())
            .unwrap();
        let options = MatchOptions::from_defaults(template.defaults());
        session
            .start_template_watch(TemplateWatchRequest::new(template, options, operation))
            .unwrap()
    }
}
fn until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + LIMIT;
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "controlled observation did not arrive"
        );
        std::thread::yield_now();
    }
}
fn progress(
    query: &OcrTextQuery,
    condition: impl Fn(OcrTextQueryProgress) -> bool,
) -> OcrTextQueryProgress {
    until(|| condition(query.progress()));
    query.progress()
}
fn completed(query: &OcrTextQuery, count: u64) -> OcrTextQueryProgress {
    progress(query, |p| {
        p.work().get(OcrTextWorkDisposition::Completed) >= count
    })
}
fn matched(outcome: &OcrTextTerminalOutcome) -> &OcrTextWatchResult {
    match outcome {
        OcrTextTerminalOutcome::Matched(result) => result,
        other => panic!("expected match: {other:?}"),
    }
}

#[test]
fn request_admission_normalizes_without_raw_edge_whitespace_limit() {
    let ocr =
        Arc::new(backend().with_candidates(vec![candidate("before é after".as_bytes(), 0.8, 0)]));
    let harness = Harness::new(ocr);
    let session = harness.open();
    let raw = format!(
        "{}e\u{301}{}",
        "\u{2003}".repeat(20_000),
        " ".repeat(20_000)
    );
    let req = request(&raw, OperationContext::new());
    assert!(!format!("{req:?}").contains("e\u{301}"));
    assert!(!format!("{req:?}").contains('é'));
    let query = session.start_ocr_text_watch(req).unwrap();
    harness.publish(1);
    let outcome = query.wait(&bounded()).unwrap();
    assert_eq!(matched(&outcome).literal(), "é");
    assert_eq!(
        matched(&outcome).matching_regions().next().unwrap().text(),
        "before é after"
    );
    assert!(!format!("{outcome:?}").contains('é'));
}

#[test]
fn invalid_options_and_interrupted_construction_publish_no_query() {
    let construct = |text: &str, confidence: f64, operation| {
        OcrTextWatchRequest::new(
            region(),
            ClipPolicy::Reject,
            text,
            confidence,
            CoordinateSpace::CapturePixels,
            OcrTextAnalysisRate::from_minimum_interval(Duration::from_secs(1)).unwrap(),
            OcrTextStability::immediate(),
            ChangeDetectionPolicy::AnalysisAlways,
            operation,
        )
    };
    assert_eq!(
        OcrTextAnalysisRate::from_minimum_interval(Duration::ZERO)
            .unwrap_err()
            .status(),
        Status::InvalidArgument
    );
    assert_eq!(
        OcrTextStability::consecutive(0).unwrap_err().status(),
        Status::InvalidArgument
    );
    for confidence in [f64::NAN, f64::INFINITY, -0.01, 1.01] {
        assert_eq!(
            construct("valid", confidence, OperationContext::new())
                .unwrap_err()
                .status(),
            Status::InvalidArgument
        );
    }
    assert_eq!(
        construct("\u{2003} \n", 0.8, OperationContext::new())
            .unwrap_err()
            .status(),
        Status::InvalidArgument
    );
    assert_eq!(
        construct(&"x".repeat(4097), 0.8, OperationContext::new())
            .unwrap_err()
            .status(),
        Status::LimitExceeded
    );
    assert!(construct(&"x".repeat(4096), 0.0, OperationContext::new()).is_ok());
    let token = CancellationToken::new();
    token.cancel();
    assert_eq!(
        construct(
            "secret",
            0.8,
            OperationContext::new()
                .with_cancellation(token)
                .with_timeout(Duration::ZERO)
                .unwrap()
        )
        .unwrap_err()
        .status(),
        Status::Cancelled
    );
}

#[test]
fn complete_initialized_identity_and_known_coordinates_are_required() {
    let mut selections: Vec<Option<Arc<dyn OcrBackend>>> = vec![None];
    selections.push(Some(Arc::new(
        ControlledOcr::new(PixelFormat::Bgra8).with_descriptor(cpu_descriptor()),
    )));
    let native = OcrBackendDescriptor::new(
        cpu_descriptor().backend_identity().clone(),
        OcrModelIdentity::accepted_g004(),
        PixelFormat::Bgra8,
    );
    selections.push(Some(Arc::new(backend().with_descriptor(native))));
    selections.push(Some(Arc::new(backend().with_provider_descriptor(
        OcrProviderDescriptor::new(
            OcrExecutionProviderPolicy::RequireCuda,
            OcrExecutionProvider::Cuda,
            None,
            ProviderProfileId::new("onnxruntime-1.29.0-api17-cuda13-cudnn9").unwrap(),
        ),
    ))));
    selections.push(Some(Arc::new(backend().with_provider_descriptor(
        OcrProviderDescriptor::new(
            OcrExecutionProviderPolicy::Cpu,
            OcrExecutionProvider::Cpu,
            None,
            ProviderProfileId::new("unknown-runtime").unwrap(),
        ),
    ))));
    let wrong_backend = OcrBackendDescriptor::new(
        OcrBackendIdentity::new(
            OcrBackendId::new("unspecified").unwrap(),
            OcrBackendVersion::new("1").unwrap(),
        ),
        OcrModelIdentity::accepted_bounded_detector(),
        PixelFormat::Bgra8,
    );
    selections.push(Some(Arc::new(backend().with_descriptor(wrong_backend))));
    for (index, ocr) in selections.into_iter().enumerate() {
        let harness = Harness::with_options(
            ocr,
            ControlledMatcher::new(PixelFormat::Rgba8),
            EngineOptions::new(),
        );
        let session = harness.open();
        assert_eq!(
            session
                .start_ocr_text_watch(request("yes", OperationContext::new()))
                .unwrap_err()
                .status(),
            if index == 0 {
                Status::VisionFailed
            } else {
                Status::Unsupported
            }
        );
        assert_eq!(harness.engine.ocr_text_observation().query_count, 0);
        let template = harness.template(&session, OperationContext::new());
        let _ = template.cancel();
    }
    let harness = Harness::new(Arc::new(backend()));
    let session = harness.open();
    let unsupported = OcrTextWatchRequest::new(
        Rect::new(CoordinateSpace::TargetLogical, 0.0, 0.0, 1.0, 1.0).unwrap(),
        ClipPolicy::Reject,
        "yes",
        0.0,
        CoordinateSpace::CapturePixels,
        OcrTextAnalysisRate::from_minimum_interval(Duration::from_secs(1)).unwrap(),
        OcrTextStability::immediate(),
        ChangeDetectionPolicy::AnalysisAlways,
        OperationContext::new(),
    )
    .unwrap();
    assert_eq!(
        session
            .start_ocr_text_watch(unsupported)
            .unwrap_err()
            .status(),
        Status::Unsupported
    );
}

#[test]
fn regions_are_not_joined_or_case_folded_and_confidence_is_inclusive() {
    let ocr = Arc::new(backend().with_calls(vec![
        ScriptedOcrCall::new(vec![candidate(b"rea", 1.0, 0), candidate(b"dy", 1.0, 1)]),
        call(b"READY"),
        ScriptedOcrCall::new(vec![candidate(b"ready", 0.79999, 0)]),
        ScriptedOcrCall::new(vec![
            candidate(b"pre ready", 0.8, 2),
            candidate(b"ready post", 1.0, 0),
        ]),
    ]));
    let harness = Harness::new(ocr.clone());
    let session = harness.open();
    let query = session
        .start_ocr_text_watch(request("ready", OperationContext::new()))
        .unwrap();
    for fill in 1..=3 {
        harness.publish(fill);
        assert_eq!(
            completed(&query, u64::from(fill)).confirmed_observations(),
            0
        );
    }
    harness.publish(4);
    let outcome = query.wait(&bounded()).unwrap();
    let result = matched(&outcome);
    assert_eq!(result.satisfying_region_indexes(), [0, 1]);
    assert_eq!(
        result
            .matching_regions()
            .map(RecognizedRegion::text)
            .collect::<Vec<_>>(),
        ["ready post", "pre ready"]
    );
    assert_eq!(result.result().regions().len(), 2);
    assert_eq!(ocr.recognition_count(), 4);
}

#[test]
fn malformed_later_candidate_cannot_publish_an_early_match() {
    for bad in [
        candidate(&[0xff], 0.8, 1),
        candidate(b"secret", f64::NAN, 1),
        ScriptedOcrCandidate::new(b"secret".as_slice(), [(100.0, 0.0); 4], 0.8, 1),
        candidate(b"duplicate", 0.8, 0),
    ] {
        let harness = Harness::new(Arc::new(
            backend().with_candidates(vec![candidate(b"ready", 0.8, 0), bad]),
        ));
        let session = harness.open();
        let query = session
            .start_ocr_text_watch(request("ready", OperationContext::new()))
            .unwrap();
        harness.publish(1);
        let outcome = query.wait(&bounded()).unwrap();
        assert_eq!(outcome.status(), Some(Status::VisionFailed));
        assert_eq!(
            query
                .progress()
                .work()
                .get(OcrTextWorkDisposition::Completed),
            0
        );
        assert!(!format!("{outcome:?}").contains("secret"));
    }
}

#[test]
fn negatives_reset_and_only_distinct_positive_analyses_confirm() {
    let ocr = Arc::new(backend().with_calls(vec![
        call(b"ready"),
        call(b"missing"),
        call(b"ready"),
        call(b"ready"),
    ]));
    let harness = Harness::new(ocr.clone());
    let session = harness.open();
    let clock = Arc::new(ManualClock::new());
    let query = session
        .start_ocr_text_watch(configured(
            "ready",
            0.8,
            OcrTextStability::consecutive(2).unwrap(),
            ChangeDetectionPolicy::ExactRgba,
            OperationContext::new().with_clock(clock.clone()),
        ))
        .unwrap();
    harness.publish(1);
    let first = completed(&query, 1);
    assert_eq!(first.confirmed_observations(), 1);
    clock.advance(Duration::from_secs(3600));
    assert_eq!(query.progress().confirmed_observations(), 1);
    assert_eq!(ocr.recognition_count(), 1);
    harness.publish(1); // unchanged bytes must still receive confirmation OCR
    assert_eq!(completed(&query, 2).confirmed_observations(), 0);
    clock.advance(Duration::from_secs(1));
    harness.publish(2);
    let third = completed(&query, 3);
    clock.advance(Duration::from_secs(1));
    harness.publish(2);
    let outcome = query.wait(&bounded()).unwrap();
    assert_eq!(matched(&outcome).confirmed_observations(), 2);
    assert_eq!(
        matched(&outcome).first_confirmed_frame(),
        third.last_accepted_frame().unwrap()
    );
}

#[test]
fn rate_deferral_keeps_latest_source_without_counting_confirmation() {
    let ocr = Arc::new(backend().with_calls(vec![call(b"missing"), call(b"ready")]));
    let harness = Harness::new(ocr.clone());
    let session = harness.open();
    let clock = Arc::new(ManualClock::new());
    let query = session
        .start_ocr_text_watch(
            OcrTextWatchRequest::new(
                region(),
                ClipPolicy::Reject,
                "ready",
                0.8,
                CoordinateSpace::CapturePixels,
                OcrTextAnalysisRate::from_minimum_interval(Duration::from_secs(60)).unwrap(),
                OcrTextStability::immediate(),
                ChangeDetectionPolicy::AnalysisAlways,
                OperationContext::new().with_clock(clock.clone()),
            )
            .unwrap(),
        )
        .unwrap();
    harness.publish(1);
    completed(&query, 1);
    harness.publish(2);
    progress(&query, |p| {
        p.work().get(OcrTextWorkDisposition::DeferredRate) == 1
    });
    clock.advance(Duration::from_secs(31));
    harness.publish(3);
    let latest = progress(&query, |p| {
        p.work().get(OcrTextWorkDisposition::Superseded) >= 1
    })
    .last_frame()
    .unwrap();
    assert_eq!(ocr.recognition_count(), 1);
    assert_eq!(query.progress().confirmed_observations(), 0);
    clock.advance(Duration::from_secs(29));
    let outcome = query.wait(&bounded()).unwrap();
    assert_eq!(matched(&outcome).frame().stamp(), latest);
}

#[test]
fn held_backend_keeps_physical_lease_while_waits_and_terminal_authority_remain_independent() {
    let gate = Arc::new(CompletionGate::new());
    let harness = Harness::new(Arc::new(
        backend().with_calls(vec![call(b"ready").with_completion_gate(gate.clone())]),
    ));
    let session = harness.open();
    let query = session
        .start_ocr_text_watch(request("ready", OperationContext::new()))
        .unwrap();
    let _release = gate.release_guard();
    harness.publish(1);
    assert!(gate.wait_until_entered(LIMIT));
    assert_eq!(
        query
            .wait(
                &OperationContext::new()
                    .with_timeout(Duration::ZERO)
                    .unwrap()
            )
            .unwrap_err()
            .status(),
        Status::DeadlineExceeded
    );
    let token = CancellationToken::new();
    token.cancel();
    assert_eq!(
        query
            .wait(&OperationContext::new().with_cancellation(token))
            .unwrap_err()
            .status(),
        Status::Cancelled
    );
    assert!(matches!(query.poll(), OcrTextQueryOutcome::Pending(_)));
    let outcome = query.cancel();
    assert!(Arc::ptr_eq(&outcome, &query.cancel()));
    assert_eq!(query.progress().in_flight_count(), 0);
    assert_eq!(query.progress().physical_in_flight_count(), 1);
    assert_eq!(
        harness.engine.ocr_text_observation().physical_ocr_in_flight,
        1
    );
    gate.release();
    until(|| query.progress().physical_in_flight_count() == 0);
    assert!(Arc::ptr_eq(&outcome, &query.wait(&bounded()).unwrap()));
    assert!(!outcome.is_match());
}

#[test]
fn newer_pending_source_does_not_revoke_compatible_admitted_success() {
    let gate = Arc::new(CompletionGate::new());
    let harness = Harness::new(Arc::new(
        backend().with_calls(vec![call(b"ready").with_completion_gate(gate.clone())]),
    ));
    let session = harness.open();
    let query = session
        .start_ocr_text_watch(request("ready", OperationContext::new()))
        .unwrap();
    let _release = gate.release_guard();
    harness.publish(1);
    assert!(gate.wait_until_entered(LIMIT));
    let admitted = query.progress().last_frame().unwrap();
    harness.publish(2);
    progress(&query, |p| {
        p.last_frame().is_some_and(|stamp| stamp != admitted)
    });
    gate.release();
    assert_eq!(
        matched(&query.wait(&bounded()).unwrap()).frame().stamp(),
        admitted
    );
}

#[test]
fn pending_residence_expires_on_the_held_query_despite_continuous_replacement() {
    let gate = Arc::new(CompletionGate::new());
    let harness = Harness::new(Arc::new(backend().with_completion_gate(gate.clone())));
    let session = harness.open();
    let clock = Arc::new(ManualClock::new());
    let query = session
        .start_ocr_text_watch(request(
            "ready",
            OperationContext::new().with_clock(clock.clone()),
        ))
        .unwrap();
    let _release = gate.release_guard();
    harness.publish(1);
    assert!(gate.wait_until_entered(LIMIT));
    clock.advance(Duration::from_secs(1));
    harness.publish(2);
    progress(&query, |p| p.pending_count() == 1);
    for fill in 3..=5 {
        clock.advance(Duration::from_secs(10));
        harness.publish(fill);
        progress(&query, |p| {
            p.work().get(OcrTextWorkDisposition::Superseded) >= u64::from(fill - 2)
        });
    }
    clock.advance(Duration::from_secs(1));
    assert!(matches!(
        &*query.wait(&bounded()).unwrap(),
        OcrTextTerminalOutcome::Overloaded(OcrTextOverload::QueueExpired)
    ));
    assert_eq!(query.progress().physical_in_flight_count(), 1);
}

#[test]
fn deadline_and_cancellation_override_backend_completion_but_latched_close_stays_first() {
    let gate = Arc::new(CompletionGate::new());
    let harness = Harness::new(Arc::new(backend().with_completion_gate(gate.clone())));
    let session = harness.open();
    let clock = Arc::new(ManualClock::new());
    let token = CancellationToken::new();
    let query = session
        .start_ocr_text_watch(request(
            "ready",
            OperationContext::new()
                .with_clock(clock.clone())
                .with_cancellation(token.clone())
                .with_timeout(Duration::from_secs(1))
                .unwrap(),
        ))
        .unwrap();
    let _release = gate.release_guard();
    harness.publish(1);
    assert!(gate.wait_until_entered(LIMIT));
    token.cancel();
    clock.advance(Duration::from_secs(2));
    assert!(matches!(
        &*query.wait(&bounded()).unwrap(),
        OcrTextTerminalOutcome::Cancelled
    ));

    let other = harness.open();
    let token = CancellationToken::new();
    let closed = other
        .start_ocr_text_watch(request(
            "ready",
            OperationContext::new().with_cancellation(token.clone()),
        ))
        .unwrap();
    other.close(&bounded()).unwrap();
    token.cancel();
    assert!(matches!(
        &*closed.cancel(),
        OcrTextTerminalOutcome::SessionClosed
    ));
}

#[test]
fn epoch_and_geometry_transitions_revoke_held_old_work_and_reset_confirmation() {
    let gate = Arc::new(CompletionGate::new());
    let ocr = Arc::new(backend().with_calls(vec![
        call(b"ready"),
        call(b"ready").with_completion_gate(gate.clone()),
        call(b"ready"),
        call(b"ready"),
    ]));
    let harness = Harness::new(ocr);
    let session = harness.open();
    let query = session
        .start_ocr_text_watch(configured(
            "ready",
            0.8,
            OcrTextStability::consecutive(2).unwrap(),
            ChangeDetectionPolicy::AnalysisAlways,
            OperationContext::new(),
        ))
        .unwrap();
    let _release = gate.release_guard();
    harness.publish(1);
    let before = completed(&query, 1).last_accepted_frame().unwrap();
    harness.publish(2);
    assert!(gate.wait_until_entered(LIMIT));
    let producer =
        ControlledProducer::new(PixelExtent::new(40, 30), PixelFormat::Bgra8, 1, 8).unwrap();
    harness.capture.publish_from(&producer, 3).unwrap();
    progress(&query, |p| {
        p.last_frame()
            .is_some_and(|stamp| stamp.geometry() != before.geometry())
            && p.confirmed_observations() == 0
    });
    gate.release();
    let successor = progress(&query, |p| {
        p.confirmed_observations() == 1
            && p.last_accepted_frame()
                .is_some_and(|stamp| stamp.epoch() != before.epoch())
    })
    .last_accepted_frame()
    .unwrap();
    harness.capture.publish_from(&producer, 4).unwrap();
    let outcome = query.wait(&bounded()).unwrap();
    assert_eq!(matched(&outcome).confirmed_observations(), 2);
    assert_eq!(matched(&outcome).first_confirmed_frame(), successor);
    assert_eq!(
        matched(&outcome).frame().stamp().geometry(),
        successor.geometry()
    );
}

#[test]
fn invalid_resize_and_target_loss_cannot_become_matches() {
    for target_loss in [false, true] {
        let gate = Arc::new(CompletionGate::new());
        let harness = Harness::new(Arc::new(
            backend().with_calls(vec![call(b"ready").with_completion_gate(gate.clone())]),
        ));
        let session = harness.open();
        let query = session
            .start_ocr_text_watch(request("ready", OperationContext::new()))
            .unwrap();
        let _release = gate.release_guard();
        harness.publish(1);
        assert!(gate.wait_until_entered(LIMIT));
        if target_loss {
            harness.capture.lose(harness.capture.target());
        } else {
            harness
                .capture
                .publish_reshaped(PixelExtent::new(8, 8), 2)
                .unwrap();
        }
        let outcome = query.wait(&bounded()).unwrap();
        assert_eq!(
            outcome.status(),
            Some(if target_loss {
                Status::TargetLost
            } else {
                Status::InvalidArgument
            })
        );
        gate.release();
        until(|| query.progress().physical_in_flight_count() == 0);
        assert!(Arc::ptr_eq(&outcome, &query.wait(&bounded()).unwrap()));
    }
}

#[test]
fn source_end_drains_final_admitted_work_without_inventing_success() {
    for positive in [false, true] {
        let gate = Arc::new(CompletionGate::new());
        let harness = Harness::new(Arc::new(backend().with_calls(vec![
            call(if positive { b"ready" } else { b"missing" }).with_completion_gate(gate.clone()),
        ])));
        let session = harness.open();
        let query = session
            .start_ocr_text_watch(request("ready", OperationContext::new()))
            .unwrap();
        let _release = gate.release_guard();
        harness.publish(1);
        assert!(gate.wait_until_entered(LIMIT));
        harness.capture.terminate(CaptureFault::SessionClosed);
        gate.release();
        let outcome = query.wait(&bounded()).unwrap();
        assert_eq!(outcome.is_match(), positive);
        if !positive {
            assert!(matches!(&*outcome, OcrTextTerminalOutcome::SessionClosed));
        }
        assert_eq!(
            session
                .start_ocr_text_watch(request("ready", OperationContext::new()))
                .unwrap_err()
                .status(),
            Status::Closed
        );
    }
}

#[test]
fn template_progress_and_other_ocr_authority_survive_a_held_ocr_backend() {
    let gate = Arc::new(CompletionGate::new());
    let harness = Harness::with_options(
        Some(Arc::new(backend().with_calls(vec![
            call(b"ready").with_completion_gate(gate.clone()),
            call(b"ready"),
        ]))),
        ControlledMatcher::new(PixelFormat::Rgba8)
            .with_candidates(vec![Candidate::new(1, 1, 0.99)]),
        EngineOptions::new(),
    );
    let first = harness.open();
    let second = harness.open();
    let held = first
        .start_ocr_text_watch(request("ready", OperationContext::new()))
        .unwrap();
    let _release = gate.release_guard();
    harness.publish(1);
    assert!(gate.wait_until_entered(LIMIT));
    let template = harness.template(&second, OperationContext::new());
    assert!(template.wait(&bounded()).unwrap().is_match());
    assert_eq!(harness.matcher.find_count(), 1);
    let next = second
        .start_ocr_text_watch(request("ready", OperationContext::new()))
        .unwrap();
    progress(&next, |p| p.last_frame().is_some());
    assert_eq!(next.progress().in_flight_count(), 0);
    let _ = held.cancel();
    assert_eq!(
        harness.engine.ocr_text_observation().physical_ocr_in_flight,
        1
    );
    gate.release();
    assert!(next.wait(&bounded()).unwrap().is_match());
}

#[test]
fn shared_query_capacity_refuses_only_the_new_query_and_releases_cancelled_slots() {
    let harness = Harness::new(Arc::new(backend()));
    let session = harness.open();
    let template = harness.template(&session, OperationContext::new());
    let queries: Vec<_> = (0..63)
        .map(|_| {
            session
                .start_ocr_text_watch(request("ready", OperationContext::new()))
                .unwrap()
        })
        .collect();
    assert_eq!(
        session
            .start_ocr_text_watch(request("ready", OperationContext::new()))
            .unwrap_err()
            .status(),
        Status::LimitExceeded
    );
    assert!(matches!(template.poll(), TemplateQueryOutcome::Pending(_)));
    let _ = queries[0].cancel();
    let replacement = session
        .start_ocr_text_watch(request("ready", OperationContext::new()))
        .unwrap();
    assert_ne!(queries[0].id(), replacement.id());
}

#[test]
fn retained_result_tracks_exact_pixels_without_pinning_capture_pool_and_ignores_handle_clones() {
    let harness = Harness::new(Arc::new(
        backend().with_candidates(vec![candidate(b"ready", 0.8, 0)]),
    ));
    let session = harness.open();
    let producer = ControlledProducer::new(EXTENT, PixelFormat::Bgra8, 1, 4).unwrap();
    let query = session
        .start_ocr_text_watch(request("ready", OperationContext::new()))
        .unwrap();
    harness.capture.publish_from(&producer, 0x37).unwrap();
    let outcome = query.wait(&bounded()).unwrap();
    let result = matched(&outcome);
    let source = result.frame().clone();
    let extent = result.retained_extent();
    let clone = outcome.clone();
    assert_eq!(harness.engine.ocr_text_observation().retained_results, 1);
    assert_eq!(
        harness
            .engine
            .ocr_text_observation()
            .retained_result_extent_bytes,
        extent.total_bytes()
    );
    for fill in 1..=8 {
        harness.capture.publish_from(&producer, fill).unwrap();
    }
    assert_eq!(producer.producer_slots_free(), producer.pool());
    session.close(&bounded()).unwrap();
    drop(query);
    drop(session);
    drop(harness);
    assert_eq!(matched(&clone).frame().stamp(), source.stamp());
    assert!(
        source
            .map(PixelFormat::Bgra8, &bounded())
            .unwrap()
            .bytes()
            .iter()
            .all(|byte| *byte == 0x37)
    );
    drop(outcome);
    drop(clone);
    drop(source);
    until(|| producer.detached_slots_free() == producer.detached_budget());
}

#[test]
fn held_conversion_blocks_native_template_entry_but_not_close_or_last_owner_release() {
    for separate_session in [false, true] {
        let gate = Arc::new(CompletionGate::new());
        let harness = Harness::with_options(
            Some(Arc::new(backend())),
            ControlledMatcher::new(PixelFormat::Rgba8),
            EngineOptions::new().with_diagnostics(DiagnosticOptions::debug(64).unwrap()),
        );
        let reader = harness.engine.take_diagnostic_reader().unwrap();
        let ocr_session = harness.open();
        let template_session = separate_session.then(|| harness.open());
        let producer = ControlledProducer::new(EXTENT, PixelFormat::Bgra8, 1, 8).unwrap();
        producer.set_conversion_gate(Some(gate.clone()));
        let query = ocr_session
            .start_ocr_text_watch(request("ready", OperationContext::new()))
            .unwrap();
        let _release = gate.release_guard();
        harness.capture.publish_from(&producer, 1).unwrap();
        assert!(gate.wait_until_entered(LIMIT));
        let template = harness.template(
            template_session.as_ref().unwrap_or(&ocr_session),
            OperationContext::new(),
        );
        until(
            || matches!(template.poll(), TemplateQueryOutcome::Pending(p) if p.pending_count() == 1),
        );
        assert_eq!(
            producer.conversion_attempts(),
            1,
            "template must not enter same-frame or shared-device conversion"
        );
        let (sent, received) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            ocr_session.close(&bounded()).unwrap();
            if let Some(session) = template_session {
                session.close(&bounded()).unwrap();
            }
            drop(template);
            drop(query);
            drop(ocr_session);
            drop(harness);
            sent.send(()).unwrap();
        });
        received
            .recv_timeout(LIMIT)
            .expect("logical final owner release cannot join held OCR mapping");
        assert!(producer.detached_slots_free() < producer.detached_budget());
        until(|| matches!(reader.drain(), DiagnosticDrain::EndOfStream));
        gate.release();
        assert!(gate.wait_until_completed(LIMIT));
        until(|| producer.detached_slots_free() == producer.detached_budget());
    }
}

#[derive(Debug)]
struct ExactSource {
    description: SessionDescription,
    state: Arc<StreamState>,
}

impl ExactSource {
    fn wire(ocr: Arc<dyn OcrBackend>) -> (Engine, Arc<Self>) {
        let issuer = IdentityIssuer::new();
        let stream = issuer.issue_stream().unwrap();
        let target = issuer
            .issue_target(ProviderId::new("ocr-watch-fixture"))
            .unwrap();
        let source = Arc::new(Self {
            description: SessionDescription::new(
                target,
                stream,
                EXTENT,
                PixelFormat::Bgra8,
                CoordinateSupport::with_target_placement(),
            ),
            state: Arc::new(StreamState::with_target_extent(stream)),
        });
        let engine = Engine::new(EngineWiring {
            engine: issuer.engine(),
            capture: source.clone(),
            matcher: Matcher::new(Arc::new(ControlledMatcher::new(PixelFormat::Rgba8))),
            loader: PackageLoader::new(),
            ocr: Some(OcrRecognizer::new(ocr)),
            input: None,
            permission: None,
        })
        .unwrap();
        (engine, source)
    }
    fn open(&self, engine: &Engine) -> Session {
        engine
            .open(self.description.target(), &OpenRequest::new(), &bounded())
            .unwrap()
    }
    fn pixels(
        &self,
        descriptor: FrameDescriptor,
        pixels: Vec<u8>,
        placement: Option<TargetPlacement>,
    ) {
        self.state
            .publish(Publication {
                descriptor,
                pixels: pixels.into_boxed_slice(),
                captured_at: MonotonicInstant::ORIGIN,
                placement,
                continuity: Continuity::Continuous,
            })
            .unwrap();
    }
}

impl CaptureProvider for ExactSource {
    fn provider(&self) -> ProviderId {
        ProviderId::new("ocr-watch-fixture")
    }
    fn discover(&self, _: &OperationContext) -> Result<Vec<TargetDescription>> {
        Ok(vec![TargetDescription::new(
            self.description.target(),
            "fixture",
            EXTENT,
            PixelFormat::Bgra8,
            self.description.coordinates(),
        )])
    }
    fn open(
        &self,
        target: TargetId,
        _: &OpenRequest,
        _: &OperationContext,
    ) -> Result<Arc<dyn CaptureSession>> {
        assert_eq!(target, self.description.target());
        Ok(Arc::new(Self {
            description: self.description.clone(),
            state: self.state.clone(),
        }))
    }
}
impl CaptureSession for ExactSource {
    fn description(&self) -> SessionDescription {
        self.description.clone()
    }
    fn frame(&self, request: &FrameRequest, operation: &OperationContext) -> Result<Frame> {
        self.state.frame(request, operation)
    }
    fn close(&self, operation: &OperationContext) -> Result<()> {
        self.state.drain(operation)
    }
    fn lifecycle(&self) -> Lifecycle {
        self.state.lifecycle()
    }
}

#[test]
fn exact_bgra_comparison_ignores_padding_but_detects_visible_change() {
    let ocr = Arc::new(backend().with_calls(vec![call(b"missing"), call(b"ready")]));
    let (engine, source) = ExactSource::wire(ocr.clone());
    let session = source.open(&engine);
    let query = session
        .start_ocr_text_watch(configured(
            "ready",
            0.8,
            OcrTextStability::immediate(),
            ChangeDetectionPolicy::ExactRgba,
            OperationContext::new(),
        ))
        .unwrap();
    let descriptor = FrameDescriptor::new(EXTENT, PixelFormat::Bgra8, 136).unwrap();
    let pixels = vec![0; descriptor.byte_len()];
    source.pixels(descriptor, pixels.clone(), None);
    completed(&query, 1);
    let mut padding_changed = pixels;
    for row in 0..24 {
        padding_changed[row * 136 + 128..row * 136 + 136].fill(0xFF);
    }
    source.pixels(descriptor, padding_changed.clone(), None);
    progress(&query, |p| {
        p.work().get(OcrTextWorkDisposition::SkippedChange) == 1
    });
    assert_eq!(ocr.recognition_count(), 1);
    padding_changed[4] = 1;
    source.pixels(descriptor, padding_changed, None);
    let outcome = query.wait(&bounded()).unwrap();
    assert!(outcome.is_match());
    assert_eq!(ocr.recognition_count(), 2);
}

#[test]
fn unresolved_initial_transform_waits_and_projects_the_exact_retina_source() {
    let ocr = Arc::new(backend().with_candidates(vec![candidate(b"ready", 0.8, 0)]));
    let (engine, source) = ExactSource::wire(ocr.clone());
    let session = source.open(&engine);
    let query = session
        .start_ocr_text_watch(
            OcrTextWatchRequest::new(
                Rect::new(CoordinateSpace::TargetLogical, 2.0, 1.0, 14.0, 10.0).unwrap(),
                ClipPolicy::Reject,
                "ready",
                0.8,
                CoordinateSpace::TargetLogical,
                OcrTextAnalysisRate::from_minimum_interval(Duration::from_nanos(1)).unwrap(),
                OcrTextStability::immediate(),
                ChangeDetectionPolicy::ExactRgba,
                OperationContext::new(),
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(query.progress().last_frame(), None);
    assert_eq!(ocr.recognition_count(), 0);
    let placement =
        TargetPlacement::new((100.0, 200.0), (16.0, 12.0), Scale::new(2.0, 2.0).unwrap()).unwrap();
    let descriptor = FrameDescriptor::packed(EXTENT, PixelFormat::Bgra8).unwrap();
    source.pixels(descriptor, vec![1; descriptor.byte_len()], Some(placement));
    let outcome = query.wait(&bounded()).unwrap();
    let result = matched(&outcome);
    assert_eq!(
        result.result().effective_region(),
        PixelRect::new(4, 2, 28, 20).unwrap()
    );
    assert_eq!(
        result.result().regions()[0].geometry().points()[0],
        Point::new(CoordinateSpace::TargetLogical, 2.5, 1.5).unwrap()
    );
    assert_eq!(result.result().transform(), result.frame().transform());
}

#[test]
fn missing_first_frame_transform_fails_explicitly_instead_of_inventing_geometry() {
    let ocr = Arc::new(backend());
    let (engine, source) = ExactSource::wire(ocr.clone());
    let session = source.open(&engine);
    let query = session
        .start_ocr_text_watch(
            OcrTextWatchRequest::new(
                Rect::new(CoordinateSpace::TargetLogical, 0.0, 0.0, 1.0, 1.0).unwrap(),
                ClipPolicy::Reject,
                "ready",
                0.8,
                CoordinateSpace::CapturePixels,
                OcrTextAnalysisRate::from_minimum_interval(Duration::from_nanos(1)).unwrap(),
                OcrTextStability::immediate(),
                ChangeDetectionPolicy::AnalysisAlways,
                OperationContext::new(),
            )
            .unwrap(),
        )
        .unwrap();
    let descriptor = FrameDescriptor::packed(EXTENT, PixelFormat::Bgra8).unwrap();
    source.pixels(descriptor, vec![1; descriptor.byte_len()], None);
    assert_eq!(
        query.wait(&bounded()).unwrap().status(),
        Some(Status::Unsupported)
    );
    assert_eq!(ocr.recognition_count(), 0);
}

#[derive(Debug)]
struct OversizedSource(FrameDescriptor);
impl FrameStorage for OversizedSource {
    fn descriptor(&self) -> FrameDescriptor {
        self.0
    }
    fn cpu_pixels(&self) -> Option<Arc<CpuPixels>> {
        None
    }
    fn read_cpu(&self, _: &OperationContext) -> Result<Arc<CpuPixels>> {
        panic!("source extent refusal must precede conversion")
    }
}

#[test]
fn a_small_roi_cannot_bypass_the_full_source_layout_ceiling() {
    let ocr = Arc::new(backend());
    let (engine, source) = ExactSource::wire(ocr.clone());
    let session = source.open(&engine);
    let query = session
        .start_ocr_text_watch(request("ready", OperationContext::new()))
        .unwrap();
    let descriptor = FrameDescriptor::new(EXTENT, PixelFormat::Bgra8, 256 * 1024 * 1024).unwrap();
    source
        .state
        .publish_storage(StoragePublication {
            captured_at: MonotonicInstant::ORIGIN,
            placement: None,
            continuity: Continuity::Continuous,
            storage: Arc::new(OversizedSource(descriptor)),
        })
        .unwrap();
    assert_eq!(
        query.wait(&bounded()).unwrap().status(),
        Some(Status::LimitExceeded)
    );
    assert_eq!(ocr.recognition_count(), 0);
}

#[test]
fn final_pending_rate_deferred_frame_drains_under_the_query_clock() {
    let harness = Harness::new(Arc::new(
        backend().with_calls(vec![call(b"missing"), call(b"ready")]),
    ));
    let session = harness.open();
    let clock = Arc::new(ManualClock::new());
    let query = session
        .start_ocr_text_watch(
            OcrTextWatchRequest::new(
                region(),
                ClipPolicy::Reject,
                "ready",
                0.8,
                CoordinateSpace::CapturePixels,
                OcrTextAnalysisRate::from_minimum_interval(Duration::from_secs(60)).unwrap(),
                OcrTextStability::immediate(),
                ChangeDetectionPolicy::AnalysisAlways,
                OperationContext::new().with_clock(clock.clone()),
            )
            .unwrap(),
        )
        .unwrap();
    harness.publish(1);
    completed(&query, 1);
    harness.publish(2);
    let final_stamp = progress(&query, |p| p.pending_count() == 1)
        .last_frame()
        .unwrap();
    harness.capture.terminate(CaptureFault::SessionClosed);
    clock.advance(Duration::from_secs(60));
    let outcome = query.wait(&bounded()).unwrap();
    assert_eq!(matched(&outcome).frame().stamp(), final_stamp);
}

#[test]
fn two_ready_sessions_rotate_queries_across_repeated_short_ocr_turns() {
    let gates: Vec<_> = (0..4).map(|_| Arc::new(CompletionGate::new())).collect();
    let ocr = Arc::new(
        backend().with_calls(
            gates
                .iter()
                .map(|gate| ScriptedOcrCall::new(Vec::new()).with_completion_gate(gate.clone()))
                .collect(),
        ),
    );
    let harness = Harness::new(ocr);
    let first = harness.open();
    let second = harness.open();
    let queries: Vec<_> = [&first, &first, &second, &second]
        .into_iter()
        .map(|session| {
            session
                .start_ocr_text_watch(request("ready", OperationContext::new()))
                .unwrap()
        })
        .collect();
    let _release: Vec<_> = gates.iter().map(CompletionGate::release_guard).collect();
    harness.publish(1);
    let mut owners = Vec::new();
    for (turn, gate) in gates.iter().enumerate() {
        assert!(gate.wait_until_entered(LIMIT));
        until(|| {
            queries
                .iter()
                .all(|query| query.progress().last_frame().is_some())
        });
        let owner = queries
            .iter()
            .position(|query| query.progress().physical_in_flight_count() == 1)
            .unwrap();
        owners.push(owner);
        harness.publish(u8::try_from(turn + 2).unwrap());
        for query in &queries {
            progress(query, |p| p.pending_count() == 1);
        }
        gate.release();
    }
    for query in &queries {
        let _ = query.cancel();
    }
    assert_eq!(owners.len(), 4);
    let mut distinct = owners.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(
        distinct,
        [0, 1, 2, 3],
        "all S*Q ready contenders receive a turn"
    );
    assert_ne!(owners[0] / 2, owners[1] / 2, "ready sessions alternate");
}

#[test]
fn barrier_deferred_template_age_survives_new_source_publications() {
    let gate = Arc::new(CompletionGate::new());
    let harness = Harness::new(Arc::new(backend()));
    let session = harness.open();
    let producer = ControlledProducer::new(EXTENT, PixelFormat::Bgra8, 1, 8).unwrap();
    producer.set_conversion_gate(Some(gate.clone()));
    let ocr = session
        .start_ocr_text_watch(request("ready", OperationContext::new()))
        .unwrap();
    let _release = gate.release_guard();
    harness.capture.publish_from(&producer, 1).unwrap();
    assert!(gate.wait_until_entered(LIMIT));
    let clock = Arc::new(ManualClock::new());
    let template = harness.template(&session, OperationContext::new().with_clock(clock.clone()));
    until(|| matches!(template.poll(), TemplateQueryOutcome::Pending(p) if p.pending_count() == 1));
    for fill in 2..=4 {
        clock.advance(Duration::from_secs(10));
        harness.capture.publish_from(&producer, fill).unwrap();
        until(|| {
            matches!(template.poll(), TemplateQueryOutcome::Pending(p)
            if p.work().get(TemplateWorkDisposition::Superseded) >= u64::from(fill - 1))
        });
    }
    clock.advance(Duration::from_secs(1));
    assert!(matches!(
        &*template.wait(&bounded()).unwrap(),
        TemplateTerminalOutcome::Overloaded(TemplateOverload::QueueExpired)
    ));
    assert_eq!(producer.conversion_attempts(), 1);
    assert_eq!(ocr.progress().physical_in_flight_count(), 1);
}

#[derive(Debug)]
struct BusyBackend {
    backend: ControlledOcr,
    slot: Mutex<()>,
}
impl OcrBackend for BusyBackend {
    fn descriptor(&self) -> OcrBackendDescriptor {
        self.backend.descriptor()
    }
    fn provider_descriptor(&self) -> Option<OcrProviderDescriptor> {
        self.backend.provider_descriptor()
    }
    fn recognize(
        &self,
        request: &OcrBackendRequest<'_>,
        output: &mut dyn OcrCandidateSink,
        operation: &OperationContext,
    ) -> Result<()> {
        let _slot = self
            .slot
            .try_lock()
            .map_err(|_| Error::new(Status::VisionFailed, "controlled OCR slot busy"))?;
        self.backend.recognize(request, output, operation)
    }
    fn close(&self, operation: &OperationContext) -> Result<()> {
        self.backend.close(operation)
    }
}

#[test]
fn one_shot_contention_remains_typed_in_both_directions_without_retry() {
    for watch_first in [true, false] {
        let gate = Arc::new(CompletionGate::new());
        let backend = Arc::new(BusyBackend {
            backend: backend().with_completion_gate(gate.clone()),
            slot: Mutex::new(()),
        });
        let harness = Harness::new(backend);
        let session = Arc::new(harness.open());
        harness.publish(1);
        let frame = session
            .acquire_frame(&FrameRequest::latest(), &bounded())
            .unwrap();
        let _release = gate.release_guard();
        if watch_first {
            let query = session
                .start_ocr_text_watch(request("ready", OperationContext::new()))
                .unwrap();
            assert!(gate.wait_until_entered(LIMIT));
            let descriptor = cpu_descriptor();
            assert_eq!(
                session
                    .recognize(OcrRequest::new(
                        &frame,
                        descriptor.backend_identity(),
                        descriptor.model_identity(),
                        OcrRegion::FullFrame,
                        CoordinateSpace::CapturePixels,
                        &bounded()
                    ))
                    .unwrap_err()
                    .status(),
                Status::VisionFailed
            );
            assert_eq!(query.progress().physical_in_flight_count(), 1);
            let _ = query.cancel();
        } else {
            let worker_session = session.clone();
            let worker = std::thread::spawn(move || {
                let descriptor = cpu_descriptor();
                worker_session.recognize(OcrRequest::new(
                    &frame,
                    descriptor.backend_identity(),
                    descriptor.model_identity(),
                    OcrRegion::FullFrame,
                    CoordinateSpace::CapturePixels,
                    &bounded(),
                ))
            });
            assert!(gate.wait_until_entered(LIMIT));
            let query = session
                .start_ocr_text_watch(request("ready", OperationContext::new()))
                .unwrap();
            assert_eq!(
                query.wait(&bounded()).unwrap().status(),
                Some(Status::VisionFailed)
            );
            assert_eq!(
                query
                    .progress()
                    .work()
                    .get(OcrTextWorkDisposition::Admitted),
                1
            );
            gate.release();
            assert!(worker.join().unwrap().is_ok());
        }
    }
}

#[test]
fn terminal_diagnostics_remain_redacted_and_late_work_cannot_emit_success() {
    let gate = Arc::new(CompletionGate::new());
    let harness = Harness::with_options(
        Some(Arc::new(backend().with_calls(vec![
            call(b"private-ready").with_completion_gate(gate.clone()),
        ]))),
        ControlledMatcher::new(PixelFormat::Rgba8),
        EngineOptions::new().with_diagnostics(DiagnosticOptions::debug(128).unwrap()),
    );
    let reader = harness.engine.take_diagnostic_reader().unwrap();
    let session = harness.open();
    let query = session
        .start_ocr_text_watch(request("private-ready", OperationContext::new()))
        .unwrap();
    let _release = gate.release_guard();
    harness.publish(1);
    assert!(gate.wait_until_entered(LIMIT));
    let outcome = query.cancel();
    assert!(!format!("{query:?} {outcome:?} {:?}", query.progress()).contains("private-ready"));
    gate.release();
    until(|| query.progress().physical_in_flight_count() == 0);
    let mut terminal = Vec::new();
    while let DiagnosticDrain::Batch(batch) = reader.drain() {
        assert!(!format!("{batch:?}").contains("private-ready"));
        for record in batch.records() {
            if let DiagnosticPayload::OcrTextWatch(value) = record.payload()
                && value.query == Some(query.id())
                && value.outcome.is_some()
            {
                terminal.push(value);
            }
        }
    }
    assert_eq!(terminal.len(), 1);
    assert_eq!(
        terminal[0].outcome,
        Some(OcrTextWatchDiagnosticOutcome::Cancelled)
    );
    assert_eq!(terminal[0].progress.unwrap().physical_in_flight_count(), 1);
}

#[test]
fn dropping_the_query_owner_does_not_free_the_physical_slot() {
    let gate = Arc::new(CompletionGate::new());
    let harness = Harness::new(Arc::new(backend().with_calls(vec![
        call(b"ready").with_completion_gate(gate.clone()),
        call(b"ready"),
    ])));
    let session = harness.open();
    let query = session
        .start_ocr_text_watch(request("ready", OperationContext::new()))
        .unwrap();
    let _release = gate.release_guard();
    harness.publish(1);
    assert!(gate.wait_until_entered(LIMIT));
    drop(query);
    assert_eq!(harness.engine.ocr_text_observation().query_count, 0);
    assert_eq!(
        harness.engine.ocr_text_observation().physical_ocr_in_flight,
        1
    );
    let next = session
        .start_ocr_text_watch(request("ready", OperationContext::new()))
        .unwrap();
    let before = progress(&next, |p| p.last_frame().is_some())
        .last_frame()
        .unwrap();
    harness.publish(2);
    progress(&next, |p| {
        p.last_frame().is_some_and(|stamp| stamp != before)
    });
    assert_eq!(next.progress().in_flight_count(), 0);
    gate.release();
    assert!(next.wait(&bounded()).unwrap().is_match());
}

#[test]
fn query_deadline_wakes_an_independent_wait_while_backend_is_held() {
    let gate = Arc::new(CompletionGate::new());
    let harness = Harness::new(Arc::new(backend().with_completion_gate(gate.clone())));
    let session = harness.open();
    let clock = Arc::new(ManualClock::new());
    let query = session
        .start_ocr_text_watch(request(
            "ready",
            OperationContext::new()
                .with_clock(clock.clone())
                .with_timeout(Duration::from_secs(1))
                .unwrap(),
        ))
        .unwrap();
    let _release = gate.release_guard();
    harness.publish(1);
    assert!(gate.wait_until_entered(LIMIT));
    clock.advance(Duration::from_secs(2));
    assert!(matches!(
        &*query.wait(&bounded()).unwrap(),
        OcrTextTerminalOutcome::DeadlineExceeded
    ));
    assert_eq!(query.progress().physical_in_flight_count(), 1);
}

#[test]
fn cancel_and_success_compete_for_one_immutable_terminal() {
    let gate = Arc::new(CompletionGate::new());
    let harness = Harness::new(Arc::new(
        backend().with_calls(vec![call(b"ready").with_completion_gate(gate.clone())]),
    ));
    let session = harness.open();
    let query = Arc::new(
        session
            .start_ocr_text_watch(request("ready", OperationContext::new()))
            .unwrap(),
    );
    let _release = gate.release_guard();
    harness.publish(1);
    assert!(gate.wait_until_entered(LIMIT));
    let contender = query.clone();
    let start = Arc::new(std::sync::Barrier::new(2));
    let other_start = start.clone();
    let cancellation = std::thread::spawn(move || {
        other_start.wait();
        contender.cancel()
    });
    start.wait();
    gate.release();
    let cancelled = cancellation.join().unwrap();
    let waited = query.wait(&bounded()).unwrap();
    assert!(Arc::ptr_eq(&cancelled, &waited));
    assert!(Arc::ptr_eq(&waited, &query.cancel()));
}

#[test]
fn executing_template_mapping_drains_before_ocr_can_claim_conversion() {
    let gate = Arc::new(CompletionGate::new());
    let harness = Harness::new(Arc::new(
        backend().with_candidates(vec![candidate(b"ready", 0.8, 0)]),
    ));
    let session = harness.open();
    let template = harness.template(&session, OperationContext::new());
    let producer = ControlledProducer::new(EXTENT, PixelFormat::Bgra8, 1, 8).unwrap();
    producer.set_conversion_gate(Some(gate.clone()));
    let _release = gate.release_guard();
    harness.capture.publish_from(&producer, 1).unwrap();
    assert!(gate.wait_until_entered(LIMIT));
    let ocr = session
        .start_ocr_text_watch(request("ready", OperationContext::new()))
        .unwrap();
    progress(&ocr, |p| p.last_frame().is_some());
    until(|| harness.engine.ocr_text_observation().mapping_barrier);
    assert_eq!(ocr.progress().physical_in_flight_count(), 0);
    let _ = template.cancel();
    assert_eq!(
        harness
            .engine
            .ocr_text_observation()
            .template_mapping_reservations,
        1
    );
    assert_eq!(producer.conversion_attempts(), 1);
    gate.release();
    assert!(ocr.wait(&bounded()).unwrap().is_match());
}

#[test]
fn shared_engine_capacity_and_physical_session_reservations_are_not_overcommitted() {
    let harness = Harness::new(Arc::new(backend()));
    let sessions: Vec<_> = (0..5).map(|_| harness.open()).collect();
    let queries: Vec<_> = sessions[..4]
        .iter()
        .flat_map(|session| {
            (0..64).map(|_| {
                session
                    .start_ocr_text_watch(request("ready", OperationContext::new()))
                    .unwrap()
            })
        })
        .collect();
    assert_eq!(
        sessions[4]
            .start_ocr_text_watch(request("ready", OperationContext::new()))
            .unwrap_err()
            .status(),
        Status::LimitExceeded
    );
    assert_eq!(harness.engine.ocr_text_observation().query_count, 256);
    drop(queries);
    until(|| harness.engine.ocr_text_observation().active_sessions == 0);

    let gate = Arc::new(CompletionGate::new());
    let harness = Harness::new(Arc::new(backend().with_completion_gate(gate.clone())));
    let first = harness.open();
    let held = first
        .start_ocr_text_watch(request("ready", OperationContext::new()))
        .unwrap();
    let _release = gate.release_guard();
    harness.publish(1);
    assert!(gate.wait_until_entered(LIMIT));
    let _ = held.cancel();
    let sessions: Vec<_> = (0..16).map(|_| harness.open()).collect();
    let queries: Vec<_> = sessions[..15]
        .iter()
        .map(|session| {
            session
                .start_ocr_text_watch(request("ready", OperationContext::new()))
                .unwrap()
        })
        .collect();
    assert_eq!(
        sessions[15]
            .start_ocr_text_watch(request("ready", OperationContext::new()))
            .unwrap_err()
            .status(),
        Status::LimitExceeded
    );
    assert_eq!(harness.engine.ocr_text_observation().active_sessions, 16);
    assert_eq!(
        harness.engine.ocr_text_observation().physical_ocr_in_flight,
        1
    );
    drop(queries);
}

#[test]
fn caller_confidence_threshold_is_not_rounded_to_profile_precision() {
    let harness = Harness::new(Arc::new(backend().with_calls(vec![
        ScriptedOcrCall::new(vec![candidate(b"ready", 0.8, 0)]),
        ScriptedOcrCall::new(vec![candidate(b"ready", 0.80001, 0)]),
    ])));
    let session = harness.open();
    let query = session
        .start_ocr_text_watch(configured(
            "ready",
            0.800001,
            OcrTextStability::immediate(),
            ChangeDetectionPolicy::AnalysisAlways,
            OperationContext::new(),
        ))
        .unwrap();
    harness.publish(1);
    assert_eq!(completed(&query, 1).confirmed_observations(), 0);
    harness.publish(2);
    let outcome = query.wait(&bounded()).unwrap();
    assert_eq!(matched(&outcome).minimum_confidence(), 0.800001);
}

#[test]
fn caller_owned_frame_clones_are_independent_of_live_result_extent_accounting() {
    let harness = Harness::new(Arc::new(
        backend().with_candidates(vec![candidate(b"ready", 0.8, 0)]),
    ));
    let session = harness.open();
    let first = session
        .start_ocr_text_watch(request("ready", OperationContext::new()))
        .unwrap();
    let second = session
        .start_ocr_text_watch(request("ready", OperationContext::new()))
        .unwrap();
    harness.publish(0x31);
    let first_result = first.wait(&bounded()).unwrap();
    let second_result = second.wait(&bounded()).unwrap();
    let frame = matched(&first_result).frame().clone();
    assert_eq!(harness.engine.ocr_text_observation().retained_results, 2);
    let extent = matched(&first_result).retained_extent().total_bytes()
        + matched(&second_result).retained_extent().total_bytes();
    assert_eq!(
        harness
            .engine
            .ocr_text_observation()
            .retained_result_extent_bytes,
        extent
    );
    drop(first);
    drop(second);
    drop(first_result);
    drop(second_result);
    // Terminal visibility precedes the worker's final query reference release.
    until(|| harness.engine.ocr_text_observation().retained_results == 0);
    assert_eq!(
        harness
            .engine
            .ocr_text_observation()
            .retained_result_extent_bytes,
        0
    );
    assert!(
        frame
            .map(PixelFormat::Bgra8, &bounded())
            .unwrap()
            .bytes()
            .iter()
            .all(|byte| *byte == 0x31)
    );
}

#[test]
fn geometry_revision_alone_revokes_old_work_without_relying_on_epoch_change() {
    let gate = Arc::new(CompletionGate::new());
    let ocr = Arc::new(backend().with_calls(vec![
        call(b"ready"),
        call(b"ready").with_completion_gate(gate.clone()),
        call(b"ready"),
        call(b"ready"),
    ]));
    let (engine, source) = ExactSource::wire(ocr);
    let session = source.open(&engine);
    let query = session
        .start_ocr_text_watch(configured(
            "ready",
            0.8,
            OcrTextStability::consecutive(2).unwrap(),
            ChangeDetectionPolicy::ExactRgba,
            OperationContext::new(),
        ))
        .unwrap();
    let _release = gate.release_guard();
    let descriptor = FrameDescriptor::packed(EXTENT, PixelFormat::Bgra8).unwrap();
    let before_placement =
        TargetPlacement::new((100.0, 200.0), (16.0, 12.0), Scale::new(2.0, 2.0).unwrap()).unwrap();
    let after_placement =
        TargetPlacement::new((200.0, 200.0), (16.0, 12.0), Scale::new(2.0, 2.0).unwrap()).unwrap();
    source.pixels(
        descriptor,
        vec![1; descriptor.byte_len()],
        Some(before_placement),
    );
    let before = completed(&query, 1).last_accepted_frame().unwrap();
    source.pixels(
        descriptor,
        vec![1; descriptor.byte_len()],
        Some(before_placement),
    );
    assert!(gate.wait_until_entered(LIMIT));
    source.pixels(
        descriptor,
        vec![1; descriptor.byte_len()],
        Some(after_placement),
    );
    let after = progress(&query, |p| {
        p.confirmed_observations() == 0
            && p.last_frame()
                .is_some_and(|stamp| stamp.geometry() != before.geometry())
    })
    .last_frame()
    .unwrap();
    assert_eq!(before.epoch(), after.epoch());
    gate.release();
    progress(&query, |p| {
        p.confirmed_observations() == 1 && p.last_accepted_frame() == Some(after)
    });
    source.pixels(
        descriptor,
        vec![1; descriptor.byte_len()],
        Some(after_placement),
    );
    let outcome = query.wait(&bounded()).unwrap();
    assert_eq!(matched(&outcome).first_confirmed_frame(), after);
    assert_eq!(
        matched(&outcome).frame().stamp().geometry(),
        after.geometry()
    );
}

#[test]
fn a_new_watch_uses_current_resized_bounds_not_the_open_description_extent() {
    let harness = Harness::new(Arc::new(
        backend().with_candidates(vec![candidate(b"ready", 0.8, 0)]),
    ));
    let session = harness.open();
    harness
        .capture
        .publish_reshaped(PixelExtent::new(40, 30), 1)
        .unwrap();
    let query = session
        .start_ocr_text_watch(
            OcrTextWatchRequest::new(
                Rect::new(CoordinateSpace::CapturePixels, 0.0, 0.0, 40.0, 30.0).unwrap(),
                ClipPolicy::Reject,
                "ready",
                0.8,
                CoordinateSpace::CapturePixels,
                OcrTextAnalysisRate::from_minimum_interval(Duration::from_nanos(1)).unwrap(),
                OcrTextStability::immediate(),
                ChangeDetectionPolicy::AnalysisAlways,
                OperationContext::new(),
            )
            .unwrap(),
        )
        .unwrap();
    let outcome = query.wait(&bounded()).unwrap();
    assert_eq!(
        matched(&outcome).result().effective_region(),
        PixelRect::new(0, 0, 40, 30).unwrap()
    );
    assert_eq!(
        matched(&outcome).frame().descriptor().extent(),
        PixelExtent::new(40, 30)
    );
}

#[test]
fn two_session_owners_of_one_maintained_stream_do_not_join_each_others_acquisition() {
    let (engine, source) = ExactSource::wire(Arc::new(
        backend().with_candidates(vec![candidate(b"ready", 0.8, 0)]),
    ));
    let first = source.open(&engine);
    let second = source.open(&engine);
    assert_eq!(first.stream(), second.stream());
    let template = engine
        .prepare_template(
            &match_fixtures::planted_template("shared-stream"),
            &bounded(),
        )
        .unwrap();
    let options = MatchOptions::from_defaults(template.defaults());
    let first_query = first
        .start_template_watch(TemplateWatchRequest::new(
            template,
            options,
            OperationContext::new(),
        ))
        .unwrap();
    let (sent, received) = mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let query = second
            .start_ocr_text_watch(request("ready", OperationContext::new()))
            .unwrap();
        sent.send((second, query)).unwrap();
    });
    let started = received.recv_timeout(LIMIT);
    if started.is_err() {
        source.state.terminate(CaptureFault::SessionClosed);
    }
    let (second, second_query) =
        started.expect("a distinct watcher must not join another live owner of the same stream");
    worker.join().unwrap();
    let descriptor = FrameDescriptor::packed(EXTENT, PixelFormat::Bgra8).unwrap();
    source.pixels(descriptor, vec![1; descriptor.byte_len()], None);
    assert!(second_query.wait(&bounded()).unwrap().is_match());
    let _ = first_query.cancel();
    first.close(&bounded()).unwrap();
    second.close(&bounded()).unwrap();
}

#[test]
fn fixed_capture_origin_errors_fail_admission_while_explicit_partial_clip_remains_valid() {
    let harness = Harness::new(Arc::new(
        backend().with_candidates(vec![candidate(b"ready", 0.8, 0)]),
    ));
    let session = harness.open();
    let make = |rect, policy| {
        OcrTextWatchRequest::new(
            rect,
            policy,
            "ready",
            0.8,
            CoordinateSpace::CapturePixels,
            OcrTextAnalysisRate::from_minimum_interval(Duration::from_nanos(1)).unwrap(),
            OcrTextStability::immediate(),
            ChangeDetectionPolicy::AnalysisAlways,
            OperationContext::new(),
        )
        .unwrap()
    };
    for (rect, policy) in [
        (
            Rect::new(CoordinateSpace::CapturePixels, -1.0, 0.0, 16.0, 16.0).unwrap(),
            ClipPolicy::Reject,
        ),
        (
            Rect::new(CoordinateSpace::CapturePixels, 0.0, -1.0, 16.0, 16.0).unwrap(),
            ClipPolicy::Reject,
        ),
        (
            Rect::new(CoordinateSpace::CapturePixels, -16.0, 0.0, 0.0, 16.0).unwrap(),
            ClipPolicy::Clip,
        ),
    ] {
        assert_eq!(
            session
                .start_ocr_text_watch(make(rect, policy))
                .unwrap_err()
                .status(),
            Status::InvalidArgument
        );
    }
    assert_eq!(harness.engine.ocr_text_observation().query_count, 0);
    let query = session
        .start_ocr_text_watch(make(
            Rect::new(CoordinateSpace::CapturePixels, -1.0, 0.0, 16.0, 16.0).unwrap(),
            ClipPolicy::Clip,
        ))
        .unwrap();
    harness.publish(1);
    let outcome = query.wait(&bounded()).unwrap();
    assert_eq!(
        matched(&outcome).result().effective_region(),
        PixelRect::new(0, 0, 16, 16).unwrap()
    );
}

#[test]
fn an_unrepresentable_next_rate_instant_never_becomes_unrestricted_analysis() {
    let ocr = Arc::new(backend());
    let harness = Harness::new(ocr.clone());
    let session = harness.open();
    let clock = Arc::new(ManualClock::new());
    let query = session
        .start_ocr_text_watch(request(
            "ready",
            OperationContext::new().with_clock(clock.clone()),
        ))
        .unwrap();
    clock.advance(Duration::MAX);
    harness.publish(1);
    completed(&query, 1);
    harness.publish(2);
    assert_eq!(
        query.wait(&bounded()).unwrap().status(),
        Some(Status::InvalidArgument)
    );
    assert_eq!(ocr.recognition_count(), 1);
}
