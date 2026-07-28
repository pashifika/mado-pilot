//! The ABI's own rules: negotiation, structure sizes, pointer validation, and
//! the state every output is left in.
//!
//! These tests speak to the table exactly as a C caller does. What they check
//! is not that the library computes the right answer — that is `lifecycle.rs`
//! and the C example — but that it refuses a malformed request without reading
//! past what the caller declared, and that a refusal leaves nothing
//! half-written behind.

// Every call below goes through an `unsafe extern "C"` table entry, and the
// safety argument is the same one each time: every pointer is a live local of
// the test, and every handle is retained by the value that produced it for
// longer than the call lasts. Repeating that on a hundred assertions would bury
// it rather than document it, so it is stated once here and inline only where a
// call has something extra to say.
#![allow(clippy::undocumented_unsafe_blocks)]

mod support;

use std::ptr;

use madopilot::layout::struct_size;
use madopilot::*;
use support::{
    Scene, bytes_view, expired_operation, negotiate, operation, package_root, str_view, table,
};

// --- Negotiation -----------------------------------------------------------

#[test]
fn a_caller_negotiates_the_complete_phase_1_prefix() {
    let api = table();

    assert_eq!(api.abi_major, MADOPILOT_ABI_MAJOR);
    assert_eq!(api.abi_minor, MADOPILOT_ABI_MINOR);
    assert_eq!(
        api.struct_size, MADOPILOT_API_SIZE_PHASE1,
        "the table reports its own size, so a newer caller can clamp to it"
    );
    assert_eq!(api.struct_size as usize, size_of::<madopilot_api_t>());
}

#[test]
fn a_caller_that_knows_only_the_information_prefix_still_negotiates() {
    let api = negotiate(
        MADOPILOT_ABI_MAJOR,
        MADOPILOT_ABI_MINOR,
        MADOPILOT_API_SIZE_INFORMATION as usize,
    )
    .expect("an older prefix is a supported caller, not a broken one");

    // The members that prefix covers keep their behavior, and the caller can
    // use them without reading anything later.
    let mut info = build_info();
    // SAFETY: `info` is a live local with its `struct_size` set.
    let status = unsafe { (api.describe_build)(&raw mut info) };
    assert_eq!(status, MADOPILOT_STATUS_OK);
    assert_eq!(info.abi_major, MADOPILOT_ABI_MAJOR);

    let mut nanos = 0_u64;
    // SAFETY: as above.
    assert_eq!(
        unsafe { (api.clock_now)(&raw mut nanos) },
        MADOPILOT_STATUS_OK
    );
}

#[test]
fn an_unsupported_abi_major_is_refused() {
    let status = negotiate(
        MADOPILOT_ABI_MAJOR + 1,
        MADOPILOT_ABI_MINOR,
        size_of::<madopilot_api_t>(),
    )
    .expect_err("a different major is a different library");
    assert_eq!(status, MADOPILOT_STATUS_UNSUPPORTED);
}

#[test]
fn a_minimum_minor_newer_than_the_library_is_refused() {
    let status = negotiate(
        MADOPILOT_ABI_MAJOR,
        MADOPILOT_ABI_MINOR + 1,
        size_of::<madopilot_api_t>(),
    )
    .expect_err("the library cannot provide a minor it does not have");
    assert_eq!(status, MADOPILOT_STATUS_UNSUPPORTED);
}

#[test]
fn a_table_size_below_the_mandatory_prefix_is_refused() {
    let status = negotiate(
        MADOPILOT_ABI_MAJOR,
        MADOPILOT_ABI_MINOR,
        MADOPILOT_API_SIZE_INFORMATION as usize - 1,
    )
    .expect_err("a caller that cannot build a deadline cannot use the table");
    assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT);
}

#[test]
fn a_caller_larger_than_the_library_gets_the_library_it_has() {
    let api = negotiate(
        MADOPILOT_ABI_MAJOR,
        MADOPILOT_ABI_MINOR,
        size_of::<madopilot_api_t>() + 512,
    )
    .expect("a newer header against an older library is not an error");
    assert_eq!(
        api.struct_size, MADOPILOT_API_SIZE_PHASE1,
        "and it learns how much of what it knows is really there"
    );
}

