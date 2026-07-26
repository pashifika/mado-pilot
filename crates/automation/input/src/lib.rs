//! MadoPilot input contracts.
//!
//! # Planned responsibility
//!
//! This package owns the input adapter contract and keeps the input operation
//! kind and the delivery mechanism as separate axes. It also owns focus policy,
//! input receipts including partial sequence execution, and input error types.
//!
//! # Allowed seam
//!
//! This package may depend on the MadoPilot core package only. Platform input
//! adapters implement these contracts; this package never depends on them and
//! never selects a delivery mechanism on a caller's behalf.
//!
//! # Implementation status
//!
//! Not implemented. This package currently establishes the repository seam only
//! and exposes no operation. No input is injected and no platform capability is
//! reported. See `docs/architecture.md`.
