//! Sessions, frames, and CPU mappings.
//!
//! # What survives what
//!
//! A frame is not owned by the session that published it, and a mapping is not
//! owned by the frame it was taken from. Each is an independently retained
//! handle over immutable state, so a caller may close and release the session
//! and keep reading the mapped bytes. That is the whole point of the copy: a
//! mapping that stopped being readable when its producer went away would force
//! every caller to keep the producer alive for as long as it wanted the pixels.
//!
//! The one direction that does not hold is the borrowed view inside
//! [`madopilot_image_t`]: it points into the mapping handle, and it dies with
//! it.

use std::mem::size_of;

use mado_pilot::{
    ClipPolicy, CoordinateSpace, CpuMapping, Frame, FrameRequest, FrameStamp, MappingObserver,
    OcrBackendDescriptor, OpenRequest, PixelRect, Rect, Session, SessionRequest, TargetId,
};

use crate::boundary::{self, Out, Versioned, covers, declared, inputs, prefixes};
use crate::engine::{
    EngineHandle, TargetList, madopilot_engine_t, madopilot_target_list_t, report,
};
use crate::error::{self, Fault, madopilot_error_t};
use crate::handle::opaque;
use crate::operation;
use crate::status::{
    MADOPILOT_ERROR_CATEGORY_CAPTURE, MADOPILOT_ERROR_CATEGORY_INPUT,
    MADOPILOT_STATUS_INVALID_ARGUMENT, MADOPILOT_STATUS_OK, madopilot_status_t,
};
use crate::types::{
    MADOPILOT_MAP_HAS_REGION, MADOPILOT_OPEN_HAS_PREFERRED_FORMAT,
    MADOPILOT_OPEN_HAS_REQUIRED_FORMAT, MADOPILOT_PIXEL_FORMAT_RGBA8,
    MADOPILOT_SPACE_CAPTURE_PIXELS, clip_policy, madopilot_frame_info_t, madopilot_frame_stamp_t,
    madopilot_image_t, madopilot_input_descriptor_t, madopilot_input_open_request_t,
    madopilot_map_request_t, madopilot_open_request_t, madopilot_operation_t,
    madopilot_pixel_format_t, madopilot_pixel_rect_t, madopilot_session_info_t, pixel_format,
    pixel_format_code, space_code,
};
use crate::view::madopilot_bytes_t;
use crate::{handle, hooks};

opaque! {
    /// An open capture session.
    ///
    /// Closing is explicit and idempotent, and it does not invalidate frames,
    /// mappings, or results the caller already holds.
    madopilot_session_t => SessionHandle
}

opaque! {
    /// One immutable published frame.
    madopilot_frame_t => FrameHandle
}

opaque! {
    /// One completed CPU mapping and the storage its byte view borrows.
    madopilot_mapping_t => MappingHandle
}

/// The payload behind a session handle.
#[derive(Debug)]
pub(crate) struct SessionHandle {
    session: Session,
    /// Copied at open, so a failed search can name the backend that failed
    /// without the caller having to still hold the engine.
    backend: String,
    /// OCR backend/model/profile identity copied at open, when explicitly configured.
    ocr_backend: Option<OcrBackendDescriptor>,
    /// Facade identity copied from the discovery snapshot.
    target: TargetId,
    /// Immutable C projection of what the session accepted.
    input_descriptor: madopilot_input_descriptor_t,
    input_available: bool,
}

impl SessionHandle {
    pub(crate) const fn session(&self) -> &Session {
        &self.session
    }

    pub(crate) fn backend(&self) -> &str {
        &self.backend
    }

    pub(crate) const fn ocr_backend(&self) -> Option<&OcrBackendDescriptor> {
        self.ocr_backend.as_ref()
    }

    pub(crate) const fn target(&self) -> TargetId {
        self.target
    }

    pub(crate) const fn input_descriptor(&self) -> &madopilot_input_descriptor_t {
        &self.input_descriptor
    }

    pub(crate) const fn accepts_input(&self) -> bool {
        self.input_available
    }
}

/// The payload behind a frame handle.
#[derive(Debug)]
pub(crate) struct FrameHandle {
    frame: Frame,
    mapping_observer: MappingObserver,
}

