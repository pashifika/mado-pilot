//! Opt-in process-local matching observations; cumulative counters are never reset.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use mado_pilot_core::{Interruption, OperationContext};
use mado_pilot_vision::Suppression;

use crate::candidates::{Peak, PeakSearch, peaks};

static FIND_CALLS: AtomicU64 = AtomicU64::new(0);
static FIND_COMPLETIONS: AtomicU64 = AtomicU64::new(0);
static FIND_FAILURES: AtomicU64 = AtomicU64::new(0);
static MAPPED_BYTES: AtomicU64 = AtomicU64::new(0);
static ACTIVE_FINDS: AtomicU64 = AtomicU64::new(0);
static PEAK_ACTIVE_FINDS: AtomicU64 = AtomicU64::new(0);
static FIND_DELAY_NANOS: AtomicU64 = AtomicU64::new(0);
static SCORE_COPIED_BYTES: AtomicU64 = AtomicU64::new(0);
static SCORE_OFFSETS_SCANNED: AtomicU64 = AtomicU64::new(0);
static BGR_CONVERSIONS: AtomicU64 = AtomicU64::new(0);
static BGR_CONVERTED_BYTES: AtomicU64 = AtomicU64::new(0);
static BGR_RETAINED_BYTES: AtomicU64 = AtomicU64::new(0);
static BGR_RETAINED_PEAK_BYTES: AtomicU64 = AtomicU64::new(0);
static BGR_EVICTED_BYTES: AtomicU64 = AtomicU64::new(0);
static BGR_EVICTED_PEAK_BYTES: AtomicU64 = AtomicU64::new(0);

/// Physical work counters, separate from logical mapped-view byte counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkSnapshot {
    /// Completed packed mapping copies since process start.
    pub mapping_copies: u64,
    /// Packed mapping bytes copied since process start.
    pub mapping_copied_bytes: u64,
    /// Score-map bytes copied for selection since process start.
    pub score_copied_bytes: u64,
    /// Score offsets inspected by selection since process start.
    pub score_offsets_scanned: u64,
    /// Completed region-to-BGR conversions since process start.
    pub bgr_conversions: u64,
    /// Packed BGR bytes produced since process start.
    pub bgr_converted_bytes: u64,
    /// Cache-retained BGR bytes across all backend instances.
    pub bgr_retained_bytes: u64,
    /// Process high-water cache-retained BGR bytes.
    pub bgr_retained_peak_bytes: u64,
    /// Evicted BGR payload awaiting its final owner release.
    pub bgr_evicted_bytes: u64,
    /// Process high-water evicted BGR payload.
    pub bgr_evicted_peak_bytes: u64,
}

/// Observes counters; take interval boundaries with no active searches.
#[must_use]
pub fn work_snapshot() -> WorkSnapshot {
    let mapping = mado_pilot_capture::benchmark_instrumentation::snapshot();
    WorkSnapshot {
        mapping_copies: mapping.copies,
        mapping_copied_bytes: mapping.copied_bytes,
        score_copied_bytes: SCORE_COPIED_BYTES.load(Ordering::Acquire),
        score_offsets_scanned: SCORE_OFFSETS_SCANNED.load(Ordering::Acquire),
        bgr_conversions: BGR_CONVERSIONS.load(Ordering::Acquire),
        bgr_converted_bytes: BGR_CONVERTED_BYTES.load(Ordering::Acquire),
        bgr_retained_bytes: BGR_RETAINED_BYTES.load(Ordering::Acquire),
        bgr_retained_peak_bytes: BGR_RETAINED_PEAK_BYTES.load(Ordering::Acquire),
        bgr_evicted_bytes: BGR_EVICTED_BYTES.load(Ordering::Acquire),
        bgr_evicted_peak_bytes: BGR_EVICTED_PEAK_BYTES.load(Ordering::Acquire),
    }
}

pub(crate) struct ScoreObservation {
    copied_bytes: u64,
    offsets: u64,
}

impl ScoreObservation {
    pub(crate) fn new(copied_values: usize) -> Self {
        Self {
            copied_bytes: (copied_values as u64).saturating_mul(size_of::<f32>() as u64),
            offsets: 0,
        }
    }

    pub(crate) fn scan(&mut self, offsets: usize) {
        self.offsets = self.offsets.saturating_add(offsets as u64);
    }
}

