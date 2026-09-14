//! Real owned-window scenarios and five fresh-process comparison workloads.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Duration;

use mado_pilot::{
    CancellationToken, CapturePacingOutcome, CapturePacingReport, CapturePacingRequest, CpuMapping,
    DefaultOcrConfig, Engine, Frame, FrameOrder, FrameRequest, NativeEngineRequest,
    OcrBackendDescriptor, OcrExecutionProvider, OpenRequest, OperationContext, PixelExtent,
    ResolvedCapturePacing, Session, Status, TargetDescription, TargetId, TargetKind,
};

use super::completion_cooldown::{checkpoint, cooldown};
use super::consumption::{self, Acquisition, Cursor, Observation, Recognition};
use super::contract::{
    Arguments, CLOSE_TIMEOUT, CONTROL_TIMEOUT, COOLDOWN, Case, Check, Failure, Metrics,
    NATIVE_INTERVAL, PROCESS_LIMIT, Pacing, Report, api, bounded, nanos, require,
};
use super::control::{Ack, Fixture};
use super::metrics::{self, Sampler};

const WARMUP: Duration = Duration::from_secs(2);
const MEASUREMENT: Duration = Duration::from_secs(6);

#[derive(Default)]
struct Resources {
    fixture: Option<Fixture>,
    engine: Option<Engine>,
    session: Option<Session>,
    sampler: Option<Sampler>,
    diagnostics: bool,
    capture_cleanup_failed: bool,
}

impl Resources {
    fn close_session(&mut self) -> Check<()> {
        let Some(session) = self.session.take() else {
            return Ok(());
        };
        let operation = api(OperationContext::new().with_timeout(CLOSE_TIMEOUT))?;
        let result = api(session.close(&operation));
        if result.is_err() {
            self.capture_cleanup_failed = true;
        }
        drop(session);
        result
    }

    fn finish_sampler(&mut self, report: &mut Report) -> Check<()> {
        let Some(sampler) = self.sampler.take() else {
            return Ok(());
        };
        let sampled = sampler
            .finish()
            .map_err(|_| Failure::Rule("sampler-finish"))?;
        let lossless = sampled.sample_losses == 0;
        report.metrics.gpu_reason = sampled.gpu_reason.clone();
        report.metrics.process = Some(sampled);
        report.check("sample_loss", lossless)
    }

    fn cleanup(&mut self, report: &mut Report) -> Check<()> {
        let sampler = self.finish_sampler(report);
        let close = self.close_session();
        drop(self.engine.take());
        let diagnostics = if std::mem::take(&mut self.diagnostics) {
            metrics::finish_capture_diagnostics()
                .map_err(|_| Failure::Rule("capture-diagnostics-finish"))
        } else {
            Ok(())
        };
        let capture_clean =
            !self.capture_cleanup_failed && close.is_ok() && diagnostics.is_ok() && sampler.is_ok();
        report.checks.insert(
            "capture_cleanup",
            if capture_clean { "pass" } else { "fail" },
        );
        let fixture = self.fixture.as_mut().map_or(Ok(()), Fixture::shutdown);
        report.checks.insert(
            "fixture_cleanup",
            if fixture.is_ok() { "pass" } else { "fail" },
        );
        drop(self.fixture.take());
        report.cleanup_status = if capture_clean && fixture.is_ok() {
            "pass"
        } else {
            "fail"
        };
        sampler.and(close).and(diagnostics).and(fixture)
    }
}

impl Drop for Resources {
    fn drop(&mut self) {
        // A panic has no successful report. Still drain owned resources before unwinding.
        if let Some(sampler) = self.sampler.take() {
            let _ = sampler.finish();
        }
        let _ = self.close_session();
        drop(self.engine.take());
        if std::mem::take(&mut self.diagnostics) {
            let _ = metrics::finish_capture_diagnostics();
        }
        if let Some(fixture) = self.fixture.as_mut() {
            let _ = fixture.shutdown();
        }
    }
}

