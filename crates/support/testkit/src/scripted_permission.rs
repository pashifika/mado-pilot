//! A permission probe that answers from a script.
//!
//! Authorization is the one capture and input precondition a test cannot arrange
//! on a real host: granting Screen Recording needs a human, and revoking it needs
//! the same human at a specific moment. This double supplies the answer instead,
//! and records what it was asked, so a test can assert both the behavior under
//! each state and that nothing consulted a permission it had no business reading.

use std::fmt;
use std::sync::Mutex;

use mado_pilot_core::{
    DiagnosticCategory, Operation, OperationContext, PermissionKind, PermissionOutcome,
    PermissionProbe, PermissionState, ProviderId, RedactedDiagnostic, Result,
};

/// Provider name qualifying this double's answers.
pub const PROVIDER: ProviderId = ProviderId::new("scripted");

/// What the probe should answer for one permission kind.
#[derive(Debug, Clone)]
pub enum Answer {
    /// Report `state`, with no diagnostic.
    State(PermissionState),
    /// Report `state` with a redacted diagnostic.
    Explained(PermissionState, RedactedDiagnostic),
    /// Fail the read itself, as a platform service that is unavailable does.
    Fail(mado_pilot_core::Error),
    /// Report each state in turn, repeating the last one once exhausted.
    ///
    /// This is how a revocation is scripted: granted first, then not granted, with
    /// the operation under test in between.
    Sequence(Vec<PermissionState>),
}

impl Answer {
    /// Answers `Granted`.
    #[must_use]
    pub const fn granted() -> Self {
        Answer::State(PermissionState::Granted)
    }

    /// Answers `NotGranted`.
    #[must_use]
    pub const fn not_granted() -> Self {
        Answer::State(PermissionState::NotGranted)
    }

    /// Answers `Unavailable`, as a platform with no such concept does.
    #[must_use]
    pub const fn unavailable() -> Self {
        Answer::State(PermissionState::Unavailable)
    }

    /// Answers `Unknown` with the undetermined diagnostic a real probe records.
    #[must_use]
    pub const fn undetermined() -> Self {
        Answer::Explained(
            PermissionState::Unknown,
            RedactedDiagnostic::new(DiagnosticCategory::PermissionUndetermined)
                .with_context("the scripted probe was told to establish nothing"),
        )
    }
}

/// A [`PermissionProbe`] whose answers a test writes.
pub struct ScriptedPermissionProbe {
    provider: ProviderId,
    capture: Mutex<Answer>,
    input: Mutex<Answer>,
    reads: Mutex<Vec<PermissionKind>>,
    positions: Mutex<Vec<(PermissionKind, usize)>>,
}

impl ScriptedPermissionProbe {
    /// Builds a probe answering `capture` and `input`.
    #[must_use]
    pub fn new(capture: Answer, input: Answer) -> Self {
        Self {
            provider: PROVIDER,
            capture: Mutex::new(capture),
            input: Mutex::new(input),
            reads: Mutex::new(Vec::new()),
            positions: Mutex::new(Vec::new()),
        }
    }

    /// Builds a probe that reports both capabilities as authorized.
    #[must_use]
    pub fn granting() -> Self {
        Self::new(Answer::granted(), Answer::granted())
    }

    /// Answers for a provider other than the default, for pairing tests.
    #[must_use]
    pub fn for_provider(mut self, provider: ProviderId) -> Self {
        self.provider = provider;
        self
    }

    /// Replaces the answer for `kind`, as a permission change would.
    pub fn set(&self, kind: PermissionKind, answer: Answer) {
        let slot = match kind {
            PermissionKind::ScreenCapture => &self.capture,
            PermissionKind::InputControl => &self.input,
            // A kind this build does not know about cannot be scripted, and
            // silently scripting the wrong one would make a test pass for the
            // wrong reason.
            _ => return,
        };
        *slot.lock().expect("uncontended") = answer;
        self.positions
            .lock()
            .expect("uncontended")
            .retain(|(recorded, _)| *recorded != kind);
    }

    /// Returns every permission kind that was read, in order.
    ///
    /// A test asserts on this to show that an operation consulted the
    /// authorization it needed and no other.
    #[must_use]
    pub fn reads(&self) -> Vec<PermissionKind> {
        self.reads.lock().expect("uncontended").clone()
    }

    /// Returns how many times `kind` was read.
    #[must_use]
    pub fn read_count(&self, kind: PermissionKind) -> usize {
        self.reads()
            .into_iter()
            .filter(|recorded| *recorded == kind)
            .count()
    }

    fn next_state(&self, kind: PermissionKind, states: &[PermissionState]) -> PermissionState {
        let mut positions = self.positions.lock().expect("uncontended");
        let position = match positions.iter_mut().find(|(recorded, _)| *recorded == kind) {
            Some((_, position)) => position,
            None => {
                positions.push((kind, 0));
                &mut positions.last_mut().expect("just pushed").1
            }
        };
        let state = states
            .get(*position)
            .copied()
            .or_else(|| states.last().copied())
            .unwrap_or(PermissionState::Unknown);
        *position = position.saturating_add(1);
        state
    }
}

