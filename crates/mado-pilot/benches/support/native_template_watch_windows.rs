// Windows transport and target-owner binding for the shared native watcher harness.

#[cfg(windows)]
use std::collections::VecDeque;
#[cfg(windows)]
use std::io::{BufRead, BufReader};
#[cfg(windows)]
use std::mem::size_of;
#[cfg(windows)]
use std::process::{Child, Command, Stdio};
#[cfg(windows)]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(windows)]
use std::sync::mpsc::{self, Receiver};

#[cfg(windows)]
use mado_pilot_platform_windows::fixture_protocol as protocol;
#[cfg(windows)]
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
#[cfg(windows)]
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
#[cfg(windows)]
use windows::Win32::System::Threading::GetCurrentProcess;
#[cfg(windows)]
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};
#[cfg(windows)]
use windows::core::{BOOL, PCWSTR};

#[cfg(windows)]
static NEXT_FIXTURE_TOKEN: AtomicU64 = AtomicU64::new(1);

#[cfg(windows)]
struct NativeFixture {
    child: Option<Child>,
    lines: Receiver<String>,
    reader: Option<thread::JoinHandle<()>>,
    pending: VecDeque<String>,
    title: String,
    generation: u64,
    revision: u64,
    moved: bool,
    resized: bool,
}

#[cfg(windows)]
impl NativeFixture {
    fn start(arguments: &Arguments) -> Result<Self, String> {
        let token = format!(
            "native-watch-{}-{}",
            std::process::id(),
            NEXT_FIXTURE_TOKEN.fetch_add(1, Ordering::Relaxed)
        );
        let title = protocol::ordinary_fixture_title(&token);
        let mut child = Command::new(&arguments.fixture_executable)
            .arg(format!("--title-token={token}"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "fixture_authority_failed".to_owned())?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| "fixture_authority_failed".to_owned())?;
        let (sender, lines) = mpsc::sync_channel(64);
        let reader = thread::Builder::new()
            .name("mado-pilot-native-watch-fixture".to_owned())
            .spawn(move || {
                for line in BufReader::new(output).lines() {
                    let Ok(line) = line else { break };
                    if line.len() > 1_024 || sender.send(line).is_err() {
                        break;
                    }
                }
            })
            .map_err(|_| "fixture_authority_failed".to_owned())?;
        let mut fixture = Self {
            child: Some(child),
            lines,
            reader: Some(reader),
            pending: VecDeque::new(),
            title,
            generation: 1,
            revision: 0,
            moved: false,
            resized: false,
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
        })
    }

    fn set_visible(&mut self) -> Result<ControlAcknowledgement, String> {
        self.post(protocol::CONTROL_SET_VISUAL_VISIBLE, 0, 0)?;
        self.acknowledge(protocol::FixtureVisualState::Visible.acknowledgement())
    }

    fn set_absent(&mut self) -> Result<ControlAcknowledgement, String> {
        self.post(protocol::CONTROL_SET_VISUAL_ABSENT, 0, 0)?;
        self.acknowledge(protocol::FixtureVisualState::Absent.acknowledgement())
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
        let mut origins = monitor_origins()?;
        origins.sort_unstable();
        origins.dedup();
        if origins.len() < 2 {
            return Err("capability_unavailable:topology".to_owned());
        }
        let (x, y) = origins[1];
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

    fn finish(&mut self) -> bool {
        let mut complete = true;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            complete &= child.wait().is_ok();
        }
        if let Some(reader) = self.reader.take() {
            complete &= reader.join().is_ok();
        }
        complete
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
    let origins = unsafe {
        &mut *std::ptr::with_exposed_provenance_mut::<Vec<(i32, i32)>>(state.0.cast_unsigned())
    };
    let mut info = MONITORINFO {
        cbSize: u32::try_from(size_of::<MONITORINFO>()).expect("MONITORINFO size fits u32"),
        ..Default::default()
    };
    // SAFETY: monitor came from EnumDisplayMonitors and info is writable.
    if unsafe { GetMonitorInfoW(monitor, &raw mut info) }.as_bool() {
        origins.push((info.rcMonitor.left, info.rcMonitor.top));
    }
    true.into()
}

#[cfg(windows)]
fn monitor_origins() -> Result<Vec<(i32, i32)>, String> {
    let mut origins = Vec::new();
    // SAFETY: enumeration is synchronous and receives the exclusive Vec pointer.
    unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect_monitor),
            LPARAM(
                isize::try_from((&raw mut origins).expose_provenance())
                    .map_err(|_| "capability_unavailable:topology".to_owned())?,
            ),
        )
    }
    .ok()
    .map_err(|_| "capability_unavailable:topology".to_owned())?;
    Ok(origins)
}

