//! Public-table request validation and independent output/lifetime contracts.
//!
//! Every pointer below names live caller storage or a retained table-produced
//! handle. Deliberately misaligned outputs point into live aligned buffers and
//! must be rejected without access. Controlled scheduling/races live in the
//! crate's internal watcher tests rather than being guessed from replay timing.
#![allow(clippy::undocumented_unsafe_blocks)]

mod support;

use std::ptr;

use madopilot::layout::struct_size;
use madopilot::*;
use support::{Flow, describe_and_release, expired_operation, operation, table};

fn options() -> madopilot_template_watch_options_t {
    madopilot_template_watch_options_t::cleared(MADOPILOT_TEMPLATE_WATCH_OPTIONS_SIZE_V1_6)
}

fn deferred_options() -> madopilot_template_watch_options_t {
    madopilot_template_watch_options_t {
        minimum_interval_nanos: 3_600_000_000_000,
        change_policy: MADOPILOT_TEMPLATE_CHANGE_ANALYSIS_ALWAYS,
        ..options()
    }
}

fn bounded_wait() -> madopilot_operation_t {
    let mut now = 0;
    assert_eq!(
        unsafe { (table().clock_now)(&raw mut now) },
        MADOPILOT_STATUS_OK
    );
    madopilot_operation_t {
        flags: MADOPILOT_OPERATION_HAS_DEADLINE,
        deadline_nanos: now
            .checked_add(5_000_000_000)
            .expect("test wait deadline fits"),
        ..operation()
    }
}

struct Query(*mut madopilot_template_query_t, Option<ReplaySession>);

impl Query {
    fn start(
        flow: &Flow,
        template: *const madopilot_template_t,
        options: &madopilot_template_watch_options_t,
        operation: &madopilot_operation_t,
    ) -> Self {
        Self::on_session(flow.session, template, options, operation)
    }

    fn on_session(
        session: *const madopilot_session_t,
        template: *const madopilot_template_t,
        options: &madopilot_template_watch_options_t,
        operation: &madopilot_operation_t,
    ) -> Self {
        let mut query = ptr::null_mut();
        assert_eq!(
            unsafe {
                (table().session_start_template_watch)(
                    session,
                    template,
                    options,
                    operation,
                    &raw mut query,
                    ptr::null_mut(),
                )
            },
            MADOPILOT_STATUS_OK,
            "valid watcher starts",
        );
        assert!(!query.is_null());
        Self(query, None)
    }

    fn maintained(flow: &Flow, options: &madopilot_template_watch_options_t) -> Self {
        let session = ReplaySession::new(flow);
        let mut lifetime = bounded_wait();
        lifetime.deadline_nanos += 55_000_000_000;
        let mut query = Self::on_session(session.session, flow.absent, options, &lifetime);
        query.1 = Some(session);
        query
    }

    fn wait(&self) -> Terminal {
        let mut result = ptr::null_mut();
        assert_eq!(
            unsafe {
                (table().template_query_wait)(
                    self.0,
                    &bounded_wait(),
                    &raw mut result,
                    ptr::null_mut(),
                )
            },
            MADOPILOT_STATUS_OK,
            "replay query reaches terminal before the safety deadline",
        );
        assert!(!result.is_null());
        Terminal(result)
    }

    fn cancel(&self) -> Terminal {
        let mut result = ptr::null_mut();
        assert_eq!(
            unsafe { (table().template_query_cancel)(self.0, &raw mut result, ptr::null_mut()) },
            MADOPILOT_STATUS_OK,
        );
        assert!(!result.is_null());
        Terminal(result)
    }

    fn pending(&self) -> madopilot_template_query_snapshot_t {
        let mut snapshot = madopilot_template_query_snapshot_t::cleared(
            MADOPILOT_TEMPLATE_QUERY_SNAPSHOT_SIZE_V1_6,
        );
        let mut result = ptr::null_mut();
        assert_eq!(
            unsafe {
                (table().template_query_poll)(
                    self.0,
                    &raw mut snapshot,
                    &raw mut result,
                    ptr::null_mut(),
                )
            },
            MADOPILOT_STATUS_OK,
        );
        assert!(result.is_null(), "an absent template remains pending");
        assert_eq!(snapshot.state, MADOPILOT_TEMPLATE_QUERY_STATE_PENDING);
        assert_ne!(snapshot.query_id, 0);
        assert!(snapshot.pending_count <= 1);
        assert!(snapshot.in_flight_count <= 1);
        snapshot
    }
}

