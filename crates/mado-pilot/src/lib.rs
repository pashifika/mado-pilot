//! Public Rust facade for MadoPilot.
//!
//! MadoPilot is a headless visual automation runtime for applications and
//! agents. This package is the normal Rust dependency for a host application:
//! it performs default adapter wiring for the deterministic replay workflow and
//! for the native workflow of the release target it was built for, and it
//! re-exports the contract vocabulary both workflows speak.
//!
//! # Responsibility
//!
//! Wiring, and a curated public surface. Every operation a caller performs is
//! implemented in `mado-pilot-runtime` and the contract packages under it; this
//! package decides which concrete adapters those contracts are satisfied by,
//! and it is the only package permitted to make that decision.
//!
//! # The deterministic workflow
//!
//! Configure replay capture and require the OpenCV CPU matching backend,
//! discover targets, open one, obtain a frame, view and map it, load an asset
//! package, prepare a template, search that exact frame, read an immutable
//! correlated outcome, and close. A complete program is in
//! `examples/deterministic-slice.rs`. It needs no desktop, no permission, and no
//! network, and it behaves identically on both release targets.
//!
//! # The native workflow
//!
//! Build the engine for the target this crate was compiled for, read the
//! authorizations that platform grants, discover real windows and displays, open
//! a session that also establishes input, capture and map frames, search them,
//! submit a bounded input sequence to the target the frames came from, inspect
//! the receipt's route, threshold, and evidence, and close. Every value in that
//! flow is platform-neutral: no Windows or macOS type is re-exported here, and a
//! host that compiles for both targets writes the flow once.
//!
//! The two constructors are separate because the platforms are. One is present
//! per build, named for the target it wires: `windows_engine` on Windows and
//! `macos_engine` on macOS. Complete programs are in
//! `examples/windows-native-input.rs` and `examples/macos-native-input.rs`.
//!
//! What differs between them is what the platform actually offers, and the
//! engine reports it rather than smoothing it over. `Engine::reads_permissions`
//! is true only where an authorization can be read without prompting;
//! [`Engine::describe_input`] says what one target accepts; and
//! [`Session::input_descriptor`] says what a session actually established, which
//! is how a caller tells "input is unavailable here" from "input failed".
//!
//! # The required backend
//!
//! Every constructor here requires the OpenCV CPU backend and never substitutes
//! another implementation for it. There is no backend-selection argument because
//! there is exactly one production matching backend: requiring it is the only
//! choice available, so a selection type would name a decision no caller can
//! make. A second backend arrives with its own constructor rather than by
//! changing these.
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
//! | Malformed request, unknown target, foreign identity, mismatched adapter pairing, template identity a loaded package does not declare | [`Status::InvalidArgument`] |
//! | Capability or conversion the request needs is unavailable | [`Status::Unsupported`] |
//! | Cancellation token set before the result committed | [`Status::Cancelled`] |
//! | Deadline passed before the result committed | [`Status::DeadlineExceeded`] |
//! | Session closed | [`Status::Closed`] |
//! | The target a session or a sequence names no longer exists | [`Status::TargetLost`] |
//! | Capture failed | [`Status::CaptureFailed`] |
//! | Asset package refused | [`Status::AssetInvalid`] |
//! | A documented capture-resource or asset ceiling would have been exceeded, or a counter reached its end: geometry revisions, stream epochs, frame sequence numbers, or the identity space | [`Status::LimitExceeded`] |
//! | Matching backend unavailable or failed | [`Status::VisionFailed`] |
//! | Input delivery was refused by the platform, its policy, or its authorization | [`Status::InputFailed`] |
//! | An invariant this implementation is responsible for did not hold | [`Status::Internal`] |
//!
//! Input adds one shape the table cannot express, because it is not a failure.
//! An admitted sequence answers with an [`InputReceipt`] rather than a status:
//! an operating system cannot recall an event that may already have native
//! effect, so a sequence that stopped part-way reports how far its route got,
//! which mechanism carried it, what evidence that route established, and what
//! it managed to release. [`SequenceOutcome`] is what a caller branches on; only
//! a sequence that was never admitted, or that reached no route threshold and
//! could have no native effect when its operation lost the race, reports a status.
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
//! Phase 1 capture/matching, Phase 2 native input, and Phase 3 singular/grouped
//! OCR are reachable here. Replay and native engine requests accept an injected
//! `Arc<dyn OcrBackend>`; ordinary constructors without one expose no OCR.
//! Separate `*_engine_with_default_ocr` constructors preserve native G-004;
//! `*_engine_with_ocr_profile` accepts the closed
//! [`OcrProfile::BoundedDetector`] selection through [`OcrProfileConfig`].
//! [`Session::recognize`] preserves singular optional-region results, while
//! [`Session::scan_ocr_zones`] borrows one through eight capture-pixel zones and
//! returns one independent caller-grouped result. See
//! `examples/ocr-default.rs` and `examples/ocr-profile-zones.rs`.
//!
//! Watchers and scheduling are not implemented. No platform-native type is
//! returned, accepted, or downcast through this API.
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
//! at 0.3.1.
//!
//! The C ABI beneath this one is versioned separately. ABI 1.0, 1.2, and 1.3
//! are frozen complete prefixes; ABI 1.4 appends explicit profile construction,
//! grouped scan ownership, and two-dimensional access without moving them.
//! ADRs 0035, 0036, and 0043 record those ownership and negotiation boundaries.
//! A Rust rename does not propagate to the C ABI.
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
//! let mapping = session.map_frame(&captured, PixelFormat::Rgba8, &operation)?;
//! session.close(&operation)?;
//! assert!(mapping.bytes().iter().all(|byte| *byte == 0x30));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mado_pilot_adapter_replay::{ReplayProvider, ReplaySource};
use mado_pilot_backend_onnx::OnnxOcrBackend;
use mado_pilot_backend_opencv::OpenCvBackend;
use mado_pilot_runtime::{
    CaptureProvider, EngineOptions as RuntimeEngineOptions, EngineWiring, IdentityIssuer, Matcher,
    OcrRecognizer, PackageLoader,
};

