//! Live reuse is observable through immutable bytes and finite copy capacity.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mado_pilot_capture::{
    CaptureFault, CpuMapping, CpuPixels, Frame, FrameDescriptor, FrameStorage, FrameView,
    PixelFormat,
};
use mado_pilot_core::{
    CancellationToken, Clock, FrameStamp, GeometryRevision, IdentityIssuer, MonotonicInstant,
    Operation, OperationContext, PixelExtent, PixelRect, Result, Status, StreamCursor,
    TransformSnapshot,
};

#[derive(Debug)]
struct Budget(AtomicUsize);

impl Budget {
    fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self(AtomicUsize::new(capacity)))
    }

    fn reserve(self: &Arc<Self>) -> Result<Arc<dyn Send + Sync>> {
        self.0
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |free| {
                free.checked_sub(1)
            })
            .map_err(|_| CaptureFault::StorageBudgetExhausted)?;
        Ok(Arc::new(Lease(Arc::clone(self))))
    }

    fn remaining(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }
}

#[derive(Debug)]
struct Lease(Arc<Budget>);

impl Drop for Lease {
    fn drop(&mut self) {
        self.0.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Debug)]
struct BoundedStorage {
    descriptor: FrameDescriptor,
    pixels: Arc<CpuPixels>,
    copies: Arc<Budget>,
    readbacks: Arc<Budget>,
    native: bool,
}

impl FrameStorage for BoundedStorage {
    fn descriptor(&self) -> FrameDescriptor {
        self.descriptor
    }

    fn cpu_pixels(&self) -> Option<Arc<CpuPixels>> {
        (!self.native).then(|| Arc::clone(&self.pixels))
    }

    fn read_cpu(&self, operation: &OperationContext) -> Result<Arc<CpuPixels>> {
        let attempt = Operation::admit(operation)?;
        let pixels = if self.native {
            let lease = self.readbacks.reserve()?;
            Arc::new(CpuPixels::with_retainer(self.pixels.bytes().into(), lease))
        } else {
            Arc::clone(&self.pixels)
        };
        Ok(attempt.commit(pixels)?)
    }

    fn reserve_cpu_copy(&self, _bytes: usize) -> Result<Option<Arc<dyn Send + Sync>>> {
        self.copies.reserve().map(Some)
    }
}

fn stamp() -> FrameStamp {
    let issuer = IdentityIssuer::new();
    StreamCursor::new(issuer.issue_stream().expect("issued"))
        .publish(GeometryRevision::FIRST)
        .expect("published")
}

fn descriptor() -> FrameDescriptor {
    FrameDescriptor::new(PixelExtent::new(6, 2), PixelFormat::Rgba8, 32).expect("valid")
}

fn bounded_frame(
    stamp: FrameStamp,
    descriptor: FrameDescriptor,
    seed: u8,
    capacity: usize,
    native: bool,
) -> (Frame, Arc<Budget>) {
    let width = usize::try_from(descriptor.extent().width()).expect("small");
    let height = usize::try_from(descriptor.extent().height()).expect("small");
    let mut bytes = vec![0xEE; descriptor.byte_len()];
    for row in 0..height {
        for column in 0..width * 4 {
            bytes[row * descriptor.stride() + column] =
                seed.wrapping_add(u8::try_from(row * width * 4 + column).expect("small"));
        }
    }
    let copies = Budget::new(capacity);
    let storage = BoundedStorage {
        descriptor,
        pixels: Arc::new(CpuPixels::new(bytes.into_boxed_slice())),
        copies: Arc::clone(&copies),
        readbacks: Budget::new(1),
        native,
    };
    let frame = Frame::from_storage(
        stamp,
        MonotonicInstant::ORIGIN,
        TransformSnapshot::frame_only(stamp.geometry(), descriptor.extent()),
        Arc::new(storage),
    )
    .expect("valid frame");
    (frame, copies)
}

