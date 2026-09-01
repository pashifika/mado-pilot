//! Benchmark-only controller for the separately linked private macOS fixture.
//!
//! This module is compiled into benchmark artifacts only. It owns one retained
//! fixture application, one bounded Unix-domain control connection, and one
//! outstanding command at a time. A fixture acknowledgement proves only that
//! the private command ran; callers must establish capture progress or product
//! input delivery through independent oracles.

use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::{DirBuilder, File, OpenOptions};
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::macos_fixture_control::{
    AuthenticatedFixtureProcess, ExecutableIdentity, FixtureApplicationLifetime,
    FixtureSocketDirectory, LaunchedFixtureApplication, authenticate_fixture_peer,
    executable_identity, next_fixture_run_nonce,
};
use crate::macos_fixture_protocol::{
    self as protocol, EVENT_FLAGS_CHANGED, EVENT_KEY_DOWN, EVENT_KEY_UP, EVENT_POINTER_MOVE,
    EVENT_POINTER_PRESS, EVENT_POINTER_RELEASE, EVENT_POINTER_SCROLL, EventSummary, EventTotals,
    FixtureCommand, FixtureCommandKind, FixtureCommandResult, FixtureMode, FixtureRenderer,
    MAX_CONTROL_LINE_BYTES, MAX_RECORDED_EVENTS, fixture_ready_facts, format_command_line,
    parse_command_result_line, parse_event_line_for_run,
};
use mado_pilot::CancellationToken;

const MAX_OUTPUT_LINE_BYTES: usize = 1_024;
const READER_QUEUE_CAPACITY: usize = MAX_RECORDED_EVENTS + 16;
const WAIT_SLICE: Duration = Duration::from_millis(25);
const DROP_WAIT: Duration = Duration::from_secs(1);
const GRACEFUL_CLOSE_WAIT: Duration = Duration::from_millis(100);
const EVENT_QUIET_WAIT: Duration = Duration::from_millis(100);
const MAX_FIXTURE_LAUNCH_ATTEMPTS: u32 = 3;

const CONTROLLED_BASE_LOGICAL_SIZE: (f64, f64) = (640.0, 452.0);
const CONTROLLED_RESIZED_LOGICAL_SIZE: (f64, f64) = (688.0, 484.0);
const POINT_COMPARISON_EPSILON: f64 = 0.5;

fn logical_size_matches(actual: (f64, f64), expected: (f64, f64)) -> bool {
    (actual.0 - expected.0).abs() <= POINT_COMPARISON_EPSILON
        && (actual.1 - expected.1).abs() <= POINT_COMPARISON_EPSILON
}

/// Returns the exact target geometry produced by the fixture's next private resize.
pub(crate) fn expected_controlled_resize_logical_size(current: (f64, f64)) -> Option<(f64, f64)> {
    if logical_size_matches(current, CONTROLLED_BASE_LOGICAL_SIZE) {
        Some(CONTROLLED_RESIZED_LOGICAL_SIZE)
    } else if logical_size_matches(current, CONTROLLED_RESIZED_LOGICAL_SIZE) {
        Some(CONTROLLED_BASE_LOGICAL_SIZE)
    } else {
        None
    }
}

/// Returns the content-view size corresponding to one declared fixture geometry.
pub(crate) fn controlled_content_logical_size(target: (f64, f64)) -> Option<(f64, f64)> {
    if !logical_size_matches(target, CONTROLLED_BASE_LOGICAL_SIZE)
        && !logical_size_matches(target, CONTROLLED_RESIZED_LOGICAL_SIZE)
    {
        return None;
    }
    let decoration_width = CONTROLLED_BASE_LOGICAL_SIZE.0 - protocol::WINDOW_POINTS.0;
    let decoration_height = CONTROLLED_BASE_LOGICAL_SIZE.1 - protocol::WINDOW_POINTS.1;
    Some((target.0 - decoration_width, target.1 - decoration_height))
}

/// Confirms frame-authoritative target geometry matches the fixture's declared state.
pub(crate) fn controlled_resize_logical_size_matches(
    actual: (f64, f64),
    expected: (f64, f64),
) -> bool {
    logical_size_matches(actual, expected)
}

fn remove_language_pin(directory: &Path, path: &Path) {
    let _writable = std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700));
    let _file_removed = std::fs::remove_file(path);
    let _directory_removed = std::fs::remove_dir(directory);
}

/// One unique controller-owned copy of a recorded language artifact.
///
/// The file lives in its own mode-0500 directory rather than beside a
/// caller-supplied artifact. Both directory and file descriptors retain the
/// created vnodes until the pin removes them on every exit path.
pub(crate) struct LanguageExecutablePin {
    directory: PathBuf,
    path: PathBuf,
    expected: Arc<[u8]>,
    _directory: File,
    _file: File,
    directory_device: u64,
    directory_inode: u64,
    device: u64,
    inode: u64,
}

impl LanguageExecutablePin {
    /// Creates one private pin from an artifact's recorded bytes.
    pub(crate) fn new(executable: &Path, expected: Arc<[u8]>) -> Result<Self, String> {
        let file_name = executable
            .file_name()
            .ok_or_else(|| "the language artifact has no file name".to_owned())?;
        let nonce = next_fixture_run_nonce()?;
        let directory = std::env::temp_dir().join(format!(
            "mado-pilot-language-pin-{}-{nonce}",
            std::process::id()
        ));
        let mut builder = DirBuilder::new();
        builder.mode(0o700);
        builder
            .create(&directory)
            .map_err(|_| "the language pin directory could not be created".to_owned())?;
        let path = directory.join(file_name);
        let mut writer = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o500)
            .open(&path)
        {
            Ok(writer) => writer,
            Err(_) => {
                remove_language_pin(&directory, &path);
                return Err("the language artifact pin could not be created".to_owned());
            }
        };
        if writer
            .write_all(expected.as_ref())
            .and_then(|()| writer.sync_all())
            .is_err()
        {
            drop(writer);
            remove_language_pin(&directory, &path);
            return Err("the language artifact pin could not be written".to_owned());
        }
        let retained = match File::open(&path) {
            Ok(retained) => retained,
            Err(_) => {
                drop(writer);
                remove_language_pin(&directory, &path);
                return Err("the language artifact pin could not be retained".to_owned());
            }
        };
        let metadata = match retained.metadata() {
            Ok(metadata) => metadata,
            Err(_) => {
                drop((writer, retained));
                remove_language_pin(&directory, &path);
                return Err("the language artifact pin identity could not be read".to_owned());
            }
        };
        if std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o500)).is_err() {
            drop((writer, retained));
            remove_language_pin(&directory, &path);
            return Err("the language pin directory could not become read-only".to_owned());
        }
        let retained_directory = match File::open(&directory) {
            Ok(retained_directory) => retained_directory,
            Err(_) => {
                drop((writer, retained));
                remove_language_pin(&directory, &path);
                return Err("the language pin directory could not be retained".to_owned());
            }
        };
        let directory_metadata = match retained_directory.metadata() {
            Ok(metadata) => metadata,
            Err(_) => {
                drop((writer, retained, retained_directory));
                remove_language_pin(&directory, &path);
                return Err("the language pin directory identity could not be read".to_owned());
            }
        };
        drop(writer);
        let guard = Self {
            directory,
            path,
            expected,
            _directory: retained_directory,
            _file: retained,
            directory_device: directory_metadata.dev(),
            directory_inode: directory_metadata.ino(),
            device: metadata.dev(),
            inode: metadata.ino(),
        };
        if !guard.is_unchanged() {
            return Err(
                "the language artifact pin did not preserve its identity and bytes".to_owned(),
            );
        }
        Ok(guard)
    }

    /// Path passed to the bounded child process.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Confirms that the private directory and artifact remain the created vnodes.
    pub(crate) fn is_unchanged(&self) -> bool {
        std::fs::symlink_metadata(&self.directory).is_ok_and(|metadata| {
            metadata.file_type().is_dir()
                && metadata.dev() == self.directory_device
                && metadata.ino() == self.directory_inode
                && metadata.mode() & 0o777 == 0o500
        }) && std::fs::symlink_metadata(&self.path).is_ok_and(|metadata| {
            metadata.file_type().is_file()
                && metadata.dev() == self.device
                && metadata.ino() == self.inode
                && metadata.mode() & 0o777 == 0o500
        }) && std::fs::read(&self.path)
            .ok()
            .as_deref()
            .is_some_and(|bytes| bytes == self.expected.as_ref())
    }
}

impl Drop for LanguageExecutablePin {
    fn drop(&mut self) {
        remove_language_pin(&self.directory, &self.path);
    }
}

