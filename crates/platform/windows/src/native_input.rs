//! Thin Win32 delivery backend for the shared Windows input controller.

use std::ffi::c_void;
use std::mem::size_of;
use std::sync::Arc;
use std::time::Duration;

use mado_pilot_core::{
    CoordinateSpace, GeometryRevision, InputDelivery, OperationContext, Point, TargetKind,
    TransformSnapshot,
};
use mado_pilot_input::{
    FocusPolicy, GeometryPolicy, InputEvent, InputFault, Key, PointerButton, PointerGeometry,
    PressedState,
};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, GetLastError, HANDLE, HWND, LPARAM, POINT, SetLastError,
    WIN32_ERROR, WPARAM,
};
use windows::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TOKEN_MANDATORY_LABEL,
    TOKEN_QUERY, TokenIntegrityLevel,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardLayout, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MOUSE_EVENT_FLAGS,
    MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN,
    MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput,
    VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_HOME,
    VK_LEFT, VK_LWIN, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB,
    VK_UP, VkKeyScanExW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetForegroundWindow, GetSystemMetrics, GetWindowThreadProcessId,
    SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_SWAPBUTTON, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SMTO_ABORTIFHUNG, SMTO_BLOCK, SendMessageTimeoutW, SetForegroundWindow, WM_COPYDATA,
};

use crate::discovery::{NativeKey, current_placement};
use crate::fixture_protocol::{ACKNOWLEDGED, CLASS_NAME, COPYDATA_TAG, encode_event, query_packet};
use crate::input::{
    DeliveryFailure, DriverState, GeometryFingerprint, InputDriver, PointerState,
    SystemButtonState, SystemKeyState,
};
use crate::provider::TargetRecord;

const BACKGROUND_EVENT_TIMEOUT: Duration = Duration::from_millis(100);
type DeliveryResult = Result<(), DeliveryFailure>;

pub(crate) struct NativeInputDriver {
    record: Arc<TargetRecord>,
}

impl NativeInputDriver {
    pub(crate) fn new(record: Arc<TargetRecord>) -> Self {
        Self { record }
    }

    fn hwnd(&self) -> Result<HWND, InputFault> {
        match self.record.key() {
            NativeKey::Window(raw) => {
                Ok(HWND(std::ptr::with_exposed_provenance_mut::<c_void>(raw)))
            }
            NativeKey::Display(_) => Err(InputFault::UnsupportedCombination),
        }
    }

    fn ensure_system_focus(&self, policy: FocusPolicy) -> Result<(), InputFault> {
        if self.record.kind() == TargetKind::Display {
            return Ok(());
        }
        let hwnd = self.hwnd()?;
        // SAFETY: GetForegroundWindow returns an opaque handle and performs no
        // dereference in this process.
        let focused = unsafe { GetForegroundWindow() } == hwnd;
        if focused {
            return Ok(());
        }
        match policy {
            FocusPolicy::Preserve | FocusPolicy::RequireFocused => Err(InputFault::FocusRequired),
            FocusPolicy::ActivateIfRequired => {
                // SAFETY: hwnd is the retained target's current native handle.
                if !unsafe { SetForegroundWindow(hwnd) }.as_bool() {
                    return Err(InputFault::FocusRefused);
                }
                // SetForegroundWindow can report success while foreground policy
                // changes concurrently, so the observable state is re-read.
                // SAFETY: as above.
                if unsafe { GetForegroundWindow() } == hwnd {
                    Ok(())
                } else {
                    Err(InputFault::FocusRefused)
                }
            }
            _ => Err(InputFault::UnsupportedCombination),
        }
    }

