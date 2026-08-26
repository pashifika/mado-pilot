//! Feature-gated process metrics for the native Windows capture benchmark.
//!
//! The benchmark runs one capture profile at a time and resets these counters
//! only while no capture session or mapped frame is live. Production builds do
//! not enable `benchmark-instrumentation`; the call sites below then optimize to
//! no-ops and this module exposes no public interface.

use mado_pilot_core::FrameStamp;

#[cfg(feature = "benchmark-instrumentation")]
use std::sync::{
    LazyLock,
    atomic::{AtomicU64, Ordering},
};
#[cfg(feature = "benchmark-instrumentation")]
use std::time::{Duration, Instant};

#[cfg(feature = "benchmark-instrumentation")]
const CALLBACK_OBSERVATION_CAPACITY: usize = 64;

#[cfg(feature = "benchmark-instrumentation")]
static CALLBACK_METRICS: CallbackMetricStore<CALLBACK_OBSERVATION_CAPACITY> =
    CallbackMetricStore::new();
#[cfg(feature = "benchmark-instrumentation")]
static CALLBACK_CLOCK_ORIGIN: LazyLock<Instant> = LazyLock::new(Instant::now);
#[cfg(feature = "benchmark-instrumentation")]
static DETACHED_LIVE: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "benchmark-instrumentation")]
static DETACHED_PEAK: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "benchmark-instrumentation")]
static STAGING_LIVE: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "benchmark-instrumentation")]
static STAGING_PEAK: AtomicU64 = AtomicU64::new(0);

/// One process-wide resource observation of the Windows capture implementation.
#[cfg(feature = "benchmark-instrumentation")]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CaptureMetricsSnapshot {
    /// Callback observations lost to contention or overwrite.
    pub callback_observation_losses: u64,
    /// Detached private textures alive at the instant of the snapshot.
    pub detached_textures_live: u64,
    /// Maximum simultaneously live detached private textures since reset.
    pub detached_textures_peak: u64,
    /// Staging textures alive at the instant of the snapshot.
    pub staging_textures_live: u64,
    /// Maximum simultaneously live staging textures since reset.
    pub staging_textures_peak: u64,
}

/// A point after which a benchmark requires callback-copy evidence.
#[cfg(feature = "benchmark-instrumentation")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallbackMetricBaseline {
    completion_cursor: u64,
    binding_cursor: u64,
    completion_losses: u64,
    binding_losses: u64,
    at_nanos: u64,
}

/// One coherently published completed callback detach-copy operation.
#[cfg(feature = "benchmark-instrumentation")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallbackCopyObservation {
    /// The complete callback-side detach-copy duration for the acquired frame.
    pub callback_copy_time: Duration,
    /// Bytes copied for the acquired frame's exact callback record.
    pub copied_bytes: u64,
}

/// Why callback instrumentation cannot be used as benchmark evidence.
#[cfg(feature = "benchmark-instrumentation")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackObservationError {
    /// A callback record was contended, missing, changed, or overwritten.
    Invalidated,
}

#[cfg(feature = "benchmark-instrumentation")]
struct CallbackMetricStore<const N: usize> {
    completions: CompletionRing<N>,
    bindings: BindingRing<N>,
}

#[cfg(feature = "benchmark-instrumentation")]
struct CompletionRing<const N: usize> {
    next: AtomicU64,
    losses: AtomicU64,
    slots: [CompletionSlot; N],
}

#[cfg(feature = "benchmark-instrumentation")]
struct BindingRing<const N: usize> {
    next: AtomicU64,
    losses: AtomicU64,
    slots: [BindingSlot; N],
}

#[cfg(feature = "benchmark-instrumentation")]
struct CompletionSlot {
    sequence: AtomicU64,
    stream: AtomicU64,
    completed_at_nanos: AtomicU64,
    elapsed_nanos: AtomicU64,
    copied_bytes: AtomicU64,
}

#[cfg(feature = "benchmark-instrumentation")]
struct BindingSlot {
    sequence: AtomicU64,
    completion_cursor: AtomicU64,
    stream: AtomicU64,
    epoch: AtomicU64,
    frame_sequence: AtomicU64,
}

#[cfg(feature = "benchmark-instrumentation")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CompletionId {
    cursor: u64,
    stream: u64,
}

#[cfg(feature = "benchmark-instrumentation")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CompletionRecord {
    stream: u64,
    completed_at_nanos: u64,
    elapsed_nanos: u64,
    copied_bytes: u64,
}

