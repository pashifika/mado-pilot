//! Engines wired from controlled doubles.
//!
//! Orchestration is tested against doubles rather than against the production
//! adapters, because the states worth checking here — a backend that answers
//! after cancellation, a deadline that passes mid-search, a frame published
//! while a search is in flight — are ones a real adapter reaches rarely and
//! never on demand.

// The module is shared by `mod support;` in each test binary, so items unused by
// one of them are expected rather than accidental.
#![allow(dead_code)]

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use mado_pilot_capture::{
    CaptureSession, Frame, FrameRequest, Lifecycle, OpenRequest, SessionDescription,
};
use mado_pilot_core::{OperationContext, ProviderId, Result, TargetId};
use mado_pilot_input::Admission;
use mado_pilot_runtime::{
    CapabilitySupport, CaptureProvider, DiagnosticOptions, Engine, EngineOptions, EngineWiring,
    IdentityIssuer, InputCapability, InputController, InputDelivery, InputDescriptor, InputFault,
    InputOpenRequest, InputOperationKind, InputProvider, InputReceipt, InputRequest, Matcher,
    PackageLoader, PermissionProbe, PixelExtent, PixelFormat, SubmissionEvidence,
    TargetDescription,
};
use mado_pilot_testkit::{ControlledCapture, ControlledInput, ControlledMatcher, ManualClock};
use mado_pilot_vision::MatchBackend;

/// The extent every controlled capture in these tests publishes.
pub(crate) const EXTENT: PixelExtent = PixelExtent::new(32, 24);

/// An engine over controlled doubles, with every double still reachable.
pub(crate) struct Harness {
    pub(crate) engine: Engine,
    pub(crate) capture: Arc<ControlledCapture>,
    pub(crate) matcher: Arc<ControlledMatcher>,
    /// The input double, for the engines wired with one.
    pub(crate) input: Option<Arc<ControlledInput>>,
}

impl Harness {
    /// Wires a capture-only engine over `matcher` and a fresh capture provider.
    pub(crate) fn new(matcher: ControlledMatcher) -> Self {
        Self::build(matcher, false)
    }

    /// Wires an engine over a matcher that prepares anything and finds nothing.
    pub(crate) fn silent() -> Self {
        Self::new(ControlledMatcher::new(PixelFormat::Rgba8))
    }

    /// Wires a capture-only engine with finite debug diagnostics.
    pub(crate) fn with_diagnostics(matcher: ControlledMatcher, capacity: usize) -> Self {
        Self::build_with_options(
            matcher,
            false,
            EngineOptions::new().with_diagnostics(
                DiagnosticOptions::debug(capacity).expect("valid diagnostic capacity"),
            ),
        )
    }

    /// Wires the same engine with the capture provider's own input double.
    pub(crate) fn with_input() -> Self {
        Self::build(ControlledMatcher::new(PixelFormat::Rgba8), true)
    }

    /// Wires the input harness with a finite diagnostic stream.
    pub(crate) fn with_input_and_diagnostics() -> Self {
        Self::build_with_options(
            ControlledMatcher::new(PixelFormat::Rgba8),
            true,
            EngineOptions::new()
                .with_diagnostics(DiagnosticOptions::normal(8).expect("valid diagnostic capacity")),
        )
    }

    fn build(matcher: ControlledMatcher, input: bool) -> Self {
        Self::build_with_options(matcher, input, EngineOptions::new())
    }

    fn build_with_options(matcher: ControlledMatcher, input: bool, options: EngineOptions) -> Self {
        let issuer = Arc::new(IdentityIssuer::new());
        let capture = Arc::new(
            ControlledCapture::new(Arc::clone(&issuer), EXTENT, PixelFormat::Rgba8)
                .expect("a valid controlled provider"),
        );
        let matcher = Arc::new(matcher);
        let input = input.then(|| Arc::new(ControlledInput::new(capture.target())));
        let engine = Engine::new_with_options(
            EngineWiring {
                engine: issuer.engine(),
                capture: Arc::clone(&capture) as Arc<dyn CaptureProvider>,
                matcher: Matcher::new(Arc::clone(&matcher) as Arc<dyn MatchBackend>),
                loader: PackageLoader::new(),
                ocr: None,
                input: input
                    .as_ref()
                    .map(|input| Arc::clone(input) as Arc<dyn InputProvider>),
                permission: None,
            },
            options,
        )
        .expect("the doubles share one provider identity");

        Self {
            engine,
            capture,
            matcher,
            input,
        }
    }

