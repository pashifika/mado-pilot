//! Engine construction and target discovery.
//!
//! An engine is the C caller's root handle: it owns the wired capture adapter
//! and the required matching backend, and every other handle is reached through
//! it. ABI 1.2 adds native release-target wiring without changing replay.
//!
//! # Boundary identities
//!
//! Facade target and stream identities expose checked nonzero ordinals scoped
//! to their issuing engine. The C boundary projects those ordinals directly,
//! so rediscovery and lazy diagnostic projection preserve correlation without
//! retaining a second identity registry.

use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use mado_pilot::replay::{ReplayFrame, ReplaySource, ReplayTarget};
use mado_pilot::{
    DiagnosticOptions, Engine, FrameDescriptor, MonotonicInstant, OcrBackend, PixelExtent,
    TargetDescription, TargetId,
};

use crate::boundary::{self, Input, Out, Versioned, covers, inputs, prefixes};
use crate::error::{self, Fault, madopilot_error_t};
use crate::handle::opaque;
use crate::operation;
use crate::status::{
    MADOPILOT_ERROR_CATEGORY_CAPTURE, MADOPILOT_ERROR_CATEGORY_ENGINE, MADOPILOT_STATUS_OK,
    madopilot_status_t,
};
use crate::types::{
    MADOPILOT_PIXEL_FORMAT_RGBA8, MADOPILOT_SOURCE_NATIVE_MACOS, MADOPILOT_SOURCE_NATIVE_WINDOWS,
    MADOPILOT_SOURCE_REPLAY_DIRECTORY, MADOPILOT_SOURCE_REPLAY_MEMORY,
    MADOPILOT_TARGET_HAS_CAPTURE_PERMISSION, MADOPILOT_TARGET_HAS_KIND,
    MADOPILOT_TARGET_SUPPORTS_PLACEMENT, madopilot_permission_kind_t, madopilot_replay_frame_t,
    madopilot_source_t, madopilot_target_kind_t, madopilot_target_t, pixel_format,
    pixel_format_code, space_code,
};
use crate::view::{self, madopilot_bytes_t, madopilot_str_t};
use crate::{handle, hooks};

opaque! {
    /// A configured engine: one capture adapter and one matching backend.
    madopilot_engine_t => EngineHandle
}

/// The facade engine behind the C handle.
#[derive(Debug)]
pub(crate) struct EngineHandle {
    engine: Engine,
}

impl EngineHandle {
    pub(crate) fn new(engine: Engine) -> Self {
        Self { engine }
    }
}

impl Deref for EngineHandle {
    type Target = Engine;

    fn deref(&self) -> &Self::Target {
        &self.engine
    }
}

opaque! {
    /// An immutable snapshot of one discovery.
    ///
    /// Every string a target descriptor reports is borrowed from this handle and
    /// becomes invalid at its final release. A session opened from a descriptor
    /// copies the identity it needs, so releasing the list does not disturb it.
    madopilot_target_list_t => TargetList
}

/// One target in an immutable discovery snapshot.
#[derive(Debug)]
pub(crate) struct TargetRecord {
    description: TargetDescription,
}

impl TargetRecord {
    fn new(description: TargetDescription) -> Self {
        Self { description }
    }

    pub(crate) const fn description(&self) -> &TargetDescription {
        &self.description
    }

    pub(crate) const fn facade_id(&self) -> TargetId {
        self.description.id()
    }

    pub(crate) const fn ordinal(&self) -> u64 {
        self.facade_id().get()
    }
}

/// The payload behind a target-list handle.
#[derive(Debug)]
pub(crate) struct TargetList(Vec<TargetRecord>);

impl TargetList {
    pub(crate) fn targets(&self) -> &[TargetRecord] {
        &self.0
    }
}

