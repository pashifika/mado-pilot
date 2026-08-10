//! Conservative legacy-message translation and per-unit submission accounting.

use mado_pilot_core::OperationContext;
use mado_pilot_input::{InputEvent, InputFault, Key, Modifier, PointerButton, PressedState};
use windows::Win32::UI::WindowsAndMessaging::{
    WHEEL_DELTA, WM_CHAR, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN,
    WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP,
};

use crate::input::{
    DriverState, GeometryFingerprint, PointerState, SubmissionFailure, SystemButtonState,
    SystemKeyState,
};

const PHYSICAL_LEFT: u8 = 1;
const PHYSICAL_RIGHT: u8 = 2;
const PHYSICAL_MIDDLE: u8 = 3;
const MK_LBUTTON: u16 = 0x0001;
const MK_RBUTTON: u16 = 0x0002;
const MK_SHIFT: u16 = 0x0004;
const MK_CONTROL: u16 = 0x0008;
const MK_MBUTTON: u16 = 0x0010;

pub(crate) type SubmissionResult = Result<(), SubmissionFailure>;

/// One scalar Win32 message. Payload values never enter public diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MessageUnit {
    pub(crate) message: u32,
    pub(crate) wparam: usize,
    pub(crate) lparam: isize,
}

/// A key resolved against the target thread's current keyboard layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResolvedWindowKey {
    pub(crate) virtual_key: u16,
    pub(crate) scan_code: u8,
    pub(crate) extended: bool,
}

/// Native operations kept behind a deterministic, no-effect test seam.
pub(crate) trait WindowMessageSource {
    fn client_point(&self, screen: (i32, i32)) -> Result<(i16, i16), InputFault>;

    fn resolve_button(&self, button: PointerButton) -> Result<u8, InputFault>;

    fn resolve_key(&self, key: Key) -> Result<ResolvedWindowKey, InputFault>;

    fn post(
        &self,
        unit: MessageUnit,
        expected_geometry: Option<GeometryFingerprint>,
        operation: &OperationContext,
    ) -> SubmissionResult;
}

pub(crate) fn submit<S: WindowMessageSource + ?Sized>(
    source: &S,
    event: &InputEvent,
    pointer: Option<PointerState>,
    state: &mut DriverState,
    operation: &OperationContext,
) -> SubmissionResult {
    match event {
        InputEvent::PointerMove(_) => {
            let pointer = pointer.ok_or(InputFault::UnsupportedCoordinate)?;
            let local = source.client_point(pointer.screen)?;
            let unit = mouse_move(local, mouse_state(state));
            post_units(
                source,
                std::slice::from_ref(&unit),
                Some(pointer.geometry),
                operation,
            )?;
            state.pointer = Some(pointer);
            Ok(())
        }
        InputEvent::PointerPress(button) => {
            submit_button(source, *button, true, pointer, state, operation)
        }
        InputEvent::PointerRelease(button) => {
            submit_button(source, *button, false, pointer, state, operation)
        }
        InputEvent::PointerScroll {
            horizontal,
            vertical,
        } => submit_scroll(source, *horizontal, *vertical, pointer, state, operation),
        InputEvent::KeyPress(key) => submit_key(source, *key, true, state, operation),
        InputEvent::KeyRelease(key) => submit_key(source, *key, false, state, operation),
        InputEvent::Text(text) => submit_text(source, text, operation),
        InputEvent::Delay(_) => Err(InputFault::UnsupportedCombination.into()),
        _ => Err(InputFault::UnsupportedCombination.into()),
    }
}