#[cfg(feature = "benchmark-instrumentation")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BindingRecord {
    completion_cursor: u64,
    stream: u64,
    epoch: u64,
    frame_sequence: u64,
}

#[cfg(feature = "benchmark-instrumentation")]
#[derive(Clone, Copy)]
struct Reservation {
    cursor: u64,
}

#[cfg(feature = "benchmark-instrumentation")]
impl CompletionSlot {
    const fn new() -> Self {
        Self {
            sequence: AtomicU64::new(0),
            stream: AtomicU64::new(0),
            completed_at_nanos: AtomicU64::new(0),
            elapsed_nanos: AtomicU64::new(0),
            copied_bytes: AtomicU64::new(0),
        }
    }
}

#[cfg(feature = "benchmark-instrumentation")]
impl BindingSlot {
    const fn new() -> Self {
        Self {
            sequence: AtomicU64::new(0),
            completion_cursor: AtomicU64::new(0),
            stream: AtomicU64::new(0),
            epoch: AtomicU64::new(0),
            frame_sequence: AtomicU64::new(0),
        }
    }
}

#[cfg(feature = "benchmark-instrumentation")]
fn next_reservation<const N: usize>(
    next: &AtomicU64,
    losses: &AtomicU64,
) -> Option<(Reservation, usize, u64, u64)> {
    let previous = next.fetch_add(1, Ordering::Relaxed);
    let Some(cursor) = previous.checked_add(1) else {
        losses.fetch_add(1, Ordering::Release);
        return None;
    };
    let Some(published) = cursor.checked_mul(2) else {
        losses.fetch_add(1, Ordering::Release);
        return None;
    };
    let capacity = u64::try_from(N).expect("callback observation capacity fits u64");
    let previous_cursor = cursor.saturating_sub(capacity);
    let Some(expected) = previous_cursor.checked_mul(2) else {
        losses.fetch_add(1, Ordering::Release);
        return None;
    };
    let index =
        usize::try_from((cursor - 1) % capacity).expect("callback observation index fits usize");
    Some((Reservation { cursor }, index, expected, published - 1))
}

#[cfg(feature = "benchmark-instrumentation")]
impl<const N: usize> CompletionRing<N> {
    const fn new() -> Self {
        Self {
            next: AtomicU64::new(0),
            losses: AtomicU64::new(0),
            slots: [const { CompletionSlot::new() }; N],
        }
    }

    fn reset(&self) {
        self.next.store(0, Ordering::Release);
        self.losses.store(0, Ordering::Release);
        for slot in &self.slots {
            slot.sequence.store(0, Ordering::Release);
            slot.stream.store(0, Ordering::Relaxed);
            slot.completed_at_nanos.store(0, Ordering::Relaxed);
            slot.elapsed_nanos.store(0, Ordering::Relaxed);
            slot.copied_bytes.store(0, Ordering::Relaxed);
        }
    }

    fn reserve(&self) -> Option<Reservation> {
        let (reservation, index, expected, writing) =
            next_reservation::<N>(&self.next, &self.losses)?;
        if self.slots[index]
            .sequence
            .compare_exchange(expected, writing, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            self.losses.fetch_add(1, Ordering::Release);
            return None;
        }
        Some(reservation)
    }

    fn publish(&self, reservation: Reservation, record: CompletionRecord) -> CompletionId {
        let capacity = u64::try_from(N).expect("callback observation capacity fits u64");
        let index = usize::try_from((reservation.cursor - 1) % capacity)
            .expect("callback index fits usize");
        let slot = &self.slots[index];
        slot.stream.store(record.stream, Ordering::Relaxed);
        slot.completed_at_nanos
            .store(record.completed_at_nanos, Ordering::Relaxed);
        slot.elapsed_nanos
            .store(record.elapsed_nanos, Ordering::Relaxed);
        slot.copied_bytes
            .store(record.copied_bytes, Ordering::Relaxed);
        slot.sequence
            .store(reservation.cursor * 2, Ordering::Release);
        CompletionId {
            cursor: reservation.cursor,
            stream: record.stream,
        }
    }

    fn record(&self, record: CompletionRecord) -> Option<CompletionId> {
        let reservation = self.reserve()?;
        Some(self.publish(reservation, record))
    }

