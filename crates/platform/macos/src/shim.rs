//! The Rust side of the internal native boundary.
//!
//! Everything `unsafe` in this package lives here or in the callback trampolines
//! that call into it. Above this module the adapter sees owned handles, borrowed
//! views tied to them, and typed statuses; it never sees a raw pointer.
//!
//! The declarations mirror `native/madopilot_macos_shim.h` by hand rather than
//! through a generator, for the reason `docs/adr/0004-c-header-authorship-and-abi-verification.md`
//! records for the public C header: a hand-written declaration is reviewable, and
//! a test asserts that the two sides agree on version and structure sizes rather
//! than trusting that they do.

use std::ffi::{c_char, c_void};
use std::fmt;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::slice;
use std::str;
use std::time::Duration;

use mado_pilot_capture::CaptureFault;
use mado_pilot_core::{PermissionState, PixelExtent};

/// The internal surface version this build was written against.
pub(crate) const ABI_VERSION: u32 = 1;

/// Largest wait the shim is ever asked for, so one native call cannot consume a
/// caller's whole budget.
pub(crate) const MAX_NATIVE_WAIT: Duration = Duration::from_secs(2);

/// Default wait for a native call made outside a caller's operation, such as the
/// close an implicit drop performs.
pub(crate) const DEFAULT_NATIVE_WAIT: Duration = Duration::from_secs(1);

/// What kind of desktop object a native target is.
pub(crate) const KIND_WINDOW: u32 = 0;
/// See [`KIND_WINDOW`].
pub(crate) const KIND_DISPLAY: u32 = 1;

/// The only pixel layout the shim publishes.
pub(crate) const PIXEL_BGRA8: u32 = 0;

/// The largest producer surface extent the shim accepts, mirroring
/// `MP_SHIM_MAX_PIXEL_EXTENT`. Asking beyond it would be refused, so the Adapter
/// does not ask.
pub(crate) const MAX_SURFACE_EXTENT: u32 = 32768;

/// The largest producer surface the shim accepts in bytes, mirroring
/// `MP_SHIM_MAX_SURFACE_BYTES`.
///
/// Separate from the extent above because it bounds a different quantity: two axes
/// inside that limit still multiply to four gibibytes. Mirrored here for the same
/// reason the extent is — the Adapter does not ask for what would be refused.
pub(crate) const MAX_SURFACE_BYTES: u64 = 268_435_456;

/// Test seams the shim exposes for the ADR 0012 injection positions.
#[cfg(test)]
pub(crate) const RAISE_AT_START: u32 = 1;
/// See [`RAISE_AT_START`].
#[cfg(test)]
pub(crate) const RAISE_BEFORE_CALLBACK: u32 = 2;
/// See [`RAISE_AT_START`].
#[cfg(test)]
pub(crate) const RAISE_AFTER_CALLBACK: u32 = 4;
/// See [`RAISE_AT_START`].
#[cfg(test)]
pub(crate) const RAISE_AT_TEARDOWN: u32 = 8;
/// The capture-start completion block, which [`RAISE_AT_START`] cannot reach because
/// it fires after the wait that block signals. See [`RAISE_AT_START`].
#[cfg(test)]
pub(crate) const RAISE_IN_START_COMPLETION: u32 = 16;

#[repr(C)]
struct OpaqueInventory {
    _private: [u8; 0],
}

#[repr(C)]
struct OpaqueSession {
    _private: [u8; 0],
}

#[repr(C)]
pub(crate) struct OpaqueFrame {
    _private: [u8; 0],
}

/// The frame handle a callback trampoline receives, before it is made safe.
pub(crate) type OpaqueFrameHandle = OpaqueFrame;

/// One discovered window or display, as the shim reports it.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct TargetInfo {
    struct_size: u32,
    pub(crate) kind: u32,
    pub(crate) native_id: u64,
    pub(crate) owner_process: i64,
    pub(crate) pixel_width: u32,
    pub(crate) pixel_height: u32,
    pub(crate) logical_x: f64,
    pub(crate) logical_y: f64,
    pub(crate) logical_width: f64,
    pub(crate) logical_height: f64,
    pub(crate) backing_scale: f64,
    pub(crate) name_len: u32,
    reserved: u32,
}

impl TargetInfo {
    fn requested() -> Self {
        Self {
            struct_size: u32::try_from(size_of::<Self>()).expect("structure size fits u32"),
            kind: 0,
            native_id: 0,
            owner_process: 0,
            pixel_width: 0,
            pixel_height: 0,
            logical_x: 0.0,
            logical_y: 0.0,
            logical_width: 0.0,
            logical_height: 0.0,
            backing_scale: 0.0,
            name_len: 0,
            reserved: 0,
        }
    }

    /// Returns the content extent, when the shim reported a usable one.
    pub(crate) fn extent(&self) -> Option<PixelExtent> {
        (self.pixel_width > 0 && self.pixel_height > 0)
            .then(|| PixelExtent::new(self.pixel_width, self.pixel_height))
    }
}

/// The layout and frame-time geometry of one produced frame.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct FrameInfo {
    struct_size: u32,
    pub(crate) pixel_format: u32,
    pub(crate) content_width: u32,
    pub(crate) content_height: u32,
    pub(crate) surface_width: u32,
    pub(crate) surface_height: u32,
    reserved: [u32; 2],
    pub(crate) display_time_nanos: u64,
    pub(crate) scale_factor: f64,
    pub(crate) content_origin_x: f64,
    pub(crate) content_origin_y: f64,
}

impl FrameInfo {
    /// Builds a report describing nothing, for the containment tests.
    #[cfg(test)]
    pub(crate) const fn empty() -> Self {
        Self {
            struct_size: 0,
            pixel_format: PIXEL_BGRA8,
            content_width: 0,
            content_height: 0,
            surface_width: 0,
            surface_height: 0,
            reserved: [0; 2],
            display_time_nanos: 0,
            scale_factor: 1.0,
            content_origin_x: 0.0,
            content_origin_y: 0.0,
        }
    }

