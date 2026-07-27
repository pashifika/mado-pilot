//! The single monotonic clock domain that every operation deadline uses.
//!
//! Deadlines are absolute instants in this domain rather than durations, so a
//! deadline that is passed down through several stages keeps meaning the same
//! moment instead of restarting at each hop. The domain is monotonic: moving
//! civil time forward or backward does not change how long an operation has
//! left.

use std::fmt;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// A point in MadoPilot's monotonic clock domain.
///
/// The origin is an unspecified moment fixed for the life of the process. Only
/// differences between instants are meaningful; the absolute value is not a wall
/// clock time and must not be presented as one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MonotonicInstant(Duration);

impl MonotonicInstant {
    /// The domain origin.
    pub const ORIGIN: Self = Self(Duration::ZERO);

    /// Builds an instant `elapsed` after the domain origin.
    ///
    /// This exists for adapters that carry a deadline across a boundary which
    /// cannot hold the type itself, such as the C ABI.
    #[must_use]
    pub const fn from_origin(elapsed: Duration) -> Self {
        Self(elapsed)
    }

    /// Returns the offset from the domain origin.
    #[must_use]
    pub const fn since_origin(self) -> Duration {
        self.0
    }

    /// Returns `self + duration`, or `None` when the sum is not representable.
    ///
    /// A caller that cannot represent the requested deadline must report that
    /// rather than silently substituting a nearer one.
    #[must_use]
    pub fn checked_add(self, duration: Duration) -> Option<Self> {
        self.0.checked_add(duration).map(Self)
    }

    /// Returns the time from `earlier` to `self`, or zero when `self` is earlier.
    #[must_use]
    pub fn saturating_duration_since(self, earlier: Self) -> Duration {
        self.0.saturating_sub(earlier.0)
    }
}

/// The monotonic time source an operation evaluates its deadline against.
///
/// This is a seam so that deadline and cancellation races can be tested
/// deterministically instead of by sleeping. Production code uses
/// [`SystemClock`]; a test supplies a clock it advances by hand.
pub trait Clock: fmt::Debug + Send + Sync {
    /// Returns the current instant in the monotonic domain.
    fn now(&self) -> MonotonicInstant;
}

/// The process monotonic clock.
///
/// Backed by [`Instant`], which is monotonic on both release targets and is not
/// affected by wall-clock adjustment.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> MonotonicInstant {
        static ORIGIN: OnceLock<Instant> = OnceLock::new();
        let origin = *ORIGIN.get_or_init(Instant::now);
        MonotonicInstant(Instant::now().saturating_duration_since(origin))
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use std::sync::Mutex;
    use std::time::Duration;

    use super::{Clock, MonotonicInstant};

    /// A clock that only moves when a test moves it.
    #[derive(Debug, Default)]
    pub(crate) struct ManualClock {
        now: Mutex<Duration>,
    }

    impl ManualClock {
        pub(crate) fn new() -> Self {
            Self::default()
        }

        /// Advances the clock by `duration`.
        pub(crate) fn advance(&self, duration: Duration) {
            let mut now = self.now.lock().expect("manual clock is not poisoned");
            *now = now.saturating_add(duration);
        }
    }

    impl Clock for ManualClock {
        fn now(&self) -> MonotonicInstant {
            MonotonicInstant::from_origin(*self.now.lock().expect("manual clock is not poisoned"))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::testing::ManualClock;
    use super::{Clock, MonotonicInstant, SystemClock};

    #[test]
    fn manual_clock_moves_only_when_advanced() {
        let clock = ManualClock::new();
        let first = clock.now();

        assert_eq!(clock.now(), first);

        clock.advance(Duration::from_millis(5));

        assert_eq!(
            clock.now().saturating_duration_since(first),
            Duration::from_millis(5)
        );
    }

    #[test]
    fn system_clock_does_not_go_backwards() {
        let clock = SystemClock;
        let first = clock.now();
        let second = clock.now();

        assert!(second >= first);
    }

    #[test]
    fn duration_since_a_later_instant_saturates_to_zero() {
        let earlier = MonotonicInstant::from_origin(Duration::from_secs(1));
        let later = MonotonicInstant::from_origin(Duration::from_secs(2));

        assert_eq!(
            earlier.saturating_duration_since(later),
            Duration::from_secs(0)
        );
        assert_eq!(
            later.saturating_duration_since(earlier),
            Duration::from_secs(1)
        );
    }

    #[test]
    fn an_unrepresentable_deadline_is_reported_rather_than_truncated() {
        let near_end = MonotonicInstant::from_origin(Duration::MAX);

        assert_eq!(near_end.checked_add(Duration::from_secs(1)), None);
    }
}
