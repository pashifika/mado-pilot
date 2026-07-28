//! What the Phase 1 C++ header declares, and what it must not.
//!
//! The wrapper's ownership shape is proved by the `static_assert`s in
//! `tests/cpp/madopilot-cpp-ownership.cpp`, which need a C++ compiler. This
//! file needs none: it reads the tracked header as text and checks its
//! inventory, so a plain `cargo test` notices a type that appeared or
//! disappeared.
//!
//! The point is the exclusion. The Phase 1 C table ends at match-result access,
//! and the wrapper may wrap only what is in it. A `Watcher`, a `Query`, or an
//! `Ocr` type here would be a promise the library underneath cannot keep, and
//! it would compile perfectly.
//!
//! The C contract beneath these names is frozen by
//! `docs/adr/0007-phase-1-c-abi-freeze.md`. The C++ surface is not an ABI and
//! is governed by the Rust-side policy in
//! `docs/adr/0006-public-rust-names-and-compatibility-policy.md`; what this
//! file protects is that a change to it is deliberate.

use std::path::PathBuf;

/// Every type the Phase 1 wrapper declares at namespace scope.
///
/// Written out rather than derived, because the point is to notice a change.
const DECLARED: &[&str] = &[
    // Borrowed views.
    "BorrowedStr",
    "BorrowedBytes",
    // Errors and results.
    "AssetDetail",
    "Error",
    "Result",
    // Typed requests.
    "Operation",
    "ReplayFrame",
    "Source",
    "PackageSource",
    "OpenRequest",
    "MapRequest",
    "MatchOptions",
    "FindRequest",
    // Projections of the C output structures that carry borrowed views.
    "BuildInfo",
    "TargetDescriptor",
    "Image",
    "PackageInfo",
    "TemplateInfo",
    "ResultInfo",
    "Match",
    // Owners, one per reference-counted C handle.
    "Cancellation",
    "TargetList",
    "Package",
    "Template",
    "Mapping",
    "Frame",
    "MatchResult",
    "Session",
    "Engine",
    // The negotiated table.
    "Api",
];

/// The concepts the Phase 1 prefix does not contain.
///
/// Each is a word that would appear in a declared type name if a later phase's
/// surface leaked into this one.
const EXCLUDED: &[&str] = &[
    "Input",
    "Key",
    "Pointer",
    "Ocr",
    "Recognition",
    "Model",
    "Watcher",
    "Watch",
    "Query",
    "Callback",
    "Subscription",
    "NativeFrame",
    "Extension",
];

fn header() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("include/madopilot/madopilot.hpp");

    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()))
}

/// Returns every `class` or `struct` name declared at namespace scope.
///
/// Nesting is by indentation: a declaration inside a class body is indented,
/// and one at namespace scope is not. That is enough because the header is
/// formatted, and a declaration that broke the rule would show up as a missing
/// name rather than as a silent pass.
fn declared_types(header: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut in_detail = false;

    for line in header.lines() {
        if line.starts_with("namespace detail {") {
            in_detail = true;
        } else if in_detail && line.starts_with("} // namespace detail") {
            in_detail = false;
        }
        if in_detail {
            continue;
        }

        let Some(rest) = line
            .strip_prefix("class ")
            .or_else(|| line.strip_prefix("struct "))
        else {
            continue;
        };
        let name: String = rest
            .chars()
            .take_while(|character| character.is_alphanumeric() || *character == '_')
            .collect();
        if !name.is_empty() && !found.contains(&name) {
            found.push(name);
        }
    }

    found
}

#[test]
fn the_header_declares_exactly_the_phase_one_surface() {
    let header = header();
    let mut found = declared_types(&header);
    // `Result` is declared once as a template and once as its void
    // specialization; the inventory names it once.
    found.sort();
    found.dedup();

    let mut expected: Vec<String> = DECLARED.iter().map(|name| (*name).to_owned()).collect();
    expected.sort();

    assert_eq!(
        found, expected,
        "the C++ header's declared types changed. Update `DECLARED` in the same \
         change, after checking that every new type wraps something the Phase 1 C \
         table actually has."
    );
}

#[test]
fn the_header_declares_no_later_phase_surface() {
    let header = header();

    for name in declared_types(&header) {
        for excluded in EXCLUDED {
            assert!(
                !name.contains(excluded),
                "`{name}` names `{excluded}`, which the Phase 1 C prefix does not \
                 contain. Input, OCR, watchers, queries, callbacks, and native-frame \
                 extensions are appended by a later phase, in C first."
            );
        }
    }
}

#[test]
fn every_owner_is_reachable_and_none_is_orphaned() {
    let header = header();

    // One owner per reference-counted C handle, and no more: an owner with no
    // handle behind it would be a type with nothing to own.
    const HANDLES: &[(&str, &str)] = &[
        ("Cancellation", "madopilot_cancellation_t"),
        ("Engine", "madopilot_engine_t"),
        ("TargetList", "madopilot_target_list_t"),
        ("Package", "madopilot_package_t"),
        ("Template", "madopilot_template_t"),
        ("Session", "madopilot_session_t"),
        ("Frame", "madopilot_frame_t"),
        ("Mapping", "madopilot_mapping_t"),
        ("MatchResult", "madopilot_result_t"),
    ];

    for (owner, handle) in HANDLES {
        assert!(
            header.contains(&format!(
                "class {owner} : public detail::Owner<{owner}, ::{handle}>"
            )),
            "`{owner}` should own `{handle}` through the move-only owner base"
        );
    }

    // `madopilot_error_t` is the tenth handle and deliberately has no owner: the
    // wrapper describes an error, copies it, and releases the handle before the
    // caller ever sees a `Result`.
    assert!(
        !header.contains("public detail::Owner<Error"),
        "the C++ error is a value, not an owner of a retained C handle"
    );
}

#[test]
fn the_header_restates_no_status_value() {
    let header = header();

    // Every enumerated type is an alias of the C type, so a caller writes the
    // `MADOPILOT_*` constant and gets whatever the header it compiled against
    // says that is. A C++ enumeration with copied values would be a second
    // vocabulary to freeze alongside the first.
    for alias in [
        "using Status = ::madopilot_status_t;",
        "using ErrorCategory = ::madopilot_error_category_t;",
        "using Space = ::madopilot_space_t;",
        "using PixelFormat = ::madopilot_pixel_format_t;",
        "using ClipPolicy = ::madopilot_clip_policy_t;",
        "using AssetFault = ::madopilot_asset_fault_t;",
        "using AssetStage = ::madopilot_asset_stage_t;",
    ] {
        assert!(
            header.contains(alias),
            "the header should contain `{alias}`"
        );
    }

    // Matched against the start of a statement rather than anywhere in the
    // text, so that the comment explaining this rule does not trip it.
    for line in header.lines() {
        let statement = line.trim_start();
        assert!(
            !statement.starts_with("enum "),
            "the wrapper declares no enumeration of its own, but this line does: \
             `{statement}`. A hand-written mirror of a frozen C constant set \
             fails silently when that set grows — it compiles, one value short — \
             so the vocabulary must have exactly one definition."
        );
    }
}
