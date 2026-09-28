//! Picker-free Win32 window and display inventory.

use std::collections::HashSet;
use std::ffi::c_void;
use std::mem::size_of;

use mado_pilot_capture::{
    CaptureFault, CoordinateSupport, PixelFormat, TargetDescription, WindowCaptureArea,
    WindowGeometry,
};
use mado_pilot_core::{
    CapabilitySupport, PixelExtent, Result, Scale, TargetCapability, TargetId, TargetKind,
    TargetPlacement,
};
use windows::Graphics::Capture::GraphicsCaptureItem;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetClientRect, GetWindowDisplayAffinity, GetWindowTextLengthW,
    GetWindowTextW, IsIconic, IsWindow, IsWindowVisible,
};
use windows::core::BOOL;

use crate::availability::capture_item_factory;
use crate::input::input_capability;
use crate::optional_api::{logical_to_physical, monitor_scale, window_dpi};
use crate::storage::validate_surface;
use crate::window_authority::{RetainedWindowAuthority, WindowAuthorityStatus};

const DEFAULT_DPI: u32 = 96;

struct ThreadDpiContext(DPI_AWARENESS_CONTEXT);

impl ThreadDpiContext {
    fn per_monitor() -> Self {
        // SAFETY: this changes only the calling thread and returns the context
        // that Drop restores before control leaves the display query.
        Self(unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) })
    }
}

impl Drop for ThreadDpiContext {
    fn drop(&mut self) {
        // SAFETY: self.0 is exactly the context returned when this guard changed
        // the current thread, and restoration happens on that same thread.
        let _restored = unsafe { SetThreadDpiAwarenessContext(self.0) };
    }
}

/// Private lookup key. Public native-window keys are descriptive, never authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum NativeKey {
    Window(usize),
    Display(usize),
}

impl NativeKey {
    pub(crate) fn kind(self) -> TargetKind {
        match self {
            Self::Window(_) => TargetKind::Window,
            Self::Display(_) => TargetKind::Display,
        }
    }

