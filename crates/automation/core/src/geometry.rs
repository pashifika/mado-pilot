//! Coordinate spaces, validated geometry, and half-open rectangles.
//!
//! Every public point and rectangle names the coordinate space it is expressed
//! in. The space is carried at run time rather than in the type, because the C
//! ABI has to carry a tag anyway and a caller must be able to submit a space
//! that turns out to be unsupported — a compile-time-only encoding could not
//! express the request that has to be rejected.
//!
//! Public coordinates always use a top-left origin with x increasing to the
//! right and y increasing downward, on both release targets, whatever the host
//! convention is.

use std::fmt;

use crate::status::{Error, Status};

/// The coordinate space a point or rectangle is expressed in.
///
/// This enum is `#[non_exhaustive]`: later phases add spaces, and a caller must
/// keep a fallback arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CoordinateSpace {
    /// Discrete pixels of the captured frame, origin at the frame's top-left.
    CapturePixels,
    /// The captured frame's extent mapped onto `0.0..=1.0` on both axes.
    FrameNormalized,
    /// The target's extent mapped onto `0.0..=1.0` on both axes.
    TargetNormalized,
    /// The target's own logical units, origin at the target's top-left.
    TargetLogical,
    /// Desktop logical units, origin at the desktop's top-left.
    DesktopLogical,
}

impl CoordinateSpace {
    /// Reports whether values in this space are confined to `0.0..=1.0`.
    #[must_use]
    pub const fn is_normalized(self) -> bool {
        matches!(
            self,
            CoordinateSpace::FrameNormalized | CoordinateSpace::TargetNormalized
        )
    }

    /// Returns a stable lowercase slug, for diagnostics and the C ABI mapping.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            CoordinateSpace::CapturePixels => "capture_pixels",
            CoordinateSpace::FrameNormalized => "frame_normalized",
            CoordinateSpace::TargetNormalized => "target_normalized",
            CoordinateSpace::TargetLogical => "target_logical",
            CoordinateSpace::DesktopLogical => "desktop_logical",
        }
    }
}

impl fmt::Display for CoordinateSpace {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A geometry rule that could not be satisfied.
///
/// This is a typed fault rather than a message so that a test — and a caller —
/// can assert which rule was broken. It converts into the public
/// [`Error`] at the package boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GeometryFault {
    /// A coordinate or size was NaN or infinite.
    NotFinite,
    /// A rectangle's right edge was left of its left edge, or its bottom edge
    /// above its top edge.
    NegativeSize,
    /// A normalized value fell outside `0.0..=1.0`.
    OutOfNormalizedRange,
    /// An operation that needs pixel content received an empty region.
    EmptyRegion,
    /// Two geometry values that had to share a coordinate space did not.
    SpaceMismatch,
    /// The selected transform snapshot has no authoritative mapping for the
    /// requested conversion.
    ConversionUnsupported,
    /// A region fell outside the destination extent and clipping was not
    /// permitted.
    OutsideExtent,
    /// A converted coordinate does not fit the destination's integer range.
    ExtentOverflow,
}

impl GeometryFault {
    /// Returns the public status this fault reports as.
    #[must_use]
    pub const fn status(self) -> Status {
        match self {
            GeometryFault::NotFinite
            | GeometryFault::NegativeSize
            | GeometryFault::OutOfNormalizedRange
            | GeometryFault::EmptyRegion
            | GeometryFault::SpaceMismatch
            | GeometryFault::OutsideExtent
            | GeometryFault::ExtentOverflow => Status::InvalidArgument,
            GeometryFault::ConversionUnsupported => Status::Unsupported,
        }
    }

    const fn detail(self) -> &'static str {
        match self {
            GeometryFault::NotFinite => "geometry contains a non-finite value",
            GeometryFault::NegativeSize => "rectangle edges are inverted",
            GeometryFault::OutOfNormalizedRange => {
                "normalized geometry falls outside zero through one"
            }
            GeometryFault::EmptyRegion => "operation requires a non-empty region",
            GeometryFault::SpaceMismatch => "geometry values use different coordinate spaces",
            GeometryFault::ConversionUnsupported => {
                "no authoritative transform represents the requested conversion"
            }
            GeometryFault::OutsideExtent => {
                "region falls outside the destination extent and clipping was not permitted"
            }
            GeometryFault::ExtentOverflow => "converted geometry exceeds the destination range",
        }
    }
}

impl fmt::Display for GeometryFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.detail())
    }
}

impl std::error::Error for GeometryFault {}

