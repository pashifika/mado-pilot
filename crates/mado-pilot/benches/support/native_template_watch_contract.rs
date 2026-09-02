//! Required one-shot native template-watch contract (Qualification Lane B).

use super::*;
use mado_pilot_testkit::native_watch_contract_report::{
    self as report, CleanupDebtFact, DiagnosticFacts, ExecutionOutcome, FailureFacts, FailureKind,
    FailureStage, FrameIdentityFacts, LifecycleFact, NativeContractReport, NativePlatform,
    NativeStatus, ProcessLifetimeFact, ResourceFacts, ScenarioFact, ScenarioName, ScenarioResult,
    ScenarioTiming,
};

const EXIT_PRODUCT_FAILURE: i32 = 1;
const EXIT_INFRASTRUCTURE: i32 = 2;
const EXIT_UNSUPPORTED: i32 = 3;

pub(super) fn run() {
    let arguments = match Arguments::parse_contract() {
        Ok(arguments) => arguments,
        Err(failure) => {
            let stage = match failure {
                ContractArgumentFailure::Protocol => FailureStage::Protocol,
                ContractArgumentFailure::FixtureUnavailable => FailureStage::FixtureLaunch,
            };
            emit_and_exit(NativeContractReport::not_executed(
                platform(),
                ExecutionOutcome::Infra,
                failure_facts(FailureKind::Authority, stage, NativeStatus::None),
                Vec::new(),
            ));
            return;
        }
    };
    debug_assert!(arguments.native_contract);

    let mut scenarios = Vec::with_capacity(report::SCENARIOS.len());
    for execute in [
        admission as fn(&Arguments) -> ScenarioAttempt,
        post_open_token_synchronization,
        watcher_match_correlation,
        two_session_fairness_contract,
        geometry_generation_contract,
        retained_ownership_and_fresh_session,
        lifecycle_termination_contract,
        cleanup_baseline,
    ] {
        match execute(&arguments) {
            ScenarioAttempt::Completed(result) => {
                let passed = result.semantic.is_pass() && result.cleanup.is_pass();
                scenarios.push(result);
                if !passed {
                    emit_and_exit(NativeContractReport::executed(platform(), scenarios));
                    return;
                }
            }
            ScenarioAttempt::Apparatus { outcome, failure } => {
                emit_and_exit(NativeContractReport::not_executed(
                    platform(),
                    outcome,
                    failure,
                    scenarios,
                ));
                return;
            }
        }
    }

    emit_and_exit(NativeContractReport::executed(platform(), scenarios));
}

fn emit_and_exit(contract: NativeContractReport) {
    let contract = if contract.validate().is_ok() {
        contract
    } else {
        NativeContractReport::not_executed(
            platform(),
            ExecutionOutcome::Infra,
            failure_facts(
                FailureKind::Protocol,
                FailureStage::Protocol,
                NativeStatus::Internal,
            ),
            Vec::new(),
        )
    };
    let json = contract
        .to_json()
        .expect("the closed typed native contract schema must serialize");
    println!("{json}");
    let code = match contract.outcome {
        ExecutionOutcome::Pass => return,
        ExecutionOutcome::Fail => EXIT_PRODUCT_FAILURE,
        ExecutionOutcome::Infra => EXIT_INFRASTRUCTURE,
        ExecutionOutcome::Unsupported => EXIT_UNSUPPORTED,
    };
    std::process::exit(code);
}

#[cfg(windows)]
const fn platform() -> NativePlatform {
    NativePlatform::WindowsX86_64
}

#[cfg(target_os = "macos")]
const fn platform() -> NativePlatform {
    NativePlatform::MacosAarch64
}

#[expect(
    clippy::large_enum_variant,
    reason = "one-shot qualification state stays stack-owned and avoids an extra allocation"
)]
enum ScenarioAttempt {
    Completed(ScenarioResult),
    Apparatus {
        outcome: ExecutionOutcome,
        failure: FailureFacts,
    },
}

#[derive(Default)]
struct ScenarioEvidence {
    expected_token: Option<VisualToken>,
    observed_token: Option<VisualToken>,
    frame: Option<FrameStamp>,
    prior_target_scale_milli: Option<[u32; 2]>,
    target_scale_milli: Option<[u32; 2]>,
    teardown_started: Option<Instant>,
    watch_elapsed: Option<Duration>,
    lifecycle: Option<LifecycleFact>,
    extra_acquisitions: u64,
    extra_publications: u64,
    extra_mappings: u64,
    extra_decodes: u64,
    status: Option<NativeStatus>,
    cleanup_failure: Option<FailureFacts>,
}

#[derive(Clone, Copy)]
struct TokenMatchAuthority<'a> {
    target: TargetId,
    template: &'a TemplateId,
    shape: MarkerShape,
    expected: VisualToken,
    stage: FailureStage,
}

fn admission(arguments: &Arguments) -> ScenarioAttempt {
    let mut fixture = match NativeFixture::start(arguments) {
        Ok(fixture) => fixture,
        Err(_) => return apparatus(ExecutionOutcome::Infra, FailureStage::FixtureLaunch),
    };
    let engine = match native_engine() {
        Ok(engine) => engine,
        Err(error) => {
            let outcome = if error.status() == Status::Unsupported {
                ExecutionOutcome::Unsupported
            } else {
                ExecutionOutcome::Infra
            };
            let _finalization = fixture.finish();
            return ScenarioAttempt::Apparatus {
                outcome,
                failure: failure_facts(
                    FailureKind::Authority,
                    FailureStage::EngineCreate,
                    native_status(error.status()),
                ),
            };
        }
    };
    if !permission_oracle(&engine) {
        drop(engine);
        let _finalization = fixture.finish();
        return ScenarioAttempt::Apparatus {
            outcome: ExecutionOutcome::Unsupported,
            failure: failure_facts(
                FailureKind::Authority,
                FailureStage::PermissionAdmission,
                NativeStatus::Unsupported,
            ),
        };
    }
    if fixture.authenticated_target(&engine).is_err() {
        drop(engine);
        let _finalization = fixture.finish();
        return apparatus(ExecutionOutcome::Infra, FailureStage::TargetDiscovery);
    }

    let teardown_started = Instant::now();
    drop(engine);
    let finalization = fixture.finish();
    let resources = finalization.resources();
    let accepted = finalization.is_accepted();
    ScenarioAttempt::Completed(ScenarioResult {
        name: ScenarioName::Admission,
        semantic: ScenarioFact::Pass,
        cleanup: cleanup_fact(accepted, resources),
        timing: ScenarioTiming {
            startup_micros: None,
            watch_micros: None,
            teardown_micros: Some(micros(teardown_started.elapsed())),
        },
        diagnostics: DiagnosticFacts {
            lifecycle: if accepted {
                LifecycleFact::Released
            } else {
                LifecycleFact::NotStarted
            },
            resources: resource_facts(resources),
            ..DiagnosticFacts::default()
        },
    })
}

fn post_open_token_synchronization(arguments: &Arguments) -> ScenarioAttempt {
    execute_standard(
        arguments,
        ScenarioName::PostOpenTokenSynchronization,
        |run, evidence| {
            evidence.expected_token = Some(run.readiness_token);
            evidence.observed_token = run.observation.last_token;
            evidence.frame = Some(run.readiness_stamp);
            evidence.target_scale_milli = run.readiness_scale_milli;
            Ok(())
        },
    )
}

fn watcher_match_correlation(arguments: &Arguments) -> ScenarioAttempt {
    execute_standard(
        arguments,
        ScenarioName::WatcherMatchCorrelation,
        |run, evidence| {
            let (query, terminal) = match_once(run, evidence)?;
            drop(terminal);
            drop(query);
            Ok(())
        },
    )
}

