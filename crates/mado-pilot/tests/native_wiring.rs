//! What the facade's wiring decides, against the adapters a host actually gets.
//!
//! The orchestration rules are covered against controlled doubles in
//! `mado-pilot-runtime`, and the per-event delivery rules against the platform
//! Adapters' own drivers. What is only checkable here is the composition a host
//! receives: which capabilities an engine built by each constructor actually
//! has, that a capture-only engine says so instead of failing late, that the
//! input and receipt vocabulary a caller branches on is reachable without naming
//! a contract package, and that two engines never accept each other's
//! identities.
//!
//! On macOS the native cases tolerate a host that has not granted Screen
//! Recording. That is not laxity: this Adapter will not prompt, so a runner that
//! has neither granted nor denied it reaches the non-prompting refusal, and a
//! test that demanded capture there would fail for the host's reason rather than
//! the code's.

use mado_pilot::replay::{ReplayFrame, ReplaySource, ReplayTarget};
use mado_pilot::{
    CleanupState, Continuity, DeliveryPlan, Engine, FrameDescriptor, FrameRequest, InputDelivery,
    InputEvent, InputFault, InputOpenRequest, InputReceipt, InputRequest, InputRequirement,
    InputSequence, Key, MonotonicInstant, OpenRequest, OperationContext, PixelExtent, PixelFormat,
    PointerGeometry, SequenceOutcome, Session, SessionRequest, Status, SubmissionEvidence,
};

const FORMAT: PixelFormat = PixelFormat::Rgba8;

fn replay_engine() -> Engine {
    let descriptor =
        FrameDescriptor::packed(PixelExtent::new(16, 12), FORMAT).expect("a valid descriptor");
    let frame = ReplayFrame::new(
        descriptor,
        MonotonicInstant::ORIGIN,
        Continuity::Continuous,
        None,
        vec![0x30; descriptor.byte_len()].into_boxed_slice(),
    )
    .expect("a valid replay frame");
    let source = ReplaySource::from_targets(vec![
        ReplayTarget::new("panel", vec![frame]).expect("a valid target"),
    ])
    .expect("a valid source");

    mado_pilot::replay_engine(source).expect("an OpenCV 4 development installation")
}

fn open_capture_only(engine: &Engine, operation: &OperationContext) -> Session {
    let targets = engine.discover(operation).expect("discovered");
    engine
        .open(targets[0].id(), &OpenRequest::new(), operation)
        .expect("opened")
}

fn typing(session: &Session) -> InputRequest {
    InputRequest::new(
        session.target(),
        InputSequence::new(vec![InputEvent::KeyPress(Key::Enter)]).expect("valid"),
        DeliveryPlan::require(InputDelivery::System),
    )
}

#[test]
fn a_replay_engine_delivers_no_input_and_reads_no_authorization() {
    let engine = replay_engine();

    assert!(
        !engine.delivers_input(),
        "replay is a source of prepared frames, so there is no target to reach"
    );
    assert!(!engine.reads_permissions());

    let error = engine
        .permissions(&OperationContext::new())
        .expect_err("there is no authorization behind a replay source");
    assert_eq!(
        error.status(),
        Status::Unsupported,
        "absent is neither granted nor refused"
    );
}

#[test]
fn a_replay_engine_describes_its_own_target_as_accepting_no_input() {
    let engine = replay_engine();
    let operation = OperationContext::new();
    let targets = engine.discover(&operation).expect("discovered");

    let descriptor = engine
        .describe_input(targets[0].id(), &operation)
        .expect("its own target is describable");

    assert!(!descriptor.is_available());
    assert_eq!(descriptor.target(), targets[0].id());
}

#[test]
fn optional_input_on_a_capture_only_engine_opens_a_truthful_session() {
    let engine = replay_engine();
    let operation = OperationContext::new();
    let targets = engine.discover(&operation).expect("discovered");

    let session = engine
        .open_session(
            targets[0].id(),
            &SessionRequest::new()
                .capturing(OpenRequest::new())
                .requesting_input(InputOpenRequest::new()),
            &operation,
        )
        .expect("optional input is not a reason to refuse capture");

    assert!(!session.accepts_input());
    assert!(!session.input_descriptor().is_available());
    // Capture is unaffected: this is a working session that simply has no input.
    session
        .acquire_frame(&FrameRequest::latest(), &operation)
        .expect("a replayed frame");
    session.close(&operation).expect("closed");
    assert!(session.is_closed());
}

