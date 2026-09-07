//! Public-only replay fixtures and consumer oracles for the paired harness.

use std::ptr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mado_pilot::replay::{ReplayFrame, ReplaySource, ReplayTarget};
use mado_pilot::{
    CancellationToken, ChangeDetectionPolicy, Continuity, Engine, FrameDescriptor, FrameRequest,
    FrameStamp, MatchOptions, MonotonicInstant, OpenRequest, OperationContext, PackageSource,
    PixelFormat, Session, Status, TemplateAnalysisRate, TemplateQuery, TemplateQueryOutcome,
    TemplateQueryProgress, TemplateTerminalOutcome, TemplateWatchRequest, TemplateWorkDisposition,
};
use mado_pilot_testkit::match_fixtures;
use madopilot::layout::struct_size;
use madopilot::*;

use super::{BATCH, Case, PENDING_INTERVAL, QUERY_LIFETIME, SETUP_WAIT, package_root};

const TEMPLATE: &str = "panel.patch";
const ABSENT: &str = "panel.absent";
const BOUNDS: [i32; 4] = [0, 0, 96, 64];
const PLANTED: [[i32; 4]; 2] = [[20, 12, 32, 22], [60, 40, 72, 50]];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp([u64; 4]);

impl Stamp {
    fn rust(stamp: FrameStamp) -> Self {
        Self([
            stamp.stream().get(),
            stamp.epoch().value(),
            stamp.sequence().value(),
            stamp.geometry().value(),
        ])
    }

