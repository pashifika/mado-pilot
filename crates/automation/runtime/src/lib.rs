//! MadoPilot runtime orchestration.
//!
//! # Planned responsibility
//!
//! This package owns the automation session, query handles, the query scheduler,
//! watchers, bounded work queues with observable drop and coalescing outcomes,
//! deadline and cancellation propagation, result-commit ordering, and diagnostic
//! event emission.
//!
//! # Allowed seam
//!
//! This package may depend on the MadoPilot core, capture, input, vision, OCR,
//! and assets contract packages. It orchestrates those contracts without knowing
//! any concrete platform or backend adapter type, so it never depends on the
//! Windows, macOS, OpenCV, or ONNX packages. Adapter selection belongs to the
//! public facade.
//!
//! # Implementation status
//!
//! Not implemented. This package currently establishes the repository seam only
//! and exposes no operation. No session, watcher, scheduler, or diagnostic event
//! is available. The default change-detection policy remains unresolved; see gate
//! `G-005` in `docs/validation-gates.md`.
