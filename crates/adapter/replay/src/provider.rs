//! The replay capture provider and the sessions it opens.
//!
//! # Lock discipline
//!
//! A session owns two locks: the mutex around its remaining frame sequence and
//! the one inside the capture package's [`StreamState`]. Neither is held while
//! the other is taken, and neither is held while the caller's operation context
//! is consulted, because that context's clock and cancellation token are the
//! caller's own code.
//!
//! Publication order between concurrent advances is bought by a reservation
//! rather than by nesting those locks. Exactly one advance at a time owns the
//! removed head of the sequence. It restores that exact frame on interruption
//! or stream refusal, so frames reach the stream in source order even though the
//! sequence mutex is released before every publication.

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::thread;
use std::time::Duration;

use mado_pilot_capture::{
    CaptureFault, CaptureProvider, CaptureSession, CoordinateSupport, Frame, FrameRequest,
    FrameSelection, Lifecycle, OpenRequest, SessionDescription, StreamState, TargetDescription,
};
use mado_pilot_core::{
    FrameOrder, GeometryRevision, IdentityIssuer, Operation, OperationContext, ProviderId, Result,
    TargetId, TransformSnapshot,
};

use crate::source::{ReplayFrame, ReplaySource, ReplayTarget};

/// Provider name that qualifies every replay target identity.
pub const PROVIDER: ProviderId = ProviderId::new("replay");

/// How long an advancing request waits before re-checking its operation context.
const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(2);

/// A capture provider backed by a configured replay source.
///
/// Everything it can produce is fixed when it is constructed. It performs no
/// desktop enumeration, no permission probe, no host DPI lookup, and no network
/// access, which is what makes the same source behave identically on Windows and
/// macOS.
pub struct ReplayProvider {
    issuer: Arc<IdentityIssuer>,
    targets: Vec<(TargetId, ReplayTarget)>,
}

impl ReplayProvider {
    /// Builds a provider that serves `source`, issuing identities from `issuer`.
    ///
    /// # Errors
    ///
    /// Returns a capture failure when a target's identity cannot be issued.
    pub fn new(issuer: Arc<IdentityIssuer>, source: ReplaySource) -> Result<Self> {
        let mut targets = Vec::new();
        for target in source.into_targets() {
            targets.push((issuer.issue_target(PROVIDER)?, target));
        }
        Ok(Self { issuer, targets })
    }

    fn description(&self, id: TargetId, target: &ReplayTarget) -> TargetDescription {
        TargetDescription::new(
            id,
            target.name(),
            target.extent(),
            target.format(),
            coordinate_support(target),
        )
    }
}

impl fmt::Debug for ReplayProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReplayProvider")
            .field("engine", &self.issuer.engine())
            .field("targets", &self.targets.len())
            .finish()
    }
}

fn coordinate_support(target: &ReplayTarget) -> CoordinateSupport {
    if target.declares_placement() {
        CoordinateSupport::with_target_placement()
    } else {
        CoordinateSupport::with_target_extent()
    }
}

impl CaptureProvider for ReplayProvider {
    fn provider(&self) -> ProviderId {
        PROVIDER
    }

    fn discover(&self, operation: &OperationContext) -> Result<Vec<TargetDescription>> {
        let attempt = Operation::admit(operation)?;
        let descriptions = self
            .targets
            .iter()
            .map(|(id, target)| self.description(*id, target))
            .collect();
        // A partial list must never look like a complete one, so the whole list
        // is committed or none of it is.
        Ok(attempt.commit(descriptions)?)
    }

    fn open(
        &self,
        target: TargetId,
        request: &OpenRequest,
        operation: &OperationContext,
    ) -> Result<Arc<dyn CaptureSession>> {
        let attempt = Operation::admit(operation)?;

        target.check_engine(self.issuer.engine())?;
        if target.provider() != PROVIDER {
            return Err(CaptureFault::ForeignTarget.into());
        }
        let (_, source) = self
            .targets
            .iter()
            .find(|(id, _)| *id == target)
            .ok_or(CaptureFault::UnknownTarget)?;

        // A required format the source cannot produce fails the open rather
        // than converting behind the caller's back: a caller who required a
        // format wanted the source to have it, not for someone to translate.
        let format = source.format();
        if let Some(required) = request.required_format()
            && required != format
        {
            return Err(CaptureFault::UnsupportedOption.into());
        }
        validate_placements(source)?;

        let stream = self.issuer.issue_stream()?;
        let description = SessionDescription::new(
            target,
            stream,
            source.extent(),
            format,
            coordinate_support(source),
        );
        let session = ReplaySession {
            description,
            state: StreamState::with_target_extent(stream),
            remaining: Mutex::new(Remainder::new(source.clone().into_frames())),
        };
        session.advance(operation)?;

        Ok(attempt.commit(Arc::new(session) as Arc<dyn CaptureSession>)?)
    }
}

