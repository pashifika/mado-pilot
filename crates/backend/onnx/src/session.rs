//! Bounded ownership and validation for the accepted detector/recognizer sessions.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use mado_pilot_core::OperationContext;
use mado_pilot_ocr::OcrModelSource;
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::{TensorElementType, ValueType};

use crate::fault::{
    OnnxBackendFault, OnnxBackendObservations, OnnxBackendOpenFault, OnnxExecutionProvider,
    OnnxProviderFallbackReason,
};
use crate::profile::{DetectorPlan, SelectedProfile};
use crate::provider::{self, PreparedProvider};
use crate::vocabulary::Vocabulary;

const DETECTOR_INPUT: &str = "x";
const DETECTOR_OUTPUT: &str = "sigmoid_0.tmp_0";
const RECOGNIZER_INPUT: &str = "x";
const RECOGNIZER_OUTPUT: &str = "fetch_name_0";
const VOCABULARY_KEY: &str = "character";

static SESSION_PAIR_ACTIVE: AtomicBool = AtomicBool::new(false);

#[derive(Debug)]
pub(crate) struct PairLease;

impl PairLease {
    fn acquire() -> Result<Self, OnnxBackendFault> {
        SESSION_PAIR_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| OnnxBackendFault::Busy)?;
        Ok(Self)
    }
}

impl Drop for PairLease {
    fn drop(&mut self) {
        SESSION_PAIR_ACTIVE.store(false, Ordering::Release);
    }
}

/// The single process-wide detector/recognizer owner.
#[derive(Debug)]
pub(crate) struct SessionPair {
    detector: Session,
    recognizer: Session,
    vocabulary: Vocabulary,
    observations: OnnxBackendObservations,
    _source: OcrModelSource,
    prepared_provider: PreparedProvider,
    _lease: Arc<PairLease>,
}

impl SessionPair {
    pub(crate) fn reserve() -> Result<Arc<PairLease>, OnnxBackendFault> {
        PairLease::acquire().map(Arc::new)
    }

    pub(crate) fn open_with_provider(
        source: OcrModelSource,
        operation: &OperationContext,
        prepared_provider: PreparedProvider,
        lease: Arc<PairLease>,
    ) -> Result<Self, OnnxBackendOpenFault> {
        let provider = prepared_provider.candidate();
        checkpoint(operation).map_err(OnnxBackendOpenFault::terminal)?;
        SelectedProfile::from_identity(source.identity())
            .map_err(OnnxBackendOpenFault::terminal)?;

        debug_assert!(
            Arc::strong_count(&lease) >= 2,
            "the initialization transaction retains its reservation"
        );
        let detector = build_session(source.detector(), provider, "detector")?;
        checkpoint(operation).map_err(OnnxBackendOpenFault::terminal)?;
        validate_graph(
            &detector,
            DETECTOR_INPUT,
            &[-1, 3, -1, -1],
            DETECTOR_OUTPUT,
            &[-1, 1, -1, -1],
        )
        .map_err(|fault| {
            OnnxBackendOpenFault::provider(
                provider,
                fault,
                OnnxProviderFallbackReason::GraphRejected,
            )
        })?;
        #[cfg(feature = "benchmark-instrumentation")]
        crate::benchmark_instrumentation::record_open_stage(
            crate::benchmark_instrumentation::OpenStage::DetectorSessionReady,
        );

        let recognizer = build_session(source.recognizer(), provider, "recognizer")?;
        checkpoint(operation).map_err(OnnxBackendOpenFault::terminal)?;
        validate_graph(
            &recognizer,
            RECOGNIZER_INPUT,
            &[-1, 3, 48, -1],
            RECOGNIZER_OUTPUT,
            &[-1, -1, 18_710],
        )
        .map_err(|fault| {
            OnnxBackendOpenFault::provider(
                provider,
                fault,
                OnnxProviderFallbackReason::GraphRejected,
            )
        })?;
        let raw_vocabulary = recognizer
            .metadata()
            .map_err(|_| {
                OnnxBackendOpenFault::provider(
                    provider,
                    OnnxBackendFault::GraphMismatch,
                    OnnxProviderFallbackReason::GraphRejected,
                )
            })?
            .custom(VOCABULARY_KEY)
            .ok_or_else(|| {
                OnnxBackendOpenFault::provider(
                    provider,
                    OnnxBackendFault::GraphMismatch,
                    OnnxProviderFallbackReason::GraphRejected,
                )
            })?;
        let profile = source.profile_metadata();
        let vocabulary = Vocabulary::parse(
            raw_vocabulary,
            profile.vocabulary_entries(),
            profile.vocabulary_sha256(),
        )
        .map_err(|_| {
            OnnxBackendOpenFault::provider(
                provider,
                OnnxBackendFault::GraphMismatch,
                OnnxProviderFallbackReason::GraphRejected,
            )
        })?;
        checkpoint(operation).map_err(OnnxBackendOpenFault::terminal)?;
        #[cfg(feature = "benchmark-instrumentation")]
        crate::benchmark_instrumentation::record_open_stage(
            crate::benchmark_instrumentation::OpenStage::RecognizerSessionReady,
        );

        Ok(Self {
            detector,
            recognizer,
            vocabulary,
            observations: OnnxBackendObservations::opened(),
            _source: source,
            prepared_provider,
            _lease: lease,
        })
    }

