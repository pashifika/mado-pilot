//! Shared production-session orchestration for native template-watch qualification.

#[cfg(target_os = "macos")]
include!("native_template_watch_macos.rs");
#[cfg(windows)]
include!("native_template_watch_windows.rs");

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use mado_pilot::{
    ChangeDetectionPolicy, CoordinateSpace, Engine, Frame, FrameOrder, FrameRequest, FrameStamp,
    MatchDefaults, MatchOptions, NativeEngineRequest, OpenRequest, OperationContext, PixelExtent,
    PixelFormat, Point, PreparedTemplate, Session, Status, TargetId, TargetKind,
    TemplateAnalysisRate, TemplateEncoding, TemplateId, TemplateOverload, TemplateQuery,
    TemplateQueryOutcome, TemplateQueryProgress, TemplateSchedulerDescriptor, TemplateSource,
    TemplateSourceRequest, TemplateStability, TemplateTerminalOutcome, TemplateWatchRequest,
    TemplateWorkDisposition,
};
use mado_pilot_backend_opencv::benchmark_instrumentation::{
    Snapshot as BackendSnapshot, install_find_delay, snapshot as backend_snapshot,
};
use mado_pilot_testkit::bench_harness::{
    self, Benchmark, Plan, Profile, QueryWorkMetrics, Sample, Workload, measure,
};
use mado_pilot_testkit::{ManualClock, native_watch_report, png};

const OPERATION_WAIT: Duration = Duration::from_secs(5);
const FIXTURE_WAIT: Duration = Duration::from_secs(10);
const FIXTURE_COMMAND_WAIT: Duration = Duration::from_secs(2);
const POLL_WAIT: Duration = Duration::from_millis(5);
const STATIC_STABILITY: Duration = Duration::from_millis(25);
const SLOW_BACKEND: Duration = Duration::from_millis(150);
const MARKER_CELL_LOGICAL: f64 = 24.0;
const MARKER_X_LOGICAL: f64 = 64.0;
const MARKER_Y_LOGICAL: f64 = 48.0;
const MARKER_PRIMARY: [u8; 3] = [0xf2, 0x6b, 0x38];
const MARKER_SECONDARY: [u8; 3] = [0x2d, 0xd4, 0xbf];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ControlAcknowledgement {
    generation: u64,
    revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MarkerShape {
    cell_width: u32,
    cell_height: u32,
    origin_x: i32,
    origin_y: i32,
}

impl MarkerShape {
    const fn extent(self) -> PixelExtent {
        PixelExtent::new(self.cell_width * 3, self.cell_height * 2)
    }
}

fn scaled_u32(value: f64, scale: f64) -> Option<u32> {
    let scaled = (value * scale).round();
    if !scaled.is_finite() || scaled < 1.0 || scaled > f64::from(u32::MAX) {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "finite rounded value was range-checked above"
    )]
    Some(scaled as u32)
}

fn scaled_i32(value: f64, scale: f64) -> Option<i32> {
    let scaled = (value * scale).round();
    if !scaled.is_finite() || scaled < f64::from(i32::MIN) || scaled > f64::from(i32::MAX) {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "finite rounded value was range-checked above"
    )]
    Some(scaled as i32)
}

fn wait_marker_state(run: &NativeRun, after: FrameStamp, visible: bool) -> Result<Frame, String> {
    let deadline = Instant::now() + OPERATION_WAIT;
    let mut stamp = after;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("fixture_authority_failed".to_owned());
        }
        let frame = run
            .session
            .acquire_frame(&FrameRequest::newer_than(stamp), &bounded(remaining))
            .map_err(|_| "typed_operation_failure:DeadlineExceeded".to_owned())?;
        let shape =
            marker_shape(&frame, &run.fixture).ok_or_else(|| "wrong_transform".to_owned())?;
        let mapping = run
            .session
            .map_frame(&frame, PixelFormat::Rgba8, &bounded(remaining))
            .map_err(|_| "wrong_region".to_owned())?;
        if marker_state(&mapping, shape) == Some(visible) {
            return Ok(frame);
        }
        stamp = frame.stamp();
    }
}

fn establish_absent(run: &mut NativeRun) -> Result<Frame, String> {
    let prior = run
        .session
        .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
        .map_err(|_| "wrong_source".to_owned())?
        .stamp();
    run.command_visible()?;
    let visible = wait_marker_state(run, prior, true)?;
    run.command_absent()?;
    wait_marker_state(run, visible.stamp(), false)
}

fn settle_absent(run: &mut NativeRun) -> Result<mado_pilot::CpuMapping, String> {
    let absent = establish_absent(run)?;
    let shape = marker_shape(&absent, &run.fixture).ok_or_else(|| "wrong_transform".to_owned())?;
    let mapping = run
        .session
        .map_frame(&absent, PixelFormat::Rgba8, &bounded(OPERATION_WAIT))
        .map_err(|_| "wrong_region".to_owned())?;
    if marker_state(&mapping, shape) != Some(false) {
        return Err("fixture_authority_failed".to_owned());
    }
    Ok(mapping)
}

fn wait_geometry_change(run: &NativeRun, after: FrameStamp) -> Result<Frame, String> {
    let deadline = Instant::now() + OPERATION_WAIT;
    let original = (after.epoch(), after.geometry());
    let mut changed = None;
    let mut stamp = after;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("wrong_transform".to_owned());
        }
        let frame = run
            .session
            .acquire_frame(&FrameRequest::newer_than(stamp), &bounded(remaining))
            .map_err(|_| "typed_operation_failure:DeadlineExceeded".to_owned())?;
        let current = (frame.stamp().epoch(), frame.stamp().geometry());
        if current != original {
            if changed == Some(current) {
                return Ok(frame);
            }
            changed = Some(current);
        }
        stamp = frame.stamp();
    }
}

fn wait_resize_change(run: &NativeRun, before: &Frame) -> Result<Frame, String> {
    let deadline = Instant::now() + OPERATION_WAIT;
    let original = (before.stamp().epoch(), before.stamp().geometry());
    let mut confirmed = None;
    let mut stamp = before.stamp();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("wrong_transform".to_owned());
        }
        let frame = run
            .session
            .acquire_frame(&FrameRequest::newer_than(stamp), &bounded(remaining))
            .map_err(|_| "typed_operation_failure:DeadlineExceeded".to_owned())?;
        let current = (frame.stamp().epoch(), frame.stamp().geometry());
        if current != original && resize_geometry_matches(before, &frame) {
            if confirmed == Some(current) {
                return Ok(frame);
            }
            confirmed = Some(current);
        } else {
            confirmed = None;
        }
        stamp = frame.stamp();
    }
}
fn marker_state(mapping: &mado_pilot::CpuMapping, shape: MarkerShape) -> Option<bool> {
    let stride = mapping.descriptor().stride();
    let bytes = mapping.bytes();
    let mut cells = [[0_u8; 3]; 6];
    for row in 0..2 {
        for column in 0..3 {
            let x = shape
                .origin_x
                .checked_add(i32::try_from(column * shape.cell_width).ok()?)?
                .checked_add(i32::try_from(shape.cell_width / 2).ok()?)?;
            let y = shape
                .origin_y
                .checked_add(i32::try_from(row * shape.cell_height).ok()?)?
                .checked_add(i32::try_from(shape.cell_height / 2).ok()?)?;
            let x = usize::try_from(x).ok()?;
            let y = usize::try_from(y).ok()?;
            let offset = y.checked_mul(stride)?.checked_add(x.checked_mul(4)?)?;
            let pixel = bytes.get(offset..offset.checked_add(3)?)?;
            cells[usize::try_from(row * 3 + column).ok()?].copy_from_slice(pixel);
        }
    }
    let same_color = |left: [u8; 3], right: [u8; 3]| {
        left.into_iter()
            .zip(right)
            .all(|(left, right)| left.abs_diff(right) <= 8)
    };
    if cells.iter().copied().all(|cell| same_color(cell, cells[0])) {
        return Some(false);
    }
    let primary = cells[0];
    let secondary = cells[1];
    let separated = primary
        .into_iter()
        .zip(secondary)
        .any(|(left, right)| left.abs_diff(right) >= 32);
    let visible = [0, 2, 4, 5]
        .into_iter()
        .all(|index| same_color(cells[index], primary))
        && [1, 3]
            .into_iter()
            .all(|index| same_color(cells[index], secondary))
        && separated;
    visible.then_some(true)
}

