//! WGC session ownership, callback fencing, detachment, and teardown.

use std::fmt;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, TryLockError, Weak};
use std::thread;
use std::time::Duration;

use mado_pilot_capture::{
    CaptureFault, CaptureSession, Continuity, Frame, FrameRequest, Lifecycle, OverflowPolicy,
    QueuePolicy, SessionDescription, StoragePublication, StreamState,
};
use mado_pilot_core::{
    Clock, MonotonicInstant, Operation, OperationContext, PixelExtent, Result, StreamId,
    SystemClock, TargetId, TargetPlacement,
};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Graphics::Direct3D11::{D3D11_TEXTURE2D_DESC, ID3D11Texture2D};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;
use windows::core::{IInspectable, Interface};

use crate::availability::{create_free_threaded_frame_pool, ensure_winrt_apartment};
use crate::discovery::{NativeKey, TargetMetadata, current_placement};
use crate::storage::{
    DETACHED_TEXTURE_BUDGET, DeviceDomain, StorageFailureSink, TexturePool, WindowsFrameStorage,
    descriptor_from_native, native_fault,
};

const WGC_PRODUCER_POOL_SIZE: i32 = 2;
const CALLBACK_POLL_INTERVAL: Duration = Duration::from_millis(2);

pub(crate) struct NativeSession {
    description: SessionDescription,
    core: Arc<SessionCore>,
    runtime: Mutex<RuntimeState>,
    close_gate: Mutex<()>,
}

struct RuntimeState {
    resources: Option<NativeResources>,
    handlers_removed: bool,
    close_task: Option<Receiver<Option<CaptureFault>>>,
    close_result: Option<Option<CaptureFault>>,
    close_fault_reported: bool,
}

struct NativeResources {
    item: GraphicsCaptureItem,
    frame_pool: Direct3D11CaptureFramePool,
    capture: GraphicsCaptureSession,
    frame_token: i64,
    closed_token: i64,
}

struct SessionCore {
    target_kind: mado_pilot_core::TargetKind,
    key: NativeKey,
    state: StreamState,
    domain: Arc<DeviceDomain>,
    textures: Arc<TexturePool>,
    callbacks: Arc<CallbackControl>,
    native_ended: AtomicBool,
    failure: OnceLock<Weak<dyn StorageFailureSink>>,
    transition: Mutex<TransitionState>,
}

#[derive(Debug, Clone, Copy)]
struct TransitionState {
    extent: PixelExtent,
    placement: TargetPlacement,
    pending_discontinuity: bool,
    pending_geometry_change: bool,
    movement_not_before: Option<i64>,
    clock_anchor: Option<(i64, MonotonicInstant)>,
    published: bool,
}

impl NativeSession {
    pub(crate) fn open(
        target: TargetId,
        stream: StreamId,
        kind: mado_pilot_core::TargetKind,
        key: NativeKey,
        metadata: TargetMetadata,
        item: GraphicsCaptureItem,
    ) -> Result<Arc<Self>> {
        let domain = DeviceDomain::create()?;
        let textures = TexturePool::new(Arc::clone(&domain));
        let callbacks = Arc::new(CallbackControl::default());
        let core = Arc::new(SessionCore {
            target_kind: kind,
            key,
            state: StreamState::with_target_extent(stream),
            domain,
            textures,
            callbacks: Arc::clone(&callbacks),
            native_ended: AtomicBool::new(false),
            failure: OnceLock::new(),
            transition: Mutex::new(TransitionState {
                extent: metadata.extent,
                placement: metadata.placement,
                pending_discontinuity: false,
                pending_geometry_change: false,
                movement_not_before: None,
                clock_anchor: None,
                published: false,
            }),
        });
        let failure: Arc<dyn StorageFailureSink> = core.clone();
        core.failure
            .set(Arc::downgrade(&failure))
            .expect("failure sink initializes once");

        let size = native_size(metadata.extent)?;
        let winrt_device = core.domain.winrt_device()?;
        let frame_pool = create_free_threaded_frame_pool(
            &winrt_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            WGC_PRODUCER_POOL_SIZE,
            size,
        )
        .map_err(native_fault)?;
        let capture = frame_pool
            .CreateCaptureSession(&item)
            .map_err(native_fault)?;

        let frame_weak = Arc::downgrade(&core);
        let frame_callbacks = Arc::clone(&callbacks);
        let frame_handler =
            TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |sender, _| {
                let Some(_lease) = frame_callbacks.admit() else {
                    return Ok(());
                };
                let Some(core) = frame_weak.upgrade() else {
                    return Ok(());
                };
                if let Some(sender) = sender.as_ref() {
                    core.on_frame(sender);
                }
                Ok(())
            });
        let frame_token = frame_pool
            .FrameArrived(&frame_handler)
            .map_err(native_fault)?;

