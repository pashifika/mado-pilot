//! Controller rules, checked against a scripted driver rather than the desktop.
//!
//! Nothing here posts an event. The cases that matter are the ones a live host
//! cannot be made to produce on cue — a platform that refuses after two events, a
//! release that will not go out, an authorization revoked mid-sequence — so the
//! driver seam is what they are written against.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use mado_pilot_core::{
    CancellationToken, CapabilitySupport, Clock, CoordinateSpace, IdentityIssuer, InputCapability,
    InputDelivery, InputOperationKind, Lifecycle, MonotonicInstant, OperationContext, Point,
    ProviderId, Status, TargetId, TargetKind,
};
use mado_pilot_input::{
    CleanupBudget, CleanupState, DeliveryPlan, FocusPolicy, InputController, InputDescriptor,
    InputEvent, InputFault, InputRequest, InputSequence, Key, Modifier, PointerButton,
    PointerGeometry, PressedState, SequenceOutcome,
};

use super::{
    DriverState, InputDriver, MacosInputController, SubmissionFailure, SystemButtonState,
    SystemKeyState, input_capability,
};

fn target() -> TargetId {
    IdentityIssuer::new()
        .issue_target(ProviderId::new("macos"))
        .expect("issued")
}

fn window_descriptor(target: TargetId) -> InputDescriptor {
    InputDescriptor::new(target, input_capability(TargetKind::Window))
}

fn point() -> Point {
    Point::new(CoordinateSpace::CapturePixels, 12.0, 8.0).expect("valid")
}

fn click() -> InputSequence {
    InputSequence::new(vec![
        InputEvent::PointerMove(point()),
        InputEvent::PointerPress(PointerButton::Primary),
        InputEvent::PointerRelease(PointerButton::Primary),
    ])
    .expect("valid")
}

/// A chord that leaves a modifier and a button held when it stops early.
fn chord() -> InputSequence {
    InputSequence::new(vec![
        InputEvent::KeyPress(Key::Modifier(Modifier::Meta)),
        InputEvent::PointerMove(point()),
        InputEvent::PointerPress(PointerButton::Primary),
        InputEvent::KeyPress(Key::Character('c')),
        InputEvent::KeyRelease(Key::Character('c')),
    ])
    .expect("valid")
}

fn system(target: TargetId, sequence: InputSequence) -> InputRequest {
    InputRequest::new(
        target,
        sequence,
        DeliveryPlan::require(InputDelivery::System),
    )
    .with_focus(FocusPolicy::RequireFocused)
}

/// What a scripted driver was asked to do, in order.
#[derive(Debug, Clone, PartialEq)]
enum Action {
    Preflight(InputDelivery),
    Submit(InputDelivery),
    Release(PressedState),
}

/// A driver whose every outcome the test writes.
#[derive(Debug, Default)]
struct ScriptedDriver {
    log: Mutex<Vec<Action>>,
    preflight: Mutex<Option<InputFault>>,
    /// Fails the delivery at this zero-based index with this failure.
    fail_delivery_at: Mutex<Option<(usize, SubmissionFailure)>>,
    /// Refuses every release from this zero-based index onward.
    refuse_release_from: Mutex<Option<usize>>,
    /// Moves the test's clock forward at this delivery index, so an operation can
    /// expire part-way through a sequence rather than before it starts.
    advance_at: Mutex<Option<(usize, Arc<ManualClock>, Duration)>>,
    submitted: Mutex<usize>,
    released: Mutex<usize>,
}

impl ScriptedDriver {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn failing_at(index: usize, failure: SubmissionFailure) -> Arc<Self> {
        let driver = Self::default();
        *driver.fail_delivery_at.lock().expect("uncontended") = Some((index, failure));
        Arc::new(driver)
    }

    fn refusing_preflight(fault: InputFault) -> Arc<Self> {
        let driver = Self::default();
        *driver.preflight.lock().expect("uncontended") = Some(fault);
        Arc::new(driver)
    }

    fn refusing_releases_from(mut self: Arc<Self>, index: usize) -> Arc<Self> {
        let driver = Arc::get_mut(&mut self).expect("uniquely owned");
        *driver.refuse_release_from.lock().expect("uncontended") = Some(index);
        self
    }

