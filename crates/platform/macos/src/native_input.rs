//! Thin `CGEvent` delivery backend for the shared macOS input controller.
//!
//! # What makes a receipt truthful here
//!
//! macOS does not fail a synthesized event posted by an unauthorized process: it
//! discards it, and both posting functions return nothing. Public post-event
//! access is therefore read again immediately before every irreversible logical
//! event rather than once at open. A revocation observed mid-sequence stops
//! delivery with the exact completed-event count; invocation-only evidence never
//! implies target observation or application consumption.

use std::sync::Arc;

use mado_pilot_core::{
    CancellationToken, CoordinateSpace, GeometryRevision, InputDelivery, OperationContext,
    PermissionState, PixelExtent, Point, Scale, TargetKind, TransformSnapshot,
};
use mado_pilot_input::{
    FocusPolicy, GeometryPolicy, InputEvent, InputFault, Key, Modifier, PointerButton,
    PointerGeometry, PressedState,
};

use crate::discovery::placement_from_points;
use crate::input::{
    DriverState, GeometryFingerprint, InputDriver, PendingTextRelease, PointerState,
    SubmissionFailure, SystemButtonState, SystemKeyState,
};
use crate::provider::TargetRecord;
use crate::shim::{self, ShimStatus};

type SubmissionResult = Result<(), SubmissionFailure>;

/// How long activation and one read-only Accessibility focus observation may
/// consume, and how often activation is polled.
///
/// Both waits are additionally bounded by the caller's operation. The native
/// focus observation carries the remaining slice into Accessibility messaging,
/// so an unresponsive application cannot hold input past it.
const ACTIVATION_SETTLE: std::time::Duration = std::time::Duration::from_millis(250);
const ACTIVATION_POLL: std::time::Duration = std::time::Duration::from_millis(10);
const FOCUS_OBSERVATION_WAIT: std::time::Duration = std::time::Duration::from_millis(250);
const PROCESS_OBSERVATION_WAIT: std::time::Duration = shim::MAX_NATIVE_WAIT;

fn focus_wait(operation: &OperationContext) -> std::time::Duration {
    operation
        .remaining()
        .map_or(FOCUS_OBSERVATION_WAIT, |remaining| {
            remaining.min(FOCUS_OBSERVATION_WAIT)
        })
}

fn process_wait(operation: &OperationContext) -> std::time::Duration {
    operation
        .remaining()
        .map_or(PROCESS_OBSERVATION_WAIT, |remaining| {
            remaining.min(PROCESS_OBSERVATION_WAIT)
        })
}

fn wait_for_activation<Observe, Sleep>(
    operation: &OperationContext,
    mut observe_focus: Observe,
    mut sleep: Sleep,
) -> Result<(), InputFault>
where
    Observe: FnMut() -> Result<bool, InputFault>,
    Sleep: FnMut(std::time::Duration),
{
    let Some(settle_deadline) = operation.now().checked_add(ACTIVATION_SETTLE) else {
        return Err(InputFault::FocusRefused);
    };
    loop {
        operation_fault(operation)?;
        if observe_focus()? {
            return Ok(());
        }
        let now = operation.now();
        if now >= settle_deadline {
            return Err(InputFault::FocusRefused);
        }
        let mut wait = settle_deadline
            .saturating_duration_since(now)
            .min(ACTIVATION_POLL);
        if let Some(remaining) = operation.remaining() {
            wait = wait.min(remaining);
        }
        if !wait.is_zero() {
            sleep(wait);
        }
    }
}

/// Hardware key codes for the keys whose position does not vary with the layout.
///
/// These are the `kVK_` values HIToolbox defines. They are transcribed rather than
/// read from Carbon because they are constants of the hardware map, and a table in
/// Rust is what a test can assert against.
const KEY_RETURN: u16 = 0x24;
const KEY_TAB: u16 = 0x30;
const KEY_SPACE: u16 = 0x31;
const KEY_BACKSPACE: u16 = 0x33;
const KEY_ESCAPE: u16 = 0x35;
const KEY_COMMAND: u16 = 0x37;
const KEY_SHIFT: u16 = 0x38;
const KEY_OPTION: u16 = 0x3A;
const KEY_CONTROL: u16 = 0x3B;
const KEY_FORWARD_DELETE: u16 = 0x75;
const KEY_HOME: u16 = 0x73;
const KEY_END: u16 = 0x77;
const KEY_PAGE_UP: u16 = 0x74;
const KEY_PAGE_DOWN: u16 = 0x79;
const KEY_ARROW_LEFT: u16 = 0x7B;
const KEY_ARROW_RIGHT: u16 = 0x7C;
const KEY_ARROW_DOWN: u16 = 0x7D;
const KEY_ARROW_UP: u16 = 0x7E;

/// Function keys one through twenty, in order.
///
/// macOS defines no key code for F21 to F24, which the shared contract still
/// accepts as a number. Those are reported unsupported rather than posted as
/// whatever the window server makes of an undefined code.
const FUNCTION_KEYS: [u16; 20] = [
    0x7A, 0x78, 0x63, 0x76, 0x60, 0x61, 0x62, 0x64, 0x65, 0x6D, 0x67, 0x6F, 0x69, 0x6B, 0x71, 0x6A,
    0x40, 0x4F, 0x50, 0x5A,
];

/// Returns the native modifier flag one logical modifier posts with.
pub(crate) const fn modifier_flag(modifier: Modifier) -> u32 {
    match modifier {
        Modifier::Shift => shim::INPUT_FLAG_SHIFT,
        Modifier::Control => shim::INPUT_FLAG_CONTROL,
        Modifier::Alt => shim::INPUT_FLAG_ALT,
        Modifier::Meta => shim::INPUT_FLAG_META,
        // A modifier this build does not know about has no flag to set, and
        // posting the event without it would deliver a different keystroke.
        _ => 0,
    }
}

/// One native post, prepared but not yet committed.
///
/// Separating preparation from the commit is what lets the deadline, target,
/// focus, and geometry be revalidated at the last possible point before an
/// irreversible event, and lets a deterministic test observe exactly what would
/// have been posted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum NativePost<'units> {
    Pointer {
        action: u32,
        button: u32,
        click_state: u64,
        location: (f64, f64),
    },
    Scroll {
        horizontal: i32,
        vertical: i32,
        location: (f64, f64),
    },
    Key {
        key_code: u16,
        down: bool,
    },
    Text(&'units [u16]),
}

impl<'units> NativePost<'units> {
    fn process_post(self) -> shim::ProcessPost<'units> {
        match self {
            NativePost::Pointer {
                action,
                button,
                click_state,
                location,
            } => shim::ProcessPost::Pointer {
                action,
                button,
                click_state,
                location,
            },
            NativePost::Scroll {
                horizontal,
                vertical,
                location,
            } => shim::ProcessPost::Scroll {
                horizontal,
                vertical,
                location,
            },
            NativePost::Key { key_code, down } => shim::ProcessPost::Key { key_code, down },
            NativePost::Text(units) => shim::ProcessPost::Text(units),
        }
    }

    const fn process_native_units(self) -> u64 {
        match self {
            NativePost::Text(_) => 2,
            NativePost::Pointer { .. } | NativePost::Scroll { .. } | NativePost::Key { .. } => 1,
        }
    }
}

/// Geometry validation required at the irreversible commit boundary.
///
/// Pointer resolution always retains the geometry policy explicitly. In
/// particular, a frame snapshot remains deliverable after the target moves,
/// while current and unchanged policies still require the resolved geometry to
/// match the live target immediately before posting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum CommitGeometry {
    NotApplicable,
    RequireCurrent(GeometryFingerprint),
    UseFrameSnapshot,
}

/// Adapter-monotonic deadline derived from one caller-clock checkpoint.
///
/// This deadline bounds native work without re-entering an arbitrary caller
/// clock after mutable target authority has been observed.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NativeCommitBudget {
    deadline_nanos: u64,
}

impl NativeCommitBudget {
    fn new(wait: std::time::Duration) -> Result<Self, InputFault> {
        let now = shim::monotonic_nanos().ok_or(InputFault::SubmissionFailed)?;
        let wait = u64::try_from(wait.as_nanos()).map_err(|_| InputFault::SubmissionFailed)?;
        if wait == 0 {
            return Err(InputFault::DeadlineExceeded);
        }
        Ok(Self {
            deadline_nanos: now.saturating_add(wait),
        })
    }

    fn remaining(self) -> Result<std::time::Duration, InputFault> {
        let now = shim::monotonic_nanos().ok_or(InputFault::SubmissionFailed)?;
        if now >= self.deadline_nanos {
            return Err(InputFault::SubmissionFailed);
        }
        Ok(std::time::Duration::from_nanos(self.deadline_nanos - now))
    }

