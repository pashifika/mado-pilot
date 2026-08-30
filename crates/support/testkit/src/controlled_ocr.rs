//! A deterministic OCR backend for contract and race tests.

use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use mado_pilot_capture::PixelFormat;
use mado_pilot_core::{CancellationToken, OperationContext, PixelRect, Result};
use mado_pilot_ocr::{
    ACCEPTED_G004_NORMALIZATION_ID, BackendCandidate, BackendId, BackendRequest, BackendVersion,
    DecoderId, LanguageProfileId, ModelComponentIdentity, ModelId, ModelVersion, NormalizationId,
    OcrBackend, OcrBackendDescriptor, OcrBackendIdentity, OcrCandidateSink, OcrFault,
    OcrModelIdentity, OcrProfileMetadata, PreprocessingId, ProfileId,
    candidate_interest_membership,
};

use crate::clock::ManualClock;

/// Backend identity published by [`ControlledOcr`].
pub const CONTROLLED_OCR_BACKEND: &str = "controlled-ocr";
/// Model identity published by [`ControlledOcr`].
pub const CONTROLLED_OCR_MODEL: &str = "controlled-ocr-model";
/// Profile identity published by [`ControlledOcr`].
pub const CONTROLLED_OCR_PROFILE: &str = "controlled-ocr-profile";

/// How a controlled OCR call responds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OcrBehavior {
    /// Return the scripted value.
    #[default]
    Succeed,
    /// Report an unavailable backend.
    Unavailable,
    /// Report failure after accepting work.
    Fail,
}

impl OcrBehavior {
    fn apply(self) -> Result<()> {
        match self {
            Self::Succeed => Ok(()),
            Self::Unavailable => Err(OcrFault::BackendUnavailable.into()),
            Self::Fail => Err(OcrFault::BackendFailed.into()),
        }
    }
}

#[derive(Debug, Default)]
struct GateState {
    entered: bool,
    released: bool,
    completed: bool,
}

/// A one-call completion gate for deterministic late and out-of-order tests.
#[derive(Debug, Default)]
pub struct CompletionGate {
    state: Mutex<GateState>,
    changed: Condvar,
}

impl CompletionGate {
    /// Builds a closed gate.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Waits until the backend has entered the gate.
    ///
    /// Returns `false` on timeout so a broken test fails instead of hanging.
    #[must_use]
    pub fn wait_until_entered(&self, timeout: Duration) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (state, _) = self
            .changed
            .wait_timeout_while(state, timeout, |state| !state.entered)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.entered
    }

    /// Waits until the released backend call reports completion.
    ///
    /// Returns `false` on timeout so a broken test fails instead of hanging.
    #[must_use]
    pub fn wait_until_completed(&self, timeout: Duration) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (state, _) = self
            .changed
            .wait_timeout_while(state, timeout, |state| !state.completed)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.completed
    }

    /// Releases the blocked backend call. Idempotent.
    pub fn release(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.released = true;
        self.changed.notify_all();
    }

    /// Returns a scope guard that releases this gate when dropped.
    ///
    /// A controller keeps this guard alive while it performs assertions around a
    /// blocked backend call. Panic unwinding then opens the gate before owners
    /// join backend worker threads, preventing a failed test from deadlocking
    /// during teardown. Explicit [`Self::release`] remains idempotent.
    #[must_use = "keep the guard alive for every scope that can leave the gate closed"]
    pub fn release_guard(self: &Arc<Self>) -> CompletionGateReleaseGuard {
        CompletionGateReleaseGuard {
            gate: Arc::clone(self),
        }
    }

    pub(crate) fn enter_and_wait(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.entered = true;
        self.changed.notify_all();
        while !state.released {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }
    pub(crate) fn complete(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.completed = true;
        self.changed.notify_all();
    }
}

/// Releases one [`CompletionGate`] when controller scope exits.
#[derive(Debug)]
pub struct CompletionGateReleaseGuard {
    gate: Arc<CompletionGate>,
}

impl Drop for CompletionGateReleaseGuard {
    fn drop(&mut self) {
        self.gate.release();
    }
}

struct GateCompletion<'gate>(Option<&'gate CompletionGate>);

impl Drop for GateCompletion<'_> {
    fn drop(&mut self) {
        if let Some(gate) = self.0 {
            gate.complete();
        }
    }
}

/// One owned candidate a controlled call can later lend to the OCR sink.
#[derive(Clone, PartialEq)]
pub struct ScriptedOcrCandidate {
    text: Arc<[u8]>,
    quadrilateral: [(f64, f64); 4],
    confidence: f64,
    detector_order: u32,
}

