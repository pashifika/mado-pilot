#![cfg_attr(not(windows), allow(missing_docs))]
#![cfg(windows)]
//! Native background-input coverage against the dedicated fixture process.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use mado_pilot_capture::{CaptureProvider, DiscoveryRequest};
use mado_pilot_core::{
    CoordinateSpace, IdentityIssuer, InputDelivery, InputOperationKind, OperationContext, Point,
    Status, TargetId, TargetKind,
};
use mado_pilot_input::{
    DeliveryPlan, FocusPolicy, InputEvent, InputOpenRequest, InputProvider, InputRequest,
    InputSequence, Key, PointerButton, SequenceOutcome,
};
use mado_pilot_platform_windows::WindowsCaptureProvider;
use mado_pilot_platform_windows::fixture_protocol::{
    CLASS_NAME, MAX_RECORDED_EVENTS, fixture_title, select_unique_fixture,
};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetCursorPos, GetForegroundWindow, SetCursorPos, SetForegroundWindow,
};
use windows::core::PCWSTR;

#[derive(Debug)]
struct FixtureProcess {
    child: Child,
}

impl FixtureProcess {
    fn spawn() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_mado-pilot-windows-input-fixture"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawned dedicated input fixture");
        let process_id = child.id();
        let output = child.stdout.take().expect("captured fixture readiness");
        let mut ready = String::new();
        BufReader::new(output)
            .read_line(&mut ready)
            .expect("read fixture readiness");
        assert!(ready.starts_with("fixture-ready "));
        assert!(ready.contains(&format!("class={CLASS_NAME}")));
        assert!(ready.contains(&format!("title={}", fixture_title(process_id))));
        assert!(ready.contains(&format!("capacity={MAX_RECORDED_EVENTS}")));
        Self { child }
    }

    fn process_id(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for FixtureProcess {
    fn drop(&mut self) {
        let _killed = self.child.kill();
        let _waited = self.child.wait();
    }
}

#[derive(Debug)]
struct DesktopState {
    cursor: POINT,
    foreground: HWND,
}

impl DesktopState {
    fn capture() -> Self {
        let mut cursor = POINT::default();
        // SAFETY: cursor is a complete writable POINT.
        unsafe { GetCursorPos(&raw mut cursor) }.expect("read cursor position");
        // SAFETY: GetForegroundWindow returns an opaque handle.
        let foreground = unsafe { GetForegroundWindow() };
        Self { cursor, foreground }
    }
}

impl Drop for DesktopState {
    fn drop(&mut self) {
        // SAFETY: the saved coordinates came from GetCursorPos.
        let _restored = unsafe { SetCursorPos(self.cursor.x, self.cursor.y) };
        if self.foreground != HWND::default() {
            // SAFETY: the handle was foreground immediately before this short
            // interactive test. Windows may still decline the restoration.
            let _focused = unsafe { SetForegroundWindow(self.foreground) };
        }
    }
}

#[test]
fn dedicated_fixture_acknowledges_background_pointer_keyboard_and_text() {
    let fixture = FixtureProcess::spawn();
    let provider = WindowsCaptureProvider::new(Arc::new(IdentityIssuer::new()));
    let title = fixture_title(fixture.process_id());
    let discovery = DiscoveryRequest::new()
        .with_kind(TargetKind::Window)
        .with_name_containing(&title);

    let mut targets = Vec::new();
    for _attempt in 0..50 {
        match provider.discover_matching(&discovery, &timed()) {
            Ok(found) if !found.is_empty() => {
                targets = found;
                break;
            }
            Ok(_) => thread::sleep(Duration::from_millis(10)),
            Err(error) if error.status() == Status::Unsupported => {
                eprintln!("skipped native input fixture: Windows discovery is unavailable");
                return;
            }
            Err(error) => panic!("native fixture discovery failed: {error}"),
        }
    }
    let target = select_unique_fixture(&targets, fixture.process_id())
        .expect("exactly one approved fixture target");
    let open = InputOpenRequest::new()
        .requiring(InputOperationKind::Pointer, InputDelivery::BackgroundTarget)
        .requiring(
            InputOperationKind::Keyboard,
            InputDelivery::BackgroundTarget,
        )
        .requiring(InputOperationKind::Text, InputDelivery::BackgroundTarget);
    let controller = InputProvider::open(&provider, target.id(), &open, &timed())
        .expect("opened background fixture controller");
    let point =
        Point::new(CoordinateSpace::TargetNormalized, 0.5, 0.5).expect("normalized fixture point");
    let sequence = InputSequence::new(vec![
        InputEvent::PointerMove(point),
        InputEvent::PointerPress(PointerButton::Primary),
        InputEvent::PointerRelease(PointerButton::Primary),
        InputEvent::KeyPress(Key::Enter),
        InputEvent::KeyRelease(Key::Enter),
        InputEvent::Text("fixture-probe".to_owned()),
    ])
    .expect("bounded fixture sequence");
    let request = InputRequest::new(
        target.id(),
        sequence,
        DeliveryPlan::require(InputDelivery::BackgroundTarget),
    );

    let receipt = controller
        .execute(&request, &timed())
        .expect("background sequence is receipted");

    assert_eq!(receipt.outcome(), SequenceOutcome::Complete, "{receipt:?}");
    assert_eq!(receipt.delivery(), Some(InputDelivery::BackgroundTarget));
    assert_eq!(receipt.delivered(), 6);
    assert_eq!(receipt.attempted(), [InputDelivery::BackgroundTarget]);
    controller.close(&timed()).expect("closed controller");
}

#[test]
#[ignore = "waits for the user to focus the dedicated fixture, then injects system input"]
fn interactive_system_delivery_targets_only_the_exact_fixture() {
    let _desktop = DesktopState::capture();
    let fixture = FixtureProcess::spawn();
    let provider = WindowsCaptureProvider::new(Arc::new(IdentityIssuer::new()));
    let title = fixture_title(fixture.process_id());
    let discovery = DiscoveryRequest::new()
        .with_kind(TargetKind::Window)
        .with_name_containing(&title);

    let mut targets = Vec::new();
    for _attempt in 0..50 {
        match provider.discover_matching(&discovery, &timed()) {
            Ok(found) if !found.is_empty() => {
                targets = found;
                break;
            }
            Ok(_) => thread::sleep(Duration::from_millis(10)),
            Err(error) => panic!("interactive fixture discovery failed: {error}"),
        }
    }
    let target = select_unique_fixture(&targets, fixture.process_id())
        .expect("exactly one approved fixture target");
    let target_id: TargetId = target.id();
    let open = InputOpenRequest::new()
        .requiring(InputOperationKind::Pointer, InputDelivery::System)
        .requiring(InputOperationKind::Keyboard, InputDelivery::System)
        .requiring(InputOperationKind::Text, InputDelivery::System);
    let controller = InputProvider::open(&provider, target_id, &open, &timed())
        .expect("opened system fixture controller");
    wait_for_user_focus(fixture.process_id());
    let point =
        Point::new(CoordinateSpace::TargetNormalized, 0.5, 0.5).expect("normalized fixture point");
    let sequence = InputSequence::new(vec![
        InputEvent::PointerMove(point),
        InputEvent::KeyPress(Key::Enter),
        InputEvent::KeyRelease(Key::Enter),
        InputEvent::Text("system-probe".to_owned()),
    ])
    .expect("bounded system sequence");
    let request = InputRequest::new(
        target_id,
        sequence,
        DeliveryPlan::require(InputDelivery::System),
    )
    .with_focus(FocusPolicy::RequireFocused);

    let receipt = controller
        .execute(&request, &timed())
        .expect("system sequence is receipted");

    assert_eq!(receipt.outcome(), SequenceOutcome::Complete, "{receipt:?}");
    assert_eq!(receipt.delivery(), Some(InputDelivery::System));
    assert_eq!(receipt.delivered(), 4);
    assert_eq!(receipt.attempted(), [InputDelivery::System]);
    controller.close(&timed()).expect("closed controller");
}

fn wait_for_user_focus(process_id: u32) {
    let class = wide(CLASS_NAME);
    let title_text = fixture_title(process_id);
    let title = wide(&title_text);
    // SAFETY: both strings are terminated and identify the dedicated fixture.
    let hwnd = unsafe { FindWindowW(PCWSTR(class.as_ptr()), PCWSTR(title.as_ptr())) }
        .expect("found exact fixture window");
    eprintln!("focus `{title_text}` within 15 seconds to authorize system delivery");
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        // SAFETY: GetForegroundWindow returns an opaque handle.
        if unsafe { GetForegroundWindow() } == hwnd {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("fixture was not focused; no system input was sent");
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn timed() -> OperationContext {
    OperationContext::new()
        .with_timeout(Duration::from_secs(5))
        .expect("representable fixture timeout")
}