#[cfg(any(windows, target_os = "macos"))]
use mado_pilot_runtime::InputProvider;
#[cfg(target_os = "macos")]
use mado_pilot_runtime::PermissionProbe;

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

/// The only backend selected by the default OCR composition.
pub const DEFAULT_OCR_BACKEND_ID: &str = mado_pilot_backend_onnx::BACKEND_ID;
/// Exact default backend implementation and native compatibility identity.
pub const DEFAULT_OCR_BACKEND_VERSION: &str = mado_pilot_backend_onnx::BACKEND_VERSION;
/// Closed runtime/provider profile required by the default OCR composition.
pub const DEFAULT_OCR_RUNTIME_PROFILE_ID: &str = mado_pilot_backend_onnx::RUNTIME_PROFILE_ID;

/// Explicit controlled prerequisites for the accepted default OCR profile.
///
/// Neither path is discovered or read until a `*_engine_with_default_ocr`
/// constructor receives this value and the caller's [`OperationContext`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultOcrConfig {
    model_root: PathBuf,
    runtime_path: PathBuf,
}

impl DefaultOcrConfig {
    /// Names the root containing the fixed G-004 relative model paths and the
    /// canonical absolute ONNX Runtime 1.29.0 file.
    pub fn new(model_root: impl Into<PathBuf>, runtime_path: impl Into<PathBuf>) -> Self {
        Self {
            model_root: model_root.into(),
            runtime_path: runtime_path.into(),
        }
    }

    /// Returns the caller-selected model root.
    #[must_use]
    pub fn model_root(&self) -> &Path {
        &self.model_root
    }

    /// Returns the caller-selected controlled runtime file.
    #[must_use]
    pub fn runtime_path(&self) -> &Path {
        &self.runtime_path
    }
}

/// Closed explicit selection for a non-default product OCR profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OcrProfile {
    /// ADR 0040/0041 bounded-detector preprocessing over the accepted model pair.
    BoundedDetector,
}

/// Controlled prerequisites for one explicitly selected product OCR profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrProfileConfig {
    profile: OcrProfile,
    model_root: PathBuf,
    runtime_path: PathBuf,
}

