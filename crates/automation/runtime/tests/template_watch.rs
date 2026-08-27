//! Public contract tests for bounded template-presence queries.

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use mado_pilot_runtime::{
    CancellationToken, CaptureFault, ClipPolicy, Clock, Continuity, DiagnosticDrain,
    DiagnosticPayload, MatchOptions, MonotonicInstant, OpenRequest, OperationContext, PixelRect,
    RegionSelection, Status, TemplateAnalysisRate, TemplateOverload, TemplateQuery,
    TemplateQueryOutcome, TemplateQueryProgress, TemplateStability, TemplateTerminalOutcome,
    TemplateWatchDiagnosticOutcome, TemplateWatchRequest, TemplateWorkDisposition,
};
use mado_pilot_testkit::{
    CompletionGate, ControlledMatcher, ManualClock, ScriptedMatchCall, match_fixtures,
};
use mado_pilot_vision::Candidate;

use support::Harness;

fn open_unpublished(harness: &Harness) -> mado_pilot_runtime::Session {
    let operation = OperationContext::new();
    let target = harness.engine.discover(&operation).expect("discovered")[0].id();
    harness
        .engine
        .open(target, &OpenRequest::new(), &operation)
        .expect("opened")
}

fn opened(harness: &Harness) -> mado_pilot_runtime::Session {
    let operation = OperationContext::new();
    let target = harness.engine.discover(&operation).expect("discovered")[0].id();
    let session = harness
        .engine
        .open(target, &OpenRequest::new(), &operation)
        .expect("opened");
    harness
        .capture
        .publish(0x31, Continuity::Continuous)
        .expect("published current frame");
    session
}

fn request(harness: &Harness) -> (mado_pilot_runtime::PreparedTemplate, MatchOptions) {
    let operation = OperationContext::new();
    let template = harness
        .engine
        .prepare_template(&match_fixtures::planted_template("watch"), &operation)
        .expect("prepared");
    let options = MatchOptions::from_defaults(template.defaults());
    (template, options)
}

fn wait_progress(
    query: &TemplateQuery,
    predicate: impl Fn(TemplateQueryProgress) -> bool,
) -> TemplateQueryProgress {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match query.poll() {
            TemplateQueryOutcome::Pending(progress) if predicate(progress) => return progress,
            TemplateQueryOutcome::Pending(_) if Instant::now() < deadline => {
                std::thread::yield_now();
            }
            TemplateQueryOutcome::Pending(progress) => {
                panic!("query progress condition timed out: {progress:?}")
            }
            TemplateQueryOutcome::Terminal(outcome) => {
                panic!("query became terminal before expected progress: {outcome:?}")
            }
        }
    }
}

fn wait_context() -> OperationContext {
    OperationContext::new()
        .with_timeout(Duration::from_secs(2))
        .expect("representable test timeout")
}

fn assert_send_sync<T: Send + Sync>() {}

#[derive(Debug, Default)]
struct PublicationExpiryClock {
    calls: AtomicUsize,
}

impl Clock for PublicationExpiryClock {
    fn now(&self) -> MonotonicInstant {
        if self.calls.fetch_add(1, Ordering::AcqRel) < 3 {
            MonotonicInstant::ORIGIN
        } else {
            MonotonicInstant::from_origin(Duration::from_secs(2))
        }
    }
}

#[test]
fn query_handle_and_immutable_outcomes_are_send_sync() {
    assert_send_sync::<TemplateQuery>();
    assert_send_sync::<TemplateQueryOutcome>();
    assert_send_sync::<TemplateTerminalOutcome>();
}

#[test]
fn invalid_rate_and_stability_are_rejected_before_a_query_exists() {
    assert_eq!(
        TemplateAnalysisRate::at_most_every(Duration::ZERO)
            .expect_err("zero has an explicit unrestricted representation")
            .status(),
        Status::InvalidArgument
    );
    assert_eq!(
        TemplateStability::consecutive(0)
            .expect_err("zero confirmations cannot establish stability")
            .status(),
        Status::InvalidArgument
    );
    assert_eq!(
        TemplateStability::duration(Duration::ZERO)
            .expect_err("zero duration is immediate, not duration stability")
            .status(),
        Status::InvalidArgument
    );
}

#[test]
fn current_frame_match_returns_exact_source_correlated_result() {
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8)
            .with_candidates(vec![Candidate::new(2, 3, 0.99)]),
    );
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let query = session
        .start_template_watch(TemplateWatchRequest::new(
            template.clone(),
            options,
            OperationContext::new(),
        ))
        .expect("started query");

    let outcome = query.wait(&wait_context()).expect("query completed");
    let TemplateTerminalOutcome::Matched(result) = outcome.as_ref() else {
        panic!("expected matched terminal outcome, got {outcome:?}");
    };

    assert_eq!(result.target(), session.target());
    assert_eq!(result.template(), template.id());
    assert_eq!(result.frame().stamp(), result.result().stamp());
    assert_eq!(result.frame().stamp().stream(), session.stream());
    assert_eq!(result.confirmed_observations(), 1);
    assert_eq!(harness.matcher.find_count(), 1);
}

#[test]
fn caller_wait_interruption_does_not_cancel_the_query() {
    let harness = Harness::silent();
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let query = session
        .start_template_watch(TemplateWatchRequest::new(
            template,
            options,
            OperationContext::new(),
        ))
        .expect("started query");
    let expired_wait = OperationContext::new()
        .with_timeout(Duration::ZERO)
        .expect("zero timeout is representable");
    assert_eq!(
        query
            .wait(&expired_wait)
            .expect_err("only this wait times out")
            .status(),
        Status::DeadlineExceeded
    );
    assert!(matches!(query.poll(), TemplateQueryOutcome::Pending(_)));

    let wait_cancel = CancellationToken::new();
    wait_cancel.cancel();
    let wait = OperationContext::new().with_cancellation(wait_cancel);

    assert_eq!(
        query
            .wait(&wait)
            .expect_err("only this wait is cancelled")
            .status(),
        Status::Cancelled
    );
    assert!(matches!(query.poll(), TemplateQueryOutcome::Pending(_)));

    let first = query.cancel();
    let repeated = query.cancel();
    assert!(Arc::ptr_eq(&first, &repeated));
    assert!(matches!(first.as_ref(), TemplateTerminalOutcome::Cancelled));
}

#[test]
fn completed_result_survives_session_close() {
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8)
            .with_candidates(vec![Candidate::new(1, 1, 0.98)]),
    );
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let query = session
        .start_template_watch(TemplateWatchRequest::new(
            template,
            options,
            OperationContext::new(),
        ))
        .expect("started query");
    let retained = query.wait(&wait_context()).expect("query completed");
    let stamp = match retained.as_ref() {
        TemplateTerminalOutcome::Matched(result) => result.frame().stamp(),
        other => panic!("expected match, got {other:?}"),
    };

    session
        .close(&wait_context())
        .expect("session closed after completion");

    let TemplateTerminalOutcome::Matched(result) = retained.as_ref() else {
        unreachable!("retained result was matched before close")
    };
    assert_eq!(result.frame().stamp(), stamp);
    assert_eq!(result.result().stamp(), stamp);
}

