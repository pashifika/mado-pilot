//! The frozen numeric vocabulary: statuses, categories, faults, stages, and
//! flag bits.
//!
//! Layout and numbers are two independent halves of the freeze, and the
//! cross-language mechanism covers only the first: `tests/c/madopilot-abi-layout.c`
//! reports sizes, alignments, and offsets, so a header-versus-Rust renumbering
//! of a status passes it. This file is the second half. It has no C toolchain
//! to compare against, so it compares three things instead — what the accepted
//! ABI freeze records fixed, what the Rust definitions say, and what the
//! hand-written header declares — and requires all three to agree in both
//! directions.
//!
//! A number here is permanent for ABI major 1. A caller that switches on `7`
//! and later gets a different meaning has no way to detect it, which is the
//! failure the freeze exists to prevent.

use madopilot::*;

/// The header, read as text rather than compiled.
///
/// Compiling it is `examples/c-abi-check.rs`'s job and needs a C toolchain;
/// what this file needs is the declared value of each name, which the source
/// states directly.
const HEADER: &str = include_str!("../include/madopilot/madopilot.h");

/// Header symbols that are not frozen numbers.
///
/// Anything the header declares with a `MADOPILOT_` name and no integer literal
/// has to be listed here, so a new one cannot be quietly skipped by the parser
/// below.
///
/// Preprocessor spellings only. An enumerator always has a number whether or
/// not the header writes one out, so listing one here would drop a frozen value
/// out of the freeze rather than record that it has none;
/// `no_enumerator_is_excused_from_the_freeze` refuses that entry.
const NOT_A_NUMBER: &[&str] = &[
    // The include guard.
    "MADOPILOT_MADOPILOT_H",
    // Per-platform linkage, one branch per toolchain.
    "MADOPILOT_EXPORT",
    // An `offsetof` expression rather than a literal. Its value is pinned by
    // `tests/layout.rs::the_information_prefix_ends_at_status_text` against the
    // member it names, and by the frozen layout report.
    "MADOPILOT_API_SIZE_INFORMATION",
    "MADOPILOT_API_SIZE_ABI_1_0",
    "MADOPILOT_API_SIZE_SESSION_OPEN_WITH_INPUT",
    "MADOPILOT_API_SIZE_ENGINE_CAPABILITIES",
    "MADOPILOT_API_SIZE_ENGINE_PERMISSION",
    "MADOPILOT_API_SIZE_TARGET_LIST_CAPABILITY",
    "MADOPILOT_API_SIZE_ENGINE_INPUT_DESCRIPTOR",
    "MADOPILOT_API_SIZE_SESSION_INPUT_DESCRIPTOR",
    "MADOPILOT_API_SIZE_SESSION_SEND_INPUT",
];

macro_rules! frozen {
    ($($name:ident = $value:literal,)*) => {
        /// Every frozen number: its name, what this library defines it as, and
        /// what the accepted ABI freeze records fixed it at.
        ///
        /// The middle column is the Rust constant itself, so this table cannot
        /// be satisfied by editing it alone — the value has to be a literal
        /// copied from the ADR, and the constant has to still equal it.
        const FROZEN: &[(&str, i128, i128)] = &[
            $((stringify!($name), $name as i128, $value),)*
        ];
    };
}

