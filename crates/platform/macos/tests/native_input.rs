#![cfg_attr(not(target_os = "macos"), allow(missing_docs))]
#![cfg(target_os = "macos")]
//! Native macOS input checks against a real desktop.
//!
//! # What runs by default and what does not
//!
//! The default suite submits nothing: it exercises read-only native observations,
//! the provider's input surface, and refusals that happen before any event. Native
//! submission remains deliberate and ignored by default. One check exercises the
//! focused system route; the exact fixture qualification exercises the explicit
//! process-directed route without taking foreground or moving the physical cursor.
//! Both are documented in `docs/macos-input-verification.md`.

#[allow(dead_code, unreachable_pub, unused_imports)]
#[path = "../src/fixture_protocol.rs"]
mod fixture_protocol;

use fixture_protocol::{
    EVENT_FLAGS_CHANGED, EVENT_KEY_DOWN, EVENT_KEY_UP, EVENT_POINTER_MOVE, EVENT_POINTER_PRESS,
    EVENT_POINTER_RELEASE, EVENT_POINTER_SCROLL, EventSummary, EventTotals,
    FIXTURE_CONTROL_VERSION, FixtureCommand, FixtureCommandKind, FixtureCommandResult, FixtureMode,
    FixtureReadyFacts, FixtureRenderer, FixtureSelectionError, MAX_READY_LINE_BYTES,
    MAX_RECORDED_EVENTS, fixture_ready_facts, fixture_title, format_command_line,
    frame_is_fixture_content, frame_is_replacement_content, parse_command_result_line,
    parse_event_line_for_run, select_unique_fixture, with_confirmed_fixture_content,
};
use mado_pilot_capture::{
    CaptureProvider, CaptureSession, Frame, FrameRequest, OpenRequest, PixelFormat,
    TargetDescription,
};
use mado_pilot_core::{
    CancellationToken, CapabilitySupport, CoordinateSpace, FrameStamp, IdentityIssuer,
    InputAddressScope, InputDelivery, InputOperationKind, OperationContext, PermissionKind,
    PermissionProbe, PermissionState, Point, Status, SubmissionEvidence, TargetId, TargetKind,
};
use mado_pilot_input::{
    CleanupState, DeliveryPlan, FocusPolicy, InputController, InputEvent, InputFault,
    InputOpenRequest, InputProvider, InputRequest, InputRequirement, InputSequence, Key, Modifier,
    PointerButton, PointerGeometry, SequenceOutcome,
};
use mado_pilot_platform_macos::{MacosCaptureProvider, MacosPermissionProbe};
use std::collections::VecDeque;
use std::ffi::{CStr, c_int, c_void};
use std::io::{Read, Write};
use std::mem::size_of;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How long the interactive check waits for a person to focus the fixture.
const FOCUS_WAIT: Duration = Duration::from_secs(15);
/// How long the fail-closed content gate waits for one authoritative frame.
const CONTENT_WAIT: Duration = Duration::from_secs(5);
/// Minimum continuous-capture interval for the route-wide sustained soak.
const SUSTAINED_CAPTURE_SOAK: Duration = Duration::from_secs(60);
/// How long the fixture is given to publish its ready line.
const READY_WAIT: Duration = Duration::from_secs(10);
/// How long the owned-window oracle allows the successor and terminal loss.
const REPLACEMENT_WAIT: Duration = Duration::from_secs(10);
/// Event capacity plus bounded ready, control, and lifecycle records.
const MAX_FIXTURE_OUTPUT_RECORDS: usize = MAX_RECORDED_EVENTS + 16;
static FIXTURE_SOCKET_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static FIXTURE_RUN_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Statuses a host without Screen Recording or without the capture framework
/// legitimately reports, which every check below tolerates.
fn is_unavailable(status: Status) -> bool {
    matches!(status, Status::Unsupported | Status::CaptureFailed)
}

fn provider() -> MacosCaptureProvider {
    MacosCaptureProvider::new(Arc::new(IdentityIssuer::new()))
}

fn context() -> OperationContext {
    OperationContext::new()
}

fn bounded(duration: Duration) -> OperationContext {
    context()
        .with_timeout(duration)
        .expect("the operation timeout is positive")
}

fn post_event_access_granted() -> bool {
    MacosPermissionProbe::new()
        .probe(PermissionKind::InputControl, &context())
        .is_ok_and(|outcome| outcome.state() == PermissionState::Granted)
}

/// Returns discovered targets, or `None` on a host that cannot discover at all.
fn discovered(provider: &MacosCaptureProvider) -> Option<Vec<TargetDescription>> {
    match provider.discover(&context()) {
        Ok(targets) => Some(targets),
        Err(error) if is_unavailable(error.status()) => None,
        Err(error) => panic!("discovery failed on an authorized host: {error}"),
    }
}

#[test]
fn every_discovered_target_reports_the_input_this_adapter_implements() {
    let provider = provider();
    let Some(targets) = discovered(&provider) else {
        println!("skipped: this host offers no capture capability");
        return;
    };

    for target in &targets {
        let input = target.capability().input();
        for kind in InputOperationKind::ALL {
            assert_eq!(
                input.pair(kind, InputDelivery::WindowMessage).support(),
                CapabilitySupport::Unsupported,
                "a discovered macOS target advertised exact-window {}",
                kind.as_str()
            );
        }
        let pointer = input.pair(InputOperationKind::Pointer, InputDelivery::System);
        assert_eq!(pointer.support(), CapabilitySupport::Supported);
        assert_eq!(pointer.permission(), Some(PermissionKind::InputControl));
        let expects_keyboard = target.capability().kind() == Some(TargetKind::Window);
        assert_eq!(
            input
                .pair(InputOperationKind::Keyboard, InputDelivery::System)
                .support()
                == CapabilitySupport::Supported,
            expects_keyboard,
            "only a window is a focusable target"
        );
    }
}

#[test]
fn a_described_target_reports_its_own_identity_and_a_foreign_one_is_refused() {
    let provider = provider();
    let Some(targets) = discovered(&provider) else {
        println!("skipped: this host offers no capture capability");
        return;
    };
    let Some(first) = targets.first() else {
        println!("skipped: this host presented no shareable target");
        return;
    };

    let descriptor = InputProvider::describe(&provider, first.id(), &context())
        .expect("a target this provider issued is describable");
    assert_eq!(descriptor.target(), first.id());
    assert_eq!(descriptor.capability(), first.capability().input());

    let foreign: TargetId = IdentityIssuer::new()
        .issue_target(mado_pilot_platform_macos::PROVIDER)
        .expect("issued elsewhere");
    let error = InputProvider::describe(&provider, foreign, &context())
        .expect_err("another engine's identity is refused");
    assert_eq!(error.status(), Status::InvalidArgument);
}

#[test]
fn an_open_that_requires_window_message_fails_without_establishing_anything() {
    let provider = provider();
    let Some(targets) = discovered(&provider) else {
        println!("skipped: this host offers no capture capability");
        return;
    };
    let Some(window) = targets
        .iter()
        .find(|target| target.capability().kind() == Some(TargetKind::Window))
    else {
        println!("skipped: this host presented no shareable window");
        return;
    };

    let error = InputProvider::open(
        &provider,
        window.id(),
        &InputOpenRequest::new()
            .with_requirement(InputRequirement::Required)
            .requiring(InputOperationKind::Pointer, InputDelivery::WindowMessage),
        &context(),
    )
    .expect_err("macOS implements no WindowMessage route");

    assert_eq!(error.status(), Status::Unsupported);
}

#[test]
fn a_preserving_request_to_an_unfocused_window_submits_nothing() {
    // The point of this check is that it is safe to run anywhere: a focus policy
    // that will not activate cannot satisfy system delivery, so the refusal
    // happens before any event and the developer's desktop is untouched.
    let provider = provider();
    let Some(targets) = discovered(&provider) else {
        println!("skipped: this host offers no capture capability");
        return;
    };
    let Some(window) = targets
        .iter()
        .find(|target| target.capability().kind() == Some(TargetKind::Window))
    else {
        println!("skipped: this host presented no shareable window");
        return;
    };

    let controller =
        InputProvider::open(&provider, window.id(), &InputOpenRequest::new(), &context())
            .expect("an optional input open succeeds for a window");
    let request = InputRequest::new(
        window.id(),
        InputSequence::new(vec![InputEvent::KeyPress(Key::Escape)]).expect("valid"),
        DeliveryPlan::require(InputDelivery::System),
    );

    let receipt = controller
        .execute(&request, &context())
        .expect("focus-policy refusal is receipt evidence");

    assert_eq!(receipt.outcome(), SequenceOutcome::Unexecuted);
    assert_eq!(receipt.submitted(), 0);
    assert_eq!(receipt.fault(), Some(InputFault::FocusRequired));
    assert_eq!(receipt.attempts().len(), 1);
    assert_eq!(receipt.attempts()[0].route(), InputDelivery::System);
    controller.close(&context()).expect("close");
    assert!(controller.is_closed());
}

#[test]
fn an_unfocused_window_refuses_a_require_focused_sequence_before_any_event() {
    let provider = provider();
    let Some(targets) = discovered(&provider) else {
        println!("skipped: this host offers no capture capability");
        return;
    };
    // A window this test process does not own cannot be frontmost while the test
    // binary is, so the refusal below is the one a caller would see.
    let Some(window) = targets
        .iter()
        .find(|target| target.capability().kind() == Some(TargetKind::Window))
    else {
        println!("skipped: this host presented no shareable window");
        return;
    };

    let controller =
        InputProvider::open(&provider, window.id(), &InputOpenRequest::new(), &context())
            .expect("input opens");
    let request = InputRequest::new(
        window.id(),
        InputSequence::new(vec![InputEvent::KeyPress(Key::Escape)]).expect("valid"),
        DeliveryPlan::require(InputDelivery::System),
    )
    .with_focus(FocusPolicy::RequireFocused);

    let receipt = controller
        .execute(&request, &context())
        .expect("an admitted sequence produces a receipt");

    assert_eq!(receipt.submitted(), 0);
    assert_eq!(receipt.outcome(), SequenceOutcome::Unexecuted);
    let fault = receipt.fault().expect("a reason");
    assert!(
        matches!(
            fault,
            InputFault::FocusRequired | InputFault::NotAuthorized | InputFault::TargetLost
        ),
        "an unfocused, unauthorized, or vanished target are the honest answers, got {fault:?}"
    );
    controller.close(&context()).expect("close");
}

#[test]
fn a_target_that_no_longer_exists_is_reported_lost_rather_than_delivered_to() {
    let provider = provider();
    let Some(_targets) = discovered(&provider) else {
        println!("skipped: this host offers no capture capability");
        return;
    };
    let issuer = Arc::new(IdentityIssuer::new());
    let own = MacosCaptureProvider::new(Arc::clone(&issuer));
    let absent = issuer
        .issue_target(mado_pilot_platform_macos::PROVIDER)
        .expect("issued by this engine for this provider");

    let error = InputProvider::describe(&own, absent, &context())
        .expect_err("an accepted identity that was never discovered is not live");

    assert_eq!(error.status(), Status::TargetLost);
}