impl From<GeometryFault> for Error {
    fn from(fault: GeometryFault) -> Self {
        Error::new(fault.status(), fault.detail())
    }
}

/// What to do when a converted region does not fit the destination extent.
///
/// The default rejects. An operation only clips when the caller says so, because
/// silently clipping a search region changes the question that was asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ClipPolicy {
    /// Fail with [`GeometryFault::OutsideExtent`] when any part falls outside.
    #[default]
    Reject,
    /// Keep the overlapping part, failing only when nothing overlaps.
    Clip,
}

/// A point in a named coordinate space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    space: CoordinateSpace,
    x: f64,
    y: f64,
}

impl Point {
    /// Builds a validated point.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryFault::NotFinite`] for NaN or infinity, and
    /// [`GeometryFault::OutOfNormalizedRange`] when a normalized space receives a
    /// value outside `0.0..=1.0`.
    pub fn new(space: CoordinateSpace, x: f64, y: f64) -> Result<Self, GeometryFault> {
        if !x.is_finite() || !y.is_finite() {
            return Err(GeometryFault::NotFinite);
        }
        if space.is_normalized() && !(in_unit_range(x) && in_unit_range(y)) {
            return Err(GeometryFault::OutOfNormalizedRange);
        }
        Ok(Self { space, x, y })
    }

    /// Returns the coordinate space.
    #[must_use]
    pub const fn space(self) -> CoordinateSpace {
        self.space
    }

    /// Returns the horizontal coordinate.
    #[must_use]
    pub const fn x(self) -> f64 {
        self.x
    }

    /// Returns the vertical coordinate.
    #[must_use]
    pub const fn y(self) -> f64 {
        self.y
    }
}

impl fmt::Display for Point {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "({}, {}) {}", self.x, self.y, self.space)
    }
}

/// A half-open rectangle in a named coordinate space.
///
/// Bounds are `[left, right) × [top, bottom)`: the left and top edges are inside
/// the rectangle and the right and bottom edges are outside it. Adjacent
/// rectangles therefore tile without overlapping and without a gap, and a
/// rectangle's width is exactly `right - left`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    space: CoordinateSpace,
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

impl Rect {
    /// Builds a validated half-open rectangle from its edges.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryFault::NotFinite`] for a NaN or infinite edge,
    /// [`GeometryFault::NegativeSize`] when an edge pair is inverted, and
    /// [`GeometryFault::OutOfNormalizedRange`] when a normalized space receives an
    /// edge outside `0.0..=1.0`. A normalized rectangle is never clamped into
    /// range: clamping would answer a different question than the caller asked.
    pub fn new(
        space: CoordinateSpace,
        left: f64,
        top: f64,
        right: f64,
        bottom: f64,
    ) -> Result<Self, GeometryFault> {
        if !(left.is_finite() && top.is_finite() && right.is_finite() && bottom.is_finite()) {
            return Err(GeometryFault::NotFinite);
        }
        if right < left || bottom < top {
            return Err(GeometryFault::NegativeSize);
        }
        if space.is_normalized()
            && !(in_unit_range(left)
                && in_unit_range(top)
                && in_unit_range(right)
                && in_unit_range(bottom))
        {
            return Err(GeometryFault::OutOfNormalizedRange);
        }
        Ok(Self {
            space,
            left,
            top,
            right,
            bottom,
        })
    }

    /// Builds a validated rectangle from an origin and a size.
    ///
    /// # Errors
    ///
    /// As [`Rect::new`]. A negative width or height is reported as
    /// [`GeometryFault::NegativeSize`].
    pub fn from_origin_size(
        space: CoordinateSpace,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> Result<Self, GeometryFault> {
        if !(x.is_finite() && y.is_finite() && width.is_finite() && height.is_finite()) {
            return Err(GeometryFault::NotFinite);
        }
        if width < 0.0 || height < 0.0 {
            return Err(GeometryFault::NegativeSize);
        }
        Self::new(space, x, y, x + width, y + height)
    }

    /// Returns the coordinate space.
    #[must_use]
    pub const fn space(self) -> CoordinateSpace {
        self.space
    }

    /// Returns the inclusive left edge.
    #[must_use]
    pub const fn left(self) -> f64 {
        self.left
    }

    /// Returns the inclusive top edge.
    #[must_use]
    pub const fn top(self) -> f64 {
        self.top
    }

    /// Returns the exclusive right edge.
    #[must_use]
    pub const fn right(self) -> f64 {
        self.right
    }

    /// Returns the exclusive bottom edge.
    #[must_use]
    pub const fn bottom(self) -> f64 {
        self.bottom
    }

    /// Returns `right - left`.
    #[must_use]
    pub fn width(self) -> f64 {
        self.right - self.left
    }

    /// Returns `bottom - top`.
    #[must_use]
    pub fn height(self) -> f64 {
        self.bottom - self.top
    }

    /// Reports whether either dimension is zero.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.width() <= 0.0 || self.height() <= 0.0
    }

