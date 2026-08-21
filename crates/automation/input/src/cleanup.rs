//! The bounds the releases after a partial failure run under.
//!
//! Cleanup exists because an operating system cannot recall an event that may
//! already have native effect: a sequence that pressed a modifier and then failed
//! has left the modifier held, and the user's next keystroke carries it. The
//! releases therefore have to run under bounds of their own.
//!
//! # Why not the request's own context
//!
//! The request's deadline has usually already passed, or its cancellation is
//! already set, and that is frequently *why* cleanup is running. Releasing under
//! that context would refuse to release at the one moment releasing matters, which
//! is precisely the outcome cleanup exists to prevent. [`CleanupBudget::context`]
//! therefore derives a fresh context from the request's clock domain, with its own
//! deadline and no cancellation.

use std::fmt;
use std::time::Duration;

use mado_pilot_core::{MonotonicInstant, OperationContext};

use crate::request::SequenceLimits;

/// How much cleanup may do before it stops and reports what is still held.
///
/// Two independent limits, because they bound different failures. The event count
/// bounds how much work one cleanup can be asked to do; the duration bounds how
/// long a platform that has started refusing can hold the controller. Either one
/// reaching its end is an `Exhausted` cleanup rather than a failed release, and the
/// receipt keeps the two apart: the releases cleanup never attempted might still
/// work, and a caller that must not leave a modifier held can send them itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CleanupBudget {
    max_events: usize,
    max_duration: Duration,
}

impl CleanupBudget {
    /// The most release events cleanup may deliver.
    ///
    /// One sequence can hold at most one release per press event, and a sequence
    /// holds at most [`SequenceLimits::MAX_EVENTS`] events, so this is the tight
    /// bound rather than a chosen one. A smaller ceiling would guarantee stuck
    /// state for a sequence that pressed more than it — which is the failure
    /// cleanup exists to avoid, so the bound that avoids it is the whole sequence.
    pub const MAX_EVENTS: usize = SequenceLimits::MAX_EVENTS;

    /// The longest cleanup may keep starting new releases.
    ///
    /// Two hundred and fifty milliseconds is generous for the work: every release
    /// is one synthetic event, and the most a sequence can owe is
    /// [`CleanupBudget::MAX_EVENTS`] of them. It is short enough that a platform
    /// which has begun refusing does not hold the controller while it does so.
    ///
    /// Like every deadline in this codebase, it is a completion contract rather
    /// than an interruption guarantee: a single platform call that hangs runs to
    /// its end, and what the bound decides is that no further release begins.
    pub const MAX_DURATION: Duration = Duration::from_millis(250);

    /// The contract's own bounds.
    #[must_use]
    pub const fn contract() -> Self {
        Self {
            max_events: Self::MAX_EVENTS,
            max_duration: Self::MAX_DURATION,
        }
    }

    /// Bounds tighter than the contract's, clamped to it.
    ///
    /// Clamping rather than refusing, for the reason [`SequenceLimits::at_most`]
    /// clamps: asking for more than the contract offers is asking for something
    /// that does not exist, and the honest answer is the bound that applies.
    #[must_use]
    pub const fn at_most(max_events: usize, max_duration: Duration) -> Self {
        Self {
            max_events: if max_events < Self::MAX_EVENTS {
                max_events
            } else {
                Self::MAX_EVENTS
            },
            max_duration: if max_duration.as_nanos() < Self::MAX_DURATION.as_nanos() {
                max_duration
            } else {
                Self::MAX_DURATION
            },
        }
    }

    /// Returns the most release events cleanup may deliver.
    #[must_use]
    pub const fn max_events(self) -> usize {
        self.max_events
    }

    /// Returns how long cleanup may keep starting releases.
    #[must_use]
    pub const fn max_duration(self) -> Duration {
        self.max_duration
    }

    /// Returns the context cleanup runs under, in `request`'s clock domain.
    ///
    /// A fresh context every time, sharing the clock and opaque activity tag:
    /// the deadline is this budget's, and there is no cancellation token, because
    /// the request's own interruption is usually what caused the failure being cleaned up.
    ///
    /// A monotonic domain so near its end that the duration is unrepresentable
    /// leaves cleanup with no deadline and bounded by its event count alone. That
    /// is deliberate. The alternative — an immediately expired deadline — would
    /// decline to release pressed state, and a stuck modifier is a worse outcome
    /// than an unbounded-in-theory release of at most
    /// [`CleanupBudget::MAX_EVENTS`] events.
    #[must_use]
    pub fn context(self, request: &OperationContext) -> OperationContext {
        let clock = request.clock();
        let started: MonotonicInstant = clock.now();
        let mut context = OperationContext::new().with_clock(clock);
        if let Some(activity_tag) = request.activity_tag() {
            context = context.with_activity_tag(activity_tag);
        }
        match started.checked_add(self.max_duration) {
            Some(deadline) => context.with_deadline(deadline),
            None => context,
        }
    }
}

