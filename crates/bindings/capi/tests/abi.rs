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
    // These two say only that the table advertises the constant the header
    // declares, which catches a wrong constant at the one place the table is
    // built and nothing more: `MADOPILOT_API_SIZE_PHASE1` is defined as
    // `size_of::<madopilot_api_t>()`, so neither can fail if a member's width
    // changes. The frozen size and layout are pinned in `layout.rs`, against
    // the committed `docs/evidence/c-abi/layout-*.txt`. Read this as a
    // self-consistency check, not as that guard.
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

/// A reported failure carries readable text, not just numbers.
///
/// Every other test here reads an error through `describe_and_release`, which
/// blanks the borrowed views on the way out. That is right for a caller keeping
/// the detail, but it meant the message surface was asserted only by the C++
/// probe: a regression that reported an empty message passed `cargo test`. The
/// text is not a compatibility promise and is deliberately not matched against
/// a fixed string — what is asserted is that there is one, and that it is the
/// diagnostic surface rather than a restatement of the status slug.
#[test]
fn a_refusal_reports_a_message_a_reader_can_use() {
    let api = table();
    let scene = Scene::new();
    let operation = operation();

    let mut engine = ptr::null_mut();
    let mut error = ptr::null_mut();
    // SAFETY: a null source is the refusal being provoked; the other outputs are
    // live locals.
    let status = unsafe {
        (api.engine_create)(
            ptr::null(),
            &raw const operation,
            &raw mut engine,
            &raw mut error,
        )
    };
    assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT);
    assert!(engine.is_null(), "the owned output stays null");
    drop(scene);

    let (detail, message) = support::describe_message_and_release(api, error);
    assert_eq!(detail.status, MADOPILOT_STATUS_INVALID_ARGUMENT);
    assert!(
        !message.is_empty(),
        "a refusal that accepted an out_error explains itself"
    );
    assert_ne!(
        message, "invalid_argument",
        "the message is diagnostic detail, not the status slug again"
    );
}

/// The mandatory prefix of every size-versioned structure, in both directions.
///
/// These are the numbers `src/` declares as `Versioned::MANDATORY` and
/// `Input::MANDATORY`, and ADR 0007 froze. They are written out here rather than
/// read from the crate because `Versioned` and `Input` are internal: what a
/// caller can observe is the refusal, so that is what is asserted, from both
/// sides. One byte below the prefix must be refused, and the prefix itself must
/// be accepted — without the second half, raising a prefix would go unnoticed;
/// without the first, lowering one would.
#[test]
fn every_versioned_output_refuses_a_size_below_its_mandatory_prefix() {
    let api = table();
    let flow = support::Flow::open();
    let request = flow.find_request();
    let result = flow.find(&request);
    let mapping = flow.map();
    let error = support::refused_error(api);

    assert_output_prefix("describe_build", 20, |out| unsafe {
        (api.describe_build)(out)
    });
    assert_output_prefix("error_describe", 16, |out| unsafe {
        (api.error_describe)(error, out)
    });
    assert_output_prefix("target_list_get", 24, |out| unsafe {
        (api.target_list_get)(flow.targets, 0, out)
    });
    assert_output_prefix("session_describe", 32, |out| unsafe {
        (api.session_describe)(flow.session, out)
    });
    assert_output_prefix("frame_stamp", 40, |out| unsafe {
        (api.frame_stamp)(flow.frame, out)
    });
    assert_output_prefix("frame_describe", 24, |out| unsafe {
        (api.frame_describe)(flow.frame, out)
    });
    assert_output_prefix("mapping_describe", 48, |out| unsafe {
        (api.mapping_describe)(mapping, out)
    });
    assert_output_prefix("mapping_stamp", 40, |out| unsafe {
        (api.mapping_stamp)(mapping, out)
    });
    assert_output_prefix("package_describe", 64, |out| unsafe {
        (api.package_describe)(flow.package, out)
    });
    assert_output_prefix("template_describe", 64, |out| unsafe {
        (api.template_describe)(flow.present, out)
    });
    assert_output_prefix("result_describe", 72, |out| unsafe {
        (api.result_describe)(result, out)
    });
    assert_output_prefix("result_stamp", 40, |out| unsafe {
        (api.result_stamp)(result, out)
    });
    // The one structure with two prefixes: 8 bytes as the options a caller
    // supplies, 24 as the report of what the search ran under. Only ADR 0007
    // records the asymmetry, so only a test keeps it.
    assert_output_prefix("result_options", 24, |out| unsafe {
        (api.result_options)(result, out)
    });
    assert_output_prefix("result_match", 56, |out| unsafe {
        (api.result_match)(result, 0, out)
    });

    // SAFETY: each handle is owned by this frame.
    unsafe {
        (api.mapping_release)(mapping);
        (api.result_release)(result);
    }
}

