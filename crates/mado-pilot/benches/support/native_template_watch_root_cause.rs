//! Bounded same-process macOS capture-lifecycle stress lane.

use super::*;

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};

const DEFAULT_ITERATIONS: usize = 129;
const MAX_ITERATIONS: usize = 512;
const MAX_RUN_TIME: Duration = Duration::from_secs(30 * 60);
const DIAGNOSTIC_RING_CAPACITY: usize = 16;
const MAPPING_SENTINEL_CAPACITY: usize = 16;

type PrefixOperation = fn(&Rc<RefCell<Cohort>>) -> Sample;

const PREFIX_WORKLOADS: [(&str, PrefixOperation); 19] = [
    ("environment_identity", environment_identity),
    ("window_absent_current", window_absent_current),
    ("window_transient_appearance", window_transient_appearance),
    ("window_persistent_appearance", window_persistent_appearance),
    ("window_disappearance_reset", window_disappearance_reset),
    ("window_strictly_newer", window_strictly_newer),
    ("window_move", window_move),
    ("window_resize", window_resize),
    ("window_topology_scale", window_topology_scale),
    ("display_current_newer", display_current_newer),
    ("permission_availability", permission_availability),
    (
        "native_high_rate_slow_backend",
        native_high_rate_slow_backend,
    ),
    ("two_query_fairness", two_query_fairness),
    ("two_session_fairness", two_session_fairness),
    ("exact_coalescing", exact_coalescing),
    ("unequal_no_coalescing", unequal_no_coalescing),
    ("queue_expiry_overload", queue_expiry_overload),
    ("stale_generation", stale_generation),
    ("wait_cancel_deadline", wait_cancel_deadline),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetentionVariant {
    None,
    NativeFrame,
    CpuMapping,
}

impl RetentionVariant {
    const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::NativeFrame => "native_frame",
            Self::CpuMapping => "cpu_mapping",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Clean,
    OpenRejected,
    ProducerProgressFailure,
    TeardownFailure,
    OwnershipFailure,
    ApparatusFailure,
    PrefixFailure,
    RunBoundReached,
}

impl Outcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::OpenRejected => "open_rejected",
            Self::ProducerProgressFailure => "producer_progress_failure",
            Self::TeardownFailure => "teardown_failure",
            Self::OwnershipFailure => "ownership_failure",
            Self::ApparatusFailure => "apparatus_failure",
            Self::PrefixFailure => "prefix_failure",
            Self::RunBoundReached => "run_bound_reached",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Prefix,
    OldOpen,
    OldWatch,
    OldToken,
    OldRetention,
    OldTeardown,
    FreshOpen,
    FreshToken,
    FreshRetention,
    FreshTeardown,
    FixtureFinalize,
    ArtifactAuthority,
    OverallRun,
}

impl Stage {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Prefix => "prefix",
            Self::OldOpen => "old_open",
            Self::OldWatch => "old_watch",
            Self::OldToken => "old_token",
            Self::OldRetention => "old_retention",
            Self::OldTeardown => "old_teardown",
            Self::FreshOpen => "fresh_open",
            Self::FreshToken => "fresh_token",
            Self::FreshRetention => "fresh_retention",
            Self::FreshTeardown => "fresh_teardown",
            Self::FixtureFinalize => "fixture_finalize",
            Self::ArtifactAuthority => "artifact_authority",
            Self::OverallRun => "overall_run",
        }
    }
}

#[derive(Debug)]
enum WatchTokenObservation {
    Matched,
    Mismatched,
    Failed(String),
}

impl fmt::Display for WatchTokenObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Matched => formatter.write_str("matched"),
            Self::Mismatched => formatter.write_str("mismatched"),
            Self::Failed(error) => write!(formatter, "failed:{error}"),
        }
    }
}

#[derive(Debug)]
enum FailureEvidence {
    TokenSynchronization(TokenSynchronizationError),
    WatchWait {
        expected: VisualToken,
        error: String,
    },
    WatchTerminal {
        expected: VisualToken,
        terminal: &'static str,
        status: Option<Status>,
        last_progress_frame: Option<FrameStamp>,
        confirmed_observations: Option<u32>,
    },
    WatchMismatch {
        expected: VisualToken,
        frame: FrameStamp,
        exact_source_match: bool,
        token_observation: WatchTokenObservation,
    },
}

