// Windows transport and target-owner binding for the shared native watcher harness.

#[cfg(windows)]
use std::collections::VecDeque;
#[cfg(windows)]
use std::io::Read;
#[cfg(windows)]
use std::mem::size_of;
#[cfg(windows)]
use std::num::NonZeroU32;
#[cfg(windows)]
use std::process::{Child, Command, Stdio};
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(windows)]
use std::sync::mpsc::{self, Receiver};

#[cfg(windows)]
use mado_pilot_platform_windows::fixture_protocol as protocol;
#[cfg(windows)]
use mado_pilot_testkit::visual_token::VisualTokenSequence;
#[cfg(windows)]
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
#[cfg(windows)]
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
#[cfg(windows)]
use windows::Win32::System::Threading::GetCurrentProcess;
#[cfg(windows)]
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI,
    SetThreadDpiAwarenessContext,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};
#[cfg(windows)]
use windows::core::{BOOL, PCWSTR};

#[cfg(windows)]
static NEXT_FIXTURE_TOKEN: AtomicU64 = AtomicU64::new(1);
#[cfg(windows)]
const MAX_OUTPUT_LINE_BYTES: usize = 1_024;

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct MonitorFact {
    origin: (i32, i32),
    dpi: (u32, u32),
}

#[cfg(windows)]
#[must_use = "fixture finalization must be checked before accepting a scenario"]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NativeFixtureFinalization {
    process_reaped: bool,
    reader_joined: bool,
    output_clean: bool,
}

#[cfg(windows)]
impl NativeFixtureFinalization {
    const fn is_accepted(self) -> bool {
        self.process_reaped && self.reader_joined && self.output_clean
    }

    const fn resources(self) -> NativeResourceFacts {
        NativeResourceFacts {
            baseline_observed: true,
            fixture_process_reaped: self.process_reaped,
            fixture_reader_joined: self.reader_joined,
            protocol_stop_acknowledged: None,
            authenticated_lifetime: None,
            launched_lifetime: None,
            bounded_containment: self.process_reaped && self.reader_joined,
            output_drained: self.output_clean,
            executable_identity_unchanged: None,
            cleanup_debt: None,
            apple_launch_accepted_live: None,
            apple_cleanup_scheduled: None,
            apple_cleanup_active: None,
            apple_cleanup_completed: None,
            apple_cleanup_exhausted: None,
        }
    }
}

#[cfg(windows)]
struct NativeFixture {
    child: Option<Child>,
    lines: Receiver<String>,
    reader: Option<thread::JoinHandle<()>>,
    reader_failed: Arc<AtomicBool>,
    pending: VecDeque<String>,
    title: String,
    generation: u64,
    revision: u64,
    moved: bool,
    resized: bool,
    visual_tokens: VisualTokenSequence,
    finish_result: Option<NativeFixtureFinalization>,
}

