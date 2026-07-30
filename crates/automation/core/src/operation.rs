//! Deadline and cancellation propagation, and terminal-outcome arbitration.
//!
//! Every potentially blocking MadoPilot operation takes an [`OperationContext`]
//! and drives it through an [`Operation`], which decides exactly one terminal
//! outcome. The type enforces that rule rather than documenting it: committing
//! consumes the [`Operation`], and once an interruption has been observed it is
//! latched, so a result produced after the fact cannot overwrite it.
//!
//! Nothing here names an async executor. A deadline is a completion contract,
//! not an interruption guarantee: an uninterruptible decoder or backend call may
//! run to the end, and what the contract promises is that its late value is
//! discarded.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::status::{Error, Status};
use crate::time::{Clock, MonotonicInstant, SystemClock};

/// A cancellation flag shared by every clone.
///
/// Cloning is cheap and cancelling from any clone cancels all of them, so one
/// token can span several concurrent operations. Cancelling is idempotent.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    flag: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Returns a token that has not been cancelled.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cancellation of every operation holding this token or a clone.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Release);
    }

    /// Reports whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }
}

/// The two ways an operation ends without producing its result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Interruption {
    /// The cancellation token was set.
    Cancelled,
    /// The absolute deadline passed.
    DeadlineExceeded,
}

impl Interruption {
    /// Returns the public status this interruption reports as.
    #[must_use]
    pub const fn status(self) -> Status {
        match self {
            Interruption::Cancelled => Status::Cancelled,
            Interruption::DeadlineExceeded => Status::DeadlineExceeded,
        }
    }
}

impl From<Interruption> for Error {
    fn from(interruption: Interruption) -> Self {
        Error::new(
            interruption.status(),
            match interruption {
                Interruption::Cancelled => "operation was cancelled",
                Interruption::DeadlineExceeded => "operation deadline passed",
            },
        )
    }
}

/// The deadline, cancellation, and clock an operation is evaluated against.
///
/// A context with no deadline explicitly means "no deadline". It is never a
/// stand-in for a very large timeout, because a caller cannot tell those apart
/// and one of them silently fails late.
#[derive(Debug, Clone)]
pub struct OperationContext {
    clock: Arc<dyn Clock>,
    deadline: Option<MonotonicInstant>,
    cancellation: Option<CancellationToken>,
}

impl OperationContext {
    /// Returns a context with no deadline, no cancellation, and the system clock.
    #[must_use]
    pub fn new() -> Self {
        Self {
            clock: Arc::new(SystemClock),
            deadline: None,
            cancellation: None,
        }
    }

    /// Replaces the clock this context evaluates its deadline against.
    ///
    /// Adapters and tests use this to make deadline behavior deterministic. A
    /// caller must not mix instants taken from different clocks in one context.
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Sets an absolute deadline in this context's clock domain.
    #[must_use]
    pub fn with_deadline(mut self, deadline: MonotonicInstant) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Sets a deadline `timeout` from now, or fails when it is unrepresentable.
    ///
    /// A timeout that cannot be represented is reported rather than clamped to a
    /// nearer instant, because a silently shortened deadline expires early.
    pub fn with_timeout(self, timeout: Duration) -> Result<Self, Error> {
        let deadline = self.clock.now().checked_add(timeout).ok_or_else(|| {
            Error::new(
                Status::InvalidArgument,
                "requested timeout is not representable in the monotonic domain",
            )
        })?;
        Ok(self.with_deadline(deadline))
    }

    /// Attaches a cancellation token.
    #[must_use]
    pub fn with_cancellation(mut self, token: CancellationToken) -> Self {
        self.cancellation = Some(token);
        self
    }

    /// Returns the absolute deadline, or `None` when the operation has none.
    #[must_use]
    pub fn deadline(&self) -> Option<MonotonicInstant> {
        self.deadline
    }

    /// Returns the cancellation token, if one is attached.
    #[must_use]
    pub fn cancellation(&self) -> Option<&CancellationToken> {
        self.cancellation.as_ref()
    }

    /// Returns the current instant in this context's clock domain.
    #[must_use]
    pub fn now(&self) -> MonotonicInstant {
        self.clock.now()
    }

    /// Returns the clock, so a nested operation shares the same domain.
    #[must_use]
    pub fn clock(&self) -> Arc<dyn Clock> {
        Arc::clone(&self.clock)
    }

    /// Returns the time left before the deadline.
    ///
    /// `None` means there is no deadline. `Some(Duration::ZERO)` means the
    /// deadline has passed, which is distinct from having none.
    #[must_use]
    pub fn remaining(&self) -> Option<Duration> {
        self.deadline
            .map(|deadline| deadline.saturating_duration_since(self.clock.now()))
    }