    fn current_geometry(&self) -> Result<(TransformSnapshot, GeometryFingerprint), InputFault> {
        self.record.ensure_live()?;
        let extent = self.record.current_extent()?;
        let placement =
            current_placement(self.record.key(), extent).ok_or(InputFault::TargetLost)?;
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
                let source = geometry
                    .source()
                    .ok_or(InputFault::MissingCoordinateSource)?;
                let transform = self
                    .record
                    .geometry()
                    .source_transform(source)
                    .ok_or(InputFault::UnsupportedCoordinate)?;
                let source_fingerprint = fingerprint(&transform)?;
                let (_, current_fingerprint) = self.current_geometry()?;
                if source_fingerprint != current_fingerprint {
                    return Err(InputFault::GeometryChanged);
                }
                Ok((transform, source_fingerprint))
            }
            GeometryPolicy::UseFrameSnapshot => {
                let source = geometry
                    .source()
                    .ok_or(InputFault::MissingCoordinateSource)?;
                let transform = self
                    .record
                    .geometry()
                    .source_transform(source)
                    .ok_or(InputFault::UnsupportedCoordinate)?;
                let fingerprint = fingerprint(&transform)?;
                Ok((transform, fingerprint))
            }
            _ => Err(InputFault::UnsupportedCombination),
        }
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
        let screen = point_to_screen(desktop, fingerprint)?;
        Ok(PointerState {
            screen,
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

        let mut cursor = POINT::default();
        // SAFETY: cursor is a complete writable POINT.
        unsafe { GetCursorPos(&raw mut cursor) }.map_err(|_| InputFault::DeliveryFailed)?;
        let screen = (cursor.x, cursor.y);
        if !contains_screen_point(current, screen) {
            return Err(InputFault::UnsupportedCoordinate);
        }
        let pointer = PointerState {
            screen,
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
    ) -> DeliveryResult {
        self.record.ensure_live()?;
        self.ensure_system_focus(focus)?;
        if target_has_higher_integrity(&self.record)? == Some(true) {
            return Err(InputFault::PolicyRefused.into());
        }

        match event {
            InputEvent::PointerMove(point) => {
                let resolved = self.resolve_pointer(*point, geometry)?;
                send_system_pointer_move(resolved.screen, &self.record)?;
                state.pointer = Some(resolved);
                Ok(())
            }
            InputEvent::PointerPress(button) => {
                let _pointer = self.pointer_for_non_move(geometry, state)?;
                let physical = physical_button(*button)?;
                send_physical_button(physical, true, &self.record)?;
                state.buttons.push(SystemButtonState {
                    logical: *button,
                    physical,
                });
                Ok(())
            }
            InputEvent::PointerRelease(button) => {
                let _pointer = self.pointer_for_non_move(geometry, state)?;
                let index = state
                    .buttons
                    .iter()
                    .rposition(|pressed| pressed.logical == *button);
                let physical = index
                    .map(|index| state.buttons[index].physical)
                    .map_or_else(|| physical_button(*button), Ok)?;
                send_physical_button(physical, false, &self.record)?;
                if let Some(index) = index {
                    state.buttons.remove(index);
                }
                Ok(())
            }
            InputEvent::PointerScroll {
                horizontal,
                vertical,
            } => {
                let _pointer = self.pointer_for_non_move(geometry, state)?;
                send_system_scroll(*horizontal, *vertical, &self.record)
            }
            InputEvent::KeyPress(key) => {
                let (virtual_key, extended) = resolve_virtual_key(*key, &self.record)?;
                send_resolved_key(virtual_key, extended, true, &self.record)?;
                state.keys.push(SystemKeyState {
                    logical: *key,
                    virtual_key: virtual_key.0,
                    extended,
                });
                Ok(())
            }
            InputEvent::KeyRelease(key) => {
                let index = state
                    .keys
                    .iter()
                    .rposition(|pressed| pressed.logical == *key);
                let (virtual_key, extended) = index
                    .map(|index| {
                        (
                            VIRTUAL_KEY(state.keys[index].virtual_key),
                            state.keys[index].extended,
                        )
                    })
                    .map_or_else(|| resolve_virtual_key(*key, &self.record), Ok)?;
                send_resolved_key(virtual_key, extended, false, &self.record)?;
                if let Some(index) = index {
                    state.keys.remove(index);
                }
                Ok(())
            }
            InputEvent::Text(text) => send_system_text(text, &self.record),
            InputEvent::Delay(_) => Err(InputFault::UnsupportedCombination.into()),
            _ => Err(InputFault::UnsupportedCombination.into()),
        }
    }

    fn deliver_background(
        &self,
        event: &InputEvent,
        geometry: PointerGeometry,
        state: &mut DriverState,
        operation: &OperationContext,
    ) -> DeliveryResult {
        let point = match event {
            InputEvent::PointerMove(point) => {
                let resolved = self.resolve_pointer(*point, geometry)?;
                state.pointer = Some(resolved);
                Some(resolved.screen)
            }
            InputEvent::PointerPress(_)
            | InputEvent::PointerRelease(_)
            | InputEvent::PointerScroll { .. } => {
                Some(self.pointer_for_non_move(geometry, state)?.screen)
            }
            _ => None,
        };
        let packet = encode_event(event, point)?;
        send_fixture_packet(&self.record, &packet, operation, InputFault::DeliveryFailed)
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
                self.ensure_system_focus(focus)?;
                if target_has_higher_integrity(&self.record)? == Some(true) {
                    Err(InputFault::PolicyRefused)
                } else {
                    Ok(())
                }
            }
            InputDelivery::BackgroundTarget => {
                if self.record.class_name() != Some(CLASS_NAME) {
                    return Err(InputFault::UnsupportedCombination);
                }
                if target_has_higher_integrity(&self.record)? == Some(true) {
                    return Err(InputFault::PolicyRefused);
                }
                send_fixture_packet(
                    &self.record,
                    &query_packet(),
                    operation,
                    InputFault::DeliveryUnavailable,
                )
                .map_err(|failure| failure.fault)
            }
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
            InputDelivery::System => self.deliver_system(focus, event, geometry, state),
            InputDelivery::BackgroundTarget => {
                self.deliver_background(event, geometry, state, operation)
            }
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
        match delivery {
            InputDelivery::System => send_system_release(pressed, state, &self.record),
            InputDelivery::BackgroundTarget => {
                let event = match pressed {
                    PressedState::Button(button) => InputEvent::PointerRelease(button),
                    PressedState::Key(key) => InputEvent::KeyRelease(key),
                    _ => return Err(InputFault::UnsupportedCombination),
                };
                let point = match event {
                    InputEvent::PointerRelease(_) => Some(
                        state
                            .pointer
                            .ok_or(InputFault::UnsupportedCoordinate)?
                            .screen,
                    ),
                    _ => None,
                };
                let packet = encode_event(&event, point)?;
                send_fixture_packet(&self.record, &packet, operation, InputFault::DeliveryFailed)
                    .map_err(|failure| failure.fault)
            }
            _ => Err(InputFault::UnsupportedCombination),
        }
    }
}

