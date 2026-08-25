//! Closed preprocessing selection and checked detector geometry.

use std::mem::size_of;

use mado_pilot_ocr::{ACCEPTED_BOUNDED_MODEL_ID, ACCEPTED_G004_MODEL_ID, OcrModelIdentity};

use crate::fault::{OnnxBackendFault, OnnxOcrProfile};

const DETECTOR_MIN_SIDE: u32 = 736;
const DETECTOR_MULTIPLE: u32 = 32;
pub(crate) const BOUNDED_MAX_WIDTH: u32 = 1_312;
pub(crate) const BOUNDED_MAX_HEIGHT: u32 = 736;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectedProfile {
    NativeG004,
    BoundedDetector,
}

impl SelectedProfile {
    pub(crate) fn from_identity(identity: &OcrModelIdentity) -> Result<Self, OnnxBackendFault> {
        match identity.model().as_str() {
            ACCEPTED_G004_MODEL_ID => Ok(Self::NativeG004),
            ACCEPTED_BOUNDED_MODEL_ID => Ok(Self::BoundedDetector),
            _ => Err(OnnxBackendFault::ProfileMismatch),
        }
    }

    pub(crate) const fn public(self) -> OnnxOcrProfile {
        match self {
            Self::NativeG004 => OnnxOcrProfile::NativeG004,
            Self::BoundedDetector => OnnxOcrProfile::BoundedDetector,
        }
    }

