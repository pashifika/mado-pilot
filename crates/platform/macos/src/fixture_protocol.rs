//! Versioned control and observation protocol for the dedicated macOS input fixture.
//!
//! The fixture is a private qualification oracle, not a product input channel.
//! A harness owns its child process and stdin pipe, identifies the one discovered
//! window by that process-qualified title, confirms deterministic captured content,
//! then submits input only through the production Adapter route.
//!
//! Commands carry only a version, monotonic nonce, fixed action, and an optional
//! row token accepted only by the event-reset command. Results carry the same
//! identity, a bounded status, and before/after native window numbers. No command
//! or observation contains typed text, pixels, a path, or another application's
//! identity. The event receiver likewise reports only an event kind and UTF-16
//! unit count.
//!
//! Selection never rests on a title alone. A candidate must be the only window
//! with the exact title belonging to the owned child, advertise the qualified
//! process-directed pairs, and match the fixture's deterministic content.

use mado_pilot_capture::TargetDescription;
use std::fmt;

use mado_pilot_core::{
    CapabilitySupport, InputAddressScope, InputDelivery, InputOperationKind, PixelExtent,
    SubmissionEvidence, TargetId, TargetKind,
};

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

/// The distinct 0xRRGGBB fill used by the opt-in replacement oracle.
///
/// It is deliberately farther than [`FILL_TOLERANCE`] from [`FILL_RGB`] in two
/// channels, so an old retained filter publishing the successor is observable.
pub const REPLACEMENT_FILL_RGB: u32 = 0x00C4_5B2E;

/// The fixture window's content size, in points.
pub const WINDOW_POINTS: (f64, f64) = (640.0, 420.0);

/// The most events included in one process-wide fixture summary.
pub const MAX_RECORDED_EVENTS: usize = 1_024;

/// The largest UTF-16 unit count retained for one observed event.
///
/// Product text posts are chunked below this bound. Clamping an unrelated,
/// unexpectedly large AppKit event keeps the fixture oracle bounded without
/// retaining its characters.
pub const MAX_EVENT_TEXT_UNITS: u32 = 256;

/// Largest ready record the fixture gate will parse.
pub const MAX_READY_LINE_BYTES: usize = 1_024;

/// Version of the private harness-to-fixture control protocol.
pub const FIXTURE_CONTROL_VERSION: u32 = 11;

/// Largest command or result record accepted by either endpoint.
pub const MAX_CONTROL_LINE_BYTES: usize = 512;

/// Which independently selected fixture rendering mode initialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureMode {
    /// The unchanged AppKit window-background renderer.
    Default,
    /// The opt-in game-like renderer.
    GameLike,
}

impl FixtureMode {
    /// Returns the exact private-protocol spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::GameLike => "game-like",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "default" => Some(Self::Default),
            "game-like" => Some(Self::GameLike),
            _ => None,
        }
    }
}

/// Which native renderer produced the fixture's approved deterministic fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureRenderer {
    /// AppKit paints the window's background colour.
    AppKitBackground,
    /// An OpenGL-backed AppKit content view clears its drawable.
    OpenGl,
}

impl FixtureRenderer {
    /// Returns the exact private-protocol spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AppKitBackground => "appkit-background",
            Self::OpenGl => "opengl",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "appkit-background" => Some(Self::AppKitBackground),
            "opengl" => Some(Self::OpenGl),
            _ => None,
        }
    }
}

/// Exact bounded facts from one accepted `fixture-ready` record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixtureReadyFacts {
    window_number: u64,
    run_nonce: u64,
    mode: FixtureMode,
    renderer: FixtureRenderer,
    execution_context_approved: bool,
}

impl FixtureReadyFacts {
    /// Returns the native identity of the visible fixture window.
    #[must_use]
    pub const fn window_number(self) -> u64 {
        self.window_number
    }
    /// Returns the harness-issued identity shared by every record in this run.
    #[must_use]
    pub const fn run_nonce(self) -> u64 {
        self.run_nonce
    }

    /// Returns the independently reported fixture mode.
    #[must_use]
    pub const fn mode(self) -> FixtureMode {
        self.mode
    }

    /// Returns the renderer that initialized successfully.
    #[must_use]
    pub const fn renderer(self) -> FixtureRenderer {
        self.renderer
    }

    /// Returns whether launch and signing facts name the approved fixture bundle.
    #[must_use]
    pub const fn execution_context_is_approved(self) -> bool {
        self.execution_context_approved
    }
}

/// One fixed, payload-free fixture transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureCommandKind {
    /// Change the current window's deterministic fill without changing identity.
    Transition,
    /// Destroy the current window and create one same-process successor.
    Replace,
    /// Minimize the current window.
    Minimize,
    /// Restore the current window without activating the application.
    Restore,
    /// Move the main window by one fixed reversible offset.
    Move,
    /// Resize the main window by one fixed reversible extent.
    Resize,
    /// Open one ordinary auxiliary window in the fixture process.
    OpenAuxiliary,
    /// Close the fixture's ordinary auxiliary window.
    CloseAuxiliary,
    /// Close the main fixture window without terminating its process.
    Close,
    /// Move the main window to the next deterministically ordered public display.
    MoveToNextDisplay,
    /// Move the main window wholly outside the current public display union.
    MoveOffscreen,
    /// Restore a window moved off-screen to its exact prior origin.
    RestoreOnscreen,
    /// Return foreground ownership to the application active before fixture launch.
    YieldForeground,
    /// Clear the bounded process-wide event summary.
    ResetEvents,
    /// Restore the base fill and clear events before one language sample.
    PrepareLanguageFlow,
    /// Snapshot the bounded process-wide event summary.
    ReadEvents,
    /// Terminate the owned fixture.
    Stop,
}