impl Drop for Query {
    fn drop(&mut self) {
        unsafe { (table().template_query_release)(self.0) };
    }
}

struct Terminal(*mut madopilot_template_query_result_t);

impl Terminal {
    fn info(&self) -> madopilot_template_query_result_info_t {
        let mut info = madopilot_template_query_result_info_t::cleared(
            MADOPILOT_TEMPLATE_QUERY_RESULT_INFO_SIZE_V1_6,
        );
        assert_eq!(
            unsafe { (table().template_query_result_info)(self.0, &raw mut info) },
            MADOPILOT_STATUS_OK,
        );
        info
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        unsafe { (table().template_query_result_release)(self.0) };
    }
}

// An absent one-frame replay may exhaust and close. A second identical frame,
// AnalysisAlways, and a long rate interval keep real work deferred instead.
struct ReplaySession {
    engine: *mut madopilot_engine_t,
    session: *mut madopilot_session_t,
}

impl ReplaySession {
    fn new(flow: &Flow) -> Self {
        let first = flow.scene().frame_input();
        let frames = [
            first,
            madopilot_replay_frame_t {
                captured_at_nanos: 1,
                ..first
            },
        ];
        let source = madopilot_source_t {
            frames: frames.as_ptr(),
            frame_count: frames.len(),
            ..flow.scene().source_input()
        };
        let mut engine = ptr::null_mut();
        assert_eq!(
            unsafe {
                (flow.api.engine_create)(&source, &operation(), &raw mut engine, ptr::null_mut())
            },
            MADOPILOT_STATUS_OK
        );
        let mut owned = Self {
            engine,
            session: ptr::null_mut(),
        };
        let mut targets = ptr::null_mut();
        assert_eq!(
            unsafe {
                (flow.api.engine_discover)(engine, &operation(), &raw mut targets, ptr::null_mut())
            },
            MADOPILOT_STATUS_OK
        );
        let request = madopilot_open_request_t {
            struct_size: struct_size::<madopilot_open_request_t>(),
            flags: 0,
            required_format: MADOPILOT_PIXEL_FORMAT_RGBA8,
            preferred_format: MADOPILOT_PIXEL_FORMAT_RGBA8,
        };
        let status = unsafe {
            (flow.api.session_open)(
                engine,
                targets,
                0,
                &request,
                &operation(),
                &raw mut owned.session,
                ptr::null_mut(),
            )
        };
        unsafe { (flow.api.target_list_release)(targets) };
        assert_eq!(status, MADOPILOT_STATUS_OK);
        owned
    }
}

impl Drop for ReplaySession {
    fn drop(&mut self) {
        unsafe {
            (table().session_release)(self.session);
            (table().engine_release)(self.engine);
        }
    }
}

fn refuse_start(
    flow: &Flow,
    session: *const madopilot_session_t,
    template: *const madopilot_template_t,
    options: *const madopilot_template_watch_options_t,
    operation: *const madopilot_operation_t,
    status: madopilot_status_t,
    category: madopilot_error_category_t,
) {
    let mut query = ptr::dangling_mut();
    let mut error = ptr::null_mut();
    assert_eq!(
        unsafe {
            (flow.api.session_start_template_watch)(
                session,
                template,
                options,
                operation,
                &raw mut query,
                &raw mut error,
            )
        },
        status,
    );
    assert!(query.is_null(), "a refused start publishes no query");
    let detail = describe_and_release(flow.api, error);
    assert_eq!(detail.status, status);
    assert_eq!(detail.category, category);
    assert_eq!(detail.flags, 0, "no backend/asset provenance is invented");
}

#[test]
fn watch_options_require_the_complete_initial_prefix_but_ignore_a_future_tail() {
    let flow = Flow::open();
    let short = madopilot_template_watch_options_t {
        // This is a real field boundary, but omits the mandatory reserved word.
        struct_size: 68,
        ..options()
    };
    refuse_start(
        &flow,
        flow.session,
        flow.present,
        &short,
        &operation(),
        MADOPILOT_STATUS_INVALID_ARGUMENT,
        MADOPILOT_ERROR_CATEGORY_ABI,
    );

    #[repr(C)]
    struct Extended {
        options: madopilot_template_watch_options_t,
        tail: [u8; 16],
    }
    let extended = Extended {
        options: madopilot_template_watch_options_t {
            struct_size: struct_size::<Extended>(),
            ..options()
        },
        tail: [0xa5; 16],
    };
    let query = Query::start(&flow, flow.present, &extended.options, &operation());
    assert_eq!(
        query.wait().info().outcome,
        MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED
    );
    assert_eq!(extended.tail, [0xa5; 16]);
}

