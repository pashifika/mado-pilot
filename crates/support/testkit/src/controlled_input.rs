//! A caller-controlled input Adapter for deterministic contract tests.
//!
//! It never calls an operating system. Tests choose where native submission
//! stops, whether the current logical event may have had effect, and how bounded
//! cleanup proceeds. The Adapter records route attempts and complete submitted
//! events so receipt semantics can be proved without perturbing a desktop.

use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use mado_pilot_core::{
    CapabilitySupport, CoordinateSpace, InputCapability, InputDelivery, InputOperationKind,
    Lifecycle, Operation, OperationContext, PermissionKind, ProviderId, Result, SubmissionEvidence,
    TargetId,
};
use mado_pilot_input::{
    Admission, FocusPolicy, InputAttempt, InputController, InputDescriptor, InputEvent, InputFault,
    InputOpenRequest, InputProvider, InputReceipt, InputRequest, PointerGeometry, PressedState,
};

/// Provider name qualifying this double's target identities.
pub const PROVIDER: ProviderId = ProviderId::new("controlled");

/// What the controller should do with each sequence it receives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Behavior {
    /// Submit every logical event.
    Complete,
    /// Submit `submitted` complete logical events, then stop before the next one.
    FailAfter {
        /// Number of complete logical events submitted.
        submitted: usize,
        /// Why the next event was not submitted.
        fault: InputFault,
    },
    /// Submit `submitted` complete events, then fail while submitting the next.
    FailDuring {
        /// Number of complete logical events submitted before the partial event.
        submitted: usize,
        /// Why submission stopped.
        fault: InputFault,
    },
    /// Refuse `route` before native effect, permitting a caller-authorized fallback.
    Refuse {
        /// Route that refuses.
        route: InputDelivery,
        /// Why it refuses.
        fault: InputFault,
    },
    /// Refuse every visited route with `fault` before native effect.
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

/// One complete logical event submitted by the controlled Adapter.
#[derive(Debug, Clone, PartialEq)]
pub struct SubmittedEvent {
    /// Route through which the event was submitted.
    pub route: InputDelivery,
    /// Event exactly as the request expressed it.
    pub event: InputEvent,
}

/// One sequence admitted to a route, with the caller policies it carried.
///
/// This log proves policy pass-through. Route refusal remains observable in the
/// receipt attempt list and does not create an admitted-sequence record.
#[derive(Debug, Clone, PartialEq)]
pub struct Admitted {
    /// Route selected after preflight.
    pub selected_route: InputDelivery,
    /// Routes the caller permitted, in caller order.
    pub routes: Vec<InputDelivery>,
    /// Focus policy carried by the request.
    pub focus: FocusPolicy,
    /// Pointer geometry policy carried by the request.
    pub geometry: PointerGeometry,
    /// Number of logical events in the sequence.
    pub events: usize,
}

/// An [`InputProvider`] whose behavior a test writes.
pub struct ControlledInput {
    target: TargetId,
    capability: InputCapability,
    behavior: Mutex<Behavior>,
    cleanup: Mutex<Cleanup>,
    log: Arc<Mutex<Vec<SubmittedEvent>>>,
    admitted: Arc<Mutex<Vec<Admitted>>>,
    releases: Arc<Mutex<Vec<PressedState>>>,
    executing: Arc<AtomicUsize>,
}

