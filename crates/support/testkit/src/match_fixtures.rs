//! The deterministic scene a matching algorithm fixture searches.
//!
//! What a backend finds, and where, needs an image with a known answer. These
//! fixtures are constructed rather than tracked because their construction
//! parameters are the answer: a patch planted at a stated offset is found at that
//! offset, and a reader can check the expectation against the generator instead
//! of against a checked-in file nobody can inspect.
//!
//! Every value here comes from integer arithmetic on the pixel coordinate, so the
//! same scene is produced byte for byte on both release targets. That matters
//! more than it looks: a cross-target score comparison is only about the backend
//! if the two hosts correlated identical pixels.
//!
//! Two properties are deliberate. The background is pseudo-random noise, so a
//! structured patch cannot correlate with it by accident and a "no match" result
//! is a real absence rather than a lucky threshold. The patch is bordered and
//! graduated, so it has the variance normalized correlation needs — a uniform
//! patch has no correlation to express and would make every fixture degenerate.

use std::sync::Arc;

use mado_pilot_capture::{Frame, PixelFormat};
use mado_pilot_core::{CoordinateSpace, PixelExtent};
use mado_pilot_vision::{
    MatchDefaults, TemplateEncoding, TemplateId, TemplateSource, TemplateSourceRequest,
};

/// The scene's extent.
pub const SCENE: PixelExtent = PixelExtent::new(96, 64);

/// The extent of the patch planted in the scene, and of both templates.
pub const PATCH: PixelExtent = PixelExtent::new(12, 10);

/// Where an exact copy of the patch is planted, in capture pixels.
///
/// Two byte-identical copies at different offsets is what makes a tolerance
/// fixture real. It is deliberately *not* an equal-score fixture: OpenCV's
/// correlation is not computed offset by offset, so identical content at two
/// offsets does not correlate to bit-identical values, and any ordering between
/// the two would rest on a difference smaller than the tolerance. Equal-score
/// ordering is covered where equal scores can actually be constructed — the
/// matcher's own rules and the adapter's peak extractor.
pub const PLANTED: [(u32, u32); 2] = [(20, 12), (60, 40)];

/// Where a half-strength copy of the patch is planted.
///
/// It is the ordering fixture: its correlation is far enough below an exact
/// copy's that descending-score order is decided by a real difference rather
/// than by rounding, so the order holds on both release targets.
pub const DEGRADED: (u32, u32) = (20, 44);

/// Returns the scene as packed row-major RGB.
#[must_use]
pub fn scene_rgb() -> Vec<u8> {
    let width = SCENE.width();
    let height = SCENE.height();
    let mut rgb = Vec::with_capacity(pixels(SCENE) * 3);

    for y in 0..height {
        for x in 0..width {
            rgb.extend_from_slice(&planted_at(x, y).unwrap_or_else(|| background(x, y)));
        }
    }

    rgb
}

/// Returns the patch as packed row-major RGB.
#[must_use]
pub fn patch_rgb() -> Vec<u8> {
    let mut rgb = Vec::with_capacity(pixels(PATCH) * 3);

    for y in 0..PATCH.height() {
        for x in 0..PATCH.width() {
            rgb.extend_from_slice(&patch(x, y));
        }
    }

    rgb
}

/// Returns a patch that is not planted anywhere in the scene.
#[must_use]
pub fn absent_rgb() -> Vec<u8> {
    let mut rgb = Vec::with_capacity(pixels(PATCH) * 3);

    for y in 0..PATCH.height() {
        for x in 0..PATCH.width() {
            // Diagonal bands in the opposite colour direction from the planted
            // patch, so a false positive would have to come from the background.
            let band = byte((x + y) * 24);
            rgb.extend_from_slice(&[0xff - band, band, 0x80]);
        }
    }

    rgb
}

/// Returns the scene as packed row-major pixels in `format`.
///
/// A capture adapter is fed raw pixels rather than a frame, so a replay source
/// built from this scene and a frame built from it are the same image by
/// construction instead of by two generators agreeing.
///
/// # Panics
///
/// Panics for a format that is not four bytes per pixel, which no supported
/// format is.
#[must_use]
pub fn scene_pixels(format: PixelFormat) -> Vec<u8> {
    to_four_channel(&scene_rgb(), format)
}

/// Returns the scene's deterministic background without planted template copies.
///
/// # Panics
///
/// Panics for a format that is not four bytes per pixel, which no supported
/// format is.
#[must_use]
pub fn background_pixels(format: PixelFormat) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(pixels(SCENE) * 3);
    for y in 0..SCENE.height() {
        for x in 0..SCENE.width() {
            rgb.extend_from_slice(&background(x, y));
        }
    }
    to_four_channel(&rgb, format)
}

/// Builds the scene as an immutable frame in `format`.
///
/// # Panics
///
/// Panics for a format that is not four bytes per pixel, which no supported
/// format is.
#[must_use]
pub fn scene_frame(format: PixelFormat) -> Frame {
    crate::vision_contract::frame_with_pixels(SCENE, format, scene_pixels(format))
}

