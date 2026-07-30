//! What a caller's declared size has to agree with before it is believed.
//!
//! `abi.rs` proves that a size below the mandatory prefix is refused and that a
//! shorter valid prefix defaults the rest. These are the three ways a size can
//! clear that lower bound and still describe a structure that cannot exist: it
//! ends inside a field, it exceeds the element stride of the array it was read
//! out of, or it stops short of a field a presence bit says is set. Each one
//! makes the library read something the caller never supplied, and the worst of
//! them fabricates a pointer.

// Every call below goes through an `unsafe extern "C"` table entry under the
// same safety argument `abi.rs` states once: every pointer is a live local of
// the test, and every handle is retained by the value that produced it for
// longer than the call lasts.
#![allow(clippy::undocumented_unsafe_blocks)]

mod support;

use std::ptr;

use madopilot::layout::struct_size;
use madopilot::*;
use support::{Flow, Scene, expired_operation, operation, table};

// --- A prefix ends at a field boundary --------------------------------------

#[test]
fn an_operation_prefix_that_ends_inside_a_field_is_refused() {
    let api = table();
    let scene = Scene::new();

    // `deadline_nanos` occupies bytes 8..16, so 12 supplies half of it. The
    // other half would come from the defaults, making a deadline that is
    // neither the caller's nor the documented "no deadline".
    let mut operation = operation();
    operation.struct_size = 12;

    let mut engine = ptr::null_mut();
    let mut error = ptr::null_mut();
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
    assert_eq!(detail.category, MADOPILOT_ERROR_CATEGORY_ABI);
}

#[test]
fn an_operation_prefix_that_ends_inside_the_cancellation_handle_is_refused_before_it_is_read() {
    let api = table();
    let scene = Scene::new();

    let mut cancellation = ptr::null_mut();
    assert_eq!(
        unsafe { (api.cancellation_create)(&raw mut cancellation) },
        MADOPILOT_STATUS_OK
    );

    // `cancellation` occupies bytes 16..24. A size of 20 splices the low half
    // of a real handle onto the high half of the default, which is an address
    // the caller never passed and the library would otherwise dereference.
    let mut operation = operation();
    operation.struct_size = 20;
    operation.cancellation = cancellation;

    let mut engine = ptr::null_mut();
    let status = unsafe {
        (api.engine_create)(
            scene.source(),
            &raw const operation,
            &raw mut engine,
            ptr::null_mut(),
        )
    };
    assert_eq!(
        status, MADOPILOT_STATUS_INVALID_ARGUMENT,
        "a half-supplied handle is refused rather than assembled"
    );
    assert!(engine.is_null());

    unsafe { (api.cancellation_release)(cancellation) };
}

#[test]
fn an_operation_prefix_that_ends_at_a_field_boundary_omits_the_fields_after_it() {
    let api = table();
    let scene = Scene::new();

    let mut cancellation = ptr::null_mut();
    assert_eq!(
        unsafe { (api.cancellation_create)(&raw mut cancellation) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(
        unsafe { (api.cancellation_cancel)(cancellation) },
        MADOPILOT_STATUS_OK
    );

    // Sixteen bytes is where `cancellation` begins, so the field is omitted
    // rather than half read. The cancelled handle the caller left in its own
    // storage is not part of the request, and the call proceeds.
    let mut operation = operation();
    operation.struct_size = 16;
    operation.cancellation = cancellation;

    let mut engine = ptr::null_mut();
    let status = unsafe {
        (api.engine_create)(
            scene.source(),
            &raw const operation,
            &raw mut engine,
            ptr::null_mut(),
        )
    };
    assert_eq!(
        status, MADOPILOT_STATUS_OK,
        "a prefix that stops at a boundary is a supported caller, not a broken one"
    );

    unsafe { (api.engine_release)(engine) };
    unsafe { (api.cancellation_release)(cancellation) };
}

#[test]
fn a_find_request_prefix_that_ends_inside_the_options_pointer_is_refused() {
    let api = table();
    let flow = Flow::open();

    let options = madopilot_match_options_t::cleared(struct_size::<madopilot_match_options_t>());
    let mut request = flow.find_request();
    // `options` occupies bytes 24..32; 28 supplies half of the pointer.
    request.struct_size = 28;
    request.options = &raw const options;

    let operation = operation();
    let mut result = ptr::null_mut();
    let status = unsafe {
        (api.session_find)(
            flow.session,
            &raw const request,
            &raw const operation,
            &raw mut result,
            ptr::null_mut(),
        )
    };
    assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT);
    assert!(result.is_null());
}

// --- An element fits the stride it sits at ----------------------------------

#[test]
fn a_replay_frame_larger_than_the_stride_it_sits_at_is_refused() {
    let api = table();
    let scene = Scene::new();

    // The element declares the full structure while the array declares a
    // 40-byte stride. Believing the element would read sixteen bytes past the
    // extent the array itself described.
    let frame = scene.frame_input();
    assert_eq!(
        frame.struct_size,
        struct_size::<madopilot_replay_frame_t>(),
        "the scene's element declares the whole structure"
    );

    let status = create_with_frame(api, &frame, 1, 40);
    assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT);
}

