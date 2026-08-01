//! Authoritative stream publication state.
//!
//! This is where a capture adapter's frames become published frames. Every
//! adapter drives the same state machine, so epochs, sequences, geometry
//! revisions, latest-frame semantics, and close behavior cannot differ between
//! a replay source and a native one.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use mado_pilot_core::{
    Error, FrameOrder, FrameStamp, GeometryRevision, MonotonicInstant, Operation, OperationContext,
    Result, Status, StreamCursor, StreamId, TargetPlacement, TransformSnapshot,
};

use crate::descriptor::FrameDescriptor;
use crate::fault::CaptureFault;
use crate::frame::Frame;
use crate::storage::{CpuFrameStorage, FrameStorage};

/// How long a waiter sleeps before re-checking its operation context.
///
/// A waiter cannot block on the condition variable for the whole remaining
/// deadline, because the deadline is measured on the operation's own clock and a
/// test drives that clock by hand. Waking periodically keeps one loop correct
/// for both a real clock and a synthetic one; the interval bounds how late an
/// interruption is noticed, not how long an operation may take.
const POLL_INTERVAL: Duration = Duration::from_millis(2);

/// How a published frame relates to the one before it.
///
/// The adapter states its intent and the stream state enforces the consequences,
/// so no adapter can accidentally keep an epoch running across a change that
/// makes pixel comparison meaningless.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Continuity {
    /// Pixels stay comparable and geometry is unchanged. Identical pixels are
    /// still a new frame with the next sequence.
    Continuous,
    /// Placement or scale changed, but the pixels remain comparable.
    GeometryChanged,
    /// Extent or pixel representation changed, so comparison with the previous
    /// frame is meaningless.
    Discontinuous,
}

/// One frame an adapter is asking the stream to publish.
#[derive(Debug)]
pub struct Publication {
    /// When the frame was captured, in the engine's monotonic domain.
    pub captured_at: MonotonicInstant,
    /// The extent, format, and stride of `pixels`.
    pub descriptor: FrameDescriptor,
    /// Target placement, when the source declares an authoritative one.
    pub placement: Option<TargetPlacement>,
    /// The frame's pixels.
    pub pixels: Box<[u8]>,
    /// How this frame relates to the previous one.
    pub continuity: Continuity,
}

/// A stream refusal that returns the Adapter's complete owned publication.
///
/// [`StreamState::publish_recoverable`] returns this value when publication
/// fails before any authoritative stream state is committed. The Adapter may
/// inspect the public error, retry or restore the unchanged publication, or
/// consume both with [`RefusedPublication::into_parts`].
pub struct RefusedPublication {
    error: Error,
    publication: Publication,
}

impl RefusedPublication {
    fn new(error: Error, publication: Publication) -> Self {
        Self { error, publication }
    }

    /// Returns the public error that refused publication.
    #[must_use]
    pub const fn error(&self) -> &Error {
        &self.error
    }

    /// Returns the unchanged owned publication.
    #[must_use]
    pub const fn publication(&self) -> &Publication {
        &self.publication
    }

    /// Consumes the refusal and returns only its public error.
    #[must_use]
    pub fn into_error(self) -> Error {
        self.error
    }

    /// Consumes the refusal and returns only the unchanged publication.
    #[must_use]
    pub fn into_publication(self) -> Publication {
        self.publication
    }

    /// Consumes the refusal and returns its public error and unchanged
    /// publication.
    #[must_use]
    pub fn into_parts(self) -> (Error, Publication) {
        (self.error, self.publication)
    }
}

impl fmt::Debug for RefusedPublication {
    /// Formats the refusal and safe publication metadata, never pixel content.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RefusedPublication")
            .field("status", &self.error.status())
            .field("detail", &self.error.detail())
            .field("captured_at", &self.publication.captured_at)
            .field("descriptor", &self.publication.descriptor)
            .field("placement", &self.publication.placement)
            .field("continuity", &self.publication.continuity)
            .field("bytes", &self.publication.pixels.len())
            .finish()
    }
}

/// One frame an Adapter is asking the stream to publish, as owned storage.
///
/// The native counterpart of [`Publication`]. There is no descriptor field: the
/// storage carries its own, and a second answer to what shape the pixels are
/// could disagree with it.
#[derive(Debug)]
pub struct StoragePublication {
    /// When the frame was captured, in the engine's monotonic domain.
    pub captured_at: MonotonicInstant,
    /// Target placement, when the source declares an authoritative one.
    pub placement: Option<TargetPlacement>,
    /// The frame's immutable storage, independent of whatever produced it.
    pub storage: Arc<dyn FrameStorage>,
    /// How this frame relates to the previous one.
    pub continuity: Continuity,
}

/// A stream refusal that returns the Adapter's unchanged storage.
///
/// [`StreamState::publish_storage`] returns this when publication fails before any
/// authoritative stream state is committed. An Adapter that pools or leases its
/// storage needs the value back to retire or reuse it, and dropping it inside the
/// stream would release a lease the Adapter is still accounting for.
pub struct RefusedStorage {
    error: Error,
    publication: StoragePublication,
}

impl RefusedStorage {
    fn new(error: Error, publication: StoragePublication) -> Self {
        Self { error, publication }
    }

    /// Returns the public error that refused publication.
    #[must_use]
    pub const fn error(&self) -> &Error {
        &self.error
    }

    /// Returns the unchanged publication.
    #[must_use]
    pub const fn publication(&self) -> &StoragePublication {
        &self.publication
    }

    /// Consumes the refusal and returns only its public error.
    #[must_use]
    pub fn into_error(self) -> Error {
        self.error
    }

    /// Consumes the refusal and returns only the unchanged publication.
    #[must_use]
    pub fn into_publication(self) -> StoragePublication {
        self.publication
    }

    /// Consumes the refusal and returns its error and unchanged publication.
    #[must_use]
    pub fn into_parts(self) -> (Error, StoragePublication) {
        (self.error, self.publication)
    }
}

impl fmt::Debug for RefusedStorage {
    /// Formats the refusal and safe publication metadata, never pixel content.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RefusedStorage")
            .field("status", &self.error.status())
            .field("detail", &self.error.detail())
            .field("captured_at", &self.publication.captured_at)
            .field("descriptor", &self.publication.storage.descriptor())
            .field("placement", &self.publication.placement)
            .field("continuity", &self.publication.continuity)
            .finish()
    }
}

/// Which frame a caller wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameSelection {
    /// The session's current published frame, waiting if none exists yet.
    Latest,
    /// A frame strictly newer than this one.
    NewerThan(FrameStamp),
}

/// A request for a frame from a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameRequest {
    selection: FrameSelection,
}

impl FrameRequest {
    /// Asks for the current published frame.
    #[must_use]
    pub const fn latest() -> Self {
        Self {
            selection: FrameSelection::Latest,
        }
    }

    /// Asks for a frame published after `stamp`.
    #[must_use]
    pub const fn newer_than(stamp: FrameStamp) -> Self {
        Self {
            selection: FrameSelection::NewerThan(stamp),
        }
    }

