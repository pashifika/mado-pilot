//! Bounded ONNX Runtime OCR backend for accepted G-004 provider profiles.
//!
//! The adapter loads one exact host-provided runtime from a caller-supplied
//! canonical path, validates the accepted detector/recognizer graphs and
//! vocabulary, reuses one session pair, and admits one synchronous inference at
//! a time. Grouped requests keep one detector run, filter exact relative
//! interests before perspective crops, and recognize each relevant detection
//! once in existing bounded batches. Path-free observations expose only
//! dimensions, counts, bytes, runs, memberships, and cleanup.
//!
//! It performs no download, ambient library search, post-publication provider
//! fallback, default wiring, public-contract modification, per-zone detector
//! loop, or captured content logging.

mod decode;
mod detect;
mod fault;
mod image;
mod inference;
mod loader;
mod model;
#[cfg(test)]
mod native_tests;
mod profile;
mod provider;
mod session;
mod vocabulary;

#[cfg(test)]
#[global_allocator]
static TEST_ACCOUNTING: mado_pilot_testkit::bench_harness::Accounting =
    mado_pilot_testkit::bench_harness::Accounting;

use std::mem::replace;
use std::path::Path;
use std::sync::{Arc, Mutex, TryLockError};

use mado_pilot_capture::PixelFormat;
use mado_pilot_core::{OperationContext, Result as CoreResult};
use mado_pilot_ocr::{
    BackendCandidate, BackendId, BackendRequest, BackendVersion, OcrBackend, OcrBackendDescriptor,
    OcrBackendIdentity, OcrCandidateSink, OcrExecutionProvider as PublicExecutionProvider,
    OcrExecutionProviderPolicy as PublicExecutionProviderPolicy, OcrModelSource,
    OcrProviderDescriptor, OcrProviderFallbackReason as PublicProviderFallbackReason,
    ProviderProfileId, candidate_interest_membership,
};

pub use fault::{
    OnnxBackendFacts, OnnxBackendFault, OnnxBackendObservations, OnnxBackendOpenFault,
    OnnxExecutionProvider, OnnxExecutionProviderPolicy, OnnxOcrProfile, OnnxProviderFallbackReason,
    OnnxRuntimeCompatibility,
};

/// Deterministic native-run gates for the revision-bound qualification bench.
///
/// This module exists only with the non-default `benchmark-instrumentation`
/// feature. Product composition does not enable it.
#[cfg(feature = "benchmark-instrumentation")]
pub mod benchmark_instrumentation {
    use std::fmt;
    use std::mem::size_of;
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;

    /// Explicit private ORT profiling directory used only by placement qualification.
    pub const ORT_PROFILE_DIR_ENV: &str = "MADO_PILOT_ORT_PROFILE_DIR";