    fn check(self) -> Result<(), InputFault> {
        self.remaining().map(|_| ())
    }

    const fn deadline_nanos(self) -> u64 {
        self.deadline_nanos
    }
}

/// The mutable native state consulted at the final commit boundary.
pub(crate) trait SystemCommitSource {
    type Prepared;

    /// Allocates and configures all native events before the final mutable gate.
    fn prepare(&self, post: NativePost<'_>, flags: u32) -> Result<Self::Prepared, ShimStatus>;

    fn prepared_unit_count(&self, prepared: &Self::Prepared) -> usize;

    /// Revalidates every mutable system-post fact within one adapter deadline.
    ///
    /// Implementations must not invoke caller-provided code. This lets the
    /// caller-clock checkpoint run before, and the atomic cancellation fence run
    /// after, the final mutable observations.
    fn revalidate_system_commit(
        &self,
        focus: FocusPolicy,
        geometry: CommitGeometry,
        budget: NativeCommitBudget,
    ) -> Result<(), InputFault>;

    /// Re-reads non-prompting authorization needed for a cleanup release.
    fn revalidate_cleanup_authorization(
        &self,
        budget: NativeCommitBudget,
    ) -> Result<(), InputFault>;

    /// Posts exactly one already-configured native unit.
    ///
    /// A failure distinguishes possible native effect from cancellation observed
    /// by the final atomic callback.
    fn post_prepared_unit(
        &self,
        prepared: &mut Self::Prepared,
        index: usize,
        deadline_nanos: u64,
        cancellation: Option<&CancellationToken>,
    ) -> Result<(), shim::PreparedPostFailure>;

    /// Classifies a failure before the current native unit could have effect.
    fn classify_post_failure(&self, status: ShimStatus) -> InputFault;
}

pub(crate) trait ProcessCommitSource {
    fn post_process(
        &self,
        event_source: Option<&shim::ProcessEventSource>,
        request: shim::ProcessPostRequest<'_>,
        operation: &OperationContext,
    ) -> Result<shim::ProcessPostOutcome, shim::ProcessPostFailure>;

    fn classify_process_failure(
        &self,
        failure: shim::ProcessPostFailure,
        operation: &OperationContext,
    ) -> InputFault {
        process_post_failure_fault(failure, operation)
    }
}

/// Effective mapping and raw native geometry selected by the process-directed
/// pointer policy.
///
/// Keeping this seam separate from native commit lets deterministic tests prove
/// which same-frame or live source is consulted without adding runtime dispatch
/// or production instrumentation.
trait ProcessGeometrySource {
    fn process_source_geometry(
        &self,
        geometry: PointerGeometry,
    ) -> Result<(TransformSnapshot, shim::NativeBounds), InputFault>;
    fn process_current_geometry(
        &self,
        operation: &OperationContext,
    ) -> Result<(TransformSnapshot, GeometryFingerprint, shim::NativeBounds), InputFault>;
}

pub(crate) struct NativeInputDriver {
    record: Arc<TargetRecord>,
}

impl NativeInputDriver {
    pub(crate) fn new(record: Arc<TargetRecord>) -> Self {
        Self { record }
    }

    /// Reads public post-event access without requesting it.
    ///
    /// Accessibility is retained in the paired migration observation for
    /// qualification only. It cannot promote a denied post-event result or demote
    /// a granted one.
    fn ensure_authorized(&self) -> Result<(), InputFault> {
        let observed = shim::process_authorization().map_err(|_| InputFault::SubmissionFailed)?;
        require_post_event_access(observed.post_event_access)
    }

    /// Distinguishes direct post-event denial from capture-query denial.
    fn classify_process_status(
        &self,
        status: ShimStatus,
        operation: &OperationContext,
    ) -> InputFault {
        if status != ShimStatus::PermissionDenied {
            return process_status_fault(status, operation);
        }
        shim::process_authorization().map_or(InputFault::SubmissionFailed, |observed| {
            process_permission_denied_fault(observed.post_event_access)
        })
    }

    fn is_focused(&self, operation: &OperationContext) -> Result<bool, InputFault> {
        if self.record.kind() == TargetKind::Display {
            return Ok(true);
        }
        match self.record.is_focused(focus_wait(operation)) {
            Ok(focused) => Ok(focused),
            Err(ShimStatus::PermissionDenied) => Err(InputFault::NotAuthorized),
            Err(ShimStatus::TargetLost) => Err(InputFault::TargetLost),
            Err(ShimStatus::TimedOut) => {
                operation.interruption().map_or(Ok(false), |interruption| {
                    Err(InputFault::from(interruption))
                })
            }
            Err(_) => Err(InputFault::SubmissionFailed),
        }
    }

    fn is_focused_with_wait(&self, wait: std::time::Duration) -> Result<bool, InputFault> {
        if self.record.kind() == TargetKind::Display {
            return Ok(true);
        }
        match self.record.is_focused(wait) {
            Ok(focused) => Ok(focused),
            Err(ShimStatus::PermissionDenied) => Err(InputFault::NotAuthorized),
            Err(ShimStatus::TargetLost) => Err(InputFault::TargetLost),
            Err(_) => Err(InputFault::SubmissionFailed),
        }
    }

    fn ensure_system_focus_with_wait(
        &self,
        policy: FocusPolicy,
        wait: std::time::Duration,
    ) -> Result<(), InputFault> {
        if self.is_focused_with_wait(wait)? {
            return Ok(());
        }
        match policy {
            FocusPolicy::Preserve | FocusPolicy::RequireFocused => Err(InputFault::FocusRequired),
            // Activation is never a final-gate action: it can invoke application
            // behavior and requires the caller's operation for bounded polling.
            FocusPolicy::ActivateIfRequired => Err(InputFault::FocusRequired),
            _ => Err(InputFault::UnsupportedCombination),
        }
    }

    fn ensure_system_focus(
        &self,
        policy: FocusPolicy,
        operation: &OperationContext,
    ) -> Result<(), InputFault> {
        if self.record.kind() == TargetKind::Display {
            // A display receives pointer input wherever the pointer is; nothing
            // about it is focusable, so nothing about focus applies.
            return Ok(());
        }
        if self.is_focused(operation)? {
            return Ok(());
        }
        match policy {
            FocusPolicy::Preserve | FocusPolicy::RequireFocused => Err(InputFault::FocusRequired),
            FocusPolicy::ActivateIfRequired => self.activate(operation),
            _ => Err(InputFault::UnsupportedCombination),
        }
    }

    /// Returns the caller-selected focus predicate for the final native gate.
    ///
    /// When `observe_now` is true, a false predicate is also refused early so a
    /// later caller-ordered route may still be selected with zero possible
    /// effect. A terminal native-event route defers that expensive observation
    /// to the final gate instead of reading the same mutable fact twice.
    ///
    /// This observation is not authority: the foreground can change while the
    /// bounded authority queries that follow are running, so the native gate
    /// re-evaluates the same predicate immediately before it posts.
    fn ensure_process_focus(
        &self,
        policy: FocusPolicy,
        observe_now: bool,
        operation: &OperationContext,
    ) -> Result<shim::ProcessFocusRequirement, InputFault> {
        match policy {
            FocusPolicy::Preserve | FocusPolicy::ActivateIfRequired => {
                Ok(shim::ProcessFocusRequirement::None)
            }
            FocusPolicy::RequireFocused => {
                if !observe_now || self.is_focused(operation)? {
                    Ok(shim::ProcessFocusRequirement::RequireFocused)
                } else {
                    Err(InputFault::FocusRequired)
                }
            }
            _ => Err(InputFault::UnsupportedCombination),
        }
    }

    /// Asks macOS to activate the owning application and reads exact focus back.
    ///
    /// This activates an application; it never raises one particular window.
    /// The retained window must still match the active application's focused
    /// Accessibility window one-to-one before delivery is accepted.
    fn activate(&self, operation: &OperationContext) -> Result<(), InputFault> {
        operation_fault(operation)?;
        match self.record.activate_owner() {
            Ok(()) => {}
            Err(ShimStatus::TargetLost) => return Err(InputFault::TargetLost),
            Err(ShimStatus::Unsupported) => return Err(InputFault::FocusRefused),
            Err(_) => return Err(InputFault::FocusRefused),
        }
        wait_for_activation(operation, || self.is_focused(operation), std::thread::sleep)
    }

