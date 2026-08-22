//! Shared orchestration for the Phase 2 native benchmark.

#[cfg(target_os = "macos")]
include!("native_phase2_macos.rs");
#[cfg(windows)]
include!("native_phase2_windows.rs");

use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

const MAX_LANGUAGE_OUTPUT_BYTES: usize = 64 * 1_024;

use mado_pilot::{
    CleanupState, ContentDigest, DeliveryPlan, Engine, FocusPolicy, Frame, FrameRequest,
    InputDelivery, InputEvent, InputOpenRequest, InputOperationKind, InputRequest,
    InputRequirement, InputSequence, Key, OpenRequest, OperationContext, PixelExtent, PixelFormat,
    SequenceOutcome, Session, SessionRequest, Status, TargetId,
};
use mado_pilot_testkit::bench_harness::{
    self, Benchmark, BoundedChildOutput, Plan, Profile, Sample, Workload, argument,
    enforce_hard_budgets, measure,
};
const OPERATION_WAIT: Duration = Duration::from_secs(2);
const CLOSE_WAIT: Duration = Duration::from_secs(5);
const PRESSURE_WAIT: Duration = Duration::from_millis(100);
const FIXTURE_WAIT: Duration = Duration::from_secs(10);
const COMMON_LANGUAGE_EVENTS: [protocol::EventSummary; 3] = [
    language_event(protocol::EVENT_POINTER_MOVE, 0),
    language_event(protocol::EVENT_KEY_DOWN, expected_key_units()),
    language_event(protocol::EVENT_KEY_UP, expected_key_units()),
];

static ARGUMENTS: OnceLock<Arguments> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkloadSet {
    Capture,
    Transitions,
    #[cfg(target_os = "macos")]
    ResizeAllocation,
    Input,
    #[cfg(target_os = "macos")]
    ProductionCapture,
    #[cfg(windows)]
    ProductionCapture1280,
    #[cfg(windows)]
    ProductionCaptureDual4k,
    #[cfg(windows)]
    ProductionTransitions1280,
    #[cfg(target_os = "macos")]
    ProductionTransitions,
    #[cfg(target_os = "macos")]
    ProcessDirected,
    #[cfg(target_os = "macos")]
    ProcessDirectedGameLike,
    #[cfg(target_os = "macos")]
    ProcessDiagnostics,
}

impl WorkloadSet {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "capture" => Some(Self::Capture),
            "transitions" => Some(Self::Transitions),
            #[cfg(target_os = "macos")]
            "resize-allocation" => Some(Self::ResizeAllocation),
            "input" => Some(Self::Input),
            #[cfg(target_os = "macos")]
            "production-capture" => Some(Self::ProductionCapture),
            #[cfg(windows)]
            "production-capture-1280x720" => Some(Self::ProductionCapture1280),
            #[cfg(windows)]
            "production-capture-dual-4k" => Some(Self::ProductionCaptureDual4k),
            #[cfg(windows)]
            "production-transitions-1280x720" => Some(Self::ProductionTransitions1280),
            #[cfg(target_os = "macos")]
            "production-transitions" => Some(Self::ProductionTransitions),
            #[cfg(target_os = "macos")]
            "process-directed" => Some(Self::ProcessDirected),
            #[cfg(target_os = "macos")]
            "process-directed-game-like" => Some(Self::ProcessDirectedGameLike),
            #[cfg(target_os = "macos")]
            "process-diagnostics" => Some(Self::ProcessDiagnostics),
            _ => None,
        }
    }

    fn measured_plan(self) -> Plan {
        match self {
            #[cfg(windows)]
            Self::ProductionCaptureDual4k => Plan::new(20, 600),
            Self::Capture => Plan::new(20, 200),
            Self::Transitions | Self::Input => Plan::new(5, 50),
            #[cfg(target_os = "macos")]
            Self::ProductionCapture => Plan::new(20, 200),
            #[cfg(windows)]
            Self::ProductionCapture1280 => Plan::new(30, 150),
            #[cfg(windows)]
            Self::ProductionTransitions1280 => Plan::new(1, 3),
            #[cfg(target_os = "macos")]
            Self::ProductionTransitions
            | Self::ResizeAllocation
            | Self::ProcessDirected
            | Self::ProcessDirectedGameLike => Plan::new(5, 50),
            #[cfg(target_os = "macos")]
            Self::ProcessDiagnostics => Plan::new(20, 200),
        }
    }

    const fn workload(self) -> &'static str {
        match self {
            Self::Capture => capture_workload_description(),
            Self::Transitions => transition_workload_description(),
            #[cfg(target_os = "macos")]
            Self::ResizeAllocation => {
                "macOS production capture allocation across controlled target resize"
            }

            Self::Input => input_workload_description(),

            #[cfg(target_os = "macos")]
            Self::ProductionCapture => {
                "macOS production capture publication age, acquisition, mapping, and retained progress"
            }
            #[cfg(windows)]
            Self::ProductionCaptureDual4k => {
                "Windows dual-4K mixed-DPI production capture, cross-seam movement, callback detachment, mapping, and dual-session pressure"
            }
            #[cfg(windows)]
            Self::ProductionCapture1280 => {
                "Windows 1280x720 production capture arrival, callback detachment, mapping, retained progress, and queue pressure"
            }
            #[cfg(windows)]
            Self::ProductionTransitions1280 => {
                "Windows 1280x720 production capture startup, retained pressure, resize recreation, and close drain"
            }
            #[cfg(target_os = "macos")]
            Self::ProductionTransitions => {
                "macOS production capture startup, controlled resize, and close transitions"
            }
            #[cfg(target_os = "macos")]
            Self::ProcessDirected => {
                "macOS AppKit process-directed discovery, posting, cleanup, and close"
            }
            #[cfg(target_os = "macos")]
            Self::ProcessDirectedGameLike => {
                "macOS OpenGL game-like process-directed discovery, posting, cleanup, and close"
            }
            #[cfg(target_os = "macos")]
            Self::ProcessDiagnostics => {
                "macOS process-directed posting with diagnostics Off, Normal, Debug, and overflow"
            }
        }
    }

    const fn queue_policy(self) -> &'static str {
        match self {
            Self::Capture => capture_queue_policy(),
            Self::Transitions => transition_queue_policy(),
            #[cfg(target_os = "macos")]
            Self::ResizeAllocation => {
                "fixture command queue depth 1; one long-lived session latest-wins queue depth 1"
            }

            Self::Input => input_queue_policy(),

            #[cfg(target_os = "macos")]
            Self::ProductionCapture => {
                "session latest-wins queue depth 1; adapter finite retained-storage limit; no input stimulus"
            }
            #[cfg(windows)]
            Self::ProductionCaptureDual4k => {
                "two WGC producer pools depth 2; two session latest-wins queues depth 1; shared Adapter retained-byte budget; fixture 16 ms repaint timer"
            }
            #[cfg(windows)]
            Self::ProductionCapture1280 => {
                "WGC producer pool depth 2; session latest-wins queue depth 1; Adapter finite retained-storage limit; fixture 16 ms repaint timer"
            }
            #[cfg(windows)]
            Self::ProductionTransitions1280 => {
                "WGC producer pool depth 2; session latest-wins queue depth 1; finite retained storage; 16 ms repaint timer; one bounded pointer resize stimulus"
            }
            #[cfg(target_os = "macos")]
            Self::ProductionTransitions => {
                "session latest-wins queue depth 1; one bounded private resize stimulus; adapter finite retained-storage limit"
            }
            #[cfg(target_os = "macos")]
            Self::ProcessDirected | Self::ProcessDirectedGameLike => {
                "one fixture command outstanding; one process-directed sequence admitted; no fallback"
            }
            #[cfg(target_os = "macos")]
            Self::ProcessDiagnostics => {
                "diagnostics Off has no queue; Normal and Debug use capacity 64; overflow uses capacity 4; every sample drains"
            }
        }
    }
}
fn profile_correctness_oracle(set: WorkloadSet) -> &'static str {
    #[cfg(target_os = "macos")]
    if matches!(
        set,
        WorkloadSet::ProductionCapture | WorkloadSet::ProductionTransitions
    ) {
        return "every retained sample checks production frame identity, frame-authoritative geometry, declared fixture content, finite retained progress, exact mapping, or bounded lifecycle outcome; the resize command is stimulus only and never substitutes for a captured result";
    }
    #[cfg(windows)]
    if set == WorkloadSet::ProductionTransitions1280 {
        return "every retained sample checks exact 1280x720 production startup, finite retained-pressure recovery, frame-authoritative resize recreation, typed target-loss and replacement recovery, or bounded idempotent close";
    }
    #[cfg(windows)]
    if matches!(
        set,
        WorkloadSet::ProductionCapture1280 | WorkloadSet::ProductionCaptureDual4k
    ) {
        return "every retained sample checks strictly newer topology-qualified production frames, exact BGRA8 mapping and declared fixture content, callback-copy accounting, finite retained progress, or observable latest-wins pressure";
    }
    "every retained sample checks complete frame identity/content, transition state, invocation-only receipt, separate fixture event, diagnostics, or common-flow outcome as its measurement states; a private acknowledgement never substitutes for product delivery or visual progress"
}
#[derive(Clone)]
struct ExecutableArtifact {
    path: PathBuf,
    bytes: Arc<[u8]>,
    #[cfg(target_os = "macos")]
    identity: ExecutableIdentity,
}