#[test]
fn every_versioned_input_refuses_a_size_below_its_mandatory_prefix() {
    let api = table();
    let flow = support::Flow::open();
    let scene = Scene::new();

    assert_input_prefix(
        "madopilot_operation_t",
        8,
        operation(),
        |operation| unsafe {
            let mut targets = ptr::null_mut();
            let status =
                (api.engine_discover)(flow.engine, operation, &raw mut targets, ptr::null_mut());
            (api.target_list_release)(targets);
            status
        },
    );

    assert_input_prefix(
        "madopilot_source_t",
        48,
        scene.source_input(),
        |source| unsafe {
            let operation = operation();
            let mut engine = ptr::null_mut();
            let status = (api.engine_create)(
                source,
                &raw const operation,
                &raw mut engine,
                ptr::null_mut(),
            );
            (api.engine_release)(engine);
            status
        },
    );

    // The array element the library reads one at a time, whose declared size is
    // checked per element rather than once for the array.
    assert_input_prefix(
        "madopilot_replay_frame_t",
        40,
        scene.frame_input(),
        |frame| create_with_frame(api, frame, 1, size_of::<madopilot_replay_frame_t>()),
    );

    assert_input_prefix(
        "madopilot_package_source_t",
        24,
        package_source(flow.root()),
        |source| unsafe {
            let operation = operation();
            let mut package = ptr::null_mut();
            let status = (api.package_load)(
                flow.engine,
                source,
                &raw const operation,
                &raw mut package,
                ptr::null_mut(),
            );
            (api.package_release)(package);
            status
        },
    );

    assert_input_prefix(
        "madopilot_open_request_t",
        8,
        open_request(),
        |request| unsafe {
            let operation = operation();
            let mut session = ptr::null_mut();
            let status = (api.session_open)(
                flow.engine,
                flow.targets,
                0,
                request,
                &raw const operation,
                &raw mut session,
                ptr::null_mut(),
            );
            (api.session_close)(session, &raw const operation, ptr::null_mut());
            (api.session_release)(session);
            status
        },
    );

    assert_input_prefix(
        "madopilot_map_request_t",
        12,
        map_request(),
        |request| unsafe {
            let operation = operation();
            let mut mapping = ptr::null_mut();
            let status = (api.frame_map)(
                flow.frame,
                request,
                &raw const operation,
                &raw mut mapping,
                ptr::null_mut(),
            );
            (api.mapping_release)(mapping);
            status
        },
    );

    assert_input_prefix(
        "madopilot_find_request_t",
        24,
        flow.find_request(),
        |request| unsafe {
            let operation = operation();
            let mut result = ptr::null_mut();
            let status = (api.session_find)(
                flow.session,
                request,
                &raw const operation,
                &raw mut result,
                ptr::null_mut(),
            );
            (api.result_release)(result);
            status
        },
    );

    // The in-direction prefix of the structure that also has an out-direction
    // one. No presence bit is set, which is the documented way of saying "the
    // template's own defaults" and is what an 8-byte options structure means.
    assert_input_prefix(
        "madopilot_match_options_t",
        8,
        madopilot_match_options_t::cleared(struct_size::<madopilot_match_options_t>()),
        |options| unsafe {
            let operation = operation();
            let mut request = flow.find_request();
            request.options = options;
            let mut result = ptr::null_mut();
            let status = (api.session_find)(
                flow.session,
                &raw const request,
                &raw const operation,
                &raw mut result,
                ptr::null_mut(),
            );
            (api.result_release)(result);
            status
        },
    );
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

    let status = create_with_frame(
        api,
        &raw const frame,
        1,
        size_of::<madopilot_replay_frame_t>(),
    );
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

    let status = create_with_frame(api, &raw const frame, usize::MAX, 64);
    assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT);
}

