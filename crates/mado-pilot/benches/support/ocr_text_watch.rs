//! Fixed controlled workload scripts; timing includes the driver and its gates.

use std::cell::Cell;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mado_pilot::{
    CancellationToken, ChangeDetectionPolicy, ClipPolicy, Continuity, CoordinateSpace,
    DiagnosticDrain, DiagnosticLevel, DiagnosticOptions, DiagnosticReader, Engine, EngineOptions,
    Error, Frame, FrameRequest, FrameStamp, MatchOptions, OcrBackend, OcrBackendDescriptor,
    OcrBackendId, OcrBackendIdentity, OcrBackendRequest, OcrBackendVersion, OcrCandidateSink,
    OcrExecutionProvider, OcrExecutionProviderPolicy, OcrModelIdentity, OcrProviderDescriptor,
    OcrRegion, OcrRequest, OcrTextAnalysisRate, OcrTextOverload, OcrTextQuery, OcrTextQueryOutcome,
    OcrTextQueryProgress, OcrTextStability, OcrTextTerminalOutcome, OcrTextWatchRequest,
    OcrTextWatchResult, OcrTextWorkDisposition as OcrWork, OpenRequest, OperationContext,
    PixelExtent, PixelFormat, PixelRect, ProviderProfileId, Rect, Session, Status,
    TemplateOverload, TemplateQuery, TemplateQueryOutcome, TemplateTerminalOutcome,
    TemplateWatchRequest, TemplateWorkDisposition as TemplateWork,
};
use mado_pilot_runtime::{
    CaptureProvider, EngineWiring, IdentityIssuer, Matcher, OcrRecognizer, PackageLoader,
};
use mado_pilot_testkit::bench_harness::{Plan, QueryWorkMetrics, Sample, Workload, measure};
use mado_pilot_testkit::{
    CompletionGate, ControlledCapture, ControlledMatcher, ControlledOcr, ControlledProducer,
    ManualClock, MatchBackend, ScriptedMatchCall, ScriptedOcrCall, ScriptedOcrCandidate,
    match_fixtures,
};

#[path = "ocr_text_watch_cases.rs"]
mod cases;
#[path = "ocr_text_watch_resources.rs"]
mod resources;

const EXTENT: PixelExtent = PixelExtent::new(960, 540);
const FORMAT: PixelFormat = PixelFormat::Bgra8;
const SOURCE_BYTES: u64 = 960 * 540 * 4;
const INTERVAL: Duration = Duration::from_millis(50);
const CAPTURE_TICK: Duration = Duration::from_millis(16);
const WAIT: Duration = Duration::from_secs(15);
const TEXT: &str = "café READY";
const LITERAL: &str = "é";
const CONFIDENCE: f64 = 0.8;

pub(super) fn workloads(plan: Plan) -> Vec<Workload> {
    let definitions = [
        (
            "steady-nonmatch",
            "21 acknowledged sources never confirm; polling never infers",
            cases::steady_nonmatch as fn(&mut Evidence),
        ),
        (
            "positive-consecutive",
            "blank checkpoint then exact third positive source, including unchanged confirmations",
            cases::positive_consecutive,
        ),
        (
            "slow-backend-saturation",
            "64 queries expire, including the held query's replacement obligation",
            cases::saturation,
        ),
        (
            "mixed-two-session",
            "two OCR/two template queries preserve class/session turns and held-mapping expiry",
            cases::mixed,
        ),
        (
            "retained-results",
            "four results/four frame owners survive 20 new publications and parent teardown",
            cases::retained,
        ),
        (
            "cancellation-close",
            "wait, query, close and owner drop precede held mapping/inference retirement",
            cases::cancellation_close,
        ),
        (
            "cold-startup",
            "one controlled engine opens, confirms and closes; not native model startup",
            cases::startup,
        ),
    ];
    definitions.into_iter().map(|(name, oracle, operation)| {
        let selected = if name == "cold-startup" { Plan::new(0, 1) } else { plan };
        let workload = measure(name, oracle, selected, || Case {
            name, operation, iteration: Cell::new(0), warmups: selected.warmup(),
        }, run_sample);
        println!("# ocr-aggregate workload={name} warmups={} samples={} driver_p95_ms={} driver_max_ms={} incorrect={} allocated_peak_bytes={} allocated_growth_bytes={}",
            selected.warmup(), selected.samples(), workload.percentile(0.95),
            workload.max_elapsed().as_secs_f64() * 1000.0, workload.incorrect(),
            workload.peak_allocated_bytes(), workload.growth_bytes());
        workload
    }).collect()
}