    /// Returns the input double this harness was wired with.
    pub(crate) fn input(&self) -> &Arc<ControlledInput> {
        self.input.as_ref().expect("wired with an input double")
    }

    /// Wires an engine over `capture` rather than over a fresh controlled one.
    ///
    /// For the cases where the provider itself is what is being tested. `issuer`
    /// has to be the one the wrapped provider was built on, or the engine will
    /// refuse its own targets as foreign. The `capture` field carries the inner
    /// provider the caller wrapped.
    pub(crate) fn over(
        issuer: &IdentityIssuer,
        inner: Arc<ControlledCapture>,
        capture: Arc<dyn CaptureProvider>,
    ) -> Self {
        let matcher = Arc::new(ControlledMatcher::new(PixelFormat::Rgba8));
        let engine = Engine::new(EngineWiring {
            engine: issuer.engine(),
            capture,
            matcher: Matcher::new(Arc::clone(&matcher) as Arc<dyn MatchBackend>),
            loader: PackageLoader::new(),
            ocr: None,
            input: None,
            permission: None,
        })
        .expect("a capture-only engine pairs with nothing");

        Self {
            engine,
            capture: inner,
            matcher,
            input: None,
        }
    }
}

/// Wires an engine from parts a test assembled itself.
///
/// The provider-pairing and permission cases need combinations [`Harness`]
/// deliberately cannot produce — an input double from another provider, a probe
/// that answers from a script — so those tests build the wiring directly and
/// this only saves them the fields they do not care about.
pub(crate) fn wire(
    issuer: &IdentityIssuer,
    capture: Arc<dyn CaptureProvider>,
    input: Option<Arc<dyn InputProvider>>,
    permission: Option<Arc<dyn PermissionProbe>>,
) -> Result<Engine> {
    Engine::new(EngineWiring {
        engine: issuer.engine(),
        capture,
        matcher: Matcher::new(Arc::new(ControlledMatcher::new(PixelFormat::Rgba8))),
        loader: PackageLoader::new(),
        ocr: None,
        input,
        permission,
    })
}

/// Returns a controlled provider and the issuer it was built on.
pub(crate) fn controlled_capture() -> (Arc<IdentityIssuer>, Arc<ControlledCapture>) {
    let issuer = Arc::new(IdentityIssuer::new());
    let capture = Arc::new(
        ControlledCapture::new(Arc::clone(&issuer), EXTENT, PixelFormat::Rgba8)
            .expect("a valid controlled provider"),
    );
    (issuer, capture)
}

/// Returns the tracked Phase 1 example package directory.
pub(crate) fn package_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/assets/phase1-slice")
}

/// A provider whose open succeeds and then makes the caller's operation lose.
///
/// It delegates to a real controlled provider, so the session handed back is one
/// that can actually be closed, and then advances `clock` past the caller's
/// deadline — after the inner adapter has already committed its own open. That
/// window is the only one in which `Engine::open` holds a committed session it is
/// about to refuse, and it cannot be reached by preparing an expired context: a
/// context already over is refused before the adapter opens anything.
///
/// Every close reaching a session it produced is counted, which is how a test
/// tells "closed" from "dropped" — the two are indistinguishable from the outside
/// otherwise, and that is exactly what made the leak invisible.
pub(crate) struct OpenThenExpire {
    inner: Arc<ControlledCapture>,
    clock: Arc<ManualClock>,
    step: Duration,
    closes: Arc<AtomicUsize>,
}

impl OpenThenExpire {
    pub(crate) fn new(
        inner: Arc<ControlledCapture>,
        clock: Arc<ManualClock>,
        step: Duration,
    ) -> Self {
        Self {
            inner,
            clock,
            step,
            closes: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Returns a handle to the close counter, readable after the engine ran.
    pub(crate) fn closes(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.closes)
    }
}

impl fmt::Debug for OpenThenExpire {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenThenExpire")
            .field("closes", &self.closes.load(Ordering::Relaxed))
            .finish()
    }
}

impl CaptureProvider for OpenThenExpire {
    fn provider(&self) -> ProviderId {
        self.inner.provider()
    }

    fn discover(&self, operation: &OperationContext) -> Result<Vec<TargetDescription>> {
        self.inner.discover(operation)
    }

