//! An input adapter a test drives by hand.
//!
//! Real input cannot be verified without perturbing the desktop it is delivered
//! to, and the cases that matter most — a mechanism that refuses part-way, cleanup
//! that cannot release everything, a deadline that expires between two events —
//! cannot be arranged on a real desktop at all. This double records exactly what
//! it was asked to deliver and fails wherever a test tells it to, so the receipt
//! contract can be exercised event by event.
//!
//! It delivers nothing. Nothing here calls an operating system, and that is the
//! point: a test asserts what the contract says about delivery, not what a
//! platform does.

use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use mado_pilot_core::{
    CoordinateSpace, InputCapability, InputDelivery, InputOperationKind, Lifecycle, Operation,
    OperationContext, PermissionKind, ProviderId, Result, TargetId,
};
use mado_pilot_input::{
    Admission, InputController, InputDescriptor, InputEvent, InputFault, InputOpenRequest,
    InputProvider, InputReceipt, InputRequest, PressedState,
};

/// Provider name qualifying this double's target identities.
pub const PROVIDER: ProviderId = ProviderId::new("controlled");

/// What the controller should do with the sequence it was given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Behavior {
    /// Deliver every event.
    Complete,
    /// Deliver the first `delivered` events, then stop with `fault`.
    FailAfter {
        /// How many events reach the target before the failure.
        delivered: usize,
        /// Why the next one does not.
        fault: InputFault,
    },
    /// Refuse `mechanism` outright, so a permitted fallback is tried next.
    Refuse {
        /// The mechanism that refuses.
        mechanism: InputDelivery,
        /// Why it refuses.
        fault: InputFault,
    },
    /// Deliver nothing and report `fault`.
    Unexecuted(InputFault),
}

/// How the releases after a partial failure go.
///
/// This is about releases the platform *refuses*. A cleanup that stops because it
/// ran out of budget is driven by the request's
/// [`CleanupBudget`](mado_pilot_input::CleanupBudget) rather than from here, and
/// the receipt keeps the two apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cleanup {
    /// Every release the budget allows succeeds.
    Complete,
    /// This many succeed and then the platform refuses the next one.
    Partial(usize),
    /// The platform refuses the first release, as a target that has gone does.
    Fails,
}

/// One event the controller was asked to deliver, with the mechanism used.
#[derive(Debug, Clone, PartialEq)]
pub struct Delivered {
    /// The mechanism the event went through.
    pub mechanism: InputDelivery,
    /// The event itself, as the request expressed it.
    pub event: InputEvent,
}

/// An [`InputProvider`] whose behavior a test writes.
pub struct ControlledInput {
    target: TargetId,
    capability: InputCapability,
    behavior: Mutex<Behavior>,
    cleanup: Mutex<Cleanup>,
    log: Arc<Mutex<Vec<Delivered>>>,
    releases: Arc<Mutex<Vec<PressedState>>>,
    executing: Arc<AtomicUsize>,
}

impl ControlledInput {
    /// Builds a provider for `target` that accepts pointer, keyboard, and text
    /// input through the system path.
    #[must_use]
    pub fn new(target: TargetId) -> Self {
        Self::with_capability(
            target,
            InputCapability::none()
                .with_pair(InputOperationKind::Pointer, InputDelivery::System)
                .with_pair(InputOperationKind::Keyboard, InputDelivery::System)
                .with_pair(InputOperationKind::Text, InputDelivery::System)
                .with_pointer_space(CoordinateSpace::CapturePixels)
                .with_pointer_space(CoordinateSpace::FrameNormalized)
                .with_permission(PermissionKind::InputControl),
        )
    }

