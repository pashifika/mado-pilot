#![cfg_attr(not(target_os = "macos"), allow(missing_docs))]
#![cfg(target_os = "macos")]
//! Native macOS input checks against a real desktop.
//!
//! # What runs by default and what does not
//!
//! macOS has no background input channel, so there is no way to deliver an event
//! to a fixture without focusing it and posting real system input. The default
//! suite therefore delivers nothing: it exercises the read-only native
//! observations, the provider's input surface, and the refusals that happen
//! before any event. Successful delivery is the explicit, user-focused check at
//! the bottom, which is ignored by default and documented in
//! `docs/macos-input-verification.md`.

use std::io::{BufRead, BufReader};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use mado_pilot_capture::{CaptureProvider, TargetDescription};
use mado_pilot_core::{
    IdentityIssuer, InputDelivery, InputOperationKind, OperationContext, PermissionKind,
    PermissionProbe, PermissionState, Status, TargetId, TargetKind,
};
use mado_pilot_input::{
    DeliveryPlan, FocusPolicy, InputEvent, InputFault, InputOpenRequest, InputProvider,
    InputRequest, InputRequirement, InputSequence, Key, SequenceOutcome,
};
use mado_pilot_platform_macos::fixture_protocol::{
    EVENT_KEY_DOWN, EVENT_KEY_UP, EVENT_POINTER_MOVE, MAX_RECORDED_EVENTS, fixture_title,
    parse_event_line, select_unique_fixture,
};
use mado_pilot_platform_macos::{MacosCaptureProvider, MacosPermissionProbe};

/// How long the interactive check waits for a person to focus the fixture.
const FOCUS_WAIT: Duration = Duration::from_secs(15);
/// How long the fixture is given to publish its ready line.
const READY_WAIT: Duration = Duration::from_secs(10);

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
        assert_eq!(
            input.permission(),
            Some(PermissionKind::InputControl),
            "every macOS target names Accessibility as the authorization input needs"
        );
        for kind in InputOperationKind::ALL {
            assert!(
                !input.supports(kind, InputDelivery::BackgroundTarget),
                "a discovered macOS target advertised background {}",
                kind.as_str()
            );
        }
        assert!(input.supports(InputOperationKind::Pointer, InputDelivery::System));
        let expects_keyboard = target.capability().kind() == Some(TargetKind::Window);
        assert_eq!(
            input.supports(InputOperationKind::Keyboard, InputDelivery::System),
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
fn an_open_that_requires_background_delivery_fails_without_establishing_anything() {
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
            .requiring(InputOperationKind::Pointer, InputDelivery::BackgroundTarget),
        &context(),
    )
    .expect_err("macOS implements no background delivery");

    assert_eq!(error.status(), Status::Unsupported);
}

#[test]
fn a_preserving_request_to_an_unfocused_window_delivers_nothing() {
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

    let error = controller
        .execute(&request, &context())
        .expect_err("preserve cannot satisfy a focus-requiring mechanism");

    assert_eq!(error.status(), Status::Unsupported);
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

    assert_eq!(receipt.delivered(), 0);
    assert_eq!(receipt.outcome(), SequenceOutcome::Unexecuted);
    let fault = receipt.failure().expect("a reason");
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
struct Fixture {
    child: Child,
    lines: Receiver<String>,
    process_id: u32,
}

impl Fixture {
    /// Starts the fixture and waits for the line it prints once its window is up.
    fn start() -> Option<Self> {
        let executable = fixture_executable()?;
        let mut child = Command::new(executable)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .ok()?;
        let stdout = child.stdout.take()?;
        let process_id = child.id();
        let lines = spawn_reader(stdout);

        let ready = wait_for(&lines, READY_WAIT, |line| line.starts_with("fixture-ready"));
        match ready {
            Some(line) => {
                println!("{line}");
                assert!(
                    line.contains(&fixture_title(process_id)),
                    "the fixture published a title this check did not expect: {line}"
                );
                Some(Self {
                    child,
                    lines,
                    process_id,
                })
            }
            None => {
                let _killed = child.kill();
                let _reaped = child.wait();
                None
            }
        }
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

impl Drop for Fixture {
    fn drop(&mut self) {
        let _killed = self.child.kill();
        let _reaped = self.child.wait();
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

/// Locates the fixture beside the test binary that cargo just built.
fn fixture_executable() -> Option<std::path::PathBuf> {
    let mut directory = std::env::current_exe().ok()?;
    directory.pop();
    if directory.ends_with("deps") {
        directory.pop();
    }
    let executable = directory.join("mado-pilot-macos-input-fixture");
    executable.is_file().then_some(executable)
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
    let Some(targets) = discovered(&provider) else {
        println!("skipped: this host offers no capture capability");
        return;
    };

    let chosen = select_unique_fixture(&targets, fixture.process_id)
        .expect("exactly one approved fixture is discoverable");

    assert_eq!(chosen.name(), fixture_title(fixture.process_id));
    assert_eq!(chosen.capability().kind(), Some(TargetKind::Window));
}

/// Delivers real system input to the fixture after a person focuses it.
///
/// Ignored by default. It moves the pointer, presses Enter, and types a fixed
/// string into whatever is frontmost, so it runs only on an interactive desktop
/// and only when the fixture is the frontmost window.
#[test]
#[ignore = "delivers real system input; run it deliberately on an interactive desktop"]
fn interactive_system_delivery_targets_only_the_exact_fixture() {
    assert!(
        accessibility_granted(),
        "this check needs Accessibility granted to the test process; macOS discards \
         a synthesized event from an untrusted process without failing the post"
    );
    let fixture = Fixture::start().expect("the fixture starts on an interactive desktop");
    let provider = provider();
    let targets = discovered(&provider).expect("this check needs Screen Recording granted");
    let chosen = select_unique_fixture(&targets, fixture.process_id)
        .expect("selection is fail-closed: zero or several matches stop here");

    println!(
        "Click the window titled `{}` within {} seconds.",
        fixture_title(fixture.process_id),
        FOCUS_WAIT.as_secs()
    );
    let controller = InputProvider::open(
        &provider,
        chosen.id(),
        &InputOpenRequest::new()
            .with_requirement(InputRequirement::Required)
            .requiring(InputOperationKind::Keyboard, InputDelivery::System),
        &context(),
    )
    .expect("input opens for the fixture");

    // `RequireFocused` never activates anything. Until a person focuses the
    // fixture, every attempt refuses and delivers nothing.
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
            receipt.delivered(),
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
    assert_eq!(receipt.delivered(), 4);
    assert_eq!(receipt.delivery(), Some(InputDelivery::System));

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
    assert!(controller.is_closed());
}
