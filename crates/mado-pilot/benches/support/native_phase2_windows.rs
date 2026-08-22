// Windows-specific implementation for the Phase 2 native benchmark.

#[cfg(windows)]
use mado_pilot::NativeEngineRequest;
#[cfg(windows)]
use mado_pilot_testkit::bench_harness::{
    CaptureResources, PHASE2_WINDOWS_PRODUCTION_1280_COPIED_BYTES_LIMIT,
    PHASE2_WINDOWS_PRODUCTION_1280_DETACHED_TEXTURES_LIMIT,
    PHASE2_WINDOWS_PRODUCTION_1280_GPU_RESOURCES_LIMIT,
    PHASE2_WINDOWS_PRODUCTION_1280_HEAP_LIMIT_BYTES,
    PHASE2_WINDOWS_PRODUCTION_1280_LATENCY_BUDGETS,
    PHASE2_WINDOWS_PRODUCTION_1280_RESIDENT_LIMIT_BYTES,
    PHASE2_WINDOWS_PRODUCTION_1280_STAGING_TEXTURES_LIMIT,
    PHASE2_WINDOWS_PRODUCTION_1280_STALE_WORK_LIMIT,
    PHASE2_WINDOWS_PRODUCTION_DUAL_4K_COPIED_BYTES_LIMIT,
    PHASE2_WINDOWS_PRODUCTION_DUAL_4K_DETACHED_TEXTURES_LIMIT,
    PHASE2_WINDOWS_PRODUCTION_DUAL_4K_GPU_RESOURCES_LIMIT,
    PHASE2_WINDOWS_PRODUCTION_DUAL_4K_HEAP_LIMIT_BYTES,
    PHASE2_WINDOWS_PRODUCTION_DUAL_4K_LATENCY_BUDGETS,
    PHASE2_WINDOWS_PRODUCTION_DUAL_4K_RESIDENT_LIMIT_BYTES,
    PHASE2_WINDOWS_PRODUCTION_DUAL_4K_STAGING_TEXTURES_LIMIT,
    PHASE2_WINDOWS_PRODUCTION_DUAL_4K_STALE_WORK_LIMIT,
    PHASE2_WINDOWS_PRODUCTION_TRANSITION_1280_HEAP_LIMIT_BYTES,
    PHASE2_WINDOWS_PRODUCTION_TRANSITION_1280_LATENCY_BUDGETS, PrefixedLineMatch,
    bounded_child_output, classify_prefixed_line, enforce_latency_budgets, measure_pair,
    nonzero_at_most,
};
#[cfg(windows)]
use std::io::{BufRead, BufReader};
#[cfg(windows)]
use std::mem::size_of;
#[cfg(windows)]
use std::process::{Child, Stdio};
#[cfg(windows)]
use std::sync::mpsc;

#[cfg(windows)]
use mado_pilot::{
    CapabilitySupport, CoordinateSpace, FrameStamp, InputAddressScope, InputReceipt, Point,
    PointerButton, SequenceLimits, SubmissionEvidence, TargetKind,
};

#[cfg(windows)]
use mado_pilot_platform_windows::benchmark::{
    CallbackCopyObservation, CaptureMetricsSnapshot, callback_metric_baseline,
    callback_observation_after, capture_metrics, dual_display_fixture_points, dual_display_seam_x,
    reset_capture_metrics,
};
#[cfg(windows)]
use mado_pilot_platform_windows::fixture_protocol as protocol;
#[cfg(windows)]
use windows::Win32::Foundation::RECT;
#[cfg(windows)]
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::GetCurrentProcess;
#[cfg(windows)]
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetWindowRect, PostMessageW, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, WM_CLOSE,
};
#[cfg(windows)]
use windows::core::PCWSTR;

#[cfg(windows)]
type NativeEngine = Engine;

#[cfg(windows)]
const LANGUAGE_PROCESS_WAIT: Duration = OPERATION_WAIT;

#[cfg(windows)]
const fn language_event(kind: u32, text_units: u32) -> protocol::EventSummary {
    protocol::EventSummary { kind, text_units }
}

#[cfg(windows)]
const CPP_LANGUAGE_EVENTS: [protocol::EventSummary; 15] = [
    language_event(protocol::EVENT_POINTER_MOVE, 0),
    language_event(protocol::EVENT_POINTER_PRESS, 0),
    language_event(protocol::EVENT_POINTER_RELEASE, 0),
    language_event(protocol::EVENT_POINTER_PRESS, 0),
    language_event(protocol::EVENT_POINTER_RELEASE, 0),
    language_event(protocol::EVENT_POINTER_PRESS, 0),
    language_event(protocol::EVENT_POINTER_RELEASE, 0),
    language_event(protocol::EVENT_POINTER_SCROLL, 0),
    language_event(protocol::EVENT_KEY_DOWN, expected_key_units()),
    language_event(protocol::EVENT_KEY_UP, expected_key_units()),
    language_event(protocol::EVENT_KEY_DOWN, expected_key_units()),
    language_event(protocol::EVENT_KEY_DOWN, expected_key_units()),
    language_event(protocol::EVENT_KEY_UP, expected_key_units()),
    language_event(protocol::EVENT_KEY_UP, expected_key_units()),
    language_event(protocol::EVENT_TEXT, 3),
];

#[cfg(windows)]
const fn input_workload_description() -> &'static str {
    "native interactive System input and the public Rust common flow"
}

#[cfg(windows)]
const fn input_queue_policy() -> &'static str {
    "session latest-wins queue depth 1; bounded input sequence executes serially"
}

#[cfg(windows)]
const fn capture_workload_description() -> &'static str {
    "native steady capture, latest acquisition, and explicit CPU mapping"
}

#[cfg(windows)]
const fn transition_workload_description() -> &'static str {
    "native open, retained-pressure recovery, resize, and close transitions"
}

#[cfg(windows)]
const fn capture_queue_policy() -> &'static str {
    "session latest-wins queue depth 1; adapter finite retained-storage limit"
}

#[cfg(windows)]
const fn transition_queue_policy() -> &'static str {
    "session latest-wins queue depth 1; retained-pressure case fills the reported finite storage limit"
}

#[cfg(windows)]
struct ThreadDpiContext(DPI_AWARENESS_CONTEXT);

#[cfg(windows)]
impl ThreadDpiContext {
    fn per_monitor() -> Self {
        // SAFETY: this changes only the benchmark thread and returns the context
        // restored by Drop before the move operation returns.
        Self(unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) })
    }
}

#[cfg(windows)]
impl Drop for ThreadDpiContext {
    fn drop(&mut self) {
        // SAFETY: self.0 is the exact prior context returned on this thread.
        let _restored = unsafe { SetThreadDpiAwarenessContext(self.0) };
    }
}

#[cfg(windows)]
struct FixtureProcess {
    child: Child,
    lines: mpsc::Receiver<String>,
    reader: Option<thread::JoinHandle<()>>,
}

#[cfg(windows)]
impl FixtureProcess {
    fn spawn(behavior: FixtureBehavior) -> Self {
        let executable = &arguments().fixture_executable;
        let mut child = Command::new(executable)
            .args(behavior.arguments())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap_or_else(|error| panic!("the benchmark fixture could not start: {error}"));
        let process_id = child.id();
        eprintln!("benchmark-fixture pid={process_id} behavior={behavior:?} state=spawned");
        let output = child
            .stdout
            .take()
            .expect("the benchmark fixture exposes readiness output");
        let (sender, lines) = mpsc::sync_channel(512);
        let reader = thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else {
                    break;
                };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let fixture = Self {
            child,
            lines,
            reader: Some(reader),
        };
        let ready = fixture
            .lines
            .recv_timeout(FIXTURE_WAIT)
            .expect("the benchmark fixture reports readiness");
        assert!(
            ready_line_is_approved(&ready, process_id),
            "the benchmark fixture readiness record was not the exact approved context"
        );
        eprintln!("benchmark-fixture pid={process_id} behavior={behavior:?} state=ready");
        fixture
    }