    pub(crate) fn commit_provider(&mut self) -> Result<(), OnnxBackendFault> {
        self.prepared_provider.commit()
    }

    pub(crate) const fn active_provider(&self) -> OnnxExecutionProvider {
        self.prepared_provider.candidate()
    }

    pub(crate) fn record_mapping(&mut self, width: u32, height: u32, bytes: usize) {
        self.observations.record_mapping(width, height, bytes);
    }

    pub(crate) fn record_detector_input(&mut self, plan: DetectorPlan) {
        self.observations.record_detector_input(
            plan.final_width(),
            plan.final_height(),
            plan.tensor_bytes(),
        );
    }

    pub(crate) fn detector_mut(&mut self) -> &mut Session {
        self.observations.record_detector_run();
        &mut self.detector
    }

    pub(crate) fn recognizer_and_vocabulary_mut(&mut self) -> (&mut Session, &Vocabulary) {
        self.observations.record_recognizer_run();
        (&mut self.recognizer, &self.vocabulary)
    }

    pub(crate) fn record_interest_filter(
        &mut self,
        selected: usize,
        ignored: usize,
        memberships: usize,
    ) {
        self.observations
            .record_interest_filter(selected, ignored, memberships);
    }

    pub(crate) fn record_unique_candidates(&mut self, candidates: usize) {
        self.observations.record_unique_candidates(candidates);
    }

    pub(crate) fn record_cleanup(&mut self) {
        self.observations.record_cleanup();
    }

    pub(crate) const fn observations(&self) -> OnnxBackendObservations {
        self.observations
    }
}

#[cfg(feature = "benchmark-instrumentation")]
impl Drop for SessionPair {
    fn drop(&mut self) {
        let _ = self.detector.end_profiling();
        let _ = self.recognizer.end_profiling();
    }
}

#[cfg(feature = "benchmark-instrumentation")]
static PROFILE_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn build_session(
    model: &[u8],
    provider_kind: OnnxExecutionProvider,
    role: &str,
) -> Result<Session, OnnxBackendOpenFault> {
    #[cfg(not(feature = "benchmark-instrumentation"))]
    let _ = role;
    let execution_provider = provider::dispatch(provider_kind).map_err(|reason| {
        OnnxBackendOpenFault::provider(provider_kind, provider::unavailable_fault(reason), reason)
    })?;
    let initialization_fault = || {
        OnnxBackendOpenFault::provider(
            provider_kind,
            OnnxBackendFault::ProviderInitializationFailed,
            OnnxProviderFallbackReason::SessionCreationFailed,
        )
    };
    let builder = Session::builder().map_err(|_| initialization_fault())?;
    let builder = builder
        .with_execution_providers([execution_provider])
        .map_err(|_| {
            OnnxBackendOpenFault::provider(
                provider_kind,
                OnnxBackendFault::ProviderInitializationFailed,
                OnnxProviderFallbackReason::RegistrationFailed,
            )
        })?;
    let builder = builder
        .with_intra_threads(1)
        .map_err(|_| initialization_fault())?;
    let builder = builder
        .with_inter_threads(1)
        .map_err(|_| initialization_fault())?;
    let builder = builder
        .with_parallel_execution(false)
        .map_err(|_| initialization_fault())?;
    let builder = builder
        .with_memory_pattern(false)
        .map_err(|_| initialization_fault())?;
    let mut builder = builder
        .with_optimization_level(GraphOptimizationLevel::All)
        .map_err(|_| initialization_fault())?;
    #[cfg(feature = "benchmark-instrumentation")]
    if let Some(root) = std::env::var_os(crate::benchmark_instrumentation::ORT_PROFILE_DIR_ENV) {
        let root = std::path::PathBuf::from(root);
        let canonical = std::fs::canonicalize(&root).map_err(|_| initialization_fault())?;
        if canonical != root
            || !canonical
                .metadata()
                .map_err(|_| initialization_fault())?
                .is_dir()
        {
            return Err(initialization_fault());
        }
        let sequence = PROFILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let prefix = canonical.join(format!(
            "mado-pilot-{role}-{}-{sequence}",
            std::process::id()
        ));
        builder = builder
            .with_profiling(prefix)
            .map_err(|_| initialization_fault())?;
    }
    builder
        .commit_from_memory(model)
        .map_err(|_| initialization_fault())
}