impl FrameHandle {
    pub(crate) const fn new(frame: Frame, mapping_observer: MappingObserver) -> Self {
        Self {
            frame,
            mapping_observer,
        }
    }

    pub(crate) const fn frame(&self) -> &Frame {
        &self.frame
    }
}

/// The payload behind a mapping handle.
#[derive(Debug)]
pub(crate) struct MappingHandle {
    mapping: CpuMapping,
}

inputs! {
    impl Input for madopilot_open_request_t {
        // Through `flags`. A caller with no format preference still has to supply a
        // request, because "any layout" is a decision.
        const MANDATORY: usize = 8;
        const NAME: &'static str = "madopilot_open_request_t";
        const PREFIXES: &'static [usize] = prefixes!(
            madopilot_open_request_t,
            struct_size,
            flags,
            required_format,
            preferred_format,
        );
        const PRESENCE: &'static [(u32, usize)] = &[
            (
                MADOPILOT_OPEN_HAS_REQUIRED_FORMAT,
                covers!(madopilot_open_request_t, required_format: madopilot_pixel_format_t),
            ),
            (
                MADOPILOT_OPEN_HAS_PREFERRED_FORMAT,
                covers!(madopilot_open_request_t, preferred_format: madopilot_pixel_format_t),
            ),
        ];

        fn defaults() -> Self {
            Self {
                struct_size: 0,
                flags: 0,
                required_format: MADOPILOT_PIXEL_FORMAT_RGBA8,
                preferred_format: MADOPILOT_PIXEL_FORMAT_RGBA8,
            }
        }

        fn presence_bits(&self) -> u32 {
            self.flags
        }
    }

    impl Input for madopilot_map_request_t {
        // Through `format`: the layout the bytes must be in is the request.
        const MANDATORY: usize = 12;
        const NAME: &'static str = "madopilot_map_request_t";
        const PREFIXES: &'static [usize] = prefixes!(
            madopilot_map_request_t,
            struct_size,
            flags,
            format,
            clip_policy,
            region,
        );
        const PRESENCE: &'static [(u32, usize)] = &[(
            MADOPILOT_MAP_HAS_REGION,
            covers!(madopilot_map_request_t, region: madopilot_pixel_rect_t),
        )];

        fn defaults() -> Self {
            Self {
                struct_size: 0,
                flags: 0,
                format: MADOPILOT_PIXEL_FORMAT_RGBA8,
                clip_policy: crate::types::MADOPILOT_CLIP_POLICY_REJECT,
                region: madopilot_pixel_rect_t::empty(),
            }
        }

        fn presence_bits(&self) -> u32 {
            self.flags
        }
    }
}

impl Versioned for madopilot_frame_stamp_t {
    const MANDATORY: usize = 40;
    const NAME: &'static str = "madopilot_frame_stamp_t";
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_frame_stamp_t,
        struct_size,
        flags,
        stream,
        epoch,
        sequence,
        geometry,
    );
    const ZEROED_PADDING: &'static [(usize, usize)] = &[];

    fn failure(struct_size: u32) -> Self {
        Self::cleared(struct_size)
    }
}

impl Versioned for madopilot_frame_info_t {
    const MANDATORY: usize = 24;
    const NAME: &'static str = "madopilot_frame_info_t";
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_frame_info_t,
        struct_size,
        flags,
        width,
        height,
        format,
        space,
        stride,
        bounds,
    );
    const ZEROED_PADDING: &'static [(usize, usize)] = &[(
        covers!(madopilot_frame_info_t, bounds: madopilot_pixel_rect_t),
        size_of::<madopilot_frame_info_t>(),
    )];

    fn failure(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            width: 0,
            height: 0,
            format: MADOPILOT_PIXEL_FORMAT_RGBA8,
            space: MADOPILOT_SPACE_CAPTURE_PIXELS,
            stride: 0,
            bounds: madopilot_pixel_rect_t::empty(),
        }
    }
}

