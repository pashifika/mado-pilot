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
    IdentityIssuer, MonotonicInstant, OperationContext, PixelExtent, ProviderId, Result,
    TargetCapability, TargetId,
};

use crate::controlled_storage::ControlledProducer;

/// Provider name qualifying this double's target identities.
pub const PROVIDER: ProviderId = ProviderId::new("controlled");

/// One target this double offers, and whether it still exists.
#[derive(Debug, Clone)]
struct ControlledTarget {
    id: TargetId,
    name: String,
    capability: TargetCapability,
    present: bool,
}

/// A capture provider whose publication a test controls directly.
pub struct ControlledCapture {
    issuer: Arc<IdentityIssuer>,
    target: TargetId,
    descriptor: FrameDescriptor,
    targets: Mutex<Vec<ControlledTarget>>,
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
            targets: Mutex::new(vec![ControlledTarget {
                id: target,
                name: "controlled".to_owned(),
                capability: TargetCapability::unclassified(),
                present: true,
            }]),
            sessions: Mutex::new(Vec::new()),
        })
    }

    /// Returns the identity of the target this provider was built with.
    #[must_use]
    pub const fn target(&self) -> TargetId {
        self.target
    }

    /// Declares what this provider can do with its first target.
    ///
    /// A test that needs a classified window or a target that accepts input says so
    /// here; the default is the unclassified, capture-only target a source of
    /// prepared frames reports.
    pub fn declare(&self, capability: TargetCapability) {
        let target = self.target;
        let mut targets = self.targets();
        if let Some(entry) = targets.iter_mut().find(|entry| entry.id == target) {
            entry.capability = capability;
        }
    }

    /// Adds another target, so discovery and filtering have something to select
    /// between.
    ///
    /// # Errors
    ///
    /// Returns an identity fault when the target identity cannot be issued.
    pub fn add_target(
        &self,
        name: impl Into<String>,
        capability: TargetCapability,
    ) -> Result<TargetId> {
        let id = self.issuer.issue_target(PROVIDER)?;
        self.targets().push(ControlledTarget {
            id,
            name: name.into(),
            capability,
            present: true,
        });
        Ok(id)
    }

    /// Loses `target`, as a window that closes does.
    ///
    /// The identity stays known and stays refused: a lost target is reported as
    /// lost rather than as one this provider never issued, and every session open
    /// on it ends with the same typed reason.
    pub fn lose(&self, target: TargetId) {
        {
            let mut targets = self.targets();
            if let Some(entry) = targets.iter_mut().find(|entry| entry.id == target) {
                entry.present = false;
            }
        }
        let sessions: Vec<Arc<ControlledSession>> = self.sessions().clone();
        for session in sessions {
            if session.description.target() == target {
                session.state.terminate(CaptureFault::TargetLost);
            }
        }
        self.sessions()
            .retain(|session| session.state.lifecycle() == Lifecycle::Open);
    }

    /// Loses `target` and offers a new one with the same descriptive metadata.
    ///
    /// This is the case a provider must not paper over: the title, the process, and
    /// the native handle can all match while the target is a different one, and the
    /// only thing that says otherwise is the identity.
    ///
    /// # Errors
    ///
    /// Returns an identity fault when the replacement identity cannot be issued.
    pub fn replace(&self, target: TargetId) -> Result<TargetId> {
        let existing = self
            .targets()
            .iter()
            .find(|entry| entry.id == target)
            .cloned();
        self.lose(target);
        let (name, capability) = existing.map_or_else(
            || ("controlled".to_owned(), TargetCapability::unclassified()),
            |entry| (entry.name, entry.capability),
        );
        self.add_target(name, capability)
    }

    /// Publishes one frame through opaque storage from `producer`.
    ///
    /// The frames every other publication here produces are CPU bytes, which no
    /// mapping ever has to convert. This one has the ownership and conversion
    /// behavior a native frame has.
    ///
    /// # Errors
    ///
    /// Returns whatever the producer or the stream returns, so a test can assert
    /// both an exhausted storage budget and a refused publication.
    pub fn publish_from(&self, producer: &ControlledProducer, fill: u8) -> Result<()> {
        let mut refusal = None;
        let sessions: Vec<Arc<ControlledSession>> = self.sessions().clone();
        for session in sessions {
            let publication = producer.publication(fill, Continuity::Continuous)?;
            if let Err(refused) = session.state.publish_storage(publication)
                && refusal.is_none()
            {
                refusal = Some(refused.into_error());
            }
        }
        self.sessions()
            .retain(|session| session.state.lifecycle() == Lifecycle::Open);

        match refusal {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Ends every open session with `fault`, as a device reset or a revoked
    /// authorization does.
    pub fn terminate(&self, fault: CaptureFault) {
        let sessions: Vec<Arc<ControlledSession>> = self.sessions().clone();
        for session in sessions {
            session.state.terminate(fault);
        }
        self.sessions()
            .retain(|session| session.state.lifecycle() == Lifecycle::Open);
    }

    /// Publishes one frame of solid `fill` to every open session.
    ///
    /// # Errors
    ///
    /// Returns whatever the stream state returns, so a test can assert that
    /// publishing to a closing session is refused.
    pub fn publish(&self, fill: u8, continuity: Continuity) -> Result<()> {
        let descriptor = self.descriptor;
        self.deliver(|| Publication {
            captured_at: MonotonicInstant::ORIGIN,
            descriptor,
            placement: None,
            continuity,
            pixels: vec![fill; descriptor.byte_len()].into_boxed_slice(),
        })
    }

    /// Publishes a frame of a different extent, forcing a discontinuity.
    ///
    /// # Errors
    ///
    /// As [`ControlledCapture::publish`].
    pub fn publish_reshaped(&self, extent: PixelExtent, fill: u8) -> Result<()> {
        let descriptor = FrameDescriptor::packed(extent, self.descriptor.format())?;
        self.deliver(|| Publication {
            captured_at: MonotonicInstant::ORIGIN,
            descriptor,
            placement: None,
            continuity: Continuity::Discontinuous,
            pixels: vec![fill; descriptor.byte_len()].into_boxed_slice(),
        })
    }

    /// Offers one publication to every session, and forgets the closed ones.
    ///
    /// Two rules, both of which a test with more than one session depends on. A
    /// refusal does not end the round: every other session still receives the
    /// frame, and the first refusal is reported once they all have. And a
    /// session that has begun closing is dropped from the list afterwards,
    /// because it will never accept another frame — keeping it would make the
    /// first close fail every later publication.
    ///
    /// `publication` is called once per session because a [`Publication`] owns
    /// its pixels and cannot be handed to two streams.
    fn deliver(&self, publication: impl Fn() -> Publication) -> Result<()> {
        // The list is copied and the guard released before publishing, so no
        // session's own lock is ever taken while the provider's is held.
        let sessions: Vec<Arc<ControlledSession>> = self.sessions().clone();
        let mut refusal = None;
        for session in sessions {
            if let Err(error) = session.state.publish(publication())
                && refusal.is_none()
            {
                refusal = Some(error);
            }
        }
        self.sessions()
            .retain(|session| session.state.lifecycle() == Lifecycle::Open);

        match refusal {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn sessions(&self) -> std::sync::MutexGuard<'_, Vec<Arc<ControlledSession>>> {
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn targets(&self) -> std::sync::MutexGuard<'_, Vec<ControlledTarget>> {
        self.targets
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
        // Deterministically ordered, and lost targets are gone from the list: a
        // provider lists what it can capture now.
        Ok(self
            .targets()
            .iter()
            .filter(|entry| entry.present)
            .map(|entry| {
                TargetDescription::new(
                    entry.id,
                    entry.name.clone(),
                    self.descriptor.extent(),
                    self.descriptor.format(),
                    CoordinateSupport::frame_only(),
                )
                .with_capability(entry.capability)
            })
            .collect())
    }

    fn open(
        &self,
        target: TargetId,
        request: &OpenRequest,
        _operation: &OperationContext,
    ) -> Result<Arc<dyn CaptureSession>> {
        self.accepts_target(target, self.issuer.engine())?;
        match self
            .targets()
            .iter()
            .find(|entry| entry.id == target)
            .map(|entry| entry.present)
        {
            Some(true) => {}
            // A target this provider issued and can no longer capture is lost, not
            // unknown. The distinction is what stops a caller from retrying a
            // window that has closed as though it had mistyped an identity.
            Some(false) => return Err(CaptureFault::TargetLost.into()),
            None => return Err(CaptureFault::UnknownTarget.into()),
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
