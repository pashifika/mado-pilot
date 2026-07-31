//! What a frame backed by native storage guarantees.
//!
//! The double below is not a platform adapter. It is the smallest thing that has
//! the two properties a real one has and CPU replay bytes do not: its pixels are
//! not readable until something converts them, and the storage it publishes holds
//! a lease on a finite resource. Those are what the rules in this file are about,
//! so a test that used CPU storage would pass without exercising any of them.
//!
//! The producer is modelled after the accepted Windows ownership rule
//! (`docs/adr/0013-windows-capture-frame-detachment.md`): a capture takes a
//! producer slot, copies into a detached slot, and releases the producer slot
//! before publishing. The point of that order is that a caller retaining frames
//! consumes detached capacity and never producer capacity, and the tests here
//! observe exactly that.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mado_pilot_capture::{
    CaptureFault, Continuity, CpuPixels, Frame, FrameDescriptor, FrameStorage, Lifecycle,
    PixelFormat, StoragePublication, StreamState,
};
use mado_pilot_core::{
    CancellationToken, ClipPolicy, Clock, CoordinateSpace, FrameStamp, GeometryRevision,
    IdentityIssuer, MonotonicInstant, Operation, OperationContext, PixelExtent, PixelRect, Rect,
    Result, Status, StreamCursor, TransformSnapshot,
};

/// A clock that stays at the origin for a fixed number of reads, then jumps past
/// any deadline set at one millisecond.
///
/// Counting reads is how a test places the expiry at an exact point in an
/// operation's sequence. Mapping native storage reads the clock three times — the
/// mapping's own admission, the conversion's admission, and the conversion's
/// commit — so `ExpireAfterReads::new(2)` is the case where an uninterruptible
/// conversion runs to the end and then may not commit.
#[derive(Debug)]
struct ExpireAfterReads {
    remaining_at_origin: usize,
    reads: AtomicUsize,
}

impl ExpireAfterReads {
    fn new(remaining_at_origin: usize) -> Self {
        Self {
            remaining_at_origin,
            reads: AtomicUsize::new(0),
        }
    }
}

impl Clock for ExpireAfterReads {
    fn now(&self) -> MonotonicInstant {
        let elapsed = if self.reads.fetch_add(1, Ordering::Relaxed) < self.remaining_at_origin {
            Duration::ZERO
        } else {
            Duration::from_millis(2)
        };
        MonotonicInstant::ORIGIN
            .checked_add(elapsed)
            .expect("test instant is representable")
    }
}

/// A producer with a finite pool and a finite detached budget.
#[derive(Debug)]
struct Producer {
    descriptor: FrameDescriptor,
    producer_slots: Mutex<usize>,
    detached_slots: Mutex<usize>,
    conversions: AtomicUsize,
}

impl Producer {
    fn new(descriptor: FrameDescriptor, pool: usize, detached: usize) -> Arc<Self> {
        Arc::new(Self {
            descriptor,
            producer_slots: Mutex::new(pool),
            detached_slots: Mutex::new(detached),
            conversions: AtomicUsize::new(0),
        })
    }

    /// Acquires a frame the way the accepted ownership rule does: take a producer
    /// slot, copy into a detached slot, release the producer slot, publish.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureFault::StorageBudgetExhausted`] when every detached slot is
    /// leased, and [`CaptureFault::SourceInvalid`] when the producer pool itself is
    /// empty — which cannot happen here, because the producer slot is released
    /// before this returns either way.
    fn capture(
        self: &Arc<Self>,
        fill: u8,
    ) -> std::result::Result<Arc<NativeStorage>, CaptureFault> {
        {
            let mut slots = self.producer_slots.lock().expect("uncontended");
            if *slots == 0 {
                return Err(CaptureFault::SourceInvalid);
            }
            *slots -= 1;
        }

        // Copy into a detached slot, then release the producer slot whatever the
        // outcome: the producer must not be held hostage by the detached budget.
        let detached = {
            let mut detached = self.detached_slots.lock().expect("uncontended");
            if *detached == 0 {
                None
            } else {
                *detached -= 1;
                Some(())
            }
        };
        *self.producer_slots.lock().expect("uncontended") += 1;

        match detached {
            Some(()) => Ok(Arc::new(NativeStorage {
                producer: Arc::clone(self),
                descriptor: self.descriptor,
                fill,
            })),
            None => Err(CaptureFault::StorageBudgetExhausted),
        }
    }