impl fmt::Display for FailureEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TokenSynchronization(error) => {
                write!(formatter, "token_synchronization{{{error}}}")
            }
            Self::WatchWait { expected, error } => write!(
                formatter,
                "watch_wait{{expected_token={},expected_marker={},error={error}}}",
                expected.value(),
                marker_state_name(expected.marker()),
            ),
            Self::WatchTerminal {
                expected,
                terminal,
                status,
                last_progress_frame,
                confirmed_observations,
            } => {
                write!(
                    formatter,
                    "watch_terminal{{expected_token={},expected_marker={},terminal={terminal},status=",
                    expected.value(),
                    marker_state_name(expected.marker()),
                )?;
                match status {
                    Some(status) => write!(formatter, "{status:?}")?,
                    None => formatter.write_str("none")?,
                }
                formatter.write_str(",last_progress_frame=")?;
                match last_progress_frame {
                    Some(frame) => write!(formatter, "{frame}")?,
                    None => formatter.write_str("none")?,
                }
                formatter.write_str(",confirmed_observations=")?;
                match confirmed_observations {
                    Some(observations) => write!(formatter, "{observations}")?,
                    None => formatter.write_str("none")?,
                }
                formatter.write_str("}")
            }
            Self::WatchMismatch {
                expected,
                frame,
                exact_source_match,
                token_observation,
            } => write!(
                formatter,
                "watch_mismatch{{expected_token={},expected_marker={},frame={frame},exact_source_match={exact_source_match},token_observation={token_observation}}}",
                expected.value(),
                marker_state_name(expected.marker()),
            ),
        }
    }
}

#[derive(Debug)]
struct Record {
    iteration: usize,
    stage: Stage,
    outcome: Outcome,
    status: Option<Status>,
    old_frame: Option<FrameStamp>,
    fresh_frame: Option<FrameStamp>,
    failure_evidence: Option<FailureEvidence>,
    elapsed_ms: u128,
}

impl Record {
    const fn new(iteration: usize, stage: Stage, outcome: Outcome) -> Self {
        Self {
            iteration,
            stage,
            outcome,
            status: None,
            old_frame: None,
            fresh_frame: None,
            failure_evidence: None,
            elapsed_ms: 0,
        }
    }
    fn with_elapsed(mut self, elapsed: Duration) -> Self {
        self.elapsed_ms = elapsed.as_millis();
        self
    }

    fn with_status(mut self, status: Status) -> Self {
        self.status = Some(status);
        self
    }
    fn with_failure_evidence(mut self, evidence: FailureEvidence) -> Self {
        self.failure_evidence = Some(evidence);
        self
    }

    fn with_old_frame(mut self, old_frame: FrameStamp) -> Self {
        self.old_frame = Some(old_frame);
        self
    }

    fn with_frames(mut self, old_frame: FrameStamp, fresh_frame: FrameStamp) -> Self {
        self.old_frame = Some(old_frame);
        self.fresh_frame = Some(fresh_frame);
        self
    }
}

struct Config {
    iterations: usize,
    retention: RetentionVariant,
    diagnostic_tier: u32,
    source_revision: String,
    source_tree: String,
}

impl Config {
    fn parse(arguments: &Arguments) -> Result<Self, ()> {
        let iterations = unique_value(&arguments.raw, "--root-cause-iterations")?
            .map(str::parse::<usize>)
            .transpose()
            .map_err(|_| ())?
            .unwrap_or(DEFAULT_ITERATIONS);
        if iterations == 0 || iterations > MAX_ITERATIONS {
            return Err(());
        }
        let retention = match unique_value(&arguments.raw, "--root-cause-retention")? {
            None | Some("native-frame") => RetentionVariant::NativeFrame,
            Some("none") => RetentionVariant::None,
            Some("cpu-mapping") => RetentionVariant::CpuMapping,
            Some(_) => return Err(()),
        };
        let diagnostic_tier = unique_value(&arguments.raw, "--root-cause-diagnostic-tier")?
            .map(str::parse::<u32>)
            .transpose()
            .map_err(|_| ())?
            .unwrap_or(0);
        if diagnostic_tier > 2 {
            return Err(());
        }
        let source_revision = unique_value(&arguments.raw, "--source-revision")?
            .filter(|value| is_hex_identity(value, 40))
            .ok_or(())?
            .to_owned();
        let source_tree = unique_value(&arguments.raw, "--source-tree")?
            .filter(|value| is_hex_identity(value, 40))
            .ok_or(())?
            .to_owned();
        Ok(Self {
            iterations,
            retention,
            diagnostic_tier,
            source_revision,
            source_tree,
        })
    }
}