impl ScriptedOcrCandidate {
    /// Builds a scripted candidate, including deliberately malformed values.
    #[must_use]
    pub fn new(
        text: impl Into<Arc<[u8]>>,
        quadrilateral: [(f64, f64); 4],
        confidence: f64,
        detector_order: u32,
    ) -> Self {
        Self {
            text: text.into(),
            quadrilateral,
            confidence,
            detector_order,
        }
    }

    fn borrowed(&self) -> BackendCandidate<'_> {
        BackendCandidate::new(
            &self.text,
            self.quadrilateral,
            self.confidence,
            self.detector_order,
        )
    }
}

impl fmt::Debug for ScriptedOcrCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScriptedOcrCandidate")
            .field("text_bytes", &self.text.len())
            .field("quadrilateral", &self.quadrilateral)
            .field("confidence", &self.confidence)
            .field("detector_order", &self.detector_order)
            .finish()
    }
}

/// One FIFO-scripted OCR call.
#[derive(Clone)]
pub struct ScriptedOcrCall {
    candidates: Vec<ScriptedOcrCandidate>,
    latency: Duration,
    behavior: OcrBehavior,
    gate: Option<Arc<CompletionGate>>,
}

impl ScriptedOcrCall {
    /// Builds a successful call returning `candidates`.
    #[must_use]
    pub fn new(candidates: Vec<ScriptedOcrCandidate>) -> Self {
        Self {
            candidates,
            latency: Duration::ZERO,
            behavior: OcrBehavior::Succeed,
            gate: None,
        }
    }

    /// Advances a controlled clock by `latency`.
    #[must_use]
    pub const fn with_latency(mut self, latency: Duration) -> Self {
        self.latency = latency;
        self
    }

    /// Sets the call outcome.
    #[must_use]
    pub const fn with_behavior(mut self, behavior: OcrBehavior) -> Self {
        self.behavior = behavior;
        self
    }

    /// Blocks the call at `gate`.
    #[must_use]
    pub fn with_completion_gate(mut self, gate: Arc<CompletionGate>) -> Self {
        self.gate = Some(gate);
        self
    }
}

impl fmt::Debug for ScriptedOcrCall {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScriptedOcrCall")
            .field("candidates", &self.candidates.len())
            .field("latency", &self.latency)
            .field("behavior", &self.behavior)
            .field("gated", &self.gate.is_some())
            .finish()
    }
}

#[derive(Debug)]
struct Script {
    default_call: ScriptedOcrCall,
    calls: VecDeque<ScriptedOcrCall>,
    close: OcrBehavior,
}

impl Default for Script {
    fn default() -> Self {
        Self {
            default_call: ScriptedOcrCall::new(Vec::new()),
            calls: VecDeque::new(),
            close: OcrBehavior::Succeed,
        }
    }
}

#[derive(Debug, Default)]
struct LatestCandidateCounts {
    generation: usize,
    selected: usize,
    ignored: usize,
}

struct LatestCandidateCountsGuard<'a> {
    generation: usize,
    selected: usize,
    ignored: usize,
    latest: &'a Mutex<LatestCandidateCounts>,
}

impl<'a> LatestCandidateCountsGuard<'a> {
    fn new(latest: &'a Mutex<LatestCandidateCounts>) -> Self {
        let mut observation = latest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let generation = observation
            .generation
            .checked_add(1)
            .expect("controlled OCR call generation exhausted");
        *observation = LatestCandidateCounts {
            generation,
            selected: 0,
            ignored: 0,
        };
        Self {
            generation,
            selected: 0,
            ignored: 0,
            latest,
        }
    }
}

impl Drop for LatestCandidateCountsGuard<'_> {
    fn drop(&mut self) {
        let mut observation = self
            .latest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if observation.generation == self.generation {
            observation.selected = self.selected;
            observation.ignored = self.ignored;
        }
    }
}

/// A backend whose candidates, latency, interruption, failures, and completion are scripted.
pub struct ControlledOcr {
    descriptor: OcrBackendDescriptor,
    clock: Option<Arc<ManualClock>>,
    cancel_during_recognize: Option<CancellationToken>,
    cancel_after_candidates: Option<(CancellationToken, usize)>,
    cancel_after_output: Option<CancellationToken>,
    cancel_during_close: Option<CancellationToken>,
    script: Mutex<Script>,
    honor_interests: bool,
    recognitions: AtomicUsize,
    closes: AtomicUsize,
    last_max_candidates: AtomicUsize,
    last_max_text_bytes: AtomicUsize,
    last_region: Mutex<Option<PixelRect>>,
    last_interests: Mutex<Option<Vec<PixelRect>>>,
    last_candidate_counts: Mutex<LatestCandidateCounts>,
}