/// Refuses a target any of whose frames contradicts its own declared placement.
///
/// The whole sequence is checked before a session exists, not one frame at a
/// time as each reaches the head. A manifest is configuration, and a manifest
/// whose third frame declares four logical units across eight pixels is a
/// configuration mistake at the moment it is read — reporting it three frames
/// later would mean the caller already believed the source was good, and the
/// session would then have nothing useful to do: publication is what removes a
/// frame from the sequence, so a frame publication will always refuse would sit
/// at the head and fail every later request identically.
///
/// The check is the constructor publication itself uses, so the two cannot
/// drift apart. Only coverage is being decided here and coverage does not
/// depend on the revision, so the first one stands in for whichever revision
/// the frame is eventually published under.
fn validate_placements(target: &ReplayTarget) -> Result<()> {
    for frame in target.frames() {
        if let Some(placement) = frame.placement() {
            TransformSnapshot::with_target(
                GeometryRevision::FIRST,
                frame.descriptor().extent(),
                placement,
            )
            .map_err(|_| CaptureFault::InconsistentDescriptor)?;
        }
    }
    Ok(())
}

/// One open replay session.
///
/// The session publishes its first frame at open, and publishes the next one
/// when a caller asks for something newer. That is what makes replay
/// deterministic: the sequence advances because a consumer consumed it, not
/// because wall-clock time passed.
struct ReplaySession {
    description: SessionDescription,
    state: StreamState,
    remaining: Mutex<Remainder>,
}

/// The part of a replay sequence that has not been published yet.
#[derive(Debug)]
struct Remainder {
    frames: VecDeque<ReplayFrame>,
    /// Set while one advance owns the head frame and has not finished with it.
    ///
    /// This is what keeps concurrent advances in source order. The head moves
    /// into its reservation while claimed; the flag stops a second request from
    /// claiming the frame behind it and publishing out of order.
    reserved: bool,
}

impl Remainder {
    fn new(frames: Vec<ReplayFrame>) -> Self {
        Self {
            frames: frames.into(),
            reserved: false,
        }
    }
}

/// One advance's exclusive claim on the next frame of a sequence.
///
/// A claim is reversible until it publishes. Dropping it without a successful
/// publication returns the exact frame to the source head and lets the next
/// advance proceed, which is what allows the operation's final arbitration to
/// happen after the frame is owned and before anything about it is observable.
#[derive(Debug)]
struct Reservation<'session> {
    session: &'session ReplaySession,
    /// The claimed frame, moved out of the source queue.
    frame: Option<ReplayFrame>,
}

impl Reservation<'_> {
    /// Publishes the claimed frame and reports the identity the stream gave it.
    ///
    /// # Errors
    ///
    /// Returns whatever the stream refused the publication with. The exact
    /// publication allocation is converted back into the claimed replay frame
    /// in that case, so dropping the reservation restores the source head.
    fn publish(mut self) -> Result<Frame> {
        let frame = self
            .frame
            .take()
            .expect("a reservation holds its frame until it publishes");
        match self
            .session
            .state
            .publish_recoverable(frame.into_publication())
        {
            Ok(published) => Ok(published),
            Err(refused) => {
                let (error, publication) = refused.into_parts();
                self.frame = Some(ReplayFrame::from_publication(publication));
                Err(error)
            }
        }
    }
}

impl Drop for Reservation<'_> {
    /// Releases the claim, restoring the frame unless publication consumed it.
    ///
    /// This runs inside the uninterruptible window after an advance commits, so
    /// its blocking `lock_remainder` is worth naming: the only other holders of
    /// that mutex are another `Drop` doing this same constant-time restoration
    /// and a [`ReplaySession::try_reserve`] that already declined to block.
    fn drop(&mut self) {
        let mut remainder = self.session.lock_remainder();
        if let Some(frame) = self.frame.take() {
            remainder.frames.push_front(frame);
        }
        remainder.reserved = false;
    }
}

impl ReplaySession {
    /// Publishes the next frame of the sequence.
    ///
    /// The frame is claimed first, arbitrated second, published third. Claiming
    /// is reversible and publishing is what makes the frame observable, so the
    /// operation's single terminal outcome is decided while nothing has changed
    /// yet: a cancellation or an expired deadline reaching the commit consumes
    /// no frame, publishes nothing, and advances no identity.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureFault::StreamEnded`] when the sequence is exhausted.
    /// Waiting instead would be honest only if a frame might still arrive, and
    /// for a finite replay sequence none ever will — so the caller is told,
    /// rather than left to discover it when the deadline expires.
    fn advance(&self, operation: &OperationContext) -> Result<Frame> {
        let mut attempt = Operation::admit(operation)?;
        let reserved = self.reserve(&mut attempt)?;
        // What is committed is the right to publish exactly this frame, which
        // is the operation's real subject and is already held by the time it is
        // arbitrated. An interruption drops the claim instead of returning it.
        // The clock is caller-supplied, so this happens with no lock held.
        //
        // What follows the commit is uninterruptible by contract, so what it
        // may contain is a claim in itself: one `StreamState::publish`, then
        // the claim release in `Reservation::drop`. That release takes the
        // sequence mutex and blocks, but only ever behind another advance's
        // own short critical section — never behind caller code, a backend, or
        // another operation's publication.
        let reserved = attempt.commit(reserved)?;
        reserved.publish()
    }