#[derive(Debug, Clone, Copy)]
enum PrefixFailure {
    Workload(&'static str),
    Panic,
}

impl PrefixFailure {
    const fn workload(self) -> &'static str {
        match self {
            Self::Workload(workload) => workload,
            Self::Panic => "panic",
        }
    }
}

struct PersistentFixture {
    fixture: NativeFixture,
    engine: Engine,
    target: TargetId,
    last_ack: ControlAcknowledgement,
}

pub(super) fn run(arguments: Arguments) {
    let config = Config::parse(&arguments).unwrap_or_else(|()| panic!("protocol_drift"));
    if let Err(status) =
        mado_pilot_platform_macos::sck_diagnostics::set_tier(config.diagnostic_tier)
    {
        println!(
            "root_cause_version=1 event=diagnostic_setup outcome=failed native_status={status}",
        );
        print_final(0, 1, Outcome::ApparatusFailure);
        panic!("root_cause_failed");
    }
    println!(
        "root_cause_version=1 event=start source_revision={} source_tree={} iterations={} retention={} target_mode=retained diagnostic_tier={} max_run_ms={} operation_ms={} token_observation_ms={} teardown_ms={} ring_capacity={}",
        config.source_revision,
        config.source_tree,
        config.iterations,
        config.retention.as_str(),
        config.diagnostic_tier,
        MAX_RUN_TIME.as_millis(),
        OPERATION_WAIT.as_millis(),
        OPERATION_WAIT.as_millis(),
        OPERATION_WAIT.as_millis(),
        DIAGNOSTIC_RING_CAPACITY,
    );

    let authority = arguments.clone();
    let started = Instant::now();
    let deadline = started + MAX_RUN_TIME;
    let mut persistent = match run_prefix(arguments) {
        Ok(persistent) => persistent,
        Err(failure) => {
            println!(
                "root_cause_version=1 event=prefix outcome=failed workload={}",
                failure.workload(),
            );
            let record = Record::new(0, Stage::Prefix, Outcome::PrefixFailure)
                .with_elapsed(started.elapsed());
            print_record("failure", &record);
            dump_sck_diagnostics();
            print_final(0, 1, Outcome::PrefixFailure);
            panic!("root_cause_failed");
        }
    };
    println!("root_cause_version=1 event=prefix outcome=clean workloads=21");

    let mut ring = VecDeque::with_capacity(DIAGNOSTIC_RING_CAPACITY);
    let mut completed = 0_usize;
    let mut failures = 0_usize;
    let mut final_outcome = Outcome::Clean;

    for iteration in 1..=config.iterations {
        if Instant::now() >= deadline {
            let record = Record::new(iteration, Stage::OverallRun, Outcome::RunBoundReached)
                .with_elapsed(started.elapsed());
            push_record(&mut ring, record);
            failures = failures.saturating_add(1);
            final_outcome = Outcome::RunBoundReached;
            break;
        }
        let iteration_started = Instant::now();
        let record = run_iteration(iteration, &mut persistent, config.retention)
            .with_elapsed(iteration_started.elapsed());
        let clean = record.outcome == Outcome::Clean;
        print_record("iteration", &record);
        push_record(&mut ring, record);
        if clean {
            completed = completed.saturating_add(1);
        } else {
            failures = failures.saturating_add(1);
            final_outcome = ring
                .back()
                .map_or(Outcome::ApparatusFailure, |record| record.outcome);
            print_ring(&ring);
            break;
        }
    }

    let PersistentFixture {
        mut fixture,
        engine,
        ..
    } = persistent;
    drop(engine);
    let finalization = fixture.finish();
    if !finalization.is_accepted() {
        let record = Record::new(
            completed.saturating_add(1),
            Stage::FixtureFinalize,
            Outcome::ApparatusFailure,
        )
        .with_elapsed(started.elapsed());
        print_record("failure", &record);
        push_record(&mut ring, record);
        failures = failures.saturating_add(1);
        final_outcome = Outcome::ApparatusFailure;
    }
    drop(fixture);

    if !artifact_identities_match(&authority) {
        let record = Record::new(
            completed,
            Stage::ArtifactAuthority,
            Outcome::ApparatusFailure,
        )
        .with_elapsed(started.elapsed());
        print_record("failure", &record);
        push_record(&mut ring, record);
        failures = failures.saturating_add(1);
        final_outcome = Outcome::ApparatusFailure;
    }

    if failures == 0 && completed == config.iterations {
        final_outcome = Outcome::Clean;
    }
    dump_sck_diagnostics();
    print_final(completed, failures, final_outcome);
    assert_eq!(failures, 0, "root_cause_failed");
}

