//! Phase 2 native capture, input, lifecycle, and common-flow measurements.
//!
//! This benchmark owns a repository fixture process and refuses any other
//! target. macOS capture and transition profiles change deterministic fixture
//! state only through the bounded private command plane; acknowledgements remain
//! separate from strictly newer-frame oracles. The interactive `System` input
//! set and Windows profile lineage retain their existing product-input stimulus.
//!
//! Ordinary `cargo test --all-targets` compiles this target and exits before it
//! opens a native capability. A measurement is explicit because it needs an
//! interactive, authorized release-target desktop and operator-supplied profile
//! conditions.
#[cfg(target_os = "macos")]
#[allow(dead_code, unreachable_pub, unused_imports)]
#[path = "support/macos_fixture.rs"]
mod macos_fixture;
#[cfg(target_os = "macos")]
use mado_pilot_platform_macos::fixture_control as macos_fixture_control;
#[cfg(target_os = "macos")]
use mado_pilot_platform_macos::fixture_protocol as macos_fixture_protocol;

use mado_pilot_testkit::bench_harness::Accounting;

#[global_allocator]
static ACCOUNTING: Accounting = Accounting;

fn main() {
    #[cfg(any(windows, target_os = "macos"))]
    native::run();

    #[cfg(not(any(windows, target_os = "macos")))]
    eprintln!("native-phase2 requires a declared MadoPilot release target");
}

#[cfg(any(windows, target_os = "macos"))]
mod native {
    #[cfg(windows)]
    use std::io::{BufRead, BufReader};
    #[cfg(target_os = "macos")]
    use std::ops::Deref;
    #[cfg(target_os = "macos")]
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    #[cfg(windows)]
    use std::process::{Child, Stdio};
    use std::rc::Rc;
    use std::sync::Arc;
    #[cfg(target_os = "macos")]
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    #[cfg(windows)]
    use std::sync::mpsc;
    use std::sync::{Mutex, OnceLock};
    use std::thread;
    use std::time::{Duration, Instant};

    const MAX_LANGUAGE_OUTPUT_BYTES: usize = 64 * 1_024;

    #[cfg(target_os = "macos")]
    static FIXTURE_FINALIZATION_SUCCEEDED: AtomicBool = AtomicBool::new(true);
    #[cfg(target_os = "macos")]
    static LANGUAGE_IDENTITY_FAILURE_REPORTED: AtomicBool = AtomicBool::new(false);
    #[cfg(target_os = "macos")]
    static LANGUAGE_OUTPUT_FAILURE_REPORTED: AtomicBool = AtomicBool::new(false);

    #[cfg(windows)]
    use mado_pilot::NativeEngineRequest;
    #[cfg(target_os = "macos")]
    use mado_pilot::{ActivityTag, FrameStamp};
    use mado_pilot::{
        CleanupState, ContentDigest, DeliveryPlan, Engine, FocusPolicy, Frame, FrameRequest,
        InputDelivery, InputEvent, InputOpenRequest, InputOperationKind, InputRequest,
        InputRequirement, InputSequence, Key, OpenRequest, OperationContext, PixelExtent,
        PixelFormat, SequenceOutcome, Session, SessionRequest, Status, TargetId,
    };
    #[cfg(target_os = "macos")]
    use mado_pilot_testkit::bench_harness::bounded_child_output_checked;
    use mado_pilot_testkit::bench_harness::{
        self, Benchmark, BoundedChildOutput, Plan, Profile, Sample, Workload, argument,
        enforce_hard_budgets, measure,
    };
    #[cfg(target_os = "macos")]
    use mado_pilot_testkit::bench_harness::{
        PHASE2_2_CAPTURE_LATENCY_BUDGETS, PHASE2_2_PROCESS_APPKIT_LATENCY_BUDGETS,
        PHASE2_2_PROCESS_DIAGNOSTIC_LATENCY_BUDGETS, PHASE2_2_PROCESS_GAME_LIKE_LATENCY_BUDGETS,
        PHASE2_2_PROCESS_HEAP_LIMIT_BYTES, PHASE2_2_TRANSITION_LATENCY_BUDGETS,
        PHASE2_PRODUCTION_CAPTURE_LATENCY_BUDGETS, PHASE2_PRODUCTION_TRANSITION_LATENCY_BUDGETS,
        enforce_latency_budgets,
    };