    fn process_id(&self) -> u32 {
        self.child.id()
    }

    fn title(&self) -> String {
        protocol::fixture_title(self.process_id())
    }

    fn window(&self) -> windows::Win32::Foundation::HWND {
        let class = protocol::CLASS_NAME
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let title = self
            .title()
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>();
        // SAFETY: both strings are terminated and remain live for this
        // lookup; the PID-qualified fixture title is unique.
        unsafe { FindWindowW(PCWSTR(class.as_ptr()), PCWSTR(title.as_ptr())) }
            .expect("the exact benchmark fixture window remains live")
    }

    fn move_to(&self, x: i32, y: i32) {
        let _dpi = ThreadDpiContext::per_monitor();
        let hwnd = self.window();
        // SAFETY: hwnd is the live fixture popup. The benchmark changes only
        // its signed desktop placement and preserves z-order and activation.
        unsafe { SetWindowPos(hwnd, None, x, y, 1_280, 720, SWP_NOZORDER | SWP_NOACTIVATE) }
            .expect("the benchmark fixture moves to the requested signed desktop position");
        let mut observed = RECT::default();
        // SAFETY: hwnd remains the live fixture popup and observed is writable.
        unsafe { GetWindowRect(hwnd, &raw mut observed) }
            .expect("the benchmark fixture exposes its moved rectangle");
        assert_eq!(
            observed,
            RECT {
                left: x,
                top: y,
                right: x + 1_280,
                bottom: y + 720,
            },
            "the fixture reaches the exact requested moving-seam placement"
        );
    }

    fn close_window(&self) -> bool {
        // SAFETY: the handle names this fixture's live top-level window; a
        // posted WM_CLOSE asks its owning GUI thread to destroy it.
        unsafe {
            PostMessageW(
                Some(self.window()),
                WM_CLOSE,
                Default::default(),
                Default::default(),
            )
        }
        .is_ok()
    }

    fn next_flow(&self, expected: &[protocol::EventSummary]) -> bool {
        expected.iter().all(|expected| {
            let deadline = Instant::now() + OPERATION_WAIT;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return false;
                }
                let Ok(line) = self.lines.recv_timeout(remaining) else {
                    return false;
                };
                let Some(observed) = protocol::parse_event_line(&line) else {
                    continue;
                };
                if observed != *expected {
                    eprintln!(
                        "benchmark fixture event mismatch: expected {expected:?}, observed {observed:?}"
                    );
                    return false;
                }
                return true;
            }
        })
    }

    fn finish_observation(mut self) -> bool {
        let _killed = self.child.kill();
        let _waited = self.child.wait();
        let deadline = Instant::now() + OPERATION_WAIT;
        let mut complete = true;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                eprintln!("benchmark fixture output reader did not terminate");
                return false;
            }
            match self.lines.recv_timeout(remaining) {
                Ok(line) => {
                    if let Some(event) = protocol::parse_event_line(&line) {
                        eprintln!("benchmark fixture emitted a trailing event: {event:?}");
                        complete = false;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    eprintln!("benchmark fixture output reader did not terminate");
                    return false;
                }
            }
        }
        if self
            .reader
            .take()
            .is_some_and(|reader| reader.join().is_err())
        {
            eprintln!("benchmark fixture output reader panicked");
            complete = false;
        }
        complete
    }

    fn next_key_pair(&self) -> bool {
        let units = expected_key_units();
        self.next_key_pair_with_units(units, units)
    }

    fn next_key_pair_with_units(&self, down_units: u32, up_units: u32) -> bool {
        self.next_event(protocol::EVENT_KEY_DOWN, down_units, true)
            && self.next_event(protocol::EVENT_KEY_UP, up_units, true)
    }

    fn next_pointer_move(&self) -> bool {
        self.next_event(protocol::EVENT_POINTER_MOVE, 0, false)
    }

    fn next_event(&self, expected: u32, text_units: u32, key_event: bool) -> bool {
        let deadline = Instant::now() + OPERATION_WAIT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let Ok(line) = self.lines.recv_timeout(remaining) else {
                return false;
            };
            let Some(event) = protocol::parse_event_line(&line) else {
                continue;
            };
            let relevant = if key_event {
                matches!(
                    event.kind,
                    protocol::EVENT_KEY_DOWN | protocol::EVENT_KEY_UP
                )
            } else {
                event.kind == expected
            };
            if relevant {
                let matches = event.kind == expected && event.text_units == text_units;
                if !matches {
                    eprintln!(
                        "benchmark fixture event mismatch: expected kind={expected} units={text_units}, observed {event:?}"
                    );
                }
                return matches;
            }
        }
    }
}

#[cfg(windows)]
impl Drop for FixtureProcess {
    fn drop(&mut self) {
        let process_id = self.child.id();
        eprintln!("benchmark-fixture pid={process_id} state=stopping");
        let status = match self.child.try_wait() {
            Ok(Some(status)) => Ok(status),
            Ok(None) => {
                if let Err(error) = self.child.kill() {
                    eprintln!("benchmark-fixture pid={process_id} state=kill-failed error={error}");
                }
                self.child.wait()
            }
            Err(error) => Err(error),
        };
        match status {
            Ok(status) => eprintln!(
                "benchmark-fixture pid={process_id} state=stopped exit_code={:?}",
                status.code()
            ),
            Err(error) => {
                eprintln!("benchmark-fixture pid={process_id} state=wait-failed error={error}")
            }
        }
    }
}

#[cfg(windows)]
struct OrdinaryFixtureProcess {
    child: Child,
    lines: mpsc::Receiver<String>,
    title: String,
}

#[cfg(windows)]
impl OrdinaryFixtureProcess {
    fn spawn() -> Self {
        let executable = arguments()
            .ordinary_fixture_executable
            .as_ref()
            .expect("the Windows input benchmark requires its ordinary fixture executable");
        let mut child = Command::new(executable)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap_or_else(|error| {
                panic!("the ordinary input benchmark fixture could not start: {error}")
            });
        let process_id = child.id();
        let title = protocol::ordinary_fixture_title(&process_id.to_string());
        let output = child
            .stdout
            .take()
            .expect("the ordinary fixture exposes readiness output");
        let (sender, lines) = mpsc::sync_channel(512);
        thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else {
                    break;
                };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let fixture = Self {
            child,
            lines,
            title,
        };
        let ready = fixture
            .lines
            .recv_timeout(FIXTURE_WAIT)
            .expect("the ordinary fixture reports readiness");
        assert!(
            ordinary_ready_line_is_approved(&ready, &fixture.title),
            "the ordinary fixture readiness record was not the exact approved context"
        );
        fixture
    }

    fn next_pointer_move(&self) -> bool {
        self.next_target_observations(&["observation role=target family=pointer-move units=1"])
    }

    fn next_pointer_button_press(&self) -> bool {
        self.next_target_observations(&[
            "observation role=target family=pointer-move units=1",
            "observation role=target family=pointer-move units=1",
            "observation role=target family=button-down units=1",
        ])
    }
    fn next_pointer_moves(&self, count: usize) -> bool {
        let deadline = Instant::now() + OPERATION_WAIT;
        (0..count).all(|_| {
            self.next_target_observation(
                "observation role=target family=pointer-move units=1",
                deadline,
            )
        })
    }