#[test]
fn watcher_diagnostics_are_bounded_source_correlated_and_content_redacted() {
    let harness = Harness::with_diagnostics(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8)
            .with_candidates(vec![Candidate::new(1, 1, 0.98)]),
        2,
    );
    let reader = harness
        .engine
        .take_diagnostic_reader()
        .expect("debug diagnostics enabled");
    let session = opened(&harness);
    let operation = OperationContext::new();
    let private_template = harness
        .engine
        .prepare_template(
            &match_fixtures::planted_template("private/account-template.png"),
            &operation,
        )
        .expect("prepared");
    let query = session
        .start_template_watch(TemplateWatchRequest::new(
            private_template.clone(),
            MatchOptions::from_defaults(private_template.defaults()),
            operation,
        ))
        .expect("started query");
    query.wait(&wait_context()).expect("query completed");

    let DiagnosticDrain::Batch(batch) = reader.drain() else {
        panic!("expected watcher diagnostic batch");
    };
    assert!(
        batch.losses().debug() > 0,
        "finite queue accounts for displaced debug records"
    );
    let terminal = batch
        .records()
        .iter()
        .find_map(|record| match record.payload() {
            DiagnosticPayload::TemplateWatch(value)
                if value.outcome == Some(TemplateWatchDiagnosticOutcome::Matched) =>
            {
                Some(value)
            }
            _ => None,
        })
        .expect("terminal watcher diagnostic retained");
    assert_eq!(terminal.query, query.id());
    assert_eq!(terminal.target, session.target());
    assert_eq!(
        terminal.frame,
        Some(match query.poll() {
            TemplateQueryOutcome::Terminal(outcome) => match outcome.as_ref() {
                TemplateTerminalOutcome::Matched(result) => result.frame().stamp(),
                other => panic!("expected match, got {other:?}"),
            },
            TemplateQueryOutcome::Pending(_) => panic!("wait returned before terminal state"),
        })
    );
    assert_eq!(terminal.pending_count, 0);
    assert_eq!(terminal.in_flight_count, 0);
    assert_eq!(terminal.session_query_count, 0);
    assert_eq!(terminal.engine_query_count, 0);
    assert_eq!(terminal.work.get(TemplateWorkDisposition::Completed), 1);
    assert!(terminal.region.is_some());
    assert!(!format!("{terminal:?}").contains("private/account-template.png"));
}

#[test]
fn explicit_cancel_wins_before_a_gated_backend_completion() {
    let gate = Arc::new(CompletionGate::new());
    let harness = Harness::with_diagnostics(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8)
            .with_candidates(vec![Candidate::new(1, 1, 0.99)])
            .with_completion_gate(Arc::clone(&gate)),
        8,
    );
    let reader = harness
        .engine
        .take_diagnostic_reader()
        .expect("debug diagnostics enabled");
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let query = session
        .start_template_watch(TemplateWatchRequest::new(
            template,
            options,
            OperationContext::new(),
        ))
        .expect("started query");
    assert!(
        gate.wait_until_entered(Duration::from_secs(2)),
        "backend reached deterministic gate"
    );

    let cancelled = query.cancel();
    gate.release();
    assert!(
        gate.wait_until_completed(Duration::from_secs(2)),
        "late backend call completed"
    );

    assert!(matches!(
        cancelled.as_ref(),
        TemplateTerminalOutcome::Cancelled
    ));
    let TemplateQueryOutcome::Terminal(observed) = query.poll() else {
        panic!("cancel committed a terminal outcome")
    };
    assert!(Arc::ptr_eq(&cancelled, &observed));
    assert_eq!(harness.matcher.completion_count(), 1);
    let DiagnosticDrain::Batch(batch) = reader.drain() else {
        panic!("expected cancellation diagnostic batch");
    };
    let terminal = batch
        .records()
        .iter()
        .find_map(|record| match record.payload() {
            DiagnosticPayload::TemplateWatch(value)
                if value.outcome == Some(TemplateWatchDiagnosticOutcome::Cancelled)
                    && value.disposition.is_none() =>
            {
                Some(value)
            }
            _ => None,
        })
        .expect("terminal cancellation diagnostic retained");
    assert!(
        terminal.frame.is_some(),
        "terminal cancellation retains its last source frame identity"
    );
}

#[test]
fn success_wins_before_repeated_explicit_cancel() {
    let gate = Arc::new(CompletionGate::new());
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8)
            .with_candidates(vec![Candidate::new(1, 1, 0.99)])
            .with_completion_gate(Arc::clone(&gate)),
    );
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let query = session
        .start_template_watch(TemplateWatchRequest::new(
            template,
            options,
            OperationContext::new(),
        ))
        .expect("started query");
    assert!(
        gate.wait_until_entered(Duration::from_secs(2)),
        "backend reached deterministic gate"
    );
    gate.release();
    let matched = query.wait(&wait_context()).expect("success committed");

    let first_cancel = query.cancel();
    let repeated_cancel = query.cancel();
    assert!(matches!(
        matched.as_ref(),
        TemplateTerminalOutcome::Matched(_)
    ));
    assert!(Arc::ptr_eq(&matched, &first_cancel));
    assert!(Arc::ptr_eq(&matched, &repeated_cancel));
}

#[test]
fn exact_unchanged_skip_is_observable_and_does_not_enter_the_backend() {
    let first = Arc::new(CompletionGate::new());
    let changed = Arc::new(CompletionGate::new());
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8).with_calls([
            ScriptedMatchCall::new(Vec::new()).with_completion_gate(Arc::clone(&first)),
            ScriptedMatchCall::new(Vec::new()).with_completion_gate(Arc::clone(&changed)),
        ]),
    );
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let query = session
        .start_template_watch(TemplateWatchRequest::new(
            template,
            options,
            OperationContext::new(),
        ))
        .expect("started query");
    assert!(first.wait_until_entered(Duration::from_secs(2)));
    first.release();
    wait_progress(&query, |progress| {
        progress.work().get(TemplateWorkDisposition::Completed) == 1
    });

    harness
        .capture
        .publish(0x31, Continuity::Continuous)
        .expect("published identical newer frame");
    let skipped = wait_progress(&query, |progress| {
        progress.work().get(TemplateWorkDisposition::SkippedChange) == 1
    });
    assert_eq!(
        skipped
            .last_frame()
            .expect("considered frame")
            .sequence()
            .value(),
        1
    );
    assert_eq!(harness.matcher.find_count(), 1);

    harness
        .capture
        .publish(0x32, Continuity::Continuous)
        .expect("published changed frame");
    assert!(changed.wait_until_entered(Duration::from_secs(2)));
    assert_eq!(harness.matcher.find_count(), 2);
    changed.release();
    let _ = query.cancel();
}

#[test]
fn unchanged_pixels_still_require_backend_confirmation_after_stability_starts() {
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8)
            .with_candidates(vec![Candidate::new(1, 1, 0.99)]),
    );
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let query = session
        .start_template_watch(
            TemplateWatchRequest::new(template, options, OperationContext::new())
                .with_stability(TemplateStability::consecutive(2).expect("valid stability")),
        )
        .expect("started query");
    wait_progress(&query, |progress| progress.confirmed_observations() == 1);

    harness
        .capture
        .publish(0x31, Continuity::Continuous)
        .expect("published identical newer frame");
    let outcome = query.wait(&wait_context()).expect("stability completed");

    let TemplateTerminalOutcome::Matched(result) = outcome.as_ref() else {
        panic!("expected stable match, got {outcome:?}");
    };
    assert_eq!(result.confirmed_observations(), 2);
    assert_eq!(harness.matcher.find_count(), 2);
}