    /// Returns the selection.
    #[must_use]
    pub const fn selection(&self) -> FrameSelection {
        self.selection
    }
}

impl Default for FrameRequest {
    fn default() -> Self {
        Self::latest()
    }
}

/// Where a stream is in its lifecycle.
///
/// Re-exported from the core package, where it now lives so that a capture stream
/// and an input controller answer the question with one type. The name keeps its
/// place here because it is part of this package's published surface.
pub use mado_pilot_core::Lifecycle;

/// The published state of one capture stream.
#[derive(Debug)]
pub struct StreamState {
    inner: Mutex<Inner>,
    published: Condvar,
    covers_target: bool,
    pending_drops: AtomicU64,
    published_any: AtomicBool,
    accepting_drops: AtomicBool,
}

#[derive(Debug)]
struct Inner {
    cursor: StreamCursor,
    geometry: GeometryRevision,
    latest: Option<Frame>,
    lifecycle: Lifecycle,
    waiters: usize,
    terminal: Option<CaptureFault>,
}

impl StreamState {
    /// Starts a stream at epoch zero with nothing published.
    #[must_use]
    pub fn new(stream: StreamId) -> Self {
        Self::build(stream, false)
    }

    /// Starts a stream whose frames cover their target.
    ///
    /// No extent is taken, for the reason
    /// [`TransformSnapshot::with_target_extent`] takes none: a frame that covers
    /// its target has the target's extent, and a magnitude pinned at open would
    /// contradict every frame published after a resize.
    #[must_use]
    pub fn with_target_extent(stream: StreamId) -> Self {
        Self::build(stream, true)
    }

    fn build(stream: StreamId, covers_target: bool) -> Self {
        Self {
            inner: Mutex::new(Inner {
                cursor: StreamCursor::new(stream),
                geometry: GeometryRevision::FIRST,
                latest: None,
                lifecycle: Lifecycle::Open,
                waiters: 0,
                terminal: None,
            }),
            published: Condvar::new(),
            covers_target,
            pending_drops: AtomicU64::new(0),
            published_any: AtomicBool::new(false),
            accepting_drops: AtomicBool::new(true),
        }
    }

    /// Returns the stream identity.
    #[must_use]
    pub fn stream(&self) -> StreamId {
        self.lock().cursor.stream()
    }

    /// Returns where the stream is in its lifecycle.
    #[must_use]
    pub fn lifecycle(&self) -> Lifecycle {
        self.lock().lifecycle
    }

    /// Returns the current published frame, without waiting.
    #[must_use]
    pub fn current(&self) -> Option<Frame> {
        self.lock().latest.clone()
    }

    /// Publishes one frame and returns it.
    ///
    /// The stream, not the adapter, decides the resulting identity. An extent or
    /// format change is treated as discontinuous whatever the adapter claimed,
    /// because an epoch that spanned one would invite a consumer to compare
    /// pixels that describe different rectangles. A change to the frame's
    /// transform metadata is treated as a geometry change on the same terms: the
    /// revision is what tells a caller which transform an answer was computed
    /// against, so two frames carrying different transforms may not share one.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureFault::SessionClosed`] once close has begun,
    /// [`CaptureFault::InconsistentDescriptor`] for a placement that does not
    /// scale to the published extent, and an identity fault when an epoch or
    /// sequence counter is exhausted.
    pub fn publish(&self, publication: Publication) -> Result<Frame> {
        self.publish_recoverable(publication)
            .map_err(RefusedPublication::into_error)
    }

    /// Publishes one frame, returning the owned publication on refusal.
    ///
    /// This has the same success, identity, validation, and state-commit rules
    /// as [`StreamState::publish`]. Unlike that convenience operation, a
    /// refusal returns both the public error and the complete unchanged
    /// [`Publication`], allowing an Adapter to restore expensive owned storage
    /// without copying it before publication.
    ///
    /// # Errors
    ///
    /// Returns [`RefusedPublication`] for every ordinary publication error.
    /// No lifecycle-adjacent state, cursor, epoch, sequence, geometry revision,
    /// or current frame is committed when this operation refuses a publication.
    ///
    /// The error is intentionally returned inline: boxing it would add an
    /// allocation exactly when an Adapter needs ownership recovery.
    #[allow(clippy::result_large_err)]
    pub fn publish_recoverable(
        &self,
        publication: Publication,
    ) -> std::result::Result<Frame, RefusedPublication> {
        let inner = self.lock();
        let prepared = match prepare(
            &inner,
            self.covers_target,
            publication.descriptor,
            publication.placement,
            publication.continuity,
            Some(publication.pixels.len()),
            self.pending_drops.load(Ordering::Acquire),
        ) {
            Ok(prepared) => prepared,
            Err(error) => return Err(RefusedPublication::new(error, publication)),
        };

        let Publication {
            captured_at,
            descriptor,
            pixels,
            ..
        } = publication;
        // The bytes become storage only after every rule has passed, so a refusal
        // above hands the Adapter back exactly what it passed in. The length was
        // checked while the caller still owned them, which is what lets this build
        // storage without a second failure path.
        let storage = Arc::new(CpuFrameStorage::from_validated(descriptor, pixels));

        Ok(self.commit(inner, prepared, captured_at, descriptor, storage))
    }

    /// Publishes Adapter-owned immutable storage as the stream's next frame.
    ///
    /// This is the entry a native Adapter uses. It has the same identity,
    /// continuity, geometry, validation, and commit rules as
    /// [`StreamState::publish`]: the stream decides the resulting identity, and an
    /// extent, format, or transform change is treated as the discontinuity it is
    /// whatever the Adapter claimed.
    ///
    /// # Errors
    ///
    /// Returns [`RefusedStorage`] for every publication error, carrying the
    /// unchanged storage back to the Adapter. Nothing is committed when a
    /// publication is refused, so the Adapter may retire or reuse the storage
    /// under its own ownership rule.
    #[allow(clippy::result_large_err)]
    pub fn publish_storage(
        &self,
        publication: StoragePublication,
    ) -> std::result::Result<Frame, RefusedStorage> {
        self.publish_storage_with(publication, |_| {})
    }