    fn geometry_from_bounds(
        bounds: shim::NativeBounds,
    ) -> Result<(TransformSnapshot, GeometryFingerprint), InputFault> {
        let extent = extent_from_points(bounds.size, bounds.scale)?;
        let placement = placement_from_points(bounds.origin, bounds.size, bounds.scale, extent)
            .map_err(|_| InputFault::UnsupportedCoordinate)?;
        let transform = TransformSnapshot::with_target(GeometryRevision::FIRST, extent, placement)
            .map_err(|_| InputFault::UnsupportedCoordinate)?;
        Ok((transform, GeometryFingerprint { extent, placement }))
    }

    fn current_geometry(
        &self,
        operation: &OperationContext,
    ) -> Result<(TransformSnapshot, GeometryFingerprint), InputFault> {
        Self::geometry_from_bounds(self.record.current_bounds(focus_wait(operation))?)
    }

    fn current_geometry_with_wait(
        &self,
        wait: std::time::Duration,
    ) -> Result<(TransformSnapshot, GeometryFingerprint), InputFault> {
        Self::geometry_from_bounds(self.record.current_bounds(wait)?)
    }

    fn policy_geometry(
        &self,
        geometry: PointerGeometry,
        operation: &OperationContext,
    ) -> Result<(TransformSnapshot, GeometryFingerprint), InputFault> {
        match geometry.policy() {
            GeometryPolicy::ReprojectCurrent => self.current_geometry(operation),
            GeometryPolicy::RequireUnchanged => {
                let transform = self.source_transform(geometry)?;
                let source_fingerprint = fingerprint(&transform)?;
                let (_, current_fingerprint) = self.current_geometry(operation)?;
                if source_fingerprint != current_fingerprint {
                    return Err(InputFault::GeometryChanged);
                }
                Ok((transform, source_fingerprint))
            }
            GeometryPolicy::UseFrameSnapshot => {
                let transform = self.source_transform(geometry)?;
                let fingerprint = fingerprint(&transform)?;
                Ok((transform, fingerprint))
            }
            _ => Err(InputFault::UnsupportedCombination),
        }
    }

    fn process_current_geometry(
        &self,
        operation: &OperationContext,
    ) -> Result<(TransformSnapshot, GeometryFingerprint, shim::NativeBounds), InputFault> {
        let authority = self
            .record
            .process_authority(process_wait(operation))
            .map_err(|failure| self.classify_process_status(failure.status, operation))?;
        if authority.target_match_count != 1 {
            return Err(InputFault::UnsupportedCombination);
        }
        let (transform, fingerprint) = Self::geometry_from_bounds(authority.bounds)?;
        Ok((transform, fingerprint, authority.bounds))
    }

    fn process_policy_geometry(
        &self,
        geometry: PointerGeometry,
        operation: &OperationContext,
    ) -> Result<
        (
            TransformSnapshot,
            GeometryFingerprint,
            shim::ProcessGeometry,
        ),
        InputFault,
    > {
        process_policy_geometry_for(self, geometry, operation)
    }

    fn resolve_process_pointer(
        &self,
        point: Point,
        geometry: PointerGeometry,
        operation: &OperationContext,
    ) -> Result<(PointerState, shim::ProcessGeometry), InputFault> {
        let (transform, fingerprint, commit_geometry) =
            self.process_policy_geometry(geometry, operation)?;
        let desktop = transform
            .convert_point(point, CoordinateSpace::DesktopLogical)
            .map_err(|_| InputFault::UnsupportedCoordinate)?;
        let location = (desktop.x(), desktop.y());
        if !contains_desktop_point(fingerprint, location) {
            return Err(InputFault::UnsupportedCoordinate);
        }
        Ok((
            PointerState {
                desktop: location,
                geometry: fingerprint,
            },
            commit_geometry,
        ))
    }

    fn process_pointer_for_non_move(
        &self,
        geometry: PointerGeometry,
        state: &mut DriverState,
        operation: &OperationContext,
    ) -> Result<(PointerState, shim::ProcessGeometry), InputFault> {
        process_pointer_for_non_move_with(self, geometry, state, operation, || {
            shim::input_pointer_location().map_err(|_| InputFault::SubmissionFailed)
        })
    }

    fn source_geometry(
        &self,
        geometry: PointerGeometry,
    ) -> Result<(TransformSnapshot, shim::NativeBounds), InputFault> {
        let source = geometry
            .source()
            .ok_or(InputFault::MissingCoordinateSource)?;
        self.record
            .geometry()
            .source_geometry(source)
            .ok_or(InputFault::UnsupportedCoordinate)
    }

    fn source_transform(&self, geometry: PointerGeometry) -> Result<TransformSnapshot, InputFault> {
        self.source_geometry(geometry)
            .map(|(transform, _)| transform)
    }

    fn resolve_pointer(
        &self,
        point: Point,
        geometry: PointerGeometry,
        operation: &OperationContext,
    ) -> Result<PointerState, InputFault> {
        let (transform, fingerprint) = self.policy_geometry(geometry, operation)?;
        let desktop = transform
            .convert_point(point, CoordinateSpace::DesktopLogical)
            .map_err(|_| InputFault::UnsupportedCoordinate)?;
        let location = (desktop.x(), desktop.y());
        if !contains_desktop_point(fingerprint, location) {
            return Err(InputFault::UnsupportedCoordinate);
        }
        Ok(PointerState {
            desktop: location,
            geometry: fingerprint,
        })
    }

    fn pointer_for_non_move(
        &self,
        geometry: PointerGeometry,
        state: &mut DriverState,
        operation: &OperationContext,
    ) -> Result<PointerState, InputFault> {
        let (_, current) = self.policy_geometry(geometry, operation)?;
        if let Some(pointer) = state.pointer {
            if pointer.geometry != current {
                return Err(InputFault::GeometryChanged);
            }
            return Ok(pointer);
        }

        let location = shim::input_pointer_location().map_err(|_| InputFault::SubmissionFailed)?;
        if !contains_desktop_point(current, location) {
            return Err(InputFault::UnsupportedCoordinate);
        }
        let pointer = PointerState {
            desktop: location,
            geometry: current,
        };
        state.pointer = Some(pointer);
        Ok(pointer)
    }

    fn deliver_system(
        &self,
        focus: FocusPolicy,
        event: &InputEvent,
        geometry: PointerGeometry,
        state: &mut DriverState,
        operation: &OperationContext,
    ) -> SubmissionResult {
        self.record.ensure_live(focus_wait(operation))?;
        self.ensure_authorized()?;
        self.ensure_system_focus(focus, operation)?;
        let flags = state.held_flags();

        match event {
            InputEvent::PointerMove(point) => {
                let resolved = self.resolve_pointer(*point, geometry, operation)?;
                let button = state
                    .dragging()
                    .map_or(Ok(shim::INPUT_BUTTON_NONE), native_button)?;
                commit_prepared(
                    self,
                    focus,
                    commit_geometry(geometry.policy(), resolved.geometry)?,
                    operation,
                    NativePost::Pointer {
                        action: shim::INPUT_POINTER_MOVE,
                        button,
                        click_state: 0,
                        location: resolved.desktop,
                    },
                    flags,
                )?;
                state.pointer = Some(resolved);
                Ok(())
            }
            InputEvent::PointerPress(button) => {
                let pointer = self.pointer_for_non_move(geometry, state, operation)?;
                let native = native_button(*button)?;
                commit_prepared(
                    self,
                    focus,
                    commit_geometry(geometry.policy(), pointer.geometry)?,
                    operation,
                    NativePost::Pointer {
                        action: shim::INPUT_POINTER_PRESS,
                        button: native,
                        click_state: shim::INPUT_SINGLE_CLICK,
                        location: pointer.desktop,
                    },
                    flags,
                )?;
                state.buttons.push(SystemButtonState {
                    logical: *button,
                    native,
                });
                Ok(())
            }
            InputEvent::PointerRelease(button) => {
                let pointer = self.pointer_for_non_move(geometry, state, operation)?;
                let index = state
                    .buttons
                    .iter()
                    .rposition(|pressed| pressed.logical == *button);
                let native = index
                    .map(|index| state.buttons[index].native)
                    .map_or_else(|| native_button(*button), Ok)?;
                commit_prepared(
                    self,
                    focus,
                    commit_geometry(geometry.policy(), pointer.geometry)?,
                    operation,
                    NativePost::Pointer {
                        action: shim::INPUT_POINTER_RELEASE,
                        button: native,
                        click_state: shim::INPUT_SINGLE_CLICK,
                        location: pointer.desktop,
                    },
                    flags,
                )?;
                if let Some(index) = index {
                    state.buttons.remove(index);
                }
                Ok(())
            }
            InputEvent::PointerScroll {
                horizontal,
                vertical,
            } => {
                let pointer = self.pointer_for_non_move(geometry, state, operation)?;
                commit_prepared(
                    self,
                    focus,
                    commit_geometry(geometry.policy(), pointer.geometry)?,
                    operation,
                    NativePost::Scroll {
                        horizontal: i32::from(*horizontal),
                        vertical: i32::from(*vertical),
                        location: pointer.desktop,
                    },
                    flags,
                )
            }
            InputEvent::KeyPress(key) => {
                let key_code = resolve_key_code(*key)?;
                // A modifier takes effect from its own press onward, so the flag it
                // adds is set on the event that presses it as well.
                let flags = flags | key_flag(*key);
                commit_prepared(
                    self,
                    focus,
                    CommitGeometry::NotApplicable,
                    operation,
                    NativePost::Key {
                        key_code,
                        down: true,
                    },
                    flags,
                )?;
                state.keys.push(SystemKeyState {
                    logical: *key,
                    key_code,
                });
                Ok(())
            }
            InputEvent::KeyRelease(key) => {
                let index = state
                    .keys
                    .iter()
                    .rposition(|pressed| pressed.logical == *key);
                let key_code = index
                    .map(|index| Ok(state.keys[index].key_code))
                    .unwrap_or_else(|| resolve_key_code(*key))?;
                // The released modifier is already clear on this event, which is
                // what makes a release observable as one.
                let flags = flags & !key_flag(*key);
                commit_prepared(
                    self,
                    focus,
                    CommitGeometry::NotApplicable,
                    operation,
                    NativePost::Key {
                        key_code,
                        down: false,
                    },
                    flags,
                )?;
                if let Some(index) = index {
                    state.keys.remove(index);
                }
                Ok(())
            }
            InputEvent::Text(text) => self.deliver_text(text, focus, operation, state, flags),
            InputEvent::Delay(_) => Err(InputFault::UnsupportedCombination.into()),
            _ => Err(InputFault::UnsupportedCombination.into()),
        }
    }

