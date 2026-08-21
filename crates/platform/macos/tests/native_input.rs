#![cfg_attr(not(target_os = "macos"), allow(missing_docs))]
#![cfg(target_os = "macos")]
//! Native macOS input checks against a real desktop.
//!
//! # What runs by default and what does not
//!
//! The default suite submits nothing: it exercises read-only native observations,
//! the provider's input surface, and refusals that happen before any event. Native
//! submission remains deliberate and ignored by default. One row exercises the
//! focused system route. The process-directed rows keep an unrelated owned
//! fixture frontmost, bind pointer qualification to an explicit display topology,
//! separate unrelated activity, and include a sustained-capture soak. Every
//! opt-in row is documented in `docs/macos-input-verification.md`.

#[allow(dead_code, unreachable_pub)]
#[path = "../src/fixture_control.rs"]
mod fixture_control;

#[allow(dead_code, unreachable_pub)]
#[path = "../src/fixture_protocol.rs"]
mod fixture_protocol;

use fixture_control::{
    AuthenticatedFixtureProcess, ExecutableIdentity, FixtureSocketDirectory,
    LaunchedFixtureApplication, authenticate_fixture_peer, executable_identity,
    next_fixture_run_nonce,
};
#[cfg(feature = "private-fixture")]
use fixture_protocol::select_unique_fixture;
use fixture_protocol::{
    EVENT_FLAGS_CHANGED, EVENT_KEY_DOWN, EVENT_KEY_UP, EVENT_POINTER_MOVE, EVENT_POINTER_PRESS,
    EVENT_POINTER_RELEASE, EVENT_POINTER_SCROLL, EventSummary, EventTotals,
    FIXTURE_CONTROL_VERSION, FixtureCommand, FixtureCommandKind, FixtureCommandResult, FixtureMode,
    FixtureReadyFacts, FixtureRenderer, FixtureSelectionError, MAX_READY_LINE_BYTES,
    MAX_RECORDED_EVENTS, event_payload_activity_tag, event_payload_fingerprint,
    fixture_ready_facts, fixture_title, format_command_line, frame_is_fixture_content,
    frame_is_replacement_content, parse_command_result_line, parse_event_line_for_run,
    with_confirmed_fixture_content,
};
use mado_pilot_capture::{
    CaptureProvider, CaptureSession, Frame, FrameRequest, OpenRequest, PixelFormat,
    TargetDescription,
};
use mado_pilot_core::{
    ActivityTag, CancellationToken, CapabilitySupport, CoordinateSpace, FrameStamp,
    GeometryRevision, IdentityIssuer, InputAddressScope, InputDelivery, InputOperationKind,
    OperationContext, PermissionKind, PermissionProbe, PermissionState, Point, Status,
    StreamCursor, SubmissionEvidence, TargetId, TargetKind,
};
use mado_pilot_input::{
    CleanupState, DeliveryPlan, FocusPolicy, GeometryPolicy, InputController, InputEvent,
    InputFault, InputOpenRequest, InputProvider, InputRequest, InputRequirement, InputSequence,
    Key, Modifier, PointerButton, PointerGeometry, SequenceOutcome,
};
use mado_pilot_platform_macos::{MacosCaptureProvider, MacosPermissionProbe, PROVIDER};
use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// How long the interactive check waits for a person to focus the fixture.
const FOCUS_WAIT: Duration = Duration::from_secs(15);
/// How long the fail-closed content gate waits for one authoritative frame.
const CONTENT_WAIT: Duration = Duration::from_secs(5);
/// Quiet period proving an asynchronous ScreenCaptureKit geometry has settled.
const GEOMETRY_SETTLE: Duration = Duration::from_millis(250);
/// Minimum continuous-capture interval for the route-wide sustained soak.
const SUSTAINED_CAPTURE_SOAK: Duration = Duration::from_secs(60);
/// How long the fixture is given to publish its ready line.
const READY_WAIT: Duration = Duration::from_secs(10);
/// How long the owned-window oracle allows the successor and terminal loss.
const REPLACEMENT_WAIT: Duration = Duration::from_secs(10);
/// Event capacity plus bounded ready, control, and lifecycle records.
const MAX_FIXTURE_OUTPUT_RECORDS: usize = MAX_RECORDED_EVENTS + 16;
/// Explicit selector binding a qualification run to one frozen display row.
const QUALIFICATION_TOPOLOGY_ENV: &str = "MADO_PILOT_MACOS_QUALIFICATION_TOPOLOGY";
/// Unique high-half correlation for every exact native qualification row.
static NEXT_QUALIFICATION_CORRELATION: AtomicU32 = AtomicU32::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QualificationTopology {
    Single,
    SameScale,
    MixedScale,
}