#[cfg(windows)]
const WGC_READY_PROBE: Duration = Duration::from_millis(50);

#[cfg(windows)]
fn native_engine() -> mado_pilot::Result<Engine> {
    mado_pilot::windows_engine(NativeEngineRequest::new())
}

#[cfg(windows)]
fn prepare_two_session_readiness(run: &mut NativeRun, second: &Session) -> Result<(), String> {
    let deadline = Instant::now() + OPERATION_WAIT;
    let mut first_cursor = None;
    let mut second_cursor = None;
    loop {
        if Instant::now() >= deadline {
            let _restored = run.command_absent();
            return Err("typed_operation_failure:DeadlineExceeded".to_owned());
        }

        run.command_visible()?;
        let visible = (|| {
            let second_visible = try_observe_marker_state(
                second,
                &run.fixture,
                &mut second_cursor,
                true,
                readiness_probe_wait(deadline),
            )?;
            let first_visible = try_observe_marker_state(
                &run.session,
                &run.fixture,
                &mut first_cursor,
                true,
                readiness_probe_wait(deadline),
            )?;
            Ok((first_visible, second_visible))
        })();
        let restored = run.command_absent();
        let (first_visible, second_visible) = match visible {
            Ok(observed) => observed,
            Err(error) => {
                let _restored = restored;
                return Err(error);
            }
        };
        restored?;

        let second_absent = try_observe_marker_state(
            second,
            &run.fixture,
            &mut second_cursor,
            false,
            readiness_probe_wait(deadline),
        )?;
        let first_absent = try_observe_marker_state(
            &run.session,
            &run.fixture,
            &mut first_cursor,
            false,
            readiness_probe_wait(deadline),
        )?;
        if first_visible && second_visible && first_absent && second_absent {
            return Ok(());
        }
    }
}

#[cfg(windows)]
fn readiness_probe_wait(deadline: Instant) -> Duration {
    WGC_READY_PROBE.min(deadline.saturating_duration_since(Instant::now()))
}

#[cfg(windows)]
fn try_observe_marker_state(
    session: &Session,
    fixture: &NativeFixture,
    cursor: &mut Option<FrameStamp>,
    expected_visible: bool,
    wait: Duration,
) -> Result<bool, String> {
    let deadline = Instant::now() + wait;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(false);
        }
        let request = cursor.map_or_else(FrameRequest::latest, FrameRequest::newer_than);
        let frame = match session.acquire_frame(&request, &bounded(remaining)) {
            Ok(frame) => frame,
            Err(error) if error.status() == Status::DeadlineExceeded => return Ok(false),
            Err(error) => {
                return Err(format!("typed_operation_failure:{:?}", error.status()));
            }
        };
        *cursor = Some(frame.stamp());
        let shape = marker_shape(&frame, fixture).ok_or_else(|| "wrong_transform".to_owned())?;
        let mapping = session
            .map_frame(&frame, PixelFormat::Rgba8, &bounded(remaining))
            .map_err(|_| "wrong_region".to_owned())?;
        if marker_state(&mapping, shape) == Some(expected_visible) {
            return Ok(true);
        }
    }
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
fn marker_shape(frame: &Frame, fixture: &NativeFixture) -> Option<MarkerShape> {
    let placement = frame.transform().target()?;
    let window = fixture.window().ok()?;
    let marker_x = scaled_i32(MARKER_X_LOGICAL, 1.0)?;
    let marker_y = scaled_i32(MARKER_Y_LOGICAL, 1.0)?;
    let marker_cell = scaled_i32(MARKER_CELL_LOGICAL, 1.0)?;
    let mut origin = POINT {
        x: marker_x,
        y: marker_y,
    };
    let mut far = POINT {
        x: marker_x.checked_add(marker_cell)?,
        y: marker_y.checked_add(marker_cell)?,
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
    // SAFETY: same as above for the marker cell's far corner.
    let far_converted = unsafe { ClientToScreen(window, &raw mut far) }.as_bool();
    // SAFETY: `prior_dpi` came from the successful context change above.
    let restored = unsafe { SetThreadDpiAwarenessContext(prior_dpi) };
    if restored.0.is_null() || !origin_converted || !far_converted {
        return None;
    }
    let cell_width = u32::try_from(far.x.checked_sub(origin.x)?).ok()?;
    let cell_height = u32::try_from(far.y.checked_sub(origin.y)?).ok()?;
    let (desktop_x, desktop_y) = placement.desktop_origin();
    Some(MarkerShape {
        cell_width,
        cell_height,
        origin_x: scaled_i32(f64::from(origin.x) - desktop_x, 1.0)?,
        origin_y: scaled_i32(f64::from(origin.y) - desktop_y, 1.0)?,
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
