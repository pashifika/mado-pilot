//! MadoPilot deterministic test support.
//!
//! # Responsibility
//!
//! Doubles and shared contract suites that let every adapter be exercised the
//! same way: a capture provider a test drives by hand, and the capture contract
//! suite both it and the production replay adapter must pass.
//!
//! Two implementations is the point. A contract that only one adapter satisfied
//! would be a description of that adapter, and the paths that matter most —
//! waiting for a frame that has not arrived, cancelling mid-wait, closing under
//! a waiter — are unreachable through an adapter whose sequence is already
//! known.
//!
//! # Allowed seam
//!
//! May depend on the core, capture, input, vision, and OCR contract packages so
//! that it can double them. A production package may reference it only as a
//! development dependency; it must never become a production dependency of
//! anything, because test support must not ship.
//!
//! # Implementation status
//!
//! Phase 1 stage 2. Capture doubles and the capture contract suite exist. Input,
//! vision, and OCR doubles and target lifecycle scripts do not.

pub mod capture_contract;
pub mod controlled_capture;

pub use controlled_capture::ControlledCapture;