    fn producer_slots_free(&self) -> usize {
        *self.producer_slots.lock().expect("uncontended")
    }

    fn detached_slots_free(&self) -> usize {
        *self.detached_slots.lock().expect("uncontended")
    }

    fn conversions(&self) -> usize {
        self.conversions.load(Ordering::Relaxed)
    }
}

/// Storage whose pixels exist only after a conversion, holding a detached slot.
#[derive(Debug)]
struct NativeStorage {
    producer: Arc<Producer>,
    descriptor: FrameDescriptor,
    fill: u8,
}

/// Deliberately invalid two-slot storage used only to prove the retained-byte
/// oracle detects aliasing. It overwrites a slot even while an older frame owns
/// a reference to that slot, which is exactly what the production ownership
/// rule forbids.
#[derive(Debug)]
struct OverwritingRing {
    descriptor: FrameDescriptor,
    slots: [Arc<Mutex<Box<[u8]>>>; 2],
    next: AtomicUsize,
}

impl OverwritingRing {
    fn new(descriptor: FrameDescriptor) -> Self {
        let empty = || {
            Arc::new(Mutex::new(
                vec![0; descriptor.byte_len()].into_boxed_slice(),
            ))
        };
        Self {
            descriptor,
            slots: [empty(), empty()],
            next: AtomicUsize::new(0),
        }
    }

    fn capture(&self, fill: u8) -> Arc<OverwrittenStorage> {
        let index = self.next.fetch_add(1, Ordering::Relaxed) % self.slots.len();
        self.slots[index].lock().expect("uncontended").fill(fill);
        Arc::new(OverwrittenStorage {
            descriptor: self.descriptor,
            pixels: Arc::clone(&self.slots[index]),
        })
    }
}

#[derive(Debug)]
struct OverwrittenStorage {
    descriptor: FrameDescriptor,
    pixels: Arc<Mutex<Box<[u8]>>>,
}

impl FrameStorage for OverwrittenStorage {
    fn descriptor(&self) -> FrameDescriptor {
        self.descriptor
    }

    fn cpu_pixels(&self) -> Option<Arc<CpuPixels>> {
        None
    }

    fn read_cpu(&self, operation: &OperationContext) -> Result<Arc<CpuPixels>> {
        let attempt = Operation::admit(operation)?;
        let bytes = self.pixels.lock().expect("uncontended").clone();
        Ok(attempt.commit(Arc::new(CpuPixels::new(bytes)))?)
    }
}

impl Drop for NativeStorage {
    fn drop(&mut self) {
        *self.producer.detached_slots.lock().expect("uncontended") += 1;
    }
}

impl FrameStorage for NativeStorage {
    fn descriptor(&self) -> FrameDescriptor {
        self.descriptor
    }

    fn cpu_pixels(&self) -> Option<Arc<CpuPixels>> {
        // Native storage is not CPU-readable, which is the whole difference.
        None
    }

    fn read_cpu(&self, operation: &OperationContext) -> Result<Arc<CpuPixels>> {
        let attempt = Operation::admit(operation)?;
        self.producer.conversions.fetch_add(1, Ordering::Relaxed);
        let pixels = Arc::new(CpuPixels::new(
            vec![self.fill; self.descriptor.byte_len()].into_boxed_slice(),
        ));
        // The conversion is allowed to finish; what it may not do is commit late.
        Ok(attempt.commit(pixels)?)
    }
}

fn descriptor(width: u32, height: u32) -> FrameDescriptor {
    FrameDescriptor::packed(PixelExtent::new(width, height), PixelFormat::Bgra8).expect("valid")
}

fn captured_at() -> MonotonicInstant {
    MonotonicInstant::ORIGIN
        .checked_add(Duration::from_millis(37))
        .expect("test timestamp is representable")
}

fn publication(storage: Arc<dyn FrameStorage>, continuity: Continuity) -> StoragePublication {
    StoragePublication {
        captured_at: captured_at(),
        placement: None,
        storage,
        continuity,
    }
}

fn stream() -> StreamState {
    let issuer = IdentityIssuer::new();
    StreamState::new(issuer.issue_stream().expect("issued"))
}

fn lone_stamp() -> FrameStamp {
    let issuer = IdentityIssuer::new();
    let mut cursor = StreamCursor::new(issuer.issue_stream().expect("issued"));
    cursor.publish(GeometryRevision::FIRST).expect("published")
}

