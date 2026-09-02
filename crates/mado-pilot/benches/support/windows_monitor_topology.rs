// Windows monitor topology queried by the native watcher qualification fixture.

use std::mem::size_of;

use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::core::BOOL;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct MonitorFact {
    pub(super) origin: (i32, i32),
    pub(super) dpi: (u32, u32),
}

unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _display: HDC,
    _rect: *mut RECT,
    state: LPARAM,
) -> BOOL {
    // SAFETY: caller passes an exclusive Vec pointer for the synchronous enumeration.
    let monitors = unsafe {
        &mut *std::ptr::with_exposed_provenance_mut::<Vec<MonitorFact>>(state.0.cast_unsigned())
    };
    if let (Some(origin), Some(dpi)) = (monitor_origin(monitor), monitor_effective_dpi(monitor)) {
        monitors.push(MonitorFact { origin, dpi });
    }
    true.into()
}

pub(super) fn monitor_origin(monitor: HMONITOR) -> Option<(i32, i32)> {
    let mut info = MONITORINFO {
        cbSize: u32::try_from(size_of::<MONITORINFO>()).expect("MONITORINFO size fits u32"),
        ..Default::default()
    };
    // SAFETY: `monitor` came from a Win32 monitor lookup and `info` is writable.
    unsafe { GetMonitorInfoW(monitor, &raw mut info) }
        .as_bool()
        .then_some((info.rcMonitor.left, info.rcMonitor.top))
}

pub(super) fn monitor_effective_dpi(monitor: HMONITOR) -> Option<(u32, u32)> {
    let mut dpi_x = 0;
    let mut dpi_y = 0;
    // SAFETY: `monitor` came from a Win32 monitor lookup and both outputs are writable.
    unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &raw mut dpi_x, &raw mut dpi_y) }
        .is_ok()
        .then_some((dpi_x, dpi_y))
        .filter(|(dpi_x, dpi_y)| *dpi_x != 0 && *dpi_y != 0)
}

pub(super) fn monitor_facts() -> Result<Vec<MonitorFact>, String> {
    let mut monitors = Vec::new();
    // SAFETY: enumeration is synchronous and receives the exclusive Vec pointer.
    unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect_monitor),
            LPARAM(
                isize::try_from((&raw mut monitors).expose_provenance())
                    .map_err(|_| "capability_unavailable:topology".to_owned())?,
            ),
        )
    }
    .ok()
    .map_err(|_| "capability_unavailable:topology".to_owned())?;
    Ok(monitors)
}

pub(super) fn next_monitor_origin(
    mut monitors: Vec<MonitorFact>,
    current_origin: (i32, i32),
    current_dpi: (u32, u32),
) -> Option<(i32, i32)> {
    monitors.sort_unstable();
    monitors.dedup();
    monitors
        .into_iter()
        .find(|monitor| monitor.origin != current_origin && monitor.dpi != current_dpi)
        .map(|monitor| monitor.origin)
}

#[cfg(test)]
mod tests {
    #[test]
    fn next_monitor_requires_a_different_origin_and_scale() {
        use super::{MonitorFact, next_monitor_origin};

        let monitors = vec![
            MonitorFact {
                origin: (0, 0),
                dpi: (96, 96),
            },
            MonitorFact {
                origin: (-3840, 0),
                dpi: (144, 144),
            },
            MonitorFact {
                origin: (0, 0),
                dpi: (96, 96),
            },
        ];

        assert_eq!(
            next_monitor_origin(monitors.clone(), (0, 0), (96, 96)),
            Some((-3840, 0))
        );
        assert_eq!(
            next_monitor_origin(monitors, (-3840, 0), (144, 144)),
            Some((0, 0))
        );
        assert_eq!(
            next_monitor_origin(
                vec![MonitorFact {
                    origin: (0, 0),
                    dpi: (96, 96),
                }],
                (0, 0),
                (96, 96),
            ),
            None
        );
    }

    #[test]
    #[ignore = "queries the approved mixed-DPI Windows qualification topology"]
    fn approved_host_exposes_distinct_effective_monitor_scale() {
        use super::monitor_facts;

        let monitors = monitor_facts().expect("monitor facts");
        let first = monitors.first().expect("at least one monitor");

        assert!(
            monitors.iter().any(|monitor| monitor.dpi != first.dpi),
            "approved mixed-DPI host must expose distinct effective monitor scale"
        );
    }
}
