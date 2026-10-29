//! The controlled matcher against the vision contract, and the public rules
//! against a backend whose answers are known in advance.
//!
//! A scripted backend is the only way to test the rules that sit between a
//! backend and a caller. A real matcher cannot be asked for two candidates with
//! exactly equal scores at chosen coordinates, or for a score outside the public
//! range, or to finish only after a deadline has passed — and those are the
//! cases where the rules either hold or quietly do not.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mado_pilot_capture::{Frame, FrameDescriptor, PixelFormat};
use mado_pilot_core::{
    CancellationToken, ClipPolicy, CoordinateSpace, MonotonicInstant, OperationContext,
    PixelExtent, PixelRect, Rect, Status,
};
use mado_pilot_testkit::controlled_matcher::{Behavior, CONTROLLED_BACKEND, ControlledMatcher};
use mado_pilot_testkit::vision_contract::{self, frame, template};
use mado_pilot_testkit::{CompletionGate, ManualClock, ScriptedMatchCall};
use mado_pilot_vision::{
    BackendDescriptor, BackendRequest, Candidate, MatchBackend, MatchDefaults, MatchOptions,
    MatchRequest, Matcher, PreparedTemplate, RegionSelection, Suppression, TemplateSource,
};

const FORMAT: PixelFormat = PixelFormat::Rgba8;

fn backend(matcher: ControlledMatcher) -> Arc<dyn MatchBackend> {
    Arc::new(matcher)
}

fn options() -> MatchOptions {
    MatchOptions::from_defaults(MatchDefaults::new(0.5, 8).expect("valid"))
}

/// Searches a 64x64 frame for an 8x8 template with `candidates` scripted.
fn search_full_frame(
    candidates: Vec<Candidate>,
    options: MatchOptions,
) -> mado_pilot_core::Result<mado_pilot_vision::MatchResult> {
    let matcher = Matcher::new(backend(
        ControlledMatcher::new(FORMAT).with_candidates(candidates),
    ));
    let prepared = matcher
        .prepare(
            &template("t", PixelExtent::new(8, 8)),
            &OperationContext::new(),
        )
        .expect("prepares");
    let image = frame(PixelExtent::new(64, 64), FORMAT, 0);

    matcher.find(
        MatchRequest::new(&image, RegionSelection::FullFrame, &prepared, options),
        &OperationContext::new(),
    )
}

fn reported(result: &mado_pilot_vision::MatchResult) -> Vec<(i32, i32, f64)> {
    result
        .matches()
        .iter()
        .map(|found| (found.bounds().left(), found.bounds().top(), found.score()))
        .collect()
}

#[test]
fn the_controlled_matcher_satisfies_the_vision_contract() {
    vision_contract::run(&backend(ControlledMatcher::new(FORMAT)));
}

#[test]
fn the_controlled_matcher_satisfies_the_contract_in_every_pixel_format() {
    for format in [PixelFormat::Rgba8, PixelFormat::Bgra8] {
        vision_contract::run(&backend(ControlledMatcher::new(format)));
    }
}

#[test]
fn alternate_formats_are_explicit_and_canonical() {
    for (default, alternate) in [
        (PixelFormat::Bgra8, PixelFormat::Rgba8),
        (PixelFormat::Rgba8, PixelFormat::Bgra8),
    ] {
        let single = ControlledMatcher::new(default).descriptor();
        assert_eq!(single.format(), default);
        assert!(single.accepts_format(default));
        assert!(!single.accepts_format(alternate));
        assert_eq!(single.clone().with_alternate_format(default), single);

        let dual = single.clone().with_alternate_format(alternate);
        assert_ne!(dual, single);
        assert_eq!(dual.format(), default);
        assert!(dual.accepts_format(default));
        assert!(dual.accepts_format(alternate));
        assert_eq!(dual.clone().with_alternate_format(alternate), dual);
        assert_eq!(dual.with_alternate_format(default), single);
        vision_contract::run(&backend(
            ControlledMatcher::new(default).with_alternate_format(alternate),
        ));
    }
}

