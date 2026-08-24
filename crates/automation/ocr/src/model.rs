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
    LanguageProfileId,
    "A bounded OCR language-profile identifier."
);
identifier!(PreprocessingId, "A bounded OCR preprocessing identifier.");
identifier!(DecoderId, "A bounded OCR decoder identifier.");
identifier!(
    NormalizationId,
    "A bounded OCR result-normalization identifier."
);

/// Accepted G-004 model identity.
pub const ACCEPTED_G004_MODEL_ID: &str = "g-004-rapidocr-ppocrv4-det-v6-rec-small-v1";
/// Accepted G-004 model revision, including RapidOCR source revision.
pub const ACCEPTED_G004_MODEL_VERSION: &str =
    "rapidocr-3.9.2+095232a4c94f7f0e6600ba5bba1177010ad696d4";
/// Accepted G-004 profile identity.
pub const ACCEPTED_G004_PROFILE_ID: &str = "g-004-rapidocr-ppocrv4-det-v6-rec-small-v1";
/// Accepted G-004 language-profile identity.
pub const ACCEPTED_G004_LANGUAGE_PROFILE_ID: &str =
    "horizontal-ja-basic-latin-ascii-digits-ui-symbols-v1";
/// Accepted G-004 preprocessing identity.
pub const ACCEPTED_G004_PREPROCESSING_ID: &str = "rapidocr-ppocrv4-det-bgr-db736-v1";
/// Accepted G-004 decoder identity.
pub const ACCEPTED_G004_DECODER_ID: &str = "rapidocr-ppocrv6-rec-small-greedy-ctc-v1";
/// Accepted G-004 result-normalization identity.
pub const ACCEPTED_G004_NORMALIZATION_ID: &str = "nfc-trim-stable-detector-order-five-decimal-v1";
/// Accepted G-004 embedded vocabulary entry count.
pub const ACCEPTED_G004_VOCABULARY_ENTRIES: u32 = 18_708;
/// Maximum bytes accepted for either OCR model component (64 MiB).
pub const MAX_MODEL_COMPONENT_BYTES: u64 = 64 * 1024 * 1024;

const ACCEPTED_G004_DETECTOR_BYTES: u64 = 4_745_517;
const ACCEPTED_G004_RECOGNIZER_BYTES: u64 = 21_234_383;
const ACCEPTED_G004_DETECTOR_SHA256: [u8; 32] = [
    0xd2, 0xa7, 0x72, 0x0d, 0x45, 0xa5, 0x42, 0x57, 0x20, 0x8b, 0x1e, 0x13, 0xe3, 0x6a, 0x84, 0x79,
    0x89, 0x4c, 0xb7, 0x41, 0x55, 0xa5, 0xef, 0xe2, 0x94, 0x62, 0x51, 0x2d, 0x42, 0xf4, 0x9d, 0xa9,
];
const ACCEPTED_G004_RECOGNIZER_SHA256: [u8; 32] = [
    0x6f, 0x32, 0x72, 0x46, 0xb5, 0x03, 0x88, 0xf3, 0xc1, 0x76, 0xae, 0x30, 0x4b, 0xd9, 0x57, 0x67,
    0xea, 0x6d, 0xc0, 0xc9, 0xae, 0x92, 0x15, 0x3e, 0xf8, 0xcb, 0xe2, 0x10, 0xb3, 0xc1, 0x48, 0x84,
];
const ACCEPTED_G004_VOCABULARY_SHA256: [u8; 32] = [
    0xf7, 0xaa, 0x89, 0x7c, 0xa8, 0x28, 0xa4, 0xc7, 0xc9, 0xe2, 0x73, 0x9c, 0x30, 0xf9, 0x16, 0x1a,
    0x33, 0x30, 0x6d, 0x53, 0x2f, 0x02, 0x0b, 0xcd, 0xb9, 0x1d, 0xcf, 0xb6, 0x64, 0xa5, 0x50, 0x7e,
];

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

    /// Returns the embedded vocabulary entry count.
    #[must_use]
    pub const fn vocabulary_entries(&self) -> u32 {
        self.vocabulary_entries
    }

    /// Returns the embedded vocabulary SHA-256.
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
    profile_metadata: OcrProfileMetadata,
}

impl OcrModelIdentity {
    /// Builds a complete model identity and enforces accepted-profile authority.
    ///
    /// # Errors
    ///
    /// Returns [`OcrFault::AcceptedProfileMismatch`] when a value claims the
    /// accepted G-004 model or profile ID but any bound field differs from ADR 0033.
    pub fn new(
        model: ModelId,
        version: ModelVersion,
        profile: ProfileId,
        detector: ModelComponentIdentity,
        recognizer: ModelComponentIdentity,
        profile_metadata: OcrProfileMetadata,
    ) -> Result<Self, OcrFault> {
        let identity = Self {
            model,
            version,
            profile,
            detector,
            recognizer,
            profile_metadata,
        };
        identity.validate_profile_authority()?;
        Ok(identity)
    }

