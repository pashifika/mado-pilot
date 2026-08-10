//! Windows implementation of the platform-neutral input controller contract.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use mado_pilot_capture::Frame;
use mado_pilot_core::{
    CapabilitySupport, CoordinateSpace, FrameOrder, FrameStamp, InputCapability, InputDelivery,
    InputOperationKind, Lifecycle, OperationContext, PixelExtent, StreamId, SubmissionEvidence,
    TargetKind, TargetPlacement, TransformSnapshot,
};
use mado_pilot_input::{
    Admission, CleanupBudget, FocusPolicy, InputAttempt, InputController, InputDescriptor,
    InputEvent, InputFault, InputReceipt, InputRequest, Key, PointerButton, PointerGeometry,
    PressedState,
};

use crate::fixture_protocol::CLASS_NAME;
use crate::native_input::NativeInputDriver;
use crate::provider::TargetRecord;

const DELAY_POLL_INTERVAL: Duration = Duration::from_millis(2);

pub(crate) fn input_capability(
    kind: TargetKind,
    class_name: Option<&str>,
    window_message_authority: bool,
) -> InputCapability {
    let mut capability = InputCapability::none().with_pair(
        InputOperationKind::Pointer,
        InputDelivery::System,
        CapabilitySupport::Supported,
        SubmissionEvidence::SystemInputAdmission,
    );
    for space in [
        CoordinateSpace::CapturePixels,
        CoordinateSpace::FrameNormalized,
        CoordinateSpace::TargetNormalized,
        CoordinateSpace::TargetLogical,
        CoordinateSpace::DesktopLogical,
    ] {
        capability = capability.with_pointer_space(InputDelivery::System, space);
    }

    if kind == TargetKind::Window {
        capability = capability
            .with_focus_required(InputOperationKind::Pointer, InputDelivery::System)
            .with_pair(
                InputOperationKind::Keyboard,
                InputDelivery::System,
                CapabilitySupport::Supported,
                SubmissionEvidence::SystemInputAdmission,
            )
            .with_focus_required(InputOperationKind::Keyboard, InputDelivery::System)
            .with_pair(
                InputOperationKind::Text,
                InputDelivery::System,
                CapabilitySupport::Supported,
                SubmissionEvidence::SystemInputAdmission,
            )
            .with_focus_required(InputOperationKind::Text, InputDelivery::System);
        if window_message_authority {
            let (support, evidence) = if class_name == Some(CLASS_NAME) {
                (
                    CapabilitySupport::Supported,
                    SubmissionEvidence::TargetProtocolAcknowledgement,
                )
            } else {
                (
                    CapabilitySupport::Unknown,
                    SubmissionEvidence::TargetQueueAdmission,
                )
            };
            for operation in InputOperationKind::ALL {
                capability = capability.with_pair(
                    operation,
                    InputDelivery::WindowMessage,
                    support,
                    evidence,
                );
            }
            for space in [
                CoordinateSpace::CapturePixels,
                CoordinateSpace::FrameNormalized,
                CoordinateSpace::TargetNormalized,
                CoordinateSpace::TargetLogical,
                CoordinateSpace::DesktopLogical,
            ] {
                capability = capability.with_pointer_space(InputDelivery::WindowMessage, space);
            }
        }
    }
    capability
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

    pub(crate) fn remove(&self, stream: StreamId) {
        self.streams
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&stream);
    }

    #[cfg(test)]
    pub(crate) fn source_transform(&self, source: FrameStamp) -> Option<TransformSnapshot> {
        self.resolve_source_transform(source).ok()
    }

    pub(crate) fn resolve_source_transform(
        &self,
        source: FrameStamp,
    ) -> Result<TransformSnapshot, InputFault> {
        let entry = *self
            .streams
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&source.stream())
            .ok_or(InputFault::UnsupportedCoordinate)?;
        if entry.stamp.epoch() != source.epoch() || entry.stamp.geometry() != source.geometry() {
            return Err(InputFault::GeometryChanged);
        }
        if !matches!(
            source.order(&entry.stamp),
            Ok(FrameOrder::Before | FrameOrder::Same)
        ) {
            return Err(InputFault::UnsupportedCoordinate);
        }
        Ok(entry.transform)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GeometryFingerprint {
    pub(crate) extent: PixelExtent,
    pub(crate) placement: TargetPlacement,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PointerState {
    pub(crate) screen: (i32, i32),
    pub(crate) geometry: GeometryFingerprint,
}

#[derive(Debug, Default)]
pub(crate) struct DriverState {
    pub(crate) pointer: Option<PointerState>,
    pub(crate) keys: Vec<SystemKeyState>,
    pub(crate) buttons: Vec<SystemButtonState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SystemKeyState {
    pub(crate) logical: Key,
    pub(crate) virtual_key: u16,
    pub(crate) scan_code: u8,
    pub(crate) extended: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SystemButtonState {
    pub(crate) logical: PointerButton,
    pub(crate) physical: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SubmissionFailure {
    pub(crate) fault: InputFault,
    pub(crate) current_event_may_have_effect: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SubmissionContexts<'operation> {
    pub(crate) operation: &'operation OperationContext,
    pub(crate) cleanup_budget: CleanupBudget,
}

impl SubmissionFailure {
    pub(crate) const fn before_event(fault: InputFault) -> Self {
        Self {
            fault,
            current_event_may_have_effect: false,
        }
    }

    pub(crate) const fn during_event(fault: InputFault) -> Self {
        Self {
            fault,
            current_event_may_have_effect: true,
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

pub(crate) trait InputDriver: fmt::Debug + Send + Sync {
    fn preflight(
        &self,
        route: InputDelivery,
        focus: FocusPolicy,
        operation: &OperationContext,
    ) -> Result<(), InputFault>;

    fn submit(
        &self,
        route: InputDelivery,
        focus: FocusPolicy,
        event: &InputEvent,
        geometry: PointerGeometry,
        state: &mut DriverState,
        contexts: SubmissionContexts<'_>,
    ) -> Result<(), SubmissionFailure>;

    fn release(
        &self,
        route: InputDelivery,
        pressed: PressedState,
        state: &mut DriverState,
        operation: &OperationContext,
    ) -> Result<(), InputFault>;
}

pub(crate) struct WindowsInputController {
    descriptor: InputDescriptor,
    driver: Arc<dyn InputDriver>,
    admission: Admission,
}

impl WindowsInputController {
    pub(crate) fn new(record: Arc<TargetRecord>) -> Arc<Self> {
        let descriptor = record.input_descriptor();
        Arc::new(Self {
            descriptor,
            driver: Arc::new(NativeInputDriver::new(record)),
            admission: Admission::new(),
        })
    }

    #[cfg(test)]
    fn with_driver(descriptor: InputDescriptor, driver: Arc<dyn InputDriver>) -> Self {
        Self {
            descriptor,
            driver,
            admission: Admission::new(),
        }
    }

    fn select_route(
        &self,
        request: &InputRequest,
        operation: &OperationContext,
    ) -> Result<(InputDelivery, SubmissionEvidence, Vec<InputAttempt>), InputReceipt> {
        let target = request.target();
        let mut prior_attempts = Vec::with_capacity(request.delivery().routes().len());
        let mut last_fault = InputFault::RouteUnavailable;

        for route in request.delivery().routes().iter().copied() {
            let evidence = match self.descriptor.preflight_route(request, route) {
                Ok(evidence) => evidence,
                Err(fault) => {
                    prior_attempts.push(InputAttempt::refused(route, fault));
                    last_fault = fault;
                    continue;
                }
            };
            match self.driver.preflight(route, request.focus(), operation) {
                Ok(()) => return Ok((route, evidence, prior_attempts)),
                Err(fault) => {
                    prior_attempts.push(InputAttempt::refused(route, fault));
                    last_fault = fault;
                    if matches!(
                        fault,
                        InputFault::Cancelled
                            | InputFault::DeadlineExceeded
                            | InputFault::TargetLost
                            | InputFault::ControllerClosed
                    ) {
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
        if held.is_empty() {
            return receipt.with_cleanup(0, 0);
        }
        let budget = request.cleanup_budget();
        let cleanup = budget.context(operation);
        let mut released = 0usize;
        let mut exhausted = false;
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
        if exhausted {
            receipt.with_exhausted_cleanup(released, held.len())
        } else {
            receipt.with_cleanup(released, held.len())
        }
    }
}

impl fmt::Debug for WindowsInputController {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WindowsInputController")
            .field("target", &self.descriptor.target())
            .field("lifecycle", &self.admission.lifecycle())
            .finish()
    }
}

impl InputController for WindowsInputController {
    fn descriptor(&self) -> InputDescriptor {
        self.descriptor.clone()
    }

    fn execute(
        &self,
        request: &InputRequest,
        operation: &OperationContext,
    ) -> mado_pilot_core::Result<InputReceipt> {
        self.descriptor.validate(request)?;
        let _guard = self.admission.admit(operation)?;
        let (route, evidence, prior_attempts) = match self.select_route(request, operation) {
            Ok(selected) => selected,
            Err(receipt) => return Ok(receipt),
        };

        let mut submitted = 0usize;
        let mut state = DriverState::default();
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
                        SubmissionContexts {
                            operation,
                            cleanup_budget: request.cleanup_budget(),
                        },
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
