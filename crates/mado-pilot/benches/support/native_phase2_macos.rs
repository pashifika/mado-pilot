// macOS-specific implementation for the Phase 2 native benchmark.

#[cfg(target_os = "macos")]
use std::ops::Deref;
#[cfg(target_os = "macos")]
use std::os::unix::net::UnixStream;
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
#[cfg(target_os = "macos")]
static FIXTURE_FINALIZATION_SUCCEEDED: AtomicBool = AtomicBool::new(true);
#[cfg(target_os = "macos")]
static LANGUAGE_IDENTITY_FAILURE_REPORTED: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "macos")]
static LANGUAGE_OUTPUT_FAILURE_REPORTED: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "macos")]
use mado_pilot::{ActivityTag, FrameStamp};
#[cfg(target_os = "macos")]
use mado_pilot_testkit::bench_harness::bounded_child_output_checked;
#[cfg(target_os = "macos")]
use mado_pilot_testkit::bench_harness::{
    PHASE2_2_CAPTURE_LATENCY_BUDGETS, PHASE2_2_PROCESS_APPKIT_LATENCY_BUDGETS,
    PHASE2_2_PROCESS_DIAGNOSTIC_LATENCY_BUDGETS, PHASE2_2_PROCESS_GAME_LIKE_LATENCY_BUDGETS,
    PHASE2_2_PROCESS_HEAP_LIMIT_BYTES, PHASE2_2_TRANSITION_LATENCY_BUDGETS,
    PHASE2_PRODUCTION_CAPTURE_HEAP_LIMIT_BYTES, PHASE2_PRODUCTION_CAPTURE_LATENCY_BUDGETS,
    PHASE2_PRODUCTION_MAPPED_BYTES_LIMIT, PHASE2_PRODUCTION_TRANSITION_HEAP_LIMIT_BYTES,
    PHASE2_PRODUCTION_TRANSITION_LATENCY_BUDGETS, enforce_latency_budgets,
};

#[cfg(target_os = "macos")]
use mado_pilot_backend_opencv::OpenCvBackend;
#[cfg(target_os = "macos")]
use mado_pilot_platform_macos::{
    MacosCaptureProvider, MacosPermissionProbe,
    fixture_control::{
        AuthenticatedFixtureProcess, DesktopInputState, ExecutableIdentity,
        authenticate_fixture_peer, desktop_input_state, executable_identity,
        process_executable_identity,
    },
};
#[cfg(target_os = "macos")]
use mado_pilot_runtime::{
    CaptureProvider, EngineOptions, EngineWiring, IdentityIssuer, InputProvider, Matcher,
    PackageLoader, PermissionProbe,
};

#[cfg(target_os = "macos")]
use crate::macos_fixture::{
    CancellationObservation, CommandAcknowledgement, FixtureController, FixtureFinalization,
    LanguageExecutablePin, LaunchMode, auxiliary_window_setup_is_proven,
    controlled_resize_logical_size_matches, expected_controlled_resize_logical_size,
    language_pins_are_unchanged, post_use_identity_gate,
};
#[cfg(target_os = "macos")]
use crate::macos_fixture_protocol as protocol;
#[cfg(target_os = "macos")]
use mado_pilot::{
    CancellationToken, CapabilitySupport, CoordinateSpace, DiagnosticDrain, DiagnosticKind,
    DiagnosticLevel, DiagnosticOptions, DiagnosticReader, InputAddressScope, InputFault,
    InputReceipt, Point, PointerGeometry, SubmissionEvidence, TargetKind,
};
#[cfg(target_os = "macos")]
struct NativeEngine {
    engine: Engine,
    provider: Arc<MacosCaptureProvider>,
}

#[cfg(target_os = "macos")]
impl NativeEngine {
    fn authenticates_fixture_target(
        &self,
        target: TargetId,
        process: AuthenticatedFixtureProcess,
    ) -> bool {
        self.provider
            .fixture_target_has_authenticated_owner(target, |owner| {
                process.matches_live_owner(owner)
            })
    }
}

#[cfg(target_os = "macos")]
impl Deref for NativeEngine {
    type Target = Engine;

    fn deref(&self) -> &Self::Target {
        &self.engine
    }
}

#[cfg(target_os = "macos")]
const FIXTURE_COMMAND_BOUND: Duration = Duration::from_millis(500);

#[cfg(target_os = "macos")]
const LANGUAGE_PROCESS_WAIT: Duration = Duration::from_secs(5);
#[cfg(target_os = "macos")]
const fn language_event(kind: u32, text_units: u32) -> protocol::EventSummary {
    protocol::EventSummary {
        kind,
        text_units,
        correlation: 0,
    }
}

#[cfg(target_os = "macos")]
const CPP_LANGUAGE_EVENTS: [protocol::EventSummary; 5] = [
    COMMON_LANGUAGE_EVENTS[0],
    COMMON_LANGUAGE_EVENTS[1],
    COMMON_LANGUAGE_EVENTS[2],
    language_event(protocol::EVENT_KEY_DOWN, 1),
    language_event(protocol::EVENT_KEY_UP, 1),
];

#[cfg(target_os = "macos")]
const fn input_workload_description() -> &'static str {
    "native Rust System input plus released C and C++ ProcessDirected common flows"
}

#[cfg(target_os = "macos")]
const fn input_queue_policy() -> &'static str {
    "session latest-wins queue depth 1; Rust System and C/C++ explicit ProcessDirected sequences execute serially with no fallback"
}

#[cfg(target_os = "macos")]
const fn capture_workload_description() -> &'static str {
    "native controlled capture, static retention, latest acquisition, and explicit CPU mapping"
}

#[cfg(target_os = "macos")]
const fn transition_workload_description() -> &'static str {
    "native controlled open, retained-pressure recovery, resize, and close transitions"
}

#[cfg(target_os = "macos")]
const fn capture_queue_policy() -> &'static str {
    "fixture command queue depth 1; session latest-wins queue depth 1; adapter finite retained-storage limit"
}

#[cfg(target_os = "macos")]
const fn transition_queue_policy() -> &'static str {
    "fixture command queue depth 1; session latest-wins queue depth 1; retained-pressure case fills the reported finite storage limit"
}

#[cfg(target_os = "macos")]
struct FixtureProcess {
    controller: Mutex<FixtureController>,
}

#[cfg(target_os = "macos")]
impl FixtureProcess {
    fn spawn(behavior: FixtureBehavior) -> Self {
        let controller = FixtureController::start(
            &arguments().fixture_executable,
            Arc::clone(&arguments().fixture_executable_bytes),
            arguments().fixture_executable_identity,
            behavior.launch_mode(),
            FIXTURE_WAIT,
        )
        .unwrap_or_else(|error| panic!("the benchmark fixture could not start: {error}"));
        Self {
            controller: Mutex::new(controller),
        }
    }

    fn process_id(&self) -> u32 {
        self.with_controller(|controller| controller.process_id())
    }

    fn authenticated_process(&self) -> Option<AuthenticatedFixtureProcess> {
        self.with_controller(|controller| controller.authenticated_process())
    }

    fn title(&self) -> String {
        protocol::fixture_title(self.process_id())
    }

    fn launch_mode(&self) -> LaunchMode {
        self.with_controller(|controller| controller.launch_mode())
    }

    fn command(
        &self,
        kind: protocol::FixtureCommandKind,
        wait: Duration,
    ) -> Result<CommandAcknowledgement, String> {
        self.with_controller(|controller| controller.command(kind, wait))
    }

    fn begin_flow(&self, event_payload_tag: u64) -> bool {
        self.with_controller(|controller| {
            controller.reset_events(event_payload_tag, OPERATION_WAIT)
        })
    }

    fn discard_setup_events(&self) -> bool {
        self.with_controller(|controller| controller.discard_setup_events(OPERATION_WAIT))
    }

    fn begin_process_flow(&self, event_payload_tag: u64) -> Option<DesktopInputState> {
        if !self.begin_flow(event_payload_tag) {
            return None;
        }
        desktop_input_state().ok()
    }

    fn process_environment_unchanged(&self, baseline: DesktopInputState) -> bool {
        desktop_input_state().is_ok_and(|observed| observed == baseline)
    }