inputs! {
    impl Input for madopilot_source_t {
        // Through `frame_stride`: a memory source cannot be read without the stride
        // of the array it points at, and a directory source is not harmed by
        // carrying fields it leaves empty.
        const MANDATORY: usize = 48;
        const NAME: &'static str = "madopilot_source_t";
        const PREFIXES: &'static [usize] = prefixes!(
            madopilot_source_t,
            struct_size,
            kind,
            directory,
            frames,
            frame_count,
            frame_stride,
            target_name,
        );
        const PRESENCE: &'static [(u32, usize)] = &[];

        fn defaults() -> Self {
            Self {
                struct_size: 0,
                kind: MADOPILOT_SOURCE_REPLAY_MEMORY,
                directory: madopilot_str_t::empty(),
                frames: std::ptr::null(),
                frame_count: 0,
                frame_stride: 0,
                target_name: madopilot_str_t::empty(),
            }
        }

        fn presence_bits(&self) -> u32 {
            // The second field is `kind`, a discriminant rather than a bit set.
            0
        }
    }

    impl Input for madopilot_replay_frame_t {
        // Through `pixels`. Everything before it describes what the pixels are, and
        // a frame without them is not a frame.
        const MANDATORY: usize = 40;
        const NAME: &'static str = "madopilot_replay_frame_t";
        const PREFIXES: &'static [usize] = prefixes!(
            madopilot_replay_frame_t,
            struct_size,
            flags,
            width,
            height,
            format,
            continuity,
            pixels,
            captured_at_nanos,
            stride,
        );
        const PRESENCE: &'static [(u32, usize)] = &[];

        fn defaults() -> Self {
            Self {
                struct_size: 0,
                flags: 0,
                width: 0,
                height: 0,
                format: MADOPILOT_PIXEL_FORMAT_RGBA8,
                continuity: crate::types::MADOPILOT_CONTINUITY_CONTINUOUS,
                pixels: madopilot_bytes_t::empty(),
                captured_at_nanos: 0,
                stride: 0,
            }
        }

        fn presence_bits(&self) -> u32 {
            self.flags
        }
    }
}

impl Versioned for madopilot_target_t {
    // Through `coordinate_spaces`: what a target is, before what it is called.
    const MANDATORY: usize = 24;
    const NAME: &'static str = "madopilot_target_t";
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_target_t,
        struct_size,
        flags,
        width,
        height,
        format,
        coordinate_spaces,
        name,
        provider,
        target,
        kind,
        capture,
        capture_permission,
        reserved,
    );
    const ZEROED_PADDING: &'static [(usize, usize)] = &[];

    fn failure(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            width: 0,
            height: 0,
            format: MADOPILOT_PIXEL_FORMAT_RGBA8,
            coordinate_spaces: 0,
            name: madopilot_str_t::empty(),
            provider: madopilot_str_t::empty(),
            target: 0,
            kind: crate::types::MADOPILOT_TARGET_KIND_UNKNOWN,
            capture: crate::types::MADOPILOT_CAPABILITY_UNKNOWN,
            capture_permission: crate::types::MADOPILOT_PERMISSION_KIND_UNSPECIFIED,
            reserved: 0,
        }
    }
}

/// Builds the replay source a caller's tagged structure describes.
///
/// # Safety
///
/// Every pointer and view the structure carries must be readable for the call.
unsafe fn replay_source(source: &madopilot_source_t) -> Result<ReplaySource, Fault> {
    match source.kind {
        MADOPILOT_SOURCE_REPLAY_DIRECTORY => {
            // SAFETY: forwarded unchanged from this function's own contract.
            let directory = unsafe { view::non_empty_string(source.directory, "directory") }?;
            ReplaySource::from_directory(directory)
                .map_err(|fault| Fault::from_error(&fault.into(), MADOPILOT_ERROR_CATEGORY_CAPTURE))
        }
        MADOPILOT_SOURCE_REPLAY_MEMORY => {
            // SAFETY: forwarded unchanged from this function's own contract.
            let frames = unsafe { replay_frames(source) }?;
            // SAFETY: as above.
            let name = unsafe { view::string(source.target_name, "target_name") }?;
            let name = if name.is_empty() { "replay" } else { name };

            let target = ReplayTarget::new(name, frames).map_err(|fault| {
                Fault::from_error(&fault.into(), MADOPILOT_ERROR_CATEGORY_CAPTURE)
            })?;
            ReplaySource::from_targets(vec![target])
                .map_err(|fault| Fault::from_error(&fault.into(), MADOPILOT_ERROR_CATEGORY_CAPTURE))
        }
        other => Err(Fault::abi(format!("unrecognized source kind {other}"))),
    }
}

#[cfg(windows)]
fn native_windows_engine(diagnostics: DiagnosticOptions) -> Result<Engine, Fault> {
    mado_pilot::windows_engine(mado_pilot::NativeEngineRequest::new().with_diagnostics(diagnostics))
        .map_err(error::facade(MADOPILOT_ERROR_CATEGORY_ENGINE))
}

