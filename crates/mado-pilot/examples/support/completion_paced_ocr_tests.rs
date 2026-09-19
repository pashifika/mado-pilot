//! Behavioral tests use synthetic time and controlled pixels, never native authority.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mado_pilot::{
    CancellationToken, CaptureFault, ClipPolicy, Clock, Continuity, CoordinateSpace,
    CoordinateSupport, FrameOrder, GeometryFault, MonotonicInstant, OcrBackend, OcrRegion,
    OperationContext, PixelExtent, PixelFormat, PixelRect, Rect, Status, TargetCapability,
    TargetDescription, TargetKind,
};
use mado_pilot_runtime::IdentityIssuer;
use mado_pilot_testkit::{ControlledProducer, ManualClock};

use super::{EXTENT, Fixture, PROVIDER};
use crate::{
    CLOSE_TIMEOUT, Consumption, Decision, RunFailure, WAIT_SLICE, close_session, consume, cooldown,
    interpret_until, select_target, with_cleanup,
};

fn policy(milliseconds: u64) -> Consumption {
    Consumption {
        cooldown: Duration::from_millis(milliseconds),
        region: OcrRegion::FullFrame,
    }
}

fn full_extra_delay(inference_ms: u64, interpretation_ms: u64) {
    let fixture = Fixture::new(Duration::from_millis(inference_ms)).expect("controlled engine");
    fixture
        .source
        .publish(EXTENT, 7, Continuity::Continuous)
        .expect("initial frame");
    let operation = fixture
        .operation(Duration::from_secs(1))
        .expect("bounded operation");
    let mut interpreted = 0;
    let mut first_completion = None;
    let completed = consume(
        &fixture.session,
        &fixture.backend.descriptor(),
        policy(20),
        &operation,
        |_, _| {
            interpreted += 1;
            fixture
                .clock
                .advance(Duration::from_millis(interpretation_ms));
            if interpreted == 1 {
                first_completion = Some(fixture.clock.now());
                fixture.source.publish(EXTENT, 8, Continuity::Continuous)?;
                Ok(Decision::Continue)
            } else {
                Ok(Decision::Stop)
            }
        },
        |slice| fixture.clock.advance(slice),
    )
    .expect("two observations");
    let acquisitions = fixture.source.acquisition_times();
    assert_eq!(completed.observations, 2);
    assert_eq!(acquisitions[0], MonotonicInstant::ORIGIN);
    assert_eq!(
        acquisitions[1]
            .saturating_duration_since(first_completion.expect("interpretation completed")),
        Duration::from_millis(20),
    );
    // Termination adds neither a final cooldown nor another acquisition.
    assert_eq!(acquisitions.len(), 2);
    assert_eq!(
        fixture.clock.elapsed(),
        Duration::from_millis(2 * (inference_ms + interpretation_ms) + 20)
    );
    close_session(&fixture.session, &operation).expect("cleanup");
}

#[test]
fn short_ocr_and_interpretation_receive_a_full_additional_cooldown() {
    full_extra_delay(2, 3);
}

#[test]
fn long_ocr_is_not_credited_as_cooldown() {
    full_extra_delay(80, 3);
}

#[test]
fn long_interpretation_is_not_credited_as_cooldown() {
    full_extra_delay(2, 80);
}

