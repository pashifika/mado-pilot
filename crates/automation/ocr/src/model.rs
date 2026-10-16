//! Bounded OCR model and profile identities with immutable component bytes.

use std::borrow::Borrow;
use std::fmt;
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::fault::OcrFault;

const MAX_IDENTIFIER_BYTES: usize = 128;

fn validate_identifier(value: &str) -> Result<(), OcrFault> {
    if value.is_empty()
        || value.len() > MAX_IDENTIFIER_BYTES
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(OcrFault::InvalidIdentifier);
    }
    Ok(())
}

macro_rules! identifier {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(Arc<str>);

        impl $name {
            /// Builds a bounded non-empty identifier.
            ///
            /// # Errors
            ///
            /// Returns [`OcrFault::InvalidIdentifier`] for an empty identifier,
            /// surrounding whitespace, control characters, or more than 128 UTF-8 bytes.
            pub fn new(value: impl Into<Arc<str>>) -> Result<Self, OcrFault> {
                let value = value.into();
                validate_identifier(&value)?;
                Ok(Self(value))
            }

            /// Returns the identifier text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }
    };
}

identifier!(BackendId, "A stable OCR backend identifier.");
identifier!(
    BackendVersion,
    "A bounded OCR backend implementation version."
);
identifier!(ModelId, "A stable OCR model identifier.");
identifier!(ModelVersion, "A bounded OCR model revision.");
identifier!(
    ProfileId,
    "A stable OCR normalization and decoder profile identifier."
);
identifier!(
    ProviderProfileId,
    "A bounded OCR execution-provider runtime profile identifier."
);
identifier!(
    LanguageProfileId,
    "A bounded OCR language-profile identifier."
);
identifier!(PreprocessingId, "A bounded OCR preprocessing identifier.");
identifier!(DecoderId, "A bounded OCR decoder identifier.");
identifier!(
    NormalizationId,
    "A bounded OCR result-normalization identifier."
);

pub use crate::catalog::{
    ACCEPTED_BOUNDED_MODEL_ID, ACCEPTED_BOUNDED_MODEL_VERSION, ACCEPTED_BOUNDED_PREPROCESSING_ID,
    ACCEPTED_BOUNDED_PROFILE_ID, ACCEPTED_G004_DECODER_ID, ACCEPTED_G004_LANGUAGE_PROFILE_ID,
    ACCEPTED_G004_MODEL_ID, ACCEPTED_G004_MODEL_VERSION, ACCEPTED_G004_NORMALIZATION_ID,
    ACCEPTED_G004_PREPROCESSING_ID, ACCEPTED_G004_PROFILE_ID, ACCEPTED_G004_VOCABULARY_ENTRIES,
    MAX_MODEL_COMPONENT_BYTES,
};

/// The exact immutable identity of one model component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModelComponentIdentity {
    byte_len: u64,
    sha256: [u8; 32],
}

impl ModelComponentIdentity {
    /// Builds a bounded component identity.
    ///
    /// # Errors
    ///
    /// Returns a typed model fault for zero or more than 64 MiB.
    pub const fn new(byte_len: u64, sha256: [u8; 32]) -> Result<Self, OcrFault> {
        if byte_len == 0 {
            return Err(OcrFault::EmptyModelComponent);
        }
        if byte_len > MAX_MODEL_COMPONENT_BYTES {
            return Err(OcrFault::ModelComponentAboveCeiling);
        }
        Ok(Self { byte_len, sha256 })
    }

    /// Returns the exact expected byte length.
    #[must_use]
    pub const fn byte_len(self) -> u64 {
        self.byte_len
    }

    /// Returns the exact expected SHA-256 digest.
    #[must_use]
    pub const fn sha256(self) -> [u8; 32] {
        self.sha256
    }
}

/// One immutable, length- and digest-validated model component.
#[derive(Clone, PartialEq, Eq)]
pub struct OcrModelComponent {
    identity: ModelComponentIdentity,
    bytes: Arc<[u8]>,
}

