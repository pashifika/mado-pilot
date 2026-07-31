//! The provider and session contracts an adapter implements.

use std::fmt::Debug;
use std::sync::Arc;

use mado_pilot_core::{EngineId, OperationContext, ProviderId, Result, TargetId};

use crate::descriptor::{PixelFormat, SessionDescription, TargetDescription};
use crate::discovery::DiscoveryRequest;
use crate::fault::CaptureFault;
use crate::frame::Frame;
use crate::stream::{FrameRequest, Lifecycle};

/// What a caller asks for when opening a session.
///
/// Required options and preferences are separate axes on purpose. A required
/// option that cannot be honored fails the open; a preference that cannot be
/// honored falls back, and the session description then reports what was
/// actually accepted. Collapsing the two would mean a caller either cannot
/// express "I need this" or cannot tell whether they got it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OpenRequest {
    required_format: Option<PixelFormat>,
    preferred_format: Option<PixelFormat>,
}

impl OpenRequest {
    /// Returns a request with no constraints.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            required_format: None,
            preferred_format: None,
        }
    }

    /// Requires this pixel format, failing the open when it is unavailable.
    #[must_use]
    pub const fn require_format(mut self, format: PixelFormat) -> Self {
        self.required_format = Some(format);
        self
    }

    /// Prefers this pixel format, accepting the source's own when unavailable.
    #[must_use]
    pub const fn prefer_format(mut self, format: PixelFormat) -> Self {
        self.preferred_format = Some(format);
        self
    }

    /// Returns the required pixel format, if any.
    #[must_use]
    pub const fn required_format(&self) -> Option<PixelFormat> {
        self.required_format
    }

    /// Returns the preferred pixel format, if any.
    #[must_use]
    pub const fn preferred_format(&self) -> Option<PixelFormat> {
        self.preferred_format
    }
}

/// A source of capture targets.
///
/// Implemented by capture adapters — the replay adapter in Phase 1, the platform
/// adapters later. Nothing above this trait names a concrete adapter except the
/// facade, which is where default wiring belongs.
pub trait CaptureProvider: Debug + Send + Sync {
    /// Returns the provider that qualifies this adapter's target identities.
    fn provider(&self) -> ProviderId;

    /// Lists the targets this provider can currently capture.
    ///
    /// # Errors
    ///
    /// Returns the operation's terminal outcome, or a capture failure when the
    /// configured source cannot be read.
    fn discover(&self, operation: &OperationContext) -> Result<Vec<TargetDescription>>;

    /// Lists the targets that satisfy `request`.
    ///
    /// The default narrows [`CaptureProvider::discover`], which is what makes a
    /// filter unable to reach a target the provider did not list and unable to
    /// change the order it listed them in. An Adapter overrides this only when the
    /// platform can answer the same question more cheaply, and must then produce
    /// the same targets in the same order as the default would.
    ///
    /// # Errors
    ///
    /// As [`CaptureProvider::discover`].
    fn discover_matching(
        &self,
        request: &DiscoveryRequest,
        operation: &OperationContext,
    ) -> Result<Vec<TargetDescription>> {
        let mut targets = self.discover(operation)?;
        targets.retain(|target| request.accepts(target));
        Ok(targets)
    }

    /// Confirms that this provider issued `target`, for `engine`.
    ///
    /// Every Adapter performs this check before acting on a caller's target, so
    /// it is written once here: an identity from another engine or another
    /// provider names something this provider knows nothing about, and acting on
    /// its ordinal would operate on an unrelated target.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureFault::ForeignTarget`] when either half does not match.
    fn accepts_target(&self, target: TargetId, engine: EngineId) -> Result<()> {
        target
            .check_issued_by(engine, self.provider())
            .map_err(|_| CaptureFault::ForeignTarget)?;
        Ok(())
    }

    /// Opens a capture session for `target`.
    ///
    /// # Errors
    ///
    /// Returns an invalid-argument outcome for a target this provider did not
    /// issue, an unsupported outcome for a required option it cannot honor, and
    /// the operation's terminal outcome when cancellation or the deadline wins.
    fn open(
        &self,
        target: TargetId,
        request: &OpenRequest,
        operation: &OperationContext,
    ) -> Result<Arc<dyn CaptureSession>>;
}

/// One open capture session.
///
/// A session maintains stream state rather than capturing on demand: a frame
/// request consumes what the session has already published, or waits for the
/// next publication. There is deliberately no one-shot capture entry point,
/// because a second acquisition path would need its own identity rules and the
/// two would drift.
pub trait CaptureSession: Debug + Send + Sync {
    /// Returns what this session actually accepted.
    fn description(&self) -> SessionDescription;

    /// Returns the frame `request` asks for, waiting when necessary.
    ///
    /// # Errors
    ///
    /// Returns a closed outcome once the session is closing, an
    /// invalid-argument outcome for a stamp from another stream, and the
    /// operation's terminal outcome when cancellation or the deadline wins.
    fn frame(&self, request: &FrameRequest, operation: &OperationContext) -> Result<Frame>;

    /// Closes the session and drains in-flight frame waits.
    ///
    /// Idempotent. Frames, views, and mappings the caller already holds stay
    /// valid: they are owned by the caller, not by the session.
    ///
    /// # Errors
    ///
    /// Returns the operation's terminal outcome when cancellation or the
    /// deadline wins before the drain finishes. The session then stays closing,
    /// and a later close continues.
    fn close(&self, operation: &OperationContext) -> Result<()>;

    /// Returns where the session is in its lifecycle.
    ///
    /// This is the one lifecycle answer an implementation gives, and the two
    /// questions below are derived from it. [`Lifecycle::Closing`] is an
    /// ordinary reachable state — a close whose operation is cancelled or
    /// already past its deadline leaves the session there — so a caller that
    /// asked only whether close had *finished* would keep handing work to a
    /// session that has stopped accepting it.
    fn lifecycle(&self) -> Lifecycle;

    /// Reports whether the session still accepts new work.
    fn is_open(&self) -> bool {
        self.lifecycle() == Lifecycle::Open
    }

    /// Reports whether the session has finished closing.
    fn is_closed(&self) -> bool {
        self.lifecycle() == Lifecycle::Closed
    }
}
