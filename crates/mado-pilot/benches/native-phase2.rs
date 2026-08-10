//! Phase 2 native capture, input, lifecycle, and common-flow measurements.
//!
//! This benchmark owns a repository fixture process and refuses any other
//! target. The fixture changes only deterministic colour or size state after a
//! benchmark key stimulus, so every retained timing sample has an observable
//! content, identity, or event oracle without capturing unrelated desktop data.
//!
//! Ordinary `cargo test --all-targets` compiles this target and exits before it
//! opens a native capability. A measurement is explicit because it needs an
//! interactive, authorized release-target desktop and operator-supplied profile
//! conditions.

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
    use std::io::{BufRead, BufReader};
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::rc::Rc;
    use std::sync::{Mutex, OnceLock, mpsc};
    use std::thread;
    use std::time::{Duration, Instant};

    use mado_pilot::{
        ContentDigest, DeliveryPlan, Engine, FocusPolicy, Frame, FrameRequest, InputDelivery,
        InputEvent, InputOpenRequest, InputOperationKind, InputRequest, InputRequirement,
        InputSequence, Key, NativeEngineRequest, OpenRequest, OperationContext, PixelExtent,
        PixelFormat, SequenceOutcome, Session, SessionRequest, Status, TargetId,
    };
    use mado_pilot_testkit::bench_harness::{
        self, Benchmark, Plan, Profile, Sample, Workload, argument, enforce_hard_budgets, measure,
    };

    #[cfg(windows)]
    use mado_pilot::{CoordinateSpace, Point};
    #[cfg(target_os = "macos")]
    use mado_pilot_platform_macos::fixture_protocol as protocol;
    #[cfg(windows)]
    use mado_pilot_platform_windows::fixture_protocol as protocol;

    const OPERATION_WAIT: Duration = Duration::from_secs(2);
    const CLOSE_WAIT: Duration = Duration::from_secs(5);
    const PRESSURE_WAIT: Duration = Duration::from_millis(100);
    const FIXTURE_WAIT: Duration = Duration::from_secs(10);

    static ARGUMENTS: OnceLock<Arguments> = OnceLock::new();

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum WorkloadSet {
        Capture,
        Transitions,
        Input,
    }

    impl WorkloadSet {
        fn parse(value: &str) -> Option<Self> {
            match value {
                "capture" => Some(Self::Capture),
                "transitions" => Some(Self::Transitions),
                "input" => Some(Self::Input),
                _ => None,
            }
        }

        fn measured_plan(self) -> Plan {
            match self {
                Self::Capture => Plan::new(20, 200),
                Self::Transitions => Plan::new(5, 20),
                Self::Input => Plan::new(5, 50),
            }
        }

        const fn workload(self) -> &'static str {
            match self {
                Self::Capture => {
                    "native steady capture, latest acquisition, and explicit CPU mapping"
                }
                Self::Transitions => {
                    "native open, retained-pressure recovery, resize, and close transitions"
                }
                Self::Input => "native input and the public Rust common flow",
            }
        }

        const fn queue_policy(self) -> &'static str {
            match self {
                Self::Capture => {
                    "session latest-wins queue depth 1; adapter finite retained-storage limit"
                }
                Self::Transitions => {
                    "session latest-wins queue depth 1; retained-pressure case fills the reported finite storage limit"
                }
                Self::Input => {
                    "session latest-wins queue depth 1; bounded input sequence executes serially"
                }
            }
        }
    }

    #[derive(Debug)]
    struct Arguments {
        set: WorkloadSet,
        fixture_executable: PathBuf,
        c_executable: Option<PathBuf>,
        cpp_executable: Option<PathBuf>,
        hardware: String,
        os_version: String,
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
                    "--workload-set must be capture, transitions, or input".to_owned()
                })
            })?;
            let fixture_executable = PathBuf::from(required("--fixture-executable")?);
            if !fixture_executable.is_file() {
                return Err("--fixture-executable does not name a built fixture binary".to_owned());
            }
            let language_executable = |name: &str| -> Result<PathBuf, String> {
                let executable = PathBuf::from(required(name)?);
                if executable.is_file() {
                    Ok(executable)
                } else {
                    Err(format!("{name} does not name a built executable"))
                }
            };
            let (c_executable, cpp_executable) = if set == WorkloadSet::Input {
                (
                    Some(language_executable("--c-executable")?),
                    Some(language_executable("--cpp-executable")?),
                )
            } else {
                (None, None)
            };
            let hardware = required("--hardware")?;
            let os_version = required("--os-version")?;
            let source_revision = required("--source-revision")?;
            let source_tree = required("--source-tree")?;
            let toolchain = required("--toolchain")?;
            let gpu_driver = required("--gpu-driver")?;
            let display_topology = required("--display-topology")?;
            let permissions_signing = required("--permissions-signing")?;
            let language_memory = if set == WorkloadSet::Input {
                "; C/C++ peak_allocated_bytes counts only harness-side Rust allocations, while peak_resident_bytes is the child process peak reported by the native OS after owned-handle cleanup"
            } else {
                ""
            };
            let notes = format!(
                "source commit {source_revision}, tree {source_tree}; toolchain {toolchain}; GPU/driver {gpu_driver}; display topology {display_topology}; permissions/signing {permissions_signing}; fixture path deliberately omitted{language_memory}"
            );
            Ok(Self {
                set,
                fixture_executable,
                hardware,
                os_version,
                c_executable,
                cpp_executable,
                notes,
            })
        }
    }

    #[derive(Debug, Clone, Copy)]
    enum FixtureBehavior {
        Animate,
        AnimateAndResize,
    }

    impl FixtureBehavior {
        const fn argument(self) -> &'static str {
            match self {
                Self::Animate => "--animate-on-input",
                Self::AnimateAndResize => "--animate-and-resize-on-input",
            }
        }
    }

    struct FixtureProcess {
        child: Child,
        lines: mpsc::Receiver<String>,
    }

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
            let fixture = Self { child, lines };
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
        fn title(&self) -> String {
            protocol::fixture_title(self.child.id())
        }

        fn next_common_flow(&self) -> bool {
            self.next_pointer_move() && self.next_key_pair()
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

    impl Drop for FixtureProcess {
        fn drop(&mut self) {
            let _killed = self.child.kill();
            let _waited = self.child.wait();
        }
    }

    struct Flow {
        engine: Engine,
        target: TargetId,
        fixture: Rc<FixtureProcess>,
    }

    impl Flow {
        fn from_fixture(fixture: Rc<FixtureProcess>) -> Self {
            let process_id = fixture.child.id();
            let engine = native_engine();
            require_permissions(&engine);
            let selection_deadline = Instant::now() + FIXTURE_WAIT;
            let target = loop {
                let targets = engine
                    .discover(&bounded(OPERATION_WAIT))
                    .expect("the benchmark fixture is discoverable");
                if let Ok(target) = protocol::select_unique_fixture(&targets, process_id) {
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
    #[derive(Clone)]
    struct LanguageProgram {
        executable: PathBuf,
        #[cfg(windows)]
        library_directory: PathBuf,
        example_name: &'static str,
        receipt_line: &'static str,
    }

    impl LanguageProgram {
        fn new(
            executable: PathBuf,
            example_name: &'static str,
            receipt_line: &'static str,
        ) -> Self {
            #[cfg(windows)]
            let library_directory = executable
                .parent()
                .and_then(Path::parent)
                .filter(|directory| directory.join("madopilot.dll").is_file())
                .unwrap_or_else(|| {
                    panic!(
                        "{} has no madopilot.dll in its cargo profile directory",
                        executable.display()
                    )
                })
                .to_path_buf();
            Self {
                executable,
                #[cfg(windows)]
                library_directory,
                example_name,
                receipt_line,
            }
        }

        fn command(&self) -> Command {
            let command = Command::new(&self.executable);
            #[cfg(windows)]
            let command = {
                let mut command = command;
                let existing = std::env::var_os("PATH").unwrap_or_default();
                let mut search = vec![self.library_directory.clone()];
                search.extend(std::env::split_paths(&existing));
                command.env(
                    "PATH",
                    std::env::join_paths(search)
                        .expect("the Windows child library path is representable"),
                );
                command
            };
            command
        }
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
                    "--workload-set <capture|transitions|input> ",
                    "--fixture-executable <path> [--c-executable <path> ",
                    "--cpp-executable <path>] --hardware <description> ",
                    "--os-version <description> --source-revision <commit> ",
                    "--source-tree <tree> --toolchain <versions> ",
                    "--gpu-driver <description> --display-topology <description> ",
                    "--permissions-signing <description>\n",
                    "(the C and C++ executables are required for the input set)",
                )
            )
        });
        let set = parsed.set;
        ARGUMENTS
            .set(parsed)
            .expect("benchmark arguments initialize exactly once");
        let plan = set.measured_plan();
        let workloads = workloads(set, plan);
        let args = arguments();
        let (id, fixture) = profile_identity(set);
        bench_harness::report(
            &Benchmark {
                id,
                workload: set.workload(),
                phase: "2",
            },
            &Profile {
                fixture: fixture.to_owned(),
                fixture_sha256: fixture_digest().to_string(),
                hardware: args.hardware.clone(),
                os_version: args.os_version.clone(),
                build_profile: format!(
                    "cargo bench, default features, debug_assertions={}; fixture cargo build --release",
                    cfg!(debug_assertions)
                ),
                correctness_oracle: "every retained sample checks frame identity/content, transition state, receipt/event sequence, or complete common-flow outcome as its measurement states",
                queue_policy: set.queue_policy(),
                notes: Some(args.notes.clone()),
            },
            plan,
            &workloads,
        );
        enforce_hard_budgets(&workloads);
    }

    fn workloads(set: WorkloadSet, plan: Plan) -> Vec<Workload> {
        match set {
            WorkloadSet::Capture => {
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
            WorkloadSet::Transitions => {
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
            WorkloadSet::Input => {
                let fixture = Rc::new(FixtureProcess::spawn(FixtureBehavior::Animate));
                let args = arguments();
                let c = LanguageProgram::new(
                    args.c_executable
                        .clone()
                        .expect("the input benchmark requires its C executable"),
                    c_example_name(),
                    "receipt: outcome 1 submitted 4 fault 0 cleanup 0",
                );
                let cpp = LanguageProgram::new(
                    args.cpp_executable
                        .clone()
                        .expect("the input benchmark requires its C++ executable"),
                    cpp_example_name(),
                    cpp_receipt_line(),
                );
                vec![
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
                        "the released C ABI performs the same bounded fixture flow in a fresh process",
                        plan,
                        || c.clone(),
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
                        "the released C++ wrapper performs the same bounded fixture flow in a fresh process",
                        plan,
                        || cpp.clone(),
                        language_common_flow,
                    ),
                ]
            }
        }
    }

    struct StimulatedFrame {
        correct: bool,
        mapped: u64,
    }

    fn advance_to_stimulated_frame(
        active: &ActiveFlow,
        state: &mut ActiveState,
    ) -> StimulatedFrame {
        let before = state.last.stamp();
        let expected_fill = alternate_benchmark_fill(state.fill)
            .expect("the retained fixture fill is one of the two declared states");
        let receipt_ok = send_frame_stimulus(&state.session);
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
                let event_ok = active.flow.fixture.next_key_pair();
                let stamp = frame.stamp();
                let correct = receipt_ok
                    && event_ok
                    && intermediate_content_ok
                    && stamp.stream() == before.stream()
                    && stamp.epoch() == before.epoch()
                    && stamp.sequence() > before.sequence();
                state.last = frame;
                state.fill = expected_fill;
                return StimulatedFrame { correct, mapped };
            }
            intermediate_content_ok &= fill.is_some();
            cursor = frame.stamp();
        }
    }

    fn stimulus_to_frame(active: &ActiveFlow) -> Sample {
        let mut state = lock_state(active);
        let before = state.last.stamp();
        let started = Instant::now();
        let stimulated = advance_to_stimulated_frame(active, &mut state);
        let elapsed = started.elapsed();
        let after = state.last.stamp();
        let delta = after
            .sequence()
            .value()
            .saturating_sub(before.sequence().value());
        Sample::new(elapsed, stimulated.correct, stimulated.mapped)
            .with_stale_work(delta.saturating_sub(1), delta)
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
        let session = flow.open_input_session();
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
        let flow = Flow::from_fixture(Rc::new(FixtureProcess::spawn(
            FixtureBehavior::AnimateAndResize,
        )));
        let session = flow.open_input_session();
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

    fn resize_recreation(active: &ActiveFlow) -> Sample {
        let mut state = lock_state(active);
        let before = state.last.stamp();
        let old_extent = state.last.descriptor().extent();
        let started = Instant::now();
        assert!(
            send_resize_stimulus(&state.session, before.geometry().value()),
            "resize stimulus returns a complete receipt"
        );
        assert!(
            resize_stimulus_observed(&active.flow.fixture),
            "the fixture observes the resize stimulus"
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
        let session = flow.open_input_session();
        let _seed = session
            .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
            .expect("the close sample owns a live session");
        let started = Instant::now();
        let first = session.close(&bounded(CLOSE_WAIT));
        let elapsed = started.elapsed();
        let second = session.close(&bounded(CLOSE_WAIT));
        let correct = first.is_ok() && second.is_ok() && session.is_closed();
        Sample::unmapped(elapsed, correct)
    }

    fn input_request_receipt(active: &ActiveFlow) -> Sample {
        let state = lock_state(active);
        let started = Instant::now();
        let receipt_ok = send_key_pair(&state.session);
        let elapsed = started.elapsed();
        let correct = receipt_ok && active.flow.fixture.next_key_pair();
        Sample::unmapped(elapsed, correct)
    }

    fn rust_common_flow(flow: &Flow) -> Sample {
        let started = Instant::now();
        let session = flow.open_input_session();
        let frame = session
            .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
            .expect("the common flow acquires a frame");
        let mapping = frame
            .map(PixelFormat::Bgra8, &bounded(OPERATION_WAIT))
            .expect("the common flow maps its frame");
        let correct = mapping_is_benchmark_content(&mapping)
            && send_confirmed_stimulus(flow, &session)
            && close(&session);
        let elapsed = started.elapsed();
        Sample::new(elapsed, correct, mapping.bytes().len() as u64)
    }
    fn language_process_load(program: &LanguageProgram) -> Sample {
        let started = Instant::now();
        let output = program.command().arg("--load-check").output();
        let elapsed = started.elapsed();
        let (correct, peak_resident) = match output {
            Ok(output) if output.status.success() && output.stderr.is_empty() => {
                let stdout = String::from_utf8(output.stdout).ok();
                let peak = stdout.as_deref().and_then(language_peak_resident_bytes);
                let complete = stdout.as_deref().is_some_and(|stdout| {
                    language_abi_line_is_present(stdout, program.example_name)
                        && stdout.lines().any(|line| {
                            line == format!("{} complete (load check)", program.example_name)
                        })
                });
                (complete && peak.is_some_and(|bytes| bytes > 0), peak)
            }
            _ => (false, None),
        };
        let sample = Sample::unmapped(elapsed, correct);
        match peak_resident {
            Some(bytes) => sample.with_peak_resident_bytes(bytes),
            None => sample,
        }
    }

    fn language_common_flow(program: &LanguageProgram) -> Sample {
        let fixture = FixtureProcess::spawn(FixtureBehavior::Animate);
        let title = fixture.title();
        let started = Instant::now();
        let output = program.command().arg(title).output();
        let (process_succeeded, stderr_empty, stdout) = match output {
            Ok(output) => (
                output.status.success(),
                output.stderr.is_empty(),
                String::from_utf8(output.stdout).ok(),
            ),
            Err(_) => (false, false, None),
        };
        let receipt_present = stdout
            .as_deref()
            .is_some_and(|stdout| stdout.lines().any(|line| line == program.receipt_line));
        let fixture_acknowledged = if process_succeeded || receipt_present {
            fixture.next_common_flow()
        } else {
            false
        };
        let elapsed = started.elapsed();
        let mapped = stdout
            .as_deref()
            .and_then(language_mapping_bytes)
            .unwrap_or(0);
        let peak_resident = stdout.as_deref().and_then(language_peak_resident_bytes);
        let correct = process_succeeded
            && stderr_empty
            && fixture_acknowledged
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

    fn send_confirmed_stimulus(flow: &Flow, session: &Session) -> bool {
        assert!(
            send_frame_stimulus(session),
            "the frame stimulus returns a complete receipt"
        );
        assert!(
            flow.fixture.next_key_pair(),
            "the fixture observes the frame stimulus's balanced key pair"
        );
        true
    }

    #[cfg(target_os = "macos")]
    fn send_frame_stimulus(session: &Session) -> bool {
        let sequence =
            InputSequence::new(vec![InputEvent::Text("b".to_owned())]).expect("text is valid");
        send_sequence(session, sequence, 1)
    }

    #[cfg(windows)]
    fn send_frame_stimulus(session: &Session) -> bool {
        send_key_pair(session)
    }

    #[cfg(target_os = "macos")]
    fn send_resize_stimulus(session: &Session, _geometry: u64) -> bool {
        let sequence =
            InputSequence::new(vec![InputEvent::Text("bb".to_owned())]).expect("text is valid");
        send_sequence(session, sequence, 1)
    }

    #[cfg(windows)]
    fn send_resize_stimulus(session: &Session, geometry: u64) -> bool {
        let coordinate = if geometry.is_multiple_of(2) {
            24.0
        } else {
            48.0
        };
        let point = Point::new(CoordinateSpace::TargetLogical, coordinate, coordinate)
            .expect("the resize stimulus point is finite");
        let sequence =
            InputSequence::new(vec![InputEvent::PointerMove(point)]).expect("the move is valid");
        send_sequence(session, sequence, 1)
    }

    #[cfg(target_os = "macos")]
    fn resize_stimulus_observed(fixture: &FixtureProcess) -> bool {
        fixture.next_key_pair_with_units(2, 1)
    }

    #[cfg(windows)]
    fn resize_stimulus_observed(fixture: &FixtureProcess) -> bool {
        fixture.next_pointer_move()
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
            && receipt.cleanup() == mado_pilot::CleanupState::NotNeeded;
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

    fn fixture_digest() -> ContentDigest {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let paths = fixture_sources(manifest);
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
    fn fixture_sources(manifest: &Path) -> Vec<PathBuf> {
        vec![
            manifest.join("../platform/macos/src/bin/mado-pilot-macos-input-fixture.rs"),
            manifest.join("../platform/macos/src/fixture_protocol.rs"),
            manifest.join("../platform/macos/native/madopilot_macos_input_fixture.h"),
            manifest.join("../platform/macos/native/madopilot_macos_input_fixture.m"),
        ]
    }

    #[cfg(windows)]
    fn fixture_sources(manifest: &Path) -> Vec<PathBuf> {
        vec![
            manifest.join("../platform/windows/src/bin/mado-pilot-windows-input-fixture.rs"),
            manifest.join("../platform/windows/src/fixture_protocol.rs"),
        ]
    }

    #[cfg(target_os = "macos")]
    fn native_engine() -> Engine {
        mado_pilot::macos_engine(NativeEngineRequest::new())
            .expect("the macOS benchmark engine builds")
    }

    #[cfg(windows)]
    fn native_engine() -> Engine {
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
    const fn cpp_receipt_line() -> &'static str {
        "receipt: outcome 1 submitted 4 evidence 1 cleanup 0"
    }

    #[cfg(windows)]
    const fn cpp_receipt_line() -> &'static str {
        "receipt: outcome 1 submitted 4 evidence 4 cleanup 0"
    }

    #[cfg(target_os = "macos")]
    fn require_permissions(engine: &Engine) {
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
    fn require_permissions(_engine: &Engine) {}

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
        1
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

    #[cfg(target_os = "macos")]
    fn ready_line_is_approved(line: &str, process_id: u32) -> bool {
        protocol::fixture_ready_context_is_approved(line, process_id)
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

    #[cfg(target_os = "macos")]
    fn profile_identity(set: WorkloadSet) -> (&'static str, &'static str) {
        let id = match set {
            WorkloadSet::Capture => "phase-2-native-capture-aarch64-apple-darwin",
            WorkloadSet::Transitions => "phase-2-native-transitions-aarch64-apple-darwin",
            WorkloadSet::Input => "phase-2-native-input-aarch64-apple-darwin",
        };
        (
            id,
            "crates/platform/macos fixture Rust, protocol, header, and Objective-C sources",
        )
    }

    #[cfg(windows)]
    fn profile_identity(set: WorkloadSet) -> (&'static str, &'static str) {
        let id = match set {
            WorkloadSet::Capture => "phase-2-native-capture-x86_64-pc-windows-msvc",
            WorkloadSet::Transitions => "phase-2-native-transitions-x86_64-pc-windows-msvc",
            WorkloadSet::Input => "phase-2-native-input-x86_64-pc-windows-msvc",
        };
        (
            id,
            "crates/platform/windows fixture Rust and protocol sources",
        )
    }
}
