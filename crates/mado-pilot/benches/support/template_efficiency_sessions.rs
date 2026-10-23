//! Prospective public Session units and explicit backend-owner scripts.
//!
//! Public answers are mode-independent. Physical work assertions describe only
//! the separately attested `prepared-cache-row-winner-v1` implementation.

use std::sync::Arc;

use mado_pilot::{
    BackendDescriptor, ClipPolicy, Continuity, CpuMapping, FindRequest, Frame, FrameRequest,
    FrameStamp, FrameView, MatchResult, OpenRequest, PixelRect, PreparedTemplate, RegionSelection,
    Session, Status, TransformSnapshot,
};
use mado_pilot_backend_opencv::OpenCvBackend;
#[cfg(feature = "template-matching-benchmark")]
use mado_pilot_backend_opencv::benchmark_instrumentation as backend;
use mado_pilot_runtime::{
    CaptureProvider, Engine, EngineWiring, IdentityIssuer, Matcher, PackageLoader,
};
use mado_pilot_testkit::{ControlledCapture, MatchBackend, MatchRequest};
use serde_json::{Value, json};

use super::fixtures::{self, SCORE_TOLERANCE, Workload};
use super::session_fixtures::{self, Case};
use super::{ClockStart, Memory, Sample, operation};

const CACHE_BYTES: u64 = 33_554_432;
const INDEPENDENT_BGR: u64 = 3840 * 2160 * 3;

#[derive(Clone, Copy, Default)]
pub(super) struct Timing {
    elapsed_ns: u64,
    cpu_ns: Option<u64>,
}

impl Timing {
    pub(super) fn finish(clock: ClockStart) -> Self {
        let (elapsed_ns, cpu_ns) = clock.finish();
        Self { elapsed_ns, cpu_ns }
    }

    pub(super) fn add(&mut self, other: Self) {
        self.elapsed_ns += other.elapsed_ns;
        self.cpu_ns = self
            .cpu_ns
            .zip(other.cpu_ns)
            .map(|(left, right)| left + right);
    }

    fn json(self) -> Value {
        json!({"elapsed_ns": self.elapsed_ns, "cpu_ns": self.cpu_ns})
    }
}

/// Owns one controlled source and its runtime; the external backend is separate.
pub(super) struct Runtime {
    capture: Option<Arc<ControlledCapture>>,
    engine: Option<Engine>,
    session: Option<Session>,
}

impl Runtime {
    fn new(row: Workload, backend: &Arc<OpenCvBackend>) -> Result<Self, String> {
        let issuer = Arc::new(IdentityIssuer::new());
        let capture = Arc::new(
            ControlledCapture::new(Arc::clone(&issuer), row.extent(), row.format)
                .map_err(|error| error.to_string())?,
        );
        let engine = Engine::new(EngineWiring {
            engine: issuer.engine(),
            capture: Arc::clone(&capture) as Arc<dyn CaptureProvider>,
            matcher: Matcher::new(Arc::clone(backend) as Arc<dyn MatchBackend>),
            loader: PackageLoader::new(),
            ocr: None,
            input: None,
            permission: None,
        })
        .map_err(|error| error.to_string())?;
        let targets = engine
            .discover(&operation())
            .map_err(|error| error.to_string())?;
        let target = targets.first().ok_or("controlled target missing")?.id();
        if target != capture.target() {
            return Err("discovery returned a foreign controlled target".into());
        }
        let session = engine
            .open(target, &OpenRequest::new(), &operation())
            .map_err(|error| error.to_string())?;
        Ok(Self {
            capture: Some(capture),
            engine: Some(engine),
            session: Some(session),
        })
    }

    pub(super) fn session(&self) -> &Session {
        self.session.as_ref().expect("live runtime session")
    }

    pub(super) fn capture(&self) -> &ControlledCapture {
        self.capture.as_ref().expect("live controlled source")
    }

    pub(super) fn publish(&self, pixels: &[u8]) -> Result<Frame, String> {
        self.capture()
            .publish_pixels(pixels, Continuity::Continuous)
            .map_err(|error| error.to_string())?;
        self.session()
            .acquire_frame(&FrameRequest::latest(), &operation())
            .map_err(|error| error.to_string())
    }

    pub(super) fn close(&self) -> Result<(), String> {
        self.session()
            .close(&operation())
            .map_err(|error| error.to_string())?;
        require(self.session().is_closed(), "session close did not settle")
    }

    pub(super) fn release(mut self) -> Result<(), String> {
        let closed = self.close();
        self.drop_owners();
        closed
    }