impl ExecutableArtifact {
    fn read(path: PathBuf, label: &str) -> Result<Self, String> {
        let bytes: Arc<[u8]> = std::fs::read(&path)
            .map_err(|_| format!("the {label} executable could not be read"))?
            .into();
        #[cfg(target_os = "macos")]
        let identity = {
            let identity_pin = LanguageExecutablePin::new(&path, Arc::clone(&bytes))?;
            let identity = executable_identity(identity_pin.path())?;
            drop(identity_pin);
            identity
        };
        #[cfg(target_os = "macos")]
        if std::fs::read(&path)
            .ok()
            .is_none_or(|observed| observed != bytes.as_ref())
        {
            return Err(format!(
                "the {label} executable changed while provenance was recorded"
            ));
        }
        Ok(Self {
            path,
            bytes,
            #[cfg(target_os = "macos")]
            identity,
        })
    }

    fn digest(&self) -> ContentDigest {
        ContentDigest::of(&self.bytes)
    }
}

struct Arguments {
    set: WorkloadSet,
    fixture_executable: PathBuf,
    #[cfg(target_os = "macos")]
    fixture_executable_bytes: Arc<[u8]>,
    #[cfg(target_os = "macos")]
    fixture_executable_identity: ExecutableIdentity,
    #[cfg(windows)]
    ordinary_fixture_executable: Option<PathBuf>,
    c_executable: Option<ExecutableArtifact>,
    cpp_executable: Option<ExecutableArtifact>,
    language_library: Option<ExecutableArtifact>,
    hardware: String,
    os_version: String,
    deployment_target: String,
    #[cfg(target_os = "macos")]
    benchmark_executable: PathBuf,
    #[cfg(target_os = "macos")]
    benchmark_executable_bytes: Arc<[u8]>,
    #[cfg(target_os = "macos")]
    benchmark_process: AuthenticatedFixtureProcess,
    #[cfg(target_os = "macos")]
    benchmark_executable_identity: ExecutableIdentity,
    benchmark_executable_sha256: String,
    notes: String,
}

impl Arguments {
    fn parse(arguments: &[String]) -> Result<Self, String> {
        let required = |name: &str| {
            argument(arguments, name)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| format!("missing required benchmark argument {name}"))
        };
        let set = required("--workload-set").and_then(|value| {
            WorkloadSet::parse(&value).ok_or_else(|| {
                "--workload-set must be capture, transitions, input, production-capture, \
                 production-capture-1280x720, production-capture-dual-4k, \
                 production-transitions-1280x720, production-transitions, \
                 resize-allocation, process-directed, process-directed-game-like, \
                 or process-diagnostics"
                    .to_owned()
            })
        })?;

        let fixture_executable = PathBuf::from(required("--fixture-executable")?);
        if !fixture_executable.is_file() {
            return Err("--fixture-executable does not name a built fixture binary".to_owned());
        }
        #[cfg(target_os = "macos")]
        let fixture_executable_bytes: Arc<[u8]> = std::fs::read(&fixture_executable)
            .map_err(|_| "the fixture executable could not be read".to_owned())?
            .into();
        #[cfg(target_os = "macos")]
        let fixture_executable_identity = executable_identity(&fixture_executable)?;
        #[cfg(target_os = "macos")]
        if std::fs::read(&fixture_executable)
            .ok()
            .is_none_or(|bytes| bytes != fixture_executable_bytes.as_ref())
        {
            return Err("the fixture executable changed while provenance was recorded".to_owned());
        }
        let language_executable = |name: &str, label: &str| {
            let executable = PathBuf::from(required(name)?);
            if !executable.is_file() {
                return Err(format!("{name} does not name a built executable"));
            }
            ExecutableArtifact::read(executable, label)
        };
        #[cfg(windows)]
        let ordinary_fixture_executable = if set == WorkloadSet::Input {
            let executable = PathBuf::from(required("--ordinary-fixture-executable")?);
            if !executable.is_file() {
                return Err(
                    "--ordinary-fixture-executable does not name a built executable".to_owned(),
                );
            }
            Some(executable)
        } else {
            None
        };
        let (c_executable, cpp_executable) = if set == WorkloadSet::Input {
            (
                Some(language_executable("--c-executable", "C")?),
                Some(language_executable("--cpp-executable", "C++")?),
            )
        } else {
            (None, None)
        };
        #[cfg(target_os = "macos")]
        let language_library = if set == WorkloadSet::Input {
            Some(language_executable(
                "--library",
                "MadoPilot dynamic library",
            )?)
        } else {
            None
        };
        #[cfg(windows)]
        let language_library: Option<ExecutableArtifact> = None;
        let hardware = required("--hardware")?;
        let os_version = required("--os-version")?;
        #[cfg(target_os = "macos")]
        let deployment_target = required("--deployment-target")?;
        #[cfg(windows)]
        let deployment_target = argument(arguments, "--deployment-target")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "unresolved".to_owned());
        let source_revision = required("--source-revision")?;
        let source_tree = required("--source-tree")?;
        let toolchain = required("--toolchain")?;
        let gpu_driver = required("--gpu-driver")?;
        let display_topology = required("--display-topology")?;
        let permissions_signing = required("--permissions-signing")?;
        let language_memory = if set == WorkloadSet::Input {
            format!(
                "; C/C++ peak_allocated_bytes counts only harness-side Rust allocations, while \
                 peak_resident_bytes is the child process peak reported by the native OS after \
                 owned-handle cleanup; each C/C++ child has a {} s execution bound, a 1 s \
                 termination/reap allowance, and a 65536-byte stdout/stderr cap per stream",
                LANGUAGE_PROCESS_WAIT.as_secs()
            )
        } else {
            String::new()
        };
        #[cfg(target_os = "macos")]
        let executable_binding = if set == WorkloadSet::Input {
            "; benchmark and fixture SHA-256 bytes are bound to validity-checked static and live Security code identities; each C/C++ program retains unique executable and library pins inside controller-owned mode-0500 directories, every spawned child must match the executable pin's live code identity, and both directory/file identities and bytes are rechecked after each exit"
        } else {
            "; benchmark and fixture SHA-256 bytes are bound to validity-checked static and audit-token-selected live Security code identities"
        };
        #[cfg(windows)]
        let executable_binding = "";
        #[cfg(windows)]
        let executable_digest = |path: &Path, label: &str| {
            std::fs::read(path)
                .map(|bytes| ContentDigest::of(&bytes))
                .map_err(|_| format!("the {label} executable could not be hashed"))
        };
        let benchmark_executable = std::env::current_exe()
            .map_err(|_| "the benchmark executable path is unavailable".to_owned())?;
        #[cfg(target_os = "macos")]
        let benchmark_executable_bytes: Arc<[u8]> = std::fs::read(&benchmark_executable)
            .map_err(|_| "the benchmark executable could not be hashed".to_owned())?
            .into();
        #[cfg(target_os = "macos")]
        let benchmark_executable_identity = executable_identity(&benchmark_executable)?;
        #[cfg(target_os = "macos")]
        if std::fs::read(&benchmark_executable)
            .ok()
            .is_none_or(|bytes| bytes != benchmark_executable_bytes.as_ref())
        {
            return Err(
                "the benchmark executable changed while provenance was recorded".to_owned(),
            );
        }
        #[cfg(target_os = "macos")]
        let benchmark_process = {
            let (server, _client) = UnixStream::pair()
                .map_err(|_| "the benchmark identity socket could not be created".to_owned())?;
            let process =
                authenticate_fixture_peer(&server, std::process::id(), &benchmark_executable)
                    .ok_or_else(|| {
                        "the running benchmark could not be audit-token authenticated".to_owned()
                    })?;
            if !process.matches_executable_identity(benchmark_executable_identity) {
                return Err(
                    "the running benchmark image differs from its recorded identity".to_owned(),
                );
            }
            process
        };
        #[cfg(target_os = "macos")]
        let benchmark_executable_sha256 =
            ContentDigest::of(&benchmark_executable_bytes).to_string();
        #[cfg(windows)]
        let benchmark_executable_sha256 =
            executable_digest(&benchmark_executable, "benchmark")?.to_string();
        #[cfg(target_os = "macos")]
        let fixture_binary = ContentDigest::of(&fixture_executable_bytes);
        #[cfg(windows)]
        let fixture_binary = executable_digest(&fixture_executable, "fixture")?;
        let mut binary_hashes = format!("; fixture executable sha256 {fixture_binary}");
        #[cfg(windows)]
        if let Some(executable) = ordinary_fixture_executable.as_deref() {
            let digest = executable_digest(executable, "ordinary fixture")?;
            binary_hashes.push_str(&format!("; ordinary fixture executable sha256 {digest}"));
        }
        if let Some(executable) = c_executable.as_ref() {
            binary_hashes.push_str(&format!("; C executable sha256 {}", executable.digest()));
        }
        if let Some(executable) = cpp_executable.as_ref() {
            binary_hashes.push_str(&format!("; C++ executable sha256 {}", executable.digest()));
        }
        if let Some(library) = language_library.as_ref() {
            binary_hashes.push_str(&format!(
                "; MadoPilot dynamic library sha256 {}",
                library.digest()
            ));
        }
        let notes = format!(
            "source commit {source_revision}, tree {source_tree}; toolchain {toolchain}; GPU/driver {gpu_driver}; display topology {display_topology}; permissions/signing {permissions_signing}{binary_hashes}; executable paths deliberately omitted{executable_binding}{language_memory}"
        );

        Ok(Self {
            set,
            fixture_executable,
            #[cfg(target_os = "macos")]
            fixture_executable_bytes,
            #[cfg(target_os = "macos")]
            fixture_executable_identity,
            #[cfg(windows)]
            ordinary_fixture_executable,
            c_executable,
            cpp_executable,
            language_library,
            hardware,
            os_version,
            deployment_target,
            #[cfg(target_os = "macos")]
            benchmark_executable,
            #[cfg(target_os = "macos")]
            benchmark_executable_bytes,
            #[cfg(target_os = "macos")]
            benchmark_process,
            #[cfg(target_os = "macos")]
            benchmark_executable_identity,
            benchmark_executable_sha256,
            notes,
        })
    }
}

