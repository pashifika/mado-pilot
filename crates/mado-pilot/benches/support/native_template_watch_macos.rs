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
use mado_pilot_testkit::sck_suspension_report;

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
struct DiagnosticPolicyReset;

#[cfg(target_os = "macos")]
impl DiagnosticPolicyReset {
    fn install(policy: sck_suspension_report::DrainPolicy) -> Self {
        let native = match policy {
            sck_suspension_report::DrainPolicy::Unchanged => {
                sck_diagnostics::SampleQueueDrainPolicy::Unchanged
            }
            sck_suspension_report::DrainPolicy::DrainSampleQueue => {
                sck_diagnostics::SampleQueueDrainPolicy::DrainSampleQueue
            }
        };
        sck_diagnostics::set_sample_queue_drain_policy(native);
        Self
    }
}

#[cfg(target_os = "macos")]
impl Drop for DiagnosticPolicyReset {
    fn drop(&mut self) {
        sck_diagnostics::set_sample_queue_drain_policy(
            sck_diagnostics::SampleQueueDrainPolicy::Unchanged,
        );
    }
}

#[cfg(target_os = "macos")]
fn diagnostic_status_event(
    event: sck_diagnostics::StatusEvent,
) -> sck_suspension_report::StatusEvent {
    sck_suspension_report::StatusEvent {
        kind: event.kind as u32,
        raw_value: event.raw_value,
        sequence: event.sequence,
        monotonic_nanos: event.monotonic_nanos,
    }
}

#[cfg(target_os = "macos")]
fn diagnostic_snapshot(
    snapshot: sck_diagnostics::Snapshot,
) -> sck_suspension_report::Snapshot {
    sck_suspension_report::Snapshot {
        close_phase: snapshot.close_phase,
        active_native_slots: snapshot.active_native_slots,
        drain_policy: snapshot.drain_policy,
        observed_total: snapshot.observed_total,
        status_counts: snapshot.status_counts,
        first_status: snapshot.first_status.map(diagnostic_status_event),
        last_status: snapshot.last_status.map(diagnostic_status_event),
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
        drain_request_generation: snapshot.drain_request_generation,
        drain_completion_generation: snapshot.drain_completion_generation,
        drain_requested_nanos: snapshot.drain_requested_nanos,
        drain_completed_nanos: snapshot.drain_completed_nanos,
        transition_overwrites: snapshot.transition_overwrites,
        transition_count: u32::try_from(snapshot.transition_count)
            .expect("fixed transition count fits u32"),
        transitions: snapshot
            .transitions
            .map(|event| event.map(diagnostic_status_event)),
    }
}

#[cfg(target_os = "macos")]
fn diagnostic_process_resources() -> sck_suspension_report::ProcessResources {
    let resources =
        sck_diagnostics::process_resources().expect("sck_diagnostic_resource_snapshot_failed");
    sck_suspension_report::ProcessResources {
        native_objects: resources.native_objects,
        detached_bytes: resources.detached_bytes,
        live_sessions: resources.live_sessions,
        callbacks_in_flight: resources.callbacks_in_flight,
    }
}

#[cfg(target_os = "macos")]
fn diagnostic_session_snapshot(session: &Session) -> Result<sck_suspension_report::Snapshot, String> {
    sck_diagnostics::session_snapshot(session.stream())
        .map(diagnostic_snapshot)
        .map_err(|_| "diagnostic_snapshot_failed".to_owned())
}

#[cfg(target_os = "macos")]
fn settled_diagnostic_session_snapshot(
    session: &Session,
) -> Result<sck_suspension_report::Snapshot, String> {
    let deadline = Instant::now() + FIXTURE_WAIT;
    let mut previous = None;
    loop {
        let snapshot = diagnostic_session_snapshot(session)?;
        let callbacks_settled = snapshot.callbacks_received == snapshot.callbacks_exited
            && snapshot.callbacks_received
                == snapshot
                    .callbacks_admitted
                    .saturating_add(snapshot.callbacks_refused);
        if callbacks_settled && previous.as_ref() == Some(&snapshot) {
            return Ok(snapshot);
        }
        if Instant::now() >= deadline {
            return Err("diagnostic_snapshot_failed".to_owned());
        }
        previous = Some(snapshot);
        thread::sleep(POLL_WAIT);
    }
}