frozen! {
    // The ABI this header and this library are.
    MADOPILOT_ABI_MAJOR = 1,
    MADOPILOT_ABI_MINOR = 1,

    // Statuses. Thirteen values, permanent for ABI major 1.
    MADOPILOT_STATUS_OK = 0,
    MADOPILOT_STATUS_INVALID_ARGUMENT = 1,
    MADOPILOT_STATUS_UNSUPPORTED = 2,
    MADOPILOT_STATUS_CANCELLED = 3,
    MADOPILOT_STATUS_DEADLINE_EXCEEDED = 4,
    MADOPILOT_STATUS_CLOSED = 5,
    MADOPILOT_STATUS_TARGET_LOST = 6,
    MADOPILOT_STATUS_LIMIT_EXCEEDED = 7,
    MADOPILOT_STATUS_CAPTURE_FAILED = 8,
    MADOPILOT_STATUS_ASSET_INVALID = 9,
    MADOPILOT_STATUS_VISION_FAILED = 10,
    MADOPILOT_STATUS_INTERNAL = 11,
    MADOPILOT_STATUS_INTERNAL_PANIC = 12,
    MADOPILOT_STATUS_INPUT_FAILED = 13,

    // The subsystem a failure came from.
    MADOPILOT_ERROR_CATEGORY_UNSPECIFIED = 0,
    MADOPILOT_ERROR_CATEGORY_ABI = 1,
    MADOPILOT_ERROR_CATEGORY_OPERATION = 2,
    MADOPILOT_ERROR_CATEGORY_ENGINE = 3,
    MADOPILOT_ERROR_CATEGORY_CAPTURE = 4,
    MADOPILOT_ERROR_CATEGORY_ASSET = 5,
    MADOPILOT_ERROR_CATEGORY_VISION = 6,
    MADOPILOT_ERROR_CATEGORY_GEOMETRY = 7,
    MADOPILOT_ERROR_CATEGORY_PERMISSION = 8,
    MADOPILOT_ERROR_CATEGORY_INPUT = 9,

    // Coordinate spaces. Every public rectangle names one.
    MADOPILOT_SPACE_CAPTURE_PIXELS = 0,
    MADOPILOT_SPACE_FRAME_NORMALIZED = 1,
    MADOPILOT_SPACE_TARGET_NORMALIZED = 2,
    MADOPILOT_SPACE_TARGET_LOGICAL = 3,
    MADOPILOT_SPACE_DESKTOP_LOGICAL = 4,

    MADOPILOT_PIXEL_FORMAT_RGBA8 = 0,
    MADOPILOT_PIXEL_FORMAT_BGRA8 = 1,

    MADOPILOT_CLIP_POLICY_REJECT = 0,
    MADOPILOT_CLIP_POLICY_CLIP = 1,

    MADOPILOT_CONTINUITY_CONTINUOUS = 0,
    MADOPILOT_CONTINUITY_DISCONTINUOUS = 1,

    MADOPILOT_SUPPRESSION_DROP_OVERLAPPING = 0,
    MADOPILOT_SUPPRESSION_KEEP_ALL = 1,

    MADOPILOT_SOURCE_REPLAY_MEMORY = 0,
    MADOPILOT_SOURCE_REPLAY_DIRECTORY = 1,
    MADOPILOT_SOURCE_NATIVE_WINDOWS = 2,
    MADOPILOT_SOURCE_NATIVE_MACOS = 3,

    MADOPILOT_PACKAGE_SOURCE_DIRECTORY = 0,
    MADOPILOT_PACKAGE_SOURCE_ARCHIVE_FILE = 1,
    MADOPILOT_PACKAGE_SOURCE_ARCHIVE_BYTES = 2,

    // Which rule an asset package broke. Twenty-nine values, appended to only.
    MADOPILOT_ASSET_FAULT_UNKNOWN = 0,
    MADOPILOT_ASSET_FAULT_LIMIT_ABOVE_CEILING = 1,
    MADOPILOT_ASSET_FAULT_SOURCE_UNREADABLE = 2,
    MADOPILOT_ASSET_FAULT_SOURCE_CHANGED = 3,
    MADOPILOT_ASSET_FAULT_MALFORMED_ARCHIVE = 4,
    MADOPILOT_ASSET_FAULT_UNSUPPORTED_COMPRESSION_METHOD = 5,
    MADOPILOT_ASSET_FAULT_ENCRYPTED_ENTRY = 6,
    MADOPILOT_ASSET_FAULT_ARCHIVE_LIMIT = 7,
    MADOPILOT_ASSET_FAULT_UNSAFE_PATH = 8,
    MADOPILOT_ASSET_FAULT_DUPLICATE_PATH = 9,
    MADOPILOT_ASSET_FAULT_UNSUPPORTED_ENTRY_TYPE = 10,
    MADOPILOT_ASSET_FAULT_DECLARED_SIZE_MISMATCH = 11,
    MADOPILOT_ASSET_FAULT_MISSING_MANIFEST = 12,
    MADOPILOT_ASSET_FAULT_MALFORMED_MANIFEST = 13,
    MADOPILOT_ASSET_FAULT_MISSING_SCHEMA_VERSION = 14,
    MADOPILOT_ASSET_FAULT_UNSUPPORTED_SCHEMA_VERSION = 15,
    MADOPILOT_ASSET_FAULT_DUPLICATE_IDENTITY = 16,
    MADOPILOT_ASSET_FAULT_MISSING_ENTRY = 17,
    MADOPILOT_ASSET_FAULT_UNKNOWN_TEMPLATE = 18,
    MADOPILOT_ASSET_FAULT_UNSUPPORTED_SOURCE = 19,
    MADOPILOT_ASSET_FAULT_INVALID_TEMPLATE_METADATA = 20,
    MADOPILOT_ASSET_FAULT_UNSUPPORTED_TEMPLATE_SPACE = 21,
    MADOPILOT_ASSET_FAULT_UNSUPPORTED_HASH_ALGORITHM = 22,
    MADOPILOT_ASSET_FAULT_MALFORMED_HASH = 23,
    MADOPILOT_ASSET_FAULT_HASH_MISMATCH = 24,
    MADOPILOT_ASSET_FAULT_UNSUPPORTED_CONTENT_ENCODING = 25,
    MADOPILOT_ASSET_FAULT_ARITHMETIC_OVERFLOW = 26,
    MADOPILOT_ASSET_FAULT_CANCELLED = 27,
    MADOPILOT_ASSET_FAULT_DEADLINE_EXCEEDED = 28,

    // How far package loading had got when it refused.
    MADOPILOT_ASSET_STAGE_UNKNOWN = 0,
    MADOPILOT_ASSET_STAGE_CONFIGURATION = 1,
    MADOPILOT_ASSET_STAGE_SOURCE = 2,
    MADOPILOT_ASSET_STAGE_DIRECTORY_PRE_PARSE = 3,
    MADOPILOT_ASSET_STAGE_DIRECTORY_OPEN = 4,
    MADOPILOT_ASSET_STAGE_ENTRY_METADATA = 5,
    MADOPILOT_ASSET_STAGE_MANIFEST = 6,
    MADOPILOT_ASSET_STAGE_EXPANSION = 7,
    MADOPILOT_ASSET_STAGE_COMMIT = 8,

    // Phase 2 native capability, permission, and input values.
    MADOPILOT_DIAGNOSTIC_UNSPECIFIED = 0,
    MADOPILOT_DIAGNOSTIC_PERMISSION_DENIED = 1,
    MADOPILOT_DIAGNOSTIC_PERMISSION_UNDETERMINED = 2,
    MADOPILOT_DIAGNOSTIC_CAPABILITY_UNAVAILABLE = 3,
    MADOPILOT_DIAGNOSTIC_TARGET_LOST = 4,
    MADOPILOT_DIAGNOSTIC_PLATFORM_FAILURE = 5,
    MADOPILOT_DIAGNOSTIC_CONFIGURATION = 6,

    MADOPILOT_PERMISSION_KIND_UNSPECIFIED = 0,
    MADOPILOT_PERMISSION_KIND_SCREEN_CAPTURE = 1,
    MADOPILOT_PERMISSION_KIND_INPUT_CONTROL = 2,

    MADOPILOT_PERMISSION_STATE_UNKNOWN = 0,
    MADOPILOT_PERMISSION_STATE_GRANTED = 1,
    MADOPILOT_PERMISSION_STATE_NOT_GRANTED = 2,
    MADOPILOT_PERMISSION_STATE_UNAVAILABLE = 3,

    MADOPILOT_TARGET_KIND_UNKNOWN = 0,
    MADOPILOT_TARGET_KIND_WINDOW = 1,
    MADOPILOT_TARGET_KIND_DISPLAY = 2,

    MADOPILOT_CAPABILITY_UNKNOWN = 0,
    MADOPILOT_CAPABILITY_SUPPORTED = 1,
    MADOPILOT_CAPABILITY_UNSUPPORTED = 2,

    MADOPILOT_INPUT_OPERATION_UNKNOWN = 0,
    MADOPILOT_INPUT_OPERATION_POINTER = 1,
    MADOPILOT_INPUT_OPERATION_KEYBOARD = 2,
    MADOPILOT_INPUT_OPERATION_TEXT = 3,

    MADOPILOT_INPUT_DELIVERY_NONE = 0,
    MADOPILOT_INPUT_DELIVERY_SYSTEM = 1,
    MADOPILOT_INPUT_DELIVERY_BACKGROUND_TARGET = 2,

    MADOPILOT_INPUT_OPTIONAL = 0,
    MADOPILOT_INPUT_REQUIRED = 1,

    MADOPILOT_FOCUS_PRESERVE = 0,
    MADOPILOT_FOCUS_REQUIRE_FOCUSED = 1,
    MADOPILOT_FOCUS_ACTIVATE_IF_REQUIRED = 2,

    MADOPILOT_GEOMETRY_REPROJECT_CURRENT = 0,
    MADOPILOT_GEOMETRY_REQUIRE_UNCHANGED = 1,
    MADOPILOT_GEOMETRY_USE_FRAME_SNAPSHOT = 2,

    MADOPILOT_POINTER_BUTTON_UNKNOWN = 0,
    MADOPILOT_POINTER_BUTTON_PRIMARY = 1,
    MADOPILOT_POINTER_BUTTON_SECONDARY = 2,
    MADOPILOT_POINTER_BUTTON_MIDDLE = 3,

    MADOPILOT_MODIFIER_UNKNOWN = 0,
    MADOPILOT_MODIFIER_SHIFT = 1,
    MADOPILOT_MODIFIER_CONTROL = 2,
    MADOPILOT_MODIFIER_ALT = 3,
    MADOPILOT_MODIFIER_META = 4,

    MADOPILOT_KEY_UNKNOWN = 0,
    MADOPILOT_KEY_CHARACTER = 1,
    MADOPILOT_KEY_FUNCTION = 2,
    MADOPILOT_KEY_MODIFIER = 3,
    MADOPILOT_KEY_ENTER = 4,
    MADOPILOT_KEY_TAB = 5,
    MADOPILOT_KEY_BACKSPACE = 6,
    MADOPILOT_KEY_DELETE = 7,
    MADOPILOT_KEY_ESCAPE = 8,
    MADOPILOT_KEY_SPACE = 9,
    MADOPILOT_KEY_ARROW_UP = 10,
    MADOPILOT_KEY_ARROW_DOWN = 11,
    MADOPILOT_KEY_ARROW_LEFT = 12,
    MADOPILOT_KEY_ARROW_RIGHT = 13,
    MADOPILOT_KEY_HOME = 14,
    MADOPILOT_KEY_END = 15,
    MADOPILOT_KEY_PAGE_UP = 16,
    MADOPILOT_KEY_PAGE_DOWN = 17,

    MADOPILOT_INPUT_EVENT_UNKNOWN = 0,
    MADOPILOT_INPUT_EVENT_POINTER_MOVE = 1,
    MADOPILOT_INPUT_EVENT_POINTER_PRESS = 2,
    MADOPILOT_INPUT_EVENT_POINTER_RELEASE = 3,
    MADOPILOT_INPUT_EVENT_POINTER_SCROLL = 4,
    MADOPILOT_INPUT_EVENT_KEY_PRESS = 5,
    MADOPILOT_INPUT_EVENT_KEY_RELEASE = 6,
    MADOPILOT_INPUT_EVENT_TEXT = 7,
    MADOPILOT_INPUT_EVENT_DELAY = 8,

    MADOPILOT_INPUT_MAX_EVENTS = 256,
    MADOPILOT_INPUT_MAX_TEXT_CHARS = 4096,
    MADOPILOT_INPUT_MAX_TEXT_UTF8_BYTES = 16384,
    MADOPILOT_INPUT_MAX_DELAY_NANOS = 5000000000,
    MADOPILOT_INPUT_MAX_SCROLL_NOTCHES = 120,
    MADOPILOT_INPUT_MIN_FUNCTION_KEY = 1,
    MADOPILOT_INPUT_MAX_FUNCTION_KEY = 24,
    MADOPILOT_INPUT_MAX_CLEANUP_EVENTS = 256,
    MADOPILOT_INPUT_MAX_CLEANUP_NANOS = 250000000,

    MADOPILOT_SEQUENCE_UNEXECUTED = 0,
    MADOPILOT_SEQUENCE_COMPLETE = 1,
    MADOPILOT_SEQUENCE_PARTIAL = 2,

    MADOPILOT_CLEANUP_NOT_NEEDED = 0,
    MADOPILOT_CLEANUP_COMPLETE = 1,
    MADOPILOT_CLEANUP_INCOMPLETE = 2,
    MADOPILOT_CLEANUP_EXHAUSTED = 3,

    MADOPILOT_INPUT_FAULT_NONE = 0,
    MADOPILOT_INPUT_FAULT_FOREIGN_TARGET = 1,
    MADOPILOT_INPUT_FAULT_UNKNOWN_TARGET = 2,
    MADOPILOT_INPUT_FAULT_TARGET_LOST = 3,
    MADOPILOT_INPUT_FAULT_PROVIDER_MISMATCH = 4,
    MADOPILOT_INPUT_FAULT_UNSUPPORTED_COMBINATION = 5,
    MADOPILOT_INPUT_FAULT_INVALID_DELIVERY_PLAN = 6,
    MADOPILOT_INPUT_FAULT_DELIVERY_UNAVAILABLE = 7,
    MADOPILOT_INPUT_FAULT_SEQUENCE_OUT_OF_BOUNDS = 8,
    MADOPILOT_INPUT_FAULT_UNSUPPORTED_COORDINATE = 9,
    MADOPILOT_INPUT_FAULT_MISSING_COORDINATE_SOURCE = 10,
    MADOPILOT_INPUT_FAULT_GEOMETRY_CHANGED = 11,
    MADOPILOT_INPUT_FAULT_FOCUS_REQUIRED = 12,
    MADOPILOT_INPUT_FAULT_FOCUS_REFUSED = 13,
    MADOPILOT_INPUT_FAULT_NOT_AUTHORIZED = 14,
    MADOPILOT_INPUT_FAULT_POLICY_REFUSED = 15,
    MADOPILOT_INPUT_FAULT_CONTROLLER_CLOSED = 16,
    MADOPILOT_INPUT_FAULT_CANCELLED = 17,
    MADOPILOT_INPUT_FAULT_DEADLINE_EXCEEDED = 18,
    MADOPILOT_INPUT_FAULT_DELIVERY_FAILED = 19,

    MADOPILOT_INPUT_PAIR_POINTER_SYSTEM = 0x1,
    MADOPILOT_INPUT_PAIR_POINTER_BACKGROUND = 0x2,
    MADOPILOT_INPUT_PAIR_KEYBOARD_SYSTEM = 0x4,
    MADOPILOT_INPUT_PAIR_KEYBOARD_BACKGROUND = 0x8,
    MADOPILOT_INPUT_PAIR_TEXT_SYSTEM = 0x10,
    MADOPILOT_INPUT_PAIR_TEXT_BACKGROUND = 0x20,
    MADOPILOT_INPUT_PAIRS_ALL = 0x3f,

    MADOPILOT_INPUT_FOCUS_SYSTEM = 0x1,
    MADOPILOT_INPUT_FOCUS_BACKGROUND = 0x2,
    MADOPILOT_INPUT_FOCUS_ALL = 0x3,

    // Flag bits. A caller masks with these, so a moved bit is as breaking as a
    // renumbered status.
    MADOPILOT_OPERATION_HAS_DEADLINE = 0x1,
    MADOPILOT_OPEN_HAS_REQUIRED_FORMAT = 0x1,
    MADOPILOT_OPEN_HAS_PREFERRED_FORMAT = 0x2,
    MADOPILOT_MAP_HAS_REGION = 0x1,
    MADOPILOT_FIND_HAS_REGION = 0x1,
    MADOPILOT_MATCH_HAS_MIN_SCORE = 0x1,
    MADOPILOT_MATCH_HAS_MAX_RESULTS = 0x2,
    MADOPILOT_MATCH_HAS_SUPPRESSION = 0x4,
    MADOPILOT_IMAGE_SHARED = 0x1,
    MADOPILOT_TARGET_SUPPORTS_PLACEMENT = 0x1,
    MADOPILOT_TARGET_HAS_KIND = 0x2,
    MADOPILOT_TARGET_HAS_CAPTURE_PERMISSION = 0x4,
    MADOPILOT_ENGINE_DELIVERS_INPUT = 0x1,
    MADOPILOT_ENGINE_READS_PERMISSIONS = 0x2,
    MADOPILOT_PERMISSION_HAS_DIAGNOSTIC = 0x1,
    MADOPILOT_PERMISSION_HAS_PLATFORM_CODE = 0x2,
    MADOPILOT_TARGET_CAPABILITY_HAS_KIND = 0x1,
    MADOPILOT_TARGET_CAPABILITY_HAS_CAPTURE_PERMISSION = 0x2,
    MADOPILOT_TARGET_CAPABILITY_HAS_INPUT_PERMISSION = 0x4,
    MADOPILOT_INPUT_DESCRIPTOR_HAS_PERMISSION = 0x1,
    MADOPILOT_INPUT_REQUEST_HAS_CLEANUP_BUDGET = 0x1,
    MADOPILOT_INPUT_RECEIPT_HAS_TARGET = 0x1,
    MADOPILOT_INPUT_RECEIPT_HAS_DELIVERY = 0x2,
    MADOPILOT_INPUT_RECEIPT_HAS_LAST_COMPLETED = 0x4,
    MADOPILOT_INPUT_RECEIPT_HAS_FAILURE = 0x8,
    MADOPILOT_INPUT_RECEIPT_USED_FALLBACK = 0x10,
    MADOPILOT_ERROR_HAS_ASSET_DETAIL = 0x1,
    MADOPILOT_ERROR_HAS_BACKEND = 0x2,
}