impl FixtureCommandKind {
    /// Returns the stable protocol spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Transition => "transition",
            Self::Replace => "replace",
            Self::Minimize => "minimize",
            Self::Restore => "restore",
            Self::YieldForeground => "yield-foreground",
            Self::Move => "move",
            Self::Resize => "resize",
            Self::OpenAuxiliary => "open-auxiliary",
            Self::CloseAuxiliary => "close-auxiliary",
            Self::Close => "close",
            Self::MoveToNextDisplay => "move-to-next-display",
            Self::MoveOffscreen => "move-offscreen",
            Self::RestoreOnscreen => "restore-onscreen",
            Self::ResetEvents => "reset-events",
            Self::PrepareLanguageFlow => "prepare-language-flow",
            Self::ReadEvents => "read-events",
            Self::Stop => "stop",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "transition" => Some(Self::Transition),
            "replace" => Some(Self::Replace),
            "minimize" => Some(Self::Minimize),
            "restore" => Some(Self::Restore),
            "yield-foreground" => Some(Self::YieldForeground),
            "move" => Some(Self::Move),
            "resize" => Some(Self::Resize),
            "open-auxiliary" => Some(Self::OpenAuxiliary),
            "close-auxiliary" => Some(Self::CloseAuxiliary),
            "move-to-next-display" => Some(Self::MoveToNextDisplay),
            "move-offscreen" => Some(Self::MoveOffscreen),
            "restore-onscreen" => Some(Self::RestoreOnscreen),
            "reset-events" => Some(Self::ResetEvents),
            "prepare-language-flow" => Some(Self::PrepareLanguageFlow),
            "read-events" => Some(Self::ReadEvents),
            "close" => Some(Self::Close),
            "stop" => Some(Self::Stop),
            _ => None,
        }
    }

    /// Returns the native fixture command code.
    #[must_use]
    pub const fn as_raw(self) -> u32 {
        match self {
            Self::Transition => 1,
            Self::Replace => 2,
            Self::Minimize => 3,
            Self::Restore => 4,
            Self::Stop => 5,
            Self::YieldForeground => 6,
            Self::Move => 7,
            Self::Resize => 8,
            Self::OpenAuxiliary => 9,
            Self::CloseAuxiliary => 10,
            Self::MoveToNextDisplay => 12,
            Self::MoveOffscreen => 15,
            Self::RestoreOnscreen => 16,
            Self::ResetEvents => 13,
            Self::PrepareLanguageFlow => 17,
            Self::ReadEvents => 14,
            Self::Close => 11,
        }
    }
}

/// One decoded private fixture command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixtureCommand {
    /// Nonzero identity generated once by the owning harness for this run.
    pub run_nonce: u64,
    /// Monotonic harness-issued command identity. Zero is never valid.
    pub nonce: u64,
    /// Private expected-payload token for `ResetEvents`; zero for every other action.
    pub event_payload_tag: u64,
    /// The fixed transition to perform.
    pub kind: FixtureCommandKind,
}

/// Bounded, process-wide input facts at one command boundary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EventTotals {
    /// Pointer-move or drag events.
    pub pointer_moves: u32,
    /// Pointer-button press events.
    pub pointer_presses: u32,
    /// Pointer-button release events.
    pub pointer_releases: u32,
    /// Pointer scroll events.
    pub pointer_scrolls: u32,
    /// Key-down events.
    pub key_downs: u32,
    /// Key-up events.
    pub key_ups: u32,
    /// Modifier-flags-changed events.
    pub flags_changed: u32,
    /// UTF-16 units observed across key events, after per-event bounding.
    pub text_units: u64,
    /// Whether an event or unit count exceeded a fixed fixture bound.
    pub saturated: bool,
}

impl EventTotals {
    /// Returns the number of event records represented by this snapshot.
    #[must_use]
    pub const fn event_count(self) -> u64 {
        self.pointer_moves as u64
            + self.pointer_presses as u64
            + self.pointer_releases as u64
            + self.pointer_scrolls as u64
            + self.key_downs as u64
            + self.key_ups as u64
            + self.flags_changed as u64
    }
}

/// One decoded private fixture command result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixtureCommandResult {
    /// The per-run protocol identity.
    pub run_nonce: u64,
    /// The command identity.
    pub nonce: u64,
    /// Native fixture status; zero is success.
    pub status: u32,
    /// Window identity before the action, or zero when unavailable.
    pub before_window: u64,
    /// Window identity after the action, or zero when unavailable.
    pub after_window: u64,
    /// High half of the activity tag shared by every event in the current row.
    pub event_correlation: u32,
    /// Whether exact observed payloads and order match the digest in that tag.
    pub event_payload_matches: bool,
    /// Process-wide input summary at this command boundary.
    pub events: EventTotals,
}

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
    /// High half of the caller's opaque activity tag.
    pub correlation: u32,
}