#[test]
fn bursts_during_ocr_and_cooldown_collapse_to_the_final_live_idle_update() {
    let fixture = Fixture::new(Duration::from_millis(10)).expect("controlled engine");
    let first = fixture
        .source
        .publish(EXTENT, 1, Continuity::Continuous)
        .expect("initial frame");
    let source = Arc::clone(&fixture.source);
    *fixture.backend.on_admission.lock().expect("admission hook") = Some(Box::new(move || {
        source
            .publish(EXTENT, 2, Continuity::Continuous)
            .expect("busy publication");
        source
            .publish(EXTENT, 3, Continuity::Continuous)
            .expect("busy publication");
    }));
    let operation = fixture
        .operation(Duration::from_secs(1))
        .expect("bounded operation");
    let mut observed = Vec::new();
    let mut final_publication = None;
    let mut final_time = None;
    let completed = consume(
        &fixture.session,
        &fixture.backend.descriptor(),
        policy(20),
        &operation,
        |result, _| {
            observed.push(result.stamp());
            Ok(if observed.len() == 2 {
                Decision::Stop
            } else {
                Decision::Continue
            })
        },
        |slice| {
            fixture.clock.advance(slice);
            if final_publication.is_none() {
                fixture
                    .source
                    .publish(EXTENT, 4, Continuity::Continuous)
                    .expect("cooldown publication");
                final_publication = Some(
                    fixture
                        .source
                        .publish(EXTENT, 5, Continuity::Continuous)
                        .expect("final live publication"),
                );
                final_time = Some(fixture.clock.now());
            }
        },
    )
    .expect("final update remains eligible without another publication");
    let final_stamp = final_publication.expect("published during cooldown");
    assert_eq!(observed, [first, final_stamp]);
    assert_eq!(completed.last, final_stamp);
    assert_eq!(fixture.backend.ocr.recognition_count(), 2);
    assert_eq!(fixture.source.acquisition_times().len(), 2);
    assert!(fixture.source.acquisition_times()[1] > final_time.expect("capture instant"));
    assert_eq!(final_stamp.sequence().value(), first.sequence().value() + 4);
    close_session(&fixture.session, &operation).expect("cleanup");
}

#[test]
fn unchanged_stamp_waits_until_the_original_deadline_without_reanalysis() {
    let fixture = Fixture::new(Duration::ZERO).expect("controlled engine");
    fixture
        .source
        .publish(EXTENT, 7, Continuity::Continuous)
        .expect("initial frame");
    let operation = fixture
        .operation(Duration::from_millis(12))
        .expect("bounded operation");
    let error = consume(
        &fixture.session,
        &fixture.backend.descriptor(),
        policy(10),
        &operation,
        |_, _| Ok(Decision::Continue),
        |slice| fixture.clock.advance(slice),
    )
    .expect_err("the same stamp is not eligible twice");
    assert_eq!(error.status(), Status::DeadlineExceeded);
    assert_eq!(fixture.backend.ocr.recognition_count(), 1);
    assert_eq!(fixture.source.acquisition_times().len(), 2);
    assert!(fixture.source.idle_waits.load(Ordering::Relaxed) > 0);
    assert_eq!(fixture.clock.elapsed(), Duration::from_millis(12));
    close_session(&fixture.session, &operation).expect("independent cleanup");
}

#[test]
fn identical_pixels_with_a_new_stamp_and_sequence_gap_are_observations() {
    let fixture = Fixture::new(Duration::ZERO).expect("controlled engine");
    let first = fixture
        .source
        .publish(EXTENT, 7, Continuity::Continuous)
        .expect("initial frame");
    let operation = fixture
        .operation(Duration::from_secs(1))
        .expect("bounded operation");
    let mut observed = Vec::new();
    let completed = consume(
        &fixture.session,
        &fixture.backend.descriptor(),
        policy(10),
        &operation,
        |result, _| {
            observed.push(result.stamp());
            if observed.len() == 1 {
                for _ in 0..3 {
                    fixture.source.state.try_record_drop()?;
                }
                fixture.source.publish(EXTENT, 7, Continuity::Continuous)?;
                Ok(Decision::Continue)
            } else {
                Ok(Decision::Stop)
            }
        },
        |slice| fixture.clock.advance(slice),
    )
    .expect("repeated pixels do not invalidate a new observation");
    assert_eq!(completed.observations, 2);
    assert_eq!(observed[0], first);
    assert_eq!(observed[1].order(&first), Ok(FrameOrder::After));
    assert_eq!(observed[1].sequence().value(), first.sequence().value() + 4);
    assert_eq!(fixture.backend.ocr.recognition_count(), 2);
    close_session(&fixture.session, &operation).expect("cleanup");
}

