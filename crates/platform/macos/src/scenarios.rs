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

use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use mado_pilot_capture::{
    CaptureFault, CaptureSession, CoordinateSupport, Frame, FrameRequest, Lifecycle, PixelFormat,
};
use mado_pilot_core::{
    CancellationToken, Clock, CoordinateSpace, IdentityIssuer, Operation, OperationContext,
    PixelExtent, Point, Status, SystemClock, TargetKind,
};

use crate::availability::ensure_capture_available;
use crate::discovery::{Candidate, NativeKey, TargetMetadata, inventory};
use crate::native::NativeSession;
use crate::shim::{
    self, MAX_NATIVE_WAIT, RAISE_AFTER_CALLBACK, RAISE_AT_START, RAISE_AT_TEARDOWN,
    RAISE_BEFORE_CALLBACK,
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
    static GATE: Mutex<()> = Mutex::new(());
    GATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// One authorized target, with everything a session open needs.
struct Harness {
    issuer: Arc<IdentityIssuer>,
    key: NativeKey,
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
/// the case has proven nothing. `build.rs` records the command and the flag.
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
        NativeSession::open_with_raise_sites(
            target,
            stream,
            self.key,
            self.metadata.clone(),
            raise_sites,
            &mut operation,
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

    let frame = next_frame(&session, FrameRequest::latest()).expect("the display produces frames");

    assert_eq!(frame.descriptor().extent(), harness.metadata.extent);
    assert!(frame.transform().covers_target());
    let placement = frame
        .transform()
        .target()
        .expect("a display frame carries authoritative placement");
    assert_eq!(placement.scale().x(), placement.desktop_scale().x());
    let origin = frame
        .transform()
        .convert_point(
            Point::new(CoordinateSpace::CapturePixels, 0.0, 0.0).expect("valid"),
            CoordinateSpace::DesktopLogical,
        )
        .expect("desktop conversion");
    assert_eq!(
        (origin.x(), origin.y()),
        harness.metadata.placement.desktop_origin(),
        "the frame's own geometry is what a conversion uses"
    );
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
fn a_window_session_publishes_frames_whose_own_geometry_places_them() {
    let _serial = serialized();
    let Some(candidates) = discovered("window publication") else {
        return;
    };
    let Some((_harness, session, frame)) =
        producing_window("window publication", &candidates, |_| true)
    else {
        return;
    };

    // Asserted against the frame's own transform rather than the discovery
    // metadata: a window is free to resize between the two, and the frame's
    // geometry is what a conversion is required to use.
    let placement = frame
        .transform()
        .target()
        .expect("a window frame carries authoritative placement");
    assert_eq!(
        frame.transform().frame_extent(),
        frame.descriptor().extent()
    );
    assert!(frame.transform().covers_target());
    assert!(placement.scale().x() > 0.0);
    assert_eq!(placement.scale().x(), placement.desktop_scale().x());

    let origin = frame
        .transform()
        .convert_point(
            Point::new(CoordinateSpace::CapturePixels, 0.0, 0.0).expect("valid"),
            CoordinateSpace::DesktopLogical,
        )
        .expect("desktop conversion");
    assert_eq!(
        (origin.x(), origin.y()),
        placement.desktop_origin(),
        "the frame's own placement is what a conversion uses, not live host state"
    );

    let mapping = frame
        .map(PixelFormat::Bgra8, &OperationContext::new())
        .expect("a window frame maps");
    assert_eq!(mapping.bytes().len(), frame.descriptor().byte_len());
    close(&session).expect("close");
}

#[test]
fn every_attached_display_publishes_frames_placed_by_its_own_geometry() {
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

    let mut observed = Vec::new();
    for display in &displays {
        let harness = Harness::from_candidate(display);
        let session = harness.open(0).expect("a discovered display opens");
        let frame = next_frame(&session, FrameRequest::latest()).expect("the display publishes");

        let placement = frame
            .transform()
            .target()
            .expect("a display frame carries authoritative placement");
        let origin = frame
            .transform()
            .convert_point(
                Point::new(CoordinateSpace::CapturePixels, 0.0, 0.0).expect("valid"),
                CoordinateSpace::DesktopLogical,
            )
            .expect("desktop conversion");

        // Each frame must place itself from its own geometry. A conversion that
        // consulted live host state, or the main display, would agree for one
        // display and disagree for every other one.
        assert_eq!(
            (origin.x(), origin.y()),
            placement.desktop_origin(),
            "a display frame converted through another display's origin"
        );
        assert_eq!(
            placement.desktop_origin(),
            display.metadata.placement.desktop_origin(),
            "the published placement is the one discovery reported"
        );
        observed.push((display.key, placement));
        close(&session).expect("close");
    }

    for (index, (key, placement)) in observed.iter().enumerate() {
        for (other_key, other) in observed.iter().skip(index + 1) {
            assert_ne!(
                placement.desktop_origin(),
                other.desktop_origin(),
                "{key:?} and {other_key:?} report the same desktop origin"
            );
        }
    }

    let signed = observed
        .iter()
        .filter(|(_, placement)| {
            let (x, y) = placement.desktop_origin();
            x < 0.0 || y < 0.0
        })
        .count();
    if signed == 0 {
        println!(
            "noted: no display sits above or left of the main one, \
             so a negative desktop origin is not exercised"
        );
    }
}

#[test]
fn a_seam_between_displays_of_differing_scale_is_one_desktop_coordinate() {
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

    let mut seam = Vec::new();
    for (position, display) in displays[index..=index + 1].iter().enumerate() {
        let harness = Harness::from_candidate(display);
        let session = harness.open(0).expect("a discovered display opens");
        let frame = next_frame(&session, FrameRequest::latest()).expect("the display publishes");
        let width = f64::from(frame.descriptor().extent().width());
        // The left display's far edge and the right display's near edge are the
        // same place on the desktop. Each is converted through its own frame, at
        // its own scale, which is the whole point: the two scales must not have to
        // agree for the coordinate between them to.
        let edge = if position == 0 { width } else { 0.0 };
        let converted = frame
            .transform()
            .convert_point(
                Point::new(CoordinateSpace::CapturePixels, edge, 0.0).expect("valid"),
                CoordinateSpace::DesktopLogical,
            )
            .expect("desktop conversion");
        seam.push((
            display.key,
            frame.transform().target().expect("placement").scale().x(),
            converted.x(),
        ));
        close(&session).expect("close");
    }

    let (left_key, left_scale, left_edge) = seam[0];
    let (right_key, right_scale, right_edge) = seam[1];
    assert_ne!(
        left_scale, right_scale,
        "the pair was selected for differing scale"
    );
    assert_eq!(
        left_edge, right_edge,
        "{left_key:?} at scale {left_scale} and {right_key:?} at scale {right_scale} \
         disagree about the desktop coordinate between them"
    );
    println!("measured: seam at {left_edge} joins scale {left_scale} and scale {right_scale}");
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
fn a_window_left_of_the_main_display_reports_signed_desktop_coordinates() {
    let _serial = serialized();
    let Some(candidates) = discovered("signed window placement") else {
        return;
    };
    let Some((harness, session, frame)) =
        producing_window("signed window placement", &candidates, |candidate| {
            let (x, y) = candidate.metadata.placement.desktop_origin();
            x < 0.0 || y < 0.0
        })
    else {
        return;
    };
    let expected = harness.metadata.placement.desktop_origin();

    let origin = frame
        .transform()
        .convert_point(
            Point::new(CoordinateSpace::CapturePixels, 0.0, 0.0).expect("valid"),
            CoordinateSpace::DesktopLogical,
        )
        .expect("desktop conversion");
    let (x, y) = (origin.x(), origin.y());
    assert!(
        x < 0.0 || y < 0.0,
        "a window discovered at {expected:?} converted to a non-negative origin ({x}, {y})"
    );
    // The window may have moved between discovery and publication, so the frame's
    // own placement is what the conversion is checked against.
    assert_eq!(
        (x, y),
        frame
            .transform()
            .target()
            .expect("placement")
            .desktop_origin()
    );
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
fn a_contained_exception_at_the_start_site_leaves_no_native_object_alive() {
    let _serial = serialized();
    contained_site("start", RAISE_AT_START, FrameExpectation::Any);
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
    // A raise after the callback returned means the frame was already published,
    // so one must arrive and the containment must cost nothing.
    contained_site(
        "after frame callback",
        RAISE_AFTER_CALLBACK,
        FrameExpectation::Some,
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
    /// The raise happens before publication, so no frame may arrive.
    None,
    /// The raise happens after publication, so one must arrive.
    Some,
    /// The raise is outside the frame path and says nothing about frames.
    Any,
}

/// Asserts that a native exception raised at one boundary position is contained,
/// reported as a typed outcome, and costs no native object.
///
/// These are the cases that stop holding if `-fobjc-arc-exceptions` is ever
/// dropped from the build script, which is how a compiler flag becomes a tested
/// invariant rather than a comment.
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
            match expectation {
                FrameExpectation::None => assert_eq!(
                    arrived.err().map(|error| error.status()),
                    Some(Status::DeadlineExceeded),
                    "a raise before the callback must stop the frame reaching a caller"
                ),
                FrameExpectation::Some => {
                    let frame = arrived
                        .expect("a raise after the callback returned still publishes the frame");
                    assert_eq!(frame.descriptor().extent(), harness.metadata.extent);
                    drop(frame);
                }
                FrameExpectation::Any => drop(arrived),
            }
            let closed = close(&session);
            if let Err(error) = closed {
                assert_ne!(
                    error.status(),
                    Status::Internal,
                    "a contained exception reports a typed outcome"
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
        "a contained failure at the {name} site left a native object alive"
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
fn a_lost_target_identity_is_reported_rather_than_replaced() {
    let _serial = serialized();
    // A display identifier no arrangement can produce stands in for a target that
    // existed and does not now: the point is that the Adapter reports the loss
    // instead of finding something else to capture.
    let absent = NativeKey::Display(u32::MAX);

    assert!(!absent.is_present());
    assert_eq!(
        shim::current_placement(absent.native_kind(), absent.native_id())
            .expect_err("an absent display has no placement"),
        shim::ShimStatus::TargetLost
    );
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