    pub(crate) fn is_present(self) -> bool {
        match self {
            Self::Window(raw) => {
                // SAFETY: this reconstructs the opaque value returned by
                // EnumWindows solely for IsWindow validation.
                let hwnd = HWND(std::ptr::with_exposed_provenance_mut::<c_void>(raw));
                // SAFETY: IsWindow accepts an opaque HWND and performs no
                // dereference in this process.
                unsafe { IsWindow(Some(hwnd)).as_bool() }
            }
            Self::Display(raw) => monitor_handles().is_ok_and(|items| items.contains(&raw)),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TargetMetadata {
    pub(crate) name: String,
    pub(crate) class_name: Option<String>,
    pub(crate) extent: PixelExtent,
    pub(crate) placement: TargetPlacement,
}

impl TargetMetadata {
    pub(crate) fn describe(
        &self,
        id: TargetId,
        kind: TargetKind,
        window_message_authority: bool,
        extent: PixelExtent,
    ) -> TargetDescription {
        TargetDescription::new(
            id,
            self.name.clone(),
            extent,
            PixelFormat::Bgra8,
            CoordinateSupport::with_target_placement(),
        )
        .with_capability(TargetCapability::new(
            kind,
            CapabilitySupport::Supported,
            input_capability(kind, self.class_name.as_deref(), window_message_authority),
        ))
    }
}

pub(crate) enum CaptureItem {
    Native(GraphicsCaptureItem),
    #[cfg(test)]
    Synthetic(u64),
}

impl std::fmt::Debug for CaptureItem {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Native(_) => formatter.write_str("CaptureItem::Native"),
            #[cfg(test)]
            Self::Synthetic(identity) => formatter
                .debug_tuple("CaptureItem::Synthetic")
                .field(identity)
                .finish(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct Candidate {
    pub(crate) key: NativeKey,
    pub(crate) metadata: TargetMetadata,
    pub(crate) item: CaptureItem,
    pub(crate) authority: Option<RetainedWindowAuthority>,
}

pub(crate) fn inventory() -> Result<Vec<Candidate>> {
    let factory = capture_item_factory()?;
    let mut candidates = window_candidates(&factory)?;
    candidates.extend(display_candidates(&factory)?);
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

/// Re-reads placement at frame arrival so a retained frame never consults live
/// host geometry after publication.
pub(crate) fn current_placement(key: NativeKey, extent: PixelExtent) -> Option<TargetPlacement> {
    match key {
        NativeKey::Window(raw) => {
            let hwnd = HWND(std::ptr::with_exposed_provenance_mut::<c_void>(raw));
            window_placement(hwnd, extent)
        }
        NativeKey::Display(raw) => {
            let _dpi = ThreadDpiContext::per_monitor();
            let monitor = HMONITOR(std::ptr::with_exposed_provenance_mut::<c_void>(raw));
            let (_, bounds) = monitor_metadata(monitor)?;
            monitor_placement(monitor, bounds, extent).ok()
        }
    }
}

/// Reads geometry only for an already retained capture item. The caller fences
/// this observation with that item's Closed registration and process authority.
pub(crate) fn current_window_geometry(
    key: NativeKey,
    extent: PixelExtent,
) -> std::result::Result<WindowGeometry, CaptureFault> {
    let NativeKey::Window(raw) = key else {
        return Err(CaptureFault::UnsupportedOption);
    };
    let hwnd = HWND(std::ptr::with_exposed_provenance_mut::<c_void>(raw));
    let dpi = ThreadDpiContext::per_monitor();
    if dpi.0.0.is_null() {
        return Err(CaptureFault::UnsupportedOption);
    }
    // SAFETY: these calls inspect the retained opaque HWND without mutating it.
    if !unsafe { IsWindow(Some(hwnd)).as_bool() } {
        return Err(CaptureFault::TargetLost);
    }
    // SAFETY: no caller memory is accessed by these window state queries.
    if unsafe { IsIconic(hwnd).as_bool() || !IsWindowVisible(hwnd).as_bool() } {
        return Err(CaptureFault::SourceInvalid);
    }
    let mut affinity = 0;
    // SAFETY: affinity is a complete writable DWORD. Failure cannot establish
    // eligibility and is deliberately not treated as unprotected.
    let observed_affinity = unsafe { GetWindowDisplayAffinity(hwnd, &raw mut affinity) }
        .ok()
        .map(|()| affinity);
    validate_affinity(observed_affinity)?;
    let mut cloaked = 0u32;
    let mut bounds = RECT::default();
    // SAFETY: both outputs match the exact DWM attribute size.
    unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&raw mut cloaked).cast::<c_void>(),
            u32::try_from(size_of::<u32>()).expect("DWORD size fits u32"),
        )
    }
    .map_err(|_| CaptureFault::UnsupportedOption)?;
    if cloaked != 0 {
        return Err(CaptureFault::SourceInvalid);
    }
    // SAFETY: bounds is a complete writable RECT.
    unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&raw mut bounds).cast::<c_void>(),
            u32::try_from(size_of::<RECT>()).expect("RECT size fits u32"),
        )
    }
    .map_err(|_| CaptureFault::UnsupportedOption)?;
    let mut client = RECT::default();
    // SAFETY: client is a complete writable RECT.
    unsafe { GetClientRect(hwnd, &raw mut client) }.map_err(|_| CaptureFault::UnsupportedOption)?;
    let mut origin = POINT {
        x: client.left,
        y: client.top,
    };
    let mut far = POINT {
        x: client.right,
        y: client.bottom,
    };
    // SAFETY: both POINT values are writable; per-monitor awareness makes these
    // physical virtual-desktop coordinates, including negative origins.
    if !unsafe { ClientToScreen(hwnd, &raw mut origin) }.as_bool()
        || !unsafe { ClientToScreen(hwnd, &raw mut far) }.as_bool()
    {
        return Err(CaptureFault::UnsupportedOption);
    }
    let client = RECT {
        left: origin.x,
        top: origin.y,
        right: far.x,
        bottom: far.y,
    };
    let dpi = window_dpi(hwnd)
        .filter(|dpi| *dpi != 0)
        .ok_or(CaptureFault::UnsupportedOption)?;
    let scale = f64::from(dpi) / f64::from(DEFAULT_DPI);
    let scale = Scale::new(scale, scale).map_err(|_| CaptureFault::SourceInvalid)?;
    geometry_from_rectangles(bounds, client, extent, scale)
}

