//! Platform-neutral MadoPilot core contracts.
//!
//! # Planned responsibility
//!
//! This package owns the vocabulary that every other MadoPilot package shares:
//! target and stream identities, geometry and coordinate spaces, monotonic time,
//! operation deadlines and cancellation, capability descriptions, and the shared
//! error and status types.
//!
//! # Allowed seam
//!
//! This package depends on no other MadoPilot package, and on no Windows, macOS,
//! OpenCV, ONNX Runtime, GUI, or async-executor type. Platform-native handles are
//! never added here.
//!
//! # Implementation status
//!
//! Not implemented. This package currently establishes the repository seam only
//! and exposes no operation. `docs/architecture.md` records the Phase 0 baseline,
//! and `docs/validation-gates.md` records the decisions that must be resolved
//! before the contracts in this package are frozen.
