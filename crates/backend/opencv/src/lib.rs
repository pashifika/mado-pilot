//! Planned MadoPilot OpenCV vision backend.
//!
//! # Planned responsibility
//!
//! This package will implement template matching and CPU image preprocessing on
//! top of OpenCV, and will report actionable backend-loading failures instead of
//! relying on an unrestricted ambient library search.
//!
//! # Allowed seam
//!
//! This package may depend on the MadoPilot core and vision contract packages. It
//! implements the vision contract and is wired in by the public facade; no
//! contract package depends on it.
//!
//! # Implementation status
//!
//! Not implemented. This package currently establishes the repository seam only
//! and exposes no operation, and it declares no OpenCV dependency, so the
//! workspace builds without a native OpenCV installation.
//!
//! Native dependency bundling, license review, deployment profiles, and
//! static-library feasibility remain unresolved; see gates `G-007` and `G-008` in
//! `docs/validation-gates.md`.