struct Case {
    name: &'static str,
    operation: fn(&mut Evidence),
    iteration: Cell<usize>,
    warmups: usize,
}

fn run_sample(case: &Case) -> Sample {
    let iteration = case.iteration.get();
    case.iteration.set(iteration + 1);
    let phase = if iteration < case.warmups {
        "warmup"
    } else {
        "sample"
    };
    println!(
        "# ocr-start workload={} phase={phase} iteration={iteration}",
        case.name
    );
    let mut evidence = Evidence::new(case.name);
    (case.operation)(&mut evidence);
    let elapsed = evidence.started.elapsed();
    println!(
        "# ocr-raw workload={} phase={phase} iteration={iteration} elapsed_ns={} semantic=passed work={:?} backend_admitted_input_mapped_bytes={} backend_input_max_bytes={} retained_read_mapped_bytes={} diagnostic_records={} diagnostic_lost_normal={} diagnostic_lost_debug={} resource_measurement={}",
        case.name,
        elapsed.as_nanos(),
        evidence.work,
        evidence.backend_input_bytes,
        evidence.backend_max_bytes,
        evidence.retained_read_bytes,
        evidence.records,
        evidence.lost_normal,
        evidence.lost_debug,
        if evidence.resident_complete {
            "available-not-qualified"
        } else {
            "missing-nonpass"
        }
    );
    let mut sample =
        Sample::new(elapsed, true, evidence.backend_max_bytes).with_query_work(evidence.work);
    if let Some(peak) = evidence.resident_peak {
        sample = sample.with_peak_resident_bytes(peak);
    }
    sample
}

struct Evidence {
    name: &'static str,
    started: Instant,
    work: QueryWorkMetrics,
    backend_input_bytes: u64,
    backend_max_bytes: u64,
    retained_read_bytes: u64,
    records: u64,
    lost_normal: u64,
    lost_debug: u64,
    last_record: u64,
    resident_peak: Option<u64>,
    resident_complete: bool,
}