pub(crate) fn release<S: WindowMessageSource + ?Sized>(
    source: &S,
    pressed: PressedState,
    state: &mut DriverState,
    operation: &OperationContext,
) -> Result<(), InputFault> {
    match pressed {
        PressedState::Button(button) => {
            let pointer = state.pointer.ok_or(InputFault::UnsupportedCoordinate)?;
            let index = state
                .buttons
                .iter()
                .rposition(|pressed| pressed.logical == button);
            let physical = index
                .map(|index| state.buttons[index].physical)
                .map_or_else(|| source.resolve_button(button), Ok)?;
            let local = source.client_point(pointer.screen)?;
            source
                .post(
                    mouse_move(local, mouse_state(state)),
                    Some(pointer.geometry),
                    operation,
                )
                .map_err(|failure| failure.fault)?;
            let unit = button_unit(
                physical,
                false,
                mouse_state_after_button(state, physical, false),
                local,
            )?;
            source
                .post(unit, Some(pointer.geometry), operation)
                .map_err(|failure| failure.fault)?;
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
            let resolved = index.map_or_else(
                || source.resolve_key(key),
                |index| Ok(resolved_from_state(state.keys[index])),
            )?;
            let unit = key_unit(resolved, false, index.is_some());
            source
                .post(unit, None, operation)
                .map_err(|failure| failure.fault)?;
            if let Some(index) = index {
                state.keys.remove(index);
            }
            Ok(())
        }
        _ => Err(InputFault::UnsupportedCombination),
    }
}

fn submit_button<S: WindowMessageSource + ?Sized>(
    source: &S,
    button: PointerButton,
    pressed: bool,
    pointer: Option<PointerState>,
    state: &mut DriverState,
    operation: &OperationContext,
) -> SubmissionResult {
    let pointer = pointer.ok_or(InputFault::UnsupportedCoordinate)?;
    let local = source.client_point(pointer.screen)?;
    let physical = state
        .buttons
        .iter()
        .rfind(|current| current.logical == button)
        .map(|current| current.physical)
        .map_or_else(|| source.resolve_button(button), Ok)?;
    let move_unit = mouse_move(local, mouse_state(state));
    if let Err(failure) = source.post(move_unit, Some(pointer.geometry), operation) {
        return Err(failure.without_pressed_state());
    }

    let transition = button_unit(
        physical,
        pressed,
        mouse_state_after_button(state, physical, pressed),
        local,
    )?;
    if let Err(failure) = source.post(transition, Some(pointer.geometry), operation) {
        let transition_may_have_effect = failure.current_event_may_have_effect;
        if pressed && transition_may_have_effect {
            state.buttons.push(SystemButtonState {
                logical: button,
                physical,
            });
        }
        let failure = if transition_may_have_effect {
            failure
        } else {
            SubmissionFailure::during_event(failure.fault)
        };
        return Err(if pressed && transition_may_have_effect {
            failure
        } else {
            failure.without_pressed_state()
        });
    }

    if pressed {
        state.buttons.push(SystemButtonState {
            logical: button,
            physical,
        });
    } else if let Some(index) = state
        .buttons
        .iter()
        .rposition(|current| current.logical == button)
    {
        state.buttons.remove(index);
    }
    Ok(())
}

fn submit_scroll<S: WindowMessageSource + ?Sized>(
    source: &S,
    horizontal: i16,
    vertical: i16,
    pointer: Option<PointerState>,
    state: &DriverState,
    operation: &OperationContext,
) -> SubmissionResult {
    let pointer = pointer.ok_or(InputFault::UnsupportedCoordinate)?;
    let screen = checked_point(pointer.screen)?;
    let mouse_state = mouse_state(state);
    let mut units = [MessageUnit {
        message: 0,
        wparam: 0,
        lparam: 0,
    }; 2];
    let mut len = 0usize;
    if vertical != 0 {
        let delta = wheel_delta(vertical, true)?;
        units[len] = wheel_unit(WM_MOUSEWHEEL, mouse_state, delta, screen);
        len += 1;
    }
    if horizontal != 0 {
        let delta = wheel_delta(horizontal, false)?;
        units[len] = wheel_unit(WM_MOUSEHWHEEL, mouse_state, delta, screen);
        len += 1;
    }
    post_units(source, &units[..len], Some(pointer.geometry), operation)
}