fn validate_affinity(affinity: Option<u32>) -> std::result::Result<(), CaptureFault> {
    match affinity {
        Some(0) => Ok(()),
        Some(_) => Err(CaptureFault::AccessDenied),
        // GetWindowDisplayAffinity only documents success for layered windows
        // under DWM composition. Ordinary/unverifiable windows may refuse too.
        None => Err(CaptureFault::UnsupportedOption),
    }
}

fn geometry_from_rectangles(
    extended: RECT,
    client: RECT,
    extent: PixelExtent,
    scale: Scale,
) -> std::result::Result<WindowGeometry, CaptureFault> {
    let matches = |rect: RECT| {
        rect.right
            .checked_sub(rect.left)
            .and_then(|width| u32::try_from(width).ok())
            == Some(extent.width())
            && rect
                .bottom
                .checked_sub(rect.top)
                .and_then(|height| u32::try_from(height).ok())
                == Some(extent.height())
    };
    if matches(extended)
        && matches(client)
        && (extended.left != client.left || extended.top != client.top)
    {
        return Err(CaptureFault::UnsupportedOption);
    }
    let (area, rect) = if matches(extended) {
        (WindowCaptureArea::WindowsExtendedFrame, extended)
    } else if matches(client) {
        (WindowCaptureArea::WindowsClient, client)
    } else {
        // Never infer an origin or scale a mismatched WGC content rectangle.
        return Err(CaptureFault::UnsupportedOption);
    };
    let placement = placement_with_scale(rect.left, rect.top, extent, scale)
        .map_err(|_| CaptureFault::SourceInvalid)?;
    Ok(WindowGeometry::new(area, placement, extent))
}

fn window_candidates(factory: &IGraphicsCaptureItemInterop) -> Result<Vec<Candidate>> {
    let mut candidates = Vec::new();
    for raw in window_handles()? {
        // SAFETY: the value came directly from EnumWindows in this inventory
        // pass and is used only through Win32 and WGC handle-taking APIs.
        let hwnd = HWND(std::ptr::with_exposed_provenance_mut::<c_void>(raw));
        if !window_is_discoverable(hwnd) {
            continue;
        }
        let Some(name) = window_title(hwnd) else {
            continue;
        };
        let class_name = window_class(hwnd);
        // Retain the owner before WGC binds this HWND. A later capture cannot
        // establish provenance for an item that may already name an old window.
        let authority =
            RetainedWindowAuthority::capture(NativeKey::Window(raw), class_name.as_deref());
        // SAFETY: factory is the documented GraphicsCaptureItem desktop interop
        // factory, and hwnd was just validated. A protected/uncapturable window
        // is filtered by the returned error without prompting.
        let Ok(item) = (unsafe { factory.CreateForWindow::<GraphicsCaptureItem>(hwnd) }) else {
            continue;
        };
        let Ok(size) = item.Size() else {
            continue;
        };
        let Some(extent) = positive_extent(size.Width, size.Height) else {
            continue;
        };
        let Some(placement) = window_placement(hwnd, extent) else {
            continue;
        };
        // Never join the capture item to authority observed after a replacement.
        // Missing or changed authority leaves capture available without identity.
        let authority =
            authority.filter(|authority| authority.status() == WindowAuthorityStatus::SameTarget);

        candidates.push(Candidate {
            key: NativeKey::Window(raw),
            metadata: TargetMetadata {
                name,
                class_name,
                extent,
                placement,
            },
            item: CaptureItem::Native(item),
            authority,
        });
    }
    Ok(candidates)
}