#[test]
fn unknown_tags_reserved_bits_and_inactive_stability_values_are_refused() {
    let flow = Flow::open();
    let cases = [
        (
            "unknown presence bit",
            madopilot_template_watch_options_t {
                flags: 1 << 31,
                ..options()
            },
        ),
        (
            "reserved word",
            madopilot_template_watch_options_t {
                reserved: 1,
                ..options()
            },
        ),
        (
            "unknown stability",
            madopilot_template_watch_options_t {
                stability_kind: -1,
                ..options()
            },
        ),
        (
            "unknown change policy",
            madopilot_template_watch_options_t {
                change_policy: -1,
                ..options()
            },
        ),
        (
            "immediate count",
            madopilot_template_watch_options_t {
                stability_observations: 1,
                ..options()
            },
        ),
        (
            "immediate duration",
            madopilot_template_watch_options_t {
                stability_duration_nanos: 1,
                ..options()
            },
        ),
        (
            "zero consecutive count",
            madopilot_template_watch_options_t {
                stability_kind: MADOPILOT_TEMPLATE_STABILITY_CONSECUTIVE,
                ..options()
            },
        ),
        (
            "consecutive duration",
            madopilot_template_watch_options_t {
                stability_kind: MADOPILOT_TEMPLATE_STABILITY_CONSECUTIVE,
                stability_observations: 2,
                stability_duration_nanos: 1,
                ..options()
            },
        ),
        (
            "zero duration",
            madopilot_template_watch_options_t {
                stability_kind: MADOPILOT_TEMPLATE_STABILITY_DURATION,
                ..options()
            },
        ),
        (
            "duration count",
            madopilot_template_watch_options_t {
                stability_kind: MADOPILOT_TEMPLATE_STABILITY_DURATION,
                stability_observations: 1,
                stability_duration_nanos: 1,
                ..options()
            },
        ),
    ];
    for (name, request) in cases {
        let mut query = ptr::dangling_mut();
        let mut error = ptr::null_mut();
        assert_eq!(
            unsafe {
                (flow.api.session_start_template_watch)(
                    flow.session,
                    flow.present,
                    &request,
                    &operation(),
                    &raw mut query,
                    &raw mut error,
                )
            },
            MADOPILOT_STATUS_INVALID_ARGUMENT,
            "{name}",
        );
        assert!(query.is_null(), "{name}");
        assert_eq!(
            describe_and_release(flow.api, error).category,
            MADOPILOT_ERROR_CATEGORY_ABI,
            "{name}"
        );
    }
}

#[test]
fn nested_match_options_keep_the_existing_tag_and_value_rules() {
    let flow = Flow::open();
    let empty = madopilot_match_options_t::cleared(struct_size::<madopilot_match_options_t>());
    let cases = [
        (
            madopilot_match_options_t {
                flags: MADOPILOT_MATCH_HAS_MIN_SCORE,
                min_score: f64::NAN,
                ..empty
            },
            MADOPILOT_ERROR_CATEGORY_VISION,
        ),
        (
            madopilot_match_options_t {
                flags: MADOPILOT_MATCH_HAS_MAX_RESULTS,
                max_results: 0,
                ..empty
            },
            MADOPILOT_ERROR_CATEGORY_VISION,
        ),
        (
            madopilot_match_options_t {
                flags: MADOPILOT_MATCH_HAS_SUPPRESSION,
                suppression: -1,
                ..empty
            },
            MADOPILOT_ERROR_CATEGORY_ABI,
        ),
    ];
    for (matching, category) in cases {
        let request = madopilot_template_watch_options_t {
            match_options: &matching,
            ..options()
        };
        refuse_start(
            &flow,
            flow.session,
            flow.present,
            &request,
            &operation(),
            MADOPILOT_STATUS_INVALID_ARGUMENT,
            category,
        );
    }
    let matching = madopilot_match_options_t {
        flags: (1 << 31) | MADOPILOT_MATCH_HAS_MAX_RESULTS,
        min_score: f64::NAN,
        max_results: 1,
        suppression: -1,
        ..empty
    };
    let request = madopilot_template_watch_options_t {
        match_options: &matching,
        ..options()
    };
    let terminal = Query::start(&flow, flow.present, &request, &operation()).wait();
    let info = terminal.info();
    assert_eq!(info.outcome, MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED);
    assert_eq!(
        info.match_count, 1,
        "existing nested option rules ignore unknown bits and inactive values"
    );
}

