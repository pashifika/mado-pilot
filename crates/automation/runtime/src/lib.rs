//! MadoPilot runtime orchestration.
//!
//! # Responsibility
//!
//! This package composes the capture, asset, and vision contracts into the
//! operations a caller actually performs. What lives here is what none of those
//! contracts can decide alone: acquiring a frame and searching it as one
//! operation, refusing a frame that belongs to another session, building the
//! authoritative envelope that says which target and which exact frame a result
//! is about, and applying the final commit check that stops late work from
//! becoming an observable success.
//!
//! It deliberately does not re-implement what a contract already owns. Frame
//! identity and ordering stay in `mado-pilot-capture`, what a match *is* stays
//! in `mado-pilot-vision`, and what makes a package valid stays in
//! `mado-pilot-assets`.
//!
//! # Allowed seam
//!
//! This package depends on the MadoPilot core, capture, vision, and assets
//! contract packages. It knows no concrete platform or backend adapter type, so
//! it never depends on the replay, Windows, macOS, OpenCV, or ONNX packages:
//! [`EngineParts`] is filled in by the public facade, which is where adapter
//! selection belongs.
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
//! Phase 1 stage 5. Target discovery and session opening, exact and latest
//! frame selection, package loading, template preparation, the deep search, the
//! result envelope, and explicit close are implemented. There is no scheduler,
//! no watcher, no bounded work queue, no coalescing policy, and no diagnostic
//! event, and none of them is reserved here as an empty seam. The default
//! change-detection policy remains unresolved; see gate `G-005` in
//! `docs/validation-gates.md`.
//!
//! **Every public name here is provisional** until gate `G-009` is resolved.
//!
//! # Where to start
//!
//! ```
//! use std::sync::Arc;
//!
//! use mado_pilot_runtime::{
//!     CaptureProvider, Continuity, Engine, EngineParts, FindRequest, IdentityIssuer, MatchOptions,
//!     Matcher, OpenRequest, OperationContext, PixelExtent, PixelFormat, PackageLoader,
//! };
//! use mado_pilot_testkit::{ControlledCapture, ControlledMatcher, match_fixtures};
//!
//! // An engine is wired from contracts. Which adapter is behind each one is
//! // the composition root's decision and is invisible from here.
//! let issuer = Arc::new(IdentityIssuer::new());
//! let capture = Arc::new(ControlledCapture::new(
//!     Arc::clone(&issuer),
//!     PixelExtent::new(96, 64),
//!     PixelFormat::Rgba8,
//! )?);
//! let engine = Engine::new(EngineParts {
//!     engine: issuer.engine(),
//!     capture: Arc::clone(&capture) as Arc<dyn CaptureProvider>,
//!     matcher: Matcher::new(Arc::new(ControlledMatcher::new(PixelFormat::Rgba8))),
//!     loader: PackageLoader::new(),
//! });
//!
//! let operation = OperationContext::new();
//! let targets = engine.discover(&operation)?;
//! let session = engine.open(targets[0].id(), &OpenRequest::new(), &operation)?;
//! capture.publish(0x40, Continuity::Continuous)?;
//!
//! let template = engine.prepare(&match_fixtures::planted_template("patch"), &operation)?;
//! let outcome = session.find_template(
//!     &FindRequest::latest(&template, MatchOptions::from_defaults(template.defaults())),
//!     &operation,
//! )?;
//!
//! // The envelope names the target and keeps the exact frame that was searched.
//! assert_eq!(outcome.target(), session.target());
//! assert_eq!(outcome.result().stamp(), outcome.frame().stamp());
//! session.close(&operation)?;
//! # Ok::<(), mado_pilot_runtime::Error>(())
//! ```

pub mod engine;
pub mod find;
pub mod session;

pub use engine::{Engine, EngineParts};
pub use find::{FindOutcome, FindRequest, FrameChoice};
pub use session::Session;

pub use mado_pilot_assets::{
    AssetFault, AssetFaultKind, AssetLimits, AssetPackage, ContentDigest, HASH_ALGORITHM,
    LoadStage, MANIFEST_PATH, Manifest, MemoryEntry, MemoryPackage, PackageLoader, PackagePath,
    PackageSource, Provenance, SCHEMA_VERSION, TemplateDeclaration,
};
pub use mado_pilot_capture::{
    CaptureFault, CaptureProvider, Continuity, CoordinateSupport, CpuMapping, Frame,
    FrameDescriptor, FrameRequest, FrameSelection, FrameView, OpenRequest, PixelFormat,
    SessionDescription, TargetDescription,
};
pub use mado_pilot_core::{
    CancellationToken, ClipPolicy, Clock, CoordinateSpace, EngineId, Error, FrameOrder,
    FrameSequence, FrameStamp, GeometryFault, GeometryRevision, IdentityFault, IdentityIssuer,
    Interruption, MonotonicInstant, OperationContext, PixelExtent, PixelRect, Point, ProviderId,
    Rect, Result, Scale, Status, StreamEpoch, StreamId, SystemClock, TargetId, TargetPlacement,
    TransformSnapshot,
};
pub use mado_pilot_vision::{
    BackendDescriptor, BackendId, Match, MatchDefaults, MatchOptions, MatchResult, Matcher,
    PreparedTemplate, RegionSelection, Suppression, TemplateEncoding, TemplateId, TemplateSource,
    TemplateSourceRequest, VisionFault,
};
