#![cfg_attr(not(target_os = "macos"), allow(missing_docs))]
#![cfg(target_os = "macos")]
//! Native macOS input checks against a real desktop.
//!
//! # What runs by default and what does not
//!
//! macOS has no target-directed input channel, so there is no way to submit an
//! event to a fixture without focusing it and posting real system input. The
//! default suite therefore submits nothing: it exercises the read-only native
//! observations, the provider's input surface, and the refusals that happen
//! before any event. Successful submission is the explicit, user-focused check
//! at the bottom, which is ignored by default and documented in
//! `docs/macos-input-verification.md`.

use std::io::{BufRead, BufReader};
use std::panic::{self, AssertUnwindSafe};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use mado_pilot_capture::{
    CaptureProvider, FrameRequest, OpenRequest, PixelFormat, TargetDescription,
};
use mado_pilot_core::{
    CapabilitySupport, IdentityIssuer, InputDelivery, InputOperationKind, OperationContext,
    PermissionKind, PermissionProbe, PermissionState, Status, TargetId, TargetKind,
};
use mado_pilot_input::{
    DeliveryPlan, FocusPolicy, InputEvent, InputFault, InputOpenRequest, InputProvider,
    InputRequest, InputRequirement, InputSequence, Key, SequenceOutcome,
};
use mado_pilot_platform_macos::fixture_protocol::{
    EVENT_KEY_DOWN, EVENT_KEY_UP, EVENT_POINTER_MOVE, FixtureSelectionError, MAX_RECORDED_EVENTS,
    fixture_ready_context_is_approved, fixture_title, frame_is_fixture_content,
    frame_is_replacement_content, parse_event_line, select_unique_fixture,
    with_confirmed_fixture_content,
};
use mado_pilot_platform_macos::{MacosCaptureProvider, MacosPermissionProbe};

/// How long the interactive check waits for a person to focus the fixture.
const FOCUS_WAIT: Duration = Duration::from_secs(15);
/// How long the fail-closed content gate waits for one authoritative frame.
const CONTENT_WAIT: Duration = Duration::from_secs(5);
/// How long the fixture is given to publish its ready line.
const READY_WAIT: Duration = Duration::from_secs(10);
/// How long the owned-window oracle allows the successor and terminal loss.
const REPLACEMENT_WAIT: Duration = Duration::from_secs(10);

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