fn unique_value<'a>(arguments: &'a [String], name: &str) -> Result<Option<&'a str>, ()> {
    let prefix = format!("{name}=");
    let mut values = arguments
        .iter()
        .filter_map(|argument| argument.strip_prefix(&prefix));
    let first = values.next();
    if values.next().is_some() || first.is_some_and(str::is_empty) {
        return Err(());
    }
    Ok(first)
}

fn is_hex_identity(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn run_prefix(arguments: Arguments) -> Result<PersistentFixture, PrefixFailure> {
    catch_unwind(AssertUnwindSafe(|| {
        let main = Rc::new(RefCell::new(Cohort::new(arguments.clone())));
        for (name, operation) in PREFIX_WORKLOADS {
            run_prefix_workload(&main, name, operation)?;
        }

        let sacrificial = Rc::new(RefCell::new(Cohort::new(arguments)));
        run_prefix_workload(
            &sacrificial,
            "native_stop_target_loss",
            native_stop_target_loss,
        )?;
        if !sacrificial.borrow_mut().finish() {
            return Err(PrefixFailure::Workload("native_stop_target_loss"));
        }

        main.borrow_mut().settled_mapping = None;
        let run = main
            .borrow_mut()
            .run
            .take()
            .ok_or(PrefixFailure::Workload("session_engine_close"))?;
        close_main_prefix(run).map_err(|_| PrefixFailure::Workload("session_engine_close"))
    }))
    .map_err(|_| PrefixFailure::Panic)?
}

fn run_prefix_workload(
    cohort: &Rc<RefCell<Cohort>>,
    name: &'static str,
    operation: PrefixOperation,
) -> Result<(), PrefixFailure> {
    let mut workloads = Vec::with_capacity(1);
    let plan = if sampled_workload(name) {
        Plan::new(3, 20)
    } else {
        Plan::new(0, 1)
    };
    add_workload(
        &mut workloads,
        name,
        "same-process historical prefix",
        plan,
        cohort,
        operation,
    );
    let workload = workloads.pop().ok_or(PrefixFailure::Workload(name))?;
    if workload.incorrect() != 0 {
        return Err(PrefixFailure::Workload(name));
    }
    Ok(())
}

fn close_main_prefix(mut run: NativeRun) -> Result<PersistentFixture, String> {
    let _absent = establish_absent(&mut run)?;
    let session_query = run.start_watch(TemplateStability::immediate())?;
    prime_pending(&session_query)?;
    let first = run.session.close(&bounded(OPERATION_WAIT)).is_ok();
    let second = run.session.close(&bounded(OPERATION_WAIT)).is_ok();
    let (session_terminal, _) = wait_terminal(&session_query)?;
    let session_stable = Arc::ptr_eq(&session_terminal, &wait_terminal(&session_query)?.0);
    let session_closed = matches!(&*session_terminal, TemplateTerminalOutcome::SessionClosed);
    let session_metrics = terminal_query_metrics(&session_query, session_closed)?;

    let engine_session = run
        .engine
        .open(run.target, &OpenRequest::new(), &bounded(OPERATION_WAIT))
        .map_err(|error| format!("typed_operation_failure:{:?}", error.status()))?;
    let expected = run.issue_visual_state(VisualMarkerState::Absent)?;
    let mut engine_observation = SessionObservation::default();
    let mut synchronization = [SessionSynchronization::new(
        &engine_session,
        &mut engine_observation,
    )];
    synchronize_sessions(
        &run.fixture,
        run.target,
        expected,
        &mut synchronization,
        Instant::now() + OPERATION_WAIT,
    )
    .map_err(|error| error.to_string())?;
    let _engine_frame = synchronization[0]
        .frame
        .take()
        .ok_or_else(|| "fixture_authority_failed".to_owned())?;
    let engine_query = engine_session
        .start_template_watch(TemplateWatchRequest::new(
            run.template.clone(),
            MatchOptions::from_defaults(run.template.defaults()),
            OperationContext::new(),
        ))
        .map_err(|error| format!("typed_operation_failure:{:?}", error.status()))?;
    prime_pending(&engine_query)?;

    let NativeRun {
        fixture: OwnedNativeFixture(fixture),
        engine,
        session,
        template,
        last_ack,
        ..
    } = run;
    drop(engine);
    let (engine_terminal, _) = wait_terminal(&engine_query)?;
    let engine_stable = Arc::ptr_eq(&engine_terminal, &wait_terminal(&engine_query)?.0);
    let scheduler_closed = matches!(&*engine_terminal, TemplateTerminalOutcome::SchedulerClosed);
    let engine_metrics = terminal_query_metrics(&engine_query, scheduler_closed)?;
    let engine_session_closed = close_twice(&engine_session).is_ok();
    wait_for_backend_idle(OPERATION_WAIT)?;
    drop(session_query);
    drop(session_terminal);
    drop(engine_query);
    drop(engine_terminal);
    drop(engine_session);
    drop(session);
    drop(template);

    let correct = first
        && second
        && session_stable
        && session_closed
        && engine_stable
        && scheduler_closed
        && engine_session_closed
        && session_metrics
            .saturating_add(engine_metrics)
            .producer_publications
            > 0;
    if !correct {
        return Err("prefix_failed".to_owned());
    }
    let retained_engine =
        native_engine().map_err(|error| format!("typed_operation_failure:{:?}", error.status()))?;
    let retained_target = fixture.authenticated_target(&retained_engine)?;
    Ok(PersistentFixture {
        fixture,
        engine: retained_engine,
        target: retained_target,
        last_ack,
    })
}

fn run_iteration(
    iteration: usize,
    persistent: &mut PersistentFixture,
    retention: RetentionVariant,
) -> Record {
    let engine = &persistent.engine;
    let target = persistent.target;
    let old_session = match engine.open(target, &OpenRequest::new(), &bounded(OPERATION_WAIT)) {
        Ok(session) => session,
        Err(error) => {
            return Record::new(iteration, Stage::OldOpen, Outcome::OpenRejected)
                .with_status(error.status());
        }
    };
    let old_stream = old_session.stream();

    let absent = match issue_fixture_visual_state(
        &mut persistent.fixture,
        &mut persistent.last_ack,
        VisualMarkerState::Absent,
    ) {
        Ok(token) => token,
        Err(_) => {
            let _ = close_twice(&old_session);
            return Record::new(iteration, Stage::OldToken, Outcome::ApparatusFailure);
        }
    };
    let mut old_observation = SessionObservation::default();
    let absent_frame = match synchronize_session_to_token_diagnostic(
        &persistent.fixture,
        target,
        &old_session,
        &mut old_observation,
        absent,
    ) {
        Ok(frame) => frame,
        Err(error) => {
            let _ = close_twice(&old_session);
            return Record::new(iteration, Stage::OldToken, Outcome::ProducerProgressFailure)
                .with_failure_evidence(FailureEvidence::TokenSynchronization(error));
        }
    };
    let last_old_stamp = absent_frame.stamp();
    let shape = match marker_shape(&absent_frame, &persistent.fixture) {
        Some(shape) => shape,
        None => {
            let _ = close_twice(&old_session);
            return Record::new(iteration, Stage::OldWatch, Outcome::ApparatusFailure)
                .with_old_frame(last_old_stamp);
        }
    };
    drop(absent_frame);
    let template = match prepare_marker(engine, shape, "root-cause-watch") {
        Ok(template) => template,
        Err(_) => {
            let _ = close_twice(&old_session);
            return Record::new(iteration, Stage::OldWatch, Outcome::ApparatusFailure)
                .with_old_frame(last_old_stamp);
        }
    };
    let query_operation = match OperationContext::new().with_timeout(OPERATION_WAIT) {
        Ok(operation) => operation,
        Err(_) => {
            let _ = close_twice(&old_session);
            return Record::new(iteration, Stage::OldWatch, Outcome::ApparatusFailure)
                .with_old_frame(last_old_stamp);
        }
    };
    let query = match old_session.start_template_watch(
        TemplateWatchRequest::new(
            template.clone(),
            MatchOptions::from_defaults(template.defaults()),
            query_operation,
        )
        .with_stability(TemplateStability::immediate())
        .with_change_policy(ChangeDetectionPolicy::default()),
    ) {
        Ok(query) => query,
        Err(_) => {
            let _ = close_twice(&old_session);
            return Record::new(iteration, Stage::OldWatch, Outcome::ApparatusFailure)
                .with_old_frame(last_old_stamp);
        }
    };
    let visible = match issue_fixture_visual_state(
        &mut persistent.fixture,
        &mut persistent.last_ack,
        VisualMarkerState::Visible,
    ) {
        Ok(token) => token,
        Err(_) => {
            let _ = query.cancel();
            let _ = close_twice(&old_session);
            return Record::new(iteration, Stage::OldToken, Outcome::ApparatusFailure)
                .with_old_frame(last_old_stamp);
        }
    };
    let (terminal, progress) = match wait_terminal_bounded(&query, OPERATION_WAIT) {
        Ok(terminal) => terminal,
        Err(error) => {
            let _ = query.cancel();
            let _ = close_twice(&old_session);
            return Record::new(iteration, Stage::OldToken, Outcome::ProducerProgressFailure)
                .with_old_frame(last_old_stamp)
                .with_failure_evidence(FailureEvidence::WatchWait {
                    expected: visible,
                    error,
                });
        }
    };
    let Some(result) = terminal_match(&terminal).cloned() else {
        let status = terminal.status();
        let mut record = Record::new(iteration, Stage::OldToken, Outcome::ProducerProgressFailure)
            .with_old_frame(last_old_stamp)
            .with_failure_evidence(FailureEvidence::WatchTerminal {
                expected: visible,
                terminal: terminal_outcome_name(&terminal),
                status,
                last_progress_frame: progress.and_then(TemplateQueryProgress::last_frame),
                confirmed_observations: progress.map(TemplateQueryProgress::confirmed_observations),
            });
        if let Some(status) = status {
            record = record.with_status(status);
        }
        let _ = close_twice(&old_session);
        return record;
    };
    let exact_source_match = matched_target_exact(&terminal, target, template.id(), shape, 1, None);
    let token_observation =
        match frame_has_visual_token(&old_session, &persistent.fixture, result.frame(), visible) {
            Ok(true) => WatchTokenObservation::Matched,
            Ok(false) => WatchTokenObservation::Mismatched,
            Err(error) => WatchTokenObservation::Failed(error),
        };
    if !exact_source_match || !matches!(&token_observation, WatchTokenObservation::Matched) {
        let result_frame = result.frame().stamp();
        let _ = close_twice(&old_session);
        return Record::new(iteration, Stage::OldToken, Outcome::ProducerProgressFailure)
            .with_old_frame(last_old_stamp)
            .with_failure_evidence(FailureEvidence::WatchMismatch {
                expected: visible,
                frame: result_frame,
                exact_source_match,
                token_observation,
            });
    }

    let old_stamp = result.frame().stamp();
    let mapping_observer = old_session.mapping_observer();
    let mapping =
        match old_session.map_frame(result.frame(), PixelFormat::Rgba8, &bounded(OPERATION_WAIT)) {
            Ok(mapping) => mapping,
            Err(_) => {
                let _ = close_twice(&old_session);
                return Record::new(iteration, Stage::OldRetention, Outcome::OwnershipFailure)
                    .with_old_frame(old_stamp);
            }
        };
    let mapping_stamp = mapping.stamp();
    let sentinel_length = mapping.bytes().len().min(MAPPING_SENTINEL_CAPACITY);
    let mut sentinel = [0_u8; MAPPING_SENTINEL_CAPACITY];
    sentinel[..sentinel_length].copy_from_slice(&mapping.bytes()[..sentinel_length]);
    let retained_result = (retention == RetentionVariant::NativeFrame).then_some(result);
    let retained_mapping = (retention == RetentionVariant::CpuMapping).then_some(mapping);

    drop(terminal);
    drop(query);
    if let Err(status) = close_twice(&old_session) {
        return Record::new(iteration, Stage::OldTeardown, Outcome::TeardownFailure)
            .with_status(status)
            .with_old_frame(old_stamp);
    }
    drop(old_session);
    drop(template);
    if wait_for_backend_idle(OPERATION_WAIT).is_err() {
        return Record::new(iteration, Stage::OldTeardown, Outcome::TeardownFailure)
            .with_old_frame(old_stamp);
    }

    let fresh_session = match engine.open(target, &OpenRequest::new(), &bounded(OPERATION_WAIT)) {
        Ok(session) => session,
        Err(error) => {
            return Record::new(iteration, Stage::FreshOpen, Outcome::OpenRejected)
                .with_status(error.status())
                .with_old_frame(old_stamp);
        }
    };
    if fresh_session.stream() == old_stream {
        let _ = close_twice(&fresh_session);
        return Record::new(iteration, Stage::FreshRetention, Outcome::OwnershipFailure)
            .with_old_frame(old_stamp);
    }
    let fresh_token = match issue_fixture_visual_state(
        &mut persistent.fixture,
        &mut persistent.last_ack,
        VisualMarkerState::Absent,
    ) {
        Ok(token) => token,
        Err(_) => {
            let _ = close_twice(&fresh_session);
            return Record::new(iteration, Stage::FreshToken, Outcome::ApparatusFailure)
                .with_old_frame(old_stamp);
        }
    };
    let mut fresh_observation = SessionObservation::default();
    let fresh_frame = match synchronize_session_to_token_diagnostic(
        &persistent.fixture,
        target,
        &fresh_session,
        &mut fresh_observation,
        fresh_token,
    ) {
        Ok(frame) => frame,
        Err(error) => {
            let _ = close_twice(&fresh_session);
            return Record::new(
                iteration,
                Stage::FreshToken,
                Outcome::ProducerProgressFailure,
            )
            .with_old_frame(old_stamp)
            .with_failure_evidence(FailureEvidence::TokenSynchronization(error));
        }
    };
    let fresh_stamp = fresh_frame.stamp();
    drop(fresh_frame);

    if let Some(retained_result) = retained_result {
        let readable = retained_result.frame().stamp() == old_stamp
            && mapping_observer
                .map_frame(
                    retained_result.frame(),
                    PixelFormat::Rgba8,
                    &bounded(OPERATION_WAIT),
                )
                .is_ok();
        if !readable {
            let _ = close_twice(&fresh_session);
            return Record::new(iteration, Stage::FreshRetention, Outcome::OwnershipFailure)
                .with_frames(old_stamp, fresh_stamp);
        }
    }
    if let Some(retained_mapping) = retained_mapping
        && (retained_mapping.stamp() != mapping_stamp
            || !retained_mapping
                .bytes()
                .starts_with(&sentinel[..sentinel_length]))
    {
        let _ = close_twice(&fresh_session);
        return Record::new(iteration, Stage::FreshRetention, Outcome::OwnershipFailure)
            .with_frames(old_stamp, fresh_stamp);
    }
    drop(mapping_observer);

    if let Err(status) = close_twice(&fresh_session) {
        return Record::new(iteration, Stage::FreshTeardown, Outcome::TeardownFailure)
            .with_status(status)
            .with_frames(old_stamp, fresh_stamp);
    }
    drop(fresh_session);
    Record::new(iteration, Stage::FreshTeardown, Outcome::Clean).with_frames(old_stamp, fresh_stamp)
}

fn terminal_outcome_name(outcome: &TemplateTerminalOutcome) -> &'static str {
    match outcome {
        TemplateTerminalOutcome::Matched(_) => "matched",
        TemplateTerminalOutcome::Cancelled => "cancelled",
        TemplateTerminalOutcome::DeadlineExceeded => "deadline_exceeded",
        TemplateTerminalOutcome::SessionClosed => "session_closed",
        TemplateTerminalOutcome::SchedulerClosed => "scheduler_closed",
        TemplateTerminalOutcome::TargetLost => "target_lost",
        TemplateTerminalOutcome::Overloaded(_) => "overloaded",
        TemplateTerminalOutcome::Failed(_) => "failed",
        _ => "unknown",
    }
}