const SOL_LOCAL: c_int = 0;
const LOCAL_PEERPID: c_int = 0x002;
const LOCAL_PEERTOKEN: c_int = 0x006;
const SIGTERM: c_int = 15;
const PROC_PIDPATHINFO_MAXSIZE: usize = 4 * 1_024;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AuditToken {
    values: [u32; 8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FixturePeerIdentity {
    effective_user_id: u32,
    process_id: u32,
    executable: PathBuf,
    audit_token: AuditToken,
}

#[derive(Debug, Clone, Copy)]
struct AuthenticatedFixtureProcess {
    process_id: u32,
    audit_token: AuditToken,
}

fn fixture_peer_is_expected(
    peer: &FixturePeerIdentity,
    current_effective_user_id: u32,
    expected_executable: &Path,
) -> bool {
    peer.effective_user_id == current_effective_user_id
        && peer.process_id > 0
        && peer.executable == expected_executable
}

fn fixture_peer_identity(stream: &UnixStream) -> Option<FixturePeerIdentity> {
    let socket = stream.as_raw_fd();
    let mut effective_user_id = 0u32;
    let mut effective_group_id = 0u32;
    // SAFETY: both scalar outputs are writable and `socket` remains open for
    // the call. `getpeereid` reads credentials already bound to this connection.
    if unsafe {
        getpeereid(
            socket,
            &raw mut effective_user_id,
            &raw mut effective_group_id,
        )
    } != 0
    {
        return None;
    }

    let mut process_id = 0i32;
    let mut process_id_size = u32::try_from(size_of::<c_int>()).ok()?;
    // SAFETY: the output points to one writable `pid_t`, its exact byte extent
    // is supplied, and LOCAL_PEERPID reads the connected Unix peer only.
    if unsafe {
        getsockopt(
            socket,
            SOL_LOCAL,
            LOCAL_PEERPID,
            (&raw mut process_id).cast::<c_void>(),
            &raw mut process_id_size,
        )
    } != 0
        || process_id_size as usize != size_of::<c_int>()
        || process_id <= 0
    {
        return None;
    }

    let mut audit_token = AuditToken { values: [0; 8] };
    let mut audit_token_size = u32::try_from(size_of::<AuditToken>()).ok()?;
    // SAFETY: the output is one writable audit token with its exact extent;
    // LOCAL_PEERTOKEN binds it to this connected peer and survives PID reuse.
    if unsafe {
        getsockopt(
            socket,
            SOL_LOCAL,
            LOCAL_PEERTOKEN,
            (&raw mut audit_token).cast::<c_void>(),
            &raw mut audit_token_size,
        )
    } != 0
        || audit_token_size as usize != size_of::<AuditToken>()
    {
        return None;
    }

    let mut executable = [0i8; PROC_PIDPATHINFO_MAXSIZE];
    // SAFETY: the fixed buffer is writable for the declared capacity and the
    // positive authenticated peer PID fits `proc_pidpath`'s `int` argument.
    let executable_len = unsafe {
        proc_pidpath(
            process_id,
            executable.as_mut_ptr().cast::<c_void>(),
            u32::try_from(executable.len()).ok()?,
        )
    };
    if executable_len <= 0 || executable_len as usize >= executable.len() {
        return None;
    }
    // SAFETY: the zero-initialized buffer retains a terminator beyond every
    // accepted result length.
    let executable = unsafe { CStr::from_ptr(executable.as_ptr()) };
    let executable = std::fs::canonicalize(Path::new(std::ffi::OsStr::from_bytes(
        executable.to_bytes(),
    )))
    .ok()?;
    Some(FixturePeerIdentity {
        effective_user_id,
        process_id: u32::try_from(process_id).ok()?,
        executable,
        audit_token,
    })
}

fn authenticate_fixture_peer(
    stream: &UnixStream,
    expected_executable: &Path,
) -> Option<AuthenticatedFixtureProcess> {
    let peer = fixture_peer_identity(stream)?;
    // SAFETY: `geteuid` has no arguments and returns process-local credentials.
    let current_effective_user_id = unsafe { geteuid() };
    fixture_peer_is_expected(&peer, current_effective_user_id, expected_executable).then_some(
        AuthenticatedFixtureProcess {
            process_id: peer.process_id,
            audit_token: peer.audit_token,
        },
    )
}

fn ready_process_id_for_peer(line: &str, authenticated_process_id: u32) -> Option<u32> {
    let reported_process_id = ready_process_id(line)?;
    (reported_process_id == authenticated_process_id).then_some(reported_process_id)
}

/// A fixture launch broker plus the audit-token-bound application it owns.
struct FixtureChild {
    launcher: Child,
    application: Option<AuthenticatedFixtureProcess>,
}

impl FixtureChild {
    fn new(launcher: Child) -> Self {
        Self {
            launcher,
            application: None,
        }
    }
}

impl Drop for FixtureChild {
    fn drop(&mut self) {
        if self.launcher.try_wait().ok().flatten().is_none() {
            if let Some(application) = self.application.as_mut() {
                // SAFETY: the token was obtained from the authenticated Unix
                // peer. libproc signals that exact process lifetime, not a
                // subsequently reused numeric PID.
                let _terminated = unsafe {
                    proc_signal_with_audittoken(&raw mut application.audit_token, SIGTERM)
                };
            }
            let deadline = Instant::now() + Duration::from_millis(500);
            while Instant::now() < deadline {
                if self.launcher.try_wait().ok().flatten().is_some() {
                    return;
                }
                thread::sleep(Duration::from_millis(10));
            }
            let _killed = self.launcher.kill();
        }
        let _reaped = self.launcher.wait();
    }
}

/// A running fixture with an owned command channel and bounded output channel.
struct Fixture {
    child: FixtureChild,
    input: Option<UnixStream>,
    lines: Arc<Mutex<Receiver<String>>>,
    process_id: u32,
    facts: FixtureReadyFacts,
    run_nonce: u64,
    next_nonce: u64,
    pending_events: VecDeque<EventSummary>,
}

impl Fixture {
    /// Starts the ordinary fixture and waits for its ready record.
    fn start() -> Option<Self> {
        Self::start_with_arguments(&[], FixtureMode::Default)
    }

    /// Starts the fixture mode that destroys and replaces its own window.
    fn start_replacing() -> Option<Self> {
        Self::start_with_arguments(&["--replace-window-after-ready"], FixtureMode::Default)
    }

    /// Starts one visible target without taking foreground ownership.
    fn start_inactive(mode: FixtureMode) -> Option<Self> {
        match mode {
            FixtureMode::Default => {
                Self::start_with_arguments(&["--inactive"], FixtureMode::Default)
            }
            FixtureMode::GameLike => {
                Self::start_with_arguments(&["--game-like", "--inactive"], FixtureMode::GameLike)
            }
        }
    }

    fn start_with_arguments(arguments: &[&str], expected_mode: FixtureMode) -> Option<Self> {
        let executable = fixture_executable()?;
        let expected_executable = std::fs::canonicalize(&executable).ok()?;
        let bundle = fixture_bundle(&executable)?;
        let require_signed_bundle =
            std::env::var_os("MADO_PILOT_MACOS_FIXTURE_EXECUTABLE").is_some();
        let socket_directory = fixture_socket_directory()?;
        let socket_path = socket_directory.socket_path();
        let listener = UnixListener::bind(&socket_path).ok()?;
        listener.set_nonblocking(true).ok()?;
        let sequence = FIXTURE_RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos() as u64);
        let run_nonce = (time ^ (u64::from(std::process::id()) << 32) ^ sequence).max(1);
        let mut command = Command::new("/usr/bin/open");
        if arguments.contains(&"--inactive") {
            command.arg("-g");
        }
        command
            .arg("-n")
            .arg("-W")
            .arg(&bundle)
            .arg("--args")
            .arg("--control-socket")
            .arg(&socket_path)
            .arg("--run-nonce")
            .arg(run_nonce.to_string())
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        let mut child = FixtureChild::new(command.spawn().ok()?);
        let deadline = Instant::now() + READY_WAIT;
        let (stream, authenticated_process) = loop {
            match listener.accept() {
                Ok((stream, _address)) => {
                    if let Some(process) = authenticate_fixture_peer(&stream, &expected_executable)
                    {
                        break (stream, process);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => return None,
            }
            if child.launcher.try_wait().ok().flatten().is_some() || Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(25));
        };
        stream.set_nonblocking(false).ok()?;
        drop(listener);
        drop(socket_directory);
        Self::from_child(
            child,
            stream,
            authenticated_process,
            require_signed_bundle,
            expected_mode,
            run_nonce,
            READY_WAIT,
        )
    }

    fn from_child(
        mut child: FixtureChild,
        input: UnixStream,
        authenticated_process: AuthenticatedFixtureProcess,
        require_signed_bundle: bool,
        expected_mode: FixtureMode,
        run_nonce: u64,
        ready_wait: Duration,
    ) -> Option<Self> {
        child.application = Some(authenticated_process);
        let lines = spawn_reader(input.try_clone().ok()?);
        let line = wait_for(&lines, ready_wait, |line| line.starts_with("fixture-ready"))?;
        println!("{line}");
        let process_id = ready_process_id_for_peer(&line, authenticated_process.process_id)?;
        let facts = fixture_ready_facts(&line, process_id)?;
        assert_eq!(
            facts.run_nonce(),
            run_nonce,
            "the fixture ready record did not echo the harness-issued run identity"
        );
        let expected_renderer = match expected_mode {
            FixtureMode::Default => FixtureRenderer::AppKitBackground,
            FixtureMode::GameLike => FixtureRenderer::OpenGl,
        };
        assert_eq!(
            (facts.mode(), facts.renderer()),
            (expected_mode, expected_renderer),
            "the fixture initialized a renderer other than the requested one: {line}"
        );
        if require_signed_bundle {
            assert!(
                facts.execution_context_is_approved(),
                "a configured fixture must truthfully report the stable signed bundle \
                 context before any input path opens: {line}"
            );
        }
        Some(Self {
            child,
            input: Some(input),
            lines: Arc::new(Mutex::new(lines)),
            process_id,
            run_nonce,
            facts,
            next_nonce: 1,
            pending_events: VecDeque::new(),
        })
    }

    fn replacement_result(&mut self, wait: Duration) -> Option<(u32, u64, u64)> {
        let line = self.wait_for_line(wait, |line| line.starts_with("fixture-replaced "))?;
        println!("{line}");
        let (run_nonce, status, old_window, new_window) = parse_replacement_line(&line)?;
        (run_nonce == self.run_nonce).then_some((status, old_window, new_window))
    }

    fn command(
        &mut self,
        kind: FixtureCommandKind,
        wait: Duration,
    ) -> Option<FixtureCommandResult> {
        let nonce = self.next_nonce;
        self.next_nonce = self.next_nonce.checked_add(1)?;
        self.command_with_nonce(
            FixtureCommand {
                run_nonce: self.run_nonce,
                nonce,
                kind,
            },
            wait,
        )
    }

    fn command_with_nonce(
        &mut self,
        command: FixtureCommand,
        wait: Duration,
    ) -> Option<FixtureCommandResult> {
        let input = self.input.as_mut()?;
        writeln!(input, "{}", format_command_line(command)).ok()?;
        input.flush().ok()?;
        let line = self.wait_for_line(wait, |line| {
            parse_command_result_line(line).is_some_and(|result| {
                result.run_nonce == command.run_nonce && result.nonce == command.nonce
            })
        })?;
        let result = parse_command_result_line(&line)?;
        assert_eq!(result.run_nonce, self.run_nonce);
        println!("{line}");
        Some(result)
    }
    fn command_is_rejected(&mut self, command: FixtureCommand, wait: Duration) -> bool {
        let Some(input) = self.input.as_mut() else {
            return false;
        };
        if writeln!(input, "{}", format_command_line(command)).is_err() || input.flush().is_err() {
            return false;
        }
        self.wait_for_line(wait, |line| line == "fixture-command-rejected status=1")
            .is_some()
    }

    fn wait_for_line(&mut self, wait: Duration, accept: impl Fn(&str) -> bool) -> Option<String> {
        let deadline = Instant::now() + wait;
        let lines = self
            .lines
            .lock()
            .expect("the fixture output receiver is not poisoned");
        while Instant::now() < deadline {
            match lines.recv_timeout(Duration::from_millis(100)) {
                Ok(line) if accept(&line) => return Some(line),
                Ok(line) => {
                    if let Some(summary) = parse_event_line_for_run(&line, self.run_nonce) {
                        self.pending_events.push_back(summary);
                    } else if line.starts_with("fixture-command-result ")
                        || line.starts_with("fixture-command-rejected ")
                    {
                        panic!("the fixture returned an unexpected control record: {line}");
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return None,
            }
        }
        None
    }

    fn summaries(&mut self, wait: Duration) -> Vec<u32> {
        let mut kinds = self
            .pending_events
            .drain(..)
            .map(|summary| summary.kind)
            .collect::<Vec<_>>();
        let deadline = Instant::now() + wait;
        let lines = self
            .lines
            .lock()
            .expect("the fixture output receiver is not poisoned");
        while Instant::now() < deadline && kinds.len() < MAX_RECORDED_EVENTS {
            match lines.recv_timeout(Duration::from_millis(100)) {
                Ok(line) => {
                    if let Some(summary) = parse_event_line_for_run(&line, self.run_nonce) {
                        kinds.push(summary.kind);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        kinds
    }

    fn event_summaries(&mut self, count: usize, wait: Duration) -> Vec<EventSummary> {
        let mut events = self.pending_events.drain(..).collect::<Vec<_>>();
        let deadline = Instant::now() + wait;
        let lines = self
            .lines
            .lock()
            .expect("the fixture output receiver is not poisoned");
        while Instant::now() < deadline && events.len() < count {
            match lines.recv_timeout(Duration::from_millis(25)) {
                Ok(line) => {
                    if let Some(summary) = parse_event_line_for_run(&line, self.run_nonce) {
                        events.push(summary);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        events
    }
    /// Cancels only after the exact process-wide fixture event is observable.
    ///
    /// The helper exclusively owns the receiver until joined. Callers must not
    /// issue fixture commands or read event summaries while it is running.
    fn cancel_after_event(
        &mut self,
        expected: EventSummary,
        cancellation: CancellationToken,
        wait: Duration,
    ) -> thread::JoinHandle<Option<EventSummary>> {
        assert!(
            self.pending_events.is_empty(),
            "stale fixture events precede the cancellation row"
        );
        {
            let lines = self
                .lines
                .lock()
                .expect("the fixture output receiver is not poisoned");
            assert!(
                matches!(lines.try_recv(), Err(mpsc::TryRecvError::Empty)),
                "queued fixture output precedes the cancellation row"
            );
        }
        let lines = Arc::clone(&self.lines);
        let run_nonce = self.run_nonce;
        thread::Builder::new()
            .name("mado-pilot-native-input-cancellation-trigger".to_owned())
            .spawn(move || {
                let deadline = Instant::now() + wait;
                let lines = lines
                    .lock()
                    .expect("the fixture output receiver is not poisoned");
                while Instant::now() < deadline {
                    match lines.recv_timeout(Duration::from_millis(25)) {
                        Ok(line) => {
                            let Some(summary) = parse_event_line_for_run(&line, run_nonce) else {
                                continue;
                            };
                            if summary == expected {
                                cancellation.cancel();
                            }
                            return Some(summary);
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
                None
            })
            .expect("the cancellation observation helper starts")
    }

    fn exact_event_summaries(&mut self, count: usize, wait: Duration) -> Vec<EventSummary> {
        let events = self.event_summaries(count, wait);
        assert_eq!(
            events.len(),
            count,
            "the {:?}/{:?} fixture observed an incomplete bounded event set: {events:?}",
            self.facts.mode(),
            self.facts.renderer(),
        );
        let extras = self.event_summaries(1, Duration::from_millis(150));
        assert!(
            extras.is_empty(),
            "the fixture observed events outside the exact submitted row: {extras:?}"
        );
        events
    }
    fn begin_event_row(&mut self, wait: Duration) {
        assert!(
            self.pending_events.is_empty(),
            "a prior row left unconsumed fixture events: {:?}",
            self.pending_events
        );
        let reset = self
            .command(FixtureCommandKind::ResetEvents, wait)
            .expect("the fixture resets its process-wide event summary");
        assert_eq!(reset.status, 0, "the fixture event reset succeeds");
        assert_eq!(reset.events, EventTotals::default());
        assert!(
            self.pending_events.is_empty(),
            "the fixture reset exposed events left by a prior row: {:?}",
            self.pending_events
        );
    }

    fn event_totals(&mut self, wait: Duration) -> EventTotals {
        let result = self
            .command(FixtureCommandKind::ReadEvents, wait)
            .expect("the fixture reads its process-wide event summary");
        assert_eq!(result.status, 0, "the fixture event summary read succeeds");
        result.events
    }

    fn expect_event_kinds(&mut self, expected: &[u32], wait: Duration) {
        let events = self.exact_event_summaries(expected.len(), wait);
        let kinds = events.iter().map(|event| event.kind).collect::<Vec<_>>();
        assert_eq!(kinds, expected);
        assert_eq!(self.event_totals(wait), event_totals(&events));
        assert!(self.pending_events.is_empty());
    }

    fn expect_text_chunks(&mut self, expected_units: &[u32], wait: Duration) {
        let expected = expected_units
            .iter()
            .flat_map(|units| {
                [
                    EventSummary {
                        kind: EVENT_KEY_DOWN,
                        text_units: *units,
                    },
                    EventSummary {
                        kind: EVENT_KEY_UP,
                        text_units: *units,
                    },
                ]
            })
            .collect::<Vec<_>>();
        let observed = self.exact_event_summaries(expected.len(), wait);
        assert_eq!(
            observed, expected,
            "text observations expose only exact UTF-16 chunk lengths"
        );
        assert_eq!(self.event_totals(wait), event_totals(&observed));
        assert!(self.pending_events.is_empty());
    }
}

fn event_totals(events: &[EventSummary]) -> EventTotals {
    let mut totals = EventTotals::default();
    for event in events {
        match event.kind {
            EVENT_POINTER_MOVE => totals.pointer_moves += 1,
            EVENT_POINTER_PRESS => totals.pointer_presses += 1,
            EVENT_POINTER_RELEASE => totals.pointer_releases += 1,
            EVENT_POINTER_SCROLL => totals.pointer_scrolls += 1,
            EVENT_KEY_DOWN => totals.key_downs += 1,
            EVENT_KEY_UP => totals.key_ups += 1,
            EVENT_FLAGS_CHANGED => totals.flags_changed += 1,
            kind => panic!("unexpected fixture event kind {kind}"),
        }
        totals.text_units += u64::from(event.text_units);
    }
    totals
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(input) = self.input.as_mut() {
            let command = FixtureCommand {
                run_nonce: self.run_nonce,
                nonce: self.next_nonce,
                kind: FixtureCommandKind::Stop,
            };
            let _written = writeln!(input, "{}", format_command_line(command));
            let _flushed = input.flush();
        }
        let _ = self.child.launcher.try_wait();
    }
}

fn spawn_reader(mut input: impl Read + Send + 'static) -> Receiver<String> {
    let (sender, receiver) = mpsc::sync_channel(MAX_FIXTURE_OUTPUT_RECORDS);
    thread::spawn(move || {
        let mut line = Vec::with_capacity(MAX_READY_LINE_BYTES);
        let mut byte = [0u8; 1];
        let mut overflow = false;
        loop {
            match input.read(&mut byte) {
                Ok(0) => {
                    if !overflow
                        && !line.is_empty()
                        && let Ok(line) = std::str::from_utf8(&line)
                    {
                        let _sent = sender.send(line.to_owned());
                    }
                    break;
                }
                Ok(_) if byte[0] == b'\n' => {
                    if !overflow
                        && let Ok(line) = std::str::from_utf8(&line)
                        && sender.send(line.to_owned()).is_err()
                    {
                        break;
                    }
                    line.clear();
                    overflow = false;
                }
                Ok(_) if !overflow && line.len() < MAX_READY_LINE_BYTES => {
                    line.push(byte[0]);
                }
                Ok(_) => {
                    line.clear();
                    overflow = true;
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
    });
    receiver
}

#[test]
fn fixture_output_reader_discards_an_overlong_record_and_recovers_at_newline() {
    let (mut output, input) = UnixStream::pair().expect("the private test channel opens");
    let lines = spawn_reader(input);
    output
        .write_all(&vec![b'x'; MAX_READY_LINE_BYTES + 1])
        .expect("the oversized record is written");
    output
        .write_all(b"\nfixture-event kind=1 units=0\n")
        .expect("the next bounded record is written");
    drop(output);

    assert_eq!(
        lines.recv_timeout(Duration::from_secs(1)),
        Ok(String::from("fixture-event kind=1 units=0"))
    );
    assert!(matches!(
        lines.recv_timeout(Duration::from_secs(1)),
        Err(RecvTimeoutError::Disconnected)
    ));
}

fn ready_process_id(line: &str) -> Option<u32> {
    let (_prefix, remainder) = line.split_once(" pid=")?;
    let (process_id, _suffix) = remainder.split_once(' ')?;
    let process_id = process_id.parse().ok()?;
    line.starts_with(&format!(
        "fixture-ready title={} ",
        fixture_title(process_id)
    ))
    .then_some(process_id)
}

#[test]
fn fixture_peer_identity_requires_the_expected_user_and_canonical_executable() {
    let expected_executable = Path::new("/private/tmp/approved-fixture");
    let peer = FixturePeerIdentity {
        effective_user_id: 501,
        process_id: 42,
        executable: expected_executable.to_path_buf(),
        audit_token: AuditToken { values: [7; 8] },
    };
    assert!(fixture_peer_is_expected(&peer, 501, expected_executable));

    let wrong_path = FixturePeerIdentity {
        executable: PathBuf::from("/private/tmp/lookalike-fixture"),
        ..peer.clone()
    };
    assert!(!fixture_peer_is_expected(
        &wrong_path,
        501,
        expected_executable
    ));
    assert!(!fixture_peer_is_expected(&peer, 502, expected_executable));
    assert!(!fixture_peer_is_expected(
        &FixturePeerIdentity {
            process_id: 0,
            ..peer
        },
        501,
        expected_executable
    ));
}

#[test]
fn ready_record_pid_must_match_the_authenticated_peer() {
    let line = format!("fixture-ready title={} pid=42 remainder", fixture_title(42));
    assert_eq!(ready_process_id_for_peer(&line, 42), Some(42));
    assert_eq!(ready_process_id_for_peer(&line, 43), None);
}

#[test]
fn fixture_control_socket_directory_is_unique_and_private() {
    let first = fixture_socket_directory().expect("a private fixture directory is created");
    let second = fixture_socket_directory().expect("a second private fixture directory is created");
    assert_ne!(first.path, second.path);
    for directory in [&first, &second] {
        let mode = std::fs::metadata(&directory.path)
            .expect("the private directory exists")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
        assert_eq!(
            directory.socket_path().parent(),
            Some(directory.path.as_path())
        );
    }
}

#[test]
fn invalid_execution_context_output_reaps_the_owned_child() {
    let (input, mut output) = UnixStream::pair().expect("the private test channel opens");
    let child = FixtureChild::new(
        Command::new("/bin/sh")
            .arg("-c")
            .arg("while :; do :; done")
            .stderr(Stdio::inherit())
            .spawn()
            .expect("the non-interactive child starts"),
    );
    let process_id = child.launcher.id();
    writeln!(
        output,
        "fixture-ready title={} pid={process_id} window=17 run=77 control-version={} \
         mode=default renderer=appkit-background launch=bundled signature=ad-hoc \
         signing-identifier=wrong.identifier bundle=dev.mado-pilot.macos-input-fixture \
         capacity={MAX_RECORDED_EVENTS}",
        fixture_title(process_id),
        FIXTURE_CONTROL_VERSION,
    )
    .expect("the malformed ready record is published");

    let rejected = panic::catch_unwind(AssertUnwindSafe(|| {
        Fixture::from_child(
            child,
            input,
            AuthenticatedFixtureProcess {
                process_id,
                audit_token: AuditToken { values: [0; 8] },
            },
            true,
            FixtureMode::Default,
            77,
            Duration::from_secs(2),
        )
    }));

    assert!(rejected.is_err(), "invalid context must fail closed");
    let still_exists = Command::new("/bin/kill")
        .arg("-0")
        .arg(process_id.to_string())
        .output()
        .expect("the process-liveness probe runs")
        .status
        .success();
    assert!(!still_exists, "the rejected fixture child must be reaped");
}

fn wait_for(
    lines: &Receiver<String>,
    wait: Duration,
    accept: impl Fn(&str) -> bool,
) -> Option<String> {
    let deadline = Instant::now() + wait;
    while Instant::now() < deadline {
        match lines.recv_timeout(Duration::from_millis(100)) {
            Ok(line) if accept(&line) => return Some(line),
            Ok(_other) => {}
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return None,
        }
    }
    None
}

fn parse_replacement_line(line: &str) -> Option<(u64, u32, u64, u64)> {
    if line.len() > 256 {
        return None;
    }
    let mut fields = line.strip_prefix("fixture-replaced ")?.split_whitespace();
    let run_nonce = fields.next()?.strip_prefix("run=")?.parse().ok()?;
    let status = fields.next()?.strip_prefix("status=")?.parse().ok()?;
    let old_window = fields.next()?.strip_prefix("old-window=")?.parse().ok()?;
    let new_window = fields.next()?.strip_prefix("new-window=")?.parse().ok()?;
    (run_nonce != 0 && fields.next().is_none())
        .then_some((run_nonce, status, old_window, new_window))
}

#[test]
fn replacement_record_is_bounded_and_structurally_exact() {
    let valid = "fixture-replaced run=9 status=0 old-window=17 new-window=18";
    assert_eq!(parse_replacement_line(valid), Some((9, 0, 17, 18)));
    assert_eq!(
        parse_replacement_line("fixture-replaced status=0 run=9 old-window=17 new-window=18"),
        None
    );
    assert_eq!(
        parse_replacement_line(
            "fixture-replaced run=9 status=0 old-window=17 new-window=18 extra=1"
        ),
        None
    );
    assert_eq!(
        parse_replacement_line(&format!(
            "fixture-replaced run=9 status=0 old-window=17 new-window=18 {}",
            "x".repeat(256)
        )),
        None
    );
}

/// Locates the fixture beside the test binary that cargo just built.
fn fixture_executable() -> Option<PathBuf> {
    if let Some(configured) = std::env::var_os("MADO_PILOT_MACOS_FIXTURE_EXECUTABLE") {
        let executable = PathBuf::from(configured);
        return executable.is_file().then_some(executable);
    }
    let mut directory = std::env::current_exe().ok()?;
    directory.pop();
    if directory.ends_with("deps") {
        directory.pop();
    }
    let executable = directory.join("mado-pilot-macos-input-fixture");
    executable.is_file().then_some(executable)
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

struct FixtureSocketDirectory {
    path: PathBuf,
}

impl FixtureSocketDirectory {
    fn socket_path(&self) -> PathBuf {
        self.path.join("control.sock")
    }
}

impl Drop for FixtureSocketDirectory {
    fn drop(&mut self) {
        let _socket_removed = std::fs::remove_file(self.socket_path());
        let _directory_removed = std::fs::remove_dir(&self.path);
    }
}

fn fixture_socket_directory() -> Option<FixtureSocketDirectory> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    for _attempt in 0..32 {
        let sequence = FIXTURE_SOCKET_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = PathBuf::from(format!(
            "/tmp/mado-pilot-fixture-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700);
        match builder.create(&path) {
            Ok(()) => {
                let permissions = std::fs::Permissions::from_mode(0o700);
                if std::fs::set_permissions(&path, permissions).is_err() {
                    let _removed = std::fs::remove_dir(&path);
                    return None;
                }
                return Some(FixtureSocketDirectory { path });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return None,
        }
    }
    None
}
fn discover_unique_fixture(
    provider: &MacosCaptureProvider,
    process_id: u32,
    wait: Duration,
) -> Result<TargetDescription, FixtureSelectionError> {
    let started = Instant::now();
    loop {
        let targets = discovered(provider).ok_or(FixtureSelectionError::NotFound)?;
        match select_unique_fixture(&targets, process_id) {
            Ok(target) => return Ok(target.clone()),
            Err(_) if started.elapsed() < wait => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => return Err(error),
        }
    }
}

#[test]
fn the_fixture_starts_publishes_its_title_and_is_selected_exactly_once() {
    if std::env::var_os("MADO_PILOT_MACOS_FIXTURE").is_none() {
        println!(
            "skipped: starting the fixture opens a window and takes focus. Set \
             MADO_PILOT_MACOS_FIXTURE=1 to run it."
        );
        return;
    }

    let mut fixture = match Fixture::start() {
        Some(fixture) => fixture,
        None if std::env::var_os("MADO_PILOT_MACOS_FIXTURE_EXECUTABLE").is_none() => {
            println!("skipped: no bundled fixture executable was configured");
            return;
        }
        None => panic!("the configured fixture bundle could not be started"),
    };
    let provider = provider();
    let chosen = discover_unique_fixture(&provider, fixture.process_id, CONTENT_WAIT)
        .expect("exactly one approved fixture becomes discoverable");

    assert_eq!(chosen.name(), fixture_title(fixture.process_id));
    assert_eq!(chosen.capability().kind(), Some(TargetKind::Window));
    let yielded = fixture
        .command(FixtureCommandKind::YieldForeground, CONTENT_WAIT)
        .expect("foreground ownership is returned after discovery");
    assert_eq!(yielded.status, 0);
    assert_eq!(yielded.before_window, yielded.after_window);
}

/// Exercises the private command channel independently of production input.
#[test]
#[ignore = "opens and controls a real fixture window on an interactive desktop"]
fn owned_fixture_control_is_versioned_idempotent_and_identity_bound() {
    assert!(
        std::env::var_os("MADO_PILOT_MACOS_FIXTURE_EXECUTABLE").is_some(),
        "control verification requires the configured signed fixture bundle"
    );
    let mut fixture = Fixture::start().expect("the owned fixture starts");
    let other_run_nonce = fixture.run_nonce.wrapping_add(1).max(1);
    assert_ne!(other_run_nonce, fixture.run_nonce);
    assert!(
        fixture.command_is_rejected(
            FixtureCommand {
                run_nonce: other_run_nonce,
                nonce: 1,
                kind: FixtureCommandKind::Transition,
            },
            CONTENT_WAIT,
        ),
        "a command carrying another run's identity is rejected before native dispatch"
    );
    fixture.begin_event_row(CONTENT_WAIT);
    assert_eq!(
        fixture.event_totals(CONTENT_WAIT),
        EventTotals::default(),
        "a reset/read boundary starts with no inherited event counts"
    );

    let transition = fixture
        .command(FixtureCommandKind::Transition, CONTENT_WAIT)
        .expect("the transition result is bounded");
    assert_eq!(transition.status, 0);
    assert_ne!(transition.before_window, 0);
    assert_eq!(transition.before_window, transition.after_window);

    let replay = fixture
        .command_with_nonce(
            FixtureCommand {
                run_nonce: fixture.run_nonce,
                nonce: transition.nonce,
                kind: FixtureCommandKind::Transition,
            },
            CONTENT_WAIT,
        )
        .expect("replaying the latest nonce returns its cached result");
    assert_eq!(
        replay, transition,
        "a duplicate command must not execute twice"
    );
    let yielded = fixture
        .command(FixtureCommandKind::YieldForeground, CONTENT_WAIT)
        .expect("foreground ownership is returned without changing window identity");
    assert_eq!(yielded.status, 0);
    assert_eq!(yielded.before_window, transition.after_window);
    assert_eq!(yielded.after_window, transition.after_window);

    for kind in [
        FixtureCommandKind::Move,
        FixtureCommandKind::Resize,
        FixtureCommandKind::Minimize,
        FixtureCommandKind::Restore,
        FixtureCommandKind::OpenAuxiliary,
        FixtureCommandKind::CloseAuxiliary,
    ] {
        let result = fixture
            .command(kind, CONTENT_WAIT)
            .expect("the window-state transition completes");
        assert_eq!(result.status, 0);
        assert_eq!(result.before_window, transition.after_window);
        assert_eq!(result.after_window, transition.after_window);
    }

    let topology = fixture
        .command(FixtureCommandKind::MoveToNextDisplay, CONTENT_WAIT)
        .expect("the bounded topology command returns a result");
    assert!(
        matches!(topology.status, 0 | 2),
        "movement succeeds or reports that fewer than two displays are available: {topology:?}"
    );
    assert_eq!(topology.before_window, transition.after_window);
    assert_eq!(topology.after_window, transition.after_window);
    let topology_replay = fixture
        .command_with_nonce(
            FixtureCommand {
                run_nonce: fixture.run_nonce,
                nonce: topology.nonce,
                kind: FixtureCommandKind::MoveToNextDisplay,
            },
            CONTENT_WAIT,
        )
        .expect("replaying topology movement returns the cached result");
    assert_eq!(
        topology_replay, topology,
        "a duplicate topology command must not move to another display"
    );

    let replacement = fixture
        .command(FixtureCommandKind::Replace, CONTENT_WAIT)
        .expect("the replacement completes");
    assert_eq!(replacement.status, 0);
    assert_eq!(replacement.before_window, transition.after_window);
    assert_ne!(replacement.after_window, 0);
    assert_ne!(replacement.before_window, replacement.after_window);

    let stale = fixture
        .command_with_nonce(
            FixtureCommand {
                run_nonce: fixture.run_nonce,
                nonce: transition.nonce,
                kind: FixtureCommandKind::Restore,
            },
            CONTENT_WAIT,
        )
        .expect("an old nonce receives an explicit refusal");
    assert_eq!(stale.status, 1);
    assert_eq!(stale.before_window, replacement.after_window);
    assert_eq!(stale.after_window, replacement.after_window);

    let closed = fixture
        .command(FixtureCommandKind::Close, CONTENT_WAIT)
        .expect("the main window closes without terminating the fixture");
    assert_eq!(closed.status, 0);
    assert_eq!(closed.before_window, replacement.after_window);
    assert_eq!(closed.after_window, 0);

    let stopped = fixture
        .command(FixtureCommandKind::Stop, CONTENT_WAIT)
        .expect("stop is acknowledged before termination");
    assert_eq!(stopped.status, 0);
    fixture.input = None;
    let deadline = Instant::now() + CONTENT_WAIT;
    while Instant::now() < deadline {
        if fixture
            .child
            .launcher
            .try_wait()
            .expect("the owned child remains waitable")
            .is_some()
        {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("the acknowledged stop did not terminate the owned fixture");
}

/// Proves a retained `SCContentFilter` never starts publishing a same-process,

/// same-title successor after its exact owned window is destroyed. If
/// ScreenCaptureKit reports an explicit terminal outcome, it must be target loss;
/// a quiescent stream is not relabeled from frame-request timeouts.
#[test]
#[ignore = "opens and replaces a real fixture window on an interactive desktop"]
fn owned_window_replacement_never_retargets_the_retained_filter() {
    assert!(
        std::env::var_os("MADO_PILOT_MACOS_FIXTURE_EXECUTABLE").is_some(),
        "replacement verification requires the explicitly configured, structurally verified \
         signed fixture bundle from docs/macos-input-verification.md"
    );
    let mut fixture =
        Fixture::start_replacing().expect("the replacement fixture starts on this desktop");
    let provider = provider();
    let original = discover_unique_fixture(&provider, fixture.process_id, CONTENT_WAIT)
        .expect("the original fixture becomes discoverable exactly once");

    let capture = CaptureProvider::open(
        &provider,
        original.id(),
        &OpenRequest::new().require_format(PixelFormat::Bgra8),
        &bounded(CONTENT_WAIT),
    )
    .expect("the original fixture opens before its scheduled replacement");
    let first = capture
        .frame(&FrameRequest::latest(), &bounded(CONTENT_WAIT))
        .expect("the original fixture publishes before replacement");
    let original_mapping = first
        .map(PixelFormat::Bgra8, &bounded(CONTENT_WAIT))
        .expect("the original frame maps");
    let original_descriptor = original_mapping.descriptor();
    assert!(frame_is_fixture_content(
        original_mapping.bytes(),
        original_descriptor.stride(),
        original_descriptor.extent(),
    ));

    let (replacement_status, old_window, new_window) = fixture
        .replacement_result(REPLACEMENT_WAIT)
        .expect("the fixture reports its bounded replacement result");
    assert_eq!(replacement_status, 0, "native replacement failed");
    assert_ne!(old_window, 0);
    assert_ne!(new_window, 0);

    let mut stamp = first.stamp();
    let observation_deadline = Instant::now() + REPLACEMENT_WAIT;
    let mut terminal = None;
    while Instant::now() < observation_deadline {
        match capture.frame(
            &FrameRequest::newer_than(stamp),
            &bounded(Duration::from_millis(500)),
        ) {
            Ok(frame) => {
                stamp = frame.stamp();
                let mapping = frame
                    .map(PixelFormat::Bgra8, &bounded(CONTENT_WAIT))
                    .expect("an admitted old-window frame maps");
                let descriptor = mapping.descriptor();
                assert!(
                    !frame_is_replacement_content(
                        mapping.bytes(),
                        descriptor.stride(),
                        descriptor.extent(),
                    ),
                    "the retained filter published the replacement window"
                );
            }
            Err(error) if error.status() == Status::DeadlineExceeded => {}
            Err(error) => {
                terminal = Some(error.status());
                break;
            }
        }
    }
    let original_close = capture.close(&bounded(CONTENT_WAIT));
    assert!(
        terminal.is_none() || terminal == Some(Status::TargetLost),
        "window destruction produced an unexpected terminal status: {terminal:?}"
    );
    match terminal {
        Some(status) => println!("retained-filter terminal={status}"),
        None => println!(
            "retained-filter quiescent for {} second(s); no terminal outcome inferred",
            REPLACEMENT_WAIT.as_secs()
        ),
    }
    original_close.expect("the observed original session closes");

    let replacement = discover_unique_fixture(&provider, fixture.process_id, CONTENT_WAIT)
        .expect("the same-process successor becomes discoverable exactly once");
    let replacement_capture = CaptureProvider::open(
        &provider,
        replacement.id(),
        &OpenRequest::new().require_format(PixelFormat::Bgra8),
        &bounded(CONTENT_WAIT),
    )
    .expect("the successor opens independently");
    let replacement_frame = replacement_capture
        .frame(&FrameRequest::latest(), &bounded(CONTENT_WAIT))
        .expect("the successor publishes its own frame");
    let replacement_mapping = replacement_frame
        .map(PixelFormat::Bgra8, &bounded(CONTENT_WAIT))
        .expect("the successor frame maps");
    let replacement_descriptor = replacement_mapping.descriptor();
    assert!(frame_is_replacement_content(
        replacement_mapping.bytes(),
        replacement_descriptor.stride(),
        replacement_descriptor.extent(),
    ));
    assert!(
        frame_is_fixture_content(
            original_mapping.bytes(),
            original_descriptor.stride(),
            original_descriptor.extent(),
        ),
        "the retained original mapping changed after replacement"
    );
    println!("replacement-content distinct; retained original mapping unchanged");
    replacement_capture
        .close(&bounded(CONTENT_WAIT))
        .expect("the successor session closes");
}

/// Delivers real system input to the exact focused fixture while capture remains
/// open.
///
/// Ignored by default. It presses Enter and types a fixed string into the
/// selected fixture, so it runs only on an interactive desktop and only after
/// the person focuses that window.
#[test]
#[ignore = "delivers real system input; run it deliberately on an interactive desktop"]
fn interactive_system_delivery_targets_only_the_exact_fixture() {
    assert!(
        std::env::var_os("MADO_PILOT_MACOS_FIXTURE_EXECUTABLE").is_some(),
        "real-input verification requires the explicitly configured, structurally verified \
         signed fixture bundle from docs/macos-input-verification.md"
    );
    assert!(
        post_event_access_granted(),
        "this check needs post-event access granted to the test process; macOS exposes no \
         delivery result after the void post"
    );
    let mut fixture = Fixture::start().expect("the fixture starts on an interactive desktop");
    let provider = provider();
    let chosen = discover_unique_fixture(&provider, fixture.process_id, CONTENT_WAIT)
        .expect("selection is fail-closed: zero or several matches stop here");

    // Capture and map the exact selected target before obtaining anything that
    // can post input, then keep capture open through delivery. This is
    // load-bearing: ScreenCaptureKit adds an auxiliary same-owner window while
    // streaming, and focus authority must still identify the selected fixture.
    let capture = CaptureProvider::open(
        &provider,
        chosen.id(),
        &OpenRequest::new().require_format(PixelFormat::Bgra8),
        &context(),
    )
    .expect("the selected fixture opens for capture");
    let frame_context = context()
        .with_timeout(CONTENT_WAIT)
        .expect("the content wait is positive");
    let frame = capture
        .frame(&FrameRequest::latest(), &frame_context)
        .expect("the selected fixture publishes a frame before input");
    let mapping = frame
        .map(PixelFormat::Bgra8, &frame_context)
        .expect("the selected fixture frame maps before input");
    let mapped = mapping.descriptor();

    let controller =
        with_confirmed_fixture_content(mapping.bytes(), mapped.stride(), mapped.extent(), || {
            InputProvider::open(
                &provider,
                chosen.id(),
                &InputOpenRequest::new()
                    .with_requirement(InputRequirement::Required)
                    .requiring(InputOperationKind::Keyboard, InputDelivery::System),
                &context(),
            )
        })
        .expect("the selected target must match the fixture's deterministic pixels")
        .expect("input opens for the confirmed fixture");

    println!(
        "Click the window titled `{}` within {} seconds.",
        fixture_title(fixture.process_id),
        FOCUS_WAIT.as_secs()
    );
    // `RequireFocused` never activates anything. Until a person focuses the
    // exact fixture, every attempt refuses and delivers nothing.
    let probe = InputRequest::new(
        chosen.id(),
        InputSequence::new(vec![InputEvent::KeyPress(Key::Escape)]).expect("valid"),
        DeliveryPlan::require(InputDelivery::System),
    )
    .with_focus(FocusPolicy::RequireFocused);
    let deadline = Instant::now() + FOCUS_WAIT;
    let mut focused = false;
    while Instant::now() < deadline {
        let receipt = controller.execute(&probe, &context()).expect("a receipt");
        if receipt.outcome() == SequenceOutcome::Complete {
            focused = true;
            break;
        }
        assert_eq!(
            receipt.submitted(),
            0,
            "an unfocused target must receive nothing"
        );
        thread::sleep(Duration::from_millis(200));
    }
    assert!(
        focused,
        "the fixture was not focused in time, so this check stopped before sending \
         anything else"
    );

    // Showing and focusing the fixture can itself enqueue an ordinary mouse-enter
    // or operator pointer event. End that observation interval before checking
    // what the bounded delivery below adds.
    let _focus_events = fixture.summaries(Duration::from_millis(250));

    let sequence = InputSequence::new(vec![
        InputEvent::KeyRelease(Key::Escape),
        InputEvent::KeyPress(Key::Enter),
        InputEvent::KeyRelease(Key::Enter),
        InputEvent::Text("system-probe".to_owned()),
    ])
    .expect("valid");
    let receipt = controller
        .execute(
            &InputRequest::new(
                chosen.id(),
                sequence,
                DeliveryPlan::require(InputDelivery::System),
            )
            .with_focus(FocusPolicy::RequireFocused),
            &context(),
        )
        .expect("a receipt");

    assert_eq!(
        receipt.outcome(),
        SequenceOutcome::Complete,
        "delivery stopped: {receipt}"
    );
    assert_eq!(receipt.submitted(), 4);
    assert_eq!(receipt.selected_route(), Some(InputDelivery::System));

    let observed = fixture.summaries(Duration::from_secs(2));
    assert!(
        observed
            .iter()
            .filter(|kind| **kind == EVENT_KEY_DOWN)
            .count()
            >= 2,
        "the fixture recorded {observed:?}"
    );
    assert!(observed.contains(&EVENT_KEY_UP));
    assert!(
        !observed.contains(&EVENT_POINTER_MOVE),
        "this check sends no pointer input"
    );

    controller.close(&context()).expect("close");
    capture.close(&context()).expect("capture close");
    assert!(controller.is_closed());
}

fn frontmost_application() -> Option<(String, u32)> {
    let front = Command::new("/usr/bin/lsappinfo")
        .arg("front")
        .output()
        .ok()?;
    if !front.status.success() {
        return None;
    }
    let asn = String::from_utf8(front.stdout).ok()?.trim().to_owned();
    if asn.is_empty() {
        return None;
    }
    let info = Command::new("/usr/bin/lsappinfo")
        .args(["info", "-only", "pid"])
        .arg(&asn)
        .output()
        .ok()?;
    if !info.status.success() {
        return None;
    }
    let pid = String::from_utf8(info.stdout)
        .ok()?
        .trim()
        .strip_prefix("\"pid\"=")?
        .parse()
        .ok()?;
    Some((asn, pid))
}

fn pointer_location() -> (f64, f64) {
    let mut x = f64::NAN;
    let mut y = f64::NAN;
    // SAFETY: both outputs are writable for the duration of the production-shim call.
    let status = unsafe { mp_shim_input_pointer_location(&raw mut x, &raw mut y) };
    assert_eq!(status, 0, "the system pointer location must be observable");
    assert!(x.is_finite() && y.is_finite());
    (x, y)
}
fn observe_fixture_fill(
    capture: &dyn CaptureSession,
    after: FrameStamp,
    replacement: bool,
) -> FrameStamp {
    let deadline = Instant::now() + CONTENT_WAIT;
    let mut cursor = after;
    while Instant::now() < deadline {
        match capture.frame(
            &FrameRequest::newer_than(cursor),
            &bounded(Duration::from_millis(500)),
        ) {
            Ok(frame) => {
                cursor = frame.stamp();
                let mapping = frame
                    .map(PixelFormat::Bgra8, &bounded(CONTENT_WAIT))
                    .expect("a controlled fixture frame maps");
                let descriptor = mapping.descriptor();
                let matches = if replacement {
                    frame_is_replacement_content(
                        mapping.bytes(),
                        descriptor.stride(),
                        descriptor.extent(),
                    )
                } else {
                    frame_is_fixture_content(
                        mapping.bytes(),
                        descriptor.stride(),
                        descriptor.extent(),
                    )
                };
                if matches {
                    return cursor;
                }
            }
            Err(error) if error.status() == Status::DeadlineExceeded => {}
            Err(error) => panic!("controlled frame observation failed: {error}"),
        }
    }
    panic!("the controlled fixture transition produced no matching newer frame");
}

fn assert_process_receipt(receipt: &mado_pilot_input::InputReceipt, submitted: usize) {
    assert_eq!(receipt.outcome(), SequenceOutcome::Complete, "{receipt}");
    assert_eq!(receipt.submitted(), submitted);
    assert_eq!(
        receipt.selected_route(),
        Some(InputDelivery::ProcessDirected)
    );
    assert_eq!(
        receipt.address_scope(),
        Some(InputAddressScope::OwningProcess)
    );
    assert_eq!(receipt.evidence(), Some(SubmissionEvidence::InvocationOnly));
    assert_eq!(receipt.cleanup(), CleanupState::NotNeeded);
    assert!(!receipt.used_fallback());
}

fn process_key_pair(target: TargetId) -> InputRequest {
    InputRequest::new(
        target,
        InputSequence::new(vec![
            InputEvent::KeyPress(Key::Enter),
            InputEvent::KeyRelease(Key::Enter),
        ])
        .expect("the lifecycle probe key pair is balanced"),
        DeliveryPlan::require(InputDelivery::ProcessDirected),
    )
    .with_focus(FocusPolicy::Preserve)
}
fn wait_for_process_unavailable(provider: &MacosCaptureProvider, target: TargetId, wait: Duration) {
    let deadline = Instant::now() + wait;
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match InputProvider::describe(provider, target, &bounded(remaining)) {
            Ok(descriptor)
                if descriptor
                    .capability()
                    .pair(InputOperationKind::Keyboard, InputDelivery::ProcessDirected)
                    .support()
                    == CapabilitySupport::Unsupported =>
            {
                return;
            }
            Ok(_) => {}
            Err(error) if error.status() == Status::TargetLost => return,
            Err(error) if error.status() == Status::DeadlineExceeded => {}
            Err(error) => panic!("process capability refresh failed: {error}"),
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("the minimized process remained input-eligible past the scenario deadline");
}

fn assert_zero_effect(receipt: &mado_pilot_input::InputReceipt, fault: InputFault) {
    assert_eq!(receipt.outcome(), SequenceOutcome::Unexecuted, "{receipt}");
    assert_eq!(receipt.fault(), Some(fault), "{receipt}");
    assert_eq!(receipt.submitted(), 0, "{receipt}");
    assert_eq!(receipt.selected_route(), None, "{receipt}");
    assert!(!receipt.possible_native_effect(), "{receipt}");
    assert_eq!(receipt.cleanup(), CleanupState::NotNeeded, "{receipt}");
}

fn assert_unrelated_desktop_state(
    fixture: &Fixture,
    foreground_fixture: &mut Fixture,
    foreground_before: &(String, u32),
    cursor_before: (f64, f64),
) {
    let foreground_events = foreground_fixture.event_totals(CONTENT_WAIT);
    assert_eq!(
        foreground_events,
        EventTotals::default(),
        "the unrelated foreground fixture observed target-process input"
    );
    let unexpected = foreground_fixture.event_summaries(1, Duration::from_millis(150));
    assert!(
        unexpected.is_empty(),
        "the unrelated foreground fixture reported target-process events: {unexpected:?}"
    );
    let foreground_after =
        frontmost_application().expect("the frontmost application remains observable");
    assert_eq!(
        &foreground_after,
        foreground_before,
        "the {:?}/{:?} process-directed route changed the unrelated foreground application",
        fixture.facts.mode(),
        fixture.facts.renderer(),
    );
    assert_eq!(
        pointer_location(),
        cursor_before,
        "the {:?}/{:?} process-directed route moved the physical cursor",
        fixture.facts.mode(),
        fixture.facts.renderer(),
    );
}

fn observe_controlled_transition(
    fixture: &mut Fixture,
    capture: &dyn CaptureSession,
    after: FrameStamp,
    replacement: bool,
) -> FrameStamp {
    let transition = fixture
        .command(FixtureCommandKind::Transition, CONTENT_WAIT)
        .expect("the separate controlled visual transition completes");
    assert_eq!(transition.status, 0);
    assert_eq!(transition.before_window, fixture.facts.window_number());
    assert_eq!(transition.after_window, fixture.facts.window_number());
    observe_fixture_fill(capture, after, replacement)
}

const QUALIFICATION_POINTER_SPACES: [CoordinateSpace; 5] = [
    CoordinateSpace::CapturePixels,
    CoordinateSpace::FrameNormalized,
    CoordinateSpace::TargetNormalized,
    CoordinateSpace::TargetLogical,
    CoordinateSpace::DesktopLogical,
];

type PointerQualificationRow = (&'static str, Vec<InputEvent>, Vec<u32>);

fn pointer_qualification_rows(
    frame: &Frame,
    space: CoordinateSpace,
) -> Vec<PointerQualificationRow> {
    let extent = frame.descriptor().extent();
    let capture_centre = Point::new(
        CoordinateSpace::CapturePixels,
        f64::from(extent.width()) / 2.0,
        f64::from(extent.height()) / 2.0,
    )
    .expect("the frame centre is finite");
    let capture_drag_end = Point::new(
        CoordinateSpace::CapturePixels,
        capture_centre.x() + 24.0,
        capture_centre.y() + 12.0,
    )
    .expect("the drag endpoint is finite");
    let centre = frame
        .transform()
        .convert_point(capture_centre, space)
        .unwrap_or_else(|error| panic!("{space} centre conversion failed: {error}"));
    let drag_end = frame
        .transform()
        .convert_point(capture_drag_end, space)
        .unwrap_or_else(|error| panic!("{space} drag conversion failed: {error}"));

    vec![
        (
            "move",
            vec![InputEvent::PointerMove(centre)],
            vec![EVENT_POINTER_MOVE],
        ),
        (
            "primary drag",
            vec![
                InputEvent::PointerMove(centre),
                InputEvent::PointerPress(PointerButton::Primary),
                InputEvent::PointerMove(drag_end),
                InputEvent::PointerRelease(PointerButton::Primary),
            ],
            vec![
                EVENT_POINTER_MOVE,
                EVENT_POINTER_PRESS,
                EVENT_POINTER_MOVE,
                EVENT_POINTER_RELEASE,
            ],
        ),
        (
            "secondary click",
            vec![
                InputEvent::PointerMove(centre),
                InputEvent::PointerPress(PointerButton::Secondary),
                InputEvent::PointerRelease(PointerButton::Secondary),
            ],
            vec![
                EVENT_POINTER_MOVE,
                EVENT_POINTER_PRESS,
                EVENT_POINTER_RELEASE,
            ],
        ),
        (
            "middle click",
            vec![
                InputEvent::PointerMove(centre),
                InputEvent::PointerPress(PointerButton::Middle),
                InputEvent::PointerRelease(PointerButton::Middle),
            ],
            vec![
                EVENT_POINTER_MOVE,
                EVENT_POINTER_PRESS,
                EVENT_POINTER_RELEASE,
            ],
        ),
        (
            "scroll",
            vec![
                InputEvent::PointerMove(centre),
                InputEvent::PointerScroll {
                    horizontal: 1,
                    vertical: -1,
                },
            ],
            vec![EVENT_POINTER_MOVE, EVENT_POINTER_SCROLL],
        ),
    ]
}

#[allow(clippy::too_many_arguments)]
fn exercise_process_pointer_rows(
    input: &dyn InputController,
    target: TargetId,
    frame: &Frame,
    geometry: PointerGeometry,
    fixture: &mut Fixture,
    foreground_fixture: &mut Fixture,
    foreground_before: &(String, u32),
) {
    for space in QUALIFICATION_POINTER_SPACES {
        for (label, events, expected_kinds) in pointer_qualification_rows(frame, space) {
            fixture.begin_event_row(CONTENT_WAIT);
            foreground_fixture.begin_event_row(CONTENT_WAIT);
            let cursor_before = pointer_location();
            let submitted = events.len();
            let receipt = input
                .execute(
                    &InputRequest::new(
                        target,
                        InputSequence::new(events)
                            .expect("the pointer qualification row is bounded and balanced"),
                        DeliveryPlan::require(InputDelivery::ProcessDirected),
                    )
                    .with_focus(FocusPolicy::Preserve)
                    .with_pointer_geometry(geometry),
                    &bounded(CONTENT_WAIT),
                )
                .unwrap_or_else(|error| {
                    panic!(
                        "{label}/{space}/{} process posting failed: {error}",
                        geometry.policy()
                    )
                });
            assert_process_receipt(&receipt, submitted);
            fixture.expect_event_kinds(&expected_kinds, CONTENT_WAIT);
            assert_unrelated_desktop_state(
                fixture,
                foreground_fixture,
                foreground_before,
                cursor_before,
            );
        }
    }
}

fn wait_for_geometry_frame(capture: &dyn CaptureSession, after: FrameStamp) -> Frame {
    let deadline = Instant::now() + CONTENT_WAIT;
    let mut cursor = after;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "capture did not publish changed geometry before the scenario deadline"
        );
        let frame = capture
            .frame(&FrameRequest::newer_than(cursor), &bounded(remaining))
            .expect("capture publishes while geometry changes");
        cursor = frame.stamp();
        if frame.stamp().geometry() == after.geometry() {
            continue;
        }
        let mapping = frame
            .map(PixelFormat::Bgra8, &bounded(remaining))
            .expect("the geometry-updated frame maps");
        let descriptor = mapping.descriptor();
        assert!(
            frame_is_fixture_content(mapping.bytes(), descriptor.stride(), descriptor.extent(),)
                || frame_is_replacement_content(
                    mapping.bytes(),
                    descriptor.stride(),
                    descriptor.extent(),
                ),
            "the geometry-updated frame contains only approved fixture content"
        );
        return frame;
    }
}

fn assert_stale_pointer_frame_refused(
    input: &dyn InputController,
    target: TargetId,
    stale_frame: &Frame,
    fixture: &mut Fixture,
    foreground_fixture: &mut Fixture,
    foreground_before: &(String, u32),
) {
    let extent = stale_frame.descriptor().extent();
    let point = Point::new(
        CoordinateSpace::CapturePixels,
        f64::from(extent.width()) / 2.0,
        f64::from(extent.height()) / 2.0,
    )
    .expect("the stale frame centre is finite");
    fixture.begin_event_row(CONTENT_WAIT);
    foreground_fixture.begin_event_row(CONTENT_WAIT);
    let cursor_before = pointer_location();
    let receipt = input
        .execute(
            &InputRequest::new(
                target,
                InputSequence::new(vec![InputEvent::PointerMove(point)])
                    .expect("the stale geometry row is bounded"),
                DeliveryPlan::require(InputDelivery::ProcessDirected),
            )
            .with_focus(FocusPolicy::Preserve)
            .with_pointer_geometry(PointerGeometry::require_unchanged_since(
                stale_frame.stamp(),
            )),
            &bounded(CONTENT_WAIT),
        )
        .expect("stale geometry returns a receipt");
    let fault = receipt
        .fault()
        .expect("the stale coordinate source returns a typed refusal");
    assert!(
        matches!(
            fault,
            InputFault::GeometryChanged | InputFault::UnsupportedCoordinate
        ),
        "stale frame refusal was {fault:?}"
    );
    assert_zero_effect(&receipt, fault);
    assert_eq!(fixture.event_totals(CONTENT_WAIT), EventTotals::default());
    assert_unrelated_desktop_state(
        fixture,
        foreground_fixture,
        foreground_before,
        cursor_before,
    );
}

fn qualify_process_directed_renderer(mode: FixtureMode) {
    let mut foreground_fixture =
        Fixture::start().expect("the unrelated foreground fixture starts first");
    let mut fixture = Fixture::start_inactive(mode)
        .expect("the owned fixture starts visible without taking foreground ownership");

    let provider = provider();
    let chosen = discover_unique_fixture(&provider, fixture.process_id, CONTENT_WAIT)
        .expect("the owned child exposes exactly one eligible fixture window");
    for kind in InputOperationKind::ALL {
        let pair = chosen
            .capability()
            .input()
            .pair(kind, InputDelivery::ProcessDirected);
        assert_eq!(
            pair.support(),
            CapabilitySupport::Unknown,
            "the admitted fixture omitted process-directed {}",
            kind.as_str()
        );
        assert_eq!(pair.evidence(), Some(SubmissionEvidence::InvocationOnly));
        assert!(!pair.focus_required());
    }
    let capture = CaptureProvider::open(
        &provider,
        chosen.id(),
        &OpenRequest::new().require_format(PixelFormat::Bgra8),
        &bounded(CONTENT_WAIT),
    )
    .expect("capture opens for the exact retained fixture");
    let opened_descriptor = InputProvider::describe(&provider, chosen.id(), &bounded(CONTENT_WAIT))
        .expect("the retained fixture remains describable after capture opens");
    assert_eq!(
        opened_descriptor
            .capability()
            .pair(InputOperationKind::Pointer, InputDelivery::ProcessDirected)
            .support(),
        CapabilitySupport::Unknown,
        "active window capture must preserve process-directed admission"
    );
    let first = capture
        .frame(&FrameRequest::latest(), &bounded(CONTENT_WAIT))
        .expect("the fixture publishes its initial frame");
    {
        let initial = first
            .map(PixelFormat::Bgra8, &bounded(CONTENT_WAIT))
            .expect("the initial frame maps");
        let descriptor = initial.descriptor();
        assert!(frame_is_fixture_content(
            initial.bytes(),
            descriptor.stride(),
            descriptor.extent(),
        ));
    }
    let auxiliary = fixture
        .command(FixtureCommandKind::OpenAuxiliary, CONTENT_WAIT)
        .expect("the additional ordinary window opens");
    assert_eq!(auxiliary.status, 0);
    thread::sleep(Duration::from_secs(10));
    let mut observed_stamp =
        observe_controlled_transition(&mut fixture, capture.as_ref(), first.stamp(), true);
    let multiple_window_descriptor =
        InputProvider::describe(&provider, chosen.id(), &bounded(CONTENT_WAIT))
            .expect("the retained target remains describable with an additional window");
    for kind in InputOperationKind::ALL {
        assert_eq!(
            multiple_window_descriptor
                .capability()
                .pair(kind, InputDelivery::ProcessDirected)
                .support(),
            CapabilitySupport::Unknown,
            "sustained capture plus an additional window revoked process-directed {}",
            kind.as_str()
        );
    }
    let foreground_deadline = Instant::now() + CONTENT_WAIT;
    let foreground_before = loop {
        if let Some(foreground) = frontmost_application()
            && foreground.1 == foreground_fixture.process_id
        {
            break foreground;
        }
        assert!(
            Instant::now() < foreground_deadline,
            "the inactive qualification target stole foreground ownership"
        );
        thread::sleep(Duration::from_millis(25));
    };
    assert_ne!(
        foreground_before.1, fixture.process_id,
        "the qualification target must remain inactive"
    );

    let inactive_descriptor =
        InputProvider::describe(&provider, chosen.id(), &bounded(CONTENT_WAIT))
            .expect("the inactive retained fixture remains describable");
    for kind in InputOperationKind::ALL {
        assert_eq!(
            inactive_descriptor
                .capability()
                .pair(kind, InputDelivery::ProcessDirected)
                .support(),
            CapabilitySupport::Unknown,
            "the inactive retained fixture lost process-directed {} admission",
            kind.as_str()
        );
    }

    let input = InputProvider::open(
        &provider,
        chosen.id(),
        &InputOpenRequest::new()
            .with_requirement(InputRequirement::Required)
            .requiring(InputOperationKind::Pointer, InputDelivery::ProcessDirected)
            .requiring(InputOperationKind::Keyboard, InputDelivery::ProcessDirected)
            .requiring(InputOperationKind::Text, InputDelivery::ProcessDirected),
        &bounded(CONTENT_WAIT),
    )
    .expect("all candidate process-directed pairs open for qualification");

    exercise_process_pointer_rows(
        input.as_ref(),
        chosen.id(),
        &first,
        PointerGeometry::require_unchanged_since(first.stamp()),
        &mut fixture,
        &mut foreground_fixture,
        &foreground_before,
    );

    let moved = fixture
        .command(FixtureCommandKind::Move, CONTENT_WAIT)
        .expect("the local movement command completes");
    assert_eq!(moved.status, 0);
    let moved_frame = wait_for_geometry_frame(capture.as_ref(), observed_stamp);
    assert_stale_pointer_frame_refused(
        input.as_ref(),
        chosen.id(),
        &first,
        &mut fixture,
        &mut foreground_fixture,
        &foreground_before,
    );
    exercise_process_pointer_rows(
        input.as_ref(),
        chosen.id(),
        &moved_frame,
        PointerGeometry::reprojected(),
        &mut fixture,
        &mut foreground_fixture,
        &foreground_before,
    );
    let moved_current = capture
        .frame(&FrameRequest::latest(), &bounded(CONTENT_WAIT))
        .expect("the moved fixture keeps publishing");
    exercise_process_pointer_rows(
        input.as_ref(),
        chosen.id(),
        &moved_current,
        PointerGeometry::require_unchanged_since(moved_current.stamp()),
        &mut fixture,
        &mut foreground_fixture,
        &foreground_before,
    );
    observed_stamp = moved_current.stamp();

    let resized = fixture
        .command(FixtureCommandKind::Resize, CONTENT_WAIT)
        .expect("the resize command completes");
    assert_eq!(resized.status, 0);
    let resized_frame = wait_for_geometry_frame(capture.as_ref(), observed_stamp);
    assert_stale_pointer_frame_refused(
        input.as_ref(),
        chosen.id(),
        &moved_current,
        &mut fixture,
        &mut foreground_fixture,
        &foreground_before,
    );
    exercise_process_pointer_rows(
        input.as_ref(),
        chosen.id(),
        &resized_frame,
        PointerGeometry::reprojected(),
        &mut fixture,
        &mut foreground_fixture,
        &foreground_before,
    );
    let resized_current = capture
        .frame(&FrameRequest::latest(), &bounded(CONTENT_WAIT))
        .expect("the resized fixture keeps publishing");
    exercise_process_pointer_rows(
        input.as_ref(),
        chosen.id(),
        &resized_current,
        PointerGeometry::require_unchanged_since(resized_current.stamp()),
        &mut fixture,
        &mut foreground_fixture,
        &foreground_before,
    );
    observed_stamp = resized_current.stamp();

    let display_count = discovered(&provider)
        .expect("the qualifying host remains discoverable")
        .iter()
        .filter(|target| target.capability().kind() == Some(TargetKind::Display))
        .count();
    assert!(
        (1..=16).contains(&display_count),
        "the bounded public display inventory is non-empty"
    );
    let mut topology_frame = resized_current;
    for _ in 1..display_count {
        let moved_display = fixture
            .command(FixtureCommandKind::MoveToNextDisplay, CONTENT_WAIT)
            .expect("the inter-display movement command completes");
        assert_eq!(moved_display.status, 0);
        let next_frame = wait_for_geometry_frame(capture.as_ref(), observed_stamp);
        assert_stale_pointer_frame_refused(
            input.as_ref(),
            chosen.id(),
            &topology_frame,
            &mut fixture,
            &mut foreground_fixture,
            &foreground_before,
        );
        exercise_process_pointer_rows(
            input.as_ref(),
            chosen.id(),
            &next_frame,
            PointerGeometry::reprojected(),
            &mut fixture,
            &mut foreground_fixture,
            &foreground_before,
        );
        let next_current = capture
            .frame(&FrameRequest::latest(), &bounded(CONTENT_WAIT))
            .expect("the inter-display fixture keeps publishing");
        exercise_process_pointer_rows(
            input.as_ref(),
            chosen.id(),
            &next_current,
            PointerGeometry::require_unchanged_since(next_current.stamp()),
            &mut fixture,
            &mut foreground_fixture,
            &foreground_before,
        );
        observed_stamp = next_current.stamp();
        topology_frame = next_current;
    }
    observed_stamp =
        observe_controlled_transition(&mut fixture, capture.as_ref(), observed_stamp, false);

    let keyboard_rows = [
        (
            "printable layout character",
            vec![
                InputEvent::KeyPress(Key::Character('a')),
                InputEvent::KeyRelease(Key::Character('a')),
            ],
            vec![EVENT_KEY_DOWN, EVENT_KEY_UP],
        ),
        (
            "modifier chord",
            vec![
                InputEvent::KeyPress(Key::Modifier(Modifier::Shift)),
                InputEvent::KeyPress(Key::Character('b')),
                InputEvent::KeyRelease(Key::Character('b')),
                InputEvent::KeyRelease(Key::Modifier(Modifier::Shift)),
            ],
            vec![
                EVENT_FLAGS_CHANGED,
                EVENT_KEY_DOWN,
                EVENT_KEY_UP,
                EVENT_FLAGS_CHANGED,
            ],
        ),
        (
            "Enter",
            vec![
                InputEvent::KeyPress(Key::Enter),
                InputEvent::KeyRelease(Key::Enter),
            ],
            vec![EVENT_KEY_DOWN, EVENT_KEY_UP],
        ),
        (
            "F1",
            vec![
                InputEvent::KeyPress(Key::Function(1)),
                InputEvent::KeyRelease(Key::Function(1)),
            ],
            vec![EVENT_KEY_DOWN, EVENT_KEY_UP],
        ),
        (
            "right arrow",
            vec![
                InputEvent::KeyPress(Key::ArrowRight),
                InputEvent::KeyRelease(Key::ArrowRight),
            ],
            vec![EVENT_KEY_DOWN, EVENT_KEY_UP],
        ),
    ];
    for (label, events, expected_kinds) in keyboard_rows {
        fixture.begin_event_row(CONTENT_WAIT);
        foreground_fixture.begin_event_row(CONTENT_WAIT);
        let cursor_before = pointer_location();
        let submitted = events.len();
        let receipt = input
            .execute(
                &InputRequest::new(
                    chosen.id(),
                    InputSequence::new(events).expect("the keyboard row is bounded and balanced"),
                    DeliveryPlan::require(InputDelivery::ProcessDirected),
                )
                .with_focus(FocusPolicy::Preserve),
                &bounded(CONTENT_WAIT),
            )
            .unwrap_or_else(|error| panic!("{label} process posting failed: {error}"));
        assert_process_receipt(&receipt, submitted);
        fixture.expect_event_kinds(&expected_kinds, CONTENT_WAIT);
        assert_unrelated_desktop_state(
            &fixture,
            &mut foreground_fixture,
            &foreground_before,
            cursor_before,
        );
    }
    observed_stamp =
        observe_controlled_transition(&mut fixture, capture.as_ref(), observed_stamp, true);

    const PROCESS_TEXT_CHUNK_UNITS: usize = 16;
    let mut boundary_text = "x".repeat(PROCESS_TEXT_CHUNK_UNITS - 1);
    boundary_text.push('\u{1F642}');
    boundary_text.push('y');
    assert_eq!(
        InputEvent::MAX_TEXT_CHARS % PROCESS_TEXT_CHUNK_UNITS,
        0,
        "the maximum row must have an exact observable chunk count"
    );
    let maximum_text = "x".repeat(InputEvent::MAX_TEXT_CHARS);
    let text_rows = [
        (
            "BMP plus surrogate pair",
            "λ🙂".to_owned(),
            vec![3u32],
            CONTENT_WAIT,
        ),
        (
            "surrogate at the native chunk boundary",
            boundary_text,
            vec![15u32, 3u32],
            CONTENT_WAIT,
        ),
        (
            "maximum representable text",
            maximum_text,
            vec![
                PROCESS_TEXT_CHUNK_UNITS as u32;
                InputEvent::MAX_TEXT_CHARS / PROCESS_TEXT_CHUNK_UNITS
            ],
            Duration::from_secs(30),
        ),
    ];
    for (label, text, expected_units, wait) in text_rows {
        fixture.begin_event_row(wait);
        foreground_fixture.begin_event_row(wait);
        let cursor_before = pointer_location();
        let receipt = input
            .execute(
                &InputRequest::new(
                    chosen.id(),
                    InputSequence::new(vec![InputEvent::Text(text)])
                        .expect("the exact text qualification row is bounded"),
                    DeliveryPlan::require(InputDelivery::ProcessDirected),
                )
                .with_focus(FocusPolicy::Preserve),
                &bounded(wait),
            )
            .unwrap_or_else(|error| panic!("{label} process posting failed: {error}"));
        assert_process_receipt(&receipt, 1);
        fixture.expect_text_chunks(&expected_units, wait);
        assert_unrelated_desktop_state(
            &fixture,
            &mut foreground_fixture,
            &foreground_before,
            cursor_before,
        );
    }
    fixture.begin_event_row(CONTENT_WAIT);
    foreground_fixture.begin_event_row(CONTENT_WAIT);
    let pressed = EventSummary {
        kind: EVENT_FLAGS_CHANGED,
        text_units: 0,
    };
    let cancellation = CancellationToken::new();
    let cancellation_observer =
        fixture.cancel_after_event(pressed, cancellation.clone(), CONTENT_WAIT);
    let cursor_before = pointer_location();
    let cancellation_receipt = input
        .execute(
            &InputRequest::new(
                chosen.id(),
                InputSequence::new(vec![
                    InputEvent::KeyPress(Key::Modifier(Modifier::Shift)),
                    InputEvent::Delay(Duration::from_secs(5)),
                    InputEvent::KeyRelease(Key::Modifier(Modifier::Shift)),
                ])
                .expect("the cancellation row has bounded pressed state"),
                DeliveryPlan::require(InputDelivery::ProcessDirected),
            )
            .with_focus(FocusPolicy::Preserve),
            &bounded(CONTENT_WAIT).with_cancellation(cancellation),
        )
        .expect("the cancelled process-directed row returns a receipt");
    let observed_press = cancellation_observer
        .join()
        .expect("the cancellation observation helper remains contained");
    assert_eq!(observed_press, Some(pressed));
    assert_eq!(
        cancellation_receipt.outcome(),
        SequenceOutcome::Partial,
        "{cancellation_receipt}"
    );
    assert_eq!(cancellation_receipt.submitted(), 1);
    assert_eq!(
        cancellation_receipt.selected_route(),
        Some(InputDelivery::ProcessDirected)
    );
    assert_eq!(
        cancellation_receipt.address_scope(),
        Some(InputAddressScope::OwningProcess)
    );
    assert_eq!(
        cancellation_receipt.evidence(),
        Some(SubmissionEvidence::InvocationOnly)
    );
    assert_eq!(cancellation_receipt.fault(), Some(InputFault::Cancelled));
    assert_eq!(cancellation_receipt.cleanup(), CleanupState::Complete);
    assert_eq!(cancellation_receipt.cleanup_owed(), 1);
    assert_eq!(cancellation_receipt.cleanup_released(), 1);
    assert!(!cancellation_receipt.used_fallback());
    let released = EventSummary {
        kind: EVENT_FLAGS_CHANGED,
        text_units: 0,
    };
    assert_eq!(fixture.exact_event_summaries(1, CONTENT_WAIT), [released]);
    assert_eq!(
        fixture.event_totals(CONTENT_WAIT),
        event_totals(&[pressed, released])
    );
    assert_unrelated_desktop_state(
        &fixture,
        &mut foreground_fixture,
        &foreground_before,
        cursor_before,
    );
    let _final_stamp =
        observe_controlled_transition(&mut fixture, capture.as_ref(), observed_stamp, false);
    let auxiliary_closed = fixture
        .command(FixtureCommandKind::CloseAuxiliary, CONTENT_WAIT)
        .expect("the additional ordinary window closes");
    assert_eq!(auxiliary_closed.status, 0);

    input.close(&bounded(CONTENT_WAIT)).expect("input closes");
    input
        .close(&bounded(CONTENT_WAIT))
        .expect("repeated input close is idempotent");
    capture
        .close(&bounded(CONTENT_WAIT))
        .expect("capture closes");
    capture
        .close(&bounded(CONTENT_WAIT))
        .expect("repeated capture close is idempotent");
    let stopped = fixture
        .command(FixtureCommandKind::Stop, CONTENT_WAIT)
        .expect("owned fixture stop is acknowledged");
    assert_eq!(stopped.status, 0);
    let foreground_stopped = foreground_fixture
        .command(FixtureCommandKind::Stop, CONTENT_WAIT)
        .expect("the unrelated foreground fixture stop is acknowledged");
    assert_eq!(foreground_stopped.status, 0);
    fixture.input = None;
}

/// Qualifies every positive operation row through both private renderer modes.
#[test]
#[ignore = "delivers real process-directed input on an interactive desktop"]
fn process_directed_delivery_qualifies_default_and_game_like_renderers() {
    assert!(
        std::env::var_os("MADO_PILOT_MACOS_FIXTURE_EXECUTABLE").is_some(),
        "qualification requires the configured signed fixture bundle"
    );
    assert!(
        post_event_access_granted(),
        "qualification requires non-prompting post-event authorization to be granted"
    );
    for mode in [FixtureMode::GameLike, FixtureMode::Default] {
        qualify_process_directed_renderer(mode);
    }
}

/// Keeps capture active beyond the indicator dwell while two renderer modes
/// receive spaced process-directed sequences under an unrelated foreground.
#[test]
#[ignore = "runs a bounded sustained-capture soak and delivers process-directed input"]
fn sustained_capture_soak_keeps_process_route_isolated() {
    assert!(
        std::env::var_os("MADO_PILOT_MACOS_FIXTURE_EXECUTABLE").is_some(),
        "qualification requires the configured signed fixture bundle"
    );
    assert!(
        post_event_access_granted(),
        "qualification requires non-prompting post-event authorization to be granted"
    );

    for mode in [FixtureMode::GameLike, FixtureMode::Default] {
        let mut foreground_fixture =
            Fixture::start().expect("the unrelated foreground fixture starts first");
        let mut fixture = Fixture::start_inactive(mode)
            .expect("the owned fixture starts visible without taking foreground ownership");
        let provider = provider();
        let chosen = discover_unique_fixture(&provider, fixture.process_id, CONTENT_WAIT)
            .expect("the owned child exposes exactly one eligible fixture window");
        let capture = CaptureProvider::open(
            &provider,
            chosen.id(),
            &OpenRequest::new().require_format(PixelFormat::Bgra8),
            &bounded(CONTENT_WAIT),
        )
        .expect("capture opens for the exact retained fixture");
        let capture_started = Instant::now();
        let first = capture
            .frame(&FrameRequest::latest(), &bounded(CONTENT_WAIT))
            .expect("the fixture publishes its initial frame");
        let auxiliary = fixture
            .command(FixtureCommandKind::OpenAuxiliary, CONTENT_WAIT)
            .expect("the additional ordinary window opens");
        assert_eq!(auxiliary.status, 0);
        thread::sleep(Duration::from_secs(10));

        let foreground_deadline = Instant::now() + CONTENT_WAIT;
        let foreground_before = loop {
            if let Some(foreground) = frontmost_application()
                && foreground.1 == foreground_fixture.process_id
            {
                break foreground;
            }
            assert!(
                Instant::now() < foreground_deadline,
                "the inactive soak target stole foreground ownership"
            );
            thread::sleep(Duration::from_millis(25));
        };

        let input = InputProvider::open(
            &provider,
            chosen.id(),
            &InputOpenRequest::new()
                .with_requirement(InputRequirement::Required)
                .requiring(InputOperationKind::Pointer, InputDelivery::ProcessDirected)
                .requiring(InputOperationKind::Keyboard, InputDelivery::ProcessDirected),
            &bounded(CONTENT_WAIT),
        )
        .expect("the soak process-directed pairs open");

        fixture.begin_event_row(CONTENT_WAIT);
        foreground_fixture.begin_event_row(CONTENT_WAIT);
        let cursor_before = pointer_location();
        let (_, pointer_events, expected_pointer_events) =
            pointer_qualification_rows(&first, CoordinateSpace::CapturePixels)
                .into_iter()
                .next()
                .expect("the pointer soak row exists");
        let submitted = pointer_events.len();
        let first_receipt = input
            .execute(
                &InputRequest::new(
                    chosen.id(),
                    InputSequence::new(pointer_events).expect("the pointer soak row is bounded"),
                    DeliveryPlan::require(InputDelivery::ProcessDirected),
                )
                .with_focus(FocusPolicy::Preserve)
                .with_pointer_geometry(PointerGeometry::require_unchanged_since(first.stamp())),
                &bounded(CONTENT_WAIT),
            )
            .expect("the first spaced process-directed sequence posts");
        assert_process_receipt(&first_receipt, submitted);
        fixture.expect_event_kinds(&expected_pointer_events, CONTENT_WAIT);
        assert_unrelated_desktop_state(
            &fixture,
            &mut foreground_fixture,
            &foreground_before,
            cursor_before,
        );
        let first_transition =
            observe_controlled_transition(&mut fixture, capture.as_ref(), first.stamp(), true);

        let soak_deadline = capture_started + SUSTAINED_CAPTURE_SOAK;
        while Instant::now() < soak_deadline {
            let latest = capture
                .frame(&FrameRequest::latest(), &bounded(CONTENT_WAIT))
                .expect("sustained capture keeps publishing retained fixture frames");
            assert!(
                latest.stamp().is_same_stream(&first_transition)
                    && latest.stamp().sequence().value() >= first_transition.sequence().value(),
                "sustained capture regressed the retained stream identity"
            );
            let mapping = latest
                .map(PixelFormat::Bgra8, &bounded(CONTENT_WAIT))
                .expect("a sustained-capture sample maps");
            assert!(
                frame_is_replacement_content(
                    mapping.bytes(),
                    mapping.descriptor().stride(),
                    mapping.descriptor().extent(),
                ),
                "ambient or unrelated pixels replaced the controlled retained content"
            );
            thread::sleep(
                soak_deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_secs(1)),
            );
        }

        fixture.begin_event_row(CONTENT_WAIT);
        foreground_fixture.begin_event_row(CONTENT_WAIT);
        let cursor_before = pointer_location();
        let second_receipt = input
            .execute(&process_key_pair(chosen.id()), &bounded(CONTENT_WAIT))
            .expect("the second spaced process-directed sequence posts");
        assert_process_receipt(&second_receipt, 2);
        fixture.expect_event_kinds(&[EVENT_KEY_DOWN, EVENT_KEY_UP], CONTENT_WAIT);
        assert_unrelated_desktop_state(
            &fixture,
            &mut foreground_fixture,
            &foreground_before,
            cursor_before,
        );
        let _second_transition =
            observe_controlled_transition(&mut fixture, capture.as_ref(), first_transition, false);
        assert!(
            capture_started.elapsed() >= SUSTAINED_CAPTURE_SOAK,
            "the capture soak ended before its frozen minimum"
        );

        let auxiliary_closed = fixture
            .command(FixtureCommandKind::CloseAuxiliary, CONTENT_WAIT)
            .expect("the additional ordinary window closes");
        assert_eq!(auxiliary_closed.status, 0);
        input.close(&bounded(CONTENT_WAIT)).expect("input closes");
        input
            .close(&bounded(CONTENT_WAIT))
            .expect("repeated input close is idempotent");
        capture
            .close(&bounded(CONTENT_WAIT))
            .expect("capture closes");
        capture
            .close(&bounded(CONTENT_WAIT))
            .expect("repeated capture close is idempotent");
        let stopped = fixture
            .command(FixtureCommandKind::Stop, CONTENT_WAIT)
            .expect("owned fixture stop is acknowledged");
        assert_eq!(stopped.status, 0);
        let foreground_stopped = foreground_fixture
            .command(FixtureCommandKind::Stop, CONTENT_WAIT)
            .expect("the unrelated foreground fixture stop is acknowledged");
        assert_eq!(foreground_stopped.status, 0);
        fixture.input = None;
        foreground_fixture.input = None;
    }
}

/// Keeps capture active with multiple same-process windows, then proves every
/// ordinary state transition is revalidated against retained process authority.
#[test]
#[ignore = "mutates real fixture windows and delivers process-directed input"]
fn process_directed_delivery_uses_process_authority_and_revalidates_window_state() {
    assert!(
        std::env::var_os("MADO_PILOT_MACOS_FIXTURE_EXECUTABLE").is_some(),
        "qualification requires the configured signed fixture bundle"
    );
    assert!(
        post_event_access_granted(),
        "qualification requires non-prompting post-event authorization to be granted"
    );
    let mut foreground_fixture =
        Fixture::start().expect("the unrelated foreground fixture starts first");
    let mut fixture = Fixture::start_inactive(FixtureMode::Default)
        .expect("the owned target fixture starts without taking foreground");
    let provider = provider();
    let chosen = discover_unique_fixture(&provider, fixture.process_id, CONTENT_WAIT)
        .expect("the owned child exposes exactly one eligible primary window");
    let capture = CaptureProvider::open(
        &provider,
        chosen.id(),
        &OpenRequest::new().require_format(PixelFormat::Bgra8),
        &bounded(CONTENT_WAIT),
    )
    .expect("desktop-independent capture opens for the retained target");
    let first = capture
        .frame(&FrameRequest::latest(), &bounded(CONTENT_WAIT))
        .expect("the retained target publishes its initial frame");
    let input = InputProvider::open(
        &provider,
        chosen.id(),
        &InputOpenRequest::new()
            .with_requirement(InputRequirement::Required)
            .requiring(InputOperationKind::Keyboard, InputDelivery::ProcessDirected),
        &bounded(CONTENT_WAIT),
    )
    .expect("the process-directed keyboard pair opens");

    let foreground_deadline = Instant::now() + CONTENT_WAIT;
    let foreground_before = loop {
        if let Some(foreground) = frontmost_application()
            && foreground.1 == foreground_fixture.process_id
        {
            break foreground;
        }
        assert!(
            Instant::now() < foreground_deadline,
            "the inactive qualification target stole foreground ownership"
        );
        thread::sleep(Duration::from_millis(25));
    };

    let auxiliary = fixture
        .command(FixtureCommandKind::OpenAuxiliary, CONTENT_WAIT)
        .expect("the auxiliary-window transition completes");
    assert_eq!(auxiliary.status, 0);
    thread::sleep(Duration::from_secs(10));
    let _active_capture_stamp =
        observe_controlled_transition(&mut fixture, capture.as_ref(), first.stamp(), true);
    let multiple_window_target =
        discover_unique_fixture(&provider, fixture.process_id, CONTENT_WAIT)
            .expect("additional same-process windows do not revoke process scope");
    for kind in InputOperationKind::ALL {
        assert_eq!(
            multiple_window_target
                .capability()
                .input()
                .pair(kind, InputDelivery::ProcessDirected)
                .support(),
            CapabilitySupport::Unknown,
            "an additional same-process window revoked process-directed {}",
            kind.as_str()
        );
    }
    fixture.begin_event_row(CONTENT_WAIT);
    foreground_fixture.begin_event_row(CONTENT_WAIT);
    let cursor_before = pointer_location();
    let multiple_window = input
        .execute(&process_key_pair(chosen.id()), &bounded(CONTENT_WAIT))
        .expect("the multi-window process returns a receipt");
    assert_process_receipt(&multiple_window, 2);
    fixture.expect_event_kinds(&[EVENT_KEY_DOWN, EVENT_KEY_UP], CONTENT_WAIT);
    assert_unrelated_desktop_state(
        &fixture,
        &mut foreground_fixture,
        &foreground_before,
        cursor_before,
    );

    let closed_auxiliary = fixture
        .command(FixtureCommandKind::CloseAuxiliary, CONTENT_WAIT)
        .expect("the auxiliary window closes");
    assert_eq!(closed_auxiliary.status, 0);
    fixture.begin_event_row(CONTENT_WAIT);
    foreground_fixture.begin_event_row(CONTENT_WAIT);
    let cursor_before = pointer_location();
    let ordinary = input
        .execute(&process_key_pair(chosen.id()), &bounded(CONTENT_WAIT))
        .expect("the single-window process returns a receipt");
    assert_process_receipt(&ordinary, 2);
    fixture.expect_event_kinds(&[EVENT_KEY_DOWN, EVENT_KEY_UP], CONTENT_WAIT);
    assert_unrelated_desktop_state(
        &fixture,
        &mut foreground_fixture,
        &foreground_before,
        cursor_before,
    );

    let minimized = fixture
        .command(FixtureCommandKind::Minimize, CONTENT_WAIT)
        .expect("the fixture minimizes");
    assert_eq!(minimized.status, 0);
    wait_for_process_unavailable(&provider, chosen.id(), CONTENT_WAIT);
    fixture.begin_event_row(CONTENT_WAIT);
    foreground_fixture.begin_event_row(CONTENT_WAIT);
    let cursor_before = pointer_location();
    let unavailable = input
        .execute(&process_key_pair(chosen.id()), &bounded(CONTENT_WAIT))
        .expect("a minimized target returns a receipt");
    let unavailable_fault = unavailable
        .fault()
        .expect("a minimized target reports why admission stopped");
    assert!(
        matches!(
            unavailable_fault,
            InputFault::TargetLost | InputFault::UnsupportedCombination
        ),
        "minimized-target refusal reported {unavailable_fault}"
    );
    assert_zero_effect(&unavailable, unavailable_fault);
    assert!(
        fixture
            .event_summaries(1, Duration::from_millis(200))
            .is_empty(),
        "a minimized target received input before refusal"
    );
    assert_eq!(fixture.event_totals(CONTENT_WAIT), EventTotals::default());
    assert_unrelated_desktop_state(
        &fixture,
        &mut foreground_fixture,
        &foreground_before,
        cursor_before,
    );

    let restored = fixture
        .command(FixtureCommandKind::Restore, CONTENT_WAIT)
        .expect("the fixture restores");
    assert_eq!(restored.status, 0);
    let restore_deadline = Instant::now() + CONTENT_WAIT;
    fixture.begin_event_row(CONTENT_WAIT);
    foreground_fixture.begin_event_row(CONTENT_WAIT);
    let cursor_before = pointer_location();
    let restored_receipt = loop {
        let receipt = input
            .execute(&process_key_pair(chosen.id()), &bounded(CONTENT_WAIT))
            .expect("the restored target returns a receipt");
        if receipt.outcome() == SequenceOutcome::Complete {
            break receipt;
        }
        assert!(
            receipt.outcome() == SequenceOutcome::Unexecuted
                && matches!(
                    receipt.fault(),
                    Some(InputFault::TargetLost | InputFault::UnsupportedCombination)
                )
                && Instant::now() < restore_deadline,
            "the restored target did not regain process authority: {receipt}"
        );
        thread::sleep(Duration::from_millis(25));
    };
    assert_process_receipt(&restored_receipt, 2);
    fixture.expect_event_kinds(&[EVENT_KEY_DOWN, EVENT_KEY_UP], CONTENT_WAIT);
    assert_unrelated_desktop_state(
        &fixture,
        &mut foreground_fixture,
        &foreground_before,
        cursor_before,
    );

    for kind in [FixtureCommandKind::Move, FixtureCommandKind::Resize] {
        let transition = fixture
            .command(kind, CONTENT_WAIT)
            .expect("the geometry transition completes");
        assert_eq!(transition.status, 0);
        fixture.begin_event_row(CONTENT_WAIT);
        foreground_fixture.begin_event_row(CONTENT_WAIT);
        let cursor_before = pointer_location();
        let receipt = input
            .execute(&process_key_pair(chosen.id()), &bounded(CONTENT_WAIT))
            .expect("the geometry-updated target returns a receipt");
        assert_process_receipt(&receipt, 2);
        fixture.expect_event_kinds(&[EVENT_KEY_DOWN, EVENT_KEY_UP], CONTENT_WAIT);
        assert_unrelated_desktop_state(
            &fixture,
            &mut foreground_fixture,
            &foreground_before,
            cursor_before,
        );
    }

    let replacement = fixture
        .command(FixtureCommandKind::Replace, CONTENT_WAIT)
        .expect("the owned fixture replaces its window");
    assert_eq!(replacement.status, 0);
    assert_ne!(replacement.before_window, replacement.after_window);
    wait_for_process_unavailable(&provider, chosen.id(), CONTENT_WAIT);
    fixture.begin_event_row(CONTENT_WAIT);
    foreground_fixture.begin_event_row(CONTENT_WAIT);
    let cursor_before = pointer_location();
    let replaced = input
        .execute(&process_key_pair(chosen.id()), &bounded(CONTENT_WAIT))
        .expect("the replaced target returns a receipt");
    let replacement_fault = replaced
        .fault()
        .expect("a replaced target reports why admission stopped");
    assert!(
        matches!(
            replacement_fault,
            InputFault::TargetLost | InputFault::UnsupportedCombination
        ),
        "replacement refusal reported {replacement_fault}"
    );
    assert_zero_effect(&replaced, replacement_fault);
    assert!(
        fixture
            .event_summaries(1, Duration::from_millis(200))
            .is_empty(),
        "a replacement window received input addressed through the stale target"
    );
    assert_eq!(fixture.event_totals(CONTENT_WAIT), EventTotals::default());
    assert_unrelated_desktop_state(
        &fixture,
        &mut foreground_fixture,
        &foreground_before,
        cursor_before,
    );

    input.close(&bounded(CONTENT_WAIT)).expect("input closes");
    input
        .close(&bounded(CONTENT_WAIT))
        .expect("repeated input close is idempotent");
    capture
        .close(&bounded(CONTENT_WAIT))
        .expect("capture closes");
    capture
        .close(&bounded(CONTENT_WAIT))
        .expect("repeated capture close is idempotent");
    let stopped = fixture
        .command(FixtureCommandKind::Stop, CONTENT_WAIT)
        .expect("owned fixture stop is acknowledged");
    assert_eq!(stopped.status, 0);
    let foreground_stopped = foreground_fixture
        .command(FixtureCommandKind::Stop, CONTENT_WAIT)
        .expect("the unrelated foreground fixture stop is acknowledged");
    assert_eq!(foreground_stopped.status, 0);
    fixture.input = None;
    foreground_fixture.input = None;
}

unsafe extern "C" {
    fn mp_shim_input_pointer_location(out_x: *mut f64, out_y: *mut f64) -> u32;
    fn getpeereid(socket: c_int, effective_user: *mut u32, effective_group: *mut u32) -> c_int;
    fn geteuid() -> u32;
    fn getsockopt(
        socket: c_int,
        level: c_int,
        option: c_int,
        value: *mut c_void,
        value_len: *mut u32,
    ) -> c_int;
}

#[link(name = "proc")]
unsafe extern "C" {
    fn proc_pidpath(process_id: c_int, buffer: *mut c_void, buffer_size: u32) -> c_int;
    fn proc_signal_with_audittoken(audit_token: *mut AuditToken, signal: c_int) -> c_int;
}