#[cfg(target_os = "macos")]
fn permission(operation: &OperationContext, report: &mut Report) -> Check<()> {
    use mado_pilot::{PermissionKind, PermissionState};
    // This constructor has no OCR model and requests no permission.
    let probe = api(mado_pilot::macos_engine(NativeEngineRequest::new()))?;
    let outcome = api(probe.permission(PermissionKind::ScreenCapture, operation))?;
    report.permission = match outcome.state() {
        PermissionState::Granted => "granted",
        PermissionState::NotGranted | PermissionState::Unknown => "denied-or-undetermined",
        _ => "unavailable",
    };
    drop(probe);
    if outcome.is_granted() {
        Ok(())
    } else {
        Err(Failure::NotRun("screen-capture-not-granted"))
    }
}

#[cfg(windows)]
fn permission(operation: &OperationContext, report: &mut Report) -> Check<()> {
    api(checkpoint(operation))?;
    report.permission = "not-required";
    Ok(())
}

fn engine(arguments: &Arguments, operation: &OperationContext) -> Check<Engine> {
    api(checkpoint(operation))?;
    let model = arguments
        .model
        .canonicalize()
        .map_err(|_| Failure::NotRun("model-unavailable"))?;
    let runtime = arguments
        .runtime
        .canonicalize()
        .map_err(|_| Failure::NotRun("runtime-unavailable"))?;
    require(model.is_dir() && runtime.is_file(), "model-prerequisite")?;
    let config = DefaultOcrConfig::new(model, runtime);
    let request = NativeEngineRequest::new().with_capture_pacing(arguments.case.pacing()?);
    #[cfg(target_os = "macos")]
    let result = mado_pilot::macos_engine_with_default_ocr(request, &config, operation);
    #[cfg(windows)]
    let result = mado_pilot::windows_engine_with_default_ocr(request, &config, operation);
    api(result)
}

fn select(targets: &[TargetDescription], title: &str) -> Check<TargetId> {
    let mut matching = targets.iter().filter(|target| {
        target.name() == title && target.capability().kind() == Some(TargetKind::Window)
    });
    let target = matching
        .next()
        .ok_or(Failure::Rule("owned-target-absent"))?;
    require(matching.next().is_none(), "owned-target-ambiguous")?;
    Ok(target.id())
}

fn pacing(
    actual: CapturePacingReport,
    expected: ResolvedCapturePacing,
    report: &mut Report,
) -> Check<()> {
    require(report.pacing.len() < 8, "pacing-report-capacity")?;
    let selection = if expected.is_required() {
        "required"
    } else if expected.is_preferred() {
        "preferred"
    } else {
        "source-default"
    };
    let outcome = match actual.outcome() {
        CapturePacingOutcome::SourceDefault => "source-default",
        CapturePacingOutcome::Applied => "applied",
        CapturePacingOutcome::PreferredUnapplied(_) => "preferred-unapplied",
        _ => return Err(Failure::Rule("pacing-report-outcome")),
    };
    report.pacing.push(Pacing {
        selection,
        requested_ns: actual.request().interval().map(nanos).transpose()?,
        configured_ns: actual.configured_interval().map(nanos).transpose()?,
        outcome,
    });
    let applied = match actual.outcome() {
        CapturePacingOutcome::SourceDefault => {
            expected.interval().is_none() && actual.configured_interval().is_none()
        }
        CapturePacingOutcome::Applied => actual
            .configured_interval()
            .zip(expected.interval())
            .is_some_and(|(configured, requested)| configured >= requested),
        CapturePacingOutcome::PreferredUnapplied(_) => {
            expected.is_preferred() && actual.configured_interval().is_none()
        }
        _ => false,
    };
    report.check("pacing_report", actual.request() == expected && applied)
}