#[test]
fn a_replay_frame_array_packed_at_its_mandatory_prefix_is_accepted() {
    let api = table();
    let scene = Scene::new();

    // The same 40-byte stride with elements that declare 40 bytes is a caller
    // built against an older header, which stays supported.
    let mut frame = scene.frame_input();
    frame.struct_size = 40;

    let status = create_with_frame(api, &frame, 1, 40);
    assert_eq!(status, MADOPILOT_STATUS_OK);
}

#[test]
fn a_frame_count_no_library_allocation_could_answer_is_refused_at_its_first_element() {
    let api = table();
    let scene = Scene::new();

    // The largest count the array span still admits at the smallest legal
    // stride. It is a claim about caller memory this library cannot see, and
    // reserving for it is the library making that claim its own: the internal
    // frames the count implies are wider than the caller's own elements, so the
    // reservation is not even a representable allocation. What the caller gets
    // to read is the refusal of the first element the count promised.
    let mut frame = scene.frame_input();
    frame.struct_size = 8;

    let count = isize::MAX.unsigned_abs() / 40;
    let status = create_with_frame(api, &frame, count, 40);
    assert_eq!(
        status, MADOPILOT_STATUS_INVALID_ARGUMENT,
        "the count is answered with a status, not with an allocation"
    );
}

// --- A presence bit names a field the prefix covers -------------------------
//
// One case per bit the header declares, because the table those refusals come
// out of is written by hand: an entry nobody added is an entry no check that
// walks the table can miss, so each bit is exercised through the entry that
// reads its structure. Every case asserts the category as well as the status.
// A defaulted field is often invalid for some other reason — an empty rectangle
// fails the geometry contract, a zero deadline is already expired — so a bare
// `MADOPILOT_STATUS_INVALID_ARGUMENT` would pass just as well without the
// boundary refusing anything. A field whose default is *valid*, as both format
// fields of `madopilot_open_request_t` are, would not even be noticed.

#[test]
fn an_operation_deadline_bit_the_prefix_omits_is_refused() {
    let api = table();
    let scene = Scene::new();

    // Eight bytes is the mandatory prefix and a real field boundary, and it
    // stops before `deadline_nanos`. The default there is zero, which is the
    // domain origin and so an instant that has already passed.
    let mut operation = operation();
    operation.struct_size = 8;
    operation.flags = MADOPILOT_OPERATION_HAS_DEADLINE;

    let mut engine = ptr::null_mut();
    let mut error = ptr::null_mut();
    let status = unsafe {
        (api.engine_create)(
            scene.source(),
            &raw const operation,
            &raw mut engine,
            &raw mut error,
        )
    };
    assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT);
    assert!(engine.is_null());
    assert_eq!(
        refusal_category(api, error),
        MADOPILOT_ERROR_CATEGORY_ABI,
        "the refusal is the boundary's, not the expiry the defaulted deadline would have produced"
    );

    // The same bit at the full prefix is honored, and its failure belongs to
    // the operation the caller described rather than to the boundary.
    let expired = expired_operation();
    let mut engine = ptr::null_mut();
    let mut error = ptr::null_mut();
    let status = unsafe {
        (api.engine_create)(
            scene.source(),
            &raw const expired,
            &raw mut engine,
            &raw mut error,
        )
    };
    assert_eq!(status, MADOPILOT_STATUS_DEADLINE_EXCEEDED);
    assert_eq!(
        refusal_category(api, error),
        MADOPILOT_ERROR_CATEGORY_OPERATION
    );
}

