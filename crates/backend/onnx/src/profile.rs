//! Catalog-driven execution profiles and checked detector geometry.
//!
//! One reviewed [`OcrProfileDescriptor`] supplies resource, graph, vocabulary,
//! sampling, normalization, and DB settings. Fields this implementation family
//! fixes in code are validated to their single supported value.

use std::mem::size_of;

use mado_pilot_ocr::{
    ACCEPTED_BOUNDED_PROFILE_ID, ACCEPTED_G004_PROFILE_ID, DetectorDescriptor, DetectorResize,
    GraphDescriptor, MAX_MODEL_COMPONENT_BYTES, OcrModelIdentity, OcrProfileDescriptor,
    PixelNormalization, TensorDescriptor, VocabularyDescriptor, VocabularySource, ocr_profile,
};

use crate::detect::DbParameters;
use crate::fault::{OnnxBackendFault, OnnxExecutionProvider, OnnxOcrProfile};
use crate::vocabulary::Vocabulary;

const RUNTIME_API: u32 = 17;
const DETECTOR_INPUT_SHAPE: [i64; 4] = [-1, 3, -1, -1];
const DETECTOR_OUTPUT_SHAPE: [i64; 4] = [-1, 1, -1, -1];
const LEGACY_DIVIDE_MEAN_STD: [f32; 3] = [0.5; 3];

/// One implemented, catalog-selected execution profile.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ExecutionProfile {
    descriptor: &'static OcrProfileDescriptor,
    preprocessing: PreprocessingDescriptor,
    db: DbParameters,
}

impl ExecutionProfile {
    /// Resolves one implemented catalog profile and its exact identity.
    pub(crate) fn from_id(profile_id: &str) -> Result<(Self, OcrModelIdentity), OnnxBackendFault> {
        let descriptor = ocr_profile(profile_id).ok_or(OnnxBackendFault::ProfileMismatch)?;
        let identity = descriptor
            .identity()
            .map_err(|_| OnnxBackendFault::ProfileMismatch)?;
        Ok((Self::implemented(descriptor)?, identity))
    }

    /// Requires a caller-built source to match one complete catalog identity.
    pub(crate) fn for_identity(identity: &OcrModelIdentity) -> Result<Self, OnnxBackendFault> {
        let (profile, accepted) = Self::from_id(identity.profile().as_str())?;
        if accepted != *identity {
            return Err(OnnxBackendFault::ProfileMismatch);
        }
        Ok(profile)
    }

    fn implemented(descriptor: &'static OcrProfileDescriptor) -> Result<Self, OnnxBackendFault> {
        let recognition = descriptor.recognition;
        let vocabulary = descriptor.vocabulary;
        let limits = descriptor.limits;
        let classes =
            i64::try_from(Vocabulary::classes()).map_err(|_| OnnxBackendFault::ProfileMismatch)?;
        let recognizer_input = [
            -1,
            3,
            i64::try_from(crate::image::RECOGNIZER_HEIGHT)
                .map_err(|_| OnnxBackendFault::ProfileMismatch)?,
            -1,
        ];
        let supported = descriptor.detection.max_candidates
            == crate::detect::MAX_DETECTOR_CANDIDATES
            && recognition.height == crate::image::RECOGNIZER_HEIGHT
            && recognition.nominal_width == crate::image::RECOGNIZER_BASE_WIDTH
            && recognition.max_width == crate::image::MAX_RECOGNIZER_WIDTH
            && recognition.batch == crate::RECOGNITION_BATCH
            && vocabulary_is_implemented(vocabulary)
            && limits.input_tensor_bytes == crate::image::MAX_TENSOR_BYTES
            && limits.output_tensor_bytes == crate::MAX_OUTPUT_BYTES
            && limits.contour_bytes == crate::detect::MAX_CONTOUR_STORAGE_BYTES
            && limits.scanline_bytes == crate::detect::MAX_COMPONENT_STACK_BYTES
            && limits.model_bytes == MAX_MODEL_COMPONENT_BYTES
            && descriptor.detector.byte_len <= limits.model_bytes
            && descriptor.recognizer.byte_len <= limits.model_bytes
            && graph_is(
                descriptor.detector_graph,
                &DETECTOR_INPUT_SHAPE,
                &DETECTOR_OUTPUT_SHAPE,
            )
            && graph_is(
                descriptor.recognizer_graph,
                &recognizer_input,
                &[-1, -1, classes],
            )
            && match (vocabulary.source, descriptor.auxiliary) {
                (VocabularySource::OnnxMetadata, None) => true,
                (VocabularySource::PaddleYaml, Some(resource)) => {
                    limits.auxiliary_bytes
                        <= u64::try_from(crate::vocabulary_yaml::MAX_SOURCE_BYTES)
                            .map_err(|_| OnnxBackendFault::ProfileMismatch)?
                        && resource.byte_len <= limits.auxiliary_bytes
                }
                _ => false,
            };
        let profile = Self {
            descriptor,
            preprocessing: PreprocessingDescriptor::from_catalog(&descriptor.detection)?,
            db: DbParameters::from_catalog(&descriptor.detection),
        };
        if !supported || !profile.permits_provider(OnnxExecutionProvider::Cpu) {
            return Err(OnnxBackendFault::ProfileMismatch);
        }
        Ok(profile)
    }

