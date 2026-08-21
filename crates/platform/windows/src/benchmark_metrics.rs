//! Feature-gated process metrics for the native Windows capture benchmark.
//!
//! The benchmark runs one capture profile at a time and resets these counters
//! only while no capture session or mapped frame is live. Production builds do
//! not enable `benchmark-instrumentation`; the call sites below then optimize to
//! no-ops and this module exposes no public interface.

#[cfg(feature = "benchmark-instrumentation")]
use std::time::{Duration, Instant};

#[cfg(feature = "benchmark-instrumentation")]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "benchmark-instrumentation")]
static CALLBACK_COPIES: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "benchmark-instrumentation")]
static CALLBACK_COPY_NANOS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "benchmark-instrumentation")]
static COPIED_BYTES: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "benchmark-instrumentation")]
static DETACHED_LIVE: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "benchmark-instrumentation")]
static DETACHED_PEAK: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "benchmark-instrumentation")]
static STAGING_LIVE: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "benchmark-instrumentation")]
static STAGING_PEAK: AtomicU64 = AtomicU64::new(0);

/// One process-wide observation of the Windows capture implementation.
#[cfg(feature = "benchmark-instrumentation")]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CaptureMetricsSnapshot {
    /// Successful callback-side detach copies since the last reset.
    pub callback_copies: u64,
    /// Aggregate callback-side detach-copy duration since the last reset.
    pub callback_copy_time: Duration,
    /// Bytes submitted to callback-side detach copies since the last reset.
    pub copied_bytes: u64,
    /// Detached private textures alive at the instant of the snapshot.
    pub detached_textures_live: u64,
    /// Maximum simultaneously live detached private textures since reset.
    pub detached_textures_peak: u64,
    /// Staging textures alive at the instant of the snapshot.
    pub staging_textures_live: u64,
    /// Maximum simultaneously live staging textures since reset.
    pub staging_textures_peak: u64,
}

/// Resets process metrics before a benchmark profile starts.
///
/// The caller must first prove that no capture session or mapped frame is live.
#[cfg(feature = "benchmark-instrumentation")]
pub fn reset_capture_metrics() {
    let detached = DETACHED_LIVE.load(Ordering::Acquire);
    let staging = STAGING_LIVE.load(Ordering::Acquire);
    CALLBACK_COPIES.store(0, Ordering::Release);
    CALLBACK_COPY_NANOS.store(0, Ordering::Release);
    COPIED_BYTES.store(0, Ordering::Release);
    DETACHED_PEAK.store(detached, Ordering::Release);
    STAGING_PEAK.store(staging, Ordering::Release);
}

/// Returns the current process metrics without changing them.
#[cfg(feature = "benchmark-instrumentation")]
#[must_use]
pub fn capture_metrics() -> CaptureMetricsSnapshot {
    CaptureMetricsSnapshot {
        callback_copies: CALLBACK_COPIES.load(Ordering::Acquire),
        callback_copy_time: Duration::from_nanos(CALLBACK_COPY_NANOS.load(Ordering::Acquire)),
        copied_bytes: COPIED_BYTES.load(Ordering::Acquire),
        detached_textures_live: DETACHED_LIVE.load(Ordering::Acquire),
        detached_textures_peak: DETACHED_PEAK.load(Ordering::Acquire),
        staging_textures_live: STAGING_LIVE.load(Ordering::Acquire),
        staging_textures_peak: STAGING_PEAK.load(Ordering::Acquire),
    }
}

pub(crate) fn time_callback_copy(_copied_bytes: u64) -> CallbackCopyTimer {
    CallbackCopyTimer {
        #[cfg(feature = "benchmark-instrumentation")]
        started: Instant::now(),
        #[cfg(feature = "benchmark-instrumentation")]
        copied_bytes: _copied_bytes,
    }
}

pub(crate) struct CallbackCopyTimer {
    #[cfg(feature = "benchmark-instrumentation")]
    started: Instant,
    #[cfg(feature = "benchmark-instrumentation")]
    copied_bytes: u64,
}

impl CallbackCopyTimer {
    pub(crate) fn finish(self) {
        #[cfg(feature = "benchmark-instrumentation")]
        {
            let nanos = u64::try_from(self.started.elapsed().as_nanos()).unwrap_or(u64::MAX);
            CALLBACK_COPIES.fetch_add(1, Ordering::Relaxed);
            CALLBACK_COPY_NANOS.fetch_add(nanos, Ordering::Relaxed);
            COPIED_BYTES.fetch_add(self.copied_bytes, Ordering::Relaxed);
        }
    }
}

pub(crate) fn record_detached_texture_created() {
    #[cfg(feature = "benchmark-instrumentation")]
    increment(&DETACHED_LIVE, &DETACHED_PEAK);
}

pub(crate) fn record_detached_texture_destroyed() {
    #[cfg(feature = "benchmark-instrumentation")]
    decrement(&DETACHED_LIVE);
}

pub(crate) fn staging_texture_created() -> StagingTextureGuard {
    #[cfg(feature = "benchmark-instrumentation")]
    increment(&STAGING_LIVE, &STAGING_PEAK);
    StagingTextureGuard
}

pub(crate) struct StagingTextureGuard;

impl Drop for StagingTextureGuard {
    fn drop(&mut self) {
        #[cfg(feature = "benchmark-instrumentation")]
        decrement(&STAGING_LIVE);
    }
}

#[cfg(feature = "benchmark-instrumentation")]
fn increment(live: &AtomicU64, peak: &AtomicU64) {
    let current = live.fetch_add(1, Ordering::AcqRel).saturating_add(1);
    peak.fetch_max(current, Ordering::AcqRel);
}

#[cfg(feature = "benchmark-instrumentation")]
fn decrement(live: &AtomicU64) {
    let previous = live.fetch_sub(1, Ordering::AcqRel);
    debug_assert!(previous > 0, "capture metric live count remains balanced");
}
