//! Picker-free window and display inventory, and the geometry it normalizes.
//!
//! # The coordinate space
//!
//! macOS reports window and display placement in one continuous plane of points
//! with a top-left origin on the main display, and signed coordinates for
//! anything above or to the left of it. Capture pixels per point is the target
//! display's backing scale, so a target's own scale and the desktop's are the same
//! factor and a placement needs no independent desktop scale. That is a real
//! difference from the Windows Adapter, whose desktop plane is measured in
//! physical pixels and therefore needs one.
//!
//! # Orientation
//!
//! macOS has two rectangle conventions, and mixing them flips every vertical
//! coordinate. AppKit measures a window's frame from the bottom-left of the main
//! display; Core Graphics window bounds, display bounds, capture-framework frames,
//! and Core Video buffer rows all measure from the top-left. This Adapter reads
//! only the top-left convention — the framework's own window and display frames at
//! discovery, Core Graphics bounds at frame time, and the buffer's first row as the
//! frame's first row — so no conversion is needed and no AppKit rectangle enters.
//! Normalization here is the choice not to mix the two rather than arithmetic that
//! reconciles them, which is why there is no flip to find in this module.

use mado_pilot_capture::{CaptureFault, CoordinateSupport, PixelFormat, TargetDescription};
use mado_pilot_core::{
    CapabilitySupport, GeometryFault, InputCapability, PermissionKind, PixelExtent, Result, Scale,
    TargetCapability, TargetId, TargetKind, TargetPlacement,
};

use crate::shim::{
    self, Inventory, KIND_DISPLAY, KIND_WINDOW, MAX_SURFACE_BYTES, MAX_SURFACE_EXTENT, ShimStatus,
};

/// The stable native lookup key. It is never exposed through a public contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum NativeKey {
    Window(u32),
    Display(u32),
}

impl NativeKey {
    pub(crate) const fn kind(self) -> TargetKind {
        match self {
            NativeKey::Window(_) => TargetKind::Window,
            NativeKey::Display(_) => TargetKind::Display,
        }
    }

    pub(crate) const fn native_kind(self) -> u32 {
        match self {
            NativeKey::Window(_) => KIND_WINDOW,
            NativeKey::Display(_) => KIND_DISPLAY,
        }
    }

    pub(crate) const fn native_id(self) -> u64 {
        match self {
            NativeKey::Window(identifier) | NativeKey::Display(identifier) => identifier as u64,
        }
    }

    /// Reports whether the target is still present, without opening anything.
    pub(crate) fn is_present(self) -> bool {
        shim::current_placement(self.native_kind(), self.native_id()).is_ok()
    }

    fn from_info(kind: u32, native_id: u64) -> Option<Self> {
        let identifier = u32::try_from(native_id).ok()?;
        match kind {
            KIND_WINDOW => Some(NativeKey::Window(identifier)),
            KIND_DISPLAY => Some(NativeKey::Display(identifier)),
            _ => None,
        }
    }
}

/// Native observations that distinguish one incarnation of a target from a
/// replacement that happens to have inherited its identifier.
///
/// macOS reuses window numbers, so the owning process is carried alongside the
/// identifier: a window that closes and is replaced by another window of another
/// process is a different target even if the number matches.
///
/// A display carries its captured extent rather than its placement. Rearranging
/// displays moves the same physical display, so its identity persists and the
/// geometry revision of the next published frame reports the move; changing its
/// mode changes the shape of what capture produces, which is a new incarnation. The
/// extent is used rather than a vendor or serial number because those describe the
/// user's hardware and this fingerprint is compared, not reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fingerprint {
    Window { owner_process: i64 },
    Display { extent: PixelExtent },
}

impl Fingerprint {
    /// Returns the owning process an open must match, or zero when there is none.
    ///
    /// A display has no owner, and a window whose owner the framework did not name
    /// records zero — which the native lookup treats as a value to match rather than
    /// as an absent constraint, so that an unnamed owner cannot stand in for any.
    pub(crate) const fn native_owner(self) -> i64 {
        match self {
            Fingerprint::Window { owner_process } => owner_process,
            Fingerprint::Display { .. } => 0,
        }
    }
}