    /// Posts one text event as bounded chunks, keeping surrogate pairs whole.
    ///
    /// A chunk that reaches the target after an earlier one succeeded is native
    /// effect this Adapter cannot take back, so the failure is reported as
    /// happening *during* the event and the receipt says so.
    ///
    /// A text chunk is a balanced native key-down/key-up pair. If the down call
    /// may have reached the irreversible posting threshold and the up call is
    /// not known to have returned, the driver records one private pending release
    /// so bounded cleanup can report and attempt it without retaining caller text.
    fn deliver_text(
        &self,
        text: &str,
        focus: FocusPolicy,
        operation: &OperationContext,
        state: &mut DriverState,
        flags: u32,
    ) -> SubmissionResult {
        let units: Vec<u16> = text.encode_utf16().collect();
        let mut sent = 0usize;
        for chunk in text_chunks(&units) {
            let result = commit_prepared(
                self,
                focus,
                CommitGeometry::NotApplicable,
                operation,
                NativePost::Text(&units[chunk.clone()]),
                flags,
            );
            if let Err(mut failure) = result {
                if text_release_may_be_pending(failure) {
                    state.pending_text_release = Some(PendingTextRelease {
                        route: InputDelivery::System,
                        flags,
                    });
                }
                if sent > 0 {
                    failure.current_event_may_have_effect = true;
                }
                return Err(failure);
            }
            sent += chunk.len();
        }
        Ok(())
    }

    fn deliver_process(
        &self,
        focus: FocusPolicy,
        event: &InputEvent,
        geometry: PointerGeometry,
        state: &mut DriverState,
        operation: &OperationContext,
    ) -> SubmissionResult {
        let focus = self.ensure_process_focus(focus, false, operation)?;
        let flags = state.held_flags();

        match event {
            InputEvent::PointerMove(point) => {
                let (resolved, commit_geometry) =
                    self.resolve_process_pointer(*point, geometry, operation)?;
                let button = state
                    .dragging()
                    .map_or(Ok(shim::INPUT_BUTTON_NONE), native_button)?;
                let result = commit_process(
                    self,
                    state.process_event_source.as_ref(),
                    commit_geometry,
                    focus,
                    operation,
                    NativePost::Pointer {
                        action: shim::INPUT_POINTER_MOVE,
                        button,
                        click_state: 0,
                        location: resolved.desktop,
                    },
                    flags,
                );
                retain_process_pointer(&mut state.pointer, resolved, result)
            }
            InputEvent::PointerPress(button) => {
                let (pointer, commit_geometry) =
                    self.process_pointer_for_non_move(geometry, state, operation)?;
                let native = native_button(*button)?;
                let result = commit_process(
                    self,
                    state.process_event_source.as_ref(),
                    commit_geometry,
                    focus,
                    operation,
                    NativePost::Pointer {
                        action: shim::INPUT_POINTER_PRESS,
                        button: native,
                        click_state: shim::INPUT_SINGLE_CLICK,
                        location: pointer.desktop,
                    },
                    flags,
                );
                retain_process_press(
                    &mut state.buttons,
                    SystemButtonState {
                        logical: *button,
                        native,
                    },
                    result,
                )
            }
            InputEvent::PointerRelease(button) => {
                let (pointer, commit_geometry) =
                    self.process_pointer_for_non_move(geometry, state, operation)?;
                let index = state
                    .buttons
                    .iter()
                    .rposition(|pressed| pressed.logical == *button);
                let native = index
                    .map(|index| state.buttons[index].native)
                    .map_or_else(|| native_button(*button), Ok)?;
                commit_process(
                    self,
                    state.process_event_source.as_ref(),
                    commit_geometry,
                    focus,
                    operation,
                    NativePost::Pointer {
                        action: shim::INPUT_POINTER_RELEASE,
                        button: native,
                        click_state: shim::INPUT_SINGLE_CLICK,
                        location: pointer.desktop,
                    },
                    flags,
                )?;
                if let Some(index) = index {
                    state.buttons.remove(index);
                }
                Ok(())
            }
            InputEvent::PointerScroll {
                horizontal,
                vertical,
            } => {
                let (pointer, commit_geometry) =
                    self.process_pointer_for_non_move(geometry, state, operation)?;
                commit_process(
                    self,
                    state.process_event_source.as_ref(),
                    commit_geometry,
                    focus,
                    operation,
                    NativePost::Scroll {
                        horizontal: i32::from(*horizontal),
                        vertical: i32::from(*vertical),
                        location: pointer.desktop,
                    },
                    flags,
                )
            }
            InputEvent::KeyPress(key) => {
                let key_code = resolve_key_code(*key)?;
                let flags = flags | key_flag(*key);
                let result = commit_process(
                    self,
                    state.process_event_source.as_ref(),
                    shim::ProcessGeometry::AuthorityOnly,
                    focus,
                    operation,
                    NativePost::Key {
                        key_code,
                        down: true,
                    },
                    flags,
                );
                retain_process_press(
                    &mut state.keys,
                    SystemKeyState {
                        logical: *key,
                        key_code,
                    },
                    result,
                )
            }
            InputEvent::KeyRelease(key) => {
                let index = state
                    .keys
                    .iter()
                    .rposition(|pressed| pressed.logical == *key);
                let key_code = index
                    .map(|index| Ok(state.keys[index].key_code))
                    .unwrap_or_else(|| resolve_key_code(*key))?;
                let flags = flags & !key_flag(*key);
                commit_process(
                    self,
                    state.process_event_source.as_ref(),
                    shim::ProcessGeometry::AuthorityOnly,
                    focus,
                    operation,
                    NativePost::Key {
                        key_code,
                        down: false,
                    },
                    flags,
                )?;
                if let Some(index) = index {
                    state.keys.remove(index);
                }
                Ok(())
            }
            InputEvent::Text(text) => {
                self.deliver_process_text(text, focus, state, operation, flags)
            }
            InputEvent::Delay(_) => Err(InputFault::UnsupportedCombination.into()),
            _ => Err(InputFault::UnsupportedCombination.into()),
        }
    }

    fn deliver_process_text(
        &self,
        text: &str,
        focus: shim::ProcessFocusRequirement,
        state: &mut DriverState,
        operation: &OperationContext,
        flags: u32,
    ) -> SubmissionResult {
        let units: Vec<u16> = text.encode_utf16().collect();
        let mut sent = 0usize;
        for chunk in text_chunks(&units) {
            let result = commit_process(
                self,
                state.process_event_source.as_ref(),
                shim::ProcessGeometry::AuthorityOnly,
                focus,
                operation,
                NativePost::Text(&units[chunk.clone()]),
                flags,
            );
            if let Err(mut failure) = result {
                if text_release_may_be_pending(failure) {
                    state.pending_text_release = Some(PendingTextRelease {
                        route: InputDelivery::ProcessDirected,
                        flags,
                    });
                }
                if sent > 0 {
                    failure.current_event_may_have_effect = true;
                }
                return Err(failure);
            }
            sent += chunk.len();
        }
        Ok(())
    }
}

impl ProcessGeometrySource for NativeInputDriver {
    fn process_source_geometry(
        &self,
        geometry: PointerGeometry,
    ) -> Result<(TransformSnapshot, shim::NativeBounds), InputFault> {
        NativeInputDriver::source_geometry(self, geometry)
    }