fn display_candidates(factory: &IGraphicsCaptureItemInterop) -> Result<Vec<Candidate>> {
    let _dpi = ThreadDpiContext::per_monitor();
    let mut candidates = Vec::new();
    for raw in monitor_handles()? {
        // SAFETY: the value came directly from EnumDisplayMonitors.
        let monitor = HMONITOR(std::ptr::with_exposed_provenance_mut::<c_void>(raw));
        let Some((device_name, bounds)) = monitor_metadata(monitor) else {
            continue;
        };
        // SAFETY: factory is the desktop capture-item interop factory and the
        // monitor is from the current enumeration. Failure filters a display
        // without presenting UI.
        let Ok(item) = (unsafe { factory.CreateForMonitor::<GraphicsCaptureItem>(monitor) }) else {
            continue;
        };
        let Ok(size) = item.Size() else {
            continue;
        };
        let Some(extent) = positive_extent(size.Width, size.Height) else {
            continue;
        };
        let placement = monitor_placement(monitor, bounds, extent)?;
        candidates.push(Candidate {
            key: NativeKey::Display(raw),
            metadata: TargetMetadata {
                name: device_name,
                class_name: None,
                extent,
                placement,
            },
            item: CaptureItem::Native(item),
            authority: None,
        });
    }
    Ok(candidates)
}

fn window_handles() -> Result<Vec<usize>> {
    let mut handles = Vec::<usize>::new();
    let pointer = (&raw mut handles).addr().cast_signed();
    // SAFETY: the LPARAM points to handles for the duration of this synchronous
    // enumeration. The callback only appends opaque HWND values.
    unsafe { EnumWindows(Some(collect_window), LPARAM(pointer)) }
        .map_err(|_| CaptureFault::SourceInvalid)?;
    Ok(handles)
}

unsafe extern "system" fn collect_window(hwnd: HWND, data: LPARAM) -> BOOL {
    let pointer = std::ptr::with_exposed_provenance_mut::<Vec<usize>>(data.0.cast_unsigned());
    // SAFETY: window_handles supplied this pointer and EnumWindows invokes the
    // callback synchronously before that vector leaves scope.
    unsafe { &mut *pointer }.push(hwnd.0.addr());
    true.into()
}

fn monitor_handles() -> Result<HashSet<usize>> {
    let mut handles = HashSet::<usize>::new();
    let pointer = (&raw mut handles).addr().cast_signed();
    // SAFETY: LPARAM remains valid for the synchronous enumeration and the
    // callback records only the opaque monitor value.
    let success =
        unsafe { EnumDisplayMonitors(None, None, Some(collect_monitor), LPARAM(pointer)) };
    if !success.as_bool() {
        return Err(CaptureFault::SourceInvalid.into());
    }
    Ok(handles)
}

unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _device_context: HDC,
    _bounds: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let pointer = std::ptr::with_exposed_provenance_mut::<HashSet<usize>>(data.0.cast_unsigned());
    // SAFETY: monitor_handles supplied the pointer for this synchronous call.
    unsafe { &mut *pointer }.insert(monitor.0.addr());
    true.into()
}

fn window_is_discoverable(hwnd: HWND) -> bool {
    // SAFETY: hwnd is an opaque value produced by EnumWindows.
    if !(unsafe { IsWindow(Some(hwnd)).as_bool() && IsWindowVisible(hwnd).as_bool() }) {
        return false;
    }
    let mut cloaked = 0u32;
    // SAFETY: cloaked points to a u32 of exactly the size requested by
    // DWMWA_CLOAKED. A failure is treated conservatively as not cloaked.
    let result = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&raw mut cloaked).cast::<c_void>(),
            u32::try_from(size_of::<u32>()).expect("u32 size fits u32"),
        )
    };
    result.is_err() || cloaked == 0
}

