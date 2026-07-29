//! What a pixel buffer, a target, and a session are, before any of them exist.

use std::fmt;

use mado_pilot_core::{CoordinateSpace, PixelExtent, ProviderId, StreamId, TargetId};

use crate::fault::CaptureFault;

/// A CPU pixel layout MadoPilot can read and produce.
///
/// Both Phase 1 formats are eight bits per channel with a trailing alpha, and
/// differ only in channel order. Two rather than one, because Windows Graphics
/// Capture hands back BGRA while most image tooling speaks RGBA, and a contract
/// that pretended there was one order would push the swap into every adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PixelFormat {
    /// Red, green, blue, alpha, one byte each.
    Rgba8,
    /// Blue, green, red, alpha, one byte each.
    Bgra8,
}

impl PixelFormat {
    /// Returns the bytes one pixel occupies.
    #[must_use]
    pub const fn bytes_per_pixel(self) -> u32 {
        match self {
            PixelFormat::Rgba8 | PixelFormat::Bgra8 => 4,
        }
    }

    /// Returns a stable lowercase slug, for diagnostics and the C ABI mapping.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            PixelFormat::Rgba8 => "rgba8",
            PixelFormat::Bgra8 => "bgra8",
        }
    }

    /// Reports whether converting from `self` to `other` needs a channel swap.
    ///
    /// Every pair is matched, rather than listing the identity pairs and negating.
    /// The negation defaulted an unlisted pair to `true`, and on a
    /// `#[non_exhaustive]` enum the pairs a third variant `X` introduces are all
    /// unlisted — including `(X, X)`, which would have swapped bytes 0 and 2 of
    /// every pixel of an `X`-into-`X` mapping and reported success. Matching
    /// exhaustively makes that variant a compile error here instead, which is
    /// where the decision belongs.
    #[must_use]
    pub const fn needs_swap(self, other: Self) -> bool {
        match (self, other) {
            (PixelFormat::Rgba8, PixelFormat::Rgba8) | (PixelFormat::Bgra8, PixelFormat::Bgra8) => {
                false
            }
            (PixelFormat::Rgba8, PixelFormat::Bgra8) | (PixelFormat::Bgra8, PixelFormat::Rgba8) => {
                true
            }
        }
    }
}

impl fmt::Display for PixelFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The extent, format, and row stride of one pixel buffer.
///
/// Stride is carried separately from width because a capture source is free to
/// pad rows, and a consumer that assumed `width * bytes_per_pixel` would read
/// the padding as image data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameDescriptor {
    extent: PixelExtent,
    format: PixelFormat,
    stride: usize,
}

impl FrameDescriptor {
    /// Builds a descriptor and checks that its arithmetic closes.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureFault::InconsistentDescriptor`] for an empty extent, a
    /// stride shorter than one row, or an extent and stride whose product does
    /// not fit `usize`.
    pub fn new(
        extent: PixelExtent,
        format: PixelFormat,
        stride: usize,
    ) -> Result<Self, CaptureFault> {
        if extent.is_empty() {
            return Err(CaptureFault::InconsistentDescriptor);
        }
        let row = row_bytes(extent.width(), format)?;
        if stride < row {
            return Err(CaptureFault::InconsistentDescriptor);
        }
        // Reject here rather than at the allocation, so a descriptor a caller
        // holds can always be turned into a length without failing later.
        stride
            .checked_mul(
                usize::try_from(extent.height())
                    .map_err(|_| CaptureFault::InconsistentDescriptor)?,
            )
            .ok_or(CaptureFault::InconsistentDescriptor)?;
        Ok(Self {
            extent,
            format,
            stride,
        })
    }

    /// Builds a descriptor whose rows are packed with no padding.
    ///
    /// # Errors
    ///
    /// As [`FrameDescriptor::new`].
    pub fn packed(extent: PixelExtent, format: PixelFormat) -> Result<Self, CaptureFault> {
        let stride = row_bytes(extent.width(), format)?;
        Self::new(extent, format, stride)
    }

    /// Returns the extent in pixels.
    #[must_use]
    pub const fn extent(self) -> PixelExtent {
        self.extent
    }

    /// Returns the pixel format.
    #[must_use]
    pub const fn format(self) -> PixelFormat {
        self.format
    }

    /// Returns the bytes between the start of one row and the next.
    #[must_use]
    pub const fn stride(self) -> usize {
        self.stride
    }

    /// Returns the bytes one row of pixels occupies, excluding padding.
    #[must_use]
    pub fn row_bytes(self) -> usize {
        row_bytes(self.extent.width(), self.format).unwrap_or(0)
    }