/// Builds the same scene with `padding` trailing bytes on every row.
///
/// The same image as [`scene_frame`], so a search over it must produce the same
/// answer; what differs is the stride, which is the one thing a capture adapter
/// does not choose and every packed fixture hides.
///
/// # Panics
///
/// As [`scene_frame`].
#[must_use]
pub fn scene_frame_with_padded_rows(format: PixelFormat, padding: usize) -> Frame {
    crate::vision_contract::frame_with_padded_rows(SCENE, format, padding, &scene_pixels(format))
}

/// Builds a template source for the patch the scene contains.
///
/// # Panics
///
/// Panics for an identity a template cannot have.
#[must_use]
pub fn planted_template(id: &str) -> TemplateSource {
    template(id, &patch_rgb())
}

/// Builds a template source for a patch the scene does not contain.
///
/// # Panics
///
/// Panics for an identity a template cannot have.
#[must_use]
pub fn absent_template(id: &str) -> TemplateSource {
    template(id, &absent_rgb())
}

/// Builds a template source of `extent` that is a solid colour.
///
/// Used where the fixture is about the extent rather than the content, such as a
/// template deliberately larger than the region it is searched in.
///
/// # Panics
///
/// Panics for an identity or extent a template cannot have.
#[must_use]
pub fn oversized_template(id: &str, extent: PixelExtent) -> TemplateSource {
    let content = crate::png::solid_rgb(extent.width(), extent.height(), [0x11, 0x22, 0x33]);

    TemplateSource::new(TemplateSourceRequest {
        id: TemplateId::new(id).expect("a non-empty identity"),
        encoding: TemplateEncoding::Png,
        extent,
        space: CoordinateSpace::CapturePixels,
        defaults: MatchDefaults::new(0.5, 8).expect("valid defaults"),
        content: Arc::from(content.as_slice()),
    })
    .expect("a valid template source")
}

fn template(id: &str, rgb: &[u8]) -> TemplateSource {
    let content = crate::png::encode_rgb(PATCH.width(), PATCH.height(), rgb);

    TemplateSource::new(TemplateSourceRequest {
        id: TemplateId::new(id).expect("a non-empty identity"),
        encoding: TemplateEncoding::Png,
        extent: PATCH,
        space: CoordinateSpace::CapturePixels,
        defaults: MatchDefaults::new(0.9, 8).expect("valid defaults"),
        content: Arc::from(content.as_slice()),
    })
    .expect("a valid template source")
}

/// Expands packed RGB into the four-channel `format` with an opaque alpha.
fn to_four_channel(rgb: &[u8], format: PixelFormat) -> Vec<u8> {
    assert_eq!(
        format.bytes_per_pixel(),
        4,
        "every supported format is four bytes per pixel"
    );

    let mut out = Vec::with_capacity(rgb.len() / 3 * 4);
    for pixel in rgb.chunks_exact(3) {
        match format {
            PixelFormat::Rgba8 => out.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 0xff]),
            PixelFormat::Bgra8 => out.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 0xff]),
            // `PixelFormat` is `#[non_exhaustive]`; a later format needs its own
            // expansion rather than a silent reinterpretation of these bytes.
            _ => panic!("the fixtures do not know how to build {format}"),
        }
    }

    out
}

/// Returns the pixel covering `(x, y)`, if the scene plants a copy there.
fn planted_at(x: u32, y: u32) -> Option<[u8; 3]> {
    if let Some(pixel) = PLANTED
        .iter()
        .find_map(|&origin| covers(origin, x, y).map(|(px, py)| patch(px, py)))
    {
        return Some(pixel);
    }

    covers(DEGRADED, x, y).map(|(px, py)| {
        // Halfway between the patch and the noise it sits on. The structure
        // survives, so the copy is still found; half its contrast does not, so it
        // correlates measurably worse than an exact copy.
        let ideal = patch(px, py);
        let noise = background(x, y);
        [
            midpoint(ideal[0], noise[0]),
            midpoint(ideal[1], noise[1]),
            midpoint(ideal[2], noise[2]),
        ]
    })
}

/// Returns `(x, y)` relative to `origin` when a patch at `origin` covers it.
fn covers(origin: (u32, u32), x: u32, y: u32) -> Option<(u32, u32)> {
    let (left, top) = origin;
    let inside = x >= left && y >= top && x < left + PATCH.width() && y < top + PATCH.height();

    inside.then(|| (x - left, y - top))
}

fn midpoint(first: u8, second: u8) -> u8 {
    byte((u32::from(first) + u32::from(second)) / 2)
}

/// The patch: a white border around a two-axis gradient.
fn patch(x: u32, y: u32) -> [u8; 3] {
    let edge = x == 0 || y == 0 || x + 1 == PATCH.width() || y + 1 == PATCH.height();
    if edge {
        return [0xff, 0xff, 0xff];
    }

    [byte(x * 20), byte(y * 24), 0x30]
}