const EVENT_FINGERPRINT_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const EVENT_FINGERPRINT_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fingerprint_u64(mut state: u64, value: u64) -> u64 {
    for byte in value.to_le_bytes() {
        state ^= u64::from(byte);
        state = state.wrapping_mul(EVENT_FINGERPRINT_PRIME);
    }
    state
}

/// Fingerprints one exact native event payload without retaining text content.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn event_payload_fingerprint(
    kind: u32,
    event_type: u64,
    flags: u64,
    button: u64,
    click_state: u64,
    x: f64,
    y: f64,
    horizontal: i64,
    vertical: i64,
    key_code: u64,
    text: &[u16],
) -> u64 {
    let fields = [
        u64::from(kind),
        event_type,
        flags,
        button,
        click_state,
        x.to_bits(),
        y.to_bits(),
        horizontal.cast_unsigned(),
        vertical.cast_unsigned(),
        key_code,
        text.len() as u64,
    ];
    let mut state = EVENT_FINGERPRINT_OFFSET;
    for field in fields {
        state = fingerprint_u64(state, field);
    }
    for unit in text {
        state = fingerprint_u64(state, u64::from(*unit));
    }
    state
}

/// Starts the order-sensitive digest for one correlated event row.
#[must_use]
pub fn begin_event_payload_digest(correlation: u32) -> u64 {
    fingerprint_u64(EVENT_FINGERPRINT_OFFSET, u64::from(correlation))
}

/// Adds one observed event fingerprint to a correlated row digest.
#[must_use]
pub fn extend_event_payload_digest(state: u64, fingerprint: u64) -> u64 {
    fingerprint_u64(state, fingerprint)
}

/// Reduces one correlated row digest to the value carried in an activity tag.
#[must_use]
pub const fn finish_event_payload_digest(state: u64) -> u32 {
    let bytes = state.to_le_bytes();
    u32::from_le_bytes([
        bytes[0] ^ bytes[4],
        bytes[1] ^ bytes[5],
        bytes[2] ^ bytes[6],
        bytes[3] ^ bytes[7],
    ])
}

/// Builds the activity tag that binds one row to exact native payloads and order.
#[must_use]
pub fn event_payload_activity_tag(correlation: u32, fingerprints: &[u64]) -> u64 {
    let digest = fingerprints.iter().copied().fold(
        begin_event_payload_digest(correlation),
        extend_event_payload_digest,
    );
    (u64::from(correlation) << 32) | u64::from(finish_event_payload_digest(digest))
}

/// Why a check refused to choose a fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureSelectionError {
    /// Nothing discovered matches the fixture this check launched.
    NotFound,
    /// More than one candidate matches, so none of them is the one.
    Ambiguous,
}

/// The selected window's captured pixels did not match the fixture contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixtureContentMismatch;

impl fmt::Display for FixtureContentMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .write_str("the selected macOS input fixture did not match its deterministic content")
    }
}

impl std::error::Error for FixtureContentMismatch {}

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
pub fn format_event_line(run_nonce: u64, summary: EventSummary) -> String {
    format!(
        "event run={run_nonce} correlation={} kind={} units={}",
        summary.correlation, summary.kind, summary.text_units
    )
}

/// Reads one line the fixture printed, or `None` for anything else it prints.
#[must_use]
pub fn parse_event_line(line: &str) -> Option<EventSummary> {
    parse_event_record(line).map(|(_run_nonce, summary)| summary)
}

/// Reads one event only when it belongs to `run_nonce`.
#[must_use]
pub fn parse_event_line_for_run(line: &str, run_nonce: u64) -> Option<EventSummary> {
    let (observed_run_nonce, summary) = parse_event_record(line)?;
    (observed_run_nonce == run_nonce).then_some(summary)
}

fn parse_event_record(line: &str) -> Option<(u64, EventSummary)> {
    let mut fields = line.trim_end().strip_prefix("event ")?.split(' ');
    let run_nonce = take_field(&mut fields, "run")?.parse::<u64>().ok()?;
    let summary = EventSummary {
        correlation: take_field(&mut fields, "correlation")?.parse().ok()?,
        kind: take_field(&mut fields, "kind")?.parse().ok()?,
        text_units: take_field(&mut fields, "units")?.parse().ok()?,
    };
    (run_nonce != 0
        && matches!(
            summary.kind,
            EVENT_POINTER_MOVE
                | EVENT_POINTER_PRESS
                | EVENT_POINTER_RELEASE
                | EVENT_POINTER_SCROLL
                | EVENT_KEY_DOWN
                | EVENT_KEY_UP
                | EVENT_FLAGS_CHANGED
        )
        && summary.text_units <= MAX_EVENT_TEXT_UNITS
        && fields.next().is_none())
    .then_some((run_nonce, summary))
}

/// Formats one bounded command for the owned fixture's private control channel.
#[must_use]
pub fn format_command_line(command: FixtureCommand) -> String {
    format!(
        "fixture-command version={} run={} nonce={} event-tag={} action={}",
        FIXTURE_CONTROL_VERSION,
        command.run_nonce,
        command.nonce,
        command.event_payload_tag,
        command.kind.as_str()
    )
}