#[test]
fn confirmed_non_match_resets_consecutive_stability() {
    let found = vec![Candidate::new(1, 1, 0.99)];
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8).with_calls([
            ScriptedMatchCall::new(found.clone()),
            ScriptedMatchCall::new(Vec::new()),
            ScriptedMatchCall::new(found.clone()),
            ScriptedMatchCall::new(found),
        ]),
    );
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let query = session
        .start_template_watch(
            TemplateWatchRequest::new(template, options, OperationContext::new())
                .with_stability(TemplateStability::consecutive(2).expect("valid stability")),
        )
        .expect("started query");
    wait_progress(&query, |progress| progress.confirmed_observations() == 1);

    harness
        .capture
        .publish(0x32, Continuity::Continuous)
        .expect("published disappearance");
    wait_progress(&query, |progress| {
        progress.generation() == 2 && progress.confirmed_observations() == 0
    });
    harness
        .capture
        .publish(0x33, Continuity::Continuous)
        .expect("published first reappearance");
    wait_progress(&query, |progress| {
        progress.generation() == 3 && progress.confirmed_observations() == 1
    });
    harness
        .capture
        .publish(0x34, Continuity::Continuous)
        .expect("published second reappearance");

    let outcome = query.wait(&wait_context()).expect("stability completed");
    let TemplateTerminalOutcome::Matched(result) = outcome.as_ref() else {
        panic!("expected stable match, got {outcome:?}");
    };
    assert_eq!(result.frame().stamp().sequence().value(), 3);
    assert_eq!(result.confirmed_observations(), 2);
}

#[test]
fn rate_deferral_uses_query_clock_and_latest_pending_frame() {
    let clock = Arc::new(ManualClock::new());
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8).with_calls([
            ScriptedMatchCall::new(Vec::new()),
            ScriptedMatchCall::new(vec![Candidate::new(1, 1, 0.99)]),
        ]),
    );
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let operation = OperationContext::new().with_clock(clock.clone());
    let query = session
        .start_template_watch(
            TemplateWatchRequest::new(template, options, operation).with_rate(
                TemplateAnalysisRate::at_most_every(Duration::from_secs(1)).expect("valid rate"),
            ),
        )
        .expect("started query");
    wait_progress(&query, |progress| {
        progress.work().get(TemplateWorkDisposition::Completed) == 1
    });

    harness
        .capture
        .publish(0x32, Continuity::Continuous)
        .expect("published rate-deferred frame");
    wait_progress(&query, |progress| {
        progress.work().get(TemplateWorkDisposition::DeferredRate) == 1
    });
    harness
        .capture
        .publish(0x33, Continuity::Continuous)
        .expect("published newest rate-deferred frame");
    wait_progress(&query, |progress| {
        progress.work().get(TemplateWorkDisposition::DeferredRate) == 2
    });
    clock.advance(Duration::from_secs(1));

    let outcome = query.wait(&wait_context()).expect("newest frame matched");
    let TemplateTerminalOutcome::Matched(result) = outcome.as_ref() else {
        panic!("expected match, got {outcome:?}");
    };
    assert_eq!(result.frame().stamp().sequence().value(), 2);
}

#[test]
fn rate_deferral_does_not_consume_eligible_queue_residence() {
    let clock = Arc::new(ManualClock::new());
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8).with_calls([
            ScriptedMatchCall::new(Vec::new()),
            ScriptedMatchCall::new(vec![Candidate::new(1, 1, 0.99)]),
        ]),
    );
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let operation = OperationContext::new().with_clock(clock.clone());
    let query = session
        .start_template_watch(
            TemplateWatchRequest::new(template, options, operation).with_rate(
                TemplateAnalysisRate::at_most_every(Duration::from_secs(60)).expect("valid rate"),
            ),
        )
        .expect("started query");
    wait_progress(&query, |progress| {
        progress.work().get(TemplateWorkDisposition::Completed) == 1
    });

    harness
        .capture
        .publish(0x32, Continuity::Continuous)
        .expect("published rate-deferred frame");
    wait_progress(&query, |progress| {
        progress.work().get(TemplateWorkDisposition::DeferredRate) == 1
    });
    clock.advance(Duration::from_secs(59));
    std::thread::sleep(Duration::from_millis(30));
    let TemplateQueryOutcome::Pending(progress) = query.poll() else {
        panic!("rate-ineligible residence cannot expire")
    };
    assert_eq!(
        progress.work().get(TemplateWorkDisposition::QueueExpired),
        0
    );

    clock.advance(Duration::from_secs(1));
    let outcome = query
        .wait(&wait_context())
        .expect("newly eligible frame admitted");
    assert!(matches!(
        outcome.as_ref(),
        TemplateTerminalOutcome::Matched(_)
    ));
}

#[test]
fn source_end_allows_an_already_pending_last_frame_to_complete() {
    let clock = Arc::new(ManualClock::new());
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8).with_calls([
            ScriptedMatchCall::new(Vec::new()),
            ScriptedMatchCall::new(vec![Candidate::new(1, 1, 0.99)]),
        ]),
    );
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let operation = OperationContext::new().with_clock(clock.clone());
    let query = session
        .start_template_watch(
            TemplateWatchRequest::new(template, options, operation).with_rate(
                TemplateAnalysisRate::at_most_every(Duration::from_secs(60)).expect("valid rate"),
            ),
        )
        .expect("started query");
    wait_progress(&query, |progress| {
        progress.work().get(TemplateWorkDisposition::Completed) == 1
    });

    harness
        .capture
        .publish(0x32, Continuity::Continuous)
        .expect("published final source frame");
    wait_progress(&query, |progress| {
        progress.work().get(TemplateWorkDisposition::DeferredRate) == 1
    });
    harness.capture.terminate(CaptureFault::StreamEnded);
    clock.advance(Duration::from_secs(60));

    let outcome = query
        .wait(&wait_context())
        .expect("already pending final frame completed");
    let TemplateTerminalOutcome::Matched(result) = outcome.as_ref() else {
        panic!("expected pending final-frame match, got {outcome:?}");
    };
    assert_eq!(result.frame().stamp().sequence().value(), 1);
}

#[test]
fn exact_queries_coalesce_backend_work_without_sharing_terminal_state() {
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8)
            .with_candidates(vec![Candidate::new(1, 1, 0.99)]),
    );
    let session = open_unpublished(&harness);
    let (template, options) = request(&harness);
    let first = session
        .start_template_watch(TemplateWatchRequest::new(
            template.clone(),
            options,
            OperationContext::new(),
        ))
        .expect("started first query");
    let second = session
        .start_template_watch(TemplateWatchRequest::new(
            template,
            options,
            OperationContext::new(),
        ))
        .expect("started second query");
    harness
        .capture
        .publish(0x40, Continuity::Continuous)
        .expect("published shared frame");

    assert!(matches!(
        first
            .wait(&wait_context())
            .expect("first completed")
            .as_ref(),
        TemplateTerminalOutcome::Matched(_)
    ));
    assert!(matches!(
        second
            .wait(&wait_context())
            .expect("second completed")
            .as_ref(),
        TemplateTerminalOutcome::Matched(_)
    ));
    assert_eq!(harness.matcher.find_count(), 1);
    assert_ne!(first.id(), second.id());
}

