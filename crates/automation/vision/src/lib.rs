//! MadoPilot vision contracts.
//!
//! # Planned responsibility
//!
//! This package owns the template-matching backend contract, template source
//! descriptors, preprocessing descriptors, and matching requests and results
//! including their correlation with a source frame identity.
//!
//! # Allowed seam
//!
//! This package may depend on the MadoPilot core and capture packages. The
//! capture dependency exists because public matching operations consume
//! capture-owned frame views; it is a contract-to-contract dependency and
//! exposes no adapter type. OpenCV and other vision backends implement these
//! contracts; this package never depends on them.
//!
//! # Implementation status
//!
//! Not implemented. This package currently establishes the repository seam only
//! and exposes no operation. No template matching is available. See
//! `docs/architecture.md`.
