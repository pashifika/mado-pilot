//! Frame-time coordinate transforms.
//!
//! A [`TransformSnapshot`] is the geometry that was authoritative when one frame
//! was captured. Conversions use a snapshot rather than live host state, so an
//! answer about a retained frame stays correct after the window has moved.
//!
//! Every conversion is explicit about both spaces, and a conversion the snapshot
//! cannot represent fails. In particular a snapshot that knows nothing about the
//! target does not fall back to treating logical units as pixels, and never
//! consults host DPI: a plausible guess about coordinates produces input
//! delivered to the wrong place, which is worse than a refusal.

use crate::geometry::{
    ClipPolicy, CoordinateSpace, GeometryFault, PixelExtent, PixelRect, Point, Rect, ceil_to_pixel,
    floor_to_pixel,
};
use crate::identity::GeometryRevision;

/// Capture pixels per logical unit, per axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale {
    x: f64,
    y: f64,
}

impl Scale {
    /// Builds a scale.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryFault::NotFinite`] for a NaN or infinite factor and
    /// [`GeometryFault::NegativeSize`] for a factor that is not strictly
    /// positive, since a zero or negative scale has no inverse.
    pub fn new(x: f64, y: f64) -> Result<Self, GeometryFault> {
        if !x.is_finite() || !y.is_finite() {
            return Err(GeometryFault::NotFinite);
        }
        if x <= 0.0 || y <= 0.0 {
            return Err(GeometryFault::NegativeSize);
        }
        Ok(Self { x, y })
    }

    /// Returns the horizontal factor.
    #[must_use]
    pub const fn x(self) -> f64 {
        self.x
    }

    /// Returns the vertical factor.
    #[must_use]
    pub const fn y(self) -> f64 {
        self.y
    }
}

/// Where a frame sits relative to its target, and at what scale.
///
/// Supplying this is a provider's assertion that the frame's capture pixels cover
/// exactly the target's logical rectangle. Phase 1 has no frame that covers only
/// part of its target, so no sub-region offset is modelled yet; adding one later
/// is additive, whereas assuming a placement that does not hold would silently
/// misplace every converted coordinate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TargetPlacement {
    desktop_origin_x: f64,
    desktop_origin_y: f64,
    logical_width: f64,
    logical_height: f64,
    scale: Scale,
}

impl TargetPlacement {
    /// Describes a target's placement in desktop-logical coordinates.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryFault::NotFinite`] for a non-finite value and
    /// [`GeometryFault::NegativeSize`] for a negative logical size.
    pub fn new(
        desktop_origin: (f64, f64),
        logical_size: (f64, f64),
        scale: Scale,
    ) -> Result<Self, GeometryFault> {
        let (desktop_origin_x, desktop_origin_y) = desktop_origin;
        let (logical_width, logical_height) = logical_size;
        if !(desktop_origin_x.is_finite()
            && desktop_origin_y.is_finite()
            && logical_width.is_finite()
            && logical_height.is_finite())
        {
            return Err(GeometryFault::NotFinite);
        }
        if logical_width <= 0.0 || logical_height <= 0.0 {
            return Err(GeometryFault::NegativeSize);
        }
        Ok(Self {
            desktop_origin_x,
            desktop_origin_y,
            logical_width,
            logical_height,
            scale,
        })
    }

    /// Returns the target origin in desktop-logical coordinates.
    #[must_use]
    pub const fn desktop_origin(self) -> (f64, f64) {
        (self.desktop_origin_x, self.desktop_origin_y)
    }

    /// Returns the target size in target-logical units.
    #[must_use]
    pub const fn logical_size(self) -> (f64, f64) {
        (self.logical_width, self.logical_height)
    }

    /// Returns the capture pixels per logical unit.
    #[must_use]
    pub const fn scale(self) -> Scale {
        self.scale
    }
}

/// The geometry that was authoritative when one frame was captured.
///
/// A snapshot always knows the frame's own extent, so conversions between capture
/// pixels and frame-normalized coordinates are always available. Everything that
/// refers to a target requires a [`TargetPlacement`], which a provider supplies
/// only when it actually knows where the target is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransformSnapshot {
    geometry: GeometryRevision,
    frame_extent: PixelExtent,
    target: Option<TargetPlacement>,
}