    fn read_slot(
        slot: &CompletionSlot,
    ) -> Result<Option<(u64, CompletionRecord)>, CallbackObservationError> {
        let before = slot.sequence.load(Ordering::Acquire);
        if before == 0 {
            return Ok(None);
        }
        if before % 2 == 1 {
            return Err(CallbackObservationError::Invalidated);
        }
        let record = CompletionRecord {
            stream: slot.stream.load(Ordering::Relaxed),
            completed_at_nanos: slot.completed_at_nanos.load(Ordering::Relaxed),
            elapsed_nanos: slot.elapsed_nanos.load(Ordering::Relaxed),
            copied_bytes: slot.copied_bytes.load(Ordering::Relaxed),
        };
        if slot.sequence.load(Ordering::Acquire) != before {
            return Err(CallbackObservationError::Invalidated);
        }
        Ok(Some((before / 2, record)))
    }

    fn read(&self, cursor: u64) -> Result<Option<CompletionRecord>, CallbackObservationError> {
        if cursor == 0 {
            return Ok(None);
        }
        let capacity = u64::try_from(N).expect("callback observation capacity fits u64");
        let index = usize::try_from((cursor - 1) % capacity).expect("callback index fits usize");
        let Some((observed_cursor, record)) = Self::read_slot(&self.slots[index])? else {
            return Ok(None);
        };
        if observed_cursor != cursor {
            return Err(CallbackObservationError::Invalidated);
        }
        Ok(Some(record))
    }

    fn copied_bytes_between(
        &self,
        start: CallbackMetricBaseline,
        end: CallbackMetricBaseline,
        streams: &[u64],
    ) -> Result<u64, CallbackObservationError> {
        let capacity = u64::try_from(N).expect("callback observation capacity fits u64");
        if start.completion_losses != end.completion_losses
            || self.losses.load(Ordering::Acquire) != start.completion_losses
            || end.completion_cursor < start.completion_cursor
            || end
                .completion_cursor
                .saturating_sub(start.completion_cursor)
                > capacity
            || end.at_nanos < start.at_nanos
        {
            return Err(CallbackObservationError::Invalidated);
        }
        let mut copied_bytes = 0_u64;
        for slot in &self.slots {
            let Some((_cursor, record)) = Self::read_slot(slot)? else {
                continue;
            };
            if !streams.contains(&record.stream)
                || record.completed_at_nanos <= start.at_nanos
                || record.completed_at_nanos > end.at_nanos
            {
                continue;
            }
            copied_bytes = copied_bytes
                .checked_add(record.copied_bytes)
                .ok_or(CallbackObservationError::Invalidated)?;
        }
        if self.losses.load(Ordering::Acquire) != start.completion_losses
            || self
                .next
                .load(Ordering::Acquire)
                .saturating_sub(start.completion_cursor)
                > capacity
        {
            return Err(CallbackObservationError::Invalidated);
        }
        Ok(copied_bytes)
    }
}

#[cfg(feature = "benchmark-instrumentation")]
impl<const N: usize> BindingRing<N> {
    const fn new() -> Self {
        Self {
            next: AtomicU64::new(0),
            losses: AtomicU64::new(0),
            slots: [const { BindingSlot::new() }; N],
        }
    }

    fn reset(&self) {
        self.next.store(0, Ordering::Release);
        self.losses.store(0, Ordering::Release);
        for slot in &self.slots {
            slot.sequence.store(0, Ordering::Release);
            slot.completion_cursor.store(0, Ordering::Relaxed);
            slot.stream.store(0, Ordering::Relaxed);
            slot.epoch.store(0, Ordering::Relaxed);
            slot.frame_sequence.store(0, Ordering::Relaxed);
        }
    }

    fn reserve(&self) -> Option<Reservation> {
        let (reservation, index, expected, writing) =
            next_reservation::<N>(&self.next, &self.losses)?;
        if self.slots[index]
            .sequence
            .compare_exchange(expected, writing, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            self.losses.fetch_add(1, Ordering::Release);
            return None;
        }
        Some(reservation)
    }

    fn record(&self, record: BindingRecord) {
        let Some(reservation) = self.reserve() else {
            return;
        };
        let capacity = u64::try_from(N).expect("callback observation capacity fits u64");
        let index = usize::try_from((reservation.cursor - 1) % capacity)
            .expect("callback index fits usize");
        let slot = &self.slots[index];
        slot.completion_cursor
            .store(record.completion_cursor, Ordering::Relaxed);
        slot.stream.store(record.stream, Ordering::Relaxed);
        slot.epoch.store(record.epoch, Ordering::Relaxed);
        slot.frame_sequence
            .store(record.frame_sequence, Ordering::Relaxed);
        slot.sequence
            .store(reservation.cursor * 2, Ordering::Release);
    }

