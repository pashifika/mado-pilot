//! Public Rust facade for MadoPilot.
//!
//! MadoPilot is a headless visual automation runtime for applications and agents.
//! This package is the normal Rust dependency for a host application: it performs
//! default adapter wiring for the running release target and re-exports a curated
//! subset of the underlying contract packages.
//!
//! # Planned responsibility
//!
//! This package will own the engine and session entry points, typed request
//! builders, default platform and backend selection, and the curated public
//! re-export surface.
//!
//! # Allowed seam
//!
//! This package may depend on the MadoPilot runtime package and on the concrete
//! Windows, macOS, OpenCV, and ONNX adapter packages. It is the only package
//! permitted to name a concrete adapter, and the C ABI package depends on it
//! rather than the reverse.
//!
//! # Implementation status
//!
//! **Nothing is implemented yet, and this package intentionally exposes no
//! operation.** No window discovery, capture, coordinate mapping, template
//! matching, OCR, watcher, input injection, or diagnostic behavior is available,
//! and no such behavior can be reached from this package.
//!
//! `docs/architecture.md` records the Phase 0 repository baseline and the planned
//! package responsibilities. Stable public Rust item names are deliberately not
//! chosen yet; see gate `G-009` in `docs/validation-gates.md`.
