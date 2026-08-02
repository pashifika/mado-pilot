//! The native Windows workflow, end to end, against one window the operator names.
//!
//! Report what authorization this platform grants, discover targets, select
//! exactly one window by its full title, open capture and input together, take
//! and map a frame, deliver one bounded sequence whose pointer position is bound
//! to that exact frame without touching focus, read the receipt, and close.
//!
//! ```text
//! cargo run --locked --package mado-pilot --example windows-native-input -- "<window title>"
//! ```
//!
//! # Why this one cannot disturb an ordinary window
//!
//! It requires background delivery and permits no substitute, and it preserves
//! focus. The Windows Adapter advertises background delivery for its own
//! dedicated fixture class alone, because arbitrary window classes share no
//! reliable background-input contract. An ordinary application window therefore
//! does not advertise it, the request is refused before any event exists, and
//! system input is never quietly put in its place — substituting it would focus a
//! window this program promised not to touch and type into whatever is focused
//! instead.
//!
//! The receiver this is meant for is `mado-pilot-windows-input-fixture` in
//! `mado-pilot-platform-windows`, whose window title is
//! `MadoPilot Input Fixture [<pid>]`. The title is a required argument rather
//! than a search: nothing is guessed, a prefix is not accepted, and more than one
//! match is refused rather than resolved.
//!
//! Every prerequisite is checked before any event is sent, and a missing one ends
//! the program with an actionable message and a non-zero status.
//!
//! # What it prints
//!
//! Counts, identities, extents, and statuses. Never a window title, never a
//! captured pixel, never the characters that were typed.

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
    use std::time::Duration;

    use mado_pilot::{
        CoordinateSpace, DeliveryPlan, Engine, Error, FocusPolicy, Frame, FrameRequest,
        InputDelivery, InputEvent, InputOpenRequest, InputOperationKind, InputRequest,
        InputRequirement, InputSequence, Key, NativeEngineRequest, OpenRequest, OperationContext,
        Point, PointerGeometry, SequenceOutcome, SessionRequest, TargetDescription, TargetId,
        TargetKind,
    };

    /// How long a discovery pass may take.
    ///
    /// Discovery enumerates top-level windows and displays, which is a real query
    /// against the window manager; a second is generous for it and short enough
    /// that an unresponsive host ends the program rather than hanging it.
    const DISCOVERY_BUDGET: Duration = Duration::from_secs(1);

    /// How long the whole input sequence may take.
    ///
    /// Each background event is one synchronous acknowledged packet, and the
    /// sequence below is five events with one short delay in it. Two seconds
    /// bounds a fixture that stops acknowledging.
    const INPUT_BUDGET: Duration = Duration::from_secs(2);

    /// How long the two-sided close may take.
    const CLOSE_BUDGET: Duration = Duration::from_secs(2);

    /// The one mechanism this program permits.
    ///
    /// Required rather than preferred, and named once here so that no later edit
    /// can widen it by accident: a plan with a second mechanism in it would be a
    /// permission to fall back to system input, and this program has promised not
    /// to.
    const MECHANISM: InputDelivery = InputDelivery::BackgroundTarget;

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

        // 1. Build the engine. This touches no Windows API and asks for no
        //    authorization: an unusable OpenCV fails here, and nothing else is
        //    substituted for it.
        let engine = mado_pilot::windows_engine(NativeEngineRequest::new())?;
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
        //    before a session exists. This is what excludes every window but the
        //    dedicated fixture class, and it happens while excluding one is still
        //    free.
        require_background_pointer_and_keyboard(&engine, target.id())?;

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
        let mapping = frame.map(frame.descriptor().format(), &bounded(DISCOVERY_BUDGET)?)?;
        println!(
            "mapping: {} byte(s) of {}, from frame sequence {}",
            mapping.bytes().len(),
            mapping.descriptor(),
            mapping.stamp().sequence().value()
        );

        // 7. Deliver one bounded sequence, addressed to the exact frame above and
        //    without touching focus. The pointer position is expressed in that
        //    frame's own capture pixels, and `RequireUnchanged` binds it to that
        //    frame's identity: if the window moved or resized since it was
        //    captured, the coordinate no longer names what was captured, and the
        //    sequence is refused rather than delivered somewhere else. The
        //    delivery plan names exactly one mechanism, so nothing is substituted
        //    for it.
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

        // 8. Close both lifecycles. Idempotent, and retryable if this one loses
        //    its own race.
        if let Err(error) = session.close(&bounded(CLOSE_BUDGET)?) {
            println!("close did not finish: {} — retrying", error.status());
            session.close(&bounded(CLOSE_BUDGET)?)?;
        }
        println!("closed: {}", session.is_closed());

        if receipt.outcome() == SequenceOutcome::Complete {
            Ok(())
        } else {
            Err(format!("the sequence did not complete: {receipt}").into())
        }
    }

    /// Returns an operation bounded by `budget`.
    fn bounded(budget: Duration) -> Result<OperationContext, Error> {
        OperationContext::new().with_timeout(budget)
    }

    /// A move to the centre of `frame`, two keystrokes, and a short delay so the
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
            InputEvent::KeyPress(Key::Character('p')),
            InputEvent::KeyRelease(Key::Character('p')),
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

    fn require_background_pointer_and_keyboard(
        engine: &Engine,
        target: TargetId,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let descriptor = engine.describe_input(target, &bounded(DISCOVERY_BUDGET)?)?;
        let capability = descriptor.capability();
        for kind in [InputOperationKind::Pointer, InputOperationKind::Keyboard] {
            if !capability.supports(kind, MECHANISM) {
                return Err(format!(
                    "the selected window does not accept background {kind} delivery, and system \
                     input is not substituted for it — point this at the dedicated input fixture"
                )
                .into());
            }
        }
        if !capability.accepts_pointer_space(CoordinateSpace::CapturePixels) {
            return Err(
                "the selected window does not accept coordinates in a frame's own capture pixels"
                    .into(),
            );
        }
        Ok(())
    }

    fn report_receipt(receipt: &mado_pilot::InputReceipt) {
        // The receipt prints as counts and categories. Which characters were
        // typed is not among them, here or in the receipt itself.
        println!("receipt: {receipt}");
        println!(
            "  outcome {} delivered {} of the sequence, via {:?}",
            receipt.outcome(),
            receipt.delivered(),
            receipt.delivery()
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
        if let Some(failure) = receipt.failure() {
            println!("  stopped because: {failure} ({})", failure.status());
        }
    }
}
