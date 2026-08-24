use std::ptr;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use mado_pilot::{
    ASSET_MANIFEST_PATH, AssetPackage, ContentDigest, DecoderId, LanguageProfileId, MemoryPackage,
    ModelComponentIdentity, ModelId, ModelVersion, NormalizationId, OcrBackend,
    OcrBackendDescriptor, OcrBackendId, OcrBackendIdentity, OcrBackendRequest, OcrBackendVersion,
    OcrCandidateSink, OcrModelIdentity, OcrProfileMetadata, PackageSource, PixelExtent,
    PixelFormat, PreprocessingId, ProfileId, Result as FacadeResult,
};
use mado_pilot_runtime::{
    CaptureProvider, Engine, EngineWiring, IdentityIssuer, Matcher, OcrRecognizer, PackageLoader,
};
use mado_pilot_testkit::{
    CompletionGate, ControlledCapture, ControlledMatcher, ControlledOcr, ScriptedOcrCandidate,
};

use super::*;
use crate::engine::{EngineHandle, madopilot_engine_t};
use crate::layout::struct_size;
use crate::status::{MADOPILOT_STATUS_CLOSED, MADOPILOT_STATUS_INTERNAL_PANIC};
use crate::table::{
    MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR, MADOPILOT_API_SIZE_CURRENT, madopilot_api_t,
    madopilot_get_api,
};
use crate::types::{madopilot_ocr_request_t, madopilot_open_request_t, madopilot_operation_t};

const DETECTOR: &[u8] = b"detector-model-bytes";
const RECOGNIZER: &[u8] = b"recognizer-model-bytes";
const MODEL_ID: &str = "fixture-ocr-model";
const BACKEND_ID: &str = "fixture-ocr-backend";
const BACKEND_VERSION: &str = "1";

fn model_identity() -> OcrModelIdentity {
    let detector = ContentDigest::of(DETECTOR);
    let recognizer = ContentDigest::of(RECOGNIZER);
    OcrModelIdentity::new(
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
    .expect("fixture model identity")
}

fn descriptor() -> OcrBackendDescriptor {
    OcrBackendDescriptor::new(
        OcrBackendIdentity::new(
            OcrBackendId::new(BACKEND_ID).expect("fixture backend id"),
            OcrBackendVersion::new(BACKEND_VERSION).expect("fixture backend version"),
        ),
        model_identity(),
        PixelFormat::Rgba8,
    )
}

fn package() -> AssetPackage {
    let detector = ContentDigest::of(DETECTOR);
    let recognizer = ContentDigest::of(RECOGNIZER);
    let vocabulary = "03".repeat(32);
    let manifest = format!(
        r#"{{
            "schema_version": 2,
            "package": {{"id": "madopilot.fixture.c-ocr", "version": "1.0.0"}},
            "license": "Apache-2.0",
            "templates": [],
            "ocr_models": [{{
                "id": "{MODEL_ID}",
                "version": "1",
                "profile": "fixture-ocr-profile",
                "language_profile": "fixture-language",
                "preprocessing": "fixture-preprocessing",
                "decoder": "fixture-decoder",
                "normalization": "{}",
                "vocabulary": {{
                    "entries": 1,
                    "content": {{"algorithm": "sha256", "value": "{vocabulary}"}}
                }},
                "detector": {{
                    "path": "models/detector.onnx",
                    "byte_len": {},
                    "content": {{"algorithm": "sha256", "value": "{detector}"}}
                }},
                "recognizer": {{
                    "path": "models/recognizer.onnx",
                    "byte_len": {},
                    "content": {{"algorithm": "sha256", "value": "{recognizer}"}}
                }}
            }}]
        }}"#,
        mado_pilot::ACCEPTED_G004_NORMALIZATION_ID,
        DETECTOR.len(),
        RECOGNIZER.len(),
    );
    let source = PackageSource::memory(
        MemoryPackage::new()
            .with_entry(ASSET_MANIFEST_PATH, manifest.into_bytes())
            .with_entry("models/detector.onnx", DETECTOR)
            .with_entry("models/recognizer.onnx", RECOGNIZER),
    );
    PackageLoader::new()
        .load(&source, &mado_pilot::OperationContext::new())
        .expect("fixture package loads")
}