    fn cancel_after_event(
        &self,
        expected: protocol::EventSummary,
        cancellation: CancellationToken,
        wait: Duration,
    ) -> Result<thread::JoinHandle<CancellationObservation>, String> {
        self.with_controller(|controller| {
            controller.cancel_after_event(expected, cancellation, wait)
        })
    }

    fn next_flow(&self, expected: &[protocol::EventSummary]) -> bool {
        self.with_controller(|controller| controller.events_are_exact(expected, OPERATION_WAIT))
    }

    fn finish_flow_after_prefix(
        &self,
        expected_remaining: &[protocol::EventSummary],
        expected_total: &[protocol::EventSummary],
    ) -> bool {
        self.with_controller(|controller| {
            controller.remaining_events_are_exact(
                expected_remaining,
                expected_total,
                OPERATION_WAIT,
            )
        })
    }

    fn close_bounded(&self, wait: Duration) -> FixtureFinalization {
        self.with_controller(|controller| controller.finish(wait))
    }

    fn next_key_pair(&self) -> bool {
        let units = expected_key_units();
        self.next_key_pair_with_units(units, units)
    }

    fn next_key_pair_with_units(&self, down_units: u32, up_units: u32) -> bool {
        self.next_flow(&[
            protocol::EventSummary {
                kind: protocol::EVENT_KEY_DOWN,
                text_units: down_units,
                correlation: 0,
            },
            protocol::EventSummary {
                kind: protocol::EVENT_KEY_UP,
                text_units: up_units,
                correlation: 0,
            },
        ])
    }

    fn with_controller<T>(&self, operation: impl FnOnce(&mut FixtureController) -> T) -> T {
        let mut controller = self
            .controller
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        operation(&mut controller)
    }
}

#[cfg(target_os = "macos")]
impl Drop for FixtureProcess {
    fn drop(&mut self) {
        let controller = self
            .controller
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !controller.finish(FIXTURE_WAIT).is_accepted() {
            FIXTURE_FINALIZATION_SUCCEEDED.store(false, Ordering::Release);
        }
    }
}

#[cfg(target_os = "macos")]
fn language_process_identity_matches(process_id: u32, expected: ExecutableIdentity) -> bool {
    let observed = process_executable_identity(process_id);
    let matches = observed == Ok(expected);
    if !matches && !LANGUAGE_IDENTITY_FAILURE_REPORTED.swap(true, Ordering::AcqRel) {
        match observed {
            Ok(_) => eprintln!("language child identity rejected: live identity mismatch"),
            Err(error) => eprintln!("language child identity rejected: {error}"),
        }
    }
    matches
}

#[cfg(target_os = "macos")]
fn capture_workloads(plan: Plan) -> Vec<Workload> {
    vec![
        measure(
            "fixture_command_acknowledgement",
            "one correlated private command acknowledgement has status success and stable window identity; it is neither a product receipt nor a visual outcome",
            plan,
            || Rc::new(FixtureProcess::spawn(FixtureBehavior::Static)),
            fixture_command_acknowledgement,
        ),
        measure(
            "controlled_stimulus_to_frame",
            "one private command acknowledgement is followed independently by a strictly newer same-stream frame carrying exactly the requested approved fill",
            plan,
            || {
                ActiveFlow::from_capture_fixture(Rc::new(FixtureProcess::spawn(
                    FixtureBehavior::Static,
                )))
            },
            stimulus_to_frame,
        ),
        measure(
            "static_latest_retained",
            "without a private transition, latest returns a same-stream frame no older than the retained frame and preserves the declared pixels",
            plan,
            || {
                ActiveFlow::from_capture_fixture(Rc::new(FixtureProcess::spawn(
                    FixtureBehavior::Static,
                )))
            },
            static_latest_retained,
        ),
        measure(
            "static_newer_repeated_pixels",
            "without a private transition, a bounded strictly-newer request evaluates the exact later frame and observes the same declared pixels",
            plan,
            || {
                ActiveFlow::from_capture_fixture(Rc::new(FixtureProcess::spawn(
                    FixtureBehavior::Static,
                )))
            },
            static_newer_repeated_pixels,
        ),
        measure(
            "latest_acquisition",
            "after an acknowledged controlled publication advances the producer, latest returns a same-stream frame no older than that publication and reports the observed sequence gap",
            plan,
            || {
                ActiveFlow::from_capture_fixture(Rc::new(FixtureProcess::spawn(
                    FixtureBehavior::Static,
                )))
            },
            latest_acquisition,
        ),
        measure(
            "cpu_map_bgra8",
            "the independently observed newer frame maps once to exact-size BGRA8 bytes carrying one declared fixture fill",
            plan,
            || {
                ActiveFlow::from_capture_fixture(Rc::new(FixtureProcess::spawn(
                    FixtureBehavior::Static,
                )))
            },
            cpu_map,
        ),
    ]
}

#[cfg(target_os = "macos")]
fn production_capture_workloads(plan: Plan) -> Vec<Workload> {
    vec![
        measure(
            "publication_age",
            "one naturally published production frame reports bounded age in the engine clock domain, advances the same stream, and carries exact fixture content and frame-authoritative geometry",
            plan,
            || {
                ActiveFlow::from_capture_fixture(Rc::new(FixtureProcess::spawn(
                    FixtureBehavior::Static,
                )))
            },
            production_publication_age,
        ),
        measure(
            "steady_frame_acquisition",
            "one strictly newer production frame arrives without input or private stimulus, preserves exact fixture content, and reports observable sequence gaps",
            plan,
            || {
                ActiveFlow::from_capture_fixture(Rc::new(FixtureProcess::spawn(
                    FixtureBehavior::Static,
                )))
            },
            production_steady_acquisition,
        ),
        measure(
            "latest_acquisition",
            "after natural production progress, latest returns a same-stream frame no older than the proven publication and reports observable sequence gaps",
            plan,
            || {
                ActiveFlow::from_capture_fixture(Rc::new(FixtureProcess::spawn(
                    FixtureBehavior::Static,
                )))
            },
            production_latest_acquisition,
        ),
        measure(
            "cpu_map_bgra8",
            "one naturally published production frame maps once to exact-size BGRA8 bytes carrying declared fixture content",
            plan,
            || {
                ActiveFlow::from_capture_fixture(Rc::new(FixtureProcess::spawn(
                    FixtureBehavior::Static,
                )))
            },
            production_cpu_map,
        ),
        measure(
            "retained_pressure_resume",
            "natural production fills the reported retained limit; one blocked publication remains observable and releasing one slot resumes with a sequence gap",
            plan,
            || Rc::new(FixtureProcess::spawn(FixtureBehavior::Static)),
            production_retained_pressure_resume,
        ),
    ]
}

#[cfg(target_os = "macos")]
fn resize_allocation_workloads(plan: Plan) -> Vec<Workload> {
    vec![
        measure(
            "fixture_resize_command",
            "one private resize command is acknowledged by the separately running fixture",
            plan,
            || Rc::new(FixtureProcess::spawn(FixtureBehavior::Static)),
            fixture_resize_command,
        ),
        measure(
            "resize_recreation",
            "one acknowledged private resize advances epoch and geometry and returns the resized \
             extent independently of the acknowledgement",
            plan,
            || {
                ActiveFlow::from_capture_fixture(Rc::new(FixtureProcess::spawn(
                    FixtureBehavior::Static,
                )))
            },
            resize_recreation,
        ),
    ]
}

#[cfg(target_os = "macos")]
fn transition_workloads(plan: Plan) -> Vec<Workload> {
    let fixture = Rc::new(FixtureProcess::spawn(FixtureBehavior::Static));
    vec![
        measure(
            "resize_recreation",
            "one acknowledged private resize advances epoch and geometry and returns the resized extent independently of the acknowledgement",
            plan,
            || ActiveFlow::from_capture_fixture(Rc::clone(&fixture)),
            resize_recreation,
        ),
        measure(
            "open_first_frame",
            "each fresh capture-only session returns a correctly identified deterministic first frame and closes",
            plan,
            || Flow::from_fixture(Rc::clone(&fixture)),
            open_first_frame,
        ),
        measure(
            "retained_pressure_resume",
            "controlled publications fill the reported retained limit; releasing one slot resumes with an observable sequence gap",
            plan,
            || (),
            retained_pressure_resume,
        ),
        measure(
            "close_drain",
            "explicit capture-session close reaches the closed state and remains idempotent",
            plan,
            || Flow::from_fixture(Rc::clone(&fixture)),
            close_drain,
        ),
    ]
}

