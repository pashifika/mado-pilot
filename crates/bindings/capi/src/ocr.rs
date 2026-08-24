//! One-shot OCR execution and immutable owned result projection.
//!
//! A request borrows its session, exact frame, validated package, backend/model
//! identity views, and operation context for one synchronous call. A successful
//! result owns only the facade's immutable `OcrResult`; it retains no parent
//! handle, frame storage, package bytes, backend buffer, lock, callback, or worker.
//! Text returned by `ocr_result_text_at` is borrowed from that result owner.

use std::mem::size_of;

use mado_pilot::{AssetPackage, Error, OcrFault, OcrRegion, OcrRequest, OcrResult, Status};

use crate::boundary::{self, Out, Versioned, covers, declared, inputs, prefixes};
use crate::capture::{FrameHandle, SessionHandle, madopilot_session_t, rect, source_rect, stamp};
use crate::error::{Fault, madopilot_error_t};
use crate::handle::opaque;
use crate::operation;
use crate::status::{
    MADOPILOT_ERROR_CATEGORY_VISION, MADOPILOT_STATUS_INVALID_ARGUMENT, MADOPILOT_STATUS_OK,
    madopilot_status_t,
};
use crate::types::{
    MADOPILOT_CLIP_POLICY_REJECT, MADOPILOT_OCR_HAS_REGION, MADOPILOT_SPACE_CAPTURE_PIXELS,
    clip_policy, madopilot_frame_stamp_t, madopilot_ocr_point_t, madopilot_ocr_region_t,
    madopilot_ocr_request_t, madopilot_ocr_result_info_t, madopilot_operation_t,
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

#[derive(Debug)]
pub(crate) struct OcrResultHandle {
    result: OcrResult,
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
    // SAFETY: as above.
    let Some(package) = (unsafe { handle::borrow::<AssetPackage>(request.package) }) else {
        return Err(Fault::abi("`package` is null"));
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
    let model = package
        .resolve_ocr_model(model_id)
        .map_err(Fault::from_asset)?;
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
            model.identity(),
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

#[cfg(test)]
#[path = "ocr_tests.rs"]
mod tests;