fn accessibility_granted() -> bool {
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

/// A fixture child process, killed when the guard is dropped.
struct FixtureChild(Child);

impl Drop for FixtureChild {
    fn drop(&mut self) {
        let _killed = self.0.kill();
        let _reaped = self.0.wait();
    }
}

/// A running fixture and its bounded output channel.
struct Fixture {
    _child: FixtureChild,
    lines: Receiver<String>,
    process_id: u32,
}

impl Fixture {
    /// Starts the ordinary fixture and waits for its ready record.
    fn start() -> Option<Self> {
        Self::start_with_arguments(&[])
    }

    /// Starts the fixture mode that destroys and replaces its own window.
    fn start_replacing() -> Option<Self> {
        Self::start_with_arguments(&["--replace-window-after-ready"])
    }

    fn start_with_arguments(arguments: &[&str]) -> Option<Self> {
        let executable = fixture_executable()?;
        let require_signed_bundle =
            std::env::var_os("MADO_PILOT_MACOS_FIXTURE_EXECUTABLE").is_some();
        let child = FixtureChild(
            Command::new(executable)
                .args(arguments)
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .ok()?,
        );
        Self::from_child(child, require_signed_bundle, READY_WAIT)
    }

    fn from_child(
        mut child: FixtureChild,
        require_signed_bundle: bool,
        ready_wait: Duration,
    ) -> Option<Self> {
        let stdout = child.0.stdout.take()?;
        let process_id = child.0.id();
        let lines = spawn_reader(stdout);

        let line = wait_for(&lines, ready_wait, |line| line.starts_with("fixture-ready"))?;
        println!("{line}");
        assert!(
            line.contains(&fixture_title(process_id)),
            "the fixture published a title this check did not expect: {line}"
        );
        if require_signed_bundle {
            assert!(
                fixture_ready_context_is_approved(&line, process_id),
                "a configured fixture must truthfully report the stable signed bundle \
                 context before any input path opens: {line}"
            );
        }
        Some(Self {
            _child: child,
            lines,
            process_id,
        })
    }

    fn replacement_result(&self, wait: Duration) -> Option<(u32, u64, u64)> {
        let line = wait_for(&self.lines, wait, |line| {
            line.starts_with("fixture-replaced ")
        })?;
        println!("{line}");
        parse_replacement_line(&line)
    }

    fn summaries(&self, wait: Duration) -> Vec<u32> {
        let mut kinds = Vec::new();
        let deadline = Instant::now() + wait;
        while Instant::now() < deadline && kinds.len() < MAX_RECORDED_EVENTS {
            match self.lines.recv_timeout(Duration::from_millis(100)) {
                Ok(line) => {
                    if let Some(summary) = parse_event_line(&line) {
                        kinds.push(summary.kind);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        kinds
    }
}

fn spawn_reader(stdout: ChildStdout) -> Receiver<String> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    receiver
}

#[test]
fn invalid_execution_context_output_reaps_the_owned_child() {
    let child = FixtureChild(
        Command::new("/bin/sh")
            .arg("-c")
            .arg(concat!(
                "printf 'fixture-ready title=MadoPilot Input Fixture [%s] pid=%s window=17 ",
                "launch=bundled signature=ad-hoc signing-identifier=wrong.identifier ",
                "bundle=dev.mado-pilot.macos-input-fixture capacity=256\\n' \"$$\" \"$$\"; ",
                "while :; do :; done"
            ))
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("the non-interactive child starts"),
    );
    let process_id = child.0.id();

    let rejected = panic::catch_unwind(AssertUnwindSafe(|| {
        Fixture::from_child(child, true, Duration::from_secs(2))
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

fn parse_replacement_line(line: &str) -> Option<(u32, u64, u64)> {
    if line.len() > 256 {
        return None;
    }
    let mut fields = line.strip_prefix("fixture-replaced ")?.split_whitespace();
    let status = fields.next()?.strip_prefix("status=")?.parse().ok()?;
    let old_window = fields.next()?.strip_prefix("old-window=")?.parse().ok()?;
    let new_window = fields.next()?.strip_prefix("new-window=")?.parse().ok()?;
    fields
        .next()
        .is_none()
        .then_some((status, old_window, new_window))
}

#[test]
fn replacement_record_is_bounded_and_structurally_exact() {
    let valid = "fixture-replaced status=0 old-window=17 new-window=18";
    assert_eq!(parse_replacement_line(valid), Some((0, 17, 18)));
    assert_eq!(
        parse_replacement_line("fixture-replaced old-window=17 status=0 new-window=18"),
        None
    );
    assert_eq!(
        parse_replacement_line("fixture-replaced status=0 old-window=17 new-window=18 extra=1"),
        None
    );
    assert_eq!(
        parse_replacement_line(&format!(
            "fixture-replaced status=0 old-window=17 new-window=18 {}",
            "x".repeat(256)
        )),
        None
    );
}

/// Locates the fixture beside the test binary that cargo just built.
fn fixture_executable() -> Option<std::path::PathBuf> {
    if let Some(configured) = std::env::var_os("MADO_PILOT_MACOS_FIXTURE_EXECUTABLE") {
        let executable = std::path::PathBuf::from(configured);
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
    let Some(fixture) = Fixture::start() else {
        println!("skipped: the fixture could not be started on this host");
        return;
    };
    let provider = provider();
    let chosen = discover_unique_fixture(&provider, fixture.process_id, CONTENT_WAIT)
        .expect("exactly one approved fixture becomes discoverable");

    assert_eq!(chosen.name(), fixture_title(fixture.process_id));
    assert_eq!(chosen.capability().kind(), Some(TargetKind::Window));
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
    let fixture =
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
        accessibility_granted(),
        "this check needs Accessibility granted to the test process; macOS discards \
         a synthesized event from an untrusted process without failing the post"
    );
    let fixture = Fixture::start().expect("the fixture starts on an interactive desktop");
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
