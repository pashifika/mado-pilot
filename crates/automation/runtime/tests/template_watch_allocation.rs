//! Allocation/progress smoke for repeated bounded watcher ownership.

mod support;

use std::time::{Duration, Instant};

use mado_pilot_runtime::{
    Continuity, MatchOptions, OpenRequest, OperationContext, TemplateTerminalOutcome,
    TemplateWatchRequest,
};
use mado_pilot_testkit::bench_harness::{Accounting, Plan, Sample, measure};
use mado_pilot_testkit::{ControlledMatcher, match_fixtures};
use mado_pilot_vision::Candidate;

use support::Harness;

#[global_allocator]
static ALLOCATOR: Accounting = Accounting;

struct Fixture {
    _harness: Harness,
    session: mado_pilot_runtime::Session,
    template: mado_pilot_runtime::PreparedTemplate,
    options: MatchOptions,
}

fn fixture() -> Fixture {
    let harness = Harness::new(
        ControlledMatcher::new(mado_pilot_runtime::PixelFormat::Rgba8)
            .with_candidates(vec![Candidate::new(1, 1, 0.99)]),
    );
    let operation = OperationContext::new();
    let target = harness.engine.discover(&operation).expect("discovered")[0].id();
    let session = harness
        .engine
        .open(target, &OpenRequest::new(), &operation)
        .expect("opened");
    harness
        .capture
        .publish(0x41, Continuity::Continuous)
        .expect("published maintained current frame");
    let template = harness
        .engine
        .prepare_template(
            &match_fixtures::planted_template("allocation-watch"),
            &operation,
        )
        .expect("prepared");
    let options = MatchOptions::from_defaults(template.defaults());
    Fixture {
        _harness: harness,
        session,
        template,
        options,
    }
}

fn one_query(fixture: &Fixture) -> Sample {
    let started = Instant::now();
    let query_operation = OperationContext::new()
        .with_timeout(Duration::from_secs(2))
        .expect("representable query timeout");
    let query = fixture
        .session
        .start_template_watch(TemplateWatchRequest::new(
            fixture.template.clone(),
            fixture.options,
            query_operation,
        ))
        .expect("started query");
    let wait = OperationContext::new()
        .with_timeout(Duration::from_secs(2))
        .expect("representable wait timeout");
    let correct = query
        .wait(&wait)
        .is_ok_and(|outcome| matches!(outcome.as_ref(), TemplateTerminalOutcome::Matched(_)));
    Sample::unmapped(started.elapsed(), correct)
}

#[test]
fn repeated_completed_queries_release_per_query_heap_and_report_progress() {
    let workload = measure(
        "template-watch-allocation-smoke",
        "every query matches and post-warmup live heap growth stays bounded",
        Plan::new(8, 64),
        fixture,
        one_query,
    );
    eprintln!(
        "template-watch-allocation-smoke growth_bytes={} peak_allocated_bytes={}",
        workload.growth_bytes(),
        workload.peak_allocated_bytes()
    );

    assert_eq!(workload.incorrect(), 0);
    assert!(
        workload.growth_bytes() <= 64 * 1024,
        "post-warmup growth was {} bytes",
        workload.growth_bytes()
    );
}
