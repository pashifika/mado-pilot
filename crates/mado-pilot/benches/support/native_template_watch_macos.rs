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
use mado_pilot_platform_macos::sck_suspension_diagnostics as sck_diagnostics;
#[cfg(target_os = "macos")]
use mado_pilot_testkit::sck_suspension_signature_report as sck_signature;
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

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy)]
struct SckSignatureFailure {
    stage: sck_signature::Stage,
    class: sck_signature::FailureClass,
}

#[cfg(target_os = "macos")]
impl SckSignatureFailure {
    const fn new(stage: sck_signature::Stage, class: sck_signature::FailureClass) -> Self {
        Self { stage, class }
    }
}

#[cfg(target_os = "macos")]
fn run_sck_suspension_signature(arguments: &Arguments) {
    use std::io::Write as _;

    let row = collect_sck_suspension_signature(arguments);
    let exit_success = row.exit_success();
    let line = row.to_json_line().expect("protocol_drift");
    println!("{line}");
    std::io::stdout().flush().expect("diagnostic_output_failed");
    if !exit_success {
        std::process::exit(2);
    }
}

#[cfg(target_os = "macos")]
fn collect_sck_suspension_signature(arguments: &Arguments) -> sck_signature::Row {
    validate_sck_signature_arguments(&arguments.raw);
    assert_eq!(
        PathBuf::from(required_value(&arguments.raw, "--fixture-executable")),
        arguments.fixture_executable,
        "protocol_drift"
    );
    let process_index = required_value(&arguments.raw, "--process-index")
        .parse::<u32>()
        .unwrap_or_else(|_| panic!("protocol_drift"));
    let topology = sck_signature::TopologyClass::parse(required_value(
        &arguments.raw,
        "--topology",
    ))
    .unwrap_or_else(|| panic!("protocol_drift"));
    assert_eq!(
        sck_signature::ORDER
            .get(usize::try_from(process_index).unwrap_or_default().saturating_sub(1)),
        Some(&topology),
        "protocol_drift"
    );
    let (provenance, identity_matches) = sck_signature_provenance(arguments);
    let mut row = sck_signature::Row {
        schema_version: sck_signature::SCHEMA_VERSION,
        process_index,
        topology,
        provenance,
        observed_topology: None,
        baseline: None,
        post_old_drop_resources: None,
        final_resources: None,
        owners: sck_signature::RetainedOwners::default(),
        old_fixture_revision: 0,
        operation_deadline_millis: sck_signature::OPERATION_DEADLINE_MILLIS,
        fresh_target_authenticated: None,
        lifecycle: sck_signature::LifecycleTrace::default(),
        fresh_fixture_revision: 0,
        old_frame: None,
        fresh_frame: None,
        streams_distinct: None,
        first_complete_latency_nanos: None,
        old_snapshot: None,
        fresh_snapshot: None,
        stage: sck_signature::Stage::Setup,
        outcome: sck_signature::Outcome::OperationFailed,
        failure: sck_signature::FailureClass::DiagnosticSnapshotFailed,
    };

    let Ok(baseline) = sck_diagnostics::process_resources() else {
        return row;
    };
    row.baseline = Some(report_resources(baseline));
    if !identity_matches {
        row.failure = sck_signature::FailureClass::IdentityMismatch;
        return finish_sck_signature(row);
    }

    let Ok(observed_topology) = sck_diagnostics::display_topology() else {
        row.stage = sck_signature::Stage::TopologyPreflight;
        return finish_sck_signature(row);
    };
    let observed_topology = report_topology(observed_topology);
    row.observed_topology = Some(observed_topology);
    if !sck_signature::topology_matches(topology, observed_topology) {
        row.stage = sck_signature::Stage::TopologyPreflight;
        row.outcome = sck_signature::Outcome::TopologyMismatch;
        row.failure = sck_signature::FailureClass::TopologyMismatch;
        return finish_sck_signature(row);
    }

    match execute_sck_signature_lifecycle(arguments, &mut row) {
        Ok(()) => {}
        Err(failure) => {
            row.stage = failure.stage;
            row.outcome = sck_signature::Outcome::OperationFailed;
            row.failure = failure.class;
        }
    }
    finish_sck_signature(row)
}

