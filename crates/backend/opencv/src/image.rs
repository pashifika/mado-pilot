//! The narrow boundary where MadoPilot bytes become OpenCV matrices.
//!
//! Every OpenCV matrix this adapter works with is created here, and every one of
//! them is owned and packed by the time it leaves. That is deliberate: a matrix
//! header that borrows a caller's buffer carries a lifetime OpenCV does not
//! track, so the one place that creates such a header also consumes it, and no
//! borrowed matrix is ever returned to the rest of the adapter.
//!
//! The conversion to three-channel BGR happens here too, because it is the only
//! preprocessing the Phase 1 matching profile performs and it is what makes the
//! image and the template comparable. See
//! `docs/adr/0003-opencv-matching-profile-and-public-score.md`.

use std::ffi::c_void;

use mado_pilot_capture::{CpuMapping, PixelFormat};
use mado_pilot_core::PixelExtent;
use mado_pilot_vision::TemplateSource;
use opencv::core::{CV_8UC4, Mat, MatTraitConst};
use opencv::imgcodecs::{IMREAD_COLOR, imdecode};
use opencv::imgproc::{COLOR_BGRA2BGR, COLOR_RGBA2BGR, cvt_color_def};

/// The mapping format requested for a source layout the adapter does not accept.
pub(crate) const DEFAULT_FORMAT: PixelFormat = PixelFormat::Bgra8;

/// The other source layout the adapter converts directly to BGR.
pub(crate) const ALTERNATE_FORMAT: PixelFormat = PixelFormat::Rgba8;

/// The bytes this profile's decoded matrix stores per pixel.
///
/// Named because the decode budget below is the pixel ceiling expressed in this
/// adapter's own storage, and a profile that stored four channels would have a
/// different budget for the same ceiling.
const BGR_CHANNELS: u64 = 3;

/// The most bytes one decoded template may occupy.
///
/// The vision contract's pixel ceiling, in the bytes this adapter actually
/// allocates: 192 MiB. It is the same number
/// [`TemplateSource::MAX_PIXELS`](mado_pilot_vision::TemplateSource::MAX_PIXELS)
/// states, so a template the contract accepts is a template this adapter can
/// decode, and there is one ceiling rather than two that could drift apart.
const MAX_DECODED_BYTES: u64 = TemplateSource::MAX_PIXELS * BGR_CHANNELS;

/// Why a matrix could not be built.
///
/// Kept separate from the public vision faults because the two callers report it
/// differently: the same malformed input is a preparation failure while
/// compiling a template and a backend failure while searching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImageFault {
    /// A dimension or byte count does not fit what OpenCV accepts.
    Unrepresentable,
    /// The mapped pixels are in a layout this adapter does not convert.
    UnexpectedFormat,
    /// The mapped pixels were shorter than their own descriptor requires.
    Truncated,
    /// Encoded content declares an extent other than the one it was loaded as.
    DeclaredExtentMismatch,
    /// A declared extent would decode to more than [`MAX_DECODED_BYTES`].
    AboveDecodeCeiling,
    /// OpenCV rejected the operation.
    Rejected,
}