fn open(
    resources: &mut Resources,
    target: TargetId,
    request: CapturePacingRequest,
    inherited: CapturePacingRequest,
    operation: &OperationContext,
    report: &mut Report,
) -> Check<Observation> {
    require(resources.session.is_none(), "session-owner-overlap")?;
    let engine = resources
        .engine
        .as_ref()
        .ok_or(Failure::Rule("engine-owner"))?;
    resources.session = Some(api(engine.open(
        target,
        &OpenRequest::new().with_capture_pacing(request),
        operation,
    ))?);
    let session = resources
        .session
        .as_ref()
        .ok_or(Failure::Rule("session-owner"))?;
    let expected = request.resolve(inherited.resolve(ResolvedCapturePacing::source_default()));
    pacing(session.description().capture_pacing(), expected, report)?;
    require(
        session.target() == target && !session.accepts_input(),
        "capture-only-session",
    )?;
    let fixture = resources
        .fixture
        .as_mut()
        .ok_or(Failure::Rule("fixture-owner"))?;
    let ack = fixture.command("pulse", operation)?;
    consumption::acquire(
        session,
        fixture,
        Acquisition {
            after: None,
            expected: ack,
            exact_counter: true,
            retained_bytes: 0,
        },
        &bounded(operation, CONTROL_TIMEOUT)?,
        report,
    )
}

fn cooldown_command(
    fixture: &mut Fixture,
    command: &'static str,
    operation: &OperationContext,
) -> Check<Ack> {
    let mut command_result = None;
    let wait = cooldown(COOLDOWN, operation, &mut |slice| {
        if command_result.is_none() {
            command_result = Some(fixture.command(command, operation));
        }
        std::thread::sleep(slice);
    });
    let ack = command_result.ok_or(Failure::Rule("cooldown-command-not-reached"))??;
    api(wait)?;
    Ok(ack)
}

fn known_after_cooldown(
    session: &Session,
    fixture: &mut Fixture,
    request: Acquisition,
    operation: &OperationContext,
    report: &mut Report,
) -> Check<Observation> {
    let witnessed = consumption::acquire(
        session,
        fixture,
        request,
        &bounded(operation, CONTROL_TIMEOUT)?,
        report,
    )?;
    let known = witnessed.publication();
    drop(witnessed);
    // The final owned publication is already known, and no witness owner spans
    // this full wait. Updates/coalescing during cooldown are controlled-only proof.
    api(cooldown(COOLDOWN, operation, &mut std::thread::sleep))?;
    consumption::acquire_known(
        session,
        fixture,
        request,
        known,
        &bounded(operation, CONTROL_TIMEOUT)?,
        report,
    )
}

fn fingerprint(mapping: &CpuMapping) -> u64 {
    // Private mutation detection only; neither bytes nor fingerprints are reported.
    let mut hash = DefaultHasher::new();
    mapping.bytes().hash(&mut hash);
    hash.finish()
}

struct Retained {
    frame: Frame,
    mapping: CpuMapping,
    original: u64,
}

impl Retained {
    fn verify(&self, operation: &OperationContext, report: &mut Report) -> Check<()> {
        let remapped = consumption::map(&self.frame, operation, report)?;
        require(
            self.frame.stamp() == self.mapping.stamp()
                && self.frame.transform() == self.mapping.transform()
                && remapped.descriptor() == self.mapping.descriptor()
                && remapped.bytes() == self.mapping.bytes()
                && fingerprint(&self.mapping) == self.original,
            "retained-mapping-changed",
        )
    }

    fn bytes(&self) -> u64 {
        self.frame.descriptor().byte_len() as u64
    }
}

fn same_hud(first: &CpuMapping, second: &CpuMapping) -> bool {
    (0..540).all(|y| {
        let a = y * first.descriptor().stride();
        let b = y * second.descriptor().stride();
        first
            .bytes()
            .get(a..a + 960 * 4)
            .zip(second.bytes().get(b..b + 960 * 4))
            .is_some_and(|(first, second)| first == second)
    })
}