fn view(frame: &Frame, column: i32) -> FrameView {
    FrameView::new(
        frame.clone(),
        PixelRect::new(column, 0, column + 1, 2).expect("valid rectangle"),
    )
    .expect("inside frame")
}

fn assert_mapping(
    mapping: &CpuMapping,
    frame: &Frame,
    region: PixelRect,
    format: PixelFormat,
    seed: u8,
) {
    let source = frame.descriptor();
    let mut expected = Vec::new();
    for row in region.top()..region.bottom() {
        for column in region.left()..region.right() {
            let offset = (u32::try_from(row).expect("nonnegative") * source.extent().width()
                + u32::try_from(column).expect("nonnegative"))
                * 4;
            let mut pixel = [0; 4];
            for (channel, byte) in pixel.iter_mut().enumerate() {
                *byte = seed.wrapping_add(
                    u8::try_from(offset + u32::try_from(channel).expect("small")).expect("small"),
                );
            }
            if source.format() != format {
                pixel.swap(0, 2);
            }
            expected.extend_from_slice(&pixel);
        }
    }
    assert_eq!(mapping.bytes(), expected);
    assert_eq!(mapping.stamp(), frame.stamp());
    assert_eq!(mapping.transform(), frame.transform());
    assert_eq!(mapping.region(), region);
    assert_eq!(
        mapping.descriptor(),
        FrameDescriptor::packed(PixelExtent::new(region.width(), region.height()), format)
            .expect("valid")
    );
    assert!(!mapping.is_shared());
}

#[test]
fn a_live_owner_reuses_one_copy_slot_before_readback_and_releases_it_with_the_last_clone() {
    let (frame, _) = bounded_frame(stamp(), descriptor(), 10, 1, true);
    let context = OperationContext::new();
    let first_view = view(&frame, 1);
    let first = first_view
        .map(PixelFormat::Rgba8, &context)
        .expect("first copy");
    assert_eq!(first.bytes(), &[14, 15, 16, 17, 38, 39, 40, 41]);
    let retained = first.clone();
    let cloned_frame = frame.clone();

    // This full-frame mapping occupies the only readback slot, not the ROI slot.
    let full = frame.map(PixelFormat::Rgba8, &context).expect("readback");
    let reused = view(&cloned_frame, 1)
        .map(PixelFormat::Rgba8, &context)
        .expect("live ROI needs neither readback nor another copy slot");
    assert_mapping(&reused, &frame, first_view.region(), PixelFormat::Rgba8, 10);
    assert_eq!(reused.source_instance(), retained.source_instance());
    assert!(!full.is_shared());
    assert_eq!(full.descriptor(), frame.descriptor());
    assert_eq!(full.source_instance(), retained.source_instance());
    assert_eq!(full.bytes().len(), descriptor().byte_len());
    drop(full);

    let other = view(&frame, 2);
    for (requested, format) in [
        (&other, PixelFormat::Rgba8),
        (&first_view, PixelFormat::Bgra8),
    ] {
        assert_eq!(
            requested
                .map(format, &context)
                .expect_err("copy capacity is occupied")
                .status(),
            Status::LimitExceeded
        );
    }
    drop(first);
    drop(reused);
    assert_eq!(
        other
            .map(PixelFormat::Rgba8, &context)
            .expect_err("the clone still owns the slot")
            .status(),
        Status::LimitExceeded
    );
    assert_mapping(
        &retained,
        &frame,
        first_view.region(),
        PixelFormat::Rgba8,
        10,
    );
    drop(retained);

    let replacement = other
        .map(PixelFormat::Rgba8, &context)
        .expect("last owner released capacity");
    assert_mapping(&replacement, &frame, other.region(), PixelFormat::Rgba8, 10);
    drop(replacement);
    let renewed = first_view
        .map(PixelFormat::Bgra8, &context)
        .expect("different format after release");
    assert_mapping(
        &renewed,
        &frame,
        first_view.region(),
        PixelFormat::Bgra8,
        10,
    );
    drop(renewed);
    let expired = first_view
        .map(PixelFormat::Rgba8, &context)
        .expect("expired weak entry can be replaced");
    assert_mapping(
        &expired,
        &frame,
        first_view.region(),
        PixelFormat::Rgba8,
        10,
    );
}