#[derive(Debug, Clone, Copy)]
enum FixtureBehavior {
    Animate,
    #[cfg(windows)]
    AnimateAndResize,
    #[cfg(windows)]
    ProductionCapture,
    #[cfg(windows)]
    ProductionCaptureAndResize,
    #[cfg(target_os = "macos")]
    Static,
    #[cfg(target_os = "macos")]
    GameLike,
}

impl FixtureBehavior {
    #[cfg(windows)]
    const fn arguments(self) -> &'static [&'static str] {
        match self {
            Self::Animate => &["--animate-on-input"],
            Self::AnimateAndResize => &["--animate-and-resize-on-input"],
            Self::ProductionCapture => &["--production-capture"],
            Self::ProductionCaptureAndResize => {
                &["--production-capture", "--animate-and-resize-on-input"]
            }
        }
    }

    #[cfg(target_os = "macos")]
    const fn launch_mode(self) -> LaunchMode {
        match self {
            Self::Animate => LaunchMode::AnimateOnInput,
            Self::Static => LaunchMode::ControlledStatic,
            Self::GameLike => LaunchMode::ControlledGameLike,
        }
    }
}

struct Flow {
    engine: NativeEngine,
    target: TargetId,
    fixture: Rc<FixtureProcess>,
}

impl Flow {
    fn from_fixture(fixture: Rc<FixtureProcess>) -> Self {
        #[cfg(target_os = "macos")]
        let process = fixture
            .authenticated_process()
            .expect("the benchmark fixture control peer remains authenticated");

        let engine = native_engine();
        require_permissions(&engine);
        let selection_deadline = Instant::now() + FIXTURE_WAIT;
        let target = loop {
            let targets = engine
                .discover(&bounded(OPERATION_WAIT))
                .expect("the benchmark fixture is discoverable");
            #[cfg(windows)]
            let selected = protocol::select_unique_fixture(&targets, fixture.process_id());
            #[cfg(target_os = "macos")]
            let selected =
                protocol::select_unique_fixture(&targets, process.process_id(), |target| {
                    engine.authenticates_fixture_target(target, process)
                });
            if let Ok(target) = selected {
                break target.id();
            }
            assert!(
                Instant::now() < selection_deadline,
                "exactly one process-qualified benchmark fixture becomes selectable"
            );
            thread::sleep(Duration::from_millis(50));
        };
        let flow = Self {
            engine,
            target,
            fixture,
        };
        flow.confirm_initial_content();
        flow
    }

    fn confirm_initial_content(&self) {
        let session = self
            .engine
            .open_session(
                self.target,
                &SessionRequest::new().capturing(OpenRequest::new()),
                &bounded(OPERATION_WAIT),
            )
            .expect("the selected fixture opens for capture");
        let frame = session
            .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
            .expect("the selected fixture publishes an initial frame");
        let mapping = frame
            .map(PixelFormat::Bgra8, &bounded(OPERATION_WAIT))
            .expect("the fixture initial frame maps to BGRA8");
        assert!(
            mapping_matches_fills(
                mapping.bytes(),
                mapping.descriptor().stride(),
                mapping.descriptor().extent(),
                &[protocol::FILL_RGB, benchmark_fill_rgb()],
            ),
            "the selected target does not contain a declared deterministic fixture fill"
        );
        assert!(close(&session), "the fixture confirmation session closes");
    }

    fn open_capture_session(&self) -> Session {
        self.engine
            .open_session(
                self.target,
                &SessionRequest::new().capturing(OpenRequest::new()),
                &bounded(OPERATION_WAIT),
            )
            .expect("the benchmark fixture opens capture")
    }

    fn open_input_session(&self) -> Session {
        self.engine
            .open_session(
                self.target,
                &SessionRequest::new()
                    .capturing(OpenRequest::new())
                    .requesting_input(
                        InputOpenRequest::new()
                            .with_requirement(InputRequirement::Required)
                            .requiring(InputOperationKind::Keyboard, input_delivery())
                            .requiring(InputOperationKind::Pointer, input_delivery()),
                    ),
                &bounded(OPERATION_WAIT),
            )
            .expect("the benchmark fixture opens capture and required keyboard input")
    }
}

struct ActiveFlow {
    flow: Flow,
    state: Mutex<ActiveState>,
}

struct ActiveState {
    session: Session,
    last: Frame,
    fill: u32,
}

impl ActiveFlow {
    fn from_fixture(fixture: Rc<FixtureProcess>) -> Self {
        let flow = Flow::from_fixture(fixture);
        let session = flow.open_input_session();
        Self::from_open_session(flow, session)
    }

    fn from_capture_fixture(fixture: Rc<FixtureProcess>) -> Self {
        let flow = Flow::from_fixture(fixture);
        #[cfg(target_os = "macos")]
        assert!(
            controlled_command_ok(&flow.fixture, protocol::FixtureCommandKind::YieldForeground,),
            "the controlled capture fixture yields foreground before sampling"
        );
        let session = open_production_capture_session(&flow);
        Self::from_open_session(flow, session)
    }

