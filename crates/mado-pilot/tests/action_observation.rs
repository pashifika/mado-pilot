//! The facade keeps action and observation as explicit correlated operations.

use std::sync::Arc;

use mado_pilot::{
    ActivityTag, CancellationToken, CapabilitySupport, Continuity, CoordinateSpace, DeliveryPlan,
    DiagnosticDrain, DiagnosticOperationKind, DiagnosticOptions, DiagnosticPayload, EngineOptions,
    FindRequest, FrameOrder, FrameRequest, InputAddressScope, InputCapability, InputDelivery,
    InputEvent, InputOpenRequest, InputOperationKind, InputRequest, InputRequirement,
    InputSequence, Key, MatchOptions, MonotonicInstant, OpenRequest, OperationContext, PixelFormat,
    SequenceOutcome, SessionRequest, Status, SubmissionEvidence,
};
use mado_pilot_runtime::{EngineWiring, IdentityIssuer, Matcher, PackageLoader};
use mado_pilot_testkit::{
    ControlledCapture, ControlledInput, ControlledMatcher, ManualClock, match_fixtures,
};

fn enter(target: mado_pilot::TargetId) -> InputRequest {
    InputRequest::new(
        target,
        InputSequence::new(vec![InputEvent::KeyPress(Key::Enter)])
            .expect("one valid logical event"),
        DeliveryPlan::require(InputDelivery::System),
    )
}

fn process_directed_enter(target: mado_pilot::TargetId) -> InputRequest {
    InputRequest::new(
        target,
        InputSequence::new(vec![InputEvent::KeyPress(Key::Enter)])
            .expect("one valid logical event"),
        DeliveryPlan::require(InputDelivery::ProcessDirected),
    )
}

fn process_directed_keyboard_capability() -> InputCapability {
    InputCapability::none()
        .with_pair(
            InputOperationKind::Keyboard,
            InputDelivery::ProcessDirected,
            CapabilitySupport::Unknown,
            SubmissionEvidence::InvocationOnly,
        )
        .with_pair(
            InputOperationKind::Keyboard,
            InputDelivery::System,
            CapabilitySupport::Supported,
            SubmissionEvidence::SystemInputAdmission,
        )
}