#[test]
fn successor_epoch_resolves_normalized_region_against_its_own_geometry() {
    let fixture = Fixture::new(Duration::ZERO).expect("controlled engine");
    for _ in 0..4 {
        fixture
            .source
            .publish(EXTENT, 7, Continuity::Continuous)
            .expect("initial sequence");
    }
    let operation = fixture
        .operation(Duration::from_secs(1))
        .expect("bounded operation");
    let smaller = PixelExtent::new(16, 12);
    let mut observed = Vec::new();
    consume(
        &fixture.session,
        &fixture.backend.descriptor(),
        Consumption {
            region: OcrRegion::Region {
                rect: Rect::new(CoordinateSpace::FrameNormalized, 0.25, 0.25, 0.75, 0.75)
                    .expect("normalized region"),
                policy: ClipPolicy::Reject,
            },
            ..policy(10)
        },
        &operation,
        |result, _| {
            observed.push((
                result.stamp(),
                *result.transform(),
                result.effective_region(),
            ));
            if observed.len() == 1 {
                fixture
                    .source
                    .publish(smaller, 8, Continuity::Discontinuous)?;
                Ok(Decision::Continue)
            } else {
                Ok(Decision::Stop)
            }
        },
        |slice| fixture.clock.advance(slice),
    )
    .expect("region re-resolved in the successor frame");
    assert_eq!(observed[1].0.order(&observed[0].0), Ok(FrameOrder::After));
    assert!(observed[1].0.epoch() > observed[0].0.epoch());
    assert!(observed[1].0.sequence() < observed[0].0.sequence());
    assert!(observed[1].0.geometry() > observed[0].0.geometry());
    assert_eq!(observed[0].1.frame_extent(), EXTENT);
    assert_eq!(observed[1].1.frame_extent(), smaller);
    assert_eq!(observed[1].1.geometry(), observed[1].0.geometry());
    assert_eq!(
        observed[0].2,
        PixelRect::new(8, 6, 24, 18).expect("original bounds")
    );
    assert_eq!(
        observed[1].2,
        PixelRect::new(4, 3, 12, 9).expect("successor bounds")
    );
    close_session(&fixture.session, &operation).expect("cleanup");
}

#[test]
fn stale_pixel_region_is_refused_before_successor_ocr_admission() {
    let fixture = Fixture::new(Duration::ZERO).expect("controlled engine");
    fixture
        .source
        .publish(EXTENT, 7, Continuity::Continuous)
        .expect("initial frame");
    let operation = fixture
        .operation(Duration::from_secs(1))
        .expect("bounded operation");
    let mut interpretations = 0;
    let error = consume(
        &fixture.session,
        &fixture.backend.descriptor(),
        Consumption {
            region: OcrRegion::Region {
                rect: Rect::new(CoordinateSpace::CapturePixels, 8.0, 8.0, 28.0, 20.0)
                    .expect("original pixel region"),
                policy: ClipPolicy::Reject,
            },
            ..policy(10)
        },
        &operation,
        |_, _| {
            interpretations += 1;
            fixture
                .source
                .publish(PixelExtent::new(16, 12), 8, Continuity::Discontinuous)?;
            Ok(Decision::Continue)
        },
        |slice| fixture.clock.advance(slice),
    )
    .expect_err("old pixel region cannot reuse an earlier mapping");
    assert_eq!(error.status(), GeometryFault::OutsideExtent.status());
    assert_eq!(fixture.backend.ocr.recognition_count(), 1);
    assert_eq!(interpretations, 1);
    assert_eq!(fixture.source.acquisition_times().len(), 2);
    close_session(&fixture.session, &operation).expect("cleanup");
}

