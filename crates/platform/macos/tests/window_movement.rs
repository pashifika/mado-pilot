#![cfg_attr(not(target_os = "macos"), allow(missing_docs))]
#![cfg(target_os = "macos")]
//! Opt-in probe for a window moving between displays inside one live stream.
//!
//! This is the one case in the macOS capture Change that no automated run can
//! reach. The transition it exercises fires when a captured window's placement
//! changes, and moving another application's window needs Accessibility
//! authorization this process does not hold — so the move has to be performed by
//! hand while the probe watches.
//!
//! It is therefore **skipped unless asked for**, and it asserts rather than only
//! reports, so a run that is asked for is evidence rather than a transcript:
//!
//! ```sh
//! MADO_PILOT_MACOS_WINDOW_MOVE_PROBE=1 \
//!   cargo test -p mado-pilot-platform-macos --test window_movement -- --nocapture
//! ```
//!
//! Drag the captured window from one display to another while it runs. It prints
//! each state as it observes it, so a drag registers visibly rather than only in
//! the summary. `MADO_PILOT_MACOS_WINDOW_MOVE_TARGET` selects a window by an index
//! from the list the probe prints or by a fragment of its name; without it the
//! widest window is used.
//!
//! Choose a window whose content keeps changing — a video, a log, an animation.
//! The framework publishes on content change, and moving a window does not by
//! itself change its content, so a static window can be dragged across a seam and
//! produce no frame to observe it in until something redraws.
//!
//! Everything here goes through the public provider surface, so the probe measures
//! what a caller would see rather than adapter internals.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mado_pilot_capture::{
    CaptureProvider, FrameRequest, OpenRequest, PixelFormat, TargetDescription,
};
use mado_pilot_core::{
    CoordinateSpace, GeometryRevision, IdentityIssuer, OperationContext, Point, Status,
    StreamEpoch, TargetKind,
};
use mado_pilot_platform_macos::MacosCaptureProvider;

/// Set to run the probe. Absent, the test reports what it would need and passes.
const PROBE_VARIABLE: &str = "MADO_PILOT_MACOS_WINDOW_MOVE_PROBE";

/// Optional index, from the list the probe prints, of the window to capture.
const TARGET_VARIABLE: &str = "MADO_PILOT_MACOS_WINDOW_MOVE_TARGET";

/// How long the probe watches for a move.
const WATCH: Duration = Duration::from_secs(75);

/// How long each frame request waits, short enough to keep printing progress.
const FRAME_WAIT: Duration = Duration::from_millis(400);

/// How often the probe says it is still watching, for someone running it by hand.
const HEARTBEAT: Duration = Duration::from_secs(10);

/// How long without a frame counts as the stream having stopped publishing.
///
/// A window whose content is changing publishes far more often than this, so a gap
/// this long at the end of a watch is the stream having stopped rather than the
/// content having settled — and those two call for opposite next steps.
const STALL: Duration = Duration::from_secs(5);

/// Where a window sits and how large it is, carrying no stream identity.
///
/// Separate from [`Observed`] so the same reading can come from a frame or from a
/// later look at the host, and the two can be compared directly.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Placed {
    origin: (f64, f64),
    scale: f64,
    extent: (u32, u32),
}

impl fmt::Display for Placed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "origin {:?} scale {} extent {}x{}",
            self.origin, self.scale, self.extent.0, self.extent.1
        )
    }
}

/// One distinct state the stream published under, in the order first seen.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Observed {
    epoch: StreamEpoch,
    geometry: GeometryRevision,
    placed: Placed,
    frames: u32,
}