#[test]
fn process_directed_invocation_and_newer_visual_observation_remain_independent_facts() {
    let issuer = Arc::new(IdentityIssuer::new());
    let capture = Arc::new(
        ControlledCapture::new(
            Arc::clone(&issuer),
            match_fixtures::SCENE,
            PixelFormat::Rgba8,
        )
        .expect("valid controlled capture"),
    );
    let input = Arc::new(ControlledInput::with_capability(
        capture.target(),
        process_directed_keyboard_capability(),
    ));
    let engine = mado_pilot::Engine::new(EngineWiring {
        engine: issuer.engine(),
        capture: capture.clone(),
        matcher: Matcher::new(Arc::new(ControlledMatcher::new(PixelFormat::Rgba8))),
        loader: PackageLoader::new(),
        ocr: None,
        input: Some(input.clone()),
        permission: None,
    })
    .expect("the controlled adapters share one provider");
    let operation = OperationContext::new();
    let session = engine
        .open_session(
            capture.target(),
            &SessionRequest::new()
                .capturing(OpenRequest::new())
                .requesting_input(
                    InputOpenRequest::new()
                        .with_requirement(InputRequirement::Required)
                        .requiring(InputOperationKind::Keyboard, InputDelivery::ProcessDirected),
                ),
            &operation,
        )
        .expect("capture and input opened");
    let process_pair = session
        .input_descriptor()
        .capability()
        .pair(InputOperationKind::Keyboard, InputDelivery::ProcessDirected);
    assert_eq!(process_pair.support(), CapabilitySupport::Unknown);
    assert!(process_pair.may_attempt());
    assert_eq!(
        process_pair.address_scope(),
        InputAddressScope::OwningProcess
    );
    assert_eq!(
        process_pair.evidence(),
        Some(SubmissionEvidence::InvocationOnly)
    );

    assert_eq!(
        session
            .input_descriptor()
            .capability()
            .pair(InputOperationKind::Keyboard, InputDelivery::System)
            .support(),
        CapabilitySupport::Supported,
        "system input is separately available but not selected implicitly"
    );

    let template = engine
        .prepare_template(
            &match_fixtures::planted_template("expected-state"),
            &operation,
        )
        .expect("template prepared");

    capture
        .publish(0x11, Continuity::Continuous)
        .expect("source frame published");
    let before = session
        .acquire_frame(&FrameRequest::latest(), &operation)
        .expect("source frame acquired");
    let precondition = session
        .find_template(
            &FindRequest::exact(
                &before,
                &template,
                MatchOptions::from_defaults(template.defaults()),
            ),
            &operation,
        )
        .expect("action precondition evaluated");
    assert!(precondition.result().is_empty());
    assert_eq!(precondition.result().stamp(), before.stamp());

    let receipt = session
        .send_input(&process_directed_enter(session.target()), &operation)
        .expect("the sequence was admitted");
    assert_eq!(receipt.outcome(), SequenceOutcome::Complete);
    assert_eq!(receipt.submitted(), 1);
    assert_eq!(
        receipt.selected_route(),
        Some(InputDelivery::ProcessDirected)
    );
    assert_eq!(
        receipt.address_scope(),
        Some(InputAddressScope::OwningProcess)
    );
    assert_eq!(receipt.evidence(), Some(SubmissionEvidence::InvocationOnly));
    assert!(!receipt.used_fallback());
    assert_eq!(receipt.attempts().len(), 1);
    assert_eq!(
        input.admitted()[0].routes,
        vec![InputDelivery::ProcessDirected],
        "caller opt-in permits no implicit system route"
    );

    capture
        .publish(0x22, Continuity::Continuous)
        .expect("post-action frame published");
    let after = session
        .acquire_frame(&FrameRequest::newer_than(before.stamp()), &operation)
        .expect("a strictly newer source-correlated frame");
    assert_eq!(before.stamp().order(&after.stamp()), Ok(FrameOrder::Before));

    let expected = session
        .find_template(
            &FindRequest::exact(
                &after,
                &template,
                MatchOptions::from_defaults(template.defaults()),
            ),
            &operation,
        )
        .expect("expected visual condition evaluated independently");
    assert!(expected.result().is_empty());
    assert_eq!(expected.result().stamp(), after.stamp());
    assert_eq!(
        receipt.submitted(),
        1,
        "observation does not rewrite the receipt"
    );

    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let cancelled = session
        .send_input(
            &enter(session.target()),
            &OperationContext::new().with_cancellation(cancellation),
        )
        .expect_err("cancellation wins before input admission");
    assert_eq!(cancelled.status(), Status::Cancelled);
    assert_eq!(input.submitted_events().len(), 1);

    let deadline = session
        .send_input(
            &enter(session.target()),
            &OperationContext::new()
                .with_clock(Arc::new(ManualClock::new()))
                .with_deadline(MonotonicInstant::ORIGIN),
        )
        .expect_err("an expired deadline wins before input admission");
    assert_eq!(deadline.status(), Status::DeadlineExceeded);
    assert_eq!(input.submitted_events().len(), 1);

    let observation_timeout = session
        .acquire_frame(
            &FrameRequest::newer_than(after.stamp()),
            &OperationContext::new()
                .with_clock(Arc::new(ManualClock::new()))
                .with_deadline(MonotonicInstant::ORIGIN),
        )
        .expect_err("an expired observation does not rewrite the input receipt");
    assert_eq!(observation_timeout.status(), Status::DeadlineExceeded);
    assert_eq!(receipt.outcome(), SequenceOutcome::Complete);
    assert_eq!(receipt.submitted(), 1);

    capture.lose(session.target());
    let target_lost = session
        .acquire_frame(&FrameRequest::latest(), &operation)
        .expect_err("target loss is an observation failure");
    assert_eq!(target_lost.status(), Status::TargetLost);
    assert_eq!(receipt.outcome(), SequenceOutcome::Complete);
    assert_eq!(receipt.submitted(), 1);
    assert!(expected.result().is_empty());
    assert_eq!(expected.result().stamp(), after.stamp());

    session.close(&operation).expect("closed");
    drop(session);
    drop(engine);
    assert_eq!(receipt.outcome(), SequenceOutcome::Complete);
    assert_eq!(receipt.submitted(), 1);
    assert!(expected.result().is_empty());
}