    /// Closed initialization boundaries observable by the qualification bench.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum OpenStage {
        /// The exact ONNX Runtime library finished initialization.
        RuntimeInitialized,
        /// Provider preparation and controlled dependency loading completed.
        ProviderPrepared,
        /// Detector session construction and graph validation completed.
        DetectorSessionReady,
        /// Recognizer session, graph, and vocabulary validation completed.
        RecognizerSessionReady,
    }

    /// Closed ordered boundaries in one fresh memory-attribution process.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum MemoryStage {
        /// Before runtime paths, models, or provider dependencies are opened.
        ProcessStart,
        /// The exact ONNX Runtime library finished initialization.
        RuntimeInitialized,
        /// Provider preparation and controlled dependency loading completed.
        ProviderPrepared,
        /// Detector session construction and graph validation completed.
        DetectorSessionReady,
        /// Recognizer session, graph, and vocabulary validation completed.
        RecognizerSessionReady,
        /// One complete pass through every fixed singular and grouped workload completed.
        FirstWarmupPassComplete,
        /// Two further warmup passes and twenty retained passes completed.
        RetainedSequenceComplete,
        /// Session teardown completed while process-global runtime/provider handles remain.
        Closed,
    }

    impl From<OpenStage> for MemoryStage {
        fn from(stage: OpenStage) -> Self {
            match stage {
                OpenStage::RuntimeInitialized => Self::RuntimeInitialized,
                OpenStage::ProviderPrepared => Self::ProviderPrepared,
                OpenStage::DetectorSessionReady => Self::DetectorSessionReady,
                OpenStage::RecognizerSessionReady => Self::RecognizerSessionReady,
            }
        }
    }

    /// Validates the complete non-duplicated memory-stage report order.
    #[must_use]
    pub fn memory_stage_sequence_is_valid(stages: &[MemoryStage]) -> bool {
        const EXPECTED: [MemoryStage; 8] = [
            MemoryStage::ProcessStart,
            MemoryStage::RuntimeInitialized,
            MemoryStage::ProviderPrepared,
            MemoryStage::DetectorSessionReady,
            MemoryStage::RecognizerSessionReady,
            MemoryStage::FirstWarmupPassComplete,
            MemoryStage::RetainedSequenceComplete,
            MemoryStage::Closed,
        ];
        stages == EXPECTED
    }

    struct OpenStageObserver {
        callback: Arc<dyn Fn(OpenStage) + Send + Sync>,
        state: Mutex<OpenStageObserverState>,
        drained: Condvar,
    }

    struct OpenStageObserverState {
        enabled: bool,
        in_flight: usize,
    }

    struct OpenStageCall {
        observer: Arc<OpenStageObserver>,
    }

    /// Exclusive disable-and-drain fence for one open-stage observer.
    pub struct OpenStageObserverGuard {
        observer: Arc<OpenStageObserver>,
    }

    static OPEN_STAGE_OBSERVER: Mutex<Option<Arc<OpenStageObserver>>> = Mutex::new(None);

    impl fmt::Debug for OpenStageObserverGuard {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .debug_struct("OpenStageObserverGuard")
                .finish_non_exhaustive()
        }
    }

    impl OpenStageObserver {
        fn begin(self: &Arc<Self>) -> Option<OpenStageCall> {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !state.enabled {
                return None;
            }
            state.in_flight += 1;
            Some(OpenStageCall {
                observer: Arc::clone(self),
            })
        }
    }

    impl Drop for OpenStageCall {
        fn drop(&mut self) {
            let mut state = self
                .observer
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.in_flight = state
                .in_flight
                .checked_sub(1)
                .expect("an open-stage call owns one in-flight slot");
            if state.in_flight == 0 {
                self.observer.drained.notify_all();
            }
        }
    }

    impl Drop for OpenStageObserverGuard {
        fn drop(&mut self) {
            {
                let mut state = self
                    .observer
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                state.enabled = false;
                while state.in_flight != 0 {
                    state = self
                        .observer
                        .drained
                        .wait(state)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
            }
            let mut active = OPEN_STAGE_OBSERVER
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if active
                .as_ref()
                .is_some_and(|observer| Arc::ptr_eq(observer, &self.observer))
            {
                *active = None;
            }
        }
    }

    /// Installs one process-global open-stage observer.
    ///
    /// Dropping the returned guard disables observation and waits for any
    /// synchronous callback already in flight. No callback runs while the
    /// observer registry lock is held.
    ///
    /// # Panics
    ///
    /// Panics when another open-stage observer is still installed.
    #[must_use]
    pub fn install_open_stage_observer(
        observe: impl Fn(OpenStage) + Send + Sync + 'static,
    ) -> OpenStageObserverGuard {
        let observer = Arc::new(OpenStageObserver {
            callback: Arc::new(observe),
            state: Mutex::new(OpenStageObserverState {
                enabled: true,
                in_flight: 0,
            }),
            drained: Condvar::new(),
        });
        let mut active = OPEN_STAGE_OBSERVER
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if active.is_some() {
            drop(active);
            panic!("only one open-stage observer may be active");
        }
        *active = Some(Arc::clone(&observer));
        OpenStageObserverGuard { observer }
    }

    pub(crate) fn record_open_stage(stage: OpenStage) {
        let observer = OPEN_STAGE_OBSERVER
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let Some(observer) = observer else {
            return;
        };
        let Some(_call) = observer.begin() else {
            return;
        };
        (observer.callback)(stage);
    }

    /// One exclusive detector/recognizer run gate installed by a qualification process.
    pub struct NativeRunGate {
        inner: crate::inference::test_hook::RunGateGuard,
    }

    impl fmt::Debug for NativeRunGate {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .debug_struct("NativeRunGate")
                .finish_non_exhaustive()
        }
    }

    impl NativeRunGate {
        /// Waits until a native run reaches admission.
        #[must_use]
        pub fn wait_until_admitted(&self, timeout: Duration) -> bool {
            self.inner.wait_until_admitted(timeout)
        }

        /// Releases the admitted run into the native session.
        pub fn release(&self) {
            self.inner.release();
        }

        /// Waits until the native session call has started.
        #[must_use]
        pub fn wait_until_run_started(&self, timeout: Duration) -> bool {
            self.inner.wait_until_run_started(timeout)
        }

        /// Waits until the cancellation monitor issues native termination.
        #[must_use]
        pub fn wait_until_termination_issued(&self, timeout: Duration) -> bool {
            self.inner.wait_until_termination_issued(timeout)
        }
    }

    /// Installs one process-global native-run gate for deterministic qualification.
    ///
    /// Qualification uses one synchronous inference, so installing a second
    /// overlapping gate is a harness error.
    #[must_use]
    pub fn install_native_run_gate() -> NativeRunGate {
        NativeRunGate {
            inner: crate::inference::test_hook::install(),
        }
    }

    /// Privacy-safe result of one synthetic maximum-width native recognizer probe.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct RecognizerBudgetProbe {
        /// Number of synthetic accepted-width candidates.
        pub candidates: usize,
        /// Number of checked planner batches produced before native admission.
        pub planned_batches: usize,
        /// Number of native recognizer submissions observed by the backend.
        pub native_runs: u64,
        /// Largest checked output estimate submitted by the probe.
        pub maximum_estimated_output_bytes: usize,
        /// Active provider-specific output budget.
        pub output_budget_bytes: usize,
    }

    /// Runs six accepted maximum-width tensors through the active CUDA recognizer.
    ///
    /// This does not create a public OCR quality fixture. It proves that the
    /// adaptive planner splits native work before the provider output boundary,
    /// while the ordinary end-to-end fixture run-count oracles remain unchanged.
    ///
    /// # Errors
    ///
    /// Returns a typed backend fault when the active provider is not CUDA, the
    /// planner or tensor arithmetic exceeds a bound, native execution fails, or
    /// the extracted output violates its active budget.
    pub fn run_maximum_width_recognizer_probe(
        backend: &crate::OnnxOcrBackend,
        operation: &mado_pilot_core::OperationContext,
    ) -> Result<RecognizerBudgetProbe, crate::OnnxBackendFault> {
        const WIDTH: usize = crate::image::MAX_RECOGNIZER_WIDTH;
        const CANDIDATES: usize = crate::RECOGNITION_BATCH;
        const CHANNELS: usize = 3;
        const HEIGHT: usize = 48;

        backend.state.try_with(|pair| {
            if pair.active_provider() != crate::OnnxExecutionProvider::Cuda {
                return Err(crate::OnnxBackendFault::ProviderUnavailable);
            }
            let budget = crate::provider::recognizer_output_budget(pair.active_provider());
            let recognition_order = (0..CANDIDATES)
                .map(|index| (index, WIDTH as f64 / HEIGHT as f64))
                .collect::<Vec<_>>();
            let before_runs = pair.observations().recognizer_runs();
            let mut start = 0;
            let mut maximum_estimated_output_bytes = 0;
            let mut planned_batches = 0;
            while start < recognition_order.len() {
                let range =
                    crate::recognition_batch_range(&recognition_order, start, budget, operation)?;
                let end = range.end;
                let batch = range.len();
                planned_batches += 1;
                let estimated_output_bytes =
                    crate::decode::estimated_recognizer_output_bytes(batch, WIDTH)?;
                maximum_estimated_output_bytes =
                    maximum_estimated_output_bytes.max(estimated_output_bytes);
                let input_elements = batch
                    .checked_mul(CHANNELS)
                    .and_then(|value| value.checked_mul(HEIGHT))
                    .and_then(|value| value.checked_mul(WIDTH))
                    .ok_or(crate::OnnxBackendFault::ResourceLimit)?;
                let input_bytes = input_elements
                    .checked_mul(size_of::<f32>())
                    .ok_or(crate::OnnxBackendFault::ResourceLimit)?;
                if input_bytes > crate::image::MAX_TENSOR_BYTES {
                    return Err(crate::OnnxBackendFault::ResourceLimit);
                }
                let input = crate::image::TensorInput {
                    data: vec![0.0; input_elements],
                    shape: [batch, CHANNELS, HEIGHT, WIDTH],
                };
                let (recognizer, vocabulary) = pair.recognizer_and_vocabulary_mut();
                let decoded = crate::inference::recognizer(
                    recognizer,
                    vocabulary,
                    &input,
                    64 * 1024,
                    budget,
                    operation,
                )?;
                if decoded.len() != batch {
                    return Err(crate::OnnxBackendFault::MalformedOutput);
                }
                start = end;
            }
            let native_runs = pair
                .observations()
                .recognizer_runs()
                .checked_sub(before_runs)
                .ok_or(crate::OnnxBackendFault::MalformedOutput)?;
            Ok(RecognizerBudgetProbe {
                candidates: CANDIDATES,
                planned_batches,
                native_runs,
                maximum_estimated_output_bytes,
                output_budget_bytes: budget,
            })
        })?
    }
    #[cfg(test)]
    mod tests {
        use std::panic::{AssertUnwindSafe, catch_unwind};
        use std::sync::{Arc, Mutex, mpsc};
        use std::thread;
        use std::time::Duration;

        use super::{
            MemoryStage, OpenStage, install_open_stage_observer, memory_stage_sequence_is_valid,
            record_open_stage,
        };

        static TEST_LOCK: Mutex<()> = Mutex::new(());

        #[test]
        fn open_stage_observer_is_exclusive_and_disabled_by_its_guard() {
            let _test = TEST_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let observed = Arc::new(Mutex::new(Vec::new()));
            let callback_observed = Arc::clone(&observed);
            let guard = install_open_stage_observer(move |stage| {
                callback_observed
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(stage);
            });

            assert!(
                catch_unwind(AssertUnwindSafe(|| install_open_stage_observer(|_| {}))).is_err()
            );
            record_open_stage(OpenStage::RuntimeInitialized);
            drop(guard);
            record_open_stage(OpenStage::ProviderPrepared);
            assert_eq!(
                *observed
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
                vec![OpenStage::RuntimeInitialized]
            );
        }

        #[test]
        fn open_stage_guard_drains_an_in_flight_callback() {
            let _test = TEST_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let (entered_sender, entered_receiver) = mpsc::channel();
            let (release_sender, release_receiver) = mpsc::channel();
            let release_receiver = Arc::new(Mutex::new(release_receiver));
            let callback_release = Arc::clone(&release_receiver);
            let guard = install_open_stage_observer(move |_| {
                entered_sender.send(()).expect("report callback entry");
                callback_release
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .recv()
                    .expect("release callback");
            });
            let marker = thread::spawn(|| record_open_stage(OpenStage::RuntimeInitialized));
            entered_receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("callback entered");

            let (drained_sender, drained_receiver) = mpsc::channel();
            let drain = thread::spawn(move || {
                drop(guard);
                drained_sender.send(()).expect("report drained guard");
            });
            assert!(
                drained_receiver
                    .recv_timeout(Duration::from_millis(20))
                    .is_err(),
                "guard returned before the callback drained"
            );
            release_sender.send(()).expect("release callback");
            marker.join().expect("stage marker joins");
            drained_receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("guard drained");
            drain.join().expect("drain thread joins");
        }

        #[test]
        fn memory_stage_sequence_rejects_missing_duplicate_and_out_of_order_rows() {
            let expected = [
                MemoryStage::ProcessStart,
                MemoryStage::RuntimeInitialized,
                MemoryStage::ProviderPrepared,
                MemoryStage::DetectorSessionReady,
                MemoryStage::RecognizerSessionReady,
                MemoryStage::FirstWarmupPassComplete,
                MemoryStage::RetainedSequenceComplete,
                MemoryStage::Closed,
            ];
            assert!(memory_stage_sequence_is_valid(&expected));
            assert!(!memory_stage_sequence_is_valid(&expected[..7]));

            let mut duplicate = expected;
            duplicate[4] = MemoryStage::DetectorSessionReady;
            assert!(!memory_stage_sequence_is_valid(&duplicate));

            let mut out_of_order = expected;
            out_of_order.swap(1, 2);
            assert!(!memory_stage_sequence_is_valid(&out_of_order));
        }
    }
}