/// Requires a completed use and every final artifact identity to remain valid.
#[must_use]
pub(crate) fn post_use_identity_gate(use_succeeded: bool, artifacts_unchanged: &[bool]) -> bool {
    use_succeeded
        && !artifacts_unchanged.is_empty()
        && artifacts_unchanged.iter().all(|unchanged| *unchanged)
}

/// Requires both retained language pins to remain the exact files created from
/// their recorded bytes.
#[must_use]
pub(crate) fn language_pins_are_unchanged(
    executable: &LanguageExecutablePin,
    library: &LanguageExecutablePin,
) -> bool {
    executable.is_unchanged() && library.is_unchanged()
}

/// Requires an acknowledged auxiliary-window command to be proven by inventory.
#[must_use]
pub(crate) fn auxiliary_window_setup_is_proven<T: PartialEq>(
    command_acknowledged: bool,
    authenticated_window_ids: &[T],
) -> bool {
    command_acknowledged
        && authenticated_window_ids
            .split_first()
            .is_some_and(|(first, rest)| rest.iter().any(|other| other != first))
}

/// How the independently linked fixture renders and whether correlated product
/// input is also allowed to drive a qualification-only visual transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    /// Static AppKit background outside correlated input qualification.
    Static,
    /// AppKit background whose benchmark stimulus is only a private command.
    ControlledStatic,
    /// The retained interactive `System` profile's input-driven animation.
    AnimateOnInput,
    /// The retained interactive `System` profile's animation and resize mode.
    AnimateAndResizeOnInput,
    /// Independently qualified dynamically loaded OpenGL renderer.
    GameLike,
    /// OpenGL renderer whose benchmark stimulus is only a private command.
    ControlledGameLike,
}

impl LaunchMode {
    fn arguments(self) -> &'static [&'static str] {
        match self {
            Self::Static => &[],
            Self::ControlledStatic => &["--independent-visual-stimulus"],
            Self::AnimateOnInput => &["--animate-on-input"],
            Self::AnimateAndResizeOnInput => &["--animate-and-resize-on-input"],
            Self::GameLike => &["--game-like"],
            Self::ControlledGameLike => &["--game-like", "--independent-visual-stimulus"],
        }
    }

    fn expected_facts(self) -> (FixtureMode, FixtureRenderer) {
        match self {
            Self::GameLike | Self::ControlledGameLike => {
                (FixtureMode::GameLike, FixtureRenderer::OpenGl)
            }
            Self::Static
            | Self::ControlledStatic
            | Self::AnimateOnInput
            | Self::AnimateAndResizeOnInput => {
                (FixtureMode::Default, FixtureRenderer::AppKitBackground)
            }
        }
    }

    /// Stable, non-sensitive profile fact emitted in benchmark metadata.
    pub const fn fact(self) -> &'static str {
        match self {
            Self::GameLike => "mode=game-like renderer=opengl",
            Self::ControlledGameLike => "mode=game-like renderer=opengl stimulus=private-control",
            Self::Static | Self::AnimateOnInput | Self::AnimateAndResizeOnInput => {
                "mode=default renderer=appkit-background"
            }
            Self::ControlledStatic => {
                "mode=default renderer=appkit-background stimulus=private-control"
            }
        }
    }
}

/// One correlated private command acknowledgement and its transport latency.
///
/// The result is never a product receipt or visual outcome.
#[derive(Debug, Clone, Copy)]
pub struct CommandAcknowledgement {
    result: FixtureCommandResult,
    elapsed: Duration,
}

impl CommandAcknowledgement {
    /// The exact private protocol result.
    pub const fn result(self) -> FixtureCommandResult {
        self.result
    }

    /// Time from the first command write through its correlated result line.
    pub const fn elapsed(self) -> Duration {
        self.elapsed
    }
}

/// Observation made by a helper that cancels only after one expected fixture
/// event has arrived.
#[derive(Debug, Clone, Copy)]
pub struct CancellationObservation {
    summary: Option<EventSummary>,
    cancelled_at: Option<Instant>,
}

impl CancellationObservation {
    /// The event summary that triggered cancellation, if the exact event arrived.
    pub const fn summary(self) -> Option<EventSummary> {
        self.summary
    }

    /// The monotonic instant immediately before cancellation was signalled.
    pub const fn cancelled_at(self) -> Option<Instant> {
        self.cancelled_at
    }
}

enum ReaderMessage {
    Line(String),
    Oversized,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FixtureCleanupDebt {
    None,
    Deferred,
}

impl FixtureCleanupDebt {
    const fn token(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Deferred => "deferred",
        }
    }
}

/// Immutable private fixture finalization facts consumed by qualification.
#[must_use = "fixture finalization must be checked before accepting a sample"]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FixtureFinalization {
    stop_acknowledged: bool,
    exit: FixtureExitObservation,
    bounded: bool,
    output_clean: bool,
    executable_unchanged: bool,
    cleanup_debt: FixtureCleanupDebt,
}

impl FixtureFinalization {
    const fn new(
        stop_acknowledged: bool,
        exit: FixtureExitObservation,
        bounded: bool,
        output_clean: bool,
        executable_unchanged: bool,
    ) -> Self {
        Self {
            stop_acknowledged,
            exit,
            bounded,
            output_clean,
            executable_unchanged,
            cleanup_debt: if exit.is_stopped() {
                FixtureCleanupDebt::None
            } else {
                FixtureCleanupDebt::Deferred
            },
        }
    }

    /// Returns true only when every finalization fact is terminal and clean.
    #[must_use]
    pub(crate) const fn is_accepted(&self) -> bool {
        self.stop_acknowledged
            && self.exit.is_stopped()
            && self.bounded
            && self.output_clean
            && self.executable_unchanged
            && matches!(self.cleanup_debt, FixtureCleanupDebt::None)
    }
}

pub(crate) fn finalize_once<ResultValue: Copy>(
    cached: &mut Option<ResultValue>,
    finalize: impl FnOnce() -> ResultValue,
) -> ResultValue {
    if let Some(result) = *cached {
        return result;
    }
    let result = finalize();
    *cached = Some(result);
    result
}

pub(crate) fn finalize_result_before_observing<ResultValue, Failure, Observation>(
    result: Result<ResultValue, Failure>,
    finalize: impl FnOnce() -> bool,
    observe: impl FnOnce() -> Observation,
) -> (Result<ResultValue, Failure>, bool, Observation) {
    let finalization_accepted = finalize();
    let observation = observe();
    (result, finalization_accepted, observation)
}

/// One owned fixture application and its bounded command/event channel.
pub struct FixtureController {
    launched: LaunchedFixtureApplication,
    application: AuthenticatedFixtureProcess,
    input: Option<UnixStream>,
    lines: Arc<Mutex<mpsc::Receiver<ReaderMessage>>>,
    reader: Option<thread::JoinHandle<()>>,
    reader_failed: Arc<AtomicBool>,
    pending_events: VecDeque<EventSummary>,
    run_nonce: u64,
    next_nonce: u64,
    launch_mode: LaunchMode,
    stopped: bool,
    expected_identity: ExecutableIdentity,
    finish_result: Option<FixtureFinalization>,
}

impl fmt::Debug for FixtureController {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FixtureController")
            .field("launch_mode", &self.launch_mode)
            .field("pending_events", &self.pending_events.len())
            .field("run_nonce", &self.run_nonce)
            .field("next_nonce", &self.next_nonce)
            .field("stopped", &self.stopped)
            .finish_non_exhaustive()
    }
}

fn strict_event_reset(pending_events_empty: bool, reset: impl FnOnce() -> bool) -> bool {
    pending_events_empty && reset()
}

fn discard_setup_events_until_quiet<State>(
    state: &mut State,
    wait: Duration,
    quiet_wait: Duration,
    mut reset: impl FnMut(&mut State, Duration) -> bool,
    mut discard_pending_events: impl FnMut(&mut State),
    mut output_remains_quiet: impl FnMut(&mut State, Instant) -> bool,
) -> bool {
    let Some(deadline) = Instant::now().checked_add(wait) else {
        return false;
    };
    while Instant::now() < deadline {
        if !reset(state, deadline.saturating_duration_since(Instant::now())) {
            return false;
        }
        discard_pending_events(state);
        let Some(quiet_deadline) = Instant::now()
            .checked_add(quiet_wait)
            .filter(|quiet_deadline| *quiet_deadline <= deadline)
        else {
            return false;
        };
        if output_remains_quiet(state, quiet_deadline) {
            return true;
        }
    }
    false
}

