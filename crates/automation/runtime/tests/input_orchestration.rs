//! What the runtime decides about input that no contract below it can.
//!
//! The per-event work of delivering a sequence belongs to the Adapter that holds
//! the native target, and these tests do not re-check it. What they check is the
//! composition: which adapters may be wired together, what an open leaves behind
//! when it refuses, which requests a session refuses before an event exists,
//! which answer wins when the caller's operation and the controller's answer race,
//! and what one close finishes.

mod support;

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use mado_pilot_runtime::{
    CancellationToken, CapabilitySupport, CaptureProvider, Continuity, DeliveryPlan, FocusPolicy,
    FrameRequest, FrameStamp, GeometryPolicy, InputCapability, InputDelivery, InputEvent,
    InputFault, InputOpenRequest, InputOperationKind, InputProvider, InputRequest,
    InputRequirement, InputSequence, Key, Modifier, MonotonicInstant, OpenRequest,
    OperationContext, PermissionKind, PermissionProbe, PermissionState, PointerGeometry,
    PressedState, ProviderId, SequenceOutcome, Session, SessionRequest, Status, SubmissionEvidence,
    TargetId,
};
use mado_pilot_testkit::controlled_input::{Behavior, Cleanup};
use mado_pilot_testkit::{ControlledInput, ManualClock, ScriptedPermissionProbe};

use support::{Answer, CountingCapture, Harness, LateAnswer};

/// A request the default controlled capability accepts whole.
fn chord() -> InputSequence {
    InputSequence::new(vec![
        InputEvent::KeyPress(Key::Modifier(Modifier::Control)),
        InputEvent::KeyPress(Key::Character('c')),
        InputEvent::KeyRelease(Key::Character('c')),
        InputEvent::KeyRelease(Key::Modifier(Modifier::Control)),
    ])
    .expect("valid")
}

fn typing(target: TargetId) -> InputRequest {
    InputRequest::new(
        target,
        InputSequence::new(vec![InputEvent::KeyPress(Key::Enter)]).expect("valid"),
        DeliveryPlan::require(InputDelivery::System),
    )
}

/// Opens one session with optional input, on a harness that publishes a frame.
fn opened_with_input(harness: &Harness, operation: &OperationContext) -> Session {
    let session = harness
        .engine
        .open_session(
            harness.capture.target(),
            &SessionRequest::new()
                .capturing(OpenRequest::new())
                .requesting_input(InputOpenRequest::new()),
            operation,
        )
        .expect("opened");
    harness
        .capture
        .publish(0x11, Continuity::Continuous)
        .expect("published");
    session
}

/// Returns a frame stamp this session actually published.
fn stamp(session: &Session, operation: &OperationContext) -> FrameStamp {
    session
        .acquire_frame(&FrameRequest::latest(), operation)
        .expect("a published frame")
        .stamp()
}

/// An operation whose deadline has already passed on its own clock.
fn expired() -> OperationContext {
    OperationContext::new()
        .with_clock(Arc::new(ManualClock::new()))
        .with_deadline(MonotonicInstant::ORIGIN)
}

/// Waits for a test-visible fact without guessing how quickly a runner schedules work.
fn wait_until(what: &str, predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !predicate() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::yield_now();
    }
}

#[test]
fn an_input_adapter_from_another_provider_cannot_be_wired_to_this_capture() {
    let (issuer, capture) = support::controlled_capture();
    // Issued by the same engine, so only the provider differs — which is the
    // whole mistake: ordinals are per provider, and a foreign adapter handed one
    // may well own a real target of its own under that number.
    let foreign_target = issuer
        .issue_target(ProviderId::new("elsewhere"))
        .expect("issued");
    let foreign = Arc::new(ForeignInput {
        inner: ControlledInput::new(foreign_target),
    });

    let error = support::wire(
        &issuer,
        capture as Arc<dyn CaptureProvider>,
        Some(foreign as Arc<dyn InputProvider>),
        None,
    )
    .expect_err("a mismatched pairing is refused at the composition root");

    assert_eq!(error.status(), Status::InvalidArgument);
}