impl fmt::Debug for ScriptedPermissionProbe {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScriptedPermissionProbe")
            .field("provider", &self.provider)
            .field("reads", &self.reads().len())
            .finish()
    }
}

impl PermissionProbe for ScriptedPermissionProbe {
    fn provider(&self) -> ProviderId {
        self.provider
    }

    fn probe(
        &self,
        kind: PermissionKind,
        operation: &OperationContext,
    ) -> Result<PermissionOutcome> {
        let attempt = Operation::admit(operation)?;
        self.reads.lock().expect("uncontended").push(kind);

        let answer = match kind {
            PermissionKind::ScreenCapture => self.capture.lock().expect("uncontended").clone(),
            PermissionKind::InputControl => self.input.lock().expect("uncontended").clone(),
            _ => Answer::undetermined(),
        };
        let outcome = match answer {
            Answer::State(state) => PermissionOutcome::new(kind, state),
            Answer::Explained(state, diagnostic) => {
                PermissionOutcome::new(kind, state).with_diagnostic(diagnostic)
            }
            Answer::Fail(error) => return Err(error),
            Answer::Sequence(states) => {
                PermissionOutcome::new(kind, self.next_state(kind, &states))
            }
        };
        Ok(attempt.commit(outcome)?)
    }
}

#[cfg(test)]
mod tests {
    use super::{Answer, ScriptedPermissionProbe};
    use mado_pilot_core::{
        CancellationToken, DiagnosticCategory, Error, OperationContext, PermissionKind,
        PermissionProbe, PermissionState, Status,
    };

    #[test]
    fn a_report_reads_each_kind_once() {
        let probe = ScriptedPermissionProbe::new(Answer::granted(), Answer::not_granted());

        let report = probe.report(&OperationContext::new()).expect("read");

        assert!(report.capture().is_granted());
        assert!(!report.input().is_granted());
        assert_eq!(probe.reads(), PermissionKind::ALL.to_vec());
        assert_eq!(probe.read_count(PermissionKind::ScreenCapture), 1);
    }

    #[test]
    fn a_revocation_is_scripted_as_a_sequence() {
        let probe = ScriptedPermissionProbe::new(
            Answer::Sequence(vec![PermissionState::Granted, PermissionState::NotGranted]),
            Answer::unavailable(),
        );
        let context = OperationContext::new();

        let before = probe
            .probe(PermissionKind::ScreenCapture, &context)
            .expect("read");
        let after = probe
            .probe(PermissionKind::ScreenCapture, &context)
            .expect("read");
        let still_after = probe
            .probe(PermissionKind::ScreenCapture, &context)
            .expect("read");

        assert_eq!(before.state(), PermissionState::Granted);
        assert_eq!(after.state(), PermissionState::NotGranted);
        assert_eq!(
            still_after.state(),
            PermissionState::NotGranted,
            "the last scripted state repeats rather than wrapping"
        );
    }

    #[test]
    fn a_state_can_be_replaced_between_operations() {
        let probe = ScriptedPermissionProbe::granting();
        let context = OperationContext::new();
        assert!(
            probe
                .probe(PermissionKind::InputControl, &context)
                .expect("read")
                .is_granted()
        );

        probe.set(PermissionKind::InputControl, Answer::not_granted());

        assert!(
            !probe
                .probe(PermissionKind::InputControl, &context)
                .expect("read")
                .is_granted()
        );
    }

    #[test]
    fn an_undetermined_answer_carries_its_category() {
        let probe = ScriptedPermissionProbe::new(Answer::undetermined(), Answer::granted());

        let outcome = probe
            .probe(PermissionKind::ScreenCapture, &OperationContext::new())
            .expect("read");

        assert_eq!(outcome.state(), PermissionState::Unknown);
        assert_eq!(
            outcome.diagnostic().map(|diagnostic| diagnostic.category()),
            Some(DiagnosticCategory::PermissionUndetermined)
        );
    }

    #[test]
    fn a_failing_probe_reports_its_error() {
        let probe = ScriptedPermissionProbe::new(
            Answer::Fail(Error::new(Status::Internal, "service unavailable")),
            Answer::granted(),
        );

        let error = probe
            .report(&OperationContext::new())
            .expect_err("the read failed");

        assert_eq!(error.status(), Status::Internal);
    }

    #[test]
    fn a_cancelled_probe_reads_nothing() {
        let probe = ScriptedPermissionProbe::granting();
        let token = CancellationToken::new();
        token.cancel();
        let cancelled = OperationContext::new().with_cancellation(token);

        let error = probe.report(&cancelled).expect_err("cancelled");

        assert_eq!(error.status(), Status::Cancelled);
        assert!(probe.reads().is_empty());
    }
}