        let closed_weak = Arc::downgrade(&core);
        let closed_callbacks = Arc::clone(&callbacks);
        let closed_handler =
            TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
                let Some(_lease) = closed_callbacks.admit() else {
                    return Ok(());
                };
                if let Some(core) = closed_weak.upgrade() {
                    core.callbacks.stop_admission();
                    core.native_ended.store(true, Ordering::Release);
                    let fault = match core.target_kind {
                        mado_pilot_core::TargetKind::Window => CaptureFault::CaptureItemClosed,
                        mado_pilot_core::TargetKind::Display => CaptureFault::DisplayDisconnected,
                        _ => CaptureFault::TargetLost,
                    };
                    core.state.terminate(fault);
                }
                Ok(())
            });
        let closed_token = match item.Closed(&closed_handler) {
            Ok(token) => token,
            Err(error) => {
                let _remove = frame_pool.RemoveFrameArrived(frame_token);
                return Err(native_fault(error).into());
            }
        };

        let description = SessionDescription::new(
            target,
            stream,
            metadata.extent,
            mado_pilot_capture::PixelFormat::Bgra8,
            mado_pilot_capture::CoordinateSupport::with_target_placement(),
        )
        .with_queue(
            QueuePolicy::new(NonZeroU32::MIN, OverflowPolicy::Reject)
                .with_retained_storage(DETACHED_TEXTURE_BUDGET),
        );
        let session = Arc::new(Self {
            description,
            core,
            runtime: Mutex::new(RuntimeState {
                resources: Some(NativeResources {
                    item,
                    frame_pool,
                    capture,
                    frame_token,
                    closed_token,
                }),
                handlers_removed: false,
                close_task: None,
                close_result: None,
                close_fault_reported: false,
            }),
            close_gate: Mutex::new(()),
        });
        {
            let runtime = session.runtime();
            runtime
                .resources
                .as_ref()
                .expect("resources exist before start")
                .capture
                .StartCapture()
                .map_err(native_fault)?;
        }
        Ok(session)
    }

    fn runtime(&self) -> MutexGuard<'_, RuntimeState> {
        self.runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn remove_handlers(&self) {
        let handlers = {
            let mut runtime = self.runtime();
            if runtime.handlers_removed {
                None
            } else {
                runtime.handlers_removed = true;
                runtime.resources.as_ref().map(|resources| {
                    (
                        resources.item.clone(),
                        resources.frame_pool.clone(),
                        resources.frame_token,
                        resources.closed_token,
                    )
                })
            }
        };
        if let Some((item, frame_pool, frame_token, closed_token)) = handlers {
            let _frame = frame_pool.RemoveFrameArrived(frame_token);
            let _closed = item.RemoveClosed(closed_token);
        }
    }

    fn start_native_close(&self, drain_callbacks: bool) {
        let mut runtime = self.runtime();
        if runtime.close_task.is_some() || runtime.close_result.is_some() {
            return;
        }
        let Some(resources) = runtime.resources.take() else {
            runtime.close_result = Some(None);
            return;
        };
        let remove_handlers = !runtime.handlers_removed;
        runtime.handlers_removed = true;
        let (complete, result) = mpsc::channel();
        runtime.close_task = Some(result);
        let capture_already_ended = self.core.native_ended.load(Ordering::Acquire);
        let callbacks = Arc::clone(&self.core.callbacks);
        thread::spawn(move || {
            let apartment = ensure_winrt_apartment().map_err(|_| CaptureFault::UnsupportedOption);
            if apartment.is_ok() && remove_handlers {
                let _frame = resources
                    .frame_pool
                    .RemoveFrameArrived(resources.frame_token);
                let _closed = resources.item.RemoveClosed(resources.closed_token);
            }
            if drain_callbacks {
                callbacks.drain_uninterruptible();
            }
            let fault = apartment
                .and_then(|()| close_native_resources(resources, capture_already_ended))
                .err();
            let _sent = complete.send(fault);
        });
    }

    fn close_native(&self, operation: &OperationContext) -> Result<()> {
        self.start_native_close(false);
        let mut attempt = Operation::admit(operation)?;
        loop {
            let result = {
                let mut runtime = self.runtime();
                if let Some(result) = runtime.close_result {
                    Some(result)
                } else if let Some(task) = runtime.close_task.as_ref() {
                    match task.try_recv() {
                        Ok(result) => {
                            runtime.close_task = None;
                            runtime.close_result = Some(result);
                            Some(result)
                        }
                        Err(TryRecvError::Empty) => None,
                        Err(TryRecvError::Disconnected) => {
                            runtime.close_task = None;
                            let result = Some(CaptureFault::SourceInvalid);
                            runtime.close_result = Some(result);
                            Some(result)
                        }
                    }
                } else {
                    runtime.close_result = Some(None);
                    Some(None)
                }
            };
            if let Some(result) = result {
                attempt.commit(())?;
                if let Some(fault) = result {
                    let mut runtime = self.runtime();
                    if !runtime.close_fault_reported {
                        runtime.close_fault_reported = true;
                        return Err(fault.into());
                    }
                }
                return Ok(());
            }
            thread::sleep(CALLBACK_POLL_INTERVAL);
            attempt.checkpoint()?;
        }
    }
}

