//! Immutable evidence of one input-submission sequence.
//!
//! Every admitted sequence produces exactly one receipt. Native submission and
//! application effect are separate facts: a receipt records routes, submission
//! thresholds, partial native effect, faults, and cleanup, but never claims that
//! the target application consumed an event or changed visual state.

use std::fmt;

use mado_pilot_core::{
    InputAddressScope, InputDelivery, InputOperationKind, PermissionState, SubmissionEvidence,
    TargetId,
};

use crate::fault::InputFault;

/// How far an admitted sequence got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SequenceOutcome {
    /// Every logical event reached the selected route's submission threshold.
    Complete,
    /// Some input may have native effect and then the sequence stopped.
    ///
    /// `submitted` counts only complete logical events. This outcome may accompany
    /// zero submitted events when part of the first event's native representation
    /// may have had an effect.
    Partial,
    /// No event or partial native representation may have had an effect.
    Unexecuted,
}

impl SequenceOutcome {
    /// Returns a stable lowercase slug, for diagnostics and the C ABI mapping.
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
    /// A button or modifier may still be held. Repeating the same release is
    /// unlikely to help because the platform refused it.
    Incomplete,
    /// Cleanup stopped at its own bound with state still held.
    ///
    /// Distinct from [`CleanupState::Incomplete`]: these releases were not
    /// attempted, so a caller may choose to submit them itself.
    Exhausted,
}

impl CleanupState {
    /// Reports whether anything the sequence pressed may still be held.
    #[must_use]
    pub const fn may_leave_state_held(self) -> bool {
        matches!(self, CleanupState::Incomplete | CleanupState::Exhausted)
    }

    /// Returns a stable lowercase slug, for diagnostics and the C ABI mapping.
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

/// One immutable route attempt in caller order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InputAttempt {
    route: InputDelivery,
    address_scope: InputAddressScope,
    outcome: SequenceOutcome,
    submitted: usize,
    last_submitted: Option<usize>,
    evidence: Option<SubmissionEvidence>,
    partial_native_effect: bool,
    fault: Option<InputFault>,
}

impl InputAttempt {
    /// Records a route refused before any native effect was possible.
    #[must_use]
    pub const fn refused(route: InputDelivery, fault: InputFault) -> Self {
        Self {
            route,
            address_scope: route.address_scope(),
            outcome: SequenceOutcome::Unexecuted,
            submitted: 0,
            last_submitted: None,
            evidence: None,
            partial_native_effect: false,
            fault: Some(fault),
        }
    }

    const fn complete(
        route: InputDelivery,
        evidence: SubmissionEvidence,
        submitted: usize,
    ) -> Self {
        Self {
            route,
            address_scope: route.address_scope(),
            outcome: SequenceOutcome::Complete,
            submitted,
            last_submitted: submitted.checked_sub(1),
            evidence: Some(evidence),
            partial_native_effect: false,
            fault: None,
        }
    }

    const fn partial(
        route: InputDelivery,
        evidence: SubmissionEvidence,
        submitted: usize,
        partial_native_effect: bool,
        fault: InputFault,
    ) -> Self {
        Self {
            route,
            address_scope: route.address_scope(),
            outcome: SequenceOutcome::Partial,
            submitted,
            last_submitted: submitted.checked_sub(1),
            evidence: Some(evidence),
            partial_native_effect,
            fault: Some(fault),
        }
    }

    /// Returns the attempted route.
    #[must_use]
    pub const fn route(self) -> InputDelivery {
        self.route
    }

    /// Returns what the attempted route addresses.
    #[must_use]
    pub const fn address_scope(self) -> InputAddressScope {
        self.address_scope
    }

    /// Returns the terminal state of this route attempt.
    #[must_use]
    pub const fn outcome(self) -> SequenceOutcome {
        self.outcome
    }

    /// Returns how many complete logical events reached the route threshold.
    #[must_use]
    pub const fn submitted(self) -> usize {
        self.submitted
    }