/// Prints where each display sits and at what scale, names the seam most worth
/// dragging across, and returns the backing scales in play.
///
/// A display's placement is read by opening it briefly, because a target
/// description carries the extent but not the desktop origin — the origin is
/// frame-time geometry, which is the whole point of the contract. The scales come
/// back because a published frame carrying a scale that belongs to no display is the
/// signature of the target having been letterboxed into a surface too small for it,
/// and counting those turns this run into a measurement of that rather than a
/// transcript to read by eye.
fn report_display_arrangement(
    provider: &MacosCaptureProvider,
    targets: &[TargetDescription],
) -> Vec<f64> {
    let mut spans: Vec<(String, f64, f64, f64)> = Vec::new();
    for display in targets
        .iter()
        .filter(|target| target.capability().kind() == Some(TargetKind::Display))
    {
        let context = OperationContext::new()
            .with_timeout(Duration::from_secs(5))
            .expect("a positive timeout");
        let Ok(session) = provider.open(
            display.id(),
            &OpenRequest::new().require_format(PixelFormat::Bgra8),
            &context,
        ) else {
            continue;
        };
        let wait = OperationContext::new()
            .with_timeout(Duration::from_secs(5))
            .expect("a positive timeout");
        if let Ok(frame) = session.frame(&FrameRequest::latest(), &wait)
            && let Some(placement) = frame.transform().target()
        {
            let (x, _y) = placement.desktop_origin();
            let (width, _height) = placement.logical_size();
            spans.push((
                display.name().to_owned(),
                x,
                x + width,
                placement.scale().x(),
            ));
        }
        let close = OperationContext::new()
            .with_timeout(Duration::from_secs(5))
            .expect("a positive timeout");
        let _closed = session.close(&close);
    }

    spans.sort_by(|left, right| left.1.total_cmp(&right.1));
    println!("display arrangement, left to right:");
    for (name, near, far, scale) in &spans {
        println!("  {name}: x {near} to {far} at scale {scale}");
    }
    for pair in spans.windows(2) {
        let (left_name, _, left_far, left_scale) = &pair[0];
        let (right_name, right_near, _, right_scale) = &pair[1];
        if left_far == right_near && left_scale != right_scale {
            println!(
                "drag across x={left_far}, between {left_name} (scale {left_scale}) and \
                 {right_name} (scale {right_scale}) — that pair covers the move and the \
                 scale change together"
            );
        }
    }
    spans.iter().map(|(_, _, _, scale)| *scale).collect()
}

/// Reports where the named window sits now, by opening it briefly.
///
/// A watch that observed no move is ambiguous on its own: the window may never have
/// been dragged, or the stream may have stopped publishing when it was. Reading the
/// window's placement after the watch settles the first half from the host instead
/// of from the operator's memory. `None` means the reading itself did not succeed,
/// which is reported as exactly that rather than as the window not having moved.
fn placement_now(provider: &MacosCaptureProvider, name: &str) -> Option<Placed> {
    let context = OperationContext::new()
        .with_timeout(Duration::from_secs(5))
        .ok()?;
    let targets = provider.discover(&context).ok()?;
    let window = targets.iter().find(|target| {
        target.capability().kind() == Some(TargetKind::Window) && target.name() == name
    })?;
    let session = provider
        .open(
            window.id(),
            &OpenRequest::new().require_format(PixelFormat::Bgra8),
            &context,
        )
        .ok()?;
    let wait = OperationContext::new()
        .with_timeout(Duration::from_secs(5))
        .ok()?;
    let read = session
        .frame(&FrameRequest::latest(), &wait)
        .ok()
        .and_then(|frame| {
            let placement = frame.transform().target()?;
            let extent = frame.descriptor().extent();
            Some(Placed {
                origin: placement.desktop_origin(),
                scale: placement.scale().x(),
                extent: (extent.width(), extent.height()),
            })
        });
    let close = OperationContext::new()
        .with_timeout(Duration::from_secs(5))
        .ok()?;
    let _closed = session.close(&close);
    read
}