fn close_native_resources(
    resources: NativeResources,
    capture_already_ended: bool,
) -> std::result::Result<(), CaptureFault> {
    ensure_winrt_apartment().map_err(|_| CaptureFault::UnsupportedOption)?;
    let NativeResources {
        item,
        frame_pool,
        capture,
        frame_token: _,
        closed_token: _,
    } = resources;
    // Item.Closed is the one state proving WGC already ended the capture.
    // Ordinary close and unrelated stream terminal faults still invoke Close.
    let capture_result = if capture_already_ended {
        Ok(())
    } else {
        capture.Close().map_err(native_fault)
    };
    let pool_result = frame_pool.Close().map_err(native_fault);
    let close_fault = capture_result.err().or_else(|| pool_result.err());
    drop(frame_pool);
    drop(capture);
    drop(item);
    close_fault.map_or(Ok(()), Err)
}

impl fmt::Debug for NativeSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeSession")
            .field("stream", &self.description.stream())
            .field("lifecycle", &self.core.state.lifecycle())
            .field("target_kind", &self.core.target_kind)
            .finish()
    }
}

impl CaptureSession for NativeSession {
    fn description(&self) -> SessionDescription {
        self.description.clone()
    }

    fn frame(&self, request: &FrameRequest, operation: &OperationContext) -> Result<Frame> {
        if !self.core.key.is_present() {
            self.core.callbacks.stop_admission();
            let fault = match self.core.target_kind {
                mado_pilot_core::TargetKind::Window => CaptureFault::CaptureItemClosed,
                mado_pilot_core::TargetKind::Display => CaptureFault::DisplayDisconnected,
                _ => CaptureFault::TargetLost,
            };
            self.core.state.terminate(fault);
        }
        self.core.state.frame(request, operation)
    }

    fn close(&self, operation: &OperationContext) -> Result<()> {
        ensure_winrt_apartment()?;
        self.core.state.begin_close();
        self.core.callbacks.stop_admission();
        let _close = lock_with_operation(&self.close_gate, operation)?;
        self.remove_handlers();
        self.core.callbacks.drain(operation)?;
        self.close_native(operation)?;
        self.core.state.drain(operation)
    }

    fn lifecycle(&self) -> Lifecycle {
        self.core.state.lifecycle()
    }
}

impl Drop for NativeSession {
    fn drop(&mut self) {
        self.core.callbacks.stop_admission();
        self.start_native_close(true);
    }
}

impl SessionCore {
    fn on_frame(&self, sender: &Direct3D11CaptureFramePool) {
        let result = self.process_frame(sender);
        if let Err(fault) = result {
            if fault == CaptureFault::SessionClosed && self.state.lifecycle() != Lifecycle::Open {
                return;
            }
            self.fail_native(fault);
        }
    }

    fn fail_native(&self, fault: CaptureFault) {
        let fault = normalize_native_fault(fault, self.target_kind, self.key.is_present());
        if fault == CaptureFault::ExplicitlyStopped {
            self.native_ended.store(true, Ordering::Release);
        }
        self.callbacks.stop_admission();
        self.state.terminate(fault);
    }