    fn drop_owners(&mut self) {
        drop(self.session.take());
        drop(self.engine.take());
        drop(self.capture.take());
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        if let Some(session) = &self.session {
            let _ = session.close(&operation());
        }
        self.drop_owners();
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(super) struct FrameFacts {
    pub(super) stamp: FrameStamp,
    transform: TransformSnapshot,
}

impl FrameFacts {
    pub(super) fn of(frame: &Frame) -> Self {
        Self {
            stamp: frame.stamp(),
            transform: *frame.transform(),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
struct MatchProjection {
    bounds: PixelRect,
    score: f64,
    template_matches: bool,
}

/// Copies at most eight scalar answers, never a frame, result or template owner.
#[derive(Clone, Copy, PartialEq)]
pub(super) struct Projection {
    pub(super) stamp: FrameStamp,
    envelope_correct: bool,
    best_consistent: bool,
    matches: [Option<MatchProjection>; 8],
    count: usize,
}

impl Projection {
    pub(super) fn capture(
        result: &MatchResult,
        frame: &Frame,
        expected: FrameFacts,
        template: &PreparedTemplate,
        searched: PixelRect,
        descriptor: &BackendDescriptor,
        target_correct: bool,
    ) -> Result<Self, String> {
        if result.matches().len() > 8 {
            return Err("public result exceeded the eight-match projection".into());
        }
        let mut matches = [None; 8];
        for (slot, found) in matches.iter_mut().zip(result.matches()) {
            *slot = Some(MatchProjection {
                bounds: found.bounds(),
                score: found.score(),
                template_matches: found.template() == template.id(),
            });
        }
        Ok(Self {
            stamp: result.stamp(),
            envelope_correct: target_correct
                && FrameFacts::of(frame) == expected
                && result.stamp() == expected.stamp
                && *result.transform() == expected.transform
                && result.searched() == searched
                && result.options() == session_fixtures::options()
                && result.backend() == descriptor
                && result.backend().id() == template.backend().as_str(),
            best_consistent: result.best() == result.matches().first()
                && result.is_empty() == result.matches().is_empty(),
            count: result.matches().len(),
            matches,
        })
    }

    pub(super) fn correct(
        self,
        row: Workload,
        template: usize,
        variant: u32,
        hit: bool,
        region: PixelRect,
    ) -> bool {
        if !self.envelope_correct || !self.best_consistent || self.count != usize::from(hit) {
            return false;
        }
        let (x, y) = fixtures::origin(row.extent(), template, variant);
        let patch = session_fixtures::patch(row);
        let bounds = PixelRect::new(
            x,
            y,
            x + i32::try_from(patch.width()).expect("bounded patch width"),
            y + i32::try_from(patch.height()).expect("bounded patch height"),
        )
        .expect("bounded planted patch");
        self.matches.iter().flatten().all(|found| {
            found.template_matches
                && found.bounds == bounds
                && region.contains_rect(found.bounds)
                && found.score.is_finite()
                && (0.0..=1.0).contains(&found.score)
                && found.score >= session_fixtures::options().min_score()
                && (found.score - 1.0).abs() <= SCORE_TOLERANCE
        })
    }

    fn json(self) -> Value {
        let matches: Vec<_> = self.matches.iter().flatten().map(|found| json!({
            "bounds": [found.bounds.left(), found.bounds.top(), found.bounds.right(), found.bounds.bottom()],
            "score": found.score, "template_matches": found.template_matches,
        })).collect();
        json!({"stamp": stamp_json(self.stamp), "envelope_correct": self.envelope_correct,
            "best_consistent": self.best_consistent, "matches": matches})
    }
}

pub(super) fn stamp_json(stamp: FrameStamp) -> Value {
    json!([
        stamp.stream().engine().to_string(),
        stamp.stream().get(),
        stamp.epoch().value(),
        stamp.sequence().value(),
        stamp.geometry().value()
    ])
}

pub(super) fn next_stamp(previous: FrameStamp, next: FrameStamp) -> bool {
    previous.stream() == next.stream()
        && previous.epoch() == next.epoch()
        && previous.geometry() == next.geometry()
        && previous.sequence().value().checked_add(1) == Some(next.sequence().value())
}

#[derive(Clone, Copy)]
struct Gauges {
    retained: Option<u64>,
    evicted: Option<u64>,
}

impl Gauges {
    fn observe() -> Self {
        #[cfg(feature = "template-matching-benchmark")]
        {
            let observed = backend::work_snapshot();
            Self {
                retained: Some(observed.bgr_retained_bytes),
                evicted: Some(observed.bgr_evicted_bytes),
            }
        }
        #[cfg(not(feature = "template-matching-benchmark"))]
        {
            Self {
                retained: None,
                evicted: None,
            }
        }
    }
}

struct Checkpoint {
    name: &'static str,
    memory: Memory,
    gauges: Gauges,
    owner_correct: Option<bool>,
    error: Option<String>,
}

impl Checkpoint {
    fn observe(name: &'static str, entry: Gauges, backends: u64, exact: Option<u64>) -> Self {
        let gauges = Gauges::observe();
        let owner_correct = gauges
            .retained
            .zip(entry.retained)
            .zip(gauges.evicted.zip(entry.evicted))
            .map(|((retained, before), (evicted, prior))| {
                retained.checked_sub(before).is_some_and(|bytes| {
                    bytes <= backends * CACHE_BYTES
                        && exact.is_none_or(|expected| bytes == expected)
                }) && evicted == prior
            });
        Self {
            name,
            memory: Memory::observe(),
            gauges,
            owner_correct,
            error: None,
        }
    }

    fn correct(&self) -> bool {
        self.owner_correct != Some(false) && self.error.is_none()
    }

    fn json(&self) -> Value {
        #[cfg(feature = "template-matching-benchmark")]
        let heap = Some(self.memory.heap);
        #[cfg(not(feature = "template-matching-benchmark"))]
        let heap: Option<u64> = None;
        let mut value = json!({
            "name": self.name, "rss_bytes": self.memory.rss, "peak_rss_bytes": self.memory.peak_rss,
            "private_bytes": self.memory.private, "footprint_bytes": self.memory.footprint,
            "heap_bytes": heap, "bgr_retained_bytes": self.gauges.retained,
            "bgr_evicted_bytes": self.gauges.evicted, "owner_correct": self.owner_correct,
        });
        if let Some(error) = &self.error {
            value["error"] = json!(error);
        }
        value
    }
}

pub(super) struct Unit {
    pub(super) primary: Timing,
    pub(super) setup: Timing,
    pub(super) teardown: Timing,
    pub(super) projections: [Option<Projection>; 8],
    strata: [Option<Timing>; 4],
    checkpoints: [Option<Checkpoint>; 8],
    pub(super) query: Option<super::queries::QueryRecord>,
    lifecycle: Option<Value>,
    pub(super) query_setup: Option<[bool; 3]>,
    pub(super) query_teardown: Option<[bool; 4]>,
    mapping_created: Option<bool>,
    mapping_released: Option<bool>,
    pub(super) error: Option<String>,
}

impl Unit {
    pub(super) fn new(setup: Timing) -> Self {
        Self {
            primary: Timing::default(),
            setup,
            teardown: Timing::default(),
            projections: [None; 8],
            strata: [None; 4],
            checkpoints: std::array::from_fn(|_| None),
            query: None,
            lifecycle: None,
            query_setup: None,
            query_teardown: None,
            mapping_created: None,
            mapping_released: None,
            error: None,
        }
    }

    pub(super) fn fail(&mut self, error: impl Into<String>) {
        let error = error.into();
        match &mut self.error {
            Some(existing) => {
                existing.push_str("; ");
                existing.push_str(&error);
            }
            None => self.error = Some(error),
        }
    }

    fn checkpoint(&mut self, checkpoint: Checkpoint) {
        let slot = self
            .checkpoints
            .iter_mut()
            .find(|slot| slot.is_none())
            .expect("at most eight declared owner checkpoints");
        *slot = Some(checkpoint);
    }

    fn finish(
        mut self,
        row: Workload,
        regions: &[PixelRect],
        work: Value,
        work_correct: bool,
    ) -> (Sample, Value) {
        let case = session_fixtures::case(row);
        let expected = match case {
            Case::Group | Case::DistinctGroup | Case::OwnerGroup => 8,
            Case::Turnover | Case::LifecycleIndependent => 4,
            Case::QueryDeadline => 0,
            _ => 1,
        };
        let projected = self.projections.iter().flatten().count() == expected
            && self
                .projections
                .iter()
                .enumerate()
                .all(|(index, projection)| {
                    projection.is_none_or(|projection| {
                        let template =
                            if matches!(case, Case::Group | Case::DistinctGroup | Case::OwnerGroup)
                            {
                                index
                            } else {
                                0
                            };
                        let variant = u32::from(
                            matches!(case, Case::QueryMatch)
                                || (matches!(case, Case::Turnover) && index == 3),
                        );
                        let hit = !matches!(case, Case::RetainedMiss)
                            && !(matches!(case, Case::Turnover) && index % 2 == 0);
                        projection.correct(row, template, variant, hit, regions[template])
                    })
                });
        let checkpoints_correct = self.checkpoints.iter().flatten().all(Checkpoint::correct);
        if !projected {
            self.fail("public projection oracle failed");
        }
        if !work_correct {
            self.fail("prepared-cache-row-winner-v1 work oracle failed");
        }
        if !checkpoints_correct {
            self.fail("logical owner checkpoint failed");
        }
        let correct = self.error.is_none();
        let mut setup = self.setup.json();
        let mut teardown = self.teardown.json();
        let overlapping = matches!(case, Case::LifecycleSession | Case::LifecycleIndependent);
        setup["included_in_primary"] = json!(overlapping);
        teardown["included_in_primary"] = json!(overlapping);
        if let Some(created) = self.mapping_created {
            setup["mapping_created"] = json!(created);
        }
        if let Some(released) = self.mapping_released {
            teardown["mapping_released"] = json!(released);
        }
        if let Some(flags) = self.query_setup {
            for (key, value) in ["session_opened", "query_started", "producer_ready"]
                .into_iter()
                .zip(flags)
            {
                setup[key] = json!(value);
            }
        }
        if let Some(flags) = self.query_teardown {
            for (key, value) in [
                "session_closed",
                "retained_terminal_checked",
                "query_and_outcome_released",
                "runtime_and_source_released",
            ]
            .into_iter()
            .zip(flags)
            {
                teardown[key] = json!(value);
            }
        }
        let strata = if matches!(case, Case::Turnover) {
            json!({"hit": [self.strata[1].map(Timing::json), self.strata[3].map(Timing::json)],
                "miss": [self.strata[0].map(Timing::json), self.strata[2].map(Timing::json)]})
        } else {
            json!({})
        };
        let observation = json!({
            "correct": correct, "error": self.error,
            "projections": self.projections.iter().flatten().copied().map(Projection::json).collect::<Vec<_>>(),
            "strata": strata,
            "checkpoints": self.checkpoints.iter().flatten().map(Checkpoint::json).collect::<Vec<_>>(),
            "query": self.query.as_ref().map(super::queries::QueryRecord::json),
            "setup": setup, "teardown": teardown, "lifecycle": self.lifecycle, "work": work,
        });
        (
            Sample {
                elapsed_ns: self.primary.elapsed_ns,
                cpu_ns: self.primary.cpu_ns,
                correct,
                error: self.error,
            },
            observation,
        )
    }
}

#[derive(Clone, Copy)]
struct WorkStart {
    #[cfg(feature = "template-matching-benchmark")]
    finds: backend::Snapshot,
    #[cfg(feature = "template-matching-benchmark")]
    work: backend::WorkSnapshot,
}

impl WorkStart {
    fn now() -> Self {
        Self {
            #[cfg(feature = "template-matching-benchmark")]
            finds: backend::snapshot(),
            #[cfg(feature = "template-matching-benchmark")]
            work: backend::work_snapshot(),
        }
    }

    fn finish(self, row: Workload, region: PixelRect, iteration: u64) -> (Value, bool) {
        #[cfg(not(feature = "template-matching-benchmark"))]
        {
            let _ = (self, row, region, iteration);
            (Value::Null, true)
        }
        #[cfg(feature = "template-matching-benchmark")]
        {
            let work = backend::work_snapshot();
            let finds = backend::snapshot().checked_delta(self.finds);
            let case = session_fixtures::case(row);
            let (calls, conversions, copies) = match case {
                Case::RetainedHit | Case::RetainedMiss | Case::Patch64 => {
                    (1, u64::from(iteration == 0), u64::from(row.roi))
                }
                Case::Group => (8, 1, if row.roi { 8 } else { 0 }),
                Case::DistinctGroup => (8, 8, 8),
                Case::OwnerGroup => (8, 1, 1),
                Case::Turnover => (4, 4, if row.roi { 4 } else { 0 }),
                Case::QueryMatch => (3, 3, if row.roi { 3 } else { 0 }),
                Case::QueryDeadline => (1, 1, u64::from(row.roi)),
                Case::LifecycleSession => (1, 1, 1),
                Case::LifecycleIndependent => (4, 2, 0),
            };
            let pixels = u64::from(region.width()) * u64::from(region.height());
            let converted = work.bgr_conversions.checked_sub(self.work.bgr_conversions);
            let converted_bytes = work
                .bgr_converted_bytes
                .checked_sub(self.work.bgr_converted_bytes);
            let mapped = work.mapping_copies.checked_sub(self.work.mapping_copies);
            let mapped_bytes = work
                .mapping_copied_bytes
                .checked_sub(self.work.mapping_copied_bytes);
            let correct = finds.is_some_and(|observed| {
                observed.find_calls == calls
                    && observed.find_completions == calls
                    && observed.find_failures == 0
            }) && converted == Some(conversions)
                && converted_bytes == Some(conversions * pixels * 3)
                && mapped == Some(copies)
                && mapped_bytes == Some(copies * pixels * 4);
            (
                json!({"accounting_contract": "prepared-cache-row-winner-v1",
                "backend_find_calls": finds.map(|value| value.find_calls),
                "backend_find_completions": finds.map(|value| value.find_completions),
                "backend_find_failures": finds.map(|value| value.find_failures),
                "bgr_conversions": converted, "bgr_converted_bytes": converted_bytes,
                "mapping_copies": mapped, "mapping_copied_bytes": mapped_bytes, "correct": correct}),
                correct,
            )
        }
    }
}

pub(super) struct Fixture {
    row: Workload,
    regions: Vec<PixelRect>,
    pixels: [Vec<u8>; 3],
    templates: Vec<PreparedTemplate>,
    descriptor: BackendDescriptor,
    external_backend: Option<Arc<OpenCvBackend>>,
    runtime: Option<Runtime>,
    frame: Option<Frame>,
    mapping: Option<CpuMapping>,
    entry: Gauges,
    checkpoints: Vec<Checkpoint>,
    setup: Timing,
    work: WorkStart,
    iteration: u64,
    last_stamp: Option<FrameStamp>,
    prepared: bool,
}

impl Fixture {
    pub(super) fn new(row: Workload) -> Result<Self, String> {
        let entry = Gauges::observe();
        let checkpoint = Checkpoint::observe("before-setup", entry, 0, Some(0));
        let case = session_fixtures::case(row);
        let external_backend = Arc::new(OpenCvBackend::new().map_err(|error| error.to_string())?);
        let descriptor = external_backend.descriptor();
        let runtime = Runtime::new(row, &external_backend)?;
        let templates = session_fixtures::templates(row)
            .iter()
            .map(|source| {
                runtime
                    .engine
                    .as_ref()
                    .expect("live compiler engine")
                    .prepare_template(source, &operation())
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let cycle = matches!(
            case,
            Case::Turnover | Case::QueryMatch | Case::QueryDeadline
        );
        let pixels = [
            session_fixtures::scene(row, 0, !cycle && !matches!(case, Case::RetainedMiss)),
            if cycle && !matches!(case, Case::QueryDeadline) {
                session_fixtures::scene(row, 0, true)
            } else {
                Vec::new()
            },
            if cycle && !matches!(case, Case::QueryDeadline) {
                session_fixtures::scene(row, 1, true)
            } else {
                Vec::new()
            },
        ];
        let persistent = matches!(
            case,
            Case::RetainedHit | Case::RetainedMiss | Case::Patch64 | Case::Turnover
        );
        let keep_backend = persistent || matches!(case, Case::QueryMatch | Case::QueryDeadline);
        let runtime = if persistent {
            Some(runtime)
        } else {
            runtime.release()?;
            None
        };
        let external_backend = if keep_backend {
            Some(external_backend)
        } else {
            drop(external_backend);
            None
        };
        let mut checkpoints = Vec::with_capacity(7);
        checkpoints.push(checkpoint);
        Ok(Self {
            row,
            regions: session_fixtures::regions(row),
            pixels,
            templates,
            descriptor,
            external_backend,
            runtime,
            frame: None,
            mapping: None,
            entry,
            checkpoints,
            setup: Timing::default(),
            work: WorkStart::now(),
            iteration: 0,
            last_stamp: None,
            prepared: false,
        })
    }

    pub(super) fn prepare_iteration(&mut self) -> Result<(), String> {
        self.work = WorkStart::now();
        let setup = ClockStart::now();
        let result = self.prepare();
        self.setup = Timing::finish(setup);
        self.prepared = result.is_ok();
        result
    }

    fn prepare(&mut self) -> Result<(), String> {
        let case = session_fixtures::case(self.row);
        if matches!(case, Case::LifecycleSession | Case::LifecycleIndependent) {
            return Ok(());
        }
        if self.runtime.is_none() {
            if self.external_backend.is_none() {
                self.external_backend = Some(Arc::new(
                    OpenCvBackend::new().map_err(|error| error.to_string())?,
                ));
            }
            self.runtime = Some(Runtime::new(
                self.row,
                self.external_backend.as_ref().expect("external backend"),
            )?);
        }
        if !matches!(
            case,
            Case::Turnover | Case::QueryMatch | Case::QueryDeadline
        ) && self.frame.is_none()
        {
            self.frame = Some(
                self.runtime
                    .as_ref()
                    .expect("prepared runtime")
                    .publish(&self.pixels[0])?,
            );
        }
        if matches!(case, Case::OwnerGroup) {
            let frame = self.frame.as_ref().expect("owner-group frame");
            self.mapping = Some(
                FrameView::new(frame.clone(), self.regions[0])
                    .map_err(|error| error.to_string())?
                    .map(self.row.format, &operation())
                    .map_err(|error| error.to_string())?,
            );
            require(
                mapping_correct(
                    self.mapping.as_ref().expect("created mapping"),
                    frame,
                    self.regions[0],
                    self.row,
                    &self.pixels[0],
                ),
                "owner-group mapping descriptor or pixels differ",
            )?;
        }
        Ok(())
    }

    pub(super) fn run(&mut self) -> (Sample, Value) {
        let case = session_fixtures::case(self.row);
        let mut unit = if !self.prepared {
            let mut unit = Unit::new(self.setup);
            unit.fail("iteration setup did not complete");
            unit
        } else {
            match case {
                Case::QueryMatch | Case::QueryDeadline => super::queries::run(
                    self.runtime.take().expect("prepared query runtime"),
                    self.row,
                    &self.templates[0],
                    &self.pixels,
                    self.regions[0],
                    &self.descriptor,
                    self.setup,
                ),
                Case::Turnover => self.turnover(),
                Case::LifecycleSession => self.lifecycle_session(),
                Case::LifecycleIndependent => self.lifecycle_independent(),
                _ => self.searches(),
            }
        };
        if matches!(case, Case::Group | Case::DistinctGroup | Case::OwnerGroup) {
            let teardown = ClockStart::now();
            drop(self.mapping.take());
            if matches!(case, Case::OwnerGroup) {
                unit.mapping_released = Some(self.mapping.is_none());
            }
            drop(self.frame.take());
            if let Some(runtime) = self.runtime.take()
                && let Err(error) = runtime.release()
            {
                unit.fail(error);
            }
            drop(self.external_backend.take());
            unit.teardown = Timing::finish(teardown);
        }
        let (work, correct) = self.work.finish(self.row, self.regions[0], self.iteration);
        self.iteration += 1;
        self.prepared = false;
        unit.finish(self.row, &self.regions, work, correct)
    }

    fn searches(&mut self) -> Unit {
        let mut unit = Unit::new(self.setup);
        let case = session_fixtures::case(self.row);
        let group = matches!(case, Case::Group | Case::DistinctGroup | Case::OwnerGroup);
        if matches!(case, Case::OwnerGroup) {
            unit.mapping_created = Some(self.mapping.is_some());
        }
        let frame = self.frame.as_ref().expect("prepared search frame");
        let runtime = self.runtime.as_ref().expect("prepared search runtime");
        let count = if group { 8 } else { 1 };
        let operation = operation();
        let requests: [Option<FindRequest<'_>>; 8] = std::array::from_fn(|index| {
            (index < count).then(|| {
                FindRequest::exact(frame, &self.templates[index], session_fixtures::options())
                    .in_region(selection(self.regions[index]))
            })
        });
        let started = ClockStart::now();
        for (index, request) in requests.iter().take(count).enumerate() {
            match search(
                runtime.session(),
                frame,
                request.as_ref().expect("prepared request"),
                self.regions[index],
                &self.descriptor,
                &operation,
            ) {
                Ok(projection) => unit.projections[index] = Some(projection),
                Err(error) => {
                    unit.fail(error);
                    break;
                }
            }
        }
        unit.primary = Timing::finish(started);
        if !group {
            if self.last_stamp.is_some_and(|stamp| stamp != frame.stamp()) {
                unit.fail("retained search changed source identity");
            }
            self.last_stamp = Some(frame.stamp());
        }
        let teardown = ClockStart::now();
        unit.teardown = Timing::finish(teardown);
        unit
    }

    fn turnover(&mut self) -> Unit {
        let mut unit = Unit::new(self.setup);
        let runtime = self.runtime.as_ref().expect("persistent turnover runtime");
        let started = ClockStart::now();
        for (index, pixels) in [0, 1, 0, 2].into_iter().enumerate() {
            let frame = match runtime.publish(&self.pixels[pixels]) {
                Ok(frame) => frame,
                Err(error) => {
                    unit.fail(error);
                    break;
                }
            };
            if self
                .last_stamp
                .is_some_and(|stamp| !next_stamp(stamp, frame.stamp()))
            {
                unit.fail("turnover publication did not advance the same stream exactly once");
            }
            self.last_stamp = Some(frame.stamp());
            let operation = operation();
            let request =
                FindRequest::exact(&frame, &self.templates[0], session_fixtures::options())
                    .in_region(selection(self.regions[0]));
            let search_start = ClockStart::now();
            let result = search(
                runtime.session(),
                &frame,
                &request,
                self.regions[0],
                &self.descriptor,
                &operation,
            );
            drop(frame);
            unit.strata[index] = Some(Timing::finish(search_start));
            match result {
                Ok(projection) => unit.projections[index] = Some(projection),
                Err(error) => {
                    unit.fail(error);
                    break;
                }
            }
        }
        unit.primary = Timing::finish(started);
        unit.teardown = Timing::finish(ClockStart::now());
        unit
    }

    pub(super) fn checkpoint(&mut self, name: &'static str) {
        let count = u64::from(self.external_backend.is_some());
        self.checkpoints
            .push(Checkpoint::observe(name, self.entry, count, None));
    }

    pub(super) fn mapping_owner_live(&self) -> bool {
        self.mapping.is_some()
    }

    pub(super) fn finish(&mut self) -> Result<Value, String> {
        drop(self.mapping.take());
        drop(self.frame.take());
        self.checkpoints.push(Checkpoint::observe(
            "frame-and-result-released",
            self.entry,
            1,
            None,
        ));
        let closed = self.runtime.as_ref().map(Runtime::close).transpose();
        let mut checkpoint = Checkpoint::observe("session-closed", self.entry, 1, None);
        if let Err(error) = closed {
            checkpoint.error = Some(error);
        }
        self.checkpoints.push(checkpoint);
        if let Some(mut runtime) = self.runtime.take() {
            runtime.drop_owners();
        }
        self.checkpoints.push(Checkpoint::observe(
            "runtime-and-source-released",
            self.entry,
            1,
            None,
        ));
        self.templates.clear();
        drop(self.external_backend.take());
        self.checkpoints.push(Checkpoint::observe(
            "final-backend-released",
            self.entry,
            0,
            Some(0),
        ));
        Ok(Value::Array(
            self.checkpoints.iter().map(Checkpoint::json).collect(),
        ))
    }

    fn lifecycle_session(&self) -> Unit {
        let mut unit = Unit::new(self.setup);
        let entry = Gauges::observe();
        unit.checkpoint(Checkpoint::observe("before-setup", entry, 0, Some(0)));
        let primary = ClockStart::now();
        let setup = ClockStart::now();
        let mut external = None;
        let mut runtime = None;
        let mut frame = None;
        let mut mapping = None;
        let mut outcome = None;
        let mut retained_after_close = false;
        let mut closed_search_refused = false;
        let mut mapping_valid_after_close = false;
        let mut source_released = false;
        let result = (|| -> Result<(), String> {
            external = Some(Arc::new(
                OpenCvBackend::new().map_err(|error| error.to_string())?,
            ));
            runtime = Some(Runtime::new(
                self.row,
                external.as_ref().expect("lifecycle backend"),
            )?);
            frame = Some(
                runtime
                    .as_ref()
                    .expect("lifecycle runtime")
                    .publish(&self.pixels[0])?,
            );
            mapping = Some(
                FrameView::new(
                    frame.as_ref().expect("lifecycle frame").clone(),
                    self.regions[0],
                )
                .map_err(|error| error.to_string())?
                .map(self.row.format, &operation())
                .map_err(|error| error.to_string())?,
            );
            unit.setup = Timing::finish(setup);
            let expected = FrameFacts::of(frame.as_ref().expect("lifecycle frame"));
            let source = mapping
                .as_ref()
                .expect("lifecycle mapping")
                .source_instance()
                .clone();
            let live = runtime.as_ref().expect("lifecycle runtime");
            outcome = Some(
                live.session()
                    .find_template(
                        &FindRequest::exact(
                            frame.as_ref().expect("lifecycle frame"),
                            &self.templates[0],
                            session_fixtures::options(),
                        )
                        .in_region(selection(self.regions[0])),
                        &operation(),
                    )
                    .map_err(|error| error.to_string())?,
            );
            let found = outcome.as_ref().expect("lifecycle outcome");
            unit.projections[0] = Some(Projection::capture(
                found.result(),
                found.frame(),
                expected,
                &self.templates[0],
                self.regions[0],
                &self.descriptor,
                found.target() == live.session().target(),
            )?);
            require(
                mapping_correct(
                    mapping.as_ref().expect("lifecycle mapping"),
                    found.frame(),
                    self.regions[0],
                    self.row,
                    &self.pixels[0],
                ),
                "live lifecycle mapping differs",
            )?;
            unit.checkpoint(Checkpoint::observe(
                "live-outcome",
                entry,
                1,
                Some(320 * 180 * 3),
            ));
            drop(frame.take());
            require(
                source.is_alive(),
                "outcome lost its source after caller-frame release",
            )?;
            unit.checkpoint(Checkpoint::observe(
                "caller-frame-released",
                entry,
                1,
                Some(320 * 180 * 3),
            ));
            let teardown = ClockStart::now();
            live.close()?;
            let after = Projection::capture(
                found.result(),
                found.frame(),
                expected,
                &self.templates[0],
                self.regions[0],
                &self.descriptor,
                found.target() == live.session().target(),
            )?;
            retained_after_close = Some(after) == unit.projections[0]
                && after.correct(self.row, 0, 0, true, self.regions[0])
                && source.is_alive();
            mapping_valid_after_close = mapping_correct(
                mapping.as_ref().expect("retained mapping"),
                found.frame(),
                self.regions[0],
                self.row,
                &self.pixels[0],
            );
            closed_search_refused = live
                .session()
                .find_template(
                    &FindRequest::exact(
                        found.frame(),
                        &self.templates[0],
                        session_fixtures::options(),
                    )
                    .in_region(selection(self.regions[0])),
                    &operation(),
                )
                .is_err_and(|error| error.status() == Status::Closed);
            require(
                retained_after_close && mapping_valid_after_close && closed_search_refused,
                "closed Session retained-value/refused-search oracle failed",
            )?;
            unit.checkpoint(Checkpoint::observe(
                "session-closed",
                entry,
                1,
                Some(320 * 180 * 3),
            ));
            drop(mapping.take());
            require(
                source.is_alive(),
                "outcome lost its source after mapping release",
            )?;
            unit.checkpoint(Checkpoint::observe(
                "mapping-released",
                entry,
                1,
                Some(320 * 180 * 3),
            ));
            drop(outcome.take());
            unit.checkpoint(Checkpoint::observe(
                "outcome-released",
                entry,
                1,
                Some(320 * 180 * 3),
            ));
            runtime.take().expect("lifecycle runtime").release()?;
            source_released = !source.is_alive();
            require(
                source_released,
                "runtime/provider retains the released source",
            )?;
            unit.checkpoint(Checkpoint::observe(
                "runtime-and-source-released",
                entry,
                1,
                Some(320 * 180 * 3),
            ));
            drop(external.take());
            unit.checkpoint(Checkpoint::observe(
                "final-backend-released",
                entry,
                0,
                Some(0),
            ));
            unit.teardown = Timing::finish(teardown);
            Ok(())
        })();
        // Drop every remaining owner even when a script step fails; no checkpoint is fabricated.
        drop(frame.take());
        if let Some(live) = runtime.as_ref()
            && let Err(error) = live.close()
        {
            unit.fail(error);
        }
        drop(mapping.take());
        drop(outcome.take());
        if let Some(mut live) = runtime.take() {
            live.drop_owners();
        }
        drop(external.take());
        unit.primary = Timing::finish(primary);
        if let Err(error) = result {
            unit.fail(error);
        }
        unit.lifecycle = Some(json!({"retained_after_close": retained_after_close,
            "closed_search_refused": closed_search_refused, "mapping_valid_after_close": mapping_valid_after_close,
            "source_released": source_released}));
        unit
    }

    fn lifecycle_independent(&self) -> Unit {
        let mut unit = Unit::new(self.setup);
        let entry = Gauges::observe();
        unit.checkpoint(Checkpoint::observe("before-setup", entry, 0, Some(0)));
        let primary = ClockStart::now();
        let setup = ClockStart::now();
        let mut a = None;
        let mut a_clone = None;
        let mut b = None;
        let mut frame = None;
        let mut source_released = false;
        let mut clone_reused = None;
        let mut independent = None;
        let mut reused = None;
        let result = (|| -> Result<(), String> {
            // Direct capture keeps all three Matcher calls on one source allocation.
            let issuer = Arc::new(IdentityIssuer::new());
            let capture = ControlledCapture::new(issuer, self.row.extent(), self.row.format)
                .map_err(|error| error.to_string())?;
            let source_session = capture
                .open(capture.target(), &OpenRequest::new(), &operation())
                .map_err(|error| error.to_string())?;
            let acquired = (|| {
                capture.publish_pixels(&self.pixels[0], Continuity::Continuous)?;
                source_session.frame(&FrameRequest::latest(), &operation())
            })();
            let closed = source_session.close(&operation());
            drop(source_session);
            drop(capture);
            frame = Some(acquired.map_err(|error: mado_pilot::Error| error.to_string())?);
            closed.map_err(|error| error.to_string())?;
            let mapping = frame
                .as_ref()
                .expect("independent frame")
                .map(self.row.format, &operation())
                .map_err(|error| error.to_string())?;
            let source = mapping.source_instance().clone();
            drop(mapping);
            let backend_a = OpenCvBackend::new().map_err(|error| error.to_string())?;
            let backend_a_clone = backend_a.clone();
            let backend_b = OpenCvBackend::new().map_err(|error| error.to_string())?;
            a = Some(Matcher::new(Arc::new(backend_a)));
            a_clone = Some(Matcher::new(Arc::new(backend_a_clone)));
            b = Some(Matcher::new(Arc::new(backend_b)));
            unit.setup = Timing::finish(setup);
            let before = conversion_count();
            let current = frame.as_ref().expect("independent frame");
            for (index, matcher) in [a.as_ref(), a_clone.as_ref(), b.as_ref()]
                .into_iter()
                .enumerate()
            {
                unit.projections[index] = Some(matcher_search(
                    matcher.expect("live independent matcher"),
                    current,
                    &self.templates[0],
                    self.regions[0],
                    &self.descriptor,
                )?);
                if index == 1 {
                    clone_reused = conversion_count()
                        .zip(before)
                        .map(|(now, before)| now.checked_sub(before) == Some(1));
                }
            }
            independent = conversion_count()
                .zip(before)
                .map(|(now, before)| now.checked_sub(before) == Some(2));
            unit.checkpoint(Checkpoint::observe(
                "two-backends-live",
                entry,
                2,
                Some(2 * INDEPENDENT_BGR),
            ));
            let teardown = ClockStart::now();
            drop(a.take());
            drop(a_clone.take());
            unit.checkpoint(Checkpoint::observe(
                "a-released",
                entry,
                1,
                Some(INDEPENDENT_BGR),
            ));
            let before_reuse = conversion_count();
            unit.projections[3] = Some(matcher_search(
                b.as_ref().expect("remaining backend B"),
                current,
                &self.templates[0],
                self.regions[0],
                &self.descriptor,
            )?);
            reused = conversion_count()
                .zip(before_reuse)
                .map(|(now, before)| now == before);
            unit.checkpoint(Checkpoint::observe(
                "b-reused",
                entry,
                1,
                Some(INDEPENDENT_BGR),
            ));
            drop(frame.take());
            source_released = !source.is_alive();
            require(source_released, "independent backend retains source frame")?;
            unit.checkpoint(Checkpoint::observe(
                "frame-and-result-released",
                entry,
                1,
                Some(INDEPENDENT_BGR),
            ));
            drop(b.take());
            unit.checkpoint(Checkpoint::observe(
                "final-backend-released",
                entry,
                0,
                Some(0),
            ));
            unit.teardown = Timing::finish(teardown);
            require(
                clone_reused != Some(false) && independent != Some(false) && reused != Some(false),
                "independent backend preparation ownership differs",
            )
        })();
        drop(frame.take());
        drop(a.take());
        drop(a_clone.take());
        drop(b.take());
        unit.primary = Timing::finish(primary);
        if let Err(error) = result {
            unit.fail(error);
        }
        unit.lifecycle = Some(
            json!({"a_clone_reused": clone_reused, "b_independent": independent,
            "b_reused": reused, "source_released": source_released}),
        );
        unit
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        drop(self.mapping.take());
        drop(self.frame.take());
        if let Some(runtime) = self.runtime.take() {
            let _ = runtime.release();
        }
        self.templates.clear();
        drop(self.external_backend.take());
    }
}

pub(super) fn selection(region: PixelRect) -> RegionSelection {
    RegionSelection::pixels(region, ClipPolicy::Reject).expect("declared valid region")
}

pub(super) fn require(correct: bool, message: &'static str) -> Result<(), String> {
    if correct { Ok(()) } else { Err(message.into()) }
}

fn search(
    session: &Session,
    frame: &Frame,
    request: &FindRequest<'_>,
    region: PixelRect,
    descriptor: &BackendDescriptor,
    operation: &mado_pilot::OperationContext,
) -> Result<Projection, String> {
    let outcome = session
        .find_template(request, operation)
        .map_err(|error| error.to_string())?;
    let projection = Projection::capture(
        outcome.result(),
        outcome.frame(),
        FrameFacts::of(frame),
        request.template(),
        region,
        descriptor,
        outcome.target() == session.target(),
    );
    drop(outcome);
    projection
}

fn matcher_search(
    matcher: &Matcher,
    frame: &Frame,
    template: &PreparedTemplate,
    region: PixelRect,
    descriptor: &BackendDescriptor,
) -> Result<Projection, String> {
    let result = matcher
        .find(
            MatchRequest::new(
                frame,
                selection(region),
                template,
                session_fixtures::options(),
            ),
            &operation(),
        )
        .map_err(|error| error.to_string())?;
    let projection = Projection::capture(
        &result,
        frame,
        FrameFacts::of(frame),
        template,
        region,
        descriptor,
        true,
    );
    drop(result);
    projection
}

fn mapping_correct(
    mapping: &CpuMapping,
    frame: &Frame,
    region: PixelRect,
    row: Workload,
    pixels: &[u8],
) -> bool {
    let descriptor = mapping.descriptor();
    if mapping.stamp() != frame.stamp()
        || mapping.transform() != frame.transform()
        || mapping.region() != region
        || descriptor.format() != row.format
        || descriptor.extent().width() != region.width()
        || descriptor.extent().height() != region.height()
        || descriptor.stride() != region.width() as usize * 4
    {
        return false;
    }
    let width = region.width() as usize * 4;
    let stride = row.width as usize * 4;
    let top = usize::try_from(region.top()).expect("nonnegative fixture top");
    let left = usize::try_from(region.left()).expect("nonnegative fixture left");
    mapping
        .bytes()
        .chunks_exact(width)
        .enumerate()
        .all(|(y, actual)| {
            let offset = (top + y) * stride + left * 4;
            actual == &pixels[offset..offset + width]
        })
}

fn conversion_count() -> Option<u64> {
    #[cfg(feature = "template-matching-benchmark")]
    {
        Some(backend::work_snapshot().bgr_conversions)
    }
    #[cfg(not(feature = "template-matching-benchmark"))]
    {
        None
    }
}