#[derive(Debug, Clone)]
struct Arguments {
    fixture_executable: PathBuf,
    raw: Vec<String>,
    qualification: bool,
    enforce_budgets: bool,
    workload_filter: Option<String>,
}

impl Arguments {
    fn parse() -> Self {
        let raw = std::env::args().skip(1).collect::<Vec<_>>();
        let fixture_executable = value(&raw, "--fixture-executable")
            .map(PathBuf::from)
            .or_else(default_fixture_executable)
            .unwrap_or_else(|| panic!("capability_unavailable:fixture_executable"));
        assert!(
            fixture_executable.is_file(),
            "capability_unavailable:fixture_executable"
        );
        let qualification = raw.iter().any(|argument| argument == "--bench");
        let workload_filter = value(&raw, "--workload").map(str::to_owned);
        assert!(
            !qualification || workload_filter.is_none(),
            "protocol_drift"
        );
        Self {
            qualification,
            enforce_budgets: raw.iter().any(|argument| argument == "--enforce-budgets"),
            workload_filter,
            fixture_executable,
            raw,
        }
    }
}

fn value<'a>(arguments: &'a [String], name: &str) -> Option<&'a str> {
    let prefix = format!("{name}=");
    arguments
        .iter()
        .find_map(|argument| argument.strip_prefix(&prefix))
}

fn default_fixture_executable() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        Some(PathBuf::from(
            "target/mado-pilot-fixtures/MadoPilotWatchFixture.app/Contents/MacOS/mado-pilot-macos-input-fixture",
        ))
    }
    #[cfg(windows)]
    {
        std::env::var_os("MADOPILOT_WINDOWS_WATCH_FIXTURE").map(PathBuf::from)
    }
}

struct NativeRun {
    fixture: NativeFixture,
    engine: Engine,
    target: TargetId,
    session: Session,
    template: PreparedTemplate,
    shape: MarkerShape,
    last_ack: ControlAcknowledgement,
}

impl NativeRun {
    fn start(arguments: &Arguments) -> Result<Self, String> {
        let fixture = NativeFixture::start(arguments)?;
        let engine = native_engine().map_err(|_| "capability_unavailable:capture".to_owned())?;
        let target = fixture.authenticated_target(&engine)?;
        let session = engine
            .open(target, &OpenRequest::new(), &bounded(OPERATION_WAIT))
            .map_err(|error| format!("typed_operation_failure:{:?}", error.status()))?;
        let frame = session
            .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
            .map_err(|error| format!("typed_operation_failure:{:?}", error.status()))?;
        let shape = marker_shape(&frame, &fixture).ok_or_else(|| "wrong_transform".to_owned())?;
        let template = prepare_marker(&engine, shape, "watch-marker-v1")?;
        Ok(Self {
            fixture,
            engine,
            target,
            session,
            template,
            shape,
            last_ack: ControlAcknowledgement {
                generation: 1,
                revision: 0,
            },
        })
    }

    fn command_absent(&mut self) -> Result<ControlAcknowledgement, String> {
        let acknowledgement = self.fixture.set_absent()?;
        self.accept_acknowledgement(acknowledgement)?;
        Ok(acknowledgement)
    }

    fn command_visible(&mut self) -> Result<ControlAcknowledgement, String> {
        let acknowledgement = self.fixture.set_visible()?;
        self.accept_acknowledgement(acknowledgement)?;
        Ok(acknowledgement)
    }

    fn accept_acknowledgement(
        &mut self,
        acknowledgement: ControlAcknowledgement,
    ) -> Result<(), String> {
        if acknowledgement.generation != self.last_ack.generation
            || acknowledgement.revision <= self.last_ack.revision
        {
            return Err("fixture_authority_failed".to_owned());
        }
        self.last_ack = acknowledgement;
        Ok(())
    }

    fn start_watch(&self, stability: TemplateStability) -> Result<TemplateQuery, String> {
        self.start_watch_with(self.template.clone(), stability, OperationContext::new())
    }

    fn start_watch_with(
        &self,
        template: PreparedTemplate,
        stability: TemplateStability,
        operation: OperationContext,
    ) -> Result<TemplateQuery, String> {
        let options = MatchOptions::from_defaults(template.defaults());
        self.session
            .start_template_watch(
                TemplateWatchRequest::new(template, options, operation)
                    .with_stability(stability)
                    .with_change_policy(ChangeDetectionPolicy::default()),
            )
            .map_err(|error| format!("typed_operation_failure:{:?}", error.status()))
    }

    fn acquire_newer(&self, stamp: FrameStamp) -> Result<Frame, String> {
        self.session
            .acquire_frame(&FrameRequest::newer_than(stamp), &bounded(OPERATION_WAIT))
            .map_err(|error| format!("typed_operation_failure:{:?}", error.status()))
    }

    fn refresh_template(&mut self, frame: &Frame, id: &str) -> Result<(), String> {
        self.shape =
            marker_shape(frame, &self.fixture).ok_or_else(|| "wrong_transform".to_owned())?;
        self.template = prepare_marker(&self.engine, self.shape, id)?;
        Ok(())
    }

    fn close(mut self) -> bool {
        let session_closed = self.session.close(&bounded(OPERATION_WAIT)).is_ok()
            && self.session.close(&bounded(OPERATION_WAIT)).is_ok();
        drop(self.engine);
        session_closed && self.fixture.finish()
    }
}

struct Cohort {
    arguments: Arguments,
    run: Option<NativeRun>,
    settled_mapping: Option<mado_pilot::CpuMapping>,
}

impl Cohort {
    fn new(arguments: Arguments) -> Self {
        Self {
            arguments,
            run: None,
            settled_mapping: None,
        }
    }

    fn run(&mut self) -> Result<&mut NativeRun, String> {
        if self.run.is_none() {
            self.run = Some(NativeRun::start(&self.arguments)?);
        }
        self.run.as_mut().ok_or_else(|| "cleanup_failed".to_owned())
    }

    fn fresh(&self) -> Result<NativeRun, String> {
        NativeRun::start(&self.arguments)
    }

    fn settle(&mut self) -> Result<(), String> {
        let mapping = {
            let run = self.run()?;
            run.session
                .benchmark_wait_template_watcher_idle(&bounded(OPERATION_WAIT))
                .map_err(|error| format!("typed_operation_failure:{:?}", error.status()))?;
            settle_absent(run)?
        };
        self.settled_mapping = Some(mapping);
        Ok(())
    }

    fn finish(&mut self) -> bool {
        self.settled_mapping = None;
        self.run.take().is_none_or(NativeRun::close)
    }
}

