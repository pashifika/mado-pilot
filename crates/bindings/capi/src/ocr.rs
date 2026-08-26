//! One-shot OCR execution and immutable owned result projection.
//!
//! A request borrows its session, exact frame, backend/model identity views, and
//! operation context for one synchronous call. An integrated accepted profile
//! borrows the model identity already retained by the engine and does not load
//! model bytes twice. A successful result owns only the facade's immutable
//! `OcrResult`; it retains no parent handle, frame storage, package bytes, backend
//! buffer, lock, callback, or worker.
//! Text returned by `ocr_result_text_at` is borrowed from that result owner.

use std::mem::{align_of, size_of};

use mado_pilot::{
    AssetPackage, Error, MAX_OCR_ZONES, OcrExecutionProvider, OcrExecutionProviderPolicy, OcrFault,
    OcrProviderFallbackReason, OcrRegion, OcrRequest, OcrResult, OcrZone, OcrZoneScanRequest,
    OcrZoneScanResult, Status,
};

use crate::boundary::{self, Input, Out, Versioned, covers, declared, inputs, prefixes};
use crate::capture::{FrameHandle, SessionHandle, madopilot_session_t, rect, source_rect, stamp};
use crate::engine::{EngineHandle, madopilot_engine_t};
use crate::error::{Fault, madopilot_error_t};
use crate::handle::opaque;
use crate::operation;
use crate::status::{
    MADOPILOT_ERROR_CATEGORY_VISION, MADOPILOT_STATUS_INVALID_ARGUMENT, MADOPILOT_STATUS_OK,
    MADOPILOT_STATUS_UNSUPPORTED, madopilot_status_t,
};
use crate::types::{
    MADOPILOT_CLIP_POLICY_REJECT, MADOPILOT_OCR_EXECUTION_PROVIDER_COREML,
    MADOPILOT_OCR_EXECUTION_PROVIDER_CPU, MADOPILOT_OCR_EXECUTION_PROVIDER_CUDA,
    MADOPILOT_OCR_HAS_REGION, MADOPILOT_OCR_PROVIDER_DESCRIPTOR_HAS_FALLBACK,
    MADOPILOT_OCR_PROVIDER_FALLBACK_BUILD_CAPABILITY_UNAVAILABLE,
    MADOPILOT_OCR_PROVIDER_FALLBACK_DEPENDENCY_UNAVAILABLE,
    MADOPILOT_OCR_PROVIDER_FALLBACK_GRAPH_REJECTED, MADOPILOT_OCR_PROVIDER_FALLBACK_NONE,
    MADOPILOT_OCR_PROVIDER_FALLBACK_PROVIDER_UNAVAILABLE,
    MADOPILOT_OCR_PROVIDER_FALLBACK_QUALIFICATION_REJECTED,
    MADOPILOT_OCR_PROVIDER_FALLBACK_REGISTRATION_FAILED,
    MADOPILOT_OCR_PROVIDER_FALLBACK_SESSION_CREATION_FAILED,
    MADOPILOT_OCR_PROVIDER_FALLBACK_UNSUPPORTED_TARGET,
    MADOPILOT_OCR_PROVIDER_POLICY_AUTO_PREFER_ACCELERATOR, MADOPILOT_OCR_PROVIDER_POLICY_CPU,
    MADOPILOT_OCR_PROVIDER_POLICY_PREFER_COREML, MADOPILOT_OCR_PROVIDER_POLICY_PREFER_CUDA,
    MADOPILOT_OCR_PROVIDER_POLICY_REQUIRE_COREML, MADOPILOT_OCR_PROVIDER_POLICY_REQUIRE_CUDA,
    MADOPILOT_SPACE_CAPTURE_PIXELS, clip_policy, madopilot_frame_stamp_t,
    madopilot_ocr_engine_descriptor_t, madopilot_ocr_point_t, madopilot_ocr_provider_descriptor_t,
    madopilot_ocr_provider_fallback_reason_t, madopilot_ocr_provider_policy_t,
    madopilot_ocr_region_t, madopilot_ocr_request_t, madopilot_ocr_result_info_t,
    madopilot_ocr_zone_result_t, madopilot_ocr_zone_scan_request_t,
    madopilot_ocr_zone_scan_result_info_t, madopilot_ocr_zone_t, madopilot_operation_t,
    madopilot_pixel_rect_t, space, space_code,
};
use crate::view::{self, madopilot_str_t};
use crate::{handle, hooks};

opaque! {
    /// One immutable source-correlated OCR result.
    ///
    /// Independently retained from its frame, package, session, engine, and backend.
    madopilot_ocr_result_t => OcrResultHandle
}

opaque! {
    /// One immutable caller-grouped OCR result.
    ///
    /// Independently retained from every caller array and parent handle.
    madopilot_ocr_zone_scan_result_t => OcrZoneScanResultHandle
}

#[derive(Debug)]
pub(crate) struct OcrResultHandle {
    result: OcrResult,
}

#[derive(Debug)]
pub(crate) struct OcrZoneScanResultHandle {
    result: OcrZoneScanResult,
}

