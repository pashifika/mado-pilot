//! Detached Core Video storage and its lazy CPU mapping.
//!
//! # Why the callback copies
//!
//! A ScreenCaptureKit surface belongs to a producer pool of fixed depth. Retaining
//! one until a consumer released the frame would let a retaining caller stall
//! capture, which the capture package's storage contract forbids. The producer
//! callback therefore copies the frame's content into Adapter-owned storage with
//! a finite detached count. Without caller byte limits, this uses the original
//! Core Video pool. With limits, explicit padded allocations carry last-owner
//! byte leases, so neither pooling nor close can hide retained payload.
//!
//! # Why mapping is separate
//!
//! The copy above is a detach, not a conversion: it preserves the native layout
//! and its row padding. Turning that into caller-readable bytes at an exact row
//! stride happens only when a caller maps the frame, under that caller's own
//! operation context.

use std::fmt;
use std::num::NonZeroU32;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use mado_pilot_capture::{CaptureFault, CpuPixels, FrameDescriptor, FrameStorage, PixelFormat};
use mado_pilot_core::{Operation, OperationContext, PixelExtent, Result};

use crate::shim::{DetachedFrame, PIXEL_BGRA8, ShimStatus};

/// How long a caller waiting on another caller's conversion sleeps before
/// re-checking its own operation context.
const MAPPING_POLL_INTERVAL: Duration = Duration::from_millis(2);

/// How many detached buffers one session may have leased at once.
///
/// These are full-frame CPU allocations rather than the GPU textures the Windows
/// Adapter budgets, so the bound is much smaller: eight covers the frames a
/// session normally has alive at once — the published latest one, whatever a
/// caller is holding, and a conversion in flight — with headroom, while keeping
/// worst-case retention within a few frame-sized allocations.
///
/// This is a reviewed bound, not a measured one. The Phase 2 numeric budgets under
/// gate `G-013` remain open, and this Change does not claim one.
pub(crate) const DETACHED_BUFFER_BUDGET: NonZeroU32 = NonZeroU32::new(8).unwrap();

/// Builds the descriptor for a frame the shim reported.
///
/// The published descriptor is packed even though the detached buffer has its own
/// row padding: the padding is the Adapter's, and a caller that received it would
/// read alignment bytes as image data.
pub(crate) fn descriptor_from_native(
    pixel_format: u32,
    extent: PixelExtent,
) -> std::result::Result<FrameDescriptor, CaptureFault> {
    if pixel_format != PIXEL_BGRA8 {
        return Err(CaptureFault::UnsupportedFormat);
    }
    FrameDescriptor::packed(extent, PixelFormat::Bgra8)
}

/// One published frame's immutable detached storage.
pub(crate) struct MacosFrameStorage {
    descriptor: FrameDescriptor,
    frame: DetachedFrame,
    mapping: Mutex<MappingState>,
    mapped: Condvar,
}

#[derive(Debug, Default)]
struct MappingState {
    active: bool,
    pixels: Option<Arc<CpuPixels>>,
}

fn wait_for_mapping(
    mapped: &Condvar,
    state: MutexGuard<'_, MappingState>,
    attempt: &mut Operation<'_>,
) -> Result<()> {
    let (state, _timeout) = mapped
        .wait_timeout(state, MAPPING_POLL_INTERVAL)
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    drop(state);
    // The operation context owns a caller-supplied clock, so consult it only
    // after releasing the mapping mutex.
    attempt.checkpoint()?;
    Ok(())
}

impl MacosFrameStorage {
    /// Wraps `frame` as the storage described by `descriptor`.
    pub(crate) fn new(descriptor: FrameDescriptor, frame: DetachedFrame) -> Arc<Self> {
        Arc::new(Self {
            descriptor,
            frame,
            mapping: Mutex::new(MappingState::default()),
            mapped: Condvar::new(),
        })
    }

    fn mapping(&self) -> MutexGuard<'_, MappingState> {
        self.mapping
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn convert(&self) -> std::result::Result<Arc<CpuPixels>, ShimStatus> {
        let lease = self.frame.reserve_cpu(self.descriptor.byte_len())?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(self.descriptor.byte_len())
            .map_err(|_| ShimStatus::BudgetExhausted)?;
        bytes.resize(self.descriptor.byte_len(), 0);
        self.frame.copy_out(&mut bytes, self.descriptor.stride())?;
        let pixels = match lease {
            Some(lease) => CpuPixels::with_retainer(bytes.into_boxed_slice(), lease),
            None => CpuPixels::new(bytes.into_boxed_slice()),
        };
        Ok(Arc::new(pixels))
    }
}

