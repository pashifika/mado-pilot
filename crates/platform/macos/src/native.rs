//! Session ownership, callback fencing, transitions, and bounded teardown.
//!
//! # What the producer callback does
//!
//! Admission, validation, a non-blocking detach into Adapter-owned storage,
//! frame-time geometry, accounting, and publication. It performs no CPU
//! conversion, no matching, no input, no host callback, and no wait: a
//! reconfiguration a resize needs is requested here and carried out by a worker,
//! because the framework's own reconfiguration is asynchronous and completing it
//! on the sample queue would stall delivery.
//!
//! # Lock order
//!
//! Callback admission inside the shim, then this module's transition state, then
//! the detached-storage pool inside the shim, then the stream's own state. No host
//! callback is invoked while any of them is held.

use std::ffi::c_void;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, TryLockError, Weak};
use std::thread;
use std::time::Duration;

use mado_pilot_capture::{
    CaptureFault, CaptureSession, Continuity, CoordinateSupport, Frame, FrameRequest, Lifecycle,
    OverflowPolicy, PixelFormat, QueuePolicy, SessionDescription, StoragePublication, StreamState,
};
use mado_pilot_core::{
    Clock, MonotonicInstant, Operation, OperationContext, PixelExtent, Result, StreamId,
    SystemClock, TargetId, TargetKind, TargetPlacement,
};

use crate::discovery::{NativeKey, PlacementReading, TargetMetadata, read_placement};
use crate::shim::{
    self, BorrowedFrame, DEFAULT_NATIVE_WAIT, DetachedFrame, FrameInfo, MAX_NATIVE_WAIT,
    OpenRequest, ShimStatus,
};
use crate::storage::{DETACHED_BUFFER_BUDGET, MacosFrameStorage, descriptor_from_native};

/// Producer queue depth. Three is the shim's floor and what the framework
/// recommends: deep enough that one slow work item does not starve delivery,
/// shallow enough that a stalled consumer cannot accumulate stale surfaces.
const PRODUCER_QUEUE_DEPTH: u32 = 3;

/// How long a caller contending for the close gate sleeps between attempts.
const CLOSE_POLL_INTERVAL: Duration = Duration::from_millis(2);

/// How many consecutive frames must disagree with the target's live geometry before
/// the producer is asked for a different surface.
///
/// One frame is not enough, and that was measured rather than guessed. While a
/// window is dragged, the window server briefly reports the size it will have on the
/// display it is heading for, so a single frame disagrees and then agrees again.
/// Reconfiguring on that blip changed the surface for one frame, and a surface change
/// is a discontinuity — so a move that kept its extent was published as one, telling
/// a caller its frames were incomparable when they were not.
const UNSETTLED_BEFORE_RECONFIGURE: u32 = 3;

pub(crate) struct NativeSession {
    description: SessionDescription,
    core: Arc<SessionCore>,
    /// The strong reference the shim holds as its callback context.
    ///
    /// Reclaimed only after a successful fence proves no callback can reach it.
    registered: *const SessionCore,
    close_gate: Mutex<()>,
    close_reported: AtomicBool,
}

// SAFETY: `registered` is an `Arc::into_raw` pointer that is only read for its
// address and only consumed in `Drop`. Everything it refers to is `Send`, and the
// pointer itself is never dereferenced outside the shim callbacks, which hold
// their own strong reference through it.
unsafe impl Send for NativeSession {}
// SAFETY: see the Send justification.
unsafe impl Sync for NativeSession {}

struct SessionCore {
    key: NativeKey,
    target_kind: TargetKind,
    state: StreamState,
    session: OnceLock<shim::Session>,
    transition: Mutex<TransitionState>,
    reconfigure: Arc<Reconfigure>,
    clock_anchor: (u64, MonotonicInstant),
}

/// What the stream last received from this Adapter.
///
/// Continuity is decided against this rather than against a flag accumulated from
/// intermediate observations. That distinction is the whole point: while a window is
/// dragged, the extent and the producer surface wobble through values that are never
/// published, and a sticky "discontinuous" flag set by one of those wobbles then
/// attached itself to a frame whose shape was identical to the one before it —
/// telling a caller its frames were incomparable when only the position had changed.
#[derive(Debug, Clone, Copy)]
struct Published {
    extent: PixelExtent,
    placement: TargetPlacement,
}

