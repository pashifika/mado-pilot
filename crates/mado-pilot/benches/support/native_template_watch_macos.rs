// macOS transport and target-owner binding for the shared native watcher harness.

#[cfg(target_os = "macos")]
use crate::macos_fixture::{
    FixtureController, LaunchMode, controlled_content_logical_size,
    controlled_resize_logical_size_matches, expected_controlled_resize_logical_size,
};
#[cfg(target_os = "macos")]
use crate::macos_fixture_control::executable_identity;
#[cfg(target_os = "macos")]
use crate::macos_fixture_protocol::{self as protocol, FixtureCommandKind};
#[cfg(target_os = "macos")]
const FIXTURE_STATUS_OK: u32 = 0;
#[cfg(target_os = "macos")]
const FIXTURE_STATUS_UNSUPPORTED: u32 = 2;

#[cfg(target_os = "macos")]
struct NativeFixture {
    controller: FixtureController,
    generation: u64,
    revision: u64,
}

#[cfg(target_os = "macos")]
impl NativeFixture {
    fn start(arguments: &Arguments) -> Result<Self, String> {
        let bytes = std::fs::read(&arguments.fixture_executable)
            .map_err(|_| "fixture_authority_failed".to_owned())?;
        let identity = executable_identity(&arguments.fixture_executable)
            .map_err(|_| "fixture_authority_failed".to_owned())?;
        let controller = FixtureController::start_once(
            &arguments.fixture_executable,
            Arc::from(bytes),
            identity,
            LaunchMode::Static,
            FIXTURE_WAIT,
        )?;
        Ok(Self {
            controller,
            generation: 1,
            revision: 0,
        })
    }

    fn process_id(&self) -> u32 {
        self.controller.process_id()
    }

    fn authenticated_target(&self, engine: &Engine) -> Result<TargetId, String> {
        let deadline = Instant::now() + FIXTURE_WAIT;
        loop {
            let process = self
                .controller
                .authenticated_process()
                .ok_or_else(|| "fixture_authority_failed".to_owned())?;
            if let Ok(targets) = engine.discover(&bounded(deadline.saturating_duration_since(Instant::now())))
                && let Ok(target) = protocol::select_unique_fixture(
                    &targets,
                    process.process_id(),
                    |target| {
                        mado_pilot_platform_macos::fixture_observation::target_has_authenticated_owner(
                            target, process,
                        )
                    },
                )
            {
                return Ok(target.id());
            }
            if Instant::now() >= deadline {
                return Err("fixture_authority_failed".to_owned());
            }
            thread::sleep(POLL_WAIT);
        }
    }

    fn command(&mut self, kind: FixtureCommandKind) -> Result<ControlAcknowledgement, String> {
        let acknowledgement = self.controller.command(kind, FIXTURE_COMMAND_WAIT)?;
        let status = acknowledgement.result().status;
        if kind == FixtureCommandKind::MoveToNextDisplay
            && status == FIXTURE_STATUS_UNSUPPORTED
        {
            return Err("capability_unavailable:topology".to_owned());
        }
        if status != FIXTURE_STATUS_OK {
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
        self.command(FixtureCommandKind::SetVisualVisible)
    }

    fn set_absent(&mut self) -> Result<ControlAcknowledgement, String> {
        self.command(FixtureCommandKind::SetVisualAbsent)
    }

    fn transition_visual(&mut self) -> Result<ControlAcknowledgement, String> {
        self.command(FixtureCommandKind::Transition)
    }

    fn move_target(&mut self) -> Result<ControlAcknowledgement, String> {
        self.command(FixtureCommandKind::Move)
    }

    fn resize_target(&mut self) -> Result<ControlAcknowledgement, String> {
        self.command(FixtureCommandKind::Resize)
    }

    fn move_next_display(&mut self) -> Result<ControlAcknowledgement, String> {
        self.command(FixtureCommandKind::MoveToNextDisplay)
    }

    fn restore_placement(&mut self) -> Result<ControlAcknowledgement, String> {
        self.command(FixtureCommandKind::RestorePlacement)
    }

    fn finish(&mut self) -> bool {
        let events_drained = self.controller.discard_watch_events(FIXTURE_WAIT);
        let fixture_finished = self.controller.finish(FIXTURE_WAIT);
        events_drained && fixture_finished
    }
}

#[cfg(target_os = "macos")]
impl Drop for NativeFixture {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

#[cfg(target_os = "macos")]
fn native_engine() -> mado_pilot::Result<Engine> {
    mado_pilot::macos_engine(NativeEngineRequest::new())
}

#[cfg(target_os = "macos")]
fn prepare_two_session_readiness(_run: &mut NativeRun, _second: &Session) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn permission_oracle(engine: &Engine) -> bool {
    use mado_pilot::{PermissionKind, PermissionState};

    engine.permissions(&bounded(OPERATION_WAIT)).is_ok_and(|report| {
        report.capture().kind() == PermissionKind::ScreenCapture
            && report.capture().state() == PermissionState::Granted
            && report.input().kind() == PermissionKind::InputControl
    })
}

#[cfg(target_os = "macos")]
fn resize_geometry_matches(before: &Frame, after: &Frame) -> bool {
    let Some(before_placement) = before.transform().target() else {
        return false;
    };
    let Some(after_placement) = after.transform().target() else {
        return false;
    };
    let Some(expected) =
        expected_controlled_resize_logical_size(before_placement.logical_size())
    else {
        return false;
    };
    before.descriptor().extent() != after.descriptor().extent()
        && before_placement.desktop_origin() == after_placement.desktop_origin()
        && before_placement.scale() == after_placement.scale()
        && before_placement.desktop_scale() == after_placement.desktop_scale()
        && controlled_resize_logical_size_matches(after_placement.logical_size(), expected)
}

#[cfg(target_os = "macos")]
fn topology_geometry_matches(before: &Frame, after: &Frame) -> bool {
    let Some(before_placement) = before.transform().target() else {
        return false;
    };
    let Some(after_placement) = after.transform().target() else {
        return false;
    };
    before_placement.desktop_origin() != after_placement.desktop_origin()
        && before_placement.logical_size() == after_placement.logical_size()
}

#[cfg(target_os = "macos")]
fn marker_shape(frame: &Frame, _fixture: &NativeFixture) -> Option<MarkerShape> {
    let placement = frame.transform().target()?;
    let scale = placement.scale();
    let (logical_width, logical_height) = placement.logical_size();
    let (content_width, content_height) =
        controlled_content_logical_size((logical_width, logical_height))?;
    let horizontal_inset = (logical_width - content_width) / 2.0;
    let top_inset = logical_height - content_height;
    if horizontal_inset < 0.0 || top_inset < 0.0 {
        return None;
    }
    Some(MarkerShape {
        cell_width: scaled_u32(MARKER_CELL_LOGICAL, scale.x())?,
        cell_height: scaled_u32(MARKER_CELL_LOGICAL, scale.y())?,
        origin_x: scaled_i32(horizontal_inset + MARKER_X_LOGICAL, scale.x())?,
        origin_y: scaled_i32(top_inset + MARKER_Y_LOGICAL, scale.y())?,
    })
}

#[cfg(target_os = "macos")]
fn target_name() -> &'static str {
    "aarch64-apple-darwin"
}

#[cfg(target_os = "macos")]
fn peak_resident_bytes() -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: `usage` points to writable storage for one `rusage`, and
    // `RUSAGE_SELF` asks libc to initialize exactly that value.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: getrusage returned success and initialized the value.
    let usage = unsafe { usage.assume_init() };
    u64::try_from(usage.ru_maxrss).ok()
}