fn submit_key<S: WindowMessageSource + ?Sized>(
    source: &S,
    key: Key,
    pressed: bool,
    state: &mut DriverState,
    operation: &OperationContext,
) -> SubmissionResult {
    let index = state
        .keys
        .iter()
        .rposition(|current| current.logical == key);
    let resolved = index.map_or_else(
        || source.resolve_key(key),
        |index| Ok(resolved_from_state(state.keys[index])),
    )?;
    let unit = key_unit(resolved, pressed, index.is_some());
    if let Err(failure) = post_units(source, std::slice::from_ref(&unit), None, operation) {
        if pressed && failure.current_event_may_have_effect {
            state.keys.push(SystemKeyState {
                logical: key,
                virtual_key: resolved.virtual_key,
                scan_code: resolved.scan_code,
                extended: resolved.extended,
            });
        }
        return Err(if pressed {
            failure
        } else {
            failure.without_pressed_state()
        });
    }
    if pressed {
        state.keys.push(SystemKeyState {
            logical: key,
            virtual_key: resolved.virtual_key,
            scan_code: resolved.scan_code,
            extended: resolved.extended,
        });
    } else if let Some(index) = index {
        state.keys.remove(index);
    }
    Ok(())
}

fn submit_text<S: WindowMessageSource + ?Sized>(
    source: &S,
    text: &str,
    operation: &OperationContext,
) -> SubmissionResult {
    let mut accepted = false;
    for unit in text.encode_utf16() {
        post_one(
            source,
            MessageUnit {
                message: WM_CHAR,
                wparam: usize::from(unit),
                lparam: key_lparam(0, false, false, false),
            },
            None,
            operation,
            &mut accepted,
        )?;
    }
    Ok(())
}

fn post_units<S: WindowMessageSource + ?Sized>(
    source: &S,
    units: &[MessageUnit],
    expected_geometry: Option<GeometryFingerprint>,
    operation: &OperationContext,
) -> SubmissionResult {
    let mut accepted = false;
    for unit in units {
        post_one(source, *unit, expected_geometry, operation, &mut accepted)?;
    }
    Ok(())
}

fn post_one<S: WindowMessageSource + ?Sized>(
    source: &S,
    unit: MessageUnit,
    expected_geometry: Option<GeometryFingerprint>,
    operation: &OperationContext,
    accepted: &mut bool,
) -> SubmissionResult {
    match source.post(unit, expected_geometry, operation) {
        Ok(()) => {
            *accepted = true;
            Ok(())
        }
        Err(failure) if *accepted || failure.current_event_may_have_effect => {
            Err(SubmissionFailure::during_event(failure.fault))
        }
        Err(failure) => Err(failure),
    }
}

fn mouse_move(point: (i16, i16), state: u16) -> MessageUnit {
    MessageUnit {
        message: WM_MOUSEMOVE,
        wparam: usize::from(state),
        lparam: packed_lparam(point.0, point.1),
    }
}

fn button_unit(
    physical: u8,
    pressed: bool,
    state: u16,
    point: (i16, i16),
) -> Result<MessageUnit, InputFault> {
    let message = match (physical, pressed) {
        (PHYSICAL_LEFT, true) => WM_LBUTTONDOWN,
        (PHYSICAL_LEFT, false) => WM_LBUTTONUP,
        (PHYSICAL_RIGHT, true) => WM_RBUTTONDOWN,
        (PHYSICAL_RIGHT, false) => WM_RBUTTONUP,
        (PHYSICAL_MIDDLE, true) => WM_MBUTTONDOWN,
        (PHYSICAL_MIDDLE, false) => WM_MBUTTONUP,
        _ => return Err(InputFault::UnsupportedCombination),
    };
    Ok(MessageUnit {
        message,
        wparam: usize::from(state),
        lparam: packed_lparam(point.0, point.1),
    })
}

fn wheel_unit(message: u32, state: u16, delta: i16, point: (i16, i16)) -> MessageUnit {
    MessageUnit {
        message,
        wparam: packed_wparam(state, delta),
        lparam: packed_lparam(point.0, point.1),
    }
}