    /// Returns the last complete logical-event index submitted on this route.
    #[must_use]
    pub const fn last_submitted(self) -> Option<usize> {
        self.last_submitted
    }

    /// Returns the strongest native evidence obtained by this attempt.
    #[must_use]
    pub const fn evidence(self) -> Option<SubmissionEvidence> {
        self.evidence
    }

    /// Reports whether the current incomplete logical event may have native
    /// effect.
    #[must_use]
    pub const fn partial_native_effect(self) -> bool {
        self.partial_native_effect
    }

    /// Reports whether any part of this attempt may have had native effect.
    #[must_use]
    pub const fn possible_native_effect(self) -> bool {
        self.submitted != 0 || self.partial_native_effect
    }

    /// Returns the route-local terminal fault.
    #[must_use]
    pub const fn fault(self) -> Option<InputFault> {
        self.fault
    }
}

/// Result of the retained target/process revalidation for one route event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum InputRevalidationCategory {
    /// Retained authority and the one-recipient rule passed.
    Passed,
    /// The retained target or owning process no longer matched.
    TargetLost,
    /// More than one eligible recipient made process-directed delivery ambiguous.
    Ambiguous,
    /// The operation ended before the final irreversible boundary.
    Interrupted,
    /// No authoritative classification was available.
    Unavailable,
}

/// Result of applying the request's geometry policy at one route event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum InputGeometryResult {
    /// The event has no coordinate or geometry dependency.
    NotApplicable,
    /// Authoritative current geometry matched the prepared geometry.
    Passed,
    /// Current geometry no longer matched the prepared geometry.
    Changed,
    /// Geometry could not be evaluated before another gate refused the event.
    NotEvaluated,
}

/// Privacy-reviewed debug facts for one logical event at a route boundary.
///
/// The record contains no event payload, process identifier, native window
/// number, title, signing identity, cursor location, or native framework type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InputEventObservation {
    /// Attempted delivery route.
    pub route: InputDelivery,
    /// Zero-based logical-event index in the caller's sequence.
    pub event_index: u64,
    /// Payload-free operation category.
    pub operation: InputOperationKind,
    /// Retained authority result.
    pub revalidation: InputRevalidationCategory,
    /// Bounded eligible-recipient count, when authority produced one.
    pub candidate_count: Option<u32>,
    /// Public authorization state observed at the final native gate.
    pub authorization: PermissionState,
    /// Geometry-policy result.
    pub geometry: InputGeometryResult,
    /// Native units required for the logical event.
    pub expected_native_units: u64,
    /// Native units whose invocation returned.
    pub invoked_native_units: u64,
    /// Typed terminal fault for this event, if it stopped the route.
    pub fault: Option<InputFault>,
}

/// One input receipt plus optional privacy-reviewed route observations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputExecution {
    receipt: InputReceipt,
    observations: Vec<InputEventObservation>,
}

impl InputExecution {
    /// Creates an execution result from its terminal receipt and debug facts.
    #[must_use]
    pub fn new(receipt: InputReceipt, observations: Vec<InputEventObservation>) -> Self {
        Self {
            receipt,
            observations,
        }
    }

    /// Returns the immutable terminal receipt.
    #[must_use]
    pub const fn receipt(&self) -> &InputReceipt {
        &self.receipt
    }

    /// Returns the privacy-reviewed per-event observations in commit order.
    #[must_use]
    pub fn observations(&self) -> &[InputEventObservation] {
        &self.observations
    }

    /// Separates the caller-owned receipt and observation storage.
    #[must_use]
    pub fn into_parts(self) -> (InputReceipt, Vec<InputEventObservation>) {
        (self.receipt, self.observations)
    }
}

