//! Thin `CGEvent` delivery backend for the shared macOS input controller.
//!
//! # What makes a receipt truthful here
//!
//! macOS does not fail a synthesized event posted by an untrusted process: it
//! discards it. `CGEventPost` returns nothing either way. So the Accessibility
//! decision is read again immediately before every irreversible event rather than
//! once at open, and a revocation observed mid-sequence stops delivery with the
//! count that had already gone out. Everything this module reports about delivery
//! rests on that check rather than on a return value the platform does not give.

use std::sync::Arc;

use mado_pilot_core::{
    CoordinateSpace, GeometryRevision, InputDelivery, OperationContext, PermissionState,
    PixelExtent, Point, Scale, TargetKind, TransformSnapshot,
};
use mado_pilot_input::{
    FocusPolicy, GeometryPolicy, InputEvent, InputFault, Key, Modifier, PointerButton,
    PointerGeometry, PressedState,
};

use crate::discovery::placement_from_points;
use crate::input::{
    DeliveryFailure, DriverState, GeometryFingerprint, InputDriver, PointerState,
    SystemButtonState, SystemKeyState,
};
use crate::provider::TargetRecord;
use crate::shim::{self, ShimStatus};

type DeliveryResult = Result<(), DeliveryFailure>;

/// How long activation and one read-only Accessibility focus observation may
/// consume, and how often activation is polled.
///
/// Both waits are additionally bounded by the caller's operation. The native
/// focus observation carries the remaining slice into Accessibility messaging,
/// so an unresponsive application cannot hold input past it.
const ACTIVATION_SETTLE: std::time::Duration = std::time::Duration::from_millis(250);
const ACTIVATION_POLL: std::time::Duration = std::time::Duration::from_millis(10);
const FOCUS_OBSERVATION_WAIT: std::time::Duration = std::time::Duration::from_millis(250);