#[test]
fn a_window_moved_between_displays_republishes_under_a_new_geometry() {
    if std::env::var_os(PROBE_VARIABLE).is_none() {
        println!(
            "skipped: set {PROBE_VARIABLE}=1 and drag the captured window between \
             displays to exercise an in-stream geometry transition; \
             moving another application's window cannot be automated without \
             Accessibility authorization"
        );
        return;
    }

    let provider = MacosCaptureProvider::new(Arc::new(IdentityIssuer::new()));
    let context = OperationContext::new()
        .with_timeout(Duration::from_secs(10))
        .expect("a positive timeout");
    let targets = match provider.discover(&context) {
        Ok(targets) => targets,
        Err(error) => {
            println!(
                "skipped: discovery reported {error}; the probe needs a host that has \
                 granted Screen Recording to this process"
            );
            return;
        }
    };

    // Printed first so the run says which way to drag. A pair that is adjacent and
    // disagrees about scale is the one worth crossing, because it exercises the
    // scale half of the scenario as well as the move.
    let display_scales = report_display_arrangement(&provider, &targets);

    let windows: Vec<&TargetDescription> = targets
        .iter()
        .filter(|target| target.capability().kind() == Some(TargetKind::Window))
        .collect();
    if windows.is_empty() {
        println!("skipped: this host reports no window target");
        return;
    }
    println!("windows this probe can capture:");
    for (index, window) in windows.iter().enumerate() {
        println!(
            "  [{index}] {}x{} {}",
            window.extent().width(),
            window.extent().height(),
            window.name()
        );
    }

    let chosen = match std::env::var(TARGET_VARIABLE) {
        Ok(value) => {
            let value = value.trim();
            // An index is convenient but shifts as windows open and close between
            // runs, so a name fragment is accepted too and is the safer selector.
            match value.parse::<usize>() {
                Ok(index) => *windows
                    .get(index)
                    .expect("the index must name one of the windows listed above"),
                Err(_) => {
                    let needle = value.to_lowercase();
                    *windows
                        .iter()
                        .find(|window| window.name().to_lowercase().contains(&needle))
                        .expect("no window's name contains that fragment")
                }
            }
        }
        // The widest window is the one most likely to be a real application window
        // rather than a panel, and the easiest to grab and drag.
        Err(_) => windows
            .iter()
            .copied()
            .max_by_key(|window| window.extent().width())
            .expect("the list is not empty"),
    };
    println!(
        "capturing [{}] — drag it to another display within {} seconds",
        chosen.name(),
        WATCH.as_secs()
    );

    let session = provider
        .open(
            chosen.id(),
            &OpenRequest::new().require_format(PixelFormat::Bgra8),
            &context,
        )
        .expect("a discovered window opens");

    let mut states: Vec<Observed> = Vec::new();
    let mut stamp = None;
    let watch_from = Instant::now();
    let deadline = watch_from + WATCH;
    let mut last_heartbeat = watch_from;
    let mut frames = 0u32;
    let mut empty_waits = 0u32;
    let mut last_frame_at: Option<Instant> = None;

    while Instant::now() < deadline {
        // Printed before the frame request rather than after a successful one. A
        // heartbeat that needs a frame to arrive falls silent in exactly the case
        // worth reporting — a stream that has stopped publishing — and an earlier
        // run of this probe was read as an undragged window for that reason.
        if last_heartbeat.elapsed() >= HEARTBEAT {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let since = last_frame_at.map_or_else(
                || "none yet".to_owned(),
                |at| format!("{:.1}s ago", at.elapsed().as_secs_f64()),
            );
            println!(
                "  watching, {}s left, {} state(s), {frames} frame(s), last {since}",
                remaining.as_secs(),
                states.len(),
            );
            last_heartbeat = Instant::now();
        }

        let wait = OperationContext::new()
            .with_timeout(FRAME_WAIT)
            .expect("a positive timeout");
        let request = stamp.map_or_else(FrameRequest::latest, FrameRequest::newer_than);
        let frame = match session.frame(&request, &wait) {
            Ok(frame) => frame,
            Err(error) if error.status() == Status::DeadlineExceeded => {
                empty_waits += 1;
                continue;
            }
            Err(error) if error.status() == Status::TargetLost => {
                println!("the window closed while the probe was watching: {error}");
                break;
            }
            Err(error) => panic!("the window stream failed: {error}"),
        };
        frames += 1;
        last_frame_at = Some(Instant::now());

        let placement = frame
            .transform()
            .target()
            .expect("a window frame carries authoritative placement");
        // The conversion is checked against this frame's own placement on every
        // frame, which is the property the scenario is about: a retained frame's
        // coordinates come from its own geometry and not from live host state.
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
            "frame {} converted through geometry that is not its own",
            frame.stamp()
        );
        assert_eq!(
            frame.transform().geometry(),
            frame.stamp().geometry(),
            "a frame's transform and its stamp disagree about the geometry revision"
        );

        let extent = frame.descriptor().extent();
        let current = Observed {
            epoch: frame.stamp().epoch(),
            geometry: frame.stamp().geometry(),
            placed: Placed {
                origin: placement.desktop_origin(),
                scale: placement.scale().x(),
                extent: (extent.width(), extent.height()),
            },
            frames: 1,
        };
        match states.last_mut() {
            // Anything the Adapter treats as a transition is a change of placement,
            // so an unchanged reading only extends the state already recorded.
            Some(last) if last.placed == current.placed => last.frames += 1,
            _ => {
                println!(
                    "  observed: epoch {} geometry {} {}",
                    current.epoch, current.geometry, current.placed
                );
                states.push(current);
            }
        }
        stamp = Some(frame.stamp());
    }

    let quiet = last_frame_at.map(|at| at.elapsed());
    println!(
        "frames: {frames} received and {empty_waits} empty wait(s) over {:.1}s",
        watch_from.elapsed().as_secs_f64()
    );
    println!("states the stream published under, in order:");
    for state in &states {
        println!(
            "  epoch {} geometry {} {} over {} frame(s)",
            state.epoch, state.geometry, state.placed, state.frames
        );
    }

    let close = OperationContext::new()
        .with_timeout(Duration::from_secs(10))
        .expect("a positive timeout");
    session.close(&close).expect("close");

    // Read after the session closed, so the reading is not competing with the
    // stream it is being compared against.
    let live = placement_now(&provider, chosen.name());
    match live {
        Some(placed) => println!("the window is now at {placed}"),
        None => println!("the window's placement could not be read after the watch"),
    }

    // One state has two causes that call for opposite next steps: nothing was
    // dragged, or something was and the stream stopped publishing when it happened.
    // They are told apart here rather than left to whoever reads the output, since
    // naming the wrong one costs another manual run.
    if states.len() <= 1 {
        let moved_after_all = states
            .first()
            .zip(live)
            .is_some_and(|(state, live)| state.placed != live);
        if let Some(quiet) = quiet
            && quiet >= STALL
        {
            panic!(
                "the stream published {frames} frame(s) and then nothing for the last \
                 {:.1}s of the watch{}. A move during that silence could not be \
                 observed, so this is the stream having stopped rather than the \
                 window having stayed put",
                quiet.as_secs_f64(),
                if moved_after_all {
                    ", and the window is no longer where the last published frame put it"
                } else {
                    ""
                }
            );
        }
        assert!(
            !moved_after_all,
            "the window is not where the last published frame put it, yet the stream \
             published one state throughout: the move was never published"
        );
    }
    assert!(
        states.len() > 1,
        "the stream published under one state throughout while frames kept arriving, \
         so no transition was exercised — the window was not moved while the probe \
         watched"
    );

    // Each transition is classified by what actually changed, and the identity the
    // Adapter assigned is asserted against that rather than merely recorded. This is
    // where a transition that advanced the wrong counter would show up.
    let mut moved_at_one_scale = 0u32;
    let mut crossed_scales = 0u32;
    for pair in states.windows(2) {
        let (before, after) = (&pair[0], &pair[1]);
        assert!(
            after.geometry > before.geometry,
            "a transition from {before:?} to {after:?} did not advance the geometry revision"
        );
        if after.placed.extent == before.placed.extent {
            // Placement changed while the pixels stayed comparable. The contract
            // calls that a geometry change, and an epoch advance here would tell a
            // caller its frames are no longer comparable when they are.
            assert_eq!(
                after.epoch, before.epoch,
                "a move that kept the extent started a new epoch: {before:?} then {after:?}"
            );
            assert_ne!(
                after.placed.origin, before.placed.origin,
                "a state changed without the extent or the origin changing"
            );
            moved_at_one_scale += 1;
        } else {
            // The extent changed, so pixels are not comparable across it whatever
            // the Adapter claimed, and the epoch has to advance.
            assert!(
                after.epoch > before.epoch,
                "an extent change did not start a new epoch: {before:?} then {after:?}"
            );
            if after.placed.scale != before.placed.scale {
                crossed_scales += 1;
            }
        }
    }

    // A published scale belonging to no attached display means the target was scaled
    // down to fit a surface too small to hold it. What separates a benign transient
    // from the defect is not how many of them there are but whether they follow one
    // another: the window server resizes a window shortly after it lands on another
    // display, so for a frame or two the producer still holds the surface the target
    // needed before that, and each such state is followed by one at the target's own
    // scale. A surface request that adopted the reduction instead produces a run of
    // them that shrinks as it goes.
    //
    // Both numbers are reported because the total cannot tell those apart, and a total
    // on its own has been read as a regression when the longest run was one. How many
    // transients a run sees depends on the window's height when it happens to cross,
    // so the total is not comparable between runs and the longest run is.
    let reduced = |state: &Observed| {
        !display_scales
            .iter()
            .any(|scale| (scale - state.placed.scale).abs() < 1e-9)
    };
    let letterboxed = states.iter().filter(|state| reduced(state)).count();
    let mut longest_reduced_run = 0u32;
    let mut consecutive = 0u32;
    for state in &states {
        consecutive = if reduced(state) { consecutive + 1 } else { 0 };
        longest_reduced_run = longest_reduced_run.max(consecutive);
    }
    println!(
        "measured: {moved_at_one_scale} move(s) at one scale as a geometry change, \
         {crossed_scales} cross-scale move(s) as a new epoch, {letterboxed} state(s) \
         published at a scale no attached display has, the longest unbroken run of \
         those being {longest_reduced_run}"
    );
    if crossed_scales == 0 {
        println!(
            "noted: no move crossed displays of differing scale; drag the window across \
             the seam named above to cover that"
        );
    }
    if moved_at_one_scale == 0 {
        println!(
            "noted: no move kept the extent, so the geometry-change path was not \
             exercised; drag the window within one display to cover that"
        );
    }
    assert!(
        moved_at_one_scale + crossed_scales > 0,
        "states changed but none of them was a move"
    );
}
