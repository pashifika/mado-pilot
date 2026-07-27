//! What the OpenCV adapter finds, and where.
//!
//! The shared vision contract suite covers the rules that hold whatever an image
//! contains. These fixtures cover the part it deliberately cannot: a real image
//! with a known answer, searched by a real backend. The scene and both templates
//! come from `mado-pilot-testkit`'s fixture generator, so the expectations below
//! are checked against how the inputs were built rather than against a recorded
//! file.

use std::sync::Arc;

use mado_pilot_capture::PixelFormat;
use mado_pilot_core::{
    ClipPolicy, CoordinateSpace, OperationContext, PixelExtent, PixelRect, Rect, Status,
};
use mado_pilot_testkit::match_fixtures::{
    DEGRADED, PATCH, PLANTED, SCENE, absent_template, planted_template,
};
use mado_pilot_testkit::{match_fixtures, png, vision_contract};
use mado_pilot_vision::{
    MatchBackend, MatchDefaults, MatchOptions, MatchRequest, MatchResult, Matcher, RegionSelection,
    TemplateEncoding, TemplateId, TemplateSource, TemplateSourceRequest, VisionFault,
};

use mado_pilot_backend_opencv::OpenCvBackend;

/// The score an exactly aligned copy correlates at.
///
/// One, by the definition of a normalized correlation coefficient — not a
/// measurement to be taken on faith.
const EXACT_MATCH_SCORE: f64 = 1.0;

/// How far a public score may sit from its reference value.
///
/// OpenCV does not correlate offset by offset: it normalizes through integral
/// images and computes the numerator over the whole region at once, so a score
/// carries rounding from arithmetic that involved the rest of the scene. Two
/// byte-identical copies of one patch at two offsets in one image were observed
/// to score `1.0` and `1.0 - 3.6e-7` depending on what else the scene contained.
/// The tolerance is therefore a property of the algorithm rather than of the
/// hosts, and it is what these fixtures compare against instead of an exact
/// value. Observed values are recorded in `docs/evidence/vision-opencv/`.
const SCORE_TOLERANCE: f64 = 1e-5;

/// The score of the half-strength copy, which orders below an exact one.
const DEGRADED_MATCH_SCORE: f64 = 0.756_341;

/// The ceiling the absent template's best offset must stay under.
///
/// The measured value is far lower; this is the level at which the "no match"
/// fixtures stop being evidence, because a threshold of `0.9` would no longer
/// have a real margin under it.
const ABSENT_SCORE_CEILING: f64 = 0.3;

/// A wider tolerance for the degraded copy's score.
///
/// An exact alignment scores one because that is what a normalized correlation
/// coefficient means, so only rounding can move it and the tight tolerance
/// applies. The degraded copy's score is a *measurement* of one OpenCV build:
/// another 4.x build may compute it slightly differently without any of it being
/// wrong. Its band is therefore wide enough to survive a patch-version change and
/// narrow enough that a profile change — a different method, a different colour
/// conversion, a different clamp — still fails here. The precise measured values
/// live in `docs/evidence/vision-opencv/`.
const DEGRADED_TOLERANCE: f64 = 1e-3;

fn matcher() -> Matcher {
    Matcher::new(Arc::new(
        OpenCvBackend::new().expect("the development OpenCV installation is usable"),
    ) as Arc<dyn MatchBackend>)
}

fn options(min_score: f64) -> MatchOptions {
    MatchOptions::from_defaults(MatchDefaults::new(min_score, 8).expect("valid defaults"))
}

/// Searches the whole scene, in `format`, for the planted patch.
fn search_scene(format: PixelFormat, min_score: f64) -> MatchResult {
    search(format, RegionSelection::FullFrame, min_score)
}

fn search(format: PixelFormat, selection: RegionSelection, min_score: f64) -> MatchResult {
    let matcher = matcher();
    let frame = match_fixtures::scene_frame(format);
    let prepared = matcher
        .prepare(&planted_template("patch"), &OperationContext::new())
        .expect("the patch prepares");

    matcher
        .find(
            MatchRequest::new(&frame, selection, &prepared, options(min_score)),
            &OperationContext::new(),
        )
        .expect("a well-formed search")
}

/// The half-open capture-pixel rectangle a planted copy occupies.
fn planted_bounds(index: usize) -> PixelRect {
    let (left, top) = PLANTED[index];
    let left = i32::try_from(left).expect("small");
    let top = i32::try_from(top).expect("small");

    PixelRect::new(
        left,
        top,
        left + i32::try_from(PATCH.width()).expect("small"),
        top + i32::try_from(PATCH.height()).expect("small"),
    )
    .expect("a valid rectangle")
}

/// The rectangle the half-strength copy occupies.
fn degraded_bounds() -> PixelRect {
    let (left, top) = DEGRADED;
    let left = i32::try_from(left).expect("small");
    let top = i32::try_from(top).expect("small");

    PixelRect::new(
        left,
        top,
        left + i32::try_from(PATCH.width()).expect("small"),
        top + i32::try_from(PATCH.height()).expect("small"),
    )
    .expect("a valid rectangle")
}