impl OcrModelComponent {
    /// Validates and commits one component allocation.
    ///
    /// # Errors
    ///
    /// Returns a typed mismatch fault before the component can reach a backend.
    pub fn new(bytes: Arc<[u8]>, identity: ModelComponentIdentity) -> Result<Self, OcrFault> {
        let actual_len =
            u64::try_from(bytes.len()).map_err(|_| OcrFault::ModelComponentAboveCeiling)?;
        if actual_len != identity.byte_len() {
            return Err(OcrFault::ModelLengthMismatch);
        }
        let actual: [u8; 32] = Sha256::digest(&bytes).into();
        if actual != identity.sha256() {
            return Err(OcrFault::ModelDigestMismatch);
        }
        Ok(Self { identity, bytes })
    }

    /// Returns the validated component identity.
    #[must_use]
    pub const fn identity(&self) -> ModelComponentIdentity {
        self.identity
    }

    /// Returns the immutable component bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Shares the component allocation without copying it.
    #[must_use]
    pub fn shared_bytes(&self) -> Arc<[u8]> {
        Arc::clone(&self.bytes)
    }
}

impl fmt::Debug for OcrModelComponent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OcrModelComponent")
            .field("identity", &self.identity)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

/// Exact bounded preprocessing, decoder, language, and vocabulary metadata.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OcrProfileMetadata {
    language_profile: LanguageProfileId,
    preprocessing: PreprocessingId,
    decoder: DecoderId,
    normalization: NormalizationId,
    vocabulary_entries: u32,
    vocabulary_sha256: [u8; 32],
}

impl OcrProfileMetadata {
    /// Builds complete profile metadata.
    ///
    /// # Errors
    ///
    /// Returns [`OcrFault::InvalidProfileMetadata`] for an empty vocabulary and
    /// [`OcrFault::UnsupportedProfile`] for normalization semantics this build
    /// does not implement.
    pub fn new(
        language_profile: LanguageProfileId,
        preprocessing: PreprocessingId,
        decoder: DecoderId,
        normalization: NormalizationId,
        vocabulary_entries: u32,
        vocabulary_sha256: [u8; 32],
    ) -> Result<Self, OcrFault> {
        if normalization.as_str() != ACCEPTED_G004_NORMALIZATION_ID {
            return Err(OcrFault::UnsupportedProfile);
        }
        if vocabulary_entries == 0 {
            return Err(OcrFault::InvalidProfileMetadata);
        }
        Ok(Self {
            language_profile,
            preprocessing,
            decoder,
            normalization,
            vocabulary_entries,
            vocabulary_sha256,
        })
    }

    /// Returns the language-profile identity.
    #[must_use]
    pub const fn language_profile(&self) -> &LanguageProfileId {
        &self.language_profile
    }

    /// Returns the preprocessing identity.
    #[must_use]
    pub const fn preprocessing(&self) -> &PreprocessingId {
        &self.preprocessing
    }

    /// Returns the decoder identity.
    #[must_use]
    pub const fn decoder(&self) -> &DecoderId {
        &self.decoder
    }

    /// Returns the result-normalization identity.
    #[must_use]
    pub const fn normalization(&self) -> &NormalizationId {
        &self.normalization
    }

    /// Returns the declared vocabulary entry count, excluding blank and space.
    #[must_use]
    pub const fn vocabulary_entries(&self) -> u32 {
        self.vocabulary_entries
    }

    /// Returns the canonical ordered vocabulary SHA-256.
    #[must_use]
    pub const fn vocabulary_sha256(&self) -> [u8; 32] {
        self.vocabulary_sha256
    }
}

/// Complete model, component, profile, and decoder identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OcrModelIdentity {
    model: ModelId,
    version: ModelVersion,
    profile: ProfileId,
    detector: ModelComponentIdentity,
    recognizer: ModelComponentIdentity,
    auxiliary: Option<ModelComponentIdentity>,
    execution_sha256: Option<[u8; 32]>,
    profile_metadata: OcrProfileMetadata,
}

