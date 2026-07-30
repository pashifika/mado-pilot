//! MadoPilot OCR contracts.
//!
//! # Planned responsibility
//!
//! This package owns the OCR backend contract, model source descriptors, OCR
//! requests and results with their source-frame correlation, and text
//! normalization rules.
//!
//! # Allowed seam
//!
//! This package may depend on the MadoPilot core, capture, and vision packages.
//! The capture dependency exists because public OCR operations consume
//! capture-owned frame views, and the vision dependency exists because OCR reuses
//! the shared preprocessing descriptors. ONNX Runtime and other OCR backends
//! implement these contracts; this package never depends on them.
//!
//! # Implementation status
//!
//! Not implemented. This package currently establishes the repository seam only
//! and exposes no operation. No model is bundled, loaded, or recognized. The
//! default model, language set, and preprocessing profile remain unresolved; see
//! gate `G-004` in `docs/validation-gates.md`.
