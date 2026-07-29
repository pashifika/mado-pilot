//! Engines wired from controlled doubles.
//!
//! Orchestration is tested against doubles rather than against the production
//! adapters, because the states worth checking here — a backend that answers
//! after cancellation, a deadline that passes mid-search, a frame published
//! while a search is in flight — are ones a real adapter reaches rarely and
//! never on demand.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use mado_pilot_capture::{
    CaptureSession, Frame, FrameRequest, Lifecycle, OpenRequest, SessionDescription,
};
use mado_pilot_core::{OperationContext, ProviderId, Result, TargetId};
use mado_pilot_runtime::{
    CaptureProvider, Engine, EngineWiring, IdentityIssuer, Matcher, PackageLoader, PixelExtent,
    PixelFormat, TargetDescription,
};
use mado_pilot_testkit::{ControlledCapture, ControlledMatcher, ManualClock};
use mado_pilot_vision::MatchBackend;

/// The extent every controlled capture in these tests publishes.
pub(crate) const EXTENT: PixelExtent = PixelExtent::new(32, 24);

/// An engine over controlled doubles, with both doubles still reachable.
pub(crate) struct Harness {
    pub(crate) engine: Engine,
    pub(crate) capture: Arc<ControlledCapture>,
    pub(crate) matcher: Arc<ControlledMatcher>,
}

impl Harness {
    /// Wires an engine over `matcher` and a fresh controlled capture provider.
    pub(crate) fn new(matcher: ControlledMatcher) -> Self {
        let issuer = Arc::new(IdentityIssuer::new());
        let capture = Arc::new(
            ControlledCapture::new(Arc::clone(&issuer), EXTENT, PixelFormat::Rgba8)
                .expect("a valid controlled provider"),
        );
        let matcher = Arc::new(matcher);
        let engine = Engine::new(EngineWiring {
            engine: issuer.engine(),
            capture: Arc::clone(&capture) as Arc<dyn CaptureProvider>,
            matcher: Matcher::new(Arc::clone(&matcher) as Arc<dyn MatchBackend>),
            loader: PackageLoader::new(),
        });

        Self {
            engine,
            capture,
            matcher,
        }
    }

    /// Wires an engine over a matcher that prepares anything and finds nothing.
    pub(crate) fn silent() -> Self {
        Self::new(ControlledMatcher::new(PixelFormat::Rgba8))
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
        });

        Self {
            engine,
            capture: inner,
            matcher,
        }
    }
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
