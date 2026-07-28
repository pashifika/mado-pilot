//! Public Rust facade for MadoPilot.
//!
//! MadoPilot is a headless visual automation runtime for applications and
//! agents. This package is the normal Rust dependency for a host application:
//! it performs default adapter wiring for the deterministic Phase 1 workflow
//! and re-exports the contract vocabulary that workflow speaks.
//!
//! # Responsibility
//!
//! Wiring, and a curated public surface. Every operation a caller performs is
//! implemented in `mado-pilot-runtime` and the contract packages under it; this
//! package decides which concrete adapters those contracts are satisfied by,
//! and it is the only package permitted to make that decision.
//!
//! # The Phase 1 workflow
//!
//! Configure replay capture and require the OpenCV CPU matching backend,
//! discover targets, open one, obtain a frame, view and map it, load an asset
//! package, prepare a template, search that exact frame, read an immutable
//! correlated outcome, and close. A complete program is in
//! `examples/deterministic-slice.rs`.
//!
//! # The required backend
//!
//! [`replay_engine`] requires the OpenCV CPU backend and never substitutes
//! another implementation for it. There is no backend-selection argument
//! because Phase 1 has exactly one production matching backend: requiring it is
//! the only choice available, so a selection type would name a decision no
//! caller can make. A second backend arrives with its own constructor rather
//! than by changing this one.
//!
//! The backend is initialized before anything else is wired, so an unusable
//! OpenCV fails engine construction with [`Status::VisionFailed`] and leaves no
//! half-configured engine behind. [`REQUIRED_BACKEND`] is the identifier that
//! backend reports, available before an engine exists so a host can state its
//! own required-backend policy; [`Engine::backend`] reports what was actually
//! selected, and every match result carries the same descriptor.
//!
//! An OpenCV that cannot be *loaded* at all is not reachable as a status: the
//! library links dynamically at load time, so an absent one stops the process
//! before any MadoPilot code runs. That gap is recorded against gate `G-007`
//! in `docs/validation-gates.md` rather than papered over here.
//!
//! # Failures
//!
//! Every failed operation reports a machine-readable category. [`Status`] is
//! that category and is what a caller branches on; [`Error::detail`] is
//! diagnostic text and is never required for control flow.
//!
//! | Outcome | Status |
//! |---|---|
//! | Malformed request, unknown target, foreign identity, template identity a loaded package does not declare | [`Status::InvalidArgument`] |
//! | Capability or conversion the request needs is unavailable | [`Status::Unsupported`] |
//! | Cancellation token set before the result committed | [`Status::Cancelled`] |
//! | Deadline passed before the result committed | [`Status::DeadlineExceeded`] |
//! | Session closed | [`Status::Closed`] |
//! | Capture failed | [`Status::CaptureFailed`] |
//! | Asset package refused | [`Status::AssetInvalid`] |
//! | Matching backend unavailable or failed | [`Status::VisionFailed`] |
//!
//! Package loading reports [`AssetFault`] instead, which carries the rule that
//! was broken *and* the stage that caught it. It converts into [`Error`] with
//! the status above, so a caller that wants one error type throughout keeps
//! using `?`.
//!
//! # Implementation status
//!
//! Phase 1 stage 5. The deterministic replay workflow above is implemented on
//! both release targets. Native window and display capture, OCR, watchers,
//! input injection, diagnostics, and scheduling are **not implemented** and
//! cannot be reached from here.
//!
//! **Every public name here is provisional.** Naming is settled by gate
//! `G-009` before Phase 1 exits, after this package's example has exercised
//! the names; see `docs/validation-gates.md`. Nothing here is a stability
//! promise yet.
//!
//! # Where to start
//!
//! ```
//! use mado_pilot::replay::{ReplayFrame, ReplaySource, ReplayTarget};
//! use mado_pilot::{
//!     Continuity, FrameDescriptor, FrameRequest, MonotonicInstant, OpenRequest, OperationContext,
//!     PixelExtent, PixelFormat, REQUIRED_BACKEND,
//! };
//!
//! // A replay source is raw pixels a caller supplies, so a workflow is
//! // reproducible without a desktop, a permission, or a network.
//! let descriptor = FrameDescriptor::packed(PixelExtent::new(8, 8), PixelFormat::Rgba8)?;
//! let frame = ReplayFrame::new(
//!     descriptor,
//!     MonotonicInstant::ORIGIN,
//!     Continuity::Continuous,
//!     None,
//!     vec![0x30; descriptor.byte_len()].into_boxed_slice(),
//! )?;
//! let source = ReplaySource::from_targets(vec![ReplayTarget::new("panel", vec![frame])?])?;
//!
//! // Construction requires the OpenCV CPU backend and reports what it selected.
//! let engine = mado_pilot::replay_engine(source)?;
//! assert_eq!(engine.backend().id(), REQUIRED_BACKEND);
//!
//! let operation = OperationContext::new();
//! let targets = engine.discover(&operation)?;
//! let session = engine.open(targets[0].id(), &OpenRequest::new(), &operation)?;
//!
//! // A mapping outlives the session it came from.
//! let captured = session.frame(&FrameRequest::latest(), &operation)?;
//! let mapping = captured.map(PixelFormat::Rgba8, &operation)?;
//! session.close(&operation)?;
//! assert!(mapping.bytes().iter().all(|byte| *byte == 0x30));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::sync::Arc;