#[test]
fn a_permission_probe_from_another_provider_cannot_be_wired_to_this_capture() {
    let (issuer, capture) = support::controlled_capture();
    let probe = Arc::new(ScriptedPermissionProbe::granting());

    let error = support::wire(
        &issuer,
        capture as Arc<dyn CaptureProvider>,
        None,
        Some(probe as Arc<dyn PermissionProbe>),
    )
    .expect_err("a probe cannot report another platform's authorization");

    assert_eq!(error.status(), Status::InvalidArgument);
}

#[test]
fn a_capture_only_engine_describes_its_own_targets_as_accepting_no_input() {
    let harness = Harness::silent();
    let operation = OperationContext::new();

    assert!(!harness.engine.delivers_input());
    let descriptor = harness
        .engine
        .describe_input(harness.capture.target(), &operation)
        .expect("its own target is describable");

    assert!(!descriptor.is_available());
    assert_eq!(descriptor.target(), harness.capture.target());
}

#[test]
fn a_capture_only_engine_still_refuses_a_target_it_did_not_issue() {
    let harness = Harness::silent();
    let foreign = mado_pilot_runtime::IdentityIssuer::new()
        .issue_target(ProviderId::new("controlled"))
        .expect("issued");

    let error = harness
        .engine
        .describe_input(foreign, &OperationContext::new())
        .expect_err("another engine's identity");

    assert_eq!(error.status(), Status::InvalidArgument);
}

#[test]
fn an_engine_without_a_probe_reads_no_authorization_rather_than_inventing_one() {
    let harness = Harness::with_input();

    assert!(!harness.engine.reads_permissions());
    let error = harness
        .engine
        .permissions(&OperationContext::new())
        .expect_err("nothing here can read an authorization");

    assert_eq!(
        error.status(),
        Status::Unsupported,
        "absent is not granted and not refused"
    );
}

#[test]
fn an_engine_with_a_probe_reports_both_authorizations_independently() {
    let (issuer, capture) = support::controlled_capture();
    let probe = Arc::new(
        ScriptedPermissionProbe::new(
            mado_pilot_testkit::Answer::granted(),
            mado_pilot_testkit::Answer::not_granted(),
        )
        .for_provider(ProviderId::new("controlled")),
    );
    let engine = support::wire(
        &issuer,
        capture as Arc<dyn CaptureProvider>,
        None,
        Some(Arc::clone(&probe) as Arc<dyn PermissionProbe>),
    )
    .expect("one provider");
    let operation = OperationContext::new();

    assert!(engine.reads_permissions());
    let report = engine.permissions(&operation).expect("both reads answered");

    assert!(report.capture().is_granted());
    assert_eq!(report.input().state(), PermissionState::NotGranted);
    assert_eq!(
        engine
            .permission(PermissionKind::InputControl, &operation)
            .expect("one read")
            .state(),
        PermissionState::NotGranted
    );
}

#[test]
fn a_session_opened_without_input_reports_none_and_delivers_none() {
    let harness = Harness::with_input();
    let operation = OperationContext::new();
    let session = harness
        .engine
        .open(harness.capture.target(), &OpenRequest::new(), &operation)
        .expect("opened");

    assert!(!session.accepts_input());
    assert!(!session.input_descriptor().is_available());

    let error = session
        .send_input(&typing(session.target()), &operation)
        .expect_err("this session established no input");

    assert_eq!(error.status(), Status::Unsupported);
    assert!(harness.input().submitted_events().is_empty());
}