fn operation_fault(operation: &OperationContext) -> Result<(), InputFault> {
    operation
        .interruption()
        .map_or(Ok(()), |interruption| Err(InputFault::from(interruption)))
}

fn fingerprint(transform: &TransformSnapshot) -> Result<GeometryFingerprint, InputFault> {
    Ok(GeometryFingerprint {
        extent: transform.frame_extent(),
        placement: transform
            .target()
            .ok_or(InputFault::UnsupportedCoordinate)?,
    })
}

fn point_to_screen(point: Point, geometry: GeometryFingerprint) -> Result<(i32, i32), InputFault> {
    let x = round_i32(point.x())?;
    let y = round_i32(point.y())?;
    let screen = (x, y);
    if contains_screen_point(geometry, screen) {
        Ok(screen)
    } else {
        Err(InputFault::UnsupportedCoordinate)
    }
}

#[allow(clippy::cast_possible_truncation)]
fn round_i32(value: f64) -> Result<i32, InputFault> {
    let rounded = value.round();
    if rounded < f64::from(i32::MIN) || rounded > f64::from(i32::MAX) {
        return Err(InputFault::UnsupportedCoordinate);
    }
    // The finite i32 bounds check above makes this narrowing exact.
    Ok(rounded as i32)
}

fn contains_screen_point(geometry: GeometryFingerprint, point: (i32, i32)) -> bool {
    let (left, top) = geometry.placement.desktop_origin();
    let right = left + f64::from(geometry.extent.width());
    let bottom = top + f64::from(geometry.extent.height());
    let x = f64::from(point.0);
    let y = f64::from(point.1);
    x >= left && x < right && y >= top && y < bottom
}