impl Default for CleanupBudget {
    fn default() -> Self {
        Self::contract()
    }
}

impl fmt::Display for CleanupBudget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "at most {} release(s) within {:?}",
            self.max_events, self.max_duration
        )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::CleanupBudget;
    use crate::request::SequenceLimits;
    use mado_pilot_core::{
        ActivityTag, CancellationToken, Clock, MonotonicInstant, OperationContext,
    };

    /// A clock a test moves by hand.
    ///
    /// The core package keeps its own for its own tests and does not publish it,
    /// and the testkit's is downstream of this package, so a deadline test here
    /// carries the four lines rather than inverting a dependency for them.
    #[derive(Debug, Default)]
    struct ManualClock {
        elapsed: Mutex<Duration>,
    }

    impl ManualClock {
        fn new() -> Self {
            Self::default()
        }

        fn advance(&self, step: Duration) {
            let mut elapsed = self.elapsed.lock().expect("uncontended");
            *elapsed = elapsed.saturating_add(step);
        }
    }

    impl Clock for ManualClock {
        fn now(&self) -> MonotonicInstant {
            MonotonicInstant::from_origin(*self.elapsed.lock().expect("uncontended"))
        }
    }

    #[test]
    fn the_event_bound_is_what_one_sequence_can_owe() {
        assert_eq!(
            CleanupBudget::MAX_EVENTS,
            SequenceLimits::MAX_EVENTS,
            "a cleanup that could not release everything a sequence pressed would \
             guarantee the stuck state it exists to prevent"
        );
    }

    #[test]
    fn tighter_bounds_apply_and_larger_ones_are_clamped() {
        let tighter = CleanupBudget::at_most(4, Duration::from_millis(10));

        assert_eq!(tighter.max_events(), 4);
        assert_eq!(tighter.max_duration(), Duration::from_millis(10));

        let larger = CleanupBudget::at_most(
            SequenceLimits::MAX_EVENTS * 4,
            CleanupBudget::MAX_DURATION * 4,
        );
        assert_eq!(larger, CleanupBudget::contract());
    }

    #[test]
    fn cleanup_preserves_identity_without_inheriting_the_requests_interruption() {
        let clock = Arc::new(ManualClock::new());
        let token = CancellationToken::new();
        token.cancel();
        let activity_tag = ActivityTag::new(0x434c_4541_4e55_5001).expect("nonzero activity tag");
        let request = OperationContext::new()
            .with_clock(clock.clone())
            .with_deadline(MonotonicInstant::ORIGIN)
            .with_cancellation(token)
            .with_activity_tag(activity_tag);
        assert!(
            request.interruption().is_some(),
            "the request is the interrupted one"
        );

        let cleanup = CleanupBudget::contract().context(&request);

        assert_eq!(
            cleanup.interruption(),
            None,
            "cleanup runs when the request could not, so it starts clean"
        );
        assert!(
            cleanup.cancellation().is_none(),
            "the request's token is not carried into cleanup"
        );
        assert_eq!(
            cleanup.activity_tag(),
            Some(activity_tag),
            "cleanup releases remain correlated with the sequence that pressed the state"
        );
    }

    #[test]
    fn the_cleanup_deadline_is_the_budgets_own() {
        let clock = Arc::new(ManualClock::new());
        let request = OperationContext::new().with_clock(clock.clone());
        let budget = CleanupBudget::at_most(8, Duration::from_millis(20));

        let cleanup = budget.context(&request);

        assert_eq!(cleanup.remaining(), Some(Duration::from_millis(20)));
        clock.advance(Duration::from_millis(21));
        assert!(
            cleanup.interruption().is_some(),
            "cleanup stops starting releases when its own bound passes"
        );
    }

    #[test]
    fn cleanup_shares_the_requests_clock_domain() {
        let clock = Arc::new(ManualClock::new());
        clock.advance(Duration::from_secs(3));
        let request = OperationContext::new().with_clock(clock.clone());

        let cleanup = CleanupBudget::contract().context(&request);

        assert_eq!(
            cleanup.now(),
            clock.now(),
            "a deadline measured against another clock bounds nothing"
        );
    }
}
