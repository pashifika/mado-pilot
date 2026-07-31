//! What a pixel buffer, a target, and a session are, before any of them exist.

use std::fmt;
use std::num::NonZeroU32;

use mado_pilot_core::{
    CoordinateSpace, InputCapability, PixelExtent, ProviderId, StreamId, TargetCapability, TargetId,
};

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
///
/// The capability is what a caller reads before opening anything: it says what
/// kind of desktop object the target is, whether capture may be attempted, which
/// input operation and delivery combinations the provider verified, and which
/// authorizations each of those ordinarily needs. A provider that describes none
/// of that reports [`TargetCapability::unclassified`], which is a capturable
/// target with no input rather than an absent answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetDescription {
    id: TargetId,
    name: String,
    extent: PixelExtent,
    format: PixelFormat,
    coordinates: CoordinateSupport,
    capability: TargetCapability,
}

impl TargetDescription {
    /// Describes a discovered target whose provider classifies nothing further.
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
            capability: TargetCapability::unclassified(),
        }
    }

    /// Declares what the provider can do with the target.
    #[must_use]
    pub fn with_capability(mut self, capability: TargetCapability) -> Self {
        self.capability = capability;
        self
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

    /// Returns what the provider can do with the target.
    #[must_use]
    pub const fn capability(&self) -> TargetCapability {
        self.capability
    }
}

/// What a session's finite publication path holds, and what it does when full.
///
/// A queue that grows with the producer's rate is how a slow consumer turns into
/// unbounded memory, so every capacity here is fixed when the session opens and
/// reported to the caller. The policy is part of the description rather than an
/// internal detail because it is what explains a dropped frame: a caller that
/// sees a sequence gap can tell whether the session was built to allow one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QueuePolicy {
    handoff: NonZeroU32,
    retained_storage: Option<NonZeroU32>,
    overflow: OverflowPolicy,
}

impl QueuePolicy {
    /// The policy of a session that publishes on the thread that produced the
    /// frame: one frame in flight, and nothing to supersede.
    #[must_use]
    pub const fn synchronous() -> Self {
        Self {
            handoff: NonZeroU32::MIN,
            retained_storage: None,
            overflow: OverflowPolicy::LatestWins,
        }
    }

    /// Declares a handoff capacity and what happens when it is full.
    #[must_use]
    pub const fn new(handoff: NonZeroU32, overflow: OverflowPolicy) -> Self {
        Self {
            handoff,
            retained_storage: None,
            overflow,
        }
    }

    /// Declares how many independently retained storage allocations the session
    /// can keep leased.
    #[must_use]
    pub const fn with_retained_storage(mut self, retained_storage: NonZeroU32) -> Self {
        self.retained_storage = Some(retained_storage);
        self
    }

    /// Returns how many frames may be in flight between producer and consumers.
    #[must_use]
    pub const fn handoff(self) -> NonZeroU32 {
        self.handoff
    }

    /// Returns the finite retained-storage capacity an Adapter declared.
    #[must_use]
    pub const fn retained_storage(self) -> Option<NonZeroU32> {
        self.retained_storage
    }

    /// Returns what the session does with a frame that does not fit.
    #[must_use]
    pub const fn overflow(self) -> OverflowPolicy {
        self.overflow
    }
}

impl fmt::Display for QueuePolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "handoff {} {}", self.handoff, self.overflow)
    }
}

/// What a session does with a frame its finite path cannot accept.
///
/// Either way the outcome is observable: dropping a frame silently would leave a
/// caller comparing frames across a gap it cannot see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OverflowPolicy {
    /// The newer frame supersedes the pending one, which is what automation
    /// wants: the current state of the screen is the useful one.
    LatestWins,
    /// The newer frame is refused and the pending one is kept.
    Reject,
}

impl OverflowPolicy {
    /// Returns a stable lowercase slug, for logs and the C ABI mapping.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            OverflowPolicy::LatestWins => "latest_wins",
            OverflowPolicy::Reject => "reject",
        }
    }
}

impl fmt::Display for OverflowPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// What a session actually accepted, which may differ from what was requested.
///
/// A caller reads this rather than assuming its preferences were honored: an
/// optional preference may fall back, and the accepted value is reported here.
/// That applies to input as much as to pixel format — a session opened with
/// optional input may open capture-only, and [`SessionDescription::input`] is
/// where a caller finds out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionDescription {
    target: TargetId,
    stream: StreamId,
    extent: PixelExtent,
    format: PixelFormat,
    coordinates: CoordinateSupport,
    input: InputCapability,
    queue: QueuePolicy,
}