fn api() -> &'static madopilot_api_t {
    let mut raw = ptr::null();
    // SAFETY: `raw` is a writable aligned output for the call.
    let status = unsafe {
        madopilot_get_api(
            MADOPILOT_ABI_MAJOR,
            MADOPILOT_ABI_MINOR,
            MADOPILOT_API_SIZE_CURRENT as usize,
            &mut raw,
        )
    };
    assert_eq!(status, MADOPILOT_STATUS_OK);
    assert!(!raw.is_null());
    // SAFETY: successful negotiation returns the process-lifetime static table.
    unsafe { &*raw }
}

fn operation() -> madopilot_operation_t {
    madopilot_operation_t {
        struct_size: struct_size::<madopilot_operation_t>(),
        flags: 0,
        deadline_nanos: 0,
        cancellation: ptr::null(),
        activity_tag: 0,
    }
}

fn string(value: &'static str) -> madopilot_str_t {
    madopilot_str_t {
        data: value.as_ptr().cast(),
        len: value.len(),
    }
}

fn c_request(
    frame: *const crate::capture::madopilot_frame_t,
    package: *const crate::assets::madopilot_package_t,
) -> madopilot_ocr_request_t {
    madopilot_ocr_request_t {
        struct_size: struct_size::<madopilot_ocr_request_t>(),
        flags: 0,
        frame,
        package,
        model_id: string(MODEL_ID),
        backend_id: string(BACKEND_ID),
        backend_version: string(BACKEND_VERSION),
        output_space: MADOPILOT_SPACE_CAPTURE_PIXELS,
        clip_policy: MADOPILOT_CLIP_POLICY_REJECT,
        region: madopilot_pixel_rect_t::empty(),
    }
}

struct Opened {
    api: &'static madopilot_api_t,
    engine: *mut madopilot_engine_t,
    session: *mut madopilot_session_t,
    frame: *mut crate::capture::madopilot_frame_t,
    package: *mut crate::assets::madopilot_package_t,
}

impl Opened {
    fn release_parents(&mut self) {
        let operation = operation();
        // SAFETY: every non-null pointer is a live handle owned by this fixture.
        unsafe {
            if !self.session.is_null() {
                let mut error = ptr::null_mut();
                let _ = (self.api.session_close)(self.session, &operation, &mut error);
                if !error.is_null() {
                    (self.api.error_release)(error);
                }
            }
            if !self.frame.is_null() {
                (self.api.frame_release)(self.frame);
                self.frame = ptr::null_mut();
            }
            if !self.package.is_null() {
                (self.api.package_release)(self.package);
                self.package = ptr::null_mut();
            }
            if !self.session.is_null() {
                (self.api.session_release)(self.session);
                self.session = ptr::null_mut();
            }
            if !self.engine.is_null() {
                (self.api.engine_release)(self.engine);
                self.engine = ptr::null_mut();
            }
        }
    }
}

impl Drop for Opened {
    fn drop(&mut self) {
        self.release_parents();
    }
}