    /// Builds a provider for `target` with exactly `capability`.
    #[must_use]
    pub fn with_capability(target: TargetId, capability: InputCapability) -> Self {
        Self {
            target,
            capability,
            behavior: Mutex::new(Behavior::Complete),
            cleanup: Mutex::new(Cleanup::Complete),
            log: Arc::new(Mutex::new(Vec::new())),
            releases: Arc::new(Mutex::new(Vec::new())),
            executing: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Sets what the controller does with the next sequence.
    pub fn set_behavior(&self, behavior: Behavior) {
        *self.behavior.lock().expect("uncontended") = behavior;
    }

    /// Sets how much of the next cleanup succeeds.
    pub fn set_cleanup(&self, cleanup: Cleanup) {
        *self.cleanup.lock().expect("uncontended") = cleanup;
    }

    /// Returns every event the controller was asked to deliver, in order.
    #[must_use]
    pub fn delivered(&self) -> Vec<Delivered> {
        self.log.lock().expect("uncontended").clone()
    }

    /// Returns every pressed state cleanup released, in the order it released them.
    #[must_use]
    pub fn released(&self) -> Vec<PressedState> {
        self.releases.lock().expect("uncontended").clone()
    }

    /// Returns how many sequences are inside `execute` right now.
    ///
    /// A test that needs one sequence to be holding the controller before it does
    /// something else waits on this rather than sleeping. A sleep is a guess about
    /// scheduling, and a test that guesses wrong fails while naming a contract rule
    /// it never reached.
    #[must_use]
    pub fn executing(&self) -> usize {
        self.executing.load(Ordering::Relaxed)
    }

    /// Returns the target this provider drives.
    #[must_use]
    pub const fn target(&self) -> TargetId {
        self.target
    }

    /// Returns what the target accepts.
    #[must_use]
    pub const fn capability(&self) -> InputCapability {
        self.capability
    }
}

impl fmt::Debug for ControlledInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ControlledInput")
            .field("target", &self.target)
            .field("delivered", &self.delivered().len())
            .finish()
    }
}

impl InputProvider for ControlledInput {
    fn provider(&self) -> ProviderId {
        PROVIDER
    }

    fn describe(&self, target: TargetId, operation: &OperationContext) -> Result<InputDescriptor> {
        let attempt = Operation::admit(operation)?;
        if target != self.target {
            return Err(InputFault::ForeignTarget.into());
        }
        Ok(attempt.commit(InputDescriptor::new(target, self.capability))?)
    }

    fn open(
        &self,
        target: TargetId,
        request: &InputOpenRequest,
        operation: &OperationContext,
    ) -> Result<Arc<dyn InputController>> {
        let attempt = Operation::admit(operation)?;
        if target != self.target {
            return Err(InputFault::ForeignTarget.into());
        }
        request.check(self.capability)?;

        let controller = Arc::new(ControlledController {
            descriptor: InputDescriptor::new(target, self.capability),
            behavior: Mutex::new(self.behavior.lock().expect("uncontended").clone()),
            cleanup: Mutex::new(*self.cleanup.lock().expect("uncontended")),
            log: Arc::clone(&self.log),
            releases: Arc::clone(&self.releases),
            executing: Arc::clone(&self.executing),
            admission: Admission::new(),
        });
        Ok(attempt.commit(controller as Arc<dyn InputController>)?)
    }
}

/// The controller [`ControlledInput`] opens.
struct ControlledController {
    descriptor: InputDescriptor,
    behavior: Mutex<Behavior>,
    cleanup: Mutex<Cleanup>,
    log: Arc<Mutex<Vec<Delivered>>>,
    releases: Arc<Mutex<Vec<PressedState>>>,
    executing: Arc<AtomicUsize>,
    admission: Admission,
}

/// What became of the releases one cleanup owed.
struct CleanupOutcome {
    released: usize,
    owed: usize,
    /// True when a bound stopped cleanup, false when a release was refused.
    exhausted: bool,
}

impl CleanupOutcome {
    /// Records this outcome on `receipt`.
    fn apply(self, receipt: InputReceipt) -> InputReceipt {
        if self.exhausted {
            receipt.with_exhausted_cleanup(self.released, self.owed)
        } else {
            receipt.with_cleanup(self.released, self.owed)
        }
    }
}

impl ControlledController {
    /// Sets what this controller does with the next sequence.
    fn behavior(&self) -> Behavior {
        self.behavior.lock().expect("uncontended").clone()
    }

    fn record(&self, mechanism: InputDelivery, events: &[InputEvent]) {
        let mut log = self.log.lock().expect("uncontended");
        for event in events {
            log.push(Delivered {
                mechanism,
                event: event.clone(),
            });
        }
    }

