//! Bounded support contracts for the Windows native input fixtures.
//!
//! The exact `MadoPilotInputFixture` class accepts only the versioned
//! `WM_COPYDATA` protocol and acknowledges complete logical events. The
//! ordinary fixture uses legacy scalar messages solely to observe the public
//! best-effort `WindowMessage` route; its control messages are private test
//! coordination and are never used by production delivery.

use std::fmt;

use mado_pilot_capture::TargetDescription;
use mado_pilot_core::{CapabilitySupport, InputDelivery, InputOperationKind, TargetKind};
use mado_pilot_input::{InputEvent, InputFault, Key, Modifier, PointerButton};

pub const CLASS_NAME: &str = "MadoPilotInputFixture";
pub const TITLE_PREFIX: &str = "MadoPilot Input Fixture";
/// Window class used by the ordinary legacy-message evidence fixture.
pub const ORDINARY_CLASS_NAME: &str = "MadoPilotOrdinaryWindowMessageFixture";
/// Title prefix used by the ordinary legacy-message evidence fixture.
pub const ORDINARY_TITLE_PREFIX: &str = "MadoPilot Ordinary WindowMessage Fixture";
/// Requests one redacted observation-count report from the ordinary fixture.
pub const CONTROL_REPORT: u32 = 0x8201;
/// Makes the sibling fixture duplicate the retained target's descriptive title.
pub const CONTROL_DUPLICATE_METADATA: u32 = 0x8202;
/// Reparents the retained target under its same-process sibling.
pub const CONTROL_REPARENT_TARGET: u32 = 0x8203;
/// Replaces the retained target with a same-process window of the same class and title.
pub const CONTROL_REPLACE_TARGET: u32 = 0x8204;
/// Destroys the retained target while keeping adversarial fixtures alive.
pub const CONTROL_DESTROY_TARGET: u32 = 0x8205;
/// Blocks the fixture message pump for the duration carried in `wParam`.
pub const CONTROL_BLOCK_QUEUE: u32 = 0x8206;
/// Replaces a target repeatedly until its retained handle value recurs or the bound is exhausted.
pub const CONTROL_REUSE_STRESS: u32 = 0x8207;
/// Moves, resizes, and repaints the ordinary fixture on its owning GUI thread.
pub const CONTROL_SET_GEOMETRY: u32 = 0x8208;
/// Allows the test host in `wParam` to restore the owned foreground fixture.
pub const CONTROL_ALLOW_FOREGROUND: u32 = 0x8209;

pub const COPYDATA_TAG: usize = 0x4d50_4946;
pub const ACKNOWLEDGED: usize = 0x4d50_414b;
pub const VERSION: u32 = 1;
pub const MAX_RECORDED_EVENTS: usize = 1_024;
pub const MAX_PACKET_BYTES: usize = HEADER_BYTES + InputEvent::MAX_TEXT_CHARS * 4;

/// The fixture's ordinary deterministic client-area fill, as `0xRRGGBB`.
pub const FILL_RGB: u32 = 0x0020_4060;
/// The alternate deterministic fill used only by the opt-in benchmark mode.
pub const BENCHMARK_FILL_RGB: u32 = 0x00c4_5b2e;
/// The placement-specific marker used only by production benchmark fixtures.
pub const BENCHMARK_MARKER_RGB: u32 = 0x0040_d080;
/// Client-space X origin of the marker that remains on the negative-X display.
pub const BENCHMARK_LEFT_MARKER_X: i32 = 64;
/// Client-space X origin of the marker that remains on the positive-X display.
pub const BENCHMARK_RIGHT_MARKER_X: i32 = 1_200;
/// Client-space Y origin shared by both placement markers.
pub const BENCHMARK_MARKER_Y: i32 = 352;
/// Width and height of each square placement marker.
pub const BENCHMARK_MARKER_SIZE: i32 = 16;
/// Per-channel tolerance used when a captured benchmark frame is checked.
pub const FILL_TOLERANCE: u8 = 8;

const HEADER_BYTES: usize = 24;
const QUERY: u32 = 0;
const POINTER_MOVE: u32 = 1;
const POINTER_PRESS: u32 = 2;
const POINTER_RELEASE: u32 = 3;
const POINTER_SCROLL: u32 = 4;
const KEY_PRESS: u32 = 5;
const KEY_RELEASE: u32 = 6;
const TEXT: u32 = 7;