use crate::decode::DecodedText;
use crate::detect::{Detection, Quad};
use crate::profile::{PreprocessingDescriptor, SelectedProfile};
use crate::session::SessionPair;

pub(crate) const MAX_OUTPUT_BYTES: usize = 256 * 1024 * 1024;
pub(crate) const RECOGNITION_BATCH: usize = 6;
/// Stable identity of the integrated CPU OCR backend.
pub const BACKEND_ID: &str = "onnxruntime-cpu";
/// Stable identity of the CUDA OCR backend.
pub const CUDA_BACKEND_ID: &str = "onnxruntime-cuda";
/// Stable identity of the CoreML OCR backend.
pub const COREML_BACKEND_ID: &str = "onnxruntime-coreml";
/// Exact backend implementation identity, including the controlled runtime profile.
pub const BACKEND_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "+ort-1.29.0-api17");
/// Closed identity of the controlled CPU runtime/provider boundary.
pub const RUNTIME_PROFILE_ID: &str = "onnxruntime-1.29.0-api17-cpu";
/// Closed identity of the controlled CUDA runtime/provider boundary.
pub const CUDA_RUNTIME_PROFILE_ID: &str = "onnxruntime-1.29.0-api17-cuda13-cudnn9";
/// Closed identity of the controlled CoreML runtime/provider boundary.
pub const COREML_RUNTIME_PROFILE_ID: &str = "onnxruntime-1.29.0-api17-coreml";

const fn public_provider(provider: OnnxExecutionProvider) -> PublicExecutionProvider {
    match provider {
        OnnxExecutionProvider::Cpu => PublicExecutionProvider::Cpu,
        OnnxExecutionProvider::Cuda => PublicExecutionProvider::Cuda,
        OnnxExecutionProvider::CoreMl => PublicExecutionProvider::CoreMl,
    }
}

const fn public_provider_policy(
    policy: OnnxExecutionProviderPolicy,
) -> PublicExecutionProviderPolicy {
    match policy {
        OnnxExecutionProviderPolicy::Cpu => PublicExecutionProviderPolicy::Cpu,
        OnnxExecutionProviderPolicy::AutoPreferAccelerator => {
            PublicExecutionProviderPolicy::AutoPreferAccelerator
        }
        OnnxExecutionProviderPolicy::PreferCuda => PublicExecutionProviderPolicy::PreferCuda,
        OnnxExecutionProviderPolicy::RequireCuda => PublicExecutionProviderPolicy::RequireCuda,
        OnnxExecutionProviderPolicy::PreferCoreMl => PublicExecutionProviderPolicy::PreferCoreMl,
        OnnxExecutionProviderPolicy::RequireCoreMl => PublicExecutionProviderPolicy::RequireCoreMl,
    }
}

const fn public_fallback_reason(
    reason: OnnxProviderFallbackReason,
) -> PublicProviderFallbackReason {
    match reason {
        OnnxProviderFallbackReason::UnsupportedTarget => {
            PublicProviderFallbackReason::UnsupportedTarget
        }
        OnnxProviderFallbackReason::BuildCapabilityUnavailable => {
            PublicProviderFallbackReason::BuildCapabilityUnavailable
        }
        OnnxProviderFallbackReason::ProviderUnavailable => {
            PublicProviderFallbackReason::ProviderUnavailable
        }
        OnnxProviderFallbackReason::DependencyUnavailable => {
            PublicProviderFallbackReason::DependencyUnavailable
        }
        OnnxProviderFallbackReason::RegistrationFailed => {
            PublicProviderFallbackReason::RegistrationFailed
        }
        OnnxProviderFallbackReason::SessionCreationFailed => {
            PublicProviderFallbackReason::SessionCreationFailed
        }
        OnnxProviderFallbackReason::GraphRejected => PublicProviderFallbackReason::GraphRejected,
        OnnxProviderFallbackReason::QualificationRejected => {
            PublicProviderFallbackReason::QualificationRejected
        }
    }
}

/// One bounded reusable ONNX OCR backend with one immutable provider session pair.
pub struct OnnxOcrBackend {
    descriptor: OcrBackendDescriptor,
    provider_descriptor: OcrProviderDescriptor,
    facts: OnnxBackendFacts,
    profile: SelectedProfile,
    state: SessionSlot<SessionPair>,
}

/// Moves the session value out under the mutex, then runs caller-owned clocks
/// and native work with only the explicit `Running` state retained.
#[derive(Debug)]
struct SessionSlot<T> {
    state: Mutex<BackendState<T>>,
}

#[derive(Debug)]
enum BackendState<T> {
    Open(T),
    Running,
    Closed,
}

struct RunningSession<'a, T> {
    slot: &'a SessionSlot<T>,
    value: Option<T>,
}

impl<T> SessionSlot<T> {
    fn new(value: T) -> Self {
        Self {
            state: Mutex::new(BackendState::Open(value)),
        }
    }

    fn try_with<R>(&self, use_value: impl FnOnce(&mut T) -> R) -> Result<R, OnnxBackendFault> {
        let mut running = RunningSession::admit(self)?;
        Ok(use_value(running.value()))
    }

