use std::mem::{align_of, size_of};
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
use crate::types::{
    madopilot_ocr_engine_descriptor_t, madopilot_ocr_request_t, madopilot_ocr_zone_scan_request_t,
    madopilot_ocr_zone_t, madopilot_open_request_t, madopilot_operation_t,
};

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

fn c_zone(left: i32, top: i32, right: i32, bottom: i32) -> madopilot_ocr_zone_t {
    madopilot_ocr_zone_t {
        struct_size: struct_size::<madopilot_ocr_zone_t>(),
        flags: 0,
        region: madopilot_pixel_rect_t {
            space: MADOPILOT_SPACE_CAPTURE_PIXELS,
            left,
            top,
            right,
            bottom,
        },
        clip_policy: MADOPILOT_CLIP_POLICY_REJECT,
    }
}

fn c_zone_request(
    frame: *const crate::capture::madopilot_frame_t,
    package: *const crate::assets::madopilot_package_t,
    zones: &[madopilot_ocr_zone_t],
) -> madopilot_ocr_zone_scan_request_t {
    madopilot_ocr_zone_scan_request_t {
        struct_size: struct_size::<madopilot_ocr_zone_scan_request_t>(),
        flags: 0,
        frame,
        package,
        model_id: string(MODEL_ID),
        backend_id: string(BACKEND_ID),
        backend_version: string(BACKEND_VERSION),
        output_space: MADOPILOT_SPACE_CAPTURE_PIXELS,
        reserved: 0,
        zones: zones.as_ptr(),
        zone_count: zones.len(),
        zone_stride: size_of::<madopilot_ocr_zone_t>(),
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

fn successful_backend() -> Arc<ControlledOcr> {
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

fn integrated_backend_with_id(
    model: OcrModelIdentity,
    backend_id: &'static str,
) -> Arc<dyn OcrBackend> {
    let descriptor = OcrBackendDescriptor::new(
        OcrBackendIdentity::new(
            OcrBackendId::new(backend_id).expect("integrated backend id is valid"),
            OcrBackendVersion::new(mado_pilot::DEFAULT_OCR_BACKEND_VERSION)
                .expect("default backend version is valid"),
        ),
        model,
        PixelFormat::Rgba8,
    );
    Arc::new(
        ControlledOcr::new(PixelFormat::Rgba8)
            .with_descriptor(descriptor)
            .with_candidates(vec![ScriptedOcrCandidate::new(
                "  魔導士 A-7  ".as_bytes(),
                [(1.0, 2.0), (20.0, 2.0), (20.0, 9.0), (1.0, 9.0)],
                0.91,
                0,
            )]),
    )
}

fn integrated_backend(model: OcrModelIdentity) -> Arc<dyn OcrBackend> {
    integrated_backend_with_id(model, mado_pilot::DEFAULT_OCR_BACKEND_ID)
}

fn integrated_bounded_backend() -> Arc<dyn OcrBackend> {
    integrated_backend(OcrModelIdentity::accepted_bounded_detector())
}

#[test]
fn package_free_integrated_profiles_accept_every_product_provider_id() {
    for backend_id in [
        mado_pilot::DEFAULT_OCR_BACKEND_ID,
        mado_pilot::CUDA_OCR_BACKEND_ID,
        mado_pilot::COREML_OCR_BACKEND_ID,
    ] {
        let descriptor = OcrBackendDescriptor::new(
            OcrBackendIdentity::new(
                OcrBackendId::new(backend_id).expect("product backend id"),
                OcrBackendVersion::new(mado_pilot::DEFAULT_OCR_BACKEND_VERSION)
                    .expect("product backend version"),
            ),
            OcrModelIdentity::accepted_g004(),
            PixelFormat::Bgra8,
        );
        assert!(is_integrated_profile(&descriptor));
    }

    let external = OcrBackendDescriptor::new(
        OcrBackendIdentity::new(
            OcrBackendId::new(BACKEND_ID).expect("fixture backend id"),
            OcrBackendVersion::new(mado_pilot::DEFAULT_OCR_BACKEND_VERSION)
                .expect("product backend version"),
        ),
        OcrModelIdentity::accepted_g004(),
        PixelFormat::Bgra8,
    );
    assert!(!is_integrated_profile(&external));
}

fn integrated_request(
    frame: *const crate::capture::madopilot_frame_t,
    model_id: &'static str,
) -> madopilot_ocr_request_t {
    madopilot_ocr_request_t {
        struct_size: struct_size::<madopilot_ocr_request_t>(),
        flags: 0,
        frame,
        package: ptr::null(),
        model_id: string(model_id),
        backend_id: string(mado_pilot::DEFAULT_OCR_BACKEND_ID),
        backend_version: string(mado_pilot::DEFAULT_OCR_BACKEND_VERSION),
        output_space: MADOPILOT_SPACE_CAPTURE_PIXELS,
        clip_policy: MADOPILOT_CLIP_POLICY_REJECT,
        region: madopilot_pixel_rect_t::empty(),
    }
}

fn integrated_default_request(
    frame: *const crate::capture::madopilot_frame_t,
) -> madopilot_ocr_request_t {
    integrated_request(frame, mado_pilot::ACCEPTED_G004_MODEL_ID)
}

fn integrated_bounded_request(
    frame: *const crate::capture::madopilot_frame_t,
) -> madopilot_ocr_request_t {
    integrated_request(frame, mado_pilot::ACCEPTED_BOUNDED_MODEL_ID)
}

fn empty_info() -> madopilot_ocr_result_info_t {
    <madopilot_ocr_result_info_t as Versioned>::failure(struct_size::<madopilot_ocr_result_info_t>())
}

fn empty_region() -> madopilot_ocr_region_t {
    <madopilot_ocr_region_t as Versioned>::failure(struct_size::<madopilot_ocr_region_t>())
}

fn empty_zone_info() -> madopilot_ocr_zone_scan_result_info_t {
    <madopilot_ocr_zone_scan_result_info_t as Versioned>::failure(struct_size::<
        madopilot_ocr_zone_scan_result_info_t,
    >())
}

fn empty_zone_result() -> madopilot_ocr_zone_result_t {
    <madopilot_ocr_zone_result_t as Versioned>::failure(struct_size::<madopilot_ocr_zone_result_t>())
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
fn grouped_result_owns_unique_candidates_and_survives_every_parent() {
    let mut fixture = opened(successful_backend());
    let zones = [
        c_zone(0, 0, 16, 12),
        c_zone(24, 0, 32, 12),
        c_zone(8, 0, 24, 12),
    ];
    let request = c_zone_request(fixture.frame, fixture.package, &zones);
    let operation = operation();
    let mut result = ptr::NonNull::<madopilot_ocr_zone_scan_result_t>::dangling().as_ptr();
    let mut error = ptr::null_mut();

    // SAFETY: all handles, views, the zone array, and writable outputs remain live.
    assert_eq!(
        // SAFETY: the complete inputs and outputs remain live for this call.
        unsafe {
            (fixture.api.session_scan_ocr_zones)(
                fixture.session,
                &request,
                &operation,
                &mut result,
                &mut error,
            )
        },
        MADOPILOT_STATUS_OK
    );
    assert!(!result.is_null());
    assert!(error.is_null());

    let mut info = empty_zone_info();
    // SAFETY: the result is retained and `info` is writable.
    assert_eq!(
        // SAFETY: this fixture owns the retained result and writable output.
        unsafe { (fixture.api.ocr_zone_scan_result_info)(result, &mut info) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(info.zone_count, 3);
    assert_eq!(info.unique_candidate_count, 1);
    assert_eq!(info.membership_count, 2);
    assert_eq!(text(info.backend_id), BACKEND_ID);
    assert_eq!(text(info.model_id), MODEL_ID);

    let mut first = empty_zone_result();
    let mut empty = empty_zone_result();
    let mut overlap = empty_zone_result();
    // SAFETY: each index is in range and every output is writable.
    unsafe {
        assert_eq!(
            (fixture.api.ocr_zone_scan_result_zone_at)(result, 0, &mut first),
            MADOPILOT_STATUS_OK
        );
        assert_eq!(
            (fixture.api.ocr_zone_scan_result_zone_at)(result, 1, &mut empty),
            MADOPILOT_STATUS_OK
        );
        assert_eq!(
            (fixture.api.ocr_zone_scan_result_zone_at)(result, 2, &mut overlap),
            MADOPILOT_STATUS_OK
        );
    }
    assert_eq!(first.region_count, 1);
    assert_eq!(empty.region_count, 0);
    assert_eq!(overlap.region_count, 1);

    let mut first_text = madopilot_str_t::empty();
    let mut overlap_text = madopilot_str_t::empty();
    // SAFETY: both group-relative indexes exist and outputs are writable.
    unsafe {
        assert_eq!(
            (fixture.api.ocr_zone_scan_result_text_at)(result, 0, 0, &mut first_text),
            MADOPILOT_STATUS_OK
        );
        assert_eq!(
            (fixture.api.ocr_zone_scan_result_text_at)(result, 2, 0, &mut overlap_text),
            MADOPILOT_STATUS_OK
        );
    }
    assert_eq!(text(first_text), "魔導士 A-7");
    assert_eq!(first_text.data, overlap_text.data);
    assert_eq!(first_text.len, overlap_text.len);

    // SAFETY: retain creates a second owner; the first release balances the
    // original reference and the final release balances the retained one.
    unsafe {
        assert_eq!(
            (fixture.api.ocr_zone_scan_result_retain)(result),
            MADOPILOT_STATUS_OK
        );
        assert_eq!(
            (fixture.api.ocr_zone_scan_result_release)(result),
            MADOPILOT_STATUS_OK
        );
    }
    fixture.release_parents();

    let mut retained_text = madopilot_str_t::empty();
    // SAFETY: one result reference remains live after every parent release.
    assert_eq!(
        // SAFETY: one retained result reference survives all released parents.
        unsafe { (fixture.api.ocr_zone_scan_result_text_at)(result, 0, 0, &mut retained_text) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(text(retained_text), "魔導士 A-7");
    // SAFETY: gives up the final owned result reference.
    assert_eq!(
        // SAFETY: this gives up the final retained result reference.
        unsafe { (fixture.api.ocr_zone_scan_result_release)(result) },
        MADOPILOT_STATUS_OK
    );
}

#[test]
fn grouped_result_supports_concurrent_const_access_with_owned_references() {
    let fixture = opened(successful_backend());
    let zones = [c_zone(0, 0, 32, 24)];
    let request = c_zone_request(fixture.frame, fixture.package, &zones);
    let operation = operation();
    let mut result = ptr::null_mut();
    let mut error = ptr::null_mut();
    // SAFETY: complete inputs and writable outputs remain live.
    assert_eq!(
        // SAFETY: complete inputs and writable outputs remain live.
        unsafe {
            (fixture.api.session_scan_ocr_zones)(
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

    let mut workers = Vec::new();
    for _ in 0..4 {
        // SAFETY: each successful retain transfers one independent reference
        // into the worker that releases it.
        assert_eq!(
            // SAFETY: the main thread owns the live reference it duplicates.
            unsafe { (fixture.api.ocr_zone_scan_result_retain)(result) },
            MADOPILOT_STATUS_OK
        );
        let api = fixture.api;
        let address = result as usize;
        workers.push(thread::spawn(move || {
            let result = address as *mut madopilot_ocr_zone_scan_result_t;
            let mut info = empty_zone_info();
            let mut zone = empty_zone_result();
            let mut text_out = madopilot_str_t::empty();
            // SAFETY: this worker owns one result reference and all outputs.
            unsafe {
                assert_eq!(
                    (api.ocr_zone_scan_result_info)(result, &mut info),
                    MADOPILOT_STATUS_OK
                );
                assert_eq!(
                    (api.ocr_zone_scan_result_zone_at)(result, 0, &mut zone),
                    MADOPILOT_STATUS_OK
                );
                assert_eq!(
                    (api.ocr_zone_scan_result_text_at)(result, 0, 0, &mut text_out),
                    MADOPILOT_STATUS_OK
                );
            }
            let owned_text = text(text_out).to_owned();
            // SAFETY: the borrowed view has been copied; this worker now gives
            // up its independent result reference.
            assert_eq!(
                // SAFETY: this worker owns the reference it releases.
                unsafe { (api.ocr_zone_scan_result_release)(result) },
                MADOPILOT_STATUS_OK
            );
            (info.zone_count, zone.region_count, owned_text)
        }));
    }
    for worker in workers {
        assert_eq!(
            worker.join().expect("const reader completed"),
            (1, 1, "魔導士 A-7".to_owned())
        );
    }
    // SAFETY: gives up the original result reference.
    assert_eq!(
        // SAFETY: this gives up the original retained result reference.
        unsafe { (fixture.api.ocr_zone_scan_result_release)(result) },
        MADOPILOT_STATUS_OK
    );
}

#[test]
fn integrated_providers_use_their_retained_model_identity_without_a_duplicate_package() {
    for backend_id in [
        mado_pilot::DEFAULT_OCR_BACKEND_ID,
        mado_pilot::CUDA_OCR_BACKEND_ID,
        mado_pilot::COREML_OCR_BACKEND_ID,
    ] {
        let fixture = opened(integrated_backend_with_id(
            OcrModelIdentity::accepted_g004(),
            backend_id,
        ));
        let mut request = integrated_default_request(fixture.frame);
        request.backend_id = string(backend_id);
        let operation = operation();
        let mut result = ptr::null_mut();
        let mut error = ptr::null_mut();

        assert_eq!(
            // SAFETY: request inputs and outputs remain valid for the synchronous call.
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
        assert!(!result.is_null());
        assert!(error.is_null());

        let mut info = empty_info();
        assert_eq!(
            // SAFETY: the result is retained and `info` is fully writable.
            unsafe { (fixture.api.ocr_result_info)(result, &mut info) },
            MADOPILOT_STATUS_OK
        );
        assert_eq!(text(info.backend_id), backend_id);
        assert_eq!(text(info.model_id), mado_pilot::ACCEPTED_G004_MODEL_ID);
        assert_eq!(text(info.profile_id), mado_pilot::ACCEPTED_G004_PROFILE_ID);

        assert_eq!(
            // SAFETY: release gives up the one owned result reference.
            unsafe { (fixture.api.ocr_result_release)(result) },
            MADOPILOT_STATUS_OK
        );
    }
}

#[test]
fn integrated_bounded_profile_supports_singular_without_a_duplicate_package() {
    let fixture = opened(integrated_bounded_backend());
    let request = integrated_bounded_request(fixture.frame);
    let operation = operation();
    let mut result = ptr::null_mut();
    let mut error = ptr::null_mut();

    assert_eq!(
        // SAFETY: request inputs and outputs remain valid for the synchronous call.
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
    assert!(!result.is_null());
    assert!(error.is_null());

    let mut info = empty_info();
    assert_eq!(
        // SAFETY: the result is retained and `info` is fully writable.
        unsafe { (fixture.api.ocr_result_info)(result, &mut info) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(text(info.backend_id), mado_pilot::DEFAULT_OCR_BACKEND_ID);
    assert_eq!(text(info.model_id), mado_pilot::ACCEPTED_BOUNDED_MODEL_ID);
    assert_eq!(
        text(info.model_version),
        mado_pilot::ACCEPTED_BOUNDED_MODEL_VERSION
    );
    assert_eq!(
        text(info.profile_id),
        mado_pilot::ACCEPTED_BOUNDED_PROFILE_ID
    );

    assert_eq!(
        // SAFETY: release gives up the one owned result reference.
        unsafe { (fixture.api.ocr_result_release)(result) },
        MADOPILOT_STATUS_OK
    );
}

#[test]
fn engine_ocr_descriptor_borrows_the_exact_retained_selection() {
    let fixture = opened(integrated_bounded_backend());
    let mut descriptor = <madopilot_ocr_engine_descriptor_t as Versioned>::failure(struct_size::<
        madopilot_ocr_engine_descriptor_t,
    >());

    assert_eq!(
        // SAFETY: the engine is retained and the full output is writable.
        unsafe { (fixture.api.engine_ocr_descriptor)(fixture.engine, &mut descriptor) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(
        text(descriptor.backend_id),
        mado_pilot::DEFAULT_OCR_BACKEND_ID
    );
    assert_eq!(
        text(descriptor.backend_version),
        mado_pilot::DEFAULT_OCR_BACKEND_VERSION
    );
    assert_eq!(
        text(descriptor.model_id),
        mado_pilot::ACCEPTED_BOUNDED_MODEL_ID
    );
    assert_eq!(
        text(descriptor.model_version),
        mado_pilot::ACCEPTED_BOUNDED_MODEL_VERSION
    );
    assert_eq!(
        text(descriptor.profile_id),
        mado_pilot::ACCEPTED_BOUNDED_PROFILE_ID
    );
}

#[test]
fn engine_ocr_descriptor_initializes_failure_before_reading_the_engine() {
    let poison = madopilot_str_t {
        data: ptr::NonNull::<u8>::dangling().as_ptr().cast(),
        len: usize::MAX,
    };
    let mut descriptor = madopilot_ocr_engine_descriptor_t {
        struct_size: struct_size::<madopilot_ocr_engine_descriptor_t>(),
        flags: u32::MAX,
        backend_id: poison,
        backend_version: poison,
        model_id: poison,
        model_version: poison,
        profile_id: poison,
    };

    assert_eq!(
        // SAFETY: the engine is deliberately null and the complete output is
        // writable, exercising failure-state initialization order.
        unsafe { (api().engine_ocr_descriptor)(ptr::null(), &mut descriptor) },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    assert_eq!(descriptor.flags, 0);
    assert!(descriptor.backend_id.data.is_null());
    assert!(descriptor.backend_version.data.is_null());
    assert!(descriptor.model_id.data.is_null());
    assert!(descriptor.model_version.data.is_null());
    assert!(descriptor.profile_id.data.is_null());
}

#[test]
fn engine_ocr_descriptor_supports_concurrent_const_access_with_owned_references() {
    let fixture = opened(integrated_bounded_backend());
    let mut workers = Vec::new();
    for _ in 0..4 {
        assert_eq!(
            // SAFETY: each successful retain transfers one independent engine
            // reference into the worker that releases it.
            unsafe { (fixture.api.engine_retain)(fixture.engine) },
            MADOPILOT_STATUS_OK
        );
        let api = fixture.api;
        let address = fixture.engine as usize;
        workers.push(thread::spawn(move || {
            let engine = address as *mut madopilot_engine_t;
            let mut descriptor =
                <madopilot_ocr_engine_descriptor_t as Versioned>::failure(struct_size::<
                    madopilot_ocr_engine_descriptor_t,
                >());
            assert_eq!(
                // SAFETY: this worker owns one engine reference and its output
                // is fully writable.
                unsafe { (api.engine_ocr_descriptor)(engine, &mut descriptor) },
                MADOPILOT_STATUS_OK
            );
            let identity = (
                text(descriptor.backend_id).to_owned(),
                text(descriptor.backend_version).to_owned(),
                text(descriptor.model_id).to_owned(),
                text(descriptor.model_version).to_owned(),
                text(descriptor.profile_id).to_owned(),
            );
            let addresses = (
                descriptor.backend_id.data.addr(),
                descriptor.backend_version.data.addr(),
                descriptor.model_id.data.addr(),
                descriptor.model_version.data.addr(),
                descriptor.profile_id.data.addr(),
            );
            // SAFETY: every borrowed view has been copied and this worker now
            // gives up its independent engine reference.
            assert_eq!(unsafe { (api.engine_release)(engine) }, MADOPILOT_STATUS_OK);
            (identity, addresses)
        }));
    }

    let expected_identity = (
        mado_pilot::DEFAULT_OCR_BACKEND_ID.to_owned(),
        mado_pilot::DEFAULT_OCR_BACKEND_VERSION.to_owned(),
        mado_pilot::ACCEPTED_BOUNDED_MODEL_ID.to_owned(),
        mado_pilot::ACCEPTED_BOUNDED_MODEL_VERSION.to_owned(),
        mado_pilot::ACCEPTED_BOUNDED_PROFILE_ID.to_owned(),
    );
    let mut retained_addresses = None;
    for worker in workers {
        let (identity, addresses) = worker.join().expect("descriptor reader completed");
        assert_eq!(identity, expected_identity);
        assert_eq!(*retained_addresses.get_or_insert(addresses), addresses);
    }
}

#[test]
fn grouped_array_and_accessor_edges_fail_before_dereference_or_borrow() {
    let backend = successful_backend();
    let fixture = opened(Arc::clone(&backend) as Arc<dyn OcrBackend>);
    let operation = operation();
    let call = |request: &madopilot_ocr_zone_scan_request_t| {
        let mut result = ptr::NonNull::<madopilot_ocr_zone_scan_result_t>::dangling().as_ptr();
        let mut error = ptr::null_mut();
        // SAFETY: the request itself and outputs are live. Each case declares
        // its intentionally invalid nested pointer and must be refused before use.
        let status = unsafe {
            (fixture.api.session_scan_ocr_zones)(
                fixture.session,
                request,
                &operation,
                &mut result,
                &mut error,
            )
        };
        assert!(result.is_null(), "failure publishes no grouped owner");
        if !error.is_null() {
            // SAFETY: the failed call returned one owned error.
            unsafe { (fixture.api.error_release)(error) };
        }
        status
    };

    let zero: [madopilot_ocr_zone_t; 0] = [];
    assert_eq!(
        call(&c_zone_request(fixture.frame, fixture.package, &zero)),
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    let nine = [c_zone(0, 0, 32, 24); 9];
    assert_eq!(
        call(&c_zone_request(fixture.frame, fixture.package, &nine)),
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );

    let one = [c_zone(0, 0, 32, 24)];
    let mut request = c_zone_request(fixture.frame, fixture.package, &one);
    request.flags = 1;
    assert_eq!(call(&request), MADOPILOT_STATUS_INVALID_ARGUMENT);
    assert_eq!(backend.recognition_count(), 0);

    request = c_zone_request(fixture.frame, fixture.package, &one);
    request.frame = ptr::null();
    assert_eq!(call(&request), MADOPILOT_STATUS_INVALID_ARGUMENT);

    request = c_zone_request(fixture.frame, fixture.package, &one);
    request.backend_id = string("wrong-backend");
    assert_eq!(call(&request), MADOPILOT_STATUS_INVALID_ARGUMENT);

    request = c_zone_request(fixture.frame, fixture.package, &one);
    request.zones = ptr::null();
    assert_eq!(call(&request), MADOPILOT_STATUS_INVALID_ARGUMENT);

    request = c_zone_request(fixture.frame, fixture.package, &one);
    request.zone_stride = size_of::<madopilot_ocr_zone_t>() - 1;
    assert_eq!(call(&request), MADOPILOT_STATUS_INVALID_ARGUMENT);

    request = c_zone_request(fixture.frame, fixture.package, &one);
    request.zone_stride = size_of::<madopilot_ocr_zone_t>() + 1;
    assert_eq!(call(&request), MADOPILOT_STATUS_INVALID_ARGUMENT);

    let storage = [0_u8; size_of::<madopilot_ocr_zone_t>() + 1];
    request = c_zone_request(fixture.frame, fixture.package, &one);
    // SAFETY: forming an unaligned raw pointer is allowed; the boundary must
    // reject it before any typed read.
    request.zones = unsafe { storage.as_ptr().add(1) }.cast();
    assert_eq!(call(&request), MADOPILOT_STATUS_INVALID_ARGUMENT);

    request = c_zone_request(fixture.frame, fixture.package, &one);
    request.zone_stride = usize::MAX & !(align_of::<madopilot_ocr_zone_t>() - 1);
    assert_eq!(call(&request), MADOPILOT_STATUS_INVALID_ARGUMENT);

    request = c_zone_request(fixture.frame, fixture.package, &one);
    request.zones = ptr::with_exposed_provenance::<madopilot_ocr_zone_t>(usize::MAX - 15);
    assert_eq!(call(&request), MADOPILOT_STATUS_INVALID_ARGUMENT);

    let mut short = one;
    short[0].struct_size = 31;
    assert_eq!(
        call(&c_zone_request(fixture.frame, fixture.package, &short)),
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    let mut unknown_flags = one;
    unknown_flags[0].flags = 1;
    assert_eq!(
        call(&c_zone_request(
            fixture.frame,
            fixture.package,
            &unknown_flags
        )),
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    assert_eq!(backend.recognition_count(), 0);
    let mut wrong_space = one;
    wrong_space[0].region.space = crate::types::MADOPILOT_SPACE_DESKTOP_LOGICAL;
    assert_eq!(
        call(&c_zone_request(
            fixture.frame,
            fixture.package,
            &wrong_space
        )),
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    let mut wrong_clip = one;
    wrong_clip[0].clip_policy = i32::MAX;
    assert_eq!(
        call(&c_zone_request(fixture.frame, fixture.package, &wrong_clip)),
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );

    let zones = [c_zone(0, 0, 16, 12), c_zone(24, 0, 32, 12)];
    let valid = c_zone_request(fixture.frame, fixture.package, &zones);
    let mut result = ptr::null_mut();
    let mut error = ptr::null_mut();
    // SAFETY: complete valid inputs and writable outputs remain live.
    assert_eq!(
        // SAFETY: complete valid inputs and writable outputs remain live.
        unsafe {
            (fixture.api.session_scan_ocr_zones)(
                fixture.session,
                &valid,
                &operation,
                &mut result,
                &mut error,
            )
        },
        MADOPILOT_STATUS_OK
    );
    assert!(error.is_null());

    let mut zone = madopilot_ocr_zone_result_t {
        struct_size: struct_size::<madopilot_ocr_zone_result_t>(),
        flags: u32::MAX,
        effective_zone: c_zone(0, 0, 1, 1).region,
        reserved: u32::MAX,
        region_count: u64::MAX,
    };
    // SAFETY: result is retained; the invalid index and writable output are intentional.
    assert_eq!(
        // SAFETY: the retained result is live and the output is writable.
        unsafe { (fixture.api.ocr_zone_scan_result_zone_at)(result, 2, &mut zone) },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    assert_eq!(zone.flags, 0);
    assert_eq!(zone.region_count, 0);
    assert_eq!(zone.effective_zone, madopilot_pixel_rect_t::empty());

    let mut region = empty_region();
    region.confidence = 1.0;
    // SAFETY: zone one is an explicit empty group; output is writable.
    assert_eq!(
        // SAFETY: the retained result is live and the output is writable.
        unsafe { (fixture.api.ocr_zone_scan_result_region_at)(result, 1, 0, &mut region) },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    assert_eq!(region.confidence, 0.0);

    let mut text_out = string("sentinel");
    // SAFETY: the empty-group index is invalid and the scalar output is writable.
    assert_eq!(
        // SAFETY: the retained result is live and the scalar output is writable.
        unsafe { (fixture.api.ocr_zone_scan_result_text_at)(result, 1, 0, &mut text_out) },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    assert!(text_out.data.is_null());
    assert_eq!(text_out.len, 0);

    let mut info = empty_zone_info();
    info.zone_count = u64::MAX;
    // SAFETY: null is the intentional invalid owner and the output is writable.
    assert_eq!(
        // SAFETY: null is the intentional owner input and the output is writable.
        unsafe { (fixture.api.ocr_zone_scan_result_info)(ptr::null(), &mut info) },
        MADOPILOT_STATUS_INVALID_ARGUMENT
    );
    assert_eq!(info.zone_count, 0);
    // SAFETY: gives up the one successful grouped-result reference.
    assert_eq!(
        // SAFETY: this gives up the successful result reference.
        unsafe { (fixture.api.ocr_zone_scan_result_release)(result) },
        MADOPILOT_STATUS_OK
    );
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

#[test]
fn grouped_c_terminal_and_malformed_output_paths_publish_no_result() {
    let fixture = opened(successful_backend());
    let zones = [c_zone(0, 0, 32, 24)];
    let request = c_zone_request(fixture.frame, fixture.package, &zones);
    let mut cancellation = ptr::null_mut();
    // SAFETY: the cancellation output is writable.
    assert_eq!(
        // SAFETY: the cancellation output is writable.
        unsafe { (fixture.api.cancellation_create)(&mut cancellation) },
        MADOPILOT_STATUS_OK
    );
    // SAFETY: the fixture owns the live cancellation handle.
    assert_eq!(
        // SAFETY: this fixture owns the live cancellation handle.
        unsafe { (fixture.api.cancellation_cancel)(cancellation) },
        MADOPILOT_STATUS_OK
    );
    let mut cancelled_operation = operation();
    cancelled_operation.cancellation = cancellation;
    let mut result = ptr::NonNull::<madopilot_ocr_zone_scan_result_t>::dangling().as_ptr();
    let mut error = ptr::null_mut();
    // SAFETY: complete inputs and outputs remain live; cancellation is authoritative.
    assert_eq!(
        // SAFETY: complete inputs and outputs remain live for the cancelled call.
        unsafe {
            (fixture.api.session_scan_ocr_zones)(
                fixture.session,
                &request,
                &cancelled_operation,
                &mut result,
                &mut error,
            )
        },
        crate::status::MADOPILOT_STATUS_CANCELLED
    );
    assert!(result.is_null());
    assert!(!error.is_null());
    // SAFETY: both returned/created handles are owned here.
    unsafe {
        (fixture.api.error_release)(error);
        (fixture.api.cancellation_release)(cancellation);
    }

    let mut expired = operation();
    expired.flags = crate::types::MADOPILOT_OPERATION_HAS_DEADLINE;
    expired.deadline_nanos = 0;
    result = ptr::NonNull::<madopilot_ocr_zone_scan_result_t>::dangling().as_ptr();
    error = ptr::null_mut();
    // SAFETY: complete inputs and outputs remain live; the origin deadline is expired.
    assert_eq!(
        // SAFETY: complete inputs and outputs remain live for the expired call.
        unsafe {
            (fixture.api.session_scan_ocr_zones)(
                fixture.session,
                &request,
                &expired,
                &mut result,
                &mut error,
            )
        },
        crate::status::MADOPILOT_STATUS_DEADLINE_EXCEEDED
    );
    assert!(result.is_null());
    assert!(!error.is_null());
    // SAFETY: the failed call returned one owned error.
    unsafe { (fixture.api.error_release)(error) };

    let malformed = Arc::new(
        ControlledOcr::new(PixelFormat::Rgba8)
            .with_descriptor(descriptor())
            .with_candidates(vec![ScriptedOcrCandidate::new(
                Arc::<[u8]>::from([0xff_u8].as_slice()),
                [(1.0, 1.0), (8.0, 1.0), (8.0, 6.0), (1.0, 6.0)],
                0.8,
                0,
            )]),
    );
    let malformed_fixture = opened(malformed);
    let request = c_zone_request(malformed_fixture.frame, malformed_fixture.package, &zones);
    result = ptr::NonNull::<madopilot_ocr_zone_scan_result_t>::dangling().as_ptr();
    error = ptr::null_mut();
    // SAFETY: complete inputs and outputs remain live; malformed UTF-8 is intentional.
    assert_eq!(
        // SAFETY: complete inputs and outputs remain live; malformed data is intentional.
        unsafe {
            (malformed_fixture.api.session_scan_ocr_zones)(
                malformed_fixture.session,
                &request,
                &operation(),
                &mut result,
                &mut error,
            )
        },
        crate::status::MADOPILOT_STATUS_VISION_FAILED
    );
    assert!(result.is_null());
    assert!(!error.is_null());
    // SAFETY: the failed call returned one owned error.
    unsafe { (malformed_fixture.api.error_release)(error) };
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

    let zones = [c_zone(0, 0, 32, 24)];
    let request = c_zone_request(fixture.frame, fixture.package, &zones);
    let mut grouped = usize::MAX as *mut madopilot_ocr_zone_scan_result_t;
    error = usize::MAX as *mut madopilot_error_t;
    // SAFETY: all C arguments and the zone array are valid; the panic is deliberate.
    let status = unsafe {
        (fixture.api.session_scan_ocr_zones)(
            fixture.session,
            &request,
            &operation,
            &mut grouped,
            &mut error,
        )
    };
    assert_eq!(status, MADOPILOT_STATUS_INTERNAL_PANIC);
    assert!(grouped.is_null());
    assert!(error.is_null());
}

#[test]
fn concurrent_close_wins_before_grouped_c_result_publication() {
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
    let _gate_release = gate.release_guard();
    let api = fixture.api;
    let session = fixture.session as usize;
    let frame = fixture.frame as usize;
    let package = fixture.package as usize;

    let worker = thread::spawn(move || {
        let operation = operation();
        let zones = [c_zone(0, 0, 32, 24)];
        let request = c_zone_request(
            frame as *const crate::capture::madopilot_frame_t,
            package as *const crate::assets::madopilot_package_t,
            &zones,
        );
        let mut result = ptr::null_mut();
        let mut error = ptr::null_mut();
        // SAFETY: the fixture remains alive until this worker joins.
        let status = unsafe {
            (api.session_scan_ocr_zones)(
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