impl Evidence {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            started: Instant::now(),
            work: QueryWorkMetrics::default(),
            backend_input_bytes: 0,
            backend_max_bytes: 0,
            retained_read_bytes: 0,
            records: 0,
            lost_normal: 0,
            lost_debug: 0,
            last_record: 0,
            resident_peak: None,
            resident_complete: true,
        }
    }

    fn checkpoint(&mut self, label: &str, rig: &Rig, frame_clones: u64) {
        let observed = rig.engine.ocr_text_observation();
        assert!(
            observed.physical_ocr_in_flight <= 1 && observed.physical_ocr_high_water <= 1,
            "physical OCR capacity exceeded"
        );
        assert!(
            observed.query_count <= 256 && observed.active_sessions <= 16,
            "shared scheduler capacity exceeded"
        );
        assert!(
            observed.mapped_cache_bytes <= 64 * 1024 * 1024,
            "logical mapping cache capacity exceeded"
        );
        let resident = self.resident();
        println!(
            "# ocr-checkpoint workload={} stage={label} elapsed_ns={} scheduler={observed:?} separate_frame_extent_bytes={frame_clones} resident_current_bytes={:?} resident_process_peak_bytes={:?} private_bytes={:?} physical_footprint_bytes={:?}",
            self.name,
            self.started.elapsed().as_nanos(),
            resident.current,
            resident.peak,
            resident.private,
            resident.footprint
        );
        self.drain(&rig.reader);
    }

    fn resident(&mut self) -> resources::Resident {
        let resident = resources::resident();
        self.resident_complete &= resident.current.is_some() && resident.peak.is_some();
        if let Some(peak) = resident.peak {
            self.resident_peak = Some(self.resident_peak.map_or(peak, |old| old.max(peak)));
        }
        resident
    }

    fn endpoint(&self, label: &str, started: Instant) {
        println!(
            "# ocr-endpoint workload={} stage={label} duration_ns={}",
            self.name,
            started.elapsed().as_nanos()
        );
    }

    fn source(&self, label: &str, stamp: FrameStamp) {
        println!(
            "# ocr-source workload={} stage={label} frame={stamp:?}",
            self.name
        );
    }

    fn drain(&mut self, reader: &DiagnosticReader) -> bool {
        match reader.drain() {
            DiagnosticDrain::Batch(batch) => {
                self.lost_normal += batch.losses().normal();
                self.lost_debug += batch.losses().debug();
                for record in batch.records() {
                    let sequence = record.sequence().get();
                    assert!(
                        sequence > self.last_record,
                        "diagnostic sequence did not advance"
                    );
                    self.last_record = sequence;
                    self.records += 1;
                }
                false
            }
            DiagnosticDrain::EndOfStream => true,
            DiagnosticDrain::OpenEmpty => false,
            _ => panic!("unrecognized diagnostic drain outcome"),
        }
    }

    fn collect(&mut self, rig: &Rig, queries: &[&OcrTextQuery], publications: u64) {
        let input = rig.ocr.input.lock().expect("observed OCR input lock");
        self.backend_input_bytes += input.total_bytes;
        self.backend_max_bytes = self.backend_max_bytes.max(input.max_bytes);
        self.work.backend_runs += rig.ocr.inner.recognition_count() as u64;
        self.work.producer_publications += publications;
        for query in queries {
            let progress = query.progress();
            assert!(
                progress.pending_count() <= 1 && progress.physical_in_flight_count() <= 1,
                "query work exceeded its finite shape"
            );
            let work = progress.work();
            assert_eq!(work.get(OcrWork::Coalesced), 0, "OCR work coalesced");
            self.work.admitted += work.get(OcrWork::Admitted);
            self.work.skipped_change += work.get(OcrWork::SkippedChange);
            self.work.deferred_rate += work.get(OcrWork::DeferredRate);
            self.work.superseded += work.get(OcrWork::Superseded);
            self.work.rejected += work.get(OcrWork::Rejected);
            self.work.queue_expired += work.get(OcrWork::QueueExpired);
            self.work.completed += work.get(OcrWork::Completed);
            self.work.failed += work.get(OcrWork::Failed);
            assert!(
                matches!(query.poll(), OcrTextQueryOutcome::Terminal(_)),
                "query did not terminate"
            );
            self.work.query_completions += 1;
        }
        self.drain(&rig.reader);
    }
}

// A narrow observer of actual backend-input mappings. It does not count change
// skips as inference or pretend that input-byte sums are all physical mappings.
// The try-lock models the one-slot backend for the controlled Busy oracle only.
#[derive(Debug)]
struct ObservedOcr {
    inner: ControlledOcr,
    slot: Mutex<()>,
    input: Mutex<InputObservations>,
}

#[derive(Debug, Default)]
struct InputObservations {
    total_bytes: u64,
    max_bytes: u64,
    sources: Vec<FrameStamp>,
    busy: u64,
}

impl OcrBackend for ObservedOcr {
    fn descriptor(&self) -> OcrBackendDescriptor {
        self.inner.descriptor()
    }
    fn provider_descriptor(&self) -> Option<OcrProviderDescriptor> {
        self.inner.provider_descriptor()
    }
    fn recognize(
        &self,
        request: &OcrBackendRequest<'_>,
        output: &mut dyn OcrCandidateSink,
        operation: &OperationContext,
    ) -> mado_pilot::Result<()> {
        let Ok(_slot) = self.slot.try_lock() else {
            self.input.lock().expect("observed OCR input lock").busy += 1;
            return Err(Error::new(
                Status::LimitExceeded,
                "controlled OCR inference slot is occupied",
            ));
        };
        let mapping = request.pixels();
        let bytes = u64::try_from(mapping.descriptor().byte_len()).expect("bounded mapping bytes");
        assert_eq!(bytes, SOURCE_BYTES, "backend input extent differs");
        assert_eq!(
            mapping.descriptor().format(),
            FORMAT,
            "backend input format differs"
        );
        assert_eq!(
            mapping.bytes().len() as u64,
            bytes,
            "backend mapping descriptor differs"
        );
        {
            let mut input = self.input.lock().expect("observed OCR input lock");
            assert!(
                input.sources.len() < 256,
                "controlled script exceeded its input bound"
            );
            input.total_bytes += bytes;
            input.max_bytes = input.max_bytes.max(bytes);
            input.sources.push(mapping.stamp());
        }
        self.inner.recognize(request, output, operation)
    }
    fn close(&self, operation: &OperationContext) -> mado_pilot::Result<()> {
        self.inner.close(operation)
    }
}

