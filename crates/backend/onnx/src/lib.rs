//! Bounded ONNX Runtime CPU OCR backend for the accepted G-004 profile.
//!
//! The adapter loads one exact host-provided runtime from a caller-supplied
//! canonical path, validates the accepted detector/recognizer graphs and
//! vocabulary, reuses one session pair, and admits one synchronous inference at
//! a time. It performs no download, ambient library search, provider fallback,
//! default wiring, or public-contract modification.

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
mod session;
mod vocabulary;

use std::mem::replace;
use std::path::Path;
use std::sync::{Mutex, TryLockError};

use mado_pilot_capture::PixelFormat;
use mado_pilot_core::{OperationContext, Result as CoreResult};
use mado_pilot_ocr::{
    BackendCandidate, BackendId, BackendRequest, BackendVersion, OcrBackend, OcrBackendDescriptor,
    OcrBackendIdentity, OcrCandidateSink, OcrModelSource,
};

pub use fault::{
    OnnxBackendFacts, OnnxBackendFault, OnnxBackendObservations, OnnxExecutionProvider,
    OnnxOcrProfile, OnnxRuntimeCompatibility,
};

use crate::decode::DecodedText;
use crate::detect::{Detection, Quad};
use crate::profile::{PreprocessingDescriptor, SelectedProfile};
use crate::session::SessionPair;

pub(crate) const MAX_OUTPUT_BYTES: usize = 256 * 1024 * 1024;
pub(crate) const RECOGNITION_BATCH: usize = 6;

/// Stable identity of the integrated CPU OCR backend.
pub const BACKEND_ID: &str = "onnxruntime-cpu";
/// Exact backend implementation identity, including the controlled runtime profile.
pub const BACKEND_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "+ort-1.29.0-api17");
/// Closed identity of the controlled native runtime and provider boundary.
pub const RUNTIME_PROFILE_ID: &str = "onnxruntime-1.29.0-api17-cpu";

/// One bounded reusable CPU backend for the exact accepted OCR source.
pub struct OnnxOcrBackend {
    descriptor: OcrBackendDescriptor,
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
    /// Opens the fixed G-004 model pair from one explicit root against one
    /// controlled runtime.
    ///
    /// Only `rapidocr-v3.9.2/ch_PP-OCRv4_det_mobile.onnx` and
    /// `rapidocr-v3.9.2/PP-OCRv6_rec_small.onnx` are read beneath `model_root`.
    /// Runtime initialization happens first so an unavailable runtime does not
    /// allocate or hash the 25.9 MiB model pair. No path is inferred from the
    /// process environment.
    ///
    /// # Errors
    ///
    /// Returns a closed [`OnnxBackendFault`] for an invalid or unavailable
    /// prerequisite, interruption, graph mismatch, or native initialization.
    pub fn open_accepted(
        model_root: &Path,
        runtime_path: &Path,
        operation: &OperationContext,
    ) -> Result<Self, OnnxBackendFault> {
        checkpoint(operation)?;
        loader::initialize(runtime_path)?;
        checkpoint(operation)?;
        let source = model::accepted_source(model_root, operation)?;
        Self::open_initialized(source, operation)
    }

    /// Opens the ADR 0038 bounded-detector profile from one explicit model root
    /// against one controlled runtime.
    ///
    /// This is a non-default selection. It validates the same immutable model
    /// components as [`Self::open_accepted`] but reports and executes the distinct
    /// bounded preprocessing identity. No environment, filename, or per-call
    /// option can select it.
    ///
    /// # Errors
    ///
    /// Returns a closed [`OnnxBackendFault`] for an invalid or unavailable
    /// prerequisite, interruption, graph mismatch, or native initialization.
    pub fn open_bounded_detector(
        model_root: &Path,
        runtime_path: &Path,
        operation: &OperationContext,
    ) -> Result<Self, OnnxBackendFault> {
        checkpoint(operation)?;
        loader::initialize(runtime_path)?;
        checkpoint(operation)?;
        let source = model::bounded_source(model_root, operation)?;
        Self::open_initialized(source, operation)
    }