impl OcrModelIdentity {
    /// Builds a complete model identity and enforces closed catalog authority.
    ///
    /// # Errors
    ///
    /// Returns [`OcrFault::AcceptedProfileMismatch`] for an incomplete or
    /// mismatched catalog tuple, including profiles requiring auxiliary resources.
    /// Generic backend IDs remain legal.
    pub fn new(
        model: ModelId,
        version: ModelVersion,
        profile: ProfileId,
        detector: ModelComponentIdentity,
        recognizer: ModelComponentIdentity,
        profile_metadata: OcrProfileMetadata,
    ) -> Result<Self, OcrFault> {
        let mut identity = Self {
            model,
            version,
            profile,
            detector,
            recognizer,
            profile_metadata,
            auxiliary: None,
            execution_sha256: None,
        };
        identity.validate_profile_authority()?;
        Ok(identity)
    }

    /// Builds the unchanged accepted native G-004 identity.
    #[must_use]
    pub fn accepted_g004() -> Self {
        crate::catalog::ocr_profile(ACCEPTED_G004_PROFILE_ID)
            .expect("catalog retains native G-004")
            .identity()
            .expect("validated accepted G-004 identity")
    }

    /// Builds the unchanged accepted bounded-detector identity.
    #[must_use]
    pub fn accepted_bounded_detector() -> Self {
        crate::catalog::ocr_profile(ACCEPTED_BOUNDED_PROFILE_ID)
            .expect("catalog retains bounded detector")
            .identity()
            .expect("validated accepted bounded identity")
    }

    pub(crate) fn from_catalog(
        descriptor: &crate::catalog::OcrProfileDescriptor,
    ) -> Result<Self, OcrFault> {
        Ok(Self {
            model: ModelId::new(descriptor.model_id)?,
            version: ModelVersion::new(descriptor.model_version)?,
            profile: ProfileId::new(descriptor.profile_id)?,
            detector: descriptor.detector.identity(),
            recognizer: descriptor.recognizer.identity(),
            auxiliary: descriptor.auxiliary.map(|resource| resource.identity()),
            execution_sha256: Some(descriptor.execution_sha256),
            profile_metadata: OcrProfileMetadata::new(
                LanguageProfileId::new(descriptor.language_profile_id)?,
                PreprocessingId::new(descriptor.preprocessing_id)?,
                DecoderId::new(descriptor.decoder_id)?,
                NormalizationId::new(descriptor.normalization_id)?,
                descriptor.vocabulary.entries,
                descriptor.vocabulary.sha256,
            )?,
        })
    }

    fn validate_profile_authority(&mut self) -> Result<(), OcrFault> {
        if !crate::catalog::is_catalog_profile(self.model.as_str(), self.profile.as_str()) {
            return Ok(());
        }
        // This constructor cannot express an auxiliary resource.
        let descriptor = crate::catalog::ocr_profile(self.profile.as_str())
            .ok_or(OcrFault::AcceptedProfileMismatch)?;
        let metadata = &self.profile_metadata;
        if self.model.as_str() != descriptor.model_id
            || self.version.as_str() != descriptor.model_version
            || self.detector != descriptor.detector.identity()
            || self.recognizer != descriptor.recognizer.identity()
            || descriptor.auxiliary.is_some()
            || metadata.language_profile.as_str() != descriptor.language_profile_id
            || metadata.preprocessing.as_str() != descriptor.preprocessing_id
            || metadata.decoder.as_str() != descriptor.decoder_id
            || metadata.normalization.as_str() != descriptor.normalization_id
            || metadata.vocabulary_entries != descriptor.vocabulary.entries
            || metadata.vocabulary_sha256 != descriptor.vocabulary.sha256
        {
            return Err(OcrFault::AcceptedProfileMismatch);
        }
        self.execution_sha256 = Some(descriptor.execution_sha256);
        Ok(())
    }

    /// Returns the model name.
    #[must_use]
    pub const fn model(&self) -> &ModelId {
        &self.model
    }

    /// Returns the exact model revision.
    #[must_use]
    pub const fn version(&self) -> &ModelVersion {
        &self.version
    }

    /// Returns the profile identity.
    #[must_use]
    pub const fn profile(&self) -> &ProfileId {
        &self.profile
    }

    /// Returns the detector component identity.
    #[must_use]
    pub const fn detector(&self) -> ModelComponentIdentity {
        self.detector
    }

