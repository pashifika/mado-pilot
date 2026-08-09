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
    CapabilitySupport, GeometryFault, PermissionKind, PixelExtent, Result, Scale, TargetCapability,
    TargetId, TargetKind, TargetPlacement,
};

use crate::input::input_capability;
use crate::shim::{self, FrameInfo, Inventory, KIND_DISPLAY, KIND_WINDOW, ShimStatus, TargetToken};

/// The native descriptive key used for ordering and request validation.
/// It is never exposed through a public contract or used to re-resolve a filter.
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

    fn from_info(kind: u32, native_id: u64) -> Option<Self> {
        let identifier = u32::try_from(native_id).ok()?;
        match kind {
            KIND_WINDOW => Some(NativeKey::Window(identifier)),
            KIND_DISPLAY => Some(NativeKey::Display(identifier)),
            _ => None,
        }
    }
}

/// Native metadata repeated to validate an originating snapshot selection.
///
/// PID and window number can both be reused by one process, so these values are
/// not an incarnation identity. [`Candidate::target`] retains the filter built
/// from this snapshot. Capture consumes that filter directly; input uses the
/// metadata only to narrow a fresh search whose logical `SCWindow` must equal the
/// retained one.
///
/// A display carries its captured extent rather than its placement because the
/// extent supplies the opening producer size and frame placement is same-frame
/// metadata. Vendor or serial values describe hardware and cannot strengthen
/// either retained-filter capture or current-display validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fingerprint {
    Window { owner_process: i64 },
    Display { extent: PixelExtent },
}

impl Fingerprint {
    /// Returns the owning process an open must match, or zero for a display.
    ///
    /// A window always carries a real process here, because the shim lists no window
    /// whose owner the framework did not name, and an open refuses a non-positive owner
    /// for a window outright. Zero reaching a window request therefore means an invented
    /// identity or one recorded before that rule, and it is refused rather than matched —
    /// two unknown owners compared equal, which is the recycled-number capture this is
    /// supposed to prevent. Zero for a display is not consulted at all.
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
    #[allow(dead_code)] // Read by the authorized-host screenRect acceptance matrix.
    pub(crate) placement: TargetPlacement,
}

impl TargetMetadata {
    /// Describes the target as this provider can act on it.
    ///
    /// The capability states what this Adapter implements and nothing more:
    /// capture through ScreenCaptureKit, every coordinate space the placement
    /// supports, and `CGEvent` system input under Accessibility. Background
    /// delivery is absent because macOS offers no per-window channel an unfocused
    /// process may post to, and advertising one would be a claim without an
    /// implementation.
    pub(crate) fn describe(&self, id: TargetId, kind: TargetKind) -> TargetDescription {
        TargetDescription::new(
            id,
            self.name.clone(),
            self.extent,
            PixelFormat::Bgra8,
            // The qualified host supplies onscreen placement in each frame's own
            // attachment dictionary. Inventory placement describes discovery;
            // only the per-frame rectangle authorizes publication.
            CoordinateSupport::with_target_placement(),
        )
        .with_capability(
            TargetCapability::new(kind, CapabilitySupport::Supported, input_capability(kind))
                .with_capture_permission(PermissionKind::ScreenCapture),
        )
    }
}

