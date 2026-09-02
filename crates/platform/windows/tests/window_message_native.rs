#![cfg_attr(not(windows), allow(missing_docs))]
#![cfg(windows)]
//! Native ordinary-window delivery, authority, and isolation coverage.

use std::collections::VecDeque;
use std::mem::size_of;
use std::num::NonZeroU32;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use mado_pilot_capture::{
    CaptureProvider, DiscoveryRequest, FrameRequest, OpenRequest, PixelFormat,
};
use mado_pilot_core::{
    CancellationToken, CapabilitySupport, CoordinateSpace, FrameStamp, IdentityIssuer,
    InputAddressScope, InputDelivery, InputOperationKind, OperationContext, Point, Status,
    SubmissionEvidence, TargetId, TargetKind,
};
use mado_pilot_input::{
    CleanupBudget, CleanupState, DeliveryPlan, FocusPolicy, InputController, InputEvent,
    InputFault, InputOpenRequest, InputProvider, InputRequest, InputSequence, Key, Modifier,
    PointerButton, PointerGeometry, SequenceOutcome,
};
use mado_pilot_platform_windows::WindowsCaptureProvider;
use mado_pilot_platform_windows::fixture_protocol::{
    CONTROL_ALLOW_FOREGROUND, CONTROL_BLOCK_QUEUE, CONTROL_DESTROY_TARGET,
    CONTROL_DUPLICATE_METADATA, CONTROL_REPARENT_TARGET, CONTROL_REPORT, CONTROL_REUSE_STRESS,
    CONTROL_SET_GEOMETRY, CONTROL_SET_VISUAL_ABSENT, CONTROL_SET_VISUAL_VISIBLE,
    FixtureVisualCommand, FixtureVisualState, ORDINARY_CLASS_NAME, ordinary_fixture_title,
};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI,
    SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetCursorPos, GetForegroundWindow, GetWindowRect, PostMessageW, SetCursorPos,
    SetForegroundWindow, WM_MOUSEMOVE, WM_NULL,
};
use windows::core::{BOOL, PCWSTR};

static NATIVE_MATRIX: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ObservationReport {
    target: u32,
    replacement: u32,
    game: u32,
    sibling: u32,
    child: u32,
    foreground: u32,
    raw_legacy: u32,
    state_legacy: u32,
    raw: u32,
    state: u32,
}

#[derive(Debug)]
struct FixtureProcess {
    child: Option<Child>,
    lines: Receiver<String>,
    reader: Option<thread::JoinHandle<()>>,
    token: String,
    pending: VecDeque<String>,
}

impl FixtureProcess {
    fn spawn(token: impl Into<String>) -> Self {
        Self::spawn_with_activation(token.into(), false)
    }

    fn spawn_activated(token: impl Into<String>) -> Self {
        Self::spawn_with_activation(token.into(), true)
    }

    fn spawn_with_activation(token: String, activate: bool) -> Self {
        let mut command = Command::new(env!(
            "CARGO_BIN_EXE_mado-pilot-windows-window-message-fixture"
        ));
        command.arg(format!("--title-token={token}"));
        if activate {
            command.arg("--activate");
        }
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawned ordinary WindowMessage fixture");
        let output = child.stdout.take().expect("captured fixture output");
        let (sender, lines) = mpsc::channel();
        let reader = thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else {
                    break;
                };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let mut fixture = Self {
            child: Some(child),
            lines,
            reader: Some(reader),
            token,
            pending: VecDeque::new(),
        };
        let ready = fixture.wait_for("fixture-ready ", Duration::from_secs(5));
        assert!(ready.contains(&format!("class={ORDINARY_CLASS_NAME}")));
        assert!(ready.contains(&format!("title={}", fixture.title())));
        fixture
    }

    fn title(&self) -> String {
        ordinary_fixture_title(&self.token)
    }
    fn role_title(&self, role: &str) -> String {
        format!("MadoPilot Ordinary WindowMessage {role} [{}]", self.token)
    }

    fn target(&self) -> HWND {
        find_window(&self.title())
    }
    fn role_window(&self, role: &str) -> HWND {
        find_window(&self.role_title(role))
    }

    fn control(&self, hwnd: HWND, message: u32, value: usize) {
        // SAFETY: hwnd belongs to this retained child fixture; every control
        // message is scalar and reserved by the fixture protocol.
        unsafe { PostMessageW(Some(hwnd), message, WPARAM(value), LPARAM(0)) }
            .expect("posted fixture control");
    }
    fn set_geometry(&self, hwnd: HWND, x: i32, y: i32, width: i32, height: i32) {
        let position = u64::from(x.cast_unsigned()) | (u64::from(y.cast_unsigned()) << 32);
        let size = u64::from(width.cast_unsigned()) | (u64::from(height.cast_unsigned()) << 32);
        let position = usize::try_from(position).expect("native geometry control requires 64-bit");
        let size = isize::try_from(size).expect("positive fixture size fits LPARAM");
        // SAFETY: hwnd belongs to this retained fixture and both values are packed scalars.
        unsafe {
            PostMessageW(
                Some(hwnd),
                CONTROL_SET_GEOMETRY,
                WPARAM(position),
                LPARAM(size),
            )
        }
        .expect("posted fixture geometry control");
    }

    fn wait_for(&mut self, prefix: &str, timeout: Duration) -> String {
        if let Some(index) = self
            .pending
            .iter()
            .position(|line| line.starts_with(prefix))
        {
            return self.pending.remove(index).expect("indexed pending line");
        }
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let line = self
                .lines
                .recv_timeout(remaining)
                .unwrap_or_else(|error| panic!("fixture did not emit `{prefix}`: {error}"));
            if line.starts_with(prefix) {
                return line;
            }
            self.pending.push_back(line);
        }
    }

    fn wait_for_observation(&mut self, prefix: &str, timeout: Duration) -> String {
        assert!(prefix.starts_with("observation "));
        if let Some(index) = self
            .pending
            .iter()
            .position(|line| line.starts_with("observation "))
        {
            let line = self.pending.remove(index).expect("indexed observation");
            assert!(
                line.starts_with(prefix),
                "fixture emitted unexpected observation `{line}` while waiting for `{prefix}`"
            );
            return line;
        }
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let line = self
                .lines
                .recv_timeout(remaining)
                .unwrap_or_else(|error| panic!("fixture did not emit `{prefix}`: {error}"));
            if line.starts_with("observation ") {
                assert!(
                    line.starts_with(prefix),
                    "fixture emitted unexpected observation `{line}` while waiting for `{prefix}`"
                );
                return line;
            }
            self.pending.push_back(line);
        }
    }
    fn assert_no_line(&mut self, prefix: &str, timeout: Duration) {
        assert!(
            self.pending.iter().all(|line| !line.starts_with(prefix)),
            "fixture emitted `{prefix}` before the bounded observation"
        );
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(remaining) {
                Ok(line) if line.starts_with(prefix) => {
                    panic!("fixture unexpectedly emitted `{line}`")
                }
                Ok(line) => self.pending.push_back(line),
                Err(RecvTimeoutError::Timeout) => return,
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("fixture exited while excluding `{prefix}`")
                }
            }
        }
    }

    fn report(&mut self, hwnd: HWND) -> ObservationReport {
        self.control(hwnd, CONTROL_REPORT, 0);
        parse_report(&self.wait_for("report ", Duration::from_secs(5)))
    }

    fn terminate(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _killed = child.kill();
            let _waited = child.wait();
        }
        if let Some(reader) = self.reader.take() {
            reader.join().expect("fixture output reader joined");
        }
    }
}

impl Drop for FixtureProcess {
    fn drop(&mut self) {
        self.terminate();
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
        unsafe { GetCursorPos(&raw mut cursor) }.expect("read physical cursor");
        // SAFETY: the return is an opaque snapshot and is not dereferenced.
        let foreground = unsafe { GetForegroundWindow() };
        Self { cursor, foreground }
    }

