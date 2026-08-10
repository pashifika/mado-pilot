#![cfg_attr(not(windows), allow(missing_docs))]
#![cfg(windows)]
//! Native ordinary-window delivery, authority, and isolation coverage.

use std::collections::VecDeque;
use std::mem::size_of;
use std::process::{Child, Command, Stdio};
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
    CleanupState, DeliveryPlan, FocusPolicy, InputController, InputEvent, InputFault,
    InputOpenRequest, InputProvider, InputRequest, InputSequence, Key, Modifier, PointerButton,
    PointerGeometry, SequenceOutcome,
};
use mado_pilot_platform_windows::WindowsCaptureProvider;
use mado_pilot_platform_windows::fixture_protocol::{
    CONTROL_BLOCK_QUEUE, CONTROL_DESTROY_TARGET, CONTROL_DUPLICATE_METADATA,
    CONTROL_REPARENT_TARGET, CONTROL_REPORT, CONTROL_REUSE_STRESS, CONTROL_SET_GEOMETRY,
    ORDINARY_CLASS_NAME, ordinary_fixture_title,
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
    FindWindowW, GetCursorPos, GetForegroundWindow, GetWindowRect, PostMessageW, WM_MOUSEMOVE,
    WM_NULL,
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
        let token = token.into();
        let mut child = Command::new(env!(
            "CARGO_BIN_EXE_mado-pilot-windows-window-message-fixture"
        ))
        .arg(format!("--title-token={token}"))
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

#[test]
fn ordinary_window_message_native_matrix() {
    let _serial = NATIVE_MATRIX.lock().expect("native matrix serialized");
    // SAFETY: DPI awareness is fixed before this test calls USER32.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }
        .expect("selected per-monitor-v2 DPI awareness");

    let mut delivery = FixtureProcess::spawn("delivery");
    let (target, controller) = open_ordinary(&delivery);
    let desktop = DesktopState::capture();
    assert_ne!(
        desktop.foreground,
        delivery.target(),
        "the no-activation fixture unexpectedly became foreground"
    );
    let request = InputRequest::new(
        target,
        complete_sequence(),
        DeliveryPlan::require(InputDelivery::WindowMessage),
    )
    .with_focus(FocusPolicy::Preserve);
    let receipt = controller
        .execute(&request, &timed(Duration::from_secs(10)))
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
    let receipt = controller
        .execute(&duplicate, &timed(Duration::from_secs(5)))
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

    let mut reparented = FixtureProcess::spawn("reparent");
    let (target, controller) = open_ordinary(&reparented);
    let retained = reparented.target();
    reparented.control(retained, CONTROL_REPARENT_TARGET, 0);
    reparented.wait_for("control reparent=ready", Duration::from_secs(5));
    assert_unexecuted_target_loss(&controller, one_key_request(target));
    assert_wrong_targets_clear(reparented.report(retained));
    drop(reparented);

    let mut replaced = FixtureProcess::spawn("replacement");
    let (target, controller) = open_ordinary(&replaced);
    let retained = replaced.target();
    replaced.control(retained, CONTROL_REUSE_STRESS, 4_096);
    let stress = replaced.wait_for("control reuse-stress=ready", Duration::from_secs(15));
    assert_unexecuted_target_loss(&controller, one_key_request(target));
    let replacement = replaced.target();
    assert_wrong_targets_clear(replaced.report(replacement));
    println!("{stress}");
    drop(replaced);

    let mut lost = FixtureProcess::spawn("target-loss");
    let (target, controller) = open_ordinary(&lost);
    let retained = lost.target();
    lost.control(retained, CONTROL_DESTROY_TARGET, 0);
    lost.wait_for("control target-loss=ready", Duration::from_secs(5));
    assert_unexecuted_target_loss(&controller, one_key_request(target));
    drop(lost);

    let restart_token = "owner-restart";
    let first_owner = FixtureProcess::spawn(restart_token);
    let (target, controller) = open_ordinary(&first_owner);
    drop(first_owner);
    let mut restarted_owner = FixtureProcess::spawn(restart_token);
    assert_unexecuted_target_loss(&controller, one_key_request(target));
    assert_wrong_targets_clear(restarted_owner.report(restarted_owner.target()));
    drop(restarted_owner);

    let desktop = DesktopState::capture();
    consumer_compatibility_rows();
    terminal_and_cleanup_rows();
    queue_pressure_rows();
    late_effect_row();
    desktop.assert_unchanged();
    topology_geometry_rows();
    controlled_unrelated_activity_row();
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

fn assert_unexecuted_target_loss(controller: &Arc<dyn InputController>, request: InputRequest) {
    let receipt = controller
        .execute(&request, &timed(Duration::from_secs(5)))
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

fn consumer_compatibility_rows() {
    let mut game = FixtureProcess::spawn("game-consumer");
    let (target, controller) = open_role(&game, "Game");
    let receipt = controller
        .execute(&key_pair_request(target), &timed(Duration::from_secs(5)))
        .expect("game-like legacy receipt");
    assert_complete_queue_receipt(&receipt, 2);
    let report = game.report(game.role_window("Game"));
    assert_eq!(
        report.game, 2,
        "game-like window observed both legacy units"
    );
    assert_eq!(report.raw, 0);
    assert_eq!(report.state, 0);
    assert_eq!(report.target, 0);
    controller
        .close(&timed(Duration::from_secs(5)))
        .expect("closed game row");
    drop(game);

    let mut raw = FixtureProcess::spawn("raw-consumer");
    let (target, controller) = open_role(&raw, "Raw");
    let receipt = controller
        .execute(&key_pair_request(target), &timed(Duration::from_secs(5)))
        .expect("Raw Input consumer receipt");
    assert_complete_queue_receipt(&receipt, 2);
    let report = raw.report(raw.role_window("Raw"));
    assert_eq!(
        report.raw_legacy, 2,
        "the raw window received legacy messages"
    );
    assert_eq!(report.raw, 0, "legacy messages synthesized no Raw Input");
    assert_eq!(report.state, 0);
    controller
        .close(&timed(Duration::from_secs(5)))
        .expect("closed raw row");
    drop(raw);

    let mut state = FixtureProcess::spawn("state-consumer");
    let (target, controller) = open_role(&state, "State");
    let before = state.report(state.role_window("State"));
    let receipt = controller
        .execute(&key_pair_request(target), &timed(Duration::from_secs(5)))
        .expect("state-polling consumer receipt");
    assert_complete_queue_receipt(&receipt, 2);
    let report = state.report(state.role_window("State"));
    assert_eq!(
        report.state_legacy,
        before.state_legacy + 2,
        "the state-polling window received both legacy messages"
    );
    assert_eq!(
        report.state, before.state,
        "posted legacy keys did not change asynchronous device state"
    );
    controller
        .close(&timed(Duration::from_secs(5)))
        .expect("closed state row");
}

fn terminal_and_cleanup_rows() {
    let mut preflight = FixtureProcess::spawn("terminal-preflight");
    let (target, controller) = open_ordinary(&preflight);
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let cancelled = OperationContext::new().with_cancellation(cancellation);
    let error = controller
        .execute(&key_pair_request(target), &cancelled)
        .expect_err("pre-admission cancellation has no receipt");
    assert_eq!(error.status(), Status::Cancelled);
    let error = controller
        .execute(&key_pair_request(target), &timed(Duration::ZERO))
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
    let receipt = controller
        .execute(
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
    let worker_controller = Arc::clone(&controller);
    let worker =
        thread::spawn(move || worker_controller.execute(&pressed_delay_request(target), &context));
    cancelled.wait_for(
        "observation role=target family=key-down",
        Duration::from_secs(5),
    );
    let cleanup_started = Instant::now();
    cancellation.cancel();
    let receipt = worker
        .join()
        .expect("cancellation worker joined")
        .expect("mid-sequence cancellation receipted");
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
}

fn late_effect_row() {
    let mut fixture = FixtureProcess::spawn("late-effect");
    let (target, controller) = open_ordinary(&fixture);
    fixture.control(fixture.target(), CONTROL_BLOCK_QUEUE, 300);
    fixture.wait_for("control queue-block=ready", Duration::from_secs(5));
    let started = Instant::now();
    let receipt = controller
        .execute(&key_pair_request(target), &timed(Duration::from_secs(2)))
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

fn topology_geometry_rows() {
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
        fixture.wait_for(
            "observation role=target family=pointer-move units=1",
            Duration::from_secs(5),
        );
        let before_route = fixture.report(fixture.target());
        assert_wrong_targets_clear(before_route);

        let stale = controller
            .execute(
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

        let current = controller
            .execute(
                &source_pointer_request(target, frame.stamp()),
                &timed(Duration::from_secs(5)),
            )
            .expect("current source-frame pointer receipt");
        assert_complete_queue_receipt(&current, 1);
        let after_current = fixture.report(fixture.target());
        assert_eq!(
            after_current.target,
            before_route.target + 1,
            "exactly one current source-frame event reached the target"
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
            "topology-monitor index={index} frame-scale={}x{} source=correlated",
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

fn controlled_unrelated_activity_row() {
    let mut fixture = FixtureProcess::spawn("unrelated-activity");
    let (target, controller) = open_ordinary(&fixture);
    let foreground = fixture.role_window("Foreground");
    let desktop = DesktopState::capture();
    assert_ne!(
        desktop.foreground,
        fixture.target(),
        "the target must remain behind an unrelated foreground application"
    );
    let foreground_value = foreground.0.addr();
    let activity = thread::spawn(move || {
        let foreground = HWND(std::ptr::with_exposed_provenance_mut::<std::ffi::c_void>(
            foreground_value,
        ));
        for _unit in 0..64 {
            // SAFETY: the retained fixture handle is live; this message carries scalar metadata.
            unsafe { PostMessageW(Some(foreground), WM_MOUSEMOVE, WPARAM(0), LPARAM(0)) }
                .expect("posted controlled unrelated activity");
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
    let receipt = controller
        .execute(&request, &timed(Duration::from_secs(5)))
        .expect("delivery during unrelated activity");
    activity.join().expect("unrelated activity joined");
    assert_complete_queue_receipt(&receipt, 3);
    let report = fixture.report(fixture.target());
    assert_eq!(report.target, 2);
    assert_eq!(report.foreground, 64);
    assert_eq!(report.sibling, 0);
    assert_eq!(report.child, 0);
    assert_eq!(report.game, 0);
    desktop.assert_unchanged();
    controller
        .close(&timed(Duration::from_secs(5)))
        .expect("closed activity row");
}

fn queue_pressure_rows() {
    let mut full = FixtureProcess::spawn("queue-full");
    let (target_id, controller) = open_ordinary(&full);
    let target = full.target();
    full.control(target, CONTROL_BLOCK_QUEUE, 15_000);
    full.wait_for("control queue-block=ready", Duration::from_secs(5));
    let capacity = fill_queue(target);
    assert!(
        capacity >= 1_000,
        "unexpectedly small Windows message queue"
    );
    let started = Instant::now();
    let receipt = controller
        .execute(&one_key_request(target_id), &timed(Duration::from_secs(5)))
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
    partial.control(target, CONTROL_BLOCK_QUEUE, 15_000);
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
    let receipt = controller
        .execute(&request, &timed(Duration::from_secs(5)))
        .expect("partial queue refusal receipted");
    assert_elapsed_budget(
        "queue-partial-refusal",
        started.elapsed(),
        Duration::from_millis(10),
    );
    assert_eq!(receipt.outcome(), SequenceOutcome::Partial, "{receipt:?}");
    assert_eq!(receipt.submitted(), 1);
    assert_eq!(receipt.fault(), Some(InputFault::SubmissionFailed));
    drop(partial);
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
