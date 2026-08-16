//! macOS implementation of the platform-neutral input controller contract.
//!
//! # Why this repeats the Windows controller
//!
//! The sequence loop, the receipt accounting, and the cleanup walk read almost
//! exactly like `mado-pilot-platform-windows`. That is deliberate for now: the two
//! Adapters agree because they implement one contract, and hoisting the shared
//! shape would edit a merged Adapter and a contract package this Change does not
//! own. The pieces that genuinely cannot be shared are already separated below —
//! the capability table, the pressed-state records, and the driver seam.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use mado_pilot_capture::Frame;
use mado_pilot_core::{
    CapabilitySupport, CoordinateSpace, FrameOrder, FrameStamp, InputCapability, InputDelivery,
    InputOperationKind, Lifecycle, OperationContext, PermissionKind, PixelExtent, StreamId,
    SubmissionEvidence, TargetKind, TargetPlacement, TransformSnapshot,
};
use mado_pilot_input::{
    Admission, FocusPolicy, InputAttempt, InputController, InputDescriptor, InputEvent, InputFault,
    InputReceipt, InputRequest, Key, PointerButton, PointerGeometry, PressedState,
};

use crate::native_input::NativeInputDriver;
use crate::provider::TargetRecord;
use crate::shim::ProcessEventSource;

const DELAY_POLL_INTERVAL: Duration = Duration::from_millis(2);

/// Returns the pairwise input contract for a discovered macOS target.
///
/// macOS always exposes `System` input. A qualifying window additionally
/// advertises process-scoped pointer, keyboard, and text as `Unknown` until the
/// operation pair passes native qualification. Both routes carry
/// invocation-only evidence; Core Graphics does not acknowledge consumption.
/// Displays accept system pointer input only.
pub(crate) fn input_capability(kind: TargetKind, process_directed: bool) -> InputCapability {
    let capability = InputCapability::none()
        .with_pair(
            InputOperationKind::Pointer,
            InputDelivery::System,
            CapabilitySupport::Supported,
            SubmissionEvidence::InvocationOnly,
        )
        .with_pointer_space(InputDelivery::System, CoordinateSpace::CapturePixels)
        .with_pointer_space(InputDelivery::System, CoordinateSpace::FrameNormalized)
        .with_pointer_space(InputDelivery::System, CoordinateSpace::TargetNormalized)
        .with_pointer_space(InputDelivery::System, CoordinateSpace::TargetLogical)
        .with_pointer_space(InputDelivery::System, CoordinateSpace::DesktopLogical)
        .with_permission(
            InputOperationKind::Pointer,
            InputDelivery::System,
            PermissionKind::InputControl,
        );

    if kind != TargetKind::Window {
        return capability;
    }
    let capability = capability
        .with_focus_required(InputOperationKind::Pointer, InputDelivery::System)
        .with_pair(
            InputOperationKind::Keyboard,
            InputDelivery::System,
            CapabilitySupport::Supported,
            SubmissionEvidence::InvocationOnly,
        )
        .with_focus_required(InputOperationKind::Keyboard, InputDelivery::System)
        .with_permission(
            InputOperationKind::Keyboard,
            InputDelivery::System,
            PermissionKind::InputControl,
        )
        .with_pair(
            InputOperationKind::Text,
            InputDelivery::System,
            CapabilitySupport::Supported,
            SubmissionEvidence::InvocationOnly,
        )
        .with_focus_required(InputOperationKind::Text, InputDelivery::System)
        .with_permission(
            InputOperationKind::Text,
            InputDelivery::System,
            PermissionKind::InputControl,
        );
    if !process_directed {
        return capability;
    }

    capability
        .with_pair(
            InputOperationKind::Pointer,
            InputDelivery::ProcessDirected,
            CapabilitySupport::Unknown,
            SubmissionEvidence::InvocationOnly,
        )
        .with_pointer_space(
            InputDelivery::ProcessDirected,
            CoordinateSpace::CapturePixels,
        )
        .with_pointer_space(
            InputDelivery::ProcessDirected,
            CoordinateSpace::FrameNormalized,
        )
        .with_pointer_space(
            InputDelivery::ProcessDirected,
            CoordinateSpace::TargetNormalized,
        )
        .with_pointer_space(
            InputDelivery::ProcessDirected,
            CoordinateSpace::TargetLogical,
        )
        .with_pointer_space(
            InputDelivery::ProcessDirected,
            CoordinateSpace::DesktopLogical,
        )
        .with_permission(
            InputOperationKind::Pointer,
            InputDelivery::ProcessDirected,
            PermissionKind::InputControl,
        )
        .with_pair(
            InputOperationKind::Keyboard,
            InputDelivery::ProcessDirected,
            CapabilitySupport::Unknown,
            SubmissionEvidence::InvocationOnly,
        )
        .with_permission(
            InputOperationKind::Keyboard,
            InputDelivery::ProcessDirected,
            PermissionKind::InputControl,
        )
        .with_pair(
            InputOperationKind::Text,
            InputDelivery::ProcessDirected,
            CapabilitySupport::Unknown,
            SubmissionEvidence::InvocationOnly,
        )
        .with_permission(
            InputOperationKind::Text,
            InputDelivery::ProcessDirected,
            PermissionKind::InputControl,
        )
}