    fn from_open_session(flow: Flow, session: Session) -> Self {
        let last = session
            .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
            .expect("the active fixture publishes a seed frame");
        let mapping = last
            .map(PixelFormat::Bgra8, &bounded(OPERATION_WAIT))
            .expect("the active fixture seed frame maps");
        let fill = benchmark_mapping_fill(&mapping)
            .expect("the active fixture seed carries one declared fill");
        Self {
            flow,
            state: Mutex::new(ActiveState {
                session,
                last,
                fill,
            }),
        }
    }
}

impl Drop for ActiveFlow {
    fn drop(&mut self) {
        let state = self
            .state
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _closed = close(&state.session);
    }
}

struct LanguageCommand {
    command: Command,
    #[cfg(target_os = "macos")]
    pinned: Arc<LanguageExecutablePin>,
    #[cfg(target_os = "macos")]
    expected_identity: ExecutableIdentity,
    #[cfg(target_os = "macos")]
    library_pin: Arc<LanguageExecutablePin>,
    #[cfg(windows)]
    expected: ExecutableArtifact,
}

impl LanguageCommand {
    fn command(&mut self) -> &mut Command {
        &mut self.command
    }

    fn executable_is_unchanged(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            language_pins_are_unchanged(&self.pinned, &self.library_pin)
        }
        #[cfg(windows)]
        {
            std::fs::read(&self.expected.path)
                .ok()
                .as_deref()
                .is_some_and(|bytes| bytes == self.expected.bytes.as_ref())
        }
    }

    fn bounded_output(&mut self) -> BoundedChildOutput {
        #[cfg(target_os = "macos")]
        {
            let expected = self.expected_identity;
            let output = bounded_child_output_checked(
                &mut self.command,
                LANGUAGE_PROCESS_WAIT,
                MAX_LANGUAGE_OUTPUT_BYTES,
                move |process_id| language_process_identity_matches(process_id, expected),
            );
            if (!output.within_bounds
                || output.status.is_none_or(|status| !status.success())
                || !output.stderr.is_empty())
                && !LANGUAGE_OUTPUT_FAILURE_REPORTED.swap(true, Ordering::AcqRel)
            {
                let stderr = String::from_utf8_lossy(&output.stderr);
                eprintln!(
                    "language child output rejected: within-bounds={} status={:?} \
                     stdout-bytes={} stderr-bytes={} stderr={stderr:?}",
                    output.within_bounds,
                    output.status,
                    output.stdout.len(),
                    output.stderr.len(),
                );
            }
            output
        }
        #[cfg(windows)]
        {
            bounded_child_output(
                &mut self.command,
                LANGUAGE_PROCESS_WAIT,
                MAX_LANGUAGE_OUTPUT_BYTES,
            )
        }
    }
}

#[derive(Clone)]
struct LanguageProgram {
    executable: ExecutableArtifact,
    #[cfg(target_os = "macos")]
    pinned: Arc<LanguageExecutablePin>,
    #[cfg(target_os = "macos")]
    library_pin: Arc<LanguageExecutablePin>,
    #[cfg(windows)]
    library_directory: PathBuf,
    example_name: &'static str,
    receipt_line: &'static str,
}

impl LanguageProgram {
    fn new(
        executable: ExecutableArtifact,
        library: Option<ExecutableArtifact>,
        example_name: &'static str,
        receipt_line: &'static str,
    ) -> Self {
        #[cfg(target_os = "macos")]
        let pinned = {
            let pinned = Arc::new(
                LanguageExecutablePin::new(&executable.path, Arc::clone(&executable.bytes))
                    .unwrap_or_else(|error| panic!("language executable pin failed: {error}")),
            );
            assert_eq!(
                executable_identity(pinned.path()),
                Ok(executable.identity),
                "the language pin retains the recorded executable code identity"
            );
            pinned
        };
        #[cfg(target_os = "macos")]
        let library_pin = {
            let library = library
                .as_ref()
                .expect("the macOS language program carries its dynamic library");
            let pinned = Arc::new(
                LanguageExecutablePin::new(&library.path, Arc::clone(&library.bytes))
                    .unwrap_or_else(|error| panic!("language library pin failed: {error}")),
            );
            assert_eq!(
                executable_identity(pinned.path()),
                Ok(library.identity),
                "the language library pin retains the recorded code identity"
            );
            pinned
        };
        #[cfg(windows)]
        let library_directory = executable
            .path
            .parent()
            .and_then(Path::parent)
            .filter(|directory| directory.join("madopilot.dll").is_file())
            .unwrap_or_else(|| {
                panic!(
                    "{} has no madopilot.dll in its cargo profile directory",
                    executable.path.display()
                )
            })
            .to_path_buf();
        #[cfg(windows)]
        let _unused_library = library;
        Self {
            executable,
            #[cfg(target_os = "macos")]
            pinned,
            #[cfg(target_os = "macos")]
            library_pin,
            #[cfg(windows)]
            library_directory,
            example_name,
            receipt_line,
        }
    }

    fn command(&self) -> LanguageCommand {
        #[cfg(target_os = "macos")]
        {
            let pinned = Arc::clone(&self.pinned);
            let library_pin = Arc::clone(&self.library_pin);
            let mut command = Command::new(pinned.path());
            command.env("DYLD_INSERT_LIBRARIES", library_pin.path());
            command.env("MADO_PILOT_EXPECT_LIBRARY_IMAGE", library_pin.path());
            LanguageCommand {
                command,
                pinned,
                expected_identity: self.executable.identity,
                library_pin,
            }
        }
        #[cfg(windows)]
        {
            let mut command = Command::new(&self.executable.path);
            let existing = std::env::var_os("PATH").unwrap_or_default();
            let mut search = vec![self.library_directory.clone()];
            search.extend(std::env::split_paths(&existing));
            command.env(
                "PATH",
                std::env::join_paths(search)
                    .expect("the Windows child library path is representable"),
            );
            LanguageCommand {
                command,
                expected: self.executable.clone(),
            }
        }
    }

    fn expected_fixture_events(&self) -> &'static [protocol::EventSummary] {
        if self.example_name == cpp_example_name() {
            &CPP_LANGUAGE_EVENTS
        } else {
            &COMMON_LANGUAGE_EVENTS
        }
    }
}

fn language_common_fixture(
    program: LanguageProgram,
    _shared_fixture: Rc<FixtureProcess>,
) -> LanguageProgram {
    program
}

