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
use std::collections::VecDeque;
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
use crate::controlled_ocr::CompletionGate;

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

/// One deterministic backend response, optionally held at a completion gate.
#[derive(Debug, Clone)]
pub struct ScriptedMatchCall {
    candidates: Vec<Candidate>,
    behavior: Behavior,
    gate: Option<Arc<CompletionGate>>,
}

impl ScriptedMatchCall {
    /// Returns one successful scripted candidate set.
    #[must_use]
    pub fn new(candidates: Vec<Candidate>) -> Self {
        Self {
            candidates,
            behavior: Behavior::Succeed,
            gate: None,
        }
    }

    /// Replaces the scripted backend behavior.
    #[must_use]
    pub const fn with_behavior(mut self, behavior: Behavior) -> Self {
        self.behavior = behavior;
        self
    }

    /// Holds this call after backend admission until `gate` is released.
    #[must_use]
    pub fn with_completion_gate(mut self, gate: Arc<CompletionGate>) -> Self {
        self.gate = Some(gate);
        self
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
    calls: VecDeque<ScriptedMatchCall>,
}

fn observe_mapped_bytes(observed: &AtomicUsize, bytes: usize) {
    match observed.compare_exchange(0, bytes, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => {}
        Err(previous) if previous == bytes => {}
        Err(_) => observed.store(usize::MAX, Ordering::Release),
    }
}

fn consistent_mapped_bytes(observed: &AtomicUsize) -> Option<usize> {
    match observed.load(Ordering::Acquire) {
        0 | usize::MAX => None,
        bytes => Some(bytes),
    }
}

/// A backend whose every answer a test chooses.
pub struct ControlledMatcher {
    format: PixelFormat,
    clock: Option<Arc<ManualClock>>,
    cancel_during_find: Option<CancellationToken>,
    completion_gate: Option<Arc<CompletionGate>>,
    script: Mutex<Script>,
    prepared: AtomicUsize,
    searches: AtomicUsize,
    completed: AtomicUsize,
    mapped_bytes: AtomicUsize,
}

impl ControlledMatcher {
    /// Returns a matcher that prepares successfully and finds nothing.
    #[must_use]
    pub fn new(format: PixelFormat) -> Self {
        Self {
            format,
            clock: None,
            cancel_during_find: None,
            completion_gate: None,
            script: Mutex::new(Script::default()),
            prepared: AtomicUsize::new(0),
            searches: AtomicUsize::new(0),
            completed: AtomicUsize::new(0),
            mapped_bytes: AtomicUsize::new(0),
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

    /// Scripts successive searches before falling back to the default response.
    #[must_use]
    pub fn with_calls(self, calls: impl IntoIterator<Item = ScriptedMatchCall>) -> Self {
        self.script().calls = calls.into_iter().collect();
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

    /// Blocks backend completion at `gate` after admission.
    ///
    /// The gate deliberately models an uninterruptible backend call. Tests may
    /// cancel, close, or supersede authority while it is blocked, then release
    /// it and prove the late result cannot commit.
    #[must_use]
    pub fn with_completion_gate(mut self, gate: Arc<CompletionGate>) -> Self {
        self.completion_gate = Some(gate);
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

    /// Returns how many searches completed successfully.
    #[must_use]
    pub fn completion_count(&self) -> usize {
        self.completed.load(Ordering::Acquire)
    }

    /// Returns the byte length observed on every backend request.
    ///
    /// `None` means no search ran or different request lengths were observed.
    #[must_use]
    pub fn consistent_mapped_bytes(&self) -> Option<usize> {
        consistent_mapped_bytes(&self.mapped_bytes)
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
            .field("mapped_bytes", &self.consistent_mapped_bytes())
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
        observe_mapped_bytes(&self.mapped_bytes, request.pixels.bytes().len());

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

        let (latency, behavior, candidates, gate) = {
            let mut script = self.script();
            match script.calls.pop_front() {
                Some(call) => (script.latency, call.behavior, call.candidates, call.gate),
                None => (
                    script.latency,
                    script.find,
                    script.candidates.clone(),
                    self.completion_gate.clone(),
                ),
            }
        };
        if let Some(gate) = &gate {
            gate.enter_and_wait();
        }

        if let Some(clock) = &self.clock {
            clock.advance(latency);
        }
        if let Some(token) = &self.cancel_during_find {
            token.cancel();
        }

        let result = behavior.apply().map(|()| candidates);
        let _ = operation;
        if result.is_ok() {
            self.completed.fetch_add(1, Ordering::AcqRel);
        }
        if let Some(gate) = &gate {
            gate.complete();
        }
        result
    }
}

/// A transparent backend wrapper that observes actual search work.
pub struct ObservedMatcher {
    backend: Arc<dyn MatchBackend>,
    searches: AtomicUsize,
    completed: AtomicUsize,
    mapped_bytes: AtomicUsize,
}

impl ObservedMatcher {
    /// Wraps one backend without changing its public descriptor or results.
    #[must_use]
    pub fn new(backend: Arc<dyn MatchBackend>) -> Self {
        Self {
            backend,
            searches: AtomicUsize::new(0),
            completed: AtomicUsize::new(0),
            mapped_bytes: AtomicUsize::new(0),
        }
    }

    /// Returns how many searches reached the wrapped backend.
    #[must_use]
    pub fn find_count(&self) -> usize {
        self.searches.load(Ordering::Acquire)
    }

    /// Returns how many wrapped searches completed successfully.
    #[must_use]
    pub fn completion_count(&self) -> usize {
        self.completed.load(Ordering::Acquire)
    }

    /// Returns the byte length observed on every wrapped backend request.
    ///
    /// `None` means no search ran or different request lengths were observed.
    #[must_use]
    pub fn consistent_mapped_bytes(&self) -> Option<usize> {
        consistent_mapped_bytes(&self.mapped_bytes)
    }
}

impl fmt::Debug for ObservedMatcher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObservedMatcher")
            .field("descriptor", &self.backend.descriptor())
            .field("searches", &self.find_count())
            .field("completed", &self.completion_count())
            .field("mapped_bytes", &self.consistent_mapped_bytes())
            .finish()
    }
}

impl MatchBackend for ObservedMatcher {
    fn descriptor(&self) -> BackendDescriptor {
        self.backend.descriptor()
    }

    fn prepare(
        &self,
        source: &TemplateSource,
        operation: &OperationContext,
    ) -> Result<PreparedTemplate> {
        self.backend.prepare(source, operation)
    }

    fn find(
        &self,
        request: &BackendRequest<'_>,
        operation: &OperationContext,
    ) -> Result<Vec<Candidate>> {
        self.searches.fetch_add(1, Ordering::AcqRel);
        observe_mapped_bytes(&self.mapped_bytes, request.pixels.bytes().len());
        let result = self.backend.find(request, operation);
        if result.is_ok() {
            self.completed.fetch_add(1, Ordering::AcqRel);
        }
        result
    }
}
