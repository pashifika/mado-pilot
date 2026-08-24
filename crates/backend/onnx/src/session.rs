//! Bounded ownership and validation for the accepted detector/recognizer sessions.

use std::sync::atomic::{AtomicBool, Ordering};

use mado_pilot_core::OperationContext;
use mado_pilot_ocr::{OcrModelIdentity, OcrModelSource};
use ort::ep::CPU;
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::{TensorElementType, ValueType};

use crate::fault::OnnxBackendFault;
use crate::vocabulary::Vocabulary;

const DETECTOR_INPUT: &str = "x";
const DETECTOR_OUTPUT: &str = "sigmoid_0.tmp_0";
const RECOGNIZER_INPUT: &str = "x";
const RECOGNIZER_OUTPUT: &str = "fetch_name_0";
const VOCABULARY_KEY: &str = "character";

static SESSION_PAIR_ACTIVE: AtomicBool = AtomicBool::new(false);

#[derive(Debug)]
struct PairLease;

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
    _source: OcrModelSource,
    _lease: PairLease,
}

impl SessionPair {
    pub(crate) fn open(
        source: OcrModelSource,
        operation: &OperationContext,
    ) -> Result<Self, OnnxBackendFault> {
        checkpoint(operation)?;
        if source.identity() != &OcrModelIdentity::accepted_g004() {
            return Err(OnnxBackendFault::ProfileMismatch);
        }

        let lease = PairLease::acquire()?;
        let detector = build_session(source.detector())?;
        checkpoint(operation)?;
        validate_graph(
            &detector,
            DETECTOR_INPUT,
            &[-1, 3, -1, -1],
            DETECTOR_OUTPUT,
            &[-1, 1, -1, -1],
        )?;

        let recognizer = build_session(source.recognizer())?;
        checkpoint(operation)?;
        validate_graph(
            &recognizer,
            RECOGNIZER_INPUT,
            &[-1, 3, 48, -1],
            RECOGNIZER_OUTPUT,
            &[-1, -1, 18_710],
        )?;
        let raw_vocabulary = recognizer
            .metadata()
            .map_err(|_| OnnxBackendFault::GraphMismatch)?
            .custom(VOCABULARY_KEY)
            .ok_or(OnnxBackendFault::GraphMismatch)?;
        let profile = source.profile_metadata();
        let vocabulary = Vocabulary::parse(
            raw_vocabulary,
            profile.vocabulary_entries(),
            profile.vocabulary_sha256(),
        )?;
        checkpoint(operation)?;

        Ok(Self {
            detector,
            recognizer,
            vocabulary,
            _source: source,
            _lease: lease,
        })
    }

    pub(crate) fn detector_mut(&mut self) -> &mut Session {
        &mut self.detector
    }

    pub(crate) fn recognizer_and_vocabulary_mut(&mut self) -> (&mut Session, &Vocabulary) {
        (&mut self.recognizer, &self.vocabulary)
    }
}

fn build_session(model: &[u8]) -> Result<Session, OnnxBackendFault> {
    Session::builder()
        .map_err(|_| OnnxBackendFault::NativeFailure)?
        .with_execution_providers([CPU::default()
            .with_arena_allocator(false)
            .build()
            .error_on_failure()])
        .map_err(|_| OnnxBackendFault::NativeFailure)?
        .with_intra_threads(1)
        .map_err(|_| OnnxBackendFault::NativeFailure)?
        .with_inter_threads(1)
        .map_err(|_| OnnxBackendFault::NativeFailure)?
        .with_parallel_execution(false)
        .map_err(|_| OnnxBackendFault::NativeFailure)?
        .with_memory_pattern(false)
        .map_err(|_| OnnxBackendFault::NativeFailure)?
        .with_optimization_level(GraphOptimizationLevel::All)
        .map_err(|_| OnnxBackendFault::NativeFailure)?
        .commit_from_memory(model)
        .map_err(|_| OnnxBackendFault::GraphMismatch)
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
    use std::sync::atomic::Ordering;

    use ort::value::{Outlet, Shape, SymbolicDimensions, TensorElementType, ValueType};

    use super::{PairLease, SESSION_PAIR_ACTIVE, validate_outlet};
    use crate::fault::OnnxBackendFault;

    #[test]
    fn session_pair_lease_is_single_and_reusable() {
        let first = PairLease::acquire().expect("first lease");
        assert!(matches!(PairLease::acquire(), Err(OnnxBackendFault::Busy)));
        drop(first);
        assert!(!SESSION_PAIR_ACTIVE.load(Ordering::Acquire));
        assert!(PairLease::acquire().is_ok());
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
