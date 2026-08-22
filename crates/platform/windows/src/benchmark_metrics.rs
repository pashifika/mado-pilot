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
use std::time::Duration;
#[cfg(feature = "benchmark-instrumentation")]
use std::time::Instant;

#[cfg(feature = "benchmark-instrumentation")]
const CALLBACK_OBSERVATION_CAPACITY: usize = 64;

#[cfg(feature = "benchmark-instrumentation")]
static CALLBACK_OBSERVATIONS: CallbackMetricRing<CALLBACK_OBSERVATION_CAPACITY> =
    CallbackMetricRing::new();
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
    /// Callback observations lost to contention or an incomplete callback.
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

/// A point after which a benchmark requires a callback observation.
#[cfg(feature = "benchmark-instrumentation")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallbackMetricBaseline {
    cursor: u64,
    losses: u64,
    completed_at_nanos: u64,
}

/// One coherently published completed callback detach-copy operation.
#[cfg(feature = "benchmark-instrumentation")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallbackCopyObservation {
    /// The complete callback-side detach-copy duration.
    pub callback_copy_time: Duration,
    /// Bytes submitted by this callback-side detach copy.
    pub copied_bytes: u64,
    /// Same-session bytes copied after the baseline through this frame.
    pub interval_copied_bytes: u64,
}

/// Why a callback observation cannot be used as benchmark evidence.
#[cfg(feature = "benchmark-instrumentation")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackObservationError {
    /// A callback record was contended, abandoned, or overwritten.
    Invalidated,
}

#[cfg(feature = "benchmark-instrumentation")]
struct CallbackMetricRing<const N: usize> {
    next: AtomicU64,
    losses: AtomicU64,
    slots: [CallbackMetricSlot; N],
}

#[cfg(feature = "benchmark-instrumentation")]
struct CallbackMetricSlot {
    sequence: AtomicU64,
    stream: AtomicU64,
    epoch: AtomicU64,
    frame_sequence: AtomicU64,
    completed_at_nanos: AtomicU64,
    elapsed_nanos: AtomicU64,
    copied_bytes: AtomicU64,
}

#[cfg(feature = "benchmark-instrumentation")]
#[derive(Clone, Copy)]
struct CallbackReservation {
    cursor: u64,
}

#[cfg(feature = "benchmark-instrumentation")]
#[derive(Clone, Copy)]
struct CallbackRecord {
    stream: u64,
    epoch: u64,
    frame_sequence: u64,
    completed_at_nanos: u64,
    elapsed_nanos: u64,
    copied_bytes: u64,
}

#[cfg(feature = "benchmark-instrumentation")]
impl CallbackMetricSlot {
    const fn new() -> Self {
        Self {
            sequence: AtomicU64::new(0),
            stream: AtomicU64::new(0),
            epoch: AtomicU64::new(0),
            frame_sequence: AtomicU64::new(0),
            completed_at_nanos: AtomicU64::new(0),
            elapsed_nanos: AtomicU64::new(0),
            copied_bytes: AtomicU64::new(0),
        }
    }
}

#[cfg(feature = "benchmark-instrumentation")]
impl<const N: usize> CallbackMetricRing<N> {
    const fn new() -> Self {
        Self {
            next: AtomicU64::new(0),
            losses: AtomicU64::new(0),
            slots: [const { CallbackMetricSlot::new() }; N],
        }
    }

    fn reset(&self) {
        self.next.store(0, Ordering::Release);
        self.losses.store(0, Ordering::Release);
        for slot in &self.slots {
            slot.sequence.store(0, Ordering::Release);
            slot.stream.store(0, Ordering::Relaxed);
            slot.epoch.store(0, Ordering::Relaxed);
            slot.frame_sequence.store(0, Ordering::Relaxed);
            slot.completed_at_nanos.store(0, Ordering::Relaxed);
            slot.elapsed_nanos.store(0, Ordering::Relaxed);
            slot.copied_bytes.store(0, Ordering::Relaxed);
        }
    }