#[test]
fn a_null_table_output_is_refused() {
    // SAFETY: passing null is exactly what this checks, and the entry rejects
    // it before writing anything.
    let status = unsafe {
        madopilot_get_api(
            MADOPILOT_ABI_MAJOR,
            MADOPILOT_ABI_MINOR,
            size_of::<madopilot_api_t>(),
            ptr::null_mut(),
        )
    };
    assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT);
}

// --- Size-versioned structures ---------------------------------------------

#[test]
fn an_output_smaller_than_its_mandatory_prefix_is_refused() {
    let api = table();
    let mut info = build_info();
    info.struct_size = 8;

    // SAFETY: `info` is a live local; the entry reads only its `struct_size`
    // before refusing.
    let status = unsafe { (api.describe_build)(&raw mut info) };
    assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT);
    assert_eq!(info.abi_major, 0, "nothing was written");
}

#[test]
fn an_output_using_an_older_valid_prefix_gets_that_prefix() {
    let api = table();
    let mut info = build_info();
    // Through `table_size`, which is the documented mandatory prefix. The two
    // string views after it are left entirely alone.
    info.struct_size = 20;

    // SAFETY: `info` is a live local larger than the size it declares, so a
    // library that honored the declaration writes strictly inside it.
    let status = unsafe { (api.describe_build)(&raw mut info) };
    assert_eq!(status, MADOPILOT_STATUS_OK);
    assert_eq!(info.abi_major, MADOPILOT_ABI_MAJOR);
    assert_eq!(info.table_size, MADOPILOT_API_SIZE_PHASE1);
    assert_eq!(info.struct_size, 20, "the library reports what it filled");
    assert!(
        info.library_version.data.is_null() && info.library_version.len == 0,
        "a field the caller did not declare is not written"
    );
}

#[test]
fn an_output_larger_than_the_library_knows_is_filled_only_as_far_as_it_goes() {
    let api = table();
    let mut info = build_info();
    info.struct_size = struct_size::<madopilot_build_info_t>() + 64;

    // SAFETY: `info` is a live local, and the library clamps to its own
    // `size_of` rather than trusting the larger declaration.
    let status = unsafe { (api.describe_build)(&raw mut info) };
    assert_eq!(status, MADOPILOT_STATUS_OK);
    assert_eq!(
        info.struct_size,
        struct_size::<madopilot_build_info_t>(),
        "the reported size is what was populated, not what was claimed"
    );
}

#[test]
fn an_input_using_an_older_valid_prefix_defaults_the_rest() {
    let api = table();
    let scene = Scene::new();
    let mut frame = scene.frame_input();
    // Through `pixels`, omitting `captured_at_nanos` and `stride`. The omitted
    // stride means packed rows, which is what this frame is.
    frame.struct_size = 40;
    frame.captured_at_nanos = u64::MAX;
    frame.stride = u64::MAX;

    let source = madopilot_source_t {
        struct_size: struct_size::<madopilot_source_t>(),
        kind: MADOPILOT_SOURCE_REPLAY_MEMORY,
        directory: madopilot_str_t::empty(),
        frames: &raw const frame,
        frame_count: 1,
        frame_stride: size_of::<madopilot_replay_frame_t>(),
        target_name: madopilot_str_t::empty(),
    };
    let operation = operation();
    let mut engine = ptr::null_mut();

    // SAFETY: every pointer is a live local that outlives the call.
    let status = unsafe {
        (api.engine_create)(
            &raw const source,
            &raw const operation,
            &raw mut engine,
            ptr::null_mut(),
        )
    };
    assert_eq!(
        status, MADOPILOT_STATUS_OK,
        "the poisoned fields beyond the declared size were never read"
    );
    // SAFETY: the engine was produced by this table and is owned here.
    unsafe { (api.engine_release)(engine) };
}