#[test]
fn an_open_request_format_bit_the_prefix_omits_is_refused() {
    let api = table();
    let flow = Flow::open();

    // `required_format` occupies bytes 8..12 and `preferred_format` 12..16.
    // Both default to RGBA8, which is a format the session would open under
    // perfectly happily: nothing downstream can tell that the caller's own
    // choice was never read, so the refusal happens here or nowhere.
    for (prefix, flag) in [
        (8, MADOPILOT_OPEN_HAS_REQUIRED_FORMAT),
        (8, MADOPILOT_OPEN_HAS_PREFERRED_FORMAT),
        (12, MADOPILOT_OPEN_HAS_PREFERRED_FORMAT),
    ] {
        let (status, error) = open_session(api, &flow, &open_request(prefix, flag));
        assert_eq!(
            status, MADOPILOT_STATUS_INVALID_ARGUMENT,
            "a {prefix} byte open request may not claim {flag:#x}"
        );
        assert_eq!(refusal_category(api, error), MADOPILOT_ERROR_CATEGORY_ABI);
    }

    let whole = struct_size::<madopilot_open_request_t>();
    let both = MADOPILOT_OPEN_HAS_REQUIRED_FORMAT | MADOPILOT_OPEN_HAS_PREFERRED_FORMAT;
    let (status, error) = open_session(api, &flow, &open_request(whole, both));
    assert_eq!(
        status, MADOPILOT_STATUS_OK,
        "a prefix that covers both fields is a caller that supplied both"
    );
    assert!(error.is_null());
}

#[test]
fn a_map_request_region_bit_the_prefix_omits_is_refused() {
    let api = table();
    let flow = Flow::open();
    let bounds = frame_bounds(api, &flow);

    // `region` occupies bytes 16..36, so the two shorter boundaries both stop
    // before it. The default is an empty rectangle, which the geometry contract
    // refuses on its own account — with its own category, which is why this
    // asserts the boundary's.
    for prefix in [12, 16] {
        let (status, error) = map_frame(
            api,
            &flow,
            &map_request(prefix, MADOPILOT_MAP_HAS_REGION, bounds),
        );
        assert_eq!(
            status, MADOPILOT_STATUS_INVALID_ARGUMENT,
            "a {prefix} byte map request may not claim a region"
        );
        assert_eq!(refusal_category(api, error), MADOPILOT_ERROR_CATEGORY_ABI);
    }

    let whole = struct_size::<madopilot_map_request_t>();
    let (status, error) = map_frame(
        api,
        &flow,
        &map_request(whole, MADOPILOT_MAP_HAS_REGION, bounds),
    );
    assert_eq!(
        status, MADOPILOT_STATUS_OK,
        "a prefix that covers the region is a caller that supplied one"
    );
    assert!(error.is_null());
}

#[test]
fn a_find_request_region_bit_the_prefix_omits_is_refused() {
    let api = table();
    let flow = Flow::open();
    let bounds = frame_bounds(api, &flow);

    // `region` occupies bytes 32..52, so both shorter boundaries stop before
    // it, and the defaulted empty rectangle is again refused by the geometry
    // contract for its own reasons at the same status.
    for prefix in [24, 32] {
        let mut request = flow.find_request();
        request.struct_size = prefix;
        request.flags = MADOPILOT_FIND_HAS_REGION;
        request.region = bounds;

        let (status, error) = run_find(api, &flow, &request);
        assert_eq!(
            status, MADOPILOT_STATUS_INVALID_ARGUMENT,
            "a {prefix} byte find request may not claim a region"
        );
        assert_eq!(refusal_category(api, error), MADOPILOT_ERROR_CATEGORY_ABI);
    }

    let mut request = flow.find_request();
    request.flags = MADOPILOT_FIND_HAS_REGION;
    request.region = bounds;
    let (status, error) = run_find(api, &flow, &request);
    assert_eq!(
        status, MADOPILOT_STATUS_OK,
        "a prefix that covers the region is a caller that supplied one"
    );
    assert!(error.is_null());
}