#[test]
fn every_frozen_number_is_the_one_this_library_defines() {
    for (name, defined, frozen) in FROZEN {
        assert_eq!(
            defined, frozen,
            "{name} is {defined} in this build; the accepted ABI freeze fixed it at \
             {frozen}, and a number in that table is permanent for ABI major 1"
        );
    }
}

#[test]
fn the_header_declares_no_number_the_freeze_does_not_cover() {
    for (name, declared, kind) in header_symbols() {
        let Some(declared) = declared else {
            assert!(
                kind != Declaration::Enumerator,
                "the header declares the enumerator `{name}` and this parser could not read its \
                 value. An enumerator has a number whichever way it is written, so give it an \
                 explicit integer literal in the header and freeze that number here. Excusing it \
                 would take a value a caller can read back out of the freeze."
            );
            assert!(
                NOT_A_NUMBER.contains(&name.as_str()),
                "the header declares `{name}` without an integer literal, and nothing here says \
                 whether it is a frozen number. Freeze its value, or — only if it is a macro that \
                 expands to no number at all — record it in NOT_A_NUMBER."
            );
            continue;
        };

        let (_, _, frozen) = FROZEN
            .iter()
            .find(|(frozen, _, _)| *frozen == name)
            .unwrap_or_else(|| {
                panic!(
                    "the header declares `{name} = {declared}` and the freeze table does not \
                     carry it. A value a caller can read is a value ABI major 1 has to keep."
                )
            });
        assert_eq!(
            declared, *frozen,
            "the header declares `{name}` as {declared} and ADR 0007 froze it at {frozen}; a \
             caller compiled against the header and a library built from Rust would disagree"
        );
    }
}

