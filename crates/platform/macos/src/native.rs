//! Session ownership, callback fencing, transitions, and bounded teardown.
//!
//! # What the producer callback does
//!
//! Admission, same-frame metadata validation, and a bounded detach into one staged
//! slot. Native delivery invokes a second contained callback to publish only after
//! every remaining throwing native frame step has completed. No fresh inventory or
//! native wait runs per frame. CPU conversion, matching, input, host callbacks, and
//! native reconfiguration never run in either producer callback.
//!
//! # Lock order
//!
//! Callback admission inside the shim, then this module's transition state, then
//! the detached-storage pool inside the shim, then the stream's own state. No host
//! callback is invoked while any of them is held.

use std::ffi::c_void;
use std::fmt;
use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, TryLockError, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use mado_pilot_capture::{
    CaptureFault, CaptureSession, Continuity, CoordinateSupport, Frame, FrameRequest, Lifecycle,
    OverflowPolicy, PixelFormat, QueuePolicy, SessionDescription, StoragePublication, StreamState,
};
use mado_pilot_core::{
    Clock, MonotonicInstant, Operation, OperationContext, PixelExtent, Result, StreamId,
    SystemClock, TargetId, TargetKind, TargetPlacement,
};

use crate::discovery::{Fingerprint, NativeKey, TargetMetadata, frame_placement};
use crate::input::GeometryLedger;
use crate::shim::{
    self, BorrowedFrame, DEFAULT_NATIVE_WAIT, DetachedFrame, FrameInfo, MAX_NATIVE_WAIT,
    OpenRequest, ShimStatus, TargetToken,
};
use crate::storage::{DETACHED_BUFFER_BUDGET, MacosFrameStorage, descriptor_from_native};

/// Producer queue depth. Three is the shim's floor and what the framework
/// recommends: deep enough that one slow work item does not starve delivery,
/// shallow enough that a stalled consumer cannot accumulate stale surfaces.
const PRODUCER_QUEUE_DEPTH: u32 = 3;

/// How long a caller contending for the close gate sleeps between attempts.
const CLOSE_POLL_INTERVAL: Duration = Duration::from_millis(2);

#[derive(Debug, Default)]
struct CloseGate {
    state: Mutex<CloseGateState>,
    idle: Condvar,
}

#[derive(Debug, Default)]
struct CloseGateState {
    owner: Option<thread::ThreadId>,
    entering: Vec<thread::ThreadId>,
}

#[derive(Debug)]
struct CloseOwner<'gate> {
    gate: &'gate CloseGate,
    thread: thread::ThreadId,
    _not_send: PhantomData<Rc<()>>,
}

#[derive(Debug)]
struct CloseEntry<'gate> {
    gate: &'gate CloseGate,
    thread: thread::ThreadId,
    _not_send: PhantomData<Rc<()>>,
}

impl CloseGate {
    fn enter(&self, operation: &OperationContext) -> Result<CloseOwner<'_>> {
        let thread = thread::current().id();
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.owner == Some(thread) || state.entering.contains(&thread) {
                /*
                 * Caller clocks may synchronously re-enter close. The outer
                 * call will finish the same idempotent teardown, but the
                 * nested call cannot report success before that teardown has
                 * actually completed.
                 */
                return Err(CaptureFault::SessionClosed.into());
            }
            state.entering.push(thread);
        }
        let _entry = CloseEntry {
            gate: self,
            thread,
            _not_send: PhantomData,
        };
        let mut attempt = Operation::admit(operation)?;
        loop {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match state.owner {
                None => {
                    state.owner = Some(thread);
                    drop(state);
                    return Ok(attempt.commit(CloseOwner {
                        gate: self,
                        thread,
                        _not_send: PhantomData,
                    })?);
                }
                Some(owner) if owner == thread => {
                    unreachable!("the entry marker caught same-thread close reentry")
                }
                Some(_) => {
                    let (state, _timed) = self
                        .idle
                        .wait_timeout(state, CLOSE_POLL_INTERVAL)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    drop(state);
                    // Caller code runs only after the gate mutex is released.
                    attempt.checkpoint()?;
                }
            }
        }
    }
}

impl Drop for CloseEntry<'_> {
    fn drop(&mut self) {
        let mut state = self
            .gate
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let index = state
            .entering
            .iter()
            .position(|thread| *thread == self.thread)
            .expect("a live close entry remains registered");
        state.entering.swap_remove(index);
    }
}

impl Drop for CloseOwner<'_> {
    fn drop(&mut self) {
        let mut state = self
            .gate
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        debug_assert_eq!(state.owner, Some(self.thread));
        state.owner = None;
        drop(state);
        self.gate.idle.notify_all();
    }
}

#[cfg(test)]
static TESTING_DELAYED_CALLBACK_ACTIVE: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
pub(crate) fn testing_delayed_callback_is_active() -> bool {
    TESTING_DELAYED_CALLBACK_ACTIVE.load(Ordering::Acquire)
}

pub(crate) struct NativeSession {
    description: SessionDescription,
    core: Arc<SessionCore>,
    /// The strong reference the shim holds as its callback context.
    ///
    /// Reclaimed only after a successful fence proves no callback can reach it.
    registered: *const SessionCore,
    close_gate: CloseGate,
    close_reported: AtomicBool,
}

/// Everything selected discovery established for one native session open.
pub(crate) struct SessionTarget {
    target: TargetId,
    stream: StreamId,
    key: NativeKey,
    fingerprint: Fingerprint,
    selection: TargetToken,
    metadata: TargetMetadata,
    /// Where each published frame's authoritative transform is recorded, so a
    /// later input request can resolve a coordinate against the frame it came
    /// from rather than against whatever the target looks like now.
    geometry: Arc<GeometryLedger>,
}

impl SessionTarget {
    pub(crate) fn new(
        target: TargetId,
        stream: StreamId,
        key: NativeKey,
        fingerprint: Fingerprint,
        selection: TargetToken,
        metadata: TargetMetadata,
        geometry: Arc<GeometryLedger>,
    ) -> Self {
        Self {
            target,
            stream,
            key,
            fingerprint,
            selection,
            metadata,
            geometry,
        }
    }
}

// SAFETY: `registered` is an `Arc::into_raw` pointer that is only read for its
// address and only consumed in `Drop`. Everything it refers to is `Send`, and the
// pointer itself is never dereferenced outside the shim callbacks, which hold
// their own strong reference through it.
unsafe impl Send for NativeSession {}
// SAFETY: see the Send justification.
unsafe impl Sync for NativeSession {}

/// One stream's entry in the target's geometry ledger, retired when it ends.
///
/// The registration rather than the ledger itself, so an entry cannot outlive the
/// session that published it whether the caller closed explicitly or dropped the
/// last reference.
struct GeometryRegistration {
    ledger: Arc<GeometryLedger>,
    stream: StreamId,
}

impl GeometryRegistration {
    fn new(ledger: Arc<GeometryLedger>, stream: StreamId) -> Self {
        Self { ledger, stream }
    }

    fn publish(&self, frame: &Frame, native_bounds: shim::NativeBounds) {
        self.ledger.publish(frame, native_bounds);
    }
}

impl Drop for GeometryRegistration {
    fn drop(&mut self) {
        self.ledger.remove(self.stream);
    }
}

struct SessionCore {
    target_kind: TargetKind,
    geometry: GeometryRegistration,
    state: StreamState,
    session: OnceLock<shim::Session>,
    pending_frame: PendingSlot<PendingFrame>,
    transition: Mutex<TransitionState>,
    reconfigure: Arc<Reconfigure>,
    clock_anchor: (u64, MonotonicInstant),
    #[cfg(test)]
    testing_sites: u32,
    #[cfg(test)]
    terminal_reports: AtomicU64,
}

/// What the stream last received from this Adapter.
///
/// Continuity is decided against this rather than against a flag accumulated from
/// intermediate observations. Every value comes from a frame that was actually
/// published; a later inventory snapshot never enters this comparison.
#[derive(Debug, Clone, Copy)]
struct Published {
    extent: PixelExtent,
    placement: TargetPlacement,
    native_bounds: shim::NativeBounds,
}

#[derive(Debug, Clone, Copy)]
struct TransitionState {
    /// The content extent the last frame was observed at.
    extent: PixelExtent,
    /// The producer surface extent the last frame was observed in.
    surface: PixelExtent,
    published: Option<Published>,
}

struct PendingFrame {
    detached: DetachedFrame,
    info: FrameInfo,
}

/// The one detached frame native delivery has staged but not yet authorized.
struct PendingSlot<T> {
    value: Mutex<Option<T>>,
}

impl<T> Default for PendingSlot<T> {
    fn default() -> Self {
        Self {
            value: Mutex::new(None),
        }
    }
}

