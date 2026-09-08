//! Foreign query contracts through the negotiated table and controlled capture.

// Every call retains its handles and borrows live aligned records for its duration.
#![allow(clippy::undocumented_unsafe_blocks)]

use std::ptr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mado_pilot::{Continuity, PixelExtent, PixelFormat};
use mado_pilot_runtime::{
    CaptureProvider, Engine, EngineWiring, IdentityIssuer, Matcher, PackageLoader,
};
use mado_pilot_testkit::{CompletionGate, ControlledCapture, ControlledMatcher};

use crate::engine::EngineHandle;
use crate::layout::struct_size;
use crate::*;

fn api() -> &'static madopilot_api_t {
    let mut api = ptr::null();
    assert_eq!(
        unsafe {
            madopilot_get_api(
                MADOPILOT_ABI_MAJOR,
                MADOPILOT_ABI_MINOR,
                size_of::<madopilot_api_t>(),
                &mut api,
            )
        },
        MADOPILOT_STATUS_OK
    );
    unsafe { &*api }
}

fn operation() -> madopilot_operation_t {
    madopilot_operation_t {
        struct_size: struct_size::<madopilot_operation_t>(),
        flags: 0,
        deadline_nanos: 0,
        cancellation: ptr::null(),
        activity_tag: 0,
    }
}

fn bounded() -> madopilot_operation_t {
    let mut now = 0;
    assert_eq!(unsafe { (api().clock_now)(&mut now) }, MADOPILOT_STATUS_OK);
    madopilot_operation_t {
        flags: MADOPILOT_OPERATION_HAS_DEADLINE,
        deadline_nanos: now + 5_000_000_000,
        ..operation()
    }
}

fn string(text: &str) -> madopilot_str_t {
    madopilot_str_t {
        data: text.as_ptr().cast(),
        len: text.len(),
    }
}

struct Opened {
    api: &'static madopilot_api_t,
    engine: *mut madopilot_engine_t,
    session: *mut madopilot_session_t,
    template: *mut madopilot_template_t,
    capture: Arc<ControlledCapture>,
    matcher: Arc<ControlledMatcher>,
}

impl Opened {
    fn new(matcher: ControlledMatcher) -> Self {
        let api = api();
        let issuer = Arc::new(IdentityIssuer::new());
        let capture = Arc::new(
            ControlledCapture::new(
                Arc::clone(&issuer),
                PixelExtent::new(32, 24),
                PixelFormat::Rgba8,
            )
            .expect("controlled capture"),
        );
        let matcher = Arc::new(matcher);
        let engine = Engine::new(EngineWiring {
            engine: issuer.engine(),
            capture: Arc::clone(&capture) as Arc<dyn CaptureProvider>,
            matcher: Matcher::new(matcher.clone()),
            loader: PackageLoader::new(),
            ocr: None,
            input: None,
            permission: None,
        })
        .expect("controlled engine");
        let engine = handle::into_raw(EngineHandle::new(engine));
        let op = bounded();
        let mut targets = ptr::null_mut();
        assert_eq!(
            unsafe { (api.engine_discover)(engine, &op, &mut targets, ptr::null_mut()) },
            MADOPILOT_STATUS_OK
        );
        let request = madopilot_open_request_t {
            struct_size: struct_size::<madopilot_open_request_t>(),
            flags: 0,
            required_format: MADOPILOT_PIXEL_FORMAT_RGBA8,
            preferred_format: MADOPILOT_PIXEL_FORMAT_RGBA8,
        };
        let mut session = ptr::null_mut();
        assert_eq!(
            unsafe {
                (api.session_open)(
                    engine,
                    targets,
                    0,
                    &request,
                    &op,
                    &mut session,
                    ptr::null_mut(),
                )
            },
            MADOPILOT_STATUS_OK
        );
        assert_eq!(
            unsafe { (api.target_list_release)(targets) },
            MADOPILOT_STATUS_OK
        );
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../fixtures/assets/phase1-slice");
        let root = root.to_str().expect("fixture path");
        let source = madopilot_package_source_t {
            struct_size: struct_size::<madopilot_package_source_t>(),
            kind: MADOPILOT_PACKAGE_SOURCE_DIRECTORY,
            path: string(root),
            archive: madopilot_bytes_t::empty(),
        };
        let mut package = ptr::null_mut();
        assert_eq!(
            unsafe { (api.package_load)(engine, &source, &op, &mut package, ptr::null_mut()) },
            MADOPILOT_STATUS_OK
        );
        let mut template = ptr::null_mut();
        assert_eq!(
            unsafe {
                (api.template_prepare_from_package)(
                    engine,
                    package,
                    string("panel.patch"),
                    &op,
                    &mut template,
                    ptr::null_mut(),
                )
            },
            MADOPILOT_STATUS_OK
        );
        assert_eq!(
            unsafe { (api.package_release)(package) },
            MADOPILOT_STATUS_OK
        );
        Self {
            api,
            engine,
            session,
            template,
            capture,
            matcher,
        }
    }