#[derive(Debug)]
struct MutableDescriptorBackend {
    descriptor: Mutex<BackendDescriptor>,
    inner: ControlledMatcher,
}

impl MatchBackend for MutableDescriptorBackend {
    fn descriptor(&self) -> BackendDescriptor {
        self.descriptor.lock().expect("descriptor").clone()
    }

    fn prepare(
        &self,
        source: &TemplateSource,
        operation: &OperationContext,
    ) -> mado_pilot_core::Result<PreparedTemplate> {
        self.inner.prepare(source, operation)
    }

    fn find(
        &self,
        request: &BackendRequest<'_>,
        operation: &OperationContext,
    ) -> mado_pilot_core::Result<Vec<Candidate>> {
        self.inner.find(request, operation)
    }
}

#[test]
fn descriptor_snapshots_select_formats_and_refuse_incompatible_mapped_inputs() {
    let single = ControlledMatcher::new(PixelFormat::Bgra8).descriptor();
    let dual = single.clone().with_alternate_format(PixelFormat::Rgba8);
    let adapter = Arc::new(MutableDescriptorBackend {
        descriptor: Mutex::new(dual.clone()),
        inner: ControlledMatcher::new(PixelFormat::Bgra8)
            .with_alternate_format(PixelFormat::Rgba8)
            .with_candidates(vec![Candidate::new(0, 0, 0.9)]),
    });
    let preserving = Matcher::new(adapter.clone());
    *adapter.descriptor.lock().expect("descriptor") = single.clone();
    let fallback = Matcher::new(adapter.clone());
    let operation = OperationContext::new();
    let source = template("snapshot", PixelExtent::new(8, 8));
    let prepared = preserving.prepare(&source, &operation).expect("prepares");
    let other = preserving
        .prepare(&source, &operation)
        .expect("prepares again");
    let image = frame(PixelExtent::new(64, 64), PixelFormat::Rgba8, 0);
    let request = MatchRequest::new(&image, RegionSelection::FullFrame, &prepared, options());
    let direct = preserving
        .map_match(&request, &operation)
        .expect("direct mapping");
    let converted = fallback
        .map_match(&request, &operation)
        .expect("fallback mapping");
    assert_eq!(preserving.descriptor(), dual);
    assert_eq!(fallback.descriptor(), single);
    assert_eq!(
        direct.pixels().expect("pixels").descriptor().format(),
        PixelFormat::Rgba8
    );
    assert_eq!(
        converted.pixels().expect("pixels").descriptor().format(),
        PixelFormat::Bgra8
    );
    assert!(preserving.requests_are_equivalent(&request, &request));
    assert!(direct.is_equivalent_request(&request));
    assert!(!direct.is_equivalent_request(&MatchRequest::new(
        &image,
        RegionSelection::FullFrame,
        &other,
        options(),
    )));
    assert_eq!(
        preserving
            .find_mapped(&direct, &other, &operation)
            .expect_err("a different compiled instance cannot reuse the request")
            .status(),
        Status::InvalidArgument
    );
    for (matcher, incompatible) in [(&preserving, &converted), (&fallback, &direct)] {
        assert_eq!(
            matcher
                .find_mapped(incompatible, &prepared, &operation)
                .expect_err("different snapshotted capabilities are incompatible")
                .status(),
            Status::InvalidArgument
        );
    }
    for (matcher, mapped) in [(&preserving, &direct), (&fallback, &converted)] {
        let result = matcher
            .find_mapped(mapped, &prepared, &operation)
            .expect("own mapping");
        assert_eq!(reported(&result), vec![(0, 0, 0.9)]);
        assert_eq!(result.backend(), &matcher.descriptor());
    }
}