impl<T> PendingSlot<T> {
    /// Stages without waiting on the sample queue. Full or contended stays finite.
    fn try_stage(&self, value: T) -> std::result::Result<(), T> {
        let mut slot = match self.value.try_lock() {
            Ok(slot) => slot,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return Err(value),
        };
        if slot.is_some() {
            return Err(value);
        }
        *slot = Some(value);
        Ok(())
    }

    fn take(&self) -> Option<T> {
        self.value
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }

    fn clear(&self) {
        drop(self.take());
    }
}

/// The reconfiguration request a producer callback leaves for its worker.
#[derive(Debug)]
struct Reconfigure {
    wanted: AtomicU64,
    shutdown: AtomicBool,
    wake: SyncSender<()>,
    worker: Mutex<ReconfigureWorker>,
    finished: Condvar,
    coalesced: AtomicU64,
    rejected: AtomicU64,
}

#[derive(Debug, Default)]
struct ReconfigureWorker {
    handle: Option<JoinHandle<()>>,
    finished: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReconfigurePublication {
    Published,
    Coalesced,
    Rejected,
}

impl Reconfigure {
    fn new() -> (Arc<Self>, Receiver<()>) {
        let (wake, receiver) = mpsc::sync_channel(1);
        (
            Arc::new(Self {
                wanted: AtomicU64::new(0),
                shutdown: AtomicBool::new(false),
                wake,
                worker: Mutex::new(ReconfigureWorker {
                    handle: None,
                    finished: true,
                }),
                finished: Condvar::new(),
                coalesced: AtomicU64::new(0),
                rejected: AtomicU64::new(0),
            }),
            receiver,
        )
    }

    /// Publishes a latest-wins request without taking any mutex.
    fn request(&self, extent: PixelExtent) -> ReconfigurePublication {
        if self.shutdown.load(Ordering::Acquire) {
            self.rejected.fetch_add(1, Ordering::AcqRel);
            return ReconfigurePublication::Rejected;
        }
        let encoded = encode_extent(extent);
        let previous = self.wanted.swap(encoded, Ordering::AcqRel);
        if self.shutdown.load(Ordering::Acquire) {
            self.wanted.store(0, Ordering::Release);
            self.rejected.fetch_add(1, Ordering::AcqRel);
            return ReconfigurePublication::Rejected;
        }
        match self.wake.try_send(()) {
            Ok(()) | Err(TrySendError::Full(())) => {}
            Err(TrySendError::Disconnected(())) => {
                self.shutdown.store(true, Ordering::Release);
                self.wanted.store(0, Ordering::Release);
                self.rejected.fetch_add(1, Ordering::AcqRel);
                return ReconfigurePublication::Rejected;
            }
        }
        if previous == 0 {
            ReconfigurePublication::Published
        } else {
            self.coalesced.fetch_add(1, Ordering::AcqRel);
            ReconfigurePublication::Coalesced
        }
    }

    fn shutdown(&self) {
        if !self.shutdown.swap(true, Ordering::AcqRel) {
            if self.wanted.swap(0, Ordering::AcqRel) != 0 {
                self.rejected.fetch_add(1, Ordering::AcqRel);
            }
            let _wake = self.wake.try_send(());
        }
    }

    fn worker(&self) -> MutexGuard<'_, ReconfigureWorker> {
        self.worker
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn prepare_worker(&self) {
        self.worker().finished = false;
    }

    fn install_worker(&self, handle: JoinHandle<()>) {
        self.worker().handle = Some(handle);
    }

    fn worker_finished(&self) {
        self.worker().finished = true;
        self.finished.notify_all();
    }

    fn drain(&self, operation: &OperationContext) -> Result<()> {
        self.shutdown();
        let mut attempt = Operation::admit(operation)?;
        let handle = loop {
            let mut worker = self.worker();
            if worker.finished {
                break worker.handle.take();
            }
            let (worker, _timed) = self
                .finished
                .wait_timeout(worker, CLOSE_POLL_INTERVAL)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            drop(worker);
            attempt.checkpoint()?;
        };
        if let Some(handle) = handle {
            handle
                .join()
                .map_err(|_| mado_pilot_core::Error::from(CaptureFault::SourceInvalid))?;
        }
        Ok(attempt.commit(())?)
    }

    fn drain_for_drop(&self, wait: Duration) -> bool {
        self.shutdown();
        let deadline = Instant::now() + wait;
        let handle = loop {
            let mut worker = self.worker();
            if worker.finished {
                break worker.handle.take();
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let left = deadline.saturating_duration_since(now);
            let (worker, _timed) = self
                .finished
                .wait_timeout(worker, left.min(CLOSE_POLL_INTERVAL))
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            drop(worker);
        };
        handle.is_none_or(|handle| handle.join().is_ok())
    }
}

const fn encode_extent(extent: PixelExtent) -> u64 {
    (extent.width() as u64) << 32 | extent.height() as u64
}

fn decode_extent(encoded: u64) -> Option<PixelExtent> {
    let width = u32::try_from(encoded >> 32).ok()?;
    let height = u32::try_from(encoded & u64::from(u32::MAX)).ok()?;
    (width > 0 && height > 0).then(|| PixelExtent::new(width, height))
}

/// Owns the callback registration for the window in which `open` can still fail.
///
/// [`Arc::into_raw`] hands the shim a strong reference that only
/// [`NativeSession::drop`] reclaims, and two exits sit between that hand-off and a
/// `NativeSession` existing to perform it. Either one leaked the session core with a
/// live capture inside it, so a caller that gave up on a slow open was told the open
/// failed while its screen was still being captured by something nothing could reach.
///
/// So the window gets an owner, and it performs the same teardown
/// [`NativeSession::drop`] does — including the same rule about when the reference may
/// be reclaimed at all.
///
/// The core is borrowed rather than held. The guard's whole life is one call, and an
/// owned clone would have to be released by `into_owned` on the success path — which
/// `ManuallyDrop` cannot do, because suppressing the drop suppresses the fields with
/// it. That was measured: a core leaked on every successful open, and through it the
/// frame the stream still held, so a contained failure at teardown then reported a
/// native object left alive.
struct PendingRegistration<'core> {
    core: &'core Arc<SessionCore>,
    registered: *const SessionCore,
}

impl<'core> PendingRegistration<'core> {
    /// Hands a strong reference to the shim and takes responsibility for it.
    fn new(core: &'core Arc<SessionCore>) -> Self {
        Self {
            core,
            // The shim keeps this address until a fence proves no callback holds it.
            registered: Arc::into_raw(Arc::clone(core)),
        }
    }

    fn context(&self) -> *mut c_void {
        self.registered.cast::<c_void>().cast_mut()
    }

    /// Hands the registration to the session that will own it from here.
    ///
    /// `ManuallyDrop` rather than `mem::forget`, because the workspace lints ask for
    /// an FFI ownership transfer to be an explicit step rather than a leak that
    /// happens to be intended.
    fn into_owned(self) -> *const SessionCore {
        ManuallyDrop::new(self).registered
    }
}

impl Drop for PendingRegistration<'_> {
    fn drop(&mut self) {
        self.core.reconfigure.shutdown();
        if let Some(session) = self.core.session.get() {
            session.disable_callbacks();
            if !self.core.reconfigure.drain_for_drop(DEFAULT_NATIVE_WAIT) {
                quarantine_session(Arc::clone(self.core), self.registered);
                return;
            }
            if !close_registered(self.core, self.registered) {
                quarantine_session(Arc::clone(self.core), self.registered);
            }
            return;
        }
        // SAFETY: this came from `Arc::into_raw` in `new`, no other owner exists on
        // this path, and either no native session was ever registered against it or a
        // fence has proved that no callback can reach it.
        drop(unsafe { Arc::from_raw(self.registered) });
    }
}

impl NativeSession {
    /// Opens and starts a session for the target `metadata` describes.
    pub(crate) fn open(
        selected: SessionTarget,
        operation: &mut Operation<'_>,
    ) -> Result<Arc<Self>> {
        Self::open_inner(selected, 0, Duration::ZERO, Duration::ZERO, operation)
    }

    /// Opens a session that raises a contained native exception at `sites`.
    ///
    /// This is how the containment and failure-path ownership cases ADR 0012
    /// requires become reachable; nothing in the product asks for a raise site.
    #[cfg(test)]
    pub(crate) fn open_with_raise_sites(
        selected: SessionTarget,
        sites: u32,
        operation: &mut Operation<'_>,
    ) -> Result<Arc<Self>> {
        Self::open_inner(selected, sites, Duration::ZERO, Duration::ZERO, operation)
    }