#[test]
fn required_input_on_a_capture_only_engine_refuses_the_open() {
    let engine = replay_engine();
    let operation = OperationContext::new();
    let targets = engine.discover(&operation).expect("discovered");

    let error = engine
        .open_session(
            targets[0].id(),
            &SessionRequest::new().requesting_input(
                InputOpenRequest::new().with_requirement(InputRequirement::Required),
            ),
            &operation,
        )
        .expect_err("the caller said the session is not useful without input");

    assert_eq!(error.status(), Status::Unsupported);
}

#[test]
fn a_capture_only_session_refuses_a_sequence_instead_of_pretending_to_deliver_it() {
    let engine = replay_engine();
    let operation = OperationContext::new();
    let session = open_capture_only(&engine, &operation);

    let error = session
        .send_input(&typing(&session), &operation)
        .expect_err("this session established no input");

    assert_eq!(error.status(), Status::Unsupported);
    session.close(&operation).expect("closed");
}

#[test]
fn a_frame_a_session_published_can_be_named_as_an_input_coordinate_source() {
    let engine = replay_engine();
    let elsewhere = replay_engine();
    let operation = OperationContext::new();
    let session = open_capture_only(&engine, &operation);
    let other = open_capture_only(&elsewhere, &operation);
    let own = session
        .acquire_frame(&FrameRequest::latest(), &operation)
        .expect("a replayed frame")
        .stamp();
    let foreign = other
        .acquire_frame(&FrameRequest::latest(), &operation)
        .expect("a replayed frame")
        .stamp();

    // Correlation reaches input through the facade's own vocabulary: the exact
    // frame identity a search would be correlated with is the one a geometry
    // policy names, and the two stamps are distinguishable without either
    // caller knowing how the engines were wired.
    assert_ne!(own.stream(), foreign.stream());
    let request =
        typing(&session).with_pointer_geometry(PointerGeometry::require_unchanged_since(own));
    assert_eq!(request.pointer_geometry().source(), Some(own));

    // Which refusal a capture-only session gives is settled rather than
    // incidental: it has no input at all, and that is the fact a caller acts on
    // whichever frame the request named. The stream rule itself is exercised
    // against a session that does have input, in `mado-pilot-runtime`.
    let error = session
        .send_input(&request, &operation)
        .expect_err("this session established no input");
    assert_eq!(error.status(), Status::Unsupported);
}

#[test]
fn two_engines_never_accept_each_others_identities() {
    let engine = replay_engine();
    let elsewhere = replay_engine();
    let operation = OperationContext::new();
    let foreign = elsewhere.discover(&operation).expect("discovered")[0].id();

    assert_ne!(engine.id(), elsewhere.id());
    assert_eq!(
        engine
            .describe_input(foreign, &operation)
            .expect_err("another engine's identity")
            .status(),
        Status::InvalidArgument
    );
    assert_eq!(
        engine
            .open(foreign, &OpenRequest::new(), &operation)
            .expect_err("another engine's identity")
            .status(),
        Status::InvalidArgument
    );
}

#[test]
fn the_receipt_vocabulary_a_caller_branches_on_is_reachable_from_the_facade() {
    let engine = replay_engine();
    let operation = OperationContext::new();
    let session = open_capture_only(&engine, &operation);

    // A partial receipt is what a native delivery produces when it stops
    // part-way, and this asserts that a host can name and read every part of one
    // through this package alone — the outcome, the mechanism, the count, the
    // reason, and what cleanup managed to release.
    let receipt = InputReceipt::partial(
        session.target(),
        InputDelivery::System,
        SubmissionEvidence::SystemInputAdmission,
        2,
        false,
        InputFault::SubmissionFailed,
    )
    .with_cleanup(1, 2);

    assert_eq!(receipt.outcome(), SequenceOutcome::Partial);
    assert!(!receipt.is_complete());
    assert_eq!(receipt.submitted(), 2);
    assert_eq!(receipt.last_submitted(), Some(1));
    assert_eq!(receipt.selected_route(), Some(InputDelivery::System));
    assert_eq!(receipt.fault(), Some(InputFault::SubmissionFailed));
    assert_eq!(receipt.cleanup(), CleanupState::Incomplete);
    assert!(receipt.cleanup().may_leave_state_held());
    session.close(&operation).expect("closed");
}