#[test]
fn an_element_stride_below_the_mandatory_prefix_is_refused() {
    let api = table();
    let scene = Scene::new();
    let frame = scene.frame_input();

    // The library cannot walk an array whose elements are smaller than the
    // prefix it has to read from each one.
    let status = create_with_frame(api, &raw const frame, 1, 8);
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
fn every_multi_output_entry_clears_a_valid_error_when_the_primary_output_is_null() {
    let api = table();

    assert_primary_rejection_clears_error(api, "engine_create", |out_error| unsafe {
        (api.engine_create)(ptr::null(), ptr::null(), ptr::null_mut(), out_error)
    });
    assert_primary_rejection_clears_error(api, "package_load", |out_error| unsafe {
        (api.package_load)(
            ptr::null(),
            ptr::null(),
            ptr::null(),
            ptr::null_mut(),
            out_error,
        )
    });
    assert_primary_rejection_clears_error(
        api,
        "template_prepare_from_package",
        |out_error| unsafe {
            (api.template_prepare_from_package)(
                ptr::null(),
                ptr::null(),
                madopilot_str_t::empty(),
                ptr::null(),
                ptr::null_mut(),
                out_error,
            )
        },
    );
    assert_primary_rejection_clears_error(api, "engine_discover", |out_error| unsafe {
        (api.engine_discover)(ptr::null(), ptr::null(), ptr::null_mut(), out_error)
    });
    assert_primary_rejection_clears_error(api, "session_open", |out_error| unsafe {
        (api.session_open)(
            ptr::null(),
            ptr::null(),
            0,
            ptr::null(),
            ptr::null(),
            ptr::null_mut(),
            out_error,
        )
    });
    assert_primary_rejection_clears_error(api, "session_acquire_frame", |out_error| unsafe {
        (api.session_acquire_frame)(ptr::null(), ptr::null(), ptr::null_mut(), out_error)
    });
    assert_primary_rejection_clears_error(api, "frame_map", |out_error| unsafe {
        (api.frame_map)(
            ptr::null(),
            ptr::null(),
            ptr::null(),
            ptr::null_mut(),
            out_error,
        )
    });
    assert_primary_rejection_clears_error(api, "session_find", |out_error| unsafe {
        (api.session_find)(
            ptr::null(),
            ptr::null(),
            ptr::null(),
            ptr::null_mut(),
            out_error,
        )
    });
}

#[test]
fn every_multi_output_entry_clears_a_valid_error_when_the_primary_output_is_misaligned() {
    let api = table();

    with_misaligned_handle_output(|out_engine| {
        assert_primary_rejection_clears_error(api, "engine_create", |out_error| unsafe {
            (api.engine_create)(ptr::null(), ptr::null(), out_engine, out_error)
        });
    });
    with_misaligned_handle_output(|out_package| {
        assert_primary_rejection_clears_error(api, "package_load", |out_error| unsafe {
            (api.package_load)(
                ptr::null(),
                ptr::null(),
                ptr::null(),
                out_package,
                out_error,
            )
        });
    });
    with_misaligned_handle_output(|out_template| {
        assert_primary_rejection_clears_error(
            api,
            "template_prepare_from_package",
            |out_error| unsafe {
                (api.template_prepare_from_package)(
                    ptr::null(),
                    ptr::null(),
                    madopilot_str_t::empty(),
                    ptr::null(),
                    out_template,
                    out_error,
                )
            },
        );
    });
    with_misaligned_handle_output(|out_targets| {
        assert_primary_rejection_clears_error(api, "engine_discover", |out_error| unsafe {
            (api.engine_discover)(ptr::null(), ptr::null(), out_targets, out_error)
        });
    });
    with_misaligned_handle_output(|out_session| {
        assert_primary_rejection_clears_error(api, "session_open", |out_error| unsafe {
            (api.session_open)(
                ptr::null(),
                ptr::null(),
                0,
                ptr::null(),
                ptr::null(),
                out_session,
                out_error,
            )
        });
    });
    with_misaligned_handle_output(|out_frame| {
        assert_primary_rejection_clears_error(api, "session_acquire_frame", |out_error| unsafe {
            (api.session_acquire_frame)(ptr::null(), ptr::null(), out_frame, out_error)
        });
    });
    with_misaligned_handle_output(|out_mapping| {
        assert_primary_rejection_clears_error(api, "frame_map", |out_error| unsafe {
            (api.frame_map)(
                ptr::null(),
                ptr::null(),
                ptr::null(),
                out_mapping,
                out_error,
            )
        });
    });
    with_misaligned_handle_output(|out_result| {
        assert_primary_rejection_clears_error(api, "session_find", |out_error| unsafe {
            (api.session_find)(ptr::null(), ptr::null(), ptr::null(), out_result, out_error)
        });
    });
}

#[test]
fn every_multi_output_entry_clears_a_valid_primary_when_the_error_output_is_misaligned() {
    let api = table();

    assert_error_rejection_clears_primary("engine_create", |out_engine, out_error| unsafe {
        (api.engine_create)(ptr::null(), ptr::null(), out_engine, out_error)
    });
    assert_error_rejection_clears_primary("package_load", |out_package, out_error| unsafe {
        (api.package_load)(
            ptr::null(),
            ptr::null(),
            ptr::null(),
            out_package,
            out_error,
        )
    });
    assert_error_rejection_clears_primary(
        "template_prepare_from_package",
        |out_template, out_error| unsafe {
            (api.template_prepare_from_package)(
                ptr::null(),
                ptr::null(),
                madopilot_str_t::empty(),
                ptr::null(),
                out_template,
                out_error,
            )
        },
    );
    assert_error_rejection_clears_primary("engine_discover", |out_targets, out_error| unsafe {
        (api.engine_discover)(ptr::null(), ptr::null(), out_targets, out_error)
    });
    assert_error_rejection_clears_primary("session_open", |out_session, out_error| unsafe {
        (api.session_open)(
            ptr::null(),
            ptr::null(),
            0,
            ptr::null(),
            ptr::null(),
            out_session,
            out_error,
        )
    });
    assert_error_rejection_clears_primary("session_acquire_frame", |out_frame, out_error| unsafe {
        (api.session_acquire_frame)(ptr::null(), ptr::null(), out_frame, out_error)
    });
    assert_error_rejection_clears_primary("frame_map", |out_mapping, out_error| unsafe {
        (api.frame_map)(
            ptr::null(),
            ptr::null(),
            ptr::null(),
            out_mapping,
            out_error,
        )
    });
    assert_error_rejection_clears_primary("session_find", |out_result, out_error| unsafe {
        (api.session_find)(ptr::null(), ptr::null(), ptr::null(), out_result, out_error)
    });
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
    let status = unsafe { (api.status_text)(MADOPILOT_STATUS_INTERNAL_PANIC + 1, &raw mut text) };
    // The status was discarded here, and the view was then dereferenced without
    // being checked. That is the wrong way round for this test in particular:
    // the regression it exists to catch is the library leaving the view in its
    // null failure state, and `from_raw_parts` requires a non-null pointer even
    // at length zero, so the test would have been undefined behaviour on
    // exactly the failure it was watching for.
    assert_eq!(
        status, MADOPILOT_STATUS_OK,
        "an unrecognized status is still a well-formed question"
    );
    assert!(
        !text.data.is_null(),
        "a succeeding status_text leaves a readable view"
    );
    // SAFETY: the status is OK and the pointer is non-null, so the view borrows
    // static storage that lives as long as the library.
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

// --- The one coordinate space a caller may supply ---------------------------

/// Every coordinate space this build has a number for, other than the one a
/// caller-supplied region may use.
const UNACCEPTED_SPACES: [madopilot_space_t; 4] = [
    MADOPILOT_SPACE_FRAME_NORMALIZED,
    MADOPILOT_SPACE_TARGET_NORMALIZED,
    MADOPILOT_SPACE_TARGET_LOGICAL,
    MADOPILOT_SPACE_DESKTOP_LOGICAL,
];

/// The status is asserted, not merely the failure.
///
/// The Phase 1 prefix has no coordinate-conversion entry, so a region in a space
/// it does not read is invalid argument from the boundary rather than
/// `MADOPILOT_STATUS_UNSUPPORTED`, which stays reserved for a request the table
/// does read and cannot satisfy. `docs/c-abi.md` documents that split, and the
/// Rust facade — which does convert — answers the equivalent question with its
/// own unsupported-coordinate outcome instead. A test that only asked "did it
/// fail" would pass whichever of the two the boundary happened to return.
#[test]
fn a_mapping_region_outside_capture_pixels_is_refused_by_the_boundary() {
    let api = table();
    let flow = support::Flow::open();
    let operation = operation();

    for space in UNACCEPTED_SPACES {
        let request = madopilot_map_request_t {
            struct_size: struct_size::<madopilot_map_request_t>(),
            flags: MADOPILOT_MAP_HAS_REGION,
            format: MADOPILOT_PIXEL_FORMAT_RGBA8,
            clip_policy: MADOPILOT_CLIP_POLICY_REJECT,
            region: madopilot_pixel_rect_t {
                space,
                left: 0,
                top: 0,
                right: 4,
                bottom: 4,
            },
        };
        let mut mapping = ptr::null_mut();
        let mut error = ptr::null_mut();
        // SAFETY: the frame is retained by the flow and every pointer is a live
        // local.
        let status = unsafe {
            (api.frame_map)(
                flow.frame,
                &raw const request,
                &raw const operation,
                &raw mut mapping,
                &raw mut error,
            )
        };

        assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT, "space {space}");
        assert!(mapping.is_null(), "space {space} publishes no mapping");

        let detail = support::describe_and_release(api, error);
        assert_eq!(detail.status, MADOPILOT_STATUS_INVALID_ARGUMENT);
        assert_eq!(
            detail.category, MADOPILOT_ERROR_CATEGORY_ABI,
            "the boundary refused it, so capture never saw it"
        );
    }
}

#[test]
fn a_search_region_outside_capture_pixels_is_refused_the_same_way() {
    let api = table();
    let flow = support::Flow::open();
    let operation = operation();

    for space in UNACCEPTED_SPACES {
        let mut request = flow.find_request();
        request.flags = MADOPILOT_FIND_HAS_REGION;
        request.region = madopilot_pixel_rect_t {
            space,
            left: 0,
            top: 0,
            right: 4,
            bottom: 4,
        };

        let mut result = ptr::null_mut();
        let mut error = ptr::null_mut();
        // SAFETY: every handle the request names is retained by the flow and
        // every pointer is a live local.
        let status = unsafe {
            (api.session_find)(
                flow.session,
                &raw const request,
                &raw const operation,
                &raw mut result,
                &raw mut error,
            )
        };

        // The two entries must not drift: a caller that learned the rule from
        // one of them applies it to the other.
        assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT, "space {space}");
        assert!(result.is_null(), "space {space} publishes no result");

        let detail = support::describe_and_release(api, error);
        assert_eq!(detail.category, MADOPILOT_ERROR_CATEGORY_ABI);
    }
}

