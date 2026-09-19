//! Public pacing invariants, atomic resolution, and additive construction compatibility.

use std::time::Duration;

use mado_pilot_capture::{
    CapturePacingOutcome, CapturePacingReport, CapturePacingRequest, CoordinateSupport,
    OpenRequest, PacingUnsupportedReason, PixelFormat, ResolvedCapturePacing, SessionDescription,
};
use mado_pilot_core::{IdentityIssuer, PixelExtent, ProviderId, Status, StreamId, TargetId};

const SOURCE_DEFAULT: ResolvedCapturePacing = ResolvedCapturePacing::source_default();
const INHERIT: CapturePacingRequest = CapturePacingRequest::inherit();
const DEFAULT_OPEN: OpenRequest = OpenRequest::new();
const SOURCE_OPEN: OpenRequest =
    DEFAULT_OPEN.with_capture_pacing(CapturePacingRequest::source_default());
const RESET_OPEN: OpenRequest = SOURCE_OPEN.with_capture_pacing(INHERIT);
const RESET_SELECTION: ResolvedCapturePacing = RESET_OPEN.capture_pacing().resolve(SOURCE_DEFAULT);

const fn legacy_description(target: TargetId, stream: StreamId) -> SessionDescription {
    SessionDescription::new(
        target,
        stream,
        PixelExtent::new(8, 6),
        PixelFormat::Rgba8,
        CoordinateSupport::frame_only(),
    )
}

#[test]
fn zero_is_rejected_instead_of_becoming_source_default() {
    assert_eq!(
        CapturePacingRequest::required(Duration::ZERO)
            .expect_err("zero cannot require an interval")
            .status(),
        Status::InvalidArgument
    );
    assert_eq!(
        CapturePacingRequest::preferred(Duration::ZERO)
            .expect_err("zero cannot prefer an interval")
            .status(),
        Status::InvalidArgument
    );
}

#[test]
fn every_layer_selects_the_interval_and_policy_as_one_value() {
    let required = CapturePacingRequest::required(Duration::from_millis(200)).expect("positive");
    let preferred = CapturePacingRequest::preferred(Duration::from_millis(60)).expect("positive");
    let source_default = CapturePacingRequest::source_default();
    let layers = [INHERIT, source_default, required, preferred];

    for common in layers {
        for platform in layers {
            for session in layers {
                let engine_default = platform.resolve(common.resolve(SOURCE_DEFAULT));
                let resolved = session.resolve(engine_default);
                let winner = [session, platform, common]
                    .into_iter()
                    .find(|selection| *selection != INHERIT)
                    .unwrap_or(source_default);

                assert_eq!(resolved.as_request(), winner);
                assert_eq!(resolved.is_required(), winner == required);
                assert_eq!(resolved.is_preferred(), winner == preferred);
                let expected_interval = if winner == required {
                    Some(Duration::from_millis(200))
                } else if winner == preferred {
                    Some(Duration::from_millis(60))
                } else {
                    None
                };
                assert_eq!(resolved.interval(), expected_interval);
            }
        }
    }
}

#[test]
fn replacing_a_session_selection_can_restore_requirement_or_bypass_it() {
    let required = CapturePacingRequest::required(Duration::from_millis(200)).expect("positive");
    let engine_default = required.resolve(SOURCE_DEFAULT);
    let preference = CapturePacingRequest::preferred(Duration::from_millis(60)).expect("positive");
    let request = OpenRequest::new()
        .with_capture_pacing(required)
        .with_capture_pacing(preference);
    let reason = PacingUnsupportedReason::SourceCannotPace;

    let preferred_report =
        CapturePacingReport::unsupported(request.capture_pacing().resolve(engine_default), reason)
            .expect("a session preference replaces the inherited requirement");
    assert_eq!(
        preferred_report.outcome(),
        CapturePacingOutcome::PreferredUnapplied(reason)
    );
    assert_eq!(
        preferred_report.request().interval(),
        Some(Duration::from_millis(60))
    );

    let restored = request.with_capture_pacing(INHERIT);
    assert_eq!(
        CapturePacingReport::unsupported(restored.capture_pacing().resolve(engine_default), reason)
            .expect_err("inheritance restores the required engine default")
            .status(),
        Status::Unsupported
    );
    let bypassed = request.with_capture_pacing(CapturePacingRequest::source_default());
    assert_eq!(
        CapturePacingReport::unsupported(bypassed.capture_pacing().resolve(engine_default), reason)
            .expect("source default bypasses the requirement"),
        CapturePacingReport::source_default()
    );
    assert_eq!(
        CapturePacingReport::unsupported(request.capture_pacing().resolve(engine_default), reason)
            .expect("replacing a copied request cannot change the original"),
        preferred_report
    );
}