/// The event kind a benchmark expects for one pointer-move stimulus.
pub const EVENT_POINTER_MOVE: u32 = POINTER_MOVE;
/// The event kind a benchmark expects for one pointer-button press stimulus.
pub const EVENT_POINTER_PRESS: u32 = POINTER_PRESS;
/// The event kind a benchmark expects for one pointer-button release stimulus.
pub const EVENT_POINTER_RELEASE: u32 = POINTER_RELEASE;
/// The event kind a benchmark expects for one pointer-wheel stimulus.
pub const EVENT_POINTER_SCROLL: u32 = POINTER_SCROLL;
/// The event kind a benchmark expects for one key-down stimulus.
pub const EVENT_KEY_DOWN: u32 = KEY_PRESS;
/// The event kind a benchmark expects for the matching key release.
pub const EVENT_KEY_UP: u32 = KEY_RELEASE;
/// The event kind a benchmark expects for one direct-text stimulus.
pub const EVENT_TEXT: u32 = TEXT;

/// Non-sensitive information the fixture retains about one accepted event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventSummary {
    pub kind: u32,
    pub text_units: u32,
}

/// Formats one accepted event without exposing its key or text payload.
#[must_use]
pub fn format_event_line(summary: EventSummary) -> String {
    format!("event kind={} units={}", summary.kind, summary.text_units)
}

/// Parses one redacted event line printed by the opt-in benchmark fixture.
#[must_use]
pub fn parse_event_line(line: &str) -> Option<EventSummary> {
    let rest = line.trim().strip_prefix("event ")?;
    let (kind, units) = rest.split_once(' ')?;
    Some(EventSummary {
        kind: kind.strip_prefix("kind=")?.parse().ok()?,
        text_units: units.strip_prefix("units=")?.parse().ok()?,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureSelectionError {
    NotFound,
    Ambiguous,
}

impl fmt::Display for FixtureSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotFound => "no approved Windows input fixture matched",
            Self::Ambiguous => "more than one approved Windows input fixture matched",
        })
    }
}

impl std::error::Error for FixtureSelectionError {}

pub fn fixture_title(process_id: u32) -> String {
    format!("{TITLE_PREFIX} [{process_id}]")
}
/// Builds the exact title for an ordinary fixture instance.
#[must_use]
pub fn ordinary_fixture_title(token: &str) -> String {
    format!("{ORDINARY_TITLE_PREFIX} [{token}]")
}

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
            input
                .pair(*operation, InputDelivery::WindowMessage)
                .support()
                == CapabilitySupport::Supported
        })
    });
    let first = matches.next().ok_or(FixtureSelectionError::NotFound)?;
    if matches.next().is_some() {
        return Err(FixtureSelectionError::Ambiguous);
    }
    Ok(first)
}

pub fn query_packet() -> Vec<u8> {
    encode_header(QUERY, 0, 0, 0, 0)
}

pub fn encode_event(
    event: &InputEvent,
    screen_point: Option<(i32, i32)>,
) -> Result<Vec<u8>, InputFault> {
    event.check()?;
    match event {
        InputEvent::PointerMove(_) => {
            let (x, y) = screen_point.ok_or(InputFault::UnsupportedCoordinate)?;
            Ok(encode_header(POINTER_MOVE, x, y, 0, 0))
        }
        InputEvent::PointerPress(button) => {
            let (x, y) = screen_point.ok_or(InputFault::UnsupportedCoordinate)?;
            Ok(encode_header(
                POINTER_PRESS,
                x,
                y,
                pointer_button_code(*button)?,
                0,
            ))
        }
        InputEvent::PointerRelease(button) => {
            let (x, y) = screen_point.ok_or(InputFault::UnsupportedCoordinate)?;
            Ok(encode_header(
                POINTER_RELEASE,
                x,
                y,
                pointer_button_code(*button)?,
                0,
            ))
        }
        InputEvent::PointerScroll {
            horizontal,
            vertical,
        } => {
            let (x, y) = screen_point.ok_or(InputFault::UnsupportedCoordinate)?;
            let packed =
                u32::from(horizontal.cast_unsigned()) | (u32::from(vertical.cast_unsigned()) << 16);
            Ok(encode_header(POINTER_SCROLL, x, y, packed, 0))
        }
        InputEvent::KeyPress(key) => Ok(encode_header(KEY_PRESS, 0, 0, key_code(*key)?, 0)),
        InputEvent::KeyRelease(key) => Ok(encode_header(KEY_RELEASE, 0, 0, key_code(*key)?, 0)),
        InputEvent::Text(text) => {
            let units = text.encode_utf16().collect::<Vec<_>>();
            let unit_count =
                u32::try_from(units.len()).map_err(|_| InputFault::SequenceOutOfBounds)?;
            let mut packet = encode_header(TEXT, 0, 0, 0, unit_count);
            packet.reserve(units.len().saturating_mul(2));
            for unit in units {
                packet.extend_from_slice(&unit.to_le_bytes());
            }
            Ok(packet)
        }
        InputEvent::Delay(_) => Err(InputFault::UnsupportedCombination),
        _ => Err(InputFault::UnsupportedCombination),
    }
}

