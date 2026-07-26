//! MadoPilot deterministic test support.
//!
//! # Planned responsibility
//!
//! This package will own replay capture sources, fake input adapters, synthetic
//! clocks, backend doubles with controlled latency and failure, target lifecycle
//! scripts, and the shared contract-test fixtures that every adapter suite runs.
//!
//! # Allowed seam
//!
//! This package may depend on the MadoPilot core, capture, input, vision, and OCR
//! contract packages so that it can provide doubles for them. A production
//! package may reference it only as a development dependency; it must never
//! become a production dependency of any package.
//!
//! # Implementation status
//!
//! Not implemented. This package currently establishes the repository seam only
//! and exposes no operation. No fixture, double, or replay source is available.
//! See `docs/architecture.md`.
