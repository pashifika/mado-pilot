//! Native window metadata describes a retained target; it never grants authority.
use std::num::{NonZeroU32, NonZeroU64};

use mado_pilot_core::{Error, PixelExtent, Status, TargetPlacement};

/// A descriptive native key, valid only with its provider's retained `TargetId`.
#[derive(Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeWindowId {
    /// A macOS window-server number, not an incarnation identity.
    Macos(NonZeroU32),
    /// A Windows HWND value, not an ownership handle or incarnation identity.
    Windows(NonZeroU64),
}

impl std::fmt::Debug for NativeWindowId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Macos(_) => "MacosWindow",
            Self::Windows(_) => "WindowsWindow",
        })
    }
}

/// Which rectangle the provider's captured pixels actually cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum WindowCaptureArea {
    /// The retained ScreenCaptureKit window rectangle.
    MacosWindow,
    /// The Windows extended frame rectangle, excluding invisible resize borders.
    WindowsExtendedFrame,
    /// The Windows client rectangle in desktop coordinates.
    WindowsClient,
}

/// Units of the desktop origin and capture rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeDesktopUnit {
    /// Quartz global top-left desktop points.
    MacosPoints,
    /// Per-monitor-aware physical virtual-desktop pixels.
    WindowsPhysicalPixels,
}

/// Exact selected capture geometry, including signed origin and independent scales.
#[derive(Clone, Copy, PartialEq)]
pub struct WindowGeometry {
    area: WindowCaptureArea,
    placement: TargetPlacement,
    extent: PixelExtent,
}

impl std::fmt::Debug for WindowGeometry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WindowGeometry")
            .field("area", &self.area)
            .field("extent", &self.extent)
            .finish_non_exhaustive()
    }
}

// TargetPlacement and Scale constructors reject every non-finite component.
impl Eq for WindowGeometry {}

impl WindowGeometry {
    /// Records authoritative provider geometry; no coordinate conversion is guessed.
    #[must_use]
    pub const fn new(
        area: WindowCaptureArea,
        placement: TargetPlacement,
        extent: PixelExtent,
    ) -> Self {
        Self {
            area,
            placement,
            extent,
        }
    }
    /// Returns the captured area kind.
    #[must_use]
    pub const fn area(self) -> WindowCaptureArea {
        self.area
    }
    /// Returns the coordinate units used by the desktop placement.
    #[must_use]
    pub const fn desktop_unit(self) -> NativeDesktopUnit {
        match self.area {
            WindowCaptureArea::MacosWindow => NativeDesktopUnit::MacosPoints,
            WindowCaptureArea::WindowsExtendedFrame | WindowCaptureArea::WindowsClient => {
                NativeDesktopUnit::WindowsPhysicalPixels
            }
        }
    }
    /// Returns the selected target/desktop placement and pixel scales.
    #[must_use]
    pub const fn placement(self) -> TargetPlacement {
        self.placement
    }
    /// Returns the original capture-pixel extent.
    #[must_use]
    pub const fn extent(self) -> PixelExtent {
        self.extent
    }
}

/// Revalidated native correspondence for the same retained target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeWindowDescription {
    id: NativeWindowId,
    geometry: WindowGeometry,
}

impl NativeWindowDescription {
    /// Associates a descriptive key with its authoritative capture geometry.
    #[must_use]
    pub const fn new(id: NativeWindowId, geometry: WindowGeometry) -> Self {
        Self { id, geometry }
    }
    /// Returns the descriptive key; it must not be used to reconstruct target authority.
    #[must_use]
    pub const fn id(self) -> NativeWindowId {
        self.id
    }
    /// Returns this observation's exact capture geometry.
    #[must_use]
    pub const fn geometry(self) -> WindowGeometry {
        self.geometry
    }
}

/// Required ceilings for one native session's accounted image payload.
///
/// Producer, detached, staging and CPU payload remain charged while retained.
/// Known linear row padding counts. Opaque OS/GPU layouts are not allocation or
/// RSS guarantees: producer/staging padding is admitted when first observable,
/// before accepted publication or CPU copying. Controlled allocations reserve
/// their bytes before allocation. Platform/global ceilings still apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureResourceLimits {
    max_frame_bytes: NonZeroU64,
    max_retained_bytes: NonZeroU64,
}

impl CaptureResourceLimits {
    /// Constructs nonzero byte ceilings.
    ///
    /// # Errors
    /// Returns `InvalidArgument` if either ceiling is zero.
    pub fn new(max_frame_bytes: u64, max_retained_bytes: u64) -> Result<Self, Error> {
        let invalid = || {
            Error::new(
                Status::InvalidArgument,
                "capture resource limits must be nonzero",
            )
        };
        Ok(Self {
            max_frame_bytes: NonZeroU64::new(max_frame_bytes).ok_or_else(invalid)?,
            max_retained_bytes: NonZeroU64::new(max_retained_bytes).ok_or_else(invalid)?,
        })
    }
    /// Returns the maximum accounted bytes in one frame, including observed padding.
    #[must_use]
    pub const fn max_frame_bytes(self) -> u64 {
        self.max_frame_bytes.get()
    }
    /// Returns the maximum simultaneously retained image bytes for this session.
    #[must_use]
    pub const fn max_retained_bytes(self) -> u64 {
        self.max_retained_bytes.get()
    }
}