    /// Returns the content extent, when the frame reported a usable one.
    pub(crate) fn extent(&self) -> Option<PixelExtent> {
        (self.content_width > 0 && self.content_height > 0)
            .then(|| PixelExtent::new(self.content_width, self.content_height))
    }

    /// Returns the extent of the producer surface the content sits in.
    pub(crate) fn surface_extent(&self) -> Option<PixelExtent> {
        (self.surface_width > 0 && self.surface_height > 0)
            .then(|| PixelExtent::new(self.surface_width, self.surface_height))
    }
}

/// A borrowed frame, valid only for the callback that received it.
pub(crate) struct BorrowedFrame<'callback> {
    handle: NonNull<OpaqueFrame>,
    lifetime: PhantomData<&'callback ()>,
}

impl BorrowedFrame<'_> {
    /// Copies the frame's content into session-owned storage.
    ///
    /// The result is independent of the producer surface, which is what keeps a
    /// retaining consumer from stalling capture.
    pub(crate) fn detach(&self) -> Result<DetachedFrame, ShimStatus> {
        let mut detached = std::ptr::null_mut();
        // SAFETY: the handle is the one the shim passed to this callback and is
        // valid for its duration, and `detached` is a writable output.
        let status = unsafe { mp_shim_frame_detach(self.handle.as_ptr(), &raw mut detached) };
        match (ShimStatus::from_raw(status), NonNull::new(detached)) {
            (ShimStatus::Ok, Some(handle)) => Ok(DetachedFrame { handle }),
            (ShimStatus::Ok, None) => Err(ShimStatus::PlatformFailure),
            (status, _) => Err(status),
        }
    }
}

impl fmt::Debug for BorrowedFrame<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("BorrowedFrame").finish()
    }
}

/// One frame's detached, immutable native storage.
pub(crate) struct DetachedFrame {
    handle: NonNull<OpaqueFrame>,
}

// SAFETY: a detached frame owns its own Core Video buffer, is never mutated after
// construction, and every shim operation on it takes the buffer's own lock.
unsafe impl Send for DetachedFrame {}
// SAFETY: see the Send justification.
unsafe impl Sync for DetachedFrame {}

impl DetachedFrame {
    /// Copies the content into `destination` at exactly `stride` bytes per row.
    pub(crate) fn copy_out(&self, destination: &mut [u8], stride: usize) -> Result<(), ShimStatus> {
        // SAFETY: the handle is owned by this value, the destination pointer and
        // length come from one live slice, and the shim validates the stride
        // against the frame's own row length before writing anything.
        let status = unsafe {
            mp_shim_frame_copy_out(
                self.handle.as_ptr(),
                destination.as_mut_ptr(),
                destination.len(),
                stride as u64,
            )
        };
        ShimStatus::from_raw(status).into_result()
    }
}

impl fmt::Debug for DetachedFrame {
    /// Formats nothing about the content, which is captured screen pixels.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("DetachedFrame").finish()
    }
}

impl Drop for DetachedFrame {
    fn drop(&mut self) {
        // SAFETY: the handle was produced by a successful detach, is owned here,
        // and is released exactly once.
        unsafe { mp_shim_frame_release(self.handle.as_ptr()) };
    }
}

/// One snapshot of the currently shareable windows and displays.
///
/// Deliberately neither `Send` nor `Sync`: it is consumed by the discovery pass
/// that produced it, and the names it hands out are borrowed from it.
pub(crate) struct Inventory {
    handle: NonNull<OpaqueInventory>,
}

impl Inventory {
    /// Enumerates the current shareable content, without presenting a picker.
    pub(crate) fn acquire(wait: Duration) -> Result<Self, ShimStatus> {
        let mut inventory = std::ptr::null_mut();
        // SAFETY: `inventory` is a writable output for one handle.
        let status = unsafe { mp_shim_inventory_acquire(nanos(wait), &raw mut inventory) };
        match (ShimStatus::from_raw(status), NonNull::new(inventory)) {
            (ShimStatus::Ok, Some(handle)) => Ok(Self { handle }),
            (ShimStatus::Ok, None) => Err(ShimStatus::PlatformFailure),
            (status, _) => Err(status),
        }
    }

    /// Returns how many targets the snapshot holds.
    pub(crate) fn len(&self) -> usize {
        let mut count = 0;
        // SAFETY: the handle is owned here and `count` is a writable output.
        let status = unsafe { mp_shim_inventory_count(self.handle.as_ptr(), &raw mut count) };
        if ShimStatus::from_raw(status) == ShimStatus::Ok {
            count
        } else {
            0
        }
    }

    /// Returns the entry at `index`.
    pub(crate) fn entry(&self, index: usize) -> Result<TargetInfo, ShimStatus> {
        let mut info = TargetInfo::requested();
        // SAFETY: the handle is owned here, and `info` carries the size this
        // build compiled against so the shim can refuse a mismatch.
        let status = unsafe { mp_shim_inventory_entry(self.handle.as_ptr(), index, &raw mut info) };
        ShimStatus::from_raw(status).into_result().map(|()| info)
    }

    /// Borrows the descriptive name of the entry at `index`.
    ///
    /// The borrow is tied to this snapshot, which is what keeps the bytes alive
    /// for as long as the name is readable.
    pub(crate) fn name(&self, index: usize) -> Result<&str, ShimStatus> {
        let mut bytes = std::ptr::null();
        let mut len = 0;
        // SAFETY: both outputs are writable, and the handle is owned here.
        let status = unsafe {
            mp_shim_inventory_name(self.handle.as_ptr(), index, &raw mut bytes, &raw mut len)
        };
        ShimStatus::from_raw(status).into_result()?;
        if bytes.is_null() {
            return Ok("");
        }
        // SAFETY: the shim documents these bytes as valid for `len` and alive
        // until this handle is released, which outlives the returned borrow.
        let view = unsafe { slice::from_raw_parts(bytes, len) };
        Ok(str::from_utf8(view).unwrap_or(""))
    }
}