#[test]
fn gated_process_pair_refuses_without_substituting_available_system_input() {
    let issuer = Arc::new(IdentityIssuer::new());
    let capture = Arc::new(
        ControlledCapture::new(
            Arc::clone(&issuer),
            match_fixtures::SCENE,
            PixelFormat::Rgba8,
        )
        .expect("valid controlled capture"),
    );
    let capability = InputCapability::none().with_pair(
        InputOperationKind::Keyboard,
        InputDelivery::System,
        CapabilitySupport::Supported,
        SubmissionEvidence::SystemInputAdmission,
    );
    let input = Arc::new(ControlledInput::with_capability(
        capture.target(),
        capability,
    ));
    let engine = mado_pilot::Engine::new(EngineWiring {
        engine: issuer.engine(),
        capture: capture.clone(),
        matcher: Matcher::new(Arc::new(ControlledMatcher::new(PixelFormat::Rgba8))),
        loader: PackageLoader::new(),
        ocr: None,
        input: Some(input.clone()),
        permission: None,
    })
    .expect("the controlled adapters share one provider");
    let operation = OperationContext::new();

    let gated = engine
        .open_session(
            capture.target(),
            &SessionRequest::new()
                .capturing(OpenRequest::new())
                .requesting_input(
                    InputOpenRequest::new()
                        .with_requirement(InputRequirement::Required)
                        .requiring(InputOperationKind::Keyboard, InputDelivery::ProcessDirected),
                ),
            &operation,
        )
        .expect_err("an unqualified process pair cannot be established");
    assert_eq!(gated.status(), Status::Unsupported);

    let session = engine
        .open_session(
            capture.target(),
            &SessionRequest::new()
                .capturing(OpenRequest::new())
                .requesting_input(InputOpenRequest::new()),
            &operation,
        )
        .expect("the separately supported system pair may establish input");
    assert_eq!(
        session
            .input_descriptor()
            .capability()
            .pair(InputOperationKind::Keyboard, InputDelivery::System)
            .support(),
        CapabilitySupport::Supported
    );

    let receipt = session
        .send_input(&process_directed_enter(session.target()), &operation)
        .expect("route-local refusal is an immutable unexecuted receipt");
    assert_eq!(receipt.outcome(), SequenceOutcome::Unexecuted);
    assert_eq!(receipt.selected_route(), None);
    assert_eq!(receipt.submitted(), 0);
    assert!(!receipt.used_fallback());
    assert_eq!(receipt.attempts().len(), 1);
    assert_eq!(
        receipt.attempts()[0].route(),
        InputDelivery::ProcessDirected
    );
    assert_eq!(
        receipt.attempts()[0].address_scope(),
        InputAddressScope::OwningProcess
    );
    assert_eq!(
        receipt.attempts()[0].fault(),
        Some(mado_pilot::InputFault::UnsupportedCombination)
    );
    assert!(input.admitted().is_empty());
    assert!(input.submitted_events().is_empty());

    session.close(&operation).expect("closed");
}