#[test]
fn absent_match_overrides_use_template_defaults_and_inactive_region_is_not_decoded() {
    let flow = Flow::open();
    let synchronous = flow.find(&flow.find_request());
    let mut expected =
        madopilot_match_options_t::cleared(struct_size::<madopilot_match_options_t>());
    assert_eq!(
        unsafe { (flow.api.result_options)(synchronous, &raw mut expected) },
        MADOPILOT_STATUS_OK
    );
    unsafe { (flow.api.result_release)(synchronous) };

    let matching = madopilot_match_options_t {
        struct_size: 8,
        flags: 0,
        min_score: f64::NAN,
        max_results: 0,
        suppression: -1,
    };
    let request = madopilot_template_watch_options_t {
        match_options: &matching,
        region: madopilot_pixel_rect_t {
            space: -1,
            left: 9,
            top: 9,
            right: 0,
            bottom: 0,
        },
        clip_policy: -1,
        ..options()
    };
    let query = Query::start(&flow, flow.present, &request, &operation());
    let terminal = query.wait();
    let actual = terminal.info();
    assert_eq!(actual.outcome, MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED);
    assert_eq!(actual.options.flags, expected.flags);
    assert_eq!(
        actual.options.min_score.to_bits(),
        expected.min_score.to_bits()
    );
    assert_eq!(actual.options.max_results, expected.max_results);
    assert_eq!(actual.options.suppression, expected.suppression);
}

#[test]
fn active_match_options_override_only_their_present_fields() {
    let flow = Flow::open();
    let matching = madopilot_match_options_t {
        flags: MADOPILOT_MATCH_HAS_MAX_RESULTS,
        min_score: f64::NAN,
        max_results: 1,
        suppression: -1,
        ..madopilot_match_options_t::cleared(struct_size::<madopilot_match_options_t>())
    };
    let request = madopilot_template_watch_options_t {
        match_options: &matching,
        ..options()
    };
    let query = Query::start(&flow, flow.present, &request, &operation());
    let terminal = query.wait();
    let info = terminal.info();
    assert_eq!(info.outcome, MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED);
    assert_eq!(info.match_count, 1);
    assert_eq!(info.options.max_results, 1);
    assert!(info.options.min_score.is_finite());
    assert_eq!(
        info.options.suppression,
        MADOPILOT_SUPPRESSION_DROP_OVERLAPPING
    );
}

#[test]
fn active_regions_keep_capture_pixel_geometry_and_clip_refusals() {
    let flow = Flow::open();
    let valid = madopilot_pixel_rect_t {
        space: MADOPILOT_SPACE_CAPTURE_PIXELS,
        left: 0,
        top: 0,
        right: 8,
        bottom: 8,
    };
    let cases = [
        (
            madopilot_pixel_rect_t {
                space: MADOPILOT_SPACE_TARGET_LOGICAL,
                ..valid
            },
            MADOPILOT_CLIP_POLICY_REJECT,
            MADOPILOT_ERROR_CATEGORY_ABI,
        ),
        (
            madopilot_pixel_rect_t { space: -1, ..valid },
            MADOPILOT_CLIP_POLICY_REJECT,
            MADOPILOT_ERROR_CATEGORY_ABI,
        ),
        (
            madopilot_pixel_rect_t { right: 0, ..valid },
            MADOPILOT_CLIP_POLICY_REJECT,
            MADOPILOT_ERROR_CATEGORY_UNSPECIFIED,
        ),
        (
            madopilot_pixel_rect_t { left: 9, ..valid },
            MADOPILOT_CLIP_POLICY_REJECT,
            MADOPILOT_ERROR_CATEGORY_GEOMETRY,
        ),
        (valid, -1, MADOPILOT_ERROR_CATEGORY_ABI),
    ];
    for (region, clip_policy, category) in cases {
        let request = madopilot_template_watch_options_t {
            flags: MADOPILOT_TEMPLATE_WATCH_HAS_REGION,
            region,
            clip_policy,
            ..options()
        };
        refuse_start(
            &flow,
            flow.session,
            flow.present,
            &request,
            &operation(),
            MADOPILOT_STATUS_INVALID_ARGUMENT,
            category,
        );
    }
}