#[test]
fn differing_match_options_do_not_coalesce() {
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8)
            .with_candidates(vec![Candidate::new(1, 1, 0.99)]),
    );
    let session = open_unpublished(&harness);
    let (template, options) = request(&harness);
    let result_limit = options.with_max_results(1).expect("valid result limit");
    let score_threshold = options.with_min_score(0.5).expect("valid score threshold");
    assert_ne!(options, result_limit);
    assert_ne!(options, score_threshold);
    let first = session
        .start_template_watch(TemplateWatchRequest::new(
            template.clone(),
            options,
            OperationContext::new(),
        ))
        .expect("started first query");
    let second = session
        .start_template_watch(TemplateWatchRequest::new(
            template.clone(),
            result_limit,
            OperationContext::new(),
        ))
        .expect("started second query");
    let third = session
        .start_template_watch(TemplateWatchRequest::new(
            template,
            score_threshold,
            OperationContext::new(),
        ))
        .expect("started third query");
    harness
        .capture
        .publish(0x40, Continuity::Continuous)
        .expect("published frame");

    first.wait(&wait_context()).expect("first completed");
    second.wait(&wait_context()).expect("second completed");
    third.wait(&wait_context()).expect("third completed");
    assert_eq!(harness.matcher.find_count(), 3);
}

#[test]
fn separately_prepared_instances_with_one_public_id_do_not_coalesce() {
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8)
            .with_candidates(vec![Candidate::new(1, 1, 0.99)]),
    );
    let session = open_unpublished(&harness);
    let (first_template, first_options) = request(&harness);
    let (second_template, second_options) = request(&harness);
    assert_eq!(first_template.id(), second_template.id());
    let first = session
        .start_template_watch(TemplateWatchRequest::new(
            first_template,
            first_options,
            OperationContext::new(),
        ))
        .expect("started first query");
    let second = session
        .start_template_watch(TemplateWatchRequest::new(
            second_template,
            second_options,
            OperationContext::new(),
        ))
        .expect("started second query");
    harness
        .capture
        .publish(0x40, Continuity::Continuous)
        .expect("published frame");

    first.wait(&wait_context()).expect("first completed");
    second.wait(&wait_context()).expect("second completed");
    assert_eq!(harness.matcher.find_count(), 2);
}

#[test]
fn equal_effective_regions_coalesce_but_unequal_regions_do_not() {
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8)
            .with_candidates(vec![Candidate::new(1, 1, 0.99)]),
    );
    let session = open_unpublished(&harness);
    let (template, options) = request(&harness);
    let full_pixels = RegionSelection::pixels(
        PixelRect::new(0, 0, 32, 24).expect("valid full-frame ROI"),
        ClipPolicy::Reject,
    )
    .expect("representable full-frame ROI");
    let half_pixels = RegionSelection::pixels(
        PixelRect::new(0, 0, 16, 24).expect("valid half-frame ROI"),
        ClipPolicy::Reject,
    )
    .expect("representable half-frame ROI");
    let full = session
        .start_template_watch(TemplateWatchRequest::new(
            template.clone(),
            options,
            OperationContext::new(),
        ))
        .expect("started full-frame query");
    let equivalent = session
        .start_template_watch(
            TemplateWatchRequest::new(template.clone(), options, OperationContext::new())
                .with_region(full_pixels),
        )
        .expect("started equivalent-region query");
    let unequal = session
        .start_template_watch(
            TemplateWatchRequest::new(template, options, OperationContext::new())
                .with_region(half_pixels),
        )
        .expect("started unequal-region query");
    harness
        .capture
        .publish(0x40, Continuity::Continuous)
        .expect("published frame");

    full.wait(&wait_context()).expect("full query completed");
    equivalent
        .wait(&wait_context())
        .expect("equivalent query completed");
    unequal
        .wait(&wait_context())
        .expect("unequal query completed");
    assert_eq!(harness.matcher.find_count(), 2);
}

#[test]
fn coalesced_analysis_keeps_stability_and_cancellation_query_local() {
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8)
            .with_candidates(vec![Candidate::new(1, 1, 0.99)]),
    );
    let session = open_unpublished(&harness);
    let (template, options) = request(&harness);
    let stable = session
        .start_template_watch(
            TemplateWatchRequest::new(template.clone(), options, OperationContext::new())
                .with_stability(TemplateStability::consecutive(2).expect("valid stability")),
        )
        .expect("started stable query");
    let immediate = session
        .start_template_watch(TemplateWatchRequest::new(
            template,
            options,
            OperationContext::new(),
        ))
        .expect("started immediate query");
    harness
        .capture
        .publish(0x40, Continuity::Continuous)
        .expect("published shared frame");

    assert!(matches!(
        immediate
            .wait(&wait_context())
            .expect("immediate query completed")
            .as_ref(),
        TemplateTerminalOutcome::Matched(_)
    ));
    wait_progress(&stable, |progress| progress.confirmed_observations() == 1);
    assert_eq!(harness.matcher.find_count(), 1);

    let cancelled = stable.cancel();
    assert!(matches!(
        cancelled.as_ref(),
        TemplateTerminalOutcome::Cancelled
    ));
    assert!(matches!(
        immediate.poll(),
        TemplateQueryOutcome::Terminal(outcome)
            if matches!(outcome.as_ref(), TemplateTerminalOutcome::Matched(_))
    ));
}

#[test]
fn two_sessions_reach_backend_admission_with_bounded_fair_progress() {
    let first_gate = Arc::new(CompletionGate::new());
    let second_gate = Arc::new(CompletionGate::new());
    let found = vec![Candidate::new(1, 1, 0.99)];
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8).with_calls([
            ScriptedMatchCall::new(found.clone()).with_completion_gate(Arc::clone(&first_gate)),
            ScriptedMatchCall::new(found).with_completion_gate(Arc::clone(&second_gate)),
        ]),
    );
    let first_session = open_unpublished(&harness);
    let second_session = open_unpublished(&harness);
    let (template, options) = request(&harness);
    let first = first_session
        .start_template_watch(TemplateWatchRequest::new(
            template.clone(),
            options,
            OperationContext::new(),
        ))
        .expect("started first session query");
    let second = second_session
        .start_template_watch(TemplateWatchRequest::new(
            template,
            options,
            OperationContext::new(),
        ))
        .expect("started second session query");
    harness
        .capture
        .publish(0x40, Continuity::Continuous)
        .expect("published to both sessions");

    assert!(first_gate.wait_until_entered(Duration::from_secs(2)));
    assert!(second_gate.wait_until_entered(Duration::from_secs(2)));
    first_gate.release();
    second_gate.release();
    first.wait(&wait_context()).expect("first completed");
    second.wait(&wait_context()).expect("second completed");
    assert_eq!(harness.matcher.find_count(), 2);
}

