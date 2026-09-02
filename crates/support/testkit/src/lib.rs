//! MadoPilot deterministic test support.
//!
//! # Responsibility
//!
//! Doubles and shared contract suites that let every adapter be exercised the
//! same way: a capture provider a test drives by hand, and the capture contract
//! suite both it and the production replay adapter must pass. It also holds the
//! measurement scaffolding every in-process benchmark target shares, so the
//! profile format `docs/performance.md` defines has one printer rather than one
//! per workload family.
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
//! Phase 1 and Phase 2 support remains complete. Capture, storage, vision, OCR,
//! permission, and input doubles, manual and controlled clocks, shared contract
//! suites, fixture writers, allocation accounting, and the benchmark harness
//! exist. `ControlledOcr` and `ControlledMatcher` script candidates, failures,
//! latency, cancellation, exact admission/completion gates, and completion
//! counters so deadline, close, coalescing, fairness, stale-generation, and
//! out-of-order behavior are deterministic. Phase 4 also provides the strict,
//! bounded, content-redacted offline `G-005` recorded-sequence evaluator; all of
//! this remains test/evidence support and is never a production dependency.
//! Target lifecycle scripts do not exist.

pub mod bench_harness;
pub mod capture_contract;
pub mod change_detection;
pub mod clock;
pub mod controlled_capture;
pub mod controlled_input;
pub mod controlled_matcher;
pub mod controlled_ocr;
pub mod controlled_storage;
pub mod fixture_checksums;
pub mod input_contract;
pub mod match_fixtures;
pub mod native_watch_report;
pub mod ocr_contract;
pub mod png;
pub mod scripted_permission;
pub mod vision_contract;
pub mod visual_token;

pub use clock::ManualClock;
pub use controlled_capture::ControlledCapture;
pub use controlled_input::ControlledInput;
pub use controlled_matcher::{Behavior, ControlledMatcher, ObservedMatcher, ScriptedMatchCall};
pub use controlled_ocr::{
    CONTROLLED_OCR_BACKEND, CONTROLLED_OCR_MODEL, CONTROLLED_OCR_PROFILE, CompletionGate,
    CompletionGateReleaseGuard, ControlledOcr, OcrBehavior, ScriptedOcrCall, ScriptedOcrCandidate,
};
pub use controlled_storage::{ControlledProducer, Conversion};
pub use mado_pilot_vision::{Candidate, MatchBackend};
pub use scripted_permission::{Answer, ScriptedPermissionProbe};