#[test]
fn an_input_smaller_than_its_mandatory_prefix_is_refused() {
    let api = table();
    let scene = Scene::new();
    let mut operation = operation();
    operation.struct_size = 4;

    let mut engine = ptr::null_mut();
    let mut error = ptr::null_mut();
    // SAFETY: as above.
    let status = unsafe {
        (api.engine_create)(
            scene.source(),
            &raw const operation,
            &raw mut engine,
            &raw mut error,
        )
    };
    assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT);
    assert!(engine.is_null(), "the owned output stays null");

    let detail = support::describe_and_release(api, error);
    assert_eq!(detail.status, MADOPILOT_STATUS_INVALID_ARGUMENT);
    assert_eq!(detail.category, MADOPILOT_ERROR_CATEGORY_ABI);
}

// --- Pointer, length, and arithmetic validation ----------------------------

#[test]
fn a_null_pointer_with_a_nonzero_length_is_refused() {
    let api = table();
    let scene = Scene::new();
    let mut frame = scene.frame_input();
    frame.pixels = madopilot_bytes_t {
        data: ptr::null(),
        len: 4096,
    };

    let status = create_with_frame(api, &frame, 1, size_of::<madopilot_replay_frame_t>());
    assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT);
}

#[test]
fn an_empty_view_is_accepted_where_it_is_permitted() {
    let api = table();
    let operation = operation();

    // `target_name` documents an empty view as "use the default name", and a
    // null pointer with a zero length is the empty view.
    let mut source = madopilot_source_t {
        struct_size: struct_size::<madopilot_source_t>(),
        kind: MADOPILOT_SOURCE_REPLAY_MEMORY,
        directory: madopilot_str_t::empty(),
        frames: ptr::null(),
        frame_count: 1,
        frame_stride: size_of::<madopilot_replay_frame_t>(),
        target_name: madopilot_str_t::empty(),
    };
    let scene = Scene::new();
    let frame = scene.frame_input();
    source.frames = &raw const frame;

    let mut engine = ptr::null_mut();
    // SAFETY: every pointer is a live local that outlives the call.
    let status = unsafe {
        (api.engine_create)(
            &raw const source,
            &raw const operation,
            &raw mut engine,
            ptr::null_mut(),
        )
    };
    assert_eq!(status, MADOPILOT_STATUS_OK);
    // SAFETY: owned here.
    unsafe { (api.engine_release)(engine) };
}

#[test]
fn a_count_and_stride_that_overflow_are_refused_before_any_address_is_formed() {
    let api = table();
    let scene = Scene::new();
    let frame = scene.frame_input();

    let status = create_with_frame(api, &frame, usize::MAX, 64);
    assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT);
}

#[test]
fn an_element_stride_below_the_mandatory_prefix_is_refused() {
    let api = table();
    let scene = Scene::new();
    let frame = scene.frame_input();

    // The library cannot walk an array whose elements are smaller than the
    // prefix it has to read from each one.
    let status = create_with_frame(api, &frame, 1, 8);
    assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT);
}

#[test]
fn an_unrecognized_tag_is_refused() {
    let api = table();
    let scene = Scene::new();
    let operation = operation();
    let frame = scene.frame_input();
    let source = madopilot_source_t {
        struct_size: struct_size::<madopilot_source_t>(),
        kind: 9999,
        directory: madopilot_str_t::empty(),
        frames: &raw const frame,
        frame_count: 1,
        frame_stride: size_of::<madopilot_replay_frame_t>(),
        target_name: madopilot_str_t::empty(),
    };

    let mut engine = ptr::null_mut();
    // SAFETY: every pointer is a live local that outlives the call.
    let status = unsafe {
        (api.engine_create)(
            &raw const source,
            &raw const operation,
            &raw mut engine,
            ptr::null_mut(),
        )
    };
    assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT);
    assert!(engine.is_null());
}