/// Decodes one exact command record.
///
/// Missing, reordered, duplicated, extra, oversized, unsupported-version, and
/// zero-identity records are rejected.
#[must_use]
pub fn parse_command_line(line: &str) -> Option<FixtureCommand> {
    if line.len() > MAX_CONTROL_LINE_BYTES {
        return None;
    }
    let mut fields = line.trim_end().strip_prefix("fixture-command ")?.split(' ');
    if take_field(&mut fields, "version")?.parse::<u32>().ok()? != FIXTURE_CONTROL_VERSION {
        return None;
    }
    let run_nonce = take_field(&mut fields, "run")?.parse::<u64>().ok()?;
    let nonce = take_field(&mut fields, "nonce")?.parse::<u64>().ok()?;
    let event_payload_tag = take_field(&mut fields, "event-tag")?.parse::<u64>().ok()?;
    let kind = FixtureCommandKind::parse(take_field(&mut fields, "action")?)?;
    if run_nonce == 0
        || nonce == 0
        || (event_payload_tag != 0 && kind != FixtureCommandKind::ResetEvents)
    {
        return None;
    }
    fields.next().is_none().then_some(FixtureCommand {
        run_nonce,
        nonce,
        event_payload_tag,
        kind,
    })
}

/// Formats one bounded command result for the owned harness.
#[must_use]
pub fn format_command_result_line(result: FixtureCommandResult) -> String {
    let events = result.events;
    format!(
        "fixture-command-result version={} run={} nonce={} status={} before-window={} \
         after-window={} pointer-moves={} pointer-presses={} pointer-releases={} \
         pointer-scrolls={} key-downs={} key-ups={} flags-changed={} text-units={} saturated={} \
         event-correlation={} event-payload-matches={}",
        FIXTURE_CONTROL_VERSION,
        result.run_nonce,
        result.nonce,
        result.status,
        result.before_window,
        result.after_window,
        events.pointer_moves,
        events.pointer_presses,
        events.pointer_releases,
        events.pointer_scrolls,
        events.key_downs,
        events.key_ups,
        events.flags_changed,
        events.text_units,
        u8::from(events.saturated),
        result.event_correlation,
        u8::from(result.event_payload_matches),
    )
}

/// Decodes one exact command-result record.
#[must_use]
pub fn parse_command_result_line(line: &str) -> Option<FixtureCommandResult> {
    if line.len() > MAX_CONTROL_LINE_BYTES {
        return None;
    }
    let mut fields = line
        .trim_end()
        .strip_prefix("fixture-command-result ")?
        .split(' ');
    if take_field(&mut fields, "version")?.parse::<u32>().ok()? != FIXTURE_CONTROL_VERSION {
        return None;
    }
    let result = FixtureCommandResult {
        run_nonce: take_field(&mut fields, "run")?.parse().ok()?,
        nonce: take_field(&mut fields, "nonce")?.parse().ok()?,
        status: take_field(&mut fields, "status")?.parse().ok()?,
        before_window: take_field(&mut fields, "before-window")?.parse().ok()?,
        after_window: take_field(&mut fields, "after-window")?.parse().ok()?,
        events: EventTotals {
            pointer_moves: take_field(&mut fields, "pointer-moves")?.parse().ok()?,
            pointer_presses: take_field(&mut fields, "pointer-presses")?.parse().ok()?,
            pointer_releases: take_field(&mut fields, "pointer-releases")?.parse().ok()?,
            pointer_scrolls: take_field(&mut fields, "pointer-scrolls")?.parse().ok()?,
            key_downs: take_field(&mut fields, "key-downs")?.parse().ok()?,
            key_ups: take_field(&mut fields, "key-ups")?.parse().ok()?,
            flags_changed: take_field(&mut fields, "flags-changed")?.parse().ok()?,
            text_units: take_field(&mut fields, "text-units")?.parse().ok()?,
            saturated: match take_field(&mut fields, "saturated")? {
                "0" => false,
                "1" => true,
                _ => return None,
            },
        },
        event_correlation: take_field(&mut fields, "event-correlation")?.parse().ok()?,
        event_payload_matches: match take_field(&mut fields, "event-payload-matches")? {
            "0" => false,
            "1" => true,
            _ => return None,
        },
    };
    (result.run_nonce != 0
        && result.nonce != 0
        && result.events.event_count() <= MAX_RECORDED_EVENTS as u64
        && fields.next().is_none())
    .then_some(result)
}

fn take_field<'a>(fields: &mut impl Iterator<Item = &'a str>, name: &str) -> Option<&'a str> {
    let (actual, value) = fields.next()?.split_once('=')?;
    (actual == name && !value.is_empty()).then_some(value)
}