impl Versioned for madopilot_image_t {
    // Through `bytes`: a descriptor without the pixels it describes is not a
    // mapping.
    const MANDATORY: usize = 48;
    const NAME: &'static str = "madopilot_image_t";
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_image_t,
        struct_size,
        flags,
        width,
        height,
        format,
        space,
        stride,
        bytes,
        region,
    );
    const ZEROED_PADDING: &'static [(usize, usize)] = &[(
        covers!(madopilot_image_t, region: madopilot_pixel_rect_t),
        size_of::<madopilot_image_t>(),
    )];

    fn failure(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            width: 0,
            height: 0,
            format: MADOPILOT_PIXEL_FORMAT_RGBA8,
            space: MADOPILOT_SPACE_CAPTURE_PIXELS,
            stride: 0,
            bytes: madopilot_bytes_t::empty(),
            region: madopilot_pixel_rect_t::empty(),
        }
    }
}

impl Versioned for madopilot_session_info_t {
    const MANDATORY: usize = 32;
    const NAME: &'static str = "madopilot_session_info_t";
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_session_info_t,
        struct_size,
        flags,
        stream,
        width,
        height,
        format,
        coordinate_spaces,
        target,
        accepts_input,
        reserved,
    );
    const ZEROED_PADDING: &'static [(usize, usize)] = &[];

    fn failure(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            stream: 0,
            width: 0,
            height: 0,
            format: MADOPILOT_PIXEL_FORMAT_RGBA8,
            coordinate_spaces: 0,
            target: 0,
            accepts_input: 0,
            reserved: 0,
        }
    }
}

/// Projects a validated pixel rectangle onto its C form.
pub(crate) const fn rect(value: PixelRect) -> madopilot_pixel_rect_t {
    madopilot_pixel_rect_t {
        space: MADOPILOT_SPACE_CAPTURE_PIXELS,
        left: value.left(),
        top: value.top(),
        right: value.right(),
        bottom: value.bottom(),
    }
}

/// Projects a frame stamp onto its C form using its engine-local stream ordinal.
pub(crate) const fn stamp(value: FrameStamp, struct_size: u32) -> madopilot_frame_stamp_t {
    madopilot_frame_stamp_t {
        struct_size,
        flags: 0,
        stream: value.stream().get(),
        epoch: value.epoch().value(),
        sequence: value.sequence().value(),
        geometry: value.geometry().value(),
    }
}

/// Resolves a caller's rectangle into the geometry contract's own type.
pub(crate) fn source_rect(value: madopilot_pixel_rect_t) -> Result<Rect, Fault> {
    let space = crate::types::space(value.space)?;
    if space != CoordinateSpace::CapturePixels {
        return Err(Fault::abi(
            "the C ABI accepts a region in capture pixels only",
        ));
    }

    Rect::new(
        space,
        f64::from(value.left),
        f64::from(value.top),
        f64::from(value.right),
        f64::from(value.bottom),
    )
    .map_err(|fault| {
        Fault::from_error(
            &fault.into(),
            crate::status::MADOPILOT_ERROR_CATEGORY_GEOMETRY,
        )
    })
}

pub(crate) fn session_open(
    engine: *const madopilot_engine_t,
    targets: *const madopilot_target_list_t,
    index: usize,
    request: *const madopilot_open_request_t,
    operation: *const madopilot_operation_t,
    out_session: *mut *mut madopilot_session_t,
    out_error: *mut *mut madopilot_error_t,
) -> madopilot_status_t {
    if let Err(status) =
        // SAFETY: the caller supplies writable, correctly aligned output addresses.
        unsafe { boundary::begin_outputs(out_session, "out_session", out_error) }
    {
        return status;
    }
    hooks::reach(hooks::Site::Entry);

    // SAFETY: `out_error` was validated above.
    unsafe {
        report(
            out_error,
            run_session_open(
                engine,
                targets,
                index,
                request,
                None,
                operation,
                out_session,
            ),
        )
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the function signature is the accepted ABI 1.2 table entry"
)]
pub(crate) fn session_open_with_input(
    engine: *const madopilot_engine_t,
    targets: *const madopilot_target_list_t,
    index: usize,
    request: *const madopilot_open_request_t,
    input_request: *const madopilot_input_open_request_t,
    operation: *const madopilot_operation_t,
    out_session: *mut *mut madopilot_session_t,
    out_error: *mut *mut madopilot_error_t,
) -> madopilot_status_t {
    if let Err(status) =
        // SAFETY: the caller supplies writable, correctly aligned output addresses.
        unsafe { boundary::begin_outputs(out_session, "out_session", out_error) }
    {
        return status;
    }
    hooks::reach(hooks::Site::Entry);

    // SAFETY: `out_error` was validated above.
    unsafe {
        report(
            out_error,
            run_session_open(
                engine,
                targets,
                index,
                request,
                Some(input_request),
                operation,
                out_session,
            ),
        )
    }
}