/// The mutable metadata one discovery pass observed for a target.
#[derive(Debug, Clone)]
pub(crate) struct TargetMetadata {
    pub(crate) name: String,
    pub(crate) extent: PixelExtent,
    pub(crate) placement: TargetPlacement,
}

impl TargetMetadata {
    /// Describes the target as this provider can act on it.
    ///
    /// The capability states what this Change implements and nothing more: capture
    /// through ScreenCaptureKit, every coordinate space the placement supports,
    /// and no input. macOS input arrives with the Change that implements and tests
    /// it, and advertising it here would be a claim without an implementation.
    pub(crate) fn describe(&self, id: TargetId, kind: TargetKind) -> TargetDescription {
        TargetDescription::new(
            id,
            self.name.clone(),
            self.extent,
            PixelFormat::Bgra8,
            CoordinateSupport::with_target_placement(),
        )
        .with_capability(
            TargetCapability::new(kind, CapabilitySupport::Supported, InputCapability::none())
                .with_capture_permission(PermissionKind::ScreenCapture),
        )
    }
}

/// One target a discovery pass observed, with everything an open needs.
#[derive(Debug, Clone)]
pub(crate) struct Candidate {
    pub(crate) key: NativeKey,
    pub(crate) fingerprint: Fingerprint,
    pub(crate) metadata: TargetMetadata,
}

/// Returns every currently shareable window and display, in a stable order.
///
/// The order is by kind, then by lowercased name, then by native key, so two
/// passes over an unchanged desktop produce the same sequence. Nothing here can
/// present a picker: the shim refuses before it would reach the framework query
/// that could.
pub(crate) fn inventory(wait: std::time::Duration) -> Result<Vec<Candidate>> {
    let inventory = Inventory::acquire(wait).map_err(discovery_error)?;
    let mut candidates = Vec::with_capacity(inventory.len());
    for index in 0..inventory.len() {
        let Ok(info) = inventory.entry(index) else {
            continue;
        };
        let Some(key) = NativeKey::from_info(info.kind, info.native_id) else {
            continue;
        };
        let Some(extent) = info.extent() else {
            continue;
        };
        let Ok(placement) = placement_from_points(
            (info.logical_x, info.logical_y),
            (info.logical_width, info.logical_height),
            info.backing_scale,
            extent,
        ) else {
            continue;
        };
        let name = inventory.name(index).unwrap_or("").to_owned();
        candidates.push(Candidate {
            key,
            fingerprint: fingerprint(key, &info, extent),
            metadata: TargetMetadata {
                name,
                extent,
                placement,
            },
        });
    }
    candidates.sort_by(|left, right| {
        left.key
            .kind()
            .cmp(&right.key.kind())
            .then_with(|| {
                left.metadata
                    .name
                    .to_lowercase()
                    .cmp(&right.metadata.name.to_lowercase())
            })
            .then_with(|| left.key.cmp(&right.key))
    });
    Ok(candidates)
}

/// What re-reading the target's live placement established for one frame.
///
/// The three outcomes are deliberately separate. Collapsing the middle one into
/// [`PlacementReading::Lost`] is how a resize in flight became a permanent target
/// loss: a window dragged between displays is resized by the window server a
/// fraction of a second after the move, so for a few frames the producer's surface
/// and the live window disagree — and a target that is plainly still there was
/// reported as gone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum PlacementReading {
    /// The live geometry agrees with this frame's own extent and scale.
    ///
    /// `wanted` is the producer surface extent the target needs at its own display's
    /// backing scale. It is not always this frame's extent: the framework scales a
    /// target down to fit a surface too small to hold it and reports the reduced
    /// size, so the size to ask a producer for comes from the target rather than
    /// from the content in hand.
    Ready {
        placement: TargetPlacement,
        wanted: Option<PixelExtent>,
    },
    /// The live geometry has moved on since this frame was produced. Carries the
    /// producer surface extent that would match the target as it is now, when one
    /// can be computed.
    Unsettled { wanted: Option<PixelExtent> },
    /// The frame's own reported scale is not a usable factor.
    Unusable,
    /// The target is no longer present.
    Lost,
}