    pub(crate) const fn preprocessing(self) -> PreprocessingDescriptor {
        match self {
            Self::NativeG004 => PreprocessingDescriptor {
                selected: self,
                min_side: DETECTOR_MIN_SIDE,
                multiple: DETECTOR_MULTIPLE,
                max_width: None,
                max_height: None,
            },
            Self::BoundedDetector => PreprocessingDescriptor {
                selected: self,
                min_side: DETECTOR_MIN_SIDE,
                multiple: DETECTOR_MULTIPLE,
                max_width: Some(BOUNDED_MAX_WIDTH),
                max_height: Some(BOUNDED_MAX_HEIGHT),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PreprocessingDescriptor {
    selected: SelectedProfile,
    min_side: u32,
    multiple: u32,
    max_width: Option<u32>,
    max_height: Option<u32>,
}

impl PreprocessingDescriptor {
    pub(crate) const fn selected(self) -> SelectedProfile {
        self.selected
    }

    pub(crate) const fn max_width(self) -> Option<u32> {
        self.max_width
    }

    pub(crate) const fn max_height(self) -> Option<u32> {
        self.max_height
    }

    pub(crate) fn plan(
        self,
        source_width: u32,
        source_height: u32,
    ) -> Result<DetectorPlan, OnnxBackendFault> {
        if source_width == 0 || source_height == 0 {
            return Err(OnnxBackendFault::InvalidPixels);
        }

        let short_side = source_width.min(source_height);
        let ratio = if short_side < self.min_side {
            f64::from(self.min_side) / f64::from(short_side)
        } else {
            1.0
        };
        let desired_width =
            round_to_multiple(f64_to_u32(f64::from(source_width) * ratio)?, self.multiple)?;
        let desired_height =
            round_to_multiple(f64_to_u32(f64::from(source_height) * ratio)?, self.multiple)?;

        let (final_width, final_height) = match (self.max_width, self.max_height) {
            (Some(max_width), Some(max_height))
                if desired_width > max_width || desired_height > max_height =>
            {
                let fit = (f64::from(max_width) / f64::from(desired_width))
                    .min(f64::from(max_height) / f64::from(desired_height));
                let width = round_with_ceiling(
                    f64_to_u32(f64::from(desired_width) * fit)?,
                    self.multiple,
                    max_width,
                )?;
                let height = round_with_ceiling(
                    f64_to_u32(f64::from(desired_height) * fit)?,
                    self.multiple,
                    max_height,
                )?;
                (width, height)
            }
            (Some(max_width), Some(max_height)) => {
                if desired_width > max_width || desired_height > max_height {
                    return Err(OnnxBackendFault::ResourceLimit);
                }
                (desired_width, desired_height)
            }
            (None, None) => (desired_width, desired_height),
            _ => return Err(OnnxBackendFault::ResourceLimit),
        };

        DetectorPlan::checked(source_width, source_height, final_width, final_height)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DetectorPlan {
    source_width: u32,
    source_height: u32,
    final_width: u32,
    final_height: u32,
    forward_x: f64,
    forward_y: f64,
    inverse_x: f64,
    inverse_y: f64,
    tensor_elements: usize,
    tensor_bytes: usize,
}

impl DetectorPlan {
    fn checked(
        source_width: u32,
        source_height: u32,
        final_width: u32,
        final_height: u32,
    ) -> Result<Self, OnnxBackendFault> {
        if source_width == 0 || source_height == 0 || final_width == 0 || final_height == 0 {
            return Err(OnnxBackendFault::InvalidPixels);
        }
        let width = usize::try_from(final_width).map_err(|_| OnnxBackendFault::ResourceLimit)?;
        let height = usize::try_from(final_height).map_err(|_| OnnxBackendFault::ResourceLimit)?;
        let tensor_elements = 3_usize
            .checked_mul(width)
            .and_then(|elements| elements.checked_mul(height))
            .ok_or(OnnxBackendFault::ResourceLimit)?;
        let tensor_bytes = tensor_elements
            .checked_mul(size_of::<f32>())
            .ok_or(OnnxBackendFault::ResourceLimit)?;
        if tensor_bytes > crate::image::MAX_TENSOR_BYTES {
            return Err(OnnxBackendFault::ResourceLimit);
        }

        let forward_x = f64::from(final_width) / f64::from(source_width);
        let forward_y = f64::from(final_height) / f64::from(source_height);
        let inverse_x = f64::from(source_width) / f64::from(final_width);
        let inverse_y = f64::from(source_height) / f64::from(final_height);
        if [forward_x, forward_y, inverse_x, inverse_y]
            .into_iter()
            .any(|scale| !scale.is_finite() || scale <= 0.0)
        {
            return Err(OnnxBackendFault::ResourceLimit);
        }

        Ok(Self {
            source_width,
            source_height,
            final_width,
            final_height,
            forward_x,
            forward_y,
            inverse_x,
            inverse_y,
            tensor_elements,
            tensor_bytes,
        })
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        source_width: u32,
        source_height: u32,
        final_width: u32,
        final_height: u32,
    ) -> Result<Self, OnnxBackendFault> {
        Self::checked(source_width, source_height, final_width, final_height)
    }

    pub(crate) const fn source_width(self) -> u32 {
        self.source_width
    }

    pub(crate) const fn source_height(self) -> u32 {
        self.source_height
    }

    pub(crate) const fn final_width(self) -> u32 {
        self.final_width
    }

    pub(crate) const fn final_height(self) -> u32 {
        self.final_height
    }

    pub(crate) const fn forward_x(self) -> f64 {
        self.forward_x
    }

    pub(crate) const fn forward_y(self) -> f64 {
        self.forward_y
    }

    pub(crate) const fn inverse_x(self) -> f64 {
        self.inverse_x
    }

    pub(crate) const fn inverse_y(self) -> f64 {
        self.inverse_y
    }

    pub(crate) const fn tensor_elements(self) -> usize {
        self.tensor_elements
    }

    pub(crate) const fn tensor_bytes(self) -> usize {
        self.tensor_bytes
    }
}

fn round_with_ceiling(value: u32, multiple: u32, ceiling: u32) -> Result<u32, OnnxBackendFault> {
    let rounded = round_to_multiple(value, multiple)?;
    if rounded <= ceiling {
        return Ok(rounded);
    }
    let bounded = ceiling - ceiling % multiple;
    if bounded < multiple {
        return Err(OnnxBackendFault::ResourceLimit);
    }
    Ok(bounded)
}

fn round_to_multiple(value: u32, multiple: u32) -> Result<u32, OnnxBackendFault> {
    if multiple == 0 {
        return Err(OnnxBackendFault::ResourceLimit);
    }
    let rounded = (f64::from(value) / f64::from(multiple)).round_ties_even();
    let result = rounded * f64::from(multiple);
    if !result.is_finite() || result < f64::from(multiple) || result > f64::from(u32::MAX) {
        return Err(OnnxBackendFault::ResourceLimit);
    }
    f64_to_u32(result)
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

#[cfg(test)]
mod tests {
    use super::{BOUNDED_MAX_HEIGHT, BOUNDED_MAX_WIDTH, SelectedProfile};
    use crate::fault::OnnxBackendFault;

    #[test]
    fn bounded_planner_covers_declared_dimensions() {
        let profile = SelectedProfile::BoundedDetector.preprocessing();
        let cases = [
            ((3_840, 2_160), (1_312, 736)),
            ((2_000, 500), (1_312, 320)),
            ((2_560, 320), (1_312, 160)),
            ((960, 540), (1_312, 736)),
            ((32, 32), (736, 736)),
            ((1_001, 563), (1_312, 736)),
            ((752, 736), (768, 736)),
            ((1_312, 736), (1_312, 736)),
        ];

        for ((source_width, source_height), (final_width, final_height)) in cases {
            let plan = profile.plan(source_width, source_height).unwrap();
            assert_eq!(
                (plan.final_width(), plan.final_height()),
                (final_width, final_height),
                "{source_width}x{source_height}"
            );
            assert!(plan.final_width() <= BOUNDED_MAX_WIDTH);
            assert!(plan.final_height() <= BOUNDED_MAX_HEIGHT);
            assert_eq!(plan.final_width() % 32, 0);
            assert_eq!(plan.final_height() % 32, 0);
            assert_eq!(
                plan.tensor_bytes(),
                plan.tensor_elements() * size_of::<f32>()
            );
        }
    }

    #[test]
    fn native_profile_retains_unbounded_db736_dimensions() {
        let plan = SelectedProfile::NativeG004
            .preprocessing()
            .plan(3_840, 2_160)
            .unwrap();
        assert_eq!((plan.final_width(), plan.final_height()), (3_840, 2_176));
    }

    #[test]
    fn maximum_bounded_plan_has_exact_tensor_ceiling() {
        let plan = SelectedProfile::BoundedDetector
            .preprocessing()
            .plan(1_312, 736)
            .unwrap();
        assert_eq!(plan.tensor_elements(), 2_896_896);
        assert_eq!(plan.tensor_bytes(), 11_587_584);
        assert_eq!(plan.forward_x(), 1.0);
        assert_eq!(plan.forward_y(), 1.0);
        assert_eq!(plan.inverse_x(), 1.0);
        assert_eq!(plan.inverse_y(), 1.0);
    }

    #[test]
    fn zero_extreme_and_overflow_dimensions_are_refused() {
        let profile = SelectedProfile::BoundedDetector.preprocessing();
        for dimensions in [(0, 1), (1, 0), (u32::MAX, 1), (u32::MAX, u32::MAX)] {
            assert!(matches!(
                profile.plan(dimensions.0, dimensions.1),
                Err(OnnxBackendFault::InvalidPixels | OnnxBackendFault::ResourceLimit)
            ));
        }
    }
}