    fn open(
        &self,
        target: TargetId,
        request: &OpenRequest,
        operation: &OperationContext,
    ) -> Result<Arc<dyn CaptureSession>> {
        let session = self.inner.open(target, request, operation)?;
        // After the inner commit, before the outer one.
        self.clock.advance(self.step);
        Ok(Arc::new(CountedClose {
            inner: session,
            closes: Arc::clone(&self.closes),
        }))
    }
}

/// A provider that counts every close its sessions are asked for.
///
/// Closing and dropping a session are indistinguishable from outside, which is
/// what makes a capture session leaked by a refused open invisible. Counting is
/// how a test tells them apart.
pub(crate) struct CountingCapture {
    inner: Arc<ControlledCapture>,
    closes: Arc<AtomicUsize>,
}

impl CountingCapture {
    pub(crate) fn new(inner: Arc<ControlledCapture>) -> Self {
        Self {
            inner,
            closes: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Returns a handle to the close counter, readable after the engine ran.
    pub(crate) fn closes(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.closes)
    }
}

impl fmt::Debug for CountingCapture {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CountingCapture")
            .field("closes", &self.closes.load(Ordering::Relaxed))
            .finish()
    }
}

impl CaptureProvider for CountingCapture {
    fn provider(&self) -> ProviderId {
        self.inner.provider()
    }

    fn discover(&self, operation: &OperationContext) -> Result<Vec<TargetDescription>> {
        self.inner.discover(operation)
    }

    fn open(
        &self,
        target: TargetId,
        request: &OpenRequest,
        operation: &OperationContext,
    ) -> Result<Arc<dyn CaptureSession>> {
        let session = self.inner.open(target, request, operation)?;
        Ok(Arc::new(CountedClose {
            inner: session,
            closes: Arc::clone(&self.closes),
        }))
    }
}

/// One session that counts the closes it is asked for.
#[derive(Debug)]
struct CountedClose {
    inner: Arc<dyn CaptureSession>,
    closes: Arc<AtomicUsize>,
}

impl CaptureSession for CountedClose {
    fn description(&self) -> SessionDescription {
        self.inner.description()
    }

    fn frame(&self, request: &FrameRequest, operation: &OperationContext) -> Result<Frame> {
        self.inner.frame(request, operation)
    }

    fn close(&self, operation: &OperationContext) -> Result<()> {
        self.closes.fetch_add(1, Ordering::Relaxed);
        self.inner.close(operation)
    }

    fn lifecycle(&self) -> Lifecycle {
        self.inner.lifecycle()
    }
}

/// An input adapter whose controller answers after the caller's clock has moved
/// past the deadline.
///
/// The late-result rule cannot be reached with a context that is already over —
/// that one is refused before the controller is asked anything. The only window
/// in which runtime holds an answer to an operation that has since lost its race
/// is the one this opens deliberately, by advancing the clock inside `execute`.
pub(crate) struct LateAnswer {
    target: TargetId,
    clock: Arc<ManualClock>,
    step: Duration,
    receipt: Answer,
}

/// What the late controller answers with once the clock has moved.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Answer {
    /// No native effect was possible.
    Unexecuted,
    /// This many complete logical events reached the route's threshold.
    Partial(usize),
}

impl LateAnswer {
    pub(crate) fn new(
        target: TargetId,
        clock: Arc<ManualClock>,
        step: Duration,
        receipt: Answer,
    ) -> Self {
        Self {
            target,
            clock,
            step,
            receipt,
        }
    }
}

impl fmt::Debug for LateAnswer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LateAnswer")
            .field("answer", &self.receipt)
            .finish()
    }
}

impl InputProvider for LateAnswer {
    fn provider(&self) -> ProviderId {
        ProviderId::new("controlled")
    }

    fn describe(&self, target: TargetId, _operation: &OperationContext) -> Result<InputDescriptor> {
        Ok(InputDescriptor::new(target, late_capability()))
    }

    fn open(
        &self,
        target: TargetId,
        request: &InputOpenRequest,
        _operation: &OperationContext,
    ) -> Result<Arc<dyn InputController>> {
        request.check(late_capability())?;
        Ok(Arc::new(LateController {
            descriptor: InputDescriptor::new(target, late_capability()),
            target: self.target,
            clock: Arc::clone(&self.clock),
            step: self.step,
            receipt: self.receipt,
            admission: Admission::new(),
        }))
    }
}

