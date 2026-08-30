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
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Catches one Rust panic and disposes of its payload without allowing a
/// panicking payload destructor to unwind into native code.
pub(crate) fn catch_panic<T>(body: impl FnOnce() -> T) -> Result<T, ()> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(value) => Ok(value),
        Err(payload) => {
            if let Err(payload) =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(payload)))
            {
                // The replacement payload already escaped its own destructor.
                // There is no safe destructor call left on this failing path.
                let _leaked = std::mem::ManuallyDrop::new(payload);
            }
            Err(())
        }
    }
}

use mado_pilot_capture::CaptureFault;
use mado_pilot_core::{OperationContext, PermissionState, PixelExtent};

/// The internal surface version this build was written against.
pub(crate) const ABI_VERSION: u32 = 22;

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
const TARGET_INFO_PROCESS_DIRECTED: u32 = 1;

/// `FrameInfo` carries a validated same-frame `SCStreamFrameInfoScreenRect`.
pub(crate) const FRAME_INFO_SCREEN_RECT: u32 = 1;
/// `FrameInfo` carries a bounded same-sample producer-capacity recommendation.
pub(crate) const FRAME_INFO_SURFACE_RECOMMENDATION: u32 = 1 << 1;

#[cfg(feature = "sck-suspension-diagnostics")]
pub(crate) const SCK_STATUS_COMPLETE: u32 = 0;
#[cfg(feature = "sck-suspension-diagnostics")]
pub(crate) const SCK_STATUS_IDLE: u32 = 1;
#[cfg(feature = "sck-suspension-diagnostics")]
pub(crate) const SCK_STATUS_BLANK: u32 = 2;
#[cfg(feature = "sck-suspension-diagnostics")]
pub(crate) const SCK_STATUS_STARTED: u32 = 3;
#[cfg(feature = "sck-suspension-diagnostics")]
pub(crate) const SCK_STATUS_SUSPENDED: u32 = 4;
#[cfg(feature = "sck-suspension-diagnostics")]
pub(crate) const SCK_STATUS_STOPPED: u32 = 5;
#[cfg(feature = "sck-suspension-diagnostics")]
pub(crate) const SCK_STATUS_MISSING: u32 = 6;
#[cfg(feature = "sck-suspension-diagnostics")]
pub(crate) const SCK_STATUS_UNKNOWN: u32 = 7;
#[cfg(feature = "sck-suspension-diagnostics")]
pub(crate) const SCK_STATUS_KIND_COUNT: usize = 8;
#[cfg(feature = "sck-suspension-diagnostics")]
pub(crate) const SCK_DIAGNOSTIC_TRANSITION_CAPACITY: usize = 16;
#[cfg(feature = "sck-suspension-diagnostics")]
pub(crate) const SCK_DIAGNOSTIC_DISPLAY_CAPACITY: usize = 2;

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
/// Keeps one Rust frame callback admitted beyond the implicit-drop fence wait.
#[cfg(test)]
pub(crate) const DELAY_IN_RUST_CALLBACK: u32 = 128;
/// Refuses the capture-start semaphore factory before framework submission.
#[cfg(test)]
pub(crate) const FAIL_START_SEMAPHORE_ALLOCATION: u32 = 256;
/// Refuses the retained session hold before framework submission.
#[cfg(test)]
pub(crate) const FAIL_START_HOLD_ALLOCATION: u32 = 512;
/// Refuses the reconfiguration semaphore factory before framework submission.
#[cfg(test)]
pub(crate) const FAIL_RECONFIGURE_SEMAPHORE_ALLOCATION: u32 = 1024;

#[cfg(test)]
const TEST_FIXTURE_SEMAPHORE_ALLOCATION_FAILURE: u32 = 0;
#[cfg(test)]
const TEST_FIXTURE_COMPLETION_EXCEPTION: u32 = 1;
#[cfg(test)]
const TEST_FIXTURE_LATE_COMPLETION: u32 = 2;
#[cfg(test)]
const TEST_FIXTURE_SUCCESSFUL_RELEASE: u32 = 3;
#[cfg(test)]
const TEST_FIXTURE_VALIDATION_FAILURE: u32 = 4;
#[cfg(test)]
const TEST_FIXTURE_HANDLE_ALLOCATION_FAILURE: u32 = 5;
#[cfg(test)]
const TEST_FIXTURE_REAPER_HANDOFF: u32 = 6;
#[cfg(test)]
const TEST_FIXTURE_RELEASE_REAPER_HANDOFF: u32 = 7;

#[repr(C)]
struct OpaqueInventory {
    _private: [u8; 0],
}

#[repr(C)]
struct OpaqueTarget {
    _private: [u8; 0],
}

#[repr(C)]
struct OpaqueProcessEventSource {
    _private: [u8; 0],
}

#[repr(C)]
struct OpaquePreparedInput {
    _private: [u8; 0],
}

#[repr(C)]
struct OpaqueSession {
    _private: [u8; 0],
}

#[cfg(all(test, feature = "sck-suspension-diagnostics"))]
#[repr(C)]
struct OpaqueSckDiagnosticsProbe {
    _private: [u8; 0],
}

#[repr(C)]
pub(crate) struct OpaqueFrame {
    _private: [u8; 0],
}

/// One privacy-safe status transition in the fixed native diagnostic ring.
#[cfg(feature = "sck-suspension-diagnostics")]
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SckDiagnosticsStatusEvent {
    pub(crate) struct_size: u32,
    pub(crate) kind: u32,
    pub(crate) raw_value: i64,
    pub(crate) sequence: u64,
    pub(crate) monotonic_nanos: u64,
}

/// One cold, bounded snapshot of private ScreenCaptureKit producer state.
#[cfg(feature = "sck-suspension-diagnostics")]
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SckDiagnosticsSnapshot {
    pub(crate) struct_size: u32,
    pub(crate) close_phase: u32,
    pub(crate) active_native_slots: u32,
    reserved0: u32,
    pub(crate) transition_count: u32,
    pub(crate) transition_capacity: u32,
    pub(crate) transition_start: u32,
    reserved1: u32,
    pub(crate) observed_total: u64,
    pub(crate) status_counts: [u64; SCK_STATUS_KIND_COUNT],
    pub(crate) first_status: SckDiagnosticsStatusEvent,
    pub(crate) last_status: SckDiagnosticsStatusEvent,
    pub(crate) callbacks_received: u64,
    pub(crate) callbacks_admitted: u64,
    pub(crate) callbacks_refused: u64,
    pub(crate) callbacks_exited: u64,
    pub(crate) stream_start_completed_nanos: u64,
    pub(crate) stream_stop_requested_nanos: u64,
    pub(crate) stream_stop_completed_nanos: u64,
    pub(crate) callback_admission_stopped_nanos: u64,
    pub(crate) callback_fence_completed_nanos: u64,
    pub(crate) close_completed_nanos: u64,
    pub(crate) first_complete_nanos: u64,
    pub(crate) session_references: u64,
    pub(crate) detached_leases: u64,
    pub(crate) native_objects: u64,
    pub(crate) detached_bytes: u64,
    pub(crate) transition_overwrites: u64,
    pub(crate) transitions: [SckDiagnosticsStatusEvent; SCK_DIAGNOSTIC_TRANSITION_CAPACITY],
}

#[cfg(feature = "sck-suspension-diagnostics")]
impl SckDiagnosticsSnapshot {
    pub(crate) fn requested() -> Self {
        Self {
            struct_size: u32::try_from(size_of::<Self>()).expect("structure size fits u32"),
            transition_capacity: u32::try_from(SCK_DIAGNOSTIC_TRANSITION_CAPACITY)
                .expect("transition capacity fits u32"),
            ..Self::default()
        }
    }
}

/// One identifier-free active display mode.
#[cfg(feature = "sck-suspension-diagnostics")]
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SckDiagnosticsDisplayMode {
    pub(crate) logical_width: u32,
    pub(crate) logical_height: u32,
    pub(crate) refresh_millihertz: u32,
    pub(crate) built_in: u32,
    pub(crate) mirrored: u32,
    reserved: [u32; 3],
}

/// One cold, bounded active-display topology snapshot.
#[cfg(feature = "sck-suspension-diagnostics")]
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SckDiagnosticsDisplayTopology {
    pub(crate) struct_size: u32,
    pub(crate) display_count: u32,
    pub(crate) mode_count: u32,
    pub(crate) mode_capacity: u32,
    pub(crate) modes: [SckDiagnosticsDisplayMode; SCK_DIAGNOSTIC_DISPLAY_CAPACITY],
}

#[cfg(feature = "sck-suspension-diagnostics")]
impl SckDiagnosticsDisplayTopology {
    pub(crate) fn requested() -> Self {
        Self {
            struct_size: u32::try_from(size_of::<Self>()).expect("structure size fits u32"),
            mode_capacity: u32::try_from(SCK_DIAGNOSTIC_DISPLAY_CAPACITY)
                .expect("display capacity fits u32"),
            ..Self::default()
        }
    }
}

#[cfg(all(test, feature = "sck-suspension-diagnostics"))]
struct SckDiagnosticsProbeInner {
    handle: NonNull<OpaqueSckDiagnosticsProbe>,
}

// SAFETY: the native probe stores counters atomically and protects the transition
// ring with its mutex. `SckDiagnosticsProbe` is not cloneable and is the only type
// that can record, while cloneable readers can only take locked snapshots.
#[cfg(all(test, feature = "sck-suspension-diagnostics"))]
unsafe impl Send for SckDiagnosticsProbeInner {}
// SAFETY: same invariant as the `Send` implementation above.
#[cfg(all(test, feature = "sck-suspension-diagnostics"))]
unsafe impl Sync for SckDiagnosticsProbeInner {}

#[cfg(all(test, feature = "sck-suspension-diagnostics"))]
impl Drop for SckDiagnosticsProbeInner {
    fn drop(&mut self) {
        // SAFETY: this is the final `Arc` and therefore the last use of the
        // uniquely owned native probe handle.
        unsafe { mp_shim_sck_diagnostics_test_probe_release(self.handle.as_ptr()) };
    }
}

#[cfg(all(test, feature = "sck-suspension-diagnostics"))]
struct SckDiagnosticsProbe {
    inner: Arc<SckDiagnosticsProbeInner>,
}

#[cfg(all(test, feature = "sck-suspension-diagnostics"))]
impl SckDiagnosticsProbe {
    fn open() -> Self {
        let mut probe = std::ptr::null_mut();
        // SAFETY: `probe` is a writable output for one opaque handle.
        let status = unsafe { mp_shim_sck_diagnostics_test_probe_open(&raw mut probe) };
        assert_eq!(ShimStatus::from_raw(status), ShimStatus::Ok);
        Self {
            inner: Arc::new(SckDiagnosticsProbeInner {
                handle: NonNull::new(probe).expect("successful probe open returns a handle"),
            }),
        }
    }

    fn reader(&self) -> SckDiagnosticsProbeReader {
        SckDiagnosticsProbeReader {
            inner: Arc::clone(&self.inner),
        }
    }

    fn record(&mut self, raw: Option<i64>) -> bool {
        let mut would_publish = 0;
        // SAFETY: this non-cloneable writer is the probe's only record capability;
        // the output is writable for one u32.
        let status = unsafe {
            mp_shim_sck_diagnostics_test_probe_record(
                self.inner.handle.as_ptr(),
                u32::from(raw.is_some()),
                raw.unwrap_or_default(),
                &raw mut would_publish,
            )
        };
        assert_eq!(ShimStatus::from_raw(status), ShimStatus::Ok);
        would_publish != 0
    }

    fn seed_saturation(&mut self) {
        // SAFETY: this non-cloneable writer is the probe's only mutation
        // capability other than `record`.
        let status = unsafe {
            mp_shim_sck_diagnostics_test_probe_seed_saturation(self.inner.handle.as_ptr())
        };
        assert_eq!(ShimStatus::from_raw(status), ShimStatus::Ok);
    }

    fn snapshot(&self) -> SckDiagnosticsSnapshot {
        self.reader().snapshot()
    }

    fn snapshot_with(
        &self,
        mut snapshot: SckDiagnosticsSnapshot,
        raise_native_exception: bool,
    ) -> (ShimStatus, SckDiagnosticsSnapshot) {
        // SAFETY: the handle remains live through `self`, and `snapshot` is a
        // writable size-versioned output.
        let status = unsafe {
            mp_shim_sck_diagnostics_test_probe_snapshot(
                self.inner.handle.as_ptr(),
                u32::from(raise_native_exception),
                &raw mut snapshot,
            )
        };
        (ShimStatus::from_raw(status), snapshot)
    }
}

#[cfg(all(test, feature = "sck-suspension-diagnostics"))]
#[derive(Clone)]
struct SckDiagnosticsProbeReader {
    inner: Arc<SckDiagnosticsProbeInner>,
}

#[cfg(all(test, feature = "sck-suspension-diagnostics"))]
impl SckDiagnosticsProbeReader {
    fn snapshot(&self) -> SckDiagnosticsSnapshot {
        let requested = SckDiagnosticsSnapshot::requested();
        let (status, snapshot) = {
            let probe = SckDiagnosticsProbe {
                inner: Arc::clone(&self.inner),
            };
            probe.snapshot_with(requested, false)
        };
        assert_eq!(status, ShimStatus::Ok);
        snapshot
    }
}

#[cfg(all(test, feature = "sck-suspension-diagnostics"))]
fn testing_sck_callback_accounting(raise_native_exception: bool) -> (u32, SckDiagnosticsSnapshot) {
    let mut callback_body_calls = 0;
    let mut snapshot = SckDiagnosticsSnapshot::requested();
    // SAFETY: both outputs are writable and the native seam owns its complete
    // stack-local admission and diagnostic lifetime.
    let status = unsafe {
        mp_shim_sck_diagnostics_test_callback_accounting(
            u32::from(raise_native_exception),
            &raw mut callback_body_calls,
            &raw mut snapshot,
        )
    };
    assert_eq!(ShimStatus::from_raw(status), ShimStatus::Ok);
    (callback_body_calls, snapshot)
}