/// The one truthful immutable account of an admitted sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputReceipt {
    target: TargetId,
    outcome: SequenceOutcome,
    selected_route: Option<InputDelivery>,
    address_scope: Option<InputAddressScope>,
    attempts: Vec<InputAttempt>,
    submitted: usize,
    last_submitted: Option<usize>,
    evidence: Option<SubmissionEvidence>,
    partial_native_effect: bool,
    fault: Option<InputFault>,
    cleanup: CleanupState,
    cleanup_released: usize,
    cleanup_owed: usize,
}

impl InputReceipt {
    /// Records a sequence whose every logical event reached `route`'s threshold.
    #[must_use]
    pub fn complete(
        target: TargetId,
        route: InputDelivery,
        evidence: SubmissionEvidence,
        submitted: usize,
    ) -> Self {
        Self::from_terminal_attempt(target, InputAttempt::complete(route, evidence, submitted))
    }

    /// Records a sequence that stopped after native effect was possible.
    ///
    /// `submitted` counts only complete logical events. `partial_native_effect`
    /// describes the current incomplete logical event independently of that count.
    #[must_use]
    pub fn partial(
        target: TargetId,
        route: InputDelivery,
        evidence: SubmissionEvidence,
        submitted: usize,
        partial_native_effect: bool,
        fault: InputFault,
    ) -> Self {
        Self::from_terminal_attempt(
            target,
            InputAttempt::partial(route, evidence, submitted, partial_native_effect, fault),
        )
    }

    /// Records a sequence for which no native effect was possible.
    #[must_use]
    pub fn unexecuted(target: TargetId, fault: InputFault) -> Self {
        Self {
            target,
            outcome: SequenceOutcome::Unexecuted,
            selected_route: None,
            address_scope: None,
            attempts: Vec::new(),
            submitted: 0,
            last_submitted: None,
            evidence: None,
            partial_native_effect: false,
            fault: Some(fault),
            cleanup: CleanupState::NotNeeded,
            cleanup_released: 0,
            cleanup_owed: 0,
        }
    }

    fn from_terminal_attempt(target: TargetId, attempt: InputAttempt) -> Self {
        Self {
            target,
            outcome: attempt.outcome(),
            selected_route: Some(attempt.route()),
            address_scope: Some(attempt.address_scope()),
            attempts: vec![attempt],
            submitted: attempt.submitted(),
            last_submitted: attempt.last_submitted(),
            evidence: attempt.evidence(),
            partial_native_effect: attempt.partial_native_effect(),
            fault: attempt.fault(),
            cleanup: CleanupState::NotNeeded,
            cleanup_released: 0,
            cleanup_owed: 0,
        }
    }

    /// Prepends routes refused before this receipt's terminal attempt.
    ///
    /// A fallback is legal only while every preceding attempt proves no possible
    /// native effect. Callers construct those records with [`InputAttempt::refused`].
    #[must_use]
    pub fn with_prior_attempts(mut self, mut attempts: Vec<InputAttempt>) -> Self {
        debug_assert!(
            attempts
                .iter()
                .all(|attempt| !attempt.possible_native_effect()),
            "fallback cannot follow possible native effect"
        );
        attempts.append(&mut self.attempts);
        self.attempts = attempts;
        self
    }

    /// Records what cleanup released out of what it owned.
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

    /// Records cleanup stopped by its own event or time bound.
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

    /// Returns the route on which native effect became possible.
    #[must_use]
    pub const fn selected_route(&self) -> Option<InputDelivery> {
        self.selected_route
    }

    /// Returns what the selected route addressed.
    #[must_use]
    pub const fn address_scope(&self) -> Option<InputAddressScope> {
        self.address_scope
    }

    /// Returns every visited route attempt in caller order.
    #[must_use]
    pub fn attempts(&self) -> &[InputAttempt] {
        &self.attempts
    }

    /// Reports whether the sequence used a route after the caller's first visited
    /// route.
    #[must_use]
    pub fn used_fallback(&self) -> bool {
        match (self.attempts.first(), self.selected_route) {
            (Some(first), Some(selected)) => first.route() != selected,
            _ => false,
        }
    }

