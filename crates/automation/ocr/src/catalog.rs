//! Reviewed, immutable OCR configuration. No model bytes or runtime parsing.

use crate::OcrFault;
use crate::model::{ModelComponentIdentity, OcrModelIdentity};

/// Exact controlled host-provided resource and its acquisition provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResourceDescriptor {
    /// Catalog-local resource identifier.
    pub id: &'static str,
    /// Portable path relative to the explicit model root.
    pub relative_path: &'static str,
    /// Exact byte length.
    pub byte_len: u64,
    /// Exact SHA-256.
    pub sha256: [u8; 32],
    /// Revision-pinned acquisition URL; never fetched by the SDK.
    pub source_url: &'static str,
    /// Immutable upstream revision.
    pub source_revision: &'static str,
    /// SPDX license identifier.
    pub license: &'static str,
    /// Reviewed license/notice provenance reference.
    pub notice: &'static str,
}

impl ResourceDescriptor {
    /// Returns the validated length/digest identity.
    #[must_use]
    pub fn identity(&self) -> ModelComponentIdentity {
        ModelComponentIdentity::new(self.byte_len, self.sha256)
            .expect("catalog resource passed build-time validation")
    }
}

/// Implemented detector sampling strategies, not model-specific dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectorResize {
    /// Released short-side expansion and multiple-of-32 rounding.
    LegacyShortSide,
    /// Released rectangular fit followed by a conditional tensor ceiling.
    BoundedRectangular,
    /// Direct no-upscale long-side cap with a checked minimum dimension.
    MaxSide,
}

/// Operation order is part of the immutable pixel contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelNormalization {
    /// Divide by 255, subtract 0.5, then divide by 0.5.
    LegacyDivide,
    /// Multiply by the f32 reciprocal of 255, subtract mean, divide by std.
    MultiplyMeanStd,
}

/// Explicit vocabulary source; no fallback between sources is permitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VocabularySource {
    /// The recognizer's exact `character` custom metadata.
    OnnxMetadata,
    /// The auxiliary resource's `PostProcess.character_dict` sequence.
    PaddleYaml,
}

/// One f32 graph outlet with exact rank/shape and finite dynamic-axis ceilings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct TensorDescriptor {
    /// Exact ONNX outlet name.
    pub name: &'static str,
    /// Exact graph shape; -1 denotes a dynamic axis.
    pub shape: &'static [i64],
    /// Positive per-axis runtime ceilings; static axes equal their shape.
    pub axis_limits: &'static [u32],
}

/// Single-input/single-output f32 graph contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct GraphDescriptor {
    /// Required graph input.
    pub input: TensorDescriptor,
    /// Required graph output.
    pub output: TensorDescriptor,
}

/// Detector pixel, sampling and DB contracts.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct DetectorDescriptor {
    /// Implemented resize strategy.
    pub resize: DetectorResize,
    /// Legacy minimum short side or candidate maximum long side.
    pub side: u32,
    /// Ties-to-even multiple after integer truncation.
    pub multiple: u32,
    /// Candidate post-rounding minimum; zero for legacy rejection semantics.
    pub minimum_dimension: u32,
    /// Rectangular ceiling, absent for the legacy unbounded shape.
    pub max_width: Option<u32>,
    /// Rectangular ceiling, absent for the legacy unbounded shape.
    pub max_height: Option<u32>,
    /// Strategy-specific detector tensor ceiling.
    pub max_tensor_bytes: Option<usize>,
    /// BGR channel normalization operation order.
    pub normalization: PixelNormalization,
    /// BGR channel means.
    pub mean: [f32; 3],
    /// BGR channel standard deviations.
    pub std: [f32; 3],
    /// DB binary threshold.
    pub threshold: f32,
    /// DB fast box score threshold.
    pub box_threshold: f64,
    /// DB polygon expansion ratio.
    pub unclip_ratio: f64,
    /// DB minimum accepted side.
    pub min_side: f32,
    /// Stable detector line-order threshold.
    pub sort_line_threshold: f32,
    /// Maximum raw detector candidates.
    pub max_candidates: usize,
}