fn late_capability() -> InputCapability {
    InputCapability::none()
        .with_pair(
            InputOperationKind::Keyboard,
            InputDelivery::System,
            CapabilitySupport::Supported,
            SubmissionEvidence::SystemInputAdmission,
        )
        .with_pair(
            InputOperationKind::Text,
            InputDelivery::System,
            CapabilitySupport::Supported,
            SubmissionEvidence::SystemInputAdmission,
        )
}

struct LateController {
    descriptor: InputDescriptor,
    target: TargetId,
    clock: Arc<ManualClock>,
    step: Duration,
    receipt: Answer,
    admission: Admission,
}

impl fmt::Debug for LateController {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LateController")
            .field("target", &self.target)
            .finish()
    }
}

impl InputController for LateController {
    fn descriptor(&self) -> InputDescriptor {
        self.descriptor.clone()
    }

    fn execute(
        &self,
        request: &InputRequest,
        operation: &OperationContext,
    ) -> Result<InputReceipt> {
        let _guard = self.admission.admit(operation)?;
        // After the controller committed to answering, before runtime reads it.
        self.clock.advance(self.step);
        Ok(match self.receipt {
            Answer::Unexecuted => {
                InputReceipt::unexecuted(request.target(), InputFault::PolicyRefused)
            }
            Answer::Partial(submitted) => InputReceipt::partial(
                request.target(),
                InputDelivery::System,
                SubmissionEvidence::SystemInputAdmission,
                submitted,
                false,
                InputFault::SubmissionFailed,
            )
            .with_cleanup(0, 0),
        })
    }

    fn close(&self, operation: &OperationContext) -> Result<()> {
        self.admission.drain(operation)
    }

    fn lifecycle(&self) -> Lifecycle {
        self.admission.lifecycle()
    }
}

/// An input adapter whose open succeeds and then makes the caller's operation
/// lose.
///
/// The capture twin of this is [`OpenThenExpire`], and it exists for the same
/// reason: the only window in which the engine holds a *committed input
/// controller* it is about to refuse is the one after the adapter's own open
/// commits and before the engine's arbitration runs, and no prepared context
/// reaches it — a context already over is refused before the adapter opens
/// anything.
///
/// Every close reaching a controller it produced is counted, because closing one
/// and dropping one are indistinguishable from outside.
pub(crate) struct OpenInputThenExpire {
    inner: Arc<ControlledInput>,
    clock: Arc<ManualClock>,
    step: Duration,
    closes: Arc<AtomicUsize>,
}

impl OpenInputThenExpire {
    pub(crate) fn new(
        inner: Arc<ControlledInput>,
        clock: Arc<ManualClock>,
        step: Duration,
    ) -> Self {
        Self {
            inner,
            clock,
            step,
            closes: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Returns a handle to the close counter, readable after the engine ran.
    pub(crate) fn closes(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.closes)
    }
}

impl fmt::Debug for OpenInputThenExpire {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenInputThenExpire")
            .field("closes", &self.closes.load(Ordering::Relaxed))
            .finish()
    }
}

impl InputProvider for OpenInputThenExpire {
    fn provider(&self) -> ProviderId {
        self.inner.provider()
    }

    fn describe(&self, target: TargetId, operation: &OperationContext) -> Result<InputDescriptor> {
        self.inner.describe(target, operation)
    }

    fn open(
        &self,
        target: TargetId,
        request: &InputOpenRequest,
        operation: &OperationContext,
    ) -> Result<Arc<dyn InputController>> {
        let controller = self.inner.open(target, request, operation)?;
        // After the inner commit, before the outer one.
        self.clock.advance(self.step);
        Ok(Arc::new(CountedControllerClose {
            inner: controller,
            closes: Arc::clone(&self.closes),
        }))
    }
}

/// One controller that counts the closes it is asked for.
#[derive(Debug)]
struct CountedControllerClose {
    inner: Arc<dyn InputController>,
    closes: Arc<AtomicUsize>,
}

impl InputController for CountedControllerClose {
    fn descriptor(&self) -> InputDescriptor {
        self.inner.descriptor()
    }

    fn execute(
        &self,
        request: &InputRequest,
        operation: &OperationContext,
    ) -> Result<InputReceipt> {
        self.inner.execute(request, operation)
    }

    fn close(&self, operation: &OperationContext) -> Result<()> {
        self.closes.fetch_add(1, Ordering::Relaxed);
        self.inner.close(operation)
    }

    fn lifecycle(&self) -> Lifecycle {
        self.inner.lifecycle()
    }
}