    #[cfg(windows)]
    use mado_pilot_testkit::bench_harness::{
        PrefixedLineMatch, bounded_child_output, classify_prefixed_line,
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
    use crate::macos_fixture_protocol as protocol;
    #[cfg(windows)]
    use mado_pilot::{
        CapabilitySupport, CoordinateSpace, InputAddressScope, InputReceipt, Point, PointerButton,
        SequenceLimits, SubmissionEvidence, TargetKind,
    };

    #[cfg(target_os = "macos")]
    use crate::macos_fixture::{
        CancellationObservation, CommandAcknowledgement, FixtureController, LanguageExecutablePin,
        LaunchMode, auxiliary_window_setup_is_proven, language_pins_are_unchanged,
        post_use_identity_gate,
    };
    #[cfg(target_os = "macos")]
    use mado_pilot::{
        CancellationToken, CapabilitySupport, CoordinateSpace, DiagnosticDrain, DiagnosticKind,
        DiagnosticLevel, DiagnosticOptions, DiagnosticReader, InputAddressScope, InputFault,
        InputReceipt, Point, PointerGeometry, SubmissionEvidence, TargetKind,
    };
    #[cfg(windows)]
    use mado_pilot_platform_windows::fixture_protocol as protocol;

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

    #[cfg(windows)]
    type NativeEngine = Engine;

    #[cfg(target_os = "macos")]
    const FIXTURE_COMMAND_BOUND: Duration = Duration::from_millis(500);

    const OPERATION_WAIT: Duration = Duration::from_secs(2);
    const CLOSE_WAIT: Duration = Duration::from_secs(5);
    const PRESSURE_WAIT: Duration = Duration::from_millis(100);
    const FIXTURE_WAIT: Duration = Duration::from_secs(10);
    #[cfg(target_os = "macos")]
    const LANGUAGE_PROCESS_WAIT: Duration = Duration::from_secs(5);
    #[cfg(windows)]
    const LANGUAGE_PROCESS_WAIT: Duration = OPERATION_WAIT;

    #[cfg(windows)]
    const fn language_event(kind: u32, text_units: u32) -> protocol::EventSummary {
        protocol::EventSummary { kind, text_units }
    }

    #[cfg(target_os = "macos")]
    const fn language_event(kind: u32, text_units: u32) -> protocol::EventSummary {
        protocol::EventSummary {
            kind,
            text_units,
            correlation: 0,
        }
    }

    const COMMON_LANGUAGE_EVENTS: [protocol::EventSummary; 3] = [
        language_event(protocol::EVENT_POINTER_MOVE, 0),
        language_event(protocol::EVENT_KEY_DOWN, expected_key_units()),
        language_event(protocol::EVENT_KEY_UP, expected_key_units()),
    ];

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

    #[cfg(target_os = "macos")]
    const CPP_LANGUAGE_EVENTS: [protocol::EventSummary; 5] = [
        COMMON_LANGUAGE_EVENTS[0],
        COMMON_LANGUAGE_EVENTS[1],
        COMMON_LANGUAGE_EVENTS[2],
        language_event(protocol::EVENT_KEY_DOWN, 1),
        language_event(protocol::EVENT_KEY_UP, 1),
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
                Self::Capture => Plan::new(20, 200),
                Self::Transitions | Self::Input => Plan::new(5, 50),
                #[cfg(target_os = "macos")]
                Self::ProductionCapture => Plan::new(20, 200),
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
    #[cfg(target_os = "macos")]
    const fn input_workload_description() -> &'static str {
        "native Rust System input plus released C and C++ ProcessDirected common flows"
    }

    #[cfg(windows)]
    const fn input_workload_description() -> &'static str {
        "native interactive System input and the public Rust common flow"
    }