#[test]
fn source_termination_precedes_retained_latest_without_terminal_drain() {
    for fault in [CaptureFault::StreamEnded, CaptureFault::TargetLost] {
        let fixture = Fixture::new(Duration::ZERO).expect("controlled engine");
        fixture
            .source
            .publish(EXTENT, 7, Continuity::Continuous)
            .expect("initial frame");
        let operation = fixture
            .operation(Duration::from_secs(1))
            .expect("bounded operation");
        let mut ended = false;
        let error = consume(
            &fixture.session,
            &fixture.backend.descriptor(),
            policy(10),
            &operation,
            |_, _| Ok(Decision::Continue),
            |slice| {
                fixture.clock.advance(slice);
                if !ended {
                    fixture
                        .source
                        .publish(EXTENT, 8, Continuity::Continuous)
                        .expect("last publication");
                    fixture.source.state.terminate(fault);
                    ended = true;
                }
            },
        )
        .expect_err("source terminal status wins over a retained newer frame");
        assert_eq!(error.status(), fault.status());
        assert_eq!(fixture.backend.ocr.recognition_count(), 1);
        assert_eq!(fixture.source.acquisition_times().len(), 2);
        assert_eq!(fixture.clock.elapsed(), Duration::from_millis(10));
        close_session(&fixture.session, &operation).expect("cleanup");
    }
}

#[test]
fn session_close_during_cooldown_is_observed_at_the_next_acquisition() {
    let fixture = Fixture::new(Duration::ZERO).expect("controlled engine");
    fixture
        .source
        .publish(EXTENT, 7, Continuity::Continuous)
        .expect("initial frame");
    let operation = fixture
        .operation(Duration::from_secs(1))
        .expect("bounded operation");
    let mut closed = false;
    let error = consume(
        &fixture.session,
        &fixture.backend.descriptor(),
        policy(10),
        &operation,
        |_, _| Ok(Decision::Continue),
        |slice| {
            fixture.clock.advance(slice);
            if !closed {
                close_session(&fixture.session, &operation).expect("another owner's close");
                closed = true;
            }
        },
    )
    .expect_err("closed session refuses a next frame");
    assert_eq!(error.status(), Status::Closed);
    assert_eq!(fixture.clock.elapsed(), Duration::from_millis(10));
    assert_eq!(fixture.backend.ocr.recognition_count(), 1);
    assert!(fixture.session.is_closed());
}

#[test]
fn capture_loss_during_admitted_ocr_allows_exact_commit_then_next_acquire_fails() {
    let fixture = Fixture::new(Duration::from_millis(30)).expect("controlled engine");
    let first = fixture
        .source
        .publish(EXTENT, 7, Continuity::Continuous)
        .expect("initial frame");
    let source = Arc::clone(&fixture.source);
    *fixture.backend.on_admission.lock().expect("admission hook") = Some(Box::new(move || {
        source.state.terminate(CaptureFault::TargetLost);
    }));
    let operation = fixture
        .operation(Duration::from_secs(1))
        .expect("bounded operation");
    let mut interpreted = Vec::new();
    let error = consume(
        &fixture.session,
        &fixture.backend.descriptor(),
        policy(10),
        &operation,
        |result, _| {
            interpreted.push(result.stamp());
            Ok(Decision::Continue)
        },
        |slice| fixture.clock.advance(slice),
    )
    .expect_err("next acquisition must report capture loss");
    assert_eq!(interpreted, [first]);
    assert_eq!(error.status(), Status::TargetLost);
    assert_eq!(fixture.backend.ocr.recognition_count(), 1);
    assert_eq!(fixture.clock.elapsed(), Duration::from_millis(40));
    close_session(&fixture.session, &operation).expect("cleanup");
}

#[test]
fn session_close_during_admitted_ocr_prevents_interpretation_and_cooldown() {
    let fixture = Fixture::new(Duration::from_millis(30)).expect("controlled engine");
    fixture
        .source
        .publish(EXTENT, 7, Continuity::Continuous)
        .expect("initial frame");
    let session = Arc::downgrade(&fixture.session);
    let operation = fixture
        .operation(Duration::from_secs(1))
        .expect("bounded operation");
    let close_operation = operation.clone();
    *fixture.backend.on_admission.lock().expect("admission hook") = Some(Box::new(move || {
        close_session(&session.upgrade().expect("live caller"), &close_operation)
            .expect("close while OCR is admitted");
    }));
    let error = consume(
        &fixture.session,
        &fixture.backend.descriptor(),
        policy(10),
        &operation,
        |_, _| panic!("closed-session OCR cannot reach interpretation"),
        |_| panic!("failed OCR cannot reach cooldown"),
    )
    .expect_err("session commit gate rejects the late result");
    assert_eq!(error.status(), Status::Closed);
    assert_eq!(fixture.backend.ocr.recognition_count(), 1);
    assert_eq!(fixture.source.acquisition_times().len(), 1);
    assert!(fixture.session.is_closed());
}

