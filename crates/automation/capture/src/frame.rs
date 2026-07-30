//! Immutable published frames and the views taken over them.

use std::fmt;
use std::sync::Arc;

use mado_pilot_core::{
    ClipPolicy, FrameStamp, GeometryFault, MonotonicInstant, PixelRect, Rect, TransformSnapshot,
};

use crate::descriptor::FrameDescriptor;
use crate::fault::CaptureFault;

/// One published frame, immutable and independently retainable.
///
/// A frame outlives the session that published it. Retaining one is cheap — the
/// pixels are shared, not copied — and nothing that happens to the session
/// afterwards can change what a retained frame says: not a later publication,
/// not a geometry change, not close.
///
/// The pixel storage is deliberately private and has no public storage enum. A
/// Phase 1 frame is always CPU bytes, but Windows frames will later be GPU
/// textures, and a caller that had learned to match on storage would break. What
/// a caller gets instead is [`Frame::map`], which is the same call either way.
#[derive(Clone)]
pub struct Frame(Arc<FrameData>);

struct FrameData {
    stamp: FrameStamp,
    captured_at: MonotonicInstant,
    descriptor: FrameDescriptor,
    transform: TransformSnapshot,
    pixels: Box<[u8]>,
}

impl Frame {
    /// Publishes pixel bytes as an immutable frame.
    ///
    /// Adapters call this; callers receive the result. The bytes are moved in
    /// and never handed back out mutably.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureFault::ByteLengthMismatch`] when `pixels` is not exactly
    /// the length `descriptor` requires, and
    /// [`CaptureFault::InconsistentDescriptor`] when the descriptor's extent
    /// disagrees with the transform snapshot's frame extent — a frame whose
    /// pixels and geometry describe different rectangles would silently
    /// mislocate every match found in it.
    pub fn new(
        stamp: FrameStamp,
        captured_at: MonotonicInstant,
        descriptor: FrameDescriptor,
        transform: TransformSnapshot,
        pixels: Box<[u8]>,
    ) -> Result<Self, CaptureFault> {
        Self::validate(stamp, descriptor, &transform, pixels.len())?;
        Ok(Self::from_validated(
            stamp,
            captured_at,
            descriptor,
            transform,
            pixels,
        ))
    }

    /// Checks the invariants shared by direct and stream-owned construction.
    pub(crate) fn validate(
        stamp: FrameStamp,
        descriptor: FrameDescriptor,
        transform: &TransformSnapshot,
        pixel_len: usize,
    ) -> Result<(), CaptureFault> {
        if pixel_len != descriptor.byte_len() {
            return Err(CaptureFault::ByteLengthMismatch);
        }
        if transform.frame_extent() != descriptor.extent() {
            return Err(CaptureFault::InconsistentDescriptor);
        }
        if transform.geometry() != stamp.geometry() {
            return Err(CaptureFault::InconsistentDescriptor);
        }
        Ok(())
    }

    /// Builds a frame whose parts have already passed [`Frame::validate`].
    pub(crate) fn from_validated(
        stamp: FrameStamp,
        captured_at: MonotonicInstant,
        descriptor: FrameDescriptor,
        transform: TransformSnapshot,
        pixels: Box<[u8]>,
    ) -> Self {
        Self(Arc::new(FrameData {
            stamp,
            captured_at,
            descriptor,
            transform,
            pixels,
        }))
    }

    /// Returns the frame's complete identity.
    #[must_use]
    pub fn stamp(&self) -> FrameStamp {
        self.0.stamp
    }

    /// Returns when the frame was captured, in the engine's monotonic domain.
    #[must_use]
    pub fn captured_at(&self) -> MonotonicInstant {
        self.0.captured_at
    }

    /// Returns the extent, format, and stride of the frame's pixels.
    #[must_use]
    pub fn descriptor(&self) -> FrameDescriptor {
        self.0.descriptor
    }

    /// Returns the transform that was authoritative when the frame was captured.
    #[must_use]
    pub fn transform(&self) -> &TransformSnapshot {
        &self.0.transform
    }

    /// Returns the whole frame as a pixel rectangle.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureFault::InconsistentDescriptor`] when the extent does not
    /// fit the signed pixel range.
    pub fn bounds(&self) -> Result<PixelRect, CaptureFault> {
        self.0
            .descriptor
            .extent()
            .to_rect()
            .map_err(|_| CaptureFault::InconsistentDescriptor)
    }

    /// Takes a view over a region of this frame.
    ///
    /// The region is resolved against the frame's own transform snapshot, so a
    /// normalized or logical rectangle means what it meant at capture time.
    /// Nothing is copied.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureFault::UnsupportedCoordinate`] when the frame has no
    /// authoritative mapping for the region's coordinate space, and
    /// [`CaptureFault::RegionOutsideFrame`] when the geometry is otherwise invalid
    /// or outside the frame under `policy`.
    pub fn view(&self, region: Rect, policy: ClipPolicy) -> Result<FrameView, CaptureFault> {
        let resolved = self
            .0
            .transform
            .resolve_capture_pixels(region, policy)
            .map_err(|fault| match fault {
                GeometryFault::ConversionUnsupported => CaptureFault::UnsupportedCoordinate,
                _ => CaptureFault::RegionOutsideFrame,
            })?;
        FrameView::new(self.clone(), resolved)
    }