#[cfg(all(test, feature = "sck-suspension-diagnostics"))]
fn testing_sck_display_topology(
    modes: &[SckDiagnosticsDisplayMode],
    display_count: u32,
    raise_native_exception: bool,
    mut topology: SckDiagnosticsDisplayTopology,
) -> (ShimStatus, SckDiagnosticsDisplayTopology) {
    // SAFETY: the slice is readable for `modes.len()` fixed records and
    // `topology` is a writable size-versioned output.
    let status = unsafe {
        mp_shim_sck_diagnostics_test_display_topology(
            modes.as_ptr(),
            u32::try_from(modes.len()).expect("test mode count fits u32"),
            display_count,
            u32::from(raise_native_exception),
            &raw mut topology,
        )
    };
    (ShimStatus::from_raw(status), topology)
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
    flags: u32,
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
            flags: 0,
        }
    }

    /// Returns the content extent, when the shim reported a usable one.
    pub(crate) fn extent(&self) -> Option<PixelExtent> {
        (self.pixel_width > 0 && self.pixel_height > 0)
            .then(|| PixelExtent::new(self.pixel_width, self.pixel_height))
    }
    /// Reports snapshot-time process-directed admission.
    pub(crate) const fn process_directed(&self) -> bool {
        self.kind == KIND_WINDOW && self.flags & TARGET_INFO_PROCESS_DIRECTED != 0
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
    /// Effective capture pixels per point after content scaling.
    pub(crate) scale_factor: f64,
    /// Raw display backing pixels per point for native geometry comparison.
    backing_scale: f64,
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
            backing_scale: 1.0,
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
            backing_scale: scale_factor,
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
            backing_scale: scale_factor,
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

    /// Overrides the raw backing scale for a downscaled-frame boundary test.
    #[cfg(test)]
    pub(crate) const fn with_backing_scale(mut self, backing_scale: f64) -> Self {
        self.backing_scale = backing_scale;
        self
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

    /// Returns the raw display backing scale after repeating native validation.
    pub(crate) fn backing_scale(&self) -> Option<f64> {
        (self.backing_scale.is_finite() && self.backing_scale > 0.0).then_some(self.backing_scale)
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

/// One isolated Core Graphics event source owned by a process-directed sequence.
///
/// The handle is intentionally neither cloneable nor shared. A controller creates
/// it after selecting the route, reuses it for every ordinary post and bounded
/// cleanup release, and drops it when that one admitted sequence ends.
pub(crate) struct ProcessEventSource {
    handle: NonNull<OpaqueProcessEventSource>,
}

impl ProcessEventSource {
    /// Creates a private-state source without posting or prompting.
    ///
    /// A nonzero activity tag is copied to the source's documented event
    /// user-data field. It remains observational metadata and never affects
    /// admission, posting, or receipt accounting.
    pub(crate) fn new(activity_tag: u64) -> Result<Self, ShimStatus> {
        let mut source = std::ptr::null_mut();
        // SAFETY: `source` is writable for one opaque handle and the native
        // boundary either leaves it null or transfers exactly one owned handle.
        let status = unsafe { mp_shim_process_event_source_create(activity_tag, &raw mut source) };
        ShimStatus::from_raw(status).into_result()?;
        NonNull::new(source)
            .map(|handle| Self { handle })
            .ok_or(ShimStatus::PlatformFailure)
    }

    fn as_ptr(&self) -> *const OpaqueProcessEventSource {
        self.handle.as_ptr()
    }
}

impl fmt::Debug for ProcessEventSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessEventSource")
            .finish_non_exhaustive()
    }
}

impl Drop for ProcessEventSource {
    fn drop(&mut self) {
        // SAFETY: construction accepted exactly one owned native handle and this
        // non-cloneable wrapper releases it exactly once.
        unsafe { mp_shim_process_event_source_release(self.handle.as_ptr()) };
    }
}

/// An exact capture selection constructed from one discovery snapshot.
///
/// The native handle retains the original `SCContentFilter`. Window input
/// compares its logical `SCWindow` with a fresh shareable-content snapshot;
/// numeric ids and process ids only narrow and validate that comparison.
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

// SAFETY: the native handle owns an immutable retained `SCContentFilter`.
// ScreenCaptureKit shareable-content queries and the handle's read-only
// operations may run from arbitrary caller threads.
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

    /// Reads bounds from a fresh observation of this retained selection.
    pub(crate) fn input_bounds(&self, wait: Duration) -> Result<NativeBounds, ShimStatus> {
        input_target_bounds(self, wait)
    }

    /// Reads whether this exact retained window is focused within a bounded wait.
    pub(crate) fn input_focused(&self, wait: Duration) -> Result<bool, ShimStatus> {
        input_target_focused(self, wait)
    }

    /// Activates only this retained target's still-matching process lifetime.
    pub(crate) fn input_activate_owner(&self) -> Result<(), ShimStatus> {
        input_activate_owner(self)
    }

    /// Revalidates retained-window and owning-process identity, current geometry,
    /// and post-event authorization without prompting or activating the target.
    pub(crate) fn process_authority(
        &self,
        wait: Duration,
    ) -> Result<ProcessAuthority, ProcessAuthorityFailure> {
        process_target_authority(self, wait)
    }

    /// Posts one bounded event to the retained target's owning process.
    ///
    /// `purpose` selects whether current retained-window admission is required.
    /// The returned unit count is invocation-only evidence. A nonzero count on
    /// failure means this logical event may already have native effect.
    pub(crate) fn process_post(
        &self,
        source: &ProcessEventSource,
        request: ProcessPostRequest<'_>,
        operation: &OperationContext,
    ) -> Result<ProcessPostOutcome, ProcessPostFailure> {
        process_post(self, source, request, operation)
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

    /// Copies the private, bounded ScreenCaptureKit producer diagnostics.
    #[cfg(feature = "sck-suspension-diagnostics")]
    pub(crate) fn sck_diagnostics_snapshot(&self) -> Result<SckDiagnosticsSnapshot, ShimStatus> {
        let mut snapshot = SckDiagnosticsSnapshot::requested();
        // SAFETY: the handle is owned here and `snapshot` is a writable,
        // size-versioned output with the exact fixed transition capacity.
        let status = unsafe {
            mp_shim_sck_diagnostics_snapshot_read(self.handle.as_ptr(), &raw mut snapshot)
        };
        ShimStatus::from_raw(status).into_result()?;
        Ok(snapshot)
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

/// The migration probe's two public non-prompting input observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProcessAuthorization {
    /// The authorization truth used immediately before event posting.
    pub(crate) post_event_access: PermissionState,
    /// The legacy Accessibility observation retained for qualification evidence.
    pub(crate) accessibility: PermissionState,
}

impl ProcessAuthorization {
    const fn from_raw(post_event_access: u32, accessibility: u32) -> Self {
        Self {
            post_event_access: permission_state(post_event_access),
            accessibility: permission_state(accessibility),
        }
    }

    /// Reports whether two available boolean observations disagree.
    #[cfg(test)]
    pub(crate) const fn disagrees(self) -> bool {
        matches!(
            (self.post_event_access, self.accessibility),
            (PermissionState::Granted, PermissionState::NotGranted)
                | (PermissionState::NotGranted, PermissionState::Granted)
        )
    }
}

/// Reads post-event access and the legacy Accessibility observation together.
pub(crate) fn process_authorization() -> Result<ProcessAuthorization, ShimStatus> {
    let mut post_event_access = u32::MAX;
    let mut accessibility = u32::MAX;
    // SAFETY: both outputs are writable for one u32, and the native read is
    // non-prompting.
    let status = unsafe {
        mp_shim_process_authorization(&raw mut post_event_access, &raw mut accessibility)
    };
    ShimStatus::from_raw(status).into_result()?;
    Ok(ProcessAuthorization::from_raw(
        post_event_access,
        accessibility,
    ))
}

fn probe(read: impl FnOnce(*mut u32) -> u32) -> Result<PermissionState, ShimStatus> {
    let mut state = u32::MAX;
    let status = read(&raw mut state);
    ShimStatus::from_raw(status).into_result()?;
    Ok(permission_state(state))
}

const fn permission_state(state: u32) -> PermissionState {
    match state {
        0 => PermissionState::Granted,
        1 => PermissionState::NotGranted,
        2 => PermissionState::Unavailable,
        // A state this build does not know about is not read as authorization.
        _ => PermissionState::Unknown,
    }
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

#[cfg(feature = "sck-suspension-diagnostics")]
pub(crate) fn sck_diagnostics_process_resources() -> Result<(u64, u64, u64, u64), ShimStatus> {
    let mut native_objects = 0;
    let mut detached_bytes = 0;
    let mut live_sessions = 0;
    let mut callbacks_in_flight = 0;
    // SAFETY: all arguments are writable outputs for one u64.
    let status = unsafe {
        mp_shim_sck_diagnostics_process_resources(
            &raw mut native_objects,
            &raw mut detached_bytes,
            &raw mut live_sessions,
            &raw mut callbacks_in_flight,
        )
    };
    ShimStatus::from_raw(status).into_result().map(|()| {
        (
            native_objects,
            detached_bytes,
            live_sessions,
            callbacks_in_flight,
        )
    })
}

#[cfg(feature = "sck-suspension-diagnostics")]
pub(crate) fn sck_diagnostics_display_topology() -> Result<SckDiagnosticsDisplayTopology, ShimStatus>
{
    let mut topology = SckDiagnosticsDisplayTopology::requested();
    // SAFETY: `topology` is a writable size-versioned output with the declared
    // fixed mode capacity.
    let status = unsafe { mp_shim_sck_diagnostics_read_display_topology(&raw mut topology) };
    ShimStatus::from_raw(status)
        .into_result()
        .map(|()| topology)
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

/// What one process-directed native request constructs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ProcessPost<'units> {
    Pointer {
        action: u32,
        button: u32,
        click_state: u64,
        location: (f64, f64),
    },
    Scroll {
        horizontal: i32,
        vertical: i32,
        location: (f64, f64),
    },
    Key {
        key_code: u16,
        down: bool,
    },
    Text(&'units [u16]),
}

/// Why one process-directed native post is being attempted.
///
/// Ordinary input requires current retained-window admission. A bounded release
/// preserves only sequence-owned state, so it revalidates the original process
/// lifetime and authorization without requiring the window to remain visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcessPostPurpose {
    Input,
    Release,
}

impl ProcessPostPurpose {
    const fn as_raw(self) -> u32 {
        match self {
            Self::Input => 0,
            Self::Release => 1,
        }
    }

    pub(crate) const fn expected_target_match_count(self) -> u32 {
        match self {
            Self::Input => 1,
            Self::Release => 0,
        }
    }
}

/// Whether the caller's focus predicate must hold at the final native gate.
///
/// The route imposes no focus requirement of its own. Only a caller that
/// selected `RequireFocused` sets this, and a sequence-owned release never does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcessFocusRequirement {
    None,
    RequireFocused,
}

impl ProcessFocusRequirement {
    const fn as_raw(self) -> u8 {
        match self {
            Self::None => 0,
            Self::RequireFocused => 1,
        }
    }
}

/// Focus-predicate result from the last native per-unit gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcessFocusObservation {
    NotApplicable,
    NotEvaluated,
    Passed,
    Refused,
    Unavailable,
}

impl ProcessFocusObservation {
    const fn from_raw(raw: u32) -> Self {
        match raw {
            0 => Self::NotApplicable,
            2 => Self::Passed,
            3 => Self::Refused,
            4 => Self::Unavailable,
            _ => Self::NotEvaluated,
        }
    }
}

/// Geometry repeated at the final native authority boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ProcessGeometry {
    AuthorityOnly,
    RequireCurrent(NativeBounds),
}

/// One bounded process-directed post and its final authority policy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ProcessPostRequest<'units> {
    pub(crate) post: ProcessPost<'units>,
    pub(crate) geometry: ProcessGeometry,
    pub(crate) purpose: ProcessPostPurpose,
    pub(crate) focus: ProcessFocusRequirement,
    pub(crate) flags: u32,
    pub(crate) wait: Duration,
}

/// One successful fresh process/window authority observation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ProcessAuthority {
    pub(crate) bounds: NativeBounds,
    pub(crate) target_match_count: u32,
}

/// Why process/window authority could not be established.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProcessAuthorityFailure {
    pub(crate) status: ShimStatus,
    pub(crate) target_match_count: u32,
}

/// Privacy-safe authorization result from the last native per-unit gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcessAuthorizationObservation {
    Unknown,
    Granted,
    NotGranted,
    Unavailable,
}

impl ProcessAuthorizationObservation {
    const fn from_raw(raw: u32) -> Self {
        match raw {
            1 => Self::Granted,
            2 => Self::NotGranted,
            3 => Self::Unavailable,
            _ => Self::Unknown,
        }
    }
}

/// Geometry-policy result from the last native per-unit gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcessGeometryObservation {
    NotApplicable,
    NotEvaluated,
    Passed,
    Changed,
}

impl ProcessGeometryObservation {
    const fn from_raw(raw: u32) -> Self {
        match raw {
            0 => Self::NotApplicable,
            2 => Self::Passed,
            3 => Self::Changed,
            _ => Self::NotEvaluated,
        }
    }
}

/// Invocation-only facts from one process-directed native request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProcessPostOutcome {
    pub(crate) invoked_native_units: u64,
    pub(crate) target_match_count: u32,
    pub(crate) authorization: ProcessAuthorizationObservation,
    pub(crate) geometry: ProcessGeometryObservation,
    pub(crate) focus: ProcessFocusObservation,
}

/// A process-directed request failure plus its exact returned-call prefix and
/// conservative irreversible-threshold state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProcessPostFailure {
    pub(crate) status: ShimStatus,
    pub(crate) invoked_native_units: u64,
    pub(crate) native_effect_may_have_occurred: bool,
    pub(crate) target_match_count: u32,
    pub(crate) authorization: ProcessAuthorizationObservation,
    pub(crate) geometry: ProcessGeometryObservation,
    pub(crate) focus: ProcessFocusObservation,
}

/// One target's live rectangle in the global point space, with its backing scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct NativeBounds {
    pub(crate) origin: (f64, f64),
    pub(crate) size: (f64, f64),
    pub(crate) scale: f64,
}

impl NativeBounds {
    /// Compares the geometry identity the native final commit gate observes.
    ///
    /// ScreenCaptureKit can report a fractional point size whose exact transform
    /// covers the rounded backing-pixel extent. The raw point sizes therefore need
    /// not be bit-identical for the two observations to describe the same capture
    /// geometry.
    pub(crate) fn capture_equivalent_to(self, other: Self) -> bool {
        self.origin == other.origin
            && self.scale == other.scale
            && self.pixel_extent().is_some_and(|extent| {
                other
                    .pixel_extent()
                    .is_some_and(|other_extent| extent == other_extent)
            })
    }