pub(super) fn run() {
    let arguments = Arguments::parse();
    let sample_plan = if arguments.qualification {
        Plan::new(3, 20)
    } else {
        Plan::new(1, 1)
    };
    let gate_plan = Plan::new(0, 1);
    let cohort = Rc::new(RefCell::new(Cohort::new(arguments.clone())));
    let mut workloads = Vec::with_capacity(24);

    add_workload(
        &mut workloads,
        "environment_identity",
        "facade/native/fixture identities, availability, and owner binding are exact",
        gate_plan,
        &cohort,
        environment_identity,
    );
    add_workload(
        &mut workloads,
        "window_absent_current",
        "acknowledged absent current frame remains pending without false stability",
        sample_plan,
        &cohort,
        window_absent_current,
    );
    add_workload(
        &mut workloads,
        "window_transient_appearance",
        "one confirmed visible frame followed by acknowledged absence remains nonterminal",
        sample_plan,
        &cohort,
        window_transient_appearance,
    );
    add_workload(
        &mut workloads,
        "window_persistent_appearance",
        "persistent marker completes only after confirmed duration on authoritative frames",
        sample_plan,
        &cohort,
        window_persistent_appearance,
    );
    add_workload(
        &mut workloads,
        "window_disappearance_reset",
        "absence resets prior confirmation before persistent reappearance",
        sample_plan,
        &cohort,
        window_disappearance_reset,
    );
    add_workload(
        &mut workloads,
        "window_strictly_newer",
        "completion excludes the named absent source and returns a strictly newer frame",
        sample_plan,
        &cohort,
        window_strictly_newer,
    );
    add_workload(
        &mut workloads,
        "window_move",
        "movement changes geometry authority and only post-move work may commit",
        sample_plan,
        &cohort,
        window_move,
    );
    add_workload(
        &mut workloads,
        "window_resize",
        "resize changes extent and geometry before a later exact match commits",
        sample_plan,
        &cohort,
        window_resize,
    );
    add_workload(
        &mut workloads,
        "window_topology_scale",
        "target scale/topology transition selects the exact new marker realization",
        gate_plan,
        &cohort,
        window_topology_scale,
    );
    add_workload(
        &mut workloads,
        "display_current_newer",
        "one owned-fixture display session finds the marker on a strictly newer source",
        gate_plan,
        &cohort,
        display_current_newer,
    );
    add_workload(
        &mut workloads,
        "permission_availability",
        "non-prompting native capability facts are available before capture work",
        gate_plan,
        &cohort,
        permission_availability,
    );
    add_workload(
        &mut workloads,
        "native_high_rate_slow_backend",
        "controlled slow production OpenCV work stays finite and producer publication advances",
        sample_plan,
        &cohort,
        native_high_rate_slow_backend,
    );
    add_workload(
        &mut workloads,
        "two_query_fairness",
        "two eligible queries both receive a terminal result without thread-order assumptions",
        sample_plan,
        &cohort,
        two_query_fairness,
    );
    add_workload(
        &mut workloads,
        "two_session_fairness",
        "two maintained production sessions independently complete",
        sample_plan,
        &cohort,
        two_session_fairness,
    );
    add_workload(
        &mut workloads,
        "exact_coalescing",
        "equal immutable query facts share exactly one backend execution",
        sample_plan,
        &cohort,
        exact_coalescing,
    );
    add_workload(
        &mut workloads,
        "unequal_no_coalescing",
        "distinct preparation instances execute independent backend work",
        sample_plan,
        &cohort,
        unequal_no_coalescing,
    );
    add_workload(
        &mut workloads,
        "queue_expiry_overload",
        "finite scheduler overload is observable and no query is silently lost",
        gate_plan,
        &cohort,
        queue_expiry_overload,
    );
    add_workload(
        &mut workloads,
        "stale_generation",
        "pre-geometry backend completion cannot commit after generation change",
        sample_plan,
        &cohort,
        stale_generation,
    );
    add_workload(
        &mut workloads,
        "wait_cancel_deadline",
        "caller wait, query cancellation, and query deadline retain separate authority",
        sample_plan,
        &cohort,
        wait_cancel_deadline,
    );
    add_workload(
        &mut workloads,
        "native_stop_target_loss",
        "owned target close terminates pending native watcher with target loss",
        gate_plan,
        &cohort,
        native_stop_target_loss,
    );
    add_workload(
        &mut workloads,
        "session_engine_close",
        "session and engine close are idempotent and wake pending queries exactly once",
        gate_plan,
        &cohort,
        session_engine_close,
    );
    add_workload(
        &mut workloads,
        "retained_result_mapping",
        "retained result/frame/mapping survive parent release without stopping a fresh producer",
        sample_plan,
        &cohort,
        retained_result_mapping,
    );
    add_workload(
        &mut workloads,
        "fresh_session",
        "a fresh production session watches successfully after predecessor teardown",
        sample_plan,
        &cohort,
        fresh_session,
    );
    add_workload(
        &mut workloads,
        "producer_progress_cleanup_privacy",
        "producer progress, bounded cleanup, and allowlisted aggregate output all pass",
        gate_plan,
        &cohort,
        producer_progress_cleanup_privacy,
    );

    let cleanup_ok = cohort.borrow_mut().finish();
    assert!(cleanup_ok, "cleanup_failed");
    let expected_workloads = if arguments.workload_filter.is_some() {
        1
    } else {
        24
    };
    assert_eq!(workloads.len(), expected_workloads, "protocol_drift");
    assert!(
        workloads.iter().all(|workload| workload.incorrect() == 0),
        "semantic_oracle_failed: {}",
        workloads
            .iter()
            .filter(|workload| workload.incorrect() != 0)
            .map(|workload| workload.name())
            .collect::<Vec<_>>()
            .join(",")
    );

    if arguments.qualification {
        report(&arguments, sample_plan, &workloads);
    } else {
        bench_harness::summarize("native-template-watch", sample_plan, &workloads);
    }
    for workload in &workloads {
        if sampled_workload(workload.name()) {
            bench_harness::enforce_hard_budgets(std::slice::from_ref(workload));
        }
    }
    if arguments.enforce_budgets {
        enforce_accepted_budgets(&workloads);
    }
}

fn add_workload(
    workloads: &mut Vec<Workload>,
    name: &'static str,
    oracle: &'static str,
    plan: Plan,
    cohort: &Rc<RefCell<Cohort>>,
    operation: fn(&Rc<RefCell<Cohort>>) -> Sample,
) {
    if cohort
        .borrow()
        .arguments
        .workload_filter
        .as_deref()
        .is_some_and(|filter| filter != name)
    {
        return;
    }
    let shared = Rc::clone(cohort);
    workloads.push(measure(
        name,
        oracle,
        plan,
        move || Rc::clone(&shared),
        operation,
    ));
}

fn sampled_workload(name: &str) -> bool {
    !matches!(
        name,
        "environment_identity"
            | "window_topology_scale"
            | "display_current_newer"
            | "permission_availability"
            | "queue_expiry_overload"
            | "native_stop_target_loss"
            | "session_engine_close"
            | "producer_progress_cleanup_privacy"
    )
}
fn environment_identity(cohort: &Rc<RefCell<Cohort>>) -> Sample {
    measured(
        || {
            let mut cohort = cohort.borrow_mut();
            let Ok(run) = cohort.run() else { return false };
            run.target == run.session.target()
                && run.session.stream() == run.session.description().stream()
                && run.shape.extent() == run.template.extent()
                && run.template.backend().as_str() == "opencv-cpu"
                && permission_oracle(&run.engine)
                && run.fixture.process_id() != 0
        },
        None,
    )
}

fn window_absent_current(cohort: &Rc<RefCell<Cohort>>) -> Sample {
    observed_sample(cohort, |run| {
        run.command_absent()?;
        let query = run.start_watch(TemplateStability::immediate())?;
        prime_pending(&query)?;
        let terminal = query.cancel();
        let expected = matches!(&*terminal, TemplateTerminalOutcome::Cancelled);
        Ok((expected, terminal_query_metrics(&query, expected)?))
    })
}

fn window_transient_appearance(cohort: &Rc<RefCell<Cohort>>) -> Sample {
    observed_sample(cohort, |run| {
        run.command_absent()?;
        let query = run.start_watch(
            TemplateStability::duration(Duration::from_secs(1))
                .map_err(|_| "protocol_drift".to_owned())?,
        )?;
        run.command_visible()?;
        let visible = wait_progress(&query, |progress| progress.confirmed_observations() >= 1)?;
        run.command_absent()?;
        wait_progress(&query, |progress| {
            progress
                .last_frame()
                .is_some_and(|stamp| visible.last_frame().is_none_or(|old| stamp != old))
                && progress.confirmed_observations() == 0
        })?;
        let terminal = query.cancel();
        let expected = matches!(&*terminal, TemplateTerminalOutcome::Cancelled);
        Ok((expected, terminal_query_metrics(&query, expected)?))
    })
}

fn window_persistent_appearance(cohort: &Rc<RefCell<Cohort>>) -> Sample {
    matched_sample(cohort, None)
}

fn window_disappearance_reset(cohort: &Rc<RefCell<Cohort>>) -> Sample {
    observed_sample(cohort, |run| {
        run.command_absent()?;
        let clock = Arc::new(ManualClock::new());
        let stability = TemplateStability::duration(STATIC_STABILITY)
            .map_err(|_| "protocol_drift".to_owned())?;
        let query = run.start_watch_with(
            run.template.clone(),
            stability,
            OperationContext::new().with_clock(clock.clone()),
        )?;
        run.command_visible()?;
        let first = wait_progress(&query, |progress| progress.confirmed_observations() >= 1)?;
        run.command_absent()?;
        let reset = wait_progress(&query, |progress| {
            progress.confirmed_observations() == 0
                && progress
                    .last_frame()
                    .is_some_and(|stamp| first.last_frame().is_none_or(|old| stamp != old))
        })?;
        run.command_visible()?;
        let second = wait_progress(&query, |progress| progress.confirmed_observations() >= 1)?;
        clock.advance(STATIC_STABILITY);
        run.command_visible()?;
        let (terminal, _) = wait_terminal(&query)?;
        let correct = matched_exact(&terminal, run, 1, reset.last_frame())
            && second.last_frame().is_some()
            && matches!(&*terminal, TemplateTerminalOutcome::Matched(result) if result.confirmed_duration() >= STATIC_STABILITY);
        Ok((correct, terminal_query_metrics(&query, correct)?))
    })
}