#[cfg(windows)]
impl NativeFixture {
    fn start(arguments: &Arguments) -> Result<Self, String> {
        let bytes = std::fs::read(&arguments.fixture_executable)
            .map_err(|_| "fixture_authority_failed".to_owned())?;
        if !fixture_bytes_match(arguments, &bytes) {
            return Err("fixture_authority_failed".to_owned());
        }
        let token = format!(
            "native-watch-{}-{}",
            std::process::id(),
            NEXT_FIXTURE_TOKEN.fetch_add(1, Ordering::Relaxed)
        );
        let title = protocol::ordinary_fixture_title(&token);
        let mut command = Command::new(&arguments.fixture_executable);
        command.env_clear();
        for key in ["SystemRoot", "WINDIR"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        let mut child = command
            .arg(format!("--title-token={token}"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "fixture_authority_failed".to_owned())?;
        if !fixture_path_matches(arguments) {
            let _reaped = terminate_child_bounded(
                &mut child,
                Instant::now() + FIXTURE_WAIT,
            );
            return Err("fixture_authority_failed".to_owned());
        }
        let Some(output) = child.stdout.take() else {
            let _reaped = terminate_child_bounded(
                &mut child,
                Instant::now() + FIXTURE_WAIT,
            );
            return Err("fixture_authority_failed".to_owned());
        };
        let (sender, lines) = mpsc::sync_channel(64);
        let reader_failed = Arc::new(AtomicBool::new(false));
        let reader_failed_for_thread = Arc::clone(&reader_failed);
        // The receiver may stop draining before Drop joins this reader, so a
        // full channel must terminate the reader instead of blocking it.
        let reader = match thread::Builder::new()
            .name("mado-pilot-native-watch-fixture".to_owned())
            .spawn(move || {
                let mut stream = output;
                let mut line = Vec::with_capacity(MAX_OUTPUT_LINE_BYTES);
                let mut byte = [0u8; 1];
                let mut overflow = false;
                loop {
                    match stream.read(&mut byte) {
                        Ok(0) => {
                            if overflow {
                                reader_failed_for_thread.store(true, Ordering::Release);
                            } else if !line.is_empty() {
                                if line.last() == Some(&b'\r') {
                                    line.pop();
                                }
                                let Ok(decoded) =
                                    String::from_utf8(std::mem::take(&mut line))
                                else {
                                    reader_failed_for_thread.store(true, Ordering::Release);
                                    break;
                                };
                                if sender.try_send(decoded).is_err() {
                                    reader_failed_for_thread.store(true, Ordering::Release);
                                }
                            }
                            break;
                        }
                        Ok(_) if byte[0] == b'\n' => {
                            if overflow {
                                reader_failed_for_thread.store(true, Ordering::Release);
                                break;
                            }
                            if line.last() == Some(&b'\r') {
                                line.pop();
                            }
                            let Ok(decoded) = String::from_utf8(std::mem::take(&mut line)) else {
                                reader_failed_for_thread.store(true, Ordering::Release);
                                break;
                            };
                            if sender.try_send(decoded).is_err() {
                                reader_failed_for_thread.store(true, Ordering::Release);
                                break;
                            }
                            line = Vec::with_capacity(MAX_OUTPUT_LINE_BYTES);
                        }
                        Ok(_) if !overflow && line.len() < MAX_OUTPUT_LINE_BYTES => {
                            line.push(byte[0]);
                        }
                        Ok(_) => {
                            overflow = true;
                            reader_failed_for_thread.store(true, Ordering::Release);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(_) => {
                            reader_failed_for_thread.store(true, Ordering::Release);
                            break;
                        }
                    }
                }
            })
        {
            Ok(reader) => reader,
            Err(_) => {
                let _reaped = terminate_child_bounded(
                    &mut child,
                    Instant::now() + FIXTURE_WAIT,
                );
                return Err("fixture_authority_failed".to_owned());
            }
        };
        let mut fixture = Self {
            child: Some(child),
            lines,
            reader: Some(reader),
            reader_failed,
            pending: VecDeque::new(),
            title,
            generation: 1,
            revision: 0,
            moved: false,
            resized: false,
            visual_tokens: VisualTokenSequence::new(),
            finish_result: None,
        };
        let ready = fixture.wait_for("fixture-ready ", FIXTURE_WAIT)?;
        let expected = format!(
            "fixture-ready class={} title={} capacity={}",
            protocol::ORDINARY_CLASS_NAME,
            fixture.title,
            protocol::MAX_RECORDED_EVENTS,
        );
        if ready.trim() != expected {
            return Err("fixture_authority_failed".to_owned());
        }
        Ok(fixture)
    }

    fn process_id(&self) -> u32 {
        self.child.as_ref().map_or(0, Child::id)
    }

    fn window(&self) -> Result<HWND, String> {
        let class = wide(protocol::ORDINARY_CLASS_NAME);
        let title = wide(&self.title);
        // SAFETY: both UTF-16 buffers are terminated and live for the call.
        let window = unsafe { FindWindowW(PCWSTR(class.as_ptr()), PCWSTR(title.as_ptr())) }
            .map_err(|_| "fixture_authority_failed".to_owned())?;
        if window == HWND::default() {
            return Err("fixture_authority_failed".to_owned());
        }
        Ok(window)
    }

    fn authenticated_target(&self, engine: &Engine) -> Result<TargetId, String> {
        let deadline = Instant::now() + FIXTURE_WAIT;
        loop {
            if let Ok(targets) =
                engine.discover(&bounded(deadline.saturating_duration_since(Instant::now())))
            {
                let mut matches = targets.iter().filter(|target| {
                    target.name() == self.title
                        && target.capability().kind() == Some(TargetKind::Window)
                        && mado_pilot_platform_windows::fixture_observation::target_has_process(
                            target.id(),
                            self.process_id(),
                        )
                });
                if let Some(target) = matches.next()
                    && matches.next().is_none()
                {
                    return Ok(target.id());
                }
            }
            if Instant::now() >= deadline {
                return Err("fixture_authority_failed".to_owned());
            }
            thread::sleep(POLL_WAIT);
        }
    }

    fn wait_for(&mut self, prefix: &str, wait: Duration) -> Result<String, String> {
        if let Some(index) = self
            .pending
            .iter()
            .position(|line| line.starts_with(prefix))
        {
            return self
                .pending
                .remove(index)
                .ok_or_else(|| "fixture_authority_failed".to_owned());
        }
        let deadline = Instant::now() + wait;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("fixture_authority_failed".to_owned());
            }
            let line = self
                .lines
                .recv_timeout(remaining)
                .map_err(|_| "fixture_authority_failed".to_owned())?;
            if line.starts_with(prefix) {
                return Ok(line);
            }
            if self.pending.len() == 64 {
                return Err("fixture_authority_failed".to_owned());
            }
            self.pending.push_back(line);
        }
    }

    fn post(&self, message: u32, wparam: usize, lparam: isize) -> Result<(), String> {
        let window = self.window()?;
        // SAFETY: the HWND is the authenticated retained fixture window and the
        // private protocol defines these arguments as scalar values.
        unsafe { PostMessageW(Some(window), message, WPARAM(wparam), LPARAM(lparam)) }
            .map_err(|_| "fixture_authority_failed".to_owned())
    }

    fn acknowledge(&mut self, prefix: &str) -> Result<ControlAcknowledgement, String> {
        let line = self.wait_for(prefix, FIXTURE_COMMAND_WAIT)?;
        if line.trim() != prefix {
            return Err("fixture_authority_failed".to_owned());
        }
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| "fixture_authority_failed".to_owned())?;
        Ok(ControlAcknowledgement {
            generation: self.generation,
            revision: self.revision,
            visual_token: None,
        })
    }

