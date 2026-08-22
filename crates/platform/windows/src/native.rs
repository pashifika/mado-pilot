//! WGC session ownership, callback fencing, detachment, and teardown.

use std::fmt;
use std::io;
use std::mem::ManuallyDrop;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, TryLockError, Weak};
use std::thread;
use std::time::{Duration, Instant};

use mado_pilot_capture::{
    CaptureFault, CaptureSession, Continuity, Frame, FrameRequest, Lifecycle, OverflowPolicy,
    QueuePolicy, SessionDescription, StoragePublication, StreamState,
};
use mado_pilot_core::{
    Clock, MonotonicInstant, Operation, OperationContext, PixelExtent, Result, StreamId,
    SystemClock, TargetId, TargetKind, TargetPlacement,
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
use crate::input::GeometryLedger;
use crate::storage::{
    DeviceDomain, DeviceTerminal, RetainedBytes, SessionMemory, StorageFailureSink, TexturePool,
    WindowsFrameStorage, descriptor_from_native, native_fault, retained_storage_capacity,
    validate_surface,
};

const WGC_PRODUCER_POOL_SIZE: i32 = 2;
const CALLBACK_POLL_INTERVAL: Duration = Duration::from_millis(2);
const TEARDOWN_START_TIMEOUT: Duration = Duration::from_secs(5);
const TEARDOWN_QUEUE_CAPACITY: usize = 64;
const TEARDOWN_WORKER_COUNT: usize = 4;

pub(crate) struct NativeSessionSource {
    target: TargetId,
    stream: StreamId,
    kind: TargetKind,
    key: NativeKey,
    metadata: TargetMetadata,
    item: GraphicsCaptureItem,
    geometry: Arc<GeometryLedger>,
}

impl NativeSessionSource {
    pub(crate) fn new(
        target: TargetId,
        stream: StreamId,
        kind: TargetKind,
        key: NativeKey,
        metadata: TargetMetadata,
        item: GraphicsCaptureItem,
        geometry: Arc<GeometryLedger>,
    ) -> Self {
        Self {
            target,
            stream,
            kind,
            key,
            metadata,
            item,
            geometry,
        }
    }
}

pub(crate) struct NativeSession {
    description: SessionDescription,
    core: Arc<SessionCore>,
    runtime: Mutex<RuntimeState>,
    close_gate: Mutex<()>,
    teardown: Mutex<Arc<TeardownExecutor>>,
}

struct RuntimeState {
    resources: Option<NativeResources>,
    frame_handler_removed: bool,
    close_task: Option<Receiver<Option<CaptureFault>>>,
    close_result: Option<Option<CaptureFault>>,
    close_fault_reported: bool,
}

type NativeResources = Arc<NativeOwnership<WgcResources>>;

struct NativeOwnership<T> {
    native: T,
    producer_bytes: Mutex<RetainedBytes>,
}

struct WgcResources {
    item: GraphicsCaptureItem,
    frame_pool: Direct3D11CaptureFramePool,
    capture: GraphicsCaptureSession,
    frame_token: i64,
    closed_token: i64,
    _teardown_permit: TeardownPermit,
}

impl<T> NativeOwnership<T> {
    fn new(native: T, producer_bytes: RetainedBytes) -> Arc<Self> {
        Arc::new(Self {
            native,
            producer_bytes: Mutex::new(producer_bytes),
        })
    }

    fn replace_producer_after_recreate(
        &self,
        replacement: RetainedBytes,
        recreate: impl FnOnce() -> std::result::Result<(), CaptureFault>,
    ) -> std::result::Result<(), CaptureFault> {
        let mut current = match self.producer_bytes.try_lock() {
            Ok(current) => current,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return Err(CaptureFault::SourceInvalid),
        };
        recreate()?;
        let retired = std::mem::replace(&mut *current, replacement);
        drop(current);
        drop(retired);
        Ok(())
    }
}

struct SessionCore {
    target_kind: TargetKind,
    stream: StreamId,
    key: NativeKey,
    state: StreamState,
    geometry: GeometryRegistration,
    domain: Arc<DeviceDomain>,
    memory: Arc<SessionMemory>,
    native: OnceLock<Weak<NativeOwnership<WgcResources>>>,
    retained_storage_capacity: NonZeroU32,
    device_terminal: Arc<DeviceTerminal>,
    textures: Arc<TexturePool>,
    callbacks: Arc<CallbackControl>,
    native_ended: Arc<AtomicBool>,
    failure: OnceLock<Weak<dyn StorageFailureSink>>,
    transition: Mutex<TransitionState>,
}

/// Owns one live stream's geometry-ledger membership.
///
/// `SessionCore` is retained by every admitted callback. Binding removal to this
/// member's final drop means teardown cannot remove the entry and then let an
/// already-admitted callback publish it again.
struct GeometryRegistration {
    ledger: Arc<GeometryLedger>,
    stream: StreamId,
}

impl GeometryRegistration {
    fn new(ledger: Arc<GeometryLedger>, stream: StreamId) -> Self {
        Self { ledger, stream }
    }

    fn publish(&self, frame: &Frame) {
        self.ledger.publish(frame);
    }
}

impl Drop for GeometryRegistration {
    fn drop(&mut self) {
        self.ledger.remove(self.stream);
    }
}

#[derive(Debug)]
struct TransitionState {
    extent: PixelExtent,
    placement: TargetPlacement,
    pending_discontinuity: bool,
    pending_geometry_change: bool,
    movement_not_before: Option<i64>,
    clock_anchor: (i64, MonotonicInstant),
    published: bool,
}

struct TeardownJob {
    resources: NativeResources,
    remove_frame_handler: bool,
    drain_callbacks: bool,
    callbacks: Arc<CallbackControl>,
    native_ended: Arc<AtomicBool>,
    complete: mpsc::Sender<Option<CaptureFault>>,
}

struct TeardownExecutor {
    sender: SyncSender<TeardownJob>,
    permits: Arc<TeardownPermits>,
    live_workers: Arc<AtomicUsize>,
}

enum TeardownExecutorSlot {
    Empty,
    Starting(Arc<TeardownStartup>),
    Ready(Arc<TeardownExecutor>),
}

struct TeardownStartup {
    state: Mutex<TeardownStartupState>,
    live_workers: Arc<AtomicUsize>,
}

struct TeardownStartupState {
    initialized: Receiver<WorkerInitialization>,
    sender: Option<SyncSender<TeardownJob>>,
    expected_workers: usize,
    reported_workers: usize,
    started_at: Instant,
    failure: Option<CaptureFault>,
    executor: Option<Arc<TeardownExecutor>>,
}

struct WorkerInitialization {
    reported_at: Instant,
    result: std::result::Result<(), CaptureFault>,
}

enum TeardownStartupPoll {
    Pending,
    Ready(Arc<TeardownExecutor>),
    Failed(CaptureFault),
}

struct TeardownPermits {
    available: Mutex<usize>,
}

struct TeardownPermit {
    permits: Arc<TeardownPermits>,
}

struct WorkerGuard {
    live_workers: Arc<AtomicUsize>,
}

impl NativeSession {
    pub(crate) fn open(
        source: NativeSessionSource,
        operation: &mut Operation<'_>,
    ) -> Result<Arc<Self>> {
        let NativeSessionSource {
            target,
            stream,
            kind,
            key,
            metadata,
            item,
            geometry,
        } = source;
        let teardown = teardown_executor(operation)?;
        let teardown_permit = teardown.reserve(operation)?;
        let clock_anchor = clock_calibration().ok_or(CaptureFault::SourceInvalid)?;
        let domain = DeviceDomain::create()?;
        let layout = validate_surface(metadata.extent.width(), metadata.extent.height())?;
        let retained_storage_capacity = retained_storage_capacity(layout)?;
        let memory = SessionMemory::production();
        let producer_bytes = layout
            .bytes()
            .checked_mul(u64::try_from(WGC_PRODUCER_POOL_SIZE).expect("positive pool size"))
            .ok_or(CaptureFault::ResourceLimitExceeded)?;
        let producer_bytes = memory.reserve(producer_bytes)?;
        let device_terminal = Arc::new(DeviceTerminal::default());
        let textures = TexturePool::new(
            Arc::clone(&domain),
            Arc::clone(&memory),
            retained_storage_capacity,
        );
        let callbacks = Arc::new(CallbackControl::default());
        let core = Arc::new(SessionCore {
            target_kind: kind,
            stream,
            key,
            state: StreamState::with_target_extent(stream),
            geometry: GeometryRegistration::new(geometry, stream),
            domain,
            memory,
            native: OnceLock::new(),
            retained_storage_capacity,
            device_terminal,
            textures,
            callbacks: Arc::clone(&callbacks),
            native_ended: Arc::new(AtomicBool::new(false)),
            failure: OnceLock::new(),
            transition: Mutex::new(TransitionState {
                extent: metadata.extent,
                placement: metadata.placement,
                pending_discontinuity: false,
                pending_geometry_change: false,
                movement_not_before: None,
                clock_anchor,
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
            .map_err(|error| native_target_fault(error, kind))?;

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
        let closed_native_ended = Arc::clone(&core.native_ended);
        let closed_handler =
            TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
                // This latch is lifetime-independent callback state. Record the
                // authoritative native end even when close already stopped owner
                // admission and the delegate must not touch SessionCore.
                record_authoritative_native_end(&closed_native_ended);
                let Some(_lease) = closed_callbacks.admit() else {
                    return Ok(());
                };
                if let Some(core) = closed_weak.upgrade() {
                    core.callbacks.stop_admission();
                    core.state.terminate(target_fault(core.target_kind));
                }
                Ok(())
            });
        let closed_token = match item.Closed(&closed_handler) {
            Ok(token) => token,
            Err(error) => {
                let _remove = frame_pool.RemoveFrameArrived(frame_token);
                return Err(native_target_fault(error, kind).into());
            }
        };

        let resources = NativeOwnership::new(
            WgcResources {
                item,
                frame_pool,
                capture,
                frame_token,
                closed_token,
                _teardown_permit: teardown_permit,
            },
            producer_bytes,
        );
        core.native
            .set(Arc::downgrade(&resources))
            .expect("native ownership initializes once");
        let description = SessionDescription::new(
            target,
            stream,
            metadata.extent,
            mado_pilot_capture::PixelFormat::Bgra8,
            mado_pilot_capture::CoordinateSupport::with_target_placement(),
        )
        .with_queue(session_queue_policy(retained_storage_capacity));
        let session = Arc::new(Self {
            description,
            core,
            runtime: Mutex::new(RuntimeState {
                resources: Some(resources),
                frame_handler_removed: false,
                close_task: None,
                close_result: None,
                close_fault_reported: false,
            }),
            close_gate: Mutex::new(()),
            teardown: Mutex::new(teardown),
        });
        {
            let runtime = session.runtime();
            runtime
                .resources
                .as_ref()
                .expect("resources exist before start")
                .native
                .capture
                .StartCapture()
                .map_err(|error| native_target_fault(error, kind))?;
        }
        Ok(session)
    }

    fn runtime(&self) -> MutexGuard<'_, RuntimeState> {
        self.runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn remove_frame_handler(&self) {
        let handler = {
            let mut runtime = self.runtime();
            if runtime.frame_handler_removed {
                None
            } else {
                runtime.frame_handler_removed = true;
                runtime.resources.as_ref().map(|resources| {
                    (
                        resources.native.frame_pool.clone(),
                        resources.native.frame_token,
                    )
                })
            }
        };
        if let Some((frame_pool, frame_token)) = handler {
            let _frame = frame_pool.RemoveFrameArrived(frame_token);
        }
    }

    fn start_native_close(
        &self,
        drain_callbacks: bool,
        restart: Option<&mut Operation<'_>>,
    ) -> Result<()> {
        let mut runtime = self.runtime();
        if runtime.close_task.is_some() || runtime.close_result.is_some() {
            return Ok(());
        }
        let Some(resources) = runtime.resources.take() else {
            runtime.close_result = Some(None);
            return Ok(());
        };
        let remove_frame_handler = !runtime.frame_handler_removed;
        let (complete, result) = mpsc::channel();
        let job = TeardownJob {
            resources,
            remove_frame_handler,
            drain_callbacks,
            callbacks: Arc::clone(&self.core.callbacks),
            native_ended: Arc::clone(&self.core.native_ended),
            complete,
        };
        match self.send_teardown(job, restart) {
            Ok(()) => {
                runtime.frame_handler_removed = true;
                runtime.close_task = Some(result);
                Ok(())
            }
            Err((job, error)) => {
                runtime.resources = Some(job.resources);
                Err(error)
            }
        }
    }

    fn send_teardown(
        &self,
        job: TeardownJob,
        restart: Option<&mut Operation<'_>>,
    ) -> std::result::Result<(), (TeardownJob, mado_pilot_core::Error)> {
        let current = self
            .teardown
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        match current.sender.try_send(job) {
            Err(TrySendError::Disconnected(job)) if restart.is_some() => {
                let replacement =
                    match teardown_executor(restart.expect("restart operation was checked")) {
                        Ok(replacement) => replacement,
                        Err(error) => return Err((job, error)),
                    };
                *self
                    .teardown
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Arc::clone(&replacement);
                replacement.sender.try_send(job).map_err(|error| {
                    (
                        match error {
                            TrySendError::Full(job) | TrySendError::Disconnected(job) => job,
                        },
                        CaptureFault::SourceInvalid.into(),
                    )
                })
            }
            Ok(()) => Ok(()),
            Err(TrySendError::Full(job) | TrySendError::Disconnected(job)) => {
                Err((job, CaptureFault::SourceInvalid.into()))
            }
        }
    }

    fn close_native(&self, operation: &OperationContext) -> Result<()> {
        let mut attempt = Operation::admit(operation)?;
        self.start_native_close(false, Some(&mut attempt))?;
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

fn session_queue_policy(retained_storage_capacity: NonZeroU32) -> QueuePolicy {
    QueuePolicy::new(NonZeroU32::MIN, OverflowPolicy::LatestWins)
        .with_process_shared_retained_storage(retained_storage_capacity)
}

fn close_native_resources(
    resources: NativeResources,
    capture_already_ended: bool,
) -> std::result::Result<(), CaptureFault> {
    // Item.Closed is the one state proving WGC already ended the capture.
    // Ordinary close and unrelated stream terminal faults still invoke Close.
    let capture_result = if capture_already_ended {
        Ok(())
    } else {
        native_close_result(resources.native.capture.Close())
    };
    let _closed = resources
        .native
        .item
        .RemoveClosed(resources.native.closed_token);
    let pool_result = native_close_result(resources.native.frame_pool.Close());
    let close_fault = capture_result.err().or_else(|| pool_result.err());
    // Callback drain guarantees the teardown job owns the final strong native
    // owner. Dropping it releases the frame pool and its producer reservation
    // together, independently of the closed SessionCore handle lifetime.
    debug_assert_eq!(Arc::strong_count(&resources), 1);
    drop(resources);
    close_fault.map_or(Ok(()), Err)
}

impl TeardownJob {
    fn run(self) {
        let Self {
            resources,
            remove_frame_handler,
            drain_callbacks,
            callbacks,
            native_ended,
            complete,
        } = self;
        if remove_frame_handler {
            let _frame = resources
                .native
                .frame_pool
                .RemoveFrameArrived(resources.native.frame_token);
        }
        let capture_already_ended =
            capture_already_ended_after_drain(drain_callbacks, &callbacks, &native_ended);
        let fault = close_native_resources(resources, capture_already_ended).err();
        let _sent = complete.send(fault);
    }
}

fn capture_already_ended_after_drain(
    drain_callbacks: bool,
    callbacks: &CallbackControl,
    native_ended: &AtomicBool,
) -> bool {
    if drain_callbacks {
        callbacks.drain_uninterruptible();
    }
    native_ended.load(Ordering::Acquire)
}

fn record_authoritative_native_end(native_ended: &AtomicBool) {
    native_ended.store(true, Ordering::Release);
}

impl TeardownExecutor {
    fn reserve(&self, operation: &mut Operation<'_>) -> Result<TeardownPermit> {
        loop {
            if let Some(permit) = self.permits.try_reserve() {
                return Ok(permit);
            }
            thread::sleep(CALLBACK_POLL_INTERVAL);
            operation.checkpoint()?;
        }
    }

    fn has_live_worker(&self) -> bool {
        self.live_workers.load(Ordering::Acquire) != 0
    }
}

impl TeardownStartup {
    fn wait(&self, operation: &mut Operation<'_>) -> Result<Arc<TeardownExecutor>> {
        loop {
            operation.checkpoint()?;
            match self.poll(operation)? {
                TeardownStartupPoll::Pending => {
                    thread::sleep(CALLBACK_POLL_INTERVAL);
                }
                TeardownStartupPoll::Ready(executor) => return Ok(executor),
                TeardownStartupPoll::Failed(fault) => return Err(fault.into()),
            }
        }
    }

    fn poll(&self, operation: &mut Operation<'_>) -> Result<TeardownStartupPoll> {
        let mut state = lock_with_attempt(&self.state, operation)?;
        if let Some(executor) = state.executor.as_ref() {
            return Ok(TeardownStartupPoll::Ready(Arc::clone(executor)));
        }
        if let Some(fault) = state.failure {
            return Ok(TeardownStartupPoll::Failed(fault));
        }

        loop {
            match state.initialized.try_recv() {
                Ok(initialization) => {
                    state.reported_workers += 1;
                    match initialization.result {
                        Ok(())
                            if initialization.reported_at.duration_since(state.started_at)
                                > TEARDOWN_START_TIMEOUT =>
                        {
                            state.failure = Some(CaptureFault::SourceInvalid);
                        }
                        Ok(()) => {}
                        Err(fault) => state.failure = Some(fault),
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if state.reported_workers < state.expected_workers {
                        state.failure = Some(CaptureFault::SourceInvalid);
                    }
                    break;
                }
            }
        }

        if state.failure.is_none()
            && state.reported_workers < state.expected_workers
            && state.started_at.elapsed() >= TEARDOWN_START_TIMEOUT
        {
            state.failure = Some(CaptureFault::SourceInvalid);
        }
        if let Some(fault) = state.failure {
            state.sender.take();
            return Ok(TeardownStartupPoll::Failed(fault));
        }
        if state.expected_workers == TEARDOWN_WORKER_COUNT
            && state.reported_workers == state.expected_workers
        {
            let executor = Arc::new(TeardownExecutor {
                sender: state
                    .sender
                    .take()
                    .expect("successful startup still owns its job sender"),
                permits: teardown_permits(),
                live_workers: Arc::clone(&self.live_workers),
            });
            state.executor = Some(Arc::clone(&executor));
            return Ok(TeardownStartupPoll::Ready(executor));
        }
        Ok(TeardownStartupPoll::Pending)
    }

    fn is_retired(&self) -> bool {
        let failed = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .failure
            .is_some();
        failed && self.live_workers.load(Ordering::Acquire) == 0
    }
}

impl TeardownPermits {
    fn new() -> Self {
        Self {
            available: Mutex::new(TEARDOWN_QUEUE_CAPACITY),
        }
    }

    fn try_reserve(self: &Arc<Self>) -> Option<TeardownPermit> {
        let mut available = self
            .available
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *available == 0 {
            return None;
        }
        *available -= 1;
        Some(TeardownPermit {
            permits: Arc::clone(self),
        })
    }

    fn release(&self) {
        let mut available = self
            .available
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *available = available.saturating_add(1).min(TEARDOWN_QUEUE_CAPACITY);
    }
}

impl Drop for TeardownPermit {
    fn drop(&mut self) {
        self.permits.release();
    }
}

impl WorkerGuard {
    fn new(live_workers: Arc<AtomicUsize>) -> Self {
        live_workers.fetch_add(1, Ordering::AcqRel);
        Self { live_workers }
    }
}

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        self.live_workers.fetch_sub(1, Ordering::AcqRel);
    }
}

fn teardown_executor(operation: &mut Operation<'_>) -> Result<Arc<TeardownExecutor>> {
    static EXECUTOR: OnceLock<Mutex<TeardownExecutorSlot>> = OnceLock::new();
    let slot = EXECUTOR.get_or_init(|| Mutex::new(TeardownExecutorSlot::Empty));
    teardown_executor_from_slot(
        slot,
        || ensure_winrt_apartment().map_err(|_| CaptureFault::UnsupportedOption),
        operation,
    )
}

fn teardown_permits() -> Arc<TeardownPermits> {
    static PERMITS: OnceLock<Arc<TeardownPermits>> = OnceLock::new();
    Arc::clone(PERMITS.get_or_init(|| Arc::new(TeardownPermits::new())))
}

fn teardown_executor_from_slot(
    slot: &Mutex<TeardownExecutorSlot>,
    initialize: fn() -> std::result::Result<(), CaptureFault>,
    operation: &mut Operation<'_>,
) -> Result<Arc<TeardownExecutor>> {
    loop {
        operation.checkpoint()?;
        let startup = {
            let mut current = lock_with_attempt(slot, operation)?;
            match &*current {
                TeardownExecutorSlot::Ready(executor) if executor.has_live_worker() => {
                    return Ok(Arc::clone(executor));
                }
                TeardownExecutorSlot::Ready(_) => {
                    *current = TeardownExecutorSlot::Empty;
                    continue;
                }
                TeardownExecutorSlot::Starting(startup) if startup.is_retired() => {
                    *current = TeardownExecutorSlot::Empty;
                    continue;
                }
                TeardownExecutorSlot::Starting(startup) => Arc::clone(startup),
                TeardownExecutorSlot::Empty => {
                    let startup = begin_teardown_executor_with(initialize);
                    *current = TeardownExecutorSlot::Starting(Arc::clone(&startup));
                    startup
                }
            }
        };

        let executor = startup.wait(operation)?;
        let mut current = lock_with_attempt(slot, operation)?;
        if matches!(
            &*current,
            TeardownExecutorSlot::Starting(existing) if Arc::ptr_eq(existing, &startup)
        ) {
            *current = TeardownExecutorSlot::Ready(Arc::clone(&executor));
        }
        return Ok(executor);
    }
}

#[cfg(test)]
fn start_teardown_executor_with(
    initialize: fn() -> std::result::Result<(), CaptureFault>,
    operation: &mut Operation<'_>,
) -> Result<Arc<TeardownExecutor>> {
    let slot = Mutex::new(TeardownExecutorSlot::Empty);
    teardown_executor_from_slot(&slot, initialize, operation)
}

fn begin_teardown_executor_with(
    initialize: fn() -> std::result::Result<(), CaptureFault>,
) -> Arc<TeardownStartup> {
    let (sender, jobs) = teardown_channel::<TeardownJob>();
    let jobs = Arc::new(Mutex::new(jobs));
    let live_workers = Arc::new(AtomicUsize::new(0));
    let (ready, initialized) = mpsc::sync_channel(TEARDOWN_WORKER_COUNT);
    let startup = Arc::new(TeardownStartup {
        state: Mutex::new(TeardownStartupState {
            initialized,
            sender: Some(sender),
            expected_workers: 0,
            reported_workers: 0,
            started_at: Instant::now(),
            failure: None,
            executor: None,
        }),
        live_workers: Arc::clone(&live_workers),
    });
    let mut expected_workers = 0;
    let mut spawn_failure = None;

    for index in 0..TEARDOWN_WORKER_COUNT {
        let jobs = Arc::clone(&jobs);
        let ready = ready.clone();
        let worker_guard = WorkerGuard::new(Arc::clone(&live_workers));
        let worker = thread::Builder::new()
            .name(format!("mado-pilot-wgc-teardown-{index}"))
            .spawn(move || {
                let _guard = worker_guard;
                let result = initialize();
                let failed = result.is_err();
                if ready
                    .send(WorkerInitialization {
                        reported_at: Instant::now(),
                        result,
                    })
                    .is_err()
                    || failed
                {
                    return;
                }
                loop {
                    let job = jobs
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .recv();
                    let Ok(job) = job else {
                        return;
                    };
                    job.run();
                }
            });
        match map_worker_start(worker) {
            Ok(()) => expected_workers += 1,
            Err(fault) => {
                spawn_failure = Some(fault);
                break;
            }
        }
    }
    drop(ready);

    {
        let mut state = startup
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.expected_workers = expected_workers;
        if let Some(fault) = spawn_failure {
            state.failure = Some(fault);
            state.sender.take();
        }
    }

    startup
}

fn teardown_channel<T>() -> (SyncSender<T>, Receiver<T>) {
    mpsc::sync_channel(TEARDOWN_QUEUE_CAPACITY)
}

fn map_worker_start(
    worker: io::Result<thread::JoinHandle<()>>,
) -> std::result::Result<(), CaptureFault> {
    worker
        .map(|_worker| ())
        .map_err(|_| CaptureFault::SourceInvalid)
}

fn lock_with_attempt<'mutex, T>(
    mutex: &'mutex Mutex<T>,
    operation: &mut Operation<'_>,
) -> Result<MutexGuard<'mutex, T>> {
    loop {
        match mutex.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::Poisoned(poisoned)) => return Ok(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) => {
                thread::sleep(CALLBACK_POLL_INTERVAL);
                operation.checkpoint()?;
            }
        }
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
            self.core.fail_native(target_fault(self.core.target_kind));
        }
        self.core.state.frame(request, operation)
    }

    fn close(&self, operation: &OperationContext) -> Result<()> {
        ensure_winrt_apartment()?;
        self.core.state.begin_close();
        self.core.callbacks.stop_admission();
        let _close = lock_with_operation(&self.close_gate, operation)?;
        self.remove_frame_handler();
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
        let _start = self.start_native_close(true, None);
        let abandoned = self.runtime().resources.take();
        if let Some(resources) = abandoned {
            // A worker pool can become unavailable after startup only if every
            // worker fails. Releasing WinRT resources on an arbitrary caller
            // thread would violate the apartment contract, so Drop quarantines
            // this ownership. Its retained teardown permit keeps the total
            // process-lifetime quarantine within TEARDOWN_QUEUE_CAPACITY.
            let _abandoned = ManuallyDrop::new(resources);
        }
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
        let fault = normalize_native_fault(fault, self.target_kind);
        self.device_terminal.record(fault);
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
            let layout = validate_surface(extent.width(), extent.height())?;
            if retained_storage_capacity(layout)? < self.retained_storage_capacity {
                return Err(CaptureFault::ResourceLimitExceeded);
            }
            let producer_bytes = layout
                .bytes()
                .checked_mul(u64::try_from(WGC_PRODUCER_POOL_SIZE).expect("positive pool size"))
                .ok_or(CaptureFault::ResourceLimitExceeded)?;
            let producer_bytes = self.memory.reserve(producer_bytes).map_err(|error| {
                if error.status() == mado_pilot_core::Status::LimitExceeded {
                    CaptureFault::ResourceLimitExceeded
                } else {
                    CaptureFault::SourceInvalid
                }
            })?;
            // The transition frame and its producer surface are released before
            // pool recreation. The first frame from the recreated pool receives
            // the discontinuity.
            drop(frame);
            let device = self.domain.winrt_device().map_err(|_| {
                self.domain
                    .device_fault()
                    .unwrap_or(CaptureFault::SourceInvalid)
            })?;
            let native = self
                .native
                .get()
                .and_then(Weak::upgrade)
                .ok_or(CaptureFault::SessionClosed)?;
            native.replace_producer_after_recreate(producer_bytes, || {
                sender
                    .Recreate(
                        &device,
                        DirectXPixelFormat::B8G8R8A8UIntNormalized,
                        WGC_PRODUCER_POOL_SIZE,
                        content_size,
                    )
                    .map_err(native_fault)
            })?;
            let _retired = self.textures.try_retire_for_resize();
            transition.extent = extent;
            transition.pending_discontinuity = transition.published;
            transition.movement_not_before = None;
            return Ok(());
        }

        let placement =
            current_placement(self.key, extent).ok_or_else(|| target_fault(self.target_kind))?;
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
        let Some(descriptor) = descriptor_from_native(&native_descriptor, extent)? else {
            let _drop = self.state.try_record_drop();
            return Ok(());
        };
        let captured_at = frame_time(&transition, native_time);

        let lease = self.textures.try_acquire(native_descriptor).map_err(|_| {
            self.domain
                .device_fault()
                .unwrap_or(CaptureFault::SourceInvalid)
        })?;
        let Some(lease) = lease else {
            let _drop = self.state.try_record_drop();
            return Ok(());
        };
        let Some(callback_copy) = self.domain.try_copy(
            lease.texture(),
            &source,
            u64::try_from(descriptor.byte_len()).expect("validated frame byte length fits u64"),
            self.stream.get(),
        )?
        else {
            let _drop = self.state.try_record_drop();
            return Ok(());
        };

        // CopyResource has been issued and the private texture owns the future
        // GPU result. Release the WGC frame before constructing or publishing a
        // public storage owner.
        drop(source);
        drop(surface);
        drop(frame);

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
            Arc::clone(&self.device_terminal),
            Arc::clone(&self.memory),
        );
        self.state
            .publish_storage_with(
                StoragePublication {
                    captured_at,
                    placement: Some(placement),
                    storage,
                    continuity,
                },
                move |frame| {
                    self.geometry.publish(frame);
                    callback_copy.publish(frame.stamp());
                },
            )
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