    fn assert_unchanged(&self) {
        let mut current = POINT::default();
        // SAFETY: current is a complete writable POINT.
        unsafe { GetCursorPos(&raw mut current) }.expect("read physical cursor");
        // SAFETY: the return is an opaque snapshot and is not dereferenced.
        let foreground = unsafe { GetForegroundWindow() };
        assert_eq!(
            current, self.cursor,
            "WindowMessage moved the physical cursor"
        );
        assert_eq!(
            foreground, self.foreground,
            "WindowMessage changed the unrelated foreground window"
        );
    }
}

#[derive(Debug)]
struct OwnedForeground {
    fixture: FixtureProcess,
    original: DesktopState,
    cursor: POINT,
    stop: Arc<AtomicBool>,
    violation: Arc<AtomicUsize>,
    active: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    monitor: Option<thread::JoinHandle<()>>,
}

impl OwnedForeground {
    fn establish() -> Self {
        let original = DesktopState::capture();
        let mut fixture = FixtureProcess::spawn_activated("owned-unrelated-foreground");
        let foreground = fixture.target();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            // SAFETY: foreground is a live, repository-owned top-level fixture window.
            let _requested = unsafe { SetForegroundWindow(foreground) };
            // SAFETY: the return is an opaque handle and is not dereferenced.
            if unsafe { GetForegroundWindow() } == foreground {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the owned unrelated fixture could not become foreground"
            );
            thread::sleep(Duration::from_millis(10));
        }
        fixture.control(
            foreground,
            CONTROL_ALLOW_FOREGROUND,
            usize::try_from(std::process::id()).expect("process identifier fits usize"),
        );
        fixture.wait_for("control foreground-delegate=ready", Duration::from_secs(5));

        let cursor = original.cursor;
        let stop = Arc::new(AtomicBool::new(false));
        let active = Arc::new(AtomicBool::new(false));
        let ready = Arc::new(AtomicBool::new(false));
        let violation = Arc::new(AtomicUsize::new(0));
        let monitor_stop = Arc::clone(&stop);
        let monitor_active = Arc::clone(&active);
        let monitor_ready = Arc::clone(&ready);
        let monitor_violation = Arc::clone(&violation);
        let expected_foreground = foreground.0.addr();
        let monitor = thread::spawn(move || {
            while !monitor_stop.load(Ordering::Acquire) {
                if !monitor_active.load(Ordering::Acquire) {
                    monitor_ready.store(false, Ordering::Release);
                    thread::sleep(Duration::from_millis(1));
                    continue;
                }
                // SAFETY: the return is an opaque handle and is not dereferenced.
                if unsafe { GetForegroundWindow() }.0.addr() != expected_foreground {
                    monitor_violation.store(1, Ordering::Release);
                    return;
                }
                let mut current = POINT::default();
                // SAFETY: current is a complete writable POINT.
                if unsafe { GetCursorPos(&raw mut current) }.is_err() {
                    monitor_violation.store(3, Ordering::Release);
                    return;
                }
                if current != cursor {
                    monitor_violation.store(2, Ordering::Release);
                    return;
                }
                monitor_ready.store(true, Ordering::Release);
                thread::sleep(Duration::from_millis(1));
            }
        });
        let guard = Self {
            fixture,
            original,
            cursor,
            stop,
            violation,
            active,
            ready,
            monitor: Some(monitor),
        };
        guard.assert_stable();
        guard
    }

    fn fixture_mut(&mut self) -> &mut FixtureProcess {
        &mut self.fixture
    }

    fn observe<T>(&self, operation: impl FnOnce() -> T) -> T {
        self.begin_observation();
        let result = operation();
        self.end_observation();
        result
    }

    fn begin_observation(&self) {
        self.active.store(false, Ordering::Release);
        self.ready.store(false, Ordering::Release);
        self.violation.store(0, Ordering::Release);
        let foreground = self.fixture.target();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            // SAFETY: foreground is a live, repository-owned top-level fixture window.
            let _requested = unsafe { SetForegroundWindow(foreground) };
            // SAFETY: the return is an opaque handle and is not dereferenced.
            if unsafe { GetForegroundWindow() } == foreground {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the owned unrelated fixture could not become foreground"
            );
            thread::sleep(Duration::from_millis(10));
        }
        self.assert_stable();
        self.active.store(true, Ordering::Release);
        while !self.ready.load(Ordering::Acquire) {
            assert_eq!(
                self.violation.load(Ordering::Acquire),
                0,
                "owned foreground monitor failed before adapter activity"
            );
            assert!(
                Instant::now() < deadline,
                "owned foreground monitor did not become ready"
            );
            thread::yield_now();
        }
    }

    fn end_observation(&self) {
        self.active.store(false, Ordering::Release);
        thread::sleep(Duration::from_millis(2));
        self.assert_stable();
    }

    fn assert_stable(&self) {
        assert_eq!(
            self.violation.load(Ordering::Acquire),
            0,
            "adapter activity changed owned foreground/cursor authority"
        );
        let current = DesktopState::capture();
        assert_eq!(
            current.foreground,
            self.fixture.target(),
            "owned unrelated fixture remains foreground"
        );
        assert_eq!(
            current.cursor, self.cursor,
            "physical cursor remains at the controlled position"
        );
    }

    fn stop_monitor(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(monitor) = self.monitor.take() {
            monitor.join().expect("foreground monitor joined");
        }
    }

    fn finish(mut self) {
        self.assert_stable();
        self.stop_monitor();
        assert_eq!(
            self.violation.load(Ordering::Acquire),
            0,
            "owned foreground monitor observed no transient mutation"
        );
    }
}

impl Drop for OwnedForeground {
    fn drop(&mut self) {
        self.stop_monitor();
        // SAFETY: the saved coordinates came from GetCursorPos.
        let _restored = unsafe { SetCursorPos(self.original.cursor.x, self.original.cursor.y) };
        if self.original.foreground != HWND::default() {
            // SAFETY: this is the opaque foreground handle captured before the test.
            let _restored = unsafe { SetForegroundWindow(self.original.foreground) };
        }
    }
}
#[test]
#[ignore = "opens real fixture windows; run deliberately on an unlocked desktop"]
fn watcher_visual_controls_acknowledge_before_fixture_cleanup() {
    let _serial = NATIVE_MATRIX.lock().expect("native matrix serialized");
    let mut fixture = FixtureProcess::spawn("watcher-visual-control");
    let target = fixture.target();

    let visible = FixtureVisualCommand::new(
        FixtureVisualState::Visible,
        NonZeroU32::new(1).expect("nonzero visual token"),
    );
    fixture.control(
        target,
        CONTROL_SET_VISUAL_VISIBLE,
        usize::try_from(visible.token().get()).expect("u32 token fits usize"),
    );
    let visible_acknowledgement = visible.acknowledgement();
    assert_eq!(
        fixture.wait_for(&visible_acknowledgement, Duration::from_secs(5)),
        visible_acknowledgement
    );

    let absent = FixtureVisualCommand::new(
        FixtureVisualState::Absent,
        NonZeroU32::new(2).expect("nonzero visual token"),
    );
    fixture.control(
        target,
        CONTROL_SET_VISUAL_ABSENT,
        usize::try_from(absent.token().get()).expect("u32 token fits usize"),
    );
    let absent_acknowledgement = absent.acknowledgement();
    assert_eq!(
        fixture.wait_for(&absent_acknowledgement, Duration::from_secs(5)),
        absent_acknowledgement
    );

    fixture.terminate();
    assert!(fixture.child.is_none());
    assert!(fixture.reader.is_none());
}

