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

use support::{CancellingClock, TempDir, TickingClock, tiny_archive, tiny_directory};

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
    let missing = {
        let temporary = TempDir::new("cancelled-before-enumeration");
        temporary.path().to_path_buf()
    };
    let token = CancellationToken::new();
    token.cancel();
    let context = OperationContext::new().with_cancellation(token);

    let fault = PackageLoader::new()
        .load(&PackageSource::directory(missing), &context)
        .expect_err("cancelled before the missing source can be opened");

    assert_eq!(fault.kind(), AssetFaultKind::Cancelled);
    assert_eq!(fault.stage(), LoadStage::Source);
    assert_eq!(fault.status(), Status::Cancelled);
}

#[test]
fn a_load_that_begins_already_expired_never_opens_the_source() {
    let missing = {
        let temporary = TempDir::new("expired-before-enumeration");
        temporary.path().to_path_buf()
    };
    let context = OperationContext::new()
        .with_clock(Arc::new(TickingClock::new()))
        .with_deadline(MonotonicInstant::ORIGIN);

    let fault = PackageLoader::new()
        .load(&PackageSource::directory(missing), &context)
        .expect_err("expired before the missing source can be opened");

    assert_eq!(fault.kind(), AssetFaultKind::DeadlineExceeded);
    assert_eq!(fault.stage(), LoadStage::Source);
    assert_eq!(fault.status(), Status::DeadlineExceeded);
}

#[test]
fn a_deadline_during_directory_enumeration_is_reported_at_the_source_stage() {
    let temporary = TempDir::new("deadline-during-enumeration");
    temporary.write(mado_pilot_assets::MANIFEST_PATH, &support::empty_manifest());
    for index in 0..16 {
        temporary.write(&format!("entries/{index:02}.bin"), b"entry");
    }
    let context = OperationContext::new()
        .with_clock(Arc::new(TickingClock::new()))
        .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(5)));

    let fault = PackageLoader::new()
        .load(&PackageSource::directory(temporary.path()), &context)
        .expect_err("the deadline expires while the tree is enumerated");

    assert_eq!(fault.kind(), AssetFaultKind::DeadlineExceeded);
    assert_eq!(fault.stage(), LoadStage::Source);
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

/// Counts the context checks one successful borrowed-archive load performs.
fn checks_for_borrowed(bytes: &[u8]) -> u64 {
    let clock = Arc::new(TickingClock::new());
    let context = OperationContext::new()
        .with_clock(clock.clone())
        .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(
            UNREACHABLE_MILLIS,
        )));
    PackageLoader::new()
        .load_archive_bytes(bytes, &context)
        .expect("the borrowed archive loads when nothing interrupts it");
    clock.reads()
}

#[test]
fn no_deadline_anywhere_inside_a_borrowed_archive_load_produces_a_package() {
    let bytes = std::fs::read(tiny_archive()).expect("readable fixture archive");
    let total = checks_for_borrowed(&bytes);
    let mut stages = Vec::new();

    for deadline in 0..total {
        let context = OperationContext::new()
            .with_clock(Arc::new(TickingClock::new()))
            .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(
                deadline,
            )));
        let fault = PackageLoader::new()
            .load_archive_bytes(&bytes, &context)
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
    assert_commits_through_the_operation(&stages);
}

#[test]
fn no_cancellation_anywhere_inside_a_borrowed_archive_load_produces_a_package() {
    let bytes = std::fs::read(tiny_archive()).expect("readable fixture archive");
    let total = checks_for_borrowed(&bytes);
    let mut stages = Vec::new();

    // Cancelling on read `point` is observed by the next context check, so the
    // last read that can still be observed is `total - 2`. What that last read
    // *is* matters here: the borrowed archive is published by `Operation::commit`,
    // so the sweep's final point is the publication point rather than a boundary
    // with nothing after it.
    for point in 0..total.saturating_sub(1) {
        let token = CancellationToken::new();
        let context = OperationContext::new()
            .with_clock(Arc::new(CancellingClock::new(token.clone(), point)))
            .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(
                UNREACHABLE_MILLIS,
            )))
            .with_cancellation(token);

        let fault = PackageLoader::new()
            .load_archive_bytes(&bytes, &context)
            .expect_err("cancellation lands before the load finishes");

        assert_eq!(fault.kind(), AssetFaultKind::Cancelled, "at {point}");
        stages.push(fault.stage());
    }

    assert!(
        stages.contains(&LoadStage::Expansion),
        "some cancellation must land while archive entries are being expanded"
    );
    assert_commits_through_the_operation(&stages);
}

/// Asserts that the last two interruption points of a sweep are the commit's own.
///
/// That a sweep reaches `LoadStage::Commit` says less than it appears to. The
/// stage is reported by *two* consecutive observation points — the checkpoint the
/// pipeline takes at that stage, and `Operation::commit` itself — so a load that
/// checkpointed and then published without committing through the operation would
/// still put one `Commit` in the list, and a membership test would pass. The
/// discriminating property is that the last two are both `Commit`: delete the
/// final commit and the second-to-last point falls back to expansion.
fn assert_commits_through_the_operation(stages: &[LoadStage]) {
    let count = stages.len();
    assert!(count >= 2, "a sweep this short proves nothing: {count}");
    assert_eq!(
        (stages[count - 2], stages[count - 1]),
        (LoadStage::Commit, LoadStage::Commit),
        "the last two interruption points must be the commit checkpoint and the \
         commit itself, so that publishing without the operation's final check \
         cannot pass this sweep"
    );
}

#[test]
fn a_borrowed_archive_loads_as_the_file_it_was_read_from() {
    let bytes = std::fs::read(tiny_archive()).expect("readable fixture archive");
    let context = OperationContext::new();

    let borrowed = PackageLoader::new()
        .load_archive_bytes(&bytes, &context)
        .expect("the borrowed archive loads");
    let owned = PackageLoader::new()
        .load(&PackageSource::archive_bytes(bytes), &context)
        .expect("the owned archive loads");
    let from_file = PackageLoader::new()
        .load(&PackageSource::archive_file(tiny_archive()), &context)
        .expect("the file loads");

    assert_eq!(borrowed, from_file);
    assert_eq!(
        borrowed, owned,
        "ownership is not part of what a package is"
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