fn window_title(hwnd: HWND) -> Option<String> {
    // SAFETY: hwnd is valid for the duration of the current enumeration.
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    let capacity = usize::try_from(length).ok()?.checked_add(1)?;
    if capacity <= 1 {
        return None;
    }
    let mut buffer = vec![0u16; capacity];
    // SAFETY: buffer is writable and includes room for the terminator.
    let written = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    let written = usize::try_from(written).ok()?;
    (written > 0).then(|| String::from_utf16_lossy(&buffer[..written]))
}

fn window_class(hwnd: HWND) -> Option<String> {
    // Win32 window class names are bounded to 256 UTF-16 code units including
    // the terminator. A failed query disables class-specific `WindowMessage`
    // capability but does not make system input or capture disappear.
    let mut buffer = [0u16; 256];
    // SAFETY: buffer is writable for the duration of the call and hwnd came from
    // the current EnumWindows snapshot.
    let written = unsafe { GetClassNameW(hwnd, &mut buffer) };
    let written = usize::try_from(written).ok()?;
    (written > 0).then(|| String::from_utf16_lossy(&buffer[..written]))
}

fn window_placement(hwnd: HWND, extent: PixelExtent) -> Option<TargetPlacement> {
    let mut client = RECT::default();
    // SAFETY: client is writable and hwnd is from the current enumeration.
    if unsafe { GetClientRect(hwnd, &raw mut client) }.is_err() {
        return None;
    }
    let logical_width = client.right.checked_sub(client.left)?;
    let logical_height = client.bottom.checked_sub(client.top)?;
    if logical_width <= 0 || logical_height <= 0 {
        return None;
    }

    let mut client_origin = POINT::default();
    let mut client_far = POINT {
        x: logical_width,
        y: logical_height,
    };
    // SAFETY: client_origin is writable and hwnd is a current window.
    let origin_converted = unsafe { ClientToScreen(hwnd, &raw mut client_origin) }.as_bool();
    // SAFETY: client_far is writable and hwnd is a current window.
    let far_converted = unsafe { ClientToScreen(hwnd, &raw mut client_far) }.as_bool();
    if !origin_converted || !far_converted {
        return None;
    }
    let converted =
        logical_to_physical(hwnd, &mut client_origin) && logical_to_physical(hwnd, &mut client_far);
    let scale = if converted {
        let physical_width = client_far.x.checked_sub(client_origin.x)?;
        let physical_height = client_far.y.checked_sub(client_origin.y)?;
        if physical_width <= 0 || physical_height <= 0 {
            return None;
        }
        Scale::new(
            f64::from(physical_width) / f64::from(logical_width),
            f64::from(physical_height) / f64::from(logical_height),
        )
        .ok()?
    } else {
        let dpi = window_dpi(hwnd).unwrap_or(DEFAULT_DPI);
        let scale = f64::from(dpi) / f64::from(DEFAULT_DPI);
        Scale::new(scale, scale).ok()?
    };

    let mut bounds = RECT::default();
    // DWM extended-frame bounds are physical and are not DPI-virtualized. WGC
    // captures that visible surface, so this origin avoids the client area's
    // title-bar offset.
    // SAFETY: bounds points to a complete RECT and hwnd is current.
    let physical_origin = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&raw mut bounds).cast::<c_void>(),
            u32::try_from(size_of::<RECT>()).expect("RECT size fits u32"),
        )
    }
    .map_or(client_origin, |()| POINT {
        x: bounds.left,
        y: bounds.top,
    });
    placement_with_scale(physical_origin.x, physical_origin.y, extent, scale).ok()
}