#[test]
fn at_most_four_live_regions_are_reused_and_eviction_preserves_owned_bytes() {
    let (frame, _) = bounded_frame(stamp(), descriptor(), 20, 5, false);
    let context = OperationContext::new();
    let mut owners: Vec<_> = (0..4)
        .map(|column| {
            Some(
                view(&frame, column)
                    .map(PixelFormat::Rgba8, &context)
                    .expect("capacity"),
            )
        })
        .collect();
    let touched = view(&frame, 0)
        .map(PixelFormat::Rgba8, &context)
        .expect("refresh recency");
    drop(touched);
    owners.push(Some(
        view(&frame, 4)
            .map(PixelFormat::Rgba8, &context)
            .expect("fifth copy"),
    ));

    for column in [0, 2, 3, 4] {
        let reused = view(&frame, column)
            .map(PixelFormat::Rgba8, &context)
            .expect("one of four live entries");
        assert_mapping(
            &reused,
            &frame,
            view(&frame, column).region(),
            PixelFormat::Rgba8,
            20,
        );
    }
    assert_eq!(
        view(&frame, 1)
            .map(PixelFormat::Rgba8, &context)
            .expect_err("evicted while all five slots are owned")
            .status(),
        Status::LimitExceeded
    );
    for (column, owner) in owners.iter().enumerate() {
        assert_mapping(
            owner.as_ref().expect("retained"),
            &frame,
            view(&frame, i32::try_from(column).expect("small")).region(),
            PixelFormat::Rgba8,
            20,
        );
    }
    drop(owners[1].take());
    let renewed = view(&frame, 1)
        .map(PixelFormat::Rgba8, &context)
        .expect("evicted owner released its slot");
    assert_mapping(
        &renewed,
        &frame,
        view(&frame, 1).region(),
        PixelFormat::Rgba8,
        20,
    );
}

#[test]
fn equal_stamps_do_not_share_pixels_and_frame_clones_keep_the_same_provenance() {
    let stamp = stamp();
    let (first, _) = bounded_frame(stamp, descriptor(), 30, 1, false);
    let (second, _) = bounded_frame(stamp, descriptor(), 90, 1, false);
    let context = OperationContext::new();
    let one = first.map(PixelFormat::Bgra8, &context).expect("first swap");
    let two = second
        .map(PixelFormat::Bgra8, &context)
        .expect("separate swap");
    let reused = first
        .clone()
        .map(PixelFormat::Bgra8, &context)
        .expect("clone reuses the full swap");
    assert_mapping(
        &one,
        &first,
        first.bounds().expect("bounds"),
        PixelFormat::Bgra8,
        30,
    );
    assert_mapping(
        &two,
        &second,
        second.bounds().expect("bounds"),
        PixelFormat::Bgra8,
        90,
    );
    assert_eq!(one.bytes(), reused.bytes());
    assert_eq!(one.source_instance(), reused.source_instance());
    assert_ne!(one.source_instance(), two.source_instance());
    assert_ne!(one.bytes(), two.bytes());
    let shared = first
        .map(PixelFormat::Rgba8, &context)
        .expect("source sharing needs no copy slot");
    assert!(shared.is_shared());
    assert_eq!(shared.descriptor(), first.descriptor());
    assert_eq!(&shared.bytes()[..4], &[30, 31, 32, 33]);
    assert_eq!(shared.source_instance(), one.source_instance());
    assert_eq!(
        format!("{:?}", one.source_instance()),
        "FrameInstance { .. }"
    );

    let provenance = one.source_instance().clone();
    drop(first);
    assert!(!provenance.is_alive());
    assert!(!shared.source_instance().is_alive());
    assert_eq!(one.source_instance(), reused.clone().source_instance());
    assert_eq!(one.bytes(), reused.bytes());
    assert_eq!(&shared.bytes()[..4], &[30, 31, 32, 33]);
    assert!(two.source_instance().is_alive());
}