pub(super) fn run() {
    let raw_arguments = std::env::args().collect::<Vec<_>>();
    if !raw_arguments.iter().any(|argument| argument == "--bench") {
        println!("native-phase2: compiled; measurement skipped because --bench was not requested");
        return;
    }
    let parsed = Arguments::parse(&raw_arguments).unwrap_or_else(|error| {
        panic!(
            "{error}\n{}",
            concat!(
                "usage: cargo bench --package mado-pilot --bench native-phase2 -- ",
                "--workload-set <capture|transitions|input|production-capture|",
                "production-capture-1280x720|production-capture-dual-4k|",
                "production-transitions-1280x720|production-transitions|",
                "resize-allocation|process-directed|process-directed-game-like|",
                "process-diagnostics> ",
                "--fixture-executable <path> [--ordinary-fixture-executable <path> ",
                "--c-executable <path> --cpp-executable <path> --library <path>] ",
                "--hardware <description> --os-version <description> ",
                "--deployment-target <description> ",
                "--source-revision <commit> --source-tree <tree> ",
                "--toolchain <versions> --gpu-driver <description> ",
                "--display-topology <description> ",
                "--permissions-signing <description>\n",
                "(on Windows the ordinary fixture, C, and C++ executables are required for the input set)",
            )
        )
    });
    let set = parsed.set;
    assert!(
        ARGUMENTS.set(parsed).is_ok(),
        "benchmark arguments initialize exactly once"
    );
    let plan = set.measured_plan();
    let workloads = workloads(set, plan);
    let args = arguments();
    let (id, fixture) = profile_identity(set);
    #[cfg(target_os = "macos")]
    {
        let benchmark_live_identity_unchanged = args
            .benchmark_process
            .matches_executable_identity(args.benchmark_executable_identity);
        let benchmark_static_identity_unchanged = executable_identity(&args.benchmark_executable)
            .is_ok_and(|identity| identity == args.benchmark_executable_identity);
        let benchmark_bytes_unchanged = std::fs::read(&args.benchmark_executable)
            .ok()
            .as_deref()
            .is_some_and(|bytes| bytes == args.benchmark_executable_bytes.as_ref());
        let fixture_static_identity_unchanged = executable_identity(&args.fixture_executable)
            .is_ok_and(|identity| identity == args.fixture_executable_identity);
        let fixture_bytes_unchanged = std::fs::read(&args.fixture_executable)
            .ok()
            .as_deref()
            .is_some_and(|bytes| bytes == args.fixture_executable_bytes.as_ref());
        let fixture_finalization_succeeded = FIXTURE_FINALIZATION_SUCCEEDED.load(Ordering::Acquire);
        assert!(
            post_use_identity_gate(
                true,
                &[
                    benchmark_live_identity_unchanged,
                    benchmark_static_identity_unchanged,
                    benchmark_bytes_unchanged,
                    fixture_static_identity_unchanged,
                    fixture_bytes_unchanged,
                    fixture_finalization_succeeded,
                ],
            ),
            "benchmark provenance gate failed: benchmark-live={benchmark_live_identity_unchanged} \
             benchmark-static={benchmark_static_identity_unchanged} \
             benchmark-bytes={benchmark_bytes_unchanged} \
             fixture-static={fixture_static_identity_unchanged} \
             fixture-bytes={fixture_bytes_unchanged} \
             fixture-finalization={fixture_finalization_succeeded}"
        );
    }
    bench_harness::report(
        &Benchmark {
            id,
            workload: set.workload(),
            phase: benchmark_phase(set),
        },
        &Profile {
            fixture,
            fixture_sha256: fixture_digest(set).to_string(),
            benchmark_executable_sha256: Some(args.benchmark_executable_sha256.clone()),
            hardware: args.hardware.clone(),
            os_version: args.os_version.clone(),
            deployment_target: Some(args.deployment_target.clone()),
            build_profile: format!(
                "cargo bench, {}debug_assertions={}; {}",
                benchmark_build_features(set),
                cfg!(debug_assertions),
                fixture_build_profile(),
            ),
            correctness_oracle: profile_correctness_oracle(set),
            queue_policy: set.queue_policy(),
            notes: Some(profile_notes(set, &args.notes)),
        },
        plan,
        &workloads,
    );
    enforce_hard_budgets(&workloads);
    enforce_premeasurement_budgets(set, &workloads);
}

fn workloads(set: WorkloadSet, plan: Plan) -> Vec<Workload> {
    match set {
        WorkloadSet::Capture => capture_workloads(plan),
        WorkloadSet::Transitions => transition_workloads(plan),
        #[cfg(target_os = "macos")]
        WorkloadSet::ProductionCapture => production_capture_workloads(plan),
        #[cfg(windows)]
        WorkloadSet::ProductionCapture1280 => production_capture_1280_workloads(plan),
        #[cfg(windows)]
        WorkloadSet::ProductionCaptureDual4k => production_capture_dual_4k_workloads(plan),
        #[cfg(windows)]
        WorkloadSet::ProductionTransitions1280 => production_transition_1280_workloads(plan),
        #[cfg(target_os = "macos")]
        WorkloadSet::ProductionTransitions => production_transition_workloads(plan),
        #[cfg(target_os = "macos")]
        WorkloadSet::ResizeAllocation => resize_allocation_workloads(plan),

        WorkloadSet::Input => {
            let fixture = Rc::new(FixtureProcess::spawn(input_fixture_behavior()));
            let args = arguments();
            let c = LanguageProgram::new(
                args.c_executable
                    .clone()
                    .expect("the input benchmark requires its C executable"),
                args.language_library.clone(),
                c_example_name(),
                "receipt: outcome 1 submitted 4 fault 0 cleanup 0",
            );
            let cpp = LanguageProgram::new(
                args.cpp_executable
                    .clone()
                    .expect("the input benchmark requires its C++ executable"),
                args.language_library.clone(),
                cpp_example_name(),
                cpp_receipt_line(),
            );
            let workloads = vec![
                measure(
                    "input_request_receipt",
                    "the complete two-event receipt corresponds to exactly one fixture key-down and key-up summary",
                    plan,
                    || ActiveFlow::from_fixture(Rc::clone(&fixture)),
                    input_request_receipt,
                ),
                measure(
                    "rust_common_flow",
                    "fresh open, frame, mapping, exact fixture input, receipt, and close all complete",
                    plan,
                    || Flow::from_fixture(Rc::clone(&fixture)),
                    rust_common_flow,
                ),
                measure(
                    "c_process_load",
                    "a fresh C process loads and negotiates the released ABI without opening native capabilities",
                    plan,
                    || c.clone(),
                    language_process_load,
                ),
                measure(
                    "c_common_flow",
                    "the released C ABI uses explicit ProcessDirected delivery and checks its invocation-only receipt separately from exact owned-fixture events",
                    plan,
                    || language_common_fixture(c.clone(), Rc::clone(&fixture)),
                    language_common_flow,
                ),
                measure(
                    "cpp_process_load",
                    "a fresh C++ process loads its wrapper and negotiates the released ABI without opening native capabilities",
                    plan,
                    || cpp.clone(),
                    language_process_load,
                ),
                measure(
                    "cpp_common_flow",
                    cpp_correctness_oracle(),
                    plan,
                    || language_common_fixture(cpp.clone(), Rc::clone(&fixture)),
                    language_common_flow,
                ),
            ];
            #[cfg(windows)]
            let mut workloads = workloads;
            #[cfg(windows)]
            workloads.insert(
                1,
                measure(
                    "ordinary_window_queue_submission",
                    "one ordinary exact-window pointer event crosses current-target pre/post fences, reports queue admission, and reaches only the selected fixture",
                    plan,
                    OrdinaryInputFlow::spawn,
                    ordinary_window_queue_submission,
                ),
            );
            #[cfg(windows)]
            workloads.insert(
                2,
                measure(
                    "ordinary_window_button_submission",
                    "one pointer move followed by a two-unit primary-button event crosses the production route and reaches only the selected ordinary fixture",
                    plan,
                    OrdinaryInputFlow::spawn,
                    ordinary_window_button_submission,
                ),
            );
            #[cfg(windows)]
            workloads.insert(
                3,
                measure(
                    "ordinary_window_max_sequence",
                    "the maximum accepted 256-event sequence crosses the ordinary production route, reports every submission, and reaches only the selected fixture",
                    plan,
                    OrdinaryInputFlow::spawn,
                    ordinary_window_max_sequence,
                ),
            );
            workloads
        }
        #[cfg(target_os = "macos")]
        WorkloadSet::ProcessDirected => process_directed_workloads(plan, FixtureBehavior::Static),
        #[cfg(target_os = "macos")]
        WorkloadSet::ProcessDirectedGameLike => {
            process_directed_workloads(plan, FixtureBehavior::GameLike)
        }
        #[cfg(target_os = "macos")]
        WorkloadSet::ProcessDiagnostics => process_diagnostic_workloads(plan),
    }
}

struct StimulatedFrame {
    correct: bool,
    mapped: u64,
    #[cfg(target_os = "macos")]
    elapsed_after_acknowledgement: Duration,
}

