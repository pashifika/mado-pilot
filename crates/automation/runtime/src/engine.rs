//! The composition of the capture, asset, and vision contracts.
//!
//! An engine is the one place that holds all three at once, and every operation
//! here exists because it spans more than one of them. Nothing in this file
//! names a concrete adapter: which capture provider and which matching backend
//! an engine was built from is the facade's decision, and an engine cannot
//! observe or change it.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use mado_pilot_assets::{AssetFault, AssetLimits, AssetPackage, PackageLoader, PackageSource};
use mado_pilot_capture::{CaptureProvider, CaptureSession, OpenRequest, TargetDescription};
use mado_pilot_core::{
    EngineId, Error, InputCapability, Operation, OperationContext, PermissionKind,
    PermissionOutcome, PermissionProbe, PermissionReport, Status, TargetId,
};
use mado_pilot_input::{
    InputController, InputDescriptor, InputFault, InputOpenRequest, InputProvider,
    check_provider_pair,
};
use mado_pilot_ocr::{OcrBackendDescriptor, OcrProviderDescriptor, OcrRecognizer};
use mado_pilot_vision::{BackendDescriptor, Matcher, PreparedTemplate, TemplateSource};

use crate::diagnostic::{
    DiagnosticOperationKind, DiagnosticOptions, DiagnosticPayload, DiagnosticReader,
    DiagnosticSink, LifecycleDiagnostic, ObservedOperation, OcrDiagnosticContext,
    PermissionDiagnostic,
};
use crate::session::Session;
use crate::watch::{TemplateSchedulerDescriptor, WatchRuntime};

/// How long the release of an already-opened capture session or input
/// controller may take when a later step of the same open refuses it.
///
/// The caller's operation is usually already over by then, so this bound exists
/// only so that an adapter which will not close cannot hold the caller in a
/// close that the caller did not ask for and cannot cancel. Generous, because it
/// is a backstop rather than a target: a replay session closes in microseconds
/// and a native one should not need a second.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(5);

/// The contract dependencies one engine orchestrates.
///
/// This is the seam a composition root wires; it is not a plugin registry. The
/// facade is the only package that fills it in, because naming a concrete
/// adapter is the facade's responsibility and nobody else's.
///
/// Exactly one capture adapter, because a target identity is what every other
/// operation is addressed to and two capture adapters would issue two identity
/// spaces under one engine. Input and permission are optional and are the same
/// provider's or nothing: a capture-only engine is an ordinary engine, and an
/// input adapter handed another provider's target would be acting on an ordinal
/// that means nothing to it.
#[derive(Debug)]
pub struct EngineWiring {
    /// The identity every target and stream this engine accepts is scoped to.
    pub engine: EngineId,
    /// The capture adapter that discovers targets and opens sessions.
    pub capture: Arc<dyn CaptureProvider>,
    /// The matcher this engine prepares templates and searches frames through.
    pub matcher: Matcher,
    /// The loader every asset package this engine loads is validated by.
    pub loader: PackageLoader,
    /// The explicitly configured one-shot OCR recognizer, when there is one.
    ///
    /// `None` exposes no OCR operation and never selects a default backend.
    pub ocr: Option<OcrRecognizer>,
    /// The input adapter sessions deliver sequences through, when there is one.
    ///
    /// `None` is a capture-only engine: every target it describes accepts no
    /// input, and a session that requires input cannot be opened on it.
    pub input: Option<Arc<dyn InputProvider>>,
    /// The non-prompting authorization probe, when the platform has one.
    ///
    /// `None` is not "authorized" and not "refused": it says this engine reads
    /// no authorization state, which is the honest answer for a platform that
    /// grants none separately.
    pub permission: Option<Arc<dyn PermissionProbe>>,
}

/// What a caller asks for when opening a session.
///
/// A typed request rather than two arguments, because the capture options and
/// the input capability are separate decisions with separate failure modes and a
/// caller that wants neither should not have to say so twice. Input is `None` by
/// default: a session establishes only what it was asked for, and an engine that
/// happens to have an input adapter wired is not a reason to open an input
/// controller a caller never mentioned.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionRequest {
    capture: OpenRequest,
    input: Option<InputOpenRequest>,
}

impl SessionRequest {
    /// Returns a request for capture alone, with no capture constraints.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies `capture` to the capture side of the open.
    #[must_use]
    pub const fn capturing(mut self, capture: OpenRequest) -> Self {
        self.capture = capture;
        self
    }

