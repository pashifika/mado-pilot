#![cfg_attr(not(windows), allow(missing_docs))]
#![cfg(windows)]
//! Native ordinary-window delivery, authority, and isolation coverage.

#[path = "../src/bin/support/ordinary_fixture_startup.rs"]
mod ordinary_fixture_startup;

use std::collections::VecDeque;
use std::mem::size_of;
use std::num::NonZeroU32;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, Once};
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
    CONTROL_BLOCK_QUEUE, CONTROL_DESTROY_TARGET, CONTROL_DUPLICATE_METADATA,
    CONTROL_REPARENT_TARGET, CONTROL_REPORT, CONTROL_REUSE_STRESS, CONTROL_SET_GEOMETRY,
    CONTROL_SET_VISUAL_ABSENT, CONTROL_SET_VISUAL_VISIBLE, FixtureVisualCommand,
    FixtureVisualState, ORDINARY_CLASS_NAME, TARGET_LOSS_ACKNOWLEDGEMENT, ordinary_fixture_title,
};
use ordinary_fixture_startup::{
    DpiMode, DpiScope, Failure as StartupFailure, PREFIX as STARTUP_ERROR_PREFIX,
    ParseError as StartupParseError,
};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows::Win32::UI::HiDpi::{
    AreDpiAwarenessContextsEqual, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor,
    GetThreadDpiAwarenessContext, MDT_EFFECTIVE_DPI, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetForegroundWindow, GetPhysicalCursorPos, GetSystemMetrics, GetWindowRect,
    PostMessageW, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SetForegroundWindow,
    SetPhysicalCursorPos, WM_MOUSEMOVE, WM_NULL,
};
use windows::core::{BOOL, PCWSTR};

static NATIVE_MATRIX: Mutex<()> = Mutex::new(());
static PROCESS_DPI: Once = Once::new();

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

#[derive(Debug, PartialEq, Eq)]
enum StartupObservation {
    Ready(String),
    Failure(StartupFailure),
    Pending(String),
    Malformed {
        error: StartupParseError,
        byte_len: usize,
        field_count: usize,
    },
    Timeout,
    Disconnected,
}

fn classify_startup_observation(
    observation: Result<String, RecvTimeoutError>,
) -> StartupObservation {
    match observation {
        Ok(line) if line.starts_with("fixture-ready ") => StartupObservation::Ready(line),
        Ok(line) if line.starts_with(STARTUP_ERROR_PREFIX) => {
            let byte_len = line.len();
            let field_count = line.split_ascii_whitespace().count();
            match line.parse() {
                Ok(failure) => StartupObservation::Failure(failure),
                Err(error) => StartupObservation::Malformed {
                    error,
                    byte_len,
                    field_count,
                },
            }
        }
        Ok(line) => StartupObservation::Pending(line),
        Err(RecvTimeoutError::Timeout) => StartupObservation::Timeout,
        Err(RecvTimeoutError::Disconnected) => StartupObservation::Disconnected,
    }
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
        Self::spawn_with_options(token.into(), false, false)
    }

    fn spawn_activated(token: impl Into<String>) -> Self {
        Self::spawn_with_options(token.into(), true, false)
    }

    fn spawn_activated_with_injected_failure(token: impl Into<String>) -> Self {
        Self::spawn_with_options(token.into(), true, true)
    }

    fn spawn_with_options(token: String, activate: bool, inject_failure: bool) -> Self {
        Self::spawn_configured(token, activate, inject_failure, None, None)
    }

    fn spawn_coordinates(token: String, dpi: DpiMode, activate: bool) -> Self {
        Self::spawn_configured(token, activate, false, Some(dpi), None)
    }

    fn spawn_configured(
        token: String,
        activate: bool,
        inject_failure: bool,
        dpi: Option<DpiMode>,
        input_oracle: Option<&str>,
    ) -> Self {
        let mut command = Command::new(env!(
            "CARGO_BIN_EXE_mado-pilot-windows-window-message-fixture"
        ));
        command.arg(format!("--title-token={token}"));
        if let Some(dpi) = dpi {
            command
                .arg(format!("--dpi={}", dpi.label()))
                .arg("--coordinate-oracle");
        }
        if let Some(mode) = input_oracle {
            require_ordinary_input();
            command
                .arg("--input-oracle")
                .arg(mode)
                .env("MADO_PILOT_WINDOWS_ORDINARY_INPUT", "1");
        }
        if activate {
            command.arg("--activate");
        }
        if inject_failure {
            command.arg("--fail-stage=foreground-request");
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
        let ready = fixture.wait_for_ready(Duration::from_secs(5));
        assert!(ready.contains(&format!("class={ORDINARY_CLASS_NAME}")));
        assert!(ready.contains(&format!("title={}", fixture.title())));
        if activate {
            assert!(
                ready.ends_with(" activation=direct") || ready.ends_with(" activation=attached"),
                "activated fixture must attribute its readiness path"
            );
        } else {
            assert!(
                !ready.ends_with(" activation=direct") && !ready.ends_with(" activation=attached"),
                "non-activated fixture readiness must remain unchanged"
            );
        }
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

    fn wait_for_ready(&mut self, timeout: Duration) -> String {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match classify_startup_observation(self.lines.recv_timeout(remaining)) {
                StartupObservation::Ready(line) => return line,
                StartupObservation::Failure(failure) => {
                    let exit = self.wait_for_child_exit().and_then(|status| status.code());
                    panic!("fixture startup failed: {failure}; exit={exit:?}");
                }
                StartupObservation::Pending(line) => self.pending.push_back(line),
                StartupObservation::Malformed {
                    error,
                    byte_len,
                    field_count,
                } => {
                    panic!(
                        "fixture startup record was invalid: {error}; bytes={byte_len} fields={field_count}"
                    )
                }
                StartupObservation::Timeout => {
                    panic!("fixture readiness timed out before a bounded result")
                }
                StartupObservation::Disconnected => {
                    let pending = self.pending.len();
                    let exit = self.wait_for_child_exit().and_then(|status| status.code());
                    panic!("fixture disconnected before readiness: exit={exit:?} pending={pending}")
                }
            }
        }
    }

    fn wait_for_child_exit(&mut self) -> Option<ExitStatus> {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if let Some(status) = self
                .child
                .as_mut()
                .expect("fixture child retained")
                .try_wait()
                .expect("fixture child status observed")
            {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(1));
        }
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

    fn terminate(&mut self) -> bool {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut complete = true;
        if let Some(mut child) = self.child.take() {
            let _killed = child.kill();
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if Instant::now() < deadline => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    _ => {
                        complete = false;
                        break;
                    }
                }
            }
        }
        if let Some(reader) = self.reader.take() {
            while !reader.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(2));
            }
            if reader.is_finished() {
                complete &= reader.join().is_ok();
            } else {
                complete = false;
            }
        }
        complete
    }
}

impl Drop for FixtureProcess {
    fn drop(&mut self) {
        if !self.terminate() {
            eprintln!("fixture teardown incomplete");
        }
    }
}

#[derive(Debug)]
struct DesktopState {
    cursor: POINT,
    foreground: HWND,
}

impl DesktopState {
    fn capture() -> Self {
        let _physical = DpiScope::enter(DpiMode::PerMonitorV2).expect("physical snapshot DPI");
        let mut cursor = POINT::default();
        // SAFETY: cursor is a complete writable POINT.
        unsafe { GetPhysicalCursorPos(&raw mut cursor) }.expect("read physical cursor");
        // SAFETY: the return is an opaque snapshot and is not dereferenced.
        let foreground = unsafe { GetForegroundWindow() };
        Self { cursor, foreground }
    }

    fn assert_unchanged(&self) {
        let current = Self::capture();
        assert_eq!(
            current.cursor, self.cursor,
            "WindowMessage moved the physical cursor"
        );
        assert_eq!(
            current.foreground, self.foreground,
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
        let fixture = FixtureProcess::spawn_activated("owned-unrelated-foreground");
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
                if unsafe { GetPhysicalCursorPos(&raw mut current) }.is_err() {
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

    fn stop_monitor(&mut self) -> bool {
        self.stop.store(true, Ordering::Release);
        if let Some(monitor) = self.monitor.take() {
            let deadline = Instant::now() + Duration::from_secs(2);
            while !monitor.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(2));
            }
            return monitor.is_finished() && monitor.join().is_ok();
        }
        true
    }

    fn finish(mut self) {
        self.assert_stable();
        assert!(self.stop_monitor(), "bounded foreground monitor teardown");
        assert_eq!(
            self.violation.load(Ordering::Acquire),
            0,
            "owned foreground monitor observed no transient mutation"
        );
        self.fixture.shutdown_input();
        println!("teardown foreground-monitor=complete foreground-fixture=complete");
    }
}

impl Drop for OwnedForeground {
    fn drop(&mut self) {
        if !self.stop_monitor() {
            eprintln!("foreground monitor teardown incomplete");
        }
        // SAFETY: the saved coordinates came from GetPhysicalCursorPos.
        let _restored =
            unsafe { SetPhysicalCursorPos(self.original.cursor.x, self.original.cursor.y) };
        if self.original.foreground != HWND::default() {
            // SAFETY: this is the opaque foreground handle captured before the test.
            let _restored = unsafe { SetForegroundWindow(self.original.foreground) };
        }
    }
}

fn select_process_dpi_awareness() {
    PROCESS_DPI.call_once(|| {
        // SAFETY: the process default is fixed before these tests call USER32.
        unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }
            .expect("selected per-monitor-v2 DPI awareness");
    });
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