#[test]
fn exact_reuse_distinguishes_equal_stamp_pixel_and_layout_collisions() {
    let matcher = Matcher::new(backend(
        ControlledMatcher::new(PixelFormat::Bgra8)
            .with_alternate_format(PixelFormat::Rgba8)
            .with_candidates(vec![Candidate::new(0, 0, 0.9)]),
    ));
    let operation = OperationContext::new();
    let prepared = matcher
        .prepare(&template("identity", PixelExtent::new(8, 8)), &operation)
        .expect("prepares");
    let image = frame(PixelExtent::new(64, 64), PixelFormat::Rgba8, 0x21);
    let clone = image.clone();
    let original = MatchRequest::new(&image, RegionSelection::FullFrame, &prepared, options());
    let cloned = MatchRequest::new(&clone, RegionSelection::FullFrame, &prepared, options());
    let mapped = matcher
        .map_match(&original, &operation)
        .expect("original mapping");
    assert!(image.is_same_instance(&clone));
    assert!(matcher.requests_are_equivalent(&original, &cloned));
    assert!(mapped.is_equivalent_request(&cloned));

    for (format, padding, fill) in [
        (PixelFormat::Rgba8, 0, 0x87),
        (PixelFormat::Bgra8, 0, 0x21),
        (PixelFormat::Rgba8, 8, 0x21),
    ] {
        let descriptor =
            FrameDescriptor::new(image.descriptor().extent(), format, 64 * 4 + padding)
                .expect("valid colliding descriptor");
        let independent = Frame::new(
            image.stamp(),
            image.captured_at(),
            descriptor,
            *image.transform(),
            vec![fill; descriptor.byte_len()].into_boxed_slice(),
        )
        .expect("independent frame with the same numeric stamp");
        let request = MatchRequest::new(
            &independent,
            RegionSelection::FullFrame,
            &prepared,
            options(),
        );
        assert!(!image.is_same_instance(&independent));
        assert!(!matcher.requests_are_equivalent(&original, &request));
        assert!(!mapped.is_equivalent_request(&request));
        let own = matcher
            .map_match(&request, &operation)
            .expect("independent mapping");
        let pixels = own.pixels().expect("pixels");
        assert_eq!(pixels.descriptor(), descriptor);
        assert!(pixels.bytes().iter().all(|byte| *byte == fill));
        assert!(own.frame().is_same_instance(&independent));
        assert_eq!(
            reported(
                &matcher
                    .find_mapped(&own, &prepared, &operation)
                    .expect("own search")
            ),
            vec![(0, 0, 0.9)]
        );
    }
}

#[test]
fn direct_formats_keep_admission_authority_on_cold_and_reusable_inputs() {
    for format in [PixelFormat::Rgba8, PixelFormat::Bgra8] {
        for reusable in [false, true] {
            let matcher = Matcher::new(backend(
                ControlledMatcher::new(PixelFormat::Bgra8)
                    .with_alternate_format(PixelFormat::Rgba8)
                    .with_candidates(vec![Candidate::new(0, 0, 0.9)]),
            ));
            let operation = OperationContext::new();
            let source = template("authority", PixelExtent::new(8, 8));
            let prepared = matcher.prepare(&source, &operation).expect("prepares");
            let image = frame(PixelExtent::new(64, 64), format, 0);
            let request =
                MatchRequest::new(&image, RegionSelection::FullFrame, &prepared, options());
            let mapped = matcher
                .map_match(&request, &operation)
                .expect("lawful mapping");
            if reusable {
                matcher
                    .find_mapped(&mapped, &prepared, &operation)
                    .expect("warm input");
            }
            let cancelled = CancellationToken::new();
            cancelled.cancel();
            let contexts = [
                (
                    OperationContext::new().with_cancellation(cancelled),
                    Status::Cancelled,
                ),
                (
                    OperationContext::new()
                        .with_clock(Arc::new(ManualClock::new()))
                        .with_deadline(MonotonicInstant::ORIGIN),
                    Status::DeadlineExceeded,
                ),
            ];
            for (ended, status) in contexts {
                assert_eq!(
                    matcher
                        .prepare(&source, &ended)
                        .expect_err("no preparation admission")
                        .status(),
                    status
                );
                assert_eq!(
                    matcher
                        .map_match(&request, &ended)
                        .expect_err("no mapping admission")
                        .status(),
                    status
                );
                assert_eq!(
                    matcher
                        .find_mapped(&mapped, &prepared, &ended)
                        .expect_err("no result admission")
                        .status(),
                    status
                );
            }
            assert_eq!(
                reported(
                    &matcher
                        .find_mapped(&mapped, &prepared, &operation)
                        .expect("independent authority")
                ),
                vec![(0, 0, 0.9)]
            );
        }
    }
}