/// The escape hatch for a symbol with no number never covers an enumerator.
///
/// An enumerator's value exists whether or not the header writes one out, so
/// moving its name here would make the suite green by dropping a frozen number
/// out of the freeze — the failure this file exists to prevent. The test above
/// already refuses the entry when it meets one; this one refuses it up front, so
/// the list can be read as the promise it makes.
#[test]
fn no_enumerator_is_excused_from_the_freeze() {
    for (name, _, kind) in header_symbols() {
        assert!(
            kind != Declaration::Enumerator || !NOT_A_NUMBER.contains(&name.as_str()),
            "`{name}` is an enumerator and NOT_A_NUMBER excuses it from the freeze. An enumerator \
             carries a number a caller can read, so it belongs in the freeze table instead."
        );
    }
}

#[test]
fn every_frozen_number_is_declared_by_the_header() {
    let declared = header_symbols();

    for (name, _, frozen) in FROZEN {
        let (_, value, _) = declared
            .iter()
            .find(|(declared, _, _)| declared == name)
            .unwrap_or_else(|| {
                panic!(
                    "`{name}` is frozen at {frozen} and the header declares no such name; within \
                     ABI major 1 a released name is never withdrawn"
                )
            });
        assert_eq!(
            *value,
            Some(*frozen),
            "the header declares `{name}` as {value:?}; the ABI freeze records fixed it at {frozen}"
        );
    }
}

