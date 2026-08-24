//! Accepted RapidOCR DB detector postprocessing with bounded OpenCV work.

use std::mem::size_of;

use opencv::core::{
    self, BORDER_CONSTANT, CV_8UC1, Mat, MatExprTraitConst, MatTraitConst, MatTraitConstManual,
    Point, Point2f, Rect, Scalar, Vector,
};
opencv::opencv_branch_5! {
    use opencv::geometry::min_area_rect;
}
opencv::not_opencv_branch_5! {
    use opencv::imgproc::min_area_rect;
}
use opencv::imgproc::{
    CHAIN_APPROX_SIMPLE, RETR_LIST, dilate, fill_convex_poly_def, find_contours_def,
    morphology_default_border_value,
};

use crate::fault::OnnxBackendFault;

pub(crate) const MAX_DETECTOR_CANDIDATES: usize = 1_000;
const THRESHOLD: f32 = 0.3;
const BOX_THRESHOLD: f64 = 0.5;
const UNCLIP_RATIO: f64 = 1.6;
const MIN_SIDE: f32 = 3.0;
const SORT_LINE_THRESHOLD: f32 = 10.0;
const MAX_COMPONENT_STACK_BYTES: usize = 16 * 1024 * 1024;
const MAX_CONTOUR_STORAGE_BYTES: usize = 2 * 1024 * 1024;
const CONTOUR_VECTOR_OVERHEAD_BYTES: usize = 64;

pub(crate) type Quad = [Point2f; 4];

#[derive(Debug, Clone)]
pub(crate) struct Detection {
    pub(crate) quad: Quad,
    pub(crate) order: u32,
}