    fn next_target_observations(&self, expected: &[&str]) -> bool {
        let deadline = Instant::now() + OPERATION_WAIT;
        expected
            .iter()
            .all(|expected_line| self.next_target_observation(expected_line, deadline))
    }

    fn next_target_observation(&self, expected_line: &str, deadline: Instant) -> bool {
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let Ok(line) = self.lines.recv_timeout(remaining) else {
                return false;
            };
            match classify_prefixed_line(&line, "observation role=", expected_line) {
                PrefixedLineMatch::Irrelevant => {}
                PrefixedLineMatch::Expected => return true,
                PrefixedLineMatch::Unexpected => {
                    eprintln!(
                        "ordinary fixture observation mismatch: expected `{expected_line}`, observed `{line}`"
                    );
                    return false;
                }
            }
        }
    }
}

#[cfg(windows)]
impl Drop for OrdinaryFixtureProcess {
    fn drop(&mut self) {
        let _killed = self.child.kill();
        let _waited = self.child.wait();
    }
}

#[cfg(windows)]
struct OrdinaryInputFlow {
    _engine: Engine,
    fixture: OrdinaryFixtureProcess,
    session: Session,
    target: TargetId,
}

#[cfg(windows)]
impl OrdinaryInputFlow {
    fn spawn() -> Self {
        let fixture = OrdinaryFixtureProcess::spawn();
        let engine = native_engine();
        require_permissions(&engine);
        let selection_deadline = Instant::now() + FIXTURE_WAIT;
        let target = loop {
            let targets = engine
                .discover(&bounded(OPERATION_WAIT))
                .expect("the ordinary benchmark fixture is discoverable");
            let mut matching = targets.iter().filter(|candidate| {
                candidate.name() == fixture.title
                    && candidate.capability().kind() == Some(TargetKind::Window)
            });
            if let Some(candidate) = matching.next() {
                assert!(
                    matching.next().is_none(),
                    "the ordinary benchmark fixture title is unique"
                );
                break candidate.id();
            }
            assert!(
                Instant::now() < selection_deadline,
                "the exact ordinary benchmark fixture becomes selectable"
            );
            thread::sleep(Duration::from_millis(50));
        };
        let descriptor = engine
            .describe_input(target, &bounded(OPERATION_WAIT))
            .expect("the ordinary fixture exposes an input descriptor");
        let pair = descriptor
            .capability()
            .pair(InputOperationKind::Pointer, InputDelivery::WindowMessage);
        assert_eq!(pair.support(), CapabilitySupport::Unknown);
        assert_eq!(
            pair.evidence(),
            Some(SubmissionEvidence::TargetQueueAdmission)
        );
        assert!(!pair.focus_required());
        let session = engine
            .open_session(
                target,
                &SessionRequest::new().requesting_input(
                    InputOpenRequest::new()
                        .with_requirement(InputRequirement::Required)
                        .requiring(InputOperationKind::Pointer, InputDelivery::WindowMessage),
                ),
                &bounded(OPERATION_WAIT),
            )
            .expect("the ordinary fixture opens required exact-window pointer input");
        Self {
            _engine: engine,
            fixture,
            session,
            target,
        }
    }
}

#[cfg(windows)]
impl Drop for OrdinaryInputFlow {
    fn drop(&mut self) {
        let _closed = close(&self.session);
    }
}

#[cfg(windows)]
struct DualDisplayFlow {
    _engine: Engine,
    _fixture: FixtureProcess,
    movement_index: Mutex<u32>,
    state: Mutex<Vec<DualDisplayState>>,
}

#[cfg(windows)]
struct DualDisplayState {
    session: Session,
    last: Frame,
    origin: (f64, f64),
    scale: f64,
    fixture_point: (f64, f64),
}

#[cfg(windows)]
impl DualDisplayFlow {
    fn spawn() -> Self {
        assert_capture_resources_released();
        reset_capture_metrics();
        let fixture = FixtureProcess::spawn(FixtureBehavior::ProductionCapture);
        let engine = native_engine();
        require_permissions(&engine);
        let targets = engine
            .discover(&bounded(OPERATION_WAIT))
            .expect("the dual-4K production displays are discoverable");
        let mut displays = Vec::new();
        for target in targets
            .iter()
            .filter(|target| target.capability().kind() == Some(TargetKind::Display))
        {
            let session = engine
                .open_session(
                    target.id(),
                    &SessionRequest::new().capturing(OpenRequest::new()),
                    &bounded(OPERATION_WAIT),
                )
                .expect("each declared display opens production capture");
            let last = session
                .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
                .expect("each declared display publishes a seed frame");
            assert_eq!(
                last.descriptor().extent(),
                PixelExtent::new(3_840, 2_160),
                "the dual-display profile refuses any non-4K display",
            );
            let seeded_mapping = last
                .map(PixelFormat::Bgra8, &bounded(OPERATION_WAIT))
                .expect("each seed frame primes the steady mapped-frame allocation");
            drop(seeded_mapping);
            let placement = last
                .transform()
                .target()
                .expect("each Windows display frame carries authoritative placement");
            let origin = placement.desktop_origin();
            displays.push(DualDisplayState {
                session,
                last,
                origin,
                scale: placement.scale().x(),
                fixture_point: (0.0, 0.0),
            });
        }
        displays.sort_by(|left, right| left.origin.0.total_cmp(&right.origin.0));
        assert_eq!(
            displays.len(),
            2,
            "the dual-4K profile requires exactly two online displays",
        );
        assert!(
            displays[0].origin == (-3_840.0, 0.0)
                && displays[1].origin == (0.0, 0.0)
                && (displays[0].scale - 1.25).abs() < f64::EPSILON
                && (displays[1].scale - 1.5).abs() < f64::EPSILON,
            "the dual-4K profile requires signed secondary 125% and primary 150% placement; observed {:?}",
            displays
                .iter()
                .map(|display| (display.origin, display.scale))
                .collect::<Vec<_>>(),
        );

        fixture.move_to(-640, 600);
        displays[0].fixture_point = (-320.0, 960.0);
        displays[1].fixture_point = (320.0, 960.0);
        Self {
            _engine: engine,
            _fixture: fixture,
            movement_index: Mutex::new(0),
            state: Mutex::new(displays),
        }
    }

    fn move_across_seam(&self) {
        let index = {
            let mut next = self
                .movement_index
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let index = *next;
            *next = next
                .checked_add(1)
                .expect("the bounded movement sample index remains representable");
            index
        };
        let x = dual_display_seam_x(index);
        self._fixture.move_to(x, 600);
        let points = dual_display_fixture_points(x, 600);
        let mut displays = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(displays.len(), points.len());
        for (display, point) in displays.iter_mut().zip(points) {
            display.fixture_point = point;
        }
    }
}

#[cfg(windows)]
impl Drop for DualDisplayFlow {
    fn drop(&mut self) {
        let displays = self
            .state
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for display in displays {
            let _closed = close(&display.session);
        }
    }
}

#[cfg(windows)]
fn production_capture_1280_workloads(plan: Plan) -> Vec<Workload> {
    let workloads = vec![
        measure(
            "steady_frame_acquisition",
            "one strictly newer 1280x720 production frame arrives from the fixture timer, preserves declared content and frame-authoritative geometry, and reports observable sequence gaps",
            plan,
            production_active_1280,
            production_steady_acquisition,
        ),
        measure(
            "callback_copy",
            "successful callback-side detach copies account for every producer publication observed before one exact mapped 1280x720 result",
            plan,
            production_active_1280,
            windows_callback_copy,
        ),
        measure(
            "latest_acquisition",
            "after timer-driven production progress, latest returns a same-stream 1280x720 frame no older than the proven publication and reports observable queue pressure",
            plan,
            production_active_1280,
            production_latest_acquisition,
        ),
        measure(
            "cpu_map_bgra8",
            "one timer-driven 1280x720 production frame maps once to exact-size BGRA8 bytes carrying declared fixture content",
            plan,
            production_active_1280,
            production_cpu_map,
        ),
    ];
    assert_capture_resources_released();
    workloads
}