    #[cfg(test)]
    pub(crate) fn open_with_delays(
        selected: SessionTarget,
        start_delay: Duration,
        stop_delay: Duration,
        operation: &mut Operation<'_>,
    ) -> Result<Arc<Self>> {
        Self::open_inner(selected, 0, start_delay, stop_delay, operation)
    }

    fn open_inner(
        selected: SessionTarget,
        testing_raise_sites: u32,
        testing_start_delay: Duration,
        testing_stop_delay: Duration,
        operation: &mut Operation<'_>,
    ) -> Result<Arc<Self>> {
        let SessionTarget {
            target,
            stream,
            key,
            fingerprint,
            selection,
            metadata,
            geometry,
        } = selected;
        let anchor = clock_calibration().ok_or(CaptureFault::SourceInvalid)?;
        let (reconfigure, reconfigure_receiver) = Reconfigure::new();
        let core = Arc::new(SessionCore {
            target_kind: key.kind(),
            geometry: GeometryRegistration::new(geometry, stream),
            state: StreamState::with_target_extent(stream),
            session: OnceLock::new(),
            pending_frame: PendingSlot::default(),
            transition: Mutex::new(TransitionState {
                extent: metadata.extent,
                surface: metadata.extent,
                published: None,
            }),
            reconfigure: Arc::clone(&reconfigure),
            clock_anchor: anchor,
            #[cfg(test)]
            testing_sites: testing_raise_sites,
            #[cfg(test)]
            terminal_reports: AtomicU64::new(0),
        });

        let pending = PendingRegistration::new(&core);
        let request = OpenRequest {
            kind: key.native_kind(),
            native_id: key.native_id(),
            // Descriptive metadata is validated against the retained filter handle;
            // it is never used to resolve another target.
            owner_process: fingerprint.native_owner(),
            target: selection,
            extent: metadata.extent,
            queue_depth: PRODUCER_QUEUE_DEPTH,
            detached_budget: DETACHED_BUFFER_BUDGET.get(),
            testing_start_delay,
            testing_stop_delay,
            testing_raise_sites,
        };
        // Every exit from here to the `NativeSession` below drops `pending`, which
        // closes whatever was opened and reclaims the registration.
        let session = shim::Session::open(
            &request,
            pending.context(),
            on_frame,
            on_frame_commit,
            on_stopped,
        )
        .map_err(|status| open_error(status, key.kind()))?;
        if let Err(unused) = core.session.set(session) {
            // Unreachable: the `OnceLock` was created a few statements above and
            // nothing else can have set it. Handled rather than discarded so that the
            // session which never reached `core` is closed by something visible.
            drop(unused);
            return Err(CaptureFault::SourceInvalid.into());
        }
        operation.checkpoint()?;

        spawn_reconfigure_worker(&core, reconfigure, reconfigure_receiver);

        let description = SessionDescription::new(
            target,
            stream,
            metadata.extent,
            PixelFormat::Bgra8,
            CoordinateSupport::with_target_placement(),
        )
        .with_queue(
            QueuePolicy::new(std::num::NonZeroU32::MIN, OverflowPolicy::Reject)
                .with_retained_storage(DETACHED_BUFFER_BUDGET),
        );
        // Consumed before `core` moves, which is also what ends the guard's borrow.
        let registered = pending.into_owned();
        let session = Arc::new(Self {
            description,
            core,
            registered,
            close_gate: CloseGate::default(),
            close_reported: AtomicBool::new(false),
        });

        loop {
            match session.core.session().start(native_wait(operation)) {
                Ok(()) => break,
                Err(ShimStatus::TimedOut) => operation.checkpoint()?,
                Err(status) => return Err(open_error(status, key.kind())),
            }
        }
        operation.checkpoint()?;
        Ok(session)
    }

    fn fence_and_close(&self, operation: &OperationContext) -> Result<()> {
        let mut attempt = Operation::admit(operation)?;
        let session = self.core.session();
        loop {
            match session.fence(native_wait(&attempt)) {
                Ok(()) => break,
                // The fence is retryable, so a native wait that expires becomes
                // this caller's own deadline or cancellation rather than a fault.
                Err(ShimStatus::TimedOut) => attempt.checkpoint()?,
                Err(status) => return Err(status.into()),
            }
        }
        self.core.discard_pending_frame();
        let closed = loop {
            match session.close(native_wait(&attempt)) {
                Ok(()) => break Ok(()),
                // Native close is a resumable phase machine. A slice expiring
                // leaves its current phase for this same caller or a later close.
                Err(ShimStatus::TimedOut) => attempt.checkpoint()?,
                Err(status) => break Err(status),
            }
        };
        attempt.commit(())?;
        match closed {
            Ok(()) => Ok(()),
            Err(status) => match close_error(status, self.core.target_kind) {
                // A close failure is reported once. A later close finds the release
                // already done and must not report it again.
                Some(error) if !self.close_reported.swap(true, Ordering::AcqRel) => Err(error),
                _ => Ok(()),
            },
        }
    }

    #[cfg(test)]
    pub(crate) fn terminal_reports(&self) -> u64 {
        self.core.terminal_reports.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn core_lifetime_probe(&self) -> Box<dyn Fn() -> bool + Send + Sync> {
        let core = Arc::downgrade(&self.core);
        Box::new(move || core.strong_count() != 0)
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
        self.core.state.frame(request, operation)
    }

    fn close(&self, operation: &OperationContext) -> Result<()> {
        self.core.state.begin_close();
        self.core.session().disable_callbacks();
        self.core.reconfigure.shutdown();
        let _gate = self.close_gate.enter(operation)?;
        self.core.reconfigure.drain(operation)?;
        self.fence_and_close(operation)?;
        self.core.state.drain(operation)
    }

    fn lifecycle(&self) -> Lifecycle {
        self.core.state.lifecycle()
    }
}

impl Drop for NativeSession {
    fn drop(&mut self) {
        let session = self.core.session();
        session.disable_callbacks();
        self.core.reconfigure.shutdown();
        if !self.core.reconfigure.drain_for_drop(DEFAULT_NATIVE_WAIT) {
            quarantine_session(Arc::clone(&self.core), self.registered);
            return;
        }
        if !close_registered(&self.core, self.registered) {
            quarantine_session(Arc::clone(&self.core), self.registered);
        }
    }
}

/// Completes native teardown after all Rust-owned auxiliary work has drained.
///
/// Returns false when the callback registration must remain quarantined.
fn close_registered(core: &Arc<SessionCore>, registered: *const SessionCore) -> bool {
    let session = core.session();
    let fenced = session.fence(DEFAULT_NATIVE_WAIT);
    if fenced.is_ok() {
        core.discard_pending_frame();
    }
    let closed = session.close(DEFAULT_NATIVE_WAIT);
    if fenced.is_err() || matches!(closed, Err(ShimStatus::TimedOut)) {
        /*
         * A timed-out native close still owns an asynchronous start or stop.
         * Keep `core`, and therefore the Rust session handle, in quarantine
         * until a worker joins that phase rather than relying on Session::drop
         * to create a second native-only quarantine.
         */
        return false;
    }
    // SAFETY: the fence returned, so the shim admits no further callback and none
    // is in flight; this consumes the single reference handed to it at open.
    drop(unsafe { Arc::from_raw(registered) });
    true
}

/// Lets bounded Drop return while preserving the ordering that native storage is
/// released only after reconfiguration work can no longer reach it.
fn quarantine_session(core: Arc<SessionCore>, registered: *const SessionCore) {
    let registered = registered as usize;
    let spawned = thread::Builder::new()
        .name("mado-pilot-sck-close-quarantine".to_owned())
        .spawn(move || {
            while !core.reconfigure.drain_for_drop(MAX_NATIVE_WAIT) {}
            let session = core.session();
            session.disable_callbacks();
            loop {
                match session.fence(DEFAULT_NATIVE_WAIT) {
                    Ok(()) => break,
                    Err(ShimStatus::TimedOut) => {}
                    Err(_) => return,
                }
            }
            core.discard_pending_frame();
            while matches!(
                session.close(DEFAULT_NATIVE_WAIT),
                Err(ShimStatus::TimedOut)
            ) {}
            // SAFETY: successful fencing above proves the callback registration is
            // unreachable, and this pointer is the one raw Arc the drop path handed
            // exclusively to this quarantine worker.
            drop(unsafe { Arc::from_raw(registered as *const SessionCore) });
        });
    if spawned.is_err() {
        // The raw Arc intentionally remains quarantined. Callback admission is
        // already disabled, so this is a bounded leak rather than a use-after-free.
    }
}

impl SessionCore {
    fn session(&self) -> &shim::Session {
        self.session
            .get()
            .expect("the native session is set before any operation reaches it")
    }

    fn fail_native(&self, fault: CaptureFault) {
        let fault = normalize_native_fault(fault, self.target_kind);
        self.discard_pending_frame();
        self.session().disable_callbacks();
        self.reconfigure.shutdown();
        self.state.terminate(fault);
    }

