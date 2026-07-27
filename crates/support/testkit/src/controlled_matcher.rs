//! A matching backend a test scripts completely.
//!
//! The vision seam is only real if two behaviourally different backends satisfy
//! it. OpenCV is one; this is the other, and it exists to reach the states a
//! real backend reaches rarely and never on demand — a backend that is not
//! available, one that fails after accepting the request, one that takes long
//! enough for a deadline to pass, and one that returns candidates a test chose
//! precisely so the ordering and suppression rules can be checked against a
//! known answer.
//!
//! It produces candidates from a script rather than from the pixels. That is
//! the point: a double that looked at the image would be a second matching
//! implementation to debug.

use std::any::Any;
use std::fmt;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use mado_pilot_capture::PixelFormat;
use mado_pilot_core::{CancellationToken, Error, OperationContext, Result, Status};
use mado_pilot_vision::{
    BackendDescriptor, BackendId, BackendRequest, Candidate, MatchBackend, PreparedTemplate,
    TemplatePayload, TemplateSource, VisionFault,
};

use crate::clock::ManualClock;

/// The backend identity the controlled matcher publishes.
pub const CONTROLLED_BACKEND: &str = "controlled";

/// How the controlled matcher responds to one kind of call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Behavior {
    /// Do the scripted thing.
    #[default]
    Succeed,
    /// Report that the backend could not be loaded or initialized.
    Unavailable,
    /// Report a failure after having accepted the request.
    Fail,
}

impl Behavior {
    fn apply(self) -> Result<()> {
        match self {
            Behavior::Succeed => Ok(()),
            Behavior::Unavailable => Err(VisionFault::BackendUnavailable.into()),
            Behavior::Fail => Err(VisionFault::BackendFailed.into()),
        }
    }
}

/// The compiled state this backend pretends to produce.
///
/// It carries nothing: what matters is that only this backend can downcast to
/// it, which is what proves the matcher routed the payload correctly. How many
/// were produced is counted separately, by `prepare_count`.
#[derive(Debug)]
struct ControlledPayload;

impl TemplatePayload for ControlledPayload {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug, Default)]
struct Script {
    candidates: Vec<Candidate>,
    latency: Duration,
    prepare: Behavior,
    find: Behavior,
}

/// A backend whose every answer a test chooses.
pub struct ControlledMatcher {
    format: PixelFormat,
    clock: Option<Arc<ManualClock>>,
    cancel_during_find: Option<CancellationToken>,
    script: Mutex<Script>,
    prepared: AtomicUsize,
    searches: AtomicUsize,
}

impl ControlledMatcher {
    /// Returns a matcher that prepares successfully and finds nothing.
    #[must_use]
    pub fn new(format: PixelFormat) -> Self {
        Self {
            format,
            clock: None,
            cancel_during_find: None,
            script: Mutex::new(Script::default()),
            prepared: AtomicUsize::new(0),
            searches: AtomicUsize::new(0),
        }
    }

    /// Scripts the candidates every search returns.
    ///
    /// They are returned exactly as given, in the order given, with whatever
    /// scores they carry. Handing back a deliberately unordered or
    /// out-of-range set is how the public rules get tested.
    #[must_use]
    pub fn with_candidates(self, candidates: Vec<Candidate>) -> Self {
        self.script().candidates = candidates;
        self
    }

    /// Makes every search advance `clock` by `latency` before it returns.
    ///
    /// A deadline can then expire during backend execution at an exact,
    /// repeatable point.
    #[must_use]
    pub fn with_latency(mut self, clock: Arc<ManualClock>, latency: Duration) -> Self {
        self.script().latency = latency;
        self.clock = Some(clock);
        self
    }

    /// Sets how preparation responds.
    #[must_use]
    pub fn preparing(self, behavior: Behavior) -> Self {
        self.script().prepare = behavior;
        self
    }

    /// Sets how searching responds.
    #[must_use]
    pub fn finding(self, behavior: Behavior) -> Self {
        self.script().find = behavior;
        self
    }

    /// Cancels `token` from inside every search, before returning candidates.
    ///
    /// This is the uninterruptible-backend case: the call runs to the end and
    /// produces a perfectly good answer that the operation contract must then
    /// refuse to publish.
    #[must_use]
    pub fn cancelling(mut self, token: CancellationToken) -> Self {
        self.cancel_during_find = Some(token);
        self
    }

    /// Returns how many templates have been prepared.
    #[must_use]
    pub fn prepare_count(&self) -> usize {
        self.prepared.load(Ordering::Acquire)
    }

    /// Returns how many searches have reached the backend.
    ///
    /// A test asserts on this to prove that a rule rejected a request *before*
    /// any backend work happened, which a returned error alone cannot show.
    #[must_use]
    pub fn find_count(&self) -> usize {
        self.searches.load(Ordering::Acquire)
    }

    fn script(&self) -> std::sync::MutexGuard<'_, Script> {
        self.script
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl fmt::Debug for ControlledMatcher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ControlledMatcher")
            .field("format", &self.format)
            .field("prepared", &self.prepare_count())
            .field("searches", &self.find_count())
            .finish_non_exhaustive()
    }
}

impl MatchBackend for ControlledMatcher {
    fn descriptor(&self) -> BackendDescriptor {
        BackendDescriptor::new(CONTROLLED_BACKEND, "1", self.format)
    }

    fn prepare(
        &self,
        source: &TemplateSource,
        operation: &OperationContext,
    ) -> Result<PreparedTemplate> {
        if let Some(interruption) = operation.interruption() {
            return Err(interruption.into());
        }
        self.script().prepare.apply()?;
        self.prepared.fetch_add(1, Ordering::AcqRel);
        Ok(PreparedTemplate::new(
            BackendId::new(CONTROLLED_BACKEND),
            source,
            Arc::new(ControlledPayload),
        ))
    }

    fn find(
        &self,
        request: &BackendRequest<'_>,
        operation: &OperationContext,
    ) -> Result<Vec<Candidate>> {
        self.searches.fetch_add(1, Ordering::AcqRel);

        // The payload must be the one this backend produced. Reaching a foreign
        // payload would mean the matcher's identity check did not run.
        if request
            .template
            .payload()
            .as_any()
            .downcast_ref::<ControlledPayload>()
            .is_none()
        {
            return Err(Error::new(
                Status::Internal,
                "controlled matcher received a payload it did not prepare",
            ));
        }

        let (latency, behavior, candidates) = {
            let script = self.script();
            (script.latency, script.find, script.candidates.clone())
        };

        if let Some(clock) = &self.clock {
            clock.advance(latency);
        }
        if let Some(token) = &self.cancel_during_find {
            token.cancel();
        }

        behavior.apply()?;
        let _ = operation;
        Ok(candidates)
    }
}