    /// Returns `self` when it is non-empty.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryFault::EmptyRegion`] otherwise. An operation that needs
    /// pixel content rejects an empty region rather than expanding it to the full
    /// frame or to a single pixel.
    pub fn require_non_empty(self) -> Result<Self, GeometryFault> {
        if self.is_empty() {
            Err(GeometryFault::EmptyRegion)
        } else {
            Ok(self)
        }
    }

    /// Reports whether `point` lies inside these half-open bounds.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryFault::SpaceMismatch`] when the point is in a different
    /// coordinate space, because comparing across spaces without a transform
    /// would produce a confident wrong answer.
    pub fn contains(self, point: Point) -> Result<bool, GeometryFault> {
        if point.space() != self.space {
            return Err(GeometryFault::SpaceMismatch);
        }
        Ok(point.x() >= self.left
            && point.x() < self.right
            && point.y() >= self.top
            && point.y() < self.bottom)
    }
}

impl fmt::Display for Rect {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "[{}, {}) x [{}, {}) {}",
            self.left, self.right, self.top, self.bottom, self.space
        )
    }
}

/// A discrete pixel extent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PixelExtent {
    width: u32,
    height: u32,
}

impl PixelExtent {
    /// Builds an extent.
    #[must_use]
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Returns the width in pixels.
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    /// Returns the height in pixels.
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }

    /// Reports whether either dimension is zero.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Returns the extent as a rectangle anchored at the origin.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryFault::ExtentOverflow`] when a dimension exceeds
    /// [`i32::MAX`] and therefore cannot be an exclusive edge.
    pub fn to_rect(self) -> Result<PixelRect, GeometryFault> {
        let right = i32::try_from(self.width).map_err(|_| GeometryFault::ExtentOverflow)?;
        let bottom = i32::try_from(self.height).map_err(|_| GeometryFault::ExtentOverflow)?;
        PixelRect::new(0, 0, right, bottom)
    }
}

impl fmt::Display for PixelExtent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}x{}px", self.width, self.height)
    }
}

/// A half-open rectangle in discrete capture pixels.
///
/// Edges may be negative, because a conversion can legitimately place a region
/// partly above or left of a frame before a clipping policy is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PixelRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl PixelRect {
    /// Builds a validated half-open pixel rectangle.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryFault::NegativeSize`] when an edge pair is inverted.
    pub fn new(left: i32, top: i32, right: i32, bottom: i32) -> Result<Self, GeometryFault> {
        if right < left || bottom < top {
            return Err(GeometryFault::NegativeSize);
        }
        Ok(Self {
            left,
            top,
            right,
            bottom,
        })
    }

    /// Returns the inclusive left edge.
    #[must_use]
    pub const fn left(self) -> i32 {
        self.left
    }

    /// Returns the inclusive top edge.
    #[must_use]
    pub const fn top(self) -> i32 {
        self.top
    }

    /// Returns the exclusive right edge.
    #[must_use]
    pub const fn right(self) -> i32 {
        self.right
    }

    /// Returns the exclusive bottom edge.
    #[must_use]
    pub const fn bottom(self) -> i32 {
        self.bottom
    }

    /// Returns `right - left`.
    ///
    /// The difference of two `i32` edges always fits `u32`, and the constructor
    /// guarantees the edges are ordered, so this cannot overflow.
    #[must_use]
    pub fn width(self) -> u32 {
        let span = i64::from(self.right) - i64::from(self.left);
        u32::try_from(span).expect("ordered i32 edges span at most u32::MAX")
    }

    /// Returns `bottom - top`.
    #[must_use]
    pub fn height(self) -> u32 {
        let span = i64::from(self.bottom) - i64::from(self.top);
        u32::try_from(span).expect("ordered i32 edges span at most u32::MAX")
    }

    /// Returns the extent this rectangle covers.
    #[must_use]
    pub fn extent(self) -> PixelExtent {
        PixelExtent::new(self.width(), self.height())
    }

    /// Reports whether either dimension is zero.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.width() == 0 || self.height() == 0
    }

    /// Reports whether `other` lies entirely within `self`.
    #[must_use]
    pub const fn contains_rect(self, other: Self) -> bool {
        other.left >= self.left
            && other.top >= self.top
            && other.right <= self.right
            && other.bottom <= self.bottom
    }

    /// Returns the overlapping region, or `None` when the rectangles are disjoint.
    #[must_use]
    pub fn intersect(self, other: Self) -> Option<Self> {
        let left = self.left.max(other.left);
        let top = self.top.max(other.top);
        let right = self.right.min(other.right);
        let bottom = self.bottom.min(other.bottom);
        if right <= left || bottom <= top {
            return None;
        }
        Some(Self {
            left,
            top,
            right,
            bottom,
        })
    }

    /// Applies `policy` against `extent`.
    ///
    /// # Errors
    ///
    /// With [`ClipPolicy::Reject`], returns [`GeometryFault::OutsideExtent`] when
    /// any part of `self` falls outside `extent`. With [`ClipPolicy::Clip`],
    /// returns the same fault only when nothing overlaps, because a request that
    /// clips to nothing has no region left to answer about.
    pub fn resolve_against(
        self,
        extent: PixelExtent,
        policy: ClipPolicy,
    ) -> Result<Self, GeometryFault> {
        let bounds = extent.to_rect()?;
        match policy {
            ClipPolicy::Reject => {
                if bounds.contains_rect(self) {
                    Ok(self)
                } else {
                    Err(GeometryFault::OutsideExtent)
                }
            }
            ClipPolicy::Clip => self.intersect(bounds).ok_or(GeometryFault::OutsideExtent),
        }
    }
}