impl TransformSnapshot {
    /// Builds a snapshot that knows only the frame's own extent.
    ///
    /// Conversions involving a target return
    /// [`GeometryFault::ConversionUnsupported`].
    #[must_use]
    pub const fn frame_only(geometry: GeometryRevision, frame_extent: PixelExtent) -> Self {
        Self {
            geometry,
            frame_extent,
            target: None,
        }
    }

    /// Builds a snapshot that also knows where its target is.
    #[must_use]
    pub const fn with_target(
        geometry: GeometryRevision,
        frame_extent: PixelExtent,
        placement: TargetPlacement,
    ) -> Self {
        Self {
            geometry,
            frame_extent,
            target: Some(placement),
        }
    }

    /// Returns the geometry revision this snapshot belongs to.
    ///
    /// A conversion reports the same revision as the frame it came from, so a
    /// caller can tell which geometry an answer was computed against.
    #[must_use]
    pub const fn geometry(&self) -> GeometryRevision {
        self.geometry
    }

    /// Returns the frame extent in capture pixels.
    #[must_use]
    pub const fn frame_extent(&self) -> PixelExtent {
        self.frame_extent
    }

    /// Returns the target placement, when the provider supplied one.
    #[must_use]
    pub const fn target(&self) -> Option<TargetPlacement> {
        self.target
    }

    /// Reports whether this snapshot can convert between two spaces.
    #[must_use]
    pub fn supports(&self, space: CoordinateSpace) -> bool {
        match space {
            CoordinateSpace::CapturePixels | CoordinateSpace::FrameNormalized => {
                !self.frame_extent.is_empty()
            }
            CoordinateSpace::TargetNormalized
            | CoordinateSpace::TargetLogical
            | CoordinateSpace::DesktopLogical => {
                self.target.is_some() && !self.frame_extent.is_empty()
            }
        }
    }

    /// Converts `point` into `to`.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryFault::ConversionUnsupported`] when this snapshot has no
    /// authoritative mapping for either space, and the validation faults of
    /// [`Point::new`] when the converted value is not representable in the
    /// destination space.
    pub fn convert_point(&self, point: Point, to: CoordinateSpace) -> Result<Point, GeometryFault> {
        let (pixel_x, pixel_y) = self.project_to_pixels(point.space(), point.x(), point.y())?;
        let (x, y) = self.project_from_pixels(to, pixel_x, pixel_y)?;
        Point::new(to, x, y)
    }

    /// Converts `rect` into `to`.
    ///
    /// Every Phase 1 transform is an axis-aligned scale and translation, so
    /// converting the two corners is exact and preserves edge ordering.
    ///
    /// # Errors
    ///
    /// As [`TransformSnapshot::convert_point`].
    pub fn convert_rect(&self, rect: Rect, to: CoordinateSpace) -> Result<Rect, GeometryFault> {
        let (left, top) = self.project_to_pixels(rect.space(), rect.left(), rect.top())?;
        let (right, bottom) = self.project_to_pixels(rect.space(), rect.right(), rect.bottom())?;
        let (left, top) = self.project_from_pixels(to, left, top)?;
        let (right, bottom) = self.project_from_pixels(to, right, bottom)?;
        Rect::new(to, left, top, right, bottom)
    }

    /// Resolves `rect` to the discrete capture pixels it covers.
    ///
    /// Near edges are floored and far edges are ceiled, so the result is the
    /// smallest pixel rectangle that fully contains the request. `policy` is then
    /// applied against the frame extent.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryFault::ConversionUnsupported`] when the source space is
    /// not represented, [`GeometryFault::ExtentOverflow`] when a rounded edge does
    /// not fit the pixel range, and [`GeometryFault::OutsideExtent`] according to
    /// `policy`.
    pub fn resolve_capture_pixels(
        &self,
        rect: Rect,
        policy: ClipPolicy,
    ) -> Result<PixelRect, GeometryFault> {
        let (left, top) = self.project_to_pixels(rect.space(), rect.left(), rect.top())?;
        let (right, bottom) = self.project_to_pixels(rect.space(), rect.right(), rect.bottom())?;
        let resolved = PixelRect::new(
            floor_to_pixel(left)?,
            floor_to_pixel(top)?,
            ceil_to_pixel(right)?,
            ceil_to_pixel(bottom)?,
        )?;
        resolved.resolve_against(self.frame_extent, policy)
    }

