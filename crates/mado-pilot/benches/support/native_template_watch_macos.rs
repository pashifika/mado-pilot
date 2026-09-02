// macOS transport and target-owner binding for the shared native watcher harness.


#[cfg(target_os = "macos")]
use crate::macos_fixture::{
    FixtureController, FixtureFinalization, FixtureProcessLifetimeFact, LaunchMode,
    controlled_content_logical_size, controlled_resize_logical_size_matches,
    expected_controlled_resize_logical_size, finalize_once,
};
#[cfg(target_os = "macos")]
use crate::macos_fixture_control::{
    FixtureCleanupCounts, executable_identity, fixture_cleanup_counts,
};
#[cfg(target_os = "macos")]
use crate::macos_fixture_protocol::{self as protocol, FixtureCommandKind};

#[cfg(target_os = "macos")]
const FIXTURE_STATUS_OK: u32 = 0;
#[cfg(target_os = "macos")]
const FIXTURE_STATUS_UNSUPPORTED: u32 = 2;

#[cfg(target_os = "macos")]
#[must_use = "fixture finalization must be checked before accepting a sample"]
#[derive(Clone, Copy)]
struct NativeFixtureFinalization {
    events_drained: bool,
    controller: FixtureFinalization,
    resources: NativeResourceFacts,
}

#[cfg(target_os = "macos")]
impl NativeFixtureFinalization {
    fn is_accepted(&self) -> bool {
        self.events_drained
            && self.controller.is_accepted()
            && self.resources.baseline_observed
            && self.resources.apple_cleanup_active == Some(0)
            && self.resources.apple_cleanup_scheduled
                == self.resources.apple_cleanup_completed
            && self.resources.apple_cleanup_exhausted == Some(0)
    }

    const fn resources(&self) -> NativeResourceFacts {
        self.resources
    }
}

fn native_lifetime_fact(value: FixtureProcessLifetimeFact) -> NativeProcessLifetimeFact {
    match value {
        FixtureProcessLifetimeFact::NotObserved => NativeProcessLifetimeFact::NotObserved,
        FixtureProcessLifetimeFact::Unknown => NativeProcessLifetimeFact::Unknown,
        FixtureProcessLifetimeFact::Live => NativeProcessLifetimeFact::Live,
        FixtureProcessLifetimeFact::Lost => NativeProcessLifetimeFact::Lost,
        FixtureProcessLifetimeFact::ObservationFailed => {
            NativeProcessLifetimeFact::ObservationFailed
        }
    }
}

#[cfg(target_os = "macos")]
struct NativeFixture {
    controller: FixtureController,
    generation: u64,
    revision: u64,
    cleanup_baseline: FixtureCleanupCounts,
    finish_result: Option<NativeFixtureFinalization>,
}

#[cfg(target_os = "macos")]
impl NativeFixture {
    fn start(arguments: &Arguments) -> Result<Self, String> {
        let cleanup_baseline =
            fixture_cleanup_counts().map_err(|_| "fixture_authority_failed".to_owned())?;
        let bytes = std::fs::read(&arguments.fixture_executable)
            .map_err(|_| "fixture_authority_failed".to_owned())?;
        if !fixture_bytes_match(arguments, &bytes) {
            return Err("fixture_authority_failed".to_owned());
        }
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
            cleanup_baseline,
            finish_result: None,
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
        let result = acknowledgement.result();
        let status = result.status;
        if kind == FixtureCommandKind::MoveToNextDisplay
            && status == FIXTURE_STATUS_UNSUPPORTED
        {
            return Err("capability_unavailable:topology".to_owned());
        }
        if status != FIXTURE_STATUS_OK {
            return Err("fixture_authority_failed".to_owned());
        }
        let visual_token = match kind {
            FixtureCommandKind::SetVisualAbsent => Some(VisualMarkerState::Absent),
            FixtureCommandKind::SetVisualVisible => Some(VisualMarkerState::Visible),
            _ => None,
        }
        .map(|marker| {
            let token = protocol::visual_token_for_command(kind, result.nonce)
                .ok_or_else(|| "fixture_authority_failed".to_owned())?;
            VisualToken::new(token.get(), marker)
                .ok_or_else(|| "fixture_authority_failed".to_owned())
        })
        .transpose()?;
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| "fixture_authority_failed".to_owned())?;
        Ok(ControlAcknowledgement {
            generation: self.generation,
            revision: self.revision,
            visual_token,
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

    fn finish(&mut self) -> NativeFixtureFinalization {
        let controller = &mut self.controller;
        let baseline = self.cleanup_baseline;
        finalize_once(&mut self.finish_result, || {
            let events_drained = controller.discard_watch_events(FIXTURE_WAIT);
            let controller = controller.finish(FIXTURE_WAIT);
            let cleanup = fixture_cleanup_counts().ok();
            let resources = NativeResourceFacts {
                baseline_observed: cleanup.is_some(),
                fixture_process_reaped: controller.process_stopped(),
                fixture_reader_joined: controller.reader_joined(),
                protocol_stop_acknowledged: Some(controller.stop_acknowledged()),
                authenticated_lifetime: Some(native_lifetime_fact(
                    controller.authenticated_lifetime(),
                )),
                launched_lifetime: Some(native_lifetime_fact(controller.launched_lifetime())),
                bounded_containment: controller.bounded(),
                output_drained: events_drained && controller.output_clean(),
                executable_identity_unchanged: Some(controller.executable_unchanged()),
                cleanup_debt: Some(if controller.has_cleanup_debt() {
                    NativeCleanupDebtFact::Deferred
                } else {
                    NativeCleanupDebtFact::None
                }),
                apple_launch_accepted_live: Some(true),
                apple_cleanup_scheduled: cleanup
                    .map(|counts| counts.scheduled.saturating_sub(baseline.scheduled)),
                apple_cleanup_active: cleanup.map(|counts| counts.active),
                apple_cleanup_completed: cleanup
                    .map(|counts| counts.completed.saturating_sub(baseline.completed)),
                apple_cleanup_exhausted: cleanup
                    .map(|counts| counts.exhausted.saturating_sub(baseline.exhausted)),
            };
            NativeFixtureFinalization {
                events_drained,
                controller,
                resources,
            }
        })
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
fn content_cell_shape(
    frame: &Frame,
    logical_x: f64,
    logical_y: f64,
    logical_cell: f64,
) -> Option<(u32, u32, i32, i32)> {
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
    Some((
        scaled_u32(logical_cell, scale.x())?,
        scaled_u32(logical_cell, scale.y())?,
        scaled_i32(horizontal_inset + logical_x, scale.x())?,
        scaled_i32(top_inset + logical_y, scale.y())?,
    ))
}

#[cfg(target_os = "macos")]
fn marker_shape(frame: &Frame, _fixture: &NativeFixture) -> Option<MarkerShape> {
    let (cell_width, cell_height, origin_x, origin_y) =
        content_cell_shape(frame, MARKER_X_LOGICAL, MARKER_Y_LOGICAL, MARKER_CELL_LOGICAL)?;
    Some(MarkerShape {
        cell_width,
        cell_height,
        origin_x,
        origin_y,
    })
}

#[cfg(target_os = "macos")]
fn token_shape(frame: &Frame, _fixture: &NativeFixture) -> Option<TokenShape> {
    let (cell_width, cell_height, origin_x, origin_y) =
        content_cell_shape(frame, TOKEN_X_LOGICAL, TOKEN_Y_LOGICAL, TOKEN_CELL_LOGICAL)?;
    Some(TokenShape {
        cell_width,
        cell_height,
        origin_x,
        origin_y,
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