fn is_integrated_profile(selected: &mado_pilot::OcrBackendDescriptor) -> bool {
    let backend_id = selected.id().as_str();
    let integrated_backend = (backend_id == mado_pilot::DEFAULT_OCR_BACKEND_ID
        || backend_id == mado_pilot::CUDA_OCR_BACKEND_ID
        || backend_id == mado_pilot::COREML_OCR_BACKEND_ID)
        && selected.version().as_str() == mado_pilot::DEFAULT_OCR_BACKEND_VERSION;
    let integrated_profile = (selected.model().as_str() == mado_pilot::ACCEPTED_G004_MODEL_ID
        && selected.profile().as_str() == mado_pilot::ACCEPTED_G004_PROFILE_ID)
        || (selected.model().as_str() == mado_pilot::ACCEPTED_BOUNDED_MODEL_ID
            && selected.profile().as_str() == mado_pilot::ACCEPTED_BOUNDED_PROFILE_ID);
    integrated_backend && integrated_profile
}

inputs! {
    impl Input for madopilot_ocr_request_t {
        const MANDATORY: usize = covers!(madopilot_ocr_request_t, output_space: crate::types::madopilot_space_t);
        const NAME: &'static str = "madopilot_ocr_request_t";
        const PREFIXES: &'static [usize] = prefixes!(
            madopilot_ocr_request_t,
            struct_size,
            flags,
            frame,
            package,
            model_id,
            backend_id,
            backend_version,
            output_space,
            clip_policy,
            region,
        );
        const PRESENCE: &'static [(u32, usize)] = &[(
            MADOPILOT_OCR_HAS_REGION,
            size_of::<madopilot_ocr_request_t>(),
        )];
        fn defaults() -> Self {
            Self {
                struct_size: 0,
                flags: 0,
                frame: std::ptr::null(),
                package: std::ptr::null(),
                model_id: madopilot_str_t::empty(),
                backend_id: madopilot_str_t::empty(),
                backend_version: madopilot_str_t::empty(),
                output_space: MADOPILOT_SPACE_CAPTURE_PIXELS,
                clip_policy: MADOPILOT_CLIP_POLICY_REJECT,
                region: madopilot_pixel_rect_t::empty(),
            }
        }

        fn presence_bits(&self) -> u32 {
            self.flags
        }
    }

    impl Input for madopilot_ocr_zone_t {
        const MANDATORY: usize = 32;
        const NAME: &'static str = "madopilot_ocr_zone_t";
        const PREFIXES: &'static [usize] = prefixes!(
            madopilot_ocr_zone_t,
            struct_size,
            flags,
            region,
            clip_policy,
        );
        const PRESENCE: &'static [(u32, usize)] = &[];

        fn defaults() -> Self {
            Self {
                struct_size: 0,
                flags: 0,
                region: madopilot_pixel_rect_t::empty(),
                clip_policy: MADOPILOT_CLIP_POLICY_REJECT,
            }
        }

        fn presence_bits(&self) -> u32 {
            self.flags
        }
    }

    impl Input for madopilot_ocr_zone_scan_request_t {
        const MANDATORY: usize = 104;
        const NAME: &'static str = "madopilot_ocr_zone_scan_request_t";
        const PREFIXES: &'static [usize] = prefixes!(
            madopilot_ocr_zone_scan_request_t,
            struct_size,
            flags,
            frame,
            package,
            model_id,
            backend_id,
            backend_version,
            output_space,
            reserved,
            zones,
            zone_count,
            zone_stride,
        );
        const PRESENCE: &'static [(u32, usize)] = &[];

        fn defaults() -> Self {
            Self {
                struct_size: 0,
                flags: 0,
                frame: std::ptr::null(),
                package: std::ptr::null(),
                model_id: madopilot_str_t::empty(),
                backend_id: madopilot_str_t::empty(),
                backend_version: madopilot_str_t::empty(),
                output_space: MADOPILOT_SPACE_CAPTURE_PIXELS,
                reserved: 0,
                zones: std::ptr::null(),
                zone_count: 0,
                zone_stride: 0,
            }
        }

        fn presence_bits(&self) -> u32 {
            self.flags
        }
    }
}

impl Versioned for madopilot_ocr_engine_descriptor_t {
    const MANDATORY: usize = 88;
    const NAME: &'static str = "madopilot_ocr_engine_descriptor_t";
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_ocr_engine_descriptor_t,
        struct_size,
        flags,
        backend_id,
        backend_version,
        model_id,
        model_version,
        profile_id,
    );
    const ZEROED_PADDING: &'static [(usize, usize)] = &[];

    fn failure(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            backend_id: madopilot_str_t::empty(),
            backend_version: madopilot_str_t::empty(),
            model_id: madopilot_str_t::empty(),
            model_version: madopilot_str_t::empty(),
            profile_id: madopilot_str_t::empty(),
        }
    }
}