    /// Returns how many complete logical events reached the selected threshold.
    #[must_use]
    pub const fn submitted(&self) -> usize {
        self.submitted
    }

    /// Returns the last complete logical-event index submitted.
    #[must_use]
    pub const fn last_submitted(&self) -> Option<usize> {
        self.last_submitted
    }

    /// Returns the strongest evidence obtained on the selected route.
    #[must_use]
    pub const fn evidence(&self) -> Option<SubmissionEvidence> {
        self.evidence
    }

    /// Reports whether the current incomplete logical event may have native
    /// effect.
    #[must_use]
    pub const fn partial_native_effect(&self) -> bool {
        self.partial_native_effect
    }

    /// Reports whether any complete or partial logical event may have had native
    /// effect.
    #[must_use]
    pub const fn possible_native_effect(&self) -> bool {
        self.submitted != 0 || self.partial_native_effect
    }

    /// Returns why the sequence stopped, for anything but a complete one.
    #[must_use]
    pub const fn fault(&self) -> Option<InputFault> {
        self.fault
    }

    /// Returns what became of state the sequence pressed.
    #[must_use]
    pub const fn cleanup(&self) -> CleanupState {
        self.cleanup
    }

    /// Returns how many sequence-owned pressed states cleanup released.
    #[must_use]
    pub const fn cleanup_released(&self) -> usize {
        self.cleanup_released
    }

    /// Returns how many sequence-owned pressed states cleanup was responsible for.
    #[must_use]
    pub const fn cleanup_owed(&self) -> usize {
        self.cleanup_owed
    }

    /// Reports whether every logical event reached the selected route threshold.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        matches!(self.outcome, SequenceOutcome::Complete)
    }
}