pub(crate) fn postprocess(
    shape: &[i64],
    probability: &[f32],
    source_width: u32,
    source_height: u32,
    request_ceiling: usize,
) -> Result<Vec<Detection>, OnnxBackendFault> {
    if shape.len() != 4 || shape[0] != 1 || shape[1] != 1 || shape[2] <= 0 || shape[3] <= 0 {
        return Err(OnnxBackendFault::MalformedOutput);
    }
    let height = usize::try_from(shape[2]).map_err(|_| OnnxBackendFault::MalformedOutput)?;
    let width = usize::try_from(shape[3]).map_err(|_| OnnxBackendFault::MalformedOutput)?;
    let elements = height
        .checked_mul(width)
        .ok_or(OnnxBackendFault::ResourceLimit)?;
    if probability.len() != elements || source_width == 0 || source_height == 0 {
        return Err(OnnxBackendFault::MalformedOutput);
    }
    let output_bytes = elements
        .checked_mul(size_of::<f32>())
        .ok_or(OnnxBackendFault::ResourceLimit)?;
    if output_bytes > crate::MAX_OUTPUT_BYTES {
        return Err(OnnxBackendFault::ResourceLimit);
    }

    let mut binary = Vec::with_capacity(elements);
    for value in probability.iter().copied() {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(OnnxBackendFault::MalformedOutput);
        }
        binary.push(u8::from(value > THRESHOLD).saturating_mul(255));
    }

    let rows = i32::try_from(height).map_err(|_| OnnxBackendFault::ResourceLimit)?;
    let columns = i32::try_from(width).map_err(|_| OnnxBackendFault::ResourceLimit)?;
    let bitmap = Mat::new_rows_cols_with_data(rows, columns, binary.as_slice())
        .map_err(|_| OnnxBackendFault::NativeFailure)?;
    let kernel_data = [1_u8; 4];
    let kernel = Mat::new_rows_cols_with_data(2, 2, kernel_data.as_slice())
        .map_err(|_| OnnxBackendFault::NativeFailure)?;
    let mut dilated = Mat::default();
    dilate(
        &bitmap,
        &mut dilated,
        &kernel,
        Point::new(-1, -1),
        1,
        BORDER_CONSTANT,
        morphology_default_border_value().map_err(|_| OnnxBackendFault::NativeFailure)?,
    )
    .map_err(|_| OnnxBackendFault::NativeFailure)?;
    drop(kernel);
    drop(bitmap);
    drop(binary);
    bounded_contour_preflight(&dilated, width, height)?;

    let mut contours = Vector::<Vector<Point>>::new();
    find_contours_def(&dilated, &mut contours, RETR_LIST, CHAIN_APPROX_SIMPLE)
        .map_err(|_| OnnxBackendFault::NativeFailure)?;
    let probability_mat = Mat::new_rows_cols_with_data(rows, columns, probability)
        .map_err(|_| OnnxBackendFault::NativeFailure)?;

    let contour_ceiling = contours
        .len()
        .min(MAX_DETECTOR_CANDIDATES)
        .min(request_ceiling);
    let mut quads = Vec::with_capacity(contour_ceiling);
    for index in 0..contour_ceiling {
        let contour = contours
            .get(index)
            .map_err(|_| OnnxBackendFault::NativeFailure)?;
        let (quad, short_side) = mini_box(&contour)?;
        if short_side < MIN_SIDE {
            continue;
        }
        let score = box_score_fast(&probability_mat, width, height, &quad)?;
        if score < BOX_THRESHOLD {
            continue;
        }
        let expanded = expand_rectangle(&quad)?;
        let expanded_short =
            distance(expanded[0], expanded[3]).min(distance(expanded[0], expanded[1]));
        if expanded_short < f64::from(MIN_SIDE + 2.0) {
            continue;
        }
        let mut scaled = scale_quad(&expanded, width, height, source_width, source_height)?;
        order_clockwise(&mut scaled);
        clip_quad(&mut scaled, source_width, source_height);
        // RapidOCR truncates these positive lengths to an integer before the
        // `<= 3` check; that is exactly the same predicate as `< 4.0`.
        if distance(scaled[0], scaled[1]) < 4.0 || distance(scaled[0], scaled[3]) < 4.0 {
            continue;
        }
        quads.push(scaled);
    }

    sort_reading_order(&mut quads);
    quads
        .into_iter()
        .enumerate()
        .map(|(index, quad)| {
            Ok(Detection {
                quad,
                order: u32::try_from(index).map_err(|_| OnnxBackendFault::ResourceLimit)?,
            })
        })
        .collect()
}
fn bounded_contour_preflight(
    bitmap: &Mat,
    width: usize,
    height: usize,
) -> Result<(), OnnxBackendFault> {
    let elements = width
        .checked_mul(height)
        .ok_or(OnnxBackendFault::ResourceLimit)?;
    if !bitmap.is_continuous() {
        return Err(OnnxBackendFault::NativeFailure);
    }
    let pixels = bitmap
        .data_bytes()
        .map_err(|_| OnnxBackendFault::NativeFailure)?;
    if pixels.len() < elements || elements > u32::MAX as usize {
        return Err(OnnxBackendFault::ResourceLimit);
    }
    bounded_boundary_storage(pixels, width, height)?;

    // Count four-connected foreground and background components. Every RETR_LIST
    // contour is either a foreground component boundary or a background hole,
    // so `components - 1` is a conservative pre-call contour bound. Scanline
    // flood fill keeps the work queue bounded by runs rather than pixels.
    let mut state = vec![0_u8; elements];
    let mut stack = Vec::<u32>::new();
    let stack_ceiling = MAX_COMPONENT_STACK_BYTES / size_of::<u32>();
    let mut components = 0usize;
    for start in 0..elements {
        if state[start] != 0 {
            continue;
        }
        components += 1;
        if components > MAX_DETECTOR_CANDIDATES + 1 {
            return Err(OnnxBackendFault::ResourceLimit);
        }
        state[start] = 2;
        stack.push(u32::try_from(start).map_err(|_| OnnxBackendFault::ResourceLimit)?);
        while let Some(seed) = stack.pop() {
            let seed = usize::try_from(seed).map_err(|_| OnnxBackendFault::ResourceLimit)?;
            let value = pixels[seed];
            let row = seed / width;
            let column = seed % width;
            let row_start = row * width;
            let mut left = column;
            while left > 0
                && state[row_start + left - 1] != 1
                && pixels[row_start + left - 1] == value
            {
                left -= 1;
            }
            let mut right = column;
            while right + 1 < width
                && state[row_start + right + 1] != 1
                && pixels[row_start + right + 1] == value
            {
                right += 1;
            }
            state[row_start + left..=row_start + right].fill(1);

            for neighbor_row in [row.checked_sub(1), (row + 1 < height).then_some(row + 1)]
                .into_iter()
                .flatten()
            {
                let neighbor_start = neighbor_row * width;
                let mut scan = left;
                while scan <= right {
                    let neighbor = neighbor_start + scan;
                    if state[neighbor] == 0 && pixels[neighbor] == value {
                        let run_start = scan;
                        while scan <= right
                            && state[neighbor_start + scan] == 0
                            && pixels[neighbor_start + scan] == value
                        {
                            state[neighbor_start + scan] = 2;
                            scan += 1;
                        }
                        if stack.len() >= stack_ceiling {
                            return Err(OnnxBackendFault::ResourceLimit);
                        }
                        stack.push(
                            u32::try_from(neighbor_start + run_start)
                                .map_err(|_| OnnxBackendFault::ResourceLimit)?,
                        );
                    } else {
                        scan += 1;
                    }
                }
            }
        }
    }
    Ok(())
}