fn key_unit(resolved: ResolvedWindowKey, pressed: bool, previous: bool) -> MessageUnit {
    MessageUnit {
        message: if pressed { WM_KEYDOWN } else { WM_KEYUP },
        wparam: usize::from(resolved.virtual_key),
        lparam: key_lparam(
            resolved.scan_code,
            resolved.extended,
            previous || !pressed,
            !pressed,
        ),
    }
}

fn key_lparam(scan_code: u8, extended: bool, previous: bool, transition: bool) -> isize {
    let mut bits = 1u32 | (u32::from(scan_code) << 16);
    if extended {
        bits |= 1 << 24;
    }
    if previous {
        bits |= 1 << 30;
    }
    if transition {
        bits |= 1 << 31;
    }
    signed_lparam(bits)
}

fn mouse_state(state: &DriverState) -> u16 {
    let mut bits = 0u16;
    for button in &state.buttons {
        bits |= button_state(button.physical);
    }
    for key in &state.keys {
        match key.logical {
            Key::Modifier(Modifier::Shift) => bits |= MK_SHIFT,
            Key::Modifier(Modifier::Control) => bits |= MK_CONTROL,
            _ => {}
        }
    }
    bits
}

fn mouse_state_after_button(state: &DriverState, physical: u8, pressed: bool) -> u16 {
    let mut bits = mouse_state(state);
    let bit = button_state(physical);
    if pressed {
        bits |= bit;
    } else if state
        .buttons
        .iter()
        .filter(|current| current.physical == physical)
        .count()
        <= 1
    {
        bits &= !bit;
    }
    bits
}

fn button_state(physical: u8) -> u16 {
    match physical {
        PHYSICAL_LEFT => MK_LBUTTON,
        PHYSICAL_RIGHT => MK_RBUTTON,
        PHYSICAL_MIDDLE => MK_MBUTTON,
        _ => 0,
    }
}

fn resolved_from_state(state: SystemKeyState) -> ResolvedWindowKey {
    ResolvedWindowKey {
        virtual_key: state.virtual_key,
        scan_code: state.scan_code,
        extended: state.extended,
    }
}

fn wheel_delta(notches: i16, vertical: bool) -> Result<i16, InputFault> {
    let notches = i32::from(notches);
    let delta = if vertical {
        notches.checked_neg()
    } else {
        Some(notches)
    }
    .and_then(|value| value.checked_mul(i32::try_from(WHEEL_DELTA).ok()?))
    .ok_or(InputFault::UnsupportedCombination)?;
    i16::try_from(delta).map_err(|_| InputFault::UnsupportedCombination)
}

fn checked_point(point: (i32, i32)) -> Result<(i16, i16), InputFault> {
    Ok((
        i16::try_from(point.0).map_err(|_| InputFault::UnsupportedCoordinate)?,
        i16::try_from(point.1).map_err(|_| InputFault::UnsupportedCoordinate)?,
    ))
}

fn packed_wparam(low: u16, high: i16) -> usize {
    let bits = u32::from(low) | (u32::from(high.cast_unsigned()) << 16);
    usize::try_from(bits).expect("u32 fits the supported Windows pointer widths")
}

fn packed_lparam(low: i16, high: i16) -> isize {
    let bits = u32::from(low.cast_unsigned()) | (u32::from(high.cast_unsigned()) << 16);
    signed_lparam(bits)
}