impl fmt::Debug for Inventory {
    /// Formats the count only. A window title is desktop content.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Inventory")
            .field("targets", &self.len())
            .finish()
    }
}

impl Drop for Inventory {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful acquire and is released once.
        unsafe { mp_shim_inventory_release(self.handle.as_ptr()) };
    }
}

/// What a session is being opened for.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OpenRequest {
    /// [`KIND_WINDOW`] or [`KIND_DISPLAY`].
    pub(crate) kind: u32,
    /// The native identity the discovery pass recorded.
    pub(crate) native_id: u64,
    /// The owning process the identity was validated against, or zero for a display.
    ///
    /// Carried so that the shim's own lookup applies the rule the caller applied to
    /// its own snapshot: a window number is recycled, so the number alone does not
    /// name the incarnation that was discovered.
    pub(crate) owner_process: i64,
    /// The producer surface size, in capture pixels.
    pub(crate) extent: PixelExtent,
    /// Producer queue depth. The shim clamps this to its reviewed range.
    pub(crate) queue_depth: u32,
    /// How many detached buffers the session may lease at once.
    pub(crate) detached_budget: u32,
    /// How long the one native query this open performs may take.
    pub(crate) wait: Duration,
    /// Zero in the product. See the shim's `MP_SHIM_RAISE_*` seams.
    pub(crate) testing_raise_sites: u32,
}

/// One open native session.
pub(crate) struct Session {
    handle: NonNull<OpaqueSession>,
}

// SAFETY: every mutable field the shim keeps for a session is guarded by its own
// mutex or is atomic, and the callbacks it registers are already delivered on a
// queue the caller does not own.
unsafe impl Send for Session {}
// SAFETY: see the Send justification.
unsafe impl Sync for Session {}

impl Session {
    /// Creates a session and registers `callbacks`. Does not start the producer.
    ///
    /// `context` is passed to both callbacks unchanged and is never dereferenced
    /// by the shim. The caller must keep whatever it points at alive until a
    /// successful [`Session::fence`].
    pub(crate) fn open(
        request: &OpenRequest,
        context: *mut c_void,
        frame: FrameCallback,
        stopped: StoppedCallback,
    ) -> Result<Self, ShimStatus> {
        let native = NativeOpenRequest {
            struct_size: u32::try_from(size_of::<NativeOpenRequest>())
                .expect("structure size fits u32"),
            kind: request.kind,
            native_id: request.native_id,
            owner_process: request.owner_process,
            pixel_width: request.extent.width(),
            pixel_height: request.extent.height(),
            queue_depth: request.queue_depth,
            detached_budget: request.detached_budget,
            timeout_nanos: nanos(request.wait),
            testing_raise_sites: request.testing_raise_sites,
            shows_cursor: false,
            reserved: [0; 3],
            callback_context: context,
            frame_callback: Some(frame),
            stopped_callback: Some(stopped),
        };
        let mut session = std::ptr::null_mut();
        // SAFETY: `native` outlives the call, carries its own size, and
        // `session` is a writable output for one handle.
        let status = unsafe { mp_shim_session_open(&raw const native, &raw mut session) };
        match (ShimStatus::from_raw(status), NonNull::new(session)) {
            (ShimStatus::Ok, Some(handle)) => Ok(Self { handle }),
            (ShimStatus::Ok, None) => Err(ShimStatus::PlatformFailure),
            (status, _) => Err(status),
        }
    }

    /// Starts the producer.
    pub(crate) fn start(&self, wait: Duration) -> Result<(), ShimStatus> {
        // SAFETY: the handle is owned here.
        let status = unsafe { mp_shim_session_start(self.handle.as_ptr(), nanos(wait)) };
        ShimStatus::from_raw(status).into_result()
    }

    /// Applies a new producer surface size and retires the detached pool.
    pub(crate) fn reconfigure(
        &self,
        extent: PixelExtent,
        wait: Duration,
    ) -> Result<(), ShimStatus> {
        // SAFETY: the handle is owned here and the extent is validated by the
        // shim against its own bounds.
        let status = unsafe {
            mp_shim_session_reconfigure(
                self.handle.as_ptr(),
                extent.width(),
                extent.height(),
                nanos(wait),
            )
        };
        ShimStatus::from_raw(status).into_result()
    }

    /// Stops admitting callbacks. Idempotent and never blocks.
    pub(crate) fn disable_callbacks(&self) {
        // SAFETY: the handle is owned here.
        let _status = unsafe { mp_shim_session_disable_callbacks(self.handle.as_ptr()) };
    }

    /// Returns only when no callback is in flight.
    pub(crate) fn fence(&self, wait: Duration) -> Result<(), ShimStatus> {
        // SAFETY: the handle is owned here.
        let status = unsafe { mp_shim_session_fence(self.handle.as_ptr(), nanos(wait)) };
        ShimStatus::from_raw(status).into_result()
    }

    /// Stops the producer and releases native state. Idempotent.
    pub(crate) fn close(&self, wait: Duration) -> Result<(), ShimStatus> {
        // SAFETY: the handle is owned here.
        let status = unsafe { mp_shim_session_close(self.handle.as_ptr(), nanos(wait)) };
        ShimStatus::from_raw(status).into_result()
    }

    /// Reports how many detached buffers the session currently has leased.
    pub(crate) fn leased(&self) -> u64 {
        let mut leased = 0;
        // SAFETY: the handle is owned here and `leased` is a writable output.
        let status = unsafe { mp_shim_session_leased(self.handle.as_ptr(), &raw mut leased) };
        if ShimStatus::from_raw(status) == ShimStatus::Ok {
            leased
        } else {
            0
        }
    }

    /// Reports how many native objects the session still owns.
    pub(crate) fn live_objects(&self) -> u64 {
        let mut live = 0;
        // SAFETY: the handle is owned here and `live` is a writable output.
        let status = unsafe { mp_shim_session_live_objects(self.handle.as_ptr(), &raw mut live) };
        if ShimStatus::from_raw(status) == ShimStatus::Ok {
            live
        } else {
            0
        }
    }
}