fn two_session_fairness_contract(arguments: &Arguments) -> ScenarioAttempt {
    execute_standard(
        arguments,
        ScenarioName::TwoSessionFairness,
        |run, evidence| {
            let second_session = run
                .engine
                .open(run.target, &OpenRequest::new(), &bounded(OPERATION_WAIT))
                .map_err(|error| {
                    failure_facts(
                        FailureKind::Operation,
                        FailureStage::SessionOpen,
                        native_status(error.status()),
                    )
                })?;
            let mut second_observation = SessionObservation::default();
            let outcome = (|| {
                let absent = issue_fixture_visual_state(
                    &mut run.fixture,
                    &mut run.last_ack,
                    VisualMarkerState::Absent,
                )
                .map_err(|_| authority_failure(FailureStage::VisualStimulus))?;
                evidence.expected_token = Some(absent);
                let mut absent_sessions = [
                    SessionSynchronization::new(&run.session, &mut run.observation),
                    SessionSynchronization::new(&second_session, &mut second_observation),
                ];
                synchronize_sessions(
                    &run.fixture,
                    run.target,
                    absent,
                    &mut absent_sessions,
                    Instant::now() + OPERATION_WAIT,
                )
                .map_err(|error| {
                    synchronization_failure(&error, FailureStage::TwoSessionProgress)
                })?;

                let first = start_query(&run.session, run.template.clone())?;
                let second = start_query(&second_session, run.template.clone())?;
                let first_before = match wait_query_publication_typed(&first) {
                    Ok(before) => before,
                    Err(failure) => {
                        record_query_publication_total(&first, evidence);
                        return Err(failure);
                    }
                };
                let second_before = match wait_query_publication_typed(&second) {
                    Ok(before) => before,
                    Err(failure) => {
                        record_query_publication_total(&first, evidence);
                        record_query_publication_total(&second, evidence);
                        return Err(failure);
                    }
                };
                let visible = match issue_fixture_visual_state_timed(
                    &mut run.fixture,
                    &mut run.last_ack,
                    VisualMarkerState::Visible,
                ) {
                    Ok(visible) => visible,
                    Err(_) => {
                        record_query_publication_total(&first, evidence);
                        record_query_publication_total(&second, evidence);
                        return Err(authority_failure(FailureStage::VisualStimulus));
                    }
                };
                let watch_started = visible.acknowledged_at;
                let phase_deadline = watch_started + OPERATION_WAIT;
                let visible = visible.token;
                evidence.expected_token = Some(visible);
                let mut visible_sessions = [
                    SessionSynchronization::new(&run.session, &mut run.observation),
                    SessionSynchronization::new(&second_session, &mut second_observation),
                ];
                if let Err(error) = synchronize_sessions(
                    &run.fixture,
                    run.target,
                    visible,
                    &mut visible_sessions,
                    phase_deadline,
                ) {
                    record_query_publication_total(&first, evidence);
                    record_query_publication_total(&second, evidence);
                    return Err(synchronization_failure(
                        &error,
                        FailureStage::TwoSessionProgress,
                    ));
                }
                let terminals = (|| {
                    let first_terminal = wait_terminal_until_typed(
                        &first,
                        FailureStage::TwoSessionProgress,
                        phase_deadline,
                        evidence,
                    )?;
                    let second_terminal = wait_terminal_until_typed(
                        &second,
                        FailureStage::TwoSessionProgress,
                        phase_deadline,
                        evidence,
                    )?;
                    Ok((first_terminal, second_terminal))
                })();
                record_query_publication_total(&first, evidence);
                record_query_publication_total(&second, evidence);
                let (first_terminal, second_terminal) = terminals?;
                let first_exact = terminal_matches_token(
                    &first_terminal,
                    &run.session,
                    &run.fixture,
                    TokenMatchAuthority {
                        target: run.target,
                        template: run.template.id(),
                        shape: run.shape,
                        expected: visible,
                        stage: FailureStage::TwoSessionProgress,
                    },
                    evidence,
                )?;
                let second_exact = terminal_matches_token(
                    &second_terminal,
                    &second_session,
                    &run.fixture,
                    TokenMatchAuthority {
                        target: run.target,
                        template: run.template.id(),
                        shape: run.shape,
                        expected: visible,
                        stage: FailureStage::TwoSessionProgress,
                    },
                    evidence,
                )?;
                if !first_exact
                    || !second_exact
                    || first.benchmark_publication_count() <= first_before
                    || second.benchmark_publication_count() <= second_before
                {
                    return Err(oracle_failure(FailureStage::TwoSessionProgress));
                }
                evidence.observed_token = Some(visible);
                evidence.watch_elapsed = Some(watch_started.elapsed());
                drop(first_terminal);
                drop(second_terminal);
                drop(first);
                drop(second);
                Ok(())
            })();
            evidence.teardown_started = Some(Instant::now());
            let closed = second_session.close(&bounded(OPERATION_WAIT)).is_ok()
                && second_session.close(&bounded(OPERATION_WAIT)).is_ok();
            if !closed {
                evidence.cleanup_failure = Some(failure_facts(
                    FailureKind::Cleanup,
                    FailureStage::NativeCleanup,
                    NativeStatus::Closed,
                ));
            }
            add_observation(evidence, &second_observation);
            outcome
        },
    )
}

fn geometry_generation_contract(arguments: &Arguments) -> ScenarioAttempt {
    if !qualification_geometry_available() {
        return apparatus(
            ExecutionOutcome::Unsupported,
            FailureStage::GeometryTransition,
        );
    }
    execute_standard(
        arguments,
        ScenarioName::GeometryGeneration,
        |run, evidence| {
            let before = command_token(run, VisualMarkerState::Absent, evidence)?;
            evidence.prior_target_scale_milli = target_scale_milli(&before);
            let acknowledgement = qualification_geometry_transition(&mut run.fixture)
                .map_err(|_| authority_failure(FailureStage::GeometryTransition))?;
            run.accept_acknowledgement(acknowledgement)
                .map_err(|_| authority_failure(FailureStage::GeometryTransition))?;
            let after = command_token(run, VisualMarkerState::Absent, evidence)?;
            evidence.target_scale_milli = target_scale_milli(&after);
            if before.stamp().geometry() == after.stamp().geometry()
                || after.stamp().order(&before.stamp()) != Ok(FrameOrder::After)
                || !qualification_geometry_matches(&before, &after)
            {
                return Err(oracle_failure(FailureStage::GeometryTransition));
            }
            run.refresh_template(&after, "watch-marker-v2-contract-geometry")
                .map_err(|_| oracle_failure(FailureStage::TemplatePreparation))?;
            let (query, terminal) = match_once(run, evidence)?;
            let result = terminal_match(&terminal)
                .ok_or_else(|| oracle_failure(FailureStage::MatchCorrelation))?;
            if result.frame().stamp().geometry() != after.stamp().geometry() {
                return Err(oracle_failure(FailureStage::GeometryTransition));
            }
            let restore = qualification_restore_geometry(&mut run.fixture)
                .map_err(|_| authority_failure(FailureStage::GeometryTransition))?;
            run.accept_acknowledgement(restore)
                .map_err(|_| authority_failure(FailureStage::GeometryTransition))?;
            let restored = command_token(run, VisualMarkerState::Absent, evidence)?;
            if restored.stamp().geometry() == after.stamp().geometry() {
                return Err(oracle_failure(FailureStage::GeometryTransition));
            }
            drop(terminal);
            drop(query);
            Ok(())
        },
    )
}