#[test]
fn four_queries_across_two_sessions_advance_in_bounded_rounds() {
    let gates: Vec<_> = (0..4).map(|_| Arc::new(CompletionGate::new())).collect();
    let calls = gates.iter().enumerate().map(|(index, gate)| {
        ScriptedMatchCall::new(vec![Candidate::new(
            i32::try_from(index + 1).expect("small candidate coordinate"),
            1,
            0.99,
        )])
        .with_completion_gate(Arc::clone(gate))
    });
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8).with_calls(calls),
    );
    let first_session = open_unpublished(&harness);
    let second_session = open_unpublished(&harness);
    let prepared: Vec<_> = (0..4).map(|_| request(&harness)).collect();
    let first_queries: Vec<_> = prepared[..2]
        .iter()
        .map(|(template, options)| {
            first_session
                .start_template_watch(TemplateWatchRequest::new(
                    template.clone(),
                    *options,
                    OperationContext::new(),
                ))
                .expect("started first-session query")
        })
        .collect();
    let second_queries: Vec<_> = prepared[2..]
        .iter()
        .map(|(template, options)| {
            second_session
                .start_template_watch(TemplateWatchRequest::new(
                    template.clone(),
                    *options,
                    OperationContext::new(),
                ))
                .expect("started second-session query")
        })
        .collect();
    harness
        .capture
        .publish(0x40, Continuity::Continuous)
        .expect("published to both sessions");

    assert!(gates[0].wait_until_entered(Duration::from_secs(2)));
    assert!(gates[1].wait_until_entered(Duration::from_secs(2)));
    assert_eq!(harness.matcher.find_count(), 2);
    gates[0].release();
    assert!(gates[2].wait_until_entered(Duration::from_secs(2)));
    assert_eq!(harness.matcher.find_count(), 3);
    gates[1].release();
    assert!(gates[3].wait_until_entered(Duration::from_secs(2)));
    assert_eq!(harness.matcher.find_count(), 4);
    gates[2].release();
    gates[3].release();

    for query in first_queries.iter().chain(&second_queries) {
        assert!(matches!(
            query
                .wait(&wait_context())
                .expect("query completed")
                .as_ref(),
            TemplateTerminalOutcome::Matched(_)
        ));
    }
    assert_eq!(harness.matcher.completion_count(), 4);
}

#[test]
fn slow_backend_reconsiders_only_the_latest_pending_frame() {
    let first_gate = Arc::new(CompletionGate::new());
    let second_gate = Arc::new(CompletionGate::new());
    let final_gate = Arc::new(CompletionGate::new());
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8).with_calls([
            ScriptedMatchCall::new(Vec::new()).with_completion_gate(Arc::clone(&first_gate)),
            ScriptedMatchCall::new(Vec::new()).with_completion_gate(Arc::clone(&second_gate)),
            ScriptedMatchCall::new(vec![Candidate::new(1, 1, 0.99)])
                .with_completion_gate(Arc::clone(&final_gate)),
        ]),
    );
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let query = session
        .start_template_watch(TemplateWatchRequest::new(
            template,
            options,
            OperationContext::new(),
        ))
        .expect("started query");
    assert!(first_gate.wait_until_entered(Duration::from_secs(2)));
    harness
        .capture
        .publish(0x32, Continuity::Continuous)
        .expect("published second in-flight frame");
    assert!(second_gate.wait_until_entered(Duration::from_secs(2)));
    harness
        .capture
        .publish(0x33, Continuity::Continuous)
        .expect("published pending frame");
    let _ = wait_progress(&query, |progress| progress.pending_count() == 1);
    harness
        .capture
        .publish(0x34, Continuity::Continuous)
        .expect("published latest replacement");
    let replaced = wait_progress(&query, |progress| {
        progress.pending_count() == 1
            && progress.work().get(TemplateWorkDisposition::Superseded) == 1
    });
    first_gate.release();
    let _ = wait_progress(&query, |progress| {
        progress.work().get(TemplateWorkDisposition::Superseded) == 2
    });
    assert!(final_gate.wait_until_entered(Duration::from_secs(2)));
    second_gate.release();
    let ready = wait_progress(&query, |progress| {
        progress.pending_count() == 0
            && progress.work().get(TemplateWorkDisposition::Admitted) == 3
            && progress.work().get(TemplateWorkDisposition::Completed) == 0
            && progress.work().get(TemplateWorkDisposition::Superseded) == 3
    });
    final_gate.release();

    let outcome = query.wait(&wait_context()).expect("latest frame matched");
    let TemplateTerminalOutcome::Matched(result) = outcome.as_ref() else {
        panic!("expected match, got {outcome:?}");
    };
    assert_eq!(replaced.pending_count(), 1);
    assert_eq!(ready.work().get(TemplateWorkDisposition::Superseded), 3);
    assert_eq!(result.frame().stamp().sequence().value(), 3);
    assert_eq!(harness.matcher.find_count(), 3);
}

#[test]
fn geometry_change_resets_stability_before_confirming_again() {
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8)
            .with_candidates(vec![Candidate::new(1, 1, 0.99)]),
    );
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let query = session
        .start_template_watch(
            TemplateWatchRequest::new(template, options, OperationContext::new())
                .with_stability(TemplateStability::consecutive(2).expect("valid stability")),
        )
        .expect("started query");
    wait_progress(&query, |progress| progress.confirmed_observations() == 1);

    harness
        .capture
        .publish(0x32, Continuity::GeometryChanged)
        .expect("published geometry change");
    let reset = wait_progress(&query, |progress| {
        progress.generation() == 2 && progress.confirmed_observations() == 1
    });
    assert_eq!(
        reset
            .last_frame()
            .expect("geometry frame considered")
            .geometry()
            .value(),
        1
    );
    harness
        .capture
        .publish(0x33, Continuity::Continuous)
        .expect("published compatible confirmation");
    let outcome = query.wait(&wait_context()).expect("stability completed");
    assert!(matches!(
        outcome.as_ref(),
        TemplateTerminalOutcome::Matched(_)
    ));
}

#[test]
fn target_loss_terminates_pending_query_and_wakes_wait() {
    let harness = Harness::silent();
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let query = session
        .start_template_watch(TemplateWatchRequest::new(
            template,
            options,
            OperationContext::new(),
        ))
        .expect("started query");
    wait_progress(&query, |progress| {
        progress.work().get(TemplateWorkDisposition::Completed) == 1
    });

    harness.capture.terminate(CaptureFault::TargetLost);

    let outcome = query
        .wait(&wait_context())
        .expect("target loss is query terminal state");
    assert!(matches!(
        outcome.as_ref(),
        TemplateTerminalOutcome::TargetLost
    ));
}

#[test]
fn coordinate_qualified_roi_is_retained_in_terminal_result() {
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8)
            .with_candidates(vec![Candidate::new(1, 1, 0.99)]),
    );
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let roi = PixelRect::new(2, 3, 28, 20).expect("valid ROI");
    let region = RegionSelection::pixels(roi, ClipPolicy::Reject).expect("representable ROI");
    let query = session
        .start_template_watch(
            TemplateWatchRequest::new(template, options, OperationContext::new())
                .with_region(region),
        )
        .expect("started query");

    let outcome = query.wait(&wait_context()).expect("ROI query completed");
    let TemplateTerminalOutcome::Matched(result) = outcome.as_ref() else {
        panic!("expected match, got {outcome:?}");
    };
    assert_eq!(result.result().searched(), roi);
}