#[test]
fn sequence_epoch_geometry_source_and_layout_transitions_keep_exact_metadata() {
    let issuer = IdentityIssuer::new();
    let mut cursor = StreamCursor::new(issuer.issue_stream().expect("stream"));
    let first = cursor.publish(GeometryRevision::FIRST).expect("first");
    let next = cursor.publish(GeometryRevision::FIRST).expect("sequence");
    cursor.begin_epoch().expect("new epoch");
    let epoch = cursor.publish(GeometryRevision::FIRST).expect("epoch");
    let geometry = cursor
        .publish(GeometryRevision::FIRST.next().expect("revision"))
        .expect("geometry");
    let other = StreamCursor::new(issuer.issue_stream().expect("other stream"))
        .publish(GeometryRevision::FIRST)
        .expect("source");
    let changed =
        FrameDescriptor::new(PixelExtent::new(7, 3), PixelFormat::Bgra8, 36).expect("layout");
    let context = OperationContext::new();
    let mut retained = Vec::new();
    for (index, stamp) in [first, next, epoch, geometry, other]
        .into_iter()
        .enumerate()
    {
        let source = if index >= 3 { changed } else { descriptor() };
        let seed = u8::try_from(index * 20).expect("small");
        let (frame, _) = bounded_frame(stamp, source, seed, 1, false);
        let mapped_view = view(&frame, 1);
        let mapping = mapped_view
            .map(PixelFormat::Rgba8, &context)
            .expect("current source");
        assert_mapping(
            &mapping,
            &frame,
            mapped_view.region(),
            PixelFormat::Rgba8,
            seed,
        );
        retained.push((frame, mapping, seed));
    }
    for (frame, mapping, seed) in &retained {
        let reused = view(frame, 1)
            .map(PixelFormat::Rgba8, &context)
            .expect("old source still reusable");
        assert_mapping(
            &reused,
            frame,
            view(frame, 1).region(),
            PixelFormat::Rgba8,
            *seed,
        );
        assert_eq!(mapping.bytes(), reused.bytes());
        assert_eq!(mapping.source_instance(), reused.source_instance());
    }
    for pair in retained.windows(2) {
        assert_ne!(pair[0].1.source_instance(), pair[1].1.source_instance());
    }
}

#[derive(Debug)]
struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> MonotonicInstant {
        MonotonicInstant::ORIGIN
    }
}

#[test]
fn cancelled_and_expired_hits_and_misses_keep_their_own_terminal_outcome() {
    let (frame, _) = bounded_frame(stamp(), descriptor(), 40, 1, true);
    let context = OperationContext::new();
    let retained = view(&frame, 0)
        .map(PixelFormat::Rgba8, &context)
        .expect("warm owner");
    let token = CancellationToken::new();
    token.cancel();
    let cancelled = OperationContext::new().with_cancellation(token);
    let expired = OperationContext::new()
        .with_clock(Arc::new(FixedClock))
        .with_deadline(MonotonicInstant::ORIGIN);
    for (operation, expected) in [
        (&cancelled, Status::Cancelled),
        (&expired, Status::DeadlineExceeded),
    ] {
        for column in [0, 1] {
            assert_eq!(
                view(&frame, column)
                    .map(PixelFormat::Rgba8, operation)
                    .expect_err("authority ended")
                    .status(),
                expected
            );
        }
    }
    let reused = view(&frame, 0)
        .map(PixelFormat::Rgba8, &context)
        .expect("independent authority");
    assert_mapping(
        &reused,
        &frame,
        view(&frame, 0).region(),
        PixelFormat::Rgba8,
        40,
    );
    assert_eq!(retained.bytes(), reused.bytes());
    drop(retained);
    drop(reused);
    let next = view(&frame, 1)
        .map(PixelFormat::Rgba8, &context)
        .expect("no failed caller retained capacity");
    assert_mapping(
        &next,
        &frame,
        view(&frame, 1).region(),
        PixelFormat::Rgba8,
        40,
    );
}

