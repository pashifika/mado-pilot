//! MadoPilot deterministic test support.
//!
//! # Responsibility
//!
//! Doubles and shared contract suites that let every adapter be exercised the
//! same way: a capture provider a test drives by hand, and the capture contract
//! suite both it and the production replay adapter must pass. It also holds the
//! measurement scaffolding the two benchmark targets share, so that the profile
//! format `docs/performance.md` defines has one printer rather than two.
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
//! Phase 1 complete, and the Phase 2 input support with it. Capture, storage,
//! vision, permission, and input doubles, a manual clock, a template-image
//! writer, the matching fixture scene, the capture, vision, and input contract
//! suites, and the benchmark harness exist. The input double records the policies
//! each admitted sequence carried, so a layer that substituted one is visible
//! rather than merely improbable. OCR doubles and target lifecycle scripts do
//! not exist.

pub mod bench_harness;
pub mod capture_contract;
pub mod clock;
pub mod controlled_capture;
pub mod controlled_input;
pub mod controlled_matcher;
pub mod controlled_storage;
pub mod fixture_checksums;
pub mod input_contract;
pub mod match_fixtures;
pub mod png;
pub mod scripted_permission;
pub mod vision_contract;

pub use clock::ManualClock;
pub use controlled_capture::ControlledCapture;
pub use controlled_input::ControlledInput;
pub use controlled_matcher::{Behavior, ControlledMatcher};
pub use controlled_storage::{ControlledProducer, Conversion};
pub use scripted_permission::{Answer, ScriptedPermissionProbe};