#[test]
fn eligible_query_expires_under_saturated_fixed_workers() {
    let first_gate = Arc::new(CompletionGate::new());
    let second_gate = Arc::new(CompletionGate::new());
    let found = vec![Candidate::new(1, 1, 0.99)];
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8).with_calls([
            ScriptedMatchCall::new(found.clone()).with_completion_gate(Arc::clone(&first_gate)),
            ScriptedMatchCall::new(found.clone()).with_completion_gate(Arc::clone(&second_gate)),
            ScriptedMatchCall::new(found),
        ]),
    );
    let session = open_unpublished(&harness);
    let operation = OperationContext::new();
    let first_template = harness
        .engine
        .prepare_template(&match_fixtures::planted_template("first"), &operation)
        .expect("prepared first");
    let second_template = harness
        .engine
        .prepare_template(&match_fixtures::planted_template("second"), &operation)
        .expect("prepared second");
    let first = session
        .start_template_watch(TemplateWatchRequest::new(
            first_template.clone(),
            MatchOptions::from_defaults(first_template.defaults()),
            OperationContext::new(),
        ))
        .expect("started first blocker");
    let second = session
        .start_template_watch(TemplateWatchRequest::new(
            second_template.clone(),
            MatchOptions::from_defaults(second_template.defaults()),
            OperationContext::new(),
        ))
        .expect("started second blocker");
    harness
        .capture
        .publish(0x40, Continuity::Continuous)
        .expect("published blocker frame");
    assert!(first_gate.wait_until_entered(Duration::from_secs(2)));
    assert!(second_gate.wait_until_entered(Duration::from_secs(2)));

    let clock = Arc::new(ManualClock::new());
    let expiring_template = harness
        .engine
        .prepare_template(&match_fixtures::planted_template("expiring"), &operation)
        .expect("prepared expiring query");
    let expiring = session
        .start_template_watch(TemplateWatchRequest::new(
            expiring_template.clone(),
            MatchOptions::from_defaults(expiring_template.defaults()),
            OperationContext::new().with_clock(clock.clone()),
        ))
        .expect("started expiring query on maintained current frame");
    wait_progress(&expiring, TemplateQueryProgress::is_pending);
    clock.advance(Duration::from_secs(31));

    let outcome = expiring
        .wait(&wait_context())
        .expect("overload is immutable query outcome");
    assert!(matches!(
        outcome.as_ref(),
        TemplateTerminalOutcome::Overloaded(TemplateOverload::QueueExpired)
    ));
    first_gate.release();
    second_gate.release();
    first
        .wait(&wait_context())
        .expect("first blocker completed");
    second
        .wait(&wait_context())
        .expect("second blocker completed");
}

#[test]
fn eligible_pending_frame_expires_while_query_analysis_slot_is_full() {
    let gate = Arc::new(CompletionGate::new());
    let clock = Arc::new(ManualClock::new());
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8)
            .with_candidates(vec![Candidate::new(1, 1, 0.99)])
            .with_completion_gate(Arc::clone(&gate)),
    );
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let query = session
        .start_template_watch(
            TemplateWatchRequest::new(
                template,
                options,
                OperationContext::new().with_clock(clock.clone()),
            )
            .with_stability(TemplateStability::consecutive(2).expect("valid stability")),
        )
        .expect("started stability query");
    assert!(gate.wait_until_entered(Duration::from_secs(2)));

    harness
        .capture
        .publish(0x41, Continuity::Continuous)
        .expect("published pending frame behind full query slot");
    wait_progress(&query, |progress| {
        progress.pending_count() == 1 && progress.in_flight_count() == 1
    });
    clock.advance(Duration::from_secs(31));
    std::thread::sleep(Duration::from_millis(50));
    let expired_while_blocked = query.poll();

    gate.release();
    assert!(gate.wait_until_completed(Duration::from_secs(2)));
    assert!(matches!(
        expired_while_blocked,
        TemplateQueryOutcome::Terminal(outcome)
            if matches!(
                outcome.as_ref(),
                TemplateTerminalOutcome::Overloaded(TemplateOverload::QueueExpired)
            )
    ));
}

#[test]
fn older_generation_finishing_last_cannot_replace_newer_success() {
    let older = Arc::new(CompletionGate::new());
    let newer = Arc::new(CompletionGate::new());
    let found = vec![Candidate::new(1, 1, 0.99)];
    let harness = Harness::with_diagnostics(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8).with_calls([
            ScriptedMatchCall::new(found.clone()).with_completion_gate(Arc::clone(&older)),
            ScriptedMatchCall::new(found).with_completion_gate(Arc::clone(&newer)),
        ]),
        16,
    );
    let reader = harness
        .engine
        .take_diagnostic_reader()
        .expect("debug diagnostics enabled");
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let query = session
        .start_template_watch(TemplateWatchRequest::new(
            template,
            options,
            OperationContext::new(),
        ))
        .expect("started query");
    assert!(older.wait_until_entered(Duration::from_secs(2)));
    harness
        .capture
        .publish(0x32, Continuity::Continuous)
        .expect("published newer generation");
    assert!(newer.wait_until_entered(Duration::from_secs(2)));
    newer.release();

    let outcome = query
        .wait(&wait_context())
        .expect("newer generation committed");
    let TemplateTerminalOutcome::Matched(result) = outcome.as_ref() else {
        panic!("expected newer match, got {outcome:?}");
    };
    assert_eq!(result.frame().stamp().sequence().value(), 1);

    older.release();
    assert!(older.wait_until_completed(Duration::from_secs(2)));
    let TemplateQueryOutcome::Terminal(retained) = query.poll() else {
        panic!("newer result remains terminal")
    };
    assert!(Arc::ptr_eq(&outcome, &retained));
    let DiagnosticDrain::Batch(batch) = reader.drain() else {
        panic!("expected diagnostic batch");
    };
    assert_eq!(
        batch
            .records()
            .iter()
            .filter(|record| {
                matches!(
                    record.payload(),
                    DiagnosticPayload::TemplateWatch(value)
                        if value.outcome == Some(TemplateWatchDiagnosticOutcome::Matched)
                            && value.disposition.is_none()
                )
            })
            .count(),
        1
    );
}

#[test]
fn session_close_is_idempotent_and_wakes_pending_query() {
    let harness = Harness::silent();
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let query = session
        .start_template_watch(TemplateWatchRequest::new(
            template,
            options,
            OperationContext::new(),
        ))
        .expect("started query");
    wait_progress(&query, |progress| {
        progress.work().get(TemplateWorkDisposition::Completed) == 1
    });

    session
        .close(&wait_context())
        .expect("first close completed");
    session
        .close(&wait_context())
        .expect("repeated close completed");

    let outcome = query.wait(&wait_context()).expect("close is query outcome");
    assert!(matches!(
        outcome.as_ref(),
        TemplateTerminalOutcome::SessionClosed
    ));
}

#[test]
fn engine_drop_closes_scheduler_and_preserves_terminal_authority() {
    let gate = Arc::new(CompletionGate::new());
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8)
            .with_candidates(vec![Candidate::new(1, 1, 0.99)])
            .with_completion_gate(Arc::clone(&gate)),
    );
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let query = session
        .start_template_watch(TemplateWatchRequest::new(
            template,
            options,
            OperationContext::new(),
        ))
        .expect("started query");
    assert!(gate.wait_until_entered(Duration::from_secs(2)));

    drop(harness.engine);
    let closed = query
        .wait(&wait_context())
        .expect("scheduler close is immutable query outcome");
    assert!(matches!(
        closed.as_ref(),
        TemplateTerminalOutcome::SchedulerClosed
    ));
    gate.release();
    assert!(gate.wait_until_completed(Duration::from_secs(2)));
    let TemplateQueryOutcome::Terminal(retained) = query.poll() else {
        panic!("scheduler close remains terminal")
    };
    assert!(Arc::ptr_eq(&closed, &retained));
}

