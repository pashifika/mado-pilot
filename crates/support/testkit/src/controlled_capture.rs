//! A capture adapter a test drives by hand.
//!
//! This exists so the capture contract has two behaviorally different
//! implementations rather than one. The replay adapter advances its sequence
//! synchronously when a consumer asks for a newer frame; this one publishes only
//! when the test says so, from whatever thread the test chooses. The waiting,
//! cancellation, and close-under-a-waiter paths are reachable here and are not
//! reachable through replay, so a contract that only replay satisfied would be a
//! description of replay.

use std::fmt;
use std::sync::{Arc, Mutex};

use mado_pilot_capture::{
    CaptureFault, CaptureProvider, CaptureSession, Continuity, CoordinateSupport, Frame,
    FrameDescriptor, FrameRequest, Lifecycle, OpenRequest, PixelFormat, Publication,
    SessionDescription, StreamState, TargetDescription,
};
use mado_pilot_core::{
    IdentityIssuer, MonotonicInstant, OperationContext, PixelExtent, ProviderId, Result, TargetId,
};

/// Provider name qualifying this double's target identities.
pub const PROVIDER: ProviderId = ProviderId::new("controlled");

/// A capture provider whose publication a test controls directly.
pub struct ControlledCapture {
    issuer: Arc<IdentityIssuer>,
    target: TargetId,
    descriptor: FrameDescriptor,
    sessions: Mutex<Vec<Arc<ControlledSession>>>,
}

impl ControlledCapture {
    /// Builds a provider offering one target of `extent` in `format`.
    ///
    /// # Errors
    ///
    /// Returns a capture fault for an extent and format that do not form a valid
    /// descriptor, and an identity fault when the target identity cannot be
    /// issued.
    pub fn new(
        issuer: Arc<IdentityIssuer>,
        extent: PixelExtent,
        format: PixelFormat,
    ) -> Result<Self> {
        let target = issuer.issue_target(PROVIDER)?;
        Ok(Self {
            issuer,
            target,
            descriptor: FrameDescriptor::packed(extent, format)?,
            sessions: Mutex::new(Vec::new()),
        })
    }

    /// Publishes one frame of solid `fill` to every open session.
    ///
    /// # Errors
    ///
    /// Returns whatever the stream state returns, so a test can assert that
    /// publishing to a closing session is refused.
    pub fn publish(&self, fill: u8, continuity: Continuity) -> Result<()> {
        let sessions: Vec<Arc<ControlledSession>> = self.sessions().clone();
        for session in sessions {
            session.state.publish(Publication {
                captured_at: MonotonicInstant::ORIGIN,
                descriptor: self.descriptor,
                placement: None,
                continuity,
                pixels: vec![fill; self.descriptor.byte_len()].into_boxed_slice(),
            })?;
        }
        Ok(())
    }

    /// Publishes a frame of a different extent, forcing a discontinuity.
    ///
    /// # Errors
    ///
    /// As [`ControlledCapture::publish`].
    pub fn publish_reshaped(&self, extent: PixelExtent, fill: u8) -> Result<()> {
        let descriptor = FrameDescriptor::packed(extent, self.descriptor.format())?;
        let sessions: Vec<Arc<ControlledSession>> = self.sessions().clone();
        for session in sessions {
            session.state.publish(Publication {
                captured_at: MonotonicInstant::ORIGIN,
                descriptor,
                placement: None,
                continuity: Continuity::Discontinuous,
                pixels: vec![fill; descriptor.byte_len()].into_boxed_slice(),
            })?;
        }
        Ok(())
    }

    fn sessions(&self) -> std::sync::MutexGuard<'_, Vec<Arc<ControlledSession>>> {
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl fmt::Debug for ControlledCapture {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ControlledCapture")
            .field("engine", &self.issuer.engine())
            .field("descriptor", &self.descriptor)
            .finish()
    }
}

impl CaptureProvider for ControlledCapture {
    fn provider(&self) -> ProviderId {
        PROVIDER
    }

    fn discover(&self, _operation: &OperationContext) -> Result<Vec<TargetDescription>> {
        Ok(vec![TargetDescription::new(
            self.target,
            "controlled",
            self.descriptor.extent(),
            self.descriptor.format(),
            CoordinateSupport::frame_only(),
        )])
    }

    fn open(
        &self,
        target: TargetId,
        request: &OpenRequest,
        _operation: &OperationContext,
    ) -> Result<Arc<dyn CaptureSession>> {
        target.check_engine(self.issuer.engine())?;
        if target != self.target {
            return Err(CaptureFault::UnknownTarget.into());
        }
        if let Some(required) = request.required_format()
            && required != self.descriptor.format()
        {
            return Err(CaptureFault::UnsupportedOption.into());
        }

        let stream = self.issuer.issue_stream()?;
        let session = Arc::new(ControlledSession {
            description: SessionDescription::new(
                target,
                stream,
                self.descriptor.extent(),
                self.descriptor.format(),
                CoordinateSupport::frame_only(),
            ),
            state: StreamState::new(stream),
        });
        self.sessions().push(Arc::clone(&session));
        Ok(session as Arc<dyn CaptureSession>)
    }
}

struct ControlledSession {
    description: SessionDescription,
    state: StreamState,
}

impl fmt::Debug for ControlledSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ControlledSession")
            .field("stream", &self.description.stream())
            .field("lifecycle", &self.state.lifecycle())
            .finish()
    }
}

impl CaptureSession for ControlledSession {
    fn description(&self) -> SessionDescription {
        self.description.clone()
    }

    fn frame(&self, request: &FrameRequest, operation: &OperationContext) -> Result<Frame> {
        self.state.frame(request, operation)
    }

    fn close(&self, operation: &OperationContext) -> Result<()> {
        self.state.drain(operation)
    }

    fn lifecycle(&self) -> Lifecycle {
        self.state.lifecycle()
    }
}
