//! Bounded self-process observations, separate from frame publication identity.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::Serialize;

#[path = "../../benches/support/ocr_text_watch_resources.rs"]
mod resources;

const INTERVAL: Duration = Duration::from_millis(100);
const MAX_SAMPLES: u64 = 2_048;

#[derive(Debug, Serialize)]
pub(super) struct ProcessSample {
    pub(super) cpu_ns: Option<u64>,
    pub(super) resident_bytes: Option<u64>,
    pub(super) peak_resident_bytes: Option<u64>,
    pub(super) private_bytes: Option<u64>,
    pub(super) footprint_bytes: Option<u64>,
}

pub(super) fn process_sample() -> ProcessSample {
    let memory = resources::resident();
    ProcessSample {
        cpu_ns: process_cpu_ns(),
        resident_bytes: memory.current,
        peak_resident_bytes: memory.peak,
        private_bytes: memory.private,
        footprint_bytes: memory.footprint,
    }
}

#[cfg(target_os = "macos")]
fn process_cpu_ns() -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: RUSAGE_SELF initializes one writable rusage; no pointer is retained.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: successful getrusage initialized the complete structure.
    let usage = unsafe { usage.assume_init() };
    let nanos = |value: libc::timeval| {
        let seconds = u64::try_from(value.tv_sec).ok()?;
        let micros = u64::try_from(value.tv_usec)
            .ok()
            .filter(|value| *value < 1_000_000)?;
        seconds
            .checked_mul(1_000_000_000)?
            .checked_add(micros * 1_000)
    };
    nanos(usage.ru_utime)?.checked_add(nanos(usage.ru_stime)?)
}

#[cfg(windows)]
fn process_cpu_ns() -> Option<u64> {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: the self-process pseudo-handle needs no close. Each output points
    // to a distinct writable FILETIME, and the API retains none of them.
    unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &raw mut creation,
            &raw mut exit,
            &raw mut kernel,
            &raw mut user,
        )
    }
    .ok()?;
    let ticks =
        |value: FILETIME| (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime);
    ticks(kernel).checked_add(ticks(user))?.checked_mul(100)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn process_cpu_ns() -> Option<u64> {
    None
}

pub(super) fn begin_capture_diagnostics() -> Result<(), String> {
    #[cfg(windows)]
    {
        use mado_pilot_platform_windows::benchmark;
        let before = benchmark::capture_metrics();
        if before.detached_textures_live != 0 || before.staging_textures_live != 0 {
            return Err("capture-diagnostics-live-owners".into());
        }
        benchmark::reset_capture_metrics();
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        // The existing ownership tier is 2; higher tiers are rejected by the shim.
        mado_pilot_platform_macos::sck_diagnostics::set_tier(2)
            .map_err(|_| "capture-diagnostics-start-failed".into())
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Err("capture-diagnostics-unsupported-platform".into())
    }
}

