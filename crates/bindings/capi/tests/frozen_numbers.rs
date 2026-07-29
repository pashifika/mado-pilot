//! The frozen numeric vocabulary: statuses, categories, faults, stages, and
//! flag bits.
//!
//! Layout and numbers are two independent halves of the freeze, and the
//! cross-language mechanism covers only the first: `tests/c/madopilot-abi-layout.c`
//! reports sizes, alignments, and offsets, so a header-versus-Rust renumbering
//! of a status passes it. This file is the second half. It has no C toolchain
//! to compare against, so it compares three things instead — what
//! `docs/adr/0007-phase-1-c-abi-freeze.md` froze, what the Rust definitions say,
//! and what the hand-written header declares — and requires all three to agree
//! in both directions.
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
const NOT_A_NUMBER: &[&str] = &[
    // The include guard.
    "MADOPILOT_MADOPILOT_H",
    // Per-platform linkage, one branch per toolchain.
    "MADOPILOT_EXPORT",
    // An `offsetof` expression rather than a literal. Its value is pinned by
    // `tests/layout.rs::the_information_prefix_ends_at_status_text` against the
    // member it names, and by the frozen layout report.
    "MADOPILOT_API_SIZE_INFORMATION",
];

macro_rules! frozen {
    ($($name:ident = $value:literal,)*) => {
        /// Every frozen number: its name, what this library defines it as, and
        /// what ADR 0007 froze it at.
        ///
        /// The middle column is the Rust constant itself, so this table cannot
        /// be satisfied by editing it alone — the value has to be a literal
        /// copied from the ADR, and the constant has to still equal it.
        const FROZEN: &[(&str, i64, i64)] = &[
            $((stringify!($name), $name as i64, $value),)*
        ];
    };
}

frozen! {
    // The ABI this header and this library are.
    MADOPILOT_ABI_MAJOR = 1,
    MADOPILOT_ABI_MINOR = 0,

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

    // The subsystem a failure came from.
    MADOPILOT_ERROR_CATEGORY_UNSPECIFIED = 0,
    MADOPILOT_ERROR_CATEGORY_ABI = 1,
    MADOPILOT_ERROR_CATEGORY_OPERATION = 2,
    MADOPILOT_ERROR_CATEGORY_ENGINE = 3,
    MADOPILOT_ERROR_CATEGORY_CAPTURE = 4,
    MADOPILOT_ERROR_CATEGORY_ASSET = 5,
    MADOPILOT_ERROR_CATEGORY_VISION = 6,
    MADOPILOT_ERROR_CATEGORY_GEOMETRY = 7,

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
    MADOPILOT_ERROR_HAS_ASSET_DETAIL = 0x1,
    MADOPILOT_ERROR_HAS_BACKEND = 0x2,
}

#[test]
fn every_frozen_number_is_the_one_this_library_defines() {
    for (name, defined, frozen) in FROZEN {
        assert_eq!(
            defined, frozen,
            "{name} is {defined} in this build; ADR 0007 froze it at {frozen}, and a number in \
             that table is permanent for ABI major 1"
        );
    }
}

#[test]
fn the_header_declares_no_number_the_freeze_does_not_cover() {
    for (name, declared) in header_symbols() {
        let Some(declared) = declared else {
            assert!(
                NOT_A_NUMBER.contains(&name),
                "the header declares `{name}` without an integer literal, and nothing here says \
                 whether it is a frozen number. Add it to the freeze table or to NOT_A_NUMBER."
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

#[test]
fn every_frozen_number_is_declared_by_the_header() {
    let declared = header_symbols();

    for (name, _, frozen) in FROZEN {
        let (_, value) = declared
            .iter()
            .find(|(declared, _)| declared == name)
            .unwrap_or_else(|| {
                panic!(
                    "`{name}` is frozen at {frozen} and the header declares no such name; within \
                     ABI major 1 a released name is never withdrawn"
                )
            });
        assert_eq!(
            *value,
            Some(*frozen),
            "the header declares `{name}` as {value:?}; ADR 0007 froze it at {frozen}"
        );
    }
}

/// Every `MADOPILOT_`-prefixed name the header declares, with its value.
///
/// `None` means the declaration carries no integer literal, which the tests
/// above require to be an accounted-for exception rather than something the
/// parser stepped over.
fn header_symbols() -> Vec<(&'static str, Option<i64>)> {
    let mut declared = Vec::new();

    for line in HEADER.lines() {
        let line = line.trim();

        // `#  define NAME VALUE`, with the indentation the platform branches use.
        let define = line
            .strip_prefix('#')
            .map(str::trim_start)
            .and_then(|directive| directive.strip_prefix("define "));

        let (name, value) = if let Some(rest) = define {
            let mut tokens = rest.split_whitespace();
            let Some(name) = tokens.next() else { continue };
            (name, tokens.next())
        } else if line.starts_with("MADOPILOT_") {
            // An enumerator, `NAME = VALUE,`. A declaration without one is not
            // a number and is reported as such rather than skipped.
            match line.split_once('=') {
                Some((name, value)) => (name.trim(), Some(value.trim().trim_end_matches(','))),
                None => (line.split_whitespace().next().unwrap_or(line), None::<&str>),
            }
        } else {
            continue;
        };

        if !name.starts_with("MADOPILOT_") {
            continue;
        }
        declared.push((name, value.and_then(integer)));
    }

    assert!(
        !declared.is_empty(),
        "the header parsed to no declarations at all, so every comparison below would be vacuous"
    );

    declared
}

/// Reads a C integer literal, decimal or hexadecimal, with any width suffix.
fn integer(token: &str) -> Option<i64> {
    let token = token.trim_end_matches(['u', 'U', 'l', 'L']);

    match token
        .strip_prefix("0x")
        .or_else(|| token.strip_prefix("0X"))
    {
        Some(hex) => i64::from_str_radix(hex, 16).ok(),
        None => token.parse().ok(),
    }
}
