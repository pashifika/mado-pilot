//! Ownership, lifetime independence, concurrency, and the deterministic flow.
//!
//! The rule these tests exist to hold the library to is simple to state and
//! easy to break: releasing a parent never invalidates a separately retained
//! child. A caller that has to keep the engine, the session, the package, and
//! the template alive in order to read a match it already has is a caller who
//! will keep all of them alive forever.

// Every call below goes through an `unsafe extern "C"` table entry, and the
// safety argument is the same one each time: every pointer is a live local of
// the test, and every handle is retained by the value that produced it for
// longer than the call lasts. Repeating that on a hundred assertions would bury
// it rather than document it, so it is stated once here and inline only where a
// call has something extra to say.
#![allow(clippy::undocumented_unsafe_blocks)]

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{mem, ptr, thread};

use madopilot::layout::struct_size;
use madopilot::*;
use support::{Flow, Scene, operation, str_view, table};

/// A handle that a test moves to another thread.
///
/// The C contract says const access from several threads is safe while each
/// keeps a live reference; Rust cannot see that through a raw pointer, so the
/// promise is restated here where a reviewer can check it against the test.
struct Shared<T>(*mut T);

// Derived `Clone`/`Copy` would add a `T: Clone` bound the opaque handle types
// cannot satisfy, so both are written out.
impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Shared<T> {}

// SAFETY: every use below keeps the referenced handle retained for longer than
// the threads that read it, and reads it only through const accessors.
unsafe impl<T> Send for Shared<T> {}

