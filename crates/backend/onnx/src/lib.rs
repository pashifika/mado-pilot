//! Planned MadoPilot ONNX Runtime OCR backend.
//!
//! # Planned responsibility
//!
//! This package will implement OCR inference on top of ONNX Runtime, including
//! explicit execution-provider selection with observable rejected candidates,
//! model metadata validation, and actionable backend-loading failures.
//!
//! # Allowed seam
//!
//! This package may depend on the MadoPilot core, vision, and OCR contract
//! packages. It implements the OCR contract and is wired in by the public facade;
//! no contract package depends on it.
//!
//! # Implementation status
//!
//! Not implemented. This package currently establishes the repository seam only
//! and exposes no operation, and it declares no ONNX Runtime dependency, so the
//! workspace builds without a native ONNX Runtime installation. No model is
//! bundled or downloaded, and nothing here performs network access.
//!
//! The default model profile, acceleration candidates, provider ordering, and
//! native packaging remain unresolved; see gates `G-004`, `G-006`, `G-007`, and
//! `G-008` in `docs/validation-gates.md`.