    fn set_visible(&mut self) -> Result<ControlAcknowledgement, String> {
        self.set_visual(
            protocol::CONTROL_SET_VISUAL_VISIBLE,
            protocol::FixtureVisualState::Visible,
            VisualMarkerState::Visible,
        )
    }

    fn set_absent(&mut self) -> Result<ControlAcknowledgement, String> {
        self.set_visual(
            protocol::CONTROL_SET_VISUAL_ABSENT,
            protocol::FixtureVisualState::Absent,
            VisualMarkerState::Absent,
        )
    }

    fn set_visual(
        &mut self,
        message: u32,
        state: protocol::FixtureVisualState,
        marker: VisualMarkerState,
    ) -> Result<ControlAcknowledgement, String> {
        let token = self
            .visual_tokens
            .issue(marker)
            .map_err(|_| "fixture_authority_failed".to_owned())?;
        let command = protocol::FixtureVisualCommand::new(
            state,
            NonZeroU32::new(token.value()).expect("shared tokens are nonzero"),
        );
        self.post(
            message,
            usize::try_from(token.value())
                .map_err(|_| "fixture_authority_failed".to_owned())?,
            0,
        )?;
        let mut acknowledgement = self.acknowledge(&command.acknowledgement())?;
        acknowledgement.visual_token = Some(token);
        Ok(acknowledgement)
    }

    fn transition_visual(&mut self) -> Result<ControlAcknowledgement, String> {
        self.post(protocol::CONTROL_TRANSITION_VISUAL, 0, 0)?;
        self.acknowledge(protocol::VISUAL_TRANSITION_ACKNOWLEDGEMENT)
    }