impl fmt::Display for PixelRect {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "[{}, {}) x [{}, {}) capture_pixels",
            self.left, self.right, self.top, self.bottom
        )
    }
}

fn in_unit_range(value: f64) -> bool {
    (0.0..=1.0).contains(&value)
}

/// Rounds a left or top edge down to the pixel that contains it.
///
/// Flooring the near edges and ceiling the far ones expands a floating region to
/// the smallest pixel rectangle that fully covers it. Expanding is the
/// conservative direction: a search region that lost a partially covered pixel
/// could miss a match that is really there.
pub(crate) fn floor_to_pixel(value: f64) -> Result<i32, GeometryFault> {
    integral_to_i32(value.floor())
}

/// Rounds a right or bottom edge up to the first pixel past the region.
pub(crate) fn ceil_to_pixel(value: f64) -> Result<i32, GeometryFault> {
    integral_to_i32(value.ceil())
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "the range check above proves the integral f64 fits i32 exactly"
)]
fn integral_to_i32(integral: f64) -> Result<i32, GeometryFault> {
    if !integral.is_finite() {
        return Err(GeometryFault::NotFinite);
    }
    if integral < f64::from(i32::MIN) || integral > f64::from(i32::MAX) {
        return Err(GeometryFault::ExtentOverflow);
    }
    Ok(integral as i32)
}

#[cfg(test)]
mod tests {
    use super::{
        ClipPolicy, CoordinateSpace, GeometryFault, PixelExtent, PixelRect, Point, Rect,
        ceil_to_pixel, floor_to_pixel,
    };
    use crate::status::Status;

    const PIXELS: CoordinateSpace = CoordinateSpace::CapturePixels;
    const FRAME: CoordinateSpace = CoordinateSpace::FrameNormalized;

    #[test]
    fn a_rectangle_includes_its_near_edges_and_excludes_its_far_edges() {
        let rect = Rect::new(PIXELS, 10.0, 20.0, 30.0, 40.0).expect("valid");

        assert_eq!(
            rect.contains(Point::new(PIXELS, 10.0, 20.0).expect("valid")),
            Ok(true)
        );
        assert_eq!(
            rect.contains(Point::new(PIXELS, 30.0, 39.0).expect("valid")),
            Ok(false)
        );
        assert_eq!(
            rect.contains(Point::new(PIXELS, 29.0, 40.0).expect("valid")),
            Ok(false)
        );
        assert_eq!(rect.width(), 20.0);
        assert_eq!(rect.height(), 20.0);
    }

    #[test]
    fn comparing_points_across_spaces_is_refused() {
        let rect = Rect::new(PIXELS, 0.0, 0.0, 10.0, 10.0).expect("valid");
        let point = Point::new(FRAME, 0.5, 0.5).expect("valid");

        assert_eq!(rect.contains(point), Err(GeometryFault::SpaceMismatch));
    }