#[test]
fn region_containment_is_resolved_against_the_analyzed_frame() {
    let flow = Flow::open();
    let frame = flow.scene().frame_input();
    let region = madopilot_pixel_rect_t {
        space: MADOPILOT_SPACE_CAPTURE_PIXELS,
        left: -1,
        top: -1,
        right: i32::try_from(frame.width).unwrap(),
        bottom: i32::try_from(frame.height).unwrap(),
    };
    let request = madopilot_template_watch_options_t {
        flags: MADOPILOT_TEMPLATE_WATCH_HAS_REGION,
        region,
        clip_policy: MADOPILOT_CLIP_POLICY_CLIP,
        ..options()
    };
    let clipped = Query::start(&flow, flow.present, &request, &operation()).wait();
    let info = clipped.info();
    assert_eq!(info.outcome, MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED);
    assert_eq!(
        (info.effective_region.left, info.effective_region.top),
        (0, 0)
    );
    assert_eq!(
        (info.effective_region.right, info.effective_region.bottom),
        (region.right, region.bottom)
    );

    let request = madopilot_template_watch_options_t {
        clip_policy: MADOPILOT_CLIP_POLICY_REJECT,
        ..request
    };
    // The clipped watcher can exhaust Flow's single-frame source before this start.
    // A fresh replay keeps admission independent of that acquisition worker.
    let session = ReplaySession::new(&flow);
    let refused = Query::on_session(session.session, flow.present, &request, &operation()).wait();
    let info = refused.info();
    assert_eq!(info.outcome, MADOPILOT_TEMPLATE_QUERY_OUTCOME_FAILED);
    assert_eq!(info.status, MADOPILOT_STATUS_INVALID_ARGUMENT);
    let mut failure = ptr::null_mut();
    assert_eq!(
        unsafe { (flow.api.template_query_result_error)(refused.0, &raw mut failure) },
        MADOPILOT_STATUS_OK
    );
    let detail = describe_and_release(flow.api, failure);
    assert_eq!(detail.status, info.status);
    assert_eq!(detail.category, MADOPILOT_ERROR_CATEGORY_UNSPECIFIED);
    assert_eq!(detail.flags, 0);
    assert_eq!(detail.asset_fault, MADOPILOT_ASSET_FAULT_UNKNOWN);
    assert_eq!(detail.asset_stage, MADOPILOT_ASSET_STAGE_UNKNOWN);
}

#[test]
fn full_width_timing_and_counts_are_accepted_without_signed_narrowing() {
    let flow = Flow::open();
    let request = madopilot_template_watch_options_t {
        minimum_interval_nanos: u64::MAX,
        stability_kind: MADOPILOT_TEMPLATE_STABILITY_DURATION,
        stability_duration_nanos: u64::MAX,
        change_policy: MADOPILOT_TEMPLATE_CHANGE_ANALYSIS_ALWAYS,
        ..options()
    };
    let query = Query::maintained(&flow, &request);
    query.pending();
    assert_eq!(
        query.cancel().info().outcome,
        MADOPILOT_TEMPLATE_QUERY_OUTCOME_CANCELLED
    );
    let request = madopilot_template_watch_options_t {
        minimum_interval_nanos: 3_600_000_000_000,
        stability_kind: MADOPILOT_TEMPLATE_STABILITY_CONSECUTIVE,
        stability_observations: u32::MAX,
        ..options()
    };
    let query = Query::maintained(&flow, &request);
    query.pending();
}