/// One target a discovery pass observed, with everything an open needs.
#[derive(Debug, Clone)]
pub(crate) struct Candidate {
    pub(crate) key: NativeKey,
    pub(crate) fingerprint: Fingerprint,
    pub(crate) target: TargetToken,
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
        let Ok(target) = inventory.target(index) else {
            continue;
        };
        candidates.push(Candidate {
            key,
            fingerprint: fingerprint(key, &info, extent),
            target,
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

/// Builds the placement for a target whose logical rectangle is `size` points at
/// `origin`, captured at `scale` pixels per point.
///
/// The logical size comes from the captured extent rather than from the reported
/// point size. Both describe the same rectangle, but only the extent is what the
/// frame actually contains, and a placement whose scaled logical size is not the
/// frame extent is refused by the transform snapshot that consumes it.
pub(crate) fn placement_from_points(
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

/// Builds the only placement a native publication may carry.
///
/// `screenRect`, content extent, and effective scale all come from the same sample
/// buffer. The shim has already checked them, and this second validation keeps the
/// Rust boundary independently defensive if the native report layout changes.
pub(crate) fn frame_placement(
    info: &FrameInfo,
) -> std::result::Result<TargetPlacement, CaptureFault> {
    let extent = info.extent().ok_or(CaptureFault::InconsistentDescriptor)?;
    let (origin, size) = info
        .screen_rect()
        .ok_or(CaptureFault::InconsistentDescriptor)?;
    placement_from_points(origin, size, info.scale_factor, extent)
        .map_err(|_| CaptureFault::InconsistentDescriptor)
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

    use super::{Fingerprint, NativeKey, TargetMetadata, frame_placement, placement_from_points};

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
    fn a_described_target_offers_capture_and_system_input_but_never_background_delivery() {
        // The negative half of this is the one that reverses quietly. macOS offers
        // no per-window channel an unfocused process may post to, so a background
        // pair appearing here would be a capability claim with nothing behind it,
        // and admission would then route a caller that asked not to disturb the
        // desktop into system input that focuses a window.
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
                .supports(CoordinateSpace::TargetNormalized),
            "the frame covers the selected target"
        );
        assert!(
            description
                .coordinates()
                .supports(CoordinateSpace::DesktopLogical),
            "qualified-host frame attachments provide same-frame desktop placement"
        );

        let input = capability.input();
        assert_eq!(
            input.permission(),
            Some(PermissionKind::InputControl),
            "Accessibility is the authorization input needs, named separately from \
             the one capture needs"
        );
        for kind in InputOperationKind::ALL {
            assert!(
                input.supports(kind, InputDelivery::System),
                "a window accepts {} through system delivery",
                kind.as_str()
            );
            assert!(
                !input.supports(kind, InputDelivery::BackgroundTarget),
                "a macOS target claimed background {}",
                kind.as_str()
            );
        }
        assert!(
            input.requires_focus(InputDelivery::System),
            "system delivery reaches whatever is focused, so it needs the target to be"
        );
    }

    #[test]
    fn a_display_accepts_pointer_input_and_nothing_that_needs_focus() {
        let issuer = IdentityIssuer::new();
        let id = issuer
            .issue_target(crate::provider::PROVIDER)
            .expect("issued");

        let input = metadata()
            .describe(id, TargetKind::Display)
            .capability()
            .input();

        assert!(input.supports(InputOperationKind::Pointer, InputDelivery::System));
        for kind in [InputOperationKind::Keyboard, InputOperationKind::Text] {
            assert!(
                !input.supports(kind, InputDelivery::System),
                "a display is not a focusable target, so {} has nothing to reach",
                kind.as_str()
            );
        }
        assert!(
            !input.requires_focus(InputDelivery::System),
            "pointer input to a display needs nothing focused"
        );
        for kind in InputOperationKind::ALL {
            assert!(!input.supports(kind, InputDelivery::BackgroundTarget));
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
    fn frame_placement_accepts_only_geometry_attached_to_that_frame() {
        let extent = PixelExtent::new(2560, 1600);
        let valid = crate::shim::FrameInfo::testing_screen_rect(
            extent,
            2.0,
            (-1280.0, 0.0),
            (1280.0, 800.0),
        );
        let placement = frame_placement(&valid).expect("same-frame geometry agrees");
        assert_eq!(placement.desktop_origin(), (-1280.0, 0.0));

        let missing = crate::shim::FrameInfo::empty();
        assert_eq!(
            frame_placement(&missing),
            Err(mado_pilot_capture::CaptureFault::InconsistentDescriptor)
        );

        let contradictory =
            crate::shim::FrameInfo::testing_screen_rect(extent, 2.0, (0.0, 0.0), (1281.0, 800.0));
        assert_eq!(
            frame_placement(&contradictory),
            Err(mado_pilot_capture::CaptureFault::InconsistentDescriptor)
        );
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
    fn a_same_frame_size_that_contradicts_its_extent_is_refused() {
        let stale_extent = PixelExtent::new(1718, 1050);
        let frame_points = (1718.0, 1108.0);

        let refused = placement_from_points((0.0, 0.0), frame_points, 1.0, stale_extent)
            .expect_err("the attached rectangle and extent describe different rectangles");

        assert_eq!(refused, GeometryFault::SpaceMismatch);
    }

    #[test]
    fn only_a_window_repeats_an_owner_in_the_native_request() {
        assert_eq!(
            Fingerprint::Window {
                owner_process: 4321
            }
            .native_owner(),
            4321,
            "the owner the discovery pass recorded is repeated with its filter"
        );
        assert_eq!(
            Fingerprint::Display {
                extent: PixelExtent::new(2560, 1600)
            }
            .native_owner(),
            0,
            "a display has no owning process"
        );
        // The metadata conversion itself preserves zero; discovery omits such a
        // window and the native open boundary independently refuses it.
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
    fn different_window_owner_metadata_remains_distinguishable() {
        let first = Fingerprint::Window { owner_process: 501 };
        let second = Fingerprint::Window { owner_process: 907 };

        assert_ne!(first, second);
        assert_eq!(first, Fingerprint::Window { owner_process: 501 });
    }

    #[test]
    fn a_display_fingerprint_tracks_opening_extent_and_not_placement() {
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
            "placement is intentionally absent from opening metadata"
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
