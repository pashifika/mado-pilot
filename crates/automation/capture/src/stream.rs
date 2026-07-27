//! Authoritative stream publication state.
//!
//! This is where a capture adapter's frames become published frames. Every
//! adapter drives the same state machine, so epochs, sequences, geometry
//! revisions, latest-frame semantics, and close behavior cannot differ between
//! a replay source and a native one.

use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

use mado_pilot_core::{
    Error, FrameOrder, FrameStamp, GeometryRevision, MonotonicInstant, Operation, OperationContext,
    Result, Status, StreamCursor, StreamId, TargetPlacement, TransformSnapshot,
};

use crate::descriptor::FrameDescriptor;
use crate::fault::CaptureFault;
use crate::frame::Frame;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lifecycle {
    /// Publishing and accepting frame requests.
    Open,
    /// Close has begun. New work is refused; in-flight waits are unwinding.
    Closing,
    /// Closed and drained.
    Closed,
}

/// The published state of one capture stream.
#[derive(Debug)]
pub struct StreamState {
    inner: Mutex<Inner>,
    published: Condvar,
}

#[derive(Debug)]
struct Inner {
    cursor: StreamCursor,
    geometry: GeometryRevision,
    latest: Option<Frame>,
    lifecycle: Lifecycle,
    waiters: usize,
}

impl StreamState {
    /// Starts a stream at epoch zero with nothing published.
    #[must_use]
    pub fn new(stream: StreamId) -> Self {
        Self {
            inner: Mutex::new(Inner {
                cursor: StreamCursor::new(stream),
                geometry: GeometryRevision::FIRST,
                latest: None,
                lifecycle: Lifecycle::Open,
                waiters: 0,
            }),
            published: Condvar::new(),
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
    /// pixels that describe different rectangles.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureFault::SessionClosed`] once close has begun, and an
    /// identity fault when an epoch or sequence counter is exhausted.
    pub fn publish(&self, publication: Publication) -> Result<Frame> {
        let mut inner = self.lock();
        if inner.lifecycle != Lifecycle::Open {
            return Err(CaptureFault::SessionClosed.into());
        }

        let reshaped = inner.latest.as_ref().is_some_and(|current| {
            let existing = current.descriptor();
            existing.extent() != publication.descriptor.extent()
                || existing.format() != publication.descriptor.format()
        });
        let continuity = if reshaped {
            Continuity::Discontinuous
        } else {
            publication.continuity
        };

        if continuity != Continuity::Continuous {
            inner.geometry = inner
                .geometry
                .next()
                .ok_or_else(|| Error::new(Status::LimitExceeded, "geometry revisions exhausted"))?;
        }
        if continuity == Continuity::Discontinuous && inner.latest.is_some() {
            inner.cursor.begin_epoch()?;
        }

        let geometry = inner.geometry;
        let extent = publication.descriptor.extent();
        let stamp = inner.cursor.publish(geometry)?;
        let transform = match publication.placement {
            Some(placement) => TransformSnapshot::with_target(geometry, extent, placement),
            None => TransformSnapshot::frame_only(geometry, extent),
        };
        let frame = Frame::new(
            stamp,
            publication.captured_at,
            publication.descriptor,
            transform,
            publication.pixels,
        )?;

        inner.latest = Some(frame.clone());
        drop(inner);
        self.published.notify_all();
        Ok(frame)
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
                if let Some(frame) = qualifying(&inner, request)? {
                    drop(inner);
                    return Ok(attempt.commit(frame)?);
                }
                if inner.lifecycle != Lifecycle::Open {
                    return Err(CaptureFault::SessionClosed.into());
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

    /// Marks the stream as closing, refusing new work and waking every waiter.
    ///
    /// Idempotent, and never moves a closed stream backwards.
    pub fn begin_close(&self) {
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
                    let mut inner = inner;
                    inner.lifecycle = Lifecycle::Closed;
                    inner.latest = None;
                    drop(inner);
                    return Ok(attempt.commit(())?);
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
    use std::thread;
    use std::time::Duration;

    use mado_pilot_core::{
        CancellationToken, IdentityIssuer, MonotonicInstant, OperationContext, PixelExtent, Scale,
        Status, TargetPlacement,
    };

    use super::{Continuity, FrameRequest, Lifecycle, Publication, StreamState};
    use crate::descriptor::{FrameDescriptor, PixelFormat};

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
        let placement =
            TargetPlacement::new((0.0, 0.0), (4.0, 4.0), Scale::new(1.0, 1.0).expect("valid"))
                .expect("valid");
        let mut request = publication(4, 4, 1, Continuity::Continuous);
        request.placement = Some(placement);

        let frame = state.publish(request).expect("published");

        assert_eq!(frame.transform().target(), Some(placement));
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
    fn a_close_that_expires_leaves_the_stream_closing_for_a_later_attempt() {
        let state = Arc::new(state());
        let current = state
            .publish(publication(4, 4, 1, Continuity::Continuous))
            .expect("published");

        // A waiter that never gives up keeps the drain from completing.
        let waiter_state = Arc::clone(&state);
        let waiter = thread::spawn(move || {
            let context = OperationContext::new()
                .with_timeout(Duration::from_millis(200))
                .expect("representable");
            waiter_state
                .frame(&FrameRequest::newer_than(current.stamp()), &context)
                .expect_err("the stream closes under it")
        });
        thread::sleep(Duration::from_millis(5));

        let closing = OperationContext::new()
            .with_timeout(Duration::from_millis(1))
            .expect("representable");
        let first = state.drain(&closing);

        waiter.join().expect("waiter finished");
        let second = OperationContext::new();
        state.drain(&second).expect("a later close finishes");

        assert!(
            first.is_err() || state.lifecycle() == Lifecycle::Closed,
            "an expired drain must not report success while waiters remain"
        );
        assert_eq!(state.lifecycle(), Lifecycle::Closed);
    }
}