    /// Publishes Adapter-owned immutable storage after committing correlated
    /// Adapter metadata at the same observable boundary.
    ///
    /// `before_observe` runs with the prepared frame while the stream mutex still
    /// excludes readers, before the frame is installed as `latest` and before any
    /// waiter is notified. Native Adapters use this narrow hook to publish metadata
    /// indexed by the new [`FrameStamp`] without creating a window in which the
    /// frame is observable but that metadata is not. The hook must be bounded and
    /// must not call back into this `StreamState`.
    ///
    /// # Errors
    ///
    /// Returns [`RefusedStorage`] under the same conditions as
    /// [`StreamState::publish_storage`]. `before_observe` is not invoked for a
    /// refused publication.
    #[allow(clippy::result_large_err)]
    pub fn publish_storage_with<F>(
        &self,
        publication: StoragePublication,
        before_observe: F,
    ) -> std::result::Result<Frame, RefusedStorage>
    where
        F: FnOnce(&Frame),
    {
        // Read the Adapter's own value before taking the stream mutex. The
        // documented lock order is platform callback state, then detached storage
        // ownership, then this mutex, and nothing about a fixed descriptor needs to
        // be read under it. Keeping the order exact leaves no exception for a later
        // reader to weigh.
        let descriptor = publication.storage.descriptor();
        let inner = self.lock();
        let prepared = match prepare(
            &inner,
            self.covers_target,
            descriptor,
            publication.placement,
            publication.continuity,
            None,
            self.pending_drops.load(Ordering::Acquire),
        ) {
            Ok(prepared) => prepared,
            Err(error) => return Err(RefusedStorage::new(error, publication)),
        };

        let StoragePublication {
            captured_at,
            storage,
            ..
        } = publication;
        Ok(self.commit_with(
            inner,
            prepared,
            captured_at,
            descriptor,
            storage,
            before_observe,
        ))
    }