#[cfg(target_os = "macos")]
fn settle_diagnostic_resources(
    baseline: sck_suspension_report::ProcessResources,
) -> sck_suspension_report::ProcessResources {
    let deadline = Instant::now() + FIXTURE_WAIT;
    loop {
        let resources = diagnostic_process_resources();
        if resources == baseline || Instant::now() >= deadline {
            return resources;
        }
        thread::sleep(POLL_WAIT);
    }
}

#[cfg(target_os = "macos")]
fn required_exact_identity(arguments: &[String], name: &str, length: usize) -> String {
    let identity = value(arguments, name).unwrap_or_else(|| panic!("identity_mismatch"));
    assert!(
        identity.len() == length && identity.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "identity_mismatch"
    );
    identity.to_owned()
}

#[cfg(target_os = "macos")]
fn sck_diagnostic_provenance(arguments: &Arguments) -> sck_suspension_report::Provenance {
    assert_eq!(
        required_enum(
            &arguments.raw,
            "--host-class",
            &["apple-m1-pro-10c-32g"]
        ),
        "apple-m1-pro-10c-32g"
    );
    assert_eq!(
        required_enum(
            &arguments.raw,
            "--os-profile",
            &["macos-26.6.2-build-25G83"]
        ),
        "macos-26.6.2-build-25G83"
    );
    assert_eq!(
        required_enum(&arguments.raw, "--sdk-profile", &["xcode-26.5"]),
        "xcode-26.5"
    );
    let topology = match required_enum(
        &arguments.raw,
        "--topology",
        &["single-display", "unsupported"],
    )
    .as_str()
    {
        "single-display" => sck_suspension_report::Topology::SingleDisplay,
        "unsupported" => sck_suspension_report::Topology::Unsupported,
        _ => unreachable!("required_enum returned an unlisted topology"),
    };
    let executable_sha256 =
        required_exact_identity(&arguments.raw, "--executable-sha256", 64);
    let actual_executable_sha256 = sck_suspension_report::sha256_file(
        &std::env::current_exe().unwrap_or_else(|_| panic!("identity_mismatch")),
    )
    .unwrap_or_else(|_| panic!("identity_mismatch"));
    assert_eq!(executable_sha256, actual_executable_sha256, "identity_mismatch");
    let fixture_sha256 = required_exact_identity(&arguments.raw, "--fixture-sha256", 64);
    let actual_fixture_sha256 =
        sck_suspension_report::sha256_file(&arguments.fixture_executable)
            .unwrap_or_else(|_| panic!("identity_mismatch"));
    assert_eq!(fixture_sha256, actual_fixture_sha256, "identity_mismatch");
    sck_suspension_report::Provenance {
        source_revision: required_exact_identity(&arguments.raw, "--source-revision", 40),
        source_tree: required_exact_identity(&arguments.raw, "--source-tree", 40),
        executable_sha256,
        fixture_sha256,
        fixture_source_sha256: required_exact_identity(
            &arguments.raw,
            "--fixture-source-sha256",
            64,
        ),
        host: sck_suspension_report::HostProfile::AppleM1Pro10c32g,
        os: sck_suspension_report::OsProfile::Macos26_6_2Build25g83,
        sdk: sck_suspension_report::SdkProfile::Xcode26_5,
        topology,
    }
}