#[derive(Debug, Clone, Copy)]
struct TransitionState {
    /// The content extent the last frame was observed at.
    extent: PixelExtent,
    /// The producer surface extent the last frame was observed in.
    surface: PixelExtent,
    /// The placement the last frame was observed at.
    placement: TargetPlacement,
    /// Producer timestamp before which a frame predates an observed move.
    movement_not_before: Option<u64>,
    /// Consecutive frames whose extent disagreed with the target's live geometry.
    unsettled_streak: u32,
    published: Option<Published>,
}

/// The reconfiguration request a producer callback leaves for its worker.
#[derive(Debug, Default)]
struct Reconfigure {
    state: Mutex<ReconfigureState>,
    requested: Condvar,
}

#[derive(Debug, Default)]
struct ReconfigureState {
    wanted: Option<PixelExtent>,
    shutdown: bool,
}

impl Reconfigure {
    /// Records a wanted surface extent without blocking the producer callback.
    fn request(&self, extent: PixelExtent) {
        {
            let mut state = self.lock();
            if state.shutdown {
                return;
            }
            // Latest wins: an intermediate size a resize passed through is not
            // worth a native round trip.
            state.wanted = Some(extent);
        }
        self.requested.notify_all();
    }

    fn shutdown(&self) {
        {
            let mut state = self.lock();
            state.shutdown = true;
            state.wanted = None;
        }
        self.requested.notify_all();
    }

    fn lock(&self) -> MutexGuard<'_, ReconfigureState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl NativeSession {
    /// Opens and starts a session for the target `metadata` describes.
    pub(crate) fn open(
        target: TargetId,
        stream: StreamId,
        key: NativeKey,
        metadata: TargetMetadata,
        operation: &mut Operation<'_>,
    ) -> Result<Arc<Self>> {
        Self::open_inner(target, stream, key, metadata, 0, operation)
    }

    /// Opens a session that raises a contained native exception at `sites`.
    ///
    /// This is how the containment and failure-path ownership cases ADR 0012
    /// requires become reachable; nothing in the product asks for a raise site.
    #[cfg(test)]
    pub(crate) fn open_with_raise_sites(
        target: TargetId,
        stream: StreamId,
        key: NativeKey,
        metadata: TargetMetadata,
        sites: u32,
        operation: &mut Operation<'_>,
    ) -> Result<Arc<Self>> {
        Self::open_inner(target, stream, key, metadata, sites, operation)
    }

    fn open_inner(
        target: TargetId,
        stream: StreamId,
        key: NativeKey,
        metadata: TargetMetadata,
        testing_raise_sites: u32,
        operation: &mut Operation<'_>,
    ) -> Result<Arc<Self>> {
        let anchor = clock_calibration().ok_or(CaptureFault::SourceInvalid)?;
        let reconfigure = Arc::new(Reconfigure::default());
        let core = Arc::new(SessionCore {
            key,
            target_kind: key.kind(),
            state: StreamState::with_target_extent(stream),
            session: OnceLock::new(),
            transition: Mutex::new(TransitionState {
                extent: metadata.extent,
                surface: metadata.extent,
                placement: metadata.placement,
                movement_not_before: None,
                unsettled_streak: 0,
                published: None,
            }),
            reconfigure: Arc::clone(&reconfigure),
            clock_anchor: anchor,
        });

        // The shim keeps this address until a fence proves no callback holds it.
        let registered = Arc::into_raw(Arc::clone(&core));
        let request = OpenRequest {
            kind: key.native_kind(),
            native_id: key.native_id(),
            extent: metadata.extent,
            queue_depth: PRODUCER_QUEUE_DEPTH,
            detached_budget: DETACHED_BUFFER_BUDGET.get(),
            wait: native_wait(operation),
            testing_raise_sites,
        };
        let session = match shim::Session::open(
            &request,
            registered.cast::<c_void>().cast_mut(),
            on_frame,
            on_stopped,
        ) {
            Ok(session) => session,
            Err(status) => {
                // The shim registered nothing, so the reference is reclaimed here
                // rather than quarantined.
                // SAFETY: `registered` came from `Arc::into_raw` above and no
                // other owner exists on this path.
                drop(unsafe { Arc::from_raw(registered) });
                return Err(open_error(status, key.kind()));
            }
        };
        core.session
            .set(session)
            .map_err(|_| CaptureFault::SourceInvalid)?;
        operation.checkpoint()?;

        spawn_reconfigure_worker(&core, reconfigure);

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
        let session = Arc::new(Self {
            description,
            core,
            registered,
            close_gate: Mutex::new(()),
            close_reported: AtomicBool::new(false),
        });

        session
            .core
            .session()
            .start(native_wait(operation))
            .map_err(|status| open_error(status, key.kind()))?;
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
        let closed = session.close(native_wait(&attempt));
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
        // Admitted before the liveness read, not after. That read is a window-server
        // query, so performing it first let an already-cancelled or already-expired
        // request do native work — and, worse, terminate the session on what the query
        // returned. A request the caller has given up on decides nothing.
        Operation::admit(operation)?;
        if !self.core.key.is_present() {
            self.core.fail_native(target_fault(self.core.target_kind));
        }
        self.core.state.frame(request, operation)
    }