    fn stage_frame(&self, borrowed: &BorrowedFrame<'_>, info: &FrameInfo) -> ShimStatus {
        #[cfg(test)]
        if (self.testing_sites & shim::PANIC_IN_RUST_CALLBACK) != 0 {
            panic!("injected Rust frame callback panic");
        }
        #[cfg(test)]
        if (self.testing_sites & shim::DELAY_IN_RUST_CALLBACK) != 0 {
            TESTING_DELAYED_CALLBACK_ACTIVE.store(true, Ordering::Release);
            thread::sleep(DEFAULT_NATIVE_WAIT.saturating_add(Duration::from_millis(250)));
            TESTING_DELAYED_CALLBACK_ACTIVE.store(false, Ordering::Release);
        }
        if info.screen_rect().is_none() {
            // A complete image without the required same-frame placement
            // is a rejected publication, not a silent capability downgrade.
            let _drop = self.state.try_record_drop();
            return ShimStatus::Ok;
        }
        let detached = match borrowed.detach() {
            Ok(detached) => detached,
            // Finite pressure: every unit of the budget is retained by a caller.
            // The candidate is dropped rather than blocking the producer.
            Err(ShimStatus::BudgetExhausted | ShimStatus::FrameIncomplete) => {
                let _drop = self.state.try_record_drop();
                return ShimStatus::Ok;
            }
            Err(status) => return status,
        };
        if self
            .pending_frame
            .try_stage(PendingFrame {
                detached,
                info: *info,
            })
            .is_err()
        {
            // A previous frame was never committed or terminal cleanup briefly
            // owns the slot. Drop this detached frame instead of waiting or growing.
            let _drop = self.state.try_record_drop();
        }
        ShimStatus::Ok
    }

    fn commit_staged_frame(&self) -> ShimStatus {
        let Some(PendingFrame { detached, info }) = self.pending_frame.take() else {
            // Duplicate commit and a stage that deliberately dropped are both safe.
            return ShimStatus::Ok;
        };
        if let Err(fault) = self.process_frame(detached, &info)
            && (fault != CaptureFault::SessionClosed || self.state.lifecycle() == Lifecycle::Open)
        {
            self.fail_native(fault);
        }
        ShimStatus::Ok
    }

    fn discard_pending_frame(&self) {
        self.pending_frame.clear();
    }