fn send_system_pointer_move(screen: (i32, i32), record: &TargetRecord) -> DeliveryResult {
    let desktop = virtual_desktop()?;
    let dx = normalize_absolute(screen.0, desktop.0, desktop.2)?;
    let dy = normalize_absolute(screen.1, desktop.1, desktop.3)?;
    let input = mouse_input(
        dx,
        dy,
        0,
        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
    );
    send_exact(std::slice::from_ref(&input), record)
}

fn physical_button(button: PointerButton) -> Result<u8, InputFault> {
    // GetSystemMetrics is infallible for SM_SWAPBUTTON; nonzero means the user's
    // primary and secondary buttons are reversed.
    // SAFETY: fixed metric index.
    let swapped = unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0;
    match (button, swapped) {
        (PointerButton::Primary, false) | (PointerButton::Secondary, true) => Ok(1),
        (PointerButton::Primary, true) | (PointerButton::Secondary, false) => Ok(2),
        (PointerButton::Middle, _) => Ok(3),
        _ => Err(InputFault::UnsupportedCombination),
    }
}

fn send_physical_button(physical: u8, pressed: bool, record: &TargetRecord) -> DeliveryResult {
    let flags = match (physical, pressed) {
        (1, true) => MOUSEEVENTF_LEFTDOWN,
        (1, false) => MOUSEEVENTF_LEFTUP,
        (2, true) => MOUSEEVENTF_RIGHTDOWN,
        (2, false) => MOUSEEVENTF_RIGHTUP,
        (3, true) => MOUSEEVENTF_MIDDLEDOWN,
        (3, false) => MOUSEEVENTF_MIDDLEUP,
        _ => return Err(InputFault::UnsupportedCombination.into()),
    };
    let input = mouse_input(0, 0, 0, flags);
    send_exact(std::slice::from_ref(&input), record)
}

fn send_system_scroll(horizontal: i16, vertical: i16, record: &TargetRecord) -> DeliveryResult {
    const WHEEL_DELTA: i32 = 120;
    let mut inputs = Vec::with_capacity(2);
    if vertical != 0 {
        // The public convention is positive down; Win32 vertical wheel input is
        // positive away from the user (up).
        let delta = -i32::from(vertical) * WHEEL_DELTA;
        inputs.push(mouse_input(0, 0, delta.cast_unsigned(), MOUSEEVENTF_WHEEL));
    }
    if horizontal != 0 {
        let delta = i32::from(horizontal) * WHEEL_DELTA;
        inputs.push(mouse_input(0, 0, delta.cast_unsigned(), MOUSEEVENTF_HWHEEL));
    }
    send_exact(&inputs, record)
}

fn send_resolved_key(
    virtual_key: VIRTUAL_KEY,
    extended: bool,
    pressed: bool,
    record: &TargetRecord,
) -> DeliveryResult {
    let mut flags = KEYBD_EVENT_FLAGS(0);
    if extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if !pressed {
        flags |= KEYEVENTF_KEYUP;
    }
    let input = keyboard_input(virtual_key, 0, flags);
    send_exact(std::slice::from_ref(&input), record)
}

fn send_system_text(text: &str, record: &TargetRecord) -> DeliveryResult {
    let units = text.encode_utf16().collect::<Vec<_>>();
    let mut inputs = Vec::with_capacity(units.len().saturating_mul(2));
    for unit in &units {
        inputs.push(keyboard_input(VIRTUAL_KEY(0), *unit, KEYEVENTF_UNICODE));
        inputs.push(keyboard_input(
            VIRTUAL_KEY(0),
            *unit,
            KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
        ));
    }

    let inserted = raw_send(&inputs);
    if inserted == inputs.len() {
        return Ok(());
    }
    // If Windows accepted only the down half of one UTF-16 unit, perform one
    // bounded release before reporting that the Text event did not complete.
    if inserted % 2 == 1
        && let Some(unit) = units.get(inserted / 2)
    {
        let release = keyboard_input(VIRTUAL_KEY(0), *unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP);
        let _released = raw_send(std::slice::from_ref(&release));
    }
    Err(send_failure(record, inserted))
}