    /// Releases what the sequence left held, under the request's cleanup budget.
    ///
    /// Two ways to stop short, kept apart because a caller acts on them
    /// differently: the budget's event count or deadline runs out, or the script
    /// says the platform refused the next release. The budget's own context is
    /// derived here rather than reusing `operation`, which is usually the
    /// interrupted one.
    fn run_cleanup(
        &self,
        held: &[PressedState],
        request: &InputRequest,
        operation: &OperationContext,
    ) -> CleanupOutcome {
        let budget = request.cleanup_budget();
        let context = budget.context(operation);
        let refuses_after = match *self.cleanup.lock().expect("uncontended") {
            Cleanup::Complete => held.len(),
            Cleanup::Partial(count) => count.min(held.len()),
            Cleanup::Fails => 0,
        };

        let mut released = 0usize;
        let mut exhausted = false;
        for state in held {
            if released >= budget.max_events() || context.interruption().is_some() {
                exhausted = true;
                break;
            }
            if released >= refuses_after {
                // The platform refused this one. Cleanup stops rather than
                // skipping ahead: the next release may depend on this one.
                break;
            }
            self.releases.lock().expect("uncontended").push(*state);
            released += 1;
        }

        CleanupOutcome {
            released,
            owed: held.len(),
            exhausted,
        }
    }
}

/// Counts one sequence as being inside `execute` for as long as it is.
struct ExecutingGuard<'controller> {
    executing: &'controller AtomicUsize,
}

impl<'controller> ExecutingGuard<'controller> {
    fn new(executing: &'controller AtomicUsize) -> Self {
        executing.fetch_add(1, Ordering::Release);
        Self { executing }
    }
}

impl Drop for ExecutingGuard<'_> {
    fn drop(&mut self) {
        self.executing.fetch_sub(1, Ordering::Release);
    }
}

impl fmt::Debug for ControlledController {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ControlledController")
            .field("target", &self.descriptor.target())
            .field("lifecycle", &self.admission.lifecycle())
            .finish()
    }
}

impl InputController for ControlledController {
    fn descriptor(&self) -> InputDescriptor {
        self.descriptor.clone()
    }

    fn execute(
        &self,
        request: &InputRequest,
        operation: &OperationContext,
    ) -> Result<InputReceipt> {
        // Admission first, and against the descriptor rather than by hand: a double
        // that admitted what a real Adapter refuses would make the suite pass for
        // requests no Adapter accepts.
        let selected = self.descriptor.admit(request)?;
        let _guard = self.admission.admit(operation)?;
        // Counted from here, so a test can wait for the controller to be held
        // instead of guessing how long admission takes.
        let _executing = ExecutingGuard::new(&self.executing);
        let target = request.target();
        let events = request.sequence().events();

        let (mechanism, behavior) = match self.behavior() {
            Behavior::Refuse { mechanism, fault } if mechanism == selected => {
                // The caller's own order decides what is tried next; a mechanism the
                // request did not permit is never substituted.
                match request
                    .delivery()
                    .modes()
                    .iter()
                    .copied()
                    .find(|candidate| *candidate != mechanism)
                {
                    Some(fallback) => (fallback, Behavior::Complete),
                    None => {
                        return Ok(
                            InputReceipt::unexecuted(target, fault).with_attempted(vec![mechanism])
                        );
                    }
                }
            }
            behavior => (selected, behavior),
        };
        let attempted = if mechanism == selected {
            vec![selected]
        } else {
            vec![selected, mechanism]
        };

        let receipt = match behavior {
            Behavior::Complete => {
                // Counted here rather than from the shared log: the log spans every
                // sequence this double has been asked to deliver, and a receipt
                // counts the events of its own sequence.
                let mut delivered = 0usize;
                for event in events {
                    if event.is_irreversible() {
                        // The deadline is checked before every irreversible event,
                        // which is what makes a partial receipt truthful rather than
                        // a guess about where the interruption landed.
                        if let Err(interruption) = Operation::admit(operation) {
                            let held = request.sequence().held_after(delivered);
                            let cleanup = self.run_cleanup(&held, request, operation);
                            return Ok(cleanup.apply(
                                InputReceipt::partial(
                                    target,
                                    mechanism,
                                    delivered,
                                    InputFault::from(interruption),
                                )
                                .with_attempted(attempted),
                            ));
                        }
                    }
                    if let InputEvent::Delay(delay) = event {
                        // A sequence that says wait, waits. A double that skipped
                        // the delay would hold the controller for no time at all,
                        // and every rule about one sequence waiting for another
                        // would be unreachable through it.
                        thread::sleep(*delay);
                    }
                    self.record(mechanism, std::slice::from_ref(event));
                    delivered += 1;
                }
                InputReceipt::complete(target, mechanism, delivered).with_attempted(attempted)
            }
            Behavior::FailAfter { delivered, fault } => {
                let delivered = delivered.min(events.len());
                self.record(mechanism, &events[..delivered]);
                let held = request.sequence().held_after(delivered);
                let cleanup = self.run_cleanup(&held, request, operation);
                cleanup.apply(
                    InputReceipt::partial(target, mechanism, delivered, fault)
                        .with_attempted(attempted),
                )
            }
            Behavior::Unexecuted(fault) => {
                InputReceipt::unexecuted(target, fault).with_attempted(attempted)
            }
            Behavior::Refuse { fault, .. } => {
                InputReceipt::unexecuted(target, fault).with_attempted(attempted)
            }
        };
        Ok(receipt)
    }