    fn process_frame(
        &self,
        detached: DetachedFrame,
        info: &FrameInfo,
    ) -> std::result::Result<(), CaptureFault> {
        let mut transition = self
            .transition
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let extent = info.extent().ok_or(CaptureFault::InconsistentDescriptor)?;
        let native_bounds = info
            .screen_rect()
            .zip(info.backing_scale())
            .map(|((origin, size), scale)| shim::NativeBounds {
                origin,
                size,
                scale,
            })
            .ok_or(CaptureFault::InconsistentDescriptor)?;
        let surface = info
            .surface_extent()
            .ok_or(CaptureFault::InconsistentDescriptor)?;

        // A requested reconfiguration taking effect changes the producer's
        // container, not what this Adapter publishes: the content is cropped out of
        // it either way. So the new surface is recorded and nothing is concluded
        // from it — the content extent below is what a caller sees.
        transition.surface = surface;

        let placement = frame_placement(info)?;
        // The hint is prospective producer capacity derived and bounded beside this
        // sample's metadata. It never changes the placement or extent published for
        // the current pixels, and a later inventory never participates.
        if let Some(wanted) = surface_request(self.target_kind, info) {
            // The worker performs the native call off this queue.
            self.request_reconfigure(wanted);
        }
        if !extent_ready_for_publication(&mut transition, extent) {
            let _drop = self.state.try_record_drop();
            return Ok(());
        }

        let descriptor = descriptor_from_native(info.pixel_format, extent)?;
        let continuity = continuity_against(transition.published, extent, placement, native_bounds);
        self.publish(
            &mut transition,
            detached,
            descriptor,
            placement,
            native_bounds,
            continuity,
            info.display_time_nanos,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn publish(
        &self,
        transition: &mut TransitionState,
        detached: DetachedFrame,
        descriptor: mado_pilot_capture::FrameDescriptor,
        placement: TargetPlacement,
        native_bounds: shim::NativeBounds,
        continuity: Continuity,
        display_time_nanos: u64,
    ) -> std::result::Result<(), CaptureFault> {
        let storage = MacosFrameStorage::new(descriptor, detached);
        self.state
            .publish_storage_with(
                StoragePublication {
                    captured_at: frame_time(self.clock_anchor, display_time_nanos),
                    placement: Some(placement),
                    storage,
                    continuity,
                },
                // Recorded while the stream still excludes readers, so no frame is
                // observable before the transform an input request would resolve
                // its coordinates against.
                |frame| self.geometry.publish(frame, native_bounds),
            )
            .map_err(|refused| {
                if refused.error().status() == mado_pilot_core::Status::Closed {
                    CaptureFault::SessionClosed
                } else {
                    CaptureFault::SourceInvalid
                }
            })?;
        transition.published = Some(Published {
            extent: descriptor.extent(),
            placement,
            native_bounds,
        });
        Ok(())
    }

    fn request_reconfigure(&self, wanted: PixelExtent) {
        if self.reconfigure.request(wanted) != ReconfigurePublication::Published {
            // Coalescing and shutdown rejection are observable in the next public
            // sequence rather than disappearing as hidden worker pressure.
            let _drop = self.state.try_record_drop();
        }
    }

    fn on_stopped(&self, status: ShimStatus) {
        #[cfg(test)]
        self.terminal_reports.fetch_add(1, Ordering::AcqRel);
        let fault = match status {
            // The framework names a deliberate stop, so it is reported as one
            // rather than as a target that went away.
            ShimStatus::StoppedByUser => CaptureFault::ExplicitlyStopped,
            // The system ended the stream without naming a cause. Revoked
            // authorization is one cause among several, so it is established by
            // reading the authorization again rather than assumed from the stop.
            ShimStatus::StoppedBySystem => system_stop_fault(),
            ShimStatus::PermissionDenied => CaptureFault::AccessDenied,
            // Only explicit ScreenCaptureKit no-source/list outcomes become target
            // loss. An unexplained stop remains CaptureFailed rather than inferring
            // absence from a wrapper or inventory observation.
            ShimStatus::Ok => CaptureFault::SourceInvalid,
            ShimStatus::Closed => CaptureFault::ExplicitlyStopped,
            ShimStatus::TargetLost => target_fault(self.target_kind),
            other => other.fault(),
        };
        self.fail_native(fault);
    }
}

fn spawn_reconfigure_worker(
    core: &Arc<SessionCore>,
    reconfigure: Arc<Reconfigure>,
    receiver: Receiver<()>,
) {
    let weak = Arc::downgrade(core);
    let owned = Arc::clone(&reconfigure);
    reconfigure.prepare_worker();
    let worker = thread::Builder::new()
        .name("mado-pilot-sck-reconfigure".to_owned())
        .spawn(move || {
            struct Finished(Arc<Reconfigure>);
            impl Drop for Finished {
                fn drop(&mut self) {
                    self.0.worker_finished();
                }
            }
            let _finished = Finished(Arc::clone(&owned));
            run_reconfigure_worker(&weak, &owned, &receiver);
        });
    match worker {
        Ok(worker) => reconfigure.install_worker(worker),
        Err(_) => {
            // Without a worker the session still captures at its opening extent;
            // request rejection is recorded as observable stream pressure.
            reconfigure.worker_finished();
            reconfigure.shutdown();
        }
    }
}

fn run_reconfigure_worker(
    core: &Weak<SessionCore>,
    reconfigure: &Reconfigure,
    receiver: &Receiver<()>,
) {
    loop {
        if reconfigure.shutdown.load(Ordering::Acquire) {
            return;
        }
        match receiver.recv_timeout(CLOSE_POLL_INTERVAL.saturating_mul(50)) {
            Ok(()) => {}
            Err(mpsc::RecvTimeoutError::Timeout) if core.strong_count() > 0 => continue,
            Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => return,
        }
        if reconfigure.shutdown.load(Ordering::Acquire) {
            return;
        }
        let Some(extent) = decode_extent(reconfigure.wanted.swap(0, Ordering::AcqRel)) else {
            // A shutdown can clear the slot after the wake token was queued.
            continue;
        };
        let Some(core) = core.upgrade() else {
            return;
        };
        // The native call happens here rather than in the producer callback, and
        // its own failure is not terminal: the session keeps publishing the
        // content that fits the surface it already has.
        let _reconfigured = core.session().reconfigure(extent, MAX_NATIVE_WAIT);
        // Collapse any redundant queued wake token. A request that arrived while
        // the native call ran remains in the atomic slot and one token is enough.
        match receiver.try_recv() {
            Ok(()) | Err(TryRecvError::Empty | TryRecvError::Disconnected) => {}
        }
        if reconfigure.wanted.load(Ordering::Acquire) != 0 {
            let _wake = reconfigure.wake.try_send(());
        }
    }
}

/// The frame callback the shim invokes on its sample queue.
///
/// # Safety
///
/// Called only by the shim, with the context registered at open.
unsafe extern "C" fn on_frame(
    context: *mut c_void,
    frame: *mut shim::OpaqueFrameHandle,
    info: *const FrameInfo,
) -> u32 {
    // SAFETY: the shim passes the registered context and its own live pointers.
    unsafe {
        shim::contained_frame_callback::<SessionCore>(
            context,
            frame,
            info,
            |core, borrowed, report| core.stage_frame(&borrowed, report),
        )
    }
}

/// Commits the frame staged by [`on_frame`] after native delivery can no longer raise.
///
/// # Safety
///
/// Called only by the shim, with the context registered at open.
unsafe extern "C" fn on_frame_commit(context: *mut c_void) -> u32 {
    // SAFETY: the shim passes the registered context after a successful stage.
    unsafe {
        shim::contained_frame_commit_callback::<SessionCore>(context, |core| {
            core.commit_staged_frame()
        })
    }
}

/// The producer-stopped callback the shim invokes.
///
/// # Safety
///
/// Called only by the shim, with the context registered at open.
unsafe extern "C" fn on_stopped(context: *mut c_void, status: u32) {
    // SAFETY: the shim passes the registered context.
    unsafe {
        shim::contained_stopped_callback::<SessionCore>(context, status, |core, status| {
            core.on_stopped(status);
        });
    }
}

/// Maps a shim status from open onto the caller's outcome.
fn open_error(status: ShimStatus, kind: TargetKind) -> mado_pilot_core::Error {
    match status {
        ShimStatus::TargetLost => target_fault(kind).into(),
        ShimStatus::TimedOut => CaptureFault::SourceInvalid.into(),
        other => normalize_native_fault(other.fault(), kind).into(),
    }
}

fn close_error(status: ShimStatus, kind: TargetKind) -> Option<mado_pilot_core::Error> {
    match status {
        // A producer that had already stopped — on its own, or because the user or
        // the system ended it — is not a failure of the close the caller asked for.
        // The close did what it was for, so it reports success.
        ShimStatus::Closed | ShimStatus::StoppedByUser | ShimStatus::StoppedBySystem => None,
        other => Some(normalize_native_fault(other.fault(), kind).into()),
    }
}

/// Decides how a frame about to be published relates to the last published one.
///
/// Compared against what the stream actually received, never against intermediate
/// observations. `StreamState` overrules an under-claim — an extent or format change
/// is discontinuous whatever an Adapter says — so the only thing that can go wrong
/// here is claiming *more* than happened, which is what a sticky flag did.
fn continuity_against(
    published: Option<Published>,
    extent: PixelExtent,
    placement: TargetPlacement,
    native_bounds: shim::NativeBounds,
) -> Continuity {
    match published {
        // Nothing has been published, so there is nothing to be discontinuous with.
        None => Continuity::Continuous,
        Some(last) if last.extent != extent => Continuity::Discontinuous,
        Some(last) if last.placement != placement => Continuity::GeometryChanged,
        Some(last) if !last.native_bounds.capture_equivalent_to(native_bounds) => {
            Continuity::GeometryChanged
        }
        Some(_) => Continuity::Continuous,
    }
}

/// Returns the producer surface extent to ask for, if any.
///
/// A recommendation is a capacity high-water hint, not publication geometry.
/// Retaining an oversized window surface prevents a 2x-to-1x move from immediately
/// surrendering the capacity a later move back to 2x needs. Displays retain their
/// prior same-frame content reconfiguration path.
fn surface_request(target_kind: TargetKind, info: &FrameInfo) -> Option<PixelExtent> {
    let surface = info.surface_extent()?;
    if target_kind == TargetKind::Window {
        let wanted = info.recommended_surface_extent()?;
        return (wanted.width() > surface.width() || wanted.height() > surface.height())
            .then_some(wanted);
    }
    match target_kind {
        TargetKind::Display => {
            let content = info.extent()?;
            (content != surface).then_some(content)
        }
        _ => None,
    }
}

/// Drops the first observation of a changed content extent before publication.
///
/// A later frame at the same extent is compared with the last published frame and
/// therefore becomes discontinuous exactly once through [`continuity_against`].
fn extent_ready_for_publication(transition: &mut TransitionState, extent: PixelExtent) -> bool {
    if extent == transition.extent {
        return true;
    }
    transition.extent = extent;
    false
}

/// Decides what a system-initiated stop means, by reading authorization again.
///
/// The framework says only that the system ended the stream. Revoked Screen
/// Recording is one cause, and it is the one a caller can act on, so it is
/// established with the same non-prompting probe discovery uses rather than
/// inferred from the stop. Anything else is reported as the stream simply having
/// ended, which is what is known.
fn system_stop_fault() -> CaptureFault {
    match shim::probe_screen_capture() {
        Ok(state) if state.is_refused() => CaptureFault::AccessDenied,
        // A probe that cannot answer is not evidence of revocation.
        Ok(_) | Err(_) => CaptureFault::StreamEnded,
    }
}

/// Reports a target-lifetime fault in the kind-specific form.
const fn target_fault(kind: TargetKind) -> CaptureFault {
    match kind {
        TargetKind::Window => CaptureFault::CaptureItemClosed,
        TargetKind::Display => CaptureFault::DisplayDisconnected,
        _ => CaptureFault::TargetLost,
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
        CaptureFault::TargetLost => target_fault(kind),
        _ => fault,
    }
}

/// Returns how long one native call may wait, bounded by the caller's budget.
fn native_wait(operation: &Operation<'_>) -> Duration {
    operation
        .context()
        .remaining()
        .map_or(MAX_NATIVE_WAIT, |remaining| remaining.min(MAX_NATIVE_WAIT))
}

/// Pairs the producer's clock with the project's, so a frame timestamp needs no
/// further clock read.
fn clock_calibration() -> Option<(u64, MonotonicInstant)> {
    let before = SystemClock.now();
    let native = shim::monotonic_nanos()?;
    let after = SystemClock.now();
    let midpoint = before.checked_add(after.saturating_duration_since(before) / 2)?;
    Some((native, midpoint))
}

/// Converts a producer timestamp into the engine's monotonic domain.
fn frame_time(anchor: (u64, MonotonicInstant), display_time_nanos: u64) -> MonotonicInstant {
    let (native_origin, project_origin) = anchor;
    if display_time_nanos >= native_origin {
        let elapsed = Duration::from_nanos(display_time_nanos - native_origin);
        project_origin
            .checked_add(elapsed)
            .unwrap_or_else(|| MonotonicInstant::from_origin(Duration::MAX))
    } else {
        let elapsed = Duration::from_nanos(native_origin - display_time_nanos);
        MonotonicInstant::from_origin(project_origin.since_origin().saturating_sub(elapsed))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, OnceLock, mpsc};
    use std::thread;
    use std::time::{Duration, Instant};

    use mado_pilot_capture::{CaptureFault, StreamState};
    use mado_pilot_core::{
        CancellationToken, Clock, GeometryRevision, IdentityIssuer, MonotonicInstant, Operation,
        OperationContext, PixelExtent, Scale, StreamCursor, TargetKind, TargetPlacement,
        TransformSnapshot,
    };

    use crate::input::GeometryLedger;
    use crate::native::GeometryRegistration;

    use super::{
        CloseGate, MAX_NATIVE_WAIT, PendingRegistration, PendingSlot, Published, Reconfigure,
        ReconfigurePublication, SessionCore, TransitionState, continuity_against, decode_extent,
        extent_ready_for_publication, frame_time, native_wait, normalize_native_fault,
        surface_request, target_fault,
    };
    use crate::shim::{FrameInfo, NativeBounds};

    fn native_bounds(origin: (f64, f64), size: (f64, f64), scale: f64) -> NativeBounds {
        NativeBounds {
            origin,
            size,
            scale,
        }
    }

    #[test]
    fn terminal_discard_outranks_a_staged_commit_and_releases_the_value() {
        struct DropProbe(Arc<AtomicU64>);

        impl Drop for DropProbe {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::AcqRel);
            }
        }

        let drops = Arc::new(AtomicU64::new(0));
        let pending = PendingSlot::default();
        assert!(pending.try_stage(DropProbe(Arc::clone(&drops))).is_ok());

        // This is the stopped callback's order: release staged storage first,
        // then publish the terminal state. A later/duplicate commit sees nothing.
        pending.clear();
        assert!(pending.take().is_none());
        assert_eq!(drops.load(Ordering::Acquire), 1);

        // The slot remains usable and a normal take transfers exactly one value.
        assert!(pending.try_stage(DropProbe(Arc::clone(&drops))).is_ok());
        drop(pending.take());
        assert!(pending.take().is_none());
        assert_eq!(drops.load(Ordering::Acquire), 2);
    }