#[test]
fn a_retained_frame_never_holds_a_producer_slot() {
    let producer = Producer::new(descriptor(8, 6), 2, 8);
    let stream = stream();
    let mut retained = Vec::new();

    for fill in 0..8u8 {
        let storage = producer.capture(fill).expect("detached capacity remains");
        retained.push(
            stream
                .publish_storage(publication(storage, Continuity::Continuous))
                .map_err(|refused| refused.into_error())
                .expect("published"),
        );
        assert_eq!(
            producer.producer_slots_free(),
            2,
            "the producer slot is released before publication, so retention cannot hold it"
        );
    }

    assert_eq!(retained.len(), 8);
    assert_eq!(
        producer.detached_slots_free(),
        0,
        "retention costs detached capacity"
    );
}

#[test]
fn an_exhausted_detached_budget_is_a_bounded_failure_rather_than_a_stall() {
    let producer = Producer::new(descriptor(4, 4), 2, 2);
    let stream = stream();
    let first = stream
        .publish_storage(publication(
            producer.capture(1).expect("free"),
            Continuity::Continuous,
        ))
        .map_err(|refused| refused.into_error())
        .expect("published");
    let second = stream
        .publish_storage(publication(
            producer.capture(2).expect("free"),
            Continuity::Continuous,
        ))
        .map_err(|refused| refused.into_error())
        .expect("published");

    let refused = producer.capture(3).expect_err("the budget is exhausted");

    assert_eq!(refused, CaptureFault::StorageBudgetExhausted);
    assert_eq!(
        producer.producer_slots_free(),
        2,
        "an exhausted budget does not consume the producer pool"
    );

    // Releasing one retained frame returns exactly one detached slot.
    drop(first);
    assert_eq!(producer.detached_slots_free(), 1);
    producer.capture(4).expect("capacity came back");
    drop(second);
}

#[test]
fn the_retained_byte_oracle_rejects_an_overwriting_two_slot_ring() {
    let ring = OverwritingRing::new(descriptor(4, 4));
    let stream = stream();
    let context = OperationContext::new();
    let retained = stream
        .publish_storage(publication(ring.capture(0x11), Continuity::Continuous))
        .map_err(|refused| refused.into_error())
        .expect("first slot published");
    let immediate = retained
        .map(PixelFormat::Bgra8, &context)
        .expect("immediate mapping")
        .bytes()
        .to_vec();

    for fill in [0x22, 0x33] {
        stream
            .publish_storage(publication(ring.capture(fill), Continuity::Continuous))
            .map_err(|refused| refused.into_error())
            .expect("blind ring published");
    }
    let delayed = retained
        .map(PixelFormat::Bgra8, &context)
        .expect("delayed mapping");

    assert_ne!(
        delayed.bytes(),
        immediate,
        "the negative control must corrupt retained bytes so the oracle can detect aliasing"
    );
    assert!(delayed.bytes().iter().all(|byte| *byte == 0x33));
}

#[test]
fn a_refused_publication_returns_the_storage_to_the_adapter() {
    let producer = Producer::new(descriptor(4, 4), 2, 2);
    let stream = stream();
    stream.begin_close();
    let storage = producer.capture(9).expect("free");

    let refused = stream
        .publish_storage(publication(
            Arc::clone(&storage) as Arc<dyn FrameStorage>,
            Continuity::Continuous,
        ))
        .expect_err("a closing stream refuses the publication");

    assert_eq!(refused.error().status(), Status::Closed);
    assert_eq!(
        refused.publication().storage.descriptor(),
        producer.descriptor
    );
    let returned = refused.into_publication();
    assert_eq!(returned.continuity, Continuity::Continuous);
    assert_eq!(
        producer.detached_slots_free(),
        1,
        "the refusal did not release the Adapter's lease"
    );
    drop(returned);
    drop(storage);
    assert_eq!(producer.detached_slots_free(), 2);
}

#[test]
fn a_refusal_never_leaks_pixel_content_into_diagnostics() {
    let producer = Producer::new(descriptor(2, 2), 1, 1);
    let stream = stream();
    stream.begin_close();

    let refused = stream
        .publish_storage(publication(
            producer.capture(0xAB).expect("free"),
            Continuity::Continuous,
        ))
        .expect_err("refused");
    let text = format!("{refused:?}");

    assert!(text.contains("RefusedStorage"), "{text}");
    assert!(text.contains("descriptor"), "{text}");
    assert!(!text.contains("171"), "pixel fill leaked: {text}");
}