    fn process_current_geometry(
        &self,
        operation: &OperationContext,
    ) -> Result<(TransformSnapshot, GeometryFingerprint, shim::NativeBounds), InputFault> {
        NativeInputDriver::process_current_geometry(self, operation)
    }
}

fn process_policy_geometry_for<S: ProcessGeometrySource + ?Sized>(
    source: &S,
    geometry: PointerGeometry,
    operation: &OperationContext,
) -> Result<
    (
        TransformSnapshot,
        GeometryFingerprint,
        shim::ProcessGeometry,
    ),
    InputFault,
> {
    match geometry.policy() {
        GeometryPolicy::ReprojectCurrent => {
            let (transform, fingerprint, bounds) = source.process_current_geometry(operation)?;
            Ok((
                transform,
                fingerprint,
                shim::ProcessGeometry::RequireCurrent(bounds),
            ))
        }
        GeometryPolicy::RequireUnchanged => {
            let (transform, source_bounds) = source.process_source_geometry(geometry)?;
            let source_fingerprint = fingerprint(&transform)?;
            // The source transform and same-sample raw native bounds name the exact
            // geometry the caller requires. Passing both to the native commit gate
            // lets that one final fresh authority observation establish
            // retained-window authority and reject movement or resize. A separate
            // live query here repeated the same ScreenCaptureKit inventory
            // immediately before the commit without strengthening the irreversible
            // fence.
            Ok((
                transform,
                source_fingerprint,
                shim::ProcessGeometry::RequireCurrent(source_bounds),
            ))
        }
        GeometryPolicy::UseFrameSnapshot => {
            let (transform, _) = source.process_source_geometry(geometry)?;
            let source_fingerprint = fingerprint(&transform)?;
            // Snapshot geometry deliberately tolerates movement. Exact retained
            // window authority is still checked once at the native commit
            // boundary, so an additional geometry inventory here was redundant.
            Ok((
                transform,
                source_fingerprint,
                shim::ProcessGeometry::AuthorityOnly,
            ))
        }
        _ => Err(InputFault::UnsupportedCombination),
    }
}

fn process_pointer_for_non_move_with<S: ProcessGeometrySource + ?Sized>(
    source: &S,
    geometry: PointerGeometry,
    state: &mut DriverState,
    operation: &OperationContext,
    read_pointer: impl FnOnce() -> Result<(f64, f64), InputFault>,
) -> Result<(PointerState, shim::ProcessGeometry), InputFault> {
    let (_, current, commit_geometry) = process_policy_geometry_for(source, geometry, operation)?;
    if let Some(pointer) = state.pointer {
        if pointer.geometry != current {
            return Err(InputFault::GeometryChanged);
        }
        return Ok((pointer, commit_geometry));
    }
    let location = read_pointer()?;
    if !contains_desktop_point(current, location) {
        return Err(InputFault::UnsupportedCoordinate);
    }
    let pointer = PointerState {
        desktop: location,
        geometry: current,
    };
    state.pointer = Some(pointer);
    Ok((pointer, commit_geometry))
}

/// Splits UTF-16 units into posts of at most one native chunk, never mid-pair.
fn text_chunks(units: &[u16]) -> Vec<std::ops::Range<usize>> {
    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < units.len() {
        let mut end = (start + shim::INPUT_MAX_TEXT_CHUNK).min(units.len());
        // A high surrogate at the end of a chunk names half a character, and the
        // window server would compose it with whatever arrived next.
        if end < units.len() && (0xD800..0xDC00).contains(&units[end - 1]) {
            end -= 1;
        }
        debug_assert!(end > start, "a chunk of at least two units always advances");
        chunks.push(start..end);
        start = end;
    }
    chunks
}

impl SystemCommitSource for NativeInputDriver {
    type Prepared = shim::PreparedInput;

    fn prepare(&self, post: NativePost<'_>, flags: u32) -> Result<Self::Prepared, ShimStatus> {
        match post {
            NativePost::Pointer {
                action,
                button,
                click_state,
                location,
            } => shim::input_prepare_pointer(action, button, click_state, location, flags),
            NativePost::Scroll {
                horizontal,
                vertical,
                location,
            } => shim::input_prepare_scroll(horizontal, vertical, location, flags),
            NativePost::Key { key_code, down } => shim::input_prepare_key(key_code, down, flags),
            NativePost::Text(units) => shim::input_prepare_text(units, flags),
        }
    }

    fn prepared_unit_count(&self, prepared: &Self::Prepared) -> usize {
        prepared.count()
    }

    fn revalidate_system_commit(
        &self,
        focus: FocusPolicy,
        geometry: CommitGeometry,
        budget: NativeCommitBudget,
    ) -> Result<(), InputFault> {
        self.record.ensure_live(budget.remaining()?)?;
        self.ensure_authorized()?;
        budget.check()?;
        self.ensure_system_focus_with_wait(focus, budget.remaining()?)?;
        if let CommitGeometry::RequireCurrent(expected) = geometry {
            let (_, current) = self.current_geometry_with_wait(budget.remaining()?)?;
            if current != expected {
                return Err(InputFault::GeometryChanged);
            }
        }
        budget.check()
    }

    fn revalidate_cleanup_authorization(
        &self,
        budget: NativeCommitBudget,
    ) -> Result<(), InputFault> {
        self.ensure_authorized()?;
        budget.check()
    }

    fn post_prepared_unit(
        &self,
        prepared: &mut Self::Prepared,
        index: usize,
        deadline_nanos: u64,
        cancellation: Option<&CancellationToken>,
    ) -> Result<(), shim::PreparedPostFailure> {
        prepared.post_unit(index, deadline_nanos, cancellation)
    }

    fn classify_post_failure(&self, status: ShimStatus) -> InputFault {
        match status {
            ShimStatus::TargetLost => InputFault::TargetLost,
            ShimStatus::PermissionDenied => InputFault::NotAuthorized,
            ShimStatus::Unsupported => InputFault::UnsupportedCombination,
            _ => self
                .ensure_authorized()
                .err()
                .unwrap_or(InputFault::SubmissionFailed),
        }
    }
}

impl ProcessCommitSource for NativeInputDriver {
    fn post_process(
        &self,
        event_source: Option<&shim::ProcessEventSource>,
        request: shim::ProcessPostRequest<'_>,
        operation: &OperationContext,
    ) -> Result<shim::ProcessPostOutcome, shim::ProcessPostFailure> {
        let Some(event_source) = event_source else {
            return Err(shim::ProcessPostFailure {
                status: ShimStatus::InvalidArgument,
                invoked_native_units: 0,
                native_effect_may_have_occurred: false,
                target_match_count: 0,
                authorization: shim::ProcessAuthorizationObservation::Unknown,
                geometry: shim::ProcessGeometryObservation::NotEvaluated,
                focus: shim::ProcessFocusObservation::NotEvaluated,
            });
        };
        self.record.process_post(event_source, request, operation)
    }

    fn classify_process_failure(
        &self,
        failure: shim::ProcessPostFailure,
        operation: &OperationContext,
    ) -> InputFault {
        process_post_failure_fault(failure, operation)
    }
}

impl std::fmt::Debug for NativeInputDriver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeInputDriver")
            .field("target", &self.record.target())
            .field("kind", &self.record.kind())
            .finish()
    }
}

impl InputDriver for NativeInputDriver {
    fn preflight(
        &self,
        delivery: InputDelivery,
        focus: FocusPolicy,
        require_early_authority: bool,
        operation: &OperationContext,
    ) -> Result<(), InputFault> {
        operation_fault(operation)?;
        match delivery {
            InputDelivery::System => {
                self.record.ensure_live(focus_wait(operation))?;
                self.ensure_authorized()?;
                self.ensure_system_focus(focus, operation)
            }
            InputDelivery::ProcessDirected => {
                if self.record.kind() != TargetKind::Window {
                    return Err(InputFault::UnsupportedCombination);
                }
                self.ensure_authorized()?;
                if require_early_authority {
                    let authority = self
                        .record
                        .process_authority(process_wait(operation))
                        .map_err(|failure| {
                            self.classify_process_status(failure.status, operation)
                        })?;
                    if authority.target_match_count != 1 {
                        return Err(InputFault::UnsupportedCombination);
                    }
                }
                self.ensure_process_focus(focus, require_early_authority, operation)
                    .map(|_| ())
            }
            _ => Err(InputFault::UnsupportedCombination),
        }
    }