impl PlacementReading {
    /// Returns the producer surface extent the live target needs, when this reading
    /// established one.
    pub(crate) const fn wanted(self) -> Option<PixelExtent> {
        match self {
            Self::Ready { wanted, .. } | Self::Unsettled { wanted } => wanted,
            // Neither reading established a live size to derive one from.
            Self::Unusable | Self::Lost => None,
        }
    }
}

/// Re-reads placement at frame arrival, so a retained frame never consults live
/// host geometry after publication.
pub(crate) fn read_placement(key: NativeKey, extent: PixelExtent, scale: f64) -> PlacementReading {
    let Ok(live) = shim::current_placement(key.native_kind(), key.native_id()) else {
        return PlacementReading::Lost;
    };
    let origin = (live.frame[0], live.frame[1]);
    let size = (live.frame[2], live.frame[3]);
    match placement_from_points(origin, size, scale, extent) {
        Ok(placement) => PlacementReading::Ready {
            placement,
            wanted: surface_for(size, live.display_scale),
        },
        // The frame agreed with nothing because its scale is unusable, which is a
        // descriptor problem rather than anything about the target.
        Err(GeometryFault::NotFinite | GeometryFault::NegativeSize) => PlacementReading::Unusable,
        Err(_) => PlacementReading::Unsettled {
            wanted: surface_for(size, live.display_scale),
        },
    }
}

/// Returns the producer surface extent that matches `size` points at `scale`.
///
/// `None` for anything the shim would refuse anyway, so a nonsensical live reading
/// asks for no reconfiguration rather than an absurd surface.
fn surface_for(size: (f64, f64), scale: f64) -> Option<PixelExtent> {
    let width = surface_pixels(size.0, scale)?;
    let height = surface_pixels(size.1, scale)?;
    // The axes are bounded one at a time above; what gets allocated is their product,
    // and the shim refuses a surface beyond its byte ceiling however the axes look.
    if u64::from(width) * u64::from(height) * 4 > MAX_SURFACE_BYTES {
        return None;
    }
    Some(PixelExtent::new(width, height))
}

#[expect(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    reason = "the range check below proves the rounded value is a positive integer \
              within the shim's own extent bound, so it converts exactly"
)]
fn surface_pixels(points: f64, scale: f64) -> Option<u32> {
    let rounded = (points * scale).round();
    if !rounded.is_finite() || rounded < 1.0 || rounded > f64::from(MAX_SURFACE_EXTENT) {
        return None;
    }
    Some(rounded as u32)
}

/// Builds the placement for a target whose logical rectangle is `size` points at
/// `origin`, captured at `scale` pixels per point.
///
/// The logical size comes from the captured extent rather than from the reported
/// point size. Both describe the same rectangle, but only the extent is what the
/// frame actually contains, and a placement whose scaled logical size is not the
/// frame extent is refused by the transform snapshot that consumes it.
fn placement_from_points(
    origin: (f64, f64),
    size: (f64, f64),
    scale: f64,
    extent: PixelExtent,
) -> std::result::Result<TargetPlacement, GeometryFault> {
    let scale = Scale::new(scale, scale)?;
    let logical = (
        f64::from(extent.width()) / scale.x(),
        f64::from(extent.height()) / scale.y(),
    );
    // The reported point size is not discarded silently: it is what the origin
    // belongs to, and a size that disagrees with the extent by a whole point or more
    // means the two observations describe different rectangles. The slack is for
    // pixel quantization, which is under a point at every scale — an exact point of
    // disagreement is two pixels on a Retina display, which is a resize, and
    // accepting it published a stale frame as covering the target it no longer fits.
    if (logical.0 - size.0).abs() >= 1.0 || (logical.1 - size.1).abs() >= 1.0 {
        return Err(GeometryFault::SpaceMismatch);
    }
    TargetPlacement::new(origin, logical, scale)
}