/// Decodes one exact bounded ready record into non-sensitive fixture facts.
///
/// Mode and renderer are an approved pair: the default mode can report only the
/// AppKit background renderer, and game-like mode can report only OpenGL. Missing,
/// reordered, duplicated, extra, oversized, and unsupported-version fields fail
/// closed.
#[must_use]
pub fn fixture_ready_facts(line: &str, process_id: u32) -> Option<FixtureReadyFacts> {
    if line.len() > MAX_READY_LINE_BYTES {
        return None;
    }
    let prefix = format!("fixture-ready title={} ", fixture_title(process_id));
    let mut fields = line.strip_prefix(&prefix)?.split(' ');
    if take_field(&mut fields, "pid")?.parse::<u32>().ok()? != process_id {
        return None;
    }
    let window_number = take_field(&mut fields, "window")?.parse::<u64>().ok()?;
    let run_nonce = take_field(&mut fields, "run")?.parse::<u64>().ok()?;
    if window_number == 0
        || run_nonce == 0
        || take_field(&mut fields, "control-version")?
            .parse::<u32>()
            .ok()?
            != FIXTURE_CONTROL_VERSION
    {
        return None;
    }
    let mode = FixtureMode::parse(take_field(&mut fields, "mode")?)?;
    let renderer = FixtureRenderer::parse(take_field(&mut fields, "renderer")?)?;
    if !matches!(
        (mode, renderer),
        (FixtureMode::Default, FixtureRenderer::AppKitBackground)
            | (FixtureMode::GameLike, FixtureRenderer::OpenGl)
    ) {
        return None;
    }
    let launch = take_field(&mut fields, "launch")?;
    let signature = take_field(&mut fields, "signature")?;
    let signing_identifier = take_field(&mut fields, "signing-identifier")?;
    if take_field(&mut fields, "bundle")? != BUNDLE_IDENTIFIER
        || take_field(&mut fields, "capacity")?.parse::<usize>().ok()? != MAX_RECORDED_EVENTS
        || fields.next().is_some()
    {
        return None;
    }
    Some(FixtureReadyFacts {
        window_number,
        run_nonce,
        mode,
        renderer,
        execution_context_approved: launch == "bundled"
            && matches!(signature, "ad-hoc" | "certificate-backed")
            && signing_identifier == BUNDLE_IDENTIFIER,
    })
}

/// Returns whether a bounded ready record exactly reports the approved signed
/// bundle context required before the interactive input path may open.
///
/// The same exact decoder validates protocol version, renderer facts, and the
/// complete record before this predicate considers the execution context.
#[must_use]
pub fn fixture_ready_context_is_approved(line: &str, process_id: u32) -> bool {
    fixture_ready_facts(line, process_id)
        .is_some_and(FixtureReadyFacts::execution_context_is_approved)
}

/// Chooses the one discovered target that is this check's authenticated fixture.
///
/// Title and capability matching only narrow discovery. `authenticates` must
/// bind the snapshot-owned target to the live audit-token-authenticated control
/// peer; it is evaluated before this function can return a target.
///
/// # Errors
///
/// Returns [`FixtureSelectionError::NotFound`] when nothing matches and
/// [`FixtureSelectionError::Ambiguous`] when more than one authenticated target
/// does. Both stop the check before any input is sent, because a check that
/// guessed would deliver real system input to whatever it guessed.
pub fn select_unique_fixture(
    targets: &[TargetDescription],
    process_id: u32,
    mut authenticates: impl FnMut(TargetId) -> bool,
) -> Result<&TargetDescription, FixtureSelectionError> {
    let expected_title = fixture_title(process_id);
    let mut matches = targets.iter().filter(|target| {
        if target.name() != expected_title || target.capability().kind() != Some(TargetKind::Window)
        {
            return false;
        }
        let input = target.capability().input();
        InputOperationKind::ALL.iter().all(|operation| {
            let process = input.pair(*operation, InputDelivery::ProcessDirected);
            input.pair(*operation, InputDelivery::System).support() == CapabilitySupport::Supported
                && input
                    .pair(*operation, InputDelivery::WindowMessage)
                    .support()
                    == CapabilitySupport::Unsupported
                && process.support() == CapabilitySupport::Unknown
                && process.address_scope() == InputAddressScope::OwningProcess
                && process.evidence() == Some(SubmissionEvidence::InvocationOnly)
        }) && authenticates(target.id())
    });
    let first = matches.next().ok_or(FixtureSelectionError::NotFound)?;
    if matches.next().is_some() {
        return Err(FixtureSelectionError::Ambiguous);
    }
    Ok(first)
}

/// Confirms a captured frame is the fixture's original deterministic content.
///
/// `pixels` is BGRA8 at `stride` bytes per row covering `extent`. The central
/// quarter is sampled: every sampled pixel must be the same colour, and that
/// colour must sit within [`FILL_TOLERANCE`] of [`FILL_RGB`].
#[must_use]
pub fn frame_is_fixture_content(pixels: &[u8], stride: usize, extent: PixelExtent) -> bool {
    frame_matches_fill(pixels, stride, extent, FILL_RGB)
}

/// Confirms a captured frame is the replacement fixture's distinct content.
///
/// This is the negative oracle for a retained `SCContentFilter`: after the
/// original window is destroyed, that filter must never publish this successor.
#[must_use]
pub fn frame_is_replacement_content(pixels: &[u8], stride: usize, extent: PixelExtent) -> bool {
    frame_matches_fill(pixels, stride, extent, REPLACEMENT_FILL_RGB)
}