#[cfg(windows)]
fn qualification_geometry_available() -> bool {
    monitor_facts().is_ok_and(|monitors| {
        monitors
            .iter()
            .any(|left| monitors.iter().any(|right| left.dpi != right.dpi))
    })
}

#[cfg(target_os = "macos")]
const fn qualification_geometry_available() -> bool {
    true
}

#[cfg(windows)]
fn qualification_geometry_transition(
    fixture: &mut NativeFixture,
) -> Result<ControlAcknowledgement, String> {
    fixture.move_next_display()
}

#[cfg(target_os = "macos")]
fn qualification_geometry_transition(
    fixture: &mut NativeFixture,
) -> Result<ControlAcknowledgement, String> {
    fixture.move_target()
}

#[cfg(windows)]
fn qualification_restore_geometry(
    fixture: &mut NativeFixture,
) -> Result<ControlAcknowledgement, String> {
    fixture.restore_placement()
}

#[cfg(target_os = "macos")]
fn qualification_restore_geometry(
    fixture: &mut NativeFixture,
) -> Result<ControlAcknowledgement, String> {
    fixture.move_target()
}

#[cfg(windows)]
fn qualification_geometry_matches(before: &Frame, after: &Frame) -> bool {
    topology_geometry_matches(before, after)
}

#[cfg(target_os = "macos")]
fn qualification_geometry_matches(before: &Frame, after: &Frame) -> bool {
    before.transform().target() != after.transform().target()
}

fn retained_ownership_and_fresh_session(arguments: &Arguments) -> ScenarioAttempt {
    let run = match start_contract(arguments, ScenarioName::RetainedOwnershipAndFreshSession) {
        StartAttempt::Ready(run) => run,
        StartAttempt::ScenarioFailure(result) => return ScenarioAttempt::Completed(result),
        StartAttempt::Apparatus { outcome, failure } => {
            return ScenarioAttempt::Apparatus { outcome, failure };
        }
    };
    let startup = run.startup_elapsed;
    let mut evidence = ScenarioEvidence::default();
    let NativeRun {
        fixture: OwnedNativeFixture(mut fixture),
        engine,
        target,
        session,
        template,
        shape,
        mut last_ack,
        mut observation,
        ..
    } = run;
    let mut cleanup_ok = true;
    let mut explicit_cleanup_observed = false;
    let semantic = (|| {
        let query = start_query(&session, template.clone())?;
        if let Err(failure) = wait_query_publication_typed(&query) {
            record_query_publication_total(&query, &mut evidence);
            return Err(failure);
        }
        let visible = match issue_fixture_visual_state_timed(
            &mut fixture,
            &mut last_ack,
            VisualMarkerState::Visible,
        ) {
            Ok(visible) => visible,
            Err(_) => {
                record_query_publication_total(&query, &mut evidence);
                return Err(authority_failure(FailureStage::VisualStimulus));
            }
        };
        evidence.expected_token = Some(visible.token);
        let phase_deadline = visible.acknowledged_at + OPERATION_WAIT;
        if let Err(failure) = synchronize_one_until_typed(
            &fixture,
            target,
            &session,
            &mut observation,
            visible.token,
            FailureStage::RetainedOwnership,
            phase_deadline,
        ) {
            record_query_publication_total(&query, &mut evidence);
            return Err(failure);
        }
        let terminal = wait_terminal_until_typed(
            &query,
            FailureStage::RetainedOwnership,
            phase_deadline,
            &mut evidence,
        );
        record_query_publication_total(&query, &mut evidence);
        let terminal = terminal?;
        if !terminal_matches_token(
            &terminal,
            &session,
            &fixture,
            TokenMatchAuthority {
                target,
                template: template.id(),
                shape,
                expected: visible.token,
                stage: FailureStage::RetainedOwnership,
            },
            &mut evidence,
        )? {
            return Err(oracle_failure(FailureStage::RetainedOwnership));
        }
        evidence.watch_elapsed = Some(visible.acknowledged_at.elapsed());
        let retained_result = terminal_match(&terminal)
            .ok_or_else(|| oracle_failure(FailureStage::RetainedOwnership))?
            .clone();
        let retained_stamp = retained_result.frame().stamp();
        let retained_observer = session.mapping_observer();
        cleanup_ok &= session.close(&bounded(OPERATION_WAIT)).is_ok();
        drop(query);
        drop(terminal);
        drop(session);

        let successor = engine
            .open(target, &OpenRequest::new(), &bounded(OPERATION_WAIT))
            .map_err(|error| operation_failure(FailureStage::FreshSession, error.status()))?;
        let successor_absent =
            issue_fixture_visual_state(&mut fixture, &mut last_ack, VisualMarkerState::Absent)
                .map_err(|_| authority_failure(FailureStage::FreshSession))?;
        evidence.expected_token = Some(successor_absent);
        let mut successor_observation = SessionObservation::default();
        let successor_ready = synchronize_one_typed(
            &fixture,
            target,
            &successor,
            &mut successor_observation,
            successor_absent,
            FailureStage::FreshSession,
        );
        add_observation(&mut evidence, &successor_observation);
        successor_ready?;
        let retained_mapping = retained_observer
            .map_frame(
                retained_result.frame(),
                PixelFormat::Rgba8,
                &bounded(OPERATION_WAIT),
            )
            .map_err(|error| operation_failure(FailureStage::RetainedOwnership, error.status()))?;
        let retained_shape = token_shape(retained_result.frame(), &fixture)
            .ok_or_else(|| oracle_failure(FailureStage::RetainedOwnership))?;
        let retained_token = decode_visual_token(&retained_mapping, retained_shape)
            .map_err(|_| oracle_failure(FailureStage::RetainedOwnership))?;
        evidence.extra_mappings = evidence.extra_mappings.saturating_add(1);
        evidence.extra_decodes = evidence.extra_decodes.saturating_add(1);
        if retained_result.frame().stamp() != retained_stamp || retained_token != visible.token {
            return Err(oracle_failure(FailureStage::RetainedOwnership));
        }
        cleanup_ok &= successor.close(&bounded(OPERATION_WAIT)).is_ok();
        drop(successor);
        drop(retained_mapping);
        drop(retained_observer);
        drop(retained_result);
        drop(engine);

        let mapping_engine = native_engine()
            .map_err(|error| operation_failure(FailureStage::FreshSession, error.status()))?;
        let mapping_target = fixture
            .authenticated_target(&mapping_engine)
            .map_err(|_| authority_failure(FailureStage::FreshSession))?;
        let mapping_session = mapping_engine
            .open(
                mapping_target,
                &OpenRequest::new(),
                &bounded(OPERATION_WAIT),
            )
            .map_err(|error| operation_failure(FailureStage::FreshSession, error.status()))?;
        let mapping_absent =
            issue_fixture_visual_state(&mut fixture, &mut last_ack, VisualMarkerState::Absent)
                .map_err(|_| authority_failure(FailureStage::FreshSession))?;
        evidence.expected_token = Some(mapping_absent);
        let mut mapping_observation = SessionObservation::default();
        let mapping_frame = match synchronize_one_typed(
            &fixture,
            mapping_target,
            &mapping_session,
            &mut mapping_observation,
            mapping_absent,
            FailureStage::FreshSession,
        ) {
            Ok(frame) => frame,
            Err(failure) => {
                add_observation(&mut evidence, &mapping_observation);
                return Err(failure);
            }
        };
        let mapping_shape = marker_shape(&mapping_frame, &fixture)
            .ok_or_else(|| oracle_failure(FailureStage::TemplatePreparation))?;
        let mapping_template = prepare_marker(
            &mapping_engine,
            mapping_shape,
            "watch-marker-v2-contract-mapping",
        )
        .map_err(|_| oracle_failure(FailureStage::TemplatePreparation))?;
        let mapping_query = start_query(&mapping_session, mapping_template.clone())?;
        if let Err(failure) = wait_query_publication_typed(&mapping_query) {
            record_query_publication_total(&mapping_query, &mut evidence);
            add_observation(&mut evidence, &mapping_observation);
            return Err(failure);
        }
        let mapping_visible = match issue_fixture_visual_state_timed(
            &mut fixture,
            &mut last_ack,
            VisualMarkerState::Visible,
        ) {
            Ok(visible) => visible,
            Err(_) => {
                record_query_publication_total(&mapping_query, &mut evidence);
                add_observation(&mut evidence, &mapping_observation);
                return Err(authority_failure(FailureStage::VisualStimulus));
            }
        };
        evidence.expected_token = Some(mapping_visible.token);
        let mapping_deadline = mapping_visible.acknowledged_at + OPERATION_WAIT;
        if let Err(failure) = synchronize_one_until_typed(
            &fixture,
            mapping_target,
            &mapping_session,
            &mut mapping_observation,
            mapping_visible.token,
            FailureStage::RetainedOwnership,
            mapping_deadline,
        ) {
            record_query_publication_total(&mapping_query, &mut evidence);
            add_observation(&mut evidence, &mapping_observation);
            return Err(failure);
        }
        add_observation(&mut evidence, &mapping_observation);
        let mapping_terminal = wait_terminal_until_typed(
            &mapping_query,
            FailureStage::RetainedOwnership,
            mapping_deadline,
            &mut evidence,
        );
        record_query_publication_total(&mapping_query, &mut evidence);
        let mapping_terminal = mapping_terminal?;
        if !terminal_matches_token(
            &mapping_terminal,
            &mapping_session,
            &fixture,
            TokenMatchAuthority {
                target: mapping_target,
                template: mapping_template.id(),
                shape: mapping_shape,
                expected: mapping_visible.token,
                stage: FailureStage::RetainedOwnership,
            },
            &mut evidence,
        )? {
            return Err(oracle_failure(FailureStage::RetainedOwnership));
        }
        let mapping_result = terminal_match(&mapping_terminal)
            .ok_or_else(|| oracle_failure(FailureStage::RetainedOwnership))?;
        let mapping = mapping_session
            .map_frame(
                mapping_result.frame(),
                PixelFormat::Rgba8,
                &bounded(OPERATION_WAIT),
            )
            .map_err(|error| operation_failure(FailureStage::RetainedOwnership, error.status()))?;
        let mapping_identity = (
            mapping.stamp(),
            mapping.bytes().len(),
            mapping_bytes_checksum(mapping.bytes()),
        );
        cleanup_ok &= mapping_session.close(&bounded(OPERATION_WAIT)).is_ok();
        drop(mapping_query);
        drop(mapping_terminal);
        drop(mapping_template);
        drop(mapping_session);
        drop(mapping_engine);

        let final_engine = native_engine()
            .map_err(|error| operation_failure(FailureStage::FreshSession, error.status()))?;
        let final_target = fixture
            .authenticated_target(&final_engine)
            .map_err(|_| authority_failure(FailureStage::FreshSession))?;
        let final_session = final_engine
            .open(final_target, &OpenRequest::new(), &bounded(OPERATION_WAIT))
            .map_err(|error| operation_failure(FailureStage::FreshSession, error.status()))?;
        let final_absent =
            issue_fixture_visual_state(&mut fixture, &mut last_ack, VisualMarkerState::Absent)
                .map_err(|_| authority_failure(FailureStage::FreshSession))?;
        evidence.expected_token = Some(final_absent);
        let mut final_observation = SessionObservation::default();
        let final_ready = synchronize_one_typed(
            &fixture,
            final_target,
            &final_session,
            &mut final_observation,
            final_absent,
            FailureStage::FreshSession,
        );
        add_observation(&mut evidence, &final_observation);
        final_ready?;
        if mapping.stamp() != mapping_identity.0
            || mapping.bytes().len() != mapping_identity.1
            || mapping_bytes_checksum(mapping.bytes()) != mapping_identity.2
        {
            return Err(oracle_failure(FailureStage::RetainedOwnership));
        }
        evidence.teardown_started = Some(Instant::now());
        cleanup_ok &= final_session.close(&bounded(OPERATION_WAIT)).is_ok();
        drop(final_session);
        drop(final_engine);
        drop(mapping);
        explicit_cleanup_observed = true;
        Ok(())
    })();
    cleanup_ok &= explicit_cleanup_observed;

    let teardown_started = evidence.teardown_started.unwrap_or_else(Instant::now);
    let finalization = fixture.finish();
    let resources = finalization.resources();
    cleanup_ok &= finalization.is_accepted();
    let semantic = semantic.map_or_else(
        |failure| ScenarioFact::Fail { failure },
        |()| ScenarioFact::Pass,
    );
    let cleanup = if cleanup_ok {
        ScenarioFact::Pass
    } else {
        ScenarioFact::failed(
            FailureKind::Cleanup,
            FailureStage::NativeCleanup,
            NativeStatus::None,
        )
    };
    ScenarioAttempt::Completed(ScenarioResult {
        name: ScenarioName::RetainedOwnershipAndFreshSession,
        semantic,
        cleanup,
        timing: ScenarioTiming {
            startup_micros: Some(micros(startup)),
            watch_micros: evidence.watch_elapsed.map(micros),
            teardown_micros: Some(micros(teardown_started.elapsed())),
        },
        diagnostics: diagnostic_facts(
            &observation,
            &evidence,
            if cleanup_ok {
                LifecycleFact::Released
            } else {
                LifecycleFact::Open
            },
            resources,
        ),
    })
}