fn fingerprint(key: NativeKey, info: &shim::TargetInfo, extent: PixelExtent) -> Fingerprint {
    match key {
        NativeKey::Window(_) => Fingerprint::Window {
            owner_process: info.owner_process,
        },
        NativeKey::Display(_) => Fingerprint::Display { extent },
    }
}

fn discovery_error(status: ShimStatus) -> mado_pilot_core::Error {
    match status {
        // A denied authorization is what discovery reports rather than an empty
        // list: an empty desktop and an unauthorized one are different answers.
        ShimStatus::PermissionDenied => CaptureFault::AccessDenied.into(),
        ShimStatus::Unsupported => CaptureFault::UnsupportedOption.into(),
        other => other.into(),
    }
}

#[cfg(test)]
mod tests {
    use mado_pilot_core::{
        CapabilitySupport, CoordinateSpace, GeometryFault, GeometryRevision, IdentityIssuer,
        InputDelivery, InputOperationKind, PermissionKind, PixelExtent, Point, TargetKind,
        TransformSnapshot,
    };

    use super::{
        Fingerprint, MAX_SURFACE_EXTENT, NativeKey, TargetMetadata, placement_from_points,
        surface_for,
    };

    /// The metadata any described target is built from. The values are arbitrary;
    /// what the description says about capability does not depend on them.
    fn metadata() -> TargetMetadata {
        let extent = PixelExtent::new(2560, 1600);
        TargetMetadata {
            name: "a target".to_owned(),
            extent,
            placement: placement_from_points((0.0, 0.0), (1280.0, 800.0), 2.0, extent)
                .expect("a doubled backing scale covers the frame"),
        }
    }

    #[test]
    fn a_described_target_offers_capture_under_screen_recording_and_no_input_at_all() {
        // The requirement this pins was rewritten during this Change to match the
        // implementation, after the spec had asked for `CGEvent` system input that no
        // code here provides. A requirement corrected toward the code and then left
        // unpinned reverses quietly: adding a pair to `describe` — the natural edit
        // when the macOS input Change lands and someone extends the capture provider
        // instead of the input one — would otherwise fail nothing.
        let issuer = IdentityIssuer::new();
        let id = issuer
            .issue_target(crate::provider::PROVIDER)
            .expect("issued");

        let description = metadata().describe(id, TargetKind::Window);
        let capability = description.capability();

        assert_eq!(capability.kind(), Some(TargetKind::Window));
        assert_eq!(
            capability.capture(),
            CapabilitySupport::Supported,
            "a discovered target is one this provider can capture"
        );
        assert_eq!(
            capability.capture_permission(),
            Some(PermissionKind::ScreenCapture),
            "the authorization capture requires is named, so a caller can read it \
             before opening anything"
        );
        assert!(
            description
                .coordinates()
                .supports(CoordinateSpace::DesktopLogical),
            "placement-bearing targets convert into the desktop plane"
        );

        // The scenario's own outcome: a caller asking about background delivery finds
        // it unavailable, before opening anything or causing an input side effect.
        // Asserted over every pair rather than the one a reader happens to think of,
        // so no operation kind can acquire a delivery mechanism unnoticed.
        let input = capability.input();
        assert!(!input.is_available());
        assert_eq!(input.permission(), None);
        for kind in InputOperationKind::ALL {
            for delivery in InputDelivery::ALL {
                assert!(
                    !input.supports(kind, delivery),
                    "a capture-only provider claimed {} over {}",
                    kind.as_str(),
                    delivery.as_str()
                );
            }
        }
    }

    #[test]
    fn a_whole_point_of_disagreement_is_a_resize_rather_than_quantization() {
        // The slack exists for pixel quantization, which is under a point at every
        // scale. A full point is two pixels at scale 2 — a resize — and accepting it
        // published a stale frame as covering a target it no longer fits, leaving the
        // right and bottom edges wrong by a logical point.
        let extent = PixelExtent::new(2560, 1600);

        let refused = placement_from_points((0.0, 0.0), (1281.0, 800.0), 2.0, extent)
            .expect_err("an exact point of disagreement describes another rectangle");
        assert_eq!(refused, GeometryFault::SpaceMismatch);

        // Under a point still passes, which is what the slack is for.
        placement_from_points((0.0, 0.0), (1280.4, 800.0), 2.0, extent)
            .expect("quantization below one point is the same rectangle");
    }

