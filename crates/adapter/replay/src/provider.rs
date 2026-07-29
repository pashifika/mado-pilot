//! The replay capture provider and the sessions it opens.

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex, TryLockError};
use std::thread;
use std::time::Duration;

use mado_pilot_capture::{
    CaptureFault, CaptureProvider, CaptureSession, CoordinateSupport, Frame, FrameRequest,
    FrameSelection, Lifecycle, OpenRequest, Publication, SessionDescription, StreamState,
    TargetDescription,
};
use mado_pilot_core::{
    FrameOrder, IdentityIssuer, Operation, OperationContext, ProviderId, Result, TargetId,
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
            remaining: Mutex::new(source.clone().into_frames().into()),
        };
        session.advance(operation)?;

        Ok(attempt.commit(Arc::new(session) as Arc<dyn CaptureSession>)?)
    }
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
    remaining: Mutex<VecDeque<ReplayFrame>>,
}

impl ReplaySession {
    /// Publishes the next frame of the sequence.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureFault::StreamEnded`] when the sequence is exhausted.
    /// Waiting instead would be honest only if a frame might still arrive, and
    /// for a finite replay sequence none ever will — so the caller is told,
    /// rather than left to discover it when the deadline expires.
    fn advance(&self, operation: &OperationContext) -> Result<Frame> {
        let mut attempt = Operation::admit(operation)?;
        self.wait_until_queue_available(&mut attempt)?;
        // The clock is caller-supplied, so final arbitration must happen without
        // an internal mutex held. Once committed, the short queue/publication
        // transaction below contains no caller callback or blocking backend work.
        attempt.commit(())?;

        let mut remaining = self
            .remaining
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let frame = remaining
            .front()
            .cloned()
            .ok_or(CaptureFault::StreamEnded)?;
        let publication = Publication {
            captured_at: frame.captured_at(),
            descriptor: frame.descriptor(),
            placement: frame.placement(),
            continuity: frame.continuity(),
            pixels: frame.into_pixels(),
        };
        let published = self.state.publish(publication)?;
        remaining.pop_front();
        Ok(published)
    }

    fn wait_until_queue_available(&self, attempt: &mut Operation<'_>) -> Result<()> {
        loop {
            match self.remaining.try_lock() {
                Ok(remaining) => {
                    if remaining.is_empty() {
                        return Err(CaptureFault::StreamEnded.into());
                    }
                    return Ok(());
                }
                Err(TryLockError::Poisoned(poisoned)) => {
                    if poisoned.into_inner().is_empty() {
                        return Err(CaptureFault::StreamEnded.into());
                    }
                    return Ok(());
                }
                Err(TryLockError::WouldBlock) => {
                    attempt.checkpoint()?;
                    thread::sleep(LOCK_POLL_INTERVAL);
                }
            }
        }
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
            remaining: Mutex::new(frames),
        });
        session
            .advance(&OperationContext::new())
            .expect("first frame publishes");
        session
    }

    /// Builds a one-frame source whose declared placement is `placement`.
    fn placed_source(extent: PixelExtent, placement: TargetPlacement) -> ReplaySource {
        let descriptor = FrameDescriptor::packed(extent, PixelFormat::Rgba8).expect("valid");
        let frame = ReplayFrame::new(
            descriptor,
            MonotonicInstant::ORIGIN,
            Continuity::Continuous,
            Some(placement),
            vec![0; descriptor.byte_len()].into_boxed_slice(),
        )
        .expect("valid replay frame");
        ReplaySource::from_targets(vec![
            ReplayTarget::new("placed", vec![frame]).expect("valid target"),
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
    fn a_deadline_while_waiting_for_the_replay_queue_consumes_nothing() {
        let session = session();
        let first = session.state.current().expect("first frame");
        let remaining = session
            .remaining
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
        assert_eq!(remaining.len(), 2, "the next replay frame remains queued");
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
    fn close_rejection_does_not_remove_the_next_replay_frame() {
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
        assert_eq!(
            session
                .remaining
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
            2,
            "a rejected publication leaves the finite source intact"
        );
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
        assert_eq!(
            session
                .remaining
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
            2
        );
    }

    #[test]
    fn concurrent_advances_publish_in_replay_source_order() {
        let session = session();
        let first = session.state.current().expect("first frame");
        let first_stamp = first.stamp();
        let remaining = session
            .remaining
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