#[test]
fn the_deterministic_flow_finds_the_planted_patch() {
    let flow = Flow::open();
    let api = flow.api;
    let request = flow.find_request();
    let result = flow.find(&request);

    let mut info = result_info();
    // SAFETY: `info` is a live local with its `struct_size` set, and the result
    // is owned by this frame.
    assert_eq!(
        unsafe { (api.result_describe)(result, &raw mut info) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(info.match_count, 2, "the fixture plants two exact copies");

    let mut found = Vec::new();
    for index in 0..usize::try_from(info.match_count).expect("a match count fits an index") {
        let mut one = match_value();
        // SAFETY: as above.
        assert_eq!(
            unsafe { (api.result_match)(result, index, &raw mut one) },
            MADOPILOT_STATUS_OK
        );
        found.push((one.bounds.left, one.bounds.top));
        assert!(one.score > 0.99, "an exact copy scores near one");
        assert_eq!(one.bounds.space, MADOPILOT_SPACE_CAPTURE_PIXELS);
    }
    found.sort_unstable();
    assert_eq!(found, vec![(20, 12), (60, 40)]);

    // SAFETY: owned by this frame.
    unsafe { (api.result_release)(result) };
}

#[test]
fn a_completed_search_with_no_match_is_a_success() {
    let flow = Flow::open();
    let api = flow.api;
    let mut request = flow.find_request();
    request.tmpl = flow.absent;
    let result = flow.find(&request);

    let mut info = result_info();
    // SAFETY: `info` is a live local and the result is owned here.
    assert_eq!(
        unsafe { (api.result_describe)(result, &raw mut info) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(info.match_count, 0);
    assert_ne!(info.backend_id.len, 0, "and it still names the backend");

    // The correlation is complete even with nothing found.
    let mut stamp = stamp();
    // SAFETY: as above.
    assert_eq!(
        unsafe { (api.result_stamp)(result, &raw mut stamp) },
        MADOPILOT_STATUS_OK
    );
    assert_ne!(stamp.stream, 0);

    // SAFETY: owned here.
    unsafe { (api.result_release)(result) };
}

#[test]
fn a_search_can_use_the_sessions_latest_frame_instead_of_an_exact_one() {
    let flow = Flow::open();
    let api = flow.api;
    let mut request = flow.find_request();
    request.frame = ptr::null();
    let result = flow.find(&request);

    let mut info = result_info();
    // SAFETY: `info` is a live local and the result is owned here.
    assert_eq!(
        unsafe { (api.result_describe)(result, &raw mut info) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(info.match_count, 2);

    // SAFETY: owned here.
    unsafe { (api.result_release)(result) };
}

#[test]
fn supplied_options_override_the_templates_defaults_and_are_reported_back() {
    let flow = Flow::open();
    let api = flow.api;

    let options = madopilot_match_options_t {
        struct_size: struct_size::<madopilot_match_options_t>(),
        flags: MADOPILOT_MATCH_HAS_MIN_SCORE | MADOPILOT_MATCH_HAS_MAX_RESULTS,
        min_score: 0.5,
        max_results: 1,
        suppression: MADOPILOT_SUPPRESSION_DROP_OVERLAPPING,
    };
    let mut request = flow.find_request();
    request.options = &raw const options;
    let result = flow.find(&request);

    let mut info = result_info();
    // SAFETY: `info` is a live local and the result is owned here.
    assert_eq!(
        unsafe { (api.result_describe)(result, &raw mut info) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(info.match_count, 1, "the limit the caller asked for");

    let mut effective =
        madopilot_match_options_t::cleared(struct_size::<madopilot_match_options_t>());
    // SAFETY: as above.
    assert_eq!(
        unsafe { (api.result_options)(result, &raw mut effective) },
        MADOPILOT_STATUS_OK
    );
    assert!((effective.min_score - 0.5).abs() < 1e-9);
    assert_eq!(effective.max_results, 1);
    assert_ne!(
        effective.flags & MADOPILOT_MATCH_HAS_SUPPRESSION,
        0,
        "every option was in effect, so every presence bit is set"
    );

    // SAFETY: owned here.
    unsafe { (api.result_release)(result) };
}

#[test]
fn a_region_narrows_the_search_to_part_of_the_frame() {
    let flow = Flow::open();
    let api = flow.api;

    let mut request = flow.find_request();
    request.flags = MADOPILOT_FIND_HAS_REGION;
    request.region = madopilot_pixel_rect_t {
        space: MADOPILOT_SPACE_CAPTURE_PIXELS,
        left: 0,
        top: 0,
        right: 48,
        bottom: 32,
    };
    let result = flow.find(&request);

    let mut info = result_info();
    // SAFETY: `info` is a live local and the result is owned here.
    assert_eq!(
        unsafe { (api.result_describe)(result, &raw mut info) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(info.match_count, 1, "only the first copy is in this corner");
    assert_eq!(info.searched.right, 48);
    assert_eq!(info.searched.bottom, 32);

    // SAFETY: owned here.
    unsafe { (api.result_release)(result) };
}

#[test]
fn a_frame_from_another_session_is_refused() {
    let flow = Flow::open();
    let api = flow.api;
    let operation = operation();

    let open = open_request();
    let mut other = ptr::null_mut();
    // SAFETY: the engine and target list are retained by the flow, and every
    // pointer is a live local.
    let status = unsafe {
        (api.session_open)(
            flow.engine,
            flow.targets,
            0,
            &raw const open,
            &raw const operation,
            &raw mut other,
            ptr::null_mut(),
        )
    };
    assert_eq!(status, MADOPILOT_STATUS_OK);

    // The frame belongs to the flow's session, not to this one. Naming this
    // session's target for content it never published is exactly what the
    // refusal prevents.
    let request = flow.find_request();
    let mut result = ptr::null_mut();
    let mut error = ptr::null_mut();
    // SAFETY: as above.
    let status = unsafe {
        (api.session_find)(
            other,
            &raw const request,
            &raw const operation,
            &raw mut result,
            &raw mut error,
        )
    };
    assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT);
    assert!(result.is_null());
    let detail = support::describe_and_release(api, error);
    assert_eq!(detail.status, MADOPILOT_STATUS_INVALID_ARGUMENT);

    // SAFETY: owned here.
    unsafe {
        (api.session_close)(other, &raw const operation, ptr::null_mut());
        (api.session_release)(other);
    }
}

#[test]
fn a_session_outlives_the_target_list_it_was_opened_from() {
    let api = table();
    let scene = Scene::new();
    let operation = operation();

    let mut engine = ptr::null_mut();
    // SAFETY: every pointer is a live local that outlives the call.
    unsafe {
        assert_eq!(
            (api.engine_create)(
                scene.source(),
                &raw const operation,
                &raw mut engine,
                ptr::null_mut()
            ),
            MADOPILOT_STATUS_OK
        );
    }

    let mut targets = ptr::null_mut();
    // SAFETY: as above.
    unsafe {
        assert_eq!(
            (api.engine_discover)(
                engine,
                &raw const operation,
                &raw mut targets,
                ptr::null_mut()
            ),
            MADOPILOT_STATUS_OK
        );
    }

    let open = open_request();
    let mut session = ptr::null_mut();
    // SAFETY: as above.
    unsafe {
        assert_eq!(
            (api.session_open)(
                engine,
                targets,
                0,
                &raw const open,
                &raw const operation,
                &raw mut session,
                ptr::null_mut()
            ),
            MADOPILOT_STATUS_OK
        );
        // Opening copied the identity, so this is the caller's to drop.
        (api.target_list_release)(targets);
    }

    let mut frame = ptr::null_mut();
    // SAFETY: the session is retained here and outlives the call.
    let status = unsafe {
        (api.session_acquire_frame)(
            session,
            &raw const operation,
            &raw mut frame,
            ptr::null_mut(),
        )
    };
    assert_eq!(
        status, MADOPILOT_STATUS_OK,
        "the session retained every dependency capture needs"
    );

    // SAFETY: each handle is owned here.
    unsafe {
        (api.frame_release)(frame);
        (api.session_close)(session, &raw const operation, ptr::null_mut());
        (api.session_release)(session);
        (api.engine_release)(engine);
    }
}

#[test]
fn a_mapping_outlives_the_frame_and_the_closed_session() {
    let mut flow = Flow::open();
    let api = flow.api;
    let operation = operation();

    let map = madopilot_map_request_t {
        struct_size: struct_size::<madopilot_map_request_t>(),
        flags: 0,
        format: MADOPILOT_PIXEL_FORMAT_RGBA8,
        clip_policy: MADOPILOT_CLIP_POLICY_REJECT,
        region: madopilot_pixel_rect_t::empty(),
    };
    let mut mapping = ptr::null_mut();
    // SAFETY: the frame is retained by the flow and every pointer is a live
    // local.
    let status = unsafe {
        (api.frame_map)(
            flow.frame,
            &raw const map,
            &raw const operation,
            &raw mut mapping,
            ptr::null_mut(),
        )
    };
    assert_eq!(status, MADOPILOT_STATUS_OK);

    let mut before = image();
    // SAFETY: `before` is a live local and the mapping is owned here.
    assert_eq!(
        unsafe { (api.mapping_describe)(mapping, &raw mut before) },
        MADOPILOT_STATUS_OK
    );
    // SAFETY: the byte view borrows storage the mapping keeps alive.
    let snapshot =
        unsafe { std::slice::from_raw_parts(before.bytes.data, before.bytes.len) }.to_vec();
    assert_ne!(
        before.flags & MADOPILOT_IMAGE_SHARED,
        0,
        "this mapping shares the frame's storage rather than copying it, which is what makes \
         releasing the frame below a real question"
    );

    // Release the producer's whole chain out from under the mapping. The frame
    // handle is moved out of the flow first, so the release below is its last
    // one and the frame's count really does reach zero; a retain-then-release
    // pair here would be a no-op and would leave the property in this test's
    // name unexercised. Dropping the flow afterwards releases the closed
    // session, the target list, the package, both templates, and the engine.
    let frame = mem::replace(&mut flow.frame, ptr::null_mut());
    // SAFETY: the flow no longer holds this handle, so this is the reference it
    // would otherwise have released at drop, and the release entries accept the
    // null the flow now carries in its place.
    unsafe {
        (api.frame_release)(frame);
        (api.session_close)(flow.session, &raw const operation, ptr::null_mut());
    }
    drop(flow);

    let mut after = image();
    // SAFETY: as above.
    assert_eq!(
        unsafe { (api.mapping_describe)(mapping, &raw mut after) },
        MADOPILOT_STATUS_OK
    );
    // SAFETY: the byte view is still borrowed from the retained mapping.
    let now = unsafe { std::slice::from_raw_parts(after.bytes.data, after.bytes.len) };
    assert_eq!(now, snapshot.as_slice(), "the bytes are unchanged");

    // SAFETY: owned here; this is the mapping's final release.
    unsafe { (api.mapping_release)(mapping) };
}

#[test]
fn a_result_outlives_the_session_package_template_and_engine() {
    let api = table();
    let result;
    let expected;

    {
        let flow = Flow::open();
        let request = flow.find_request();
        result = flow.find(&request);

        let mut info = result_info();
        // SAFETY: `info` is a live local and the result is owned here.
        unsafe { (api.result_describe)(result, &raw mut info) };
        expected = info.match_count;

        // Dropping the flow releases the template, the package, the frame, the
        // session, the target list, and the engine, in that order.
    }

    let mut info = result_info();
    // SAFETY: the result is still retained by this frame.
    assert_eq!(
        unsafe { (api.result_describe)(result, &raw mut info) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(info.match_count, expected);
    assert_ne!(
        info.backend_id.len, 0,
        "the backend view is owned by the result, not by the engine"
    );

    let mut one = match_value();
    // SAFETY: as above.
    assert_eq!(
        unsafe { (api.result_match)(result, 0, &raw mut one) },
        MADOPILOT_STATUS_OK
    );
    assert_ne!(
        one.template_id.len, 0,
        "the identity view is owned by the result, not by the template"
    );

    // SAFETY: owned here.
    unsafe { (api.result_release)(result) };
}

#[test]
fn a_retained_reference_survives_its_sibling_being_released() {
    let flow = Flow::open();
    let api = flow.api;
    let request = flow.find_request();
    let result = flow.find(&request);

    // SAFETY: the result is owned here; this adds a second owned reference.
    unsafe { (api.result_retain)(result) };
    // SAFETY: and this gives one of the two back.
    unsafe { (api.result_release)(result) };

    let mut info = result_info();
    // SAFETY: one owned reference remains, so the result is still alive.
    assert_eq!(
        unsafe { (api.result_describe)(result, &raw mut info) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(info.match_count, 2);

    // SAFETY: the final release destroys the result exactly once.
    unsafe { (api.result_release)(result) };
}

// --- The retain half of every handle's lifecycle ----------------------------
//
// `result_retain` is covered above. The five below, and `mapping_stamp`, were
// reachable from the header and from nowhere else: no test, example, C program,
// or C++ probe called them. Each is a hand-written one-line wrapper generic over
// its payload type, so a `handle::release` written into a `_retain`, or a
// sibling payload type named in one, compiles and ships. Each test takes the
// extra reference, gives another one back, and only then reads the payload —
// a wrapper that did not increment leaves that read looking at a dropped value.

#[test]
fn a_cancellation_survives_the_reference_it_was_created_with() {
    let api = table();

    let mut cancellation = ptr::null_mut();
    // SAFETY: `cancellation` is a live local.
    assert_eq!(
        unsafe { (api.cancellation_create)(&raw mut cancellation) },
        MADOPILOT_STATUS_OK
    );

    // SAFETY: the handle is retained by this frame; this takes a second
    // reference and gives the first back.
    unsafe {
        assert_eq!((api.cancellation_retain)(cancellation), MADOPILOT_STATUS_OK);
        assert_eq!(
            (api.cancellation_release)(cancellation),
            MADOPILOT_STATUS_OK
        );
    }

    let mut cancelled = 1_i32;
    // SAFETY: the reference `cancellation_retain` took is still live.
    unsafe {
        assert_eq!(
            (api.cancellation_is_cancelled)(cancellation, &raw mut cancelled),
            MADOPILOT_STATUS_OK
        );
    }
    assert_eq!(cancelled, 0, "the token is the one that was created");

    // The payload is not just readable but still the same shared state.
    // SAFETY: as above.
    unsafe {
        assert_eq!((api.cancellation_cancel)(cancellation), MADOPILOT_STATUS_OK);
        assert_eq!(
            (api.cancellation_is_cancelled)(cancellation, &raw mut cancelled),
            MADOPILOT_STATUS_OK
        );
    }
    assert_eq!(cancelled, 1);

    // SAFETY: the last reference this frame holds.
    unsafe {
        assert_eq!(
            (api.cancellation_release)(cancellation),
            MADOPILOT_STATUS_OK
        );
    }
}

#[test]
fn an_error_survives_the_reference_the_refusal_returned() {
    let api = table();
    let error = support::refused_error(api);

    // SAFETY: the handle is owned by this frame; this takes a second reference
    // and gives the first back.
    unsafe {
        assert_eq!((api.error_retain)(error), MADOPILOT_STATUS_OK);
        assert_eq!((api.error_release)(error), MADOPILOT_STATUS_OK);
    }

    // Reads through the retained reference, and releases it.
    let detail = support::describe_and_release(api, error);
    assert_eq!(detail.status, MADOPILOT_STATUS_INVALID_ARGUMENT);
    assert_eq!(detail.category, MADOPILOT_ERROR_CATEGORY_ABI);
}

#[test]
fn a_target_list_survives_the_reference_discovery_returned() {
    let mut flow = Flow::open();
    let api = flow.api;

    let mut before = 0_usize;
    // SAFETY: the list is retained by the flow and `before` is a live local.
    assert_eq!(
        unsafe { (api.target_list_count)(flow.targets, &raw mut before) },
        MADOPILOT_STATUS_OK
    );
    assert_ne!(before, 0, "the replay source declares a target");

    // SAFETY: the list is retained by the flow; this takes this frame's own
    // reference.
    unsafe { assert_eq!((api.target_list_retain)(flow.targets), MADOPILOT_STATUS_OK) };

    // The flow's reference is moved out and given back, so the only one left is
    // the reference `target_list_retain` took.
    let targets = mem::replace(&mut flow.targets, ptr::null_mut());
    // SAFETY: the flow no longer holds this handle.
    unsafe { assert_eq!((api.target_list_release)(targets), MADOPILOT_STATUS_OK) };

    let mut after = 0_usize;
    // SAFETY: this frame's reference is still live.
    assert_eq!(
        unsafe { (api.target_list_count)(targets, &raw mut after) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(after, before, "the list is the one discovery produced");

    // SAFETY: the last reference.
    unsafe { assert_eq!((api.target_list_release)(targets), MADOPILOT_STATUS_OK) };
}

#[test]
fn a_session_survives_the_reference_open_returned() {
    let mut flow = Flow::open();
    let api = flow.api;
    let operation = operation();

    let mut before = session_info();
    // SAFETY: the session is retained by the flow and `before` is a live local.
    assert_eq!(
        unsafe { (api.session_describe)(flow.session, &raw mut before) },
        MADOPILOT_STATUS_OK
    );
    assert_ne!(before.stream, 0);

    // SAFETY: the session is retained by the flow; this takes this frame's own
    // reference.
    unsafe { assert_eq!((api.session_retain)(flow.session), MADOPILOT_STATUS_OK) };

    let session = mem::replace(&mut flow.session, ptr::null_mut());
    // SAFETY: the flow no longer holds this handle.
    unsafe { assert_eq!((api.session_release)(session), MADOPILOT_STATUS_OK) };

    let mut after = session_info();
    // SAFETY: this frame's reference is still live.
    assert_eq!(
        unsafe { (api.session_describe)(session, &raw mut after) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(
        after.stream, before.stream,
        "the session is the one that was opened"
    );

    // SAFETY: the last reference; releasing a session does not close it, so the
    // close comes first.
    unsafe {
        (api.session_close)(session, &raw const operation, ptr::null_mut());
        assert_eq!((api.session_release)(session), MADOPILOT_STATUS_OK);
    }
}

#[test]
fn a_mapping_survives_the_reference_frame_map_returned() {
    let flow = Flow::open();
    let api = flow.api;
    let mapping = flow.map();

    let mut before = image();
    // SAFETY: the mapping is owned by this frame and `before` is a live local.
    assert_eq!(
        unsafe { (api.mapping_describe)(mapping, &raw mut before) },
        MADOPILOT_STATUS_OK
    );
    assert_ne!(before.bytes.len, 0);

    // SAFETY: the handle is owned by this frame; this takes a second reference
    // and gives the first back.
    unsafe {
        assert_eq!((api.mapping_retain)(mapping), MADOPILOT_STATUS_OK);
        assert_eq!((api.mapping_release)(mapping), MADOPILOT_STATUS_OK);
    }

    let mut after = image();
    // SAFETY: the reference `mapping_retain` took is still live.
    assert_eq!(
        unsafe { (api.mapping_describe)(mapping, &raw mut after) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!((after.width, after.height), (before.width, before.height));
    assert_eq!(after.bytes.len, before.bytes.len);
    assert_eq!(
        after.bytes.data, before.bytes.data,
        "the mapping is the one that was mapped, not a rebuilt one"
    );

    // SAFETY: the last reference.
    unsafe { assert_eq!((api.mapping_release)(mapping), MADOPILOT_STATUS_OK) };
}

#[test]
fn a_mapping_reports_the_identity_of_the_frame_it_came_from() {
    let flow = Flow::open();
    let api = flow.api;

    let mut expected = stamp();
    // SAFETY: the frame is retained by the flow and `expected` is a live local.
    assert_eq!(
        unsafe { (api.frame_stamp)(flow.frame, &raw mut expected) },
        MADOPILOT_STATUS_OK
    );
    assert_ne!(expected.stream, 0, "a frame carries a real stream identity");

    let mapping = flow.map();
    let mut measured = stamp();
    // SAFETY: the mapping is owned by this frame and `measured` is a live local.
    assert_eq!(
        unsafe { (api.mapping_stamp)(mapping, &raw mut measured) },
        MADOPILOT_STATUS_OK
    );

    // The whole correlation, not just the parts that happen to be non-zero: a
    // mapping whose stamp came from somewhere else would still report a
    // plausible-looking one.
    assert_eq!(measured.struct_size, expected.struct_size);
    assert_eq!(measured.flags, expected.flags);
    assert_eq!(measured.stream, expected.stream);
    assert_eq!(measured.epoch, expected.epoch);
    assert_eq!(measured.sequence, expected.sequence);
    assert_eq!(measured.geometry, expected.geometry);

    // SAFETY: owned here.
    unsafe { (api.mapping_release)(mapping) };
}

#[test]
fn an_immutable_result_is_read_concurrently_from_several_threads() {
    let flow = Flow::open();
    let api = flow.api;
    let request = flow.find_request();
    let result = flow.find(&request);

    let shared = Shared(result);
    let mut workers = Vec::new();
    for _ in 0..8 {
        workers.push(thread::spawn(move || {
            let handle = shared;
            for _ in 0..64 {
                // Each thread takes its own reference before reading, which is
                // the contract, and gives it back afterwards.
                // SAFETY: the spawning frame keeps a reference alive for longer
                // than every thread, so the count never reaches zero here.
                unsafe { (api.result_retain)(handle.0) };

                let mut info = result_info();
                // SAFETY: this thread holds its own reference.
                assert_eq!(
                    unsafe { (api.result_describe)(handle.0, &raw mut info) },
                    MADOPILOT_STATUS_OK
                );
                assert_eq!(info.match_count, 2, "the result is immutable");

                // SAFETY: giving back this thread's own reference.
                unsafe { (api.result_release)(handle.0) };
            }
        }));
    }
    for worker in workers {
        worker.join().expect("no thread observed a data race");
    }

    // SAFETY: the original reference is still this frame's to release.
    unsafe { (api.result_release)(result) };
}

#[test]
fn close_is_idempotent_and_refuses_later_work() {
    let flow = Flow::open();
    let api = flow.api;
    let operation = operation();

    // SAFETY: the session is retained by the flow.
    unsafe {
        assert_eq!(
            (api.session_close)(flow.session, &raw const operation, ptr::null_mut()),
            MADOPILOT_STATUS_OK
        );
        assert_eq!(
            (api.session_close)(flow.session, &raw const operation, ptr::null_mut()),
            MADOPILOT_STATUS_OK,
            "close is idempotent"
        );
    }

    let mut closed = 0_i32;
    // SAFETY: `closed` is a live local.
    unsafe { (api.session_is_closed)(flow.session, &raw mut closed) };
    assert_eq!(closed, 1);

    let mut frame = ptr::null_mut();
    let mut error = ptr::null_mut();
    // SAFETY: as above.
    let status = unsafe {
        (api.session_acquire_frame)(
            flow.session,
            &raw const operation,
            &raw mut frame,
            &raw mut error,
        )
    };
    assert_eq!(status, MADOPILOT_STATUS_CLOSED);
    assert!(frame.is_null(), "and produces no frame output");
    let detail = support::describe_and_release(api, error);
    assert_eq!(detail.status, MADOPILOT_STATUS_CLOSED);

    let request = flow.find_request();
    let mut result = ptr::null_mut();
    // SAFETY: as above.
    let status = unsafe {
        (api.session_find)(
            flow.session,
            &raw const request,
            &raw const operation,
            &raw mut result,
            ptr::null_mut(),
        )
    };
    assert_eq!(status, MADOPILOT_STATUS_CLOSED);
    assert!(result.is_null());
}

#[test]
fn close_racing_an_in_flight_search_has_exactly_one_terminal_outcome() {
    let flow = Flow::open();
    let api = flow.api;
    let request = flow.find_request();

    let session = Shared(flow.session);
    let searching = Arc::new(AtomicBool::new(true));

    let closer = {
        let searching = Arc::clone(&searching);
        thread::spawn(move || {
            let handle = session;
            let operation = operation();
            while searching.load(Ordering::Acquire) {
                // SAFETY: the spawning frame keeps the session retained for
                // longer than this thread runs.
                unsafe { (api.session_close)(handle.0, &raw const operation, ptr::null_mut()) };
            }
        })
    };

    // The outcomes are recorded in order rather than only counted. A count is
    // decided by the loop: with one arm per outcome and a `panic!` for anything
    // else, `succeeded + closed == 64` holds after 64 iterations whatever the
    // library did, so it could not fail. What the loop does not decide is the
    // ORDER, and close is terminal, so every success must precede every
    // refusal. A session that accepted work again after being closed would
    // break that and nothing here would have noticed before.
    let mut refused_at = None;
    let mut succeeded = 0usize;
    for attempt in 0..64 {
        let operation = operation();
        let mut result = ptr::null_mut();
        // SAFETY: every handle the request names is retained by the flow.
        let status = unsafe {
            (api.session_find)(
                flow.session,
                &raw const request,
                &raw const operation,
                &raw mut result,
                ptr::null_mut(),
            )
        };
        match status {
            MADOPILOT_STATUS_OK => {
                succeeded += 1;
                assert!(!result.is_null());
                assert_eq!(
                    refused_at, None,
                    "attempt {attempt} succeeded after attempt {refused_at:?} was refused; \
                     close is terminal"
                );
                // SAFETY: owned here.
                unsafe { (api.result_release)(result) };
            }
            MADOPILOT_STATUS_CLOSED | MADOPILOT_STATUS_CANCELLED => {
                assert!(result.is_null(), "a refused search produces no result");
                refused_at = refused_at.or(Some(attempt));
            }
            other => panic!("unexpected terminal outcome {other}"),
        }
    }
    searching.store(false, Ordering::Release);
    closer.join().expect("close is safe to call repeatedly");

    // Whether the closer won a race is deliberately not asserted. It cannot be
    // forced: the thread might not be scheduled before the loop sets the flag,
    // and requiring a win would make this test fail for a reason that is not a
    // defect. `refused_at` and `succeeded` are therefore evidence for the
    // ordering check above rather than a claim of their own — but the terminal
    // behaviour below IS deterministic, because it closes the session itself.
    let _ = (refused_at, succeeded);

    let after = operation();
    // SAFETY: the session is retained by the flow.
    let closed = unsafe { (api.session_close)(flow.session, &raw const after, ptr::null_mut()) };
    assert_eq!(
        closed, MADOPILOT_STATUS_OK,
        "close is idempotent, however many times the racing thread already ran"
    );

    // Close is terminal, so this state is permanent.
    let mut result = ptr::null_mut();
    // SAFETY: as in the loop.
    let status = unsafe {
        (api.session_find)(
            flow.session,
            &raw const request,
            &raw const after,
            &raw mut result,
            ptr::null_mut(),
        )
    };
    assert!(
        status == MADOPILOT_STATUS_CLOSED || status == MADOPILOT_STATUS_CANCELLED,
        "a closed session stays closed, got {status}"
    );
    assert!(result.is_null(), "a refused search produces no result");
}

/// A second session on the same engine gets its own stream identity.
///
/// Named for what it opens. It was called `a_second_engine_...`, which is a
/// stronger claim about a different thing — two engines never sharing a stream
/// number — and this test opens one engine. The assertion below is about a
/// stream identity never being reused while the library is loaded, and two
/// sessions on one engine is the case that exercises it.
#[test]
fn a_second_session_mints_a_distinct_stream_identity() {
    let flow = Flow::open();
    let api = flow.api;
    let operation = operation();

    let open = open_request();
    let mut other = ptr::null_mut();
    // SAFETY: the engine and target list are retained by the flow.
    unsafe {
        assert_eq!(
            (api.session_open)(
                flow.engine,
                flow.targets,
                0,
                &raw const open,
                &raw const operation,
                &raw mut other,
                ptr::null_mut()
            ),
            MADOPILOT_STATUS_OK
        );
    }

    let mut first = session_info();
    let mut second = session_info();
    // SAFETY: both handles are retained and both outputs are live locals.
    unsafe {
        (api.session_describe)(flow.session, &raw mut first);
        (api.session_describe)(other, &raw mut second);
    }
    assert_ne!(
        first.stream, second.stream,
        "a stream identity is never reused while the library is loaded"
    );

    // SAFETY: owned here.
    unsafe {
        (api.session_close)(other, &raw const operation, ptr::null_mut());
        (api.session_release)(other);
    }
}

#[test]
fn the_package_reports_the_identities_it_declares() {
    let flow = Flow::open();
    let api = flow.api;

    let mut info = package_info();
    // SAFETY: the package is retained by the flow.
    assert_eq!(
        unsafe { (api.package_describe)(flow.package, &raw mut info) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(info.template_count, 2);

    let mut declared = Vec::new();
    let declared_count =
        usize::try_from(info.template_count).expect("a template count fits an index");
    for index in 0..declared_count {
        let mut id = madopilot_str_t::empty();
        // SAFETY: as above.
        assert_eq!(
            unsafe { (api.package_template_id)(flow.package, index, &raw mut id) },
            MADOPILOT_STATUS_OK
        );
        // SAFETY: the view borrows from the package the flow keeps retained.
        let bytes = unsafe { std::slice::from_raw_parts(id.data.cast::<u8>(), id.len) };
        declared.push(String::from_utf8(bytes.to_vec()).expect("an identity is UTF-8"));
    }
    declared.sort();
    assert_eq!(declared, vec!["panel.absent", "panel.patch"]);

    let mut id = madopilot_str_t::empty();
    // SAFETY: as above.
    let status = unsafe { (api.package_template_id)(flow.package, declared_count, &raw mut id) };
    assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT);
    assert_eq!(id.len, 0, "a rejected accessor clears its output");
}

#[test]
fn a_template_reports_what_it_was_compiled_into() {
    let flow = Flow::open();
    let api = flow.api;

    let mut info = template_info();
    // SAFETY: the template is retained by the flow.
    assert_eq!(
        unsafe { (api.template_describe)(flow.present, &raw mut info) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!((info.width, info.height), (12, 10));
    assert!((info.min_score - 0.9).abs() < 1e-9);
    assert_eq!(info.max_results, 8);
    // SAFETY: the view borrows from the template the flow keeps retained.
    let id = unsafe { std::slice::from_raw_parts(info.id.data.cast::<u8>(), info.id.len) };
    assert_eq!(id, b"panel.patch");
}

#[test]
fn a_template_outlives_the_package_it_came_from() {
    let api = table();
    let mut flow = Flow::open();

    // SAFETY: both handles are retained by the flow; this takes an extra
    // reference to the template so it can outlive the package release below.
    unsafe { (api.template_retain)(flow.present) };
    let template = flow.present;

    // The package handle is moved out of the flow, so the release below is its
    // last one and the package's count reaches zero. A template that borrowed
    // the package's decoded bytes rather than owning what it compiled reads
    // freed memory from here on; a retain-then-release pair would leave the
    // package alive and prove nothing.
    let package = mem::replace(&mut flow.package, ptr::null_mut());
    // SAFETY: the flow no longer holds this handle, so this is the reference it
    // would otherwise have released at drop.
    unsafe { (api.package_release)(package) };

    let mut info = template_info();
    // SAFETY: the extra template reference taken above is still live.
    assert_eq!(
        unsafe { (api.template_describe)(template, &raw mut info) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!((info.width, info.height), (12, 10));

    // SAFETY: giving back the extra reference.
    unsafe { (api.template_release)(template) };
}

// --- Output structures the tests fill in ------------------------------------

fn open_request() -> madopilot_open_request_t {
    madopilot_open_request_t {
        struct_size: struct_size::<madopilot_open_request_t>(),
        flags: 0,
        required_format: MADOPILOT_PIXEL_FORMAT_RGBA8,
        preferred_format: MADOPILOT_PIXEL_FORMAT_RGBA8,
    }
}

fn result_info() -> madopilot_result_info_t {
    madopilot_result_info_t {
        struct_size: struct_size::<madopilot_result_info_t>(),
        flags: 0,
        match_count: 0,
        backend_id: madopilot_str_t::empty(),
        backend_version: madopilot_str_t::empty(),
        searched: madopilot_pixel_rect_t::empty(),
    }
}

fn match_value() -> madopilot_match_t {
    madopilot_match_t {
        struct_size: struct_size::<madopilot_match_t>(),
        flags: 0,
        score: 0.0,
        template_id: madopilot_str_t::empty(),
        bounds: madopilot_pixel_rect_t::empty(),
    }
}

fn stamp() -> madopilot_frame_stamp_t {
    madopilot_frame_stamp_t::cleared(struct_size::<madopilot_frame_stamp_t>())
}

fn image() -> madopilot_image_t {
    madopilot_image_t {
        struct_size: struct_size::<madopilot_image_t>(),
        flags: 0,
        width: 0,
        height: 0,
        format: MADOPILOT_PIXEL_FORMAT_RGBA8,
        space: MADOPILOT_SPACE_CAPTURE_PIXELS,
        stride: 0,
        bytes: madopilot_bytes_t::empty(),
        region: madopilot_pixel_rect_t::empty(),
    }
}

fn session_info() -> madopilot_session_info_t {
    madopilot_session_info_t {
        struct_size: struct_size::<madopilot_session_info_t>(),
        flags: 0,
        stream: 0,
        width: 0,
        height: 0,
        format: MADOPILOT_PIXEL_FORMAT_RGBA8,
        coordinate_spaces: 0,
    }
}

fn package_info() -> madopilot_package_info_t {
    madopilot_package_info_t {
        struct_size: struct_size::<madopilot_package_info_t>(),
        flags: 0,
        template_count: 0,
        package_id: madopilot_str_t::empty(),
        package_version: madopilot_str_t::empty(),
        license: madopilot_str_t::empty(),
    }
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

#[test]
fn the_support_helper_borrows_a_string_without_copying() {
    let value = String::from("panel.patch");
    let view = str_view(&value);
    assert_eq!(view.len, value.len());
    assert_eq!(view.data.cast::<u8>(), value.as_ptr());
}