#[cfg(target_os = "macos")]
fn production_transition_workloads(plan: Plan) -> Vec<Workload> {
    let fixture = Rc::new(FixtureProcess::spawn(FixtureBehavior::Static));
    vec![
        measure(
            "open_first_frame",
            "each fresh production capture session returns one correctly identified and mapped first frame, then closes",
            plan,
            || Flow::from_fixture(Rc::clone(&fixture)),
            open_first_frame,
        ),
        measure(
            "resize_recreation",
            "one bounded private resize stimulus drives the production capture session; only the independently observed new epoch, geometry, extent, and frame-authoritative transform satisfy the oracle",
            plan,
            || ActiveFlow::from_capture_fixture(Rc::clone(&fixture)),
            resize_recreation,
        ),
        measure(
            "close_drain",
            "explicit production capture close drains within its bound, reaches the closed state, and remains idempotent",
            plan,
            || Flow::from_fixture(Rc::clone(&fixture)),
            close_drain,
        ),
    ]
}

#[cfg(target_os = "macos")]
const PROCESS_DIAGNOSTIC_CAPACITY: usize = 64;
#[cfg(target_os = "macos")]
const PROCESS_OVERFLOW_CAPACITY: usize = 4;
#[cfg(target_os = "macos")]
const PROCESS_OVERFLOW_SUBMISSIONS: usize = 4;

#[cfg(target_os = "macos")]
const POINTER_EVENT: protocol::EventSummary = protocol::EventSummary {
    kind: protocol::EVENT_POINTER_MOVE,
    text_units: 0,
    correlation: 0,
};

#[cfg(target_os = "macos")]
static NEXT_PROCESS_CORRELATION: AtomicU32 = AtomicU32::new(1);

#[cfg(target_os = "macos")]
fn process_row_activity(fingerprints: &[u64]) -> (ActivityTag, u32) {
    let correlation = NEXT_PROCESS_CORRELATION.fetch_add(1, Ordering::Relaxed);
    assert_ne!(correlation, 0, "benchmark row correlation exhausted");
    let value = protocol::event_payload_activity_tag(correlation, fingerprints);
    (
        ActivityTag::new(value).expect("benchmark row activity tag is nonzero"),
        correlation,
    )
}

#[cfg(target_os = "macos")]
const fn correlated_event(
    event: protocol::EventSummary,
    correlation: u32,
) -> protocol::EventSummary {
    protocol::EventSummary {
        correlation,
        ..event
    }
}

#[cfg(target_os = "macos")]
fn process_pointer_fingerprint(x: f64, y: f64) -> u64 {
    protocol::event_payload_fingerprint(
        protocol::EVENT_POINTER_MOVE,
        5,
        0,
        0,
        0,
        x,
        y,
        0,
        0,
        0,
        &[],
    )
}

#[cfg(target_os = "macos")]
fn process_enter_fingerprints() -> [u64; 2] {
    [
        protocol::event_payload_fingerprint(
            protocol::EVENT_KEY_DOWN,
            10,
            0,
            0,
            0,
            0.0,
            0.0,
            0,
            0,
            0x24,
            &[],
        ),
        protocol::event_payload_fingerprint(
            protocol::EVENT_KEY_UP,
            11,
            0,
            0,
            0,
            0.0,
            0.0,
            0,
            0,
            0x24,
            &[],
        ),
    ]
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessDiagnosticCase {
    Off,
    Normal,
    Debug,
    Overflow,
}

#[cfg(target_os = "macos")]
impl ProcessDiagnosticCase {
    fn options(self) -> DiagnosticOptions {
        match self {
            Self::Off => DiagnosticOptions::off(),
            Self::Normal => DiagnosticOptions::normal(PROCESS_DIAGNOSTIC_CAPACITY)
                .expect("the process diagnostic capacity is valid"),
            Self::Debug => DiagnosticOptions::debug(PROCESS_DIAGNOSTIC_CAPACITY)
                .expect("the process diagnostic capacity is valid"),
            Self::Overflow => DiagnosticOptions::debug(PROCESS_OVERFLOW_CAPACITY)
                .expect("the process overflow capacity is valid"),
        }
    }

    const fn submissions(self) -> usize {
        match self {
            Self::Overflow => PROCESS_OVERFLOW_SUBMISSIONS,
            Self::Off | Self::Normal | Self::Debug => 1,
        }
    }
}

#[cfg(target_os = "macos")]
fn process_directed_workloads(plan: Plan, behavior: FixtureBehavior) -> Vec<Workload> {
    vec![
        measure(
            "discovery_open_retained_authority",
            "one fresh discovery selects the retained primary target while its process owns an additional ordinary window, exposes every explicit unknown process-scoped invocation-only pair, and opens required input before the shared deadline",
            plan,
            move || ProcessDiscoveryFlow::new(behavior),
            discovery_open_retained_authority,
        ),
        measure(
            "event_authority_preflight_post",
            "with an additional same-process window present, one pointer event revalidates retained process authority, direct post-event authorization, and production posting; its invocation-only receipt and exact process event match without fallback, then a separately acknowledged controlled transition produces a strictly newer approved frame without attributing that visual change to input",
            plan,
            move || ProcessFlow::new(behavior, ProcessDiagnosticCase::Off),
            event_authority_preflight_post,
        ),
        measure(
            "release_cleanup",
            "with an additional same-process window present, cancellation begins only after the fixture separately observes one key-down; the production receipt reports one possible effect and one bounded cleanup release, then an independent controlled transition produces a strictly newer approved frame",
            plan,
            move || ProcessFlow::new(behavior, ProcessDiagnosticCase::Off),
            release_cleanup,
        ),
        measure(
            "session_close",
            "with an additional same-process window present, a fresh process-directed session closes within its own bound and repeated close is idempotent",
            plan,
            move || ProcessCloseFlow::new(behavior),
            process_session_close,
        ),
        measure(
            "fixture_controller_close",
            "the private fixture controller acknowledges stop, closes its bounded channel, and confirms termination of the same authenticated NSWorkspace application within one shared deadline",
            plan,
            move || behavior,
            fixture_controller_close,
        ),
    ]
}

#[cfg(target_os = "macos")]
fn process_diagnostic_workloads(plan: Plan) -> Vec<Workload> {
    vec![
        measure(
            "event_diagnostics_off",
            "the process-directed invocation-only receipt and separate pointer observation are exact and no diagnostic reader exists",
            plan,
            || ProcessFlow::new(FixtureBehavior::Static, ProcessDiagnosticCase::Off),
            process_diagnostic_event,
        ),
        measure(
            "event_diagnostics_normal",
            "the invocation diagnostics drain exactly before the independently observed newer frame, which emits no normal records",
            plan,
            || ProcessFlow::new(FixtureBehavior::Static, ProcessDiagnosticCase::Normal),
            process_diagnostic_event,
        ),
        measure(
            "event_diagnostics_debug",
            "input start, route-attempt, and terminal records drain in order before the newer-frame observation drains its debug pair separately",
            plan,
            || ProcessFlow::new(FixtureBehavior::Static, ProcessDiagnosticCase::Debug),
            process_diagnostic_event,
        ),
        measure(
            "event_diagnostic_overflow",
            "four receipts and observations remain exact while input and subsequent newer-frame drains separately report every retained record and debug loss",
            plan,
            || ProcessFlow::new(FixtureBehavior::Static, ProcessDiagnosticCase::Overflow),
            process_diagnostic_event,
        ),
    ]
}

#[cfg(target_os = "macos")]
struct ProcessDiscoveryFlow {
    engine: NativeEngine,
    fixture: Rc<FixtureProcess>,
}

#[cfg(target_os = "macos")]
impl ProcessDiscoveryFlow {
    fn new(behavior: FixtureBehavior) -> Self {
        let engine = native_engine_with_diagnostics(DiagnosticOptions::off());
        require_permissions(&engine);
        let fixture = start_inactive_process_fixture(behavior, &engine);
        let _seed_target = discover_process_target(&engine, &fixture);
        Self { engine, fixture }
    }
}
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy)]
struct ProcessVisualCursor {
    stamp: FrameStamp,
    fill: u32,
}

