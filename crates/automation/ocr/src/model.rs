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
identifier!(
    ProfileId,
    "A stable OCR normalization and decoder profile identifier."
);

/// The expected immutable identity of one model component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModelComponentIdentity {
    byte_len: u64,
    sha256: [u8; 32],
}

impl ModelComponentIdentity {
    /// Builds an expected component identity.
    #[must_use]
    pub const fn new(byte_len: u64, sha256: [u8; 32]) -> Self {
        Self { byte_len, sha256 }
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

/// Inputs used to commit one immutable OCR model source.
#[derive(Debug)]
pub struct OcrModelSourceRequest {
    /// Stable model identity.
    pub model: ModelId,
    /// Stable normalization/preprocessing/decoder profile identity.
    pub profile: ProfileId,
    /// Detector component bytes.
    pub detector: Arc<[u8]>,
    /// Exact detector component identity.
    pub detector_identity: ModelComponentIdentity,
    /// Recognizer component bytes.
    pub recognizer: Arc<[u8]>,
    /// Exact recognizer component identity.
    pub recognizer_identity: ModelComponentIdentity,
}

/// An immutable, digest-validated OCR detector/recognizer pair.
///
/// Cloning shares the committed component allocations. A backend can therefore
/// retain a source without copying model bytes per request or session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrModelSource {
    model: ModelId,
    profile: ProfileId,
    detector: Arc<[u8]>,
    detector_identity: ModelComponentIdentity,
    recognizer: Arc<[u8]>,
    recognizer_identity: ModelComponentIdentity,
}

impl OcrModelSource {
    /// Maximum bytes accepted for either model component (64 MiB).
    pub const MAX_COMPONENT_BYTES: u64 = 64 * 1024 * 1024;

    /// Validates and commits immutable model component bytes.
    ///
    /// # Errors
    ///
    /// Returns a typed model fault for empty, over-limit, length-mismatched, or
    /// digest-mismatched component bytes. No source is published on failure.
    pub fn new(request: OcrModelSourceRequest) -> Result<Self, OcrFault> {
        validate_component(&request.detector, request.detector_identity)?;
        validate_component(&request.recognizer, request.recognizer_identity)?;
        Ok(Self {
            model: request.model,
            profile: request.profile,
            detector: request.detector,
            detector_identity: request.detector_identity,
            recognizer: request.recognizer,
            recognizer_identity: request.recognizer_identity,
        })
    }

    /// Returns the stable model identity.
    #[must_use]
    pub const fn model(&self) -> &ModelId {
        &self.model
    }

    /// Returns the stable profile identity.
    #[must_use]
    pub const fn profile(&self) -> &ProfileId {
        &self.profile
    }

    /// Returns the detector bytes.
    #[must_use]
    pub fn detector(&self) -> &[u8] {
        &self.detector
    }

    /// Shares ownership of the detector bytes without copying them.
    #[must_use]
    pub fn shared_detector(&self) -> Arc<[u8]> {
        Arc::clone(&self.detector)
    }

    /// Returns the detector's validated identity.
    #[must_use]
    pub const fn detector_identity(&self) -> ModelComponentIdentity {
        self.detector_identity
    }

    /// Returns the recognizer bytes.
    #[must_use]
    pub fn recognizer(&self) -> &[u8] {
        &self.recognizer
    }

    /// Shares ownership of the recognizer bytes without copying them.
    #[must_use]
    pub fn shared_recognizer(&self) -> Arc<[u8]> {
        Arc::clone(&self.recognizer)
    }

    /// Returns the recognizer's validated identity.
    #[must_use]
    pub const fn recognizer_identity(&self) -> ModelComponentIdentity {
        self.recognizer_identity
    }
}

fn validate_component(bytes: &[u8], identity: ModelComponentIdentity) -> Result<(), OcrFault> {
    if bytes.is_empty() {
        return Err(OcrFault::EmptyModelComponent);
    }
    let actual_len =
        u64::try_from(bytes.len()).map_err(|_| OcrFault::ModelComponentAboveCeiling)?;
    if actual_len > OcrModelSource::MAX_COMPONENT_BYTES {
        return Err(OcrFault::ModelComponentAboveCeiling);
    }
    if actual_len != identity.byte_len() {
        return Err(OcrFault::ModelLengthMismatch);
    }
    let actual: [u8; 32] = Sha256::digest(bytes).into();
    if actual != identity.sha256() {
        return Err(OcrFault::ModelDigestMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sha2::{Digest, Sha256};

    use super::{
        ModelComponentIdentity, ModelId, OcrModelSource, OcrModelSourceRequest, ProfileId,
    };
    use crate::OcrFault;

    fn identity(bytes: &[u8]) -> ModelComponentIdentity {
        ModelComponentIdentity::new(bytes.len() as u64, Sha256::digest(bytes).into())
    }

    #[test]
    fn committed_model_shares_immutable_component_allocations() {
        let detector: Arc<[u8]> = Arc::from(&b"detector"[..]);
        let recognizer: Arc<[u8]> = Arc::from(&b"recognizer"[..]);
        let source = OcrModelSource::new(OcrModelSourceRequest {
            model: ModelId::new("model").unwrap(),
            profile: ProfileId::new("profile").unwrap(),
            detector: Arc::clone(&detector),
            detector_identity: identity(&detector),
            recognizer: Arc::clone(&recognizer),
            recognizer_identity: identity(&recognizer),
        })
        .unwrap();

        assert!(Arc::ptr_eq(&detector, &source.shared_detector()));
        assert!(Arc::ptr_eq(&recognizer, &source.shared_recognizer()));
    }

    #[test]
    fn digest_mismatch_is_refused_before_commit() {
        let bytes: Arc<[u8]> = Arc::from(&b"model"[..]);
        let mut wrong = identity(&bytes);
        wrong.sha256[0] ^= 1;

        let fault = OcrModelSource::new(OcrModelSourceRequest {
            model: ModelId::new("model").unwrap(),
            profile: ProfileId::new("profile").unwrap(),
            detector: Arc::clone(&bytes),
            detector_identity: wrong,
            recognizer: Arc::clone(&bytes),
            recognizer_identity: identity(&bytes),
        })
        .unwrap_err();

        assert_eq!(fault, OcrFault::ModelDigestMismatch);
    }
}