#[cfg(target_os = "macos")]
fn finish_sck_signature(mut row: sck_signature::Row) -> sck_signature::Row {
    let Some(baseline) = row.baseline else {
        return row;
    };
    match wait_for_sck_resource_baseline(baseline) {
        Ok(final_resources) => {
            row.final_resources = Some(final_resources);
            if final_resources != baseline {
                row.stage = sck_signature::Stage::Cleanup;
                row.outcome = sck_signature::Outcome::BaselineLeak;
                row.failure = sck_signature::FailureClass::BaselineLeak;
            }
        }
        Err(()) => {
            row.final_resources = None;
            row.stage = sck_signature::Stage::Cleanup;
            row.outcome = sck_signature::Outcome::OperationFailed;
            row.failure = sck_signature::FailureClass::DiagnosticSnapshotFailed;
        }
    }
    row
}

#[cfg(target_os = "macos")]
fn wait_for_sck_resource_baseline(
    baseline: sck_signature::ProcessResources,
) -> Result<sck_signature::ProcessResources, ()> {
    let deadline = Instant::now() + OPERATION_WAIT;
    loop {
        let current =
            report_resources(sck_diagnostics::process_resources().map_err(|_| ())?);
        if current == baseline || Instant::now() >= deadline {
            return Ok(current);
        }
        thread::sleep(POLL_WAIT);
    }
}