impl OcrProfileConfig {
    /// Selects `profile` and names its controlled model root and runtime file.
    pub fn new(
        profile: OcrProfile,
        model_root: impl Into<PathBuf>,
        runtime_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            profile,
            model_root: model_root.into(),
            runtime_path: runtime_path.into(),
        }
    }

    /// Returns the selected closed product profile.
    #[must_use]
    pub const fn profile(&self) -> OcrProfile {
        self.profile
    }

    /// Returns the caller-selected model root.
    #[must_use]
    pub fn model_root(&self) -> &Path {
        &self.model_root
    }

    /// Returns the caller-selected controlled runtime file.
    #[must_use]
    pub fn runtime_path(&self) -> &Path {
        &self.runtime_path
    }
}

#[derive(Clone, Copy)]
enum IntegratedOcr<'a> {
    Default(&'a DefaultOcrConfig, &'a OperationContext),
    Profile(&'a OcrProfileConfig, &'a OperationContext),
}

impl<'a> IntegratedOcr<'a> {
    const fn operation(self) -> &'a OperationContext {
        match self {
            Self::Default(_, operation) | Self::Profile(_, operation) => operation,
        }
    }
}

fn configured_ocr(
    explicit: Option<Arc<dyn OcrBackend>>,
    integrated: Option<IntegratedOcr<'_>>,
) -> Result<Option<OcrRecognizer>> {
    match (explicit, integrated) {
        (Some(_), Some(_)) => Err(Error::new(
            Status::InvalidArgument,
            "an explicit OCR backend and an integrated OCR profile are mutually exclusive",
        )),
        (Some(backend), None) => Ok(Some(OcrRecognizer::new(backend))),
        (None, Some(IntegratedOcr::Default(config, operation))) => {
            let backend = OnnxOcrBackend::open_accepted(
                config.model_root(),
                config.runtime_path(),
                operation,
            )
            .map_err(Error::from)?;
            Ok(Some(OcrRecognizer::new(Arc::new(backend))))
        }
        (None, Some(IntegratedOcr::Profile(config, operation))) => {
            let backend = match config.profile() {
                OcrProfile::BoundedDetector => OnnxOcrBackend::open_bounded_detector(
                    config.model_root(),
                    config.runtime_path(),
                    operation,
                ),
            }
            .map_err(Error::from)?;
            Ok(Some(OcrRecognizer::new(Arc::new(backend))))
        }
        (None, None) => Ok(None),
    }
}

fn construction_checkpoint(operation: Option<&OperationContext>) -> Result<()> {
    match operation.and_then(OperationContext::interruption) {
        Some(interruption) => Err(interruption.into()),
        None => Ok(()),
    }
}

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
    diagnostics: DiagnosticOptions,
    ocr: Option<Arc<dyn OcrBackend>>,
}