fn close_twice(session: &Session) -> Result<(), Status> {
    let first = session
        .close(&bounded(OPERATION_WAIT))
        .err()
        .map(|error| error.status());
    let second = session
        .close(&bounded(OPERATION_WAIT))
        .err()
        .map(|error| error.status());
    first.or(second).map_or(Ok(()), Err)
}

fn push_record(ring: &mut VecDeque<Record>, record: Record) {
    if ring.len() == DIAGNOSTIC_RING_CAPACITY {
        let _ = ring.pop_front();
    }
    ring.push_back(record);
}

struct RecordOutput<'record> {
    event: &'static str,
    record: &'record Record,
}

impl fmt::Display for RecordOutput<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let record = self.record;
        write!(
            formatter,
            "root_cause_version=1 event={} iteration={} stage={} outcome={} status=",
            self.event,
            record.iteration,
            record.stage.as_str(),
            record.outcome.as_str(),
        )?;
        match record.status {
            Some(status) => write!(formatter, "{status:?}")?,
            None => formatter.write_str("none")?,
        }
        formatter.write_str(" old_frame=")?;
        match record.old_frame {
            Some(stamp) => write!(formatter, "{stamp}")?,
            None => formatter.write_str("none")?,
        }
        formatter.write_str(" fresh_frame=")?;
        match record.fresh_frame {
            Some(stamp) => write!(formatter, "{stamp}")?,
            None => formatter.write_str("none")?,
        }
        write!(formatter, " elapsed_ms={}", record.elapsed_ms)?;
        formatter.write_str(" failure_evidence=")?;
        match &record.failure_evidence {
            Some(evidence) => write!(formatter, "{evidence}"),
            None => formatter.write_str("none"),
        }
    }
}