fn monitor_metadata(monitor: HMONITOR) -> Option<(String, RECT)> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = u32::try_from(size_of::<MONITORINFOEXW>()).ok()?;
    // SAFETY: MONITORINFOEXW begins with MONITORINFO as required by
    // GetMonitorInfoW, and monitor comes from the current enumeration.
    if !unsafe { GetMonitorInfoW(monitor, &raw mut info.monitorInfo) }.as_bool() {
        return None;
    }
    let length = info
        .szDevice
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(info.szDevice.len());
    let name = String::from_utf16_lossy(&info.szDevice[..length]);
    (!name.is_empty()).then_some((name, info.monitorInfo.rcMonitor))
}

fn monitor_placement(
    monitor: HMONITOR,
    bounds: RECT,
    extent: PixelExtent,
) -> Result<TargetPlacement> {
    let scale = monitor_scale(monitor).ok_or(CaptureFault::UnsupportedOption)?;
    let scale = Scale::new(scale, scale).map_err(|_| CaptureFault::SourceInvalid)?;
    placement_with_scale(bounds.left, bounds.top, extent, scale)
        .map_err(|_| CaptureFault::SourceInvalid.into())
}

fn placement_with_scale(
    physical_x: i32,
    physical_y: i32,
    extent: PixelExtent,
    scale: Scale,
) -> std::result::Result<TargetPlacement, mado_pilot_core::GeometryFault> {
    let desktop_scale = Scale::new(1.0, 1.0)?;
    TargetPlacement::new(
        (f64::from(physical_x), f64::from(physical_y)),
        (
            f64::from(extent.width()) / scale.x(),
            f64::from(extent.height()) / scale.y(),
        ),
        scale,
    )
    .map(|placement| placement.with_desktop_scale(desktop_scale))
}

fn positive_extent(width: i32, height: i32) -> Option<PixelExtent> {
    let width = u32::try_from(width).ok().filter(|value| *value > 0)?;
    let height = u32::try_from(height).ok().filter(|value| *value > 0)?;
    validate_surface(width, height).ok()?;
    Some(PixelExtent::new(width, height))
}

#[cfg(test)]
mod tests {
    use super::{placement_with_scale, positive_extent};
    use mado_pilot_core::{
        CoordinateSpace, GeometryRevision, PixelExtent, Point, Scale, TransformSnapshot,
    };

    #[test]
    fn signed_mixed_dpi_placement_preserves_virtual_screen_coordinates() {
        let extent = PixelExtent::new(1920, 1080);
        let placement =
            placement_with_scale(-1920, -120, extent, Scale::new(1.5, 1.5).expect("scale"))
                .expect("valid");
        let snapshot = TransformSnapshot::with_target(GeometryRevision::FIRST, extent, placement)
            .expect("placement covers the frame");
        let frame_origin = Point::new(CoordinateSpace::CapturePixels, 0.0, 0.0).expect("valid");
        let desktop_origin = snapshot
            .convert_point(frame_origin, CoordinateSpace::DesktopLogical)
            .expect("desktop conversion");

        assert_eq!(placement.desktop_origin(), (-1920.0, -120.0));
        assert_eq!((desktop_origin.x(), desktop_origin.y()), (-1920.0, -120.0));
        assert!(snapshot.covers_target());
    }

    #[test]
    fn differently_scaled_adjacent_monitors_share_one_desktop_seam() {
        let primary_extent = PixelExtent::new(1920, 1080);
        let primary =
            placement_with_scale(0, 0, primary_extent, Scale::new(1.0, 1.0).expect("scale"))
                .expect("primary placement");
        let scaled_extent = PixelExtent::new(1920, 1080);
        let scaled =
            placement_with_scale(1920, 0, scaled_extent, Scale::new(1.5, 1.5).expect("scale"))
                .expect("scaled placement");
        let primary_snapshot =
            TransformSnapshot::with_target(GeometryRevision::FIRST, primary_extent, primary)
                .expect("primary snapshot");
        let scaled_snapshot =
            TransformSnapshot::with_target(GeometryRevision::FIRST, scaled_extent, scaled)
                .expect("scaled snapshot");
        let primary_far = Point::new(CoordinateSpace::CapturePixels, 1920.0, 0.0).expect("point");
        let scaled_near = Point::new(CoordinateSpace::CapturePixels, 0.0, 0.0).expect("point");

        assert_eq!(
            primary_snapshot
                .convert_point(primary_far, CoordinateSpace::DesktopLogical)
                .expect("primary conversion")
                .x(),
            scaled_snapshot
                .convert_point(scaled_near, CoordinateSpace::DesktopLogical)
                .expect("scaled conversion")
                .x()
        );
        assert_eq!(scaled.logical_size(), (1280.0, 720.0));
    }