impl ControlledOcr {
    /// Returns a controlled backend that recognizes nothing successfully.
    #[must_use]
    pub fn new(format: PixelFormat) -> Self {
        let model = OcrModelIdentity::new(
            ModelId::new(CONTROLLED_OCR_MODEL).expect("constant model identity"),
            ModelVersion::new("1").expect("constant model version"),
            ProfileId::new(CONTROLLED_OCR_PROFILE).expect("constant profile identity"),
            ModelComponentIdentity::new(1, [1; 32]).expect("constant detector identity"),
            ModelComponentIdentity::new(1, [2; 32]).expect("constant recognizer identity"),
            OcrProfileMetadata::new(
                LanguageProfileId::new("controlled-language").expect("constant language"),
                PreprocessingId::new("controlled-preprocessing").expect("constant preprocessing"),
                DecoderId::new("controlled-decoder").expect("constant decoder"),
                NormalizationId::new(ACCEPTED_G004_NORMALIZATION_ID)
                    .expect("constant normalization"),
                1,
                [3; 32],
            )
            .expect("constant profile metadata"),
        )
        .expect("controlled model identity");
        Self {
            descriptor: OcrBackendDescriptor::new(
                OcrBackendIdentity::new(
                    BackendId::new(CONTROLLED_OCR_BACKEND).expect("constant backend identity"),
                    BackendVersion::new("1").expect("constant backend version"),
                ),
                model,
                format,
            ),
            clock: None,
            cancel_during_close: None,
            cancel_during_recognize: None,
            cancel_after_candidates: None,
            cancel_after_output: None,
            honor_interests: false,
            script: Mutex::new(Script::default()),
            recognitions: AtomicUsize::new(0),
            closes: AtomicUsize::new(0),
            last_max_candidates: AtomicUsize::new(0),
            last_max_text_bytes: AtomicUsize::new(0),
            last_region: Mutex::new(None),
            last_interests: Mutex::new(None),
            last_candidate_counts: Mutex::new(LatestCandidateCounts::default()),
        }
    }

    /// Replaces the exact descriptor published by this backend.
    #[must_use]
    pub fn with_descriptor(mut self, descriptor: OcrBackendDescriptor) -> Self {
        self.descriptor = descriptor;
        self
    }

    /// Scripts the candidates returned by every unscripted call.
    #[must_use]
    pub fn with_candidates(self, candidates: Vec<ScriptedOcrCandidate>) -> Self {
        self.script().default_call.candidates = candidates;
        self
    }

    /// Filters scripted candidates through grouped request interests.
    #[must_use]
    pub const fn honoring_interests(mut self) -> Self {
        self.honor_interests = true;
        self
    }

    /// Advances `clock` by `latency` before every unscripted call returns.
    #[must_use]
    pub fn with_latency(mut self, clock: Arc<ManualClock>, latency: Duration) -> Self {
        self.script().default_call.latency = latency;
        self.clock = Some(clock);
        self
    }

    /// Sets recognition behavior for every unscripted call.
    #[must_use]
    pub fn recognizing(self, behavior: OcrBehavior) -> Self {
        self.script().default_call.behavior = behavior;
        self
    }

    /// Sets close behavior.
    #[must_use]
    pub fn closing(self, behavior: OcrBehavior) -> Self {
        self.script().close = behavior;
        self
    }

    /// Cancels `token` after backend admission and before output.
    #[must_use]
    pub fn cancelling(mut self, token: CancellationToken) -> Self {
        self.cancel_during_recognize = Some(token);
        self
    }

    /// Cancels `token` after the sink accepts `count` candidates.
    #[must_use]
    pub fn cancelling_after_candidates(mut self, token: CancellationToken, count: usize) -> Self {
        self.cancel_after_candidates = Some((token, count));
        self
    }

    /// Cancels `token` after every scripted candidate has reached the sink.
    #[must_use]
    pub fn cancelling_after_output(mut self, token: CancellationToken) -> Self {
        self.cancel_after_output = Some(token);
        self
    }

    /// Cancels `token` from inside close before returning its scripted outcome.
    #[must_use]
    pub fn cancelling_close(mut self, token: CancellationToken) -> Self {
        self.cancel_during_close = Some(token);
        self
    }

    /// Blocks every unscripted call at `gate`.
    #[must_use]
    pub fn with_completion_gate(self, gate: Arc<CompletionGate>) -> Self {
        self.script().default_call.gate = Some(gate);
        self
    }

    /// Scripts calls in FIFO admission order.
    #[must_use]
    pub fn with_calls(self, calls: Vec<ScriptedOcrCall>) -> Self {
        self.script().calls = calls.into();
        self
    }