pub fn summarize(packet: &[u8]) -> Option<EventSummary> {
    if packet.len() < HEADER_BYTES
        || packet.len() > MAX_PACKET_BYTES
        || read_u32(packet, 0)? != VERSION
    {
        return None;
    }
    let kind = read_u32(packet, 4)?;
    let a = read_u32(packet, 8)?;
    let b = read_u32(packet, 12)?;
    let data = read_u32(packet, 16)?;
    let text_units = read_u32(packet, 20)?;
    let expected = if kind == TEXT {
        if a != 0
            || b != 0
            || data != 0
            || text_units == 0
            || usize::try_from(text_units).ok()? > InputEvent::MAX_TEXT_CHARS * 2
        {
            return None;
        }
        HEADER_BYTES.checked_add(usize::try_from(text_units).ok()?.checked_mul(2)?)?
    } else {
        if text_units != 0 {
            return None;
        }
        HEADER_BYTES
    };
    if packet.len() != expected || !valid_scalar_payload(kind, a, b, data) {
        return None;
    }
    if kind == TEXT {
        let units = packet[HEADER_BYTES..]
            .chunks_exact(2)
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]));
        let mut chars = 0usize;
        for character in char::decode_utf16(units) {
            character.ok()?;
            chars = chars.checked_add(1)?;
        }
        if chars == 0 || chars > InputEvent::MAX_TEXT_CHARS {
            return None;
        }
    }
    Some(EventSummary { kind, text_units })
}

pub fn is_query(summary: EventSummary) -> bool {
    summary.kind == QUERY
}

pub fn kind_name(summary: EventSummary) -> &'static str {
    match summary.kind {
        QUERY => "query",
        POINTER_MOVE => "pointer-move",
        POINTER_PRESS => "pointer-press",
        POINTER_RELEASE => "pointer-release",
        POINTER_SCROLL => "pointer-scroll",
        KEY_PRESS => "key-press",
        KEY_RELEASE => "key-release",
        TEXT => "text",
        _ => "invalid",
    }
}

fn valid_scalar_payload(kind: u32, a: u32, b: u32, data: u32) -> bool {
    match kind {
        QUERY => a == 0 && b == 0 && data == 0,
        POINTER_MOVE => data == 0,
        POINTER_PRESS | POINTER_RELEASE => matches!(data, 1..=3),
        POINTER_SCROLL => valid_scroll(data),
        KEY_PRESS | KEY_RELEASE => a == 0 && b == 0 && valid_key_code(data),
        TEXT => a == 0 && b == 0 && data == 0,
        _ => false,
    }
}

fn valid_key_code(code: u32) -> bool {
    if code & 0x8000_0000 != 0 {
        return char::from_u32(code & 0x7fff_ffff).is_some_and(|character| !character.is_control());
    }
    matches!(code, 0x101..=0x118 | 0x201..=0x204 | 0x301..=0x30e)
}