fn run_session_open(
    engine: *const madopilot_engine_t,
    targets: *const madopilot_target_list_t,
    index: usize,
    request: *const madopilot_open_request_t,
    input_request: Option<*const madopilot_input_open_request_t>,
    operation: *const madopilot_operation_t,
    out_session: *mut *mut madopilot_session_t,
) -> Result<(), Fault> {
    // SAFETY: the caller keeps both handles retained for the call.
    let Some(engine) = (unsafe { handle::borrow::<EngineHandle>(engine) }) else {
        return Err(Fault::abi("`engine` is null"));
    };
    // SAFETY: as above.
    let Some(list) = (unsafe { handle::borrow::<TargetList>(targets) }) else {
        return Err(Fault::abi("`targets` is null"));
    };
    // SAFETY: the caller keeps the capture request and, when selected, the
    // separate input request readable for the call.
    let request = unsafe { boundary::read_input::<madopilot_open_request_t>(request) }?;
    let input = match input_request {
        // SAFETY: `session_open_with_input` requires this pointer to remain
        // readable for the call; `open_request` validates its prefix first.
        Some(request) => Some(unsafe { crate::input::open_request(request) }?),
        None => None,
    };
    // SAFETY: the caller keeps the operation readable for the call.
    let context = unsafe { operation::context(operation) }?;
    context.admit()?;

    let index = boundary::index_within(index, list.targets().len(), "target")?;
    // Copied here, so the target list may be released the moment this returns.
    let target = &list.targets()[index];
    let facade_target = target.facade_id();
    facade_target.check_engine(engine.id()).map_err(|fault| {
        let error: mado_pilot::Error = fault.into();
        Fault::from_error(&error, MADOPILOT_ERROR_CATEGORY_CAPTURE)
    })?;

    let mut open = OpenRequest::new();
    if declared!(
        request,
        madopilot_open_request_t,
        MADOPILOT_OPEN_HAS_REQUIRED_FORMAT
    ) {
        open = open.require_format(pixel_format(request.required_format)?);
    }
    if declared!(
        request,
        madopilot_open_request_t,
        MADOPILOT_OPEN_HAS_PREFERRED_FORMAT
    ) {
        open = open.prefer_format(pixel_format(request.preferred_format)?);
    }

    if let Some(required) = input
        .as_ref()
        .filter(|request| request.requirement().is_required())
    {
        let capability = target.description().capability().input();
        required
            .check(capability)
            .map_err(error::facade(MADOPILOT_ERROR_CATEGORY_INPUT))?;
    }

    let mut session_request = SessionRequest::new().capturing(open);
    if let Some(input) = input {
        session_request = session_request.requesting_input(input);
    }

    let session = engine
        .open_session(facade_target, &session_request, context.inner())
        .map_err(error::facade(MADOPILOT_ERROR_CATEGORY_CAPTURE))?;
    hooks::reach(hooks::Site::AfterTemporary);
    context.commit()?;

    let input_available = session.accepts_input();
    let input_descriptor = crate::input::descriptor(
        facade_target.get(),
        session.input_descriptor(),
        u32::try_from(size_of::<madopilot_input_descriptor_t>())
            .expect("the C input descriptor is smaller than 4 GiB"),
    )?;
    let payload = SessionHandle {
        session,
        backend: engine.backend().id().to_owned(),
        ocr_backend: engine.ocr_backend(),
        target: facade_target,
        input_descriptor,
        input_available,
    };
    // SAFETY: `out_session` was validated by the entry before any work began.
    unsafe { out_session.write(handle::into_raw(payload)) };

    Ok(())
}

pub(crate) fn session_retain(session: *const madopilot_session_t) -> madopilot_status_t {
    // SAFETY: the handle is null or one this module produced, and the caller
    // holds a live reference for the call.
    unsafe { handle::retain::<SessionHandle>(session) }

    MADOPILOT_STATUS_OK
}

pub(crate) fn session_release(session: *mut madopilot_session_t) -> madopilot_status_t {
    // SAFETY: as `session_retain`, and the caller is giving up its reference.
    unsafe { handle::release::<SessionHandle>(session) }

    MADOPILOT_STATUS_OK
}