fn bounded_boundary_storage(
    pixels: &[u8],
    width: usize,
    height: usize,
) -> Result<(), OnnxBackendFault> {
    let reserved_overhead = (MAX_DETECTOR_CANDIDATES + 1)
        .checked_mul(CONTOUR_VECTOR_OVERHEAD_BYTES)
        .ok_or(OnnxBackendFault::ResourceLimit)?;
    let point_budget = MAX_CONTOUR_STORAGE_BYTES
        .checked_sub(reserved_overhead)
        .ok_or(OnnxBackendFault::ResourceLimit)?;
    // One boundary edge can yield at most one CHAIN_APPROX_NONE point and
    // CHAIN_APPROX_SIMPLE can only remove points. Four times Point size covers
    // vector spare capacity, a reallocation's old buffer, and native bookkeeping.
    let bytes_per_edge = size_of::<Point>()
        .checked_mul(4)
        .ok_or(OnnxBackendFault::ResourceLimit)?;
    let edge_ceiling = point_budget / bytes_per_edge;
    let mut boundary_edges = 0usize;
    for row in 0..height {
        for column in 0..width {
            let index = row * width + column;
            if pixels[index] == 0 {
                continue;
            }
            let edges = usize::from(row == 0 || pixels[index - width] == 0)
                + usize::from(row + 1 == height || pixels[index + width] == 0)
                + usize::from(column == 0 || pixels[index - 1] == 0)
                + usize::from(column + 1 == width || pixels[index + 1] == 0);
            boundary_edges = boundary_edges
                .checked_add(edges)
                .ok_or(OnnxBackendFault::ResourceLimit)?;
            if boundary_edges > edge_ceiling {
                return Err(OnnxBackendFault::ResourceLimit);
            }
        }
    }
    Ok(())
}

fn mini_box(contour: &Vector<Point>) -> Result<(Quad, f32), OnnxBackendFault> {
    let rectangle = min_area_rect(contour).map_err(|_| OnnxBackendFault::NativeFailure)?;
    let mut points = [Point2f::default(); 4];
    rectangle
        .points(&mut points)
        .map_err(|_| OnnxBackendFault::NativeFailure)?;
    let mut by_x = points;
    by_x.sort_by(|left, right| left.x.total_cmp(&right.x));
    let (left_top, left_bottom) = if by_x[0].y <= by_x[1].y {
        (by_x[0], by_x[1])
    } else {
        (by_x[1], by_x[0])
    };
    let (right_top, right_bottom) = if by_x[2].y <= by_x[3].y {
        (by_x[2], by_x[3])
    } else {
        (by_x[3], by_x[2])
    };
    Ok((
        [left_top, right_top, right_bottom, left_bottom],
        rectangle.size.width.min(rectangle.size.height),
    ))
}