fn valid_scroll(data: u32) -> bool {
    let Ok(horizontal_bits) = u16::try_from(data & u32::from(u16::MAX)) else {
        return false;
    };
    let Ok(vertical_bits) = u16::try_from(data >> 16) else {
        return false;
    };
    let horizontal = i16::from_le_bytes(horizontal_bits.to_le_bytes());
    let vertical = i16::from_le_bytes(vertical_bits.to_le_bytes());
    (horizontal != 0 || vertical != 0)
        && horizontal.unsigned_abs() <= InputEvent::MAX_SCROLL_NOTCHES.unsigned_abs()
        && vertical.unsigned_abs() <= InputEvent::MAX_SCROLL_NOTCHES.unsigned_abs()
}

fn encode_header(kind: u32, a: i32, b: i32, data: u32, text_units: u32) -> Vec<u8> {
    let mut packet = Vec::with_capacity(HEADER_BYTES);
    packet.extend_from_slice(&VERSION.to_le_bytes());
    packet.extend_from_slice(&kind.to_le_bytes());
    packet.extend_from_slice(&a.to_le_bytes());
    packet.extend_from_slice(&b.to_le_bytes());
    packet.extend_from_slice(&data.to_le_bytes());
    packet.extend_from_slice(&text_units.to_le_bytes());
    packet
}

fn read_u32(packet: &[u8], offset: usize) -> Option<u32> {
    let bytes: [u8; 4] = packet
        .get(offset..offset.checked_add(4)?)?
        .try_into()
        .ok()?;
    Some(u32::from_le_bytes(bytes))
}

fn pointer_button_code(button: PointerButton) -> Result<u32, InputFault> {
    match button {
        PointerButton::Primary => Ok(1),
        PointerButton::Secondary => Ok(2),
        PointerButton::Middle => Ok(3),
        _ => Err(InputFault::UnsupportedCombination),
    }
}