#[test]
fn a_capture_pixel_region_is_the_one_a_caller_may_supply() {
    let api = table();
    let flow = support::Flow::open();
    let operation = operation();

    let request = madopilot_map_request_t {
        struct_size: struct_size::<madopilot_map_request_t>(),
        flags: MADOPILOT_MAP_HAS_REGION,
        format: MADOPILOT_PIXEL_FORMAT_RGBA8,
        clip_policy: MADOPILOT_CLIP_POLICY_REJECT,
        region: madopilot_pixel_rect_t {
            space: MADOPILOT_SPACE_CAPTURE_PIXELS,
            left: 0,
            top: 0,
            right: 4,
            bottom: 4,
        },
    };
    let mut mapping = ptr::null_mut();
    // SAFETY: the frame is retained by the flow and every pointer is a live
    // local.
    let status = unsafe {
        (api.frame_map)(
            flow.frame,
            &raw const request,
            &raw const operation,
            &raw mut mapping,
            ptr::null_mut(),
        )
    };

    assert_eq!(status, MADOPILOT_STATUS_OK);
    assert!(!mapping.is_null());
    // SAFETY: the mapping was produced by this table and is owned here.
    unsafe { (api.mapping_release)(mapping) };
}

// --- Helpers ----------------------------------------------------------------