pub(super) fn finish_capture_diagnostics() -> Result<(), String> {
    #[cfg(windows)]
    {
        let after = mado_pilot_platform_windows::benchmark::capture_metrics();
        if after.detached_textures_live != 0 || after.staging_textures_live != 0 {
            return Err("capture-diagnostics-live-owners".into());
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        mado_pilot_platform_macos::sck_diagnostics::dump()
            .map_err(|_| "capture-diagnostics-finish-failed".into())
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Err("capture-diagnostics-unsupported-platform".into())
    }
}

#[derive(Debug, Serialize)]
pub(super) struct SampledMetrics {
    pub(super) cpu_ns: Option<u64>,
    pub(super) max_resident_bytes: Option<u64>,
    pub(super) callback_copied_bytes: Option<u64>,
    pub(super) capture_metrics_reason: Option<String>,
    pub(super) platform: &'static str,
    pub(super) sampler_elapsed_ns: u64,
    pub(super) sample_count: u64,
    pub(super) sample_losses: u64,
    pub(super) max_sample_gap_ns: u64,
    pub(super) cpu_reason: Option<String>,
    pub(super) resident_reason: Option<String>,
    pub(super) max_private_bytes: Option<u64>,
    pub(super) max_footprint_bytes: Option<u64>,
    pub(super) callback_invalid_intervals: u64,
    pub(super) native_detached_textures_peak: Option<u64>,
    pub(super) native_staging_textures_peak: Option<u64>,
    pub(super) gpu_engine_percent_mean: Option<f64>,
    pub(super) gpu_engine_percent_max: Option<f64>,
    pub(super) gpu_sample_count: u64,
    pub(super) gpu_sampled_ns: u64,
    pub(super) gpu_reason: Option<String>,
}

pub(super) struct Sampler {
    stop: SyncSender<()>,
    worker: Option<JoinHandle<Result<SampledMetrics, String>>>,
}

impl Sampler {
    pub(super) fn start(stream: Option<u64>) -> Result<Self, String> {
        if stream == Some(0) {
            return Err("metric-stream-invalid".into());
        }
        let state = Observations::start(stream);
        let (stop, receiver) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("capture-pacing-metrics".into())
            .spawn(move || state.run(receiver))
            .map_err(|_| "metric-sampler-start-failed".to_owned())?;
        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }

    pub(super) fn finish(mut self) -> Result<SampledMetrics, String> {
        let _ = self.stop.try_send(());
        self.worker
            .take()
            .ok_or("metric-sampler-already-joined")?
            .join()
            .map_err(|_| "metric-sampler-panicked".to_owned())?
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        let _ = self.stop.try_send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct Observations {
    started: Instant,
    last_sample: Instant,
    initial_cpu: Option<u64>,
    metrics: SampledMetrics,
    #[cfg(windows)]
    stream: Option<u64>,
    #[cfg(windows)]
    baseline: mado_pilot_platform_windows::benchmark::CallbackMetricBaseline,
}

impl Observations {
    fn start(stream: Option<u64>) -> Self {
        let started = Instant::now();
        let initial = process_sample();
        let platform = if cfg!(windows) {
            "windows"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            "unsupported"
        };
        let copied = cfg!(windows) && stream.is_some();
        let mut state = Self {
            started,
            last_sample: started,
            initial_cpu: initial.cpu_ns,
            metrics: SampledMetrics {
                cpu_ns: None,
                max_resident_bytes: None,
                callback_copied_bytes: copied.then_some(0),
                capture_metrics_reason: if copied {
                    None
                } else {
                    Some(
                        if stream.is_none() {
                            "capture-disabled"
                        } else {
                            "native-copy-bytes-not-exposed"
                        }
                        .into(),
                    )
                },
                platform,
                sampler_elapsed_ns: 0,
                sample_count: 0,
                sample_losses: 0,
                max_sample_gap_ns: 0,
                cpu_reason: None,
                resident_reason: None,
                max_private_bytes: initial.private_bytes,
                max_footprint_bytes: initial.footprint_bytes,
                callback_invalid_intervals: 0,
                native_detached_textures_peak: None,
                native_staging_textures_peak: None,
                gpu_engine_percent_mean: None,
                gpu_engine_percent_max: None,
                gpu_sample_count: 0,
                gpu_sampled_ns: 0,
                gpu_reason: Some(
                    if cfg!(windows) {
                        "gpu-no-complete-samples"
                    } else {
                        "per-process-gpu-unavailable-without-privileged-collector"
                    }
                    .into(),
                ),
            },
            #[cfg(windows)]
            stream,
            #[cfg(windows)]
            baseline: mado_pilot_platform_windows::benchmark::callback_metric_baseline(),
        };
        state.observe_process(initial);
        state
    }

    fn observe_process(&mut self, sample: ProcessSample) {
        self.metrics.sample_count += 1;
        // A final OS high-water covers earlier gaps in current-RSS sampling.
        self.metrics.resident_reason = sample
            .peak_resident_bytes
            .is_none()
            .then(|| "process-resident-observation-incomplete".into());
        self.metrics.max_resident_bytes = self
            .metrics
            .max_resident_bytes
            .max(sample.resident_bytes)
            .max(sample.peak_resident_bytes);
        self.metrics.max_private_bytes = self
            .metrics
            .max_private_bytes
            .zip(sample.private_bytes)
            .map(|(left, right)| left.max(right));
        self.metrics.max_footprint_bytes = self
            .metrics
            .max_footprint_bytes
            .zip(sample.footprint_bytes)
            .map(|(left, right)| left.max(right));
        self.metrics.cpu_ns = self
            .initial_cpu
            .zip(sample.cpu_ns)
            .and_then(|(start, end)| end.checked_sub(start));
        self.metrics.cpu_reason = self
            .metrics
            .cpu_ns
            .is_none()
            .then(|| "process-cpu-observation-unavailable".into());
    }

    fn observe_capture(&mut self) {
        #[cfg(windows)]
        {
            use mado_pilot_platform_windows::benchmark;
            let end = benchmark::callback_metric_baseline();
            if let Some(stream) = self.stream {
                let interval =
                    benchmark::callback_copied_bytes_between(self.baseline, end, &[stream]);
                match interval {
                    Ok(bytes) => {
                        if let Some(total) = self.metrics.callback_copied_bytes {
                            self.metrics.callback_copied_bytes = total.checked_add(bytes);
                            if self.metrics.callback_copied_bytes.is_none() {
                                self.metrics.callback_invalid_intervals += 1;
                                self.metrics.capture_metrics_reason =
                                    Some("callback-copy-total-overflow".into());
                            }
                        }
                    }
                    Err(_) => {
                        self.metrics.callback_invalid_intervals += 1;
                        self.metrics.callback_copied_bytes = None;
                        self.metrics.capture_metrics_reason =
                            Some("callback-copy-interval-invalidated".into());
                    }
                }
            }
            // Adjacent baselines avoid treating ordinary ring reuse as loss or callbacks.
            self.baseline = end;
            let native = benchmark::capture_metrics();
            self.metrics.native_detached_textures_peak = Some(native.detached_textures_peak);
            self.metrics.native_staging_textures_peak = Some(native.staging_textures_peak);
        }
    }

    fn run(mut self, stop: Receiver<()>) -> Result<SampledMetrics, String> {
        #[cfg(windows)]
        let mut gpu = match gpu::Query::start() {
            Ok(query) => Some(query),
            Err("gpu-query-close-failed") => return Err("gpu-query-close-failed".into()),
            Err(reason) => {
                self.metrics.gpu_reason = Some(reason.into());
                None
            }
        };
        #[cfg(windows)]
        let mut gpu_failed = false;
        let mut next = self.started + INTERVAL;
        loop {
            let finishing = match stop.recv_timeout(next.saturating_duration_since(Instant::now()))
            {
                Ok(()) | Err(RecvTimeoutError::Disconnected) => true,
                Err(RecvTimeoutError::Timeout) => false,
            };
            if self.metrics.sample_count >= MAX_SAMPLES {
                return Err("metric-sample-cap-exceeded".into());
            }
            #[cfg(windows)]
            if let Some(query) = gpu.as_mut().filter(|_| !gpu_failed) {
                match query.sample() {
                    Ok((percent, elapsed_ns)) => {
                        self.metrics.gpu_sampled_ns = self
                            .metrics
                            .gpu_sampled_ns
                            .checked_add(elapsed_ns)
                            .ok_or("gpu-duration-overflow")?;
                        let weight = elapsed_ns as f64 / self.metrics.gpu_sampled_ns as f64;
                        let mean = self.metrics.gpu_engine_percent_mean.unwrap_or(percent);
                        self.metrics.gpu_engine_percent_mean =
                            Some(mean + (percent - mean) * weight);
                        self.metrics.gpu_engine_percent_max = Some(
                            self.metrics
                                .gpu_engine_percent_max
                                .unwrap_or(percent)
                                .max(percent),
                        );
                        self.metrics.gpu_sample_count += 1;
                        self.metrics.gpu_reason = None;
                    }
                    Err(reason) => {
                        gpu_failed = true;
                        self.metrics.gpu_engine_percent_mean = None;
                        self.metrics.gpu_engine_percent_max = None;
                        self.metrics.gpu_reason = Some(reason.into());
                    }
                }
            }
            self.observe_capture();
            self.observe_process(process_sample());
            let now = Instant::now();
            let gap = u64::try_from(now.duration_since(self.last_sample).as_nanos())
                .map_err(|_| "metric-duration-overflow".to_owned())?;
            self.metrics.max_sample_gap_ns = self.metrics.max_sample_gap_ns.max(gap);
            self.last_sample = now;
            let missed = now.saturating_duration_since(next).as_nanos() / INTERVAL.as_nanos();
            self.metrics.sample_losses +=
                u64::try_from(missed).map_err(|_| "metric-duration-overflow".to_owned())?;
            if finishing {
                #[cfg(windows)]
                if let Some(query) = gpu.take() {
                    query.finish().map_err(str::to_owned)?;
                }
                self.metrics.sampler_elapsed_ns =
                    u64::try_from(now.duration_since(self.started).as_nanos())
                        .map_err(|_| "metric-duration-overflow".to_owned())?;
                return Ok(self.metrics);
            }
            next = now + INTERVAL
                - Duration::from_nanos(
                    u64::try_from(
                        now.saturating_duration_since(next).as_nanos() % INTERVAL.as_nanos(),
                    )
                    .map_err(|_| "metric-duration-overflow".to_owned())?,
                );
        }
    }
}

#[cfg(windows)]
mod gpu {
    use std::time::Instant;

    use windows::Win32::System::Performance::{
        PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE,
        PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA, PdhAddEnglishCounterW, PdhCloseQuery,
        PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW,
    };
    use windows::core::PCWSTR;

    const BUFFER_WORDS: usize = 8_192;
    const BUFFER_BYTES: u32 = (BUFFER_WORDS * size_of::<u64>()) as u32;
    const MAX_ITEMS: u32 = 256;

    pub(super) struct Query {
        query: PDH_HQUERY,
        counter: PDH_HCOUNTER,
        buffer: Box<[u64]>,
        own_prefix: Vec<u16>,
        collected: Instant,
    }

    impl Query {
        pub(super) fn start() -> Result<Self, &'static str> {
            let mut handle = PDH_HQUERY::default();
            // SAFETY: null selects local live data; the output owns a new query.
            if unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &raw mut handle) } != 0 {
                return Err("gpu-query-open-unavailable");
            }
            let prefix = format!("pid_{}_", std::process::id());
            let path: Vec<u16> = format!(r"\GPU Engine({prefix}*)\Utilization Percentage")
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let mut query = Self {
                query: handle,
                counter: PDH_HCOUNTER::default(),
                buffer: vec![0_u64; BUFFER_WORDS].into_boxed_slice(),
                own_prefix: prefix.encode_utf16().collect(),
                collected: Instant::now(),
            };
            // SAFETY: the path is NUL-terminated and alive for this call; the
            // query owns the returned counter and retains no Rust path pointer.
            if unsafe {
                PdhAddEnglishCounterW(
                    query.query,
                    PCWSTR(path.as_ptr()),
                    0,
                    &raw mut query.counter,
                )
            } != 0
            {
                query.finish()?;
                return Err("gpu-counter-add-unavailable");
            }
            // SAFETY: the exclusively owned query and counter remain open.
            if unsafe { PdhCollectQueryData(query.query) } != 0 {
                query.finish()?;
                return Err("gpu-initial-collection-unavailable");
            }
            query.collected = Instant::now();
            Ok(query)
        }

        pub(super) fn sample(&mut self) -> Result<(f64, u64), &'static str> {
            // SAFETY: this thread alone accesses the live, owned query.
            if unsafe { PdhCollectQueryData(self.query) } != 0 {
                return Err("gpu-collection-unavailable");
            }
            let collected = Instant::now();
            let elapsed_ns = u64::try_from(collected.duration_since(self.collected).as_nanos())
                .map_err(|_| "gpu-duration-overflow")?;
            self.collected = collected;
            if elapsed_ns == 0 {
                return Err("gpu-collection-interval-invalid");
            }
            let mut bytes = 0;
            let mut count = 0;
            // SAFETY: zero size with no buffer requests only the required size.
            let status = unsafe {
                PdhGetFormattedCounterArrayW(
                    self.counter,
                    PDH_FMT_DOUBLE,
                    &raw mut bytes,
                    &raw mut count,
                    None,
                )
            };
            if status != PDH_MORE_DATA {
                return Err("gpu-formatted-array-unavailable");
            }
            if bytes > BUFFER_BYTES || count > MAX_ITEMS {
                return Err("gpu-formatted-array-cap-exceeded");
            }
            bytes = BUFFER_BYTES;
            let pointer = self
                .buffer
                .as_mut_ptr()
                .cast::<PDH_FMT_COUNTERVALUE_ITEM_W>();
            // SAFETY: u64 storage has the item's required alignment on the
            // supported x64 target and supplies exactly BUFFER_BYTES writable bytes.
            if unsafe {
                PdhGetFormattedCounterArrayW(
                    self.counter,
                    PDH_FMT_DOUBLE,
                    &raw mut bytes,
                    &raw mut count,
                    Some(pointer),
                )
            } != 0
            {
                return Err("gpu-formatted-array-unavailable");
            }
            let item_bytes = count as usize * size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>();
            if count == 0
                || count > MAX_ITEMS
                || bytes > BUFFER_BYTES
                || item_bytes > bytes as usize
            {
                return Err("gpu-own-process-instances-unavailable");
            }
            let base = self.buffer.as_ptr() as usize;
            let mut percent = 0.0;
            for index in 0..count as usize {
                // SAFETY: successful PDH initialized count complete items; the
                // checked byte extent fits the still-owned allocation.
                let item = unsafe { pointer.add(index).read() };
                let offset = (item.szName.0 as usize)
                    .checked_sub(base)
                    .filter(|offset| *offset >= item_bytes && *offset % 2 == 0)
                    .ok_or("gpu-instance-pointer-invalid")?;
                let units = (bytes as usize)
                    .checked_sub(offset)
                    .ok_or("gpu-instance-pointer-invalid")?
                    / 2;
                if units == 0 {
                    return Err("gpu-instance-name-invalid");
                }
                // SAFETY: the checked offset/extent is inside initialized u64
                // storage. Read only while no PDH call can mutate this buffer.
                let name = unsafe {
                    std::slice::from_raw_parts(
                        self.buffer.as_ptr().cast::<u16>().add(offset / 2),
                        units.min(512),
                    )
                };
                let end = name
                    .iter()
                    .position(|unit| *unit == 0)
                    .ok_or("gpu-instance-name-invalid")?;
                if !name[..end].starts_with(&self.own_prefix) {
                    return Err("gpu-instance-owner-mismatch");
                }
                if item.FmtValue.CStatus != PDH_CSTATUS_VALID_DATA
                    && item.FmtValue.CStatus != PDH_CSTATUS_NEW_DATA
                {
                    return Err("gpu-instance-sample-invalid");
                }
                // SAFETY: PDH_FMT_DOUBLE selected this union member and the
                // preceding status check proves this item's formatted value valid.
                let value = unsafe { item.FmtValue.Anonymous.doubleValue };
                if !value.is_finite() || !(0.0..=100.0).contains(&value) {
                    return Err("gpu-instance-value-invalid");
                }
                percent += value;
            }
            // This sum is engine aggregation, not device-wide GPU utilization.
            Ok((percent, elapsed_ns))
        }

        pub(super) fn finish(mut self) -> Result<(), &'static str> {
            let handle = std::mem::take(&mut self.query);
            // SAFETY: this consumes the query and all its counter handles once.
            if unsafe { PdhCloseQuery(handle) } == 0 {
                Ok(())
            } else {
                Err("gpu-query-close-failed")
            }
        }
    }

    impl Drop for Query {
        fn drop(&mut self) {
            if !self.query.is_invalid() {
                // SAFETY: error paths release the exclusively owned query once.
                let _ = unsafe { PdhCloseQuery(self.query) };
            }
        }
    }
}