    /// Returns whether this exact identity was built from this profile.
    pub(crate) fn admits(self, identity: &OcrModelIdentity) -> bool {
        let descriptor = self.descriptor;
        let metadata = identity.profile_metadata();
        identity.model().as_str() == descriptor.model_id
            && identity.version().as_str() == descriptor.model_version
            && identity.profile().as_str() == descriptor.profile_id
            && identity.detector() == descriptor.detector.identity()
            && identity.recognizer() == descriptor.recognizer.identity()
            && identity.auxiliary() == descriptor.auxiliary.map(|resource| resource.identity())
            && identity.execution_sha256() == Some(descriptor.execution_sha256)
            && metadata.language_profile().as_str() == descriptor.language_profile_id
            && metadata.preprocessing().as_str() == descriptor.preprocessing_id
            && metadata.decoder().as_str() == descriptor.decoder_id
            && metadata.normalization().as_str() == descriptor.normalization_id
            && metadata.vocabulary_entries() == descriptor.vocabulary.entries
            && metadata.vocabulary_sha256() == descriptor.vocabulary.sha256
    }

    /// Returns whether the catalog declares this provider for the controlled runtime.
    pub(crate) fn permits_provider(self, provider: OnnxExecutionProvider) -> bool {
        let code = match provider {
            OnnxExecutionProvider::Cpu => "cpu",
            OnnxExecutionProvider::Cuda => "cuda",
            OnnxExecutionProvider::CoreMl => "coreml",
        };
        self.descriptor.applicability.iter().any(|entry| {
            entry.provider == code
                && entry.runtime_version == crate::loader::RUNTIME_VERSION
                && entry.runtime_api == RUNTIME_API
        })
    }

    pub(crate) const fn descriptor(self) -> &'static OcrProfileDescriptor {
        self.descriptor
    }

    pub(crate) const fn id(self) -> &'static str {
        self.descriptor.profile_id
    }

    /// Reports legacy preprocessing names for their exact accepted IDs only.
    pub(crate) fn public(self) -> OnnxOcrProfile {
        match self.descriptor.profile_id {
            ACCEPTED_G004_PROFILE_ID => OnnxOcrProfile::NativeG004,
            ACCEPTED_BOUNDED_PROFILE_ID => OnnxOcrProfile::BoundedDetector,
            _ => OnnxOcrProfile::Catalog,
        }
    }

    pub(crate) const fn preprocessing(self) -> PreprocessingDescriptor {
        self.preprocessing
    }

    pub(crate) const fn db(self) -> DbParameters {
        self.db
    }

    pub(crate) const fn detector_graph(self) -> GraphDescriptor {
        self.descriptor.detector_graph
    }

    pub(crate) const fn recognizer_graph(self) -> GraphDescriptor {
        self.descriptor.recognizer_graph
    }

    pub(crate) const fn vocabulary(self) -> VocabularyDescriptor {
        self.descriptor.vocabulary
    }
}

fn vocabulary_is_implemented(vocabulary: VocabularyDescriptor) -> bool {
    usize::try_from(vocabulary.entries).ok() == Some(Vocabulary::entries())
        && usize::try_from(vocabulary.classes).ok() == Some(Vocabulary::classes())
}