#[test]
fn interrupted_direct_search_does_not_cancel_an_independent_mapped_search() {
    for format in [PixelFormat::Rgba8, PixelFormat::Bgra8] {
        for reusable in [false, true] {
            for expires in [false, true] {
                let gate = Arc::new(CompletionGate::new());
                let found = vec![Candidate::new(0, 0, 0.9)];
                let calls = reusable
                    .then(|| ScriptedMatchCall::new(found.clone()))
                    .into_iter()
                    .chain([
                        ScriptedMatchCall::new(found.clone()).with_completion_gate(gate.clone())
                    ]);
                let matcher = Matcher::new(backend(
                    ControlledMatcher::new(PixelFormat::Bgra8)
                        .with_alternate_format(PixelFormat::Rgba8)
                        .with_calls(calls)
                        .with_candidates(found),
                ));
                let operation = OperationContext::new();
                let prepared = matcher
                    .prepare(&template("independent", PixelExtent::new(8, 8)), &operation)
                    .expect("prepares");
                let image = frame(PixelExtent::new(64, 64), format, 0);
                let request =
                    MatchRequest::new(&image, RegionSelection::FullFrame, &prepared, options());
                let mapped = matcher
                    .map_match(&request, &operation)
                    .expect("lawful mapping");
                if reusable {
                    matcher
                        .find_mapped(&mapped, &prepared, &operation)
                        .expect("warm input");
                }
                let token = CancellationToken::new();
                let clock = Arc::new(ManualClock::new());
                let interrupted = OperationContext::new()
                    .with_clock(clock.clone())
                    .with_deadline(MonotonicInstant::from_origin(Duration::from_secs(1)))
                    .with_cancellation(token.clone());
                std::thread::scope(|scope| {
                    let _release = gate.release_guard();
                    let worker =
                        scope.spawn(|| matcher.find_mapped(&mapped, &prepared, &interrupted));
                    assert!(gate.wait_until_entered(Duration::from_secs(2)));
                    let expected = if expires {
                        clock.advance(Duration::from_secs(1));
                        Status::DeadlineExceeded
                    } else {
                        token.cancel();
                        Status::Cancelled
                    };
                    let independent = matcher
                        .find_mapped(&mapped, &prepared, &operation)
                        .expect("another request retains authority over the same immutable input");
                    assert_eq!(reported(&independent), vec![(0, 0, 0.9)]);
                    gate.release();
                    assert_eq!(
                        worker
                            .join()
                            .expect("worker")
                            .expect_err("late result")
                            .status(),
                        expected
                    );
                    assert_eq!(reported(&independent), vec![(0, 0, 0.9)]);
                });
            }
        }
    }
}

#[derive(Debug)]
struct CommitInterruptionClock {
    reads: AtomicUsize,
    cancellation: Option<CancellationToken>,
}

impl mado_pilot_core::Clock for CommitInterruptionClock {
    fn now(&self) -> MonotonicInstant {
        // find_mapped samples admission, its post-backend checkpoint, then commit.
        let read = self.reads.fetch_add(1, Ordering::AcqRel);
        if read == 1
            && let Some(token) = &self.cancellation
        {
            token.cancel();
        }
        MonotonicInstant::from_origin(if read < 2 {
            Duration::ZERO
        } else {
            Duration::from_secs(1)
        })
    }
}