impl fmt::Debug for Session {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Session")
            .field("leased", &self.leased())
            .field("live_objects", &self.live_objects())
            .finish()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: releasing the handle closes it first when the owner did not,
        // and the handle is released exactly once.
        unsafe { mp_shim_session_release(self.handle.as_ptr()) };
    }
}

/// Reports whether this host offers the capture capability at all.
pub(crate) fn capture_available() -> Result<(), ShimStatus> {
    // SAFETY: the call takes no arguments and writes nothing.
    ShimStatus::from_raw(unsafe { mp_shim_capture_available() }).into_result()
}

/// Reads the Screen Recording authorization without requesting it.
pub(crate) fn probe_screen_capture() -> Result<PermissionState, ShimStatus> {
    probe(|state| {
        // SAFETY: `state` is a writable output for one u32.
        unsafe { mp_shim_probe_screen_capture(state) }
    })
}

/// Reads the Accessibility authorization without requesting it.
pub(crate) fn probe_accessibility() -> Result<PermissionState, ShimStatus> {
    probe(|state| {
        // SAFETY: `state` is a writable output for one u32.
        unsafe { mp_shim_probe_accessibility(state) }
    })
}

fn probe(read: impl FnOnce(*mut u32) -> u32) -> Result<PermissionState, ShimStatus> {
    let mut state = u32::MAX;
    let status = read(&raw mut state);
    ShimStatus::from_raw(status).into_result()?;
    Ok(match state {
        0 => PermissionState::Granted,
        1 => PermissionState::NotGranted,
        2 => PermissionState::Unavailable,
        // A state this build does not know about is not read as authorization.
        _ => PermissionState::Unknown,
    })
}

/// The signing and launch context an authorization answer was read in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LaunchContext {
    /// A main bundle with an identifier: the context authorization is granted to.
    Bundled,
    /// A bare executable, whose grant follows the launching process instead.
    Unbundled,
    /// The shim could not establish the context.
    Unknown,
}

impl LaunchContext {
    /// Returns Adapter-authored diagnostic context naming this launch context.
    ///
    /// A literal rather than an owned string, because the redacted diagnostic
    /// surface accepts only text that exists in reviewed source.
    pub(crate) const fn as_context(self) -> &'static str {
        match self {
            LaunchContext::Bundled => "probed from a bundled application context",
            LaunchContext::Unbundled => "probed from an unbundled executable context",
            LaunchContext::Unknown => "probed from an unestablished launch context",
        }
    }
}

/// Reports the signing and launch context the probes are read in.
pub(crate) fn launch_context() -> LaunchContext {
    let mut context = u32::MAX;
    // SAFETY: `context` is a writable output for one u32.
    let status = unsafe { mp_shim_launch_context(&raw mut context) };
    if ShimStatus::from_raw(status) != ShimStatus::Ok {
        return LaunchContext::Unknown;
    }
    match context {
        1 => LaunchContext::Bundled,
        2 => LaunchContext::Unbundled,
        _ => LaunchContext::Unknown,
    }
}

/// The target's placement as it is now, with the scale of the display holding it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LivePlacement {
    /// Origin and size in global points: x, y, width, height.
    pub(crate) frame: [f64; 4],
    /// Capture pixels per point for the display the target is on.
    pub(crate) display_scale: f64,
}

/// Reads the target's placement as it is now, in global points.
///
/// For the frame-time transform an Adapter records with a publication rather than
/// for live consultation afterwards. The display scale comes back with it because
/// a target that has moved needs a producer surface sized for its new display, and
/// the frame in hand reports the scale it was produced at rather than that one.
pub(crate) fn current_placement(kind: u32, native_id: u64) -> Result<LivePlacement, ShimStatus> {
    let mut frame = [0.0; 4];
    let mut display_scale = 0.0;
    // SAFETY: `frame` is writable for the four doubles the shim documents and
    // `display_scale` for one.
    let status = unsafe {
        mp_shim_current_placement(kind, native_id, frame.as_mut_ptr(), &raw mut display_scale)
    };
    ShimStatus::from_raw(status)
        .into_result()
        .map(|()| LivePlacement {
            frame,
            display_scale,
        })
}

/// Classifies one capture-framework error code as the shim maps it.
///
/// Only the mapping tests read this; the Adapter receives statuses the shim has
/// already classified from a live error object, domain included.
#[cfg(test)]
pub(crate) fn classify_stream_error(code: i64) -> ShimStatus {
    // SAFETY: the call reads no memory and writes none.
    ShimStatus::from_raw(unsafe { mp_shim_classify_stream_error(code) })
}

/// Reads the host clock the producer timestamps frames on, in nanoseconds.
pub(crate) fn monotonic_nanos() -> Option<u64> {
    let mut nanos = 0;
    // SAFETY: `nanos` is a writable output for one u64.
    let status = unsafe { mp_shim_monotonic_nanos(&raw mut nanos) };
    (ShimStatus::from_raw(status) == ShimStatus::Ok).then_some(nanos)
}

/// Reports how many native objects the shim owns process-wide.
///
/// Only the ownership cases ADR 0012 requires read this, so it is compiled for
/// tests alone rather than left as an operation the Adapter could come to rely on.
#[cfg(test)]
pub(crate) fn live_objects() -> u64 {
    let mut live = 0;
    // SAFETY: `live` is a writable output for one u64.
    let status = unsafe { mp_shim_live_objects(&raw mut live) };
    if ShimStatus::from_raw(status) == ShimStatus::Ok {
        live
    } else {
        0
    }
}

/// Returns the surface version and structure sizes the linked shim was built to.
pub(crate) fn linked_layout() -> (u32, [u32; 3]) {
    // SAFETY: the version call takes no arguments.
    let version = unsafe { mp_shim_abi_version() };
    let mut sizes = [0; 3];
    let [target_info, frame_info, open_request] = &mut sizes;
    // SAFETY: all three outputs are writable for one u32 each.
    let status = unsafe {
        mp_shim_struct_sizes(
            &raw mut *target_info,
            &raw mut *frame_info,
            &raw mut *open_request,
        )
    };
    if ShimStatus::from_raw(status) != ShimStatus::Ok {
        return (version, [0; 3]);
    }
    (version, sizes)
}

