//! Explicitly feature-gated local OCR fixture support.
//!
//! This module is absent from release builds. Its extra exported constructor is
//! declared only by `examples/ocr-private-fixture.h`, never by the public ABI
//! header or function table. It exists so C and C++ examples can exercise the
//! released OCR ownership surface without ONNX Runtime, native input, or network.

use std::sync::Arc;

use mado_pilot::{
    BackendCandidate, ContentDigest, DecoderId, LanguageProfileId, ModelComponentIdentity, ModelId,
    ModelVersion, NormalizationId, OcrBackend, OcrBackendDescriptor, OcrBackendId,
    OcrBackendIdentity, OcrBackendRequest, OcrBackendVersion, OcrCandidateSink, OcrModelIdentity,
    OcrProfileMetadata, OperationContext, PixelFormat, PreprocessingId, ProfileId, Result,
};

use crate::engine::madopilot_engine_t;
use crate::error::madopilot_error_t;
use crate::status::madopilot_status_t;
use crate::types::{madopilot_operation_t, madopilot_source_t};

pub(crate) const MODEL_ID: &str = "fixture-ocr-model";
pub(crate) const BACKEND_ID: &str = "fixture-ocr-backend";
pub(crate) const BACKEND_VERSION: &str = "1";
const DETECTOR: &[u8] = b"detector-model-bytes";
const RECOGNIZER: &[u8] = b"recognizer-model-bytes";

#[derive(Debug)]
struct FixtureOcr {
    descriptor: OcrBackendDescriptor,
}

impl OcrBackend for FixtureOcr {
    fn descriptor(&self) -> OcrBackendDescriptor {
        self.descriptor.clone()
    }

    fn recognize(
        &self,
        _request: &OcrBackendRequest<'_>,
        output: &mut dyn OcrCandidateSink,
        _operation: &OperationContext,
    ) -> Result<()> {
        output.push(BackendCandidate::new(
            "魔導士 A-7".as_bytes(),
            [(1.0, 2.0), (20.0, 2.0), (20.0, 9.0), (1.0, 9.0)],
            0.91,
            0,
        ))
    }

    fn close(&self, _operation: &OperationContext) -> Result<()> {
        Ok(())
    }
}

pub(crate) fn ocr_backend() -> Arc<dyn OcrBackend> {
    let detector = ContentDigest::of(DETECTOR);
    let recognizer = ContentDigest::of(RECOGNIZER);
    let model = OcrModelIdentity::new(
        ModelId::new(MODEL_ID).expect("fixture model id"),
        ModelVersion::new("1").expect("fixture model version"),
        ProfileId::new("fixture-ocr-profile").expect("fixture profile"),
        ModelComponentIdentity::new(DETECTOR.len() as u64, *detector.as_bytes())
            .expect("fixture detector identity"),
        ModelComponentIdentity::new(RECOGNIZER.len() as u64, *recognizer.as_bytes())
            .expect("fixture recognizer identity"),
        OcrProfileMetadata::new(
            LanguageProfileId::new("fixture-language").expect("fixture language"),
            PreprocessingId::new("fixture-preprocessing").expect("fixture preprocessing"),
            DecoderId::new("fixture-decoder").expect("fixture decoder"),
            NormalizationId::new(mado_pilot::ACCEPTED_G004_NORMALIZATION_ID)
                .expect("accepted normalization"),
            1,
            [3; 32],
        )
        .expect("fixture profile metadata"),
    )
    .expect("fixture model identity");
    Arc::new(FixtureOcr {
        descriptor: OcrBackendDescriptor::new(
            OcrBackendIdentity::new(
                OcrBackendId::new(BACKEND_ID).expect("fixture backend id"),
                OcrBackendVersion::new(BACKEND_VERSION).expect("fixture backend version"),
            ),
            model,
            PixelFormat::Rgba8,
        ),
    })
}

/// Creates a replay engine wired to the deterministic local OCR fixture.
///
/// This symbol exists only under the `private-fixture` Cargo feature and is not
/// part of the public C ABI. Callers use the ordinary ABI 1.3 table after it
/// returns the engine.
///
/// # Safety
///
/// The source and operation inputs and both outputs follow the same validity,
/// lifetime, initialization, and alignment contract as `engine_create`.
#[doc(hidden)]
#[unsafe(no_mangle)]
pub(crate) unsafe extern "C" fn madopilot_fixture_engine_create(
    source: *const madopilot_source_t,
    options: *const crate::types::madopilot_engine_options_t,
    operation: *const madopilot_operation_t,
    out_engine: *mut *mut madopilot_engine_t,
    out_error: *mut *mut madopilot_error_t,
) -> madopilot_status_t {
    crate::boundary::boundary(|| {
        crate::engine::create_private_ocr_fixture(source, options, operation, out_engine, out_error)
    })
}