fn print_record(event: &'static str, record: &Record) {
    println!("{}", RecordOutput { event, record });
}

fn print_ring(ring: &VecDeque<Record>) {
    for record in ring {
        print_record("diagnostic", record);
    }
}

fn dump_sck_diagnostics() {
    if let Err(status) = mado_pilot_platform_macos::sck_diagnostics::dump() {
        println!(
            "root_cause_version=1 event=diagnostic_dump outcome=failed native_status={status}",
        );
    }
}

fn print_final(completed: usize, failures: usize, outcome: Outcome) {
    let peak_resident_bytes =
        peak_resident_bytes().map_or_else(|| "unavailable".to_owned(), |bytes| bytes.to_string());
    println!(
        "root_cause_version=1 event=final completed={} failures={} outcome={} peak_resident_bytes={}",
        completed,
        failures,
        outcome.as_str(),
        peak_resident_bytes,
    );
}

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;
    use mado_pilot_runtime::{FrameSequence, GeometryRevision, IdentityIssuer, StreamEpoch};

    fn frame_stamp() -> FrameStamp {
        let issuer = IdentityIssuer::new();
        FrameStamp::new(
            issuer.issue_stream().expect("issued stream"),
            StreamEpoch::FIRST,
            FrameSequence::FIRST,
            GeometryRevision::FIRST,
        )
    }

    #[test]
    fn synchronization_failure_record_preserves_bounded_observation() {
        let expected =
            VisualToken::new(42, VisualMarkerState::Absent).expect("nonzero expected token");
        let observed =
            VisualToken::new(41, VisualMarkerState::Visible).expect("nonzero observed token");
        let last_frame = frame_stamp();
        let error = TokenSynchronizationError {
            failure: TokenSynchronizationFailure::Timeout,
            expected,
            sessions: vec![SessionObservationDiagnostic {
                session: 0,
                last_frame: Some(last_frame),
                last_token: Some(observed),
                last_decode_failure: None,
                acquisition_attempt_count: 7,
                publication_count: 5,
                mapping_attempt_count: 4,
                decode_attempt_count: 3,
                remaining_micros_at_last_publication: Some(17),
                last_status: BoundedNativeStatus::DeadlineExceeded,
                closed: false,
            }],
            elapsed: Duration::from_micros(23),
        };
        let record = Record::new(9, Stage::FreshToken, Outcome::ProducerProgressFailure)
            .with_failure_evidence(FailureEvidence::TokenSynchronization(error));

        let output = RecordOutput {
            event: "failure",
            record: &record,
        }
        .to_string();

        assert!(output.contains("stage=fresh_token outcome=producer_progress_failure"));
        assert!(output.contains("expected_token=42"));
        assert!(output.contains(&format!("last_frame={last_frame}")));
        assert!(output.contains("last_token=41"));
        assert!(output.contains("acquisition_attempts=7,publications=5"));
        assert!(output.contains("mapping_attempts=4,decode_attempts=3"));
        assert!(output.contains("last_status=deadline_exceeded"));
        assert!(!output.contains('\n'));
        assert!(output.len() < 2_048);
    }

    #[test]
    fn watch_mismatch_record_preserves_expected_token_and_frame() {
        let expected =
            VisualToken::new(43, VisualMarkerState::Visible).expect("nonzero expected token");
        let frame = frame_stamp();
        let record = Record::new(4, Stage::OldToken, Outcome::ProducerProgressFailure)
            .with_failure_evidence(FailureEvidence::WatchMismatch {
                expected,
                frame,
                exact_source_match: true,
                token_observation: WatchTokenObservation::Mismatched,
            });

        let output = RecordOutput {
            event: "failure",
            record: &record,
        }
        .to_string();

        assert!(output.contains("expected_token=43"));
        assert!(output.contains(&format!("frame={frame}")));
        assert!(output.contains("exact_source_match=true"));
        assert!(output.contains("token_observation=mismatched"));
        assert!(!output.contains('\n'));
        assert!(output.len() < 1_024);
    }
}