    fn baseline(&self) -> CallbackMetricBaseline {
        CallbackMetricBaseline {
            cursor: self.next.load(Ordering::Acquire),
            losses: self.losses.load(Ordering::Acquire),
            completed_at_nanos: callback_now_nanos(),
        }
    }

    fn reserve(&self) -> Option<CallbackReservation> {
        let previous = self.next.fetch_add(1, Ordering::Relaxed);
        let Some(cursor) = previous.checked_add(1) else {
            self.losses.fetch_add(1, Ordering::Release);
            return None;
        };
        let Some(published) = cursor.checked_mul(2) else {
            self.losses.fetch_add(1, Ordering::Release);
            return None;
        };
        let capacity = u64::try_from(N).expect("callback observation capacity fits u64");
        let previous_cursor = cursor.saturating_sub(capacity);
        let Some(expected) = previous_cursor.checked_mul(2) else {
            self.losses.fetch_add(1, Ordering::Release);
            return None;
        };
        let index = usize::try_from((cursor - 1) % capacity)
            .expect("callback observation index fits usize");
        if self.slots[index]
            .sequence
            .compare_exchange(expected, published - 1, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            self.losses.fetch_add(1, Ordering::Release);
            return None;
        }
        Some(CallbackReservation { cursor })
    }

    fn publish(&self, reservation: CallbackReservation, record: CallbackRecord) {
        let capacity = u64::try_from(N).expect("callback observation capacity fits u64");
        let index = usize::try_from((reservation.cursor - 1) % capacity)
            .expect("callback observation index fits usize");
        let slot = &self.slots[index];
        slot.stream.store(record.stream, Ordering::Relaxed);
        slot.epoch.store(record.epoch, Ordering::Relaxed);
        slot.frame_sequence
            .store(record.frame_sequence, Ordering::Relaxed);
        slot.completed_at_nanos
            .store(record.completed_at_nanos, Ordering::Relaxed);
        slot.elapsed_nanos
            .store(record.elapsed_nanos, Ordering::Relaxed);
        slot.copied_bytes
            .store(record.copied_bytes, Ordering::Relaxed);
        slot.sequence
            .store(reservation.cursor * 2, Ordering::Release);
    }
    fn record(&self, record: CallbackRecord) {
        let Some(reservation) = self.reserve() else {
            return;
        };
        self.publish(reservation, record);
    }

    fn invalidate(&self) {
        self.losses.fetch_add(1, Ordering::Release);
    }

    fn read_slot(
        slot: &CallbackMetricSlot,
    ) -> Result<Option<CallbackRecord>, CallbackObservationError> {
        let before = slot.sequence.load(Ordering::Acquire);
        if before == 0 || before % 2 == 1 {
            return Ok(None);
        }
        let record = CallbackRecord {
            stream: slot.stream.load(Ordering::Relaxed),
            epoch: slot.epoch.load(Ordering::Relaxed),
            frame_sequence: slot.frame_sequence.load(Ordering::Relaxed),
            completed_at_nanos: slot.completed_at_nanos.load(Ordering::Relaxed),
            elapsed_nanos: slot.elapsed_nanos.load(Ordering::Relaxed),
            copied_bytes: slot.copied_bytes.load(Ordering::Relaxed),
        };
        if slot.sequence.load(Ordering::Acquire) != before {
            return Err(CallbackObservationError::Invalidated);
        }
        Ok(Some(record))
    }