    assert!(fixture.terminate(), "bounded fixture teardown");
    assert!(fixture.child.is_none());
    assert!(fixture.reader.is_none());
}

#[test]
#[ignore = "opens a real fixture window; run deliberately on an unlocked desktop"]
fn wgc_reports_target_loss_after_acknowledged_fixture_destruction() {
    let _serial = NATIVE_MATRIX.lock().expect("native matrix serialized");
    let mut fixture = FixtureProcess::spawn("watcher-target-loss");
    let issuer = Arc::new(IdentityIssuer::new());
    let provider = WindowsCaptureProvider::new(issuer);
    let targets = provider
        .discover_matching(
            &DiscoveryRequest::new()
                .with_kind(TargetKind::Window)
                .with_name_containing(fixture.title()),
            &timed(Duration::from_secs(5)),
        )
        .expect("fixture target discovery");
    let target = targets.first().expect("fixture target").id();
    let session = CaptureProvider::open(
        &provider,
        target,
        &OpenRequest::new().require_format(PixelFormat::Bgra8),
        &timed(Duration::from_secs(5)),
    )
    .expect("fixture WGC session");
    let first = session
        .frame(&FrameRequest::latest(), &timed(Duration::from_secs(5)))
        .expect("fixture first frame");
    let first_stamp = first.stamp();
    drop(first);

    let (started_sender, started_receiver) = mpsc::sync_channel(0);
    let (activity_sender, activity_receiver) = mpsc::channel();
    let (terminal_sender, terminal_receiver) = mpsc::sync_channel(1);
    let waiter_session = Arc::clone(&session);
    let waiter = thread::spawn(move || {
        started_sender.send(()).expect("start waiter");
        let mut stamp = first_stamp;
        loop {
            match waiter_session.frame(&FrameRequest::newer_than(stamp), &OperationContext::new()) {
                Ok(frame) => {
                    stamp = frame.stamp();
                    let _activity = activity_sender.send(());
                }
                Err(error) => {
                    terminal_sender
                        .send(error.status())
                        .expect("report terminal frame status");
                    return;
                }
            }
        }
    });
    started_receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("frame waiter starts");

    let quiet_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = quiet_deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "fixture never reached a 200 ms in-flight frame wait"
        );
        match activity_receiver.recv_timeout(remaining.min(Duration::from_millis(200))) {
            Ok(()) => {}
            Err(RecvTimeoutError::Timeout) => break,
            Err(RecvTimeoutError::Disconnected) => {
                let status = terminal_receiver
                    .try_recv()
                    .expect("frame waiter terminated without reporting status");
                panic!("frame waiter terminated before fixture destruction with {status:?}");
            }
        }
    }

    fixture.control(fixture.target(), CONTROL_DESTROY_TARGET, 0);
    assert_eq!(
        fixture.wait_for(TARGET_LOSS_ACKNOWLEDGEMENT, Duration::from_secs(5)),
        TARGET_LOSS_ACKNOWLEDGEMENT
    );
    let acknowledged_at = Instant::now();
    let status = terminal_receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("in-flight frame wait terminates after acknowledged fixture destruction");
    let detection_latency = acknowledged_at.elapsed();
    assert_eq!(
        status,
        Status::TargetLost,
        "acknowledged fixture destruction must terminate the in-flight WGC wait as TargetLost"
    );
    assert!(
        detection_latency < Duration::from_secs(1),
        "SystemClock latency from acknowledged destruction to TargetLost must stay below one second; observed {detection_latency:?}"
    );
    waiter.join().expect("frame waiter joins");

    session
        .close(&timed(Duration::from_secs(20)))
        .expect("target-lost session close");
    assert!(fixture.terminate(), "bounded fixture teardown");
}

#[test]
fn startup_observation_classifies_ready_failure_malformed_timeout_and_disconnect() {
    let ready = "fixture-ready class=ordinary title=fixture capacity=512".to_owned();
    assert_eq!(
        classify_startup_observation(Ok(ready.clone())),
        StartupObservation::Ready(ready)
    );

    let failure = "fixture-startup-error version=2 stage=foreground-request primary-kind=boolean primary-code=none foreground=absent attach=skipped direct-request=refused dpi-after-failure=not-observed ambient-win32=0x00000000 cleanup-stage=none cleanup-kind=none cleanup-code=none";
    match classify_startup_observation(Ok(failure.to_owned())) {
        StartupObservation::Failure(parsed) => assert_eq!(parsed.to_string(), failure),
        observation => panic!("expected startup failure, got {observation:?}"),
    }

    let malformed = format!("{STARTUP_ERROR_PREFIX}version=1");
    match classify_startup_observation(Ok(malformed.clone())) {
        StartupObservation::Malformed {
            error,
            byte_len,
            field_count,
        } => {
            assert_eq!(error.to_string(), "record version is unsupported");
            assert_eq!(byte_len, malformed.len());
            assert_eq!(field_count, 2);
        }
        observation => panic!("expected malformed startup record, got {observation:?}"),
    }

    assert_eq!(
        classify_startup_observation(Err(RecvTimeoutError::Timeout)),
        StartupObservation::Timeout
    );
    assert_eq!(
        classify_startup_observation(Err(RecvTimeoutError::Disconnected)),
        StartupObservation::Disconnected
    );
}

#[test]
#[ignore = "opens and activates real fixture windows; run deliberately on an unlocked desktop"]
fn activated_ordinary_fixture_startup_reaches_ready() {
    let _serial = NATIVE_MATRIX.lock().expect("native matrix serialized");
    select_process_dpi_awareness();
    OwnedForeground::establish().finish();
}

#[test]
#[ignore = "opens and activates real fixture windows; run deliberately on an unlocked desktop"]
fn injected_foreground_request_failure_is_reported() {
    let _serial = NATIVE_MATRIX.lock().expect("native matrix serialized");
    select_process_dpi_awareness();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        FixtureProcess::spawn_activated_with_injected_failure("startup-red-control")
    }))
    .expect_err("injected startup failure must not reach readiness");
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .expect("startup failure panic carries text");
    let request_failure = message
        .contains("version=2 stage=foreground-request primary-kind=boolean primary-code=none")
        && message.contains("ambient-win32=0x00000000");
    let environmental_attach_failure = message
        .contains("version=2 stage=foreground-attach primary-kind=windows-hresult")
        && message.contains("foreground=present attach=attempted direct-request=refused")
        && message.contains("ambient-win32=none");
    assert!(
        message.contains("direct-request=refused")
            && message.contains("exit=Some(1)")
            && (request_failure || environmental_attach_failure),
        "unexpected bounded startup failure: {message}"
    );
}

#[test]
#[ignore = "opens and activates real fixture windows; run deliberately on an unlocked desktop"]
fn ordinary_window_message_native_matrix() {
    let _serial = NATIVE_MATRIX.lock().expect("native matrix serialized");
    select_process_dpi_awareness();
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

#[derive(Debug, Clone, Copy)]
struct CoordinateOracle {
    client: (i32, i32),
    screen: (i32, i32),
    physical: (i32, i32),
    physical_span: (i32, i32),
    logical_span: i32,
}

fn coordinate_field(line: &str, name: &str) -> i32 {
    line.split_whitespace()
        .find_map(|field| field.strip_prefix(name))
        .unwrap_or_else(|| panic!("coordinate record omitted {name}: {line}"))
        .parse()
        .expect("signed coordinate field")
}

impl FixtureProcess {
    fn coordinate_oracle(&mut self) -> CoordinateOracle {
        self.control(self.target(), CONTROL_REPORT, 1);
        let line = self.wait_for("coordinate-oracle ", Duration::from_secs(5));
        assert!(!line.contains("failed="), "{line}");
        let oracle = CoordinateOracle {
            client: (
                coordinate_field(&line, "client-x="),
                coordinate_field(&line, "client-y="),
            ),
            screen: (
                coordinate_field(&line, "screen-x="),
                coordinate_field(&line, "screen-y="),
            ),
            physical: (
                coordinate_field(&line, "physical-x="),
                coordinate_field(&line, "physical-y="),
            ),
            physical_span: (
                coordinate_field(&line, "span-x="),
                coordinate_field(&line, "span-y="),
            ),
            logical_span: coordinate_field(&line, "logical-span="),
        };
        assert!(oracle.physical_span.0 > 0 && oracle.physical_span.1 > 0);
        assert!(oracle.logical_span > 0);
        self.wait_for("report ", Duration::from_secs(5));
        oracle
    }

    fn assert_coordinate(
        &mut self,
        family: &str,
        after: u32,
        expected: (i32, i32),
        tolerance: i32,
    ) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let line = self.wait_for(
                "coordinate ",
                deadline.saturating_duration_since(Instant::now()),
            );
            if !line.contains(&format!("family={family} ")) {
                continue;
            }
            let sequence = coordinate_field(&line, "sequence=");
            if sequence <= i32::try_from(after).expect("bounded fixture count") {
                continue;
            }
            assert!(
                line.contains("role=target "),
                "wrong window observed the event: {line}"
            );
            assert_point_near(
                (coordinate_field(&line, "x="), coordinate_field(&line, "y=")),
                expected,
                tolerance,
            );
            return;
        }
    }
}

fn assert_point_near(actual: (i32, i32), expected: (i32, i32), tolerance: i32) {
    assert!(
        (i64::from(actual.0) - i64::from(expected.0)).abs() <= i64::from(tolerance)
            && (i64::from(actual.1) - i64::from(expected.1)).abs() <= i64::from(tolerance),
        "arrival {actual:?} differs from {expected:?}, declared tolerance={tolerance}",
    );
}