    /// Resolves the whole frame as a pixel rectangle.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryFault::ExtentOverflow`] when the frame extent does not
    /// fit the signed pixel range.
    pub fn frame_bounds(&self) -> Result<PixelRect, GeometryFault> {
        self.frame_extent.to_rect()
    }

    fn project_to_pixels(
        &self,
        from: CoordinateSpace,
        x: f64,
        y: f64,
    ) -> Result<(f64, f64), GeometryFault> {
        if !self.supports(from) {
            return Err(GeometryFault::ConversionUnsupported);
        }
        let frame_width = f64::from(self.frame_extent.width());
        let frame_height = f64::from(self.frame_extent.height());
        match from {
            CoordinateSpace::CapturePixels => Ok((x, y)),
            // A target-normalized coordinate maps onto the frame the same way a
            // frame-normalized one does, because a placement asserts that the
            // frame covers exactly the target. What differs is that the target
            // spaces require that assertion to have been made at all.
            CoordinateSpace::FrameNormalized | CoordinateSpace::TargetNormalized => {
                Ok((x * frame_width, y * frame_height))
            }
            CoordinateSpace::TargetLogical => {
                let scale = self.placement()?.scale;
                Ok((x * scale.x, y * scale.y))
            }
            CoordinateSpace::DesktopLogical => {
                let placement = self.placement()?;
                let scale = placement.scale;
                Ok((
                    (x - placement.desktop_origin_x) * scale.x,
                    (y - placement.desktop_origin_y) * scale.y,
                ))
            }
        }
    }

    fn project_from_pixels(
        &self,
        to: CoordinateSpace,
        x: f64,
        y: f64,
    ) -> Result<(f64, f64), GeometryFault> {
        if !self.supports(to) {
            return Err(GeometryFault::ConversionUnsupported);
        }
        let frame_width = f64::from(self.frame_extent.width());
        let frame_height = f64::from(self.frame_extent.height());
        match to {
            CoordinateSpace::CapturePixels => Ok((x, y)),
            CoordinateSpace::FrameNormalized | CoordinateSpace::TargetNormalized => {
                Ok((x / frame_width, y / frame_height))
            }
            CoordinateSpace::TargetLogical => {
                let scale = self.placement()?.scale;
                Ok((x / scale.x, y / scale.y))
            }
            CoordinateSpace::DesktopLogical => {
                let placement = self.placement()?;
                let scale = placement.scale;
                Ok((
                    x / scale.x + placement.desktop_origin_x,
                    y / scale.y + placement.desktop_origin_y,
                ))
            }
        }
    }