    fn observation_after(
        &self,
        baseline: CallbackMetricBaseline,
        stream: u64,
        epoch: u64,
        frame_sequence: u64,
    ) -> Result<Option<CallbackCopyObservation>, CallbackObservationError> {
        if self.losses.load(Ordering::Acquire) != baseline.losses {
            return Err(CallbackObservationError::Invalidated);
        }
        let current = self.next.load(Ordering::Acquire);
        let capacity = u64::try_from(N).expect("callback observation capacity fits u64");
        if current < baseline.cursor || current.saturating_sub(baseline.cursor) > capacity {
            return Err(CallbackObservationError::Invalidated);
        }

        let mut target = None;
        for slot in &self.slots {
            let Some(record) = Self::read_slot(slot)? else {
                continue;
            };
            if record.stream == stream
                && record.epoch == epoch
                && record.frame_sequence == frame_sequence
            {
                if record.completed_at_nanos <= baseline.completed_at_nanos {
                    return Ok(None);
                }
                target = Some(record);
                break;
            }
        }
        let Some(target) = target else {
            return Ok(None);
        };

        let mut interval_copied_bytes = 0_u64;
        let mut target_seen = false;
        for slot in &self.slots {
            let Some(record) = Self::read_slot(slot)? else {
                continue;
            };
            if record.stream != stream
                || record.completed_at_nanos <= baseline.completed_at_nanos
                || record.completed_at_nanos > target.completed_at_nanos
                || record.epoch > target.epoch
                || (record.epoch == target.epoch && record.frame_sequence > target.frame_sequence)
            {
                continue;
            }
            interval_copied_bytes = interval_copied_bytes
                .checked_add(record.copied_bytes)
                .ok_or(CallbackObservationError::Invalidated)?;
            target_seen |= record.epoch == epoch && record.frame_sequence == frame_sequence;
        }
        if !target_seen
            || self.losses.load(Ordering::Acquire) != baseline.losses
            || self
                .next
                .load(Ordering::Acquire)
                .saturating_sub(baseline.cursor)
                > capacity
        {
            return Err(CallbackObservationError::Invalidated);
        }
        Ok(Some(CallbackCopyObservation {
            callback_copy_time: Duration::from_nanos(target.elapsed_nanos),
            copied_bytes: target.copied_bytes,
            interval_copied_bytes,
        }))
    }
}