impl FixtureController {
    /// Launches one signed app bundle through NSWorkspace, binds the control
    /// peer to that exact retained application instance, and waits for the
    /// current versioned protocol ready facts.
    pub fn start(
        executable: &Path,
        expected_executable: Arc<[u8]>,
        expected_identity: ExecutableIdentity,
        launch_mode: LaunchMode,
        wait: Duration,
    ) -> Result<Self, String> {
        Self::start_with_max_attempts(
            executable,
            expected_executable,
            expected_identity,
            launch_mode,
            wait,
            MAX_FIXTURE_LAUNCH_ATTEMPTS,
        )
    }

    /// Launches exactly once for a no-retry qualification process.
    pub fn start_once(
        executable: &Path,
        expected_executable: Arc<[u8]>,
        expected_identity: ExecutableIdentity,
        launch_mode: LaunchMode,
        wait: Duration,
    ) -> Result<Self, String> {
        Self::start_with_max_attempts(
            executable,
            expected_executable,
            expected_identity,
            launch_mode,
            wait,
            1,
        )
    }

    fn start_with_max_attempts(
        executable: &Path,
        expected_executable: Arc<[u8]>,
        expected_identity: ExecutableIdentity,
        launch_mode: LaunchMode,
        wait: Duration,
        max_launch_attempts: u32,
    ) -> Result<Self, String> {
        let executable = executable
            .canonicalize()
            .map_err(|_| "the fixture executable cannot be canonicalized".to_owned())?;
        let bundle = fixture_bundle(&executable).ok_or_else(|| {
            "the fixture executable is not inside a .app/Contents/MacOS bundle".to_owned()
        })?;
        if std::fs::read(&executable)
            .ok()
            .as_deref()
            .is_none_or(|bytes| bytes != expected_executable.as_ref())
        {
            return Err("the fixture executable changed after provenance was recorded".to_owned());
        }
        if executable_identity(&executable)? != expected_identity {
            return Err(
                "the fixture code identity changed after provenance was recorded".to_owned(),
            );
        }
        let socket_directory = FixtureSocketDirectory::new()?;
        let socket_path = socket_directory.socket_path();
        let run_nonce = next_fixture_run_nonce()?;
        let listener = UnixListener::bind(&socket_path)
            .map_err(|_| "the fixture control listener could not bind".to_owned())?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "the fixture control listener could not be bounded".to_owned())?;