impl fmt::Debug for MacosFrameStorage {
    /// Formats the layout and mapping state, never the content.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.mapping();
        formatter
            .debug_struct("MacosFrameStorage")
            .field("descriptor", &self.descriptor)
            .field("mapping_active", &state.active)
            .field("mapped", &state.pixels.is_some())
            .finish()
    }
}

impl FrameStorage for MacosFrameStorage {
    fn descriptor(&self) -> FrameDescriptor {
        self.descriptor
    }

    fn reserve_cpu_copy(&self, bytes: usize) -> Result<Option<Arc<dyn Send + Sync>>> {
        self.frame
            .reserve_cpu(bytes)
            .map(|lease| lease.map(|lease| lease as Arc<dyn Send + Sync>))
            .map_err(Into::into)
    }

    fn cpu_pixels(&self) -> Option<Arc<CpuPixels>> {
        // Fixed for this storage's lifetime. Even once a conversion is cached,
        // native storage stays a conversion path rather than changing its answer
        // from None to Some, which a mapping would then read as shareable bytes.
        None
    }

    fn read_cpu(&self, operation: &OperationContext) -> Result<Arc<CpuPixels>> {
        let mut attempt = Operation::admit(operation)?;
        loop {
            let mut state = self.mapping();
            if let Some(pixels) = &state.pixels {
                return Ok(attempt.commit(Arc::clone(pixels))?);
            }
            if !state.active {
                // One conversion runs at a time; the rest wait for its result
                // rather than each copying the same buffer.
                state.active = true;
                drop(state);
                break;
            }
            wait_for_mapping(&self.mapped, state, &mut attempt)?;
        }

        let converted = self.convert();
        let mut result = match converted {
            // A conversion that finished after it was no longer allowed to commit
            // releases its bytes here rather than caching them: a late result may
            // not become the frame's mapping.
            Ok(pixels) => attempt.commit(pixels).map_err(Into::into),
            Err(status) => Err(status.into()),
        };
        {
            let mut state = self.mapping();
            state.active = false;
            if let Ok(pixels) = &result {
                state.pixels = Some(Arc::clone(pixels));
            }
            // Reading the cache back proves the value a later caller will see is
            // the one this caller is returning.
            if let Some(cached) = &state.pixels
                && let Ok(pixels) = &result
                && !Arc::ptr_eq(cached, pixels)
            {
                result = Ok(Arc::clone(cached));
            }
        }
        self.mapped.notify_all();
        result
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use mado_pilot_capture::{CaptureFault, FrameStorage, PixelFormat};
    use mado_pilot_core::{Clock, MonotonicInstant, Operation, OperationContext, PixelExtent};

    use super::{MappingState, descriptor_from_native, wait_for_mapping};
    use crate::shim::PIXEL_BGRA8;

    #[derive(Debug)]
    struct ReentrantMappingClock {
        mapping: Arc<Mutex<MappingState>>,
        reads: AtomicUsize,
    }

    impl Clock for ReentrantMappingClock {
        fn now(&self) -> MonotonicInstant {
            let mapping = self
                .mapping
                .try_lock()
                .expect("the caller clock must not run under the mapping mutex");
            drop(mapping);
            self.reads.fetch_add(1, Ordering::Relaxed);
            MonotonicInstant::ORIGIN
        }
    }

    #[test]
    fn a_mapping_wait_releases_its_mutex_before_reading_the_caller_clock() {
        let mapping = Arc::new(Mutex::new(MappingState {
            active: true,
            pixels: None,
        }));
        let clock = Arc::new(ReentrantMappingClock {
            mapping: Arc::clone(&mapping),
            reads: AtomicUsize::new(0),
        });
        let context = OperationContext::new()
            .with_clock(clock.clone())
            .with_deadline(MonotonicInstant::from_origin(Duration::from_secs(1)));
        let mut attempt = Operation::admit(&context).expect("mapping is admitted");
        clock.reads.store(0, Ordering::Relaxed);
        let state = mapping.lock().expect("mapping state is not poisoned");

        wait_for_mapping(&std::sync::Condvar::new(), state, &mut attempt)
            .expect("the unexpired mapping wait continues");

        assert_eq!(clock.reads.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_published_descriptor_carries_no_adapter_row_padding() {
        let descriptor = descriptor_from_native(PIXEL_BGRA8, PixelExtent::new(1710, 1112))
            .expect("bgra8 is the published layout");

        assert_eq!(descriptor.format(), PixelFormat::Bgra8);
        assert_eq!(descriptor.stride(), 1710 * 4);
        assert_eq!(descriptor.byte_len(), 1710 * 4 * 1112);
    }

    #[test]
    fn an_unpublished_pixel_layout_is_refused() {
        assert_eq!(
            descriptor_from_native(PIXEL_BGRA8 + 1, PixelExtent::new(8, 6)),
            Err(CaptureFault::UnsupportedFormat)
        );
    }

    #[test]
    fn padded_detached_and_cpu_storage_remain_charged_after_close_until_last_owner() {
        let _serial = crate::shim::NATIVE_LIFECYCLE_TEST_SERIAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (frame, budget) = crate::shim::testing_limited_frame(64, 104)
            .expect("64 padded native bytes plus 40 packed CPU bytes");
        assert_eq!(budget.used(), 64);
        let descriptor =
            descriptor_from_native(PIXEL_BGRA8, PixelExtent::new(5, 2)).expect("packed mapping");
        let storage = super::MacosFrameStorage::new(descriptor, frame);
        let pixels = storage
            .read_cpu(&OperationContext::new())
            .expect("inclusive ceiling");
        let shared = storage
            .read_cpu(&OperationContext::new())
            .expect("cached pixels");
        assert!(Arc::ptr_eq(&pixels, &shared));
        assert!(pixels.bytes().iter().copied().eq(0..40u8));
        assert_eq!(budget.used(), 104);
        drop(storage);
        assert_eq!(budget.used(), 40);
        drop(pixels);
        assert_eq!(budget.used(), 40);
        drop(shared);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn converted_and_cropped_mappings_share_the_native_retained_ceiling() {
        use mado_pilot_capture::{Frame, FrameView};
        use mado_pilot_core::{
            GeometryRevision, IdentityIssuer, PixelRect, StreamCursor, TransformSnapshot,
        };

        let _serial = crate::shim::NATIVE_LIFECYCLE_TEST_SERIAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (native, budget) = crate::shim::testing_limited_frame(64, 152)
            .expect("64 native + 40 cached + 40 converted + 8 cropped bytes");
        let extent = PixelExtent::new(5, 2);
        let descriptor = descriptor_from_native(PIXEL_BGRA8, extent).expect("descriptor");
        let storage = super::MacosFrameStorage::new(descriptor, native);
        let issuer = IdentityIssuer::new();
        let mut cursor = StreamCursor::new(issuer.issue_stream().expect("stream"));
        let stamp = cursor.publish(GeometryRevision::FIRST).expect("stamp");
        let frame = Frame::from_storage(
            stamp,
            MonotonicInstant::ORIGIN,
            TransformSnapshot::frame_only(stamp.geometry(), extent),
            storage,
        )
        .expect("frame");
        let operation = OperationContext::new();
        let converted = frame
            .map(PixelFormat::Rgba8, &operation)
            .expect("converted");
        assert_eq!(&converted.bytes()[..4], &[2, 1, 0, 3]);
        assert_eq!(budget.used(), 144);
        let region = PixelRect::new(0, 0, 1, 2).expect("region");
        let view = FrameView::new(frame.clone(), region).expect("view");
        let cropped = view
            .map(PixelFormat::Bgra8, &operation)
            .expect("exact ceiling");
        assert_eq!(cropped.bytes(), &[0, 1, 2, 3, 20, 21, 22, 23]);
        assert_eq!(budget.used(), 152);
        assert_eq!(
            view.map(PixelFormat::Bgra8, &operation)
                .expect_err("copy exceeds ceiling")
                .status(),
            mado_pilot_core::Status::LimitExceeded,
        );
        drop(cropped);
        let cropped = view
            .map(PixelFormat::Bgra8, &operation)
            .expect("released bytes reusable");
        drop(view);
        drop(frame);
        assert_eq!(budget.used(), 48);
        drop(converted);
        assert_eq!(budget.used(), 8);
        drop(cropped);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn native_padding_and_mapping_are_refused_before_exceeding_each_ceiling() {
        let _serial = crate::shim::NATIVE_LIFECYCLE_TEST_SERIAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(matches!(
            crate::shim::testing_limited_frame(63, 104),
            Err(crate::shim::ShimStatus::BudgetExhausted),
        ));
        let (frame, budget) =
            crate::shim::testing_limited_frame(64, 103).expect("the detached frame fits");
        let descriptor =
            descriptor_from_native(PIXEL_BGRA8, PixelExtent::new(5, 2)).expect("packed mapping");
        let storage = super::MacosFrameStorage::new(descriptor, frame);
        assert_eq!(
            storage
                .read_cpu(&OperationContext::new())
                .expect_err("mapping exceeds ceiling")
                .status(),
            mado_pilot_core::Error::from(CaptureFault::StorageBudgetExhausted).status(),
        );
        assert_eq!(budget.used(), 64);
        drop(storage);
        assert_eq!(budget.used(), 0);
    }
}
