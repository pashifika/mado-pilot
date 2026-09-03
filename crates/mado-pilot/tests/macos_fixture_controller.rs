#![cfg_attr(not(target_os = "macos"), allow(missing_docs))]
#![cfg(target_os = "macos")]
//! Deterministic regressions for the benchmark-only macOS fixture controller.

use mado_pilot_platform_macos::fixture_control as macos_fixture_control;
use mado_pilot_platform_macos::fixture_protocol as macos_fixture_protocol;

#[allow(dead_code, unreachable_pub, unused_imports)]
#[path = "../benches/support/macos_fixture.rs"]
mod macos_fixture;

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use macos_fixture::{FixtureController, LaunchMode};
use mado_pilot::{
    Engine, FrameRequest, OpenRequest, OperationContext, PixelFormat, SessionRequest, TargetId,
};
use mado_pilot_backend_opencv::OpenCvBackend;
use mado_pilot_platform_macos::{
    MacosCaptureProvider, MacosPermissionProbe,
    fixture_control::{AuthenticatedFixtureProcess, executable_identity},
};
use mado_pilot_runtime::{
    CaptureProvider, EngineOptions, EngineWiring, IdentityIssuer, InputProvider, Matcher,
    PackageLoader, PermissionProbe,
};

const OPERATION_WAIT: Duration = Duration::from_secs(5);
const FIXTURE_WAIT: Duration = Duration::from_secs(10);

fn bounded(wait: Duration) -> OperationContext {
    OperationContext::new()
        .with_timeout(wait)
        .expect("the test timeout is positive")
}

fn focused_engine() -> (Engine, Arc<MacosCaptureProvider>) {
    let backend = OpenCvBackend::new().expect("the required OpenCV backend initializes");
    let issuer = Arc::new(IdentityIssuer::new());
    let engine_id = issuer.engine();
    let provider = Arc::new(MacosCaptureProvider::new(issuer));
    let engine = Engine::new_with_options(
        EngineWiring {
            engine: engine_id,
            capture: Arc::clone(&provider) as Arc<dyn CaptureProvider>,
            matcher: Matcher::new(Arc::new(backend)),
            loader: PackageLoader::new(),
            ocr: None,
            input: Some(Arc::clone(&provider) as Arc<dyn InputProvider>),
            permission: Some(Arc::new(MacosPermissionProbe::new()) as Arc<dyn PermissionProbe>),
        },
        EngineOptions::new(),
    )
    .expect("the focused macOS engine builds");
    (engine, provider)
}