#[test]
fn direct_mapped_results_require_authority_at_final_commit() {
    for format in [PixelFormat::Rgba8, PixelFormat::Bgra8] {
        for reusable in [false, true] {
            let matcher = Matcher::new(backend(
                ControlledMatcher::new(PixelFormat::Bgra8)
                    .with_alternate_format(PixelFormat::Rgba8)
                    .with_candidates(vec![Candidate::new(0, 0, 0.9)]),
            ));
            let operation = OperationContext::new();
            let prepared = matcher
                .prepare(&template("commit", PixelExtent::new(8, 8)), &operation)
                .expect("prepares");
            let image = frame(PixelExtent::new(64, 64), format, 0);
            let request =
                MatchRequest::new(&image, RegionSelection::FullFrame, &prepared, options());
            let mapped = matcher.map_match(&request, &operation).expect("mapped");
            if reusable {
                matcher
                    .find_mapped(&mapped, &prepared, &operation)
                    .expect("warm input");
            }
            for cancels in [false, true] {
                let token = CancellationToken::new();
                let interrupted = OperationContext::new()
                    .with_clock(Arc::new(CommitInterruptionClock {
                        reads: AtomicUsize::new(0),
                        cancellation: cancels.then(|| token.clone()),
                    }))
                    .with_deadline(MonotonicInstant::from_origin(Duration::from_secs(1)))
                    .with_cancellation(token);
                assert_eq!(
                    matcher
                        .find_mapped(&mapped, &prepared, &interrupted)
                        .expect_err("final commit authority ended")
                        .status(),
                    if cancels {
                        Status::Cancelled
                    } else {
                        Status::DeadlineExceeded
                    }
                );
            }
            assert_eq!(
                reported(
                    &matcher
                        .find_mapped(&mapped, &prepared, &operation)
                        .expect("still reusable")
                ),
                vec![(0, 0, 0.9)]
            );
        }
    }
}

#[test]
fn candidates_are_reported_in_canonical_order_whatever_order_they_arrive_in() {
    let result = search_full_frame(
        vec![
            Candidate::new(40, 40, 0.6),
            Candidate::new(0, 0, 0.95),
            Candidate::new(20, 20, 0.8),
        ],
        options(),
    )
    .expect("searches");

    assert_eq!(
        reported(&result),
        vec![(0, 0, 0.95), (20, 20, 0.8), (40, 40, 0.6)]
    );
}

#[test]
fn equal_scores_are_broken_by_geometry_rather_than_by_arrival_order() {
    let forwards = search_full_frame(
        vec![
            Candidate::new(30, 10, 0.8),
            Candidate::new(10, 10, 0.8),
            Candidate::new(20, 0, 0.8),
        ],
        options(),
    )
    .expect("searches");
    let backwards = search_full_frame(
        vec![
            Candidate::new(20, 0, 0.8),
            Candidate::new(10, 10, 0.8),
            Candidate::new(30, 10, 0.8),
        ],
        options(),
    )
    .expect("searches");

    assert_eq!(
        reported(&forwards),
        vec![(20, 0, 0.8), (10, 10, 0.8), (30, 10, 0.8)]
    );
    assert_eq!(reported(&forwards), reported(&backwards));
}

#[test]
fn a_candidate_below_the_threshold_is_not_reported() {
    let result = search_full_frame(
        vec![Candidate::new(0, 0, 0.49), Candidate::new(20, 20, 0.5)],
        options(),
    )
    .expect("searches");

    assert_eq!(
        reported(&result),
        vec![(20, 20, 0.5)],
        "the threshold is inclusive at its own value"
    );
}

#[test]
fn a_search_where_nothing_reaches_the_threshold_succeeds_with_no_matches() {
    let result = search_full_frame(
        vec![Candidate::new(0, 0, 0.1), Candidate::new(20, 20, 0.2)],
        options(),
    )
    .expect("a search that finds nothing is not a failure");

    assert!(result.is_empty());
    assert_eq!(result.best(), None);
}