    fn publish(&self, fill: u8) {
        self.capture
            .publish(fill, Continuity::Continuous)
            .expect("controlled publication");
    }

    fn release_parents(&mut self) {
        let op = bounded();
        if !self.session.is_null() {
            assert_eq!(
                unsafe { (self.api.session_close)(self.session, &op, ptr::null_mut()) },
                MADOPILOT_STATUS_OK
            );
        }
        unsafe {
            (self.api.template_release)(std::mem::replace(&mut self.template, ptr::null_mut()));
            (self.api.session_release)(std::mem::replace(&mut self.session, ptr::null_mut()));
            (self.api.engine_release)(std::mem::replace(&mut self.engine, ptr::null_mut()));
        }
    }
}

impl Drop for Opened {
    fn drop(&mut self) {
        self.release_parents();
    }
}

fn options() -> madopilot_template_watch_options_t {
    madopilot_template_watch_options_t {
        struct_size: struct_size::<madopilot_template_watch_options_t>(),
        flags: 0,
        match_options: ptr::null(),
        region: madopilot_pixel_rect_t::empty(),
        clip_policy: MADOPILOT_CLIP_POLICY_REJECT,
        minimum_interval_nanos: 0,
        stability_kind: MADOPILOT_TEMPLATE_STABILITY_IMMEDIATE,
        stability_observations: 0,
        stability_duration_nanos: 0,
        change_policy: MADOPILOT_TEMPLATE_CHANGE_EXACT_RGBA,
        reserved: 0,
    }
}

struct Query(*mut madopilot_template_query_t);

// SAFETY: each thread receives an independently retained query reference and
// calls only the documented concurrent C operations.
unsafe impl Send for Query {}

impl Query {
    fn start(
        opened: &Opened,
        options: &madopilot_template_watch_options_t,
        op: &madopilot_operation_t,
    ) -> Self {
        let mut query = ptr::null_mut();
        assert_eq!(
            unsafe {
                (opened.api.session_start_template_watch)(
                    opened.session,
                    opened.template,
                    options,
                    op,
                    &mut query,
                    ptr::null_mut(),
                )
            },
            MADOPILOT_STATUS_OK
        );
        assert!(!query.is_null());
        Self(query)
    }

    fn retained(&self) -> Self {
        assert_eq!(
            unsafe { (api().template_query_retain)(self.0) },
            MADOPILOT_STATUS_OK
        );
        Self(self.0)
    }

    fn poll(&self) -> (madopilot_template_query_snapshot_t, Option<Terminal>) {
        let mut snapshot = madopilot_template_query_snapshot_t::cleared(struct_size::<
            madopilot_template_query_snapshot_t,
        >());
        let mut result = ptr::null_mut();
        assert_eq!(
            unsafe {
                (api().template_query_poll)(self.0, &mut snapshot, &mut result, ptr::null_mut())
            },
            MADOPILOT_STATUS_OK
        );
        (snapshot, (!result.is_null()).then_some(Terminal(result)))
    }