fn semantic_live(
    session: &Session,
    fixture: &mut Fixture,
    backend: &OcrBackendDescriptor,
    initial: Observation,
    operation: &OperationContext,
    report: &mut Report,
) -> Check<Retained> {
    let mut cursor = Cursor::default();
    require(initial.marker.state == 0, "blank-source")?;
    consumption::recognize(
        session,
        backend,
        initial,
        &mut cursor,
        Recognition {
            expected_state: 0,
            required_cooldown: COOLDOWN,
        },
        &bounded(operation, CONTROL_TIMEOUT)?,
        report,
    )?;
    let shown = fixture.command("show", operation)?;
    let live = known_after_cooldown(
        session,
        fixture,
        Acquisition {
            after: cursor.last,
            expected: shown,
            exact_counter: true,
            retained_bytes: 0,
        },
        operation,
        report,
    )?;
    consumption::recognize(
        session,
        backend,
        live,
        &mut cursor,
        Recognition {
            expected_state: 1,
            required_cooldown: COOLDOWN,
        },
        &bounded(operation, CONTROL_TIMEOUT)?,
        report,
    )?;
    report.check("completion_cooldown_live_idle", true)?;

    let burst = fixture.command("burst", operation)?;
    let final_counter = burst
        .counter
        .checked_add(8)
        .ok_or(Failure::Rule("fixture-counter-overflow"))?;
    let expected = Ack {
        counter: final_counter,
        ..burst
    };
    let final_frame = known_after_cooldown(
        session,
        fixture,
        Acquisition {
            after: cursor.last,
            expected,
            exact_counter: true,
            retained_bytes: 0,
        },
        operation,
        report,
    )?;
    let mapping = consumption::map(
        &final_frame.frame,
        &bounded(operation, CONTROL_TIMEOUT)?,
        report,
    )?;
    let retained = Retained {
        frame: final_frame.frame.clone(),
        original: fingerprint(&mapping),
        mapping,
    };
    consumption::recognize(
        session,
        backend,
        final_frame,
        &mut cursor,
        Recognition {
            expected_state: 1,
            required_cooldown: COOLDOWN,
        },
        &bounded(operation, CONTROL_TIMEOUT)?,
        report,
    )?;
    report.check("burst_latest", true)?;

    let pulse = cooldown_command(fixture, "pulse", operation)?;
    let repeated = consumption::acquire(
        session,
        fixture,
        Acquisition {
            after: cursor.last,
            expected: pulse,
            exact_counter: true,
            retained_bytes: retained.bytes(),
        },
        &bounded(operation, CONTROL_TIMEOUT)?,
        report,
    )?;
    {
        let pixels = consumption::map(
            &repeated.frame,
            &bounded(operation, CONTROL_TIMEOUT)?,
            report,
        )?;
        report.check(
            "static_observations",
            same_hud(&retained.mapping, &pixels)
                && repeated.frame.stamp().order(&retained.frame.stamp()) == Ok(FrameOrder::After),
        )?;
    }
    consumption::recognize(
        session,
        backend,
        repeated,
        &mut cursor,
        Recognition {
            expected_state: 1,
            required_cooldown: COOLDOWN,
        },
        &bounded(operation, CONTROL_TIMEOUT)?,
        report,
    )?;

    let resized = cooldown_command(fixture, "resize", operation)?;
    let current = consumption::acquire(
        session,
        fixture,
        Acquisition {
            after: cursor.last,
            expected: resized,
            exact_counter: true,
            retained_bytes: retained.bytes(),
        },
        &bounded(operation, CONTROL_TIMEOUT)?,
        report,
    )?;
    require(
        current.frame.descriptor().extent() == PixelExtent::new(1040, 640)
            && current.frame.stamp().epoch() > retained.frame.stamp().epoch()
            && current.frame.stamp().geometry() > retained.frame.stamp().geometry()
            && retained.frame.descriptor().extent() == PixelExtent::new(960, 576),
        "resize-identity",
    )?;
    retained.verify(&bounded(operation, CONTROL_TIMEOUT)?, report)?;
    consumption::recognize(
        session,
        backend,
        current,
        &mut cursor,
        Recognition {
            expected_state: 1,
            required_cooldown: COOLDOWN,
        },
        &bounded(operation, CONTROL_TIMEOUT)?,
        report,
    )?;
    api(cooldown(COOLDOWN, operation, &mut std::thread::sleep))?;
    report.check("resize_retained_mapping", true)?;

    // Native session is live, but these are observed pre-admission interruptions,
    // not an invented in-flight OCR/close race. Exact races remain model-free tests.
    let admitted = report.metrics.ocr_admissions;
    let token = CancellationToken::new();
    let cancelled = operation.clone().with_cancellation(token.clone());
    let wait = cooldown(COOLDOWN, &cancelled, &mut |slice| {
        token.cancel();
        std::thread::sleep(slice);
    });
    require(
        wait.as_ref().err().map(|error| error.status()) == Some(Status::Cancelled),
        "cancellation-cooldown",
    )?;
    let next = consumption::step(
        session,
        backend,
        fixture,
        &mut cursor,
        COOLDOWN,
        &cancelled,
        report,
    );
    report.check(
        "cancellation",
        next == Err(Failure::Api(Status::Cancelled)) && report.metrics.ocr_admissions == admitted,
    )?;

    let deadline = bounded(operation, Duration::from_millis(20))?;
    let wait = cooldown(COOLDOWN, &deadline, &mut std::thread::sleep);
    require(
        wait.as_ref().err().map(|error| error.status()) == Some(Status::DeadlineExceeded),
        "deadline-cooldown",
    )?;
    let next = consumption::step(
        session,
        backend,
        fixture,
        &mut cursor,
        COOLDOWN,
        &deadline,
        report,
    );
    report.check(
        "deadline",
        next == Err(Failure::Api(Status::DeadlineExceeded))
            && report.metrics.ocr_admissions == admitted,
    )?;
    Ok(retained)
}