    fn close(&self) -> Result<Option<T>, OnnxBackendFault> {
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => return Err(OnnxBackendFault::Busy),
            Err(TryLockError::Poisoned(poisoned)) => {
                let mut state = poisoned.into_inner();
                let stale = replace(&mut *state, BackendState::Closed);
                drop(state);
                drop(stale);
                return Err(OnnxBackendFault::NativeFailure);
            }
        };
        match replace(&mut *state, BackendState::Closed) {
            BackendState::Open(value) => Ok(Some(value)),
            BackendState::Running => {
                *state = BackendState::Running;
                Err(OnnxBackendFault::Busy)
            }
            BackendState::Closed => Ok(None),
        }
    }

    fn close_mut(&mut self) -> Option<T> {
        let state = self
            .state
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match replace(state, BackendState::Closed) {
            BackendState::Open(value) => Some(value),
            BackendState::Running | BackendState::Closed => None,
        }
    }
}

impl<'a, T> RunningSession<'a, T> {
    fn admit(slot: &'a SessionSlot<T>) -> Result<Self, OnnxBackendFault> {
        let mut state = match slot.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => return Err(OnnxBackendFault::Busy),
            Err(TryLockError::Poisoned(poisoned)) => {
                let mut state = poisoned.into_inner();
                let stale = replace(&mut *state, BackendState::Closed);
                drop(state);
                drop(stale);
                return Err(OnnxBackendFault::NativeFailure);
            }
        };
        match replace(&mut *state, BackendState::Running) {
            BackendState::Open(value) => Ok(Self {
                slot,
                value: Some(value),
            }),
            BackendState::Running => {
                *state = BackendState::Running;
                Err(OnnxBackendFault::Busy)
            }
            BackendState::Closed => {
                *state = BackendState::Closed;
                Err(OnnxBackendFault::Closed)
            }
        }
    }

    fn value(&mut self) -> &mut T {
        self.value
            .as_mut()
            .expect("an admitted session owns its value")
    }
}

impl<T> Drop for RunningSession<'_, T> {
    fn drop(&mut self) {
        let Some(value) = self.value.take() else {
            return;
        };
        let mut state = self
            .slot
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let replacement = if std::thread::panicking() {
            BackendState::Closed
        } else {
            BackendState::Open(value)
        };
        let previous = replace(&mut *state, replacement);
        drop(state);
        debug_assert!(matches!(previous, BackendState::Running));
    }
}

impl OnnxOcrBackend {
    /// Opens the fixed G-004 model pair with the released CPU-only policy.
    pub fn open_accepted(
        model_root: &Path,
        runtime_path: &Path,
        operation: &OperationContext,
    ) -> Result<Self, OnnxBackendFault> {
        Self::open_accepted_with_provider_policy(
            model_root,
            runtime_path,
            OnnxExecutionProviderPolicy::Cpu,
            operation,
        )
        .map_err(OnnxBackendOpenFault::fault)
    }

    /// Opens the fixed G-004 model pair with an explicit initialization policy.
    pub fn open_accepted_with_provider_policy(
        model_root: &Path,
        runtime_path: &Path,
        provider_policy: OnnxExecutionProviderPolicy,
        operation: &OperationContext,
    ) -> Result<Self, OnnxBackendOpenFault> {
        Self::open_accepted_with_provider_config(
            model_root,
            runtime_path,
            provider_policy,
            None,
            operation,
        )
    }

    /// Opens the fixed model pair with provider policy and an explicit dependency root.
    pub fn open_accepted_with_provider_config(
        model_root: &Path,
        runtime_path: &Path,
        provider_policy: OnnxExecutionProviderPolicy,
        provider_root: Option<&Path>,
        operation: &OperationContext,
    ) -> Result<Self, OnnxBackendOpenFault> {
        checkpoint(operation)?;
        initialize_runtime(runtime_path)?;
        checkpoint(operation)?;
        let source = model::accepted_source(model_root, operation)?;
        Self::open_initialized_with_provider_policy(
            source,
            provider_policy,
            provider_root,
            runtime_path,
            operation,
        )
    }

    /// Opens the bounded-detector profile with the released CPU-only policy.
    pub fn open_bounded_detector(
        model_root: &Path,
        runtime_path: &Path,
        operation: &OperationContext,
    ) -> Result<Self, OnnxBackendFault> {
        Self::open_bounded_detector_with_provider_policy(
            model_root,
            runtime_path,
            OnnxExecutionProviderPolicy::Cpu,
            operation,
        )
        .map_err(OnnxBackendOpenFault::fault)
    }

    /// Opens the bounded-detector profile with an explicit initialization policy.
    pub fn open_bounded_detector_with_provider_policy(
        model_root: &Path,
        runtime_path: &Path,
        provider_policy: OnnxExecutionProviderPolicy,
        operation: &OperationContext,
    ) -> Result<Self, OnnxBackendOpenFault> {
        Self::open_bounded_detector_with_provider_config(
            model_root,
            runtime_path,
            provider_policy,
            None,
            operation,
        )
    }

    /// Opens the bounded model pair with provider policy and an explicit dependency root.
    pub fn open_bounded_detector_with_provider_config(
        model_root: &Path,
        runtime_path: &Path,
        provider_policy: OnnxExecutionProviderPolicy,
        provider_root: Option<&Path>,
        operation: &OperationContext,
    ) -> Result<Self, OnnxBackendOpenFault> {
        checkpoint(operation)?;
        initialize_runtime(runtime_path)?;
        checkpoint(operation)?;
        let source = model::bounded_source(model_root, operation)?;
        Self::open_initialized_with_provider_policy(
            source,
            provider_policy,
            provider_root,
            runtime_path,
            operation,
        )
    }

    /// Opens one validated source with the released CPU-only policy.
    pub fn open(
        source: OcrModelSource,
        runtime_path: &Path,
        operation: &OperationContext,
    ) -> Result<Self, OnnxBackendFault> {
        Self::open_with_provider_policy(
            source,
            runtime_path,
            OnnxExecutionProviderPolicy::Cpu,
            operation,
        )
        .map_err(OnnxBackendOpenFault::fault)
    }

    /// Opens one validated source with an explicit initialization policy.
    pub fn open_with_provider_policy(
        source: OcrModelSource,
        runtime_path: &Path,
        provider_policy: OnnxExecutionProviderPolicy,
        operation: &OperationContext,
    ) -> Result<Self, OnnxBackendOpenFault> {
        Self::open_with_provider_config(source, runtime_path, provider_policy, None, operation)
    }

    /// Opens one validated source with provider policy and an explicit dependency root.
    pub fn open_with_provider_config(
        source: OcrModelSource,
        runtime_path: &Path,
        provider_policy: OnnxExecutionProviderPolicy,
        provider_root: Option<&Path>,
        operation: &OperationContext,
    ) -> Result<Self, OnnxBackendOpenFault> {
        checkpoint(operation)?;
        initialize_runtime(runtime_path)?;
        checkpoint(operation)?;
        Self::open_initialized_with_provider_policy(
            source,
            provider_policy,
            provider_root,
            runtime_path,
            operation,
        )
    }