#[test]
fn overlapping_candidates_are_suppressed_and_the_canonical_first_wins() {
    // Both 8x8 templates at (0,0) and (4,4) overlap. The higher score wins.
    let result = search_full_frame(
        vec![Candidate::new(0, 0, 0.7), Candidate::new(4, 4, 0.9)],
        options(),
    )
    .expect("searches");

    assert_eq!(reported(&result), vec![(4, 4, 0.9)]);
}

#[test]
fn equal_score_overlaps_pick_the_same_winner_every_time() {
    for _ in 0..4 {
        let result = search_full_frame(
            vec![Candidate::new(4, 4, 0.8), Candidate::new(0, 0, 0.8)],
            options(),
        )
        .expect("searches");

        assert_eq!(
            reported(&result),
            vec![(0, 0, 0.8)],
            "the canonically first candidate wins suppression"
        );
    }
}

#[test]
fn adjacent_candidates_that_only_touch_are_both_reported() {
    // Half-open bounds: 0..8 and 8..16 share an edge and no pixel.
    let result = search_full_frame(
        vec![Candidate::new(0, 0, 0.9), Candidate::new(8, 0, 0.9)],
        options(),
    )
    .expect("searches");

    assert_eq!(reported(&result), vec![(0, 0, 0.9), (8, 0, 0.9)]);
}

#[test]
fn keeping_every_candidate_is_available_when_a_caller_asks_for_it() {
    let result = search_full_frame(
        vec![Candidate::new(0, 0, 0.7), Candidate::new(4, 4, 0.9)],
        options().with_suppression(Suppression::KeepAll),
    )
    .expect("searches");

    assert_eq!(reported(&result), vec![(4, 4, 0.9), (0, 0, 0.7)]);
}

#[test]
fn the_result_limit_applies_after_ordering_and_suppression() {
    let result = search_full_frame(
        vec![
            Candidate::new(0, 0, 0.6),
            Candidate::new(16, 0, 0.9),
            Candidate::new(32, 0, 0.7),
        ],
        options().with_max_results(2).expect("valid"),
    )
    .expect("searches");

    assert_eq!(
        reported(&result),
        vec![(16, 0, 0.9), (32, 0, 0.7)],
        "the limit takes the canonical first results, it does not reorder them"
    );
}

#[test]
fn a_roi_reports_its_matches_in_full_frame_coordinates() {
    let matcher = Matcher::new(backend(
        ControlledMatcher::new(FORMAT).with_candidates(vec![Candidate::new(2, 3, 0.9)]),
    ));
    let prepared = matcher
        .prepare(
            &template("t", PixelExtent::new(8, 8)),
            &OperationContext::new(),
        )
        .expect("prepares");
    let image = frame(PixelExtent::new(64, 64), FORMAT, 0);
    let roi = Rect::new(CoordinateSpace::CapturePixels, 16.0, 24.0, 48.0, 56.0).expect("valid");

    let result = matcher
        .find(
            MatchRequest::new(
                &image,
                RegionSelection::Region {
                    rect: roi,
                    policy: ClipPolicy::Reject,
                },
                &prepared,
                options(),
            ),
            &OperationContext::new(),
        )
        .expect("searches");

    assert_eq!(
        result.searched(),
        PixelRect::new(16, 24, 48, 56).expect("valid")
    );
    assert_eq!(
        reported(&result),
        vec![(18, 27, 0.9)],
        "an ROI-local candidate is translated by the region's origin"
    );
}

#[test]
fn a_backend_score_outside_the_public_range_is_a_backend_failure() {
    for score in [1.5, -0.1, f64::NAN, f64::INFINITY] {
        let error = search_full_frame(vec![Candidate::new(0, 0, score)], options())
            .expect_err("the public score range is the contract");

        assert_eq!(error.status(), Status::VisionFailed, "score {score}");
    }
}

#[test]
fn a_candidate_outside_the_searched_region_is_a_backend_failure() {
    let error = search_full_frame(vec![Candidate::new(60, 0, 0.9)], options())
        .expect_err("an 8x8 match at x=60 leaves a 64-wide frame");

    assert_eq!(error.status(), Status::VisionFailed);
}