fn opened(backend: Arc<dyn OcrBackend>) -> Opened {
    let api = api();
    let issuer = Arc::new(IdentityIssuer::new());
    let capture = Arc::new(
        ControlledCapture::new(
            Arc::clone(&issuer),
            PixelExtent::new(32, 24),
            PixelFormat::Rgba8,
        )
        .expect("controlled capture"),
    );
    let engine = Engine::new(EngineWiring {
        engine: issuer.engine(),
        capture: Arc::clone(&capture) as Arc<dyn CaptureProvider>,
        matcher: Matcher::new(Arc::new(ControlledMatcher::new(PixelFormat::Rgba8))),
        loader: PackageLoader::new(),
        ocr: Some(OcrRecognizer::new(backend)),
        input: None,
        permission: None,
    })
    .expect("fixture engine");
    let engine = handle::into_raw(EngineHandle::new(engine));
    let package = handle::into_raw(package());
    let operation = operation();

    let mut targets = ptr::null_mut();
    let mut error = ptr::null_mut();
    // SAFETY: every output is writable and `engine` is retained by this fixture.
    let status = unsafe { (api.engine_discover)(engine, &operation, &mut targets, &mut error) };
    assert_eq!(status, MADOPILOT_STATUS_OK);
    assert!(error.is_null());

    let open = madopilot_open_request_t {
        struct_size: struct_size::<madopilot_open_request_t>(),
        flags: 0,
        required_format: crate::types::MADOPILOT_PIXEL_FORMAT_RGBA8,
        preferred_format: crate::types::MADOPILOT_PIXEL_FORMAT_RGBA8,
    };
    let mut session = ptr::null_mut();
    // SAFETY: handles and inputs remain live for the call; outputs are writable.
    let status = unsafe {
        (api.session_open)(
            engine,
            targets,
            0,
            &open,
            &operation,
            &mut session,
            &mut error,
        )
    };
    assert_eq!(status, MADOPILOT_STATUS_OK);
    assert!(error.is_null());
    // SAFETY: the fixture owns the target-list reference.
    unsafe { (api.target_list_release)(targets) };

    capture
        .publish(0x44, mado_pilot::Continuity::Continuous)
        .expect("published fixture frame");
    let mut frame = ptr::null_mut();
    // SAFETY: the retained session and writable outputs satisfy the entry contract.
    let status =
        unsafe { (api.session_acquire_frame)(session, &operation, &mut frame, &mut error) };
    assert_eq!(status, MADOPILOT_STATUS_OK);
    assert!(error.is_null());

    Opened {
        api,
        engine,
        session,
        frame,
        package,
    }
}

fn successful_backend() -> Arc<dyn OcrBackend> {
    Arc::new(
        ControlledOcr::new(PixelFormat::Rgba8)
            .with_descriptor(descriptor())
            .with_candidates(vec![ScriptedOcrCandidate::new(
                "  魔導士 A-7  ".as_bytes(),
                [(1.0, 2.0), (20.0, 2.0), (20.0, 9.0), (1.0, 9.0)],
                0.91,
                0,
            )]),
    )
}

fn empty_info() -> madopilot_ocr_result_info_t {
    <madopilot_ocr_result_info_t as Versioned>::failure(struct_size::<madopilot_ocr_result_info_t>())
}

fn empty_region() -> madopilot_ocr_region_t {
    <madopilot_ocr_region_t as Versioned>::failure(struct_size::<madopilot_ocr_region_t>())
}

fn text(view: madopilot_str_t) -> &'static str {
    // SAFETY: tests call this only while the owner of a library-returned view is retained.
    unsafe {
        std::str::from_utf8(std::slice::from_raw_parts(view.data.cast(), view.len))
            .expect("library text is UTF-8")
    }
}