    fn advancing_at(
        mut self: Arc<Self>,
        index: usize,
        clock: Arc<ManualClock>,
        step: Duration,
    ) -> Arc<Self> {
        let driver = Arc::get_mut(&mut self).expect("uniquely owned");
        *driver.advance_at.lock().expect("uncontended") = Some((index, clock, step));
        self
    }

    fn actions(&self) -> Vec<Action> {
        self.log.lock().expect("uncontended").clone()
    }

    fn record(&self, action: Action) {
        self.log.lock().expect("uncontended").push(action);
    }
}

impl InputDriver for ScriptedDriver {
    fn preflight(
        &self,
        delivery: InputDelivery,
        _focus: FocusPolicy,
        _operation: &OperationContext,
    ) -> Result<(), InputFault> {
        self.record(Action::Preflight(delivery));
        match *self.preflight.lock().expect("uncontended") {
            Some(fault) => Err(fault),
            None => Ok(()),
        }
    }

    fn submit(
        &self,
        delivery: InputDelivery,
        _focus: FocusPolicy,
        _event: &InputEvent,
        _geometry: PointerGeometry,
        state: &mut DriverState,
        _operation: &OperationContext,
    ) -> Result<(), SubmissionFailure> {
        self.record(Action::Submit(delivery));
        let mut submitted = self.submitted.lock().expect("uncontended");
        let index = *submitted;
        *submitted += 1;
        drop(submitted);
        if let Some((at, clock, step)) = self.advance_at.lock().expect("uncontended").as_ref()
            && *at == index
        {
            clock.advance(*step);
        }
        if let Some((at, failure)) = *self.fail_delivery_at.lock().expect("uncontended")
            && at == index
        {
            return Err(failure);
        }
        // The scripted driver records pressed state the way the native one does,
        // so cleanup has something to release.
        match _event {
            InputEvent::PointerPress(button) => state.buttons.push(SystemButtonState {
                logical: *button,
                native: 0,
            }),
            InputEvent::KeyPress(key) => state.keys.push(SystemKeyState {
                logical: *key,
                key_code: 0,
            }),
            _ => {}
        }
        Ok(())
    }

    fn release(
        &self,
        _delivery: InputDelivery,
        pressed: PressedState,
        _state: &mut DriverState,
        operation: &OperationContext,
    ) -> Result<(), InputFault> {
        self.record(Action::Release(pressed));
        if let Some(interruption) = operation.interruption() {
            return Err(InputFault::from(interruption));
        }
        let mut released = self.released.lock().expect("uncontended");
        let index = *released;
        *released += 1;
        drop(released);
        match *self.refuse_release_from.lock().expect("uncontended") {
            Some(from) if index >= from => Err(InputFault::SubmissionFailed),
            _ => Ok(()),
        }
    }
}

/// A clock the test moves by hand.
#[derive(Debug, Default)]
struct ManualClock {
    elapsed: Mutex<Duration>,
}

