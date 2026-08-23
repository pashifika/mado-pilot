//! A deterministic OCR backend for contract and race tests.

use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use mado_pilot_capture::PixelFormat;
use mado_pilot_core::{CancellationToken, OperationContext, Result};
use mado_pilot_ocr::{
    BackendCandidate, BackendId, BackendRequest, BackendVersion, ModelId, OcrBackend,
    OcrBackendDescriptor, OcrFault, ProfileId,
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

    /// Releases the blocked backend call. Idempotent.
    pub fn release(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.released = true;
        self.changed.notify_all();
    }

    fn enter_and_wait(&self) {
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
}

#[derive(Debug, Default)]
struct Script {
    candidates: Vec<BackendCandidate>,
    latency: Duration,
    recognize: OcrBehavior,
    close: OcrBehavior,
}

/// A backend whose candidates, latency, interruption, failures, and completion are scripted.
pub struct ControlledOcr {
    descriptor: OcrBackendDescriptor,
    clock: Option<Arc<ManualClock>>,
    cancel_during_recognize: Option<CancellationToken>,
    gate: Option<Arc<CompletionGate>>,
    script: Mutex<Script>,
    recognitions: AtomicUsize,
    closes: AtomicUsize,
}

impl ControlledOcr {
    /// Returns a controlled backend that recognizes nothing successfully.
    #[must_use]
    pub fn new(format: PixelFormat) -> Self {
        Self {
            descriptor: OcrBackendDescriptor::new(
                BackendId::new(CONTROLLED_OCR_BACKEND).expect("constant backend identity"),
                BackendVersion::new("1").expect("constant backend version"),
                ModelId::new(CONTROLLED_OCR_MODEL).expect("constant model identity"),
                ProfileId::new(CONTROLLED_OCR_PROFILE).expect("constant profile identity"),
                format,
            ),
            clock: None,
            cancel_during_recognize: None,
            gate: None,
            script: Mutex::new(Script::default()),
            recognitions: AtomicUsize::new(0),
            closes: AtomicUsize::new(0),
        }
    }

    /// Replaces the exact descriptor published by this backend.
    #[must_use]
    pub fn with_descriptor(mut self, descriptor: OcrBackendDescriptor) -> Self {
        self.descriptor = descriptor;
        self
    }

    /// Scripts the candidates returned by every recognition call.
    #[must_use]
    pub fn with_candidates(self, candidates: Vec<BackendCandidate>) -> Self {
        self.script().candidates = candidates;
        self
    }

    /// Advances `clock` by `latency` before every recognition returns.
    #[must_use]
    pub fn with_latency(mut self, clock: Arc<ManualClock>, latency: Duration) -> Self {
        self.script().latency = latency;
        self.clock = Some(clock);
        self
    }

    /// Sets recognition behavior.
    #[must_use]
    pub fn recognizing(self, behavior: OcrBehavior) -> Self {
        self.script().recognize = behavior;
        self
    }

    /// Sets close behavior.
    #[must_use]
    pub fn closing(self, behavior: OcrBehavior) -> Self {
        self.script().close = behavior;
        self
    }

    /// Cancels `token` after backend admission and before returning candidates.
    #[must_use]
    pub fn cancelling(mut self, token: CancellationToken) -> Self {
        self.cancel_during_recognize = Some(token);
        self
    }

    /// Blocks every recognition at `gate` until the test releases it.
    #[must_use]
    pub fn with_completion_gate(mut self, gate: Arc<CompletionGate>) -> Self {
        self.gate = Some(gate);
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
        _request: &BackendRequest<'_>,
        operation: &OperationContext,
    ) -> Result<Vec<BackendCandidate>> {
        self.recognitions.fetch_add(1, Ordering::AcqRel);
        if let Some(interruption) = operation.interruption() {
            return Err(interruption.into());
        }
        let (latency, behavior, candidates) = {
            let script = self.script();
            (script.latency, script.recognize, script.candidates.clone())
        };
        if let Some(gate) = &self.gate {
            gate.enter_and_wait();
        }
        if let Some(clock) = &self.clock {
            clock.advance(latency);
        }
        if let Some(token) = &self.cancel_during_recognize {
            token.cancel();
        }
        behavior.apply()?;
        Ok(candidates)
    }

    fn close(&self, operation: &OperationContext) -> Result<()> {
        self.closes.fetch_add(1, Ordering::AcqRel);
        if let Some(interruption) = operation.interruption() {
            return Err(interruption.into());
        }
        self.script().close.apply()
    }
}