    fn set_geometry(
        &mut self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Result<ControlAcknowledgement, String> {
        let position = u64::from(x.cast_unsigned()) | (u64::from(y.cast_unsigned()) << 32);
        let size = u64::from(width.cast_unsigned()) | (u64::from(height.cast_unsigned()) << 32);
        self.post(
            protocol::CONTROL_SET_GEOMETRY,
            usize::try_from(position).map_err(|_| "fixture_authority_failed".to_owned())?,
            isize::try_from(size).map_err(|_| "fixture_authority_failed".to_owned())?,
        )?;
        self.acknowledge("control geometry=ready")
    }

    fn move_target(&mut self) -> Result<ControlAcknowledgement, String> {
        self.moved = !self.moved;
        let (x, y) = if self.moved { (220, 180) } else { (120, 120) };
        let (width, height) = if self.resized { (480, 320) } else { (360, 240) };
        self.set_geometry(x, y, width, height)
    }

    fn resize_target(&mut self) -> Result<ControlAcknowledgement, String> {
        self.resized = !self.resized;
        let (width, height) = if self.resized { (480, 320) } else { (360, 240) };
        let (x, y) = if self.moved { (220, 180) } else { (120, 120) };
        self.set_geometry(x, y, width, height)
    }

    fn move_next_display(&mut self) -> Result<ControlAcknowledgement, String> {
        let window = self.window()?;
        // SAFETY: `window` is the authenticated live fixture HWND.
        let current = unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) };
        let current_origin =
            monitor_origin(current).ok_or_else(|| "capability_unavailable:topology".to_owned())?;
        let current_dpi =
            monitor_dpi(current).ok_or_else(|| "capability_unavailable:topology".to_owned())?;
        let (x, y) = next_monitor_origin(monitor_facts()?, current_origin, current_dpi)
            .ok_or_else(|| "capability_unavailable:topology".to_owned())?;
        let (width, height) = if self.resized { (480, 320) } else { (360, 240) };
        self.moved = true;
        self.set_geometry(x.saturating_add(80), y.saturating_add(80), width, height)
    }

    fn restore_placement(&mut self) -> Result<ControlAcknowledgement, String> {
        self.moved = false;
        let (width, height) = if self.resized { (480, 320) } else { (360, 240) };
        self.set_geometry(120, 120, width, height)
    }

    fn close_target(&mut self) -> Result<ControlAcknowledgement, String> {
        self.post(protocol::CONTROL_DESTROY_TARGET, 0, 0)?;
        self.acknowledge(protocol::TARGET_LOSS_ACKNOWLEDGEMENT)
    }

    fn finish(&mut self) -> NativeFixtureFinalization {
        if let Some(result) = self.finish_result {
            return result;
        }
        let deadline = Instant::now() + FIXTURE_WAIT;
        let process_reaped = self
            .child
            .take()
            .is_none_or(|mut child| terminate_child_bounded(&mut child, deadline));
        let reader_joined = self.reader.take().is_none_or(|reader| {
            while !reader.is_finished() && Instant::now() < deadline {
                thread::sleep(POLL_WAIT);
            }
            reader.is_finished() && reader.join().is_ok()
        });
        while let Ok(line) = self.lines.try_recv() {
            if self.pending.len() == 64 {
                self.reader_failed.store(true, Ordering::Release);
                break;
            }
            self.pending.push_back(line);
        }
        let output_clean =
            reader_joined && self.pending.is_empty() && !self.reader_failed.load(Ordering::Acquire);
        let result = NativeFixtureFinalization {
            process_reaped,
            reader_joined,
            output_clean,
        };
        self.finish_result = Some(result);
        result
    }
}

fn terminate_child_bounded(child: &mut Child, deadline: Instant) -> bool {
    let _termination_requested = child.kill();
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => return true,
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL_WAIT),
            Ok(None) | Err(_) => return false,
        }
    }
}

#[cfg(windows)]
impl Drop for NativeFixture {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

#[cfg(windows)]
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
    if let (Some(origin), Some(dpi)) = (monitor_origin(monitor), monitor_dpi(monitor)) {
        monitors.push(MonitorFact { origin, dpi });
    }
    true.into()
}