fn dpi_modes(variable: &str) -> Vec<DpiMode> {
    match std::env::var(variable) {
        Ok(value) => vec![value.parse().expect("explicit supported DPI selector")],
        Err(std::env::VarError::NotPresent) => {
            vec![DpiMode::Unaware, DpiMode::System, DpiMode::PerMonitorV2]
        }
        Err(error) => panic!("invalid DPI selector: {error}"),
    }
}

fn declared_monitors_match(declaration: &str, monitors: &[MonitorRow]) -> bool {
    let mut rows = declaration.split(';');
    for monitor in monitors {
        let Some(row) = rows.next() else {
            return false;
        };
        let mut fields = row.split(',');
        for expected in [
            i64::from(monitor.bounds.left),
            i64::from(monitor.bounds.top),
            i64::from(monitor.bounds.right),
            i64::from(monitor.bounds.bottom),
            i64::from(monitor.dpi_x),
            i64::from(monitor.dpi_y),
        ] {
            if fields.next().and_then(|field| field.parse::<i64>().ok()) != Some(expected) {
                return false;
            }
        }
        if fields.next().is_some() {
            return false;
        }
    }
    rows.next().is_none()
}

#[test]
fn declared_dpi_monitors_reject_topology_changes() {
    let monitors = [
        MonitorRow {
            bounds: RECT {
                left: 0,
                top: 0,
                right: 2560,
                bottom: 1440,
            },
            dpi_x: 144,
            dpi_y: 144,
        },
        MonitorRow {
            bounds: RECT {
                left: -1920,
                top: 0,
                right: 0,
                bottom: 1080,
            },
            dpi_x: 120,
            dpi_y: 120,
        },
    ];
    let declaration = "0,0,2560,1440,144,144;-1920,0,0,1080,120,120";
    assert!(declared_monitors_match(declaration, &monitors));
    assert!(!declared_monitors_match(declaration, &monitors[..1]));
    assert!(!declared_monitors_match(
        declaration,
        &[monitors[0], monitors[1], monitors[0]]
    ));
    let mut changed = monitors;
    changed[1].dpi_y = 144;
    assert!(!declared_monitors_match(declaration, &changed));
    changed = monitors;
    changed[1].bounds.left = -2000;
    assert!(!declared_monitors_match(declaration, &changed));
    for malformed in ["", "0,0,2560,1440,144", "0,0,2560,1440,144,invalid"] {
        assert!(!declared_monitors_match(malformed, &monitors[..1]));
    }
}

struct DpiDesktopRestore(Option<DesktopState>);

impl DpiDesktopRestore {
    fn restore(&mut self) -> bool {
        let Some(saved) = self.0.take() else {
            return true;
        };
        // SAFETY: this opt-in row owns restoration of its saved physical cursor.
        let cursor = unsafe { SetPhysicalCursorPos(saved.cursor.x, saved.cursor.y) }.is_ok();
        // SAFETY: no handle is dereferenced; Windows validates the saved foreground.
        if unsafe { GetForegroundWindow() } != saved.foreground
            && saved.foreground != HWND::default()
        {
            // SAFETY: Windows validates the saved opaque window handle.
            let _requested = unsafe { SetForegroundWindow(saved.foreground) };
        }
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            let current = DesktopState::capture();
            if current.cursor == saved.cursor && current.foreground == saved.foreground {
                println!("dpi-teardown desktop=complete");
                return cursor;
            }
            if Instant::now() >= deadline {
                eprintln!("dpi-teardown desktop=incomplete");
                return false;
            }
            thread::sleep(Duration::from_millis(2));
        }
    }
}

impl Drop for DpiDesktopRestore {
    fn drop(&mut self) {
        let _restored = self.restore();
    }
}

struct DpiSessions {
    input: Arc<dyn InputController>,
    capture: Arc<dyn mado_pilot_capture::CaptureSession>,
    closed: bool,
}

impl DpiSessions {
    fn close(&mut self) -> bool {
        if self.closed {
            return true;
        }
        self.closed = true;
        let started = Instant::now();
        let input = self.input.close(&timed(Duration::from_secs(2))).is_ok();
        let capture = self.capture.close(&timed(Duration::from_secs(2))).is_ok();
        let bounded = started.elapsed() <= Duration::from_secs(5);
        println!("dpi-teardown input={input} capture={capture} bounded={bounded}");
        input && capture && bounded
    }
}

impl Drop for DpiSessions {
    fn drop(&mut self) {
        let _closed = self.close();
    }
}

fn assert_wrong_window_delta(before: ObservationReport, after: ObservationReport) {
    assert_eq!(
        (
            after.replacement,
            after.game,
            after.sibling,
            after.child,
            after.foreground,
            after.raw_legacy,
            after.state_legacy
        ),
        (
            before.replacement,
            before.game,
            before.sibling,
            before.child,
            before.foreground,
            before.raw_legacy,
            before.state_legacy
        ),
        "input reached a wrong owned window",
    );
}

fn execute_dpi_row(
    foreground: Option<&OwnedForeground>,
    input: &Arc<dyn InputController>,
    request: &InputRequest,
    caller: DpiMode,
) -> mado_pilot_input::InputReceipt {
    let _caller = DpiScope::enter(caller).expect("selected caller DPI context");
    // SAFETY: this reads only the calling thread's opaque DPI context.
    let incoming = unsafe { GetThreadDpiAwarenessContext() };
    let operation = timed(Duration::from_secs(5));
    let receipt = if let Some(foreground) = foreground {
        execute_observed(foreground, input, request, &operation)
    } else {
        input.execute(request, &operation)
    }
    .expect("DPI input receipt");
    assert!(
        // SAFETY: both opaque contexts were returned by USER32 on this thread.
        unsafe { AreDpiAwarenessContextsEqual(incoming, GetThreadDpiAwarenessContext()).as_bool() },
        "adapter leaked its temporary DPI context"
    );
    assert_eq!(receipt.outcome(), SequenceOutcome::Complete, "{receipt:?}");
    assert_eq!(receipt.submitted(), request.sequence().len());
    receipt
}