#[cfg(target_os = "macos")]
struct ProcessFlow {
    visual: Mutex<ProcessVisualCursor>,
    _engine: NativeEngine,
    fixture: Rc<FixtureProcess>,
    session: Session,
    pointer_request: InputRequest,
    pointer_fingerprint: u64,
    reader: Option<DiagnosticReader>,
    diagnostics: ProcessDiagnosticCase,
}

#[cfg(target_os = "macos")]
impl ProcessFlow {
    fn new(behavior: FixtureBehavior, diagnostics: ProcessDiagnosticCase) -> Self {
        let engine = native_engine_with_diagnostics(diagnostics.options());
        require_permissions(&engine);
        let fixture = start_inactive_process_fixture(behavior, &engine);
        let reader = engine.take_diagnostic_reader();
        let target = discover_process_target(&engine, &fixture);
        let session = engine
            .open_session(
                target,
                &process_session_request(true),
                &bounded(OPERATION_WAIT),
            )
            .expect("the process-directed benchmark session opens");
        let seed = session
            .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
            .expect("the process-directed fixture publishes its seed frame");
        let mapping = seed
            .map(PixelFormat::Bgra8, &bounded(OPERATION_WAIT))
            .expect("the process-directed seed frame maps");
        let fill = benchmark_mapping_fill(&mapping)
            .expect("the process-directed target is the owned deterministic fixture");
        let extent = seed.descriptor().extent();
        let centre = Point::new(
            CoordinateSpace::CapturePixels,
            f64::from(extent.width()) / 2.0,
            f64::from(extent.height()) / 2.0,
        )
        .expect("the process-directed benchmark point is finite");
        let desktop = seed
            .transform()
            .convert_point(centre, CoordinateSpace::DesktopLogical)
            .expect("the process benchmark point resolves to desktop logical");
        let pointer_fingerprint = process_pointer_fingerprint(desktop.x(), desktop.y());
        let pointer_request = InputRequest::new(
            target,
            InputSequence::new(vec![InputEvent::PointerMove(centre)])
                .expect("the process-directed benchmark event is bounded"),
            DeliveryPlan::require(InputDelivery::ProcessDirected),
        )
        .with_focus(FocusPolicy::Preserve)
        .with_pointer_geometry(PointerGeometry::require_unchanged_since(seed.stamp()));
        if let Some(reader) = reader.as_ref() {
            let _ = reader.drain();
        }
        Self {
            visual: Mutex::new(ProcessVisualCursor {
                stamp: seed.stamp(),
                fill,
            }),
            _engine: engine,
            fixture,
            session,
            pointer_request,
            reader,
            diagnostics,
            pointer_fingerprint,
        }
    }
}

#[cfg(target_os = "macos")]
impl Drop for ProcessFlow {
    fn drop(&mut self) {
        let _closed = close(&self.session);
    }
}

#[cfg(target_os = "macos")]
struct ProcessCloseFlow {
    _engine: NativeEngine,
    fixture: Rc<FixtureProcess>,
    target: TargetId,
}

#[cfg(target_os = "macos")]
impl ProcessCloseFlow {
    fn new(behavior: FixtureBehavior) -> Self {
        let engine = native_engine_with_diagnostics(DiagnosticOptions::off());
        require_permissions(&engine);
        let fixture = start_inactive_process_fixture(behavior, &engine);
        let target = discover_process_target(&engine, &fixture);
        Self {
            _engine: engine,
            fixture,
            target,
        }
    }
}

#[cfg(target_os = "macos")]
fn start_inactive_process_fixture(
    behavior: FixtureBehavior,
    engine: &NativeEngine,
) -> Rc<FixtureProcess> {
    let fixture = Rc::new(FixtureProcess::spawn(behavior));
    assert_eq!(
        fixture.launch_mode(),
        behavior.launch_mode(),
        "the ready mode/renderer facts match the requested profile"
    );
    confirm_auxiliary_window_setup(engine, &fixture);
    assert!(
        controlled_command_ok(&fixture, protocol::FixtureCommandKind::YieldForeground),
        "the process-directed fixture yields foreground before sampling"
    );
    fixture
}

#[cfg(target_os = "macos")]
fn confirm_auxiliary_window_setup(engine: &NativeEngine, fixture: &FixtureProcess) {
    let acknowledged = controlled_command_ok(fixture, protocol::FixtureCommandKind::OpenAuxiliary);
    let authenticated_window_ids = if acknowledged {
        authenticated_fixture_window_inventory(engine, fixture)
    } else {
        Vec::new()
    };
    assert!(
        auxiliary_window_setup_is_proven(acknowledged, &authenticated_window_ids),
        "the acknowledged auxiliary-window setup is independently visible as two distinct \
         live authenticated windows in production discovery"
    );
}

#[cfg(target_os = "macos")]
fn confirm_transient_auxiliary_window_setup(engine: &NativeEngine, fixture: &FixtureProcess) {
    confirm_auxiliary_window_setup(engine, fixture);
    assert!(
        controlled_command_ok(fixture, protocol::FixtureCommandKind::CloseAuxiliary),
        "the proven auxiliary window closes before title-selected language sampling"
    );
    let deadline = Instant::now() + FIXTURE_WAIT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "production discovery did not retire the closed auxiliary window before language \
             sampling"
        );
        if authenticated_fixture_window_ids(engine, fixture, remaining)
            .is_some_and(|window_ids| window_ids.len() == 1)
        {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(target_os = "macos")]
fn authenticated_fixture_window_inventory(
    engine: &NativeEngine,
    fixture: &FixtureProcess,
) -> Vec<TargetId> {
    let deadline = Instant::now() + FIXTURE_WAIT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Vec::new();
        }
        if let Some(authenticated_window_ids) =
            authenticated_fixture_window_ids(engine, fixture, remaining)
            && auxiliary_window_setup_is_proven(true, &authenticated_window_ids)
        {
            return authenticated_window_ids;
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(target_os = "macos")]
fn authenticated_fixture_window_ids(
    engine: &NativeEngine,
    fixture: &FixtureProcess,
    wait: Duration,
) -> Option<Vec<TargetId>> {
    let targets = engine.discover(&bounded(wait)).ok()?;
    let process = fixture.authenticated_process()?;
    Some(
        targets
            .iter()
            .filter(|target| target.capability().kind() == Some(TargetKind::Window))
            .filter(|target| engine.authenticates_fixture_target(target.id(), process))
            .map(mado_pilot::TargetDescription::id)
            .collect(),
    )
}

#[cfg(target_os = "macos")]
fn discover_process_target(engine: &NativeEngine, fixture: &FixtureProcess) -> TargetId {
    let deadline = Instant::now() + FIXTURE_WAIT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "the process-directed benchmark fixture was not discoverable before its deadline"
        );
        let Ok(targets) = engine.discover(&bounded(remaining)) else {
            thread::sleep(Duration::from_millis(25));
            continue;
        };
        let process = fixture
            .authenticated_process()
            .expect("the process fixture control peer remains authenticated");
        if let Ok(target) =
            protocol::select_unique_fixture(&targets, process.process_id(), |target| {
                engine.authenticates_fixture_target(target, process)
            })
        {
            assert!(
                process_pairs_are_explicit(target),
                "the process-directed descriptor is explicit and truthful"
            );
            return target.id();
        }
        assert!(
            Instant::now() < deadline,
            "exactly one process-qualified fixture becomes selectable"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(target_os = "macos")]
fn process_pairs_are_explicit(target: &mado_pilot::TargetDescription) -> bool {
    target.capability().kind() == Some(TargetKind::Window)
        && InputOperationKind::ALL.iter().all(|kind| {
            let pair = target
                .capability()
                .input()
                .pair(*kind, InputDelivery::ProcessDirected);
            pair.support() == CapabilitySupport::Unknown
                && pair.address_scope() == InputAddressScope::OwningProcess
                && pair.evidence() == Some(SubmissionEvidence::InvocationOnly)
                && !pair.focus_required()
        })
}

#[cfg(target_os = "macos")]
fn process_session_request(capturing: bool) -> SessionRequest {
    let request = SessionRequest::new().requesting_input(
        InputOpenRequest::new()
            .with_requirement(InputRequirement::Required)
            .requiring(InputOperationKind::Pointer, InputDelivery::ProcessDirected)
            .requiring(InputOperationKind::Keyboard, InputDelivery::ProcessDirected),
    );
    if capturing {
        request.capturing(OpenRequest::new())
    } else {
        request
    }
}