#[test]
#[ignore = "opens and activates real fixture windows; run deliberately on an unlocked desktop"]
fn ordinary_window_message_native_matrix() {
    let _serial = NATIVE_MATRIX.lock().expect("native matrix serialized");
    // SAFETY: DPI awareness is fixed before this test calls USER32.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }
        .expect("selected per-monitor-v2 DPI awareness");
    let mut foreground = OwnedForeground::establish();

    let mut delivery = FixtureProcess::spawn("delivery");
    let (target, controller) = open_ordinary(&delivery);
    let desktop = DesktopState::capture();
    assert_eq!(
        desktop.foreground,
        foreground.fixture.target(),
        "the owned unrelated fixture is foreground before delivery"
    );
    assert_ne!(
        desktop.foreground,
        delivery.target(),
        "the no-activation delivery target remains in the background"
    );
    let request = InputRequest::new(
        target,
        complete_sequence(),
        DeliveryPlan::require(InputDelivery::WindowMessage),
    )
    .with_focus(FocusPolicy::Preserve);
    let receipt = execute_observed(
        &foreground,
        &controller,
        &request,
        &timed(Duration::from_secs(10)),
    )
    .expect("ordinary sequence receipted");
    assert_eq!(receipt.outcome(), SequenceOutcome::Complete, "{receipt:?}");
    assert_eq!(receipt.selected_route(), Some(InputDelivery::WindowMessage));
    assert_eq!(receipt.submitted(), request.sequence().len());
    assert_eq!(
        receipt.evidence(),
        Some(SubmissionEvidence::TargetQueueAdmission)
    );

    let report = delivery.report(delivery.target());
    assert_eq!(report.target, 22, "every accepted native unit was observed");
    assert_wrong_targets_clear(report);
    desktop.assert_unchanged();

    delivery.control(delivery.target(), CONTROL_DUPLICATE_METADATA, 0);
    delivery.wait_for("control duplicate-metadata=ready", Duration::from_secs(5));
    let duplicate = key_pair_request(target);
    let desktop = DesktopState::capture();
    let receipt = execute_observed(
        &foreground,
        &controller,
        &duplicate,
        &timed(Duration::from_secs(5)),
    )
    .expect("duplicate metadata request receipted");
    assert_eq!(receipt.outcome(), SequenceOutcome::Complete);
    let report = delivery.report(delivery.target());
    assert_eq!(report.target, 24);
    assert_wrong_targets_clear(report);
    desktop.assert_unchanged();
    controller
        .close(&timed(Duration::from_secs(5)))
        .expect("closed controller");
    let after_cleanup = delivery.report(delivery.target());
    assert_eq!(
        after_cleanup.target, 24,
        "close submitted no redundant release"
    );
    controller
        .close(&timed(Duration::from_secs(5)))
        .expect("repeated close is idempotent");
    assert_eq!(
        delivery.report(delivery.target()).target,
        24,
        "repeated close submitted no second release"
    );
    drop(delivery);
    foreground.assert_stable();

    let mut reparented = FixtureProcess::spawn("reparent");
    let reparented_provider = WindowsCaptureProvider::new(Arc::new(IdentityIssuer::new()));
    let (target, controller) = open_ordinary_title(&reparented_provider, &reparented.title());
    let retained = reparented.target();
    reparented.control(retained, CONTROL_REPARENT_TARGET, 0);
    reparented.wait_for("control reparent=ready", Duration::from_secs(5));
    let descriptor =
        InputProvider::describe(&reparented_provider, target, &timed(Duration::from_secs(5)))
            .expect("the live target still has a truthful input descriptor");
    for operation in InputOperationKind::ALL {
        assert_eq!(
            descriptor
                .capability()
                .pair(operation, InputDelivery::WindowMessage)
                .support(),
            CapabilitySupport::Unsupported,
            "a target whose exact current authority changed does not expose WindowMessage"
        );
    }
    assert_unexecuted_target_loss(&foreground, &controller, one_key_request(target));
    assert_wrong_targets_clear(reparented.report(retained));
    drop(reparented);

    let mut replaced = FixtureProcess::spawn("replacement");
    let (target, controller) = open_ordinary(&replaced);
    let retained = replaced.target();
    replaced.control(retained, CONTROL_REUSE_STRESS, 4_096);
    let stress = replaced.wait_for("control reuse-stress=ready", Duration::from_secs(15));
    assert_unexecuted_target_loss(&foreground, &controller, one_key_request(target));
    let replacement = replaced.target();
    assert_wrong_targets_clear(replaced.report(replacement));
    println!("{stress}");
    drop(replaced);

    let mut lost = FixtureProcess::spawn("target-loss");
    let (target, controller) = open_ordinary(&lost);
    let retained = lost.target();
    lost.control(retained, CONTROL_DESTROY_TARGET, 0);
    lost.wait_for("control target-loss=ready", Duration::from_secs(5));
    assert_unexecuted_target_loss(&foreground, &controller, one_key_request(target));
    drop(lost);

    let restart_token = "owner-restart";
    let first_owner = FixtureProcess::spawn(restart_token);
    let (target, controller) = open_ordinary(&first_owner);
    drop(first_owner);
    let mut restarted_owner = FixtureProcess::spawn(restart_token);
    assert_unexecuted_target_loss(&foreground, &controller, one_key_request(target));
    assert_wrong_targets_clear(restarted_owner.report(restarted_owner.target()));
    drop(restarted_owner);
    foreground.assert_stable();

    let desktop = DesktopState::capture();
    operation_profile_rows(&foreground);
    consumer_compatibility_rows(&foreground);
    terminal_and_cleanup_rows(&foreground);
    queue_pressure_rows(&foreground);
    late_effect_row(&foreground);
    desktop.assert_unchanged();
    foreground.assert_stable();
    topology_geometry_rows(&foreground);
    controlled_unrelated_activity_row(&mut foreground);
    foreground.finish();
}

fn complete_sequence() -> InputSequence {
    let point =
        Point::new(CoordinateSpace::TargetNormalized, 0.5, 0.5).expect("normalized fixture point");
    InputSequence::new(vec![
        InputEvent::PointerMove(point),
        InputEvent::PointerPress(PointerButton::Primary),
        InputEvent::PointerRelease(PointerButton::Primary),
        InputEvent::PointerPress(PointerButton::Secondary),
        InputEvent::PointerRelease(PointerButton::Secondary),
        InputEvent::PointerPress(PointerButton::Middle),
        InputEvent::PointerRelease(PointerButton::Middle),
        InputEvent::PointerScroll {
            horizontal: -2,
            vertical: 3,
        },
        InputEvent::KeyPress(Key::Modifier(Modifier::Shift)),
        InputEvent::KeyPress(Key::Function(6)),
        InputEvent::KeyRelease(Key::Function(6)),
        InputEvent::KeyRelease(Key::Modifier(Modifier::Shift)),
        InputEvent::Text("A\u{1f642}".to_owned()),
        InputEvent::Delay(Duration::from_millis(1)),
    ])
    .expect("bounded ordinary sequence")
}