#[test]
fn optional_input_that_cannot_be_established_opens_the_session_capture_only() {
    let (issuer, capture) = support::controlled_capture();
    let counting = Arc::new(CountingCapture::new(Arc::clone(&capture)));
    let closes = counting.closes();
    // A target that accepts nothing: the open is refused by the input adapter,
    // not by the request.
    let input = Arc::new(ControlledInput::with_capability(
        capture.target(),
        InputCapability::none(),
    ));
    let engine = support::wire(
        &issuer,
        counting as Arc<dyn CaptureProvider>,
        Some(input as Arc<dyn InputProvider>),
        None,
    )
    .expect("one provider");
    let operation = OperationContext::new();

    let session = engine
        .open_session(
            capture.target(),
            &SessionRequest::new().requesting_input(InputOpenRequest::new()),
            &operation,
        )
        .expect("optional input is not a reason to refuse capture");

    assert!(!session.accepts_input());
    assert!(
        !session.input_descriptor().is_available(),
        "the descriptor reports what was accepted, not what was asked for"
    );
    assert_eq!(
        closes.load(std::sync::atomic::Ordering::Relaxed),
        0,
        "the capture session the caller did get was not released"
    );
    session.close(&operation).expect("closed");
}

#[test]
fn required_input_that_cannot_be_established_closes_the_capture_it_committed() {
    let (issuer, capture) = support::controlled_capture();
    let counting = Arc::new(CountingCapture::new(Arc::clone(&capture)));
    let closes = counting.closes();
    let input = Arc::new(ControlledInput::with_capability(
        capture.target(),
        InputCapability::none(),
    ));
    let engine = support::wire(
        &issuer,
        counting as Arc<dyn CaptureProvider>,
        Some(input as Arc<dyn InputProvider>),
        None,
    )
    .expect("one provider");

    let error = engine
        .open_session(
            capture.target(),
            &SessionRequest::new().requesting_input(
                InputOpenRequest::new().with_requirement(InputRequirement::Required),
            ),
            &OperationContext::new(),
        )
        .expect_err("the session is not useful without input");

    assert_eq!(error.status(), Status::Unsupported);
    assert_eq!(
        closes.load(std::sync::atomic::Ordering::Relaxed),
        1,
        "the capture committed for a session that will not exist is closed, not dropped"
    );
}

#[test]
fn required_input_an_engine_has_no_adapter_for_is_refused_before_capture_survives() {
    let (issuer, capture) = support::controlled_capture();
    let counting = Arc::new(CountingCapture::new(Arc::clone(&capture)));
    let closes = counting.closes();
    let engine = support::wire(&issuer, counting as Arc<dyn CaptureProvider>, None, None)
        .expect("a capture-only engine");

    let error = engine
        .open_session(
            capture.target(),
            &SessionRequest::new().requesting_input(
                InputOpenRequest::new().with_requirement(InputRequirement::Required),
            ),
            &OperationContext::new(),
        )
        .expect_err("this engine delivers no input at all");

    assert_eq!(error.status(), Status::Unsupported);
    assert_eq!(closes.load(std::sync::atomic::Ordering::Relaxed), 1);
}

#[test]
fn a_sequence_reaches_the_controller_carrying_the_callers_own_policies() {
    let harness = Harness::with_input();
    let operation = OperationContext::new();
    let session = opened_with_input(&harness, &operation);
    let source = stamp(&session, &operation);
    let request = InputRequest::new(
        session.target(),
        chord(),
        DeliveryPlan::require(InputDelivery::System),
    )
    .with_focus(FocusPolicy::RequireFocused)
    .with_pointer_geometry(PointerGeometry::require_unchanged_since(source));

    let receipt = session.send_input(&request, &operation).expect("executed");

    assert!(receipt.is_complete());
    let admitted = harness.input().admitted();
    assert_eq!(admitted.len(), 1);
    assert_eq!(admitted[0].focus, FocusPolicy::RequireFocused);
    assert_eq!(
        admitted[0].geometry.policy(),
        GeometryPolicy::RequireUnchanged
    );
    assert_eq!(
        admitted[0].geometry.source(),
        Some(source),
        "the exact frame identity travels whole"
    );
    assert_eq!(admitted[0].routes, [InputDelivery::System]);
}