/// A result's match bounds, sorted geometrically so a comparison does not depend
/// on an ordering the tolerance cannot guarantee.
fn bounds_of(result: &MatchResult) -> Vec<PixelRect> {
    let mut bounds: Vec<PixelRect> = result
        .matches()
        .iter()
        .map(|found| found.bounds())
        .collect();
    bounds.sort_by_key(|rectangle| (rectangle.top(), rectangle.left()));

    bounds
}

fn rect(left: f64, top: f64, right: f64, bottom: f64) -> Rect {
    Rect::new(CoordinateSpace::CapturePixels, left, top, right, bottom).expect("a valid rectangle")
}

#[test]
fn every_planted_copy_is_found_at_its_exact_offset() {
    let result = search_scene(PixelFormat::Rgba8, 0.9);

    assert_eq!(
        result.matches().len(),
        PLANTED.len(),
        "one planted copy is one match, not a hill of overlapping offsets"
    );
    // Compared as a set. Both copies correlate at one to within the tolerance, so
    // which of the two the canonical order puts first rests on a difference
    // smaller than the tolerance, and asserting it would be asserting rounding.
    assert_eq!(
        bounds_of(&result),
        vec![planted_bounds(0), planted_bounds(1)]
    );
    for found in result.matches() {
        assert_eq!(found.template().as_str(), "patch");
        assert!(
            (found.score() - EXACT_MATCH_SCORE).abs() <= SCORE_TOLERANCE,
            "an exact alignment correlates at one, within tolerance: {}",
            found.score()
        );
    }
}

#[test]
fn a_weaker_copy_is_ordered_after_the_exact_ones() {
    let result = search_scene(PixelFormat::Rgba8, 0.5);
    let matches = result.matches();

    assert_eq!(matches.len(), 3, "the lower threshold admits the weak copy");
    let weakest = &matches[2];
    assert_eq!(
        weakest.bounds(),
        degraded_bounds(),
        "descending score puts the half-strength copy last"
    );
    assert!(
        (weakest.score() - DEGRADED_MATCH_SCORE).abs() <= DEGRADED_TOLERANCE,
        "the weak copy scored {}",
        weakest.score()
    );
    assert!(
        matches[1].score() - weakest.score() > SCORE_TOLERANCE,
        "the ordering must rest on a real difference, not on rounding"
    );
    assert_eq!(
        bounds_of(&result)[..2],
        [planted_bounds(0), planted_bounds(1)],
        "the two exact copies are still both found"
    );
}

#[test]
fn the_two_supported_frame_formats_reach_the_same_answer() {
    let rgba = search_scene(PixelFormat::Rgba8, 0.9);
    let bgra = search_scene(PixelFormat::Bgra8, 0.9);

    assert_eq!(bounds_of(&rgba), bounds_of(&bgra));
    for (left, right) in rgba.matches().iter().zip(bgra.matches()) {
        assert_eq!(
            left.score(),
            right.score(),
            "a channel swap in the mapping must not move a score"
        );
    }
}

#[test]
fn a_patch_the_scene_does_not_contain_is_a_successful_empty_result() {
    let matcher = matcher();
    let frame = match_fixtures::scene_frame(PixelFormat::Rgba8);
    let prepared = matcher
        .prepare(&absent_template("absent"), &OperationContext::new())
        .expect("the absent patch prepares");

    let result = matcher
        .find(
            MatchRequest::new(&frame, RegionSelection::FullFrame, &prepared, options(0.9)),
            &OperationContext::new(),
        )
        .expect("a well-formed question with the answer 'not there'");

    assert!(result.is_empty());
    assert_eq!(result.stamp(), frame.stamp());
}

#[test]
fn an_absent_patch_stays_well_below_the_fixture_threshold() {
    let matcher = matcher();
    let frame = match_fixtures::scene_frame(PixelFormat::Rgba8);
    let prepared = matcher
        .prepare(&absent_template("absent"), &OperationContext::new())
        .expect("prepares");

    // A zero threshold reports the best offset there is, which is what makes the
    // margin measurable rather than assumed.
    let result = matcher
        .find(
            MatchRequest::new(
                &frame,
                RegionSelection::FullFrame,
                &prepared,
                options(0.0).with_max_results(1).expect("valid"),
            ),
            &OperationContext::new(),
        )
        .expect("searches");

    let best = result.matches()[0].score();
    assert!(
        best <= ABSENT_SCORE_CEILING,
        "the absent patch's best correlation was {best}, leaving no margin below the threshold"
    );
}