#[cfg(windows)]
fn production_capture_dual_4k_workloads(plan: Plan) -> Vec<Workload> {
    let [arrival, callback_copy] = measure_pair(
        (
            "dual_display_frame_arrival",
            "one strictly newer frame arrives from each topology-qualified 4K display while the deterministic fixture straddles their signed-origin seam; both exact mappings carry declared fixture content",
        ),
        (
            "dual_display_callback_copy",
            "callback-side detach copies account for both 4K display sessions before two exact mapped results carry declared fixture content across the signed-origin seam",
        ),
        plan,
        DualDisplayFlow::spawn,
        dual_display_samples,
    );
    let moving = measure(
        "dual_display_moving_seam",
        "300 strictly newer correlated frame pairs follow the controlled fixture across the signed mixed-DPI seam with exact placement, mapping, callback-copy, and cleanup evidence",
        Plan::new(0, 300),
        DualDisplayFlow::spawn,
        dual_display_moving_seam,
    );
    let workloads = vec![arrival, callback_copy, moving];
    assert_capture_resources_released();
    workloads
}

#[cfg(windows)]
fn production_transition_1280_workloads(plan: Plan) -> Vec<Workload> {
    eprintln!("production-transitions: open_first_frame");
    let open = measure(
        "open_first_frame",
        "each fresh production capture-only session returns and maps one exact 1280x720 declared-content frame, then closes",
        plan,
        production_flow_1280,
        production_open_first_frame,
    );
    eprintln!("production-transitions: retained_pressure_resume");
    let pressure = measure(
        "retained_pressure_resume",
        "timer-driven 1280x720 production fills the reported retained limit; one blocked publication remains observable and releasing one slot resumes with a sequence gap",
        Plan::new(0, 1),
        production_fixture_1280,
        production_retained_pressure_resume,
    );
    eprintln!("production-transitions: resize_recreation");
    let resize = measure(
        "resize_recreation",
        "one bounded pointer stimulus resizes the production fixture between exact 1280x720 and 1440x810 surfaces; only a new epoch, geometry, extent, and transform satisfy the oracle",
        plan,
        production_resize_active_1280,
        production_resize_recreation,
    );
    eprintln!("production-transitions: target_loss_recovery");
    let recovery = measure(
        "target_loss_recovery",
        "closing the owned production target yields typed TargetLost while a retained mapping survives; a fresh PID-qualified replacement opens and maps exact 1280x720 content",
        Plan::new(0, 1),
        production_recovery_setup,
        production_target_loss_recovery,
    );
    eprintln!("production-transitions: close_drain");
    let close = measure(
        "close_drain",
        "explicit production capture-only close drains within its bound, reaches the closed state, and remains idempotent",
        plan,
        production_flow_1280,
        production_close_drain,
    );
    let workloads = vec![open, pressure, resize, recovery, close];
    assert_capture_resources_released();
    workloads
}

#[cfg(windows)]
fn capture_workloads(plan: Plan) -> Vec<Workload> {
    vec![
        measure(
            "stimulus_to_frame",
            "one acknowledged deterministic fixture change produces a newer stamped frame carrying the opposite declared fill; intermediate publications are discarded and counted",
            plan,
            || ActiveFlow::from_fixture(Rc::new(FixtureProcess::spawn(FixtureBehavior::Animate))),
            stimulus_to_frame,
        ),
        measure(
            "latest_acquisition",
            "after an exact stimulated publication advances the producer, latest returns a same-stream frame no older than that publication and reports the observed sequence gap",
            plan,
            || ActiveFlow::from_fixture(Rc::new(FixtureProcess::spawn(FixtureBehavior::Animate))),
            latest_acquisition,
        ),
        measure(
            "cpu_map_bgra8",
            "the newer frame maps once to exact-size BGRA8 bytes carrying one declared fixture fill",
            plan,
            || ActiveFlow::from_fixture(Rc::new(FixtureProcess::spawn(FixtureBehavior::Animate))),
            cpu_map,
        ),
    ]
}

#[cfg(windows)]
fn transition_workloads(plan: Plan) -> Vec<Workload> {
    let fixture = Rc::new(FixtureProcess::spawn(FixtureBehavior::AnimateAndResize));
    vec![
        measure(
            "resize_recreation",
            "one deterministic fixture resize advances epoch and geometry and returns the resized extent",
            plan,
            || ActiveFlow::from_fixture(Rc::clone(&fixture)),
            resize_recreation,
        ),
        measure(
            "open_first_frame",
            "each fresh session returns a correctly identified deterministic first frame and closes",
            plan,
            || Flow::from_fixture(Rc::clone(&fixture)),
            open_first_frame,
        ),
        measure(
            "retained_pressure_resume",
            "filling the reported retained limit rejects publication, releasing one slot resumes with an observable sequence gap",
            plan,
            || (),
            retained_pressure_resume,
        ),
        measure(
            "close_drain",
            "explicit close reaches the closed state and remains idempotent",
            plan,
            || Flow::from_fixture(Rc::clone(&fixture)),
            close_drain,
        ),
    ]
}

#[cfg(windows)]
fn production_active_1280() -> ActiveFlow {
    assert_capture_resources_released();
    reset_capture_metrics();
    let active = ActiveFlow::from_capture_fixture(Rc::new(FixtureProcess::spawn(
        FixtureBehavior::ProductionCapture,
    )));
    assert_eq!(
        active
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .last
            .descriptor()
            .extent(),
        PixelExtent::new(1_280, 720),
        "the production fixture exposes an exact 1280x720 capture surface",
    );
    active
}

#[cfg(windows)]
fn production_resize_active_1280() -> ActiveFlow {
    assert_capture_resources_released();
    reset_capture_metrics();
    let active = ActiveFlow::from_fixture(Rc::new(FixtureProcess::spawn(
        FixtureBehavior::ProductionCaptureAndResize,
    )));
    assert_eq!(
        active
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .last
            .descriptor()
            .extent(),
        PixelExtent::new(1_280, 720),
        "the production resize fixture begins at exactly 1280x720",
    );
    active
}

#[cfg(windows)]
fn production_flow_1280() -> Flow {
    assert_capture_resources_released();
    reset_capture_metrics();
    let flow = Flow::from_fixture(Rc::new(FixtureProcess::spawn(
        FixtureBehavior::ProductionCapture,
    )));
    let session = flow.open_capture_session();
    let frame = session
        .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
        .expect("the production flow publishes an initial frame");
    assert_eq!(
        frame.descriptor().extent(),
        PixelExtent::new(1_280, 720),
        "the production flow begins at exactly 1280x720",
    );
    assert!(close(&session), "the production setup session closes");
    flow
}

#[cfg(windows)]
fn production_fixture_1280() -> Rc<FixtureProcess> {
    assert_capture_resources_released();
    reset_capture_metrics();
    Rc::new(FixtureProcess::spawn(FixtureBehavior::ProductionCapture))
}

#[cfg(windows)]
fn production_recovery_setup() {
    assert_capture_resources_released();
    reset_capture_metrics();
}