#[cfg(windows)]
fn monitor_origin(monitor: HMONITOR) -> Option<(i32, i32)> {
    let mut info = MONITORINFO {
        cbSize: u32::try_from(size_of::<MONITORINFO>()).expect("MONITORINFO size fits u32"),
        ..Default::default()
    };
    // SAFETY: `monitor` came from a Win32 monitor lookup and `info` is writable.
    unsafe { GetMonitorInfoW(monitor, &raw mut info) }
        .as_bool()
        .then_some((info.rcMonitor.left, info.rcMonitor.top))
}

#[cfg(windows)]
fn monitor_dpi(monitor: HMONITOR) -> Option<(u32, u32)> {
    let mut dpi_x = 0;
    let mut dpi_y = 0;
    // SAFETY: `monitor` came from a Win32 monitor lookup and both outputs are writable.
    unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &raw mut dpi_x, &raw mut dpi_y) }
        .is_ok()
        .then_some((dpi_x, dpi_y))
        .filter(|(dpi_x, dpi_y)| *dpi_x != 0 && *dpi_y != 0)
}

#[cfg(windows)]
fn monitor_facts() -> Result<Vec<MonitorFact>, String> {
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

#[cfg(windows)]
fn next_monitor_origin(
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

#[cfg(windows)]
fn native_engine() -> mado_pilot::Result<Engine> {
    mado_pilot::windows_engine(NativeEngineRequest::new())
}

#[cfg(windows)]
fn permission_oracle(_engine: &Engine) -> bool {
    true
}

#[cfg(windows)]
fn resize_geometry_matches(before: &Frame, after: &Frame) -> bool {
    let before_extent = before.descriptor().extent();
    let after_extent = after.descriptor().extent();
    let Some(before_placement) = before.transform().target() else {
        return false;
    };
    let Some(after_placement) = after.transform().target() else {
        return false;
    };
    // The fixture alternates requested outer sizes 360x240 and 480x320. WGC
    // excludes constant non-client chrome, so its authoritative frame extents
    // must change by the same exact 120x80 rather than equal the outer sizes.
    before_extent.width().abs_diff(after_extent.width()) == 120
        && before_extent.height().abs_diff(after_extent.height()) == 80
        && before_placement.desktop_origin() == after_placement.desktop_origin()
        && before_placement.scale() == after_placement.scale()
        && before_placement.desktop_scale() == after_placement.desktop_scale()
}

#[cfg(windows)]
fn target_scale_changed(
    before: &mado_pilot::TargetPlacement,
    after: &mado_pilot::TargetPlacement,
) -> bool {
    // Windows virtual-desktop coordinates are physical, so every placement's
    // desktop scale is 1.0. The target scale carries the monitor's effective DPI.
    before.scale() != after.scale()
}

#[cfg(windows)]
fn topology_geometry_matches(before: &Frame, after: &Frame) -> bool {
    let Some(before_placement) = before.transform().target() else {
        return false;
    };
    let Some(after_placement) = after.transform().target() else {
        return false;
    };
    target_scale_changed(&before_placement, &after_placement)
}

#[cfg(windows)]
fn client_cell_shape(
    frame: &Frame,
    fixture: &NativeFixture,
    logical_x: f64,
    logical_y: f64,
    logical_cell: f64,
) -> Option<(u32, u32, i32, i32)> {
    let placement = frame.transform().target()?;
    let window = fixture.window().ok()?;
    let cell_x = scaled_i32(logical_x, 1.0)?;
    let cell_y = scaled_i32(logical_y, 1.0)?;
    let cell_size = scaled_i32(logical_cell, 1.0)?;
    let mut origin = POINT {
        x: cell_x,
        y: cell_y,
    };
    let mut far = POINT {
        x: cell_x.checked_add(cell_size)?,
        y: cell_y.checked_add(cell_size)?,
    };
    // SAFETY: the predefined context is valid on the approved Windows floor and
    // affects only this thread; the returned prior context is restored below
    // before any fallible coordinate processing.
    let prior_dpi =
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    if prior_dpi.0.is_null() {
        return None;
    }
    // SAFETY: both points are writable and `window` is the authenticated live fixture HWND.
    let origin_converted = unsafe { ClientToScreen(window, &raw mut origin) }.as_bool();
    // SAFETY: same as above for the cell's far corner.
    let far_converted = unsafe { ClientToScreen(window, &raw mut far) }.as_bool();
    // SAFETY: `prior_dpi` came from the successful context change above.
    let restored = unsafe { SetThreadDpiAwarenessContext(prior_dpi) };
    if restored.0.is_null() || !origin_converted || !far_converted {
        return None;
    }
    let cell_width = u32::try_from(far.x.checked_sub(origin.x)?).ok()?;
    let cell_height = u32::try_from(far.y.checked_sub(origin.y)?).ok()?;
    let (desktop_x, desktop_y) = placement.desktop_origin();
    Some((
        cell_width,
        cell_height,
        scaled_i32(f64::from(origin.x) - desktop_x, 1.0)?,
        scaled_i32(f64::from(origin.y) - desktop_y, 1.0)?,
    ))
}

#[cfg(windows)]
fn marker_shape(frame: &Frame, fixture: &NativeFixture) -> Option<MarkerShape> {
    let (cell_width, cell_height, origin_x, origin_y) = client_cell_shape(
        frame,
        fixture,
        MARKER_X_LOGICAL,
        MARKER_Y_LOGICAL,
        MARKER_CELL_LOGICAL,
    )?;
    Some(MarkerShape {
        cell_width,
        cell_height,
        origin_x,
        origin_y,
    })
}

#[cfg(windows)]
fn token_shape(frame: &Frame, fixture: &NativeFixture) -> Option<TokenShape> {
    let (cell_width, cell_height, origin_x, origin_y) = client_cell_shape(
        frame,
        fixture,
        TOKEN_X_LOGICAL,
        TOKEN_Y_LOGICAL,
        TOKEN_CELL_LOGICAL,
    )?;
    Some(TokenShape {
        cell_width,
        cell_height,
        origin_x,
        origin_y,
    })
}

#[cfg(windows)]
fn target_name() -> &'static str {
    "x86_64-pc-windows-msvc"
}

#[cfg(windows)]
fn peak_resident_bytes() -> Option<u64> {
    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: u32::try_from(size_of::<PROCESS_MEMORY_COUNTERS>()).ok()?,
        ..Default::default()
    };
    // SAFETY: pseudo-handle is valid for this process and counters is writable.
    unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            &raw mut counters,
            u32::try_from(size_of::<PROCESS_MEMORY_COUNTERS>()).ok()?,
        )
    }
    .ok()?;
    u64::try_from(counters.PeakWorkingSetSize).ok()
}