    #[cfg(target_os = "macos")]
    const fn input_queue_policy() -> &'static str {
        "session latest-wins queue depth 1; Rust System and C/C++ explicit ProcessDirected sequences execute serially with no fallback"
    }

    #[cfg(windows)]
    const fn input_queue_policy() -> &'static str {
        "session latest-wins queue depth 1; bounded input sequence executes serially"
    }

    #[cfg(target_os = "macos")]
    const fn capture_workload_description() -> &'static str {
        "native controlled capture, static retention, latest acquisition, and explicit CPU mapping"
    }

    #[cfg(windows)]
    const fn capture_workload_description() -> &'static str {
        "native steady capture, latest acquisition, and explicit CPU mapping"
    }

    #[cfg(target_os = "macos")]
    const fn transition_workload_description() -> &'static str {
        "native controlled open, retained-pressure recovery, resize, and close transitions"
    }

    #[cfg(windows)]
    const fn transition_workload_description() -> &'static str {
        "native open, retained-pressure recovery, resize, and close transitions"
    }

    #[cfg(target_os = "macos")]
    const fn capture_queue_policy() -> &'static str {
        "fixture command queue depth 1; session latest-wins queue depth 1; adapter finite retained-storage limit"
    }

    #[cfg(windows)]
    const fn capture_queue_policy() -> &'static str {
        "session latest-wins queue depth 1; adapter finite retained-storage limit"
    }

    #[cfg(target_os = "macos")]
    const fn transition_queue_policy() -> &'static str {
        "fixture command queue depth 1; session latest-wins queue depth 1; retained-pressure case fills the reported finite storage limit"
    }

    fn profile_correctness_oracle(set: WorkloadSet) -> &'static str {
        #[cfg(target_os = "macos")]
        if matches!(
            set,
            WorkloadSet::ProductionCapture | WorkloadSet::ProductionTransitions
        ) {
            return "every retained sample checks production frame identity, frame-authoritative geometry, declared fixture content, finite retained progress, exact mapping, or bounded lifecycle outcome; the resize command is stimulus only and never substitutes for a captured result";
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

    #[cfg(windows)]
    const fn transition_queue_policy() -> &'static str {
        "session latest-wins queue depth 1; retained-pressure case fills the reported finite storage limit"
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
                    "--workload-set must be capture, transitions, input, resize-allocation, \
                     process-directed, process-directed-game-like, or process-diagnostics"
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
                return Err(
                    "the fixture executable changed while provenance was recorded".to_owned(),
                );
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
        #[cfg(target_os = "macos")]
        Static,
        #[cfg(target_os = "macos")]
        GameLike,
    }

    impl FixtureBehavior {
        #[cfg(windows)]
        const fn argument(self) -> &'static str {
            match self {
                Self::Animate => "--animate-on-input",
                Self::AnimateAndResize => "--animate-and-resize-on-input",
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
                .arg(behavior.argument())
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap_or_else(|error| panic!("the benchmark fixture could not start: {error}"));
            let process_id = child.id();
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
            fixture
        }

        fn process_id(&self) -> u32 {
            self.child.id()
        }

        fn title(&self) -> String {
            protocol::fixture_title(self.process_id())
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
            let _killed = self.child.kill();
            let _waited = self.child.wait();
        }
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

        fn close_bounded(&self, wait: Duration) -> bool {
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
            if !controller.finish(FIXTURE_WAIT) {
                FIXTURE_FINALIZATION_SUCCEEDED.store(false, Ordering::Release);
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

        #[cfg(target_os = "macos")]
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

        #[cfg(target_os = "macos")]
        fn from_capture_fixture(fixture: Rc<FixtureProcess>) -> Self {
            let flow = Flow::from_fixture(fixture);
            #[cfg(target_os = "macos")]
            assert!(
                controlled_command_ok(&flow.fixture, protocol::FixtureCommandKind::YieldForeground,),
                "the controlled capture fixture yields foreground before sampling"
            );
            let session = open_benchmark_capture_session(&flow);
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
            println!(
                "native-phase2: compiled; measurement skipped because --bench was not requested"
            );
            return;
        }
        let parsed = Arguments::parse(&raw_arguments).unwrap_or_else(|error| {
            panic!(
                "{error}\n{}",
                concat!(
                    "usage: cargo bench --package mado-pilot --bench native-phase2 -- ",
                    "--workload-set <capture|transitions|input|production-capture|",
                    "production-transitions|resize-allocation|process-directed|",
                    "process-directed-game-like|process-diagnostics> ",
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
            let benchmark_static_identity_unchanged =
                executable_identity(&args.benchmark_executable)
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
            let fixture_finalization_succeeded =
                FIXTURE_FINALIZATION_SUCCEEDED.load(Ordering::Acquire);
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
                    "cargo bench, default features, debug_assertions={}; {}",
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
            WorkloadSet::ProcessDirected => {
                process_directed_workloads(plan, FixtureBehavior::Static)
            }
            #[cfg(target_os = "macos")]
            WorkloadSet::ProcessDirectedGameLike => {
                process_directed_workloads(plan, FixtureBehavior::GameLike)
            }
            #[cfg(target_os = "macos")]
            WorkloadSet::ProcessDiagnostics => process_diagnostic_workloads(plan),
        }
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
        let pressure_fixture = Rc::new(FixtureProcess::spawn(FixtureBehavior::Static));
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
                || Rc::clone(&pressure_fixture),
                production_retained_pressure_resume,
            ),
        ]
    }

    #[cfg(windows)]
    fn capture_workloads(plan: Plan) -> Vec<Workload> {
        vec![
            measure(
                "stimulus_to_frame",
                "one acknowledged deterministic fixture change produces a newer stamped frame carrying the opposite declared fill; intermediate publications are discarded and counted",
                plan,
                || {
                    ActiveFlow::from_fixture(Rc::new(FixtureProcess::spawn(
                        FixtureBehavior::Animate,
                    )))
                },
                stimulus_to_frame,
            ),
            measure(
                "latest_acquisition",
                "after an exact stimulated publication advances the producer, latest returns a same-stream frame no older than that publication and reports the observed sequence gap",
                plan,
                || {
                    ActiveFlow::from_fixture(Rc::new(FixtureProcess::spawn(
                        FixtureBehavior::Animate,
                    )))
                },
                latest_acquisition,
            ),
            measure(
                "cpu_map_bgra8",
                "the newer frame maps once to exact-size BGRA8 bytes carrying one declared fixture fill",
                plan,
                || {
                    ActiveFlow::from_fixture(Rc::new(FixtureProcess::spawn(
                        FixtureBehavior::Animate,
                    )))
                },
                cpu_map,
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
        let acknowledged =
            controlled_command_ok(fixture, protocol::FixtureCommandKind::OpenAuxiliary);
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
        let correct = first.is_ok()
            && second.is_ok()
            && session.is_closed()
            && flow.fixture.process_id() != 0;
        Sample::unmapped(elapsed, correct)
    }

    #[cfg(target_os = "macos")]
    fn fixture_controller_close(behavior: &FixtureBehavior) -> Sample {
        let fixture = FixtureProcess::spawn(*behavior);
        let started = Instant::now();
        let correct = fixture.close_bounded(measured_close_bound());
        Sample::unmapped(started.elapsed(), correct)
    }

    #[cfg(target_os = "macos")]
    fn complete_process_receipt(
        receipt: &InputReceipt,
        target: TargetId,
        submitted: usize,
    ) -> bool {
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

    struct StimulatedFrame {
        correct: bool,
        mapped: u64,
        #[cfg(target_os = "macos")]
        elapsed_after_acknowledgement: Duration,
    }

    fn advance_to_stimulated_frame(
        active: &ActiveFlow,
        state: &mut ActiveState,
    ) -> StimulatedFrame {
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
            && benchmark_mapping_fill(&mapping) == Some(state.fill);
        let mapped = mapping.bytes().len() as u64;
        state.last = frame;
        Sample::new(elapsed, correct, mapped).with_stale_work(delta.saturating_sub(1), delta)
    }

    #[cfg(target_os = "macos")]
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
            && benchmark_mapping_fill(&mapping) == Some(state.fill)
            && delta > 0;
        state.last = latest;
        Sample::new(elapsed, correct, mapping.bytes().len() as u64)
            .with_stale_work(delta.saturating_sub(1), delta)
    }

    #[cfg(target_os = "macos")]
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
            && benchmark_mapping_fill(&mapping) == Some(state.fill);
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

    #[cfg(target_os = "macos")]
    fn production_retained_pressure_resume(fixture: &Rc<FixtureProcess>) -> Sample {
        let flow = Flow::from_fixture(Rc::clone(fixture));
        let session = open_benchmark_capture_session(&flow);
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
        let correct = blocked.status() == Status::DeadlineExceeded && delta > 1;
        drop(retained);
        drop(resumed);
        let correct = correct && close(&session);
        Sample::unmapped(elapsed, correct).with_stale_work(delta.saturating_sub(1), delta)
    }

    #[cfg(target_os = "macos")]
    fn fixture_resize_command(fixture: &Rc<FixtureProcess>) -> Sample {
        let started = Instant::now();
        let correct = controlled_command_ok(fixture, protocol::FixtureCommandKind::Resize);
        Sample::unmapped(started.elapsed(), correct)
    }

    fn resize_recreation(active: &ActiveFlow) -> Sample {
        let mut state = lock_state(active);
        let before = state.last.stamp();
        let old_extent = state.last.descriptor().extent();
        let started = Instant::now();
        assert!(
            send_resize_stimulus(&active.flow, &state.session, before.geometry().value(),),
            "resize stimulus is acknowledged"
        );

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
        };
        let elapsed = started.elapsed();
        let correct = frame.stamp().epoch() > before.epoch()
            && frame.stamp().geometry() > before.geometry()
            && frame.transform().geometry() == frame.stamp().geometry();
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
            std::iter::repeat_n(InputEvent::PointerMove(point), SequenceLimits::MAX_EVENTS)
                .collect(),
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
    fn complete_ordinary_receipt(
        receipt: &InputReceipt,
        target: TargetId,
        submitted: usize,
    ) -> bool {
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
                    && stdout.lines().any(|line| {
                        line == format!("{} complete (load check)", program.example_name)
                    })
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

    #[cfg(target_os = "macos")]
    fn send_resize_stimulus(flow: &Flow, _session: &Session, _geometry: u64) -> bool {
        controlled_command_ok(&flow.fixture, protocol::FixtureCommandKind::Resize)
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

    #[cfg(target_os = "macos")]
    fn open_benchmark_capture_session(flow: &Flow) -> Session {
        flow.open_capture_session()
    }

    #[cfg(windows)]
    fn open_benchmark_capture_session(flow: &Flow) -> Session {
        flow.open_input_session()
    }

    #[cfg(target_os = "macos")]
    const fn input_fixture_behavior() -> FixtureBehavior {
        FixtureBehavior::Static
    }

    #[cfg(windows)]
    const fn input_fixture_behavior() -> FixtureBehavior {
        FixtureBehavior::Animate
    }

    #[cfg(target_os = "macos")]
    const fn pressure_fixture_behavior() -> FixtureBehavior {
        FixtureBehavior::Static
    }

    #[cfg(windows)]
    const fn pressure_fixture_behavior() -> FixtureBehavior {
        FixtureBehavior::AnimateAndResize
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
        if session.close(&bounded(CLOSE_WAIT)).is_err()
            && session.close(&bounded(CLOSE_WAIT)).is_err()
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
            WorkloadSet::ProcessDiagnostics => {
                PHASE2_2_PROCESS_DIAGNOSTIC_LATENCY_BUDGETS.as_slice()
            }
        };
        enforce_latency_budgets(workloads, latency);
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

    #[cfg(windows)]
    fn enforce_premeasurement_budgets(_set: WorkloadSet, _workloads: &[Workload]) {}

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

    #[cfg(windows)]
    const fn benchmark_phase(_set: WorkloadSet) -> &'static str {
        "2"
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

    #[cfg(windows)]
    fn profile_notes(_set: WorkloadSet, notes: &str) -> String {
        notes.to_owned()
    }

    #[cfg(target_os = "macos")]
    const fn fixture_build_profile() -> &'static str {
        "fixture cargo build --release --features private-fixture; signed .app; controlled NSWorkspace new-instance launch"
    }

    #[cfg(windows)]
    const fn fixture_build_profile() -> &'static str {
        "fixture cargo build --release"
    }
    #[cfg(target_os = "macos")]
    const fn measured_close_bound() -> Duration {
        Duration::from_secs(1)
    }

    #[cfg(windows)]
    const fn measured_close_bound() -> Duration {
        CLOSE_WAIT
    }

    fn mapping_matches_fills(
        pixels: &[u8],
        stride: usize,
        extent: PixelExtent,
        fills: &[u32],
    ) -> bool {
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

    #[cfg(target_os = "macos")]
    fn fixture_sources(manifest: &Path, _set: WorkloadSet) -> Vec<PathBuf> {
        vec![
            manifest.join("../platform/macos/src/bin/mado-pilot-macos-input-fixture.rs"),
            manifest.join("../platform/macos/src/fixture_protocol.rs"),
            manifest.join("../platform/macos/native/madopilot_macos_input_fixture.h"),
            manifest.join("../platform/macos/native/madopilot_macos_input_fixture.m"),
        ]
    }

    #[cfg(windows)]
    fn fixture_sources(manifest: &Path, set: WorkloadSet) -> Vec<PathBuf> {
        let mut sources = vec![
            manifest.join("../platform/windows/src/bin/mado-pilot-windows-input-fixture.rs"),
            manifest.join("../platform/windows/src/fixture_protocol.rs"),
        ];
        if set == WorkloadSet::Input {
            sources.push(
                manifest.join(
                    "../platform/windows/src/bin/mado-pilot-windows-window-message-fixture.rs",
                ),
            );
        }
        sources
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
            EngineWiring {
                engine: engine_id,
                capture: Arc::clone(&provider) as Arc<dyn CaptureProvider>,
                matcher: Matcher::new(Arc::new(backend)),
                loader: PackageLoader::new(),
                input: Some(Arc::clone(&provider) as Arc<dyn InputProvider>),
                permission: Some(Arc::new(MacosPermissionProbe::new()) as Arc<dyn PermissionProbe>),
            },
            EngineOptions::new().with_diagnostics(diagnostics),
        )
        .expect("the macOS benchmark engine builds");
        NativeEngine { engine, provider }
    }

    #[cfg(windows)]
    fn native_engine() -> NativeEngine {
        mado_pilot::windows_engine(NativeEngineRequest::new())
            .expect("the Windows benchmark engine builds")
    }
    #[cfg(target_os = "macos")]
    const fn c_example_name() -> &'static str {
        "macos-native-input"
    }

    #[cfg(windows)]
    const fn c_example_name() -> &'static str {
        "windows-native-input"
    }

    #[cfg(target_os = "macos")]
    const fn cpp_example_name() -> &'static str {
        "macos-native-input-cpp"
    }

    #[cfg(windows)]
    const fn cpp_example_name() -> &'static str {
        "windows-native-input-cpp"
    }

    #[cfg(target_os = "macos")]
    const fn cpp_correctness_oracle() -> &'static str {
        "the released C++ wrapper uses explicit ProcessDirected delivery and checks its invocation-only receipt separately from exact owned-fixture events"
    }

    #[cfg(windows)]
    const fn cpp_correctness_oracle() -> &'static str {
        "the released C++ wrapper submits the full bounded pointer, button, wheel, key, modifier, text, and delay flow through a fresh process"
    }

    #[cfg(target_os = "macos")]
    const fn cpp_receipt_line() -> &'static str {
        "receipt: outcome 1 submitted 5 evidence 1 cleanup 0"
    }

    #[cfg(windows)]
    const fn cpp_receipt_line() -> &'static str {
        "receipt: outcome 1 submitted 16 evidence 4 cleanup 0"
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

    #[cfg(windows)]
    fn require_permissions(_engine: &NativeEngine) {}

    #[cfg(target_os = "macos")]
    const fn input_delivery() -> InputDelivery {
        InputDelivery::System
    }

    #[cfg(windows)]
    const fn input_delivery() -> InputDelivery {
        InputDelivery::WindowMessage
    }

    #[cfg(target_os = "macos")]
    const fn focus_policy() -> FocusPolicy {
        FocusPolicy::ActivateIfRequired
    }

    #[cfg(windows)]
    const fn focus_policy() -> FocusPolicy {
        FocusPolicy::Preserve
    }

    #[cfg(target_os = "macos")]
    const fn expected_key_units() -> u32 {
        0
    }

    #[cfg(windows)]
    const fn expected_key_units() -> u32 {
        0
    }

    #[cfg(target_os = "macos")]
    const fn benchmark_fill_rgb() -> u32 {
        protocol::REPLACEMENT_FILL_RGB
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

    #[cfg(target_os = "macos")]
    fn profile_identity(set: WorkloadSet) -> (&'static str, String) {
        let id = match set {
            WorkloadSet::Capture => "phase-2-2-controlled-capture-aarch64-apple-darwin",
            WorkloadSet::Transitions => "phase-2-2-controlled-transitions-aarch64-apple-darwin",
            WorkloadSet::ResizeAllocation => "phase-2-macos-resize-allocation-aarch64-apple-darwin",
            WorkloadSet::Input => "phase-2-native-input-aarch64-apple-darwin",
            WorkloadSet::ProductionCapture => "phase-2-production-capture-aarch64-apple-darwin",
            WorkloadSet::ProductionTransitions => {
                "phase-2-production-transitions-aarch64-apple-darwin"
            }
            WorkloadSet::ProcessDirected => {
                "phase-2-2-process-directed-appkit-aarch64-apple-darwin"
            }
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

    #[cfg(windows)]
    fn profile_identity(set: WorkloadSet) -> (&'static str, String) {
        let id = match set {
            WorkloadSet::Capture => "phase-2-native-capture-x86_64-pc-windows-msvc",
            WorkloadSet::Transitions => "phase-2-native-transitions-x86_64-pc-windows-msvc",
            WorkloadSet::Input => "phase-2-native-input-x86_64-pc-windows-msvc",
        };
        let fixture = if set == WorkloadSet::Input {
            "crates/platform/windows dedicated and ordinary fixture Rust sources plus shared protocol"
        } else {
            "crates/platform/windows fixture Rust and protocol sources"
        };
        (id, fixture.to_owned())
    }
}
