//! Planned MadoPilot macOS platform adapter.
//!
//! # Planned responsibility
//!
//! This package will own macOS target and display discovery, ScreenCaptureKit
//! streams with their native frame lifetime, Retina and multi-display coordinate
//! transforms, non-prompting Screen Recording and Accessibility permission
//! probes reported separately, and `CGEvent` input. Its Objective-C shim stays
//! narrow and internal to this package, and exposes no Objective-C type through
//! any Rust or public API.
//!
//! # Allowed seam
//!
//! This package may depend on the MadoPilot core, capture, and input contract
//! packages. It implements those contracts and is wired in by the public facade;
//! no contract package depends on it.
//!
//! # Implementation status
//!
//! Not implemented. This package currently establishes the repository seam only
//! and exposes no operation, and it declares no macOS dependency. Nothing here
//! requests a permission or presents permission UI. The `aarch64-apple-darwin`
//! release target is verified natively in continuous integration at the build
//! level only.
//!
//! The shim language and its containment rules are settled:
//! `docs/adr/0012-macos-shim-language-and-containment.md` resolves gate `G-003`
//! and selects Objective-C with ARC, compiled with `-fobjc-arc-exceptions`. No
//! package implements that boundary yet, so the ADR is enforced by review until
//! this one does. The minimum supported macOS version remains unresolved; see
//! gate `G-001` in `docs/validation-gates.md`.