    /// Takes a view over the whole frame.
    ///
    /// # Errors
    ///
    /// As [`Frame::bounds`].
    pub fn full_view(&self) -> Result<FrameView, CaptureFault> {
        FrameView::new(self.clone(), self.bounds()?)
    }

    /// Returns the frame's raw bytes.
    ///
    /// This is the seam mapping reads through, and is not how a caller obtains
    /// pixels: a caller maps, which is the call that will still work when the
    /// storage is a GPU texture.
    pub(crate) fn pixels(&self) -> &[u8] {
        &self.0.pixels
    }
}

impl fmt::Debug for Frame {
    /// Formats identity and shape, never pixel content.
    ///
    /// Diagnostics must not carry captured images, and a derived `Debug` would
    /// print every byte of the frame into whatever log caught it.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Frame")
            .field("stamp", &self.0.stamp)
            .field("captured_at", &self.0.captured_at)
            .field("descriptor", &self.0.descriptor)
            .field("bytes", &self.0.pixels.len())
            .finish()
    }
}

/// A validated rectangular region of one exact frame.
///
/// A view retains its source frame, so releasing the frame does not invalidate
/// the view. It copies nothing: creating a view is a bounds check, and the
/// pixels are materialized only if the view is mapped.
#[derive(Clone)]
pub struct FrameView {
    frame: Frame,
    region: PixelRect,
}

impl FrameView {
    /// Builds a view over `region` of `frame`.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureFault::RegionOutsideFrame`] when the region is empty or
    /// not contained by the frame. An empty region is refused rather than
    /// reshaped, because an operation that needs pixels has nothing to work on.
    pub fn new(frame: Frame, region: PixelRect) -> Result<Self, CaptureFault> {
        let bounds = frame.bounds()?;
        if region.is_empty() || !bounds.contains_rect(region) {
            return Err(CaptureFault::RegionOutsideFrame);
        }
        Ok(Self { frame, region })
    }

    /// Returns the exact source frame.
    #[must_use]
    pub const fn frame(&self) -> &Frame {
        &self.frame
    }

    /// Returns the source frame's complete identity.
    #[must_use]
    pub fn stamp(&self) -> FrameStamp {
        self.frame.stamp()
    }

    /// Returns the validated region, in capture pixels.
    #[must_use]
    pub const fn region(&self) -> PixelRect {
        self.region
    }

    /// Returns the transform that was authoritative for the source frame.
    #[must_use]
    pub fn transform(&self) -> &TransformSnapshot {
        self.frame.transform()
    }
}

impl fmt::Debug for FrameView {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FrameView")
            .field("stamp", &self.frame.stamp())
            .field("region", &self.region)
            .finish()
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use mado_pilot_core::{
        FrameStamp, GeometryRevision, IdentityIssuer, MonotonicInstant, PixelExtent, StreamCursor,
        TransformSnapshot,
    };

    use super::Frame;
    use crate::descriptor::{FrameDescriptor, PixelFormat};

    /// Builds a frame whose bytes are a deterministic function of `seed`.
    pub(crate) fn frame(stamp: FrameStamp, width: u32, height: u32, seed: u8) -> Frame {
        let extent = PixelExtent::new(width, height);
        let descriptor = FrameDescriptor::packed(extent, PixelFormat::Rgba8).expect("valid");
        let pixels: Vec<u8> = (0..descriptor.byte_len())
            .map(|index| {
                u8::try_from(index % 251).expect("modulus keeps the value in range") ^ seed
            })
            .collect();
        Frame::new(
            stamp,
            MonotonicInstant::ORIGIN,
            descriptor,
            TransformSnapshot::frame_only(stamp.geometry(), extent),
            pixels.into_boxed_slice(),
        )
        .expect("valid")
    }

    /// Builds one frame from a fresh engine, for tests that need only pixels.
    pub(crate) fn any_frame(width: u32, height: u32, seed: u8) -> Frame {
        let issuer = IdentityIssuer::new();
        let mut cursor = StreamCursor::new(issuer.issue_stream().expect("issued"));
        let stamp = cursor.publish(GeometryRevision::FIRST).expect("published");
        frame(stamp, width, height, seed)
    }
}

#[cfg(test)]
mod tests {
    use super::{Frame, FrameView, testing};
    use crate::descriptor::{FrameDescriptor, PixelFormat};
    use crate::fault::CaptureFault;
    use mado_pilot_core::{
        ClipPolicy, CoordinateSpace, GeometryRevision, IdentityIssuer, MonotonicInstant,
        PixelExtent, PixelRect, Rect, StreamCursor, TransformSnapshot,
    };