    /// Returns the total bytes the buffer occupies, including padding.
    #[must_use]
    pub fn byte_len(self) -> usize {
        let height = usize::try_from(self.extent.height()).unwrap_or(0);
        self.stride.saturating_mul(height)
    }
}

impl fmt::Display for FrameDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} {} stride {}",
            self.extent, self.format, self.stride
        )
    }
}

fn row_bytes(width: u32, format: PixelFormat) -> Result<usize, CaptureFault> {
    let bytes = u64::from(width) * u64::from(format.bytes_per_pixel());
    usize::try_from(bytes).map_err(|_| CaptureFault::InconsistentDescriptor)
}

/// Which coordinate conversions a target's declared metadata can support.
///
/// Capture-pixel and frame-normalized conversion always work, so they are not
/// listed: a frame always knows its own extent. Everything here depends on
/// metadata the source may simply not have, and a caller is told up front rather
/// than discovering it from a failed conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CoordinateSupport {
    target_normalized: bool,
    target_logical: bool,
    desktop_logical: bool,
}

impl CoordinateSupport {
    /// Only the conversions every frame supports.
    #[must_use]
    pub const fn frame_only() -> Self {
        Self {
            target_normalized: false,
            target_logical: false,
            desktop_logical: false,
        }
    }

    /// Frame-local and target-normalized conversions, for a source that declares
    /// the target's content extent without logical or desktop placement.
    #[must_use]
    pub const fn with_target_extent() -> Self {
        Self {
            target_normalized: true,
            target_logical: false,
            desktop_logical: false,
        }
    }

    /// Every conversion, for a source that declares a full target placement.
    #[must_use]
    pub const fn with_target_placement() -> Self {
        Self {
            target_normalized: true,
            target_logical: true,
            desktop_logical: true,
        }
    }

    /// Reports whether `space` is convertible for this target.
    ///
    /// A coordinate space this build does not know about reports unsupported.
    /// Defaulting the other way would let a newer caller believe a conversion
    /// exists that an older library cannot perform.
    #[must_use]
    pub const fn supports(self, space: CoordinateSpace) -> bool {
        match space {
            CoordinateSpace::CapturePixels | CoordinateSpace::FrameNormalized => true,
            CoordinateSpace::TargetNormalized => self.target_normalized,
            CoordinateSpace::TargetLogical => self.target_logical,
            CoordinateSpace::DesktopLogical => self.desktop_logical,
            _ => false,
        }
    }
}

/// One discovered capture target, as its provider describes it.
///
/// `name` is descriptive only. It never establishes that two observations are
/// the same target; [`TargetId`] does that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetDescription {
    id: TargetId,
    name: String,
    extent: PixelExtent,
    format: PixelFormat,
    coordinates: CoordinateSupport,
}

impl TargetDescription {
    /// Describes a discovered target.
    #[must_use]
    pub fn new(
        id: TargetId,
        name: impl Into<String>,
        extent: PixelExtent,
        format: PixelFormat,
        coordinates: CoordinateSupport,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            extent,
            format,
            coordinates,
        }
    }

    /// Returns the target identity.
    #[must_use]
    pub const fn id(&self) -> TargetId {
        self.id
    }

    /// Returns the provider that discovered the target.
    #[must_use]
    pub const fn provider(&self) -> ProviderId {
        self.id.provider()
    }

    /// Returns the descriptive name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the declared content extent.
    #[must_use]
    pub const fn extent(&self) -> PixelExtent {
        self.extent
    }

    /// Returns the declared pixel format.
    #[must_use]
    pub const fn format(&self) -> PixelFormat {
        self.format
    }

    /// Returns the coordinate conversions the declared metadata supports.
    #[must_use]
    pub const fn coordinates(&self) -> CoordinateSupport {
        self.coordinates
    }
}

/// What a session actually accepted, which may differ from what was requested.
///
/// A caller reads this rather than assuming its preferences were honored: an
/// optional preference may fall back, and the accepted value is reported here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionDescription {
    target: TargetId,
    stream: StreamId,
    extent: PixelExtent,
    format: PixelFormat,
    coordinates: CoordinateSupport,
}

impl SessionDescription {
    /// Describes an opened session.
    #[must_use]
    pub const fn new(
        target: TargetId,
        stream: StreamId,
        extent: PixelExtent,
        format: PixelFormat,
        coordinates: CoordinateSupport,
    ) -> Self {
        Self {
            target,
            stream,
            extent,
            format,
            coordinates,
        }
    }

    /// Returns the target this session captures.
    #[must_use]
    pub const fn target(&self) -> TargetId {
        self.target
    }