#[cfg(windows)]
fn windows_callback_copy(active: &ActiveFlow) -> Sample {
    let mut state = lock_state(active);
    let before_stamp = state.last.stamp();
    let operation = bounded(OPERATION_WAIT);
    let (frame, observation) = acquire_correlated_frame(&state.session, before_stamp, &operation);
    let mapping = frame
        .map(PixelFormat::Bgra8, &operation)
        .expect("the callback-copy result maps for its content oracle");
    let after_metrics = capture_metrics();
    let stamp = frame.stamp();
    let delta = stamp
        .sequence()
        .value()
        .saturating_sub(before_stamp.sequence().value());
    let mapped = u64::try_from(mapping.bytes().len()).expect("mapped bytes fit u64");
    let correct = stamp.stream() == before_stamp.stream()
        && stamp.epoch() == before_stamp.epoch()
        && stamp.sequence() > before_stamp.sequence()
        && frame.descriptor().extent() == PixelExtent::new(1_280, 720)
        && mapping.stamp() == stamp
        && mapping_is_benchmark_content(&mapping)
        && observation.copied_bytes == mapped
        && after_metrics.callback_observation_losses == 0
        && after_metrics.detached_textures_peak > 0
        && after_metrics.staging_textures_peak > 0;
    state.last = frame;
    Sample::new(observation.callback_copy_time, correct, mapped)
        .with_stale_work(delta.saturating_sub(1), delta)
        .with_capture_resources(capture_resources(
            observation.interval_copied_bytes,
            after_metrics,
            2,
        ))
        .with_peak_resident_bytes(peak_resident_bytes())
}

#[cfg(windows)]
fn dual_display_moving_seam(flow: &DualDisplayFlow) -> Sample {
    flow.move_across_seam();
    let (arrival, _callback) = dual_display_samples(flow);
    arrival
}

#[cfg(windows)]
fn dual_display_samples(flow: &DualDisplayFlow) -> (Sample, Sample) {
    let mut displays = flow
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let operation = bounded(OPERATION_WAIT);
    let started = Instant::now();
    let frames = displays
        .iter()
        .map(|display| acquire_correlated_frame(&display.session, display.last.stamp(), &operation))
        .collect::<Vec<_>>();
    let arrival = started.elapsed();
    let mut correct = true;
    let mut mapped_bytes = 0_u64;
    let mut copied_bytes = 0_u64;
    let mut callback_time = Duration::ZERO;
    let mut stale = 0_u64;
    let mut scheduled = 0_u64;
    let surface_bytes = 3_840_u64 * 2_160 * 4;
    for (display, (frame, observation)) in displays.iter_mut().zip(frames) {
        let before = display.last.stamp();
        let mapping = frame
            .map(PixelFormat::Bgra8, &operation)
            .expect("each 4K display frame maps to BGRA8");
        let stamp = frame.stamp();
        let delta = stamp
            .sequence()
            .value()
            .saturating_sub(before.sequence().value());
        let placement = frame
            .transform()
            .target()
            .expect("each 4K display frame retains placement");
        let desktop_point = Point::new(
            CoordinateSpace::DesktopLogical,
            display.fixture_point.0,
            display.fixture_point.1,
        )
        .expect("the fixture sample point is finite");
        let capture_point = frame
            .transform()
            .convert_point(desktop_point, CoordinateSpace::CapturePixels)
            .expect("the display transform resolves the fixture point");
        let identity_ok = stamp.stream() == before.stream()
            && stamp.epoch() == before.epoch()
            && stamp.sequence() > before.sequence()
            && frame.descriptor().extent() == PixelExtent::new(3_840, 2_160)
            && mapping.stamp() == stamp;
        let placement_ok = placement.desktop_origin() == display.origin
            && (placement.scale().x() - display.scale).abs() < f64::EPSILON;
        let pixel_ok = mapping_pixel_is_benchmark_content(&mapping, capture_point);
        correct &=
            identity_ok && placement_ok && pixel_ok && observation.copied_bytes == surface_bytes;
        mapped_bytes = mapped_bytes
            .saturating_add(u64::try_from(mapping.bytes().len()).expect("mapped bytes fit u64"));
        copied_bytes = copied_bytes.saturating_add(observation.interval_copied_bytes);
        callback_time = callback_time.saturating_add(observation.callback_copy_time);
        stale = stale.saturating_add(delta.saturating_sub(1));
        scheduled = scheduled.saturating_add(delta);
        display.last = frame;
    }
    let after_metrics = capture_metrics();
    let callback_elapsed =
        callback_time / u32::try_from(displays.len()).expect("two displays fit u32");
    correct &= mapped_bytes == surface_bytes.saturating_mul(2)
        && copied_bytes == surface_bytes.saturating_mul(2)
        && after_metrics.callback_observation_losses == 0
        && after_metrics.detached_textures_peak >= 4
        && after_metrics.staging_textures_peak > 0;
    let resources = capture_resources(copied_bytes, after_metrics, 4);
    let resident = peak_resident_bytes();
    (
        Sample::new(arrival, correct, mapped_bytes)
            .with_stale_work(stale, scheduled)
            .with_capture_resources(resources)
            .with_peak_resident_bytes(resident),
        Sample::new(callback_elapsed, correct, mapped_bytes)
            .with_stale_work(stale, scheduled)
            .with_capture_resources(resources)
            .with_peak_resident_bytes(resident),
    )
}

#[cfg(windows)]
fn acquire_correlated_frame(
    session: &Session,
    after: FrameStamp,
    operation: &OperationContext,
) -> (Frame, CallbackCopyObservation) {
    let baseline = callback_metric_baseline();
    let mut floor = after;
    loop {
        let queued = session
            .acquire_frame(&FrameRequest::latest(), operation)
            .expect("the session exposes its current queue floor");
        let queued_stamp = queued.stamp();
        assert_eq!(queued_stamp.stream(), floor.stream());
        assert!(
            queued_stamp.epoch() > floor.epoch()
                || (queued_stamp.epoch() == floor.epoch()
                    && queued_stamp.sequence() >= floor.sequence()),
            "the latest queue floor never moves backward"
        );
        floor = queued_stamp;
        drop(queued);

        let frame = session
            .acquire_frame(&FrameRequest::newer_than(floor), operation)
            .expect("the session publishes within the shared callback deadline");
        match callback_observation_after(baseline, frame.stamp())
            .expect("callback instrumentation remains coherent and lossless")
        {
            Some(observation) => return (frame, observation),
            None => {
                floor = frame.stamp();
            }
        }
    }
}

#[cfg(windows)]
const fn capture_resources(
    copied_bytes: u64,
    metrics: CaptureMetricsSnapshot,
    producer_resources: u64,
) -> CaptureResources {
    CaptureResources {
        copied_bytes,
        detached_textures_peak: metrics.detached_textures_peak,
        staging_textures_peak: metrics.staging_textures_peak,
        gpu_resources_peak: producer_resources
            .saturating_add(metrics.detached_textures_peak)
            .saturating_add(metrics.staging_textures_peak),
    }
}

#[cfg(windows)]
fn peak_resident_bytes() -> u64 {
    let mut counters = PROCESS_MEMORY_COUNTERS::default();
    let bytes = u32::try_from(size_of::<PROCESS_MEMORY_COUNTERS>())
        .expect("PROCESS_MEMORY_COUNTERS size fits u32");
    counters.cb = bytes;
    // SAFETY: the pseudo handle names this process and counters is writable
    // for the complete native structure declared by bytes.
    unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &raw mut counters, bytes) }
        .expect("the benchmark reads its process memory counters");
    u64::try_from(counters.PeakWorkingSetSize).unwrap_or(u64::MAX)
}

#[cfg(windows)]
fn assert_capture_resources_released() {
    let metrics = capture_metrics();
    assert_eq!(
        (
            metrics.detached_textures_live,
            metrics.staging_textures_live,
        ),
        (0, 0),
        "the preceding production capture workload releases every owned GPU resource",
    );
}

