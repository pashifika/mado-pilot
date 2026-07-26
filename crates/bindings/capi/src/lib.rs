//! Planned MadoPilot C ABI boundary.
//!
//! # Planned responsibility
//!
//! This package will own the separately versioned C ABI: product-prefixed
//! `extern "C"` functions, fixed-width types, opaque handles with a complete
//! lifecycle, explicit pointer-length views, size-versioned structures, a
//! versioned function table, module-owned allocation and release, callback
//! dispatch with an unregistration fence, and panic and native-exception
//! containment at the boundary.
//!
//! # Allowed seam
//!
//! This package may depend on the `mado-pilot` facade package only. The facade
//! never depends on this package, and this package never reaches past the facade
//! into the runtime, platform, or backend packages.
//!
//! # Implementation status
//!
//! **Nothing is implemented yet.** This package exposes no `extern "C"` function,
//! no exported symbol, and no status code, and Phase 0 produces no C header, no
//! import library, and no shared or static native library. The reserved public
//! artifact names in `docs/architecture.md` are reservations only.
//!
//! The exact version-one status codes, mandatory function-table prefix, structure
//! layouts, and Rust-error mapping remain unresolved; see gate `G-010` in
//! `docs/validation-gates.md`.