fn operation_profile_rows(foreground: &OwnedForeground) {
    let mut drag = FixtureProcess::spawn("drag");
    let (target, controller) = open_ordinary(&drag);
    let start =
        Point::new(CoordinateSpace::TargetNormalized, 0.25, 0.25).expect("normalized drag start");
    let end =
        Point::new(CoordinateSpace::TargetNormalized, 0.75, 0.75).expect("normalized drag end");
    let request = InputRequest::new(
        target,
        InputSequence::new(vec![
            InputEvent::PointerMove(start),
            InputEvent::PointerPress(PointerButton::Primary),
            InputEvent::PointerMove(end),
            InputEvent::PointerRelease(PointerButton::Primary),
        ])
        .expect("bounded drag sequence"),
        DeliveryPlan::require(InputDelivery::WindowMessage),
    )
    .with_focus(FocusPolicy::Preserve);
    let receipt = execute_observed(
        foreground,
        &controller,
        &request,
        &timed(Duration::from_secs(5)),
    )
    .expect("drag row receipted");
    assert_complete_queue_receipt(&receipt, request.sequence().len());
    let report = drag.report(drag.target());
    assert_eq!(
        report.target, 6,
        "drag produced move, positioned down, held move, and positioned up"
    );
    assert_wrong_targets_clear(report);
    controller
        .close(&timed(Duration::from_secs(5)))
        .expect("closed drag row");
    println!("native-operation-row drag=passed accepted-units=6");

    let mut keys = vec![Key::Character('m')];
    keys.extend((1..=Key::MAX_FUNCTION).map(Key::Function));
    keys.extend(Modifier::ALL.map(Key::Modifier));
    keys.extend([
        Key::Enter,
        Key::Tab,
        Key::Backspace,
        Key::Delete,
        Key::Escape,
        Key::Space,
        Key::ArrowUp,
        Key::ArrowDown,
        Key::ArrowLeft,
        Key::ArrowRight,
        Key::Home,
        Key::End,
        Key::PageUp,
        Key::PageDown,
    ]);
    let mut events = Vec::with_capacity(keys.len() * 2);
    for key in keys {
        events.push(InputEvent::KeyPress(key));
        events.push(InputEvent::KeyRelease(key));
    }
    let mut keyboard = FixtureProcess::spawn("bounded-key-vocabulary");
    let (target, controller) = open_ordinary(&keyboard);
    let request = InputRequest::new(
        target,
        InputSequence::new(events).expect("bounded complete key vocabulary"),
        DeliveryPlan::require(InputDelivery::WindowMessage),
    )
    .with_focus(FocusPolicy::Preserve);
    let receipt = execute_observed(
        foreground,
        &controller,
        &request,
        &timed(Duration::from_secs(10)),
    )
    .expect("bounded key vocabulary receipted");
    assert_complete_queue_receipt(&receipt, request.sequence().len());
    let report = keyboard.report(keyboard.target());
    assert_eq!(
        usize::try_from(report.target).expect("observation count fits usize"),
        request.sequence().len(),
        "every bounded key down/up reached only the retained target"
    );
    assert_wrong_targets_clear(report);
    controller
        .close(&timed(Duration::from_secs(5)))
        .expect("closed bounded key row");
    println!(
        "native-operation-row bounded-key-vocabulary=passed logical-events={}",
        request.sequence().len()
    );
}

fn one_key_request(target: TargetId) -> InputRequest {
    InputRequest::new(
        target,
        InputSequence::new(vec![InputEvent::KeyPress(Key::Function(6))])
            .expect("bounded key sequence"),
        DeliveryPlan::require(InputDelivery::WindowMessage),
    )
    .with_focus(FocusPolicy::Preserve)
}

fn open_ordinary(fixture: &FixtureProcess) -> (TargetId, Arc<dyn InputController>) {
    let provider = WindowsCaptureProvider::new(Arc::new(IdentityIssuer::new()));
    open_ordinary_title(&provider, &fixture.title())
}

fn open_role(fixture: &FixtureProcess, role: &str) -> (TargetId, Arc<dyn InputController>) {
    let provider = WindowsCaptureProvider::new(Arc::new(IdentityIssuer::new()));
    open_ordinary_title(&provider, &fixture.role_title(role))
}

fn open_ordinary_title(
    provider: &WindowsCaptureProvider,
    title: &str,
) -> (TargetId, Arc<dyn InputController>) {
    let discovery = DiscoveryRequest::new()
        .with_kind(TargetKind::Window)
        .with_name_containing(title);
    let mut found = Vec::new();
    for _attempt in 0..100 {
        match provider.discover_matching(&discovery, &timed(Duration::from_secs(5))) {
            Ok(targets) if !targets.is_empty() => {
                found = targets;
                break;
            }
            Ok(_) => thread::sleep(Duration::from_millis(10)),
            Err(error) if error.status() == Status::Unsupported => {
                panic!("Windows discovery unavailable on the required native host")
            }
            Err(error) => panic!("ordinary fixture discovery failed: {error}"),
        }
    }
    let mut matches = found.iter().filter(|candidate| candidate.name() == title);
    let target = matches.next().expect("ordinary fixture discovered");
    assert!(matches.next().is_none(), "ordinary title resolves uniquely");
    for operation in InputOperationKind::ALL {
        let pair = target
            .capability()
            .input()
            .pair(operation, InputDelivery::WindowMessage);
        assert_eq!(pair.support(), CapabilitySupport::Unknown);
        assert_eq!(pair.address_scope(), InputAddressScope::ExactWindow);
        assert!(!pair.focus_required());
        assert_eq!(
            pair.evidence(),
            Some(SubmissionEvidence::TargetQueueAdmission)
        );
    }
    let target_id = target.id();
    let open = InputOpenRequest::new()
        .requiring(InputOperationKind::Pointer, InputDelivery::WindowMessage)
        .requiring(InputOperationKind::Keyboard, InputDelivery::WindowMessage)
        .requiring(InputOperationKind::Text, InputDelivery::WindowMessage);
    let controller =
        InputProvider::open(provider, target_id, &open, &timed(Duration::from_secs(5)))
            .expect("opened ordinary exact-window controller");
    (target_id, controller)
}

fn execute_observed(
    foreground: &OwnedForeground,
    controller: &Arc<dyn InputController>,
    request: &InputRequest,
    operation: &OperationContext,
) -> mado_pilot_core::Result<mado_pilot_input::InputReceipt> {
    foreground.observe(|| controller.execute(request, operation))
}

fn assert_unexecuted_target_loss(
    foreground: &OwnedForeground,
    controller: &Arc<dyn InputController>,
    request: InputRequest,
) {
    let receipt = execute_observed(
        foreground,
        controller,
        &request,
        &timed(Duration::from_secs(5)),
    )
    .expect("authority refusal receipted");
    assert_eq!(
        receipt.outcome(),
        SequenceOutcome::Unexecuted,
        "{receipt:?}"
    );
    assert_eq!(receipt.submitted(), 0);
    assert_eq!(receipt.fault(), Some(InputFault::TargetLost));
}

fn key_pair_request(target: TargetId) -> InputRequest {
    InputRequest::new(
        target,
        InputSequence::new(vec![
            InputEvent::KeyPress(Key::Function(6)),
            InputEvent::KeyRelease(Key::Function(6)),
        ])
        .expect("bounded key pair"),
        DeliveryPlan::require(InputDelivery::WindowMessage),
    )
    .with_focus(FocusPolicy::Preserve)
}

fn pressed_delay_request(target: TargetId) -> InputRequest {
    InputRequest::new(
        target,
        InputSequence::new(vec![
            InputEvent::KeyPress(Key::Modifier(Modifier::Shift)),
            InputEvent::Delay(Duration::from_millis(500)),
            InputEvent::KeyRelease(Key::Modifier(Modifier::Shift)),
        ])
        .expect("bounded pressed-state delay sequence"),
        DeliveryPlan::require(InputDelivery::WindowMessage),
    )
    .with_focus(FocusPolicy::Preserve)
}

fn two_pressed_delay_request(target: TargetId) -> InputRequest {
    InputRequest::new(
        target,
        InputSequence::new(vec![
            InputEvent::KeyPress(Key::Modifier(Modifier::Control)),
            InputEvent::KeyPress(Key::Modifier(Modifier::Shift)),
            InputEvent::Delay(Duration::from_millis(500)),
            InputEvent::KeyRelease(Key::Modifier(Modifier::Shift)),
            InputEvent::KeyRelease(Key::Modifier(Modifier::Control)),
        ])
        .expect("bounded two-key cleanup sequence"),
        DeliveryPlan::require(InputDelivery::WindowMessage),
    )
    .with_focus(FocusPolicy::Preserve)
    .with_cleanup_budget(CleanupBudget::at_most(1, Duration::from_secs(1)))
}

