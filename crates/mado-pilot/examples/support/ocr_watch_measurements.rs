//! Qualification-only observations; physical OCR zero is not ORT process-global teardown.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mado_pilot::{CpuMapping, Engine, OcrTextRetainedExtent, OcrTextWatchResult, OperationContext};
use mado_pilot_backend_onnx::benchmark_instrumentation::{
    ORT_PROFILE_DIR_ENV, OpenStage, OpenStageObserverGuard, install_open_stage_observer,
};

#[path = "../../benches/support/ocr_text_watch_resources.rs"]
mod resources;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Stage {
    ProcessStart,
    RuntimeInitialized,
    ProviderPrepared,
    DetectorSessionReady,
    RecognizerSessionReady,
    EngineReady,
    SessionReady,
    QueryTerminal,
    LogicalCloseReturned,
    PhysicalOcrZero,
    ParentsDropped,
    RetainedResultsDropped,
}

const STAGES: [Stage; 12] = [
    Stage::ProcessStart,
    Stage::RuntimeInitialized,
    Stage::ProviderPrepared,
    Stage::DetectorSessionReady,
    Stage::RecognizerSessionReady,
    Stage::EngineReady,
    Stage::SessionReady,
    Stage::QueryTerminal,
    Stage::LogicalCloseReturned,
    Stage::PhysicalOcrZero,
    Stage::ParentsDropped,
    Stage::RetainedResultsDropped,
];

impl Stage {
    fn label(self) -> &'static str {
        match self {
            Self::ProcessStart => "process-start",
            Self::RuntimeInitialized => "runtime-initialized",
            Self::ProviderPrepared => "provider-prepared",
            Self::DetectorSessionReady => "detector-session-ready",
            Self::RecognizerSessionReady => "recognizer-session-ready",
            Self::EngineReady => "engine-ready",
            Self::SessionReady => "session-ready",
            Self::QueryTerminal => "query-terminal",
            Self::LogicalCloseReturned => "logical-close-returned",
            Self::PhysicalOcrZero => "physical-ocr-zero",
            Self::ParentsDropped => "parents-dropped",
            Self::RetainedResultsDropped => "retained-results-dropped",
        }
    }

    fn open_index(self) -> Option<usize> {
        match self {
            Self::RuntimeInitialized => Some(0),
            Self::ProviderPrepared => Some(1),
            Self::DetectorSessionReady => Some(2),
            Self::RecognizerSessionReady => Some(3),
            _ => None,
        }
    }
}

impl From<OpenStage> for Stage {
    fn from(stage: OpenStage) -> Self {
        match stage {
            OpenStage::RuntimeInitialized => Self::RuntimeInitialized,
            OpenStage::ProviderPrepared => Self::ProviderPrepared,
            OpenStage::DetectorSessionReady => Self::DetectorSessionReady,
            OpenStage::RecognizerSessionReady => Self::RecognizerSessionReady,
        }
    }
}

#[derive(Debug)]
struct Observation {
    stage: Stage,
    elapsed_ns: u64,
    resident: resources::Resident,
}

#[derive(Debug)]
struct Records {
    started: Instant,
    rows: [Option<Observation>; STAGES.len()],
    count: usize,
    open_counts: [u64; 4],
    failure: Option<&'static str>,
}

impl Records {
    fn new(started: Instant) -> Self {
        Self {
            started,
            rows: [const { None }; STAGES.len()],
            count: 0,
            open_counts: [0; 4],
            failure: None,
        }
    }

    fn sample(&mut self, stage: Stage) {
        let Ok(elapsed_ns) = u64::try_from(self.started.elapsed().as_nanos()) else {
            self.failure = Some("measurement timestamp overflow");
            return;
        };
        self.record(Observation {
            stage,
            elapsed_ns,
            resident: resources::resident(),
        });
    }