    fn process_frame(
        &self,
        sender: &Direct3D11CaptureFramePool,
    ) -> std::result::Result<(), CaptureFault> {
        let mut transition = match self.transition.try_lock() {
            Ok(transition) => transition,
            Err(TryLockError::WouldBlock) => return Ok(()),
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };
        let frame = sender.TryGetNextFrame().map_err(native_fault)?;
        let frame = WgcFrameGuard(Some(frame));
        let content_size = frame.frame().ContentSize().map_err(native_fault)?;
        let native_time = frame
            .frame()
            .SystemRelativeTime()
            .map_err(native_fault)?
            .Duration;
        let extent = positive_extent(content_size)?;

        if extent != transition.extent {
            // The transition frame and its producer surface are released before
            // pool recreation. The first frame from the recreated pool receives
            // the discontinuity.
            drop(frame);
            let device = self.domain.winrt_device().map_err(|_| {
                self.domain
                    .device_fault()
                    .unwrap_or(CaptureFault::SourceInvalid)
            })?;
            sender
                .Recreate(
                    &device,
                    DirectXPixelFormat::B8G8R8A8UIntNormalized,
                    WGC_PRODUCER_POOL_SIZE,
                    content_size,
                )
                .map_err(native_fault)?;
            let _retired = self.textures.try_retire_for_resize();
            transition.extent = extent;
            transition.pending_discontinuity = transition.published;
            transition.movement_not_before = None;
            return Ok(());
        }

        let placement = current_placement(self.key, extent).ok_or(CaptureFault::TargetLost)?;
        if transition.published && placement != transition.placement {
            transition.placement = placement;
            transition.pending_geometry_change = true;
            transition.movement_not_before =
                Some(native_monotonic_now().unwrap_or_else(|| native_time.saturating_add(1)));
            let _drop = self.state.try_record_drop();
            return Ok(());
        }
        if let Some(not_before) = transition.movement_not_before {
            if native_time < not_before {
                let _drop = self.state.try_record_drop();
                return Ok(());
            }
            transition.movement_not_before = None;
        }

        let surface = frame.frame().Surface().map_err(native_fault)?;
        let access: IDirect3DDxgiInterfaceAccess = surface.cast().map_err(native_fault)?;
        // SAFETY: WGC Surface exposes IDirect3DDxgiInterfaceAccess and the
        // requested interface is the documented D3D11 texture owner.
        let source: ID3D11Texture2D = unsafe { access.GetInterface() }.map_err(native_fault)?;
        let mut native_descriptor = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: output is valid for the complete descriptor.
        unsafe { source.GetDesc(&raw mut native_descriptor) };
        let descriptor = descriptor_from_native(&native_descriptor, extent)
            .map_err(|_| CaptureFault::UnsupportedFormat)?;

        let lease = self.textures.try_acquire(native_descriptor).map_err(|_| {
            self.domain
                .device_fault()
                .unwrap_or(CaptureFault::SourceInvalid)
        })?;
        let Some(lease) = lease else {
            let _drop = self.state.try_record_drop();
            return Ok(());
        };
        if !self.domain.try_copy(lease.texture(), &source)? {
            let _drop = self.state.try_record_drop();
            return Ok(());
        }

        // CopyResource has been issued and the private texture owns the future
        // GPU result. Release the WGC frame before constructing or publishing a
        // public storage owner.
        drop(source);
        drop(surface);
        drop(frame);

        let captured_at = frame_time(&mut transition, native_time);
        let continuity = if transition.pending_discontinuity {
            Continuity::Discontinuous
        } else if transition.pending_geometry_change {
            Continuity::GeometryChanged
        } else {
            Continuity::Continuous
        };
        let storage = WindowsFrameStorage::new(
            descriptor,
            Arc::clone(&self.domain),
            lease,
            self.failure
                .get()
                .cloned()
                .expect("failure sink initialized before capture starts"),
        );
        self.state
            .publish_storage(StoragePublication {
                captured_at,
                placement: Some(placement),
                storage,
                continuity,
            })
            .map_err(|refused| {
                if refused.error().status() == mado_pilot_core::Status::Closed {
                    CaptureFault::SessionClosed
                } else {
                    CaptureFault::SourceInvalid
                }
            })?;
        transition.placement = placement;
        transition.pending_discontinuity = false;
        transition.pending_geometry_change = false;
        transition.published = true;
        Ok(())
    }
}

impl StorageFailureSink for SessionCore {
    fn storage_failed(&self, fault: CaptureFault) {
        self.fail_native(fault);
    }
}