fn assert_complete_queue_receipt(receipt: &mado_pilot_input::InputReceipt, submitted: usize) {
    assert_eq!(receipt.outcome(), SequenceOutcome::Complete, "{receipt:?}");
    assert_eq!(receipt.submitted(), submitted);
    assert_eq!(receipt.selected_route(), Some(InputDelivery::WindowMessage));
    assert_eq!(
        receipt.evidence(),
        Some(SubmissionEvidence::TargetQueueAdmission)
    );
    assert!(!receipt.used_fallback());
}

fn consumer_compatibility_rows(foreground: &OwnedForeground) {
    let mut game = FixtureProcess::spawn("game-consumer");
    let (target, controller) = open_role(&game, "Game");
    let receipt = execute_observed(
        foreground,
        &controller,
        &key_pair_request(target),
        &timed(Duration::from_secs(5)),
    )
    .expect("game-like legacy receipt");
    assert_complete_queue_receipt(&receipt, 2);
    let report = game.report(game.role_window("Game"));
    assert_eq!(
        report,
        ObservationReport {
            game: 2,
            ..empty_report()
        },
        "only the game-like consumer observed the two legacy units"
    );
    controller
        .close(&timed(Duration::from_secs(5)))
        .expect("closed game row");
    drop(game);

    let mut raw = FixtureProcess::spawn("raw-consumer");
    let (target, controller) = open_role(&raw, "Raw");
    let receipt = execute_observed(
        foreground,
        &controller,
        &key_pair_request(target),
        &timed(Duration::from_secs(5)),
    )
    .expect("Raw Input consumer receipt");
    assert_complete_queue_receipt(&receipt, 2);
    let report = raw.report(raw.role_window("Raw"));
    assert_eq!(
        report,
        ObservationReport {
            raw_legacy: 2,
            ..empty_report()
        },
        "only the selected Raw Input consumer observed legacy units"
    );
    controller
        .close(&timed(Duration::from_secs(5)))
        .expect("closed raw row");
    drop(raw);

    let mut state = FixtureProcess::spawn("state-consumer");
    let (target, controller) = open_role(&state, "State");
    let before = state.report(state.role_window("State"));
    let receipt = execute_observed(
        foreground,
        &controller,
        &key_pair_request(target),
        &timed(Duration::from_secs(5)),
    )
    .expect("state-polling consumer receipt");
    assert_complete_queue_receipt(&receipt, 2);
    let report = state.report(state.role_window("State"));
    assert_eq!(
        report,
        ObservationReport {
            state_legacy: before.state_legacy + 2,
            ..before
        },
        "only the selected state-polling consumer's legacy counter changed"
    );
    controller
        .close(&timed(Duration::from_secs(5)))
        .expect("closed state row");
}

fn terminal_and_cleanup_rows(foreground: &OwnedForeground) {
    let mut preflight = FixtureProcess::spawn("terminal-preflight");
    let (target, controller) = open_ordinary(&preflight);
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let cancelled = OperationContext::new().with_cancellation(cancellation);
    let error = execute_observed(
        foreground,
        &controller,
        &key_pair_request(target),
        &cancelled,
    )
    .expect_err("pre-admission cancellation has no receipt");
    assert_eq!(error.status(), Status::Cancelled);
    let error = execute_observed(
        foreground,
        &controller,
        &key_pair_request(target),
        &timed(Duration::ZERO),
    )
    .expect_err("pre-admission deadline has no receipt");
    assert_eq!(error.status(), Status::DeadlineExceeded);
    assert_eq!(preflight.report(preflight.target()).target, 0);
    controller
        .close(&timed(Duration::from_secs(5)))
        .expect("closed preflight");
    drop(preflight);

    let mut deadline = FixtureProcess::spawn("deadline-cleanup");
    let (target, controller) = open_ordinary(&deadline);
    let started = Instant::now();
    let receipt = execute_observed(
        foreground,
        &controller,
        &pressed_delay_request(target),
        &timed(Duration::from_millis(20)),
    )
    .expect("mid-sequence deadline receipted");
    assert_elapsed_budget(
        "deadline-cleanup",
        started.elapsed(),
        Duration::from_millis(250),
    );
    assert_eq!(receipt.outcome(), SequenceOutcome::Partial, "{receipt:?}");
    assert_eq!(receipt.submitted(), 1);
    assert_eq!(receipt.fault(), Some(InputFault::DeadlineExceeded));
    assert_eq!(receipt.cleanup(), CleanupState::Complete);
    assert_eq!(receipt.cleanup_released(), 1);
    assert_eq!(receipt.cleanup_owed(), 1);
    assert_eq!(
        deadline.report(deadline.target()).target,
        2,
        "deadline cleanup released the accepted modifier"
    );
    controller
        .close(&timed(Duration::from_secs(5)))
        .expect("closed deadline");
    drop(deadline);

    let mut cancelled = FixtureProcess::spawn("cancel-cleanup");
    let (target, controller) = open_ordinary(&cancelled);
    let cancellation = CancellationToken::new();
    let context = timed(Duration::from_secs(5)).with_cancellation(cancellation.clone());
    foreground.begin_observation();
    let worker_controller = Arc::clone(&controller);
    let worker =
        thread::spawn(move || worker_controller.execute(&pressed_delay_request(target), &context));
    cancelled.wait_for_observation(
        "observation role=target family=key-down",
        Duration::from_secs(5),
    );
    let cleanup_started = Instant::now();
    cancellation.cancel();
    let receipt = worker
        .join()
        .expect("cancellation worker joined")
        .expect("mid-sequence cancellation receipted");
    foreground.end_observation();
    assert_elapsed_budget(
        "cancellation-cleanup",
        cleanup_started.elapsed(),
        Duration::from_millis(250),
    );
    assert_eq!(receipt.outcome(), SequenceOutcome::Partial, "{receipt:?}");
    assert!(
        receipt.submitted() == 1 || (receipt.submitted() == 0 && receipt.partial_native_effect()),
        "observed cancellation must conservatively account for the first event: {receipt:?}"
    );
    assert_eq!(receipt.fault(), Some(InputFault::Cancelled));
    assert_eq!(receipt.cleanup(), CleanupState::Complete);
    assert_eq!(receipt.cleanup_released(), 1);
    assert_eq!(receipt.cleanup_owed(), 1);
    assert_eq!(
        cancelled.report(cancelled.target()).target,
        2,
        "cancellation cleanup released the accepted modifier"
    );
    controller
        .close(&timed(Duration::from_secs(5)))
        .expect("closed cancellation");

    let mut failed = FixtureProcess::spawn("cleanup-failure");
    let (target, controller) = open_ordinary(&failed);
    let cancellation = CancellationToken::new();
    let context = timed(Duration::from_secs(5)).with_cancellation(cancellation.clone());
    foreground.begin_observation();
    let worker_controller = Arc::clone(&controller);
    let worker =
        thread::spawn(move || worker_controller.execute(&pressed_delay_request(target), &context));
    failed.wait_for_observation(
        "observation role=target family=key-down",
        Duration::from_secs(5),
    );
    let retained = failed.target();
    failed.control(retained, CONTROL_DESTROY_TARGET, 0);
    failed.wait_for("control target-loss=ready", Duration::from_secs(5));
    cancellation.cancel();
    let receipt = worker
        .join()
        .expect("cleanup-failure worker joined")
        .expect("failed cleanup is receipted");
    foreground.end_observation();
    assert_eq!(receipt.outcome(), SequenceOutcome::Partial, "{receipt:?}");
    assert_eq!(receipt.submitted(), 1);
    assert_eq!(receipt.fault(), Some(InputFault::Cancelled));
    assert_eq!(receipt.cleanup(), CleanupState::Incomplete);
    assert_eq!(receipt.cleanup_released(), 0);
    assert_eq!(receipt.cleanup_owed(), 1);
    controller
        .close(&timed(Duration::from_secs(5)))
        .expect("closed failed-cleanup controller");
    drop(failed);

    let mut exhausted = FixtureProcess::spawn("cleanup-exhaustion");
    let (target, controller) = open_ordinary(&exhausted);
    let cancellation = CancellationToken::new();
    let context = timed(Duration::from_secs(5)).with_cancellation(cancellation.clone());
    foreground.begin_observation();
    let worker_controller = Arc::clone(&controller);
    let worker = thread::spawn(move || {
        worker_controller.execute(&two_pressed_delay_request(target), &context)
    });
    for _pressed in 0..2 {
        exhausted.wait_for_observation(
            "observation role=target family=key-down",
            Duration::from_secs(5),
        );
    }
    thread::sleep(Duration::from_millis(10));
    cancellation.cancel();
    let receipt = worker
        .join()
        .expect("cleanup-exhaustion worker joined")
        .expect("exhausted cleanup is receipted");
    foreground.end_observation();
    assert_eq!(receipt.outcome(), SequenceOutcome::Partial, "{receipt:?}");
    assert_eq!(receipt.submitted(), 2);
    assert_eq!(receipt.fault(), Some(InputFault::Cancelled));
    assert_eq!(receipt.cleanup(), CleanupState::Exhausted);
    assert_eq!(receipt.cleanup_released(), 1);
    assert_eq!(receipt.cleanup_owed(), 2);
    assert_eq!(
        exhausted.report(exhausted.target()).target,
        3,
        "bounded cleanup released exactly one of two accepted keys"
    );
    controller
        .close(&timed(Duration::from_secs(5)))
        .expect("closed exhausted-cleanup controller");
}