#[test]
fn a_string_that_is_not_utf8_is_refused() {
    let api = table();
    let operation = operation();
    let invalid = [0xffu8, 0xfe, 0xfd];
    let source = madopilot_source_t {
        struct_size: struct_size::<madopilot_source_t>(),
        kind: MADOPILOT_SOURCE_REPLAY_DIRECTORY,
        directory: madopilot_str_t {
            data: invalid.as_ptr().cast::<std::ffi::c_char>(),
            len: invalid.len(),
        },
        frames: ptr::null(),
        frame_count: 0,
        frame_stride: 0,
        target_name: madopilot_str_t::empty(),
    };

    let mut engine = ptr::null_mut();
    // SAFETY: every pointer is a live local that outlives the call.
    let status = unsafe {
        (api.engine_create)(
            &raw const source,
            &raw const operation,
            &raw mut engine,
            ptr::null_mut(),
        )
    };
    assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT);
}

#[test]
fn a_null_required_output_is_refused() {
    let api = table();
    let scene = Scene::new();
    let operation = operation();

    // SAFETY: passing null for the owned output is what this checks.
    let status = unsafe {
        (api.engine_create)(
            scene.source(),
            &raw const operation,
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT);
}

#[test]
fn a_behaviour_bearing_entry_rejects_a_null_handle() {
    let api = table();
    let mut count = 42_usize;

    // SAFETY: `count` is a live local; the handle is deliberately null.
    let status = unsafe { (api.target_list_count)(ptr::null(), &raw mut count) };
    assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT);
    assert_eq!(count, 0, "the scalar output was cleared before validation");
}

#[test]
fn retain_and_release_accept_null() {
    let api = table();

    // SAFETY: null is the documented no-op for every retain and release.
    unsafe {
        assert_eq!((api.engine_retain)(ptr::null()), MADOPILOT_STATUS_OK);
        assert_eq!((api.engine_release)(ptr::null_mut()), MADOPILOT_STATUS_OK);
        assert_eq!((api.error_release)(ptr::null_mut()), MADOPILOT_STATUS_OK);
        assert_eq!((api.result_release)(ptr::null_mut()), MADOPILOT_STATUS_OK);
        assert_eq!((api.mapping_release)(ptr::null_mut()), MADOPILOT_STATUS_OK);
    }
}

// --- Deadlines and cancellation --------------------------------------------

#[test]
fn an_already_expired_deadline_stops_every_blocking_entry() {
    let api = table();
    let scene = Scene::new();
    let expired = expired_operation();

    let mut engine = ptr::null_mut();
    let mut error = ptr::null_mut();
    // SAFETY: every pointer is a live local that outlives the call.
    let status = unsafe {
        (api.engine_create)(
            scene.source(),
            &raw const expired,
            &raw mut engine,
            &raw mut error,
        )
    };
    assert_eq!(status, MADOPILOT_STATUS_DEADLINE_EXCEEDED);
    assert!(
        engine.is_null(),
        "the owned output stays in its failure state"
    );

    let detail = support::describe_and_release(api, error);
    assert_eq!(detail.status, MADOPILOT_STATUS_DEADLINE_EXCEEDED);
    assert_eq!(detail.category, MADOPILOT_ERROR_CATEGORY_OPERATION);
}

