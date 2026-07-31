//! Picker-free Win32 window and display inventory.

use std::collections::HashSet;
use std::ffi::c_void;
use std::mem::size_of;

use mado_pilot_capture::{CaptureFault, CoordinateSupport, PixelFormat, TargetDescription};
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
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetClientRect, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsWindow, IsWindowVisible,
};
use windows::core::BOOL;

use crate::availability::capture_item_factory;
use crate::optional_api::{logical_to_physical, monitor_scale, window_dpi};

const DEFAULT_DPI: u32 = 96;
const MAX_CLASS_NAME: usize = 256;

/// The stable native lookup key. It is never exposed through a public contract.
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

/// Native observations that distinguish a window incarnation from an obvious
/// handle replacement. The retained WGC item and its Closed event provide the
/// authoritative lifetime signal; this fingerprint rejects replacement before
/// that event is delivered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Fingerprint {
    Window {
        process_id: u32,
        thread_id: u32,
        class_name: String,
    },
    Display {
        device_name: String,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct TargetMetadata {
    pub(crate) name: String,
    pub(crate) extent: PixelExtent,
    pub(crate) placement: TargetPlacement,
}

impl TargetMetadata {
    pub(crate) fn describe(&self, id: TargetId, kind: TargetKind) -> TargetDescription {
        TargetDescription::new(
            id,
            self.name.clone(),
            self.extent,
            PixelFormat::Bgra8,
            CoordinateSupport::with_target_placement(),
        )
        .with_capability(TargetCapability::new(
            kind,
            CapabilitySupport::Supported,
            mado_pilot_core::InputCapability::none(),
        ))
    }
}

#[derive(Debug)]
pub(crate) struct Candidate {
    pub(crate) key: NativeKey,
    pub(crate) fingerprint: Fingerprint,
    pub(crate) metadata: TargetMetadata,
    pub(crate) item: GraphicsCaptureItem,
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
            let monitor = HMONITOR(std::ptr::with_exposed_provenance_mut::<c_void>(raw));
            let (_, bounds) = monitor_metadata(monitor)?;
            monitor_placement(monitor, bounds, extent).ok()
        }
    }
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

        let mut process_id = 0;
        // SAFETY: the output points to a valid local u32 and hwnd was validated.
        let thread_id = unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut process_id)) };
        let class_name = window_class(hwnd);
        candidates.push(Candidate {
            key: NativeKey::Window(raw),
            fingerprint: Fingerprint::Window {
                process_id,
                thread_id,
                class_name,
            },
            metadata: TargetMetadata {
                name,
                extent,
                placement,
            },
            item,
        });
    }
    Ok(candidates)
}

fn display_candidates(factory: &IGraphicsCaptureItemInterop) -> Result<Vec<Candidate>> {
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
            fingerprint: Fingerprint::Display {
                device_name: device_name.clone(),
            },
            metadata: TargetMetadata {
                name: device_name,
                extent,
                placement,
            },
            item,
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

fn window_class(hwnd: HWND) -> String {
    let mut buffer = [0u16; MAX_CLASS_NAME];
    // SAFETY: buffer is writable and hwnd is from the current enumeration.
    let written = unsafe { GetClassNameW(hwnd, &mut buffer) };
    usize::try_from(written)
        .ok()
        .filter(|length| *length > 0)
        .map_or_else(String::new, |length| {
            String::from_utf16_lossy(&buffer[..length])
        })
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
    TargetPlacement::new(
        (
            f64::from(physical_x) / scale.x(),
            f64::from(physical_y) / scale.y(),
        ),
        (
            f64::from(extent.width()) / scale.x(),
            f64::from(extent.height()) / scale.y(),
        ),
        scale,
    )
}

fn positive_extent(width: i32, height: i32) -> Option<PixelExtent> {
    Some(PixelExtent::new(
        u32::try_from(width).ok().filter(|value| *value > 0)?,
        u32::try_from(height).ok().filter(|value| *value > 0)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::{placement_with_scale, positive_extent};
    use mado_pilot_core::{
        CoordinateSpace, GeometryRevision, PixelExtent, Point, Scale, TransformSnapshot,
    };

    #[test]
    fn signed_mixed_dpi_placement_is_frame_authoritative() {
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

        assert_eq!(placement.desktop_origin(), (-1280.0, -80.0));
        assert_eq!((desktop_origin.x(), desktop_origin.y()), (-1280.0, -80.0));
        assert!(snapshot.covers_target());
    }

    #[test]
    fn empty_or_negative_native_sizes_are_filtered() {
        assert_eq!(positive_extent(0, 10), None);
        assert_eq!(positive_extent(10, -1), None);
        assert_eq!(positive_extent(10, 20), Some(PixelExtent::new(10, 20)));
    }
}