impl Drop for ScoreObservation {
    fn drop(&mut self) {
        SCORE_COPIED_BYTES.fetch_add(self.copied_bytes, Ordering::Relaxed);
        SCORE_OFFSETS_SCANNED.fetch_add(self.offsets, Ordering::Release);
    }
}

pub(crate) fn record_bgr_conversion(bytes: usize) {
    BGR_CONVERSIONS.fetch_add(1, Ordering::Relaxed);
    BGR_CONVERTED_BYTES.fetch_add(bytes as u64, Ordering::Release);
}

pub(crate) fn retain_bgr(bytes: usize) {
    let current = BGR_RETAINED_BYTES.fetch_add(bytes as u64, Ordering::AcqRel) + bytes as u64;
    BGR_RETAINED_PEAK_BYTES.fetch_max(current, Ordering::Release);
}

pub(crate) fn retire_bgr(bytes: usize) {
    BGR_RETAINED_BYTES.fetch_sub(bytes as u64, Ordering::Release);
    let current = BGR_EVICTED_BYTES.fetch_add(bytes as u64, Ordering::AcqRel) + bytes as u64;
    BGR_EVICTED_PEAK_BYTES.fetch_max(current, Ordering::Release);
}

pub(crate) fn release_evicted_bgr(bytes: usize) {
    BGR_EVICTED_BYTES.fetch_sub(bytes as u64, Ordering::Release);
}

/// Selection-only benchmark input; the supplied score slice is disposable.
#[derive(Debug, Clone, Copy)]
pub struct SelectionRequest {
    /// Score-map columns.
    pub width: usize,
    /// Score-map rows.
    pub height: usize,
    /// Template columns.
    pub template_width: usize,
    /// Template rows.
    pub template_height: usize,
    /// Inclusive public-score threshold.
    pub min_score: f64,
    /// Maximum returned candidates.
    pub candidate_budget: usize,
    /// Requested overlap policy.
    pub suppression: Suppression,
}

/// Selected positions and public scores, without an extra result-vector copy.
#[derive(Debug)]
pub struct SelectedScores(Vec<Peak>);

impl SelectedScores {
    /// Iterates in canonical result order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (usize, usize, f64)> + '_ {
        self.0.iter().map(|peak| (peak.left, peak.top, peak.score))
    }
}

/// Runs the production selector independently of OpenCV score generation.
///
/// # Errors
/// Returns the caller's cancellation or deadline interruption.
pub fn select_scores(
    scores: &mut [f32],
    request: SelectionRequest,
    operation: &OperationContext,
) -> Result<SelectedScores, Interruption> {
    peaks(
        scores,
        PeakSearch {
            width: request.width,
            height: request.height,
            template_width: request.template_width,
            template_height: request.template_height,
            min_score: request.min_score,
            candidate_budget: request.candidate_budget,
            suppression: request.suppression,
        },
        operation,
    )
    .map(SelectedScores)
}

/// Exclusive process-wide controlled delay for one native qualification row.
///
/// The guard is intentionally neither cloneable nor shareable. Installation
/// fails while another delay is active, and dropping the guard restores the
/// production path.
#[derive(Debug)]
pub struct FindDelayGuard {
    nanos: u64,
}

/// Installs a bounded delay before each observed backend search.
///
/// This seam exists only in qualification builds. It controls backend latency
/// without replacing the facade-wired OpenCV backend or executing work on a
/// capture callback.
pub fn install_find_delay(delay: Duration) -> Option<FindDelayGuard> {
    let nanos = u64::try_from(delay.as_nanos())
        .ok()
        .filter(|value| *value != 0)?;
    FIND_DELAY_NANOS
        .compare_exchange(0, nanos, Ordering::AcqRel, Ordering::Acquire)
        .ok()
        .map(|_| FindDelayGuard { nanos })
}

impl Drop for FindDelayGuard {
    fn drop(&mut self) {
        let restored =
            FIND_DELAY_NANOS.compare_exchange(self.nanos, 0, Ordering::AcqRel, Ordering::Acquire);
        debug_assert!(
            restored.is_ok(),
            "the installed OpenCV delay remained exclusive"
        );
    }
}

pub(crate) fn configured_find_delay() -> Duration {
    Duration::from_nanos(FIND_DELAY_NANOS.load(Ordering::Acquire))
}

