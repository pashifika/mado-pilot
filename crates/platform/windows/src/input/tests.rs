use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use crate::fixture_protocol::CLASS_NAME;
use mado_pilot_capture::{
    Continuity, CpuFrameStorage, FrameDescriptor, FrameRequest, FrameStorage, PixelFormat,
    StoragePublication, StreamState,
};
use mado_pilot_core::{
    CancellationToken, CapabilitySupport, Clock, CoordinateSpace, GeometryRevision, IdentityIssuer,
    InputCapability, InputDelivery, InputOperationKind, MonotonicInstant, OperationContext,
    PixelExtent, ProviderId, Scale, StreamCursor, SubmissionEvidence, TargetKind, TargetPlacement,
    TransformSnapshot,
};
use mado_pilot_input::{
    CleanupState, DeliveryPlan, FocusPolicy, InputController, InputDescriptor, InputEvent,
    InputFault, InputReceipt, InputRequest, InputSequence, Key, Modifier, PointerButton,
    PointerGeometry, PressedState, SequenceOutcome,
};

use super::{
    DriverState, GeometryLedger, InputDriver, PendingRelease, SubmissionFailure,
    WindowsInputController, input_capability, wait_delay,
};

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
#[derive(Debug, Default)]
struct StepClock {
    milliseconds: AtomicUsize,
}

impl Clock for StepClock {
    fn now(&self) -> MonotonicInstant {
        let milliseconds = self.milliseconds.fetch_add(1, Ordering::AcqRel);
        MonotonicInstant::from_origin(Duration::from_millis(
            u64::try_from(milliseconds).expect("test tick fits u64"),
        ))
    }
}

#[derive(Debug, Default)]
struct ScriptedDriver {
    unavailable: Mutex<Option<(InputDelivery, InputFault)>>,
    fail_at: Mutex<Option<(usize, InputFault, bool, bool)>>,
    cancel_after: Mutex<Option<(usize, CancellationToken)>>,
    advance_after: Mutex<Option<(usize, Arc<ManualClock>, Duration)>>,
    attempts: AtomicUsize,
    preflights: Mutex<Vec<InputDelivery>>,
    submitted: Mutex<Vec<(InputDelivery, InputEvent)>>,
    released: Mutex<Vec<(InputDelivery, PressedState)>>,
}

impl ScriptedDriver {
    fn unavailable(self, delivery: InputDelivery, fault: InputFault) -> Self {
        *self.unavailable.lock().expect("uncontended") = Some((delivery, fault));
        self
    }

    fn fail_at(self, index: usize, fault: InputFault) -> Self {
        *self.fail_at.lock().expect("uncontended") = Some((index, fault, false, false));
        self
    }

    fn fail_during(self, index: usize, fault: InputFault) -> Self {
        *self.fail_at.lock().expect("uncontended") = Some((index, fault, true, true));
        self
    }

    fn fail_during_without_pressed_state(self, index: usize, fault: InputFault) -> Self {
        *self.fail_at.lock().expect("uncontended") = Some((index, fault, true, false));
        self
    }

    fn cancel_after(self, submitted: usize, token: CancellationToken) -> Self {
        *self.cancel_after.lock().expect("uncontended") = Some((submitted, token));
        self
    }

    fn advance_after(self, submitted: usize, clock: Arc<ManualClock>, step: Duration) -> Self {
        *self.advance_after.lock().expect("uncontended") = Some((submitted, clock, step));
        self
    }
}

impl InputDriver for ScriptedDriver {
    fn preflight(
        &self,
        delivery: InputDelivery,
        _focus: FocusPolicy,
        _operation: &OperationContext,
    ) -> Result<(), InputFault> {
        self.preflights.lock().expect("uncontended").push(delivery);
        if let Some((unavailable, fault)) = *self.unavailable.lock().expect("uncontended")
            && unavailable == delivery
        {
            return Err(fault);
        }
        Ok(())
    }

    fn submit(
        &self,
        route: InputDelivery,
        _focus: FocusPolicy,
        event: &InputEvent,
        _geometry: PointerGeometry,
        _state: &mut DriverState,
        _operation: &OperationContext,
    ) -> Result<(), SubmissionFailure> {
        let index = self.attempts.fetch_add(1, Ordering::AcqRel);
        if let Some((failure_index, fault, during_event, may_leave_pressed_state)) =
            *self.fail_at.lock().expect("uncontended")
            && failure_index == index
        {
            let failure = if during_event {
                SubmissionFailure::during_event(fault)
            } else {
                SubmissionFailure::before_event(fault)
            };
            return Err(if may_leave_pressed_state {
                failure
            } else {
                failure.without_pressed_state()
            });
        }
        self.submitted
            .lock()
            .expect("uncontended")
            .push((route, event.clone()));
        let submitted = index + 1;
        if let Some((cancel_after, token)) = &*self.cancel_after.lock().expect("uncontended")
            && *cancel_after == submitted
        {
            token.cancel();
        }
        if let Some((advance_after, clock, step)) =
            &*self.advance_after.lock().expect("uncontended")
            && *advance_after == submitted
        {
            clock.advance(*step);
        }
        Ok(())
    }

    fn release(
        &self,
        delivery: InputDelivery,
        pressed: PendingRelease,
        _state: &mut DriverState,
        _operation: &OperationContext,
    ) -> Result<(), InputFault> {
        let PendingRelease::Logical(pressed) = pressed else {
            panic!("scripted logical driver received native Unicode state");
        };
        self.released
            .lock()
            .expect("uncontended")
            .push((delivery, pressed));
        Ok(())
    }
}

fn target() -> mado_pilot_core::TargetId {
    IdentityIssuer::new()
        .issue_target(ProviderId::new("windows"))
        .expect("issued")
}

fn capability() -> InputCapability {
    let mut capability = InputCapability::none();
    for operation in InputOperationKind::ALL {
        capability = capability
            .with_pair(
                operation,
                InputDelivery::System,
                CapabilitySupport::Supported,
                SubmissionEvidence::SystemInputAdmission,
            )
            .with_focus_required(operation, InputDelivery::System)
            .with_pair(
                operation,
                InputDelivery::WindowMessage,
                CapabilitySupport::Supported,
                SubmissionEvidence::TargetQueueAdmission,
            );
    }
    for space in [
        CoordinateSpace::CapturePixels,
        CoordinateSpace::FrameNormalized,
        CoordinateSpace::TargetNormalized,
        CoordinateSpace::TargetLogical,
        CoordinateSpace::DesktopLogical,
    ] {
        capability = capability
            .with_pointer_space(InputDelivery::System, space)
            .with_pointer_space(InputDelivery::WindowMessage, space);
    }
    capability
}