/// A byte no failure state and no successful value contains.
const POISON: u8 = 0xAA;

/// A structure whose every byte, padding included, is [`POISON`].
///
/// # Safety
///
/// `S` must have no invalid bit pattern. Every public structure qualifies:
/// their fields are fixed-width integers, `f32`, raw pointers, and aggregates
/// of those, none of which has a niche. Filling the padding as well is the
/// point — it makes "the library wrote nothing" a statement about every byte
/// rather than about the fields a test remembered to check.
unsafe fn poisoned<S: Copy>() -> S {
    let mut value = std::mem::MaybeUninit::<S>::uninit();

    // SAFETY: the write covers the whole structure, so no byte is left
    // uninitialized, and the caller's contract makes the result a valid `S`.
    unsafe {
        ptr::write_bytes(value.as_mut_ptr().cast::<u8>(), POISON, size_of::<S>());
        value.assume_init()
    }
}

/// Writes the first field of a size-versioned structure.
///
/// Every one of them begins with a `uint32_t struct_size` at offset zero, which
/// is how the library reads it before it knows anything else about the
/// structure. Reading and writing it the same way keeps these helpers free of a
/// per-structure accessor.
fn set_struct_size<S>(value: &mut S, size: u32) {
    // SAFETY: `S` is a `#[repr(C)]` structure whose first field is a `u32`, so
    // the cast addresses that field and nothing else.
    unsafe { (&raw mut *value).cast::<u32>().write(size) };
}