#[test]
fn query_deadline_before_start_admission_publishes_no_handle() {
    let harness = Harness::silent();
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let clock = Arc::new(ManualClock::new());
    let expired = OperationContext::new()
        .with_clock(clock)
        .with_timeout(Duration::ZERO)
        .expect("zero timeout representable");

    let error = session
        .start_template_watch(TemplateWatchRequest::new(template, options, expired))
        .expect_err("expired query cannot publish a handle");

    assert_eq!(error.status(), Status::DeadlineExceeded);
    assert_eq!(harness.matcher.find_count(), 0);
}

#[test]
fn query_interruption_at_publication_returns_error_and_releases_capacity() {
    let harness = Harness::silent();
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let operation = OperationContext::new()
        .with_clock(Arc::new(PublicationExpiryClock::default()))
        .with_deadline(MonotonicInstant::from_origin(Duration::from_secs(1)));

    let error = session
        .start_template_watch(TemplateWatchRequest::new(
            template.clone(),
            options,
            operation,
        ))
        .expect_err("publication boundary refuses the newly expired query");
    assert_eq!(error.status(), Status::DeadlineExceeded);

    let mut admitted = Vec::new();
    for _ in 0..harness.engine.template_scheduler().max_session_queries() {
        admitted.push(
            session
                .start_template_watch(TemplateWatchRequest::new(
                    template.clone(),
                    options,
                    OperationContext::new(),
                ))
                .expect("refused publication released its session capacity"),
        );
    }
    for query in admitted {
        let _ = query.cancel();
    }
}

#[test]
fn deadline_during_gated_backend_discards_late_match() {
    let gate = Arc::new(CompletionGate::new());
    let clock = Arc::new(ManualClock::new());
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8)
            .with_candidates(vec![Candidate::new(1, 1, 0.99)])
            .with_completion_gate(Arc::clone(&gate)),
    );
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let query_operation = OperationContext::new()
        .with_clock(clock.clone())
        .with_timeout(Duration::from_secs(1))
        .expect("deadline representable");
    let query = session
        .start_template_watch(TemplateWatchRequest::new(
            template,
            options,
            query_operation,
        ))
        .expect("started query");
    assert!(gate.wait_until_entered(Duration::from_secs(2)));
    clock.advance(Duration::from_secs(2));
    gate.release();

    let outcome = query
        .wait(&wait_context())
        .expect("deadline is immutable query outcome");
    assert!(matches!(
        outcome.as_ref(),
        TemplateTerminalOutcome::DeadlineExceeded
    ));
    assert!(gate.wait_until_completed(Duration::from_secs(2)));
    assert_eq!(harness.matcher.completion_count(), 1);
}

#[test]
fn duration_stability_advances_only_on_confirmed_matches() {
    let clock = Arc::new(ManualClock::new());
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8)
            .with_candidates(vec![Candidate::new(1, 1, 0.99)]),
    );
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let query = session
        .start_template_watch(
            TemplateWatchRequest::new(
                template,
                options,
                OperationContext::new().with_clock(clock.clone()),
            )
            .with_stability(
                TemplateStability::duration(Duration::from_secs(1)).expect("valid stability"),
            ),
        )
        .expect("started query");
    wait_progress(&query, |progress| {
        progress.confirmed_observations() == 1 && progress.confirmed_duration() == Duration::ZERO
    });

    harness
        .capture
        .publish(0x31, Continuity::Continuous)
        .expect("published unchanged confirmation");
    wait_progress(&query, |progress| {
        progress.confirmed_observations() == 2 && progress.confirmed_duration() == Duration::ZERO
    });
    clock.advance(Duration::from_secs(1));
    harness
        .capture
        .publish(0x31, Continuity::Continuous)
        .expect("published confirmation after duration");

    let outcome = query
        .wait(&wait_context())
        .expect("duration stability completed");
    let TemplateTerminalOutcome::Matched(result) = outcome.as_ref() else {
        panic!("expected match, got {outcome:?}");
    };
    assert_eq!(result.confirmed_observations(), 3);
    assert_eq!(result.confirmed_duration(), Duration::from_secs(1));
}

#[test]
fn epoch_change_resets_stability_before_confirming_again() {
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8)
            .with_candidates(vec![Candidate::new(1, 1, 0.99)]),
    );
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let query = session
        .start_template_watch(
            TemplateWatchRequest::new(template, options, OperationContext::new())
                .with_stability(TemplateStability::consecutive(2).expect("valid stability")),
        )
        .expect("started query");
    wait_progress(&query, |progress| progress.confirmed_observations() == 1);

    harness
        .capture
        .publish(0x32, Continuity::Discontinuous)
        .expect("published discontinuity");
    let reset = wait_progress(&query, |progress| {
        progress.generation() == 2 && progress.confirmed_observations() == 1
    });
    assert_eq!(
        reset
            .last_frame()
            .expect("epoch frame considered")
            .epoch()
            .value(),
        1
    );
    harness
        .capture
        .publish(0x33, Continuity::Continuous)
        .expect("published compatible confirmation");
    let outcome = query.wait(&wait_context()).expect("stability completed");
    assert!(matches!(
        outcome.as_ref(),
        TemplateTerminalOutcome::Matched(_)
    ));
}

#[test]
fn pending_query_keeps_watcher_lifecycle_until_source_end_after_session_drop() {
    let gate = Arc::new(CompletionGate::new());
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8)
            .with_completion_gate(Arc::clone(&gate)),
    );
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let query = session
        .start_template_watch(TemplateWatchRequest::new(
            template,
            options,
            OperationContext::new(),
        ))
        .expect("started query");
    assert!(gate.wait_until_entered(Duration::from_secs(2)));

    drop(session);
    harness.capture.terminate(CaptureFault::StreamEnded);
    gate.release();

    let outcome = query
        .wait(&wait_context())
        .expect("source end remains observable after session value drops");
    assert!(matches!(
        outcome.as_ref(),
        TemplateTerminalOutcome::SessionClosed
    ));
}

#[test]
fn fixed_session_capacity_refusal_emits_rejected_disposition() {
    let harness = Harness::with_diagnostics(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8),
        256,
    );
    let reader = harness
        .engine
        .take_diagnostic_reader()
        .expect("debug diagnostics enabled");
    let session = open_unpublished(&harness);
    let (template, options) = request(&harness);
    let limit = harness.engine.template_scheduler().max_session_queries();
    let mut queries =
        Vec::with_capacity(usize::try_from(limit).expect("u32 fits supported target usize"));
    for _ in 0..limit {
        queries.push(
            session
                .start_template_watch(TemplateWatchRequest::new(
                    template.clone(),
                    options,
                    OperationContext::new(),
                ))
                .expect("query within fixed session capacity"),
        );
    }

    let error = session
        .start_template_watch(TemplateWatchRequest::new(
            template,
            options,
            OperationContext::new(),
        ))
        .expect_err("query beyond fixed capacity is refused");

    assert_eq!(error.status(), Status::LimitExceeded);
    let DiagnosticDrain::Batch(batch) = reader.drain() else {
        panic!("expected diagnostic batch");
    };
    assert!(batch.records().iter().any(|record| {
        matches!(
            record.payload(),
            DiagnosticPayload::TemplateWatch(value)
                if value.disposition == Some(TemplateWorkDisposition::Rejected)
                    && value.outcome == Some(TemplateWatchDiagnosticOutcome::Overloaded)
        )
    }));
    drop(queries);
    session.close(&wait_context()).expect("closed session");
}