#[test]
fn frame_and_mapping_ownership_is_released_before_the_first_cooldown_slice() {
    let fixture = Fixture::new(Duration::ZERO).expect("controlled engine");
    let producer =
        ControlledProducer::new(EXTENT, PixelFormat::Rgba8, 1, 2).expect("two detached slots");
    fixture
        .source
        .state
        .publish_storage(
            producer
                .publication(7, Continuity::Continuous)
                .expect("first storage"),
        )
        .map_err(|refused| refused.into_error())
        .expect("first publication");
    let operation = fixture
        .operation(Duration::from_secs(1))
        .expect("bounded operation");
    let mut interpreted = 0;
    let mut checked_release = false;
    consume(
        &fixture.session,
        &fixture.backend.descriptor(),
        policy(10),
        &operation,
        |_, _| {
            interpreted += 1;
            if interpreted == 1 {
                fixture
                    .source
                    .state
                    .publish_storage(producer.publication(8, Continuity::Continuous)?)
                    .map_err(|refused| refused.into_error())?;
                assert_eq!(producer.detached_slots_free(), 0);
                Ok(Decision::Continue)
            } else {
                Ok(Decision::Stop)
            }
        },
        |slice| {
            if !checked_release {
                assert_eq!(producer.detached_slots_free(), 1);
                assert_eq!(producer.releases(), 1);
                fixture
                    .source
                    .state
                    .publish_storage(
                        producer
                            .publication(9, Continuity::Continuous)
                            .expect("released capacity can publish"),
                    )
                    .map_err(|refused| refused.into_error())
                    .expect("no retained observation backlog");
                checked_release = true;
            }
            fixture.clock.advance(slice);
        },
    )
    .expect("capture continues within its finite detached capacity");
    assert!(checked_release);
    assert_eq!(producer.producer_slots_free(), 1);
    close_session(&fixture.session, &operation).expect("cleanup");
    assert_eq!(producer.detached_slots_free(), 2);
}

#[test]
fn cooldown_interruption_never_reaches_next_acquisition_or_ocr_admission() {
    // Separate races: mid-slice cancellation, clipped deadline, simultaneous
    // interruption, and cancellation/deadline exactly at cooldown completion.
    for (deadline_ms, cancel_ms, expected) in [
        (100, Some(2), Status::Cancelled),
        (3, None, Status::DeadlineExceeded),
        (3, Some(3), Status::Cancelled),
        (100, Some(4), Status::Cancelled),
        (4, None, Status::DeadlineExceeded),
    ] {
        let fixture = Fixture::new(Duration::ZERO).expect("controlled engine");
        fixture
            .source
            .publish(EXTENT, 7, Continuity::Continuous)
            .expect("initial frame");
        let token = CancellationToken::new();
        let operation = fixture
            .operation(Duration::from_millis(deadline_ms))
            .expect("bounded operation")
            .with_cancellation(token.clone());
        let mut slices = Vec::new();
        let error = consume(
            &fixture.session,
            &fixture.backend.descriptor(),
            policy(4),
            &operation,
            |_, _| {
                fixture.source.publish(EXTENT, 8, Continuity::Continuous)?;
                Ok(Decision::Continue)
            },
            |slice| {
                assert!(!slice.is_zero() && slice <= WAIT_SLICE);
                slices.push(slice);
                fixture.clock.advance(slice);
                if cancel_ms.is_some_and(|at| fixture.clock.elapsed() >= Duration::from_millis(at))
                {
                    token.cancel();
                }
            },
        )
        .expect_err("interruption wins before another acquisition");
        assert_eq!(error.status(), expected);
        assert_eq!(fixture.source.acquisition_times().len(), 1);
        assert_eq!(fixture.backend.ocr.recognition_count(), 1);
        if deadline_ms == 3 {
            assert_eq!(slices, [Duration::from_millis(2), Duration::from_millis(1)]);
        }
        close_session(&fixture.session, &operation).expect("independent cleanup");
    }
}