    /// Waits until this request owns the next frame of the sequence.
    fn reserve(&self, attempt: &mut Operation<'_>) -> Result<Reservation<'_>> {
        loop {
            if let Some(reservation) = self.try_reserve()? {
                return Ok(reservation);
            }
            attempt.checkpoint()?;
            thread::sleep(LOCK_POLL_INTERVAL);
        }
    }

    /// Claims the head of the sequence when no other advance holds it.
    ///
    /// Returns `Ok(None)` while another advance owns the head or is between
    /// operations on the sequence, which is the caller's cue to consult its
    /// operation context and try again.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureFault::StreamEnded`] for an exhausted sequence.
    fn try_reserve(&self) -> Result<Option<Reservation<'_>>> {
        let mut remainder = match self.remaining.try_lock() {
            Ok(remainder) => remainder,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return Ok(None),
        };
        if remainder.reserved {
            return Ok(None);
        }
        let frame = remainder
            .frames
            .pop_front()
            .ok_or(CaptureFault::StreamEnded)?;
        remainder.reserved = true;
        Ok(Some(Reservation {
            session: self,
            frame: Some(frame),
        }))
    }

    fn lock_remainder(&self) -> MutexGuard<'_, Remainder> {
        self.remaining
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl fmt::Debug for ReplaySession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReplaySession")
            .field("stream", &self.description.stream())
            .field("lifecycle", &self.state.lifecycle())
            .finish()
    }
}

impl CaptureSession for ReplaySession {
    fn description(&self) -> SessionDescription {
        self.description.clone()
    }

    fn frame(&self, request: &FrameRequest, operation: &OperationContext) -> Result<Frame> {
        let FrameSelection::NewerThan(stamp) = request.selection() else {
            return self.state.frame(request, operation);
        };
        if self.state.lifecycle() != Lifecycle::Open {
            return Err(CaptureFault::SessionClosed.into());
        }
        let current = self.state.current().ok_or(CaptureFault::SessionClosed)?;
        // A stamp from another stream is refused without touching the sequence:
        // advancing it would consume a frame to answer a request that was never
        // valid. Validate this before operation admission to preserve that typed
        // rejection for every foreign-stream request.
        let order = current
            .stamp()
            .order(&stamp)
            .map_err(|_| CaptureFault::ForeignStream)?;
        match order {
            // The common stream path owns lifecycle admission for an already
            // maintained frame. Going through it prevents this replay fast path
            // from returning cached data after closing begins.
            FrameOrder::After => self.state.frame(request, operation),
            FrameOrder::Before | FrameOrder::Same => self.advance(operation),
        }
    }

    fn close(&self, operation: &OperationContext) -> Result<()> {
        self.state.drain(operation)
    }

    fn lifecycle(&self) -> Lifecycle {
        self.state.lifecycle()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar, Mutex, Weak};

    use mado_pilot_capture::{Continuity, FrameDescriptor, PixelFormat};
    use mado_pilot_core::{
        Clock, GeometryRevision, MonotonicInstant, PixelExtent, Scale, Status, StreamCursor,
        TargetPlacement,
    };

    use super::*;

    #[derive(Debug, Default)]
    struct AdvanceAfterAdmission {
        reads: AtomicUsize,
    }

    impl Clock for AdvanceAfterAdmission {
        fn now(&self) -> MonotonicInstant {
            let reads = self.reads.fetch_add(1, Ordering::Relaxed);
            MonotonicInstant::ORIGIN
                .checked_add(if reads == 0 {
                    Duration::ZERO
                } else {
                    Duration::from_millis(2)
                })
                .expect("test instant is representable")
        }
    }

    #[derive(Debug)]
    struct ContentionClock {
        reads: AtomicUsize,
        contenders: Arc<(Mutex<usize>, Condvar)>,
    }

    impl ContentionClock {
        fn new(contenders: Arc<(Mutex<usize>, Condvar)>) -> Self {
            Self {
                reads: AtomicUsize::new(0),
                contenders,
            }
        }
    }

    impl Clock for ContentionClock {
        fn now(&self) -> MonotonicInstant {
            if self.reads.fetch_add(1, Ordering::Relaxed) == 1 {
                let (count, ready) = &*self.contenders;
                let mut count = count
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                *count += 1;
                ready.notify_all();
            }
            MonotonicInstant::ORIGIN
        }
    }

    /// How long a test barrier waits before it declares the interleaving lost.
    ///
    /// Reaching it is a failure, not a timing tolerance: every wait in these
    /// tests is released by another thread's progress, never by elapsed time.
    const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