#[test]
fn an_unavailable_backend_is_reported_without_a_fallback() {
    let matcher = Matcher::new(backend(
        ControlledMatcher::new(FORMAT).preparing(Behavior::Unavailable),
    ));

    let error = matcher
        .prepare(
            &template("t", PixelExtent::new(8, 8)),
            &OperationContext::new(),
        )
        .expect_err("an unavailable backend cannot prepare");

    assert_eq!(error.status(), Status::VisionFailed);
}

#[test]
fn a_backend_that_fails_mid_search_reports_a_vision_failure() {
    let matcher = Matcher::new(backend(
        ControlledMatcher::new(FORMAT).finding(Behavior::Fail),
    ));
    let prepared = matcher
        .prepare(
            &template("t", PixelExtent::new(8, 8)),
            &OperationContext::new(),
        )
        .expect("prepares");
    let image = frame(PixelExtent::new(64, 64), FORMAT, 0);

    let error = matcher
        .find(
            MatchRequest::new(&image, RegionSelection::FullFrame, &prepared, options()),
            &OperationContext::new(),
        )
        .expect_err("the backend failed");

    assert_eq!(error.status(), Status::VisionFailed);
}

#[test]
fn a_deadline_that_expires_inside_the_backend_discards_its_answer() {
    let clock = Arc::new(ManualClock::new());
    let double = ControlledMatcher::new(FORMAT)
        .with_candidates(vec![Candidate::new(0, 0, 0.99)])
        .with_latency(Arc::clone(&clock), Duration::from_millis(50));
    let matcher = Matcher::new(backend(double));

    let context = OperationContext::new()
        .with_clock(Arc::clone(&clock) as Arc<dyn mado_pilot_core::Clock>)
        .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(10)));
    let prepared = matcher
        .prepare(&template("t", PixelExtent::new(8, 8)), &context)
        .expect("prepares before the deadline");
    let image = frame(PixelExtent::new(64, 64), FORMAT, 0);

    let error = matcher
        .find(
            MatchRequest::new(&image, RegionSelection::FullFrame, &prepared, options()),
            &context,
        )
        .expect_err("the backend finished, and finished too late");

    assert_eq!(error.status(), Status::DeadlineExceeded);
}

#[test]
fn a_backend_that_finishes_after_cancellation_cannot_publish_its_answer() {
    let token = CancellationToken::new();
    let double = ControlledMatcher::new(FORMAT)
        .with_candidates(vec![Candidate::new(0, 0, 0.99)])
        .cancelling(token.clone());
    let matcher = Matcher::new(backend(double));

    let prepared = matcher
        .prepare(
            &template("t", PixelExtent::new(8, 8)),
            &OperationContext::new(),
        )
        .expect("prepares");
    let image = frame(PixelExtent::new(64, 64), FORMAT, 0);
    let context = OperationContext::new().with_cancellation(token);

    let error = matcher
        .find(
            MatchRequest::new(&image, RegionSelection::FullFrame, &prepared, options()),
            &context,
        )
        .expect_err("a late answer is discarded, not returned");

    assert_eq!(error.status(), Status::Cancelled);
}

#[test]
fn an_already_cancelled_search_never_reaches_the_backend() {
    let double = Arc::new(ControlledMatcher::new(FORMAT));
    let matcher = Matcher::new(Arc::clone(&double) as Arc<dyn MatchBackend>);
    let prepared = matcher
        .prepare(
            &template("t", PixelExtent::new(8, 8)),
            &OperationContext::new(),
        )
        .expect("prepares");
    let image = frame(PixelExtent::new(64, 64), FORMAT, 0);

    let token = CancellationToken::new();
    token.cancel();
    let context = OperationContext::new().with_cancellation(token);

    assert!(
        matcher
            .find(
                MatchRequest::new(&image, RegionSelection::FullFrame, &prepared, options()),
                &context,
            )
            .is_err()
    );
    assert_eq!(
        double.find_count(),
        0,
        "the refusal must happen before any backend work"
    );
}