fn lifecycle_termination_contract(arguments: &Arguments) -> ScenarioAttempt {
    let run = match start_contract(arguments, ScenarioName::LifecycleTermination) {
        StartAttempt::Ready(run) => run,
        StartAttempt::ScenarioFailure(result) => return ScenarioAttempt::Completed(result),
        StartAttempt::Apparatus { outcome, failure } => {
            return ScenarioAttempt::Apparatus { outcome, failure };
        }
    };
    let startup = run.startup_elapsed;
    let mut evidence = ScenarioEvidence::default();
    let NativeRun {
        fixture: OwnedNativeFixture(mut fixture),
        engine,
        target,
        session,
        template,
        shape: _,
        mut last_ack,
        observation,
        ..
    } = run;
    let mut cleanup_ok = true;
    let mut explicit_cleanup_observed = false;
    let semantic = (|| {
        let session_query = start_query(&session, template.clone())?;
        if let Err(failure) = wait_query_publication_typed(&session_query) {
            record_query_publication_total(&session_query, &mut evidence);
            return Err(failure);
        }
        evidence.teardown_started = Some(Instant::now());
        cleanup_ok &= session.close(&bounded(OPERATION_WAIT)).is_ok();
        cleanup_ok &= session.close(&bounded(OPERATION_WAIT)).is_ok();
        let session_terminal = wait_terminal_typed(
            &session_query,
            FailureStage::SessionTermination,
            &mut evidence,
        );
        record_query_publication_total(&session_query, &mut evidence);
        let session_terminal = session_terminal?;
        if !matches!(&*session_terminal, TemplateTerminalOutcome::SessionClosed)
            || !Arc::ptr_eq(
                &session_terminal,
                &wait_terminal_typed(
                    &session_query,
                    FailureStage::SessionTermination,
                    &mut evidence,
                )?,
            )
        {
            return Err(oracle_failure(FailureStage::SessionTermination));
        }
        drop(session_query);
        drop(session_terminal);
        drop(session);

        let engine_session = engine
            .open(target, &OpenRequest::new(), &bounded(OPERATION_WAIT))
            .map_err(|error| operation_failure(FailureStage::SessionOpen, error.status()))?;
        let engine_absent =
            issue_fixture_visual_state(&mut fixture, &mut last_ack, VisualMarkerState::Absent)
                .map_err(|_| authority_failure(FailureStage::Readiness))?;
        evidence.expected_token = Some(engine_absent);
        let mut engine_observation = SessionObservation::default();
        let engine_ready = synchronize_one_typed(
            &fixture,
            target,
            &engine_session,
            &mut engine_observation,
            engine_absent,
            FailureStage::Readiness,
        );
        add_observation(&mut evidence, &engine_observation);
        engine_ready?;
        let engine_query = start_query(&engine_session, template)?;
        if let Err(failure) = wait_query_publication_typed(&engine_query) {
            record_query_publication_total(&engine_query, &mut evidence);
            return Err(failure);
        }
        drop(engine);
        let engine_terminal = wait_terminal_typed(
            &engine_query,
            FailureStage::EngineTermination,
            &mut evidence,
        );
        record_query_publication_total(&engine_query, &mut evidence);
        let engine_terminal = engine_terminal?;
        if !matches!(&*engine_terminal, TemplateTerminalOutcome::SchedulerClosed)
            || !Arc::ptr_eq(
                &engine_terminal,
                &wait_terminal_typed(
                    &engine_query,
                    FailureStage::EngineTermination,
                    &mut evidence,
                )?,
            )
        {
            return Err(oracle_failure(FailureStage::EngineTermination));
        }
        cleanup_ok &= engine_session.close(&bounded(OPERATION_WAIT)).is_ok();
        drop(engine_query);
        drop(engine_terminal);
        drop(engine_session);

        let target_engine = native_engine()
            .map_err(|error| operation_failure(FailureStage::TargetTermination, error.status()))?;
        let target = fixture
            .authenticated_target(&target_engine)
            .map_err(|_| authority_failure(FailureStage::TargetDiscovery))?;
        let target_session = target_engine
            .open(target, &OpenRequest::new(), &bounded(OPERATION_WAIT))
            .map_err(|error| operation_failure(FailureStage::SessionOpen, error.status()))?;
        let target_absent =
            issue_fixture_visual_state(&mut fixture, &mut last_ack, VisualMarkerState::Absent)
                .map_err(|_| authority_failure(FailureStage::Readiness))?;
        evidence.expected_token = Some(target_absent);
        let mut target_observation = SessionObservation::default();
        let target_frame = synchronize_one_typed(
            &fixture,
            target,
            &target_session,
            &mut target_observation,
            target_absent,
            FailureStage::Readiness,
        );
        add_observation(&mut evidence, &target_observation);
        let target_frame = target_frame?;
        let target_shape = marker_shape(&target_frame, &fixture)
            .ok_or_else(|| oracle_failure(FailureStage::TemplatePreparation))?;
        let target_template = prepare_marker(
            &target_engine,
            target_shape,
            "watch-marker-v2-contract-target-loss",
        )
        .map_err(|_| oracle_failure(FailureStage::TemplatePreparation))?;
        let target_query = start_query(&target_session, target_template)?;
        if let Err(failure) = wait_query_publication_typed(&target_query) {
            record_query_publication_total(&target_query, &mut evidence);
            return Err(failure);
        }
        #[cfg(windows)]
        {
            let target_closed = fixture
                .close_target()
                .map_err(|_| authority_failure(FailureStage::TargetTermination))
                .and_then(|acknowledgement| {
                    accept_control_acknowledgement(&mut last_ack, acknowledgement)
                        .map_err(|_| authority_failure(FailureStage::TargetTermination))
                });
            if let Err(failure) = target_closed {
                record_query_publication_total(&target_query, &mut evidence);
                return Err(failure);
            }
        }
        #[cfg(target_os = "macos")]
        {
            cleanup_ok &= fixture.finish().is_accepted();
        }
        let target_terminal = wait_terminal_typed(
            &target_query,
            FailureStage::TargetTermination,
            &mut evidence,
        );
        record_query_publication_total(&target_query, &mut evidence);
        let target_terminal = target_terminal?;
        if !matches!(&*target_terminal, TemplateTerminalOutcome::TargetLost) {
            return Err(oracle_failure(FailureStage::TargetTermination));
        }
        cleanup_ok &= target_session.close(&bounded(OPERATION_WAIT)).is_ok();
        drop(target_query);
        drop(target_terminal);
        drop(target_session);
        drop(target_engine);
        evidence.lifecycle = Some(LifecycleFact::TargetLost);
        explicit_cleanup_observed = true;
        Ok(())
    })();
    cleanup_ok &= explicit_cleanup_observed;

    let teardown_started = evidence.teardown_started.unwrap_or_else(Instant::now);
    let finalization = fixture.finish();
    let resources = finalization.resources();
    cleanup_ok &= finalization.is_accepted();
    ScenarioAttempt::Completed(ScenarioResult {
        name: ScenarioName::LifecycleTermination,
        semantic: semantic.map_or_else(
            |failure| ScenarioFact::Fail { failure },
            |()| ScenarioFact::Pass,
        ),
        cleanup: if cleanup_ok {
            ScenarioFact::Pass
        } else {
            ScenarioFact::failed(
                FailureKind::Cleanup,
                FailureStage::NativeCleanup,
                NativeStatus::None,
            )
        },
        timing: ScenarioTiming {
            startup_micros: Some(micros(startup)),
            watch_micros: None,
            teardown_micros: Some(micros(teardown_started.elapsed())),
        },
        diagnostics: diagnostic_facts(
            &observation,
            &evidence,
            if cleanup_ok {
                LifecycleFact::Released
            } else {
                LifecycleFact::Open
            },
            resources,
        ),
    })
}