    fn open_initialized_with_provider_policy(
        source: OcrModelSource,
        provider_policy: OnnxExecutionProviderPolicy,
        provider_root: Option<&Path>,
        runtime_path: &Path,
        operation: &OperationContext,
    ) -> Result<Self, OnnxBackendOpenFault> {
        let profile = SelectedProfile::from_identity(source.identity())?;
        let model_identity = source.identity().clone();
        let reservation = SessionPair::reserve()?;
        let preparation =
            provider::prepare(provider_policy, provider_root, runtime_path, operation);
        #[cfg(feature = "benchmark-instrumentation")]
        if preparation.is_ok() {
            benchmark_instrumentation::record_open_stage(
                benchmark_instrumentation::OpenStage::ProviderPrepared,
            );
        }
        let candidate_source = source.clone();
        let candidate_reservation = Arc::clone(&reservation);
        let cpu_reservation = Arc::clone(&reservation);
        let (provider, fallback_reason, mut sessions) = initialize_provider(
            provider_policy,
            preparation,
            operation,
            move |candidate| {
                SessionPair::open_with_provider(
                    candidate_source,
                    operation,
                    candidate,
                    candidate_reservation,
                )
            },
            move |cpu| SessionPair::open_with_provider(source, operation, cpu, cpu_reservation),
        )?;
        let backend_identity = OcrBackendIdentity::new(
            BackendId::new(match provider {
                OnnxExecutionProvider::Cpu => BACKEND_ID,
                OnnxExecutionProvider::Cuda => CUDA_BACKEND_ID,
                OnnxExecutionProvider::CoreMl => COREML_BACKEND_ID,
            })
            .map_err(|_| OnnxBackendFault::GraphMismatch)?,
            BackendVersion::new(BACKEND_VERSION).map_err(|_| OnnxBackendFault::GraphMismatch)?,
        );
        let descriptor =
            OcrBackendDescriptor::new(backend_identity, model_identity, PixelFormat::Bgra8);
        let preprocessing = profile.preprocessing();
        let facts = OnnxBackendFacts::accepted_with_provider(
            preprocessing,
            u64::try_from(image::MAX_TENSOR_BYTES).map_err(|_| OnnxBackendFault::ResourceLimit)?,
            u64::try_from(MAX_OUTPUT_BYTES).map_err(|_| OnnxBackendFault::ResourceLimit)?,
            u32::try_from(detect::MAX_DETECTOR_CANDIDATES)
                .map_err(|_| OnnxBackendFault::ResourceLimit)?,
            u32::try_from(RECOGNITION_BATCH).map_err(|_| OnnxBackendFault::ResourceLimit)?,
            provider_policy,
            provider,
            fallback_reason,
        );
        let provider_descriptor = OcrProviderDescriptor::new(
            public_provider_policy(provider_policy),
            public_provider(provider),
            fallback_reason.map(public_fallback_reason),
            ProviderProfileId::new(provider.runtime_profile_id())
                .map_err(|_| OnnxBackendFault::GraphMismatch)?,
        );
        checkpoint(operation)?;
        sessions.commit_provider()?;
        Ok(Self {
            descriptor,
            provider_descriptor,
            facts,
            profile,
            state: SessionSlot::new(sessions),
        })
    }

    /// Returns closed provider, compatibility, and resource facts.
    #[must_use]
    pub const fn facts(&self) -> OnnxBackendFacts {
        self.facts
    }

    /// Returns cumulative path-free mapping, inference, and session observations.
    ///
    /// # Errors
    ///
    /// Returns [`OnnxBackendFault::Busy`] while inference owns the session pair,
    /// or [`OnnxBackendFault::Closed`] after close.
    pub fn observations(&self) -> Result<OnnxBackendObservations, OnnxBackendFault> {
        self.state.try_with(|pair| pair.observations())
    }

    fn recognize_with_pair(
        pair: &mut SessionPair,
        preprocessing: PreprocessingDescriptor,
        request: &BackendRequest<'_>,
        operation: &OperationContext,
    ) -> Result<Vec<OwnedCandidate>, OnnxBackendFault> {
        let region = request.region();
        pair.record_mapping(
            region.width(),
            region.height(),
            request.pixels().bytes().len(),
        );
        let outcome = image::with_bgra_view(request.pixels(), |source| {
            checkpoint(operation)?;
            let detector_input = image::detector_input(source, preprocessing)?;
            pair.record_detector_input(detector_input.plan);
            checkpoint(operation)?;
            let detections = inference::detector(
                pair.detector_mut(),
                &detector_input,
                request.max_candidates(),
                operation,
            )?;
            checkpoint(operation)?;
            let raw_candidates = detections.len();
            let detections = if let Some(interests) = request.interests() {
                let mut selected = Vec::with_capacity(raw_candidates);
                let mut memberships = 0_usize;
                for detection in detections {
                    let quadrilateral = detection
                        .quad
                        .map(|point| (f64::from(point.x), f64::from(point.y)));
                    let membership = candidate_interest_membership(
                        quadrilateral,
                        request.region().extent(),
                        interests,
                    )
                    .map_err(|_| OnnxBackendFault::MalformedOutput)?;
                    memberships = memberships
                        .checked_add(
                            usize::try_from(membership.count_ones())
                                .map_err(|_| OnnxBackendFault::ResourceLimit)?,
                        )
                        .ok_or(OnnxBackendFault::ResourceLimit)?;
                    if membership != 0 {
                        selected.push(detection);
                    }
                }
                pair.record_interest_filter(
                    selected.len(),
                    raw_candidates - selected.len(),
                    memberships,
                );
                selected
            } else {
                pair.record_interest_filter(raw_candidates, 0, 0);
                detections
            };
            if detections.is_empty() {
                return Ok(Vec::new());
            }

            let recognizer_output_budget =
                provider::recognizer_output_budget(pair.active_provider());
            let mut recognition_order = detections
                .iter()
                .enumerate()
                .map(|(index, detection)| Ok((index, image::recognition_ratio(&detection.quad)?)))
                .collect::<Result<Vec<_>, OnnxBackendFault>>()?;
            recognition_order.sort_by(|left, right| left.1.total_cmp(&right.1));
            let mut decoded: Vec<Option<DecodedText>> =
                (0..detections.len()).map(|_| None).collect();

            let mut start = 0;
            while start < recognition_order.len() {
                let range = recognition_batch_range(
                    &recognition_order,
                    start,
                    recognizer_output_budget,
                    operation,
                )?;
                let end = range.end;
                let batch = &recognition_order[range];
                let quads = batch
                    .iter()
                    .map(|(index, _)| detections[*index].quad)
                    .collect::<Vec<Quad>>();
                let input = image::recognition_input(source, &quads)?;
                let (recognizer, vocabulary) = pair.recognizer_and_vocabulary_mut();
                let batch_output = inference::recognizer(
                    recognizer,
                    vocabulary,
                    &input,
                    request.max_text_bytes(),
                    recognizer_output_budget,
                    operation,
                )?;
                if batch_output.len() != batch.len() {
                    return Err(OnnxBackendFault::MalformedOutput);
                }
                for ((index, _), text) in batch.iter().zip(batch_output) {
                    decoded[*index] = Some(text);
                }
                start = end;
            }

            let candidates = detections
                .into_iter()
                .zip(decoded)
                .map(|(detection, decoded)| {
                    let decoded = decoded.ok_or(OnnxBackendFault::MalformedOutput)?;
                    Ok(OwnedCandidate::new(detection, decoded))
                })
                .collect::<Result<Vec<_>, OnnxBackendFault>>()?;
            pair.record_unique_candidates(candidates.len());
            Ok(candidates)
        });
        pair.record_cleanup();
        outcome
    }
}

fn recognition_batch_range(
    recognition_order: &[(usize, f64)],
    start: usize,
    max_output_bytes: usize,
    operation: &OperationContext,
) -> Result<std::ops::Range<usize>, OnnxBackendFault> {
    checkpoint(operation)?;
    if start >= recognition_order.len() {
        return Err(OnnxBackendFault::ResourceLimit);
    }

    let mut end = start;
    while end < recognition_order.len() && end - start < RECOGNITION_BATCH {
        let candidate_batch = end - start + 1;
        let candidate_width = image::recognizer_width_for_ratio(recognition_order[end].1)?;
        let candidate_bytes =
            decode::estimated_recognizer_output_bytes(candidate_batch, candidate_width)?;
        if candidate_bytes > max_output_bytes {
            break;
        }
        end += 1;
    }
    if end == start {
        return Err(OnnxBackendFault::ResourceLimit);
    }
    Ok(start..end)
}

