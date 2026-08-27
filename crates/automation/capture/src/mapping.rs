//! CPU mappings: the point where a frame's pixels become readable bytes.

use std::fmt;
use std::sync::Arc;

use mado_pilot_core::{
    FrameStamp, Operation, OperationContext, PixelExtent, PixelRect, TransformSnapshot,
};

use crate::descriptor::{FrameDescriptor, PixelFormat};
use crate::fault::CaptureFault;
use crate::frame::{Frame, FrameView};
use crate::storage::CpuPixels;

/// A CPU-readable image owned by the caller.
///
/// A mapping is what makes the ownership contract observable: once it exists,
/// releasing the source frame, releasing the source view, publishing later
/// frames, and closing the session all leave it readable and unchanged. It
/// either retains the frame it came from or owns an independent allocation, and
/// which of the two is an implementation detail a caller never has to reason
/// about.
///
/// The byte view borrows from the mapping, so Rust's own lifetimes enforce the
/// rule the C ABI will have to state in prose: a borrowed view dies with its
/// owner.
#[derive(Clone)]
pub struct CpuMapping {
    stamp: FrameStamp,
    transform: TransformSnapshot,
    region: PixelRect,
    descriptor: FrameDescriptor,
    storage: MappingStorage,
}

#[derive(Clone)]
enum MappingStorage {
    /// The frame's own CPU pixels, already in the requested layout: share them.
    ///
    /// The pixels are retained directly rather than through the frame, so a
    /// mapping holds exactly what its bytes need and nothing else. That matters
    /// for a native frame: a mapping that retained the frame would keep its
    /// Adapter-owned storage — and whatever lease that storage holds — alive for
    /// as long as the caller kept the mapped bytes.
    Shared(Arc<CpuPixels>),
    /// Pixels this mapping obtained or built for itself, by conversion from
    /// native storage, by cropping, or by a channel swap.
    Owned(Arc<CpuPixels>),
}

impl CpuMapping {
    /// Returns the complete identity of the frame these pixels came from.
    ///
    /// A mapping taken from an exact retained frame reports that frame, not
    /// whatever the session published since.
    #[must_use]
    pub const fn stamp(&self) -> FrameStamp {
        self.stamp
    }

    /// Returns the transform that was authoritative for the source frame.
    #[must_use]
    pub const fn transform(&self) -> &TransformSnapshot {
        &self.transform
    }

    /// Returns the source-frame region these pixels cover, in capture pixels.
    #[must_use]
    pub const fn region(&self) -> PixelRect {
        self.region
    }

    /// Returns the mapped image's extent, format, and stride.
    #[must_use]
    pub const fn descriptor(&self) -> FrameDescriptor {
        self.descriptor
    }

    /// Returns the mapped bytes.
    ///
    /// Valid for as long as the mapping is.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        match &self.storage {
            MappingStorage::Shared(pixels) | MappingStorage::Owned(pixels) => pixels.bytes(),
        }
    }

    /// Reports whether the mapping shares the source frame's own CPU pixels.
    ///
    /// Exposed for tests and diagnostics that assert the zero-copy path is
    /// actually taken. Behavior does not depend on it. A mapping of native storage
    /// is never shared, because obtaining CPU bytes from it is a copy however
    /// little else the mapping had to do.
    #[must_use]
    pub const fn is_shared(&self) -> bool {
        matches!(self.storage, MappingStorage::Shared(_))
    }
}

impl fmt::Debug for CpuMapping {
    /// Formats identity and shape, never pixel content.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CpuMapping")
            .field("stamp", &self.stamp)
            .field("region", &self.region)
            .field("descriptor", &self.descriptor)
            .field("shared", &self.is_shared())
            .finish()
    }
}

impl Frame {
    /// Maps the whole frame into `format`.
    ///
    /// # Errors
    ///
    /// Returns the operation's terminal outcome when cancellation or the
    /// deadline wins before the mapping commits, and a capture fault when the
    /// frame's descriptor cannot be honored.
    pub fn map(
        &self,
        format: PixelFormat,
        operation: &OperationContext,
    ) -> mado_pilot_core::Result<CpuMapping> {
        let bounds = self.bounds()?;
        map_region(self, bounds, format, operation)
    }
}