#[derive(Debug)]
struct CancelOnClock(CancellationToken);

impl Clock for CancelOnClock {
    fn now(&self) -> MonotonicInstant {
        self.0.cancel();
        MonotonicInstant::ORIGIN
    }
}

fn future_deadline() -> MonotonicInstant {
    MonotonicInstant::ORIGIN
        .checked_add(Duration::from_millis(1))
        .expect("representable")
}

#[test]
fn cancellation_after_admission_prevents_both_cached_and_new_results() {
    for warm in [false, true] {
        let (frame, _) = bounded_frame(stamp(), descriptor(), 50, 1, false);
        let mapped_view = view(&frame, 1);
        let retained = warm.then(|| {
            mapped_view
                .map(PixelFormat::Rgba8, &OperationContext::new())
                .expect("warm")
        });
        let token = CancellationToken::new();
        let operation = OperationContext::new()
            .with_cancellation(token.clone())
            .with_clock(Arc::new(CancelOnClock(token)))
            .with_deadline(future_deadline());
        assert_eq!(
            mapped_view
                .map(PixelFormat::Rgba8, &operation)
                .expect_err("cancelled before commit")
                .status(),
            Status::Cancelled
        );
        let recovered = mapped_view
            .map(PixelFormat::Rgba8, &OperationContext::new())
            .expect("capacity is available or reusable");
        assert_mapping(
            &recovered,
            &frame,
            mapped_view.region(),
            PixelFormat::Rgba8,
            50,
        );
        drop(retained);
    }
}

#[derive(Debug)]
struct ExpireWhenReserved(Arc<Budget>);

impl Clock for ExpireWhenReserved {
    fn now(&self) -> MonotonicInstant {
        if self.0.remaining() == 0 {
            future_deadline()
        } else {
            MonotonicInstant::ORIGIN
        }
    }
}

#[test]
fn expiry_during_copy_releases_the_reservation_without_a_late_result() {
    let (frame, copies) = bounded_frame(stamp(), descriptor(), 60, 1, false);
    let mapped_view = view(&frame, 1);
    let operation = OperationContext::new()
        .with_clock(Arc::new(ExpireWhenReserved(copies)))
        .with_deadline(future_deadline());
    assert_eq!(
        mapped_view
            .map(PixelFormat::Bgra8, &operation)
            .expect_err("deadline ended during preparation")
            .status(),
        Status::DeadlineExceeded
    );
    let recovered = mapped_view
        .map(PixelFormat::Bgra8, &OperationContext::new())
        .expect("refused preparation released its reservation");
    assert_mapping(
        &recovered,
        &frame,
        mapped_view.region(),
        PixelFormat::Bgra8,
        60,
    );
}

#[derive(Debug)]
struct ReentrantClock {
    view: FrameView,
    retained: Mutex<Option<CpuMapping>>,
}

impl Clock for ReentrantClock {
    fn now(&self) -> MonotonicInstant {
        let mapping = self
            .view
            .map(PixelFormat::Rgba8, &OperationContext::new())
            .expect("a clock callback can reuse live pixels with no free copy slot");
        let previous = self.retained.lock().expect("uncontended").replace(mapping);
        drop(previous);
        MonotonicInstant::ORIGIN
    }
}

