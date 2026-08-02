//! What the dedicated macOS input fixture is, and how a check selects it safely.
//!
//! macOS offers no background input channel, so — unlike the Windows fixture —
//! this one cannot acknowledge a packet, and there is no protocol to version. It
//! is a receiver: a window with a stable identity and one fixed colour that
//! reports what arrived. That leaves selection as the part which has to be
//! fail-closed, because the events a check sends are real system input and a
//! wrong target is somebody's application.
//!
//! Selection therefore never rests on a title alone. A candidate must be a
//! window, must carry the exact process-qualified title this module builds, must
//! advertise the system input this Adapter implements and no background delivery,
//! and must be the only such candidate. A check then confirms the selection
//! against the window's own deterministic content before it sends anything.

use std::fmt;

use mado_pilot_capture::TargetDescription;
use mado_pilot_core::{InputDelivery, InputOperationKind, PixelExtent, TargetKind};

/// The bundle identifier the fixture claims when it is run from an app bundle.
///
/// macOS grants authorization per application identity, so a fixture that is
/// bundled is one the operating system can recognize across runs. Running the
/// bare executable is supported and reports itself as unbundled instead.
pub const BUNDLE_IDENTIFIER: &str = "dev.mado-pilot.macos-input-fixture";

/// The stable prefix of the fixture's window title.
pub const TITLE_PREFIX: &str = "MadoPilot Input Fixture";

/// The one colour the fixture window is filled with, as 0xRRGGBB.
pub const FILL_RGB: u32 = 0x002E_5FA3;

/// The fixture window's content size, in points.
pub const WINDOW_POINTS: (f64, f64) = (640.0, 420.0);

/// The most events the fixture reports before it stops reporting.
pub const MAX_RECORDED_EVENTS: usize = 256;

/// What one observed event was. Mirrors `madopilot_macos_input_fixture.h`.
pub const EVENT_POINTER_MOVE: u32 = 1;
/// See [`EVENT_POINTER_MOVE`].
pub const EVENT_POINTER_PRESS: u32 = 2;
/// See [`EVENT_POINTER_MOVE`].
pub const EVENT_POINTER_RELEASE: u32 = 3;
/// See [`EVENT_POINTER_MOVE`].
pub const EVENT_POINTER_SCROLL: u32 = 4;
/// See [`EVENT_POINTER_MOVE`].
pub const EVENT_KEY_DOWN: u32 = 5;
/// See [`EVENT_POINTER_MOVE`].
pub const EVENT_KEY_UP: u32 = 6;
/// See [`EVENT_POINTER_MOVE`].
pub const EVENT_FLAGS_CHANGED: u32 = 7;

/// How far a captured channel may sit from the declared fill and still match.
///
/// The window is filled in sRGB and captured in the display's own space, so a
/// wide-gamut display returns a converted value rather than the declared one. The
/// load-bearing half of the check is uniformity — an application window is not one
/// flat colour — and the tolerance keeps that check from failing on a display
/// whose colour space is not sRGB.
pub const FILL_TOLERANCE: u8 = 24;

/// Non-sensitive information the fixture reports about one accepted event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventSummary {
    /// One of the `EVENT_*` values above.
    pub kind: u32,
    /// UTF-16 units the event carried. The characters are never reported.
    pub text_units: u32,
}

/// Why a check refused to choose a fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureSelectionError {
    /// Nothing discovered matches the fixture this check launched.
    NotFound,
    /// More than one candidate matches, so none of them is the one.
    Ambiguous,
}

impl fmt::Display for FixtureSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotFound => "no approved macOS input fixture matched",
            Self::Ambiguous => "more than one approved macOS input fixture matched",
        })
    }
}

impl std::error::Error for FixtureSelectionError {}

/// Returns the exact window title the fixture with `process_id` publishes.
#[must_use]
pub fn fixture_title(process_id: u32) -> String {
    format!("{TITLE_PREFIX} [{process_id}]")
}

/// Formats one observed event as the single line the fixture prints.
#[must_use]
pub fn format_event_line(summary: EventSummary) -> String {
    format!("event kind={} units={}", summary.kind, summary.text_units)
}

/// Reads one line the fixture printed, or `None` for anything else it prints.
#[must_use]
pub fn parse_event_line(line: &str) -> Option<EventSummary> {
    let rest = line.trim().strip_prefix("event ")?;
    let (kind, units) = rest.split_once(' ')?;
    Some(EventSummary {
        kind: kind.strip_prefix("kind=")?.parse().ok()?,
        text_units: units.strip_prefix("units=")?.parse().ok()?,
    })
}

