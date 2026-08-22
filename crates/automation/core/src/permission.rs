//! Non-prompting authorization probes for the two sensitive capabilities.
//!
//! Screen capture and input control are authorized separately by both release
//! targets, so they are reported separately here. A probe answers what the
//! operating system will already tell it: it never calls a permission-request
//! API, presents a dialog, opens system settings, shows a target picker, or
//! elevates the process. A caller that wants a permission asks the user through
//! its own interface, because MadoPilot has no interface to ask through.
//!
//! An authorization is not a promise. Permission can be revoked between a probe
//! and the operation it was meant to clear, so every capture and input operation
//! still reports its own typed outcome.

use std::fmt;
use std::fmt::Debug;

use crate::diagnostic::RedactedDiagnostic;
use crate::identity::ProviderId;
use crate::operation::OperationContext;
use crate::status::Result;

/// Which sensitive capability an authorization applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum PermissionKind {
    /// Reading the contents of windows and displays.
    ScreenCapture,
    /// Delivering pointer, keyboard, or text input.
    InputControl,
}

impl PermissionKind {
    /// Every permission kind version one knows about.
    ///
    /// A report covers both, so the list exists once here rather than in each
    /// caller that has to iterate it.
    pub const ALL: [Self; 2] = [PermissionKind::ScreenCapture, PermissionKind::InputControl];

    /// Returns a stable lowercase slug, for logs and the C ABI mapping.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            PermissionKind::ScreenCapture => "screen_capture",
            PermissionKind::InputControl => "input_control",
        }
    }
}

impl fmt::Display for PermissionKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// What a non-prompting probe was able to establish.
///
/// The four states are not a scale. `Unavailable` and `Unknown` are different
/// answers to different questions — one says the platform has no such
/// authorization concept, the other says this platform has one and the probe
/// could not read it — and neither of them means an operation will succeed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PermissionState {
    /// The operating system authorizes the capability for this process.
    Granted,
    /// The operating system withholds it.
    NotGranted,
    /// This platform or build has no corresponding authorization to grant. The
    /// operation may still be refused by a capability, policy, or focus rule.
    Unavailable,
    /// The probe could not establish the state without prompting, which it will
    /// not do.
    Unknown,
}

impl PermissionState {
    /// Reports whether the operating system has authorized the capability.
    ///
    /// Only [`PermissionState::Granted`] is authorization. This is a method
    /// rather than a comparison so that a state added later cannot silently
    /// become authorization at a call site that matched everything else.
    #[must_use]
    pub const fn is_granted(self) -> bool {
        matches!(self, PermissionState::Granted)
    }

    /// Reports whether the state establishes that the capability is refused.
    ///
    /// `Unavailable` and `Unknown` are not refusals: an operation attempted
    /// under either may still succeed, and one attempted under `Granted` may
    /// still fail.
    #[must_use]
    pub const fn is_refused(self) -> bool {
        matches!(self, PermissionState::NotGranted)
    }

    /// Returns a stable lowercase slug, for logs and the C ABI mapping.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            PermissionState::Granted => "granted",
            PermissionState::NotGranted => "not_granted",
            PermissionState::Unavailable => "unavailable",
            PermissionState::Unknown => "unknown",
        }
    }
}

impl fmt::Display for PermissionState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One permission kind, its state, and why the probe reached that state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PermissionOutcome {
    kind: PermissionKind,
    state: PermissionState,
    diagnostic: Option<RedactedDiagnostic>,
}

impl PermissionOutcome {
    /// Records `state` for `kind` with no further diagnostic.
    #[must_use]
    pub const fn new(kind: PermissionKind, state: PermissionState) -> Self {
        Self {
            kind,
            state,
            diagnostic: None,
        }
    }

    /// Adds the redacted diagnostic that explains the state.
    #[must_use]
    pub const fn with_diagnostic(mut self, diagnostic: RedactedDiagnostic) -> Self {
        self.diagnostic = Some(diagnostic);
        self
    }

    /// Returns the permission kind this outcome describes.
    #[must_use]
    pub const fn kind(self) -> PermissionKind {
        self.kind
    }

    /// Returns what the probe established.
    #[must_use]
    pub const fn state(self) -> PermissionState {
        self.state
    }

    /// Returns the redacted diagnostic, when the Adapter recorded one.
    #[must_use]
    pub const fn diagnostic(self) -> Option<RedactedDiagnostic> {
        self.diagnostic
    }

    /// Reports whether the operating system has authorized this capability.
    #[must_use]
    pub const fn is_granted(self) -> bool {
        self.state.is_granted()
    }
}

impl fmt::Display for PermissionOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} {}", self.kind, self.state)?;
        if let Some(diagnostic) = self.diagnostic {
            write!(formatter, " [{diagnostic}]")?;
        }
        Ok(())
    }
}