    #[test]
    fn pixel_bytes_must_match_the_descriptor_exactly() {
        let issuer = IdentityIssuer::new();
        let mut cursor = StreamCursor::new(issuer.issue_stream().expect("issued"));
        let stamp = cursor.publish(GeometryRevision::FIRST).expect("published");
        let extent = PixelExtent::new(4, 4);
        let descriptor = FrameDescriptor::packed(extent, PixelFormat::Rgba8).expect("valid");

        let short = Frame::new(
            stamp,
            MonotonicInstant::ORIGIN,
            descriptor,
            TransformSnapshot::frame_only(stamp.geometry(), extent),
            vec![0; descriptor.byte_len() - 1].into_boxed_slice(),
        );

        assert_eq!(short.err(), Some(CaptureFault::ByteLengthMismatch));
    }

    #[test]
    fn a_frame_whose_geometry_disagrees_with_its_pixels_is_refused() {
        let issuer = IdentityIssuer::new();
        let mut cursor = StreamCursor::new(issuer.issue_stream().expect("issued"));
        let stamp = cursor.publish(GeometryRevision::FIRST).expect("published");
        let descriptor =
            FrameDescriptor::packed(PixelExtent::new(4, 4), PixelFormat::Rgba8).expect("valid");

        let mismatched = Frame::new(
            stamp,
            MonotonicInstant::ORIGIN,
            descriptor,
            // The transform describes a different rectangle than the pixels.
            TransformSnapshot::frame_only(stamp.geometry(), PixelExtent::new(8, 8)),
            vec![0; descriptor.byte_len()].into_boxed_slice(),
        );

        assert_eq!(
            mismatched.err(),
            Some(CaptureFault::InconsistentDescriptor),
            "geometry that disagrees with pixels mislocates every match"
        );
    }

    #[test]
    fn a_frame_whose_transform_revision_disagrees_with_its_stamp_is_refused() {
        let issuer = IdentityIssuer::new();
        let mut cursor = StreamCursor::new(issuer.issue_stream().expect("issued"));
        let stamp = cursor.publish(GeometryRevision::FIRST).expect("published");
        let extent = PixelExtent::new(4, 4);
        let descriptor = FrameDescriptor::packed(extent, PixelFormat::Rgba8).expect("valid");

        let mismatched = Frame::new(
            stamp,
            MonotonicInstant::ORIGIN,
            descriptor,
            TransformSnapshot::frame_only(
                GeometryRevision::FIRST.next().expect("representable"),
                extent,
            ),
            vec![0; descriptor.byte_len()].into_boxed_slice(),
        );

        assert_eq!(mismatched.err(), Some(CaptureFault::InconsistentDescriptor));
    }

    #[test]
    fn a_retained_frame_is_unchanged_by_anything_that_happens_later() {
        let frame = testing::any_frame(8, 6, 0x5A);
        let retained = frame.clone();
        let before: Vec<u8> = frame.pixels().to_vec();

        drop(frame);

        assert_eq!(retained.pixels(), before.as_slice());
        assert_eq!(retained.descriptor().extent(), PixelExtent::new(8, 6));
    }

    #[test]
    fn a_view_keeps_its_source_frame_alive() {
        let frame = testing::any_frame(8, 6, 0x11);
        let stamp = frame.stamp();
        let view = frame.full_view().expect("valid");

        drop(frame);

        assert_eq!(
            view.stamp(),
            stamp,
            "the view retains the exact source frame"
        );
        assert_eq!(view.region(), PixelRect::new(0, 0, 8, 6).expect("valid"));
    }

    #[test]
    fn a_view_resolves_its_region_through_the_frames_own_transform() {
        let frame = testing::any_frame(100, 50, 0x22);
        let half = Rect::new(CoordinateSpace::FrameNormalized, 0.0, 0.0, 0.5, 1.0).expect("valid");

        let view = frame.view(half, ClipPolicy::Reject).expect("inside");

        assert_eq!(view.region(), PixelRect::new(0, 0, 50, 50).expect("valid"));
    }

    #[test]
    fn an_empty_region_is_refused_rather_than_reshaped() {
        let frame = testing::any_frame(8, 6, 0x33);

        assert_eq!(
            FrameView::new(frame.clone(), PixelRect::new(2, 2, 2, 5).expect("valid")).err(),
            Some(CaptureFault::RegionOutsideFrame)
        );
    }

    #[test]
    fn a_region_outside_the_frame_is_refused() {
        let frame = testing::any_frame(8, 6, 0x44);

        assert_eq!(
            FrameView::new(frame, PixelRect::new(0, 0, 9, 6).expect("valid")).err(),
            Some(CaptureFault::RegionOutsideFrame)
        );
    }

    #[test]
    fn debug_output_never_contains_pixel_content() {
        let frame = testing::any_frame(4, 4, 0x7F);
        let text = format!("{frame:?}");

        assert!(text.contains("Frame"), "{text}");
        assert!(text.contains("bytes: 64"), "{text}");
        assert!(
            !text.contains(", 0,"),
            "a byte array leaked into diagnostics: {text}"
        );
    }
}