    /// Uses `clock` for per-call scripted latency.
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<ManualClock>) -> Self {
        self.clock = Some(clock);
        self
    }

    /// Returns how many calls reached backend recognition.
    #[must_use]
    pub fn recognition_count(&self) -> usize {
        self.recognitions.load(Ordering::Acquire)
    }

    /// Returns how many calls reached backend close.
    #[must_use]
    pub fn close_count(&self) -> usize {
        self.closes.load(Ordering::Acquire)
    }

    /// Returns the latest requested candidate ceiling.
    #[must_use]
    pub fn last_max_candidates(&self) -> usize {
        self.last_max_candidates.load(Ordering::Acquire)
    }

    /// Returns the latest requested per-candidate text ceiling.
    #[must_use]
    pub fn last_max_text_bytes(&self) -> usize {
        self.last_max_text_bytes.load(Ordering::Acquire)
    }

    /// Returns the latest effective source region seen by the backend.
    #[must_use]
    pub fn last_region(&self) -> Option<PixelRect> {
        *self
            .last_region
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Returns the latest caller-order relative interests.
    #[must_use]
    pub fn last_interests(&self) -> Option<Vec<PixelRect>> {
        self.last_interests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Returns candidates submitted by the latest admitted call.
    #[must_use]
    pub fn last_selected_candidates(&self) -> usize {
        self.last_candidate_counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .selected
    }

    /// Returns candidates removed by honored interests in the latest admitted call.
    #[must_use]
    pub fn last_ignored_candidates(&self) -> usize {
        self.last_candidate_counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .ignored
    }

    fn script(&self) -> std::sync::MutexGuard<'_, Script> {
        self.script
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl fmt::Debug for ControlledOcr {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ControlledOcr")
            .field("descriptor", &self.descriptor)
            .field("recognitions", &self.recognition_count())
            .field("closes", &self.close_count())
            .finish_non_exhaustive()
    }
}

impl OcrBackend for ControlledOcr {
    fn descriptor(&self) -> OcrBackendDescriptor {
        self.descriptor.clone()
    }

    fn recognize(
        &self,
        request: &BackendRequest<'_>,
        output: &mut dyn OcrCandidateSink,
        operation: &OperationContext,
    ) -> Result<()> {
        self.recognitions.fetch_add(1, Ordering::AcqRel);
        let mut counts = LatestCandidateCountsGuard::new(&self.last_candidate_counts);
        self.last_max_candidates
            .store(request.max_candidates(), Ordering::Release);
        self.last_max_text_bytes
            .store(request.max_text_bytes(), Ordering::Release);
        *self
            .last_region
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(request.region());
        let interests = request.interests();
        *self
            .last_interests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            interests.map(|interests| interests.zones().to_vec());
        if let Some(interruption) = operation.interruption() {
            return Err(interruption.into());
        }
        let call = {
            let mut script = self.script();
            script
                .calls
                .pop_front()
                .unwrap_or_else(|| script.default_call.clone())
        };
        if let Some(gate) = &call.gate {
            gate.enter_and_wait();
        }
        let _completion = GateCompletion(call.gate.as_deref());
        if let Some(clock) = &self.clock {
            clock.advance(call.latency);
        }
        if let Some(token) = &self.cancel_during_recognize {
            token.cancel();
        }
        if call.behavior == OcrBehavior::Unavailable {
            call.behavior.apply()?;
        }
        for candidate in &call.candidates {
            if self.honor_interests
                && let Some(interests) = interests
            {
                let membership = candidate_interest_membership(
                    candidate.quadrilateral,
                    request.region().extent(),
                    interests,
                )?;
                if membership == 0 {
                    counts.ignored += 1;
                    continue;
                }
            }
            output.push(candidate.borrowed())?;
            counts.selected += 1;
            if let Some((token, count)) = &self.cancel_after_candidates
                && counts.selected == *count
            {
                token.cancel();
            }
        }
        if let Some(token) = &self.cancel_after_output {
            token.cancel();
        }
        if call.behavior == OcrBehavior::Fail {
            call.behavior.apply()?;
        }
        Ok(())
    }

    fn close(&self, operation: &OperationContext) -> Result<()> {
        self.closes.fetch_add(1, Ordering::AcqRel);
        if let Some(interruption) = operation.interruption() {
            return Err(interruption.into());
        }
        if let Some(token) = &self.cancel_during_close {
            token.cancel();
        }
        self.script().close.apply()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_guard_opens_gate_on_scope_exit() {
        let gate = Arc::new(CompletionGate::new());
        let worker_gate = Arc::clone(&gate);
        let worker = std::thread::spawn(move || {
            worker_gate.enter_and_wait();
            worker_gate.complete();
        });
        let release = gate.release_guard();

        assert!(gate.wait_until_entered(Duration::from_secs(2)));
        drop(release);
        assert!(gate.wait_until_completed(Duration::from_secs(2)));
        worker.join().expect("gate worker completed");
    }
}