impl QualificationTopology {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "single" => Some(Self::Single),
            "same-scale" => Some(Self::SameScale),
            "mixed-scale" => Some(Self::MixedScale),
            _ => None,
        }
    }

    fn required() -> Self {
        let value = std::env::var(QUALIFICATION_TOPOLOGY_ENV).unwrap_or_else(|_| {
            panic!(
                "qualification requires {QUALIFICATION_TOPOLOGY_ENV}=\
                 <single|same-scale|mixed-scale>"
            )
        });
        Self::parse(&value).unwrap_or_else(|| {
            panic!(
                "{QUALIFICATION_TOPOLOGY_ENV} must be exactly single, same-scale, or mixed-scale; \
                 got {value:?}"
            )
        })
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::SameScale => "same-scale",
            Self::MixedScale => "mixed-scale",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct QualificationGeometry {
    origin: (f64, f64),
    logical: (f64, f64),
    backing: (u32, u32),
    scale: (f64, f64),
}

impl QualificationGeometry {
    fn mapping_is_consistent(self) -> bool {
        self.origin.0.is_finite()
            && self.origin.1.is_finite()
            && self.logical.0.is_finite()
            && self.logical.1.is_finite()
            && self.scale.0.is_finite()
            && self.scale.1.is_finite()
            && self.logical.0 > 0.0
            && self.logical.1 > 0.0
            && self.backing.0 > 0
            && self.backing.1 > 0
            && self.scale.0 > 0.0
            && self.scale.1 > 0.0
            && approximately(f64::from(self.backing.0) / self.logical.0, self.scale.0)
            && approximately(f64::from(self.backing.1) / self.logical.1, self.scale.1)
    }

    fn has_scale(self, scale: f64) -> bool {
        approximately(self.scale.0, scale) && approximately(self.scale.1, scale)
    }

    fn has_signed_origin(self) -> bool {
        self.origin.0 < 0.0 || self.origin.1 < 0.0
    }

    fn is_horizontally_adjacent_to(self, right: Self) -> bool {
        let vertical_overlap = self.origin.1 < right.origin.1 + right.logical.1
            && right.origin.1 < self.origin.1 + self.logical.1;
        approximately(self.origin.0 + self.logical.0, right.origin.0) && vertical_overlap
    }
}

#[derive(Clone, Copy, Debug)]
struct ObservedQualificationGeometry {
    geometry: QualificationGeometry,
    stamp: FrameStamp,
}

fn approximately(left: f64, right: f64) -> bool {
    (left - right).abs() <= 1.0e-6
}

fn validate_qualification_topology(
    expected: QualificationTopology,
    geometries: &[QualificationGeometry],
) -> Result<(), String> {
    if geometries.is_empty() {
        return Err("no display geometry was observed".to_owned());
    }
    if let Some(invalid) = geometries
        .iter()
        .copied()
        .find(|geometry| !geometry.mapping_is_consistent())
    {
        return Err(format!(
            "display geometry has inconsistent logical/backing scale: {invalid:?}"
        ));
    }

    let mut ordered = geometries.to_vec();
    ordered.sort_by(|left, right| left.origin.0.total_cmp(&right.origin.0));
    match expected {
        QualificationTopology::Single => {
            if ordered.len() != 1 || !ordered[0].has_scale(2.0) {
                return Err(format!(
                    "single requires exactly one 2x display, observed {ordered:?}"
                ));
            }
        }
        QualificationTopology::SameScale => {
            if ordered.len() != 2
                || ordered.iter().any(|geometry| !geometry.has_scale(2.0))
                || !ordered[0].is_horizontally_adjacent_to(ordered[1])
            {
                return Err(format!(
                    "same-scale requires exactly two horizontally adjacent 2x displays, \
                     observed {ordered:?}"
                ));
            }
        }
        QualificationTopology::MixedScale => {
            let mixed_signed_seam = ordered.windows(2).any(|pair| {
                pair[0].is_horizontally_adjacent_to(pair[1])
                    && ((pair[0].has_scale(1.0) && pair[1].has_scale(2.0))
                        || (pair[0].has_scale(2.0) && pair[1].has_scale(1.0)))
                    && (pair[0].has_signed_origin() || pair[1].has_signed_origin())
            });
            if ordered.len() < 2 || !mixed_signed_seam {
                return Err(format!(
                    "mixed-scale requires horizontally adjacent 2x/1x displays with a signed \
                     origin, observed {ordered:?}"
                ));
            }
        }
    }
    Ok(())
}

fn qualification_geometry(frame: &Frame) -> QualificationGeometry {
    let placement = frame
        .transform()
        .target()
        .expect("a native qualification frame carries target placement");
    let extent = frame.descriptor().extent();
    QualificationGeometry {
        origin: placement.desktop_origin(),
        logical: placement.logical_size(),
        backing: (extent.width(), extent.height()),
        scale: (placement.scale().x(), placement.scale().y()),
    }
}

fn observe_qualification_topology(
    provider: &MacosCaptureProvider,
    expected: QualificationTopology,
) -> Vec<ObservedQualificationGeometry> {
    let displays = discovered(provider)
        .expect("the qualifying host remains discoverable")
        .into_iter()
        .filter(|target| target.capability().kind() == Some(TargetKind::Display));
    let mut observed = Vec::new();
    for display in displays {
        let capture = CaptureProvider::open(
            provider,
            display.id(),
            &OpenRequest::new().require_format(PixelFormat::Bgra8),
            &bounded(CONTENT_WAIT),
        )
        .expect("each qualifying display opens for authoritative geometry");
        let frame = capture
            .frame(&FrameRequest::latest(), &bounded(CONTENT_WAIT))
            .expect("each qualifying display publishes authoritative geometry");
        observed.push(ObservedQualificationGeometry {
            geometry: qualification_geometry(&frame),
            stamp: frame.stamp(),
        });
        capture
            .close(&bounded(CONTENT_WAIT))
            .expect("qualification display capture closes");
    }
    observed.sort_by(|left, right| left.geometry.origin.0.total_cmp(&right.geometry.origin.0));
    let geometries: Vec<_> = observed.iter().map(|item| item.geometry).collect();
    validate_qualification_topology(expected, &geometries)
        .unwrap_or_else(|reason| panic!("{} topology refused: {reason}", expected.label()));
    for (ordinal, item) in observed.iter().enumerate() {
        println!(
            "qualification-topology={} display={} logical={}x{} backing={}x{} \
             origin=({},{}) scale={}x{} frame={:?}",
            expected.label(),
            ordinal,
            item.geometry.logical.0,
            item.geometry.logical.1,
            item.geometry.backing.0,
            item.geometry.backing.1,
            item.geometry.origin.0,
            item.geometry.origin.1,
            item.geometry.scale.0,
            item.geometry.scale.1,
            item.stamp,
        );
    }
    observed
}

fn display_for_window(
    displays: &[QualificationGeometry],
    window: QualificationGeometry,
) -> Result<usize, String> {
    let centre = (
        window.origin.0 + window.logical.0 / 2.0,
        window.origin.1 + window.logical.1 / 2.0,
    );
    let matches: Vec<_> = displays
        .iter()
        .enumerate()
        .filter(|(_, display)| {
            approximately(display.scale.0, window.scale.0)
                && approximately(display.scale.1, window.scale.1)
                && centre.0 >= display.origin.0
                && centre.0 < display.origin.0 + display.logical.0
                && centre.1 >= display.origin.1
                && centre.1 < display.origin.1 + display.logical.1
        })
        .map(|(index, _)| index)
        .collect();
    if matches.len() != 1 {
        return Err(format!(
            "retained window at {centre:?} with scale {:?} matched display indexes {matches:?}",
            window.scale
        ));
    }
    Ok(matches[0])
}

fn validate_window_topology(
    expected: QualificationTopology,
    displays: &[ObservedQualificationGeometry],
    visits: &[ObservedQualificationGeometry],
) -> Result<(), String> {
    let display_geometries: Vec<_> = displays.iter().map(|display| display.geometry).collect();
    validate_qualification_topology(expected, &display_geometries)
        .map_err(|reason| format!("display topology changed before traversal: {reason}"))?;
    if visits.is_empty() {
        return Err("no retained-window geometry was observed".to_owned());
    }
    if let Some(invalid) = visits
        .iter()
        .find(|visit| !visit.geometry.mapping_is_consistent())
    {
        return Err(format!(
            "retained-window geometry has inconsistent logical/backing scale: {invalid:?}"
        ));
    }
    if visits.windows(2).any(|pair| {
        pair[0].stamp.geometry() == pair[1].stamp.geometry()
            || !pair[0].stamp.is_same_stream(&pair[1].stamp)
    }) {
        return Err(format!(
            "inter-display traversal did not advance same-stream geometry: {visits:?}"
        ));
    }
    let display_visits: Vec<_> = visits
        .iter()
        .map(|visit| display_for_window(&display_geometries, visit.geometry))
        .collect::<Result<_, _>>()?;
    let closes_mixed_cycle = expected == QualificationTopology::MixedScale && displays.len() > 1;
    let expected_visits = displays.len() + usize::from(closes_mixed_cycle);
    if visits.len() != expected_visits {
        return Err(format!(
            "retained-window traversal visited {} geometries; expected {expected_visits} for \
             {} frozen displays",
            visits.len(),
            displays.len()
        ));
    }
    if closes_mixed_cycle && display_visits.first() != display_visits.last() {
        return Err(format!(
            "mixed-scale traversal did not close its display cycle: displays={display_visits:?}"
        ));
    }
    let mut covered_displays = display_visits.clone();
    covered_displays.sort_unstable();
    covered_displays.dedup();
    if covered_displays != (0..displays.len()).collect::<Vec<_>>() {
        return Err(format!(
            "retained-window traversal did not visit every frozen display: \
             displays={display_visits:?}"
        ));
    }

    match expected {
        QualificationTopology::Single => {
            if visits.len() != 1 || !visits[0].geometry.has_scale(2.0) || display_visits != [0] {
                return Err(format!(
                    "single retained-window traversal must stay on its one 2x display: \
                     visits={visits:?}, displays={display_visits:?}"
                ));
            }
        }
        QualificationTopology::SameScale => {
            if visits.len() != 2
                || visits.iter().any(|visit| !visit.geometry.has_scale(2.0))
                || display_visits[0] == display_visits[1]
            {
                return Err(format!(
                    "same-scale retained-window traversal must cross both adjacent 2x displays: \
                     visits={visits:?}, displays={display_visits:?}"
                ));
            }
        }
        QualificationTopology::MixedScale => {
            let crossed_scale = display_visits.windows(2).any(|pair| {
                let left = display_geometries[pair[0]];
                let right = display_geometries[pair[1]];
                ((left.has_scale(1.0) && right.has_scale(2.0))
                    || (left.has_scale(2.0) && right.has_scale(1.0)))
                    && (left.is_horizontally_adjacent_to(right)
                        || right.is_horizontally_adjacent_to(left))
            });
            if !crossed_scale
                || !visits
                    .iter()
                    .any(|visit| visit.geometry.has_signed_origin())
            {
                return Err(format!(
                    "mixed-scale retained-window traversal must cross an adjacent 2x/1x seam, \
                     cover every frozen display, and publish a signed target origin: \
                     visits={visits:?}, displays={display_visits:?}"
                ));
            }
        }
    }
    Ok(())
}

#[test]
fn qualification_topology_rows_cannot_substitute_for_one_another() {
    let geometry = |origin: (f64, f64), logical: (f64, f64), backing: (u32, u32), scale: f64| {
        QualificationGeometry {
            origin,
            logical,
            backing,
            scale: (scale, scale),
        }
    };
    let single = [geometry((0.0, 0.0), (1_512.0, 982.0), (3_024, 1_964), 2.0)];
    let same_scale = [
        geometry((-1_512.0, 0.0), (1_512.0, 982.0), (3_024, 1_964), 2.0),
        geometry((0.0, 0.0), (2_560.0, 1_440.0), (5_120, 2_880), 2.0),
    ];
    let mixed_scale = [
        geometry((-3_840.0, -720.0), (3_840.0, 2_160.0), (3_840, 2_160), 1.0),
        geometry((0.0, 0.0), (2_560.0, 1_440.0), (5_120, 2_880), 2.0),
    ];
    let restored_three_display_mixed = [
        mixed_scale[0],
        mixed_scale[1],
        geometry((2_560.0, 0.0), (1_512.0, 982.0), (3_024, 1_964), 2.0),
    ];

    assert_eq!(
        QualificationTopology::parse("single"),
        Some(QualificationTopology::Single)
    );
    assert_eq!(
        QualificationTopology::parse("same-scale"),
        Some(QualificationTopology::SameScale)
    );
    assert_eq!(
        QualificationTopology::parse("mixed-scale"),
        Some(QualificationTopology::MixedScale)
    );
    assert_eq!(QualificationTopology::parse("mixed"), None);
    assert!(validate_qualification_topology(QualificationTopology::Single, &single).is_ok());
    assert!(validate_qualification_topology(QualificationTopology::SameScale, &same_scale).is_ok());
    assert!(
        validate_qualification_topology(QualificationTopology::MixedScale, &mixed_scale).is_ok()
    );
    assert!(validate_qualification_topology(QualificationTopology::Single, &same_scale).is_err());
    assert!(
        validate_qualification_topology(QualificationTopology::SameScale, &mixed_scale).is_err()
    );
    assert!(
        validate_qualification_topology(QualificationTopology::MixedScale, &same_scale).is_err()
    );
    assert!(
        validate_qualification_topology(
            QualificationTopology::MixedScale,
            &restored_three_display_mixed,
        )
        .is_ok(),
        "a complete topology may include another display beyond the required mixed-scale seam"
    );
}

#[test]
fn window_topology_must_visit_each_frozen_display() {
    let geometry = |origin: (f64, f64), logical: (u32, u32), scale: u32| QualificationGeometry {
        origin,
        logical: (f64::from(logical.0), f64::from(logical.1)),
        backing: (
            logical.0.checked_mul(scale).expect("backing width"),
            logical.1.checked_mul(scale).expect("backing height"),
        ),
        scale: (f64::from(scale), f64::from(scale)),
    };
    let issuer = IdentityIssuer::new();
    let mut cursor = StreamCursor::new(issuer.issue_stream().expect("test stream"));
    let first_stamp = cursor
        .publish(GeometryRevision::FIRST)
        .expect("first geometry");
    let second_stamp = cursor
        .publish(GeometryRevision::FIRST.next().expect("second revision"))
        .expect("second geometry");
    let third_stamp = cursor
        .publish(
            second_stamp
                .geometry()
                .next()
                .expect("third geometry revision"),
        )
        .expect("third geometry");
    let displays = [
        ObservedQualificationGeometry {
            geometry: geometry((-1_512.0, 0.0), (1_512, 982), 2),
            stamp: first_stamp,
        },
        ObservedQualificationGeometry {
            geometry: geometry((0.0, 0.0), (2_560, 1_440), 2),
            stamp: second_stamp,
        },
    ];
    let visits = [
        ObservedQualificationGeometry {
            geometry: geometry((-1_400.0, 100.0), (400, 300), 2),
            stamp: first_stamp,
        },
        ObservedQualificationGeometry {
            geometry: geometry((100.0, 100.0), (400, 300), 2),
            stamp: second_stamp,
        },
    ];
    assert!(validate_window_topology(QualificationTopology::SameScale, &displays, &visits).is_ok());

    let same_display_visits = [
        visits[0],
        ObservedQualificationGeometry {
            geometry: geometry((-900.0, 200.0), (400, 300), 2),
            stamp: second_stamp,
        },
    ];
    assert!(
        validate_window_topology(
            QualificationTopology::SameScale,
            &displays,
            &same_display_visits,
        )
        .is_err(),
        "same-stream geometry revisions on one display do not prove seam traversal"
    );

    let mixed_displays = [
        ObservedQualificationGeometry {
            geometry: geometry((-3_840.0, 0.0), (3_840, 2_160), 1),
            stamp: first_stamp,
        },
        ObservedQualificationGeometry {
            geometry: geometry((0.0, 0.0), (2_560, 1_440), 2),
            stamp: second_stamp,
        },
    ];
    let mixed_visits = [
        ObservedQualificationGeometry {
            geometry: geometry((-3_600.0, 100.0), (400, 300), 1),
            stamp: first_stamp,
        },
        ObservedQualificationGeometry {
            geometry: geometry((100.0, 100.0), (400, 300), 2),
            stamp: second_stamp,
        },
        ObservedQualificationGeometry {
            geometry: geometry((-3_500.0, 100.0), (400, 300), 1),
            stamp: third_stamp,
        },
    ];
    assert!(
        validate_window_topology(
            QualificationTopology::MixedScale,
            &mixed_displays,
            &mixed_visits,
        )
        .is_ok(),
        "display adjacency, not unrelated window widths, proves the closed mixed-scale crossing"
    );
}

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

#[derive(Clone, Copy, Debug)]
struct ExpectedFixtureEvent {
    kind: u32,
    text_units: u32,
    payload_fingerprint: u64,
}

fn expected_native_event_type(kind: u32, button: u32, key_down: bool) -> u64 {
    match kind {
        EVENT_POINTER_MOVE => match button {
            u32::MAX => 5,
            0 => 6,
            1 => 7,
            2 => 27,
            _ => panic!("unsupported pointer button in the qualification oracle"),
        },
        EVENT_POINTER_PRESS => match button {
            0 => 1,
            1 => 3,
            2 => 25,
            _ => panic!("unsupported pointer button in the qualification oracle"),
        },
        EVENT_POINTER_RELEASE => match button {
            0 => 2,
            1 => 4,
            2 => 26,
            _ => panic!("unsupported pointer button in the qualification oracle"),
        },
        EVENT_POINTER_SCROLL => 22,
        EVENT_KEY_DOWN => {
            assert!(key_down);
            10
        }
        EVENT_KEY_UP => {
            assert!(!key_down);
            11
        }
        EVENT_FLAGS_CHANGED => 12,
        _ => panic!("unsupported event kind in the qualification oracle"),
    }
}

#[allow(clippy::too_many_arguments)]
fn expected_fixture_event(
    kind: u32,
    button: u32,
    click_state: u64,
    x: f64,
    y: f64,
    horizontal: i32,
    vertical: i32,
    key_code: u16,
    key_down: bool,
    flags: u64,
    text: &[u16],
) -> ExpectedFixtureEvent {
    let native_type = expected_native_event_type(kind, button, key_down);
    let native_button = if matches!(
        kind,
        EVENT_POINTER_MOVE | EVENT_POINTER_PRESS | EVENT_POINTER_RELEASE
    ) && button != u32::MAX
    {
        u64::from(button)
    } else {
        0
    };
    let text = if key_code == 0 { text } else { &[] };
    let text_units = u32::try_from(text.len()).expect("fixture text length fits u32");
    let payload_fingerprint = event_payload_fingerprint(
        kind,
        native_type,
        flags,
        native_button,
        click_state,
        x,
        y,
        i64::from(horizontal),
        i64::from(vertical),
        u64::from(key_code),
        text,
    );
    ExpectedFixtureEvent {
        kind,
        text_units,
        payload_fingerprint,
    }
}

fn qualification_operation(
    expected: &[ExpectedFixtureEvent],
    duration: Duration,
) -> (OperationContext, u32) {
    let correlation = NEXT_QUALIFICATION_CORRELATION.fetch_add(1, Ordering::Relaxed);
    assert_ne!(correlation, 0, "qualification row correlation exhausted");
    let fingerprints = expected
        .iter()
        .map(|event| event.payload_fingerprint)
        .collect::<Vec<_>>();
    let activity_tag = event_payload_activity_tag(correlation, &fingerprints);
    let operation = OperationContext::new()
        .with_activity_tag(ActivityTag::new(activity_tag).expect("row activity tag is nonzero"))
        .with_timeout(duration)
        .expect("the qualification row timeout is positive");
    (operation, correlation)
}

fn refresh_qualification_deadline(
    setup: &OperationContext,
    duration: Duration,
) -> OperationContext {
    OperationContext::new()
        .with_activity_tag(
            setup
                .activity_tag()
                .expect("the qualification setup carries its private row token"),
        )
        .with_timeout(duration)
        .expect("the qualification row timeout is positive")
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
        .issue_target(PROVIDER)
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
fn delay_only_focus_policies_probe_unfocused_windows_without_posting_input() {
    // A default test must never turn an ambient desktop window into an input
    // target. Probe focus with a reversible delay-only sequence, and accept a
    // complete result only as evidence that this particular window was focused.
    let provider = provider();
    let Some(targets) = discovered(&provider) else {
        println!("skipped: this host offers no capture capability");
        return;
    };
    let windows = targets
        .iter()
        .filter(|target| target.capability().kind() == Some(TargetKind::Window))
        .collect::<Vec<_>>();
    if windows.is_empty() {
        println!("skipped: this host presented no shareable window");
        return;
    }

    for policy in [FocusPolicy::Preserve, FocusPolicy::RequireFocused] {
        let mut observed_refusal = false;
        for window in &windows {
            let controller =
                InputProvider::open(&provider, window.id(), &InputOpenRequest::new(), &context())
                    .expect("an optional input open succeeds for a window");
            let request = InputRequest::new(
                window.id(),
                InputSequence::new(vec![InputEvent::Delay(Duration::ZERO)]).expect("valid"),
                DeliveryPlan::require(InputDelivery::System),
            )
            .with_focus(policy);
            let receipt = controller
                .execute(&request, &context())
                .expect("the reversible focus probe produces a receipt");
            controller.close(&context()).expect("close");

            match receipt.fault() {
                Some(InputFault::FocusRequired) => {
                    assert_eq!(receipt.outcome(), SequenceOutcome::Unexecuted);
                    assert_eq!(receipt.submitted(), 0);
                    assert_eq!(receipt.attempts().len(), 1);
                    assert_eq!(receipt.attempts()[0].route(), InputDelivery::System);
                    observed_refusal = true;
                    break;
                }
                Some(InputFault::NotAuthorized | InputFault::TargetLost) => {
                    assert_eq!(receipt.submitted(), 0);
                }
                None => {
                    assert_eq!(receipt.outcome(), SequenceOutcome::Complete);
                    assert_eq!(
                        receipt.submitted(),
                        1,
                        "only the reversible delay completed"
                    );
                }
                fault => panic!("unexpected focus-probe result for {policy:?}: {fault:?}"),
            }
        }
        if !observed_refusal {
            println!("skipped: no observable unfocused window for {policy:?}");
        }
    }
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
        .issue_target(PROVIDER)
        .expect("issued by this engine for this provider");

    let error = InputProvider::describe(&own, absent, &context())
        .expect_err("an accepted identity that was never discovered is not live");

    assert_eq!(error.status(), Status::TargetLost);
}

fn ready_process_id_for_peer(line: &str, authenticated_process_id: u32) -> Option<u32> {
    let reported_process_id = ready_process_id(line)?;
    (reported_process_id == authenticated_process_id).then_some(reported_process_id)
}

/// A fixture launch plus the audit-token-bound application it owns.
enum FixtureLauncher {
    Workspace(LaunchedFixtureApplication),
    Child(Child),
}

impl FixtureLauncher {
    fn process_id(&self) -> u32 {
        match self {
            Self::Workspace(application) => application.process_id(),
            Self::Child(child) => child.id(),
        }
    }

    fn exited(&mut self) -> bool {
        match self {
            Self::Workspace(application) => application.is_live().is_ok_and(|live| !live),
            Self::Child(child) => child.try_wait().ok().flatten().is_some(),
        }
    }

    fn is_live(&self) -> bool {
        match self {
            Self::Workspace(application) => application.is_live() == Ok(true),
            Self::Child(_) => true,
        }
    }

    fn liveness_is_known(&self) -> bool {
        match self {
            Self::Workspace(application) => application.is_live().is_ok(),
            Self::Child(_) => true,
        }
    }

    fn terminate(&mut self) {
        if let Self::Workspace(application) = self {
            let _terminated = application.terminate();
        }
    }

    fn kill(&mut self) {
        match self {
            Self::Workspace(application) => {
                let _killed = application.kill();
            }
            Self::Child(child) => {
                let _killed = child.kill();
            }
        }
    }
}

struct FixtureChild {
    launcher: FixtureLauncher,
    application: Option<AuthenticatedFixtureProcess>,
}

impl FixtureChild {
    fn new(child: Child) -> Self {
        Self {
            launcher: FixtureLauncher::Child(child),
            application: None,
        }
    }

    fn from_launched(application: LaunchedFixtureApplication) -> Self {
        Self {
            launcher: FixtureLauncher::Workspace(application),
            application: None,
        }
    }

    fn process_id(&self) -> u32 {
        self.launcher.process_id()
    }

    fn exited(&mut self) -> bool {
        self.launcher.exited()
    }
}

impl Drop for FixtureChild {
    fn drop(&mut self) {
        if self.exited() {
            return;
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        if let Some(application) = self.application.as_mut() {
            let _terminated = application.terminate();
        }
        self.launcher.terminate();
        let term_deadline = deadline.min(Instant::now() + Duration::from_millis(100));
        while Instant::now() < term_deadline {
            if self.exited() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        if let Some(application) = self.application.as_mut() {
            let _killed = application.kill();
        }
        self.launcher.kill();
        while Instant::now() < deadline {
            if self.exited() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

enum ReaderMessage {
    Line(String),
    Oversized,
    Failed,
}

fn fixture_protocol_line(message: ReaderMessage) -> String {
    match message {
        ReaderMessage::Line(line) => line,
        ReaderMessage::Oversized => panic!("the fixture emitted an oversized protocol record"),
        ReaderMessage::Failed => panic!("the fixture output protocol failed"),
    }
}

struct FixtureReadyExpectation {
    require_signed_bundle: bool,
    mode: FixtureMode,
    run_nonce: u64,
    wait: Duration,
}

/// A running fixture with an owned command channel and bounded output channel.
struct Fixture {
    child: FixtureChild,
    input: Option<UnixStream>,
    lines: Arc<Mutex<Receiver<ReaderMessage>>>,
    process_id: u32,
    facts: FixtureReadyFacts,
    run_nonce: u64,
    next_nonce: u64,
    pending_events: VecDeque<EventSummary>,
    stopped: bool,
    expected_executable: Option<PathBuf>,
    expected_executable_bytes: Option<Arc<[u8]>>,
    expected_identity: Option<ExecutableIdentity>,
}

impl Fixture {
    /// Starts the ordinary fixture and waits for its ready record.
    fn start() -> Option<Self> {
        Self::start_active(FixtureMode::Default)
    }

    /// Starts one visible fixture as the foreground application.
    fn start_active(mode: FixtureMode) -> Option<Self> {
        match mode {
            FixtureMode::Default => Self::start_with_arguments(&[], FixtureMode::Default),
            FixtureMode::GameLike => {
                Self::start_with_arguments(&["--game-like"], FixtureMode::GameLike)
            }
        }
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

    /// Starts the independently identified foreground fixture bundle.
    fn start_foreground() -> Option<Self> {
        let executable = PathBuf::from(std::env::var_os(
            "MADO_PILOT_MACOS_FOREGROUND_FIXTURE_EXECUTABLE",
        )?);
        executable.is_file().then_some(())?;
        Self::start_executable_with_arguments(executable, &[], FixtureMode::Default, false)
    }

    fn start_with_arguments(arguments: &[&str], expected_mode: FixtureMode) -> Option<Self> {
        let executable = fixture_executable()?;
        let require_signed_bundle =
            std::env::var_os("MADO_PILOT_MACOS_FIXTURE_EXECUTABLE").is_some();
        Self::start_executable_with_arguments(
            executable,
            arguments,
            expected_mode,
            require_signed_bundle,
        )
    }

    fn start_executable_with_arguments(
        executable: PathBuf,
        arguments: &[&str],
        expected_mode: FixtureMode,
        require_signed_bundle: bool,
    ) -> Option<Self> {
        let expected_executable = std::fs::canonicalize(&executable).ok()?;
        let expected_executable_bytes: Arc<[u8]> = std::fs::read(&expected_executable).ok()?.into();
        let expected_identity = executable_identity(&expected_executable)
            .inspect_err(|error| eprintln!("fixture code identity: {error}"))
            .ok()?;
        let bundle = fixture_bundle(&executable)?;
        let socket_directory = FixtureSocketDirectory::new().ok()?;
        let socket_path = socket_directory.socket_path();
        let listener = UnixListener::bind(&socket_path).ok()?;
        listener.set_nonblocking(true).ok()?;
        let run_nonce = next_fixture_run_nonce().ok()?;
        let mut launch_arguments = vec![
            OsString::from("--control-socket"),
            socket_path.as_os_str().to_owned(),
            OsString::from("--run-nonce"),
            OsString::from(run_nonce.to_string()),
        ];
        launch_arguments.extend(arguments.iter().map(OsString::from));
        let argument_views = launch_arguments
            .iter()
            .map(OsString::as_os_str)
            .collect::<Vec<&OsStr>>();
        let launched = LaunchedFixtureApplication::launch(&bundle, &argument_views).ok()?;
        let mut child = FixtureChild::from_launched(launched);
        let mut expected_process_id = child.process_id();
        let mut launch_attempts = 1_u32;
        let deadline = Instant::now() + READY_WAIT;
        let (stream, authenticated_process) = loop {
            if child.exited() {
                if launch_attempts >= 3 || Instant::now() >= deadline {
                    return None;
                }
                eprintln!(
                    "fixture-launch-retry attempt={} reason=exited-before-control-connection",
                    launch_attempts + 1
                );
                let launched = LaunchedFixtureApplication::launch(&bundle, &argument_views).ok()?;
                child = FixtureChild::from_launched(launched);
                expected_process_id = child.process_id();
                launch_attempts += 1;
                continue;
            }
            if Instant::now() >= deadline {
                return None;
            }
            match listener.accept() {
                Ok((stream, _address)) => {
                    if let Some(process) = authenticate_fixture_peer(
                        &stream,
                        expected_process_id,
                        &expected_executable,
                    ) {
                        let identity_matches = loop {
                            if !child.launcher.is_live() {
                                break None;
                            }
                            match process.executable_identity() {
                                Ok(identity) => break Some(identity == expected_identity),
                                Err(_) if Instant::now() >= deadline => return None,
                                Err(_) => thread::sleep(Duration::from_millis(25)),
                            }
                        };
                        match identity_matches {
                            Some(true) => {
                                child.application = Some(process);
                                break (stream, process);
                            }
                            Some(false) => return None,
                            None => continue,
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => return None,
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
            Some((
                expected_executable,
                expected_executable_bytes,
                expected_identity,
            )),
            FixtureReadyExpectation {
                require_signed_bundle,
                mode: expected_mode,
                run_nonce,
                wait: READY_WAIT,
            },
        )
    }

    fn from_child(
        child: FixtureChild,
        input: UnixStream,
        authenticated_process: AuthenticatedFixtureProcess,
        expected_provenance: Option<(PathBuf, Arc<[u8]>, ExecutableIdentity)>,
        expectation: FixtureReadyExpectation,
    ) -> Option<Self> {
        let FixtureReadyExpectation {
            require_signed_bundle,
            mode: expected_mode,
            run_nonce,
            wait: ready_wait,
        } = expectation;
        let lines = spawn_reader(input.try_clone().ok()?);
        let line = match lines.recv_timeout(ready_wait).ok()? {
            ReaderMessage::Line(line) if line.starts_with("fixture-ready ") => line,
            ReaderMessage::Line(_) | ReaderMessage::Oversized | ReaderMessage::Failed => {
                return None;
            }
        };
        let process_id = ready_process_id_for_peer(&line, authenticated_process.process_id())?;
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
            "the fixture initialized a renderer other than the requested one"
        );
        if require_signed_bundle {
            assert!(
                facts.execution_context_is_approved(),
                "a configured fixture must truthfully report the stable signed bundle \
                 context before any input path opens"
            );
        }
        println!(
            "fixture-ready-approved mode={:?} renderer={:?} execution-context-approved={}",
            facts.mode(),
            facts.renderer(),
            facts.execution_context_is_approved()
        );
        let (expected_executable, expected_executable_bytes, expected_identity) =
            expected_provenance.map_or((None, None, None), |(path, bytes, identity)| {
                (Some(path), Some(bytes), Some(identity))
            });
        Some(Self {
            child,
            input: Some(input),
            lines: Arc::new(Mutex::new(lines)),
            stopped: false,
            process_id,
            run_nonce,
            facts,
            next_nonce: 1,
            pending_events: VecDeque::new(),
            expected_executable,
            expected_executable_bytes,
            expected_identity,
        })
    }

    fn authenticated_process(&self) -> Option<AuthenticatedFixtureProcess> {
        self.input.as_ref()?;
        let process = self.child.application?;
        let expected_identity = self.expected_identity?;
        (self.child.launcher.is_live()
            && process.matches_live_owner(i64::from(self.process_id))
            && process.matches_executable_identity(expected_identity))
        .then_some(process)
    }

    fn executable_provenance_unchanged(&self, wait: Duration) -> bool {
        let (Some(path), Some(expected_bytes), Some(expected_identity)) = (
            self.expected_executable.as_ref(),
            self.expected_executable_bytes.as_ref(),
            self.expected_identity,
        ) else {
            return true;
        };
        let artifact_unchanged = std::fs::read(path)
            .is_ok_and(|bytes| bytes == expected_bytes.as_ref())
            && executable_identity(path).is_ok_and(|identity| identity == expected_identity);
        if !artifact_unchanged || !self.child.launcher.liveness_is_known() {
            return false;
        }
        let deadline = Instant::now() + wait;
        loop {
            if !self.child.launcher.is_live() {
                return true;
            }
            let Some(process) = self.child.application else {
                return false;
            };
            match process.executable_identity() {
                Ok(identity) => return identity == expected_identity,
                Err(_) if Instant::now() >= deadline => return false,
                Err(_) => thread::sleep(Duration::from_millis(25)),
            }
        }
    }

    fn replacement_result(&mut self, wait: Duration) -> Option<(u32, u64, u64)> {
        let line = self.wait_for_line(wait, |line| line.starts_with("fixture-replaced "))?;
        let (run_nonce, status, old_window, new_window) = parse_replacement_line(&line)?;
        println!(
            "fixture-replacement-observed success={}",
            status == 0 && old_window != 0 && new_window != 0
        );
        (run_nonce == self.run_nonce).then_some((status, old_window, new_window))
    }

    fn command(
        &mut self,
        kind: FixtureCommandKind,
        wait: Duration,
    ) -> Option<FixtureCommandResult> {
        self.command_with_event_payload_tag(kind, 0, wait)
    }

    fn command_with_event_payload_tag(
        &mut self,
        kind: FixtureCommandKind,
        event_payload_tag: u64,
        wait: Duration,
    ) -> Option<FixtureCommandResult> {
        let nonce = self.next_nonce;
        self.next_nonce = self.next_nonce.checked_add(1)?;
        self.command_with_nonce(
            FixtureCommand {
                run_nonce: self.run_nonce,
                nonce,
                event_payload_tag,
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
        if matches!(
            command.kind,
            FixtureCommandKind::ResetEvents | FixtureCommandKind::PrepareLanguageFlow
        ) && result.status == 0
        {
            // Event lines emitted before the reset acknowledgement belong to
            // the previous observation interval. The native snapshot remains
            // authoritative and later reads still expose any post-reset event.
            self.pending_events.clear();
        }
        if command.kind == FixtureCommandKind::Stop && result.status == 0 {
            self.stopped = true;
        }
        println!(
            "fixture-command-observed action={} success={}",
            command.kind.as_str(),
            result.status == 0
        );
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
                Ok(message) => {
                    let line = fixture_protocol_line(message);
                    if accept(&line) {
                        return Some(line);
                    }
                    if let Some(summary) = parse_event_line_for_run(&line, self.run_nonce) {
                        assert!(
                            self.pending_events.len() < MAX_RECORDED_EVENTS,
                            "the fixture emitted more queued events than the protocol permits"
                        );
                        self.pending_events.push_back(summary);
                    } else {
                        panic!("the fixture returned an unexpected protocol record");
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
                Ok(message) => {
                    let line = fixture_protocol_line(message);
                    let summary = parse_event_line_for_run(&line, self.run_nonce)
                        .unwrap_or_else(|| panic!("unexpected fixture event record"));
                    kinds.push(summary.kind);
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
                Ok(message) => {
                    let line = fixture_protocol_line(message);
                    let summary = parse_event_line_for_run(&line, self.run_nonce)
                        .unwrap_or_else(|| panic!("unexpected fixture event record"));
                    events.push(summary);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        events
    }

    /// Changes fixture window state after the exact press, then cancels.
    ///
    /// This closes the race that matters for cleanup: the bounded release must
    /// use retained process authority even after ordinary window admission
    /// becomes unavailable.
    fn move_offscreen_and_cancel_after_event(
        &mut self,
        expected: EventSummary,
        cancellation: CancellationToken,
        wait: Duration,
    ) -> thread::JoinHandle<(Option<EventSummary>, Option<FixtureCommandResult>)> {
        assert!(
            self.pending_events.is_empty(),
            "stale fixture events precede the cleanup-state transition row"
        );
        {
            let lines = self
                .lines
                .lock()
                .expect("the fixture output receiver is not poisoned");
            assert!(
                matches!(lines.try_recv(), Err(mpsc::TryRecvError::Empty)),
                "queued fixture output precedes the cleanup-state transition row"
            );
        }

        let mut input = self
            .input
            .as_ref()
            .expect("the fixture control channel remains connected")
            .try_clone()
            .expect("the fixture control channel is clonable");
        let command = FixtureCommand {
            run_nonce: self.run_nonce,
            nonce: self.next_nonce,
            event_payload_tag: 0,
            kind: FixtureCommandKind::MoveOffscreen,
        };
        self.next_nonce = self
            .next_nonce
            .checked_add(1)
            .expect("the bounded fixture command nonce does not overflow");
        let lines = Arc::clone(&self.lines);
        let run_nonce = self.run_nonce;
        thread::Builder::new()
            .name("mado-pilot-native-input-cleanup-transition".to_owned())
            .spawn(move || {
                let deadline = Instant::now() + wait;
                let lines = lines
                    .lock()
                    .expect("the fixture output receiver is not poisoned");
                let mut observed = None;
                while Instant::now() < deadline {
                    match lines.recv_timeout(Duration::from_millis(25)) {
                        Ok(message) => {
                            let line = fixture_protocol_line(message);
                            let summary = parse_event_line_for_run(&line, run_nonce)
                                .unwrap_or_else(|| panic!("unexpected fixture event record"));
                            observed = Some(summary);
                            if summary == expected {
                                writeln!(input, "{}", format_command_line(command))
                                    .expect("the off-screen cleanup command is writable");
                                input
                                    .flush()
                                    .expect("the off-screen cleanup command is flushed");
                                break;
                            }
                            return (observed, None);
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => {
                            cancellation.cancel();
                            return (observed, None);
                        }
                    }
                }

                let mut result = None;
                while Instant::now() < deadline {
                    match lines.recv_timeout(Duration::from_millis(25)) {
                        Ok(message) => {
                            let line = fixture_protocol_line(message);
                            if let Some(candidate) = parse_command_result_line(&line)
                                && candidate.run_nonce == command.run_nonce
                                && candidate.nonce == command.nonce
                            {
                                result = Some(candidate);
                                break;
                            }
                            panic!("unexpected fixture cleanup-transition record");
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
                cancellation.cancel();
                (observed, result)
            })
            .expect("the cleanup-state transition helper starts")
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

    fn begin_correlated_event_row(&mut self, event_payload_tag: u64, wait: Duration) {
        assert_ne!(event_payload_tag, 0, "a correlated row token is nonzero");
        assert!(
            self.pending_events.is_empty(),
            "a prior row left unconsumed fixture events: {:?}",
            self.pending_events
        );
        let reset = self
            .command_with_event_payload_tag(
                FixtureCommandKind::ResetEvents,
                event_payload_tag,
                wait,
            )
            .expect("the fixture resets its correlated event summary");
        assert_eq!(reset.status, 0, "the fixture event reset succeeds");
        assert_eq!(reset.events, EventTotals::default());
        assert!(
            self.pending_events.is_empty(),
            "the fixture reset exposed events left by a prior row: {:?}",
            self.pending_events
        );
    }

    fn begin_operation_event_row(&mut self, operation: &OperationContext, wait: Duration) {
        let event_payload_tag = operation
            .activity_tag()
            .expect("a qualification operation carries a private row token")
            .get();
        self.begin_correlated_event_row(event_payload_tag, wait);
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

    fn expect_exact_events(
        &mut self,
        expected: &[ExpectedFixtureEvent],
        correlation: u32,
        wait: Duration,
    ) {
        let observed = self.exact_event_summaries(expected.len(), wait);
        let expected_summaries = expected
            .iter()
            .map(|event| EventSummary {
                kind: event.kind,
                text_units: event.text_units,
                correlation,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            observed, expected_summaries,
            "event kinds, lengths, and row correlation must match exactly"
        );
        let result = self
            .command(FixtureCommandKind::ReadEvents, wait)
            .expect("the fixture reads its exact event payload report");
        assert_eq!(result.status, 0);
        assert_eq!(result.events, event_totals(&observed));
        assert_eq!(result.event_correlation, correlation);
        assert!(
            result.event_payload_matches,
            "the privacy-safe observed payload digest differs from the submitted row"
        );
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
        let provenance_unchanged = self.executable_provenance_unchanged(CONTENT_WAIT);
        if !self.stopped {
            self.stopped = self
                .command(FixtureCommandKind::Stop, CONTENT_WAIT)
                .is_some_and(|result| result.status == 0);
        }
        self.input = None;
        let deadline = Instant::now() + CONTENT_WAIT;
        let mut exited = self.child.exited();
        while !exited && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(25));
            exited = self.child.exited();
        }
        if !(provenance_unchanged && self.stopped && exited) && !thread::panicking() {
            panic!(
                "native qualification fixture teardown failed: \
                 provenance={provenance_unchanged} stopped={} exited={exited}",
                self.stopped
            );
        }
    }
}

fn spawn_reader(mut input: impl Read + Send + 'static) -> Receiver<ReaderMessage> {
    let (sender, receiver) = mpsc::sync_channel(MAX_FIXTURE_OUTPUT_RECORDS);
    thread::spawn(move || {
        let mut line = Vec::with_capacity(MAX_READY_LINE_BYTES);
        let mut byte = [0u8; 1];
        let mut overflow = false;
        loop {
            match input.read(&mut byte) {
                Ok(0) => {
                    let terminal = if overflow {
                        Some(ReaderMessage::Oversized)
                    } else if line.is_empty() {
                        None
                    } else {
                        Some(ReaderMessage::Failed)
                    };
                    if let Some(message) = terminal {
                        let _sent = sender.try_send(message);
                    }
                    break;
                }
                Ok(_) if byte[0] == b'\n' => {
                    if overflow {
                        let _sent = sender.try_send(ReaderMessage::Oversized);
                        break;
                    }
                    let Ok(decoded) = String::from_utf8(std::mem::take(&mut line)) else {
                        let _sent = sender.try_send(ReaderMessage::Failed);
                        break;
                    };
                    if sender.try_send(ReaderMessage::Line(decoded)).is_err() {
                        break;
                    }
                    line = Vec::with_capacity(MAX_READY_LINE_BYTES);
                }
                Ok(_) if line.len() < MAX_READY_LINE_BYTES - 1 => line.push(byte[0]),
                Ok(_) => overflow = true,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => {
                    let _sent = sender.try_send(ReaderMessage::Failed);
                    break;
                }
            }
        }
    });
    receiver
}

#[test]
fn fixture_output_reader_rejects_an_overlong_record() {
    let (mut output, input) = UnixStream::pair().expect("the private test channel opens");
    let lines = spawn_reader(input);
    output
        .write_all(&vec![b'x'; MAX_READY_LINE_BYTES + 1])
        .expect("the oversized record is written");
    drop(output);

    assert!(matches!(
        lines.recv_timeout(Duration::from_secs(1)),
        Ok(ReaderMessage::Oversized)
    ));
}

#[test]
fn fixture_output_reader_rejects_invalid_utf8_and_unterminated_records() {
    for malformed in [vec![0xFF, b'\n'], b"unterminated".to_vec()] {
        let (mut output, input) = UnixStream::pair().expect("the private test channel opens");
        let lines = spawn_reader(input);
        output
            .write_all(&malformed)
            .expect("the malformed record is written");
        drop(output);
        assert!(matches!(
            lines.recv_timeout(Duration::from_secs(1)),
            Ok(ReaderMessage::Failed)
        ));
    }
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
fn ready_record_pid_must_match_the_authenticated_peer() {
    let line = format!("fixture-ready title={} pid=42 remainder", fixture_title(42));
    assert_eq!(ready_process_id_for_peer(&line, 42), Some(42));
    assert_eq!(ready_process_id_for_peer(&line, 43), None);
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
    let process_id = child.process_id();
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
            AuthenticatedFixtureProcess::for_test(process_id),
            None,
            FixtureReadyExpectation {
                require_signed_bundle: true,
                mode: FixtureMode::Default,
                run_nonce: 77,
                wait: Duration::from_secs(2),
            },
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

fn discover_unique_fixture(
    provider: &MacosCaptureProvider,
    fixture: &Fixture,
    wait: Duration,
) -> Result<TargetDescription, FixtureSelectionError> {
    #[cfg(not(feature = "private-fixture"))]
    {
        let _ = (provider, fixture, wait);
        Err(FixtureSelectionError::NotFound)
    }

    #[cfg(feature = "private-fixture")]
    {
        let started = Instant::now();
        loop {
            let process = fixture
                .authenticated_process()
                .ok_or(FixtureSelectionError::NotFound)?;
            let targets = discovered(provider).ok_or(FixtureSelectionError::NotFound)?;
            match select_unique_fixture(&targets, process.process_id(), |target| {
                provider.fixture_target_has_authenticated_owner(target, |owner| {
                    process.matches_live_owner(owner)
                })
            }) {
                Ok(target) => return Ok(target.clone()),
                Err(_) if started.elapsed() < wait => {
                    thread::sleep(Duration::from_millis(25));
                }
                Err(error) => return Err(error),
            }
        }
    }
}

#[cfg(feature = "private-fixture")]
fn authenticated_fixture_window_ids(
    provider: &MacosCaptureProvider,
    fixture: &Fixture,
) -> Vec<TargetId> {
    let Some(process) = fixture.authenticated_process() else {
        return Vec::new();
    };
    discovered(provider)
        .unwrap_or_default()
        .into_iter()
        .filter(|target| target.capability().kind() == Some(TargetKind::Window))
        .filter(|target| {
            provider.fixture_target_has_authenticated_owner(target.id(), |owner| {
                process.matches_live_owner(owner)
            })
        })
        .map(|target| target.id())
        .collect()
}

#[cfg(feature = "private-fixture")]
fn require_auxiliary_fixture_windows(
    provider: &MacosCaptureProvider,
    fixture: &Fixture,
    wait: Duration,
) {
    let deadline = Instant::now() + wait;
    loop {
        let window_ids = authenticated_fixture_window_ids(provider, fixture);
        if window_ids
            .split_first()
            .is_some_and(|(first, rest)| rest.iter().any(|other| other != first))
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the acknowledged auxiliary window never appeared in production discovery"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(not(feature = "private-fixture"))]
fn require_auxiliary_fixture_windows(
    _provider: &MacosCaptureProvider,
    _fixture: &Fixture,
    _wait: Duration,
) {
    panic!("auxiliary-window inventory proof requires the `private-fixture` feature");
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
    let chosen = discover_unique_fixture(&provider, &fixture, CONTENT_WAIT)
        .expect("exactly one approved fixture becomes discoverable");

    assert_eq!(chosen.name(), fixture_title(fixture.process_id));
    assert_eq!(chosen.capability().kind(), Some(TargetKind::Window));
    let yielded = fixture
        .command(FixtureCommandKind::YieldForeground, CONTENT_WAIT)
        .expect("foreground ownership is returned after discovery");
    assert_eq!(yielded.status, 0);
    assert_eq!(yielded.before_window, yielded.after_window);
}

#[test]
#[ignore = "opens two real fixture windows on an interactive desktop"]
fn fixture_launcher_owns_distinct_same_bundle_instances() {
    assert!(
        std::env::var_os("MADO_PILOT_MACOS_FIXTURE_EXECUTABLE").is_some(),
        "multi-instance verification requires the configured signed fixture bundle"
    );
    let first = Fixture::start().expect("the first owned fixture starts");
    let second = Fixture::start_with_arguments(&["--animate-on-input"], FixtureMode::Default)
        .expect("the second animated owned fixture starts");

    assert_ne!(first.process_id, second.process_id);
    assert!(first.authenticated_process().is_some());
    assert!(second.authenticated_process().is_some());
}

#[test]
#[ignore = "opens a real animated fixture window on an interactive desktop"]
fn fixture_launcher_passes_animated_mode_arguments() {
    assert!(
        std::env::var_os("MADO_PILOT_MACOS_FIXTURE_EXECUTABLE").is_some(),
        "animated launch verification requires the configured signed fixture bundle"
    );
    let fixture = Fixture::start_with_arguments(&["--animate-on-input"], FixtureMode::Default)
        .expect("the animated owned fixture starts");

    assert!(fixture.authenticated_process().is_some());
}

#[test]
#[ignore = "opens and inventories two real fixture windows on an interactive desktop"]
fn auxiliary_window_acknowledgement_requires_two_production_inventory_windows() {
    assert!(
        std::env::var_os("MADO_PILOT_MACOS_FIXTURE_EXECUTABLE").is_some(),
        "auxiliary-window verification requires the configured signed fixture bundle"
    );
    let mut fixture = Fixture::start().expect("the owned fixture starts");
    let provider = provider();
    let opened = fixture
        .command(FixtureCommandKind::OpenAuxiliary, CONTENT_WAIT)
        .expect("the auxiliary command is acknowledged");
    assert_eq!(opened.status, 0);

    require_auxiliary_fixture_windows(&provider, &fixture, CONTENT_WAIT);
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
                event_payload_tag: 0,
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
                event_payload_tag: 0,
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
                event_payload_tag: 0,
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
                event_payload_tag: 0,
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
        if fixture.child.exited() {
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
    let original = discover_unique_fixture(&provider, &fixture, CONTENT_WAIT)
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

    let replacement = discover_unique_fixture(&provider, &fixture, CONTENT_WAIT)
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
    let chosen = discover_unique_fixture(&provider, &fixture, CONTENT_WAIT)
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
        "Click the controlled MadoPilot fixture window within {} seconds.",
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

#[test]
#[ignore = "attempts signed-fixture activation and delivers real system input on success"]
fn system_activation_never_redirects_to_unrelated_foreground() {
    assert!(
        std::env::var_os("MADO_PILOT_MACOS_FIXTURE_EXECUTABLE").is_some(),
        "activation verification requires the configured signed fixture bundle"
    );
    assert!(
        std::env::var_os("MADO_PILOT_MACOS_FOREGROUND_FIXTURE_EXECUTABLE").is_some(),
        "activation verification requires the unrelated signed foreground fixture"
    );
    assert!(
        post_event_access_granted(),
        "this check needs post-event access granted to the test process"
    );

    let mut foreground_fixture = Fixture::start_foreground()
        .expect("the unrelated foreground fixture starts from its independent bundle");
    let foreground_before = wait_until_frontmost_fixture(&foreground_fixture);
    let mut fixture = Fixture::start_inactive(FixtureMode::Default)
        .expect("the target fixture starts without taking foreground ownership");
    assert_eq!(
        frontmost_application(),
        Some(foreground_before),
        "the inactive target launch must preserve the unrelated foreground fixture"
    );

    let provider = provider();
    let chosen = discover_unique_fixture(&provider, &fixture, CONTENT_WAIT)
        .expect("the retained target is selected exactly once");
    let capture = CaptureProvider::open(
        &provider,
        chosen.id(),
        &OpenRequest::new().require_format(PixelFormat::Bgra8),
        &bounded(CONTENT_WAIT),
    )
    .expect("the retained target opens for capture");
    let frame = capture
        .frame(&FrameRequest::latest(), &bounded(CONTENT_WAIT))
        .expect("the retained target publishes a frame");
    let mapping = frame
        .map(PixelFormat::Bgra8, &bounded(CONTENT_WAIT))
        .expect("the retained target frame maps");
    let mapped = mapping.descriptor();
    let input =
        with_confirmed_fixture_content(mapping.bytes(), mapped.stride(), mapped.extent(), || {
            InputProvider::open(
                &provider,
                chosen.id(),
                &InputOpenRequest::new()
                    .with_requirement(InputRequirement::Required)
                    .requiring(InputOperationKind::Keyboard, InputDelivery::System),
                &bounded(CONTENT_WAIT),
            )
        })
        .expect("the retained target matches the controlled fixture pixels")
        .expect("system input opens for the retained target");

    fixture.begin_event_row(CONTENT_WAIT);
    foreground_fixture.begin_event_row(CONTENT_WAIT);
    let cursor_before = pointer_location();
    let receipt = input
        .execute(
            &InputRequest::new(
                chosen.id(),
                InputSequence::new(vec![
                    InputEvent::KeyPress(Key::Enter),
                    InputEvent::KeyRelease(Key::Enter),
                ])
                .expect("the activation key pair is balanced"),
                DeliveryPlan::require(InputDelivery::System),
            )
            .with_focus(FocusPolicy::ActivateIfRequired),
            &bounded(CONTENT_WAIT),
        )
        .expect("activation and system delivery return a receipt");
    match receipt.outcome() {
        SequenceOutcome::Complete => {
            assert_eq!(receipt.submitted(), 2);
            assert_eq!(receipt.selected_route(), Some(InputDelivery::System));
            assert_eq!(
                wait_until_frontmost_fixture(&fixture),
                fixture.process_id,
                "successful activation must use the retained target's process lifetime"
            );
            fixture.expect_event_kinds(&[EVENT_KEY_DOWN, EVENT_KEY_UP], CONTENT_WAIT);
            assert_eq!(
                foreground_fixture.event_totals(CONTENT_WAIT),
                EventTotals::default(),
                "system input reached the previous foreground application"
            );
        }
        SequenceOutcome::Unexecuted => {
            assert_eq!(receipt.submitted(), 0);
            assert_eq!(receipt.fault(), Some(InputFault::FocusRefused));
            assert_eq!(receipt.attempts().len(), 1);
            assert_eq!(receipt.attempts()[0].route(), InputDelivery::System);
            assert_eq!(
                frontmost_application(),
                Some(foreground_before),
                "a refused activation changed foreground ownership"
            );
            assert_eq!(fixture.event_totals(CONTENT_WAIT), EventTotals::default());
            assert_eq!(
                foreground_fixture.event_totals(CONTENT_WAIT),
                EventTotals::default(),
                "a refused activation posted into the foreground process"
            );
        }
        outcome => panic!("unexpected activation outcome {outcome:?}: {receipt}"),
    }
    assert_eq!(
        pointer_location(),
        cursor_before,
        "keyboard activation moved the physical cursor"
    );

    input.close(&bounded(CONTENT_WAIT)).expect("input closes");
    capture
        .close(&bounded(CONTENT_WAIT))
        .expect("capture closes");
    let stopped = fixture
        .command(FixtureCommandKind::Stop, CONTENT_WAIT)
        .expect("target fixture stop is acknowledged");
    assert_eq!(stopped.status, 0);
    let foreground_stopped = foreground_fixture
        .command(FixtureCommandKind::Stop, CONTENT_WAIT)
        .expect("foreground fixture stop is acknowledged");
    assert_eq!(foreground_stopped.status, 0);
    fixture.input = None;
    foreground_fixture.input = None;
}

fn frontmost_application() -> Option<u32> {
    let mut process_id = 0;
    // SAFETY: the production shim writes one `u32` and contains native exceptions.
    let status = unsafe { mp_shim_input_frontmost_process(&raw mut process_id) };
    (status == 0 && process_id != 0).then_some(process_id)
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

fn post_untagged_process_key_pair(process_id: u32) {
    let process_id = i32::try_from(process_id).expect("fixture process id fits pid_t");
    for key_down in [true, false] {
        // SAFETY: Core Graphics accepts a null source, returns one retained event,
        // and `CGEventPostToPid` borrows it only for the duration of the call.
        unsafe {
            let event = CGEventCreateKeyboardEvent(std::ptr::null(), 0x24, key_down);
            assert!(
                !event.is_null(),
                "the unrelated source creates its keyboard event"
            );
            CGEventPostToPid(process_id, event);
            CFRelease(event.cast_const());
        }
    }
}
#[track_caller]
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

#[track_caller]
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
fn wait_for_process_unavailable(
    provider: &MacosCaptureProvider,
    target: TargetId,
    kind: InputOperationKind,
    wait: Duration,
) {
    let deadline = Instant::now() + wait;
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match InputProvider::describe(provider, target, &bounded(remaining)) {
            Ok(descriptor)
                if descriptor
                    .capability()
                    .pair(kind, InputDelivery::ProcessDirected)
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
    panic!("the unavailable process target remained input-eligible past the scenario deadline");
}

fn wait_for_process_available(
    provider: &MacosCaptureProvider,
    target: TargetId,
    kind: InputOperationKind,
    wait: Duration,
) {
    let deadline = Instant::now() + wait;
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match InputProvider::describe(provider, target, &bounded(remaining)) {
            Ok(descriptor)
                if descriptor
                    .capability()
                    .pair(kind, InputDelivery::ProcessDirected)
                    .support()
                    == CapabilitySupport::Unknown =>
            {
                return;
            }
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.status(),
                    Status::TargetLost | Status::DeadlineExceeded
                ) => {}
            Err(error) => panic!("process capability refresh failed: {error}"),
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("the restored process target remained unavailable past the scenario deadline");
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
    foreground_before: &u32,
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

#[track_caller]
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

#[derive(Debug, Clone, Copy)]
struct ControlledVisualObservation {
    stamp: FrameStamp,
    replacement_fill: bool,
}

#[track_caller]
fn observe_tagged_input_transition(
    capture: &dyn CaptureSession,
    observation: &mut ControlledVisualObservation,
) {
    observation.replacement_fill = !observation.replacement_fill;
    observation.stamp =
        observe_fixture_fill(capture, observation.stamp, observation.replacement_fill);
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
    assert!(
        extent.width() > 2 && extent.height() > 2,
        "the qualification frame has interior endpoint coordinates"
    );
    let capture_centre = Point::new(
        CoordinateSpace::CapturePixels,
        f64::from(extent.width()) / 2.0,
        f64::from(extent.height()) / 2.0,
    )
    .expect("the frame centre is finite");
    let capture_leading = Point::new(CoordinateSpace::CapturePixels, 1.0, capture_centre.y())
        .expect("the leading frame endpoint is finite");
    let capture_trailing = Point::new(
        CoordinateSpace::CapturePixels,
        f64::from(extent.width() - 1),
        capture_centre.y(),
    )
    .expect("the trailing frame endpoint is finite");
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
    let leading = frame
        .transform()
        .convert_point(capture_leading, space)
        .unwrap_or_else(|error| panic!("{space} leading endpoint conversion failed: {error}"));
    let trailing = frame
        .transform()
        .convert_point(capture_trailing, space)
        .unwrap_or_else(|error| panic!("{space} trailing endpoint conversion failed: {error}"));
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
            "leading seam endpoint",
            vec![InputEvent::PointerMove(leading)],
            vec![EVENT_POINTER_MOVE],
        ),
        (
            "trailing seam endpoint",
            vec![InputEvent::PointerMove(trailing)],
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

fn fixture_native_button(button: PointerButton) -> u32 {
    match button {
        PointerButton::Primary => 0,
        PointerButton::Secondary => 1,
        PointerButton::Middle => 2,
        _ => panic!("unsupported button cannot form a positive qualification row"),
    }
}

fn expected_native_pointer_location(frame: &Frame, point: Point) -> (f64, f64) {
    let desktop = frame
        .transform()
        .convert_point(point, CoordinateSpace::DesktopLogical)
        .expect("the expected pointer point converts to desktop logical");
    let geometry = qualification_geometry(frame);
    let snap = |value: f64, origin: f64, scale: f64| {
        let snapped = origin + ((value - origin) * scale).round() / scale;
        if snapped == 0.0 { 0.0 } else { snapped }
    };
    (
        snap(desktop.x(), geometry.origin.0, geometry.scale.0),
        snap(desktop.y(), geometry.origin.1, geometry.scale.1),
    )
}

fn expected_process_pointer_events(
    frame: &Frame,
    events: &[InputEvent],
) -> Vec<ExpectedFixtureEvent> {
    let mut location = None;
    let mut active_button = None;
    events
        .iter()
        .map(|event| match event {
            InputEvent::PointerMove(point) => {
                let (x, y) = expected_native_pointer_location(frame, *point);
                location = Some((x, y));
                expected_fixture_event(
                    EVENT_POINTER_MOVE,
                    active_button.unwrap_or(u32::MAX),
                    u64::from(active_button.is_some()),
                    x,
                    y,
                    0,
                    0,
                    0,
                    false,
                    0,
                    &[],
                )
            }
            InputEvent::PointerPress(button) => {
                let native_button = fixture_native_button(*button);
                let (x, y) = location.expect("a positive press row first positions the pointer");
                active_button = Some(native_button);
                expected_fixture_event(
                    EVENT_POINTER_PRESS,
                    native_button,
                    1,
                    x,
                    y,
                    0,
                    0,
                    0,
                    false,
                    0,
                    &[],
                )
            }
            InputEvent::PointerRelease(button) => {
                let native_button = fixture_native_button(*button);
                assert_eq!(
                    active_button,
                    Some(native_button),
                    "a positive release balances its row-owned press"
                );
                let (x, y) = location.expect("a positive release row has a pointer location");
                active_button = None;
                expected_fixture_event(
                    EVENT_POINTER_RELEASE,
                    native_button,
                    1,
                    x,
                    y,
                    0,
                    0,
                    0,
                    false,
                    0,
                    &[],
                )
            }
            InputEvent::PointerScroll {
                horizontal,
                vertical,
            } => {
                let (x, y) = location.expect("a positive scroll row first positions the pointer");
                expected_fixture_event(
                    EVENT_POINTER_SCROLL,
                    u32::MAX,
                    0,
                    x,
                    y,
                    -i32::from(*horizontal),
                    -i32::from(*vertical),
                    0,
                    false,
                    0,
                    &[],
                )
            }
            _ => panic!("a pointer qualification row contains only pointer events"),
        })
        .collect()
}
const CG_EVENT_FLAG_MASK_SHIFT: u64 = 0x0002_0000;
const CG_EVENT_FLAG_MASK_SECONDARY_FN: u64 = 0x0080_0000;

const QUALIFICATION_A_TEXT: [u16; 1] = [0x0061];
fn qualification_key_code(key: Key) -> u16 {
    match key {
        Key::Character('a') => 0x00,
        Key::Character('b') => 0x0B,
        Key::Modifier(Modifier::Shift) => 0x38,
        Key::Enter => 0x24,
        Key::Function(1) => 0x7A,
        Key::ArrowRight => 0x7C,
        _ => panic!("the positive qualification matrix has no native oracle for {key:?}"),
    }
}
fn qualification_delivered_flags(key: Key, requested_flags: u64) -> u64 {
    if matches!(key, Key::Function(_) | Key::ArrowRight) {
        requested_flags | CG_EVENT_FLAG_MASK_SECONDARY_FN
    } else {
        requested_flags
    }
}

fn expected_process_keyboard_events(events: &[InputEvent]) -> Vec<ExpectedFixtureEvent> {
    let mut flags = 0;
    events
        .iter()
        .map(|event| match event {
            InputEvent::KeyPress(key) => {
                if *key == Key::Modifier(Modifier::Shift) {
                    flags |= CG_EVENT_FLAG_MASK_SHIFT;
                }
                expected_fixture_event(
                    if matches!(key, Key::Modifier(_)) {
                        EVENT_FLAGS_CHANGED
                    } else {
                        EVENT_KEY_DOWN
                    },
                    u32::MAX,
                    0,
                    0.0,
                    0.0,
                    0,
                    0,
                    qualification_key_code(*key),
                    true,
                    qualification_delivered_flags(*key, flags),
                    if *key == Key::Character('a') {
                        &QUALIFICATION_A_TEXT
                    } else {
                        &[]
                    },
                )
            }
            InputEvent::KeyRelease(key) => {
                if *key == Key::Modifier(Modifier::Shift) {
                    flags &= !CG_EVENT_FLAG_MASK_SHIFT;
                }
                expected_fixture_event(
                    if matches!(key, Key::Modifier(_)) {
                        EVENT_FLAGS_CHANGED
                    } else {
                        EVENT_KEY_UP
                    },
                    u32::MAX,
                    0,
                    0.0,
                    0.0,
                    0,
                    0,
                    qualification_key_code(*key),
                    false,
                    qualification_delivered_flags(*key, flags),
                    if *key == Key::Character('a') {
                        &QUALIFICATION_A_TEXT
                    } else {
                        &[]
                    },
                )
            }
            _ => panic!("a keyboard qualification row contains only key events"),
        })
        .collect()
}

fn expected_process_text_events(text: &str) -> Vec<ExpectedFixtureEvent> {
    const CHUNK_UNITS: usize = 16;
    let units = text.encode_utf16().collect::<Vec<_>>();
    let mut events = Vec::new();
    let mut start = 0;
    while start < units.len() {
        let mut end = (start + CHUNK_UNITS).min(units.len());
        if end < units.len() && (0xD800..0xDC00).contains(&units[end - 1]) {
            end -= 1;
        }
        let chunk = &units[start..end];
        events.push(expected_fixture_event(
            EVENT_KEY_DOWN,
            u32::MAX,
            0,
            0.0,
            0.0,
            0,
            0,
            0,
            true,
            0,
            chunk,
        ));
        events.push(expected_fixture_event(
            EVENT_KEY_UP,
            u32::MAX,
            0,
            0.0,
            0.0,
            0,
            0,
            0,
            false,
            0,
            chunk,
        ));
        start = end;
    }
    events
}

#[allow(clippy::too_many_arguments)]
fn exercise_process_pointer_rows(
    input: &dyn InputController,
    target: TargetId,
    frame: &Frame,
    geometry: PointerGeometry,
    capture: &dyn CaptureSession,
    visual: &mut ControlledVisualObservation,
    fixture: &mut Fixture,
    foreground_fixture: &mut Fixture,
    foreground_before: &u32,
) {
    for space in QUALIFICATION_POINTER_SPACES {
        let rows = pointer_qualification_rows(frame, space);
        for (row_index, base_row) in rows.into_iter().enumerate() {
            // Reprojection resolves against authority at each operation, so its
            // independent frame oracle must advance with a settling target too.
            let current_frame = if geometry.policy() == GeometryPolicy::ReprojectCurrent {
                Some(
                    capture
                        .frame(&FrameRequest::latest(), &bounded(CONTENT_WAIT))
                        .expect("reprojected input has a current capture-frame oracle"),
                )
            } else {
                None
            };
            let (label, events, expected_kinds) = if let Some(current) = &current_frame {
                pointer_qualification_rows(current, space)
                    .into_iter()
                    .nth(row_index)
                    .expect("the pointer qualification row set is stable")
            } else {
                base_row
            };
            let oracle_frame = current_frame.as_ref().unwrap_or(frame);
            let expected = expected_process_pointer_events(oracle_frame, &events);
            assert_eq!(
                expected.iter().map(|event| event.kind).collect::<Vec<_>>(),
                expected_kinds,
                "the independent native oracle preserves the declared event sequence"
            );
            let (operation, correlation) = qualification_operation(&expected, CONTENT_WAIT);
            fixture.begin_operation_event_row(&operation, CONTENT_WAIT);
            foreground_fixture.begin_event_row(CONTENT_WAIT);
            let cursor_before = pointer_location();
            let submitted = events.len();
            let operation = refresh_qualification_deadline(&operation, CONTENT_WAIT);
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
                    &operation,
                )
                .unwrap_or_else(|error| {
                    panic!(
                        "{label}/{space}/{} process posting failed: {error}",
                        geometry.policy()
                    )
                });
            assert_process_receipt(&receipt, submitted);
            fixture.expect_exact_events(&expected, correlation, CONTENT_WAIT);
            assert_unrelated_desktop_state(
                fixture,
                foreground_fixture,
                foreground_before,
                cursor_before,
            );
            observe_tagged_input_transition(capture, visual);
        }
    }
}

fn wait_for_geometry_frame(
    capture: &dyn CaptureSession,
    after: FrameStamp,
    replacement: bool,
    context_label: &str,
) -> Frame {
    let deadline = Instant::now() + CONTENT_WAIT;
    let mut cursor = after;
    let mut frame = loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "capture did not publish changed geometry before the scenario deadline"
        );
        let frame = capture
            .frame(&FrameRequest::newer_than(cursor), &bounded(remaining))
            .expect("capture publishes while geometry changes");
        cursor = frame.stamp();
        if frame.stamp().geometry() != after.geometry() {
            break frame;
        }
    };
    let mut geometry = qualification_geometry(&frame);
    let mut unchanged_since = Instant::now();
    while unchanged_since.elapsed() < GEOMETRY_SETTLE {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "capture geometry did not settle before the scenario deadline"
        );
        thread::sleep(
            remaining
                .min(Duration::from_millis(25))
                .min(GEOMETRY_SETTLE.saturating_sub(unchanged_since.elapsed())),
        );
        let latest = capture
            .frame(&FrameRequest::latest(), &bounded(remaining))
            .expect("capture retains the latest geometry while it settles");
        let latest_geometry = qualification_geometry(&latest);
        if latest_geometry != geometry {
            geometry = latest_geometry;
            unchanged_since = Instant::now();
        }
        frame = latest;
    }
    assert_fixture_frame_content(&frame, replacement, context_label);
    frame
}

fn assert_stale_pointer_frame_refused(
    input: &dyn InputController,
    target: TargetId,
    stale_frame: &Frame,
    fixture: &mut Fixture,
    foreground_fixture: &mut Fixture,
    foreground_before: &u32,
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

fn exercise_process_require_focused_success_row(
    input: &dyn InputController,
    target: TargetId,
    capture: &dyn CaptureSession,
    visual: &mut ControlledVisualObservation,
    fixture: &mut Fixture,
) {
    assert_eq!(
        frontmost_application(),
        Some(fixture.process_id),
        "the positive RequireFocused row starts with the target in front"
    );
    let _ = fixture.summaries(Duration::from_millis(250));
    let events = vec![
        InputEvent::KeyPress(Key::Enter),
        InputEvent::KeyRelease(Key::Enter),
    ];
    let expected = expected_process_keyboard_events(&events);
    let (operation, correlation) = qualification_operation(&expected, CONTENT_WAIT);
    fixture.begin_operation_event_row(&operation, CONTENT_WAIT);
    let cursor_before = pointer_location();
    let submitted = events.len();
    let receipt = input
        .execute(
            &InputRequest::new(
                target,
                InputSequence::new(events).expect("the focused key pair is balanced"),
                DeliveryPlan::require(InputDelivery::ProcessDirected),
            )
            .with_focus(FocusPolicy::RequireFocused),
            &refresh_qualification_deadline(&operation, CONTENT_WAIT),
        )
        .expect("the focused RequireFocused process request succeeds");
    assert_process_receipt(&receipt, submitted);
    fixture.expect_exact_events(&expected, correlation, CONTENT_WAIT);
    assert_eq!(
        frontmost_application(),
        Some(fixture.process_id),
        "RequireFocused changed the already-focused target application"
    );
    assert_eq!(
        pointer_location(),
        cursor_before,
        "the focused keyboard row moved the physical cursor"
    );
    observe_tagged_input_transition(capture, visual);
}

fn exercise_process_require_focused_refusal_row(
    input: &dyn InputController,
    target: TargetId,
    fixture: &mut Fixture,
    foreground_fixture: &mut Fixture,
    foreground_before: &u32,
) {
    assert_eq!(
        frontmost_application(),
        Some(*foreground_before),
        "the refusal row starts with the unrelated fixture in front"
    );
    let _ = fixture.summaries(Duration::from_millis(250));
    let _ = foreground_fixture.summaries(Duration::from_millis(250));
    let events = vec![
        InputEvent::KeyPress(Key::Enter),
        InputEvent::KeyRelease(Key::Enter),
    ];
    let expected = expected_process_keyboard_events(&events);
    let (operation, _) = qualification_operation(&expected, CONTENT_WAIT);
    fixture.begin_operation_event_row(&operation, CONTENT_WAIT);
    foreground_fixture.begin_event_row(CONTENT_WAIT);
    let cursor_before = pointer_location();
    let receipt = input
        .execute(
            &InputRequest::new(
                target,
                InputSequence::new(events).expect("the focus probe key pair is balanced"),
                DeliveryPlan::require(InputDelivery::ProcessDirected),
            )
            .with_focus(FocusPolicy::RequireFocused),
            &refresh_qualification_deadline(&operation, CONTENT_WAIT),
        )
        .expect("an inactive RequireFocused process request returns a receipt");
    assert_zero_effect(&receipt, InputFault::FocusRequired);
    assert_eq!(fixture.event_totals(CONTENT_WAIT), EventTotals::default());
    assert!(
        fixture
            .event_summaries(1, Duration::from_millis(150))
            .is_empty(),
        "the inactive RequireFocused process request reached the target fixture"
    );
    assert_unrelated_desktop_state(
        fixture,
        foreground_fixture,
        foreground_before,
        cursor_before,
    );
}

fn qualify_process_require_focused_success(mode: FixtureMode) {
    let mut fixture = Fixture::start_active(mode)
        .expect("the focused-row fixture starts as the foreground application");
    assert_eq!(
        wait_until_frontmost_fixture(&fixture),
        fixture.process_id,
        "the positive focus row requires the target's launch activation"
    );
    let provider = provider();
    let chosen = discover_unique_fixture(&provider, &fixture, CONTENT_WAIT)
        .expect("the focused-row child exposes one uniquely selected fixture window");
    let capture = CaptureProvider::open(
        &provider,
        chosen.id(),
        &OpenRequest::new().require_format(PixelFormat::Bgra8),
        &bounded(CONTENT_WAIT),
    )
    .expect("capture opens for the focused-row fixture");
    let first = capture
        .frame(&FrameRequest::latest(), &bounded(CONTENT_WAIT))
        .expect("the focused-row fixture publishes its initial frame");
    assert_fixture_frame_content(&first, false, "focused-row initial frame");
    let mut visual = ControlledVisualObservation {
        stamp: first.stamp(),
        replacement_fill: false,
    };
    let input = InputProvider::open(
        &provider,
        chosen.id(),
        &InputOpenRequest::new()
            .with_requirement(InputRequirement::Required)
            .requiring(InputOperationKind::Keyboard, InputDelivery::ProcessDirected),
        &bounded(CONTENT_WAIT),
    )
    .expect("the focused process-directed keyboard pair opens");
    exercise_process_require_focused_success_row(
        input.as_ref(),
        chosen.id(),
        capture.as_ref(),
        &mut visual,
        &mut fixture,
    );
    input.close(&bounded(CONTENT_WAIT)).expect("input closes");
    capture
        .close(&bounded(CONTENT_WAIT))
        .expect("focused-row capture closes");
    let stopped = fixture
        .command(FixtureCommandKind::Stop, CONTENT_WAIT)
        .expect("the focused-row fixture stop is acknowledged");
    assert_eq!(stopped.status, 0);
    fixture.input = None;
}

fn qualify_process_directed_renderer(
    mode: FixtureMode,
    expected_topology: QualificationTopology,
    observed_topology: &[ObservedQualificationGeometry],
) {
    qualify_process_require_focused_success(mode);
    let mut foreground_fixture = Fixture::start_foreground()
        .expect("the unrelated foreground fixture starts from its independent bundle");
    let foreground_before = wait_until_frontmost_fixture(&foreground_fixture);
    let mut fixture = Fixture::start_inactive(mode)
        .expect("the owned fixture starts visible without taking foreground ownership");
    assert_ne!(
        foreground_before, fixture.process_id,
        "the qualification target must remain inactive"
    );

    let provider = provider();
    let chosen = discover_unique_fixture(&provider, &fixture, CONTENT_WAIT)
        .expect("the owned child exposes one uniquely selected fixture window");
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
    let auxiliary = fixture
        .command(FixtureCommandKind::OpenAuxiliary, CONTENT_WAIT)
        .expect("the additional ordinary window opens");
    assert_eq!(auxiliary.status, 0);
    require_auxiliary_fixture_windows(&provider, &fixture, CONTENT_WAIT);
    thread::sleep(Duration::from_secs(10));
    let multiple_window_target = discover_unique_fixture(&provider, &fixture, CONTENT_WAIT)
        .expect("the retained primary remains discoverable with an ordinary sibling");
    for kind in InputOperationKind::ALL {
        assert_eq!(
            multiple_window_target
                .capability()
                .input()
                .pair(kind, InputDelivery::ProcessDirected)
                .support(),
            CapabilitySupport::Unknown,
            "sustained capture plus an additional window revoked process-directed {}",
            kind.as_str()
        );
    }
    let mut visual = ControlledVisualObservation {
        stamp: first.stamp(),
        replacement_fill: false,
    };

    let inactive_descriptor = InputProvider::describe(
        &provider,
        multiple_window_target.id(),
        &bounded(CONTENT_WAIT),
    )
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

    exercise_process_require_focused_refusal_row(
        input.as_ref(),
        chosen.id(),
        &mut fixture,
        &mut foreground_fixture,
        &foreground_before,
    );

    exercise_process_pointer_rows(
        input.as_ref(),
        chosen.id(),
        &first,
        PointerGeometry::require_unchanged_since(first.stamp()),
        capture.as_ref(),
        &mut visual,
        &mut fixture,
        &mut foreground_fixture,
        &foreground_before,
    );

    let moved = fixture
        .command(FixtureCommandKind::Move, CONTENT_WAIT)
        .expect("the local movement command completes");
    assert_eq!(moved.status, 0);
    let moved_frame = wait_for_geometry_frame(
        capture.as_ref(),
        visual.stamp,
        visual.replacement_fill,
        "moved target",
    );
    visual.stamp = moved_frame.stamp();
    assert_stale_pointer_frame_refused(
        input.as_ref(),
        chosen.id(),
        &first,
        &mut fixture,
        &mut foreground_fixture,
        &foreground_before,
    );
    drop(first);
    exercise_process_pointer_rows(
        input.as_ref(),
        chosen.id(),
        &moved_frame,
        PointerGeometry::reprojected(),
        capture.as_ref(),
        &mut visual,
        &mut fixture,
        &mut foreground_fixture,
        &foreground_before,
    );
    drop(moved_frame);
    let moved_current = capture
        .frame(&FrameRequest::latest(), &bounded(CONTENT_WAIT))
        .expect("the moved fixture keeps publishing");
    exercise_process_pointer_rows(
        input.as_ref(),
        chosen.id(),
        &moved_current,
        PointerGeometry::require_unchanged_since(moved_current.stamp()),
        capture.as_ref(),
        &mut visual,
        &mut fixture,
        &mut foreground_fixture,
        &foreground_before,
    );

    let resized = fixture
        .command(FixtureCommandKind::Resize, CONTENT_WAIT)
        .expect("the resize command completes");
    assert_eq!(resized.status, 0);
    let resized_frame = wait_for_geometry_frame(
        capture.as_ref(),
        visual.stamp,
        visual.replacement_fill,
        "resized target",
    );
    visual.stamp = resized_frame.stamp();
    assert_stale_pointer_frame_refused(
        input.as_ref(),
        chosen.id(),
        &moved_current,
        &mut fixture,
        &mut foreground_fixture,
        &foreground_before,
    );
    drop(moved_current);
    exercise_process_pointer_rows(
        input.as_ref(),
        chosen.id(),
        &resized_frame,
        PointerGeometry::reprojected(),
        capture.as_ref(),
        &mut visual,
        &mut fixture,
        &mut foreground_fixture,
        &foreground_before,
    );
    drop(resized_frame);
    let resized_current = capture
        .frame(&FrameRequest::latest(), &bounded(CONTENT_WAIT))
        .expect("the resized fixture keeps publishing");
    exercise_process_pointer_rows(
        input.as_ref(),
        chosen.id(),
        &resized_current,
        PointerGeometry::require_unchanged_since(resized_current.stamp()),
        capture.as_ref(),
        &mut visual,
        &mut fixture,
        &mut foreground_fixture,
        &foreground_before,
    );

    let display_count = observed_topology.len();
    assert!(
        (1..=16).contains(&display_count),
        "the bounded public display inventory is non-empty"
    );
    let mut topology_frame = resized_current;
    let mut topology_visits = vec![ObservedQualificationGeometry {
        geometry: qualification_geometry(&topology_frame),
        stamp: topology_frame.stamp(),
    }];
    let transition_count =
        if expected_topology == QualificationTopology::MixedScale && display_count > 1 {
            display_count
        } else {
            display_count - 1
        };
    for _ in 0..transition_count {
        let moved_display = fixture
            .command(FixtureCommandKind::MoveToNextDisplay, CONTENT_WAIT)
            .expect("the inter-display movement command completes");
        assert_eq!(moved_display.status, 0);
        let next_frame = wait_for_geometry_frame(
            capture.as_ref(),
            visual.stamp,
            visual.replacement_fill,
            "inter-display target",
        );
        visual.stamp = next_frame.stamp();
        assert_stale_pointer_frame_refused(
            input.as_ref(),
            chosen.id(),
            &topology_frame,
            &mut fixture,
            &mut foreground_fixture,
            &foreground_before,
        );
        drop(topology_frame);
        exercise_process_pointer_rows(
            input.as_ref(),
            chosen.id(),
            &next_frame,
            PointerGeometry::reprojected(),
            capture.as_ref(),
            &mut visual,
            &mut fixture,
            &mut foreground_fixture,
            &foreground_before,
        );
        drop(next_frame);
        let next_current = capture
            .frame(&FrameRequest::latest(), &bounded(CONTENT_WAIT))
            .expect("the inter-display fixture keeps publishing");
        exercise_process_pointer_rows(
            input.as_ref(),
            chosen.id(),
            &next_current,
            PointerGeometry::require_unchanged_since(next_current.stamp()),
            capture.as_ref(),
            &mut visual,
            &mut fixture,
            &mut foreground_fixture,
            &foreground_before,
        );
        topology_frame = next_current;
        topology_visits.push(ObservedQualificationGeometry {
            geometry: qualification_geometry(&topology_frame),
            stamp: topology_frame.stamp(),
        });
    }
    validate_window_topology(expected_topology, observed_topology, &topology_visits)
        .unwrap_or_else(|reason| {
            panic!(
                "{:?} {} retained-window topology refused: {reason}",
                fixture.facts.renderer(),
                expected_topology.label()
            )
        });
    for (ordinal, visit) in topology_visits.iter().enumerate() {
        println!(
            "qualification-window-topology={} renderer={:?} visit={} logical={}x{} \
             backing={}x{} origin=({},{}) scale={}x{} frame={:?}",
            expected_topology.label(),
            fixture.facts.renderer(),
            ordinal,
            visit.geometry.logical.0,
            visit.geometry.logical.1,
            visit.geometry.backing.0,
            visit.geometry.backing.1,
            visit.geometry.origin.0,
            visit.geometry.origin.1,
            visit.geometry.scale.0,
            visit.geometry.scale.1,
            visit.stamp,
        );
    }

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
        let expected = expected_process_keyboard_events(&events);
        assert_eq!(
            expected.iter().map(|event| event.kind).collect::<Vec<_>>(),
            expected_kinds,
            "the independent native oracle preserves the declared key sequence"
        );
        let (operation, correlation) = qualification_operation(&expected, CONTENT_WAIT);
        fixture.begin_operation_event_row(&operation, CONTENT_WAIT);
        foreground_fixture.begin_event_row(CONTENT_WAIT);
        let cursor_before = pointer_location();
        let submitted = events.len();
        let operation = refresh_qualification_deadline(&operation, CONTENT_WAIT);
        let receipt = input
            .execute(
                &InputRequest::new(
                    chosen.id(),
                    InputSequence::new(events).expect("the keyboard row is bounded and balanced"),
                    DeliveryPlan::require(InputDelivery::ProcessDirected),
                )
                .with_focus(FocusPolicy::Preserve),
                &operation,
            )
            .unwrap_or_else(|error| panic!("{label} process posting failed: {error}"));
        assert_process_receipt(&receipt, submitted);
        fixture.expect_exact_events(&expected, correlation, CONTENT_WAIT);
        assert_unrelated_desktop_state(
            &fixture,
            &mut foreground_fixture,
            &foreground_before,
            cursor_before,
        );
        observe_tagged_input_transition(capture.as_ref(), &mut visual);
    }

    const PROCESS_TEXT_CHUNK_UNITS: usize = 16;
    let process_text_chunk_units =
        u32::try_from(PROCESS_TEXT_CHUNK_UNITS).expect("text chunk size fits u32");
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
        // The 4,096-character boundary expands to 512 native units. Each
        // unit deliberately refreshes retained-window/process authority.
        (
            "maximum representable text",
            maximum_text,
            vec![process_text_chunk_units; InputEvent::MAX_TEXT_CHARS / PROCESS_TEXT_CHUNK_UNITS],
            Duration::from_secs(120),
        ),
    ];
    for (label, text, expected_units, wait) in text_rows {
        let expected = expected_process_text_events(&text);
        let expected_chunk_units = expected
            .chunks_exact(2)
            .map(|pair| {
                assert_eq!(pair[0].text_units, pair[1].text_units);
                pair[0].text_units
            })
            .collect::<Vec<_>>();
        assert_eq!(
            expected_chunk_units, expected_units,
            "the independent native oracle preserves the declared UTF-16 chunk boundaries"
        );
        let (operation, correlation) = qualification_operation(&expected, wait);
        fixture.begin_operation_event_row(&operation, wait);
        foreground_fixture.begin_event_row(wait);
        let cursor_before = pointer_location();
        let operation = refresh_qualification_deadline(&operation, wait);
        let receipt = input
            .execute(
                &InputRequest::new(
                    chosen.id(),
                    InputSequence::new(vec![InputEvent::Text(text)])
                        .expect("the exact text qualification row is bounded"),
                    DeliveryPlan::require(InputDelivery::ProcessDirected),
                )
                .with_focus(FocusPolicy::Preserve),
                &operation,
            )
            .unwrap_or_else(|error| panic!("{label} process posting failed: {error}"));
        assert_process_receipt(&receipt, 1);
        fixture.expect_exact_events(&expected, correlation, wait);
        assert_unrelated_desktop_state(
            &fixture,
            &mut foreground_fixture,
            &foreground_before,
            cursor_before,
        );
        observe_tagged_input_transition(capture.as_ref(), &mut visual);
    }
    let cleanup_events = [
        InputEvent::KeyPress(Key::Modifier(Modifier::Shift)),
        InputEvent::KeyRelease(Key::Modifier(Modifier::Shift)),
    ];
    let expected_cleanup = expected_process_keyboard_events(&cleanup_events);
    let (operation, correlation) = qualification_operation(&expected_cleanup, CONTENT_WAIT);
    fixture.begin_operation_event_row(&operation, CONTENT_WAIT);
    foreground_fixture.begin_event_row(CONTENT_WAIT);
    let pressed = EventSummary {
        kind: expected_cleanup[0].kind,
        text_units: expected_cleanup[0].text_units,
        correlation,
    };
    let cancellation = CancellationToken::new();
    let cancellation_observer =
        fixture.move_offscreen_and_cancel_after_event(pressed, cancellation.clone(), CONTENT_WAIT);
    let cursor_before = pointer_location();
    let operation = refresh_qualification_deadline(&operation, CONTENT_WAIT);
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
            &operation.with_cancellation(cancellation),
        )
        .expect("the cancelled process-directed row returns a receipt");
    let (observed_press, offscreen) = cancellation_observer
        .join()
        .expect("the cleanup-state transition helper remains contained");
    assert_eq!(observed_press, Some(pressed));
    let offscreen = offscreen.expect("the retained fixture moved off-screen before cancellation");
    assert_eq!(offscreen.status, 0);
    assert_eq!(offscreen.before_window, offscreen.after_window);
    wait_for_process_unavailable(
        &provider,
        chosen.id(),
        InputOperationKind::Keyboard,
        CONTENT_WAIT,
    );
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
        kind: expected_cleanup[1].kind,
        text_units: expected_cleanup[1].text_units,
        correlation,
    };
    assert_eq!(fixture.exact_event_summaries(1, CONTENT_WAIT), [released]);
    let report = fixture
        .command(FixtureCommandKind::ReadEvents, CONTENT_WAIT)
        .expect("the fixture reports the complete cleanup event row");
    assert_eq!(report.status, 0);
    assert_eq!(report.events, event_totals(&[pressed, released]));
    assert_eq!(report.event_correlation, correlation);
    assert!(
        report.event_payload_matches,
        "the cleanup row payload digest must cover the exact press and bounded release"
    );
    assert_unrelated_desktop_state(
        &fixture,
        &mut foreground_fixture,
        &foreground_before,
        cursor_before,
    );
    let onscreen = fixture
        .command(FixtureCommandKind::RestoreOnscreen, CONTENT_WAIT)
        .expect("the retained fixture returns to its exact prior origin after cleanup");
    assert_eq!(onscreen.status, 0);
    assert_eq!(onscreen.before_window, onscreen.after_window);
    observe_tagged_input_transition(capture.as_ref(), &mut visual);
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

fn qualify_process_directed_mode(mode: FixtureMode) {
    assert!(
        std::env::var_os("MADO_PILOT_MACOS_FIXTURE_EXECUTABLE").is_some(),
        "qualification requires the configured signed fixture bundle"
    );
    assert!(
        post_event_access_granted(),
        "qualification requires non-prompting post-event authorization to be granted"
    );
    let expected_topology = QualificationTopology::required();
    let topology_provider = provider();
    let observed_topology = observe_qualification_topology(&topology_provider, expected_topology);
    qualify_process_directed_renderer(mode, expected_topology, &observed_topology);
}

/// Qualifies every positive operation row through the AppKit renderer.
#[test]
#[ignore = "delivers real process-directed input on an interactive desktop"]
fn process_directed_delivery_qualifies_appkit_renderer() {
    qualify_process_directed_mode(FixtureMode::Default);
}

/// Qualifies every positive operation row through the game-like renderer.
#[test]
#[ignore = "delivers real process-directed input on an interactive desktop"]
fn process_directed_delivery_qualifies_game_like_renderer() {
    qualify_process_directed_mode(FixtureMode::GameLike);
}

fn fixture_frame_content_matches(frame: &Frame, replacement: bool, context_label: &str) -> bool {
    let mapping = frame
        .map(PixelFormat::Bgra8, &bounded(CONTENT_WAIT))
        .unwrap_or_else(|error| panic!("{context_label} frame maps: {error}"));
    let descriptor = mapping.descriptor();
    if replacement {
        frame_is_replacement_content(mapping.bytes(), descriptor.stride(), descriptor.extent())
    } else {
        frame_is_fixture_content(mapping.bytes(), descriptor.stride(), descriptor.extent())
    }
}

fn assert_fixture_frame_content(frame: &Frame, replacement: bool, context_label: &str) {
    assert!(
        fixture_frame_content_matches(frame, replacement, context_label),
        "{context_label} published unexpected fixture content"
    );
}

fn observe_unchanged_fixture_content(
    capture: &dyn CaptureSession,
    after: FrameStamp,
    replacement: bool,
    context_label: &str,
) -> FrameStamp {
    let frame = capture
        .frame(&FrameRequest::newer_than(after), &bounded(CONTENT_WAIT))
        .unwrap_or_else(|error| panic!("{context_label} publishes a newer frame: {error}"));
    assert!(
        frame.stamp().is_same_stream(&after),
        "{context_label} changed stream identity without an approved transition"
    );
    assert_eq!(
        frame.stamp().epoch(),
        after.epoch(),
        "{context_label} changed epoch without an approved transition"
    );
    assert_eq!(
        frame.stamp().geometry(),
        after.geometry(),
        "{context_label} changed geometry without an approved transition"
    );
    assert!(
        frame.stamp().sequence().value() > after.sequence().value(),
        "{context_label} did not publish a strictly newer frame"
    );
    assert_fixture_frame_content(&frame, replacement, context_label);
    frame.stamp()
}

fn wait_for_fixture_geometry(
    capture: &dyn CaptureSession,
    after: FrameStamp,
    expected: QualificationGeometry,
    replacement: bool,
    context_label: &str,
) -> Frame {
    let deadline = Instant::now() + CONTENT_WAIT;
    let mut cursor = after;
    let mut matching_since = None;
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match capture.frame(
            &FrameRequest::newer_than(cursor),
            &bounded(remaining.min(Duration::from_millis(500))),
        ) {
            Ok(frame) => {
                cursor = frame.stamp();
                if qualification_geometry(&frame) != expected
                    || !fixture_frame_content_matches(&frame, replacement, context_label)
                {
                    matching_since = None;
                    continue;
                }
                let stable_since = matching_since.get_or_insert_with(Instant::now);
                if stable_since.elapsed() >= GEOMETRY_SETTLE {
                    return frame;
                }
            }
            Err(error) if error.status() == Status::DeadlineExceeded => {}
            Err(error) => panic!("{context_label} frame refresh failed: {error}"),
        }
    }
    panic!("{context_label} did not republish and retain the expected geometry and content");
}

fn wait_until_frontmost_fixture(fixture: &Fixture) -> u32 {
    let deadline = Instant::now() + CONTENT_WAIT;
    let mut observed = None;
    loop {
        if let Some(frontmost) = frontmost_application() {
            observed = Some(frontmost);
            if frontmost == fixture.process_id {
                return frontmost;
            }
        }
        assert!(
            Instant::now() < deadline,
            "fixture {} did not become frontmost; observed {observed:?}",
            fixture.process_id
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn post_isolated_process_key_pair(
    input: &dyn InputController,
    target: TargetId,
    capture: &dyn CaptureSession,
    visual: &mut ControlledVisualObservation,
    fixture: &mut Fixture,
    foreground_fixture: &mut Fixture,
    foreground_before: &u32,
) -> mado_pilot_input::InputReceipt {
    let expected = expected_process_keyboard_events(&[
        InputEvent::KeyPress(Key::Enter),
        InputEvent::KeyRelease(Key::Enter),
    ]);
    let (operation, correlation) = qualification_operation(&expected, CONTENT_WAIT);
    fixture.begin_operation_event_row(&operation, CONTENT_WAIT);
    foreground_fixture.begin_event_row(CONTENT_WAIT);
    post_untagged_process_key_pair(fixture.process_id);
    thread::sleep(Duration::from_millis(100));
    assert_eq!(
        fixture.event_totals(CONTENT_WAIT),
        EventTotals::default(),
        "an untagged same-process source received correlated event credit"
    );
    assert!(
        fixture.pending_events.is_empty(),
        "an untagged same-process source received correlated event records"
    );
    let after_untagged = capture
        .frame(&FrameRequest::latest(), &bounded(CONTENT_WAIT))
        .expect("the retained fixture remains capturable after unrelated input");
    assert_fixture_frame_content(
        &after_untagged,
        visual.replacement_fill,
        "qualification target after untagged same-process input",
    );
    let cursor_before = pointer_location();
    let operation = refresh_qualification_deadline(&operation, CONTENT_WAIT);
    let receipt = input
        .execute(&process_key_pair(target), &operation)
        .expect("the bracketed process-directed key pair returns a receipt");
    assert_process_receipt(&receipt, 2);
    fixture.expect_exact_events(&expected, correlation, CONTENT_WAIT);
    assert_unrelated_desktop_state(
        fixture,
        foreground_fixture,
        foreground_before,
        cursor_before,
    );
    observe_tagged_input_transition(capture, visual);
    receipt
}

fn qualify_controlled_unrelated_activity(mode: FixtureMode) {
    let mut foreground_fixture = Fixture::start_foreground()
        .expect("the unrelated foreground fixture starts from its independent bundle");
    let mut fixture = Fixture::start_inactive(mode)
        .expect("the qualification target starts without taking foreground");
    let provider = provider();
    let chosen = discover_unique_fixture(&provider, &fixture, CONTENT_WAIT)
        .expect("the qualification target is selected exactly once");
    let foreground = discover_unique_fixture(&provider, &foreground_fixture, CONTENT_WAIT)
        .expect("the foreground fixture is selected exactly once");

    let capture = CaptureProvider::open(
        &provider,
        chosen.id(),
        &OpenRequest::new().require_format(PixelFormat::Bgra8),
        &bounded(CONTENT_WAIT),
    )
    .expect("the qualification target opens for capture");
    let foreground_capture = CaptureProvider::open(
        &provider,
        foreground.id(),
        &OpenRequest::new().require_format(PixelFormat::Bgra8),
        &bounded(CONTENT_WAIT),
    )
    .expect("the foreground fixture opens for capture");
    let target_initial = capture
        .frame(&FrameRequest::latest(), &bounded(CONTENT_WAIT))
        .expect("the qualification target publishes an initial frame");
    assert_fixture_frame_content(&target_initial, false, "qualification target");
    let foreground_initial = foreground_capture
        .frame(&FrameRequest::latest(), &bounded(CONTENT_WAIT))
        .expect("the foreground fixture publishes an initial frame");
    let foreground_mapping = foreground_initial
        .map(PixelFormat::Bgra8, &bounded(CONTENT_WAIT))
        .expect("the foreground fixture initial frame maps");
    let foreground_descriptor = foreground_mapping.descriptor();

    let system_input = with_confirmed_fixture_content(
        foreground_mapping.bytes(),
        foreground_descriptor.stride(),
        foreground_descriptor.extent(),
        || {
            InputProvider::open(
                &provider,
                foreground.id(),
                &InputOpenRequest::new()
                    .with_requirement(InputRequirement::Required)
                    .requiring(InputOperationKind::Keyboard, InputDelivery::System),
                &bounded(CONTENT_WAIT),
            )
        },
    )
    .expect("the foreground input gate matches approved fixture pixels")
    .expect("system keyboard input opens for the owned foreground fixture");
    let process_input = InputProvider::open(
        &provider,
        chosen.id(),
        &InputOpenRequest::new()
            .with_requirement(InputRequirement::Required)
            .requiring(InputOperationKind::Keyboard, InputDelivery::ProcessDirected),
        &bounded(CONTENT_WAIT),
    )
    .expect("process-directed keyboard input opens for the inactive target");

    let foreground_before = wait_until_frontmost_fixture(&foreground_fixture);
    assert_ne!(
        foreground_before, fixture.process_id,
        "the qualification target must remain inactive"
    );
    thread::sleep(Duration::from_secs(10));
    let mut target_visual = ControlledVisualObservation {
        stamp: target_initial.stamp(),
        replacement_fill: false,
    };

    let before_receipt = post_isolated_process_key_pair(
        process_input.as_ref(),
        chosen.id(),
        capture.as_ref(),
        &mut target_visual,
        &mut fixture,
        &mut foreground_fixture,
        &foreground_before,
    );

    fixture.begin_event_row(CONTENT_WAIT);
    foreground_fixture.begin_event_row(CONTENT_WAIT);
    let foreground_stamp = observe_controlled_transition(
        &mut foreground_fixture,
        foreground_capture.as_ref(),
        foreground_initial.stamp(),
        true,
    );
    target_visual.stamp = observe_unchanged_fixture_content(
        capture.as_ref(),
        target_visual.stamp,
        true,
        "qualification target during unrelated foreground redraw",
    );
    assert_eq!(
        fixture.event_totals(CONTENT_WAIT),
        EventTotals::default(),
        "the foreground redraw reached the target process"
    );
    assert_eq!(
        foreground_fixture.event_totals(CONTENT_WAIT),
        EventTotals::default(),
        "the private foreground redraw synthesized input"
    );
    assert_eq!(
        frontmost_application().as_ref(),
        Some(&foreground_before),
        "the private foreground redraw changed frontmost ownership"
    );

    fixture.begin_event_row(CONTENT_WAIT);
    foreground_fixture.begin_event_row(CONTENT_WAIT);
    let system_receipt = system_input
        .execute(
            &InputRequest::new(
                foreground.id(),
                InputSequence::new(vec![
                    InputEvent::KeyPress(Key::Enter),
                    InputEvent::KeyRelease(Key::Enter),
                ])
                .expect("the scripted foreground action is balanced"),
                DeliveryPlan::require(InputDelivery::System),
            )
            .with_focus(FocusPolicy::RequireFocused),
            &bounded(CONTENT_WAIT),
        )
        .expect("the scripted foreground keyboard action returns a receipt");
    assert_eq!(
        system_receipt.outcome(),
        SequenceOutcome::Complete,
        "{system_receipt}"
    );
    assert_eq!(system_receipt.submitted(), 2);
    assert_eq!(system_receipt.selected_route(), Some(InputDelivery::System));
    assert!(!system_receipt.used_fallback());
    foreground_fixture.expect_event_kinds(&[EVENT_KEY_DOWN, EVENT_KEY_UP], CONTENT_WAIT);
    assert_eq!(
        fixture.event_totals(CONTENT_WAIT),
        EventTotals::default(),
        "the target process observed the scripted foreground action"
    );
    target_visual.stamp = observe_unchanged_fixture_content(
        capture.as_ref(),
        target_visual.stamp,
        true,
        "qualification target during scripted foreground action",
    );
    let foreground_stamp = observe_unchanged_fixture_content(
        foreground_capture.as_ref(),
        foreground_stamp,
        true,
        "foreground fixture after scripted keyboard action",
    );
    assert_eq!(
        frontmost_application().as_ref(),
        Some(&foreground_before),
        "the scripted foreground action changed frontmost ownership"
    );

    let after_receipt = post_isolated_process_key_pair(
        process_input.as_ref(),
        chosen.id(),
        capture.as_ref(),
        &mut target_visual,
        &mut fixture,
        &mut foreground_fixture,
        &foreground_before,
    );
    assert_eq!(
        after_receipt, before_receipt,
        "unrelated foreground activity changed the process-directed receipt"
    );
    let target_stamp = target_visual.stamp;
    println!(
        "qualification-unrelated-activity renderer={:?} process-sequences=2 \
         process-logical-events=4 foreground-system-logical-events=2 \
         foreground-private-transitions=1 target-visual-transitions=2 \
         process-window-cursor-invariance=true foreground-preserved=true \
         receipts-unchanged=true target-frame={target_stamp:?} \
         foreground-frame={foreground_stamp:?}",
        fixture.facts.renderer(),
    );

    process_input
        .close(&bounded(CONTENT_WAIT))
        .expect("process-directed input closes");
    system_input
        .close(&bounded(CONTENT_WAIT))
        .expect("foreground system input closes");
    capture
        .close(&bounded(CONTENT_WAIT))
        .expect("qualification target capture closes");
    foreground_capture
        .close(&bounded(CONTENT_WAIT))
        .expect("foreground capture closes");
    let stopped = fixture
        .command(FixtureCommandKind::Stop, CONTENT_WAIT)
        .expect("qualification target stop is acknowledged");
    assert_eq!(stopped.status, 0);
    let foreground_stopped = foreground_fixture
        .command(FixtureCommandKind::Stop, CONTENT_WAIT)
        .expect("foreground fixture stop is acknowledged");
    assert_eq!(foreground_stopped.status, 0);
    fixture.input = None;
    foreground_fixture.input = None;
}

/// Proves scripted foreground redraw and System input cannot contaminate the
/// AppKit process-directed target's receipts, events, or visual correlation.
#[test]
#[ignore = "delivers scripted System and process-directed input on an interactive desktop"]
fn controlled_unrelated_activity_remains_outside_appkit_process_evidence() {
    assert!(
        std::env::var_os("MADO_PILOT_MACOS_FIXTURE_EXECUTABLE").is_some(),
        "qualification requires the configured signed fixture bundle"
    );
    assert!(
        post_event_access_granted(),
        "qualification requires non-prompting post-event authorization to be granted"
    );
    qualify_controlled_unrelated_activity(FixtureMode::Default);
}

/// Proves scripted foreground redraw and System input cannot contaminate the
/// game-like process-directed target's receipts, events, or visual correlation.
#[test]
#[ignore = "delivers scripted System and process-directed input on an interactive desktop"]
fn controlled_unrelated_activity_remains_outside_game_like_process_evidence() {
    assert!(
        std::env::var_os("MADO_PILOT_MACOS_FIXTURE_EXECUTABLE").is_some(),
        "qualification requires the configured signed fixture bundle"
    );
    assert!(
        post_event_access_granted(),
        "qualification requires non-prompting post-event authorization to be granted"
    );
    qualify_controlled_unrelated_activity(FixtureMode::GameLike);
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

    let mut foreground_fixture = Fixture::start_foreground()
        .expect("the unrelated foreground fixture starts from its independent bundle");
    for mode in [FixtureMode::GameLike, FixtureMode::Default] {
        let mut fixture = Fixture::start_inactive(mode)
            .expect("the owned fixture starts visible without taking foreground ownership");
        let provider = provider();
        let chosen = discover_unique_fixture(&provider, &fixture, CONTENT_WAIT)
            .expect("the owned child exposes one uniquely selected fixture window");
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
        require_auxiliary_fixture_windows(&provider, &fixture, CONTENT_WAIT);
        thread::sleep(Duration::from_secs(10));
        let mut visual = ControlledVisualObservation {
            stamp: first.stamp(),
            replacement_fill: false,
        };

        let foreground_before = wait_until_frontmost_fixture(&foreground_fixture);

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
        let expected = expected_process_pointer_events(&first, &pointer_events);
        assert_eq!(
            expected.iter().map(|event| event.kind).collect::<Vec<_>>(),
            expected_pointer_events
        );
        let (operation, correlation) = qualification_operation(&expected, CONTENT_WAIT);
        fixture.begin_operation_event_row(&operation, CONTENT_WAIT);
        let submitted = pointer_events.len();
        let operation = refresh_qualification_deadline(&operation, CONTENT_WAIT);
        let first_receipt = input
            .execute(
                &InputRequest::new(
                    chosen.id(),
                    InputSequence::new(pointer_events).expect("the pointer soak row is bounded"),
                    DeliveryPlan::require(InputDelivery::ProcessDirected),
                )
                .with_focus(FocusPolicy::Preserve)
                .with_pointer_geometry(PointerGeometry::require_unchanged_since(first.stamp())),
                &operation,
            )
            .expect("the first spaced process-directed sequence posts");
        assert_process_receipt(&first_receipt, submitted);
        fixture.expect_exact_events(&expected, correlation, CONTENT_WAIT);
        assert_unrelated_desktop_state(
            &fixture,
            &mut foreground_fixture,
            &foreground_before,
            cursor_before,
        );
        observe_tagged_input_transition(capture.as_ref(), &mut visual);
        let first_transition = visual.stamp;

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
        let expected = expected_process_keyboard_events(&[
            InputEvent::KeyPress(Key::Enter),
            InputEvent::KeyRelease(Key::Enter),
        ]);
        let (operation, correlation) = qualification_operation(&expected, CONTENT_WAIT);
        fixture.begin_operation_event_row(&operation, CONTENT_WAIT);
        let operation = refresh_qualification_deadline(&operation, CONTENT_WAIT);
        let second_receipt = input
            .execute(&process_key_pair(chosen.id()), &operation)
            .expect("the second spaced process-directed sequence posts");
        assert_process_receipt(&second_receipt, 2);
        fixture.expect_exact_events(&expected, correlation, CONTENT_WAIT);
        assert_unrelated_desktop_state(
            &fixture,
            &mut foreground_fixture,
            &foreground_before,
            cursor_before,
        );
        observe_tagged_input_transition(capture.as_ref(), &mut visual);
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
        fixture.input = None;
    }
    let foreground_stopped = foreground_fixture
        .command(FixtureCommandKind::Stop, CONTENT_WAIT)
        .expect("the unrelated foreground fixture stop is acknowledged");
    assert_eq!(foreground_stopped.status, 0);
    foreground_fixture.input = None;
}

/// Moves each retained renderer target outside every display and then closes it,
/// proving pointer delivery refuses before posting in both target-loss states.
#[test]
#[ignore = "moves and closes real fixture windows"]
fn process_directed_pointer_refuses_offscreen_and_closed_targets() {
    assert!(
        std::env::var_os("MADO_PILOT_MACOS_FIXTURE_EXECUTABLE").is_some(),
        "qualification requires the configured signed fixture bundle"
    );
    let mut foreground_fixture = Fixture::start_foreground()
        .expect("the unrelated foreground fixture starts from its independent bundle");
    for mode in [FixtureMode::Default, FixtureMode::GameLike] {
        let mut fixture = Fixture::start_inactive(mode)
            .expect("the owned target fixture starts without taking foreground");
        let provider = provider();
        let chosen = discover_unique_fixture(&provider, &fixture, CONTENT_WAIT)
            .expect("the owned child exposes one uniquely selected primary window");
        let capture = CaptureProvider::open(
            &provider,
            chosen.id(),
            &OpenRequest::new().require_format(PixelFormat::Bgra8),
            &bounded(CONTENT_WAIT),
        )
        .expect("desktop-independent capture opens for the retained target");
        let first = capture
            .frame(&FrameRequest::latest(), &bounded(CONTENT_WAIT))
            .expect("the retained target publishes before it leaves the display union");
        assert_fixture_frame_content(&first, false, "off-screen target baseline");
        let first_geometry = qualification_geometry(&first);
        let extent = first.descriptor().extent();
        let point = Point::new(
            CoordinateSpace::CapturePixels,
            f64::from(extent.width()) / 2.0,
            f64::from(extent.height()) / 2.0,
        )
        .expect("the retained frame centre is finite");
        let request = InputRequest::new(
            chosen.id(),
            InputSequence::new(vec![InputEvent::PointerMove(point)])
                .expect("the target-loss pointer row is bounded"),
            DeliveryPlan::require(InputDelivery::ProcessDirected),
        )
        .with_focus(FocusPolicy::Preserve)
        .with_pointer_geometry(PointerGeometry::require_unchanged_since(first.stamp()));
        let input = InputProvider::open(
            &provider,
            chosen.id(),
            &InputOpenRequest::new()
                .with_requirement(InputRequirement::Required)
                .requiring(InputOperationKind::Pointer, InputDelivery::ProcessDirected),
            &bounded(CONTENT_WAIT),
        )
        .expect("the process-directed pointer pair opens");
        let foreground_before = wait_until_frontmost_fixture(&foreground_fixture);

        let offscreen = fixture
            .command(FixtureCommandKind::MoveOffscreen, CONTENT_WAIT)
            .expect("the retained fixture moves wholly outside the display union");
        assert_eq!(offscreen.status, 0);
        assert_eq!(offscreen.before_window, offscreen.after_window);
        wait_for_process_unavailable(
            &provider,
            chosen.id(),
            InputOperationKind::Pointer,
            CONTENT_WAIT,
        );
        fixture.begin_event_row(CONTENT_WAIT);
        foreground_fixture.begin_event_row(CONTENT_WAIT);
        let cursor_before = pointer_location();
        let receipt = input
            .execute(&request, &bounded(CONTENT_WAIT))
            .expect("an off-screen target returns a receipt");
        let fault = receipt
            .fault()
            .expect("an off-screen target reports why admission stopped");
        assert!(
            matches!(
                fault,
                InputFault::TargetLost | InputFault::UnsupportedCombination
            ),
            "off-screen refusal reported {fault}"
        );
        assert_zero_effect(&receipt, fault);
        assert!(
            fixture
                .event_summaries(1, Duration::from_millis(200))
                .is_empty(),
            "an off-screen target received input before refusal"
        );
        assert_eq!(fixture.event_totals(CONTENT_WAIT), EventTotals::default());
        assert_unrelated_desktop_state(
            &fixture,
            &mut foreground_fixture,
            &foreground_before,
            cursor_before,
        );

        let onscreen = fixture
            .command(FixtureCommandKind::RestoreOnscreen, CONTENT_WAIT)
            .expect("the retained fixture returns to its exact prior origin");
        assert_eq!(onscreen.status, 0);
        assert_eq!(onscreen.before_window, onscreen.after_window);
        wait_for_process_available(
            &provider,
            chosen.id(),
            InputOperationKind::Pointer,
            CONTENT_WAIT,
        );
        let restored_frame = wait_for_fixture_geometry(
            capture.as_ref(),
            first.stamp(),
            first_geometry,
            false,
            "restored off-screen target",
        );
        let recovery_events = vec![InputEvent::PointerMove(point)];
        let expected = expected_process_pointer_events(&restored_frame, &recovery_events);
        let (operation, correlation) = qualification_operation(&expected, CONTENT_WAIT);
        let recovery_request = InputRequest::new(
            chosen.id(),
            InputSequence::new(recovery_events)
                .expect("the restored-target pointer row is bounded"),
            DeliveryPlan::require(InputDelivery::ProcessDirected),
        )
        .with_focus(FocusPolicy::Preserve)
        .with_pointer_geometry(PointerGeometry::require_unchanged_since(
            restored_frame.stamp(),
        ));
        fixture.begin_operation_event_row(&operation, CONTENT_WAIT);
        foreground_fixture.begin_event_row(CONTENT_WAIT);
        let cursor_before = pointer_location();
        let operation = refresh_qualification_deadline(&operation, CONTENT_WAIT);
        let recovery_receipt = input
            .execute(&recovery_request, &operation)
            .expect("the restored exact target accepts process-directed pointer input");
        assert_process_receipt(&recovery_receipt, 1);
        fixture.expect_exact_events(&expected, correlation, CONTENT_WAIT);
        assert_unrelated_desktop_state(
            &fixture,
            &mut foreground_fixture,
            &foreground_before,
            cursor_before,
        );
        let recovered_frame = wait_for_fixture_geometry(
            capture.as_ref(),
            restored_frame.stamp(),
            first_geometry,
            true,
            "restored target after pointer delivery",
        );
        assert!(
            recovered_frame
                .stamp()
                .is_same_stream(&restored_frame.stamp())
        );
        assert_eq!(
            recovered_frame.stamp().epoch(),
            restored_frame.stamp().epoch(),
            "pointer delivery changed the restored capture epoch"
        );

        let closed = fixture
            .command(FixtureCommandKind::Close, CONTENT_WAIT)
            .expect("the retained fixture window closes");
        assert_eq!(closed.status, 0);
        assert_eq!(closed.before_window, onscreen.after_window);
        assert_eq!(closed.after_window, 0);
        wait_for_process_unavailable(
            &provider,
            chosen.id(),
            InputOperationKind::Pointer,
            CONTENT_WAIT,
        );
        fixture.begin_event_row(CONTENT_WAIT);
        foreground_fixture.begin_event_row(CONTENT_WAIT);
        let cursor_before = pointer_location();
        let receipt = input
            .execute(&request, &bounded(CONTENT_WAIT))
            .expect("a closed target returns a receipt");
        let fault = receipt
            .fault()
            .expect("a closed target reports why admission stopped");
        assert!(
            matches!(
                fault,
                InputFault::TargetLost | InputFault::UnsupportedCombination
            ),
            "closed-target refusal reported {fault}"
        );
        assert_zero_effect(&receipt, fault);
        assert!(
            fixture
                .event_summaries(1, Duration::from_millis(200))
                .is_empty(),
            "a closed target received input before refusal"
        );
        assert_eq!(fixture.event_totals(CONTENT_WAIT), EventTotals::default());
        assert_unrelated_desktop_state(
            &fixture,
            &mut foreground_fixture,
            &foreground_before,
            cursor_before,
        );

        input.close(&bounded(CONTENT_WAIT)).expect("input closes");
        capture
            .close(&bounded(CONTENT_WAIT))
            .expect("capture closes");
        let stopped = fixture
            .command(FixtureCommandKind::Stop, CONTENT_WAIT)
            .expect("owned fixture stop is acknowledged");
        assert_eq!(stopped.status, 0);
        fixture.input = None;
    }
    let foreground_stopped = foreground_fixture
        .command(FixtureCommandKind::Stop, CONTENT_WAIT)
        .expect("the unrelated foreground fixture stop is acknowledged");
    assert_eq!(foreground_stopped.status, 0);
    foreground_fixture.input = None;
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
    let mut foreground_fixture = Fixture::start_foreground()
        .expect("the unrelated foreground fixture starts from its independent bundle");
    let mut fixture = Fixture::start_inactive(FixtureMode::Default)
        .expect("the owned target fixture starts without taking foreground");
    let provider = provider();
    let chosen = discover_unique_fixture(&provider, &fixture, CONTENT_WAIT)
        .expect("the owned child exposes one uniquely selected primary window");
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
    let foreground_before = wait_until_frontmost_fixture(&foreground_fixture);
    assert_ne!(
        foreground_before, fixture.process_id,
        "the qualification target must remain inactive"
    );

    let auxiliary = fixture
        .command(FixtureCommandKind::OpenAuxiliary, CONTENT_WAIT)
        .expect("the auxiliary-window transition completes");
    assert_eq!(auxiliary.status, 0);
    require_auxiliary_fixture_windows(&provider, &fixture, CONTENT_WAIT);
    thread::sleep(Duration::from_secs(10));
    let active_capture_stamp =
        observe_controlled_transition(&mut fixture, capture.as_ref(), first.stamp(), true);
    let mut visual = ControlledVisualObservation {
        stamp: active_capture_stamp,
        replacement_fill: true,
    };
    let multiple_window_target = discover_unique_fixture(&provider, &fixture, CONTENT_WAIT)
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
    let expected = expected_process_keyboard_events(&[
        InputEvent::KeyPress(Key::Enter),
        InputEvent::KeyRelease(Key::Enter),
    ]);
    let (operation, correlation) = qualification_operation(&expected, CONTENT_WAIT);
    fixture.begin_operation_event_row(&operation, CONTENT_WAIT);
    let operation = refresh_qualification_deadline(&operation, CONTENT_WAIT);
    let multiple_window = input
        .execute(&process_key_pair(chosen.id()), &operation)
        .expect("the multi-window process returns a receipt");
    assert_process_receipt(&multiple_window, 2);
    fixture.expect_exact_events(&expected, correlation, CONTENT_WAIT);
    assert_unrelated_desktop_state(
        &fixture,
        &mut foreground_fixture,
        &foreground_before,
        cursor_before,
    );
    observe_tagged_input_transition(capture.as_ref(), &mut visual);

    let closed_auxiliary = fixture
        .command(FixtureCommandKind::CloseAuxiliary, CONTENT_WAIT)
        .expect("the auxiliary window closes");
    assert_eq!(closed_auxiliary.status, 0);
    fixture.begin_event_row(CONTENT_WAIT);
    foreground_fixture.begin_event_row(CONTENT_WAIT);
    let cursor_before = pointer_location();
    let expected = expected_process_keyboard_events(&[
        InputEvent::KeyPress(Key::Enter),
        InputEvent::KeyRelease(Key::Enter),
    ]);
    let (operation, correlation) = qualification_operation(&expected, CONTENT_WAIT);
    fixture.begin_operation_event_row(&operation, CONTENT_WAIT);
    let operation = refresh_qualification_deadline(&operation, CONTENT_WAIT);
    let ordinary = input
        .execute(&process_key_pair(chosen.id()), &operation)
        .expect("the retained process returns a receipt");
    assert_process_receipt(&ordinary, 2);
    fixture.expect_exact_events(&expected, correlation, CONTENT_WAIT);
    assert_unrelated_desktop_state(
        &fixture,
        &mut foreground_fixture,
        &foreground_before,
        cursor_before,
    );
    observe_tagged_input_transition(capture.as_ref(), &mut visual);

    let minimized = fixture
        .command(FixtureCommandKind::Minimize, CONTENT_WAIT)
        .expect("the fixture minimizes");
    assert_eq!(minimized.status, 0);
    wait_for_process_unavailable(
        &provider,
        chosen.id(),
        InputOperationKind::Keyboard,
        CONTENT_WAIT,
    );
    fixture.begin_event_row(CONTENT_WAIT);
    foreground_fixture.begin_event_row(CONTENT_WAIT);
    let cursor_before = pointer_location();
    let minimized_result = input
        .execute(&process_key_pair(chosen.id()), &bounded(CONTENT_WAIT))
        .expect("a minimized target returns a receipt");
    if let Some(fault) = minimized_result.fault() {
        assert!(
            matches!(
                fault,
                InputFault::TargetLost | InputFault::UnsupportedCombination
            ),
            "minimized-target refusal reported {fault}"
        );
        assert_zero_effect(&minimized_result, fault);
        assert!(
            fixture
                .event_summaries(1, Duration::from_millis(200))
                .is_empty(),
            "a refused minimized target received input"
        );
        assert_eq!(fixture.event_totals(CONTENT_WAIT), EventTotals::default());
    } else {
        assert_process_receipt(&minimized_result, 2);
        let expected = expected_process_keyboard_events(&[
            InputEvent::KeyPress(Key::Enter),
            InputEvent::KeyRelease(Key::Enter),
        ])
        .into_iter()
        .map(|event| EventSummary {
            kind: event.kind,
            text_units: event.text_units,
            correlation: 0,
        })
        .collect::<Vec<_>>();
        assert_eq!(
            fixture.event_summaries(expected.len(), CONTENT_WAIT),
            expected,
            "a target that returned before final authority receives the exact row"
        );
        assert_eq!(fixture.event_totals(CONTENT_WAIT), event_totals(&expected));
    }
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
    let expected = expected_process_keyboard_events(&[
        InputEvent::KeyPress(Key::Enter),
        InputEvent::KeyRelease(Key::Enter),
    ]);
    let (operation, correlation) = qualification_operation(&expected, CONTENT_WAIT);
    fixture.begin_operation_event_row(&operation, CONTENT_WAIT);
    foreground_fixture.begin_event_row(CONTENT_WAIT);
    let cursor_before = pointer_location();
    let operation = refresh_qualification_deadline(&operation, CONTENT_WAIT);
    let restored_receipt = loop {
        let receipt = input
            .execute(&process_key_pair(chosen.id()), &operation)
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
    fixture.expect_exact_events(&expected, correlation, CONTENT_WAIT);
    assert_unrelated_desktop_state(
        &fixture,
        &mut foreground_fixture,
        &foreground_before,
        cursor_before,
    );
    observe_tagged_input_transition(capture.as_ref(), &mut visual);

    for kind in [FixtureCommandKind::Move, FixtureCommandKind::Resize] {
        let transition = fixture
            .command(kind, CONTENT_WAIT)
            .expect("the geometry transition completes");
        assert_eq!(transition.status, 0);
        fixture.begin_event_row(CONTENT_WAIT);
        foreground_fixture.begin_event_row(CONTENT_WAIT);
        let cursor_before = pointer_location();
        let expected = expected_process_keyboard_events(&[
            InputEvent::KeyPress(Key::Enter),
            InputEvent::KeyRelease(Key::Enter),
        ]);
        let (operation, correlation) = qualification_operation(&expected, CONTENT_WAIT);
        fixture.begin_operation_event_row(&operation, CONTENT_WAIT);
        let operation = refresh_qualification_deadline(&operation, CONTENT_WAIT);
        let receipt = input
            .execute(&process_key_pair(chosen.id()), &operation)
            .expect("the geometry-updated target returns a receipt");
        assert_process_receipt(&receipt, 2);
        fixture.expect_exact_events(&expected, correlation, CONTENT_WAIT);
        assert_unrelated_desktop_state(
            &fixture,
            &mut foreground_fixture,
            &foreground_before,
            cursor_before,
        );
        observe_tagged_input_transition(capture.as_ref(), &mut visual);
    }

    let offscreen = fixture
        .command(FixtureCommandKind::MoveOffscreen, CONTENT_WAIT)
        .expect("the retained fixture moves wholly outside the display union");
    assert_eq!(offscreen.status, 0);
    assert_eq!(offscreen.before_window, offscreen.after_window);
    wait_for_process_unavailable(
        &provider,
        chosen.id(),
        InputOperationKind::Keyboard,
        CONTENT_WAIT,
    );
    fixture.begin_event_row(CONTENT_WAIT);
    foreground_fixture.begin_event_row(CONTENT_WAIT);
    let cursor_before = pointer_location();
    let offscreen_receipt = input
        .execute(&process_key_pair(chosen.id()), &bounded(CONTENT_WAIT))
        .expect("an off-screen target returns a receipt");
    if let Some(fault) = offscreen_receipt.fault() {
        assert!(
            matches!(
                fault,
                InputFault::TargetLost | InputFault::UnsupportedCombination
            ),
            "off-screen refusal reported {fault}"
        );
        assert_zero_effect(&offscreen_receipt, fault);
        assert!(
            fixture
                .event_summaries(1, Duration::from_millis(200))
                .is_empty(),
            "a refused off-screen target received input"
        );
        assert_eq!(fixture.event_totals(CONTENT_WAIT), EventTotals::default());
    } else {
        assert_process_receipt(&offscreen_receipt, 2);
        let expected = expected_process_keyboard_events(&[
            InputEvent::KeyPress(Key::Enter),
            InputEvent::KeyRelease(Key::Enter),
        ])
        .into_iter()
        .map(|event| EventSummary {
            kind: event.kind,
            text_units: event.text_units,
            correlation: 0,
        })
        .collect::<Vec<_>>();
        assert_eq!(
            fixture.event_summaries(expected.len(), CONTENT_WAIT),
            expected,
            "an off-screen target that returned before final authority receives the exact row"
        );
        assert_eq!(fixture.event_totals(CONTENT_WAIT), event_totals(&expected));
    }
    assert_unrelated_desktop_state(
        &fixture,
        &mut foreground_fixture,
        &foreground_before,
        cursor_before,
    );

    let onscreen = fixture
        .command(FixtureCommandKind::RestoreOnscreen, CONTENT_WAIT)
        .expect("the retained fixture returns to its exact prior origin");
    assert_eq!(onscreen.status, 0);
    assert_eq!(onscreen.before_window, onscreen.after_window);

    let replacement = fixture
        .command(FixtureCommandKind::Replace, CONTENT_WAIT)
        .expect("the owned fixture replaces its window");
    assert_eq!(replacement.status, 0);
    assert_ne!(replacement.before_window, replacement.after_window);
    let replacement_deadline = Instant::now() + CONTENT_WAIT;
    let expected = expected_process_keyboard_events(&[
        InputEvent::KeyPress(Key::Enter),
        InputEvent::KeyRelease(Key::Enter),
    ])
    .into_iter()
    .map(|event| EventSummary {
        kind: event.kind,
        text_units: event.text_units,
        correlation: 0,
    })
    .collect::<Vec<_>>();
    loop {
        fixture.begin_event_row(CONTENT_WAIT);
        foreground_fixture.begin_event_row(CONTENT_WAIT);
        let cursor_before = pointer_location();
        let replaced = input
            .execute(&process_key_pair(chosen.id()), &bounded(CONTENT_WAIT))
            .expect("the replaced target returns a receipt");
        if let Some(fault) = replaced.fault() {
            assert!(
                matches!(
                    fault,
                    InputFault::TargetLost | InputFault::UnsupportedCombination
                ),
                "replacement refusal reported {fault}"
            );
            assert_zero_effect(&replaced, fault);
            assert!(
                fixture
                    .event_summaries(1, Duration::from_millis(200))
                    .is_empty(),
                "a refused replacement transition received input through the stale target"
            );
            assert_eq!(fixture.event_totals(CONTENT_WAIT), EventTotals::default());
            assert_unrelated_desktop_state(
                &fixture,
                &mut foreground_fixture,
                &foreground_before,
                cursor_before,
            );
            break;
        }

        assert_process_receipt(&replaced, 2);
        assert_eq!(
            fixture.event_summaries(expected.len(), CONTENT_WAIT),
            expected,
            "the still-live retained window permits only the exact process row"
        );
        assert_eq!(fixture.event_totals(CONTENT_WAIT), event_totals(&expected));
        assert_unrelated_desktop_state(
            &fixture,
            &mut foreground_fixture,
            &foreground_before,
            cursor_before,
        );
        assert!(
            Instant::now() < replacement_deadline,
            "the replaced retained window remained authoritative past the scenario deadline"
        );
        thread::sleep(Duration::from_millis(25));
    }

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
    fn CFRelease(value: *const std::ffi::c_void);
    fn CGEventCreateKeyboardEvent(
        source: *const std::ffi::c_void,
        virtual_key: u16,
        key_down: bool,
    ) -> *mut std::ffi::c_void;
    fn CGEventPostToPid(process_id: i32, event: *const std::ffi::c_void);
    fn mp_shim_input_pointer_location(out_x: *mut f64, out_y: *mut f64) -> u32;
    fn mp_shim_input_frontmost_process(out_process: *mut u32) -> u32;
}
