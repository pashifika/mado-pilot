//! Opaque frame storage a test drives by hand.
//!
//! Replay storage is CPU bytes, so every mapping of it shares them and no
//! conversion ever fails. A native frame is the opposite: its pixels do not exist
//! until something converts them, the conversion can be slow or fail, and the
//! storage holds a lease on a finite resource its producer needs back. This double
//! has all three properties, which is what makes the rules about retention,
//! conversion, and release reachable without a platform.
//!
//! The producer is modelled after the accepted Windows ownership rule in
//! `docs/adr/0013-windows-capture-frame-detachment.md`: a capture takes a producer
//! slot, copies into a detached slot, and releases the producer slot before it
//! publishes. Retention therefore costs detached capacity and never producer
//! capacity, and a test can observe both.

use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use mado_pilot_capture::{
    CaptureFault, Continuity, CpuPixels, FrameDescriptor, FrameStorage, PixelFormat,
    StoragePublication,
};
use mado_pilot_core::{
    MonotonicInstant, Operation, OperationContext, PixelExtent, Result, TargetPlacement,
};

/// What a conversion should do when a mapping asks for CPU pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conversion {
    /// Produce the pixels immediately.
    Immediate,
    /// Sleep for this long first, so a deadline can expire during it.
    Slow(Duration),
    /// Fail with this fault, as a lost device does.
    Fails(CaptureFault),
}

/// A producer with a finite pool and a finite detached budget.
///
/// Cloning is by [`Arc`], so a test and the storage it published share one set of
/// counters.
#[derive(Debug, Clone)]
pub struct ControlledProducer {
    inner: Arc<ProducerState>,
}

#[derive(Debug)]
struct ProducerState {
    descriptor: Mutex<FrameDescriptor>,
    producer_slots: Mutex<usize>,
    pool: usize,
    detached_slots: Mutex<usize>,
    detached_budget: usize,
    conversion: Mutex<Conversion>,
    conversions: AtomicUsize,
    drops: AtomicUsize,
}

impl ControlledProducer {
    /// Builds a producer of `extent` frames with `pool` producer slots and
    /// `detached` slots a caller's retention can occupy.
    ///
    /// # Errors
    ///
    /// Returns a capture fault for an extent and format that do not form a valid
    /// descriptor.
    pub fn new(
        extent: PixelExtent,
        format: PixelFormat,
        pool: usize,
        detached: usize,
    ) -> Result<Self> {
        Ok(Self {
            inner: Arc::new(ProducerState {
                descriptor: Mutex::new(FrameDescriptor::packed(extent, format)?),
                producer_slots: Mutex::new(pool),
                pool,
                detached_slots: Mutex::new(detached),
                detached_budget: detached,
                conversion: Mutex::new(Conversion::Immediate),
                conversions: AtomicUsize::new(0),
                drops: AtomicUsize::new(0),
            }),
        })
    }

    /// Sets what the next conversions do.
    pub fn set_conversion(&self, conversion: Conversion) {
        *self.inner.conversion.lock().expect("uncontended") = conversion;
    }

    /// Changes the extent the producer captures, as a resize does.
    ///
    /// # Errors
    ///
    /// Returns a capture fault for an extent that does not form a valid descriptor.
    pub fn resize(&self, extent: PixelExtent) -> Result<()> {
        let format = self.descriptor().format();
        *self.inner.descriptor.lock().expect("uncontended") =
            FrameDescriptor::packed(extent, format)?;
        Ok(())
    }

    /// Returns the layout the producer currently captures.
    #[must_use]
    pub fn descriptor(&self) -> FrameDescriptor {
        *self.inner.descriptor.lock().expect("uncontended")
    }