    /// Returns the recognizer component identity.
    #[must_use]
    pub const fn recognizer(&self) -> ModelComponentIdentity {
        self.recognizer
    }

    /// Returns the required auxiliary identity, if the catalog declares one.
    #[must_use]
    pub const fn auxiliary(&self) -> Option<ModelComponentIdentity> {
        self.auxiliary
    }

    /// Returns the bound catalog execution digest, absent for generic backends.
    #[must_use]
    pub const fn execution_sha256(&self) -> Option<[u8; 32]> {
        self.execution_sha256
    }

    /// Returns exact profile metadata.
    #[must_use]
    pub const fn profile_metadata(&self) -> &OcrProfileMetadata {
        &self.profile_metadata
    }
}

/// Inputs used to commit one immutable OCR model source.
#[derive(Debug)]
pub struct OcrModelSourceRequest {
    /// Complete model and profile identity.
    pub identity: OcrModelIdentity,
    /// Validated detector component.
    pub detector: OcrModelComponent,
    /// Validated recognizer component.
    pub recognizer: OcrModelComponent,
}

/// Immutable, digest-validated OCR models and any required auxiliary resource.
///
/// Cloning shares identity and component allocations. A backend can therefore
/// retain a source without copying model bytes per request or session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrModelSource {
    identity: OcrModelIdentity,
    detector: OcrModelComponent,
    recognizer: OcrModelComponent,
    auxiliary: Option<OcrModelComponent>,
}

impl OcrModelSource {
    /// Maximum bytes accepted for either model component.
    pub const MAX_COMPONENT_BYTES: u64 = MAX_MODEL_COMPONENT_BYTES;

    /// Commits two independently validated model components.
    ///
    /// # Errors
    ///
    /// Returns [`OcrFault::ModelMismatch`] for mismatched components or a profile
    /// requiring an auxiliary source. No source is published on failure.
    pub fn new(request: OcrModelSourceRequest) -> Result<Self, OcrFault> {
        Self::commit(request, None)
    }

    /// Commits the complete resource set for an explicitly auxiliary-backed profile.
    ///
    /// # Errors
    ///
    /// Returns [`OcrFault::ModelMismatch`] unless all three validated components
    /// agree exactly with the model identity, including the required auxiliary.
    pub fn new_with_auxiliary(
        request: OcrModelSourceRequest,
        auxiliary: OcrModelComponent,
    ) -> Result<Self, OcrFault> {
        Self::commit(request, Some(auxiliary))
    }

    fn commit(
        request: OcrModelSourceRequest,
        auxiliary: Option<OcrModelComponent>,
    ) -> Result<Self, OcrFault> {
        if request.detector.identity() != request.identity.detector()
            || request.recognizer.identity() != request.identity.recognizer()
            || auxiliary.as_ref().map(OcrModelComponent::identity) != request.identity.auxiliary()
        {
            return Err(OcrFault::ModelMismatch);
        }
        Ok(Self {
            identity: request.identity,
            detector: request.detector,
            recognizer: request.recognizer,
            auxiliary,
        })
    }

    /// Returns the same captured immutable auxiliary bytes validated at construction.
    #[must_use]
    pub const fn auxiliary(&self) -> Option<&OcrModelComponent> {
        self.auxiliary.as_ref()
    }

    /// Returns complete model and profile identity.
    #[must_use]
    pub const fn identity(&self) -> &OcrModelIdentity {
        &self.identity
    }

    /// Returns the stable model identity.
    #[must_use]
    pub const fn model(&self) -> &ModelId {
        self.identity.model()
    }

    /// Returns the stable profile identity.
    #[must_use]
    pub const fn profile(&self) -> &ProfileId {
        self.identity.profile()
    }

    /// Returns exact profile metadata.
    #[must_use]
    pub const fn profile_metadata(&self) -> &OcrProfileMetadata {
        self.identity.profile_metadata()
    }

    /// Returns the detector bytes.
    #[must_use]
    pub fn detector(&self) -> &[u8] {
        self.detector.bytes()
    }