    #[test]
    fn a_target_lifetime_fault_is_reported_in_its_kind_specific_form() {
        assert_eq!(
            target_fault(TargetKind::Window),
            CaptureFault::CaptureItemClosed
        );
        assert_eq!(
            target_fault(TargetKind::Display),
            CaptureFault::DisplayDisconnected
        );
        assert_eq!(
            normalize_native_fault(CaptureFault::TargetLost, TargetKind::Display),
            CaptureFault::DisplayDisconnected
        );
        assert_eq!(
            normalize_native_fault(CaptureFault::DisplayDisconnected, TargetKind::Window),
            CaptureFault::CaptureItemClosed
        );
        assert_eq!(
            normalize_native_fault(CaptureFault::AccessDenied, TargetKind::Window),
            CaptureFault::AccessDenied
        );
    }

    #[test]
    fn a_producer_timestamp_uses_the_precalibrated_project_clock() {
        let anchor = (
            1_000_000_000,
            MonotonicInstant::from_origin(Duration::from_secs(10)),
        );

        let at_anchor = frame_time(anchor, 1_000_000_000);
        let earlier = frame_time(anchor, 999_000_000);
        let later = frame_time(anchor, 1_003_000_000);

        assert_eq!(at_anchor.since_origin(), Duration::from_secs(10));
        assert_eq!(
            at_anchor.saturating_duration_since(earlier),
            Duration::from_millis(1)
        );
        assert_eq!(
            later.saturating_duration_since(at_anchor),
            Duration::from_millis(3)
        );
    }

    #[test]
    fn a_native_wait_never_exceeds_the_callers_remaining_budget() {
        let context = OperationContext::new()
            .with_timeout(Duration::from_millis(30))
            .expect("timeout");
        let operation = Operation::admit(&context).expect("admitted");

        assert!(native_wait(&operation) <= Duration::from_millis(30));

        let open = OperationContext::new();
        let unbounded = Operation::admit(&open).expect("admitted");
        assert_eq!(native_wait(&unbounded), MAX_NATIVE_WAIT);
    }

    /// Builds a session core with no native session behind it.
    ///
    /// Enough for the registration's own accounting, which is what the case below is
    /// about, and reachable on any host because it opens nothing.
    fn unregistered_core() -> Arc<SessionCore> {
        let issuer = IdentityIssuer::new();
        let extent = PixelExtent::new(64, 48);
        let stream = issuer.issue_stream().expect("stream identity");
        Arc::new(SessionCore {
            target_kind: TargetKind::Display,
            geometry: GeometryRegistration::new(Arc::new(GeometryLedger::default()), stream),
            state: StreamState::with_target_extent(stream),
            session: OnceLock::new(),
            pending_frame: PendingSlot::default(),
            transition: Mutex::new(TransitionState {
                extent,
                surface: extent,
                published: None,
            }),
            reconfigure: Reconfigure::new().0,
            clock_anchor: (0, MonotonicInstant::from_origin(Duration::ZERO)),
            #[cfg(test)]
            testing_sites: 0,
            #[cfg(test)]
            terminal_reports: AtomicU64::new(0),
        })
    }

    #[test]
    fn dropping_geometry_registration_retires_only_its_stream_history() {
        let ledger = Arc::new(GeometryLedger::default());
        let issuer = IdentityIssuer::new();
        let retired_stream = issuer.issue_stream().expect("retired stream");
        let live_stream = issuer.issue_stream().expect("live stream");
        let mut retired_cursor = StreamCursor::new(retired_stream);
        let mut live_cursor = StreamCursor::new(live_stream);
        let revision = GeometryRevision::FIRST;
        let retired_stamp = retired_cursor.publish(revision).expect("retired stamp");
        let live_stamp = live_cursor.publish(revision).expect("live stamp");
        let transform = TransformSnapshot::with_target_extent(revision, PixelExtent::new(64, 48));
        let bounds = native_bounds((0.0, 0.0), (64.0, 48.0), 1.0);
        ledger.record(retired_stamp, transform, bounds);
        ledger.record(live_stamp, transform, bounds);
        let registration = GeometryRegistration::new(Arc::clone(&ledger), retired_stream);

        drop(registration);

        assert_eq!(ledger.source_geometry(retired_stamp), None);
        assert_eq!(
            ledger.source_geometry(live_stamp),
            Some((transform, bounds)),
            "closing one stream does not retire another live stream's source geometry"
        );
    }

    #[test]
    fn an_open_that_never_reached_a_session_reclaims_what_it_registered() {
        let core = unregistered_core();
        assert_eq!(Arc::strong_count(&core), 1);

        let pending = PendingRegistration::new(&core);
        assert_eq!(
            Arc::strong_count(&core),
            2,
            "the core itself, and the reference the shim is handed"
        );

        // Every exit between the shim taking the context and a NativeSession existing
        // to reclaim it comes through here. Two of them used to leak the core with a
        // live capture inside it.
        drop(pending);
        assert_eq!(
            Arc::strong_count(&core),
            1,
            "an interrupted open leaves nothing registered against the core"
        );
    }

    #[test]
    fn a_registration_handed_on_outlives_the_guard() {
        let core = unregistered_core();
        let registered = PendingRegistration::new(&core).into_owned();
        assert_eq!(
            Arc::strong_count(&core),
            2,
            "the guard is gone and the reference it handed the shim is not"
        );

        // What NativeSession::drop does with it once a fence has succeeded.
        // SAFETY: `registered` came from one `Arc::into_raw` and is consumed once.
        drop(unsafe { Arc::from_raw(registered) });
        assert_eq!(Arc::strong_count(&core), 1);
    }

    #[test]
    fn a_same_frame_placement_change_advances_only_the_geometry_revision() {
        let extent = PixelExtent::new(1718, 1050);
        let scale = Scale::new(1.0, 1.0).expect("scale");
        let at =
            |x: f64| TargetPlacement::new((x, 400.0), (1718.0, 1050.0), scale).expect("placement");
        let original = at(-2489.0);
        let moved = at(0.0);
        let published = Published {
            extent,
            placement: original,
            native_bounds: native_bounds((-2489.0, 400.0), (1718.0, 1050.0), 1.0),
        };

        assert_eq!(
            continuity_against(
                Some(published),
                extent,
                moved,
                native_bounds((0.0, 400.0), (1718.0, 1050.0), 1.0),
            ),
            mado_pilot_capture::Continuity::GeometryChanged
        );
        assert_eq!(
            continuity_against(Some(published), extent, original, published.native_bounds,),
            mado_pilot_capture::Continuity::Continuous
        );
        assert_eq!(
            continuity_against(
                Some(published),
                PixelExtent::new(3436, 2216),
                moved,
                native_bounds((0.0, 400.0), (1718.0, 1108.0), 2.0),
            ),
            mado_pilot_capture::Continuity::Discontinuous
        );
        assert_eq!(
            continuity_against(
                None,
                extent,
                moved,
                native_bounds((0.0, 400.0), (1718.0, 1050.0), 1.0),
            ),
            mado_pilot_capture::Continuity::Continuous
        );
        assert_eq!(
            continuity_against(
                Some(published),
                extent,
                original,
                native_bounds((-2489.0, 400.0), (1718.0, 1050.0), 2.0),
            ),
            mado_pilot_capture::Continuity::GeometryChanged,
            "a raw backing-scale change advances the geometry revision even when effective frame placement is unchanged"
        );
    }

    #[test]
    fn raw_fractional_backing_geometry_controls_the_geometry_revision() {
        let extent = PixelExtent::new(320, 240);
        let placement = TargetPlacement::new(
            (-120.0, 80.0),
            (320.0, 240.0),
            Scale::new(1.0, 1.0).expect("scale"),
        )
        .expect("placement");
        let published = Published {
            extent,
            placement,
            native_bounds: native_bounds((-120.0, 80.0), (320.4, 240.0), 2.0),
        };

        assert_eq!(
            continuity_against(
                Some(published),
                extent,
                placement,
                native_bounds((-120.0, 80.0), (320.49, 240.0), 2.0),
            ),
            mado_pilot_capture::Continuity::Continuous,
            "fractional point sizes with the same rounded backing-pixel extent are equivalent"
        );
        assert_eq!(
            continuity_against(
                Some(published),
                extent,
                placement,
                native_bounds((-120.0, 80.0), (320.75, 240.0), 2.0),
            ),
            mado_pilot_capture::Continuity::GeometryChanged,
            "a raw size change that crosses a backing-pixel boundary advances geometry even when effective capture placement is unchanged"
        );
    }