/// Both authorization outcomes, as one probe read them.
///
/// The two are carried side by side rather than reduced to one answer: a target
/// whose capture is authorized and whose input is not is an ordinary,
/// usable state, and a combined verdict would hide it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PermissionReport {
    capture: PermissionOutcome,
    input: PermissionOutcome,
}

impl PermissionReport {
    /// Assembles a report from the two independent outcomes.
    ///
    /// # Panics
    ///
    /// Panics when either outcome describes the wrong permission kind. A report
    /// whose fields disagree with their own labels would make every accessor
    /// below lie, and the argument order is a programming mistake in the
    /// Adapter rather than a runtime condition a caller could handle.
    #[must_use]
    pub const fn new(capture: PermissionOutcome, input: PermissionOutcome) -> Self {
        assert!(
            matches!(capture.kind, PermissionKind::ScreenCapture),
            "the first outcome must describe screen capture"
        );
        assert!(
            matches!(input.kind, PermissionKind::InputControl),
            "the second outcome must describe input control"
        );
        Self { capture, input }
    }

    /// Returns the screen-capture outcome.
    #[must_use]
    pub const fn capture(self) -> PermissionOutcome {
        self.capture
    }

    /// Returns the input-control outcome.
    #[must_use]
    pub const fn input(self) -> PermissionOutcome {
        self.input
    }

    /// Returns the outcome for `kind`.
    ///
    /// Every kind is matched rather than defaulted. A permission kind added
    /// later makes this a compile error inside this package, which is where the
    /// decision about what a report says for it belongs; a wildcard arm would
    /// answer for it with whichever state happened to be convenient.
    #[must_use]
    pub const fn outcome(self, kind: PermissionKind) -> PermissionOutcome {
        match kind {
            PermissionKind::ScreenCapture => self.capture,
            PermissionKind::InputControl => self.input,
        }
    }
}

impl fmt::Display for PermissionReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}, {}", self.capture, self.input)
    }
}

/// Reads authorization states without asking the user for anything.
///
/// Implemented by platform Adapters and by test doubles. The contract is as much
/// about what an implementation may not do as what it returns: no
/// permission-request API, no dialog, no settings window, no picker, and no
/// privilege escalation. An implementation that cannot answer returns
/// [`PermissionState::Unknown`], which is the honest result of refusing to
/// prompt.
///
/// The trait offers no operation that could request access, which is why the rule
/// is expressed as a trait and not as a policy flag on a request: there is
/// nothing here for a caller to switch on, and an Adapter that prompted would be
/// doing so outside the contract rather than through it.
///
/// Every read carries the caller's operation context, so a probe that has to
/// consult a slow platform service is interruptible like any other blocking
/// operation.
pub trait PermissionProbe: Debug + Send + Sync {
    /// Returns the provider whose authorization this probe reads.
    ///
    /// Wiring compares this against the capture and input providers, so a probe
    /// cannot report one platform's authorization for another platform's target.
    fn provider(&self) -> ProviderId;

    /// Reads the current state of `kind`.
    ///
    /// # Errors
    ///
    /// Returns the operation's terminal outcome when cancellation or the
    /// deadline wins, and a platform failure when the probe itself could not
    /// run. A probe that ran and could not establish the state succeeds with
    /// [`PermissionState::Unknown`] instead of failing.
    fn probe(
        &self,
        kind: PermissionKind,
        operation: &OperationContext,
    ) -> Result<PermissionOutcome>;