#[cfg(not(windows))]
fn native_windows_engine(_diagnostics: DiagnosticOptions) -> Result<Engine, Fault> {
    Err(Fault::from_error(
        &mado_pilot::Error::new(
            mado_pilot::Status::Unsupported,
            "native Windows construction is unavailable in this build",
        ),
        MADOPILOT_ERROR_CATEGORY_ENGINE,
    ))
}

#[cfg(target_os = "macos")]
fn native_macos_engine(diagnostics: DiagnosticOptions) -> Result<Engine, Fault> {
    mado_pilot::macos_engine(mado_pilot::NativeEngineRequest::new().with_diagnostics(diagnostics))
        .map_err(error::facade(MADOPILOT_ERROR_CATEGORY_ENGINE))
}

#[cfg(not(target_os = "macos"))]
fn native_macos_engine(_diagnostics: DiagnosticOptions) -> Result<Engine, Fault> {
    Err(Fault::from_error(
        &mado_pilot::Error::new(
            mado_pilot::Status::Unsupported,
            "native macOS construction is unavailable in this build",
        ),
        MADOPILOT_ERROR_CATEGORY_ENGINE,
    ))
}

/// Reads the caller's frame array, one validated element at a time.
///
/// # Safety
///
/// As [`replay_source`].
unsafe fn replay_frames(source: &madopilot_source_t) -> Result<Vec<ReplayFrame>, Fault> {
    // The span is computed and rejected before a single element address is
    // formed, so an overflowing count never becomes a pointer.
    let span = boundary::span(
        source.frame_count,
        source.frame_stride,
        madopilot_replay_frame_t::MANDATORY,
        "frames",
    )?;
    if span == 0 {
        return Err(Fault::abi("a memory replay source declares no frames"));
    }
    if source.frames.is_null() {
        return Err(Fault::abi(format!(
            "`frames` is null with a count of {}",
            source.frame_count
        )));
    }

    // Nothing is reserved from `frame_count`. The count is a caller's claim
    // about memory this library cannot see, and reserving against it turns an
    // implausible number into an allocation failure, which aborts the process
    // instead of returning a status a caller can read. Growing as elements are
    // validated costs one reallocation per doubling and bounds the library's
    // memory by the frames it has actually accepted.
    let mut frames = Vec::new();
    for index in 0..source.frame_count {
        // SAFETY: `span` proved that `index * stride` stays inside one
        // representable object, and the caller contract requires that object to
        // be readable for the call.
        let element = unsafe { source.frames.cast::<u8>().add(index * source.frame_stride) }
            .cast::<madopilot_replay_frame_t>();
        // SAFETY: as above. `read_element` validates alignment and declared
        // size, and reads no further than the stride the array declared, so it
        // stays inside the element even when the element claims to be larger.
        let frame = unsafe {
            boundary::read_element::<madopilot_replay_frame_t>(element, source.frame_stride)
        }?;
        // SAFETY: as above, for the frame's own pixel view.
        frames.push(unsafe { replay_frame(&frame, index) }?);
    }

    Ok(frames)
}

/// # Safety
///
/// As [`replay_source`].
unsafe fn replay_frame(
    frame: &madopilot_replay_frame_t,
    index: usize,
) -> Result<ReplayFrame, Fault> {
    let format = pixel_format(frame.format)?;
    let continuity = crate::types::continuity(frame.continuity)?;
    let extent = PixelExtent::new(frame.width, frame.height);

    let descriptor = if frame.stride == 0 {
        FrameDescriptor::packed(extent, format)
    } else {
        let stride = usize::try_from(frame.stride).map_err(|_| {
            Fault::abi(format!(
                "frame {index} declares a stride of {} bytes, which is not addressable",
                frame.stride
            ))
        })?;
        FrameDescriptor::new(extent, format, stride)
    }
    .map_err(|fault| Fault::from_error(&fault.into(), MADOPILOT_ERROR_CATEGORY_CAPTURE))?;

    // SAFETY: forwarded unchanged from this function's own contract.
    let pixels = unsafe { view::bytes(frame.pixels, "pixels") }?;
    if pixels.len() != descriptor.byte_len() {
        return Err(Fault::abi(format!(
            "frame {index} supplies {} bytes for a descriptor that needs {}",
            pixels.len(),
            descriptor.byte_len()
        )));
    }

    ReplayFrame::new(
        descriptor,
        MonotonicInstant::from_origin(Duration::from_nanos(frame.captured_at_nanos)),
        continuity,
        None,
        pixels.to_vec().into_boxed_slice(),
    )
    .map_err(|fault| Fault::from_error(&fault.into(), MADOPILOT_ERROR_CATEGORY_CAPTURE))
}