#[test]
fn a_match_option_bit_for_a_field_the_prefix_omits_is_refused() {
    let api = table();
    let flow = Flow::open();

    // Eight bytes is the mandatory prefix and a real field boundary, but it
    // stops before `min_score`. Honoring the bit there would search at a
    // threshold of 0.0, which qualifies every candidate, and report that same
    // 0.0 back as the threshold the search ran under. `max_results` and
    // `suppression` sit after it, so each one has boundaries of its own that
    // stop short.
    for (prefix, flag) in [
        (8, MADOPILOT_MATCH_HAS_MIN_SCORE),
        (8, MADOPILOT_MATCH_HAS_MAX_RESULTS),
        (16, MADOPILOT_MATCH_HAS_MAX_RESULTS),
        (8, MADOPILOT_MATCH_HAS_SUPPRESSION),
        (16, MADOPILOT_MATCH_HAS_SUPPRESSION),
        (20, MADOPILOT_MATCH_HAS_SUPPRESSION),
    ] {
        let mut options = madopilot_match_options_t::cleared(prefix);
        options.flags = flag;

        let mut request = flow.find_request();
        request.options = &raw const options;

        let (status, error) = run_find(api, &flow, &request);
        assert_eq!(
            status, MADOPILOT_STATUS_INVALID_ARGUMENT,
            "a {prefix} byte options structure may not claim {flag:#x}"
        );
        assert_eq!(
            refusal_category(api, error),
            MADOPILOT_ERROR_CATEGORY_ABI,
            "no search ran under an option nobody set"
        );
    }
}

#[test]
fn a_match_options_prefix_that_sets_no_bit_asks_for_the_template_defaults() {
    let api = table();
    let flow = Flow::open();

    let options = madopilot_match_options_t::cleared(8);
    let mut request = flow.find_request();
    request.options = &raw const options;

    let result = flow.find(&request);
    let effective = effective_options(api, result);
    unsafe { (api.result_release)(result) };

    let mut declared = template_info();
    assert_eq!(
        unsafe { (api.template_describe)(flow.present, &raw mut declared) },
        MADOPILOT_STATUS_OK
    );

    assert_eq!(
        effective.min_score, declared.min_score,
        "the shortest options prefix is the documented way to ask for the template's own defaults"
    );
    assert_eq!(effective.max_results, declared.max_results);
}

#[test]
fn a_match_option_bit_the_prefix_covers_is_honored() {
    let api = table();
    let flow = Flow::open();

    // Sixteen bytes is where `max_results` begins, so `min_score` is the last
    // field the prefix covers and the only bit it may set.
    let mut options = madopilot_match_options_t::cleared(16);
    options.flags = MADOPILOT_MATCH_HAS_MIN_SCORE;
    options.min_score = 0.5;

    let mut request = flow.find_request();
    request.options = &raw const options;

    let result = flow.find(&request);
    let effective = effective_options(api, result);
    unsafe { (api.result_release)(result) };

    assert!(
        (effective.min_score - 0.5).abs() < f64::EPSILON,
        "the search ran under the threshold the covered field carried"
    );
}

// --- Shared helpers ---------------------------------------------------------

/// The category a refusal reported.
///
/// A presence-bit refusal that arrives under any other category is a different
/// refusal wearing the same status, which is exactly what a missing table entry
/// looks like from outside the library.
fn refusal_category(
    api: &'static madopilot_api_t,
    error: *mut madopilot_error_t,
) -> madopilot_error_category_t {
    support::describe_and_release(api, error).category
}

/// The whole frame, as a region a request may name.
fn frame_bounds(api: &'static madopilot_api_t, flow: &Flow) -> madopilot_pixel_rect_t {
    let mut info = madopilot_frame_info_t {
        struct_size: struct_size::<madopilot_frame_info_t>(),
        flags: 0,
        width: 0,
        height: 0,
        format: MADOPILOT_PIXEL_FORMAT_RGBA8,
        space: MADOPILOT_SPACE_CAPTURE_PIXELS,
        stride: 0,
        bounds: madopilot_pixel_rect_t::empty(),
    };
    assert_eq!(
        unsafe { (api.frame_describe)(flow.frame, &raw mut info) },
        MADOPILOT_STATUS_OK
    );

    info.bounds
}