impl Versioned for madopilot_ocr_provider_descriptor_t {
    const MANDATORY: usize = 40;
    const NAME: &'static str = "madopilot_ocr_provider_descriptor_t";
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_ocr_provider_descriptor_t,
        struct_size,
        flags,
        requested_policy,
        active_provider,
        initialization_fell_back,
        fallback_reason,
        runtime_profile,
    );
    const ZEROED_PADDING: &'static [(usize, usize)] = &[];

    fn failure(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            requested_policy: 0,
            active_provider: crate::types::MADOPILOT_OCR_EXECUTION_PROVIDER_UNSPECIFIED,
            initialization_fell_back: 0,
            fallback_reason: MADOPILOT_OCR_PROVIDER_FALLBACK_NONE,
            runtime_profile: madopilot_str_t::empty(),
        }
    }
}
impl Versioned for madopilot_ocr_result_info_t {
    const MANDATORY: usize = 168;
    const NAME: &'static str = "madopilot_ocr_result_info_t";
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_ocr_result_info_t,
        struct_size,
        flags,
        source,
        effective_region,
        output_space,
        reserved,
        region_count,
        backend_id,
        backend_version,
        model_id,
        model_version,
        profile_id,
    );
    const ZEROED_PADDING: &'static [(usize, usize)] = &[(
        covers!(madopilot_ocr_result_info_t, reserved: u32),
        std::mem::offset_of!(madopilot_ocr_result_info_t, region_count),
    )];

    fn failure(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            source: madopilot_frame_stamp_t::cleared(
                u32::try_from(size_of::<madopilot_frame_stamp_t>())
                    .expect("frame stamp is smaller than 4 GiB"),
            ),
            effective_region: madopilot_pixel_rect_t::empty(),
            output_space: MADOPILOT_SPACE_CAPTURE_PIXELS,
            reserved: 0,
            region_count: 0,
            backend_id: madopilot_str_t::empty(),
            backend_version: madopilot_str_t::empty(),
            model_id: madopilot_str_t::empty(),
            model_version: madopilot_str_t::empty(),
            profile_id: madopilot_str_t::empty(),
        }
    }
}

impl Versioned for madopilot_ocr_zone_scan_result_info_t {
    const MANDATORY: usize = 176;
    const NAME: &'static str = "madopilot_ocr_zone_scan_result_info_t";
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_ocr_zone_scan_result_info_t,
        struct_size,
        flags,
        source,
        source_envelope,
        output_space,
        zone_count,
        unique_candidate_count,
        membership_count,
        backend_id,
        backend_version,
        model_id,
        model_version,
        profile_id,
    );
    const ZEROED_PADDING: &'static [(usize, usize)] = &[];

    fn failure(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            source: madopilot_frame_stamp_t::cleared(
                u32::try_from(size_of::<madopilot_frame_stamp_t>())
                    .expect("frame stamp is smaller than 4 GiB"),
            ),
            source_envelope: madopilot_pixel_rect_t::empty(),
            output_space: MADOPILOT_SPACE_CAPTURE_PIXELS,
            zone_count: 0,
            unique_candidate_count: 0,
            membership_count: 0,
            backend_id: madopilot_str_t::empty(),
            backend_version: madopilot_str_t::empty(),
            model_id: madopilot_str_t::empty(),
            model_version: madopilot_str_t::empty(),
            profile_id: madopilot_str_t::empty(),
        }
    }
}

impl Versioned for madopilot_ocr_zone_result_t {
    const MANDATORY: usize = 40;
    const NAME: &'static str = "madopilot_ocr_zone_result_t";
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_ocr_zone_result_t,
        struct_size,
        flags,
        effective_zone,
        reserved,
        region_count,
    );
    const ZEROED_PADDING: &'static [(usize, usize)] = &[];

    fn failure(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            effective_zone: madopilot_pixel_rect_t::empty(),
            reserved: 0,
            region_count: 0,
        }
    }
}
impl Versioned for madopilot_ocr_region_t {
    const MANDATORY: usize = 80;
    const NAME: &'static str = "madopilot_ocr_region_t";
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_ocr_region_t,
        struct_size,
        flags,
        confidence,
        points,
    );
    const ZEROED_PADDING: &'static [(usize, usize)] = &[];

    fn failure(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            confidence: 0.0,
            points: [madopilot_ocr_point_t { x: 0.0, y: 0.0 }; 4],
        }
    }
}