#[test]
fn a_request_addressed_to_another_target_is_refused_before_any_event() {
    let harness = Harness::with_input();
    let operation = OperationContext::new();
    let session = opened_with_input(&harness, &operation);
    let elsewhere = harness
        .capture
        .add_target(
            "second",
            mado_pilot_runtime::TargetCapability::unclassified(),
        )
        .expect("issued");

    let error = session
        .send_input(&typing(elsewhere), &operation)
        .expect_err("this session captures one target and delivers to that one");

    assert_eq!(error.status(), Status::InvalidArgument);
    assert!(harness.input().admitted().is_empty());
}

#[test]
fn a_source_frame_from_another_stream_is_refused_before_any_event() {
    let harness = Harness::with_input();
    let elsewhere = Harness::with_input();
    let operation = OperationContext::new();
    let session = opened_with_input(&harness, &operation);
    let other_session = opened_with_input(&elsewhere, &operation);
    let foreign_stamp = stamp(&other_session, &operation);

    let error = session
        .send_input(
            &InputRequest::new(
                session.target(),
                chord(),
                DeliveryPlan::require(InputDelivery::System),
            )
            .with_pointer_geometry(PointerGeometry::require_unchanged_since(foreign_stamp)),
            &operation,
        )
        .expect_err("another stream's frame describes another geometry");

    assert_eq!(error.status(), Status::InvalidArgument);
    assert!(harness.input().admitted().is_empty());
}

#[test]
fn a_partially_submitted_sequence_is_never_retried_through_another_route() {
    let harness = Harness::with_input();
    harness.input().set_behavior(Behavior::FailAfter {
        submitted: 2,
        fault: InputFault::SubmissionFailed,
    });
    let operation = OperationContext::new();
    let session = opened_with_input(&harness, &operation);

    let receipt = session
        .send_input(
            &InputRequest::new(
                session.target(),
                chord(),
                DeliveryPlan::ordered(vec![InputDelivery::System, InputDelivery::WindowMessage])
                    .expect("valid"),
            ),
            &operation,
        )
        .expect("an admitted sequence answers with a receipt");

    assert_eq!(receipt.outcome(), SequenceOutcome::Partial);
    assert_eq!(receipt.submitted(), 2);
    assert_eq!(
        harness.input().admitted().len(),
        1,
        "native submission cannot be taken back, so nothing is sent twice"
    );
    assert_eq!(harness.input().submitted_events().len(), 2);
}

#[test]
fn a_mechanism_the_caller_did_not_permit_is_never_substituted() {
    let harness = Harness::with_input();
    let operation = OperationContext::new();
    let session = opened_with_input(&harness, &operation);

    let receipt = session
        .send_input(
            &InputRequest::new(
                session.target(),
                chord(),
                DeliveryPlan::require(InputDelivery::WindowMessage),
            ),
            &operation,
        )
        .expect("route refusal is receipt evidence");

    assert_eq!(receipt.outcome(), SequenceOutcome::Unexecuted);
    assert_eq!(receipt.fault(), Some(InputFault::UnsupportedCombination));
    assert_eq!(receipt.attempts().len(), 1);
    assert_eq!(receipt.attempts()[0].route(), InputDelivery::WindowMessage);
    assert!(
        harness.input().submitted_events().is_empty(),
        "system input is not substituted for target-directed window messages"
    );
}