    #[test]
    fn a_surface_too_small_for_its_target_is_asked_to_grow() {
        let surface = PixelExtent::new(1718, 1108);
        let target = PixelExtent::new(3436, 2216);
        let info = FrameInfo::testing_screen_rect_with_surface_recommendation(
            surface,
            surface,
            1.0,
            (0.0, 0.0),
            (1718.0, 1108.0),
            target,
        );

        assert_eq!(surface_request(TargetKind::Window, &info), Some(target));
    }

    #[test]
    fn a_filled_surface_at_the_wrong_scale_is_still_asked_to_grow() {
        // The case a content-extent comparison cannot see. A window keeps its point
        // size across a move from a 1x display to a 2x one, so it needs twice the
        // pixels — and the framework downscales into the old surface and fills it
        // exactly. Reading a filled surface as settled captures the target at half
        // resolution for the life of the stream.
        let surface = PixelExtent::new(1718, 1108);
        let target = PixelExtent::new(3436, 2216);
        let info = FrameInfo::testing_screen_rect_with_surface_recommendation(
            surface,
            surface,
            // raw scaleFactor=2 and contentScale=0.5 produced effective scale 1.
            1.0,
            (34.0, 191.0),
            (1718.0, 1108.0),
            target,
        )
        .with_backing_scale(2.0);
        let placement = crate::discovery::frame_placement(&info).expect("same-frame placement");

        assert_eq!(
            surface_request(TargetKind::Window, &info),
            Some(target),
            "a surface the content fills can still be the wrong size for the target"
        );
        assert_eq!(placement.scale().x(), 1.0);
        assert_eq!(info.backing_scale(), Some(2.0));
        assert_eq!(placement.desktop_origin(), (34.0, 191.0));
    }

    #[test]
    fn a_surface_already_the_size_the_target_needs_is_not_asked_for_again() {
        let surface = PixelExtent::new(3436, 2216);
        let content = PixelExtent::new(1718, 1108);
        let info = FrameInfo::testing_screen_rect_with_surface_recommendation(
            content,
            surface,
            1.0,
            (0.0, 0.0),
            (1718.0, 1108.0),
            surface,
        );

        assert_eq!(surface_request(TargetKind::Window, &info), None);
    }

    #[test]
    fn a_window_without_a_valid_hint_does_not_surrender_an_oversized_surface() {
        let content = PixelExtent::new(1718, 1108);
        let mut info = FrameInfo::testing_screen_rect(content, 1.0, (0.0, 0.0), (1718.0, 1108.0));
        info.surface_width = 3436;
        info.surface_height = 2216;

        assert_eq!(surface_request(TargetKind::Window, &info), None);
    }

    #[test]
    fn a_display_frame_without_a_hint_keeps_the_existing_content_reconfigure_path() {
        let content = PixelExtent::new(1920, 1080);
        let info = FrameInfo::testing_screen_rect(content, 1.0, (0.0, 0.0), (1920.0, 1080.0));
        let mut resized = info;
        resized.surface_width = 2560;
        resized.surface_height = 1440;

        assert_eq!(
            surface_request(TargetKind::Display, &resized),
            Some(content)
        );
    }

    #[test]
    fn a_larger_surface_is_retained_when_a_window_returns_to_one_x() {
        let content = PixelExtent::new(1718, 1108);
        let surface = PixelExtent::new(3436, 2216);
        let info = FrameInfo::testing_screen_rect_with_surface_recommendation(
            content,
            surface,
            1.0,
            (0.0, 0.0),
            (1718.0, 1108.0),
            content,
        );

        assert_eq!(
            surface_request(TargetKind::Window, &info),
            None,
            "a 2x-to-1x move cannot surrender already-allocated capacity"
        );
    }

    #[test]
    fn growth_in_one_axis_requests_the_exact_bounded_recommendation() {
        let content = PixelExtent::new(1000, 800);
        let surface = PixelExtent::new(2000, 1600);
        let wanted = PixelExtent::new(2500, 1200);
        let info = FrameInfo::testing_screen_rect_with_surface_recommendation(
            content,
            surface,
            1.0,
            (0.0, 0.0),
            (1000.0, 800.0),
            wanted,
        );

        assert_eq!(surface_request(TargetKind::Window, &info), Some(wanted));
    }

    #[test]
    fn a_reconfigured_two_x_extent_becomes_one_discontinuity_after_the_transition_drop() {
        let one_x = PixelExtent::new(1718, 1108);
        let two_x = PixelExtent::new(3436, 2216);
        let one_scale = Scale::new(1.0, 1.0).expect("scale");
        let published_placement =
            TargetPlacement::new((-700.0, 191.0), (1718.0, 1108.0), one_scale).expect("placement");
        let mut transition = TransitionState {
            extent: one_x,
            surface: one_x,
            published: Some(Published {
                extent: one_x,
                placement: published_placement,
                native_bounds: native_bounds((-700.0, 191.0), (1718.0, 1108.0), 1.0),
            }),
        };
        let reduced = FrameInfo::testing_screen_rect_with_surface_recommendation(
            one_x,
            one_x,
            1.0,
            (34.0, 191.0),
            (1718.0, 1108.0),
            two_x,
        )
        .with_backing_scale(2.0);
        let reduced_placement =
            crate::discovery::frame_placement(&reduced).expect("reduced placement");
        assert!(extent_ready_for_publication(&mut transition, one_x));
        assert_eq!(
            continuity_against(
                transition.published,
                one_x,
                reduced_placement,
                native_bounds((34.0, 191.0), (1718.0, 1108.0), 2.0),
            ),
            mado_pilot_capture::Continuity::GeometryChanged
        );
        assert_eq!(surface_request(TargetKind::Window, &reduced), Some(two_x));
        transition.published = Some(Published {
            extent: one_x,
            placement: reduced_placement,
            native_bounds: native_bounds((34.0, 191.0), (1718.0, 1108.0), 2.0),
        });

        let settled = FrameInfo::testing_screen_rect_with_surface_recommendation(
            two_x,
            two_x,
            2.0,
            (34.0, 191.0),
            (1718.0, 1108.0),
            two_x,
        );
        let settled_placement =
            crate::discovery::frame_placement(&settled).expect("settled placement");
        assert!(
            !extent_ready_for_publication(&mut transition, two_x),
            "the first changed-extent observation is dropped"
        );
        assert!(extent_ready_for_publication(&mut transition, two_x));
        assert_eq!(surface_request(TargetKind::Window, &settled), None);
        assert_eq!(
            continuity_against(
                transition.published,
                two_x,
                settled_placement,
                native_bounds((34.0, 191.0), (1718.0, 1108.0), 2.0),
            ),
            mado_pilot_capture::Continuity::Discontinuous,
            "StreamState turns this one publication into the new epoch"
        );
    }

    #[test]
    fn a_reconfiguration_request_keeps_only_the_latest_extent() {
        let (reconfigure, _receiver) = Reconfigure::new();

        assert_eq!(
            reconfigure.request(PixelExtent::new(100, 100)),
            ReconfigurePublication::Published
        );
        assert_eq!(
            reconfigure.request(PixelExtent::new(200, 200)),
            ReconfigurePublication::Coalesced
        );

        assert_eq!(
            decode_extent(reconfigure.wanted.load(Ordering::Acquire)),
            Some(PixelExtent::new(200, 200)),
            "an intermediate size a resize passed through is not worth a round trip"
        );
        assert_eq!(reconfigure.coalesced.load(Ordering::Acquire), 1);
    }

    #[test]
    fn every_valid_extent_round_trips_through_the_atomic_slot_encoding() {
        for extent in [
            PixelExtent::new(1, 1),
            PixelExtent::new(u32::MAX, 1),
            PixelExtent::new(1, u32::MAX),
            PixelExtent::new(u32::MAX, u32::MAX),
        ] {
            let encoded = super::encode_extent(extent);
            assert_ne!(encoded, 0, "zero remains the empty-slot sentinel");
            assert_eq!(decode_extent(encoded), Some(extent));
        }
    }

    #[test]
    fn a_request_after_shutdown_is_refused_so_nothing_queues_unread() {
        let (reconfigure, _receiver) = Reconfigure::new();

        reconfigure.shutdown();
        assert_eq!(
            reconfigure.request(PixelExtent::new(64, 64)),
            ReconfigurePublication::Rejected
        );

        assert!(reconfigure.shutdown.load(Ordering::Acquire));
        assert_eq!(reconfigure.wanted.load(Ordering::Acquire), 0);
        assert_eq!(reconfigure.rejected.load(Ordering::Acquire), 1);
    }