    /// A one-shot barrier two test threads use to force one interleaving.
    #[derive(Debug, Default)]
    struct Handshake {
        raised: Mutex<bool>,
        reached: Condvar,
    }

    impl Handshake {
        fn raise(&self) {
            let mut raised = self
                .raised
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *raised = true;
            self.reached.notify_all();
        }

        /// Blocks until [`Handshake::raise`], reporting whether it arrived.
        fn wait(&self) -> bool {
            let raised = self
                .raised
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let (raised, timeout) = self
                .reached
                .wait_timeout_while(raised, HANDSHAKE_TIMEOUT, |raised| !*raised)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *raised && !timeout.timed_out()
        }
    }

    /// Expires a request's deadline exactly at its final arbitration.
    ///
    /// The first read is admission and passes. Every later read is the commit,
    /// which an advance reaches holding its claim on the next frame and having
    /// published nothing. That read announces the claim, waits for a second
    /// request to be spinning behind it, records what the sequence looked like
    /// while both were true, and only then reports the deadline as expired.
    #[derive(Debug)]
    struct InterruptAtCommit {
        reads: AtomicUsize,
        session: Weak<ReplaySession>,
        claimed: Arc<Handshake>,
        contended: Arc<Handshake>,
        /// How many frames remain queued behind the reserved head.
        queued: usize,
        interrupted_a_claim: AtomicBool,
    }

    impl InterruptAtCommit {
        fn new(
            session: &Arc<ReplaySession>,
            claimed: Arc<Handshake>,
            contended: Arc<Handshake>,
            queued: usize,
        ) -> Self {
            Self {
                reads: AtomicUsize::new(0),
                session: Arc::downgrade(session),
                claimed,
                contended,
                queued,
                interrupted_a_claim: AtomicBool::new(false),
            }
        }

        /// Reports whether the interruption landed on a claimed, unpublished
        /// frame while a second request waited behind it.
        fn interrupted_a_claim(&self) -> bool {
            self.interrupted_a_claim.load(Ordering::Relaxed)
        }
    }

    impl Clock for InterruptAtCommit {
        fn now(&self) -> MonotonicInstant {
            if self.reads.fetch_add(1, Ordering::Relaxed) == 0 {
                return MonotonicInstant::ORIGIN;
            }
            self.claimed.raise();
            let contended = self.contended.wait();
            if let Some(session) = self.session.upgrade() {
                let remainder = session.lock_remainder();
                self.interrupted_a_claim.store(
                    contended && remainder.reserved && remainder.frames.len() == self.queued,
                    Ordering::Relaxed,
                );
            }
            MonotonicInstant::ORIGIN
                .checked_add(Duration::from_millis(2))
                .expect("test instant is representable")
        }
    }

    /// Closes the stream exactly at a request's final arbitration.
    ///
    /// The first read is admission and passes. The second is the commit, which
    /// an advance reaches holding its claim on the next frame. Closing there is
    /// the one thing that refuses a publication the commit has already
    /// authorised, so the request runs the whole claim-arbitrate-publish
    /// sequence and is turned away by the stream at the last step — the path
    /// that decides whether a refused publication consumes the frame it failed
    /// to publish.
    #[derive(Debug)]
    struct CloseAtCommit {
        reads: AtomicUsize,
        session: Weak<ReplaySession>,
        /// How many frames remain queued behind the reserved head.
        queued: usize,
        refused_a_claim: AtomicBool,
    }

    impl CloseAtCommit {
        fn new(session: &Arc<ReplaySession>, queued: usize) -> Self {
            Self {
                reads: AtomicUsize::new(0),
                session: Arc::downgrade(session),
                queued,
                refused_a_claim: AtomicBool::new(false),
            }
        }

        /// Reports whether the refusal met a claimed, unpublished frame.
        fn refused_a_claim(&self) -> bool {
            self.refused_a_claim.load(Ordering::Relaxed)
        }
    }

    impl Clock for CloseAtCommit {
        fn now(&self) -> MonotonicInstant {
            if self.reads.fetch_add(1, Ordering::Relaxed) == 1
                && let Some(session) = self.session.upgrade()
            {
                {
                    let remainder = session.lock_remainder();
                    self.refused_a_claim.store(
                        remainder.reserved && remainder.frames.len() == self.queued,
                        Ordering::Relaxed,
                    );
                }
                session.state.begin_close();
            }
            // The deadline this clock is paired with is never reached: the
            // request must be refused by the stream, not interrupted before it
            // gets there.
            MonotonicInstant::ORIGIN
        }
    }

    /// How many refused claim attempts a waiting request tolerates.
    ///
    /// [`SignalWhenWaiting`] advances one [`WAIT_STEP`] per refusal, so this is
    /// a count of refusals rather than an elapsed time. Reaching it means the
    /// claim ahead was never released, which is a wedged session: it must fail
    /// the test with a deadline the same round it happens, not spin the run
    /// until something outside kills it.
    const REFUSAL_BUDGET: u32 = 500;