fn box_score_fast(
    probability: &impl MatTraitConst,
    width: usize,
    height: usize,
    quad: &Quad,
) -> Result<f64, OnnxBackendFault> {
    let max_x = i32::try_from(width - 1).map_err(|_| OnnxBackendFault::ResourceLimit)?;
    let max_y = i32::try_from(height - 1).map_err(|_| OnnxBackendFault::ResourceLimit)?;
    let xmin = f32_to_i32(
        quad.iter()
            .map(|point| point.x)
            .fold(f32::INFINITY, f32::min)
            .floor(),
    )?
    .clamp(0, max_x);
    let xmax = f32_to_i32(
        quad.iter()
            .map(|point| point.x)
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil(),
    )?
    .clamp(0, max_x);
    let ymin = f32_to_i32(
        quad.iter()
            .map(|point| point.y)
            .fold(f32::INFINITY, f32::min)
            .floor(),
    )?
    .clamp(0, max_y);
    let ymax = f32_to_i32(
        quad.iter()
            .map(|point| point.y)
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil(),
    )?
    .clamp(0, max_y);
    let roi_width = xmax - xmin + 1;
    let roi_height = ymax - ymin + 1;
    if roi_width <= 0 || roi_height <= 0 {
        return Err(OnnxBackendFault::MalformedOutput);
    }

    let mut mask = Mat::zeros(roi_height, roi_width, CV_8UC1)
        .map_err(|_| OnnxBackendFault::NativeFailure)?
        .to_mat()
        .map_err(|_| OnnxBackendFault::NativeFailure)?;
    let mut points = Vector::<Point>::with_capacity(quad.len());
    for point in quad {
        points.push(Point::new(
            f32_to_i32(point.x - xmin as f32)?,
            f32_to_i32(point.y - ymin as f32)?,
        ));
    }
    fill_convex_poly_def(&mut mask, &points, Scalar::all(1.0))
        .map_err(|_| OnnxBackendFault::NativeFailure)?;
    let roi = Mat::roi(probability, Rect::new(xmin, ymin, roi_width, roi_height))
        .map_err(|_| OnnxBackendFault::NativeFailure)?;
    let score = core::mean(&roi, &mask).map_err(|_| OnnxBackendFault::NativeFailure)?[0];
    if score.is_finite() {
        Ok(score)
    } else {
        Err(OnnxBackendFault::MalformedOutput)
    }
}

fn expand_rectangle(quad: &Quad) -> Result<Quad, OnnxBackendFault> {
    const ARC_TOLERANCE: f64 = 0.25;
    const MAX_UNCLIP_POINTS: usize = 65_536;

    let area = polygon_area(quad);
    let perimeter = (0..quad.len())
        .map(|index| distance(quad[index], quad[(index + 1) % quad.len()]))
        .sum::<f64>();
    if !area.is_finite() || area <= 0.0 || !perimeter.is_finite() || perimeter <= 0.0 {
        return Err(OnnxBackendFault::MalformedOutput);
    }
    let delta = area * UNCLIP_RATIO / perimeter;
    let mut source = quad
        .iter()
        .map(|point| Ok((f32_to_i64(point.x)?, f32_to_i64(point.y)?)))
        .collect::<Result<Vec<_>, OnnxBackendFault>>()?;
    if !clipper_orientation(&source) {
        source.reverse();
    }
    let normals = (0..source.len())
        .map(|index| unit_normal(source[index], source[(index + 1) % source.len()]))
        .collect::<Result<Vec<_>, OnnxBackendFault>>()?;

    // Clipper 6.4.2's default JT_ROUND setup. The accepted RapidOCR source
    // converts mini-box coordinates to IntPoint by truncation, uses the default
    // 0.25 arc tolerance, and rounds generated vertices half away from zero.
    let tolerance = ARC_TOLERANCE.min(delta * ARC_TOLERANCE);
    let mut total_steps = std::f64::consts::PI / (1.0 - tolerance / delta).acos();
    total_steps = total_steps.min(delta * std::f64::consts::PI);
    if !total_steps.is_finite() || total_steps <= 0.0 {
        return Err(OnnxBackendFault::MalformedOutput);
    }
    let sin = (std::f64::consts::TAU / total_steps).sin();
    let cos = (std::f64::consts::TAU / total_steps).cos();
    let steps_per_radian = total_steps / std::f64::consts::TAU;
    let mut expanded = Vector::<Point>::new();
    let mut previous = source.len() - 1;
    for current in 0..source.len() {
        let sin_angle = (normals[previous].0 * normals[current].1
            - normals[current].0 * normals[previous].1)
            .clamp(-1.0, 1.0);
        let dot =
            normals[previous].0 * normals[current].0 + normals[previous].1 * normals[current].1;
        let angle = sin_angle.atan2(dot);
        let corner_steps = usize::try_from(clipper_round(steps_per_radian * angle.abs())?.max(1))
            .map_err(|_| OnnxBackendFault::ResourceLimit)?;
        if expanded
            .len()
            .checked_add(corner_steps + 1)
            .is_none_or(|points| points > MAX_UNCLIP_POINTS)
        {
            return Err(OnnxBackendFault::ResourceLimit);
        }

        let (mut x, mut y) = normals[previous];
        for _ in 0..corner_steps {
            expanded.push(offset_point(source[current], x, y, delta)?);
            let old_x = x;
            x = x * cos - sin * y;
            y = old_x * sin + y * cos;
        }
        expanded.push(offset_point(
            source[current],
            normals[current].0,
            normals[current].1,
            delta,
        )?);
        previous = current;
    }
    mini_box(&expanded).map(|(box_points, _)| box_points)
}