fn window_strictly_newer(cohort: &Rc<RefCell<Cohort>>) -> Sample {
    let baseline = {
        let mut cohort = cohort.borrow_mut();
        let Ok(run) = cohort.run() else {
            return Sample::unmapped(Duration::ZERO, false);
        };
        if run.command_absent().is_err() {
            return Sample::unmapped(Duration::ZERO, false);
        }
        run.session
            .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
            .ok()
            .map(|frame| frame.stamp())
    };
    matched_sample(cohort, baseline)
}

fn window_move(cohort: &Rc<RefCell<Cohort>>) -> Sample {
    geometry_sample(cohort, GeometryAction::Move)
}

fn window_resize(cohort: &Rc<RefCell<Cohort>>) -> Sample {
    geometry_sample(cohort, GeometryAction::Resize)
}

fn window_topology_scale(cohort: &Rc<RefCell<Cohort>>) -> Sample {
    geometry_sample(cohort, GeometryAction::Topology)
}

fn project_marker_shape(
    target_frame: &Frame,
    display_frame: &Frame,
    target_shape: MarkerShape,
) -> Option<MarkerShape> {
    let target_origin = Point::new(
        CoordinateSpace::CapturePixels,
        f64::from(target_shape.origin_x),
        f64::from(target_shape.origin_y),
    )
    .ok()?;
    let target_cell_end = Point::new(
        CoordinateSpace::CapturePixels,
        f64::from(target_shape.origin_x) + f64::from(target_shape.cell_width),
        f64::from(target_shape.origin_y) + f64::from(target_shape.cell_height),
    )
    .ok()?;
    let desktop_origin = target_frame
        .transform()
        .convert_point(target_origin, CoordinateSpace::DesktopLogical)
        .ok()?;
    let desktop_cell_end = target_frame
        .transform()
        .convert_point(target_cell_end, CoordinateSpace::DesktopLogical)
        .ok()?;
    let display_origin = display_frame
        .transform()
        .convert_point(desktop_origin, CoordinateSpace::CapturePixels)
        .ok()?;
    let display_cell_end = display_frame
        .transform()
        .convert_point(desktop_cell_end, CoordinateSpace::CapturePixels)
        .ok()?;
    Some(MarkerShape {
        cell_width: scaled_u32(display_cell_end.x() - display_origin.x(), 1.0)?,
        cell_height: scaled_u32(display_cell_end.y() - display_origin.y(), 1.0)?,
        origin_x: scaled_i32(display_origin.x(), 1.0)?,
        origin_y: scaled_i32(display_origin.y(), 1.0)?,
    })
}

fn marker_shape_fits(frame: &Frame, shape: MarkerShape) -> bool {
    let Ok(left) = u32::try_from(shape.origin_x) else {
        return false;
    };
    let Ok(top) = u32::try_from(shape.origin_y) else {
        return false;
    };
    let expected = shape.extent();
    let actual = frame.descriptor().extent();
    left.checked_add(expected.width())
        .is_some_and(|right| right <= actual.width())
        && top
            .checked_add(expected.height())
            .is_some_and(|bottom| bottom <= actual.height())
}

fn display_current_newer(cohort: &Rc<RefCell<Cohort>>) -> Sample {
    observed_sample(cohort, |run| {
        run.command_absent()?;
        let target_frame = run
            .session
            .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
            .map_err(|_| "typed_operation_failure:CaptureFailed".to_owned())?;
        let target_shape = marker_shape(&target_frame, &run.fixture)
            .ok_or_else(|| "wrong_transform".to_owned())?;
        let displays = run
            .engine
            .discover(&bounded(OPERATION_WAIT))
            .map_err(|_| "capability_unavailable:capture".to_owned())?
            .into_iter()
            .filter(|target| target.capability().kind() == Some(TargetKind::Display));

        for display in displays {
            let display_id = display.id();
            let Ok(session) =
                run.engine
                    .open(display_id, &OpenRequest::new(), &bounded(OPERATION_WAIT))
            else {
                continue;
            };
            let Ok(frame) =
                session.acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
            else {
                session
                    .close(&bounded(OPERATION_WAIT))
                    .map_err(|_| "cleanup_failed".to_owned())?;
                continue;
            };
            let Some(shape) = project_marker_shape(&target_frame, &frame, target_shape) else {
                session
                    .close(&bounded(OPERATION_WAIT))
                    .map_err(|_| "cleanup_failed".to_owned())?;
                continue;
            };
            if !marker_shape_fits(&frame, shape) {
                session
                    .close(&bounded(OPERATION_WAIT))
                    .map_err(|_| "cleanup_failed".to_owned())?;
                continue;
            }

            let result = (|| {
                let template = prepare_marker(&run.engine, shape, "watch-marker-v1-display")?;
                let template_id = template.id().clone();
                let options = MatchOptions::from_defaults(template.defaults());
                let query = session
                    .start_template_watch(TemplateWatchRequest::new(
                        template,
                        options,
                        OperationContext::new(),
                    ))
                    .map_err(|_| "typed_operation_failure:VisionFailed".to_owned())?;
                let baseline = prime_pending(&query)?;
                let newer_than = baseline.last_frame();
                run.command_visible()?;
                // Full-display matching uses the predeclared operation bound.
                // A shorter wall-clock wait would measure host scheduling and
                // cancel valid high-resolution work before it can commit.
                let terminal = wait_terminal(&query);
                if terminal.is_err() {
                    let _ = query.cancel();
                }
                let (terminal, _) = terminal?;
                let correct =
                    matched_target_exact(&terminal, display_id, &template_id, shape, 1, newer_than);
                Ok((correct, terminal_query_metrics(&query, correct)?))
            })();
            let closed = session.close(&bounded(OPERATION_WAIT)).is_ok();
            let hidden = run.command_absent();
            if !closed {
                return Err("cleanup_failed".to_owned());
            }
            hidden?;
            return result;
        }

        run.command_absent()?;
        Err("capability_unavailable:display_target".to_owned())
    })
}

fn permission_availability(cohort: &Rc<RefCell<Cohort>>) -> Sample {
    measured(
        || {
            let mut cohort = cohort.borrow_mut();
            cohort.run().is_ok_and(|run| permission_oracle(&run.engine))
        },
        None,
    )
}

fn native_high_rate_slow_backend(cohort: &Rc<RefCell<Cohort>>) -> Sample {
    observed_sample(cohort, |run| {
        run.command_absent()?;
        let baseline = run
            .session
            .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
            .map_err(|_| "wrong_source".to_owned())?
            .stamp();
        let delay = install_find_delay(SLOW_BACKEND).ok_or_else(|| "protocol_drift".to_owned())?;
        let query = run.start_watch(TemplateStability::immediate())?;
        wait_progress(&query, TemplateQueryProgress::is_in_flight)?;
        run.command_visible()?;
        run.command_absent()?;
        run.command_visible()?;
        let (terminal, _) = wait_terminal(&query)?;
        drop(delay);
        wait_for_backend_idle(OPERATION_WAIT)?;
        let later = run.acquire_newer(baseline).is_ok();
        let correct = terminal.is_match() && later;
        Ok((correct, terminal_query_metrics(&query, correct)?))
    })
}

fn two_query_fairness(cohort: &Rc<RefCell<Cohort>>) -> Sample {
    paired_query_sample(cohort, true, false)
}

fn two_session_fairness(cohort: &Rc<RefCell<Cohort>>) -> Sample {
    observed_sample(cohort, |run| {
        run.command_visible()?;
        let second_session = run
            .engine
            .open(run.target, &OpenRequest::new(), &bounded(OPERATION_WAIT))
            .map_err(|_| "typed_operation_failure:CaptureFailed".to_owned())?;
        let delay = install_find_delay(SLOW_BACKEND).ok_or_else(|| "protocol_drift".to_owned())?;
        let first = run.start_watch(TemplateStability::immediate())?;
        let second = second_session
            .start_template_watch(TemplateWatchRequest::new(
                run.template.clone(),
                MatchOptions::from_defaults(run.template.defaults()),
                OperationContext::new(),
            ))
            .map_err(|_| "typed_operation_failure:VisionFailed".to_owned())?;
        let (first_terminal, _) = wait_terminal(&first)?;
        let (second_terminal, _) = wait_terminal(&second)?;
        drop(delay);
        wait_for_backend_idle(OPERATION_WAIT)?;
        let closed = second_session.close(&bounded(OPERATION_WAIT)).is_ok();
        run.command_absent()?;
        let first_expected = first_terminal.is_match();
        let second_expected = second_terminal.is_match();
        let metrics = terminal_query_metrics(&first, first_expected)?
            .saturating_add(terminal_query_metrics(&second, second_expected)?);
        Ok((first_expected && second_expected && closed, metrics))
    })
}