impl FrameView {
    /// Maps this view's region into `format`.
    ///
    /// The result covers only the view's region and reports the same complete
    /// source-frame identity as the view.
    ///
    /// # Errors
    ///
    /// As [`Frame::map`].
    pub fn map(
        &self,
        format: PixelFormat,
        operation: &OperationContext,
    ) -> mado_pilot_core::Result<CpuMapping> {
        map_region(self.frame(), self.region(), format, operation)
    }
}

/// Produces a mapping of `region` of `frame` in `format`.
///
/// The operation context is checked before the copy is admitted and again before
/// the result commits, so a mapping that loses the race is discarded whole
/// rather than handed back half-filled.
fn map_region(
    frame: &Frame,
    region: PixelRect,
    format: PixelFormat,
    operation: &OperationContext,
) -> mado_pilot_core::Result<CpuMapping> {
    let mut attempt = Operation::admit(operation)?;

    let source = frame.descriptor();
    let bounds = frame.bounds()?;
    if region.is_empty() || !bounds.contains_rect(region) {
        return Err(CaptureFault::RegionOutsideFrame.into());
    }

    // Nothing to crop and nothing to swap means the storage's own CPU bytes are
    // already the mapping's bytes, whether they were there to begin with or a
    // conversion produced them.
    let unconverted = region == bounds && source.format() == format;
    let (pixels, shared) = match frame.storage().cpu_pixels() {
        Some(pixels) => (pixels, true),
        None => {
            let converted = frame.storage().read_cpu(operation)?;
            // The conversion is the expensive part of mapping native storage, and
            // an interruption that arrived during it must discard the result
            // rather than let it commit.
            attempt.checkpoint()?;
            (converted, false)
        }
    };

    let mapping = if unconverted {
        CpuMapping {
            stamp: frame.stamp(),
            transform: *frame.transform(),
            region,
            descriptor: source,
            storage: if shared {
                MappingStorage::Shared(pixels)
            } else {
                MappingStorage::Owned(pixels)
            },
        }
    } else {
        let (descriptor, bytes) = copy_region(source, pixels.bytes(), region, format)?;
        attempt.checkpoint()?;
        CpuMapping {
            stamp: frame.stamp(),
            transform: *frame.transform(),
            region,
            descriptor,
            storage: MappingStorage::Owned(Arc::new(CpuPixels::new(bytes))),
        }
    };

    Ok(attempt.commit(mapping)?)
}

/// Copies `region` out of `pixels`, swapping channels when the format differs.
///
/// The result is packed: a mapping the caller owns has no reason to carry the
/// source's row padding, and a packed stride is the one a consumer can predict.
fn copy_region(
    source: FrameDescriptor,
    pixels: &[u8],
    region: PixelRect,
    format: PixelFormat,
) -> Result<(FrameDescriptor, Box<[u8]>), CaptureFault> {
    let bytes_per_pixel = usize::try_from(format.bytes_per_pixel())
        .map_err(|_| CaptureFault::InconsistentDescriptor)?;
    if source.format().bytes_per_pixel() != format.bytes_per_pixel() {
        return Err(CaptureFault::UnsupportedFormat);
    }

    let extent = PixelExtent::new(region.width(), region.height());
    let descriptor = FrameDescriptor::packed(extent, format)?;
    let left = usize::try_from(region.left()).map_err(|_| CaptureFault::RegionOutsideFrame)?;
    let top = usize::try_from(region.top()).map_err(|_| CaptureFault::RegionOutsideFrame)?;
    let row_bytes = descriptor.row_bytes();

    let mut output = vec![0u8; descriptor.byte_len()];
    // Every offset below is checked, including the ones a 64-bit target cannot
    // overflow. The bounds that make them safe today — `left` and `top` below
    // `i32::MAX`, four bytes per pixel — are properties of this build's pixel
    // formats and pointer width rather than of this loop, so a `+` here would be
    // a silent dependency on both. A wrapped offset would land inside the buffer
    // and copy the wrong pixels rather than fail.
    let column_bytes = left
        .checked_mul(bytes_per_pixel)
        .ok_or(CaptureFault::InconsistentDescriptor)?;
    for row in 0..usize::try_from(extent.height()).map_err(|_| CaptureFault::RegionOutsideFrame)? {
        let source_start = top
            .checked_add(row)
            .and_then(|line| line.checked_mul(source.stride()))
            .and_then(|offset| offset.checked_add(column_bytes))
            .ok_or(CaptureFault::InconsistentDescriptor)?;
        let source_end = source_start
            .checked_add(row_bytes)
            .ok_or(CaptureFault::InconsistentDescriptor)?;
        let source_row = pixels
            .get(source_start..source_end)
            .ok_or(CaptureFault::ByteLengthMismatch)?;
        let target_start = row
            .checked_mul(row_bytes)
            .ok_or(CaptureFault::InconsistentDescriptor)?;
        let target_end = target_start
            .checked_add(row_bytes)
            .ok_or(CaptureFault::InconsistentDescriptor)?;
        let target_row = output
            .get_mut(target_start..target_end)
            .ok_or(CaptureFault::ByteLengthMismatch)?;
        target_row.copy_from_slice(source_row);
    }

    if source.format().needs_swap(format) {
        swap_red_and_blue(&mut output);
    }

    Ok((descriptor, output.into_boxed_slice()))
}