    fn read_slot(slot: &BindingSlot) -> Result<Option<BindingRecord>, CallbackObservationError> {
        let before = slot.sequence.load(Ordering::Acquire);
        if before == 0 || before % 2 == 1 {
            return Ok(None);
        }
        let record = BindingRecord {
            completion_cursor: slot.completion_cursor.load(Ordering::Relaxed),
            stream: slot.stream.load(Ordering::Relaxed),
            epoch: slot.epoch.load(Ordering::Relaxed),
            frame_sequence: slot.frame_sequence.load(Ordering::Relaxed),
        };
        if slot.sequence.load(Ordering::Acquire) != before {
            return Err(CallbackObservationError::Invalidated);
        }
        Ok(Some(record))
    }
}

#[cfg(feature = "benchmark-instrumentation")]
impl<const N: usize> CallbackMetricStore<N> {
    const fn new() -> Self {
        Self {
            completions: CompletionRing::new(),
            bindings: BindingRing::new(),
        }
    }

    fn reset(&self) {
        self.completions.reset();
        self.bindings.reset();
    }

    fn baseline(&self) -> CallbackMetricBaseline {
        CallbackMetricBaseline {
            completion_cursor: self.completions.next.load(Ordering::Acquire),
            binding_cursor: self.bindings.next.load(Ordering::Acquire),
            completion_losses: self.completions.losses.load(Ordering::Acquire),
            binding_losses: self.bindings.losses.load(Ordering::Acquire),
            at_nanos: callback_now_nanos(),
        }
    }

    fn losses(&self) -> u64 {
        self.completions
            .losses
            .load(Ordering::Acquire)
            .saturating_add(self.bindings.losses.load(Ordering::Acquire))
    }

    fn complete(&self, record: CompletionRecord) -> Option<CompletionId> {
        self.completions.record(record)
    }

    fn bind(&self, completion: CompletionId, stamp: FrameStamp) {
        if completion.stream != stamp.stream().get() {
            self.bindings.losses.fetch_add(1, Ordering::Release);
            return;
        }
        self.bindings.record(BindingRecord {
            completion_cursor: completion.cursor,
            stream: completion.stream,
            epoch: stamp.epoch().value(),
            frame_sequence: stamp.sequence().value(),
        });
    }

    fn validate(&self, baseline: CallbackMetricBaseline) -> Result<(), CallbackObservationError> {
        let capacity = u64::try_from(N).expect("callback observation capacity fits u64");
        if self.completions.losses.load(Ordering::Acquire) != baseline.completion_losses
            || self.bindings.losses.load(Ordering::Acquire) != baseline.binding_losses
            || self
                .completions
                .next
                .load(Ordering::Acquire)
                .saturating_sub(baseline.completion_cursor)
                > capacity
            || self
                .bindings
                .next
                .load(Ordering::Acquire)
                .saturating_sub(baseline.binding_cursor)
                > capacity
        {
            return Err(CallbackObservationError::Invalidated);
        }
        Ok(())
    }

    fn observation_after(
        &self,
        baseline: CallbackMetricBaseline,
        stream: u64,
        epoch: u64,
        frame_sequence: u64,
    ) -> Result<Option<CallbackCopyObservation>, CallbackObservationError> {
        self.validate(baseline)?;
        let mut binding = None;
        for slot in &self.bindings.slots {
            let Some(record) = BindingRing::<N>::read_slot(slot)? else {
                continue;
            };
            if record.stream == stream
                && record.epoch == epoch
                && record.frame_sequence == frame_sequence
            {
                binding = Some(record);
                break;
            }
        }
        let Some(binding) = binding else {
            return Ok(None);
        };
        let Some(completion) = self.completions.read(binding.completion_cursor)? else {
            return Err(CallbackObservationError::Invalidated);
        };
        if completion.stream != stream {
            return Err(CallbackObservationError::Invalidated);
        }
        if completion.completed_at_nanos <= baseline.at_nanos {
            return Ok(None);
        }
        self.validate(baseline)?;
        Ok(Some(CallbackCopyObservation {
            callback_copy_time: Duration::from_nanos(completion.elapsed_nanos),
            copied_bytes: completion.copied_bytes,
        }))
    }