fn closed(session: &Session, report: &mut Report) -> Check<()> {
    let cleanup = api(OperationContext::new().with_timeout(CLOSE_TIMEOUT))?;
    api(session.close(&cleanup))?;
    let result = session.acquire_frame(&FrameRequest::latest(), &cleanup);
    report.check(
        "session_close",
        result
            .err()
            .is_some_and(|error| error.status() == Status::Closed),
    )
}

fn semantic(
    resources: &mut Resources,
    target: TargetId,
    backend: &OcrBackendDescriptor,
    initial: Observation,
    operation: &OperationContext,
    report: &mut Report,
) -> Check<()> {
    resources.sampler = Some(
        Sampler::start(Some(initial.frame.stamp().stream().get()))
            .map_err(|_| Failure::Rule("sampler-start"))?,
    );
    let started = operation.now();
    let work = (|| {
        let session = resources
            .session
            .as_ref()
            .ok_or(Failure::Rule("session-owner"))?;
        let fixture = resources
            .fixture
            .as_mut()
            .ok_or(Failure::Rule("fixture-owner"))?;
        fixture.command("reset-stats", operation)?;
        let retained = semantic_live(session, fixture, backend, initial, operation, report)?;
        closed(session, report)?;
        retained.verify(&bounded(operation, CONTROL_TIMEOUT)?, report)?;
        resources.close_session()?;
        // A retained frame remains independently mappable after its session owner drops.
        retained.verify(&bounded(operation, CONTROL_TIMEOUT)?, report)?;
        drop(retained);

        let inherited = api(CapturePacingRequest::required(NATIVE_INTERVAL))?;
        let target_frame = open(
            resources,
            target,
            CapturePacingRequest::inherit(),
            inherited,
            &bounded(operation, CONTROL_TIMEOUT)?,
            report,
        )?;
        let mut stamp = target_frame.frame.stamp();
        let mapping = consumption::map(
            &target_frame.frame,
            &bounded(operation, CONTROL_TIMEOUT)?,
            report,
        )?;
        let retained = Retained {
            frame: target_frame.frame,
            original: fingerprint(&mapping),
            mapping,
        };
        let fixture = resources
            .fixture
            .as_mut()
            .ok_or(Failure::Rule("fixture-owner"))?;
        fixture.command("close-window", operation)?;
        let session = resources
            .session
            .as_ref()
            .ok_or(Failure::Rule("session-owner"))?;
        let waiting = bounded(operation, CONTROL_TIMEOUT)?;
        let outcome = loop {
            match session.acquire_frame(&FrameRequest::newer_than(stamp), &waiting) {
                Ok(frame) => {
                    require(
                        frame.stamp().order(&stamp) == Ok(FrameOrder::After),
                        "target-close-order",
                    )?;
                    consumption::layout(&frame, retained.bytes(), report)?;
                    let pixels = consumption::map(&frame, &waiting, report)?;
                    super::control::marker(pixels.descriptor(), pixels.bytes(), fixture.token)?;
                    stamp = frame.stamp();
                }
                Err(error) => break error.status(),
            }
        };
        let label = match outcome {
            Status::TargetLost => "target-lost",
            Status::Closed => "closed",
            #[cfg(target_os = "macos")]
            Status::DeadlineExceeded => "quiescent-deadline",
            _ => return Err(Failure::Rule("target-close-outcome")),
        };
        report.checks.insert("target_close", label);
        retained.verify(&bounded(operation, CONTROL_TIMEOUT)?, report)?;
        closed(session, report)?;
        retained.verify(&bounded(operation, CONTROL_TIMEOUT)?, report)?;
        drop(retained);
        resources.close_session()?;
        resources
            .fixture
            .as_mut()
            .ok_or(Failure::Rule("fixture-owner"))?
            .command("stats", operation)?;
        Ok(())
    })();
    report.metrics.measurement_ns = nanos(operation.now().saturating_duration_since(started))?;
    let sampler = resources.finish_sampler(report);
    work.and(sampler)
}

