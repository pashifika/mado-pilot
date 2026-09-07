//! Public C++ non-goals that compilation alone cannot reject.
//!
//! Ownership, value projections, and negotiated-table behavior are exercised by
//! `tests/cpp/madopilot-cpp-ownership.cpp`, not a source inventory.

use std::path::PathBuf;

const EXCLUDED: &[&str] = &[
    "OcrWatch",
    "Ocr_Watch",
    "Watch_Ocr",
    "OcrQuery",
    "Ocr_Query",
    "Callback",
    "Subscription",
    "Packaging",
    "NativeFrame",
    "Extension",
];

fn header() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("include/madopilot/madopilot.hpp");

    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()))
}

/// Ignore documentation when checking forbidden public concepts.
fn without_comments(header: &str) -> String {
    let mut out = String::with_capacity(header.len());
    let bytes = header.as_bytes();
    let mut index = 0;
    let mut in_line = false;
    let mut in_block = false;

    while index < bytes.len() {
        let rest = &bytes[index..];
        if in_line {
            if rest[0] == b'\n' {
                in_line = false;
                out.push('\n');
            }
            index += 1;
        } else if in_block {
            if rest.starts_with(b"*/") {
                in_block = false;
                index += 2;
            } else {
                if rest[0] == b'\n' {
                    out.push('\n');
                }
                index += 1;
            }
        } else if rest.starts_with(b"//") {
            in_line = true;
            index += 2;
        } else if rest.starts_with(b"/*") {
            in_block = true;
            index += 2;
        } else {
            out.push(char::from(rest[0]));
            index += 1;
        }
    }

    out
}

#[test]
fn no_deferred_concept_appears_anywhere_in_the_header() {
    let header = without_comments(&header()).to_lowercase();

    for excluded in EXCLUDED {
        assert!(
            !header.contains(&excluded.to_lowercase()),
            "the C++ header exposes deferred `{excluded}`; pull-based template \
             queries do not authorize callbacks, OCR watchers, or packaging"
        );
    }
}

/// The superseded development-only ABI 1.1 route and completion claims must not
/// return as C++ conveniences around the corrected ABI 1.2 contract.
#[test]
fn unreleased_abi_1_1_vocabulary_is_absent() {
    let header = without_comments(&header()).to_lowercase();

    for removed in [
        "backgroundtarget",
        "background_target",
        "last_completed",
        "delivered",
    ] {
        assert!(
            !header.contains(removed),
            "the C++ header resurrected removed ABI 1.1 vocabulary: `{removed}`"
        );
    }
}