#[cfg(target_os = "macos")]
fn discovery_open_retained_authority(flow: &ProcessDiscoveryFlow) -> Sample {
    let operation = bounded(OPERATION_WAIT);
    let started = Instant::now();
    let targets = flow
        .engine
        .discover(&operation)
        .expect("fresh retained-authority discovery completes");
    let process = flow
        .fixture
        .authenticated_process()
        .expect("the process fixture control peer remains authenticated");
    let selected = protocol::select_unique_fixture(&targets, process.process_id(), |target| {
        flow.engine.authenticates_fixture_target(target, process)
    })
    .expect("fresh discovery selects exactly the owned fixture");
    let descriptor_ok = process_pairs_are_explicit(selected);
    let session = flow
        .engine
        .open_session(selected.id(), &process_session_request(false), &operation)
        .expect("fresh retained authority opens process-directed input");
    let elapsed = started.elapsed();
    let correct = descriptor_ok
        && session.target() == selected.id()
        && session.accepts_input()
        && close(&session);
    Sample::unmapped(elapsed, correct)
}

#[cfg(target_os = "macos")]
fn observe_controlled_process_visual(flow: &ProcessFlow) -> Option<usize> {
    let before = *flow
        .visual
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let expected_fill = alternate_benchmark_fill(before.fill)
        .expect("the process fixture remains in one approved fill");

    let operation = bounded(OPERATION_WAIT);
    let mut cursor = before.stamp;
    let mut observed_frames = 0usize;
    loop {
        let Ok(frame) = flow
            .session
            .acquire_frame(&FrameRequest::newer_than(cursor), &operation)
        else {
            return None;
        };
        observed_frames = observed_frames.checked_add(1)?;
        let Ok(mapping) = frame.map(PixelFormat::Bgra8, &operation) else {
            return None;
        };
        let fill = benchmark_mapping_fill(&mapping)?;
        let stamp = frame.stamp();
        if fill == expected_fill {
            let exact = stamp.stream() == before.stamp.stream()
                && stamp.epoch() == before.stamp.epoch()
                && stamp.sequence() > before.stamp.sequence()
                && mapping.stamp() == stamp;
            if exact {
                *flow
                    .visual
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                    ProcessVisualCursor { stamp, fill };
            }
            return exact.then_some(observed_frames);
        }
        cursor = stamp;
    }
}

#[cfg(target_os = "macos")]
fn stimulate_and_observe_process_visual(flow: &ProcessFlow) -> Option<usize> {
    controlled_command_ok(&flow.fixture, protocol::FixtureCommandKind::Transition)
        .then(|| observe_controlled_process_visual(flow))
        .flatten()
}

#[cfg(target_os = "macos")]
fn event_authority_preflight_post(flow: &ProcessFlow) -> Sample {
    let (activity_tag, correlation) = process_row_activity(&[flow.pointer_fingerprint]);
    let environment = flow
        .fixture
        .begin_process_flow(activity_tag.get())
        .expect("the pointer sample starts with empty events and an observed desktop state");
    let expected = correlated_event(POINTER_EVENT, correlation);
    let started = Instant::now();
    let receipt = flow
        .session
        .send_input(
            &flow.pointer_request,
            &bounded(OPERATION_WAIT).with_activity_tag(activity_tag),
        )
        .expect("the process-directed pointer invocation returns a receipt");
    let elapsed = started.elapsed();
    let events_ok = flow.fixture.next_flow(&[expected]);
    let visual_ok = stimulate_and_observe_process_visual(flow).is_some();
    Sample::unmapped(
        elapsed,
        complete_process_receipt(&receipt, flow.session.target(), 1)
            && events_ok
            && visual_ok
            && flow.fixture.process_environment_unchanged(environment),
    )
}

#[cfg(target_os = "macos")]
fn release_cleanup(flow: &ProcessFlow) -> Sample {
    let cancellation = CancellationToken::new();
    let fingerprints = process_enter_fingerprints();
    let (activity_tag, correlation) = process_row_activity(&fingerprints);
    let environment = flow
        .fixture
        .begin_process_flow(activity_tag.get())
        .expect("the cleanup sample starts with empty events and an observed desktop state");
    let pressed = correlated_event(
        protocol::EventSummary {
            kind: protocol::EVENT_KEY_DOWN,
            text_units: expected_key_units(),
            correlation: 0,
        },
        correlation,
    );
    let observer = flow
        .fixture
        .cancel_after_event(pressed, cancellation.clone(), OPERATION_WAIT)
        .expect("the cleanup trigger owns the fixture event receiver");
    let sequence = InputSequence::new(vec![
        InputEvent::KeyPress(Key::Enter),
        InputEvent::Delay(Duration::from_secs(5)),
        InputEvent::KeyRelease(Key::Enter),
    ])
    .expect("the cleanup sequence stays within the shared limits");
    let request = InputRequest::new(
        flow.session.target(),
        sequence,
        DeliveryPlan::require(InputDelivery::ProcessDirected),
    )
    .with_focus(FocusPolicy::Preserve);
    let operation = bounded(OPERATION_WAIT)
        .with_activity_tag(activity_tag)
        .with_cancellation(cancellation);
    let receipt = flow.session.send_input(&request, &operation).ok();
    let observed = observer
        .join()
        .expect("the cleanup observation helper remains contained");

    let elapsed = observed
        .cancelled_at()
        .map_or(OPERATION_WAIT, |cancelled| cancelled.elapsed());
    let expected_release = correlated_event(
        protocol::EventSummary {
            kind: protocol::EVENT_KEY_UP,
            text_units: expected_key_units(),
            correlation: 0,
        },
        correlation,
    );
    let release_ok = flow
        .fixture
        .finish_flow_after_prefix(&[expected_release], &[pressed, expected_release]);
    let visual_ok = stimulate_and_observe_process_visual(flow).is_some();
    let correct = observed.summary() == Some(pressed)
        && receipt
            .as_ref()
            .is_some_and(|receipt| cleanup_receipt_is_exact(receipt, flow.session.target()))
        && release_ok
        && visual_ok
        && flow.fixture.process_environment_unchanged(environment);
    Sample::unmapped(elapsed, correct)
}

#[cfg(target_os = "macos")]
fn process_session_close(flow: &ProcessCloseFlow) -> Sample {
    let session = flow
        ._engine
        .open_session(
            flow.target,
            &process_session_request(false),
            &bounded(OPERATION_WAIT),
        )
        .expect("a fresh process-directed close sample opens");
    let started = Instant::now();
    let first = session.close(&bounded(Duration::from_secs(1)));
    let elapsed = started.elapsed();
    let second = session.close(&bounded(Duration::from_secs(1)));
    let correct =
        first.is_ok() && second.is_ok() && session.is_closed() && flow.fixture.process_id() != 0;
    Sample::unmapped(elapsed, correct)
}

#[cfg(target_os = "macos")]
fn fixture_controller_close(behavior: &FixtureBehavior) -> Sample {
    let fixture = FixtureProcess::spawn(*behavior);
    let started = Instant::now();
    let correct = fixture.close_bounded(measured_close_bound()).is_accepted();
    Sample::unmapped(started.elapsed(), correct)
}

#[cfg(target_os = "macos")]
fn complete_process_receipt(receipt: &InputReceipt, target: TargetId, submitted: usize) -> bool {
    receipt.target() == target
        && receipt.outcome() == SequenceOutcome::Complete
        && receipt.submitted() == submitted
        && receipt.last_submitted() == submitted.checked_sub(1)
        && receipt.selected_route() == Some(InputDelivery::ProcessDirected)
        && receipt.address_scope() == Some(InputAddressScope::OwningProcess)
        && receipt.evidence() == Some(SubmissionEvidence::InvocationOnly)
        && receipt.fault().is_none()
        && receipt.cleanup() == CleanupState::NotNeeded
        && !receipt.used_fallback()
        && !receipt.partial_native_effect()
        && receipt.attempts().len() == 1
        && receipt.attempts()[0].route() == InputDelivery::ProcessDirected
}