/// Converts a mapped RGBA or BGRA region into an owned three-channel BGR matrix.
///
/// The mapping's own descriptor selects the conversion; alpha is dropped.
///
/// # Errors
///
/// Returns [`ImageFault`] when the mapping is in an unsupported layout or
/// contradicts its own descriptor, when a dimension does not fit OpenCV's
/// signed extents, or when OpenCV rejects the conversion.
pub(crate) fn region_to_bgr(pixels: &CpuMapping) -> Result<Mat, ImageFault> {
    let descriptor = pixels.descriptor();
    let code = match descriptor.format() {
        PixelFormat::Bgra8 => COLOR_BGRA2BGR,
        PixelFormat::Rgba8 => COLOR_RGBA2BGR,
        // `PixelFormat` is `#[non_exhaustive]`; a later layout is refused
        // rather than read as one of these.
        _ => return Err(ImageFault::UnexpectedFormat),
    };

    let extent = descriptor.extent();
    let rows = signed(extent.height())?;
    let columns = signed(extent.width())?;
    let stride = descriptor.stride();
    let bytes = pixels.bytes();

    // The last row needs `width * 4` bytes; the rows before it need a full
    // stride each. A mapping that cannot satisfy that is not describing itself.
    let row_bytes = usize::try_from(extent.width())
        .ok()
        .and_then(|width| width.checked_mul(4))
        .ok_or(ImageFault::Unrepresentable)?;
    let required = usize::try_from(extent.height())
        .ok()
        .and_then(|height| height.checked_sub(1))
        .and_then(|rows| rows.checked_mul(stride))
        .and_then(|leading| leading.checked_add(row_bytes))
        .ok_or(ImageFault::Unrepresentable)?;
    if stride < row_bytes || bytes.len() < required {
        return Err(ImageFault::Truncated);
    }

    let mut bgr = Mat::default();
    // SAFETY: `view` is a matrix header over `bytes`, which OpenCV does not
    // reference-count and whose lifetime it does not track. Three things make
    // that sound here. The header describes no more than `bytes` holds, checked
    // above against the mapping's own stride and extent. It is only ever read:
    // `cvt_color_def` takes it as an input array and writes into `bgr`, which
    // owns its own storage. And it is dropped at the end of this function, while
    // `pixels` is still borrowed by the caller, so the header cannot outlive the
    // bytes it points at.
    let view = unsafe {
        Mat::new_rows_cols_with_data_unsafe(
            rows,
            columns,
            CV_8UC4,
            bytes.as_ptr().cast::<c_void>().cast_mut(),
            stride,
        )
    }
    .map_err(|_| ImageFault::Rejected)?;

    cvt_color_def(&view, &mut bgr, code).map_err(|_| ImageFault::Rejected)?;
    drop(view);
    #[cfg(feature = "benchmark-instrumentation")]
    crate::benchmark_instrumentation::record_bgr_conversion(bgr.total().saturating_mul(3));

    Ok(bgr)
}

/// Decodes encoded template content of `declared` extent into an owned
/// three-channel BGR matrix.
///
/// `IMREAD_COLOR` is what makes the result comparable to a converted region: it
/// yields three 8-bit BGR channels whatever the file declared, so a greyscale or
/// palette PNG and an alpha-carrying one all arrive in one layout. A template's
/// alpha is dropped rather than honoured; masked matching is not part of the
/// Phase 1 profile.
///
/// `declared` is the extent the template metadata gives the content, and two
/// things about it are settled before OpenCV is handed anything. `imdecode`
/// allocates from the dimensions in the file's own header, and a highly
/// compressible image is small in a package and large in memory, so the decode
/// has to be bounded by something other than the file's size.
///
/// First the size: `declared` must decode to at most [`MAX_DECODED_BYTES`],
/// computed here rather than trusted, so an extent no capture could produce is
/// refused instead of allocated. Then the agreement: the header must declare
/// exactly `declared`, so content cannot bring its own larger dimensions to an
/// extent that passed the ceiling. Equality alone would bound nothing — it makes
/// two attacker-chosen numbers agree — which is why the ceiling comes first.
///
/// The caller compares the decoded extent as well: this says what the file
/// claims to be, and that says what it turned out to be.
///
/// The content is PNG — the only encoding this adapter accepts, checked by the
/// caller — so the declaration is the IHDR chunk, at a fixed offset.
///
/// A template source cannot carry an extent above the ceiling, so the first
/// refusal is not reachable through
/// [`TemplateSource`](mado_pilot_vision::TemplateSource) today. It is enforced
/// here anyway: the allocation is this adapter's, and a bound that lives only in
/// the caller is a bound the next caller does not have.
///
/// # Errors
///
/// Returns [`ImageFault`] when `declared` would decode above the ceiling, when
/// the content does not begin with a PNG header, when that header declares an
/// extent other than `declared`, when the content is longer than OpenCV's signed
/// extents allow, when it does not decode, or when it decodes to something other
/// than a non-empty three-channel image.
pub(crate) fn decode_to_bgr(content: &[u8], declared: PixelExtent) -> Result<Mat, ImageFault> {
    // The product of two `u32` dimensions is exact in `u64`; multiplying by the
    // channel count is the step that can leave the type, so it is checked.
    let pixels = u64::from(declared.width()) * u64::from(declared.height());
    if pixels
        .checked_mul(BGR_CHANNELS)
        .is_none_or(|bytes| bytes > MAX_DECODED_BYTES)
    {
        return Err(ImageFault::AboveDecodeCeiling);
    }

    match png_extent(content) {
        None => return Err(ImageFault::Rejected),
        Some(extent) if extent != declared => return Err(ImageFault::DeclaredExtentMismatch),
        Some(_) => {}
    }

    let length = i32::try_from(content.len()).map_err(|_| ImageFault::Unrepresentable)?;
    let encoded =
        Mat::new_rows_cols_with_data::<u8>(1, length, content).map_err(|_| ImageFault::Rejected)?;

    let decoded = imdecode(&encoded, IMREAD_COLOR).map_err(|_| ImageFault::Rejected)?;
    if decoded.empty() || decoded.channels() != 3 {
        return Err(ImageFault::Rejected);
    }

    Ok(decoded)
}

