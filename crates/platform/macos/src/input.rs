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
    CoordinateSpace, FrameOrder, FrameStamp, InputCapability, InputDelivery, InputOperationKind,
    Lifecycle, OperationContext, PermissionKind, PixelExtent, StreamId, TargetKind,
    TargetPlacement, TransformSnapshot,
};
use mado_pilot_input::{
    Admission, FocusPolicy, InputController, InputDescriptor, InputEvent, InputFault, InputReceipt,
    InputRequest, Key, PointerButton, PointerGeometry, PressedState,
};

use crate::native_input::NativeInputDriver;
use crate::provider::TargetRecord;

const DELAY_POLL_INTERVAL: Duration = Duration::from_millis(2);

/// Returns what a discovered macOS target of `kind` accepts.
///
/// System delivery only. macOS has no per-window event channel an unfocused
/// process may post to, so there is no background mechanism to advertise and no
/// fixture class that could earn one; a request for background delivery is
/// refused by admission rather than quietly satisfied through system input.
///
/// A display is pointer-only. Keyboard and text need something focused to receive
/// them, and a display is not a focusable target.
pub(crate) fn input_capability(kind: TargetKind) -> InputCapability {
    let capability = InputCapability::none()
        .with_pair(InputOperationKind::Pointer, InputDelivery::System)
        .with_permission(PermissionKind::InputControl)
        .with_pointer_space(CoordinateSpace::CapturePixels)
        .with_pointer_space(CoordinateSpace::FrameNormalized)
        .with_pointer_space(CoordinateSpace::TargetNormalized)
        .with_pointer_space(CoordinateSpace::TargetLogical)
        .with_pointer_space(CoordinateSpace::DesktopLogical);

    if kind == TargetKind::Window {
        capability
            .with_pair(InputOperationKind::Keyboard, InputDelivery::System)
            .with_pair(InputOperationKind::Text, InputDelivery::System)
            .with_focus_required(InputDelivery::System)
    } else {
        capability
    }
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

/// Why delivery stopped, and whether the event it stopped on may have taken effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DeliveryFailure {
    pub(crate) fault: InputFault,
    pub(crate) current_event_may_have_effect: bool,
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

/// The seam the controller drives, so its rules are testable without posting an
/// event to the host desktop.
pub(crate) trait InputDriver: fmt::Debug + Send + Sync {
    fn preflight(
        &self,
        delivery: InputDelivery,
        focus: FocusPolicy,
        operation: &OperationContext,
    ) -> Result<(), InputFault>;

    /// Delivers one event, or reports how far into it the platform got.
    ///
    /// The request's cleanup budget is deliberately absent. It bounds the releases
    /// that follow a partial failure, and on macOS a delivery failure leaves no
    /// pressed state the event itself created — `NativeInputDriver::deliver_text`
    /// records why. The controller still applies that budget to the sequence's own
    /// pressed state.
    fn deliver(
        &self,
        delivery: InputDelivery,
        focus: FocusPolicy,
        event: &InputEvent,
        geometry: PointerGeometry,
        state: &mut DriverState,
        operation: &OperationContext,
    ) -> Result<(), DeliveryFailure>;

    fn release(
        &self,
        delivery: InputDelivery,
        pressed: PressedState,
        state: &mut DriverState,
        operation: &OperationContext,
    ) -> Result<(), InputFault>;
}

pub(crate) struct MacosInputController {
    descriptor: InputDescriptor,
    driver: Arc<dyn InputDriver>,
    admission: Admission,
}

impl MacosInputController {
    pub(crate) fn new(record: Arc<TargetRecord>) -> Arc<Self> {
        let descriptor = record.input_descriptor();
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
                        operation,
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