    fn record(&mut self, observation: Observation) {
        if let Some(index) = observation.stage.open_index() {
            let Some(count) = self.open_counts[index].checked_add(1) else {
                self.failure = Some("open-stage observation count overflow");
                return;
            };
            self.open_counts[index] = count;
        }
        if self.failure.is_some() {
            return;
        }
        if STAGES.get(self.count) != Some(&observation.stage) {
            self.failure = Some("missing, duplicate, extra, or out-of-order measurement stage");
            return;
        }
        if self.count > 0
            && self.rows[self.count - 1]
                .as_ref()
                .is_some_and(|previous| previous.elapsed_ns > observation.elapsed_ns)
        {
            self.failure = Some("measurement clock moved backwards");
            return;
        }
        self.rows[self.count] = Some(observation);
        self.count += 1;
    }

    fn complete(&self) -> Result<(), &'static str> {
        if let Some(failure) = self.failure {
            return Err(failure);
        }
        if self.count != STAGES.len() || self.rows.iter().any(Option::is_none) {
            return Err("measurement stages are incomplete");
        }
        if self.open_counts != [1; 4] {
            return Err("native construction observations differ");
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(super) struct Measurements {
    records: Arc<Mutex<Records>>,
    observer: OpenStageObserverGuard,
    physical_ocr_after_close: Option<u32>,
    physical_ocr_final: Option<u32>,
    retained_extent: Option<OcrTextRetainedExtent>,
    retained_read_mapped_bytes: u64,
}

impl Measurements {
    pub(super) fn start() -> Result<Self, Box<dyn std::error::Error>> {
        let started = Instant::now();
        if std::env::var_os(ORT_PROFILE_DIR_ENV).is_some_and(|value| !value.is_empty()) {
            return Err("qualification requires ORT profiling to be disabled".into());
        }
        let mut records = Records::new(started);
        records.sample(Stage::ProcessStart);
        let records = Arc::new(Mutex::new(records));
        let observed = Arc::clone(&records);
        let observer = install_open_stage_observer(move |stage| {
            observed
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .sample(stage.into());
        });
        Ok(Self {
            records,
            observer,
            physical_ocr_after_close: None,
            physical_ocr_final: None,
            retained_extent: None,
            retained_read_mapped_bytes: 0,
        })
    }

    pub(super) fn record(&self, stage: Stage) {
        // Keep semantic teardown possible after poison; finish refuses that report.
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .sample(stage);
    }

    pub(super) fn close_returned(
        &mut self,
        engine: &Engine,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.physical_ocr_after_close = Some(engine.ocr_text_observation().physical_ocr_in_flight);
        self.record(Stage::LogicalCloseReturned);
        // This is a separate observation deadline, not a forced native cleanup fence.
        let cleanup = OperationContext::new().with_timeout(Duration::from_secs(10))?;
        let checkpoint = || match cleanup.interruption() {
            Some(interruption) => Err(mado_pilot::Error::new(
                interruption.status(),
                "physical OCR retirement observation interrupted",
            )),
            None => Ok(()),
        };
        loop {
            checkpoint()?;
            let physical = engine.ocr_text_observation().physical_ocr_in_flight;
            checkpoint()?;
            if physical == 0 {
                self.physical_ocr_final = Some(physical);
                self.record(Stage::PhysicalOcrZero);
                return Ok(());
            }
            let remaining = cleanup
                .remaining()
                .ok_or("physical OCR retirement observation must have a deadline")?;
            std::thread::sleep(remaining.min(Duration::from_millis(1)));
        }
    }

    pub(super) fn retained_result(&mut self, result: &OcrTextWatchResult) {
        self.retained_extent = Some(result.retained_extent());
    }

    pub(super) fn retained_read(
        &mut self,
        mapping: &CpuMapping,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let bytes = u64::try_from(mapping.descriptor().byte_len())?;
        self.retained_read_mapped_bytes = self
            .retained_read_mapped_bytes
            .checked_add(bytes)
            .ok_or("retained mapping observation overflow")?;
        Ok(())
    }

    // Called only after semantic/source/retention checks and all caller pixel owners are gone.
    pub(super) fn finish(self, scenario: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.record(Stage::RetainedResultsDropped);
        // Keep observation through query/parent/result teardown, then disable and drain
        // before validating or taking the recorder lock. Never drain while holding it.
        drop(self.observer);
        let records = self
            .records
            .lock()
            .map_err(|_| "measurement recorder lock was poisoned")?;
        records.complete()?;
        let physical_ocr_after_close = self
            .physical_ocr_after_close
            .ok_or("logical-close physical OCR observation is missing")?;
        let physical_ocr_final = self
            .physical_ocr_final
            .filter(|value| *value == 0)
            .ok_or("physical OCR quiescence was not observed")?;
        let (scenario, extent) = match (scenario, self.retained_extent) {
            ("transition", Some(extent)) => ("transition", extent),
            ("retina", Some(extent)) => ("retina", extent),
            ("negative", None) if self.retained_read_mapped_bytes == 0 => {
                ("negative", OcrTextRetainedExtent::default())
            }
            _ => return Err("scenario result extent is incomplete or inconsistent".into()),
        };
        if scenario != "negative" && self.retained_read_mapped_bytes == 0 {
            return Err("retained caller mapping observations are missing".into());
        }
        let open_stages = records
            .open_counts
            .iter()
            .try_fold(0_u64, |total, count| total.checked_add(*count))
            .ok_or("open-stage observation total overflow")?;
        for row in &records.rows {
            let observation = row.as_ref().ok_or("measurement stage is missing")?;
            let resident = &observation.resident;
            println!(
                "# ocr-real-stage stage={} elapsed_ns={} resident_current_bytes={:?} resident_process_peak_bytes={:?} private_bytes={:?} physical_footprint_bytes={:?}",
                observation.stage.label(),
                observation.elapsed_ns,
                resident.current,
                resident.peak,
                resident.private,
                resident.footprint,
            );
        }
        println!(
            "# ocr-real-summary schema=1 scenario={scenario} open_stages={open_stages} detector_sessions_created={} recognizer_sessions_created={} physical_ocr_after_close={physical_ocr_after_close} physical_ocr_final={physical_ocr_final} retained_source_extent_bytes={} retained_text_extent_bytes={} retained_index_extent_bytes={} retained_read_mapped_bytes={}",
            records.open_counts[2],
            records.open_counts[3],
            extent.source_bytes(),
            extent.text_bytes(),
            extent.index_bytes(),
            self.retained_read_mapped_bytes,
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(records: &mut Records, stage: Stage, elapsed_ns: u64) {
        records.record(Observation {
            stage,
            elapsed_ns,
            resident: resources::Resident {
                current: None,
                peak: None,
                private: None,
                footprint: None,
            },
        });
    }

    #[test]
    fn missing_reordered_or_regressing_stages_cannot_be_repaired() {
        let mut missing = Records::new(Instant::now());
        for stage in &STAGES[..STAGES.len() - 1] {
            record(&mut missing, *stage, 1);
        }
        assert!(missing.complete().is_err());

        let mut reordered = Records::new(Instant::now());
        record(&mut reordered, Stage::ProcessStart, 1);
        record(&mut reordered, Stage::ProviderPrepared, 2);
        for stage in &STAGES[1..] {
            record(&mut reordered, *stage, 3);
        }
        assert!(reordered.complete().is_err());

        let mut regressed = Records::new(Instant::now());
        record(&mut regressed, Stage::ProcessStart, 2);
        for stage in &STAGES[1..] {
            record(&mut regressed, *stage, 1);
        }
        assert!(regressed.complete().is_err());
    }

    #[test]
    fn extra_construction_during_teardown_or_after_completion_invalidates_report() {
        for boundary in [Stage::LogicalCloseReturned, Stage::RetainedResultsDropped] {
            let mut records = Records::new(Instant::now());
            for stage in STAGES {
                record(&mut records, stage, 1);
                if stage == boundary {
                    record(&mut records, OpenStage::DetectorSessionReady.into(), 1);
                }
            }
            assert!(records.complete().is_err());
        }

        let mut complete = Records::new(Instant::now());
        for stage in STAGES {
            record(&mut complete, stage, 1);
        }
        assert!(complete.complete().is_ok());
    }
}
