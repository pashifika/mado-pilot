//! Windows implementation of the platform-neutral input controller contract.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use mado_pilot_capture::Frame;
use mado_pilot_core::{
    CoordinateSpace, FrameOrder, FrameStamp, InputCapability, InputDelivery, InputOperationKind,
    Lifecycle, OperationContext, PixelExtent, StreamId, TargetKind, TargetPlacement,
    TransformSnapshot,
};
use mado_pilot_input::{
    Admission, CleanupBudget, FocusPolicy, InputController, InputDescriptor, InputEvent,
    InputFault, InputReceipt, InputRequest, Key, PointerButton, PointerGeometry, PressedState,
};

use crate::fixture_protocol::CLASS_NAME;
use crate::native_input::NativeInputDriver;
use crate::provider::TargetRecord;

const DELAY_POLL_INTERVAL: Duration = Duration::from_millis(2);

pub(crate) fn input_capability(kind: TargetKind, class_name: Option<&str>) -> InputCapability {
    let mut capability = InputCapability::none()
        .with_pair(InputOperationKind::Pointer, InputDelivery::System)
        .with_pointer_space(CoordinateSpace::CapturePixels)
        .with_pointer_space(CoordinateSpace::FrameNormalized)
        .with_pointer_space(CoordinateSpace::TargetNormalized)
        .with_pointer_space(CoordinateSpace::TargetLogical)
        .with_pointer_space(CoordinateSpace::DesktopLogical);

    if kind == TargetKind::Window {
        capability = capability
            .with_pair(InputOperationKind::Keyboard, InputDelivery::System)
            .with_pair(InputOperationKind::Text, InputDelivery::System)
            .with_focus_required(InputDelivery::System);
        if class_name == Some(CLASS_NAME) {
            capability = capability
                .with_pair(InputOperationKind::Pointer, InputDelivery::BackgroundTarget)
                .with_pair(
                    InputOperationKind::Keyboard,
                    InputDelivery::BackgroundTarget,
                )
                .with_pair(InputOperationKind::Text, InputDelivery::BackgroundTarget);
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
    pub(crate) extended: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SystemButtonState {
    pub(crate) logical: PointerButton,
    pub(crate) physical: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DeliveryFailure {
    pub(crate) fault: InputFault,
    pub(crate) current_event_may_have_effect: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DeliveryContexts<'operation> {
    pub(crate) operation: &'operation OperationContext,
    pub(crate) cleanup_budget: CleanupBudget,
}

impl DeliveryFailure {
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

impl From<InputFault> for DeliveryFailure {
    fn from(fault: InputFault) -> Self {
        Self::before_event(fault)
    }
}

struct StoppedDelivery {
    delivery: InputDelivery,
    attempted: Vec<InputDelivery>,
    delivered: usize,
    failure: DeliveryFailure,
}

pub(crate) trait InputDriver: fmt::Debug + Send + Sync {
    fn preflight(
        &self,
        delivery: InputDelivery,
        focus: FocusPolicy,
        operation: &OperationContext,
    ) -> Result<(), InputFault>;

    fn deliver(
        &self,
        delivery: InputDelivery,
        focus: FocusPolicy,
        event: &InputEvent,
        geometry: PointerGeometry,
        state: &mut DriverState,
        contexts: DeliveryContexts<'_>,
    ) -> Result<(), DeliveryFailure>;

    fn release(
        &self,
        delivery: InputDelivery,
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

    fn select_delivery(
        &self,
        request: &InputRequest,
        operation: &OperationContext,
    ) -> Result<(InputDelivery, Vec<InputDelivery>), InputReceipt> {
        let target = request.target();
        let first = match self.descriptor.admit(request) {
            Ok(first) => first,
            Err(fault) => return Err(InputReceipt::unexecuted(target, fault)),
        };
        let capability = self.descriptor.capability();
        let mut reached_first = false;
        let mut attempted = Vec::new();
        let mut last_fault = InputFault::DeliveryUnavailable;

        for candidate in request.delivery().modes().iter().copied() {
            if !reached_first {
                reached_first = candidate == first;
                if !reached_first {
                    continue;
                }
            }
            if !request.sequence().supported_by(capability, candidate) {
                continue;
            }
            if capability.requires_focus(candidate) && request.focus() == FocusPolicy::Preserve {
                last_fault = InputFault::FocusRequired;
                continue;
            }
            attempted.push(candidate);
            match self.driver.preflight(candidate, request.focus(), operation) {
                Ok(()) => return Ok((candidate, attempted)),
                Err(fault) => {
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

        Err(InputReceipt::unexecuted(target, last_fault).with_attempted(attempted))
    }

    fn stopped_receipt(
        &self,
        request: &InputRequest,
        stopped: StoppedDelivery,
        state: &mut DriverState,
        operation: &OperationContext,
    ) -> InputReceipt {
        let receipt = if stopped.delivered == 0 && !stopped.failure.current_event_may_have_effect {
            InputReceipt::unexecuted(request.target(), stopped.failure.fault)
        } else {
            InputReceipt::partial(
                request.target(),
                stopped.delivery,
                stopped.delivered,
                stopped.failure.fault,
            )
        }
        .with_attempted(stopped.attempted);
        self.run_cleanup(
            receipt,
            request,
            stopped.delivery,
            stopped.delivered,
            state,
            operation,
        )
    }

    fn run_cleanup(
        &self,
        receipt: InputReceipt,
        request: &InputRequest,
        delivery: InputDelivery,
        delivered: usize,
        state: &mut DriverState,
        operation: &OperationContext,
    ) -> InputReceipt {
        let held = request.sequence().held_after(delivered);
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
                .release(delivery, *pressed, state, &cleanup)
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
        // Static admission precedes the serialization claim, so an invalid
        // request never waits behind a valid sequence.
        self.descriptor.admit(request)?;
        let _guard = self.admission.admit(operation)?;
        let (delivery, attempted) = match self.select_delivery(request, operation) {
            Ok(selected) => selected,
            Err(receipt) => return Ok(receipt),
        };

        let mut delivered = 0usize;
        let mut state = DriverState::default();
        for event in request.sequence().events() {
            let result = if let InputEvent::Delay(delay) = event {
                wait_delay(*delay, operation).map_err(DeliveryFailure::from)
            } else {
                match operation.interruption() {
                    Some(interruption) => Err(DeliveryFailure::before_event(InputFault::from(
                        interruption,
                    ))),
                    None => self.driver.deliver(
                        delivery,
                        request.focus(),
                        event,
                        request.pointer_geometry(),
                        &mut state,
                        DeliveryContexts {
                            operation,
                            cleanup_budget: request.cleanup_budget(),
                        },
                    ),
                }
            };
            if let Err(failure) = result {
                return Ok(self.stopped_receipt(
                    request,
                    StoppedDelivery {
                        delivery,
                        attempted,
                        delivered,
                        failure,
                    },
                    &mut state,
                    operation,
                ));
            }
            delivered += 1;
        }
        Ok(InputReceipt::complete(request.target(), delivery, delivered).with_attempted(attempted))
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