#[test]
fn a_template_larger_than_the_region_never_reaches_the_backend() {
    let double = Arc::new(ControlledMatcher::new(FORMAT));
    let matcher = Matcher::new(Arc::clone(&double) as Arc<dyn MatchBackend>);
    let prepared = matcher
        .prepare(
            &template("large", PixelExtent::new(128, 128)),
            &OperationContext::new(),
        )
        .expect("prepares");
    let image = frame(PixelExtent::new(64, 64), FORMAT, 0);

    let result = matcher
        .find(
            MatchRequest::new(&image, RegionSelection::FullFrame, &prepared, options()),
            &OperationContext::new(),
        )
        .expect("succeeds with nothing");

    assert!(result.is_empty());
    assert_eq!(
        double.find_count(),
        0,
        "undefined backend behavior is avoided by not asking"
    );
}

#[test]
fn a_prepared_template_is_reusable_across_searches() {
    let double =
        Arc::new(ControlledMatcher::new(FORMAT).with_candidates(vec![Candidate::new(0, 0, 0.9)]));
    let matcher = Matcher::new(Arc::clone(&double) as Arc<dyn MatchBackend>);
    let prepared = matcher
        .prepare(
            &template("t", PixelExtent::new(8, 8)),
            &OperationContext::new(),
        )
        .expect("prepares");
    let image = frame(PixelExtent::new(64, 64), FORMAT, 0);

    for _ in 0..3 {
        let result = matcher
            .find(
                MatchRequest::new(&image, RegionSelection::FullFrame, &prepared, options()),
                &OperationContext::new(),
            )
            .expect("searches");
        assert_eq!(reported(&result), vec![(0, 0, 0.9)]);
    }

    assert_eq!(double.prepare_count(), 1, "preparation is paid once");
    assert_eq!(double.find_count(), 3);
}

#[test]
fn a_result_outlives_every_handle_it_came_from() {
    let result = {
        let matcher = Matcher::new(backend(
            ControlledMatcher::new(FORMAT).with_candidates(vec![Candidate::new(8, 8, 0.9)]),
        ));
        let prepared = matcher
            .prepare(
                &template("t", PixelExtent::new(8, 8)),
                &OperationContext::new(),
            )
            .expect("prepares");
        let image = frame(PixelExtent::new(64, 64), FORMAT, 0);
        matcher
            .find(
                MatchRequest::new(&image, RegionSelection::FullFrame, &prepared, options()),
                &OperationContext::new(),
            )
            .expect("searches")
    };

    assert_eq!(reported(&result), vec![(8, 8, 0.9)]);
    assert_eq!(result.backend().id(), CONTROLLED_BACKEND);
    assert_eq!(
        result.matches()[0].template().as_str(),
        "t",
        "reading a result repeatedly returns the same immutable values"
    );
    assert_eq!(reported(&result), vec![(8, 8, 0.9)]);
}

#[test]
fn a_view_selects_its_own_region() {
    let matcher = Matcher::new(backend(
        ControlledMatcher::new(FORMAT).with_candidates(vec![Candidate::new(0, 0, 0.9)]),
    ));
    let prepared = matcher
        .prepare(
            &template("t", PixelExtent::new(8, 8)),
            &OperationContext::new(),
        )
        .expect("prepares");
    let image = frame(PixelExtent::new(64, 64), FORMAT, 0);
    let view = image
        .view(
            Rect::new(CoordinateSpace::CapturePixels, 32.0, 16.0, 64.0, 48.0).expect("valid"),
            ClipPolicy::Reject,
        )
        .expect("a valid view");

    let request = MatchRequest::from_view(&view, &prepared, options()).expect("valid");
    let result = matcher
        .find(request, &OperationContext::new())
        .expect("searches");

    assert_eq!(
        result.searched(),
        PixelRect::new(32, 16, 64, 48).expect("valid")
    );
    assert_eq!(reported(&result), vec![(32, 16, 0.9)]);
}