    /// Returns the interruption that currently applies, if any.
    ///
    /// Cancellation is reported ahead of deadline expiry when both hold, so one
    /// shared token produces one consistent answer across operations.
    #[must_use]
    pub fn interruption(&self) -> Option<Interruption> {
        if self
            .cancellation
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
        {
            return Some(Interruption::Cancelled);
        }
        if self
            .deadline
            .is_some_and(|deadline| self.clock.now() >= deadline)
        {
            return Some(Interruption::DeadlineExceeded);
        }
        None
    }
}

impl Default for OperationContext {
    fn default() -> Self {
        Self::new()
    }
}

/// One attempt at an operation, holding its single terminal outcome.
///
/// The lifecycle is admit, then any number of checkpoints, then one commit:
///
/// ```
/// use mado_pilot_core::operation::{Operation, OperationContext};
///
/// let context = OperationContext::new();
/// let mut operation = Operation::admit(&context)?;
/// // ... first stage ...
/// operation.checkpoint()?;
/// // ... second stage ...
/// let value = operation.commit(42)?;
/// assert_eq!(value, 42);
/// # Ok::<(), mado_pilot_core::operation::Interruption>(())
/// ```
///
/// `commit` takes `self`, so an operation cannot commit twice. A checkpoint that
/// observes an interruption latches it, so a caller that ignores the checkpoint
/// error and commits anyway still gets that same interruption rather than a
/// success.
#[derive(Debug)]
pub struct Operation<'context> {
    context: &'context OperationContext,
    terminal: Option<Interruption>,
}

impl<'context> Operation<'context> {
    /// Checks the context before any blocking work is admitted.
    ///
    /// An operation that is already cancelled or already past its deadline never
    /// starts, so it cannot acquire resources it would immediately abandon.
    pub fn admit(context: &'context OperationContext) -> Result<Self, Interruption> {
        match context.interruption() {
            Some(interruption) => Err(interruption),
            None => Ok(Self {
                context,
                terminal: None,
            }),
        }
    }

    /// Checks the context between interruptible stages.
    ///
    /// The first interruption observed is latched and returned by every later
    /// call and by [`Operation::commit`].
    pub fn checkpoint(&mut self) -> Result<(), Interruption> {
        if let Some(terminal) = self.terminal {
            return Err(terminal);
        }
        match self.context.interruption() {
            Some(interruption) => {
                self.terminal = Some(interruption);
                Err(interruption)
            }
            None => Ok(()),
        }
    }

    /// Commits `value` as the operation's single successful outcome.
    ///
    /// `value` is already computed by the time this is called, so a late result
    /// is discarded here rather than prevented earlier. That is the deliberate
    /// shape of the contract: work that cannot be interrupted is allowed to
    /// finish, and what it may not do is become observable.
    /// The latch is read and not written: this consumes the operation, and
    /// `Operation` has no `Drop`, so an interruption stored here would be
    /// dropped in the same expression that stored it. `checkpoint` latches
    /// because there are later calls to answer; there are none after this.
    pub fn commit<T>(self, value: T) -> Result<T, Interruption> {
        if let Some(terminal) = self.terminal {
            return Err(terminal);
        }
        match self.context.interruption() {
            Some(interruption) => Err(interruption),
            None => Ok(value),
        }
    }

