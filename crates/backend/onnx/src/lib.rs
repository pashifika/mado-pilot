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
#[cfg(test)]
mod native_tests;
mod session;
mod vocabulary;

use std::path::Path;
use std::sync::{Mutex, TryLockError};

use mado_pilot_capture::PixelFormat;
use mado_pilot_core::{OperationContext, Result as CoreResult};
use mado_pilot_ocr::{
    BackendCandidate, BackendId, BackendRequest, BackendVersion, OcrBackend, OcrBackendDescriptor,
    OcrBackendIdentity, OcrCandidateSink, OcrModelSource,
};
use opencv::core::MatTraitConst;

pub use fault::{
    OnnxBackendFacts, OnnxBackendFault, OnnxExecutionProvider, OnnxRuntimeCompatibility,
};

use crate::decode::DecodedText;
use crate::detect::{Detection, Quad};
use crate::session::SessionPair;

pub(crate) const MAX_OUTPUT_BYTES: usize = 256 * 1024 * 1024;
pub(crate) const RECOGNITION_BATCH: usize = 6;

const BACKEND_ID: &str = "onnxruntime-cpu";
const BACKEND_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "+ort-1.29.0-api17");

/// One bounded reusable CPU backend for the exact accepted OCR source.
pub struct OnnxOcrBackend {
    descriptor: OcrBackendDescriptor,
    facts: OnnxBackendFacts,
    state: Mutex<Option<SessionPair>>,
}

impl OnnxOcrBackend {
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
        let model_identity = source.identity().clone();
        loader::initialize(runtime_path)?;
        checkpoint(operation)?;
        let sessions = SessionPair::open(source, operation)?;
        let backend_identity = OcrBackendIdentity::new(
            BackendId::new(BACKEND_ID).map_err(|_| OnnxBackendFault::GraphMismatch)?,
            BackendVersion::new(BACKEND_VERSION).map_err(|_| OnnxBackendFault::GraphMismatch)?,
        );
        let descriptor =
            OcrBackendDescriptor::new(backend_identity, model_identity, PixelFormat::Bgra8);
        let facts = OnnxBackendFacts::accepted(
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
            state: Mutex::new(Some(sessions)),
        })
    }

    /// Returns closed provider, compatibility, and resource facts.
    #[must_use]
    pub const fn facts(&self) -> OnnxBackendFacts {
        self.facts
    }

    fn recognize_locked(
        pair: &mut SessionPair,
        request: &BackendRequest<'_>,
        operation: &OperationContext,
    ) -> Result<Vec<OwnedCandidate>, OnnxBackendFault> {
        image::with_bgra_view(request.pixels(), |source| {
            checkpoint(operation)?;
            let detector_input = image::detector_input(source)?;
            checkpoint(operation)?;
            let source_width =
                u32::try_from(source.cols()).map_err(|_| OnnxBackendFault::InvalidPixels)?;
            let source_height =
                u32::try_from(source.rows()).map_err(|_| OnnxBackendFault::InvalidPixels)?;
            let detections = inference::detector(
                pair.detector_mut(),
                &detector_input,
                source_width,
                source_height,
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
        let candidates = {
            let mut state = match self.state.try_lock() {
                Ok(state) => state,
                Err(TryLockError::WouldBlock) => return Err(OnnxBackendFault::Busy.into()),
                Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            };
            let pair = state.as_mut().ok_or(OnnxBackendFault::Closed)?;
            Self::recognize_locked(pair, request, operation)
                .map_err(mado_pilot_core::Error::from)?
        };

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
        let pair = match self.state.try_lock() {
            Ok(mut state) => state.take(),
            Err(TryLockError::WouldBlock) => return Err(OnnxBackendFault::Busy.into()),
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner().take(),
        };
        drop(pair);
        checkpoint(operation).map_err(mado_pilot_core::Error::from)
    }
}

impl Drop for OnnxOcrBackend {
    fn drop(&mut self) {
        self.state
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
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
    use super::{
        MAX_OUTPUT_BYTES, OnnxBackendFacts, OnnxBackendFault, OnnxExecutionProvider,
        OnnxRuntimeCompatibility, RECOGNITION_BATCH,
    };

    #[test]
    fn facts_are_closed_and_do_not_carry_runtime_paths() {
        let facts = OnnxBackendFacts::accepted(
            1024,
            u64::try_from(MAX_OUTPUT_BYTES).expect("output ceiling fits"),
            1000,
            u32::try_from(RECOGNITION_BATCH).expect("batch ceiling fits"),
        );
        assert_eq!(facts.provider(), OnnxExecutionProvider::Cpu);
        assert_eq!(facts.runtime(), OnnxRuntimeCompatibility::Version1_29Api17);
        assert!(!format!("{facts:?}").contains('/'));
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
}