/// How the header spells one declaration.
///
/// The distinction the tests need is whether a value exists at all. A macro may
/// expand to something that is not a number; an enumerator always has one, so a
/// value this parser could not read is a parse failure rather than a symbol
/// with nothing to freeze.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Declaration {
    /// A preprocessor `#define`.
    Define,
    /// A name inside an `enum { … }` block.
    Enumerator,
    /// A `MADOPILOT_`-prefixed name somewhere else — today, the export macro
    /// standing in front of a function declaration.
    Other,
}

/// Every `MADOPILOT_`-prefixed name the header declares, with its value and how
/// it is spelled.
///
/// `None` means the declaration carries no integer literal, which the tests
/// above require to be an accounted-for exception rather than something the
/// parser stepped over.
fn header_symbols() -> Vec<(String, Option<i128>, Declaration)> {
    let mut declared = Vec::new();
    let mut in_comment = false;
    let mut in_enum = false;

    for line in HEADER.lines() {
        // Comments come off before anything is parsed. The header's style puts
        // them on the line above, but one written after a value used to leave
        // the value unreadable and the symbol reported as undeclared.
        let line = uncommented(line, &mut in_comment);
        let line = line.trim();

        // `enum { … };`. Which block a name sits in is what tells an enumerator
        // from a macro, and the two are held to different rules.
        if line.starts_with("enum") && line.ends_with('{') {
            in_enum = true;
            continue;
        }
        if in_enum && line.starts_with('}') {
            in_enum = false;
            continue;
        }

        // `#  define NAME VALUE`, with the indentation the platform branches use.
        let define = line
            .strip_prefix('#')
            .map(str::trim_start)
            .and_then(|directive| directive.strip_prefix("define "));

        let (name, value, kind) = if let Some(rest) = define {
            let mut tokens = rest.split_whitespace();
            let Some(name) = tokens.next() else { continue };
            (name, tokens.next(), Declaration::Define)
        } else if line.starts_with("MADOPILOT_") {
            let kind = if in_enum {
                Declaration::Enumerator
            } else {
                Declaration::Other
            };

            // An enumerator, `NAME = VALUE,`. A declaration without one is not
            // a number and is reported as such rather than skipped.
            match line.split_once('=') {
                Some((name, value)) => (
                    name.trim(),
                    Some(value.trim().trim_end_matches(',').trim()),
                    kind,
                ),
                None => (
                    line.split_whitespace().next().unwrap_or(line),
                    None::<&str>,
                    kind,
                ),
            }
        } else {
            continue;
        };

        if !name.starts_with("MADOPILOT_") {
            continue;
        }
        declared.push((name.to_owned(), value.and_then(integer), kind));
    }

    assert!(
        !declared.is_empty(),
        "the header parsed to no declarations at all, so every comparison below would be vacuous"
    );

    declared
}