#[test]
fn clock_callbacks_can_reenter_a_hit_with_capacity_one() {
    let (frame, _) = bounded_frame(stamp(), descriptor(), 70, 1, false);
    let mapped_view = view(&frame, 1);
    let first = mapped_view
        .map(PixelFormat::Rgba8, &OperationContext::new())
        .expect("warm");
    let clock = Arc::new(ReentrantClock {
        view: mapped_view.clone(),
        retained: Mutex::new(None),
    });
    let operation = OperationContext::new()
        .with_clock(clock.clone())
        .with_deadline(future_deadline());
    let outer = mapped_view
        .map(PixelFormat::Rgba8, &operation)
        .expect("reentrant hit");
    let nested = clock
        .retained
        .lock()
        .expect("uncontended")
        .take()
        .expect("clock produced a mapping");
    assert_mapping(
        &nested,
        &frame,
        mapped_view.region(),
        PixelFormat::Rgba8,
        70,
    );
    assert_eq!(outer.bytes(), nested.bytes());
    drop(first);
    drop(outer);
    assert_eq!(
        view(&frame, 2)
            .map(PixelFormat::Rgba8, &OperationContext::new())
            .expect_err("nested owner retains the slot")
            .status(),
        Status::LimitExceeded
    );
    drop(nested);
    let next = view(&frame, 2)
        .map(PixelFormat::Rgba8, &OperationContext::new())
        .expect("nested owner released the slot");
    assert_mapping(
        &next,
        &frame,
        view(&frame, 2).region(),
        PixelFormat::Rgba8,
        70,
    );
}

#[derive(Debug)]
struct FillDuringCheckpoint {
    view: FrameView,
    copies: Arc<Budget>,
    retained: Mutex<Option<CpuMapping>>,
}

impl Clock for FillDuringCheckpoint {
    fn now(&self) -> MonotonicInstant {
        if self.copies.remaining() == 1 && self.retained.lock().expect("uncontended").is_none() {
            let mapping = self
                .view
                .map(PixelFormat::Rgba8, &OperationContext::new())
                .expect("independent fill");
            *self.retained.lock().expect("uncontended") = Some(mapping);
        }
        MonotonicInstant::ORIGIN
    }
}

#[test]
fn publication_reuses_a_same_key_fill_and_releases_the_losing_copy() {
    let (frame, copies) = bounded_frame(stamp(), descriptor(), 80, 2, false);
    let mapped_view = view(&frame, 1);
    let clock = Arc::new(FillDuringCheckpoint {
        view: mapped_view.clone(),
        copies,
        retained: Mutex::new(None),
    });
    let operation = OperationContext::new()
        .with_clock(clock.clone())
        .with_deadline(future_deadline());
    let outer = mapped_view
        .map(PixelFormat::Rgba8, &operation)
        .expect("outer fill");
    let nested = clock
        .retained
        .lock()
        .expect("uncontended")
        .take()
        .expect("checkpoint filled the same key");
    assert_mapping(&outer, &frame, mapped_view.region(), PixelFormat::Rgba8, 80);
    assert_eq!(outer.bytes(), nested.bytes());
    let other = view(&frame, 2)
        .map(PixelFormat::Rgba8, &OperationContext::new())
        .expect("deduplication released the second copy slot");
    assert_mapping(
        &other,
        &frame,
        view(&frame, 2).region(),
        PixelFormat::Rgba8,
        80,
    );
    drop(outer);
    assert_eq!(
        view(&frame, 3)
            .map(PixelFormat::Rgba8, &OperationContext::new())
            .expect_err("nested and other owners fill the budget")
            .status(),
        Status::LimitExceeded
    );
    drop(nested);
    let third = view(&frame, 3)
        .map(PixelFormat::Rgba8, &OperationContext::new())
        .expect("last same-key owner released capacity");
    assert_mapping(
        &third,
        &frame,
        view(&frame, 3).region(),
        PixelFormat::Rgba8,
        80,
    );
}