#[test]
fn a_mechanism_that_needs_focus_is_refused_when_the_caller_preserves_it() {
    let (issuer, capture) = support::controlled_capture();
    let input = Arc::new(ControlledInput::with_capability(
        capture.target(),
        InputCapability::none()
            .with_pair(
                InputOperationKind::Keyboard,
                InputDelivery::System,
                CapabilitySupport::Supported,
                SubmissionEvidence::SystemInputAdmission,
            )
            .with_focus_required(InputOperationKind::Keyboard, InputDelivery::System),
    ));
    let engine = support::wire(
        &issuer,
        Arc::clone(&capture) as Arc<dyn CaptureProvider>,
        Some(Arc::clone(&input) as Arc<dyn InputProvider>),
        None,
    )
    .expect("one provider");
    let operation = OperationContext::new();
    let session = engine
        .open_session(
            capture.target(),
            &SessionRequest::new().requesting_input(InputOpenRequest::new()),
            &operation,
        )
        .expect("opened");

    let preserved = session
        .send_input(&typing(session.target()), &operation)
        .expect("focus-policy refusal is receipt evidence");
    assert_eq!(preserved.outcome(), SequenceOutcome::Unexecuted);
    assert_eq!(preserved.fault(), Some(InputFault::FocusRequired));
    assert_eq!(preserved.attempts().len(), 1);

    let permitted = session
        .send_input(
            &typing(session.target()).with_focus(FocusPolicy::ActivateIfRequired),
            &operation,
        )
        .expect("the caller permitted the activation this mechanism needs");
    assert!(permitted.is_complete());
    assert_eq!(input.admitted().len(), 1, "the refused one never admitted");
}

#[test]
fn a_geometry_change_stops_the_sequence_without_submitting_the_affected_event() {
    let harness = Harness::with_input();
    harness.input().set_behavior(Behavior::FailAfter {
        submitted: 1,
        fault: InputFault::GeometryChanged,
    });
    let operation = OperationContext::new();
    let session = opened_with_input(&harness, &operation);
    let source = stamp(&session, &operation);

    let receipt = session
        .send_input(
            &InputRequest::new(
                session.target(),
                chord(),
                DeliveryPlan::require(InputDelivery::System),
            )
            .with_pointer_geometry(PointerGeometry::require_unchanged_since(source)),
            &operation,
        )
        .expect("an admitted sequence answers with a receipt");

    assert_eq!(receipt.outcome(), SequenceOutcome::Partial);
    assert_eq!(receipt.submitted(), 1);
    assert_eq!(receipt.fault(), Some(InputFault::GeometryChanged));
    assert_eq!(
        harness.input().submitted_events().len(),
        1,
        "the event the changed geometry would have mislocated was not submitted"
    );
}

#[test]
fn a_stopped_sequence_releases_only_what_it_pressed_and_reports_the_counts() {
    let harness = Harness::with_input();
    harness.input().set_behavior(Behavior::FailAfter {
        submitted: 2,
        fault: InputFault::SubmissionFailed,
    });
    harness.input().set_cleanup(Cleanup::Partial(1));
    let operation = OperationContext::new();
    let session = opened_with_input(&harness, &operation);

    let receipt = session
        .send_input(
            &InputRequest::new(
                session.target(),
                chord(),
                DeliveryPlan::require(InputDelivery::System),
            ),
            &operation,
        )
        .expect("executed");

    assert_eq!(receipt.cleanup_owed(), 2);
    assert_eq!(receipt.cleanup_released(), 1);
    assert!(receipt.cleanup().may_leave_state_held());
    assert_eq!(
        harness.input().released(),
        vec![PressedState::Key(Key::Character('c'))],
        "cleanup releases this sequence's own state, in reverse order of pressing"
    );
}

#[test]
fn a_target_the_adapter_reports_as_lost_is_reported_as_lost() {
    let harness = Harness::with_input();
    harness
        .input()
        .set_behavior(Behavior::Unexecuted(InputFault::TargetLost));
    let operation = OperationContext::new();
    let session = opened_with_input(&harness, &operation);

    let receipt = session
        .send_input(&typing(session.target()), &operation)
        .expect("an admitted sequence answers with a receipt, including this one");

    assert_eq!(
        receipt.outcome(),
        SequenceOutcome::Unexecuted,
        "nothing reached a target that is gone"
    );
    assert_eq!(receipt.fault(), Some(InputFault::TargetLost));
    assert_eq!(
        InputFault::TargetLost.status(),
        Status::TargetLost,
        "a caller that branches on the status reads the same thing"
    );
}