pub(crate) fn create(
    source: *const madopilot_source_t,
    operation: *const crate::types::madopilot_operation_t,
    out_engine: *mut *mut madopilot_engine_t,
    out_error: *mut *mut madopilot_error_t,
) -> madopilot_status_t {
    create_inner(source, std::ptr::null(), operation, out_engine, out_error)
}

pub(crate) fn create_with_options(
    source: *const madopilot_source_t,
    options: *const crate::types::madopilot_engine_options_t,
    operation: *const crate::types::madopilot_operation_t,
    out_engine: *mut *mut madopilot_engine_t,
    out_error: *mut *mut madopilot_error_t,
) -> madopilot_status_t {
    create_inner(source, options, operation, out_engine, out_error)
}

fn create_inner(
    source: *const madopilot_source_t,
    options: *const crate::types::madopilot_engine_options_t,
    operation: *const crate::types::madopilot_operation_t,
    out_engine: *mut *mut madopilot_engine_t,
    out_error: *mut *mut madopilot_error_t,
) -> madopilot_status_t {
    create_inner_with_ocr(source, options, operation, out_engine, out_error, None)
}

#[cfg(feature = "private-fixture")]
pub(crate) fn create_private_ocr_fixture(
    source: *const madopilot_source_t,
    options: *const crate::types::madopilot_engine_options_t,
    operation: *const crate::types::madopilot_operation_t,
    out_engine: *mut *mut madopilot_engine_t,
    out_error: *mut *mut madopilot_error_t,
) -> madopilot_status_t {
    create_inner_with_ocr(
        source,
        options,
        operation,
        out_engine,
        out_error,
        Some(crate::fixture::ocr_backend()),
    )
}

fn create_inner_with_ocr(
    source: *const madopilot_source_t,
    options: *const crate::types::madopilot_engine_options_t,
    operation: *const crate::types::madopilot_operation_t,
    out_engine: *mut *mut madopilot_engine_t,
    out_error: *mut *mut madopilot_error_t,
    ocr_backend: Option<Arc<dyn OcrBackend>>,
) -> madopilot_status_t {
    if let Err(status) =
        // SAFETY: the caller supplies writable, correctly aligned output addresses.
        unsafe { boundary::begin_outputs(out_engine, "out_engine", out_error) }
    {
        return status;
    }
    hooks::reach(hooks::Site::Entry);

    // SAFETY: `out_error` was validated above.
    unsafe {
        report(
            out_error,
            build_engine(source, options, operation, out_engine, ocr_backend),
        )
    }
}

fn build_engine(
    source: *const madopilot_source_t,
    options: *const crate::types::madopilot_engine_options_t,
    operation: *const crate::types::madopilot_operation_t,
    out_engine: *mut *mut madopilot_engine_t,
    ocr_backend: Option<Arc<dyn OcrBackend>>,
) -> Result<(), Fault> {
    // SAFETY: the caller keeps every structure and handle it named readable and
    // retained for the call.
    let context = unsafe { operation::context(operation) }?;
    context.admit()?;

    // SAFETY: as above.
    let request = unsafe { boundary::read_input::<madopilot_source_t>(source) }?;
    // SAFETY: null selects defaults; otherwise the caller keeps the options readable.
    let diagnostics = unsafe { crate::diagnostic::options(options) }?;

    // SAFETY: as above. Fields not selected by the source tag remain unread.
    let fixture_ocr = ocr_backend.is_some();
    let engine = match request.kind {
        MADOPILOT_SOURCE_REPLAY_MEMORY | MADOPILOT_SOURCE_REPLAY_DIRECTORY => {
            // SAFETY: the replay fields selected by the tag remain readable for
            // the call under this function's boundary contract.
            let configured = unsafe { replay_source(&request) }?;
            let mut replay =
                mado_pilot::ReplayEngineRequest::new(configured).with_diagnostics(diagnostics);
            if let Some(backend) = ocr_backend {
                replay = replay.with_ocr_backend(backend);
            }
            mado_pilot::replay_engine(replay)
                .map_err(error::facade(MADOPILOT_ERROR_CATEGORY_ENGINE))?
        }
        MADOPILOT_SOURCE_NATIVE_WINDOWS if !fixture_ocr => native_windows_engine(diagnostics)?,
        MADOPILOT_SOURCE_NATIVE_MACOS if !fixture_ocr => native_macos_engine(diagnostics)?,
        MADOPILOT_SOURCE_NATIVE_WINDOWS | MADOPILOT_SOURCE_NATIVE_MACOS => {
            return Err(Fault::abi(
                "the private OCR fixture constructor accepts replay sources only",
            ));
        }
        other => return Err(Fault::abi(format!("unrecognized source kind {other}"))),
    };
    let engine = EngineHandle::new(engine);
    hooks::reach(hooks::Site::AfterTemporary);

    // The engine exists but is not the caller's yet. An operation that ran out
    // of time here drops it rather than publishing work the caller is no longer
    // entitled to.
    context.commit()?;

    // SAFETY: `out_engine` was validated by the entry before any work began.
    unsafe { out_engine.write(handle::into_raw(engine)) };

    Ok(())
}