/// The sizes this build compiled its mirrored structures to.
pub(crate) fn declared_layout() -> [u32; 3] {
    [
        u32::try_from(size_of::<TargetInfo>()).expect("structure size fits u32"),
        u32::try_from(size_of::<FrameInfo>()).expect("structure size fits u32"),
        u32::try_from(size_of::<NativeOpenRequest>()).expect("structure size fits u32"),
    ]
}

/// What a shim entry point reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShimStatus {
    /// The operation succeeded.
    Ok,
    /// A pointer, length, structure size, or enumerated value failed validation.
    InvalidArgument,
    /// This host does not offer the capability at all.
    Unsupported,
    /// The operating system refused for want of authorization.
    PermissionDenied,
    /// The target existed when it was discovered and does not now.
    TargetLost,
    /// The platform reported a failure none of the others explains.
    PlatformFailure,
    /// A native exception was contained at the boundary.
    NativeException,
    /// The producer stopped accepting work.
    Closed,
    /// A bounded native wait reached the budget it was given.
    TimedOut,
    /// Every unit of the session's detached-storage budget is leased.
    BudgetExhausted,
    /// The producer surface could not be read as a complete frame.
    FrameIncomplete,
    /// The user stopped the stream through a system control.
    StoppedByUser,
    /// The operating system ended the stream without naming a cause.
    StoppedBySystem,
    /// A status this build does not know about.
    Unrecognized(u32),
}

impl ShimStatus {
    /// Reads a raw status, treating an unknown value as a failure rather than as
    /// success, because a newer shim must not silently widen what succeeds.
    pub(crate) const fn from_raw(status: u32) -> Self {
        match status {
            0 => ShimStatus::Ok,
            1 => ShimStatus::InvalidArgument,
            2 => ShimStatus::Unsupported,
            3 => ShimStatus::PermissionDenied,
            4 => ShimStatus::TargetLost,
            5 => ShimStatus::PlatformFailure,
            6 => ShimStatus::NativeException,
            7 => ShimStatus::Closed,
            8 => ShimStatus::TimedOut,
            9 => ShimStatus::BudgetExhausted,
            10 => ShimStatus::FrameIncomplete,
            11 => ShimStatus::StoppedByUser,
            12 => ShimStatus::StoppedBySystem,
            other => ShimStatus::Unrecognized(other),
        }
    }

    /// Returns the raw value, for the callback returns the shim reads.
    pub(crate) const fn as_raw(self) -> u32 {
        match self {
            ShimStatus::Ok => 0,
            ShimStatus::InvalidArgument => 1,
            ShimStatus::Unsupported => 2,
            ShimStatus::PermissionDenied => 3,
            ShimStatus::TargetLost => 4,
            ShimStatus::PlatformFailure => 5,
            ShimStatus::NativeException => 6,
            ShimStatus::Closed => 7,
            ShimStatus::TimedOut => 8,
            ShimStatus::BudgetExhausted => 9,
            ShimStatus::FrameIncomplete => 10,
            ShimStatus::StoppedByUser => 11,
            ShimStatus::StoppedBySystem => 12,
            ShimStatus::Unrecognized(other) => other,
        }
    }

    const fn into_result(self) -> Result<(), Self> {
        match self {
            ShimStatus::Ok => Ok(()),
            other => Err(other),
        }
    }

    /// Returns the capture fault this status reports as.
    ///
    /// Every variant is matched. A status added later is a compile error in this
    /// package, which is where the decision about what it means belongs.
    pub(crate) const fn fault(self) -> CaptureFault {
        match self {
            // A successful status has no fault, but a caller that asked for one
            // is in a failure path already, so this reports the conservative
            // outcome rather than inventing success.
            ShimStatus::Ok | ShimStatus::PlatformFailure | ShimStatus::NativeException => {
                CaptureFault::SourceInvalid
            }
            ShimStatus::InvalidArgument => CaptureFault::InconsistentDescriptor,
            ShimStatus::Unsupported | ShimStatus::Unrecognized(_) => {
                CaptureFault::UnsupportedOption
            }
            ShimStatus::PermissionDenied => CaptureFault::AccessDenied,
            ShimStatus::TargetLost => CaptureFault::TargetLost,
            ShimStatus::Closed | ShimStatus::StoppedByUser => CaptureFault::ExplicitlyStopped,
            // The system ended the stream and did not say why. The stream is over
            // and no frame follows, which is what this reports; an Adapter that can
            // establish a cause reports that instead.
            ShimStatus::StoppedBySystem => CaptureFault::StreamEnded,
            ShimStatus::TimedOut => CaptureFault::SourceInvalid,
            ShimStatus::BudgetExhausted => CaptureFault::StorageBudgetExhausted,
            ShimStatus::FrameIncomplete => CaptureFault::SourceInvalid,
        }
    }
}

impl From<ShimStatus> for mado_pilot_core::Error {
    fn from(status: ShimStatus) -> Self {
        status.fault().into()
    }
}

fn nanos(wait: Duration) -> u64 {
    let bounded = if wait > MAX_NATIVE_WAIT {
        MAX_NATIVE_WAIT
    } else {
        wait
    };
    u64::try_from(bounded.as_nanos()).unwrap_or(u64::MAX)
}

/// The frame callback signature the shim invokes.
///
/// The implementation must contain its own panics: a panic escaping an
/// `extern "C"` callback aborts the process, which ADR 0012 measured.
pub(crate) type FrameCallback =
    unsafe extern "C" fn(*mut c_void, *mut OpaqueFrame, *const FrameInfo) -> u32;

/// The producer-stopped callback signature the shim invokes.
pub(crate) type StoppedCallback = unsafe extern "C" fn(*mut c_void, u32);