#[derive(Debug)]
struct CancelOnClockRead {
    clock: Arc<ManualClock>,
    token: CancellationToken,
    armed: AtomicBool,
}

impl Clock for CancelOnClockRead {
    fn now(&self) -> MonotonicInstant {
        if self.armed.swap(false, Ordering::AcqRel) {
            self.token.cancel();
        }
        self.clock.now()
    }
}

#[test]
fn final_cooldown_check_catches_cancellation_during_the_last_clock_sample() {
    let clock = Arc::new(CancelOnClockRead {
        clock: Arc::new(ManualClock::new()),
        token: CancellationToken::new(),
        armed: AtomicBool::new(false),
    });
    let operation = OperationContext::new()
        .with_clock(clock.clone())
        .with_cancellation(clock.token.clone())
        .with_timeout(Duration::from_secs(1))
        .expect("bounded operation");
    let error = cooldown(WAIT_SLICE, &operation, &mut |slice| {
        clock.clock.advance(slice);
        clock.armed.store(true, Ordering::Release);
    })
    .expect_err("completion still arbitrates interruption");
    assert_eq!(error.status(), Status::Cancelled);
}

#[test]
fn elapsed_cooldown_arithmetic_cannot_overflow_or_extend_an_absolute_deadline() {
    let clock = Arc::new(ManualClock::new());
    clock.advance(Duration::MAX - Duration::from_millis(5));
    let operation = OperationContext::new()
        .with_clock(clock.clone())
        .with_deadline(MonotonicInstant::from_origin(Duration::MAX));
    let mut slices = Vec::new();
    let error = cooldown(Duration::MAX, &operation, &mut |slice| {
        slices.push(slice);
        clock.advance(slice);
    })
    .expect_err("the original deadline wins even when a cooldown endpoint would overflow");
    assert_eq!(error.status(), Status::DeadlineExceeded);
    assert_eq!(
        slices,
        [
            Duration::from_millis(2),
            Duration::from_millis(2),
            Duration::from_millis(1)
        ]
    );
    assert_eq!(clock.elapsed(), Duration::MAX);
}

#[test]
fn interruption_at_interpretation_completion_prevents_waiting_even_on_stop() {
    let fixture = Fixture::new(Duration::ZERO).expect("controlled engine");
    fixture
        .source
        .publish(EXTENT, 7, Continuity::Continuous)
        .expect("initial frame");
    let token = CancellationToken::new();
    let operation = fixture
        .operation(Duration::from_secs(1))
        .expect("bounded operation")
        .with_cancellation(token.clone());
    let error = consume(
        &fixture.session,
        &fixture.backend.descriptor(),
        policy(10),
        &operation,
        |result, operation| {
            let decision = interpret_until(result, "READY", operation)?;
            token.cancel();
            Ok(decision)
        },
        |_| panic!("interrupted interpretation cannot start cooldown"),
    )
    .expect_err("late stop decision cannot override cancellation");
    assert_eq!(error.status(), Status::Cancelled);
    assert_eq!(fixture.source.acquisition_times().len(), 1);
    assert_eq!(fixture.backend.ocr.recognition_count(), 1);
    close_session(&fixture.session, &operation).expect("independent cleanup");
}