    /// Also establishes the input `input` asks for.
    ///
    /// Whether an unavailable capability fails the open is the request's own
    /// [`InputRequirement`](mado_pilot_input::InputRequirement), so this is the
    /// one place a caller says "I need input" and the one place that says what
    /// happens when it is not there.
    #[must_use]
    pub fn requesting_input(mut self, input: InputOpenRequest) -> Self {
        self.input = Some(input);
        self
    }

    /// Returns the capture options.
    #[must_use]
    pub const fn capture(&self) -> &OpenRequest {
        &self.capture
    }

    /// Returns the input to establish, when the caller asked for any.
    #[must_use]
    pub const fn input(&self) -> Option<&InputOpenRequest> {
        self.input.as_ref()
    }
}

impl From<OpenRequest> for SessionRequest {
    fn from(capture: OpenRequest) -> Self {
        Self::new().capturing(capture)
    }
}

/// Engine-wide optional runtime behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EngineOptions {
    diagnostics: DiagnosticOptions,
}

impl EngineOptions {
    /// Returns the allocation-free default.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            diagnostics: DiagnosticOptions::off(),
        }
    }

    /// Enables the validated diagnostic stream configuration.
    #[must_use]
    pub const fn with_diagnostics(mut self, diagnostics: DiagnosticOptions) -> Self {
        self.diagnostics = diagnostics;
        self
    }

    /// Returns the selected diagnostic stream configuration.
    #[must_use]
    pub const fn diagnostics(self) -> DiagnosticOptions {
        self.diagnostics
    }
}

/// An engine over one capture adapter, one matching backend, and the optional
/// input and permission adapters that belong to the same provider.
#[derive(Debug)]
pub struct Engine {
    engine: EngineId,
    capture: Arc<dyn CaptureProvider>,
    matcher: Matcher,
    loader: PackageLoader,
    ocr: Option<OcrRecognizer>,
    ocr_diagnostic: Option<OcrDiagnosticContext>,
    input: Option<Arc<dyn InputProvider>>,
    permission: Option<Arc<dyn PermissionProbe>>,
    diagnostics: Option<DiagnosticSink>,
    diagnostic_reader: Mutex<Option<DiagnosticReader>>,
    watcher: WatchRuntime,
}

impl Engine {
    /// Builds an engine from wired contract dependencies.
    ///
    /// The pairing is checked here rather than at the first operation that would
    /// notice, because a mismatched wiring is a composition-root mistake and the
    /// operation that would notice is one that delivers input.
    ///
    /// # Errors
    ///
    /// Returns [`Status::InvalidArgument`] when the input provider or the
    /// permission probe reports a different provider than the capture adapter.
    pub fn new(wiring: EngineWiring) -> Result<Self, Error> {
        Self::new_with_options(wiring, EngineOptions::new())
    }

    /// Builds an engine with explicit engine-wide behavior.
    ///
    /// The default [`Engine::new`] path keeps diagnostics fully disabled.
    ///
    /// # Errors
    ///
    /// Returns [`Status::InvalidArgument`] for mismatched providers.
    pub fn new_with_options(wiring: EngineWiring, options: EngineOptions) -> Result<Self, Error> {
        let capture = wiring.capture.provider();
        if let Some(input) = wiring.input.as_ref() {
            check_provider_pair(capture, input.provider())?;
        }
        if let Some(permission) = wiring.permission.as_ref() {
            check_provider_pair(capture, permission.provider())?;
        }
        let (diagnostics, reader) = match DiagnosticSink::create(options.diagnostics()) {
            Some((sink, reader)) => (Some(sink), Some(reader)),
            None => (None, None),
        };
        let ocr_diagnostic = diagnostics
            .as_ref()
            .zip(wiring.ocr.as_ref())
            .and_then(|(diagnostics, ocr)| diagnostics.ocr_model(&ocr.descriptor()));
        let watcher = WatchRuntime::new(wiring.matcher.clone(), diagnostics.clone());

        Ok(Self {
            engine: wiring.engine,
            capture: wiring.capture,
            matcher: wiring.matcher,
            loader: wiring.loader,
            ocr: wiring.ocr,
            ocr_diagnostic,
            input: wiring.input,
            permission: wiring.permission,
            diagnostics,
            diagnostic_reader: Mutex::new(reader),
            watcher,
        })
    }