pub(crate) fn session_describe(
    session: *const madopilot_session_t,
    out_info: *mut madopilot_session_info_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable output structure whose
    // `struct_size` it has set.
    let out = match unsafe { Out::begin(out_info) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);

    // SAFETY: the caller keeps the handle retained for the call.
    let Some(session) = (unsafe { handle::borrow::<SessionHandle>(session) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };

    let description = session.session.description();
    let mut coordinate_spaces = 0;
    for space in [
        CoordinateSpace::CapturePixels,
        CoordinateSpace::FrameNormalized,
        CoordinateSpace::TargetNormalized,
        CoordinateSpace::TargetLogical,
        CoordinateSpace::DesktopLogical,
    ] {
        if description.coordinates().supports(space) {
            coordinate_spaces |= 1 << space_code(space);
        }
    }

    let format = match pixel_format_code(description.format()) {
        Ok(format) => format,
        Err(fault) => return fault.status(),
    };

    // SAFETY: `out` was validated above.
    unsafe {
        out.commit(madopilot_session_info_t {
            struct_size: out.declared_size(),
            flags: 0,
            stream: session.session.stream().get(),
            width: description.extent().width(),
            height: description.extent().height(),
            format,
            coordinate_spaces,
            target: session.target().get(),
            accepts_input: i32::from(session.accepts_input()),
            reserved: 0,
        });
    }

    MADOPILOT_STATUS_OK
}

pub(crate) fn session_close(
    session: *const madopilot_session_t,
    operation: *const madopilot_operation_t,
    out_error: *mut *mut madopilot_error_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable, correctly aligned output address
    // or null.
    if let Err(fault) = unsafe { boundary::begin_error_out(out_error) } {
        // The only output this entry has is the error one, and the only way to
        // arrive here is that it was rejected. Unlike `begin_outputs`, there is
        // nowhere to describe the fault, so the status is all the caller gets.
        return fault.status();
    }
    hooks::reach(hooks::Site::Entry);

    // SAFETY: `out_error` was validated above.
    unsafe { report(out_error, run_session_close(session, operation)) }
}

fn run_session_close(
    session: *const madopilot_session_t,
    operation: *const madopilot_operation_t,
) -> Result<(), Fault> {
    // SAFETY: the caller keeps the handle retained for the call.
    let Some(session) = (unsafe { handle::borrow::<SessionHandle>(session) }) else {
        return Err(Fault::abi("`session` is null"));
    };
    // SAFETY: the caller keeps the operation structure readable for the call.
    let context = unsafe { operation::context(operation) }?;

    // Close is deliberately admitted even when the operation is already over:
    // the context it carries bounds the drain, and refusing to start closing
    // because the deadline passed would leave the session open forever.
    session
        .session
        .close(context.inner())
        .map_err(error::facade(MADOPILOT_ERROR_CATEGORY_CAPTURE))
}

pub(crate) fn session_is_closed(
    session: *const madopilot_session_t,
    out_closed: *mut i32,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable, correctly aligned output address.
    if let Err(fault) = unsafe { boundary::begin_scalar_out(out_closed, "out_closed", 0) } {
        return fault.status();
    }
    hooks::reach(hooks::Site::Entry);

    // SAFETY: the caller keeps the handle retained for the call.
    let Some(session) = (unsafe { handle::borrow::<SessionHandle>(session) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    // SAFETY: `out_closed` was validated above.
    unsafe { boundary::commit_scalar(out_closed, i32::from(session.session.is_closed())) };

    MADOPILOT_STATUS_OK
}

pub(crate) fn session_acquire_frame(
    session: *const madopilot_session_t,
    operation: *const madopilot_operation_t,
    out_frame: *mut *mut madopilot_frame_t,
    out_error: *mut *mut madopilot_error_t,
) -> madopilot_status_t {
    if let Err(status) =
        // SAFETY: the caller supplies writable, correctly aligned output addresses.
        unsafe { boundary::begin_outputs(out_frame, "out_frame", out_error) }
    {
        return status;
    }
    hooks::reach(hooks::Site::Entry);

    // SAFETY: `out_error` was validated above.
    unsafe { report(out_error, run_session_frame(session, operation, out_frame)) }
}

fn run_session_frame(
    session: *const madopilot_session_t,
    operation: *const madopilot_operation_t,
    out_frame: *mut *mut madopilot_frame_t,
) -> Result<(), Fault> {
    // SAFETY: the caller keeps the handle retained for the call.
    let Some(session) = (unsafe { handle::borrow::<SessionHandle>(session) }) else {
        return Err(Fault::abi("`session` is null"));
    };
    // SAFETY: the caller keeps the operation structure readable for the call.
    let context = unsafe { operation::context(operation) }?;
    context.admit()?;

    let frame = session
        .session
        .acquire_frame(&FrameRequest::latest(), context.inner())
        .map_err(error::facade(MADOPILOT_ERROR_CATEGORY_CAPTURE))?;
    hooks::reach(hooks::Site::AfterTemporary);
    context.commit()?;

    let payload = FrameHandle::new(frame, session.session.mapping_observer());
    // SAFETY: `out_frame` was validated by the entry before any work began.
    unsafe { out_frame.write(handle::into_raw(payload)) };

    Ok(())
}

pub(crate) fn frame_retain(frame: *const madopilot_frame_t) -> madopilot_status_t {
    // SAFETY: the handle is null or one this module produced, and the caller
    // holds a live reference for the call.
    unsafe { handle::retain::<FrameHandle>(frame) }

    MADOPILOT_STATUS_OK
}

pub(crate) fn frame_release(frame: *mut madopilot_frame_t) -> madopilot_status_t {
    // SAFETY: as `frame_retain`, and the caller is giving up its reference.
    unsafe { handle::release::<FrameHandle>(frame) }

    MADOPILOT_STATUS_OK
}

pub(crate) fn frame_stamp(
    frame: *const madopilot_frame_t,
    out_stamp: *mut madopilot_frame_stamp_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable output structure whose
    // `struct_size` it has set.
    let out = match unsafe { Out::begin(out_stamp) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);

    // SAFETY: the caller keeps the handle retained for the call.
    let Some(frame) = (unsafe { handle::borrow::<FrameHandle>(frame) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    // SAFETY: `out` was validated above.
    unsafe { out.commit(stamp(frame.frame.stamp(), out.declared_size())) };

    MADOPILOT_STATUS_OK
}

pub(crate) fn frame_describe(
    frame: *const madopilot_frame_t,
    out_info: *mut madopilot_frame_info_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable output structure whose
    // `struct_size` it has set.
    let out = match unsafe { Out::begin(out_info) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);

    // SAFETY: the caller keeps the handle retained for the call.
    let Some(frame) = (unsafe { handle::borrow::<FrameHandle>(frame) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };

    let descriptor = frame.frame.descriptor();
    let bounds = match frame.frame.bounds() {
        Ok(bounds) => bounds,
        Err(fault) => return error::status_code(fault.status()),
    };
    let format = match pixel_format_code(descriptor.format()) {
        Ok(format) => format,
        Err(fault) => return fault.status(),
    };

    // SAFETY: `out` was validated above.
    unsafe {
        out.commit(madopilot_frame_info_t {
            struct_size: out.declared_size(),
            flags: 0,
            width: descriptor.extent().width(),
            height: descriptor.extent().height(),
            format,
            space: MADOPILOT_SPACE_CAPTURE_PIXELS,
            stride: descriptor.stride() as u64,
            bounds: rect(bounds),
        });
    }

    MADOPILOT_STATUS_OK
}

pub(crate) fn frame_map(
    frame: *const madopilot_frame_t,
    request: *const madopilot_map_request_t,
    operation: *const madopilot_operation_t,
    out_mapping: *mut *mut madopilot_mapping_t,
    out_error: *mut *mut madopilot_error_t,
) -> madopilot_status_t {
    if let Err(status) =
        // SAFETY: the caller supplies writable, correctly aligned output addresses.
        unsafe { boundary::begin_outputs(out_mapping, "out_mapping", out_error) }
    {
        return status;
    }
    hooks::reach(hooks::Site::Entry);

    // SAFETY: `out_error` was validated above.
    unsafe {
        report(
            out_error,
            run_frame_map(frame, request, operation, out_mapping),
        )
    }
}

fn run_frame_map(
    frame: *const madopilot_frame_t,
    request: *const madopilot_map_request_t,
    operation: *const madopilot_operation_t,
    out_mapping: *mut *mut madopilot_mapping_t,
) -> Result<(), Fault> {
    // SAFETY: the caller keeps the handle retained for the call.
    let Some(frame) = (unsafe { handle::borrow::<FrameHandle>(frame) }) else {
        return Err(Fault::abi("`frame` is null"));
    };
    // SAFETY: the caller keeps both structures readable for the call.
    let request = unsafe { boundary::read_input::<madopilot_map_request_t>(request) }?;
    // SAFETY: as above.
    let context = unsafe { operation::context(operation) }?;
    context.admit()?;

    let format = pixel_format(request.format)?;
    let mapping = if declared!(request, madopilot_map_request_t, MADOPILOT_MAP_HAS_REGION) {
        let region = source_rect(request.region)?;
        let policy: ClipPolicy = clip_policy(request.clip_policy)?;
        frame
            .mapping_observer
            .map_region(&frame.frame, region, policy, format, context.inner())
    } else {
        frame
            .mapping_observer
            .map_frame(&frame.frame, format, context.inner())
    }
    .map_err(error::facade(MADOPILOT_ERROR_CATEGORY_CAPTURE))?;
    hooks::reach(hooks::Site::AfterTemporary);
    context.commit()?;

    let payload = MappingHandle { mapping };
    // SAFETY: `out_mapping` was validated by the entry before any work began.
    unsafe { out_mapping.write(handle::into_raw(payload)) };

    Ok(())
}

pub(crate) fn mapping_retain(mapping: *const madopilot_mapping_t) -> madopilot_status_t {
    // SAFETY: the handle is null or one this module produced, and the caller
    // holds a live reference for the call.
    unsafe { handle::retain::<MappingHandle>(mapping) }

    MADOPILOT_STATUS_OK
}

pub(crate) fn mapping_release(mapping: *mut madopilot_mapping_t) -> madopilot_status_t {
    // SAFETY: as `mapping_retain`, and the caller is giving up its reference.
    // Every byte view borrowed from this mapping becomes invalid at the final
    // release, and the retained frame storage is dropped exactly once.
    unsafe { handle::release::<MappingHandle>(mapping) }

    MADOPILOT_STATUS_OK
}

pub(crate) fn mapping_describe(
    mapping: *const madopilot_mapping_t,
    out_image: *mut madopilot_image_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable output structure whose
    // `struct_size` it has set.
    let out = match unsafe { Out::begin(out_image) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);

    // SAFETY: the caller keeps the handle retained for the call.
    let Some(mapping) = (unsafe { handle::borrow::<MappingHandle>(mapping) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };

    let descriptor = mapping.mapping.descriptor();
    let format = match pixel_format_code(descriptor.format()) {
        Ok(format) => format,
        Err(fault) => return fault.status(),
    };
    let shared = if mapping.mapping.is_shared() {
        crate::types::MADOPILOT_IMAGE_SHARED
    } else {
        0
    };

    // SAFETY: `out` was validated above, and the byte view borrows from the
    // mapping the caller keeps retained.
    unsafe {
        out.commit(madopilot_image_t {
            struct_size: out.declared_size(),
            flags: shared,
            width: descriptor.extent().width(),
            height: descriptor.extent().height(),
            format,
            space: MADOPILOT_SPACE_CAPTURE_PIXELS,
            stride: descriptor.stride() as u64,
            bytes: madopilot_bytes_t::borrowed(mapping.mapping.bytes()),
            region: rect(mapping.mapping.region()),
        });
    }

    MADOPILOT_STATUS_OK
}

pub(crate) fn mapping_stamp(
    mapping: *const madopilot_mapping_t,
    out_stamp: *mut madopilot_frame_stamp_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable output structure whose
    // `struct_size` it has set.
    let out = match unsafe { Out::begin(out_stamp) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);

    // SAFETY: the caller keeps the handle retained for the call.
    let Some(mapping) = (unsafe { handle::borrow::<MappingHandle>(mapping) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    // SAFETY: `out` was validated above.
    unsafe { out.commit(stamp(mapping.mapping.stamp(), out.declared_size())) };

    MADOPILOT_STATUS_OK
}
