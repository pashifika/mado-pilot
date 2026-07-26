//! MadoPilot capture, frame, and mapping contracts.
//!
//! # Planned responsibility
//!
//! This package owns the capture provider and capture session contracts, the
//! immutable frame and frame-view model, frame identity and geometry revisions,
//! CPU mapping requests, and stream policy descriptions. It defines what a
//! capture adapter must provide without describing how any platform provides it.
//!
//! # Allowed seam
//!
//! This package may depend on the MadoPilot core package only. Platform capture
//! adapters implement these contracts; this package never depends on them.
//!
//! # Implementation status
//!
//! Not implemented. This package currently establishes the repository seam only
//! and exposes no operation. No capture backend, frame stream, or mapping
//! behavior is available. See `docs/architecture.md`.