/// Wraps a frame callback body so no panic and no invalid handle crosses back.
///
/// # Safety
///
/// `frame` and `info` must be the pointers the shim passed to this callback, and
/// `context` must be the value registered with the session that is calling.
pub(crate) unsafe fn contained_frame_callback<C>(
    context: *mut c_void,
    frame: *mut OpaqueFrame,
    info: *const FrameInfo,
    body: impl FnOnce(&C, BorrowedFrame<'_>, &FrameInfo) -> ShimStatus,
) -> u32 {
    let Some(handle) = NonNull::new(frame) else {
        return ShimStatus::InvalidArgument.as_raw();
    };
    if context.is_null() || info.is_null() {
        return ShimStatus::InvalidArgument.as_raw();
    }
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: the caller guarantees `context` is the registered value for a
        // live session and `info` points to one complete report the shim owns
        // for the duration of this call.
        let owner = unsafe { &*context.cast::<C>() };
        // SAFETY: as above for `info`.
        let report = unsafe { &*info };
        let borrowed = BorrowedFrame {
            handle,
            lifetime: PhantomData,
        };
        body(owner, borrowed, report)
    }));
    match outcome {
        Ok(status) => status.as_raw(),
        // A panicking host callback becomes a typed failure rather than an abort.
        Err(_) => ShimStatus::PlatformFailure.as_raw(),
    }
}

/// Wraps a producer-stopped callback body so no panic crosses back.
///
/// # Safety
///
/// `context` must be the value registered with the session that is calling.
pub(crate) unsafe fn contained_stopped_callback<C>(
    context: *mut c_void,
    status: u32,
    body: impl FnOnce(&C, ShimStatus),
) {
    if context.is_null() {
        return;
    }
    let _outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: the caller guarantees `context` is the registered value for a
        // live session.
        let owner = unsafe { &*context.cast::<C>() };
        body(owner, ShimStatus::from_raw(status));
    }));
}

#[repr(C)]
#[derive(Debug)]
struct NativeOpenRequest {
    struct_size: u32,
    kind: u32,
    native_id: u64,
    owner_process: i64,
    pixel_width: u32,
    pixel_height: u32,
    queue_depth: u32,
    detached_budget: u32,
    timeout_nanos: u64,
    testing_raise_sites: u32,
    shows_cursor: bool,
    reserved: [u8; 3],
    callback_context: *mut c_void,
    frame_callback: Option<FrameCallback>,
    stopped_callback: Option<StoppedCallback>,
}

unsafe extern "C" {
    fn mp_shim_abi_version() -> u32;
    fn mp_shim_struct_sizes(
        out_target_info: *mut u32,
        out_frame_info: *mut u32,
        out_open_request: *mut u32,
    ) -> u32;
    fn mp_shim_capture_available() -> u32;
    fn mp_shim_probe_screen_capture(out_state: *mut u32) -> u32;
    fn mp_shim_probe_accessibility(out_state: *mut u32) -> u32;
    fn mp_shim_launch_context(out_context: *mut u32) -> u32;
    #[cfg(test)]
    fn mp_shim_classify_stream_error(code: i64) -> u32;
    fn mp_shim_monotonic_nanos(out_nanos: *mut u64) -> u32;
    #[cfg(test)]
    fn mp_shim_live_objects(out_live: *mut u64) -> u32;

    fn mp_shim_inventory_acquire(timeout_nanos: u64, out: *mut *mut OpaqueInventory) -> u32;
    fn mp_shim_inventory_count(inventory: *const OpaqueInventory, out_count: *mut usize) -> u32;
    fn mp_shim_inventory_entry(
        inventory: *const OpaqueInventory,
        index: usize,
        out_info: *mut TargetInfo,
    ) -> u32;
    fn mp_shim_inventory_name(
        inventory: *const OpaqueInventory,
        index: usize,
        out_bytes: *mut *const u8,
        out_len: *mut usize,
    ) -> u32;
    fn mp_shim_inventory_release(inventory: *mut OpaqueInventory);

    fn mp_shim_current_placement(
        kind: u32,
        native_id: u64,
        out_frame: *mut f64,
        out_scale: *mut f64,
    ) -> u32;

    fn mp_shim_session_open(request: *const NativeOpenRequest, out: *mut *mut OpaqueSession)
    -> u32;
    fn mp_shim_session_start(session: *mut OpaqueSession, timeout_nanos: u64) -> u32;
    fn mp_shim_session_reconfigure(
        session: *mut OpaqueSession,
        pixel_width: u32,
        pixel_height: u32,
        timeout_nanos: u64,
    ) -> u32;
    fn mp_shim_session_disable_callbacks(session: *mut OpaqueSession) -> u32;
    fn mp_shim_session_fence(session: *mut OpaqueSession, timeout_nanos: u64) -> u32;
    fn mp_shim_session_close(session: *mut OpaqueSession, timeout_nanos: u64) -> u32;
    fn mp_shim_session_release(session: *mut OpaqueSession);
    fn mp_shim_session_leased(session: *const OpaqueSession, out_leased: *mut u64) -> u32;
    fn mp_shim_session_live_objects(session: *const OpaqueSession, out_live: *mut u64) -> u32;

    fn mp_shim_frame_detach(borrowed: *mut OpaqueFrame, out: *mut *mut OpaqueFrame) -> u32;
    fn mp_shim_frame_release(frame: *mut OpaqueFrame);
    fn mp_shim_frame_copy_out(
        frame: *const OpaqueFrame,
        destination: *mut u8,
        capacity: usize,
        destination_stride: u64,
    ) -> u32;
}

// The C header spells the borrowed name view as `const uint8_t *`; this keeps the
// Rust side honest about the element type it reads back.
const _: () = assert!(size_of::<c_char>() == size_of::<u8>());