#[test]
fn a_cancelled_token_stops_an_entry_before_it_starts() {
    let api = table();
    let scene = Scene::new();

    let mut cancellation = ptr::null_mut();
    // SAFETY: `cancellation` is a live local.
    assert_eq!(
        unsafe { (api.cancellation_create)(&raw mut cancellation) },
        MADOPILOT_STATUS_OK
    );
    // SAFETY: the handle is retained by this frame.
    assert_eq!(
        unsafe { (api.cancellation_cancel)(cancellation) },
        MADOPILOT_STATUS_OK
    );

    let mut cancelled = 0_i32;
    // SAFETY: as above.
    assert_eq!(
        unsafe { (api.cancellation_is_cancelled)(cancellation, &raw mut cancelled) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(cancelled, 1);

    let operation = madopilot_operation_t {
        struct_size: struct_size::<madopilot_operation_t>(),
        flags: 0,
        deadline_nanos: 0,
        cancellation,
    };
    let mut engine = ptr::null_mut();
    // SAFETY: as above.
    let status = unsafe {
        (api.engine_create)(
            scene.source(),
            &raw const operation,
            &raw mut engine,
            ptr::null_mut(),
        )
    };
    assert_eq!(status, MADOPILOT_STATUS_CANCELLED);
    assert!(engine.is_null());

    // SAFETY: this frame owns the reference it is giving up.
    unsafe { (api.cancellation_release)(cancellation) };
}

#[test]
fn the_clock_reports_a_domain_a_deadline_can_be_built_in() {
    let api = table();
    let mut first = 0_u64;
    let mut second = 0_u64;

    // SAFETY: both are live locals.
    unsafe {
        assert_eq!((api.clock_now)(&raw mut first), MADOPILOT_STATUS_OK);
        assert_eq!((api.clock_now)(&raw mut second), MADOPILOT_STATUS_OK);
    }
    assert!(second >= first, "the domain is monotonic");
}

// --- Status text ------------------------------------------------------------

#[test]
fn every_status_has_a_slug_and_an_unallocated_one_does_not_claim_a_name() {
    let api = table();

    for status in MADOPILOT_STATUS_OK..=MADOPILOT_STATUS_INTERNAL_PANIC {
        let mut text = madopilot_str_t::empty();
        // SAFETY: `text` is a live local.
        assert_eq!(
            unsafe { (api.status_text)(status, &raw mut text) },
            MADOPILOT_STATUS_OK
        );
        assert_ne!(text.len, 0, "status {status} has a slug");
    }

    let mut text = madopilot_str_t::empty();
    // SAFETY: as above.
    unsafe { (api.status_text)(MADOPILOT_STATUS_INTERNAL_PANIC + 1, &raw mut text) };
    // SAFETY: the view borrows static storage that lives as long as the library.
    let unrecognized = unsafe { std::slice::from_raw_parts(text.data.cast::<u8>(), text.len) };
    assert_eq!(unrecognized, b"unrecognized");
}

// --- The package source a caller cannot mistake -----------------------------

#[test]
fn a_directory_that_is_not_a_package_reports_the_rule_and_the_stage() {
    let api = table();
    let flow = support::Flow::open();
    let operation = operation();
    let missing = format!("{}/does-not-exist", package_root());

    let source = madopilot_package_source_t {
        struct_size: struct_size::<madopilot_package_source_t>(),
        kind: MADOPILOT_PACKAGE_SOURCE_DIRECTORY,
        path: str_view(&missing),
        archive: madopilot_bytes_t::empty(),
    };
    let mut package = ptr::null_mut();
    let mut error = ptr::null_mut();
    // SAFETY: every pointer is a live local, and the engine is retained by the
    // flow.
    let status = unsafe {
        (api.package_load)(
            flow.engine,
            &raw const source,
            &raw const operation,
            &raw mut package,
            &raw mut error,
        )
    };
    assert_ne!(status, MADOPILOT_STATUS_OK);
    assert!(package.is_null());

    let detail = support::describe_and_release(api, error);
    assert_eq!(detail.category, MADOPILOT_ERROR_CATEGORY_ASSET);
    assert_ne!(
        detail.flags & MADOPILOT_ERROR_HAS_ASSET_DETAIL,
        0,
        "package loading is the one operation that carries more than a status"
    );
    assert_eq!(detail.asset_fault, MADOPILOT_ASSET_FAULT_SOURCE_UNREADABLE);
    assert_eq!(detail.asset_stage, MADOPILOT_ASSET_STAGE_SOURCE);
}

#[test]
fn preparing_a_template_under_an_expired_deadline_publishes_no_handle() {
    let api = table();
    let flow = support::Flow::open();
    let expired = expired_operation();

    let mut prepared = ptr::null_mut();
    let mut error = ptr::null_mut();
    // SAFETY: both handles are retained by the flow and every pointer is a live
    // local.
    let status = unsafe {
        (api.template_prepare_from_package)(
            flow.engine,
            flow.package,
            str_view("panel.patch"),
            &raw const expired,
            &raw mut prepared,
            &raw mut error,
        )
    };

    // This entry resolves the identity itself rather than going through
    // `Engine::prepare_from_package`, so it owns its own admission and commit.
    // A template it compiled and then lost the race for is dropped, not
    // published.
    assert_eq!(status, MADOPILOT_STATUS_DEADLINE_EXCEEDED);
    assert!(prepared.is_null(), "a refused entry writes no handle");

    let detail = support::describe_and_release(api, error);
    assert_eq!(detail.status, MADOPILOT_STATUS_DEADLINE_EXCEEDED);
}

#[test]
fn an_undeclared_template_is_the_callers_mistake_not_an_invalid_package() {
    let api = table();
    let flow = support::Flow::open();
    let operation = operation();

    let mut prepared = ptr::null_mut();
    let mut error = ptr::null_mut();
    // SAFETY: both handles are retained by the flow and every pointer is a live
    // local.
    let status = unsafe {
        (api.template_prepare_from_package)(
            flow.engine,
            flow.package,
            str_view("panel.nothing"),
            &raw const operation,
            &raw mut prepared,
            &raw mut error,
        )
    };
    assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT);
    assert!(prepared.is_null());

    let detail = support::describe_and_release(api, error);
    assert_eq!(detail.status, MADOPILOT_STATUS_INVALID_ARGUMENT);
    assert_eq!(detail.category, MADOPILOT_ERROR_CATEGORY_ASSET);

    // The rule and the stage, not just the status. Without them this is
    // indistinguishable from every other malformed request, which is what left
    // `MADOPILOT_ASSET_FAULT_UNKNOWN_TEMPLATE` declared and unreachable.
    assert_ne!(detail.flags & MADOPILOT_ERROR_HAS_ASSET_DETAIL, 0);
    assert_eq!(detail.asset_fault, MADOPILOT_ASSET_FAULT_UNKNOWN_TEMPLATE);
    assert_eq!(detail.asset_stage, MADOPILOT_ASSET_STAGE_COMMIT);

    // No backend is named, because the identity was refused before one ran.
    assert_eq!(detail.flags & MADOPILOT_ERROR_HAS_BACKEND, 0);
}

// --- Helpers ----------------------------------------------------------------

fn build_info() -> madopilot_build_info_t {
    madopilot_build_info_t {
        struct_size: struct_size::<madopilot_build_info_t>(),
        flags: 0,
        abi_major: 0,
        abi_minor: 0,
        table_size: 0,
        reserved: 0,
        library_version: madopilot_str_t::empty(),
        required_backend: madopilot_str_t::empty(),
    }
}

fn create_with_frame(
    api: &'static madopilot_api_t,
    frame: &madopilot_replay_frame_t,
    count: usize,
    stride: usize,
) -> madopilot_status_t {
    let operation = operation();
    let source = madopilot_source_t {
        struct_size: struct_size::<madopilot_source_t>(),
        kind: MADOPILOT_SOURCE_REPLAY_MEMORY,
        directory: madopilot_str_t::empty(),
        frames: &raw const *frame,
        frame_count: count,
        frame_stride: stride,
        target_name: madopilot_str_t::empty(),
    };
    let mut engine = ptr::null_mut();

    // SAFETY: every pointer is a live local that outlives the call. A rejected
    // count or stride never becomes an address, which is the point of the test.
    let status = unsafe {
        (api.engine_create)(
            &raw const source,
            &raw const operation,
            &raw mut engine,
            ptr::null_mut(),
        )
    };
    // SAFETY: null is a no-op, and a success would leave a handle to release.
    unsafe { (api.engine_release)(engine) };

    status
}

#[test]
fn a_byte_view_helper_borrows_without_copying() {
    // Guards the support helper the other tests rely on: a view that pointed
    // somewhere else would make every pointer-validation test vacuous.
    let bytes = [1u8, 2, 3];
    let view = bytes_view(&bytes);
    assert_eq!(view.len, 3);
    assert_eq!(view.data, bytes.as_ptr());
}