fn controller(
    target: mado_pilot_core::TargetId,
    driver: Arc<ScriptedDriver>,
) -> WindowsInputController {
    WindowsInputController::with_driver(InputDescriptor::new(target, capability()), driver)
}

fn chord() -> InputSequence {
    InputSequence::new(vec![
        InputEvent::KeyPress(Key::Modifier(Modifier::Control)),
        InputEvent::KeyPress(Key::Character('c')),
        InputEvent::KeyRelease(Key::Character('c')),
        InputEvent::KeyRelease(Key::Modifier(Modifier::Control)),
    ])
    .expect("valid")
}

fn attempted_routes(receipt: &InputReceipt) -> Vec<InputDelivery> {
    receipt
        .attempts()
        .iter()
        .map(|attempt| attempt.route())
        .collect()
}

#[test]
fn target_classes_advertise_only_the_verified_route_matrix() {
    let ordinary = input_capability(TargetKind::Window, Some("OrdinaryWindow"), true);
    let fixture = input_capability(TargetKind::Window, Some(CLASS_NAME), true);
    let unavailable = input_capability(TargetKind::Window, Some("OrdinaryWindow"), false);
    let display = input_capability(TargetKind::Display, None, false);

    for operation in InputOperationKind::ALL {
        assert_eq!(
            ordinary.pair(operation, InputDelivery::System).support(),
            CapabilitySupport::Supported
        );
        assert_eq!(
            ordinary
                .pair(operation, InputDelivery::WindowMessage)
                .support(),
            CapabilitySupport::Unknown
        );
        assert_eq!(
            ordinary
                .pair(operation, InputDelivery::WindowMessage)
                .evidence(),
            Some(SubmissionEvidence::TargetQueueAdmission)
        );
        assert_eq!(
            fixture.pair(operation, InputDelivery::System).support(),
            CapabilitySupport::Supported
        );
        assert_eq!(
            fixture
                .pair(operation, InputDelivery::WindowMessage)
                .support(),
            CapabilitySupport::Supported
        );
        assert_eq!(
            fixture
                .pair(operation, InputDelivery::WindowMessage)
                .evidence(),
            Some(SubmissionEvidence::TargetProtocolAcknowledgement)
        );
        assert_eq!(
            unavailable
                .pair(operation, InputDelivery::WindowMessage)
                .support(),
            CapabilitySupport::Unsupported
        );
        assert_eq!(
            fixture
                .pair(operation, InputDelivery::ProcessDirected)
                .support(),
            CapabilitySupport::Unsupported
        );
    }
    assert_eq!(
        display
            .pair(InputOperationKind::Pointer, InputDelivery::System)
            .support(),
        CapabilitySupport::Supported
    );
    for operation in [InputOperationKind::Keyboard, InputOperationKind::Text] {
        assert_eq!(
            display.pair(operation, InputDelivery::System).support(),
            CapabilitySupport::Unsupported
        );
    }
    for operation in InputOperationKind::ALL {
        assert_eq!(
            display
                .pair(operation, InputDelivery::WindowMessage)
                .support(),
            CapabilitySupport::Unsupported
        );
        assert!(
            ordinary
                .pair(operation, InputDelivery::System)
                .focus_required()
        );
        assert!(
            fixture
                .pair(operation, InputDelivery::System)
                .focus_required()
        );
        assert!(
            !ordinary
                .pair(operation, InputDelivery::WindowMessage)
                .focus_required()
        );
    }
    assert!(
        !display
            .pair(InputOperationKind::Pointer, InputDelivery::System)
            .focus_required()
    );
    for space in [
        CoordinateSpace::CapturePixels,
        CoordinateSpace::FrameNormalized,
        CoordinateSpace::TargetNormalized,
        CoordinateSpace::TargetLogical,
        CoordinateSpace::DesktopLogical,
    ] {
        assert!(
            ordinary
                .pair(InputOperationKind::Pointer, InputDelivery::System)
                .accepts_pointer_space(space)
        );
        assert!(
            ordinary
                .pair(InputOperationKind::Pointer, InputDelivery::WindowMessage)
                .accepts_pointer_space(space)
        );
        assert!(
            fixture
                .pair(InputOperationKind::Pointer, InputDelivery::WindowMessage)
                .accepts_pointer_space(space)
        );
        assert!(
            display
                .pair(InputOperationKind::Pointer, InputDelivery::System)
                .accepts_pointer_space(space)
        );
    }
}

#[test]
fn geometry_ledger_retains_only_the_live_streams_latest_revision() {
    let ledger = GeometryLedger::default();
    let mut cursor =
        StreamCursor::new(IdentityIssuer::new().issue_stream().expect("issued stream"));
    let first = cursor
        .publish(GeometryRevision::FIRST)
        .expect("first frame");
    let first_transform = transform((0.0, 0.0), GeometryRevision::FIRST);
    ledger.record(first, first_transform);
    let later_same_revision = cursor
        .publish(GeometryRevision::FIRST)
        .expect("later frame");

    assert_eq!(ledger.source_transform(first), Some(first_transform));
    assert_eq!(ledger.source_transform(later_same_revision), None);
    assert_eq!(
        ledger.resolve_source_transform(later_same_revision),
        Err(InputFault::UnsupportedCoordinate)
    );
    ledger.record(later_same_revision, first_transform);
    assert_eq!(ledger.source_transform(first), Some(first_transform));
    assert_eq!(
        ledger.source_transform(later_same_revision),
        Some(first_transform)
    );

    let next_revision = GeometryRevision::FIRST.next().expect("next revision");
    let moved = cursor.publish(next_revision).expect("moved frame");
    let moved_transform = transform((10.0, 20.0), next_revision);
    ledger.record(moved, moved_transform);

    assert_eq!(ledger.source_transform(first), None);
    assert_eq!(
        ledger.resolve_source_transform(first),
        Err(InputFault::GeometryChanged)
    );
    assert_eq!(ledger.source_transform(moved), Some(moved_transform));
    ledger.remove(moved.stream());
    assert_eq!(ledger.source_transform(moved), None);
    assert_eq!(
        ledger.resolve_source_transform(moved),
        Err(InputFault::UnsupportedCoordinate)
    );
}