    #[test]
    fn callback_side_reconfiguration_is_non_blocking_and_latest_wins_under_contention() {
        let (reconfigure, _receiver) = Reconfigure::new();
        let held_worker_state = reconfigure.worker();
        let (published, observed) = mpsc::channel();
        let producer = Arc::clone(&reconfigure);
        let producer_thread = thread::spawn(move || {
            let began = Instant::now();
            let first = producer.request(PixelExtent::new(100, 100));
            let second = producer.request(PixelExtent::new(200, 200));
            published
                .send((first, second, began.elapsed()))
                .expect("observe callback-side publication");
        });

        let result = observed.recv_timeout(Duration::from_millis(250));
        drop(held_worker_state);
        producer_thread.join().expect("producer thread");
        let (first, second, elapsed) =
            result.expect("publication returns while the worker-state mutex is held");

        assert_eq!(first, ReconfigurePublication::Published);
        assert_eq!(second, ReconfigurePublication::Coalesced);
        assert!(elapsed < Duration::from_millis(250));
        assert_eq!(
            decode_extent(reconfigure.wanted.load(Ordering::Acquire)),
            Some(PixelExtent::new(200, 200))
        );
    }

    #[test]
    fn successful_drain_finishes_in_flight_reconfiguration_before_native_teardown() {
        let (reconfigure, _receiver) = Reconfigure::new();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        reconfigure.prepare_worker();
        let worker_state = Arc::clone(&reconfigure);
        let worker = thread::spawn(move || {
            entered_tx.send(()).expect("worker entered");
            release_rx.recv().expect("release worker");
            worker_state.worker_finished();
        });
        reconfigure.install_worker(worker);
        entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("worker entered its in-flight section");

        let native_teardown = Arc::new(AtomicBool::new(false));
        let teardown_flag = Arc::clone(&native_teardown);
        let draining = Arc::clone(&reconfigure);
        let (finished_tx, finished_rx) = mpsc::channel();
        let close_thread = thread::spawn(move || {
            let context = OperationContext::new()
                .with_timeout(Duration::from_secs(2))
                .expect("timeout");
            let drained = draining.drain(&context).is_ok();
            teardown_flag.store(true, Ordering::Release);
            finished_tx.send(drained).expect("close result");
        });

        assert!(
            finished_rx.recv_timeout(Duration::from_millis(40)).is_err(),
            "native teardown cannot overtake in-flight reconfiguration"
        );
        assert!(!native_teardown.load(Ordering::Acquire));
        release_tx.send(()).expect("release reconfiguration");
        assert!(
            finished_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("drain completes")
        );
        close_thread.join().expect("close thread");
        assert!(native_teardown.load(Ordering::Acquire));
    }

    #[test]
    fn drop_timeout_keeps_in_flight_reconfiguration_quarantined_until_it_can_join() {
        let (reconfigure, _receiver) = Reconfigure::new();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        reconfigure.prepare_worker();
        let worker_state = Arc::clone(&reconfigure);
        let worker = thread::spawn(move || {
            entered_tx.send(()).expect("worker entered");
            release_rx.recv().expect("release worker");
            worker_state.worker_finished();
        });
        reconfigure.install_worker(worker);
        entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("worker entered its in-flight section");

        assert!(!reconfigure.drain_for_drop(Duration::from_millis(10)));
        {
            let worker = reconfigure.worker();
            assert!(!worker.finished);
            assert!(
                worker.handle.is_some(),
                "the join handle remains quarantined"
            );
        }

        release_tx.send(()).expect("release reconfiguration");
        assert!(reconfigure.drain_for_drop(Duration::from_secs(1)));
        let worker = reconfigure.worker();
        assert!(worker.finished);
        assert!(worker.handle.is_none(), "the completed worker was joined");
    }

    #[test]
    fn a_cancelled_close_gate_reports_cancellation_before_ownership() {
        let token = CancellationToken::new();
        token.cancel();
        let context = OperationContext::new().with_cancellation(token);
        let gate = CloseGate::default();

        let error = gate.enter(&context).expect_err("cancelled");

        assert_eq!(error.status(), mado_pilot_core::Status::Cancelled);
    }

    #[test]
    fn concurrent_close_callers_never_own_the_gate_together() {
        let gate = Arc::new(CloseGate::default());
        let (owner_tx, owner_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let first_gate = Arc::clone(&gate);
        let first = thread::spawn(move || {
            let owner = first_gate
                .enter(&OperationContext::new())
                .expect("first close ownership");
            owner_tx.send(()).expect("report owner");
            release_rx.recv().expect("release owner");
            drop(owner);
        });
        owner_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("first caller owns the close gate");

        let (attempted_tx, attempted_rx) = mpsc::channel();
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let second_gate = Arc::clone(&gate);
        let second = thread::spawn(move || {
            attempted_tx.send(()).expect("report attempt");
            let owner = second_gate
                .enter(&OperationContext::new())
                .expect("second close ownership");
            acquired_tx.send(()).expect("report acquisition");
            drop(owner);
        });
        attempted_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("second caller attempts close");
        assert_eq!(
            acquired_rx.recv_timeout(Duration::from_millis(20)),
            Err(mpsc::RecvTimeoutError::Timeout),
            "the second close remains outside the shared session while the first owns it"
        );

        release_tx.send(()).expect("release first close");
        acquired_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("second close proceeds after release");
        first.join().expect("first close caller");
        second.join().expect("second close caller");
    }

    #[test]
    fn a_caller_clock_reentering_the_owned_close_gate_is_rejected_without_deadlock() {
        #[derive(Debug)]
        struct ReentrantClock {
            gate: Arc<CloseGate>,
            entered: AtomicBool,
        }

        impl Clock for ReentrantClock {
            fn now(&self) -> MonotonicInstant {
                if !self.entered.swap(true, Ordering::AcqRel) {
                    let error = self
                        .gate
                        .enter(&OperationContext::new())
                        .expect_err("same-thread reentrant close cannot report success");
                    assert_eq!(error.status(), mado_pilot_core::Status::Closed);
                }
                MonotonicInstant::ORIGIN
            }
        }

        let gate = Arc::new(CloseGate::default());
        let (finished_tx, finished_rx) = mpsc::channel();
        let worker_gate = Arc::clone(&gate);
        let worker = thread::spawn(move || {
            let _owner = worker_gate
                .enter(&OperationContext::new())
                .expect("initial close ownership");
            let clock = Arc::new(ReentrantClock {
                gate: Arc::clone(&worker_gate),
                entered: AtomicBool::new(false),
            });
            Operation::admit(
                &OperationContext::new()
                    .with_clock(clock.clone())
                    .with_deadline(MonotonicInstant::from_origin(Duration::from_secs(1))),
            )
            .expect("the caller clock returns from its reentrant close");
            finished_tx
                .send(clock.entered.load(Ordering::Acquire))
                .expect("report completion");
        });

        assert!(
            finished_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("reentrant close must not deadlock")
        );
        worker.join().expect("close gate worker");
    }
    #[test]
    fn a_caller_clock_reentering_before_close_ownership_never_recurses() {
        #[derive(Debug)]
        struct PanicClock;

        impl Clock for PanicClock {
            fn now(&self) -> MonotonicInstant {
                panic!("a reentrant close consulted caller-owned operation state");
            }
        }

        #[derive(Debug)]
        struct InitialReentrantClock {
            gate: Arc<CloseGate>,
            calls: AtomicUsize,
            rejected: AtomicBool,
        }

        impl Clock for InitialReentrantClock {
            fn now(&self) -> MonotonicInstant {
                if self.calls.fetch_add(1, Ordering::AcqRel) == 0 {
                    let nested = OperationContext::new()
                        .with_clock(Arc::new(PanicClock))
                        .with_deadline(MonotonicInstant::from_origin(Duration::from_secs(1)));
                    let error = self
                        .gate
                        .enter(&nested)
                        .expect_err("same-thread reentry is refused without caller code");
                    self.rejected.store(
                        error.status() == mado_pilot_core::Status::Closed,
                        Ordering::Release,
                    );
                }
                MonotonicInstant::ORIGIN
            }
        }

        let gate = Arc::new(CloseGate::default());
        let clock = Arc::new(InitialReentrantClock {
            gate: Arc::clone(&gate),
            calls: AtomicUsize::new(0),
            rejected: AtomicBool::new(false),
        });
        let context = OperationContext::new()
            .with_clock(clock.clone())
            .with_deadline(MonotonicInstant::from_origin(Duration::from_secs(1)));

        let _owner = gate.enter(&context).expect("outer close owns the gate");

        assert!(clock.rejected.load(Ordering::Acquire));
    }
}