#[cfg(windows)]
fn production_open_first_frame(flow: &Flow) -> Sample {
    let started = Instant::now();
    let session = open_production_capture_session(flow);
    let frame = session
        .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
        .expect("a fresh production session publishes its first frame");
    let elapsed = started.elapsed();
    let mapping = frame
        .map(PixelFormat::Bgra8, &bounded(OPERATION_WAIT))
        .expect("the first production frame maps");
    let correct = frame.stamp().stream() == session.stream()
        && frame.descriptor().extent() == PixelExtent::new(1_280, 720)
        && mapping_is_benchmark_content(&mapping)
        && close(&session);
    Sample::new(
        elapsed,
        correct,
        u64::try_from(mapping.bytes().len()).expect("mapped bytes fit u64"),
    )
    .with_peak_resident_bytes(peak_resident_bytes())
}

#[cfg(windows)]
fn production_resize_recreation(active: &ActiveFlow) -> Sample {
    let mut state = lock_state(active);
    let before = state.last.stamp();
    let old_extent = state.last.descriptor().extent();
    let expected_extent = if old_extent == PixelExtent::new(1_280, 720) {
        PixelExtent::new(1_440, 810)
    } else {
        assert_eq!(
            old_extent,
            PixelExtent::new(1_440, 810),
            "production resize starts from one declared extent",
        );
        PixelExtent::new(1_280, 720)
    };
    let started = Instant::now();
    assert!(
        send_resize_stimulus(&active.flow, &state.session, before.geometry().value()),
        "production resize stimulus is acknowledged",
    );
    let resize_deadline = Instant::now() + FIXTURE_WAIT;
    let frame = loop {
        let candidate = state
            .session
            .acquire_frame(
                &FrameRequest::newer_than(state.last.stamp()),
                &bounded(OPERATION_WAIT),
            )
            .expect("the resized production fixture keeps publishing");
        if candidate.descriptor().extent() != old_extent {
            break candidate;
        }
        state.last = candidate;
        assert!(
            Instant::now() < resize_deadline,
            "the production fixture changes extent before the absolute deadline",
        );
    };
    let elapsed = started.elapsed();
    let mapping = frame
        .map(PixelFormat::Bgra8, &bounded(OPERATION_WAIT))
        .expect("the resized production frame maps");
    let correct = frame.stamp().epoch() > before.epoch()
        && frame.stamp().geometry() > before.geometry()
        && frame.transform().geometry() == frame.stamp().geometry()
        && frame.descriptor().extent() == expected_extent
        && mapping_is_benchmark_content(&mapping);
    state.last = frame;
    Sample::new(
        elapsed,
        correct,
        u64::try_from(mapping.bytes().len()).expect("mapped bytes fit u64"),
    )
}

#[cfg(windows)]
fn production_close_drain(flow: &Flow) -> Sample {
    let session = open_production_capture_session(flow);
    let seed = session
        .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
        .expect("the production close sample owns a live session");
    let exact_extent = seed.descriptor().extent() == PixelExtent::new(1_280, 720);
    let started = Instant::now();
    let first = session.close(&bounded(measured_close_bound()));
    let elapsed = started.elapsed();
    let second = session.close(&bounded(measured_close_bound()));
    let correct = exact_extent && first.is_ok() && second.is_ok() && session.is_closed();
    Sample::unmapped(elapsed, correct).with_peak_resident_bytes(peak_resident_bytes())
}

#[cfg(windows)]
fn production_target_loss_recovery(_: &()) -> Sample {
    let fixture = Rc::new(FixtureProcess::spawn(FixtureBehavior::ProductionCapture));
    let flow = Flow::from_fixture(Rc::clone(&fixture));
    let session = open_production_capture_session(&flow);
    let mut last = session
        .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
        .expect("the production recovery session publishes a seed frame");
    let retained_mapping = last
        .map(PixelFormat::Bgra8, &bounded(OPERATION_WAIT))
        .expect("the production recovery seed maps before target loss");
    let original_exact = last.descriptor().extent() == PixelExtent::new(1_280, 720)
        && mapping_is_benchmark_content(&retained_mapping);

    let started = Instant::now();
    assert!(
        fixture.close_window(),
        "the owned production target accepts WM_CLOSE",
    );
    let deadline = Instant::now() + FIXTURE_WAIT;
    let lost = loop {
        match session.acquire_frame(
            &FrameRequest::newer_than(last.stamp()),
            &bounded(PRESSURE_WAIT),
        ) {
            Ok(frame) => last = frame,
            Err(error) if error.status() == Status::TargetLost => break true,
            Err(error)
                if error.status() == Status::DeadlineExceeded && Instant::now() < deadline =>
            {
                continue;
            }
            Err(_) => break false,
        }
    };
    let first_close = session.close(&bounded(CLOSE_WAIT));
    let second_close = session.close(&bounded(CLOSE_WAIT));
    let retained_survives =
        retained_mapping.bytes().len() == retained_mapping.descriptor().byte_len();
    drop(retained_mapping);
    drop(last);
    drop(session);
    drop(flow);
    drop(fixture);

    let replacement = Rc::new(FixtureProcess::spawn(FixtureBehavior::ProductionCapture));
    let replacement_flow = Flow::from_fixture(Rc::clone(&replacement));
    let replacement_session = open_production_capture_session(&replacement_flow);
    let replacement_frame = replacement_session
        .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
        .expect("the replacement production target publishes a frame");
    let replacement_mapping = replacement_frame
        .map(PixelFormat::Bgra8, &bounded(OPERATION_WAIT))
        .expect("the replacement production frame maps");
    let elapsed = started.elapsed();
    let replacement_exact = replacement_frame.descriptor().extent() == PixelExtent::new(1_280, 720)
        && mapping_is_benchmark_content(&replacement_mapping);
    let replacement_bytes =
        u64::try_from(replacement_mapping.bytes().len()).expect("mapped bytes fit u64");
    let correct = original_exact
        && lost
        && first_close.is_ok()
        && second_close.is_ok()
        && retained_survives
        && replacement_exact
        && close(&replacement_session);
    Sample::new(elapsed, correct, replacement_bytes.saturating_mul(2))
        .with_peak_resident_bytes(peak_resident_bytes())
}

#[cfg(windows)]
fn ordinary_window_queue_submission(flow: &OrdinaryInputFlow) -> Sample {
    let point = Point::new(CoordinateSpace::TargetNormalized, 0.5, 0.5)
        .expect("the ordinary benchmark point is normalized");
    let sequence = InputSequence::new(vec![InputEvent::PointerMove(point)])
        .expect("the ordinary benchmark sequence is bounded");
    let started = Instant::now();
    let receipt = flow
        .session
        .send_input(
            &InputRequest::new(
                flow.target,
                sequence,
                DeliveryPlan::require(InputDelivery::WindowMessage),
            )
            .with_focus(FocusPolicy::Preserve),
            &bounded(OPERATION_WAIT),
        )
        .expect("ordinary exact-window submission returns a receipt");
    let elapsed = started.elapsed();
    let correct =
        complete_ordinary_receipt(&receipt, flow.target, 1) && flow.fixture.next_pointer_move();
    Sample::unmapped(elapsed, correct)
}

#[cfg(windows)]
fn ordinary_window_button_submission(flow: &OrdinaryInputFlow) -> Sample {
    let point = Point::new(CoordinateSpace::TargetNormalized, 0.5, 0.5)
        .expect("the ordinary benchmark point is normalized");
    let sequence = InputSequence::new(vec![
        InputEvent::PointerMove(point),
        InputEvent::PointerPress(PointerButton::Primary),
    ])
    .expect("the ordinary button benchmark sequence is bounded");
    let started = Instant::now();
    let receipt = flow
        .session
        .send_input(
            &InputRequest::new(
                flow.target,
                sequence,
                DeliveryPlan::require(InputDelivery::WindowMessage),
            )
            .with_focus(FocusPolicy::Preserve),
            &bounded(OPERATION_WAIT),
        )
        .expect("ordinary button submission returns a receipt");
    let elapsed = started.elapsed();
    let correct = complete_ordinary_receipt(&receipt, flow.target, 2)
        && flow.fixture.next_pointer_button_press();
    Sample::unmapped(elapsed, correct)
}

