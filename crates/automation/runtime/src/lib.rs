//! MadoPilot runtime orchestration.
//!
//! # Responsibility
//!
//! This package composes the capture, input, asset, and vision contracts into
//! the operations a caller actually performs. What lives here is what none of
//! those contracts can decide alone: acquiring a frame and searching it as one
//! operation, refusing a frame that belongs to another session, building the
//! authoritative envelope that says which target and which exact frame a result
//! is about, admitting capture and input as one session with one lifecycle, and
//! applying the final commit check that stops late work from becoming an
//! observable success.
//!
//! It deliberately does not re-implement what a contract already owns. Frame
//! identity and ordering stay in `mado-pilot-capture`, what a match *is* stays
//! in `mado-pilot-vision`, what makes a package valid stays in
//! `mado-pilot-assets`, and the per-event work of delivering a sequence —
//! selecting a permitted mechanism, arbitrating focus, resolving a coordinate
//! against the target's live geometry, revalidating before each irreversible
//! event, and releasing what a stopped sequence pressed — stays in
//! `mado-pilot-input` and the Adapter that implements it. That seam is not a
//! preference: an engine cannot observe which adapter is behind a contract, and
//! per-event revalidation is a question only the adapter holding the native
//! target can answer.
//!
//! What this package adds over that delegation is the composition: one capture
//! provider paired with an input provider of the same identity, an open that
//! releases what it already committed when a later step refuses, a session that
//! refuses a request addressed to another target or carrying another stream's
//! frame, one terminal outcome per sequence that preserves its route-threshold
//! accounting, and one close that drains both sides.
//!
//! # Allowed seam
//!
//! This package depends on the MadoPilot core, capture, input, vision, and
//! assets contract packages. It knows no concrete platform or backend adapter
//! type, so it never depends on the replay, Windows, macOS, OpenCV, or ONNX
//! packages: [`EngineWiring`] is filled in by the public facade, which is where
//! adapter selection belongs.
//!
//! # Re-exports
//!
//! The contract types below are re-exported because the facade's dependency row
//! lists no contract package. Every core, capture, vision, or asset type the
//! public Rust API exposes reaches a caller through this package, so a host
//! never has to depend on a contract package to name a value the facade handed
//! it. See `docs/architecture.md`.
//!
//! # Implementation status
//!
//! Phase 1 complete, and the Phase 2 input composition with it. Target
//! discovery and session opening, exact and latest frame selection, package
//! loading, template preparation, the deep search, the result envelope, and
//! explicit close are implemented; so are optional input and permission wiring,
//! required-versus-optional input admission, one bounded sequence at a time per
//! controller, the terminal receipt rule, and the two-sided close. There is no
//! scheduler, no watcher, no bounded work queue, no coalescing policy, and no
//! diagnostic event, and none of them is reserved here as an empty seam. The
//! default change-detection policy remains unresolved; see gate `G-005` in
//! `docs/validation-gates.md`.
//!
//! **The public names here are reviewed, not yet stable.**
//! `docs/adr/0006-public-rust-names-and-compatibility-policy.md` records the
//! review that settled them and the policy that now applies: renaming or
//! removing one is a breaking change needing an ADR and a version bump, while
//! adding is free. The stability promise itself begins at 1.0.
//!
//! # Where to start
//!
//! ```
//! use std::sync::Arc;
//!
//! use mado_pilot_runtime::{
//!     CaptureProvider, Continuity, DeliveryPlan, Engine, EngineWiring, FindRequest,
//!     IdentityIssuer, InputDelivery, InputEvent, InputOpenRequest, InputProvider, InputRequest,
//!     InputSequence, Key, MatchOptions, Matcher, OpenRequest, OperationContext, PackageLoader,
//!     PixelExtent, PixelFormat, SessionRequest,
//! };
//! use mado_pilot_testkit::{ControlledCapture, ControlledInput, ControlledMatcher, match_fixtures};
//!
//! // An engine is wired from contracts. Which adapter is behind each one is
//! // the composition root's decision and is invisible from here.
//! let issuer = Arc::new(IdentityIssuer::new());
//! let capture = Arc::new(ControlledCapture::new(
//!     Arc::clone(&issuer),
//!     PixelExtent::new(96, 64),
//!     PixelFormat::Rgba8,
//! )?);
//! let operation = OperationContext::new();
//! let targets = capture.discover(&operation)?;
//! let target = targets[0].id();
//!
//! // Input is optional and, when present, is the capture provider's own.
//! let input = Arc::new(ControlledInput::new(target));
//! let engine = Engine::new(EngineWiring {
//!     engine: issuer.engine(),
//!     capture: Arc::clone(&capture) as Arc<dyn CaptureProvider>,
//!     matcher: Matcher::new(Arc::new(ControlledMatcher::new(PixelFormat::Rgba8))),
//!     loader: PackageLoader::new(),
//!     input: Some(Arc::clone(&input) as Arc<dyn InputProvider>),
//!     permission: None,
//! })?;
//!
//! let session = engine.open_session(
//!     target,
//!     &SessionRequest::new()
//!         .capturing(OpenRequest::new())
//!         .requesting_input(InputOpenRequest::new()),
//!     &operation,
//! )?;
//! capture.publish(0x40, Continuity::Continuous)?;
//!
//! let template = engine.prepare_template(&match_fixtures::planted_template("patch"), &operation)?;
//! let outcome = session.find_template(
//!     &FindRequest::latest(&template, MatchOptions::from_defaults(template.defaults())),
//!     &operation,
//! )?;
//!
//! // The envelope names the target and keeps the exact frame that was searched.
//! assert_eq!(outcome.target(), session.target());
//! assert_eq!(outcome.result().stamp(), outcome.frame().stamp());
//!
//! // One bounded sequence, and one receipt saying exactly what it did.
//! let receipt = session.send_input(
//!     &InputRequest::new(
//!         session.target(),
//!         InputSequence::new(vec![InputEvent::KeyPress(Key::Enter)])?,
//!         DeliveryPlan::require(InputDelivery::System),
//!     ),
//!     &operation,
//! )?;
//! assert!(receipt.is_complete());
//!
//! // One close, both lifecycles.
//! session.close(&operation)?;
//! assert!(session.is_closed());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod diagnostic;
pub mod engine;
pub mod find;
pub mod session;