impl ControlledInput {
    /// Builds a provider for `target` whose three operation kinds are supported
    /// through system input with system-admission evidence.
    #[must_use]
    pub fn new(target: TargetId) -> Self {
        let capability = InputCapability::none()
            .with_pair(
                InputOperationKind::Pointer,
                InputDelivery::System,
                CapabilitySupport::Supported,
                SubmissionEvidence::SystemInputAdmission,
            )
            .with_pointer_space(InputDelivery::System, CoordinateSpace::CapturePixels)
            .with_pointer_space(InputDelivery::System, CoordinateSpace::FrameNormalized)
            .with_permission(
                InputOperationKind::Pointer,
                InputDelivery::System,
                PermissionKind::InputControl,
            )
            .with_pair(
                InputOperationKind::Keyboard,
                InputDelivery::System,
                CapabilitySupport::Supported,
                SubmissionEvidence::SystemInputAdmission,
            )
            .with_permission(
                InputOperationKind::Keyboard,
                InputDelivery::System,
                PermissionKind::InputControl,
            )
            .with_pair(
                InputOperationKind::Text,
                InputDelivery::System,
                CapabilitySupport::Supported,
                SubmissionEvidence::SystemInputAdmission,
            )
            .with_permission(
                InputOperationKind::Text,
                InputDelivery::System,
                PermissionKind::InputControl,
            );
        Self::with_capability(target, capability)
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
            admitted: Arc::new(Mutex::new(Vec::new())),
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

    /// Returns every completely submitted logical event, in order.
    #[must_use]
    pub fn submitted_events(&self) -> Vec<SubmittedEvent> {
        self.log.lock().expect("uncontended").clone()
    }

    /// Returns every sequence admitted to native submission, in order.
    ///
    /// Preflight-refused routes do not appear here; their immutable attempt
    /// records remain on the receipt.
    #[must_use]
    pub fn admitted(&self) -> Vec<Admitted> {
        self.admitted.lock().expect("uncontended").clone()
    }

    /// Returns every pressed state cleanup released, in the order it released them.
    #[must_use]
    pub fn released(&self) -> Vec<PressedState> {
        self.releases.lock().expect("uncontended").clone()
    }

    /// Clears retained test observations without changing adapter behavior.
    ///
    /// Benchmark fixtures use this between samples so the double itself does not
    /// turn repeated input into unbounded retained history.
    pub fn clear_observations(&self) {
        self.log.lock().expect("uncontended").clear();
        self.admitted.lock().expect("uncontended").clear();
        self.releases.lock().expect("uncontended").clear();
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
            .field("submitted", &self.submitted_events().len())
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
            admitted: Arc::clone(&self.admitted),
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
    log: Arc<Mutex<Vec<SubmittedEvent>>>,
    admitted: Arc<Mutex<Vec<Admitted>>>,
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

    fn record(&self, route: InputDelivery, events: &[InputEvent]) {
        let mut log = self.log.lock().expect("uncontended");
        for event in events {
            log.push(SubmittedEvent {
                route,
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

    fn wait_delay(
        delay: Duration,
        operation: &OperationContext,
    ) -> std::result::Result<(), InputFault> {
        let end = operation
            .now()
            .checked_add(delay)
            .ok_or(InputFault::DeadlineExceeded)?;
        loop {
            Operation::admit(operation).map_err(InputFault::from)?;
            let now = operation.now();
            if now >= end {
                return Ok(());
            }
            thread::sleep(
                end.saturating_duration_since(now)
                    .min(Duration::from_millis(2)),
            );
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
        self.descriptor.validate(request)?;
        let _guard = self.admission.admit(operation)?;
        let _executing = ExecutingGuard::new(&self.executing);
        let target = request.target();
        let events = request.sequence().events();
        let behavior = self.behavior();
        let mut attempts = Vec::with_capacity(request.delivery().routes().len());
        let mut last_fault = InputFault::RouteUnavailable;

        for route in request.delivery().routes().iter().copied() {
            let evidence = match self.descriptor.preflight_route(request, route) {
                Ok(evidence) => evidence,
                Err(fault) => {
                    attempts.push(InputAttempt::refused(route, fault));
                    last_fault = fault;
                    continue;
                }
            };

            let route_behavior = match &behavior {
                Behavior::Refuse {
                    route: refused,
                    fault,
                } if *refused == route => {
                    attempts.push(InputAttempt::refused(route, *fault));
                    last_fault = *fault;
                    continue;
                }
                Behavior::Refuse { .. } => Behavior::Complete,
                Behavior::Unexecuted(fault) => {
                    attempts.push(InputAttempt::refused(route, *fault));
                    last_fault = *fault;
                    continue;
                }
                behavior => behavior.clone(),
            };

            self.admitted.lock().expect("uncontended").push(Admitted {
                selected_route: route,
                routes: request.delivery().routes().to_vec(),
                focus: request.focus(),
                geometry: request.pointer_geometry(),
                events: events.len(),
            });

            match route_behavior {
                Behavior::Complete => {
                    let mut submitted = 0usize;
                    for event in events {
                        let submission = match event {
                            InputEvent::Delay(delay) => Self::wait_delay(*delay, operation),
                            _ => Operation::admit(operation)
                                .map(|_| ())
                                .map_err(InputFault::from),
                        };
                        if let Err(fault) = submission {
                            if submitted == 0 {
                                attempts.push(InputAttempt::refused(route, fault));
                                return Ok(InputReceipt::unexecuted(target, fault)
                                    .with_prior_attempts(attempts));
                            }
                            let held = request.sequence().possibly_held_after(submitted, false);
                            let cleanup = self.run_cleanup(&held, request, operation);
                            return Ok(cleanup.apply(
                                InputReceipt::partial(
                                    target, route, evidence, submitted, false, fault,
                                )
                                .with_prior_attempts(attempts),
                            ));
                        }
                        self.record(route, std::slice::from_ref(event));
                        submitted += 1;
                    }

                    return Ok(InputReceipt::complete(target, route, evidence, submitted)
                        .with_prior_attempts(attempts));
                }
                Behavior::FailAfter { submitted, fault } => {
                    let submitted = submitted.min(events.len());
                    self.record(route, &events[..submitted]);
                    if submitted == 0 {
                        attempts.push(InputAttempt::refused(route, fault));
                        return Ok(
                            InputReceipt::unexecuted(target, fault).with_prior_attempts(attempts)
                        );
                    }
                    let held = request.sequence().possibly_held_after(submitted, false);
                    let cleanup = self.run_cleanup(&held, request, operation);
                    return Ok(cleanup.apply(
                        InputReceipt::partial(target, route, evidence, submitted, false, fault)
                            .with_prior_attempts(attempts),
                    ));
                }
                Behavior::FailDuring { submitted, fault } => {
                    let submitted = submitted.min(events.len().saturating_sub(1));
                    self.record(route, &events[..submitted]);
                    let held = request.sequence().possibly_held_after(submitted, true);
                    let cleanup = self.run_cleanup(&held, request, operation);
                    return Ok(cleanup.apply(
                        InputReceipt::partial(target, route, evidence, submitted, true, fault)
                            .with_prior_attempts(attempts),
                    ));
                }
                Behavior::Refuse { .. } | Behavior::Unexecuted(_) => unreachable!(),
            }
        }

        Ok(InputReceipt::unexecuted(target, last_fault).with_prior_attempts(attempts))
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
        CapabilitySupport, CoordinateSpace, IdentityIssuer, InputCapability, InputDelivery,
        InputOperationKind, Lifecycle, OperationContext, Point, ProviderId, Status,
        SubmissionEvidence, TargetId,
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
        assert_eq!(receipt.submitted(), 4);
        assert_eq!(receipt.cleanup(), CleanupState::NotNeeded);
        let submitted = provider.submitted_events();
        assert_eq!(submitted.len(), 4);
        assert_eq!(submitted[0].route, InputDelivery::System);
        assert_eq!(
            submitted[1].event,
            InputEvent::KeyPress(Key::Character('c'))
        );
    }

    #[test]
    fn a_partial_failure_reports_its_count_and_releases_what_it_pressed() {
        let target = target();
        let provider = ControlledInput::new(target);
        provider.set_behavior(Behavior::FailAfter {
            submitted: 2,
            fault: InputFault::SubmissionFailed,
        });
        let context = OperationContext::new();
        let controller = provider
            .open(target, &InputOpenRequest::new(), &context)
            .expect("opened");

        let receipt = controller
            .execute(&request(target, chord()), &context)
            .expect("executed");

        assert_eq!(receipt.outcome(), SequenceOutcome::Partial);
        assert_eq!(receipt.submitted(), 2);
        assert_eq!(receipt.last_submitted(), Some(1));
        assert_eq!(receipt.fault(), Some(InputFault::SubmissionFailed));
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
            submitted: 2,
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
    fn a_refused_route_falls_back_only_where_the_caller_permitted_it() {
        let target = target();
        let provider = ControlledInput::with_capability(
            target,
            InputCapability::none()
                .with_pair(
                    InputOperationKind::Pointer,
                    InputDelivery::System,
                    CapabilitySupport::Supported,
                    SubmissionEvidence::SystemInputAdmission,
                )
                .with_pointer_space(InputDelivery::System, CoordinateSpace::CapturePixels)
                .with_pair(
                    InputOperationKind::Pointer,
                    InputDelivery::WindowMessage,
                    CapabilitySupport::Unknown,
                    SubmissionEvidence::TargetQueueAdmission,
                )
                .with_pointer_space(InputDelivery::WindowMessage, CoordinateSpace::CapturePixels),
        );
        provider.set_behavior(Behavior::Refuse {
            route: InputDelivery::WindowMessage,
            fault: InputFault::RouteUnavailable,
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
                        InputDelivery::WindowMessage,
                        InputDelivery::System,
                    ])
                    .expect("valid"),
                ),
                &context,
            )
            .expect("executed");

        assert!(permitted.is_complete());
        assert_eq!(permitted.selected_route(), Some(InputDelivery::System));
        assert!(permitted.used_fallback());
        assert_eq!(permitted.attempts().len(), 2);
        assert_eq!(
            permitted.attempts()[0].fault(),
            Some(InputFault::RouteUnavailable)
        );

        let required = controller
            .execute(
                &InputRequest::new(
                    target,
                    sequence,
                    DeliveryPlan::require(InputDelivery::WindowMessage),
                ),
                &context,
            )
            .expect("executed");

        assert_eq!(required.outcome(), SequenceOutcome::Unexecuted);
        assert_eq!(required.submitted(), 0);
        assert_eq!(required.attempts().len(), 1);
        assert!(!required.used_fallback());
    }

    #[test]
    fn an_unsupported_combination_fails_admission_before_any_event() {
        let target = target();
        let provider = ControlledInput::with_capability(
            target,
            InputCapability::none()
                .with_pair(
                    InputOperationKind::Pointer,
                    InputDelivery::System,
                    CapabilitySupport::Supported,
                    SubmissionEvidence::SystemInputAdmission,
                )
                .with_pointer_space(InputDelivery::System, CoordinateSpace::CapturePixels),
        );
        let context = OperationContext::new();
        let controller = provider
            .open(target, &InputOpenRequest::new(), &context)
            .expect("opened");

        let receipt = controller
            .execute(&request(target, chord()), &context)
            .expect("well-formed refusal returns a receipt");

        assert_eq!(receipt.outcome(), SequenceOutcome::Unexecuted);
        assert_eq!(receipt.submitted(), 0);
        assert_eq!(
            receipt.attempts()[0].fault(),
            Some(InputFault::UnsupportedCombination)
        );
        assert!(
            provider.submitted_events().is_empty(),
            "preflight refused before native submission"
        );
    }

    #[test]
    fn a_partial_native_press_is_terminal_and_cleanup_is_conservative() {
        let target = target();
        let provider = ControlledInput::with_capability(
            target,
            InputCapability::none()
                .with_pair(
                    InputOperationKind::Pointer,
                    InputDelivery::WindowMessage,
                    CapabilitySupport::Unknown,
                    SubmissionEvidence::TargetQueueAdmission,
                )
                .with_pointer_space(InputDelivery::WindowMessage, CoordinateSpace::CapturePixels)
                .with_pair(
                    InputOperationKind::Pointer,
                    InputDelivery::System,
                    CapabilitySupport::Supported,
                    SubmissionEvidence::SystemInputAdmission,
                )
                .with_pointer_space(InputDelivery::System, CoordinateSpace::CapturePixels),
        );
        provider.set_behavior(Behavior::FailDuring {
            submitted: 0,
            fault: InputFault::SubmissionFailed,
        });
        let context = OperationContext::new();
        let controller = provider
            .open(target, &InputOpenRequest::new(), &context)
            .expect("opened");
        let sequence = InputSequence::new(vec![InputEvent::PointerPress(PointerButton::Primary)])
            .expect("valid");

        let receipt = controller
            .execute(
                &InputRequest::new(
                    target,
                    sequence,
                    DeliveryPlan::ordered(vec![
                        InputDelivery::WindowMessage,
                        InputDelivery::System,
                    ])
                    .expect("valid"),
                ),
                &context,
            )
            .expect("executed");

        assert_eq!(receipt.outcome(), SequenceOutcome::Partial);
        assert_eq!(receipt.submitted(), 0);
        assert_eq!(receipt.last_submitted(), None);
        assert!(receipt.partial_native_effect());
        assert_eq!(receipt.selected_route(), Some(InputDelivery::WindowMessage));
        assert_eq!(receipt.attempts().len(), 1, "fallback is forbidden");
        assert_eq!(receipt.cleanup_owed(), 1);
        assert_eq!(receipt.cleanup_released(), 1);
        assert_eq!(
            provider.released(),
            [PressedState::Button(PointerButton::Primary)]
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
        assert!(provider.submitted_events().is_empty());
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

        assert_eq!(provider.submitted_events().len(), 2);
        drop(Arc::clone(&first));
    }
}