#[test]
fn a_session_whose_capture_ended_submits_nothing() {
    let harness = Harness::with_input();
    let operation = OperationContext::new();
    let session = opened_with_input(&harness, &operation);

    harness.capture.lose(harness.capture.target());

    let error = session
        .send_input(&typing(session.target()), &operation)
        .expect_err("this session is over");

    assert_eq!(error.status(), Status::Closed);
    assert!(harness.input().admitted().is_empty());
}

#[test]
fn a_receipt_with_native_effect_survives_an_operation_that_lost_its_race() {
    let (issuer, capture) = support::controlled_capture();
    let clock = Arc::new(ManualClock::new());
    let late = Arc::new(LateAnswer::new(
        capture.target(),
        Arc::clone(&clock),
        Duration::from_millis(50),
        Answer::Partial(2),
    ));
    let engine = support::wire(
        &issuer,
        Arc::clone(&capture) as Arc<dyn CaptureProvider>,
        Some(late as Arc<dyn InputProvider>),
        None,
    )
    .expect("one provider");
    let operation = OperationContext::new()
        .with_clock(clock)
        .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(10)));
    let session = engine
        .open_session(
            capture.target(),
            &SessionRequest::new().requesting_input(InputOpenRequest::new()),
            &operation,
        )
        .expect("opened before the clock moved");

    let receipt = session
        .send_input(&typing(session.target()), &operation)
        .expect("an account of what reached the target is never discarded");

    assert_eq!(receipt.outcome(), SequenceOutcome::Partial);
    assert_eq!(receipt.submitted(), 2);
}

#[test]
fn a_receipt_without_native_effect_loses_to_the_operation_that_expired() {
    let (issuer, capture) = support::controlled_capture();
    let clock = Arc::new(ManualClock::new());
    let late = Arc::new(LateAnswer::new(
        capture.target(),
        Arc::clone(&clock),
        Duration::from_millis(50),
        Answer::Unexecuted,
    ));
    let engine = support::wire(
        &issuer,
        Arc::clone(&capture) as Arc<dyn CaptureProvider>,
        Some(late as Arc<dyn InputProvider>),
        None,
    )
    .expect("one provider");
    let operation = OperationContext::new()
        .with_clock(clock)
        .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(10)));
    let session = engine
        .open_session(
            capture.target(),
            &SessionRequest::new().requesting_input(InputOpenRequest::new()),
            &operation,
        )
        .expect("opened before the clock moved");

    let error = session
        .send_input(&typing(session.target()), &operation)
        .expect_err("nothing reached the target, so the late answer is just late");

    assert_eq!(error.status(), Status::DeadlineExceeded);
}

#[test]
fn a_cancellation_that_races_the_final_event_publishes_one_consistent_receipt() {
    let harness = Harness::with_input();
    let token = CancellationToken::new();
    let operation = OperationContext::new().with_cancellation(token.clone());
    let session = opened_with_input(&harness, &operation);
    // Wait until the first irreversible event has actually reached the double.
    // Sleeping in a separate canceller thread can lose the intended race on a
    // loaded runner and let the entire sequence complete before cancellation.
    let sequence = InputSequence::new(vec![
        InputEvent::KeyPress(Key::Modifier(Modifier::Control)),
        InputEvent::Delay(Duration::from_millis(60)),
        InputEvent::KeyPress(Key::Character('c')),
    ])
    .expect("valid");

    let receipt = thread::scope(|scope| {
        let worker = scope.spawn(|| {
            session
                .send_input(
                    &InputRequest::new(
                        session.target(),
                        sequence,
                        DeliveryPlan::require(InputDelivery::System),
                    ),
                    &operation,
                )
                .expect("an admitted sequence answers with a receipt")
        });
        wait_until("the modifier to reach the native route", || {
            !harness.input().submitted_events().is_empty()
        });
        token.cancel();
        worker.join().expect("the sequence finished")
    });

    assert_eq!(receipt.outcome(), SequenceOutcome::Partial);
    assert_eq!(
        receipt.submitted(),
        harness.input().submitted_events().len(),
        "the receipt counts exactly the events submitted to the native route"
    );
    assert_eq!(receipt.fault(), Some(InputFault::Cancelled));
    assert_eq!(
        receipt.cleanup_owed(),
        1,
        "the modifier the sequence pressed is what cleanup owns"
    );
}