/// One cumulative process-wide OpenCV matching observation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// Backend searches admitted since process start.
    pub find_calls: u64,
    /// Searches that returned candidates successfully.
    pub find_completions: u64,
    /// Searches that returned an error or unwound.
    pub find_failures: u64,
    /// CPU mapping bytes presented to admitted searches.
    pub mapped_bytes: u64,
    /// Searches currently executing.
    pub active_finds: u64,
    /// Largest concurrent search count observed since process start.
    pub peak_active_finds: u64,
}

impl Snapshot {
    /// Computes an interval observation when every cumulative field advanced
    /// monotonically and no search remains active at either boundary.
    #[must_use]
    pub fn checked_delta(self, earlier: Self) -> Option<Self> {
        if earlier.active_finds != 0 || self.active_finds != 0 {
            return None;
        }
        Some(Self {
            find_calls: self.find_calls.checked_sub(earlier.find_calls)?,
            find_completions: self
                .find_completions
                .checked_sub(earlier.find_completions)?,
            find_failures: self.find_failures.checked_sub(earlier.find_failures)?,
            mapped_bytes: self.mapped_bytes.checked_sub(earlier.mapped_bytes)?,
            active_finds: 0,
            peak_active_finds: self.peak_active_finds,
        })
    }
}

/// Takes one coherent-enough cumulative observation.
///
/// Each field is monotonic except `active_finds`. Qualification accepts a delta
/// only when the active count is zero at both boundaries, so a mixed read cannot
/// make unfinished work look complete.
#[must_use]
pub fn snapshot() -> Snapshot {
    Snapshot {
        find_calls: FIND_CALLS.load(Ordering::Acquire),
        find_completions: FIND_COMPLETIONS.load(Ordering::Acquire),
        find_failures: FIND_FAILURES.load(Ordering::Acquire),
        mapped_bytes: MAPPED_BYTES.load(Ordering::Acquire),
        active_finds: ACTIVE_FINDS.load(Ordering::Acquire),
        peak_active_finds: PEAK_ACTIVE_FINDS.load(Ordering::Acquire),
    }
}

pub(crate) struct FindObservation {
    finished: bool,
}

impl FindObservation {
    pub(crate) fn begin(mapped_bytes: usize) -> Self {
        FIND_CALLS.fetch_add(1, Ordering::Relaxed);
        MAPPED_BYTES.fetch_add(
            u64::try_from(mapped_bytes).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        let active = ACTIVE_FINDS
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        PEAK_ACTIVE_FINDS.fetch_max(active, Ordering::AcqRel);
        Self { finished: false }
    }

    pub(crate) fn finish(mut self, succeeded: bool) {
        if succeeded {
            FIND_COMPLETIONS.fetch_add(1, Ordering::Release);
        } else {
            FIND_FAILURES.fetch_add(1, Ordering::Release);
        }
        self.finished = true;
        ACTIVE_FINDS.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Drop for FindObservation {
    fn drop(&mut self) {
        if !self.finished {
            FIND_FAILURES.fetch_add(1, Ordering::Release);
            ACTIVE_FINDS.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{Snapshot, install_find_delay};

    #[test]
    fn a_checked_delta_rejects_active_or_regressing_observations() {
        let earlier = Snapshot {
            find_calls: 3,
            find_completions: 2,
            find_failures: 1,
            mapped_bytes: 64,
            active_finds: 0,
            peak_active_finds: 1,
        };
        let later = Snapshot {
            find_calls: 5,
            find_completions: 4,
            find_failures: 1,
            mapped_bytes: 160,
            active_finds: 0,
            peak_active_finds: 2,
        };
        assert_eq!(
            later.checked_delta(earlier),
            Some(Snapshot {
                find_calls: 2,
                find_completions: 2,
                find_failures: 0,
                mapped_bytes: 96,
                active_finds: 0,
                peak_active_finds: 2,
            })
        );
        assert_eq!(
            Snapshot {
                active_finds: 1,
                ..later
            }
            .checked_delta(earlier),
            None
        );
        assert_eq!(earlier.checked_delta(later), None);
    }

    #[test]
    fn a_delay_guard_excludes_another_owner_and_releases_its_slot() {
        let guard = install_find_delay(Duration::from_millis(7)).expect("exclusive delay");
        assert!(install_find_delay(Duration::from_millis(1)).is_none());
        drop(guard);
        let replacement = install_find_delay(Duration::from_millis(1)).expect("released slot");
        drop(replacement);
    }
}
