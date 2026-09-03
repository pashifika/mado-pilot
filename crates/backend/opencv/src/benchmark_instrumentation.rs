//! Process-local observations for native qualification benchmarks.
//!
//! Product composition does not enable this module. Counters are cumulative so
//! concurrent matching never races a reset; a benchmark takes two snapshots and
//! compares their checked delta.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static FIND_CALLS: AtomicU64 = AtomicU64::new(0);
static FIND_COMPLETIONS: AtomicU64 = AtomicU64::new(0);
static FIND_FAILURES: AtomicU64 = AtomicU64::new(0);
static MAPPED_BYTES: AtomicU64 = AtomicU64::new(0);
static ACTIVE_FINDS: AtomicU64 = AtomicU64::new(0);
static PEAK_ACTIVE_FINDS: AtomicU64 = AtomicU64::new(0);
static FIND_DELAY_NANOS: AtomicU64 = AtomicU64::new(0);

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

    use super::{FindObservation, Snapshot, configured_find_delay, install_find_delay, snapshot};

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
    fn observation_and_delay_guards_restore_process_state() {
        let before = snapshot();
        FindObservation::begin(32).finish(true);
        drop(FindObservation::begin(16));
        let delta = snapshot()
            .checked_delta(before)
            .expect("both observations completed");
        assert_eq!(delta.find_calls, 2);
        assert_eq!(delta.find_completions, 1);
        assert_eq!(delta.find_failures, 1);
        assert_eq!(delta.mapped_bytes, 48);

        let guard = install_find_delay(Duration::from_millis(7)).expect("exclusive delay");
        assert_eq!(configured_find_delay(), Duration::from_millis(7));
        assert!(install_find_delay(Duration::from_millis(1)).is_none());
        drop(guard);
        assert_eq!(configured_find_delay(), Duration::ZERO);
    }
}