#[cfg(target_os = "macos")]
fn cleanup_receipt_is_exact(receipt: &InputReceipt, target: TargetId) -> bool {
    receipt.target() == target
        && receipt.outcome() == SequenceOutcome::Partial
        && receipt.submitted() == 1
        && receipt.last_submitted() == Some(0)
        && receipt.selected_route() == Some(InputDelivery::ProcessDirected)
        && receipt.address_scope() == Some(InputAddressScope::OwningProcess)
        && receipt.evidence() == Some(SubmissionEvidence::InvocationOnly)
        && receipt.fault() == Some(InputFault::Cancelled)
        && receipt.cleanup() == CleanupState::Complete
        && receipt.possible_native_effect()
        && !receipt.used_fallback()
        && receipt.attempts().len() == 1
        && receipt.attempts()[0].route() == InputDelivery::ProcessDirected
}

#[cfg(target_os = "macos")]
fn process_diagnostic_event(flow: &ProcessFlow) -> Sample {
    let submissions = flow.diagnostics.submissions();
    let fingerprints = vec![flow.pointer_fingerprint; submissions];
    let (activity_tag, correlation) = process_row_activity(&fingerprints);
    let environment = flow
        .fixture
        .begin_process_flow(activity_tag.get())
        .expect("the diagnostic sample starts with empty events and an observed desktop state");
    let mut receipts_correct = true;
    let mut slowest = Duration::ZERO;
    for _ in 0..submissions {
        let started = Instant::now();
        let receipt = flow
            .session
            .send_input(
                &flow.pointer_request,
                &bounded(OPERATION_WAIT).with_activity_tag(activity_tag),
            )
            .expect("the diagnostic process-directed invocation returns a receipt");
        slowest = slowest.max(started.elapsed());
        receipts_correct &= complete_process_receipt(&receipt, flow.session.target(), 1);
    }
    let expected_events = vec![correlated_event(POINTER_EVENT, correlation); submissions];
    let events_correct = flow.fixture.next_flow(&expected_events);
    let diagnostics_correct =
        process_diagnostics_are_exact(flow.reader.as_ref(), flow.diagnostics, submissions);
    let observed_frames = stimulate_and_observe_process_visual(flow);
    let visual_diagnostics_correct = process_visual_diagnostics_are_exact(
        flow.reader.as_ref(),
        flow.diagnostics,
        observed_frames,
    );
    Sample::unmapped(
        slowest,
        receipts_correct
            && events_correct
            && diagnostics_correct
            && observed_frames.is_some()
            && visual_diagnostics_correct
            && flow.fixture.process_environment_unchanged(environment),
    )
}

#[cfg(target_os = "macos")]
struct ExpectedProcessDrain {
    records: usize,
    normal_losses: u64,
    debug_losses: u64,
    input_records: usize,
    attempt_records: usize,
    started_records: usize,
}

#[cfg(target_os = "macos")]
fn process_diagnostics_are_exact(
    reader: Option<&DiagnosticReader>,
    diagnostics: ProcessDiagnosticCase,
    submissions: usize,
) -> bool {
    match diagnostics {
        ProcessDiagnosticCase::Off => reader.is_none(),
        ProcessDiagnosticCase::Normal => process_drain_matches(
            reader,
            ExpectedProcessDrain {
                records: submissions,
                normal_losses: 0,
                debug_losses: 0,
                input_records: submissions,
                attempt_records: 0,
                started_records: 0,
            },
        ),
        ProcessDiagnosticCase::Debug => process_drain_matches(
            reader,
            ExpectedProcessDrain {
                records: submissions * 3,
                normal_losses: 0,
                debug_losses: 0,
                input_records: submissions,
                attempt_records: submissions,
                started_records: submissions,
            },
        ),
        ProcessDiagnosticCase::Overflow => process_drain_matches(
            reader,
            ExpectedProcessDrain {
                records: PROCESS_OVERFLOW_CAPACITY,
                normal_losses: 0,
                debug_losses: (submissions * 2) as u64,
                input_records: PROCESS_OVERFLOW_CAPACITY,
                attempt_records: 0,
                started_records: 0,
            },
        ),
    }
}

#[cfg(target_os = "macos")]
fn process_drain_matches(
    reader: Option<&DiagnosticReader>,
    expected: ExpectedProcessDrain,
) -> bool {
    let Some(reader) = reader else {
        return false;
    };
    let DiagnosticDrain::Batch(batch) = reader.drain() else {
        return false;
    };
    let retained = batch.records();
    let sequences_increase = retained
        .windows(2)
        .all(|pair| pair[0].sequence() < pair[1].sequence());
    retained.len() == expected.records
        && batch.losses().normal() == expected.normal_losses
        && batch.losses().debug() == expected.debug_losses
        && retained
            .iter()
            .filter(|record| record.kind() == DiagnosticKind::Input)
            .count()
            == expected.input_records
        && retained
            .iter()
            .filter(|record| record.kind() == DiagnosticKind::RouteAttempt)
            .count()
            == expected.attempt_records
        && retained
            .iter()
            .filter(|record| {
                record.kind() == DiagnosticKind::OperationStarted
                    && record.level() == DiagnosticLevel::Debug
            })
            .count()
            == expected.started_records
        && sequences_increase
        && matches!(reader.drain(), DiagnosticDrain::OpenEmpty)
}

#[cfg(target_os = "macos")]
fn process_visual_diagnostics_are_exact(
    reader: Option<&DiagnosticReader>,
    diagnostics: ProcessDiagnosticCase,
    observed_frames: Option<usize>,
) -> bool {
    let Some(observed_frames) = observed_frames else {
        if let Some(reader) = reader {
            let _ = reader.drain();
        }
        return false;
    };
    match diagnostics {
        ProcessDiagnosticCase::Off => reader.is_none(),
        ProcessDiagnosticCase::Normal => {
            reader.is_some_and(|reader| matches!(reader.drain(), DiagnosticDrain::OpenEmpty))
        }
        ProcessDiagnosticCase::Debug => {
            process_visual_drain_matches(reader, observed_frames, PROCESS_DIAGNOSTIC_CAPACITY)
        }
        ProcessDiagnosticCase::Overflow => {
            process_visual_drain_matches(reader, observed_frames, PROCESS_OVERFLOW_CAPACITY)
        }
    }
}

#[cfg(target_os = "macos")]
fn process_visual_drain_matches(
    reader: Option<&DiagnosticReader>,
    observed_frames: usize,
    capacity: usize,
) -> bool {
    let Some(generated_records) = observed_frames
        .checked_mul(2)
        .filter(|generated| *generated > 0)
    else {
        return false;
    };
    let Some(reader) = reader else {
        return false;
    };
    let DiagnosticDrain::Batch(batch) = reader.drain() else {
        return false;
    };
    let expected_records = generated_records.min(capacity);
    let expected_debug_losses =
        u64::try_from(generated_records - expected_records).unwrap_or(u64::MAX);
    let retained = batch.records();
    retained.len() == expected_records
        && batch.losses().normal() == 0
        && batch.losses().debug() == expected_debug_losses
        && retained
            .windows(2)
            .all(|pair| pair[0].sequence() < pair[1].sequence())
        && retained.chunks_exact(2).all(|pair| {
            pair[0].level() == DiagnosticLevel::Debug
                && pair[0].kind() == DiagnosticKind::OperationStarted
                && pair[1].level() == DiagnosticLevel::Debug
                && pair[1].kind() == DiagnosticKind::Frame
                && pair[0].operation() == pair[1].operation()
        })
        && matches!(reader.drain(), DiagnosticDrain::OpenEmpty)
}

#[cfg(target_os = "macos")]
fn fixture_command_acknowledgement(fixture: &Rc<FixtureProcess>) -> Sample {
    let acknowledgement = fixture
        .command(
            protocol::FixtureCommandKind::Transition,
            FIXTURE_COMMAND_BOUND,
        )
        .expect("the bounded private transition is acknowledged");
    let result = acknowledgement.result();
    let correct = result.status == 0
        && result.before_window != 0
        && result.before_window == result.after_window;
    Sample::unmapped(acknowledgement.elapsed(), correct)
}

