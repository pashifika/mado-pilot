//! What the OpenCV adapter finds, and where.
//!
//! The shared vision contract suite covers the rules that hold whatever an image
//! contains. These fixtures cover the part it deliberately cannot: a real image
//! with a known answer, searched by a real backend. The scene and both templates
//! come from `mado-pilot-testkit`'s fixture generator, and the colour and
//! oversized fixtures from integer noise below, so the expectations are checked
//! against how the inputs were built rather than against a recorded file.

use std::sync::Arc;

use mado_pilot_capture::{CpuPixels, FrameDescriptor, FrameStorage, PixelFormat};
use mado_pilot_core::{
    ClipPolicy, CoordinateSpace, Error, GeometryFault, OperationContext, PixelExtent, PixelRect,
    Rect, Status,
};
use mado_pilot_testkit::match_fixtures::{
    DEGRADED, PATCH, PLANTED, SCENE, absent_template, planted_template,
};
use mado_pilot_testkit::{match_fixtures, png, vision_contract};
use mado_pilot_vision::{
    MatchBackend, MatchDefaults, MatchOptions, MatchRequest, MatchResult, Matcher, RegionSelection,
    Suppression, TemplateEncoding, TemplateId, TemplateSource, TemplateSourceRequest, VisionFault,
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

/// The threshold the result-limit fixture searches the scene at.
///
/// Fifteen offsets clear it: both exact copies, the degraded copy and twelve
/// neighbours of the exact copies, the lowest at 0.481. The highest excluded
/// offset scores 0.456, so the survivor counts below rest on margins above
/// 0.01 rather than on rounding. The eighth survivor scores 0.507 and the ninth
/// 0.503, which is what makes the first eight a set rather than a tie.
const LIMIT_THRESHOLD: f64 = 0.47;

/// Both layouts the adapter converts directly.
const LAYOUTS: [PixelFormat; 2] = [PixelFormat::Rgba8, PixelFormat::Bgra8];

/// Trailing bytes per row in the padded fixtures.
const PADDING: usize = 64;

/// The colour fixture's template extent.
const COLOUR_PATCH: PixelExtent = PixelExtent::new(12, 10);

/// Where the colour fixture plants its template in its own colours.
const TRUE_COLOURS: (u32, u32) = (4, 7);

/// Where the colour fixture plants the template with red and blue exchanged.
const SWAPPED_COLOURS: (u32, u32) = (30, 7);

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

/// Searches the whole scene for the planted patch, with `padding` trailing
/// bytes on every row.
fn search_padded_scene(format: PixelFormat, min_score: f64, padding: usize) -> MatchResult {
    search_frame(
        &match_fixtures::scene_frame_with_padded_rows(format, padding),
        RegionSelection::FullFrame,
        min_score,
    )
}

fn search(format: PixelFormat, selection: RegionSelection, min_score: f64) -> MatchResult {
    search_frame(&match_fixtures::scene_frame(format), selection, min_score)
}

fn search_frame(
    frame: &mado_pilot_capture::Frame,
    selection: RegionSelection,
    min_score: f64,
) -> MatchResult {
    find_in(
        frame,
        &planted_template("patch"),
        selection,
        options(min_score),
    )
}

/// Searches `frame` for `template` on a fresh backend.
fn find_in(
    frame: &mado_pilot_capture::Frame,
    template: &TemplateSource,
    selection: RegionSelection,
    options: MatchOptions,
) -> MatchResult {
    let matcher = matcher();
    let prepared = matcher
        .prepare(template, &OperationContext::new())
        .expect("the template prepares");

    matcher
        .find(
            MatchRequest::new(frame, selection, &prepared, options),
            &OperationContext::new(),
        )
        .expect("a well-formed search")
}

/// The scene in each accepted layout, packed and with padded rows.
fn scene_frames() -> Vec<mado_pilot_capture::Frame> {
    LAYOUTS
        .into_iter()
        .flat_map(|format| {
            [
                match_fixtures::scene_frame(format),
                match_fixtures::scene_frame_with_padded_rows(format, PADDING),
            ]
        })
        .collect()
}

/// The half-open capture-pixel rectangle of `extent` at `origin`.
fn bounds_at((left, top): (u32, u32), extent: PixelExtent) -> PixelRect {
    let left = i32::try_from(left).expect("small");
    let top = i32::try_from(top).expect("small");

    PixelRect::new(
        left,
        top,
        left + i32::try_from(extent.width()).expect("small"),
        top + i32::try_from(extent.height()).expect("small"),
    )
    .expect("a valid rectangle")
}

/// The half-open capture-pixel rectangle a planted copy occupies.
fn planted_bounds(index: usize) -> PixelRect {
    bounds_at(PLANTED[index], PATCH)
}

/// The rectangle the half-strength copy occupies.
fn degraded_bounds() -> PixelRect {
    bounds_at(DEGRADED, PATCH)
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

/// Asserts that the scene's exact copies score one and its degraded copy its
/// measured value, each within its own tolerance.
fn assert_scene_scores(result: &MatchResult) {
    for found in result.matches() {
        let (expected, tolerance) = if found.bounds() == degraded_bounds() {
            (DEGRADED_MATCH_SCORE, DEGRADED_TOLERANCE)
        } else {
            (EXACT_MATCH_SCORE, SCORE_TOLERANCE)
        };
        assert!(
            (found.score() - expected).abs() <= tolerance,
            "{:?} scored {}",
            found.bounds(),
            found.score()
        );
    }
}

/// Asserts that two results report the same bounds, with scores equal within
/// the exact-score tolerance rather than bit for bit.
fn assert_equivalent(left: &MatchResult, right: &MatchResult) {
    assert_eq!(bounds_of(left), bounds_of(right));
    for found in left.matches() {
        let other = right
            .matches()
            .iter()
            .find(|other| other.bounds() == found.bounds())
            .expect("the same bounds");
        assert!(
            (found.score() - other.score()).abs() <= SCORE_TOLERANCE,
            "{:?}: {} against {}",
            found.bounds(),
            found.score(),
            other.score()
        );
    }
}

/// Integer noise, identical on both release targets.
fn noise(x: u32, y: u32, salt: u32) -> [u8; 4] {
    let mut hash =
        x.wrapping_mul(0x9e37_79b1) ^ y.wrapping_mul(0x85eb_ca77) ^ salt.wrapping_mul(0xc2b2_ae3d);
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x85eb_ca6b);
    hash ^= hash >> 13;
    hash = hash.wrapping_mul(0xc2b2_ae35);
    (hash ^ (hash >> 16)).to_le_bytes()
}

/// Stores one RGB pixel in `format` with `alpha`.
fn four_channel(format: PixelFormat, [red, green, blue]: [u8; 3], alpha: u8) -> [u8; 4] {
    match format {
        PixelFormat::Rgba8 => [red, green, blue, alpha],
        PixelFormat::Bgra8 => [blue, green, red, alpha],
        _ => panic!("no fixture layout for {format}"),
    }
}

fn template_source(id: &str, extent: PixelExtent, content: &[u8]) -> TemplateSource {
    TemplateSource::new(TemplateSourceRequest {
        id: TemplateId::new(id).expect("non-empty"),
        encoding: TemplateEncoding::Png,
        extent,
        space: CoordinateSpace::CapturePixels,
        defaults: MatchDefaults::new(0.9, 8).expect("valid defaults"),
        content: Arc::from(content),
    })
    .expect("a valid template source")
}

/// A pixel whose blue channel inverts its red one, so exchanging the two
/// inverts the template's correlation instead of preserving it.
fn chroma(x: u32, y: u32) -> [u8; 3] {
    let [red, green, ..] = noise(x, y, 1);
    [red, green, 0xff - red]
}

fn exchange_red_and_blue([red, green, blue]: [u8; 3]) -> [u8; 3] {
    [blue, green, red]
}

/// A noise scene in `format` holding the colour template at [`TRUE_COLOURS`]
/// and its red/blue exchange at [`SWAPPED_COLOURS`], with both templates.
///
/// Each template correlates at one with its own copy and at about -0.33 with
/// the other, which the public score clamps to zero; nothing else in the scene
/// reaches 0.2. A conversion that swapped red and blue would therefore find
/// each template at the other's origin instead.
fn colour_fixture(
    format: PixelFormat,
) -> (mado_pilot_capture::Frame, TemplateSource, TemplateSource) {
    let extent = PixelExtent::new(48, 24);
    let planted = |(left, top): (u32, u32), x: u32, y: u32| {
        (x >= left
            && y >= top
            && x < left + COLOUR_PATCH.width()
            && y < top + COLOUR_PATCH.height())
        .then(|| chroma(x - left, y - top))
    };
    let mut pixels = Vec::new();
    for y in 0..extent.height() {
        for x in 0..extent.width() {
            let [red, green, blue, _] = noise(x, y, 2);
            let rgb = planted(TRUE_COLOURS, x, y)
                .or_else(|| planted(SWAPPED_COLOURS, x, y).map(exchange_red_and_blue))
                .unwrap_or([red, green, blue]);
            pixels.extend_from_slice(&four_channel(format, rgb, 0xff));
        }
    }

    let mut colours = Vec::new();
    let mut exchanged = Vec::new();
    for y in 0..COLOUR_PATCH.height() {
        for x in 0..COLOUR_PATCH.width() {
            colours.extend_from_slice(&chroma(x, y));
            exchanged.extend_from_slice(&exchange_red_and_blue(chroma(x, y)));
        }
    }
    let encode = |rgb: &[u8]| png::encode_rgb(COLOUR_PATCH.width(), COLOUR_PATCH.height(), rgb);

    (
        vision_contract::frame_with_pixels(extent, format, pixels),
        template_source("colours", COLOUR_PATCH, &encode(&colours)),
        template_source("exchanged", COLOUR_PATCH, &encode(&exchanged)),
    )
}

/// Storage whose CPU bytes may end before its own descriptor does.
///
/// Safe public storage can publish such bytes, and a full-frame mapping in
/// the source layout shares them without a copy that would notice.
#[derive(Debug)]
struct ShortPixels {
    descriptor: FrameDescriptor,
    pixels: Arc<CpuPixels>,
}

impl FrameStorage for ShortPixels {
    fn descriptor(&self) -> FrameDescriptor {
        self.descriptor
    }

    fn cpu_pixels(&self) -> Option<Arc<CpuPixels>> {
        Some(Arc::clone(&self.pixels))
    }

    fn read_cpu(&self, _operation: &OperationContext) -> mado_pilot_core::Result<Arc<CpuPixels>> {
        Ok(Arc::clone(&self.pixels))
    }
}

/// Builds a repeated non-uniform pattern with exact matches at x=0, 2, and 4.
/// The four-pixel-wide placements overlap, so only `KeepAll` may report all three.
fn overlapping_fixture() -> (mado_pilot_capture::Frame, TemplateSource) {
    let template_extent = PixelExtent::new(4, 4);
    let frame_extent = PixelExtent::new(8, 4);
    let color = |x: u32| {
        if x.is_multiple_of(2) {
            [0xf0, 0x20, 0x40]
        } else {
            [0x10, 0xd0, 0x80]
        }
    };

    let mut template_rgb = Vec::new();
    for _ in 0..template_extent.height() {
        for x in 0..template_extent.width() {
            template_rgb.extend_from_slice(&color(x));
        }
    }

    let mut frame_pixels = Vec::new();
    for _ in 0..frame_extent.height() {
        for x in 0..frame_extent.width() {
            let [red, green, blue] = color(x);
            frame_pixels.extend_from_slice(&[red, green, blue, 0xff]);
        }
    }

    let frame = vision_contract::frame_with_pixels(frame_extent, PixelFormat::Rgba8, frame_pixels);
    let encoded = png::encode_rgb(
        template_extent.width(),
        template_extent.height(),
        &template_rgb,
    );
    let template = TemplateSource::new(TemplateSourceRequest {
        id: TemplateId::new("periodic").expect("non-empty"),
        encoding: TemplateEncoding::Png,
        extent: template_extent,
        space: CoordinateSpace::CapturePixels,
        defaults: MatchDefaults::new(0.99, 3).expect("valid defaults"),
        content: Arc::from(encoded.as_slice()),
    })
    .expect("a valid template source");

    (frame, template)
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
    // The lower threshold admits the degraded copy, so a measured score is
    // compared as well as the exact ones.
    let rgba = search_scene(PixelFormat::Rgba8, 0.5);
    let bgra = search_scene(PixelFormat::Bgra8, 0.5);

    for result in [&rgba, &bgra] {
        assert_eq!(
            bounds_of(result),
            vec![planted_bounds(0), planted_bounds(1), degraded_bounds()]
        );
        assert_scene_scores(result);
    }
    assert_equivalent(&rgba, &bgra);
}

#[test]
fn a_red_blue_inversion_does_not_match_in_either_layout() {
    for format in LAYOUTS {
        let (frame, colours, exchanged) = colour_fixture(format);
        for (template, origin) in [(&colours, TRUE_COLOURS), (&exchanged, SWAPPED_COLOURS)] {
            let result = find_in(&frame, template, RegionSelection::FullFrame, options(0.9));

            assert_eq!(
                bounds_of(&result),
                vec![bounds_at(origin, COLOUR_PATCH)],
                "{format} must keep red and blue apart"
            );
            assert!(
                (result.matches()[0].score() - EXACT_MATCH_SCORE).abs() <= SCORE_TOLERANCE,
                "{format}: {}",
                result.matches()[0].score()
            );
        }
    }
}

#[test]
fn alpha_is_neither_a_mask_nor_a_scoring_channel() {
    let patch_rgba: Vec<u8> = match_fixtures::patch_rgb()
        .chunks_exact(3)
        .zip(0_u32..)
        .flat_map(|(rgb, index)| [rgb[0], rgb[1], rgb[2], noise(index, 0, 5)[3]])
        .collect();
    let opaque_template = planted_template("patch");
    let translucent_template = template_source(
        "translucent",
        PATCH,
        &png::encode_rgba(PATCH.width(), PATCH.height(), &patch_rgba),
    );
    for format in LAYOUTS {
        let opaque = match_fixtures::scene_frame(format);
        let mut pixels = match_fixtures::scene_pixels(format);
        for (pixel, index) in pixels.chunks_exact_mut(4).zip(0_u32..) {
            pixel[3] = noise(index, 0, 4)[3];
        }
        let translucent = vision_contract::frame_with_pixels(SCENE, format, pixels);
        let reference = find_in(
            &opaque,
            &opaque_template,
            RegionSelection::FullFrame,
            options(0.5),
        );
        assert_eq!(
            bounds_of(&reference),
            vec![planted_bounds(0), planted_bounds(1), degraded_bounds()]
        );
        assert_scene_scores(&reference);

        for (frame, template) in [
            (&opaque, &translucent_template),
            (&translucent, &opaque_template),
            (&translucent, &translucent_template),
        ] {
            assert_equivalent(
                &find_in(frame, template, RegionSelection::FullFrame, options(0.5)),
                &reference,
            );
        }
    }
}

#[test]
fn a_frame_whose_rows_are_padded_reaches_the_same_answer() {
    // Both accepted layouts map the whole frame without converting it, so the
    // frame's own stride reaches `region_to_bgr`. That is the path a Windows
    // capture frame takes first: a driver's row pitch is not the caller's to
    // choose, and most fixtures in this repository are packed. A backend that
    // used `width * 4` where the stride belongs reads each row shifted a
    // little further into the previous one's padding, so the answers below
    // stop agreeing.
    for format in LAYOUTS {
        let packed = search_scene(format, 0.9);
        let padded = search_padded_scene(format, 0.9, PADDING);

        assert_equivalent(&padded, &packed);
        assert_eq!(
            padded.matches().len(),
            PLANTED.len(),
            "and both planted copies are still found"
        );
    }
}

#[test]
fn a_patch_the_scene_does_not_contain_is_a_successful_empty_result() {
    for frame in scene_frames() {
        let result = find_in(
            &frame,
            &absent_template("absent"),
            RegionSelection::FullFrame,
            options(0.9),
        );

        assert!(result.is_empty());
        assert_eq!(result.stamp(), frame.stamp());
    }
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
    for frame in scene_frames() {
        let result = search_frame(
            &frame,
            RegionSelection::Region {
                rect: rect(55.0, 35.0, 80.0, 60.0),
                policy: ClipPolicy::Reject,
            },
            0.9,
        );

        assert_eq!(
            result.matches().len(),
            1,
            "only the copy inside the region is searched for: {:?}",
            frame.descriptor()
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
        assert_eq!(result.transform(), frame.transform());
    }
}

#[test]
fn a_clipped_region_reports_full_frame_coordinates() {
    let expected = planted_bounds(1);
    for frame in scene_frames() {
        let result = search_frame(
            &frame,
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
        assert_eq!(result.matches().len(), 1, "{:?}", frame.descriptor());
        assert_eq!(result.matches()[0].bounds(), expected);
        assert_eq!(result.transform(), frame.transform());
    }
}

#[test]
fn an_explicit_empty_region_is_invalid_geometry() {
    let matcher = matcher();
    let frame = match_fixtures::scene_frame(PixelFormat::Rgba8);
    let prepared = matcher
        .prepare(&planted_template("patch"), &OperationContext::new())
        .expect("the patch prepares");
    let error = matcher
        .find(
            MatchRequest::new(
                &frame,
                RegionSelection::Region {
                    rect: rect(12.0, 12.0, 12.0, 24.0),
                    policy: ClipPolicy::Clip,
                },
                &prepared,
                options(0.9),
            ),
            &OperationContext::new(),
        )
        .expect_err("an explicit empty ROI is malformed rather than absent");

    assert_eq!(error, Error::from(GeometryFault::EmptyRegion));
}

#[test]
fn keep_all_reports_overlapping_opencv_candidates() {
    let matcher = matcher();
    let (frame, template) = overlapping_fixture();
    let prepared = matcher
        .prepare(&template, &OperationContext::new())
        .expect("the periodic template prepares");
    let options =
        MatchOptions::from_defaults(template.defaults()).with_suppression(Suppression::KeepAll);
    let result = matcher
        .find(
            MatchRequest::new(&frame, RegionSelection::FullFrame, &prepared, options),
            &OperationContext::new(),
        )
        .expect("the repeated pattern searches");

    assert_eq!(
        bounds_of(&result),
        vec![
            PixelRect::new(0, 0, 4, 4).expect("valid"),
            PixelRect::new(2, 0, 6, 4).expect("valid"),
            PixelRect::new(4, 0, 8, 4).expect("valid"),
        ],
        "KeepAll must not be converted to overlap suppression by the backend"
    );
}

#[test]
fn a_template_larger_than_the_region_is_a_successful_empty_result() {
    for format in LAYOUTS {
        let frame = match_fixtures::scene_frame(format);
        let result = find_in(
            &frame,
            &match_fixtures::oversized_template("wide", PixelExtent::new(120, 80)),
            RegionSelection::FullFrame,
            options(0.5),
        );

        assert!(result.is_empty());
        assert_eq!(result.stamp(), frame.stamp());
    }
}

#[test]
fn suppression_and_result_limits_are_unchanged_in_both_layouts() {
    let template = planted_template("patch");
    for (suppression, survivors) in [
        (Suppression::DropOverlapping, 3),
        (Suppression::KeepAll, 15),
    ] {
        let mut unlimited = Vec::new();
        for format in LAYOUTS {
            let frame = match_fixtures::scene_frame(format);
            let search = |limit| {
                let options = options(LIMIT_THRESHOLD)
                    .with_max_results(limit)
                    .expect("a positive limit")
                    .with_suppression(suppression);
                find_in(&frame, &template, RegionSelection::FullFrame, options)
            };
            // Sixty-four is above either policy's survivor count.
            let all = search(64);
            let matches = all.matches();
            assert_eq!(matches.len(), survivors, "{suppression:?} in {format}");
            assert!(
                matches
                    .windows(2)
                    .all(|pair| pair[0].score() >= pair[1].score()),
                "canonical order is descending score"
            );
            let mut leaders: Vec<_> = matches[..3].iter().map(|found| found.bounds()).collect();
            leaders.sort_by_key(|rectangle| (rectangle.top(), rectangle.left()));
            assert_eq!(
                leaders,
                [planted_bounds(0), planted_bounds(1), degraded_bounds()]
            );

            for limit in [1, 8, 64] {
                let limited = search(limit);
                let kept = survivors.min(usize::try_from(limit).expect("small"));
                assert_eq!(limited.matches().len(), kept, "limit {limit}");
                if kept == 1 {
                    // The two exact copies tie within rounding, so either may lead.
                    let leader = &limited.matches()[0];
                    assert!([planted_bounds(0), planted_bounds(1)].contains(&leader.bounds()));
                    assert!((leader.score() - EXACT_MATCH_SCORE).abs() <= SCORE_TOLERANCE);
                } else {
                    let mut prefix: Vec<_> =
                        matches[..kept].iter().map(|found| found.bounds()).collect();
                    prefix.sort_by_key(|rectangle| (rectangle.top(), rectangle.left()));
                    assert_eq!(bounds_of(&limited), prefix, "limit {limit}");
                }
            }
            unlimited.push(all);
        }
        assert_equivalent(&unlimited[0], &unlimited[1]);
    }
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
fn spatially_constant_templates_cannot_match_an_unrelated_blue_frame() {
    let matcher = matcher();
    for extent in [PixelExtent::new(1, 1), PixelExtent::new(2, 2)] {
        for rgb in [[255, 0, 0], [0, 0, 0], [255, 255, 255], [73; 3]] {
            let source = template_source(
                "constant",
                extent,
                &png::solid_rgb(extent.width(), extent.height(), rgb),
            );
            assert_eq!(
                matcher
                    .prepare(&source, &OperationContext::new())
                    .expect_err("constant content must fail before publishing a prepared template"),
                Error::from(VisionFault::TemplatePreparationFailed),
                "{extent:?}, {rgb:?}"
            );
        }
    }
    for format in LAYOUTS {
        let frame = vision_contract::frame_with_pixels(
            PixelExtent::new(8, 8),
            format,
            four_channel(format, [0, 0, 255], 255).repeat(8 * 8),
        );
        let operation = OperationContext::new();
        let prepared = matcher
            .prepare(&planted_template("valid-after-rejection"), &operation)
            .expect("nonconstant content remains eligible");
        let unrelated = matcher
            .find(
                MatchRequest::new(&frame, RegionSelection::FullFrame, &prepared, options(0.9)),
                &operation,
            )
            .expect("a valid template can search either source layout after refusal");
        assert!(unrelated.matches().is_empty());
        let scene = match_fixtures::scene_frame(format);
        for _ in 0..2 {
            let result = matcher
                .find(
                    MatchRequest::new(&scene, RegionSelection::FullFrame, &prepared, options(0.5)),
                    &operation,
                )
                .expect("valid preparation remains reusable");
            assert_eq!(
                bounds_of(&result),
                [planted_bounds(0), planted_bounds(1), degraded_bounds()]
            );
            assert_scene_scores(&result);
            assert_eq!(result.stamp(), scene.stamp());
            assert_eq!(result.backend(), &matcher.descriptor());
            assert_eq!(result.matches()[2].bounds(), degraded_bounds());
            assert!(
                result
                    .matches()
                    .windows(2)
                    .all(|pair| pair[0].score() >= pair[1].score())
            );
        }
    }
}

#[test]
fn alpha_only_variation_is_a_typed_preparation_failure() {
    let extent = PixelExtent::new(2, 2);
    let matcher = matcher();
    for rgb in [[255, 0, 0], [0, 0, 0], [255, 255, 255], [31, 79, 151]] {
        let rgba: Vec<u8> = [0, 73, 179, 255]
            .into_iter()
            .flat_map(|alpha| [rgb[0], rgb[1], rgb[2], alpha])
            .collect();
        let source = template_source(
            "alpha-only",
            extent,
            &png::encode_rgba(extent.width(), extent.height(), &rgba),
        );
        assert_eq!(
            matcher
                .prepare(&source, &OperationContext::new())
                .expect_err("alpha is not a scoring channel"),
            Error::from(VisionFault::TemplatePreparationFailed),
            "{rgb:?}"
        );
    }
}

#[test]
fn variation_in_any_one_bgr_channel_remains_reusable_in_both_source_layouts() {
    let extent = PixelExtent::new(2, 2);
    let matcher = matcher();
    let operation = OperationContext::new();
    for channel in 0..3 {
        let mut rgb = [[31, 79, 151]; 4];
        // A last-pixel difference must suffice even when two channels are constant.
        rgb[3][channel] += 100;
        let rgba: Vec<u8> = rgb
            .iter()
            .zip([0, 73, 179, 255])
            .flat_map(|(pixel, alpha)| [pixel[0], pixel[1], pixel[2], alpha])
            .collect();
        for encoded in [
            png::encode_rgb(extent.width(), extent.height(), rgb.as_flattened()),
            png::encode_rgba(extent.width(), extent.height(), &rgba),
        ] {
            let source = template_source("one-channel", extent, &encoded);
            let prepared = matcher
                .prepare(&source, &operation)
                .expect("spatial variation");
            for format in LAYOUTS {
                let pixels = rgb
                    .iter()
                    .flat_map(|&pixel| four_channel(format, pixel, 211))
                    .collect();
                let frame = vision_contract::frame_with_pixels(extent, format, pixels);
                for _ in 0..2 {
                    let result = matcher
                        .find(
                            MatchRequest::new(
                                &frame,
                                RegionSelection::FullFrame,
                                &prepared,
                                options(0.9),
                            ),
                            &operation,
                        )
                        .expect("prepared template remains reusable");
                    assert_eq!(result.backend(), &matcher.descriptor());
                    assert_eq!(result.stamp(), frame.stamp());
                    assert_eq!(bounds_of(&result), [bounds_at((0, 0), extent)]);
                    assert!(
                        (result.matches()[0].score() - EXACT_MATCH_SCORE).abs() <= SCORE_TOLERANCE
                    );
                }
            }
        }
    }
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
    for format in LAYOUTS {
        let matcher = matcher();
        let frame = match_fixtures::scene_frame(format);
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
        let later = vision_contract::frame(PixelExtent::new(32, 32), format, 0x7f);
        assert_ne!(later.stamp(), expected_stamp);

        drop(frame);
        assert_eq!(result.stamp(), expected_stamp);
        assert_eq!(result.transform(), &expected_transform);
        assert_eq!(
            bounds_of(&result),
            vec![planted_bounds(0), planted_bounds(1)]
        );
    }
}

#[test]
fn one_retained_frame_keeps_different_template_results_independent() {
    let matcher = matcher();
    let frame = match_fixtures::scene_frame(PixelFormat::Rgba8);
    let context = OperationContext::new();
    let present = matcher
        .prepare(&planted_template("present"), &context)
        .expect("present template");
    let absent = matcher
        .prepare(&absent_template("absent"), &context)
        .expect("absent template");
    for (selection, expected) in [
        (
            RegionSelection::FullFrame,
            vec![planted_bounds(0), planted_bounds(1)],
        ),
        (
            RegionSelection::Region {
                rect: rect(10.0, 5.0, 45.0, 30.0),
                policy: ClipPolicy::Reject,
            },
            vec![planted_bounds(0)],
        ),
    ] {
        for (template, expected_bounds) in [
            (&present, expected.as_slice()),
            (&absent, [].as_slice()),
            (&present, expected.as_slice()),
        ] {
            let result = matcher
                .find(
                    MatchRequest::new(&frame, selection, template, options(0.9)),
                    &context,
                )
                .expect("independent search");
            assert_eq!(bounds_of(&result), expected_bounds);
            assert_eq!(result.stamp(), frame.stamp());
        }
    }
}

/// Publishes `pixels` in `format` under `basis`'s stamp and transform.
fn equal_stamp_frame(
    basis: &mado_pilot_capture::Frame,
    format: PixelFormat,
    pixels: Vec<u8>,
) -> mado_pilot_capture::Frame {
    mado_pilot_capture::Frame::new(
        basis.stamp(),
        basis.captured_at(),
        FrameDescriptor::packed(basis.descriptor().extent(), format).expect("descriptor"),
        *basis.transform(),
        pixels.into_boxed_slice(),
    )
    .expect("same metadata with independent immutable pixels")
}

#[test]
fn equal_stamp_frames_never_share_different_pixels() {
    let matcher = matcher();
    let scene = match_fixtures::scene_frame(PixelFormat::Rgba8);
    let same = |format, pixels| equal_stamp_frame(&scene, format, pixels);
    let background = same(
        PixelFormat::Rgba8,
        match_fixtures::background_pixels(PixelFormat::Rgba8),
    );
    let bgra_scene = same(
        PixelFormat::Bgra8,
        match_fixtures::scene_pixels(PixelFormat::Bgra8),
    );
    let bgra_background = same(
        PixelFormat::Bgra8,
        match_fixtures::background_pixels(PixelFormat::Bgra8),
    );
    let clone = scene.clone();
    let context = OperationContext::new();
    let prepared = matcher
        .prepare(&planted_template("patch"), &context)
        .expect("template");
    let region = RegionSelection::Region {
        rect: rect(10.0, 5.0, 45.0, 30.0),
        policy: ClipPolicy::Reject,
    };
    for (frame, found) in [
        (&scene, true),
        (&background, false),
        (&bgra_scene, true),
        (&bgra_background, false),
        (&clone, true),
        (&bgra_scene, true),
        (&scene, true),
    ] {
        for (selection, expected) in [
            (
                RegionSelection::FullFrame,
                vec![planted_bounds(0), planted_bounds(1)],
            ),
            (region, vec![planted_bounds(0)]),
        ] {
            let result = matcher
                .find(
                    MatchRequest::new(frame, selection, &prepared, options(0.9)),
                    &context,
                )
                .expect("search");
            let expected = if found { expected } else { vec![] };
            assert_eq!(bounds_of(&result), expected, "{:?}", frame.descriptor());
            assert_eq!(result.stamp(), scene.stamp());
        }
    }
}

#[test]
fn backend_preparation_uses_the_mapping_region_not_the_diagnostic_region() {
    let backend = OpenCvBackend::new().expect("backend");
    let frame = match_fixtures::scene_frame(PixelFormat::Rgba8);
    let background = equal_stamp_frame(
        &frame,
        PixelFormat::Bgra8,
        match_fixtures::background_pixels(PixelFormat::Bgra8),
    );
    let context = OperationContext::new();
    let template = backend
        .prepare(&planted_template("patch"), &context)
        .expect("template");
    let diagnostic = PixelRect::new(999, 999, 1011, 1009).expect("diagnostic rectangle");
    let patch = Some(rect(20.0, 12.0, 32.0, 22.0));
    let corner = Some(rect(0.0, 0.0, 12.0, 10.0));
    for format in [PixelFormat::Rgba8, PixelFormat::Bgra8, PixelFormat::Rgba8] {
        for (source, region, expected) in [
            (&frame, patch, vec![(0, 0)]),
            (&frame, corner, vec![]),
            (&frame, None, vec![(20, 12), (60, 40)]),
            (&background, patch, vec![]),
            (&background, None, vec![]),
            (&frame, patch, vec![(0, 0)]),
        ] {
            let view = match region {
                Some(region) => source.view(region, ClipPolicy::Reject),
                None => source.full_view(),
            };
            let pixels = view.expect("view").map(format, &context).expect("pixels");
            assert_eq!(pixels.descriptor().format(), format);
            let found = backend
                .find(
                    &mado_pilot_vision::BackendRequest {
                        template: &template,
                        pixels: &pixels,
                        region: diagnostic,
                        options: options(0.9),
                    },
                    &context,
                )
                .expect("either accepted layout searches");
            let mut origins: Vec<_> = found.iter().map(|candidate| candidate.origin()).collect();
            origins.sort_unstable();
            assert_eq!(origins, expected, "{format} of {:?}", source.descriptor());
            assert!(
                found
                    .iter()
                    .all(|candidate| (candidate.score() - EXACT_MATCH_SCORE).abs()
                        <= SCORE_TOLERANCE)
            );
        }
    }
}

#[test]
fn a_persistent_backend_follows_frame_turnover_layouts_and_regions() {
    let matcher = matcher();
    let context = OperationContext::new();
    let prepared = matcher
        .prepare(&planted_template("patch"), &context)
        .expect("template");
    let roi = |left, top, right, bottom, policy| RegionSelection::Region {
        rect: rect(left, top, right, bottom),
        policy,
    };
    let first = roi(10.0, 5.0, 45.0, 30.0, ClipPolicy::Reject);
    let second = roi(55.0, 35.0, 200.0, 200.0, ClipPolicy::Clip);
    let both = vec![planted_bounds(0), planted_bounds(1)];
    let cases = [
        (
            match_fixtures::scene_frame(PixelFormat::Rgba8),
            RegionSelection::FullFrame,
            both.clone(),
        ),
        (
            match_fixtures::scene_frame(PixelFormat::Bgra8),
            second,
            vec![planted_bounds(1)],
        ),
        (
            match_fixtures::scene_frame_with_padded_rows(PixelFormat::Rgba8, PADDING),
            first,
            vec![planted_bounds(0)],
        ),
        (
            vision_contract::frame_with_pixels(
                SCENE,
                PixelFormat::Bgra8,
                match_fixtures::background_pixels(PixelFormat::Bgra8),
            ),
            RegionSelection::FullFrame,
            vec![],
        ),
        (
            match_fixtures::scene_frame_with_padded_rows(PixelFormat::Bgra8, PADDING),
            RegionSelection::FullFrame,
            both,
        ),
    ];
    // Five sources outnumber the two retained preparations, so every round
    // turns the earlier ones over; each case repeats once as a compatible hit.
    for _ in 0..2 {
        for (frame, selection, expected) in &cases {
            for _ in 0..2 {
                let result = matcher
                    .find(
                        MatchRequest::new(frame, *selection, &prepared, options(0.9)),
                        &context,
                    )
                    .expect("search");
                assert_eq!(&bounds_of(&result), expected, "{:?}", frame.descriptor());
                assert_eq!(result.stamp(), frame.stamp());
            }
        }
    }
}

#[test]
fn a_region_above_the_preparation_allowance_is_still_searched() {
    // 4096 by 2731 converts to 33,558,528 BGR bytes, just above the
    // 33,554,432 a backend may retain, so the search runs on uncached pixels.
    let extent = PixelExtent::new(4096, 2731);
    let origin = (3001, 2003);
    let patch = match_fixtures::patch_rgb();
    let rgb: Vec<[u8; 3]> = (0..extent.height())
        .flat_map(|y| (0..extent.width()).map(move |x| (x, y)))
        .map(|(x, y)| {
            if x >= origin.0
                && y >= origin.1
                && x < origin.0 + PATCH.width()
                && y < origin.1 + PATCH.height()
            {
                let index = usize::try_from((y - origin.1) * PATCH.width() + (x - origin.0))
                    .expect("small")
                    * 3;
                [patch[index], patch[index + 1], patch[index + 2]]
            } else {
                let [red, green, blue, _] = noise(x, y, 3);
                [red, green, blue]
            }
        })
        .collect();
    let matcher = matcher();
    let context = OperationContext::new();
    let prepared = matcher
        .prepare(&planted_template("patch"), &context)
        .expect("template");
    for format in LAYOUTS {
        let pixels = rgb
            .iter()
            .flat_map(|&pixel| four_channel(format, pixel, 0xff))
            .collect();
        let frame = vision_contract::frame_with_pixels(extent, format, pixels);
        let result = matcher
            .find(
                MatchRequest::new(&frame, RegionSelection::FullFrame, &prepared, options(0.9)),
                &context,
            )
            .expect("an oversized region is searched, not refused");

        assert_eq!(
            bounds_of(&result),
            vec![bounds_at(origin, PATCH)],
            "{format}"
        );
        assert!((result.matches()[0].score() - EXACT_MATCH_SCORE).abs() <= SCORE_TOLERANCE);
    }
    let scene = match_fixtures::scene_frame(PixelFormat::Rgba8);
    let result = matcher
        .find(
            MatchRequest::new(&scene, RegionSelection::FullFrame, &prepared, options(0.9)),
            &context,
        )
        .expect("ordinary preparation still works");
    assert_eq!(
        bounds_of(&result),
        vec![planted_bounds(0), planted_bounds(1)]
    );
}

#[test]
fn pixels_short_of_their_last_readable_row_are_refused_before_opencv_reads_them() {
    let context = OperationContext::new();
    for format in LAYOUTS {
        for padding in [0, PADDING] {
            let basis = match_fixtures::scene_frame_with_padded_rows(format, padding);
            let descriptor = basis.descriptor();
            let bytes = basis
                .map(format, &context)
                .expect("basis bytes")
                .bytes()
                .to_vec();
            // The last row needs its pixels but not its padding.
            let readable = descriptor.byte_len() - padding;
            for (length, accepted) in [(readable, true), (readable - 1, false)] {
                let storage = Arc::new(ShortPixels {
                    descriptor,
                    pixels: Arc::new(CpuPixels::new(bytes[..length].into())),
                });
                let frame = mado_pilot_capture::Frame::from_storage(
                    basis.stamp(),
                    basis.captured_at(),
                    *basis.transform(),
                    storage,
                )
                .expect("safe public storage");
                let matcher = matcher();
                let prepared = matcher
                    .prepare(&planted_template("patch"), &context)
                    .expect("template");
                // A refusal that had published preparation would succeed the
                // second time.
                for _ in 0..2 {
                    let outcome = matcher.find(
                        MatchRequest::new(
                            &frame,
                            RegionSelection::FullFrame,
                            &prepared,
                            options(0.9),
                        ),
                        &context,
                    );
                    if accepted {
                        assert_eq!(
                            bounds_of(&outcome.expect("every readable byte is present")),
                            vec![planted_bounds(0), planted_bounds(1)]
                        );
                    } else {
                        assert_eq!(
                            outcome.expect_err("one byte short of the last row"),
                            Error::from(VisionFault::BackendFailed),
                            "{format} with {padding} bytes of padding"
                        );
                    }
                }
            }
        }
    }
}