fn frame_matches_fill(pixels: &[u8], stride: usize, extent: PixelExtent, fill: u32) -> bool {
    let width = extent.width() as usize;
    let height = extent.height() as usize;
    if width < 8 || height < 8 || stride < width.saturating_mul(4) {
        return false;
    }
    if pixels.len() < stride.saturating_mul(height) {
        return false;
    }
    let expected = [
        u8::try_from(fill & 0xFF).unwrap_or(0),
        u8::try_from((fill >> 8) & 0xFF).unwrap_or(0),
        u8::try_from((fill >> 16) & 0xFF).unwrap_or(0),
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

/// Runs `confirmed` only after captured pixels prove the selected fixture's
/// deterministic content.
///
/// The interactive harness places its input-controller open inside this gate,
/// so a capture, mapping, or content mismatch cannot leak a controller that can
/// post the first probe. Keeping the continuation inside the gate also gives the
/// non-interactive regression a direct observable: mismatched pixels invoke no
/// event-producing continuation.
///
/// # Errors
///
/// Returns [`FixtureContentMismatch`] without invoking `confirmed` when the
/// mapped BGRA8 frame does not satisfy [`frame_is_fixture_content`].
pub fn with_confirmed_fixture_content<T>(
    pixels: &[u8],
    stride: usize,
    extent: PixelExtent,
    confirmed: impl FnOnce() -> T,
) -> Result<T, FixtureContentMismatch> {
    if !frame_is_fixture_content(pixels, stride, extent) {
        return Err(FixtureContentMismatch);
    }
    Ok(confirmed())
}

#[cfg(test)]
mod tests {
    use mado_pilot_capture::{CoordinateSupport, PixelFormat, TargetDescription};
    use mado_pilot_core::{
        CapabilitySupport, IdentityIssuer, InputCapability, InputDelivery, InputOperationKind,
        PixelExtent, SubmissionEvidence, TargetCapability, TargetKind,
    };

    use super::{
        BUNDLE_IDENTIFIER, EventSummary, EventTotals, FILL_RGB, FIXTURE_CONTROL_VERSION,
        FixtureCommand, FixtureCommandKind, FixtureCommandResult, FixtureContentMismatch,
        FixtureMode, FixtureRenderer, FixtureSelectionError, MAX_RECORDED_EVENTS,
        REPLACEMENT_FILL_RGB, fixture_ready_context_is_approved, fixture_ready_facts,
        format_command_line, format_command_result_line, format_event_line,
        frame_is_fixture_content, frame_is_replacement_content, parse_command_line,
        parse_command_result_line, parse_event_line, parse_event_line_for_run,
        select_unique_fixture, with_confirmed_fixture_content,
    };
    use crate::PROVIDER;

    const PROCESS: u32 = 4242;

    fn input_capability(kind: TargetKind, process_directed: bool) -> InputCapability {
        let mut capability = InputCapability::none().with_pair(
            InputOperationKind::Pointer,
            InputDelivery::System,
            CapabilitySupport::Supported,
            SubmissionEvidence::InvocationOnly,
        );
        if kind != TargetKind::Window {
            return capability;
        }
        for operation in [InputOperationKind::Keyboard, InputOperationKind::Text] {
            capability = capability.with_pair(
                operation,
                InputDelivery::System,
                CapabilitySupport::Supported,
                SubmissionEvidence::InvocationOnly,
            );
        }
        if process_directed {
            for operation in InputOperationKind::ALL {
                capability = capability.with_pair(
                    operation,
                    InputDelivery::ProcessDirected,
                    CapabilitySupport::Unknown,
                    SubmissionEvidence::InvocationOnly,
                );
            }
        }
        capability
    }

    fn described(name: &str, kind: TargetKind) -> TargetDescription {
        let id = IdentityIssuer::new()
            .issue_target(PROVIDER)
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
            input_capability(kind, kind == TargetKind::Window),
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

    fn declared_bgr(fill: u32) -> [u8; 3] {
        [
            (fill & 0xFF) as u8,
            ((fill >> 8) & 0xFF) as u8,
            ((fill >> 16) & 0xFF) as u8,
        ]
    }

    fn ready_line_for(identifier: &str, mode: FixtureMode, renderer: FixtureRenderer) -> String {
        format!(
            "fixture-ready title={} pid={PROCESS} window=17 run=91 control-version={} mode={} \
             renderer={} launch=bundled signature=ad-hoc signing-identifier={identifier} \
             bundle={BUNDLE_IDENTIFIER} capacity={MAX_RECORDED_EVENTS}",
            super::fixture_title(PROCESS),
            FIXTURE_CONTROL_VERSION,
            mode.as_str(),
            renderer.as_str(),
        )
    }

    fn ready_line(identifier: &str) -> String {
        ready_line_for(
            identifier,
            FixtureMode::Default,
            FixtureRenderer::AppKitBackground,
        )
    }

    #[test]
    fn the_exact_process_qualified_fixture_is_selected_once() {
        let candidates = [
            described("Some Editor", TargetKind::Window),
            fixture(),
            described(&super::fixture_title(PROCESS + 1), TargetKind::Window),
        ];

        let chosen =
            select_unique_fixture(&candidates, PROCESS, |_| true).expect("one approved fixture");

        assert_eq!(chosen.name(), super::fixture_title(PROCESS));
    }

    #[test]
    fn an_absent_or_repeated_fixture_stops_the_check_before_input() {
        assert_eq!(
            select_unique_fixture(
                &[described("Some Editor", TargetKind::Window)],
                PROCESS,
                |_| true,
            ),
            Err(FixtureSelectionError::NotFound)
        );
        assert_eq!(
            select_unique_fixture(&[fixture(), fixture()], PROCESS, |_| true),
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
            select_unique_fixture(&[named_like_the_fixture], PROCESS, |_| true),
            Err(FixtureSelectionError::NotFound)
        );
    }

    #[test]
    fn selection_requires_the_capability_matrix_and_not_only_the_title() {
        let capture_only = {
            let id = IdentityIssuer::new()
                .issue_target(PROVIDER)
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
                InputCapability::none(),
            ))
        };

        assert_eq!(
            select_unique_fixture(&[capture_only], PROCESS, |_| true),
            Err(FixtureSelectionError::NotFound),
            "a window that merely borrowed the title accepts none of the input the \
             fixture must accept"
        );
    }

    #[test]
    fn a_foreign_owner_with_the_fixture_title_and_capabilities_is_not_selected() {
        let foreign = fixture();

        assert_eq!(
            select_unique_fixture(&[foreign], PROCESS, |_| false),
            Err(FixtureSelectionError::NotFound),
            "observable title and capability facts cannot authorize a process"
        );
    }

    #[test]
    fn a_selected_fixture_exposes_only_process_scoped_unqualified_delivery() {
        let chosen = fixture();
        let input = chosen.capability().input();

        for kind in InputOperationKind::ALL {
            assert_eq!(
                input.pair(kind, InputDelivery::WindowMessage).support(),
                CapabilitySupport::Unsupported
            );
            assert_eq!(
                input.pair(kind, InputDelivery::ProcessDirected).support(),
                CapabilitySupport::Unknown
            );
        }
    }

    #[test]
    fn deterministic_content_confirms_a_selection_and_a_mixed_frame_refuses_it() {
        let (flat, stride, extent) = flat_frame(declared_bgr(FILL_RGB));
        assert!(frame_is_fixture_content(&flat, stride, extent));
        assert!(!frame_is_replacement_content(&flat, stride, extent));

        let (replacement, stride, extent) = flat_frame(declared_bgr(REPLACEMENT_FILL_RGB));
        assert!(frame_is_replacement_content(&replacement, stride, extent));
        assert!(!frame_is_fixture_content(&replacement, stride, extent));

        let (mut mixed, stride, extent) = flat_frame(declared_bgr(FILL_RGB));
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
    fn a_title_and_capability_match_with_wrong_pixels_runs_zero_input_events() {
        use std::cell::Cell;

        let candidates = [fixture()];
        select_unique_fixture(&candidates, PROCESS, |_| true)
            .expect("the title and capability matrix deliberately match");
        let (wrong, stride, extent) = flat_frame([0x10, 0x20, 0x30]);
        let posted = Cell::new(0usize);

        let result = with_confirmed_fixture_content(&wrong, stride, extent, || {
            posted.set(posted.get() + 1);
        });

        assert_eq!(result, Err(FixtureContentMismatch));
        assert_eq!(
            posted.get(),
            0,
            "a convincing title and capability match still cannot reach input"
        );
    }

    #[test]
    fn a_colour_space_conversion_within_the_tolerance_still_matches() {
        let declared = declared_bgr(FILL_RGB);
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
            correlation: 42,
        };
        let line = format_event_line(99, summary);

        assert_eq!(line, "event run=99 correlation=42 kind=5 units=3");
        assert_eq!(parse_event_line(&line), Some(summary));
        assert_eq!(parse_event_line_for_run(&line, 99), Some(summary));
        assert_eq!(parse_event_line_for_run(&line, 100), None);
        assert_eq!(parse_event_line("fixture-ready title=x"), None);
    }

    #[test]
    fn commands_and_results_round_trip_with_exact_bounded_fields() {
        let command = FixtureCommand {
            run_nonce: 99,
            nonce: 7,
            event_payload_tag: 0,
            kind: FixtureCommandKind::Transition,
        };
        let command_line = format_command_line(command);
        assert_eq!(
            command_line,
            "fixture-command version=11 run=99 nonce=7 event-tag=0 action=transition"
        );
        assert_eq!(parse_command_line(&command_line), Some(command));

        let result = FixtureCommandResult {
            run_nonce: command.run_nonce,
            nonce: command.nonce,
            status: 0,
            before_window: 17,
            after_window: 17,
            event_correlation: 42,
            event_payload_matches: true,
            events: EventTotals {
                pointer_moves: 1,
                key_downs: 2,
                key_ups: 2,
                text_units: 6,
                ..EventTotals::default()
            },
        };
        let result_line = format_command_result_line(result);
        assert_eq!(
            result_line,
            "fixture-command-result version=11 run=99 nonce=7 status=0 before-window=17 \
             after-window=17 pointer-moves=1 pointer-presses=0 pointer-releases=0 \
             pointer-scrolls=0 key-downs=2 key-ups=2 flags-changed=0 text-units=6 saturated=0 \
             event-correlation=42 event-payload-matches=1"
        );
        assert_eq!(parse_command_result_line(&result_line), Some(result));

        let topology = FixtureCommand {
            run_nonce: 99,
            nonce: 8,
            event_payload_tag: 0,
            kind: FixtureCommandKind::MoveToNextDisplay,
        };
        let topology_line = format_command_line(topology);
        assert_eq!(
            topology_line,
            "fixture-command version=11 run=99 nonce=8 event-tag=0 \
             action=move-to-next-display"
        );
        assert_eq!(parse_command_line(&topology_line), Some(topology));

        let prepare = FixtureCommand {
            run_nonce: 99,
            nonce: 9,
            event_payload_tag: 0,
            kind: FixtureCommandKind::PrepareLanguageFlow,
        };
        let prepare_line = format_command_line(prepare);
        assert_eq!(
            prepare_line,
            "fixture-command version=11 run=99 nonce=9 event-tag=0 \
             action=prepare-language-flow"
        );
        assert_eq!(parse_command_line(&prepare_line), Some(prepare));
    }

    #[test]
    fn malformed_or_unbounded_control_records_fail_closed() {
        for line in [
            "fixture-command version=10 run=9 nonce=1 event-tag=0 action=transition",
            "fixture-command run=9 version=11 nonce=1 event-tag=0 action=transition",
            "fixture-command version=11 run=0 nonce=1 event-tag=0 action=transition",
            "fixture-command version=11 run=9 nonce=0 event-tag=0 action=transition",
            "fixture-command version=11 run=9 nonce=1 event-tag=1 action=transition",
            "fixture-command version=11 run=9 nonce=1 event-tag=0 action=unknown",
            "fixture-command version=11 run=9 nonce=1 event-tag=0 action=transition extra=1",
            "fixture-command-result version=11 run=9 nonce=1 status=0 before-window=17",
            "fixture-command-result version=11 run=9 nonce=1 status=0 before-window=17 after-window=17 extra=1",
        ] {
            assert!(
                parse_command_line(line).is_none() && parse_command_result_line(line).is_none(),
                "a malformed record was accepted: {line}"
            );
        }
        assert!(parse_command_line(&"x".repeat(super::MAX_CONTROL_LINE_BYTES + 1)).is_none());
        assert!(
            parse_command_result_line(&"x".repeat(super::MAX_CONTROL_LINE_BYTES + 1)).is_none()
        );
    }

    #[test]
    fn a_configured_ready_record_requires_exact_structured_context_fields() {
        let line = ready_line(BUNDLE_IDENTIFIER);
        assert!(fixture_ready_context_is_approved(&line, PROCESS));
        assert!(!fixture_ready_context_is_approved(
            &line.replace("launch=bundled", "launch=bundled-debug"),
            PROCESS
        ));
        assert!(!fixture_ready_context_is_approved(
            &line.replace("signature=ad-hoc", "signature=ad-hoc-debug"),
            PROCESS
        ));
        assert!(!fixture_ready_context_is_approved(
            &format!("{line} extra=field"),
            PROCESS
        ));
    }

    #[test]
    fn ready_mode_and_renderer_facts_are_exact_and_deterministic() {
        let default = fixture_ready_facts(&ready_line(BUNDLE_IDENTIFIER), PROCESS)
            .expect("the default ready facts decode");
        assert_eq!(default.window_number(), 17);
        assert_eq!(default.mode(), FixtureMode::Default);
        assert_eq!(default.renderer(), FixtureRenderer::AppKitBackground);
        assert!(default.execution_context_is_approved());

        let game_line = ready_line_for(
            BUNDLE_IDENTIFIER,
            FixtureMode::GameLike,
            FixtureRenderer::OpenGl,
        );
        let game =
            fixture_ready_facts(&game_line, PROCESS).expect("the game-like ready facts decode");
        assert_eq!(game.mode(), FixtureMode::GameLike);
        assert_eq!(game.renderer(), FixtureRenderer::OpenGl);
        assert!(game.execution_context_is_approved());

        for mismatched in [
            ready_line_for(
                BUNDLE_IDENTIFIER,
                FixtureMode::Default,
                FixtureRenderer::OpenGl,
            ),
            ready_line_for(
                BUNDLE_IDENTIFIER,
                FixtureMode::GameLike,
                FixtureRenderer::AppKitBackground,
            ),
            game_line.replace("renderer=opengl", "renderer=unsupported"),
            game_line.replace("mode=game-like", "mode=game"),
        ] {
            assert!(
                fixture_ready_facts(&mismatched, PROCESS).is_none(),
                "a mismatched renderer fact was accepted: {mismatched}"
            );
        }
    }

    #[test]
    fn signing_identifier_prefixes_and_suffixes_are_rejected() {
        assert!(!fixture_ready_context_is_approved(
            &ready_line(&format!("prefix.{BUNDLE_IDENTIFIER}")),
            PROCESS
        ));
        assert!(!fixture_ready_context_is_approved(
            &ready_line(&format!("{BUNDLE_IDENTIFIER}.suffix")),
            PROCESS
        ));
    }
}