    /// Returns the identity that scopes this engine's targets and streams.
    ///
    /// A target identity issued by another engine is refused rather than
    /// resolved, so a caller holding two engines can tell which one an identity
    /// belongs to before it asks.
    #[must_use]
    pub const fn id(&self) -> EngineId {
        self.engine
    }

    /// Returns the matching backend that will produce every score.
    #[must_use]
    pub fn backend(&self) -> BackendDescriptor {
        self.matcher.descriptor()
    }

    /// Returns the fixed finite template watcher scheduler contract.
    #[must_use]
    pub fn template_scheduler(&self) -> TemplateSchedulerDescriptor {
        self.watcher.descriptor()
    }

    /// Returns the configured OCR backend/model/profile descriptor, if any.
    #[must_use]
    pub fn ocr_backend(&self) -> Option<OcrBackendDescriptor> {
        self.ocr.as_ref().map(OcrRecognizer::descriptor)
    }

    /// Returns immutable execution-provider initialization facts, when available.
    #[must_use]
    pub fn ocr_provider(&self) -> Option<OcrProviderDescriptor> {
        self.ocr
            .as_ref()
            .and_then(OcrRecognizer::provider_descriptor)
    }

    /// Returns the limits every package loaded through this engine is held to.
    #[must_use]
    pub const fn limits(&self) -> AssetLimits {
        self.loader.limits()
    }
    /// Takes the engine's one independently owned diagnostic reader.
    ///
    /// Returns `None` when diagnostics are off or a reader was already taken.
    /// Releasing a reader never changes operation behavior.
    #[must_use]
    pub fn take_diagnostic_reader(&self) -> Option<DiagnosticReader> {
        self.diagnostic_reader
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
    fn observe(
        &self,
        operation: &OperationContext,
        kind: DiagnosticOperationKind,
    ) -> Result<Option<ObservedOperation>, Error> {
        self.diagnostics
            .as_ref()
            .map(|diagnostics| diagnostics.observe(operation, kind))
            .transpose()
    }

    fn normal(
        &self,
        observed: Option<ObservedOperation>,
        operation: &OperationContext,
        payload: impl FnOnce() -> DiagnosticPayload,
    ) {
        if let (Some(diagnostics), Some(observed)) = (&self.diagnostics, observed) {
            diagnostics.normal(observed, operation, payload);
        }
    }

    /// Lists the targets this engine's capture adapter can currently capture.
    ///
    /// # Errors
    ///
    /// Returns a capture failure when the configured source cannot be read, and
    /// the operation's terminal outcome when cancellation or the deadline wins.
    pub fn discover(&self, operation: &OperationContext) -> Result<Vec<TargetDescription>, Error> {
        let _observed = self.observe(operation, DiagnosticOperationKind::Discovery)?;
        self.capture.discover(operation)
    }

    /// Reports whether this engine can deliver input at all.
    ///
    /// False for an engine wired without an input adapter. It answers a question
    /// about the engine, not about a target: an engine that can deliver input
    /// still has targets that accept none, which
    /// [`Engine::describe_input`] reports.
    #[must_use]
    pub fn delivers_input(&self) -> bool {
        self.input.is_some()
    }

    /// Reports whether this engine can read authorization states.
    ///
    /// False for a platform that grants none separately, which is not a claim
    /// that an operation will be authorized or refused.
    #[must_use]
    pub fn reads_permissions(&self) -> bool {
        self.permission.is_some()
    }

    /// Describes what `target` accepts as input, without establishing anything.
    ///
    /// A capture-only engine answers for its own targets rather than failing:
    /// through this engine the target accepts no input, which is what the
    /// descriptor then says. A foreign identity is still refused.
    ///
    /// How much is checked differs with the wiring, and the difference is
    /// visible rather than hidden. An engine with an input adapter asks that
    /// adapter, which knows whether the target is still there. A capture-only
    /// engine has nothing to ask — the capture contract offers no liveness query
    /// short of opening — so it checks only that the identity is one of its own
    /// and answers "no input" for a target that may since have gone. That answer
    /// stays true either way: a target this engine cannot deliver input to is
    /// one it cannot deliver input to.
    ///
    /// # Errors
    ///
    /// Returns an invalid-argument outcome for a target this engine did not
    /// issue, a target-lost outcome from an input adapter for one that no longer
    /// exists, and the operation's terminal outcome when cancellation or the
    /// deadline wins.
    pub fn describe_input(
        &self,
        target: TargetId,
        operation: &OperationContext,
    ) -> Result<InputDescriptor, Error> {
        let _observed = self.observe(operation, DiagnosticOperationKind::InputDescription)?;
        let attempt = Operation::admit(operation)?;
        let descriptor = match self.input.as_ref() {
            Some(input) => input.describe(target, operation)?,
            None => {
                self.capture.accepts_target(target, self.engine)?;
                InputDescriptor::new(target, InputCapability::none())
            }
        };
        Ok(attempt.commit(descriptor)?)
    }

    /// Reads both authorization states without asking the user for anything.
    ///
    /// # Errors
    ///
    /// Returns [`Status::Unsupported`] when this engine reads no authorization
    /// state, the probe's own failure when the read could not run, and the
    /// operation's terminal outcome when cancellation or the deadline wins.
    pub fn permissions(&self, operation: &OperationContext) -> Result<PermissionReport, Error> {
        let observed = self.observe(operation, DiagnosticOperationKind::Permission)?;
        let result = self.probe().and_then(|probe| probe.report(operation));
        match &result {
            Ok(report) => {
                for outcome in [report.capture(), report.input()] {
                    self.normal(observed, operation, || {
                        DiagnosticPayload::Permission(PermissionDiagnostic {
                            permission: outcome.kind(),
                            state: Some(outcome.state()),
                            fault: None,
                        })
                    });
                }
            }
            Err(error) => {
                for permission in PermissionKind::ALL {
                    self.normal(observed, operation, || {
                        DiagnosticPayload::Permission(PermissionDiagnostic {
                            permission,
                            state: None,
                            fault: Some(error.status()),
                        })
                    });
                }
            }
        }
        result
    }

    /// Reads one authorization state without asking the user for anything.
    ///
    /// # Errors
    ///
    /// As [`Engine::permissions`].
    pub fn permission(
        &self,
        kind: PermissionKind,
        operation: &OperationContext,
    ) -> Result<PermissionOutcome, Error> {
        let observed = self.observe(operation, DiagnosticOperationKind::Permission)?;
        let result = self.probe().and_then(|probe| probe.probe(kind, operation));
        self.normal(observed, operation, || {
            DiagnosticPayload::Permission(PermissionDiagnostic {
                permission: kind,
                state: result.as_ref().ok().map(|outcome| outcome.state()),
                fault: result.as_ref().err().map(|error| error.status()),
            })
        });
        result
    }

    fn probe(&self) -> Result<&Arc<dyn PermissionProbe>, Error> {
        self.permission.as_ref().ok_or_else(|| {
            Error::new(
                Status::Unsupported,
                "this engine reads no authorization state",
            )
        })
    }

    /// Opens a capture session for `target` that can search its own frames.
    ///
    /// Capture only, whatever this engine is wired with. Establishing input is a
    /// separate decision with its own failure mode, so it is asked for
    /// explicitly through [`Engine::open_session`] rather than acquired by
    /// having wired an input adapter.
    ///
    /// The adapter arbitrates the open and so does this method, which means the
    /// adapter can commit a session that this arbitration then refuses. A session
    /// that exists is closed rather than dropped in that case: dropping one does
    /// not close it, so the platform's side would outlive every reference to it.
    ///
    /// # Errors
    ///
    /// Returns an invalid-argument outcome for a target this engine did not
    /// issue, an unsupported outcome for a required option the adapter cannot
    /// honor, and the operation's terminal outcome when cancellation or the
    /// deadline wins. When that outcome arrives after the adapter committed and
    /// the session could not then be closed, the status is still the operation's
    /// but the detail names the release that failed.
    pub fn open(
        &self,
        target: TargetId,
        request: &OpenRequest,
        operation: &OperationContext,
    ) -> Result<Session, Error> {
        self.open_session(
            target,
            &SessionRequest::new().capturing(*request),
            operation,
        )
    }

    /// Opens a session for `target` establishing capture and the input the
    /// request asks for.
    ///
    /// Capture opens first and is what a target identity is scoped to, so it is
    /// also what has to be released when anything after it refuses. A required
    /// input capability that cannot be established therefore closes the capture
    /// session already committed for it and reports one open failure; an
    /// optional one that cannot be established opens the session capture-only,
    /// and [`Session::input_descriptor`] reports what was actually accepted.
    ///
    /// # Errors
    ///
    /// Returns an invalid-argument outcome for a target this engine did not
    /// issue, an unsupported outcome for a required capture option or a required
    /// input capability the adapter cannot honor, and the operation's terminal
    /// outcome when cancellation or the deadline wins. When a failure arrives
    /// after an adapter committed and what it committed could not then be
    /// closed, the status is still the failure's but the detail names the
    /// release that failed.
    pub fn open_session(
        &self,
        target: TargetId,
        request: &SessionRequest,
        operation: &OperationContext,
    ) -> Result<Session, Error> {
        let observed = self.observe(operation, DiagnosticOperationKind::SessionOpen)?;
        let result = (|| {
            let attempt = Operation::admit(operation)?;
            let capture = self.capture.open(target, request.capture(), operation)?;

            // Capture is committed from here, so every later refusal releases it.
            let input = match request.input() {
                None => None,
                Some(open) => match self.open_input(target, open, operation) {
                    Ok(input) => input,
                    Err(error) => return Err(release_capture(&capture, error)),
                },
            };

            // Commit on the unit before building the cheap public session value.
            let interruption = match attempt.commit(()) {
                Ok(()) => {
                    return Ok(Session::new(
                        capture,
                        self.matcher.clone(),
                        self.ocr.clone(),
                        self.ocr_diagnostic,
                        input,
                        self.diagnostics.clone(),
                        self.watcher.clone(),
                    ));
                }
                Err(interruption) => interruption,
            };

            let error = match input {
                Some(controller) => release_controller(&controller, Error::from(interruption)),
                None => Error::from(interruption),
            };
            Err(release_capture(&capture, error))
        })();
        self.normal(observed, operation, || {
            DiagnosticPayload::Lifecycle(LifecycleDiagnostic {
                target: Some(target),
                lifecycle: if result.is_ok() {
                    mado_pilot_core::Lifecycle::Open
                } else {
                    mado_pilot_core::Lifecycle::Closed
                },
                fault: result.as_ref().err().map(|error| error.status()),
            })
        });
        result
    }

    /// Establishes the input `request` asks for, or reports why it could not.
    ///
    /// `Ok(None)` is the optional case: nothing was established and the session
    /// is truthfully capture-only. Only a *required* capability turns a refusal
    /// into an open failure.
    ///
    /// Optional means optional, including when the adapter's refusal is a
    /// terminal one. Whether the caller's operation is over is not the adapter's
    /// answer to give — it is decided by this engine's own final arbitration a
    /// few lines above, which catches an expired operation whatever the adapter
    /// said and releases the capture committed for it. Refusing here on a
    /// terminal status would be that check with an extra step in every case it
    /// can actually reach, and in the one case it could not — an adapter
    /// reporting a terminal status of its own while the caller's operation is
    /// still running — it would quietly make an explicitly optional capability
    /// required.
    fn open_input(
        &self,
        target: TargetId,
        request: &InputOpenRequest,
        operation: &OperationContext,
    ) -> Result<Option<Arc<dyn InputController>>, Error> {
        let Some(provider) = self.input.as_ref() else {
            return if request.requirement().is_required() {
                Err(InputFault::RouteUnavailable.into())
            } else {
                Ok(None)
            };
        };

        match provider.open(target, request, operation) {
            Ok(controller) => Ok(Some(controller)),
            Err(error) if request.requirement().is_required() => Err(error),
            Err(_) => Ok(None),
        }
    }

    /// Loads and validates an asset package from `source`.
    ///
    /// This reports the asset package's own typed fault rather than the shared
    /// error, because which rule a package broke and how far loading had got
    /// are both part of that contract, and flattening them into a status would
    /// leave a caller reading the message to tell a bad hash from a bad path.
    ///
    /// # Errors
    ///
    /// Returns an [`AssetFault`] carrying the rule that was broken and the
    /// stage that caught it.
    pub fn load_package(
        &self,
        source: &PackageSource,
        operation: &OperationContext,
    ) -> Result<AssetPackage, AssetFault> {
        self.loader.load(source, operation)
    }

    /// Loads and validates an asset package from an archive this call borrows.
    ///
    /// The same stages and the same typed faults as [`Engine::load_package`];
    /// what differs is that nothing has to own the archive. A caller — or a
    /// boundary holding a caller's view for one call — hands the bytes over for
    /// the duration of the load rather than buying a copy of them, and the
    /// committed package owns each template's content independently.
    ///
    /// # Errors
    ///
    /// Returns an [`AssetFault`] carrying the rule that was broken and the
    /// stage that caught it, including the loader's configured source ceiling.
    pub fn load_archive_bytes(
        &self,
        bytes: &[u8],
        operation: &OperationContext,
    ) -> Result<AssetPackage, AssetFault> {
        self.loader.load_archive_bytes(bytes, operation)
    }

    /// Compiles a template source for this engine's backend.
    ///
    /// A template source does not have to come from an asset package: the
    /// vision contract accepts one built from bytes a caller read itself, and
    /// an engine that only accepted packaged templates would make the manifest
    /// mandatory for a caller that has no use for it. The packaged form is
    /// [`Engine::prepare_from_package`]; the two differ in where the source
    /// comes from and in nothing else.
    ///
    /// # Errors
    ///
    /// Returns the backend's typed preparation failure, and the operation's
    /// terminal outcome when cancellation or the deadline wins.
    pub fn prepare_template(
        &self,
        source: &TemplateSource,
        operation: &OperationContext,
    ) -> Result<PreparedTemplate, Error> {
        let _observed = self.observe(operation, DiagnosticOperationKind::TemplatePreparation)?;
        let prepared = self.matcher.prepare(source, operation)?;
        if let Some(diagnostics) = &self.diagnostics {
            diagnostics.register_template(&prepared);
        }
        Ok(prepared)
    }

    /// Compiles the template `id` names in `package` for this engine's backend.
    ///
    /// Resolution and compilation run under one operation, so a request that
    /// loses its race is refused rather than half-applied, and an identity the
    /// package does not contain is refused before the backend is asked to do
    /// any work at all.
    ///
    /// The asset detail behind a missing identity is available from
    /// [`AssetPackage::resolve_template`] for a caller that needs to
    /// distinguish it from another asset failure by more than its status.
    ///
    /// # Errors
    ///
    /// Returns an invalid-argument outcome when `package` contains no template
    /// with that identity — a package that loaded is valid, and asking it for
    /// something it never declared is the caller's mistake — the backend's
    /// typed preparation failure, and the operation's terminal outcome when
    /// cancellation or the deadline wins.
    pub fn prepare_from_package(
        &self,
        package: &AssetPackage,
        id: &str,
        operation: &OperationContext,
    ) -> Result<PreparedTemplate, Error> {
        let _observed = self.observe(operation, DiagnosticOperationKind::TemplatePreparation)?;
        let mut attempt = Operation::admit(operation)?;
        let source = package.resolve_template(id)?;
        attempt.checkpoint()?;

        let prepared = self.matcher.prepare(&source, operation)?;
        if let Some(diagnostics) = &self.diagnostics {
            diagnostics.register_template(&prepared);
        }
        Ok(attempt.commit(prepared)?)
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.watcher.close();
    }
}

/// Returns the context a release runs under.
///
/// A fresh one, because the caller's operation is what refused the open and is
/// usually already over: passing it would leave the adapter in
/// [`Lifecycle::Closing`](mado_pilot_capture::Lifecycle) — a leak with an extra
/// step. Bounded, so an adapter that will not close turns a refusal the caller
/// cannot cancel into a slow one rather than a hang.
fn release_context() -> OperationContext {
    OperationContext::new()
        .with_timeout(RELEASE_TIMEOUT)
        .unwrap_or_else(|_| OperationContext::new())
}

/// Closes `capture` and reports `refusal`, naming a release that itself failed.
fn release_capture(capture: &Arc<dyn CaptureSession>, refusal: Error) -> Error {
    match capture.close(&release_context()) {
        Ok(()) => refusal,
        Err(error) => Error::new(
            refusal.status(),
            format!(
                "{}, and the capture session opened for it could not be closed: {}",
                refusal.detail(),
                error.detail()
            ),
        ),
    }
}

/// Closes `controller` and reports `refusal`, naming a release that itself
/// failed.
fn release_controller(controller: &Arc<dyn InputController>, refusal: Error) -> Error {
    match controller.close(&release_context()) {
        Ok(()) => refusal,
        Err(error) => Error::new(
            refusal.status(),
            format!(
                "{}, and the input controller opened for it could not be closed: {}",
                refusal.detail(),
                error.detail()
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::any::Any;
    use std::sync::Arc;

    use mado_pilot_capture::{CaptureProvider, Continuity, OpenRequest, PixelFormat};
    use mado_pilot_core::{IdentityIssuer, OperationContext, PixelExtent};
    use mado_pilot_testkit::{ControlledCapture, ControlledMatcher, match_fixtures};
    use mado_pilot_vision::{
        BackendId, MatchBackend, MatchOptions, PreparedTemplate, TemplatePayload,
    };

    use crate::diagnostic::{DiagnosticDrain, DiagnosticPayload, MAX_DIAGNOSTIC_CAPACITY};
    use crate::find::FindRequest;

    use super::*;

    #[derive(Debug)]
    struct MetadataPayload;

    impl TemplatePayload for MetadataPayload {
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    #[test]
    fn template_metadata_ceiling_does_not_change_prepare_or_search_outcomes() {
        let issuer = Arc::new(IdentityIssuer::new());
        let capture = Arc::new(
            ControlledCapture::new(
                Arc::clone(&issuer),
                PixelExtent::new(32, 24),
                PixelFormat::Rgba8,
            )
            .expect("valid controlled capture"),
        );
        let backend = Arc::new(ControlledMatcher::new(PixelFormat::Rgba8));
        let engine = Engine::new_with_options(
            EngineWiring {
                engine: issuer.engine(),
                capture: Arc::clone(&capture) as Arc<dyn CaptureProvider>,
                matcher: Matcher::new(Arc::clone(&backend) as Arc<dyn MatchBackend>),
                loader: PackageLoader::new(),
                ocr: None,
                input: None,
                permission: None,
            },
            EngineOptions::new()
                .with_diagnostics(DiagnosticOptions::normal(4).expect("valid diagnostic capacity")),
        )
        .expect("valid engine");
        let reader = engine
            .take_diagnostic_reader()
            .expect("enabled diagnostic reader");
        let operation = OperationContext::new();
        let target = engine
            .discover(&operation)
            .expect("discovered")
            .remove(0)
            .id();
        let session = engine
            .open(target, &OpenRequest::new(), &operation)
            .expect("opened");
        capture
            .publish(0x11, Continuity::Continuous)
            .expect("published");
        assert!(
            matches!(reader.drain(), DiagnosticDrain::Batch(_)),
            "the open lifecycle is drained before the search assertion"
        );

        let source = match_fixtures::planted_template("metadata-ceiling-template");
        let metadata_backend = BackendId::new("diagnostic-metadata");
        let live_metadata: Vec<_> = (0..MAX_DIAGNOSTIC_CAPACITY)
            .map(|_| {
                PreparedTemplate::new(metadata_backend.clone(), &source, Arc::new(MetadataPayload))
            })
            .collect();
        let diagnostics = engine.diagnostics.as_ref().expect("enabled diagnostics");
        for template in &live_metadata {
            diagnostics.register_template(template);
        }

        let prepared = engine
            .prepare_template(&source, &operation)
            .expect("diagnostic metadata cannot fail preparation");
        assert_eq!(prepared.id(), source.id());
        assert_eq!(backend.prepare_count(), 1);

        let outcome = session
            .find_template(
                &FindRequest::latest(&prepared, MatchOptions::from_defaults(prepared.defaults())),
                &operation,
            )
            .expect("diagnostic metadata cannot fail search");
        assert!(outcome.result().is_empty());
        assert_eq!(backend.find_count(), 1);

        let DiagnosticDrain::Batch(batch) = reader.drain() else {
            panic!("an omitted normal search record reports a loss-only batch");
        };
        assert!(batch.is_empty());
        assert_eq!(batch.losses().normal(), 1);
        assert_eq!(batch.losses().debug(), 0);

        drop(live_metadata);
        let reclaimed = session
            .find_template(
                &FindRequest::latest(&prepared, MatchOptions::from_defaults(prepared.defaults())),
                &operation,
            )
            .expect("expired diagnostic metadata is reclaimed before the ceiling refuses");
        assert!(reclaimed.result().is_empty());
        let DiagnosticDrain::Batch(batch) = reader.drain() else {
            panic!("the reclaimed search record is retained");
        };
        assert!(batch.losses().is_empty());
        assert_eq!(batch.records().len(), 1);
        assert!(matches!(
            batch.records()[0].payload(),
            DiagnosticPayload::Search(search) if search.region == Some(reclaimed.result().searched())
        ));
    }
}