    /// Shares ownership of the detector bytes without copying them.
    #[must_use]
    pub fn shared_detector(&self) -> Arc<[u8]> {
        self.detector.shared_bytes()
    }

    /// Returns the detector's validated identity.
    #[must_use]
    pub const fn detector_identity(&self) -> ModelComponentIdentity {
        self.detector.identity()
    }

    /// Returns the recognizer bytes.
    #[must_use]
    pub fn recognizer(&self) -> &[u8] {
        self.recognizer.bytes()
    }

    /// Shares ownership of the recognizer bytes without copying them.
    #[must_use]
    pub fn shared_recognizer(&self) -> Arc<[u8]> {
        self.recognizer.shared_bytes()
    }

    /// Returns the recognizer's validated identity.
    #[must_use]
    pub const fn recognizer_identity(&self) -> ModelComponentIdentity {
        self.recognizer.identity()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sha2::{Digest, Sha256};

    use super::{
        ACCEPTED_G004_NORMALIZATION_ID, DecoderId, LanguageProfileId, ModelComponentIdentity,
        ModelId, ModelVersion, NormalizationId, OcrModelComponent, OcrModelIdentity,
        OcrModelSource, OcrModelSourceRequest, OcrProfileMetadata, PreprocessingId, ProfileId,
    };
    use crate::OcrFault;

    fn component(bytes: &[u8]) -> ModelComponentIdentity {
        ModelComponentIdentity::new(
            u64::try_from(bytes.len()).unwrap(),
            Sha256::digest(bytes).into(),
        )
        .unwrap()
    }

    fn identity(
        detector: ModelComponentIdentity,
        recognizer: ModelComponentIdentity,
    ) -> OcrModelIdentity {
        OcrModelIdentity::new(
            ModelId::new("model").unwrap(),
            ModelVersion::new("1").unwrap(),
            ProfileId::new("test-profile").unwrap(),
            detector,
            recognizer,
            OcrProfileMetadata::new(
                LanguageProfileId::new("test-language").unwrap(),
                PreprocessingId::new("test-preprocessing").unwrap(),
                DecoderId::new("test-decoder").unwrap(),
                NormalizationId::new(ACCEPTED_G004_NORMALIZATION_ID).unwrap(),
                1,
                [7; 32],
            )
            .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn committed_model_shares_immutable_component_allocations() {
        let detector: Arc<[u8]> = Arc::from(&b"detector"[..]);
        let recognizer: Arc<[u8]> = Arc::from(&b"recognizer"[..]);
        let source = OcrModelSource::new(OcrModelSourceRequest {
            identity: identity(component(&detector), component(&recognizer)),
            detector: OcrModelComponent::new(Arc::clone(&detector), component(&detector)).unwrap(),
            recognizer: OcrModelComponent::new(Arc::clone(&recognizer), component(&recognizer))
                .unwrap(),
        })
        .unwrap();

        assert!(Arc::ptr_eq(&detector, &source.shared_detector()));
        assert!(Arc::ptr_eq(&recognizer, &source.shared_recognizer()));
    }

    #[test]
    fn digest_mismatch_is_refused_before_commit() {
        let bytes: Arc<[u8]> = Arc::from(&b"model"[..]);
        let mut digest: [u8; 32] = Sha256::digest(&bytes).into();
        digest[0] ^= 1;
        let wrong =
            ModelComponentIdentity::new(u64::try_from(bytes.len()).unwrap(), digest).unwrap();

        let fault = OcrModelComponent::new(Arc::clone(&bytes), wrong).unwrap_err();

        assert_eq!(fault, OcrFault::ModelDigestMismatch);
    }

    #[test]
    fn every_closed_profile_field_is_authoritative() {
        for accepted in [
            OcrModelIdentity::accepted_g004(),
            OcrModelIdentity::accepted_bounded_detector(),
        ] {
            let drifted = OcrModelIdentity::new(
                accepted.model().clone(),
                accepted.version().clone(),
                accepted.profile().clone(),
                accepted.detector(),
                accepted.recognizer(),
                OcrProfileMetadata::new(
                    accepted.profile_metadata().language_profile().clone(),
                    accepted.profile_metadata().preprocessing().clone(),
                    accepted.profile_metadata().decoder().clone(),
                    accepted.profile_metadata().normalization().clone(),
                    accepted.profile_metadata().vocabulary_entries(),
                    [0; 32],
                )
                .unwrap(),
            )
            .unwrap_err();

            assert_eq!(drifted, OcrFault::AcceptedProfileMismatch);
        }
    }

    #[test]
    fn cross_profile_preprocessing_is_rejected() {
        let bounded = OcrModelIdentity::accepted_bounded_detector();
        let mismatch = OcrModelIdentity::new(
            bounded.model().clone(),
            bounded.version().clone(),
            bounded.profile().clone(),
            bounded.detector(),
            bounded.recognizer(),
            OcrProfileMetadata::new(
                bounded.profile_metadata().language_profile().clone(),
                PreprocessingId::new(super::ACCEPTED_G004_PREPROCESSING_ID).unwrap(),
                bounded.profile_metadata().decoder().clone(),
                bounded.profile_metadata().normalization().clone(),
                bounded.profile_metadata().vocabulary_entries(),
                bounded.profile_metadata().vocabulary_sha256(),
            )
            .unwrap(),
        )
        .unwrap_err();

        assert_eq!(mismatch, OcrFault::AcceptedProfileMismatch);
    }

    #[test]
    fn candidate_model_or_profile_claims_are_reserved_in_every_build() {
        let candidate = "ppocrv6-tiny-det-small-rec-max736-v1";
        let generic = identity(component(b"detector"), component(b"recognizer"));
        for (model, profile) in [
            (candidate, "generic-profile"),
            ("generic-model", candidate),
            (candidate, candidate),
        ] {
            assert_eq!(
                OcrModelIdentity::new(
                    ModelId::new(model).unwrap(),
                    generic.version().clone(),
                    ProfileId::new(profile).unwrap(),
                    generic.detector(),
                    generic.recognizer(),
                    generic.profile_metadata().clone(),
                ),
                Err(OcrFault::AcceptedProfileMismatch),
            );
        }
        assert!(generic.execution_sha256().is_none());
        assert!(generic.auxiliary().is_none());
    }

    #[test]
    fn auxiliary_commit_requires_exact_presence_and_identity_and_shares_bytes() {
        let detector =
            OcrModelComponent::new(Arc::from(&b"detector"[..]), component(b"detector")).unwrap();
        let recognizer =
            OcrModelComponent::new(Arc::from(&b"recognizer"[..]), component(b"recognizer"))
                .unwrap();
        let bytes: Arc<[u8]> = Arc::from(&b"vocabulary"[..]);
        let auxiliary = OcrModelComponent::new(Arc::clone(&bytes), component(&bytes)).unwrap();
        let mut model = identity(detector.identity(), recognizer.identity());
        // The internal identity fixture represents an auxiliary-backed descriptor;
        // every component still passes real length and SHA-256 validation.
        model.auxiliary = Some(auxiliary.identity());
        let request = || OcrModelSourceRequest {
            identity: model.clone(),
            detector: detector.clone(),
            recognizer: recognizer.clone(),
        };
        assert_eq!(OcrModelSource::new(request()), Err(OcrFault::ModelMismatch));
        let wrong =
            OcrModelComponent::new(Arc::from(&b"changed"[..]), component(b"changed")).unwrap();
        assert_eq!(
            OcrModelSource::new_with_auxiliary(request(), wrong),
            Err(OcrFault::ModelMismatch)
        );
        let source = OcrModelSource::new_with_auxiliary(request(), auxiliary.clone()).unwrap();
        assert!(Arc::ptr_eq(
            &bytes,
            &source.auxiliary().unwrap().shared_bytes()
        ));
        assert_eq!(source.auxiliary().unwrap().identity(), auxiliary.identity());
        let ordinary = OcrModelSourceRequest {
            identity: identity(detector.identity(), recognizer.identity()),
            detector,
            recognizer,
        };
        assert_eq!(
            OcrModelSource::new_with_auxiliary(ordinary, auxiliary),
            Err(OcrFault::ModelMismatch)
        );
    }
}