use mado_pilot_adapter_replay::{ReplayProvider, ReplaySource};
use mado_pilot_backend_opencv::OpenCvBackend;
use mado_pilot_runtime::{EngineParts, IdentityIssuer, Matcher, PackageLoader};

/// Replay capture configuration.
///
/// A replay source is the Phase 1 capture input: raw frames a caller supplies
/// from memory or reads from a directory, published under the same identity
/// rules a native adapter will use later. They are grouped in a module because
/// a replay manifest and an asset manifest each declare a schema version, and
/// two constants of that name at one crate root would be a coin flip.
pub mod replay {
    pub use mado_pilot_adapter_replay::{
        MANIFEST_NAME, ReplayFault, ReplayFrame, ReplaySource, ReplayTarget, SCHEMA_VERSION,
    };
}

/// The identifier of the matching backend this build requires.
///
/// Available before an engine exists, so a host can state a required-backend
/// policy without first having to construct one. [`Engine::backend`] reports
/// the descriptor of the backend that was actually selected.
pub const REQUIRED_BACKEND: &str = mado_pilot_backend_opencv::BACKEND_ID;

/// Builds an engine over `source` that matches through the OpenCV CPU backend.
///
/// The backend is initialized first. A build that cannot use its OpenCV fails
/// here rather than at the first search, and no other matching implementation
/// is substituted for it.
///
/// # Errors
///
/// Returns [`Status::VisionFailed`] when the required matching backend cannot
/// be initialized, and a capture failure when the replay source cannot be
/// accepted.
pub fn replay_engine(source: ReplaySource) -> Result<Engine> {
    // Required, not preferred: constructing the backend is what proves this
    // host's OpenCV is usable, and a failure here leaves no engine that could
    // have fallen back to something else.
    let backend = OpenCvBackend::new()?;

    let issuer = Arc::new(IdentityIssuer::new());
    let engine = issuer.engine();
    let capture = ReplayProvider::new(issuer, source)?;

    Ok(Engine::new(EngineParts {
        engine,
        capture: Arc::new(capture),
        matcher: Matcher::new(Arc::new(backend)),
        loader: PackageLoader::new(),
    }))
}

pub use mado_pilot_runtime::{
    AssetFault, AssetFaultKind, AssetPackage, CancellationToken, CaptureFault, ClipPolicy, Clock,
    ContentDigest, Continuity, CoordinateSpace, CoordinateSupport, CpuMapping, Engine, EngineId,
    Error, FindOutcome, FindRequest, Frame, FrameChoice, FrameDescriptor, FrameOrder, FrameRequest,
    FrameSelection, FrameSequence, FrameStamp, FrameView, GeometryFault, GeometryRevision,
    HASH_ALGORITHM, IdentityFault, Interruption, LoadStage, MANIFEST_PATH, Manifest, Match,
    MatchDefaults, MatchOptions, MatchResult, MemoryEntry, MemoryPackage, MonotonicInstant,
    OpenRequest, OperationContext, PackagePath, PackageSource, PixelExtent, PixelFormat, PixelRect,
    Point, PreparedTemplate, Provenance, ProviderId, Rect, RegionSelection, Result, SCHEMA_VERSION,
    Scale, Session, SessionDescription, Status, StreamEpoch, StreamId, Suppression, SystemClock,
    TargetDescription, TargetId, TargetPlacement, TemplateDeclaration, TemplateEncoding,
    TemplateId, TemplateSource, TemplateSourceRequest, TransformSnapshot, VisionFault,
};