fn normalize_native_fault(
    fault: CaptureFault,
    kind: mado_pilot_core::TargetKind,
    target_present: bool,
) -> CaptureFault {
    match fault {
        CaptureFault::CaptureItemClosed if target_present => CaptureFault::ExplicitlyStopped,
        CaptureFault::CaptureItemClosed if kind == mado_pilot_core::TargetKind::Display => {
            CaptureFault::DisplayDisconnected
        }
        CaptureFault::DisplayDisconnected
            if kind == mado_pilot_core::TargetKind::Window && target_present =>
        {
            CaptureFault::ExplicitlyStopped
        }
        CaptureFault::DisplayDisconnected if kind == mado_pilot_core::TargetKind::Window => {
            CaptureFault::CaptureItemClosed
        }
        _ => fault,
    }
}

struct WgcFrameGuard(Option<Direct3D11CaptureFrame>);

impl WgcFrameGuard {
    fn frame(&self) -> &Direct3D11CaptureFrame {
        self.0.as_ref().expect("frame exists before guard drop")
    }
}

impl Drop for WgcFrameGuard {
    fn drop(&mut self) {
        if let Some(frame) = self.0.take() {
            let _close = frame.Close();
        }
    }
}

#[derive(Debug, Default)]
struct CallbackControl {
    state: Mutex<CallbackState>,
    drained: Condvar,
}

#[derive(Debug)]
struct CallbackState {
    accepting: bool,
    active: usize,
    fenced: bool,
}

impl Default for CallbackState {
    fn default() -> Self {
        Self {
            accepting: true,
            active: 0,
            fenced: false,
        }
    }
}

impl CallbackControl {
    fn admit(self: &Arc<Self>) -> Option<CallbackLease> {
        let mut state = self.lock();
        if !state.accepting || state.fenced {
            return None;
        }
        state.active += 1;
        Some(CallbackLease {
            control: Arc::clone(self),
        })
    }

    fn stop_admission(&self) {
        let mut state = self.lock();
        state.accepting = false;
    }

    fn drain(&self, operation: &OperationContext) -> Result<()> {
        let mut attempt = Operation::admit(operation)?;
        loop {
            let state = self.lock();
            if state.fenced {
                drop(state);
                return Ok(attempt.commit(())?);
            }
            if state.active == 0 {
                drop(state);
                attempt.commit(())?;
                let mut state = self.lock();
                // Admission stopped before handler removal, so active can only
                // decrease while draining and cannot become non-zero here.
                debug_assert_eq!(state.active, 0);
                state.fenced = true;
                return Ok(());
            }
            let (_state, _) = self
                .drained
                .wait_timeout(state, CALLBACK_POLL_INTERVAL)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            attempt.checkpoint()?;
        }
    }