impl std::fmt::Debug for OnnxOcrBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OnnxOcrBackend")
            .field("descriptor", &self.descriptor)
            .field("facts", &self.facts)
            .finish_non_exhaustive()
    }
}

impl OcrBackend for OnnxOcrBackend {
    fn descriptor(&self) -> OcrBackendDescriptor {
        self.descriptor.clone()
    }

    fn provider_descriptor(&self) -> Option<OcrProviderDescriptor> {
        Some(self.provider_descriptor.clone())
    }

    fn recognize(
        &self,
        request: &BackendRequest<'_>,
        output: &mut dyn OcrCandidateSink,
        operation: &OperationContext,
    ) -> CoreResult<()> {
        checkpoint(operation).map_err(mado_pilot_core::Error::from)?;
        let preprocessing = self.profile.preprocessing();
        let candidates = self
            .state
            .try_with(|pair| Self::recognize_with_pair(pair, preprocessing, request, operation))
            .map_err(mado_pilot_core::Error::from)?
            .map_err(mado_pilot_core::Error::from)?;

        checkpoint(operation).map_err(mado_pilot_core::Error::from)?;
        for candidate in &candidates {
            output.push(BackendCandidate::new(
                candidate.text.as_bytes(),
                candidate.quadrilateral,
                candidate.confidence,
                candidate.order,
            ))?;
        }
        checkpoint(operation).map_err(mado_pilot_core::Error::from)
    }

    fn close(&self, operation: &OperationContext) -> CoreResult<()> {
        checkpoint(operation).map_err(mado_pilot_core::Error::from)?;
        let pair = self.state.close().map_err(mado_pilot_core::Error::from)?;
        drop(pair);
        checkpoint(operation).map_err(mado_pilot_core::Error::from)
    }
}

impl Drop for OnnxOcrBackend {
    fn drop(&mut self) {
        drop(self.state.close_mut());
    }
}

#[derive(Debug)]
struct OwnedCandidate {
    text: String,
    quadrilateral: [(f64, f64); 4],
    confidence: f64,
    order: u32,
}

impl OwnedCandidate {
    fn new(detection: Detection, decoded: DecodedText) -> Self {
        Self {
            text: decoded.text,
            quadrilateral: detection
                .quad
                .map(|point| (f64::from(point.x), f64::from(point.y))),
            confidence: decoded.confidence,
            order: detection.order,
        }
    }
}

fn initialize_runtime(runtime_path: &Path) -> Result<(), OnnxBackendFault> {
    loader::initialize(runtime_path)?;
    #[cfg(feature = "benchmark-instrumentation")]
    benchmark_instrumentation::record_open_stage(
        benchmark_instrumentation::OpenStage::RuntimeInitialized,
    );
    Ok(())
}

fn initialize_provider<T>(
    policy: OnnxExecutionProviderPolicy,
    preparation: Result<provider::PreparedProvider, provider::ProviderPreparationFault>,
    operation: &OperationContext,
    open_candidate: impl FnOnce(provider::PreparedProvider) -> Result<T, OnnxBackendOpenFault>,
    open_cpu: impl FnOnce(provider::PreparedProvider) -> Result<T, OnnxBackendOpenFault>,
) -> Result<(OnnxExecutionProvider, Option<OnnxProviderFallbackReason>, T), OnnxBackendOpenFault> {
    match preparation {
        Ok(prepared) => {
            let candidate_provider = prepared.candidate();
            match open_candidate(prepared) {
                Ok(candidate) => Ok((candidate_provider, None, candidate)),
                Err(attempt) => {
                    if policy.permits_cpu_fallback() && attempt.provider_reason().is_some() {
                        checkpoint(operation)?;
                        let reason = attempt
                            .provider_reason()
                            .expect("fallback admission requires a provider reason");
                        let cpu = open_cpu(provider::PreparedProvider::cpu())
                            .map_err(|fault| fault.with_provider_reason(reason))?;
                        Ok((OnnxExecutionProvider::Cpu, Some(reason), cpu))
                    } else {
                        Err(attempt)
                    }
                }
            }
        }
        Err(provider::ProviderPreparationFault::Terminal(fault)) => Err(fault.into()),
        Err(provider::ProviderPreparationFault::Provider(reason))
            if policy.permits_cpu_fallback() =>
        {
            checkpoint(operation)?;
            let cpu = open_cpu(provider::PreparedProvider::cpu())
                .map_err(|fault| fault.with_provider_reason(reason))?;
            Ok((OnnxExecutionProvider::Cpu, Some(reason), cpu))
        }
        Err(provider::ProviderPreparationFault::Provider(reason)) => {
            checkpoint(operation)?;
            Err(provider::unavailable_fault(reason).into())
        }
    }
}

