//! Planned MadoPilot macOS platform adapter.
//!
//! # Planned responsibility
//!
//! This package will own macOS target and display discovery, ScreenCaptureKit
//! streams with their native frame lifetime, Retina and multi-display coordinate
//! transforms, non-prompting Screen Recording and Accessibility permission
//! probes reported separately, and `CGEvent` input. Any Objective-C or
//! Objective-C++ shim stays narrow and internal to this package.
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
//! The minimum supported macOS version and the shim language choice remain
//! unresolved; see gates `G-001` and `G-003` in `docs/validation-gates.md`.
