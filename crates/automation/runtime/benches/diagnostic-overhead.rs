//! Measures capture/mapping, input submission, and close/drain with diagnostics
//! off, normal, debug, and under bounded-queue pressure.
//! Use `cargo test --locked --workspace --all-targets` for the smoke plan, or:
//!
//! ```text
//! cargo bench --locked --package mado-pilot-runtime --bench diagnostic-overhead -- \
//!     --hardware "..." --os-version "..."
//! ```

use std::sync::Arc;
use std::time::Instant;

use mado_pilot_assets::ContentDigest;
use mado_pilot_capture::CaptureProvider;
use mado_pilot_input::InputProvider;
use mado_pilot_runtime::{
    Continuity, DeliveryPlan, DiagnosticDrain, DiagnosticKind, DiagnosticLevel, DiagnosticOptions,
    DiagnosticReader, Engine, EngineOptions, EngineWiring, IdentityIssuer, InputDelivery,
    InputEvent, InputOpenRequest, InputReceipt, InputRequest, InputSequence, Key, Matcher,
    OpenRequest, OperationContext, PackageLoader, PixelExtent, PixelFormat, SequenceOutcome,
    Session, SessionRequest, SubmissionEvidence,
};
use mado_pilot_testkit::bench_harness::{Accounting, Benchmark, Plan, Profile, Sample, measure};
use mado_pilot_testkit::{ControlledCapture, ControlledInput, ControlledMatcher, bench_harness};
use mado_pilot_vision::MatchBackend;

#[global_allocator]
static ALLOCATOR: Accounting = Accounting;

const DIAGNOSTIC_CAPACITY: usize = 64;
const OVERFLOW_CAPACITY: usize = 4;
const OVERFLOW_SUBMISSIONS: usize = 4;
const FIXTURE_DESCRIPTION: &str = "controlled capture/input fixture: 32x24 RGBA8 frame mapping, one Enter event, System route, system-input-admission evidence, explicit session close and diagnostic drain";

#[derive(Debug, Clone, Copy)]
enum ExpectedDiagnostics {
    Off,
    Normal,
    Debug,
    Overflow,
}

impl ExpectedDiagnostics {
    fn options(self) -> DiagnosticOptions {
        match self {
            Self::Off => DiagnosticOptions::off(),
            Self::Normal => DiagnosticOptions::normal(DIAGNOSTIC_CAPACITY).expect("valid capacity"),
            Self::Debug => DiagnosticOptions::debug(DIAGNOSTIC_CAPACITY).expect("valid capacity"),
            Self::Overflow => DiagnosticOptions::debug(OVERFLOW_CAPACITY).expect("valid capacity"),
        }
    }

    const fn submissions(self) -> usize {
        match self {
            Self::Overflow => OVERFLOW_SUBMISSIONS,
            Self::Off | Self::Normal | Self::Debug => 1,
        }
    }
}

#[derive(Debug)]
struct Fixture {
    _engine: Engine,
    session: Session,
    input: Arc<ControlledInput>,
    operation: OperationContext,
    request: InputRequest,
    reader: Option<DiagnosticReader>,
    expected: ExpectedDiagnostics,
}