impl fmt::Display for InputReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} {} submitted event(s)",
            self.outcome, self.submitted
        )?;
        if let Some(route) = self.selected_route {
            write!(formatter, " via {route}")?;
        }
        if let Some(evidence) = self.evidence {
            write!(formatter, " ({evidence})")?;
        }
        if let Some(fault) = self.fault {
            write!(formatter, ": {fault}")?;
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
    use super::{CleanupState, InputAttempt, InputReceipt, SequenceOutcome};
    use crate::fault::InputFault;
    use mado_pilot_core::{
        IdentityIssuer, InputAddressScope, InputDelivery, ProviderId, SubmissionEvidence, TargetId,
    };

    fn target() -> TargetId {
        IdentityIssuer::new()
            .issue_target(ProviderId::new("fake"))
            .expect("issued")
    }

    fn partial(submitted: usize, partial_native_effect: bool) -> InputReceipt {
        InputReceipt::partial(
            target(),
            InputDelivery::System,
            SubmissionEvidence::SystemInputAdmission,
            submitted,
            partial_native_effect,
            InputFault::SubmissionFailed,
        )
    }

    #[test]
    fn complete_submission_records_route_scope_evidence_and_last_event() {
        let receipt = InputReceipt::complete(
            target(),
            InputDelivery::System,
            SubmissionEvidence::SystemInputAdmission,
            3,
        );

        assert!(receipt.is_complete());
        assert_eq!(receipt.outcome(), SequenceOutcome::Complete);
        assert_eq!(receipt.selected_route(), Some(InputDelivery::System));
        assert_eq!(
            receipt.address_scope(),
            Some(InputAddressScope::FocusedSystem)
        );
        assert_eq!(receipt.submitted(), 3);
        assert_eq!(receipt.last_submitted(), Some(2));
        assert_eq!(
            receipt.evidence(),
            Some(SubmissionEvidence::SystemInputAdmission)
        );
        assert!(!receipt.partial_native_effect());
        assert_eq!(receipt.fault(), None);
        assert_eq!(receipt.cleanup(), CleanupState::NotNeeded);
    }

    #[test]
    fn zero_count_partial_submission_preserves_possible_native_effect() {
        let receipt = partial(0, true);

        assert_eq!(receipt.outcome(), SequenceOutcome::Partial);
        assert_eq!(receipt.submitted(), 0);
        assert_eq!(receipt.last_submitted(), None);
        assert!(receipt.partial_native_effect());
        assert!(receipt.attempts()[0].possible_native_effect());
    }

    #[test]
    fn unexecuted_refusal_has_no_selected_route_or_evidence() {
        let receipt = InputReceipt::unexecuted(target(), InputFault::ControllerClosed)
            .with_prior_attempts(vec![InputAttempt::refused(
                InputDelivery::WindowMessage,
                InputFault::ControllerClosed,
            )]);

        assert_eq!(receipt.outcome(), SequenceOutcome::Unexecuted);
        assert_eq!(receipt.submitted(), 0);
        assert_eq!(receipt.last_submitted(), None);
        assert_eq!(receipt.selected_route(), None);
        assert_eq!(receipt.address_scope(), None);
        assert_eq!(receipt.evidence(), None);
        assert_eq!(receipt.attempts().len(), 1);
        assert!(!receipt.attempts()[0].possible_native_effect());
        assert!(!receipt.used_fallback());
    }

    #[test]
    fn multi_route_preflight_is_immutable_and_ordered() {
        let receipt = InputReceipt::complete(
            target(),
            InputDelivery::System,
            SubmissionEvidence::SystemInputAdmission,
            1,
        )
        .with_prior_attempts(vec![
            InputAttempt::refused(InputDelivery::WindowMessage, InputFault::RouteUnavailable),
            InputAttempt::refused(
                InputDelivery::ProcessDirected,
                InputFault::UnsupportedCombination,
            ),
        ]);
        let retained = receipt.clone();
        drop(receipt);

        assert!(retained.used_fallback());
        assert_eq!(
            retained
                .attempts()
                .iter()
                .map(|attempt| attempt.route())
                .collect::<Vec<_>>(),
            [
                InputDelivery::WindowMessage,
                InputDelivery::ProcessDirected,
                InputDelivery::System,
            ]
        );
        assert_eq!(
            retained.attempts()[2].evidence(),
            Some(SubmissionEvidence::SystemInputAdmission)
        );
    }

    #[test]
    fn cleanup_state_follows_from_exact_counts() {
        let owed_nothing = partial(1, false).with_cleanup(0, 0);
        let released_all = partial(1, false).with_cleanup(2, 2);
        let incomplete = partial(1, false).with_cleanup(1, 2);
        let exhausted = partial(1, false).with_exhausted_cleanup(1, 2);

        assert_eq!(owed_nothing.cleanup(), CleanupState::NotNeeded);
        assert_eq!(released_all.cleanup(), CleanupState::Complete);
        assert_eq!(incomplete.cleanup(), CleanupState::Incomplete);
        assert_eq!(exhausted.cleanup(), CleanupState::Exhausted);
        assert!(incomplete.cleanup().may_leave_state_held());
        assert!(exhausted.cleanup().may_leave_state_held());
        assert_eq!(exhausted.cleanup_released(), 1);
        assert_eq!(exhausted.cleanup_owed(), 2);
    }

    #[test]
    fn exhausted_cleanup_is_complete_when_every_release_finished() {
        let receipt = partial(1, false).with_exhausted_cleanup(2, 2);

        assert_eq!(receipt.cleanup(), CleanupState::Complete);
        assert!(!receipt.cleanup().may_leave_state_held());
    }

    #[test]
    fn receipt_display_uses_submission_not_application_effect_language() {
        let receipt = partial(2, false).with_cleanup(1, 2);
        let text = receipt.to_string();

        assert!(
            text.contains("partial 2 submitted event(s) via system (system_input_admission)"),
            "{text}"
        );
        assert!(text.contains("cleanup incomplete 1/2"), "{text}");
        assert!(!text.contains("delivered"), "{text}");
    }
}