fn advance_to_stimulated_frame(active: &ActiveFlow, state: &mut ActiveState) -> StimulatedFrame {
    let before = state.last.stamp();
    let expected_fill = alternate_benchmark_fill(state.fill)
        .expect("the retained fixture fill is one of the two declared states");
    let stimulus_acknowledged = send_confirmed_stimulus(&active.flow, &state.session);
    #[cfg(target_os = "macos")]
    let started_after_acknowledgement = Instant::now();

    let operation = bounded(OPERATION_WAIT);
    let mut cursor = before;
    let mut mapped = 0u64;
    let mut intermediate_content_ok = true;

    loop {
        let frame = state
            .session
            .acquire_frame(&FrameRequest::newer_than(cursor), &operation)
            .expect("the deterministic stimulus produces another frame before the deadline");
        let mapping = frame
            .map(PixelFormat::Bgra8, &operation)
            .expect("a candidate stimulated frame maps");
        mapped = mapped.saturating_add(mapping.bytes().len() as u64);
        let fill = benchmark_mapping_fill(&mapping);
        if fill == Some(expected_fill) {
            let stamp = frame.stamp();
            let correct = stimulus_acknowledged
                && intermediate_content_ok
                && stamp.stream() == before.stream()
                && stamp.epoch() == before.epoch()
                && stamp.sequence() > before.sequence();
            state.last = frame;
            state.fill = expected_fill;
            return StimulatedFrame {
                correct,
                mapped,
                #[cfg(target_os = "macos")]
                elapsed_after_acknowledgement: started_after_acknowledgement.elapsed(),
            };
        }
        intermediate_content_ok &= fill.is_some();
        cursor = frame.stamp();
    }
}

fn stimulus_to_frame(active: &ActiveFlow) -> Sample {
    let mut state = lock_state(active);
    let before = state.last.stamp();
    #[cfg(windows)]
    let started = Instant::now();
    let stimulated = advance_to_stimulated_frame(active, &mut state);
    #[cfg(target_os = "macos")]
    let elapsed = stimulated.elapsed_after_acknowledgement;
    #[cfg(windows)]
    let elapsed = started.elapsed();
    let after = state.last.stamp();
    let delta = after
        .sequence()
        .value()
        .saturating_sub(before.sequence().value());
    Sample::new(elapsed, stimulated.correct, stimulated.mapped)
        .with_stale_work(delta.saturating_sub(1), delta)
}

fn production_steady_acquisition(active: &ActiveFlow) -> Sample {
    let mut state = lock_state(active);
    let before = state.last.stamp();
    let operation = bounded(OPERATION_WAIT);
    let started = Instant::now();
    let frame = state
        .session
        .acquire_frame(&FrameRequest::newer_than(before), &operation)
        .expect("production capture publishes a strictly newer frame");
    let elapsed = started.elapsed();
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
        && mapping_is_benchmark_content(&mapping);
    let mapped = mapping.bytes().len() as u64;
    state.last = frame;
    Sample::new(elapsed, correct, mapped).with_stale_work(delta.saturating_sub(1), delta)
}

fn production_latest_acquisition(active: &ActiveFlow) -> Sample {
    let mut state = lock_state(active);
    let before = state.last.stamp();
    let operation = bounded(OPERATION_WAIT);
    let progressed = state
        .session
        .acquire_frame(&FrameRequest::newer_than(before), &operation)
        .expect("production progress is observed before latest is measured");
    let progressed_stamp = progressed.stamp();
    let started = Instant::now();
    let latest = state
        .session
        .acquire_frame(&FrameRequest::latest(), &operation)
        .expect("latest returns the maintained production frame");
    let elapsed = started.elapsed();
    let mapping = latest
        .map(PixelFormat::Bgra8, &operation)
        .expect("latest production content maps for its oracle");
    let stamp = latest.stamp();
    let delta = stamp
        .sequence()
        .value()
        .saturating_sub(before.sequence().value());
    let correct = progressed_stamp.stream() == before.stream()
        && progressed_stamp.epoch() == before.epoch()
        && progressed_stamp.sequence() > before.sequence()
        && stamp.stream() == before.stream()
        && stamp.epoch() == before.epoch()
        && stamp.sequence() >= progressed_stamp.sequence()
        && latest.transform().geometry() == stamp.geometry()
        && mapping.stamp() == stamp
        && mapping_is_benchmark_content(&mapping)
        && delta > 0;
    state.last = latest;
    Sample::new(elapsed, correct, mapping.bytes().len() as u64)
        .with_stale_work(delta.saturating_sub(1), delta)
}

fn production_cpu_map(active: &ActiveFlow) -> Sample {
    let mut state = lock_state(active);
    let before = state.last.stamp();
    let operation = bounded(OPERATION_WAIT);
    let frame = state
        .session
        .acquire_frame(&FrameRequest::newer_than(before), &operation)
        .expect("production capture publishes a frame to map");
    let started = Instant::now();
    let mapping = frame
        .map(PixelFormat::Bgra8, &operation)
        .expect("the production frame maps to BGRA8");
    let elapsed = started.elapsed();
    let stamp = frame.stamp();
    let correct = stamp.stream() == before.stream()
        && stamp.epoch() == before.epoch()
        && stamp.sequence() > before.sequence()
        && frame.transform().geometry() == stamp.geometry()
        && mapping.stamp() == stamp
        && mapping.bytes().len() == mapping.descriptor().byte_len()
        && mapping_is_benchmark_content(&mapping);
    let mapped = mapping.bytes().len() as u64;
    state.last = frame;
    Sample::new(elapsed, correct, mapped)
}

fn latest_acquisition(active: &ActiveFlow) -> Sample {
    let mut state = lock_state(active);
    let before = state.last.stamp();
    let stimulated = advance_to_stimulated_frame(active, &mut state);
    let published = state.last.stamp();
    let started = Instant::now();
    let frame = state
        .session
        .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
        .expect("latest acquisition returns the maintained stimulated frame");
    let elapsed = started.elapsed();
    let stamp = frame.stamp();
    let delta = stamp
        .sequence()
        .value()
        .saturating_sub(before.sequence().value());
    let correct = stimulated.correct
        && stamp.stream() == before.stream()
        && stamp.epoch() == before.epoch()
        && stamp.sequence() >= published.sequence()
        && delta > 0;
    state.last = frame;
    Sample::unmapped(elapsed, correct).with_stale_work(delta.saturating_sub(1), delta)
}

fn cpu_map(active: &ActiveFlow) -> Sample {
    let mut state = lock_state(active);
    let before = state.last.stamp();
    let expected_fill = alternate_benchmark_fill(state.fill)
        .expect("the retained fixture fill is one of the two declared states");
    assert!(
        send_confirmed_stimulus(&active.flow, &state.session),
        "mapping stimulus completes"
    );
    let operation = bounded(OPERATION_WAIT);
    let mut cursor = before;
    let mut intermediate_content_ok = true;

    loop {
        let frame = state
            .session
            .acquire_frame(&FrameRequest::newer_than(cursor), &operation)
            .expect("mapping stimulus produces another frame before the deadline");
        let started = Instant::now();
        let mapping = frame
            .map(PixelFormat::Bgra8, &operation)
            .expect("the newer frame maps to BGRA8");
        let elapsed = started.elapsed();
        let fill = benchmark_mapping_fill(&mapping);
        if fill == Some(expected_fill) {
            let stamp = frame.stamp();
            let correct = intermediate_content_ok
                && stamp.stream() == before.stream()
                && stamp.epoch() == before.epoch()
                && stamp.sequence() > before.sequence()
                && mapping.stamp() == stamp
                && mapping.bytes().len() == mapping.descriptor().byte_len();
            let mapped = mapping.bytes().len() as u64;
            state.last = frame;
            state.fill = expected_fill;
            return Sample::new(elapsed, correct, mapped);
        }
        intermediate_content_ok &= fill.is_some();
        cursor = frame.stamp();
    }
}

fn open_first_frame(flow: &Flow) -> Sample {
    let started = Instant::now();
    let session = open_benchmark_capture_session(flow);

    let frame = session
        .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
        .expect("a fresh session publishes its first frame");
    let elapsed = started.elapsed();
    let mapping = frame
        .map(PixelFormat::Bgra8, &bounded(OPERATION_WAIT))
        .expect("the first frame maps");
    let correct = frame.stamp().stream() == session.stream()
        && mapping_is_benchmark_content(&mapping)
        && close(&session);
    Sample::new(elapsed, correct, mapping.bytes().len() as u64)
}