#[test]
fn a_frame_waiter_cannot_observe_a_stamp_before_its_geometry_commit() {
    let stream = IdentityIssuer::new().issue_stream().expect("issued stream");
    let state = Arc::new(StreamState::with_target_extent(stream));
    let ledger = Arc::new(GeometryLedger::default());
    let descriptor =
        FrameDescriptor::packed(PixelExtent::new(4, 4), PixelFormat::Bgra8).expect("descriptor");
    let storage: Arc<dyn FrameStorage> = Arc::new(
        CpuFrameStorage::new(
            descriptor,
            vec![0; descriptor.byte_len()].into_boxed_slice(),
        )
        .expect("storage"),
    );
    let placement = TargetPlacement::new(
        (10.0, 20.0),
        (4.0, 4.0),
        Scale::new(1.0, 1.0).expect("scale"),
    )
    .expect("placement");
    let publication = StoragePublication {
        captured_at: MonotonicInstant::ORIGIN,
        placement: Some(placement),
        storage,
        continuity: Continuity::Continuous,
    };

    let (waiter_started_tx, waiter_started_rx) = mpsc::channel();
    let (observed_tx, observed_rx) = mpsc::channel();
    let waiter_state = Arc::clone(&state);
    let waiter = thread::spawn(move || {
        waiter_started_tx.send(()).expect("started");
        let frame = waiter_state
            .frame(&FrameRequest::latest(), &OperationContext::new())
            .expect("frame becomes observable");
        observed_tx.send(frame).expect("observed");
    });
    waiter_started_rx.recv().expect("waiter entered");

    let (hook_tx, hook_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let publisher_state = Arc::clone(&state);
    let publisher_ledger = Arc::clone(&ledger);
    let publisher = thread::spawn(move || {
        publisher_state
            .publish_storage_with(publication, |frame| {
                publisher_ledger.publish(frame);
                hook_tx.send(frame.stamp()).expect("hook reached");
                release_rx.recv().expect("release publication");
            })
            .expect("published")
    });

    let staged_stamp = hook_rx.recv().expect("metadata commit reached");
    assert!(
        ledger.source_transform(staged_stamp).is_some(),
        "the ledger is populated at the pre-observe boundary"
    );
    assert!(
        observed_rx.try_recv().is_err(),
        "the frame remains unobservable while correlated metadata is staged"
    );
    release_tx.send(()).expect("release hook");

    let observed = observed_rx.recv().expect("waiter woke");
    assert_eq!(observed.stamp(), staged_stamp);
    assert!(ledger.source_transform(observed.stamp()).is_some());
    assert_eq!(
        publisher.join().expect("publisher joined").stamp(),
        staged_stamp
    );
    waiter.join().expect("waiter joined");
}

fn transform(origin: (f64, f64), revision: GeometryRevision) -> TransformSnapshot {
    let extent = PixelExtent::new(64, 48);
    let placement = TargetPlacement::new(
        origin,
        (64.0, 48.0),
        Scale::new(1.0, 1.0).expect("unit scale"),
    )
    .expect("placement");
    TransformSnapshot::with_target(revision, extent, placement).expect("transform")
}

#[test]
fn system_focus_policy_is_explicit_before_any_delivery() {
    let target = target();
    let driver = Arc::new(ScriptedDriver::default());
    let preserving_controller = controller(target, Arc::clone(&driver));
    let preserving = InputRequest::new(
        target,
        chord(),
        DeliveryPlan::require(InputDelivery::System),
    );

    let receipt = preserving_controller
        .execute(&preserving, &OperationContext::new())
        .expect("focus refusal is receipt evidence");
    assert_eq!(receipt.outcome(), SequenceOutcome::Unexecuted);
    assert_eq!(receipt.fault(), Some(InputFault::FocusRequired));
    assert_eq!(attempted_routes(&receipt), [InputDelivery::System]);
    assert!(driver.preflights.lock().expect("uncontended").is_empty());
    assert!(driver.submitted.lock().expect("uncontended").is_empty());

    let refusing = Arc::new(
        ScriptedDriver::default().unavailable(InputDelivery::System, InputFault::FocusRefused),
    );
    let requiring_controller = controller(target, Arc::clone(&refusing));
    let requiring = InputRequest::new(
        target,
        chord(),
        DeliveryPlan::require(InputDelivery::System),
    )
    .with_focus(FocusPolicy::RequireFocused);
    let receipt = requiring_controller
        .execute(&requiring, &OperationContext::new())
        .expect("runtime focus refusal is receipted");
    assert_eq!(receipt.outcome(), SequenceOutcome::Unexecuted);
    assert_eq!(receipt.fault(), Some(InputFault::FocusRefused));
    assert_eq!(attempted_routes(&receipt), [InputDelivery::System]);
    assert!(refusing.submitted.lock().expect("uncontended").is_empty());
}

#[test]
fn unavailable_exact_window_route_falls_back_only_when_system_was_permitted() {
    let target = target();
    let driver = Arc::new(
        ScriptedDriver::default()
            .unavailable(InputDelivery::WindowMessage, InputFault::RouteUnavailable),
    );
    let controller = controller(target, Arc::clone(&driver));
    let request = InputRequest::new(
        target,
        chord(),
        DeliveryPlan::ordered(vec![InputDelivery::WindowMessage, InputDelivery::System])
            .expect("valid"),
    )
    .with_focus(FocusPolicy::ActivateIfRequired);

    let receipt = controller
        .execute(&request, &OperationContext::new())
        .expect("executed");

    assert_eq!(receipt.outcome(), SequenceOutcome::Complete);
    assert_eq!(receipt.selected_route(), Some(InputDelivery::System));
    assert!(receipt.used_fallback());
    assert_eq!(
        attempted_routes(&receipt),
        [InputDelivery::WindowMessage, InputDelivery::System]
    );
    assert!(
        driver
            .submitted
            .lock()
            .expect("uncontended")
            .iter()
            .all(|(route, _)| *route == InputDelivery::System)
    );
}

#[test]
fn a_required_exact_window_route_failure_never_sends_system_input() {
    let target = target();
    let driver = Arc::new(
        ScriptedDriver::default()
            .unavailable(InputDelivery::WindowMessage, InputFault::RouteUnavailable),
    );
    let controller = controller(target, Arc::clone(&driver));
    let request = InputRequest::new(
        target,
        chord(),
        DeliveryPlan::require(InputDelivery::WindowMessage),
    );

    let receipt = controller
        .execute(&request, &OperationContext::new())
        .expect("receipted");

    assert_eq!(receipt.outcome(), SequenceOutcome::Unexecuted);
    assert_eq!(receipt.fault(), Some(InputFault::RouteUnavailable));
    assert_eq!(attempted_routes(&receipt), [InputDelivery::WindowMessage]);
    assert!(driver.submitted.lock().expect("uncontended").is_empty());
}

#[test]
fn a_partial_sequence_never_retries_through_another_route() {
    let target = target();
    let driver = Arc::new(ScriptedDriver::default().fail_at(1, InputFault::PolicyRefused));
    let controller = controller(target, Arc::clone(&driver));
    let request = InputRequest::new(
        target,
        chord(),
        DeliveryPlan::ordered(vec![InputDelivery::WindowMessage, InputDelivery::System])
            .expect("valid"),
    )
    .with_focus(FocusPolicy::ActivateIfRequired);

    let receipt = controller
        .execute(&request, &OperationContext::new())
        .expect("receipted");

    assert_eq!(receipt.outcome(), SequenceOutcome::Partial);
    assert_eq!(receipt.submitted(), 1);
    assert_eq!(receipt.selected_route(), Some(InputDelivery::WindowMessage));
    assert_eq!(receipt.fault(), Some(InputFault::PolicyRefused));
    assert_eq!(attempted_routes(&receipt), [InputDelivery::WindowMessage]);
    assert_eq!(receipt.cleanup(), CleanupState::Complete);
    assert_eq!(receipt.cleanup_owed(), 1);
    assert_eq!(
        driver.released.lock().expect("uncontended").as_slice(),
        [(
            InputDelivery::WindowMessage,
            PressedState::Key(Key::Modifier(Modifier::Control))
        )]
    );
}

#[test]
fn a_pre_submission_refusal_is_unexecuted_and_never_falls_back() {
    let target = target();
    let driver = Arc::new(ScriptedDriver::default().fail_at(0, InputFault::FocusRequired));
    let controller = controller(target, Arc::clone(&driver));
    let request = InputRequest::new(
        target,
        chord(),
        DeliveryPlan::ordered(vec![InputDelivery::System, InputDelivery::WindowMessage])
            .expect("valid"),
    )
    .with_focus(FocusPolicy::ActivateIfRequired);

    let receipt = controller
        .execute(&request, &OperationContext::new())
        .expect("pre-submission refusal is receipted");

    assert_eq!(receipt.outcome(), SequenceOutcome::Unexecuted);
    assert_eq!(receipt.submitted(), 0);
    assert_eq!(receipt.fault(), Some(InputFault::FocusRequired));
    assert_eq!(attempted_routes(&receipt), [InputDelivery::System]);
    assert!(driver.submitted.lock().expect("uncontended").is_empty());
}

#[test]
fn a_partial_native_event_is_not_misreported_as_unexecuted() {
    let target = target();
    let driver = Arc::new(ScriptedDriver::default().fail_during(0, InputFault::SubmissionFailed));
    let controller = controller(target, Arc::clone(&driver));
    let request = InputRequest::new(
        target,
        InputSequence::new(vec![InputEvent::Text("bounded".to_owned())]).expect("valid"),
        DeliveryPlan::ordered(vec![InputDelivery::WindowMessage, InputDelivery::System])
            .expect("valid"),
    )
    .with_focus(FocusPolicy::ActivateIfRequired);

    let receipt = controller
        .execute(&request, &OperationContext::new())
        .expect("receipted");

    assert_eq!(receipt.outcome(), SequenceOutcome::Partial);
    assert_eq!(receipt.submitted(), 0);
    assert_eq!(receipt.last_submitted(), None);
    assert_eq!(receipt.selected_route(), Some(InputDelivery::WindowMessage));
    assert_eq!(receipt.fault(), Some(InputFault::SubmissionFailed));
    assert_eq!(attempted_routes(&receipt), [InputDelivery::WindowMessage]);
    assert!(receipt.partial_native_effect());
    assert!(driver.submitted.lock().expect("uncontended").is_empty());
}

#[test]
fn a_partial_positioning_effect_does_not_release_an_unpressed_button() {
    let target = target();
    let driver = Arc::new(
        ScriptedDriver::default()
            .fail_during_without_pressed_state(0, InputFault::SubmissionFailed),
    );
    let controller = controller(target, Arc::clone(&driver));
    let request = InputRequest::new(
        target,
        InputSequence::new(vec![InputEvent::PointerPress(PointerButton::Primary)])
            .expect("valid pointer press"),
        DeliveryPlan::require(InputDelivery::WindowMessage),
    );

    let receipt = controller
        .execute(&request, &OperationContext::new())
        .expect("receipted");

    assert_eq!(receipt.outcome(), SequenceOutcome::Partial);
    assert!(receipt.partial_native_effect());
    assert_eq!(receipt.cleanup(), CleanupState::NotNeeded);
    assert_eq!(receipt.cleanup_owed(), 0);
    assert!(driver.released.lock().expect("uncontended").is_empty());
}

#[test]
fn cleanup_releases_only_sequence_owned_state_newest_first() {
    let target = target();
    let driver = Arc::new(ScriptedDriver::default().fail_at(2, InputFault::SubmissionFailed));
    let controller = controller(target, Arc::clone(&driver));
    let request = InputRequest::new(
        target,
        chord(),
        DeliveryPlan::require(InputDelivery::WindowMessage),
    );

    let receipt = controller
        .execute(&request, &OperationContext::new())
        .expect("receipted");

    assert_eq!(receipt.submitted(), 2);
    assert_eq!(receipt.cleanup_released(), 2);
    assert_eq!(receipt.cleanup_owed(), 2);
    assert_eq!(
        driver.released.lock().expect("uncontended").as_slice(),
        [
            (
                InputDelivery::WindowMessage,
                PressedState::Key(Key::Character('c'))
            ),
            (
                InputDelivery::WindowMessage,
                PressedState::Key(Key::Modifier(Modifier::Control))
            ),
        ]
    );
}

#[test]
fn cancellation_between_submissions_preserves_the_partial_count_and_runs_cleanup() {
    let target = target();
    let token = CancellationToken::new();
    let driver = Arc::new(ScriptedDriver::default().cancel_after(1, token.clone()));
    let controller = controller(target, Arc::clone(&driver));
    let request = InputRequest::new(
        target,
        chord(),
        DeliveryPlan::require(InputDelivery::WindowMessage),
    );
    let operation = OperationContext::new().with_cancellation(token);

    let receipt = controller.execute(&request, &operation).expect("receipted");

    assert_eq!(receipt.outcome(), SequenceOutcome::Partial);
    assert_eq!(receipt.submitted(), 1);
    assert_eq!(receipt.fault(), Some(InputFault::Cancelled));
    assert_eq!(receipt.cleanup(), CleanupState::Complete);
    assert_eq!(receipt.cleanup_released(), 1);
}

#[test]
fn deadline_between_submissions_preserves_the_partial_count_and_runs_cleanup() {
    let target = target();
    let clock = Arc::new(ManualClock::default());
    let driver = Arc::new(ScriptedDriver::default().advance_after(
        1,
        Arc::clone(&clock),
        Duration::from_millis(5),
    ));
    let controller = controller(target, Arc::clone(&driver));
    let request = InputRequest::new(
        target,
        chord(),
        DeliveryPlan::require(InputDelivery::WindowMessage),
    );
    let operation = OperationContext::new()
        .with_clock(clock)
        .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(5)));

    let receipt = controller.execute(&request, &operation).expect("receipted");

    assert_eq!(receipt.outcome(), SequenceOutcome::Partial);
    assert_eq!(receipt.submitted(), 1);
    assert_eq!(receipt.fault(), Some(InputFault::DeadlineExceeded));
    assert_eq!(receipt.cleanup(), CleanupState::Complete);
    assert_eq!(receipt.cleanup_released(), 1);
}