    /// Builds the exact accepted G-004 identity from ADR 0033 constants.
    #[must_use]
    pub fn accepted_g004() -> Self {
        Self::new(
            ModelId::new(ACCEPTED_G004_MODEL_ID).expect("accepted model identity"),
            ModelVersion::new(ACCEPTED_G004_MODEL_VERSION).expect("accepted model version"),
            ProfileId::new(ACCEPTED_G004_PROFILE_ID).expect("accepted profile identity"),
            ModelComponentIdentity::new(
                ACCEPTED_G004_DETECTOR_BYTES,
                ACCEPTED_G004_DETECTOR_SHA256,
            )
            .expect("accepted detector identity"),
            ModelComponentIdentity::new(
                ACCEPTED_G004_RECOGNIZER_BYTES,
                ACCEPTED_G004_RECOGNIZER_SHA256,
            )
            .expect("accepted recognizer identity"),
            OcrProfileMetadata::new(
                LanguageProfileId::new(ACCEPTED_G004_LANGUAGE_PROFILE_ID)
                    .expect("accepted language profile"),
                PreprocessingId::new(ACCEPTED_G004_PREPROCESSING_ID)
                    .expect("accepted preprocessing"),
                DecoderId::new(ACCEPTED_G004_DECODER_ID).expect("accepted decoder"),
                NormalizationId::new(ACCEPTED_G004_NORMALIZATION_ID)
                    .expect("accepted normalization"),
                ACCEPTED_G004_VOCABULARY_ENTRIES,
                ACCEPTED_G004_VOCABULARY_SHA256,
            )
            .expect("accepted profile metadata"),
        )
        .expect("accepted G-004 constants are self-consistent")
    }

    fn validate_profile_authority(&self) -> Result<(), OcrFault> {
        if self.profile.as_str() != ACCEPTED_G004_PROFILE_ID
            && self.model.as_str() != ACCEPTED_G004_MODEL_ID
        {
            return Ok(());
        }
        if self.model.as_str() != ACCEPTED_G004_MODEL_ID
            || self.profile.as_str() != ACCEPTED_G004_PROFILE_ID
            || self.version.as_str() != ACCEPTED_G004_MODEL_VERSION
            || self.detector.byte_len() != ACCEPTED_G004_DETECTOR_BYTES
            || self.detector.sha256() != ACCEPTED_G004_DETECTOR_SHA256
            || self.recognizer.byte_len() != ACCEPTED_G004_RECOGNIZER_BYTES
            || self.recognizer.sha256() != ACCEPTED_G004_RECOGNIZER_SHA256
            || self.profile_metadata.language_profile.as_str() != ACCEPTED_G004_LANGUAGE_PROFILE_ID
            || self.profile_metadata.preprocessing.as_str() != ACCEPTED_G004_PREPROCESSING_ID
            || self.profile_metadata.decoder.as_str() != ACCEPTED_G004_DECODER_ID
            || self.profile_metadata.normalization.as_str() != ACCEPTED_G004_NORMALIZATION_ID
            || self.profile_metadata.vocabulary_entries != ACCEPTED_G004_VOCABULARY_ENTRIES
            || self.profile_metadata.vocabulary_sha256 != ACCEPTED_G004_VOCABULARY_SHA256
        {
            return Err(OcrFault::AcceptedProfileMismatch);
        }
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

/// An immutable, digest-validated OCR detector/recognizer pair.
///
/// Cloning shares identity and component allocations. A backend can therefore
/// retain a source without copying model bytes per request or session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrModelSource {
    identity: OcrModelIdentity,
    detector: OcrModelComponent,
    recognizer: OcrModelComponent,
}

impl OcrModelSource {
    /// Maximum bytes accepted for either model component.
    pub const MAX_COMPONENT_BYTES: u64 = MAX_MODEL_COMPONENT_BYTES;

    /// Commits two independently validated model components.
    ///
    /// # Errors
    ///
    /// Returns [`OcrFault::ModelMismatch`] when either component identity differs
    /// from the complete model identity. No source is published on failure.
    pub fn new(request: OcrModelSourceRequest) -> Result<Self, OcrFault> {
        if request.detector.identity() != request.identity.detector()
            || request.recognizer.identity() != request.identity.recognizer()
        {
            return Err(OcrFault::ModelMismatch);
        }
        Ok(Self {
            identity: request.identity,
            detector: request.detector,
            recognizer: request.recognizer,
        })
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
    fn accepted_profile_id_rejects_any_metadata_drift() {
        let accepted = OcrModelIdentity::accepted_g004();
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