    /// Returns the stream this session publishes to.
    #[must_use]
    pub const fn stream(&self) -> StreamId {
        self.stream
    }

    /// Returns the accepted content extent.
    #[must_use]
    pub const fn extent(&self) -> PixelExtent {
        self.extent
    }

    /// Returns the accepted pixel format.
    #[must_use]
    pub const fn format(&self) -> PixelFormat {
        self.format
    }

    /// Returns the coordinate conversions this session's frames support.
    #[must_use]
    pub const fn coordinates(&self) -> CoordinateSupport {
        self.coordinates
    }
}

#[cfg(test)]
mod tests {
    use super::{CoordinateSupport, FrameDescriptor, PixelFormat};
    use crate::fault::CaptureFault;
    use mado_pilot_core::{CoordinateSpace, PixelExtent};

    #[test]
    fn a_packed_descriptor_has_no_row_padding() {
        let descriptor =
            FrameDescriptor::packed(PixelExtent::new(64, 48), PixelFormat::Rgba8).expect("valid");

        assert_eq!(descriptor.row_bytes(), 256);
        assert_eq!(descriptor.stride(), 256);
        assert_eq!(descriptor.byte_len(), 256 * 48);
    }

    #[test]
    fn a_padded_stride_is_carried_rather_than_recomputed() {
        let descriptor =
            FrameDescriptor::new(PixelExtent::new(64, 48), PixelFormat::Bgra8, 320).expect("valid");

        assert_eq!(descriptor.row_bytes(), 256);
        assert_eq!(descriptor.stride(), 320);
        assert_eq!(descriptor.byte_len(), 320 * 48);
    }

    #[test]
    fn a_stride_shorter_than_one_row_is_refused() {
        assert_eq!(
            FrameDescriptor::new(PixelExtent::new(64, 48), PixelFormat::Rgba8, 255),
            Err(CaptureFault::InconsistentDescriptor)
        );
    }

    #[test]
    fn an_empty_extent_is_refused() {
        assert_eq!(
            FrameDescriptor::packed(PixelExtent::new(0, 48), PixelFormat::Rgba8),
            Err(CaptureFault::InconsistentDescriptor)
        );
        assert_eq!(
            FrameDescriptor::packed(PixelExtent::new(64, 0), PixelFormat::Rgba8),
            Err(CaptureFault::InconsistentDescriptor)
        );
    }

    #[test]
    fn an_extent_whose_byte_length_overflows_is_refused() {
        assert_eq!(
            FrameDescriptor::packed(PixelExtent::new(u32::MAX, u32::MAX), PixelFormat::Rgba8),
            Err(CaptureFault::InconsistentDescriptor)
        );
    }

    #[test]
    fn only_a_different_channel_order_needs_a_swap() {
        assert!(!PixelFormat::Rgba8.needs_swap(PixelFormat::Rgba8));
        assert!(!PixelFormat::Bgra8.needs_swap(PixelFormat::Bgra8));
        assert!(PixelFormat::Rgba8.needs_swap(PixelFormat::Bgra8));
        assert!(PixelFormat::Bgra8.needs_swap(PixelFormat::Rgba8));
    }

    #[test]
    fn frame_local_conversions_are_supported_without_target_metadata() {
        let support = CoordinateSupport::frame_only();

        assert!(support.supports(CoordinateSpace::CapturePixels));
        assert!(support.supports(CoordinateSpace::FrameNormalized));
        assert!(!support.supports(CoordinateSpace::TargetNormalized));
        assert!(!support.supports(CoordinateSpace::TargetLogical));
        assert!(!support.supports(CoordinateSpace::DesktopLogical));
    }

    #[test]
    fn a_declared_target_extent_supports_only_target_normalized_conversion() {
        let support = CoordinateSupport::with_target_extent();

        assert!(support.supports(CoordinateSpace::CapturePixels));
        assert!(support.supports(CoordinateSpace::FrameNormalized));
        assert!(support.supports(CoordinateSpace::TargetNormalized));
        assert!(!support.supports(CoordinateSpace::TargetLogical));
        assert!(!support.supports(CoordinateSpace::DesktopLogical));
    }

    #[test]
    fn a_declared_placement_supports_every_conversion() {
        let support = CoordinateSupport::with_target_placement();

        for space in [
            CoordinateSpace::CapturePixels,
            CoordinateSpace::FrameNormalized,
            CoordinateSpace::TargetNormalized,
            CoordinateSpace::TargetLogical,
            CoordinateSpace::DesktopLogical,
        ] {
            assert!(support.supports(space), "{space}");
        }
    }
}