#[test]
fn owned_c_result_and_views_survive_every_parent_release() {
    let mut fixture = opened(successful_backend());
    let request = c_request(fixture.frame, fixture.package);
    let operation = operation();
    let mut result = ptr::null_mut();
    let mut error = ptr::null_mut();
    // SAFETY: request inputs and outputs remain valid for the synchronous call.
    let status = unsafe {
        (fixture.api.session_recognize)(
            fixture.session,
            &request,
            &operation,
            &mut result,
            &mut error,
        )
    };
    assert_eq!(status, MADOPILOT_STATUS_OK);
    assert!(!result.is_null());
    assert!(error.is_null());

    let mut expected_source =
        madopilot_frame_stamp_t::cleared(struct_size::<madopilot_frame_stamp_t>());
    assert_eq!(
        // SAFETY: the fixture retains its frame and the output is fully writable.
        unsafe { (fixture.api.frame_stamp)(fixture.frame, &mut expected_source) },
        MADOPILOT_STATUS_OK
    );
    let mut info = empty_info();
    // SAFETY: result is retained and `info` is a full writable output.
    assert_eq!(
        // SAFETY: `result` is retained and `info` is a full writable output.
        unsafe { (fixture.api.ocr_result_info)(result, &mut info) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(info.region_count, 1);
    assert_eq!(info.source, expected_source);
    assert_eq!(text(info.backend_id), BACKEND_ID);
    assert_eq!(text(info.model_id), MODEL_ID);

    let mut region = empty_region();
    // SAFETY: result is retained and index zero is within `region_count`.
    assert_eq!(
        // SAFETY: `result` is retained and index zero is within the result count.
        unsafe { (fixture.api.ocr_result_region_at)(result, 0, &mut region) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(region.confidence, 0.91);
    let mut recognized = madopilot_str_t::empty();
    // SAFETY: result is retained and the output is writable.
    assert_eq!(
        // SAFETY: `result` is retained and `recognized` is writable.
        unsafe { (fixture.api.ocr_result_text_at)(result, 0, &mut recognized) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(text(recognized), "魔導士 A-7");

    fixture.release_parents();
    // SAFETY: the result owns all values independently of every released parent.
    assert_eq!(
        // SAFETY: the result owns its values after every parent was released.
        unsafe { (fixture.api.ocr_result_info)(result, &mut info) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(text(info.model_id), MODEL_ID);
    assert_eq!(text(recognized), "魔導士 A-7");

    // SAFETY: retain adds one owned reference; each release gives up exactly one.
    unsafe {
        assert_eq!((fixture.api.ocr_result_retain)(result), MADOPILOT_STATUS_OK);
        assert_eq!(
            (fixture.api.ocr_result_release)(result),
            MADOPILOT_STATUS_OK
        );
        assert_eq!(
            (fixture.api.ocr_result_info)(result, &mut info),
            MADOPILOT_STATUS_OK
        );
        assert_eq!(
            (fixture.api.ocr_result_release)(result),
            MADOPILOT_STATUS_OK
        );
    }
    result = ptr::null_mut();
    assert!(result.is_null());
}

#[test]
fn invalid_inputs_and_indexes_leave_every_output_in_failure_state() {
    let fixture = opened(successful_backend());
    let operation = operation();
    let mut result = usize::MAX as *mut madopilot_ocr_result_t;
    let mut error = usize::MAX as *mut madopilot_error_t;
    // SAFETY: outputs are valid; a null request is an intentionally invalid input.
    let status = unsafe {
        (fixture.api.session_recognize)(
            fixture.session,
            ptr::null(),
            &operation,
            &mut result,
            &mut error,
        )
    };
    assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT);
    assert!(result.is_null());
    assert!(!error.is_null());
    // SAFETY: the call returned one owned error reference.
    unsafe { (fixture.api.error_release)(error) };

    let request = c_request(fixture.frame, fixture.package);
    error = ptr::null_mut();
    // SAFETY: all inputs and outputs satisfy the successful call contract.
    assert_eq!(
        // SAFETY: all inputs and outputs satisfy the successful call contract.
        unsafe {
            (fixture.api.session_recognize)(
                fixture.session,
                &request,
                &operation,
                &mut result,
                &mut error,
            )
        },
        MADOPILOT_STATUS_OK
    );
    assert!(error.is_null());

    let mut info = empty_info();
    info.region_count = u64::MAX;
    info.backend_id = string("sentinel");
    // SAFETY: a null handle is an invalid input; output is full and writable.
    assert_eq!(
        // SAFETY: null is intentional and the full output is writable.
        unsafe { (fixture.api.ocr_result_info)(ptr::null(), &mut info) },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    assert_eq!(info.region_count, 0);
    assert_eq!(info.backend_id.len, 0);

    let mut region = empty_region();
    region.confidence = 1.0;
    // SAFETY: result is retained; index one is intentionally out of range.
    assert_eq!(
        // SAFETY: the result is retained; index one is intentionally invalid.
        unsafe { (fixture.api.ocr_result_region_at)(result, 1, &mut region) },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    assert_eq!(region.confidence, 0.0);
    let mut recognized = string("sentinel");
    // SAFETY: result is retained; index one is intentionally out of range.
    assert_eq!(
        // SAFETY: the result is retained; index one is intentionally invalid.
        unsafe { (fixture.api.ocr_result_text_at)(result, 1, &mut recognized) },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    assert_eq!(recognized.len, 0);
    assert!(recognized.data.is_null());
    // SAFETY: the fixture owns the result reference.
    unsafe { (fixture.api.ocr_result_release)(result) };
}

#[derive(Debug)]
struct PanickingBackend {
    descriptor: OcrBackendDescriptor,
}

impl OcrBackend for PanickingBackend {
    fn descriptor(&self) -> OcrBackendDescriptor {
        self.descriptor.clone()
    }

    fn recognize(
        &self,
        _request: &OcrBackendRequest<'_>,
        _output: &mut dyn OcrCandidateSink,
        _operation: &mado_pilot::OperationContext,
    ) -> FacadeResult<()> {
        panic!("controlled OCR panic")
    }

    fn close(&self, _operation: &mado_pilot::OperationContext) -> FacadeResult<()> {
        Ok(())
    }
}

#[test]
fn backend_panic_is_contained_with_initialized_outputs() {
    let fixture = opened(Arc::new(PanickingBackend {
        descriptor: descriptor(),
    }));
    let request = c_request(fixture.frame, fixture.package);
    let operation = operation();
    let mut result = usize::MAX as *mut madopilot_ocr_result_t;
    let mut error = usize::MAX as *mut madopilot_error_t;
    // SAFETY: all C arguments are valid; the backend panic is deliberate.
    let status = unsafe {
        (fixture.api.session_recognize)(
            fixture.session,
            &request,
            &operation,
            &mut result,
            &mut error,
        )
    };
    assert_eq!(status, MADOPILOT_STATUS_INTERNAL_PANIC);
    assert!(result.is_null());
    assert!(error.is_null());
}

#[test]
fn concurrent_close_wins_before_c_result_publication() {
    let gate = Arc::new(CompletionGate::new());
    let backend = Arc::new(
        ControlledOcr::new(PixelFormat::Rgba8)
            .with_descriptor(descriptor())
            .with_candidates(vec![ScriptedOcrCandidate::new(
                b"closed" as &'static [u8],
                [(1.0, 1.0), (8.0, 1.0), (8.0, 6.0), (1.0, 6.0)],
                0.8,
                0,
            )])
            .with_completion_gate(Arc::clone(&gate)),
    );
    let fixture = opened(backend);
    let api = fixture.api;
    let session = fixture.session as usize;
    let frame = fixture.frame as usize;
    let package = fixture.package as usize;

    let worker = thread::spawn(move || {
        let operation = operation();
        let request = c_request(
            frame as *const crate::capture::madopilot_frame_t,
            package as *const crate::assets::madopilot_package_t,
        );
        let mut result = ptr::null_mut();
        let mut error = ptr::null_mut();
        // SAFETY: the fixture remains alive until this worker joins.
        let status = unsafe {
            (api.session_recognize)(
                session as *const madopilot_session_t,
                &request,
                &operation,
                &mut result,
                &mut error,
            )
        };
        (status, result as usize, error as usize)
    });
    assert!(gate.wait_until_entered(Duration::from_secs(1)));
    let mut error = ptr::null_mut();
    // SAFETY: the retained session remains live and both outputs are valid.
    assert_eq!(
        // SAFETY: the retained session remains live and both outputs are valid.
        unsafe { (api.session_close)(fixture.session, &operation(), &mut error) },
        MADOPILOT_STATUS_OK
    );
    assert!(error.is_null());
    gate.release();

    let (status, result, error) = worker.join().expect("worker did not panic");
    assert_eq!(status, MADOPILOT_STATUS_CLOSED);
    assert_eq!(result, 0);
    assert_ne!(error, 0);
    // SAFETY: the failed call returned one owned error reference.
    unsafe { (api.error_release)(error as *mut madopilot_error_t) };
}