#[test]
fn target_loss_after_submission_is_partial_and_cleans_sequence_state() {
    let target = target();
    let driver = Arc::new(ScriptedDriver::default().fail_at(1, InputFault::TargetLost));
    let controller = controller(target, Arc::clone(&driver));
    let request = InputRequest::new(
        target,
        chord(),
        DeliveryPlan::require(InputDelivery::WindowMessage),
    );

    let receipt = controller
        .execute(&request, &OperationContext::new())
        .expect("receipted");

    assert_eq!(receipt.outcome(), SequenceOutcome::Partial);
    assert_eq!(receipt.submitted(), 1);
    assert_eq!(receipt.fault(), Some(InputFault::TargetLost));
    assert_eq!(receipt.cleanup(), CleanupState::Complete);
    assert_eq!(receipt.cleanup_released(), 1);
}

#[test]
fn target_loss_before_submission_is_an_unexecuted_receipt() {
    let target = target();
    let driver = Arc::new(
        ScriptedDriver::default().unavailable(InputDelivery::System, InputFault::TargetLost),
    );
    let controller = controller(target, Arc::clone(&driver));
    let request = InputRequest::new(
        target,
        chord(),
        DeliveryPlan::require(InputDelivery::System),
    )
    .with_focus(FocusPolicy::ActivateIfRequired);

    let receipt = controller
        .execute(&request, &OperationContext::new())
        .expect("receipted");

    assert_eq!(receipt.outcome(), SequenceOutcome::Unexecuted);
    assert_eq!(receipt.fault(), Some(InputFault::TargetLost));
    assert_eq!(receipt.submitted(), 0);
    assert!(driver.submitted.lock().expect("uncontended").is_empty());
}