fn phase(
    resources: &mut Resources,
    backend: &OcrBackendDescriptor,
    interval: Duration,
    duration: Duration,
    operation: &OperationContext,
    report: &mut Report,
) -> Check<()> {
    let started = operation.now();
    let end = started
        .checked_add(duration)
        .ok_or(Failure::Rule("measurement-overflow"))?;
    let mut cursor = Cursor::default();
    let work = (|| {
        while operation.now() < end {
            api(checkpoint(operation))?;
            let session = resources
                .session
                .as_ref()
                .ok_or(Failure::Rule("session-owner"))?;
            let fixture = resources
                .fixture
                .as_mut()
                .ok_or(Failure::Rule("fixture-owner"))?;
            consumption::step(
                session,
                backend,
                fixture,
                &mut cursor,
                interval,
                &bounded(operation, CONTROL_TIMEOUT)?,
                report,
            )?;
        }
        Ok(())
    })();
    // The last admitted recognition, interpretation, and full cooldown are included.
    report.metrics.measurement_ns = nanos(operation.now().saturating_duration_since(started))?;
    work
}

fn comparison(
    resources: &mut Resources,
    case: Case,
    backend: &OcrBackendDescriptor,
    operation: &OperationContext,
    report: &mut Report,
) -> Check<()> {
    resources
        .fixture
        .as_mut()
        .ok_or(Failure::Rule("fixture-owner"))?
        .command("animate", operation)?;
    if case == Case::CaptureOff {
        for key in [
            "owned_source",
            "source_identity",
            "pacing_report",
            "cooldown_spacing",
        ] {
            report.checks.insert(key, "not-applicable");
        }
        api(cooldown(WARMUP, operation, &mut std::thread::sleep))?;
    } else {
        let fixture = resources
            .fixture
            .as_mut()
            .ok_or(Failure::Rule("fixture-owner"))?;
        let expected = fixture.ack;
        let session = resources
            .session
            .as_ref()
            .ok_or(Failure::Rule("session-owner"))?;
        let authenticated = consumption::acquire(
            session,
            fixture,
            Acquisition {
                after: None,
                expected,
                exact_counter: false,
                retained_bytes: 0,
            },
            &bounded(operation, CONTROL_TIMEOUT)?,
            report,
        )?;
        drop(authenticated);
        phase(
            resources,
            backend,
            case.cooldown(),
            WARMUP,
            operation,
            report,
        )?;
    }
    // Only a successfully completed warmup is excluded; failures retain attempted work.
    report.samples.clear();
    report.metrics = Metrics {
        gpu_reason: Some("per-process-gpu-unavailable".to_owned()),
        ..Metrics::default()
    };
    resources
        .fixture
        .as_mut()
        .ok_or(Failure::Rule("fixture-owner"))?
        .command("reset-stats", operation)?;
    resources.sampler = Some(
        Sampler::start(
            resources
                .session
                .as_ref()
                .map(|session| session.stream().get()),
        )
        .map_err(|_| Failure::Rule("sampler-start"))?,
    );
    let work = if case == Case::CaptureOff {
        let started = operation.now();
        let work = api(cooldown(MEASUREMENT, operation, &mut std::thread::sleep));
        report.metrics.measurement_ns = nanos(operation.now().saturating_duration_since(started))?;
        work
    } else {
        phase(
            resources,
            backend,
            case.cooldown(),
            MEASUREMENT,
            operation,
            report,
        )
    };
    let sampled = resources.finish_sampler(report);
    let fixture = resources
        .fixture
        .as_mut()
        .ok_or(Failure::Rule("fixture-owner"))?;
    let paused = fixture.command("pause", operation);
    let stats = fixture.command("stats", operation);
    work.and(sampled)
        .and(paused.map(|_| ()))
        .and(stats.map(|_| ()))?;
    consumption::sample_capacity(report)?;
    if case != Case::CaptureOff {
        require(
            report.metrics.ocr_committed >= 3
                && report.metrics.ocr_committed == report.metrics.ocr_admissions,
            "comparison-observations",
        )?;
        let mut durations = [0_u64; super::contract::SAMPLE_LIMIT];
        for (destination, sample) in durations.iter_mut().zip(&report.samples) {
            *destination = sample.ocr_ns;
        }
        let durations = &mut durations[..report.samples.len()];
        durations.sort_unstable();
        let rank = (durations.len() * 95).div_ceil(100).saturating_sub(1);
        require(durations[rank] <= 5_000_000_000, "ocr-p95-budget")?;
    }
    Ok(())
}