/// Original-source perspective crop, orientation, resize and right-zero-pad CTC input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct RecognitionDescriptor {
    /// Tensor height.
    pub height: usize,
    /// Nominal minimum tensor width.
    pub nominal_width: usize,
    /// Maximum dynamic tensor width.
    pub max_width: usize,
    /// Maximum recognition batch.
    pub batch: usize,
}

/// Ordered dictionary and blank/space CTC semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct VocabularyDescriptor {
    /// Required vocabulary source.
    pub source: VocabularySource,
    /// Dictionary entries before adding blank and terminal space.
    pub entries: u32,
    /// SHA-256 of entries joined by LF without terminal LF.
    pub sha256: [u8; 32],
    /// Graph classes including blank at zero and terminal space.
    pub classes: u32,
}

/// Finite implementation work and memory ceilings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResourceLimits {
    /// Maximum input tensor bytes.
    pub input_tensor_bytes: usize,
    /// Maximum output tensor bytes.
    pub output_tensor_bytes: usize,
    /// Maximum contour storage bytes.
    pub contour_bytes: usize,
    /// Maximum scanline work queue bytes.
    pub scanline_bytes: usize,
    /// Maximum model component bytes.
    pub model_bytes: u64,
    /// Maximum auxiliary source bytes.
    pub auxiliary_bytes: u64,
}

/// Exact target/provider/runtime applicability; not an automatic fallback policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProfileApplicability {
    /// Rust target triple.
    pub target: &'static str,
    /// `cpu` or independently accepted `cuda`.
    pub provider: &'static str,
    /// Exact ONNX Runtime version.
    pub runtime_version: &'static str,
    /// ONNX Runtime API floor.
    pub runtime_api: u32,
}

/// Comparative presentation code, independent from model support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchmarkCode {
    /// Qualified comparison baseline.
    Reference,
    /// All gates and each workload's comparative speed threshold pass.
    Fast,
    /// All gates pass without meeting the Fast threshold.
    Balanced,
    /// Previously accepted, without applicable comparative evidence.
    Unrated,
}

/// Target/provider-scoped comparative evidence reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct BenchmarkSummary {
    /// Rust target triple.
    pub target: &'static str,
    /// Execution provider code.
    pub provider: &'static str,
    /// Evidence-derived code.
    pub code: BenchmarkCode,
    /// Criteria reference, absent for unqualified comparative metadata.
    pub criteria: Option<&'static str>,
    /// Report reference, absent for unqualified comparative metadata.
    pub report: Option<&'static str>,
}

/// Complete build-validated execution descriptor plus separate presentation.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct OcrProfileDescriptor {
    /// Closed model ID.
    pub model_id: &'static str,
    /// Immutable model/profile revision.
    pub model_version: &'static str,
    /// Stable selection ID.
    pub profile_id: &'static str,
    /// Horizontal language/orientation scope.
    pub language_profile_id: &'static str,
    /// Immutable preprocessing ID.
    pub preprocessing_id: &'static str,
    /// Immutable decoder ID.
    pub decoder_id: &'static str,
    /// Public normalization semantics.
    pub normalization_id: &'static str,
    /// Required detector resource.
    pub detector: &'static ResourceDescriptor,
    /// Required recognizer resource.
    pub recognizer: &'static ResourceDescriptor,
    /// Required external vocabulary resource, if any.
    pub auxiliary: Option<&'static ResourceDescriptor>,
    /// Detector graph.
    pub detector_graph: GraphDescriptor,
    /// Recognizer graph.
    pub recognizer_graph: GraphDescriptor,
    /// Detector execution settings.
    pub detection: DetectorDescriptor,
    /// Recognition execution settings.
    pub recognition: RecognitionDescriptor,
    /// Explicit dictionary contract.
    pub vocabulary: VocabularyDescriptor,
    /// Work/memory ceilings.
    pub limits: ResourceLimits,
    /// Exact target/provider/runtime combinations.
    pub applicability: &'static [ProfileApplicability],
    /// SHA-256 of canonical execution data and referenced resources, not presentation/evidence.
    pub execution_sha256: [u8; 32],
    /// Readable model-pair name; never part of identity.
    pub display_name: &'static str,
    /// Optional readable execution variant.
    pub variant: Option<&'static str>,
    /// Applicable comparative metadata; candidate descriptors have none.
    pub benchmarks: &'static [BenchmarkSummary],
}

