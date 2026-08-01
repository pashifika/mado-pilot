//! What actually happened, for a sequence that was admitted.
//!
//! Every admitted sequence produces exactly one receipt. That is the whole reason
//! this type exists rather than a bare `Result`: an operating system cannot undo a
//! delivered event, so "it failed" is not an answer a caller can act on. The
//! receipt says which mechanism was used, how many events were delivered, which
//! one was last, why it stopped, and what cleanup managed to release.

use std::fmt;

use mado_pilot_core::{InputDelivery, TargetId};

use crate::fault::InputFault;

/// How far an admitted sequence got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SequenceOutcome {
    /// Every event was delivered.
    Complete,
    /// Some input may have reached the target and then the sequence stopped.
    ///
    /// `delivered` still counts only complete logical events. This outcome may
    /// therefore accompany zero completed events when a platform accepted part
    /// of the first event and cannot undo or precisely observe that native work.
    Partial,
    /// No event was delivered.
    ///
    /// The honest outcome for a sequence whose deadline passed while it waited for
    /// the controller: nothing happened, and the caller may retry without
    /// wondering what half-took effect.
    Unexecuted,
}

impl SequenceOutcome {
    /// Returns a stable lowercase slug, for logs and the C ABI mapping.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            SequenceOutcome::Complete => "complete",
            SequenceOutcome::Partial => "partial",
            SequenceOutcome::Unexecuted => "unexecuted",
        }
    }
}

impl fmt::Display for SequenceOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// What became of the state a stopped sequence had pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CleanupState {
    /// The sequence held nothing when it stopped.
    NotNeeded,
    /// Everything the sequence had pressed was released.
    Complete,
    /// Cleanup ran and a release it attempted did not succeed.
    ///
    /// A button or modifier may still be held, which the caller has to know: the
    /// user's next click becomes a drag, and the next keystroke carries a modifier.
    /// The platform refused what cleanup asked of it, so repeating the same release
    /// is unlikely to do better.
    Incomplete,
    /// Cleanup stopped at its own bound with state still held.
    ///
    /// Distinct from [`CleanupState::Incomplete`], and the distinction is
    /// actionable: nothing refused these releases, they were never attempted. A
    /// caller that must not leave a modifier held can send them itself, which is
    /// exactly the wrong conclusion to draw from a platform that said no.
    Exhausted,
}

impl CleanupState {
    /// Reports whether anything the sequence pressed may still be held.
    #[must_use]
    pub const fn may_leave_state_held(self) -> bool {
        matches!(self, CleanupState::Incomplete | CleanupState::Exhausted)
    }

    /// Returns a stable lowercase slug, for logs and the C ABI mapping.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            CleanupState::NotNeeded => "not_needed",
            CleanupState::Complete => "complete",
            CleanupState::Incomplete => "incomplete",
            CleanupState::Exhausted => "exhausted",
        }
    }
}

impl fmt::Display for CleanupState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The one truthful account of an admitted sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputReceipt {
    target: TargetId,
    outcome: SequenceOutcome,
    delivery: Option<InputDelivery>,
    attempted: Vec<InputDelivery>,
    delivered: usize,
    last_completed: Option<usize>,
    failure: Option<InputFault>,
    cleanup: CleanupState,
    cleanup_released: usize,
    cleanup_owed: usize,
}

impl InputReceipt {
    /// Records a sequence that delivered every event through `delivery`.
    #[must_use]
    pub fn complete(target: TargetId, delivery: InputDelivery, delivered: usize) -> Self {
        Self {
            target,
            outcome: SequenceOutcome::Complete,
            delivery: Some(delivery),
            attempted: vec![delivery],
            delivered,
            last_completed: delivered.checked_sub(1),
            failure: None,
            cleanup: CleanupState::NotNeeded,
            cleanup_released: 0,
            cleanup_owed: 0,
        }
    }