struct Rig {
    engine: Engine,
    capture: Arc<ControlledCapture>,
    ocr: Arc<ObservedOcr>,
    matcher: Arc<ControlledMatcher>,
    clock: Arc<ManualClock>,
    reader: DiagnosticReader,
}

impl Rig {
    fn new(ocr: ControlledOcr) -> Self {
        Self::with_matcher(ocr, ControlledMatcher::new(FORMAT))
    }

    fn with_matcher(ocr: ControlledOcr, matcher: ControlledMatcher) -> Self {
        let issuer = Arc::new(IdentityIssuer::new());
        let capture = Arc::new(
            ControlledCapture::new(Arc::clone(&issuer), EXTENT, FORMAT)
                .expect("controlled capture"),
        );
        let ocr = Arc::new(ObservedOcr {
            inner: ocr,
            slot: Mutex::new(()),
            input: Mutex::new(InputObservations::default()),
        });
        let matcher = Arc::new(matcher);
        let engine = Engine::new_with_options(
            EngineWiring {
                engine: issuer.engine(),
                capture: Arc::clone(&capture) as Arc<dyn CaptureProvider>,
                matcher: Matcher::new(Arc::clone(&matcher) as Arc<dyn MatchBackend>),
                loader: PackageLoader::new(),
                ocr: Some(OcrRecognizer::new(Arc::clone(&ocr) as Arc<dyn OcrBackend>)),
                input: None,
                permission: None,
            },
            EngineOptions::new().with_diagnostics(
                DiagnosticOptions::new(DiagnosticLevel::Debug, 256).expect("bounded diagnostics"),
            ),
        )
        .expect("controlled engine");
        let reader = engine
            .take_diagnostic_reader()
            .expect("enabled diagnostics");
        Self {
            engine,
            capture,
            ocr,
            matcher,
            clock: Arc::new(ManualClock::new()),
            reader,
        }
    }

    fn open(&self) -> Session {
        self.engine
            .open(self.capture.target(), &OpenRequest::new(), &bounded())
            .expect("controlled session")
    }

    fn query(
        &self,
        session: &Session,
        confirmations: u32,
        policy: ChangeDetectionPolicy,
    ) -> OcrTextQuery {
        let stability = if confirmations == 1 {
            OcrTextStability::immediate()
        } else {
            OcrTextStability::consecutive(confirmations).expect("positive confirmations")
        };
        session
            .start_ocr_text_watch(
                OcrTextWatchRequest::new(
                    Rect::new(CoordinateSpace::CapturePixels, 0.0, 0.0, 960.0, 540.0)
                        .expect("fixed ROI"),
                    ClipPolicy::Reject,
                    " e\u{301} ",
                    CONFIDENCE,
                    CoordinateSpace::CapturePixels,
                    OcrTextAnalysisRate::from_minimum_interval(INTERVAL).expect("positive rate"),
                    stability,
                    policy,
                    OperationContext::new()
                        .with_clock(Arc::clone(&self.clock) as Arc<dyn mado_pilot::Clock>),
                )
                .expect("controlled request"),
            )
            .expect("controlled query")
    }

    fn template(&self, session: &Session, id: &str) -> TemplateQuery {
        let template = self
            .engine
            .prepare_template(&match_fixtures::planted_template(id), &bounded())
            .expect("controlled template");
        let options = MatchOptions::from_defaults(template.defaults());
        session
            .start_template_watch(
                TemplateWatchRequest::new(
                    template,
                    options,
                    OperationContext::new()
                        .with_clock(Arc::clone(&self.clock) as Arc<dyn mado_pilot::Clock>),
                )
                .with_change_policy(ChangeDetectionPolicy::AnalysisAlways),
            )
            .expect("template query")
    }

    fn publish(&self, session: &Session, fill: u8) -> FrameStamp {
        self.clock.advance(CAPTURE_TICK);
        self.capture
            .publish(fill, Continuity::Continuous)
            .expect("publication accepted");
        session
            .acquire_frame(&FrameRequest::latest(), &bounded())
            .expect("published current frame")
            .stamp()
    }

    fn quiescent(&self) {
        until("physical work retirement", || {
            let observed = self.engine.ocr_text_observation();
            observed.physical_ocr_in_flight == 0 && observed.template_mapping_reservations == 0
        });
    }
}