/// The background: reproducible noise, so nothing correlates with it by accident.
fn background(x: u32, y: u32) -> [u8; 3] {
    // Two odd multipliers and one xor-shift, which is enough decorrelation for a
    // fixture and is exactly reproducible in integer arithmetic.
    let mixed = x
        .wrapping_mul(2_654_435_761)
        .wrapping_add(y.wrapping_mul(2_246_822_519));
    let mixed = mixed ^ (mixed >> 15);

    [byte(mixed), byte(mixed >> 8), byte(mixed >> 16)]
}

fn pixels(extent: PixelExtent) -> usize {
    usize::try_from(extent.width()).expect("a fixture stays small")
        * usize::try_from(extent.height()).expect("a fixture stays small")
}

fn byte(value: u32) -> u8 {
    u8::try_from(value & 0xff).expect("a masked value is one byte")
}

#[cfg(test)]
mod tests {
    use mado_pilot_capture::PixelFormat;

    use super::{
        DEGRADED, PATCH, PLANTED, SCENE, absent_rgb, background_pixels, patch_rgb, scene_frame,
        scene_rgb, to_four_channel,
    };

    #[test]
    fn the_scene_is_the_declared_extent() {
        assert_eq!(scene_rgb().len(), 96 * 64 * 3);
        assert_eq!(patch_rgb().len(), 12 * 10 * 3);
        assert_eq!(absent_rgb().len(), 12 * 10 * 3);
    }

    #[test]
    fn the_scene_is_byte_identical_between_runs() {
        assert_eq!(scene_rgb(), scene_rgb());
    }

    #[test]
    fn the_background_scene_is_deterministic_and_differs_from_the_planted_scene() {
        let background = background_pixels(PixelFormat::Rgba8);

        assert_eq!(background.len(), 96 * 64 * 4);
        assert_eq!(background, background_pixels(PixelFormat::Rgba8));
        assert_ne!(
            background,
            to_four_channel(&scene_rgb(), PixelFormat::Rgba8)
        );
    }

    #[test]
    fn every_planted_copy_holds_the_patch_exactly() {
        let scene = scene_rgb();
        let patch = patch_rgb();
        let width = usize::try_from(SCENE.width()).expect("small");

        for (left, top) in PLANTED {
            let left = usize::try_from(left).expect("small");
            let top = usize::try_from(top).expect("small");
            for row in 0..usize::try_from(PATCH.height()).expect("small") {
                let columns = usize::try_from(PATCH.width()).expect("small") * 3;
                let start = ((top + row) * width + left) * 3;
                assert_eq!(
                    &scene[start..start + columns],
                    &patch[row * columns..(row + 1) * columns],
                    "planted copy at ({left}, {top}) row {row}"
                );
            }
        }
    }

    #[test]
    fn every_planted_copy_stays_inside_the_scene() {
        for (left, top) in PLANTED.iter().copied().chain([DEGRADED]) {
            assert!(left + PATCH.width() <= SCENE.width());
            assert!(top + PATCH.height() <= SCENE.height());
        }
    }

    #[test]
    fn no_two_planted_copies_overlap() {
        let origins: Vec<(u32, u32)> = PLANTED.iter().copied().chain([DEGRADED]).collect();

        for (index, &(left, top)) in origins.iter().enumerate() {
            for &(other_left, other_top) in &origins[index + 1..] {
                let apart = left.abs_diff(other_left) >= PATCH.width()
                    || top.abs_diff(other_top) >= PATCH.height();
                assert!(
                    apart,
                    "({left}, {top}) overlaps ({other_left}, {other_top})"
                );
            }
        }
    }

    #[test]
    fn the_degraded_copy_is_not_the_patch_it_was_built_from() {
        let scene = scene_rgb();
        let width = usize::try_from(SCENE.width()).expect("small");
        let (left, top) = DEGRADED;
        let start = ((usize::try_from(top).expect("small") + 1) * width
            + usize::try_from(left).expect("small")
            + 1)
            * 3;

        assert_ne!(
            &scene[start..start + 3],
            &patch_rgb()[(PATCH.width() as usize + 1) * 3..][..3]
        );
    }

    #[test]
    fn the_two_formats_describe_the_same_scene_in_opposite_channel_order() {
        let rgba = to_four_channel(&[10, 20, 30], PixelFormat::Rgba8);
        let bgra = to_four_channel(&[10, 20, 30], PixelFormat::Bgra8);

        assert_eq!(rgba, vec![10, 20, 30, 0xff]);
        assert_eq!(bgra, vec![30, 20, 10, 0xff]);
    }

    #[test]
    fn a_frame_is_built_for_either_supported_format() {
        for format in [PixelFormat::Rgba8, PixelFormat::Bgra8] {
            let frame = scene_frame(format);

            assert_eq!(frame.descriptor().extent(), SCENE);
            assert_eq!(frame.descriptor().format(), format);
        }
    }

    #[test]
    fn the_absent_patch_is_not_the_planted_one() {
        assert_ne!(patch_rgb(), absent_rgb());
    }
}