pub(super) fn run(arguments: &Arguments, report: &mut Report) -> Check<()> {
    let operation = api(OperationContext::new().with_timeout(arguments.case.timeout()))?;
    let started = operation.now();
    let setup = bounded(&operation, Duration::from_secs(60))?;
    let mut resources = Resources::default();
    let work = (|| {
        // Permission is decided before spawning a fixture or touching model files.
        permission(&setup, report)?;
        if arguments.case != Case::CaptureOff {
            metrics::begin_capture_diagnostics()
                .map_err(|_| Failure::Rule("capture-diagnostics-start"))?;
            resources.diagnostics = true;
        }
        resources.fixture = Some(Fixture::spawn(arguments, &setup)?);
        resources
            .fixture
            .as_mut()
            .ok_or(Failure::Rule("fixture-owner"))?
            .ready(&setup)?;
        resources.engine = Some(engine(arguments, &setup)?);
        let engine = resources
            .engine
            .as_ref()
            .ok_or(Failure::Rule("engine-owner"))?;
        let backend = engine
            .ocr_backend()
            .ok_or(Failure::Rule("ocr-backend-unavailable"))?;
        let provider = engine
            .ocr_provider()
            .ok_or(Failure::Rule("ocr-provider-unavailable"))?;
        report.check(
            "cpu_model",
            backend.profile().as_str() == mado_pilot::ACCEPTED_G004_PROFILE_ID
                && backend.model().as_str() == mado_pilot::ACCEPTED_G004_MODEL_ID
                && backend.id().as_str() == mado_pilot::DEFAULT_OCR_BACKEND_ID
                && provider.active_provider() == OcrExecutionProvider::Cpu
                && !provider.initialization_fell_back(),
        )?;
        if arguments.case == Case::CaptureOff {
            report.startup_ns = Some(nanos(operation.now().saturating_duration_since(started))?);
            api(checkpoint(&setup))?;
            return comparison(&mut resources, arguments.case, &backend, &operation, report);
        }
        let title = resources
            .fixture
            .as_ref()
            .ok_or(Failure::Rule("fixture-owner"))?
            .title();
        let target = select(&api(engine.discover(&setup))?, &title)?;
        let inherited = arguments.case.pacing()?;
        if arguments.case == Case::Semantic {
            let initial = open(
                &mut resources,
                target,
                CapturePacingRequest::source_default(),
                inherited,
                &setup,
                report,
            )?;
            drop(initial);
            report.check("source_default", true)?;
            resources.close_session()?;
            let preferred = api(CapturePacingRequest::preferred(Duration::from_millis(60)))?;
            let initial = open(&mut resources, target, preferred, inherited, &setup, report)?;
            drop(initial);
            report.check("preferred_override", true)?;
            resources.close_session()?;
        }
        let initial = open(
            &mut resources,
            target,
            CapturePacingRequest::inherit(),
            inherited,
            &setup,
            report,
        )?;
        if arguments.case == Case::Semantic {
            report.check("required_applied", true)?;
        }
        report.startup_ns = Some(nanos(operation.now().saturating_duration_since(started))?);
        api(checkpoint(&setup))?;
        if arguments.case == Case::Semantic {
            semantic(
                &mut resources,
                target,
                &backend,
                initial,
                &operation,
                report,
            )
        } else {
            drop(initial);
            comparison(&mut resources, arguments.case, &backend, &operation, report)
        }
    })();
    if report.startup_ns.is_none() && resources.engine.is_some() {
        report.startup_ns = Some(nanos(operation.now().saturating_duration_since(started))?);
    }
    let cleanup = resources.cleanup(report);
    let result = work.and(cleanup);
    if result.is_ok() {
        let process = report
            .metrics
            .process
            .as_ref()
            .ok_or(Failure::Rule("process-metrics-unavailable"))?;
        let resident = process
            .max_resident_bytes
            .ok_or(Failure::Rule("resident-metric-unavailable"))?;
        let high_water = metrics::process_sample().peak_resident_bytes;
        report.check(
            "resource_budgets",
            resident <= PROCESS_LIMIT
                && high_water.is_some_and(|value| value <= PROCESS_LIMIT)
                && report
                    .startup_ns
                    .is_some_and(|value| value <= 60_000_000_000),
        )?;
        let gap = report.samples.windows(2).any(|rows| {
            rows[0].stream_id == rows[1].stream_id
                && rows[0].epoch == rows[1].epoch
                && rows[1].sequence.saturating_sub(rows[0].sequence) > 1
        });
        report
            .checks
            .insert("sequence_gaps", if gap { "pass" } else { "not-applicable" });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use mado_pilot::{CoordinateSupport, TargetCapability};
    use mado_pilot_runtime::IdentityIssuer;

    #[test]
    fn exact_owned_selection_refuses_missing_ambiguous_and_nonwindow_names() {
        let issuer = IdentityIssuer::new();
        let provider = mado_pilot::ProviderId::new("capture-pacing-test");
        let first = issuer.issue_target(provider).expect("target");
        let second = issuer.issue_target(provider).expect("target");
        let title = "MadoPilot Pacing 0123456789abcdef";
        let describe = |id, kind| {
            TargetDescription::new(
                id,
                title,
                PixelExtent::new(960, 576),
                mado_pilot::PixelFormat::Bgra8,
                CoordinateSupport::frame_only(),
            )
            .with_capability(TargetCapability::capture_only(kind))
        };
        let owned = describe(first, TargetKind::Window);
        assert_eq!(
            select(std::slice::from_ref(&owned), title).expect("exact owner"),
            first
        );
        assert!(select(std::slice::from_ref(&owned), "MadoPilot Pacing").is_err());
        assert!(select(&[describe(second, TargetKind::Display)], title).is_err());
        let duplicate = describe(second, TargetKind::Window);
        assert!(select(&[owned, duplicate], title).is_err());
    }
}
