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
    InputFault, InputReceipt, InputRequest, InputSequence, Key, Modifier, PointerGeometry,
    PressedState, SequenceOutcome,
};

use super::{
    DriverState, GeometryLedger, InputDriver, SubmissionContexts, SubmissionFailure,
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
    fail_at: Mutex<Option<(usize, InputFault, bool)>>,
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
        *self.fail_at.lock().expect("uncontended") = Some((index, fault, false));
        self
    }

    fn fail_during(self, index: usize, fault: InputFault) -> Self {
        *self.fail_at.lock().expect("uncontended") = Some((index, fault, true));
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
        _contexts: SubmissionContexts<'_>,
    ) -> Result<(), SubmissionFailure> {
        let index = self.attempts.fetch_add(1, Ordering::AcqRel);
        if let Some((failure_index, fault, during_event)) =
            *self.fail_at.lock().expect("uncontended")
            && failure_index == index
        {
            return Err(if during_event {
                SubmissionFailure::during_event(fault)
            } else {
                SubmissionFailure::before_event(fault)
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
        pressed: PressedState,
        _state: &mut DriverState,
        _operation: &OperationContext,
    ) -> Result<(), InputFault> {
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