    /// Captures one frame of solid `fill`, detaching it from the producer.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureFault::StorageBudgetExhausted`] when every detached slot is
    /// leased by a frame, mapping, or backend a caller still holds. The producer
    /// slot is released either way, so an exhausted budget never costs producer
    /// capacity.
    pub fn capture(&self, fill: u8) -> Result<Arc<dyn FrameStorage>> {
        {
            let mut slots = self.inner.producer_slots.lock().expect("uncontended");
            if *slots == 0 {
                return Err(CaptureFault::SourceInvalid.into());
            }
            *slots -= 1;
        }
        let detached = {
            let mut detached = self.inner.detached_slots.lock().expect("uncontended");
            if *detached == 0 {
                false
            } else {
                *detached -= 1;
                true
            }
        };
        *self.inner.producer_slots.lock().expect("uncontended") += 1;

        if !detached {
            return Err(CaptureFault::StorageBudgetExhausted.into());
        }
        Ok(Arc::new(ControlledStorage {
            producer: Arc::clone(&self.inner),
            descriptor: self.descriptor(),
            fill,
        }))
    }

    /// Captures one frame and wraps it as a publication.
    ///
    /// # Errors
    ///
    /// As [`ControlledProducer::capture`].
    pub fn publication(&self, fill: u8, continuity: Continuity) -> Result<StoragePublication> {
        Ok(StoragePublication {
            captured_at: MonotonicInstant::ORIGIN,
            placement: None,
            storage: self.capture(fill)?,
            continuity,
        })
    }

    /// Captures one frame and wraps it as a publication with `placement`.
    ///
    /// # Errors
    ///
    /// As [`ControlledProducer::capture`].
    pub fn placed_publication(
        &self,
        fill: u8,
        placement: TargetPlacement,
        continuity: Continuity,
    ) -> Result<StoragePublication> {
        Ok(StoragePublication {
            captured_at: MonotonicInstant::ORIGIN,
            placement: Some(placement),
            storage: self.capture(fill)?,
            continuity,
        })
    }

    /// Returns how many producer slots are free.
    ///
    /// A retaining caller must never reduce this: that is the property detachment
    /// exists for.
    #[must_use]
    pub fn producer_slots_free(&self) -> usize {
        *self.inner.producer_slots.lock().expect("uncontended")
    }

    /// Returns how many producer slots the pool has.
    #[must_use]
    pub fn pool(&self) -> usize {
        self.inner.pool
    }

    /// Returns how many detached slots are free.
    #[must_use]
    pub fn detached_slots_free(&self) -> usize {
        *self.inner.detached_slots.lock().expect("uncontended")
    }

    /// Returns how many detached slots the budget has.
    #[must_use]
    pub fn detached_budget(&self) -> usize {
        self.inner.detached_budget
    }

    /// Returns how many conversions to CPU pixels have run.
    #[must_use]
    pub fn conversions(&self) -> usize {
        self.inner.conversions.load(Ordering::Relaxed)
    }

    /// Returns how many storage values have been released.
    #[must_use]
    pub fn releases(&self) -> usize {
        self.inner.drops.load(Ordering::Relaxed)
    }
}

/// Storage whose pixels exist only after a conversion, holding a detached slot.
struct ControlledStorage {
    producer: Arc<ProducerState>,
    descriptor: FrameDescriptor,
    fill: u8,
}

impl fmt::Debug for ControlledStorage {
    /// Formats the layout, never the pixels.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ControlledStorage")
            .field("descriptor", &self.descriptor)
            .finish()
    }
}

impl Drop for ControlledStorage {
    fn drop(&mut self) {
        *self.producer.detached_slots.lock().expect("uncontended") += 1;
        self.producer.drops.fetch_add(1, Ordering::Relaxed);
    }
}

impl FrameStorage for ControlledStorage {
    fn descriptor(&self) -> FrameDescriptor {
        self.descriptor
    }

    fn cpu_pixels(&self) -> Option<Arc<CpuPixels>> {
        // Native storage is not CPU-readable, which is the whole difference from
        // replay bytes.
        None
    }