#[cfg(feature = "benchmark-instrumentation")]
fn callback_now_nanos() -> u64 {
    u64::try_from(CALLBACK_CLOCK_ORIGIN.elapsed().as_nanos()).unwrap_or(u64::MAX)
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

/// Returns one declared-content point on each side of the moving fixture.
#[cfg(feature = "benchmark-instrumentation")]
#[must_use]
pub fn dual_display_fixture_points(window_x: i32, window_y: i32) -> [(f64, f64); 2] {
    const FIXTURE_WIDTH: i32 = 1_280;
    const FIXTURE_HEIGHT: i32 = 720;
    assert!(
        window_x < 0 && window_x + FIXTURE_WIDTH > 0,
        "the moving fixture must straddle the signed desktop seam"
    );
    let vertical = f64::from(window_y) + f64::from(FIXTURE_HEIGHT) / 2.0;
    [
        (f64::from(window_x) / 2.0, vertical),
        (f64::from(window_x + FIXTURE_WIDTH) / 2.0, vertical),
    ]
}

/// Resets process metrics before a benchmark profile starts.
///
/// The caller must first prove that no capture session or mapped frame is live.
#[cfg(feature = "benchmark-instrumentation")]
pub fn reset_capture_metrics() {
    let detached = DETACHED_LIVE.load(Ordering::Acquire);
    let staging = STAGING_LIVE.load(Ordering::Acquire);
    CALLBACK_OBSERVATIONS.reset();
    DETACHED_PEAK.store(detached, Ordering::Release);
    STAGING_PEAK.store(staging, Ordering::Release);
}

/// Returns the current process resource metrics without changing them.
#[cfg(feature = "benchmark-instrumentation")]
#[must_use]
pub fn capture_metrics() -> CaptureMetricsSnapshot {
    CaptureMetricsSnapshot {
        callback_observation_losses: CALLBACK_OBSERVATIONS.losses.load(Ordering::Acquire),
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
    CALLBACK_OBSERVATIONS.baseline()
}

/// Finds the coherent callback record for one acquired frame after `baseline`.
#[cfg(feature = "benchmark-instrumentation")]
pub fn callback_observation_after(
    baseline: CallbackMetricBaseline,
    stamp: FrameStamp,
) -> Result<Option<CallbackCopyObservation>, CallbackObservationError> {
    CALLBACK_OBSERVATIONS.observation_after(
        baseline,
        stamp.stream().get(),
        stamp.epoch().value(),
        stamp.sequence().value(),
    )
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
    copied_bytes: u64,
    #[cfg(feature = "benchmark-instrumentation")]
    elapsed_nanos: u64,
    #[cfg(feature = "benchmark-instrumentation")]
    completed_at_nanos: u64,
    #[cfg(feature = "benchmark-instrumentation")]
    stream: u64,
}

impl CallbackCopyTimer {
    pub(crate) fn finish(self) -> CompletedCallbackCopy {
        #[cfg(feature = "benchmark-instrumentation")]
        {
            let elapsed_nanos =
                u64::try_from(self.started.elapsed().as_nanos()).unwrap_or(u64::MAX);
            CompletedCallbackCopy {
                copied_bytes: self.copied_bytes,
                elapsed_nanos,
                completed_at_nanos: callback_now_nanos(),
                stream: self.stream,
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
        #[cfg(feature = "benchmark-instrumentation")]
        {
            if self.stream != stamp.stream().get() {
                CALLBACK_OBSERVATIONS.invalidate();
                return;
            }
            CALLBACK_OBSERVATIONS.record(CallbackRecord {
                stream: self.stream,
                epoch: stamp.epoch().value(),
                frame_sequence: stamp.sequence().value(),
                completed_at_nanos: self.completed_at_nanos,
                elapsed_nanos: self.elapsed_nanos,
                copied_bytes: self.copied_bytes,
            });
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

#[cfg(all(test, feature = "benchmark-instrumentation"))]
mod tests {
    use super::*;

    fn baseline_at<const N: usize>(
        ring: &CallbackMetricRing<N>,
        completed_at_nanos: u64,
    ) -> CallbackMetricBaseline {
        let mut baseline = ring.baseline();
        baseline.completed_at_nanos = completed_at_nanos;
        baseline
    }

    const fn record(
        stream: u64,
        epoch: u64,
        frame_sequence: u64,
        completed_at_nanos: u64,
        elapsed_nanos: u64,
        copied_bytes: u64,
    ) -> CallbackRecord {
        CallbackRecord {
            stream,
            epoch,
            frame_sequence,
            completed_at_nanos,
            elapsed_nanos,
            copied_bytes,
        }
    }

    fn publish(
        ring: &CallbackMetricRing<4>,
        stream: u64,
        epoch: u64,
        frame_sequence: u64,
        completed_at_nanos: u64,
        elapsed_nanos: u64,
        copied_bytes: u64,
    ) {
        let reservation = ring.reserve().expect("the local ring has capacity");
        ring.publish(
            reservation,
            record(
                stream,
                epoch,
                frame_sequence,
                completed_at_nanos,
                elapsed_nanos,
                copied_bytes,
            ),
        );
    }

    #[test]
    fn an_in_progress_generation_cannot_be_accepted_as_a_mixed_record() {
        let ring = CallbackMetricRing::<4>::new();
        let baseline = baseline_at(&ring, 0);
        let reservation = ring.reserve().expect("the first slot is available");
        let slot = &ring.slots[0];
        slot.stream.store(7, Ordering::Relaxed);
        slot.epoch.store(11, Ordering::Relaxed);
        slot.frame_sequence.store(13, Ordering::Relaxed);
        slot.completed_at_nanos.store(1, Ordering::Relaxed);
        slot.elapsed_nanos.store(17, Ordering::Relaxed);
        slot.copied_bytes.store(19, Ordering::Relaxed);

        assert_eq!(ring.observation_after(baseline, 7, 11, 13), Ok(None));

        ring.publish(reservation, record(7, 11, 13, 1, 17, 19));
        assert_eq!(
            ring.observation_after(baseline, 7, 11, 13),
            Ok(Some(CallbackCopyObservation {
                callback_copy_time: Duration::from_nanos(17),
                copied_bytes: 19,
                interval_copied_bytes: 19,
            }))
        );
    }

    #[test]
    fn a_prebaseline_callback_cannot_satisfy_a_later_frame() {
        let ring = CallbackMetricRing::<4>::new();
        publish(&ring, 3, 5, 7, 1, 11, 13);
        let baseline = baseline_at(&ring, 2);

        assert_eq!(ring.observation_after(baseline, 3, 5, 7), Ok(None));
    }

    #[test]
    fn completion_before_baseline_published_afterward_remains_ineligible() {
        let ring = CallbackMetricRing::<4>::new();
        let baseline = baseline_at(&ring, 2);
        publish(&ring, 3, 5, 7, 1, 11, 13);

        assert_eq!(ring.observation_after(baseline, 3, 5, 7), Ok(None));
    }

    #[test]
    fn another_session_callback_cannot_satisfy_the_requested_frame() {
        let ring = CallbackMetricRing::<4>::new();
        let baseline = baseline_at(&ring, 0);
        publish(&ring, 1, 10, 20, 1, 30, 40);

        assert_eq!(ring.observation_after(baseline, 2, 10, 20), Ok(None));
    }

    #[test]
    fn interval_bytes_include_intervening_same_session_copies_only() {
        let ring = CallbackMetricRing::<4>::new();
        let baseline = baseline_at(&ring, 0);
        publish(&ring, 1, 10, 100, 1, 11, 100);
        publish(&ring, 2, 20, 200, 2, 22, 999);
        publish(&ring, 1, 10, 101, 3, 33, 200);

        assert_eq!(
            ring.observation_after(baseline, 1, 10, 101),
            Ok(Some(CallbackCopyObservation {
                callback_copy_time: Duration::from_nanos(33),
                copied_bytes: 200,
                interval_copied_bytes: 300,
            }))
        );
    }
    #[test]
    fn out_of_order_callbacks_remain_bound_to_their_session_and_frame() {
        let ring = CallbackMetricRing::<4>::new();
        let baseline = baseline_at(&ring, 0);
        let first = ring.reserve().expect("first reservation succeeds");
        let second = ring.reserve().expect("second reservation succeeds");
        ring.publish(second, record(2, 20, 200, 1, 22, 222));
        ring.publish(first, record(1, 10, 100, 2, 11, 111));

        assert_eq!(
            ring.observation_after(baseline, 1, 10, 100),
            Ok(Some(CallbackCopyObservation {
                callback_copy_time: Duration::from_nanos(11),
                copied_bytes: 111,
                interval_copied_bytes: 111,
            }))
        );
        assert_eq!(
            ring.observation_after(baseline, 2, 20, 200),
            Ok(Some(CallbackCopyObservation {
                callback_copy_time: Duration::from_nanos(22),
                copied_bytes: 222,
                interval_copied_bytes: 222,
            }))
        );
    }

    #[test]
    fn a_late_lower_reservation_completed_after_the_baseline_is_visible() {
        let ring = CallbackMetricRing::<4>::new();
        let first = ring.reserve().expect("first reservation succeeds");
        let second = ring.reserve().expect("second reservation succeeds");
        ring.publish(second, record(2, 20, 200, 1, 22, 222));
        let baseline = baseline_at(&ring, 1);
        ring.publish(first, record(1, 10, 100, 2, 11, 111));

        assert_eq!(
            ring.observation_after(baseline, 1, 10, 100),
            Ok(Some(CallbackCopyObservation {
                callback_copy_time: Duration::from_nanos(11),
                copied_bytes: 111,
                interval_copied_bytes: 111,
            }))
        );
    }

    #[test]
    fn contended_instrumentation_invalidates_the_baseline() {
        let ring = CallbackMetricRing::<1>::new();
        let baseline = baseline_at(&ring, 0);
        let first = ring.reserve().expect("first reservation succeeds");
        assert!(
            ring.reserve().is_none(),
            "an in-progress slot is not overwritten"
        );
        ring.publish(first, record(1, 10, 20, 1, 30, 40));

        assert_eq!(
            ring.observation_after(baseline, 1, 10, 20),
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
    fn moving_fixture_points_stay_inside_opposite_display_halves() {
        for index in 0..300 {
            let x = dual_display_seam_x(index);
            let [left, right] = dual_display_fixture_points(x, 600);
            assert!(f64::from(x) < left.0 && left.0 < 0.0);
            assert!(0.0 < right.0 && right.0 < f64::from(x + 1_280));
            assert_eq!(left.1, 960.0);
            assert_eq!(right.1, 960.0);
        }
    }
}