fn late_effect_row(foreground: &OwnedForeground) {
    let mut fixture = FixtureProcess::spawn("late-effect");
    let (target, controller) = open_ordinary(&fixture);
    fixture.control(fixture.target(), CONTROL_BLOCK_QUEUE, 300);
    fixture.wait_for("control queue-block=ready", Duration::from_secs(5));
    let started = Instant::now();
    let receipt = execute_observed(
        foreground,
        &controller,
        &key_pair_request(target),
        &timed(Duration::from_secs(2)),
    )
    .expect("hung-target queue receipt");
    assert_complete_queue_receipt(&receipt, 2);
    assert_elapsed_budget(
        "hung-target-queue-admission",
        started.elapsed(),
        Duration::from_millis(10),
    );
    fixture.assert_no_line("observation role=target", Duration::from_millis(50));
    fixture.wait_for("control queue-block=complete", Duration::from_secs(2));
    assert_eq!(
        fixture.report(fixture.target()).target,
        2,
        "accepted messages were observed only after the receipt"
    );
    controller
        .close(&timed(Duration::from_secs(5)))
        .expect("closed late row");
}

#[derive(Debug, Clone, Copy)]
struct MonitorRow {
    bounds: RECT,
    dpi_x: u32,
    dpi_y: u32,
}

fn topology_geometry_rows(foreground: &OwnedForeground) {
    let desktop = DesktopState::capture();
    let monitors = monitor_rows();
    assert!(
        !monitors.is_empty(),
        "approved native host exposes a display"
    );

    let topology = if monitors.len() == 1 {
        "single-display"
    } else if monitors
        .iter()
        .all(|monitor| (monitor.dpi_x, monitor.dpi_y) == (monitors[0].dpi_x, monitors[0].dpi_y))
    {
        "same-dpi-multi-display"
    } else {
        "mixed-dpi-multi-display"
    };
    let unavailable = match topology {
        "single-display" => "same-dpi-multi-display,mixed-dpi-multi-display",
        "same-dpi-multi-display" => "single-display,mixed-dpi-multi-display",
        _ => "single-display,same-dpi-multi-display",
    };
    let dpi = monitors
        .iter()
        .map(|monitor| format!("{}x{}", monitor.dpi_x, monitor.dpi_y))
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "topology monitors={} dpi=[{dpi}] executed={topology} unavailable={unavailable}",
        monitors.len()
    );

    let mut fixture = FixtureProcess::spawn("topology-geometry");
    let provider = WindowsCaptureProvider::new(Arc::new(IdentityIssuer::new()));
    let (target, controller) = open_ordinary_title(&provider, &fixture.title());
    let capture = CaptureProvider::open(
        &provider,
        target,
        &OpenRequest::new().require_format(PixelFormat::Bgra8),
        &timed(Duration::from_secs(5)),
    )
    .expect("opened topology capture stream");
    let mut frame = capture
        .frame(&FrameRequest::latest(), &timed(Duration::from_secs(5)))
        .expect("captured initial topology frame");
    let mut captures = vec![capture];

    for (index, monitor) in monitors.iter().enumerate() {
        let previous = frame.stamp();
        let available_width = monitor.bounds.right - monitor.bounds.left;
        let available_height = monitor.bounds.bottom - monitor.bounds.top;
        let index = i32::try_from(index).expect("monitor count fits i32");
        let width = (480 + index * 32).min(available_width - 96).max(320);
        let height = (320 + index * 24).min(available_height - 96).max(240);
        fixture.set_geometry(
            fixture.target(),
            monitor.bounds.left + 48,
            monitor.bounds.top + 48,
            width,
            height,
        );
        fixture.wait_for("control geometry=ready", Duration::from_secs(5));
        let mut actual = RECT::default();
        // SAFETY: the fixture target is live and actual is writable.
        unsafe { GetWindowRect(fixture.target(), &raw mut actual) }
            .expect("read moved topology fixture");
        println!(
            "topology-monitor index={index} requested=({},{} {}x{}) actual=({},{} {}x{})",
            monitor.bounds.left + 48,
            monitor.bounds.top + 48,
            width,
            height,
            actual.left,
            actual.top,
            actual.right - actual.left,
            actual.bottom - actual.top,
        );
        let next_capture = CaptureProvider::open(
            &provider,
            target,
            &OpenRequest::new().require_format(PixelFormat::Bgra8),
            &timed(Duration::from_secs(5)),
        )
        .expect("reopened capture after cross-monitor geometry change");
        frame = next_capture
            .frame(&FrameRequest::latest(), &timed(Duration::from_secs(5)))
            .expect("captured destination-monitor frame");
        captures.push(next_capture);
        println!("topology-monitor index={index} capture=ready");
        // An ordinary target may receive unrelated legacy messages. Seed one before
        // the route boundary so this row proves deltas rather than misattributing an
        // absolute process-lifetime counter to the request under test.
        fixture.control(fixture.target(), WM_MOUSEMOVE, 0);
        fixture.wait_for_observation(
            "observation role=target family=pointer-move units=1",
            Duration::from_secs(5),
        );
        let before_route = fixture.report(fixture.target());
        assert_wrong_targets_clear(before_route);

        let stale = execute_observed(
            foreground,
            &controller,
            &source_pointer_request(target, previous),
            &timed(Duration::from_secs(5)),
        )
        .expect("stale source-frame refusal receipted");
        assert_eq!(stale.outcome(), SequenceOutcome::Unexecuted, "{stale:?}");
        assert_eq!(stale.submitted(), 0);
        assert_eq!(stale.fault(), Some(InputFault::GeometryChanged));
        let after_stale = fixture.report(fixture.target());
        assert_eq!(
            after_stale.target, before_route.target,
            "stale source-frame geometry reached no target"
        );
        assert_wrong_targets_clear(after_stale);

        let current = execute_observed(
            foreground,
            &controller,
            &source_pointer_and_wheel_request(target, frame.stamp()),
            &timed(Duration::from_secs(5)),
        )
        .expect("current source-frame pointer and wheel receipt");
        assert_complete_queue_receipt(&current, 2);
        let after_current = fixture.report(fixture.target());
        assert_eq!(
            after_current.target,
            before_route.target + 3,
            "one pointer and two wheel units reached the exact target"
        );
        assert_wrong_targets_clear(after_current);
        let placement = frame
            .transform()
            .target()
            .expect("Windows frame carries exact target placement");
        let origin = placement.desktop_origin();
        assert!(
            origin.0 >= f64::from(monitor.bounds.left)
                && origin.0 < f64::from(monitor.bounds.right)
                && origin.1 >= f64::from(monitor.bounds.top)
                && origin.1 < f64::from(monitor.bounds.bottom),
            "frame placement remains on the selected physical monitor: {placement:?}"
        );
        println!(
            "topology-monitor index={index} frame-scale={}x{} source=correlated stimulus=pointer+wheel",
            placement.scale().x(),
            placement.scale().y()
        );
    }

    controller
        .close(&timed(Duration::from_secs(5)))
        .expect("closed topology input");
    for capture in captures {
        capture
            .close(&timed(Duration::from_secs(5)))
            .expect("closed topology capture");
    }
    desktop.assert_unchanged();
}