/// Chooses the one discovered target that is this check's own fixture.
///
/// # Errors
///
/// Returns [`FixtureSelectionError::NotFound`] when nothing matches and
/// [`FixtureSelectionError::Ambiguous`] when more than one does. Both stop the
/// check before any input is sent, because a check that guessed would deliver
/// real system input to whatever it guessed.
pub fn select_unique_fixture(
    targets: &[TargetDescription],
    process_id: u32,
) -> Result<&TargetDescription, FixtureSelectionError> {
    let expected_title = fixture_title(process_id);
    let mut matches = targets.iter().filter(|target| {
        if target.name() != expected_title || target.capability().kind() != Some(TargetKind::Window)
        {
            return false;
        }
        let input = target.capability().input();
        InputOperationKind::ALL.iter().all(|operation| {
            input.supports(*operation, InputDelivery::System)
                && !input.supports(*operation, InputDelivery::BackgroundTarget)
        })
    });
    let first = matches.next().ok_or(FixtureSelectionError::NotFound)?;
    if matches.next().is_some() {
        return Err(FixtureSelectionError::Ambiguous);
    }
    Ok(first)
}

/// Confirms a captured frame is the fixture's own deterministic content.
///
/// `pixels` is BGRA8 at `stride` bytes per row covering `extent`. The central
/// quarter is sampled: every sampled pixel must be the same colour, and that
/// colour must sit within [`FILL_TOLERANCE`] of the declared fill.
#[must_use]
pub fn frame_is_fixture_content(pixels: &[u8], stride: usize, extent: PixelExtent) -> bool {
    let width = extent.width() as usize;
    let height = extent.height() as usize;
    if width < 8 || height < 8 || stride < width.saturating_mul(4) {
        return false;
    }
    if pixels.len() < stride.saturating_mul(height) {
        return false;
    }
    let expected = [
        u8::try_from(FILL_RGB & 0xFF).unwrap_or(0),
        u8::try_from((FILL_RGB >> 8) & 0xFF).unwrap_or(0),
        u8::try_from((FILL_RGB >> 16) & 0xFF).unwrap_or(0),
    ];

    let mut sampled: Option<[u8; 3]> = None;
    for row in (height / 4)..(height * 3 / 4) {
        for column in (width / 4)..(width * 3 / 4) {
            let start = row * stride + column * 4;
            let pixel = [pixels[start], pixels[start + 1], pixels[start + 2]];
            match sampled {
                None => sampled = Some(pixel),
                Some(first) if first == pixel => {}
                // An application window is not one flat colour, so a second
                // colour in the sampled region means this is not the fixture.
                Some(_) => return false,
            }
        }
    }
    let Some(observed) = sampled else {
        return false;
    };
    observed
        .iter()
        .zip(expected)
        .all(|(seen, want)| seen.abs_diff(want) <= FILL_TOLERANCE)
}

#[cfg(test)]
mod tests {
    use mado_pilot_capture::{CoordinateSupport, PixelFormat, TargetDescription};
    use mado_pilot_core::{
        CapabilitySupport, IdentityIssuer, InputDelivery, InputOperationKind, PixelExtent,
        TargetCapability, TargetKind,
    };

    use super::{
        EventSummary, FILL_RGB, FixtureSelectionError, format_event_line, frame_is_fixture_content,
        parse_event_line, select_unique_fixture,
    };
    use crate::input::input_capability;

    const PROCESS: u32 = 4242;

    fn described(name: &str, kind: TargetKind) -> TargetDescription {
        let id = IdentityIssuer::new()
            .issue_target(crate::provider::PROVIDER)
            .expect("issued");
        TargetDescription::new(
            id,
            name.to_owned(),
            PixelExtent::new(1280, 840),
            PixelFormat::Bgra8,
            CoordinateSupport::with_target_placement(),
        )
        .with_capability(TargetCapability::new(
            kind,
            CapabilitySupport::Supported,
            input_capability(kind),
        ))
    }

    fn fixture() -> TargetDescription {
        described(&super::fixture_title(PROCESS), TargetKind::Window)
    }

    fn flat_frame(fill: [u8; 3]) -> (Vec<u8>, usize, PixelExtent) {
        let extent = PixelExtent::new(64, 48);
        let stride = 64 * 4;
        let mut pixels = vec![0u8; stride * 48];
        for row in 0..48 {
            for column in 0..64 {
                let start = row * stride + column * 4;
                pixels[start] = fill[0];
                pixels[start + 1] = fill[1];
                pixels[start + 2] = fill[2];
                pixels[start + 3] = 0xFF;
            }
        }
        (pixels, stride, extent)
    }