impl OcrProfileDescriptor {
    /// Builds only an accepted identity, even in maintainer-feature builds.
    ///
    /// # Errors
    /// Returns a profile fault for an unaccepted or modified descriptor.
    pub fn identity(&self) -> Result<OcrModelIdentity, OcrFault> {
        let accepted = accepted_ocr_profile(self.profile_id)
            .filter(|accepted| *accepted == self)
            .ok_or(OcrFault::UnsupportedProfile)?;
        OcrModelIdentity::from_catalog(accepted)
    }

    /// Explicit maintainer identity; never grants product support.
    ///
    /// # Errors
    /// Returns a profile fault for an undeclared or modified descriptor.
    #[cfg(feature = "model-candidates")]
    #[doc(hidden)]
    pub fn candidate_identity(&self) -> Result<OcrModelIdentity, OcrFault> {
        let declared = declared_ocr_profile(self.profile_id)
            .filter(|declared| *declared == self)
            .ok_or(OcrFault::UnsupportedProfile)?;
        OcrModelIdentity::from_catalog(declared)
    }
}

include!(concat!(env!("OUT_DIR"), "/model_catalog.rs"));

/// Accepted production profiles. Feature unification never widens this list.
#[must_use]
pub const fn accepted_ocr_profiles() -> &'static [OcrProfileDescriptor] {
    ACCEPTED_PROFILES
}

/// Looks up an accepted production profile without allocating or loading models.
#[must_use]
pub fn accepted_ocr_profile(id: &str) -> Option<&'static OcrProfileDescriptor> {
    ACCEPTED_PROFILES
        .iter()
        .find(|profile| profile.profile_id == id)
}

/// Looks up an explicit maintainer profile, including unqualified candidates.
#[cfg(feature = "model-candidates")]
#[doc(hidden)]
#[must_use]
pub fn declared_ocr_profile(id: &str) -> Option<&'static OcrProfileDescriptor> {
    accepted_ocr_profile(id).or_else(|| {
        CANDIDATE_PROFILES
            .iter()
            .find(|profile| profile.profile_id == id)
    })
}

/// Whether either ID claims catalog authority, including reserved candidates.
#[must_use]
pub fn is_catalog_profile(model_id: &str, profile_id: &str) -> bool {
    CLOSED_PROFILES
        .iter()
        .any(|&(model, profile, _)| model == model_id || profile == profile_id)
}

/// Whether either closed ID requires an explicit auxiliary source.
#[must_use]
pub fn catalog_profile_requires_auxiliary(model_id: &str, profile_id: &str) -> bool {
    CLOSED_PROFILES.iter().any(|&(model, profile, auxiliary)| {
        auxiliary && (model == model_id || profile == profile_id)
    })
}

#[cfg(test)]
mod tests {
    use super::{
        ACCEPTED_BOUNDED_PROFILE_ID, ACCEPTED_G004_PROFILE_ID, DEFAULT_OCR_PROFILE_ID,
        accepted_ocr_profile, accepted_ocr_profiles, catalog_profile_requires_auxiliary,
        is_catalog_profile,
    };
    use crate::{OcrFault, OcrModelIdentity};