    fn copied_bytes_between(
        &self,
        start: CallbackMetricBaseline,
        end: CallbackMetricBaseline,
        streams: &[u64],
    ) -> Result<u64, CallbackObservationError> {
        if start.binding_losses != end.binding_losses
            || self.bindings.losses.load(Ordering::Acquire) != start.binding_losses
        {
            return Err(CallbackObservationError::Invalidated);
        }
        self.completions.copied_bytes_between(start, end, streams)
    }
}

/// Returns the bounded signed-X fixture placement for one moving-seam sample.
#[cfg(feature = "benchmark-instrumentation")]
#[must_use]
pub fn dual_display_seam_x(index: u32) -> i32 {
    const LEFT: i32 = -960;
    const STEP: i32 = 16;
    const HALF_PERIOD: u32 = 40;
    let phase = index % (HALF_PERIOD * 2);
    let step = if phase <= HALF_PERIOD {
        phase
    } else {
        HALF_PERIOD * 2 - phase
    };
    LEFT + i32::try_from(step).expect("bounded seam step fits i32") * STEP
}

/// Returns the placement-marker center on each side of the moving fixture.
#[cfg(feature = "benchmark-instrumentation")]
#[must_use]
pub fn dual_display_fixture_marker_points(window_x: i32, window_y: i32) -> [(f64, f64); 2] {
    use crate::fixture_protocol::{
        BENCHMARK_LEFT_MARKER_X, BENCHMARK_MARKER_SIZE, BENCHMARK_MARKER_Y,
        BENCHMARK_RIGHT_MARKER_X,
    };

    const FIXTURE_WIDTH: i32 = 1_280;
    assert!(
        window_x < 0 && window_x + FIXTURE_WIDTH > 0,
        "the moving fixture must straddle the signed desktop seam"
    );
    assert!(
        window_x + BENCHMARK_LEFT_MARKER_X + BENCHMARK_MARKER_SIZE <= 0
            && window_x + BENCHMARK_RIGHT_MARKER_X >= 0,
        "each moving fixture marker must remain on its declared display"
    );
    let center = BENCHMARK_MARKER_SIZE / 2;
    let vertical = f64::from(window_y + BENCHMARK_MARKER_Y + center);
    [
        (
            f64::from(window_x + BENCHMARK_LEFT_MARKER_X + center),
            vertical,
        ),
        (
            f64::from(window_x + BENCHMARK_RIGHT_MARKER_X + center),
            vertical,
        ),
    ]
}

/// Returns whether one captured BGRA pixel carries the benchmark marker color.
#[cfg(feature = "benchmark-instrumentation")]
#[must_use]
pub fn benchmark_marker_pixel_matches(observed_bgr: &[u8]) -> bool {
    use crate::fixture_protocol::{BENCHMARK_MARKER_RGB, FILL_TOLERANCE};

    let expected = [
        (BENCHMARK_MARKER_RGB & 0xff) as u8,
        ((BENCHMARK_MARKER_RGB >> 8) & 0xff) as u8,
        ((BENCHMARK_MARKER_RGB >> 16) & 0xff) as u8,
    ];
    observed_bgr.get(..3).is_some_and(|observed| {
        observed
            .iter()
            .zip(expected)
            .all(|(actual, wanted)| actual.abs_diff(wanted) <= FILL_TOLERANCE)
    })
}

/// Resets process metrics before a benchmark profile starts.
///
/// The caller must first prove that no capture session or mapped frame is live.
#[cfg(feature = "benchmark-instrumentation")]
pub fn reset_capture_metrics() {
    let detached = DETACHED_LIVE.load(Ordering::Acquire);
    let staging = STAGING_LIVE.load(Ordering::Acquire);
    CALLBACK_METRICS.reset();
    DETACHED_PEAK.store(detached, Ordering::Release);
    STAGING_PEAK.store(staging, Ordering::Release);
}

/// Returns the current process resource metrics without changing them.
#[cfg(feature = "benchmark-instrumentation")]
#[must_use]
pub fn capture_metrics() -> CaptureMetricsSnapshot {
    CaptureMetricsSnapshot {
        callback_observation_losses: CALLBACK_METRICS.losses(),
        detached_textures_live: DETACHED_LIVE.load(Ordering::Acquire),
        detached_textures_peak: DETACHED_PEAK.load(Ordering::Acquire),
        staging_textures_live: STAGING_LIVE.load(Ordering::Acquire),
        staging_textures_peak: STAGING_PEAK.load(Ordering::Acquire),
    }
}