fn source_pointer_request(target: TargetId, source: FrameStamp) -> InputRequest {
    let point =
        Point::new(CoordinateSpace::TargetNormalized, 0.5, 0.5).expect("normalized topology point");
    InputRequest::new(
        target,
        InputSequence::new(vec![InputEvent::PointerMove(point)])
            .expect("bounded topology sequence"),
        DeliveryPlan::require(InputDelivery::WindowMessage),
    )
    .with_focus(FocusPolicy::Preserve)
    .with_pointer_geometry(PointerGeometry::require_unchanged_since(source))
}

fn source_pointer_and_wheel_request(target: TargetId, source: FrameStamp) -> InputRequest {
    let point =
        Point::new(CoordinateSpace::TargetNormalized, 0.5, 0.5).expect("normalized topology point");
    InputRequest::new(
        target,
        InputSequence::new(vec![
            InputEvent::PointerMove(point),
            InputEvent::PointerScroll {
                horizontal: 1,
                vertical: 1,
            },
        ])
        .expect("bounded topology pointer and wheel sequence"),
        DeliveryPlan::require(InputDelivery::WindowMessage),
    )
    .with_focus(FocusPolicy::Preserve)
    .with_pointer_geometry(PointerGeometry::require_unchanged_since(source))
}

fn monitor_rows() -> Vec<MonitorRow> {
    let mut handles = Vec::<usize>::new();
    let pointer = (&raw mut handles).addr().cast_signed();
    // SAFETY: the vector remains live for the synchronous monitor enumeration.
    let enumerated =
        unsafe { EnumDisplayMonitors(None, None, Some(collect_monitor), LPARAM(pointer)) };
    assert!(enumerated.as_bool(), "enumerated native monitors");

    let mut rows = handles
        .into_iter()
        .map(|raw| {
            let monitor = HMONITOR(std::ptr::with_exposed_provenance_mut::<std::ffi::c_void>(
                raw,
            ));
            let mut info = MONITORINFO {
                cbSize: u32::try_from(size_of::<MONITORINFO>()).expect("MONITORINFO fits u32"),
                ..MONITORINFO::default()
            };
            // SAFETY: monitor came from enumeration and info is fully writable.
            assert!(unsafe { GetMonitorInfoW(monitor, &raw mut info) }.as_bool());
            let mut dpi_x = 0;
            let mut dpi_y = 0;
            // SAFETY: monitor came from enumeration and both outputs are writable.
            unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &raw mut dpi_x, &raw mut dpi_y) }
                .expect("read monitor DPI");
            MonitorRow {
                bounds: info.rcMonitor,
                dpi_x,
                dpi_y,
            }
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|monitor| {
        let contains_initial = monitor.bounds.left <= 120
            && 120 < monitor.bounds.right
            && monitor.bounds.top <= 120
            && 120 < monitor.bounds.bottom;
        (!contains_initial, monitor.bounds.left, monitor.bounds.top)
    });
    rows
}

unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _device_context: HDC,
    _bounds: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let pointer = std::ptr::with_exposed_provenance_mut::<Vec<usize>>(data.0.cast_unsigned());
    // SAFETY: monitor_rows supplied the vector for this synchronous callback.
    unsafe { &mut *pointer }.push(monitor.0.addr());
    true.into()
}

fn controlled_unrelated_activity_row(foreground: &mut OwnedForeground) {
    let mut delivery = FixtureProcess::spawn("unrelated-activity");
    let (target, controller) = open_ordinary(&delivery);
    let foreground_window = foreground.fixture.target();
    let desktop = DesktopState::capture();
    assert_eq!(
        desktop.foreground, foreground_window,
        "the repository-owned unrelated application is foreground"
    );
    assert_ne!(
        desktop.foreground,
        delivery.target(),
        "the delivery target remains in the background"
    );
    let before_activity = foreground.fixture_mut().report(foreground_window);
    let foreground_value = foreground_window.0.addr();
    let receipt = foreground
        .observe(|| {
            let activity = thread::spawn(move || {
                let foreground = HWND(std::ptr::with_exposed_provenance_mut::<std::ffi::c_void>(
                    foreground_value,
                ));
                for _unit in 0..64 {
                    // SAFETY: the retained foreground fixture is live; this message is scalar.
                    unsafe { PostMessageW(Some(foreground), WM_MOUSEMOVE, WPARAM(0), LPARAM(0)) }
                        .expect("posted controlled unrelated activity");
                    assert_eq!(
                        // SAFETY: the return is an opaque handle and is not dereferenced.
                        unsafe { GetForegroundWindow() },
                        foreground,
                        "owned foreground authority remained stable during activity"
                    );
                    thread::sleep(Duration::from_millis(1));
                }
            });
            let request = InputRequest::new(
                target,
                InputSequence::new(vec![
                    InputEvent::KeyPress(Key::Function(6)),
                    InputEvent::Delay(Duration::from_millis(100)),
                    InputEvent::KeyRelease(Key::Function(6)),
                ])
                .expect("bounded activity sequence"),
                DeliveryPlan::require(InputDelivery::WindowMessage),
            )
            .with_focus(FocusPolicy::Preserve);
            let receipt = controller.execute(&request, &timed(Duration::from_secs(5)));
            activity.join().expect("unrelated activity joined");
            receipt
        })
        .expect("delivery during unrelated activity");
    assert_complete_queue_receipt(&receipt, 3);
    let report = delivery.report(delivery.target());
    assert_eq!(report.target, 2);
    assert_wrong_targets_clear(report);
    let after_activity = foreground.fixture_mut().report(foreground_window);
    assert_eq!(
        after_activity.target,
        before_activity.target + 64,
        "the actual foreground fixture observed every controlled activity unit"
    );
    assert_wrong_targets_clear(after_activity);
    desktop.assert_unchanged();
    controller
        .close(&timed(Duration::from_secs(5)))
        .expect("closed activity row");
}

