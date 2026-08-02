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
use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mado_pilot_capture::CaptureFault;
use mado_pilot_core::{PermissionState, PixelExtent};

/// The internal surface version this build was written against.
pub(crate) const ABI_VERSION: u32 = 4;

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

/// `FrameInfo` carries a validated same-frame `SCStreamFrameInfoScreenRect`.
pub(crate) const FRAME_INFO_SCREEN_RECT: u32 = 1;
/// `FrameInfo` carries a bounded same-sample producer-capacity recommendation.
pub(crate) const FRAME_INFO_SURFACE_RECOMMENDATION: u32 = 1 << 1;

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
/// Makes the Rust frame body panic so the returned non-OK status traverses the
/// complete native terminal trampoline.
#[cfg(test)]
pub(crate) const PANIC_IN_RUST_CALLBACK: u32 = 32;
/// The asynchronous stop-completion trampoline, after its registering entry point
/// has returned.
#[cfg(test)]
pub(crate) const RAISE_IN_STOP_COMPLETION: u32 = 64;

#[repr(C)]
struct OpaqueInventory {
    _private: [u8; 0],
}

#[repr(C)]
struct OpaqueTarget {
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
    flags: u32,
    reserved: u32,
    pub(crate) display_time_nanos: u64,
    pub(crate) scale_factor: f64,
    pub(crate) content_origin_x: f64,
    pub(crate) content_origin_y: f64,
    screen_x: f64,
    screen_y: f64,
    screen_width: f64,
    screen_height: f64,
    recommended_surface_width: u32,
    recommended_surface_height: u32,
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
            flags: 0,
            reserved: 0,
            display_time_nanos: 0,
            scale_factor: 1.0,
            content_origin_x: 0.0,
            content_origin_y: 0.0,
            screen_x: 0.0,
            screen_y: 0.0,
            screen_width: 0.0,
            screen_height: 0.0,
            recommended_surface_width: 0,
            recommended_surface_height: 0,
        }
    }

    /// Builds a same-frame geometry report for pure Rust boundary tests.
    #[cfg(test)]
    pub(crate) const fn testing_screen_rect(
        extent: PixelExtent,
        scale_factor: f64,
        origin: (f64, f64),
        size: (f64, f64),
    ) -> Self {
        Self {
            struct_size: 0,
            pixel_format: PIXEL_BGRA8,
            content_width: extent.width(),
            content_height: extent.height(),
            surface_width: extent.width(),
            surface_height: extent.height(),
            flags: FRAME_INFO_SCREEN_RECT,
            reserved: 0,
            display_time_nanos: 0,
            scale_factor,
            content_origin_x: 0.0,
            content_origin_y: 0.0,
            screen_x: origin.0,
            screen_y: origin.1,
            screen_width: size.0,
            screen_height: size.1,
            recommended_surface_width: 0,
            recommended_surface_height: 0,
        }
    }

    /// Builds same-frame geometry with a private producer-capacity hint.
    #[cfg(test)]
    pub(crate) const fn testing_screen_rect_with_surface_recommendation(
        extent: PixelExtent,
        surface: PixelExtent,
        scale_factor: f64,
        origin: (f64, f64),
        size: (f64, f64),
        recommended: PixelExtent,
    ) -> Self {
        Self {
            struct_size: 0,
            pixel_format: PIXEL_BGRA8,
            content_width: extent.width(),
            content_height: extent.height(),
            surface_width: surface.width(),
            surface_height: surface.height(),
            flags: FRAME_INFO_SCREEN_RECT | FRAME_INFO_SURFACE_RECOMMENDATION,
            reserved: 0,
            display_time_nanos: 0,
            scale_factor,
            content_origin_x: 0.0,
            content_origin_y: 0.0,
            screen_x: origin.0,
            screen_y: origin.1,
            screen_width: size.0,
            screen_height: size.1,
            recommended_surface_width: recommended.width(),
            recommended_surface_height: recommended.height(),
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

    /// Returns the frame-attached onscreen rectangle after native validation.
    pub(crate) fn screen_rect(&self) -> Option<((f64, f64), (f64, f64))> {
        (self.flags & FRAME_INFO_SCREEN_RECT != 0).then_some((
            (self.screen_x, self.screen_y),
            (self.screen_width, self.screen_height),
        ))
    }

    /// Returns the same-sample producer capacity hint after repeating its bounds.
    pub(crate) fn recommended_surface_extent(&self) -> Option<PixelExtent> {
        if self.flags & FRAME_INFO_SURFACE_RECOMMENDATION == 0
            || self.recommended_surface_width == 0
            || self.recommended_surface_height == 0
            || self.recommended_surface_width > MAX_SURFACE_EXTENT
            || self.recommended_surface_height > MAX_SURFACE_EXTENT
        {
            return None;
        }
        let bytes = u64::from(self.recommended_surface_width)
            .checked_mul(u64::from(self.recommended_surface_height))?
            .checked_mul(4)?;
        (bytes <= MAX_SURFACE_BYTES).then(|| {
            PixelExtent::new(
                self.recommended_surface_width,
                self.recommended_surface_height,
            )
        })
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

    /// Constructs and retains the exact capture filter represented by `index`.
    pub(crate) fn target(&self, index: usize) -> Result<TargetToken, ShimStatus> {
        TargetToken::from_inventory(self, index)
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

/// An exact capture selection constructed from one discovery snapshot.
///
/// The native handle retains an `SCContentFilter`, not an `SCWindow` wrapper.
/// Numeric ids and process ids remain descriptive metadata and are never used to
/// re-resolve the selection at open.
#[derive(Clone)]
pub(crate) struct TargetToken {
    inner: Arc<TargetTokenInner>,
}

struct TargetTokenInner {
    handle: Option<NonNull<OpaqueTarget>>,
    #[cfg(test)]
    synthetic_identity: u64,
    #[cfg(test)]
    synthetic_live: AtomicBool,
}

// SAFETY: the native handle owns an immutable retained SCContentFilter and every
// operation on it is read-only.
unsafe impl Send for TargetTokenInner {}
// SAFETY: see the Send justification.
unsafe impl Sync for TargetTokenInner {}

impl TargetToken {
    fn from_inventory(inventory: &Inventory, index: usize) -> Result<Self, ShimStatus> {
        let mut target = std::ptr::null_mut();
        // SAFETY: the inventory is owned here and `target` is a writable output.
        let status =
            unsafe { mp_shim_inventory_target(inventory.handle.as_ptr(), index, &raw mut target) };
        ShimStatus::from_raw(status).into_result()?;
        let handle = NonNull::new(target).ok_or(ShimStatus::PlatformFailure)?;
        Ok(Self {
            inner: Arc::new(TargetTokenInner {
                handle: Some(handle),
                #[cfg(test)]
                synthetic_identity: 0,
                #[cfg(test)]
                synthetic_live: AtomicBool::new(true),
            }),
        })
    }

    fn as_ptr(&self) -> *const OpaqueTarget {
        self.inner
            .handle
            .map_or(std::ptr::null(), |handle| handle.as_ptr())
    }

    #[cfg(test)]
    pub(crate) fn synthetic(identity: u64) -> Self {
        Self {
            inner: Arc::new(TargetTokenInner {
                handle: None,
                synthetic_identity: identity,
                synthetic_live: AtomicBool::new(true),
            }),
        }
    }

    #[cfg(test)]
    pub(crate) fn synthetic_identity(&self) -> u64 {
        self.inner.synthetic_identity
    }

    /// Marks a synthetic retained selection as lost without changing its
    /// descriptive process or native-window metadata.
    #[cfg(test)]
    pub(crate) fn mark_synthetic_lost(&self) {
        self.inner.synthetic_live.store(false, Ordering::Release);
    }

    /// Reads bounds from the exact retained selection this token owns.
    pub(crate) fn input_bounds(&self) -> Result<NativeBounds, ShimStatus> {
        input_target_bounds(self)
    }
}

impl fmt::Debug for TargetToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TargetToken")
            .field("retained_selection", &self.inner.handle.is_some())
            .finish_non_exhaustive()
    }
}

impl Drop for TargetTokenInner {
    fn drop(&mut self) {
        let Some(handle) = self.handle else {
            return;
        };
        // SAFETY: the last Rust clone owns the one native handle and releases it once.
        unsafe { mp_shim_target_release(handle.as_ptr()) };
    }
}

/// What a session is being opened for.
#[derive(Debug, Clone)]
pub(crate) struct OpenRequest {
    /// [`KIND_WINDOW`] or [`KIND_DISPLAY`].
    pub(crate) kind: u32,
    /// Descriptive native metadata repeated to validate the retained selection.
    pub(crate) native_id: u64,
    /// The descriptive owning process, or zero for a display.
    pub(crate) owner_process: i64,
    /// The exact retained capture filter selected at discovery.
    pub(crate) target: TargetToken,
    /// The producer surface size, in capture pixels.
    pub(crate) extent: PixelExtent,
    /// Producer queue depth. The shim clamps this to its reviewed range.
    pub(crate) queue_depth: u32,
    /// How many detached buffers the session may lease at once.
    pub(crate) detached_budget: u32,
    /// Test-only delay inside the capture-start completion.
    pub(crate) testing_start_delay: Duration,
    /// Test-only delay inside the producer-stop completion.
    pub(crate) testing_stop_delay: Duration,
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
        frame_commit: FrameCommitCallback,
        stopped: StoppedCallback,
    ) -> Result<Self, ShimStatus> {
        validate_open_shape_and_metadata(request)?;
        if request.target.as_ptr().is_null() {
            return Err(ShimStatus::InvalidArgument);
        }
        let native = NativeOpenRequest {
            struct_size: u32::try_from(size_of::<NativeOpenRequest>())
                .expect("structure size fits u32"),
            kind: request.kind,
            native_id: request.native_id,
            owner_process: request.owner_process,
            target: request.target.as_ptr(),
            pixel_width: request.extent.width(),
            pixel_height: request.extent.height(),
            queue_depth: request.queue_depth,
            detached_budget: request.detached_budget,
            testing_start_delay_nanos: nanos(request.testing_start_delay),
            testing_stop_delay_nanos: nanos(request.testing_stop_delay),
            testing_raise_sites: request.testing_raise_sites,
            shows_cursor: false,
            reserved: [0; 3],
            callback_context: context,
            frame_callback: Some(frame),
            frame_commit_callback: Some(frame_commit),
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

/// Validates the platform-neutral part of an open request.
///
/// The native boundary repeats these checks and additionally validates the exact
/// retained target handle. Keeping the handle check separate gives tests a safe
/// pure seam without manufacturing an Objective-C object or weakening production
/// selection validation.
fn validate_open_shape_and_metadata(request: &OpenRequest) -> Result<(), ShimStatus> {
    let width = request.extent.width();
    let height = request.extent.height();
    let surface_bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(ShimStatus::InvalidArgument)?;
    if (request.kind != KIND_WINDOW && request.kind != KIND_DISPLAY)
        || (request.kind == KIND_WINDOW && request.owner_process <= 0)
        || width > MAX_SURFACE_EXTENT
        || height > MAX_SURFACE_EXTENT
        || surface_bytes > MAX_SURFACE_BYTES
        || request.detached_budget == 0
    {
        return Err(ShimStatus::InvalidArgument);
    }
    Ok(())
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

/// The bundle-launch context an authorization answer was read in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LaunchContext {
    /// A main bundle with an identifier: the context authorization is granted to.
    Bundled,
    /// A bare executable, whose grant follows the launching process instead.
    Unbundled,
    /// The shim could not establish the context.
    Unknown,
}

/// What public Security.framework inspection established about this code's
/// signature, independently of how it was launched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SignatureMode {
    /// Security.framework reported unsigned code and no signing identifier.
    Unsigned,
    /// Security.framework affirmatively rejected signed code as invalid.
    Invalid,
    /// The code is structurally valid and sealed without a certificate identity.
    AdHoc,
    /// The code is structurally valid and backed by a certificate identity.
    CertificateBacked,
    /// The public API was unavailable or could not establish a result.
    PlatformFailure,
}

/// Two independent axes needed to interpret a macOS authorization answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExecutionContext {
    launch: LaunchContext,
    signature: SignatureMode,
    /// Kept separate from ordinary diagnostics: an identifier is reportable only
    /// through a deliberate fixture evidence path, never ambient log context.
    signing_identifier: Option<String>,
}

impl ExecutionContext {
    fn from_raw(launch: u32, signature: u32, identifier: &[u8]) -> Self {
        let launch = match launch {
            1 => LaunchContext::Bundled,
            2 => LaunchContext::Unbundled,
            _ => LaunchContext::Unknown,
        };
        let mut signature = match signature {
            1 => SignatureMode::Unsigned,
            2 => SignatureMode::Invalid,
            3 => SignatureMode::AdHoc,
            4 => SignatureMode::CertificateBacked,
            _ => SignatureMode::PlatformFailure,
        };
        let signing_identifier = if matches!(
            signature,
            SignatureMode::AdHoc | SignatureMode::CertificateBacked
        ) {
            match str::from_utf8(identifier) {
                Ok(identifier) if !identifier.is_empty() => Some(identifier.to_owned()),
                _ => {
                    signature = SignatureMode::PlatformFailure;
                    None
                }
            }
        } else {
            None
        };
        Self {
            launch,
            signature,
            signing_identifier,
        }
    }

    /// Returns reviewed, static diagnostic context naming both independent axes.
    ///
    /// The dynamically read signing identifier is deliberately not interpolated:
    /// [`RedactedDiagnostic`](mado_pilot_core::RedactedDiagnostic) accepts only
    /// Adapter-authored literals so ambient diagnostics cannot leak host data.
    pub(crate) fn as_context(&self) -> &'static str {
        let _identifier_is_deliberately_redacted = self.signing_identifier.as_deref();
        match (self.launch, self.signature) {
            (LaunchContext::Bundled, SignatureMode::Unsigned) => {
                "probed from a bundled application with unsigned code"
            }
            (LaunchContext::Bundled, SignatureMode::Invalid) => {
                "probed from a bundled application with an invalid signature"
            }
            (LaunchContext::Bundled, SignatureMode::AdHoc) => {
                "probed from a bundled application with a valid ad-hoc signature"
            }
            (LaunchContext::Bundled, SignatureMode::CertificateBacked) => {
                "probed from a bundled application with a valid certificate-backed signature"
            }
            (LaunchContext::Bundled, SignatureMode::PlatformFailure) => {
                "probed from a bundled application with signature inspection unavailable"
            }
            (LaunchContext::Unbundled, SignatureMode::Unsigned) => {
                "probed from an unbundled executable with unsigned code"
            }
            (LaunchContext::Unbundled, SignatureMode::Invalid) => {
                "probed from an unbundled executable with an invalid signature"
            }
            (LaunchContext::Unbundled, SignatureMode::AdHoc) => {
                "probed from an unbundled executable with a valid ad-hoc signature"
            }
            (LaunchContext::Unbundled, SignatureMode::CertificateBacked) => {
                "probed from an unbundled executable with a valid certificate-backed signature"
            }
            (LaunchContext::Unbundled, SignatureMode::PlatformFailure) => {
                "probed from an unbundled executable with signature inspection unavailable"
            }
            (LaunchContext::Unknown, SignatureMode::Unsigned) => {
                "probed from an unknown launch context with unsigned code"
            }
            (LaunchContext::Unknown, SignatureMode::Invalid) => {
                "probed from an unknown launch context with an invalid signature"
            }
            (LaunchContext::Unknown, SignatureMode::AdHoc) => {
                "probed from an unknown launch context with a valid ad-hoc signature"
            }
            (LaunchContext::Unknown, SignatureMode::CertificateBacked) => {
                "probed from an unknown launch context with a valid certificate-backed signature"
            }
            (LaunchContext::Unknown, SignatureMode::PlatformFailure) => {
                "probed from an unknown launch context with signature inspection unavailable"
            }
        }
    }

    #[cfg(test)]
    fn signing_identifier(&self) -> Option<&str> {
        self.signing_identifier.as_deref()
    }
}

/// Reports the separate bundle-launch and signature contexts the probes use.
pub(crate) fn execution_context() -> ExecutionContext {
    const IDENTIFIER_CAPACITY: usize = 256;
    let mut launch = u32::MAX;
    let mut signature = u32::MAX;
    let mut identifier = [0u8; IDENTIFIER_CAPACITY];
    let mut identifier_len = usize::MAX;
    // SAFETY: every scalar output is writable, and `identifier` supplies the
    // capacity declared beside its pointer.
    let status = unsafe {
        mp_shim_execution_context(
            &raw mut launch,
            &raw mut signature,
            identifier.as_mut_ptr(),
            identifier.len(),
            &raw mut identifier_len,
        )
    };
    if ShimStatus::from_raw(status) != ShimStatus::Ok {
        return ExecutionContext::from_raw(0, 0, &[]);
    }
    let Some(identifier) = identifier.get(..identifier_len) else {
        return ExecutionContext::from_raw(launch, 0, &[]);
    };
    ExecutionContext::from_raw(launch, signature, identifier)
}

#[cfg(test)]
fn testing_classify_signature(
    signing_info_status: i32,
    validity_status: i32,
    has_identifier: bool,
    signature_flags: u32,
) -> Result<SignatureMode, ShimStatus> {
    let mut signature = u32::MAX;
    // SAFETY: `signature` is writable for one u32 and every other value is a
    // scalar consumed by the deterministic native classifier.
    let status = unsafe {
        mp_shim_testing_classify_signature(
            signing_info_status,
            validity_status,
            has_identifier,
            signature_flags,
            &raw mut signature,
        )
    };
    ShimStatus::from_raw(status).into_result()?;
    Ok(match signature {
        1 => SignatureMode::Unsigned,
        2 => SignatureMode::Invalid,
        3 => SignatureMode::AdHoc,
        4 => SignatureMode::CertificateBacked,
        _ => SignatureMode::PlatformFailure,
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

/// One pointer button, as the native surface numbers them.
pub(crate) const INPUT_BUTTON_PRIMARY: u32 = 0;
/// See [`INPUT_BUTTON_PRIMARY`].
pub(crate) const INPUT_BUTTON_SECONDARY: u32 = 1;
/// See [`INPUT_BUTTON_PRIMARY`].
pub(crate) const INPUT_BUTTON_MIDDLE: u32 = 2;
/// No button is involved, which a move reports and nothing else may.
pub(crate) const INPUT_BUTTON_NONE: u32 = u32::MAX;

/// What one pointer post does.
pub(crate) const INPUT_POINTER_MOVE: u32 = 0;
/// See [`INPUT_POINTER_MOVE`].
pub(crate) const INPUT_POINTER_PRESS: u32 = 1;
/// See [`INPUT_POINTER_MOVE`].
pub(crate) const INPUT_POINTER_RELEASE: u32 = 2;

/// Modifier state one posted event carries.
pub(crate) const INPUT_FLAG_SHIFT: u32 = 1;
/// See [`INPUT_FLAG_SHIFT`].
pub(crate) const INPUT_FLAG_CONTROL: u32 = 1 << 1;
/// See [`INPUT_FLAG_SHIFT`].
pub(crate) const INPUT_FLAG_ALT: u32 = 1 << 2;
/// See [`INPUT_FLAG_SHIFT`].
pub(crate) const INPUT_FLAG_META: u32 = 1 << 3;

/// The most UTF-16 units one posted text event carries, mirroring
/// `MP_SHIM_INPUT_MAX_TEXT_CHUNK`. A longer string is posted in chunks so the
/// count reported to a caller is the count that was posted.
pub(crate) const INPUT_MAX_TEXT_CHUNK: usize = 16;

/// The click count an ordinary single press or release declares.
pub(crate) const INPUT_SINGLE_CLICK: u64 = 1;

/// One target's live rectangle in the global point space, with its backing scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct NativeBounds {
    pub(crate) origin: (f64, f64),
    pub(crate) size: (f64, f64),
    pub(crate) scale: f64,
}

/// Reports the frontmost ordinary window and the process that owns it.
pub(crate) fn input_frontmost_window() -> Result<(u64, i64), ShimStatus> {
    let mut window = 0u64;
    let mut owner = 0i64;
    // SAFETY: both outputs are writable for one value each.
    let status = unsafe { mp_shim_input_frontmost_window(&raw mut window, &raw mut owner) };
    ShimStatus::from_raw(status).into_result()?;
    Ok((window, owner))
}

/// Reports one target's current rectangle, and thereby whether it still exists.
fn input_target_bounds(target: &TargetToken) -> Result<NativeBounds, ShimStatus> {
    let Some(handle) = target.inner.handle else {
        #[cfg(test)]
        {
            if target.inner.synthetic_live.load(Ordering::Acquire) {
                return Ok(NativeBounds {
                    origin: (0.0, 0.0),
                    size: (64.0, 48.0),
                    scale: 1.0,
                });
            }
            return Err(ShimStatus::TargetLost);
        }
        #[cfg(not(test))]
        return Err(ShimStatus::InvalidArgument);
    };
    let mut values = [0.0f64; 5];
    let [x, y, width, height, scale] = &mut values;
    // SAFETY: all five outputs are writable for one f64 each.
    let status = unsafe {
        mp_shim_input_target_bounds(
            handle.as_ptr(),
            &raw mut *x,
            &raw mut *y,
            &raw mut *width,
            &raw mut *height,
            &raw mut *scale,
        )
    };
    ShimStatus::from_raw(status).into_result()?;
    Ok(NativeBounds {
        origin: (values[0], values[1]),
        size: (values[2], values[3]),
        scale: values[4],
    })
}

/// Reads the pointer location in the global point space.
pub(crate) fn input_pointer_location() -> Result<(f64, f64), ShimStatus> {
    let mut x = 0.0f64;
    let mut y = 0.0f64;
    // SAFETY: both outputs are writable for one f64 each.
    let status = unsafe { mp_shim_input_pointer_location(&raw mut x, &raw mut y) };
    ShimStatus::from_raw(status).into_result()?;
    Ok((x, y))
}

/// Activates the application owning `owner_process`, presenting no interface.
pub(crate) fn input_activate_owner(owner_process: i64) -> Result<(), ShimStatus> {
    // SAFETY: the call takes one scalar and writes nothing.
    ShimStatus::from_raw(unsafe { mp_shim_input_activate_owner(owner_process) }).into_result()
}

/// Resolves one Unicode scalar to a key code reachable without modifiers.
pub(crate) fn input_resolve_character(scalar: u32) -> Result<u16, ShimStatus> {
    let mut key_code = 0u16;
    // SAFETY: the output is writable for one u16.
    let status = unsafe { mp_shim_input_resolve_character(scalar, &raw mut key_code) };
    ShimStatus::from_raw(status).into_result()?;
    Ok(key_code)
}

/// Posts one pointer event at a global point.
pub(crate) fn input_post_pointer(
    action: u32,
    button: u32,
    click_state: u64,
    location: (f64, f64),
    flags: u32,
) -> Result<(), ShimStatus> {
    // SAFETY: the call takes scalars only and writes nothing.
    ShimStatus::from_raw(unsafe {
        mp_shim_input_post_pointer(action, button, click_state, location.0, location.1, flags)
    })
    .into_result()
}

/// Posts one line-unit scroll, positive being down and right.
pub(crate) fn input_post_scroll(
    horizontal: i32,
    vertical: i32,
    flags: u32,
) -> Result<(), ShimStatus> {
    // SAFETY: the call takes scalars only and writes nothing.
    ShimStatus::from_raw(unsafe { mp_shim_input_post_scroll(horizontal, vertical, flags) })
        .into_result()
}

/// Posts one key event for a hardware key code.
pub(crate) fn input_post_key(key_code: u16, down: bool, flags: u32) -> Result<(), ShimStatus> {
    // SAFETY: the call takes scalars only and writes nothing.
    ShimStatus::from_raw(unsafe { mp_shim_input_post_key(key_code, down, flags) }).into_result()
}

/// Posts one bounded chunk of UTF-16 units as text.
///
/// The error carries how many units had already reached the target, because a
/// caller that stops mid-text has to report native effect it cannot take back.
pub(crate) fn input_post_text(units: &[u16], flags: u32) -> Result<(), (ShimStatus, usize)> {
    if units.is_empty() || units.len() > INPUT_MAX_TEXT_CHUNK {
        return Err((ShimStatus::InvalidArgument, 0));
    }
    let mut posted = 0usize;
    // SAFETY: `units` is a complete initialized slice whose length is passed
    // beside it, and `posted` is writable for one `usize`.
    let status =
        unsafe { mp_shim_input_post_text(units.as_ptr(), units.len(), flags, &raw mut posted) };
    match ShimStatus::from_raw(status) {
        ShimStatus::Ok => Ok(()),
        other => Err((other, posted)),
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

/// The frame-commit callback signature the shim invokes after native work succeeds.
pub(crate) type FrameCommitCallback = unsafe extern "C" fn(*mut c_void) -> u32;

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

/// Wraps a staged-frame commit so no panic or invalid context crosses back.
///
/// # Safety
///
/// `context` must be the value registered with the session that is calling.
pub(crate) unsafe fn contained_frame_commit_callback<C>(
    context: *mut c_void,
    body: impl FnOnce(&C) -> ShimStatus,
) -> u32 {
    if context.is_null() {
        return ShimStatus::InvalidArgument.as_raw();
    }
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: the caller guarantees `context` is the registered value for a
        // live session.
        let owner = unsafe { &*context.cast::<C>() };
        body(owner)
    }));
    match outcome {
        Ok(status) => status.as_raw(),
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

#[cfg(test)]
fn testing_terminalize_twice(
    context: *mut c_void,
    stopped: StoppedCallback,
    first: ShimStatus,
    second: ShimStatus,
) -> Result<(), ShimStatus> {
    // SAFETY: the test owns `context` for the call and the callback uses the same
    // signature as a real session registration.
    let status = unsafe {
        mp_shim_testing_terminalize_twice(context, Some(stopped), first.as_raw(), second.as_raw())
    };
    ShimStatus::from_raw(status).into_result()
}

#[cfg(test)]
fn testing_gate_retries(
    completion_delay: Duration,
    first_wait: Duration,
    second_wait: Duration,
) -> Result<[ShimStatus; 4], ShimStatus> {
    let mut statuses = [u32::MAX; 4];
    let [start_first, start_second, stop_first, stop_second] = &mut statuses;
    // SAFETY: every output is writable for one status and all durations are
    // encoded in the fixed-width nanosecond unit the shim declares.
    let status = unsafe {
        mp_shim_testing_gate_retries(
            nanos(completion_delay),
            nanos(first_wait),
            nanos(second_wait),
            &raw mut *start_first,
            &raw mut *start_second,
            &raw mut *stop_first,
            &raw mut *stop_second,
        )
    };
    ShimStatus::from_raw(status).into_result()?;
    Ok(statuses.map(ShimStatus::from_raw))
}

#[cfg(test)]
fn testing_stop_completion_exception() -> Result<(ShimStatus, bool), ShimStatus> {
    let mut completion = u32::MAX;
    let mut started = true;
    // SAFETY: both outputs are writable and the native seam contains its injected
    // exception before returning.
    let status =
        unsafe { mp_shim_testing_stop_completion_exception(&raw mut completion, &raw mut started) };
    ShimStatus::from_raw(status).into_result()?;
    Ok((ShimStatus::from_raw(completion), started))
}

#[cfg(test)]
fn testing_surface_recommendation(
    logical_size: (f64, f64),
    display_scale: f64,
) -> Option<PixelExtent> {
    let mut width = 0;
    let mut height = 0;
    // SAFETY: both outputs are writable and the native seam performs no allocation
    // or callback; it applies the exact helper used by frame delivery.
    let status = unsafe {
        mp_shim_testing_surface_recommendation(
            logical_size.0,
            logical_size.1,
            display_scale,
            &raw mut width,
            &raw mut height,
        )
    };
    (ShimStatus::from_raw(status) == ShimStatus::Ok).then(|| PixelExtent::new(width, height))
}

#[cfg(test)]
fn testing_input_text_second_allocation_failure() -> Result<(ShimStatus, [usize; 5]), ShimStatus> {
    let mut delivery = u32::MAX;
    let mut observations = [usize::MAX; 5];
    let [allocations, configurations, posts, releases, posted] = &mut observations;
    // SAFETY: every output is writable for its declared scalar type. The native
    // seam uses fake objects and never posts to the host system.
    let status = unsafe {
        mp_shim_testing_input_text_second_allocation_failure(
            &raw mut delivery,
            &raw mut *allocations,
            &raw mut *configurations,
            &raw mut *posts,
            &raw mut *releases,
            &raw mut *posted,
        )
    };
    ShimStatus::from_raw(status).into_result()?;
    Ok((ShimStatus::from_raw(delivery), observations))
}

#[repr(C)]
#[derive(Debug)]
struct NativeOpenRequest {
    struct_size: u32,
    kind: u32,
    native_id: u64,
    owner_process: i64,
    target: *const OpaqueTarget,
    pixel_width: u32,
    pixel_height: u32,
    queue_depth: u32,
    detached_budget: u32,
    testing_start_delay_nanos: u64,
    testing_stop_delay_nanos: u64,
    testing_raise_sites: u32,
    shows_cursor: bool,
    reserved: [u8; 3],
    callback_context: *mut c_void,
    frame_callback: Option<FrameCallback>,
    frame_commit_callback: Option<FrameCommitCallback>,
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
    fn mp_shim_execution_context(
        out_launch: *mut u32,
        out_signature: *mut u32,
        out_identifier: *mut u8,
        identifier_capacity: usize,
        out_identifier_len: *mut usize,
    ) -> u32;
    #[cfg(test)]
    fn mp_shim_testing_classify_signature(
        signing_info_status: i32,
        validity_status: i32,
        has_identifier: bool,
        signature_flags: u32,
        out_signature: *mut u32,
    ) -> u32;
    #[cfg(test)]
    fn mp_shim_classify_stream_error(code: i64) -> u32;
    fn mp_shim_monotonic_nanos(out_nanos: *mut u64) -> u32;
    #[cfg(test)]
    fn mp_shim_live_objects(out_live: *mut u64) -> u32;
    #[cfg(test)]
    fn mp_shim_testing_terminalize_twice(
        context: *mut c_void,
        stopped_callback: Option<StoppedCallback>,
        first: u32,
        second: u32,
    ) -> u32;
    #[cfg(test)]
    fn mp_shim_testing_gate_retries(
        completion_delay_nanos: u64,
        first_wait_nanos: u64,
        second_wait_nanos: u64,
        out_start_first: *mut u32,
        out_start_second: *mut u32,
        out_stop_first: *mut u32,
        out_stop_second: *mut u32,
    ) -> u32;
    #[cfg(test)]
    fn mp_shim_testing_stop_completion_exception(
        out_status: *mut u32,
        out_started: *mut bool,
    ) -> u32;
    #[cfg(test)]
    fn mp_shim_testing_surface_recommendation(
        logical_width: f64,
        logical_height: f64,
        display_scale: f64,
        out_width: *mut u32,
        out_height: *mut u32,
    ) -> u32;
    #[cfg(test)]
    fn mp_shim_testing_input_text_second_allocation_failure(
        out_delivery_status: *mut u32,
        out_allocations: *mut usize,
        out_configurations: *mut usize,
        out_posts: *mut usize,
        out_releases: *mut usize,
        out_posted: *mut usize,
    ) -> u32;

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
    fn mp_shim_inventory_target(
        inventory: *const OpaqueInventory,
        index: usize,
        out: *mut *mut OpaqueTarget,
    ) -> u32;
    fn mp_shim_inventory_release(inventory: *mut OpaqueInventory);
    fn mp_shim_target_release(target: *mut OpaqueTarget);
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

    fn mp_shim_input_frontmost_window(out_window_id: *mut u64, out_owner_pid: *mut i64) -> u32;
    fn mp_shim_input_target_bounds(
        target: *const OpaqueTarget,
        out_x: *mut f64,
        out_y: *mut f64,
        out_width: *mut f64,
        out_height: *mut f64,
        out_scale: *mut f64,
    ) -> u32;
    fn mp_shim_input_pointer_location(out_x: *mut f64, out_y: *mut f64) -> u32;
    fn mp_shim_input_activate_owner(owner_process: i64) -> u32;
    fn mp_shim_input_resolve_character(scalar: u32, out_key_code: *mut u16) -> u32;
    fn mp_shim_input_post_pointer(
        action: u32,
        button: u32,
        click_state: u64,
        x: f64,
        y: f64,
        flags: u32,
    ) -> u32;
    fn mp_shim_input_post_scroll(horizontal: i32, vertical: i32, flags: u32) -> u32;
    fn mp_shim_input_post_key(key_code: u16, down: bool, flags: u32) -> u32;
    fn mp_shim_input_post_text(
        units: *const u16,
        count: usize,
        flags: u32,
        out_posted: *mut usize,
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
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::Duration;

    use super::{
        ABI_VERSION, DEFAULT_NATIVE_WAIT, ExecutionContext, FrameInfo, KIND_DISPLAY, KIND_WINDOW,
        LaunchContext, MAX_NATIVE_WAIT, MAX_SURFACE_EXTENT, OpaqueFrame, OpenRequest, ShimStatus,
        SignatureMode, TargetToken, contained_frame_callback, contained_frame_commit_callback,
        contained_stopped_callback, declared_layout, execution_context, linked_layout,
        live_objects, monotonic_nanos, nanos, testing_classify_signature, testing_gate_retries,
        testing_input_text_second_allocation_failure, testing_stop_completion_exception,
        testing_surface_recommendation, testing_terminalize_twice,
        validate_open_shape_and_metadata,
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

    #[test]
    fn native_signature_classification_distinguishes_every_reported_state() {
        let classify = |information, validity, identifier, flags| {
            testing_classify_signature(information, validity, identifier, flags)
                .expect("the deterministic native classifier runs")
        };

        assert_eq!(classify(0, -67062, false, 0), SignatureMode::Unsigned);
        assert_eq!(
            classify(0, -67061, false, 0),
            SignatureMode::Invalid,
            "missing partial metadata cannot override an invalid signature"
        );
        assert_eq!(
            classify(0, 0, false, 0),
            SignatureMode::PlatformFailure,
            "successful validity without a signing identifier is contradictory"
        );
        assert_eq!(
            classify(0, -67062, true, 0),
            SignatureMode::Invalid,
            "an unsigned validity status with signed metadata is contradictory"
        );
        assert_eq!(classify(0, -67061, true, 0), SignatureMode::Invalid);
        assert_eq!(classify(0, 0, true, 0x0002), SignatureMode::AdHoc);
        assert_eq!(classify(0, 0, true, 0), SignatureMode::CertificateBacked);
        assert_eq!(classify(-4, 0, false, 0), SignatureMode::PlatformFailure);
        assert_eq!(
            classify(0, -4, true, 0),
            SignatureMode::PlatformFailure,
            "an unreadable platform result is not mislabeled as invalid code"
        );
    }

    #[test]
    fn bundle_launch_and_signature_mode_remain_independent_axes() {
        let bundled = ExecutionContext::from_raw(1, 3, b"dev.mado-pilot.fixture");
        let unbundled = ExecutionContext::from_raw(2, 3, b"dev.mado-pilot.fixture");

        assert_eq!(bundled.launch, LaunchContext::Bundled);
        assert_eq!(unbundled.launch, LaunchContext::Unbundled);
        assert_eq!(bundled.signature, SignatureMode::AdHoc);
        assert_eq!(unbundled.signature, SignatureMode::AdHoc);
        assert_ne!(bundled.as_context(), unbundled.as_context());
    }

    #[test]
    fn signing_identifier_is_explicitly_reportable_but_redacted_from_diagnostics() {
        let identifier = "dev.mado-pilot.fixture.private-host-value";
        let valid = ExecutionContext::from_raw(1, 4, identifier.as_bytes());
        assert_eq!(valid.signing_identifier(), Some(identifier));
        assert!(!valid.as_context().contains(identifier));

        let invalid = ExecutionContext::from_raw(1, 2, identifier.as_bytes());
        assert_eq!(invalid.signing_identifier(), None);
        assert!(!invalid.as_context().contains(identifier));
    }

    #[test]
    fn native_execution_context_preserves_identifier_invariants_without_permissions() {
        let context = execution_context();
        assert_eq!(
            context.signing_identifier().is_some(),
            matches!(
                context.signature,
                SignatureMode::AdHoc | SignatureMode::CertificateBacked
            )
        );
        if let Some(identifier) = context.signing_identifier() {
            assert!(!context.as_context().contains(identifier));
        }
    }

    #[test]
    fn native_surface_recommendations_use_raw_display_scale_and_existing_limits() {
        assert_eq!(
            testing_surface_recommendation((1718.0, 1108.0), 2.0),
            Some(PixelExtent::new(3436, 2216))
        );
        assert_eq!(testing_surface_recommendation((1718.0, 1108.0), 0.5), None);
        assert_eq!(testing_surface_recommendation((1718.0, 1108.0), 5.0), None);
        assert_eq!(
            testing_surface_recommendation((8193.0, 8193.0), 1.0),
            None,
            "a recommendation over the 256 MiB pair limit is refused"
        );
    }

    #[test]
    fn text_posts_nothing_when_the_release_event_cannot_be_allocated() {
        let (delivery, [allocations, configurations, posts, releases, posted]) =
            testing_input_text_second_allocation_failure().expect("native text failure seam runs");

        assert_eq!(delivery, ShimStatus::PlatformFailure);
        assert_eq!(
            allocations, 2,
            "the forced failure is the second allocation"
        );
        assert_eq!(configurations, 0, "no half-pair is configured alone");
        assert_eq!(posts, 0, "the key-down never reaches the system");
        assert_eq!(releases, 1, "the first native event is released on failure");
        assert_eq!(posted, 0, "the caller observes no native effect");
    }

    #[test]
    fn rust_rejects_an_out_of_bounds_surface_recommendation_at_the_abi_boundary() {
        let content = PixelExtent::new(64, 48);
        let over_axis = FrameInfo::testing_screen_rect_with_surface_recommendation(
            content,
            content,
            1.0,
            (0.0, 0.0),
            (64.0, 48.0),
            PixelExtent::new(MAX_SURFACE_EXTENT + 1, 1),
        );
        let over_bytes = FrameInfo::testing_screen_rect_with_surface_recommendation(
            content,
            content,
            1.0,
            (0.0, 0.0),
            (64.0, 48.0),
            PixelExtent::new(8193, 8193),
        );

        assert_eq!(over_axis.recommended_surface_extent(), None);
        assert_eq!(over_bytes.recommended_surface_extent(), None);
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
        let validate = |width: u32, height: u32| {
            let request = OpenRequest {
                kind: KIND_DISPLAY,
                native_id: u64::from(u32::MAX),
                owner_process: 0,
                target: TargetToken::synthetic(1),
                extent: PixelExtent::new(width, height),
                queue_depth: 3,
                detached_budget: 8,
                testing_start_delay: Duration::ZERO,
                testing_stop_delay: Duration::ZERO,
                testing_raise_sites: 0,
            };
            validate_open_shape_and_metadata(&request).err()
        };

        // Both axes are inside the per-axis limit and their product is four
        // gibibytes. The axis bound is what protects the conversions; it never
        // protected the allocation, and the two are thirty-two times apart.
        assert_eq!(
            validate(MAX_SURFACE_EXTENT, MAX_SURFACE_EXTENT),
            Some(ShimStatus::InvalidArgument)
        );

        // 8192 x 8192 BGRA is exactly the ceiling, so the boundary is inclusive: this
        // request is structurally valid. The retained native-token check remains a
        // separate, mandatory production check and is intentionally not bypassed.
        assert_eq!(validate(8192, 8192), None);
    }

    /// A window opened without a real owning process is refused before anything else.
    ///
    /// Deterministic anywhere, for the reason the byte-ceiling case above is: the
    /// request is validated before the framework is loaded or authorization consulted.
    #[test]
    fn a_window_request_without_an_owning_process_is_refused() {
        let validate = |kind: u32, owner_process: i64| {
            let request = OpenRequest {
                kind,
                native_id: u64::from(u32::MAX),
                owner_process,
                target: TargetToken::synthetic(1),
                extent: PixelExtent::new(64, 48),
                queue_depth: 3,
                detached_budget: 8,
                testing_start_delay: Duration::ZERO,
                testing_stop_delay: Duration::ZERO,
                testing_raise_sites: 0,
            };
            validate_open_shape_and_metadata(&request).err()
        };

        // Discovery no longer lists a window whose owner the framework did not
        // name, so zero reaching a window request cannot match the boundary shape
        // of a real selection and is refused.
        assert_eq!(
            validate(KIND_WINDOW, 0),
            Some(ShimStatus::InvalidArgument),
            "a listed window selection always carries its owning process metadata"
        );
        assert_eq!(validate(KIND_WINDOW, -1), Some(ShimStatus::InvalidArgument));

        // A display has no owner and the field is not consulted for one, so the same
        // zero is not an error there.
        assert_eq!(validate(KIND_DISPLAY, 0), None);
    }

    #[test]
    fn native_and_rust_callback_failures_terminalize_once_with_the_first_typed_status() {
        struct Probe {
            calls: AtomicU64,
            status: AtomicU64,
        }

        unsafe extern "C" fn record(context: *mut c_void, status: u32) {
            // SAFETY: every invocation below passes a live `Probe` for the duration
            // of the native test seam.
            let probe = unsafe { &*context.cast::<Probe>() };
            probe.status.store(u64::from(status), Ordering::Release);
            probe.calls.fetch_add(1, Ordering::AcqRel);
        }

        for first in [ShimStatus::NativeException, ShimStatus::PlatformFailure] {
            let probe = Probe {
                calls: AtomicU64::new(0),
                status: AtomicU64::new(u64::MAX),
            };
            testing_terminalize_twice(
                (&raw const probe).cast_mut().cast(),
                record,
                first,
                ShimStatus::StoppedBySystem,
            )
            .expect("the native terminal seam runs");

            assert_eq!(probe.calls.load(Ordering::Acquire), 1);
            assert_eq!(
                probe.status.load(Ordering::Acquire),
                u64::from(first.as_raw()),
                "the first callback fault is the terminal outcome"
            );
            assert_eq!(first.fault(), CaptureFault::SourceInvalid);
        }
    }

    #[test]
    fn delayed_native_start_and_stop_gates_resume_after_the_first_wait_expires() {
        let statuses = testing_gate_retries(
            Duration::from_millis(30),
            Duration::from_millis(1),
            Duration::from_secs(1),
        )
        .expect("native retry gates");

        assert_eq!(
            statuses,
            [
                ShimStatus::TimedOut,
                ShimStatus::Ok,
                ShimStatus::TimedOut,
                ShimStatus::Ok,
            ],
            "both asynchronous phases preserve pending work for the later close wait"
        );
    }

    #[test]
    fn a_stop_completion_exception_is_contained_and_settles_its_gate() {
        let (status, started) = testing_stop_completion_exception()
            .expect("the deterministic native stop-completion seam runs");

        assert_eq!(status, ShimStatus::NativeException);
        assert!(
            !started,
            "the completion clears started even when it raises"
        );
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
    fn a_panicking_frame_commit_becomes_a_typed_failure_rather_than_an_abort() {
        struct Owner;
        let owner = Owner;

        let status = without_panic_output(|| {
            // SAFETY: the context is a live Owner and the body contains no
            // reference borrowed from native code.
            unsafe {
                contained_frame_commit_callback::<Owner>(
                    (&raw const owner).cast_mut().cast::<c_void>(),
                    |_owner| panic!("a frame commit panicked"),
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