fn select_fixture_target(
    engine: &Engine,
    provider: &MacosCaptureProvider,
    process: AuthenticatedFixtureProcess,
) -> TargetId {
    let selection_deadline = Instant::now() + FIXTURE_WAIT;
    loop {
        let targets = engine
            .discover(&bounded(OPERATION_WAIT))
            .expect("the focused fixture is discoverable");
        let selected = macos_fixture_protocol::select_unique_fixture(
            &targets,
            process.process_id(),
            |target| {
                provider.fixture_target_has_authenticated_owner(target, |owner| {
                    process.matches_live_owner(owner)
                })
            },
        );
        if let Ok(target) = selected {
            return target.id();
        }
        assert!(
            Instant::now() < selection_deadline,
            "exactly one authenticated focused fixture becomes selectable"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn exercise_capture_session(
    engine: &Engine,
    target: TargetId,
    engine_iteration: usize,
    session_iteration: usize,
    repetitions: usize,
) {
    let session = engine
        .open_session(
            target,
            &SessionRequest::new().capturing(OpenRequest::new()),
            &bounded(OPERATION_WAIT),
        )
        .unwrap_or_else(|error| {
            panic!(
                "engine {engine_iteration} fresh session {session_iteration}/{repetitions} opens: \
                 status={:?}",
                error.status()
            )
        });
    let frame = session
        .acquire_frame(&FrameRequest::latest(), &bounded(OPERATION_WAIT))
        .unwrap_or_else(|error| {
            panic!(
                "engine {engine_iteration} fresh session {session_iteration}/{repetitions} \
                 publishes: status={:?}",
                error.status()
            )
        });
    let mapping = frame
        .map(PixelFormat::Bgra8, &bounded(OPERATION_WAIT))
        .unwrap_or_else(|error| {
            panic!(
                "engine {engine_iteration} fresh session {session_iteration}/{repetitions} maps: \
                 status={:?}",
                error.status()
            )
        });
    let descriptor = mapping.descriptor();
    assert!(
        macos_fixture_protocol::frame_is_fixture_content(
            mapping.bytes(),
            descriptor.stride(),
            descriptor.extent(),
        ),
        "engine {engine_iteration} fresh session {session_iteration}/{repetitions} maps the fixture"
    );
    session
        .close(&bounded(OPERATION_WAIT))
        .unwrap_or_else(|error| {
            panic!(
                "engine {engine_iteration} fresh session {session_iteration}/{repetitions} closes: \
                 status={:?}",
                error.status()
            )
        });
    session
        .close(&bounded(OPERATION_WAIT))
        .unwrap_or_else(|error| {
            panic!(
                "engine {engine_iteration} fresh session {session_iteration}/{repetitions} closes \
                 idempotently: status={:?}",
                error.status()
            )
        });
    assert!(
        macos_fixture_protocol::frame_is_fixture_content(
            mapping.bytes(),
            descriptor.stride(),
            descriptor.extent(),
        ),
        "engine {engine_iteration} retained mapping {session_iteration}/{repetitions} survives close"
    );
}

/// Exercises only the owned private control channel; acknowledgements prove fixture
/// effects and never stand in for capture or matching observations.
#[test]
#[ignore = "opens a real signed fixture application on an interactive desktop"]
fn watcher_visual_controls_acknowledge_and_cleanup_without_capture() {
    let executable = PathBuf::from(
        std::env::var_os("MADO_PILOT_MACOS_FIXTURE_EXECUTABLE")
            .expect("set the exact signed fixture executable"),
    );
    let executable_bytes: Arc<[u8]> = std::fs::read(&executable)
        .expect("the signed fixture executable is readable")
        .into();
    let identity =
        executable_identity(&executable).expect("the signed fixture has a valid code identity");
    let mut fixture = FixtureController::start(
        &executable,
        executable_bytes,
        identity,
        LaunchMode::ControlledStatic,
        FIXTURE_WAIT,
    )
    .expect("authenticated watcher fixture starts");

    for kind in [
        macos_fixture_protocol::FixtureCommandKind::SetVisualVisible,
        macos_fixture_protocol::FixtureCommandKind::SetVisualAbsent,
    ] {
        let result = fixture
            .command(kind, OPERATION_WAIT)
            .expect("visual command receives its correlated acknowledgement")
            .result();
        assert_eq!(result.status, 0);
        assert_ne!(result.before_window, 0);
        assert_eq!(result.after_window, result.before_window);
    }

    assert!(
        fixture.finish(FIXTURE_WAIT).is_accepted(),
        "the watcher fixture closes its control channel, process, and output reader"
    );
}

/// Drives the exact fixture controller and facade wiring used by the native
/// benchmark without allocation accounting or profile generation.
#[test]
#[ignore = "repeatedly opens real ScreenCaptureKit sessions on an interactive desktop"]
fn repeated_fresh_capture_sessions_survive_fixture_and_engine_turnover() {
    const REPETITIONS: usize = 64;

    let executable = PathBuf::from(
        std::env::var_os("MADO_PILOT_MACOS_FIXTURE_EXECUTABLE")
            .expect("set the exact signed fixture executable"),
    );
    let executable_bytes: Arc<[u8]> = std::fs::read(&executable)
        .expect("the signed fixture executable is readable")
        .into();
    let identity =
        executable_identity(&executable).expect("the signed fixture has a valid code identity");
    const FIXTURE_GENERATIONS: usize = 6;
    for fixture_iteration in 0..FIXTURE_GENERATIONS {
        let mut fixture = FixtureController::start(
            &executable,
            Arc::clone(&executable_bytes),
            identity,
            LaunchMode::ControlledStatic,
            FIXTURE_WAIT,
        )
        .unwrap_or_else(|error| {
            panic!(
                "authenticated benchmark fixture {fixture_iteration}/{FIXTURE_GENERATIONS} starts: \
                 {error}"
            )
        });
        let process = fixture
            .authenticated_process()
            .expect("the benchmark fixture remains authenticated");
        let engine_count = if fixture_iteration + 1 == FIXTURE_GENERATIONS {
            3
        } else {
            1
        };

        for engine_iteration in 0..engine_count {
            let (engine, provider) = focused_engine();
            let target = select_fixture_target(&engine, &provider, process);
            let observed_engine = fixture_iteration * 3 + engine_iteration;

            // `Flow::from_fixture` performs this confirmation lifecycle before
            // a benchmark workload starts sampling.
            exercise_capture_session(&engine, target, observed_engine, 0, REPETITIONS + 1);
            if engine_count == 3 && engine_iteration == 1 {
                let yielded = fixture
                    .command(
                        macos_fixture_protocol::FixtureCommandKind::YieldForeground,
                        OPERATION_WAIT,
                    )
                    .expect("the transition fixture yields foreground");
                assert_eq!(yielded.result().status, 0);
            }
            let repetitions = if engine_count == 1 || engine_iteration == 1 {
                1
            } else {
                REPETITIONS
            };
            for session_iteration in 1..=repetitions {
                exercise_capture_session(
                    &engine,
                    target,
                    observed_engine,
                    session_iteration,
                    repetitions + 1,
                );
            }
        }

        assert!(
            fixture.finish(FIXTURE_WAIT).is_accepted(),
            "the authenticated fixture stops cleanly"
        );
    }
}
