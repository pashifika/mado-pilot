//! Bounded BGRA image views, profile preprocessing, and perspective crops.

use std::ffi::c_void;
use std::mem::size_of;

use mado_pilot_capture::{CpuMapping, PixelFormat};
use opencv::core::{
    BORDER_REPLICATE, CV_8UC4, Mat, MatTraitConst, MatTraitConstManual, Point2f,
    ROTATE_90_COUNTERCLOCKWISE, Scalar, Size, rotate,
};
opencv::opencv_branch_5! {
    use opencv::geometry::get_perspective_transform_slice_def;
}
opencv::not_opencv_branch_5! {
    use opencv::imgproc::get_perspective_transform_slice_def;
}
use opencv::imgproc::{INTER_CUBIC, INTER_LINEAR, resize, warp_perspective};

use crate::detect::Quad;
use crate::fault::OnnxBackendFault;

pub(crate) const MAX_TENSOR_BYTES: usize = 256 * 1024 * 1024;
pub(crate) const MAX_RECOGNIZER_WIDTH: usize = 4_096;
const DETECTOR_MIN_SIDE: u32 = 736;
const DETECTOR_MULTIPLE: u32 = 32;
const RECOGNIZER_HEIGHT: usize = 48;
const RECOGNIZER_BASE_WIDTH: usize = 320;
const CHANNELS: usize = 3;
const BGRA_BYTES: usize = 4;

#[derive(Debug)]
pub(crate) struct TensorInput {
    pub(crate) data: Vec<f32>,
    pub(crate) shape: [usize; 4],
}

/// Exposes one validated borrowed BGRA matrix only for the closure's duration.
pub(crate) fn with_bgra_view<T>(
    pixels: &CpuMapping,
    use_view: impl FnOnce(&Mat) -> Result<T, OnnxBackendFault>,
) -> Result<T, OnnxBackendFault> {
    let descriptor = pixels.descriptor();
    if descriptor.format() != PixelFormat::Bgra8 {
        return Err(OnnxBackendFault::InvalidPixels);
    }
    let extent = descriptor.extent();
    let rows = i32::try_from(extent.height()).map_err(|_| OnnxBackendFault::InvalidPixels)?;
    let columns = i32::try_from(extent.width()).map_err(|_| OnnxBackendFault::InvalidPixels)?;
    let stride = descriptor.stride();
    let row_bytes = usize::try_from(extent.width())
        .ok()
        .and_then(|width| width.checked_mul(BGRA_BYTES))
        .ok_or(OnnxBackendFault::InvalidPixels)?;
    let required = usize::try_from(extent.height())
        .ok()
        .and_then(|height| height.checked_sub(1))
        .and_then(|leading_rows| leading_rows.checked_mul(stride))
        .and_then(|leading| leading.checked_add(row_bytes))
        .ok_or(OnnxBackendFault::InvalidPixels)?;
    if stride < row_bytes || pixels.bytes().len() < required {
        return Err(OnnxBackendFault::InvalidPixels);
    }

    // SAFETY: the checked extent, stride, and final-row length fit inside the
    // borrowed mapping. The closure receives only `&Mat`; the header is dropped
    // before this function returns, while `pixels` still owns the bytes.
    let view = unsafe {
        Mat::new_rows_cols_with_data_unsafe(
            rows,
            columns,
            CV_8UC4,
            pixels.bytes().as_ptr().cast::<c_void>().cast_mut(),
            stride,
        )
    }
    .map_err(|_| OnnxBackendFault::NativeFailure)?;
    let result = use_view(&view);
    drop(view);
    result
}