    fn begin_route(
        &self,
        delivery: InputDelivery,
        state: &mut DriverState,
        operation: &OperationContext,
    ) -> Result<(), InputFault> {
        operation_fault(operation)?;
        state.process_event_source = match delivery {
            InputDelivery::System => None,
            InputDelivery::ProcessDirected => Some(
                shim::ProcessEventSource::new(operation.activity_tag().map_or(0, |tag| tag.get()))
                    .map_err(|status| self.classify_process_status(status, operation))?,
            ),
            _ => return Err(InputFault::UnsupportedCombination),
        };
        Ok(())
    }

    fn submit(
        &self,
        delivery: InputDelivery,
        focus: FocusPolicy,
        event: &InputEvent,
        geometry: PointerGeometry,
        state: &mut DriverState,
        operation: &OperationContext,
    ) -> SubmissionResult {
        operation_fault(operation)?;
        match delivery {
            InputDelivery::System => self.deliver_system(focus, event, geometry, state, operation),
            InputDelivery::ProcessDirected => {
                self.deliver_process(focus, event, geometry, state, operation)
            }
            _ => Err(InputFault::UnsupportedCombination.into()),
        }
    }

    fn release_pending(
        &self,
        delivery: InputDelivery,
        state: &mut DriverState,
        operation: &OperationContext,
    ) -> Result<bool, InputFault> {
        operation_fault(operation)?;
        let Some(pending) = state.pending_text_release else {
            return Ok(false);
        };
        if pending.route != delivery {
            return Err(InputFault::UnsupportedCombination);
        }
        match delivery {
            InputDelivery::System => commit_cleanup(
                self,
                operation,
                NativePost::Key {
                    key_code: 0,
                    down: false,
                },
                pending.flags,
            ),
            InputDelivery::ProcessDirected => release_pending_process(
                self,
                state.process_event_source.as_ref(),
                pending,
                operation,
            ),
            _ => Err(InputFault::UnsupportedCombination),
        }?;
        state.pending_text_release = None;
        Ok(true)
    }

    fn release(
        &self,
        delivery: InputDelivery,
        pressed: PressedState,
        state: &mut DriverState,
        operation: &OperationContext,
    ) -> Result<(), InputFault> {
        operation_fault(operation)?;
        match delivery {
            InputDelivery::System => release_system(pressed, state, self, operation),
            InputDelivery::ProcessDirected => release_process(pressed, state, self, operation),
            _ => Err(InputFault::UnsupportedCombination),
        }
    }
}

/// Releases one pressed state under the cleanup context.
///
/// Focus and geometry are deliberately not revalidated. A release is what stops a
/// button or modifier being held across the rest of the user's session, and a
/// window that stopped being frontmost is exactly when it matters most.
fn release_system<S: SystemCommitSource + ?Sized>(
    pressed: PressedState,
    state: &mut DriverState,
    source: &S,
    cleanup: &OperationContext,
) -> Result<(), InputFault> {
    match pressed {
        PressedState::Button(button) => {
            let index = state
                .buttons
                .iter()
                .rposition(|held| held.logical == button);
            let native = index
                .map(|index| state.buttons[index].native)
                .map_or_else(|| native_button(button), Ok)?;
            let location = state
                .pointer
                .ok_or(InputFault::UnsupportedCoordinate)?
                .desktop;
            let flags = state.held_flags();
            commit_cleanup(
                source,
                cleanup,
                NativePost::Pointer {
                    action: shim::INPUT_POINTER_RELEASE,
                    button: native,
                    click_state: shim::INPUT_SINGLE_CLICK,
                    location,
                },
                flags,
            )?;
            if let Some(index) = index {
                state.buttons.remove(index);
            }
            Ok(())
        }
        PressedState::Key(key) => {
            let index = state.keys.iter().rposition(|held| held.logical == key);
            let key_code = index
                .map(|index| Ok(state.keys[index].key_code))
                .unwrap_or_else(|| resolve_key_code(key))?;
            let flags = state.held_flags() & !key_flag(key);
            commit_cleanup(
                source,
                cleanup,
                NativePost::Key {
                    key_code,
                    down: false,
                },
                flags,
            )?;
            if let Some(index) = index {
                state.keys.remove(index);
            }
            Ok(())
        }
        _ => Err(InputFault::UnsupportedCombination),
    }
}

fn retain_process_press<T>(
    pressed: &mut Vec<T>,
    native_state: T,
    result: SubmissionResult,
) -> SubmissionResult {
    if result.is_ok()
        || result
            .as_ref()
            .is_err_and(|failure| failure.current_event_may_have_effect)
    {
        pressed.push(native_state);
    }
    result
}

fn retain_process_pointer(
    pointer: &mut Option<PointerState>,
    resolved: PointerState,
    result: SubmissionResult,
) -> SubmissionResult {
    if result.is_ok()
        || result
            .as_ref()
            .is_err_and(|failure| failure.current_event_may_have_effect)
    {
        *pointer = Some(resolved);
    }
    result
}

fn text_release_may_be_pending(failure: SubmissionFailure) -> bool {
    failure.current_event_may_have_effect && failure.invoked_native_units < 2
}

fn release_pending_process<S: ProcessCommitSource + ?Sized>(
    source: &S,
    event_source: Option<&shim::ProcessEventSource>,
    pending: PendingTextRelease,
    cleanup: &OperationContext,
) -> Result<(), InputFault> {
    commit_process_for(
        source,
        event_source,
        ProcessCommit::release(pending.flags),
        cleanup,
        NativePost::Key {
            key_code: 0,
            down: false,
        },
    )
    .map_err(|failure| failure.fault)
}

fn release_process<S: ProcessCommitSource + ?Sized>(
    pressed: PressedState,
    state: &mut DriverState,
    source: &S,
    cleanup: &OperationContext,
) -> Result<(), InputFault> {
    match pressed {
        PressedState::Button(button) => {
            let index = state
                .buttons
                .iter()
                .rposition(|held| held.logical == button);
            let native = index
                .map(|index| state.buttons[index].native)
                .map_or_else(|| native_button(button), Ok)?;
            let location = state
                .pointer
                .ok_or(InputFault::UnsupportedCoordinate)?
                .desktop;
            let flags = state.held_flags();
            commit_process_for(
                source,
                state.process_event_source.as_ref(),
                ProcessCommit::release(flags),
                cleanup,
                NativePost::Pointer {
                    action: shim::INPUT_POINTER_RELEASE,
                    button: native,
                    click_state: shim::INPUT_SINGLE_CLICK,
                    location,
                },
            )
            .map_err(|failure| failure.fault)?;
            if let Some(index) = index {
                state.buttons.remove(index);
            }
            Ok(())
        }
        PressedState::Key(key) => {
            let index = state.keys.iter().rposition(|held| held.logical == key);
            let key_code = index
                .map(|index| Ok(state.keys[index].key_code))
                .unwrap_or_else(|| resolve_key_code(key))?;
            let flags = state.held_flags() & !key_flag(key);
            commit_process_for(
                source,
                state.process_event_source.as_ref(),
                ProcessCommit::release(flags),
                cleanup,
                NativePost::Key {
                    key_code,
                    down: false,
                },
            )
            .map_err(|failure| failure.fault)?;
            if let Some(index) = index {
                state.keys.remove(index);
            }
            Ok(())
        }
        _ => Err(InputFault::UnsupportedCombination),
    }
}

fn require_post_event_access(access: PermissionState) -> Result<(), InputFault> {
    match access {
        PermissionState::Granted => Ok(()),
        PermissionState::NotGranted => Err(InputFault::NotAuthorized),
        PermissionState::Unavailable | PermissionState::Unknown => {
            Err(InputFault::SubmissionFailed)
        }
        _ => Err(InputFault::SubmissionFailed),
    }
}

fn operation_fault(operation: &OperationContext) -> Result<(), InputFault> {
    let interruption = shim::catch_panic(|| operation.interruption())
        .map_err(|()| InputFault::SubmissionFailed)?;
    interruption.map_or(Ok(()), |interruption| Err(InputFault::from(interruption)))
}

fn fault_after_failed_gate(operation: &OperationContext, fault: InputFault) -> InputFault {
    shim::catch_panic(|| operation.interruption())
        .map_or(InputFault::SubmissionFailed, |interruption| {
            interruption.map_or(fault, InputFault::from)
        })
}