fn key_code(key: Key) -> Result<u32, InputFault> {
    let code = match key {
        Key::Character(character) => 0x8000_0000 | u32::from(character),
        Key::Function(number) => 0x100 + u32::from(number),
        Key::Modifier(modifier) => {
            0x200
                + match modifier {
                    Modifier::Shift => 1,
                    Modifier::Control => 2,
                    Modifier::Alt => 3,
                    Modifier::Meta => 4,
                    _ => return Err(InputFault::UnsupportedCombination),
                }
        }
        Key::Enter => 0x301,
        Key::Tab => 0x302,
        Key::Backspace => 0x303,
        Key::Delete => 0x304,
        Key::Escape => 0x305,
        Key::Space => 0x306,
        Key::ArrowUp => 0x307,
        Key::ArrowDown => 0x308,
        Key::ArrowLeft => 0x309,
        Key::ArrowRight => 0x30a,
        Key::Home => 0x30b,
        Key::End => 0x30c,
        Key::PageUp => 0x30d,
        Key::PageDown => 0x30e,
        _ => return Err(InputFault::UnsupportedCombination),
    };
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::{
        EventSummary, FixtureSelectionError, HEADER_BYTES, encode_event, fixture_title,
        format_event_line, is_query, parse_event_line, query_packet, select_unique_fixture,
        summarize,
    };
    use mado_pilot_capture::{CoordinateSupport, PixelFormat, TargetDescription};
    use mado_pilot_core::{
        CapabilitySupport, IdentityIssuer, InputCapability, InputDelivery, InputOperationKind,
        PixelExtent, ProviderId, SubmissionEvidence, TargetCapability, TargetKind,
    };
    use mado_pilot_input::{InputEvent, InputFault, Key};

    #[test]
    fn query_and_event_packets_are_versioned_and_bounded() {
        let query = summarize(&query_packet()).expect("valid query");
        assert!(is_query(query));

        let packet = encode_event(&InputEvent::Text("A😀".to_owned()), None).expect("valid text");
        let summary = summarize(&packet).expect("valid event");
        assert_eq!(summary.text_units, 3);
        assert_eq!(packet.len(), HEADER_BYTES + 6);
        assert_eq!(
            encode_event(
                &InputEvent::Text("x".repeat(InputEvent::MAX_TEXT_CHARS + 1)),
                None,
            ),
            Err(InputFault::SequenceOutOfBounds)
        );
    }

    #[test]
    fn redacted_event_lines_round_trip_without_payload_text() {
        let summary = EventSummary {
            kind: 7,
            text_units: 3,
        };
        let line = format_event_line(summary);

        assert_eq!(line, "event kind=7 units=3");
        assert_eq!(parse_event_line(&line), Some(summary));
        assert_eq!(parse_event_line("event kind=7 units=3 trailing"), None);
    }

    #[test]
    fn malformed_or_trailing_payload_is_refused() {
        let mut packet =
            encode_event(&InputEvent::KeyPress(Key::Enter), None).expect("valid key packet");
        packet.push(0);
        assert_eq!(summarize(&packet), None);
    }

    #[test]
    fn invalid_scalar_payload_and_utf16_are_refused() {
        let mut invalid_key =
            encode_event(&InputEvent::KeyPress(Key::Enter), None).expect("valid key packet");
        invalid_key[16..20].copy_from_slice(&0xffff_u32.to_le_bytes());
        assert_eq!(summarize(&invalid_key), None);

        let mut invalid_text =
            encode_event(&InputEvent::Text("A".to_owned()), None).expect("valid text packet");
        invalid_text[HEADER_BYTES..].copy_from_slice(&0xd800_u16.to_le_bytes());
        assert_eq!(summarize(&invalid_text), None);
    }

    #[test]
    fn receiver_rechecks_key_and_scroll_bounds_independently() {
        let mut key =
            encode_event(&InputEvent::KeyPress(Key::Enter), None).expect("valid key packet");
        key[8..12].copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(summarize(&key), None);

        let mut control = encode_event(&InputEvent::KeyPress(Key::Character('A')), None)
            .expect("valid character packet");
        control[16..20].copy_from_slice(&(0x8000_0000_u32 | 0x1f).to_le_bytes());
        assert_eq!(summarize(&control), None);

        let mut scroll = encode_event(
            &InputEvent::PointerScroll {
                horizontal: 1,
                vertical: 0,
            },
            Some((0, 0)),
        )
        .expect("valid scroll packet");
        scroll[16..20].copy_from_slice(&0_u32.to_le_bytes());
        assert_eq!(summarize(&scroll), None);
        scroll[16..20].copy_from_slice(
            &u32::from(InputEvent::MAX_SCROLL_NOTCHES.cast_unsigned() + 1).to_le_bytes(),
        );
        assert_eq!(summarize(&scroll), None);
    }

    #[test]
    fn fixture_selection_requires_one_exact_capable_identity() {
        let process_id = 42;
        let expected = fixture_title(process_id);
        let ordinary = description(&expected, false);
        let fixture = description(&expected, true);
        let unrelated = description("another title", true);
        let display = description_with_kind(&expected, true, TargetKind::Display);

        let candidates = [ordinary.clone(), unrelated, fixture.clone()];
        let selected =
            select_unique_fixture(&candidates, process_id).expect("one approved fixture");
        assert_eq!(selected.id(), fixture.id());
        assert_eq!(
            select_unique_fixture(&[ordinary], process_id),
            Err(FixtureSelectionError::NotFound)
        );
        assert_eq!(
            select_unique_fixture(&[display], process_id),
            Err(FixtureSelectionError::NotFound)
        );
        assert_eq!(
            select_unique_fixture(&[fixture.clone(), fixture], process_id),
            Err(FixtureSelectionError::Ambiguous)
        );
    }

    fn description(name: &str, window_message: bool) -> TargetDescription {
        description_with_kind(name, window_message, TargetKind::Window)
    }

    fn description_with_kind(
        name: &str,
        window_message: bool,
        kind: TargetKind,
    ) -> TargetDescription {
        let target = IdentityIssuer::new()
            .issue_target(ProviderId::new("windows"))
            .expect("issued target");
        let mut input = InputCapability::none();
        if window_message {
            for operation in InputOperationKind::ALL {
                input = input.with_pair(
                    operation,
                    InputDelivery::WindowMessage,
                    CapabilitySupport::Supported,
                    SubmissionEvidence::TargetProtocolAcknowledgement,
                );
            }
        }
        TargetDescription::new(
            target,
            name,
            PixelExtent::new(1, 1),
            PixelFormat::Bgra8,
            CoordinateSupport::with_target_placement(),
        )
        .with_capability(TargetCapability::new(
            kind,
            CapabilitySupport::Supported,
            input,
        ))
    }
}