    /// How far [`SignalWhenWaiting`] advances for each refused claim.
    const WAIT_STEP: Duration = Duration::from_millis(1);

    /// Announces that a request has been refused the next frame at least once.
    ///
    /// The first read is admission, which happens before the request has looked
    /// at the sequence. Every later read is a checkpoint in the wait for a
    /// claim, so a second read is proof that this request found the frame
    /// already claimed and is waiting behind it.
    #[derive(Debug)]
    struct SignalWhenWaiting {
        reads: AtomicUsize,
        waiting: Arc<Handshake>,
    }

    impl SignalWhenWaiting {
        fn new(waiting: Arc<Handshake>) -> Self {
            Self {
                reads: AtomicUsize::new(0),
                waiting,
            }
        }
    }

    impl Clock for SignalWhenWaiting {
        fn now(&self) -> MonotonicInstant {
            let reads = self.reads.fetch_add(1, Ordering::Relaxed);
            if reads == 0 {
                return MonotonicInstant::ORIGIN;
            }
            self.waiting.raise();
            // Every refusal moves this request's clock forward. The wait is
            // released by the claim ahead being released, so the advance is
            // dead code in a healthy run; a claim that is never released hits
            // the deadline instead of spinning, which is the difference
            // between a failing test and a hanging one.
            MonotonicInstant::ORIGIN
                .checked_add(WAIT_STEP * u32::try_from(reads).unwrap_or(u32::MAX))
                .expect("test instant is representable")
        }
    }

    #[derive(Debug)]
    struct QueueObservingClock {
        session: Weak<ReplaySession>,
        observed_locked: AtomicBool,
    }

    impl QueueObservingClock {
        fn new(session: &Arc<ReplaySession>) -> Self {
            Self {
                session: Arc::downgrade(session),
                observed_locked: AtomicBool::new(false),
            }
        }

        fn observed_locked(&self) -> bool {
            self.observed_locked.load(Ordering::Relaxed)
        }
    }

    impl Clock for QueueObservingClock {
        fn now(&self) -> MonotonicInstant {
            if self
                .session
                .upgrade()
                .is_some_and(|session| session.remaining.try_lock().is_err())
            {
                self.observed_locked.store(true, Ordering::Relaxed);
            }
            MonotonicInstant::ORIGIN
        }
    }

    fn session() -> Arc<ReplaySession> {
        let issuer = IdentityIssuer::new();
        let target = issuer.issue_target(PROVIDER).expect("issued target");
        let stream = issuer.issue_stream().expect("issued stream");
        let extent = PixelExtent::new(8, 6);
        let descriptor = FrameDescriptor::packed(extent, PixelFormat::Rgba8).expect("valid");
        let frames = (0..3)
            .map(|fill| {
                ReplayFrame::new(
                    descriptor,
                    MonotonicInstant::ORIGIN,
                    Continuity::Continuous,
                    None,
                    vec![fill; descriptor.byte_len()].into_boxed_slice(),
                )
                .expect("valid replay frame")
            })
            .collect();
        let session = Arc::new(ReplaySession {
            description: SessionDescription::new(
                target,
                stream,
                extent,
                PixelFormat::Rgba8,
                CoordinateSupport::with_target_extent(),
            ),
            state: StreamState::with_target_extent(stream),
            remaining: Mutex::new(Remainder::new(frames)),
        });
        session
            .advance(&OperationContext::new())
            .expect("first frame publishes");
        session
    }

    /// Builds a one-frame source whose declared placement is `placement`.
    fn placed_source(extent: PixelExtent, placement: TargetPlacement) -> ReplaySource {
        placed_sequence(vec![(extent, placement)])
    }

    /// Builds a source whose frames carry the extent and placement they pair.
    fn placed_sequence(frames: Vec<(PixelExtent, TargetPlacement)>) -> ReplaySource {
        let frames = frames
            .into_iter()
            .map(|(extent, placement)| {
                let descriptor =
                    FrameDescriptor::packed(extent, PixelFormat::Rgba8).expect("valid");
                ReplayFrame::new(
                    descriptor,
                    MonotonicInstant::ORIGIN,
                    Continuity::Continuous,
                    Some(placement),
                    vec![0; descriptor.byte_len()].into_boxed_slice(),
                )
                .expect("valid replay frame")
            })
            .collect();
        ReplaySource::from_targets(vec![
            ReplayTarget::new("placed", frames).expect("valid target"),
        ])
        .expect("valid source")
    }

    fn open_first_target(source: ReplaySource) -> Result<Arc<dyn CaptureSession>> {
        let issuer = Arc::new(IdentityIssuer::new());
        let provider = ReplayProvider::new(issuer, source).expect("provider");
        let operation = OperationContext::new();
        let targets = provider.discover(&operation).expect("discovered");
        provider.open(targets[0].id(), &OpenRequest::new(), &operation)
    }