fn send_system_release(
    pressed: PressedState,
    state: &mut DriverState,
    record: &TargetRecord,
) -> Result<(), InputFault> {
    match pressed {
        PressedState::Button(button) => {
            let index = state
                .buttons
                .iter()
                .rposition(|pressed| pressed.logical == button);
            let physical = index
                .map(|index| state.buttons[index].physical)
                .map_or_else(|| physical_button(button), Ok)?;
            send_physical_button(physical, false, record).map_err(|failure| failure.fault)?;
            if let Some(index) = index {
                state.buttons.remove(index);
            }
            Ok(())
        }
        PressedState::Key(key) => {
            let index = state
                .keys
                .iter()
                .rposition(|pressed| pressed.logical == key);
            let (virtual_key, extended) = index
                .map(|index| {
                    (
                        VIRTUAL_KEY(state.keys[index].virtual_key),
                        state.keys[index].extended,
                    )
                })
                .map_or_else(|| resolve_virtual_key(key, record), Ok)?;
            send_resolved_key(virtual_key, extended, false, record)
                .map_err(|failure| failure.fault)?;
            if let Some(index) = index {
                state.keys.remove(index);
            }
            Ok(())
        }
        _ => Err(InputFault::UnsupportedCombination),
    }
}

fn mouse_input(dx: i32, dy: i32, data: u32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn keyboard_input(virtual_key: VIRTUAL_KEY, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: virtual_key,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send_exact(inputs: &[INPUT], record: &TargetRecord) -> DeliveryResult {
    let inserted = raw_send(inputs);
    if inserted == inputs.len() {
        Ok(())
    } else {
        Err(send_failure(record, inserted))
    }
}

fn raw_send(inputs: &[INPUT]) -> usize {
    if inputs.is_empty() {
        return 0;
    }
    let size = i32::try_from(size_of::<INPUT>()).expect("INPUT size fits i32");
    // SAFETY: inputs is a complete contiguous INPUT slice and cbSize names its
    // exact element layout.
    usize::try_from(unsafe { SendInput(inputs, size) }).unwrap_or(0)
}

fn send_failure(record: &TargetRecord, inserted: usize) -> DeliveryFailure {
    if inserted > 0 {
        return DeliveryFailure::during_event(InputFault::DeliveryFailed);
    }
    let fault = match target_has_higher_integrity(record) {
        Ok(Some(true)) => InputFault::PolicyRefused,
        Err(fault) => fault,
        Ok(_) if record.ensure_live().is_err() => InputFault::TargetLost,
        Ok(_) => {
            // Microsoft documents that SendInput's return and last-error value do
            // not identify UIPI. We claim PolicyRefused only when the independent
            // integrity comparison proves it.
            InputFault::DeliveryFailed
        }
    };
    DeliveryFailure::before_event(fault)
}

fn resolve_virtual_key(key: Key, record: &TargetRecord) -> Result<(VIRTUAL_KEY, bool), InputFault> {
    let value = match key {
        Key::Character(character) => {
            let mut encoded = [0u16; 2];
            let units = character.encode_utf16(&mut encoded);
            if units.len() != 1 {
                return Err(InputFault::UnsupportedCombination);
            }
            let hwnd = match record.key() {
                NativeKey::Window(raw) => {
                    HWND(std::ptr::with_exposed_provenance_mut::<c_void>(raw))
                }
                NativeKey::Display(_) => return Err(InputFault::UnsupportedCombination),
            };
            // SAFETY: hwnd is the retained target's current native handle.
            let thread = unsafe { GetWindowThreadProcessId(hwnd, None) };
            if thread == 0 {
                return Err(InputFault::TargetLost);
            }
            // SAFETY: thread is the current owner returned for hwnd.
            let layout = unsafe { GetKeyboardLayout(thread) };
            // SAFETY: layout is the target thread's current keyboard layout.
            let mapped = unsafe { VkKeyScanExW(units[0], layout) };
            if mapped == -1 || ((mapped.cast_unsigned() >> 8) & 0xff) != 0 {
                return Err(InputFault::UnsupportedCombination);
            }
            VIRTUAL_KEY(mapped.cast_unsigned() & 0xff)
        }
        Key::Function(number) if (1..=24).contains(&number) => {
            VIRTUAL_KEY(VK_F1.0 + u16::from(number - 1))
        }
        Key::Modifier(mado_pilot_input::Modifier::Shift) => VK_SHIFT,
        Key::Modifier(mado_pilot_input::Modifier::Control) => VK_CONTROL,
        Key::Modifier(mado_pilot_input::Modifier::Alt) => VK_MENU,
        Key::Modifier(mado_pilot_input::Modifier::Meta) => VK_LWIN,
        Key::Enter => VK_RETURN,
        Key::Tab => VK_TAB,
        Key::Backspace => VK_BACK,
        Key::Delete => VK_DELETE,
        Key::Escape => VK_ESCAPE,
        Key::Space => VK_SPACE,
        Key::ArrowUp => VK_UP,
        Key::ArrowDown => VK_DOWN,
        Key::ArrowLeft => VK_LEFT,
        Key::ArrowRight => VK_RIGHT,
        Key::Home => VK_HOME,
        Key::End => VK_END,
        Key::PageUp => VK_PRIOR,
        Key::PageDown => VK_NEXT,
        _ => return Err(InputFault::UnsupportedCombination),
    };
    let extended = matches!(
        key,
        Key::Modifier(mado_pilot_input::Modifier::Meta)
            | Key::Delete
            | Key::ArrowUp
            | Key::ArrowDown
            | Key::ArrowLeft
            | Key::ArrowRight
            | Key::Home
            | Key::End
            | Key::PageUp
            | Key::PageDown
    );
    Ok((value, extended))
}

fn virtual_desktop() -> Result<(i32, i32, i32, i32), InputFault> {
    // SAFETY: the call uses a fixed documented metric index.
    let left = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    // SAFETY: the call uses a fixed documented metric index.
    let top = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    // SAFETY: the call uses a fixed documented metric index.
    let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    // SAFETY: the call uses a fixed documented metric index.
    let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    if width <= 0 || height <= 0 {
        return Err(InputFault::UnsupportedCoordinate);
    }
    Ok((left, top, width, height))
}

fn normalize_absolute(value: i32, origin: i32, extent: i32) -> Result<i32, InputFault> {
    let offset = i64::from(value) - i64::from(origin);
    if offset < 0 || offset >= i64::from(extent) {
        return Err(InputFault::UnsupportedCoordinate);
    }
    if extent == 1 {
        return Ok(0);
    }
    let normalized = offset
        .checked_mul(65_535)
        .ok_or(InputFault::UnsupportedCoordinate)?
        / i64::from(extent - 1);
    i32::try_from(normalized).map_err(|_| InputFault::UnsupportedCoordinate)
}

#[repr(C)]
struct CopyData {
    tag: usize,
    bytes: u32,
    data: *mut c_void,
}

fn send_fixture_packet(
    record: &TargetRecord,
    packet: &[u8],
    operation: &OperationContext,
    ordinary_failure: InputFault,
) -> DeliveryResult {
    operation_fault(operation)?;
    record.ensure_live()?;
    if record.class_name() != Some(CLASS_NAME) {
        return Err(InputFault::UnsupportedCombination.into());
    }
    let hwnd = match record.key() {
        NativeKey::Window(raw) => HWND(std::ptr::with_exposed_provenance_mut::<c_void>(raw)),
        NativeKey::Display(_) => return Err(InputFault::UnsupportedCombination.into()),
    };
    let bytes = u32::try_from(packet.len()).map_err(|_| InputFault::SequenceOutOfBounds)?;
    let copy = CopyData {
        tag: COPYDATA_TAG,
        bytes,
        data: packet.as_ptr().cast_mut().cast::<c_void>(),
    };
    let timeout = timeout_millis(operation)?;
    let mut acknowledged = 0usize;

    // Clear stale last-error state so ERROR_ACCESS_DENIED is attributed to this
    // call. The pointer remains valid because SendMessageTimeoutW is synchronous
    // and WM_COPYDATA copies the payload before returning.
    // SAFETY: hwnd is the retained target; copy points to packet for this call.
    let sent = unsafe {
        SetLastError(WIN32_ERROR(0));
        SendMessageTimeoutW(
            hwnd,
            WM_COPYDATA,
            WPARAM(0),
            LPARAM((&raw const copy).addr().cast_signed()),
            SMTO_ABORTIFHUNG | SMTO_BLOCK,
            timeout,
            Some(&raw mut acknowledged),
        )
    };
    if sent.0 == 0 {
        // SAFETY: immediately follows the failing Win32 call on this thread.
        let error = unsafe { GetLastError() };
        return if error == ERROR_ACCESS_DENIED {
            Err(DeliveryFailure::before_event(InputFault::PolicyRefused))
        } else if record.ensure_live().is_err() {
            Err(DeliveryFailure::during_event(InputFault::TargetLost))
        } else {
            Err(DeliveryFailure::during_event(ordinary_failure))
        };
    }
    if acknowledged == ACKNOWLEDGED {
        Ok(())
    } else {
        Err(DeliveryFailure::during_event(ordinary_failure))
    }
}

fn timeout_millis(operation: &OperationContext) -> Result<u32, InputFault> {
    operation_fault(operation)?;
    let duration = operation
        .remaining()
        .map_or(BACKGROUND_EVENT_TIMEOUT, |remaining| {
            remaining.min(BACKGROUND_EVENT_TIMEOUT)
        });
    if duration.is_zero() {
        return Err(InputFault::DeadlineExceeded);
    }
    let millis = duration.as_millis().max(1);
    Ok(u32::try_from(millis).unwrap_or(u32::MAX))
}

fn target_has_higher_integrity(record: &TargetRecord) -> Result<Option<bool>, InputFault> {
    let hwnd = match record.key() {
        NativeKey::Window(raw) => HWND(std::ptr::with_exposed_provenance_mut::<c_void>(raw)),
        NativeKey::Display(_) => return Ok(Some(false)),
    };
    let mut process_id = 0u32;
    // SAFETY: process_id is writable and hwnd is the retained target.
    if unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut process_id)) } == 0 || process_id == 0
    {
        return Err(InputFault::TargetLost);
    }

    // Failure to inspect an otherwise-live process is not enough evidence to
    // claim UIPI. Delivery proceeds and maps only an independently proven
    // higher integrity or ERROR_ACCESS_DENIED to PolicyRefused.
    // SAFETY: process_id came from the current target window.
    let target = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }
    {
        Ok(handle) => OwnedHandle::new(handle),
        Err(_) => return Ok(None),
    };
    // SAFETY: pseudo handle for this process; it must not be closed.
    let current = unsafe { GetCurrentProcess() };
    let Some(current_level) = integrity_level(current) else {
        return Ok(None);
    };
    let Some(target_level) = integrity_level(target.raw()) else {
        return Ok(None);
    };
    Ok(Some(target_level > current_level))
}