fn queue_pressure_rows(foreground: &OwnedForeground) {
    let mut full = FixtureProcess::spawn("queue-full");
    let (target_id, controller) = open_ordinary(&full);
    let target = full.target();
    full.control(target, CONTROL_BLOCK_QUEUE, 300);
    full.wait_for("control queue-block=ready", Duration::from_secs(5));
    let capacity = fill_queue(target);
    assert!(
        capacity >= 1_000,
        "unexpectedly small Windows message queue"
    );
    let started = Instant::now();
    let receipt = execute_observed(
        foreground,
        &controller,
        &one_key_request(target_id),
        &timed(Duration::from_secs(5)),
    )
    .expect("queue refusal receipted");
    assert_elapsed_budget(
        "queue-full-refusal",
        started.elapsed(),
        Duration::from_millis(10),
    );
    assert_eq!(
        receipt.outcome(),
        SequenceOutcome::Unexecuted,
        "{receipt:?}"
    );
    assert_eq!(receipt.submitted(), 0);
    assert_eq!(receipt.fault(), Some(InputFault::SubmissionFailed));
    drop(full);

    let mut partial = FixtureProcess::spawn("queue-partial");
    let (target_id, controller) = open_ordinary(&partial);
    let target = partial.target();
    partial.control(target, CONTROL_BLOCK_QUEUE, 300);
    partial.wait_for("control queue-block=ready", Duration::from_secs(5));
    enqueue_nulls(target, capacity.saturating_sub(2));
    let point =
        Point::new(CoordinateSpace::TargetNormalized, 0.5, 0.5).expect("normalized fixture point");
    let sequence = InputSequence::new(vec![
        InputEvent::PointerMove(point),
        InputEvent::PointerScroll {
            horizontal: 1,
            vertical: 1,
        },
    ])
    .expect("bounded partial sequence");
    let request = InputRequest::new(
        target_id,
        sequence,
        DeliveryPlan::require(InputDelivery::WindowMessage),
    )
    .with_focus(FocusPolicy::Preserve);
    let started = Instant::now();
    let receipt = execute_observed(
        foreground,
        &controller,
        &request,
        &timed(Duration::from_secs(5)),
    )
    .expect("partial queue refusal receipted");
    assert_elapsed_budget(
        "queue-partial-refusal",
        started.elapsed(),
        Duration::from_millis(10),
    );
    assert_eq!(receipt.outcome(), SequenceOutcome::Partial, "{receipt:?}");
    assert_eq!(receipt.submitted(), 1);
    assert!(receipt.partial_native_effect());
    assert_eq!(receipt.selected_route(), Some(InputDelivery::WindowMessage));
    assert_eq!(
        receipt.evidence(),
        Some(SubmissionEvidence::TargetQueueAdmission)
    );
    assert!(!receipt.used_fallback());
    assert_eq!(receipt.fault(), Some(InputFault::SubmissionFailed));
    partial.wait_for("control queue-block=complete", Duration::from_secs(5));
    partial.wait_for_observation(
        "observation role=target family=pointer-move",
        Duration::from_secs(5),
    );
    partial.wait_for_observation(
        "observation role=target family=vertical-wheel",
        Duration::from_secs(5),
    );
    let report = partial.report(target);
    assert_eq!(
        report.target, 2,
        "only the accepted pointer and vertical-wheel prefix reached the target"
    );
    assert_wrong_targets_clear(report);
    controller
        .close(&timed(Duration::from_secs(5)))
        .expect("closed partial-wheel controller");
    drop(partial);

    let mut text = FixtureProcess::spawn("queue-partial-text");
    let (target_id, controller) = open_ordinary(&text);
    let target = text.target();
    text.control(target, CONTROL_BLOCK_QUEUE, 1_000);
    text.wait_for("control queue-block=ready", Duration::from_secs(5));
    enqueue_nulls(target, capacity.saturating_sub(1));
    let request = InputRequest::new(
        target_id,
        InputSequence::new(vec![InputEvent::Text("😀".to_owned())])
            .expect("bounded surrogate-pair sequence"),
        DeliveryPlan::require(InputDelivery::WindowMessage),
    )
    .with_focus(FocusPolicy::Preserve);
    let receipt = execute_observed(
        foreground,
        &controller,
        &request,
        &timed(Duration::from_secs(5)),
    )
    .expect("partial UTF-16 queue refusal receipted");
    assert_eq!(receipt.outcome(), SequenceOutcome::Partial, "{receipt:?}");
    assert_eq!(receipt.submitted(), 0);
    assert!(receipt.partial_native_effect());
    assert_eq!(receipt.fault(), Some(InputFault::SubmissionFailed));
    text.wait_for("control queue-block=complete", Duration::from_secs(5));
    text.wait_for_observation(
        "observation role=target family=text-unit",
        Duration::from_secs(5),
    );
    let report = text.report(target);
    assert_eq!(
        report.target, 1,
        "only the first UTF-16 unit reached the target queue"
    );
    assert_wrong_targets_clear(report);
    controller
        .close(&timed(Duration::from_secs(5)))
        .expect("closed partial-text controller");
}

fn fill_queue(hwnd: HWND) -> usize {
    for accepted in 0..20_000usize {
        // SAFETY: WM_NULL carries no payload and targets the blocked fixture queue.
        if unsafe { PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0)) }.is_err() {
            return accepted;
        }
    }
    panic!("Windows queue admitted more than the bounded pressure workload");
}

fn enqueue_nulls(hwnd: HWND, count: usize) {
    for _index in 0..count {
        // SAFETY: WM_NULL carries no payload and targets the blocked fixture queue.
        unsafe { PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0)) }
            .expect("filled the measured queue capacity");
    }
}

fn empty_report() -> ObservationReport {
    ObservationReport {
        target: 0,
        replacement: 0,
        game: 0,
        sibling: 0,
        child: 0,
        foreground: 0,
        raw_legacy: 0,
        state_legacy: 0,
        raw: 0,
        state: 0,
    }
}

fn assert_wrong_targets_clear(report: ObservationReport) {
    assert_eq!(report.replacement, 0, "replacement received legacy input");
    assert_eq!(report.game, 0, "game-like sibling received legacy input");
    assert_eq!(report.sibling, 0, "sibling received legacy input");
    assert_eq!(report.child, 0, "child received legacy input");
    assert_eq!(
        report.foreground, 0,
        "unrelated foreground fixture received input"
    );
    assert_eq!(report.raw_legacy, 0, "raw sibling received legacy input");
    assert_eq!(
        report.state_legacy, 0,
        "state sibling received legacy input"
    );
    assert_eq!(report.raw, 0, "legacy posts synthesized Raw Input");
    assert_eq!(
        report.state, 0,
        "legacy posts changed asynchronous key state"
    );
}

fn parse_report(line: &str) -> ObservationReport {
    let mut values = [0u32; 10];
    for (index, name) in [
        "target",
        "replacement",
        "game",
        "sibling",
        "child",
        "foreground",
        "raw-legacy",
        "state-legacy",
        "raw",
        "state",
    ]
    .iter()
    .enumerate()
    {
        let field = line
            .split_whitespace()
            .find_map(|field| field.strip_prefix(&format!("{name}=")))
            .unwrap_or_else(|| panic!("report omitted {name}: {line}"));
        values[index] = field.parse().expect("report count is numeric");
    }
    ObservationReport {
        target: values[0],
        replacement: values[1],
        game: values[2],
        sibling: values[3],
        child: values[4],
        foreground: values[5],
        raw_legacy: values[6],
        state_legacy: values[7],
        raw: values[8],
        state: values[9],
    }
}

fn find_window(title: &str) -> HWND {
    let class = wide(ORDINARY_CLASS_NAME);
    let title = wide(title);
    // SAFETY: both strings are terminated and owned for this lookup.
    unsafe { FindWindowW(PCWSTR(class.as_ptr()), PCWSTR(title.as_ptr())) }
        .expect("found exact ordinary fixture window")
}

fn assert_elapsed_budget(row: &str, elapsed: Duration, ceiling: Duration) {
    println!(
        "performance row={row} elapsed_us={} ceiling_us={}",
        elapsed.as_micros(),
        ceiling.as_micros()
    );
    assert!(
        elapsed <= ceiling,
        "{row} took {elapsed:?}, exceeding its {ceiling:?} ceiling"
    );
}

fn timed(timeout: Duration) -> OperationContext {
    OperationContext::new()
        .with_timeout(timeout)
        .expect("representable native timeout")
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