    fn read_cpu(&self, operation: &OperationContext) -> Result<Arc<CpuPixels>> {
        let mut attempt = Operation::admit(operation)?;
        let conversion = *self.producer.conversion.lock().expect("uncontended");
        if let Conversion::Fails(fault) = conversion {
            return Err(fault.into());
        }
        if let Conversion::Slow(delay) = conversion {
            // A real conversion cannot be interrupted once the GPU has it, which is
            // exactly the case the contract has to survive: the work finishes and
            // then finds it is no longer allowed to commit.
            thread::sleep(delay);
            attempt.checkpoint()?;
        }
        self.producer.conversions.fetch_add(1, Ordering::Relaxed);
        let pixels = Arc::new(CpuPixels::new(
            vec![self.fill; self.descriptor.byte_len()].into_boxed_slice(),
        ));
        Ok(attempt.commit(pixels)?)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{ControlledProducer, Conversion};
    use mado_pilot_capture::{CaptureFault, PixelFormat};
    use mado_pilot_core::{OperationContext, PixelExtent, Status};

    fn producer(pool: usize, detached: usize) -> ControlledProducer {
        ControlledProducer::new(PixelExtent::new(4, 3), PixelFormat::Bgra8, pool, detached)
            .expect("valid")
    }

    #[test]
    fn storage_is_not_cpu_readable_until_it_is_converted() {
        let producer = producer(2, 2);
        let storage = producer.capture(0x5A).expect("free");

        assert!(storage.cpu_pixels().is_none());
        let pixels = storage
            .read_cpu(&OperationContext::new())
            .expect("converted");

        assert_eq!(producer.conversions(), 1);
        assert!(pixels.bytes().iter().all(|byte| *byte == 0x5A));
    }

    #[test]
    fn retention_costs_detached_capacity_and_not_producer_capacity() {
        let producer = producer(2, 3);
        let retained: Vec<_> = (0..3)
            .map(|fill| producer.capture(fill).expect("free"))
            .collect();

        assert_eq!(producer.producer_slots_free(), producer.pool());
        assert_eq!(producer.detached_slots_free(), 0);
        assert_eq!(
            producer
                .capture(4)
                .expect_err("the budget is exhausted")
                .status(),
            Status::LimitExceeded
        );

        drop(retained);
        assert_eq!(producer.detached_slots_free(), producer.detached_budget());
        assert_eq!(producer.releases(), 3);
    }

    #[test]
    fn a_failing_conversion_reports_its_fault() {
        let producer = producer(2, 2);
        let storage = producer.capture(1).expect("free");
        producer.set_conversion(Conversion::Fails(CaptureFault::SourceInvalid));

        let error = storage
            .read_cpu(&OperationContext::new())
            .expect_err("the conversion failed");

        assert_eq!(error.status(), Status::CaptureFailed);
        assert_eq!(producer.conversions(), 0);
    }

    #[test]
    fn a_slow_conversion_finishes_and_then_cannot_commit() {
        let producer = producer(2, 2);
        let storage = producer.capture(1).expect("free");
        producer.set_conversion(Conversion::Slow(Duration::from_millis(20)));
        let expiring = OperationContext::new()
            .with_timeout(Duration::from_millis(5))
            .expect("representable");

        let error = storage
            .read_cpu(&expiring)
            .expect_err("the deadline passed during the conversion");

        assert_eq!(error.status(), Status::DeadlineExceeded);
        assert_eq!(
            producer.conversions(),
            0,
            "an interrupted conversion produces no pixels to commit"
        );
    }

    #[test]
    fn a_resize_changes_what_later_frames_describe() {
        let producer = producer(2, 4);
        let before = producer.capture(1).expect("free");
        producer.resize(PixelExtent::new(8, 6)).expect("valid");
        let after = producer.capture(2).expect("free");

        assert_eq!(before.descriptor().extent(), PixelExtent::new(4, 3));
        assert_eq!(after.descriptor().extent(), PixelExtent::new(8, 6));
    }
}