fn signed_lparam(bits: u32) -> isize {
    isize::try_from(i32::from_ne_bytes(bits.to_ne_bytes()))
        .expect("i32 fits the supported Windows pointer widths")
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use mado_pilot_core::{
        CoordinateSpace, GeometryRevision, OperationContext, PixelExtent, Point, Scale,
        TargetPlacement,
    };
    use mado_pilot_input::{InputEvent, InputFault, Key, Modifier, PointerButton, PressedState};
    use windows::Win32::UI::WindowsAndMessaging::{
        WM_CHAR, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
        WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP,
    };

    use super::{
        MessageUnit, ResolvedWindowKey, WindowMessageSource, checked_point, release, submit,
    };
    use crate::input::{
        DriverState, GeometryFingerprint, PointerState, SubmissionFailure, SystemKeyState,
    };

    fn lparam_bits(value: isize) -> u32 {
        u32::from_ne_bytes(
            i32::try_from(value)
                .expect("message lParam remains a signed 32-bit payload")
                .to_ne_bytes(),
        )
    }

    fn high_word(value: usize) -> u16 {
        u16::try_from((value >> 16) & usize::from(u16::MAX)).expect("masked high word fits u16")
    }

    #[derive(Debug, Default)]
    struct FakeSource {
        posts: Mutex<Vec<MessageUnit>>,
        fail_at: Mutex<Option<(usize, SubmissionFailure)>>,
        client: Mutex<(i16, i16)>,
        button: Mutex<Option<u8>>,
        key: Mutex<Option<ResolvedWindowKey>>,
    }

    impl FakeSource {
        fn with_client(client: (i16, i16)) -> Self {
            Self {
                client: Mutex::new(client),
                ..Self::default()
            }
        }

        fn fail_at(&self, index: usize, failure: SubmissionFailure) {
            *self.fail_at.lock().expect("uncontended") = Some((index, failure));
        }

        fn clear_failure(&self) {
            *self.fail_at.lock().expect("uncontended") = None;
        }

        fn resolve_button_as(&self, physical: u8) {
            *self.button.lock().expect("uncontended") = Some(physical);
        }

        fn resolve_key_as(&self, resolved: ResolvedWindowKey) {
            *self.key.lock().expect("uncontended") = Some(resolved);
        }

        fn posts(&self) -> Vec<MessageUnit> {
            self.posts.lock().expect("uncontended").clone()
        }
    }

    impl WindowMessageSource for FakeSource {
        fn client_point(&self, _screen: (i32, i32)) -> Result<(i16, i16), InputFault> {
            Ok(*self.client.lock().expect("uncontended"))
        }

        fn resolve_button(&self, button: PointerButton) -> Result<u8, InputFault> {
            if let Some(physical) = *self.button.lock().expect("uncontended") {
                return Ok(physical);
            }
            match button {
                PointerButton::Primary => Ok(1),
                PointerButton::Secondary => Ok(2),
                PointerButton::Middle => Ok(3),
                _ => Err(InputFault::UnsupportedCombination),
            }
        }

        fn resolve_key(&self, key: Key) -> Result<ResolvedWindowKey, InputFault> {
            if let Some(resolved) = *self.key.lock().expect("uncontended") {
                return Ok(resolved);
            }
            Ok(match key {
                Key::Modifier(Modifier::Shift) => ResolvedWindowKey {
                    virtual_key: 0x10,
                    scan_code: 0x2a,
                    extended: false,
                },
                Key::Modifier(Modifier::Control) => ResolvedWindowKey {
                    virtual_key: 0x11,
                    scan_code: 0x1d,
                    extended: false,
                },
                Key::Delete => ResolvedWindowKey {
                    virtual_key: 0x2e,
                    scan_code: 0x53,
                    extended: true,
                },
                _ => ResolvedWindowKey {
                    virtual_key: 0x41,
                    scan_code: 0x1e,
                    extended: false,
                },
            })
        }

        fn post(
            &self,
            unit: MessageUnit,
            _expected_geometry: Option<GeometryFingerprint>,
            _operation: &OperationContext,
        ) -> Result<(), SubmissionFailure> {
            let mut posts = self.posts.lock().expect("uncontended");
            let index = posts.len();
            if let Some((fail_at, failure)) = *self.fail_at.lock().expect("uncontended")
                && index == fail_at
            {
                return Err(failure);
            }
            posts.push(unit);
            Ok(())
        }
    }

    fn pointer() -> PointerState {
        PointerState {
            screen: (120, -30),
            geometry: GeometryFingerprint {
                extent: PixelExtent::new(640, 480),
                placement: TargetPlacement::new(
                    (0.0, 0.0),
                    (640.0, 480.0),
                    Scale::new(1.0, 1.0).expect("scale"),
                )
                .expect("placement"),
            },
        }
    }

    fn operation() -> OperationContext {
        OperationContext::new()
    }

    #[test]
    fn pointer_move_uses_checked_client_coordinates() {
        let source = FakeSource::with_client((-7, 11));
        let mut state = DriverState::default();
        submit(
            &source,
            &InputEvent::PointerMove(
                Point::new(CoordinateSpace::CapturePixels, 1.0, 2.0).expect("point"),
            ),
            Some(pointer()),
            &mut state,
            &operation(),
        )
        .expect("submitted");

        assert_eq!(source.posts()[0].message, WM_MOUSEMOVE);
        assert_eq!(lparam_bits(source.posts()[0].lparam), 0x000b_fff9);
        assert_eq!(state.pointer, Some(pointer()));
    }

    #[test]
    fn every_button_uses_move_before_down_and_up() {
        let source = FakeSource::with_client((5, 6));
        let mut state = DriverState {
            pointer: Some(pointer()),
            ..DriverState::default()
        };
        for (button, down, up) in [
            (PointerButton::Primary, WM_LBUTTONDOWN, WM_LBUTTONUP),
            (PointerButton::Secondary, WM_RBUTTONDOWN, WM_RBUTTONUP),
            (PointerButton::Middle, WM_MBUTTONDOWN, WM_MBUTTONUP),
        ] {
            submit(
                &source,
                &InputEvent::PointerPress(button),
                state.pointer,
                &mut state,
                &operation(),
            )
            .expect("down");
            submit(
                &source,
                &InputEvent::PointerRelease(button),
                state.pointer,
                &mut state,
                &operation(),
            )
            .expect("up");
            let posts = source.posts();
            assert_eq!(posts[posts.len() - 4].message, WM_MOUSEMOVE);
            assert_eq!(posts[posts.len() - 3].message, down);
            assert_eq!(posts[posts.len() - 2].message, WM_MOUSEMOVE);
            assert_eq!(posts[posts.len() - 1].message, up);
        }
    }

    #[test]
    fn accepted_move_then_refused_button_is_partial_native_effect() {
        let source = FakeSource::with_client((5, 6));
        source.fail_at(
            1,
            SubmissionFailure::before_event(InputFault::SubmissionFailed),
        );
        let mut state = DriverState {
            pointer: Some(pointer()),
            ..DriverState::default()
        };
        let failure = submit(
            &source,
            &InputEvent::PointerPress(PointerButton::Primary),
            state.pointer,
            &mut state,
            &operation(),
        )
        .expect_err("second unit refused");

        assert!(failure.current_event_may_have_effect);
        assert!(!failure.current_event_may_leave_pressed_state);
        assert_eq!(source.posts().len(), 1);
        assert!(state.buttons.is_empty());
    }

    #[test]
    fn indeterminate_button_press_retains_its_original_physical_mapping() {
        let source = FakeSource::with_client((5, 6));
        source.resolve_button_as(1);
        source.fail_at(1, SubmissionFailure::during_event(InputFault::TargetLost));
        let mut state = DriverState {
            pointer: Some(pointer()),
            ..DriverState::default()
        };
        let failure = submit(
            &source,
            &InputEvent::PointerPress(PointerButton::Primary),
            state.pointer,
            &mut state,
            &operation(),
        )
        .expect_err("button post has indeterminate effect");

        assert!(failure.current_event_may_leave_pressed_state);
        assert_eq!(state.buttons[0].physical, 1);
        source.clear_failure();
        source.resolve_button_as(2);
        release(
            &source,
            PressedState::Button(PointerButton::Primary),
            &mut state,
            &operation(),
        )
        .expect("cleanup uses retained mapping");
        assert_eq!(source.posts()[1].message, WM_MOUSEMOVE);
        assert_eq!(source.posts()[2].message, WM_LBUTTONUP);
        assert!(state.buttons.is_empty());
    }

    #[test]
    fn wheel_uses_screen_coordinates_and_axis_signs() {
        let source = FakeSource::default();
        let mut state = DriverState {
            pointer: Some(pointer()),
            ..DriverState::default()
        };
        submit(
            &source,
            &InputEvent::PointerScroll {
                horizontal: 2,
                vertical: 3,
            },
            state.pointer,
            &mut state,
            &operation(),
        )
        .expect("submitted");

        let posts = source.posts();
        assert_eq!(posts[0].message, WM_MOUSEWHEEL);
        assert_eq!(high_word(posts[0].wparam), (-360i16).cast_unsigned());
        assert_eq!(posts[1].message, WM_MOUSEHWHEEL);
        assert_eq!(high_word(posts[1].wparam), 240u16);
        assert_eq!(lparam_bits(posts[0].lparam), 0xffe2_0078);
    }

    #[test]
    fn unrepresentable_wheel_coordinate_is_refused_before_post() {
        assert_eq!(
            checked_point((i32::from(i16::MAX) + 1, 0)),
            Err(InputFault::UnsupportedCoordinate)
        );
        let source = FakeSource::default();
        let mut state = DriverState {
            pointer: Some(PointerState {
                screen: (i32::from(i16::MAX) + 1, 0),
                ..pointer()
            }),
            ..DriverState::default()
        };
        let pointer = state.pointer;
        assert_eq!(
            submit(
                &source,
                &InputEvent::PointerScroll {
                    horizontal: 0,
                    vertical: 1,
                },
                pointer,
                &mut state,
                &operation(),
            ),
            Err(SubmissionFailure::before_event(
                InputFault::UnsupportedCoordinate
            ))
        );
        assert!(source.posts().is_empty());
    }

    #[test]
    fn key_fields_follow_sequence_state() {
        let source = FakeSource::default();
        let mut state = DriverState::default();
        submit(
            &source,
            &InputEvent::KeyPress(Key::Delete),
            None,
            &mut state,
            &operation(),
        )
        .expect("down");
        submit(
            &source,
            &InputEvent::KeyRelease(Key::Delete),
            None,
            &mut state,
            &operation(),
        )
        .expect("up");

        let posts = source.posts();
        assert_eq!(posts[0].message, WM_KEYDOWN);
        let down = lparam_bits(posts[0].lparam);
        assert_eq!((down >> 16) & 0xff, 0x53);
        assert_eq!((down >> 24) & 1, 1);
        assert_eq!((down >> 30) & 1, 0);
        assert_eq!(posts[1].message, WM_KEYUP);
        let up = lparam_bits(posts[1].lparam);
        assert_eq!((up >> 30) & 1, 1);
        assert_eq!((up >> 31) & 1, 1);
    }

    #[test]
    fn standalone_key_release_sets_previous_and_transition_bits() {
        let source = FakeSource::default();
        let mut state = DriverState::default();
        submit(
            &source,
            &InputEvent::KeyRelease(Key::Delete),
            None,
            &mut state,
            &operation(),
        )
        .expect("standalone key release");

        let up = lparam_bits(source.posts()[0].lparam);
        assert_eq!((up >> 30) & 1, 1);
        assert_eq!((up >> 31) & 1, 1);
    }

    #[test]
    fn indeterminate_key_press_retains_its_original_layout_mapping() {
        let source = FakeSource::default();
        source.resolve_key_as(ResolvedWindowKey {
            virtual_key: 0x2e,
            scan_code: 0x53,
            extended: true,
        });
        source.fail_at(0, SubmissionFailure::during_event(InputFault::TargetLost));
        let mut state = DriverState::default();
        submit(
            &source,
            &InputEvent::KeyPress(Key::Delete),
            None,
            &mut state,
            &operation(),
        )
        .expect_err("key post has indeterminate effect");

        assert_eq!(state.keys[0].scan_code, 0x53);
        source.clear_failure();
        source.resolve_key_as(ResolvedWindowKey {
            virtual_key: 0x41,
            scan_code: 0x1e,
            extended: false,
        });
        release(
            &source,
            PressedState::Key(Key::Delete),
            &mut state,
            &operation(),
        )
        .expect("cleanup uses retained mapping");

        let up = source.posts()[0];
        assert_eq!(up.wparam, 0x2e);
        assert_eq!((lparam_bits(up.lparam) >> 16) & 0xff, 0x53);
        assert_eq!((lparam_bits(up.lparam) >> 24) & 1, 1);
        assert_eq!((lparam_bits(up.lparam) >> 30) & 1, 1);
        assert!(state.keys.is_empty());
    }

    #[test]
    fn modifier_state_is_carried_by_pointer_messages() {
        let source = FakeSource::with_client((5, 6));
        let mut state = DriverState {
            pointer: Some(pointer()),
            keys: vec![
                SystemKeyState {
                    logical: Key::Modifier(Modifier::Shift),
                    virtual_key: 0x10,
                    scan_code: 0x2a,
                    extended: false,
                },
                SystemKeyState {
                    logical: Key::Modifier(Modifier::Control),
                    virtual_key: 0x11,
                    scan_code: 0x1d,
                    extended: false,
                },
            ],
            ..DriverState::default()
        };
        submit(
            &source,
            &InputEvent::PointerPress(PointerButton::Primary),
            state.pointer,
            &mut state,
            &operation(),
        )
        .expect("submitted");
        let posts = source.posts();
        assert_eq!(posts[0].wparam & 0x000c, 0x000c);
        assert_eq!(posts[1].wparam & 0x000d, 0x000d);
    }

    #[test]
    fn surrogate_pair_is_streamed_and_partial_failure_is_visible() {
        let source = FakeSource::default();
        source.fail_at(
            1,
            SubmissionFailure::before_event(InputFault::SubmissionFailed),
        );
        let mut state = DriverState::default();
        let failure = submit(
            &source,
            &InputEvent::Text("😀".to_owned()),
            None,
            &mut state,
            &operation(),
        )
        .expect_err("second UTF-16 unit refused");

        assert!(failure.current_event_may_have_effect);
        let posts = source.posts();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].message, WM_CHAR);
        assert_eq!(posts[0].wparam, usize::from(0xd83d_u16));
    }

    #[test]
    fn button_requires_sequence_pointer_and_posts_nothing() {
        let source = FakeSource::default();
        let mut state = DriverState::default();
        assert_eq!(
            submit(
                &source,
                &InputEvent::PointerPress(PointerButton::Primary),
                None,
                &mut state,
                &operation(),
            ),
            Err(SubmissionFailure::before_event(
                InputFault::UnsupportedCoordinate
            ))
        );
        assert!(source.posts().is_empty());
    }

    #[test]
    fn cleanup_posts_only_release_and_removes_owned_state() {
        let source = FakeSource::with_client((5, 6));
        let mut state = DriverState {
            pointer: Some(pointer()),
            keys: vec![SystemKeyState {
                logical: Key::Delete,
                virtual_key: 0x2e,
                scan_code: 0x53,
                extended: true,
            }],
            ..DriverState::default()
        };
        release(
            &source,
            PressedState::Key(Key::Delete),
            &mut state,
            &operation(),
        )
        .expect("released");
        assert_eq!(source.posts().as_slice()[0].message, WM_KEYUP);
        assert!(state.keys.is_empty());
    }

    #[test]
    fn geometry_revision_is_not_packed_into_native_payload() {
        let pointer = PointerState {
            geometry: GeometryFingerprint {
                extent: PixelExtent::new(640, 480),
                placement: TargetPlacement::new(
                    (0.0, 0.0),
                    (640.0, 480.0),
                    Scale::new(2.0, 1.0).expect("scale"),
                )
                .expect("placement"),
            },
            ..pointer()
        };
        let _revision = GeometryRevision::FIRST;
        let source = FakeSource::with_client((1, 2));
        let mut state = DriverState::default();
        submit(
            &source,
            &InputEvent::PointerMove(
                Point::new(CoordinateSpace::CapturePixels, 1.0, 2.0).expect("point"),
            ),
            Some(pointer),
            &mut state,
            &operation(),
        )
        .expect("submitted");
        assert_eq!(lparam_bits(source.posts()[0].lparam), 0x0002_0001);
    }
}