pub(crate) fn detector_input(image: &Mat) -> Result<TensorInput, OnnxBackendFault> {
    let source_height = u32::try_from(image.rows()).map_err(|_| OnnxBackendFault::InvalidPixels)?;
    let source_width = u32::try_from(image.cols()).map_err(|_| OnnxBackendFault::InvalidPixels)?;
    if source_height == 0 || source_width == 0 {
        return Err(OnnxBackendFault::InvalidPixels);
    }

    let ratio = if source_height.min(source_width) < DETECTOR_MIN_SIDE {
        f64::from(DETECTOR_MIN_SIDE) / f64::from(source_height.min(source_width))
    } else {
        1.0
    };
    let scaled_height = f64_to_u32(f64::from(source_height) * ratio)?;
    let scaled_width = f64_to_u32(f64::from(source_width) * ratio)?;
    let height = round_to_multiple(scaled_height, DETECTOR_MULTIPLE)?;
    let width = round_to_multiple(scaled_width, DETECTOR_MULTIPLE)?;
    let height_usize = usize::try_from(height).map_err(|_| OnnxBackendFault::ResourceLimit)?;
    let width_usize = usize::try_from(width).map_err(|_| OnnxBackendFault::ResourceLimit)?;
    let elements = tensor_elements(1, height_usize, width_usize)?;

    let mut resized = Mat::default();
    resize(
        image,
        &mut resized,
        Size::new(
            i32::try_from(width).map_err(|_| OnnxBackendFault::ResourceLimit)?,
            i32::try_from(height).map_err(|_| OnnxBackendFault::ResourceLimit)?,
        ),
        0.0,
        0.0,
        INTER_LINEAR,
    )
    .map_err(|_| OnnxBackendFault::NativeFailure)?;
    let data = bgra_to_planar(&resized, 1, height_usize, width_usize, elements)?;
    Ok(TensorInput {
        data,
        shape: [1, CHANNELS, height_usize, width_usize],
    })
}

pub(crate) fn recognition_ratio(quad: &Quad) -> Result<f64, OnnxBackendFault> {
    let (width, height, rotate) = crop_dimensions(quad)?;
    let ratio = if rotate {
        f64::from(height) / f64::from(width)
    } else {
        f64::from(width) / f64::from(height)
    };
    if !ratio.is_finite() || ratio <= 0.0 {
        return Err(OnnxBackendFault::MalformedOutput);
    }
    Ok(ratio)
}

pub(crate) fn recognition_input(
    source: &Mat,
    quads: &[Quad],
) -> Result<TensorInput, OnnxBackendFault> {
    if quads.is_empty() || quads.len() > crate::RECOGNITION_BATCH {
        return Err(OnnxBackendFault::ResourceLimit);
    }

    let mut crops = Vec::with_capacity(quads.len());
    let mut total_crop_bytes = 0usize;
    let mut max_ratio = RECOGNIZER_BASE_WIDTH as f64 / RECOGNIZER_HEIGHT as f64;
    for quad in quads {
        let crop = perspective_crop(source, quad)?;
        let rows = usize::try_from(crop.rows()).map_err(|_| OnnxBackendFault::MalformedOutput)?;
        let columns =
            usize::try_from(crop.cols()).map_err(|_| OnnxBackendFault::MalformedOutput)?;
        let crop_bytes = rows
            .checked_mul(columns)
            .and_then(|pixels| pixels.checked_mul(BGRA_BYTES))
            .ok_or(OnnxBackendFault::ResourceLimit)?;
        total_crop_bytes = total_crop_bytes
            .checked_add(crop_bytes)
            .ok_or(OnnxBackendFault::ResourceLimit)?;
        if total_crop_bytes > MAX_TENSOR_BYTES {
            return Err(OnnxBackendFault::ResourceLimit);
        }
        max_ratio = max_ratio.max(columns as f64 / rows as f64);
        crops.push(crop);
    }

    let width = f64_to_usize(RECOGNIZER_HEIGHT as f64 * max_ratio)?;
    if width == 0 || width > MAX_RECOGNIZER_WIDTH {
        return Err(OnnxBackendFault::ResourceLimit);
    }
    let elements = tensor_elements(quads.len(), RECOGNIZER_HEIGHT, width)?;
    let mut data = vec![0.0_f32; elements];
    let plane = RECOGNIZER_HEIGHT
        .checked_mul(width)
        .ok_or(OnnxBackendFault::ResourceLimit)?;
    let batch_stride = plane
        .checked_mul(CHANNELS)
        .ok_or(OnnxBackendFault::ResourceLimit)?;

    for (batch_index, crop) in crops.iter().enumerate() {
        let crop_height =
            usize::try_from(crop.rows()).map_err(|_| OnnxBackendFault::MalformedOutput)?;
        let crop_width =
            usize::try_from(crop.cols()).map_err(|_| OnnxBackendFault::MalformedOutput)?;
        let ratio = crop_width as f64 / crop_height as f64;
        let resized_width = width.min(f64_to_usize((RECOGNIZER_HEIGHT as f64 * ratio).ceil())?);
        if resized_width == 0 {
            return Err(OnnxBackendFault::MalformedOutput);
        }
        let mut resized = Mat::default();
        resize(
            crop,
            &mut resized,
            Size::new(
                i32::try_from(resized_width).map_err(|_| OnnxBackendFault::ResourceLimit)?,
                i32::try_from(RECOGNIZER_HEIGHT).map_err(|_| OnnxBackendFault::ResourceLimit)?,
            ),
            0.0,
            0.0,
            INTER_LINEAR,
        )
        .map_err(|_| OnnxBackendFault::NativeFailure)?;
        fill_bgra_planar_padded(
            &resized,
            &mut data[batch_index * batch_stride..(batch_index + 1) * batch_stride],
            RECOGNIZER_HEIGHT,
            resized_width,
            width,
        )?;
    }

    Ok(TensorInput {
        data,
        shape: [quads.len(), CHANNELS, RECOGNIZER_HEIGHT, width],
    })
}