pub use diagnostic::{
    DiagnosticBatch, DiagnosticDrain, DiagnosticKind, DiagnosticLevel, DiagnosticLosses,
    DiagnosticOperationId, DiagnosticOperationKind, DiagnosticOptions, DiagnosticPayload,
    DiagnosticReader, DiagnosticRecord, DiagnosticRecordSequence, DiagnosticTemplateId,
    FrameDiagnostic, InputDiagnostic, InputEventDiagnostic, InputOperationSet, LifecycleDiagnostic,
    MAX_DIAGNOSTIC_CAPACITY, MappingDiagnostic, OperationStartedDiagnostic, PermissionDiagnostic,
    RouteAttemptDiagnostic, SearchDiagnostic, SearchDiagnosticOutcome,
};
pub use engine::{Engine, EngineOptions, EngineWiring, SessionRequest};
pub use find::{FindOutcome, FindRequest, SearchFrame};
pub use session::{MappingObserver, Session};

pub use mado_pilot_assets::{
    AssetFault, AssetFaultKind, AssetLimits, AssetPackage, ContentDigest, HASH_ALGORITHM,
    LoadStage, MANIFEST_PATH, Manifest, MemoryEntry, MemoryPackage, PackageLoader, PackagePath,
    PackageSource, Provenance, SCHEMA_VERSION, TemplateDeclaration,
};
pub use mado_pilot_capture::{
    CaptureFault, CaptureProvider, Continuity, CoordinateSupport, CpuMapping, Frame,
    FrameDescriptor, FrameRequest, FrameSelection, FrameView, OpenRequest, OverflowPolicy,
    PixelFormat, QueuePolicy, RetainedStoragePolicy, SessionDescription, TargetDescription,
};
pub use mado_pilot_core::{
    ActivityTag, CancellationToken, CapabilitySupport, ClipPolicy, Clock, CoordinateSpace,
    DiagnosticCategory, EngineId, Error, FrameOrder, FrameSequence, FrameStamp, GeometryFault,
    GeometryRevision, IdentityFault, IdentityIssuer, InputAddressScope, InputCapability,
    InputDelivery, InputOperationKind, InputRouteCapability, Interruption, Lifecycle,
    MonotonicInstant, OperationContext, PermissionKind, PermissionOutcome, PermissionProbe,
    PermissionReport, PermissionState, PixelExtent, PixelRect, PlatformCode, Point, ProviderId,
    Rect, RedactedDiagnostic, Result, Scale, Status, StreamEpoch, StreamId, SubmissionEvidence,
    SystemClock, TargetCapability, TargetId, TargetKind, TargetPlacement, TransformSnapshot,
};
pub use mado_pilot_input::{
    CleanupBudget, CleanupState, DeliveryPlan, FocusPolicy, GeometryPolicy, InputAttempt,
    InputController, InputDescriptor, InputEvent, InputEventObservation, InputExecution,
    InputFault, InputGeometryResult, InputOpenRequest, InputProvider, InputReceipt, InputRequest,
    InputRequirement, InputRevalidationCategory, InputSequence, Key, Modifier, PointerButton,
    PointerGeometry, PressedState, SequenceLimits, SequenceOutcome,
};
pub use mado_pilot_vision::{
    BackendDescriptor, BackendId, Match, MatchDefaults, MatchOptions, MatchResult, Matcher,
    PreparedTemplate, RegionSelection, Suppression, TemplateEncoding, TemplateId, TemplateSource,
    TemplateSourceRequest, VisionFault,
};