fn cleanup_baseline(arguments: &Arguments) -> ScenarioAttempt {
    execute_standard(
        arguments,
        ScenarioName::CleanupBaseline,
        |_run, evidence| {
            evidence.lifecycle = Some(LifecycleFact::Open);
            Ok(())
        },
    )
}

fn execute_standard(
    arguments: &Arguments,
    name: ScenarioName,
    operation: impl FnOnce(&mut NativeRun, &mut ScenarioEvidence) -> Result<(), FailureFacts>,
) -> ScenarioAttempt {
    let mut run = match start_contract(arguments, name) {
        StartAttempt::Ready(run) => run,
        StartAttempt::ScenarioFailure(result) => return ScenarioAttempt::Completed(result),
        StartAttempt::Apparatus { outcome, failure } => {
            return ScenarioAttempt::Apparatus { outcome, failure };
        }
    };
    let startup = run.startup_elapsed;
    let mut evidence = ScenarioEvidence {
        expected_token: Some(run.readiness_token),
        observed_token: run.observation.last_token,
        frame: Some(run.readiness_stamp),
        target_scale_milli: run.readiness_scale_milli,
        ..ScenarioEvidence::default()
    };
    let semantic = operation(&mut run, &mut evidence).map_or_else(
        |failure| ScenarioFact::Fail { failure },
        |()| ScenarioFact::Pass,
    );
    let observation = run.observation;
    let teardown_started = evidence.teardown_started;
    let finalization = run.finalize();
    let teardown_elapsed =
        teardown_started.map_or(finalization.elapsed, |started| started.elapsed());
    let cleanup = evidence.cleanup_failure.map_or_else(
        || cleanup_fact(finalization.accepted, finalization.resources),
        |failure| ScenarioFact::Fail { failure },
    );
    ScenarioAttempt::Completed(ScenarioResult {
        name,
        semantic,
        cleanup,
        timing: ScenarioTiming {
            startup_micros: Some(micros(startup)),
            watch_micros: evidence.watch_elapsed.map(micros),
            teardown_micros: Some(micros(teardown_elapsed)),
        },
        diagnostics: diagnostic_facts(
            &observation,
            &evidence,
            if finalization.accepted {
                LifecycleFact::Released
            } else {
                LifecycleFact::Open
            },
            finalization.resources,
        ),
    })
}