impl ReplayEngineRequest {
    /// Requests an engine over `source` with the default asset limits.
    #[must_use]
    pub fn new(source: ReplaySource) -> Self {
        Self {
            source,
            limits: AssetLimits::default(),
            diagnostics: DiagnosticOptions::off(),
            ocr: None,
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

    /// Enables the engine-scoped bounded diagnostic stream.
    #[must_use]
    pub const fn with_diagnostics(mut self, diagnostics: DiagnosticOptions) -> Self {
        self.diagnostics = diagnostics;
        self
    }

    /// Configures the exact OCR backend used by one-shot recognition.
    ///
    /// There is no default OCR backend. The caller retains backend/model
    /// selection authority and requests must name this backend's descriptor.
    #[must_use]
    pub fn with_ocr_backend(mut self, backend: Arc<dyn OcrBackend>) -> Self {
        self.ocr = Some(backend);
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

    /// Returns the selected diagnostic configuration.
    #[must_use]
    pub const fn diagnostics(&self) -> DiagnosticOptions {
        self.diagnostics
    }

    /// Returns the configured OCR backend, if any.
    #[must_use]
    pub fn ocr_backend(&self) -> Option<&dyn OcrBackend> {
        self.ocr.as_deref()
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
    replay_engine_inner(request.into(), None)
}

/// Builds a replay engine with the accepted G-004 model and CPU ONNX backend.
///
/// `config` supplies both controlled paths explicitly. Model loading, runtime
/// initialization, and session construction use `operation`; a late,
/// cancelled, or failed construction publishes no engine and selects no
/// alternate model or provider.
///
/// # Errors
///
/// Returns [`Status::Unsupported`] when a controlled prerequisite is
/// unavailable, [`Status::AssetInvalid`] when model bytes do not match G-004,
/// [`Status::InvalidArgument`] for malformed controlled paths, and the existing
/// matching/capture construction failures.
pub fn replay_engine_with_default_ocr(
    request: impl Into<ReplayEngineRequest>,
    config: &DefaultOcrConfig,
    operation: &OperationContext,
) -> Result<Engine> {
    replay_engine_inner(
        request.into(),
        Some(IntegratedOcr::Default(config, operation)),
    )
}

/// Builds a replay engine with one explicitly selected product OCR profile.
///
/// Construction consumes only the controlled paths in `config` under
/// `operation`. It publishes no engine on interruption or profile failure and
/// never substitutes the released default profile.
///
/// # Errors
///
/// Returns the selected profile's typed prerequisite, identity, interruption,
/// or native initialization failure and the existing matching/capture failures.
pub fn replay_engine_with_ocr_profile(
    request: impl Into<ReplayEngineRequest>,
    config: &OcrProfileConfig,
    operation: &OperationContext,
) -> Result<Engine> {
    replay_engine_inner(
        request.into(),
        Some(IntegratedOcr::Profile(config, operation)),
    )
}

fn replay_engine_inner(
    request: ReplayEngineRequest,
    integrated_ocr: Option<IntegratedOcr<'_>>,
) -> Result<Engine> {
    let operation = integrated_ocr.map(IntegratedOcr::operation);
    construction_checkpoint(operation)?;
    // Required, not preferred: constructing the backend is what proves this
    // host's OpenCV is usable, and a failure here leaves no engine that could
    // have fallen back to something else.
    let backend = OpenCvBackend::new()?;
    let ocr = configured_ocr(request.ocr, integrated_ocr)?;

    let issuer = Arc::new(IdentityIssuer::new());
    let engine = issuer.engine();
    let capture = ReplayProvider::new(issuer, request.source)?;

    let engine = Engine::new_with_options(
        EngineWiring {
            engine,
            capture: Arc::new(capture),
            matcher: Matcher::new(Arc::new(backend)),
            loader: PackageLoader::with_limits(request.limits),
            ocr,
            // Replay is a source of prepared frames, so there is no target for input
            // to reach and no authorization behind one. A capture-only engine says
            // exactly that.
            input: None,
            permission: None,
        },
        RuntimeEngineOptions::new().with_diagnostics(request.diagnostics),
    )?;
    construction_checkpoint(operation)?;
    Ok(engine)
}

/// What a native engine is built from.
///
/// Which platform is not a field: an engine is built by the constructor for one
/// release target, so a request cannot name a platform the build does not
/// contain. What a caller can decide is the policy the engine then applies, and
/// this is where that lives — every later option arrives as a method here rather
/// than as another constructor per target.
#[derive(Debug, Clone, Default)]
pub struct NativeEngineRequest {
    limits: AssetLimits,
    diagnostics: DiagnosticOptions,
    ocr: Option<Arc<dyn OcrBackend>>,
}

impl NativeEngineRequest {
    /// Requests a native engine with the default asset limits.
    #[must_use]
    pub fn new() -> Self {
        Self {
            limits: AssetLimits::default(),
            diagnostics: DiagnosticOptions::off(),
            ocr: None,
        }
    }

    /// Applies `limits` to every package the engine loads.
    ///
    /// As [`ReplayEngineRequest::with_limits`]: this can tighten what an
    /// untrusted package may allocate and cannot loosen it.
    #[must_use]
    pub const fn with_limits(mut self, limits: AssetLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Enables the engine-scoped bounded diagnostic stream.
    #[must_use]
    pub const fn with_diagnostics(mut self, diagnostics: DiagnosticOptions) -> Self {
        self.diagnostics = diagnostics;
        self
    }

    /// Configures the exact OCR backend used by one-shot recognition.
    ///
    /// No native constructor selects a default OCR backend or runtime.
    #[must_use]
    pub fn with_ocr_backend(mut self, backend: Arc<dyn OcrBackend>) -> Self {
        self.ocr = Some(backend);
        self
    }

    /// Returns the limits the engine will apply.
    #[must_use]
    pub const fn limits(&self) -> AssetLimits {
        self.limits
    }

    /// Returns the selected diagnostic configuration.
    #[must_use]
    pub const fn diagnostics(&self) -> DiagnosticOptions {
        self.diagnostics
    }

    /// Returns the configured OCR backend, if any.
    #[must_use]
    pub fn ocr_backend(&self) -> Option<&dyn OcrBackend> {
        self.ocr.as_deref()
    }
}

impl From<AssetLimits> for NativeEngineRequest {
    fn from(limits: AssetLimits) -> Self {
        Self::new().with_limits(limits)
    }
}

/// Builds an engine over native Windows discovery, capture, and input.
///
/// Present on Windows builds only. Discovery is picker-free, capture is Windows
/// Graphics Capture, and input exposes the platform's system route plus the
/// fixture-gated `WindowMessage` route; none of those native types reaches this
/// API, which speaks the same platform-neutral vocabulary the replay engine does.
///
/// Construction touches no Windows API and asks for no authorization: it
/// selects adapters, and every native call happens in the operation that needs
/// it. Windows grants no separate capture or input authorization this adapter
/// can read, so the engine reads none — `Engine::reads_permissions` reports
/// `false` there, which is not a claim that an operation will be permitted.
///
/// # Atomic construction
///
/// The one step that can fail on its own is the matching backend, and it runs
/// first, so a build whose OpenCV is unusable fails before an identity space or
/// an adapter exists. Nothing constructed after it holds a native resource — the
/// provider acquires those per operation — so a later refusal leaves nothing
/// open, and a failed construction yields no engine at all rather than a
/// half-configured one.
///
/// # Errors
///
/// Returns [`Status::VisionFailed`] when the required matching backend cannot be
/// initialized.
#[cfg(windows)]
pub fn windows_engine(request: impl Into<NativeEngineRequest>) -> Result<Engine> {
    windows_engine_inner(request.into(), None)
}

/// Builds the native Windows engine with the accepted default CPU OCR profile.
#[cfg(windows)]
pub fn windows_engine_with_default_ocr(
    request: impl Into<NativeEngineRequest>,
    config: &DefaultOcrConfig,
    operation: &OperationContext,
) -> Result<Engine> {
    windows_engine_inner(
        request.into(),
        Some(IntegratedOcr::Default(config, operation)),
    )
}

/// Builds the native Windows engine with an explicitly selected OCR profile.
#[cfg(windows)]
pub fn windows_engine_with_ocr_profile(
    request: impl Into<NativeEngineRequest>,
    config: &OcrProfileConfig,
    operation: &OperationContext,
) -> Result<Engine> {
    windows_engine_inner(
        request.into(),
        Some(IntegratedOcr::Profile(config, operation)),
    )
}

#[cfg(windows)]
fn windows_engine_inner(
    request: NativeEngineRequest,
    integrated_ocr: Option<IntegratedOcr<'_>>,
) -> Result<Engine> {
    let operation = integrated_ocr.map(IntegratedOcr::operation);
    construction_checkpoint(operation)?;
    let diagnostics = request.diagnostics();
    let limits = request.limits();

    // Required, not preferred, and first: constructing the backend is what
    // proves this host's OpenCV is usable, and a failure here leaves no adapter
    // and no identity space behind.
    let backend = OpenCvBackend::new()?;
    let ocr = configured_ocr(request.ocr, integrated_ocr)?;

    let issuer = Arc::new(IdentityIssuer::new());
    let engine = issuer.engine();
    let provider = Arc::new(mado_pilot_platform_windows::WindowsCaptureProvider::new(
        issuer,
    ));

    let engine = Engine::new_with_options(
        EngineWiring {
            engine,
            capture: Arc::clone(&provider) as Arc<dyn CaptureProvider>,
            matcher: Matcher::new(Arc::new(backend)),
            loader: PackageLoader::with_limits(limits),
            ocr,
            input: Some(provider as Arc<dyn InputProvider>),
            permission: None,
        },
        RuntimeEngineOptions::new().with_diagnostics(diagnostics),
    )?;
    construction_checkpoint(operation)?;
    Ok(engine)
}

/// Builds an engine over native macOS discovery, capture, permissions, and
/// input.
///
/// Present on macOS builds only. Discovery is picker-free, capture is
/// ScreenCaptureKit, and input keeps focus-dependent `System` separate from the
/// candidate, caller-selected `ProcessDirected` route on gated windows. The latter
/// remains owning-process scoped, unknown in application compatibility, and
/// invocation-only in evidence; the per-target descriptor reports whether it
/// is currently attemptable. Neither native route nor either authorization
/// type reaches this API, which speaks the same platform-neutral vocabulary on
/// both release targets.
///
/// Construction touches no macOS API, requests no authorization, and presents
/// nothing: it selects adapters. Every native call, including the two
/// non-prompting authorization reads behind `Engine::permissions`, happens in
/// the operation that needs it.
///
/// # Atomic construction
///
/// As the Windows constructor: the matching backend runs first and is the one
/// step that can fail on its own, nothing constructed after it holds a native
/// resource, and a failed construction yields no engine rather than a
/// half-configured one.
///
/// # Errors
///
/// Returns [`Status::VisionFailed`] when the required matching backend cannot be
/// initialized.
#[cfg(target_os = "macos")]
pub fn macos_engine(request: impl Into<NativeEngineRequest>) -> Result<Engine> {
    macos_engine_inner(request.into(), None)
}

/// Builds the native macOS engine with the accepted default CPU OCR profile.
#[cfg(target_os = "macos")]
pub fn macos_engine_with_default_ocr(
    request: impl Into<NativeEngineRequest>,
    config: &DefaultOcrConfig,
    operation: &OperationContext,
) -> Result<Engine> {
    macos_engine_inner(
        request.into(),
        Some(IntegratedOcr::Default(config, operation)),
    )
}

/// Builds the native macOS engine with an explicitly selected OCR profile.
#[cfg(target_os = "macos")]
pub fn macos_engine_with_ocr_profile(
    request: impl Into<NativeEngineRequest>,
    config: &OcrProfileConfig,
    operation: &OperationContext,
) -> Result<Engine> {
    macos_engine_inner(
        request.into(),
        Some(IntegratedOcr::Profile(config, operation)),
    )
}

#[cfg(target_os = "macos")]
fn macos_engine_inner(
    request: NativeEngineRequest,
    integrated_ocr: Option<IntegratedOcr<'_>>,
) -> Result<Engine> {
    let operation = integrated_ocr.map(IntegratedOcr::operation);
    construction_checkpoint(operation)?;
    let diagnostics = request.diagnostics();
    let limits = request.limits();

    let backend = OpenCvBackend::new()?;
    let ocr = configured_ocr(request.ocr, integrated_ocr)?;

    let issuer = Arc::new(IdentityIssuer::new());
    let engine = issuer.engine();
    let provider = Arc::new(mado_pilot_platform_macos::MacosCaptureProvider::new(issuer));

    let engine = Engine::new_with_options(
        EngineWiring {
            engine,
            capture: Arc::clone(&provider) as Arc<dyn CaptureProvider>,
            matcher: Matcher::new(Arc::new(backend)),
            loader: PackageLoader::with_limits(limits),
            ocr,
            input: Some(provider as Arc<dyn InputProvider>),
            permission: Some(
                Arc::new(mado_pilot_platform_macos::MacosPermissionProbe::new())
                    as Arc<dyn PermissionProbe>,
            ),
        },
        RuntimeEngineOptions::new().with_diagnostics(diagnostics),
    )?;
    construction_checkpoint(operation)?;
    Ok(engine)
}

pub use mado_pilot_runtime::{
    ACCEPTED_BOUNDED_MODEL_ID, ACCEPTED_BOUNDED_MODEL_VERSION, ACCEPTED_BOUNDED_PREPROCESSING_ID,
    ACCEPTED_BOUNDED_PROFILE_ID, ACCEPTED_G004_DECODER_ID, ACCEPTED_G004_LANGUAGE_PROFILE_ID,
    ACCEPTED_G004_MODEL_ID, ACCEPTED_G004_MODEL_VERSION, ACCEPTED_G004_NORMALIZATION_ID,
    ACCEPTED_G004_PREPROCESSING_ID, ACCEPTED_G004_PROFILE_ID, ACCEPTED_G004_VOCABULARY_ENTRIES,
    ActivityTag, AssetFault, AssetFaultKind, AssetLimits, AssetPackage, BackendCandidate,
    BackendDescriptor, BackendId, CancellationToken, CapabilitySupport, CaptureFault,
    CleanupBudget, CleanupState, ClipPolicy, Clock, Confidence, ContentDigest, Continuity,
    CoordinateSpace, CoordinateSupport, CpuMapping, DecoderId, DeliveryPlan, DiagnosticBatch,
    DiagnosticCategory, DiagnosticDrain, DiagnosticKind, DiagnosticLevel, DiagnosticLosses,
    DiagnosticOcrModelInstanceId, DiagnosticOperationId, DiagnosticOperationKind,
    DiagnosticOptions, DiagnosticPayload, DiagnosticReader, DiagnosticRecord,
    DiagnosticRecordSequence, DiagnosticTemplateId, Engine, EngineId, EngineOptions, Error,
    FindOutcome, FindRequest, FocusPolicy, Frame, FrameDescriptor, FrameDiagnostic, FrameOrder,
    FrameRequest, FrameSelection, FrameSequence, FrameStamp, FrameView, GeometryFault,
    GeometryPolicy, GeometryRevision, IdentityFault, InputAddressScope, InputAttempt,
    InputCapability, InputDelivery, InputDescriptor, InputDiagnostic, InputEvent, InputFault,
    InputOpenRequest, InputOperationKind, InputOperationSet, InputReceipt, InputRequest,
    InputRequirement, InputRouteCapability, InputSequence, Interruption, Key, LanguageProfileId,
    Lifecycle, LifecycleDiagnostic, LoadStage, MAX_BACKEND_TEXT_BYTES, MAX_DIAGNOSTIC_CAPACITY,
    MAX_MODEL_COMPONENT_BYTES, MAX_OCR_CANDIDATES, MAX_OCR_ZONES, MAX_TEXT_BYTES, Manifest,
    MappingDiagnostic, MappingObserver, Match, MatchDefaults, MatchOptions, MatchResult,
    MemoryEntry, MemoryPackage, ModelComponentIdentity, ModelId, ModelVersion, Modifier,
    MonotonicInstant, NormalizationId, OcrBackend, OcrBackendDescriptor, OcrBackendId,
    OcrBackendIdentity, OcrBackendRequest, OcrBackendVersion, OcrCandidateSink, OcrDiagnostic,
    OcrDiagnosticOutcome, OcrDiagnosticProfile, OcrFault, OcrModelComponent, OcrModelIdentity,
    OcrModelSource, OcrModelSourceRequest, OcrProfileMetadata, OcrQuadrilateral, OcrRegion,
    OcrRequest, OcrRequestedRegionDiagnostic, OcrResult, OcrZone, OcrZoneGroup, OcrZoneScanRequest,
    OcrZoneScanResult, OpenRequest, OperationContext, OperationStartedDiagnostic, OverflowPolicy,
    PackagePath, PackageSource, PermissionDiagnostic, PermissionKind, PermissionOutcome,
    PermissionReport, PermissionState, PixelExtent, PixelFormat, PixelRect, PlatformCode, Point,
    PointerButton, PointerGeometry, PreparedTemplate, PreprocessingId, PressedState, ProfileId,
    Provenance, ProviderId, QueuePolicy, RecognizedRegion, Rect, RedactedDiagnostic,
    RegionSelection, Result, RetainedStoragePolicy, RouteAttemptDiagnostic, Scale,
    SearchDiagnostic, SearchDiagnosticOutcome, SearchFrame, SequenceLimits, SequenceOutcome,
    Session, SessionDescription, SessionRequest, Status, StreamEpoch, StreamId, SubmissionEvidence,
    Suppression, SystemClock, TargetCapability, TargetDescription, TargetId, TargetKind,
    TargetPlacement, TemplateDeclaration, TemplateEncoding, TemplateId, TemplateSource,
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

#[cfg(test)]
mod tests {
    use super::{CancellationToken, OperationContext, Status, construction_checkpoint};

    #[test]
    fn default_construction_checkpoint_refuses_a_late_cancellation() {
        let cancellation = CancellationToken::new();
        let operation = OperationContext::new().with_cancellation(cancellation.clone());
        cancellation.cancel();

        assert_eq!(
            construction_checkpoint(Some(&operation))
                .expect_err("a cancelled construction cannot publish")
                .status(),
            Status::Cancelled
        );
    }
}