fn polygon_area(quad: &Quad) -> f64 {
    let twice_area = (0..quad.len())
        .map(|index| {
            let current = quad[index];
            let next = quad[(index + 1) % quad.len()];
            f64::from(current.x) * f64::from(next.y) - f64::from(next.x) * f64::from(current.y)
        })
        .sum::<f64>();
    twice_area.abs() / 2.0
}

fn clipper_orientation(path: &[(i64, i64)]) -> bool {
    let mut area = 0.0_f64;
    let mut previous = path.len() - 1;
    for current in 0..path.len() {
        area += (path[previous].0 + path[current].0) as f64
            * (path[previous].1 - path[current].1) as f64;
        previous = current;
    }
    -area * 0.5 >= 0.0
}

fn unit_normal(first: (i64, i64), second: (i64, i64)) -> Result<(f64, f64), OnnxBackendFault> {
    let x = (second.0 - first.0) as f64;
    let y = (second.1 - first.1) as f64;
    let length = x.hypot(y);
    if !length.is_finite() || length <= 0.0 {
        return Err(OnnxBackendFault::MalformedOutput);
    }
    Ok((y / length, -x / length))
}

fn offset_point(
    source: (i64, i64),
    normal_x: f64,
    normal_y: f64,
    delta: f64,
) -> Result<Point, OnnxBackendFault> {
    let x = clipper_round(source.0 as f64 + normal_x * delta)?;
    let y = clipper_round(source.1 as f64 + normal_y * delta)?;
    Ok(Point::new(
        i32::try_from(x).map_err(|_| OnnxBackendFault::ResourceLimit)?,
        i32::try_from(y).map_err(|_| OnnxBackendFault::ResourceLimit)?,
    ))
}

fn clipper_round(value: f64) -> Result<i64, OnnxBackendFault> {
    let adjusted = if value < 0.0 {
        value - 0.5
    } else {
        value + 0.5
    };
    f64_to_i64(adjusted)
}

fn scale_quad(
    quad: &Quad,
    map_width: usize,
    map_height: usize,
    source_width: u32,
    source_height: u32,
) -> Result<Quad, OnnxBackendFault> {
    let mut scaled = [Point2f::default(); 4];
    for (destination, point) in scaled.iter_mut().zip(quad) {
        let x = (f64::from(point.x) / map_width as f64 * f64::from(source_width))
            .round_ties_even()
            .clamp(0.0, f64::from(source_width));
        let y = (f64::from(point.y) / map_height as f64 * f64::from(source_height))
            .round_ties_even()
            .clamp(0.0, f64::from(source_height));
        *destination = Point2f::new(f64_to_f32(x)?, f64_to_f32(y)?);
    }
    Ok(scaled)
}

fn order_clockwise(quad: &mut Quad) {
    let mut by_x = *quad;
    by_x.sort_by(|left, right| left.x.total_cmp(&right.x));
    let mut left = [by_x[0], by_x[1]];
    let mut right = [by_x[2], by_x[3]];
    left.sort_by(|first, second| first.y.total_cmp(&second.y));
    right.sort_by(|first, second| first.y.total_cmp(&second.y));
    *quad = [left[0], right[0], right[1], left[1]];
}

