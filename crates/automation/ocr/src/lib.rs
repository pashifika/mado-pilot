//! Platform-neutral MadoPilot OCR contracts.
//!
//! This crate owns bounded backend/model/profile identities, immutable validated
//! model sources, exact-frame requests, backend candidate validation, accepted
//! G-004 normalization and ordering, immutable source-correlated results, and
//! deadline/cancellation-aware commit.
//!
//! It depends only on platform-neutral core and capture contracts. It exposes no
//! ONNX Runtime, platform, executor, facade, C ABI, or C++ type and performs no
//! model discovery, download, default wiring, or inference itself.
//!
//! [`OcrRecognizer`] maps one borrowed immutable frame region in the selected
//! backend format, applies every public result rule after the backend returns,
//! and owns all committed text, geometry, identity, and transform data. Retained
//! results therefore do not retain backend buffers or capture producer slots.
//!
//! The accepted profile normalizes text to NFC, trims leading/trailing Unicode
//! whitespace, omits empty normalized text, rounds finite confidence in
//! `0.0..=1.0` to five decimals, and orders at most 1,000 regions by the
//! backend's unique stable detector order.

pub mod backend;
pub mod fault;
pub mod model;
pub mod recognizer;
pub mod request;
pub mod result;

pub use backend::{BackendCandidate, BackendRequest, OcrBackend, OcrBackendDescriptor};
pub use fault::OcrFault;
pub use model::{
    BackendId, BackendVersion, ModelComponentIdentity, ModelId, OcrModelSource,
    OcrModelSourceRequest, ProfileId,
};
pub use recognizer::{MAX_CANDIDATES, MAX_TEXT_BYTES, OcrRecognizer};
pub use request::{OcrRegion, OcrRequest};
pub use result::{Confidence, OcrQuadrilateral, OcrResult, RecognizedRegion};
