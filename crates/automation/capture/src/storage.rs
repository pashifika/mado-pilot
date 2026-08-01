//! The immutable storage a published frame owns.
//!
//! A Phase 1 frame was always CPU bytes. A Windows frame is a GPU texture and a
//! macOS frame is a native buffer, and neither may appear in a platform-neutral
//! package, so a frame retains [`FrameStorage`] instead: an Adapter-facing
//! interface with exactly two questions on it, whether the pixels are already
//! CPU-readable and how to obtain them if they are not.
//!
//! # What this seam is not
//!
//! There is no downcast, no type tag, and no extension table. That is deliberate,
//! and it is the difference between deepening the frame's implementation and
//! publishing a native-frame interface: a caller that could ask "is this a D3D11
//! texture" would freeze D3D11 into backend-neutral code and preempt the deferred
//! `G-011` design. Callers map; the call is the same either way.
//!
//! # Ownership
//!
//! Storage is immutable, and an implementation that answers
//! [`FrameStorage::cpu_pixels`] once must answer identically for its whole
//! lifetime. Storage must also be independent of whatever produced it: retaining
//! a published frame may not retain a producer-pool slot whose reuse capture
//! needs to progress, which is why an Adapter detaches its own copy before it
//! publishes rather than publishing the producer's frame.

use std::fmt;
use std::sync::Arc;

use mado_pilot_core::{OperationContext, Result};

use crate::descriptor::FrameDescriptor;
use crate::fault::CaptureFault;

/// Immutable CPU pixel bytes, retainable on their own.
///
/// A mapping retains this rather than the frame it came from, so releasing the
/// frame, publishing a later one, or closing the session leaves mapped bytes
/// readable. Nothing hands out a mutable reference to the bytes after
/// construction.
pub struct CpuPixels {
    bytes: Box<[u8]>,
    _retained: Option<Arc<dyn Send + Sync>>,
}

impl CpuPixels {
    /// Takes ownership of `bytes` as immutable pixels.
    #[must_use]
    pub fn new(bytes: Box<[u8]>) -> Self {
        Self {
            bytes,
            _retained: None,
        }
    }

    /// Takes ownership of pixels and an ownership-only resource retainer.
    ///
    /// Native Adapters use this when mapped bytes must keep allocation accounting
    /// alive after their source frame and session are gone. The retainer is never
    /// exposed or interpreted by the capture contract.
    #[must_use]
    pub fn with_retainer(bytes: Box<[u8]>, retainer: Arc<dyn Send + Sync>) -> Self {
        Self {
            bytes,
            _retained: Some(retainer),
        }
    }

    /// Returns the bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns how many bytes are stored.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Reports whether the storage holds no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

impl fmt::Debug for CpuPixels {
    /// Formats the length, never the content.
    ///
    /// A derived `Debug` would print every byte of a captured frame into whatever
    /// log caught it.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CpuPixels")
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

/// The immutable storage behind one published frame.
///
/// Implemented by capture Adapters. The two operations are the whole seam: one
/// asks whether the pixels can be shared without conversion, the other performs
/// the conversion under the caller's operation context. Everything else about the
/// storage — a texture, a surface, a mapped buffer, a device, a lease — stays in
/// the Adapter that owns it.
///
/// # Contract
///
/// - [`FrameStorage::descriptor`] is fixed for the storage's lifetime and must
///   equal the descriptor its publication declares.
/// - [`FrameStorage::cpu_pixels`] either always returns `Some` or always returns
///   `None` for one storage value. Mapping decides whether to share bytes on that
///   answer, so a storage that changed it could hand a caller bytes that no longer
///   describe the frame.
/// - Neither operation may block on the producer, wait for a consumer, run a
///   backend, or invoke a host callback.
/// - The storage keeps alive everything its bytes need, including native device
///   resources, for as long as it exists.
pub trait FrameStorage: fmt::Debug + Send + Sync {
    /// Returns the layout of the stored pixels.
    fn descriptor(&self) -> FrameDescriptor;

    /// Returns the pixels when the storage already holds them on the CPU.
    ///
    /// `None` means a conversion is required, not that the storage is empty.
    fn cpu_pixels(&self) -> Option<Arc<CpuPixels>>;