fn normalize_native_fault(fault: CaptureFault, kind: TargetKind) -> CaptureFault {
    match fault {
        CaptureFault::CaptureItemClosed if kind == TargetKind::Display => {
            CaptureFault::DisplayDisconnected
        }
        CaptureFault::DisplayDisconnected if kind == TargetKind::Window => {
            CaptureFault::CaptureItemClosed
        }
        _ => fault,
    }
}

pub(crate) fn native_target_fault(error: windows::core::Error, kind: TargetKind) -> CaptureFault {
    normalize_native_fault(native_fault(error), kind)
}

fn native_close_result(result: windows::core::Result<()>) -> std::result::Result<(), CaptureFault> {
    match result.map_err(native_fault) {
        Err(
            CaptureFault::CaptureItemClosed
            | CaptureFault::DisplayDisconnected
            | CaptureFault::ExplicitlyStopped,
        ) => Ok(()),
        result => result,
    }
}

const fn target_fault(kind: TargetKind) -> CaptureFault {
    match kind {
        TargetKind::Window => CaptureFault::CaptureItemClosed,
        TargetKind::Display => CaptureFault::DisplayDisconnected,
        _ => CaptureFault::TargetLost,
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
    validate_surface(extent.width(), extent.height())?;
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
    validate_surface(width, height)?;
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

fn clock_calibration() -> Option<(i64, MonotonicInstant)> {
    let before = SystemClock.now();
    let native = native_monotonic_now()?;
    let after = SystemClock.now();
    let midpoint = before.checked_add(after.saturating_duration_since(before) / 2)?;
    Some((native, midpoint))
}

fn frame_time(transition: &TransitionState, native_time: i64) -> MonotonicInstant {
    let (native_origin, mado_origin) = transition.clock_anchor;
    let elapsed_ticks = i128::from(native_time) - i128::from(native_origin);
    let elapsed_nanos = elapsed_ticks.unsigned_abs().saturating_mul(100);
    let elapsed = u64::try_from(elapsed_nanos)
        .map(Duration::from_nanos)
        .unwrap_or(Duration::MAX);
    if elapsed_ticks >= 0 {
        mado_origin
            .checked_add(elapsed)
            .unwrap_or_else(|| MonotonicInstant::from_origin(Duration::MAX))
    } else {
        MonotonicInstant::from_origin(mado_origin.since_origin().saturating_sub(elapsed))
    }
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
    use std::io;
    use std::mem::ManuallyDrop;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::thread;
    use std::time::{Duration, Instant};

    use mado_pilot_capture::{
        CaptureFault, Continuity, CoordinateSupport, CpuPixels, FrameDescriptor, FrameRequest,
        OverflowPolicy, PixelFormat, Publication, RetainedStoragePolicy, SessionDescription,
        StreamState,
    };
    use mado_pilot_core::{
        CancellationToken, IdentityIssuer, MonotonicInstant, Operation, OperationContext,
        PixelExtent, ProviderId, Scale, Status, TargetPlacement,
    };
    use windows::Graphics::SizeInt32;
    use windows::Win32::Foundation::RO_E_CLOSED;

    use crate::input::GeometryLedger;
    use crate::storage::{
        GLOBAL_RETAINED_BYTES, SESSION_RETAINED_BYTES, SessionMemory, retained_storage_capacity,
        validate_surface,
    };

    use super::{
        CallbackControl, GeometryRegistration, NativeOwnership, TEARDOWN_QUEUE_CAPACITY,
        TEARDOWN_WORKER_COUNT, TeardownExecutorSlot, TeardownPermits, TransitionState,
        WGC_PRODUCER_POOL_SIZE, capture_already_ended_after_drain, frame_time, map_worker_start,
        native_close_result, native_size, normalize_native_fault, positive_extent,
        record_authoritative_native_end, session_queue_policy, start_teardown_executor_with,
        target_fault, teardown_channel, teardown_executor_from_slot,
    };

    static STALLED_INITIALIZERS: AtomicUsize = AtomicUsize::new(0);
    static RELEASE_INITIALIZERS: AtomicBool = AtomicBool::new(false);

    struct NativeDropProbe {
        memory: Arc<SessionMemory>,
        observed: Arc<AtomicBool>,
        expected: u64,
    }

    impl Drop for NativeDropProbe {
        fn drop(&mut self) {
            assert_eq!(
                self.memory.usage(),
                (self.expected, self.expected),
                "producer bytes remain charged until the native payload drops"
            );
            self.observed.store(true, Ordering::Release);
        }
    }

    #[test]
    fn r1_2_initial_pool_and_resize_validate_shape_before_native_allocation() {
        native_size(PixelExtent::new(8192, 4096)).expect("exact surface ceiling");
        assert_eq!(
            native_size(PixelExtent::new(8192, 4097))
                .expect_err("initial frame pool is refused before CreateFreeThreaded")
                .status(),
            Status::LimitExceeded
        );
        assert_eq!(
            positive_extent(SizeInt32 {
                Width: 8192,
                Height: 4097,
            }),
            Err(CaptureFault::ResourceLimitExceeded),
            "resize is refused before FramePool::Recreate"
        );
    }

    #[test]
    fn r1_2_producer_reservation_stays_with_queued_native_ownership_until_close() {
        let memory = SessionMemory::testing_isolated(512, 512);
        let native_dropped = Arc::new(AtomicBool::new(false));
        let owner = NativeOwnership::new(
            NativeDropProbe {
                memory: Arc::clone(&memory),
                observed: Arc::clone(&native_dropped),
                expected: 128,
            },
            memory.reserve(128).expect("initial producer reservation"),
        );
        let closed_session_link = Arc::downgrade(&owner);
        let (sender, queued) = mpsc::sync_channel(1);

        sender
            .try_send(owner)
            .expect("teardown queue accepts owner");
        assert_eq!(
            memory.usage(),
            (128, 128),
            "queued native resources keep both counters charged"
        );

        let closing = queued.recv().expect("worker receives native owner");
        drop(closing);
        assert_eq!(
            memory.usage(),
            (0, 0),
            "native close releases producer bytes even while the closed core link remains"
        );
        assert!(closed_session_link.upgrade().is_none());
        assert!(
            native_dropped.load(Ordering::Acquire),
            "the queued native payload actually closed before its charge released"
        );
    }

    #[test]
    fn r1_2_producer_reservation_stays_charged_in_quarantine() {
        let memory = SessionMemory::testing_isolated(512, 512);
        let owner = NativeOwnership::new(
            (),
            memory.reserve(192).expect("initial producer reservation"),
        );
        let quarantined = ManuallyDrop::new(owner);

        assert_eq!(
            memory.usage(),
            (192, 192),
            "quarantined native ownership must not undercount live resources"
        );

        // Production quarantine is process-lifetime. Recover only so this
        // isolated test can prove the final native-owner release is exact.
        drop(ManuallyDrop::into_inner(quarantined));
        assert_eq!(memory.usage(), (0, 0));
    }

    #[test]
    fn r1_2_resize_replaces_producer_reservation_only_after_native_success() {
        let memory = SessionMemory::testing_isolated(512, 512);
        let owner = NativeOwnership::new(
            (),
            memory.reserve(64).expect("initial producer reservation"),
        );

        let held = owner
            .producer_bytes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let replacement = memory.reserve(96).expect("contended replacement");
        let contention = owner.replace_producer_after_recreate(replacement, || {
            panic!("contended replacement must refuse before native recreation")
        });
        assert_eq!(contention, Err(CaptureFault::SourceInvalid));
        drop(held);
        assert_eq!(
            memory.usage(),
            (64, 64),
            "non-blocking contention keeps only the current producer lease"
        );

        let replacement = memory.reserve(96).expect("replacement is pre-reserved");
        let failure = owner.replace_producer_after_recreate(replacement, || {
            assert_eq!(
                memory.usage(),
                (160, 160),
                "old and replacement reservations cover native recreation"
            );
            Err(CaptureFault::SourceInvalid)
        });
        assert_eq!(failure, Err(CaptureFault::SourceInvalid));
        assert_eq!(
            memory.usage(),
            (64, 64),
            "failed recreation keeps the old owner and retires the replacement"
        );

        let replacement = memory.reserve(96).expect("replacement retry");
        owner
            .replace_producer_after_recreate(replacement, || {
                assert_eq!(memory.usage(), (160, 160));
                Ok(())
            })
            .expect("recreation succeeds");
        assert_eq!(
            memory.usage(),
            (96, 96),
            "success retires exactly the old reservation"
        );

        drop(owner);
        assert_eq!(memory.usage(), (0, 0));
    }

    #[test]
    fn r1_3_declared_latest_wins_matches_two_publications_before_observation() {
        let issuer = IdentityIssuer::new();
        let state = StreamState::with_target_extent(issuer.issue_stream().expect("stream"));
        let descriptor = FrameDescriptor::packed(PixelExtent::new(2, 2), PixelFormat::Bgra8)
            .expect("descriptor");
        let publish = |fill| Publication {
            captured_at: MonotonicInstant::ORIGIN,
            descriptor,
            placement: None,
            pixels: vec![fill; descriptor.byte_len()].into_boxed_slice(),
            continuity: Continuity::Continuous,
        };

        let first = state.publish(publish(0x11)).expect("first publication");
        let second = state.publish(publish(0x22)).expect("second publication");
        let observed = state
            .frame(&FrameRequest::latest(), &OperationContext::new())
            .expect("latest frame");

        assert_eq!(
            session_queue_policy(std::num::NonZeroU32::MIN).overflow(),
            OverflowPolicy::LatestWins
        );
        assert_eq!(observed.stamp(), second.stamp());
        assert_eq!(
            second.stamp().sequence().value(),
            first.stamp().sequence().value() + 1,
            "supersession itself invents no producer-drop gap"
        );
        assert_eq!(
            observed
                .map(PixelFormat::Bgra8, &OperationContext::new())
                .expect("mapping")
                .bytes(),
            &[0x22; 16]
        );
    }

    #[test]
    fn r1_2_production_multi_session_description_matches_shared_pressure_and_resume() {
        let layout = validate_surface(3840, 2160).expect("4K layout");
        let surface_bytes = layout.bytes();
        let capacity = retained_storage_capacity(layout).expect("4K retained capacity");
        assert_eq!(capacity.get(), 40);
        let producer_bytes = surface_bytes
            .checked_mul(u64::try_from(WGC_PRODUCER_POOL_SIZE).expect("positive producer count"))
            .expect("producer bytes");
        let memories =
            SessionMemory::testing_shared(SESSION_RETAINED_BYTES, GLOBAL_RETAINED_BYTES, 3);
        let first_producer = memories[0]
            .reserve(producer_bytes)
            .expect("first session admitted");
        let second_producer = memories[1]
            .reserve(producer_bytes)
            .expect("second session admitted");
        let third_producer = memories[2]
            .reserve(producer_bytes)
            .expect("third session admitted");
        let issuer = IdentityIssuer::new();
        let descriptions: Vec<_> = (0..3)
            .map(|_| {
                SessionDescription::new(
                    issuer
                        .issue_target(ProviderId::new("windows"))
                        .expect("target"),
                    issuer.issue_stream().expect("stream"),
                    PixelExtent::new(3840, 2160),
                    PixelFormat::Bgra8,
                    CoordinateSupport::with_target_placement(),
                )
                .with_queue(session_queue_policy(capacity))
            })
            .collect();
        for description in &descriptions {
            assert_eq!(description.queue().retained_storage(), Some(capacity));
            assert_eq!(
                description.queue().retained_storage_policy(),
                Some(RetainedStoragePolicy::ProcessShared),
                "every admitted session reports that its local maximum shares process backing"
            );
        }

        let state = StreamState::with_target_extent(descriptions[2].stream());
        let descriptor = FrameDescriptor::packed(PixelExtent::new(2, 2), PixelFormat::Bgra8)
            .expect("descriptor");
        let publish = |descriptor, fill, continuity| Publication {
            captured_at: MonotonicInstant::ORIGIN,
            descriptor,
            placement: None,
            pixels: vec![fill; descriptor.byte_len()].into_boxed_slice(),
            continuity,
        };
        let first_fill = memories[0]
            .reserve(SESSION_RETAINED_BYTES - producer_bytes)
            .expect("first session reaches its exact local ceiling");
        let zero_retained_fill_bytes = GLOBAL_RETAINED_BYTES
            .checked_sub(SESSION_RETAINED_BYTES)
            .and_then(|remaining| remaining.checked_sub(producer_bytes * 2))
            .expect("remaining process budget");
        let zero_retained_fill = memories[1]
            .reserve(zero_retained_fill_bytes)
            .expect("other sessions consume the exact remaining process budget");
        assert_eq!(memories[2].usage().1, GLOBAL_RETAINED_BYTES);

        assert_eq!(
            memories[2]
                .reserve(surface_bytes)
                .expect_err("shared pressure can refuse below the local count")
                .status(),
            Status::LimitExceeded
        );
        assert!(
            !state
                .try_record_drop()
                .expect("pre-publication pressure is bounded"),
            "before the first frame there is no identity on which to expose a gap"
        );

        drop(zero_retained_fill);
        let third_first_storage = memories[2]
            .reserve(surface_bytes)
            .expect("global release admits the session's first retained allocation");
        let first = state
            .publish(publish(descriptor, 0x31, Continuity::Continuous))
            .expect("first publication");

        let after_first_fill_bytes = GLOBAL_RETAINED_BYTES
            .checked_sub(SESSION_RETAINED_BYTES)
            .and_then(|remaining| remaining.checked_sub(producer_bytes * 2 + surface_bytes))
            .expect("remaining process budget after the first storage");
        let after_first_fill = memories[1]
            .reserve(after_first_fill_bytes)
            .expect("other sessions refill the process budget");
        assert_eq!(memories[2].usage().1, GLOBAL_RETAINED_BYTES);
        assert_eq!(
            memories[2]
                .reserve(surface_bytes)
                .expect_err("shared pressure refuses another below-local-count allocation")
                .status(),
            Status::LimitExceeded
        );
        assert!(
            state.try_record_drop().expect("pressure drop recorded"),
            "a prior publication makes process pressure observable"
        );

        drop(after_first_fill);
        let resumed_storage = memories[2]
            .reserve(surface_bytes)
            .expect("global release resumes the contended session at a resize");
        let resized_descriptor =
            FrameDescriptor::packed(PixelExtent::new(3, 2), PixelFormat::Bgra8)
                .expect("resized descriptor");
        let discontinuous = state
            .publish(publish(resized_descriptor, 0x32, Continuity::Discontinuous))
            .expect("resized publication");
        assert_eq!(
            discontinuous.stamp().sequence().value(),
            0,
            "a resized epoch still starts at FIRST"
        );
        assert_ne!(
            discontinuous.stamp().epoch(),
            first.stamp().epoch(),
            "the resize advances the stream epoch"
        );

        drop(third_first_storage);
        let pressure_visible_storage = memories[2]
            .reserve(surface_bytes)
            .expect("old-storage release admits the next same-size frame");
        let pressure_visible = state
            .publish(publish(resized_descriptor, 0x33, Continuity::Continuous))
            .expect("continuous publication after the resize");
        assert_eq!(
            pressure_visible.stamp().sequence().value(),
            discontinuous.stamp().sequence().value() + 2,
            "the pre-resize shared-pressure refusal remains visible after FIRST"
        );

        drop(pressure_visible_storage);
        drop(resumed_storage);
        drop(first_fill);
        drop(first_producer);
        drop(second_producer);
        drop(third_producer);
        assert_eq!(memories[0].usage().1, 0);
    }

    #[test]
    fn r1_2_process_shared_mapping_stays_charged_after_session_close_until_pixels_release() {
        let layout = validate_surface(3840, 2160).expect("4K layout");
        let mapping_bytes = layout.bytes().checked_mul(2).expect("staging plus output");
        let mut memories =
            SessionMemory::testing_shared(SESSION_RETAINED_BYTES, GLOBAL_RETAINED_BYTES, 4);
        let source = memories.remove(0);
        let mapping = source
            .reserve(mapping_bytes)
            .expect("mapping admitted before close");
        let retainer: Arc<dyn Send + Sync> = Arc::new(mapping);
        let pixels = Arc::new(CpuPixels::with_retainer(
            vec![0; 1].into_boxed_slice(),
            retainer,
        ));
        drop(source);

        let first_peer = memories[0]
            .reserve(SESSION_RETAINED_BYTES)
            .expect("first peer reaches its local ceiling");
        let second_peer = memories[1]
            .reserve(GLOBAL_RETAINED_BYTES - SESSION_RETAINED_BYTES - mapping_bytes)
            .expect("mapping plus peers reaches the exact global ceiling");
        assert_eq!(memories[2].usage().1, GLOBAL_RETAINED_BYTES);
        assert_eq!(
            memories[2]
                .reserve(1)
                .expect_err("closed-session pixels still consume shared backing")
                .status(),
            Status::LimitExceeded
        );

        drop(pixels);
        let resumed = memories[2]
            .reserve(1)
            .expect("final mapped-pixel release resumes another session");
        drop(resumed);
        drop(second_peer);
        drop(first_peer);
        assert_eq!(memories[0].usage().1, 0);
    }

    fn stalled_teardown_initializer() -> Result<(), CaptureFault> {
        STALLED_INITIALIZERS.fetch_add(1, Ordering::AcqRel);
        while !RELEASE_INITIALIZERS.load(Ordering::Acquire) {
            thread::yield_now();
        }
        Ok(())
    }

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
    fn native_target_faults_are_normalized_by_target_kind() {
        assert_eq!(
            normalize_native_fault(
                CaptureFault::CaptureItemClosed,
                mado_pilot_core::TargetKind::Display,
            ),
            CaptureFault::DisplayDisconnected
        );
        assert_eq!(
            normalize_native_fault(
                CaptureFault::CaptureItemClosed,
                mado_pilot_core::TargetKind::Window,
            ),
            CaptureFault::CaptureItemClosed
        );
        assert_eq!(
            normalize_native_fault(
                CaptureFault::DisplayDisconnected,
                mado_pilot_core::TargetKind::Window,
            ),
            CaptureFault::CaptureItemClosed
        );
        assert_eq!(
            normalize_native_fault(
                CaptureFault::DisplayDisconnected,
                mado_pilot_core::TargetKind::Display,
            ),
            CaptureFault::DisplayDisconnected
        );
        assert_eq!(
            normalize_native_fault(
                CaptureFault::ExplicitlyStopped,
                mado_pilot_core::TargetKind::Window,
            ),
            CaptureFault::ExplicitlyStopped
        );
        assert_eq!(
            normalize_native_fault(
                CaptureFault::DeviceRemoved,
                mado_pilot_core::TargetKind::Window,
            ),
            CaptureFault::DeviceRemoved
        );
        assert_eq!(
            target_fault(mado_pilot_core::TargetKind::Window),
            CaptureFault::CaptureItemClosed
        );
        assert_eq!(
            target_fault(mado_pilot_core::TargetKind::Display),
            CaptureFault::DisplayDisconnected
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
    fn geometry_membership_outlives_an_admitted_callback_and_is_removed_last() {
        let stream = IdentityIssuer::new().issue_stream().expect("issued stream");
        let state = StreamState::with_target_extent(stream);
        let ledger = Arc::new(GeometryLedger::default());
        let extent = PixelExtent::new(4, 4);
        let descriptor = FrameDescriptor::packed(extent, PixelFormat::Bgra8).expect("descriptor");
        let placement = TargetPlacement::new(
            (10.0, 20.0),
            (4.0, 4.0),
            Scale::new(1.0, 1.0).expect("scale"),
        )
        .expect("placement");
        let frame = state
            .publish(Publication {
                captured_at: MonotonicInstant::ORIGIN,
                descriptor,
                placement: Some(placement),
                pixels: vec![0; descriptor.byte_len()].into_boxed_slice(),
                continuity: Continuity::Continuous,
            })
            .expect("published");
        ledger.publish(&frame);

        let owner = Arc::new(GeometryRegistration::new(Arc::clone(&ledger), stream));
        let callback_owner = Arc::clone(&owner);
        let (admitted_tx, admitted_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let callback = thread::spawn(move || {
            admitted_tx.send(()).expect("callback admitted");
            release_rx.recv().expect("callback released");
            drop(callback_owner);
        });
        admitted_rx.recv().expect("callback owns registration");

        drop(owner);
        assert!(
            ledger.source_transform(frame.stamp()).is_some(),
            "teardown owner removal cannot outrun the admitted callback"
        );
        release_tx.send(()).expect("finish callback");
        callback.join().expect("callback joined");
        assert_eq!(
            ledger.source_transform(frame.stamp()),
            None,
            "the final owner removes the dead stream after callback completion"
        );
    }

    #[test]
    fn native_end_state_is_sampled_after_the_admitted_callback_drain() {
        let control = Arc::new(CallbackControl::default());
        let native_ended = Arc::new(AtomicBool::new(false));
        let lease = control.admit().expect("callback admitted");
        control.stop_admission();
        let worker = {
            let control = Arc::clone(&control);
            let native_ended = Arc::clone(&native_ended);
            thread::spawn(move || capture_already_ended_after_drain(true, &control, &native_ended))
        };

        assert!(
            !worker.is_finished(),
            "the admitted callback holds the drain"
        );
        record_authoritative_native_end(&native_ended);
        drop(lease);
        assert!(
            worker.join().expect("drain worker"),
            "the worker reads native termination only after the callback finishes"
        );
    }

    #[test]
    fn authoritative_native_end_latches_after_owner_admission_stops() {
        let control = Arc::new(CallbackControl::default());
        let native_ended = AtomicBool::new(false);
        control.stop_admission();

        record_authoritative_native_end(&native_ended);

        assert!(control.admit().is_none());
        assert!(capture_already_ended_after_drain(
            false,
            &control,
            &native_ended
        ));
    }

    #[test]
    fn native_close_absorbs_an_already_closed_result() {
        let result = native_close_result(Err(windows::core::Error::from_hresult(RO_E_CLOSED)));

        result.expect("closing an already closed native owner is idempotent");
    }

    #[test]
    fn teardown_start_reports_thread_creation_failure_without_panicking() {
        let result = map_worker_start(Err(io::Error::other("injected worker creation failure")));

        assert_eq!(result.err(), Some(CaptureFault::SourceInvalid));
    }

    #[test]
    fn teardown_start_reports_apartment_initialization_failure() {
        let context = OperationContext::new();
        let mut operation = Operation::admit(&context).expect("operation admitted");
        let result =
            start_teardown_executor_with(|| Err(CaptureFault::UnsupportedOption), &mut operation);

        assert_eq!(
            result.err().expect("apartment failure").status(),
            Status::Unsupported
        );
    }

    #[test]
    fn teardown_executor_starts_only_the_fixed_worker_count() {
        let context = OperationContext::new();
        let mut operation = Operation::admit(&context).expect("operation admitted");
        let executor =
            start_teardown_executor_with(|| Ok(()), &mut operation).expect("executor starts");

        assert_eq!(
            executor.live_workers.load(Ordering::Acquire),
            TEARDOWN_WORKER_COUNT
        );
    }

    #[test]
    fn teardown_start_observes_operation_cancellation_before_spawning() {
        let cancellation = CancellationToken::new();
        let context = OperationContext::new().with_cancellation(cancellation.clone());
        let mut operation = Operation::admit(&context).expect("operation admitted");
        cancellation.cancel();

        let error = start_teardown_executor_with(|| Ok(()), &mut operation)
            .err()
            .expect("cancelled startup");

        assert_eq!(error.status(), Status::Cancelled);
    }

    #[test]
    fn cancelled_waiters_share_one_in_flight_teardown_generation() {
        STALLED_INITIALIZERS.store(0, Ordering::Release);
        RELEASE_INITIALIZERS.store(false, Ordering::Release);
        let slot = Mutex::new(TeardownExecutorSlot::Empty);

        for attempt in 0..2 {
            let cancellation = CancellationToken::new();
            let context = OperationContext::new().with_cancellation(cancellation.clone());
            let mut operation = Operation::admit(&context).expect("operation admitted");
            let cancel = thread::spawn(move || {
                if attempt == 0 {
                    let started = Instant::now();
                    while STALLED_INITIALIZERS.load(Ordering::Acquire) < TEARDOWN_WORKER_COUNT
                        && started.elapsed() < Duration::from_secs(1)
                    {
                        thread::yield_now();
                    }
                } else {
                    thread::sleep(Duration::from_millis(10));
                }
                cancellation.cancel();
            });

            let error =
                teardown_executor_from_slot(&slot, stalled_teardown_initializer, &mut operation)
                    .err()
                    .expect("cancelled startup wait");
            cancel.join().expect("cancellation trigger");
            assert_eq!(error.status(), Status::Cancelled);
            assert_eq!(
                STALLED_INITIALIZERS.load(Ordering::Acquire),
                TEARDOWN_WORKER_COUNT,
                "a cancelled retry must wait on the existing startup generation"
            );
        }

        RELEASE_INITIALIZERS.store(true, Ordering::Release);
        let context = OperationContext::new();
        let mut operation = Operation::admit(&context).expect("operation admitted");
        let executor =
            teardown_executor_from_slot(&slot, stalled_teardown_initializer, &mut operation)
                .expect("the retained generation completes");
        assert_eq!(
            executor.live_workers.load(Ordering::Acquire),
            TEARDOWN_WORKER_COUNT
        );
        assert_eq!(
            STALLED_INITIALIZERS.load(Ordering::Acquire),
            TEARDOWN_WORKER_COUNT
        );
    }

    #[test]
    fn teardown_queue_is_finite_and_non_blocking() {
        let (sender, _jobs) = teardown_channel();
        for _ in 0..TEARDOWN_QUEUE_CAPACITY {
            sender.try_send(()).expect("capacity remains");
        }
        assert!(matches!(
            sender.try_send(()),
            Err(mpsc::TrySendError::Full(()))
        ));
    }

    #[test]
    fn teardown_permits_bound_live_and_queued_session_ownership() {
        let permits = Arc::new(TeardownPermits::new());
        let mut held = Vec::new();
        for _ in 0..TEARDOWN_QUEUE_CAPACITY {
            held.push(permits.try_reserve().expect("permit within bound"));
        }
        assert!(permits.try_reserve().is_none());
        held.pop();
        assert!(permits.try_reserve().is_some());
    }

    #[test]
    fn native_frame_times_use_the_precalibrated_project_clock() {
        let transition = TransitionState {
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
            clock_anchor: (100, MonotonicInstant::from_origin(Duration::from_secs(10))),
            published: false,
        };

        let delayed_first_callback = frame_time(&transition, 100);
        let earlier_frame = frame_time(&transition, 90);
        let second = frame_time(&transition, 130);

        assert_eq!(
            delayed_first_callback.since_origin(),
            Duration::from_secs(10)
        );
        assert_eq!(
            delayed_first_callback.saturating_duration_since(earlier_frame),
            Duration::from_micros(1)
        );
        assert_eq!(
            second.saturating_duration_since(delayed_first_callback),
            Duration::from_micros(3)
        );
    }
}