    #[test]
    fn accepted_enumeration_default_and_legacy_identity_roundtrips_are_unchanged() {
        let profiles = accepted_ocr_profiles();
        assert_eq!(profiles.len(), 2);
        assert_eq!(
            DEFAULT_OCR_PROFILE_ID,
            "g-004-rapidocr-ppocrv4-det-v6-rec-small-v1"
        );
        assert!(
            profiles
                .iter()
                .any(|p| p.profile_id == ACCEPTED_G004_PROFILE_ID)
        );
        assert!(
            profiles
                .iter()
                .any(|p| p.profile_id == ACCEPTED_BOUNDED_PROFILE_ID)
        );
        assert!(std::ptr::eq(profiles, accepted_ocr_profiles()));
        for identity in [
            OcrModelIdentity::accepted_g004(),
            OcrModelIdentity::accepted_bounded_detector(),
        ] {
            let profile = accepted_ocr_profile(identity.profile().as_str()).unwrap();
            assert_eq!(profile.identity().unwrap(), identity);
            assert_eq!(
                identity.version().as_str(),
                "rapidocr-3.9.2+095232a4c94f7f0e6600ba5bba1177010ad696d4"
            );
            assert_eq!(identity.detector().byte_len(), 4_745_517);
            assert_eq!(identity.recognizer().byte_len(), 21_234_383);
            assert_eq!(
                identity.detector().sha256(),
                [
                    0xd2, 0xa7, 0x72, 0x0d, 0x45, 0xa5, 0x42, 0x57, 0x20, 0x8b, 0x1e, 0x13, 0xe3,
                    0x6a, 0x84, 0x79, 0x89, 0x4c, 0xb7, 0x41, 0x55, 0xa5, 0xef, 0xe2, 0x94, 0x62,
                    0x51, 0x2d, 0x42, 0xf4, 0x9d, 0xa9,
                ]
            );
            assert_eq!(
                identity.recognizer().sha256(),
                [
                    0x6f, 0x32, 0x72, 0x46, 0xb5, 0x03, 0x88, 0xf3, 0xc1, 0x76, 0xae, 0x30, 0x4b,
                    0xd9, 0x57, 0x67, 0xea, 0x6d, 0xc0, 0xc9, 0xae, 0x92, 0x15, 0x3e, 0xf8, 0xcb,
                    0xe2, 0x10, 0xb3, 0xc1, 0x48, 0x84,
                ]
            );
            assert_eq!(identity.profile_metadata().vocabulary_entries(), 18_708);
            assert!(identity.auxiliary().is_none());
            assert_eq!(
                OcrModelIdentity::new(
                    identity.model().clone(),
                    identity.version().clone(),
                    identity.profile().clone(),
                    identity.detector(),
                    identity.recognizer(),
                    identity.profile_metadata().clone(),
                )
                .unwrap(),
                identity
            );
        }
    }

    #[test]
    fn reserved_candidates_never_enter_production_lookup() {
        let candidate = "ppocrv6-tiny-det-small-rec-max736-v1";
        assert!(accepted_ocr_profile(candidate).is_none());
        assert!(is_catalog_profile(candidate, "other"));
        assert!(is_catalog_profile("other", candidate));
        assert!(catalog_profile_requires_auxiliary(candidate, "other"));
        assert!(catalog_profile_requires_auxiliary("other", candidate));
        assert!(!catalog_profile_requires_auxiliary("generic", "generic"));
        assert!(!catalog_profile_requires_auxiliary(
            ACCEPTED_G004_PROFILE_ID,
            ACCEPTED_G004_PROFILE_ID
        ));
    }

    #[test]
    fn copied_descriptors_cannot_change_execution_authority() {
        let mut changed = *accepted_ocr_profile(DEFAULT_OCR_PROFILE_ID).unwrap();
        changed.detection.threshold = 0.31;
        assert_eq!(changed.identity(), Err(OcrFault::UnsupportedProfile));
    }

    #[cfg(feature = "model-candidates")]
    #[test]
    fn maintainer_identity_does_not_enable_ordinary_candidate_construction() {
        let descriptor =
            super::declared_ocr_profile("ppocrv6-tiny-det-small-rec-max736-v1").unwrap();
        assert_eq!(descriptor.identity(), Err(OcrFault::UnsupportedProfile));
        let identity = descriptor.candidate_identity().unwrap();
        assert_eq!(
            identity.auxiliary(),
            Some(descriptor.auxiliary.unwrap().identity())
        );
        assert_eq!(
            identity.execution_sha256(),
            Some(descriptor.execution_sha256)
        );
        assert_eq!(
            OcrModelIdentity::new(
                identity.model().clone(),
                identity.version().clone(),
                identity.profile().clone(),
                identity.detector(),
                identity.recognizer(),
                identity.profile_metadata().clone(),
            ),
            Err(OcrFault::AcceptedProfileMismatch)
        );
        assert_eq!(accepted_ocr_profiles().len(), 2);
    }
}