fn backend() -> ControlledOcr {
    ControlledOcr::new(FORMAT)
        .with_descriptor(OcrBackendDescriptor::new(
            OcrBackendIdentity::new(
                OcrBackendId::new(mado_pilot::DEFAULT_OCR_BACKEND_ID).expect("backend id"),
                OcrBackendVersion::new(mado_pilot::DEFAULT_OCR_BACKEND_VERSION)
                    .expect("backend version"),
            ),
            OcrModelIdentity::accepted_bounded_detector(),
            FORMAT,
        ))
        .with_provider_descriptor(OcrProviderDescriptor::new(
            OcrExecutionProviderPolicy::Cpu,
            OcrExecutionProvider::Cpu,
            None,
            ProviderProfileId::new(mado_pilot::DEFAULT_OCR_RUNTIME_PROFILE_ID)
                .expect("runtime profile"),
        ))
}

fn positive() -> Vec<ScriptedOcrCandidate> {
    vec![ScriptedOcrCandidate::new(
        "  cafe\u{301} READY  ".as_bytes(),
        [(53.0, 60.0), (203.0, 60.0), (203.0, 112.0), (53.0, 112.0)],
        CONFIDENCE,
        0,
    )]
}

fn bounded() -> OperationContext {
    OperationContext::new()
        .with_timeout(WAIT)
        .expect("finite operation deadline")
}

fn until(label: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !predicate() {
        assert!(
            Instant::now() < deadline,
            "bounded checkpoint did not complete: {label}"
        );
        std::thread::yield_now();
    }
}

fn progress(
    query: &OcrTextQuery,
    predicate: impl Fn(OcrTextQueryProgress) -> bool,
) -> OcrTextQueryProgress {
    until("query progress", || {
        assert!(
            matches!(query.poll(), OcrTextQueryOutcome::Pending(_)),
            "query terminated before checkpoint"
        );
        predicate(query.progress())
    });
    query.progress()
}

fn settled(query: &OcrTextQuery, stamp: FrameStamp) -> OcrTextQueryProgress {
    progress(query, |p| {
        p.last_frame() == Some(stamp) && p.pending_count() == 0 && p.physical_in_flight_count() == 0
    })
}

fn matched(terminal: &OcrTextTerminalOutcome) -> &OcrTextWatchResult {
    let OcrTextTerminalOutcome::Matched(result) = terminal else {
        panic!("expected a matching terminal")
    };
    result
}

fn verify_result(
    result: &OcrTextWatchResult,
    stamp: FrameStamp,
    first: FrameStamp,
    confirmations: u32,
) {
    assert!(
        result.frame().stamp() == stamp
            && result.result().stamp() == stamp
            && result.result().transform() == result.frame().transform(),
        "exact result source differs"
    );
    assert!(
        result.literal() == LITERAL && result.minimum_confidence() == CONFIDENCE,
        "normalized predicate or inclusive threshold differs"
    );
    assert!(
        result.result().regions().len() == 1 && result.result().regions()[0].text() == TEXT,
        "normalized output differs"
    );
    assert!(
        result.satisfying_region_indexes() == [0] && result.matching_regions().len() == 1,
        "satisfying indexes differ"
    );
    assert_eq!(
        result.confirmed_observations(),
        confirmations,
        "confirmation count differs"
    );
    let stability = if confirmations == 1 {
        OcrTextStability::immediate()
    } else {
        OcrTextStability::consecutive(confirmations).expect("positive confirmations")
    };
    assert_eq!(result.stability(), stability, "retained stability differs");
    assert_eq!(
        result.first_confirmed_frame(),
        first,
        "first confirmation source differs"
    );
    assert_eq!(
        result.result().effective_region(),
        PixelRect::new(0, 0, 960, 540).expect("ROI")
    );
    assert_eq!(result.retained_extent().source_bytes(), SOURCE_BYTES);
}

fn verify_pixels(evidence: &mut Evidence, frame: &Frame, stamp: FrameStamp, fill: u8) {
    let mapped = frame
        .map(FORMAT, &bounded())
        .expect("retained exact mapping");
    assert!(
        mapped.stamp() == stamp && mapped.bytes().iter().all(|byte| *byte == fill),
        "retained pixels differ from their exact source"
    );
    evidence.retained_read_bytes += mapped.descriptor().byte_len() as u64;
}