pub(crate) fn engine_ocr_descriptor(
    engine: *const madopilot_engine_t,
    out_descriptor: *mut madopilot_ocr_engine_descriptor_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable output structure whose
    // `struct_size` it has set.
    let out = match unsafe { Out::begin(out_descriptor) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);
    // SAFETY: the caller keeps the engine retained for the call.
    let Some(engine) = (unsafe { handle::borrow::<EngineHandle>(engine) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    let Some(selected) = engine.retained_ocr_backend() else {
        return MADOPILOT_STATUS_UNSUPPORTED;
    };
    let value = madopilot_ocr_engine_descriptor_t {
        struct_size: out.declared_size(),
        flags: 0,
        backend_id: madopilot_str_t::borrowed(selected.id().as_str()),
        backend_version: madopilot_str_t::borrowed(selected.version().as_str()),
        model_id: madopilot_str_t::borrowed(selected.model().as_str()),
        model_version: madopilot_str_t::borrowed(selected.model_identity().version().as_str()),
        profile_id: madopilot_str_t::borrowed(selected.profile().as_str()),
    };
    // SAFETY: `out` was validated; every view borrows the retained engine handle.
    unsafe { out.commit(value) };
    MADOPILOT_STATUS_OK
}

const fn provider_policy_code(
    policy: OcrExecutionProviderPolicy,
) -> madopilot_ocr_provider_policy_t {
    match policy {
        OcrExecutionProviderPolicy::Cpu => MADOPILOT_OCR_PROVIDER_POLICY_CPU,
        OcrExecutionProviderPolicy::AutoPreferAccelerator => {
            MADOPILOT_OCR_PROVIDER_POLICY_AUTO_PREFER_ACCELERATOR
        }
        OcrExecutionProviderPolicy::PreferCuda => MADOPILOT_OCR_PROVIDER_POLICY_PREFER_CUDA,
        OcrExecutionProviderPolicy::RequireCuda => MADOPILOT_OCR_PROVIDER_POLICY_REQUIRE_CUDA,
        OcrExecutionProviderPolicy::PreferCoreMl => MADOPILOT_OCR_PROVIDER_POLICY_PREFER_COREML,
        OcrExecutionProviderPolicy::RequireCoreMl => MADOPILOT_OCR_PROVIDER_POLICY_REQUIRE_COREML,
    }
}

const fn execution_provider_code(provider: OcrExecutionProvider) -> i32 {
    match provider {
        OcrExecutionProvider::Cpu => MADOPILOT_OCR_EXECUTION_PROVIDER_CPU,
        OcrExecutionProvider::Cuda => MADOPILOT_OCR_EXECUTION_PROVIDER_CUDA,
        OcrExecutionProvider::CoreMl => MADOPILOT_OCR_EXECUTION_PROVIDER_COREML,
    }
}

const fn fallback_reason_code(
    reason: OcrProviderFallbackReason,
) -> madopilot_ocr_provider_fallback_reason_t {
    match reason {
        OcrProviderFallbackReason::UnsupportedTarget => {
            MADOPILOT_OCR_PROVIDER_FALLBACK_UNSUPPORTED_TARGET
        }
        OcrProviderFallbackReason::BuildCapabilityUnavailable => {
            MADOPILOT_OCR_PROVIDER_FALLBACK_BUILD_CAPABILITY_UNAVAILABLE
        }
        OcrProviderFallbackReason::ProviderUnavailable => {
            MADOPILOT_OCR_PROVIDER_FALLBACK_PROVIDER_UNAVAILABLE
        }
        OcrProviderFallbackReason::DependencyUnavailable => {
            MADOPILOT_OCR_PROVIDER_FALLBACK_DEPENDENCY_UNAVAILABLE
        }
        OcrProviderFallbackReason::RegistrationFailed => {
            MADOPILOT_OCR_PROVIDER_FALLBACK_REGISTRATION_FAILED
        }
        OcrProviderFallbackReason::SessionCreationFailed => {
            MADOPILOT_OCR_PROVIDER_FALLBACK_SESSION_CREATION_FAILED
        }
        OcrProviderFallbackReason::GraphRejected => MADOPILOT_OCR_PROVIDER_FALLBACK_GRAPH_REJECTED,
        OcrProviderFallbackReason::QualificationRejected => {
            MADOPILOT_OCR_PROVIDER_FALLBACK_QUALIFICATION_REJECTED
        }
    }
}

pub(crate) fn engine_ocr_provider_descriptor(
    engine: *const madopilot_engine_t,
    out_descriptor: *mut madopilot_ocr_provider_descriptor_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable output structure whose
    // `struct_size` it has set.
    let out = match unsafe { Out::begin(out_descriptor) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);
    // SAFETY: the caller keeps the engine retained for the call.
    let Some(engine) = (unsafe { handle::borrow::<EngineHandle>(engine) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    let Some(selected) = engine.retained_ocr_provider() else {
        return MADOPILOT_STATUS_UNSUPPORTED;
    };
    let fallback = selected.fallback_reason();
    let value = madopilot_ocr_provider_descriptor_t {
        struct_size: out.declared_size(),
        flags: if fallback.is_some() {
            MADOPILOT_OCR_PROVIDER_DESCRIPTOR_HAS_FALLBACK
        } else {
            0
        },
        requested_policy: provider_policy_code(selected.requested_policy()),
        active_provider: execution_provider_code(selected.active_provider()),
        initialization_fell_back: u32::from(selected.initialization_fell_back()),
        fallback_reason: fallback
            .map_or(MADOPILOT_OCR_PROVIDER_FALLBACK_NONE, fallback_reason_code),
        runtime_profile: madopilot_str_t::borrowed(selected.runtime_profile().as_str()),
    };
    // SAFETY: `out` was validated; the view borrows the retained engine handle.
    unsafe { out.commit(value) };
    MADOPILOT_STATUS_OK
}

pub(crate) fn session_recognize(
    session: *const madopilot_session_t,
    request: *const madopilot_ocr_request_t,
    operation: *const madopilot_operation_t,
    out_result: *mut *mut madopilot_ocr_result_t,
    out_error: *mut *mut madopilot_error_t,
) -> madopilot_status_t {
    if let Err(status) =
        // SAFETY: the caller supplies writable, correctly aligned output addresses.
        unsafe { boundary::begin_outputs(out_result, "out_result", out_error) }
    {
        return status;
    }
    hooks::reach(hooks::Site::Entry);

    // SAFETY: `out_error` was validated above.
    unsafe {
        crate::engine::report(
            out_error,
            run_session_recognize(session, request, operation, out_result),
        )
    }
}

fn run_session_recognize(
    session: *const madopilot_session_t,
    request: *const madopilot_ocr_request_t,
    operation: *const madopilot_operation_t,
    out_result: *mut *mut madopilot_ocr_result_t,
) -> Result<(), Fault> {
    // SAFETY: every handle is retained by the caller for the whole call.
    let Some(session) = (unsafe { handle::borrow::<SessionHandle>(session) }) else {
        return Err(Fault::abi("`session` is null"));
    };
    // SAFETY: the request remains readable and unmodified for the call.
    let request = unsafe { boundary::read_input::<madopilot_ocr_request_t>(request) }?;
    // SAFETY: the operation, when non-null, remains readable for the call.
    let context = unsafe { operation::context(operation) }?;
    context.admit()?;

    // SAFETY: required handles remain retained for the call.
    let Some(frame) = (unsafe { handle::borrow::<FrameHandle>(request.frame) }) else {
        return Err(Fault::abi("`frame` is null"));
    };
    // Validate every pointer-length input before resolving model or invoking work.
    // SAFETY: the caller keeps these views readable and unmodified for the call.
    let model_id = unsafe { view::non_empty_string(request.model_id, "model_id") }?;
    // SAFETY: as above.
    let backend_id = unsafe { view::non_empty_string(request.backend_id, "backend_id") }?;
    // SAFETY: as above.
    let backend_version =
        unsafe { view::non_empty_string(request.backend_version, "backend_version") }?;

    let Some(selected) = session.ocr_backend() else {
        return Err(ocr_fault(OcrFault::BackendUnavailable));
    };
    if backend_id != selected.id().as_str() || backend_version != selected.version().as_str() {
        return Err(ocr_fault(OcrFault::BackendMismatch));
    }

    let package_model = if request.package.is_null() {
        if !is_integrated_profile(selected) || model_id != selected.model().as_str() {
            return Err(ocr_fault(OcrFault::ModelMismatch));
        }
        None
    } else {
        // SAFETY: the caller retains the non-null package handle for the call.
        let Some(package) = (unsafe { handle::borrow::<AssetPackage>(request.package) }) else {
            return Err(Fault::abi("`package` is null"));
        };
        Some(
            package
                .resolve_ocr_model(model_id)
                .map_err(Fault::from_asset)?,
        )
    };
    let model = package_model
        .as_ref()
        .map_or(selected.model_identity(), |source| source.identity());
    let region = if declared!(request, madopilot_ocr_request_t, MADOPILOT_OCR_HAS_REGION) {
        OcrRegion::Region {
            rect: source_rect(request.region)?,
            policy: clip_policy(request.clip_policy)?,
        }
    } else {
        OcrRegion::FullFrame
    };
    let output_space = space(request.output_space)?;

    let result = session
        .session()
        .recognize(OcrRequest::new(
            frame.frame(),
            selected.backend_identity(),
            model,
            region,
            output_space,
            context.inner(),
        ))
        .map_err(|error| recognition_failure(&error, selected.id().as_str()))?;
    hooks::reach(hooks::Site::AfterTemporary);
    context.commit()?;

    // SAFETY: `out_result` was initialized and validated before any input or work.
    unsafe { out_result.write(handle::into_raw(OcrResultHandle { result })) };
    Ok(())
}

pub(crate) fn session_scan_ocr_zones(
    session: *const madopilot_session_t,
    request: *const madopilot_ocr_zone_scan_request_t,
    operation: *const madopilot_operation_t,
    out_result: *mut *mut madopilot_ocr_zone_scan_result_t,
    out_error: *mut *mut madopilot_error_t,
) -> madopilot_status_t {
    if let Err(status) =
        // SAFETY: the caller supplies writable, correctly aligned output addresses.
        unsafe { boundary::begin_outputs(out_result, "out_result", out_error) }
    {
        return status;
    }
    hooks::reach(hooks::Site::Entry);

    // SAFETY: `out_error` was validated above.
    unsafe {
        crate::engine::report(
            out_error,
            run_session_scan_ocr_zones(session, request, operation, out_result),
        )
    }
}

fn run_session_scan_ocr_zones(
    session: *const madopilot_session_t,
    request: *const madopilot_ocr_zone_scan_request_t,
    operation: *const madopilot_operation_t,
    out_result: *mut *mut madopilot_ocr_zone_scan_result_t,
) -> Result<(), Fault> {
    // SAFETY: every handle is retained by the caller for the whole call.
    let Some(session) = (unsafe { handle::borrow::<SessionHandle>(session) }) else {
        return Err(Fault::abi("`session` is null"));
    };
    // SAFETY: the request remains readable and unmodified for the call.
    let request = unsafe { boundary::read_input::<madopilot_ocr_zone_scan_request_t>(request) }?;
    if request.flags != 0 {
        return Err(Fault::abi(format!(
            "madopilot_ocr_zone_scan_request_t sets unknown flags {:#x}",
            request.flags
        )));
    }
    if request.reserved != 0 {
        return Err(Fault::abi(
            "madopilot_ocr_zone_scan_request_t reserved must be zero",
        ));
    }
    // SAFETY: the operation, when non-null, remains readable for the call.
    let context = unsafe { operation::context(operation) }?;
    context.admit()?;

    // SAFETY: required handles remain retained for the call.
    let Some(frame) = (unsafe { handle::borrow::<FrameHandle>(request.frame) }) else {
        return Err(Fault::abi("`frame` is null"));
    };
    // SAFETY: the caller keeps these views readable and unmodified for the call.
    let model_id = unsafe { view::non_empty_string(request.model_id, "model_id") }?;
    // SAFETY: as above.
    let backend_id = unsafe { view::non_empty_string(request.backend_id, "backend_id") }?;
    // SAFETY: as above.
    let backend_version =
        unsafe { view::non_empty_string(request.backend_version, "backend_version") }?;

    let Some(selected) = session.ocr_backend() else {
        return Err(ocr_fault(OcrFault::BackendUnavailable));
    };
    if backend_id != selected.id().as_str() || backend_version != selected.version().as_str() {
        return Err(ocr_fault(OcrFault::BackendMismatch));
    }

    let package_model = if request.package.is_null() {
        if !is_integrated_profile(selected) || model_id != selected.model().as_str() {
            return Err(ocr_fault(OcrFault::ModelMismatch));
        }
        None
    } else {
        // SAFETY: the caller retains the non-null package handle for the call.
        let Some(package) = (unsafe { handle::borrow::<AssetPackage>(request.package) }) else {
            return Err(Fault::abi("`package` is null"));
        };
        Some(
            package
                .resolve_ocr_model(model_id)
                .map_err(Fault::from_asset)?,
        )
    };
    let model = package_model
        .as_ref()
        .map_or(selected.model_identity(), |source| source.identity());
    let output_space = space(request.output_space)?;
    let zones = read_zones(&request)?;

    let result = session
        .session()
        .scan_ocr_zones(
            OcrZoneScanRequest::new(
                frame.frame(),
                selected.backend_identity(),
                model,
                &zones,
                output_space,
                context.inner(),
            )
            .map_err(|error| recognition_failure(&error, selected.id().as_str()))?,
        )
        .map_err(|error| recognition_failure(&error, selected.id().as_str()))?;
    hooks::reach(hooks::Site::AfterTemporary);
    context.commit()?;

    // SAFETY: `out_result` was initialized and validated before any input or work.
    unsafe { out_result.write(handle::into_raw(OcrZoneScanResultHandle { result })) };
    Ok(())
}

fn read_zones(request: &madopilot_ocr_zone_scan_request_t) -> Result<Vec<OcrZone>, Fault> {
    if !(1..=MAX_OCR_ZONES).contains(&request.zone_count) {
        return Err(ocr_fault(OcrFault::ZoneCountOutOfRange));
    }
    if request.zones.is_null() {
        return Err(Fault::abi(format!(
            "`zones` is null with a count of {}",
            request.zone_count
        )));
    }
    let alignment = align_of::<madopilot_ocr_zone_t>();
    if !request.zones.addr().is_multiple_of(alignment) {
        return Err(Fault::abi("`zones` is not correctly aligned"));
    }
    if !request.zone_stride.is_multiple_of(alignment) {
        return Err(Fault::abi(format!(
            "`zones` declares a {} byte stride, which is not a multiple of its {alignment} byte alignment",
            request.zone_stride
        )));
    }
    let span = boundary::span(
        request.zone_count,
        request.zone_stride,
        madopilot_ocr_zone_t::MANDATORY,
        "zones",
    )?;
    request
        .zones
        .addr()
        .checked_add(span)
        .ok_or_else(|| Fault::abi("`zones` final address overflows"))?;

    let mut zones = Vec::with_capacity(request.zone_count);
    for index in 0..request.zone_count {
        let offset = index
            .checked_mul(request.zone_stride)
            .ok_or_else(|| Fault::abi("`zones` element address overflows"))?;
        // SAFETY: the checked complete span proves this byte offset remains in
        // the caller's array object and preserves the base pointer's provenance.
        let element = unsafe {
            request
                .zones
                .cast::<u8>()
                .add(offset)
                .cast::<madopilot_ocr_zone_t>()
        };
        // SAFETY: the checked span/address arithmetic stays inside the caller's
        // readable array; `read_element` validates alignment and declared size.
        let zone = unsafe {
            boundary::read_element::<madopilot_ocr_zone_t>(element, request.zone_stride)
        }?;
        if zone.flags != 0 {
            return Err(Fault::abi(format!(
                "OCR zone {index} sets unknown flags {:#x}",
                zone.flags
            )));
        }
        zones.push(OcrZone::new(
            source_rect(zone.region)?,
            clip_policy(zone.clip_policy)?,
        ));
    }
    Ok(zones)
}

fn ocr_fault(fault: OcrFault) -> Fault {
    Fault::from_error(&Error::from(fault), MADOPILOT_ERROR_CATEGORY_VISION)
}

fn recognition_failure(error: &Error, backend: &str) -> Fault {
    if error.status() == Status::Closed {
        return Fault::closed("the session has closed and starts no further OCR work");
    }
    Fault::from_error(error, MADOPILOT_ERROR_CATEGORY_VISION).with_backend(backend)
}

pub(crate) fn result_retain(result: *const madopilot_ocr_result_t) -> madopilot_status_t {
    // SAFETY: null is a no-op; otherwise the caller owns a live reference.
    unsafe { handle::retain::<OcrResultHandle>(result) };
    MADOPILOT_STATUS_OK
}

pub(crate) fn result_release(result: *mut madopilot_ocr_result_t) -> madopilot_status_t {
    // SAFETY: null is a no-op; otherwise the caller gives up one live reference.
    unsafe { handle::release::<OcrResultHandle>(result) };
    MADOPILOT_STATUS_OK
}

pub(crate) fn result_info(
    result: *const madopilot_ocr_result_t,
    out_info: *mut madopilot_ocr_result_info_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable size-versioned output.
    let out = match unsafe { Out::begin(out_info) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);
    // SAFETY: the caller keeps the result retained for the call.
    let Some(result) = (unsafe { handle::borrow::<OcrResultHandle>(result) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };

    let value = &result.result;
    let backend = value.backend();
    let model = backend.model_identity();
    let region_count =
        u64::try_from(value.regions().len()).expect("OCR result count is bounded below u64::MAX");
    // SAFETY: `out` was validated; every string borrows from the retained result.
    unsafe {
        out.commit(madopilot_ocr_result_info_t {
            struct_size: out.declared_size(),
            flags: 0,
            source: stamp(
                value.stamp(),
                u32::try_from(size_of::<madopilot_frame_stamp_t>())
                    .expect("frame stamp is smaller than 4 GiB"),
            ),
            effective_region: rect(value.effective_region()),
            output_space: space_code(value.output_space()),
            reserved: 0,
            region_count,
            backend_id: madopilot_str_t::borrowed(backend.id().as_str()),
            backend_version: madopilot_str_t::borrowed(backend.version().as_str()),
            model_id: madopilot_str_t::borrowed(model.model().as_str()),
            model_version: madopilot_str_t::borrowed(model.version().as_str()),
            profile_id: madopilot_str_t::borrowed(model.profile().as_str()),
        });
    }
    MADOPILOT_STATUS_OK
}

pub(crate) fn result_region_at(
    result: *const madopilot_ocr_result_t,
    index: usize,
    out_region: *mut madopilot_ocr_region_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable size-versioned output.
    let out = match unsafe { Out::begin(out_region) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);
    // SAFETY: the caller keeps the result retained for the call.
    let Some(result) = (unsafe { handle::borrow::<OcrResultHandle>(result) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    let index = match boundary::index_within(index, result.result.regions().len(), "OCR region") {
        Ok(index) => index,
        Err(fault) => return fault.status(),
    };
    let region = &result.result.regions()[index];
    let points = region
        .geometry()
        .points()
        .map(|point| madopilot_ocr_point_t {
            x: point.x(),
            y: point.y(),
        });
    // SAFETY: `out` was validated above.
    unsafe {
        out.commit(madopilot_ocr_region_t {
            struct_size: out.declared_size(),
            flags: 0,
            confidence: region.confidence().get(),
            points,
        });
    }
    MADOPILOT_STATUS_OK
}

pub(crate) fn result_text_at(
    result: *const madopilot_ocr_result_t,
    index: usize,
    out_text: *mut madopilot_str_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable, correctly aligned scalar output.
    let prepared =
        unsafe { boundary::begin_scalar_out(out_text, "out_text", madopilot_str_t::empty()) };
    if let Err(fault) = prepared {
        return fault.status();
    }
    hooks::reach(hooks::Site::Entry);
    // SAFETY: the caller keeps the result retained for the call.
    let Some(result) = (unsafe { handle::borrow::<OcrResultHandle>(result) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    let index = match boundary::index_within(index, result.result.regions().len(), "OCR text") {
        Ok(index) => index,
        Err(fault) => return fault.status(),
    };
    // SAFETY: `out_text` was initialized above; the view borrows from the live result.
    unsafe {
        boundary::commit_scalar(
            out_text,
            madopilot_str_t::borrowed(result.result.regions()[index].text()),
        )
    };
    MADOPILOT_STATUS_OK
}

pub(crate) fn zone_result_retain(
    result: *const madopilot_ocr_zone_scan_result_t,
) -> madopilot_status_t {
    // SAFETY: null is a no-op; otherwise the caller owns a live reference.
    unsafe { handle::retain::<OcrZoneScanResultHandle>(result) };
    MADOPILOT_STATUS_OK
}

pub(crate) fn zone_result_release(
    result: *mut madopilot_ocr_zone_scan_result_t,
) -> madopilot_status_t {
    // SAFETY: null is a no-op; otherwise the caller gives up one live reference.
    unsafe { handle::release::<OcrZoneScanResultHandle>(result) };
    MADOPILOT_STATUS_OK
}

pub(crate) fn zone_result_info(
    result: *const madopilot_ocr_zone_scan_result_t,
    out_info: *mut madopilot_ocr_zone_scan_result_info_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable size-versioned output.
    let out = match unsafe { Out::begin(out_info) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);
    // SAFETY: the caller keeps the result retained for the call.
    let Some(result) = (unsafe { handle::borrow::<OcrZoneScanResultHandle>(result) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };

    let value = &result.result;
    let backend = value.backend();
    let model = backend.model_identity();
    let membership_count: usize = (0..value.effective_zones().len())
        .map(|index| {
            value
                .group(index)
                .expect("effective zone has a group")
                .len()
        })
        .sum();
    // SAFETY: `out` was validated; every string borrows from the retained result.
    unsafe {
        out.commit(madopilot_ocr_zone_scan_result_info_t {
            struct_size: out.declared_size(),
            flags: 0,
            source: stamp(
                value.stamp(),
                u32::try_from(size_of::<madopilot_frame_stamp_t>())
                    .expect("frame stamp is smaller than 4 GiB"),
            ),
            source_envelope: rect(value.source_envelope()),
            output_space: space_code(value.output_space()),
            zone_count: u64::try_from(value.effective_zones().len())
                .expect("zone count is bounded below u64::MAX"),
            unique_candidate_count: u64::try_from(value.unique_candidates().len())
                .expect("candidate count is bounded below u64::MAX"),
            membership_count: u64::try_from(membership_count)
                .expect("membership count is bounded below u64::MAX"),
            backend_id: madopilot_str_t::borrowed(backend.id().as_str()),
            backend_version: madopilot_str_t::borrowed(backend.version().as_str()),
            model_id: madopilot_str_t::borrowed(model.model().as_str()),
            model_version: madopilot_str_t::borrowed(model.version().as_str()),
            profile_id: madopilot_str_t::borrowed(model.profile().as_str()),
        });
    }
    MADOPILOT_STATUS_OK
}

pub(crate) fn zone_result_zone_at(
    result: *const madopilot_ocr_zone_scan_result_t,
    zone_index: usize,
    out_zone: *mut madopilot_ocr_zone_result_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable size-versioned output.
    let out = match unsafe { Out::begin(out_zone) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);
    // SAFETY: the caller keeps the result retained for the call.
    let Some(result) = (unsafe { handle::borrow::<OcrZoneScanResultHandle>(result) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    let zone_index = match boundary::index_within(
        zone_index,
        result.result.effective_zones().len(),
        "OCR zone",
    ) {
        Ok(index) => index,
        Err(fault) => return fault.status(),
    };
    let group = result
        .result
        .group(zone_index)
        .expect("an effective zone always has one group");
    // SAFETY: `out` was validated above.
    unsafe {
        out.commit(madopilot_ocr_zone_result_t {
            struct_size: out.declared_size(),
            flags: 0,
            effective_zone: rect(result.result.effective_zones()[zone_index]),
            reserved: 0,
            region_count: u64::try_from(group.len())
                .expect("group count is bounded below u64::MAX"),
        });
    }
    MADOPILOT_STATUS_OK
}

pub(crate) fn zone_result_region_at(
    result: *const madopilot_ocr_zone_scan_result_t,
    zone_index: usize,
    region_index: usize,
    out_region: *mut madopilot_ocr_region_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable size-versioned output.
    let out = match unsafe { Out::begin(out_region) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);
    // SAFETY: the caller keeps the result retained for the call.
    let Some(result) = (unsafe { handle::borrow::<OcrZoneScanResultHandle>(result) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    let zone_index = match boundary::index_within(
        zone_index,
        result.result.effective_zones().len(),
        "OCR zone",
    ) {
        Ok(index) => index,
        Err(fault) => return fault.status(),
    };
    let group = result
        .result
        .group(zone_index)
        .expect("an effective zone always has one group");
    let region_index = match boundary::index_within(region_index, group.len(), "OCR zone region") {
        Ok(index) => index,
        Err(fault) => return fault.status(),
    };
    let region = group
        .get(region_index)
        .expect("a checked group-relative index resolves one candidate");
    let points = region
        .geometry()
        .points()
        .map(|point| madopilot_ocr_point_t {
            x: point.x(),
            y: point.y(),
        });
    // SAFETY: `out` was validated above.
    unsafe {
        out.commit(madopilot_ocr_region_t {
            struct_size: out.declared_size(),
            flags: 0,
            confidence: region.confidence().get(),
            points,
        });
    }
    MADOPILOT_STATUS_OK
}

pub(crate) fn zone_result_text_at(
    result: *const madopilot_ocr_zone_scan_result_t,
    zone_index: usize,
    region_index: usize,
    out_text: *mut madopilot_str_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable, correctly aligned scalar output.
    let prepared =
        unsafe { boundary::begin_scalar_out(out_text, "out_text", madopilot_str_t::empty()) };
    if let Err(fault) = prepared {
        return fault.status();
    }
    hooks::reach(hooks::Site::Entry);
    // SAFETY: the caller keeps the result retained for the call.
    let Some(result) = (unsafe { handle::borrow::<OcrZoneScanResultHandle>(result) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    let zone_index = match boundary::index_within(
        zone_index,
        result.result.effective_zones().len(),
        "OCR zone",
    ) {
        Ok(index) => index,
        Err(fault) => return fault.status(),
    };
    let group = result
        .result
        .group(zone_index)
        .expect("an effective zone always has one group");
    let region_index = match boundary::index_within(region_index, group.len(), "OCR zone text") {
        Ok(index) => index,
        Err(fault) => return fault.status(),
    };
    let region = group
        .get(region_index)
        .expect("a checked group-relative index resolves one candidate");
    // SAFETY: `out_text` was initialized above; the view borrows from the live result.
    unsafe {
        boundary::commit_scalar(out_text, madopilot_str_t::borrowed(region.text()));
    }
    MADOPILOT_STATUS_OK
}
#[cfg(test)]
#[path = "ocr_tests.rs"]
mod tests;