        let mut launch_arguments = vec![
            OsString::from("--control-socket"),
            socket_path.as_os_str().to_owned(),
            OsString::from("--run-nonce"),
            OsString::from(run_nonce.to_string()),
        ];
        launch_arguments.extend(launch_mode.arguments().iter().map(OsString::from));
        let argument_views = launch_arguments
            .iter()
            .map(OsString::as_os_str)
            .collect::<Vec<&OsStr>>();
        let launched = LaunchedFixtureApplication::launch(&bundle, &argument_views)?;
        let mut expected_process_id = launched.process_id();
        let mut launch_guard = LaunchGuard::new(launched);
        let mut launch_attempts = 1_u32;
        let deadline = Instant::now() + wait;
        let (stream, application, accepted_launch) = loop {
            if max_launch_attempts > 1
                && launch_guard.lifetime()? == FixtureApplicationLifetime::Lost
            {
                if launch_attempts >= max_launch_attempts || Instant::now() >= deadline {
                    return Err(format!(
                        "the fixture application exited before connecting after \
                         {launch_attempts} launch attempt(s)"
                    ));
                }
                eprintln!(
                    "fixture-launch-retry attempt={} reason=exited-before-control-connection",
                    launch_attempts + 1
                );
                drop(launch_guard);
                let launched = LaunchedFixtureApplication::launch(&bundle, &argument_views)?;
                expected_process_id = launched.process_id();
                launch_guard = LaunchGuard::new(launched);
                launch_attempts += 1;
                continue;
            }
            match listener.accept() {
                Ok((stream, _address)) => {
                    if let Some(application) =
                        authenticate_fixture_peer(&stream, expected_process_id, &executable)
                    {
                        let identity_matches = loop {
                            match application.executable_identity() {
                                Ok(identity) => break Some(identity == expected_identity),
                                Err(error) => {
                                    if !application.is_live() {
                                        break None;
                                    }
                                    if Instant::now() >= deadline {
                                        return Err(format!(
                                            "the launched fixture identity remained unavailable: \
                                             {error}"
                                        ));
                                    }
                                    thread::sleep(WAIT_SLICE);
                                }
                            }
                        };
                        let Some(identity_matches) = identity_matches else {
                            continue;
                        };
                        if !identity_matches {
                            return Err(
                                "the launched fixture identity differs from recorded provenance"
                                    .to_owned(),
                            );
                        }
                        launch_guard.application = Some(application);
                        let accepted_launch = wait_for_launched_live(&launch_guard, deadline)?;
                        break (stream, application, accepted_launch);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => return Err("the fixture control listener failed".to_owned()),
            }
            if Instant::now() >= deadline {
                return Err(
                    "the fixture application did not connect before the deadline".to_owned(),
                );
            }
            thread::sleep(WAIT_SLICE);
        };
        let process_id = application.process_id();
        stream
            .set_nonblocking(false)
            .map_err(|_| "the fixture control connection could not become blocking".to_owned())?;
        drop(listener);
        drop(socket_directory);
        let input = stream
            .try_clone()
            .map_err(|_| "the fixture control connection could not be cloned".to_owned())?;
        let (sender, receiver) = mpsc::sync_channel(READER_QUEUE_CAPACITY);
        let reader_failed = Arc::new(AtomicBool::new(false));
        let reader_failure = Arc::clone(&reader_failed);
        let reader = thread::Builder::new()
            .name("mado-pilot-benchmark-fixture-reader".to_owned())
            .spawn(move || read_bounded_lines(stream, &sender, &reader_failure))
            .map_err(|_| "the fixture output reader could not start".to_owned())?;
        let lines = Arc::new(Mutex::new(receiver));
        let ready = wait_for_ready(&lines, &reader_failed, deadline)?;
        let facts = fixture_ready_facts(&ready, process_id).ok_or_else(|| {
            format!(
                "the fixture ready record did not match protocol v{}",
                protocol::FIXTURE_CONTROL_VERSION
            )
        })?;
        let expected = launch_mode.expected_facts();
        if !facts.execution_context_is_approved()
            || facts.run_nonce() != run_nonce
            || facts.mode() != expected.0
            || facts.renderer() != expected.1
            || facts.window_number() == 0
        {
            return Err(
                "the fixture ready facts did not match the requested approved run and mode"
                    .to_owned(),
            );
        }

        let (launched, application) = launch_guard.take(accepted_launch);
        Ok(Self {
            launched,
            application,
            input: Some(input),
            lines,
            reader: Some(reader),
            reader_failed,
            pending_events: VecDeque::new(),
            run_nonce,
            next_nonce: 1,
            launch_mode,
            expected_identity,
            stopped: false,
            finish_result: None,
        })
    }

    /// Owned application process identity used only for fail-closed fixture selection.
    pub const fn process_id(&self) -> u32 {
        self.application.process_id()
    }

    /// Returns the still-live audit-token identity only while the authenticated
    /// control connection remains usable.
    pub fn authenticated_process(&self) -> Option<AuthenticatedFixtureProcess> {
        if self.stopped
            || self.input.is_none()
            || self.reader_failed.load(Ordering::Acquire)
            || !self.application.is_live()
            || !self
                .application
                .matches_executable_identity(self.expected_identity)
        {
            return None;
        }
        Some(self.application)
    }

    /// The explicit renderer/mode fact validated from the ready record.
    pub const fn launch_mode(&self) -> LaunchMode {
        self.launch_mode
    }

    /// Sends exactly one private command and waits for its matching result.
    pub fn command(
        &mut self,
        kind: FixtureCommandKind,
        wait: Duration,
    ) -> Result<CommandAcknowledgement, String> {
        self.send_command(kind, 0, wait)
    }

    fn send_command(
        &mut self,
        kind: FixtureCommandKind,
        event_payload_tag: u64,
        wait: Duration,
    ) -> Result<CommandAcknowledgement, String> {
        if self.stopped {
            return Err("the fixture controller is already stopped".to_owned());
        }
        let nonce = self.next_nonce;
        self.next_nonce = self
            .next_nonce
            .checked_add(1)
            .ok_or_else(|| "the fixture command identity is exhausted".to_owned())?;

        let encoded = format_command_line(FixtureCommand {
            run_nonce: self.run_nonce,
            nonce,
            event_payload_tag,
            kind,
        });
        if encoded.len().saturating_add(1) > MAX_CONTROL_LINE_BYTES {
            return Err("the encoded fixture command exceeds its fixed line bound".to_owned());
        }
        let input = self
            .input
            .as_mut()
            .ok_or_else(|| "the fixture command channel is closed".to_owned())?;
        let started = Instant::now();
        writeln!(input, "{encoded}")
            .and_then(|()| input.flush())
            .map_err(|_| "the fixture command could not be written".to_owned())?;
        let deadline = started + wait;
        loop {
            let message = recv_message(&self.lines, &self.reader_failed, deadline)?;
            match message {
                ReaderMessage::Line(line) => {
                    if let Some(event) = parse_event_line_for_run(&line, self.run_nonce) {
                        if self.pending_events.len() == MAX_RECORDED_EVENTS {
                            return Err("the bounded fixture event buffer is full".to_owned());
                        }
                        self.pending_events.push_back(event);
                    } else if let Some(result) = parse_command_result_line(&line) {
                        if result.run_nonce != self.run_nonce || result.nonce != nonce {
                            return Err(
                                "the fixture returned an out-of-run or out-of-order command result"
                                    .to_owned(),
                            );
                        }
                        return Ok(CommandAcknowledgement {
                            result,
                            elapsed: started.elapsed(),
                        });
                    } else if line.starts_with("fixture-command-") {
                        return Err("the fixture rejected or malformed a command".to_owned());
                    } else {
                        return Err("the fixture emitted an unexpected protocol record".to_owned());
                    }
                }
                ReaderMessage::Oversized => {
                    return Err("the fixture emitted an oversized protocol line".to_owned());
                }
                ReaderMessage::Failed => {
                    return Err("the fixture output reader failed".to_owned());
                }
            }
        }
    }

    /// Fences and clears bounded AppKit event summaries observed during a
    /// capture-only watcher run.
    ///
    /// The exact process totals must equal the already bounded event records
    /// before they are cleared. A reset acknowledgement then proves later
    /// cleanup cannot inherit those observations. Event payloads are counts
    /// only; text is never retained by the fixture protocol.
    pub fn discard_watch_events(&mut self, wait: Duration) -> bool {
        let deadline = Instant::now() + wait;
        let Some(report) = self
            .command(
                FixtureCommandKind::ReadEvents,
                deadline.saturating_duration_since(Instant::now()),
            )
            .ok()
            .map(CommandAcknowledgement::result)
            .filter(|result| result.status == 0)
        else {
            return false;
        };
        let observed = event_totals(self.pending_events.make_contiguous());
        if observed != Some(report.events) {
            return false;
        }
        self.pending_events.clear();
        self.reset_events(0, deadline.saturating_duration_since(Instant::now()))
    }
    /// Resets the fixture's bounded event counters and refuses queued prior-run output.
    pub fn reset_events(&mut self, event_payload_tag: u64, wait: Duration) -> bool {
        let pending_events_empty = self.pending_events.is_empty();
        strict_event_reset(pending_events_empty, || {
            self.send_command(FixtureCommandKind::ResetEvents, event_payload_tag, wait)
                .is_ok_and(|ack| {
                    ack.result.status == 0
                        && ack.result.events == EventTotals::default()
                        && self.pending_events.is_empty()
                })
        })
    }

    /// Establishes the first measurement baseline after fixture-only window setup.
    ///
    /// Event lines ordered before the successful reset acknowledgement belong to
    /// setup, not product input. Each fresh language fixture uses this fence only
    /// before its timed sample.
    pub fn discard_setup_events(&mut self, wait: Duration) -> bool {
        discard_setup_events_until_quiet(
            self,
            wait,
            EVENT_QUIET_WAIT,
            |controller, remaining| {
                controller
                    .send_command(FixtureCommandKind::PrepareLanguageFlow, 0, remaining)
                    .is_ok_and(|ack| {
                        ack.result.status == 0 && ack.result.events == EventTotals::default()
                    })
            },
            |controller| controller.pending_events.clear(),
            |controller, quiet_deadline| controller.output_remains_quiet(quiet_deadline),
        )
    }

    /// Reads one exact event row and verifies the fixture's independent totals.
    pub fn events_are_exact(&mut self, expected: &[EventSummary], wait: Duration) -> bool {
        self.remaining_events_are_exact(expected, expected, wait)
    }

    /// Reads an exact suffix after an exclusive observer consumed a known prefix,
    /// then verifies the fixture's cumulative independent totals for the full row.
    pub fn remaining_events_are_exact(
        &mut self,
        expected_remaining: &[EventSummary],
        expected_total: &[EventSummary],
        wait: Duration,
    ) -> bool {
        let deadline = Instant::now() + wait;
        let observed = self.event_summaries(
            expected_remaining.len(),
            deadline.saturating_duration_since(Instant::now()),
        );
        let Some(totals) = event_totals(expected_total) else {
            return false;
        };
        let expected_correlation = expected_total.first().map_or(0, |event| event.correlation);
        let correlation_is_exact = expected_total
            .iter()
            .all(|event| event.correlation == expected_correlation);
        let observed_report = self
            .command(
                FixtureCommandKind::ReadEvents,
                deadline.saturating_duration_since(Instant::now()),
            )
            .ok()
            .filter(|ack| ack.result.status == 0)
            .map(|ack| ack.result);
        let report_is_exact = observed_report.is_some_and(|report| {
            report.events == totals
                && (expected_correlation == 0
                    || (correlation_is_exact
                        && report.event_correlation == expected_correlation
                        && report.event_payload_matches))
        });
        let quiet = Instant::now()
            .checked_add(EVENT_QUIET_WAIT)
            .filter(|quiet_deadline| *quiet_deadline <= deadline)
            .is_some_and(|quiet_deadline| self.output_remains_quiet(quiet_deadline));
        let exact = observed == expected_remaining
            && report_is_exact
            && self.pending_events.is_empty()
            && quiet;
        if !exact {
            eprintln!(
                "fixture event oracle mismatch: expected={expected_remaining:?} observed={observed:?} \
                 report-exact={report_is_exact} pending={} quiet={quiet}",
                self.pending_events.len()
            );
        }
        exact
    }

    fn output_remains_quiet(&self, deadline: Instant) -> bool {
        while Instant::now() < deadline {
            if self.reader_failed.load(Ordering::Acquire) {
                return false;
            }
            let received = self
                .lines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .recv_timeout(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(WAIT_SLICE),
                );
            match received {
                Ok(_) | Err(mpsc::RecvTimeoutError::Disconnected) => return false,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
        !self.reader_failed.load(Ordering::Acquire)
    }

    /// Returns exactly `count` bounded event summaries or fewer on timeout/failure.
    pub fn event_summaries(&mut self, count: usize, wait: Duration) -> Vec<EventSummary> {
        let mut events = Vec::with_capacity(count.min(MAX_RECORDED_EVENTS));
        while events.len() < count {
            if let Some(event) = self.pending_events.pop_front() {
                events.push(event);
            } else {
                break;
            }
        }
        let deadline = Instant::now() + wait;
        while events.len() < count && Instant::now() < deadline {
            let Ok(message) = recv_message(&self.lines, &self.reader_failed, deadline) else {
                break;
            };
            match message {
                ReaderMessage::Line(line) => {
                    if let Some(event) = parse_event_line_for_run(&line, self.run_nonce) {
                        events.push(event);
                    } else {
                        self.reader_failed.store(true, Ordering::Release);
                        break;
                    }
                }
                ReaderMessage::Oversized | ReaderMessage::Failed => {
                    self.reader_failed.store(true, Ordering::Release);
                    break;
                }
            }
        }
        events
    }

    /// Starts one helper that cancels only after the exact event arrives.
    ///
    /// The receiver is exclusive while the helper runs. Callers must join it
    /// before reading another acknowledgement or event.
    pub fn cancel_after_event(
        &mut self,
        expected: EventSummary,
        cancellation: CancellationToken,
        wait: Duration,
    ) -> Result<thread::JoinHandle<CancellationObservation>, String> {
        if !self.pending_events.is_empty() {
            return Err("stale fixture events precede the cleanup sample".to_owned());
        }
        if self.reader_failed.load(Ordering::Acquire) {
            return Err("the fixture output protocol already failed".to_owned());
        }
        {
            let receiver = self
                .lines
                .lock()
                .map_err(|_| "the fixture output receiver is poisoned".to_owned())?;
            match receiver.try_recv() {
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err("the fixture output channel is disconnected".to_owned());
                }
                Ok(_) => return Err("queued fixture output precedes the cleanup sample".to_owned()),
            }
        }
        let lines = Arc::clone(&self.lines);
        let run_nonce = self.run_nonce;
        let reader_failed = Arc::clone(&self.reader_failed);
        thread::Builder::new()
            .name("mado-pilot-benchmark-cleanup-trigger".to_owned())
            .spawn(move || {
                let deadline = Instant::now() + wait;
                let Ok(message) = recv_message(&lines, &reader_failed, deadline) else {
                    return CancellationObservation {
                        summary: None,
                        cancelled_at: None,
                    };
                };
                let ReaderMessage::Line(line) = message else {
                    return CancellationObservation {
                        summary: None,
                        cancelled_at: None,
                    };
                };
                let Some(summary) = parse_event_line_for_run(&line, run_nonce) else {
                    reader_failed.store(true, Ordering::Release);
                    return CancellationObservation {
                        summary: None,
                        cancelled_at: None,
                    };
                };
                if summary != expected {
                    return CancellationObservation {
                        summary: Some(summary),
                        cancelled_at: None,
                    };
                }
                let cancelled_at = Instant::now();
                cancellation.cancel();
                CancellationObservation {
                    summary: Some(summary),
                    cancelled_at: Some(cancelled_at),
                }
            })
            .map_err(|_| "the cleanup observation helper could not start".to_owned())
    }
    /// Stops the private fixture, terminates its exact launched application when
    /// needed, and returns every bounded finalization fact. Idempotent.
    pub(crate) fn finish(&mut self, wait: Duration) -> FixtureFinalization {
        if let Some(result) = self.finish_result {
            return result;
        }
        let deadline = Instant::now() + wait;
        let executable_unchanged = self
            .application
            .matches_executable_identity(self.expected_identity);
        let stop_acknowledged = self
            .command(
                FixtureCommandKind::Stop,
                deadline.saturating_duration_since(Instant::now()),
            )
            .is_ok_and(|ack| ack.result.status == 0);
        self.shutdown_input();
        let graceful_deadline = deadline.min(Instant::now() + GRACEFUL_CLOSE_WAIT);
        let mut exit = wait_for_authenticated_application_exit(
            &self.application,
            &self.launched,
            graceful_deadline,
        );
        if !exit.is_stopped() {
            exit = terminate_authenticated_application(
                &mut self.application,
                &mut self.launched,
                deadline,
            );
        }
        let process_stopped_in_time = exit.is_stopped() && Instant::now() <= deadline;
        self.stopped = exit.is_stopped();

        let output_clean = finish_reader_output_is_clean(
            self.reader.take(),
            &self.lines,
            &self.reader_failed,
            self.pending_events.is_empty(),
            deadline,
        );
        let bounded = process_stopped_in_time && Instant::now() <= deadline;
        let result = FixtureFinalization::new(
            stop_acknowledged,
            exit,
            bounded,
            output_clean,
            executable_unchanged,
        );
        if !result.is_accepted() {
            eprintln!(
                "fixture-finalization-failed stop-acknowledged={} authenticated={} launched={} \
                 exact-process-stopped={} bounded={} output-clean={} \
                 executable-identity-unchanged={} cleanup-debt={} pending-events={} \
                 reader-failed={}",
                result.stop_acknowledged,
                result.exit.authenticated.token(),
                result.exit.launched.token(),
                result.exit.is_stopped(),
                result.bounded,
                result.output_clean,
                result.executable_unchanged,
                result.cleanup_debt.token(),
                self.pending_events.len(),
                self.reader_failed.load(Ordering::Acquire),
            );
        }
        self.finish_result = Some(result);
        result
    }

    fn shutdown_input(&mut self) {
        if let Some(input) = self.input.take() {
            let _shutdown = input.shutdown(Shutdown::Both);
        }
    }

    fn terminate_owned(&mut self) {
        self.shutdown_input();
        let deadline = Instant::now() + DROP_WAIT;
        self.stopped = terminate_authenticated_application(
            &mut self.application,
            &mut self.launched,
            deadline,
        )
        .is_stopped();
        let _output_clean = finish_reader_output_is_clean(
            self.reader.take(),
            &self.lines,
            &self.reader_failed,
            self.pending_events.is_empty(),
            deadline,
        );
    }
}
fn event_totals(events: &[EventSummary]) -> Option<EventTotals> {
    let mut totals = EventTotals::default();
    for event in events {
        let count = match event.kind {
            EVENT_POINTER_MOVE => &mut totals.pointer_moves,
            EVENT_POINTER_PRESS => &mut totals.pointer_presses,
            EVENT_POINTER_RELEASE => &mut totals.pointer_releases,
            EVENT_POINTER_SCROLL => &mut totals.pointer_scrolls,
            EVENT_KEY_DOWN => &mut totals.key_downs,
            EVENT_KEY_UP => &mut totals.key_ups,
            EVENT_FLAGS_CHANGED => &mut totals.flags_changed,
            _ => return None,
        };
        *count = count.checked_add(1)?;
        totals.text_units = totals.text_units.checked_add(u64::from(event.text_units))?;
    }
    Some(totals)
}

impl Drop for FixtureController {
    fn drop(&mut self) {
        if !self.stopped {
            self.terminate_owned();
        }
    }
}

fn finish_reader_output_is_clean(
    reader: Option<thread::JoinHandle<()>>,
    lines: &Arc<Mutex<mpsc::Receiver<ReaderMessage>>>,
    reader_failed: &AtomicBool,
    pending_events_empty: bool,
    deadline: Instant,
) -> bool {
    let mut output_clean = pending_events_empty && !reader_failed.load(Ordering::Acquire);
    if let Some(reader) = reader {
        while !reader.is_finished() && Instant::now() < deadline {
            let received = lines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .recv_timeout(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(WAIT_SLICE),
                );
            match received {
                Ok(_message) => output_clean = false,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        if reader.is_finished() {
            output_clean &= reader.join().is_ok();
        } else {
            output_clean = false;
        }
    }
    loop {
        let received = lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .try_recv();
        match received {
            Ok(_message) => output_clean = false,
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => break,
        }
    }
    output_clean && !reader_failed.load(Ordering::Acquire)
}

fn wait_for_ready(
    lines: &Arc<Mutex<mpsc::Receiver<ReaderMessage>>>,
    reader_failed: &AtomicBool,
    deadline: Instant,
) -> Result<String, String> {
    match recv_message(lines, reader_failed, deadline)? {
        ReaderMessage::Line(line) if line.starts_with("fixture-ready ") => Ok(line),
        ReaderMessage::Line(_other) => {
            Err("the fixture emitted a protocol record before readiness".to_owned())
        }
        ReaderMessage::Oversized => Err("the fixture emitted an oversized ready record".to_owned()),
        ReaderMessage::Failed => Err("the fixture output reader failed".to_owned()),
    }
}

fn recv_message(
    lines: &Arc<Mutex<mpsc::Receiver<ReaderMessage>>>,
    reader_failed: &AtomicBool,
    deadline: Instant,
) -> Result<ReaderMessage, String> {
    loop {
        if reader_failed.load(Ordering::Acquire) {
            return Err("the fixture output protocol failed".to_owned());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("the fixture protocol operation exceeded its deadline".to_owned());
        }
        let received = lines
            .lock()
            .map_err(|_| "the fixture output receiver is poisoned".to_owned())?
            .recv_timeout(remaining.min(WAIT_SLICE));
        match received {
            Ok(message) => return Ok(message),
            Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return Err("the fixture protocol operation exceeded its deadline".to_owned());
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("the fixture output channel disconnected".to_owned());
            }
        }
    }
}

fn read_bounded_lines(
    mut stream: UnixStream,
    sender: &mpsc::SyncSender<ReaderMessage>,
    reader_failed: &AtomicBool,
) {
    let mut line = Vec::with_capacity(MAX_OUTPUT_LINE_BYTES);
    let mut byte = [0u8; 1];
    let mut overflow = false;
    loop {
        match stream.read(&mut byte) {
            Ok(0) => {
                if overflow || !line.is_empty() {
                    reader_failed.store(true, Ordering::Release);
                    let _sent = sender.try_send(ReaderMessage::Failed);
                }
                return;
            }
            Ok(_) if byte[0] == b'\n' => {
                if overflow {
                    reader_failed.store(true, Ordering::Release);
                    let _sent = sender.try_send(ReaderMessage::Oversized);
                    return;
                }
                let Ok(decoded) = String::from_utf8(std::mem::take(&mut line)) else {
                    reader_failed.store(true, Ordering::Release);
                    let _sent = sender.try_send(ReaderMessage::Failed);
                    return;
                };
                if sender.try_send(ReaderMessage::Line(decoded)).is_err() {
                    reader_failed.store(true, Ordering::Release);
                    return;
                }
                line = Vec::with_capacity(MAX_OUTPUT_LINE_BYTES);
            }
            Ok(_) if !overflow && line.len() < MAX_OUTPUT_LINE_BYTES.saturating_sub(1) => {
                line.push(byte[0]);
            }
            Ok(_) => overflow = true,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => {
                reader_failed.store(true, Ordering::Release);
                let _sent = sender.try_send(ReaderMessage::Failed);
                return;
            }
        }
    }
}

fn fixture_bundle(executable: &Path) -> Option<PathBuf> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app")
        .then(|| bundle.to_path_buf())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthenticatedProcessLifetime {
    Live,
    Lost,
}

impl AuthenticatedProcessLifetime {
    const fn token(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Lost => "lost",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaunchedApplicationLifetime {
    NotObserved,
    Unknown,
    Live,
    Lost,
    ObservationFailed,
}

impl LaunchedApplicationLifetime {
    const fn token(self) -> &'static str {
        match self {
            Self::NotObserved => "not-observed",
            Self::Unknown => "unknown",
            Self::Live => "live",
            Self::Lost => "lost",
            Self::ObservationFailed => "observation-failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FixtureExitObservation {
    authenticated: AuthenticatedProcessLifetime,
    launched: LaunchedApplicationLifetime,
}

impl FixtureExitObservation {
    const fn is_stopped(self) -> bool {
        matches!(self.authenticated, AuthenticatedProcessLifetime::Lost)
            && matches!(self.launched, LaunchedApplicationLifetime::Lost)
    }
}

fn observe_fixture_exit_with(
    authenticated_is_live: impl FnOnce() -> bool,
    launched_lifetime: impl FnOnce() -> Result<FixtureApplicationLifetime, String>,
) -> FixtureExitObservation {
    if authenticated_is_live() {
        return FixtureExitObservation {
            authenticated: AuthenticatedProcessLifetime::Live,
            launched: LaunchedApplicationLifetime::NotObserved,
        };
    }
    let launched = match launched_lifetime() {
        Ok(FixtureApplicationLifetime::Unknown) => LaunchedApplicationLifetime::Unknown,
        Ok(FixtureApplicationLifetime::Live) => LaunchedApplicationLifetime::Live,
        Ok(FixtureApplicationLifetime::Lost) => LaunchedApplicationLifetime::Lost,
        Err(_) => LaunchedApplicationLifetime::ObservationFailed,
    };
    FixtureExitObservation {
        authenticated: AuthenticatedProcessLifetime::Lost,
        launched,
    }
}

fn observe_fixture_exit(
    application: &AuthenticatedFixtureProcess,
    launched: &LaunchedFixtureApplication,
) -> FixtureExitObservation {
    observe_fixture_exit_with(|| application.is_live(), || launched.lifetime())
}

fn wait_for_launched_application_exit(
    application: &LaunchedFixtureApplication,
    deadline: Instant,
) -> bool {
    loop {
        if matches!(application.is_live(), Ok(false)) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_authenticated_application_exit(
    application: &AuthenticatedFixtureProcess,
    launched: &LaunchedFixtureApplication,
    deadline: Instant,
) -> FixtureExitObservation {
    loop {
        let observation = observe_fixture_exit(application, launched);
        if observation.is_stopped() || Instant::now() >= deadline {
            return observation;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn terminate_authenticated_application(
    application: &mut AuthenticatedFixtureProcess,
    launched: &mut LaunchedFixtureApplication,
    deadline: Instant,
) -> FixtureExitObservation {
    let _authenticated_terminated = application.terminate();
    let _launched_terminated = launched.terminate();
    let term_deadline = deadline.min(Instant::now() + GRACEFUL_CLOSE_WAIT);
    let observation = wait_for_authenticated_application_exit(application, launched, term_deadline);
    if observation.is_stopped() {
        return observation;
    }
    let _authenticated_killed = application.kill();
    let _launched_killed = launched.kill();
    wait_for_authenticated_application_exit(application, launched, deadline)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FixtureLaunchAcceptanceError {
    Lost,
    ObservationFailed,
    DeadlineExceeded,
}

impl FixtureLaunchAcceptanceError {
    const fn message(self) -> &'static str {
        match self {
            Self::Lost => {
                "the fixture application exited before its launched lifetime became observable"
            }
            Self::ObservationFailed => "the launched fixture lifetime could not be established",
            Self::DeadlineExceeded => {
                "the launched fixture lifetime did not become observable before the deadline"
            }
        }
    }
}

fn wait_for_launched_live_with(
    mut observe: impl FnMut() -> Result<FixtureApplicationLifetime, String>,
    mut deadline_expired: impl FnMut() -> bool,
    mut wait: impl FnMut(),
) -> Result<(), FixtureLaunchAcceptanceError> {
    loop {
        match observe().map_err(|_| FixtureLaunchAcceptanceError::ObservationFailed)? {
            FixtureApplicationLifetime::Live if deadline_expired() => {
                return Err(FixtureLaunchAcceptanceError::DeadlineExceeded);
            }
            FixtureApplicationLifetime::Live => return Ok(()),
            FixtureApplicationLifetime::Lost => {
                return Err(FixtureLaunchAcceptanceError::Lost);
            }
            FixtureApplicationLifetime::Unknown if deadline_expired() => {
                return Err(FixtureLaunchAcceptanceError::DeadlineExceeded);
            }
            FixtureApplicationLifetime::Unknown => wait(),
        }
    }
}

struct AcceptedFixtureLaunch;

fn wait_for_launched_live(
    guard: &LaunchGuard,
    deadline: Instant,
) -> Result<AcceptedFixtureLaunch, String> {
    wait_for_launched_live_with(
        || guard.lifetime(),
        || Instant::now() >= deadline,
        || thread::sleep(WAIT_SLICE),
    )
    .map_err(|failure| failure.message().to_owned())?;
    Ok(AcceptedFixtureLaunch)
}

struct LaunchGuard {
    launched: Option<LaunchedFixtureApplication>,
    application: Option<AuthenticatedFixtureProcess>,
}

impl LaunchGuard {
    fn new(launched: LaunchedFixtureApplication) -> Self {
        Self {
            launched: Some(launched),
            application: None,
        }
    }

    fn lifetime(&self) -> Result<FixtureApplicationLifetime, String> {
        self.launched
            .as_ref()
            .ok_or_else(|| "the guarded launched application is missing".to_owned())?
            .lifetime()
    }

    fn take(
        mut self,
        _accepted: AcceptedFixtureLaunch,
    ) -> (LaunchedFixtureApplication, AuthenticatedFixtureProcess) {
        let application = self
            .application
            .take()
            .expect("the authenticated fixture process exists");
        let launched = self
            .launched
            .take()
            .expect("the guarded launched application exists");
        (launched, application)
    }
}

impl Drop for LaunchGuard {
    fn drop(&mut self) {
        let deadline = Instant::now() + DROP_WAIT;
        if let (Some(application), Some(launched)) =
            (self.application.as_mut(), self.launched.as_mut())
        {
            let _exit = terminate_authenticated_application(application, launched, deadline);
            return;
        }
        if let Some(launched) = self.launched.as_mut() {
            let _terminated = launched.terminate();
            let term_deadline = deadline.min(Instant::now() + GRACEFUL_CLOSE_WAIT);
            if !wait_for_launched_application_exit(launched, term_deadline) {
                let _killed = launched.kill();
                let _stopped = wait_for_launched_application_exit(launched, deadline);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AuthenticatedProcessLifetime, FixtureCleanupDebt, FixtureExitObservation,
        FixtureFinalization, FixtureLaunchAcceptanceError, LanguageExecutablePin,
        LaunchedApplicationLifetime, MAX_OUTPUT_LINE_BYTES, ReaderMessage,
        auxiliary_window_setup_is_proven, controlled_content_logical_size,
        controlled_resize_logical_size_matches, discard_setup_events_until_quiet,
        expected_controlled_resize_logical_size, finalize_once, finalize_result_before_observing,
        finish_reader_output_is_clean, fixture_bundle, language_pins_are_unchanged,
        next_fixture_run_nonce, observe_fixture_exit_with, post_use_identity_gate,
        read_bounded_lines, strict_event_reset, wait_for_launched_live_with,
    };
    use crate::macos_fixture_control::FixtureApplicationLifetime;
    use crate::macos_fixture_protocol::{EVENT_KEY_DOWN, EventSummary, format_event_line};
    use std::cell::Cell;
    use std::collections::VecDeque;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex, mpsc};
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn fixture_bundle_derivation_accepts_only_the_expected_layout() {
        assert_eq!(
            fixture_bundle(Path::new(
                "/tmp/MadoPilotFixture.app/Contents/MacOS/mado-pilot-macos-input-fixture"
            )),
            Some(PathBuf::from("/tmp/MadoPilotFixture.app"))
        );
        assert_eq!(
            fixture_bundle(Path::new("/tmp/mado-pilot-macos-input-fixture")),
            None
        );
    }

    #[test]
    fn controlled_resize_geometry_tracks_the_fixture_toggle() {
        assert_eq!(
            expected_controlled_resize_logical_size((640.0, 451.75)),
            Some((688.0, 484.0)),
        );
        assert_eq!(
            expected_controlled_resize_logical_size((687.999_98, 483.75)),
            Some((640.0, 452.0)),
        );
        assert!(controlled_resize_logical_size_matches(
            (688.0, 483.75),
            (688.0, 484.0),
        ));
        assert!(controlled_resize_logical_size_matches(
            (640.0, 451.75),
            (640.0, 452.0),
        ));
        assert_eq!(
            controlled_content_logical_size((640.0, 451.75)),
            Some((640.0, 419.75)),
        );
        assert_eq!(
            controlled_content_logical_size((687.999_98, 483.75)),
            Some((687.999_98, 451.75)),
        );
    }

    #[test]
    fn controlled_resize_geometry_rejects_unknown_or_wrong_geometry() {
        assert_eq!(
            expected_controlled_resize_logical_size((700.0, 484.0)),
            None,
        );
        assert!(!controlled_resize_logical_size_matches(
            (688.0, 480.0),
            (688.0, 484.0),
        ));
        assert_eq!(controlled_content_logical_size((700.0, 484.0)), None);
    }

    #[test]
    fn launch_acceptance_waits_for_unknown_then_accepts_live_without_retry() {
        let mut lifetimes = VecDeque::from([
            FixtureApplicationLifetime::Unknown,
            FixtureApplicationLifetime::Live,
        ]);
        let mut waits = 0;

        assert_eq!(
            wait_for_launched_live_with(
                || Ok(lifetimes.pop_front().expect("the fixed lifetime arrives")),
                || false,
                || waits += 1,
            ),
            Ok(())
        );
        assert_eq!(waits, 1);
        assert!(lifetimes.is_empty());
    }

    #[test]
    fn launch_acceptance_rejects_lost_without_waiting() {
        let mut waited = false;
        assert_eq!(
            wait_for_launched_live_with(
                || Ok(FixtureApplicationLifetime::Lost),
                || false,
                || waited = true,
            ),
            Err(FixtureLaunchAcceptanceError::Lost)
        );
        assert!(!waited);
    }

    #[test]
    fn launch_acceptance_rejects_observation_failure_without_waiting() {
        let mut waited = false;
        assert_eq!(
            wait_for_launched_live_with(
                || Err("unretained native detail".to_owned()),
                || false,
                || waited = true,
            ),
            Err(FixtureLaunchAcceptanceError::ObservationFailed)
        );
        assert!(!waited);
    }

    #[test]
    fn launch_acceptance_rejects_unknown_at_the_existing_deadline() {
        let mut observations = 0;
        let mut deadline_checks = VecDeque::from([false, true]);
        let mut waits = 0;

        assert_eq!(
            wait_for_launched_live_with(
                || {
                    observations += 1;
                    Ok(FixtureApplicationLifetime::Unknown)
                },
                || {
                    deadline_checks
                        .pop_front()
                        .expect("the fixed deadline observation arrives")
                },
                || waits += 1,
            ),
            Err(FixtureLaunchAcceptanceError::DeadlineExceeded)
        );
        assert_eq!(observations, 2);
        assert_eq!(waits, 1);
        assert!(deadline_checks.is_empty());
    }

    #[test]
    fn launch_acceptance_rejects_live_observed_after_the_deadline() {
        assert_eq!(
            wait_for_launched_live_with(
                || Ok(FixtureApplicationLifetime::Live),
                || true,
                || panic!("an expired acceptance must not wait"),
            ),
            Err(FixtureLaunchAcceptanceError::DeadlineExceeded)
        );
    }

    #[test]
    fn accepted_live_lifetime_becomes_exact_lost_after_fast_exit() {
        let mut lifetimes = VecDeque::from([
            FixtureApplicationLifetime::Unknown,
            FixtureApplicationLifetime::Live,
            FixtureApplicationLifetime::Lost,
        ]);

        assert_eq!(
            wait_for_launched_live_with(
                || Ok(lifetimes.pop_front().expect("the startup lifetime arrives")),
                || false,
                || {},
            ),
            Ok(())
        );
        assert_eq!(
            observe_fixture_exit_with(
                || false,
                || Ok(lifetimes.pop_front().expect("the exit lifetime arrives")),
            ),
            FixtureExitObservation {
                authenticated: AuthenticatedProcessLifetime::Lost,
                launched: LaunchedApplicationLifetime::Lost,
            }
        );
        assert!(lifetimes.is_empty());
    }

    #[test]
    fn exact_exit_observation_is_lazy_and_requires_both_lifetimes_lost() {
        let mut launched_probes = 0;
        assert_eq!(
            observe_fixture_exit_with(
                || true,
                || {
                    launched_probes += 1;
                    Ok(FixtureApplicationLifetime::Lost)
                },
            ),
            FixtureExitObservation {
                authenticated: AuthenticatedProcessLifetime::Live,
                launched: LaunchedApplicationLifetime::NotObserved,
            }
        );
        assert_eq!(launched_probes, 0);

        assert_eq!(
            observe_fixture_exit_with(|| false, || Ok(FixtureApplicationLifetime::Lost)),
            FixtureExitObservation {
                authenticated: AuthenticatedProcessLifetime::Lost,
                launched: LaunchedApplicationLifetime::Lost,
            }
        );
        assert_eq!(
            observe_fixture_exit_with(|| false, || Ok(FixtureApplicationLifetime::Unknown)),
            FixtureExitObservation {
                authenticated: AuthenticatedProcessLifetime::Lost,
                launched: LaunchedApplicationLifetime::Unknown,
            }
        );
        assert_eq!(
            observe_fixture_exit_with(|| false, || Err("unretained native detail".to_owned()),),
            FixtureExitObservation {
                authenticated: AuthenticatedProcessLifetime::Lost,
                launched: LaunchedApplicationLifetime::ObservationFailed,
            }
        );
    }

    #[test]
    fn typed_finalization_rejects_each_incomplete_fact() {
        let stopped = FixtureExitObservation {
            authenticated: AuthenticatedProcessLifetime::Lost,
            launched: LaunchedApplicationLifetime::Lost,
        };
        let accepted = FixtureFinalization::new(true, stopped, true, true, true);
        assert!(accepted.is_accepted());
        assert_eq!(accepted.cleanup_debt, FixtureCleanupDebt::None);

        assert!(
            !FixtureFinalization {
                stop_acknowledged: false,
                ..accepted
            }
            .is_accepted()
        );
        assert!(
            !FixtureFinalization {
                exit: FixtureExitObservation {
                    authenticated: AuthenticatedProcessLifetime::Live,
                    launched: LaunchedApplicationLifetime::NotObserved,
                },
                ..accepted
            }
            .is_accepted()
        );
        assert!(
            !FixtureFinalization {
                exit: FixtureExitObservation {
                    authenticated: AuthenticatedProcessLifetime::Lost,
                    launched: LaunchedApplicationLifetime::Unknown,
                },
                ..accepted
            }
            .is_accepted()
        );
        assert!(
            !FixtureFinalization {
                bounded: false,
                ..accepted
            }
            .is_accepted()
        );
        assert!(
            !FixtureFinalization {
                output_clean: false,
                ..accepted
            }
            .is_accepted()
        );
        assert!(
            !FixtureFinalization {
                executable_unchanged: false,
                ..accepted
            }
            .is_accepted()
        );
        assert!(
            !FixtureFinalization {
                cleanup_debt: FixtureCleanupDebt::Deferred,
                ..accepted
            }
            .is_accepted()
        );

        let deferred = FixtureFinalization::new(
            true,
            FixtureExitObservation {
                authenticated: AuthenticatedProcessLifetime::Lost,
                launched: LaunchedApplicationLifetime::Unknown,
            },
            true,
            true,
            true,
        );
        assert_eq!(deferred.cleanup_debt, FixtureCleanupDebt::Deferred);
        assert!(!deferred.is_accepted());
    }

    #[test]
    fn accepted_sample_finalizes_before_observing_latency() {
        let phase = Cell::new(0);
        let (result, accepted, observed) = finalize_result_before_observing(
            Ok::<_, ()>(()),
            || {
                assert_eq!(phase.get(), 0);
                phase.set(1);
                false
            },
            || {
                assert_eq!(phase.get(), 1);
                phase.set(2);
                7
            },
        );
        assert_eq!(result, Ok(()));
        assert!(!accepted);
        assert_eq!(observed, 7);
        assert_eq!(phase.get(), 2);
    }

    #[test]
    fn failed_sample_finalizes_before_propagating_operation_error() {
        let phase = Cell::new(0);
        let (result, accepted, observed) = finalize_result_before_observing(
            Err::<(), _>("typed_operation_failure"),
            || {
                assert_eq!(phase.get(), 0);
                phase.set(1);
                true
            },
            || {
                assert_eq!(phase.get(), 1);
                phase.set(2);
                11
            },
        );
        assert_eq!(result, Err("typed_operation_failure"));
        assert!(accepted);
        assert_eq!(observed, 11);
        assert_eq!(phase.get(), 2);
    }

    #[test]
    fn finalization_cache_invokes_cleanup_once() {
        let calls = Cell::new(0);
        let mut cached = None;
        let first = finalize_once(&mut cached, || {
            calls.set(calls.get() + 1);
            17
        });
        let second = finalize_once(&mut cached, || {
            calls.set(calls.get() + 1);
            23
        });
        assert_eq!(first, 17);
        assert_eq!(second, 17);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn language_pin_pair_rejects_either_replaced_file() {
        fn pin_pair() -> (LanguageExecutablePin, LanguageExecutablePin) {
            let parent = std::env::temp_dir();
            let executable = LanguageExecutablePin::new(
                &parent.join("mado-pilot-language-executable-source"),
                Arc::from(b"recorded executable".as_slice()),
            )
            .expect("create the executable pin");
            let library = LanguageExecutablePin::new(
                &parent.join("mado-pilot-language-library-source"),
                Arc::from(b"recorded library".as_slice()),
            )
            .expect("create the library pin");
            (executable, library)
        }

        fn set_pin_directory_mode(pin: &LanguageExecutablePin, mode: u32) {
            std::fs::set_permissions(&pin.directory, std::fs::Permissions::from_mode(mode))
                .expect("change the pin directory mode for mutation");
        }

        let (executable, library) = pin_pair();
        assert!(language_pins_are_unchanged(&executable, &library));
        assert_eq!(
            std::fs::remove_file(executable.path())
                .expect_err("the private pin directory refuses replacement")
                .kind(),
            std::io::ErrorKind::PermissionDenied
        );
        set_pin_directory_mode(&executable, 0o700);
        std::fs::remove_file(executable.path()).expect("remove the executable pin");
        std::fs::write(executable.path(), b"replacement").expect("replace the executable pin");
        set_pin_directory_mode(&executable, 0o500);
        assert!(!language_pins_are_unchanged(&executable, &library));
        drop((executable, library));

        let (executable, library) = pin_pair();
        assert!(language_pins_are_unchanged(&executable, &library));
        set_pin_directory_mode(&library, 0o700);
        std::fs::remove_file(library.path()).expect("remove the library pin");
        std::fs::write(library.path(), b"replacement").expect("replace the library pin");
        set_pin_directory_mode(&library, 0o500);
        assert!(!language_pins_are_unchanged(&executable, &library));
    }

    #[test]
    fn language_executable_pin_rejects_replacement_and_removes_it() {
        let nonce = next_fixture_run_nonce().expect("issue a unique pin test identity");
        let source = std::env::temp_dir().join(format!(
            "mado-pilot-language-pin-source-{}-{nonce}",
            std::process::id()
        ));
        let expected: Arc<[u8]> = Arc::from(b"recorded language executable".as_slice());
        std::fs::write(&source, expected.as_ref()).expect("write the pin source");
        let pin = LanguageExecutablePin::new(&source, Arc::clone(&expected))
            .expect("create the private executable pin");
        let pin_path = pin.path().to_path_buf();

        std::fs::set_permissions(&pin.directory, std::fs::Permissions::from_mode(0o700))
            .expect("make the pin directory writable for mutation");
        std::fs::remove_file(&pin_path).expect("remove the original pin");
        std::fs::write(&pin_path, b"changed language executable").expect("replace the pin");
        std::fs::set_permissions(&pin.directory, std::fs::Permissions::from_mode(0o500))
            .expect("restore the private directory mode");
        assert!(!post_use_identity_gate(true, &[pin.is_unchanged()]));

        drop(pin);
        assert!(!pin_path.exists());
        std::fs::remove_file(source).expect("remove the pin source");
    }

    #[test]
    fn setup_events_retry_until_quiet_then_strict_reset_rejects_leftovers() {
        #[derive(Default)]
        struct SetupState {
            pending: VecDeque<u8>,
            reset_calls: usize,
            quiet_calls: usize,
            discarded_before_ack: usize,
            observed_during_quiet: usize,
        }

        let mut state = SetupState::default();
        assert!(discard_setup_events_until_quiet(
            &mut state,
            Duration::from_secs(1),
            Duration::from_millis(1),
            |state, _remaining| {
                state.reset_calls += 1;
                if state.reset_calls == 1 {
                    state.pending.push_back(1);
                }
                true
            },
            |state| {
                state.discarded_before_ack += state.pending.len();
                state.pending.clear();
            },
            |state, _quiet_deadline| {
                state.quiet_calls += 1;
                if state.quiet_calls == 1 {
                    state.observed_during_quiet += 1;
                    false
                } else {
                    true
                }
            },
        ));
        assert_eq!(state.discarded_before_ack, 1);
        assert_eq!(state.observed_during_quiet, 1);
        assert_eq!(state.reset_calls, 2);
        assert_eq!(state.quiet_calls, 2);
        assert!(state.pending.is_empty());

        state.pending.push_back(2);
        let mut strict_command_called = false;
        assert!(!strict_event_reset(state.pending.is_empty(), || {
            strict_command_called = true;
            true
        }));
        assert!(!strict_command_called);
    }

    #[test]
    fn final_identity_gate_rejects_each_changed_artifact() {
        assert!(!post_use_identity_gate(true, &[]));
        assert!(!post_use_identity_gate(true, &[false]));
        assert!(!post_use_identity_gate(true, &[true, false]));
        assert!(!post_use_identity_gate(false, &[true, true]));
        assert!(post_use_identity_gate(true, &[true, true]));
    }

    #[test]
    fn auxiliary_setup_rejects_an_acknowledged_no_op_without_a_second_window() {
        assert!(!auxiliary_window_setup_is_proven(true, &[7_u64]));
        assert!(!auxiliary_window_setup_is_proven(true, &[7_u64, 7]));
        assert!(auxiliary_window_setup_is_proven(true, &[7_u64, 8]));
        assert!(!auxiliary_window_setup_is_proven(false, &[7_u64, 8]));
    }

    #[test]
    fn finish_drains_an_event_enqueued_before_reader_eof() {
        let run_nonce = 91;
        let (sender, receiver) = mpsc::sync_channel(1);
        let reader = thread::spawn(move || {
            sender
                .send(ReaderMessage::Line("fixture-log".to_owned()))
                .expect("the first line is queued");
            sender
                .send(ReaderMessage::Line(format_event_line(
                    run_nonce,
                    EventSummary {
                        kind: EVENT_KEY_DOWN,
                        text_units: 0,
                        correlation: 0,
                    },
                )))
                .expect("the trailing event is queued");
        });
        let lines = Arc::new(Mutex::new(receiver));

        assert!(!finish_reader_output_is_clean(
            Some(reader),
            &lines,
            &AtomicBool::new(false),
            true,
            Instant::now() + Duration::from_secs(1),
        ));
    }

    #[test]
    fn reader_failure_cannot_be_accepted_as_clean_finalization() {
        let (_sender, receiver) = mpsc::sync_channel(1);
        let lines = Arc::new(Mutex::new(receiver));

        assert!(!finish_reader_output_is_clean(
            None,
            &lines,
            &AtomicBool::new(true),
            true,
            Instant::now() + Duration::from_secs(1),
        ));
    }

    #[test]
    fn output_lines_are_bounded_including_the_newline() {
        let (reader, mut writer) = UnixStream::pair().expect("private socket pair opens");
        let (sender, receiver) = mpsc::sync_channel(2);
        let reader_failed = Arc::new(AtomicBool::new(false));
        let task_failed = Arc::clone(&reader_failed);
        let task = thread::spawn(move || read_bounded_lines(reader, &sender, &task_failed));
        writeln!(writer, "{}", "x".repeat(MAX_OUTPUT_LINE_BYTES - 1))
            .expect("the maximum line is written");
        writeln!(writer, "{}", "x".repeat(MAX_OUTPUT_LINE_BYTES))
            .expect("the oversized line is written");
        drop(writer);

        assert!(matches!(
            receiver.recv().expect("the bounded line arrives"),
            ReaderMessage::Line(line) if line.len() == MAX_OUTPUT_LINE_BYTES - 1
        ));
        assert!(matches!(
            receiver.recv().expect("the oversized marker arrives"),
            ReaderMessage::Oversized
        ));
        task.join().expect("the bounded reader exits");
    }
}