    /// Returns the context this operation is bound to.
    #[must_use]
    pub const fn context(&self) -> &'context OperationContext {
        self.context
    }

    /// Returns the latched terminal interruption, if one was observed.
    #[must_use]
    pub const fn terminal(&self) -> Option<Interruption> {
        self.terminal
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::{CancellationToken, Interruption, Operation, OperationContext};
    use crate::status::Status;
    use crate::time::{MonotonicInstant, testing::ManualClock};

    fn manual() -> (Arc<ManualClock>, OperationContext) {
        let clock = Arc::new(ManualClock::new());
        let context = OperationContext::new().with_clock(clock.clone());
        (clock, context)
    }

    #[test]
    fn a_context_without_a_deadline_never_expires() {
        let (clock, context) = manual();
        clock.advance(Duration::from_secs(60 * 60 * 24 * 365));

        assert_eq!(context.deadline(), None);
        assert_eq!(context.remaining(), None);
        assert_eq!(context.interruption(), None);
    }

    #[test]
    fn an_expired_deadline_is_reported_before_admission() {
        let (clock, context) = manual();
        let context = context.with_deadline(MonotonicInstant::from_origin(Duration::from_secs(1)));
        clock.advance(Duration::from_secs(1));

        assert_eq!(
            Operation::admit(&context).err(),
            Some(Interruption::DeadlineExceeded)
        );
    }

    #[test]
    fn an_already_cancelled_token_is_reported_before_admission() {
        let token = CancellationToken::new();
        token.cancel();
        let context = OperationContext::new().with_cancellation(token);

        assert_eq!(
            Operation::admit(&context).err(),
            Some(Interruption::Cancelled)
        );
    }

    #[test]
    fn a_deadline_at_the_current_instant_has_already_passed() {
        let (clock, context) = manual();
        clock.advance(Duration::from_millis(10));
        let now = context.now();
        let context = context.with_deadline(now);

        assert_eq!(context.interruption(), Some(Interruption::DeadlineExceeded));
        assert_eq!(context.remaining(), Some(Duration::ZERO));
    }

    #[test]
    fn cancellation_after_admission_stops_the_commit() {
        let token = CancellationToken::new();
        let context = OperationContext::new().with_cancellation(token.clone());
        let operation = Operation::admit(&context).expect("admitted");

        token.cancel();

        assert_eq!(operation.commit(7).err(), Some(Interruption::Cancelled));
    }

    #[test]
    fn a_late_result_cannot_replace_a_latched_interruption() {
        let token = CancellationToken::new();
        let context = OperationContext::new().with_cancellation(token.clone());
        let mut operation = Operation::admit(&context).expect("admitted");

        token.cancel();
        assert_eq!(operation.checkpoint().err(), Some(Interruption::Cancelled));

        // A caller that ignores the checkpoint error still cannot commit: the
        // terminal outcome was decided at the checkpoint.
        assert_eq!(
            operation.commit("late").err(),
            Some(Interruption::Cancelled)
        );
    }

    #[test]
    fn an_interruption_latched_from_a_deadline_survives_a_clock_that_cannot_move_back() {
        let (clock, context) = manual();
        let context = context.with_deadline(MonotonicInstant::from_origin(Duration::from_secs(5)));
        let mut operation = Operation::admit(&context).expect("admitted");

        clock.advance(Duration::from_secs(5));
        assert_eq!(
            operation.checkpoint().err(),
            Some(Interruption::DeadlineExceeded)
        );
        assert_eq!(
            operation.terminal(),
            Some(Interruption::DeadlineExceeded),
            "the first interruption observed is the one that stays committed"
        );
    }

    #[test]
    fn cancellation_wins_over_deadline_expiry_when_both_hold() {
        let (clock, context) = manual();
        let token = CancellationToken::new();
        let context = context
            .with_deadline(MonotonicInstant::from_origin(Duration::from_secs(1)))
            .with_cancellation(token.clone());
        clock.advance(Duration::from_secs(2));
        token.cancel();

        assert_eq!(context.interruption(), Some(Interruption::Cancelled));
    }

    #[test]
    fn an_uninterrupted_operation_commits_exactly_its_value() {
        let context = OperationContext::new();
        let mut operation = Operation::admit(&context).expect("admitted");

        operation.checkpoint().expect("not interrupted");

        assert_eq!(operation.commit(vec![1, 2, 3]), Ok(vec![1, 2, 3]));
    }

    #[test]
    fn one_shared_token_interrupts_every_context_holding_a_clone() {
        let token = CancellationToken::new();
        let first = OperationContext::new().with_cancellation(token.clone());
        let second = OperationContext::new().with_cancellation(token.clone());
        let first_operation = Operation::admit(&first).expect("admitted");
        let second_operation = Operation::admit(&second).expect("admitted");

        token.cancel();

        assert_eq!(
            first_operation.commit(()).err(),
            Some(Interruption::Cancelled)
        );
        assert_eq!(
            second_operation.commit(()).err(),
            Some(Interruption::Cancelled)
        );
    }

    #[test]
    fn a_committed_success_is_unaffected_by_later_cancellation() {
        let token = CancellationToken::new();
        let context = OperationContext::new().with_cancellation(token.clone());
        let operation = Operation::admit(&context).expect("admitted");

        let committed = operation.commit(99).expect("committed before cancellation");
        token.cancel();

        assert_eq!(committed, 99);
    }

    #[test]
    fn interruptions_report_their_public_status() {
        assert_eq!(Interruption::Cancelled.status(), Status::Cancelled);
        assert_eq!(
            Interruption::DeadlineExceeded.status(),
            Status::DeadlineExceeded
        );
    }

    #[test]
    fn an_unrepresentable_timeout_is_rejected() {
        let (clock, context) = manual();
        clock.advance(Duration::from_secs(1));

        let error = context
            .with_timeout(Duration::MAX)
            .expect_err("not representable");

        assert_eq!(error.status(), Status::InvalidArgument);
    }

    #[test]
    fn a_timeout_becomes_an_absolute_deadline_in_the_context_clock_domain() {
        let (clock, context) = manual();
        clock.advance(Duration::from_secs(10));
        let context = context
            .with_timeout(Duration::from_secs(5))
            .expect("representable");

        assert_eq!(context.remaining(), Some(Duration::from_secs(5)));

        clock.advance(Duration::from_secs(4));
        assert_eq!(context.remaining(), Some(Duration::from_secs(1)));
        assert_eq!(context.interruption(), None);

        clock.advance(Duration::from_secs(1));
        assert_eq!(context.interruption(), Some(Interruption::DeadlineExceeded));
    }
}