#[cfg(target_os = "macos")]
fn static_latest_retained(active: &ActiveFlow) -> Sample {
    let state = lock_state(active);
    let before = state.last.stamp();
    let started = Instant::now();
    let latest = state
        .session
        .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
        .expect("latest retains the static fixture frame");
    let elapsed = started.elapsed();
    let mapping = latest
        .map(PixelFormat::Bgra8, &bounded(OPERATION_WAIT))
        .expect("the retained static frame maps");
    let stamp = latest.stamp();
    let correct = stamp.stream() == before.stream()
        && (stamp.epoch() > before.epoch()
            || (stamp.epoch() == before.epoch() && stamp.sequence() >= before.sequence()))
        && benchmark_mapping_fill(&mapping) == Some(state.fill);
    Sample::new(elapsed, correct, mapping.bytes().len() as u64)
}

#[cfg(target_os = "macos")]
fn static_newer_repeated_pixels(active: &ActiveFlow) -> Sample {
    let mut state = lock_state(active);
    let before = state.last.stamp();
    let started = Instant::now();
    let frame = state
        .session
        .acquire_frame(&FrameRequest::newer_than(before), &bounded(OPERATION_WAIT))
        .expect("the static capture publishes a later authoritative frame");
    let elapsed = started.elapsed();
    let mapping = frame
        .map(PixelFormat::Bgra8, &bounded(OPERATION_WAIT))
        .expect("the later static frame maps");
    let stamp = frame.stamp();
    let correct = stamp.stream() == before.stream()
        && stamp.epoch() == before.epoch()
        && stamp.sequence() > before.sequence()
        && benchmark_mapping_fill(&mapping) == Some(state.fill);
    state.last = frame;
    Sample::new(elapsed, correct, mapping.bytes().len() as u64)
}

#[cfg(target_os = "macos")]
fn production_publication_age(active: &ActiveFlow) -> Sample {
    let mut state = lock_state(active);
    let before = state.last.stamp();
    let operation = bounded(OPERATION_WAIT);
    let frame = state
        .session
        .acquire_frame(&FrameRequest::newer_than(before), &operation)
        .expect("production capture publishes a newer frame");
    let elapsed = operation
        .now()
        .saturating_duration_since(frame.captured_at());
    let mapping = frame
        .map(PixelFormat::Bgra8, &operation)
        .expect("the production frame maps for its content oracle");
    let stamp = frame.stamp();
    let delta = stamp
        .sequence()
        .value()
        .saturating_sub(before.sequence().value());
    let correct = stamp.stream() == before.stream()
        && stamp.epoch() == before.epoch()
        && stamp.sequence() > before.sequence()
        && frame.transform().geometry() == stamp.geometry()
        && mapping.stamp() == stamp
        && benchmark_mapping_fill(&mapping) == Some(state.fill);
    let mapped = mapping.bytes().len() as u64;
    state.last = frame;
    Sample::new(elapsed, correct, mapped).with_stale_work(delta.saturating_sub(1), delta)
}

#[cfg(target_os = "macos")]
fn fixture_resize_command(fixture: &Rc<FixtureProcess>) -> Sample {
    let started = Instant::now();
    let correct = controlled_command_ok(fixture, protocol::FixtureCommandKind::Resize);
    Sample::unmapped(started.elapsed(), correct)
}

#[cfg(target_os = "macos")]
fn language_common_flow(program: &LanguageProgram) -> Sample {
    let fixture = Rc::new(FixtureProcess::spawn(FixtureBehavior::Animate));
    let engine = native_engine();
    require_permissions(&engine);
    confirm_transient_auxiliary_window_setup(&engine, &fixture);
    assert!(
        fixture.discard_setup_events(),
        "fixture-only auxiliary-window events are retired before language sampling"
    );
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
    let mapped = stdout
        .as_deref()
        .and_then(language_mapping_bytes)
        .unwrap_or(0);
    let peak_resident = stdout.as_deref().and_then(language_peak_resident_bytes);
    let abi_present = stdout
        .as_deref()
        .is_some_and(|stdout| language_abi_line_is_present(stdout, program.example_name));
    let completion_present = stdout.as_deref().is_some_and(|stdout| {
        stdout
            .lines()
            .any(|line| line == format!("{} complete", program.example_name))
    });
    let peak_present = peak_resident.is_some_and(|bytes| bytes > 0);
    let correct = executable_unchanged
        && process_succeeded
        && stderr_empty
        && fixture_acknowledged
        && receipt_present
        && abi_present
        && completion_present
        && mapped > 0
        && peak_present;
    if !correct {
        eprintln!(
            "{} common-flow rejection: executable={executable_unchanged} \
             process={process_succeeded} stderr-empty={stderr_empty} \
             fixture={fixture_acknowledged} receipt={receipt_present} abi={abi_present} \
             complete={completion_present} mapped={} peak={peak_present}",
            program.example_name,
            mapped > 0
        );
    }
    let sample = Sample::new(elapsed, correct, mapped);
    match peak_resident {
        Some(bytes) => sample.with_peak_resident_bytes(bytes),
        None => sample,
    }
}

#[cfg(target_os = "macos")]
fn controlled_command_ok(fixture: &FixtureProcess, kind: protocol::FixtureCommandKind) -> bool {
    match fixture.command(kind, OPERATION_WAIT) {
        Ok(acknowledgement) => {
            let result = acknowledgement.result();
            let accepted = result.status == 0
                && result.before_window != 0
                && result.before_window == result.after_window;
            if !accepted {
                eprintln!(
                    "benchmark fixture command {kind:?} returned an invalid acknowledgement: \
                     {result:?}"
                );
            }
            accepted
        }
        Err(error) => {
            eprintln!("benchmark fixture command {kind:?} failed: {error}");
            false
        }
    }
}

#[cfg(target_os = "macos")]
fn send_confirmed_stimulus(flow: &Flow, _session: &Session) -> bool {
    controlled_command_ok(&flow.fixture, protocol::FixtureCommandKind::Transition)
}

#[cfg(target_os = "macos")]
fn send_resize_stimulus(flow: &Flow, _session: &Session, _geometry: u64) -> bool {
    controlled_command_ok(&flow.fixture, protocol::FixtureCommandKind::Resize)
}

#[cfg(target_os = "macos")]
fn open_benchmark_capture_session(flow: &Flow) -> Session {
    flow.open_capture_session()
}

#[cfg(target_os = "macos")]
const fn input_fixture_behavior() -> FixtureBehavior {
    FixtureBehavior::Static
}

#[cfg(target_os = "macos")]
const fn pressure_fixture_behavior() -> FixtureBehavior {
    FixtureBehavior::Static
}

#[cfg(target_os = "macos")]
fn enforce_premeasurement_budgets(set: WorkloadSet, workloads: &[Workload]) {
    let latency = match set {
        WorkloadSet::Capture => PHASE2_2_CAPTURE_LATENCY_BUDGETS.as_slice(),
        WorkloadSet::Transitions => PHASE2_2_TRANSITION_LATENCY_BUDGETS.as_slice(),
        WorkloadSet::ResizeAllocation | WorkloadSet::Input => &[],
        WorkloadSet::ProductionCapture => PHASE2_PRODUCTION_CAPTURE_LATENCY_BUDGETS.as_slice(),
        WorkloadSet::ProductionTransitions => {
            PHASE2_PRODUCTION_TRANSITION_LATENCY_BUDGETS.as_slice()
        }
        WorkloadSet::ProcessDirected => PHASE2_2_PROCESS_APPKIT_LATENCY_BUDGETS.as_slice(),
        WorkloadSet::ProcessDirectedGameLike => {
            PHASE2_2_PROCESS_GAME_LIKE_LATENCY_BUDGETS.as_slice()
        }
        WorkloadSet::ProcessDiagnostics => PHASE2_2_PROCESS_DIAGNOSTIC_LATENCY_BUDGETS.as_slice(),
    };
    enforce_latency_budgets(workloads, latency);
    let (heap_limit, mapped_workloads): (Option<usize>, &[&str]) = match set {
        WorkloadSet::ProductionCapture => (
            Some(PHASE2_PRODUCTION_CAPTURE_HEAP_LIMIT_BYTES),
            &[
                "publication_age",
                "steady_frame_acquisition",
                "latest_acquisition",
                "cpu_map_bgra8",
            ],
        ),
        WorkloadSet::ProductionTransitions => (
            Some(PHASE2_PRODUCTION_TRANSITION_HEAP_LIMIT_BYTES),
            &["open_first_frame"],
        ),
        _ => (None, &[]),
    };
    if let Some(limit) = heap_limit {
        for workload in workloads {
            assert!(
                workload.peak_allocated_bytes() <= limit,
                "{} exceeded the accepted macOS production live Rust heap ceiling: {} > {} bytes",
                workload.name(),
                workload.peak_allocated_bytes(),
                limit,
            );
        }
    }
    for name in mapped_workloads {
        let workload = workloads
            .iter()
            .find(|workload| workload.name() == *name)
            .unwrap_or_else(|| panic!("the macOS production profile omitted {name}"));
        assert!(
            workload.mapped_bytes_per_result() <= PHASE2_PRODUCTION_MAPPED_BYTES_LIMIT,
            "{} exceeded the accepted macOS production mapped-byte ceiling: {} > {} bytes",
            workload.name(),
            workload.mapped_bytes_per_result(),
            PHASE2_PRODUCTION_MAPPED_BYTES_LIMIT,
        );
    }
    if matches!(
        set,
        WorkloadSet::ProcessDirected
            | WorkloadSet::ProcessDirectedGameLike
            | WorkloadSet::ProcessDiagnostics
    ) {
        for workload in workloads {
            assert!(
                workload.peak_allocated_bytes() <= PHASE2_2_PROCESS_HEAP_LIMIT_BYTES,
                "{} exceeded the frozen 16 MiB process-directed live Rust heap ceiling: {} bytes",
                workload.name(),
                workload.peak_allocated_bytes(),
            );
        }
    }
}

