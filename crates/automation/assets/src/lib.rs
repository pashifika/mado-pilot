//! MadoPilot asset contracts.
//!
//! # Planned responsibility
//!
//! This package owns the versioned asset manifest schema, manifest and entry
//! validation, deterministic and network-free loading from directory, memory, and
//! archive sources, and resolution of validated entries into vision template and
//! OCR model source descriptors.
//!
//! # Allowed seam
//!
//! This package may depend on the MadoPilot core, vision, and OCR contract
//! packages. Vision and OCR never depend on asset-package representations, so a
//! caller may supply direct file or memory sources without adopting the asset
//! manifest.
//!
//! # Implementation status
//!
//! Not implemented. This package currently establishes the repository seam only
//! and exposes no operation. No manifest is parsed, validated, or loaded. Archive
//! entry, byte, and compression-ratio safety ceilings remain unresolved; see gate
//! `G-014` in `docs/validation-gates.md`.
