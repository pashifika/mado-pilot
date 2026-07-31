//! WGC session ownership, callback fencing, detachment, and teardown.

use std::fmt;
use std::num::NonZeroU32;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, TryLockError};
use std::thread;
use std::time::Duration;

use mado_pilot_capture::{
    CaptureFault, CaptureSession, Continuity, Frame, FrameRequest, Lifecycle, OverflowPolicy,
    QueuePolicy, SessionDescription, StoragePublication, StreamState,
};
use mado_pilot_core::{
    Clock, Operation, OperationContext, PixelExtent, Result, StreamId, SystemClock, TargetId,
    TargetPlacement,
};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Graphics::Direct3D11::{D3D11_TEXTURE2D_DESC, ID3D11Texture2D};
use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;
use windows::core::{IInspectable, Interface};

use crate::discovery::{NativeKey, TargetMetadata, current_placement};
use crate::storage::{
    DETACHED_TEXTURE_BUDGET, DeviceDomain, TexturePool, WindowsFrameStorage,
    descriptor_from_native, native_fault,
};

const WGC_PRODUCER_POOL_SIZE: i32 = 2;
const CALLBACK_POLL_INTERVAL: Duration = Duration::from_millis(2);
const NATIVE_RELEASE_QUIESCENCE: Duration = Duration::from_millis(5);

pub(crate) struct NativeSession {
    description: SessionDescription,
    core: Arc<SessionCore>,
    runtime: Mutex<RuntimeState>,
    close_gate: Mutex<()>,
}

struct RuntimeState {
    resources: Option<NativeResources>,
    handlers_removed: bool,
    native_closed: bool,
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
    transition: Mutex<TransitionState>,
}

#[derive(Debug, Clone, Copy)]
struct TransitionState {
    extent: PixelExtent,
    placement: TargetPlacement,
    pending_discontinuity: bool,
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
            callbacks,
            transition: Mutex::new(TransitionState {
                extent: metadata.extent,
                placement: metadata.placement,
                pending_discontinuity: false,
            }),
        });

        let size = native_size(metadata.extent)?;
        let winrt_device = core.domain.winrt_device()?;
        let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
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
        let frame_handler =
            TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |sender, _| {
                let Some(core) = frame_weak.upgrade() else {
                    return Ok(());
                };
                let Some(_lease) = core.callbacks.admit() else {
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
        let closed_handler =
            TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
                if let Some(core) = closed_weak.upgrade()
                    && let Some(_lease) = core.callbacks.admit()
                {
                    core.callbacks.stop_admission();
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
                native_closed: false,
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

    fn close_native(&self) -> Result<()> {
        let capture_already_ended = self.core.state.terminal().is_some();
        let resources = {
            let mut runtime = self.runtime();
            if runtime.native_closed {
                return Ok(());
            }
            runtime.native_closed = true;
            runtime.resources.take()
        };
        if let Some(resources) = resources {
            let NativeResources {
                item,
                frame_pool,
                capture,
                frame_token: _,
                closed_token: _,
            } = resources;
            // A terminal native outcome means WGC already ended the capture.
            // Calling GraphicsCaptureSession::Close again after Item.Closed can
            // wait indefinitely inside WinRT. Releasing that ended session is
            // sufficient; an ordinary caller close still invokes Close.
            let capture_result = if capture_already_ended {
                Ok(())
            } else {
                capture.Close().map_err(native_fault)
            };
            let pool_result = frame_pool.Close().map_err(native_fault);
            let close_fault = capture_result.err().or_else(|| pool_result.err());
            drop(frame_pool);
            if capture_already_ended || close_fault.is_some() {
                // A removed handler can still leave a queued agile delegate's
                // sender reference in the WGC event queue for one scheduling
                // turn. Releasing the terminal session in that interval can
                // deadlock its RPC teardown against the queued release. The
                // pool is already closed and callbacks are fenced, so allow a
                // small bounded quiescence window before the final release.
                thread::sleep(NATIVE_RELEASE_QUIESCENCE);
            }
            drop(capture);
            drop(item);
            if let Some(fault) = close_fault {
                return Err(fault.into());
            }
        }
        Ok(())
    }
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
        self.core.state.begin_close();
        self.core.callbacks.stop_admission();
        let _close = lock_with_operation(&self.close_gate, operation)?;
        self.remove_handlers();
        self.core.callbacks.drain(operation)?;
        self.close_native()?;
        self.core.state.drain(operation)
    }

    fn lifecycle(&self) -> Lifecycle {
        self.core.state.lifecycle()
    }
}

impl Drop for NativeSession {
    fn drop(&mut self) {
        self.core.callbacks.stop_admission();
        self.remove_handlers();
        let _closed = self.close_native();
    }
}

impl SessionCore {
    fn on_frame(&self, sender: &Direct3D11CaptureFramePool) {
        let result = self.process_frame(sender);
        if let Err(fault) = result {
            self.callbacks.stop_admission();
            self.state.terminate(fault);
        }
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
        let extent = positive_extent(content_size)?;

        if extent != transition.extent {
            // The transition frame and its producer surface are released before
            // pool recreation. The first frame from the recreated pool receives
            // the discontinuity.
            drop(frame);
            let device = self
                .domain
                .winrt_device()
                .map_err(|_| CaptureFault::SourceInvalid)?;
            sender
                .Recreate(
                    &device,
                    DirectXPixelFormat::B8G8R8A8UIntNormalized,
                    WGC_PRODUCER_POOL_SIZE,
                    content_size,
                )
                .map_err(native_fault)?;
            self.textures.retire_for_resize();
            transition.extent = extent;
            transition.pending_discontinuity = true;
            return Ok(());
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

        let Some(lease) = self
            .textures
            .try_acquire(native_descriptor)
            .map_err(|_| CaptureFault::SourceInvalid)?
        else {
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

        let placement = current_placement(self.key, extent).ok_or(CaptureFault::TargetLost)?;
        let continuity = if transition.pending_discontinuity {
            Continuity::Discontinuous
        } else if placement != transition.placement {
            Continuity::GeometryChanged
        } else {
            Continuity::Continuous
        };
        let storage = WindowsFrameStorage::new(descriptor, Arc::clone(&self.domain), lease);
        self.state
            .publish_storage(StoragePublication {
                captured_at: SystemClock.now(),
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
        Ok(())
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

    use mado_pilot_core::{CancellationToken, OperationContext, Status};

    use super::CallbackControl;

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
}