fn focus_wait(operation: &OperationContext) -> std::time::Duration {
    operation
        .remaining()
        .map_or(FOCUS_OBSERVATION_WAIT, |remaining| {
            remaining.min(FOCUS_OBSERVATION_WAIT)
        })
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
/// accepts as a number. Those are reported unsupported rather than delivered as
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
    },
    Key {
        key_code: u16,
        down: bool,
    },
    Text(&'units [u16]),
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

/// The mutable native state consulted at the final commit boundary.
pub(crate) trait SystemCommitSource {
    fn revalidate_system_commit(
        &self,
        focus: FocusPolicy,
        geometry: CommitGeometry,
        operation: &OperationContext,
    ) -> Result<(), InputFault>;

    /// Re-reads the non-prompting authorization needed for a cleanup release.
    ///
    /// Cleanup deliberately does not consult focus or geometry: a release must
    /// still be attempted after either changes. Authorization is different
    /// because macOS silently discards an untrusted post, so an absent grant
    /// cannot truthfully be reported as a completed cleanup.
    fn revalidate_cleanup_authorization(&self) -> Result<(), InputFault>;

    /// Posts one prepared event, reporting how many text units had reached the
    /// target when a text post could not finish.
    fn post(&self, post: NativePost<'_>, flags: u32) -> Result<(), (ShimStatus, usize)>;

    /// Classifies a post that failed with nothing yet delivered by that event.
    fn classify_post_failure(&self, status: ShimStatus) -> InputFault;
}

pub(crate) struct NativeInputDriver {
    record: Arc<TargetRecord>,
}

impl NativeInputDriver {
    pub(crate) fn new(record: Arc<TargetRecord>) -> Self {
        Self { record }
    }

    /// Reads the Accessibility decision without requesting it.
    ///
    /// Anything but a granted decision is refused. An unavailable or unreadable
    /// state is not authorization, and treating it as one would post events macOS
    /// silently drops while the receipt claimed they were delivered.
    fn ensure_authorized(&self) -> Result<(), InputFault> {
        match shim::probe_accessibility() {
            Ok(PermissionState::Granted) => Ok(()),
            _ => Err(InputFault::NotAuthorized),
        }
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
            Err(_) => Err(InputFault::DeliveryFailed),
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

    /// Asks macOS to activate the owning application and reads exact focus back.
    ///
    /// This activates an application; it never raises one particular window.
    /// The retained window must still match the active application's focused
    /// Accessibility window one-to-one before delivery is accepted.
    fn activate(&self, operation: &OperationContext) -> Result<(), InputFault> {
        match shim::input_activate_owner(self.record.owner_process()) {
            Ok(()) => {}
            Err(ShimStatus::TargetLost) => return Err(InputFault::TargetLost),
            Err(ShimStatus::Unsupported) => return Err(InputFault::FocusRefused),
            Err(_) => return Err(InputFault::FocusRefused),
        }
        let deadline = operation.now().checked_add(ACTIVATION_SETTLE);
        loop {
            if let Some(interruption) = operation.interruption() {
                return Err(InputFault::from(interruption));
            }
            if self.is_focused(operation)? {
                return Ok(());
            }
            match deadline {
                Some(deadline) if operation.now() < deadline => {
                    std::thread::sleep(ACTIVATION_POLL);
                }
                // Either the wait is over, or the clock domain cannot express one.
                // Both mean the activation is not going to be waited on further.
                _ => return Err(InputFault::FocusRefused),
            }
        }
    }

    fn current_geometry(&self) -> Result<(TransformSnapshot, GeometryFingerprint), InputFault> {
        let bounds = self.record.current_bounds()?;
        let extent = extent_from_points(bounds.size, bounds.scale)?;
        let placement = placement_from_points(bounds.origin, bounds.size, bounds.scale, extent)
            .map_err(|_| InputFault::UnsupportedCoordinate)?;
        let transform = TransformSnapshot::with_target(GeometryRevision::FIRST, extent, placement)
            .map_err(|_| InputFault::UnsupportedCoordinate)?;
        Ok((transform, GeometryFingerprint { extent, placement }))
    }

    fn policy_geometry(
        &self,
        geometry: PointerGeometry,
    ) -> Result<(TransformSnapshot, GeometryFingerprint), InputFault> {
        match geometry.policy() {
            GeometryPolicy::ReprojectCurrent => self.current_geometry(),
            GeometryPolicy::RequireUnchanged => {
                let transform = self.source_transform(geometry)?;
                let source_fingerprint = fingerprint(&transform)?;
                let (_, current_fingerprint) = self.current_geometry()?;
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

    fn source_transform(&self, geometry: PointerGeometry) -> Result<TransformSnapshot, InputFault> {
        let source = geometry
            .source()
            .ok_or(InputFault::MissingCoordinateSource)?;
        self.record
            .geometry()
            .source_transform(source)
            .ok_or(InputFault::UnsupportedCoordinate)
    }

    fn resolve_pointer(
        &self,
        point: Point,
        geometry: PointerGeometry,
    ) -> Result<PointerState, InputFault> {
        let (transform, fingerprint) = self.policy_geometry(geometry)?;
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
    ) -> Result<PointerState, InputFault> {
        let (_, current) = self.policy_geometry(geometry)?;
        if let Some(pointer) = state.pointer {
            if pointer.geometry != current {
                return Err(InputFault::GeometryChanged);
            }
            return Ok(pointer);
        }

        let location = shim::input_pointer_location().map_err(|_| InputFault::DeliveryFailed)?;
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
    ) -> DeliveryResult {
        self.record.ensure_live()?;
        self.ensure_authorized()?;
        self.ensure_system_focus(focus, operation)?;
        let flags = state.held_flags();

        match event {
            InputEvent::PointerMove(point) => {
                let resolved = self.resolve_pointer(*point, geometry)?;
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
                let pointer = self.pointer_for_non_move(geometry, state)?;
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
                let pointer = self.pointer_for_non_move(geometry, state)?;
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
                let pointer = self.pointer_for_non_move(geometry, state)?;
                commit_prepared(
                    self,
                    focus,
                    commit_geometry(geometry.policy(), pointer.geometry)?,
                    operation,
                    NativePost::Scroll {
                        horizontal: i32::from(*horizontal),
                        vertical: i32::from(*vertical),
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
            InputEvent::Text(text) => self.deliver_text(text, focus, operation, flags),
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
    /// # Why no cleanup budget reaches here
    ///
    /// The Windows Adapter passes one, because Win32 can accept the down half of a
    /// UTF-16 unit and leave it pressed, which is a release cleanup has to send.
    /// Core Graphics posts a chunk's down and up as two complete events, so a
    /// failure between them leaves no pressed state this sequence recorded and
    /// there is nothing for a budget to bound.
    fn deliver_text(
        &self,
        text: &str,
        focus: FocusPolicy,
        operation: &OperationContext,
        flags: u32,
    ) -> DeliveryResult {
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
    fn revalidate_system_commit(
        &self,
        focus: FocusPolicy,
        geometry: CommitGeometry,
        operation: &OperationContext,
    ) -> Result<(), InputFault> {
        self.record.ensure_live()?;
        // Authorization is re-read here and not only at preflight, because macOS
        // revokes it while a process is running and discards the events that
        // follow without saying so.
        self.ensure_authorized()?;
        self.ensure_system_focus(focus, operation)?;
        if let CommitGeometry::RequireCurrent(expected) = geometry {
            let (_, current) = self.current_geometry()?;
            if current != expected {
                return Err(InputFault::GeometryChanged);
            }
        }
        Ok(())
    }

    fn revalidate_cleanup_authorization(&self) -> Result<(), InputFault> {
        self.ensure_authorized()
    }

    fn post(&self, post: NativePost<'_>, flags: u32) -> Result<(), (ShimStatus, usize)> {
        match post {
            NativePost::Pointer {
                action,
                button,
                click_state,
                location,
            } => shim::input_post_pointer(action, button, click_state, location, flags)
                .map_err(|status| (status, 0)),
            NativePost::Scroll {
                horizontal,
                vertical,
            } => shim::input_post_scroll(horizontal, vertical, flags).map_err(|status| (status, 0)),
            NativePost::Key { key_code, down } => {
                shim::input_post_key(key_code, down, flags).map_err(|status| (status, 0))
            }
            NativePost::Text(units) => shim::input_post_text(units, flags),
        }
    }

    fn classify_post_failure(&self, status: ShimStatus) -> InputFault {
        match status {
            ShimStatus::TargetLost => InputFault::TargetLost,
            ShimStatus::PermissionDenied => InputFault::NotAuthorized,
            ShimStatus::Unsupported => InputFault::UnsupportedCombination,
            _ if self.ensure_authorized().is_err() => InputFault::NotAuthorized,
            _ if self.record.ensure_live().is_err() => InputFault::TargetLost,
            _ => InputFault::DeliveryFailed,
        }
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
        operation: &OperationContext,
    ) -> Result<(), InputFault> {
        operation_fault(operation)?;
        self.record.ensure_live()?;
        match delivery {
            InputDelivery::System => {
                self.ensure_authorized()?;
                self.ensure_system_focus(focus, operation)
            }
            // There is no macOS background channel, so this is refused rather than
            // substituted: a caller that asked not to disturb the desktop did not
            // ask for system input.
            _ => Err(InputFault::UnsupportedCombination),
        }
    }

    fn deliver(
        &self,
        delivery: InputDelivery,
        focus: FocusPolicy,
        event: &InputEvent,
        geometry: PointerGeometry,
        state: &mut DriverState,
        operation: &OperationContext,
    ) -> DeliveryResult {
        operation_fault(operation)?;
        match delivery {
            InputDelivery::System => self.deliver_system(focus, event, geometry, state, operation),
            _ => Err(InputFault::UnsupportedCombination.into()),
        }
    }

    fn release(
        &self,
        delivery: InputDelivery,
        pressed: PressedState,
        state: &mut DriverState,
        operation: &OperationContext,
    ) -> Result<(), InputFault> {
        operation_fault(operation)?;
        if delivery != InputDelivery::System {
            return Err(InputFault::UnsupportedCombination);
        }
        release_system(pressed, state, self, operation)
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

fn operation_fault(operation: &OperationContext) -> Result<(), InputFault> {
    operation
        .interruption()
        .map_or(Ok(()), |interruption| Err(InputFault::from(interruption)))
}

/// Revalidates and then posts, with arbitration adjacent to the irreversible act.
pub(crate) fn commit_prepared<S: SystemCommitSource + ?Sized>(
    source: &S,
    focus: FocusPolicy,
    geometry: CommitGeometry,
    operation: &OperationContext,
    post: NativePost<'_>,
    flags: u32,
) -> DeliveryResult {
    operation_fault(operation)?;
    source.revalidate_system_commit(focus, geometry, operation)?;
    // Revalidation performs target, authorization, focus, and geometry queries, so
    // arbitration is checked once more as the last operation before the post.
    operation_fault(operation)?;
    source.post(post, flags).map_err(|(status, posted)| {
        if posted > 0 {
            DeliveryFailure::during_event(InputFault::DeliveryFailed)
        } else {
            DeliveryFailure::before_event(source.classify_post_failure(status))
        }
    })
}

fn commit_cleanup<S: SystemCommitSource + ?Sized>(
    source: &S,
    cleanup: &OperationContext,
    post: NativePost<'_>,
    flags: u32,
) -> Result<(), InputFault> {
    operation_fault(cleanup)?;
    source.revalidate_cleanup_authorization()?;
    // Keep cancellation/deadline arbitration adjacent to the release after the
    // authorization probe, just as ordinary commits do after their full gate.
    operation_fault(cleanup)?;
    source
        .post(post, flags)
        .map_err(|(status, _)| source.classify_post_failure(status))
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