    /// Opens the accepted detector and recognizer against one controlled runtime.
    ///
    /// `runtime_path` must be an absolute canonical path whose target-specific
    /// filename and runtime version match ADR 0034. The runtime remains loaded
    /// until process exit because ONNX API tables contain pointers into it.
    ///
    /// # Errors
    ///
    /// Returns a closed [`OnnxBackendFault`] for interruption, runtime loading,
    /// profile/graph mismatch, native initialization, or the process-wide
    /// one-session-pair ceiling.
    pub fn open(
        source: OcrModelSource,
        runtime_path: &Path,
        operation: &OperationContext,
    ) -> Result<Self, OnnxBackendFault> {
        checkpoint(operation)?;
        loader::initialize(runtime_path)?;
        checkpoint(operation)?;
        Self::open_initialized(source, operation)
    }

    fn open_initialized(
        source: OcrModelSource,
        operation: &OperationContext,
    ) -> Result<Self, OnnxBackendFault> {
        let profile = SelectedProfile::from_identity(source.identity())?;
        let model_identity = source.identity().clone();
        let sessions = SessionPair::open(source, operation)?;
        let backend_identity = OcrBackendIdentity::new(
            BackendId::new(BACKEND_ID).map_err(|_| OnnxBackendFault::GraphMismatch)?,
            BackendVersion::new(BACKEND_VERSION).map_err(|_| OnnxBackendFault::GraphMismatch)?,
        );
        let descriptor =
            OcrBackendDescriptor::new(backend_identity, model_identity, PixelFormat::Bgra8);
        let preprocessing = profile.preprocessing();
        let facts = OnnxBackendFacts::accepted(
            preprocessing,
            u64::try_from(image::MAX_TENSOR_BYTES).map_err(|_| OnnxBackendFault::ResourceLimit)?,
            u64::try_from(MAX_OUTPUT_BYTES).map_err(|_| OnnxBackendFault::ResourceLimit)?,
            u32::try_from(detect::MAX_DETECTOR_CANDIDATES)
                .map_err(|_| OnnxBackendFault::ResourceLimit)?,
            u32::try_from(RECOGNITION_BATCH).map_err(|_| OnnxBackendFault::ResourceLimit)?,
        );
        checkpoint(operation)?;
        Ok(Self {
            descriptor,
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
        pair.record_mapping(request.pixels().bytes().len());
        image::with_bgra_view(request.pixels(), |source| {
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
            if detections.is_empty() {
                return Ok(Vec::new());
            }

            let mut recognition_order = detections
                .iter()
                .enumerate()
                .map(|(index, detection)| Ok((index, image::recognition_ratio(&detection.quad)?)))
                .collect::<Result<Vec<_>, OnnxBackendFault>>()?;
            recognition_order.sort_by(|left, right| left.1.total_cmp(&right.1));
            let mut decoded: Vec<Option<DecodedText>> =
                (0..detections.len()).map(|_| None).collect();

            for batch in recognition_order.chunks(RECOGNITION_BATCH) {
                checkpoint(operation)?;
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
                    operation,
                )?;
                if batch_output.len() != batch.len() {
                    return Err(OnnxBackendFault::MalformedOutput);
                }
                for ((index, _), text) in batch.iter().zip(batch_output) {
                    decoded[*index] = Some(text);
                }
            }

            detections
                .into_iter()
                .zip(decoded)
                .map(|(detection, decoded)| {
                    let decoded = decoded.ok_or(OnnxBackendFault::MalformedOutput)?;
                    Ok(OwnedCandidate::new(detection, decoded))
                })
                .collect()
        })
    }
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

fn checkpoint(operation: &OperationContext) -> Result<(), OnnxBackendFault> {
    operation
        .interruption()
        .map_or(Ok(()), |interruption| Err(interruption.into()))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use mado_pilot_core::{Clock, MonotonicInstant, OperationContext};

    use super::{
        MAX_OUTPUT_BYTES, OnnxBackendFacts, OnnxBackendFault, OnnxBackendObservations,
        OnnxExecutionProvider, OnnxOcrProfile, OnnxRuntimeCompatibility, RECOGNITION_BATCH,
        SessionSlot, checkpoint,
    };
    use crate::profile::SelectedProfile;

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
        observations.record_mapping(8_294_400);
        observations.record_detector_input(1_312, 736, 11_587_584);
        observations.record_detector_run();
        observations.record_recognizer_run();

        assert_eq!(observations.mapped_bytes(), 8_294_400);
        assert_eq!(observations.latest_detector_width(), Some(1_312));
        assert_eq!(observations.latest_detector_height(), Some(736));
        assert_eq!(observations.detector_tensor_bytes(), 11_587_584);
        assert_eq!(observations.detector_resizes(), 1);
        assert_eq!(observations.detector_runs(), 1);
        assert_eq!(observations.recognizer_runs(), 1);
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