    #[test]
    fn non_finite_geometry_is_rejected() {
        assert_eq!(
            Point::new(PIXELS, f64::NAN, 0.0),
            Err(GeometryFault::NotFinite)
        );
        assert_eq!(
            Point::new(PIXELS, 0.0, f64::INFINITY),
            Err(GeometryFault::NotFinite)
        );
        assert_eq!(
            Rect::new(PIXELS, 0.0, 0.0, f64::NEG_INFINITY, 1.0),
            Err(GeometryFault::NotFinite)
        );
        assert_eq!(
            Rect::from_origin_size(PIXELS, 0.0, 0.0, f64::NAN, 1.0),
            Err(GeometryFault::NotFinite)
        );
    }

    #[test]
    fn inverted_edges_are_rejected() {
        assert_eq!(
            Rect::new(PIXELS, 10.0, 0.0, 5.0, 10.0),
            Err(GeometryFault::NegativeSize)
        );
        assert_eq!(
            Rect::new(PIXELS, 0.0, 10.0, 10.0, 5.0),
            Err(GeometryFault::NegativeSize)
        );
        assert_eq!(
            Rect::from_origin_size(PIXELS, 0.0, 0.0, -1.0, 1.0),
            Err(GeometryFault::NegativeSize)
        );
    }

    #[test]
    fn a_normalized_rectangle_outside_its_domain_is_rejected_not_clamped() {
        assert_eq!(
            Rect::new(FRAME, -0.1, 0.0, 1.0, 1.0),
            Err(GeometryFault::OutOfNormalizedRange)
        );
        assert_eq!(
            Rect::new(FRAME, 0.0, 0.0, 1.5, 1.0),
            Err(GeometryFault::OutOfNormalizedRange)
        );
        assert_eq!(
            Point::new(FRAME, 1.000_001, 0.5),
            Err(GeometryFault::OutOfNormalizedRange)
        );
        assert!(Rect::new(FRAME, 0.0, 0.0, 1.0, 1.0).is_ok());
    }

    #[test]
    fn pixel_space_accepts_values_outside_the_unit_range() {
        assert!(Rect::new(PIXELS, -50.0, -50.0, 4000.0, 3000.0).is_ok());
    }

    #[test]
    fn an_empty_region_is_refused_where_content_is_required() {
        let zero_width = Rect::new(PIXELS, 5.0, 0.0, 5.0, 10.0).expect("valid");
        let zero_height = Rect::new(PIXELS, 0.0, 5.0, 10.0, 5.0).expect("valid");

        assert!(zero_width.is_empty());
        assert!(zero_height.is_empty());
        assert_eq!(
            zero_width.require_non_empty(),
            Err(GeometryFault::EmptyRegion)
        );
        assert_eq!(
            zero_height.require_non_empty(),
            Err(GeometryFault::EmptyRegion)
        );
        assert!(
            Rect::new(PIXELS, 0.0, 0.0, 1.0, 1.0)
                .expect("valid")
                .require_non_empty()
                .is_ok()
        );
    }

    #[test]
    fn edge_rounding_expands_rather_than_shrinks() {
        assert_eq!(floor_to_pixel(10.9), Ok(10));
        assert_eq!(ceil_to_pixel(20.1), Ok(21));
        assert_eq!(floor_to_pixel(-0.1), Ok(-1));
        assert_eq!(ceil_to_pixel(-0.1), Ok(0));
        assert_eq!(floor_to_pixel(7.0), Ok(7), "an exact edge does not move");
        assert_eq!(ceil_to_pixel(7.0), Ok(7), "an exact edge does not move");
    }

    #[test]
    fn rounding_beyond_the_pixel_range_is_reported() {
        assert_eq!(
            floor_to_pixel(f64::from(i32::MIN) - 1.0),
            Err(GeometryFault::ExtentOverflow)
        );
        assert_eq!(
            ceil_to_pixel(f64::from(i32::MAX) + 1.0),
            Err(GeometryFault::ExtentOverflow)
        );
        assert_eq!(floor_to_pixel(f64::NAN), Err(GeometryFault::NotFinite));
    }

    #[test]
    fn pixel_rectangles_measure_their_half_open_span() {
        let rect = PixelRect::new(-10, -5, 10, 5).expect("valid");

        assert_eq!(rect.width(), 20);
        assert_eq!(rect.height(), 10);
        assert_eq!(rect.extent(), PixelExtent::new(20, 10));
        assert!(!rect.is_empty());
    }