    fn declared_bgr() -> [u8; 3] {
        [
            (FILL_RGB & 0xFF) as u8,
            ((FILL_RGB >> 8) & 0xFF) as u8,
            ((FILL_RGB >> 16) & 0xFF) as u8,
        ]
    }

    #[test]
    fn the_exact_process_qualified_fixture_is_selected_once() {
        let candidates = [
            described("Some Editor", TargetKind::Window),
            fixture(),
            described(&super::fixture_title(PROCESS + 1), TargetKind::Window),
        ];

        let chosen = select_unique_fixture(&candidates, PROCESS).expect("one approved fixture");

        assert_eq!(chosen.name(), super::fixture_title(PROCESS));
    }

    #[test]
    fn an_absent_or_repeated_fixture_stops_the_check_before_input() {
        assert_eq!(
            select_unique_fixture(&[described("Some Editor", TargetKind::Window)], PROCESS),
            Err(FixtureSelectionError::NotFound)
        );
        assert_eq!(
            select_unique_fixture(&[fixture(), fixture()], PROCESS),
            Err(FixtureSelectionError::Ambiguous),
            "two windows with the same title is exactly when guessing is worst"
        );
    }

    #[test]
    fn a_display_is_never_selected_however_it_is_named() {
        // A display advertises pointer input, so a check that matched on the title
        // and on "accepts some input" would take one. It accepts no keyboard or
        // text, which is what excludes it here.
        let named_like_the_fixture = described(&super::fixture_title(PROCESS), TargetKind::Display);

        assert_eq!(
            select_unique_fixture(&[named_like_the_fixture], PROCESS),
            Err(FixtureSelectionError::NotFound)
        );
    }

    #[test]
    fn selection_requires_the_capability_matrix_and_not_only_the_title() {
        let capture_only = {
            let id = IdentityIssuer::new()
                .issue_target(crate::provider::PROVIDER)
                .expect("issued");
            TargetDescription::new(
                id,
                super::fixture_title(PROCESS),
                PixelExtent::new(1280, 840),
                PixelFormat::Bgra8,
                CoordinateSupport::with_target_placement(),
            )
            .with_capability(TargetCapability::new(
                TargetKind::Window,
                CapabilitySupport::Supported,
                mado_pilot_core::InputCapability::none(),
            ))
        };

        assert_eq!(
            select_unique_fixture(&[capture_only], PROCESS),
            Err(FixtureSelectionError::NotFound),
            "a window that merely borrowed the title accepts none of the input the \
             fixture must accept"
        );
    }

    #[test]
    fn a_selected_fixture_never_advertises_background_delivery() {
        let chosen = fixture();
        let input = chosen.capability().input();

        for kind in InputOperationKind::ALL {
            assert!(!input.supports(kind, InputDelivery::BackgroundTarget));
        }
    }

    #[test]
    fn deterministic_content_confirms_a_selection_and_a_mixed_frame_refuses_it() {
        let (flat, stride, extent) = flat_frame(declared_bgr());
        assert!(frame_is_fixture_content(&flat, stride, extent));

        let (mut mixed, stride, extent) = flat_frame(declared_bgr());
        let centre = (extent.height() as usize / 2) * stride + (extent.width() as usize / 2) * 4;
        mixed[centre] = mixed[centre].wrapping_add(64);
        assert!(
            !frame_is_fixture_content(&mixed, stride, extent),
            "one differing pixel in the sampled region is not flat content"
        );

        let (other, stride, extent) = flat_frame([0x10, 0x20, 0x30]);
        assert!(
            !frame_is_fixture_content(&other, stride, extent),
            "a flat frame of another colour is some other window"
        );
    }

    #[test]
    fn a_colour_space_conversion_within_the_tolerance_still_matches() {
        let declared = declared_bgr();
        let converted = [
            declared[0].saturating_add(super::FILL_TOLERANCE),
            declared[1].saturating_sub(super::FILL_TOLERANCE),
            declared[2].saturating_add(super::FILL_TOLERANCE),
        ];
        let (frame, stride, extent) = flat_frame(converted);

        assert!(frame_is_fixture_content(&frame, stride, extent));
    }

    #[test]
    fn an_event_line_round_trips_without_carrying_any_text() {
        let summary = EventSummary {
            kind: super::EVENT_KEY_DOWN,
            text_units: 3,
        };
        let line = format_event_line(summary);

        assert_eq!(line, "event kind=5 units=3");
        assert_eq!(parse_event_line(&line), Some(summary));
        assert_eq!(parse_event_line("fixture-ready title=x"), None);
    }
}