fn fault_after_native_timeout(operation: &OperationContext, fault: InputFault) -> InputFault {
    shim::catch_panic(|| operation.remaining()).map_or(InputFault::SubmissionFailed, |remaining| {
        if remaining == Some(std::time::Duration::ZERO) {
            InputFault::DeadlineExceeded
        } else {
            fault
        }
    })
}

fn prepared_post_failure_fault<S: SystemCommitSource + ?Sized>(
    source: &S,
    operation: &OperationContext,
    failure: shim::PreparedPostFailure,
) -> InputFault {
    if failure.native_effect_may_have_occurred {
        return InputFault::SubmissionFailed;
    }
    match failure.status {
        ShimStatus::TimedOut if failure.cancellation_prevented_effect => InputFault::Cancelled,
        ShimStatus::TimedOut => {
            fault_after_native_timeout(operation, source.classify_post_failure(failure.status))
        }
        status => source.classify_post_failure(status),
    }
}

/// Caller-clock and adapter-clock state captured immediately before one system
/// post unit's final mutable gate.
///
/// Native preparation has already completed. After authority, only adapter
/// monotonic time and cloned atomic cancellation are consulted.
#[derive(Debug)]
struct SystemCommitCheckpoint<'a> {
    budget: NativeCommitBudget,
    cancellation: Option<&'a CancellationToken>,
}

impl<'a> SystemCommitCheckpoint<'a> {
    fn new(operation: &'a OperationContext) -> Result<Self, InputFault> {
        let cancellation = operation.cancellation();
        if cancellation.is_some_and(CancellationToken::is_cancelled) {
            return Err(InputFault::Cancelled);
        }
        let remaining = shim::catch_panic(|| operation.remaining())
            .map_err(|()| InputFault::SubmissionFailed)?;
        if remaining == Some(std::time::Duration::ZERO) {
            return Err(InputFault::DeadlineExceeded);
        }
        let wait = remaining.map_or(FOCUS_OBSERVATION_WAIT, |remaining| {
            remaining.min(FOCUS_OBSERVATION_WAIT)
        });
        Ok(Self {
            budget: NativeCommitBudget::new(wait)?,
            cancellation,
        })
    }

    const fn budget(&self) -> NativeCommitBudget {
        self.budget
    }

    const fn cancellation(&self) -> Option<&CancellationToken> {
        self.cancellation
    }

    fn check_final_fence(&self) -> Result<(), InputFault> {
        if self
            .cancellation
            .is_some_and(CancellationToken::is_cancelled)
        {
            return Err(InputFault::Cancelled);
        }
        self.budget.check()
    }
}

const fn process_permission_denied_fault(post_event_access: PermissionState) -> InputFault {
    match post_event_access {
        PermissionState::NotGranted => InputFault::NotAuthorized,
        PermissionState::Granted | PermissionState::Unavailable | PermissionState::Unknown => {
            InputFault::SubmissionFailed
        }
        _ => InputFault::SubmissionFailed,
    }
}

/// Names the fault one native process-directed status reports as.
///
/// Every variant is matched. A status added later is a compile error here, which
/// is where the decision about what a process-directed caller sees belongs: the
/// alternative is a wildcard that silently reports a typed refusal as an
/// unexplained submission failure.
fn process_status_fault(status: ShimStatus, operation: &OperationContext) -> InputFault {
    match status {
        ShimStatus::TargetLost => InputFault::TargetLost,
        ShimStatus::PermissionDenied => InputFault::NotAuthorized,
        ShimStatus::Unsupported => InputFault::UnsupportedCombination,
        ShimStatus::GeometryChanged => InputFault::GeometryChanged,
        ShimStatus::FocusRequired => InputFault::FocusRequired,
        ShimStatus::Closed => InputFault::ControllerClosed,
        ShimStatus::TimedOut => operation
            .interruption()
            .map_or(InputFault::SubmissionFailed, InputFault::from),
        // A successful status has no fault, and the rest name no process-directed
        // outcome a caller can act on differently.
        ShimStatus::Ok
        | ShimStatus::InvalidArgument
        | ShimStatus::PlatformFailure
        | ShimStatus::NativeException
        | ShimStatus::BudgetExhausted
        | ShimStatus::FrameIncomplete
        | ShimStatus::StoppedByUser
        | ShimStatus::StoppedBySystem
        | ShimStatus::Unrecognized(_) => InputFault::SubmissionFailed,
    }
}

/// Names the fault behind one native process-post failure.
///
/// A denial reported after event-post access was granted is the caller's focus
/// predicate that could not be observed at all, which is an authorization
/// answer rather than an unexplained submission failure.
fn process_post_failure_fault(
    failure: shim::ProcessPostFailure,
    operation: &OperationContext,
) -> InputFault {
    if failure.status != ShimStatus::PermissionDenied {
        return process_status_fault(failure.status, operation);
    }
    if failure.focus == shim::ProcessFocusObservation::Unavailable {
        return InputFault::NotAuthorized;
    }
    match failure.authorization {
        shim::ProcessAuthorizationObservation::NotGranted => InputFault::NotAuthorized,
        shim::ProcessAuthorizationObservation::Unknown
        | shim::ProcessAuthorizationObservation::Granted
        | shim::ProcessAuthorizationObservation::Unavailable => InputFault::SubmissionFailed,
    }
}

/// The policy one process-directed native unit is committed under.
///
/// Grouped rather than passed positionally because geometry, purpose, focus, and
/// modifier state are all read together by the final native gate, and a release
/// differs from ordinary input in more than one of them at once.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ProcessCommit {
    pub(crate) geometry: shim::ProcessGeometry,
    pub(crate) purpose: shim::ProcessPostPurpose,
    pub(crate) focus: shim::ProcessFocusRequirement,
    pub(crate) flags: u32,
}

impl ProcessCommit {
    /// Names one ordinary input commit under the caller's focus predicate.
    const fn input(
        geometry: shim::ProcessGeometry,
        focus: shim::ProcessFocusRequirement,
        flags: u32,
    ) -> Self {
        Self {
            geometry,
            purpose: shim::ProcessPostPurpose::Input,
            focus,
            flags,
        }
    }

    /// Names one sequence-owned release.
    ///
    /// A release requires no focus predicate and no ordinary window admission: a
    /// window that stopped being frontmost is exactly when a held key or button
    /// most needs releasing.
    pub(crate) const fn release(flags: u32) -> Self {
        Self {
            geometry: shim::ProcessGeometry::AuthorityOnly,
            purpose: shim::ProcessPostPurpose::Release,
            focus: shim::ProcessFocusRequirement::None,
            flags,
        }
    }
}

fn commit_process<S: ProcessCommitSource + ?Sized>(
    source: &S,
    event_source: Option<&shim::ProcessEventSource>,
    geometry: shim::ProcessGeometry,
    focus: shim::ProcessFocusRequirement,
    operation: &OperationContext,
    post: NativePost<'_>,
    flags: u32,
) -> SubmissionResult {
    commit_process_for(
        source,
        event_source,
        ProcessCommit::input(geometry, focus, flags),
        operation,
        post,
    )
}

/// Posts one prepared native unit through the process-directed route.
fn commit_process_for<S: ProcessCommitSource + ?Sized>(
    source: &S,
    event_source: Option<&shim::ProcessEventSource>,
    commit: ProcessCommit,
    operation: &OperationContext,
    post: NativePost<'_>,
) -> SubmissionResult {
    let ProcessCommit {
        geometry,
        purpose,
        focus,
        flags,
    } = commit;
    operation_fault(operation)?;
    let cancellation = operation.cancellation();
    let expected_units = post.process_native_units();
    let result = source.post_process(
        event_source,
        shim::ProcessPostRequest {
            post: post.process_post(),
            geometry,
            purpose,
            focus,
            flags,
            wait: PROCESS_OBSERVATION_WAIT,
        },
        operation,
    );
    let cancelled = cancellation.is_some_and(CancellationToken::is_cancelled);
    match result {
        Ok(outcome) if cancelled => Err(SubmissionFailure::after_native_units(
            InputFault::Cancelled,
            usize::try_from(outcome.invoked_native_units).unwrap_or(usize::MAX),
        )),
        Ok(outcome)
            if outcome.invoked_native_units == expected_units
                && outcome.target_match_count == purpose.expected_target_match_count() =>
        {
            Ok(())
        }
        Ok(outcome) => Err(SubmissionFailure::after_native_units(
            InputFault::SubmissionFailed,
            usize::try_from(outcome.invoked_native_units).unwrap_or(usize::MAX),
        )),
        Err(failure) if cancelled => Err(SubmissionFailure::after_native_attempt(
            InputFault::Cancelled,
            usize::try_from(failure.invoked_native_units).unwrap_or(usize::MAX),
            failure.native_effect_may_have_occurred,
        )),
        Err(failure) => {
            let fault = shim::catch_panic(|| source.classify_process_failure(failure, operation))
                .unwrap_or(InputFault::SubmissionFailed);
            Err(SubmissionFailure::after_native_attempt(
                fault,
                usize::try_from(failure.invoked_native_units).unwrap_or(usize::MAX),
                failure.native_effect_may_have_occurred,
            ))
        }
    }
}