#[expect(
    clippy::large_enum_variant,
    reason = "one-shot qualification state stays stack-owned and avoids an extra allocation"
)]
enum StartAttempt {
    Ready(NativeRun),
    ScenarioFailure(ScenarioResult),
    Apparatus {
        outcome: ExecutionOutcome,
        failure: FailureFacts,
    },
}

struct ProductFailureContext {
    fixture: NativeFixture,
    engine: Engine,
    session: Option<Session>,
    startup: Option<Duration>,
    observation: SessionObservation,
    expected_token: Option<VisualToken>,
}

fn start_contract(arguments: &Arguments, name: ScenarioName) -> StartAttempt {
    let mut fixture = match NativeFixture::start(arguments) {
        Ok(fixture) => fixture,
        Err(_) => {
            return StartAttempt::Apparatus {
                outcome: ExecutionOutcome::Infra,
                failure: authority_failure(FailureStage::FixtureLaunch),
            };
        }
    };
    let engine = match native_engine() {
        Ok(engine) => engine,
        Err(error) => {
            let _finalization = fixture.finish();
            return StartAttempt::Apparatus {
                outcome: if error.status() == Status::Unsupported {
                    ExecutionOutcome::Unsupported
                } else {
                    ExecutionOutcome::Infra
                },
                failure: failure_facts(
                    FailureKind::Authority,
                    FailureStage::EngineCreate,
                    native_status(error.status()),
                ),
            };
        }
    };
    if !permission_oracle(&engine) {
        drop(engine);
        let _finalization = fixture.finish();
        return StartAttempt::Apparatus {
            outcome: ExecutionOutcome::Unsupported,
            failure: failure_facts(
                FailureKind::Authority,
                FailureStage::PermissionAdmission,
                NativeStatus::Unsupported,
            ),
        };
    }
    let target = match fixture.authenticated_target(&engine) {
        Ok(target) => target,
        Err(_) => {
            drop(engine);
            let _finalization = fixture.finish();
            return StartAttempt::Apparatus {
                outcome: ExecutionOutcome::Infra,
                failure: authority_failure(FailureStage::TargetDiscovery),
            };
        }
    };
    let session = match engine.open(target, &OpenRequest::new(), &bounded(OPERATION_WAIT)) {
        Ok(session) => session,
        Err(error) => {
            return start_product_failure(
                name,
                ProductFailureContext {
                    fixture,
                    engine,
                    session: None,
                    startup: None,
                    observation: SessionObservation::default(),
                    expected_token: None,
                },
                operation_failure(FailureStage::SessionOpen, error.status()),
            );
        }
    };
    let startup_started = Instant::now();
    let mut last_ack = ControlAcknowledgement {
        generation: 1,
        revision: 0,
        visual_token: None,
    };
    let expected =
        match issue_fixture_visual_state(&mut fixture, &mut last_ack, VisualMarkerState::Absent) {
            Ok(expected) => expected,
            Err(_) => {
                return start_product_failure(
                    name,
                    ProductFailureContext {
                        fixture,
                        engine,
                        session: Some(session),
                        startup: Some(startup_started.elapsed()),
                        observation: SessionObservation::default(),
                        expected_token: None,
                    },
                    authority_failure(FailureStage::Readiness),
                );
            }
        };
    let mut observation = SessionObservation::default();
    let frame = {
        let mut synchronization = [SessionSynchronization::new(&session, &mut observation)];
        if let Err(error) = synchronize_sessions(
            &fixture,
            target,
            expected,
            &mut synchronization,
            Instant::now() + OPERATION_WAIT,
        ) {
            let failure = synchronization_failure(&error, FailureStage::Readiness);
            return start_product_failure(
                name,
                ProductFailureContext {
                    fixture,
                    engine,
                    session: Some(session),
                    startup: Some(startup_started.elapsed()),
                    observation,
                    expected_token: Some(expected),
                },
                failure,
            );
        }
        synchronization[0]
            .frame
            .take()
            .expect("successful synchronization owns its frame")
    };
    let startup_elapsed = startup_started.elapsed();
    let shape = match marker_shape(&frame, &fixture) {
        Some(shape) => shape,
        None => {
            return start_product_failure(
                name,
                ProductFailureContext {
                    fixture,
                    engine,
                    session: Some(session),
                    startup: Some(startup_elapsed),
                    observation,
                    expected_token: Some(expected),
                },
                oracle_failure(FailureStage::TemplatePreparation),
            );
        }
    };
    let template = match prepare_marker(&engine, shape, "watch-marker-v2-contract") {
        Ok(template) => template,
        Err(_) => {
            return start_product_failure(
                name,
                ProductFailureContext {
                    fixture,
                    engine,
                    session: Some(session),
                    startup: Some(startup_elapsed),
                    observation,
                    expected_token: Some(expected),
                },
                oracle_failure(FailureStage::TemplatePreparation),
            );
        }
    };
    StartAttempt::Ready(NativeRun {
        fixture: OwnedNativeFixture(fixture),
        engine,
        target,
        session,
        template,
        shape,
        last_ack,
        observation,
        startup_elapsed,
        readiness_token: expected,
        readiness_stamp: frame.stamp(),
        readiness_scale_milli: target_scale_milli(&frame),
    })
}

fn start_product_failure(
    name: ScenarioName,
    context: ProductFailureContext,
    failure: FailureFacts,
) -> StartAttempt {
    let ProductFailureContext {
        mut fixture,
        engine,
        session,
        startup,
        observation,
        expected_token,
    } = context;
    let teardown_started = Instant::now();
    let session_closed = session.is_none_or(|session| {
        let closed = session.close(&bounded(OPERATION_WAIT)).is_ok();
        drop(session);
        closed
    });
    drop(engine);
    let finalization = fixture.finish();
    let resources = finalization.resources();
    let cleanup_ok = session_closed && finalization.is_accepted();
    StartAttempt::ScenarioFailure(ScenarioResult {
        name,
        semantic: ScenarioFact::Fail { failure },
        cleanup: cleanup_fact(cleanup_ok, resources),
        timing: ScenarioTiming {
            startup_micros: startup.map(micros),
            watch_micros: None,
            teardown_micros: Some(micros(teardown_started.elapsed())),
        },
        diagnostics: diagnostic_facts(
            &observation,
            &ScenarioEvidence {
                expected_token,
                ..ScenarioEvidence::default()
            },
            if cleanup_ok {
                LifecycleFact::Released
            } else {
                LifecycleFact::Open
            },
            resources,
        ),
    })
}