    /// Records a sequence for which some input may have reached the target before
    /// it stopped.
    ///
    /// `delivered` counts only logical events known to have completed. It may be
    /// zero when the platform reports that only part of the first event's native
    /// representation took effect. `failure` is why delivery stopped. Cleanup is
    /// recorded separately with
    /// [`InputReceipt::with_cleanup`], because it happens after this outcome is
    /// already decided.
    #[must_use]
    pub fn partial(
        target: TargetId,
        delivery: InputDelivery,
        delivered: usize,
        failure: InputFault,
    ) -> Self {
        Self {
            target,
            outcome: SequenceOutcome::Partial,
            delivery: Some(delivery),
            attempted: vec![delivery],
            delivered,
            last_completed: delivered.checked_sub(1),
            failure: Some(failure),
            cleanup: CleanupState::NotNeeded,
            cleanup_released: 0,
            cleanup_owed: 0,
        }
    }

    /// Records a sequence that delivered nothing.
    #[must_use]
    pub fn unexecuted(target: TargetId, failure: InputFault) -> Self {
        Self {
            target,
            outcome: SequenceOutcome::Unexecuted,
            delivery: None,
            attempted: Vec::new(),
            delivered: 0,
            last_completed: None,
            failure: Some(failure),
            cleanup: CleanupState::NotNeeded,
            cleanup_released: 0,
            cleanup_owed: 0,
        }
    }

    /// Records every mechanism that was tried, in the order they were tried.
    ///
    /// A caller that permitted fallback needs this to know whether its first
    /// choice worked. The mechanism that succeeded, if any, is the last one here
    /// and is also [`InputReceipt::delivery`].
    #[must_use]
    pub fn with_attempted(mut self, attempted: Vec<InputDelivery>) -> Self {
        self.attempted = attempted;
        self
    }

    /// Records what cleanup released out of what it owned.
    ///
    /// `owed` is how many pressed states the sequence held when it stopped and
    /// `released` is how many of them cleanup managed to release. The state follows
    /// from the two counts, so a receipt cannot claim complete cleanup while owing
    /// a release.
    ///
    /// Use this when cleanup attempted every release it owed. A cleanup that
    /// stopped at its own bound records [`InputReceipt::with_exhausted_cleanup`]
    /// instead, because the two leave a caller with different options.
    #[must_use]
    pub fn with_cleanup(mut self, released: usize, owed: usize) -> Self {
        self.cleanup_released = released;
        self.cleanup_owed = owed;
        self.cleanup = if owed == 0 {
            CleanupState::NotNeeded
        } else if released >= owed {
            CleanupState::Complete
        } else {
            CleanupState::Incomplete
        };
        self
    }

    /// Records a cleanup that stopped at its own event or time bound.
    ///
    /// A cleanup that released everything it owed is complete however it got
    /// there, so exhaustion is recorded only when a release is still outstanding:
    /// a receipt cannot report state as possibly held while accounting for all of
    /// it.
    #[must_use]
    pub fn with_exhausted_cleanup(mut self, released: usize, owed: usize) -> Self {
        self.cleanup_released = released;
        self.cleanup_owed = owed;
        self.cleanup = if owed == 0 {
            CleanupState::NotNeeded
        } else if released >= owed {
            CleanupState::Complete
        } else {
            CleanupState::Exhausted
        };
        self
    }

    /// Returns the target the sequence was addressed to.
    #[must_use]
    pub const fn target(&self) -> TargetId {
        self.target
    }

    /// Returns how far the sequence got.
    #[must_use]
    pub const fn outcome(&self) -> SequenceOutcome {
        self.outcome
    }

    /// Returns the mechanism that delivered events, when any did.
    #[must_use]
    pub const fn delivery(&self) -> Option<InputDelivery> {
        self.delivery
    }

    /// Returns every mechanism that was tried, in order.
    #[must_use]
    pub fn attempted(&self) -> &[InputDelivery] {
        &self.attempted
    }