fn integrity_level(process: HANDLE) -> Option<u32> {
    let mut token = HANDLE::default();
    // SAFETY: token is writable and process is a live process handle or the
    // documented current-process pseudo handle.
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut token) }.ok()?;
    let token = OwnedHandle::new(token);

    let mut required = 0u32;
    // The first call intentionally obtains the required byte count.
    // SAFETY: required is writable; a null output buffer with zero length is the
    // documented sizing query.
    let _sizing = unsafe {
        GetTokenInformation(token.raw(), TokenIntegrityLevel, None, 0, &raw mut required)
    };
    if required == 0 {
        return None;
    }
    let word = size_of::<usize>();
    let words = usize::try_from(required).ok()?.checked_add(word - 1)? / word;
    let mut storage = vec![0usize; words];
    let bytes = u32::try_from(storage.len().checked_mul(word)?).ok()?;
    // SAFETY: storage is aligned for TOKEN_MANDATORY_LABEL, spans bytes, and
    // remains alive while the returned SID pointers are read.
    unsafe {
        GetTokenInformation(
            token.raw(),
            TokenIntegrityLevel,
            Some(storage.as_mut_ptr().cast::<c_void>()),
            bytes,
            &raw mut required,
        )
    }
    .ok()?;

    // SAFETY: GetTokenInformation initialized a TOKEN_MANDATORY_LABEL at the
    // aligned start of storage.
    let label = unsafe { &*storage.as_ptr().cast::<TOKEN_MANDATORY_LABEL>() };
    let sid = label.Label.Sid;
    // SAFETY: Sid belongs to the initialized token-information buffer.
    let count = unsafe { GetSidSubAuthorityCount(sid) };
    if count.is_null() {
        return None;
    }
    // SAFETY: count points into the valid SID.
    let count = u32::from(unsafe { *count });
    if count == 0 {
        return None;
    }
    // SAFETY: count - 1 is the final sub-authority of the valid SID.
    let level = unsafe { GetSidSubAuthority(sid, count - 1) };
    if level.is_null() {
        None
    } else {
        // SAFETY: level points into the valid SID.
        Some(unsafe { *level })
    }
}