fn exact_coalescing(cohort: &Rc<RefCell<Cohort>>) -> Sample {
    paired_query_sample(cohort, true, false)
}

fn unequal_no_coalescing(cohort: &Rc<RefCell<Cohort>>) -> Sample {
    paired_query_sample(cohort, true, true)
}

fn queue_expiry_overload(cohort: &Rc<RefCell<Cohort>>) -> Sample {
    const QUERY_COUNT: usize = 32;

    observed_sample(cohort, |run| {
        run.command_absent()?;
        let descriptor = TemplateSchedulerDescriptor::selected_default();
        let expiry = descriptor.eligible_queue_expiry();
        let mut templates = Vec::with_capacity(QUERY_COUNT);
        for index in 0..QUERY_COUNT {
            templates.push(prepare_marker(
                &run.engine,
                run.shape,
                &format!("watch-marker-v1-overload-{index}"),
            )?);
        }
        let before_delay = expiry
            .checked_add(Duration::from_secs(1))
            .ok_or_else(|| "protocol_drift".to_owned())?;
        let delay = install_find_delay(before_delay).ok_or_else(|| "protocol_drift".to_owned())?;
        let before = backend_snapshot();
        let mut queries = Vec::with_capacity(templates.len());
        for template in templates {
            queries.push(run.start_watch_with(
                template,
                TemplateStability::immediate(),
                OperationContext::new(),
            )?);
        }
        if queries.len() != QUERY_COUNT {
            return Err("unbounded_queue".to_owned());
        }

        let admission_deadline = Instant::now() + OPERATION_WAIT;
        loop {
            if queries.iter().any(|query| {
                matches!(
                    query.poll(),
                    TemplateQueryOutcome::Pending(progress) if progress.is_in_flight()
                )
            }) {
                break;
            }
            if Instant::now() >= admission_deadline {
                return Err("typed_operation_failure:DeadlineExceeded".to_owned());
            }
            thread::sleep(POLL_WAIT);
        }

        thread::sleep(
            expiry
                .checked_add(Duration::from_millis(100))
                .ok_or_else(|| "protocol_drift".to_owned())?,
        );
        let overload_deadline = Instant::now() + OPERATION_WAIT;
        loop {
            if queries.iter().any(|query| {
                matches!(
                    query.poll(),
                    TemplateQueryOutcome::Terminal(terminal)
                        if matches!(
                            &*terminal,
                            TemplateTerminalOutcome::Overloaded(
                                TemplateOverload::QueueExpired
                            )
                        )
                )
            }) {
                break;
            }
            if Instant::now() >= overload_deadline {
                return Err("typed_operation_failure:DeadlineExceeded".to_owned());
            }
            thread::sleep(POLL_WAIT);
        }

        let mut metrics = QueryWorkMetrics::default();
        for query in &queries {
            let terminal = query.cancel();
            let expected = matches!(
                &*terminal,
                TemplateTerminalOutcome::Cancelled
                    | TemplateTerminalOutcome::Overloaded(TemplateOverload::QueueExpired)
            );
            let query_metrics = terminal_query_metrics(query, expected)?;
            let publications = metrics
                .producer_publications
                .max(query_metrics.producer_publications);
            metrics = metrics.saturating_add(query_metrics);
            metrics.producer_publications = publications;
        }
        drop(delay);
        wait_for_backend_idle(OPERATION_WAIT)?;
        let delta = backend_snapshot()
            .checked_delta(before)
            .ok_or_else(|| "unaccounted_work".to_owned())?;
        let expected_queries = u64::try_from(QUERY_COUNT).expect("query count fits");
        let correct = metrics.query_completions == expected_queries
            && metrics.query_failures == 0
            && metrics.producer_publications != 0
            && metrics.queue_expired != 0
            && metrics.admitted == delta.find_calls
            && metrics.admitted != 0
            && metrics.admitted <= u64::from(descriptor.max_in_flight_analyses())
            && metrics.coalesced == 0
            && metrics.rejected == 0
            && metrics.completed == 0
            && metrics.failed == 0
            && metrics.superseded >= metrics.admitted
            && delta.find_completions == delta.find_calls
            && delta.find_failures == 0;
        Ok((correct, metrics))
    })
}

fn stale_generation(cohort: &Rc<RefCell<Cohort>>) -> Sample {
    observed_sample(cohort, |run| {
        let prior = run
            .session
            .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
            .map_err(|_| "wrong_source".to_owned())?
            .stamp();
        run.command_visible()?;
        let old_frame = wait_marker_state(run, prior, true)?;
        let old_geometry = old_frame.stamp().geometry();
        let delay = install_find_delay(SLOW_BACKEND).ok_or_else(|| "protocol_drift".to_owned())?;
        let query = run.start_watch(TemplateStability::immediate())?;
        wait_progress(&query, TemplateQueryProgress::is_in_flight)?;
        let resize = run.fixture.resize_target()?;
        run.accept_acknowledgement(resize)?;
        let moved_frame = wait_resize_change(run, &old_frame)?;
        let moved_geometry = moved_frame.stamp().geometry();
        drop(delay);
        run.command_visible()?;
        let (terminal, _) = wait_terminal(&query)?;
        wait_for_backend_idle(OPERATION_WAIT)?;
        let new_geometry =
            terminal_match(&terminal).map(|result| result.frame().stamp().geometry());
        let restore = run.fixture.resize_target()?;
        run.accept_acknowledgement(restore)?;
        let restored_frame = wait_resize_change(run, &moved_frame)?;
        let restored_placement = restored_frame.transform().target()
            == old_frame.transform().target()
            && restored_frame.descriptor().extent() == old_frame.descriptor().extent();
        let correct = terminal.is_match()
            && old_geometry != moved_geometry
            && new_geometry == Some(moved_geometry)
            && restored_placement;
        Ok((correct, terminal_query_metrics(&query, correct)?))
    })
}

fn wait_cancel_deadline(cohort: &Rc<RefCell<Cohort>>) -> Sample {
    observed_sample(cohort, |run| {
        let _absent = establish_absent(run)?;
        let wait_query = run.start_watch(TemplateStability::immediate())?;
        let wait_baseline = prime_pending(&wait_query)?;
        let wait_status = wait_query
            .wait(&bounded(Duration::from_millis(25)))
            .expect_err("caller wait expires")
            .status();
        let still_pending = matches!(wait_query.poll(), TemplateQueryOutcome::Pending(_));
        let cancelled_terminal = wait_query.cancel();
        let cancelled = matches!(&*cancelled_terminal, TemplateTerminalOutcome::Cancelled);

        let deadline_query = run.start_watch_with(
            run.template.clone(),
            TemplateStability::immediate(),
            OperationContext::new()
                .with_timeout(Duration::from_millis(250))
                .map_err(|_| "protocol_drift".to_owned())?,
        )?;
        prime_pending(&deadline_query)?;
        let deadline_terminal = wait_terminal(&deadline_query)?.0;
        let deadline = matches!(
            &*deadline_terminal,
            TemplateTerminalOutcome::DeadlineExceeded
        );
        wait_for_backend_idle(OPERATION_WAIT)?;
        let mut metrics = terminal_query_metrics(&wait_query, cancelled)?
            .saturating_add(terminal_query_metrics(&deadline_query, deadline)?);
        metrics.producer_publications = u64::from(metrics.producer_publications != 0);
        Ok((
            wait_status == Status::DeadlineExceeded
                && still_pending
                && cancelled
                && deadline
                && wait_baseline.confirmed_observations() == 0,
            metrics,
        ))
    })
}

fn native_stop_target_loss(cohort: &Rc<RefCell<Cohort>>) -> Sample {
    destructive_sample(cohort, |mut run| {
        let _absent = establish_absent(&mut run)?;
        let query = run.start_watch(TemplateStability::immediate())?;
        prime_pending(&query)?;
        #[cfg(target_os = "windows")]
        {
            let close = run.fixture.close_target()?;
            run.accept_acknowledgement(close)?;
        }
        #[cfg(target_os = "macos")]
        {
            // A destroyed-window ScreenCaptureKit filter can remain quiescent
            // without a terminal callback. Ending the authenticated fixture
            // process exercises the stream authority required by this row.
            if !run.fixture.finish() {
                return Err("fixture_authority_failed".to_owned());
            }
        }
        let (terminal, _) = wait_terminal(&query)?;
        wait_for_backend_idle(OPERATION_WAIT)?;
        let expected = matches!(&*terminal, TemplateTerminalOutcome::TargetLost);
        Ok((expected, terminal_query_metrics(&query, expected)?))
    })
}