    fn c(stamp: madopilot_frame_stamp_t) -> Self {
        assert_eq!(stamp.struct_size, struct_size::<madopilot_frame_stamp_t>());
        assert_eq!(stamp.flags, 0);
        Self([stamp.stream, stamp.epoch, stamp.sequence, stamp.geometry])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Progress {
    id: u64,
    frame: Option<Stamp>,
    generation: u64,
    observations: u32,
    duration: u64,
    pending: u32,
    in_flight: u32,
    work: [u64; 9],
}

impl Progress {
    fn rust(progress: TemplateQueryProgress) -> Self {
        use TemplateWorkDisposition as Work;
        Self {
            id: progress.id().get(),
            frame: progress.last_frame().map(Stamp::rust),
            generation: progress.generation(),
            observations: progress.confirmed_observations(),
            duration: u64::try_from(progress.confirmed_duration().as_nanos())
                .expect("fixture duration fits"),
            pending: progress.pending_count(),
            in_flight: progress.in_flight_count(),
            work: [
                Work::Admitted,
                Work::SkippedChange,
                Work::DeferredRate,
                Work::Coalesced,
                Work::Superseded,
                Work::Rejected,
                Work::QueueExpired,
                Work::Completed,
                Work::Failed,
            ]
            .map(|kind| progress.work().get(kind)),
        }
    }

    fn c(snapshot: madopilot_template_query_snapshot_t) -> Self {
        assert_eq!(
            snapshot.struct_size,
            struct_size::<madopilot_template_query_snapshot_t>()
        );
        assert_eq!(snapshot.flags & !MADOPILOT_TEMPLATE_QUERY_HAS_LAST_FRAME, 0);
        let stamp = Stamp::c(snapshot.last_frame);
        let frame = if snapshot.flags & MADOPILOT_TEMPLATE_QUERY_HAS_LAST_FRAME != 0 {
            assert_ne!(
                stamp.0[0], 0,
                "a present frame has a nonzero stream identity"
            );
            Some(stamp)
        } else {
            assert_eq!(stamp, Stamp([0; 4]));
            None
        };
        Self {
            id: snapshot.query_id,
            frame,
            generation: snapshot.generation,
            observations: snapshot.confirmed_observations,
            duration: snapshot.confirmed_duration_nanos,
            pending: snapshot.pending_count,
            in_flight: snapshot.in_flight_count,
            work: [
                snapshot.admitted,
                snapshot.skipped_change,
                snapshot.deferred_rate,
                snapshot.coalesced,
                snapshot.superseded,
                snapshot.rejected,
                snapshot.queue_expired,
                snapshot.completed,
                snapshot.failed,
            ],
        }
    }

    fn settled(self) -> bool {
        self.id != 0
            && self.frame.is_some()
            && self.generation == 1
            && self.observations == 0
            && self.duration == 0
            && self.pending == 1
            && self.in_flight == 0
            && self.work[0] == 1
            && self.work[1] == 0
            && self.work[2] == 1
            && self.work[3] == 0
            && self.work[5] == 0
            && self.work[6] == 0
            && self.work[7] == 1
            && self.work[8] == 0
    }

    fn terminal(self, id: u64) -> bool {
        self == Self {
            id,
            frame: None,
            generation: 0,
            observations: 0,
            duration: 0,
            pending: 0,
            in_flight: 0,
            work: [0; 9],
        }
    }
}

struct Expected {
    target: u64,
    query_id: u64,
    stamp: Stamp,
    pixels: Vec<u8>,
    backend: [Vec<u8>; 2],
}

pub(super) struct Exercise {
    pub correct: bool,
    pub readable_view: Option<usize>,
}

fn operation(timeout: Duration) -> OperationContext {
    OperationContext::new()
        .with_timeout(timeout)
        .expect("finite fixture deadline")
}

fn source(pixels: &[u8], pending: bool) -> ReplaySource {
    let descriptor = FrameDescriptor::packed(match_fixtures::SCENE, PixelFormat::Rgba8)
        .expect("fixture descriptor");
    let count = if pending { 2 } else { 1 };
    let frames = (0..count)
        .map(|index| {
            ReplayFrame::new(
                descriptor,
                MonotonicInstant::ORIGIN
                    .checked_add(Duration::from_nanos(index))
                    .expect("fixture timestamp"),
                Continuity::Continuous,
                None,
                pixels.to_vec().into_boxed_slice(),
            )
            .expect("fixture replay frame")
        })
        .collect();
    ReplaySource::from_targets(vec![
        ReplayTarget::new("panel", frames).expect("fixture target"),
    ])
    .expect("fixture replay source")
}

struct RustParents {
    session: Session,
    _engine: Engine,
}

impl Drop for RustParents {
    fn drop(&mut self) {
        self.session
            .close(&operation(SETUP_WAIT))
            .expect("bounded replay close");
    }
}

pub(super) struct RustFlow {
    // Query drop precedes parents; a retained result owns no query/thread owner.
    query: Option<Arc<TemplateQuery>>,
    result: Option<Arc<TemplateTerminalOutcome>>,
    parents: Option<RustParents>,
    wait: OperationContext,
    mapping_operation: OperationContext,
    expected: Expected,
    pending: Option<Progress>,
}

impl RustFlow {
    pub(super) fn new(case: Case) -> Self {
        let pixels = match_fixtures::scene_pixels(PixelFormat::Rgba8);
        let engine = mado_pilot::replay_engine(source(&pixels, case.pending()))
            .expect("real OpenCV replay engine");
        let setup = operation(SETUP_WAIT);
        let targets = engine.discover(&setup).expect("discover replay target");
        assert_eq!(targets.len(), 1);
        let target = targets[0].id();
        let session = engine
            .open(target, &OpenRequest::new(), &setup)
            .expect("open replay session");
        let frame = session
            .acquire_frame(&FrameRequest::latest(), &setup)
            .expect("maintained first frame");
        let stamp = Stamp::rust(frame.stamp());
        let package = engine
            .load_package(&PackageSource::directory(package_root()), &setup)
            .expect("tracked package");
        let template = engine
            .prepare_from_package(
                &package,
                if case.pending() { ABSENT } else { TEMPLATE },
                &setup,
            )
            .expect("tracked template");
        let rate = if case.pending() {
            TemplateAnalysisRate::at_most_every(PENDING_INTERVAL)
                .expect("finite positive rate interval")
        } else {
            TemplateAnalysisRate::unrestricted()
        };
        let query = Arc::new(
            session
                .start_template_watch(
                    TemplateWatchRequest::new(
                        template.clone(),
                        MatchOptions::from_defaults(template.defaults()),
                        operation(QUERY_LIFETIME),
                    )
                    .with_change_policy(ChangeDetectionPolicy::AnalysisAlways)
                    .with_rate(rate),
                )
                .expect("public query start"),
        );
        // Sharing Arc<TemplateQuery> mirrors C retain: exactly one Rust owner,
        // whose cancellation-on-Drop runs only on the last shared reference.
        drop(template);
        drop(package);
        drop(frame);
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let wait = operation(QUERY_LIFETIME).with_cancellation(cancellation);
        let query_id = query.id().get();
        let mut flow = Self {
            query: Some(query),
            result: None,
            parents: Some(RustParents {
                session,
                _engine: engine,
            }),
            wait,
            mapping_operation: operation(QUERY_LIFETIME),
            expected: Expected {
                target: target.get(),
                query_id,
                stamp,
                pixels,
                backend: [Vec::new(), Vec::new()],
            },
            pending: None,
        };
        if case.pending() {
            flow.pending = Some(flow.prime_pending());
        } else {
            let result = flow
                .query()
                .wait(&operation(SETUP_WAIT))
                .expect("bounded terminal setup wait");
            let TemplateTerminalOutcome::Matched(matched) = result.as_ref() else {
                panic!("matched fixture produced {result:?}");
            };
            flow.expected.backend = [
                matched.result().backend().id().as_bytes().to_vec(),
                matched.result().backend().version().as_bytes().to_vec(),
            ];
            // Subsequent observations run with a retained clone, not a temporary
            // wait output that accidentally kept a different path alive.
            flow.result = Some(Arc::clone(&result));
            drop(result);
            assert!(flow.read_matched(flow.result()));
        }
        if case == Case::RetainedFrame {
            flow.close_parents();
            drop(flow.query.take());
        }
        flow
    }

    fn query(&self) -> &Arc<TemplateQuery> {
        self.query
            .as_ref()
            .expect("query retained for this scenario")
    }

    fn result(&self) -> &Arc<TemplateTerminalOutcome> {
        self.result.as_ref().expect("matched result retained")
    }

    fn pending_progress(&self) -> Progress {
        match self.query().poll() {
            TemplateQueryOutcome::Pending(progress) => Progress::rust(progress),
            TemplateQueryOutcome::Terminal(outcome) => {
                panic!("pending fixture terminated: {outcome:?}")
            }
        }
    }

    fn prime_pending(&self) -> Progress {
        let deadline = Instant::now() + SETUP_WAIT;
        loop {
            let progress = self.pending_progress();
            if progress.settled() {
                assert_eq!(progress.id, self.expected.query_id);
                assert_eq!(
                    progress.frame,
                    Some(self.expected.stamp),
                    "the completed no-match analysis used the acquired first frame"
                );
                return progress;
            }
            assert!(
                Instant::now() < deadline,
                "no-match/rate-held setup did not settle: {progress:?}"
            );
            std::thread::yield_now();
        }
    }

    pub(super) fn close_parents(&mut self) {
        drop(self.parents.take());
    }

    pub(super) fn first_poll(&self) -> TemplateQueryOutcome {
        self.query().poll()
    }

    pub(super) fn closed_outcome(&self, outcome: &TemplateQueryOutcome) -> bool {
        self.query().id().get() == self.expected.query_id
            && matches!(outcome, TemplateQueryOutcome::Terminal(result)
                if matches!(result.as_ref(), TemplateTerminalOutcome::SessionClosed)
                    && result.status() == Some(Status::Closed))
    }

    pub(super) fn create_cancel_release() -> Exercise {
        let mut flow = Self::new(Case::CreateCancelRelease);
        let result = flow.query().cancel();
        let correct = flow.query().id().get() == flow.expected.query_id
            && matches!(result.as_ref(), TemplateTerminalOutcome::Cancelled)
            && result.status() == Some(Status::Cancelled)
            && matches!(flow.query().poll(), TemplateQueryOutcome::Terminal(observed)
                if matches!(observed.as_ref(), TemplateTerminalOutcome::Cancelled)
                    && observed.status() == Some(Status::Cancelled));
        drop(flow.query.take());
        flow.close_parents();
        let correct = correct
            && matches!(result.as_ref(), TemplateTerminalOutcome::Cancelled)
            && result.status() == Some(Status::Cancelled);
        drop(result);
        // All fixture, caller authority and parent/query/result owners are gone
        // before timed() reads its post-window heap and allocation counters.
        drop(flow);
        Exercise {
            correct,
            readable_view: None,
        }
    }

    fn read_matched(&self, outcome: &TemplateTerminalOutcome) -> bool {
        let TemplateTerminalOutcome::Matched(matched) = outcome else {
            return false;
        };
        let found = matched.result();
        let transform = matched.frame().transform();
        let options = found.options();
        let mut seen = 0_u8;
        let mut correct = outcome.status().is_none()
            && matched.target().get() == self.expected.target
            && matched.template().as_str() == TEMPLATE
            && Stamp::rust(matched.frame().stamp()) == self.expected.stamp
            && found.stamp() == matched.frame().stamp()
            && found.transform() == transform
            && transform.geometry() == matched.frame().stamp().geometry()
            && transform.frame_extent() == match_fixtures::SCENE
            && transform.covers_target()
            && transform.target().is_none()
            && matched.confirmed_observations() == 1
            && matched.confirmed_duration().is_zero()
            && found.backend().id().as_bytes() == self.expected.backend[0]
            && found.backend().version().as_bytes() == self.expected.backend[1]
            && !self.expected.backend[0].is_empty()
            && !self.expected.backend[1].is_empty()
            && options.min_score() == 0.9
            && options.max_results() == 8
            && options.suppression() == mado_pilot::Suppression::DropOverlapping
            && [
                found.searched().left(),
                found.searched().top(),
                found.searched().right(),
                found.searched().bottom(),
            ] == BOUNDS
            && found.matches().len() == PLANTED.len();
        for one in found.matches() {
            let bounds = one.bounds();
            correct &= one.template().as_str() == TEMPLATE;
            correct &= planted(
                [bounds.left(), bounds.top(), bounds.right(), bounds.bottom()],
                one.score(),
                &mut seen,
            );
        }
        correct && seen == 3
    }

    fn retained_frame(&self) -> (bool, usize) {
        let TemplateTerminalOutcome::Matched(matched) = self.result().as_ref() else {
            panic!("retained result must be matched");
        };
        let frame = matched.frame().clone();
        let mapping = frame
            .map(PixelFormat::Rgba8, &self.mapping_operation)
            .expect("retained frame maps");
        let correct = self.parents.is_none()
            && self.query.is_none()
            && Stamp::rust(frame.stamp()) == self.expected.stamp
            && Stamp::rust(mapping.stamp()) == self.expected.stamp
            && frame.descriptor() == mapping.descriptor()
            && mapping.descriptor().extent() == match_fixtures::SCENE
            && mapping.descriptor().format() == PixelFormat::Rgba8
            && mapping.is_shared()
            && mapping.bytes() == self.expected.pixels
            && self.read_matched(self.result());
        (correct, mapping.bytes().len())
    }

    pub(super) fn exercise(&self, case: Case) -> Exercise {
        let mut correct = true;
        let mut readable_view = None;
        for _ in 0..BATCH {
            let flow = std::hint::black_box(self);
            correct &= std::hint::black_box(match case {
                Case::PendingPoll => {
                    flow.pending_progress() == flow.pending.expect("settled snapshot")
                }
                Case::TerminalPoll => match flow.query().poll() {
                    TemplateQueryOutcome::Terminal(outcome) => flow.read_matched(&outcome),
                    TemplateQueryOutcome::Pending(_) => false,
                },
                Case::TerminalRead => flow.read_matched(flow.result()),
                Case::WaitCancelled => {
                    let waited = flow.query().wait(&flow.wait);
                    let interrupted =
                        matches!(&waited, Err(error) if error.status() == Status::Cancelled);
                    interrupted
                        && flow.pending_progress() == flow.pending.expect("settled snapshot")
                }
                Case::QueryClone => {
                    let query = Arc::clone(flow.query());
                    drop(query);
                    flow.pending_progress() == flow.pending.expect("settled snapshot")
                }
                Case::ResultClone => {
                    let result = Arc::clone(flow.result());
                    let readable = flow.read_matched(&result);
                    drop(result);
                    readable && flow.read_matched(flow.result())
                }
                Case::RetainedFrame => {
                    let (readable, bytes) = flow.retained_frame();
                    readable_view = Some(bytes);
                    readable
                }
                Case::FirstTerminal | Case::CreateCancelRelease => {
                    unreachable!("lifecycle observation has per-sample setup")
                }
            });
        }
        Exercise {
            correct,
            readable_view,
        }
    }
}

// A small bench-local owner for the existing C table's many handle types. It
// never dereferences an opaque pointer and never changes the library allocator.
struct CHandle<T> {
    pointer: *mut T,
    release: unsafe extern "C" fn(*mut T) -> madopilot_status_t,
}

impl<T> CHandle<T> {
    fn new(pointer: *mut T, release: unsafe extern "C" fn(*mut T) -> madopilot_status_t) -> Self {
        assert!(!pointer.is_null(), "successful C call must return an owner");
        Self { pointer, release }
    }

    fn shared(&self, retain: unsafe extern "C" fn(*const T) -> madopilot_status_t) -> Self {
        // SAFETY: this owner keeps the issuing library's exact handle live.
        ok(unsafe { retain(self.pointer) }, "retain");
        Self::new(self.pointer, self.release)
    }
}

impl<T> Drop for CHandle<T> {
    fn drop(&mut self) {
        // SAFETY: every owner represents exactly one produced/retained reference
        // and stores the releaser from that same negotiated static table.
        ok(unsafe { (self.release)(self.pointer) }, "release");
    }
}

struct CParents {
    api: &'static madopilot_api_t,
    session: CHandle<madopilot_session_t>,
    engine: CHandle<madopilot_engine_t>,
}

impl Drop for CParents {
    fn drop(&mut self) {
        let operation = c_operation(self.api, SETUP_WAIT);
        // SAFETY: the session and operation remain live through bounded close.
        ok(
            unsafe {
                (self.api.session_close)(
                    self.session.pointer,
                    &raw const operation,
                    ptr::null_mut(),
                )
            },
            "session_close",
        );
    }
}

pub(super) struct CPoll {
    snapshot: madopilot_template_query_snapshot_t,
    result: Option<CHandle<madopilot_template_query_result_t>>,
}

pub(super) struct CFlow {
    api: &'static madopilot_api_t,
    query: Option<CHandle<madopilot_template_query_t>>,
    result: Option<CHandle<madopilot_template_query_result_t>>,
    parents: Option<CParents>,
    wait: madopilot_operation_t,
    mapping_operation: madopilot_operation_t,
    _wait_cancellation: CHandle<madopilot_cancellation_t>,
    expected: Expected,
    pending: Option<Progress>,
}

impl CFlow {
    pub(super) fn new(case: Case) -> Self {
        let api = api();
        let pixels = match_fixtures::scene_pixels(PixelFormat::Rgba8);
        let setup = c_operation(api, SETUP_WAIT);
        let engine = c_engine(api, &pixels, case.pending(), &setup);
        let mut targets = ptr::null_mut();
        let mut session = ptr::null_mut();
        let open = madopilot_open_request_t {
            struct_size: struct_size::<madopilot_open_request_t>(),
            flags: 0,
            required_format: MADOPILOT_PIXEL_FORMAT_RGBA8,
            preferred_format: MADOPILOT_PIXEL_FORMAT_RGBA8,
        };
        // SAFETY: all handles are live owners from this table and every output
        // and request is separate, aligned stack storage for the complete call.
        unsafe {
            ok(
                (api.engine_discover)(
                    engine.pointer,
                    &raw const setup,
                    &raw mut targets,
                    ptr::null_mut(),
                ),
                "engine_discover",
            );
            let targets = CHandle::new(targets, api.target_list_release);
            ok(
                (api.session_open)(
                    engine.pointer,
                    targets.pointer,
                    0,
                    &raw const open,
                    &raw const setup,
                    &raw mut session,
                    ptr::null_mut(),
                ),
                "session_open",
            );
        }
        let parents = CParents {
            api,
            session: CHandle::new(session, api.session_release),
            engine,
        };
        let mut session_info = madopilot_session_info_t {
            struct_size: struct_size::<madopilot_session_info_t>(),
            flags: 0,
            stream: 0,
            width: 0,
            height: 0,
            format: MADOPILOT_PIXEL_FORMAT_RGBA8,
            coordinate_spaces: 0,
            target: 0,
            accepts_input: 0,
            reserved: 0,
        };
        let mut frame = ptr::null_mut();
        let mut stamp = madopilot_frame_stamp_t::cleared(struct_size::<madopilot_frame_stamp_t>());
        let root = package_root().to_string_lossy().into_owned();
        let source = madopilot_package_source_t {
            struct_size: struct_size::<madopilot_package_source_t>(),
            kind: MADOPILOT_PACKAGE_SOURCE_DIRECTORY,
            path: str_view(&root),
            archive: madopilot_bytes_t::empty(),
        };
        let mut package = ptr::null_mut();
        let mut template = ptr::null_mut();
        let mut query = ptr::null_mut();
        let mut options = madopilot_template_watch_options_t::cleared(struct_size::<
            madopilot_template_watch_options_t,
        >());
        options.change_policy = MADOPILOT_TEMPLATE_CHANGE_ANALYSIS_ALWAYS;
        if case.pending() {
            options.minimum_interval_nanos =
                u64::try_from(PENDING_INTERVAL.as_nanos()).expect("fixture interval fits");
        }
        let query_operation = c_operation(api, QUERY_LIFETIME);
        // SAFETY: all inputs are retained through the synchronous call; none of
        // their call-local pointer views are stored by a successful start.
        unsafe {
            ok(
                (api.session_describe)(parents.session.pointer, &raw mut session_info),
                "session_describe",
            );
            assert!(session_info.target != 0 && session_info.stream != 0);
            assert_eq!(
                session_info.struct_size,
                struct_size::<madopilot_session_info_t>()
            );
            ok(
                (api.session_acquire_frame)(
                    parents.session.pointer,
                    &raw const setup,
                    &raw mut frame,
                    ptr::null_mut(),
                ),
                "session_acquire_frame",
            );
            let frame = CHandle::new(frame, api.frame_release);
            ok(
                (api.frame_stamp)(frame.pointer, &raw mut stamp),
                "frame_stamp",
            );
            ok(
                (api.package_load)(
                    parents.engine.pointer,
                    &raw const source,
                    &raw const setup,
                    &raw mut package,
                    ptr::null_mut(),
                ),
                "package_load",
            );
            let package = CHandle::new(package, api.package_release);
            ok(
                (api.template_prepare_from_package)(
                    parents.engine.pointer,
                    package.pointer,
                    str_view(if case.pending() { ABSENT } else { TEMPLATE }),
                    &raw const setup,
                    &raw mut template,
                    ptr::null_mut(),
                ),
                "template_prepare_from_package",
            );
            let template = CHandle::new(template, api.template_release);
            ok(
                (api.session_start_template_watch)(
                    parents.session.pointer,
                    template.pointer,
                    &raw const options,
                    &raw const query_operation,
                    &raw mut query,
                    ptr::null_mut(),
                ),
                "session_start_template_watch",
            );
        }
        let mut cancellation = ptr::null_mut();
        // SAFETY: cancellation output is separate live storage; the resulting
        // owner stays retained for every operation that borrows it.
        unsafe {
            ok(
                (api.cancellation_create)(&raw mut cancellation),
                "cancellation_create",
            );
            assert!(!cancellation.is_null());
            ok(
                (api.cancellation_cancel)(cancellation),
                "cancellation_cancel",
            );
        }
        let cancellation = CHandle::new(cancellation, api.cancellation_release);
        let mut wait = c_operation(api, QUERY_LIFETIME);
        wait.cancellation = cancellation.pointer;
        let mut flow = Self {
            api,
            query: Some(CHandle::new(query, api.template_query_release)),
            result: None,
            parents: Some(parents),
            wait,
            _wait_cancellation: cancellation,
            mapping_operation: c_operation(api, QUERY_LIFETIME),
            expected: Expected {
                target: session_info.target,
                query_id: 0,
                stamp: Stamp::c(stamp),
                pixels,
                backend: [Vec::new(), Vec::new()],
            },
            pending: None,
        };
        if case.pending() {
            let pending = flow.prime_pending();
            flow.expected.query_id = pending.id;
            flow.pending = Some(pending);
        } else {
            let wait = c_operation(api, SETUP_WAIT);
            let mut result = ptr::null_mut();
            // SAFETY: query is owned, wait is separate finite caller authority,
            // and result is writable storage unrelated to either input.
            ok(
                unsafe {
                    (api.template_query_wait)(
                        flow.query().pointer,
                        &raw const wait,
                        &raw mut result,
                        ptr::null_mut(),
                    )
                },
                "template_query_wait",
            );
            let result = CHandle::new(result, api.template_query_result_release);
            let info = flow.info(&result);
            assert_eq!(info.outcome, MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED);
            assert!(info.query_id != 0);
            flow.expected.query_id = info.query_id;
            // SAFETY: both views were returned by info for the retained result.
            flow.expected.backend = unsafe {
                [
                    str_bytes(info.backend_id).to_vec(),
                    str_bytes(info.backend_version).to_vec(),
                ]
            };
            flow.result = Some(result.shared(api.template_query_result_retain));
            drop(result);
            assert!(flow.read_matched(flow.result()));
        }
        if case == Case::RetainedFrame {
            flow.close_parents();
            drop(flow.query.take());
        }
        flow
    }

    fn query(&self) -> &CHandle<madopilot_template_query_t> {
        self.query
            .as_ref()
            .expect("query retained for this scenario")
    }

    fn result(&self) -> &CHandle<madopilot_template_query_result_t> {
        self.result.as_ref().expect("matched result retained")
    }

    pub(super) fn first_poll(&self) -> CPoll {
        let mut snapshot = madopilot_template_query_snapshot_t::cleared(struct_size::<
            madopilot_template_query_snapshot_t,
        >());
        let mut result = ptr::null_mut();
        // SAFETY: query is retained and outputs are nonoverlapping stack values.
        ok(
            unsafe {
                (self.api.template_query_poll)(
                    self.query().pointer,
                    &raw mut snapshot,
                    &raw mut result,
                    ptr::null_mut(),
                )
            },
            "template_query_poll",
        );
        let result = (!result.is_null())
            .then(|| CHandle::new(result, self.api.template_query_result_release));
        CPoll { snapshot, result }
    }

    fn pending_progress(&self) -> Progress {
        let observed = self.first_poll();
        assert_eq!(
            observed.snapshot.state, MADOPILOT_TEMPLATE_QUERY_STATE_PENDING,
            "pending fixture terminated"
        );
        assert!(
            observed.result.is_none(),
            "pending poll must not allocate an owner"
        );
        Progress::c(observed.snapshot)
    }

    fn prime_pending(&self) -> Progress {
        let deadline = Instant::now() + SETUP_WAIT;
        loop {
            let progress = self.pending_progress();
            if progress.settled() {
                assert_eq!(
                    progress.frame,
                    Some(self.expected.stamp),
                    "the completed no-match analysis used the acquired first frame"
                );
                return progress;
            }
            assert!(
                Instant::now() < deadline,
                "no-match/rate-held setup did not settle: {progress:?}"
            );
            std::thread::yield_now();
        }
    }

    pub(super) fn close_parents(&mut self) {
        drop(self.parents.take());
    }

    fn info(
        &self,
        result: &CHandle<madopilot_template_query_result_t>,
    ) -> madopilot_template_query_result_info_t {
        let mut info = madopilot_template_query_result_info_t::cleared(struct_size::<
            madopilot_template_query_result_info_t,
        >());
        // SAFETY: result is retained; info is aligned, independent writable data.
        ok(
            unsafe { (self.api.template_query_result_info)(result.pointer, &raw mut info) },
            "template_query_result_info",
        );
        assert_eq!(
            info.struct_size,
            struct_size::<madopilot_template_query_result_info_t>()
        );
        info
    }

    pub(super) fn closed_outcome(&self, observed: &CPoll) -> bool {
        let Some(result) = &observed.result else {
            return false;
        };
        let info = self.info(result);
        let mut failure = ptr::null_mut();
        // SAFETY: result is retained and failure is separate writable storage.
        ok(
            unsafe { (self.api.template_query_result_error)(result.pointer, &raw mut failure) },
            "template_query_result_error",
        );
        let no_failure = failure.is_null();
        if !no_failure {
            drop(CHandle::new(failure, self.api.error_release));
        }
        observed.snapshot.state == MADOPILOT_TEMPLATE_QUERY_STATE_TERMINAL
            && Progress::c(observed.snapshot)
                .terminal(self.pending.expect("query identity before close").id)
            && info.query_id == observed.snapshot.query_id
            && info.outcome == MADOPILOT_TEMPLATE_QUERY_OUTCOME_SESSION_CLOSED
            && info.status == MADOPILOT_STATUS_CLOSED
            && info.overload == MADOPILOT_TEMPLATE_OVERLOAD_NONE
            && info.target == 0
            && info.match_count == 0
            && info.confirmed_observations == 0
            && info.confirmed_duration_nanos == 0
            && Stamp::c(info.source) == Stamp([0; 4])
            && info.template_id.data.is_null()
            && info.template_id.len == 0
            && info.backend_id.data.is_null()
            && info.backend_id.len == 0
            && info.backend_version.data.is_null()
            && info.backend_version.len == 0
            && no_failure
    }

    pub(super) fn create_cancel_release() -> Exercise {
        let mut flow = Self::new(Case::CreateCancelRelease);
        let mut result = ptr::null_mut();
        // SAFETY: the query is retained; the result output is independent,
        // writable storage. The returned reference uses this table's releaser.
        ok(
            unsafe {
                (flow.api.template_query_cancel)(
                    flow.query().pointer,
                    &raw mut result,
                    ptr::null_mut(),
                )
            },
            "template_query_cancel",
        );
        let result = CHandle::new(result, flow.api.template_query_result_release);
        let observed = flow.first_poll();
        let correct = observed.snapshot.state == MADOPILOT_TEMPLATE_QUERY_STATE_TERMINAL
            && Progress::c(observed.snapshot).terminal(flow.expected.query_id)
            && observed
                .result
                .as_ref()
                .is_some_and(|result| flow.cancelled_result(result))
            && flow.cancelled_result(&result);
        drop(observed);
        drop(flow.query.take());
        flow.close_parents();
        let correct = correct && flow.cancelled_result(&result);
        drop(result);
        drop(flow);
        Exercise {
            correct,
            readable_view: None,
        }
    }

    fn cancelled_result(&self, result: &CHandle<madopilot_template_query_result_t>) -> bool {
        let info = self.info(result);
        let mut failure = ptr::null_mut();
        // SAFETY: the result remains owned and failure is a separate output.
        ok(
            unsafe { (self.api.template_query_result_error)(result.pointer, &raw mut failure) },
            "template_query_result_error",
        );
        let no_failure = failure.is_null();
        if !no_failure {
            drop(CHandle::new(failure, self.api.error_release));
        }
        info.query_id == self.expected.query_id
            && info.outcome == MADOPILOT_TEMPLATE_QUERY_OUTCOME_CANCELLED
            && info.status == MADOPILOT_STATUS_CANCELLED
            && info.overload == MADOPILOT_TEMPLATE_OVERLOAD_NONE
            && info.target == 0
            && info.match_count == 0
            && info.confirmed_observations == 0
            && info.confirmed_duration_nanos == 0
            && Stamp::c(info.source) == Stamp([0; 4])
            && info.template_id.data.is_null()
            && info.template_id.len == 0
            && info.backend_id.data.is_null()
            && info.backend_id.len == 0
            && info.backend_version.data.is_null()
            && info.backend_version.len == 0
            && no_failure
    }

    fn read_matched(&self, result: &CHandle<madopilot_template_query_result_t>) -> bool {
        let info = self.info(result);
        let transform = info.transform;
        let mut correct = info.outcome == MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED
            && info.status == MADOPILOT_STATUS_OK
            && info.overload == MADOPILOT_TEMPLATE_OVERLOAD_NONE
            && info.query_id == self.expected.query_id
            && info.target == self.expected.target
            && Stamp::c(info.source) == self.expected.stamp
            && info.match_count == PLANTED.len() as u64
            && info.confirmed_observations == 1
            && info.confirmed_duration_nanos == 0
            && info.options.struct_size == struct_size::<madopilot_match_options_t>()
            && info.options.flags
                == (MADOPILOT_MATCH_HAS_MIN_SCORE
                    | MADOPILOT_MATCH_HAS_MAX_RESULTS
                    | MADOPILOT_MATCH_HAS_SUPPRESSION)
            && info.options.min_score == 0.9
            && info.options.max_results == 8
            && info.options.suppression == MADOPILOT_SUPPRESSION_DROP_OVERLAPPING
            && rect(info.effective_region) == BOUNDS
            && transform.struct_size == struct_size::<madopilot_transform_snapshot_t>()
            && transform.geometry == self.expected.stamp.0[3]
            && transform.width == match_fixtures::SCENE.width()
            && transform.height == match_fixtures::SCENE.height()
            && transform.flags == MADOPILOT_TRANSFORM_COVERS_TARGET
            && [
                transform.desktop_origin_x,
                transform.desktop_origin_y,
                transform.logical_width,
                transform.logical_height,
                transform.target_scale_x,
                transform.target_scale_y,
                transform.desktop_scale_x,
                transform.desktop_scale_y,
            ] == [0.0; 8];
        // SAFETY: the info and match views borrow this still-retained result.
        unsafe {
            correct &= str_bytes(info.template_id) == TEMPLATE.as_bytes()
                && str_bytes(info.backend_id) == self.expected.backend[0]
                && str_bytes(info.backend_version) == self.expected.backend[1]
                && !self.expected.backend[0].is_empty()
                && !self.expected.backend[1].is_empty();
            let mut seen = 0_u8;
            // Two fixed initialized outputs: never allocate a Vec from a foreign
            // count or let a malformed count turn this workload into an unbounded loop.
            for index in 0..PLANTED.len() {
                let mut one = madopilot_match_t {
                    struct_size: struct_size::<madopilot_match_t>(),
                    flags: 0,
                    score: 0.0,
                    template_id: madopilot_str_t::empty(),
                    bounds: madopilot_pixel_rect_t::empty(),
                };
                ok(
                    (self.api.template_query_result_match_at)(result.pointer, index, &raw mut one),
                    "template_query_result_match_at",
                );
                correct &= one.struct_size == struct_size::<madopilot_match_t>()
                    && one.flags == 0
                    && str_bytes(one.template_id) == TEMPLATE.as_bytes();
                correct &= planted(rect(one.bounds), one.score, &mut seen);
            }
            correct &= seen == 3;
        }
        correct
    }

    fn retained_frame(&self) -> (bool, usize) {
        let mut frame = ptr::null_mut();
        let mut stamp = madopilot_frame_stamp_t::cleared(struct_size::<madopilot_frame_stamp_t>());
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
        let request = madopilot_map_request_t {
            struct_size: struct_size::<madopilot_map_request_t>(),
            flags: 0,
            format: MADOPILOT_PIXEL_FORMAT_RGBA8,
            clip_policy: MADOPILOT_CLIP_POLICY_REJECT,
            region: madopilot_pixel_rect_t::empty(),
        };
        let mut mapping = ptr::null_mut();
        let mut image = madopilot_image_t {
            struct_size: struct_size::<madopilot_image_t>(),
            flags: 0,
            width: 0,
            height: 0,
            format: MADOPILOT_PIXEL_FORMAT_RGBA8,
            space: MADOPILOT_SPACE_CAPTURE_PIXELS,
            stride: 0,
            bytes: madopilot_bytes_t::empty(),
            region: madopilot_pixel_rect_t::empty(),
        };
        // SAFETY: no parent is used; the result, then returned frame, then mapping
        // each own independent references. The byte view is consumed before its
        // mapping owner drops. Every output is separate writable stack storage.
        unsafe {
            ok(
                (self.api.template_query_result_frame)(self.result().pointer, &raw mut frame),
                "template_query_result_frame",
            );
            let frame = CHandle::new(frame, self.api.frame_release);
            ok(
                (self.api.frame_stamp)(frame.pointer, &raw mut stamp),
                "frame_stamp",
            );
            let source = Stamp::c(stamp);
            ok(
                (self.api.frame_describe)(frame.pointer, &raw mut info),
                "frame_describe",
            );
            ok(
                (self.api.frame_map)(
                    frame.pointer,
                    &raw const request,
                    &raw const self.mapping_operation,
                    &raw mut mapping,
                    ptr::null_mut(),
                ),
                "frame_map",
            );
            let mapping = CHandle::new(mapping, self.api.mapping_release);
            ok(
                (self.api.mapping_describe)(mapping.pointer, &raw mut image),
                "mapping_describe",
            );
            ok(
                (self.api.mapping_stamp)(mapping.pointer, &raw mut stamp),
                "mapping_stamp",
            );
            assert!(!image.bytes.data.is_null());
            let bytes = std::slice::from_raw_parts(image.bytes.data, image.bytes.len);
            let correct = self.parents.is_none()
                && self.query.is_none()
                && source == self.expected.stamp
                && Stamp::c(stamp) == self.expected.stamp
                && info.struct_size == struct_size::<madopilot_frame_info_t>()
                && info.flags == 0
                && info.width == match_fixtures::SCENE.width()
                && info.height == match_fixtures::SCENE.height()
                && info.format == MADOPILOT_PIXEL_FORMAT_RGBA8
                && info.space == MADOPILOT_SPACE_CAPTURE_PIXELS
                && rect(info.bounds) == BOUNDS
                && image.struct_size == struct_size::<madopilot_image_t>()
                && image.flags == MADOPILOT_IMAGE_SHARED
                && image.width == info.width
                && image.height == info.height
                && image.format == info.format
                && image.space == info.space
                && image.stride == info.stride
                && image.stride == u64::from(info.width) * 4
                && rect(image.region) == BOUNDS
                && bytes == self.expected.pixels
                && self.read_matched(self.result());
            (correct, bytes.len())
        }
    }

    pub(super) fn exercise(&self, case: Case) -> Exercise {
        let mut correct = true;
        let mut readable_view = None;
        for _ in 0..BATCH {
            let flow = std::hint::black_box(self);
            correct &= std::hint::black_box(match case {
                Case::PendingPoll => {
                    flow.pending_progress() == flow.pending.expect("settled snapshot")
                }
                Case::TerminalPoll => {
                    let observed = flow.first_poll();
                    observed.snapshot.state == MADOPILOT_TEMPLATE_QUERY_STATE_TERMINAL
                        && observed.result.as_ref().is_some_and(|result| {
                            Progress::c(observed.snapshot).terminal(flow.expected.query_id)
                                && flow.read_matched(result)
                        })
                }
                Case::TerminalRead => flow.read_matched(flow.result()),
                Case::WaitCancelled => {
                    let mut result = ptr::null_mut();
                    // SAFETY: wait cancellation is owned independently of query;
                    // the valid output is initialized even on caller interruption.
                    let status = unsafe {
                        (flow.api.template_query_wait)(
                            flow.query().pointer,
                            &raw const flow.wait,
                            &raw mut result,
                            ptr::null_mut(),
                        )
                    };
                    let no_result = result.is_null();
                    if !no_result {
                        drop(CHandle::new(result, flow.api.template_query_result_release));
                    }
                    status == MADOPILOT_STATUS_CANCELLED
                        && no_result
                        && flow.pending_progress() == flow.pending.expect("settled snapshot")
                }
                Case::QueryClone => {
                    let query = flow.query().shared(flow.api.template_query_retain);
                    drop(query);
                    flow.pending_progress() == flow.pending.expect("settled snapshot")
                }
                Case::ResultClone => {
                    let result = flow.result().shared(flow.api.template_query_result_retain);
                    let readable = flow.read_matched(&result);
                    drop(result);
                    readable && flow.read_matched(flow.result())
                }
                Case::RetainedFrame => {
                    let (readable, bytes) = flow.retained_frame();
                    readable_view = Some(bytes);
                    readable
                }
                Case::FirstTerminal | Case::CreateCancelRelease => {
                    unreachable!("lifecycle observation has per-sample setup")
                }
            });
        }
        Exercise {
            correct,
            readable_view,
        }
    }
}

fn planted(bounds: [i32; 4], score: f64, seen: &mut u8) -> bool {
    let Some(index) = PLANTED.iter().position(|expected| *expected == bounds) else {
        return false;
    };
    let bit = 1 << index;
    let unique = *seen & bit == 0;
    *seen |= bit;
    unique && score.is_finite() && (0.0..=1.0).contains(&score) && (score - 1.0).abs() <= 1e-5
}

fn rect(rect: madopilot_pixel_rect_t) -> [i32; 4] {
    assert_eq!(rect.space, MADOPILOT_SPACE_CAPTURE_PIXELS);
    [rect.left, rect.top, rect.right, rect.bottom]
}

/// # Safety
/// `view` must come from a successful accessor, and its named owner must remain
/// retained for the entire returned borrow. No byte view escapes a fixture call.
unsafe fn str_bytes<'a>(view: madopilot_str_t) -> &'a [u8] {
    if view.len == 0 {
        return &[];
    }
    assert!(
        !view.data.is_null(),
        "nonempty borrowed view must have storage"
    );
    // SAFETY: the caller keeps the result that owns this validated view alive.
    unsafe { std::slice::from_raw_parts(view.data.cast::<u8>(), view.len) }
}

fn str_view(value: &str) -> madopilot_str_t {
    madopilot_str_t {
        data: value.as_ptr().cast(),
        len: value.len(),
    }
}

fn ok(status: madopilot_status_t, call: &str) {
    assert_eq!(status, MADOPILOT_STATUS_OK, "{call}");
}

fn api() -> &'static madopilot_api_t {
    let mut api = ptr::null();
    // SAFETY: the table output is aligned writable storage. This binary links
    // the library for its lifetime and asks for the complete current extent.
    ok(
        unsafe {
            madopilot_get_api(
                MADOPILOT_ABI_MAJOR,
                MADOPILOT_ABI_MINOR,
                size_of::<madopilot_api_t>(),
                &raw mut api,
            )
        },
        "madopilot_get_api",
    );
    // SAFETY: successful negotiation returns the statically linked table.
    let api = unsafe { api.as_ref() }.expect("negotiated table");
    assert_eq!(api.abi_major, MADOPILOT_ABI_MAJOR);
    assert!(api.abi_minor >= 6);
    assert!(
        usize::try_from(api.struct_size).expect("table size fits") >= size_of::<madopilot_api_t>()
    );
    api
}

fn c_operation(api: &madopilot_api_t, timeout: Duration) -> madopilot_operation_t {
    let mut now = 0;
    // SAFETY: now is live aligned writable scalar storage.
    ok(unsafe { (api.clock_now)(&raw mut now) }, "clock_now");
    madopilot_operation_t {
        struct_size: struct_size::<madopilot_operation_t>(),
        flags: MADOPILOT_OPERATION_HAS_DEADLINE,
        deadline_nanos: now
            .checked_add(u64::try_from(timeout.as_nanos()).expect("fixture timeout fits"))
            .expect("fixture deadline fits"),
        cancellation: ptr::null(),
        activity_tag: 0,
    }
}

fn c_engine(
    api: &madopilot_api_t,
    pixels: &[u8],
    pending: bool,
    operation: &madopilot_operation_t,
) -> CHandle<madopilot_engine_t> {
    let first = madopilot_replay_frame_t {
        struct_size: struct_size::<madopilot_replay_frame_t>(),
        flags: 0,
        width: match_fixtures::SCENE.width(),
        height: match_fixtures::SCENE.height(),
        format: MADOPILOT_PIXEL_FORMAT_RGBA8,
        continuity: MADOPILOT_CONTINUITY_CONTINUOUS,
        pixels: madopilot_bytes_t {
            data: pixels.as_ptr(),
            len: pixels.len(),
        },
        captured_at_nanos: 0,
        stride: 0,
    };
    let frames = [
        first,
        madopilot_replay_frame_t {
            captured_at_nanos: 1,
            ..first
        },
    ];
    let source = madopilot_source_t {
        struct_size: struct_size::<madopilot_source_t>(),
        kind: MADOPILOT_SOURCE_REPLAY_MEMORY,
        directory: madopilot_str_t::empty(),
        frames: frames.as_ptr(),
        frame_count: if pending { 2 } else { 1 },
        frame_stride: size_of::<madopilot_replay_frame_t>(),
        target_name: str_view("panel"),
    };
    let mut engine = ptr::null_mut();
    // SAFETY: this call-local source and frame array never move after pointers
    // are bound; both and the caller's pixels outlive synchronous engine_create.
    ok(
        unsafe {
            (api.engine_create)(
                &raw const source,
                operation,
                &raw mut engine,
                ptr::null_mut(),
            )
        },
        "engine_create",
    );
    CHandle::new(engine, api.engine_release)
}