fn struct_size_of<S>(value: &S) -> u32 {
    // SAFETY: as `set_struct_size`.
    unsafe { (&raw const *value).cast::<u32>().read() }
}

/// Asserts where one output structure's mandatory prefix actually is.
///
/// One byte below it is refused with nothing written, and the prefix itself is
/// accepted and reported back. Both halves are needed: lowering a prefix makes
/// the first fail, raising one makes the second fail.
fn assert_output_prefix<S: Copy>(
    entry: &str,
    mandatory: u32,
    invoke: impl Fn(*mut S) -> madopilot_status_t,
) {
    assert!(
        mandatory as usize <= size_of::<S>(),
        "{entry} declares a {mandatory} byte prefix of a smaller structure"
    );

    // SAFETY: the output structures satisfy `poisoned`'s contract.
    let mut refused: S = unsafe { poisoned() };
    set_struct_size(&mut refused, mandatory - 1);

    let status = invoke(&raw mut refused);
    assert_eq!(
        status,
        MADOPILOT_STATUS_INVALID_ARGUMENT,
        "{entry} accepted an output declaring {} bytes, one below its {mandatory} byte mandatory \
         prefix",
        mandatory - 1
    );
    // SAFETY: every byte of `refused` was written, by `poisoned` and then by
    // `set_struct_size`, so none of them is uninitialized.
    let bytes =
        unsafe { std::slice::from_raw_parts((&raw const refused).cast::<u8>(), size_of::<S>()) };
    assert!(
        bytes[size_of::<u32>()..].iter().all(|byte| *byte == POISON),
        "{entry} wrote through an output it refused"
    );

    // SAFETY: as above.
    let mut accepted: S = unsafe { poisoned() };
    set_struct_size(&mut accepted, mandatory);

    let status = invoke(&raw mut accepted);
    assert_eq!(
        status, MADOPILOT_STATUS_OK,
        "{entry} refused an output declaring its own {mandatory} byte mandatory prefix"
    );
    assert_eq!(
        struct_size_of(&accepted),
        mandatory,
        "{entry} reports the prefix it filled, not the one it knows"
    );
}

/// Asserts where one input structure's mandatory prefix actually is.
///
/// The value passed in is a request the library accepts at its full size, so
/// the only variable is the declared size. A prefix-length request omits the
/// fields after it, which the library defaults.
fn assert_input_prefix<S: Copy>(
    argument: &str,
    mandatory: u32,
    valid: S,
    invoke: impl Fn(*const S) -> madopilot_status_t,
) {
    assert!(
        mandatory as usize <= size_of::<S>(),
        "{argument} declares a {mandatory} byte prefix of a smaller structure"
    );

    let mut refused = valid;
    set_struct_size(&mut refused, mandatory - 1);
    assert_eq!(
        invoke(&raw const refused),
        MADOPILOT_STATUS_INVALID_ARGUMENT,
        "{argument} was accepted at {} bytes, one below its {mandatory} byte mandatory prefix",
        mandatory - 1
    );

    let mut accepted = valid;
    set_struct_size(&mut accepted, mandatory);
    assert_eq!(
        invoke(&raw const accepted),
        MADOPILOT_STATUS_OK,
        "{argument} was refused at its own {mandatory} byte mandatory prefix"
    );
}