#[cfg(target_os = "macos")]
fn run_sck_suspension_diagnostic(arguments: &Arguments) {
    let variant = sck_suspension_report::RetentionVariant::parse(
        value(&arguments.raw, "--retention-variant")
            .unwrap_or_else(|| panic!("identity_mismatch")),
    )
    .unwrap_or_else(|| panic!("identity_mismatch"));
    let policy = sck_suspension_report::DrainPolicy::parse(
        value(&arguments.raw, "--drain-policy").unwrap_or_else(|| panic!("identity_mismatch")),
    )
    .unwrap_or_else(|| panic!("identity_mismatch"));
    let process_index = value(&arguments.raw, "--process-index")
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or_else(|| panic!("identity_mismatch"));
    let index = usize::try_from(process_index)
        .ok()
        .and_then(|index| index.checked_sub(1))
        .filter(|index| *index < sck_suspension_report::PROCESS_COUNT)
        .unwrap_or_else(|| panic!("identity_mismatch"));
    assert_eq!(
        sck_suspension_report::ORDER[index],
        (policy, variant),
        "protocol_drift"
    );

    let baseline = diagnostic_process_resources();
    let provenance = sck_diagnostic_provenance(arguments);
    let unsupported_topology = matches!(
        provenance.topology,
        sck_suspension_report::Topology::Unsupported
    );
    let mut row = sck_suspension_report::Row {
        schema_version: sck_suspension_report::SCHEMA_VERSION,
        process_index,
        policy,
        variant,
        provenance,
        baseline,
        final_resources: baseline,
        owners: sck_suspension_report::RetainedOwners::default(),
        fixture_revision: 0,
        first_complete_latency_nanos: None,
        old_snapshot: None,
        fresh_snapshot: None,
        stage: sck_suspension_report::Stage::Setup,
        outcome: if unsupported_topology {
            sck_suspension_report::Outcome::UnsupportedTopology
        } else {
            sck_suspension_report::Outcome::OperationFailed
        },
    };

    let diagnostic_result = if matches!(
        row.outcome,
        sck_suspension_report::Outcome::UnsupportedTopology
    ) {
        Err("capability_unavailable:topology".to_owned())
    } else {
        let _policy_reset = DiagnosticPolicyReset::install(policy);
        execute_sck_suspension_diagnostic(arguments, &mut row)
    };
    if diagnostic_result.is_err()
        && let Some(snapshot) = sck_diagnostics::take_open_failure_snapshot()
    {
        let snapshot = diagnostic_snapshot(snapshot);
        if matches!(
            row.stage,
            sck_suspension_report::Stage::OldCapture
                | sck_suspension_report::Stage::OldClose
        ) {
            row.old_snapshot.get_or_insert(snapshot);
        } else {
            row.fresh_snapshot.get_or_insert(snapshot);
        }
    }
    row.final_resources = settle_diagnostic_resources(baseline);
    let line = row
        .to_json_line()
        .unwrap_or_else(|_| panic!("privacy_violation"));
    println!("{line}");
    assert!(
        diagnostic_result.is_ok()
            && row.completed()
            && sck_suspension_report::validate_row(&row).is_ok(),
        "sck_suspension_diagnostic_failed"
    );
}