fn retained_pressure_resume(_: &()) -> Sample {
    let flow = Flow::from_fixture(Rc::new(FixtureProcess::spawn(pressure_fixture_behavior())));
    #[cfg(target_os = "macos")]
    assert!(
        controlled_command_ok(&flow.fixture, protocol::FixtureCommandKind::YieldForeground,),
        "the retained-pressure fixture yields foreground before sampling"
    );
    let session = open_benchmark_capture_session(&flow);

    let capacity = session
        .description()
        .queue()
        .retained_storage()
        .expect("a native session reports a finite retained-storage limit")
        .get() as usize;
    assert!(capacity >= 2, "retained pressure needs at least two slots");
    let first = session
        .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
        .expect("the pressure session publishes its first frame");
    let mut retained = Vec::with_capacity(capacity);
    retained.push(first);
    while retained.len() < capacity {
        let before = retained.last().expect("a retained frame").stamp();
        assert!(
            send_confirmed_stimulus(&flow, &session),
            "retained pressure stimulus completes"
        );
        retained.push(
            session
                .acquire_frame(&FrameRequest::newer_than(before), &bounded(OPERATION_WAIT))
                .expect("retained storage fills to its reported limit"),
        );
    }
    let before = retained.last().expect("the last retained frame").stamp();
    assert!(
        send_confirmed_stimulus(&flow, &session),
        "blocked publication stimulus completes"
    );
    let blocked = session
        .acquire_frame(&FrameRequest::newer_than(before), &bounded(PRESSURE_WAIT))
        .expect_err("a full retained budget cannot invent publication progress");
    retained.remove(0);
    let started = Instant::now();
    assert!(
        send_confirmed_stimulus(&flow, &session),
        "resume stimulus completes"
    );
    let resumed = session
        .acquire_frame(&FrameRequest::newer_than(before), &bounded(OPERATION_WAIT))
        .expect("releasing one retained slot resumes publication");
    let elapsed = started.elapsed();
    let delta = resumed
        .stamp()
        .sequence()
        .value()
        .saturating_sub(before.sequence().value());
    let correct = blocked.status() == Status::DeadlineExceeded && delta > 1;
    drop(retained);
    drop(resumed);
    let correct = correct && close(&session);
    Sample::unmapped(elapsed, correct).with_stale_work(delta.saturating_sub(1), delta)
}

fn production_retained_pressure_resume(fixture: &Rc<FixtureProcess>) -> Sample {
    let flow = Flow::from_fixture(Rc::clone(fixture));
    let session = open_production_capture_session(&flow);
    let capacity = session
        .description()
        .queue()
        .retained_storage()
        .expect("a production session reports a finite retained-storage limit")
        .get() as usize;
    assert!(capacity >= 2, "retained pressure needs at least two slots");

    let first = session
        .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
        .expect("the production pressure session publishes its first frame");
    #[cfg(windows)]
    let exact_extent = first.descriptor().extent() == PixelExtent::new(1_280, 720);
    #[cfg(target_os = "macos")]
    let exact_extent = true;
    let mut retained = Vec::with_capacity(capacity);
    retained.push(first);
    while retained.len() < capacity {
        let before = retained
            .last()
            .expect("a retained production frame")
            .stamp();
        retained.push(
            session
                .acquire_frame(&FrameRequest::newer_than(before), &bounded(OPERATION_WAIT))
                .expect("natural production fills the reported retained limit"),
        );
    }

    let before = retained
        .last()
        .expect("the last retained production frame")
        .stamp();
    let blocked = session
        .acquire_frame(&FrameRequest::newer_than(before), &bounded(PRESSURE_WAIT))
        .expect_err("a full production retained budget cannot invent publication progress");
    retained.remove(0);
    let started = Instant::now();
    let resumed = session
        .acquire_frame(&FrameRequest::newer_than(before), &bounded(OPERATION_WAIT))
        .expect("releasing one retained production slot resumes publication");
    let elapsed = started.elapsed();
    let delta = resumed
        .stamp()
        .sequence()
        .value()
        .saturating_sub(before.sequence().value());
    let correct = exact_extent && blocked.status() == Status::DeadlineExceeded && delta > 1;
    drop(retained);
    drop(resumed);
    let correct = correct && close(&session);
    Sample::unmapped(elapsed, correct).with_stale_work(delta.saturating_sub(1), delta)
}

fn resize_recreation(active: &ActiveFlow) -> Sample {
    let mut state = lock_state(active);
    let before = state.last.stamp();
    let old_extent = state.last.descriptor().extent();
    #[cfg(target_os = "macos")]
    let expected_logical_size = {
        let placement = state
            .last
            .transform()
            .target()
            .expect("the macOS fixture frame carries authoritative target placement");
        expected_controlled_resize_logical_size(placement.logical_size()).unwrap_or_else(|| {
            let scale = placement.scale();
            panic!(
                "the macOS fixture starts from one declared target geometry: \
                 extent={old_extent:?} logical={:?} scale={}x{}",
                placement.logical_size(),
                scale.x(),
                scale.y(),
            )
        })
    };
    let started = Instant::now();
    assert!(
        send_resize_stimulus(&active.flow, &state.session, before.geometry().value(),),
        "resize stimulus is acknowledged"
    );

    let resize_deadline = Instant::now() + FIXTURE_WAIT;
    let frame = loop {
        let candidate = state
            .session
            .acquire_frame(
                &FrameRequest::newer_than(state.last.stamp()),
                &bounded(OPERATION_WAIT),
            )
            .expect("the resized fixture keeps publishing");
        if candidate.descriptor().extent() != old_extent {
            break candidate;
        }
        state.last = candidate;
        assert!(
            Instant::now() < resize_deadline,
            "the resized fixture changes extent before the absolute deadline",
        );
    };
    let elapsed = started.elapsed();
    let correct = frame.stamp().epoch() > before.epoch()
        && frame.stamp().geometry() > before.geometry()
        && frame.transform().geometry() == frame.stamp().geometry();
    #[cfg(target_os = "macos")]
    let correct = correct
        && frame.transform().target().is_some_and(|placement| {
            controlled_resize_logical_size_matches(placement.logical_size(), expected_logical_size)
        });
    state.last = frame;
    Sample::unmapped(elapsed, correct)
}

fn close_drain(flow: &Flow) -> Sample {
    let session = open_benchmark_capture_session(flow);

    let _seed = session
        .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
        .expect("the close sample owns a live session");
    let started = Instant::now();
    let first = session.close(&bounded(measured_close_bound()));
    let elapsed = started.elapsed();
    let second = session.close(&bounded(measured_close_bound()));

    let correct = first.is_ok() && second.is_ok() && session.is_closed();
    Sample::unmapped(elapsed, correct)
}

fn input_request_receipt(active: &ActiveFlow) -> Sample {
    #[cfg(target_os = "macos")]
    assert!(
        active.flow.fixture.begin_flow(0),
        "the input receipt sample starts with empty fixture event state"
    );
    let state = lock_state(active);
    let started = Instant::now();
    let receipt_ok = send_key_pair(&state.session);
    let elapsed = started.elapsed();
    let correct = receipt_ok && active.flow.fixture.next_key_pair();
    Sample::unmapped(elapsed, correct)
}

fn rust_common_flow(flow: &Flow) -> Sample {
    #[cfg(target_os = "macos")]
    assert!(
        flow.fixture.begin_flow(0),
        "the Rust common flow starts with empty fixture event state"
    );
    let started = Instant::now();
    let session = flow.open_input_session();
    let frame = session
        .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
        .expect("the common flow acquires a frame");
    let mapping = frame
        .map(PixelFormat::Bgra8, &bounded(OPERATION_WAIT))
        .expect("the common flow maps its frame");
    let correct = mapping_is_benchmark_content(&mapping)
        && send_key_pair(&session)
        && flow.fixture.next_key_pair()
        && close(&session);
    let elapsed = started.elapsed();
    Sample::new(elapsed, correct, mapping.bytes().len() as u64)
}
fn language_process_load(program: &LanguageProgram) -> Sample {
    let started = Instant::now();
    let mut command = program.command();
    command.command().arg("--load-check");
    let output = command.bounded_output();
    let executable_unchanged = command.executable_is_unchanged();
    let elapsed = started.elapsed();
    let (correct, peak_resident) = if executable_unchanged
        && output.within_bounds
        && output.status.is_some_and(|status| status.success())
        && output.stderr.is_empty()
    {
        let stdout = String::from_utf8(output.stdout).ok();
        let peak = stdout.as_deref().and_then(language_peak_resident_bytes);
        let complete = stdout.as_deref().is_some_and(|stdout| {
            language_abi_line_is_present(stdout, program.example_name)
                && stdout
                    .lines()
                    .any(|line| line == format!("{} complete (load check)", program.example_name))
        });
        (complete && peak.is_some_and(|bytes| bytes > 0), peak)
    } else {
        (false, None)
    };
    let sample = Sample::unmapped(elapsed, correct);
    match peak_resident {
        Some(bytes) => sample.with_peak_resident_bytes(bytes),
        None => sample,
    }
}