impl ManualClock {
    fn advance(&self, step: Duration) {
        let mut elapsed = self.elapsed.lock().expect("uncontended");
        *elapsed = elapsed.saturating_add(step);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> MonotonicInstant {
        MonotonicInstant::from_origin(*self.elapsed.lock().expect("uncontended"))
    }
}

#[test]
fn a_supported_system_sequence_reports_exact_counts_and_the_system_mechanism() {
    let target = target();
    let driver = ScriptedDriver::new();
    let controller =
        MacosInputController::with_driver(window_descriptor(target), Arc::clone(&driver) as _);

    let receipt = controller
        .execute(&system(target, click()), &OperationContext::new())
        .expect("an admitted sequence produces a receipt");

    assert_eq!(receipt.outcome(), SequenceOutcome::Complete);
    assert_eq!(receipt.submitted(), 3);
    assert_eq!(receipt.last_submitted(), Some(2));
    assert_eq!(receipt.selected_route(), Some(InputDelivery::System));
    assert_eq!(receipt.attempts().len(), 1);
    assert_eq!(receipt.attempts()[0].route(), InputDelivery::System);
    assert!(!receipt.used_fallback());
    assert_eq!(receipt.cleanup(), CleanupState::NotNeeded);
}

#[test]
fn a_window_message_request_is_refused_before_any_event_and_never_substituted() {
    let target = target();
    let driver = ScriptedDriver::new();
    let controller =
        MacosInputController::with_driver(window_descriptor(target), Arc::clone(&driver) as _);
    let request = InputRequest::new(
        target,
        click(),
        DeliveryPlan::require(InputDelivery::WindowMessage),
    )
    .with_focus(FocusPolicy::RequireFocused);

    let receipt = controller
        .execute(&request, &OperationContext::new())
        .expect("route refusal is receipt evidence");

    assert_eq!(receipt.outcome(), SequenceOutcome::Unexecuted);
    assert_eq!(receipt.selected_route(), None);
    assert_eq!(receipt.submitted(), 0);
    assert_eq!(receipt.fault(), Some(InputFault::UnsupportedCombination));
    assert_eq!(receipt.attempts().len(), 1);
    assert_eq!(receipt.attempts()[0].route(), InputDelivery::WindowMessage);
    assert_eq!(
        receipt.attempts()[0].fault(),
        Some(InputFault::UnsupportedCombination)
    );
    assert!(
        driver.actions().is_empty(),
        "unsupported delivery is recorded without reaching the native driver"
    );
}

#[test]
fn a_permitted_fallback_records_each_route_and_only_drives_the_supported_one() {
    // The receipt retains every route the caller asked the Adapter to visit,
    // including the unsupported route refused before the native driver. Only
    // the supported system route reaches native preflight and submission.
    let target = target();
    let driver = ScriptedDriver::new();
    let controller =
        MacosInputController::with_driver(window_descriptor(target), Arc::clone(&driver) as _);
    let request = InputRequest::new(
        target,
        click(),
        DeliveryPlan::ordered(vec![InputDelivery::WindowMessage, InputDelivery::System])
            .expect("valid"),
    )
    .with_focus(FocusPolicy::RequireFocused);

    let receipt = controller
        .execute(&request, &OperationContext::new())
        .expect("the permitted fallback is available");

    assert_eq!(receipt.selected_route(), Some(InputDelivery::System));
    assert_eq!(receipt.attempts().len(), 2);
    assert_eq!(receipt.attempts()[0].route(), InputDelivery::WindowMessage);
    assert_eq!(
        receipt.attempts()[0].fault(),
        Some(InputFault::UnsupportedCombination)
    );
    assert_eq!(receipt.attempts()[1].route(), InputDelivery::System);
    assert_eq!(receipt.attempts()[1].outcome(), SequenceOutcome::Complete);
    assert!(
        receipt.used_fallback(),
        "the selected route follows one recorded refusal"
    );
    assert_eq!(
        driver.actions(),
        vec![
            Action::Preflight(InputDelivery::System),
            Action::Submit(InputDelivery::System),
            Action::Submit(InputDelivery::System),
            Action::Submit(InputDelivery::System),
        ],
        "no WindowMessage preflight reached the driver"
    );
}
#[test]
fn a_preserving_request_against_a_focus_requiring_window_is_refused() {
    let target = target();
    let driver = ScriptedDriver::new();
    let controller =
        MacosInputController::with_driver(window_descriptor(target), Arc::clone(&driver) as _);
    let preserving = InputRequest::new(
        target,
        click(),
        DeliveryPlan::require(InputDelivery::System),
    );

    let receipt = controller
        .execute(&preserving, &OperationContext::new())
        .expect("focus refusal is receipt evidence");

    assert_eq!(receipt.outcome(), SequenceOutcome::Unexecuted);
    assert_eq!(receipt.fault(), Some(InputFault::FocusRequired));
    assert_eq!(receipt.attempts().len(), 1);
    assert_eq!(receipt.attempts()[0].route(), InputDelivery::System);
    assert!(driver.actions().is_empty());
}

#[test]
fn a_display_target_accepts_a_pointer_sequence_without_any_focus_policy() {
    let target = target();
    let descriptor = InputDescriptor::new(target, input_capability(TargetKind::Display));
    let driver = ScriptedDriver::new();
    let controller = MacosInputController::with_driver(descriptor, Arc::clone(&driver) as _);

    let receipt = controller
        .execute(
            &InputRequest::new(
                target,
                click(),
                DeliveryPlan::require(InputDelivery::System),
            ),
            &OperationContext::new(),
        )
        .expect("a display needs nothing focused");

    assert_eq!(receipt.outcome(), SequenceOutcome::Complete);
}

#[test]
fn a_keystroke_to_a_display_is_refused_because_nothing_there_can_receive_it() {
    let target = target();
    let descriptor = InputDescriptor::new(target, input_capability(TargetKind::Display));
    let controller = MacosInputController::with_driver(descriptor, ScriptedDriver::new() as _);

    let receipt = controller
        .execute(
            &InputRequest::new(
                target,
                InputSequence::new(vec![InputEvent::KeyPress(Key::Enter)]).expect("valid"),
                DeliveryPlan::require(InputDelivery::System),
            ),
            &OperationContext::new(),
        )
        .expect("unsupported target-kind delivery is receipt evidence");

    assert_eq!(receipt.outcome(), SequenceOutcome::Unexecuted);
    assert_eq!(receipt.fault(), Some(InputFault::UnsupportedCombination));
    assert_eq!(receipt.attempts().len(), 1);
    assert_eq!(receipt.attempts()[0].route(), InputDelivery::System);
}

#[test]
fn a_stop_part_way_releases_exactly_what_that_sequence_had_pressed() {
    let target = target();
    // Fails on the fifth event, after a modifier, a move, a button, and a key.
    let driver = ScriptedDriver::failing_at(
        4,
        SubmissionFailure::before_event(InputFault::NotAuthorized),
    );
    let controller =
        MacosInputController::with_driver(window_descriptor(target), Arc::clone(&driver) as _);

    let receipt = controller
        .execute(&system(target, chord()), &OperationContext::new())
        .expect("a receipt, not an error");

    assert_eq!(receipt.outcome(), SequenceOutcome::Partial);
    assert_eq!(receipt.submitted(), 4);
    assert_eq!(receipt.fault(), Some(InputFault::NotAuthorized));
    assert_eq!(receipt.cleanup(), CleanupState::Complete);
    assert_eq!(receipt.cleanup_owed(), 3);
    assert_eq!(receipt.cleanup_released(), 3);
    assert_eq!(
        driver
            .actions()
            .into_iter()
            .filter_map(|action| match action {
                Action::Release(pressed) => Some(pressed),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![
            PressedState::Key(Key::Character('c')),
            PressedState::Button(PointerButton::Primary),
            PressedState::Key(Key::Modifier(Modifier::Meta)),
        ],
        "newest first, so the modifier this sequence pressed first is released last"
    );
}

#[test]
fn a_release_the_platform_refuses_is_incomplete_rather_than_exhausted() {
    let target = target();
    let driver = ScriptedDriver::failing_at(
        4,
        SubmissionFailure::before_event(InputFault::SubmissionFailed),
    )
    .refusing_releases_from(1);
    let controller =
        MacosInputController::with_driver(window_descriptor(target), Arc::clone(&driver) as _);

    let receipt = controller
        .execute(&system(target, chord()), &OperationContext::new())
        .expect("a receipt");

    assert_eq!(receipt.cleanup(), CleanupState::Incomplete);
    assert_eq!(receipt.cleanup_released(), 1);
    assert_eq!(receipt.cleanup_owed(), 3);
    assert!(receipt.cleanup().may_leave_state_held());
}

#[test]
fn a_cleanup_that_runs_out_of_its_own_bound_is_exhausted_and_says_so() {
    let target = target();
    let driver = ScriptedDriver::failing_at(
        4,
        SubmissionFailure::before_event(InputFault::SubmissionFailed),
    );
    let controller =
        MacosInputController::with_driver(window_descriptor(target), Arc::clone(&driver) as _);
    let request = system(target, chord())
        .with_cleanup_budget(CleanupBudget::at_most(1, Duration::from_millis(50)));

    let receipt = controller
        .execute(&request, &OperationContext::new())
        .expect("a receipt");

    assert_eq!(
        receipt.cleanup(),
        CleanupState::Exhausted,
        "the releases it never attempted might still work, which is not what a \
         refused release means"
    );
    assert_eq!(receipt.cleanup_released(), 1);
    assert_eq!(receipt.cleanup_owed(), 3);
}

#[test]
fn cleanup_runs_under_its_own_bound_and_not_the_interrupted_request() {
    let target = target();
    let clock = Arc::new(ManualClock::default());
    // The deadline passes on the fourth event, after a modifier and a button are
    // already held. That is the state cleanup exists for, and the request's own
    // context is expired by the time it runs.
    let driver =
        ScriptedDriver::new().advancing_at(3, Arc::clone(&clock), Duration::from_millis(50));
    let controller =
        MacosInputController::with_driver(window_descriptor(target), Arc::clone(&driver) as _);
    let operation = OperationContext::new()
        .with_clock(Arc::clone(&clock) as Arc<dyn Clock>)
        .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(10)));

    let receipt = controller
        .execute(&system(target, chord()), &operation)
        .expect("a receipt");

    assert!(
        operation.interruption().is_some(),
        "the request itself is interrupted by the time cleanup runs"
    );
    assert_eq!(receipt.outcome(), SequenceOutcome::Partial);
    assert_eq!(receipt.submitted(), 4);
    assert_eq!(receipt.fault(), Some(InputFault::DeadlineExceeded));
    assert_eq!(
        receipt.cleanup(),
        CleanupState::Complete,
        "releasing under the interrupted context would decline to release at the \
         one moment releasing matters"
    );
    assert_eq!(receipt.cleanup_released(), 3);
}

#[test]
fn a_cancellation_between_events_preserves_the_count_already_submitted() {
    let target = target();
    let token = CancellationToken::new();
    let driver = ScriptedDriver::new();
    let controller =
        MacosInputController::with_driver(window_descriptor(target), Arc::clone(&driver) as _);
    let operation = OperationContext::new().with_cancellation(token.clone());

    // Cancelling before execution stops at the first event with nothing submitted.
    token.cancel();
    let error = controller
        .execute(&system(target, chord()), &operation)
        .expect_err("an already-interrupted operation produces no receipt");

    assert_eq!(error.status(), Status::Cancelled);
    assert!(driver.actions().is_empty());
}

#[test]
fn a_native_effect_before_any_event_completed_is_partial_and_not_unexecuted() {
    // ADR 0015: a caller that retries an `Unexecuted` sequence must be able to
    // trust that nothing happened. Native effect with no completed event is
    // therefore reported as partial with a zero count.
    let target = target();
    let driver = ScriptedDriver::failing_at(
        0,
        SubmissionFailure::during_event(InputFault::SubmissionFailed),
    );
    let controller =
        MacosInputController::with_driver(window_descriptor(target), Arc::clone(&driver) as _);

    let receipt = controller
        .execute(&system(target, click()), &OperationContext::new())
        .expect("a receipt");

    assert_eq!(receipt.outcome(), SequenceOutcome::Partial);
    assert_eq!(receipt.submitted(), 0);
    assert_eq!(receipt.last_submitted(), None);
    assert_eq!(receipt.selected_route(), Some(InputDelivery::System));
}

#[test]
fn a_failure_before_the_first_event_took_effect_is_unexecuted() {
    let target = target();
    let driver = ScriptedDriver::failing_at(
        0,
        SubmissionFailure::before_event(InputFault::NotAuthorized),
    );
    let controller =
        MacosInputController::with_driver(window_descriptor(target), Arc::clone(&driver) as _);

    let receipt = controller
        .execute(&system(target, click()), &OperationContext::new())
        .expect("a receipt");

    assert_eq!(receipt.outcome(), SequenceOutcome::Unexecuted);
    assert_eq!(receipt.submitted(), 0);
    assert_eq!(receipt.fault(), Some(InputFault::NotAuthorized));
}

#[test]
fn a_target_lost_at_preflight_delivers_nothing_and_reports_the_attempt() {
    let target = target();
    let driver = ScriptedDriver::refusing_preflight(InputFault::TargetLost);
    let controller =
        MacosInputController::with_driver(window_descriptor(target), Arc::clone(&driver) as _);

    let receipt = controller
        .execute(&system(target, click()), &OperationContext::new())
        .expect("a receipt");

    assert_eq!(receipt.outcome(), SequenceOutcome::Unexecuted);
    assert_eq!(receipt.fault(), Some(InputFault::TargetLost));
    assert_eq!(receipt.attempts().len(), 1);
    assert_eq!(receipt.attempts()[0].route(), InputDelivery::System);
    assert_eq!(
        driver.actions(),
        vec![Action::Preflight(InputDelivery::System)]
    );
}

#[test]
fn an_unauthorized_preflight_delivers_nothing() {
    let target = target();
    let driver = ScriptedDriver::refusing_preflight(InputFault::NotAuthorized);
    let controller =
        MacosInputController::with_driver(window_descriptor(target), Arc::clone(&driver) as _);

    let receipt = controller
        .execute(&system(target, click()), &OperationContext::new())
        .expect("a receipt");

    assert_eq!(receipt.outcome(), SequenceOutcome::Unexecuted);
    assert_eq!(receipt.fault(), Some(InputFault::NotAuthorized));
    assert_eq!(receipt.cleanup(), CleanupState::NotNeeded);
}

#[test]
fn a_delay_observes_the_deadline_rather_than_sleeping_through_it() {
    let target = target();
    let driver = ScriptedDriver::new();
    let controller =
        MacosInputController::with_driver(window_descriptor(target), Arc::clone(&driver) as _);
    let sequence = InputSequence::new(vec![
        InputEvent::KeyPress(Key::Escape),
        InputEvent::Delay(Duration::from_secs(5)),
        InputEvent::KeyRelease(Key::Escape),
    ])
    .expect("valid");
    let operation = OperationContext::new()
        .with_timeout(Duration::from_millis(30))
        .expect("representable");

    let receipt = controller
        .execute(&system(target, sequence), &operation)
        .expect("a receipt");

    assert_eq!(receipt.outcome(), SequenceOutcome::Partial);
    assert_eq!(receipt.submitted(), 1, "the keystroke, not the delay");
    assert_eq!(receipt.fault(), Some(InputFault::DeadlineExceeded));
    assert_eq!(
        receipt.cleanup(),
        CleanupState::Complete,
        "the key the sequence pressed is released even though its deadline passed"
    );
}

#[test]
fn close_stops_admission_and_repeats_nothing() {
    let target = target();
    let driver = ScriptedDriver::new();
    let controller =
        MacosInputController::with_driver(window_descriptor(target), Arc::clone(&driver) as _);

    controller
        .close(&OperationContext::new())
        .expect("an idle controller drains");
    assert_eq!(controller.lifecycle(), Lifecycle::Closed);
    assert!(controller.is_closed());

    let error = controller
        .execute(&system(target, click()), &OperationContext::new())
        .expect_err("a closed controller admits nothing");
    assert_eq!(error.status(), Status::Closed);

    controller
        .close(&OperationContext::new())
        .expect("close is idempotent");
    assert!(driver.actions().is_empty());
}

#[test]
fn a_request_for_another_target_never_reaches_the_driver() {
    let driver = ScriptedDriver::new();
    let controller =
        MacosInputController::with_driver(window_descriptor(target()), Arc::clone(&driver) as _);

    let error = controller
        .execute(&system(target(), click()), &OperationContext::new())
        .expect_err("a foreign target is refused");

    assert_eq!(error.status(), Status::InvalidArgument);
    assert!(driver.actions().is_empty());
}

#[test]
fn the_advertised_capability_is_what_admission_decides_against() {
    let capability = input_capability(TargetKind::Window);

    for kind in InputOperationKind::ALL {
        let system = capability.pair(kind, InputDelivery::System);
        assert_eq!(
            system.permission(),
            Some(mado_pilot_core::PermissionKind::InputControl)
        );
        assert_eq!(system.support(), CapabilitySupport::Supported);
        assert_eq!(
            capability
                .pair(kind, InputDelivery::WindowMessage)
                .support(),
            CapabilitySupport::Unsupported
        );
    }
    assert!(
        !InputCapability::none().is_available(),
        "the shared vocabulary still describes a capture-only target"
    );
    for space in [
        CoordinateSpace::CapturePixels,
        CoordinateSpace::FrameNormalized,
        CoordinateSpace::TargetNormalized,
        CoordinateSpace::TargetLogical,
        CoordinateSpace::DesktopLogical,
    ] {
        assert!(
            capability
                .pair(InputOperationKind::Pointer, InputDelivery::System)
                .accepts_pointer_space(space),
            "macOS capture publishes an authoritative placement, so {space} resolves"
        );
    }
}