fn dpi_arrival_matrix(route: InputDelivery) {
    assert_eq!(
        std::env::var("MADO_PILOT_WINDOWS_DPI_INPUT").as_deref(),
        Ok("1"),
        "set current owned-desktop input authority explicitly"
    );
    let topology = std::env::var("MADO_PILOT_WINDOWS_DPI_TOPOLOGY")
        .expect("declare single-display or mixed-dpi-signed before execution");
    let declared_monitors = std::env::var("MADO_PILOT_WINDOWS_DPI_MONITORS")
        .expect("declare each monitor's physical bounds and effective DPI before execution");
    let callers = dpi_modes("MADO_PILOT_WINDOWS_DPI_CALLER");
    let targets = dpi_modes("MADO_PILOT_WINDOWS_DPI_TARGET");
    let _serial = NATIVE_MATRIX.lock().expect("native matrix serialized");
    select_process_dpi_awareness();
    let _physical_scope = DpiScope::enter(DpiMode::PerMonitorV2).expect("physical setup scope");
    let monitors = monitor_rows();
    assert!(
        declared_monitors_match(&declared_monitors, &monitors),
        "declared monitor bounds/DPI unavailable"
    );
    match topology.as_str() {
        "single-display" => assert_eq!(monitors.len(), 1, "declared topology unavailable"),
        "mixed-dpi-signed" => {
            assert!(monitors.len() > 1);
            assert!(
                monitors
                    .iter()
                    .any(|row| row.bounds.left < 0 || row.bounds.top < 0)
            );
            assert!(
                monitors
                    .iter()
                    .any(|row| (row.dpi_x, row.dpi_y) != (monitors[0].dpi_x, monitors[0].dpi_y)),
                "mixed DPI unavailable"
            );
        }
        _ => panic!("unsupported declared topology"),
    }
    // Absolute input has 65536 positions per axis; include its endpoint rounding.
    // SAFETY: setup is PMv2 and both metrics are documented scalar queries.
    let extent =
        unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN).max(GetSystemMetrics(SM_CYVIRTUALSCREEN)) };
    assert!(extent > 0);
    let physical_tolerance =
        i32::try_from((i64::from(extent) + 65534) / 65535).expect("bounded quantization");
    let mut restore = DpiDesktopRestore(Some(DesktopState::capture()));
    for target_dpi in targets {
        for (index, monitor) in monitors.iter().enumerate() {
            let mut fixture = FixtureProcess::spawn_coordinates(
                format!("dpi-{}-{index}", target_dpi.label()),
                target_dpi,
                route == InputDelivery::System,
            );
            fixture.set_geometry(
                fixture.target(),
                monitor.bounds.left + 48,
                monitor.bounds.top + 48,
                480,
                320,
            );
            fixture.wait_for("control geometry=ready", Duration::from_secs(5));
            let oracle = fixture.coordinate_oracle();
            assert!(
                oracle.physical.0 >= monitor.bounds.left
                    && oracle.physical.0 < monitor.bounds.right
                    && oracle.physical.1 >= monitor.bounds.top
                    && oracle.physical.1 < monitor.bounds.bottom
            );
            let target_span = oracle.physical_span.0.min(oracle.physical_span.1);
            let message_tolerance = if route == InputDelivery::System {
                // Convert the declared physical quantization bound through the
                // fixture's measured scale, plus one logical rounding pixel.
                i32::try_from(
                    (i64::from(physical_tolerance) * i64::from(oracle.logical_span)
                        + i64::from(target_span)
                        - 1)
                        / i64::from(target_span),
                )
                .expect("client tolerance")
                    + 1
            } else {
                1
            };
            println!(
                "dpi-oracle topology={topology} target={} monitor={index} physical={:?} client={:?} screen={:?} physical-tolerance={physical_tolerance} message-tolerance={message_tolerance}",
                target_dpi.label(),
                oracle.physical,
                oracle.client,
                oracle.screen,
            );
            let foreground = if route == InputDelivery::WindowMessage {
                Some(OwnedForeground::establish())
            } else {
                None
            };
            let provider = WindowsCaptureProvider::new(Arc::new(IdentityIssuer::new()));
            let (target, input) = open_ordinary_title(&provider, &fixture.title());
            let capture = CaptureProvider::open(
                &provider,
                target,
                &OpenRequest::new().require_format(PixelFormat::Bgra8),
                &timed(Duration::from_secs(5)),
            )
            .expect("opened frame-derived DPI oracle");
            let mut sessions = DpiSessions {
                input,
                capture,
                closed: false,
            };
            let preparation = (route == InputDelivery::System).then(|| {
                (
                    POINT {
                        x: oracle
                            .physical
                            .0
                            .checked_add(oracle.physical_span.0)
                            .expect("oracle x"),
                        y: oracle
                            .physical
                            .1
                            .checked_add(oracle.physical_span.1)
                            .expect("oracle y"),
                    },
                    (
                        oracle
                            .client
                            .0
                            .checked_add(oracle.logical_span)
                            .expect("client x"),
                        oracle
                            .client
                            .1
                            .checked_add(oracle.logical_span)
                            .expect("client y"),
                    ),
                )
            });
            for caller in &callers {
                if let Some((physical, client)) = preparation {
                    // A no-op move need not emit WM_MOUSEMOVE; settle setup before the baseline.
                    let current = DesktopState::capture();
                    assert_eq!(current.foreground, fixture.target());
                    // SAFETY: setup is PMv2; Windows validates the physical hit-test point.
                    let hit = unsafe {
                        windows::Win32::UI::WindowsAndMessaging::WindowFromPoint(physical)
                    };
                    assert_eq!(
                        hit,
                        fixture.target(),
                        "preparation must hit the owned target"
                    );
                    if current.cursor != physical {
                        let before = fixture.report(fixture.target());
                        // SAFETY: the declared preparation point hits this live owned fixture.
                        unsafe { SetPhysicalCursorPos(physical.x, physical.y) }
                            .expect("owned cursor preparation");
                        fixture.assert_coordinate(
                            "pointer-move",
                            before.target,
                            client,
                            message_tolerance,
                        );
                    }
                    let settled = DesktopState::capture();
                    assert_eq!(settled.cursor, physical);
                    assert_eq!(settled.foreground, fixture.target());
                }
                let frame = sessions
                    .capture
                    .frame(&FrameRequest::latest(), &timed(Duration::from_secs(5)))
                    .expect("source frame");
                let physical = Point::new(
                    CoordinateSpace::DesktopLogical,
                    f64::from(oracle.physical.0),
                    f64::from(oracle.physical.1),
                )
                .expect("physical oracle");
                let point = frame
                    .transform()
                    .convert_point(physical, CoordinateSpace::CapturePixels)
                    .expect("frame-derived oracle point");
                let before = fixture.report(fixture.target());
                let desktop = DesktopState::capture();
                if route == InputDelivery::System {
                    assert_eq!(
                        desktop.foreground,
                        fixture.target(),
                        "owned System target must be focused"
                    );
                }
                let request = InputRequest::new(
                    target,
                    InputSequence::new(vec![InputEvent::PointerMove(point)]).expect("move"),
                    DeliveryPlan::require(route),
                )
                .with_focus(if route == InputDelivery::System {
                    FocusPolicy::RequireFocused
                } else {
                    FocusPolicy::Preserve
                })
                .with_pointer_geometry(PointerGeometry::require_unchanged_since(frame.stamp()));
                let receipt =
                    execute_dpi_row(foreground.as_ref(), &sessions.input, &request, *caller);
                assert_eq!(receipt.selected_route(), Some(route));
                fixture.assert_coordinate(
                    "pointer-move",
                    before.target,
                    oracle.client,
                    message_tolerance,
                );
                if route == InputDelivery::System {
                    let arrived = DesktopState::capture();
                    assert_eq!(arrived.foreground, fixture.target());
                    assert_point_near(
                        (arrived.cursor.x, arrived.cursor.y),
                        oracle.physical,
                        physical_tolerance,
                    );
                } else {
                    desktop.assert_unchanged();
                }
                let after_move = fixture.report(fixture.target());
                assert_wrong_window_delta(before, after_move);
                let mut events = Vec::new();
                if route == InputDelivery::WindowMessage {
                    events.push(InputEvent::PointerMove(point));
                }
                events.extend([
                    InputEvent::PointerPress(PointerButton::Primary),
                    InputEvent::PointerRelease(PointerButton::Primary),
                    InputEvent::PointerScroll {
                        horizontal: 1,
                        vertical: 1,
                    },
                ]);
                let non_move = InputRequest::new(
                    target,
                    InputSequence::new(events).expect("bounded non-move sequence"),
                    DeliveryPlan::require(route),
                )
                .with_focus(request.focus())
                .with_pointer_geometry(PointerGeometry::require_unchanged_since(frame.stamp()));
                let desktop = DesktopState::capture();
                execute_dpi_row(foreground.as_ref(), &sessions.input, &non_move, *caller);
                fixture.assert_coordinate(
                    "button-down",
                    after_move.target,
                    oracle.client,
                    message_tolerance,
                );
                fixture.assert_coordinate(
                    "button-up",
                    after_move.target,
                    oracle.client,
                    message_tolerance,
                );
                fixture.assert_coordinate(
                    "vertical-wheel",
                    after_move.target,
                    oracle.screen,
                    message_tolerance,
                );
                fixture.assert_coordinate(
                    "horizontal-wheel",
                    after_move.target,
                    oracle.screen,
                    message_tolerance,
                );
                desktop.assert_unchanged();
                let after = fixture.report(fixture.target());
                assert_wrong_window_delta(after_move, after);
                if route == InputDelivery::WindowMessage {
                    assert_eq!((after.raw, after.state), (before.raw, before.state));
                }
                println!(
                    "dpi-arrival caller={} target={} route={route:?} monitor={index} arrival=complete",
                    caller.label(),
                    target_dpi.label()
                );
            }
            assert!(sessions.close(), "session teardown blocks acceptance");
            if let Some(foreground) = foreground {
                foreground.finish();
            }
            fixture.shutdown_input();
            println!("dpi-teardown fixture=complete");
        }
    }
    assert!(restore.restore(), "desktop restoration blocks acceptance");
}

#[test]
#[ignore = "requires explicit owned-desktop authority and declared DPI/topology; sends real input"]
fn dpi_system_arrival_matrix() {
    dpi_arrival_matrix(InputDelivery::System);
}

#[test]
#[ignore = "requires explicit owned-desktop authority and declared DPI/topology; posts real messages"]
fn dpi_window_message_arrival_matrix() {
    dpi_arrival_matrix(InputDelivery::WindowMessage);
}

#[test]
#[ignore = "requires a stationary cursor on a scaled interactive desktop; reads only"]
fn desktop_snapshots_preserve_physical_coordinates_and_caller_dpi() {
    let _serial = NATIVE_MATRIX.lock().expect("native matrix serialized");
    select_process_dpi_awareness();
    let _physical = DpiScope::enter(DpiMode::PerMonitorV2).expect("physical reference");
    let expected = DesktopState::capture();
    for caller in [DpiMode::Unaware, DpiMode::System, DpiMode::PerMonitorV2] {
        let _caller = DpiScope::enter(caller).expect("caller DPI");
        // SAFETY: this reads only the calling thread's opaque DPI context.
        let incoming = unsafe { GetThreadDpiAwarenessContext() };
        let observed = DesktopState::capture();
        assert_eq!(
            observed.cursor, expected.cursor,
            "caller-independent physical snapshot"
        );
        expected.assert_unchanged();
        // SAFETY: both opaque contexts were returned by USER32 on this thread.
        assert!(unsafe {
            AreDpiAwarenessContextsEqual(incoming, GetThreadDpiAwarenessContext()).as_bool()
        });
    }
}