fn perspective_crop(source: &Mat, quad: &Quad) -> Result<Mat, OnnxBackendFault> {
    let (width, height, should_rotate) = crop_dimensions(quad)?;
    let target = [
        Point2f::new(0.0, 0.0),
        Point2f::new(width as f32, 0.0),
        Point2f::new(width as f32, height as f32),
        Point2f::new(0.0, height as f32),
    ];
    let transform = get_perspective_transform_slice_def(*quad, target)
        .map_err(|_| OnnxBackendFault::NativeFailure)?;
    let mut crop = Mat::default();
    let warp_result = opencv::opencv_branch_5! {{
        warp_perspective(
            source,
            &mut crop,
            &transform,
            Size::new(width, height),
            INTER_CUBIC,
            BORDER_REPLICATE,
            Scalar::default(),
            opencv::core::AlgorithmHint::ALGO_HINT_DEFAULT,
        )
    } else {
        warp_perspective(
            source,
            &mut crop,
            &transform,
            Size::new(width, height),
            INTER_CUBIC,
            BORDER_REPLICATE,
            Scalar::default(),
        )
    }};
    warp_result.map_err(|_| OnnxBackendFault::NativeFailure)?;
    if !should_rotate {
        return Ok(crop);
    }
    let mut rotated = Mat::default();
    rotate(&crop, &mut rotated, ROTATE_90_COUNTERCLOCKWISE)
        .map_err(|_| OnnxBackendFault::NativeFailure)?;
    Ok(rotated)
}

fn crop_dimensions(quad: &Quad) -> Result<(i32, i32, bool), OnnxBackendFault> {
    let width = f64_to_i32(distance(quad[0], quad[1]).max(distance(quad[2], quad[3])))?;
    let height = f64_to_i32(distance(quad[0], quad[3]).max(distance(quad[1], quad[2])))?;
    if width <= 0 || height <= 0 {
        return Err(OnnxBackendFault::MalformedOutput);
    }
    let bytes = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(BGRA_BYTES))
        .ok_or(OnnxBackendFault::ResourceLimit)?;
    if bytes > MAX_TENSOR_BYTES {
        return Err(OnnxBackendFault::ResourceLimit);
    }
    Ok((width, height, f64::from(height) / f64::from(width) >= 1.5))
}

fn distance(left: Point2f, right: Point2f) -> f64 {
    let x = f64::from(left.x - right.x);
    let y = f64::from(left.y - right.y);
    x.hypot(y)
}

fn round_to_multiple(value: u32, multiple: u32) -> Result<u32, OnnxBackendFault> {
    let rounded = (f64::from(value) / f64::from(multiple)).round_ties_even();
    let result = rounded * f64::from(multiple);
    if !result.is_finite() || result < f64::from(multiple) || result > f64::from(u32::MAX) {
        return Err(OnnxBackendFault::ResourceLimit);
    }
    f64_to_u32(result)
}

fn tensor_elements(batch: usize, height: usize, width: usize) -> Result<usize, OnnxBackendFault> {
    let elements = batch
        .checked_mul(CHANNELS)
        .and_then(|value| value.checked_mul(height))
        .and_then(|value| value.checked_mul(width))
        .ok_or(OnnxBackendFault::ResourceLimit)?;
    let bytes = elements
        .checked_mul(size_of::<f32>())
        .ok_or(OnnxBackendFault::ResourceLimit)?;
    if bytes > MAX_TENSOR_BYTES {
        return Err(OnnxBackendFault::ResourceLimit);
    }
    Ok(elements)
}

