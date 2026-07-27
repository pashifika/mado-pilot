//! The replay capture provider and the sessions it opens.

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex};

use mado_pilot_capture::{
    CaptureFault, CaptureProvider, CaptureSession, CoordinateSupport, Frame, FrameRequest,
    FrameSelection, OpenRequest, Publication, SessionDescription, StreamState, TargetDescription,
};
use mado_pilot_core::{
    FrameOrder, IdentityIssuer, Operation, OperationContext, ProviderId, Result, TargetId,
};

use crate::source::{ReplayFrame, ReplaySource, ReplayTarget};

/// Provider name that qualifies every replay target identity.
pub const PROVIDER: ProviderId = ProviderId::new("replay");

/// A capture provider backed by a configured replay source.
///
/// Everything it can produce is fixed when it is constructed. It performs no
/// desktop enumeration, no permission probe, no host DPI lookup, and no network
/// access, which is what makes the same source behave identically on Windows and
/// macOS.
pub struct ReplayProvider {
    issuer: Arc<IdentityIssuer>,
    targets: Vec<(TargetId, ReplayTarget)>,
}

impl ReplayProvider {
    /// Builds a provider that serves `source`, issuing identities from `issuer`.
    ///
    /// # Errors
    ///
    /// Returns a capture failure when a target's identity cannot be issued.
    pub fn new(issuer: Arc<IdentityIssuer>, source: ReplaySource) -> Result<Self> {
        let mut targets = Vec::new();
        for target in source.into_targets() {
            targets.push((issuer.issue_target(PROVIDER)?, target));
        }
        Ok(Self { issuer, targets })
    }

    fn description(&self, id: TargetId, target: &ReplayTarget) -> TargetDescription {
        TargetDescription::new(
            id,
            target.name(),
            target.extent(),
            target.format(),
            coordinate_support(target),
        )
    }
}

impl fmt::Debug for ReplayProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReplayProvider")
            .field("engine", &self.issuer.engine())
            .field("targets", &self.targets.len())
            .finish()
    }
}

fn coordinate_support(target: &ReplayTarget) -> CoordinateSupport {
    if target.declares_placement() {
        CoordinateSupport::with_target_placement()
    } else {
        CoordinateSupport::frame_only()
    }
}

impl CaptureProvider for ReplayProvider {
    fn provider(&self) -> ProviderId {
        PROVIDER
    }

    fn discover(&self, operation: &OperationContext) -> Result<Vec<TargetDescription>> {
        let attempt = Operation::admit(operation)?;
        let descriptions = self
            .targets
            .iter()
            .map(|(id, target)| self.description(*id, target))
            .collect();
        // A partial list must never look like a complete one, so the whole list
        // is committed or none of it is.
        Ok(attempt.commit(descriptions)?)
    }

    fn open(
        &self,
        target: TargetId,
        request: &OpenRequest,
        operation: &OperationContext,
    ) -> Result<Arc<dyn CaptureSession>> {
        let attempt = Operation::admit(operation)?;

        target.check_engine(self.issuer.engine())?;
        if target.provider() != PROVIDER {
            return Err(CaptureFault::ForeignTarget.into());
        }
        let (_, source) = self
            .targets
            .iter()
            .find(|(id, _)| *id == target)
            .ok_or(CaptureFault::UnknownTarget)?;

        // A required format the source cannot produce fails the open rather
        // than converting behind the caller's back: a caller who required a
        // format wanted the source to have it, not for someone to translate.
        let format = source.format();
        if let Some(required) = request.required_format()
            && required != format
        {
            return Err(CaptureFault::UnsupportedOption.into());
        }

        let stream = self.issuer.issue_stream()?;
        let description = SessionDescription::new(
            target,
            stream,
            source.extent(),
            format,
            coordinate_support(source),
        );
        let session = ReplaySession {
            description,
            state: StreamState::new(stream),
            remaining: Mutex::new(source.clone().into_frames().into()),
        };
        session.advance()?;

        Ok(attempt.commit(Arc::new(session) as Arc<dyn CaptureSession>)?)
    }
}

/// One open replay session.
///
/// The session publishes its first frame at open, and publishes the next one
/// when a caller asks for something newer. That is what makes replay
/// deterministic: the sequence advances because a consumer consumed it, not
/// because wall-clock time passed.
struct ReplaySession {
    description: SessionDescription,
    state: StreamState,
    remaining: Mutex<VecDeque<ReplayFrame>>,
}

impl ReplaySession {
    /// Publishes the next frame of the sequence.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureFault::StreamEnded`] when the sequence is exhausted.
    /// Waiting instead would be honest only if a frame might still arrive, and
    /// for a finite replay sequence none ever will — so the caller is told,
    /// rather than left to discover it when the deadline expires.
    fn advance(&self) -> Result<Frame> {
        let next = {
            let mut remaining = self
                .remaining
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            remaining.pop_front()
        };
        let Some(frame) = next else {
            return Err(CaptureFault::StreamEnded.into());
        };
        let publication = Publication {
            captured_at: frame.captured_at(),
            descriptor: frame.descriptor(),
            placement: frame.placement(),
            continuity: frame.continuity(),
            pixels: frame.into_pixels(),
        };
        self.state.publish(publication)
    }
}

impl fmt::Debug for ReplaySession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReplaySession")
            .field("stream", &self.description.stream())
            .field("lifecycle", &self.state.lifecycle())
            .finish()
    }
}

impl CaptureSession for ReplaySession {
    fn description(&self) -> SessionDescription {
        self.description.clone()
    }

    fn frame(&self, request: &FrameRequest, operation: &OperationContext) -> Result<Frame> {
        if let FrameSelection::NewerThan(stamp) = request.selection() {
            let current = self.state.current();
            match current.as_ref().map(|frame| frame.stamp().order(&stamp)) {
                // A stamp from another stream is refused without touching the
                // sequence: advancing it would consume a frame to answer a
                // request that was never valid.
                Some(Err(_)) => return Err(CaptureFault::ForeignStream.into()),
                Some(Ok(FrameOrder::After)) => {}
                _ => {
                    self.advance()?;
                }
            }
        }
        self.state.frame(request, operation)
    }

    fn close(&self, operation: &OperationContext) -> Result<()> {
        self.state.drain(operation)
    }

    fn is_closed(&self) -> bool {
        self.state.lifecycle() == mado_pilot_capture::Lifecycle::Closed
    }
}