#[test]
fn a_region_of_interest_reports_full_frame_coordinates() {
    let expected = planted_bounds(1);
    let result = search(
        PixelFormat::Rgba8,
        RegionSelection::Region {
            rect: rect(55.0, 35.0, 80.0, 60.0),
            policy: ClipPolicy::Reject,
        },
        0.9,
    );

    assert_eq!(
        result.matches().len(),
        1,
        "only the copy inside the region is searched for"
    );
    assert_eq!(
        result.matches()[0].bounds(),
        expected,
        "a match is reported against the whole frame, never against the region"
    );
    assert_eq!(
        result.searched(),
        PixelRect::new(55, 35, 80, 60).expect("valid")
    );
}

#[test]
fn a_clipped_region_reports_full_frame_coordinates() {
    let expected = planted_bounds(1);
    let result = search(
        PixelFormat::Rgba8,
        RegionSelection::Region {
            rect: rect(55.0, 35.0, 200.0, 200.0),
            policy: ClipPolicy::Clip,
        },
        0.9,
    );

    assert_eq!(
        result.searched(),
        PixelRect::new(
            55,
            35,
            i32::try_from(SCENE.width()).expect("small"),
            i32::try_from(SCENE.height()).expect("small")
        )
        .expect("valid"),
        "the region is clipped to the frame's own extent"
    );
    assert_eq!(result.matches().len(), 1);
    assert_eq!(result.matches()[0].bounds(), expected);
}

#[test]
fn a_template_larger_than_the_region_is_a_successful_empty_result() {
    let matcher = matcher();
    let frame = match_fixtures::scene_frame(PixelFormat::Rgba8);
    let prepared = matcher
        .prepare(
            &match_fixtures::oversized_template("wide", PixelExtent::new(120, 80)),
            &OperationContext::new(),
        )
        .expect("an oversized template still prepares");

    let result = matcher
        .find(
            MatchRequest::new(&frame, RegionSelection::FullFrame, &prepared, options(0.5)),
            &OperationContext::new(),
        )
        .expect("a well-formed search of a region too small to hold the template");

    assert!(result.is_empty());
    assert_eq!(result.stamp(), frame.stamp());
}

#[test]
fn a_malformed_option_is_refused_before_the_backend_runs() {
    assert_eq!(
        options(0.9).with_min_score(1.5),
        Err(VisionFault::InvalidMatchScore)
    );
    assert_eq!(
        options(0.9).with_max_results(0),
        Err(VisionFault::InvalidMatchResultLimit)
    );
}

#[test]
fn content_that_does_not_decode_fails_preparation() {
    let source = TemplateSource::new(TemplateSourceRequest {
        id: TemplateId::new("truncated").expect("non-empty"),
        encoding: TemplateEncoding::Png,
        extent: PATCH,
        space: CoordinateSpace::CapturePixels,
        defaults: MatchDefaults::new(0.9, 8).expect("valid"),
        // A well-formed signature with no image behind it, which is exactly what
        // a caller who trusted a file extension would hand over.
        content: Arc::from([0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'].as_slice()),
    })
    .expect("a structurally valid source");

    let error = matcher()
        .prepare(&source, &OperationContext::new())
        .expect_err("content that is not an image does not prepare");

    assert_eq!(error.status(), Status::VisionFailed);
}

#[test]
fn a_template_that_decodes_to_another_extent_fails_preparation() {
    let source = TemplateSource::new(TemplateSourceRequest {
        id: TemplateId::new("mislabelled").expect("non-empty"),
        encoding: TemplateEncoding::Png,
        // The metadata claims the patch's extent; the bytes are one pixel.
        extent: PATCH,
        space: CoordinateSpace::CapturePixels,
        defaults: MatchDefaults::new(0.9, 8).expect("valid"),
        content: Arc::from(png::solid_rgb(1, 1, [0, 0, 0]).as_slice()),
    })
    .expect("a structurally valid source");

    let error = matcher()
        .prepare(&source, &OperationContext::new())
        .expect_err("a template must not match at an extent it does not have");

    assert_eq!(error.status(), Status::VisionFailed);
}

#[test]
fn a_result_keeps_its_own_source_after_a_later_frame_exists() {
    let matcher = matcher();
    let frame = match_fixtures::scene_frame(PixelFormat::Rgba8);
    let expected_stamp = frame.stamp();
    let expected_transform = *frame.transform();
    let prepared = matcher
        .prepare(&planted_template("patch"), &OperationContext::new())
        .expect("prepares");

    let result = matcher
        .find(
            MatchRequest::new(&frame, RegionSelection::FullFrame, &prepared, options(0.9)),
            &OperationContext::new(),
        )
        .expect("searches");

    // A newer frame with a different extent, published after the search.
    let later = vision_contract::frame(PixelExtent::new(32, 32), PixelFormat::Rgba8, 0x7f);
    assert_ne!(later.stamp(), expected_stamp);

    drop(frame);
    assert_eq!(result.stamp(), expected_stamp);
    assert_eq!(result.transform(), &expected_transform);
    assert_eq!(
        bounds_of(&result),
        vec![planted_bounds(0), planted_bounds(1)]
    );
}
