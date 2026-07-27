//! Deadline and cancellation propagation through asset loading.
//!
//! Time is a clock these tests control, never a sleep. Both clocks below
//! advance one millisecond per read, so a deadline is a count of context checks
//! rather than a wall-clock wait. A test that raced a real timer would pass or
//! fail on how busy the machine was, and would say nothing about *where* the
//! loader noticed.
//!
//! The interruption tests are written as sweeps over every possible
//! interruption point rather than as one hand-picked point. The property worth
//! pinning is not that interruption at check seventeen is reported at expansion
//! — it is that no interruption anywhere produces a package, and that the
//! points do reach as deep as expansion and commit.

use std::sync::Arc;
use std::time::Duration;

use mado_pilot_assets::{AssetFaultKind, LoadStage, PackageLoader, PackageSource};
use mado_pilot_core::{CancellationToken, MonotonicInstant, OperationContext, Status};

mod support;

use support::{CancellingClock, TickingClock, tiny_archive, tiny_directory};

/// A deadline no sweep can reach: the clock would have to be read 3.6 million
/// times.
const UNREACHABLE_MILLIS: u64 = 3_600_000;

/// Counts the context checks one successful load performs.
fn checks_for(source: &PackageSource) -> u64 {
    let clock = Arc::new(TickingClock::new());
    let context = OperationContext::new()
        .with_clock(clock.clone())
        .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(
            UNREACHABLE_MILLIS,
        )));
    PackageLoader::new()
        .load(source, &context)
        .expect("the source loads when nothing interrupts it");
    clock.reads()
}

#[test]
fn a_load_that_begins_already_cancelled_never_opens_the_source() {
    let token = CancellationToken::new();
    token.cancel();
    let context = OperationContext::new().with_cancellation(token);

    let fault = PackageLoader::new()
        .load(&PackageSource::directory(tiny_directory()), &context)
        .expect_err("cancelled");

    assert_eq!(fault.kind(), AssetFaultKind::Cancelled);
    assert_eq!(fault.stage(), LoadStage::Source);
    assert_eq!(fault.status(), Status::Cancelled);
}

#[test]
fn a_load_that_begins_already_expired_never_opens_the_source() {
    let context = OperationContext::new()
        .with_clock(Arc::new(TickingClock::new()))
        .with_deadline(MonotonicInstant::ORIGIN);

    let fault = PackageLoader::new()
        .load(&PackageSource::directory(tiny_directory()), &context)
        .expect_err("expired");

    assert_eq!(fault.kind(), AssetFaultKind::DeadlineExceeded);
    assert_eq!(fault.stage(), LoadStage::Source);
    assert_eq!(fault.status(), Status::DeadlineExceeded);
}

#[test]
fn no_deadline_anywhere_inside_a_load_produces_a_package() {
    let source = PackageSource::directory(tiny_directory());
    let total = checks_for(&source);
    let mut stages = Vec::new();

    for deadline in 0..total {
        let context = OperationContext::new()
            .with_clock(Arc::new(TickingClock::new()))
            .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(
                deadline,
            )));
        let fault = PackageLoader::new()
            .load(&source, &context)
            .expect_err("the deadline expires before the load finishes");

        assert_eq!(
            fault.kind(),
            AssetFaultKind::DeadlineExceeded,
            "at {deadline}"
        );
        stages.push(fault.stage());
    }

    assert!(
        stages.contains(&LoadStage::Expansion),
        "some deadline must expire while entries are being read and hashed"
    );
    assert!(
        stages.contains(&LoadStage::Commit),
        "some deadline must expire at the final check before commit"
    );
}

#[test]
fn no_cancellation_anywhere_inside_a_load_produces_a_package() {
    let source = PackageSource::archive_file(tiny_archive());
    let total = checks_for(&source);
    let mut stages = Vec::new();

    // Cancelling on read `point` is observed by the next context check, so the
    // last read that can still be observed is `total - 2`.
    for point in 0..total.saturating_sub(1) {
        let token = CancellationToken::new();
        let context = OperationContext::new()
            .with_clock(Arc::new(CancellingClock::new(token.clone(), point)))
            .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(
                UNREACHABLE_MILLIS,
            )))
            .with_cancellation(token);

        let fault = PackageLoader::new()
            .load(&source, &context)
            .expect_err("cancellation lands before the load finishes");

        assert_eq!(fault.kind(), AssetFaultKind::Cancelled, "at {point}");
        stages.push(fault.stage());
    }

    assert!(
        stages.contains(&LoadStage::Expansion),
        "some cancellation must land while archive entries are being expanded"
    );
}

#[test]
fn cancellation_is_reported_ahead_of_expiry_when_both_hold() {
    let token = CancellationToken::new();
    token.cancel();
    let context = OperationContext::new()
        .with_clock(Arc::new(TickingClock::new()))
        .with_deadline(MonotonicInstant::ORIGIN)
        .with_cancellation(token);

    let fault = PackageLoader::new()
        .load(&PackageSource::directory(tiny_directory()), &context)
        .expect_err("interrupted");

    assert_eq!(
        fault.kind(),
        AssetFaultKind::Cancelled,
        "one shared token must produce one consistent answer across operations"
    );
}

#[test]
fn a_context_with_no_deadline_loads_however_long_it_takes() {
    let package = PackageLoader::new()
        .load(
            &PackageSource::directory(tiny_directory()),
            &OperationContext::new(),
        )
        .expect("no deadline means no deadline, not a very large one");

    assert_eq!(package.template_count(), 6);
}

#[test]
fn a_deadline_beyond_the_work_commits_normally_and_is_still_consulted() {
    let source = PackageSource::archive_file(tiny_archive());
    let clock = Arc::new(TickingClock::new());
    let context = OperationContext::new()
        .with_clock(clock.clone())
        .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(
            UNREACHABLE_MILLIS,
        )));

    let package = PackageLoader::new()
        .load(&source, &context)
        .expect("committed in time");

    assert_eq!(package.template_count(), 6);
    let templates = u64::try_from(package.template_count()).expect("six templates");
    assert!(
        clock.reads() > templates,
        "the context must be consulted at least once per entry, not once per load"
    );
}

#[test]
fn cancellation_after_a_successful_commit_cannot_take_the_package_back() {
    let token = CancellationToken::new();
    let context = OperationContext::new().with_cancellation(token.clone());

    let package = PackageLoader::new()
        .load(&PackageSource::directory(tiny_directory()), &context)
        .expect("committed before cancellation");
    token.cancel();

    assert_eq!(package.template_count(), 6);
    assert!(package.resolve_template("template.0000").is_ok());
}