    #[test]
    fn empty_or_negative_native_sizes_are_filtered() {
        assert_eq!(positive_extent(0, 10), None);
        assert_eq!(positive_extent(10, -1), None);
        assert_eq!(positive_extent(10, 20), Some(PixelExtent::new(10, 20)));
    }

    #[test]
    fn retained_geometry_distinguishes_extended_frame_from_client_without_guessing() {
        use mado_pilot_capture::{CaptureFault, NativeDesktopUnit, WindowCaptureArea};
        use windows::Win32::Foundation::RECT;

        let extended = RECT {
            left: -1400,
            top: -200,
            right: -1120,
            bottom: 0,
        };
        let client = RECT {
            left: -1390,
            top: -170,
            right: -1130,
            bottom: -10,
        };
        let scale = Scale::new(1.25, 1.25).expect("scale");
        let frame =
            super::geometry_from_rectangles(extended, client, PixelExtent::new(280, 200), scale)
                .expect("exact extended frame");
        assert_eq!(frame.area(), WindowCaptureArea::WindowsExtendedFrame);
        assert_eq!(
            frame.desktop_unit(),
            NativeDesktopUnit::WindowsPhysicalPixels
        );
        assert_eq!(frame.placement().desktop_origin(), (-1400.0, -200.0));
        assert_eq!(frame.placement().logical_size(), (224.0, 160.0));

        let client_geometry =
            super::geometry_from_rectangles(extended, client, PixelExtent::new(260, 160), scale)
                .expect("exact client");
        assert_eq!(client_geometry.area(), WindowCaptureArea::WindowsClient);
        assert_eq!(
            client_geometry.placement().desktop_origin(),
            (-1390.0, -170.0)
        );
        let snapshot = TransformSnapshot::with_target(
            GeometryRevision::FIRST,
            client_geometry.extent(),
            client_geometry.placement(),
        )
        .expect("client transform");
        let far = snapshot
            .convert_point(
                Point::new(CoordinateSpace::CapturePixels, 260.0, 160.0).expect("far corner"),
                CoordinateSpace::DesktopLogical,
            )
            .expect("physical desktop conversion");
        assert_eq!((far.x(), far.y()), (-1130.0, -10.0));
        assert_eq!(
            super::geometry_from_rectangles(extended, client, PixelExtent::new(270, 180), scale),
            Err(CaptureFault::UnsupportedOption),
        );
        let shifted = RECT {
            left: -1390,
            top: -190,
            right: -1110,
            bottom: 10,
        };
        assert_eq!(
            super::geometry_from_rectangles(extended, shifted, PixelExtent::new(280, 200), scale),
            Err(CaptureFault::UnsupportedOption),
            "same size at different origins does not identify the captured rectangle",
        );
    }

    #[test]
    fn unknown_affinity_is_not_mistaken_for_unprotected_capture() {
        use mado_pilot_capture::CaptureFault;

        assert_eq!(super::validate_affinity(Some(0)), Ok(()));
        assert_eq!(
            super::validate_affinity(Some(1)),
            Err(CaptureFault::AccessDenied)
        );
        assert_eq!(
            super::validate_affinity(Some(0x11)),
            Err(CaptureFault::AccessDenied)
        );
        assert_eq!(
            super::validate_affinity(None),
            Err(CaptureFault::UnsupportedOption)
        );
    }
}