    #[test]
    fn a_source_whose_placement_does_not_cover_its_frame_fails_to_open() {
        let extent = PixelExtent::new(8, 6);
        // A source states its own placement, and a manifest is not something the
        // adapter authored: 8x6 pixels cannot be 4x3 logical units at scale 1.
        let inconsistent =
            TargetPlacement::new((0.0, 0.0), (4.0, 3.0), Scale::new(1.0, 1.0).expect("valid"))
                .expect("valid");

        let error = open_first_target(placed_source(extent, inconsistent))
            .expect_err("an inconsistent placement is refused");

        assert_eq!(
            error.status(),
            Status::InvalidArgument,
            "a displaced transform must not be published instead"
        );
    }

    #[test]
    fn a_source_whose_later_placement_does_not_cover_its_frame_fails_to_open() {
        let covers_eight_by_six =
            TargetPlacement::new((0.0, 0.0), (8.0, 6.0), Scale::new(1.0, 1.0).expect("valid"))
                .expect("valid");

        // The second frame is twice as wide as the placement it declares, and a
        // session would not reach it until the first frame had already
        // published. A manifest is checked as a whole, so the caller learns
        // this before it holds a session that cannot get past frame two.
        let error = open_first_target(placed_sequence(vec![
            (PixelExtent::new(8, 6), covers_eight_by_six),
            (PixelExtent::new(16, 6), covers_eight_by_six),
        ]))
        .expect_err("an inconsistent placement anywhere in the sequence is refused");

        assert_eq!(error.status(), Status::InvalidArgument);
    }

    #[test]
    fn a_source_whose_placement_covers_its_frame_opens() {
        let extent = PixelExtent::new(8, 6);
        let consistent = TargetPlacement::new(
            (100.0, 50.0),
            (4.0, 3.0),
            Scale::new(2.0, 2.0).expect("valid"),
        )
        .expect("valid");

        let session = open_first_target(placed_source(extent, consistent)).expect("opened");
        let frame = session
            .frame(&FrameRequest::latest(), &OperationContext::new())
            .expect("first frame");

        assert_eq!(frame.transform().target(), Some(consistent));
    }

    #[test]
    fn a_successful_advance_transfers_the_replay_pixel_allocation() {
        let session = session();
        let first = session.state.current().expect("first frame");
        let (source_pixels, descriptor, captured_at, placement, continuity) = {
            let remainder = session.lock_remainder();
            let next = remainder.frames.front().expect("next frame");
            (
                next.pixels().as_ptr() as usize,
                next.descriptor(),
                next.captured_at(),
                next.placement(),
                next.continuity(),
            )
        };

        let published = session
            .frame(
                &FrameRequest::newer_than(first.stamp()),
                &OperationContext::new(),
            )
            .expect("next frame publishes");
        let mapping = published
            .map(PixelFormat::Rgba8, &OperationContext::new())
            .expect("matching format maps");

        assert!(
            mapping.is_shared(),
            "the full matching mapping shares storage"
        );
        assert_eq!(mapping.bytes().as_ptr() as usize, source_pixels);
        assert_eq!(published.descriptor(), descriptor);
        assert_eq!(published.captured_at(), captured_at);
        assert_eq!(published.transform().target(), placement);
        assert_eq!(continuity, Continuity::Continuous);
        let remainder = session.lock_remainder();
        assert_eq!(remainder.frames.len(), 1);
        assert!(!remainder.reserved);
    }

    #[test]
    fn a_deadline_while_waiting_for_the_replay_queue_consumes_nothing() {
        let session = session();
        let first = session.state.current().expect("first frame");
        let remaining = session.lock_remainder();
        let deadline = MonotonicInstant::ORIGIN
            .checked_add(Duration::from_millis(1))
            .expect("representable deadline");
        let operation = OperationContext::new()
            .with_clock(Arc::new(AdvanceAfterAdmission::default()))
            .with_deadline(deadline);

        let error = session
            .frame(&FrameRequest::newer_than(first.stamp()), &operation)
            .expect_err("deadline wins while the queue is contended");

        assert_eq!(error.status(), Status::DeadlineExceeded);
        assert_eq!(
            remaining.frames.len(),
            2,
            "the next replay frame remains queued"
        );
        drop(remaining);

        let next = session
            .frame(
                &FrameRequest::newer_than(first.stamp()),
                &OperationContext::new(),
            )
            .expect("the unconsumed frame still publishes");
        assert_eq!(next.stamp().sequence().value(), 1);
    }