impl SessionDescription {
    /// Describes an opened capture-only session with a synchronous queue.
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
            input: InputCapability::none(),
            queue: QueuePolicy::synchronous(),
        }
    }

    /// Declares the input capability this session actually established.
    #[must_use]
    pub const fn with_input(mut self, input: InputCapability) -> Self {
        self.input = input;
        self
    }

    /// Declares the finite capacities this session was built with.
    #[must_use]
    pub const fn with_queue(mut self, queue: QueuePolicy) -> Self {
        self.queue = queue;
        self
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

    /// Returns the input capability this session established.
    ///
    /// An empty capability means capture-only, which is a successful open rather
    /// than a degraded one whenever input was optional.
    #[must_use]
    pub const fn input(&self) -> InputCapability {
        self.input
    }

    /// Returns the finite capacities this session was built with.
    #[must_use]
    pub const fn queue(&self) -> QueuePolicy {
        self.queue
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use super::{
        CoordinateSupport, FrameDescriptor, OverflowPolicy, PixelFormat, QueuePolicy,
        SessionDescription, TargetDescription,
    };
    use crate::fault::CaptureFault;
    use mado_pilot_core::{
        CapabilitySupport, CoordinateSpace, IdentityIssuer, InputCapability, InputDelivery,
        InputOperationKind, PermissionKind, PixelExtent, ProviderId, TargetCapability, TargetKind,
    };

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
    fn a_target_description_reports_an_unclassified_capability_by_default() {
        let issuer = IdentityIssuer::new();
        let id = issuer
            .issue_target(ProviderId::new("replay"))
            .expect("issued");

        let description = TargetDescription::new(
            id,
            "sequence",
            PixelExtent::new(8, 6),
            PixelFormat::Rgba8,
            CoordinateSupport::with_target_extent(),
        );

        assert_eq!(description.capability().kind(), None);
        assert_eq!(
            description.capability().capture(),
            CapabilitySupport::Supported
        );
        assert!(!description.capability().input().is_available());
    }

    #[test]
    fn a_declared_capability_travels_with_the_target_description() {
        let issuer = IdentityIssuer::new();
        let id = issuer
            .issue_target(ProviderId::new("windows"))
            .expect("issued");
        let capability = TargetCapability::new(
            TargetKind::Window,
            CapabilitySupport::Supported,
            InputCapability::none()
                .with_pair(
                    InputOperationKind::Keyboard,
                    InputDelivery::BackgroundTarget,
                )
                .with_permission(PermissionKind::InputControl),
        );

        let description = TargetDescription::new(
            id,
            "Editor",
            PixelExtent::new(8, 6),
            PixelFormat::Bgra8,
            CoordinateSupport::with_target_placement(),
        )
        .with_capability(capability);

        assert_eq!(description.capability(), capability);
        assert_eq!(description.capability().kind(), Some(TargetKind::Window));
    }

    #[test]
    fn a_session_description_defaults_to_capture_only_and_a_synchronous_queue() {
        let issuer = IdentityIssuer::new();
        let target = issuer
            .issue_target(ProviderId::new("replay"))
            .expect("issued");
        let stream = issuer.issue_stream().expect("issued");

        let description = SessionDescription::new(
            target,
            stream,
            PixelExtent::new(8, 6),
            PixelFormat::Rgba8,
            CoordinateSupport::frame_only(),
        );

        assert!(!description.input().is_available());
        assert_eq!(description.queue(), QueuePolicy::synchronous());
        assert_eq!(description.queue().handoff().get(), 1);
        assert_eq!(description.queue().retained_storage(), None);
        assert_eq!(description.queue().overflow(), OverflowPolicy::LatestWins);
    }

    #[test]
    fn a_session_description_reports_the_input_and_capacities_it_accepted() {
        let issuer = IdentityIssuer::new();
        let target = issuer
            .issue_target(ProviderId::new("windows"))
            .expect("issued");
        let stream = issuer.issue_stream().expect("issued");
        let input = InputCapability::none()
            .with_pair(InputOperationKind::Pointer, InputDelivery::System)
            .with_focus_required(InputDelivery::System);
        let queue = QueuePolicy::new(
            NonZeroU32::new(2).expect("non-zero"),
            OverflowPolicy::LatestWins,
        );
        assert_eq!(queue.retained_storage(), None);
        let retained_storage = NonZeroU32::new(8).expect("non-zero");
        let queue = queue.with_retained_storage(retained_storage);

        let description = SessionDescription::new(
            target,
            stream,
            PixelExtent::new(8, 6),
            PixelFormat::Bgra8,
            CoordinateSupport::with_target_placement(),
        )
        .with_input(input)
        .with_queue(queue);

        assert_eq!(description.input(), input);
        assert_eq!(description.queue(), queue);
        assert_eq!(
            description.queue().retained_storage(),
            Some(retained_storage)
        );
        assert_eq!(queue.to_string(), "handoff 2 latest_wins");
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