/// Returns `line` with its comments removed, carrying block state to the next
/// line.
///
/// Spans are removed rather than the line truncated at the first marker, so a
/// declaration that follows a comment on the same line is still parsed instead
/// of silently disappearing.
fn uncommented(line: &str, in_comment: &mut bool) -> String {
    let mut code = String::with_capacity(line.len());
    let mut rest = line;

    while !rest.is_empty() {
        if *in_comment {
            let Some(end) = rest.find("*/") else { break };
            *in_comment = false;
            rest = &rest[end + "*/".len()..];
            continue;
        }

        let block = rest.find("/*");
        let to_end = rest.find("//");
        match (block, to_end) {
            (Some(at), _) if to_end.is_none_or(|to_end| at < to_end) => {
                code.push_str(&rest[..at]);
                *in_comment = true;
                rest = &rest[at + "/*".len()..];
            }
            (_, Some(at)) => {
                code.push_str(&rest[..at]);
                break;
            }
            (_, None) => {
                code.push_str(rest);
                break;
            }
        }
    }

    code
}

/// Reads a C integer literal, decimal or hexadecimal, with any width suffix or
/// the fixed-width `UINT64_C(...)` spelling used for semantic 64-bit masks.
fn integer(token: &str) -> Option<i128> {
    let token = token
        .strip_prefix("UINT64_C(")
        .and_then(|value| value.strip_suffix(')'))
        .unwrap_or(token);
    let token = token.trim_end_matches(['u', 'U', 'l', 'L']);

    match token
        .strip_prefix("0x")
        .or_else(|| token.strip_prefix("0X"))
    {
        Some(hex) => i128::from_str_radix(hex, 16).ok(),
        None => token.parse().ok(),
    }
}