    /// Converts the storage into CPU pixels.
    ///
    /// The result is owned by the caller and independent of the storage's
    /// producer. An implementation that already holds CPU pixels returns them
    /// without copying.
    ///
    /// # Errors
    ///
    /// Returns the operation's terminal outcome when cancellation or the deadline
    /// wins before the conversion commits, and a capture failure when the native
    /// conversion fails. A conversion that finishes after it is no longer allowed
    /// to commit releases its resources and reports the interruption.
    fn read_cpu(&self, operation: &OperationContext) -> Result<Arc<CpuPixels>>;
}

/// CPU bytes as storage, for sources whose frames are already pixels.
///
/// This is what the replay Adapter publishes and what a native Adapter's own CPU
/// fallback would publish. It exists here rather than in an Adapter because the
/// capture package needs it for the frames a caller builds directly.
#[derive(Debug)]
pub struct CpuFrameStorage {
    descriptor: FrameDescriptor,
    pixels: Arc<CpuPixels>,
}

impl CpuFrameStorage {
    /// Wraps `pixels` as storage described by `descriptor`.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureFault::ByteLengthMismatch`] when `pixels` is not exactly
    /// the length `descriptor` requires. Checking here rather than at publication
    /// means storage that exists is storage whose bytes and descriptor agree.
    pub fn new(
        descriptor: FrameDescriptor,
        pixels: Box<[u8]>,
    ) -> std::result::Result<Self, CaptureFault> {
        if pixels.len() != descriptor.byte_len() {
            return Err(CaptureFault::ByteLengthMismatch);
        }
        Ok(Self::from_validated(descriptor, pixels))
    }

    /// Wraps bytes whose length has already been checked against `descriptor`.
    ///
    /// Publication checks the length while the Adapter still owns the bytes, so
    /// that a refusal can hand them back. Re-checking here would add a failure
    /// path that cannot return them, so the check happens once, where recovery is
    /// still possible.
    pub(crate) fn from_validated(descriptor: FrameDescriptor, pixels: Box<[u8]>) -> Self {
        debug_assert_eq!(
            pixels.len(),
            descriptor.byte_len(),
            "storage bytes must match the descriptor the caller validated"
        );
        Self {
            descriptor,
            pixels: Arc::new(CpuPixels::new(pixels)),
        }
    }

    /// Returns the stored pixels.
    #[must_use]
    pub fn pixels(&self) -> Arc<CpuPixels> {
        Arc::clone(&self.pixels)
    }
}

impl FrameStorage for CpuFrameStorage {
    fn descriptor(&self) -> FrameDescriptor {
        self.descriptor
    }

    fn cpu_pixels(&self) -> Option<Arc<CpuPixels>> {
        Some(Arc::clone(&self.pixels))
    }

    fn read_cpu(&self, _operation: &OperationContext) -> Result<Arc<CpuPixels>> {
        // Already CPU bytes: there is nothing to convert, so there is nothing for
        // an interruption to arrive in the middle of. The operation context is
        // still checked by the mapping that called this, which is where the
        // caller's single terminal outcome is decided.
        Ok(Arc::clone(&self.pixels))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{CpuFrameStorage, CpuPixels, FrameStorage};
    use crate::descriptor::{FrameDescriptor, PixelFormat};
    use crate::fault::CaptureFault;
    use mado_pilot_core::{OperationContext, PixelExtent};

    fn descriptor(width: u32, height: u32) -> FrameDescriptor {
        FrameDescriptor::packed(PixelExtent::new(width, height), PixelFormat::Rgba8).expect("valid")
    }

    #[test]
    fn cpu_storage_shares_its_pixels_without_copying() {
        let descriptor = descriptor(4, 3);
        let storage = CpuFrameStorage::new(
            descriptor,
            vec![7; descriptor.byte_len()].into_boxed_slice(),
        )
        .expect("valid");

        let shared = storage.cpu_pixels().expect("already on the cpu");
        let read = storage
            .read_cpu(&OperationContext::new())
            .expect("nothing to convert");

        assert!(Arc::ptr_eq(&shared, &read));
        assert_eq!(shared.len(), descriptor.byte_len());
        assert!(shared.bytes().iter().all(|byte| *byte == 7));
    }

    #[test]
    fn storage_whose_bytes_disagree_with_its_descriptor_is_refused() {
        let descriptor = descriptor(4, 3);

        let error = CpuFrameStorage::new(
            descriptor,
            vec![0; descriptor.byte_len() - 1].into_boxed_slice(),
        )
        .expect_err("short bytes are refused");

        assert_eq!(error, CaptureFault::ByteLengthMismatch);
    }

    #[test]
    fn pixels_outlive_the_storage_that_held_them() {
        let descriptor = descriptor(2, 2);
        let storage = CpuFrameStorage::new(
            descriptor,
            vec![3; descriptor.byte_len()].into_boxed_slice(),
        )
        .expect("valid");
        let retained = storage.pixels();

        drop(storage);

        assert_eq!(retained.bytes(), [3; 16]);
    }

    #[test]
    fn debug_output_never_contains_pixel_content() {
        let pixels = CpuPixels::new(vec![17, 23, 91, 201].into_boxed_slice());

        let text = format!("{pixels:?}");

        assert!(text.contains("bytes: 4"), "{text}");
        assert!(
            !text.contains("17"),
            "captured pixels leaked into diagnostics: {text}"
        );
    }
}