fn checkpoint(operation: &OperationContext) -> Result<(), OnnxBackendFault> {
    operation
        .interruption()
        .map_or(Ok(()), |interruption| Err(interruption.into()))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::Arc;
    use std::time::Duration;

    use mado_pilot_core::{CancellationToken, Clock, MonotonicInstant, OperationContext};

    use super::{
        MAX_OUTPUT_BYTES, OnnxBackendFacts, OnnxBackendFault, OnnxBackendObservations,
        OnnxBackendOpenFault, OnnxExecutionProvider, OnnxExecutionProviderPolicy, OnnxOcrProfile,
        OnnxProviderFallbackReason, OnnxRuntimeCompatibility, RECOGNITION_BATCH,
        RUNTIME_PROFILE_ID, SessionSlot, checkpoint, initialize_provider, recognition_batch_range,
    };
    use crate::profile::SelectedProfile;
    use crate::provider::{
        CUDA_RECOGNIZER_OUTPUT_BYTES, PreparedProvider, ProviderPreparationFault,
    };

    struct DropProbe<'a>(&'a Cell<u32>);

    impl Drop for DropProbe<'_> {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    fn recognition_ratio_for_width(width: usize) -> f64 {
        width as f64 / 48.0
    }

    fn recognition_order(widths: &[usize]) -> Vec<(usize, f64)> {
        widths
            .iter()
            .copied()
            .enumerate()
            .map(|(index, width)| (index, recognition_ratio_for_width(width)))
            .collect()
    }

    #[test]
    fn adaptive_batches_keep_ordinary_widths_and_split_maximum_width() {
        let operation = OperationContext::new();
        for width in [320, 2_048] {
            let order = recognition_order(&[width; RECOGNITION_BATCH]);
            assert_eq!(
                recognition_batch_range(&order, 0, CUDA_RECOGNIZER_OUTPUT_BYTES, &operation),
                Ok(0..6)
            );
        }

        let order = recognition_order(&[4_096; RECOGNITION_BATCH]);
        assert_eq!(
            recognition_batch_range(&order, 0, CUDA_RECOGNIZER_OUTPUT_BYTES, &operation),
            Ok(0..3)
        );
        assert_eq!(
            recognition_batch_range(&order, 3, CUDA_RECOGNIZER_OUTPUT_BYTES, &operation),
            Ok(3..6)
        );
        assert_eq!(
            recognition_batch_range(&order, 0, MAX_OUTPUT_BYTES, &operation),
            Ok(0..6)
        );
    }

    #[test]
    fn adaptive_batches_cover_each_candidate_once_and_restore_index_order() {
        let operation = OperationContext::new();
        let mut order = recognition_order(&[4_096, 320, 2_048, 4_096, 1_280, 4_096]);
        order.sort_by(|left, right| left.1.total_cmp(&right.1));
        let mut restored = vec![None; order.len()];
        let mut start = 0;
        let mut batch_count = 0;
        while start < order.len() {
            let range =
                recognition_batch_range(&order, start, CUDA_RECOGNIZER_OUTPUT_BYTES, &operation)
                    .expect("accepted batch");
            assert!(!range.is_empty());
            assert!(range.len() <= RECOGNITION_BATCH);
            let end = range.end;
            for (index, _) in &order[range] {
                assert!(restored[*index].replace(*index).is_none());
            }
            start = end;
            batch_count += 1;
        }
        assert_eq!(restored, (0..order.len()).map(Some).collect::<Vec<_>>());
        assert_eq!(batch_count, 2);
    }

    #[test]
    fn adaptive_batch_planner_rejects_zero_items_and_oversized_single_item() {
        let operation = OperationContext::new();
        assert_eq!(
            recognition_batch_range(&[], 0, CUDA_RECOGNIZER_OUTPUT_BYTES, &operation),
            Err(OnnxBackendFault::ResourceLimit)
        );
        let order = recognition_order(&[4_096]);
        assert_eq!(
            recognition_batch_range(&order, 0, 0, &operation),
            Err(OnnxBackendFault::ResourceLimit)
        );
        assert_eq!(
            recognition_batch_range(&order, 0, CUDA_RECOGNIZER_OUTPUT_BYTES, &operation),
            Ok(0..1)
        );
    }

    #[test]
    fn cancellation_before_the_next_adaptive_batch_prevents_planning() {
        let cancellation = CancellationToken::new();
        let operation = OperationContext::new().with_cancellation(cancellation.clone());
        let order = recognition_order(&[4_096; RECOGNITION_BATCH]);
        assert_eq!(
            recognition_batch_range(&order, 0, CUDA_RECOGNIZER_OUTPUT_BYTES, &operation),
            Ok(0..3)
        );

        cancellation.cancel();
        assert_eq!(
            recognition_batch_range(&order, 3, CUDA_RECOGNIZER_OUTPUT_BYTES, &operation),
            Err(OnnxBackendFault::Cancelled)
        );
    }

    #[test]
    fn preferred_provider_failures_drop_candidates_before_one_fresh_cpu_open() {
        for reason in [
            OnnxProviderFallbackReason::RegistrationFailed,
            OnnxProviderFallbackReason::SessionCreationFailed,
            OnnxProviderFallbackReason::SessionCreationFailed,
            OnnxProviderFallbackReason::GraphRejected,
        ] {
            let candidate_calls = Cell::new(0);
            let cpu_calls = Cell::new(0);
            let partial_drops = Cell::new(0);
            let operation = OperationContext::new();
            let (active, fallback, value) = initialize_provider(
                OnnxExecutionProviderPolicy::PreferCuda,
                Ok(PreparedProvider::for_test(OnnxExecutionProvider::Cuda)),
                &operation,
                |provider| {
                    let provider = provider.candidate();
                    assert_eq!(provider, OnnxExecutionProvider::Cuda);
                    candidate_calls.set(candidate_calls.get() + 1);
                    let _partial = DropProbe(&partial_drops);
                    Err::<&str, _>(OnnxBackendOpenFault::provider(
                        provider,
                        OnnxBackendFault::ProviderInitializationFailed,
                        reason,
                    ))
                },
                |cpu| {
                    assert_eq!(cpu.candidate(), OnnxExecutionProvider::Cpu);
                    assert_eq!(
                        partial_drops.get(),
                        1,
                        "the failed accelerator candidate is gone before CPU opens"
                    );
                    cpu_calls.set(cpu_calls.get() + 1);
                    Ok("fresh-cpu")
                },
            )
            .expect("preferred provider falls back during initialization");

            assert_eq!(active, OnnxExecutionProvider::Cpu);
            assert_eq!(fallback, Some(reason));
            assert_eq!(value, "fresh-cpu");
            assert_eq!(candidate_calls.get(), 1);
            assert_eq!(cpu_calls.get(), 1);
            assert_eq!(partial_drops.get(), 1);
        }
    }

    #[test]
    fn required_terminal_and_interrupted_provider_failures_never_open_cpu() {
        let operation = OperationContext::new();
        for (policy, attempt, expected) in [
            (
                OnnxExecutionProviderPolicy::RequireCuda,
                OnnxBackendOpenFault::provider(
                    OnnxExecutionProvider::Cuda,
                    OnnxBackendFault::ProviderInitializationFailed,
                    OnnxProviderFallbackReason::RegistrationFailed,
                ),
                OnnxBackendFault::ProviderInitializationFailed,
            ),
            (
                OnnxExecutionProviderPolicy::PreferCuda,
                OnnxBackendOpenFault::terminal(OnnxBackendFault::Cancelled),
                OnnxBackendFault::Cancelled,
            ),
        ] {
            let cpu_calls = Cell::new(0);
            let fault = initialize_provider(
                policy,
                Ok(PreparedProvider::for_test(OnnxExecutionProvider::Cuda)),
                &operation,
                |_| Err::<(), _>(attempt),
                |_| {
                    cpu_calls.set(cpu_calls.get() + 1);
                    Ok(())
                },
            )
            .expect_err("required or terminal failure cannot fall back");
            assert_eq!(fault.fault(), expected);
            assert_eq!(cpu_calls.get(), 0);
        }

        let cpu_calls = Cell::new(0);
        let fault = initialize_provider(
            OnnxExecutionProviderPolicy::RequireCuda,
            Err(ProviderPreparationFault::Provider(
                OnnxProviderFallbackReason::DependencyUnavailable,
            )),
            &operation,
            |_| Ok(()),
            |_| {
                cpu_calls.set(cpu_calls.get() + 1);
                Ok(())
            },
        )
        .expect_err("required preparation failure cannot fall back");
        assert_eq!(
            fault.fault(),
            OnnxBackendFault::ProviderDependencyUnavailable
        );
        assert_eq!(cpu_calls.get(), 0);

        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let cancelled = OperationContext::new().with_cancellation(cancellation);
        for policy in [
            OnnxExecutionProviderPolicy::PreferCuda,
            OnnxExecutionProviderPolicy::RequireCuda,
        ] {
            let fault = initialize_provider(
                policy,
                Err(ProviderPreparationFault::Provider(
                    OnnxProviderFallbackReason::DependencyUnavailable,
                )),
                &cancelled,
                |_| Ok(()),
                |_| {
                    cpu_calls.set(cpu_calls.get() + 1);
                    Ok(())
                },
            )
            .expect_err("terminal cancellation supersedes provider failure");
            assert_eq!(fault.fault(), OnnxBackendFault::Cancelled);
            assert_eq!(cpu_calls.get(), 0);
        }
    }

    #[test]
    fn failed_cpu_fallback_retains_the_accelerator_reason_and_cpu_fault() {
        let operation = OperationContext::new();
        let preparation_fault = initialize_provider(
            OnnxExecutionProviderPolicy::PreferCuda,
            Err(ProviderPreparationFault::Provider(
                OnnxProviderFallbackReason::DependencyUnavailable,
            )),
            &operation,
            |_| Ok(()),
            |_| {
                Err::<(), _>(OnnxBackendOpenFault::terminal(
                    OnnxBackendFault::GraphMismatch,
                ))
            },
        )
        .expect_err("fresh CPU construction also fails");
        assert_eq!(preparation_fault.fault(), OnnxBackendFault::GraphMismatch);
        assert_eq!(
            preparation_fault.provider_reason(),
            Some(OnnxProviderFallbackReason::DependencyUnavailable)
        );
        assert!(
            preparation_fault
                .to_string()
                .contains("controlled accelerator")
        );
        let public_error: mado_pilot_core::Error = preparation_fault.into();
        assert_eq!(
            public_error.status(),
            mado_pilot_core::Status::InvalidArgument
        );
        assert!(public_error.detail().contains("controlled accelerator"));

        let candidate_fault = initialize_provider(
            OnnxExecutionProviderPolicy::PreferCuda,
            Ok(PreparedProvider::for_test(OnnxExecutionProvider::Cuda)),
            &operation,
            |provider| {
                Err::<(), _>(OnnxBackendOpenFault::provider(
                    provider.candidate(),
                    OnnxBackendFault::ProviderInitializationFailed,
                    OnnxProviderFallbackReason::SessionCreationFailed,
                ))
            },
            |_| {
                Err::<(), _>(OnnxBackendOpenFault::terminal(
                    OnnxBackendFault::ProviderInitializationFailed,
                ))
            },
        )
        .expect_err("candidate and fresh CPU construction both fail");
        assert_eq!(
            candidate_fault.fault(),
            OnnxBackendFault::ProviderInitializationFailed
        );
        assert_eq!(
            candidate_fault.provider_reason(),
            Some(OnnxProviderFallbackReason::SessionCreationFailed)
        );
    }

    #[test]
    fn facts_are_closed_and_do_not_carry_runtime_paths() {
        let facts = OnnxBackendFacts::accepted(
            SelectedProfile::BoundedDetector.preprocessing(),
            u64::try_from(crate::image::MAX_TENSOR_BYTES).expect("tensor ceiling fits"),
            u64::try_from(MAX_OUTPUT_BYTES).expect("output ceiling fits"),
            1000,
            u32::try_from(RECOGNITION_BATCH).expect("batch ceiling fits"),
        );
        assert_eq!(facts.provider(), OnnxExecutionProvider::Cpu);
        assert_eq!(
            facts.requested_provider_policy(),
            OnnxExecutionProviderPolicy::Cpu
        );
        assert_eq!(facts.fallback_reason(), None);
        assert!(!facts.initialization_fell_back());
        assert_eq!(facts.runtime_profile_id(), RUNTIME_PROFILE_ID);
        assert_eq!(
            OnnxProviderFallbackReason::DependencyUnavailable.status(),
            mado_pilot_core::Status::Unsupported
        );
        assert_eq!(facts.runtime(), OnnxRuntimeCompatibility::Version1_29Api17);
        assert_eq!(facts.profile(), OnnxOcrProfile::BoundedDetector);
        assert_eq!(facts.max_detector_width(), Some(1_312));
        assert_eq!(facts.max_detector_height(), Some(736));
        assert_eq!(facts.max_detector_tensor_bytes(), 11_587_584);
        assert!(!format!("{facts:?}").contains('/'));

        let native = OnnxBackendFacts::accepted(
            SelectedProfile::NativeG004.preprocessing(),
            u64::try_from(crate::image::MAX_TENSOR_BYTES).expect("tensor ceiling fits"),
            u64::try_from(MAX_OUTPUT_BYTES).expect("output ceiling fits"),
            1000,
            u32::try_from(RECOGNITION_BATCH).expect("batch ceiling fits"),
        );
        assert_eq!(native.profile(), OnnxOcrProfile::NativeG004);
        assert_eq!(native.max_detector_width(), None);
        assert_eq!(native.max_detector_height(), None);
        assert_eq!(
            native.max_detector_tensor_bytes(),
            u64::try_from(crate::image::MAX_TENSOR_BYTES).unwrap()
        );
    }

    #[test]
    fn observations_report_only_bounded_dimensions_counts_and_bytes() {
        let mut observations = OnnxBackendObservations::opened();
        observations.record_mapping(3_840, 2_160, 33_177_600);
        observations.record_detector_input(960, 512, 5_898_240);
        observations.record_detector_run();
        observations.record_recognizer_run();
        observations.record_interest_filter(8, 2, 8);
        assert_eq!(observations.unique_candidates(), 0);
        observations.record_unique_candidates(8);
        observations.record_cleanup();

        assert_eq!(observations.mapped_bytes(), 33_177_600);
        assert_eq!(observations.mapping_calls(), 1);
        assert_eq!(observations.latest_mapping_width(), Some(3_840));
        assert_eq!(observations.latest_mapping_height(), Some(2_160));
        assert_eq!(observations.latest_detector_width(), Some(960));
        assert_eq!(observations.latest_detector_height(), Some(512));
        assert_eq!(observations.detector_tensor_bytes(), 5_898_240);
        assert_eq!(observations.detector_resizes(), 1);
        assert_eq!(observations.detector_runs(), 1);
        assert_eq!(observations.recognizer_runs(), 1);
        assert_eq!(observations.selected_candidates(), 8);
        assert_eq!(observations.ignored_candidates(), 2);
        assert_eq!(observations.unique_candidates(), 8);
        assert_eq!(observations.memberships(), 8);
        assert_eq!(observations.cleanup_completions(), 1);
        assert_eq!(observations.session_pairs(), 1);
        assert_eq!(observations.sessions(), 2);

        let debug = format!("{observations:?}");
        for sensitive in ["/", ".onnx", "sha256", "魔導士", "NativeFailure"] {
            assert!(!debug.contains(sensitive));
        }
    }

    #[test]
    fn faults_expose_only_static_closed_details() {
        for fault in [
            OnnxBackendFault::RuntimeUnavailable,
            OnnxBackendFault::NativeFailure,
            OnnxBackendFault::MalformedOutput,
        ] {
            assert!(!fault.detail().contains('/'));
            assert!(!fault.detail().contains(".onnx"));
        }
    }

    #[derive(Debug)]
    struct StateLockCheckingClock {
        slot: Arc<SessionSlot<()>>,
    }

    impl Clock for StateLockCheckingClock {
        fn now(&self) -> MonotonicInstant {
            assert!(
                self.slot.state.try_lock().is_ok(),
                "caller-owned clock ran while the session state lock was held"
            );
            MonotonicInstant::ORIGIN
        }
    }

    #[test]
    fn caller_clock_runs_outside_the_session_state_lock() {
        let slot = Arc::new(SessionSlot::new(()));
        let operation = OperationContext::new()
            .with_clock(Arc::new(StateLockCheckingClock {
                slot: Arc::clone(&slot),
            }))
            .with_deadline(MonotonicInstant::from_origin(Duration::from_secs(1)));

        let checkpoint_result = slot
            .try_with(|()| checkpoint(&operation))
            .expect("session admission succeeds");

        assert_eq!(checkpoint_result, Ok(()));
    }

    #[test]
    fn panic_during_session_work_closes_instead_of_reusing_the_value() {
        let slot = SessionSlot::new(());
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = slot.try_with(|()| -> () {
                panic!("simulated caller panic");
            });
        }));

        assert!(panic.is_err());
        assert_eq!(slot.try_with(|()| ()), Err(OnnxBackendFault::Closed));
    }
}