/// The latest authoritative transform for every live capture stream of a target.
///
/// Geometry revisions make an older frame in the same revision equivalent to the
/// latest transform in that revision. A frame from an older revision is not
/// reconstructed after movement or resize: the request is refused instead.
#[derive(Debug, Default)]
pub(crate) struct GeometryLedger {
    streams: Mutex<HashMap<StreamId, GeometryEntry>>,
}

#[derive(Debug, Clone, Copy)]
struct GeometryEntry {
    stamp: FrameStamp,
    transform: TransformSnapshot,
}

impl GeometryLedger {
    pub(crate) fn publish(&self, frame: &Frame) {
        self.record(frame.stamp(), *frame.transform());
    }

    fn record(&self, stamp: FrameStamp, transform: TransformSnapshot) {
        self.streams
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(stamp.stream(), GeometryEntry { stamp, transform });
    }

    /// Retires the entry a finished stream left behind.
    ///
    /// A target record outlives the sessions opened on it, and each open mints a
    /// fresh stream identity, so without this the map would grow by one entry per
    /// open for as long as the record is retained.
    pub(crate) fn remove(&self, stream: StreamId) {
        self.streams
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&stream);
    }

    pub(crate) fn source_transform(&self, source: FrameStamp) -> Option<TransformSnapshot> {
        let entry = *self
            .streams
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&source.stream())?;
        if entry.stamp.epoch() != source.epoch()
            || entry.stamp.geometry() != source.geometry()
            || !matches!(
                source.order(&entry.stamp),
                Ok(FrameOrder::Before | FrameOrder::Same)
            )
        {
            return None;
        }
        Some(entry.transform)
    }
}

/// The shape and place a pointer coordinate was resolved against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GeometryFingerprint {
    pub(crate) extent: PixelExtent,
    pub(crate) placement: TargetPlacement,
}

/// Where the pointer is, in the global point space, and under which geometry.
///
/// Points rather than integers, because the macOS desktop plane is measured in
/// points and Core Graphics accepts a fractional location. Rounding here would
/// move a Retina click by half a capture pixel for no reason.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PointerState {
    pub(crate) desktop: (f64, f64),
    pub(crate) geometry: GeometryFingerprint,
}

/// What this sequence pressed and has not released, with its native resolution.
#[derive(Debug, Default)]
pub(crate) struct DriverState {
    pub(crate) pointer: Option<PointerState>,
    pub(crate) keys: Vec<SystemKeyState>,
    pub(crate) buttons: Vec<SystemButtonState>,
    pub(crate) pending_text_release: Option<PendingTextRelease>,
    pub(crate) process_event_source: Option<ProcessEventSource>,
}

/// A synthesized text key-down whose matching key-up did not run.
///
/// The payload is deliberately absent: a key-up for virtual key zero balances
/// the native state without retaining or re-emitting caller text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PendingTextRelease {
    pub(crate) route: InputDelivery,
    pub(crate) flags: u32,
}