fn session_engine_close(cohort: &Rc<RefCell<Cohort>>) -> Sample {
    destructive_sample(cohort, |mut run| {
        let _absent = establish_absent(&mut run)?;
        let query = run.start_watch(TemplateStability::immediate())?;
        prime_pending(&query)?;
        let first = run.session.close(&bounded(OPERATION_WAIT)).is_ok();
        let second = run.session.close(&bounded(OPERATION_WAIT)).is_ok();
        let (terminal, _) = wait_terminal(&query)?;
        let stable = Arc::ptr_eq(&terminal, &wait_terminal(&query)?.0);
        let expected = matches!(&*terminal, TemplateTerminalOutcome::SessionClosed);
        let metrics = terminal_query_metrics(&query, expected)?;
        drop(run.engine);
        wait_for_backend_idle(OPERATION_WAIT)?;
        Ok((first && second && stable && expected, metrics))
    })
}

fn retained_result_mapping(cohort: &Rc<RefCell<Cohort>>) -> Sample {
    destructive_sample(cohort, |mut run| {
        let _absent = establish_absent(&mut run)?;
        let query = run.start_watch(TemplateStability::immediate())?;
        prime_pending(&query)?;
        run.command_visible()?;
        let (terminal, _) = wait_terminal(&query)?;
        let matched = terminal.is_match();
        let metrics = terminal_query_metrics(&query, matched)?;
        let result = terminal_match(&terminal)
            .ok_or_else(|| "wrong_match".to_owned())?
            .clone();
        let mapping = run
            .session
            .map_frame(result.frame(), PixelFormat::Rgba8, &bounded(OPERATION_WAIT))
            .map_err(|_| "ownership_pinned".to_owned())?;
        let retained_stamp = mapping.stamp();
        let retained_prefix = mapping
            .bytes()
            .get(..mapping.bytes().len().min(16))
            .map(<[u8]>::to_vec)
            .ok_or_else(|| "ownership_pinned".to_owned())?;
        run.session
            .close(&bounded(OPERATION_WAIT))
            .map_err(|_| "cleanup_failed".to_owned())?;
        drop(query);
        drop(terminal);
        drop(run.engine);
        let retained =
            mapping.stamp() == retained_stamp && mapping.bytes().starts_with(&retained_prefix);
        let fresh_engine =
            native_engine().map_err(|_| "capability_unavailable:capture".to_owned())?;
        let fresh_target = run.fixture.authenticated_target(&fresh_engine)?;
        let fresh_session = fresh_engine
            .open(fresh_target, &OpenRequest::new(), &bounded(OPERATION_WAIT))
            .map_err(|_| "producer_stalled".to_owned())?;
        let progressed = fresh_session
            .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
            .is_ok();
        let fresh_closed = fresh_session.close(&bounded(OPERATION_WAIT)).is_ok();
        drop(fresh_engine);
        Ok((retained && progressed && fresh_closed, metrics))
    })
}

fn fresh_session(cohort: &Rc<RefCell<Cohort>>) -> Sample {
    destructive_sample(cohort, |mut run| {
        let _absent = establish_absent(&mut run)?;
        run.session
            .close(&bounded(OPERATION_WAIT))
            .map_err(|_| "cleanup_failed".to_owned())?;
        let session = run
            .engine
            .open(run.target, &OpenRequest::new(), &bounded(OPERATION_WAIT))
            .map_err(|_| "typed_operation_failure:CaptureFailed".to_owned())?;
        let query = session
            .start_template_watch(TemplateWatchRequest::new(
                run.template.clone(),
                MatchOptions::from_defaults(run.template.defaults()),
                OperationContext::new(),
            ))
            .map_err(|_| "typed_operation_failure:VisionFailed".to_owned())?;
        prime_pending(&query)?;
        run.command_visible()?;
        let (terminal, _) = wait_terminal(&query)?;
        let closed = session.close(&bounded(OPERATION_WAIT)).is_ok();
        wait_for_backend_idle(OPERATION_WAIT)?;
        let matched = terminal.is_match();
        Ok((matched && closed, terminal_query_metrics(&query, matched)?))
    })
}

fn producer_progress_cleanup_privacy(cohort: &Rc<RefCell<Cohort>>) -> Sample {
    measured(
        || {
            let mut cohort = cohort.borrow_mut();
            let Ok(run) = cohort.run() else {
                return false;
            };
            let before = bench_harness::live_allocated_bytes();
            let progressed = run.command_absent().is_ok()
                && run
                    .session
                    .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
                    .is_ok();
            let after = bench_harness::live_allocated_bytes();
            progressed && after.saturating_sub(before) <= 4_096 && privacy_tokens_are_bounded()
        },
        peak_resident_bytes(),
    )
}

#[derive(Debug, Clone, Copy)]
enum GeometryAction {
    Move,
    Resize,
    Topology,
}

fn geometry_sample(cohort: &Rc<RefCell<Cohort>>, action: GeometryAction) -> Sample {
    observed_sample(cohort, |run| {
        run.command_absent()?;
        let before = run
            .session
            .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
            .map_err(|_| "wrong_source".to_owned())?;
        let acknowledgement = match action {
            GeometryAction::Move => run.fixture.move_target(),
            GeometryAction::Resize => run.fixture.resize_target(),
            GeometryAction::Topology => run.fixture.move_next_display(),
        }?;
        run.accept_acknowledgement(acknowledgement)?;
        let after = match action {
            GeometryAction::Resize => wait_resize_change(run, &before),
            GeometryAction::Move | GeometryAction::Topology => {
                wait_geometry_change(run, before.stamp())
            }
        }?;
        run.refresh_template(&after, "watch-marker-v1-geometry")?;
        let query = run.start_watch(TemplateStability::immediate())?;
        prime_pending(&query)?;
        run.command_visible()?;
        let (terminal, _) = wait_terminal(&query)?;
        let geometry_changed = before.stamp().geometry() != after.stamp().geometry();
        let exact = matched_exact(&terminal, run, 1, Some(before.stamp()));
        let restore = match action {
            GeometryAction::Move => Some(run.fixture.move_target()),
            GeometryAction::Resize => None,
            GeometryAction::Topology => Some(run.fixture.restore_placement()),
        };
        let restored = if let Some(restore) = restore {
            let restore = restore?;
            run.accept_acknowledgement(restore)?;
            let restored_frame = wait_geometry_change(run, after.stamp())?;
            let restored = before.descriptor() == restored_frame.descriptor()
                && before.transform().covers_target() == restored_frame.transform().covers_target()
                && before.transform().target() == restored_frame.transform().target();
            run.refresh_template(&restored_frame, "watch-marker-v1-restored")?;
            restored
        } else {
            true
        };
        run.command_absent()?;
        let correct = geometry_changed && exact && restored;
        Ok((correct, terminal_query_metrics(&query, correct)?))
    })
}

fn matched_sample(cohort: &Rc<RefCell<Cohort>>, newer_than: Option<FrameStamp>) -> Sample {
    observed_sample(cohort, |run| {
        run.command_absent()?;
        let stability = TemplateStability::duration(STATIC_STABILITY)
            .map_err(|_| "protocol_drift".to_owned())?;
        let query = run.start_watch(stability)?;
        run.command_visible()?;
        wait_progress(&query, |progress| progress.confirmed_observations() >= 1)?;
        thread::sleep(STATIC_STABILITY);
        run.command_visible()?;
        let (terminal, _) = wait_terminal(&query)?;
        let correct = matched_exact(&terminal, run, 1, newer_than)
            && matches!(&*terminal, TemplateTerminalOutcome::Matched(result) if result.confirmed_duration() >= STATIC_STABILITY);
        run.command_absent()?;
        Ok((correct, terminal_query_metrics(&query, correct)?))
    })
}