    fn wait(&self) -> Terminal {
        let mut result = ptr::null_mut();
        assert_eq!(
            unsafe {
                (api().template_query_wait)(self.0, &bounded(), &mut result, ptr::null_mut())
            },
            MADOPILOT_STATUS_OK
        );
        assert!(!result.is_null());
        Terminal(result)
    }

    fn cancel(&self) -> Terminal {
        let mut result = ptr::null_mut();
        assert_eq!(
            unsafe { (api().template_query_cancel)(self.0, &mut result, ptr::null_mut()) },
            MADOPILOT_STATUS_OK
        );
        assert!(!result.is_null());
        Terminal(result)
    }
}

impl Drop for Query {
    fn drop(&mut self) {
        unsafe { (api().template_query_release)(self.0) };
    }
}

struct Terminal(*mut madopilot_template_query_result_t);

impl Terminal {
    fn info(&self) -> madopilot_template_query_result_info_t {
        let mut info = madopilot_template_query_result_info_t::cleared(struct_size::<
            madopilot_template_query_result_info_t,
        >());
        assert_eq!(
            unsafe { (api().template_query_result_info)(self.0, &mut info) },
            MADOPILOT_STATUS_OK
        );
        info
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        unsafe { (api().template_query_result_release)(self.0) };
    }
}

fn gated() -> (Opened, Arc<CompletionGate>) {
    let gate = Arc::new(CompletionGate::new());
    let opened = Opened::new(
        ControlledMatcher::new(PixelFormat::Rgba8)
            .with_match(3, 4, 0.97)
            .with_completion_gate(Arc::clone(&gate)),
    );
    (opened, gate)
}

fn progress_until(
    query: &Query,
    ready: impl Fn(&madopilot_template_query_snapshot_t) -> bool,
) -> madopilot_template_query_snapshot_t {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let (snapshot, terminal) = query.poll();
        assert!(terminal.is_none(), "query must remain pending");
        assert_eq!(snapshot.state, MADOPILOT_TEMPLATE_QUERY_STATE_PENDING);
        if ready(&snapshot) {
            return snapshot;
        }
        assert!(Instant::now() < deadline, "query progress deadline");
        std::thread::yield_now();
    }
}