impl DriverState {
    /// Returns the native modifier flags this sequence is currently holding.
    ///
    /// Only the modifiers this sequence pressed. A synthesized event carries
    /// exactly these, so a caller that asked for a plain keystroke gets one even
    /// while the user is holding a modifier of their own.
    pub(crate) fn held_flags(&self) -> u32 {
        self.keys
            .iter()
            .filter_map(|pressed| match pressed.logical {
                Key::Modifier(modifier) => Some(crate::native_input::modifier_flag(modifier)),
                _ => None,
            })
            .fold(0, |flags, flag| flags | flag)
    }

    /// Returns the button a move should be reported as dragging, if any.
    pub(crate) fn dragging(&self) -> Option<PointerButton> {
        self.buttons.last().map(|pressed| pressed.logical)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SystemKeyState {
    pub(crate) logical: Key,
    pub(crate) key_code: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SystemButtonState {
    pub(crate) logical: PointerButton,
    pub(crate) native: u32,
}

/// Why native submission stopped at one logical event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SubmissionFailure {
    pub(crate) fault: InputFault,
    pub(crate) current_event_may_have_effect: bool,
    /// Native units from the current logical event whose invocation completed.
    pub(crate) invoked_native_units: usize,
}

impl SubmissionFailure {
    pub(crate) const fn before_event(fault: InputFault) -> Self {
        Self {
            fault,
            current_event_may_have_effect: false,
            invoked_native_units: 0,
        }
    }

    pub(crate) const fn after_native_units(fault: InputFault, invoked_native_units: usize) -> Self {
        Self {
            fault,
            current_event_may_have_effect: invoked_native_units != 0,
            invoked_native_units,
        }
    }

    pub(crate) const fn after_native_attempt(
        fault: InputFault,
        invoked_native_units: usize,
        current_event_may_have_effect: bool,
    ) -> Self {
        Self {
            fault,
            current_event_may_have_effect: current_event_may_have_effect
                || invoked_native_units != 0,
            invoked_native_units,
        }
    }
}

impl From<InputFault> for SubmissionFailure {
    fn from(fault: InputFault) -> Self {
        Self::before_event(fault)
    }
}

struct StoppedSubmission {
    route: InputDelivery,
    evidence: SubmissionEvidence,
    prior_attempts: Vec<InputAttempt>,
    submitted: usize,
    failure: SubmissionFailure,
}

/// The native seam driven by the controller.
pub(crate) trait InputDriver: fmt::Debug + Send + Sync {
    /// Refuses a route before it is selected.
    ///
    /// `require_early_authority` is true when the controller still needs a
    /// zero-effect decision at this boundary: either a later caller-ordered
    /// route may be tried, or the sequence contains no native event whose final
    /// commit gate could perform the check. A terminal route with at least one
    /// native event may defer expensive mutable target authority to that final
    /// gate without changing fallback semantics.
    fn preflight(
        &self,
        route: InputDelivery,
        focus: FocusPolicy,
        require_early_authority: bool,
        operation: &OperationContext,
    ) -> Result<(), InputFault>;

    /// Creates route-private state after preflight and before route selection is
    /// committed. Failure is still eligible for caller-ordered fallback because
    /// no event has been submitted.
    fn begin_route(
        &self,
        _route: InputDelivery,
        _state: &mut DriverState,
        _operation: &OperationContext,
    ) -> Result<(), InputFault> {
        Ok(())
    }

    /// Submits one logical event or reports whether its native representation may
    /// have had effect.
    fn submit(
        &self,
        route: InputDelivery,
        focus: FocusPolicy,
        event: &InputEvent,
        geometry: PointerGeometry,
        state: &mut DriverState,
        operation: &OperationContext,
    ) -> Result<(), SubmissionFailure>;

    fn release(
        &self,
        route: InputDelivery,
        pressed: PressedState,
        state: &mut DriverState,
        operation: &OperationContext,
    ) -> Result<(), InputFault>;

    /// Releases a route-private native state that has no public
    /// [`PressedState`] representation.
    ///
    /// Returns `true` only when one pending state was released. The default is
    /// correct for adapters and test doubles that create no such state.
    fn release_pending(
        &self,
        _route: InputDelivery,
        _state: &mut DriverState,
        _operation: &OperationContext,
    ) -> Result<bool, InputFault> {
        Ok(false)
    }
}

pub(crate) struct MacosInputController {
    descriptor: InputDescriptor,
    driver: Arc<dyn InputDriver>,
    admission: Admission,
}

impl MacosInputController {
    pub(crate) fn new(record: Arc<TargetRecord>, descriptor: InputDescriptor) -> Arc<Self> {
        Arc::new(Self {
            descriptor,
            driver: Arc::new(NativeInputDriver::new(record)),
            admission: Admission::new(),
        })
    }

    #[cfg(test)]
    pub(crate) fn with_driver(descriptor: InputDescriptor, driver: Arc<dyn InputDriver>) -> Self {
        Self {
            descriptor,
            driver,
            admission: Admission::new(),
        }
    }

    fn execute_inner(
        &self,
        request: &InputRequest,
        operation: &OperationContext,
    ) -> mado_pilot_core::Result<InputReceipt> {
        self.descriptor.validate(request)?;
        let _guard = self.admission.admit(operation)?;
        let mut state = DriverState::default();
        let (route, evidence, prior_attempts) =
            match self.select_route(request, &mut state, operation) {
                Ok(selected) => selected,
                Err(receipt) => return Ok(receipt),
            };

        let mut submitted = 0usize;
        for event in request.sequence().events() {
            let result = if let InputEvent::Delay(delay) = event {
                wait_delay(*delay, operation).map_err(SubmissionFailure::from)
            } else {
                match operation.interruption() {
                    Some(interruption) => Err(SubmissionFailure::before_event(InputFault::from(
                        interruption,
                    ))),
                    None => self.driver.submit(
                        route,
                        request.focus(),
                        event,
                        request.pointer_geometry(),
                        &mut state,
                        operation,
                    ),
                }
            };
            if let Err(failure) = result {
                return Ok(self.stopped_receipt(
                    request,
                    StoppedSubmission {
                        route,
                        evidence,
                        prior_attempts,
                        submitted,
                        failure,
                    },
                    &mut state,
                    operation,
                ));
            }
            submitted += 1;
        }
        Ok(
            InputReceipt::complete(request.target(), route, evidence, submitted)
                .with_prior_attempts(prior_attempts),
        )
    }

    fn select_route(
        &self,
        request: &InputRequest,
        state: &mut DriverState,
        operation: &OperationContext,
    ) -> Result<(InputDelivery, SubmissionEvidence, Vec<InputAttempt>), InputReceipt> {
        let target = request.target();
        let mut prior_attempts = Vec::with_capacity(request.delivery().routes().len());
        let mut last_fault = InputFault::RouteUnavailable;

        let routes = request.delivery().routes();
        let has_native_event = request
            .sequence()
            .events()
            .iter()
            .any(|event| !matches!(event, InputEvent::Delay(_)));
        for (index, route) in routes.iter().copied().enumerate() {
            let evidence = match self.descriptor.preflight_route(request, route) {
                Ok(evidence) => evidence,
                Err(fault) => {
                    prior_attempts.push(InputAttempt::refused(route, fault));
                    last_fault = fault;
                    continue;
                }
            };
            let require_early_authority = index + 1 < routes.len() || !has_native_event;
            let prepared = self
                .driver
                .preflight(route, request.focus(), require_early_authority, operation)
                .and_then(|()| self.driver.begin_route(route, state, operation));
            match prepared {
                Ok(()) => return Ok((route, evidence, prior_attempts)),
                Err(fault) => {
                    prior_attempts.push(InputAttempt::refused(route, fault));
                    last_fault = fault;
                    *state = DriverState::default();
                    let early_process_target_loss_allows_fallback = matches!(
                        (route, fault),
                        (InputDelivery::ProcessDirected, InputFault::TargetLost)
                    ) && index + 1 < routes.len();
                    if matches!(
                        fault,
                        InputFault::Cancelled
                            | InputFault::DeadlineExceeded
                            | InputFault::TargetLost
                            | InputFault::ControllerClosed
                    ) && !early_process_target_loss_allows_fallback
                    {
                        break;
                    }
                }
            }
        }

        Err(InputReceipt::unexecuted(target, last_fault).with_prior_attempts(prior_attempts))
    }

    fn stopped_receipt(
        &self,
        request: &InputRequest,
        mut stopped: StoppedSubmission,
        state: &mut DriverState,
        operation: &OperationContext,
    ) -> InputReceipt {
        if stopped.submitted == 0 && !stopped.failure.current_event_may_have_effect {
            stopped
                .prior_attempts
                .push(InputAttempt::refused(stopped.route, stopped.failure.fault));
            return InputReceipt::unexecuted(request.target(), stopped.failure.fault)
                .with_prior_attempts(stopped.prior_attempts);
        }

        let prior_attempts = std::mem::take(&mut stopped.prior_attempts);
        let receipt = InputReceipt::partial(
            request.target(),
            stopped.route,
            stopped.evidence,
            stopped.submitted,
            stopped.failure.current_event_may_have_effect,
            stopped.failure.fault,
        )
        .with_prior_attempts(prior_attempts);
        self.run_cleanup(receipt, request, &stopped, state, operation)
    }

    fn run_cleanup(
        &self,
        receipt: InputReceipt,
        request: &InputRequest,
        stopped: &StoppedSubmission,
        state: &mut DriverState,
        operation: &OperationContext,
    ) -> InputReceipt {
        let held = request.sequence().possibly_held_after(
            stopped.submitted,
            stopped.failure.current_event_may_have_effect,
        );
        let pending = usize::from(state.pending_text_release.is_some());
        let owed = pending + held.len();
        if owed == 0 {
            return receipt.with_cleanup(0, 0);
        }
        let budget = request.cleanup_budget();
        let cleanup = budget.context(operation);
        let mut released = 0usize;
        let mut exhausted = false;
        if pending != 0 {
            if released >= budget.max_events() || cleanup.interruption().is_some() {
                exhausted = true;
            } else {
                match self.driver.release_pending(stopped.route, state, &cleanup) {
                    Ok(true) => released += 1,
                    Ok(false) | Err(_) => return receipt.with_cleanup(released, owed),
                }
            }
        }
        if !exhausted {
            for pressed in &held {
                if released >= budget.max_events() || cleanup.interruption().is_some() {
                    exhausted = true;
                    break;
                }
                if self
                    .driver
                    .release(stopped.route, *pressed, state, &cleanup)
                    .is_err()
                {
                    break;
                }
                released += 1;
            }
        }
        if exhausted {
            receipt.with_exhausted_cleanup(released, owed)
        } else {
            receipt.with_cleanup(released, owed)
        }
    }
}

impl fmt::Debug for MacosInputController {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MacosInputController")
            .field("target", &self.descriptor.target())
            .field("lifecycle", &self.admission.lifecycle())
            .finish()
    }
}

impl InputController for MacosInputController {
    fn descriptor(&self) -> InputDescriptor {
        self.descriptor.clone()
    }

    fn execute(
        &self,
        request: &InputRequest,
        operation: &OperationContext,
    ) -> mado_pilot_core::Result<InputReceipt> {
        self.execute_inner(request, operation)
    }

    fn close(&self, operation: &OperationContext) -> mado_pilot_core::Result<()> {
        self.admission.drain(operation)
    }

    fn lifecycle(&self) -> Lifecycle {
        self.admission.lifecycle()
    }
}

fn wait_delay(delay: Duration, operation: &OperationContext) -> Result<(), InputFault> {
    if let Some(interruption) = operation.interruption() {
        return Err(InputFault::from(interruption));
    }
    let end = operation
        .now()
        .checked_add(delay)
        .ok_or(InputFault::DeadlineExceeded)?;
    loop {
        if let Some(interruption) = operation.interruption() {
            return Err(InputFault::from(interruption));
        }
        let now = operation.now();
        if now >= end {
            return match operation.interruption() {
                Some(interruption) => Err(InputFault::from(interruption)),
                None => Ok(()),
            };
        }
        thread::sleep(end.saturating_duration_since(now).min(DELAY_POLL_INTERVAL));
    }
}

#[cfg(test)]
mod tests;