    #[test]
    fn a_retina_target_reports_two_capture_pixels_per_point() {
        let extent = PixelExtent::new(2560, 1600);
        let placement = placement_from_points((0.0, 0.0), (1280.0, 800.0), 2.0, extent)
            .expect("a doubled backing scale covers the frame");

        assert_eq!(placement.logical_size(), (1280.0, 800.0));
        assert_eq!(placement.scale().x(), 2.0);
        assert_eq!(
            placement.desktop_scale().x(),
            placement.scale().x(),
            "macOS measures the desktop in the same points as the target"
        );
    }

    #[test]
    fn a_signed_origin_survives_conversion_to_desktop_points() {
        let extent = PixelExtent::new(3840, 2160);
        let placement = placement_from_points((-1920.0, -240.0), (1920.0, 1080.0), 2.0, extent)
            .expect("a display left of the main one has a negative origin");
        let snapshot = TransformSnapshot::with_target(GeometryRevision::FIRST, extent, placement)
            .expect("the placement covers the frame");

        let frame_origin = Point::new(CoordinateSpace::CapturePixels, 0.0, 0.0).expect("valid");
        let desktop = snapshot
            .convert_point(frame_origin, CoordinateSpace::DesktopLogical)
            .expect("desktop conversion");

        assert_eq!((desktop.x(), desktop.y()), (-1920.0, -240.0));
        assert!(snapshot.covers_target());
    }

    #[test]
    fn differently_scaled_adjacent_displays_share_one_desktop_seam() {
        let retina_extent = PixelExtent::new(2560, 1600);
        let retina = placement_from_points((0.0, 0.0), (1280.0, 800.0), 2.0, retina_extent)
            .expect("retina placement");
        let plain_extent = PixelExtent::new(1920, 1080);
        let plain = placement_from_points((1280.0, 0.0), (1920.0, 1080.0), 1.0, plain_extent)
            .expect("unscaled placement");
        let retina_snapshot =
            TransformSnapshot::with_target(GeometryRevision::FIRST, retina_extent, retina)
                .expect("retina snapshot");
        let plain_snapshot =
            TransformSnapshot::with_target(GeometryRevision::FIRST, plain_extent, plain)
                .expect("plain snapshot");

        let retina_far = Point::new(CoordinateSpace::CapturePixels, 2560.0, 0.0).expect("point");
        let plain_near = Point::new(CoordinateSpace::CapturePixels, 0.0, 0.0).expect("point");

        assert_eq!(
            retina_snapshot
                .convert_point(retina_far, CoordinateSpace::DesktopLogical)
                .expect("retina conversion")
                .x(),
            plain_snapshot
                .convert_point(plain_near, CoordinateSpace::DesktopLogical)
                .expect("plain conversion")
                .x(),
            "the seam between the two displays is one point in the shared plane"
        );
    }

    #[test]
    fn a_live_size_that_has_moved_on_is_unsettled_rather_than_lost() {
        // The reading a cross-display drag produces: the window server has already
        // resized the target, so the live point size no longer matches the extent
        // the producer is still delivering. Reporting that as target loss ended a
        // stream whose target was plainly still there, which is the defect this
        // classification exists to prevent.
        let stale_extent = PixelExtent::new(1718, 1050);
        let live_points = (1718.0, 1108.0);

        let refused = placement_from_points((0.0, 0.0), live_points, 1.0, stale_extent)
            .expect_err("the extent and the live size describe different rectangles");

        assert_eq!(refused, GeometryFault::SpaceMismatch);
        assert_eq!(
            surface_for(live_points, 2.0),
            Some(PixelExtent::new(3436, 2216)),
            "the surface asked for is the live size at the new display's scale"
        );
    }