fn match_once(
    run: &mut NativeRun,
    evidence: &mut ScenarioEvidence,
) -> Result<(TemplateQuery, Arc<TemplateTerminalOutcome>), FailureFacts> {
    let query = start_query(&run.session, run.template.clone())?;
    let before = match wait_query_publication_typed(&query) {
        Ok(before) => before,
        Err(failure) => {
            record_query_publication_total(&query, evidence);
            return Err(failure);
        }
    };
    let visible = match issue_fixture_visual_state_timed(
        &mut run.fixture,
        &mut run.last_ack,
        VisualMarkerState::Visible,
    ) {
        Ok(visible) => visible,
        Err(_) => {
            record_query_publication_total(&query, evidence);
            return Err(authority_failure(FailureStage::VisualStimulus));
        }
    };
    evidence.expected_token = Some(visible.token);
    let phase_deadline = visible.acknowledged_at + OPERATION_WAIT;
    let synchronization = synchronize_one_until_typed(
        &run.fixture,
        run.target,
        &run.session,
        &mut run.observation,
        visible.token,
        FailureStage::MatchCorrelation,
        phase_deadline,
    );
    if let Err(failure) = synchronization {
        record_query_publication_total(&query, evidence);
        return Err(failure);
    }
    let terminal = wait_terminal_until_typed(
        &query,
        FailureStage::MatchCorrelation,
        phase_deadline,
        evidence,
    );
    record_query_publication_total(&query, evidence);
    let terminal = terminal?;
    if !terminal_matches_token(
        &terminal,
        &run.session,
        &run.fixture,
        TokenMatchAuthority {
            target: run.target,
            template: run.template.id(),
            shape: run.shape,
            expected: visible.token,
            stage: FailureStage::MatchCorrelation,
        },
        evidence,
    )? || query.benchmark_publication_count() <= before
    {
        return Err(oracle_failure(FailureStage::MatchCorrelation));
    }
    evidence.observed_token = Some(visible.token);
    evidence.watch_elapsed = Some(visible.acknowledged_at.elapsed());
    Ok((query, terminal))
}

fn command_token(
    run: &mut NativeRun,
    marker: VisualMarkerState,
    evidence: &mut ScenarioEvidence,
) -> Result<Frame, FailureFacts> {
    let expected = issue_fixture_visual_state(&mut run.fixture, &mut run.last_ack, marker)
        .map_err(|_| authority_failure(FailureStage::VisualStimulus))?;
    evidence.expected_token = Some(expected);
    let frame = synchronize_one_typed(
        &run.fixture,
        run.target,
        &run.session,
        &mut run.observation,
        expected,
        FailureStage::Readiness,
    )?;
    evidence.observed_token = Some(expected);
    evidence.frame = Some(frame.stamp());
    Ok(frame)
}

fn start_query(
    session: &Session,
    template: PreparedTemplate,
) -> Result<TemplateQuery, FailureFacts> {
    session
        .start_template_watch(
            TemplateWatchRequest::new(
                template.clone(),
                MatchOptions::from_defaults(template.defaults()),
                OperationContext::new(),
            )
            .with_stability(TemplateStability::immediate())
            .with_change_policy(ChangeDetectionPolicy::default()),
        )
        .map_err(|error| operation_failure(FailureStage::WatchStart, error.status()))
}

fn wait_query_publication_typed(query: &TemplateQuery) -> Result<u64, FailureFacts> {
    let deadline = Instant::now() + OPERATION_WAIT;
    loop {
        let publications = query.benchmark_publication_count();
        match query.poll() {
            TemplateQueryOutcome::Pending(progress)
                if publications != 0 && progress.confirmed_observations() == 0 =>
            {
                return Ok(publications);
            }
            TemplateQueryOutcome::Pending(_) => {}
            TemplateQueryOutcome::Terminal(terminal) => {
                return Err(failure_facts(
                    FailureKind::Oracle,
                    FailureStage::WatchProgress,
                    terminal.status().map_or(NativeStatus::None, native_status),
                ));
            }
        }
        if Instant::now() >= deadline {
            return Err(failure_facts(
                FailureKind::Timeout,
                FailureStage::WatchProgress,
                NativeStatus::DeadlineExceeded,
            ));
        }
        thread::sleep(POLL_WAIT);
    }
}

fn wait_terminal_typed(
    query: &TemplateQuery,
    stage: FailureStage,
    evidence: &mut ScenarioEvidence,
) -> Result<Arc<TemplateTerminalOutcome>, FailureFacts> {
    wait_terminal_until_typed(query, stage, Instant::now() + OPERATION_WAIT, evidence)
}

fn wait_terminal_until_typed(
    query: &TemplateQuery,
    stage: FailureStage,
    deadline: Instant,
    evidence: &mut ScenarioEvidence,
) -> Result<Arc<TemplateTerminalOutcome>, FailureFacts> {
    loop {
        match query.poll() {
            TemplateQueryOutcome::Terminal(terminal) => return Ok(terminal),
            TemplateQueryOutcome::Pending(progress) => {
                if let Some(frame) = progress.last_frame() {
                    evidence.frame = Some(frame);
                }
            }
        }
        if Instant::now() >= deadline {
            evidence.status = Some(NativeStatus::DeadlineExceeded);
            return Err(failure_facts(
                FailureKind::Timeout,
                stage,
                NativeStatus::DeadlineExceeded,
            ));
        }
        thread::sleep(POLL_WAIT);
    }
}

fn record_query_publication_total(query: &TemplateQuery, evidence: &mut ScenarioEvidence) {
    evidence.extra_publications = evidence
        .extra_publications
        .saturating_add(query.benchmark_publication_count());
}

fn terminal_matches_token(
    terminal: &TemplateTerminalOutcome,
    session: &Session,
    fixture: &NativeFixture,
    authority: TokenMatchAuthority<'_>,
    evidence: &mut ScenarioEvidence,
) -> Result<bool, FailureFacts> {
    if !matched_target_exact(
        terminal,
        authority.target,
        authority.template,
        authority.shape,
        1,
        None,
    ) {
        evidence.status = terminal.status().map(native_status);
        return Ok(false);
    }
    let result = terminal_match(terminal).ok_or_else(|| oracle_failure(authority.stage))?;
    let token_shape =
        token_shape(result.frame(), fixture).ok_or_else(|| oracle_failure(authority.stage))?;
    let mapping = session
        .map_frame(result.frame(), PixelFormat::Rgba8, &bounded(OPERATION_WAIT))
        .map_err(|error| {
            let status = native_status(error.status());
            evidence.status = Some(status);
            failure_facts(FailureKind::Operation, authority.stage, status)
        })?;
    evidence.extra_mappings = evidence.extra_mappings.saturating_add(1);
    evidence.extra_decodes = evidence.extra_decodes.saturating_add(1);
    let decoded =
        decode_visual_token(&mapping, token_shape).map_err(|_| oracle_failure(authority.stage))?;
    evidence.frame = Some(result.frame().stamp());
    evidence.observed_token = Some(decoded);
    Ok(decoded == authority.expected)
}

fn synchronize_one_typed(
    fixture: &NativeFixture,
    target: TargetId,
    session: &Session,
    observation: &mut SessionObservation,
    expected: VisualToken,
    stage: FailureStage,
) -> Result<Frame, FailureFacts> {
    synchronize_one_until_typed(
        fixture,
        target,
        session,
        observation,
        expected,
        stage,
        Instant::now() + OPERATION_WAIT,
    )
}

