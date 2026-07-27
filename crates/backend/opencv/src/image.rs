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
use opencv::core::{CV_8UC4, Mat, MatTraitConst};
use opencv::imgcodecs::{IMREAD_COLOR, imdecode};
use opencv::imgproc::{COLOR_BGRA2BGR, cvt_color_def};

/// The pixel format the adapter asks the matcher to map a searched region into.
///
/// OpenCV's own channel order is blue-green-red, so asking for BGRA means the
/// matcher performs any channel swap once, while it is already copying, and the
/// adapter then only has to drop the alpha channel.
pub(crate) const REQUIRED_FORMAT: PixelFormat = PixelFormat::Bgra8;

/// Why a matrix could not be built.
///
/// Kept separate from the public vision faults because the two callers report it
/// differently: the same malformed input is a preparation failure while
/// compiling a template and a backend failure while searching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImageFault {
    /// A dimension or byte count does not fit what OpenCV accepts.
    Unrepresentable,
    /// The mapped pixels were not in the format the descriptor promised.
    UnexpectedFormat,
    /// The mapped pixels were shorter than their own descriptor requires.
    Truncated,
    /// OpenCV rejected the operation.
    Rejected,
}

/// Converts a mapped BGRA region into an owned three-channel BGR matrix.
///
/// # Errors
///
/// Returns [`ImageFault`] when the mapping contradicts its own descriptor, when
/// a dimension does not fit OpenCV's signed extents, or when OpenCV rejects the
/// conversion.
pub(crate) fn region_to_bgr(pixels: &CpuMapping) -> Result<Mat, ImageFault> {
    let descriptor = pixels.descriptor();
    if descriptor.format() != REQUIRED_FORMAT {
        return Err(ImageFault::UnexpectedFormat);
    }

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

    cvt_color_def(&view, &mut bgr, COLOR_BGRA2BGR).map_err(|_| ImageFault::Rejected)?;
    drop(view);

    Ok(bgr)
}

/// Decodes encoded template content into an owned three-channel BGR matrix.
///
/// `IMREAD_COLOR` is what makes the result comparable to a converted region: it
/// yields three 8-bit BGR channels whatever the file declared, so a greyscale or
/// palette PNG and an alpha-carrying one all arrive in one layout. A template's
/// alpha is dropped rather than honoured; masked matching is not part of the
/// Phase 1 profile.
///
/// # Errors
///
/// Returns [`ImageFault`] when the content is longer than OpenCV's signed
/// extents allow, when it does not decode, or when it decodes to something other
/// than a non-empty three-channel image.
pub(crate) fn decode_to_bgr(content: &[u8]) -> Result<Mat, ImageFault> {
    let length = i32::try_from(content.len()).map_err(|_| ImageFault::Unrepresentable)?;
    let encoded =
        Mat::new_rows_cols_with_data::<u8>(1, length, content).map_err(|_| ImageFault::Rejected)?;

    let decoded = imdecode(&encoded, IMREAD_COLOR).map_err(|_| ImageFault::Rejected)?;
    if decoded.empty() || decoded.channels() != 3 {
        return Err(ImageFault::Rejected);
    }

    Ok(decoded)
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
    use mado_pilot_core::PixelExtent;
    use opencv::core::MatTraitConst;

    use super::{ImageFault, decode_to_bgr, extent_of};

    /// A two-by-two PNG, so the decode path is exercised against real bytes
    /// rather than a signature.
    fn png() -> Vec<u8> {
        mado_pilot_testkit::png::solid_rgb(2, 2, [10, 20, 30])
    }

    #[test]
    fn encoded_content_decodes_to_three_bgr_channels() {
        let decoded = decode_to_bgr(&png()).expect("a valid PNG decodes");

        assert_eq!(decoded.channels(), 3);
        assert_eq!(
            extent_of(&decoded).expect("a real extent"),
            PixelExtent::new(2, 2)
        );
    }

    #[test]
    fn content_that_is_not_an_image_is_refused() {
        assert_eq!(
            decode_to_bgr(b"not an image").err(),
            Some(ImageFault::Rejected)
        );
    }

    #[test]
    fn a_bare_signature_without_a_body_is_refused() {
        assert_eq!(
            decode_to_bgr(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']).err(),
            Some(ImageFault::Rejected)
        );
    }

    #[test]
    fn empty_content_is_refused() {
        assert_eq!(decode_to_bgr(&[]).err(), Some(ImageFault::Rejected));
    }
}