#[test]
fn wait_interruption_does_not_cancel_query_or_bypass_committed_terminal() {
    let (opened, gate) = gated();
    let query = Query::start(&opened, &options(), &bounded());
    let _release = gate.release_guard();
    opened.publish(0x41);
    assert!(gate.wait_until_entered(Duration::from_secs(5)));
    let (pending, terminal) = query.poll();
    assert_eq!(pending.state, MADOPILOT_TEMPLATE_QUERY_STATE_PENDING);
    assert!(terminal.is_none());
    let expired = madopilot_operation_t {
        flags: MADOPILOT_OPERATION_HAS_DEADLINE,
        deadline_nanos: 0,
        ..operation()
    };
    let mut output = ptr::dangling_mut();
    assert_eq!(
        unsafe { (api().template_query_wait)(query.0, &expired, &mut output, ptr::null_mut()) },
        MADOPILOT_STATUS_DEADLINE_EXCEEDED
    );
    assert!(output.is_null());

    let mut cancellation = ptr::null_mut();
    assert_eq!(
        unsafe { (api().cancellation_create)(&mut cancellation) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(
        unsafe { (api().cancellation_cancel)(cancellation) },
        MADOPILOT_STATUS_OK
    );
    let cancelled_wait = madopilot_operation_t {
        cancellation,
        ..operation()
    };
    assert_eq!(
        unsafe {
            (api().template_query_wait)(query.0, &cancelled_wait, &mut output, ptr::null_mut())
        },
        MADOPILOT_STATUS_CANCELLED
    );
    assert!(output.is_null());
    gate.release();
    let terminal = query.wait();
    assert_eq!(
        terminal.info().outcome,
        MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED
    );
    // A cached terminal must not bypass the next wait's independent authority.
    assert_eq!(
        unsafe {
            (api().template_query_wait)(query.0, &cancelled_wait, &mut output, ptr::null_mut())
        },
        MADOPILOT_STATUS_CANCELLED
    );
    assert!(output.is_null());
    assert_eq!(
        unsafe { (api().cancellation_release)(cancellation) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(
        query.cancel().info().outcome,
        MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED
    );
}

#[test]
fn request_template_and_cancellation_reference_can_be_released_after_start() {
    let (mut opened, gate) = gated();
    let mut cancellation = ptr::null_mut();
    assert_eq!(
        unsafe { (api().cancellation_create)(&mut cancellation) },
        MADOPILOT_STATUS_OK
    );
    let query = {
        let overrides = madopilot_match_options_t {
            struct_size: struct_size::<madopilot_match_options_t>(),
            flags: MADOPILOT_MATCH_HAS_MAX_RESULTS,
            min_score: 0.0,
            max_results: 2,
            suppression: MADOPILOT_SUPPRESSION_DROP_OVERLAPPING,
        };
        let request = madopilot_template_watch_options_t {
            match_options: &overrides,
            ..options()
        };
        let op = madopilot_operation_t {
            cancellation,
            ..bounded()
        };
        Query::start(&opened, &request, &op)
    };
    let _release = gate.release_guard();
    assert_eq!(
        unsafe { (api().cancellation_release)(cancellation) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(
        unsafe {
            (api().template_release)(std::mem::replace(&mut opened.template, ptr::null_mut()))
        },
        MADOPILOT_STATUS_OK
    );
    opened.publish(0x42);
    assert!(gate.wait_until_entered(Duration::from_secs(5)));
    gate.release();
    let terminal = query.wait();
    let info = terminal.info();
    assert_eq!(info.outcome, MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED);
    assert_eq!(info.options.max_results, 2);
    assert!((info.options.min_score - 0.9).abs() < f64::EPSILON);
}

#[test]
fn no_match_and_unconfirmed_stability_remain_pending() {
    use mado_pilot_testkit::ScriptedMatchCall;
    let opened = Opened::new(ControlledMatcher::new(PixelFormat::Rgba8).with_calls([
        ScriptedMatchCall::new(vec![]),
        ScriptedMatchCall::matching(3, 4, 0.97),
        ScriptedMatchCall::matching(3, 4, 0.97),
    ]));
    let request = madopilot_template_watch_options_t {
        stability_kind: MADOPILOT_TEMPLATE_STABILITY_CONSECUTIVE,
        stability_observations: 2,
        ..options()
    };
    let query = Query::start(&opened, &request, &bounded());
    opened.publish(0x43);
    let first = progress_until(&query, |p| p.completed == 1);
    assert_eq!(first.confirmed_observations, 0);
    opened.publish(0x44);
    let second = progress_until(&query, |p| p.confirmed_observations == 1);
    for _ in 0..256 {
        let (pending, terminal) = query.poll();
        assert!(terminal.is_none());
        assert_eq!(pending.confirmed_observations, 1);
        assert_eq!(pending.generation, second.generation);
        assert!(pending.pending_count <= 1);
        assert!(pending.in_flight_count <= 1);
    }
    opened.publish(0x45);
    let info = query.wait().info();
    assert_eq!(info.outcome, MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED);
    assert_eq!(info.confirmed_observations, 2);
}

#[test]
fn independently_retained_poll_wait_cancel_references_share_one_terminal() {
    let (opened, gate) = gated();
    let query = Query::start(&opened, &options(), &bounded());
    let _release = gate.release_guard();
    opened.publish(0x46);
    assert!(gate.wait_until_entered(Duration::from_secs(5)));
    let waiter = query.retained();
    let canceller = query.retained();
    let observer = query.retained();
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let wait_barrier = Arc::clone(&barrier);
    let wait = std::thread::spawn(move || {
        wait_barrier.wait();
        waiter.wait().info().outcome
    });
    let cancel_barrier = Arc::clone(&barrier);
    let cancel = std::thread::spawn(move || {
        cancel_barrier.wait();
        let outcome = canceller.cancel().info().outcome;
        assert_eq!(canceller.cancel().info().outcome, outcome);
        outcome
    });
    drop(query);
    let (snapshot, terminal) = observer.poll();
    assert_eq!(snapshot.state, MADOPILOT_TEMPLATE_QUERY_STATE_PENDING);
    assert!(terminal.is_none(), "non-final release cannot cancel");
    barrier.wait();
    let outcome = wait.join().expect("waiter exits");
    assert_eq!(cancel.join().expect("canceller exits"), outcome);
    assert_eq!(outcome, MADOPILOT_TEMPLATE_QUERY_OUTCOME_CANCELLED);
    gate.release();
    assert!(gate.wait_until_completed(Duration::from_secs(5)));
    assert_eq!(observer.wait().info().outcome, outcome);
}

#[test]
fn session_close_authority_survives_an_interrupted_drain_and_late_completion() {
    let (opened, gate) = gated();
    let _release = gate.release_guard();
    let query = Query::start(&opened, &options(), &bounded());
    opened.publish(0x51);
    assert!(gate.wait_until_entered(Duration::from_secs(5)));
    let expired = madopilot_operation_t {
        flags: MADOPILOT_OPERATION_HAS_DEADLINE,
        deadline_nanos: 0,
        ..operation()
    };
    assert_eq!(
        unsafe { (api().session_close)(opened.session, &expired, ptr::null_mut()) },
        MADOPILOT_STATUS_DEADLINE_EXCEEDED
    );
    let info = query.cancel().info();
    assert_eq!(
        info.outcome,
        MADOPILOT_TEMPLATE_QUERY_OUTCOME_SESSION_CLOSED
    );
    assert_eq!(info.status, MADOPILOT_STATUS_CLOSED);
    let mut refused = ptr::dangling_mut();
    assert_eq!(
        unsafe {
            (api().session_start_template_watch)(
                opened.session,
                opened.template,
                &options(),
                &bounded(),
                &mut refused,
                ptr::null_mut(),
            )
        },
        MADOPILOT_STATUS_CLOSED
    );
    assert!(refused.is_null());
    gate.release();
    assert!(gate.wait_until_completed(Duration::from_secs(5)));
    assert_eq!(query.wait().info().outcome, info.outcome);
}

#[test]
fn engine_release_closes_scheduler_without_retaining_parents_in_query() {
    let mut opened = Opened::new(ControlledMatcher::new(PixelFormat::Rgba8));
    let query = Query::start(&opened, &options(), &bounded());
    assert_eq!(
        unsafe { (api().engine_release)(std::mem::replace(&mut opened.engine, ptr::null_mut())) },
        MADOPILOT_STATUS_OK
    );
    let info = query.wait().info();
    assert_eq!(
        info.outcome,
        MADOPILOT_TEMPLATE_QUERY_OUTCOME_SCHEDULER_CLOSED
    );
    assert_eq!(info.status, MADOPILOT_STATUS_CLOSED);
    let mut refused = ptr::dangling_mut();
    assert_eq!(
        unsafe {
            (api().session_start_template_watch)(
                opened.session,
                opened.template,
                &options(),
                &bounded(),
                &mut refused,
                ptr::null_mut(),
            )
        },
        MADOPILOT_STATUS_CLOSED
    );
    assert!(refused.is_null());
}

#[test]
fn target_loss_defeats_a_late_backend_match() {
    let (opened, gate) = gated();
    let _release = gate.release_guard();
    let query = Query::start(&opened, &options(), &bounded());
    opened.publish(0x52);
    assert!(gate.wait_until_entered(Duration::from_secs(5)));
    opened.capture.lose(opened.capture.target());
    let info = query.wait().info();
    assert_eq!(info.outcome, MADOPILOT_TEMPLATE_QUERY_OUTCOME_TARGET_LOST);
    assert_eq!(info.status, MADOPILOT_STATUS_TARGET_LOST);
    gate.release();
    assert!(gate.wait_until_completed(Duration::from_secs(5)));
    assert_eq!(query.cancel().info().outcome, info.outcome);
}

#[test]
fn resize_supersedes_old_analysis_and_returns_the_new_frame_geometry() {
    use mado_pilot_testkit::ScriptedMatchCall;
    let old_gate = Arc::new(CompletionGate::new());
    let new_gate = Arc::new(CompletionGate::new());
    let opened = Opened::new(ControlledMatcher::new(PixelFormat::Rgba8).with_calls([
        ScriptedMatchCall::matching(3, 4, 0.97).with_completion_gate(Arc::clone(&old_gate)),
        ScriptedMatchCall::matching(3, 4, 0.97).with_completion_gate(Arc::clone(&new_gate)),
    ]));
    let _old_release = old_gate.release_guard();
    let _new_release = new_gate.release_guard();
    let query = Query::start(&opened, &options(), &bounded());
    opened.publish(0x53);
    assert!(old_gate.wait_until_entered(Duration::from_secs(5)));
    let mut original = ptr::null_mut();
    assert_eq!(
        unsafe {
            (api().session_acquire_frame)(
                opened.session,
                &bounded(),
                &mut original,
                ptr::null_mut(),
            )
        },
        MADOPILOT_STATUS_OK
    );
    let mut old = madopilot_frame_stamp_t {
        struct_size: struct_size::<madopilot_frame_stamp_t>(),
        flags: 0,
        stream: 0,
        epoch: 0,
        sequence: 0,
        geometry: 0,
    };
    assert_eq!(
        unsafe { (api().frame_stamp)(original, &mut old) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(
        unsafe { (api().frame_release)(original) },
        MADOPILOT_STATUS_OK
    );
    opened
        .capture
        .publish_reshaped(PixelExtent::new(48, 32), 0x54)
        .expect("resized frame");
    progress_until(&query, |p| p.superseded >= 1);
    old_gate.release();
    assert!(new_gate.wait_until_entered(Duration::from_secs(5)));
    assert!(
        query.poll().1.is_none(),
        "old geometry cannot commit while its replacement is held"
    );
    new_gate.release();
    let info = query.wait().info();
    assert_eq!(info.outcome, MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED);
    assert_eq!((info.transform.width, info.transform.height), (48, 32));
    assert!(info.source.geometry > old.geometry);
    assert!(info.source.epoch > old.epoch);
    assert_eq!(info.transform.geometry, info.source.geometry);
}

#[test]
fn query_deadline_is_terminal_data_and_does_not_wait_for_a_held_backend() {
    let (opened, gate) = gated();
    let _release = gate.release_guard();
    let mut lifetime = bounded();
    lifetime.deadline_nanos -= 4_000_000_000;
    let query = Query::start(&opened, &options(), &lifetime);
    opened.publish(0x55);
    assert!(gate.wait_until_entered(Duration::from_secs(5)));
    let terminal = query.wait();
    assert_eq!(
        terminal.info().outcome,
        MADOPILOT_TEMPLATE_QUERY_OUTCOME_DEADLINE_EXCEEDED
    );
    assert_eq!(terminal.info().status, MADOPILOT_STATUS_DEADLINE_EXCEEDED);
    gate.release();
    assert!(gate.wait_until_completed(Duration::from_secs(5)));
    assert_eq!(
        query.cancel().info().outcome,
        MADOPILOT_TEMPLATE_QUERY_OUTCOME_DEADLINE_EXCEEDED
    );
}

#[test]
fn failed_terminal_error_and_borrowed_detail_survive_all_other_owners() {
    let mut opened = Opened::new(
        ControlledMatcher::new(PixelFormat::Rgba8).finding(mado_pilot_testkit::Behavior::Fail),
    );
    let query = Query::start(&opened, &options(), &bounded());
    opened.publish(0x56);
    let terminal = query.wait();
    let info = terminal.info();
    assert_eq!(info.outcome, MADOPILOT_TEMPLATE_QUERY_OUTCOME_FAILED);
    assert_eq!(info.status, MADOPILOT_STATUS_VISION_FAILED);
    let mut error = ptr::null_mut();
    assert_eq!(
        unsafe { (api().template_query_result_error)(terminal.0, &mut error) },
        MADOPILOT_STATUS_OK
    );
    let mut before = madopilot_error_detail_t {
        struct_size: struct_size::<madopilot_error_detail_t>(),
        flags: 0,
        status: MADOPILOT_STATUS_INTERNAL,
        category: MADOPILOT_ERROR_CATEGORY_UNSPECIFIED,
        asset_fault: MADOPILOT_ASSET_FAULT_UNKNOWN,
        asset_stage: MADOPILOT_ASSET_STAGE_UNKNOWN,
        message: madopilot_str_t::empty(),
        backend: madopilot_str_t::empty(),
    };
    assert_eq!(
        unsafe { (api().error_describe)(error, &mut before) },
        MADOPILOT_STATUS_OK
    );
    let detail =
        unsafe { std::slice::from_raw_parts(before.message.data.cast::<u8>(), before.message.len) }
            .to_vec();
    drop(terminal);
    drop(query);
    opened.release_parents();
    let mut after = before;
    assert_eq!(
        unsafe { (api().error_describe)(error, &mut after) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(after.status, MADOPILOT_STATUS_VISION_FAILED);
    assert_eq!(after.category, MADOPILOT_ERROR_CATEGORY_VISION);
    assert_eq!(after.flags, 0, "no fabricated backend or asset provenance");
    assert_eq!(
        unsafe { std::slice::from_raw_parts(after.message.data.cast::<u8>(), after.message.len) },
        detail
    );
    assert_eq!(unsafe { (api().error_release)(error) }, MADOPILOT_STATUS_OK);
}

#[test]
fn final_query_release_and_unpublished_panic_reclaim_admission_capacity() {
    let opened = Opened::new(ControlledMatcher::new(PixelFormat::Rgba8));
    let mut descriptor = madopilot_template_scheduler_descriptor_t::cleared(struct_size::<
        madopilot_template_scheduler_descriptor_t,
    >());
    assert_eq!(
        unsafe { (api().engine_template_scheduler_descriptor)(opened.engine, &mut descriptor) },
        MADOPILOT_STATUS_OK
    );
    for _ in 0..=descriptor.max_session_queries {
        let query = Query::start(&opened, &options(), &bounded());
        drop(query);
        let mut unpublished = ptr::dangling_mut();
        let status = hooks::armed(hooks::Site::AfterTemporary, || unsafe {
            (api().session_start_template_watch)(
                opened.session,
                opened.template,
                &options(),
                &bounded(),
                &mut unpublished,
                ptr::null_mut(),
            )
        });
        assert_eq!(status, MADOPILOT_STATUS_INTERNAL_PANIC);
        assert!(unpublished.is_null());
    }
    let query = Query::start(&opened, &options(), &bounded());
    assert!(query.poll().1.is_none());
}

#[test]
fn cancel_panic_publishes_no_owner_but_cannot_undo_winning_cancellation() {
    let opened = Opened::new(ControlledMatcher::new(PixelFormat::Rgba8));
    let query = Query::start(&opened, &options(), &bounded());
    let mut output = ptr::dangling_mut();
    let status = hooks::armed(hooks::Site::AfterTemporary, || unsafe {
        (api().template_query_cancel)(query.0, &mut output, ptr::null_mut())
    });
    assert_eq!(status, MADOPILOT_STATUS_INTERNAL_PANIC);
    assert!(output.is_null());
    assert_eq!(
        query.wait().info().outcome,
        MADOPILOT_TEMPLATE_QUERY_OUTCOME_CANCELLED
    );
}

#[test]
fn query_cancellation_token_owns_authority_after_the_start_records_are_gone() {
    let (opened, gate) = gated();
    let _release = gate.release_guard();
    let mut cancellation = ptr::null_mut();
    assert_eq!(
        unsafe { (api().cancellation_create)(&mut cancellation) },
        MADOPILOT_STATUS_OK
    );
    let query = {
        let lifetime = madopilot_operation_t {
            cancellation,
            ..bounded()
        };
        Query::start(&opened, &options(), &lifetime)
    };
    opened.publish(0x57);
    assert!(gate.wait_until_entered(Duration::from_secs(5)));
    assert_eq!(
        unsafe { (api().cancellation_cancel)(cancellation) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(
        unsafe { (api().cancellation_release)(cancellation) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(
        query.wait().info().outcome,
        MADOPILOT_TEMPLATE_QUERY_OUTCOME_CANCELLED
    );
    gate.release();
    assert!(gate.wait_until_completed(Duration::from_secs(5)));
    assert_eq!(
        query.poll().1.expect("terminal").info().outcome,
        MADOPILOT_TEMPLATE_QUERY_OUTCOME_CANCELLED
    );
}

#[test]
fn rate_deferred_observation_confirms_duration_without_poll_driven_stability() {
    let opened = Opened::new(ControlledMatcher::new(PixelFormat::Rgba8).with_match(3, 4, 0.97));
    let request = madopilot_template_watch_options_t {
        minimum_interval_nanos: 100_000_000,
        stability_kind: MADOPILOT_TEMPLATE_STABILITY_DURATION,
        stability_duration_nanos: 50_000_000,
        ..options()
    };
    let query = Query::start(&opened, &request, &bounded());
    opened.publish(0x58);
    let first = progress_until(&query, |p| p.confirmed_observations == 1);
    assert_eq!(first.confirmed_duration_nanos, 0);
    for _ in 0..64 {
        let (pending, terminal) = query.poll();
        assert!(terminal.is_none());
        assert_eq!(pending.confirmed_duration_nanos, 0);
    }
    opened.publish(0x59);
    let result = query.wait();
    let info = result.info();
    assert_eq!(info.outcome, MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED);
    assert_eq!(info.confirmed_observations, 2);
    assert!(info.confirmed_duration_nanos >= request.stability_duration_nanos);
    assert_eq!(opened.matcher.find_count(), 2);
}

#[test]
fn a_template_prepared_by_another_backend_is_refused_before_query_publication() {
    use mado_pilot::replay::{ReplayFrame, ReplaySource, ReplayTarget};
    use mado_pilot::{FrameDescriptor, MonotonicInstant, OperationContext};
    let opened = Opened::new(ControlledMatcher::new(PixelFormat::Rgba8));
    let descriptor = FrameDescriptor::packed(PixelExtent::new(32, 24), PixelFormat::Rgba8).unwrap();
    let frame = ReplayFrame::new(
        descriptor,
        MonotonicInstant::ORIGIN,
        Continuity::Continuous,
        None,
        vec![0; descriptor.byte_len()].into_boxed_slice(),
    )
    .unwrap();
    let source =
        ReplaySource::from_targets(vec![ReplayTarget::new("foreign", vec![frame]).unwrap()])
            .unwrap();
    let other = mado_pilot::replay_engine(source).expect("OpenCV replay engine");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/assets/phase1-slice");
    let operation = OperationContext::new();
    let package = other
        .load_package(&mado_pilot::PackageSource::directory(root), &operation)
        .expect("validated foreign package");
    let template = other
        .prepare_from_package(&package, "panel.patch", &operation)
        .expect("OpenCV prepared template");
    let template = handle::into_raw(template);
    let mut output = ptr::dangling_mut();
    assert_eq!(
        unsafe {
            (api().session_start_template_watch)(
                opened.session,
                template,
                &options(),
                &bounded(),
                &mut output,
                ptr::null_mut(),
            )
        },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    assert!(output.is_null());
    assert_eq!(
        unsafe { (api().template_release)(template) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(opened.matcher.find_count(), 0);
}