fn bgra_to_planar(
    image: &Mat,
    batch: usize,
    height: usize,
    width: usize,
    elements: usize,
) -> Result<Vec<f32>, OnnxBackendFault> {
    if batch != 1 {
        return Err(OnnxBackendFault::ResourceLimit);
    }
    let mut output = vec![0.0_f32; elements];
    fill_bgra_planar_padded(image, &mut output, height, width, width)?;
    Ok(output)
}

fn fill_bgra_planar_padded(
    image: &Mat,
    output: &mut [f32],
    height: usize,
    source_width: usize,
    padded_width: usize,
) -> Result<(), OnnxBackendFault> {
    if image.channels() != 4
        || image.rows() != i32::try_from(height).map_err(|_| OnnxBackendFault::ResourceLimit)?
        || image.cols()
            != i32::try_from(source_width).map_err(|_| OnnxBackendFault::ResourceLimit)?
        || !image.is_continuous()
    {
        return Err(OnnxBackendFault::NativeFailure);
    }
    let bytes = image
        .data_bytes()
        .map_err(|_| OnnxBackendFault::NativeFailure)?;
    let expected = height
        .checked_mul(source_width)
        .and_then(|pixels| pixels.checked_mul(BGRA_BYTES))
        .ok_or(OnnxBackendFault::ResourceLimit)?;
    if bytes.len() < expected {
        return Err(OnnxBackendFault::NativeFailure);
    }
    let plane = height
        .checked_mul(padded_width)
        .ok_or(OnnxBackendFault::ResourceLimit)?;
    if output.len()
        != plane
            .checked_mul(CHANNELS)
            .ok_or(OnnxBackendFault::ResourceLimit)?
    {
        return Err(OnnxBackendFault::ResourceLimit);
    }

    for row in 0..height {
        for column in 0..source_width {
            let pixel = (row * source_width + column) * BGRA_BYTES;
            let planar = row * padded_width + column;
            output[planar] = normalize(bytes[pixel]);
            output[plane + planar] = normalize(bytes[pixel + 1]);
            output[2 * plane + planar] = normalize(bytes[pixel + 2]);
        }
    }
    Ok(())
}

fn normalize(channel: u8) -> f32 {
    let scaled = f32::from(channel) / 255.0;
    let centered = scaled - 0.5;
    centered / 0.5
}

fn f64_to_u32(value: f64) -> Result<u32, OnnxBackendFault> {
    if !value.is_finite() || value < 0.0 || value > f64::from(u32::MAX) {
        return Err(OnnxBackendFault::ResourceLimit);
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the finite nonnegative value was range-checked and profile semantics require truncation"
    )]
    Ok(value as u32)
}

fn f64_to_usize(value: f64) -> Result<usize, OnnxBackendFault> {
    if !value.is_finite() || value < 0.0 || value > usize::MAX as f64 {
        return Err(OnnxBackendFault::ResourceLimit);
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the finite nonnegative value was range-checked and profile semantics require truncation"
    )]
    Ok(value as usize)
}

fn f64_to_i32(value: f64) -> Result<i32, OnnxBackendFault> {
    if !value.is_finite() || value < f64::from(i32::MIN) || value > f64::from(i32::MAX) {
        return Err(OnnxBackendFault::ResourceLimit);
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the finite crop dimension was range-checked and profile semantics require truncation"
    )]
    Ok(value as i32)
}
#[cfg(test)]
mod tests {
    use super::{
        DETECTOR_MULTIPLE, MAX_TENSOR_BYTES, normalize, round_to_multiple, tensor_elements,
    };
    use crate::fault::OnnxBackendFault;

    #[test]
    fn detector_rounding_matches_python_ties_to_even() {
        assert_eq!(round_to_multiple(752, DETECTOR_MULTIPLE), Ok(768));
        assert_eq!(round_to_multiple(720, DETECTOR_MULTIPLE), Ok(704));
    }

    #[test]
    fn tensor_ceiling_is_checked_before_allocation() {
        assert_eq!(
            tensor_elements(1, MAX_TENSOR_BYTES, MAX_TENSOR_BYTES),
            Err(OnnxBackendFault::ResourceLimit)
        );
    }

    #[test]
    fn normalization_matches_rapidocr_float32_operation_order() {
        assert_eq!(normalize(0), -1.0);
        assert_eq!(normalize(255), 1.0);
        let expected = ((128_f32 / 255.0) - 0.5) / 0.5;
        assert_eq!(normalize(128).to_bits(), expected.to_bits());
    }
}
