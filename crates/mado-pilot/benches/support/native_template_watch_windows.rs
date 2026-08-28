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
use windows::Win32::Foundation::{BOOL, HDC, HMONITOR, HWND, LPARAM, POINT, RECT, WPARAM};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, MONITORINFO};
#[cfg(windows)]
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
#[cfg(windows)]
use windows::Win32::System::Threading::GetCurrentProcess;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    ClientToScreen, FindWindowW, GetWindowRect, PostMessageW,
};
#[cfg(windows)]
use windows::core::PCWSTR;

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
            if let Ok(targets) = engine.discover(&bounded(deadline.saturating_duration_since(Instant::now()))) {
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
        self.acknowledge("control destroy=ready")
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
        &mut *std::ptr::with_exposed_provenance_mut::<Vec<(i32, i32)>>(
            state.0.cast_unsigned(),
        )
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
    .map_err(|_| "capability_unavailable:topology".to_owned())?;
    Ok(origins)
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
fn marker_shape(frame: &Frame, fixture: &NativeFixture) -> Option<MarkerShape> {
    let placement = frame.transform().target()?;
    let scale = placement.scale();
    let window = fixture.window().ok()?;
    let mut outer = RECT::default();
    let mut client_origin = POINT::default();
    // SAFETY: `window` is the authenticated live fixture HWND and both output
    // records are writable for the duration of their synchronous USER32 calls.
    unsafe { GetWindowRect(window, &raw mut outer) }.ok()?;
    // SAFETY: the zero client point is converted for the same live HWND.
    unsafe { ClientToScreen(window, &raw mut client_origin) }.ok()?;
    let inset_x = client_origin.x.checked_sub(outer.left)?;
    let inset_y = client_origin.y.checked_sub(outer.top)?;
    Some(MarkerShape {
        cell_width: scaled_u32(MARKER_CELL_LOGICAL, scale.x())?,
        cell_height: scaled_u32(MARKER_CELL_LOGICAL, scale.y())?,
        origin_x: inset_x.checked_add(scaled_i32(MARKER_X_LOGICAL, scale.x())?)?,
        origin_y: inset_y.checked_add(scaled_i32(MARKER_Y_LOGICAL, scale.y())?)?,
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