fn paired_query_sample(
    cohort: &Rc<RefCell<Cohort>>,
    controlled_delay: bool,
    distinct_preparation: bool,
) -> Sample {
    observed_sample(cohort, |run| {
        let _absent = establish_absent(run)?;
        let second_template = if distinct_preparation {
            prepare_marker(&run.engine, run.shape, "watch-marker-v1-distinct")?
        } else {
            run.template.clone()
        };
        let clock = Arc::new(ManualClock::new());
        let rate = TemplateAnalysisRate::at_most_every(Duration::from_secs(1))
            .map_err(|_| "protocol_drift".to_owned())?;
        let first = run
            .session
            .start_template_watch(
                TemplateWatchRequest::new(
                    run.template.clone(),
                    MatchOptions::from_defaults(run.template.defaults()),
                    OperationContext::new().with_clock(clock.clone()),
                )
                .with_rate(rate),
            )
            .map_err(|_| "typed_operation_failure:VisionFailed".to_owned())?;
        let second = run
            .session
            .start_template_watch(
                TemplateWatchRequest::new(
                    second_template.clone(),
                    MatchOptions::from_defaults(second_template.defaults()),
                    OperationContext::new().with_clock(clock.clone()),
                )
                .with_rate(rate),
            )
            .map_err(|_| "typed_operation_failure:VisionFailed".to_owned())?;
        let ready = |progress: TemplateQueryProgress| {
            progress.confirmed_observations() == 0
                && progress.work().get(TemplateWorkDisposition::Completed) >= 1
                && progress.work().get(TemplateWorkDisposition::DeferredRate) >= 1
                && progress.pending_count() == 1
                && progress.in_flight_count() == 0
        };
        let first_ready = wait_progress(&first, ready)?;
        let second_ready = wait_progress(&second, ready)?;
        let first_stamp = first_ready
            .last_frame()
            .ok_or_else(|| "wrong_source".to_owned())?;
        let second_stamp = second_ready
            .last_frame()
            .ok_or_else(|| "wrong_source".to_owned())?;
        let baseline = if first_stamp.order(&second_stamp) == Ok(FrameOrder::Before) {
            second_stamp
        } else {
            first_stamp
        };
        let delay = controlled_delay
            .then(|| install_find_delay(SLOW_BACKEND).ok_or_else(|| "protocol_drift".to_owned()))
            .transpose()?;
        let before = backend_snapshot();
        run.command_visible()?;
        let visible = wait_marker_state(run, baseline, true)?;
        if visible.stamp().geometry() != baseline.geometry() {
            return Err("wrong_transform".to_owned());
        }
        // Both queries remain rate-deferred on the shared manual clock while
        // the production watcher observes the acknowledged visible revision.
        thread::sleep(Duration::from_millis(100));
        clock.advance(Duration::from_secs(1));
        let (first_terminal, _) = wait_terminal(&first)?;
        let (second_terminal, _) = wait_terminal(&second)?;
        drop(delay);
        wait_for_backend_idle(OPERATION_WAIT)?;
        let delta = backend_snapshot()
            .checked_delta(before)
            .ok_or_else(|| "unaccounted_work".to_owned())?;
        let expected_calls = if distinct_preparation { 2 } else { 1 };
        let exact_calls = delta.find_calls == expected_calls
            && delta.find_completions == expected_calls
            && delta.find_failures == 0;
        run.command_absent()?;
        let first_expected = first_terminal.is_match();
        let second_expected = second_terminal.is_match();
        let first_metrics = terminal_query_metrics(&first, first_expected)?;
        let second_metrics = terminal_query_metrics(&second, second_expected)?;
        let publications = first_metrics
            .producer_publications
            .max(second_metrics.producer_publications);
        let mut metrics = first_metrics.saturating_add(second_metrics);
        metrics.producer_publications = publications;
        Ok((first_expected && second_expected && exact_calls, metrics))
    })
}

fn destructive_sample(
    cohort: &Rc<RefCell<Cohort>>,
    operation: impl FnOnce(NativeRun) -> Result<(bool, QueryWorkMetrics), String>,
) -> Sample {
    let before = backend_snapshot();
    let started = Instant::now();
    let run = cohort.borrow().fresh();
    let (correct, metrics) = run
        .and_then(operation)
        .unwrap_or_else(|code| panic!("{code}"));
    finish_observed_sample(started.elapsed(), correct, metrics, before)
}

fn observed_sample(
    cohort: &Rc<RefCell<Cohort>>,
    operation: impl FnOnce(&mut NativeRun) -> Result<(bool, QueryWorkMetrics), String>,
) -> Sample {
    let before = backend_snapshot();
    let started = Instant::now();
    let (correct, metrics) = {
        let mut cohort = cohort.borrow_mut();
        cohort
            .run()
            .and_then(operation)
            .unwrap_or_else(|code| panic!("{code}"))
    };
    // Terminal authority is already fixed; this bounded wait closes only the
    // cumulative work interval and fails if late work does not release.
    wait_for_backend_idle(OPERATION_WAIT).unwrap_or_else(|code| panic!("{code}"));
    let elapsed = started.elapsed();
    // Excluded from latency and query/backend accounting, this fence joins the
    // watcher acquisition worker, then forces and retains one authoritative
    // absent mapping so native producer state cannot cross the allocator endpoint.
    cohort
        .borrow_mut()
        .settle()
        .unwrap_or_else(|code| panic!("{code}"));
    finish_observed_sample(elapsed, correct, metrics, before)
}

fn finish_observed_sample(
    elapsed: Duration,
    correct: bool,
    mut metrics: QueryWorkMetrics,
    before: BackendSnapshot,
) -> Sample {
    let after = backend_snapshot();
    let delta = after.checked_delta(before);
    if let Some(delta) = delta {
        metrics.backend_runs = delta.find_calls;
        metrics.backend_completions = Some(delta.find_completions);
        metrics.backend_failures = Some(delta.find_failures);
    }
    let accounted = delta.is_some_and(|delta| {
        metrics.admitted == delta.find_calls
            && delta.find_calls == delta.find_completions.saturating_add(delta.find_failures)
            && delta.active_finds == 0
    });
    let mapped = delta.map_or(0, |value| value.mapped_bytes);
    let sample = Sample::new(elapsed, correct && accounted, mapped).with_query_work(metrics);
    match peak_resident_bytes() {
        Some(bytes) => sample.with_peak_resident_bytes(bytes),
        None => sample,
    }
}

fn terminal_query_metrics(
    query: &TemplateQuery,
    expected_terminal: bool,
) -> Result<QueryWorkMetrics, String> {
    if !matches!(query.poll(), TemplateQueryOutcome::Terminal(_)) {
        return Err("silent_query_loss".to_owned());
    }
    let progress = query.benchmark_work_snapshot();
    if progress.pending_count() != 0 || progress.in_flight_count() != 0 {
        return Err("unaccounted_work".to_owned());
    }
    let work = progress.work();
    Ok(QueryWorkMetrics {
        query_completions: u64::from(expected_terminal),
        query_failures: u64::from(!expected_terminal),
        stale_discards: work.get(TemplateWorkDisposition::Superseded),
        producer_publications: query.benchmark_publication_count(),
        admitted: work.get(TemplateWorkDisposition::Admitted),
        skipped_change: work.get(TemplateWorkDisposition::SkippedChange),
        deferred_rate: work.get(TemplateWorkDisposition::DeferredRate),
        coalesced: work.get(TemplateWorkDisposition::Coalesced),
        superseded: work.get(TemplateWorkDisposition::Superseded),
        rejected: work.get(TemplateWorkDisposition::Rejected),
        queue_expired: work.get(TemplateWorkDisposition::QueueExpired),
        completed: work.get(TemplateWorkDisposition::Completed),
        failed: work.get(TemplateWorkDisposition::Failed),
        ..QueryWorkMetrics::default()
    })
}

fn measured(operation: impl FnOnce() -> bool, resident: Option<u64>) -> Sample {
    let started = Instant::now();
    let correct = operation();
    let sample = Sample::unmapped(started.elapsed(), correct);
    match resident {
        Some(bytes) => sample.with_peak_resident_bytes(bytes),
        None => sample,
    }
}

fn wait_progress(
    query: &TemplateQuery,
    predicate: impl Fn(TemplateQueryProgress) -> bool,
) -> Result<TemplateQueryProgress, String> {
    let deadline = Instant::now() + OPERATION_WAIT;
    loop {
        match query.poll() {
            TemplateQueryOutcome::Pending(progress) if predicate(progress) => return Ok(progress),
            TemplateQueryOutcome::Pending(_) => {}
            TemplateQueryOutcome::Terminal(_) => return Err("authority_violation".to_owned()),
        }
        if Instant::now() >= deadline {
            return Err("typed_operation_failure:DeadlineExceeded".to_owned());
        }
        thread::sleep(POLL_WAIT);
    }
}