#[cfg(target_os = "macos")]
const fn benchmark_phase(set: WorkloadSet) -> &'static str {
    match set {
        WorkloadSet::Input
        | WorkloadSet::ProductionCapture
        | WorkloadSet::ProductionTransitions => "2",
        WorkloadSet::Capture
        | WorkloadSet::Transitions
        | WorkloadSet::ResizeAllocation
        | WorkloadSet::ProcessDirected
        | WorkloadSet::ProcessDirectedGameLike
        | WorkloadSet::ProcessDiagnostics => "2.2",
    }
}

#[cfg(target_os = "macos")]
fn profile_notes(set: WorkloadSet, notes: &str) -> String {
    let lineage = match set {
        WorkloadSet::Capture => {
            "lineage=controlled-private-command-capture; mode=default renderer=appkit-background"
        }
        WorkloadSet::Transitions => {
            "lineage=controlled-private-command-transitions; mode=default renderer=appkit-background"
        }
        WorkloadSet::ResizeAllocation => {
            "lineage=focused-resize-allocation; mode=default renderer=appkit-background stimulus=private-control"
        }
        WorkloadSet::Input => {
            "lineage=rust-System-unchanged-cross-language-ProcessDirected-cutover; mode=default renderer=appkit-background"
        }
        WorkloadSet::ProductionCapture => {
            "lineage=production-capture-publication; mode=default renderer=appkit-background stimulus=none"
        }
        WorkloadSet::ProductionTransitions => {
            "lineage=production-capture-transitions; mode=default renderer=appkit-background resize-stimulus=private-control"
        }
        WorkloadSet::ProcessDirected => {
            "lineage=process-directed-appkit; mode=default renderer=appkit-background stimulus=private-control"
        }
        WorkloadSet::ProcessDirectedGameLike => {
            "lineage=process-directed-game-like; mode=game-like renderer=opengl stimulus=private-control"
        }
        WorkloadSet::ProcessDiagnostics => {
            "lineage=process-directed-diagnostics; mode=default renderer=appkit-background stimulus=private-control"
        }
    };
    format!("{notes}; {lineage}")
}

#[cfg(target_os = "macos")]
const fn fixture_build_profile() -> &'static str {
    "fixture cargo build --release --features private-fixture; signed .app; controlled NSWorkspace new-instance launch"
}

#[cfg(target_os = "macos")]
const fn measured_close_bound() -> Duration {
    Duration::from_secs(1)
}

#[cfg(target_os = "macos")]
fn fixture_sources(manifest: &Path, _set: WorkloadSet) -> Vec<PathBuf> {
    vec![
        manifest.join("../platform/macos/src/bin/mado-pilot-macos-input-fixture.rs"),
        manifest.join("../platform/macos/src/fixture_protocol.rs"),
        manifest.join("../platform/macos/native/madopilot_macos_input_fixture.h"),
        manifest.join("../platform/macos/native/madopilot_macos_input_fixture.m"),
    ]
}

#[cfg(target_os = "macos")]
fn native_engine() -> NativeEngine {
    native_engine_with_diagnostics(DiagnosticOptions::off())
}

#[cfg(target_os = "macos")]
fn native_engine_with_diagnostics(diagnostics: DiagnosticOptions) -> NativeEngine {
    let backend = OpenCvBackend::new().expect("the required OpenCV backend initializes");
    let issuer = Arc::new(IdentityIssuer::new());
    let engine_id = issuer.engine();
    let provider = Arc::new(MacosCaptureProvider::new(issuer));
    let engine = Engine::new_with_options(
        EngineWiring { engine: engine_id, capture: Arc::clone(&provider) as Arc<dyn CaptureProvider>, matcher: Matcher::new(Arc::new(backend)), loader: PackageLoader::new(), ocr: None, input: Some(Arc::clone(&provider) as Arc<dyn InputProvider>), permission: Some(Arc::new(MacosPermissionProbe::new()) as Arc<dyn PermissionProbe>), },
        EngineOptions::new().with_diagnostics(diagnostics),
    )
    .expect("the macOS benchmark engine builds");
    NativeEngine { engine, provider }
}

#[cfg(target_os = "macos")]
const fn c_example_name() -> &'static str {
    "macos-native-input"
}

#[cfg(target_os = "macos")]
const fn cpp_example_name() -> &'static str {
    "macos-native-input-cpp"
}

#[cfg(target_os = "macos")]
const fn cpp_correctness_oracle() -> &'static str {
    "the released C++ wrapper uses explicit ProcessDirected delivery and checks its invocation-only receipt separately from exact owned-fixture events"
}

#[cfg(target_os = "macos")]
const fn cpp_receipt_line() -> &'static str {
    "receipt: outcome 1 submitted 5 evidence 1 cleanup 0"
}

#[cfg(target_os = "macos")]
fn require_permissions(engine: &NativeEngine) {
    let report = engine
        .permissions(&bounded(OPERATION_WAIT))
        .expect("the macOS benchmark reads permissions without prompting");
    for kind in mado_pilot::PermissionKind::ALL {
        assert!(
            report.outcome(kind).is_granted(),
            "the native benchmark requires {kind} to be granted before it starts"
        );
    }
}

#[cfg(target_os = "macos")]
const fn input_delivery() -> InputDelivery {
    InputDelivery::System
}

#[cfg(target_os = "macos")]
const fn focus_policy() -> FocusPolicy {
    FocusPolicy::ActivateIfRequired
}

#[cfg(target_os = "macos")]
const fn expected_key_units() -> u32 {
    0
}

#[cfg(target_os = "macos")]
const fn benchmark_fill_rgb() -> u32 {
    protocol::REPLACEMENT_FILL_RGB
}

#[cfg(target_os = "macos")]
fn profile_identity(set: WorkloadSet) -> (&'static str, String) {
    let id = match set {
        WorkloadSet::Capture => "phase-2-2-controlled-capture-aarch64-apple-darwin",
        WorkloadSet::Transitions => "phase-2-2-controlled-transitions-aarch64-apple-darwin",
        WorkloadSet::ResizeAllocation => "phase-2-macos-resize-allocation-aarch64-apple-darwin",
        WorkloadSet::Input => "phase-2-native-input-aarch64-apple-darwin",
        WorkloadSet::ProductionCapture => "phase-2-production-capture-aarch64-apple-darwin",
        WorkloadSet::ProductionTransitions => "phase-2-production-transitions-aarch64-apple-darwin",
        WorkloadSet::ProcessDirected => "phase-2-2-process-directed-appkit-aarch64-apple-darwin",
        WorkloadSet::ProcessDirectedGameLike => {
            "phase-2-2-process-directed-game-like-aarch64-apple-darwin"
        }
        WorkloadSet::ProcessDiagnostics => {
            "phase-2-2-process-directed-diagnostics-aarch64-apple-darwin"
        }
    };
    (
        id,
        format!(
            "separately linked private macOS fixture Rust, protocol-v{}, header, \
             Objective-C, and renderer sources",
            protocol::FIXTURE_CONTROL_VERSION
        ),
    )
}