#[cfg(windows)]
fn ordinary_window_max_sequence(flow: &OrdinaryInputFlow) -> Sample {
    let point = Point::new(CoordinateSpace::TargetNormalized, 0.5, 0.5)
        .expect("the maximum-sequence benchmark point is normalized");
    let sequence = InputSequence::new(
        std::iter::repeat_n(InputEvent::PointerMove(point), SequenceLimits::MAX_EVENTS).collect(),
    )
    .expect("the maximum ordinary sequence is accepted");
    let started = Instant::now();
    let receipt = flow
        .session
        .send_input(
            &InputRequest::new(
                flow.target,
                sequence,
                DeliveryPlan::require(InputDelivery::WindowMessage),
            )
            .with_focus(FocusPolicy::Preserve),
            &bounded(OPERATION_WAIT),
        )
        .expect("maximum ordinary submission returns a receipt");
    let elapsed = started.elapsed();
    let correct = complete_ordinary_receipt(&receipt, flow.target, SequenceLimits::MAX_EVENTS)
        && flow.fixture.next_pointer_moves(SequenceLimits::MAX_EVENTS);
    Sample::unmapped(elapsed, correct)
}

#[cfg(windows)]
fn complete_ordinary_receipt(receipt: &InputReceipt, target: TargetId, submitted: usize) -> bool {
    receipt.target() == target
        && receipt.outcome() == SequenceOutcome::Complete
        && receipt.selected_route() == Some(InputDelivery::WindowMessage)
        && receipt.address_scope() == Some(InputAddressScope::ExactWindow)
        && receipt.submitted() == submitted
        && receipt.last_submitted() == submitted.checked_sub(1)
        && receipt.evidence() == Some(SubmissionEvidence::TargetQueueAdmission)
        && receipt.attempts().len() == 1
        && !receipt.used_fallback()
        && !receipt.partial_native_effect()
        && receipt.fault().is_none()
        && receipt.cleanup() == CleanupState::NotNeeded
}

#[cfg(windows)]
fn language_common_flow(program: &LanguageProgram) -> Sample {
    let fixture = FixtureProcess::spawn(FixtureBehavior::Animate);
    let title = fixture.title();
    let started = Instant::now();
    let mut command = program.command();
    command.command().arg(title);
    let BoundedChildOutput {
        status,
        stdout,
        stderr,
        within_bounds,
    } = command.bounded_output();
    let executable_unchanged = command.executable_is_unchanged();
    let process_succeeded = within_bounds && status.is_some_and(|status| status.success());
    let stderr_empty = within_bounds && stderr.is_empty();
    let stdout = within_bounds
        .then(|| String::from_utf8(stdout).ok())
        .flatten();
    let receipt_present = stdout
        .as_deref()
        .is_some_and(|stdout| stdout.lines().any(|line| line == program.receipt_line));
    let fixture_acknowledged = if process_succeeded || receipt_present {
        fixture.next_flow(program.expected_fixture_events())
    } else {
        false
    };
    let elapsed = started.elapsed();
    let fixture_sequence_complete = fixture.finish_observation();
    let mapped = stdout
        .as_deref()
        .and_then(language_mapping_bytes)
        .unwrap_or(0);
    let peak_resident = stdout.as_deref().and_then(language_peak_resident_bytes);
    let correct = executable_unchanged
        && process_succeeded
        && stderr_empty
        && fixture_acknowledged
        && fixture_sequence_complete
        && receipt_present
        && stdout.as_deref().is_some_and(|stdout| {
            language_abi_line_is_present(stdout, program.example_name)
                && stdout
                    .lines()
                    .any(|line| line == format!("{} complete", program.example_name))
        })
        && mapped > 0
        && peak_resident.is_some_and(|bytes| bytes > 0);
    let sample = Sample::new(elapsed, correct, mapped);
    match peak_resident {
        Some(bytes) => sample.with_peak_resident_bytes(bytes),
        None => sample,
    }
}

#[cfg(windows)]
fn send_confirmed_stimulus(flow: &Flow, session: &Session) -> bool {
    assert!(
        send_key_pair(session),
        "the frame stimulus returns a complete receipt"
    );
    assert!(
        flow.fixture.next_key_pair(),
        "the fixture observes the frame stimulus's balanced key pair"
    );
    true
}

#[cfg(windows)]
fn send_resize_stimulus(_flow: &Flow, session: &Session, geometry: u64) -> bool {
    let coordinate = if geometry.is_multiple_of(2) {
        24.0
    } else {
        48.0
    };
    let point = Point::new(CoordinateSpace::TargetLogical, coordinate, coordinate)
        .expect("the resize stimulus point is finite");
    let sequence =
        InputSequence::new(vec![InputEvent::PointerMove(point)]).expect("the move is valid");
    let sent = send_sequence(session, sequence, 1);
    sent && _flow.fixture.next_pointer_move()
}

#[cfg(windows)]
fn open_benchmark_capture_session(flow: &Flow) -> Session {
    flow.open_input_session()
}

#[cfg(windows)]
const fn input_fixture_behavior() -> FixtureBehavior {
    FixtureBehavior::Animate
}

#[cfg(windows)]
const fn pressure_fixture_behavior() -> FixtureBehavior {
    FixtureBehavior::AnimateAndResize
}