    fn placement(&self) -> Result<TargetPlacement, GeometryFault> {
        self.target.ok_or(GeometryFault::ConversionUnsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::{Scale, TargetPlacement, TransformSnapshot};
    use crate::geometry::{
        ClipPolicy, CoordinateSpace, GeometryFault, PixelExtent, PixelRect, Point, Rect,
    };
    use crate::identity::GeometryRevision;

    const PIXELS: CoordinateSpace = CoordinateSpace::CapturePixels;
    const FRAME: CoordinateSpace = CoordinateSpace::FrameNormalized;
    const TARGET_NORMALIZED: CoordinateSpace = CoordinateSpace::TargetNormalized;
    const TARGET_LOGICAL: CoordinateSpace = CoordinateSpace::TargetLogical;
    const DESKTOP: CoordinateSpace = CoordinateSpace::DesktopLogical;

    fn frame_only() -> TransformSnapshot {
        TransformSnapshot::frame_only(GeometryRevision::FIRST, PixelExtent::new(1920, 1080))
    }

    fn with_target() -> TransformSnapshot {
        let placement = TargetPlacement::new(
            (100.0, 50.0),
            (960.0, 540.0),
            Scale::new(2.0, 2.0).expect("valid"),
        )
        .expect("valid");
        TransformSnapshot::with_target(
            GeometryRevision::FIRST,
            PixelExtent::new(1920, 1080),
            placement,
        )
    }

    #[test]
    fn frame_normalized_and_capture_pixels_convert_both_ways() {
        let snapshot = frame_only();
        let normalized = Point::new(FRAME, 0.5, 0.25).expect("valid");

        let pixels = snapshot
            .convert_point(normalized, PIXELS)
            .expect("supported");

        assert_eq!(pixels.x(), 960.0);
        assert_eq!(pixels.y(), 270.0);
        assert_eq!(
            snapshot.convert_point(pixels, FRAME).expect("supported"),
            normalized
        );
    }

    #[test]
    fn a_frame_only_snapshot_refuses_every_target_conversion() {
        let snapshot = frame_only();
        let point = Point::new(PIXELS, 10.0, 10.0).expect("valid");

        for space in [TARGET_NORMALIZED, TARGET_LOGICAL, DESKTOP] {
            assert!(!snapshot.supports(space), "{space}");
            assert_eq!(
                snapshot.convert_point(point, space),
                Err(GeometryFault::ConversionUnsupported),
                "{space}"
            );
        }
    }

    #[test]
    fn logical_units_are_never_assumed_to_equal_pixels() {
        let snapshot = frame_only();
        let logical = Rect::new(TARGET_LOGICAL, 0.0, 0.0, 100.0, 100.0).expect("valid");

        assert_eq!(
            snapshot.resolve_capture_pixels(logical, ClipPolicy::Reject),
            Err(GeometryFault::ConversionUnsupported),
            "an identity transform would place input at the wrong scale"
        );
    }

    #[test]
    fn target_logical_scales_by_the_placement_factor() {
        let snapshot = with_target();
        let logical = Point::new(TARGET_LOGICAL, 480.0, 270.0).expect("valid");

        let pixels = snapshot.convert_point(logical, PIXELS).expect("supported");

        assert_eq!(pixels.x(), 960.0);
        assert_eq!(pixels.y(), 540.0);
        assert_eq!(
            snapshot
                .convert_point(pixels, TARGET_LOGICAL)
                .expect("supported"),
            logical
        );
    }

    #[test]
    fn desktop_logical_applies_the_target_origin() {
        let snapshot = with_target();
        let desktop = Point::new(DESKTOP, 100.0, 50.0).expect("valid");

        let pixels = snapshot.convert_point(desktop, PIXELS).expect("supported");

        assert_eq!(
            (pixels.x(), pixels.y()),
            (0.0, 0.0),
            "the target origin is the frame origin"
        );
        assert_eq!(
            snapshot.convert_point(pixels, DESKTOP).expect("supported"),
            desktop
        );
    }

    #[test]
    fn target_normalized_requires_a_placement_even_though_it_scales_like_the_frame() {
        let point = Point::new(TARGET_NORMALIZED, 0.5, 0.5).expect("valid");

        assert_eq!(
            frame_only().convert_point(point, PIXELS),
            Err(GeometryFault::ConversionUnsupported)
        );

        let converted = with_target()
            .convert_point(point, PIXELS)
            .expect("supported");
        assert_eq!((converted.x(), converted.y()), (960.0, 540.0));
    }

    #[test]
    fn a_fractional_region_expands_to_the_pixels_it_touches() {
        let snapshot = frame_only();
        let rect = Rect::new(PIXELS, 10.4, 20.6, 30.1, 40.9).expect("valid");

        let resolved = snapshot
            .resolve_capture_pixels(rect, ClipPolicy::Reject)
            .expect("inside the frame");

        assert_eq!(resolved, PixelRect::new(10, 20, 31, 41).expect("valid"));
    }

    #[test]
    fn a_full_frame_normalized_region_resolves_to_the_whole_frame() {
        let snapshot = frame_only();
        let rect = Rect::new(FRAME, 0.0, 0.0, 1.0, 1.0).expect("valid");

        let resolved = snapshot
            .resolve_capture_pixels(rect, ClipPolicy::Reject)
            .expect("inside the frame");

        assert_eq!(resolved, snapshot.frame_bounds().expect("representable"));
        assert_eq!(resolved.extent(), PixelExtent::new(1920, 1080));
    }

    #[test]
    fn an_overhanging_region_is_refused_unless_clipping_was_requested() {
        let snapshot = frame_only();
        let rect = Rect::new(PIXELS, 1900.0, 1070.0, 2000.0, 1100.0).expect("valid");

        assert_eq!(
            snapshot.resolve_capture_pixels(rect, ClipPolicy::Reject),
            Err(GeometryFault::OutsideExtent)
        );
        assert_eq!(
            snapshot.resolve_capture_pixels(rect, ClipPolicy::Clip),
            PixelRect::new(1900, 1070, 1920, 1080)
        );
    }

    #[test]
    fn a_conversion_reports_the_source_frames_geometry_revision() {
        let revision = GeometryRevision::FIRST.next().expect("representable");
        let snapshot = TransformSnapshot::frame_only(revision, PixelExtent::new(64, 64));

        assert_eq!(snapshot.geometry(), revision);
    }

    #[test]
    fn an_empty_frame_supports_no_conversion() {
        let snapshot =
            TransformSnapshot::frame_only(GeometryRevision::FIRST, PixelExtent::new(0, 0));
        let point = Point::new(PIXELS, 0.0, 0.0).expect("valid");

        assert!(!snapshot.supports(PIXELS));
        assert_eq!(
            snapshot.convert_point(point, PIXELS),
            Err(GeometryFault::ConversionUnsupported),
            "dividing by a zero extent has no meaningful answer"
        );
    }

    #[test]
    fn a_pixel_region_outside_the_normalized_domain_is_refused_on_conversion() {
        let snapshot = frame_only();
        let outside = Rect::new(PIXELS, -10.0, -10.0, 10.0, 10.0).expect("valid");

        assert_eq!(
            snapshot.convert_rect(outside, FRAME),
            Err(GeometryFault::OutOfNormalizedRange),
            "a negative pixel cannot be expressed as a normalized coordinate"
        );
    }

    #[test]
    fn a_rectangle_round_trips_through_a_target_space() {
        let snapshot = with_target();
        let pixels = Rect::new(PIXELS, 100.0, 200.0, 300.0, 400.0).expect("valid");

        let logical = snapshot
            .convert_rect(pixels, TARGET_LOGICAL)
            .expect("supported");
        let back = snapshot.convert_rect(logical, PIXELS).expect("supported");

        assert_eq!(logical.left(), 50.0);
        assert_eq!(logical.right(), 150.0);
        assert_eq!(back, pixels);
    }

    #[test]
    fn a_scale_must_be_finite_and_positive() {
        assert_eq!(Scale::new(f64::NAN, 1.0), Err(GeometryFault::NotFinite));
        assert_eq!(Scale::new(0.0, 1.0), Err(GeometryFault::NegativeSize));
        assert_eq!(Scale::new(-1.0, 1.0), Err(GeometryFault::NegativeSize));
        assert!(Scale::new(1.5, 1.5).is_ok());
    }

    #[test]
    fn a_placement_must_have_a_positive_logical_size() {
        let scale = Scale::new(1.0, 1.0).expect("valid");

        assert_eq!(
            TargetPlacement::new((0.0, 0.0), (0.0, 100.0), scale),
            Err(GeometryFault::NegativeSize)
        );
        assert_eq!(
            TargetPlacement::new((f64::INFINITY, 0.0), (10.0, 10.0), scale),
            Err(GeometryFault::NotFinite)
        );
    }

    #[test]
    fn placement_accessors_report_what_was_supplied() {
        let placement = with_target().target().expect("present");

        assert_eq!(placement.desktop_origin(), (100.0, 50.0));
        assert_eq!(placement.logical_size(), (960.0, 540.0));
        assert_eq!(placement.scale().x(), 2.0);
        assert_eq!(placement.scale().y(), 2.0);
    }
}
