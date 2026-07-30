//! Planned MadoPilot Windows platform adapter.
//!
//! # Planned responsibility
//!
//! This package will own Windows target and display discovery, Windows Graphics
//! Capture streams with their Direct3D 11 resource lifetime, coordinate mapping
//! across mixed-DPI displays, and the explicit system and background input
//! implementations together with their reported capabilities and permissions.
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
//! and exposes no operation, and it declares no Windows dependency. The
//! `x86_64-pc-windows-msvc` release target is verified natively in continuous
//! integration at the build level only.
//!
//! The minimum supported Windows version and the capture frame-pool ownership
//! strategy remain unresolved; see gates `G-001` and `G-002` in
//! `docs/validation-gates.md`.