#[cfg(windows)]
fn enforce_premeasurement_budgets(set: WorkloadSet, workloads: &[Workload]) {
    let (latency, heap_limit, resident_limit, stale_limit) = match set {
        WorkloadSet::ProductionCapture1280 => (
            PHASE2_WINDOWS_PRODUCTION_1280_LATENCY_BUDGETS.as_slice(),
            PHASE2_WINDOWS_PRODUCTION_1280_HEAP_LIMIT_BYTES,
            PHASE2_WINDOWS_PRODUCTION_1280_RESIDENT_LIMIT_BYTES,
            Some(PHASE2_WINDOWS_PRODUCTION_1280_STALE_WORK_LIMIT),
        ),
        WorkloadSet::ProductionCaptureDual4k => (
            PHASE2_WINDOWS_PRODUCTION_DUAL_4K_LATENCY_BUDGETS.as_slice(),
            PHASE2_WINDOWS_PRODUCTION_DUAL_4K_HEAP_LIMIT_BYTES,
            PHASE2_WINDOWS_PRODUCTION_DUAL_4K_RESIDENT_LIMIT_BYTES,
            Some(PHASE2_WINDOWS_PRODUCTION_DUAL_4K_STALE_WORK_LIMIT),
        ),
        WorkloadSet::ProductionTransitions1280 => (
            PHASE2_WINDOWS_PRODUCTION_TRANSITION_1280_LATENCY_BUDGETS.as_slice(),
            PHASE2_WINDOWS_PRODUCTION_TRANSITION_1280_HEAP_LIMIT_BYTES,
            PHASE2_WINDOWS_PRODUCTION_1280_RESIDENT_LIMIT_BYTES,
            None,
        ),
        WorkloadSet::Capture | WorkloadSet::Transitions | WorkloadSet::Input => return,
    };
    enforce_latency_budgets(workloads, latency);
    for workload in workloads {
        assert!(
            workload.peak_allocated_bytes() <= heap_limit,
            "{} exceeded the accepted Windows production live Rust heap ceiling: {} bytes",
            workload.name(),
            workload.peak_allocated_bytes(),
        );
        if let Some(resident) = workload.peak_resident_bytes() {
            assert!(
                resident <= resident_limit,
                "{} exceeded the accepted Windows production resident ceiling: {resident} bytes",
                workload.name(),
            );
        }
        if let Some(limit) = stale_limit
            && let Some(ratio) = workload.stale_work_ratio()
        {
            assert!(
                ratio <= limit,
                "{} exceeded the accepted Windows production stale-work ceiling: {ratio}",
                workload.name(),
            );
        }
    }
    match set {
        WorkloadSet::ProductionCapture1280 => {
            let callback = workloads
                .iter()
                .find(|workload| workload.name() == "callback_copy")
                .expect("the accepted Windows capture profile includes callback_copy");
            assert_eq!(
                callback.copied_bytes(),
                Some(PHASE2_WINDOWS_PRODUCTION_1280_COPIED_BYTES_LIMIT),
                "callback_copy exceeded one exact 1280x720 producer-surface copy"
            );
            nonzero_at_most(
                "callback_copy detached-texture peak",
                callback.detached_textures_peak(),
                PHASE2_WINDOWS_PRODUCTION_1280_DETACHED_TEXTURES_LIMIT,
            );
            nonzero_at_most(
                "callback_copy staging-texture peak",
                callback.staging_textures_peak(),
                PHASE2_WINDOWS_PRODUCTION_1280_STAGING_TEXTURES_LIMIT,
            );
            nonzero_at_most(
                "callback_copy total GPU-resource peak",
                callback.gpu_resources_peak(),
                PHASE2_WINDOWS_PRODUCTION_1280_GPU_RESOURCES_LIMIT,
            );
        }
        WorkloadSet::ProductionCaptureDual4k => {
            for workload in workloads {
                let copied = workload
                    .copied_bytes()
                    .expect("each accepted dual-4K workload reports callback-copy bytes");
                assert!(
                    copied <= PHASE2_WINDOWS_PRODUCTION_DUAL_4K_COPIED_BYTES_LIMIT,
                    "{} exceeded the accepted dual-4K callback-copy ceiling: {copied} bytes",
                    workload.name(),
                );
                let detached = workload
                    .detached_textures_peak()
                    .expect("each accepted dual-4K workload reports detached textures");
                assert!(
                    detached <= PHASE2_WINDOWS_PRODUCTION_DUAL_4K_DETACHED_TEXTURES_LIMIT,
                    "{} exceeded the accepted dual-4K detached-texture ceiling: {detached}",
                    workload.name(),
                );
                let staging = workload
                    .staging_textures_peak()
                    .expect("each accepted dual-4K workload reports staging textures");
                assert!(
                    staging <= PHASE2_WINDOWS_PRODUCTION_DUAL_4K_STAGING_TEXTURES_LIMIT,
                    "{} exceeded the accepted dual-4K staging-texture ceiling: {staging}",
                    workload.name(),
                );
                let resources = workload
                    .gpu_resources_peak()
                    .expect("each accepted dual-4K workload reports total GPU resources");
                assert!(
                    resources <= PHASE2_WINDOWS_PRODUCTION_DUAL_4K_GPU_RESOURCES_LIMIT,
                    "{} exceeded the accepted dual-4K total GPU-resource ceiling: {resources}",
                    workload.name(),
                );
            }
        }
        WorkloadSet::ProductionTransitions1280
        | WorkloadSet::Capture
        | WorkloadSet::Transitions
        | WorkloadSet::Input => {}
    }
}

#[cfg(windows)]
const fn benchmark_phase(_set: WorkloadSet) -> &'static str {
    "2"
}

#[cfg(windows)]
fn profile_notes(_set: WorkloadSet, notes: &str) -> String {
    notes.to_owned()
}

#[cfg(windows)]
const fn fixture_build_profile() -> &'static str {
    "fixture cargo build --release"
}
#[cfg(windows)]
const fn measured_close_bound() -> Duration {
    CLOSE_WAIT
}

#[cfg(windows)]
fn fixture_sources(manifest: &Path, set: WorkloadSet) -> Vec<PathBuf> {
    let mut sources = vec![
        manifest.join("../platform/windows/src/bin/mado-pilot-windows-input-fixture.rs"),
        manifest.join("../platform/windows/src/fixture_protocol.rs"),
    ];
    if set == WorkloadSet::Input {
        sources.push(
            manifest
                .join("../platform/windows/src/bin/mado-pilot-windows-window-message-fixture.rs"),
        );
    }
    sources
}

#[cfg(windows)]
fn native_engine() -> NativeEngine {
    mado_pilot::windows_engine(NativeEngineRequest::new())
        .expect("the Windows benchmark engine builds")
}
#[cfg(windows)]
const fn c_example_name() -> &'static str {
    "windows-native-input"
}

#[cfg(windows)]
const fn cpp_example_name() -> &'static str {
    "windows-native-input-cpp"
}

#[cfg(windows)]
const fn cpp_correctness_oracle() -> &'static str {
    "the released C++ wrapper submits the full bounded pointer, button, wheel, key, modifier, text, and delay flow through a fresh process"
}

#[cfg(windows)]
const fn cpp_receipt_line() -> &'static str {
    "receipt: outcome 1 submitted 16 evidence 4 cleanup 0"
}

#[cfg(windows)]
fn require_permissions(_engine: &NativeEngine) {}

#[cfg(windows)]
const fn input_delivery() -> InputDelivery {
    InputDelivery::WindowMessage
}

#[cfg(windows)]
const fn focus_policy() -> FocusPolicy {
    FocusPolicy::Preserve
}

#[cfg(windows)]
const fn expected_key_units() -> u32 {
    0
}

#[cfg(windows)]
const fn benchmark_fill_rgb() -> u32 {
    protocol::BENCHMARK_FILL_RGB
}

#[cfg(windows)]
fn ready_line_is_approved(line: &str, process_id: u32) -> bool {
    line.trim()
        == format!(
            "fixture-ready class={} title={} capacity={}",
            protocol::CLASS_NAME,
            protocol::fixture_title(process_id),
            protocol::MAX_RECORDED_EVENTS,
        )
}

#[cfg(windows)]
fn ordinary_ready_line_is_approved(line: &str, title: &str) -> bool {
    line.trim()
        == format!(
            "fixture-ready class={} title={} capacity={}",
            protocol::ORDINARY_CLASS_NAME,
            title,
            protocol::MAX_RECORDED_EVENTS,
        )
}

#[cfg(windows)]
fn profile_identity(set: WorkloadSet) -> (&'static str, String) {
    let id = match set {
        WorkloadSet::Capture => "phase-2-native-capture-x86_64-pc-windows-msvc",
        WorkloadSet::Transitions => "phase-2-native-transitions-x86_64-pc-windows-msvc",
        WorkloadSet::Input => "phase-2-native-input-x86_64-pc-windows-msvc",
        WorkloadSet::ProductionCapture1280 => {
            "phase-2-production-capture-1280x720-x86_64-pc-windows-msvc"
        }
        WorkloadSet::ProductionCaptureDual4k => {
            "phase-2-production-capture-dual-4k-x86_64-pc-windows-msvc"
        }
        WorkloadSet::ProductionTransitions1280 => {
            "phase-2-production-transitions-1280x720-x86_64-pc-windows-msvc"
        }
    };
    let fixture = if set == WorkloadSet::Input {
        "crates/platform/windows dedicated and ordinary fixture Rust sources plus shared protocol"
    } else {
        "crates/platform/windows fixture Rust and protocol sources"
    };
    (id, fixture.to_owned())
}