    #[test]
    fn an_interruption_after_the_next_frame_is_claimed_consumes_nothing() {
        let session = session();
        let first = session.state.current().expect("first frame");
        let first_stamp = first.stamp();
        let (claimed_pixels, descriptor, captured_at, placement, continuity) = {
            let remainder = session.lock_remainder();
            let next = remainder.frames.front().expect("next frame");
            (
                next.pixels().as_ptr() as usize,
                next.descriptor(),
                next.captured_at(),
                next.placement(),
                next.continuity(),
            )
        };
        let claimed = Arc::new(Handshake::default());
        let contended = Arc::new(Handshake::default());
        // Two frames are left after the session published its first. The claim
        // owns the head while the second frame remains in the queue.
        let clock = Arc::new(InterruptAtCommit::new(
            &session,
            Arc::clone(&claimed),
            Arc::clone(&contended),
            1,
        ));
        let expires = MonotonicInstant::ORIGIN
            .checked_add(Duration::from_millis(1))
            .expect("representable deadline");
        // The follower's deadline is a refusal budget, not a timeout: it can
        // only be reached by being refused the claim REFUSAL_BUDGET times,
        // which happens if and only if the claim ahead is never released.
        let budget_exhausted = MonotonicInstant::ORIGIN
            .checked_add(WAIT_STEP * REFUSAL_BUDGET)
            .expect("representable deadline");

        let interrupted = {
            let session = Arc::clone(&session);
            let clock = Arc::clone(&clock);
            thread::spawn(move || {
                let operation = OperationContext::new()
                    .with_clock(clock)
                    .with_deadline(expires);
                session.frame(&FrameRequest::newer_than(first_stamp), &operation)
            })
        };

        // The claiming request is now parked inside its own final arbitration,
        // holding the next frame and having published nothing. Only then does a
        // second request start, so it is guaranteed to meet the claim.
        assert!(
            claimed.wait(),
            "the first request reached its final arbitration"
        );
        let follower = {
            let session = Arc::clone(&session);
            let contended = Arc::clone(&contended);
            thread::spawn(move || {
                let operation = OperationContext::new()
                    .with_clock(Arc::new(SignalWhenWaiting::new(contended)))
                    .with_deadline(budget_exhausted);
                session.frame(&FrameRequest::newer_than(first_stamp), &operation)
            })
        };

        let error = interrupted
            .join()
            .expect("the claiming thread finished")
            .expect_err("the deadline wins after the frame is claimed");
        let published = follower
            .join()
            .expect("the following thread finished")
            .expect(
                "the released frame publishes for the request behind it; an exceeded deadline \
                 here means the interrupted claim was never released",
            );

        assert_eq!(error.status(), Status::DeadlineExceeded);
        assert!(
            clock.interrupted_a_claim(),
            "the interruption must land between the claim and the publication"
        );
        assert_eq!(
            published.stamp().sequence().value(),
            1,
            "the interrupted request advanced no identity"
        );
        let mapping = published
            .map(PixelFormat::Rgba8, &OperationContext::new())
            .expect("frame maps");
        assert!(
            mapping.bytes().iter().all(|byte| *byte == 1),
            "the frame the interrupted request claimed was left for the next one"
        );
        assert_eq!(
            mapping.bytes().as_ptr() as usize,
            claimed_pixels,
            "rollback returns the exact allocation to the follower"
        );
        assert_eq!(published.descriptor(), descriptor);
        assert_eq!(published.captured_at(), captured_at);
        assert_eq!(published.transform().target(), placement);
        assert_eq!(continuity, Continuity::Continuous);
        let remainder = session.lock_remainder();
        assert_eq!(
            remainder.frames.len(),
            1,
            "exactly one frame was consumed, by the request that published it"
        );
        assert!(
            !remainder.reserved,
            "an interrupted claim is released, not leaked"
        );
        let head = remainder.frames.front().expect("last frame remains");
        assert!(head.pixels().iter().all(|byte| *byte == 2));
    }

    #[test]
    fn operation_clock_is_never_called_while_the_replay_queue_is_locked() {
        let session = session();
        let first = session.state.current().expect("first frame");
        let clock = Arc::new(QueueObservingClock::new(&session));
        let deadline = MonotonicInstant::ORIGIN
            .checked_add(Duration::from_secs(1))
            .expect("representable deadline");
        let operation = OperationContext::new()
            .with_clock(clock.clone())
            .with_deadline(deadline);

        session
            .frame(&FrameRequest::newer_than(first.stamp()), &operation)
            .expect("next frame publishes");

        assert!(
            !clock.observed_locked(),
            "caller clock callbacks must run outside the replay queue mutex"
        );
    }

    #[test]
    fn a_newer_than_request_after_closing_never_reaches_the_replay_sequence() {
        let session = session();
        let first = session.state.current().expect("first frame");
        session.state.begin_close();

        let error = session
            .frame(
                &FrameRequest::newer_than(first.stamp()),
                &OperationContext::new(),
            )
            .expect_err("closing rejects publication");

        assert_eq!(error.status(), Status::Closed);
        let remainder = session.lock_remainder();
        assert_eq!(
            remainder.frames.len(),
            2,
            "a request refused before the sequence is consulted consumes nothing"
        );
        assert!(
            !remainder.reserved,
            "a request refused before the sequence is consulted claims nothing"
        );
    }