#[cfg(target_os = "macos")]
mod macos {
    use mado_pilot::{
        NativeEngineRequest, OpenRequest, OperationContext, PermissionKind, REQUIRED_BACKEND,
        Status,
    };

    /// Statuses a host without Screen Recording authorization legitimately
    /// reports, which every native case below tolerates.
    fn is_unavailable(status: Status) -> bool {
        matches!(status, Status::Unsupported | Status::CaptureFailed)
    }

    #[test]
    fn a_macos_engine_selects_the_required_backend_before_it_wires_anything() {
        let engine = mado_pilot::macos_engine(NativeEngineRequest::new())
            .expect("an OpenCV 4 development installation");

        assert_eq!(
            engine.backend().id(),
            REQUIRED_BACKEND,
            "the one step that can fail on its own runs first"
        );
    }

    #[test]
    fn a_macos_engine_delivers_input_and_reads_both_authorizations() {
        let engine =
            mado_pilot::macos_engine(NativeEngineRequest::new()).expect("an OpenCV installation");

        assert!(engine.delivers_input());
        assert!(engine.reads_permissions());

        // Which states this host reports depends on what the user granted this
        // process, so the assertion is about independence rather than values.
        // Nothing here prompts, opens settings, or presents any interface.
        let report = engine
            .permissions(&OperationContext::new())
            .expect("a non-prompting probe always answers");

        assert_eq!(report.capture().kind(), PermissionKind::ScreenCapture);
        assert_eq!(report.input().kind(), PermissionKind::InputControl);
        assert_eq!(
            report.outcome(PermissionKind::InputControl),
            report.input(),
            "one authorization never stands in for the other"
        );
    }

    #[test]
    fn a_macos_engine_refuses_another_engines_identity() {
        let engine =
            mado_pilot::macos_engine(NativeEngineRequest::new()).expect("an OpenCV installation");
        let elsewhere =
            mado_pilot::macos_engine(NativeEngineRequest::new()).expect("an OpenCV installation");
        let operation = OperationContext::new();

        let targets = match elsewhere.discover(&operation) {
            Ok(targets) => targets,
            Err(error) if is_unavailable(error.status()) => return,
            Err(error) => panic!("discovery failed on an authorized host: {error}"),
        };
        let Some(foreign) = targets.first().map(|target| target.id()) else {
            return;
        };

        assert_ne!(engine.id(), elsewhere.id());
        assert_eq!(
            engine
                .open(foreign, &OpenRequest::new(), &operation)
                .expect_err("another engine's identity")
                .status(),
            Status::InvalidArgument
        );
        assert_eq!(
            engine
                .describe_input(foreign, &operation)
                .expect_err("another engine's identity")
                .status(),
            Status::InvalidArgument
        );
    }

    #[test]
    fn a_macos_engine_describes_what_a_discovered_target_accepts() {
        let engine =
            mado_pilot::macos_engine(NativeEngineRequest::new()).expect("an OpenCV installation");
        let operation = OperationContext::new();

        let targets = match engine.discover(&operation) {
            Ok(targets) => targets,
            Err(error) if is_unavailable(error.status()) => return,
            Err(error) => panic!("discovery failed on an authorized host: {error}"),
        };
        let Some(target) = targets.first() else {
            return;
        };

        match engine.describe_input(target.id(), &operation) {
            Ok(descriptor) => {
                assert_eq!(descriptor.target(), target.id());
                assert!(
                    descriptor.limits().max_events() <= mado_pilot::SequenceLimits::MAX_EVENTS,
                    "an Adapter may tighten the contract ceiling and never widen it"
                );
            }
            // A target can go between discovery and the description of it, which
            // is the answer rather than a failure of this test.
            Err(error) => assert_eq!(error.status(), Status::TargetLost),
        }
    }
}