    #[test]
    fn the_widest_representable_pixel_span_does_not_overflow() {
        let rect = PixelRect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX).expect("valid");

        assert_eq!(rect.width(), u32::MAX);
        assert_eq!(rect.height(), u32::MAX);
    }

    #[test]
    fn an_extent_beyond_the_signed_pixel_range_is_reported() {
        assert_eq!(
            PixelExtent::new(u32::MAX, 1).to_rect(),
            Err(GeometryFault::ExtentOverflow)
        );
        assert_eq!(
            PixelExtent::new(1920, 1080).to_rect(),
            PixelRect::new(0, 0, 1920, 1080)
        );
    }

    #[test]
    fn rejecting_is_the_default_clipping_policy() {
        assert_eq!(ClipPolicy::default(), ClipPolicy::Reject);
    }

    #[test]
    fn a_region_outside_the_extent_is_refused_unless_clipping_was_requested() {
        let extent = PixelExtent::new(100, 100);
        let overhanging = PixelRect::new(50, 50, 150, 150).expect("valid");

        assert_eq!(
            overhanging.resolve_against(extent, ClipPolicy::Reject),
            Err(GeometryFault::OutsideExtent)
        );
        assert_eq!(
            overhanging.resolve_against(extent, ClipPolicy::Clip),
            PixelRect::new(50, 50, 100, 100)
        );
    }

    #[test]
    fn a_contained_region_survives_either_policy_unchanged() {
        let extent = PixelExtent::new(100, 100);
        let inside = PixelRect::new(10, 10, 20, 20).expect("valid");

        assert_eq!(
            inside.resolve_against(extent, ClipPolicy::Reject),
            Ok(inside)
        );
        assert_eq!(inside.resolve_against(extent, ClipPolicy::Clip), Ok(inside));
    }

    #[test]
    fn clipping_to_nothing_is_a_failure_rather_than_an_empty_region() {
        let extent = PixelExtent::new(100, 100);
        let disjoint = PixelRect::new(200, 200, 300, 300).expect("valid");

        assert_eq!(
            disjoint.resolve_against(extent, ClipPolicy::Clip),
            Err(GeometryFault::OutsideExtent)
        );
    }

    #[test]
    fn touching_rectangles_do_not_intersect() {
        let left = PixelRect::new(0, 0, 10, 10).expect("valid");
        let right = PixelRect::new(10, 0, 20, 10).expect("valid");

        assert_eq!(left.intersect(right), None);
    }

    #[test]
    fn faults_map_to_public_statuses() {
        // Every variant is paired with the status it reports. This asserted
        // four of the eight before, so half the mapping was released with
        // nothing checking it.
        //
        // `status()` is itself an exhaustive match, so a new fault already
        // fails to compile there. What the match at the end of the loop adds
        // is that it fails to compile *here* as well, which is what brings
        // whoever adds the fault into this test to extend the list. Nothing
        // forces the pair itself to be added; that residual is why the list is
        // stated in full rather than sampled.
        let expectations = [
            (GeometryFault::NotFinite, Status::InvalidArgument),
            (GeometryFault::NegativeSize, Status::InvalidArgument),
            (GeometryFault::OutOfNormalizedRange, Status::InvalidArgument),
            (GeometryFault::EmptyRegion, Status::InvalidArgument),
            (GeometryFault::SpaceMismatch, Status::InvalidArgument),
            (GeometryFault::ConversionUnsupported, Status::Unsupported),
            (GeometryFault::OutsideExtent, Status::InvalidArgument),
            (GeometryFault::ExtentOverflow, Status::InvalidArgument),
        ];

        for (fault, status) in expectations {
            assert_eq!(fault.status(), status, "{fault:?}");
            match fault {
                GeometryFault::NotFinite
                | GeometryFault::NegativeSize
                | GeometryFault::OutOfNormalizedRange
                | GeometryFault::EmptyRegion
                | GeometryFault::SpaceMismatch
                | GeometryFault::ConversionUnsupported
                | GeometryFault::OutsideExtent
                | GeometryFault::ExtentOverflow => {}
            }
        }
    }

    #[test]
    fn normalized_spaces_are_the_ones_confined_to_the_unit_range() {
        assert!(CoordinateSpace::FrameNormalized.is_normalized());
        assert!(CoordinateSpace::TargetNormalized.is_normalized());
        assert!(!CoordinateSpace::CapturePixels.is_normalized());
        assert!(!CoordinateSpace::TargetLogical.is_normalized());
        assert!(!CoordinateSpace::DesktopLogical.is_normalized());
    }
}