    #[test]
    fn a_publication_refused_after_the_claim_leaves_the_frame_queued() {
        let session = session();
        let first = session.state.current().expect("first frame");
        let (claimed_pixels, descriptor, captured_at, placement, continuity) = {
            let remainder = session.lock_remainder();
            let next = remainder.frames.front().expect("next frame");
            (
                next.pixels().as_ptr() as usize,
                next.descriptor(),
                next.captured_at(),
                next.placement(),
                next.continuity(),
            )
        };
        // The claim owns the head while one later frame remains queued. A
        // refusal must restore that exact head before releasing the claim.
        let clock = Arc::new(CloseAtCommit::new(&session, 1));
        let deadline = MonotonicInstant::ORIGIN
            .checked_add(Duration::from_secs(1))
            .expect("representable deadline");
        let operation = OperationContext::new()
            .with_clock(clock.clone())
            .with_deadline(deadline);

        // Closing begins inside this request's final arbitration, so unlike a
        // session that was already closing it gets all the way to a claimed
        // frame and a committed right to publish it before the stream refuses.
        let error = session
            .frame(&FrameRequest::newer_than(first.stamp()), &operation)
            .expect_err("a stream that began closing refuses the publication");

        assert_eq!(error.status(), Status::Closed);
        assert!(
            clock.refused_a_claim(),
            "the refusal must land on a claimed, unpublished frame"
        );
        let remainder = session.lock_remainder();
        assert_eq!(
            remainder.frames.len(),
            2,
            "a frame that failed to publish is not consumed"
        );
        assert!(
            !remainder.reserved,
            "a refused publication releases the claim it was holding"
        );
        let restored = remainder.frames.front().expect("refused head restored");
        assert_eq!(restored.pixels().as_ptr() as usize, claimed_pixels);
        assert!(restored.pixels().iter().all(|byte| *byte == 1));
        assert_eq!(restored.descriptor(), descriptor);
        assert_eq!(restored.captured_at(), captured_at);
        assert_eq!(restored.placement(), placement);
        assert_eq!(restored.continuity(), continuity);
    }

    #[test]
    fn a_foreign_request_after_closing_reports_closed_without_consuming_replay() {
        let session = session();
        session.state.begin_close();
        let issuer = IdentityIssuer::new();
        let _first = issuer.issue_stream().expect("first stream");
        let foreign_stream = issuer.issue_stream().expect("foreign stream");
        let foreign = StreamCursor::new(foreign_stream)
            .publish(GeometryRevision::FIRST)
            .expect("foreign stamp");

        let error = session
            .frame(&FrameRequest::newer_than(foreign), &OperationContext::new())
            .expect_err("closing takes precedence over request qualification");

        assert_eq!(error.status(), Status::Closed);
        assert_eq!(session.lock_remainder().frames.len(), 2);
    }

    #[test]
    fn concurrent_advances_publish_in_replay_source_order() {
        let session = session();
        let first = session.state.current().expect("first frame");
        let first_stamp = first.stamp();
        let remaining = session.lock_remainder();
        let contenders = Arc::new((Mutex::new(0usize), Condvar::new()));
        let deadline = MonotonicInstant::ORIGIN
            .checked_add(Duration::from_secs(1))
            .expect("representable deadline");
        let mut handles = Vec::new();

        for _ in 0..2 {
            let session = Arc::clone(&session);
            let contenders = Arc::clone(&contenders);
            handles.push(thread::spawn(move || {
                let operation = OperationContext::new()
                    .with_clock(Arc::new(ContentionClock::new(contenders)))
                    .with_deadline(deadline);
                session.frame(&FrameRequest::newer_than(first_stamp), &operation)
            }));
        }

        let (count, ready) = &*contenders;
        let count = count
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (count, wait) = ready
            .wait_timeout_while(count, Duration::from_secs(1), |count| *count < 2)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(!wait.timed_out(), "both requests reached replay contention");
        assert_eq!(*count, 2);
        drop(count);
        drop(remaining);

        let mut frames: Vec<Frame> = handles
            .into_iter()
            .map(|handle| handle.join().expect("request thread finished"))
            .collect::<Result<_>>()
            .expect("both frames publish");
        frames.sort_by_key(|frame| frame.stamp().sequence().value());

        assert_eq!(frames[0].stamp().sequence().value(), 1);
        assert_eq!(frames[1].stamp().sequence().value(), 2);
        for (frame, fill) in frames.iter().zip([1u8, 2]) {
            assert!(
                frame
                    .map(PixelFormat::Rgba8, &OperationContext::new())
                    .expect("frame maps")
                    .bytes()
                    .iter()
                    .all(|byte| *byte == fill),
                "published identity must preserve replay source order"
            );
        }
    }
}