#[test]
fn one_sequence_at_a_time_holds_the_controller() {
    let harness = Harness::with_input();
    let operation = OperationContext::new();
    let session = opened_with_input(&harness, &operation);
    let holding = InputSequence::new(vec![
        InputEvent::KeyPress(Key::Enter),
        InputEvent::Delay(Duration::from_millis(120)),
    ])
    .expect("valid");

    thread::scope(|scope| {
        let holder = scope.spawn(|| {
            session
                .send_input(
                    &InputRequest::new(
                        session.target(),
                        holding,
                        DeliveryPlan::require(InputDelivery::System),
                    ),
                    &OperationContext::new(),
                )
                .expect("the first sequence runs")
        });

        while harness.input().executing() == 0 {
            thread::yield_now();
        }

        // A waiter whose own deadline passes delivers nothing and reports that,
        // rather than accumulating behind the sequence in flight.
        let waiting = OperationContext::new()
            .with_timeout(Duration::from_millis(20))
            .expect("representable");
        let refused = session
            .send_input(&typing(session.target()), &waiting)
            .expect_err("the controller was held");

        assert_eq!(refused.status(), Status::DeadlineExceeded);
        assert!(holder.join().expect("holder finished").is_complete());
    });

    assert_eq!(
        harness.input().admitted().len(),
        1,
        "the waiter never entered the controller"
    );
}

#[test]
fn one_close_finishes_both_lifecycles_and_repeats_neither() {
    let harness = Harness::with_input();
    let operation = OperationContext::new();
    let session = opened_with_input(&harness, &operation);

    session.close(&operation).expect("closed");
    assert!(session.is_closed());
    session.close(&operation).expect("already closed");
    assert!(session.is_closed());

    let error = session
        .send_input(&typing(session.target()), &operation)
        .expect_err("a closed session delivers nothing");

    assert_eq!(error.status(), Status::Closed);
    assert!(harness.input().submitted_events().is_empty());
}

#[test]
fn a_close_that_lost_its_race_leaves_both_sides_closing_and_a_later_close_continues() {
    let harness = Harness::with_input();
    let operation = OperationContext::new();
    let session = opened_with_input(&harness, &operation);

    let error = session
        .close(&expired())
        .expect_err("the deadline wins the close");

    assert_eq!(error.status(), Status::DeadlineExceeded);
    assert!(
        !session.is_closed(),
        "neither drain finished, so the lifecycle is not over"
    );

    session
        .close(&OperationContext::new())
        .expect("a later close continues the drain");
    assert!(session.is_closed());
}

/// An input adapter that reports a provider the capture side does not.
#[derive(Debug)]
struct ForeignInput {
    inner: ControlledInput,
}

impl InputProvider for ForeignInput {
    fn provider(&self) -> ProviderId {
        ProviderId::new("elsewhere")
    }

    fn describe(
        &self,
        target: TargetId,
        operation: &OperationContext,
    ) -> mado_pilot_runtime::Result<mado_pilot_runtime::InputDescriptor> {
        self.inner.describe(target, operation)
    }

    fn open(
        &self,
        target: TargetId,
        request: &InputOpenRequest,
        operation: &OperationContext,
    ) -> mado_pilot_runtime::Result<Arc<dyn mado_pilot_runtime::InputController>> {
        self.inner.open(target, request, operation)
    }
}

#[test]
fn an_engine_with_an_input_adapter_describes_what_that_adapter_reports() {
    let harness = Harness::with_input();
    let operation = OperationContext::new();

    assert!(harness.engine.delivers_input());
    let descriptor = harness
        .engine
        .describe_input(harness.capture.target(), &operation)
        .expect("the input adapter describes its own target");

    assert!(descriptor.is_available());
    assert_eq!(
        descriptor.capability(),
        harness.input().capability(),
        "the engine reports what the adapter reports rather than a summary of it"
    );
    assert_eq!(descriptor.target(), harness.capture.target());
}