/// Arbitrates, revalidates, and then posts without running caller code between
/// the final mutable authority observation and the irreversible native call.
pub(crate) fn commit_prepared<S: SystemCommitSource + ?Sized>(
    source: &S,
    focus: FocusPolicy,
    geometry: CommitGeometry,
    operation: &OperationContext,
    post: NativePost<'_>,
    flags: u32,
) -> SubmissionResult {
    let mut prepared = source
        .prepare(post, flags)
        .map_err(|status| SubmissionFailure::before_event(source.classify_post_failure(status)))?;
    let unit_count = source.prepared_unit_count(&prepared);
    if unit_count == 0 {
        return Err(InputFault::SubmissionFailed.into());
    }
    for (invoked_native_units, index) in (0..unit_count).enumerate() {
        let checkpoint = SystemCommitCheckpoint::new(operation)
            .map_err(|fault| SubmissionFailure::after_native_units(fault, invoked_native_units))?;
        if let Err(fault) = source.revalidate_system_commit(focus, geometry, checkpoint.budget()) {
            return Err(SubmissionFailure::after_native_units(
                fault_after_failed_gate(operation, fault),
                invoked_native_units,
            ));
        }
        checkpoint
            .check_final_fence()
            .map_err(|fault| SubmissionFailure::after_native_units(fault, invoked_native_units))?;
        if let Err(failure) = source.post_prepared_unit(
            &mut prepared,
            index,
            checkpoint.budget().deadline_nanos(),
            checkpoint.cancellation(),
        ) {
            let fault = prepared_post_failure_fault(source, operation, failure);
            return Err(SubmissionFailure::after_native_attempt(
                fault,
                invoked_native_units,
                failure.native_effect_may_have_occurred,
            ));
        }
    }
    Ok(())
}

fn commit_cleanup<S: SystemCommitSource + ?Sized>(
    source: &S,
    cleanup: &OperationContext,
    post: NativePost<'_>,
    flags: u32,
) -> Result<(), InputFault> {
    let mut prepared = source
        .prepare(post, flags)
        .map_err(|status| source.classify_post_failure(status))?;
    let unit_count = source.prepared_unit_count(&prepared);
    if unit_count == 0 {
        return Err(InputFault::SubmissionFailed);
    }
    for index in 0..unit_count {
        let checkpoint = SystemCommitCheckpoint::new(cleanup)?;
        if let Err(fault) = source.revalidate_cleanup_authorization(checkpoint.budget()) {
            return Err(fault_after_failed_gate(cleanup, fault));
        }
        checkpoint.check_final_fence()?;
        source
            .post_prepared_unit(
                &mut prepared,
                index,
                checkpoint.budget().deadline_nanos(),
                checkpoint.cancellation(),
            )
            .map_err(|failure| prepared_post_failure_fault(source, cleanup, failure))?;
    }
    Ok(())
}

fn commit_geometry(
    policy: GeometryPolicy,
    fingerprint: GeometryFingerprint,
) -> Result<CommitGeometry, InputFault> {
    match policy {
        GeometryPolicy::ReprojectCurrent | GeometryPolicy::RequireUnchanged => {
            Ok(CommitGeometry::RequireCurrent(fingerprint))
        }
        GeometryPolicy::UseFrameSnapshot => Ok(CommitGeometry::UseFrameSnapshot),
        _ => Err(InputFault::UnsupportedCombination),
    }
}

fn fingerprint(transform: &TransformSnapshot) -> Result<GeometryFingerprint, InputFault> {
    Ok(GeometryFingerprint {
        extent: transform.frame_extent(),
        placement: transform
            .target()
            .ok_or(InputFault::UnsupportedCoordinate)?,
    })
}

/// Reports whether a global point lies inside the target's own rectangle.
///
/// Half-open, so the far edge belongs to whatever is next to the target rather
/// than to the target.
fn contains_desktop_point(geometry: GeometryFingerprint, point: (f64, f64)) -> bool {
    let (left, top) = geometry.placement.desktop_origin();
    let (width, height) = geometry.placement.logical_size();
    point.0 >= left && point.0 < left + width && point.1 >= top && point.1 < top + height
}

/// Converts a point size at `scale` into the capture extent it covers.
fn extent_from_points(size: (f64, f64), scale: f64) -> Result<PixelExtent, InputFault> {
    Scale::new(scale, scale).map_err(|_| InputFault::UnsupportedCoordinate)?;
    let width = pixels_from_points(size.0, scale)?;
    let height = pixels_from_points(size.1, scale)?;
    Ok(PixelExtent::new(width, height))
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn pixels_from_points(points: f64, scale: f64) -> Result<u32, InputFault> {
    let pixels = (points * scale).round();
    if !pixels.is_finite() || pixels < 1.0 || pixels > f64::from(u32::MAX) {
        return Err(InputFault::UnsupportedCoordinate);
    }
    // The finite bounds check above makes this narrowing exact and non-negative.
    Ok(pixels as u32)
}

fn native_button(button: PointerButton) -> Result<u32, InputFault> {
    match button {
        PointerButton::Primary => Ok(shim::INPUT_BUTTON_PRIMARY),
        PointerButton::Secondary => Ok(shim::INPUT_BUTTON_SECONDARY),
        PointerButton::Middle => Ok(shim::INPUT_BUTTON_MIDDLE),
        _ => Err(InputFault::UnsupportedCombination),
    }
}

/// Returns the modifier flag a key contributes while it is held, if any.
fn key_flag(key: Key) -> u32 {
    match key {
        Key::Modifier(modifier) => modifier_flag(modifier),
        _ => 0,
    }
}

/// Resolves one logical key to a hardware key code.
///
/// Everything but a printable character is a fixed position on the keyboard map.
/// A character is resolved through the active layout and refused when that layout
/// produces it only with modifiers, because pressing the key the caller named
/// would then deliver a different character.
pub(crate) fn resolve_key_code(key: Key) -> Result<u16, InputFault> {
    match key {
        Key::Character(character) => shim::input_resolve_character(u32::from(character))
            .map_err(|_| InputFault::UnsupportedCombination),
        Key::Function(number) => FUNCTION_KEYS
            .get(usize::from(number).wrapping_sub(1))
            .copied()
            .ok_or(InputFault::UnsupportedCombination),
        Key::Modifier(Modifier::Shift) => Ok(KEY_SHIFT),
        Key::Modifier(Modifier::Control) => Ok(KEY_CONTROL),
        Key::Modifier(Modifier::Alt) => Ok(KEY_OPTION),
        Key::Modifier(Modifier::Meta) => Ok(KEY_COMMAND),
        Key::Enter => Ok(KEY_RETURN),
        Key::Tab => Ok(KEY_TAB),
        Key::Backspace => Ok(KEY_BACKSPACE),
        Key::Delete => Ok(KEY_FORWARD_DELETE),
        Key::Escape => Ok(KEY_ESCAPE),
        Key::Space => Ok(KEY_SPACE),
        Key::ArrowUp => Ok(KEY_ARROW_UP),
        Key::ArrowDown => Ok(KEY_ARROW_DOWN),
        Key::ArrowLeft => Ok(KEY_ARROW_LEFT),
        Key::ArrowRight => Ok(KEY_ARROW_RIGHT),
        Key::Home => Ok(KEY_HOME),
        Key::End => Ok(KEY_END),
        Key::PageUp => Ok(KEY_PAGE_UP),
        Key::PageDown => Ok(KEY_PAGE_DOWN),
        // A key this build does not know about has no code to post.
        _ => Err(InputFault::UnsupportedCombination),
    }
}

/// Returns the extent and placement a live rectangle describes, for tests.
#[cfg(test)]
pub(crate) fn placement_for(
    origin: (f64, f64),
    size: (f64, f64),
    scale: f64,
) -> Result<(PixelExtent, mado_pilot_core::TargetPlacement), InputFault> {
    let extent = extent_from_points(size, scale)?;
    let placement = placement_from_points(origin, size, scale, extent)
        .map_err(|_| InputFault::UnsupportedCoordinate)?;
    Ok((extent, placement))
}

#[cfg(test)]
mod tests;
