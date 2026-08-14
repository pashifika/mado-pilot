//! MadoPilot input contracts.
//!
//! # Responsibility
//!
//! This package says what an input request is and what an Adapter must report
//! about one. It keeps the operation kind and the delivery mechanism as separate
//! axes, owns focus and geometry policy, owns the receipt including partial
//! execution and cleanup, and owns the input error type.
//!
//! # Why one execute operation
//!
//! There is one [`InputController::execute`] over a typed [`InputRequest`]
//! rather than a method per primitive.
//! Delivery selection, admission, geometry resolution, deadline arbitration,
//! partial receipts, and cleanup are identical for a click, a keystroke, and a
//! phrase; five methods would have had five copies of all of it, and a caller
//! sending a chord would have had no way to say that the modifier and the key
//! belong to one sequence.
//!
//! # What this package will not do for a caller
//!
//! It never picks a delivery mechanism the caller did not permit. Substituting
//! system input for a target-directed route focuses a window the caller explicitly
//! asked not to disturb and injects into whatever is focused instead, so a
//! substitution the caller did not authorize is refused rather than guessed at.
//! It also never claims a sequence is atomic: an operating system cannot recall an
//! event that may already have native effect, so partial execution is reported
//! rather than hidden.
//!
//! # Allowed seam
//!
//! This package depends on the MadoPilot core package only. Platform input
//! adapters implement these contracts; this package never depends on them, and
//! nothing here names a virtual-key code, an event tap, or a window message.
//!
//! # Implementation status
//!
//! Phase 2 contracts, complete. The event, policy, sequence, descriptor, request,
//! receipt, and provider/controller contracts below are implemented and tested,
//! along with the shared admission rule, the per-controller serialization every
//! Adapter uses, and the bounds cleanup runs under. Both platform Adapters
//! implement them, `mado-pilot-runtime` composes one of them with a capture
//! provider of the same identity, and the Rust facade's native constructors are
//! what a caller reaches all of it through. No C ABI or C++ entry reaches this
//! package. See `docs/architecture.md`.
//!
//! **The public names here are reviewed, not yet stable.**
//! `docs/adr/0006-public-rust-names-and-compatibility-policy.md` records the
//! policy that applies: renaming or removing one is a breaking change needing an
//! ADR and a version bump, while adding is free.

pub mod cleanup;
pub mod controller;
pub mod descriptor;
pub mod event;
pub mod fault;
pub mod policy;
pub mod receipt;
pub mod request;

pub use cleanup::CleanupBudget;
pub use controller::{
    Admission, AdmissionGuard, InputController, InputOpenRequest, InputProvider, InputRequirement,
    check_provider_pair,
};
pub use descriptor::InputDescriptor;
pub use event::{InputEvent, Key, Modifier, PointerButton, PressedState};
pub use fault::InputFault;
pub use policy::{DeliveryPlan, FocusPolicy, GeometryPolicy, PointerGeometry};
pub use receipt::{
    CleanupState, InputAttempt, InputEventObservation, InputExecution, InputGeometryResult,
    InputReceipt, InputRevalidationCategory, SequenceOutcome,
};
pub use request::{InputRequest, InputSequence, SequenceLimits};