fn open_request(struct_size: u32, flags: u32) -> madopilot_open_request_t {
    madopilot_open_request_t {
        struct_size,
        flags,
        required_format: MADOPILOT_PIXEL_FORMAT_RGBA8,
        preferred_format: MADOPILOT_PIXEL_FORMAT_RGBA8,
    }
}

fn open_session(
    api: &'static madopilot_api_t,
    flow: &Flow,
    request: &madopilot_open_request_t,
) -> (madopilot_status_t, *mut madopilot_error_t) {
    let operation = operation();
    let mut session = ptr::null_mut();
    let mut error = ptr::null_mut();
    let status = unsafe {
        (api.session_open)(
            flow.engine,
            flow.targets,
            0,
            &raw const *request,
            &raw const operation,
            &raw mut session,
            &raw mut error,
        )
    };
    // Null on every refusal, and the release entry accepts null.
    unsafe { (api.session_release)(session) };

    (status, error)
}

fn map_request(
    struct_size: u32,
    flags: u32,
    region: madopilot_pixel_rect_t,
) -> madopilot_map_request_t {
    madopilot_map_request_t {
        struct_size,
        flags,
        format: MADOPILOT_PIXEL_FORMAT_RGBA8,
        clip_policy: MADOPILOT_CLIP_POLICY_REJECT,
        region,
    }
}

fn map_frame(
    api: &'static madopilot_api_t,
    flow: &Flow,
    request: &madopilot_map_request_t,
) -> (madopilot_status_t, *mut madopilot_error_t) {
    let operation = operation();
    let mut mapping = ptr::null_mut();
    let mut error = ptr::null_mut();
    let status = unsafe {
        (api.frame_map)(
            flow.frame,
            &raw const *request,
            &raw const operation,
            &raw mut mapping,
            &raw mut error,
        )
    };
    unsafe { (api.mapping_release)(mapping) };

    (status, error)
}

fn run_find(
    api: &'static madopilot_api_t,
    flow: &Flow,
    request: &madopilot_find_request_t,
) -> (madopilot_status_t, *mut madopilot_error_t) {
    let operation = operation();
    let mut result = ptr::null_mut();
    let mut error = ptr::null_mut();
    let status = unsafe {
        (api.session_find)(
            flow.session,
            &raw const *request,
            &raw const operation,
            &raw mut result,
            &raw mut error,
        )
    };
    unsafe { (api.result_release)(result) };

    (status, error)
}

fn create_with_frame(
    api: &'static madopilot_api_t,
    frame: &madopilot_replay_frame_t,
    frame_count: usize,
    frame_stride: usize,
) -> madopilot_status_t {
    let source = madopilot_source_t {
        struct_size: struct_size::<madopilot_source_t>(),
        kind: MADOPILOT_SOURCE_REPLAY_MEMORY,
        directory: madopilot_str_t::empty(),
        frames: &raw const *frame,
        frame_count,
        frame_stride,
        target_name: madopilot_str_t::empty(),
    };
    let operation = operation();
    let mut engine = ptr::null_mut();

    let status = unsafe {
        (api.engine_create)(
            &raw const source,
            &raw const operation,
            &raw mut engine,
            ptr::null_mut(),
        )
    };
    unsafe { (api.engine_release)(engine) };

    status
}

fn effective_options(
    api: &'static madopilot_api_t,
    result: *mut madopilot_result_t,
) -> madopilot_match_options_t {
    let mut options =
        madopilot_match_options_t::cleared(struct_size::<madopilot_match_options_t>());
    assert_eq!(
        unsafe { (api.result_options)(result, &raw mut options) },
        MADOPILOT_STATUS_OK
    );

    options
}

fn template_info() -> madopilot_template_info_t {
    madopilot_template_info_t {
        struct_size: struct_size::<madopilot_template_info_t>(),
        flags: 0,
        width: 0,
        height: 0,
        min_score: 0.0,
        id: madopilot_str_t::empty(),
        backend: madopilot_str_t::empty(),
        max_results: 0,
        space: MADOPILOT_SPACE_CAPTURE_PIXELS,
    }
}