fn prime_pending(query: &TemplateQuery) -> Result<TemplateQueryProgress, String> {
    wait_progress(query, |progress| {
        progress.confirmed_observations() == 0
            && progress.last_frame().is_some()
            && progress.work().get(TemplateWorkDisposition::Completed) >= 1
            && progress.pending_count() <= 1
            && progress.in_flight_count() <= 2
    })
}

fn wait_terminal(
    query: &TemplateQuery,
) -> Result<(Arc<TemplateTerminalOutcome>, Option<TemplateQueryProgress>), String> {
    wait_terminal_bounded(query, OPERATION_WAIT)
}

fn wait_terminal_bounded(
    query: &TemplateQuery,
    wait: Duration,
) -> Result<(Arc<TemplateTerminalOutcome>, Option<TemplateQueryProgress>), String> {
    let deadline = Instant::now() + wait;
    let mut last = None;
    loop {
        match query.poll() {
            TemplateQueryOutcome::Pending(progress) => last = Some(progress),
            TemplateQueryOutcome::Terminal(terminal) => return Ok((terminal, last)),
        }
        if Instant::now() >= deadline {
            return Err("typed_operation_failure:DeadlineExceeded".to_owned());
        }
        thread::sleep(POLL_WAIT);
    }
}

fn wait_for_backend_idle(wait: Duration) -> Result<(), String> {
    let deadline = Instant::now() + wait;
    while Instant::now() < deadline {
        if backend_snapshot().active_finds == 0 {
            return Ok(());
        }
        thread::sleep(POLL_WAIT);
    }
    Err("unaccounted_work".to_owned())
}

fn matched_exact(
    terminal: &TemplateTerminalOutcome,
    run: &NativeRun,
    observations: u32,
    newer_than: Option<FrameStamp>,
) -> bool {
    matched_target_exact(
        terminal,
        run.target,
        run.template.id(),
        run.shape,
        observations,
        newer_than,
    )
}

fn matched_target_exact(
    terminal: &TemplateTerminalOutcome,
    target: TargetId,
    template: &TemplateId,
    shape: MarkerShape,
    observations: u32,
    newer_than: Option<FrameStamp>,
) -> bool {
    let TemplateTerminalOutcome::Matched(result) = terminal else {
        return false;
    };
    let match_result = result.result();
    let stamp = result.frame().stamp();
    let bounds = match_result.best().map(|candidate| candidate.bounds());
    let expected = shape.extent();
    result.target() == target
        && result.template() == template
        && result.confirmed_observations() >= observations
        && match_result.stamp() == stamp
        && match_result.transform() == result.frame().transform()
        && match_result.backend().id() == "opencv-cpu"
        && match_result.matches().len() == 1
        && bounds.is_some_and(|bounds| {
            bounds.left() == shape.origin_x
                && bounds.top() == shape.origin_y
                && bounds.width() == expected.width()
                && bounds.height() == expected.height()
                && match_result.searched().contains_rect(bounds)
        })
        && newer_than.is_none_or(|prior| stamp.order(&prior) == Ok(FrameOrder::After))
}

fn terminal_match(outcome: &TemplateTerminalOutcome) -> Option<&mado_pilot::TemplateWatchResult> {
    match outcome {
        TemplateTerminalOutcome::Matched(result) => Some(result),
        _ => None,
    }
}

fn prepare_marker(
    engine: &Engine,
    shape: MarkerShape,
    id: &str,
) -> Result<PreparedTemplate, String> {
    let width = shape.cell_width * 3;
    let height = shape.cell_height * 2;
    let capacity = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(3))
        .ok_or_else(|| "wrong_region".to_owned())?;
    let mut rgb = Vec::with_capacity(capacity);
    for y in 0..height {
        for x in 0..width {
            let cell = (x / shape.cell_width, y / shape.cell_height);
            rgb.extend_from_slice(if matches!(cell, (1, 0) | (0, 1)) {
                &MARKER_SECONDARY
            } else {
                &MARKER_PRIMARY
            });
        }
    }
    let source = TemplateSource::new(TemplateSourceRequest {
        id: TemplateId::new(id).map_err(|_| "protocol_drift".to_owned())?,
        encoding: TemplateEncoding::Png,
        extent: shape.extent(),
        space: CoordinateSpace::CapturePixels,
        defaults: MatchDefaults::new(0.95, 1).map_err(|_| "protocol_drift".to_owned())?,
        content: Arc::from(png::encode_rgb(width, height, &rgb)),
    })
    .map_err(|_| "protocol_drift".to_owned())?;
    engine
        .prepare_template(&source, &bounded(OPERATION_WAIT))
        .map_err(|_| "typed_operation_failure:VisionFailed".to_owned())
}

fn bounded(wait: Duration) -> OperationContext {
    OperationContext::new()
        .with_timeout(wait)
        .expect("positive qualification bound")
}

fn privacy_tokens_are_bounded() -> bool {
    const ALLOWLIST: [&str; 7] = [
        "native-watch-control-v1",
        "watch-marker-v1",
        "opencv-cpu",
        "aarch64-apple-darwin",
        "x86_64-pc-windows-msvc",
        "precursor",
        "final",
    ];
    ALLOWLIST
        .iter()
        .all(|value| value.len() <= 64 && !value.contains('/') && !value.contains('\\'))
}

fn report(arguments: &Arguments, plan: Plan, workloads: &[Workload]) {
    let source = required_identity(&arguments.raw, "--source-revision", 40);
    let tree = required_identity(&arguments.raw, "--source-tree", 40);
    let executable = required_identity(&arguments.raw, "--executable-sha256", 64);
    let fixture = required_identity(&arguments.raw, "--fixture-sha256", 64);
    let fixture_source = required_identity(&arguments.raw, "--fixture-source-sha256", 64);
    let process = required_identity(&arguments.raw, "--process-index", 1);
    let cohort = required_enum(&arguments.raw, "--cohort", &["precursor", "final"]);
    let host = required_enum(
        &arguments.raw,
        "--host-class",
        &["apple-m1-pro-10c-32g", "windows-i7-12700kf-32g"],
    );
    let backend = required_enum(&arguments.raw, "--backend", &["opencv-4.14.0"]);
    let toolchain = required_enum(
        &arguments.raw,
        "--toolchain",
        &[
            "rust-1.97.1-8bab26f4-llvm-22.1.6",
            "rust-1.97.1-msvc-19.44.35228",
        ],
    );
    let (hardware, os_version) = Profile::host(&arguments.raw);
    assert!(privacy_tokens_are_bounded(), "privacy_violation");
    let profile = Profile {
        fixture: native_watch_report::FIXTURE_DESCRIPTION.to_owned(),
        fixture_sha256: fixture,
        benchmark_executable_sha256: Some(executable),
        hardware,
        os_version,
        deployment_target: Some(target_name().to_owned()),
        build_profile: native_watch_report::BUILD_PROFILE.to_owned(),
        correctness_oracle: native_watch_report::CORRECTNESS_ORACLE,
        queue_policy: native_watch_report::QUEUE_POLICY,
        notes: Some(format!(
            "source {source}; tree {tree}; fixture-source {fixture_source}; backend {backend}; toolchain {toolchain}; host {host}; cohort {cohort}; process {process}; control native-watch-control-v1"
        )),
    };
    native_watch_report::validate(
        &profile,
        native_watch_report::Provenance {
            source: &source,
            tree: &tree,
            fixture_source: &fixture_source,
            backend: &backend,
            toolchain: &toolchain,
            host: &host,
            cohort: &cohort,
            process_index: &process,
        },
    )
    .unwrap_or_else(|_| panic!("privacy_violation"));
    bench_harness::report(
        &Benchmark {
            id: "phase-4-native-template-watch",
            workload: "native Rust facade maintained-session template-watch matrix",
            phase: "4",
        },
        &profile,
        plan,
        workloads,
    );
}

fn required_identity(arguments: &[String], name: &str, length: usize) -> String {
    let value = value(arguments, name).unwrap_or_else(|| panic!("identity_mismatch"));
    assert!(
        value.len() >= length
            && value.len() <= 96
            && value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() || byte.is_ascii_digit()),
        "identity_mismatch"
    );
    value.to_owned()
}

fn required_enum(arguments: &[String], name: &str, accepted: &[&str]) -> String {
    let value = value(arguments, name).unwrap_or_else(|| panic!("identity_mismatch"));
    assert!(accepted.contains(&value), "identity_mismatch");
    value.to_owned()
}

fn enforce_accepted_budgets(_workloads: &[Workload]) {
    panic!("capability_unavailable:profile");
}