fn language_abi_line_is_present(stdout: &str, example_name: &str) -> bool {
    let prefix = format!("{example_name}: abi 1.2 table ");
    stdout.lines().any(|line| line.starts_with(&prefix))
}

fn language_mapping_bytes(stdout: &str) -> Option<u64> {
    let mut mappings = stdout.lines().filter_map(|line| {
        line.strip_prefix("mapping: ")
            .and_then(|value| value.strip_suffix(" byte(s)"))
            .and_then(|value| value.parse().ok())
    });
    let mapping = mappings.next()?;
    if mappings.next().is_none() {
        Some(mapping)
    } else {
        None
    }
}

fn language_peak_resident_bytes(stdout: &str) -> Option<u64> {
    let mut peaks = stdout.lines().filter_map(|line| {
        line.strip_prefix("peak resident: ")
            .and_then(|value| value.strip_suffix(" byte(s)"))
            .and_then(|value| value.parse().ok())
    });
    let peak = peaks.next()?;
    if peaks.next().is_none() {
        Some(peak)
    } else {
        None
    }
}

fn lock_state(active: &ActiveFlow) -> std::sync::MutexGuard<'_, ActiveState> {
    active
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn open_production_capture_session(flow: &Flow) -> Session {
    flow.open_capture_session()
}

fn send_key_pair(session: &Session) -> bool {
    let sequence = InputSequence::new(vec![
        InputEvent::KeyPress(Key::Character('b')),
        InputEvent::KeyRelease(Key::Character('b')),
    ])
    .expect("the benchmark key pair is valid");
    send_sequence(session, sequence, 2)
}

fn send_sequence(session: &Session, sequence: InputSequence, submitted: usize) -> bool {
    let receipt = session
        .send_input(
            &InputRequest::new(
                session.target(),
                sequence,
                DeliveryPlan::require(input_delivery()),
            )
            .with_focus(focus_policy()),
            &bounded(OPERATION_WAIT),
        )
        .expect("the benchmark sequence returns a receipt");
    let complete = receipt.outcome() == SequenceOutcome::Complete
        && receipt.submitted() == submitted
        && receipt.fault().is_none()
        && receipt.cleanup() == CleanupState::NotNeeded;
    if !complete {
        eprintln!("benchmark stimulus receipt: {receipt:?}");
    }
    complete
}

fn close(session: &Session) -> bool {
    if session.close(&bounded(CLOSE_WAIT)).is_err() && session.close(&bounded(CLOSE_WAIT)).is_err()
    {
        return false;
    }
    session.is_closed()
}

fn bounded(duration: Duration) -> OperationContext {
    OperationContext::new()
        .with_timeout(duration)
        .expect("the benchmark operation timeout is positive")
}

fn mapping_is_benchmark_content(mapping: &mado_pilot::CpuMapping) -> bool {
    benchmark_mapping_fill(mapping).is_some()
}

#[cfg(windows)]
fn mapping_pixel_is_benchmark_content(mapping: &mado_pilot::CpuMapping, point: Point) -> bool {
    mapping_pixel_matches_any(mapping, point, [protocol::FILL_RGB, benchmark_fill_rgb()])
}

#[cfg(windows)]
fn mapping_pixel_is_benchmark_marker(mapping: &mado_pilot::CpuMapping, point: Point) -> bool {
    mapping_pixel_matches_any(mapping, point, [protocol::BENCHMARK_MARKER_RGB])
}

#[cfg(windows)]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a validated in-frame capture point is integral and bounded by the 4K descriptor"
)]
fn mapping_pixel_matches_any<const N: usize>(
    mapping: &mado_pilot::CpuMapping,
    point: Point,
    fills: [u32; N],
) -> bool {
    let descriptor = mapping.descriptor();
    let x = point.x();
    let y = point.y();
    if x < 0.0 || y < 0.0 || x.fract() != 0.0 || y.fract() != 0.0 {
        return false;
    }
    let x = x as usize;
    let y = y as usize;
    let width = descriptor.extent().width() as usize;
    let height = descriptor.extent().height() as usize;
    if x >= width || y >= height {
        return false;
    }
    let Some(offset) = y
        .checked_mul(descriptor.stride())
        .and_then(|row| x.checked_mul(4).and_then(|column| row.checked_add(column)))
    else {
        return false;
    };
    let Some(seen) = mapping.bytes().get(offset..offset.saturating_add(3)) else {
        return false;
    };
    fills.into_iter().any(|fill| {
        let wanted = [
            (fill & 0xff) as u8,
            ((fill >> 8) & 0xff) as u8,
            ((fill >> 16) & 0xff) as u8,
        ];
        seen.iter()
            .zip(wanted)
            .all(|(observed, expected)| observed.abs_diff(expected) <= protocol::FILL_TOLERANCE)
    })
}

fn benchmark_mapping_fill(mapping: &mado_pilot::CpuMapping) -> Option<u32> {
    [protocol::FILL_RGB, benchmark_fill_rgb()]
        .into_iter()
        .find(|fill| {
            mapping_matches_fills(
                mapping.bytes(),
                mapping.descriptor().stride(),
                mapping.descriptor().extent(),
                &[*fill],
            )
        })
}

fn alternate_benchmark_fill(fill: u32) -> Option<u32> {
    if fill == protocol::FILL_RGB {
        Some(benchmark_fill_rgb())
    } else if fill == benchmark_fill_rgb() {
        Some(protocol::FILL_RGB)
    } else {
        None
    }
}

fn benchmark_build_features(set: WorkloadSet) -> &'static str {
    #[cfg(windows)]
    if matches!(
        set,
        WorkloadSet::ProductionCapture1280
            | WorkloadSet::ProductionCaptureDual4k
            | WorkloadSet::ProductionTransitions1280
    ) {
        return "default features; platform/windows benchmark-instrumentation dev feature; ";
    }
    let _ = set;
    "default features, "
}

fn mapping_matches_fills(pixels: &[u8], stride: usize, extent: PixelExtent, fills: &[u32]) -> bool {
    let width = extent.width() as usize;
    let height = extent.height() as usize;
    if width < 8
        || height < 8
        || stride < width.saturating_mul(4)
        || pixels.len() < stride.saturating_mul(height)
    {
        return false;
    }
    let points = [
        (width / 4, height / 4),
        (width / 2, height / 4),
        (width * 3 / 4 - 1, height / 4),
        (width / 4, height / 2),
        (width / 2, height / 2),
        (width * 3 / 4 - 1, height / 2),
        (width / 4, height * 3 / 4 - 1),
        (width / 2, height * 3 / 4 - 1),
        (width * 3 / 4 - 1, height * 3 / 4 - 1),
    ];
    let first_offset = points[0].1 * stride + points[0].0 * 4;
    let first = &pixels[first_offset..first_offset + 3];
    points.iter().all(|(column, row)| {
        let offset = row * stride + column * 4;
        pixels[offset..offset + 3] == *first
    }) && fills.iter().any(|fill| {
        let wanted = [
            (fill & 0xff) as u8,
            ((fill >> 8) & 0xff) as u8,
            ((fill >> 16) & 0xff) as u8,
        ];
        first
            .iter()
            .zip(wanted)
            .all(|(seen, want)| seen.abs_diff(want) <= protocol::FILL_TOLERANCE)
    })
}

fn arguments() -> &'static Arguments {
    ARGUMENTS
        .get()
        .expect("benchmark arguments are initialized")
}

fn fixture_digest(set: WorkloadSet) -> ContentDigest {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let paths = fixture_sources(manifest, set);
    let mut combined = Vec::new();
    for path in paths {
        combined.extend_from_slice(
            &std::fs::read(&path)
                .unwrap_or_else(|error| panic!("fixture source {}: {error}", path.display())),
        );
    }
    ContentDigest::of(&combined)
}