#[cfg(target_os = "macos")]
fn execute_sck_suspension_diagnostic(
    arguments: &Arguments,
    row: &mut sck_suspension_report::Row,
) -> Result<(), String> {
    row.stage = sck_suspension_report::Stage::OldCapture;
    let mut run = NativeRun::start(arguments)?;
    macro_rules! old_try {
        ($expression:expr) => {
            match $expression {
                Ok(value) => value,
                Err(error) => {
                    row.old_snapshot = diagnostic_session_snapshot(&run.session).ok();
                    return Err(error);
                }
            }
        };
    }

    let absent = old_try!(establish_absent(&mut run));
    drop(absent);
    let query = old_try!(run.start_watch(TemplateStability::immediate()));
    old_try!(prime_pending(&query));
    old_try!(run.command_visible());
    let (terminal, _) = old_try!(wait_terminal(&query));
    if !terminal.is_match() {
        row.old_snapshot = diagnostic_session_snapshot(&run.session).ok();
        return Err("wrong_match".to_owned());
    }
    let source_frame = old_try!(
        terminal_match(&terminal)
            .ok_or_else(|| "wrong_match".to_owned())
            .map(|matched| matched.frame().clone())
    );
    let mapping = if row.variant.retains_mapping() {
        Some(old_try!(
            run.session
                .map_frame(
                    &source_frame,
                    PixelFormat::Rgba8,
                    &bounded(OPERATION_WAIT),
                )
                .map_err(|_| "ownership_pinned".to_owned())
        ))
    } else {
        None
    };
    let mapping_stamp = mapping.as_ref().map(mado_pilot::CpuMapping::stamp);
    let mapping_prefix = mapping.as_ref().map(|mapping| {
        mapping
            .bytes()
            .get(..mapping.bytes().len().min(16))
            .expect("bounded prefix")
            .to_vec()
    });
    let retained_frame = row.variant.retains_frame().then_some(source_frame);
    row.owners.frame = retained_frame.is_some();
    row.owners.mapping = mapping.is_some();
    let old_observer = run.session.mapping_observer();

    drop(query);
    drop(terminal);
    old_try!(
        run.session
            .benchmark_wait_template_watcher_idle(&bounded(OPERATION_WAIT))
            .map_err(|error| format!("typed_operation_failure:{:?}", error.status()))
    );

    row.stage = sck_suspension_report::Stage::OldClose;
    let old_close = run
        .session
        .close(&bounded(OPERATION_WAIT))
        .map_err(|_| "cleanup_failed".to_owned());
    let last_ack = run.last_ack;
    let NativeRun {
        mut fixture,
        engine,
        session: old_session,
        ..
    } = run;
    drop(engine);
    row.old_snapshot = Some(diagnostic_session_snapshot(&old_session)?);
    old_close?;

    row.stage = sck_suspension_report::Stage::FreshOpen;
    let fresh_engine = native_engine().map_err(|_| "capability_unavailable:capture".to_owned())?;
    let fresh_target = fixture.authenticated_target(&fresh_engine)?;
    let fresh_session = fresh_engine
        .open(
            fresh_target,
            &OpenRequest::new(),
            &bounded(OPERATION_WAIT),
        )
        .map_err(|_| "producer_stalled".to_owned())?;
    macro_rules! fresh_try {
        ($expression:expr) => {
            match $expression {
                Ok(value) => value,
                Err(error) => {
                    row.fresh_snapshot = diagnostic_session_snapshot(&fresh_session).ok();
                    return Err(error);
                }
            }
        };
    }

    row.stage = sck_suspension_report::Stage::FixtureRevision;
    let acknowledgement = fresh_try!(fixture.set_absent());
    if acknowledgement.generation != last_ack.generation
        || acknowledgement.revision <= last_ack.revision
    {
        row.fresh_snapshot = diagnostic_session_snapshot(&fresh_session).ok();
        return Err("fixture_authority_failed".to_owned());
    }
    row.fixture_revision = acknowledgement.revision;

    row.stage = sck_suspension_report::Stage::FreshCapture;
    let fresh_complete = wait_marker_state_on(&fresh_session, &fixture, None, false).is_ok();
    let open_snapshot = fresh_try!(
        sck_diagnostics::session_snapshot(fresh_session.stream())
            .map_err(|_| "diagnostic_snapshot_failed".to_owned())
    );

    row.stage = sck_suspension_report::Stage::FreshClose;
    let fresh_close = fresh_session
        .close(&bounded(OPERATION_WAIT))
        .map_err(|_| "cleanup_failed".to_owned());
    let fresh_snapshot = diagnostic_session_snapshot(&fresh_session)?;
    row.first_complete_latency_nanos = (fresh_snapshot.first_complete_nanos != 0
        && fresh_snapshot.stream_start_completed_nanos != 0)
        .then(|| {
            fresh_snapshot
                .first_complete_nanos
                .saturating_sub(fresh_snapshot.stream_start_completed_nanos)
        });
    row.fresh_snapshot = Some(fresh_snapshot);
    fresh_close?;
    row.fresh_snapshot = Some(settled_diagnostic_session_snapshot(&fresh_session)?);
    drop(fresh_session);
    drop(fresh_engine);

    row.old_snapshot = Some(settled_diagnostic_session_snapshot(&old_session)?);
    row.stage = sck_suspension_report::Stage::Cleanup;
    row.owners.frame_readable_after_fresh_close = retained_frame.as_ref().is_some_and(|frame| {
        old_observer
            .map_frame(frame, PixelFormat::Rgba8, &bounded(OPERATION_WAIT))
            .is_ok_and(|mapping| mapping.stamp() == frame.stamp() && !mapping.bytes().is_empty())
    });
    row.owners.mapping_readable_after_fresh_close = mapping.as_ref().is_some_and(|mapping| {
        mapping_stamp == Some(mapping.stamp())
            && mapping_prefix
                .as_deref()
                .is_some_and(|prefix| mapping.bytes().starts_with(prefix))
    });
    drop(retained_frame);
    drop(mapping);
    drop(old_session);
    if !fixture.finish() {
        return Err("cleanup_failed".to_owned());
    }
    drop(fixture);

    row.outcome = if fresh_complete {
        sck_suspension_report::Outcome::CompleteFrame
    } else if open_snapshot.status_count(sck_diagnostics::StatusKind::Complete) == 0
        && open_snapshot.last_status.is_some()
    {
        sck_suspension_report::Outcome::ExplicitProducerState
    } else {
        return Err("producer_stalled".to_owned());
    };
    row.stage = sck_suspension_report::Stage::Complete;
    Ok(())
}