    fn pixel_extent(self) -> Option<PixelExtent> {
        Some(PixelExtent::new(
            native_pixels_from_points(self.size.0, self.scale)?,
            native_pixels_from_points(self.size.1, self.scale)?,
        ))
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn native_pixels_from_points(points: f64, scale: f64) -> Option<u32> {
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let pixels = (points * scale).round();
    if !pixels.is_finite() || pixels < 1.0 || pixels > f64::from(MAX_SURFACE_EXTENT) {
        return None;
    }
    Some(pixels as u32)
}

/// Reports whether the exact retained window is focused.
fn input_target_focused(target: &TargetToken, wait: Duration) -> Result<bool, ShimStatus> {
    let mut focused = false;
    // SAFETY: the target pointer is either its retained native handle or null for
    // a test-only synthetic token, the timeout is bounded, and the output is
    // writable for one C boolean.
    let status =
        unsafe { mp_shim_input_target_focused(target.as_ptr(), nanos(wait), &raw mut focused) };
    ShimStatus::from_raw(status).into_result()?;
    Ok(focused)
}

/// Reports one target's current rectangle, and thereby whether it still exists.
fn input_target_bounds(target: &TargetToken, wait: Duration) -> Result<NativeBounds, ShimStatus> {
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
    // SAFETY: the target is retained, the wait is bounded, and all five outputs
    // are writable for one f64 each.
    let status = unsafe {
        mp_shim_input_target_bounds(
            handle.as_ptr(),
            nanos(wait),
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

fn process_target_authority(
    target: &TargetToken,
    wait: Duration,
) -> Result<ProcessAuthority, ProcessAuthorityFailure> {
    let Some(handle) = target.inner.handle else {
        #[cfg(test)]
        {
            return input_target_bounds(target, wait)
                .map(|bounds| ProcessAuthority {
                    bounds,
                    target_match_count: 1,
                })
                .map_err(|status| ProcessAuthorityFailure {
                    status,
                    target_match_count: 0,
                });
        }
        #[cfg(not(test))]
        return Err(ProcessAuthorityFailure {
            status: ShimStatus::InvalidArgument,
            target_match_count: 0,
        });
    };
    let mut native = NativeProcessAuthority {
        struct_size: u32::try_from(size_of::<NativeProcessAuthority>())
            .expect("structure size fits u32"),
        target_match_count: 0,
        logical_x: 0.0,
        logical_y: 0.0,
        logical_width: 0.0,
        logical_height: 0.0,
        backing_scale: 0.0,
    };
    // SAFETY: the retained opaque target stays alive for the call and `native`
    // is writable for the exact size-versioned structure declared to C.
    let status = ShimStatus::from_raw(unsafe {
        mp_shim_process_authority(handle.as_ptr(), nanos(wait), &raw mut native)
    });
    if status != ShimStatus::Ok {
        return Err(ProcessAuthorityFailure {
            status,
            target_match_count: native.target_match_count,
        });
    }
    Ok(ProcessAuthority {
        bounds: NativeBounds {
            origin: (native.logical_x, native.logical_y),
            size: (native.logical_width, native.logical_height),
            scale: native.backing_scale,
        },
        target_match_count: native.target_match_count,
    })
}

fn process_post(
    target: &TargetToken,
    source: &ProcessEventSource,
    request: ProcessPostRequest<'_>,
    operation: &OperationContext,
) -> Result<ProcessPostOutcome, ProcessPostFailure> {
    #[cfg(test)]
    let test_wait = request.wait;
    let ProcessPostRequest {
        post,
        geometry,
        purpose,
        focus,
        flags,
        wait: _,
    } = request;
    let Some(handle) = target.inner.handle else {
        #[cfg(test)]
        {
            if let Err(status) = input_target_bounds(target, test_wait) {
                return Err(ProcessPostFailure {
                    status,
                    invoked_native_units: 0,
                    native_effect_may_have_occurred: false,
                    target_match_count: 0,
                    authorization: ProcessAuthorizationObservation::Unknown,
                    geometry: ProcessGeometryObservation::NotEvaluated,
                    focus: ProcessFocusObservation::NotEvaluated,
                });
            }
            return Ok(ProcessPostOutcome {
                invoked_native_units: u64::from(matches!(post, ProcessPost::Text(_))) + 1,
                target_match_count: purpose.expected_target_match_count(),
                authorization: ProcessAuthorizationObservation::Granted,
                geometry: match geometry {
                    ProcessGeometry::AuthorityOnly => ProcessGeometryObservation::NotApplicable,
                    ProcessGeometry::RequireCurrent(_) => ProcessGeometryObservation::Passed,
                },
                focus: match focus {
                    ProcessFocusRequirement::None => ProcessFocusObservation::NotApplicable,
                    ProcessFocusRequirement::RequireFocused => ProcessFocusObservation::Passed,
                },
            });
        }
        #[cfg(not(test))]
        return Err(ProcessPostFailure {
            status: ShimStatus::InvalidArgument,
            invoked_native_units: 0,
            native_effect_may_have_occurred: false,
            target_match_count: 0,
            authorization: ProcessAuthorizationObservation::Unknown,
            geometry: ProcessGeometryObservation::NotEvaluated,
            focus: ProcessFocusObservation::NotEvaluated,
        });
    };

    let (
        event_kind,
        action,
        button,
        click_state,
        x,
        y,
        horizontal,
        vertical,
        key_code,
        key_down,
        text_units,
        text_unit_count,
    ) = match post {
        ProcessPost::Pointer {
            action,
            button,
            click_state,
            location,
        } => (
            0,
            action,
            button,
            click_state,
            location.0,
            location.1,
            0,
            0,
            0,
            false,
            std::ptr::null(),
            0,
        ),
        ProcessPost::Scroll {
            horizontal,
            vertical,
            location,
        } => (
            1,
            0,
            0,
            0,
            location.0,
            location.1,
            horizontal,
            vertical,
            0,
            false,
            std::ptr::null(),
            0,
        ),
        ProcessPost::Key { key_code, down } => (
            2,
            0,
            0,
            0,
            0.0,
            0.0,
            0,
            0,
            key_code,
            down,
            std::ptr::null(),
            0,
        ),
        ProcessPost::Text(units) => (
            3,
            0,
            0,
            0,
            0.0,
            0.0,
            0,
            0,
            0,
            false,
            units.as_ptr(),
            units.len(),
        ),
    };
    let (geometry_check, expected) = match geometry {
        ProcessGeometry::AuthorityOnly => (
            0,
            NativeBounds {
                origin: (0.0, 0.0),
                size: (0.0, 0.0),
                scale: 0.0,
            },
        ),
        ProcessGeometry::RequireCurrent(expected) => (1, expected),
    };
    let checkpoint = ProcessOperationCheckpoint::new(operation);
    let checkpoint_context = std::ptr::from_ref(&checkpoint).cast_mut().cast::<c_void>();
    let cancellation = ProcessCancellationFence::new(operation);
    let cancellation_context = std::ptr::from_ref(&cancellation)
        .cast_mut()
        .cast::<c_void>();
    let request = NativeProcessPostRequest {
        struct_size: u32::try_from(size_of::<NativeProcessPostRequest>())
            .expect("structure size fits u32"),
        event_kind,
        target: handle.as_ptr(),
        event_source: source.as_ptr(),
        timeout_nanos: nanos(MAX_NATIVE_WAIT),
        flags,
        geometry_check,
        purpose: purpose.as_raw(),
        action,
        button,
        click_state,
        x,
        y,
        horizontal,
        vertical,
        key_code,
        key_down,
        focus_requirement: focus.as_raw(),
        reserved: [0; 4],
        text_units,
        text_unit_count,
        expected_x: expected.origin.0,
        expected_y: expected.origin.1,
        expected_width: expected.size.0,
        expected_height: expected.size.1,
        expected_scale: expected.scale,
        interruption_context: checkpoint_context,
        interruption_callback: Some(process_operation_checkpoint_callback),
        cancellation_context,
        cancellation_callback: Some(process_cancellation_callback),
    };
    let mut report = NativeProcessPostReport {
        struct_size: u32::try_from(size_of::<NativeProcessPostReport>())
            .expect("structure size fits u32"),
        target_match_count: 0,
        invoked_native_units: 0,
        native_effect_may_have_occurred: 0,
        authorization: 0,
        geometry_result: 1,
        focus_result: 1,
    };
    // SAFETY: the target and optional text slice outlive the call; request and
    // report match the C layout and the report is exclusively writable.
    let status =
        ShimStatus::from_raw(unsafe { mp_shim_process_post(&raw const request, &raw mut report) });
    if status == ShimStatus::Ok {
        Ok(ProcessPostOutcome {
            invoked_native_units: report.invoked_native_units,
            target_match_count: report.target_match_count,
            authorization: ProcessAuthorizationObservation::from_raw(report.authorization),
            geometry: ProcessGeometryObservation::from_raw(report.geometry_result),
            focus: ProcessFocusObservation::from_raw(report.focus_result),
        })
    } else {
        Err(ProcessPostFailure {
            status,
            invoked_native_units: report.invoked_native_units,
            native_effect_may_have_occurred: report.native_effect_may_have_occurred != 0,
            target_match_count: report.target_match_count,
            authorization: ProcessAuthorizationObservation::from_raw(report.authorization),
            geometry: ProcessGeometryObservation::from_raw(report.geometry_result),
            focus: ProcessFocusObservation::from_raw(report.focus_result),
        })
    }
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

/// Activates the application retained by `target`, presenting no interface.
fn input_activate_owner(target: &TargetToken) -> Result<(), ShimStatus> {
    // SAFETY: the immutable retained target handle stays alive for the call.
    ShimStatus::from_raw(unsafe { mp_shim_input_activate_owner(target.as_ptr()) }).into_result()
}

/// Resolves one Unicode scalar to a key code reachable without modifiers.
pub(crate) fn input_resolve_character(scalar: u32) -> Result<u16, ShimStatus> {
    let mut key_code = 0u16;
    // SAFETY: the output is writable for one u16.
    let status = unsafe { mp_shim_input_resolve_character(scalar, &raw mut key_code) };
    ShimStatus::from_raw(status).into_result()?;
    Ok(key_code)
}

/// One prepared system-post attempt that did not complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PreparedPostFailure {
    pub(crate) status: ShimStatus,
    pub(crate) native_effect_may_have_occurred: bool,
    /// The atomic callback itself observed cancellation and stopped before effect.
    pub(crate) cancellation_prevented_effect: bool,
}

/// A fully configured system-delivery event batch.
///
/// Native storage is prepared before final mutable target authority is read.
/// Drop releases every Core Graphics object whether or not any unit was posted.
pub(crate) struct PreparedInput {
    raw: NonNull<OpaquePreparedInput>,
    count: usize,
}

impl PreparedInput {
    fn from_native(status: u32, raw: *mut OpaquePreparedInput) -> Result<Self, ShimStatus> {
        ShimStatus::from_raw(status).into_result()?;
        let raw = NonNull::new(raw).ok_or(ShimStatus::PlatformFailure)?;
        let mut prepared = Self { raw, count: 0 };
        let mut count = 0usize;
        // SAFETY: the owned handle remains live and `count` is writable.
        let status = unsafe { mp_shim_input_prepared_count(prepared.raw.as_ptr(), &raw mut count) };
        ShimStatus::from_raw(status).into_result()?;
        if !(1..=2).contains(&count) {
            return Err(ShimStatus::PlatformFailure);
        }
        prepared.count = count;
        Ok(prepared)
    }

    pub(crate) const fn count(&self) -> usize {
        self.count
    }

    pub(crate) fn post_unit(
        &mut self,
        index: usize,
        deadline_nanos: u64,
        cancellation: Option<&mado_pilot_core::CancellationToken>,
    ) -> Result<(), PreparedPostFailure> {
        let fence = ProcessCancellationFence::from_cancellation(cancellation);
        let context = std::ptr::from_ref(&fence).cast_mut().cast::<c_void>();
        let mut native_effect_may_have_occurred = 0u32;
        // SAFETY: the prepared handle and stack-owned atomic fence remain live
        // for this synchronous call; the scalar output is writable.
        let status = unsafe {
            mp_shim_input_post_prepared(
                self.raw.as_ptr(),
                index,
                deadline_nanos,
                context,
                Some(process_cancellation_callback),
                &raw mut native_effect_may_have_occurred,
            )
        };
        let status = ShimStatus::from_raw(status);
        if status == ShimStatus::Ok {
            return Ok(());
        }
        let native_effect_may_have_occurred = native_effect_may_have_occurred == 1;
        Err(PreparedPostFailure {
            status,
            native_effect_may_have_occurred,
            cancellation_prevented_effect: status == ShimStatus::TimedOut
                && !native_effect_may_have_occurred
                && fence.cancellation_observed(),
        })
    }
}

impl Drop for PreparedInput {
    fn drop(&mut self) {
        // SAFETY: this wrapper uniquely owns the handle and releases it once.
        let _ = unsafe { mp_shim_input_prepared_release(self.raw.as_ptr()) };
    }
}

pub(crate) fn input_prepare_pointer(
    action: u32,
    button: u32,
    click_state: u64,
    location: (f64, f64),
    flags: u32,
) -> Result<PreparedInput, ShimStatus> {
    let mut raw = std::ptr::null_mut();
    // SAFETY: every input is scalar and `raw` is writable for one handle.
    let status = unsafe {
        mp_shim_input_prepare_pointer(
            action,
            button,
            click_state,
            location.0,
            location.1,
            flags,
            &raw mut raw,
        )
    };
    PreparedInput::from_native(status, raw)
}

pub(crate) fn input_prepare_scroll(
    horizontal: i32,
    vertical: i32,
    location: (f64, f64),
    flags: u32,
) -> Result<PreparedInput, ShimStatus> {
    let mut raw = std::ptr::null_mut();
    // SAFETY: every input is scalar and `raw` is writable for one handle.
    let status = unsafe {
        mp_shim_input_prepare_scroll(
            horizontal,
            vertical,
            location.0,
            location.1,
            flags,
            &raw mut raw,
        )
    };
    PreparedInput::from_native(status, raw)
}

pub(crate) fn input_prepare_key(
    key_code: u16,
    down: bool,
    flags: u32,
) -> Result<PreparedInput, ShimStatus> {
    let mut raw = std::ptr::null_mut();
    // SAFETY: every input is scalar and `raw` is writable for one handle.
    let status = unsafe { mp_shim_input_prepare_key(key_code, down, flags, &raw mut raw) };
    PreparedInput::from_native(status, raw)
}

pub(crate) fn input_prepare_text(units: &[u16], flags: u32) -> Result<PreparedInput, ShimStatus> {
    if units.is_empty() || units.len() > INPUT_MAX_TEXT_CHUNK {
        return Err(ShimStatus::InvalidArgument);
    }
    let mut raw = std::ptr::null_mut();
    // SAFETY: the initialized slice remains borrowed for the synchronous
    // preparation call and `raw` is writable for one handle.
    let status =
        unsafe { mp_shim_input_prepare_text(units.as_ptr(), units.len(), flags, &raw mut raw) };
    PreparedInput::from_native(status, raw)
}

/// Returns the version, structure sizes, and process-field offsets compiled into
/// the linked shim.
pub(crate) fn linked_layout() -> (u32, [u32; 6], [u32; 6]) {
    // SAFETY: the version call takes no arguments.
    let version = unsafe { mp_shim_abi_version() };
    let mut sizes = [0; 6];
    let [
        target_info,
        frame_info,
        open_request,
        process_authority,
        process_post_request,
        process_post_report,
    ] = &mut sizes;
    // SAFETY: all six outputs are writable for one u32 each.
    let size_status = unsafe {
        mp_shim_struct_sizes(
            &raw mut *target_info,
            &raw mut *frame_info,
            &raw mut *open_request,
            &raw mut *process_authority,
            &raw mut *process_post_request,
            &raw mut *process_post_report,
        )
    };
    if ShimStatus::from_raw(size_status) != ShimStatus::Ok {
        return (version, [0; 6], [0; 6]);
    }

    let mut offsets = [0; 6];
    let [
        authority_target_count,
        request_target,
        request_event_source,
        request_timeout,
        report_target_count,
        report_invoked_units,
    ] = &mut offsets;
    // SAFETY: all six outputs are writable for one u32 each.
    let offset_status = unsafe {
        mp_shim_process_struct_offsets(
            &raw mut *authority_target_count,
            &raw mut *request_target,
            &raw mut *request_event_source,
            &raw mut *request_timeout,
            &raw mut *report_target_count,
            &raw mut *report_invoked_units,
        )
    };
    if ShimStatus::from_raw(offset_status) != ShimStatus::Ok {
        return (version, sizes, [0; 6]);
    }
    (version, sizes, offsets)
}

/// The sizes this build compiled its mirrored structures to.
pub(crate) fn declared_layout() -> [u32; 6] {
    [
        u32::try_from(size_of::<TargetInfo>()).expect("structure size fits u32"),
        u32::try_from(size_of::<FrameInfo>()).expect("structure size fits u32"),
        u32::try_from(size_of::<NativeOpenRequest>()).expect("structure size fits u32"),
        u32::try_from(size_of::<NativeProcessAuthority>()).expect("structure size fits u32"),
        u32::try_from(size_of::<NativeProcessPostRequest>()).expect("structure size fits u32"),
        u32::try_from(size_of::<NativeProcessPostReport>()).expect("structure size fits u32"),
    ]
}

/// The process-field offsets this build compiled its mirrors to.
pub(crate) fn declared_process_offsets() -> [u32; 6] {
    [
        u32::try_from(std::mem::offset_of!(
            NativeProcessAuthority,
            target_match_count
        ))
        .expect("field offset fits u32"),
        u32::try_from(std::mem::offset_of!(NativeProcessPostRequest, target))
            .expect("field offset fits u32"),
        u32::try_from(std::mem::offset_of!(NativeProcessPostRequest, event_source))
            .expect("field offset fits u32"),
        u32::try_from(std::mem::offset_of!(
            NativeProcessPostRequest,
            timeout_nanos
        ))
        .expect("field offset fits u32"),
        u32::try_from(std::mem::offset_of!(
            NativeProcessPostReport,
            target_match_count
        ))
        .expect("field offset fits u32"),
        u32::try_from(std::mem::offset_of!(
            NativeProcessPostReport,
            invoked_native_units
        ))
        .expect("field offset fits u32"),
    ]
}

/// Returns private diagnostic sizes and offsets compiled into the linked shim.
#[cfg(feature = "sck-suspension-diagnostics")]
pub(crate) fn linked_sck_diagnostics_layout() -> [u32; 9] {
    let mut layout = [0; 9];
    let [
        event_size,
        snapshot_size,
        event_raw_value,
        event_sequence,
        event_monotonic_nanos,
        status_counts,
        first_status,
        callbacks_received,
        transitions,
    ] = &mut layout;
    // SAFETY: all nine outputs are writable for one u32 each.
    let status = unsafe {
        mp_shim_sck_diagnostics_layout(
            &raw mut *event_size,
            &raw mut *snapshot_size,
            &raw mut *event_raw_value,
            &raw mut *event_sequence,
            &raw mut *event_monotonic_nanos,
            &raw mut *status_counts,
            &raw mut *first_status,
            &raw mut *callbacks_received,
            &raw mut *transitions,
        )
    };
    if ShimStatus::from_raw(status) == ShimStatus::Ok {
        layout
    } else {
        [0; 9]
    }
}

/// Returns private display-topology sizes and offsets compiled into the linked shim.
#[cfg(feature = "sck-suspension-diagnostics")]
pub(crate) fn linked_sck_diagnostics_topology_layout() -> [u32; 4] {
    let mut layout = [0; 4];
    let [mode_size, topology_size, mode_refresh, topology_modes] = &mut layout;
    // SAFETY: all four outputs are writable for one u32 each.
    let status = unsafe {
        mp_shim_sck_diagnostics_topology_layout(
            &raw mut *mode_size,
            &raw mut *topology_size,
            &raw mut *mode_refresh,
            &raw mut *topology_modes,
        )
    };
    if ShimStatus::from_raw(status) == ShimStatus::Ok {
        layout
    } else {
        [0; 4]
    }
}

/// Returns private diagnostic sizes and offsets compiled into the Rust mirrors.
#[cfg(feature = "sck-suspension-diagnostics")]
pub(crate) fn declared_sck_diagnostics_layout() -> [u32; 9] {
    [
        u32::try_from(size_of::<SckDiagnosticsStatusEvent>()).expect("structure size fits u32"),
        u32::try_from(size_of::<SckDiagnosticsSnapshot>()).expect("structure size fits u32"),
        u32::try_from(std::mem::offset_of!(SckDiagnosticsStatusEvent, raw_value))
            .expect("field offset fits u32"),
        u32::try_from(std::mem::offset_of!(SckDiagnosticsStatusEvent, sequence))
            .expect("field offset fits u32"),
        u32::try_from(std::mem::offset_of!(
            SckDiagnosticsStatusEvent,
            monotonic_nanos
        ))
        .expect("field offset fits u32"),
        u32::try_from(std::mem::offset_of!(SckDiagnosticsSnapshot, status_counts))
            .expect("field offset fits u32"),
        u32::try_from(std::mem::offset_of!(SckDiagnosticsSnapshot, first_status))
            .expect("field offset fits u32"),
        u32::try_from(std::mem::offset_of!(
            SckDiagnosticsSnapshot,
            callbacks_received
        ))
        .expect("field offset fits u32"),
        u32::try_from(std::mem::offset_of!(SckDiagnosticsSnapshot, transitions))
            .expect("field offset fits u32"),
    ]
}

/// Returns private display-topology sizes and offsets compiled into the Rust mirrors.
#[cfg(feature = "sck-suspension-diagnostics")]
pub(crate) fn declared_sck_diagnostics_topology_layout() -> [u32; 4] {
    [
        u32::try_from(size_of::<SckDiagnosticsDisplayMode>()).expect("structure size fits u32"),
        u32::try_from(size_of::<SckDiagnosticsDisplayTopology>()).expect("structure size fits u32"),
        u32::try_from(std::mem::offset_of!(
            SckDiagnosticsDisplayMode,
            refresh_millihertz
        ))
        .expect("field offset fits u32"),
        u32::try_from(std::mem::offset_of!(SckDiagnosticsDisplayTopology, modes))
            .expect("field offset fits u32"),
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
    /// Authoritative target geometry changed after event preparation.
    GeometryChanged,
    /// A caller-selected focus predicate was false at the final authority gate.
    FocusRequired,
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
            13 => ShimStatus::GeometryChanged,
            14 => ShimStatus::FocusRequired,
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
            ShimStatus::GeometryChanged => 13,
            ShimStatus::FocusRequired => 14,
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
            ShimStatus::FrameIncomplete
            | ShimStatus::GeometryChanged
            | ShimStatus::FocusRequired => CaptureFault::SourceInvalid,
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

/// Caller-clock state consulted immediately before a native mutable gate.
///
/// The checkpoint runs while every mutable observation is still ahead of it, so
/// arbitrary caller clock code cannot stale retained-window authority. It also
/// supplies the caller clock's current remaining slice for adapter-owned native
/// wait bounding.
#[derive(Debug)]
struct ProcessOperationCheckpoint<'a> {
    operation: &'a OperationContext,
}

impl<'a> ProcessOperationCheckpoint<'a> {
    const fn new(operation: &'a OperationContext) -> Self {
        Self { operation }
    }
}

/// Atomic cancellation state consulted after final mutable authority.
#[derive(Debug)]
struct ProcessCancellationFence {
    cancellation: Option<mado_pilot_core::CancellationToken>,
    cancellation_observed: AtomicBool,
}

impl ProcessCancellationFence {
    fn new(operation: &OperationContext) -> Self {
        Self::from_cancellation(operation.cancellation())
    }

    fn from_cancellation(cancellation: Option<&mado_pilot_core::CancellationToken>) -> Self {
        Self {
            cancellation: cancellation.cloned(),
            cancellation_observed: AtomicBool::new(false),
        }
    }

    fn cancellation_observed(&self) -> bool {
        self.cancellation_observed.load(Ordering::Acquire)
    }
}
type ProcessCheckpointCallback = unsafe extern "C" fn(*mut c_void, *mut u64) -> u32;
type ProcessCancellationCallback = unsafe extern "C" fn(*mut c_void) -> u32;

unsafe extern "C" fn process_operation_checkpoint_callback(
    context: *mut c_void,
    out_wait_nanos: *mut u64,
) -> u32 {
    if context.is_null() || out_wait_nanos.is_null() {
        return ShimStatus::InvalidArgument.as_raw();
    }
    // SAFETY: the output was validated for one writable u64.
    unsafe { out_wait_nanos.write(0) };
    let outcome = catch_panic(|| {
        // SAFETY: `process_post` passes a stack-owned checkpoint that outlives
        // its synchronous native call. The shim never stores the pointer.
        let checkpoint = unsafe { &*context.cast::<ProcessOperationCheckpoint<'_>>() };
        if checkpoint
            .operation
            .cancellation()
            .is_some_and(mado_pilot_core::CancellationToken::is_cancelled)
        {
            return ShimStatus::TimedOut;
        }
        let wait = checkpoint
            .operation
            .remaining()
            .map_or(MAX_NATIVE_WAIT, |remaining| remaining.min(MAX_NATIVE_WAIT));
        if wait.is_zero() {
            return ShimStatus::TimedOut;
        }
        // SAFETY: the output remains exclusively writable for this callback.
        unsafe { out_wait_nanos.write(nanos(wait)) };
        ShimStatus::Ok
    });
    outcome.map_or(ShimStatus::PlatformFailure.as_raw(), ShimStatus::as_raw)
}

unsafe extern "C" fn process_cancellation_callback(context: *mut c_void) -> u32 {
    if context.is_null() {
        return ShimStatus::InvalidArgument.as_raw();
    }
    let outcome = catch_panic(|| {
        // SAFETY: the native callers pass a stack-owned fence that outlives
        // their synchronous call. The shim never stores the pointer.
        let fence = unsafe { &*context.cast::<ProcessCancellationFence>() };
        if fence
            .cancellation
            .as_ref()
            .is_some_and(mado_pilot_core::CancellationToken::is_cancelled)
        {
            fence.cancellation_observed.store(true, Ordering::Release);
            ShimStatus::TimedOut
        } else {
            ShimStatus::Ok
        }
    });
    outcome.map_or(ShimStatus::PlatformFailure.as_raw(), ShimStatus::as_raw)
}

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
    let outcome = catch_panic(|| {
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
    });
    match outcome {
        Ok(status) => status.as_raw(),
        // A panicking host callback becomes a typed failure rather than an abort.
        Err(()) => ShimStatus::PlatformFailure.as_raw(),
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
    let outcome = catch_panic(|| {
        // SAFETY: the caller guarantees `context` is the registered value for a
        // live session.
        let owner = unsafe { &*context.cast::<C>() };
        body(owner)
    });
    match outcome {
        Ok(status) => status.as_raw(),
        Err(()) => ShimStatus::PlatformFailure.as_raw(),
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
    let _outcome = catch_panic(|| {
        // SAFETY: the caller guarantees `context` is the registered value for a
        // live session.
        let owner = unsafe { &*context.cast::<C>() };
        body(owner, ShimStatus::from_raw(status));
    });
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
fn testing_seconds_to_nanos(seconds: f64) -> u64 {
    // SAFETY: the native helper accepts one scalar and performs no callbacks.
    unsafe { mp_shim_testing_seconds_to_nanos(seconds) }
}

#[cfg(test)]
fn testing_stop_callback_exception() -> Result<(ShimStatus, u32, ShimStatus), ShimStatus> {
    let mut terminal = u32::MAX;
    let mut calls = u32::MAX;
    let mut fence = u32::MAX;
    // SAFETY: every output is writable. The native seam owns its fake session
    // and contains the injected Objective-C exception before returning.
    let status = unsafe {
        mp_shim_testing_stop_callback_exception(&raw mut terminal, &raw mut calls, &raw mut fence)
    };
    ShimStatus::from_raw(status).into_result()?;
    Ok((
        ShimStatus::from_raw(terminal),
        calls,
        ShimStatus::from_raw(fence),
    ))
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
#[derive(Debug, PartialEq, Eq)]
struct SessionSyncInit {
    attempts: u32,
    initialized: u32,
    destroyed: u32,
    success: bool,
}

#[cfg(test)]
fn testing_session_sync_init(fail_at: u32) -> Result<SessionSyncInit, ShimStatus> {
    let mut attempts = 0;
    let mut initialized = 0;
    let mut destroyed = 0;
    let mut success = 0;
    // SAFETY: every output points to one writable `u32`; the native seam keeps
    // all synchronization objects within its call and invokes no callback.
    let status = unsafe {
        mp_shim_testing_session_sync_init(
            fail_at,
            &raw mut attempts,
            &raw mut initialized,
            &raw mut destroyed,
            &raw mut success,
        )
    };
    ShimStatus::from_raw(status).into_result()?;
    Ok(SessionSyncInit {
        attempts,
        initialized,
        destroyed,
        success: success != 0,
    })
}

#[cfg(test)]
fn testing_resource_allocation_failures() -> Result<[ShimStatus; 2], ShimStatus> {
    let mut semaphore = u32::MAX;
    let mut session_hold = u32::MAX;
    // SAFETY: both outputs point to one writable status and both native
    // factories are invoked with deterministic failure before retaining input.
    let status = unsafe {
        mp_shim_testing_resource_allocation_failures(&raw mut semaphore, &raw mut session_hold)
    };
    ShimStatus::from_raw(status).into_result()?;
    Ok([
        ShimStatus::from_raw(semaphore),
        ShimStatus::from_raw(session_hold),
    ])
}

#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
struct FixtureLaunchObservation {
    launch_status: ShimStatus,
    submission_calls: u32,
    graceful_termination_calls: u32,
    force_termination_calls: u32,
    terminated: bool,
    live_during_handle: u64,
    live_after_release: u64,
}

#[cfg(test)]
fn testing_fixture_application_launch(
    scenario: u32,
) -> Result<FixtureLaunchObservation, ShimStatus> {
    let mut launch_status = u32::MAX;
    let mut submission_calls = u32::MAX;
    let mut graceful_termination_calls = u32::MAX;
    let mut force_termination_calls = u32::MAX;
    let mut terminated = u32::MAX;
    let mut live_during_handle = u64::MAX;
    let mut live_after_release = u64::MAX;
    // SAFETY: every output points to one writable scalar. The native seam drives
    // the production launch helper with retained Objective-C test objects and
    // joins its asynchronous completion before returning.
    let status = unsafe {
        mp_shim_testing_fixture_application_launch(
            scenario,
            &raw mut launch_status,
            &raw mut submission_calls,
            &raw mut graceful_termination_calls,
            &raw mut force_termination_calls,
            &raw mut terminated,
            &raw mut live_during_handle,
            &raw mut live_after_release,
        )
    };
    ShimStatus::from_raw(status).into_result()?;
    Ok(FixtureLaunchObservation {
        launch_status: ShimStatus::from_raw(launch_status),
        submission_calls,
        graceful_termination_calls,
        force_termination_calls,
        terminated: terminated != 0,
        live_during_handle,
        live_after_release,
    })
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
fn testing_target_without_process_lifetime() -> Result<(bool, bool), ShimStatus> {
    let mut capture_metadata_retained = u32::MAX;
    let mut process_metadata_retained = u32::MAX;
    // SAFETY: both outputs are writable. The native seam uses retained NSObject
    // instances and the same target materialization helper as discovery.
    let status = unsafe {
        mp_shim_testing_target_without_process_lifetime(
            &raw mut capture_metadata_retained,
            &raw mut process_metadata_retained,
        )
    };
    ShimStatus::from_raw(status).into_result()?;
    Ok((
        capture_metadata_retained == 1,
        process_metadata_retained == 1,
    ))
}

#[cfg(test)]
fn testing_input_activation_lifetime_loss() -> Result<(ShimStatus, [u32; 2]), ShimStatus> {
    let mut activation = u32::MAX;
    let mut calls = [u32::MAX; 2];
    let [validation_calls, activation_calls] = &mut calls;
    // SAFETY: every output is writable for its declared scalar type. The native
    // seam uses a sentinel target only through injected pure callbacks.
    let status = unsafe {
        mp_shim_testing_input_activation_lifetime_loss(
            &raw mut activation,
            &raw mut *validation_calls,
            &raw mut *activation_calls,
        )
    };
    ShimStatus::from_raw(status).into_result()?;
    Ok((ShimStatus::from_raw(activation), calls))
}

#[cfg(test)]
const TEST_INPUT_ENVIRONMENT_STABLE_IDENTITY: u32 = 0;
#[cfg(test)]
const TEST_INPUT_ENVIRONMENT_PID_CHANGE: u32 = 1;
#[cfg(test)]
const TEST_INPUT_ENVIRONMENT_LAUNCH_TIME_CHANGE: u32 = 2;
#[cfg(test)]
const TEST_INPUT_ENVIRONMENT_APPLICATION_CHANGE: u32 = 3;
#[cfg(test)]
const TEST_INPUT_ENVIRONMENT_POINTER_FAILURE: u32 = 4;
#[cfg(test)]
const TEST_INPUT_ENVIRONMENT_SECOND_LIFETIME_FAILURE: u32 = 5;
#[cfg(test)]
const TEST_INPUT_ENVIRONMENT_COMPLETE_TRACE: u32 = 0x12312;
#[cfg(test)]
const TEST_INPUT_ENVIRONMENT_POINTER_FAILURE_TRACE: u32 = 0x123;

#[cfg(test)]
const TEST_INPUT_SINGLE_CONFIGURE_EXCEPTION: u32 = 0;
#[cfg(test)]
const TEST_INPUT_SINGLE_POST_EXCEPTION: u32 = 1;
#[cfg(test)]
const TEST_INPUT_TEXT_SECOND_ALLOCATION_FAILURE: u32 = 0;
#[cfg(test)]
const TEST_INPUT_TEXT_CONFIGURE_EXCEPTION: u32 = 1;
#[cfg(test)]
const TEST_INPUT_TEXT_POST_EXCEPTION: u32 = 2;
#[cfg(test)]
const TEST_PREPARED_INPUT_SUCCESS: u32 = 0;
#[cfg(test)]
const TEST_PREPARED_INPUT_CANCELLED: u32 = 1;
#[cfg(test)]
const TEST_PREPARED_INPUT_DEADLINE: u32 = 2;
#[cfg(test)]
const TEST_PREPARED_INPUT_POST_EXCEPTION: u32 = 3;
#[cfg(test)]
const TEST_PROCESS_EVENT_SOURCE_NATIVE_FAILURE: u32 = 0;
#[cfg(test)]
const TEST_PROCESS_EVENT_SOURCE_WRAPPER_FAILURE: u32 = 1;

#[cfg(test)]
fn testing_input_single_event_failure(
    scenario: u32,
) -> Result<(ShimStatus, [usize; 4]), ShimStatus> {
    let mut delivery = u32::MAX;
    let mut observations = [usize::MAX; 4];
    let [configurations, posts, releases, posted] = &mut observations;
    // SAFETY: every output is writable for its declared scalar type. The native
    // seam uses a fake object and injected callbacks that never post to the host.
    let status = unsafe {
        mp_shim_testing_input_single_event_failure(
            scenario,
            &raw mut delivery,
            &raw mut *configurations,
            &raw mut *posts,
            &raw mut *releases,
            &raw mut *posted,
        )
    };
    ShimStatus::from_raw(status).into_result()?;
    Ok((ShimStatus::from_raw(delivery), observations))
}

#[cfg(test)]
fn testing_input_text_failure(scenario: u32) -> Result<(ShimStatus, [usize; 5]), ShimStatus> {
    let mut delivery = u32::MAX;
    let mut observations = [usize::MAX; 5];
    let [allocations, configurations, posts, releases, posted] = &mut observations;
    // SAFETY: every output is writable for its declared scalar type. The native
    // seam uses fake objects and never posts to the host system.
    let status = unsafe {
        mp_shim_testing_input_text_failure(
            scenario,
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

#[cfg(test)]
fn testing_prepared_input_gate(
    scenario: u32,
) -> Result<(ShimStatus, bool, u64, usize), ShimStatus> {
    let mut delivery = u32::MAX;
    let mut native_effect_may_have_occurred = u32::MAX;
    let mut post_calls = u64::MAX;
    let mut next_index = usize::MAX;
    // SAFETY: every output is writable. The native seam posts only a sentinel
    // through an injected callback and never reaches the host desktop.
    let status = unsafe {
        mp_shim_testing_prepared_input_gate(
            scenario,
            &raw mut delivery,
            &raw mut native_effect_may_have_occurred,
            &raw mut post_calls,
            &raw mut next_index,
        )
    };
    ShimStatus::from_raw(status).into_result()?;
    Ok((
        ShimStatus::from_raw(delivery),
        native_effect_may_have_occurred == 1,
        post_calls,
        next_index,
    ))
}

#[cfg(test)]
#[derive(Debug, PartialEq)]
struct TestingInputEnvironmentSample {
    process: i64,
    process_launch_time: f64,
    pointer_x: f64,
    pointer_y: f64,
}

#[cfg(test)]
fn testing_input_environment(
    scenario: u32,
) -> Result<(ShimStatus, TestingInputEnvironmentSample, [u32; 3], u32), ShimStatus> {
    let mut sampling = u32::MAX;
    let mut process = i64::MAX;
    let mut process_launch_time = f64::NAN;
    let mut pointer_x = f64::NAN;
    let mut pointer_y = f64::NAN;
    let mut calls = [u32::MAX; 3];
    let [frontmost_calls, lifetime_calls, pointer_calls] = &mut calls;
    let mut operation_trace = u32::MAX;
    // SAFETY: every output is writable for its declared scalar type. The native
    // seam samples only deterministic objects and values through injected ops.
    let status = unsafe {
        mp_shim_testing_input_environment(
            scenario,
            &raw mut sampling,
            &raw mut process,
            &raw mut process_launch_time,
            &raw mut pointer_x,
            &raw mut pointer_y,
            &raw mut *frontmost_calls,
            &raw mut *lifetime_calls,
            &raw mut *pointer_calls,
            &raw mut operation_trace,
        )
    };
    ShimStatus::from_raw(status).into_result()?;
    Ok((
        ShimStatus::from_raw(sampling),
        TestingInputEnvironmentSample {
            process,
            process_launch_time,
            pointer_x,
            pointer_y,
        },
        calls,
        operation_trace,
    ))
}

#[cfg(test)]
fn testing_required_ax_error_status(scenario: u32) -> ShimStatus {
    // SAFETY: the testing seam accepts one scalar scenario and returns a status.
    ShimStatus::from_raw(unsafe { mp_shim_testing_required_ax_error_status(scenario) })
}

#[cfg(test)]
fn testing_process_event_source_release_exception() -> Result<(u32, bool), ShimStatus> {
    let mut release_calls = u32::MAX;
    let mut cleanup_completed = u32::MAX;
    // SAFETY: both outputs are writable scalars. The native seam uses a sentinel
    // source consumed only by an injected release callback.
    let status = unsafe {
        mp_shim_testing_process_event_source_release_exception(
            &raw mut release_calls,
            &raw mut cleanup_completed,
        )
    };
    ShimStatus::from_raw(status).into_result()?;
    Ok((release_calls, cleanup_completed == 1))
}

#[cfg(test)]
fn testing_process_event_source_allocation_failure(
    scenario: u32,
) -> Result<(ShimStatus, bool, [u32; 3]), ShimStatus> {
    let mut creation = u32::MAX;
    let mut source_is_null = u32::MAX;
    let mut calls = [u32::MAX; 3];
    let [create_calls, allocation_calls, release_calls] = &mut calls;
    // SAFETY: every output is a writable scalar. The native seam uses only
    // injected factory functions and never creates or posts a host event.
    let status = unsafe {
        mp_shim_testing_process_event_source_allocation_failure(
            scenario,
            &raw mut creation,
            &raw mut source_is_null,
            &raw mut *create_calls,
            &raw mut *allocation_calls,
            &raw mut *release_calls,
        )
    };
    ShimStatus::from_raw(status).into_result()?;
    Ok((ShimStatus::from_raw(creation), source_is_null == 1, calls))
}

#[cfg(test)]
fn testing_target_release_exception(raise_slot: u32) -> Result<(u32, bool), ShimStatus> {
    let mut release_calls = u32::MAX;
    let mut cleanup_completed = u32::MAX;
    // SAFETY: both outputs are writable scalars. The native seam uses sentinel
    // target objects consumed only by an injected release callback.
    let status = unsafe {
        mp_shim_testing_target_release_exception(
            raise_slot,
            &raw mut release_calls,
            &raw mut cleanup_completed,
        )
    };
    ShimStatus::from_raw(status).into_result()?;
    Ok((release_calls, cleanup_completed == 1))
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProcessPostTestObservation {
    delivery: ShimStatus,
    invoked_native_units: u64,
    native_effect_may_have_occurred: bool,
    target_match_count: u32,
    focus: ProcessFocusObservation,
    calls: [u64; 9],
}

#[cfg(test)]
fn testing_process_post(scenario: u32) -> Result<ProcessPostTestObservation, ShimStatus> {
    let mut delivery = u32::MAX;
    let mut invoked_native_units = u64::MAX;
    let mut native_effect_may_have_occurred = u32::MAX;
    let mut target_match_count = u32::MAX;
    let mut focus_result = u32::MAX;
    let mut calls = [u64::MAX; 9];
    let [
        authority,
        preflight,
        lifetime,
        focus,
        prepare,
        post,
        release,
        checkpoint,
        cancellation,
    ] = &mut calls;
    // SAFETY: every output is writable for its declared scalar type. The native
    // seam uses sentinel events handled only by injected callbacks and never
    // invokes Core Graphics posting.
    let status = unsafe {
        mp_shim_testing_process_post(
            scenario,
            &raw mut delivery,
            &raw mut invoked_native_units,
            &raw mut native_effect_may_have_occurred,
            &raw mut target_match_count,
            &raw mut focus_result,
            &raw mut *authority,
            &raw mut *preflight,
            &raw mut *lifetime,
            &raw mut *focus,
            &raw mut *prepare,
            &raw mut *post,
            &raw mut *release,
            &raw mut *checkpoint,
            &raw mut *cancellation,
        )
    };
    ShimStatus::from_raw(status).into_result()?;
    Ok(ProcessPostTestObservation {
        delivery: ShimStatus::from_raw(delivery),
        invoked_native_units,
        native_effect_may_have_occurred: native_effect_may_have_occurred == 1,
        target_match_count,
        focus: ProcessFocusObservation::from_raw(focus_result),
        calls,
    })
}

#[cfg(test)]
fn testing_validate_process_post(scenario: u32) -> Result<(ShimStatus, u32, u64, u32), ShimStatus> {
    let mut delivery = u32::MAX;
    let mut target_match_count = u32::MAX;
    let mut invoked_native_units = u64::MAX;
    let mut native_effect_may_have_occurred = u32::MAX;
    // SAFETY: every output is writable. Every native scenario fails validation
    // before dereferencing sentinel retained objects or invoking Core Graphics.
    let status = unsafe {
        mp_shim_testing_validate_process_post(
            scenario,
            &raw mut delivery,
            &raw mut target_match_count,
            &raw mut invoked_native_units,
            &raw mut native_effect_may_have_occurred,
        )
    };
    ShimStatus::from_raw(status).into_result()?;
    Ok((
        ShimStatus::from_raw(delivery),
        target_match_count,
        invoked_native_units,
        native_effect_may_have_occurred,
    ))
}

#[cfg(test)]
fn testing_process_authority_rules(scenario: u32) -> Result<(ShimStatus, u32), ShimStatus> {
    let mut authority = u32::MAX;
    let mut target_match_count = u32::MAX;
    // SAFETY: both outputs are writable scalars. The native seam uses only local
    // Objective-C test objects and performs no platform query.
    let status = unsafe {
        mp_shim_testing_process_authority_rules(
            scenario,
            &raw mut authority,
            &raw mut target_match_count,
        )
    };
    ShimStatus::from_raw(status).into_result()?;
    Ok((ShimStatus::from_raw(authority), target_match_count))
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

#[repr(C)]
#[derive(Debug)]
struct NativeProcessAuthority {
    struct_size: u32,
    target_match_count: u32,
    logical_x: f64,
    logical_y: f64,
    logical_width: f64,
    logical_height: f64,
    backing_scale: f64,
}

#[repr(C)]
#[derive(Debug)]
struct NativeProcessPostRequest {
    struct_size: u32,
    event_kind: u32,
    target: *const OpaqueTarget,
    event_source: *const OpaqueProcessEventSource,
    timeout_nanos: u64,
    flags: u32,
    geometry_check: u32,
    purpose: u32,
    action: u32,
    button: u32,
    click_state: u64,
    x: f64,
    y: f64,
    horizontal: i32,
    vertical: i32,
    key_code: u16,
    key_down: bool,
    focus_requirement: u8,
    reserved: [u8; 4],
    text_units: *const u16,
    text_unit_count: usize,
    expected_x: f64,
    expected_y: f64,
    expected_width: f64,
    expected_height: f64,
    expected_scale: f64,
    interruption_context: *mut c_void,
    interruption_callback: Option<ProcessCheckpointCallback>,
    cancellation_context: *mut c_void,
    cancellation_callback: Option<ProcessCancellationCallback>,
}

#[repr(C)]
#[derive(Debug)]
struct NativeProcessPostReport {
    struct_size: u32,
    target_match_count: u32,
    invoked_native_units: u64,
    authorization: u32,
    geometry_result: u32,
    focus_result: u32,
    native_effect_may_have_occurred: u32,
}

unsafe extern "C" {
    fn mp_shim_abi_version() -> u32;
    fn mp_shim_struct_sizes(
        out_target_info: *mut u32,
        out_frame_info: *mut u32,
        out_open_request: *mut u32,
        out_process_authority: *mut u32,
        out_process_post_request: *mut u32,
        out_process_post_report: *mut u32,
    ) -> u32;
    fn mp_shim_process_struct_offsets(
        out_authority_target_match_count: *mut u32,
        out_request_target: *mut u32,
        out_request_event_source: *mut u32,
        out_request_timeout_nanos: *mut u32,
        out_report_target_match_count: *mut u32,
        out_report_invoked_native_units: *mut u32,
    ) -> u32;
    #[cfg(feature = "sck-suspension-diagnostics")]
    fn mp_shim_sck_diagnostics_layout(
        out_event_size: *mut u32,
        out_snapshot_size: *mut u32,
        out_event_raw_value_offset: *mut u32,
        out_event_sequence_offset: *mut u32,
        out_event_monotonic_nanos_offset: *mut u32,
        out_status_counts_offset: *mut u32,
        out_first_status_offset: *mut u32,
        out_callbacks_received_offset: *mut u32,
        out_transitions_offset: *mut u32,
    ) -> u32;
    #[cfg(feature = "sck-suspension-diagnostics")]
    fn mp_shim_sck_diagnostics_topology_layout(
        out_mode_size: *mut u32,
        out_topology_size: *mut u32,
        out_mode_refresh_offset: *mut u32,
        out_topology_modes_offset: *mut u32,
    ) -> u32;
    #[cfg(feature = "sck-suspension-diagnostics")]
    fn mp_shim_sck_diagnostics_read_display_topology(
        out_topology: *mut SckDiagnosticsDisplayTopology,
    ) -> u32;
    fn mp_shim_capture_available() -> u32;
    fn mp_shim_probe_screen_capture(out_state: *mut u32) -> u32;
    fn mp_shim_process_authorization(
        out_post_event_access: *mut u32,
        out_accessibility: *mut u32,
    ) -> u32;
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
    fn mp_shim_testing_seconds_to_nanos(seconds: f64) -> u64;
    #[cfg(test)]
    fn mp_shim_testing_stop_callback_exception(
        out_terminal_status: *mut u32,
        out_terminal_calls: *mut u32,
        out_fence_status: *mut u32,
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
    fn mp_shim_testing_session_sync_init(
        fail_at: u32,
        out_attempts: *mut u32,
        out_initialized: *mut u32,
        out_destroyed: *mut u32,
        out_success: *mut u32,
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
    fn mp_shim_testing_target_without_process_lifetime(
        out_capture_metadata_retained: *mut u32,
        out_process_metadata_retained: *mut u32,
    ) -> u32;
    #[cfg(test)]
    fn mp_shim_testing_resource_allocation_failures(
        out_semaphore_status: *mut u32,
        out_session_hold_status: *mut u32,
    ) -> u32;
    #[cfg(test)]
    fn mp_shim_testing_fixture_application_launch(
        scenario: u32,
        out_launch_status: *mut u32,
        out_submission_calls: *mut u32,
        out_graceful_termination_calls: *mut u32,
        out_force_termination_calls: *mut u32,
        out_terminated: *mut u32,
        out_live_during_handle: *mut u64,
        out_live_after_release: *mut u64,
    ) -> u32;
    #[cfg(test)]
    fn mp_shim_testing_input_activation_lifetime_loss(
        out_activation_status: *mut u32,
        out_validation_calls: *mut u32,
        out_activation_calls: *mut u32,
    ) -> u32;
    #[cfg(test)]
    fn mp_shim_testing_input_environment(
        scenario: u32,
        out_sampling_status: *mut u32,
        out_process: *mut i64,
        out_process_launch_time: *mut f64,
        out_pointer_x: *mut f64,
        out_pointer_y: *mut f64,
        out_frontmost_calls: *mut u32,
        out_lifetime_calls: *mut u32,
        out_pointer_calls: *mut u32,
        out_operation_trace: *mut u32,
    ) -> u32;
    #[cfg(test)]
    fn mp_shim_testing_input_single_event_failure(
        scenario: u32,
        out_delivery_status: *mut u32,
        out_configurations: *mut usize,
        out_posts: *mut usize,
        out_releases: *mut usize,
        out_posted: *mut usize,
    ) -> u32;
    #[cfg(test)]
    fn mp_shim_testing_input_text_failure(
        scenario: u32,
        out_delivery_status: *mut u32,
        out_allocations: *mut usize,
        out_configurations: *mut usize,
        out_posts: *mut usize,
        out_releases: *mut usize,
        out_posted: *mut usize,
    ) -> u32;
    #[cfg(test)]
    fn mp_shim_testing_prepared_input_gate(
        scenario: u32,
        out_delivery_status: *mut u32,
        out_native_effect_may_have_occurred: *mut u32,
        out_post_calls: *mut u64,
        out_next_index: *mut usize,
    ) -> u32;
    #[cfg(test)]
    fn mp_shim_testing_required_ax_error_status(scenario: u32) -> u32;
    #[cfg(test)]
    fn mp_shim_testing_process_event_source_release_exception(
        out_release_calls: *mut u32,
        out_cleanup_completed: *mut u32,
    ) -> u32;
    #[cfg(test)]
    fn mp_shim_testing_process_event_source_allocation_failure(
        scenario: u32,
        out_creation_status: *mut u32,
        out_source_is_null: *mut u32,
        out_create_calls: *mut u32,
        out_allocation_calls: *mut u32,
        out_release_calls: *mut u32,
    ) -> u32;
    #[cfg(test)]
    fn mp_shim_testing_target_release_exception(
        raise_slot: u32,
        out_release_calls: *mut u32,
        out_cleanup_completed: *mut u32,
    ) -> u32;
    #[cfg(test)]
    fn mp_shim_testing_process_post(
        scenario: u32,
        out_delivery_status: *mut u32,
        out_invoked_native_units: *mut u64,
        out_native_effect_may_have_occurred: *mut u32,
        out_target_match_count: *mut u32,
        out_focus_result: *mut u32,
        out_authority_calls: *mut u64,
        out_preflight_calls: *mut u64,
        out_lifetime_calls: *mut u64,
        out_focus_calls: *mut u64,
        out_prepare_calls: *mut u64,
        out_post_calls: *mut u64,
        out_release_calls: *mut u64,
        out_checkpoint_calls: *mut u64,
        out_cancellation_calls: *mut u64,
    ) -> u32;
    #[cfg(test)]
    fn mp_shim_testing_validate_process_post(
        scenario: u32,
        out_delivery_status: *mut u32,
        out_target_match_count: *mut u32,
        out_invoked_native_units: *mut u64,
        out_native_effect_may_have_occurred: *mut u32,
    ) -> u32;
    #[cfg(test)]
    fn mp_shim_testing_process_authority_rules(
        scenario: u32,
        out_authority_status: *mut u32,
        out_target_match_count: *mut u32,
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
    #[cfg(feature = "sck-suspension-diagnostics")]
    fn mp_shim_sck_diagnostics_snapshot_read(
        session: *const OpaqueSession,
        out_snapshot: *mut SckDiagnosticsSnapshot,
    ) -> u32;

    fn mp_shim_frame_detach(borrowed: *mut OpaqueFrame, out: *mut *mut OpaqueFrame) -> u32;
    fn mp_shim_frame_release(frame: *mut OpaqueFrame);
    #[cfg(feature = "sck-suspension-diagnostics")]
    fn mp_shim_sck_diagnostics_process_resources(
        out_native_objects: *mut u64,
        out_detached_bytes: *mut u64,
        out_live_sessions: *mut u64,
        out_callbacks_in_flight: *mut u64,
    ) -> u32;
    fn mp_shim_frame_copy_out(
        frame: *const OpaqueFrame,
        destination: *mut u8,
        capacity: usize,
        destination_stride: u64,
    ) -> u32;

    #[cfg(all(test, feature = "sck-suspension-diagnostics"))]
    fn mp_shim_sck_diagnostics_test_probe_open(
        out_probe: *mut *mut OpaqueSckDiagnosticsProbe,
    ) -> u32;
    #[cfg(all(test, feature = "sck-suspension-diagnostics"))]
    fn mp_shim_sck_diagnostics_test_probe_record(
        probe: *mut OpaqueSckDiagnosticsProbe,
        present: u32,
        raw_value: i64,
        out_would_publish: *mut u32,
    ) -> u32;
    #[cfg(all(test, feature = "sck-suspension-diagnostics"))]
    fn mp_shim_sck_diagnostics_test_probe_seed_saturation(
        probe: *mut OpaqueSckDiagnosticsProbe,
    ) -> u32;
    #[cfg(all(test, feature = "sck-suspension-diagnostics"))]
    fn mp_shim_sck_diagnostics_test_probe_snapshot(
        probe: *mut OpaqueSckDiagnosticsProbe,
        raise_native_exception: u32,
        out_snapshot: *mut SckDiagnosticsSnapshot,
    ) -> u32;
    #[cfg(all(test, feature = "sck-suspension-diagnostics"))]
    fn mp_shim_sck_diagnostics_test_probe_release(probe: *mut OpaqueSckDiagnosticsProbe);
    #[cfg(all(test, feature = "sck-suspension-diagnostics"))]
    fn mp_shim_sck_diagnostics_test_callback_accounting(
        raise_native_exception: u32,
        out_callback_body_calls: *mut u32,
        out_snapshot: *mut SckDiagnosticsSnapshot,
    ) -> u32;
    #[cfg(all(test, feature = "sck-suspension-diagnostics"))]
    fn mp_shim_sck_diagnostics_test_display_topology(
        modes: *const SckDiagnosticsDisplayMode,
        mode_count: u32,
        display_count: u32,
        raise_native_exception: u32,
        out_topology: *mut SckDiagnosticsDisplayTopology,
    ) -> u32;

    fn mp_shim_input_target_focused(
        target: *const OpaqueTarget,
        timeout_nanos: u64,
        out_focused: *mut bool,
    ) -> u32;
    fn mp_shim_input_target_bounds(
        target: *const OpaqueTarget,
        timeout_nanos: u64,
        out_x: *mut f64,
        out_y: *mut f64,
        out_width: *mut f64,
        out_height: *mut f64,
        out_scale: *mut f64,
    ) -> u32;
    fn mp_shim_process_authority(
        target: *const OpaqueTarget,
        timeout_nanos: u64,
        out_authority: *mut NativeProcessAuthority,
    ) -> u32;
    fn mp_shim_process_event_source_create(
        activity_tag: u64,
        out_source: *mut *mut OpaqueProcessEventSource,
    ) -> u32;
    fn mp_shim_process_event_source_release(source: *mut OpaqueProcessEventSource);
    fn mp_shim_process_post(
        request: *const NativeProcessPostRequest,
        out_report: *mut NativeProcessPostReport,
    ) -> u32;
    fn mp_shim_input_pointer_location(out_x: *mut f64, out_y: *mut f64) -> u32;
    fn mp_shim_input_activate_owner(target: *const OpaqueTarget) -> u32;
    fn mp_shim_input_resolve_character(scalar: u32, out_key_code: *mut u16) -> u32;
    fn mp_shim_input_prepare_pointer(
        action: u32,
        button: u32,
        click_state: u64,
        x: f64,
        y: f64,
        flags: u32,
        out_prepared: *mut *mut OpaquePreparedInput,
    ) -> u32;
    fn mp_shim_input_prepare_scroll(
        horizontal: i32,
        vertical: i32,
        x: f64,
        y: f64,
        flags: u32,
        out_prepared: *mut *mut OpaquePreparedInput,
    ) -> u32;
    fn mp_shim_input_prepare_key(
        key_code: u16,
        down: bool,
        flags: u32,
        out_prepared: *mut *mut OpaquePreparedInput,
    ) -> u32;
    fn mp_shim_input_prepare_text(
        units: *const u16,
        count: usize,
        flags: u32,
        out_prepared: *mut *mut OpaquePreparedInput,
    ) -> u32;
    fn mp_shim_input_prepared_count(
        prepared: *const OpaquePreparedInput,
        out_count: *mut usize,
    ) -> u32;
    fn mp_shim_input_post_prepared(
        prepared: *mut OpaquePreparedInput,
        index: usize,
        deadline_nanos: u64,
        cancellation_context: *mut c_void,
        cancellation_callback: Option<ProcessCancellationCallback>,
        out_native_effect_may_have_occurred: *mut u32,
    ) -> u32;
    fn mp_shim_input_prepared_release(prepared: *mut OpaquePreparedInput) -> u32;
}

// The C header spells the borrowed name view as `const uint8_t *`; this keeps the
// Rust side honest about the element type it reads back.
const _: () = assert!(size_of::<c_char>() == size_of::<u8>());

#[cfg(test)]
mod tests {
    use std::ffi::c_void;
    use std::panic;
    use std::ptr::NonNull;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::Duration;

    use super::{
        ABI_VERSION, DEFAULT_NATIVE_WAIT, ExecutionContext, FrameInfo, KIND_DISPLAY, KIND_WINDOW,
        LaunchContext, MAX_NATIVE_WAIT, MAX_SURFACE_EXTENT, OpaqueFrame, OpenRequest,
        ProcessAuthorization, ProcessCancellationFence, ProcessEventSource,
        ProcessFocusObservation, ProcessOperationCheckpoint, SessionSyncInit, ShimStatus,
        SignatureMode, TEST_FIXTURE_COMPLETION_EXCEPTION, TEST_FIXTURE_HANDLE_ALLOCATION_FAILURE,
        TEST_FIXTURE_LATE_COMPLETION, TEST_FIXTURE_REAPER_HANDOFF,
        TEST_FIXTURE_RELEASE_REAPER_HANDOFF, TEST_FIXTURE_SEMAPHORE_ALLOCATION_FAILURE,
        TEST_FIXTURE_SUCCESSFUL_RELEASE, TEST_FIXTURE_VALIDATION_FAILURE,
        TEST_INPUT_ENVIRONMENT_APPLICATION_CHANGE, TEST_INPUT_ENVIRONMENT_COMPLETE_TRACE,
        TEST_INPUT_ENVIRONMENT_LAUNCH_TIME_CHANGE, TEST_INPUT_ENVIRONMENT_PID_CHANGE,
        TEST_INPUT_ENVIRONMENT_POINTER_FAILURE, TEST_INPUT_ENVIRONMENT_POINTER_FAILURE_TRACE,
        TEST_INPUT_ENVIRONMENT_SECOND_LIFETIME_FAILURE, TEST_INPUT_ENVIRONMENT_STABLE_IDENTITY,
        TEST_INPUT_SINGLE_CONFIGURE_EXCEPTION, TEST_INPUT_SINGLE_POST_EXCEPTION,
        TEST_INPUT_TEXT_CONFIGURE_EXCEPTION, TEST_INPUT_TEXT_POST_EXCEPTION,
        TEST_INPUT_TEXT_SECOND_ALLOCATION_FAILURE, TEST_PREPARED_INPUT_CANCELLED,
        TEST_PREPARED_INPUT_DEADLINE, TEST_PREPARED_INPUT_POST_EXCEPTION,
        TEST_PREPARED_INPUT_SUCCESS, TEST_PROCESS_EVENT_SOURCE_NATIVE_FAILURE,
        TEST_PROCESS_EVENT_SOURCE_WRAPPER_FAILURE, TargetToken, TestingInputEnvironmentSample,
        catch_panic, contained_frame_callback, contained_frame_commit_callback,
        contained_stopped_callback, declared_layout, declared_process_offsets, execution_context,
        linked_layout, live_objects, monotonic_nanos, nanos, process_cancellation_callback,
        process_operation_checkpoint_callback, testing_classify_signature,
        testing_fixture_application_launch, testing_gate_retries,
        testing_input_activation_lifetime_loss, testing_input_environment,
        testing_input_single_event_failure, testing_input_text_failure,
        testing_prepared_input_gate, testing_process_authority_rules,
        testing_process_event_source_allocation_failure,
        testing_process_event_source_release_exception, testing_process_post,
        testing_required_ax_error_status, testing_resource_allocation_failures,
        testing_seconds_to_nanos, testing_session_sync_init, testing_stop_callback_exception,
        testing_stop_completion_exception, testing_surface_recommendation,
        testing_target_release_exception, testing_target_without_process_lifetime,
        testing_terminalize_twice, testing_validate_process_post, validate_open_shape_and_metadata,
    };
    #[cfg(feature = "sck-suspension-diagnostics")]
    use super::{
        SCK_DIAGNOSTIC_DISPLAY_CAPACITY, SCK_DIAGNOSTIC_TRANSITION_CAPACITY, SCK_STATUS_BLANK,
        SCK_STATUS_COMPLETE, SCK_STATUS_IDLE, SCK_STATUS_MISSING, SCK_STATUS_STARTED,
        SCK_STATUS_STOPPED, SCK_STATUS_SUSPENDED, SCK_STATUS_UNKNOWN, SckDiagnosticsDisplayMode,
        SckDiagnosticsDisplayTopology, SckDiagnosticsProbe, SckDiagnosticsSnapshot,
        SckDiagnosticsStatusEvent, declared_sck_diagnostics_layout,
        declared_sck_diagnostics_topology_layout, linked_sck_diagnostics_layout,
        linked_sck_diagnostics_topology_layout, testing_sck_callback_accounting,
        testing_sck_display_topology,
    };
    use mado_pilot_capture::CaptureFault;
    use mado_pilot_core::{
        CancellationToken, Clock, MonotonicInstant, OperationContext, PixelExtent,
    };

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
        let (version, sizes, offsets) = linked_layout();

        assert_eq!(version, ABI_VERSION);
        assert_eq!(sizes, declared_layout());
        assert_eq!(offsets, declared_process_offsets());
    }

    #[cfg(feature = "sck-suspension-diagnostics")]
    #[test]
    fn the_linked_sck_diagnostic_layout_is_exact() {
        let declared = declared_sck_diagnostics_layout();

        assert_eq!(declared, [32, 808, 8, 16, 24, 40, 104, 168, 296]);
        assert_eq!(linked_sck_diagnostics_layout(), declared);
        let requested = SckDiagnosticsSnapshot::requested();
        assert_eq!(requested.struct_size, 808);
        assert_eq!(
            requested.transition_capacity,
            u32::try_from(SCK_DIAGNOSTIC_TRANSITION_CAPACITY).expect("capacity fits u32")
        );
        let topology_declared = declared_sck_diagnostics_topology_layout();
        assert_eq!(topology_declared, [32, 80, 8, 16]);
        assert_eq!(linked_sck_diagnostics_topology_layout(), topology_declared);
        let requested_topology = SckDiagnosticsDisplayTopology::requested();
        assert_eq!(requested_topology.struct_size, 80);
        assert_eq!(
            requested_topology.mode_capacity,
            u32::try_from(SCK_DIAGNOSTIC_DISPLAY_CAPACITY).expect("capacity fits u32")
        );
    }

    #[cfg(feature = "sck-suspension-diagnostics")]
    #[test]
    fn diagnostic_display_topology_is_bounded_sorted_and_identifier_free() {
        let modes = [
            SckDiagnosticsDisplayMode {
                logical_width: 3_840,
                logical_height: 2_160,
                refresh_millihertz: 120_000,
                built_in: 0,
                mirrored: 0,
                ..SckDiagnosticsDisplayMode::default()
            },
            SckDiagnosticsDisplayMode {
                logical_width: 1_512,
                logical_height: 982,
                refresh_millihertz: 120_000,
                built_in: 1,
                mirrored: 0,
                ..SckDiagnosticsDisplayMode::default()
            },
        ];
        let (status, topology) = testing_sck_display_topology(
            &modes,
            2,
            false,
            SckDiagnosticsDisplayTopology::requested(),
        );
        assert_eq!(status, ShimStatus::Ok);
        assert_eq!(topology.display_count, 2);
        assert_eq!(topology.mode_count, 2);
        assert_eq!(topology.modes[0].logical_width, 1_512);
        assert_eq!(topology.modes[1].logical_width, 3_840);
    }

    #[cfg(feature = "sck-suspension-diagnostics")]
    #[test]
    fn diagnostic_display_topology_rejects_invalid_shape_and_contains_exceptions() {
        let valid = SckDiagnosticsDisplayMode {
            logical_width: 1_512,
            logical_height: 982,
            refresh_millihertz: 120_000,
            built_in: 1,
            mirrored: 0,
            ..SckDiagnosticsDisplayMode::default()
        };
        for invalid in [
            SckDiagnosticsDisplayMode {
                logical_width: 0,
                ..valid
            },
            SckDiagnosticsDisplayMode {
                refresh_millihertz: 1_000_001,
                ..valid
            },
            SckDiagnosticsDisplayMode {
                built_in: 2,
                ..valid
            },
            SckDiagnosticsDisplayMode {
                mirrored: 2,
                ..valid
            },
        ] {
            let (status, topology) = testing_sck_display_topology(
                &[invalid],
                1,
                false,
                SckDiagnosticsDisplayTopology::requested(),
            );
            assert_eq!(status, ShimStatus::InvalidArgument);
            assert_eq!(topology.display_count, 0);
            assert_eq!(topology.mode_count, 0);
        }

        let (status, _) = testing_sck_display_topology(
            &[valid],
            2,
            false,
            SckDiagnosticsDisplayTopology::requested(),
        );
        assert_eq!(status, ShimStatus::InvalidArgument);
        let (status, topology) = testing_sck_display_topology(
            &[valid],
            1,
            true,
            SckDiagnosticsDisplayTopology::requested(),
        );
        assert_eq!(status, ShimStatus::NativeException);
        assert_eq!(topology.display_count, 0);
        assert_eq!(topology.mode_count, 0);
        let (status, _) = testing_sck_display_topology(
            &[valid],
            1,
            false,
            SckDiagnosticsDisplayTopology::default(),
        );
        assert_eq!(status, ShimStatus::InvalidArgument);
    }

    #[cfg(feature = "sck-suspension-diagnostics")]
    fn diagnostic_transitions(snapshot: &SckDiagnosticsSnapshot) -> Vec<SckDiagnosticsStatusEvent> {
        let count = usize::try_from(snapshot.transition_count)
            .expect("transition count fits usize")
            .min(SCK_DIAGNOSTIC_TRANSITION_CAPACITY);
        let start = usize::try_from(snapshot.transition_start).expect("ring index fits usize")
            % SCK_DIAGNOSTIC_TRANSITION_CAPACITY;
        (0..count)
            .map(|offset| {
                snapshot.transitions[(start + offset) % SCK_DIAGNOSTIC_TRANSITION_CAPACITY]
            })
            .collect()
    }

    #[cfg(feature = "sck-suspension-diagnostics")]
    #[test]
    fn diagnostic_statuses_normalize_without_changing_complete_publication() {
        let mut probe = SckDiagnosticsProbe::open();
        let cases = [
            (Some(0), SCK_STATUS_COMPLETE, true),
            (Some(1), SCK_STATUS_IDLE, false),
            (Some(2), SCK_STATUS_BLANK, false),
            (Some(3), SCK_STATUS_SUSPENDED, false),
            (Some(4), SCK_STATUS_STARTED, false),
            (Some(5), SCK_STATUS_STOPPED, false),
            (None, SCK_STATUS_MISSING, false),
            (Some(99), SCK_STATUS_UNKNOWN, false),
        ];
        for (raw, _, would_publish) in cases {
            assert_eq!(probe.record(raw), would_publish);
        }

        let snapshot = probe.snapshot();
        assert_eq!(snapshot.observed_total, 8);
        assert_eq!(snapshot.status_counts, [1; 8]);
        assert_eq!(snapshot.first_status.kind, SCK_STATUS_COMPLETE);
        assert_eq!(snapshot.first_status.raw_value, 0);
        assert_eq!(snapshot.last_status.kind, SCK_STATUS_UNKNOWN);
        assert_eq!(snapshot.last_status.raw_value, 99);
        assert_eq!(
            snapshot.first_complete_nanos,
            snapshot.first_status.monotonic_nanos
        );
        assert_ne!(snapshot.first_complete_nanos, 0);

        let transitions = diagnostic_transitions(&snapshot);
        assert_eq!(transitions.len(), cases.len());
        for (index, (event, (raw, kind, _))) in transitions.iter().zip(cases).enumerate() {
            assert_eq!(event.struct_size, 32);
            assert_eq!(event.kind, kind);
            assert_eq!(event.raw_value, raw.unwrap_or(i64::MIN));
            assert_eq!(
                event.sequence,
                u64::try_from(index + 1).expect("sequence fits")
            );
            if let Some(previous) = index.checked_sub(1) {
                assert!(
                    transitions[previous].monotonic_nanos <= event.monotonic_nanos,
                    "transition timestamps regressed"
                );
            }
        }
    }

    #[cfg(feature = "sck-suspension-diagnostics")]
    #[test]
    fn diagnostic_counters_saturate_and_the_ring_overwrites_oldest_transitions() {
        let mut probe = SckDiagnosticsProbe::open();
        probe.seed_saturation();
        for index in 0..20 {
            let raw = i64::from(index % 2);
            let _ = probe.record(Some(raw));
        }

        let snapshot = probe.snapshot();
        assert_eq!(snapshot.observed_total, u64::MAX);
        assert_eq!(snapshot.status_counts, [u64::MAX; 8]);
        assert_eq!(snapshot.transition_count, 16);
        assert_eq!(snapshot.transition_overwrites, 4);
        assert_eq!(snapshot.first_status.raw_value, 0);
        assert_eq!(snapshot.last_status.raw_value, 1);
        let transitions = diagnostic_transitions(&snapshot);
        assert_eq!(transitions.len(), 16);
        for (offset, event) in transitions.iter().enumerate() {
            assert_eq!(event.raw_value, i64::try_from((offset + 4) % 2).unwrap());
            assert_eq!(event.sequence, u64::MAX);
        }
    }

    #[cfg(feature = "sck-suspension-diagnostics")]
    #[test]
    fn diagnostic_snapshot_rejects_size_and_capacity_mismatch_without_writing() {
        let probe = SckDiagnosticsProbe::open();
        let mut wrong_size = SckDiagnosticsSnapshot::requested();
        wrong_size.struct_size -= 1;
        wrong_size.observed_total = 41;
        let (status, returned) = probe.snapshot_with(wrong_size, false);
        assert_eq!(status, ShimStatus::InvalidArgument);
        assert_eq!(returned, wrong_size);

        let mut wrong_capacity = SckDiagnosticsSnapshot::requested();
        wrong_capacity.transition_capacity -= 1;
        wrong_capacity.observed_total = 42;
        let (status, returned) = probe.snapshot_with(wrong_capacity, false);
        assert_eq!(status, ShimStatus::InvalidArgument);
        assert_eq!(returned, wrong_capacity);
    }

    #[cfg(feature = "sck-suspension-diagnostics")]
    #[test]
    fn diagnostic_snapshot_contains_native_exceptions_without_writing() {
        let probe = SckDiagnosticsProbe::open();
        let mut requested = SckDiagnosticsSnapshot::requested();
        requested.observed_total = 43;
        let (status, returned) = probe.snapshot_with(requested, true);
        assert_eq!(status, ShimStatus::NativeException);
        assert_eq!(returned, requested);
    }

    #[cfg(feature = "sck-suspension-diagnostics")]
    #[test]
    fn diagnostic_snapshot_is_safe_while_one_writer_records_statuses() {
        let mut probe = SckDiagnosticsProbe::open();
        let first_reader = probe.reader();
        let second_reader = first_reader.clone();
        let barrier = Arc::new(std::sync::Barrier::new(3));

        let writer_barrier = Arc::clone(&barrier);
        let writer = std::thread::spawn(move || {
            writer_barrier.wait();
            for index in 0..10_000 {
                let _ = probe.record(Some(i64::from(index % 2)));
                if index % 64 == 0 {
                    std::thread::yield_now();
                }
            }
            probe
        });
        let spawn_reader = |reader: super::SckDiagnosticsProbeReader,
                            barrier: Arc<std::sync::Barrier>| {
            std::thread::spawn(move || {
                barrier.wait();
                for _ in 0..2_000 {
                    let snapshot = reader.snapshot();
                    assert!(
                        usize::try_from(snapshot.transition_count).unwrap()
                            <= SCK_DIAGNOSTIC_TRANSITION_CAPACITY
                    );
                    for event in diagnostic_transitions(&snapshot) {
                        assert_eq!(event.struct_size, 32);
                        assert!(event.kind < 8);
                    }
                }
            })
        };
        let first = spawn_reader(first_reader, Arc::clone(&barrier));
        let second = spawn_reader(second_reader, barrier);

        let probe = writer.join().expect("diagnostic writer did not panic");
        first.join().expect("first diagnostic reader did not panic");
        second
            .join()
            .expect("second diagnostic reader did not panic");
        let snapshot = probe.snapshot();
        assert_eq!(snapshot.observed_total, 10_000);
        assert_eq!(snapshot.status_counts[0], 5_000);
        assert_eq!(snapshot.status_counts[1], 5_000);
        assert_eq!(snapshot.transition_count, 16);
        assert_eq!(snapshot.transition_overwrites, 9_984);
    }

    #[cfg(feature = "sck-suspension-diagnostics")]
    #[test]
    fn diagnostic_callback_accounting_exits_on_exception_and_refuses_after_fence() {
        for raise_native_exception in [false, true] {
            let (callback_body_calls, snapshot) =
                testing_sck_callback_accounting(raise_native_exception);
            assert_eq!(
                callback_body_calls, 1,
                "the post-fence delivery cannot enter the callback body"
            );
            assert_eq!(snapshot.callbacks_received, 2);
            assert_eq!(snapshot.callbacks_admitted, 1);
            assert_eq!(snapshot.callbacks_refused, 1);
            assert_eq!(snapshot.callbacks_exited, 2);
            assert_eq!(
                snapshot.session_references, 1,
                "one raw async hold is excluded from structural ownership"
            );
            assert_eq!(
                snapshot.callbacks_received,
                snapshot.callbacks_admitted + snapshot.callbacks_refused
            );
        }
    }

    #[test]
    fn process_event_source_has_an_owned_native_lifecycle() {
        let source = ProcessEventSource::new(0)
            .expect("this host can create a private Core Graphics source");
        drop(source);
    }

    #[test]
    fn process_event_source_release_contains_exceptions_and_finishes_cleanup() {
        let (release_calls, cleanup_completed) = testing_process_event_source_release_exception()
            .expect("the native release boundary contains its injected exception");

        assert_eq!(release_calls, 1);
        assert!(cleanup_completed);
    }

    fn assert_failed_input_environment_sample(
        scenario: u32,
        expected_status: ShimStatus,
        expected_calls: [u32; 3],
        expected_trace: u32,
    ) {
        let (status, sample, calls, trace) =
            testing_input_environment(scenario).expect("the production-shaped sampler runs");

        assert_eq!(status, expected_status);
        assert_eq!(
            sample,
            TestingInputEnvironmentSample {
                process: 0,
                process_launch_time: 0.0,
                pointer_x: 0.0,
                pointer_y: 0.0,
            }
        );
        assert_eq!(calls, expected_calls);
        assert_eq!(trace, expected_trace);
    }

    #[test]
    fn input_environment_samples_stable_identity_in_production_order() {
        let (status, sample, calls, trace) =
            testing_input_environment(TEST_INPUT_ENVIRONMENT_STABLE_IDENTITY)
                .expect("the production-shaped sampler runs");

        assert_eq!(status, ShimStatus::Ok);
        assert_eq!(
            sample,
            TestingInputEnvironmentSample {
                process: 42,
                process_launch_time: 1000.0,
                pointer_x: 12.5,
                pointer_y: -4.25,
            }
        );
        assert_eq!(calls, [2, 2, 1]);
        assert_eq!(trace, TEST_INPUT_ENVIRONMENT_COMPLETE_TRACE);
    }

    #[test]
    fn input_environment_rejects_second_frontmost_pid_change() {
        assert_failed_input_environment_sample(
            TEST_INPUT_ENVIRONMENT_PID_CHANGE,
            ShimStatus::PlatformFailure,
            [2, 2, 1],
            TEST_INPUT_ENVIRONMENT_COMPLETE_TRACE,
        );
    }

    #[test]
    fn input_environment_rejects_same_pid_with_changed_launch_time() {
        assert_failed_input_environment_sample(
            TEST_INPUT_ENVIRONMENT_LAUNCH_TIME_CHANGE,
            ShimStatus::PlatformFailure,
            [2, 2, 1],
            TEST_INPUT_ENVIRONMENT_COMPLETE_TRACE,
        );
    }

    #[test]
    fn input_environment_rejects_same_pid_with_changed_application_object() {
        assert_failed_input_environment_sample(
            TEST_INPUT_ENVIRONMENT_APPLICATION_CHANGE,
            ShimStatus::PlatformFailure,
            [2, 2, 1],
            TEST_INPUT_ENVIRONMENT_COMPLETE_TRACE,
        );
    }

    #[test]
    fn input_environment_pointer_failure_publishes_default_outputs() {
        assert_failed_input_environment_sample(
            TEST_INPUT_ENVIRONMENT_POINTER_FAILURE,
            ShimStatus::PlatformFailure,
            [1, 1, 1],
            TEST_INPUT_ENVIRONMENT_POINTER_FAILURE_TRACE,
        );
    }

    #[test]
    fn input_environment_second_lifetime_failure_publishes_default_outputs() {
        assert_failed_input_environment_sample(
            TEST_INPUT_ENVIRONMENT_SECOND_LIFETIME_FAILURE,
            ShimStatus::TargetLost,
            [2, 2, 1],
            TEST_INPUT_ENVIRONMENT_COMPLETE_TRACE,
        );
    }

    #[test]
    fn process_event_source_creation_failures_leave_no_owner_or_native_source() {
        let baseline = live_objects();
        for (scenario, expected_calls) in [
            (TEST_PROCESS_EVENT_SOURCE_NATIVE_FAILURE, [1, 0, 0]),
            (TEST_PROCESS_EVENT_SOURCE_WRAPPER_FAILURE, [1, 1, 1]),
        ] {
            let (status, source_is_null, calls) =
                testing_process_event_source_allocation_failure(scenario)
                    .expect("the deterministic allocation seam runs");

            assert_eq!(status, ShimStatus::PlatformFailure);
            assert!(source_is_null);
            assert_eq!(calls, expected_calls);
            assert_eq!(
                live_objects(),
                baseline,
                "a failed source factory retains no opaque owner"
            );
        }
    }

    #[test]
    fn target_release_contains_each_object_exception_and_finishes_cleanup() {
        for raise_slot in 0..3 {
            let (release_calls, cleanup_completed) = testing_target_release_exception(raise_slot)
                .expect("the native target release boundary contains its injected exception");

            assert_eq!(release_calls, 3, "raise slot {raise_slot}");
            assert!(cleanup_completed, "raise slot {raise_slot}");
        }
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
    fn post_event_access_remains_truth_when_accessibility_disagrees() {
        let denied = ProcessAuthorization::from_raw(1, 0);
        assert_eq!(
            denied.post_event_access,
            mado_pilot_core::PermissionState::NotGranted
        );
        assert!(denied.disagrees());

        let granted = ProcessAuthorization::from_raw(0, 1);
        assert_eq!(
            granted.post_event_access,
            mado_pilot_core::PermissionState::Granted
        );
        assert!(granted.disagrees());
    }

    #[test]
    fn unavailable_migration_observations_are_not_invented_as_disagreement() {
        let unavailable = ProcessAuthorization::from_raw(2, 3);
        assert_eq!(
            unavailable.post_event_access,
            mado_pilot_core::PermissionState::Unavailable
        );
        assert_eq!(
            unavailable.accessibility,
            mado_pilot_core::PermissionState::Unknown
        );
        assert!(!unavailable.disagrees());
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
    fn missing_public_process_lifetime_keeps_capture_target_materialized() {
        let (capture_metadata_retained, process_metadata_retained) =
            testing_target_without_process_lifetime()
                .expect("the native target-materialization seam runs");
        assert!(capture_metadata_retained);
        assert!(
            !process_metadata_retained,
            "capture identity survives without inventing process-post authority"
        );
    }

    #[test]
    fn activation_refuses_a_replacement_observed_after_the_attempt() {
        let (status, calls) = testing_input_activation_lifetime_loss()
            .expect("the retained-process activation seam runs");

        assert_eq!(status, ShimStatus::TargetLost);
        assert_eq!(
            calls,
            [2, 1],
            "the exact retained target is checked before and after one activation attempt"
        );
    }

    #[test]
    fn single_system_events_release_native_storage_and_preserve_the_post_threshold() {
        let rows = [
            (TEST_INPUT_SINGLE_CONFIGURE_EXCEPTION, 0, 0),
            (TEST_INPUT_SINGLE_POST_EXCEPTION, 1, 1),
        ];
        for (scenario, expected_posts, expected_posted) in rows {
            let (delivery, [configurations, posts, releases, posted]) =
                testing_input_single_event_failure(scenario)
                    .expect("native single-event failure seam runs");

            assert_eq!(delivery, ShimStatus::NativeException);
            assert_eq!(configurations, 1);
            assert_eq!(posts, expected_posts);
            assert_eq!(releases, 1, "the owned event is released on every exit");
            assert_eq!(
                posted, expected_posted,
                "only entry into the void post crosses the possible-effect threshold"
            );
        }
    }

    #[test]
    fn prepared_system_posts_enforce_the_final_fence_and_effect_threshold() {
        let rows = [
            (TEST_PREPARED_INPUT_SUCCESS, ShimStatus::Ok, true, 1, 1),
            (
                TEST_PREPARED_INPUT_CANCELLED,
                ShimStatus::TimedOut,
                false,
                0,
                0,
            ),
            (
                TEST_PREPARED_INPUT_DEADLINE,
                ShimStatus::TimedOut,
                false,
                0,
                0,
            ),
            (
                TEST_PREPARED_INPUT_POST_EXCEPTION,
                ShimStatus::NativeException,
                true,
                1,
                1,
            ),
        ];
        for (scenario, expected_status, expected_effect, expected_posts, expected_next) in rows {
            let (delivery, effect, posts, next_index) =
                testing_prepared_input_gate(scenario).expect("prepared-input seam runs");

            assert_eq!(delivery, expected_status, "scenario {scenario}");
            assert_eq!(effect, expected_effect, "scenario {scenario}");
            assert_eq!(posts, expected_posts, "scenario {scenario}");
            assert_eq!(next_index, expected_next, "scenario {scenario}");
        }
    }

    #[test]
    fn text_posts_nothing_when_the_release_event_cannot_be_allocated() {
        let (delivery, [allocations, configurations, posts, releases, posted]) =
            testing_input_text_failure(TEST_INPUT_TEXT_SECOND_ALLOCATION_FAILURE)
                .expect("native text allocation-failure seam runs");

        assert_eq!(delivery, ShimStatus::PlatformFailure);
        assert_eq!(
            allocations, 2,
            "the forced failure is the second allocation"
        );
        assert_eq!(configurations, 0, "no half-pair is configured alone");
        assert_eq!(posts, 0, "the key-down never reaches the post threshold");
        assert_eq!(releases, 1, "the first native event is released on failure");
        assert_eq!(posted, 0, "the caller observes no possible native effect");
    }

    #[test]
    fn text_configuration_exception_posts_nothing_and_releases_the_pair() {
        let (delivery, [allocations, configurations, posts, releases, posted]) =
            testing_input_text_failure(TEST_INPUT_TEXT_CONFIGURE_EXCEPTION)
                .expect("native text configuration-exception seam runs");

        assert_eq!(delivery, ShimStatus::NativeException);
        assert_eq!(allocations, 2);
        assert_eq!(configurations, 1);
        assert_eq!(posts, 0);
        assert_eq!(releases, 2, "both prepared events are released");
        assert_eq!(posted, 0, "configuration precedes the post threshold");
    }

    #[test]
    fn text_post_exception_preserves_possible_effect_and_releases_the_pair() {
        let (delivery, [allocations, configurations, posts, releases, posted]) =
            testing_input_text_failure(TEST_INPUT_TEXT_POST_EXCEPTION)
                .expect("native text post-exception seam runs");

        assert_eq!(delivery, ShimStatus::NativeException);
        assert_eq!(allocations, 2);
        assert_eq!(configurations, 2);
        assert_eq!(posts, 1);
        assert_eq!(releases, 2, "both prepared events are released");
        assert_eq!(
            posted, 1,
            "entry into the key-down post remains observable after its exception"
        );
    }

    #[test]
    fn missing_required_accessibility_data_is_unavailable_not_observed_unfocused() {
        let rows = [
            (0, ShimStatus::Ok),
            (1, ShimStatus::PermissionDenied),
            (2, ShimStatus::PlatformFailure),
            (3, ShimStatus::PlatformFailure),
            (4, ShimStatus::PlatformFailure),
            (5, ShimStatus::PlatformFailure),
            (6, ShimStatus::PlatformFailure),
            (7, ShimStatus::InvalidArgument),
        ];
        for (scenario, expected) in rows {
            assert_eq!(
                testing_required_ax_error_status(scenario),
                expected,
                "scenario {scenario}"
            );
        }
    }

    #[test]
    fn process_cancellation_callback_reads_only_atomic_state() {
        #[derive(Debug)]
        struct PanickingClock;

        impl Clock for PanickingClock {
            fn now(&self) -> MonotonicInstant {
                panic!("the native cancellation fence must not call the operation clock")
            }
        }

        let cancellation = CancellationToken::new();
        let operation = OperationContext::new()
            .with_clock(Arc::new(PanickingClock))
            .with_deadline(MonotonicInstant::ORIGIN)
            .with_cancellation(cancellation.clone());
        let fence = ProcessCancellationFence::new(&operation);
        let context = std::ptr::from_ref(&fence).cast_mut().cast::<c_void>();

        // SAFETY: `context` points to the live fence for this synchronous call.
        let observed = without_panic_output(|| unsafe { process_cancellation_callback(context) });
        assert_eq!(observed, ShimStatus::Ok.as_raw());
        assert!(!fence.cancellation_observed());

        cancellation.cancel();
        // SAFETY: `context` still points to the live fence for this synchronous call.
        let observed = unsafe { process_cancellation_callback(context) };
        assert_eq!(observed, ShimStatus::TimedOut.as_raw());
        assert!(fence.cancellation_observed());
    }

    #[test]
    fn process_operation_checkpoint_uses_the_caller_clock_and_contains_panics() {
        #[derive(Debug, Default)]
        struct ManualClock(AtomicU64);

        impl Clock for ManualClock {
            fn now(&self) -> MonotonicInstant {
                MonotonicInstant::from_origin(Duration::from_nanos(self.0.load(Ordering::SeqCst)))
            }
        }

        let clock = Arc::new(ManualClock::default());
        clock.0.store(2, Ordering::SeqCst);
        let operation = OperationContext::new()
            .with_clock(clock.clone())
            .with_deadline(MonotonicInstant::from_origin(Duration::from_nanos(5)));
        let checkpoint = ProcessOperationCheckpoint::new(&operation);
        let context = std::ptr::from_ref(&checkpoint).cast_mut().cast::<c_void>();
        let mut wait = u64::MAX;
        // SAFETY: both pointers remain valid for this synchronous call.
        let observed = unsafe { process_operation_checkpoint_callback(context, &raw mut wait) };
        assert_eq!(observed, ShimStatus::Ok.as_raw());
        assert_eq!(wait, 3);

        clock.0.store(5, Ordering::SeqCst);
        wait = u64::MAX;
        // SAFETY: both pointers remain valid for this synchronous call.
        let observed = unsafe { process_operation_checkpoint_callback(context, &raw mut wait) };
        assert_eq!(observed, ShimStatus::TimedOut.as_raw());
        assert_eq!(wait, 0);

        #[derive(Debug)]
        struct PanickingClock;
        impl Clock for PanickingClock {
            fn now(&self) -> MonotonicInstant {
                panic!("contained caller clock panic")
            }
        }
        let panicking = OperationContext::new()
            .with_clock(Arc::new(PanickingClock))
            .with_deadline(MonotonicInstant::ORIGIN);
        let checkpoint = ProcessOperationCheckpoint::new(&panicking);
        let context = std::ptr::from_ref(&checkpoint).cast_mut().cast::<c_void>();
        wait = u64::MAX;
        // SAFETY: both pointers remain valid for this synchronous call.
        let observed = without_panic_output(|| unsafe {
            process_operation_checkpoint_callback(context, &raw mut wait)
        });
        assert_eq!(observed, ShimStatus::PlatformFailure.as_raw());
        assert_eq!(wait, 0);
    }

    #[test]
    fn process_post_revalidates_authority_and_authorization_before_posting() {
        let observed = testing_process_post(0).expect("native process-post seam runs");

        assert_eq!(observed.delivery, ShimStatus::Ok);
        assert_eq!(observed.invoked_native_units, 1);
        assert_eq!(observed.target_match_count, 1);
        assert_eq!(observed.focus, ProcessFocusObservation::NotApplicable);
        assert_eq!(
            observed.calls,
            [1, 2, 2, 0, 1, 1, 1, 2, 1],
            "cheap direct authorization and retained lifetime checks run before construction, while exact retained-window authority and geometry run once at the final bounded post gate"
        );
    }

    #[test]
    fn process_release_uses_lifetime_and_authorization_without_window_visibility() {
        let observed = testing_process_post(15).expect("native process-post seam runs");

        assert_eq!(observed.delivery, ShimStatus::Ok);
        assert_eq!(observed.invoked_native_units, 1);
        assert_eq!(observed.target_match_count, 0);
        assert_eq!(observed.focus, ProcessFocusObservation::NotApplicable);
        assert_eq!(
            observed.calls,
            [0, 2, 2, 0, 1, 1, 1, 2, 1],
            "release skips current window admission and focus but rechecks authorization and process lifetime after construction before posting"
        );
    }

    #[test]
    fn process_post_fails_closed_before_native_effect() {
        let rows = [
            (
                1,
                ShimStatus::PermissionDenied,
                0,
                [0, 1, 0, 0, 0, 0, 0, 1, 0],
            ),
            (2, ShimStatus::TargetLost, 0, [1, 1, 1, 0, 1, 0, 1, 2, 0]),
            (3, ShimStatus::Unsupported, 0, [1, 1, 1, 0, 1, 0, 1, 2, 0]),
            (
                4,
                ShimStatus::InvalidArgument,
                0,
                [0, 0, 0, 0, 0, 0, 0, 0, 0],
            ),
            (6, ShimStatus::Unsupported, 0, [0, 0, 0, 0, 0, 0, 0, 0, 0]),
            (
                8,
                ShimStatus::GeometryChanged,
                1,
                [1, 1, 1, 0, 1, 0, 1, 2, 0],
            ),
            (10, ShimStatus::TargetLost, 0, [0, 1, 1, 0, 0, 0, 0, 1, 0]),
            (11, ShimStatus::TimedOut, 0, [0, 0, 0, 0, 0, 0, 0, 1, 0]),
            (
                12,
                ShimStatus::PlatformFailure,
                0,
                [0, 1, 1, 0, 1, 0, 0, 1, 0],
            ),
            (14, ShimStatus::TimedOut, 0, [0, 1, 1, 0, 1, 0, 1, 2, 0]),
        ];
        for (scenario, delivery, target_count, calls) in rows {
            let observed = testing_process_post(scenario).expect("native process-post seam runs");
            assert_eq!(observed.delivery, delivery, "scenario {scenario}");
            assert_eq!(observed.invoked_native_units, 0, "scenario {scenario}");
            assert_eq!(
                observed.target_match_count, target_count,
                "scenario {scenario}"
            );
            assert_eq!(observed.calls, calls, "scenario {scenario}");
        }
    }

    #[test]
    fn process_post_checks_caller_deadline_before_final_authority() {
        let observed = testing_process_post(27).expect("native process-post seam runs");

        assert_eq!(observed.delivery, ShimStatus::TimedOut);
        assert_eq!(observed.invoked_native_units, 0);
        assert!(!observed.native_effect_may_have_occurred);
        assert_eq!(observed.target_match_count, 0);
        assert_eq!(
            observed.calls,
            [0, 1, 1, 0, 1, 0, 1, 2, 0],
            "the caller-clock checkpoint expires before any final mutable observation"
        );
    }

    #[test]
    fn process_post_bounds_late_native_authority_without_rechecking_the_caller_clock() {
        let observed = testing_process_post(31).expect("native process-post seam runs");

        assert_eq!(observed.delivery, ShimStatus::TimedOut);
        assert_eq!(observed.invoked_native_units, 0);
        assert!(!observed.native_effect_may_have_occurred);
        assert_eq!(observed.target_match_count, 1);
        assert_eq!(
            observed.calls,
            [1, 2, 1, 0, 1, 0, 1, 2, 0],
            "adapter monotonic time rejects a native authority call that returns outside its supplied slice"
        );
    }

    #[test]
    fn process_post_rechecks_native_budget_after_final_process_lifetime() {
        let observed = testing_process_post(32).expect("native process-post seam runs");

        assert_eq!(observed.delivery, ShimStatus::TimedOut);
        assert_eq!(observed.invoked_native_units, 0);
        assert!(!observed.native_effect_may_have_occurred);
        assert_eq!(observed.target_match_count, 1);
        assert_eq!(
            observed.calls,
            [1, 2, 2, 0, 1, 0, 1, 2, 0],
            "the final adapter-clock check catches a deadline crossed during process-lifetime validation"
        );
    }

    #[test]
    fn process_post_reads_atomic_cancellation_after_final_process_lifetime() {
        let observed = testing_process_post(30).expect("native process-post seam runs");

        assert_eq!(observed.delivery, ShimStatus::TimedOut);
        assert_eq!(observed.invoked_native_units, 0);
        assert!(!observed.native_effect_may_have_occurred);
        assert_eq!(observed.target_match_count, 1);
        assert_eq!(
            observed.calls,
            [1, 2, 2, 0, 1, 0, 1, 2, 1],
            "cancellation raised during lifetime validation is caught immediately before posting"
        );
    }

    #[test]
    fn process_post_rechecks_lifetime_after_the_operation_checkpoint() {
        let observed = testing_process_post(29).expect("native process-post seam runs");

        assert_eq!(observed.delivery, ShimStatus::TargetLost);
        assert_eq!(observed.invoked_native_units, 0);
        assert!(!observed.native_effect_may_have_occurred);
        assert_eq!(observed.target_match_count, 1);
        assert_eq!(
            observed.calls,
            [1, 2, 2, 0, 1, 0, 1, 2, 0],
            "lifetime invalidated at the checkpoint is refused by the last identity read"
        );
    }

    #[test]
    fn process_post_refuses_authority_changes_after_event_preparation() {
        let rows = [
            (16, ShimStatus::TargetLost, 0, [1, 1, 1, 0, 1, 0, 1, 2, 0]),
            (
                17,
                ShimStatus::PermissionDenied,
                1,
                [1, 2, 1, 0, 1, 0, 1, 2, 0],
            ),
            (18, ShimStatus::TargetLost, 1, [1, 2, 2, 0, 1, 0, 1, 2, 0]),
            (
                19,
                ShimStatus::GeometryChanged,
                1,
                [1, 1, 1, 0, 1, 0, 1, 2, 0],
            ),
        ];
        for (scenario, delivery, target_count, calls) in rows {
            let observed = testing_process_post(scenario).expect("native process-post seam runs");
            assert_eq!(observed.delivery, delivery, "scenario {scenario}");
            assert_eq!(observed.invoked_native_units, 0, "scenario {scenario}");
            assert_eq!(
                observed.target_match_count, target_count,
                "scenario {scenario}"
            );
            assert_eq!(observed.calls, calls, "scenario {scenario}");
        }
    }

    #[test]
    fn process_post_accepts_source_geometry_restored_before_the_final_gate() {
        let observed = testing_process_post(25).expect("native process-post seam runs");

        assert_eq!(observed.delivery, ShimStatus::Ok);
        assert_eq!(observed.invoked_native_units, 1);
        assert_eq!(observed.target_match_count, 1);
        assert!(observed.native_effect_may_have_occurred);
        assert_eq!(
            observed.calls,
            [1, 2, 2, 0, 1, 1, 1, 2, 1],
            "only the source-matching final observation is authoritative; an earlier transient move adds no inventory read"
        );
    }

    #[test]
    fn process_post_compares_fractional_windows_in_capture_geometry() {
        let observed = testing_process_post(26).expect("native process-post seam runs");

        assert_eq!(
            observed.delivery,
            ShimStatus::Ok,
            "the exact 641-pixel source transform must match its unchanged 320.4-point native window"
        );
        assert_eq!(observed.invoked_native_units, 1);
        assert_eq!(observed.target_match_count, 1);
        assert_eq!(observed.calls, [1, 2, 2, 0, 1, 1, 1, 2, 1]);
    }

    #[test]
    fn process_post_rejects_every_invalid_native_request_before_effect() {
        let scenarios = [
            "null request",
            "request prefix",
            "report prefix",
            "null target",
            "target magic",
            "target kind",
            "target native id",
            "target process",
            "target filter",
            "target owner",
            "target lifetime",
            "target launch",
            "null source",
            "source magic",
            "source value",
            "interruption context",
            "interruption callback",
            "timeout",
            "flags",
            "geometry policy",
            "reserved bytes",
            "geometry bounds",
            "pointer coordinate",
            "pointer action",
            "pointer button",
            "pointer click count",
            "zero scroll",
            "scroll range",
            "key code",
            "key geometry",
            "text pointer",
            "text count",
            "text UTF-16",
            "event kind",
            "post purpose",
            "null output",
            "scroll coordinate",
            "focus requirement",
            "release focus requirement",
            "cancellation context",
            "cancellation callback",
        ];
        for (scenario, description) in scenarios.into_iter().enumerate() {
            let scenario = u32::try_from(scenario).expect("validation scenario index fits u32");
            let (delivery, target_count, invoked_units, native_effect) =
                testing_validate_process_post(scenario)
                    .expect("native request-validation seam runs");
            let expected = if scenario == 10 {
                ShimStatus::Unsupported
            } else {
                ShimStatus::InvalidArgument
            };
            assert_eq!(delivery, expected, "scenario {scenario}: {description}");
            if !matches!(scenario, 2 | 35) {
                assert_eq!(target_count, 0, "scenario {scenario}: {description}");
                assert_eq!(invoked_units, 0, "scenario {scenario}: {description}");
                assert_eq!(native_effect, 0, "scenario {scenario}: {description}");
            }
        }
    }

    #[test]
    fn process_post_contains_native_exceptions_and_releases_prepared_events() {
        let observed = testing_process_post(5).expect("native process-post seam runs");

        assert_eq!(observed.delivery, ShimStatus::NativeException);
        assert_eq!(observed.invoked_native_units, 0);
        assert!(!observed.native_effect_may_have_occurred);
        assert_eq!(observed.calls, [0, 1, 1, 0, 1, 0, 1, 1, 0]);
    }

    #[test]
    fn process_post_separates_possible_effect_from_returned_call_count() {
        let observed = testing_process_post(20).expect("native process-post seam runs");

        assert_eq!(observed.delivery, ShimStatus::NativeException);
        assert_eq!(observed.invoked_native_units, 0);
        assert!(observed.native_effect_may_have_occurred);
        assert_eq!(observed.calls, [1, 2, 2, 0, 1, 1, 1, 2, 1]);
    }

    #[test]
    fn process_release_exception_after_the_final_post_is_contained() {
        let observed = testing_process_post(33).expect("native process-post seam runs");

        assert_eq!(observed.delivery, ShimStatus::NativeException);
        assert_eq!(observed.invoked_native_units, 1);
        assert!(observed.native_effect_may_have_occurred);
        assert_eq!(
            observed.calls,
            [1, 2, 2, 0, 1, 1, 1, 2, 1],
            "the final post returned before the release exception, and the outer native boundary contained it"
        );
    }

    #[test]
    fn process_post_stops_text_after_authority_or_authorization_changes() {
        let revoked = testing_process_post(7).expect("native process-post seam runs");
        assert_eq!(revoked.delivery, ShimStatus::PermissionDenied);
        assert_eq!(revoked.invoked_native_units, 1);
        assert_eq!(revoked.calls, [1, 3, 2, 0, 1, 1, 1, 3, 1]);

        let lost = testing_process_post(9).expect("native process-post seam runs");
        assert_eq!(lost.delivery, ShimStatus::TargetLost);
        assert_eq!(lost.invoked_native_units, 1);
        assert_eq!(lost.calls, [2, 3, 3, 0, 2, 1, 2, 4, 1]);

        let interrupted = testing_process_post(13).expect("native process-post seam runs");
        assert_eq!(interrupted.delivery, ShimStatus::TimedOut);
        assert_eq!(interrupted.invoked_native_units, 1);
        assert_eq!(interrupted.calls, [1, 2, 2, 0, 1, 1, 1, 3, 1]);
    }

    /// A caller-selected focus predicate is repeated after event preparation and
    /// after the potentially blocking retained-window authority query. Its
    /// combined observation returns the later exact-window geometry, so neither
    /// focus loss during authority nor a target/geometry change during focus can
    /// authorize a post.
    #[test]
    fn process_post_requires_caller_selected_focus_at_the_final_gate() {
        let focused = testing_process_post(24).expect("native process-post seam runs");
        assert_eq!(focused.delivery, ShimStatus::Ok);
        assert_eq!(focused.invoked_native_units, 1);
        assert_eq!(focused.focus, ProcessFocusObservation::Passed);
        assert_eq!(
            focused.calls,
            [1, 2, 2, 2, 1, 1, 1, 2, 1],
            "a focused target repeats its combined predicate after the final retained-window authority check"
        );

        let lost_during_authority =
            testing_process_post(34).expect("native process-post seam runs");
        assert_eq!(lost_during_authority.delivery, ShimStatus::FocusRequired);
        assert_eq!(lost_during_authority.invoked_native_units, 0);
        assert!(!lost_during_authority.native_effect_may_have_occurred);
        assert_eq!(lost_during_authority.target_match_count, 1);
        assert_eq!(
            lost_during_authority.focus,
            ProcessFocusObservation::Refused
        );
        assert_eq!(
            lost_during_authority.calls,
            [1, 1, 1, 2, 1, 0, 1, 2, 0],
            "the retained-window authority query completes before the final focus predicate"
        );

        let replaced_during_focus =
            testing_process_post(28).expect("native process-post seam runs");
        assert_eq!(replaced_during_focus.delivery, ShimStatus::TargetLost);
        assert_eq!(replaced_during_focus.invoked_native_units, 0);
        assert!(!replaced_during_focus.native_effect_may_have_occurred);
        assert_eq!(replaced_during_focus.target_match_count, 0);
        assert_eq!(
            replaced_during_focus.focus,
            ProcessFocusObservation::Unavailable
        );
        assert_eq!(
            replaced_during_focus.calls,
            [1, 1, 1, 2, 1, 0, 1, 2, 0],
            "the combined focus observation rejects an exact-window replacement"
        );

        let moved_during_focus = testing_process_post(35).expect("native process-post seam runs");
        assert_eq!(moved_during_focus.delivery, ShimStatus::GeometryChanged);
        assert_eq!(moved_during_focus.invoked_native_units, 0);
        assert!(!moved_during_focus.native_effect_may_have_occurred);
        assert_eq!(moved_during_focus.target_match_count, 1);
        assert_eq!(moved_during_focus.focus, ProcessFocusObservation::Passed);
        assert_eq!(
            moved_during_focus.calls,
            [1, 1, 1, 2, 1, 0, 1, 2, 0],
            "RequireUnchanged uses geometry returned by the combined final focus observation"
        );

        let moved_with_authority_only =
            testing_process_post(36).expect("native process-post seam runs");
        assert_eq!(moved_with_authority_only.delivery, ShimStatus::Ok);
        assert_eq!(moved_with_authority_only.invoked_native_units, 1);
        assert!(moved_with_authority_only.native_effect_may_have_occurred);
        assert_eq!(moved_with_authority_only.target_match_count, 1);
        assert_eq!(
            moved_with_authority_only.focus,
            ProcessFocusObservation::Passed
        );
        assert_eq!(
            moved_with_authority_only.calls,
            [1, 2, 2, 2, 1, 1, 1, 2, 1],
            "geometry movement does not become focus loss when RequireUnchanged was not selected"
        );

        let unfocused = testing_process_post(21).expect("native process-post seam runs");
        assert_eq!(unfocused.delivery, ShimStatus::FocusRequired);
        assert_eq!(unfocused.invoked_native_units, 0);
        assert!(!unfocused.native_effect_may_have_occurred);
        assert_eq!(unfocused.focus, ProcessFocusObservation::Refused);
        assert_eq!(
            unfocused.calls,
            [0, 1, 1, 1, 0, 0, 0, 1, 0],
            "an unfocused target refuses before any event is constructed without paying for retained-window inventory"
        );

        let lost_late = testing_process_post(22).expect("native process-post seam runs");
        assert_eq!(lost_late.delivery, ShimStatus::FocusRequired);
        assert_eq!(lost_late.invoked_native_units, 0);
        assert!(!lost_late.native_effect_may_have_occurred);
        assert_eq!(lost_late.target_match_count, 1);
        assert_eq!(lost_late.focus, ProcessFocusObservation::Refused);
        assert_eq!(
            lost_late.calls,
            [1, 1, 1, 2, 1, 0, 1, 2, 0],
            "focus lost after event preparation is refused after fresh retained-window authority"
        );

        let unobservable = testing_process_post(23).expect("native process-post seam runs");
        assert_eq!(unobservable.delivery, ShimStatus::PermissionDenied);
        assert_eq!(unobservable.invoked_native_units, 0);
        assert!(!unobservable.native_effect_may_have_occurred);
        assert_eq!(unobservable.focus, ProcessFocusObservation::Unavailable);
        assert_eq!(
            unobservable.calls,
            [0, 1, 1, 1, 0, 0, 0, 1, 0],
            "an unobservable focus predicate fails closed rather than posting or paying for retained-window inventory"
        );
    }

    #[test]
    fn process_authority_preserves_scope_with_additional_same_process_windows() {
        let expected = [
            (0, ShimStatus::Ok, 1),
            (1, ShimStatus::TargetLost, 0),
            (2, ShimStatus::TargetLost, 0),
            (3, ShimStatus::TargetLost, 0),
            (4, ShimStatus::TargetLost, 0),
            (5, ShimStatus::Ok, 1),
            (6, ShimStatus::Unsupported, 0),
            (7, ShimStatus::TargetLost, 0),
            (8, ShimStatus::TargetLost, 0),
            (9, ShimStatus::Ok, 1),
            (10, ShimStatus::TargetLost, 0),
        ];

        for (scenario, status, retained_match_count) in expected {
            assert_eq!(
                testing_process_authority_rules(scenario).expect("native authority rule seam runs"),
                (status, retained_match_count),
                "scenario {scenario}"
            );
        }
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
    fn frame_timestamp_conversion_rejects_every_out_of_range_float() {
        assert_eq!(testing_seconds_to_nanos(1.5), 1_500_000_000);
        assert_eq!(testing_seconds_to_nanos(0.0), 0);
        assert_eq!(testing_seconds_to_nanos(-1.0), 0);
        assert_eq!(testing_seconds_to_nanos(f64::NAN), 0);
        assert_eq!(testing_seconds_to_nanos(f64::INFINITY), 0);
        assert_eq!(
            testing_seconds_to_nanos(2_f64.powi(64) / 1e9),
            0,
            "the first value whose integer cast could exceed u64 is rejected"
        );
    }

    #[test]
    fn a_stream_stop_error_exception_is_contained_and_drains_admission() {
        let (terminal, calls, fence) =
            testing_stop_callback_exception().expect("the native stream-stop exception seam runs");

        assert_eq!(terminal, ShimStatus::NativeException);
        assert_eq!(calls, 1);
        assert_eq!(fence, ShimStatus::Ok);
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
    fn every_session_synchronization_initialization_failure_unwinds_only_owned_objects() {
        #[cfg(feature = "sck-suspension-diagnostics")]
        const STAGES: u32 = 12;
        #[cfg(not(feature = "sck-suspension-diagnostics"))]
        const STAGES: u32 = 10;
        for fail_at in 1..=STAGES {
            let observed =
                testing_session_sync_init(fail_at).expect("the native initialization seam runs");
            assert_eq!(
                observed,
                SessionSyncInit {
                    attempts: fail_at,
                    initialized: fail_at - 1,
                    destroyed: fail_at - 1,
                    success: false,
                },
                "failure at pthread initialization stage {fail_at}"
            );
        }

        assert_eq!(
            testing_session_sync_init(0).expect("all synchronization objects initialize"),
            SessionSyncInit {
                attempts: STAGES,
                initialized: STAGES,
                destroyed: STAGES,
                success: true,
            }
        );

        assert_eq!(
            testing_session_sync_init(STAGES + 1),
            Err(ShimStatus::InvalidArgument)
        );
    }

    #[test]
    fn asynchronous_resource_allocation_failures_are_typed_before_submission() {
        assert_eq!(
            testing_resource_allocation_failures()
                .expect("the native resource-allocation seam runs"),
            [ShimStatus::PlatformFailure; 2]
        );
    }

    #[test]
    fn fixture_launcher_allocation_failure_precedes_workspace_submission() {
        let observed =
            testing_fixture_application_launch(TEST_FIXTURE_SEMAPHORE_ALLOCATION_FAILURE)
                .expect("the native fixture-launch seam runs");

        assert_eq!(observed.launch_status, ShimStatus::PlatformFailure);
        assert_eq!(observed.submission_calls, 0);
        assert_eq!(observed.graceful_termination_calls, 0);
        assert_eq!(observed.force_termination_calls, 0);
        assert!(!observed.terminated);
        assert_eq!(observed.live_after_release, observed.live_during_handle);
    }

    #[test]
    fn fixture_launcher_contains_completion_and_reaps_every_unpublished_application() {
        for (scenario, expected_status) in [
            (
                TEST_FIXTURE_COMPLETION_EXCEPTION,
                ShimStatus::NativeException,
            ),
            (TEST_FIXTURE_LATE_COMPLETION, ShimStatus::TimedOut),
            (TEST_FIXTURE_VALIDATION_FAILURE, ShimStatus::PlatformFailure),
            (
                TEST_FIXTURE_HANDLE_ALLOCATION_FAILURE,
                ShimStatus::PlatformFailure,
            ),
        ] {
            let observed = testing_fixture_application_launch(scenario)
                .expect("the native fixture-launch seam joins its completion");

            assert_eq!(
                observed.launch_status, expected_status,
                "scenario {scenario}"
            );
            assert_eq!(observed.submission_calls, 1, "scenario {scenario}");
            assert_eq!(
                observed.graceful_termination_calls, 1,
                "scenario {scenario}"
            );
            assert_eq!(observed.force_termination_calls, 1, "scenario {scenario}");
            assert!(observed.terminated, "scenario {scenario}");
            assert_eq!(
                observed.live_after_release, observed.live_during_handle,
                "scenario {scenario}"
            );
        }
    }

    #[test]
    fn fixture_launcher_transfers_unverified_force_exit_to_a_retained_reaper() {
        let observed = testing_fixture_application_launch(TEST_FIXTURE_REAPER_HANDOFF)
            .expect("the native fixture-launch seam joins its retained reaper");

        assert_eq!(observed.launch_status, ShimStatus::PlatformFailure);
        assert_eq!(observed.submission_calls, 1);
        assert_eq!(observed.graceful_termination_calls, 2);
        assert_eq!(observed.force_termination_calls, 2);
        assert!(observed.terminated);
        assert_eq!(observed.live_after_release, observed.live_during_handle);
    }

    #[test]
    fn fixture_launcher_release_returns_the_native_live_count_to_baseline() {
        let observed = testing_fixture_application_launch(TEST_FIXTURE_SUCCESSFUL_RELEASE)
            .expect("the native fixture-launch seam releases its successful handle");

        assert_eq!(observed.launch_status, ShimStatus::Ok);
        assert_eq!(observed.submission_calls, 1);
        assert_eq!(observed.graceful_termination_calls, 1);
        assert_eq!(observed.force_termination_calls, 1);
        assert!(observed.terminated);
        assert_eq!(observed.live_during_handle, observed.live_after_release + 1);
    }

    #[test]
    fn fixture_launcher_release_hands_unverified_force_exit_to_the_reaper() {
        let observed = testing_fixture_application_launch(TEST_FIXTURE_RELEASE_REAPER_HANDOFF)
            .expect("fixture handle release joins its retained reaper");

        assert_eq!(observed.launch_status, ShimStatus::Ok);
        assert_eq!(observed.submission_calls, 1);
        assert_eq!(observed.graceful_termination_calls, 2);
        assert_eq!(observed.force_termination_calls, 2);
        assert!(observed.terminated);
        assert_eq!(observed.live_during_handle, observed.live_after_release + 1);
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
    fn a_panicking_payload_destructor_is_contained_before_native_return() {
        struct PanickingPayload;

        impl Drop for PanickingPayload {
            fn drop(&mut self) {
                panic!("panic payload destructor");
            }
        }

        let outcome: Result<(), ()> =
            without_panic_output(|| catch_panic(|| panic::panic_any(PanickingPayload)));

        assert_eq!(outcome, Err(()));
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
