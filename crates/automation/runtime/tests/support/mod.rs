//! Engines wired from controlled doubles.
//!
//! Orchestration is tested against doubles rather than against the production
//! adapters, because the states worth checking here — a backend that answers
//! after cancellation, a deadline that passes mid-search, a frame published
//! while a search is in flight — are ones a real adapter reaches rarely and
//! never on demand.

use std::path::PathBuf;
use std::sync::Arc;

use mado_pilot_runtime::{
    CaptureProvider, Engine, EngineParts, IdentityIssuer, Matcher, PackageLoader, PixelExtent,
    PixelFormat,
};
use mado_pilot_testkit::{ControlledCapture, ControlledMatcher};
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
        let engine = Engine::new(EngineParts {
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
}

/// Returns the tracked Phase 1 example package directory.
pub(crate) fn package_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/assets/phase1-slice")
}