#[cfg(target_os = "macos")]
fn execute_sck_signature_lifecycle(
    arguments: &Arguments,
    row: &mut sck_signature::Row,
) -> Result<(), SckSignatureFailure> {
    let mut run = NativeRun::start(arguments).map_err(|error| {
        let class = if error == "capability_unavailable:capture" {
            sck_signature::FailureClass::PermissionUnavailable
        } else if error == "fixture_authority_failed" {
            sck_signature::FailureClass::FixtureUnavailable
        } else {
            sck_signature::FailureClass::TargetUnavailable
        };
        SckSignatureFailure::new(sck_signature::Stage::FixtureLaunch, class)
    })?;

    establish_absent(&mut run).map_err(|_| {
        SckSignatureFailure::new(
            sck_signature::Stage::OldCapture,
            sck_signature::FailureClass::OldCaptureFailed,
        )
    })?;
    let query = run
        .start_watch(TemplateStability::immediate())
        .map_err(|_| {
            SckSignatureFailure::new(
                sck_signature::Stage::OldCapture,
                sck_signature::FailureClass::OldCaptureFailed,
            )
        })?;
    prime_pending(&query).map_err(|_| {
        SckSignatureFailure::new(
            sck_signature::Stage::OldCapture,
            sck_signature::FailureClass::OldCaptureFailed,
        )
    })?;
    let old_ack = run.command_visible().map_err(|_| {
        SckSignatureFailure::new(
            sck_signature::Stage::OldCapture,
            sck_signature::FailureClass::OldCaptureFailed,
        )
    })?;
    row.old_fixture_revision = old_ack.revision;
    let (terminal, _) = wait_terminal(&query).map_err(|_| {
        SckSignatureFailure::new(
            sck_signature::Stage::OldCapture,
            sck_signature::FailureClass::OldCaptureFailed,
        )
    })?;
    let result = terminal_match(&terminal)
        .ok_or_else(|| {
            SckSignatureFailure::new(
                sck_signature::Stage::OldCapture,
                sck_signature::FailureClass::OldCaptureFailed,
            )
        })?
        .clone();
    row.lifecycle
        .record(sck_signature::LifecycleStep::OldResultRetained)
        .expect("protocol_drift");
    let mapping = run
        .session
        .map_frame(result.frame(), PixelFormat::Rgba8, &bounded(OPERATION_WAIT))
        .map_err(|_| {
            SckSignatureFailure::new(
                sck_signature::Stage::OldCapture,
                sck_signature::FailureClass::OldCaptureFailed,
            )
        })?;
    row.lifecycle
        .record(sck_signature::LifecycleStep::OldMappingRetained)
        .expect("protocol_drift");
    let old_stamp = mapping.stamp();
    let old_stream = old_stamp.stream();
    row.old_frame = Some(report_frame_identity(old_stamp));
    let prefix_length = mapping.bytes().len().min(16);
    let mut retained_prefix = [0_u8; 16];
    retained_prefix[..prefix_length].copy_from_slice(&mapping.bytes()[..prefix_length]);
    row.owners.terminal_result = true;
    row.owners.mapping = true;

    row.lifecycle
        .record(sck_signature::LifecycleStep::OldSessionCloseAttempted)
        .expect("protocol_drift");
    run.session.close(&bounded(OPERATION_WAIT)).map_err(|_| {
        SckSignatureFailure::new(
            sck_signature::Stage::OldClose,
            sck_signature::FailureClass::OldCloseFailed,
        )
    })?;
    let old_observer = sck_diagnostics::session_observer(old_stream).map_err(|_| {
        SckSignatureFailure::new(
            sck_signature::Stage::OldOwnerDrop,
            sck_signature::FailureClass::DiagnosticSnapshotFailed,
        )
    })?;
    drop(query);
    drop(terminal);
    drop(run.session);
    drop(run.engine);
    row.lifecycle
        .record(sck_signature::LifecycleStep::OldPublicOwnersDropped)
        .expect("protocol_drift");

    row.old_snapshot = Some(
        old_observer
            .snapshot()
            .map(report_snapshot)
            .map_err(|_| {
                SckSignatureFailure::new(
                    sck_signature::Stage::OldOwnerDrop,
                    sck_signature::FailureClass::DiagnosticSnapshotFailed,
                )
            })?,
    );
    row.post_old_drop_resources = Some(
        sck_diagnostics::process_resources()
            .map(report_resources)
            .map_err(|_| {
                SckSignatureFailure::new(
                    sck_signature::Stage::OldOwnerDrop,
                    sck_signature::FailureClass::DiagnosticSnapshotFailed,
                )
            })?,
    );
    row.lifecycle
        .record(sck_signature::LifecycleStep::OldSnapshotRecorded)
        .expect("protocol_drift");

    let fresh_ack = run.fixture.set_absent().map_err(|_| {
        SckSignatureFailure::new(
            sck_signature::Stage::FreshOpen,
            sck_signature::FailureClass::FixtureUnavailable,
        )
    })?;
    row.fresh_fixture_revision = fresh_ack.revision;
    let fresh_engine = native_engine().map_err(|_| {
        SckSignatureFailure::new(
            sck_signature::Stage::FreshOpen,
            sck_signature::FailureClass::FreshOpenFailed,
        )
    })?;
    let fresh_target = run.fixture.authenticated_target(&fresh_engine).map_err(|_| {
        SckSignatureFailure::new(
            sck_signature::Stage::FreshOpen,
            sck_signature::FailureClass::TargetUnavailable,
        )
    })?;
    row.fresh_target_authenticated = Some(true);
    let fresh_session = fresh_engine
        .open(
            fresh_target,
            &OpenRequest::new(),
            &bounded(OPERATION_WAIT),
        )
        .map_err(|_| {
            row.fresh_snapshot = sck_diagnostics::take_open_failure_snapshot()
                .map(report_snapshot);
            SckSignatureFailure::new(
                sck_signature::Stage::FreshOpen,
                sck_signature::FailureClass::FreshOpenFailed,
            )
        })?;
    row.lifecycle
        .record(sck_signature::LifecycleStep::FreshSessionOpened)
        .expect("protocol_drift");
    row.lifecycle
        .record(sck_signature::LifecycleStep::FreshCaptureAttempted)
        .expect("protocol_drift");
    let fresh_capture =
        fresh_session.acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT));
    let capture_operation_failed = fresh_capture
        .as_ref()
        .is_err_and(|error| error.status() != Status::DeadlineExceeded);
    let fresh_capture = fresh_capture.ok();
    let fresh_stream = fresh_session.stream();
    row.lifecycle
        .record(sck_signature::LifecycleStep::FreshSessionCloseAttempted)
        .expect("protocol_drift");
    let fresh_closed = fresh_session.close(&bounded(OPERATION_WAIT)).is_ok();
    let post_close_snapshot = sck_diagnostics::session_snapshot(fresh_stream)
        .map(report_snapshot)
        .map_err(|_| {
            SckSignatureFailure::new(
                sck_signature::Stage::FreshClose,
                sck_signature::FailureClass::DiagnosticSnapshotFailed,
            )
        })?;
    drop(fresh_session);
    drop(fresh_engine);
    row.fresh_snapshot = Some(post_close_snapshot);
    if !fresh_closed {
        return Err(SckSignatureFailure::new(
            sck_signature::Stage::FreshClose,
            sck_signature::FailureClass::FreshCloseFailed,
        ));
    }

    let retained = mapping.stamp() == old_stamp
        && mapping.bytes()[..prefix_length] == retained_prefix[..prefix_length]
        && result.frame().stamp() == old_stamp;
    if !retained {
        return Err(SckSignatureFailure::new(
            sck_signature::Stage::FreshClose,
            sck_signature::FailureClass::MappingUnreadable,
        ));
    }
    row.owners.mapping_readable_after_fresh_close = true;
    row.streams_distinct = Some(old_stream != fresh_stream);
    row.lifecycle
        .record(sck_signature::LifecycleStep::MappingVerified)
        .expect("protocol_drift");

    match fresh_capture {
        Some(frame) => {
            row.fresh_frame = Some(report_frame_identity(frame.stamp()));
            row.first_complete_latency_nanos = post_close_snapshot
                .first_complete_nanos
                .checked_sub(post_close_snapshot.stream_start_completed_nanos);
            row.stage = sck_signature::Stage::Complete;
            row.outcome = sck_signature::Outcome::CompleteFrame;
            row.failure = sck_signature::FailureClass::None;
        }
        None if capture_operation_failed => {
            row.stage = sck_signature::Stage::FreshCapture;
            row.outcome = sck_signature::Outcome::OperationFailed;
            row.failure = sck_signature::FailureClass::FreshCaptureFailed;
        }
        None => {
            match sck_signature::classify_fresh_timeout(&post_close_snapshot) {
                sck_signature::FreshTimeoutOutcome::ExplicitProducerState => {
                    row.stage = sck_signature::Stage::Complete;
                    row.outcome = sck_signature::Outcome::ExplicitProducerState;
                    row.failure = sck_signature::FailureClass::None;
                }
                sck_signature::FreshTimeoutOutcome::MissingProgress => {
                    row.stage = sck_signature::Stage::FreshCapture;
                    row.outcome = sck_signature::Outcome::MissingProgress;
                    row.failure = sck_signature::FailureClass::FreshCaptureDeadline;
                }
            }
        }
    }

    drop(result);
    drop(mapping);
    row.lifecycle
        .record(sck_signature::LifecycleStep::RetainedOwnersDropped)
        .expect("protocol_drift");
    row.lifecycle
        .record(sck_signature::LifecycleStep::FixtureCloseAttempted)
        .expect("protocol_drift");
    if !run.fixture.finish() {
        return Err(SckSignatureFailure::new(
            sck_signature::Stage::Cleanup,
            sck_signature::FailureClass::CleanupFailed,
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn sck_signature_provenance(
    arguments: &Arguments,
) -> (sck_signature::Provenance, bool) {
    let source_revision = required_digest(&arguments.raw, "--source-revision", 40);
    let source_tree = required_digest(&arguments.raw, "--source-tree", 40);
    let protocol_sha256 = required_digest(&arguments.raw, "--protocol-sha256", 64);
    let executable_sha256 = required_digest(&arguments.raw, "--executable-sha256", 64);
    let fixture_sha256 = required_digest(&arguments.raw, "--fixture-sha256", 64);
    let fixture_source_sha256 =
        required_digest(&arguments.raw, "--fixture-source-sha256", 64);
    let protocol_file = PathBuf::from(required_value(&arguments.raw, "--protocol-file"));
    let actual_executable = std::env::current_exe()
        .ok()
        .and_then(|path| sck_signature::sha256_file(&path).ok());
    let actual_fixture = sck_signature::sha256_file(&arguments.fixture_executable).ok();
    let actual_protocol = sck_signature::sha256_file(&protocol_file).ok();
    let identity_matches = actual_executable.as_deref() == Some(executable_sha256.as_str())
        && actual_fixture.as_deref() == Some(fixture_sha256.as_str())
        && actual_protocol.as_deref() == Some(protocol_sha256.as_str());
    (
        sck_signature::Provenance {
            source_revision,
            source_tree,
            protocol_sha256,
            executable_sha256,
            fixture_sha256,
            fixture_source_sha256,
            host: sck_signature::HostProfile::AppleM1Pro10c32g,
            os: sck_signature::OsProfile::Macos26_6_2Build25g83,
            sdk: sck_signature::SdkProfile::Xcode26_5,
        },
        identity_matches,
    )
}

#[cfg(target_os = "macos")]
fn validate_sck_signature_arguments(arguments: &[String]) {
    const VALUE_ARGUMENTS: [&str; 10] = [
        "--fixture-executable=",
        "--process-index=",
        "--topology=",
        "--source-revision=",
        "--source-tree=",
        "--protocol-file=",
        "--protocol-sha256=",
        "--executable-sha256=",
        "--fixture-sha256=",
        "--fixture-source-sha256=",
    ];
    assert!(
        arguments.iter().all(|argument| {
            argument == "--sck-suspension-signature"
                || VALUE_ARGUMENTS
                    .iter()
                    .any(|prefix| argument.starts_with(prefix))
        }),
        "protocol_drift"
    );
    assert_eq!(
        arguments
            .iter()
            .filter(|argument| argument.as_str() == "--sck-suspension-signature")
            .count(),
        1,
        "protocol_drift"
    );
}

#[cfg(target_os = "macos")]
fn required_value<'a>(arguments: &'a [String], name: &str) -> &'a str {
    let prefix = format!("{name}=");
    let mut values = arguments
        .iter()
        .filter_map(|argument| argument.strip_prefix(&prefix));
    let selected = values.next().unwrap_or_else(|| panic!("protocol_drift"));
    assert!(values.next().is_none(), "protocol_drift");
    selected
}

#[cfg(target_os = "macos")]
fn required_digest(arguments: &[String], name: &str, length: usize) -> String {
    let value = required_value(arguments, name);
    assert!(
        value.len() == length && value.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "protocol_drift"
    );
    value.to_owned()
}

#[cfg(target_os = "macos")]
const fn report_frame_identity(stamp: FrameStamp) -> sck_signature::FrameIdentity {
    sck_signature::FrameIdentity {
        stream_ordinal: stamp.stream().get(),
        epoch: stamp.epoch().value(),
        sequence: stamp.sequence().value(),
        geometry_revision: stamp.geometry().value(),
    }
}

#[cfg(target_os = "macos")]
fn report_topology(topology: sck_diagnostics::DisplayTopology) -> sck_signature::DisplayTopology {
    sck_signature::DisplayTopology {
        display_count: topology.display_count,
        has_distinct_backing_scales: topology.has_distinct_backing_scales,
    }
}

#[cfg(target_os = "macos")]
const fn report_resources(
    resources: sck_diagnostics::ProcessResources,
) -> sck_signature::ProcessResources {
    sck_signature::ProcessResources {
        native_objects: resources.native_objects,
        detached_bytes: resources.detached_bytes,
        live_sessions: resources.live_sessions,
        callbacks_in_flight: resources.callbacks_in_flight,
    }
}

#[cfg(target_os = "macos")]
fn report_snapshot(snapshot: sck_diagnostics::Snapshot) -> sck_signature::Snapshot {
    sck_signature::Snapshot {
        close_phase: snapshot.close_phase,
        active_native_slots: snapshot.active_native_slots,
        observed_total: snapshot.observed_total,
        status_counts: snapshot.status_counts,
        first_status: snapshot.first_status.map(report_status_event),
        last_status: snapshot.last_status.map(report_status_event),
        callbacks_received: snapshot.callbacks_received,
        callbacks_admitted: snapshot.callbacks_admitted,
        callbacks_refused: snapshot.callbacks_refused,
        callbacks_exited: snapshot.callbacks_exited,
        stream_start_completed_nanos: snapshot.stream_start_completed_nanos,
        stream_stop_requested_nanos: snapshot.stream_stop_requested_nanos,
        stream_stop_completed_nanos: snapshot.stream_stop_completed_nanos,
        callback_admission_stopped_nanos: snapshot.callback_admission_stopped_nanos,
        callback_fence_completed_nanos: snapshot.callback_fence_completed_nanos,
        close_completed_nanos: snapshot.close_completed_nanos,
        first_complete_nanos: snapshot.first_complete_nanos,
        session_references: snapshot.session_references,
        detached_leases: snapshot.detached_leases,
        native_objects: snapshot.native_objects,
        detached_bytes: snapshot.detached_bytes,
        transition_overwrites: snapshot.transition_overwrites,
        transition_count: u32::try_from(snapshot.transition_count).unwrap_or(u32::MAX),
        transitions: snapshot.transitions.map(|event| event.map(report_status_event)),
    }
}

#[cfg(target_os = "macos")]
const fn report_status_event(
    event: sck_diagnostics::StatusEvent,
) -> sck_signature::StatusEvent {
    sck_signature::StatusEvent {
        kind: event.kind as u32,
        raw_value: event.raw_value,
        sequence: event.sequence,
        monotonic_nanos: event.monotonic_nanos,
    }
}
