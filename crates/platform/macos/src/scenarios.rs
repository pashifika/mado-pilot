//! Controlled adapter scenarios against real ScreenCaptureKit targets.
//!
//! These live inside the package rather than in `tests/` because the containment
//! and failure-path ownership cases ADR 0012 requires are reached through the
//! session-scoped raise sites, which are not part of any public surface.
//!
//! # What a skip means
//!
//! Every scenario needs a host that offers the capture framework *and* has granted
//! Screen Recording to the process running the tests. A continuous-integration
//! runner has neither granted nor denied it, and this Adapter will not prompt, so
//! these scenarios report a skip there instead of a pass. The skip is printed with
//! its reason so a green run cannot be read as evidence the scenario ran.

use std::ffi::c_void;
use std::sync::{Arc, Barrier, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use mado_pilot_capture::{
    CaptureFault, CaptureSession, CoordinateSupport, Frame, FrameRequest, Lifecycle, PixelFormat,
    RetainedStoragePolicy,
};
use mado_pilot_core::{
    CancellationToken, Clock, CoordinateSpace, IdentityIssuer, Operation, OperationContext,
    PixelExtent, Point, Status, SystemClock, TargetKind,
};

use crate::availability::ensure_capture_available;
use crate::discovery::{Candidate, Fingerprint, NativeKey, TargetMetadata, inventory};
use crate::input::GeometryLedger;
use crate::native::{NativeSession, SessionTarget, testing_delayed_callback_is_active};
use crate::shim::{
    self, DELAY_IN_RUST_CALLBACK, FAIL_RECONFIGURE_SEMAPHORE_ALLOCATION,
    FAIL_START_HOLD_ALLOCATION, MAX_NATIVE_WAIT, PANIC_IN_RUST_CALLBACK, RAISE_AFTER_CALLBACK,
    RAISE_AT_START, RAISE_AT_START_SUBMISSION, RAISE_AT_TEARDOWN, RAISE_BEFORE_CALLBACK,
    RAISE_IN_START_COMPLETION, RAISE_IN_STOP_COMPLETION,
};
use crate::storage::DETACHED_BUFFER_BUDGET;

/// How long a scenario waits for the producer to publish something.
const FRAME_WAIT: Duration = Duration::from_secs(5);

/// How long a scenario collecting a run of frames waits for each one.
const COLLECT_WAIT: Duration = Duration::from_millis(500);

/// How long one window candidate is given to publish before the next is tried.
///
/// A producing window delivers its first frame within a stream start and a refresh
/// interval, well inside this. The budget is deliberately much shorter than
/// [`FRAME_WAIT`] because a scenario may work through several idle candidates before
/// it finds a live one, and the full budget each time would dominate the suite.
const LIVENESS_WAIT: Duration = Duration::from_secs(1);

/// How far ahead of its delivery a frame's own time may legitimately sit.
///
/// The producer reports a frame's *display* time, so a frame handed over before the
/// refresh it was scheduled for carries a time slightly in the future. Measured on
/// the verification host it is usually 1 to 11 ms behind delivery and occasionally
/// ahead of it, by up to 3 ms. The bound is a whole refresh interval with margin,
/// which still fails by orders of magnitude on a timestamp converted at the wrong
/// scale.
const FRAME_TIME_LEAD: Duration = Duration::from_millis(50);

/// Runs the scenarios one at a time, for two reasons rather than one.
///
/// The ownership cases compare the shim's process-wide count of the native objects
/// it owns, and that count only means something in a quiet process. And every
/// scenario captures the same display, so running them together would have them
/// competing for the producer they are measuring. A poisoned gate is taken anyway:
/// one scenario's failure should report itself rather than turn every later
/// scenario into a second, less informative failure.
fn serialized() -> MutexGuard<'static, ()> {
    shim::NATIVE_LIFECYCLE_TEST_SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// One authorized target, with everything a session open needs.
struct Harness {
    issuer: Arc<IdentityIssuer>,
    key: NativeKey,
    fingerprint: Fingerprint,
    target: shim::TargetToken,
    metadata: TargetMetadata,
}

/// Names why a scenario cannot run on this host, and returns `None` so it does not.
///
/// Every skip is routed through this one place so that a build which cannot tolerate
/// one is able to refuse it. The sanitizer build is that build: it reports a freed
/// access performed *during* a live capture, so a run whose scenarios never opened a
/// stream reports nothing at all — and a skip and a pass are the same line of output.
/// A green sanitizer run has to mean the capture ran, so there it fails instead.
/// This is the rule the replay symlink tests already follow: a host that cannot run
/// the case has proven nothing. `CONTRIBUTING.md` step 10 is the command.
fn skipped<T>(scenario: &str, reason: &str) -> Option<T> {
    if cfg!(mado_pilot_asan) {
        panic!("the sanitizer run needs {scenario} to capture, and this host cannot: {reason}");
    }
    println!("skipped {scenario}: {reason}");
    None
}

/// Returns every currently shareable target, or `None` when this host cannot run
/// the scenarios.
fn discovered(scenario: &str) -> Option<Vec<Candidate>> {
    if let Err(error) = ensure_capture_available() {
        return skipped(
            scenario,
            &format!("the capture framework is unavailable ({error})"),
        );
    }
    match inventory(MAX_NATIVE_WAIT) {
        Ok(candidates) => Some(candidates),
        Err(error) => skipped(
            scenario,
            &format!(
                "discovery reported {error}; Screen Recording is not granted to the \
                 test process"
            ),
        ),
    }
}

impl Harness {
    /// Returns a harness for a display, or `None` when this host cannot run the
    /// scenarios.
    ///
    /// A display is the default target because its lifetime and extent do not
    /// depend on what the user happens to be doing while the tests run.
    fn acquire(scenario: &str) -> Option<Self> {
        Self::acquire_kind(scenario, TargetKind::Display)
    }

    fn acquire_kind(scenario: &str, kind: TargetKind) -> Option<Self> {
        let candidates = discovered(scenario)?;
        let Some(chosen) = candidates
            .into_iter()
            .find(|candidate| candidate.key.kind() == kind)
        else {
            return skipped(scenario, &format!("this host reports no {kind} target"));
        };
        Some(Self::from_candidate(&chosen))
    }

    fn from_candidate(candidate: &Candidate) -> Self {
        Self {
            issuer: Arc::new(IdentityIssuer::new()),
            key: candidate.key,
            fingerprint: candidate.fingerprint,
            target: candidate.target.clone(),
            metadata: candidate.metadata.clone(),
        }
    }

    /// Returns a harness for a display that is currently producing frames.
    ///
    /// The framework publishes when content changes and not otherwise, so a
    /// display nobody is touching produces its first frame and then nothing. A
    /// scenario whose subject is *what happens as frames arrive* would pass
    /// vacuously against such a display, so liveness is established here — by
    /// opening a throwaway session and requiring it to advance — rather than
    /// assumed. `None` with a printed note means no attached display is changing.
    fn acquire_producing(scenario: &str) -> Option<Self> {
        let candidates = discovered(scenario)?;
        let displays = candidates
            .iter()
            .filter(|candidate| candidate.key.kind() == TargetKind::Display);
        for display in displays {
            let harness = Self::from_candidate(display);
            let Ok(session) = harness.open(0) else {
                continue;
            };
            let advanced = next_frame(&session, FrameRequest::latest())
                .and_then(|first| next_frame(&session, FrameRequest::newer_than(first.stamp())))
                .is_ok();
            let _closed = close(&session);
            drop(session);
            if advanced {
                return Some(harness);
            }
        }
        skipped(
            scenario,
            "no attached display is currently producing frames, so nothing would be \
             exercised",
        )
    }

    fn open(&self, raise_sites: u32) -> mado_pilot_core::Result<Arc<NativeSession>> {
        let context = OperationContext::new()
            .with_timeout(Duration::from_secs(10))
            .expect("a positive timeout");
        let mut operation = Operation::admit(&context).expect("admitted");
        let target = self.issuer.issue_target(crate::provider::PROVIDER)?;
        let stream = self.issuer.issue_stream()?;
        let selected = SessionTarget::new(
            target,
            stream,
            self.key,
            self.fingerprint,
            self.target.clone(),
            self.metadata.clone(),
            Arc::new(GeometryLedger::default()),
        );
        NativeSession::open_with_raise_sites(selected, raise_sites, &mut operation)
    }

    fn open_with_delays(
        &self,
        start_delay: Duration,
        stop_delay: Duration,
    ) -> mado_pilot_core::Result<Arc<NativeSession>> {
        let context = OperationContext::new()
            .with_timeout(Duration::from_secs(10))
            .expect("a positive timeout");
        self.open_with_delays_in(start_delay, stop_delay, &context)
    }

    fn open_with_delays_in(
        &self,
        start_delay: Duration,
        stop_delay: Duration,
        context: &OperationContext,
    ) -> mado_pilot_core::Result<Arc<NativeSession>> {
        let mut operation = Operation::admit(context).expect("admitted");
        let target = self.issuer.issue_target(crate::provider::PROVIDER)?;
        let stream = self.issuer.issue_stream()?;
        let selected = SessionTarget::new(
            target,
            stream,
            self.key,
            self.fingerprint,
            self.target.clone(),
            self.metadata.clone(),
            Arc::new(GeometryLedger::default()),
        );
        NativeSession::open_with_delays(selected, start_delay, stop_delay, &mut operation)
    }

    fn open_unstarted_shim(
        &self,
        start_delay: Duration,
        stop_delay: Duration,
        failure_sites: u32,
    ) -> Result<shim::Session, shim::ShimStatus> {
        unsafe extern "C" fn ignore_frame(
            _context: *mut c_void,
            _frame: *mut shim::OpaqueFrameHandle,
            _info: *const shim::FrameInfo,
        ) -> u32 {
            shim::ShimStatus::Ok.as_raw()
        }
        unsafe extern "C" fn ignore_stopped(_context: *mut c_void, _status: u32) {}
        unsafe extern "C" fn ignore_frame_commit(_context: *mut c_void) -> u32 {
            shim::ShimStatus::Ok.as_raw()
        }

        shim::Session::open(
            &shim::OpenRequest {
                kind: self.key.native_kind(),
                native_id: self.key.native_id(),
                owner_process: self.fingerprint.native_owner(),
                target: self.target.clone(),
                extent: self.metadata.extent,
                queue_depth: 3,
                detached_budget: DETACHED_BUFFER_BUDGET.get(),
                testing_start_delay: start_delay,
                testing_stop_delay: stop_delay,
                testing_raise_sites: failure_sites,
            },
            std::ptr::null_mut(),
            ignore_frame,
            ignore_frame_commit,
            ignore_stopped,
        )
    }
}

/// Waits for a frame the session publishes, or reports why it never arrived.
fn next_frame(session: &NativeSession, request: FrameRequest) -> mado_pilot_core::Result<Frame> {
    next_frame_within(session, request, FRAME_WAIT)
}

/// Waits `wait` for a frame rather than the default.
///
/// A scenario that has to collect several frames uses a short budget per frame, so
/// the whole sequence finishes while the display is still changing. Waiting the
/// full budget each time would spend half a minute on a busy display and finish
/// against a quiet one, which is how a scenario ends up unable to observe the thing
/// it is about.
fn next_frame_within(
    session: &NativeSession,
    request: FrameRequest,
    wait: Duration,
) -> mado_pilot_core::Result<Frame> {
    let context = OperationContext::new()
        .with_timeout(wait)
        .expect("a positive timeout");
    session.frame(&request, &context)
}

/// Returns the first window matching `wanted` that actually publishes, with the frame
/// it published and the session that published it.
///
/// The framework publishes on content change, so a window nobody is touching delivers
/// nothing — not even a first frame, which is where a window differs from a display.
/// A scenario whose subject is a *published frame* therefore has to establish that its
/// candidate produces one, and move on to the next candidate when it does not. Taking
/// only the first match turned an idle window into a failure of a property the run
/// never got to test, which is how this scenario failed on a verification host whose
/// left-hand display happened to hold nothing that was redrawing.
///
/// Only two outcomes move on to the next candidate: an expired budget, which is what
/// idleness looks like, and a target lost mid-probe, which is the race the Adapter is
/// required to report. Every other refusal fails the scenario rather than being
/// absorbed into a skip — a skip has to name its own reason, and one that blamed an
/// idle desktop for a revoked authorization would name the wrong one.
fn producing_window(
    scenario: &str,
    candidates: &[Candidate],
    wanted: impl Fn(&Candidate) -> bool,
) -> Option<(Harness, Arc<NativeSession>, Frame)> {
    let mut considered = 0u32;
    for candidate in candidates
        .iter()
        .filter(|candidate| candidate.key.kind() == TargetKind::Window && wanted(candidate))
    {
        considered += 1;
        let harness = Harness::from_candidate(candidate);
        let session = match harness.open(0) {
            Ok(session) => session,
            // A window discovered a moment ago may have closed or resized since, and
            // the Adapter is required to say so rather than capture something else.
            Err(error) if error.status() == Status::TargetLost => continue,
            Err(error) => panic!("a discovered window failed to open: {error}"),
        };
        match next_frame_within(&session, FrameRequest::latest(), LIVENESS_WAIT) {
            Ok(frame) => return Some((harness, session, frame)),
            Err(error)
                if error.status() == Status::DeadlineExceeded
                    || error.status() == Status::TargetLost =>
            {
                let _closed = close(&session);
            }
            Err(error) => panic!("a window session failed rather than going quiet: {error}"),
        }
    }
    if considered == 0 {
        println!("noted {scenario}: this host reports no window matching the scenario");
    } else {
        println!(
            "noted {scenario}: none of the {considered} matching window(s) published \
             within the budget, so every one of them is idle"
        );
    }
    None
}

fn close(session: &NativeSession) -> mado_pilot_core::Result<()> {
    let context = OperationContext::new()
        .with_timeout(Duration::from_secs(10))
        .expect("a positive timeout");
    session.close(&context)
}

/// Proves that a later discovery cannot terminate an already-open retained filter.
#[test]
fn an_open_selection_keeps_producing_across_a_fresh_discovery_snapshot() {
    let _serial = serialized();
    let Some(harness) = Harness::acquire_producing("selection across discovery refresh") else {
        return;
    };
    let session = harness.open(0).expect("retained selection opens");
    let first = next_frame(&session, FrameRequest::latest()).expect("first frame");
    let Some(_fresh) = discovered("selection across discovery refresh") else {
        close(&session).expect("close after unavailable refresh");
        return;
    };
    match next_frame_within(
        &session,
        FrameRequest::newer_than(first.stamp()),
        FRAME_WAIT,
    ) {
        Ok(_) => {}
        Err(error) if error.status() == Status::DeadlineExceeded => {
            // The content stopped changing after the fresh snapshot. The session
            // remains live, which the lifecycle assertion below covers; what
            // cannot be shown from here is advancement with nothing to advance to.
            println!(
                "noted: the display went idle after the fresh discovery snapshot, \
                 so continued publication is not exercised"
            );
        }
        Err(error) => panic!("the retained filter failed after fresh discovery: {error}"),
    }
    assert_eq!(
        session.lifecycle(),
        Lifecycle::Open,
        "a fresh snapshot must not terminate the retained filter"
    );
    close(&session).expect("close");
}

#[test]
fn a_display_session_publishes_frames_that_cover_the_display() {
    let _serial = serialized();
    let Some(harness) = Harness::acquire("display publication") else {
        return;
    };
    let session = harness.open(0).expect("a discovered display opens");
    let description = session.description();
    assert_eq!(
        description.coordinates(),
        CoordinateSupport::with_target_placement()
    );
    assert_eq!(
        description.queue().retained_storage(),
        Some(DETACHED_BUFFER_BUDGET),
        "the finite detached budget is what the caller is told about"
    );
    assert_eq!(
        description.queue().retained_storage_policy(),
        Some(RetainedStoragePolicy::Guaranteed),
        "the macOS detached pool is isolated per session"
    );

    let frame = next_frame(&session, FrameRequest::latest()).expect("the display produces frames");

    assert_eq!(frame.descriptor().extent(), harness.metadata.extent);
    assert!(frame.transform().covers_target());
    let placement = frame
        .transform()
        .target()
        .expect("the qualified host publishes frame-attached screenRect placement");
    let origin = frame
        .transform()
        .convert_point(
            Point::new(CoordinateSpace::CapturePixels, 0.0, 0.0).expect("valid"),
            CoordinateSpace::DesktopLogical,
        )
        .expect("same-frame desktop conversion");
    assert_eq!((origin.x(), origin.y()), placement.desktop_origin());
    close(&session).expect("close");
}

#[test]
fn successive_frames_carry_advancing_times_in_the_engine_clock_domain() {
    let _serial = serialized();
    let Some(harness) = Harness::acquire_producing("frame time domain") else {
        return;
    };
    let session = harness.open(0).expect("a discovered display opens");

    // The seed establishes a stamp to ask past and is not itself compared: the
    // latest frame may have been published during the open, so nothing is known
    // about when it was produced relative to this run. Every frame after it is a
    // distinct publication this run watched arrive, which is what makes both the
    // ordering and the elapsed comparisons below sound.
    let seed = next_frame(&session, FrameRequest::latest()).expect("the display publishes");
    let mut stamp = seed.stamp();
    let started = SystemClock.now();
    let mut times = Vec::new();
    for _ in 0..5 {
        match next_frame_within(&session, FrameRequest::newer_than(stamp), COLLECT_WAIT) {
            Ok(frame) => {
                stamp = frame.stamp();
                times.push(frame.captured_at());
            }
            Err(error) if error.status() == Status::CaptureFailed => {
                panic!("the producer failed while frame times were collected: {error}");
            }
            // The display stopped changing. What was collected is still asserted.
            Err(_) => break,
        }
    }
    let finished = SystemClock.now();
    close(&session).expect("close");

    if times.len() < 2 {
        println!(
            "noted: the chosen display published once and then went idle, so \
             advancing frame time is not exercised"
        );
        return;
    }

    // The producer reports a frame's time on the host's own clock in units that are
    // not the engine's, so converting it into this domain is the Adapter's. A
    // conversion that dropped the unit put every frame a host uptime before the
    // stream's own anchor and collapsed them all onto the domain origin — which no
    // scenario noticed, because none of them read a frame's time.
    for pair in times.windows(2) {
        assert!(
            pair[1] > pair[0],
            "a later publication did not carry a later time: {:?} then {:?}",
            pair[0],
            pair[1]
        );
    }
    let last = *times.last().expect("the run holds at least two frames");
    let furthest_ahead = finished
        .checked_add(FRAME_TIME_LEAD)
        .expect("a representable instant");
    assert!(
        last <= furthest_ahead,
        "a frame was timed {:?} past the clock read that followed receiving it, \
         further ahead than a producer scheduling one refresh ahead explains",
        last.saturating_duration_since(finished)
    );
    // Each of these frames was published after the run started, so the span between
    // the first and the last cannot exceed the run's own wall time by more than the
    // budget the last frame was waited for. This is the half that fails when a
    // conversion runs the clock at the wrong rate rather than losing its origin.
    let span = last.saturating_duration_since(times[0]);
    let wall = finished.saturating_duration_since(started) + COLLECT_WAIT;
    assert!(
        span <= wall,
        "frame times spanned {span:?} across a run that took at most {wall:?}"
    );
}

#[test]
fn a_window_session_publishes_same_frame_desktop_placement() {
    let _serial = serialized();
    let Some(candidates) = discovered("window publication") else {
        return;
    };
    let Some((_harness, session, frame)) =
        producing_window("window publication", &candidates, |_| true)
    else {
        return;
    };

    assert_eq!(
        frame.transform().frame_extent(),
        frame.descriptor().extent()
    );
    assert!(frame.transform().covers_target());
    let placement = frame
        .transform()
        .target()
        .expect("a qualified-host window frame carries screenRect placement");
    let origin = frame
        .transform()
        .convert_point(
            Point::new(CoordinateSpace::CapturePixels, 0.0, 0.0).expect("valid"),
            CoordinateSpace::DesktopLogical,
        )
        .expect("desktop conversion");
    assert_eq!((origin.x(), origin.y()), placement.desktop_origin());

    let mapping = frame
        .map(PixelFormat::Bgra8, &OperationContext::new())
        .expect("a window frame maps");
    assert_eq!(mapping.bytes().len(), frame.descriptor().byte_len());
    close(&session).expect("close");
}

#[test]
fn every_attached_display_carries_same_frame_desktop_conversion() {
    let _serial = serialized();
    let Some(candidates) = discovered("multi-display placement") else {
        return;
    };
    let displays: Vec<Candidate> = candidates
        .into_iter()
        .filter(|candidate| candidate.key.kind() == TargetKind::Display)
        .collect();
    if displays.len() < 2 {
        println!(
            "noted: {} display(s) attached, so cross-display placement is not exercised",
            displays.len()
        );
    }

    for display in &displays {
        let harness = Harness::from_candidate(display);
        let session = harness.open(0).expect("a discovered display opens");
        let frame = next_frame(&session, FrameRequest::latest()).expect("the display publishes");
        assert_eq!(frame.descriptor().extent(), display.metadata.extent);
        let placement = frame
            .transform()
            .target()
            .expect("each display frame carries screenRect placement");
        assert_eq!(
            placement.desktop_origin(),
            display.metadata.placement.desktop_origin(),
            "screenRect and shareable-content use different desktop origins"
        );
        assert_eq!(
            placement.logical_size(),
            display.metadata.placement.logical_size(),
            "screenRect and shareable-content use different logical units"
        );
        assert_eq!(
            placement.scale().x(),
            display.metadata.placement.scale().x(),
            "screenRect/contentRect scaling disagrees with display backing scale"
        );
        let origin = frame
            .transform()
            .convert_point(
                Point::new(CoordinateSpace::CapturePixels, 0.0, 0.0).expect("valid"),
                CoordinateSpace::DesktopLogical,
            )
            .expect("desktop conversion");
        assert_eq!((origin.x(), origin.y()), placement.desktop_origin());
        close(&session).expect("close");
    }
}

#[test]
fn mixed_scale_displays_publish_their_own_frame_geometry() {
    let _serial = serialized();
    let Some(candidates) = discovered("mixed-scale seam through frames") else {
        return;
    };
    let mut displays: Vec<Candidate> = candidates
        .into_iter()
        .filter(|candidate| candidate.key.kind() == TargetKind::Display)
        .collect();
    displays.sort_by(|left, right| {
        left.metadata
            .placement
            .desktop_origin()
            .0
            .total_cmp(&right.metadata.placement.desktop_origin().0)
    });

    // The pair this scenario is about: horizontally adjacent, and disagreeing about
    // how many capture pixels a point is worth.
    let Some(index) = displays.windows(2).position(|pair| {
        let left = pair[0].metadata.placement;
        let right = pair[1].metadata.placement;
        let left_far = left.desktop_origin().0 + left.logical_size().0;
        left_far == right.desktop_origin().0 && left.scale().x() != right.scale().x()
    }) else {
        println!(
            "noted: no two horizontally adjacent displays differ in scale, \
             so the mixed-scale seam is not exercised through frames"
        );
        return;
    };

    for display in &displays[index..=index + 1] {
        let harness = Harness::from_candidate(display);
        let session = harness.open(0).expect("a discovered display opens");
        let frame = next_frame(&session, FrameRequest::latest()).expect("the display publishes");
        let placement = frame
            .transform()
            .target()
            .expect("mixed-scale frame carries screenRect placement");
        assert_eq!(
            placement.desktop_origin(),
            display.metadata.placement.desktop_origin()
        );
        assert_eq!(
            placement.scale().x(),
            display.metadata.placement.scale().x()
        );
        close(&session).expect("close");
    }
}

#[test]
fn horizontally_adjacent_displays_share_one_desktop_seam() {
    let _serial = serialized();
    let Some(candidates) = discovered("multi-display seam") else {
        return;
    };
    let mut spans: Vec<(f64, f64, f64)> = candidates
        .iter()
        .filter(|candidate| candidate.key.kind() == TargetKind::Display)
        .map(|display| {
            let placement = display.metadata.placement;
            let (x, _y) = placement.desktop_origin();
            let (width, _height) = placement.logical_size();
            (x, x + width, placement.scale().x())
        })
        .collect();
    if spans.len() < 2 {
        println!("noted: fewer than two displays attached, so no seam exists to check");
        return;
    }
    spans.sort_by(|left, right| left.0.total_cmp(&right.0));

    let mut seams = 0;
    for window in spans.windows(2) {
        let (_, left_far, left_scale) = window[0];
        let (right_near, _, right_scale) = window[1];
        if left_far != right_near {
            // Displays may be stacked or offset rather than tiled, which is a
            // legitimate arrangement and not a seam to assert.
            continue;
        }
        seams += 1;
        // The seam is one coordinate in the shared point plane whatever the two
        // scales are. That is the property macOS gives and Windows does not, and
        // it is what lets a caller reason across displays in one space.
        assert_eq!(
            left_far, right_near,
            "adjacent displays disagree about the coordinate between them"
        );
        if left_scale != right_scale {
            println!(
                "noted: seam at {left_far} joins displays of scale {left_scale} and {right_scale}"
            );
        }
    }
    if seams == 0 {
        println!("noted: no two displays are horizontally adjacent in this arrangement");
    }

    let scales: Vec<f64> = spans.iter().map(|(_, _, scale)| *scale).collect();
    if scales.windows(2).all(|pair| pair[0] == pair[1]) {
        println!(
            "noted: every attached display reports scale {}, \
             so a seam between differing scales is not exercised on this host",
            scales[0]
        );
    }
}

#[test]
fn a_window_left_of_the_main_display_preserves_its_signed_frame_coordinates() {
    let _serial = serialized();
    let Some(candidates) = discovered("signed window placement") else {
        return;
    };
    let Some((_harness, session, frame)) =
        producing_window("signed window placement", &candidates, |candidate| {
            let (x, y) = candidate.metadata.placement.desktop_origin();
            x < 0.0 || y < 0.0
        })
    else {
        return;
    };
    let placement = frame
        .transform()
        .target()
        .expect("screenRect is attached to this signed-origin frame");
    let origin = frame
        .transform()
        .convert_point(
            Point::new(CoordinateSpace::CapturePixels, 0.0, 0.0).expect("valid"),
            CoordinateSpace::DesktopLogical,
        )
        .expect("signed desktop conversion");
    assert_eq!((origin.x(), origin.y()), placement.desktop_origin());
    assert!(origin.x() < 0.0 || origin.y() < 0.0);
    close(&session).expect("close");
}

#[test]
fn a_retina_display_reports_more_than_one_capture_pixel_per_point() {
    let _serial = serialized();
    let Some(harness) = Harness::acquire("retina scale") else {
        return;
    };
    let scale = harness.metadata.placement.scale().x();
    let (logical_width, _logical_height) = harness.metadata.placement.logical_size();

    assert!(scale > 0.0);
    assert_eq!(
        f64::from(harness.metadata.extent.width()),
        (logical_width * scale).round(),
        "the captured extent is the logical width at the display's backing scale"
    );
    if scale <= 1.0 {
        println!("noted: the current display is not a Retina display, so scale is {scale}");
    }
}

#[test]
fn retained_frames_never_stall_the_producer() {
    let _serial = serialized();
    let Some(harness) = Harness::acquire_producing("retained producer progress") else {
        return;
    };
    let session = harness.open(0).expect("open");

    let first = next_frame(&session, FrameRequest::latest()).expect("first frame");
    let mut retained = vec![first.clone()];
    let mut stamp = first.stamp();

    // The framework publishes a frame when the content changes and not otherwise,
    // so a target nobody is touching legitimately produces nothing. Establish that
    // this one is producing before asserting anything about not stalling it —
    // otherwise a static display would fail a scenario it never exercised.
    match next_frame(&session, FrameRequest::newer_than(stamp)) {
        Ok(frame) => {
            stamp = frame.stamp();
            retained.push(frame);
        }
        Err(error) if error.status() == Status::CaptureFailed => {
            panic!("the producer failed before the retention phase: {error}");
        }
        Err(_) => {
            println!(
                "noted: the chosen display published nothing further, so it is idle \
                 and producer progress under retention is not exercised"
            );
            close(&session).expect("close");
            return;
        }
    }

    // Hold the whole budget, which is what a consumer that never releases does.
    for _ in 0..DETACHED_BUFFER_BUDGET.get() {
        match next_frame_within(&session, FrameRequest::newer_than(stamp), COLLECT_WAIT) {
            Ok(frame) => {
                stamp = frame.stamp();
                retained.push(frame);
            }
            Err(error) if error.status() == Status::CaptureFailed => {
                panic!("the producer failed while frames were retained: {error}");
            }
            // A budget exhausted by the frames already held is the bounded
            // outcome, not a failure: the producer keeps running and drops.
            Err(_) => break,
        }
    }

    // Releasing capacity has to let the stream advance again, which is what
    // proves the producer was never blocked on the consumer.
    retained.truncate(1);
    match next_frame(&session, FrameRequest::newer_than(stamp)) {
        Ok(resumed) => assert!(resumed.stamp().sequence() > stamp.sequence()),
        Err(error) if error.status() == Status::DeadlineExceeded => {
            // The content stopped changing during the retention phase. The
            // producer is still live, which the lifecycle assertion below covers;
            // what cannot be shown from here is advancement with nothing to
            // advance to.
            println!(
                "noted: the display went idle during retention, so resumption \
                 after release is not exercised"
            );
        }
        Err(error) => panic!("the stream failed after capacity was released: {error}"),
    }
    assert_eq!(
        session.lifecycle(),
        Lifecycle::Open,
        "retaining the whole budget must not terminate the session"
    );
    close(&session).expect("close");
}

#[test]
fn a_mapping_outlives_its_frame_its_session_and_its_close() {
    let _serial = serialized();
    let Some(harness) = Harness::acquire("mapping lifetime") else {
        return;
    };
    let session = harness.open(0).expect("open");
    let frame = next_frame(&session, FrameRequest::latest()).expect("frame");
    let descriptor = frame.descriptor();

    let mapping = frame
        .map(PixelFormat::Bgra8, &OperationContext::new())
        .expect("a published frame maps in the format it was captured in");
    assert_eq!(mapping.descriptor(), descriptor);
    assert_eq!(
        mapping.bytes().len(),
        descriptor.byte_len(),
        "a mapping produces exactly the descriptor's own row stride"
    );
    assert_eq!(descriptor.stride(), descriptor.row_bytes());

    drop(frame);
    close(&session).expect("close");
    drop(session);

    assert_eq!(mapping.bytes().len(), descriptor.byte_len());
}

#[test]
fn concurrent_mappings_of_one_frame_share_a_single_conversion() {
    let _serial = serialized();
    let Some(harness) = Harness::acquire("mapping arbitration") else {
        return;
    };
    let session = harness.open(0).expect("open");
    let frame = next_frame(&session, FrameRequest::latest()).expect("frame");

    let readers: Vec<_> = (0..4)
        .map(|_| {
            let frame = frame.clone();
            thread::spawn(move || {
                frame
                    .map(PixelFormat::Bgra8, &OperationContext::new())
                    .expect("mapping")
                    .bytes()
                    .len()
            })
        })
        .collect();
    let lengths: Vec<usize> = readers
        .into_iter()
        .map(|reader| reader.join().expect("reader"))
        .collect();

    assert!(
        lengths
            .iter()
            .all(|length| *length == frame.descriptor().byte_len())
    );
    close(&session).expect("close");
}

#[test]
fn close_is_idempotent_and_leaves_no_native_object_alive() {
    let _serial = serialized();
    let Some(harness) = Harness::acquire("idempotent close") else {
        return;
    };
    let baseline = shim::live_objects();
    let session = harness.open(0).expect("open");
    // Released before the assertion on purpose. A frame a caller still holds owns
    // a detached buffer by contract, so leaving one alive here would assert that
    // the retention rule is broken rather than that close releases what it owns.
    drop(next_frame(&session, FrameRequest::latest()).expect("frame"));

    close(&session).expect("the first close succeeds");
    assert_eq!(session.lifecycle(), Lifecycle::Closed);
    close(&session).expect("a repeated close is not a second failure");

    drop(session);
    assert!(
        settles_to(baseline),
        "every native object the session owned is released by close"
    );
}

#[test]
fn concurrent_close_callers_serialize_one_shared_native_session() {
    let _serial = serialized();
    let Some(harness) = Harness::acquire("concurrent close serialization") else {
        return;
    };
    let baseline = shim::live_objects();
    let session = harness.open(0).expect("open");
    drop(next_frame(&session, FrameRequest::latest()).expect("frame"));
    let ready = Arc::new(Barrier::new(3));

    thread::scope(|scope| {
        let mut callers = Vec::new();
        for _ in 0..2 {
            let session = Arc::clone(&session);
            let ready = Arc::clone(&ready);
            callers.push(scope.spawn(move || {
                ready.wait();
                session.close(
                    &OperationContext::new()
                        .with_timeout(Duration::from_secs(5))
                        .expect("close timeout"),
                )
            }));
        }
        ready.wait();
        for caller in callers {
            caller
                .join()
                .expect("close caller did not panic")
                .expect("serialized close succeeds");
        }
    });

    assert_eq!(session.lifecycle(), Lifecycle::Closed);
    drop(session);
    assert!(
        settles_to(baseline),
        "concurrent close callers release each native object exactly once"
    );
}

#[test]
fn implicit_drop_quarantines_a_registration_after_a_fence_timeout() {
    let _serial = serialized();
    let Some(harness) = Harness::acquire("implicit close fence timeout") else {
        return;
    };
    let baseline = shim::live_objects();
    let session = harness
        .open(DELAY_IN_RUST_CALLBACK)
        .expect("open with one delayed Rust callback");
    let deadline = Instant::now() + FRAME_WAIT;
    while !testing_delayed_callback_is_active() {
        assert!(
            Instant::now() < deadline,
            "the delayed callback becomes active before the scenario deadline"
        );
        thread::sleep(Duration::from_millis(2));
    }

    drop(session);

    assert!(
        settles_to(baseline),
        "the quarantine worker resumes teardown and releases the callback registration"
    );
}

#[test]
fn implicit_drop_retains_the_session_handle_until_a_delayed_native_stop_joins() {
    let _serial = serialized();
    let Some(harness) = Harness::acquire("implicit delayed native stop") else {
        return;
    };
    let baseline = shim::live_objects();
    let session = harness
        .open_with_delays(Duration::ZERO, Duration::from_millis(2_500))
        .expect("open session with a delayed native stop");
    let core_is_alive = session.core_lifetime_probe();

    drop(session);

    assert!(
        core_is_alive(),
        "the quarantine worker retains the Rust session handle while native stop is pending"
    );
    assert!(
        shim::live_objects() > baseline,
        "native ownership remains live until the pending stop completes"
    );
    assert!(
        settles_to(baseline),
        "the joined delayed-stop worker releases every native object"
    );
    let deadline = Instant::now() + Duration::from_secs(1);
    while core_is_alive() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    assert!(
        !core_is_alive(),
        "the Rust session handle is released only after the close worker joins"
    );
}

#[test]
fn a_frame_a_caller_still_holds_keeps_its_own_storage_after_close() {
    let _serial = serialized();
    let Some(harness) = Harness::acquire("retention across close") else {
        return;
    };
    let baseline = shim::live_objects();
    let session = harness.open(0).expect("open");
    let frame = next_frame(&session, FrameRequest::latest()).expect("frame");

    close(&session).expect("close");
    drop(session);

    // The retained frame is the caller's, not the session's, so it is still
    // mappable and its buffer is still alive after everything else is gone.
    let mapping = frame
        .map(PixelFormat::Bgra8, &OperationContext::new())
        .expect("a retained frame maps after its session is gone");
    assert_eq!(mapping.bytes().len(), frame.descriptor().byte_len());
    assert!(
        shim::live_objects() > baseline,
        "the retained frame's own buffer is still owned"
    );

    drop(mapping);
    drop(frame);
    assert!(
        settles_to(baseline),
        "releasing the last retained frame releases the last buffer"
    );
}

#[test]
fn a_cancelled_close_leaves_a_state_a_later_close_finishes() {
    let _serial = serialized();
    let Some(harness) = Harness::acquire("retryable close") else {
        return;
    };
    let session = harness.open(0).expect("open");
    let _frame = next_frame(&session, FrameRequest::latest()).expect("frame");

    let token = CancellationToken::new();
    token.cancel();
    let cancelled = OperationContext::new().with_cancellation(token);
    let error = session
        .close(&cancelled)
        .expect_err("a cancelled close cannot finish the drain");
    assert_eq!(error.status(), Status::Cancelled);
    assert_ne!(
        session.lifecycle(),
        Lifecycle::Open,
        "a cancelled close still stops accepting new work"
    );

    close(&session).expect("a later close continues rather than restarting");
    assert_eq!(session.lifecycle(), Lifecycle::Closed);
}

#[test]
fn native_start_settles_after_internal_slice_within_caller_deadline() {
    let _serial = serialized();
    let Some(harness) = Harness::acquire("start beyond one internal wait slice") else {
        return;
    };
    let baseline = shim::live_objects();
    let submissions =
        shim::testing_capture_lifecycle_counts().expect("read lifecycle submission baseline");
    let session = harness
        .open_with_delays(MAX_NATIVE_WAIT, Duration::ZERO)
        .expect("the caller's ten-second operation still owns the accepted start");

    close(&session).expect("close the session");
    drop(session);
    assert!(
        settles_to(baseline),
        "the delayed start and close release every native object"
    );
    let settled =
        shim::testing_capture_lifecycle_counts().expect("read lifecycle submission outcome");
    assert_eq!(
        [settled[0] - submissions[0], settled[1] - submissions[1]],
        [1, 1],
        "one delayed open owns one start and one stop"
    );
}

#[test]
fn native_start_without_a_caller_deadline_waits_through_internal_slices() {
    let _serial = serialized();
    let Some(harness) = Harness::acquire("native start without caller deadline") else {
        return;
    };
    let baseline = shim::live_objects();
    let submissions =
        shim::testing_capture_lifecycle_counts().expect("read lifecycle submission baseline");
    let context = OperationContext::new();
    let session = harness
        .open_with_delays_in(
            MAX_NATIVE_WAIT + Duration::from_millis(50),
            Duration::ZERO,
            &context,
        )
        .expect("an unbounded caller joins the accepted start until settlement");
    close(&session).expect("close session");
    drop(session);
    assert!(
        settles_to(baseline),
        "unbounded start left native ownership"
    );
    let settled =
        shim::testing_capture_lifecycle_counts().expect("read lifecycle submission outcome");
    assert_eq!(
        [settled[0] - submissions[0], settled[1] - submissions[1]],
        [1, 1],
        "internal slices do not resubmit native start"
    );
}

#[test]
fn simultaneous_start_callers_join_one_native_submission() {
    let _serial = serialized();
    let Some(harness) = Harness::acquire("simultaneous native start callers") else {
        return;
    };
    let baseline = shim::live_objects();
    let submissions =
        shim::testing_capture_lifecycle_counts().expect("read lifecycle submission baseline");
    let session = Arc::new(
        harness
            .open_unstarted_shim(Duration::from_millis(150), Duration::ZERO, 0)
            .expect("open unstarted shim session"),
    );
    let barrier = Arc::new(Barrier::new(3));
    let callers = (0..2)
        .map(|_| {
            let session = Arc::clone(&session);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                session.start(MAX_NATIVE_WAIT)
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    for caller in callers {
        assert_eq!(caller.join().expect("start caller"), Ok(()));
    }
    session.close(MAX_NATIVE_WAIT).expect("close session");
    drop(session);
    assert!(
        settles_to(baseline),
        "concurrent start left native ownership"
    );
    let settled =
        shim::testing_capture_lifecycle_counts().expect("read lifecycle submission outcome");
    assert_eq!(
        [settled[0] - submissions[0], settled[1] - submissions[1]],
        [1, 1],
        "both callers join one accepted start and close submits one stop"
    );
}

#[test]
fn releasing_a_session_during_pending_start_joins_and_stops_once() {
    let _serial = serialized();
    let Some(harness) = Harness::acquire("release during pending native start") else {
        return;
    };
    let baseline = shim::live_objects();
    let submissions =
        shim::testing_capture_lifecycle_counts().expect("read lifecycle submission baseline");
    let session = harness
        .open_unstarted_shim(Duration::from_millis(150), Duration::ZERO, 0)
        .expect("open unstarted shim session");
    assert_eq!(
        session.start(Duration::from_millis(5)),
        Err(shim::ShimStatus::TimedOut)
    );
    drop(session);
    assert!(settles_to(baseline), "release left native ownership");
    let settled =
        shim::testing_capture_lifecycle_counts().expect("read lifecycle submission outcome");
    assert_eq!(
        [settled[0] - submissions[0], settled[1] - submissions[1]],
        [1, 1],
        "release joins the accepted start and submits one stop"
    );
}

#[test]
fn caller_deadline_during_accepted_start_still_reaps_the_session() {
    let _serial = serialized();
    let Some(harness) = Harness::acquire("deadline during accepted native start") else {
        return;
    };
    let baseline = shim::live_objects();
    let submissions =
        shim::testing_capture_lifecycle_counts().expect("read lifecycle submission baseline");
    let context = OperationContext::new()
        .with_timeout(Duration::from_millis(100))
        .expect("positive timeout");
    let error = harness
        .open_with_delays_in(Duration::from_millis(150), Duration::ZERO, &context)
        .expect_err("the caller deadline expires while native start remains accepted");
    assert_eq!(error.status(), Status::DeadlineExceeded);
    assert!(settles_to(baseline), "deadline left native ownership");
    let settled =
        shim::testing_capture_lifecycle_counts().expect("read lifecycle submission outcome");
    assert_eq!(
        [settled[0] - submissions[0], settled[1] - submissions[1]],
        [1, 1],
        "deadline cleanup joins and stops the accepted start"
    );
}

#[test]
fn caller_cancellation_during_accepted_start_still_reaps_the_session() {
    let _serial = serialized();
    let Some(harness) = Harness::acquire("cancellation during accepted native start") else {
        return;
    };
    let baseline = shim::live_objects();
    let submissions =
        shim::testing_capture_lifecycle_counts().expect("read lifecycle submission baseline");
    let cancellation = CancellationToken::new();
    let canceller = cancellation.clone();
    let context = OperationContext::new()
        .with_timeout(Duration::from_secs(1))
        .expect("positive timeout")
        .with_cancellation(cancellation);
    let cancel_thread = thread::spawn(move || {
        thread::sleep(Duration::from_millis(20));
        canceller.cancel();
    });
    let error = harness
        .open_with_delays_in(Duration::from_millis(150), Duration::ZERO, &context)
        .expect_err("caller cancellation wins after native start was accepted");
    cancel_thread.join().expect("canceller");
    assert_eq!(error.status(), Status::Cancelled);
    assert!(settles_to(baseline), "cancellation left native ownership");
    let settled =
        shim::testing_capture_lifecycle_counts().expect("read lifecycle submission outcome");
    assert_eq!(
        [settled[0] - submissions[0], settled[1] - submissions[1]],
        [1, 1],
        "cancellation cleanup joins and stops the accepted start"
    );
}

#[test]
fn a_close_timeout_during_native_start_is_resumable() {
    let _serial = serialized();
    let Some(harness) = Harness::acquire("retryable delayed native start") else {
        return;
    };
    let baseline = shim::live_objects();
    let submissions =
        shim::testing_capture_lifecycle_counts().expect("read lifecycle submission baseline");
    let session = harness
        .open_unstarted_shim(Duration::from_millis(150), Duration::ZERO, 0)
        .expect("open unstarted shim session");

    assert_eq!(
        session.start(Duration::from_millis(5)),
        Err(shim::ShimStatus::TimedOut),
        "the injected start completion remains in flight"
    );
    assert_eq!(
        session.close(Duration::from_millis(5)),
        Err(shim::ShimStatus::TimedOut),
        "the first close preserves its pending-start phase"
    );
    session
        .close(MAX_NATIVE_WAIT)
        .expect("a later close joins the same start and completes teardown");
    session
        .close(Duration::from_millis(1))
        .expect("terminal close remains idempotent");
    drop(session);

    assert!(
        settles_to(baseline),
        "the resumed delayed-start close releases every native object"
    );
    let settled =
        shim::testing_capture_lifecycle_counts().expect("read lifecycle submission outcome");
    assert_eq!(
        [settled[0] - submissions[0], settled[1] - submissions[1]],
        [1, 1],
        "close retries join one accepted start and submit one stop"
    );
}

#[test]
fn close_during_a_pending_failed_start_joins_failure_and_stops_once() {
    let _serial = serialized();
    let Some(harness) = Harness::acquire("close during pending failed start") else {
        return;
    };
    let baseline = shim::live_objects();
    let submissions =
        shim::testing_capture_lifecycle_counts().expect("read lifecycle submission baseline");
    let session = harness
        .open_unstarted_shim(
            Duration::from_millis(150),
            Duration::ZERO,
            RAISE_IN_START_COMPLETION,
        )
        .expect("open unstarted shim session");
    assert_eq!(
        session.start(Duration::from_millis(5)),
        Err(shim::ShimStatus::TimedOut),
        "start remains pending when close begins"
    );
    assert_eq!(
        session.close(MAX_NATIVE_WAIT),
        Err(shim::ShimStatus::NativeException),
        "close reports the cached start failure after stopping accepted ownership"
    );
    drop(session);
    assert!(
        settles_to(baseline),
        "failed start close left native ownership"
    );
    let settled =
        shim::testing_capture_lifecycle_counts().expect("read lifecycle submission outcome");
    assert_eq!(
        [settled[0] - submissions[0], settled[1] - submissions[1]],
        [1, 1],
        "failed settlement still has one accepted start and at most one stop"
    );
}

#[test]
fn a_close_timeout_during_native_stop_is_resumable() {
    let _serial = serialized();
    let Some(harness) = Harness::acquire("retryable delayed native stop") else {
        return;
    };
    let baseline = shim::live_objects();
    let session = harness
        .open_with_delays(Duration::ZERO, Duration::from_millis(150))
        .expect("open session with delayed stop completion");
    let first = OperationContext::new()
        .with_timeout(Duration::from_millis(5))
        .expect("timeout");

    let error = session
        .close(&first)
        .expect_err("the first close expires inside the native stop phase");
    assert_eq!(error.status(), Status::DeadlineExceeded);
    assert_ne!(session.lifecycle(), Lifecycle::Open);

    close(&session).expect("a later close resumes and completes the pending stop");
    assert_eq!(session.lifecycle(), Lifecycle::Closed);
    drop(session);
    assert!(
        settles_to(baseline),
        "the resumed delayed-stop close releases every native object"
    );
}

#[test]
fn a_closed_session_refuses_further_frame_requests() {
    let _serial = serialized();
    let Some(harness) = Harness::acquire("closed refusal") else {
        return;
    };
    let session = harness.open(0).expect("open");
    let _frame = next_frame(&session, FrameRequest::latest()).expect("frame");
    close(&session).expect("close");

    let error = next_frame(&session, FrameRequest::latest())
        .expect_err("a closed session accepts no frame work");

    assert!(
        matches!(error.status(), Status::Closed | Status::TargetLost),
        "a closed session reports why it stopped, not a capture failure: {error}"
    );
}

#[test]
fn a_start_session_hold_allocation_failure_is_typed_and_leaves_no_native_object_alive() {
    let _serial = serialized();
    start_allocation_failure("capture-start session hold", FAIL_START_HOLD_ALLOCATION);
}

#[test]
fn a_reconfigure_semaphore_allocation_failure_is_typed_before_framework_submission() {
    let _serial = serialized();
    let Some(harness) = Harness::acquire("reconfigure semaphore allocation failure") else {
        return;
    };
    let baseline = shim::live_objects();
    let session = harness
        .open_unstarted_shim(
            Duration::ZERO,
            Duration::ZERO,
            FAIL_RECONFIGURE_SEMAPHORE_ALLOCATION,
        )
        .expect("open unstarted shim session");

    assert_eq!(
        session.reconfigure(harness.metadata.extent, MAX_NATIVE_WAIT),
        Err(shim::ShimStatus::PlatformFailure)
    );
    session
        .close(MAX_NATIVE_WAIT)
        .expect("close the unstarted session");
    drop(session);
    assert!(
        settles_to(baseline),
        "reconfigure allocation failure left a native object alive"
    );
}

#[test]
fn a_contained_exception_at_the_start_site_leaves_no_native_object_alive() {
    let _serial = serialized();
    contained_site("start", RAISE_AT_START, FrameExpectation::Any);
}

#[test]
fn a_start_submission_exception_settles_once_without_framework_ownership() {
    let _serial = serialized();
    let Some(harness) = Harness::acquire("start-submission containment") else {
        return;
    };
    let baseline = shim::live_objects();
    let submissions =
        shim::testing_capture_lifecycle_counts().expect("read lifecycle submission baseline");
    let session = harness
        .open_unstarted_shim(Duration::ZERO, Duration::ZERO, RAISE_AT_START_SUBMISSION)
        .expect("open unstarted shim session");
    assert_eq!(
        session.start(MAX_NATIVE_WAIT),
        Err(shim::ShimStatus::NativeException)
    );
    assert_eq!(
        session.start(Duration::from_millis(1)),
        Err(shim::ShimStatus::NativeException),
        "later callers observe the cached submission failure"
    );
    drop(session);
    assert!(
        settles_to(baseline),
        "submission exception left native ownership"
    );
    let settled =
        shim::testing_capture_lifecycle_counts().expect("read lifecycle submission outcome");
    assert_eq!(
        [settled[0] - submissions[0], settled[1] - submissions[1]],
        [0, 0],
        "the framework accepted neither a start nor a stop"
    );
}

/// The capture-start completion block, which the start site above cannot reach.
///
/// That block is invoked by the framework, so an exception leaving it unwinds into a
/// frame with no handler above it anywhere — an abort rather than a status, which is
/// what ADR 0012 rule 1 exists to prevent. It went in without the `@try` every other
/// trampoline in the shim carries, and nothing could observe that: the block's own
/// stop message is the thing that can raise there, and it is reached only when a start
/// succeeds after teardown has already run.
///
/// So the seam raises at that position instead. Without the containment this case does
/// not fail, it aborts the test process — which is the observable, and the reason the
/// assertion below is about the session rather than about the exception.
#[test]
fn a_contained_exception_in_the_start_completion_leaves_no_native_object_alive() {
    let _serial = serialized();
    let submissions =
        shim::testing_capture_lifecycle_counts().expect("read lifecycle submission baseline");
    contained_site(
        "the start completion",
        RAISE_IN_START_COMPLETION,
        FrameExpectation::Any,
    );
    let settled =
        shim::testing_capture_lifecycle_counts().expect("read lifecycle submission outcome");
    assert_eq!(
        [settled[0] - submissions[0], settled[1] - submissions[1]],
        [1, 1],
        "completion containment preserves one accepted start and one stop"
    );
}

#[test]
fn a_contained_exception_in_the_stop_completion_is_reported_once_and_retryable() {
    let _serial = serialized();
    let Some(harness) = Harness::acquire("stop-completion containment") else {
        return;
    };
    let baseline = shim::live_objects();
    let session = harness
        .open(RAISE_IN_STOP_COMPLETION)
        .expect("the stop-completion seam is reached only after open");

    let first = close(&session).expect_err("the contained exception is reported once");
    assert_eq!(first.status(), Status::CaptureFailed);
    close(&session).expect("a later close resumes and completes without another stop");
    drop(session);

    assert!(
        settles_to(baseline),
        "the stop-completion exception left native ownership behind"
    );
}

#[test]
fn a_contained_exception_before_a_frame_callback_leaves_no_native_object_alive() {
    let _serial = serialized();
    // A raise before the callback means the Adapter is never handed the frame, so
    // nothing is ever published. Requiring that observable is what proves the
    // raise fired rather than the display having gone quiet.
    contained_site(
        "before frame callback",
        RAISE_BEFORE_CALLBACK,
        FrameExpectation::None,
    );
}

#[test]
fn a_contained_exception_after_a_frame_callback_leaves_no_native_object_alive() {
    let _serial = serialized();
    // The first callback staged its detached frame before this raise. Native
    // terminalization must discard it before the separate commit callback can make
    // it observable, so the fault deterministically outranks the candidate.
    contained_site(
        "after frame callback",
        RAISE_AFTER_CALLBACK,
        FrameExpectation::None,
    );
}

#[test]
fn a_panicking_rust_frame_callback_terminalizes_the_session_once() {
    let _serial = serialized();
    contained_site(
        "a Rust frame callback panic",
        PANIC_IN_RUST_CALLBACK,
        FrameExpectation::None,
    );
}

#[test]
fn a_contained_exception_at_teardown_leaves_no_native_object_alive() {
    let _serial = serialized();
    contained_site("teardown", RAISE_AT_TEARDOWN, FrameExpectation::Any);
}

/// What a raise site implies about whether a frame can still reach a caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameExpectation {
    /// A callback-boundary fault outranks any frame queued before terminalization.
    None,
    /// The raise is outside the frame path and says nothing about frames.
    Any,
}

/// Asserts that a native exception raised at one boundary position is contained,
/// reported as a typed outcome, and costs no native object.
///
/// These are the cases that stop holding if `-fobjc-arc-exceptions` is ever
/// dropped from the build script, which is how a compiler flag becomes a tested
/// invariant rather than a comment.
fn start_allocation_failure(name: &str, site: u32) {
    let Some(harness) = Harness::acquire(&format!("{name} allocation failure")) else {
        return;
    };
    let baseline = shim::live_objects();
    let error = harness
        .open(site)
        .expect_err("allocation failure prevents the asynchronous start submission");
    assert_eq!(error.status(), Status::CaptureFailed);
    assert!(
        settles_to(baseline),
        "{name} allocation failure left a native object alive"
    );
}

fn contained_site(name: &str, site: u32, expectation: FrameExpectation) {
    let scenario = format!("containment at {name}");
    // The frame sites need a display that is actually producing, or the raise
    // never fires and the case passes without having run.
    let harness = if expectation == FrameExpectation::Any {
        Harness::acquire(&scenario)
    } else {
        Harness::acquire_producing(&scenario)
    };
    let Some(harness) = harness else {
        return;
    };
    let baseline = shim::live_objects();

    let opened = harness.open(site);
    match opened {
        Ok(session) => {
            // Whatever it produced is released here: the question is what the
            // contained failure cost, not what a caller is still holding.
            let arrived = next_frame(&session, FrameRequest::latest());
            let terminal_request = match expectation {
                FrameExpectation::None => {
                    assert_eq!(
                        arrived.err().map(|error| error.status()),
                        Some(Status::CaptureFailed),
                        "a callback failure becomes the defined typed session outcome"
                    );
                    Some(FrameRequest::latest())
                }
                FrameExpectation::Any => {
                    drop(arrived);
                    None
                }
            };
            let expects_terminal = terminal_request.is_some();
            if let Some(request) = terminal_request {
                let error = next_frame(&session, request)
                    .expect_err("the callback boundary failure terminalized the session");
                assert_eq!(error.status(), Status::CaptureFailed);
                assert_eq!(
                    session.terminal_reports(),
                    1,
                    "native and Rust callback failure paths share one terminal gate"
                );
            }
            let closed = close(&session);
            if let Err(error) = closed {
                assert_ne!(
                    error.status(),
                    Status::Internal,
                    "a contained exception reports a typed outcome"
                );
            }
            if expects_terminal {
                assert_eq!(
                    session.terminal_reports(),
                    1,
                    "close cannot deliver a second terminal callback"
                );
            }
            drop(session);
        }
        Err(error) => {
            // A raise at the start site fails the open, which is the typed
            // outcome the caller sees rather than a crash.
            assert_ne!(error.status(), Status::Internal);
        }
    }

    assert!(
        settles_to(baseline),
        "a contained failure at the {name} site left a native object alive: {} against a \
         baseline of {baseline}",
        shim::live_objects()
    );
}

#[test]
fn a_long_producer_run_keeps_live_native_objects_bounded() {
    let _serial = serialized();
    let Some(harness) = Harness::acquire_producing("autorelease bound") else {
        return;
    };
    let session = harness.open(0).expect("open");
    let first = next_frame(&session, FrameRequest::latest()).expect("first frame");
    let after_first = shim::live_objects();
    let mut stamp = first.stamp();
    drop(first);

    // Measured over a duration rather than over publications. The pool this
    // scenario is about drains per producer *work item*, and the producer runs a
    // work item for every sample it delivers — including the ones filtered out as
    // unchanged, which never become publications. Counting publications would make
    // the evidence depend on how busy the user's desktop happens to be, and a run
    // of three frames bounds nothing.
    let mut observed = 1;
    let mut peak = after_first;
    let mut samples = 0u32;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(frame) =
            next_frame_within(&session, FrameRequest::newer_than(stamp), COLLECT_WAIT)
        {
            stamp = frame.stamp();
            observed += 1;
        }
        peak = peak.max(shim::live_objects());
        samples += 1;
    }

    assert!(
        peak <= after_first + u64::from(DETACHED_BUFFER_BUDGET.get()),
        "live native objects grew during the run: {after_first} at the first frame, \
         peak {peak} over {samples} sample(s) and {observed} publication(s)"
    );
    assert!(
        samples >= 5,
        "the run sampled {samples} time(s), which bounds nothing"
    );
    println!(
        "measured: peak {peak} live native objects against a first-frame {after_first} \
         over {samples} sample(s) and {observed} publication(s)"
    );
    close(&session).expect("close");
}

#[test]
fn a_frame_extent_the_shim_cannot_describe_is_refused() {
    let _serial = serialized();
    let unsupported = crate::storage::descriptor_from_native(9, PixelExtent::new(8, 6));

    assert_eq!(unsupported, Err(CaptureFault::UnsupportedFormat));
}

/// Waits briefly for the shim's owned-object count to return to `baseline`.
///
/// A native release can complete just after the operation that requested it
/// returns, so this polls rather than sampling once and calling a race a leak.
fn settles_to(baseline: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if shim::live_objects() <= baseline {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
}
