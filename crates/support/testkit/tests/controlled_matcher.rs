//! The controlled matcher against the vision contract, and the public rules
//! against a backend whose answers are known in advance.
//!
//! A scripted backend is the only way to test the rules that sit between a
//! backend and a caller. A real matcher cannot be asked for two candidates with
//! exactly equal scores at chosen coordinates, or for a score outside the public
//! range, or to finish only after a deadline has passed — and those are the
//! cases where the rules either hold or quietly do not.

use std::sync::Arc;
use std::time::Duration;

use mado_pilot_capture::PixelFormat;
use mado_pilot_core::{
    CancellationToken, ClipPolicy, CoordinateSpace, MonotonicInstant, OperationContext,
    PixelExtent, PixelRect, Rect, Status,
};
use mado_pilot_testkit::ManualClock;
use mado_pilot_testkit::controlled_matcher::{Behavior, CONTROLLED_BACKEND, ControlledMatcher};
use mado_pilot_testkit::vision_contract::{self, frame, template};
use mado_pilot_vision::{
    Candidate, MatchBackend, MatchDefaults, MatchOptions, MatchRequest, Matcher, RegionSelection,
    Suppression,
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