#[test]
fn retained_frame_mapping_is_debug_observed_without_delaying_stream_seal() {
    let issuer = Arc::new(IdentityIssuer::new());
    let capture = Arc::new(
        ControlledCapture::new(
            Arc::clone(&issuer),
            match_fixtures::SCENE,
            PixelFormat::Rgba8,
        )
        .expect("valid controlled capture"),
    );
    let engine = mado_pilot::Engine::new_with_options(
        EngineWiring {
            engine: issuer.engine(),
            capture: capture.clone(),
            matcher: Matcher::new(Arc::new(ControlledMatcher::new(PixelFormat::Rgba8))),
            loader: PackageLoader::new(),
            ocr: None,
            input: None,
            permission: None,
        },
        EngineOptions::new()
            .with_diagnostics(DiagnosticOptions::debug(32).expect("bounded diagnostics")),
    )
    .expect("the controlled adapter opens");
    let reader = engine
        .take_diagnostic_reader()
        .expect("an enabled engine exposes one reader");
    let operation = OperationContext::new()
        .with_activity_tag(ActivityTag::new(0x4d41_5050_494e_4701).expect("nonzero activity"));
    let session = engine
        .open_session(
            capture.target(),
            &SessionRequest::new().capturing(OpenRequest::new()),
            &operation,
        )
        .expect("session opens");
    capture
        .publish(0x44, Continuity::Continuous)
        .expect("source frame published");
    let frame = session
        .acquire_frame(&FrameRequest::latest(), &operation)
        .expect("source frame acquired");
    let observer = session.mapping_observer();
    session.close(&operation).expect("session closes");
    drop(session);

    let mapping = observer
        .map_frame(&frame, PixelFormat::Rgba8, &operation)
        .expect("a retained frame maps after session release");
    assert_eq!(mapping.stamp(), frame.stamp());
    assert!(!mapping.bytes().is_empty());
    drop(engine);

    let batch = match reader.drain() {
        DiagnosticDrain::Batch(batch) => batch,
        other => panic!("expected retained mapping diagnostics, got {other:?}"),
    };
    let mapping_record = batch
        .records()
        .iter()
        .find(|record| {
            matches!(
                record.payload(),
                DiagnosticPayload::Mapping(mapping) if mapping.frame == frame.stamp()
            )
        })
        .expect("one debug mapping fact");
    let DiagnosticPayload::Mapping(mapped) = mapping_record.payload() else {
        unreachable!("selected by mapping payload")
    };
    assert_eq!(mapped.target, capture.target());
    assert_eq!(mapped.source, CoordinateSpace::CapturePixels);
    assert_eq!(mapped.destination, CoordinateSpace::CapturePixels);
    assert_eq!(mapping_record.activity(), operation.activity_tag());
    assert!(batch.records().iter().any(|record| {
        record.operation() == mapping_record.operation()
            && matches!(
                record.payload(),
                DiagnosticPayload::OperationStarted(started)
                    if started.operation == DiagnosticOperationKind::Mapping
            )
    }));
    assert!(matches!(reader.drain(), DiagnosticDrain::EndOfStream));
}

#[test]
fn retained_diagnostics_survive_repeated_sessions_reader_drop_and_engine_drop() {
    const SESSION_CYCLES: usize = 64;

    let issuer = Arc::new(IdentityIssuer::new());
    let capture = Arc::new(
        ControlledCapture::new(
            Arc::clone(&issuer),
            match_fixtures::SCENE,
            PixelFormat::Rgba8,
        )
        .expect("valid controlled capture"),
    );
    let engine = mado_pilot::Engine::new_with_options(
        EngineWiring {
            engine: issuer.engine(),
            capture: capture.clone(),
            matcher: Matcher::new(Arc::new(ControlledMatcher::new(PixelFormat::Rgba8))),
            loader: PackageLoader::new(),
            ocr: None,
            input: None,
            permission: None,
        },
        EngineOptions::new().with_diagnostics(
            DiagnosticOptions::debug(SESSION_CYCLES * 8).expect("bounded diagnostics"),
        ),
    )
    .expect("the controlled adapter opens");
    let reader = engine
        .take_diagnostic_reader()
        .expect("an enabled engine exposes one reader");
    let operation = OperationContext::new();

    for _ in 0..SESSION_CYCLES {
        let session = engine
            .open_session(
                capture.target(),
                &SessionRequest::new().capturing(OpenRequest::new()),
                &operation,
            )
            .expect("session opens");
        session.close(&operation).expect("session closes");
    }

    let batch = match reader.drain() {
        DiagnosticDrain::Batch(batch) => batch,
        other => panic!("expected retained lifecycle diagnostics, got {other:?}"),
    };
    assert_eq!(batch.losses().normal(), 0);
    assert_eq!(batch.losses().debug(), 0);
    let before: Vec<_> = batch
        .records()
        .iter()
        .map(|record| (record.sequence(), record.kind(), record.operation()))
        .collect();
    assert!(
        before.len() >= SESSION_CYCLES * 2,
        "every open/close cycle retains lifecycle evidence"
    );
    assert!(
        before
            .windows(2)
            .all(|pair| pair[0].0.get() < pair[1].0.get()),
        "the batch retains the engine's total commit order"
    );

    drop(reader);
    drop(engine);

    let after: Vec<_> = batch
        .records()
        .iter()
        .map(|record| (record.sequence(), record.kind(), record.operation()))
        .collect();
    assert_eq!(after, before, "the owned batch is lifecycle-independent");
}