/// Exchanges the first and third byte of every four-byte pixel.
///
/// This is the whole difference between the two Phase 1 formats. Alpha and the
/// middle channel are untouched.
fn swap_red_and_blue(pixels: &mut [u8]) {
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use mado_pilot_core::{
        CancellationToken, Clock, MonotonicInstant, OperationContext, PixelExtent, PixelRect,
        Status,
    };

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

    use crate::descriptor::PixelFormat;
    use crate::frame::{FrameView, testing};

    #[test]
    fn a_full_frame_mapping_in_the_source_format_shares_storage() {
        let frame = testing::any_frame(8, 6, 0x10);
        let context = OperationContext::new();

        let mapping = frame.map(PixelFormat::Rgba8, &context).expect("mapped");

        assert!(mapping.is_shared(), "no crop and no swap means no copy");
        assert_eq!(mapping.stamp(), frame.stamp());
        assert_eq!(mapping.descriptor(), frame.descriptor());
        assert_eq!(mapping.bytes().len(), frame.descriptor().byte_len());
    }

    #[test]
    fn a_mapping_outlives_every_source_handle() {
        let frame = testing::any_frame(8, 6, 0x20);
        let stamp = frame.stamp();
        let view = frame.full_view().expect("valid");
        let context = OperationContext::new();
        let mapping = view.map(PixelFormat::Rgba8, &context).expect("mapped");
        let expected: Vec<u8> = mapping.bytes().to_vec();

        drop(view);
        drop(frame);

        assert_eq!(mapping.bytes(), expected.as_slice());
        assert_eq!(mapping.stamp(), stamp);
    }

    #[test]
    fn a_region_mapping_contains_only_that_region() {
        let frame = testing::any_frame(8, 6, 0x30);
        let region = PixelRect::new(2, 1, 6, 4).expect("valid");
        let view = FrameView::new(frame.clone(), region).expect("valid");
        let context = OperationContext::new();

        let mapping = view.map(PixelFormat::Rgba8, &context).expect("mapped");

        assert!(!mapping.is_shared());
        assert_eq!(mapping.region(), region);
        assert_eq!(mapping.descriptor().extent(), PixelExtent::new(4, 3));
        assert_eq!(mapping.bytes().len(), 4 * 4 * 3);

        // Row zero of the mapping is row one of the frame, starting at pixel two.
        let source = frame.descriptor();
        let start = source.stride() + 2 * 4;
        assert_eq!(
            &mapping.bytes()[..16],
            &frame
                .map(PixelFormat::Rgba8, &context)
                .expect("mapped")
                .bytes()[start..start + 16]
        );
    }

    #[test]
    fn a_format_change_swaps_red_and_blue_and_leaves_alpha_alone() {
        let frame = testing::any_frame(4, 2, 0x00);
        let context = OperationContext::new();
        let source = frame.map(PixelFormat::Rgba8, &context).expect("mapped");
        let expected: Vec<u8> = source.bytes().to_vec();

        let swapped = frame.map(PixelFormat::Bgra8, &context).expect("mapped");

        assert!(!swapped.is_shared(), "a channel swap needs its own buffer");
        assert_eq!(swapped.descriptor().format(), PixelFormat::Bgra8);
        for (index, pixel) in swapped.bytes().chunks_exact(4).enumerate() {
            let original = &expected[index * 4..index * 4 + 4];
            assert_eq!(pixel[0], original[2], "pixel {index} blue");
            assert_eq!(pixel[1], original[1], "pixel {index} green");
            assert_eq!(pixel[2], original[0], "pixel {index} red");
            assert_eq!(pixel[3], original[3], "pixel {index} alpha");
        }
    }

    #[test]
    fn swapping_twice_returns_the_original_bytes() {
        let frame = testing::any_frame(4, 2, 0x60);
        let context = OperationContext::new();
        let original: Vec<u8> = frame
            .map(PixelFormat::Rgba8, &context)
            .expect("m")
            .bytes()
            .to_vec();

        let mut swapped: Vec<u8> = frame
            .map(PixelFormat::Bgra8, &context)
            .expect("mapped")
            .bytes()
            .to_vec();
        super::swap_red_and_blue(&mut swapped);

        assert_eq!(swapped, original);
    }

    #[test]
    fn a_cancelled_mapping_exposes_no_partial_result() {
        let frame = testing::any_frame(8, 6, 0x40);
        let token = CancellationToken::new();
        token.cancel();
        let context = OperationContext::new().with_cancellation(token);

        let error = frame
            .map(PixelFormat::Rgba8, &context)
            .expect_err("cancelled before admission");

        assert_eq!(error.status(), Status::Cancelled);
    }

    #[test]
    fn deadline_after_the_copy_still_discards_the_mapping() {
        let frame = testing::any_frame(8, 6, 0x50);
        let clock = Arc::new(AdvanceAfterAdmission::default());
        let deadline = MonotonicInstant::ORIGIN
            .checked_add(Duration::from_millis(1))
            .expect("test deadline is representable");
        let context = OperationContext::new()
            .with_clock(clock)
            .with_deadline(deadline);
        let region = PixelRect::new(0, 0, 4, 3).expect("valid");
        let view = FrameView::new(frame, region).expect("valid");

        let error = view
            .map(PixelFormat::Bgra8, &context)
            .expect_err("deadline wins after the copy and before commit");

        assert_eq!(error.status(), Status::DeadlineExceeded);
    }

    #[test]
    fn a_padded_source_row_is_not_copied_into_the_mapping() {
        use crate::descriptor::FrameDescriptor;
        use crate::frame::Frame;
        use mado_pilot_core::{
            GeometryRevision, IdentityIssuer, MonotonicInstant, StreamCursor, TransformSnapshot,
        };

        let issuer = IdentityIssuer::new();
        let mut cursor = StreamCursor::new(issuer.issue_stream().expect("issued"));
        let stamp = cursor.publish(GeometryRevision::FIRST).expect("published");
        let extent = PixelExtent::new(2, 2);
        // Sixteen bytes of pixels per row plus sixteen bytes of padding.
        let descriptor = FrameDescriptor::new(extent, PixelFormat::Rgba8, 24).expect("valid");
        let mut pixels = vec![0xFFu8; descriptor.byte_len()];
        for row in 0..2usize {
            for byte in 0..8usize {
                pixels[row * 24 + byte] = u8::try_from(row * 8 + byte).expect("small");
            }
        }
        let frame = Frame::new(
            stamp,
            MonotonicInstant::ORIGIN,
            descriptor,
            TransformSnapshot::frame_only(stamp.geometry(), extent),
            pixels.into_boxed_slice(),
        )
        .expect("valid");
        let context = OperationContext::new();

        // Mapping the whole frame in its own format shares the source buffer,
        // padding included. The descriptor reports the source stride, which is
        // exactly why stride is part of the descriptor rather than derived from
        // the width.
        let shared = frame.map(PixelFormat::Rgba8, &context).expect("mapped");
        assert!(shared.is_shared());
        assert_eq!(shared.descriptor().stride(), 24);
        assert_eq!(shared.bytes().len(), 48);

        // A region mapping owns its buffer and is packed, so the source padding
        // is left behind rather than read as image data.
        let region = PixelRect::new(0, 0, 1, 2).expect("valid");
        let cropped = FrameView::new(frame, region)
            .expect("valid")
            .map(PixelFormat::Rgba8, &context)
            .expect("mapped");

        assert!(!cropped.is_shared());
        assert_eq!(cropped.descriptor().stride(), 4);
        assert_eq!(cropped.bytes(), &[0, 1, 2, 3, 8, 9, 10, 11]);
    }
}