    fn close(&self, operation: &OperationContext) -> Result<()> {
        self.admission.drain(operation)
    }

    fn lifecycle(&self) -> Lifecycle {
        self.admission.lifecycle()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{Behavior, Cleanup, ControlledInput};
    use mado_pilot_core::{
        CoordinateSpace, IdentityIssuer, InputCapability, InputDelivery, InputOperationKind,
        Lifecycle, OperationContext, Point, ProviderId, Status, TargetId,
    };
    use mado_pilot_input::{
        CleanupState, DeliveryPlan, InputEvent, InputFault, InputOpenRequest, InputProvider,
        InputRequest, InputSequence, Key, Modifier, PointerButton, PressedState, SequenceOutcome,
    };

    fn target() -> TargetId {
        IdentityIssuer::new()
            .issue_target(ProviderId::new("controlled"))
            .expect("issued")
    }

    fn point() -> Point {
        Point::new(CoordinateSpace::CapturePixels, 2.0, 2.0).expect("valid")
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

    fn request(target: TargetId, sequence: InputSequence) -> InputRequest {
        InputRequest::new(
            target,
            sequence,
            DeliveryPlan::require(InputDelivery::System),
        )
    }

    #[test]
    fn a_complete_sequence_records_every_event_in_order() {
        let target = target();
        let provider = ControlledInput::new(target);
        let context = OperationContext::new();
        let controller = provider
            .open(target, &InputOpenRequest::new(), &context)
            .expect("opened");

        let receipt = controller
            .execute(&request(target, chord()), &context)
            .expect("executed");

        assert_eq!(receipt.outcome(), SequenceOutcome::Complete);
        assert_eq!(receipt.delivered(), 4);
        assert_eq!(receipt.cleanup(), CleanupState::NotNeeded);
        let delivered = provider.delivered();
        assert_eq!(delivered.len(), 4);
        assert_eq!(delivered[0].mechanism, InputDelivery::System);
        assert_eq!(
            delivered[1].event,
            InputEvent::KeyPress(Key::Character('c'))
        );
    }

    #[test]
    fn a_partial_failure_reports_its_count_and_releases_what_it_pressed() {
        let target = target();
        let provider = ControlledInput::new(target);
        provider.set_behavior(Behavior::FailAfter {
            delivered: 2,
            fault: InputFault::DeliveryFailed,
        });
        let context = OperationContext::new();
        let controller = provider
            .open(target, &InputOpenRequest::new(), &context)
            .expect("opened");

        let receipt = controller
            .execute(&request(target, chord()), &context)
            .expect("executed");

        assert_eq!(receipt.outcome(), SequenceOutcome::Partial);
        assert_eq!(receipt.delivered(), 2);
        assert_eq!(receipt.last_completed(), Some(1));
        assert_eq!(receipt.failure(), Some(InputFault::DeliveryFailed));
        assert_eq!(receipt.cleanup(), CleanupState::Complete);
        assert_eq!(receipt.cleanup_owed(), 2);
        assert_eq!(
            provider.released(),
            vec![
                PressedState::Key(Key::Character('c')),
                PressedState::Key(Key::Modifier(Modifier::Control)),
            ],
            "cleanup releases in reverse order of pressing"
        );
    }

    #[test]
    fn cleanup_that_cannot_release_everything_says_so() {
        let target = target();
        let provider = ControlledInput::new(target);
        provider.set_behavior(Behavior::FailAfter {
            delivered: 2,
            fault: InputFault::PolicyRefused,
        });
        provider.set_cleanup(Cleanup::Partial(1));
        let context = OperationContext::new();
        let controller = provider
            .open(target, &InputOpenRequest::new(), &context)
            .expect("opened");

        let receipt = controller
            .execute(&request(target, chord()), &context)
            .expect("executed");

        assert_eq!(receipt.cleanup(), CleanupState::Incomplete);
        assert!(receipt.cleanup().may_leave_state_held());
        assert_eq!(receipt.cleanup_released(), 1);
        assert_eq!(receipt.cleanup_owed(), 2);
        assert_eq!(provider.released().len(), 1);
    }

    #[test]
    fn a_refused_mechanism_falls_back_only_where_the_caller_permitted_it() {
        let target = target();
        let provider = ControlledInput::with_capability(
            target,
            InputCapability::none()
                .with_pair(InputOperationKind::Pointer, InputDelivery::System)
                .with_pair(InputOperationKind::Pointer, InputDelivery::BackgroundTarget)
                .with_pointer_space(CoordinateSpace::CapturePixels),
        );
        provider.set_behavior(Behavior::Refuse {
            mechanism: InputDelivery::BackgroundTarget,
            fault: InputFault::DeliveryUnavailable,
        });
        let context = OperationContext::new();
        let controller = provider
            .open(target, &InputOpenRequest::new(), &context)
            .expect("opened");
        let sequence = InputSequence::new(vec![InputEvent::PointerMove(point())]).expect("valid");

        let permitted = controller
            .execute(
                &InputRequest::new(
                    target,
                    sequence.clone(),
                    DeliveryPlan::ordered(vec![
                        InputDelivery::BackgroundTarget,
                        InputDelivery::System,
                    ])
                    .expect("valid"),
                ),
                &context,
            )
            .expect("executed");

        assert!(permitted.is_complete());
        assert_eq!(permitted.delivery(), Some(InputDelivery::System));
        assert!(permitted.used_fallback());

        let required = controller
            .execute(
                &InputRequest::new(
                    target,
                    sequence,
                    DeliveryPlan::require(InputDelivery::BackgroundTarget),
                ),
                &context,
            )
            .expect("executed");

        assert_eq!(required.outcome(), SequenceOutcome::Unexecuted);
        assert_eq!(required.delivered(), 0);
        assert!(!required.used_fallback());
    }

    #[test]
    fn an_unsupported_combination_fails_admission_before_any_event() {
        let target = target();
        let provider = ControlledInput::with_capability(
            target,
            InputCapability::none()
                .with_pair(InputOperationKind::Pointer, InputDelivery::System)
                .with_pointer_space(CoordinateSpace::CapturePixels),
        );
        let context = OperationContext::new();
        let controller = provider
            .open(target, &InputOpenRequest::new(), &context)
            .expect("opened");

        let error = controller
            .execute(&request(target, chord()), &context)
            .expect_err("keyboard input was never advertised");

        assert_eq!(error.status(), Status::Unsupported);
        assert!(
            provider.delivered().is_empty(),
            "admission failed before delivery"
        );
    }

    #[test]
    fn a_closed_controller_admits_nothing_and_closes_idempotently() {
        let target = target();
        let provider = ControlledInput::new(target);
        let context = OperationContext::new();
        let controller = provider
            .open(target, &InputOpenRequest::new(), &context)
            .expect("opened");

        controller.close(&context).expect("closed");
        assert_eq!(controller.lifecycle(), Lifecycle::Closed);
        controller.close(&context).expect("already closed");

        let error = controller
            .execute(&request(target, chord()), &context)
            .expect_err("a closed controller accepts nothing");

        assert_eq!(error.status(), Status::Closed);
        assert!(provider.delivered().is_empty());
    }

    #[test]
    fn a_foreign_target_is_refused_before_a_controller_exists() {
        let provider = ControlledInput::new(target());
        let context = OperationContext::new();

        let error = provider
            .open(target(), &InputOpenRequest::new(), &context)
            .expect_err("another target");

        assert_eq!(error.status(), Status::InvalidArgument);
    }

    #[test]
    fn two_controllers_share_one_log_so_a_test_sees_one_history() {
        let target = target();
        let provider = ControlledInput::new(target);
        let context = OperationContext::new();
        let first = provider
            .open(target, &InputOpenRequest::new(), &context)
            .expect("opened");
        let second = provider
            .open(target, &InputOpenRequest::new(), &context)
            .expect("opened");

        first
            .execute(
                &request(
                    target,
                    InputSequence::new(vec![InputEvent::KeyPress(Key::Enter)]).expect("valid"),
                ),
                &context,
            )
            .expect("executed");
        second
            .execute(
                &request(
                    target,
                    InputSequence::new(vec![InputEvent::PointerPress(PointerButton::Primary)])
                        .expect("valid"),
                ),
                &context,
            )
            .expect("executed");

        assert_eq!(provider.delivered().len(), 2);
        drop(Arc::clone(&first));
    }
}