// Focused, opt-in ordinary input rows. Never call the broad native matrix here.
fn require_ordinary_input() {
    assert_eq!(
        std::env::var("MADO_PILOT_WINDOWS_ORDINARY_INPUT").as_deref(),
        Ok("1"),
        "ordinary input requires explicit controller authority"
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OrdinaryInputReport([u32; 8]);

impl OrdinaryInputReport {
    const EMPTY: Self = Self([0; 8]);

    fn complete(route: InputDelivery) -> Self {
        match route {
            InputDelivery::WindowMessage => Self([4, 1, 0, 0, 3, 1, 0, 0]),
            InputDelivery::System => Self([4, 1, 3, 3, 3, 1, 0, 0]),
            _ => panic!("unsupported ordinary oracle route"),
        }
    }

    fn parse(line: &str) -> Self {
        let mut fields = line.split_ascii_whitespace();
        assert_eq!(fields.next(), Some("input-oracle"));
        let mut values = [0; 8];
        for (value, prefix) in values.iter_mut().zip([
            "keys=",
            "key-match=",
            "packet-down=",
            "packet-up=",
            "text-units=",
            "text-match=",
            "unexpected=",
            "overflow=",
        ]) {
            *value = fields
                .next()
                .and_then(|field| field.strip_prefix(prefix))
                .and_then(|field| field.parse().ok())
                .expect("ordered scalar input oracle field");
        }
        assert!(fields.next().is_none(), "input oracle has no extra fields");
        Self(values)
    }
}

impl FixtureProcess {
    fn spawn_input(label: &str, route: InputDelivery, activate: bool) -> Self {
        require_ordinary_input();
        let mode = match route {
            InputDelivery::WindowMessage => "window-message",
            InputDelivery::System => "system",
            _ => panic!("unsupported ordinary fixture route"),
        };
        let token = format!("ordinary-{label}-{}", std::process::id());
        let fixture = Self::spawn_configured(token, activate, false, None, Some(mode));
        fixture.assert_input_owner();
        fixture
    }

    fn assert_input_owner(&self) {
        ordinary_assert_window_owner(
            self.target(),
            self.child.as_ref().expect("owned fixture child").id(),
        );
    }

    fn input_report(&mut self) -> OrdinaryInputReport {
        self.assert_input_owner();
        let deadline = Instant::now() + Duration::from_secs(5);
        self.control(self.target(), CONTROL_REPORT, 2);
        let report = OrdinaryInputReport::parse(&self.wait_for(
            "input-oracle ",
            deadline.saturating_duration_since(Instant::now()),
        ));
        self.control(self.target(), CONTROL_REPORT, 3);
        let ime = self.wait_for(
            "input-oracle-ime ",
            deadline.saturating_duration_since(Instant::now()),
        );
        let (processed, packets) = ime
            .strip_prefix("input-oracle-ime processed=")
            .and_then(|value| value.split_once(" packets="))
            .expect("bounded IME observation");
        let processed = processed.parse::<u32>().expect("IME key count");
        let packets = packets.parse::<u32>().expect("IME packet count");
        assert!(packets <= processed && processed <= 3);
        println!("ordinary-ime processed={processed} packets={packets}");
        report
    }

    fn wait_input_arrivals(&mut self, route: InputDelivery) {
        // Posted reports can overtake System input. Wait for every legacy unit,
        // including packet key-ups and translated characters, before requesting it.
        let expected = match route {
            InputDelivery::WindowMessage => [2, 2, 3],
            InputDelivery::System => [5, 5, 3],
            _ => panic!("unsupported ordinary arrival route"),
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut arrived = [0; 3];
        while arrived != expected {
            assert!(
                Instant::now() < deadline,
                "ordinary input arrival timed out"
            );
            let line = self.wait_for(
                "observation role=target ",
                deadline.saturating_duration_since(Instant::now()),
            );
            let index = match line.as_str() {
                "observation role=target family=key-down units=1" => 0,
                "observation role=target family=key-up units=1" => 1,
                "observation role=target family=text-unit units=1" => 2,
                // Showing a window can synthesize passive hover without input.
                "observation role=target family=pointer-move units=1" => continue,
                _ => panic!("ordinary target received an unexpected event family"),
            };
            arrived[index] += 1;
            assert!(
                arrived[index] <= expected[index],
                "extra ordinary input unit"
            );
        }
        assert_eq!(arrived, expected);
        assert_eq!(self.input_report(), OrdinaryInputReport::complete(route));
    }

    fn shutdown_input(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::WM_CLOSE;

        self.assert_input_owner();
        let process = self.child.as_ref().expect("owned fixture child").id();
        let windows = [
            self.target(),
            self.role_window("Game"),
            self.role_window("Sibling"),
            self.role_window("Foreground"),
            self.role_window("Raw"),
            self.role_window("State"),
        ];
        for window in windows {
            ordinary_assert_window_owner(window, process);
            self.control(window, WM_CLOSE, 0);
        }
        // Closing Sibling also destroys its child. All top-level windows must
        // close naturally; Drop's bounded kill is failure containment only.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self
                .child
                .as_mut()
                .expect("owned fixture child")
                .try_wait()
                .expect("observed owned fixture exit")
            {
                assert!(status.success(), "ordinary fixture exited successfully");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "ordinary fixture shutdown timed out"
            );
            thread::sleep(Duration::from_millis(2));
        }
        while !self.reader.as_ref().expect("fixture reader").is_finished() {
            assert!(
                Instant::now() < deadline,
                "ordinary fixture reader timed out"
            );
            thread::sleep(Duration::from_millis(2));
        }
        assert!(self.reader.take().expect("fixture reader").join().is_ok());
        drop(self.child.take());
    }
}

fn ordinary_assert_window_owner(window: HWND, process: u32) {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

    let mut observed = 0;
    // SAFETY: observed is writable; Windows validates the opaque window.
    let thread = unsafe { GetWindowThreadProcessId(window, Some(&raw mut observed)) };
    assert_ne!(thread, 0, "ordinary window remains live");
    assert_eq!(
        observed, process,
        "ordinary window belongs to the retained child"
    );
}

fn ordinary_assert_focused(window: HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{
        GUITHREADINFO, GetGUIThreadInfo, GetWindowThreadProcessId,
    };

    // SAFETY: these are read-only queries of the owned opaque window.
    assert_eq!(unsafe { GetForegroundWindow() }, window);
    // SAFETY: no optional process output is requested.
    let thread = unsafe { GetWindowThreadProcessId(window, None) };
    assert_ne!(thread, 0);
    let mut info = GUITHREADINFO {
        cbSize: u32::try_from(size_of::<GUITHREADINFO>()).expect("GUI info size"),
        ..GUITHREADINFO::default()
    };
    // SAFETY: info has the documented size and is writable for this query.
    unsafe { GetGUIThreadInfo(thread, &raw mut info) }.expect("owned target focus query");
    assert_eq!(
        info.hwndFocus, window,
        "only the exact owned target has keyboard focus"
    );
}

fn ordinary_key_state() -> [bool; 12] {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_F6, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU,
        VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
    };

    [
        VK_SHIFT,
        VK_CONTROL,
        VK_MENU,
        VK_LSHIFT,
        VK_RSHIFT,
        VK_LCONTROL,
        VK_RCONTROL,
        VK_LMENU,
        VK_RMENU,
        VK_LWIN,
        VK_RWIN,
        VK_F6,
    ]
    .map(|key| {
        // SAFETY: fixed modifier/F6 virtual keys; only the current down bit is used.
        (unsafe { GetAsyncKeyState(i32::from(key.0)) }) < 0
    })
}

fn ordinary_sequence() -> InputSequence {
    InputSequence::new(vec![
        InputEvent::KeyPress(Key::Modifier(Modifier::Shift)),
        InputEvent::KeyPress(Key::Function(6)),
        InputEvent::KeyRelease(Key::Function(6)),
        InputEvent::KeyRelease(Key::Modifier(Modifier::Shift)),
        InputEvent::Text("A\u{1f642}".to_owned()),
    ])
    .expect("bounded, fully paired ordinary input")
}

struct OrdinaryInputSession {
    target: TargetId,
    input: Arc<dyn InputController>,
    close_attempted: bool,
}

impl OrdinaryInputSession {
    fn open(title: &str, window: HWND, process: u32) -> Self {
        ordinary_assert_window_owner(window, process);
        assert_eq!(find_window(title), window, "exact owned fixture title");
        let provider = WindowsCaptureProvider::new(Arc::new(IdentityIssuer::new()));
        let found = provider
            .discover_matching(
                &DiscoveryRequest::new()
                    .with_kind(TargetKind::Window)
                    .with_name_containing(title),
                &timed(Duration::from_secs(5)),
            )
            .expect("ordinary input discovery");
        let mut matches = found.iter().filter(|candidate| candidate.name() == title);
        let target = matches.next().expect("exact ordinary input target").id();
        assert!(matches.next().is_none(), "ordinary input target is unique");
        let request = InputOpenRequest::new()
            .requiring(InputOperationKind::Keyboard, InputDelivery::WindowMessage)
            .requiring(InputOperationKind::Text, InputDelivery::WindowMessage)
            .requiring(InputOperationKind::Keyboard, InputDelivery::System)
            .requiring(InputOperationKind::Text, InputDelivery::System);
        let input =
            InputProvider::open(&provider, target, &request, &timed(Duration::from_secs(5)))
                .expect("opened ordinary keyboard/text controller");
        let session = Self {
            target,
            input,
            close_attempted: false,
        };
        ordinary_assert_window_owner(window, process);
        assert_eq!(find_window(title), window);
        session
    }

    fn for_fixture(fixture: &FixtureProcess) -> Self {
        Self::open(
            &fixture.title(),
            fixture.target(),
            fixture.child.as_ref().expect("owned fixture child").id(),
        )
    }

    fn execute(
        &self,
        delivery: DeliveryPlan,
        focus: FocusPolicy,
    ) -> mado_pilot_input::InputReceipt {
        self.input
            .execute(
                &InputRequest::new(self.target, ordinary_sequence(), delivery).with_focus(focus),
                &timed(Duration::from_secs(5)),
            )
            .expect("ordinary input receipt")
    }

    fn close(&mut self) -> bool {
        if std::mem::replace(&mut self.close_attempted, true) {
            return true;
        }
        self.input.close(&timed(Duration::from_secs(2))).is_ok()
    }
}

impl Drop for OrdinaryInputSession {
    fn drop(&mut self) {
        if !self.close() {
            eprintln!("ordinary-teardown input=0");
        }
    }
}

fn assert_ordinary_complete(receipt: &mado_pilot_input::InputReceipt, route: InputDelivery) {
    assert_eq!(receipt.outcome(), SequenceOutcome::Complete);
    assert_eq!(receipt.selected_route(), Some(route));
    assert_eq!(receipt.submitted(), 5);
    assert_eq!(receipt.last_submitted(), Some(4));
    assert_eq!(receipt.fault(), None);
    assert!(!receipt.partial_native_effect());
    assert!(!receipt.used_fallback());
    assert_eq!(receipt.attempts().len(), 1);
    assert_eq!(receipt.attempts()[0].route(), route);
    assert_eq!(receipt.cleanup(), CleanupState::NotNeeded);
    assert_eq!((receipt.cleanup_released(), receipt.cleanup_owed()), (0, 0));
    assert_eq!(
        receipt.evidence(),
        Some(match route {
            InputDelivery::WindowMessage => SubmissionEvidence::TargetQueueAdmission,
            InputDelivery::System => SubmissionEvidence::SystemInputAdmission,
            _ => panic!("unsupported ordinary receipt route"),
        })
    );
}

fn assert_ordinary_refusal(
    receipt: &mado_pilot_input::InputReceipt,
    routes: &[InputDelivery],
    fault: InputFault,
) {
    assert_eq!(receipt.outcome(), SequenceOutcome::Unexecuted);
    assert_eq!(receipt.selected_route(), None);
    assert_eq!(receipt.submitted(), 0);
    assert_eq!(receipt.last_submitted(), None);
    assert_eq!(receipt.evidence(), None);
    assert_eq!(receipt.fault(), Some(fault));
    assert!(!receipt.possible_native_effect());
    assert!(!receipt.used_fallback());
    assert_eq!(receipt.cleanup(), CleanupState::NotNeeded);
    assert_eq!((receipt.cleanup_released(), receipt.cleanup_owed()), (0, 0));
    assert_eq!(receipt.attempts().len(), routes.len());
    for (attempt, route) in receipt.attempts().iter().zip(routes) {
        assert_eq!(attempt.route(), *route);
        assert_eq!(attempt.outcome(), SequenceOutcome::Unexecuted);
        assert_eq!(attempt.submitted(), 0);
        assert_eq!(attempt.fault(), Some(fault));
        assert!(!attempt.possible_native_effect());
    }
}

#[test]
#[ignore = "requires explicit ordinary-input authority and an unlocked controlled desktop"]
fn ordinary_window_message_input_acceptance() {
    require_ordinary_input();
    let _serial = NATIVE_MATRIX.lock().expect("ordinary input serialized");
    let dpi = DpiScope::enter(DpiMode::PerMonitorV2).expect("ordinary caller DPI scope");
    let mut restore = DpiDesktopRestore(Some(DesktopState::capture()));
    let keys = ordinary_key_state();
    assert_eq!(keys, [false; 12], "modifiers and F6 must start released");
    let mut foreground =
        FixtureProcess::spawn_input("wm-foreground", InputDelivery::WindowMessage, true);
    ordinary_assert_focused(foreground.target());
    let mut fixture = FixtureProcess::spawn_input("wm-target", InputDelivery::WindowMessage, false);
    let desktop = DesktopState::capture();
    assert_eq!(desktop.foreground, foreground.target());
    assert_ne!(desktop.foreground, fixture.target());
    assert_eq!(fixture.input_report(), OrdinaryInputReport::EMPTY);
    assert_eq!(foreground.input_report(), OrdinaryInputReport::EMPTY);
    let before = fixture.report(fixture.target());
    let foreground_before = foreground.report(foreground.target());
    let mut session = OrdinaryInputSession::for_fixture(&fixture);
    desktop.assert_unchanged();
    assert_eq!(ordinary_key_state(), keys);
    let receipt = session.execute(
        DeliveryPlan::ordered(vec![InputDelivery::WindowMessage, InputDelivery::System])
            .expect("explicit WM-first plan"),
        FocusPolicy::Preserve,
    );
    assert_ordinary_complete(&receipt, InputDelivery::WindowMessage);
    fixture.wait_input_arrivals(InputDelivery::WindowMessage);
    assert!(
        session.close(),
        "ordinary controller close blocks acceptance"
    );
    assert_eq!(
        fixture.input_report(),
        OrdinaryInputReport::complete(InputDelivery::WindowMessage)
    );
    let after = fixture.report(fixture.target());
    assert_wrong_window_delta(before, after);
    assert_eq!((after.raw, after.state), (before.raw, before.state));
    assert_eq!(foreground.input_report(), OrdinaryInputReport::EMPTY);
    assert_ordinary_observers_unchanged(foreground_before, foreground.report(foreground.target()));
    desktop.assert_unchanged();
    assert_eq!(ordinary_key_state(), keys);
    fixture.shutdown_input();
    foreground.shutdown_input();
    assert!(
        restore.restore(),
        "ordinary desktop restoration blocks acceptance"
    );
    assert_eq!(ordinary_key_state(), [false; 12]);
    drop(dpi);
    println!("ordinary-acceptance route=window-message complete=1");
}

#[test]
#[ignore = "requires explicit ordinary-input authority and an unlocked controlled desktop"]
fn ordinary_system_input_acceptance() {
    require_ordinary_input();
    let _serial = NATIVE_MATRIX.lock().expect("ordinary input serialized");
    let dpi = DpiScope::enter(DpiMode::PerMonitorV2).expect("ordinary caller DPI scope");
    let mut restore = DpiDesktopRestore(Some(DesktopState::capture()));
    assert_eq!(
        ordinary_key_state(),
        [false; 12],
        "modifiers and F6 must start released"
    );
    let mut fixture = FixtureProcess::spawn_input("system-target", InputDelivery::System, true);
    ordinary_assert_focused(fixture.target());
    assert_eq!(fixture.input_report(), OrdinaryInputReport::EMPTY);
    let before = fixture.report(fixture.target());
    let desktop = DesktopState::capture();
    let mut session = OrdinaryInputSession::for_fixture(&fixture);
    ordinary_assert_focused(fixture.target());
    assert_eq!(ordinary_key_state(), [false; 12]);
    let receipt = session.execute(
        DeliveryPlan::require(InputDelivery::System),
        FocusPolicy::RequireFocused,
    );
    assert_ordinary_complete(&receipt, InputDelivery::System);
    fixture.wait_input_arrivals(InputDelivery::System);
    assert!(
        session.close(),
        "ordinary controller close blocks acceptance"
    );
    assert_eq!(
        fixture.input_report(),
        OrdinaryInputReport::complete(InputDelivery::System)
    );
    let after = fixture.report(fixture.target());
    // Raw and asynchronous-state observers in this fixture may see System input.
    // They must not receive legacy-window delivery intended for the target.
    assert_wrong_window_delta(before, after);
    ordinary_assert_focused(fixture.target());
    desktop.assert_unchanged();
    assert_eq!(
        ordinary_key_state(),
        [false; 12],
        "paired System input leaves no held keys"
    );
    fixture.shutdown_input();
    assert!(
        restore.restore(),
        "ordinary desktop restoration blocks acceptance"
    );
    assert_eq!(ordinary_key_state(), [false; 12]);
    drop(dpi);
    println!("ordinary-acceptance route=system complete=1");
}

#[test]
#[ignore = "requires explicit ordinary-input authority and an unlocked controlled desktop"]
fn ordinary_focus_fallback_acceptance() {
    require_ordinary_input();
    let _serial = NATIVE_MATRIX.lock().expect("ordinary input serialized");
    let dpi = DpiScope::enter(DpiMode::PerMonitorV2).expect("ordinary caller DPI scope");
    let mut restore = DpiDesktopRestore(Some(DesktopState::capture()));
    let keys = ordinary_key_state();
    assert_eq!(keys, [false; 12], "modifiers and F6 must start released");
    let mut foreground =
        FixtureProcess::spawn_input("focus-foreground", InputDelivery::WindowMessage, true);
    let mut fixture =
        FixtureProcess::spawn_input("focus-target", InputDelivery::WindowMessage, false);
    ordinary_assert_focused(foreground.target());
    let desktop = DesktopState::capture();
    assert_ne!(desktop.foreground, fixture.target());
    assert_eq!(fixture.input_report(), OrdinaryInputReport::EMPTY);
    assert_eq!(foreground.input_report(), OrdinaryInputReport::EMPTY);
    let before = fixture.report(fixture.target());
    let foreground_before = foreground.report(foreground.target());
    let mut session = OrdinaryInputSession::for_fixture(&fixture);
    for routes in [
        &[InputDelivery::System][..],
        &[InputDelivery::System, InputDelivery::WindowMessage][..],
    ] {
        desktop.assert_unchanged();
        let receipt = session.execute(
            DeliveryPlan::ordered(routes.to_vec()).expect("explicit focus-bound plan"),
            FocusPolicy::RequireFocused,
        );
        // Native preflight may visit both routes. RequireFocused applies to
        // both, so fallback cannot turn the refusal into background delivery.
        assert_ordinary_refusal(&receipt, routes, InputFault::FocusRequired);
        assert_eq!(fixture.input_report(), OrdinaryInputReport::EMPTY);
        assert_ordinary_observers_unchanged(before, fixture.report(fixture.target()));
        assert_eq!(foreground.input_report(), OrdinaryInputReport::EMPTY);
        assert_ordinary_observers_unchanged(
            foreground_before,
            foreground.report(foreground.target()),
        );
        desktop.assert_unchanged();
        assert_eq!(ordinary_key_state(), keys);
    }
    assert!(
        session.close(),
        "ordinary controller close blocks acceptance"
    );
    thread::sleep(Duration::from_millis(100));
    assert_eq!(fixture.input_report(), OrdinaryInputReport::EMPTY);
    assert_ordinary_observers_unchanged(before, fixture.report(fixture.target()));
    assert_eq!(foreground.input_report(), OrdinaryInputReport::EMPTY);
    assert_ordinary_observers_unchanged(foreground_before, foreground.report(foreground.target()));
    desktop.assert_unchanged();
    assert_eq!(ordinary_key_state(), keys);
    fixture.shutdown_input();
    foreground.shutdown_input();
    assert!(
        restore.restore(),
        "ordinary desktop restoration blocks acceptance"
    );
    assert_eq!(ordinary_key_state(), [false; 12]);
    drop(dpi);
    println!("ordinary-acceptance route=focus complete=1");
}

// The helper never creates a window or accepts a target from ambient discovery.
// Its private stdin grant pins this child, its parent, and the parent's live fixture.
mod ordinary_uipi {
    use super::*;
    use std::ffi::OsString;
    use std::io::{Read, Write};
    use std::mem::size_of_val;
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;
    use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, WAIT_TIMEOUT};
    use windows::Win32::Security::{
        CreateWellKnownSid, GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation,
        IsValidSid, PSID, SID_AND_ATTRIBUTES, SetTokenInformation, TOKEN_ADJUST_DEFAULT,
        TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenIntegrityLevel, TokenUIAccess, WinLowLabelSid,
    };
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetProcessTimes, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, QueryFullProcessImageNameW,
        WaitForSingleObject,
    };
    use windows::core::PWSTR;

    const HELPER_AUTHORITY: &str = "MADO_PILOT_WINDOWS_UIPI_HELPER";
    const GRANT_MAGIC: u64 = u64::from_le_bytes(*b"MPUIPI01");
    const HELPER_COMPLETE: &str = "ordinary-uipi-helper refused=1 closed=1";

    fn unavailable() -> ! {
        eprintln!("ordinary-prerequisite-unavailable");
        panic!("ordinary UIPI prerequisite is unavailable");
    }

    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: only successfully opened real handles enter this owner.
            let _closed = unsafe { CloseHandle(self.0) };
        }
    }

    fn process(pid: u32) -> Option<Handle> {
        // SAFETY: pid is from the parent's private grant or retained child.
        unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                false,
                pid,
            )
        }
        .ok()
        .map(Handle)
    }

    fn birth(process: HANDLE) -> Option<u64> {
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        // SAFETY: the retained process handle and all four complete outputs are valid.
        unsafe {
            GetProcessTimes(
                process,
                &raw mut creation,
                &raw mut exit,
                &raw mut kernel,
                &raw mut user,
            )
        }
        .ok()?;
        Some((u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime))
    }

    fn assert_live(process: &Handle, expected_birth: u64) {
        assert_eq!(
            birth(process.0),
            Some(expected_birth),
            "owned process identity changed"
        );
        // SAFETY: process was opened with SYNCHRONIZE and remains retained.
        assert_eq!(unsafe { WaitForSingleObject(process.0, 0) }, WAIT_TIMEOUT);
    }

    fn image_matches(process: HANDLE, expected: &Path) -> bool {
        let mut image = [0u16; 32_768];
        let mut length = u32::try_from(image.len()).expect("bounded process image");
        // SAFETY: image is writable for length UTF-16 units; the process is retained.
        if unsafe {
            QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                PWSTR(image.as_mut_ptr()),
                &raw mut length,
            )
        }
        .is_err()
        {
            return false;
        }
        let Ok(length) = usize::try_from(length) else {
            return false;
        };
        let Some(image) = image.get(..length) else {
            return false;
        };
        let actual = std::path::PathBuf::from(OsString::from_wide(image)).canonicalize();
        let expected = expected.canonicalize();
        matches!((actual, expected), (Ok(actual), Ok(expected)) if actual == expected)
    }

    fn token(process: HANDLE, adjust: bool) -> Option<Handle> {
        let mut token = HANDLE::default();
        let access = if adjust {
            TOKEN_QUERY | TOKEN_ADJUST_DEFAULT
        } else {
            TOKEN_QUERY
        };
        // SAFETY: process is retained or the current-process pseudo handle; output is writable.
        unsafe { OpenProcessToken(process, access, &raw mut token) }.ok()?;
        Some(Handle(token))
    }

    fn integrity(token: &Handle) -> Option<(u32, u32)> {
        // An integrity SID has a small fixed representation. Refuse an oversized
        // result rather than allocating from an unchecked native length.
        let mut storage = [0usize; 128];
        let mut length = 0;
        // SAFETY: storage is aligned for TOKEN_MANDATORY_LABEL and writable for its full size.
        unsafe {
            GetTokenInformation(
                token.0,
                TokenIntegrityLevel,
                Some(storage.as_mut_ptr().cast()),
                u32::try_from(size_of_val(&storage)).ok()?,
                &raw mut length,
            )
        }
        .ok()?;
        if usize::try_from(length).ok()? < size_of::<TOKEN_MANDATORY_LABEL>()
            || usize::try_from(length).ok()? > size_of_val(&storage)
        {
            return None;
        }
        // SAFETY: the successful query initialized an aligned mandatory label and its SID.
        let label = unsafe { &*storage.as_ptr().cast::<TOKEN_MANDATORY_LABEL>() };
        // SAFETY: this SID is owned by the live, initialized token-information buffer.
        if !unsafe { IsValidSid(label.Label.Sid) }.as_bool() {
            return None;
        }
        // SAFETY: the validated SID contains its sub-authority count.
        let count = unsafe { *GetSidSubAuthorityCount(label.Label.Sid) };
        if count == 0 {
            return None;
        }
        // SAFETY: count - 1 is inside the validated SID and storage is still live.
        let level = unsafe { *GetSidSubAuthority(label.Label.Sid, u32::from(count - 1)) };
        Some((level, label.Label.Attributes))
    }

    fn lower_sender() -> Option<u32> {
        // SAFETY: this pseudo handle refers only to the independently gated helper.
        let current = unsafe { GetCurrentProcess() };
        let token = token(current, true)?;
        let (original, attributes) = integrity(&token)?;
        let mut ui_access = 0u32;
        let mut length = 0;
        // SAFETY: TokenUIAccess returns one DWORD into the complete writable output.
        unsafe {
            GetTokenInformation(
                token.0,
                TokenUIAccess,
                Some((&raw mut ui_access).cast()),
                u32::try_from(size_of::<u32>()).ok()?,
                &raw mut length,
            )
        }
        .ok()?;
        if ui_access != 0 || length != u32::try_from(size_of::<u32>()).ok()? {
            return None;
        }
        // Keep the label and SID in one aligned live allocation for SetTokenInformation.
        let mut storage = [0usize; 32];
        let label_bytes = size_of::<TOKEN_MANDATORY_LABEL>();
        // SAFETY: the label prefix is smaller than storage; the remaining bytes hold the SID.
        let sid = PSID(unsafe { storage.as_mut_ptr().cast::<u8>().add(label_bytes) }.cast());
        let mut sid_bytes = u32::try_from(size_of_val(&storage) - label_bytes).ok()?;
        // SAFETY: sid points to sid_bytes writable bytes in aligned storage.
        unsafe { CreateWellKnownSid(WinLowLabelSid, None, Some(sid), &raw mut sid_bytes) }.ok()?;
        // SAFETY: CreateWellKnownSid initialized the low-integrity SID, with one sub-authority.
        let low = unsafe { *GetSidSubAuthority(sid, 0) };
        if original <= low {
            return None;
        }
        let label = TOKEN_MANDATORY_LABEL {
            Label: SID_AND_ATTRIBUTES {
                Sid: sid,
                Attributes: attributes,
            },
        };
        // SAFETY: the aligned prefix is large enough; its SID points into the retained suffix.
        unsafe {
            storage
                .as_mut_ptr()
                .cast::<TOKEN_MANDATORY_LABEL>()
                .write(label)
        };
        let bytes = u32::try_from(label_bytes).ok()?.checked_add(sid_bytes)?;
        // SAFETY: only this helper's own primary token is lowered; storage covers bytes.
        unsafe {
            SetTokenInformation(token.0, TokenIntegrityLevel, storage.as_ptr().cast(), bytes)
        }
        .ok()?;
        if integrity(&token)?.0 != low {
            return None;
        }
        Some(low)
    }

    struct SenderProcess {
        child: Child,
        output: Option<thread::JoinHandle<std::io::Result<Vec<u8>>>>,
        exited: bool,
    }

    impl Drop for SenderProcess {
        fn drop(&mut self) {
            let deadline = Instant::now() + Duration::from_secs(2);
            if !self.exited {
                let _killed = self.child.kill();
                loop {
                    match self.child.try_wait() {
                        Ok(Some(_)) => {
                            self.exited = true;
                            break;
                        }
                        Ok(None) if Instant::now() < deadline => {
                            thread::sleep(Duration::from_millis(2))
                        }
                        _ => break,
                    }
                }
            }
            if let Some(reader) = self.output.take() {
                while !reader.is_finished() && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(2));
                }
                if reader.is_finished() {
                    let _joined = reader.join();
                } else {
                    eprintln!("ordinary-teardown helper-reader=0");
                }
            }
            if !self.exited {
                eprintln!("ordinary-teardown helper=0");
            }
        }
    }

    pub(super) fn run_sender(fixture: &FixtureProcess) {
        assert!(
            std::env::var_os(HELPER_AUTHORITY).is_none(),
            "parent is not a sender helper"
        );
        fixture.assert_input_owner();
        let target = fixture.child.as_ref().expect("owned UIPI target child");
        let target_birth = birth(HANDLE(target.as_raw_handle())).unwrap_or_else(|| unavailable());
        // SAFETY: the parent's pseudo handle is queried, never owned or closed.
        let parent_birth = birth(unsafe { GetCurrentProcess() }).unwrap_or_else(|| unavailable());
        let executable = std::env::current_exe().expect("current test executable");
        let child = Command::new(&executable)
            .args([
                "--exact",
                "ordinary_uipi_sender_helper",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("MADO_PILOT_WINDOWS_ORDINARY_INPUT", "1")
            .env(HELPER_AUTHORITY, "1")
            .env("RUST_BACKTRACE", "0")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap_or_else(|_| unavailable());
        let mut sender = SenderProcess {
            child,
            output: None,
            exited: false,
        };
        let output = sender.child.stdout.take().expect("private sender output");
        sender.output = Some(thread::spawn(move || {
            let mut bytes = Vec::with_capacity(4097);
            output.take(4097).read_to_end(&mut bytes)?;
            Ok(bytes)
        }));
        let grant = [
            GRANT_MAGIC,
            u64::from(sender.child.id()),
            u64::from(std::process::id()),
            parent_birth,
            u64::from(target.id()),
            target_birth,
            u64::try_from(fixture.target().0.addr()).expect("64-bit target handle"),
        ];
        {
            let mut input = sender
                .child
                .stdin
                .take()
                .expect("private sender authority pipe");
            for field in grant {
                input
                    .write_all(&field.to_le_bytes())
                    .expect("bounded sender authority handoff");
            }
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        let status = loop {
            if let Some(status) = sender.child.try_wait().expect("observed sender process") {
                sender.exited = true;
                break status;
            }
            assert!(Instant::now() < deadline, "ordinary UIPI sender timed out");
            thread::sleep(Duration::from_millis(2));
        };
        while !sender.output.as_ref().expect("sender reader").is_finished() {
            assert!(Instant::now() < deadline, "ordinary UIPI output timed out");
            thread::sleep(Duration::from_millis(2));
        }
        let bytes = sender
            .output
            .take()
            .expect("sender reader")
            .join()
            .expect("sender reader joined")
            .expect("sender output read");
        assert!(bytes.len() <= 4096, "bounded UIPI sender output");
        let output = std::str::from_utf8(&bytes).expect("scalar sender output");
        assert!(
            status.success(),
            "ordinary UIPI helper must pass, never skip"
        );
        // libtest may prefix a nocapture line with the selector. Match the one
        // fixed suffix, not any payload or a partial success token.
        assert_eq!(
            output
                .lines()
                .filter(|line| line.ends_with(HELPER_COMPLETE))
                .count(),
            1
        );
        fixture.assert_input_owner();
    }

    pub(super) fn sender() {
        require_ordinary_input();
        assert_eq!(
            std::env::var(HELPER_AUTHORITY).as_deref(),
            Ok("1"),
            "explicit helper authority"
        );
        let (send, receive) = mpsc::sync_channel(1);
        let reader = thread::spawn(move || {
            let mut bytes = [0u8; 56];
            let mut input = std::io::stdin().lock();
            let result = input.read_exact(&mut bytes).and_then(|()| {
                let mut extra = [0u8; 1];
                if input.read(&mut extra)? != 0 {
                    return Err(std::io::Error::other("extra sender authority bytes"));
                }
                Ok(bytes)
            });
            let _sent = send.send(result);
        });
        let bytes = receive
            .recv_timeout(Duration::from_secs(5))
            .expect("bounded private parent authority")
            .expect("complete parent authority");
        reader.join().expect("authority reader joined");
        let mut grant = [0u64; 7];
        for (field, bytes) in grant.iter_mut().zip(bytes.chunks_exact(8)) {
            *field = u64::from_le_bytes(bytes.try_into().expect("fixed grant field"));
        }
        let [
            magic,
            child,
            parent,
            parent_birth,
            target,
            target_birth,
            window,
        ] = grant;
        assert_eq!(magic, GRANT_MAGIC, "parent authority version");
        assert_eq!(
            child,
            u64::from(std::process::id()),
            "grant belongs to this helper"
        );
        let parent_id = u32::try_from(parent).expect("parent PID");
        let target_id = u32::try_from(target).expect("target PID");
        assert_ne!(parent_id, target_id);
        assert_ne!(parent_id, std::process::id());
        assert_ne!(target_id, std::process::id());
        let parent = process(parent_id).unwrap_or_else(|| unavailable());
        let target = process(target_id).unwrap_or_else(|| unavailable());
        assert_live(&parent, parent_birth);
        assert_live(&target, target_birth);
        let executable = std::env::current_exe().expect("current helper image");
        assert!(
            image_matches(parent.0, &executable),
            "parent is this test executable"
        );
        assert!(
            image_matches(
                target.0,
                Path::new(env!(
                    "CARGO_BIN_EXE_mado-pilot-windows-window-message-fixture"
                ))
            ),
            "target is the exact fixture executable"
        );
        let window = HWND(std::ptr::with_exposed_provenance_mut(
            usize::try_from(window).expect("target handle fits pointer"),
        ));
        let title = ordinary_fixture_title(&format!("ordinary-uipi-target-{parent_id}"));
        ordinary_assert_window_owner(window, target_id);
        assert_eq!(find_window(&title), window);
        ordinary_assert_focused(window);
        let dpi = DpiScope::enter(DpiMode::PerMonitorV2).expect("sender thread DPI scope");
        let desktop = DesktopState::capture();
        let keys = ordinary_key_state();
        let mut session = OrdinaryInputSession::open(&title, window, target_id);
        let low = lower_sender().unwrap_or_else(|| unavailable());
        // Reopen only the exact owned target after lowering, just as the
        // production preflight does. Unavailable inspection must not admit input.
        let lower_target = process(target_id).unwrap_or_else(|| unavailable());
        assert_live(&lower_target, target_birth);
        let target_token = token(lower_target.0, false).unwrap_or_else(|| unavailable());
        let target_level = integrity(&target_token).unwrap_or_else(|| unavailable()).0;
        if target_level <= low {
            unavailable();
        }
        assert_live(&parent, parent_birth);
        assert_live(&target, target_birth);
        ordinary_assert_window_owner(window, target_id);
        assert_eq!(find_window(&title), window);
        ordinary_assert_focused(window);
        let routes = [InputDelivery::WindowMessage, InputDelivery::System];
        let receipt = session.execute(
            DeliveryPlan::ordered(routes.to_vec()).expect("explicit privilege-bound plan"),
            FocusPolicy::RequireFocused,
        );
        assert_ordinary_refusal(&receipt, &routes, InputFault::PolicyRefused);
        assert!(session.close(), "sender controller close blocks acceptance");
        desktop.assert_unchanged();
        assert_eq!(ordinary_key_state(), keys);
        drop(dpi);
        println!("{HELPER_COMPLETE}");
    }
}

#[test]
#[ignore = "requires explicit ordinary-input authority and a private parent-owned UIPI grant"]
fn ordinary_uipi_sender_helper() {
    ordinary_uipi::sender();
}

#[test]
#[ignore = "requires explicit ordinary-input authority and an unlocked controlled desktop"]
fn ordinary_uipi_acceptance() {
    require_ordinary_input();
    let _serial = NATIVE_MATRIX.lock().expect("ordinary input serialized");
    let dpi = DpiScope::enter(DpiMode::PerMonitorV2).expect("ordinary caller DPI scope");
    let mut restore = DpiDesktopRestore(Some(DesktopState::capture()));
    let keys = ordinary_key_state();
    assert_eq!(keys, [false; 12], "modifiers and F6 must start released");
    let mut fixture =
        FixtureProcess::spawn_input("uipi-target", InputDelivery::WindowMessage, true);
    ordinary_assert_focused(fixture.target());
    let desktop = DesktopState::capture();
    assert_eq!(fixture.input_report(), OrdinaryInputReport::EMPTY);
    let before = fixture.report(fixture.target());
    ordinary_uipi::run_sender(&fixture);
    thread::sleep(Duration::from_millis(100));
    assert_eq!(fixture.input_report(), OrdinaryInputReport::EMPTY);
    assert_ordinary_observers_unchanged(before, fixture.report(fixture.target()));
    ordinary_assert_focused(fixture.target());
    desktop.assert_unchanged();
    assert_eq!(ordinary_key_state(), keys);
    fixture.shutdown_input();
    assert!(
        restore.restore(),
        "ordinary desktop restoration blocks acceptance"
    );
    assert_eq!(ordinary_key_state(), [false; 12]);
    drop(dpi);
    println!("ordinary-acceptance route=uipi complete=1");
}

fn assert_ordinary_observers_unchanged(before: ObservationReport, after: ObservationReport) {
    // The exact keyboard/text oracle, not the target's aggregate legacy count,
    // distinguishes input from passive WM_MOUSEMOVE generated by window setup.
    assert_wrong_window_delta(before, after);
    assert_eq!((after.raw, after.state), (before.raw, before.state));
}