#[test]
fn mapping_native_storage_converts_once_and_owns_its_bytes() {
    let producer = Producer::new(descriptor(4, 3), 2, 2);
    let frame = Frame::from_storage(
        lone_stamp(),
        MonotonicInstant::ORIGIN,
        TransformSnapshot::frame_only(GeometryRevision::FIRST, PixelExtent::new(4, 3)),
        producer.capture(0x5A).expect("free"),
    )
    .expect("valid");
    let context = OperationContext::new();

    let mapping = frame.map(PixelFormat::Bgra8, &context).expect("mapped");

    assert!(
        !mapping.is_shared(),
        "obtaining CPU bytes from native storage is a copy"
    );
    assert_eq!(producer.conversions(), 1);
    assert_eq!(mapping.stamp(), frame.stamp());
    assert_eq!(mapping.descriptor(), producer.descriptor);
    assert!(mapping.bytes().iter().all(|byte| *byte == 0x5A));
}

#[test]
fn a_mapping_outlives_the_frame_and_the_lease_it_came_from() {
    let producer = Producer::new(descriptor(4, 3), 2, 1);
    let frame = Frame::from_storage(
        lone_stamp(),
        MonotonicInstant::ORIGIN,
        TransformSnapshot::frame_only(GeometryRevision::FIRST, PixelExtent::new(4, 3)),
        producer.capture(0x11).expect("free"),
    )
    .expect("valid");
    let mapping = frame
        .map(PixelFormat::Bgra8, &OperationContext::new())
        .expect("mapped");
    let expected: Vec<u8> = mapping.bytes().to_vec();

    drop(frame);

    assert_eq!(
        producer.detached_slots_free(),
        1,
        "a mapping owns its bytes, so the lease returns with the frame"
    );
    assert_eq!(mapping.bytes(), expected.as_slice());
}

#[test]
fn a_view_of_a_native_frame_maps_only_its_region() {
    let producer = Producer::new(descriptor(8, 6), 2, 2);
    let frame = Frame::from_storage(
        lone_stamp(),
        MonotonicInstant::ORIGIN,
        TransformSnapshot::frame_only(GeometryRevision::FIRST, PixelExtent::new(8, 6)),
        producer.capture(0x22).expect("free"),
    )
    .expect("valid");
    let half = Rect::new(CoordinateSpace::FrameNormalized, 0.0, 0.0, 0.5, 1.0).expect("valid");

    let view = frame.view(half, ClipPolicy::Reject).expect("inside");
    let mapping = view
        .map(PixelFormat::Bgra8, &OperationContext::new())
        .expect("mapped");

    assert_eq!(view.region(), PixelRect::new(0, 0, 4, 6).expect("valid"));
    assert_eq!(mapping.region(), view.region());
    assert_eq!(mapping.descriptor().extent(), PixelExtent::new(4, 6));
    assert_eq!(mapping.bytes().len(), 4 * 6 * 4);
    assert_eq!(
        mapping.stamp(),
        frame.stamp(),
        "the view kept its exact frame"
    );
}

#[test]
fn a_cancelled_mapping_of_native_storage_exposes_no_bytes() {
    let producer = Producer::new(descriptor(4, 4), 2, 2);
    let frame = Frame::from_storage(
        lone_stamp(),
        MonotonicInstant::ORIGIN,
        TransformSnapshot::frame_only(GeometryRevision::FIRST, PixelExtent::new(4, 4)),
        producer.capture(0x33).expect("free"),
    )
    .expect("valid");
    let token = CancellationToken::new();
    token.cancel();
    let cancelled = OperationContext::new().with_cancellation(token);

    let error = frame
        .map(PixelFormat::Bgra8, &cancelled)
        .expect_err("cancelled before admission");

    assert_eq!(error.status(), Status::Cancelled);
    assert_eq!(
        producer.conversions(),
        0,
        "a cancelled mapping does not convert"
    );
    assert!(
        frame
            .map(PixelFormat::Bgra8, &OperationContext::new())
            .is_ok(),
        "the frame is untouched by a failed mapping"
    );
}