pub(crate) fn retain(engine: *const madopilot_engine_t) -> madopilot_status_t {
    // SAFETY: the handle is null or one this module produced, and the caller
    // holds a live reference for the call.
    unsafe { handle::retain::<EngineHandle>(engine) }

    MADOPILOT_STATUS_OK
}

pub(crate) fn release(engine: *mut madopilot_engine_t) -> madopilot_status_t {
    // SAFETY: as `retain`, and the caller is giving up the reference it owns.
    unsafe { handle::release::<EngineHandle>(engine) }

    MADOPILOT_STATUS_OK
}

pub(crate) fn discover(
    engine: *const madopilot_engine_t,
    operation: *const crate::types::madopilot_operation_t,
    out_targets: *mut *mut madopilot_target_list_t,
    out_error: *mut *mut madopilot_error_t,
) -> madopilot_status_t {
    if let Err(status) =
        // SAFETY: the caller supplies writable, correctly aligned output addresses.
        unsafe { boundary::begin_outputs(out_targets, "out_targets", out_error) }
    {
        return status;
    }
    hooks::reach(hooks::Site::Entry);

    // SAFETY: `out_error` was validated above.
    unsafe { report(out_error, run_discover(engine, operation, out_targets)) }
}

fn run_discover(
    engine: *const madopilot_engine_t,
    operation: *const crate::types::madopilot_operation_t,
    out_targets: *mut *mut madopilot_target_list_t,
) -> Result<(), Fault> {
    // SAFETY: the caller keeps the engine retained for the call.
    let Some(engine) = (unsafe { handle::borrow::<EngineHandle>(engine) }) else {
        return Err(Fault::abi("`engine` is null"));
    };
    // SAFETY: the caller keeps the operation structure readable for the call.
    let context = unsafe { operation::context(operation) }?;
    context.admit()?;

    let targets = engine
        .discover(context.inner())
        .map_err(error::facade(MADOPILOT_ERROR_CATEGORY_CAPTURE))?;
    let targets = targets
        .into_iter()
        .map(TargetRecord::new)
        .collect::<Vec<_>>();
    hooks::reach(hooks::Site::AfterTemporary);
    context.commit()?;

    // SAFETY: `out_targets` was validated by the entry before any work began.
    unsafe { out_targets.write(handle::into_raw(TargetList(targets))) };

    Ok(())
}

pub(crate) fn target_list_retain(targets: *const madopilot_target_list_t) -> madopilot_status_t {
    // SAFETY: the handle is null or one this module produced, and the caller
    // holds a live reference for the call.
    unsafe { handle::retain::<TargetList>(targets) }

    MADOPILOT_STATUS_OK
}

pub(crate) fn target_list_release(targets: *mut madopilot_target_list_t) -> madopilot_status_t {
    // SAFETY: as `target_list_retain`, and the caller is giving up its own
    // reference.
    unsafe { handle::release::<TargetList>(targets) }

    MADOPILOT_STATUS_OK
}