    fn close(&self, operation: &OperationContext) -> Result<()> {
        self.core.state.begin_close();
        self.core.session().disable_callbacks();
        self.core.reconfigure.shutdown();
        let _gate = lock_with_operation(&self.close_gate, operation)?;
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
        let fenced = session.fence(DEFAULT_NATIVE_WAIT);
        let _closed = session.close(DEFAULT_NATIVE_WAIT);
        if fenced.is_ok() {
            // SAFETY: the fence returned, so the shim admits no further callback
            // and none is in flight; this consumes the single reference handed to
            // it at open.
            drop(unsafe { Arc::from_raw(self.registered) });
        }
        // Otherwise the reference stays quarantined: a callback that is still in
        // flight would read freed state, and one leaked session core is a bounded
        // cost against that.
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
        self.session().disable_callbacks();
        self.reconfigure.shutdown();
        self.state.terminate(fault);
    }

    fn on_frame(&self, borrowed: &BorrowedFrame<'_>, info: &FrameInfo) -> ShimStatus {
        match self.process_frame(borrowed, info) {
            Ok(()) => ShimStatus::Ok,
            Err(fault) => {
                if fault == CaptureFault::SessionClosed && self.state.lifecycle() != Lifecycle::Open
                {
                    // An ordinary close raced this callback. Nothing failed.
                    return ShimStatus::Closed;
                }
                self.fail_native(fault);
                ShimStatus::PlatformFailure
            }
        }
    }

    fn process_frame(
        &self,
        borrowed: &BorrowedFrame<'_>,
        info: &FrameInfo,
    ) -> std::result::Result<(), CaptureFault> {
        let mut transition = match self.transition.try_lock() {
            Ok(transition) => transition,
            // Another transition is committing. Dropping is always safe and the
            // alternative is waiting inside the producer callback.
            Err(TryLockError::WouldBlock) => return Ok(()),
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };
        let extent = info.extent().ok_or(CaptureFault::InconsistentDescriptor)?;
        let surface = info
            .surface_extent()
            .ok_or(CaptureFault::InconsistentDescriptor)?;

        // A requested reconfiguration taking effect changes the producer's
        // container, not what this Adapter publishes: the content is cropped out of
        // it either way. So the new surface is recorded and nothing is concluded
        // from it — the content extent below is what a caller sees.
        transition.surface = surface;

        // Read before any reconfiguration is asked for, because the size to ask for
        // comes from the live target rather than from this frame.
        let reading = read_placement(self.key, extent, info.scale_factor);
        if let Some(wanted) = surface_request(surface, reading.wanted()) {
            // The worker performs the native call off this queue.
            self.reconfigure.request(wanted);
        }
        if extent != transition.extent {
            transition.extent = extent;
            let _drop = self.state.try_record_drop();
            return Ok(());
        }

        let placement = match reading {
            PlacementReading::Ready { placement, .. } => placement,
            // The window server has already resized or moved the target since this
            // frame was produced — which is what a drag between displays looks
            // like, because the target is resized a moment after it lands. The
            // frame is a transition frame, so it is dropped and the producer is
            // asked for a surface matching the target as it is now. Reporting this
            // as target loss, which an earlier version did, ends a stream whose
            // target is plainly still there.
            PlacementReading::Unsettled { wanted } => {
                // No continuity is decided here. Whether the next publication is a
                // geometry change or a discontinuity follows from what the extent
                // turns out to be once the producer and the window agree again, and
                // the ordinary comparisons above already answer that. Deciding it
                // here published a move that kept its extent as a discontinuity.
                transition.unsettled_streak = transition.unsettled_streak.saturating_add(1);
                if should_reconfigure(transition.unsettled_streak)
                    && let Some(wanted) = wanted
                {
                    self.reconfigure.request(wanted);
                }
                let _drop = self.state.try_record_drop();
                return Ok(());
            }
            PlacementReading::Unusable => return Err(CaptureFault::InconsistentDescriptor),
            PlacementReading::Lost => return Err(target_fault(self.target_kind)),
        };
        transition.unsettled_streak = 0;
        if placement != transition.placement {
            // A move was just observed. This frame's pixels predate it, so it is
            // dropped and frames older than the move are refused below until the
            // producer catches up.
            //
            // Not gated on having published before. A target that moves while capture
            // is starting is the same move, and skipping the fence for it published a
            // queued pre-move frame under the placement read after the move — every
            // desktop coordinate taken from that frame silently displaced.
            transition.placement = placement;
            transition.movement_not_before =
                Some(shim::monotonic_nanos().unwrap_or(info.display_time_nanos.saturating_add(1)));
            let _drop = self.state.try_record_drop();
            return Ok(());
        }
        if let Some(not_before) = transition.movement_not_before {
            if info.display_time_nanos < not_before {
                // This frame was produced before the move was observed, so its
                // pixels do not belong to the geometry now recorded.
                let _drop = self.state.try_record_drop();
                return Ok(());
            }
            transition.movement_not_before = None;
        }

        let descriptor = descriptor_from_native(info.pixel_format, extent)?;
        let detached = match borrowed.detach() {
            Ok(detached) => detached,
            // Finite pressure: every unit of the budget is retained by a caller.
            // The candidate is dropped rather than blocking the producer.
            Err(ShimStatus::BudgetExhausted) => {
                let _drop = self.state.try_record_drop();
                return Ok(());
            }
            Err(ShimStatus::FrameIncomplete) => {
                let _drop = self.state.try_record_drop();
                return Ok(());
            }
            Err(status) => return Err(status.fault()),
        };

        let continuity = continuity_against(transition.published, extent, placement);
        self.publish(
            &mut transition,
            detached,
            descriptor,
            placement,
            continuity,
            info.display_time_nanos,
        )
    }

