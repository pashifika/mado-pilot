//! The layout invariants that hold without a C compiler.
//!
//! The numeric agreement between the header and the Rust definitions is proved
//! by `examples/c-abi-check.rs`, which needs a C toolchain. These are the
//! properties that do not: the frozen numbers themselves, `struct_size` first,
//! mandatory prefixes that are real prefixes, thin handle pointers, and a
//! function-table order that a refactor cannot quietly rearrange.
//!
//! **These values are frozen** for ABI major 1 by
//! `docs/adr/0007-phase-1-c-abi-freeze.md`. What this file protects is that a
//! change to any of them fails a test rather than a caller.

use madopilot::layout::{HANDLE_POINTERS, LAYOUT, TypeLayout, report};
use madopilot::*;

/// The layout ADR 0007 froze, on each release target.
///
/// These are the tracked evidence files, compiled in rather than read at run
/// time so the comparison needs no working directory and no C toolchain. They
/// are the *only* place the frozen sizes, alignments, and field offsets are
/// written down as numbers; every other test in this file checks a structural
/// property that a coordinated edit to the header and to `src/types.rs` would
/// keep true.
const FROZEN_LAYOUT: [(&str, &str); 2] = [
    (
        "aarch64-apple-darwin",
        include_str!("../../../../docs/evidence/c-abi/layout-aarch64-apple-darwin.txt"),
    ),
    (
        "x86_64-pc-windows-msvc",
        include_str!("../../../../docs/evidence/c-abi/layout-x86_64-pc-windows-msvc.txt"),
    ),
];

/// Every versioned structure, and the mandatory prefix it documents.
///
/// A prefix that is not a field boundary would mean the library reads or writes
/// half a field, so each one is checked against the offsets as well as against
/// the size.
///
/// `madopilot_match_options_t` appears twice because it is the one structure
/// used in both directions and the only one with two prefixes: 8 bytes as the
/// request a caller supplies, 24 as the report `result_options` writes back.
/// Both have to be real boundaries, and ADR 0007 is the only other place the
/// asymmetry is recorded.
const MANDATORY: &[(&str, usize)] = &[
    ("madopilot_build_info_t", 20),
    ("madopilot_operation_t", 8),
    ("madopilot_frame_stamp_t", 40),
    ("madopilot_frame_info_t", 24),
    ("madopilot_image_t", 48),
    ("madopilot_target_t", 24),
    ("madopilot_session_info_t", 32),
    ("madopilot_open_request_t", 8),
    ("madopilot_map_request_t", 12),
    ("madopilot_match_options_t", 8),
    ("madopilot_match_options_t", 24),
    ("madopilot_find_request_t", 24),
    ("madopilot_match_t", 56),
    ("madopilot_result_info_t", 72),
    ("madopilot_package_info_t", 64),
    ("madopilot_template_info_t", 64),
    ("madopilot_error_detail_t", 16),
    ("madopilot_replay_frame_t", 40),
    ("madopilot_source_t", 48),
    ("madopilot_package_source_t", 24),
];

/// The Phase 1 function-table order.
///
/// Written out rather than derived, because the point is to notice a change.
/// Appending to the end is how the table grows; anything else here is a
/// compatibility break and has to be an explicit edit to this list.
const TABLE_ORDER: &[&str] = &[
    "struct_size",
    "abi_major",
    "abi_minor",
    "reserved",
    "describe_build",
    "clock_now",
    "status_text",
    "cancellation_create",
    "cancellation_retain",
    "cancellation_release",
    "cancellation_cancel",
    "cancellation_is_cancelled",
    "error_retain",
    "error_release",
    "error_describe",
    "engine_create",
    "engine_retain",
    "engine_release",
    "package_load",
    "package_retain",
    "package_release",
    "package_describe",
    "package_template_id",
    "template_prepare_from_package",
    "template_retain",
    "template_release",
    "template_describe",
    "engine_discover",
    "target_list_retain",
    "target_list_release",
    "target_list_count",
    "target_list_get",
    "session_open",
    "session_retain",
    "session_release",
    "session_describe",
    "session_close",
    "session_is_closed",
    "session_acquire_frame",
    "frame_retain",
    "frame_release",
    "frame_stamp",
    "frame_describe",
    "frame_map",
    "mapping_retain",
    "mapping_release",
    "mapping_describe",
    "mapping_stamp",
    "session_find",
    "result_retain",
    "result_release",
    "result_describe",
    "result_stamp",
    "result_options",
    "result_match",
];

fn find(name: &str) -> &'static TypeLayout {
    LAYOUT
        .iter()
        .find(|layout| layout.name == name)
        .unwrap_or_else(|| panic!("`{name}` is measured by the layout report"))
}