    #[test]
    fn a_surface_beyond_what_the_shim_accepts_is_never_asked_for() {
        assert_eq!(surface_for((1.0, 1.0), 1.0), Some(PixelExtent::new(1, 1)));
        assert_eq!(surface_for((0.4, 10.0), 1.0), None, "under one pixel");
        assert_eq!(
            surface_for((f64::from(MAX_SURFACE_EXTENT) + 1.0, 10.0), 1.0),
            None,
            "beyond the shim's own bound"
        );
        assert_eq!(surface_for((f64::NAN, 10.0), 1.0), None);
        assert_eq!(surface_for((10.0, 10.0), f64::INFINITY), None);

        // The extent bound is per axis and the allocation is their product, so a
        // surface can sit inside the first and outside the second.
        assert_eq!(
            surface_for((8192.0, 8192.0), 1.0),
            Some(PixelExtent::new(8192, 8192)),
            "exactly the byte ceiling"
        );
        assert_eq!(
            surface_for((8192.0, 8193.0), 1.0),
            None,
            "past the byte ceiling with both axes well inside the extent bound"
        );
    }

    #[test]
    fn only_a_window_carries_an_owner_into_the_native_lookup() {
        assert_eq!(
            Fingerprint::Window {
                owner_process: 4321
            }
            .native_owner(),
            4321,
            "the owner the discovery pass recorded is what an open must match"
        );
        assert_eq!(
            Fingerprint::Display {
                extent: PixelExtent::new(2560, 1600)
            }
            .native_owner(),
            0,
            "a display has no owning process"
        );
        // A window the framework named no owner for records zero, and the lookup
        // matches that value rather than treating it as no constraint — otherwise an
        // unnamed owner would stand in for every owner.
        assert_eq!(Fingerprint::Window { owner_process: 0 }.native_owner(), 0);
    }

    #[test]
    fn a_point_size_that_contradicts_the_captured_extent_is_refused() {
        let error = placement_from_points(
            (0.0, 0.0),
            (1280.0, 800.0),
            2.0,
            PixelExtent::new(1280, 800),
        )
        .expect_err("an unscaled extent cannot belong to a doubled scale");

        assert_eq!(error, GeometryFault::SpaceMismatch);
    }

    #[test]
    fn a_non_positive_backing_scale_is_refused() {
        assert_eq!(
            placement_from_points((0.0, 0.0), (10.0, 10.0), 0.0, PixelExtent::new(10, 10)),
            Err(GeometryFault::NegativeSize)
        );
        assert_eq!(
            placement_from_points((0.0, 0.0), (10.0, 10.0), f64::NAN, PixelExtent::new(10, 10)),
            Err(GeometryFault::NotFinite)
        );
    }

    #[test]
    fn a_reused_window_number_from_another_process_is_another_target() {
        let first = Fingerprint::Window { owner_process: 501 };
        let second = Fingerprint::Window { owner_process: 907 };

        assert_ne!(first, second);
        assert_eq!(first, Fingerprint::Window { owner_process: 501 });
    }

    #[test]
    fn a_display_mode_change_is_a_new_incarnation_and_a_move_is_not() {
        let original = Fingerprint::Display {
            extent: PixelExtent::new(2560, 1600),
        };
        let rescaled = Fingerprint::Display {
            extent: PixelExtent::new(1920, 1200),
        };

        assert_ne!(
            original, rescaled,
            "a mode change changes the shape of what capture produces"
        );
        assert_eq!(
            original,
            Fingerprint::Display {
                extent: PixelExtent::new(2560, 1600)
            },
            "rearranging displays moves the same display, so its identity persists"
        );
    }

    #[test]
    fn a_native_key_keeps_its_kind_and_identifier() {
        let window = NativeKey::Window(17);
        let display = NativeKey::Display(23);

        assert_eq!(window.native_id(), 17);
        assert_eq!(display.native_id(), 23);
        assert_ne!(window.native_kind(), display.native_kind());
        assert!(window.kind() != display.kind());
    }
}