    /// Reports whether delivery fell back from the caller's first choice.
    #[must_use]
    pub fn used_fallback(&self) -> bool {
        match (self.attempted.first(), self.delivery) {
            (Some(first), Some(used)) => *first != used,
            _ => false,
        }
    }

    /// Returns how many events reached the target.
    #[must_use]
    pub const fn delivered(&self) -> usize {
        self.delivered
    }

    /// Returns the index of the last event that completed.
    #[must_use]
    pub const fn last_completed(&self) -> Option<usize> {
        self.last_completed
    }

    /// Returns why the sequence stopped, for anything but a complete one.
    #[must_use]
    pub const fn failure(&self) -> Option<InputFault> {
        self.failure
    }

    /// Returns what became of the state the sequence had pressed.
    #[must_use]
    pub const fn cleanup(&self) -> CleanupState {
        self.cleanup
    }

    /// Returns how many pressed states cleanup released.
    #[must_use]
    pub const fn cleanup_released(&self) -> usize {
        self.cleanup_released
    }

    /// Returns how many pressed states cleanup was responsible for.
    #[must_use]
    pub const fn cleanup_owed(&self) -> usize {
        self.cleanup_owed
    }

    /// Reports whether the sequence delivered everything it was asked to.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        matches!(self.outcome, SequenceOutcome::Complete)
    }
}

