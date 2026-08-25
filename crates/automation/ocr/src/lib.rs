//! Platform-neutral MadoPilot OCR contracts.
//!
//! This crate owns bounded backend/model/profile identities, immutable validated
//! model sources, exact-frame singular and grouped requests, backend candidate
//! validation, accepted G-004 normalization and ordering, immutable
//! source-correlated results, and deadline/cancellation-aware commit.
//!
//! It depends only on platform-neutral core and capture contracts. It exposes no
//! ONNX Runtime, platform, executor, facade, C ABI, or C++ type and performs no
//! model discovery, download, default wiring, or inference itself.
//!
//! [`OcrRecognizer::recognize`] maps one borrowed immutable frame region.
//! [`OcrRecognizer::scan_zones`] resolves one through eight caller-order zones,
//! maps their smallest capture-pixel envelope once, and carries relative
//! interests through the unchanged backend trait. Both accept borrowed
//! candidates through a core-owned bounded sink and commit atomically without
//! retaining backend buffers or capture producer slots.
//!
//! The supported normalization profile accepts at most 1,000 candidates and 16
//! KiB of raw text per candidate, normalizes text to NFC, trims leading/trailing
//! Unicode whitespace, omits empty text, limits retained text to 4 KiB, rounds
//! finite confidence in `0.0..=1.0` to five decimals, and orders by the backend's
//! unique stable detector order. Grouped results apply exact half-open centroid
//! membership, own each relevant candidate once, expose explicit empty groups,
//! and bound one through eight zones to at most 8,000 compact memberships.

pub mod backend;
pub mod fault;
pub mod model;
pub mod recognizer;
pub mod request;
pub mod result;

pub use backend::{
    BackendCandidate, BackendInterests, BackendRequest, OcrBackend, OcrBackendDescriptor,
    OcrBackendIdentity, OcrCandidateSink, candidate_interest_membership,
};
pub use fault::OcrFault;
pub use model::{
    ACCEPTED_BOUNDED_MODEL_ID, ACCEPTED_BOUNDED_MODEL_VERSION, ACCEPTED_BOUNDED_PREPROCESSING_ID,
    ACCEPTED_BOUNDED_PROFILE_ID, ACCEPTED_G004_DECODER_ID, ACCEPTED_G004_LANGUAGE_PROFILE_ID,
    ACCEPTED_G004_MODEL_ID, ACCEPTED_G004_MODEL_VERSION, ACCEPTED_G004_NORMALIZATION_ID,
    ACCEPTED_G004_PREPROCESSING_ID, ACCEPTED_G004_PROFILE_ID, ACCEPTED_G004_VOCABULARY_ENTRIES,
    BackendId, BackendVersion, DecoderId, LanguageProfileId, MAX_MODEL_COMPONENT_BYTES,
    ModelComponentIdentity, ModelId, ModelVersion, NormalizationId, OcrModelComponent,
    OcrModelIdentity, OcrModelSource, OcrModelSourceRequest, OcrProfileMetadata, PreprocessingId,
    ProfileId,
};
pub use recognizer::{MAX_BACKEND_TEXT_BYTES, MAX_CANDIDATES, MAX_TEXT_BYTES, OcrRecognizer};
pub use request::{MAX_OCR_ZONES, OcrRegion, OcrRequest, OcrZone, OcrZoneScanRequest};
pub use result::{
    Confidence, OcrQuadrilateral, OcrResult, OcrZoneGroup, OcrZoneScanResult, RecognizedRegion,
};