#[test]
fn invalid_region_and_unrepresentable_timing_fail_before_query_publication() {
    let harness = Harness::silent();
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let empty = PixelRect::new(4, 4, 4, 8).expect("valid empty rectangle");
    let empty_region =
        RegionSelection::pixels(empty, ClipPolicy::Reject).expect("representable region");

    let region_error = session
        .start_template_watch(
            TemplateWatchRequest::new(template.clone(), options, OperationContext::new())
                .with_region(empty_region),
        )
        .expect_err("empty region is rejected at start");
    assert_eq!(region_error.status(), Status::InvalidArgument);

    let clock = Arc::new(ManualClock::new());
    clock.advance(Duration::from_nanos(1));
    let timing_error = session
        .start_template_watch(
            TemplateWatchRequest::new(template, options, OperationContext::new().with_clock(clock))
                .with_rate(
                    TemplateAnalysisRate::at_most_every(Duration::MAX)
                        .expect("nonzero interval is structurally valid"),
                ),
        )
        .expect_err("unrepresentable rate is rejected at start");
    assert_eq!(timing_error.status(), Status::InvalidArgument);
    assert_eq!(harness.matcher.find_count(), 0);
}

#[test]
fn incompatible_newer_source_discards_older_in_flight_confirmation() {
    let older = Arc::new(CompletionGate::new());
    let found = vec![Candidate::new(1, 1, 0.99)];
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8).with_calls([
            ScriptedMatchCall::new(found.clone()).with_completion_gate(Arc::clone(&older)),
            ScriptedMatchCall::new(found.clone()),
            ScriptedMatchCall::new(found),
        ]),
    );
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let query = session
        .start_template_watch(
            TemplateWatchRequest::new(template, options, OperationContext::new())
                .with_stability(TemplateStability::consecutive(2).expect("valid stability")),
        )
        .expect("started query");
    assert!(older.wait_until_entered(Duration::from_secs(2)));

    harness
        .capture
        .publish(0x42, Continuity::GeometryChanged)
        .expect("published incompatible geometry");
    wait_progress(&query, |progress| {
        progress.generation() == 2 && progress.confirmed_observations() == 1
    });
    older.release();
    assert!(older.wait_until_completed(Duration::from_secs(2)));
    assert!(matches!(query.poll(), TemplateQueryOutcome::Pending(_)));

    harness
        .capture
        .publish(0x43, Continuity::Continuous)
        .expect("published compatible confirmation");
    let outcome = query.wait(&wait_context()).expect("stability completed");
    let TemplateTerminalOutcome::Matched(result) = outcome.as_ref() else {
        panic!("expected stable match, got {outcome:?}");
    };
    assert_eq!(result.frame().stamp().sequence().value(), 2);
    assert_eq!(result.confirmed_observations(), 2);
}

#[test]
fn target_loss_authority_wins_over_in_flight_backend_success() {
    let gate = Arc::new(CompletionGate::new());
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8)
            .with_candidates(vec![Candidate::new(1, 1, 0.99)])
            .with_completion_gate(Arc::clone(&gate)),
    );
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let query = session
        .start_template_watch(TemplateWatchRequest::new(
            template,
            options,
            OperationContext::new(),
        ))
        .expect("started query");
    assert!(gate.wait_until_entered(Duration::from_secs(2)));

    harness.capture.terminate(CaptureFault::TargetLost);
    let lost = query
        .wait(&wait_context())
        .expect("target loss became terminal");
    assert!(matches!(lost.as_ref(), TemplateTerminalOutcome::TargetLost));

    gate.release();
    assert!(gate.wait_until_completed(Duration::from_secs(2)));
    let TemplateQueryOutcome::Terminal(retained) = query.poll() else {
        panic!("target loss remains terminal")
    };
    assert!(Arc::ptr_eq(&lost, &retained));
}

#[test]
fn session_close_authority_wins_over_in_flight_backend_success() {
    let gate = Arc::new(CompletionGate::new());
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8)
            .with_candidates(vec![Candidate::new(1, 1, 0.99)])
            .with_completion_gate(Arc::clone(&gate)),
    );
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let query = session
        .start_template_watch(TemplateWatchRequest::new(
            template,
            options,
            OperationContext::new(),
        ))
        .expect("started query");
    assert!(gate.wait_until_entered(Duration::from_secs(2)));

    session.close(&wait_context()).expect("session closed");
    let closed = query
        .wait(&wait_context())
        .expect("session close became terminal");
    assert!(matches!(
        closed.as_ref(),
        TemplateTerminalOutcome::SessionClosed
    ));

    gate.release();
    assert!(gate.wait_until_completed(Duration::from_secs(2)));
    let TemplateQueryOutcome::Terminal(retained) = query.poll() else {
        panic!("session close remains terminal")
    };
    assert!(Arc::ptr_eq(&closed, &retained));
}

#[test]
fn exhausted_source_refuses_later_queries_on_the_same_session() {
    let harness = Harness::silent();
    let session = opened(&harness);
    let (template, options) = request(&harness);
    let first = session
        .start_template_watch(TemplateWatchRequest::new(
            template.clone(),
            options,
            OperationContext::new(),
        ))
        .expect("started first query");
    wait_progress(&first, |progress| {
        progress.work().get(TemplateWorkDisposition::Completed) == 1
    });

    harness.capture.terminate(CaptureFault::StreamEnded);
    let first_outcome = first.wait(&wait_context()).expect("source end observed");
    assert!(matches!(
        first_outcome.as_ref(),
        TemplateTerminalOutcome::SessionClosed
    ));

    let refusal = session
        .start_template_watch(TemplateWatchRequest::new(
            template,
            options,
            OperationContext::new(),
        ))
        .expect_err("ended source cannot publish a later query");
    assert_eq!(refusal.status(), Status::Closed);
}

#[test]
fn active_session_capacity_is_reusable_after_acquisition_exits() {
    let harness = Harness::silent();
    let (template, options) = request(&harness);
    let capacity = harness.engine.template_scheduler().max_active_sessions();
    let mut active = Vec::new();
    for _ in 0..capacity {
        let session = open_unpublished(&harness);
        let query = session
            .start_template_watch(TemplateWatchRequest::new(
                template.clone(),
                options,
                OperationContext::new(),
            ))
            .expect("session admitted within the fixed capacity");
        active.push((session, query));
    }

    let replacement = open_unpublished(&harness);
    let refusal = replacement
        .start_template_watch(TemplateWatchRequest::new(
            template.clone(),
            options,
            OperationContext::new(),
        ))
        .expect_err("one more active session exceeds the fixed capacity");
    assert_eq!(refusal.status(), Status::LimitExceeded);

    let _ = active[0].1.cancel();
    let deadline = Instant::now() + Duration::from_secs(2);
    let replacement_query = loop {
        match replacement.start_template_watch(TemplateWatchRequest::new(
            template.clone(),
            options,
            OperationContext::new(),
        )) {
            Ok(query) => break query,
            Err(error) if error.status() == Status::LimitExceeded && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) => panic!("released session capacity was not reusable: {error}"),
        }
    };

    let _ = replacement_query.cancel();
    for (_, query) in active {
        let _ = query.cancel();
    }
}