/// Captures a callback observation baseline before requesting a frame.
#[cfg(feature = "benchmark-instrumentation")]
#[must_use]
pub fn callback_metric_baseline() -> CallbackMetricBaseline {
    CALLBACK_METRICS.baseline()
}

/// Finds the coherent callback record for one acquired frame after `baseline`.
#[cfg(feature = "benchmark-instrumentation")]
pub fn callback_observation_after(
    baseline: CallbackMetricBaseline,
    stamp: FrameStamp,
) -> Result<Option<CallbackCopyObservation>, CallbackObservationError> {
    CALLBACK_METRICS.observation_after(
        baseline,
        stamp.stream().get(),
        stamp.epoch().value(),
        stamp.sequence().value(),
    )
}

/// Sums coherent callback-copy bytes for selected streams over one sample.
#[cfg(feature = "benchmark-instrumentation")]
pub fn callback_copied_bytes_between(
    start: CallbackMetricBaseline,
    end: CallbackMetricBaseline,
    streams: &[u64],
) -> Result<u64, CallbackObservationError> {
    CALLBACK_METRICS.copied_bytes_between(start, end, streams)
}

pub(crate) fn time_callback_copy(_copied_bytes: u64, _stream: u64) -> CallbackCopyTimer {
    CallbackCopyTimer {
        #[cfg(feature = "benchmark-instrumentation")]
        started: Instant::now(),
        #[cfg(feature = "benchmark-instrumentation")]
        copied_bytes: _copied_bytes,
        #[cfg(feature = "benchmark-instrumentation")]
        stream: _stream,
    }
}

pub(crate) struct CallbackCopyTimer {
    #[cfg(feature = "benchmark-instrumentation")]
    started: Instant,
    #[cfg(feature = "benchmark-instrumentation")]
    copied_bytes: u64,
    #[cfg(feature = "benchmark-instrumentation")]
    stream: u64,
}

pub(crate) struct CompletedCallbackCopy {
    #[cfg(feature = "benchmark-instrumentation")]
    completion: Option<CompletionId>,
}

impl CallbackCopyTimer {
    pub(crate) fn finish(self) -> CompletedCallbackCopy {
        #[cfg(feature = "benchmark-instrumentation")]
        {
            let elapsed_nanos =
                u64::try_from(self.started.elapsed().as_nanos()).unwrap_or(u64::MAX);
            CompletedCallbackCopy {
                completion: CALLBACK_METRICS.complete(CompletionRecord {
                    stream: self.stream,
                    completed_at_nanos: callback_now_nanos(),
                    elapsed_nanos,
                    copied_bytes: self.copied_bytes,
                }),
            }
        }
        #[cfg(not(feature = "benchmark-instrumentation"))]
        {
            CompletedCallbackCopy {}
        }
    }
}

