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
//! | A documented capture-resource or asset ceiling would have been exceeded, or a counter reached its end: geometry revisions, stream epochs, frame sequence numbers, or the identity space | [`Status::LimitExceeded`] |
//! | Matching backend unavailable or failed | [`Status::VisionFailed`] |
//! | An invariant this implementation is responsible for did not hold | [`Status::Internal`] |
//!
//! Package loading reports [`AssetFault`] instead, which carries the rule that
//! was broken *and* the stage that caught it. It converts into [`Error`], and
//! [`AssetFault::status`] is the category that conversion carries, so a caller
//! that wants one error type throughout keeps using `?`. That category is not
//! always [`Status::AssetInvalid`]: a package that would cross one of the
//! archive ceilings reports [`Status::LimitExceeded`]; a compression method,
//! schema version, content source, coordinate space, hash algorithm, or content
//! encoding this build does not implement, and an encrypted archive entry,
//! report [`Status::Unsupported`]; a configured limit above the implementation
//! ceiling, or a template identity the committed package does not declare,
//! reports [`Status::InvalidArgument`]; a load interrupted before it committed
//! reports [`Status::Cancelled`] or [`Status::DeadlineExceeded`]; and a size
//! computation this loader is responsible for preventing reports
//! [`Status::Internal`]. Branch on [`AssetFault::kind`] when the rule matters
//! and on the status when only the category does.
//!
//! # Implementation status
//!
//! Phase 1, complete. The deterministic replay workflow above is implemented on
//! both release targets. Native window and display capture, OCR, watchers,
//! input injection, diagnostics, and scheduling are **not implemented** and
//! cannot be reached from here.
//!
//! # Names, and what may change
//!
//! **The names here are reviewed, not yet stable.** Every one of them was
//! exercised by this package's example, its contract suite, the C ABI, and the
//! C++ wrapper before being settled;
//! `docs/adr/0006-public-rust-names-and-compatibility-policy.md` records the
//! review and the six renames it produced.
//!
//! What that policy means for a caller: adding an item, a method, or an
//! enumeration variant is free, and every public enumeration a later phase may
//! extend is already `#[non_exhaustive]`, so keep a fallback arm. Renaming or
//! removing one of these names is a breaking change and needs an ADR and a
//! version bump. The stability promise itself begins at 1.0; this package is
//! at 0.1.
//!
//! The C ABI beneath this one is versioned separately and is already frozen at
//! 1.0 — see `docs/adr/0007-phase-1-c-abi-freeze.md`. A Rust rename does not
//! propagate to it.
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
//! let captured = session.acquire_frame(&FrameRequest::latest(), &operation)?;
//! let mapping = captured.map(PixelFormat::Rgba8, &operation)?;
//! session.close(&operation)?;
//! assert!(mapping.bytes().iter().all(|byte| *byte == 0x30));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::sync::Arc;

use mado_pilot_adapter_replay::{ReplayProvider, ReplaySource};
use mado_pilot_backend_opencv::OpenCvBackend;
use mado_pilot_runtime::{EngineWiring, IdentityIssuer, Matcher, PackageLoader};

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

/// What a replay engine is built from.
///
/// The composition root has no type of its own — [`replay_engine`] is a
/// function, because for Phase 1 there is exactly one adapter pair to wire and
/// a builder would have named a decision no caller can make. What a caller can
/// decide is the policy the engine then applies, and this is where that lives:
/// every later option arrives as a method here rather than as a second
/// constructor.
///
/// A [`ReplaySource`] converts into a request that applies the default limits,
/// so `replay_engine(source)` stays the whole of the common case.
#[derive(Debug, Clone)]
pub struct ReplayEngineRequest {
    source: ReplaySource,
    limits: AssetLimits,
}

impl ReplayEngineRequest {
    /// Requests an engine over `source` with the default asset limits.
    #[must_use]
    pub fn new(source: ReplaySource) -> Self {
        Self {
            source,
            limits: AssetLimits::default(),
        }
    }