#[cfg(all(test, windows))]
mod tests {

    #[test]
    fn topology_uses_per_target_scale_on_the_physical_windows_desktop() {
        let placement_with_target_scale = |scale| {
            mado_pilot::TargetPlacement::new(
                (0.0, 0.0),
                (240.0, 160.0),
                mado_pilot::Scale::new(scale, scale).expect("target scale"),
            )
            .expect("placement")
            .with_desktop_scale(mado_pilot::Scale::new(1.0, 1.0).expect("desktop scale"))
        };
        let before = placement_with_target_scale(1.5);
        let after = placement_with_target_scale(1.25);

        assert_eq!(before.desktop_scale(), after.desktop_scale());
        assert!(super::target_scale_changed(&before, &after));
        assert!(!super::target_scale_changed(&before, &before));
    }

    #[test]
    fn next_monitor_origin_never_selects_the_current_display() {
        let origins = vec![(0, 0), (-3840, 0), (0, 0)];

        assert_eq!(
            super::next_monitor_origin(origins.clone(), (0, 0)),
            Some((-3840, 0))
        );
        assert_eq!(
            super::next_monitor_origin(origins, (-3840, 0)),
            Some((0, 0))
        );
        assert_eq!(super::next_monitor_origin(vec![(0, 0)], (0, 0)), None);
        assert_eq!(
            super::next_monitor_origin(vec![(-3840, 0), (0, 0)], (10, 10)),
            None
        );
    }
}