/// The extent a PNG's header declares, or `None` for content that is not one.
///
/// A PNG begins with an eight-byte signature and then its IHDR chunk, whose
/// four-byte length and four-byte type are followed by the width and height as
/// big-endian unsigned 32-bit values. Nothing else in the file can move them,
/// which is what makes reading them cheap enough to do before the decode.
fn png_extent(content: &[u8]) -> Option<PixelExtent> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
    const HEADER: usize = 24;

    let header: &[u8; HEADER] = content.get(..HEADER)?.try_into().ok()?;
    if header[..8] != SIGNATURE || &header[12..16] != b"IHDR" {
        return None;
    }

    let width = u32::from_be_bytes(header[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(header[20..24].try_into().ok()?);

    Some(PixelExtent::new(width, height))
}

/// Returns a matrix's extent, or [`ImageFault::Unrepresentable`] for dimensions
/// that are not a non-negative extent.
pub(crate) fn extent_of(image: &Mat) -> Result<PixelExtent, ImageFault> {
    let width = u32::try_from(image.cols()).map_err(|_| ImageFault::Unrepresentable)?;
    let height = u32::try_from(image.rows()).map_err(|_| ImageFault::Unrepresentable)?;

    Ok(PixelExtent::new(width, height))
}

fn signed(value: u32) -> Result<i32, ImageFault> {
    i32::try_from(value).map_err(|_| ImageFault::Unrepresentable)
}

#[cfg(test)]
mod tests {
    use mado_pilot_capture::PixelFormat;
    use mado_pilot_core::{OperationContext, PixelExtent};
    use mado_pilot_testkit::vision_contract;
    use opencv::core::{MatTraitConst, MatTraitConstManual};

    use super::{ImageFault, decode_to_bgr, extent_of, region_to_bgr};

    /// Distinct red, green and blue in every pixel, so a red/blue inversion
    /// cannot pass as the same image.
    const RGB: [[u8; 3]; 4] = [[200, 10, 30], [1, 2, 3], [40, 50, 60], [250, 128, 7]];

    /// `RGB` as packed BGR, the only layout this profile stores.
    const BGR: [u8; 12] = [30, 10, 200, 3, 2, 1, 60, 50, 40, 7, 128, 250];

    /// Varying alpha, which no conversion may treat as colour or as a mask.
    const ALPHA: [u8; 4] = [0, 85, 170, 255];

    /// A two-by-two PNG, so the decode path is exercised against real bytes
    /// rather than a signature.
    fn png() -> Vec<u8> {
        mado_pilot_testkit::png::solid_rgb(2, 2, [10, 20, 30])
    }

    /// The extent `png` declares and decodes to.
    fn extent() -> PixelExtent {
        PixelExtent::new(2, 2)
    }

    #[test]
    fn encoded_content_decodes_to_three_bgr_channels() {
        let decoded = decode_to_bgr(&png(), extent()).expect("a valid PNG decodes");

        assert_eq!(decoded.channels(), 3);
        assert_eq!(extent_of(&decoded).expect("a real extent"), extent());
    }

    #[test]
    fn both_accepted_layouts_convert_to_the_same_bgr_without_alpha_or_padding() {
        for (format, order) in [
            (PixelFormat::Rgba8, [0, 1, 2]),
            (PixelFormat::Bgra8, [2, 1, 0]),
        ] {
            let mut packed = Vec::new();
            for (rgb, alpha) in RGB.iter().zip(ALPHA) {
                packed.extend_from_slice(&[rgb[order[0]], rgb[order[1]], rgb[order[2]], alpha]);
            }
            // Twelve bytes of padding is three pixels of filler per row, which
            // would change the output if the stride were ignored.
            for padding in [0, 12] {
                let frame =
                    vision_contract::frame_with_padded_rows(extent(), format, padding, &packed);
                let pixels = frame
                    .map(format, &OperationContext::new())
                    .expect("direct mapping");
                let bgr = region_to_bgr(&pixels).expect("an accepted layout converts");

                assert_eq!(bgr.channels(), 3);
                assert_eq!(bgr.data_bytes().expect("packed BGR"), BGR, "{format}");
            }
        }
    }

    #[test]
    fn an_alpha_carrying_template_decodes_to_its_colour_channels_only() {
        let rgba: Vec<u8> = RGB
            .iter()
            .zip(ALPHA)
            .flat_map(|(rgb, alpha)| [rgb[0], rgb[1], rgb[2], alpha])
            .collect();
        let opaque = decode_to_bgr(
            &mado_pilot_testkit::png::encode_rgb(2, 2, RGB.as_flattened()),
            extent(),
        )
        .expect("an RGB PNG decodes");
        let translucent =
            decode_to_bgr(&mado_pilot_testkit::png::encode_rgba(2, 2, &rgba), extent())
                .expect("an RGBA PNG decodes");

        assert_eq!(translucent.channels(), 3);
        assert_eq!(opaque.data_bytes().expect("packed BGR"), BGR);
        assert_eq!(translucent.data_bytes().expect("packed BGR"), BGR);
    }

    #[test]
    fn content_that_is_not_an_image_is_refused() {
        assert_eq!(
            decode_to_bgr(b"not an image", extent()).err(),
            Some(ImageFault::Rejected)
        );
    }

    #[test]
    fn a_bare_signature_without_a_body_is_refused() {
        assert_eq!(
            decode_to_bgr(
                &[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'],
                extent()
            )
            .err(),
            Some(ImageFault::Rejected)
        );
    }

    #[test]
    fn empty_content_is_refused() {
        assert_eq!(
            decode_to_bgr(&[], extent()).err(),
            Some(ImageFault::Rejected)
        );
    }

    #[test]
    fn content_declaring_an_extent_the_metadata_does_not_is_refused() {
        // The content is a real, decodable two-by-two PNG. What makes it a
        // refusal is the disagreement with what it was loaded as, and the
        // distinct fault is what says the header was read rather than the
        // decoded image.
        assert_eq!(
            decode_to_bgr(&png(), PixelExtent::new(64, 64)).err(),
            Some(ImageFault::DeclaredExtentMismatch)
        );
    }

    #[test]
    fn a_header_declaring_far_more_than_it_carries_is_refused_before_the_decode() {
        // 30000 by 30000 is 2.7 GB of BGR pixels out of sixty-odd bytes. The
        // fault has to be the extent one: this content does not decode, so
        // `Rejected` would mean OpenCV was handed the header and allocated from
        // it, which is the whole thing being prevented.
        let bomb = mado_pilot_testkit::png::declared_without_body(30_000, 30_000);

        assert_eq!(
            decode_to_bgr(&bomb, extent()).err(),
            Some(ImageFault::DeclaredExtentMismatch)
        );
    }

    #[test]
    fn a_declaration_that_agrees_with_a_huge_header_is_still_refused_before_the_decode() {
        // The same 2.7 GB header, declared honestly. Agreement is all the
        // previous test proves, and agreement between two numbers an attacker
        // chose bounds nothing: this is the case where the metadata and the file
        // tell the same enormous story, and the ceiling is the only thing left
        // between it and an OpenCV allocation.
        let bomb = mado_pilot_testkit::png::declared_without_body(30_000, 30_000);

        assert_eq!(
            decode_to_bgr(&bomb, PixelExtent::new(30_000, 30_000)).err(),
            Some(ImageFault::AboveDecodeCeiling),
            "the fault has to be the ceiling: `Rejected` would mean imdecode was reached"
        );
    }

    #[test]
    fn the_decode_ceiling_is_the_contracts_pixel_ceiling_in_this_profiles_bytes() {
        // The ceiling is checked against the declared extent alone, so content
        // never enters it. An extent one pixel over is refused with no image at
        // all, and the multiplication that gets there cannot leave `u64` — the
        // largest extent a `PixelExtent` can hold is refused rather than wrapped
        // into a passing value.
        let over = super::MAX_DECODED_BYTES / super::BGR_CHANNELS + 1;
        let width = u32::try_from(over).expect("the ceiling is far below u32::MAX");

        assert_eq!(
            decode_to_bgr(&[], PixelExtent::new(width, 1)).err(),
            Some(ImageFault::AboveDecodeCeiling)
        );
        assert_eq!(
            decode_to_bgr(&[], PixelExtent::new(u32::MAX, u32::MAX)).err(),
            Some(ImageFault::AboveDecodeCeiling)
        );
    }
}