    fn drain_uninterruptible(&self) {
        loop {
            let state = self.lock();
            if state.fenced {
                return;
            }
            if state.active == 0 {
                drop(state);
                let mut state = self.lock();
                if state.active == 0 {
                    state.fenced = true;
                    return;
                }
                continue;
            }
            let _state = self
                .drained
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    fn lock(&self) -> MutexGuard<'_, CallbackState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

struct CallbackLease {
    control: Arc<CallbackControl>,
}

impl Drop for CallbackLease {
    fn drop(&mut self) {
        {
            let mut state = self.control.lock();
            state.active = state.active.saturating_sub(1);
        }
        self.control.drained.notify_all();
    }
}

fn native_size(extent: PixelExtent) -> Result<SizeInt32> {
    Ok(SizeInt32 {
        Width: i32::try_from(extent.width()).map_err(|_| CaptureFault::InconsistentDescriptor)?,
        Height: i32::try_from(extent.height()).map_err(|_| CaptureFault::InconsistentDescriptor)?,
    })
}

fn positive_extent(size: SizeInt32) -> std::result::Result<PixelExtent, CaptureFault> {
    let width = u32::try_from(size.Width).map_err(|_| CaptureFault::InconsistentDescriptor)?;
    let height = u32::try_from(size.Height).map_err(|_| CaptureFault::InconsistentDescriptor)?;
    if width == 0 || height == 0 {
        return Err(CaptureFault::InconsistentDescriptor);
    }
    Ok(PixelExtent::new(width, height))
}

fn native_monotonic_now() -> Option<i64> {
    static FREQUENCY: OnceLock<i64> = OnceLock::new();
    let frequency = *FREQUENCY.get_or_init(|| {
        let mut frequency = 0;
        // SAFETY: frequency points to one writable i64.
        unsafe { QueryPerformanceFrequency(&raw mut frequency) }
            .map(|()| frequency)
            .unwrap_or(0)
    });
    if frequency <= 0 {
        return None;
    }
    let mut counter = 0;
    // SAFETY: counter points to one writable i64.
    unsafe { QueryPerformanceCounter(&raw mut counter) }.ok()?;
    let scaled = i128::from(counter)
        .checked_mul(10_000_000)?
        .checked_div(i128::from(frequency))?;
    i64::try_from(scaled).ok()
}

fn frame_time(transition: &mut TransitionState, native_time: i64) -> MonotonicInstant {
    let (native_origin, mado_origin) = *transition
        .clock_anchor
        .get_or_insert_with(|| (native_time, SystemClock.now()));
    let elapsed = native_time.saturating_sub(native_origin);
    let elapsed = u64::try_from(elapsed)
        .ok()
        .and_then(|ticks| ticks.checked_mul(100))
        .map(Duration::from_nanos)
        .unwrap_or(Duration::ZERO);
    mado_origin.checked_add(elapsed).unwrap_or(mado_origin)
}

fn lock_with_operation<'mutex>(
    mutex: &'mutex Mutex<()>,
    operation: &OperationContext,
) -> Result<MutexGuard<'mutex, ()>> {
    let mut attempt = Operation::admit(operation)?;
    loop {
        match mutex.try_lock() {
            Ok(guard) => return Ok(attempt.commit(guard)?),
            Err(TryLockError::Poisoned(poisoned)) => {
                return Ok(attempt.commit(poisoned.into_inner())?);
            }
            Err(TryLockError::WouldBlock) => {
                thread::sleep(CALLBACK_POLL_INTERVAL);
                attempt.checkpoint()?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use mado_pilot_capture::CaptureFault;
    use mado_pilot_core::{
        CancellationToken, OperationContext, PixelExtent, Scale, Status, TargetPlacement,
    };

    use super::{CallbackControl, TransitionState, frame_time, normalize_native_fault};

    #[test]
    fn callback_fence_is_retryable_after_cancelled_drain() {
        let control = Arc::new(CallbackControl::default());
        let lease = control.admit().expect("admitted before stop");
        control.stop_admission();
        assert!(control.admit().is_none(), "stop disables admission first");

        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let interrupted = OperationContext::new().with_cancellation(cancellation);
        let error = control
            .drain(&interrupted)
            .expect_err("active callback cannot drain through cancellation");
        assert_eq!(error.status(), Status::Cancelled);

        drop(lease);
        control
            .drain(&OperationContext::new())
            .expect("later close completes the same drain");
        assert!(control.lock().fenced);
        assert!(control.admit().is_none(), "nothing admits after the fence");
    }

    #[test]
    fn native_close_faults_are_normalized_by_target_kind_and_presence() {
        assert_eq!(
            normalize_native_fault(
                CaptureFault::CaptureItemClosed,
                mado_pilot_core::TargetKind::Display,
                false,
            ),
            CaptureFault::DisplayDisconnected
        );
        assert_eq!(
            normalize_native_fault(
                CaptureFault::CaptureItemClosed,
                mado_pilot_core::TargetKind::Window,
                true,
            ),
            CaptureFault::ExplicitlyStopped
        );
        assert_eq!(
            normalize_native_fault(
                CaptureFault::DeviceRemoved,
                mado_pilot_core::TargetKind::Window,
                true,
            ),
            CaptureFault::DeviceRemoved
        );
    }

    #[test]
    fn uninterruptible_drop_drain_waits_for_an_admitted_callback() {
        let control = Arc::new(CallbackControl::default());
        let lease = control.admit().expect("callback admitted");
        control.stop_admission();
        let drainer = {
            let control = Arc::clone(&control);
            thread::spawn(move || control.drain_uninterruptible())
        };

        assert!(
            !drainer.is_finished(),
            "active callback keeps the fence open"
        );
        drop(lease);
        drainer.join().expect("drop drain completed");
        assert!(control.lock().fenced);
    }

    #[test]
    fn native_frame_times_keep_the_wgc_relative_interval() {
        let mut transition = TransitionState {
            extent: PixelExtent::new(4, 4),
            placement: TargetPlacement::new(
                (0.0, 0.0),
                (4.0, 4.0),
                Scale::new(1.0, 1.0).expect("scale"),
            )
            .expect("placement"),
            pending_discontinuity: false,
            pending_geometry_change: false,
            movement_not_before: None,
            clock_anchor: None,
            published: false,
        };

        let first = frame_time(&mut transition, 100);
        let second = frame_time(&mut transition, 130);

        assert_eq!(
            second.saturating_duration_since(first),
            Duration::from_micros(3)
        );
    }
}