#[test]
fn close_is_idempotent_and_stops_admission() {
    let target = target();
    let driver = Arc::new(ScriptedDriver::default());
    let controller = controller(target, driver);
    let operation = OperationContext::new();

    controller.close(&operation).expect("closed");
    controller.close(&operation).expect("still closed");
    assert_eq!(controller.lifecycle(), mado_pilot_core::Lifecycle::Closed);

    let request = InputRequest::new(
        target,
        chord(),
        DeliveryPlan::require(InputDelivery::WindowMessage),
    );
    let error = controller
        .execute(&request, &operation)
        .expect_err("closed controller");
    assert_eq!(error.status(), mado_pilot_core::Status::Closed);
}

#[test]
fn delay_observes_cancellation_without_native_submission() {
    let token = CancellationToken::new();
    token.cancel();
    let operation = OperationContext::new().with_cancellation(token);
    assert_eq!(
        wait_delay(Duration::from_millis(10), &operation),
        Err(InputFault::Cancelled)
    );
}

#[test]
fn delay_observes_deadline_with_a_deterministic_clock() {
    let operation = OperationContext::new()
        .with_clock(Arc::new(StepClock::default()))
        .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(3)));
    assert_eq!(
        wait_delay(Duration::from_millis(10), &operation),
        Err(InputFault::DeadlineExceeded)
    );
}