/// Compares one target's tracked evidence against what this build measured.
///
/// Reports the first line that differs rather than the whole report, because a
/// single field moving shifts nothing else and a whole-report dump would bury
/// it.
fn assert_frozen(target: &str, frozen: &str, measured: &str) {
    if measured == frozen {
        return;
    }

    let difference = frozen
        .lines()
        .zip(measured.lines())
        .enumerate()
        .find(|(_, (frozen, measured))| frozen != measured)
        .map_or_else(
            || {
                format!(
                    "the reports agree line for line and then one of them ends: the frozen \
                     record has {} lines, this build measured {}",
                    frozen.lines().count(),
                    measured.lines().count(),
                )
            },
            |(index, (frozen, measured))| {
                format!(
                    "line {}: the frozen record says `{frozen}`, this build measured `{measured}`",
                    index + 1
                )
            },
        );

    panic!(
        "the measured layout is not the one frozen for {target} in \
         `docs/evidence/c-abi/layout-{target}.txt`.\n  {difference}\nWithin ABI major 1 that \
         report only gains structures and fields appended after every existing one; anything \
         else is a compatibility break. A deliberate additive minor regenerates the evidence on \
         both release targets with `cargo run --package mado-pilot-capi --example c-abi-check` \
         and records the new numbers in `docs/adr/0007-phase-1-c-abi-freeze.md`."
    );
}

#[test]
fn the_measured_layout_is_the_one_that_was_frozen() {
    let measured = report();

    for (target, frozen) in FROZEN_LAYOUT {
        assert_frozen(target, frozen, &measured);
    }
}

#[test]
fn both_release_targets_froze_the_same_layout() {
    let [(first, one), (second, two)] = FROZEN_LAYOUT;

    assert_eq!(
        one, two,
        "`docs/evidence/c-abi/` records the {first} and {second} reports as byte-identical, and \
         the test above pins this build against both. Regenerating one without the other leaves \
         one release target unpinned."
    );
}

#[test]
fn every_versioned_structure_begins_with_struct_size() {
    for (name, _) in MANDATORY {
        let layout = find(name);
        let first = layout.fields.first().expect("a structure has fields");
        assert_eq!(first.name, "struct_size", "{name} begins with struct_size");
        assert_eq!(first.offset, 0, "{name} puts it at offset zero");
    }
}

#[test]
fn the_second_field_leaves_no_implicit_padding_after_struct_size() {
    for (name, _) in MANDATORY {
        let layout = find(name);
        let second = layout
            .fields
            .get(1)
            .expect("a structure has a second field");
        assert_eq!(
            second.offset, 4,
            "{name} follows struct_size with a 32-bit field"
        );
    }
}

#[test]
fn every_mandatory_prefix_is_a_real_field_boundary() {
    for (name, mandatory) in MANDATORY {
        let layout = find(name);
        assert!(
            *mandatory <= layout.size,
            "{name} declares a {mandatory} byte prefix but is only {} bytes",
            layout.size
        );

        let boundary = layout.size == *mandatory
            || layout.fields.iter().any(|field| field.offset == *mandatory);
        assert!(
            boundary,
            "{name}'s {mandatory} byte prefix ends inside a field"
        );
    }
}

#[test]
fn every_structure_has_ascending_field_offsets() {
    for layout in LAYOUT {
        let mut previous = None;
        for field in layout.fields {
            if let Some(previous) = previous {
                assert!(
                    field.offset > previous,
                    "{}.{} is not after the field before it",
                    layout.name,
                    field.name
                );
            }
            previous = Some(field.offset);
        }
    }
}

#[test]
fn the_function_table_keeps_its_phase_1_order() {
    let table = find("madopilot_api_t");
    let measured: Vec<&str> = table.fields.iter().map(|field| field.name).collect();

    assert_eq!(
        measured, TABLE_ORDER,
        "the Phase 1 table order changed; within an ABI major, members are only appended"
    );
    assert_eq!(
        table.size, MADOPILOT_API_SIZE_PHASE1 as usize,
        "the advertised table size is the measured one"
    );
    assert!(
        MADOPILOT_API_SIZE_INFORMATION as usize <= table.size,
        "the mandatory prefix fits inside the table"
    );
}

#[test]
fn the_information_prefix_ends_at_status_text() {
    let table = find("madopilot_api_t");
    let status_text = table
        .fields
        .iter()
        .find(|field| field.name == "status_text")
        .expect("the information group ends with status_text");

    assert_eq!(
        MADOPILOT_API_SIZE_INFORMATION as usize,
        status_text.offset + size_of::<usize>(),
        "the constant and the member it names have to move together"
    );
}

#[test]
fn every_opaque_handle_is_a_thin_pointer() {
    for layout in HANDLE_POINTERS {
        assert_eq!(
            layout.size,
            size_of::<*const ()>(),
            "{} is not a thin pointer",
            layout.name
        );
        assert!(layout.fields.is_empty(), "{} is opaque", layout.name);
    }
}

#[test]
fn the_two_view_primitives_are_a_pointer_and_a_length() {
    for name in ["madopilot_str_t", "madopilot_bytes_t"] {
        let layout = find(name);
        assert_eq!(layout.size, 2 * size_of::<*const ()>());
        assert_eq!(layout.fields.len(), 2, "{name} is a pointer and a length");
        assert_eq!(layout.fields[0].name, "data");
        assert_eq!(layout.fields[1].name, "len");
    }
}

#[test]
fn the_report_names_every_structure_the_header_declares() {
    let report = report();

    for (name, _) in MANDATORY {
        assert!(
            report.contains(&format!("type {name} size=")),
            "{name} is missing from the layout report the C probe is diffed against"
        );
    }
}