    /// Records one candidate frame that a finite Adapter path had to drop.
    ///
    /// The next successful same-epoch publication advances past every recorded
    /// drop, so a waiter observes a sequence gap while still receiving only a
    /// real immutable frame. A discontinuous publication must begin its new
    /// epoch at sequence `FrameSequence::FIRST`, so it preserves the debt for
    /// the first later non-discontinuous publication; repeated discontinuities
    /// preserve it in the same way. Before the first publication there is no
    /// frame identity against which a caller could observe a gap, so the
    /// candidate is dropped without accounting and this returns `Ok(false)`.
    ///
    /// This operation is lock-free. It is intended for a native producer callback
    /// whose bounded path may not block when every retained-storage lease is
    /// occupied or another publication is committing.
    ///
    /// # Errors
    ///
    /// Returns a limit-exceeded outcome if the pending-drop counter is exhausted.
    /// `Ok(false)` means no frame has yet been published.
    pub fn try_record_drop(&self) -> Result<bool> {
        if !self.accepting_drops.load(Ordering::Acquire) {
            return Err(CaptureFault::SessionClosed.into());
        }
        if !self.published_any.load(Ordering::Acquire) {
            return Ok(false);
        }
        self.pending_drops
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |pending| {
                pending.checked_add(1)
            })
            .map_err(|_| Error::new(Status::LimitExceeded, "pending frame drops exhausted"))?;
        Ok(true)
    }

    /// Commits one validated publication and wakes the stream's waiters.
    fn commit(
        &self,
        inner: MutexGuard<'_, Inner>,
        prepared: Prepared,
        captured_at: MonotonicInstant,
        descriptor: FrameDescriptor,
        storage: Arc<dyn FrameStorage>,
    ) -> Frame {
        self.commit_with(inner, prepared, captured_at, descriptor, storage, |_| {})
    }

    /// Installs one prepared frame only after its correlated metadata is ready.
    fn commit_with<F>(
        &self,
        mut inner: MutexGuard<'_, Inner>,
        prepared: Prepared,
        captured_at: MonotonicInstant,
        descriptor: FrameDescriptor,
        storage: Arc<dyn FrameStorage>,
        before_observe: F,
    ) -> Frame
    where
        F: FnOnce(&Frame),
    {
        let frame = Frame::from_validated(
            prepared.stamp,
            captured_at,
            descriptor,
            prepared.transform,
            storage,
        );

        before_observe(&frame);
        inner.cursor = prepared.cursor;
        inner.geometry = prepared.geometry;
        inner.latest = Some(frame.clone());
        if prepared.consumed_drops > 0 {
            self.pending_drops
                .fetch_sub(prepared.consumed_drops, Ordering::AcqRel);
        }
        self.published_any.store(true, Ordering::Release);
        drop(inner);
        self.published.notify_all();
        frame
    }

    /// Returns the frame `request` asks for, waiting when necessary.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureFault::ForeignStream`] for a stamp from another stream,
    /// [`CaptureFault::SessionClosed`] when the stream closes while waiting, and
    /// the operation's terminal outcome when cancellation or the deadline wins.
    pub fn frame(&self, request: &FrameRequest, operation: &OperationContext) -> Result<Frame> {
        let mut attempt = Operation::admit(operation)?;
        loop {
            {
                let mut inner = self.lock();
                if let Some(terminal) = inner.terminal {
                    // The terminal fault outranks the closed outcome: a caller
                    // needs to know that capture ended because the target was lost,
                    // not that the session it was using is no longer open.
                    return Err(terminal.into());
                }
                if inner.lifecycle != Lifecycle::Open {
                    return Err(CaptureFault::SessionClosed.into());
                }
                if let Some(frame) = qualifying(&inner, request)? {
                    drop(inner);
                    return Ok(attempt.commit(frame)?);
                }
                inner.waiters += 1;
                let (mut inner, _) = self
                    .published
                    .wait_timeout(inner, POLL_INTERVAL)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                inner.waiters -= 1;
                if inner.waiters == 0 {
                    drop(inner);
                    // A drain may be waiting for the last waiter to leave.
                    self.published.notify_all();
                }
            }
            // The operation context is consulted with no lock held: its clock is
            // supplied by the caller, and calling out under a lock is how
            // deadlocks are built.
            attempt.checkpoint()?;
        }
    }

    /// Ends the stream with a typed terminal fault.
    ///
    /// This is how an Adapter reports that capture stopped for a reason of its own
    /// — the target closed, a display was disconnected, a device was lost,
    /// authorization was revoked — rather than because a caller closed the
    /// session. The fault is committed into the same ordered state as publication,
    /// so every waiter observes it after the last frame that was actually
    /// published, and no frame can be published after it.
    ///
    /// Admission stops immediately, which is what makes the outcome ordered: a
    /// caller that then asks for a frame is told why capture ended rather than
    /// that the session is closed. The first fault recorded is the one reported,
    /// because the first thing that went wrong is the explanation and whatever it
    /// caused afterwards is not.
    ///
    /// Idempotent, and never moves a closed stream backwards.
    pub fn terminate(&self, fault: CaptureFault) {
        self.accepting_drops.store(false, Ordering::Release);
        {
            let mut inner = self.lock();
            if inner.terminal.is_none() {
                inner.terminal = Some(fault);
            }
            if inner.lifecycle == Lifecycle::Open {
                inner.lifecycle = Lifecycle::Closing;
            }
        }
        self.published.notify_all();
    }

    /// Returns the terminal fault, when capture ended in one.
    ///
    /// `None` covers both a running stream and one a caller closed: an ordinary
    /// close is not a fault, and reporting one would make every clean shutdown
    /// look like a failure.
    #[must_use]
    pub fn terminal(&self) -> Option<CaptureFault> {
        self.lock().terminal
    }

    /// Marks the stream as closing, refusing new work and waking every waiter.
    ///
    /// Idempotent, and never moves a closed stream backwards.
    pub fn begin_close(&self) {
        self.accepting_drops.store(false, Ordering::Release);
        {
            let mut inner = self.lock();
            if inner.lifecycle == Lifecycle::Open {
                inner.lifecycle = Lifecycle::Closing;
            }
        }
        self.published.notify_all();
    }

    /// Waits for in-flight frame waits to unwind, then marks the stream closed.
    ///
    /// # Errors
    ///
    /// Returns the operation's terminal outcome when cancellation or the
    /// deadline wins first. The stream then stays in [`Lifecycle::Closing`], so a
    /// later close continues the drain rather than restarting it.
    pub fn drain(&self, operation: &OperationContext) -> Result<()> {
        self.begin_close();
        let mut attempt = Operation::admit(operation)?;
        loop {
            {
                let inner = self.lock();
                if inner.lifecycle == Lifecycle::Closed {
                    drop(inner);
                    return Ok(attempt.commit(())?);
                }
                if inner.waiters == 0 {
                    drop(inner);
                    // The caller clock is invoked by commit, so final arbitration
                    // happens without the stream mutex held and before the
                    // irreversible Closed transition.
                    attempt.commit(())?;
                    let mut inner = self.lock();
                    if inner.lifecycle != Lifecycle::Closed {
                        debug_assert_eq!(inner.lifecycle, Lifecycle::Closing);
                        debug_assert_eq!(inner.waiters, 0);
                        inner.lifecycle = Lifecycle::Closed;
                        inner.latest = None;
                    }
                    return Ok(());
                }
                let _unused = self
                    .published
                    .wait_timeout(inner, POLL_INTERVAL)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
            attempt.checkpoint()?;
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The identity and geometry one publication resolved to, before it commits.
///
/// Every rule that can refuse a publication is applied while producing this, and
/// nothing in the stream has changed yet. That is what lets a refusal hand the
/// Adapter back its untouched bytes or storage: the decision and the commit are
/// separate steps rather than one pass that mutates as it validates.
#[derive(Debug)]
struct Prepared {
    cursor: StreamCursor,
    geometry: GeometryRevision,
    stamp: FrameStamp,
    transform: TransformSnapshot,
    consumed_drops: u64,
}

/// Applies every publication rule against `inner` without committing anything.
///
/// `pixel_len` is `Some` only when the caller still owns loose bytes whose length
/// has to be checked here; storage has already agreed with its own descriptor.
fn prepare(
    inner: &Inner,
    covers_target: bool,
    descriptor: FrameDescriptor,
    placement: Option<TargetPlacement>,
    declared: Continuity,
    pixel_len: Option<usize>,
    pending_drops: u64,
) -> Result<Prepared> {
    if let Some(terminal) = inner.terminal {
        return Err(terminal.into());
    }
    if inner.lifecycle != Lifecycle::Open {
        return Err(CaptureFault::SessionClosed.into());
    }
    if let Some(len) = pixel_len
        && len != descriptor.byte_len()
    {
        return Err(CaptureFault::ByteLengthMismatch.into());
    }

    let current = inner.latest.as_ref();
    let reshaped = current.is_some_and(|frame| {
        let existing = frame.descriptor();
        existing.extent() != descriptor.extent() || existing.format() != descriptor.format()
    });
    // The other half of the same rule. A snapshot is its revision, its frame
    // extent, whether the frame covers its target, and its placement; the
    // revision is being decided here, the extent is what `reshaped` covers,
    // and target coverage follows placement presence because the stream's
    // own coverage is fixed for the session. So a placement that differs
    // from the current frame's is the remaining way the snapshot can change,
    // and an adapter claiming continuity across it is overruled.
    let replaced_transform = current.is_some_and(|frame| frame.transform().target() != placement);
    let continuity = if reshaped {
        Continuity::Discontinuous
    } else if replaced_transform && declared == Continuity::Continuous {
        Continuity::GeometryChanged
    } else {
        declared
    };

    let mut geometry = inner.geometry;
    let mut cursor = inner.cursor.clone();
    if continuity != Continuity::Continuous {
        geometry = geometry
            .next()
            .ok_or_else(|| Error::new(Status::LimitExceeded, "geometry revisions exhausted"))?;
    }
    let consumed_drops = if continuity == Continuity::Discontinuous && inner.latest.is_some() {
        cursor.begin_epoch()?;
        // FIRST is the only legal first sequence in a new epoch, so it cannot
        // also represent drops from the preceding epoch. Keep that debt until
        // a same-epoch publication can expose it as a checked sequence gap.
        0
    } else if inner.latest.is_some() {
        cursor.skip(pending_drops)?;
        pending_drops
    } else {
        0
    };

    let extent = descriptor.extent();
    let stamp = cursor.publish(geometry)?;
    let transform = match (placement, covers_target) {
        (Some(placement), _) => TransformSnapshot::with_target(geometry, extent, placement)
            .map_err(|_| CaptureFault::InconsistentDescriptor)?,
        (None, true) => TransformSnapshot::with_target_extent(geometry, extent),
        (None, false) => TransformSnapshot::frame_only(geometry, extent),
    };
    Frame::validate(stamp, descriptor, &transform)?;

    Ok(Prepared {
        cursor,
        geometry,
        stamp,
        transform,
        consumed_drops,
    })
}

/// Returns the published frame that satisfies `request`, if one already has.
fn qualifying(inner: &Inner, request: &FrameRequest) -> Result<Option<Frame>> {
    let Some(current) = inner.latest.as_ref() else {
        return Ok(None);
    };
    match request.selection() {
        FrameSelection::Latest => Ok(Some(current.clone())),
        FrameSelection::NewerThan(stamp) => {
            match current.stamp().order(&stamp) {
                Ok(FrameOrder::After) => Ok(Some(current.clone())),
                Ok(FrameOrder::Before | FrameOrder::Same) => Ok(None),
                // Comparing sequences across streams would produce a confident
                // wrong answer, so the request is refused instead.
                Err(_) => Err(CaptureFault::ForeignStream.into()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;

    use mado_pilot_core::{
        CancellationToken, Clock, CoordinateSpace, IdentityIssuer, MonotonicInstant,
        OperationContext, PixelExtent, Point, Scale, Status, TargetPlacement,
    };

    #[derive(Debug)]
    struct OriginClock;

    impl Clock for OriginClock {
        fn now(&self) -> MonotonicInstant {
            MonotonicInstant::ORIGIN
        }
    }

    #[derive(Debug, Default)]
    struct ExpireAtCommitClock {
        reads: AtomicUsize,
    }

    impl Clock for ExpireAtCommitClock {
        fn now(&self) -> MonotonicInstant {
            let elapsed = if self.reads.fetch_add(1, Ordering::Relaxed) == 0 {
                Duration::ZERO
            } else {
                Duration::from_millis(2)
            };
            MonotonicInstant::ORIGIN
                .checked_add(elapsed)
                .expect("test instant is representable")
        }
    }

    use super::{Continuity, FrameRequest, Lifecycle, Publication, StreamState};
    use crate::descriptor::{FrameDescriptor, PixelFormat};
    use crate::fault::CaptureFault;

    fn publication(width: u32, height: u32, fill: u8, continuity: Continuity) -> Publication {
        let extent = PixelExtent::new(width, height);
        let descriptor = FrameDescriptor::packed(extent, PixelFormat::Rgba8).expect("valid");
        Publication {
            captured_at: MonotonicInstant::ORIGIN,
            descriptor,
            placement: None,
            pixels: vec![fill; descriptor.byte_len()].into_boxed_slice(),
            continuity,
        }
    }

    fn state() -> StreamState {
        let issuer = IdentityIssuer::new();
        StreamState::new(issuer.issue_stream().expect("issued"))
    }

    fn covering_state() -> StreamState {
        let issuer = IdentityIssuer::new();
        StreamState::with_target_extent(issuer.issue_stream().expect("issued"))
    }

    fn placement(desktop_origin: (f64, f64), logical_size: (f64, f64)) -> TargetPlacement {
        TargetPlacement::new(
            desktop_origin,
            logical_size,
            Scale::new(1.0, 1.0).expect("valid"),
        )
        .expect("valid")
    }

    #[test]
    fn the_first_frame_is_epoch_zero_sequence_zero() {
        let state = state();

        let frame = state
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("published");

        assert_eq!(frame.stamp().epoch().value(), 0);
        assert_eq!(frame.stamp().sequence().value(), 0);
        assert_eq!(frame.stamp().geometry().value(), 0);
    }

    #[test]
    fn identical_pixels_still_advance_the_sequence() {
        let state = state();
        let first = state
            .publish(publication(4, 4, 7, Continuity::Continuous))
            .expect("published");
        let second = state
            .publish(publication(4, 4, 7, Continuity::Continuous))
            .expect("published");

        assert_eq!(first.stamp().epoch(), second.stamp().epoch());
        assert_eq!(second.stamp().sequence().value(), 1);
        assert_ne!(first.stamp(), second.stamp());
    }

    #[test]
    fn a_geometry_change_advances_the_revision_without_breaking_the_epoch() {
        let state = state();
        let first = state
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("published");
        let second = state
            .publish(publication(4, 4, 2, Continuity::GeometryChanged))
            .expect("published");

        assert_eq!(second.stamp().epoch(), first.stamp().epoch());
        assert_eq!(second.stamp().sequence().value(), 1);
        assert_eq!(second.stamp().geometry().value(), 1);
    }

    #[test]
    fn an_extent_change_starts_a_later_epoch_even_when_the_adapter_says_otherwise() {
        let state = state();
        state
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("published");

        // The adapter claims continuity, but the pixels describe a different
        // rectangle, so the stream overrides it.
        let reshaped = state
            .publish(publication(8, 8, 1, Continuity::Continuous))
            .expect("published");

        assert_eq!(reshaped.stamp().epoch().value(), 1);
        assert_eq!(reshaped.stamp().sequence().value(), 0);
        assert_eq!(reshaped.stamp().geometry().value(), 1);
    }

    #[test]
    fn a_rejected_publication_preserves_identity_and_geometry() {
        let state = state();
        let first = state
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("published");
        let descriptor =
            FrameDescriptor::packed(PixelExtent::new(8, 8), PixelFormat::Rgba8).expect("valid");

        let error = state
            .publish(Publication {
                captured_at: MonotonicInstant::ORIGIN,
                descriptor,
                placement: None,
                pixels: vec![2; descriptor.byte_len() - 1].into_boxed_slice(),
                continuity: Continuity::Discontinuous,
            })
            .expect_err("malformed pixels are rejected");
        let next = state
            .publish(publication(4, 4, 3, Continuity::Continuous))
            .expect("the next valid frame publishes");

        assert_eq!(error.status(), Status::InvalidArgument);
        assert_eq!(next.stamp().epoch(), first.stamp().epoch());
        assert_eq!(next.stamp().sequence().value(), 1);
        assert_eq!(next.stamp().geometry(), first.stamp().geometry());
    }

    #[test]
    fn a_closed_stream_returns_the_complete_publication() {
        let state = state();
        let current = state
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("published");
        let request = publication(4, 4, 173, Continuity::GeometryChanged);
        let pixels = request.pixels.as_ptr();
        let captured_at = request.captured_at;
        let descriptor = request.descriptor;
        let placement = request.placement;
        let continuity = request.continuity;
        state.begin_close();

        let refused = state
            .publish_recoverable(request)
            .expect_err("closing returns publication ownership");

        assert_eq!(refused.error().status(), Status::Closed);
        assert_eq!(refused.publication().pixels.as_ptr(), pixels);
        assert_eq!(refused.publication().captured_at, captured_at);
        assert_eq!(refused.publication().descriptor, descriptor);
        assert_eq!(refused.publication().placement, placement);
        assert_eq!(refused.publication().continuity, continuity);
        assert_eq!(
            state.current().expect("current frame remains").stamp(),
            current.stamp()
        );
        assert_eq!(state.lifecycle(), Lifecycle::Closing);
        let returned = refused.into_publication();
        assert_eq!(returned.pixels.as_ptr(), pixels);
        assert_eq!(returned.captured_at, captured_at);
        assert_eq!(returned.descriptor, descriptor);
        assert_eq!(returned.placement, placement);
        assert_eq!(returned.continuity, continuity);
    }

    #[test]
    fn malformed_pixels_are_returned_without_advancing_stream_state() {
        let state = state();
        let first = state
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("published");
        let descriptor =
            FrameDescriptor::packed(PixelExtent::new(8, 8), PixelFormat::Rgba8).expect("valid");
        let request = Publication {
            captured_at: MonotonicInstant::ORIGIN,
            descriptor,
            placement: None,
            pixels: vec![181; descriptor.byte_len() - 1].into_boxed_slice(),
            continuity: Continuity::Discontinuous,
        };
        let pixels = request.pixels.as_ptr();

        let refused = state
            .publish_recoverable(request)
            .expect_err("malformed pixels are returned");

        assert_eq!(refused.error().status(), Status::InvalidArgument);
        assert_eq!(refused.publication().pixels.as_ptr(), pixels);
        assert_eq!(
            refused.publication().pixels.len(),
            descriptor.byte_len() - 1
        );
        assert_eq!(
            state.current().expect("current frame remains").stamp(),
            first.stamp()
        );

        let next = state
            .publish(publication(4, 4, 3, Continuity::Continuous))
            .expect("the next valid frame publishes");
        assert_eq!(next.stamp().epoch(), first.stamp().epoch());
        assert_eq!(next.stamp().sequence().value(), 1);
        assert_eq!(next.stamp().geometry(), first.stamp().geometry());
    }

    #[test]
    fn inconsistent_geometry_is_returned_without_advancing_stream_state() {
        let state = state();
        let first = state
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("published");
        let mut request = publication(4, 4, 191, Continuity::GeometryChanged);
        request.placement = Some(placement((0.0, 0.0), (2.0, 4.0)));
        let pixels = request.pixels.as_ptr();

        let refused = state
            .publish_recoverable(request)
            .expect_err("inconsistent geometry is returned");

        assert_eq!(refused.error().status(), Status::InvalidArgument);
        assert_eq!(refused.publication().pixels.as_ptr(), pixels);
        assert_eq!(
            state.current().expect("current frame remains").stamp(),
            first.stamp()
        );

        let next = state
            .publish(publication(4, 4, 3, Continuity::Continuous))
            .expect("the next valid frame publishes");
        assert_eq!(next.stamp().sequence().value(), 1);
        assert_eq!(next.stamp().geometry(), first.stamp().geometry());
    }

    #[test]
    fn legacy_publication_keeps_success_and_failure_behavior() {
        let issuer = IdentityIssuer::new();
        let stream = issuer.issue_stream().expect("issued");
        let legacy = StreamState::new(stream);
        let recoverable = StreamState::new(stream);

        let legacy_frame = legacy
            .publish(publication(4, 4, 7, Continuity::Continuous))
            .expect("legacy publication succeeds");
        let recoverable_frame = recoverable
            .publish_recoverable(publication(4, 4, 7, Continuity::Continuous))
            .expect("recoverable publication succeeds");

        assert_eq!(legacy_frame.stamp(), recoverable_frame.stamp());
        assert_eq!(legacy_frame.descriptor(), recoverable_frame.descriptor());
        assert_eq!(legacy_frame.transform(), recoverable_frame.transform());

        let descriptor =
            FrameDescriptor::packed(PixelExtent::new(8, 8), PixelFormat::Rgba8).expect("valid");
        let malformed = || Publication {
            captured_at: MonotonicInstant::ORIGIN,
            descriptor,
            placement: None,
            pixels: vec![199; descriptor.byte_len() - 1].into_boxed_slice(),
            continuity: Continuity::Discontinuous,
        };
        let legacy_error = legacy
            .publish(malformed())
            .expect_err("legacy publication refuses malformed pixels");
        let recoverable_error = recoverable
            .publish_recoverable(malformed())
            .expect_err("recoverable publication refuses malformed pixels")
            .into_error();

        assert_eq!(legacy_error, recoverable_error);
        assert_eq!(
            legacy.current().expect("legacy current frame").stamp(),
            legacy_frame.stamp()
        );
        assert_eq!(
            recoverable
                .current()
                .expect("recoverable current frame")
                .stamp(),
            recoverable_frame.stamp()
        );
    }

    #[test]
    fn refused_publication_debug_output_never_contains_pixel_content() {
        let state = state();
        let descriptor =
            FrameDescriptor::packed(PixelExtent::new(2, 2), PixelFormat::Rgba8).expect("valid");
        let request = Publication {
            captured_at: MonotonicInstant::ORIGIN,
            descriptor,
            placement: None,
            pixels: vec![
                17, 23, 91, 201, 17, 23, 91, 201, 17, 23, 91, 201, 17, 23, 91,
            ]
            .into_boxed_slice(),
            continuity: Continuity::Continuous,
        };

        let refused = state
            .publish_recoverable(request)
            .expect_err("short pixels are refused");
        let text = format!("{refused:?}");

        assert!(text.contains("RefusedPublication"), "{text}");
        assert!(text.contains("InvalidArgument"), "{text}");
        assert!(text.contains("descriptor"), "{text}");
        assert!(text.contains("bytes: 15"), "{text}");
        assert!(
            !text.contains("[17, 23, 91, 201"),
            "captured pixels leaked into diagnostics: {text}"
        );
        let (error, publication) = refused.into_parts();
        assert_eq!(error.status(), Status::InvalidArgument);
        assert_eq!(publication.pixels.len(), 15);
    }

    #[test]
    fn a_format_change_also_starts_a_later_epoch() {
        let state = state();
        state
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("published");
        let extent = PixelExtent::new(4, 4);
        let descriptor = FrameDescriptor::packed(extent, PixelFormat::Bgra8).expect("valid");
        let reshaped = state
            .publish(Publication {
                captured_at: MonotonicInstant::ORIGIN,
                descriptor,
                placement: None,
                pixels: vec![1; descriptor.byte_len()].into_boxed_slice(),
                continuity: Continuity::Continuous,
            })
            .expect("published");

        assert_eq!(reshaped.stamp().epoch().value(), 1);
    }

    #[test]
    fn a_placement_makes_target_conversions_available_on_the_frame() {
        let state = state();
        let placement = placement((0.0, 0.0), (4.0, 4.0));
        let mut request = publication(4, 4, 1, Continuity::Continuous);
        request.placement = Some(placement);

        let frame = state.publish(request).expect("published");

        assert_eq!(frame.transform().target(), Some(placement));
        assert!(frame.transform().covers_target());
    }

    #[test]
    fn a_placement_change_advances_the_revision_even_when_the_adapter_says_otherwise() {
        let state = state();
        let mut first_request = publication(4, 4, 1, Continuity::Continuous);
        first_request.placement = Some(placement((0.0, 0.0), (4.0, 4.0)));
        let first = state.publish(first_request).expect("published");
        let mut moved_request = publication(4, 4, 1, Continuity::Continuous);
        moved_request.placement = Some(placement((500.0, 300.0), (4.0, 4.0)));

        // The pixels are comparable and the adapter says so, but the transform a
        // caller would correlate against is a different one.
        let moved = state.publish(moved_request).expect("published");

        assert_eq!(moved.stamp().epoch(), first.stamp().epoch());
        assert_eq!(moved.stamp().sequence().value(), 1);
        assert_eq!(moved.stamp().geometry().value(), 1);
        assert_ne!(moved.transform(), first.transform());
    }

    #[test]
    fn a_placement_appearing_mid_stream_advances_the_revision() {
        let state = state();
        let unplaced = state
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("published");
        let mut placed_request = publication(4, 4, 1, Continuity::Continuous);
        placed_request.placement = Some(placement((0.0, 0.0), (4.0, 4.0)));

        let placed = state.publish(placed_request).expect("published");

        assert!(
            !unplaced
                .transform()
                .supports(CoordinateSpace::TargetLogical)
        );
        assert!(placed.transform().supports(CoordinateSpace::TargetLogical));
        assert_eq!(
            placed.stamp().geometry().value(),
            1,
            "what a frame can convert may not change inside one revision"
        );
    }

    #[test]
    fn an_unchanged_placement_keeps_the_revision() {
        let state = state();
        let mut request = publication(4, 4, 1, Continuity::Continuous);
        request.placement = Some(placement((0.0, 0.0), (4.0, 4.0)));
        let first = state.publish(request).expect("published");
        let mut repeat = publication(4, 4, 2, Continuity::Continuous);
        repeat.placement = Some(placement((0.0, 0.0), (4.0, 4.0)));

        let second = state.publish(repeat).expect("published");

        assert_eq!(second.stamp().geometry(), first.stamp().geometry());
        assert_eq!(second.stamp().sequence().value(), 1);
    }

    #[test]
    fn a_placement_that_does_not_scale_to_the_published_extent_is_refused() {
        let state = state();
        let mut request = publication(4, 4, 1, Continuity::Continuous);
        // Half the frame, at a scale that does not make up the difference: a
        // manifest a replay source did not author can say this.
        request.placement = Some(placement((0.0, 0.0), (2.0, 4.0)));

        let error = state.publish(request).expect_err("refused");

        assert_eq!(error.status(), Status::InvalidArgument);
        assert!(
            state.current().is_none(),
            "a refused publication publishes nothing"
        );
    }

    #[test]
    fn target_normalized_tracks_the_frame_across_a_mid_stream_resize() {
        let state = covering_state();
        let middle = Point::new(CoordinateSpace::TargetNormalized, 0.5, 0.5).expect("valid");
        let same_middle = Point::new(CoordinateSpace::FrameNormalized, 0.5, 0.5).expect("valid");

        let first = state
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("published");
        let before = first
            .transform()
            .convert_point(middle, CoordinateSpace::CapturePixels)
            .expect("the frame covers its target");

        let resized = state
            .publish(publication(8, 8, 1, Continuity::Continuous))
            .expect("published");
        let after = resized
            .transform()
            .convert_point(middle, CoordinateSpace::CapturePixels)
            .expect("the frame still covers its target");

        assert_eq!((before.x(), before.y()), (2.0, 2.0));
        assert_eq!(
            (after.x(), after.y()),
            (4.0, 4.0),
            "the target's extent is the frame's, so a resize moves it"
        );
        for frame in [&first, &resized] {
            assert_eq!(
                frame
                    .transform()
                    .convert_point(middle, CoordinateSpace::CapturePixels)
                    .expect("supported"),
                frame
                    .transform()
                    .convert_point(same_middle, CoordinateSpace::CapturePixels)
                    .expect("supported"),
                "a frame covers exactly its target, so the two spaces coincide"
            );
        }
        assert_ne!(
            resized.stamp().geometry(),
            first.stamp().geometry(),
            "the resize is a transform change and advances the revision"
        );
    }

    #[test]
    fn latest_returns_the_current_frame_without_renaming_it() {
        let state = state();
        let published = state
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("published");
        let context = OperationContext::new();

        let first = state.frame(&FrameRequest::latest(), &context).expect("got");
        let second = state.frame(&FrameRequest::latest(), &context).expect("got");

        assert_eq!(first.stamp(), published.stamp());
        assert_eq!(second.stamp(), published.stamp());
    }

    #[test]
    fn a_newer_than_request_from_another_stream_is_refused() {
        let first = state();
        let second = state();
        let foreign = second
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("published");
        first
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("published");
        let context = OperationContext::new();

        let error = first
            .frame(&FrameRequest::newer_than(foreign.stamp()), &context)
            .expect_err("refused");

        assert_eq!(error.status(), Status::InvalidArgument);
    }

    #[test]
    fn a_stamp_from_an_earlier_epoch_is_older_than_the_current_frame() {
        let state = state();
        let early = state
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("published");
        let late = state
            .publish(publication(8, 8, 1, Continuity::Discontinuous))
            .expect("published");
        let context = OperationContext::new();

        let got = state
            .frame(&FrameRequest::newer_than(early.stamp()), &context)
            .expect("current epoch frame is newer");

        assert_eq!(got.stamp(), late.stamp());
    }

    #[test]
    fn a_wait_for_a_newer_frame_is_satisfied_by_publication() {
        let state = Arc::new(state());
        let current = state
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("published");
        let publisher = Arc::clone(&state);
        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            publisher
                .publish(publication(4, 4, 2, Continuity::Continuous))
                .expect("published")
        });
        let context = OperationContext::new();

        let got = state
            .frame(&FrameRequest::newer_than(current.stamp()), &context)
            .expect("woken by publication");
        let expected = handle.join().expect("publisher finished");

        assert_eq!(got.stamp(), expected.stamp());
    }

    #[test]
    fn a_wait_that_is_cancelled_does_not_later_become_a_success() {
        let state = Arc::new(state());
        let current = state
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("published");
        let token = CancellationToken::new();
        let context = OperationContext::new().with_cancellation(token.clone());
        let canceller = Arc::clone(&state);
        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            token.cancel();
            thread::sleep(Duration::from_millis(10));
            // A frame arriving after cancellation must not rescue the request.
            canceller
                .publish(publication(4, 4, 2, Continuity::Continuous))
                .expect("published");
        });

        let error = state
            .frame(&FrameRequest::newer_than(current.stamp()), &context)
            .expect_err("cancelled");
        handle.join().expect("canceller finished");

        assert_eq!(error.status(), Status::Cancelled);
    }

    #[test]
    fn a_wait_ends_when_the_deadline_passes() {
        let state = state();
        let current = state
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("published");
        let context = OperationContext::new()
            .with_timeout(Duration::from_millis(20))
            .expect("representable");

        let error = state
            .frame(&FrameRequest::newer_than(current.stamp()), &context)
            .expect_err("expired");

        assert_eq!(error.status(), Status::DeadlineExceeded);
    }

    #[test]
    fn closing_refuses_new_work_and_wakes_waiters() {
        let state = Arc::new(state());
        let current = state
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("published");
        let closer = Arc::clone(&state);
        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            closer.begin_close();
        });
        let context = OperationContext::new();

        let error = state
            .frame(&FrameRequest::newer_than(current.stamp()), &context)
            .expect_err("closed while waiting");
        handle.join().expect("closer finished");

        assert_eq!(error.status(), Status::Closed);
        assert_eq!(
            state
                .publish(publication(4, 4, 2, Continuity::Continuous))
                .err()
                .map(|error| error.status()),
            Some(Status::Closed)
        );
    }

    #[test]
    fn a_terminated_stream_reports_why_capture_ended() {
        let state = state();
        let published = state
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("published");
        let context = OperationContext::new();

        state.terminate(CaptureFault::TargetLost);

        assert_eq!(state.terminal(), Some(CaptureFault::TargetLost));
        assert_eq!(state.lifecycle(), Lifecycle::Closing);
        assert_eq!(
            state
                .frame(&FrameRequest::latest(), &context)
                .expect_err("capture ended")
                .status(),
            Status::TargetLost,
            "the reason outranks the closed outcome"
        );
        assert_eq!(
            state
                .publish(publication(4, 4, 2, Continuity::Continuous))
                .expect_err("nothing publishes after the end")
                .status(),
            Status::TargetLost
        );
        assert!(
            published.full_view().is_ok(),
            "a frame published before the end stays usable"
        );
    }

    #[test]
    fn the_first_terminal_fault_is_the_one_reported() {
        let state = state();

        state.terminate(CaptureFault::TargetLost);
        state.terminate(CaptureFault::SourceInvalid);

        assert_eq!(
            state.terminal(),
            Some(CaptureFault::TargetLost),
            "what went wrong first is the explanation"
        );
    }

    #[test]
    fn a_terminated_stream_still_closes_cleanly() {
        let state = state();
        state
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("published");
        state.terminate(CaptureFault::TargetLost);

        state
            .drain(&OperationContext::new())
            .expect("close finishes after a terminal fault");

        assert_eq!(state.lifecycle(), Lifecycle::Closed);
        assert_eq!(
            state.terminal(),
            Some(CaptureFault::TargetLost),
            "closing does not erase why capture ended"
        );
    }

    #[test]
    fn an_ordinary_close_records_no_fault() {
        let state = state();

        state.drain(&OperationContext::new()).expect("drained");

        assert_eq!(state.terminal(), None);
    }

    #[test]
    fn a_waiter_observes_the_terminal_fault_rather_than_a_closed_stream() {
        let state = Arc::new(state());
        let current = state
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("published");
        let ender = Arc::clone(&state);
        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            ender.terminate(CaptureFault::SourceInvalid);
        });

        let error = state
            .frame(
                &FrameRequest::newer_than(current.stamp()),
                &OperationContext::new(),
            )
            .expect_err("capture ended while waiting");
        handle.join().expect("ender finished");

        assert_eq!(error.status(), Status::CaptureFailed);
    }

    #[test]
    fn close_is_idempotent() {
        let state = state();
        state
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("published");
        let context = OperationContext::new();

        state.drain(&context).expect("drained");
        assert_eq!(state.lifecycle(), Lifecycle::Closed);

        state.drain(&context).expect("already drained");
        assert_eq!(state.lifecycle(), Lifecycle::Closed);
    }

    #[test]
    fn a_finite_path_drop_becomes_a_sequence_gap_without_replacing_the_frame() {
        let state = state();
        let first = state
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("published");

        let dropped = state.try_record_drop().expect("drop recorded");

        assert!(dropped, "a current frame makes the drop observable");
        assert_eq!(
            state.current().expect("current remains").stamp(),
            first.stamp(),
            "a drop never invents frame storage"
        );
        let later = state
            .publish(publication(4, 4, 2, Continuity::Continuous))
            .expect("later publication");
        assert_eq!(later.stamp().sequence().value(), 2);
    }

    #[test]
    fn finite_path_drop_debt_survives_discontinuities_until_a_gap_can_represent_it() {
        let state = state();
        state
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("first publication");
        assert!(state.try_record_drop().expect("first drop recorded"));

        let first_discontinuity = state
            .publish(publication(8, 8, 2, Continuity::Discontinuous))
            .expect("first discontinuity");
        assert_eq!(first_discontinuity.stamp().sequence().value(), 0);
        assert!(state.try_record_drop().expect("second drop recorded"));

        let second_discontinuity = state
            .publish(publication(16, 16, 3, Continuity::Discontinuous))
            .expect("second discontinuity");
        assert_eq!(second_discontinuity.stamp().sequence().value(), 0);

        let pressure_visible = state
            .publish(publication(16, 16, 4, Continuity::Continuous))
            .expect("continuous publication exposes the accumulated debt");
        assert_eq!(
            pressure_visible.stamp().sequence().value(),
            3,
            "two pending drops remain debt across both FIRST publications"
        );
    }

    #[test]
    fn unrepresentable_drop_debt_neither_wraps_nor_disappears_at_a_discontinuity() {
        let state = state();
        let first = state
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("first publication");
        state.pending_drops.store(u64::MAX, Ordering::Release);

        let error = state
            .publish(publication(4, 4, 2, Continuity::Continuous))
            .expect_err("checked skip refuses an unrepresentable gap");
        assert_eq!(error.status(), Status::LimitExceeded);
        assert_eq!(
            state.current().expect("current remains").stamp(),
            first.stamp()
        );
        assert_eq!(state.pending_drops.load(Ordering::Acquire), u64::MAX);

        let discontinuous = state
            .publish(publication(8, 8, 3, Continuity::Discontinuous))
            .expect("FIRST does not apply the unrepresentable debt");
        assert_eq!(discontinuous.stamp().sequence().value(), 0);
        assert_eq!(
            state.pending_drops.load(Ordering::Acquire),
            u64::MAX,
            "the discontinuity cannot consume debt it did not represent"
        );
    }

    #[test]
    fn a_finite_path_drop_is_recorded_while_the_stream_mutex_is_busy() {
        let state = state();
        state
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("published");
        let guard = state.lock();

        assert!(
            state.try_record_drop().expect("lock-free drop accounting"),
            "the existing frame makes the drop observable"
        );

        drop(guard);
        let later = state
            .publish(publication(4, 4, 2, Continuity::Continuous))
            .expect("later publication");
        assert_eq!(later.stamp().sequence().value(), 2);
    }

    #[test]
    fn a_frame_retained_across_close_stays_usable() {
        let state = state();
        let frame = state
            .publish(publication(4, 4, 9, Continuity::Continuous))
            .expect("published");
        let stamp = frame.stamp();
        let context = OperationContext::new();

        state.drain(&context).expect("drained");

        assert_eq!(frame.stamp(), stamp);
        assert!(frame.full_view().is_ok());
        assert!(
            frame
                .map(PixelFormat::Rgba8, &context)
                .expect("mapped after close")
                .bytes()
                .iter()
                .all(|byte| *byte == 9)
        );
    }

    #[test]
    fn expiry_at_final_close_commit_does_not_publish_the_closed_transition() {
        let state = state();
        state
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("published");
        let deadline = MonotonicInstant::ORIGIN
            .checked_add(Duration::from_millis(1))
            .expect("representable deadline");
        let expiring = OperationContext::new()
            .with_clock(Arc::new(ExpireAtCommitClock::default()))
            .with_deadline(deadline);

        let error = state
            .drain(&expiring)
            .expect_err("deadline wins final commit");

        assert_eq!(error.status(), Status::DeadlineExceeded);
        assert_eq!(state.lifecycle(), Lifecycle::Closing);
        state
            .drain(&OperationContext::new())
            .expect("a later close finishes");
        assert_eq!(state.lifecycle(), Lifecycle::Closed);
    }

    #[test]
    fn a_close_that_expires_leaves_the_stream_closing_for_a_later_attempt() {
        let state = state();
        state
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("published");
        let expired = OperationContext::new()
            .with_clock(Arc::new(OriginClock))
            .with_deadline(MonotonicInstant::ORIGIN);

        let error = state.drain(&expired).expect_err("deadline wins admission");

        assert_eq!(error.status(), Status::DeadlineExceeded);
        assert_eq!(state.lifecycle(), Lifecycle::Closing);
        assert_eq!(
            state
                .frame(&FrameRequest::latest(), &OperationContext::new())
                .expect_err("closing refuses new work")
                .status(),
            Status::Closed
        );

        state
            .drain(&OperationContext::new())
            .expect("a later close finishes");
        assert_eq!(state.lifecycle(), Lifecycle::Closed);
    }
}