fn synchronize_one_until_typed(
    fixture: &NativeFixture,
    target: TargetId,
    session: &Session,
    observation: &mut SessionObservation,
    expected: VisualToken,
    stage: FailureStage,
    deadline: Instant,
) -> Result<Frame, FailureFacts> {
    let mut synchronization = [SessionSynchronization::new(session, observation)];
    synchronize_sessions(fixture, target, expected, &mut synchronization, deadline)
        .map_err(|error| synchronization_failure(&error, stage))?;
    synchronization[0]
        .frame
        .take()
        .ok_or_else(|| oracle_failure(stage))
}
fn synchronization_failure(error: &TokenSynchronizationError, stage: FailureStage) -> FailureFacts {
    match error.failure {
        TokenSynchronizationFailure::Timeout => {
            failure_facts(FailureKind::Timeout, stage, NativeStatus::DeadlineExceeded)
        }
        TokenSynchronizationFailure::Operation { status, .. } => operation_failure(stage, status),
        TokenSynchronizationFailure::Protocol { .. } => {
            failure_facts(FailureKind::Protocol, stage, NativeStatus::None)
        }
    }
}

fn add_observation(evidence: &mut ScenarioEvidence, observation: &SessionObservation) {
    evidence.extra_acquisitions = evidence
        .extra_acquisitions
        .saturating_add(observation.acquisition_attempt_count);
    evidence.extra_publications = evidence
        .extra_publications
        .saturating_add(observation.publication_count);
    evidence.extra_mappings = evidence
        .extra_mappings
        .saturating_add(observation.mapping_attempt_count);
    evidence.extra_decodes = evidence
        .extra_decodes
        .saturating_add(observation.decode_attempt_count);
    if observation.last_frame.is_some() {
        evidence.frame = observation.last_frame;
    }
    if observation.last_token.is_some() {
        evidence.observed_token = observation.last_token;
    }
    let status = bounded_status(observation.last_status);
    if status != NativeStatus::None {
        evidence.status = Some(status);
    }
}

fn diagnostic_facts(
    observation: &SessionObservation,
    evidence: &ScenarioEvidence,
    default_lifecycle: LifecycleFact,
    resources: NativeResourceFacts,
) -> DiagnosticFacts {
    DiagnosticFacts {
        expected_token: evidence
            .expected_token
            .map(|token| u64::from(token.value())),
        observed_token: evidence
            .observed_token
            .or(observation.last_token)
            .map(|token| u64::from(token.value())),
        frame: evidence
            .frame
            .or(observation.last_frame)
            .map(frame_identity),
        prior_target_scale_milli: evidence.prior_target_scale_milli,
        target_scale_milli: evidence.target_scale_milli,
        lifecycle: evidence.lifecycle.unwrap_or(default_lifecycle),
        acquisitions: observation
            .acquisition_attempt_count
            .saturating_add(evidence.extra_acquisitions),
        publications: observation
            .publication_count
            .saturating_add(evidence.extra_publications),
        mappings: observation
            .mapping_attempt_count
            .saturating_add(evidence.extra_mappings),
        decodes: observation
            .decode_attempt_count
            .saturating_add(evidence.extra_decodes),
        status: evidence
            .status
            .unwrap_or_else(|| bounded_status(observation.last_status)),
        resources: resource_facts(resources),
    }
}

fn frame_identity(stamp: FrameStamp) -> FrameIdentityFacts {
    FrameIdentityFacts {
        stream: stamp.stream().get(),
        epoch: stamp.epoch().value(),
        sequence: stamp.sequence().value(),
        geometry: stamp.geometry().value(),
    }
}

fn resource_facts(resources: NativeResourceFacts) -> ResourceFacts {
    ResourceFacts {
        protocol_stop_acknowledged: resources.protocol_stop_acknowledged,
        authenticated_lifetime: resources.authenticated_lifetime.map(process_lifetime_fact),
        launched_lifetime: resources.launched_lifetime.map(process_lifetime_fact),
        bounded_containment: resources.bounded_containment,
        output_drained: resources.output_drained,
        executable_identity_unchanged: resources.executable_identity_unchanged,
        cleanup_debt: resources.cleanup_debt.map(|debt| match debt {
            NativeCleanupDebtFact::None => CleanupDebtFact::None,
            NativeCleanupDebtFact::Deferred => CleanupDebtFact::Deferred,
        }),
        apple_launch_accepted_live: resources.apple_launch_accepted_live,

        fixture_process_reaped: resources.fixture_process_reaped,
        fixture_reader_joined: resources.fixture_reader_joined,
        apple_cleanup_scheduled: resources.apple_cleanup_scheduled,
        apple_cleanup_active: resources.apple_cleanup_active,
        apple_cleanup_completed: resources.apple_cleanup_completed,
        apple_cleanup_exhausted: resources.apple_cleanup_exhausted,
    }
}
fn mapping_bytes_checksum(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn process_lifetime_fact(value: NativeProcessLifetimeFact) -> ProcessLifetimeFact {
    match value {
        NativeProcessLifetimeFact::NotObserved => ProcessLifetimeFact::NotObserved,
        NativeProcessLifetimeFact::Unknown => ProcessLifetimeFact::Unknown,
        NativeProcessLifetimeFact::Live => ProcessLifetimeFact::Live,
        NativeProcessLifetimeFact::Lost => ProcessLifetimeFact::Lost,
        NativeProcessLifetimeFact::ObservationFailed => ProcessLifetimeFact::ObservationFailed,
    }
}

fn cleanup_fact(accepted: bool, resources: NativeResourceFacts) -> ScenarioFact {
    if accepted && resources.baseline_observed {
        ScenarioFact::Pass
    } else {
        ScenarioFact::failed(
            FailureKind::Cleanup,
            if resources.baseline_observed {
                FailureStage::NativeCleanup
            } else {
                FailureStage::ResourceBaseline
            },
            NativeStatus::None,
        )
    }
}

fn apparatus(outcome: ExecutionOutcome, stage: FailureStage) -> ScenarioAttempt {
    ScenarioAttempt::Apparatus {
        outcome,
        failure: authority_failure(stage),
    }
}

const fn authority_failure(stage: FailureStage) -> FailureFacts {
    failure_facts(FailureKind::Authority, stage, NativeStatus::None)
}

const fn oracle_failure(stage: FailureStage) -> FailureFacts {
    failure_facts(FailureKind::Oracle, stage, NativeStatus::None)
}

const fn operation_failure(stage: FailureStage, status: Status) -> FailureFacts {
    failure_facts(FailureKind::Operation, stage, native_status(status))
}

const fn failure_facts(
    kind: FailureKind,
    stage: FailureStage,
    status: NativeStatus,
) -> FailureFacts {
    FailureFacts {
        kind,
        stage,
        status,
    }
}

const fn bounded_status(status: BoundedNativeStatus) -> NativeStatus {
    match status {
        BoundedNativeStatus::NotAttempted | BoundedNativeStatus::Published => NativeStatus::None,
        BoundedNativeStatus::DeadlineExceeded => NativeStatus::DeadlineExceeded,
        BoundedNativeStatus::Failed(status) => native_status(status),
    }
}

const fn native_status(status: Status) -> NativeStatus {
    match status {
        Status::InvalidArgument => NativeStatus::InvalidArgument,
        Status::Unsupported => NativeStatus::Unsupported,
        Status::Cancelled => NativeStatus::Cancelled,
        Status::DeadlineExceeded => NativeStatus::DeadlineExceeded,
        Status::Closed => NativeStatus::Closed,
        Status::TargetLost => NativeStatus::TargetLost,
        Status::LimitExceeded => NativeStatus::LimitExceeded,
        Status::CaptureFailed => NativeStatus::CaptureFailed,
        Status::AssetInvalid => NativeStatus::AssetInvalid,
        Status::VisionFailed => NativeStatus::VisionFailed,
        Status::InputFailed => NativeStatus::InputFailed,
        Status::Internal => NativeStatus::Internal,
        _ => NativeStatus::Other,
    }
}

fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}