impl fmt::Display for InputReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} {} event(s)", self.outcome, self.delivered)?;
        if let Some(delivery) = self.delivery {
            write!(formatter, " via {delivery}")?;
        }
        if let Some(failure) = self.failure {
            write!(formatter, ": {failure}")?;
        }
        if self.cleanup != CleanupState::NotNeeded {
            write!(
                formatter,
                " [cleanup {} {}/{}]",
                self.cleanup, self.cleanup_released, self.cleanup_owed
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{CleanupState, InputReceipt, SequenceOutcome};
    use crate::fault::InputFault;
    use mado_pilot_core::{IdentityIssuer, InputDelivery, ProviderId, TargetId};

    fn target() -> TargetId {
        IdentityIssuer::new()
            .issue_target(ProviderId::new("fake"))
            .expect("issued")
    }

    #[test]
    fn a_complete_receipt_names_the_mechanism_and_the_last_event() {
        let receipt = InputReceipt::complete(target(), InputDelivery::System, 3);

        assert!(receipt.is_complete());
        assert_eq!(receipt.outcome(), SequenceOutcome::Complete);
        assert_eq!(receipt.delivery(), Some(InputDelivery::System));
        assert_eq!(receipt.delivered(), 3);
        assert_eq!(receipt.last_completed(), Some(2));
        assert_eq!(receipt.failure(), None);
        assert_eq!(receipt.cleanup(), CleanupState::NotNeeded);
    }

    #[test]
    fn a_partial_receipt_reports_how_far_it_got_and_why_it_stopped() {
        let receipt = InputReceipt::partial(
            target(),
            InputDelivery::BackgroundTarget,
            2,
            InputFault::DeliveryFailed,
        );

        assert!(!receipt.is_complete());
        assert_eq!(receipt.outcome(), SequenceOutcome::Partial);
        assert_eq!(receipt.delivered(), 2);
        assert_eq!(receipt.last_completed(), Some(1));
        assert_eq!(receipt.failure(), Some(InputFault::DeliveryFailed));
    }

    #[test]
    fn a_partial_receipt_can_report_native_effect_before_any_event_completed() {
        let receipt = InputReceipt::partial(
            target(),
            InputDelivery::System,
            0,
            InputFault::DeliveryFailed,
        );

        assert_eq!(receipt.outcome(), SequenceOutcome::Partial);
        assert_eq!(receipt.delivered(), 0);
        assert_eq!(receipt.last_completed(), None);
        assert_eq!(receipt.delivery(), Some(InputDelivery::System));
    }

    #[test]
    fn an_unexecuted_receipt_delivered_nothing() {
        let receipt = InputReceipt::unexecuted(target(), InputFault::ControllerClosed);

        assert_eq!(receipt.outcome(), SequenceOutcome::Unexecuted);
        assert_eq!(receipt.delivered(), 0);
        assert_eq!(receipt.last_completed(), None);
        assert_eq!(receipt.delivery(), None);
        assert!(receipt.attempted().is_empty());
        assert!(!receipt.used_fallback());
    }

    #[test]
    fn fallback_is_visible_in_the_attempt_order() {
        let receipt = InputReceipt::complete(target(), InputDelivery::System, 1)
            .with_attempted(vec![InputDelivery::BackgroundTarget, InputDelivery::System]);

        assert!(receipt.used_fallback());
        assert_eq!(
            receipt.attempted(),
            [InputDelivery::BackgroundTarget, InputDelivery::System]
        );

        let no_fallback = InputReceipt::complete(target(), InputDelivery::System, 1);
        assert!(!no_fallback.used_fallback());
    }

    #[test]
    fn cleanup_state_follows_from_its_counts() {
        let owed_nothing = InputReceipt::partial(
            target(),
            InputDelivery::System,
            1,
            InputFault::DeliveryFailed,
        )
        .with_cleanup(0, 0);
        let released_all = InputReceipt::partial(
            target(),
            InputDelivery::System,
            1,
            InputFault::DeliveryFailed,
        )
        .with_cleanup(2, 2);
        let stuck = InputReceipt::partial(
            target(),
            InputDelivery::System,
            1,
            InputFault::DeliveryFailed,
        )
        .with_cleanup(1, 2);

        assert_eq!(owed_nothing.cleanup(), CleanupState::NotNeeded);
        assert_eq!(released_all.cleanup(), CleanupState::Complete);
        assert_eq!(stuck.cleanup(), CleanupState::Incomplete);
        assert!(stuck.cleanup().may_leave_state_held());
        assert!(!released_all.cleanup().may_leave_state_held());
        assert_eq!(stuck.cleanup_released(), 1);
        assert_eq!(stuck.cleanup_owed(), 2);
    }

    #[test]
    fn an_exhausted_cleanup_is_distinguishable_from_a_refused_release() {
        let refused = InputReceipt::partial(
            target(),
            InputDelivery::System,
            1,
            InputFault::DeliveryFailed,
        )
        .with_cleanup(1, 2);
        let exhausted = InputReceipt::partial(
            target(),
            InputDelivery::System,
            1,
            InputFault::DeliveryFailed,
        )
        .with_exhausted_cleanup(1, 2);

        assert_eq!(refused.cleanup(), CleanupState::Incomplete);
        assert_eq!(exhausted.cleanup(), CleanupState::Exhausted);
        assert!(refused.cleanup().may_leave_state_held());
        assert!(exhausted.cleanup().may_leave_state_held());
        assert_eq!(exhausted.cleanup_released(), 1);
        assert_eq!(exhausted.cleanup_owed(), 2);
    }

    #[test]
    fn a_cleanup_that_released_everything_is_complete_however_it_stopped() {
        let receipt = InputReceipt::partial(
            target(),
            InputDelivery::System,
            1,
            InputFault::DeliveryFailed,
        )
        .with_exhausted_cleanup(2, 2);

        assert_eq!(
            receipt.cleanup(),
            CleanupState::Complete,
            "nothing is held, so nothing is owed to a bound"
        );
        assert!(!receipt.cleanup().may_leave_state_held());
    }

    #[test]
    fn a_receipt_reads_as_one_account() {
        let receipt = InputReceipt::partial(
            target(),
            InputDelivery::System,
            2,
            InputFault::PolicyRefused,
        )
        .with_cleanup(1, 2);
        let text = receipt.to_string();

        assert!(text.contains("partial 2 event(s) via system"), "{text}");
        assert!(text.contains("cleanup incomplete 1/2"), "{text}");
    }
}