impl Fixture {
    fn new(expected: ExpectedDiagnostics) -> Self {
        let issuer = Arc::new(IdentityIssuer::new());
        let capture = Arc::new(
            ControlledCapture::new(
                Arc::clone(&issuer),
                PixelExtent::new(32, 24),
                PixelFormat::Rgba8,
            )
            .expect("valid controlled capture"),
        );
        let input = Arc::new(ControlledInput::new(capture.target()));
        let matcher = Arc::new(ControlledMatcher::new(PixelFormat::Rgba8));
        let engine = Engine::new_with_options(
            EngineWiring {
                engine: issuer.engine(),
                capture: Arc::clone(&capture) as Arc<dyn CaptureProvider>,
                matcher: Matcher::new(Arc::clone(&matcher) as Arc<dyn MatchBackend>),
                loader: PackageLoader::new(),
                input: Some(Arc::clone(&input) as Arc<dyn InputProvider>),
                permission: None,
            },
            EngineOptions::new().with_diagnostics(expected.options()),
        )
        .expect("controlled adapters share one provider");
        let reader = engine.take_diagnostic_reader();
        let operation = OperationContext::new();
        let session = engine
            .open_session(
                capture.target(),
                &SessionRequest::new()
                    .capturing(OpenRequest::new())
                    .requesting_input(InputOpenRequest::new()),
                &operation,
            )
            .expect("controlled session opens");
        capture
            .publish(0x30, Continuity::Continuous)
            .expect("controlled source frame publishes");
        let request = InputRequest::new(
            session.target(),
            InputSequence::new(vec![InputEvent::KeyPress(Key::Enter)]).expect("valid sequence"),
            DeliveryPlan::require(InputDelivery::System),
        );

        // Session opening is outside every measured input sample. Drain its
        // records so each retained batch belongs only to that sample.
        if let Some(reader) = &reader {
            while matches!(reader.drain(), DiagnosticDrain::Batch(_)) {}
        }

        Self {
            _engine: engine,
            session,
            input,
            operation,
            request,
            reader,
            expected,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct CloseFixture {
    expected: ExpectedDiagnostics,
}

impl CloseFixture {
    const fn new(expected: ExpectedDiagnostics) -> Self {
        Self { expected }
    }
}

#[derive(Debug)]
struct CloseCase {
    _engine: Engine,
    session: Session,
    operation: OperationContext,
    reader: Option<DiagnosticReader>,
}

impl CloseCase {
    fn new(expected: ExpectedDiagnostics) -> Self {
        let issuer = Arc::new(IdentityIssuer::new());
        let capture = Arc::new(
            ControlledCapture::new(
                Arc::clone(&issuer),
                PixelExtent::new(32, 24),
                PixelFormat::Rgba8,
            )
            .expect("valid controlled capture"),
        );
        let engine = Engine::new_with_options(
            EngineWiring {
                engine: issuer.engine(),
                capture: Arc::clone(&capture) as Arc<dyn CaptureProvider>,
                matcher: Matcher::new(Arc::new(ControlledMatcher::new(PixelFormat::Rgba8))),
                loader: PackageLoader::new(),
                input: None,
                permission: None,
            },
            EngineOptions::new().with_diagnostics(expected.options()),
        )
        .expect("controlled adapters share one provider");
        let reader = engine.take_diagnostic_reader();
        let operation = OperationContext::new();
        let session = engine
            .open_session(
                capture.target(),
                &SessionRequest::new().capturing(OpenRequest::new()),
                &operation,
            )
            .expect("controlled close session opens");
        clear_diagnostics(reader.as_ref());

        Self {
            _engine: engine,
            session,
            operation,
            reader,
        }
    }
}
fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let plan = Plan::from(&arguments);
    let (hardware, os_version) = Profile::host(&arguments);
    let workloads = [
        measure(
            "input_submission_diagnostics_off",
            "the complete receipt is unchanged and no diagnostic reader exists",
            plan,
            || Fixture::new(ExpectedDiagnostics::Off),
            submit_input,
        ),
        measure(
            "input_submission_diagnostics_normal",
            "the complete receipt is unchanged and exactly its one normal record drains without loss",
            plan,
            || Fixture::new(ExpectedDiagnostics::Normal),
            submit_input,
        ),
        measure(
            "input_submission_diagnostics_debug",
            "the complete receipt is unchanged and its debug start, route attempt, and normal terminal record drain without loss",
            plan,
            || Fixture::new(ExpectedDiagnostics::Debug),
            submit_input,
        ),
        measure(
            "input_submission_diagnostic_overflow",
            "four complete receipts remain unchanged while the four-slot queue retains four normal records and reports eight debug losses",
            plan,
            || Fixture::new(ExpectedDiagnostics::Overflow),
            submit_input,
        ),
        measure(
            "capture_mapping_diagnostics_off",
            "the exact retained frame maps to 3072 RGBA8 bytes and no diagnostic reader exists",
            plan,
            || Fixture::new(ExpectedDiagnostics::Off),
            capture_and_map,
        ),
        measure(
            "capture_mapping_diagnostics_normal",
            "the exact retained frame maps unchanged while normal diagnostics retain no debug-only frame or mapping facts",
            plan,
            || Fixture::new(ExpectedDiagnostics::Normal),
            capture_and_map,
        ),
        measure(
            "capture_mapping_diagnostics_debug",
            "the exact retained frame maps unchanged with one frame fact, one mapping fact, and their two starts",
            plan,
            || Fixture::new(ExpectedDiagnostics::Debug),
            capture_and_map,
        ),
        measure(
            "session_close_drain_diagnostics_off",
            "explicit close succeeds with no diagnostic reader or drain allocation",
            plan,
            || CloseFixture::new(ExpectedDiagnostics::Off),
            close_and_drain,
        ),
        measure(
            "session_close_drain_diagnostics_normal",
            "explicit close and its one normal lifecycle record drain without loss",
            plan,
            || CloseFixture::new(ExpectedDiagnostics::Normal),
            close_and_drain,
        ),
        measure(
            "session_close_drain_diagnostics_debug",
            "explicit close and its debug start plus normal lifecycle record drain without loss",
            plan,
            || CloseFixture::new(ExpectedDiagnostics::Debug),
            close_and_drain,
        ),
    ];

    if arguments.iter().any(|argument| argument == "--bench") {
        bench_harness::report(
            &Benchmark {
                id: "phase-2-input-diagnostic-overhead",
                workload: "capture/mapping, input submission, and close/drain with diagnostics off, enabled, and under bounded pressure",
                phase: "2.2",
            },
            &Profile {
                fixture: "source-defined controlled capture/input fixture in crates/automation/runtime/benches/diagnostic-overhead.rs".to_owned(),
                fixture_sha256: ContentDigest::of(FIXTURE_DESCRIPTION.as_bytes()).to_string(),
                benchmark_executable_sha256: None,
                hardware,
                os_version,
                deployment_target: None,
                build_profile: format!(
                    "cargo bench, default features, debug_assertions={}",
                    cfg!(debug_assertions)
                ),
                correctness_oracle: "every retained sample checks exact frame/mapping or receipt outcomes, exact diagnostic categories, queue losses, and increasing record sequences",
                queue_policy: "diagnostics Off has no queue; Normal and Debug use capacity 64; overflow uses capacity 4; producer emission is non-blocking and every sample drains",
                notes: Some("input spans time submission/emission; capture spans acquisition/mapping/emission; close spans explicit close plus one diagnostic drain. Oracle checks follow each span.".to_owned()),
            },
            plan,
            &workloads,
        );
    } else {
        bench_harness::summarize("diagnostic-overhead", plan, &workloads);
    }

    bench_harness::enforce_hard_budgets(&workloads);
}

fn submit_input(fixture: &Fixture) -> Sample {
    let submissions = fixture.expected.submissions();
    let mut receipts: [Option<InputReceipt>; OVERFLOW_SUBMISSIONS] = std::array::from_fn(|_| None);
    let started = Instant::now();
    for receipt in receipts.iter_mut().take(submissions) {
        *receipt = Some(
            fixture
                .session
                .send_input(&fixture.request, &fixture.operation)
                .expect("controlled input completes"),
        );
    }
    let elapsed = started.elapsed();

    fixture.input.clear_observations();
    let receipts_correct = receipts
        .iter()
        .take(submissions)
        .all(|receipt| receipt.as_ref().is_some_and(receipt_is_complete));
    let diagnostics_correct = diagnostics_are_exact(fixture, submissions);
    Sample::unmapped(elapsed, receipts_correct && diagnostics_correct)
}

fn capture_and_map(fixture: &Fixture) -> Sample {
    let started = Instant::now();
    let frame = fixture
        .session
        .acquire_frame(
            &mado_pilot_runtime::FrameRequest::latest(),
            &fixture.operation,
        )
        .expect("controlled frame is available");
    let mapping = fixture
        .session
        .map_frame(&frame, PixelFormat::Rgba8, &fixture.operation)
        .expect("controlled frame maps");
    let elapsed = started.elapsed();

    let mapped = u64::try_from(mapping.bytes().len()).expect("fixture byte length fits u64");
    let mapping_correct = mapped == 32 * 24 * 4
        && mapping.stamp() == frame.stamp()
        && mapping.descriptor().format() == PixelFormat::Rgba8;
    Sample::new(
        elapsed,
        mapping_correct && capture_diagnostics_are_exact(fixture),
        mapped,
    )
}

fn capture_diagnostics_are_exact(fixture: &Fixture) -> bool {
    match fixture.expected {
        ExpectedDiagnostics::Off => fixture.reader.is_none(),
        ExpectedDiagnostics::Normal => fixture
            .reader
            .as_ref()
            .is_some_and(|reader| matches!(reader.drain(), DiagnosticDrain::OpenEmpty)),
        ExpectedDiagnostics::Debug => drain_matches(
            fixture.reader.as_ref(),
            ExpectedDrain {
                records: 4,
                normal_losses: 0,
                debug_losses: 0,
                input_records: 0,
                attempt_records: 0,
                started_records: 2,
                frame_records: 1,
                mapping_records: 1,
            },
        ),
        ExpectedDiagnostics::Overflow => false,
    }
}
fn close_and_drain(fixture: &CloseFixture) -> Sample {
    let case = CloseCase::new(fixture.expected);
    let started = Instant::now();
    let closed = case.session.close(&case.operation).is_ok();
    let drained = case.reader.as_ref().map(DiagnosticReader::drain);
    let elapsed = started.elapsed();

    let diagnostics_correct = close_drain_is_exact(fixture.expected, drained.as_ref())
        && case
            .reader
            .as_ref()
            .is_none_or(|reader| matches!(reader.drain(), DiagnosticDrain::OpenEmpty));
    Sample::unmapped(elapsed, closed && diagnostics_correct)
}

fn clear_diagnostics(reader: Option<&DiagnosticReader>) {
    if let Some(reader) = reader {
        while matches!(reader.drain(), DiagnosticDrain::Batch(_)) {}
    }
}

fn close_drain_is_exact(expected: ExpectedDiagnostics, drained: Option<&DiagnosticDrain>) -> bool {
    match (expected, drained) {
        (ExpectedDiagnostics::Off, None) => true,
        (ExpectedDiagnostics::Normal, Some(DiagnosticDrain::Batch(batch))) => {
            batch.losses().is_empty()
                && batch.records().len() == 1
                && batch.records()[0].kind() == DiagnosticKind::Lifecycle
                && batch.records()[0].level() == DiagnosticLevel::Normal
        }
        (ExpectedDiagnostics::Debug, Some(DiagnosticDrain::Batch(batch))) => {
            batch.losses().is_empty()
                && batch.records().len() == 2
                && batch
                    .records()
                    .iter()
                    .filter(|record| record.kind() == DiagnosticKind::Lifecycle)
                    .count()
                    == 1
                && batch
                    .records()
                    .iter()
                    .filter(|record| record.kind() == DiagnosticKind::OperationStarted)
                    .count()
                    == 1
        }
        _ => false,
    }
}

fn receipt_is_complete(receipt: &InputReceipt) -> bool {
    receipt.outcome() == SequenceOutcome::Complete
        && receipt.selected_route() == Some(InputDelivery::System)
        && receipt.submitted() == 1
        && receipt.evidence() == Some(SubmissionEvidence::SystemInputAdmission)
        && receipt.fault().is_none()
}

fn diagnostics_are_exact(fixture: &Fixture, submissions: usize) -> bool {
    match fixture.expected {
        ExpectedDiagnostics::Off => fixture.reader.is_none(),
        ExpectedDiagnostics::Normal => drain_matches(
            fixture.reader.as_ref(),
            ExpectedDrain {
                records: submissions,
                normal_losses: 0,
                debug_losses: 0,
                input_records: submissions,
                attempt_records: 0,
                started_records: 0,
                frame_records: 0,
                mapping_records: 0,
            },
        ),
        ExpectedDiagnostics::Debug => drain_matches(
            fixture.reader.as_ref(),
            ExpectedDrain {
                records: submissions * 3,
                normal_losses: 0,
                debug_losses: 0,
                input_records: submissions,
                attempt_records: submissions,
                started_records: submissions,
                frame_records: 0,
                mapping_records: 0,
            },
        ),
        ExpectedDiagnostics::Overflow => drain_matches(
            fixture.reader.as_ref(),
            ExpectedDrain {
                records: OVERFLOW_CAPACITY,
                normal_losses: 0,
                debug_losses: (submissions * 2) as u64,
                input_records: OVERFLOW_CAPACITY,
                attempt_records: 0,
                started_records: 0,
                frame_records: 0,
                mapping_records: 0,
            },
        ),
    }
}

struct ExpectedDrain {
    records: usize,
    normal_losses: u64,
    debug_losses: u64,
    input_records: usize,
    attempt_records: usize,
    started_records: usize,
    frame_records: usize,
    mapping_records: usize,
}

fn drain_matches(reader: Option<&DiagnosticReader>, expected: ExpectedDrain) -> bool {
    let Some(reader) = reader else {
        return false;
    };
    let DiagnosticDrain::Batch(batch) = reader.drain() else {
        return false;
    };
    let retained = batch.records();
    let sequences_increase = retained
        .windows(2)
        .all(|pair| pair[0].sequence() < pair[1].sequence());
    let input_count = retained
        .iter()
        .filter(|record| record.kind() == DiagnosticKind::Input)
        .count();
    let attempt_count = retained
        .iter()
        .filter(|record| record.kind() == DiagnosticKind::RouteAttempt)
        .count();
    let debug_count = retained
        .iter()
        .filter(|record| {
            record.kind() == DiagnosticKind::OperationStarted
                && record.level() == DiagnosticLevel::Debug
        })
        .count();
    let frame_count = retained
        .iter()
        .filter(|record| record.kind() == DiagnosticKind::Frame)
        .count();
    let mapping_count = retained
        .iter()
        .filter(|record| record.kind() == DiagnosticKind::Mapping)
        .count();
    let expected_debug = expected.started_records;

    retained.len() == expected.records
        && batch.losses().normal() == expected.normal_losses
        && batch.losses().debug() == expected.debug_losses
        && input_count == expected.input_records
        && attempt_count == expected.attempt_records
        && debug_count == expected_debug
        && frame_count == expected.frame_records
        && mapping_count == expected.mapping_records
        && sequences_increase
        && matches!(reader.drain(), DiagnosticDrain::OpenEmpty)
}
