//! The native Windows `WindowMessage` workflow with unrelated foreground, end to end.
//!
//! Report what authorization this platform grants, discover targets, select
//! exactly one ordinary window by its full title, open capture and input
//! together, verify the source-frame condition, submit one bounded sequence
//! without touching focus, verify the expected condition on a strictly newer
//! frame, drain correlated diagnostics, and close.
//!
//! Start the ordinary evidence fixture, then pass its full title. The example
//! launches and monitors a second repository-owned fixture as the unrelated
//! foreground application:
//!
//! ```text
//! cargo run --locked --package mado-pilot-platform-windows --bin mado-pilot-windows-window-message-fixture -- --title-token=example
//! cargo run --locked --package mado-pilot --example windows-native-input -- "MadoPilot Ordinary WindowMessage Fixture [example]"
//! ```
//!
//! `cargo run` locates the foreground fixture beside the example's Cargo profile.
//! Set `MADO_PILOT_WINDOW_MESSAGE_FIXTURE` only when the fixture binary lives
//! elsewhere.
//!
//! # Why delivery does not disturb the owned foreground window
//!
//! Setup deliberately activates the unrelated child application before native
//! work begins and monitors it through input, visual observation, diagnostics,
//! and close. Delivery then requires exact-window `WindowMessage`, permits no
//! substitute, and preserves focus. An ordinary target advertises this route as
//! unknown-but-attemptable: successful submission proves target-queue admission,
//! not that the application consumed the legacy message or changed state.
//! `System` is never quietly substituted, so the input route does not activate
//! the target, move the real cursor, or type into the owned foreground.
//!
//! The full title is a required argument rather than a search heuristic:
//! nothing is guessed, a prefix is not accepted, and more than one match is
//! refused rather than resolved. The included ordinary fixture paints a
//! deterministic post-input fill so the example can demonstrate that queue
//! admission and a strictly newer visual observation are separate facts.
//!
//! Every prerequisite is checked before any event is sent, and a missing one
//! ends the program with an actionable message and a non-zero status.
//!
//! # What it prints
//!
//! Counts, identities, extents, and statuses. Never a window title, never a
//! captured pixel, never the characters that were typed.

#[cfg(windows)]
#[path = "support/native_observation.rs"]
mod native_observation;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    {
        windows::run()
    }
    #[cfg(not(windows))]
    {
        Err("this example demonstrates the Windows adapter and needs a Windows build".into())
    }
}

#[cfg(windows)]
mod windows {
    use super::native_observation;