struct OwnedHandle(HANDLE);

impl OwnedHandle {
    fn new(handle: HANDLE) -> Self {
        Self(handle)
    }

    fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: this wrapper owns one ordinary kernel handle. It is never
            // constructed for the current-process pseudo handle.
            let _closed = unsafe { CloseHandle(self.0) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{normalize_absolute, point_to_screen};
    use crate::input::GeometryFingerprint;
    use mado_pilot_core::{CoordinateSpace, PixelExtent, Point, Scale, TargetPlacement};
    use mado_pilot_input::InputFault;

    #[test]
    fn signed_virtual_desktop_coordinates_normalize_at_both_edges() {
        assert_eq!(normalize_absolute(-1920, -1920, 3840), Ok(0));
        assert_eq!(normalize_absolute(1919, -1920, 3840), Ok(65_535));
        assert_eq!(
            normalize_absolute(1920, -1920, 3840),
            Err(InputFault::UnsupportedCoordinate)
        );
    }

    #[test]
    fn a_mixed_dpi_transform_keeps_physical_virtual_desktop_coordinates() {
        let geometry = GeometryFingerprint {
            extent: PixelExtent::new(1920, 1080),
            placement: TargetPlacement::new(
                (-1920.0, -120.0),
                (1280.0, 720.0),
                Scale::new(1.5, 1.5).expect("scale"),
            )
            .expect("placement")
            .with_desktop_scale(Scale::new(1.0, 1.0).expect("desktop scale")),
        };
        let point = Point::new(CoordinateSpace::DesktopLogical, -1919.6, -119.6).expect("point");

        assert_eq!(point_to_screen(point, geometry), Ok((-1920, -120)));
    }

    #[test]
    fn the_far_edge_is_not_silently_clamped_into_an_adjacent_target() {
        let geometry = GeometryFingerprint {
            extent: PixelExtent::new(100, 50),
            placement: TargetPlacement::new(
                (10.0, 20.0),
                (100.0, 50.0),
                Scale::new(1.0, 1.0).expect("scale"),
            )
            .expect("placement"),
        };
        let edge = Point::new(CoordinateSpace::DesktopLogical, 110.0, 30.0).expect("point");

        assert_eq!(
            point_to_screen(edge, geometry),
            Err(InputFault::UnsupportedCoordinate)
        );
    }
}