#[cfg(test)]
mod tests {
    use std::ffi::c_void;
    use std::panic;
    use std::ptr::NonNull;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use super::{
        ABI_VERSION, DEFAULT_NATIVE_WAIT, FrameInfo, KIND_DISPLAY, KIND_WINDOW, MAX_NATIVE_WAIT,
        MAX_SURFACE_EXTENT, OpaqueFrame, OpenRequest, Session, ShimStatus,
        contained_frame_callback, contained_stopped_callback, declared_layout, linked_layout,
        live_objects, monotonic_nanos, nanos,
    };
    use mado_pilot_capture::CaptureFault;
    use mado_pilot_core::PixelExtent;

    /// Runs `body` with panic reporting suppressed, so a deliberately panicking
    /// callback does not print a backtrace over the test output.
    fn without_panic_output<T>(body: impl FnOnce() -> T) -> T {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(|_info| {}));
        let outcome = body();
        panic::set_hook(previous);
        outcome
    }

    #[test]
    fn the_linked_shim_matches_the_declarations_this_build_mirrors() {
        let (version, sizes) = linked_layout();

        assert_eq!(version, ABI_VERSION);
        assert_eq!(sizes, declared_layout());
    }

    /// A surface the shim would refuse to allocate is refused before it tries.
    ///
    /// Deterministic on any host, and that is a property of where the check sits: the
    /// request is validated before the framework is loaded and before authorization is
    /// consulted, so this runs where the capture scenarios report a skip — including a
    /// continuous-integration runner, which is the only place the pool allocation
    /// itself could never be attempted.
    #[test]
    fn a_surface_larger_than_the_byte_ceiling_is_refused_before_anything_is_allocated() {
        extern "C" fn unreached_frame(
            _context: *mut c_void,
            _frame: *mut OpaqueFrame,
            _info: *const FrameInfo,
        ) -> u32 {
            unreachable!("the request is refused before a producer exists")
        }
        extern "C" fn unreached_stopped(_context: *mut c_void, _status: u32) {
            unreachable!("the request is refused before a producer exists")
        }

        // `kCGNullDirectDisplay`, so no host resolves it. The request is refused
        // before the lookup in the case this is about, and by the lookup in the case
        // it is contrasted against, and neither opens a producer either way.
        let open = |width: u32, height: u32| {
            let request = OpenRequest {
                kind: KIND_DISPLAY,
                native_id: u64::from(u32::MAX),
                owner_process: 0,
                extent: PixelExtent::new(width, height),
                queue_depth: 3,
                detached_budget: 8,
                wait: DEFAULT_NATIVE_WAIT,
                testing_raise_sites: 0,
            };
            Session::open(
                &request,
                std::ptr::null_mut(),
                unreached_frame,
                unreached_stopped,
            )
            .err()
        };

        // Both axes are inside the per-axis limit and their product is four
        // gibibytes. The axis bound is what protects the conversions; it never
        // protected the allocation, and the two are thirty-two times apart.
        assert_eq!(
            open(MAX_SURFACE_EXTENT, MAX_SURFACE_EXTENT),
            Some(ShimStatus::InvalidArgument)
        );

        // 8192 x 8192 BGRA is exactly the ceiling, so the boundary is inclusive: this
        // request is refused for whatever this host says about an absent display or
        // its authorization, but never as a malformed one.
        assert_ne!(open(8192, 8192), Some(ShimStatus::InvalidArgument));
    }

    /// A window opened without a real owning process is refused before anything else.
    ///
    /// Deterministic anywhere, for the reason the byte-ceiling case above is: the
    /// request is validated before the framework is loaded or authorization consulted.
    #[test]
    fn a_window_request_without_an_owning_process_is_refused() {
        extern "C" fn unreached_frame(
            _context: *mut c_void,
            _frame: *mut OpaqueFrame,
            _info: *const FrameInfo,
        ) -> u32 {
            unreachable!("the request is refused before a producer exists")
        }
        extern "C" fn unreached_stopped(_context: *mut c_void, _status: u32) {
            unreachable!("the request is refused before a producer exists")
        }

        let open = |kind: u32, owner_process: i64| {
            let request = OpenRequest {
                kind,
                native_id: u64::from(u32::MAX),
                owner_process,
                extent: PixelExtent::new(64, 48),
                queue_depth: 3,
                detached_budget: 8,
                wait: DEFAULT_NATIVE_WAIT,
                testing_raise_sites: 0,
            };
            Session::open(
                &request,
                std::ptr::null_mut(),
                unreached_frame,
                unreached_stopped,
            )
            .err()
        };

        // Zero is what a window whose owner the framework did not name used to record,
        // and it matched the next such window rather than distinguishing it. Discovery
        // no longer lists one, so zero reaching a window request is a fabricated
        // identity and is refused rather than compared.
        assert_eq!(
            open(KIND_WINDOW, 0),
            Some(ShimStatus::InvalidArgument),
            "a window with no owning process names no incarnation"
        );
        assert_eq!(open(KIND_WINDOW, -1), Some(ShimStatus::InvalidArgument));

        // A display has no owner and the field is not consulted for one, so the same
        // zero is not an error there.
        assert_ne!(open(KIND_DISPLAY, 0), Some(ShimStatus::InvalidArgument));
    }

    #[test]
    fn an_unrecognized_status_is_never_read_as_success() {
        let status = ShimStatus::from_raw(4242);

        assert_eq!(status, ShimStatus::Unrecognized(4242));
        assert_eq!(status.fault(), CaptureFault::UnsupportedOption);
        assert_eq!(status.as_raw(), 4242);
    }

    #[test]
    fn every_status_round_trips_through_its_raw_value() {
        for raw in 0..=12 {
            assert_eq!(ShimStatus::from_raw(raw).as_raw(), raw);
        }
    }

    /// Pins every capture-framework error code the Adapter's behaviour depends on
    /// against the value `SCError.h` gives it.
    ///
    /// This table was wrong once, transcribed from memory rather than from the
    /// SDK, and the cost was a user stopping capture being reported as a capture
    /// failure while two real failures were reported as a lost target. Nothing in
    /// the type system catches that, so it is asserted here per code.
    #[test]
    fn stream_error_codes_map_to_the_outcomes_the_sdk_names() {
        use super::classify_stream_error;

        // Authorization refused for the stream this process asked for.
        assert_eq!(classify_stream_error(-3801), ShimStatus::PermissionDenied);
        // A start that failed is a failure, not a lifecycle outcome.
        assert_eq!(classify_stream_error(-3802), ShimStatus::PlatformFailure);
        // -3805 and -3806 are a dropped and a mismatched application connection.
        // They were previously read as benign state results, which reported a real
        // failure as a lost target.
        assert_eq!(classify_stream_error(-3805), ShimStatus::PlatformFailure);
        assert_eq!(classify_stream_error(-3806), ShimStatus::PlatformFailure);
        // Our own call finding the stream already in the state it asked for.
        assert_eq!(classify_stream_error(-3807), ShimStatus::Closed);
        assert_eq!(classify_stream_error(-3808), ShimStatus::Closed);
        // The source is no longer listable, which is target loss.
        for absent in [-3813, -3814, -3815] {
            assert_eq!(
                classify_stream_error(absent),
                ShimStatus::TargetLost,
                "{absent}"
            );
        }
        // The user stopped the stream. This is the one the wrong table lost.
        assert_eq!(classify_stream_error(-3817), ShimStatus::StoppedByUser);
        // The system ended the stream without naming a cause.
        assert_eq!(classify_stream_error(-3821), ShimStatus::StoppedBySystem);
        // Anything unlisted stays a platform failure rather than being guessed at.
        assert_eq!(classify_stream_error(-3812), ShimStatus::PlatformFailure);
        assert_eq!(classify_stream_error(0), ShimStatus::PlatformFailure);
    }

    #[test]
    fn a_deliberate_stop_is_a_lifecycle_outcome_and_not_a_capture_failure() {
        assert_eq!(
            ShimStatus::StoppedByUser.fault(),
            CaptureFault::ExplicitlyStopped
        );
        assert_eq!(
            ShimStatus::StoppedByUser.fault().status(),
            mado_pilot_core::Status::Closed
        );
        assert_eq!(
            ShimStatus::StoppedBySystem.fault(),
            CaptureFault::StreamEnded
        );
        assert_eq!(
            ShimStatus::StoppedBySystem.fault().status(),
            mado_pilot_core::Status::Closed
        );
    }

    #[test]
    fn a_native_wait_is_bounded_by_the_reviewed_ceiling() {
        assert_eq!(nanos(Duration::from_millis(5)), 5_000_000);
        assert_eq!(
            nanos(Duration::from_secs(3600)),
            u64::try_from(MAX_NATIVE_WAIT.as_nanos()).expect("ceiling fits u64")
        );
        assert!(DEFAULT_NATIVE_WAIT <= MAX_NATIVE_WAIT);
    }

    #[test]
    fn a_panicking_frame_callback_becomes_a_typed_failure_rather_than_an_abort() {
        struct Owner;
        let owner = Owner;
        let info = FrameInfo::empty();
        let frame = NonNull::<OpaqueFrame>::dangling().as_ptr();

        let status = without_panic_output(|| {
            // SAFETY: the context is a live Owner, the report is a live
            // FrameInfo, and the body below never dereferences the frame handle.
            unsafe {
                contained_frame_callback::<Owner>(
                    (&raw const owner).cast_mut().cast::<c_void>(),
                    frame,
                    &raw const info,
                    |_owner, _borrowed, _report| panic!("a host callback panicked"),
                )
            }
        });

        assert_eq!(ShimStatus::from_raw(status), ShimStatus::PlatformFailure);
    }

    #[test]
    fn a_frame_callback_refuses_a_missing_handle_before_reaching_its_body() {
        struct Owner;
        let owner = Owner;
        let info = FrameInfo::empty();
        let reached = AtomicBool::new(false);

        // SAFETY: the context and report are live; the handle is deliberately null.
        let status = unsafe {
            contained_frame_callback::<Owner>(
                (&raw const owner).cast_mut().cast::<c_void>(),
                std::ptr::null_mut(),
                &raw const info,
                |_owner, _borrowed, _report| {
                    reached.store(true, Ordering::Release);
                    ShimStatus::Ok
                },
            )
        };

        assert_eq!(ShimStatus::from_raw(status), ShimStatus::InvalidArgument);
        assert!(
            !reached.load(Ordering::Acquire),
            "a body that had run would have been handed an invalid handle"
        );
    }

    #[test]
    fn a_panicking_stopped_callback_is_contained_without_a_status_to_return() {
        struct Owner;
        let owner = Owner;

        without_panic_output(|| {
            // SAFETY: the context is a live Owner.
            unsafe {
                contained_stopped_callback::<Owner>(
                    (&raw const owner).cast_mut().cast::<c_void>(),
                    ShimStatus::Closed.as_raw(),
                    |_owner, _status| panic!("a host stop handler panicked"),
                );
            }
        });
    }

    #[test]
    fn the_shim_owns_no_native_object_before_a_session_exists() {
        // Every test in this package shares one process, so this asserts a floor
        // rather than an exact count: nothing here leaks its way upward.
        assert!(live_objects() < 64);
    }

    #[test]
    fn the_producer_clock_advances() {
        let first = monotonic_nanos().expect("the host clock is readable");
        let second = monotonic_nanos().expect("the host clock is readable");

        assert!(second >= first);
        assert!(first > 0);
    }

    #[test]
    fn storage_pressure_and_target_loss_stay_distinguishable() {
        assert_eq!(
            ShimStatus::BudgetExhausted.fault(),
            CaptureFault::StorageBudgetExhausted
        );
        assert_eq!(ShimStatus::TargetLost.fault(), CaptureFault::TargetLost);
        assert_eq!(
            ShimStatus::PermissionDenied.fault(),
            CaptureFault::AccessDenied
        );
        assert_eq!(ShimStatus::Closed.fault(), CaptureFault::ExplicitlyStopped);
    }
}