    use std::io::{BufRead, BufReader};
    use std::path::PathBuf;
    use std::process::{Child, ChildStdout, Command, Stdio};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, mpsc};
    use std::thread;
    use std::time::{Duration, Instant};

    use mado_pilot::{
        CoordinateSpace, DeliveryPlan, DiagnosticOptions, Engine, Error, FocusPolicy, Frame,
        FrameRequest, InputDelivery, InputEvent, InputOpenRequest, InputOperationKind,
        InputReceipt, InputRequest, InputRequirement, InputSequence, Key, NativeEngineRequest,
        OpenRequest, OperationContext, PixelFormat, Point, PointerGeometry, SequenceOutcome,
        Session, SessionRequest, TargetDescription, TargetId, TargetKind,
    };
    use mado_pilot_platform_windows::fixture_protocol::{
        ORDINARY_CLASS_NAME, ordinary_fixture_title,
    };
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GetForegroundWindow, SetForegroundWindow,
    };
    use windows::core::PCWSTR;

    /// How long a discovery pass may take.
    ///
    /// Discovery enumerates top-level windows and displays, which is a real query
    /// against the window manager; a second is generous for it and short enough
    /// that an unresponsive host ends the program rather than hanging it.
    const DISCOVERY_BUDGET: Duration = Duration::from_secs(1);

    /// How long a child may take to publish its single readiness record.
    const FIXTURE_READY_BUDGET: Duration = Duration::from_secs(5);

    /// How long the whole input sequence may take.
    ///
    /// The sequence below has four logical events and three asynchronous native
    /// units; the delay is a logical event but posts no window message.
    const INPUT_BUDGET: Duration = Duration::from_secs(2);

    /// How long a strictly-newer frame may take to show the expected condition.
    const OBSERVATION_BUDGET: Duration = Duration::from_secs(5);

    /// Enough room for the complete example while keeping retention finite.
    const DIAGNOSTIC_CAPACITY: usize = 256;

    /// How long the two-sided close may take.
    const CLOSE_BUDGET: Duration = Duration::from_secs(2);

    /// The one mechanism this program permits.
    ///
    /// Required rather than preferred, and named once here so that no later edit
    /// can widen it by accident: a plan with a second mechanism in it would be a
    /// permission to fall back to system input, and this program has promised not
    /// to.
    const MECHANISM: InputDelivery = InputDelivery::WindowMessage;

    #[derive(Debug)]
    struct ChildGuard {
        child: Option<Child>,
        original: HWND,
        restore_on_drop: bool,
    }

    impl ChildGuard {
        fn new(child: Child, original: HWND) -> Self {
            Self {
                child: Some(child),
                original,
                restore_on_drop: true,
            }
        }

        fn child_mut(&mut self) -> &mut Child {
            self.child.as_mut().expect("foreground child remains owned")
        }

        fn transfer(mut self) -> (Child, HWND) {
            let child = self.child.take().expect("foreground child remains owned");
            self.restore_on_drop = false;
            (child, self.original)
        }
    }

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if let Some(child) = self.child.as_mut() {
                let _killed = child.kill();
                let _waited = child.wait();
            }
            if self.restore_on_drop && self.original != HWND::default() {
                // SAFETY: this is the opaque foreground handle observed before spawn.
                let _restored = unsafe { SetForegroundWindow(self.original) };
            }
        }
    }
    fn read_fixture_ready(output: ChildStdout) -> Result<String, Box<dyn std::error::Error>> {
        let (sender, receiver) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let mut ready = String::new();
            let result = BufReader::new(output).read_line(&mut ready).map(|_| ready);
            let _sent = sender.send(result);
        });
        match receiver.recv_timeout(FIXTURE_READY_BUDGET) {
            Ok(result) => Ok(result?),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                Err("the owned foreground fixture timed out before readiness".into())
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err("the owned foreground fixture readiness reader stopped".into())
            }
        }
    }

    #[derive(Debug)]
    struct OwnedForeground {
        child: Child,
        window: HWND,
        original: HWND,
        stop: Arc<AtomicBool>,
        violation: Arc<AtomicUsize>,
        monitor: Option<thread::JoinHandle<()>>,
    }

    impl OwnedForeground {
        fn establish() -> Result<Self, Box<dyn std::error::Error>> {
            // Capture this before `--activate` can replace it. The startup guard
            // attempts restoration on every later setup failure.
            // SAFETY: the return is an opaque handle and is not dereferenced.
            let original = unsafe { GetForegroundWindow() };
            let executable = foreground_fixture_executable()?;
            let token = format!("example-owned-foreground-{}", std::process::id());
            let mut child = ChildGuard::new(
                Command::new(executable)
                    .arg(format!("--title-token={token}"))
                    .arg("--activate")
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::inherit())
                    .spawn()?,
                original,
            );
            let title = ordinary_fixture_title(&token);
            let output = child
                .child_mut()
                .stdout
                .take()
                .ok_or("the owned foreground fixture exposes readiness output")?;
            let ready = read_fixture_ready(output)?;
            if !ready.starts_with("fixture-ready ")
                || !ready.contains(&format!(
                    "class={ORDINARY_CLASS_NAME} title={title} capacity="
                ))
            {
                return Err("the owned foreground fixture returned malformed readiness".into());
            }

            let class = wide(ORDINARY_CLASS_NAME);
            let title = wide(&title);
            // SAFETY: both strings are terminated and remain live for this lookup.
            let window = unsafe { FindWindowW(PCWSTR(class.as_ptr()), PCWSTR(title.as_ptr())) }
                .map_err(|_| "the owned foreground fixture window was not found")?;
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                // SAFETY: `window` is the live top-level window of the child this value owns.
                let _requested = unsafe { SetForegroundWindow(window) };
                // SAFETY: the return is an opaque handle and is not dereferenced.
                if unsafe { GetForegroundWindow() } == window {
                    break;
                }
                if child.child_mut().try_wait()?.is_some() {
                    return Err("the owned foreground fixture exited during activation".into());
                }
                if Instant::now() >= deadline {
                    return Err(
                        "Windows foreground-lock policy refused the owned fixture; rerun from \
                         the active terminal without interacting with another application"
                            .into(),
                    );
                }
                thread::sleep(Duration::from_millis(10));
            }

            let stop = Arc::new(AtomicBool::new(false));
            let violation = Arc::new(AtomicUsize::new(0));
            let ready = Arc::new(AtomicBool::new(false));
            let monitor_stop = Arc::clone(&stop);
            let monitor_violation = Arc::clone(&violation);
            let monitor_ready = Arc::clone(&ready);
            let expected = window.0.addr();
            let monitor = thread::spawn(move || {
                while !monitor_stop.load(Ordering::Acquire) {
                    // SAFETY: the return is an opaque handle and is not dereferenced.
                    if unsafe { GetForegroundWindow() }.0.addr() != expected {
                        monitor_violation.store(1, Ordering::Release);
                        return;
                    }
                    monitor_ready.store(true, Ordering::Release);
                    thread::sleep(Duration::from_millis(1));
                }
            });
            let (child, original) = child.transfer();
            let guard = Self {
                child,
                window,
                original,
                stop,
                violation,
                monitor: Some(monitor),
            };
            while !ready.load(Ordering::Acquire) {
                if guard.violation.load(Ordering::Acquire) != 0 {
                    return Err("the owned fixture lost foreground before input work began".into());
                }
                if Instant::now() >= deadline {
                    return Err("the owned foreground monitor did not become ready".into());
                }
                thread::yield_now();
            }
            println!("foreground: owned unrelated application is monitored");
            Ok(guard)
        }

        fn assert_stable(&self) -> Result<(), Box<dyn std::error::Error>> {
            if self.violation.load(Ordering::Acquire) != 0 {
                return Err(
                    "the owned unrelated application stopped being foreground during the workflow"
                        .into(),
                );
            }
            // SAFETY: the return is an opaque handle and is not dereferenced.
            if unsafe { GetForegroundWindow() } != self.window {
                return Err("the owned unrelated application is no longer foreground".into());
            }
            Ok(())
        }

        fn stop_monitor(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(monitor) = self.monitor.take() {
                let _joined = monitor.join();
            }
        }

        fn finish(mut self) -> Result<(), Box<dyn std::error::Error>> {
            let stable = self.assert_stable();
            let running = self.child.try_wait()?.is_none();
            self.stop_monitor();
            stable?;
            if !running {
                return Err("the owned unrelated foreground application exited early".into());
            }
            println!("foreground: unchanged through input, observation, diagnostics, and close");
            Ok(())
        }
    }

    impl Drop for OwnedForeground {
        fn drop(&mut self) {
            self.stop_monitor();
            let _killed = self.child.kill();
            let _waited = self.child.wait();
            if self.original != HWND::default() {
                // SAFETY: this is the opaque foreground handle observed before setup.
                let _restored = unsafe { SetForegroundWindow(self.original) };
            }
        }
    }

    fn foreground_fixture_executable() -> Result<PathBuf, Box<dyn std::error::Error>> {
        let executable = match std::env::var_os("MADO_PILOT_WINDOW_MESSAGE_FIXTURE") {
            Some(path) if !path.is_empty() => PathBuf::from(path),
            _ => {
                let current = std::env::current_exe()?;
                current
                    .parent()
                    .and_then(std::path::Path::parent)
                    .ok_or("the example executable has no Cargo profile directory")?
                    .join(format!(
                        "mado-pilot-windows-window-message-fixture{}",
                        std::env::consts::EXE_SUFFIX
                    ))
            }
        };
        if !executable.is_file() {
            return Err(format!(
                "{} does not exist; build the Windows window-message fixture first or set \
                 MADO_PILOT_WINDOW_MESSAGE_FIXTURE to its full path",
                executable.display()
            )
            .into());
        }
        Ok(executable)
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub(super) fn run() -> Result<(), Box<dyn std::error::Error>> {
        let title = match std::env::args().nth(1) {
            Some(title) if !title.trim().is_empty() => title,
            _ => {
                return Err(
                    "usage: windows-native-input \"<full window title>\" — the title is \
                            required and is matched exactly, because the events this sends are \
                            real input and a wrong target is somebody's application"
                        .into(),
                );
            }
        };
        let foreground = OwnedForeground::establish()?;

        // 1. Build the engine with a bounded debug stream. This touches no
        //    Windows API and asks for no authorization: an unusable OpenCV fails
        //    here, and nothing else is substituted for it.
        let engine = mado_pilot::windows_engine(
            NativeEngineRequest::new()
                .with_diagnostics(DiagnosticOptions::debug(DIAGNOSTIC_CAPACITY)?),
        )?;
        let diagnostics = engine
            .take_diagnostic_reader()
            .ok_or("an enabled engine exposes one diagnostic reader")?;
        println!("backend: {}", engine.backend());

        // 2. Say what this platform grants. Windows has no separate capture or
        //    input authorization this Adapter can read, so the engine reads none —
        //    which is a statement about the platform and not a claim that an
        //    operation will be permitted. Integrity level and interface privilege
        //    isolation are still enforced by the system, and a delivery they
        //    refuse is reported by the receipt.
        println!(
            "authorization: this build reads none ({})",
            engine.reads_permissions()
        );

        // 3. Discover, and select exactly one window by its full title. A prefix
        //    is not accepted and an ambiguous match is refused: this is the last
        //    point at which the wrong target can still be excluded for free.
        let targets = engine.discover(&bounded(DISCOVERY_BUDGET)?)?;
        println!("discovered: {} target(s)", targets.len());
        let target = select_unique_window(&targets, &title)?;
        println!(
            "selected: {} kind {:?} extent {}",
            target.id(),
            target.capability().kind(),
            target.extent()
        );

        // 4. Confirm the target advertises the mechanism this program permits,
        //    before a session exists. Ordinary windows expose it as
        //    unknown-but-attemptable; unsupported targets are refused while
        //    substitution is still free.
        require_window_message_pointer_and_keyboard(&engine, target.id())?;

        // 5. Open capture and input as one session. Input is required, so a
        //    capability that cannot be established fails the open and releases the
        //    capture committed for it rather than handing back a session that
        //    silently cannot type.
        let session = engine.open_session(
            target.id(),
            &SessionRequest::new()
                .capturing(OpenRequest::new())
                .requesting_input(
                    InputOpenRequest::new()
                        .with_requirement(InputRequirement::Required)
                        .requiring(InputOperationKind::Pointer, MECHANISM)
                        .requiring(InputOperationKind::Keyboard, MECHANISM),
                ),
            &bounded(DISCOVERY_BUDGET)?,
        )?;
        println!(
            "session: stream {:?} input available {}",
            session.stream(),
            session.accepts_input()
        );

        // Everything from here holds a session that has to be closed. Dropping
        // one does not close it, so the work is one fallible step and the close
        // is unconditional: an example that returned early through `?` would be
        // demonstrating a leak in a program whose whole job is to demonstrate
        // the right shape.
        let worked = deliver(&session);
        let closed = shut_down(&session);
        drop(session);
        drop(engine);
        let diagnostics_drained = native_observation::drain_diagnostics(&diagnostics);
        let foreground_held = foreground.finish();
        let receipt = worked?;
        closed?;
        diagnostics_drained?;
        foreground_held?;

        if receipt.outcome() == SequenceOutcome::Complete {
            Ok(())
        } else {
            Err(format!("the sequence did not complete: {receipt}").into())
        }
    }

    /// Steps 6 through 8: capture and check the source frame, submit input, then
    /// check the expected condition on a strictly newer frame.
    fn deliver(session: &Session) -> Result<InputReceipt, Box<dyn std::error::Error>> {
        // 6. Take one frame and map it. The pixels are never printed; what is
        //    reported is the identity that correlates this frame with anything
        //    searched in it, and with the input sent because of it.
        let frame = session.acquire_frame(&FrameRequest::latest(), &bounded(DISCOVERY_BUDGET)?)?;
        let stamp = frame.stamp();
        println!(
            "frame: epoch {} sequence {} geometry {} {}",
            stamp.epoch().value(),
            stamp.sequence().value(),
            stamp.geometry().value(),
            frame.descriptor()
        );
        println!(
            "frame accepts target-logical coordinates: {}",
            frame.transform().supports(CoordinateSpace::TargetLogical)
        );

        // The mapping is CPU bytes the caller owns, and it outlives the session.
        // Only its size and its source identity are reported: the bytes
        // themselves are the contents of somebody's window.
        let mapping = session.map_frame(&frame, PixelFormat::Bgra8, &bounded(DISCOVERY_BUDGET)?)?;
        println!(
            "mapping: {} byte(s) of {}, from frame sequence {}",
            mapping.bytes().len(),
            mapping.descriptor(),
            mapping.stamp().sequence().value()
        );
        if native_observation::expected_condition_matches(&mapping) {
            return Err(
                "the expected fixture condition is already present on the source frame".into(),
            );
        }

        // 7. Submit one bounded sequence, addressed to the exact frame above and
        //    without touching focus. The pointer position is expressed in that
        //    frame's own capture pixels, and `RequireUnchanged` binds it to that
        //    frame's identity: if the window moved or resized since it was
        //    captured, the coordinate no longer names what was captured, and the
        //    sequence is refused rather than submitted somewhere else. The
        //    delivery plan names exactly one mechanism, so nothing is
        //    substituted for it.
        let receipt = session.send_input(
            &InputRequest::new(
                session.target(),
                sequence(&frame)?,
                DeliveryPlan::require(MECHANISM),
            )
            .with_focus(FocusPolicy::Preserve)
            .with_pointer_geometry(PointerGeometry::require_unchanged_since(stamp)),
            &bounded(INPUT_BUDGET)?,
        )?;
        report_receipt(&receipt);
        if receipt.outcome() == SequenceOutcome::Complete {
            // 8. A complete receipt proves native submission only. Application
            //    effect is a separate visual fact from a strictly newer frame.
            native_observation::observe_expected_condition(session, stamp, OBSERVATION_BUDGET)?;
        }
        Ok(receipt)
    }

    /// Step 9: close both lifecycles, whatever the work above did.
    ///
    /// Idempotent, and retryable if the first close loses its own race.
    fn shut_down(session: &Session) -> Result<(), Box<dyn std::error::Error>> {
        if let Err(error) = session.close(&bounded(CLOSE_BUDGET)?) {
            println!("close did not finish: {} — retrying", error.status());
            session.close(&bounded(CLOSE_BUDGET)?)?;
        }
        println!("closed: {}", session.is_closed());
        Ok(())
    }

    /// Returns an operation bounded by `budget` and correlated to this run.
    fn bounded(budget: Duration) -> Result<OperationContext, Error> {
        native_observation::bounded(budget)
    }

    /// A move to the centre of `frame`, one keystroke, and a short delay so the
    /// receipt has something to bound.
    ///
    /// A move rather than a click: it is the least a pointer event can do, and it
    /// is enough to show a coordinate travelling from a captured frame to the
    /// input sent because of it. Nothing here presses a pointer button.
    fn sequence(frame: &Frame) -> Result<InputSequence, Error> {
        let extent = frame.descriptor().extent();
        let centre = Point::new(
            CoordinateSpace::CapturePixels,
            f64::from(extent.width()) / 2.0,
            f64::from(extent.height()) / 2.0,
        )?;

        Ok(InputSequence::new(vec![
            InputEvent::PointerMove(centre),
            InputEvent::KeyPress(Key::Character('m')),
            InputEvent::KeyRelease(Key::Character('m')),
            InputEvent::Delay(Duration::from_millis(20)),
        ])?)
    }

    /// Returns the one window whose full title is `title`, or why there is none.
    fn select_unique_window<'targets>(
        targets: &'targets [TargetDescription],
        title: &str,
    ) -> Result<&'targets TargetDescription, Box<dyn std::error::Error>> {
        let mut matched = targets.iter().filter(|target| {
            target.name() == title && target.capability().kind() == Some(TargetKind::Window)
        });
        let selected = matched
            .next()
            .ok_or("no discovered window carries exactly that title")?;
        if matched.next().is_some() {
            return Err(
                "more than one discovered window carries that title, so which one was \
                        meant cannot be established — nothing was sent"
                    .into(),
            );
        }
        Ok(selected)
    }

    fn require_window_message_pointer_and_keyboard(
        engine: &Engine,
        target: TargetId,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let descriptor = engine.describe_input(target, &bounded(DISCOVERY_BUDGET)?)?;
        let capability = descriptor.capability();
        for kind in [InputOperationKind::Pointer, InputOperationKind::Keyboard] {
            if !capability.pair(kind, MECHANISM).may_attempt() {
                return Err(format!(
                    "the selected window cannot attempt {kind} through the window-message route, \
                     and system input is not substituted for it"
                )
                .into());
            }
        }
        if !capability
            .pair(InputOperationKind::Pointer, MECHANISM)
            .accepts_pointer_space(CoordinateSpace::CapturePixels)
        {
            return Err(
                "the selected window does not accept coordinates in a frame's own capture pixels"
                    .into(),
            );
        }
        Ok(())
    }

    fn report_receipt(receipt: &InputReceipt) {
        // The receipt prints as counts and categories. Which characters were
        // typed is not among them, here or in the receipt itself.
        println!("receipt: {receipt}");
        println!(
            "  outcome {} submitted {} event(s), via {:?}, scope {:?}, evidence {:?}",
            receipt.outcome(),
            receipt.submitted(),
            receipt.selected_route(),
            receipt.address_scope(),
            receipt.evidence()
        );
        if receipt.used_fallback() {
            println!("  a permitted fallback was used");
        }
        if receipt.cleanup().may_leave_state_held() {
            println!(
                "  cleanup {} — {} of {} releases were made, so a key may still be held",
                receipt.cleanup(),
                receipt.cleanup_released(),
                receipt.cleanup_owed()
            );
        }
        if let Some(fault) = receipt.fault() {
            println!("  stopped because: {fault} ({})", fault.status());
        }
    }
}