#[test]
fn a_conversion_that_finishes_late_cannot_commit() {
    let producer = Producer::new(descriptor(4, 4), 2, 2);
    let frame = Frame::from_storage(
        lone_stamp(),
        MonotonicInstant::ORIGIN,
        TransformSnapshot::frame_only(GeometryRevision::FIRST, PixelExtent::new(4, 4)),
        producer.capture(0x44).expect("free"),
    )
    .expect("valid");
    let deadline = MonotonicInstant::ORIGIN
        .checked_add(Duration::from_millis(1))
        .expect("representable");
    let expiring = OperationContext::new()
        .with_clock(Arc::new(ExpireAfterReads::new(2)))
        .with_deadline(deadline);

    let error = frame
        .map(PixelFormat::Bgra8, &expiring)
        .expect_err("the deadline wins after the conversion");

    assert_eq!(error.status(), Status::DeadlineExceeded);
    assert_eq!(
        producer.conversions(),
        1,
        "uninterruptible work is allowed to finish; it is not allowed to commit"
    );
}

#[test]
fn a_native_frame_correlates_to_the_geometry_it_was_captured_under() {
    let producer = Producer::new(descriptor(4, 4), 2, 4);
    let resized = Producer::new(descriptor(8, 8), 2, 4);
    let stream = stream();

    let before = stream
        .publish_storage(publication(
            producer.capture(1).expect("free"),
            Continuity::Continuous,
        ))
        .map_err(|refused| refused.into_error())
        .expect("published");
    let after = stream
        .publish_storage(publication(
            resized.capture(2).expect("free"),
            // The Adapter claims continuity; the extent contradicts it.
            Continuity::Continuous,
        ))
        .map_err(|refused| refused.into_error())
        .expect("published");
    let context = OperationContext::new();
    let old_mapping = before.map(PixelFormat::Bgra8, &context).expect("mapped");

    assert_eq!(
        before.captured_at(),
        captured_at(),
        "storage publication preserved the capture timestamp"
    );
    assert_eq!(after.stamp().epoch().value(), 1, "a resize begins an epoch");
    assert_eq!(after.stamp().sequence().value(), 0);
    assert_ne!(after.stamp().geometry(), before.stamp().geometry());
    assert_eq!(
        old_mapping.stamp(),
        before.stamp(),
        "a mapping reports the exact frame it came from, not the current one"
    );
    assert_eq!(
        old_mapping.transform().geometry(),
        before.stamp().geometry()
    );
}

#[test]
fn close_keeps_retained_native_frames_usable_and_refuses_later_publication() {
    let producer = Producer::new(descriptor(4, 4), 2, 2);
    let stream = stream();
    let retained = stream
        .publish_storage(publication(
            producer.capture(0x77).expect("free"),
            Continuity::Continuous,
        ))
        .map_err(|refused| refused.into_error())
        .expect("published");
    let context = OperationContext::new();

    stream.drain(&context).expect("drained");
    let after_close = producer.capture(0x78).expect("free");
    let refused = stream
        .publish_storage(publication(after_close, Continuity::Continuous))
        .expect_err("a closed stream publishes nothing");

    assert_eq!(stream.lifecycle(), Lifecycle::Closed);
    assert_eq!(refused.error().status(), Status::Closed);
    let mapping = retained
        .map(PixelFormat::Bgra8, &context)
        .expect("a retained frame maps after close");
    assert!(mapping.bytes().iter().all(|byte| *byte == 0x77));
    drop(refused);
}

#[test]
fn storage_is_released_only_after_the_last_frame_that_holds_it() {
    let producer = Producer::new(descriptor(4, 4), 2, 1);
    let stream = stream();
    let published = stream
        .publish_storage(publication(
            producer.capture(0x99).expect("free"),
            Continuity::Continuous,
        ))
        .map_err(|refused| refused.into_error())
        .expect("published");
    let retained = published.clone();
    let view = published.full_view().expect("valid");
    let context = OperationContext::new();

    assert_eq!(producer.detached_slots_free(), 0);

    drop(published);
    assert_eq!(
        producer.detached_slots_free(),
        0,
        "the clone still holds it"
    );
    drop(retained);
    assert_eq!(producer.detached_slots_free(), 0, "the view still holds it");
    let mapping = view.map(PixelFormat::Bgra8, &context).expect("mapped");
    drop(view);
    assert_eq!(
        producer.detached_slots_free(),
        0,
        "the stream's own current frame still holds it"
    );

    stream.drain(&context).expect("drained");
    assert_eq!(
        producer.detached_slots_free(),
        1,
        "the last holder released the lease"
    );
    assert!(mapping.bytes().iter().all(|byte| *byte == 0x99));
}