mod unicode_cleanup {
    use super::*;
    use crate::input::{GeometryFingerprint, SystemButtonState, SystemKeyState};
    use crate::native_input::{
        SystemCommitSource, SystemReleaseSource, send_system_release, send_system_text,
    };
    use mado_pilot_core::ActivityTag;
    use mado_pilot_input::CleanupBudget;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP,
        KEYEVENTF_UNICODE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT, VIRTUAL_KEY,
    };

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum NativeUnit {
        Key(u16, u16, u32),
        Mouse(u32),
    }

    type ReleaseRecord = (
        PendingRelease,
        Option<MonotonicInstant>,
        Option<ActivityTag>,
    );

    #[derive(Debug, Default)]
    struct NativeLedgerDriver {
        text_prefix: Mutex<Option<usize>>,
        fail_release: AtomicUsize,
        calls: Mutex<Vec<Vec<NativeUnit>>>,
        routes: Mutex<Vec<InputDelivery>>,
        releases: Mutex<Vec<ReleaseRecord>>,
        cancel_after_text: Mutex<Option<CancellationToken>>,
        advance_after_text: Mutex<Option<(Arc<ManualClock>, Duration)>>,
        advance_before_release: Mutex<Option<(Arc<ManualClock>, Duration)>>,
        advance_after_release: Mutex<Option<(Arc<ManualClock>, Duration)>>,
    }

    impl NativeLedgerDriver {
        fn with_prefix(prefix: usize) -> Self {
            Self {
                text_prefix: Mutex::new(Some(prefix)),
                ..Self::default()
            }
        }

        fn calls(&self) -> Vec<Vec<NativeUnit>> {
            self.calls.lock().expect("uncontended").clone()
        }
    }

    impl SystemCommitSource for NativeLedgerDriver {
        fn revalidate_system_commit(
            &self,
            _focus: FocusPolicy,
            _geometry: Option<GeometryFingerprint>,
        ) -> Result<(), InputFault> {
            Ok(())
        }

        fn send_input(&self, inputs: &[INPUT]) -> usize {
            let units: Vec<_> = inputs
                .iter()
                .map(|input| {
                    if input.r#type == INPUT_KEYBOARD {
                        // SAFETY: all test and production constructors set INPUT_KEYBOARD.
                        let key = unsafe { input.Anonymous.ki };
                        NativeUnit::Key(key.wVk.0, key.wScan, key.dwFlags.0)
                    } else {
                        assert_eq!(input.r#type, INPUT_MOUSE);
                        // SAFETY: the tag identifies the initialized MOUSEINPUT.
                        NativeUnit::Mouse(unsafe { input.Anonymous.mi.dwFlags.0 })
                    }
                })
                .collect();
            let text = matches!(units.first(), Some(NativeUnit::Key(0, _, flags))
                if *flags == KEYEVENTF_UNICODE.0);
            let release = units.len() == 1
                && matches!(units[0],
                NativeUnit::Key(_, _, flags) if flags & KEYEVENTF_KEYUP.0 != 0)
                || units == [NativeUnit::Mouse(MOUSEEVENTF_LEFTUP.0)];
            self.calls.lock().expect("uncontended").push(units);
            if text {
                if let Some(token) = self.cancel_after_text.lock().expect("uncontended").take() {
                    token.cancel();
                }
                if let Some((clock, step)) =
                    self.advance_after_text.lock().expect("uncontended").take()
                {
                    clock.advance(step);
                }
                return self
                    .text_prefix
                    .lock()
                    .expect("uncontended")
                    .take()
                    .unwrap_or(inputs.len());
            }
            if release
                && self.fail_release.fetch_update(
                    Ordering::AcqRel,
                    Ordering::Acquire,
                    |remaining| remaining.checked_sub(1),
                ) == Ok(1)
            {
                return 0;
            }
            inputs.len()
        }

        fn submission_failure(&self, inserted: usize) -> SubmissionFailure {
            if inserted == 0 {
                SubmissionFailure::before_event(InputFault::SubmissionFailed)
            } else {
                SubmissionFailure::during_event(InputFault::SubmissionFailed)
            }
        }
    }

    impl SystemReleaseSource for NativeLedgerDriver {
        fn release_key(&self, _key: Key) -> Result<(VIRTUAL_KEY, u8, bool), InputFault> {
            panic!("successful presses must retain their native key representation");
        }

        fn release_button(&self, _button: PointerButton) -> Result<u8, InputFault> {
            panic!("successful presses must retain their native button representation");
        }
    }

    impl InputDriver for NativeLedgerDriver {
        fn preflight(
            &self,
            route: InputDelivery,
            _focus: FocusPolicy,
            _operation: &OperationContext,
        ) -> Result<(), InputFault> {
            self.routes.lock().expect("uncontended").push(route);
            assert_eq!(route, InputDelivery::System);
            Ok(())
        }

        fn submit(
            &self,
            route: InputDelivery,
            focus: FocusPolicy,
            event: &InputEvent,
            _geometry: PointerGeometry,
            state: &mut DriverState,
            operation: &OperationContext,
        ) -> Result<(), SubmissionFailure> {
            assert_eq!(route, InputDelivery::System);
            assert!(
                state.pending_unicode.is_none(),
                "new submission has no pending native state"
            );
            match event {
                InputEvent::Text(text) => send_system_text(text, self, focus, operation, state),
                InputEvent::KeyPress(key) => {
                    let input = INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: INPUT_0 {
                            ki: KEYBDINPUT {
                                wVk: VIRTUAL_KEY(0x11),
                                wScan: 0x1d,
                                ..KEYBDINPUT::default()
                            },
                        },
                    };
                    assert_eq!(self.send_input(&[input]), 1);
                    state.keys.push(SystemKeyState {
                        logical: *key,
                        virtual_key: 0x11,
                        scan_code: 0x1d,
                        extended: false,
                    });
                    Ok(())
                }
                InputEvent::PointerPress(button) => {
                    let input = INPUT {
                        r#type: INPUT_MOUSE,
                        Anonymous: INPUT_0 {
                            mi: MOUSEINPUT {
                                dwFlags: MOUSEEVENTF_LEFTDOWN,
                                ..MOUSEINPUT::default()
                            },
                        },
                    };
                    assert_eq!(self.send_input(&[input]), 1);
                    state.buttons.push(SystemButtonState {
                        logical: *button,
                        physical: 1,
                    });
                    Ok(())
                }
                _ => panic!("unexpected ordinary event after text: {event:?}"),
            }
        }

        fn release(
            &self,
            route: InputDelivery,
            pressed: PendingRelease,
            state: &mut DriverState,
            operation: &OperationContext,
        ) -> Result<(), InputFault> {
            assert_eq!(route, InputDelivery::System);
            assert!(operation.cancellation().is_none());
            self.releases.lock().expect("uncontended").push((
                pressed,
                operation.deadline(),
                operation.activity_tag(),
            ));
            if let Some((clock, step)) = self
                .advance_before_release
                .lock()
                .expect("uncontended")
                .take()
            {
                clock.advance(step);
            }
            let result = send_system_release(pressed, state, self, operation);
            if let Some((clock, step)) = self
                .advance_after_release
                .lock()
                .expect("uncontended")
                .take()
            {
                clock.advance(step);
            }
            result
        }
    }

    fn setup(
        driver: Arc<NativeLedgerDriver>,
        text: &str,
        held: bool,
        budget: CleanupBudget,
    ) -> (WindowsInputController, InputRequest) {
        let target = target();
        let controller =
            WindowsInputController::with_driver(InputDescriptor::new(target, capability()), driver);
        let mut events = Vec::new();
        if held {
            events.push(InputEvent::KeyPress(Key::Modifier(Modifier::Control)));
            events.push(InputEvent::PointerPress(PointerButton::Primary));
        }
        events.push(InputEvent::Text(text.to_owned()));
        events.push(InputEvent::Delay(Duration::ZERO));
        let request = InputRequest::new(
            target,
            InputSequence::new(events).expect("bounded"),
            DeliveryPlan::ordered(vec![InputDelivery::System, InputDelivery::WindowMessage])
                .expect("ordered"),
        )
        .with_focus(FocusPolicy::RequireFocused)
        .with_cleanup_budget(budget);
        (controller, request)
    }

    fn budget(events: usize) -> CleanupBudget {
        CleanupBudget::at_most(events, Duration::from_millis(10))
    }

    fn unicode_units(text: &str) -> Vec<NativeUnit> {
        text.encode_utf16()
            .flat_map(|unit| {
                [
                    NativeUnit::Key(0, unit, KEYEVENTF_UNICODE.0),
                    NativeUnit::Key(0, unit, (KEYEVENTF_UNICODE | KEYEVENTF_KEYUP).0),
                ]
            })
            .collect()
    }

    #[test]
    fn unicode_odd_prefix_spends_one_shared_event_allowance_newest_first() {
        for allowance in [0, 1, 2, 3, 8] {
            let driver = Arc::new(NativeLedgerDriver::with_prefix(3));
            let (controller, request) =
                setup(driver.clone(), "A\u{1f642}", true, budget(allowance));
            let receipt = controller
                .execute(&request, &OperationContext::new())
                .expect("receipt");
            let released = allowance.min(3);
            assert_eq!(receipt.submitted(), 2);
            assert_eq!(receipt.fault(), Some(InputFault::SubmissionFailed));
            assert!(receipt.partial_native_effect());
            assert_eq!(receipt.cleanup_owed(), 3);
            assert_eq!(receipt.cleanup_released(), released);
            assert_eq!(
                receipt.cleanup(),
                if released == 3 {
                    CleanupState::Complete
                } else {
                    CleanupState::Exhausted
                }
            );
            assert_eq!(attempted_routes(&receipt), [InputDelivery::System]);
            let mut expected = vec![
                vec![NativeUnit::Key(0x11, 0x1d, 0)],
                vec![NativeUnit::Mouse(MOUSEEVENTF_LEFTDOWN.0)],
                unicode_units("A\u{1f642}"),
            ];
            let releases = [
                vec![NativeUnit::Key(
                    0,
                    0xd83d,
                    (KEYEVENTF_UNICODE | KEYEVENTF_KEYUP).0,
                )],
                vec![NativeUnit::Mouse(MOUSEEVENTF_LEFTUP.0)],
                vec![NativeUnit::Key(0x11, 0x1d, KEYEVENTF_KEYUP.0)],
            ];
            expected.extend_from_slice(&releases[..released]);
            assert_eq!(
                driver.calls(),
                expected,
                "no inline release, retry, or text completion"
            );
        }
    }

    #[test]
    fn unicode_zero_even_odd_and_full_prefixes_preserve_logical_text_accounting() {
        for text in ["AB", "\u{1f642}"] {
            for prefix in 0..=4 {
                let driver = Arc::new(NativeLedgerDriver::with_prefix(prefix));
                let (controller, request) = setup(driver.clone(), text, false, budget(4));
                let receipt = controller
                    .execute(&request, &OperationContext::new())
                    .expect("receipt");
                let owed = usize::from(prefix % 2 == 1);
                assert_eq!(receipt.submitted(), if prefix == 4 { 2 } else { 0 });
                assert_eq!(receipt.partial_native_effect(), prefix > 0 && prefix < 4);
                assert_eq!(receipt.cleanup_owed(), owed);
                assert_eq!(receipt.cleanup_released(), owed);
                assert_eq!(
                    receipt.cleanup(),
                    if owed == 0 {
                        CleanupState::NotNeeded
                    } else {
                        CleanupState::Complete
                    }
                );
                assert_eq!(
                    receipt.fault(),
                    (prefix < 4).then_some(InputFault::SubmissionFailed)
                );
                assert_eq!(
                    receipt.outcome(),
                    match prefix {
                        0 => SequenceOutcome::Unexecuted,
                        4 => SequenceOutcome::Complete,
                        _ => SequenceOutcome::Partial,
                    }
                );
                let mut expected = vec![unicode_units(text)];
                if owed != 0 {
                    let unit = text.encode_utf16().nth(prefix / 2).expect("unmatched unit");
                    expected.push(vec![NativeUnit::Key(
                        0,
                        unit,
                        (KEYEVENTF_UNICODE | KEYEVENTF_KEYUP).0,
                    )]);
                }
                assert_eq!(driver.calls(), expected);
                assert_eq!(
                    driver.routes.lock().expect("uncontended").as_slice(),
                    [InputDelivery::System]
                );
            }
        }
    }

    #[test]
    fn unicode_failed_release_stops_without_credit_or_modifier_retry() {
        for failed_release in 1..=3 {
            let driver = Arc::new(NativeLedgerDriver::with_prefix(1));
            driver.fail_release.store(failed_release, Ordering::Release);
            let (controller, request) = setup(driver.clone(), "AB", true, budget(8));
            let receipt = controller
                .execute(&request, &OperationContext::new())
                .expect("receipt");
            assert_eq!(receipt.fault(), Some(InputFault::SubmissionFailed));
            assert_eq!(receipt.submitted(), 2);
            assert!(receipt.partial_native_effect());
            assert_eq!(receipt.cleanup(), CleanupState::Incomplete);
            assert_eq!(
                (receipt.cleanup_owed(), receipt.cleanup_released()),
                (3, failed_release - 1)
            );
            let releases = [
                vec![NativeUnit::Key(
                    0,
                    65,
                    (KEYEVENTF_UNICODE | KEYEVENTF_KEYUP).0,
                )],
                vec![NativeUnit::Mouse(MOUSEEVENTF_LEFTUP.0)],
                vec![NativeUnit::Key(0x11, 0x1d, KEYEVENTF_KEYUP.0)],
            ];
            assert_eq!(&driver.calls()[3..], &releases[..failed_release]);
            assert_eq!(
                driver.releases.lock().expect("uncontended").len(),
                failed_release
            );
        }
    }

    #[test]
    fn unicode_zero_and_even_prefixes_still_release_prior_logical_state() {
        for prefix in [0, 2] {
            let driver = Arc::new(NativeLedgerDriver::with_prefix(prefix));
            let (controller, request) = setup(driver.clone(), "AB", true, budget(8));
            let receipt = controller
                .execute(&request, &OperationContext::new())
                .expect("receipt");
            assert_eq!(receipt.submitted(), 2);
            assert_eq!(receipt.fault(), Some(InputFault::SubmissionFailed));
            assert_eq!(receipt.partial_native_effect(), prefix != 0);
            assert_eq!((receipt.cleanup_owed(), receipt.cleanup_released()), (2, 2));
            assert_eq!(receipt.cleanup(), CleanupState::Complete);
            assert_eq!(
                &driver.calls()[3..],
                [
                    vec![NativeUnit::Mouse(MOUSEEVENTF_LEFTUP.0)],
                    vec![NativeUnit::Key(0x11, 0x1d, KEYEVENTF_KEYUP.0)],
                ]
            );
        }
    }

    #[test]
    fn unicode_cleanup_uses_one_fresh_context_after_cancellation_or_deadline() {
        for cancel in [true, false] {
            for expire_cleanup in [true, false] {
                let clock = Arc::new(ManualClock::default());
                let token = CancellationToken::new();
                let driver = Arc::new(NativeLedgerDriver::with_prefix(3));
                if cancel {
                    *driver.cancel_after_text.lock().expect("uncontended") = Some(token.clone());
                }
                *driver.advance_after_text.lock().expect("uncontended") =
                    Some((clock.clone(), Duration::from_millis(5)));
                if expire_cleanup {
                    *driver.advance_after_release.lock().expect("uncontended") =
                        Some((clock.clone(), Duration::from_millis(10)));
                }
                let tag = ActivityTag::new(7).expect("tag");
                let operation = OperationContext::new()
                    .with_clock(clock)
                    .with_cancellation(token)
                    .with_activity_tag(tag)
                    .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(5)));
                let (controller, request) = setup(driver.clone(), "\u{1f642}", true, budget(8));
                let receipt = controller.execute(&request, &operation).expect("receipt");
                assert_eq!(receipt.fault(), Some(InputFault::SubmissionFailed));
                assert_eq!(receipt.submitted(), 2);
                assert!(receipt.partial_native_effect());
                let released = if expire_cleanup { 1 } else { 3 };
                assert_eq!(
                    (receipt.cleanup_owed(), receipt.cleanup_released()),
                    (3, released)
                );
                assert_eq!(
                    receipt.cleanup(),
                    if expire_cleanup {
                        CleanupState::Exhausted
                    } else {
                        CleanupState::Complete
                    }
                );
                let contexts = driver.releases.lock().expect("uncontended");
                assert_eq!(contexts.len(), released);
                for (_, deadline, activity) in contexts.iter() {
                    assert_eq!(
                        *deadline,
                        Some(MonotonicInstant::from_origin(Duration::from_millis(15)))
                    );
                    assert_eq!(*activity, Some(tag));
                }
                assert_eq!(driver.calls().len(), 3 + released);
            }
        }
    }

    #[test]
    fn unicode_full_acceptance_then_interruption_counts_text_once() {
        for cancel in [true, false] {
            let clock = Arc::new(ManualClock::default());
            let token = CancellationToken::new();
            let driver = Arc::new(NativeLedgerDriver::default());
            if cancel {
                *driver.cancel_after_text.lock().expect("uncontended") = Some(token.clone());
            } else {
                *driver.advance_after_text.lock().expect("uncontended") =
                    Some((clock.clone(), Duration::from_millis(5)));
            }
            let operation = OperationContext::new()
                .with_clock(clock)
                .with_cancellation(token)
                .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(5)));
            let (controller, request) = setup(driver.clone(), "\u{1f642}", true, budget(8));
            let receipt = controller.execute(&request, &operation).expect("receipt");
            assert_eq!(receipt.submitted(), 3);
            assert_eq!(
                receipt.fault(),
                Some(if cancel {
                    InputFault::Cancelled
                } else {
                    InputFault::DeadlineExceeded
                })
            );
            assert!(!receipt.partial_native_effect());
            assert_eq!((receipt.cleanup_owed(), receipt.cleanup_released()), (2, 2));
            assert_eq!(receipt.cleanup(), CleanupState::Complete);
            assert_eq!(driver.calls().len(), 5);
        }
    }

    #[test]
    fn unicode_exhaustion_does_not_leak_pending_state_into_next_execution() {
        let driver = Arc::new(NativeLedgerDriver::with_prefix(1));
        let (controller, request) = setup(driver.clone(), "AB", false, budget(0));
        let first = controller
            .execute(&request, &OperationContext::new())
            .expect("first");
        assert_eq!(first.cleanup(), CleanupState::Exhausted);
        assert_eq!((first.cleanup_owed(), first.cleanup_released()), (1, 0));
        let second = controller
            .execute(&request, &OperationContext::new())
            .expect("second");
        assert_eq!(second.outcome(), SequenceOutcome::Complete);
        assert_eq!(second.cleanup(), CleanupState::NotNeeded);
        assert_eq!(second.cleanup_owed(), 0);
        assert_eq!(driver.calls(), [unicode_units("AB"), unicode_units("AB")]);
    }

    #[test]
    fn unicode_zero_cleanup_duration_starts_no_release() {
        let driver = Arc::new(NativeLedgerDriver::with_prefix(1));
        let (controller, request) = setup(
            driver.clone(),
            "AB",
            true,
            CleanupBudget::at_most(8, Duration::ZERO),
        );
        let receipt = controller
            .execute(&request, &OperationContext::new())
            .expect("receipt");
        assert_eq!(receipt.cleanup(), CleanupState::Exhausted);
        assert_eq!((receipt.cleanup_owed(), receipt.cleanup_released()), (3, 0));
        assert_eq!(driver.calls().len(), 3);
    }

    #[test]
    fn unicode_cleanup_deadline_at_native_boundary_is_exhausted_without_a_send() {
        let clock = Arc::new(ManualClock::default());
        let driver = Arc::new(NativeLedgerDriver::with_prefix(1));
        *driver.advance_before_release.lock().expect("uncontended") =
            Some((clock.clone(), Duration::from_millis(10)));
        let (controller, request) = setup(driver.clone(), "AB", true, budget(8));
        let operation = OperationContext::new().with_clock(clock);
        let receipt = controller.execute(&request, &operation).expect("receipt");
        assert_eq!(receipt.fault(), Some(InputFault::SubmissionFailed));
        assert_eq!(receipt.submitted(), 2);
        assert_eq!(receipt.cleanup(), CleanupState::Exhausted);
        assert_eq!((receipt.cleanup_owed(), receipt.cleanup_released()), (3, 0));
        assert_eq!(
            driver.calls().len(),
            3,
            "no native release after its deadline"
        );
    }
}