    /// Reads both permission kinds.
    ///
    /// The default reads them one at a time in [`PermissionKind::ALL`] order.
    /// An Adapter overrides this only when one native call answers both, and
    /// must keep the two outcomes independent when it does.
    ///
    /// # Errors
    ///
    /// As [`PermissionProbe::probe`]. The whole report fails when either read
    /// fails, because a half report would be indistinguishable from a target
    /// that genuinely has one authorization.
    fn report(&self, operation: &OperationContext) -> Result<PermissionReport> {
        Ok(PermissionReport::new(
            self.probe(PermissionKind::ScreenCapture, operation)?,
            self.probe(PermissionKind::InputControl, operation)?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::{
        PermissionKind, PermissionOutcome, PermissionProbe, PermissionReport, PermissionState,
    };
    use crate::diagnostic::{DiagnosticCategory, PlatformCode, RedactedDiagnostic};
    use crate::identity::ProviderId;
    use crate::operation::{CancellationToken, Operation, OperationContext};
    use crate::status::{Error, Result, Status};

    const FAKE: ProviderId = ProviderId::new("fake");

    /// A probe that answers from a script and records what it was asked.
    #[derive(Debug)]
    struct ScriptedProbe {
        capture: Result<PermissionState>,
        input: Result<PermissionState>,
        asked: Mutex<Vec<PermissionKind>>,
    }

    impl ScriptedProbe {
        fn new(capture: Result<PermissionState>, input: Result<PermissionState>) -> Self {
            Self {
                capture,
                input,
                asked: Mutex::new(Vec::new()),
            }
        }

        fn asked(&self) -> Vec<PermissionKind> {
            self.asked.lock().expect("uncontended").clone()
        }
    }

    impl PermissionProbe for ScriptedProbe {
        fn provider(&self) -> ProviderId {
            FAKE
        }

        fn probe(
            &self,
            kind: PermissionKind,
            operation: &OperationContext,
        ) -> Result<PermissionOutcome> {
            let attempt = Operation::admit(operation)?;
            self.asked.lock().expect("uncontended").push(kind);
            let state = match kind {
                PermissionKind::ScreenCapture => self.capture.clone()?,
                PermissionKind::InputControl => self.input.clone()?,
            };
            Ok(attempt.commit(PermissionOutcome::new(kind, state))?)
        }
    }

    #[test]
    fn only_granted_is_authorization() {
        assert!(PermissionState::Granted.is_granted());
        for state in [
            PermissionState::NotGranted,
            PermissionState::Unavailable,
            PermissionState::Unknown,
        ] {
            assert!(!state.is_granted(), "{state}");
        }
    }

    #[test]
    fn an_absent_permission_concept_is_not_a_refusal() {
        assert!(PermissionState::NotGranted.is_refused());
        assert!(
            !PermissionState::Unavailable.is_refused(),
            "a platform without the concept has refused nothing"
        );
        assert!(!PermissionState::Unknown.is_refused());
    }

    #[test]
    fn a_report_keeps_the_two_authorizations_independent() {
        let report = PermissionReport::new(
            PermissionOutcome::new(PermissionKind::ScreenCapture, PermissionState::Granted),
            PermissionOutcome::new(PermissionKind::InputControl, PermissionState::NotGranted),
        );

        assert!(report.capture().is_granted());
        assert!(!report.input().is_granted());
        assert_eq!(
            report.outcome(PermissionKind::ScreenCapture),
            report.capture()
        );
        assert_eq!(report.outcome(PermissionKind::InputControl), report.input());
    }

    #[test]
    #[should_panic(expected = "must describe screen capture")]
    fn a_report_refuses_mislabelled_outcomes() {
        let _unused = PermissionReport::new(
            PermissionOutcome::new(PermissionKind::InputControl, PermissionState::Granted),
            PermissionOutcome::new(PermissionKind::InputControl, PermissionState::Granted),
        );
    }

    #[test]
    fn a_report_reads_both_kinds_and_keeps_the_answers_apart() {
        let probe = ScriptedProbe::new(
            Ok(PermissionState::Granted),
            Ok(PermissionState::NotGranted),
        );

        let report = probe
            .report(&OperationContext::new())
            .expect("both reads succeeded");

        assert_eq!(
            probe.asked(),
            PermissionKind::ALL.to_vec(),
            "each kind is read on its own"
        );
        assert!(report.capture().is_granted());
        assert!(report.input().state().is_refused());
    }

    #[test]
    fn a_platform_without_an_input_authorization_reports_it_as_unavailable() {
        let probe = ScriptedProbe::new(
            Ok(PermissionState::Granted),
            Ok(PermissionState::Unavailable),
        );

        let report = probe.report(&OperationContext::new()).expect("read");

        assert!(!report.input().is_granted());
        assert!(
            !report.input().state().is_refused(),
            "an absent concept has refused nothing, and promises nothing either"
        );
    }

    #[test]
    fn a_failed_read_fails_the_whole_report() {
        let probe = ScriptedProbe::new(
            Ok(PermissionState::Granted),
            Err(Error::new(Status::Internal, "probe service unavailable")),
        );

        let error = probe
            .report(&OperationContext::new())
            .expect_err("a half report would look like a real one");

        assert_eq!(error.status(), Status::Internal);
    }

    #[test]
    fn a_cancelled_probe_reads_nothing() {
        let probe = ScriptedProbe::new(Ok(PermissionState::Granted), Ok(PermissionState::Granted));
        let token = CancellationToken::new();
        token.cancel();
        let context = OperationContext::new().with_cancellation(token);

        let error = probe
            .report(&context)
            .expect_err("cancelled before admission");

        assert_eq!(error.status(), Status::Cancelled);
        assert!(
            probe.asked().is_empty(),
            "a cancelled probe does not consult the platform"
        );
    }

    #[test]
    fn an_undetermined_outcome_carries_a_redacted_reason() {
        let outcome =
            PermissionOutcome::new(PermissionKind::ScreenCapture, PermissionState::Unknown)
                .with_diagnostic(
                    RedactedDiagnostic::new(DiagnosticCategory::PermissionUndetermined)
                        .with_platform(PlatformCode::new("osstatus", -25300))
                        .with_context("preflight probe returned no decision"),
                );

        assert_eq!(outcome.state(), PermissionState::Unknown);
        let text = outcome.to_string();
        assert!(text.contains("screen_capture unknown"), "{text}");
        assert!(text.contains("permission_undetermined"), "{text}");
        assert!(text.contains("osstatus:-25300"), "{text}");
    }
}