    fn publish(
        &self,
        transition: &mut TransitionState,
        detached: DetachedFrame,
        descriptor: mado_pilot_capture::FrameDescriptor,
        placement: TargetPlacement,
        continuity: Continuity,
        display_time_nanos: u64,
    ) -> std::result::Result<(), CaptureFault> {
        let storage = MacosFrameStorage::new(descriptor, detached);
        self.state
            .publish_storage(StoragePublication {
                captured_at: frame_time(self.clock_anchor, display_time_nanos),
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
        transition.published = Some(Published {
            extent: descriptor.extent(),
            placement,
        });
        Ok(())
    }

    fn on_stopped(&self, status: ShimStatus) {
        let fault = match status {
            // The framework names a deliberate stop, so it is reported as one
            // rather than as a target that went away.
            ShimStatus::StoppedByUser => CaptureFault::ExplicitlyStopped,
            // The system ended the stream without naming a cause. Revoked
            // authorization is one cause among several, so it is established by
            // reading the authorization again rather than assumed from the stop.
            ShimStatus::StoppedBySystem => system_stop_fault(),
            ShimStatus::PermissionDenied => CaptureFault::AccessDenied,
            // The stream ended with nothing to distinguish it by. A producer that
            // stops without an error most often means its target is gone, and the
            // kind-specific loss is the conservative reading of that.
            ShimStatus::Ok | ShimStatus::Closed => target_fault(self.target_kind),
            other => other.fault(),
        };
        self.fail_native(fault);
    }
}

fn spawn_reconfigure_worker(core: &Arc<SessionCore>, reconfigure: Arc<Reconfigure>) {
    let weak = Arc::downgrade(core);
    let owned = Arc::clone(&reconfigure);
    let worker = thread::Builder::new()
        .name("mado-pilot-sck-reconfigure".to_owned())
        .spawn(move || run_reconfigure_worker(&weak, &owned));
    if worker.is_err() {
        // Without a worker the session still captures at its opening extent; a
        // resize then reports a discontinuity and keeps publishing the content
        // that fits. Shutting the request path down says so rather than letting
        // callbacks queue requests nothing will read.
        reconfigure.shutdown();
    }
}

fn run_reconfigure_worker(core: &Weak<SessionCore>, reconfigure: &Reconfigure) {
    loop {
        let wanted = {
            let mut state = reconfigure.lock();
            while state.wanted.is_none() && !state.shutdown {
                let (next, _timeout) = reconfigure
                    .requested
                    .wait_timeout(state, CLOSE_POLL_INTERVAL.saturating_mul(50))
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                state = next;
                if state.wanted.is_none() && core.strong_count() == 0 {
                    return;
                }
            }
            if state.shutdown {
                return;
            }
            state.wanted.take()
        };
        let Some(extent) = wanted else {
            continue;
        };
        let Some(core) = core.upgrade() else {
            return;
        };
        // The native call happens here rather than in the producer callback, and
        // its own failure is not terminal: the session keeps publishing the
        // content that fits the surface it already has.
        let _reconfigured = core.session().reconfigure(extent, MAX_NATIVE_WAIT);
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
            |core, borrowed, report| core.on_frame(&borrowed, report),
        )
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
) -> Continuity {
    match published {
        // Nothing has been published, so there is nothing to be discontinuous with.
        None => Continuity::Continuous,
        Some(last) if last.extent != extent => Continuity::Discontinuous,
        Some(last) if last.placement != placement => Continuity::GeometryChanged,
        Some(_) => Continuity::Continuous,
    }
}

/// Reports whether a run of frames disagreeing with live geometry has lasted long
/// enough to be worth a native reconfiguration.
const fn should_reconfigure(streak: u32) -> bool {
    streak >= UNSETTLED_BEFORE_RECONFIGURE
}

/// Returns the producer surface extent to ask for, if any.
///
/// The question is only ever whether the surface is the size the *target* needs at its
/// own display's backing scale. What the frame in hand contains cannot answer it, in
/// either direction:
///
/// - A surface too small to hold the target is filled anyway, because the framework
///   scales the target down to fit and reports a scale factor absorbing the reduction.
///   Asking for the reported content extent adopts that reduction, and the next frame
///   scales into the smaller surface and reports less again. Measured on a window moved
///   onto a higher-scale display: the epoch advanced ten times inside a second while
///   frames published at 94% of the target's resolution.
/// - Reading a filled surface as "nothing to do" is the same mistake standing still. A
///   window moved from a 1x display to a 2x one without changing its point size needs
///   twice the pixels, and the framework will downscale into the old surface and fill it
///   exactly. Treating that as settled captures the target at half resolution for the
///   life of the stream.
///
/// So the comparison is `wanted` against `surface`, and the content extent does not
/// enter into it.
fn surface_request(surface: PixelExtent, wanted: Option<PixelExtent>) -> Option<PixelExtent> {
    match wanted {
        // The surface is not the size the target needs, whatever it currently holds.
        Some(wanted) if wanted != surface => Some(wanted),
        // Either the surface is already right, or the target could not be read and
        // there is nothing to size a request from. The content extent is exactly the
        // wrong answer in that second case, so nothing is asked for.
        _ => None,
    }
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
                thread::sleep(CLOSE_POLL_INTERVAL);
                attempt.checkpoint()?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use mado_pilot_capture::CaptureFault;
    use mado_pilot_core::{
        CancellationToken, MonotonicInstant, Operation, OperationContext, PixelExtent, Scale,
        TargetKind, TargetPlacement,
    };

    use super::{
        MAX_NATIVE_WAIT, Published, Reconfigure, UNSETTLED_BEFORE_RECONFIGURE, continuity_against,
        frame_time, native_wait, normalize_native_fault, should_reconfigure, surface_request,
        target_fault,
    };

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

    #[test]
    fn a_move_that_keeps_its_extent_is_a_geometry_change_and_not_a_new_epoch() {
        let extent = PixelExtent::new(1718, 1050);
        let scale = Scale::new(1.0, 1.0).expect("scale");
        let at =
            |x: f64| TargetPlacement::new((x, 400.0), (1718.0, 1050.0), scale).expect("placement");
        let published = Published {
            extent,
            placement: at(-2041.0),
        };

        // The case a real drag produced: same extent, same scale, new origin. An
        // epoch advance here tells a caller its frames are incomparable when only
        // the position changed.
        assert_eq!(
            continuity_against(Some(published), extent, at(-2489.0)),
            mado_pilot_capture::Continuity::GeometryChanged
        );
        // The same frame again is continuous, whatever wobbled in between.
        assert_eq!(
            continuity_against(Some(published), extent, at(-2041.0)),
            mado_pilot_capture::Continuity::Continuous
        );
        // A different extent is discontinuous, which is the cross-scale move.
        assert_eq!(
            continuity_against(Some(published), PixelExtent::new(3436, 2216), at(-270.0)),
            mado_pilot_capture::Continuity::Discontinuous
        );
        // Nothing published yet cannot be discontinuous with anything.
        assert_eq!(
            continuity_against(None, extent, at(0.0)),
            mado_pilot_capture::Continuity::Continuous
        );
    }

    #[test]
    fn one_frame_disagreeing_with_live_geometry_does_not_reconfigure() {
        // The blip a drag produces. Reconfiguring on it changes the producer
        // surface, and a surface change is a discontinuity, so acting on a single
        // frame turned a move that kept its extent into an epoch advance.
        assert!(!should_reconfigure(1));
        assert!(!should_reconfigure(UNSETTLED_BEFORE_RECONFIGURE - 1));
        assert!(should_reconfigure(UNSETTLED_BEFORE_RECONFIGURE));
        assert!(should_reconfigure(u32::MAX));
        // A single disagreeing frame must never be enough, which is what the first
        // assertion above already pins for the value this build compiled with.
        const { assert!(UNSETTLED_BEFORE_RECONFIGURE > 1) }
    }

    #[test]
    fn a_surface_too_small_for_its_target_is_asked_to_grow() {
        // Measured on a 1718x1108-point window moved onto a backing-scale-2 display
        // while the producer still held the surface it had at scale 1: the framework
        // scaled the target into the small surface and reported 1623x1047 pixels at
        // scale 0.9449. Asking for that reported extent adopts the reduction.
        let surface = PixelExtent::new(1718, 1050);
        let target = PixelExtent::new(3436, 2216);

        assert_eq!(surface_request(surface, Some(target)), Some(target));
    }

    #[test]
    fn a_filled_surface_at_the_wrong_scale_is_still_asked_to_grow() {
        // The case a content-extent comparison cannot see. A window keeps its point
        // size across a move from a 1x display to a 2x one, so it needs twice the
        // pixels — and the framework downscales into the old surface and fills it
        // exactly. Reading a filled surface as settled captures the target at half
        // resolution for the life of the stream.
        let surface = PixelExtent::new(1000, 800);
        let target = PixelExtent::new(2000, 1600);

        assert_eq!(
            surface_request(surface, Some(target)),
            Some(target),
            "a surface the content fills can still be the wrong size for the target"
        );
    }

    #[test]
    fn a_surface_already_the_size_the_target_needs_is_not_asked_for_again() {
        // Content short of a correctly sized surface is the framework mid-resize.
        // Asking again would spend a native round trip per frame to no effect.
        let surface = PixelExtent::new(3436, 2216);

        assert_eq!(surface_request(surface, Some(surface)), None);
    }

    #[test]
    fn a_surface_request_needs_a_live_reading_to_size_it() {
        // A target that could not be read gives nothing to size a request from, and
        // the content extent is exactly the wrong answer, so nothing is asked for.
        let surface = PixelExtent::new(1718, 1050);

        assert_eq!(surface_request(surface, None), None);
    }

    #[test]
    fn a_reconfiguration_request_keeps_only_the_latest_extent() {
        let reconfigure = Reconfigure::default();

        reconfigure.request(PixelExtent::new(100, 100));
        reconfigure.request(PixelExtent::new(200, 200));

        assert_eq!(
            reconfigure.lock().wanted,
            Some(PixelExtent::new(200, 200)),
            "an intermediate size a resize passed through is not worth a round trip"
        );
    }

    #[test]
    fn a_request_after_shutdown_is_refused_so_nothing_queues_unread() {
        let reconfigure = Arc::new(Reconfigure::default());

        reconfigure.shutdown();
        reconfigure.request(PixelExtent::new(64, 64));

        let state = reconfigure.lock();
        assert!(state.shutdown);
        assert_eq!(state.wanted, None);
    }

    #[test]
    fn a_cancelled_close_gate_reports_cancellation_rather_than_waiting() {
        let token = CancellationToken::new();
        token.cancel();
        let context = OperationContext::new().with_cancellation(token);
        let gate = std::sync::Mutex::new(());

        let error = super::lock_with_operation(&gate, &context).expect_err("cancelled");

        assert_eq!(error.status(), mado_pilot_core::Status::Cancelled);
    }
}
