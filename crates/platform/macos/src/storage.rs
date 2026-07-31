//! Detached Core Video storage and its lazy CPU mapping.
//!
//! # Why the callback copies
//!
//! A ScreenCaptureKit surface belongs to a producer pool of fixed depth. Retaining
//! one until a consumer released the frame would let a retaining caller stall
//! capture, which the capture package's storage contract forbids. The producer
//! callback therefore copies the frame's content into an Adapter-owned buffer from
//! a finite pool and publishes that, so a retained public frame pins nothing the
//! producer needs.
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
        let mut bytes = vec![0u8; self.descriptor.byte_len()];
        self.frame.copy_out(&mut bytes, self.descriptor.stride())?;
        Ok(Arc::new(CpuPixels::new(bytes.into_boxed_slice())))
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
            let (_state, _timeout) = self
                .mapped
                .wait_timeout(state, MAPPING_POLL_INTERVAL)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            // The operation context is consulted with no lock held.
            attempt.checkpoint()?;
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
    use mado_pilot_capture::{CaptureFault, PixelFormat};
    use mado_pilot_core::PixelExtent;

    use super::{DETACHED_BUFFER_BUDGET, descriptor_from_native};
    use crate::shim::PIXEL_BGRA8;

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
    fn the_detached_budget_is_finite_and_small_enough_to_bound_memory() {
        assert!(
            DETACHED_BUFFER_BUDGET.get() >= 2,
            "a mapping and a publication overlap"
        );
        assert!(
            DETACHED_BUFFER_BUDGET.get() <= 16,
            "these are full-frame CPU allocations, not GPU textures"
        );
    }
}