    /// Applies `limits` to every package the engine loads.
    ///
    /// [`AssetLimits`] can only be built at or below the implementation
    /// ceilings, so this can tighten what an untrusted package may allocate and
    /// cannot loosen it. [`Engine::limits`] reports what is in effect.
    #[must_use]
    pub const fn with_limits(mut self, limits: AssetLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Returns the replay source the engine captures from.
    #[must_use]
    pub const fn source(&self) -> &ReplaySource {
        &self.source
    }

    /// Returns the limits the engine will apply.
    #[must_use]
    pub const fn limits(&self) -> AssetLimits {
        self.limits
    }
}

impl From<ReplaySource> for ReplayEngineRequest {
    fn from(source: ReplaySource) -> Self {
        Self::new(source)
    }
}

/// Builds an engine over a replay source that matches through the OpenCV CPU
/// backend.
///
/// Takes a [`ReplaySource`] directly for the common case, or a
/// [`ReplayEngineRequest`] when the host has policy to state.
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
pub fn replay_engine(request: impl Into<ReplayEngineRequest>) -> Result<Engine> {
    let request = request.into();

    // Required, not preferred: constructing the backend is what proves this
    // host's OpenCV is usable, and a failure here leaves no engine that could
    // have fallen back to something else.
    let backend = OpenCvBackend::new()?;

    let issuer = Arc::new(IdentityIssuer::new());
    let engine = issuer.engine();
    let capture = ReplayProvider::new(issuer, request.source)?;

    Ok(Engine::new(EngineWiring {
        engine,
        capture: Arc::new(capture),
        matcher: Matcher::new(Arc::new(backend)),
        loader: PackageLoader::with_limits(request.limits),
    }))
}

pub use mado_pilot_runtime::{
    AssetFault, AssetFaultKind, AssetLimits, AssetPackage, BackendDescriptor, BackendId,
    CancellationToken, CaptureFault, ClipPolicy, Clock, ContentDigest, Continuity, CoordinateSpace,
    CoordinateSupport, CpuMapping, Engine, EngineId, Error, FindOutcome, FindRequest, Frame,
    FrameDescriptor, FrameOrder, FrameRequest, FrameSelection, FrameSequence, FrameStamp,
    FrameView, GeometryFault, GeometryRevision, IdentityFault, Interruption, LoadStage, Manifest,
    Match, MatchDefaults, MatchOptions, MatchResult, MemoryEntry, MemoryPackage, MonotonicInstant,
    OpenRequest, OperationContext, OverflowPolicy, PackagePath, PackageSource, PixelExtent,
    PixelFormat, PixelRect, Point, PreparedTemplate, Provenance, ProviderId, QueuePolicy, Rect,
    RegionSelection, Result, RetainedStoragePolicy, Scale, SearchFrame, Session,
    SessionDescription, Status, StreamEpoch, StreamId, Suppression, SystemClock, TargetDescription,
    TargetId, TargetPlacement, TemplateDeclaration, TemplateEncoding, TemplateId, TemplateSource,
    TemplateSourceRequest, TransformSnapshot, VisionFault,
};

/// The asset vocabulary's three module-level constants, qualified.
///
/// Their names are unqualified in `mado-pilot-assets`, where the package name
/// already says which schema, which manifest, and which hash they are about. At
/// this crate's root nothing says it: `mado_pilot::SCHEMA_VERSION` would sit
/// beside [`replay::SCHEMA_VERSION`] with no way to tell which schema either
/// one versions, and a caller importing both would have to alias one anyway.
pub use mado_pilot_runtime::{
    HASH_ALGORITHM as ASSET_HASH_ALGORITHM, MANIFEST_PATH as ASSET_MANIFEST_PATH,
    SCHEMA_VERSION as ASSET_SCHEMA_VERSION,
};
