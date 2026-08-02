//! Non-prompting Screen Recording and Accessibility probes.
//!
//! macOS authorizes screen capture and input control separately, so this reports
//! them separately and never lets one stand in for the other. Neither probe calls
//! a permission-request API, opens System Settings, or presents any interface: the
//! Core Graphics preflight and the Accessibility trust check both read the
//! decision the operating system has already made. The variants of both that can
//! prompt exist, and this package deliberately does not call them.
//!
//! An authorization is not a promise. macOS can revoke either between a probe and
//! the operation it was meant to clear, so discovery, open, and every published
//! frame still report their own typed outcome.

use std::fmt;

use mado_pilot_core::{
    DiagnosticCategory, OperationContext, PermissionKind, PermissionOutcome, PermissionProbe,
    PermissionState, ProviderId, RedactedDiagnostic, Result,
};

use crate::shim;

/// Reads the two macOS authorizations without asking the user for anything.
///
/// Construction touches no native API, so an application can hold a probe on a
/// host that offers neither authorization.
#[derive(Debug, Default, Clone, Copy)]
pub struct MacosPermissionProbe;

impl MacosPermissionProbe {
    /// Creates a probe.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl PermissionProbe for MacosPermissionProbe {
    fn provider(&self) -> ProviderId {
        crate::provider::PROVIDER
    }

    fn probe(
        &self,
        kind: PermissionKind,
        operation: &OperationContext,
    ) -> Result<PermissionOutcome> {
        let attempt = mado_pilot_core::Operation::admit(operation)?;
        // Launch and code signature are independent parts of the execution
        // context that makes an answer interpretable. Neither is inferred from
        // the other, and the native read never requests authorization.
        let context = shim::execution_context();
        let state = match kind {
            PermissionKind::ScreenCapture => shim::probe_screen_capture(),
            PermissionKind::InputControl => shim::probe_accessibility(),
            // A permission kind this build does not know about has no macOS
            // authorization behind it here. Reporting it as unavailable says the
            // concept is absent from this Adapter rather than that it is refused.
            _ => Ok(PermissionState::Unavailable),
        };
        let outcome = match state {
            Ok(state) => {
                let outcome = PermissionOutcome::new(kind, state);
                match category(state) {
                    Some(category) => outcome.with_diagnostic(
                        RedactedDiagnostic::new(category).with_context(context.as_context()),
                    ),
                    // An authorization that is held needs no explanation, and a
                    // diagnostic attached to one would have to invent a category.
                    None => outcome,
                }
            }
            // A probe that could not run reports the honest result of refusing to
            // prompt rather than failing the read.
            Err(status) => PermissionOutcome::new(kind, PermissionState::Unknown).with_diagnostic(
                RedactedDiagnostic::new(DiagnosticCategory::PermissionUndetermined)
                    .with_platform(mado_pilot_core::PlatformCode::new(
                        "madopilot_shim",
                        i64::from(status.as_raw()),
                    ))
                    .with_context(context.as_context()),
            ),
        };
        Ok(attempt.commit(outcome)?)
    }
}

impl fmt::Display for MacosPermissionProbe {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("macos non-prompting permission probe")
    }
}

/// Returns what a caller has to act on, or `None` when nothing needs acting on.
///
/// Neither native check returns an error code — both answer with a boolean — so a
/// state-derived diagnostic carries a category and a static, redacted execution
/// context and nothing numeric. Inventing a code space for a boolean would make
/// the report look like it had consulted something it had not.
fn category(state: PermissionState) -> Option<DiagnosticCategory> {
    match state {
        PermissionState::Granted => None,
        PermissionState::NotGranted => Some(DiagnosticCategory::PermissionDenied),
        PermissionState::Unavailable => Some(DiagnosticCategory::CapabilityUnavailable),
        // A state this build does not know about is undetermined, not authorized.
        _ => Some(DiagnosticCategory::PermissionUndetermined),
    }
}

#[cfg(test)]
mod tests {
    use mado_pilot_core::{
        CancellationToken, OperationContext, PermissionKind, PermissionProbe, PermissionState,
        Status,
    };

    use super::MacosPermissionProbe;

    #[test]
    fn the_two_authorizations_are_read_independently() {
        let probe = MacosPermissionProbe::new();

        let report = probe
            .report(&OperationContext::new())
            .expect("a non-prompting probe always answers");

        // Which states a host reports depends on what the user granted this
        // process, so the assertion is about independence rather than values.
        assert_eq!(report.capture().kind(), PermissionKind::ScreenCapture);
        assert_eq!(report.input().kind(), PermissionKind::InputControl);
        for outcome in [report.capture(), report.input()] {
            assert_eq!(
                outcome.diagnostic().is_none(),
                outcome.is_granted(),
                "only a held authorization is reported without a category to act on"
            );
        }
    }

    #[test]
    fn a_denied_capture_state_leaves_the_input_state_untouched() {
        let probe = MacosPermissionProbe::new();

        let first = probe
            .report(&OperationContext::new())
            .expect("first read succeeds");
        let second = probe
            .report(&OperationContext::new())
            .expect("reading capture again cannot change accessibility");

        assert_eq!(first.input().state(), second.input().state());
        assert_eq!(first.capture().state(), second.capture().state());
    }

    #[test]
    fn a_cancelled_probe_consults_no_authorization() {
        let token = CancellationToken::new();
        token.cancel();
        let context = OperationContext::new().with_cancellation(token);

        let error = MacosPermissionProbe::new()
            .probe(PermissionKind::ScreenCapture, &context)
            .expect_err("cancelled before admission");

        assert_eq!(error.status(), Status::Cancelled);
    }

    #[test]
    fn a_probe_never_reports_an_unknown_state_as_authorization() {
        let probe = MacosPermissionProbe::new();

        let outcome = probe
            .probe(PermissionKind::InputControl, &OperationContext::new())
            .expect("read");

        if outcome.state() == PermissionState::Unknown {
            assert!(!outcome.is_granted());
        }
    }
}