fn graph_is(graph: GraphDescriptor, input: &[i64], output: &[i64]) -> bool {
    tensor_is(graph.input, input) && tensor_is(graph.output, output)
}

fn tensor_is(tensor: TensorDescriptor, shape: &[i64]) -> bool {
    tensor.shape == shape
        && tensor.axis_limits.len() == shape.len()
        && tensor
            .axis_limits
            .iter()
            .zip(shape)
            .all(|(limit, axis)| *limit > 0 && (*axis < 0 || i64::from(*limit) == *axis))
}

/// Detector pixel conversion after direct BGRA resizing, in BGR plane order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum DetectorNormalization {
    /// `pixel / 255`, then `- 0.5`, then `/ 0.5`.
    LegacyDivide,
    /// `pixel * (1 / 255) - mean[channel]`, then `/ std[channel]`.
    MultiplyMeanStd { mean: [f32; 3], std: [f32; 3] },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DetectorSampling {
    /// Released DB736 short-side expansion without a final ceiling.
    ShortSide,
    /// ADR 0040 rectangular fit followed by a conditional tensor ceiling.
    BoundedRectangular {
        max_width: u32,
        max_height: u32,
        max_tensor_bytes: usize,
    },
    /// No-upscale long-side cap with an explicit post-rounding minimum.
    MaxSide {
        minimum_dimension: u32,
        max_tensor_bytes: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PreprocessingDescriptor {
    sampling: DetectorSampling,
    side: u32,
    multiple: u32,
    normalization: DetectorNormalization,
}

impl PreprocessingDescriptor {
    fn from_catalog(detection: &DetectorDescriptor) -> Result<Self, OnnxBackendFault> {
        let side = detection.side;
        let multiple = detection.multiple;
        let minimum = detection.minimum_dimension;
        if side == 0 || multiple == 0 {
            return Err(OnnxBackendFault::ProfileMismatch);
        }
        let bounded_tensor = |bytes: usize| bytes > 0 && bytes <= crate::image::MAX_TENSOR_BYTES;
        let sampling = match (
            detection.resize,
            detection.max_width,
            detection.max_height,
            detection.max_tensor_bytes,
        ) {
            (DetectorResize::LegacyShortSide, None, None, None) if minimum == 0 => {
                DetectorSampling::ShortSide
            }
            (
                DetectorResize::BoundedRectangular,
                Some(max_width),
                Some(max_height),
                Some(max_tensor_bytes),
            ) if minimum == 0
                && max_width >= multiple
                && max_height >= multiple
                && bounded_tensor(max_tensor_bytes) =>
            {
                DetectorSampling::BoundedRectangular {
                    max_width,
                    max_height,
                    max_tensor_bytes,
                }
            }
            (
                DetectorResize::MaxSide,
                Some(max_width),
                Some(max_height),
                Some(max_tensor_bytes),
            ) if max_width == side
                && max_height == side
                && side.is_multiple_of(multiple)
                && minimum >= multiple
                && minimum <= side
                && bounded_tensor(max_tensor_bytes) =>
            {
                DetectorSampling::MaxSide {
                    minimum_dimension: minimum,
                    max_tensor_bytes,
                }
            }
            _ => return Err(OnnxBackendFault::ProfileMismatch),
        };
        let normalization = match detection.normalization {
            PixelNormalization::LegacyDivide
                if detection.mean == LEGACY_DIVIDE_MEAN_STD
                    && detection.std == LEGACY_DIVIDE_MEAN_STD =>
            {
                DetectorNormalization::LegacyDivide
            }
            PixelNormalization::MultiplyMeanStd
                if detection.mean.iter().all(|mean| mean.is_finite())
                    && detection
                        .std
                        .iter()
                        .all(|std| std.is_finite() && *std > 0.0) =>
            {
                DetectorNormalization::MultiplyMeanStd {
                    mean: detection.mean,
                    std: detection.std,
                }
            }
            _ => return Err(OnnxBackendFault::ProfileMismatch),
        };
        Ok(Self {
            sampling,
            side,
            multiple,
            normalization,
        })
    }

    pub(crate) const fn normalization(self) -> DetectorNormalization {
        self.normalization
    }

    pub(crate) const fn max_width(self) -> Option<u32> {
        match self.sampling {
            DetectorSampling::ShortSide => None,
            DetectorSampling::BoundedRectangular { max_width, .. } => Some(max_width),
            DetectorSampling::MaxSide { .. } => Some(self.side),
        }
    }

    pub(crate) const fn max_height(self) -> Option<u32> {
        match self.sampling {
            DetectorSampling::ShortSide => None,
            DetectorSampling::BoundedRectangular { max_height, .. } => Some(max_height),
            DetectorSampling::MaxSide { .. } => Some(self.side),
        }
    }

    pub(crate) fn plan(
        self,
        source_width: u32,
        source_height: u32,
    ) -> Result<DetectorPlan, OnnxBackendFault> {
        if source_width == 0 || source_height == 0 {
            return Err(OnnxBackendFault::InvalidPixels);
        }
        match self.sampling {
            DetectorSampling::ShortSide => self.short_side_plan(source_width, source_height, None),
            DetectorSampling::BoundedRectangular {
                max_width,
                max_height,
                max_tensor_bytes,
            } => self.short_side_plan(
                source_width,
                source_height,
                Some((max_width, max_height, max_tensor_bytes)),
            ),
            DetectorSampling::MaxSide {
                minimum_dimension,
                max_tensor_bytes,
            } => self.max_side_plan(
                source_width,
                source_height,
                minimum_dimension,
                max_tensor_bytes,
            ),
        }
    }

    fn short_side_plan(
        self,
        source_width: u32,
        source_height: u32,
        bounds: Option<(u32, u32, usize)>,
    ) -> Result<DetectorPlan, OnnxBackendFault> {
        let short_side = source_width.min(source_height);
        let ratio = if short_side < self.side {
            f64::from(self.side) / f64::from(short_side)
        } else {
            1.0
        };
        let desired_width =
            round_to_multiple(f64_to_u32(f64::from(source_width) * ratio)?, self.multiple)?;
        let desired_height =
            round_to_multiple(f64_to_u32(f64::from(source_height) * ratio)?, self.multiple)?;

        let (final_width, final_height) = match bounds {
            Some((max_width, max_height, max_tensor_bytes))
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
                fit_with_tensor_ceiling(width, height, self.multiple, max_tensor_bytes)?
            }
            _ => (desired_width, desired_height),
        };
        DetectorPlan::checked(source_width, source_height, final_width, final_height)
    }

    /// Paddle `DetResizeForTest` max-side sampling: no upscaling before rounding.
    fn max_side_plan(
        self,
        source_width: u32,
        source_height: u32,
        minimum_dimension: u32,
        max_tensor_bytes: usize,
    ) -> Result<DetectorPlan, OnnxBackendFault> {
        let long_side = source_width.max(source_height);
        let ratio = if long_side > self.side {
            f64::from(self.side) / f64::from(long_side)
        } else {
            1.0
        };
        let final_width = nearest_multiple_at_least(
            f64_to_u32(f64::from(source_width) * ratio)?,
            self.multiple,
            minimum_dimension,
        )?;
        let final_height = nearest_multiple_at_least(
            f64_to_u32(f64::from(source_height) * ratio)?,
            self.multiple,
            minimum_dimension,
        )?;
        if final_width > self.side || final_height > self.side {
            return Err(OnnxBackendFault::ResourceLimit);
        }
        let plan = DetectorPlan::checked(source_width, source_height, final_width, final_height)?;
        if plan.tensor_bytes() > max_tensor_bytes {
            return Err(OnnxBackendFault::ResourceLimit);
        }
        Ok(plan)
    }
}

fn fit_with_tensor_ceiling(
    width: u32,
    height: u32,
    multiple: u32,
    max_bytes: usize,
) -> Result<(u32, u32), OnnxBackendFault> {
    let bytes_per_pixel = 3 * size_of::<f32>();
    let max_pixels =
        u32::try_from(max_bytes / bytes_per_pixel).map_err(|_| OnnxBackendFault::ResourceLimit)?;
    let pixels = width
        .checked_mul(height)
        .ok_or(OnnxBackendFault::ResourceLimit)?;
    if pixels <= max_pixels {
        return Ok((width, height));
    }

    let fit = (f64::from(max_pixels) / f64::from(pixels)).sqrt();
    let scaled_width = f64_to_u32(f64::from(width) * fit)?;
    let scaled_height = f64_to_u32(f64::from(height) * fit)?;
    let bounded_width = floor_to_multiple(scaled_width, multiple)?;
    let bounded_height = floor_to_multiple(scaled_height, multiple)?;
    if bounded_width
        .checked_mul(bounded_height)
        .is_none_or(|bounded_pixels| bounded_pixels > max_pixels)
    {
        return Err(OnnxBackendFault::ResourceLimit);
    }
    Ok((bounded_width, bounded_height))
}

fn floor_to_multiple(value: u32, multiple: u32) -> Result<u32, OnnxBackendFault> {
    if multiple == 0 {
        return Err(OnnxBackendFault::ResourceLimit);
    }
    let bounded = value - value % multiple;
    if bounded < multiple {
        return Err(OnnxBackendFault::ResourceLimit);
    }
    Ok(bounded)
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

/// Ties-to-even multiple that may round to zero, then the explicit minimum clamp.
fn nearest_multiple_at_least(
    value: u32,
    multiple: u32,
    minimum: u32,
) -> Result<u32, OnnxBackendFault> {
    if multiple == 0 {
        return Err(OnnxBackendFault::ResourceLimit);
    }
    let rounded = (f64::from(value) / f64::from(multiple)).round_ties_even() * f64::from(multiple);
    Ok(f64_to_u32(rounded)?.max(minimum))
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
    use mado_pilot_ocr::{ACCEPTED_BOUNDED_PROFILE_ID, ACCEPTED_G004_PROFILE_ID, OcrModelIdentity};

    use super::{
        DetectorNormalization, DetectorSampling, ExecutionProfile, PreprocessingDescriptor,
        round_to_multiple,
    };
    use crate::fault::{OnnxBackendFault, OnnxExecutionProvider, OnnxOcrProfile};

    const CANDIDATE_PROFILE_ID: &str = "ppocrv6-tiny-det-small-rec-max736-v1";
    const CANDIDATE_MAX_TENSOR_BYTES: usize = 736 * 736 * 3 * 4;

    fn bounded() -> PreprocessingDescriptor {
        ExecutionProfile::from_id(ACCEPTED_BOUNDED_PROFILE_ID)
            .expect("accepted bounded profile")
            .0
            .preprocessing()
    }

    fn native() -> PreprocessingDescriptor {
        ExecutionProfile::from_id(ACCEPTED_G004_PROFILE_ID)
            .expect("accepted G-004 profile")
            .0
            .preprocessing()
    }

    fn max_side(max_tensor_bytes: usize) -> PreprocessingDescriptor {
        PreprocessingDescriptor {
            sampling: DetectorSampling::MaxSide {
                minimum_dimension: 32,
                max_tensor_bytes,
            },
            side: 736,
            multiple: 32,
            normalization: DetectorNormalization::MultiplyMeanStd {
                mean: [0.485, 0.456, 0.406],
                std: [0.229, 0.224, 0.225],
            },
        }
    }

    #[test]
    fn bounded_planner_covers_declared_dimensions() {
        let profile = bounded();
        let cases = [
            ((3_840, 2_160), (960, 512)),
            ((2_000, 500), (1_312, 320)),
            ((2_560, 320), (1_312, 160)),
            ((960, 540), (1_312, 736)),
            ((32, 32), (736, 736)),
            ((1_001, 563), (1_312, 736)),
            ((752, 736), (768, 736)),
            ((1_312, 736), (1_312, 736)),
            ((1_440, 720), (1_024, 480)),
        ];

        for ((source_width, source_height), (final_width, final_height)) in cases {
            let plan = profile.plan(source_width, source_height).unwrap();
            assert_eq!(
                (plan.final_width(), plan.final_height()),
                (final_width, final_height),
                "{source_width}x{source_height}"
            );
            assert!(plan.final_width() <= 1_312);
            assert!(plan.final_height() <= 736);
            assert_eq!(plan.final_width() % 32, 0);
            assert_eq!(plan.final_height() % 32, 0);
            assert_eq!(
                plan.tensor_bytes(),
                plan.tensor_elements() * size_of::<f32>()
            );
            assert!(plan.tensor_bytes() <= 11_587_584);
        }
    }

    #[test]
    fn multiple_of_32_rounding_preserves_ties_to_even_in_both_directions() {
        assert_eq!(round_to_multiple(752, 32), Ok(768));
        assert_eq!(round_to_multiple(720, 32), Ok(704));
    }

    #[test]
    fn native_profile_retains_unbounded_db736_dimensions() {
        let plan = native().plan(3_840, 2_160).unwrap();
        assert_eq!((plan.final_width(), plan.final_height()), (3_840, 2_176));
        assert_eq!(
            native().normalization(),
            DetectorNormalization::LegacyDivide
        );
        assert_eq!(native().max_width(), None);
        assert_eq!(native().max_height(), None);
    }

    #[test]
    fn high_area_plan_respects_the_compound_tensor_ceiling() {
        let plan = bounded().plan(3_840, 2_160).unwrap();
        assert_eq!((plan.final_width(), plan.final_height()), (960, 512));
        assert_eq!(plan.tensor_elements(), 1_474_560);
        assert_eq!(plan.tensor_bytes(), 5_898_240);
        assert!(plan.tensor_bytes() <= 6 * 1024 * 1024);
        assert_eq!(plan.forward_x(), 960.0 / 3_840.0);
        assert_eq!(plan.forward_y(), 512.0 / 2_160.0);
        assert_eq!(plan.inverse_x(), 3_840.0 / 960.0);
        assert_eq!(plan.inverse_y(), 2_160.0 / 512.0);
    }

    #[test]
    fn reference_source_retains_released_detector_geometry() {
        let plan = bounded().plan(960, 540).unwrap();
        assert_eq!((plan.final_width(), plan.final_height()), (1_312, 736));
        assert_eq!(plan.tensor_bytes(), 11_587_584);
        assert_eq!(
            bounded().normalization(),
            DetectorNormalization::LegacyDivide
        );
    }

    #[test]
    fn zero_extreme_and_overflow_dimensions_are_refused() {
        let profile = bounded();
        for dimensions in [(0, 1), (1, 0), (u32::MAX, 1), (u32::MAX, u32::MAX)] {
            assert!(matches!(
                profile.plan(dimensions.0, dimensions.1),
                Err(OnnxBackendFault::InvalidPixels | OnnxBackendFault::ResourceLimit)
            ));
        }
    }

    #[test]
    fn max_side_caps_without_short_side_upscaling_and_clamps_the_minimum() {
        let profile = max_side(CANDIDATE_MAX_TENSOR_BYTES);
        let cases = [
            ((960, 540), (736, 416)),
            ((3_840, 2_160), (736, 416)),
            ((2_000, 500), (736, 192)),
            ((500, 2_000), (192, 736)),
            ((736, 736), (736, 736)),
            ((737, 10), (736, 32)),
            ((720, 400), (704, 384)),
            ((100, 50), (96, 64)),
            ((48, 16), (64, 32)),
            ((80, 31), (64, 32)),
            ((1, 1), (32, 32)),
            ((u32::MAX, 1), (736, 32)),
            ((1, u32::MAX), (32, 736)),
            ((u32::MAX, u32::MAX), (736, 736)),
        ];
        for ((source_width, source_height), expected) in cases {
            let plan = profile.plan(source_width, source_height).unwrap();
            assert_eq!(
                (plan.final_width(), plan.final_height()),
                expected,
                "{source_width}x{source_height}"
            );
            assert!(plan.final_width() <= 736 && plan.final_height() <= 736);
            assert!(plan.final_width() >= 32 && plan.final_height() >= 32);
            assert_eq!(plan.final_width() % 32, 0);
            assert_eq!(plan.final_height() % 32, 0);
            assert!(plan.tensor_bytes() <= CANDIDATE_MAX_TENSOR_BYTES);
            assert_eq!(
                plan.inverse_x(),
                f64::from(source_width) / f64::from(plan.final_width())
            );
            assert_eq!(
                plan.inverse_y(),
                f64::from(source_height) / f64::from(plan.final_height())
            );
            assert!(plan.inverse_x().is_finite() && plan.inverse_y().is_finite());
        }
        assert_eq!(profile.max_width(), Some(736));
        assert_eq!(profile.max_height(), Some(736));
    }

    #[test]
    fn max_side_refuses_zero_dimensions_and_an_exceeded_tensor_ceiling() {
        let profile = max_side(CANDIDATE_MAX_TENSOR_BYTES);
        for dimensions in [(0, 1), (1, 0), (0, 0)] {
            assert_eq!(
                profile.plan(dimensions.0, dimensions.1),
                Err(OnnxBackendFault::InvalidPixels)
            );
        }
        let narrowed = max_side(CANDIDATE_MAX_TENSOR_BYTES - 1);
        assert_eq!(
            narrowed.plan(736, 736),
            Err(OnnxBackendFault::ResourceLimit)
        );
        assert!(narrowed.plan(736, 704).is_ok());
    }

    #[test]
    fn legacy_profiles_keep_exact_identities_and_provider_scope() {
        let (native, identity) = ExecutionProfile::from_id(ACCEPTED_G004_PROFILE_ID).unwrap();
        assert_eq!(identity, OcrModelIdentity::accepted_g004());
        assert_eq!(native.public(), OnnxOcrProfile::NativeG004);
        assert!(native.admits(&identity));
        assert!(native.permits_provider(OnnxExecutionProvider::Cpu));
        assert!(native.permits_provider(OnnxExecutionProvider::Cuda));
        assert!(!native.permits_provider(OnnxExecutionProvider::CoreMl));
        assert_eq!(native.detector_graph().output.name, "sigmoid_0.tmp_0");

        let (bounded, identity) = ExecutionProfile::from_id(ACCEPTED_BOUNDED_PROFILE_ID).unwrap();
        assert_eq!(identity, OcrModelIdentity::accepted_bounded_detector());
        assert_eq!(bounded.public(), OnnxOcrProfile::BoundedDetector);
        assert!(!bounded.admits(&OcrModelIdentity::accepted_g004()));
        assert_eq!(
            ExecutionProfile::for_identity(&OcrModelIdentity::accepted_g004())
                .unwrap()
                .id(),
            ACCEPTED_G004_PROFILE_ID
        );

        for id in ["unknown-profile", ""] {
            assert_eq!(
                ExecutionProfile::from_id(id).map(|(profile, _)| profile.id()),
                Err(OnnxBackendFault::ProfileMismatch)
            );
        }
    }

    #[test]
    fn ordinary_max_side_profile_binds_exact_execution_and_cpu_scope() {
        let (candidate, identity) =
            ExecutionProfile::from_id(CANDIDATE_PROFILE_ID).expect("declared profile");
        assert_eq!(candidate.public(), OnnxOcrProfile::Catalog);
        assert_eq!(candidate.id(), CANDIDATE_PROFILE_ID);
        assert!(candidate.admits(&identity));
        assert!(candidate.permits_provider(OnnxExecutionProvider::Cpu));
        assert!(!candidate.permits_provider(OnnxExecutionProvider::Cuda));
        assert!(!candidate.permits_provider(OnnxExecutionProvider::CoreMl));
        assert_eq!(
            candidate.preprocessing(),
            max_side(CANDIDATE_MAX_TENSOR_BYTES)
        );
        assert_eq!(candidate.detector_graph().input.name, "x");
        assert_eq!(candidate.detector_graph().output.name, "fetch_name_0");
        assert_eq!(candidate.recognizer_graph().output.name, "fetch_name_0");
        assert_eq!(
            candidate.vocabulary().source,
            mado_pilot_ocr::VocabularySource::PaddleYaml
        );
        assert!(candidate.descriptor().auxiliary.is_some());
        assert_eq!(
            ExecutionProfile::for_identity(&identity).map(ExecutionProfile::id),
            Ok(CANDIDATE_PROFILE_ID)
        );
        assert!(
            !ExecutionProfile::from_id(ACCEPTED_G004_PROFILE_ID)
                .unwrap()
                .0
                .admits(&identity)
        );

        let (declared_native, _) = ExecutionProfile::from_id(ACCEPTED_G004_PROFILE_ID).unwrap();
        assert_eq!(declared_native.public(), OnnxOcrProfile::NativeG004);
        assert_eq!(
            candidate.db(),
            declared_native.db(),
            "the candidate starts from the reviewed RapidOCR DB strategy"
        );
    }
}