impl CompletedCallbackCopy {
    pub(crate) fn publish(self, stamp: FrameStamp) {
        #[cfg(not(feature = "benchmark-instrumentation"))]
        let _ = stamp;
        #[cfg(feature = "benchmark-instrumentation")]
        if let Some(completion) = self.completion {
            CALLBACK_METRICS.bind(completion, stamp);
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

#[cfg(feature = "benchmark-instrumentation")]
fn callback_now_nanos() -> u64 {
    u64::try_from(CALLBACK_CLOCK_ORIGIN.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(all(test, feature = "benchmark-instrumentation"))]
mod tests {
    use super::*;

    fn baseline_at<const N: usize>(
        store: &CallbackMetricStore<N>,
        at_nanos: u64,
    ) -> CallbackMetricBaseline {
        let mut baseline = store.baseline();
        baseline.at_nanos = at_nanos;
        baseline
    }

    fn complete<const N: usize>(
        store: &CallbackMetricStore<N>,
        stream: u64,
        completed_at_nanos: u64,
        elapsed_nanos: u64,
        copied_bytes: u64,
    ) -> CompletionId {
        store
            .complete(CompletionRecord {
                stream,
                completed_at_nanos,
                elapsed_nanos,
                copied_bytes,
            })
            .expect("the local completion ring has capacity")
    }

    fn bind<const N: usize>(
        store: &CallbackMetricStore<N>,
        completion: CompletionId,
        epoch: u64,
        frame_sequence: u64,
    ) {
        store.bindings.record(BindingRecord {
            completion_cursor: completion.cursor,
            stream: completion.stream,
            epoch,
            frame_sequence,
        });
    }

    #[test]
    fn a_completion_is_not_accepted_until_its_frame_binding_is_coherent() {
        let store = CallbackMetricStore::<4>::new();
        let baseline = baseline_at(&store, 0);
        let completion = complete(&store, 7, 1, 17, 19);
        assert_eq!(store.observation_after(baseline, 7, 11, 13), Ok(None));

        bind(&store, completion, 11, 13);
        assert_eq!(
            store.observation_after(baseline, 7, 11, 13),
            Ok(Some(CallbackCopyObservation {
                callback_copy_time: Duration::from_nanos(17),
                copied_bytes: 19,
            }))
        );
    }

    #[test]
    fn completion_before_baseline_bound_afterward_remains_ineligible() {
        let store = CallbackMetricStore::<4>::new();
        let completion = complete(&store, 3, 1, 11, 13);
        let baseline = baseline_at(&store, 2);
        bind(&store, completion, 5, 7);

        assert_eq!(store.observation_after(baseline, 3, 5, 7), Ok(None));
    }

    #[test]
    fn another_session_binding_cannot_satisfy_the_requested_frame() {
        let store = CallbackMetricStore::<4>::new();
        let baseline = baseline_at(&store, 0);
        let completion = complete(&store, 1, 1, 30, 40);
        bind(&store, completion, 10, 20);

        assert_eq!(store.observation_after(baseline, 2, 10, 20), Ok(None));
    }

    #[test]
    fn out_of_order_bindings_remain_associated_with_their_completions() {
        let store = CallbackMetricStore::<4>::new();
        let baseline = baseline_at(&store, 0);
        let first = complete(&store, 1, 1, 11, 111);
        let second = complete(&store, 2, 2, 22, 222);
        bind(&store, second, 20, 200);
        bind(&store, first, 10, 100);

        assert_eq!(
            store.observation_after(baseline, 1, 10, 100),
            Ok(Some(CallbackCopyObservation {
                callback_copy_time: Duration::from_nanos(11),
                copied_bytes: 111,
            }))
        );
        assert_eq!(
            store.observation_after(baseline, 2, 20, 200),
            Ok(Some(CallbackCopyObservation {
                callback_copy_time: Duration::from_nanos(22),
                copied_bytes: 222,
            }))
        );
    }

    #[test]
    fn sample_totals_include_trailing_selected_stream_completions() {
        let store = CallbackMetricStore::<8>::new();
        let start = baseline_at(&store, 0);
        let first = complete(&store, 1, 1, 11, 100);
        let target = complete(&store, 1, 2, 22, 200);
        let other = complete(&store, 2, 3, 33, 999);
        bind(&store, target, 10, 101);
        let _trailing = complete(&store, 1, 4, 44, 300);
        bind(&store, first, 10, 100);
        bind(&store, other, 20, 200);
        let end = baseline_at(&store, 5);

        assert_eq!(
            store.observation_after(start, 1, 10, 101),
            Ok(Some(CallbackCopyObservation {
                callback_copy_time: Duration::from_nanos(22),
                copied_bytes: 200,
            }))
        );
        assert_eq!(store.copied_bytes_between(start, end, &[1]), Ok(600));
        assert_eq!(store.copied_bytes_between(start, end, &[2]), Ok(999));
    }

    #[test]
    fn sample_totals_exclude_prestart_and_postend_completions() {
        let store = CallbackMetricStore::<4>::new();
        let _prestart = complete(&store, 1, 1, 11, 50);
        let start = baseline_at(&store, 2);
        let _inside = complete(&store, 1, 3, 22, 100);
        let end = baseline_at(&store, 4);
        let _postend = complete(&store, 1, 5, 33, 200);

        assert_eq!(store.copied_bytes_between(start, end, &[1]), Ok(100));
    }

    #[test]
    fn in_progress_completion_invalidates_the_sample_total() {
        let store = CallbackMetricStore::<2>::new();
        let start = baseline_at(&store, 0);
        let reservation = store
            .completions
            .reserve()
            .expect("one completion reservation succeeds");
        let end = baseline_at(&store, 1);

        assert_eq!(
            store.copied_bytes_between(start, end, &[1]),
            Err(CallbackObservationError::Invalidated)
        );

        store.completions.publish(
            reservation,
            CompletionRecord {
                stream: 1,
                completed_at_nanos: 1,
                elapsed_nanos: 30,
                copied_bytes: 40,
            },
        );
    }

    #[test]
    fn contended_completion_instrumentation_invalidates_the_baseline() {
        let store = CallbackMetricStore::<1>::new();
        let baseline = baseline_at(&store, 0);
        let reservation = store
            .completions
            .reserve()
            .expect("first reservation succeeds");
        assert!(store.completions.reserve().is_none());
        store.completions.publish(
            reservation,
            CompletionRecord {
                stream: 1,
                completed_at_nanos: 1,
                elapsed_nanos: 30,
                copied_bytes: 40,
            },
        );
        assert_eq!(
            store.observation_after(baseline, 1, 10, 20),
            Err(CallbackObservationError::Invalidated)
        );
    }

    #[test]
    fn the_three_hundred_frame_schedule_moves_and_always_straddles_the_seam() {
        let positions = (0..300).map(dual_display_seam_x).collect::<Vec<_>>();

        assert!(positions.windows(2).all(|pair| pair[0] != pair[1]));
        assert!(positions.iter().all(|x| *x < 0 && *x + 1_280 > 0));
        assert!(positions.contains(&-960));
        assert!(positions.contains(&-320));
    }

    #[test]
    fn moving_fixture_marker_points_stay_inside_opposite_display_halves() {
        for index in 0..300 {
            let x = dual_display_seam_x(index);
            let [left, right] = dual_display_fixture_marker_points(x, 600);
            assert!(f64::from(x) < left.0 && left.0 < 0.0);
            assert!(0.0 < right.0 && right.0 < f64::from(x + 1_280));
            assert_eq!(left.1, 960.0);
            assert_eq!(right.1, 960.0);
        }
    }

    #[test]
    fn strictly_newer_frame_with_prior_fixture_placement_is_rejected() {
        use crate::fixture_protocol::{BENCHMARK_MARKER_RGB, FILL_RGB};

        const WINDOW_Y: i32 = 600;
        const MARKER_CLIENT_X: [i32; 2] = [64, 1_200];

        struct SyntheticFrame {
            sequence: u64,
            sampled_bgr: [u8; 3],
        }
        const MARKER_CLIENT_Y: i32 = 352;
        const MARKER_SIZE: i32 = 16;

        fn contains_marker(window_x: i32, window_y: i32, marker_x: i32, point: (f64, f64)) -> bool {
            let left = f64::from(window_x + marker_x);
            let top = f64::from(window_y + MARKER_CLIENT_Y);
            point.0 >= left
                && point.0 < left + f64::from(MARKER_SIZE)
                && point.1 >= top
                && point.1 < top + f64::from(MARKER_SIZE)
        }

        fn captured_bgr(rendered_x: i32, marker_x: i32, requested_point: (f64, f64)) -> [u8; 3] {
            let rgb = if contains_marker(rendered_x, WINDOW_Y, marker_x, requested_point) {
                BENCHMARK_MARKER_RGB
            } else {
                FILL_RGB
            };
            [
                (rgb & 0xff) as u8,
                ((rgb >> 8) & 0xff) as u8,
                ((rgb >> 16) & 0xff) as u8,
            ]
        }

        let positions = (0..300).map(dual_display_seam_x).collect::<Vec<_>>();
        for (sequence, pair) in positions.windows(2).enumerate() {
            let prior_x = pair[0];
            let requested_x = pair[1];
            let points = dual_display_fixture_marker_points(requested_x, WINDOW_Y);
            for (marker_x, point) in MARKER_CLIENT_X.into_iter().zip(points) {
                let requested_frame = SyntheticFrame {
                    sequence: u64::try_from(sequence).expect("bounded schedule index fits u64"),
                    sampled_bgr: captured_bgr(requested_x, marker_x, point),
                };
                let newer_prior_placement = SyntheticFrame {
                    sequence: requested_frame.sequence + 1,
                    sampled_bgr: captured_bgr(prior_x, marker_x, point),
                };
                assert!(benchmark_marker_pixel_matches(&requested_frame.sampled_bgr));
                assert!(newer_prior_placement.sequence > requested_frame.sequence);
                assert!(!benchmark_marker_pixel_matches(
                    &newer_prior_placement.sampled_bgr
                ));
            }
        }
    }
}