#[test]
fn neutral_selection_preserves_large_positive_durations_without_native_conversion() {
    let common = CapturePacingRequest::required(Duration::MAX).expect("positive");
    let engine_default = common.resolve(SOURCE_DEFAULT);
    assert_eq!(engine_default.interval(), Some(Duration::MAX));
    assert_eq!(
        CapturePacingRequest::preferred(Duration::MAX)
            .expect("positive preference")
            .resolve(SOURCE_DEFAULT)
            .interval(),
        Some(Duration::MAX)
    );

    let selected = CapturePacingRequest::preferred(Duration::from_nanos(1))
        .expect("positive")
        .resolve(engine_default);
    assert!(selected.is_preferred());
    assert_eq!(selected.interval(), Some(Duration::from_nanos(1)));
    assert_eq!(engine_default.interval(), Some(Duration::MAX));
}

#[test]
fn applied_reports_reject_shortening_and_preserve_selected_policy() {
    let minimum = Duration::from_nanos(101);
    for request in [
        CapturePacingRequest::required(minimum).expect("positive"),
        CapturePacingRequest::preferred(minimum).expect("positive"),
    ] {
        let resolved = request.resolve(SOURCE_DEFAULT);
        for shorter in [Duration::ZERO, Duration::from_nanos(100)] {
            assert_eq!(
                CapturePacingReport::applied(resolved, shorter)
                    .expect_err("an applied minimum cannot be shortened")
                    .status(),
                Status::InvalidArgument
            );
        }
        for configured in [minimum, Duration::from_nanos(200)] {
            let report = CapturePacingReport::applied(resolved, configured).expect("not shortened");
            assert_eq!(report.request(), resolved);
            assert_eq!(report.configured_interval(), Some(configured));
            assert_eq!(report.outcome(), CapturePacingOutcome::Applied);
        }
    }
}

#[test]
fn source_default_cannot_masquerade_as_an_applied_interval() {
    assert_eq!(
        CapturePacingReport::applied(SOURCE_DEFAULT, Duration::from_nanos(1))
            .expect_err("no interval was selected")
            .status(),
        Status::InvalidArgument
    );
    for reason in [
        PacingUnsupportedReason::SourceCannotPace,
        PacingUnsupportedReason::NativeControlUnavailable,
    ] {
        let report = CapturePacingReport::unsupported(SOURCE_DEFAULT, reason)
            .expect("source default needs no pacing capability");
        assert_eq!(report.outcome(), CapturePacingOutcome::SourceDefault);
        assert_eq!(report.request(), SOURCE_DEFAULT);
        assert_eq!(report.request().interval(), None);
        assert_eq!(report.configured_interval(), None);
    }
}

#[test]
fn capability_absence_never_publishes_a_required_unapplied_report() {
    let interval = Duration::from_millis(60);
    let required = CapturePacingRequest::required(interval)
        .expect("positive")
        .resolve(SOURCE_DEFAULT);
    let preferred = CapturePacingRequest::preferred(interval)
        .expect("positive")
        .resolve(SOURCE_DEFAULT);

    for reason in [
        PacingUnsupportedReason::SourceCannotPace,
        PacingUnsupportedReason::NativeControlUnavailable,
    ] {
        assert_eq!(
            CapturePacingReport::unsupported(required, reason)
                .expect_err("a requirement must fail rather than publish an unapplied report")
                .status(),
            Status::Unsupported
        );
        let report = CapturePacingReport::unsupported(preferred, reason)
            .expect("only the preference can succeed unapplied");
        assert_eq!(report.request(), preferred);
        assert_eq!(report.request().interval(), Some(interval));
        assert_eq!(report.configured_interval(), None);
        assert_eq!(
            report.outcome(),
            CapturePacingOutcome::PreferredUnapplied(reason)
        );
    }
}

#[test]
fn old_const_construction_inherits_while_explicit_source_default_bypasses() {
    let required = CapturePacingRequest::required(Duration::from_millis(200))
        .expect("positive")
        .resolve(SOURCE_DEFAULT);
    for request in [DEFAULT_OPEN, OpenRequest::default(), RESET_OPEN] {
        assert_eq!(request.capture_pacing().resolve(required), required);
    }
    assert_eq!(CapturePacingRequest::default().resolve(required), required);
    assert_eq!(
        SOURCE_OPEN.capture_pacing().resolve(required),
        SOURCE_DEFAULT
    );
    assert_eq!(RESET_SELECTION, SOURCE_DEFAULT);

    let issuer = IdentityIssuer::new();
    let target = issuer
        .issue_target(ProviderId::new("pacing"))
        .expect("issued");
    let stream = issuer.issue_stream().expect("issued");
    let old_description = legacy_description(target, stream);
    assert_eq!(
        old_description.capture_pacing(),
        CapturePacingReport::source_default()
    );
}