fn open_request() -> madopilot_open_request_t {
    madopilot_open_request_t {
        struct_size: struct_size::<madopilot_open_request_t>(),
        flags: 0,
        required_format: MADOPILOT_PIXEL_FORMAT_RGBA8,
        preferred_format: MADOPILOT_PIXEL_FORMAT_RGBA8,
    }
}

fn map_request() -> madopilot_map_request_t {
    madopilot_map_request_t {
        struct_size: struct_size::<madopilot_map_request_t>(),
        flags: 0,
        format: MADOPILOT_PIXEL_FORMAT_RGBA8,
        clip_policy: MADOPILOT_CLIP_POLICY_REJECT,
        region: madopilot_pixel_rect_t::empty(),
    }
}

/// A directory package source over the tracked fixture.
///
/// The view borrows `path`, which every caller below keeps alive for the call.
fn package_source(path: &str) -> madopilot_package_source_t {
    madopilot_package_source_t {
        struct_size: struct_size::<madopilot_package_source_t>(),
        kind: MADOPILOT_PACKAGE_SOURCE_DIRECTORY,
        path: str_view(path),
        archive: madopilot_bytes_t::empty(),
    }
}

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

fn assert_primary_rejection_clears_error(
    api: &'static madopilot_api_t,
    entry: &str,
    invoke: impl FnOnce(*mut *mut madopilot_error_t) -> madopilot_status_t,
) {
    let sentinel = error_sentinel(api);
    let mut error = sentinel;

    let status = invoke(&raw mut error);
    assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT, "{entry}");
    assert!(
        error.is_null(),
        "{entry} must clear a valid out_error even when its primary output is invalid"
    );

    // SAFETY: null is the documented no-op cleanup path. `sentinel` is the
    // original owned handle saved before the output slot was cleared.
    unsafe {
        assert_eq!(
            (api.error_release)(error),
            MADOPILOT_STATUS_OK,
            "{entry} must leave no stale release path"
        );
        assert_eq!(
            (api.error_release)(sentinel),
            MADOPILOT_STATUS_OK,
            "release the saved sentinel owner"
        );
    }
}

fn assert_error_rejection_clears_primary<T>(
    entry: &str,
    invoke: impl FnOnce(*mut *mut T, *mut *mut madopilot_error_t) -> madopilot_status_t,
) {
    let mut primary = ptr::NonNull::<T>::dangling().as_ptr();

    with_misaligned_handle_output(|out_error| {
        let status = invoke(&raw mut primary, out_error);
        assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT, "{entry}");
    });
    assert!(
        primary.is_null(),
        "{entry} must clear a valid primary output even when out_error is invalid"
    );
}

fn error_sentinel(api: &'static madopilot_api_t) -> *mut madopilot_error_t {
    let scene = Scene::new();
    let mut invalid_operation = operation();
    invalid_operation.struct_size = 0;
    let mut engine = ptr::null_mut();
    let mut error = ptr::null_mut();

    // SAFETY: all outputs are live locals. The undersized operation is rejected
    // after output initialization and produces an owned public error handle.
    let status = unsafe {
        (api.engine_create)(
            scene.source(),
            &raw const invalid_operation,
            &raw mut engine,
            &raw mut error,
        )
    };
    assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT);
    assert!(engine.is_null());
    assert!(!error.is_null());

    error
}

fn with_misaligned_handle_output<T>(invoke: impl FnOnce(*mut *mut T)) {
    let mut storage = [0_usize; 2];
    let output = storage
        .as_mut_ptr()
        .cast::<u8>()
        .wrapping_add(1)
        .cast::<*mut T>();
    assert!(!output.is_aligned());

    invoke(output);
}

fn create_with_frame(
    api: &madopilot_api_t,
    frame: *const madopilot_replay_frame_t,
    count: usize,
    stride: usize,
) -> madopilot_status_t {
    let operation = operation();
    let source = madopilot_source_t {
        struct_size: struct_size::<madopilot_source_t>(),
        kind: MADOPILOT_SOURCE_REPLAY_MEMORY,
        directory: madopilot_str_t::empty(),
        frames: frame,
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