fn clip_quad(quad: &mut Quad, width: u32, height: u32) {
    let max_x = width.saturating_sub(1) as f32;
    let max_y = height.saturating_sub(1) as f32;
    for point in quad {
        point.x = point.x.clamp(0.0, max_x).trunc();
        point.y = point.y.clamp(0.0, max_y).trunc();
    }
}

fn sort_reading_order(quads: &mut [Quad]) {
    quads.sort_by(|left, right| left[0].y.total_cmp(&right[0].y));
    let mut start = 0usize;
    while start < quads.len() {
        let mut end = start + 1;
        while end < quads.len() && quads[end][0].y - quads[end - 1][0].y < SORT_LINE_THRESHOLD {
            end += 1;
        }
        quads[start..end].sort_by(|left, right| left[0].x.total_cmp(&right[0].x));
        start = end;
    }
}

fn distance(left: Point2f, right: Point2f) -> f64 {
    f64::from(left.x - right.x).hypot(f64::from(left.y - right.y))
}

fn f32_to_i32(value: f32) -> Result<i32, OnnxBackendFault> {
    if !value.is_finite() || value < i32::MIN as f32 || value > i32::MAX as f32 {
        return Err(OnnxBackendFault::MalformedOutput);
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the finite value was range-checked and profile semantics require truncation"
    )]
    Ok(value as i32)
}

fn f64_to_f32(value: f64) -> Result<f32, OnnxBackendFault> {
    if !value.is_finite() || value < f64::from(f32::MIN) || value > f64::from(f32::MAX) {
        return Err(OnnxBackendFault::MalformedOutput);
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the finite coordinate was checked against the complete f32 range"
    )]
    Ok(value as f32)
}

fn f32_to_i64(value: f32) -> Result<i64, OnnxBackendFault> {
    f32_to_i32(value).map(i64::from)
}

fn f64_to_i64(value: f64) -> Result<i64, OnnxBackendFault> {
    if !value.is_finite() || value < i64::MIN as f64 || value > i64::MAX as f64 {
        return Err(OnnxBackendFault::ResourceLimit);
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the finite Clipper coordinate was checked against the complete i64 range"
    )]
    Ok(value as i64)
}
#[cfg(test)]
mod tests {
    use super::postprocess;

    #[test]
    fn bounded_probability_map_produces_one_ordered_box() {
        let mut probability = vec![0.0_f32; 32 * 32];
        for row in 8..24 {
            for column in 6..26 {
                probability[row * 32 + column] = 0.9;
            }
        }
        let detections = postprocess(&[1, 1, 32, 32], &probability, 32, 32, 1000)
            .expect("valid probability map");

        assert_eq!(detections.len(), 1);
        assert_eq!(detections[0].order, 0);
        assert!(detections[0].quad[0].x <= detections[0].quad[1].x);
        assert!(detections[0].quad[0].y <= detections[0].quad[3].y);
    }

    #[test]
    fn probability_outside_sigmoid_range_is_rejected() {
        assert!(matches!(
            postprocess(&[1, 1, 1, 1], &[1.1], 1, 1, 1),
            Err(crate::fault::OnnxBackendFault::MalformedOutput)
        ));
    }

    #[test]
    fn disconnected_mask_is_refused_before_native_contour_materialization() {
        let mut probability = vec![0.0_f32; 256 * 256];
        for row in (0..256).step_by(4) {
            for column in (0..256).step_by(4) {
                probability[row * 256 + column] = 0.9;
            }
        }
        assert!(matches!(
            postprocess(&[1, 1, 256, 256], &probability, 256, 256, 1000),
            Err(crate::fault::OnnxBackendFault::ResourceLimit)
        ));
    }

    #[test]
    fn one_serpentine_component_is_bounded_by_total_contour_storage() {
        let mut probability = vec![0.0_f32; 1024 * 1024];
        for row in (0..1024).step_by(4) {
            probability[row * 1024..(row + 1) * 1024].fill(0.9);
        }
        for row in 0..1024 {
            probability[row * 1024] = 0.9;
        }
        assert!(matches!(
            postprocess(&[1, 1, 1024, 1024], &probability, 1024, 1024, 1000),
            Err(crate::fault::OnnxBackendFault::ResourceLimit)
        ));
    }
}
