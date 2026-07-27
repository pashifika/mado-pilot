//! A clock a test moves by hand.
//!
//! Deadline behavior is tested by advancing time, never by sleeping. A sleeping
//! test asserts something about how busy the machine is; this one asserts
//! something about the code.

use std::sync::Mutex;
use std::time::Duration;

use mado_pilot_core::{Clock, MonotonicInstant};

/// A monotonic clock that only moves when a test moves it.
#[derive(Debug, Default)]
pub struct ManualClock {
    elapsed: Mutex<Duration>,
}

impl ManualClock {
    /// Returns a clock at the domain origin.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves the clock forward by `step`.
    ///
    /// Saturating rather than wrapping: a monotonic clock that went backwards
    /// would make a latched deadline unlatch, which is the one thing the
    /// operation contract relies on not happening.
    pub fn advance(&self, step: Duration) {
        let mut elapsed = self.lock();
        *elapsed = elapsed.saturating_add(step);
    }

    /// Returns the elapsed time since the domain origin.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        *self.lock()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Duration> {
        self.elapsed.lock().unwrap_or_else(|poisoned| {
            // A poisoned manual clock still holds a valid instant: the panic
            // that poisoned it was in a test, not in the clock.
            poisoned.into_inner()
        })
    }
}

impl Clock for ManualClock {
    fn now(&self) -> MonotonicInstant {
        MonotonicInstant::from_origin(self.elapsed())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use mado_pilot_core::{Clock, MonotonicInstant};

    use super::ManualClock;

    #[test]
    fn a_new_clock_sits_at_the_domain_origin() {
        assert_eq!(ManualClock::new().now(), MonotonicInstant::ORIGIN);
    }

    #[test]
    fn advancing_moves_the_reported_instant() {
        let clock = ManualClock::new();
        clock.advance(Duration::from_millis(250));
        clock.advance(Duration::from_millis(250));

        assert_eq!(
            clock.now(),
            MonotonicInstant::from_origin(Duration::from_millis(500))
        );
    }

    #[test]
    fn a_clock_cannot_be_pushed_past_the_end_of_the_domain() {
        let clock = ManualClock::new();
        clock.advance(Duration::MAX);
        clock.advance(Duration::from_secs(1));

        assert_eq!(clock.elapsed(), Duration::MAX);
    }
}