#[test]
fn cancelled_work_and_failed_cleanup_preserve_both_failures_and_independent_authority() {
    let fixture = Fixture::new(Duration::ZERO).expect("controlled engine");
    fixture
        .source
        .publish(EXTENT, 7, Continuity::Continuous)
        .expect("initial frame");
    let token = CancellationToken::new();
    let operation = fixture
        .operation(Duration::from_millis(2))
        .expect("bounded operation")
        .with_cancellation(token.clone());
    let work = consume(
        &fixture.session,
        &fixture.backend.descriptor(),
        policy(10),
        &operation,
        |_, _| Ok(Decision::Continue),
        |slice| {
            fixture.clock.advance(slice);
            token.cancel();
        },
    );
    *fixture.source.close_failure.lock().expect("close failure") = Some(Status::CaptureFailed);
    let error = with_cleanup(work, close_session(&fixture.session, &operation))
        .expect_err("both failures remain observable");
    match error {
        RunFailure::WorkAndCleanup(work, cleanup) => {
            assert_eq!(work.status(), Status::Cancelled);
            assert_eq!(cleanup.status(), Status::CaptureFailed);
        }
        other => panic!("lost one failure: {other:?}"),
    }
    assert_eq!(
        *fixture
            .source
            .close_authorities
            .lock()
            .expect("close authority"),
        [(false, Some(CLOSE_TIMEOUT))],
    );
    assert!(!fixture.session.is_closed());
    *fixture.source.close_failure.lock().expect("close failure") = None;
    close_session(&fixture.session, &operation).expect("retryable independent cleanup");
    assert!(fixture.session.is_closed());
}

#[test]
fn exact_stop_does_not_cool_down_and_cleanup_failure_is_not_success() {
    let fixture = Fixture::new(Duration::from_millis(3)).expect("controlled engine");
    fixture
        .source
        .publish(EXTENT, 7, Continuity::Continuous)
        .expect("initial frame");
    let operation = fixture
        .operation(Duration::from_secs(1))
        .expect("bounded operation");
    let work = consume(
        &fixture.session,
        &fixture.backend.descriptor(),
        policy(10),
        &operation,
        |result, operation| {
            assert_eq!(
                interpret_until(result, "REA", operation)?,
                Decision::Continue
            );
            interpret_until(result, "READY", operation)
        },
        |_| panic!("stop has no cooldown"),
    );
    assert_eq!(work.as_ref().expect("exact match stops").observations, 1);
    *fixture.source.close_failure.lock().expect("close failure") = Some(Status::DeadlineExceeded);
    assert!(matches!(
        with_cleanup(work, close_session(&fixture.session, &operation)),
        Err(RunFailure::Cleanup(error)) if error.status() == Status::DeadlineExceeded,
    ));
    assert_eq!(fixture.source.acquisition_times().len(), 1);
    assert_eq!(fixture.clock.elapsed(), Duration::from_millis(3));
    *fixture.source.close_failure.lock().expect("close failure") = None;
    close_session(&fixture.session, &operation).expect("cleanup");
}

#[test]
fn target_selection_requires_one_exact_name_and_kind_without_fallback() {
    let issuer = IdentityIssuer::new();
    let window_id = issuer.issue_target(PROVIDER).expect("window identity");
    let describe = |id, kind| {
        TargetDescription::new(
            id,
            "authorized",
            EXTENT,
            PixelFormat::Rgba8,
            CoordinateSupport::frame_only(),
        )
        .with_capability(TargetCapability::capture_only(kind))
    };
    let mut targets = vec![
        describe(
            issuer.issue_target(PROVIDER).expect("display identity"),
            TargetKind::Display,
        ),
        describe(window_id, TargetKind::Window),
    ];
    assert_eq!(
        select_target(&targets, TargetKind::Window, "authorized").expect("unique exact window"),
        window_id
    );
    assert_eq!(
        select_target(&targets, TargetKind::Window, "author")
            .expect_err("substring is not authority")
            .status(),
        Status::InvalidArgument
    );
    targets.push(describe(
        issuer.issue_target(PROVIDER).expect("duplicate name"),
        TargetKind::Window,
    ));
    assert_eq!(
        select_target(&targets, TargetKind::Window, "authorized")
            .expect_err("ambiguous name refuses capture")
            .status(),
        Status::InvalidArgument
    );
}