pub(crate) fn target_list_count(
    targets: *const madopilot_target_list_t,
    out_count: *mut usize,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable, correctly aligned output address.
    if let Err(fault) = unsafe { boundary::begin_scalar_out(out_count, "out_count", 0_usize) } {
        return fault.status();
    }
    hooks::reach(hooks::Site::Entry);

    // SAFETY: the caller keeps the handle retained for the call.
    let Some(targets) = (unsafe { handle::borrow::<TargetList>(targets) }) else {
        return crate::status::MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    // SAFETY: `out_count` was validated above.
    unsafe { boundary::commit_scalar(out_count, targets.targets().len()) };

    MADOPILOT_STATUS_OK
}

pub(crate) fn target_list_get(
    targets: *const madopilot_target_list_t,
    index: usize,
    out_target: *mut madopilot_target_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable output structure whose
    // `struct_size` it has set.
    let out = match unsafe { Out::begin(out_target) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);

    // SAFETY: the caller keeps the handle retained for the call.
    let Some(list) = (unsafe { handle::borrow::<TargetList>(targets) }) else {
        return crate::status::MADOPILOT_STATUS_INVALID_ARGUMENT;
    };

    let described = boundary::index_within(index, list.targets().len(), "target")
        .and_then(|index| describe_target(&list.targets()[index], out.declared_size()));
    match described {
        Ok(value) => {
            // SAFETY: `out` was validated above, and every view in `value`
            // borrows from `list`, whose handle stays retained for the call.
            unsafe { out.commit(value) };
            MADOPILOT_STATUS_OK
        }
        Err(fault) => fault.status(),
    }
}

fn describe_target(target: &TargetRecord, struct_size: u32) -> Result<madopilot_target_t, Fault> {
    let description = target.description();
    let mut coordinate_spaces = 0;
    for space in [
        mado_pilot::CoordinateSpace::CapturePixels,
        mado_pilot::CoordinateSpace::FrameNormalized,
        mado_pilot::CoordinateSpace::TargetNormalized,
        mado_pilot::CoordinateSpace::TargetLogical,
        mado_pilot::CoordinateSpace::DesktopLogical,
    ] {
        if description.coordinates().supports(space) {
            coordinate_spaces |= 1 << space_code(space);
        }
    }

    let placement_bit = if description
        .coordinates()
        .supports(mado_pilot::CoordinateSpace::TargetLogical)
    {
        MADOPILOT_TARGET_SUPPORTS_PLACEMENT
    } else {
        0
    };

    let capability = description.capability();
    let (kind_flag, kind) = match capability.kind() {
        Some(kind)
            if struct_size as usize
                >= covers!(madopilot_target_t, kind: madopilot_target_kind_t) =>
        {
            (
                MADOPILOT_TARGET_HAS_KIND,
                crate::input::target_kind_code(kind),
            )
        }
        _ => (0, crate::types::MADOPILOT_TARGET_KIND_UNKNOWN),
    };
    let (permission_flag, capture_permission) = match capability.capture_permission() {
        Some(permission)
            if struct_size as usize
                >= covers!(
                    madopilot_target_t,
                    capture_permission: madopilot_permission_kind_t
                ) =>
        {
            (
                MADOPILOT_TARGET_HAS_CAPTURE_PERMISSION,
                crate::input::permission_kind_code(permission),
            )
        }
        _ => (0, crate::types::MADOPILOT_PERMISSION_KIND_UNSPECIFIED),
    };

    Ok(madopilot_target_t {
        struct_size,
        flags: placement_bit | kind_flag | permission_flag,
        width: description.extent().width(),
        height: description.extent().height(),
        format: pixel_format_code(description.format())?,
        coordinate_spaces,
        name: madopilot_str_t::borrowed(description.name()),
        provider: madopilot_str_t::borrowed(description.provider().name()),
        target: target.ordinal(),
        kind,
        capture: crate::input::capability_support_code(capability.capture()),
        capture_permission,
        reserved: 0,
    })
}

/// Turns a fault into a status, reporting it through `out_error` when asked.
///
/// # Safety
///
/// `out_error` must be null or the address the entry already validated.
pub(crate) unsafe fn report(
    out_error: *mut *mut madopilot_error_t,
    outcome: Result<(), Fault>,
) -> madopilot_status_t {
    match outcome {
        Ok(()) => MADOPILOT_STATUS_OK,
        // SAFETY: forwarded unchanged from this function's own contract.
        Err(fault) => unsafe { error::emit(out_error, fault) },
    }
}