fn validate_graph(
    session: &Session,
    input_name: &str,
    input_shape: &[i64],
    output_name: &str,
    output_shape: &[i64],
) -> Result<(), OnnxBackendFault> {
    if session.inputs().len() != 1 || session.outputs().len() != 1 {
        return Err(OnnxBackendFault::GraphMismatch);
    }
    validate_outlet(&session.inputs()[0], input_name, input_shape)?;
    validate_outlet(&session.outputs()[0], output_name, output_shape)
}

fn validate_outlet(
    outlet: &ort::value::Outlet,
    expected_name: &str,
    expected_shape: &[i64],
) -> Result<(), OnnxBackendFault> {
    let ValueType::Tensor { ty, shape, .. } = outlet.dtype() else {
        return Err(OnnxBackendFault::GraphMismatch);
    };
    if outlet.name() != expected_name
        || *ty != TensorElementType::Float32
        || shape.as_ref() != expected_shape
    {
        return Err(OnnxBackendFault::GraphMismatch);
    }
    Ok(())
}

fn checkpoint(operation: &OperationContext) -> Result<(), OnnxBackendFault> {
    operation
        .interruption()
        .map_or(Ok(()), |interruption| Err(interruption.into()))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, atomic::Ordering};

    use ort::value::{Outlet, Shape, SymbolicDimensions, TensorElementType, ValueType};

    use super::{SESSION_PAIR_ACTIVE, SessionPair, validate_outlet};
    use crate::fault::OnnxBackendFault;

    #[test]
    fn session_pair_reservation_is_shared_across_fallback_candidates() {
        let transaction = SessionPair::reserve().expect("first reservation");
        let candidate = Arc::clone(&transaction);
        drop(candidate);
        assert!(matches!(
            SessionPair::reserve(),
            Err(OnnxBackendFault::Busy)
        ));
        drop(transaction);
        assert!(!SESSION_PAIR_ACTIVE.load(Ordering::Acquire));
        assert!(SessionPair::reserve().is_ok());
    }

    #[test]
    fn graph_validation_rejects_name_shape_and_element_type_mismatch() {
        let outlet = |name, ty, shape| {
            Outlet::new(
                name,
                ValueType::Tensor {
                    ty,
                    shape: Shape::new(shape),
                    dimension_symbols: SymbolicDimensions::empty(4),
                },
            )
        };
        let accepted = outlet("x", TensorElementType::Float32, [-1, 3, -1, -1]);
        assert_eq!(validate_outlet(&accepted, "x", &[-1, 3, -1, -1]), Ok(()));

        let wrong_name = outlet("other", TensorElementType::Float32, [-1, 3, -1, -1]);
        let wrong_shape = outlet("x", TensorElementType::Float32, [-1, 4, -1, -1]);
        let wrong_type = outlet("x", TensorElementType::Uint8, [-1, 3, -1, -1]);
        for candidate in [&wrong_name, &wrong_shape, &wrong_type] {
            assert_eq!(
                validate_outlet(candidate, "x", &[-1, 3, -1, -1]),
                Err(OnnxBackendFault::GraphMismatch)
            );
        }
    }
}