#[test]
fn required_start_inputs_and_query_operation_faults_publish_nothing() {
    let flow = Flow::open();
    let request = options();
    let operation = operation();
    for (session, template, options, operation) in [
        (
            ptr::null(),
            flow.present.cast_const(),
            &request as *const _,
            &operation as *const _,
        ),
        (
            flow.session.cast_const(),
            ptr::null(),
            &request as *const _,
            &operation as *const _,
        ),
        (
            flow.session.cast_const(),
            flow.present.cast_const(),
            ptr::null(),
            &operation as *const _,
        ),
        (
            flow.session.cast_const(),
            flow.present.cast_const(),
            &request as *const _,
            ptr::null(),
        ),
    ] {
        refuse_start(
            &flow,
            session,
            template,
            options,
            operation,
            MADOPILOT_STATUS_INVALID_ARGUMENT,
            MADOPILOT_ERROR_CATEGORY_ABI,
        );
    }
    let partial = madopilot_operation_t {
        struct_size: 20,
        ..operation
    };
    refuse_start(
        &flow,
        flow.session,
        flow.present,
        &request,
        &partial,
        MADOPILOT_STATUS_INVALID_ARGUMENT,
        MADOPILOT_ERROR_CATEGORY_ABI,
    );
    refuse_start(
        &flow,
        flow.session,
        flow.present,
        &request,
        &expired_operation(),
        MADOPILOT_STATUS_DEADLINE_EXCEEDED,
        MADOPILOT_ERROR_CATEGORY_OPERATION,
    );

    let mut cancellation = ptr::null_mut();
    assert_eq!(
        unsafe { (flow.api.cancellation_create)(&raw mut cancellation) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(
        unsafe { (flow.api.cancellation_cancel)(cancellation) },
        MADOPILOT_STATUS_OK
    );
    let cancelled = madopilot_operation_t {
        cancellation,
        ..operation
    };
    refuse_start(
        &flow,
        flow.session,
        flow.present,
        &request,
        &cancelled,
        MADOPILOT_STATUS_CANCELLED,
        MADOPILOT_ERROR_CATEGORY_OPERATION,
    );
    unsafe { (flow.api.cancellation_release)(cancellation) };
}

#[test]
fn same_backend_template_can_cross_engines_and_start_borrows_only_call_local_storage() {
    let receiver = Flow::open();
    let query = {
        let foreign = Flow::open();
        let matching = Box::new(madopilot_match_options_t {
            flags: MADOPILOT_MATCH_HAS_MAX_RESULTS,
            max_results: 1,
            ..madopilot_match_options_t::cleared(struct_size::<madopilot_match_options_t>())
        });
        let request = Box::new(madopilot_template_watch_options_t {
            match_options: &*matching,
            ..options()
        });
        let mut cancellation = ptr::null_mut();
        assert_eq!(
            unsafe { (receiver.api.cancellation_create)(&raw mut cancellation) },
            MADOPILOT_STATUS_OK
        );
        let operation = madopilot_operation_t {
            cancellation,
            ..operation()
        };
        let query = Query::start(&receiver, foreign.present, &request, &operation);
        assert_eq!(
            unsafe { (receiver.api.cancellation_release)(cancellation) },
            MADOPILOT_STATUS_OK
        );
        // Drop foreign template/package/engine and every borrowed request record.
        query
    };
    let terminal = query.wait();
    drop(query);
    drop(receiver);
    let info = terminal.info();
    assert_eq!(info.outcome, MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED);
    assert_eq!(info.match_count, 1);
    assert_eq!(support::view_to_string(info.template_id), "panel.patch");
    assert_eq!(info.options.max_results, 1);
}

const POISON: u8 = 0xa5;

#[repr(C, align(8))]
struct Record([u8; 320]);

impl Record {
    fn new(size: u32) -> Self {
        let mut record = Self([POISON; 320]);
        record.0[..4].copy_from_slice(&size.to_ne_bytes());
        record
    }

    fn as_mut_ptr<T>(&mut self) -> *mut T {
        self.0.as_mut_ptr().cast()
    }

    fn assert_zeroed(&self, size: usize, sizes: &[(usize, u32)]) {
        let mut expected = vec![0; size];
        for &(offset, value) in sizes {
            expected[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
        }
        assert_eq!(&self.0[..size], &expected);
        assert!(
            self.0[size..].iter().all(|byte| *byte == POISON),
            "unknown output bytes remain untouched"
        );
    }
}

#[test]
fn poll_initializes_each_legal_output_even_when_a_sibling_is_invalid() {
    let flow = Flow::open();
    let query = Query::maintained(&flow, &deferred_options());
    let mut result = ptr::dangling_mut();
    let mut error = ptr::null_mut();
    assert_eq!(
        unsafe {
            (flow.api.template_query_poll)(
                query.0,
                ptr::null_mut(),
                &raw mut result,
                &raw mut error,
            )
        },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    assert!(result.is_null());
    assert_eq!(
        describe_and_release(flow.api, error).category,
        MADOPILOT_ERROR_CATEGORY_ABI
    );

    let mut snapshot = Record::new(152);
    let untouched = snapshot.0;
    let mut result = ptr::dangling_mut();
    let mut error = ptr::null_mut();
    assert_eq!(
        unsafe {
            (flow.api.template_query_poll)(
                query.0,
                snapshot.as_mut_ptr(),
                &raw mut result,
                &raw mut error,
            )
        },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    assert_eq!(snapshot.0, untouched);
    assert!(result.is_null());
    assert_eq!(
        describe_and_release(flow.api, error).category,
        MADOPILOT_ERROR_CATEGORY_ABI
    );

    let mut storage = Record::new(0);
    let untouched = storage.0;
    let invalid_snapshot = unsafe { storage.0.as_mut_ptr().add(1).cast() };
    let mut result = ptr::dangling_mut();
    let mut error = ptr::null_mut();
    assert_eq!(
        unsafe {
            (flow.api.template_query_poll)(
                query.0,
                invalid_snapshot,
                &raw mut result,
                &raw mut error,
            )
        },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    assert_eq!(storage.0, untouched);
    assert!(result.is_null());
    assert_eq!(
        describe_and_release(flow.api, error).category,
        MADOPILOT_ERROR_CATEGORY_ABI
    );

    let mut snapshot = Record::new(MADOPILOT_TEMPLATE_QUERY_SNAPSHOT_SIZE_V1_6 + 8);
    let mut error = ptr::null_mut();
    assert_eq!(
        unsafe {
            (flow.api.template_query_poll)(
                query.0,
                snapshot.as_mut_ptr(),
                ptr::null_mut(),
                &raw mut error,
            )
        },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    snapshot.assert_zeroed(160, &[(0, 160), (48, 40)]);
    assert_eq!(
        describe_and_release(flow.api, error).category,
        MADOPILOT_ERROR_CATEGORY_ABI
    );

    let mut snapshot = Record::new(MADOPILOT_TEMPLATE_QUERY_SNAPSHOT_SIZE_V1_6);
    let mut storage = Record::new(0);
    let untouched = storage.0;
    let invalid_result = unsafe { storage.0.as_mut_ptr().add(1).cast() };
    let mut error = ptr::null_mut();
    assert_eq!(
        unsafe {
            (flow.api.template_query_poll)(
                query.0,
                snapshot.as_mut_ptr(),
                invalid_result,
                &raw mut error,
            )
        },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    snapshot.assert_zeroed(160, &[(0, 160), (48, 40)]);
    assert_eq!(storage.0, untouched);
    assert_eq!(
        describe_and_release(flow.api, error).category,
        MADOPILOT_ERROR_CATEGORY_ABI
    );

    let mut snapshot = Record::new(MADOPILOT_TEMPLATE_QUERY_SNAPSHOT_SIZE_V1_6);
    let mut result = ptr::dangling_mut();
    let mut storage = Record::new(0);
    let invalid_error = unsafe { storage.0.as_mut_ptr().add(1).cast() };
    assert_eq!(
        unsafe {
            (flow.api.template_query_poll)(
                query.0,
                snapshot.as_mut_ptr(),
                &raw mut result,
                invalid_error,
            )
        },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    snapshot.assert_zeroed(160, &[(0, 160), (48, 40)]);
    assert!(result.is_null());
    query.pending();
}

#[test]
fn invalid_cancel_outputs_cannot_compete_query_authority() {
    let flow = Flow::open();
    let query = Query::maintained(&flow, &deferred_options());
    let mut error = ptr::null_mut();
    assert_eq!(
        unsafe { (flow.api.template_query_cancel)(query.0, ptr::null_mut(), &raw mut error) },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    assert_eq!(
        describe_and_release(flow.api, error).category,
        MADOPILOT_ERROR_CATEGORY_ABI
    );
    let id = query.pending().query_id;

    let mut storage = Record::new(0);
    let untouched = storage.0;
    let invalid_result = unsafe { storage.0.as_mut_ptr().add(1).cast() };
    let mut error = ptr::null_mut();
    assert_eq!(
        unsafe { (flow.api.template_query_cancel)(query.0, invalid_result, &raw mut error) },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    assert_eq!(storage.0, untouched);
    assert_eq!(
        describe_and_release(flow.api, error).category,
        MADOPILOT_ERROR_CATEGORY_ABI
    );
    assert_eq!(query.pending().query_id, id);

    let mut result = ptr::dangling_mut();
    let mut storage = Record::new(0);
    let invalid_error = unsafe { storage.0.as_mut_ptr().add(1).cast() };
    assert_eq!(
        unsafe { (flow.api.template_query_cancel)(query.0, &raw mut result, invalid_error) },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    assert!(result.is_null());
    assert_eq!(query.pending().query_id, id);
    assert_eq!(
        query.cancel().info().outcome,
        MADOPILOT_TEMPLATE_QUERY_OUTCOME_CANCELLED
    );
}

#[test]
fn invalid_start_output_clears_its_legal_sibling_before_reading_inputs() {
    let api = table();
    let mut error = ptr::null_mut();
    assert_eq!(
        unsafe {
            (api.session_start_template_watch)(
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null_mut(),
                &raw mut error,
            )
        },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    assert_eq!(
        describe_and_release(api, error).category,
        MADOPILOT_ERROR_CATEGORY_ABI
    );

    let mut query = ptr::dangling_mut();
    let mut storage = Record::new(0);
    let invalid_error = unsafe { storage.0.as_mut_ptr().add(1).cast() };
    assert_eq!(
        unsafe {
            (api.session_start_template_watch)(
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                &raw mut query,
                invalid_error,
            )
        },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    assert!(query.is_null());
}

#[test]
fn null_query_and_result_inputs_reset_all_legal_owned_outputs() {
    let api = table();
    let mut result = ptr::dangling_mut();
    let mut error = ptr::null_mut();
    assert_eq!(
        unsafe {
            (api.template_query_wait)(ptr::null(), &operation(), &raw mut result, &raw mut error)
        },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    assert!(result.is_null());
    assert_eq!(
        describe_and_release(api, error).category,
        MADOPILOT_ERROR_CATEGORY_ABI
    );
    let mut result = ptr::dangling_mut();
    let mut error = ptr::null_mut();
    assert_eq!(
        unsafe { (api.template_query_cancel)(ptr::null(), &raw mut result, &raw mut error) },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    assert!(result.is_null());
    assert_eq!(
        describe_and_release(api, error).category,
        MADOPILOT_ERROR_CATEGORY_ABI
    );

    let mut frame = ptr::dangling_mut();
    assert_eq!(
        unsafe { (api.template_query_result_frame)(ptr::null(), &raw mut frame) },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    assert!(frame.is_null());
    let mut failure = ptr::dangling_mut();
    assert_eq!(
        unsafe { (api.template_query_result_error)(ptr::null(), &raw mut failure) },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    assert!(failure.is_null());
    assert_eq!(
        unsafe { (api.template_query_result_error)(ptr::null(), ptr::null_mut()) },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );

    assert_eq!(
        unsafe { (api.template_query_retain)(ptr::null()) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(
        unsafe { (api.template_query_release)(ptr::null_mut()) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(
        unsafe { (api.template_query_result_retain)(ptr::null()) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(
        unsafe { (api.template_query_result_release)(ptr::null_mut()) },
        MADOPILOT_STATUS_OK
    );
}

#[test]
fn nonmatched_and_out_of_range_accessors_reset_outputs_without_inventing_facts() {
    let flow = Flow::open();
    let query = Query::maintained(&flow, &deferred_options());
    let cancelled = query.cancel();
    let info = cancelled.info();
    assert_eq!(info.outcome, MADOPILOT_TEMPLATE_QUERY_OUTCOME_CANCELLED);
    assert_eq!(info.status, MADOPILOT_STATUS_CANCELLED);
    assert_eq!(info.target, 0);
    assert_eq!(info.match_count, 0);
    assert!(info.template_id.data.is_null());
    assert_eq!(info.template_id.len, 0);
    assert_eq!(info.options.flags, 0);
    assert_eq!(info.transform.flags, 0);

    let mut found = Record::new(struct_size::<madopilot_match_t>() + 8);
    assert_eq!(
        unsafe { (flow.api.template_query_result_match_at)(cancelled.0, 0, found.as_mut_ptr()) },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    found.assert_zeroed(56, &[(0, 56)]);
    let mut frame = ptr::dangling_mut();
    assert_eq!(
        unsafe { (flow.api.template_query_result_frame)(cancelled.0, &raw mut frame) },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    assert!(frame.is_null());
    let mut failure = ptr::dangling_mut();
    assert_eq!(
        unsafe { (flow.api.template_query_result_error)(cancelled.0, &raw mut failure) },
        MADOPILOT_STATUS_OK
    );
    assert!(failure.is_null());

    let matched = Query::start(&flow, flow.present, &options(), &operation()).wait();
    let info = matched.info();
    let mut found = Record::new(struct_size::<madopilot_match_t>());
    assert_eq!(
        unsafe {
            (flow.api.template_query_result_match_at)(
                matched.0,
                usize::try_from(info.match_count).unwrap(),
                found.as_mut_ptr(),
            )
        },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    found.assert_zeroed(56, &[(0, 56)]);
}
