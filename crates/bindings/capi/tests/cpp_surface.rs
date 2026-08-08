//! What the ABI 1.1 C++ header declares, and what it must not.
//!
//! The wrapper's ownership shape is proved by the `static_assert`s in
//! `tests/cpp/madopilot-cpp-ownership.cpp`, which need a C++ compiler. This
//! file needs none: it reads the tracked header as text and checks its
//! inventory, so a plain `cargo test` notices a type that appeared or
//! disappeared.
//!
//! ABI 1.1 ends at native permission, capability, and session input. A
//! `Watcher`, `Ocr`, or packaging type here would promise a deferred C entry
//! that does not exist, and it would compile perfectly.
//!
//! The complete 1.0 prefix is frozen by
//! `docs/adr/0007-phase-1-c-abi-freeze.md`; the additive 1.1 suffix is frozen by
//! `docs/adr/0017-c-abi-1-1-native-input-prefix.md` and the old-header fixture
//! under `tests/abi-compat/v1.1/`. The C++ surface is not an ABI and is governed by
//! `docs/adr/0006-public-rust-names-and-compatibility-policy.md`; what this
//! file protects is that a change to it is deliberate.

use std::path::PathBuf;

/// Every type the ABI 1.1 wrapper declares at namespace scope.
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
    "InputOpenRequest",
    "InputEvent",
    "InputRequest",
    "OpenRequest",
    "MapRequest",
    "MatchOptions",
    "FindRequest",
    // Projections of the C output structures that carry borrowed views.
    "EngineCapabilities",
    "PermissionDiagnostic",
    "Permission",
    "TargetCapability",
    "InputDescriptor",
    "InputFailure",
    "InputReceipt",
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

/// The concepts the ABI 1.1 suffix still defers.
///
/// Each is a word that would appear in a declared type name if a deferred
/// surface leaked into this wrapper.
const EXCLUDED: &[&str] = &[
    "Ocr",
    "Recognition",
    "Model",
    "Watcher",
    "Watch",
    "Query",
    "Callback",
    "Subscription",
    "Acceleration",
    "Packaging",
    "NativeFrame",
    "Extension",
];

fn header() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("include/madopilot/madopilot.hpp");

    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()))
}

/// Returns every `class` or `struct` name declared at namespace scope,
/// attributed or not.
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
        // An attribute may sit between the keyword and the name, as in
        // `class [[nodiscard]] Result`. Skipping it keeps the type in the
        // inventory; stopping at it would drop the type silently, which is the
        // one outcome an inventory must not produce.
        let rest = match rest.trim_start().strip_prefix("[[") {
            Some(after) => after
                .split_once("]]")
                .map_or(after, |(_, name)| name)
                .trim_start(),
            None => rest,
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
fn the_header_declares_exactly_the_abi_1_1_surface() {
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
         change, after checking that every new type wraps something the ABI 1.1 C \
         table actually has."
    );
}

#[test]
fn the_header_declares_no_deferred_surface() {
    let header = header();

    for name in declared_types(&header) {
        for excluded in EXCLUDED {
            assert!(
                !name.contains(excluded),
                "`{name}` names `{excluded}`, which ABI 1.1 defers. OCR, \
                 watchers, queries, callbacks, acceleration, packaging, and \
                 native-frame extensions must appear in C first."
            );
        }
    }
}

/// Returns the header with its comments removed.
///
/// The scan below looks for words rather than declarations, so a word that only
/// appears in prose would be a false failure. Stripping the comments first is
/// what makes the scan exact instead of approximate.
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

/// No later-phase concept appears anywhere in the code the header declares.
///
/// [`the_header_declares_no_deferred_surface`] checks the same words against
/// declared TYPE names only, so a later-phase *method* on an existing type —
/// `Session::register_frame_callback`, say — passed it, though the spec forbids
/// types or methods alike. None of these words occurs anywhere in the header
/// today, comments included, which is what lets this be a scan of the whole
/// declared surface rather than a parse of part of it: it catches a method, a
/// member, an alias, or a type equally.
///
/// The comparison folds case, and that is load-bearing rather than defensive.
/// [`EXCLUDED`] spells the words for type names, so they are `PascalCase`, while
/// a method is `snake_case`: a case-sensitive scan let `register_frame_callback`
/// through, which is the very example this exists to catch. Folding case
/// introduces no false positive — none of these words occurs in the header's
/// code under any spelling.
#[test]
fn no_deferred_concept_appears_anywhere_in_the_header() {
    let header = without_comments(&header()).to_lowercase();

    for excluded in EXCLUDED {
        assert!(
            !header.contains(&excluded.to_lowercase()),
            "the header names `{excluded}`, which ABI 1.1 defers. OCR, watchers, \
             queries, callbacks, acceleration, packaging, and native-frame \
             extensions must appear in C first — as a type or method, either way."
        );
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
        "using PermissionKind = ::madopilot_permission_kind_t;",
        "using PermissionState = ::madopilot_permission_state_t;",
        "using DiagnosticCategory = ::madopilot_diagnostic_category_t;",
        "using TargetKind = ::madopilot_target_kind_t;",
        "using CapabilitySupport = ::madopilot_capability_support_t;",
        "using InputOperationKind = ::madopilot_input_operation_kind_t;",
        "using InputDelivery = ::madopilot_input_delivery_t;",
        "using InputRequirement = ::madopilot_input_requirement_t;",
        "using FocusPolicy = ::madopilot_focus_policy_t;",
        "using GeometryPolicy = ::madopilot_geometry_policy_t;",
        "using PointerButton = ::madopilot_pointer_button_t;",
        "using Key = ::madopilot_key_t;",
        "using Modifier = ::madopilot_modifier_t;",
        "using InputEventKind = ::madopilot_input_event_kind_t;",
        "using SequenceOutcome = ::madopilot_sequence_outcome_t;",
        "using CleanupState = ::madopilot_cleanup_state_t;",
        "using InputFault = ::madopilot_input_fault_t;",
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