#[test]
fn a_target_this_engine_issued_but_the_input_adapter_does_not_drive_is_refused_by_it() {
    let harness = Harness::with_input();
    // Issued by this engine and this provider, so the engine-level identity check
    // accepts it and the refusal has to come from the adapter itself.
    let elsewhere = harness
        .capture
        .add_target(
            "second",
            mado_pilot_runtime::TargetCapability::unclassified(),
        )
        .expect("issued");

    let error = harness
        .engine
        .describe_input(elsewhere, &OperationContext::new())
        .expect_err("this adapter drives one target");

    assert_eq!(error.status(), Status::InvalidArgument);
}

#[test]
fn a_commit_that_refuses_after_both_adapters_opened_releases_both() {
    let (issuer, capture) = support::controlled_capture();
    let clock = Arc::new(ManualClock::new());
    let counting = Arc::new(CountingCapture::new(Arc::clone(&capture)));
    let capture_closes = counting.closes();
    let input = Arc::new(support::OpenInputThenExpire::new(
        Arc::new(ControlledInput::new(capture.target())),
        Arc::clone(&clock),
        Duration::from_millis(50),
    ));
    let controller_closes = input.closes();
    let engine = support::wire(
        &issuer,
        counting as Arc<dyn CaptureProvider>,
        Some(input as Arc<dyn InputProvider>),
        None,
    )
    .expect("one provider");
    let operation = OperationContext::new()
        .with_clock(clock)
        .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(10)));

    let error = engine
        .open_session(
            capture.target(),
            &SessionRequest::new().requesting_input(InputOpenRequest::new()),
            &operation,
        )
        .expect_err("the engine's own arbitration refuses after both adapters committed");

    assert_eq!(error.status(), Status::DeadlineExceeded);
    assert_eq!(
        controller_closes.load(std::sync::atomic::Ordering::Relaxed),
        1,
        "a controller that exists is closed rather than dropped, because dropping one does not close it"
    );
    assert_eq!(
        capture_closes.load(std::sync::atomic::Ordering::Relaxed),
        1,
        "and so is the capture session opened beside it"
    );
}

#[test]
fn an_operation_that_expires_during_open_releases_its_capture_and_yields_no_session() {
    let (issuer, capture) = support::controlled_capture();
    let clock = Arc::new(ManualClock::new());
    // Capture opens and then the caller's operation loses, so everything after
    // the capture commit runs under an operation that is already over.
    let expiring = Arc::new(support::OpenThenExpire::new(
        Arc::clone(&capture),
        Arc::clone(&clock),
        Duration::from_millis(50),
    ));
    let capture_closes = expiring.closes();
    let input = Arc::new(ControlledInput::new(capture.target()));
    let engine = support::wire(
        &issuer,
        expiring as Arc<dyn CaptureProvider>,
        Some(input as Arc<dyn InputProvider>),
        None,
    )
    .expect("one provider");
    let operation = OperationContext::new()
        .with_clock(clock)
        .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(10)));

    let error = engine
        .open_session(
            capture.target(),
            &SessionRequest::new().requesting_input(InputOpenRequest::new()),
            &operation,
        )
        .expect_err("an operation that is over cannot be answered with a session at all");

    // Deliberately an outcome assertion rather than a branch assertion: whether
    // the input open observed the interruption first or the engine's own commit
    // did, the caller must get the interruption and the capture must be closed.
    // The two are indistinguishable from here, and that is the point — neither
    // may quietly hand back a capture-only session instead.
    assert_eq!(error.status(), Status::DeadlineExceeded);
    assert_eq!(
        capture_closes.load(std::sync::atomic::Ordering::Relaxed),
        1,
        "optional input does not turn an expired operation into a capture-only success"
    );
}
