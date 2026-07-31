//! The native discovery, permission, and storage scenarios, through the doubles.
//!
//! Every scenario the two new capture specifications state, in the order the
//! specifications state them. The ones a working adapter cannot be made to
//! demonstrate on request — a window that closes at a chosen moment, a device that
//! is lost mid-stream, a producer whose budget is two — are driven through
//! [`ControlledCapture`] and [`ControlledProducer`], which is what those doubles
//! exist for. A platform adapter meets the same rules through its own native
//! cases, and the shared suite in `capture_contract` covers what can be checked
//! without telling anything to fail.

use std::sync::Arc;
use std::time::Duration;

use mado_pilot_capture::{
    CaptureFault, CaptureProvider, DiscoveryRequest, FrameRequest, OpenRequest, PixelFormat,
};
use mado_pilot_core::{
    CapabilitySupport, DiagnosticCategory, IdentityIssuer, InputCapability, InputDelivery,
    InputOperationKind, Lifecycle, OperationContext, PermissionKind, PermissionProbe,
    PermissionState, PixelExtent, ProviderId, Status, TargetCapability, TargetKind,
};
use mado_pilot_testkit::{
    Answer, ControlledCapture, ControlledProducer, Conversion, ScriptedPermissionProbe,
};

fn provider() -> ControlledCapture {
    ControlledCapture::new(
        Arc::new(IdentityIssuer::new()),
        PixelExtent::new(8, 6),
        PixelFormat::Bgra8,
    )
    .expect("built")
}

fn context() -> OperationContext {
    OperationContext::new()
}

fn producer(pool: usize, detached: usize) -> ControlledProducer {
    ControlledProducer::new(PixelExtent::new(8, 6), PixelFormat::Bgra8, pool, detached)
        .expect("built")
}

// Requirement: native target discovery is explicit and provider-qualified.

#[test]
fn a_caller_discovers_targets_with_provider_qualified_identities_and_capabilities() {
    let provider = provider();
    provider.declare(TargetCapability::new(
        TargetKind::Window,
        CapabilitySupport::Supported,
        InputCapability::none()
            .with_pair(
                InputOperationKind::Keyboard,
                InputDelivery::BackgroundTarget,
            )
            .with_permission(PermissionKind::InputControl),
    ));
    let display = provider
        .add_target(
            "Built-in",
            TargetCapability::capture_only(TargetKind::Display),
        )
        .expect("issued");

    let targets = provider.discover(&context()).expect("discovered");

    assert_eq!(targets.len(), 2, "both targets are listed, in order");
    assert_eq!(targets[1].id(), display);
    for target in &targets {
        assert_eq!(target.provider(), provider.provider());
    }
    let window = &targets[0];
    assert_eq!(window.capability().kind(), Some(TargetKind::Window));
    assert!(window.capability().input().supports(
        InputOperationKind::Keyboard,
        InputDelivery::BackgroundTarget
    ));
    assert!(
        !window
            .capability()
            .input()
            .supports(InputOperationKind::Keyboard, InputDelivery::System),
        "the pair that was advertised is the pair that is claimed"
    );
    assert!(
        !targets[1].capability().input().is_available(),
        "a display that accepts no input says so"
    );
}

#[test]
fn a_target_identity_from_another_provider_is_refused_without_opening_anything() {
    let provider = provider();
    let foreign = IdentityIssuer::new()
        .issue_target(ProviderId::new("windows"))
        .expect("issued");

    let error = provider
        .open(foreign, &OpenRequest::new(), &context())
        .expect_err("another provider's identity");

    assert_eq!(error.status(), Status::InvalidArgument);
}

// Requirement: discovery filtering preserves identity and authorization.

#[test]
fn a_filter_selects_only_from_the_current_authorized_result_set() {
    let provider = provider();
    provider.declare(TargetCapability::capture_only(TargetKind::Window));
    let display = provider
        .add_target(
            "Built-in",
            TargetCapability::capture_only(TargetKind::Display),
        )
        .expect("issued");
    let protected = provider
        .add_target(
            "Protected",
            TargetCapability::new(
                TargetKind::Window,
                CapabilitySupport::Unsupported,
                InputCapability::none(),
            ),
        )
        .expect("issued");

    let windows = provider
        .discover_matching(
            &DiscoveryRequest::new().with_kind(TargetKind::Window),
            &context(),
        )
        .expect("discovered");
    let capturable = provider
        .discover_matching(&DiscoveryRequest::new().requiring_capture(), &context())
        .expect("discovered");

    assert_eq!(windows.len(), 2, "the display is not a window");
    assert!(windows.iter().any(|target| target.id() == protected));
    assert!(
        capturable.iter().all(|target| target.id() != protected),
        "a target whose capture is unsupported is not selected by a capture filter"
    );
    assert!(capturable.iter().any(|target| target.id() == display));
}

#[test]
fn a_lost_target_is_reported_lost_even_when_another_target_matches_its_metadata() {
    let provider = provider();
    let original = provider.target();
    let original_name = provider
        .discover(&context())
        .expect("discovered")
        .first()
        .map(|target| target.name().to_owned())
        .expect("one target");

    let replacement = provider.replace(original).expect("issued");

    let error = provider
        .open(original, &OpenRequest::new(), &context())
        .expect_err("the original target is gone");
    assert_eq!(
        error.status(),
        Status::TargetLost,
        "a closed window is lost, not unknown, and not silently substituted"
    );

    let listed = provider.discover(&context()).expect("discovered");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id(), replacement);
    assert_eq!(
        listed[0].name(),
        original_name,
        "the replacement carries the same descriptive metadata"
    );
    assert_ne!(
        listed[0].id(),
        original,
        "matching metadata does not make it the same target"
    );
    provider
        .open(replacement, &OpenRequest::new(), &context())
        .expect("the replacement opens under its own identity");
}

#[test]
fn losing_a_target_ends_its_open_sessions_with_the_same_reason() {
    let provider = provider();
    let target = provider.target();
    let session = provider
        .open(target, &OpenRequest::new(), &context())
        .expect("opened");
    provider
        .publish(1, mado_pilot_capture::Continuity::Continuous)
        .expect("published");

    provider.lose(target);

    let error = session
        .frame(&FrameRequest::latest(), &context())
        .expect_err("capture ended");
    assert_eq!(error.status(), Status::TargetLost);
    assert_eq!(session.lifecycle(), Lifecycle::Closing);
    session.close(&context()).expect("close still finishes");
    assert!(session.is_closed());
}

// Requirement: permission queries are non-prompting and operation-specific.

#[test]
fn permission_is_reported_separately_for_capture_and_input() {
    let probe = ScriptedPermissionProbe::new(Answer::granted(), Answer::not_granted());

    let report = probe.report(&context()).expect("probed");

    assert!(report.capture().is_granted());
    assert!(report.input().state().is_refused());
    assert_eq!(probe.reads(), PermissionKind::ALL.to_vec());
}

#[test]
fn an_undetermined_probe_returns_a_state_rather_than_presenting_anything() {
    let probe = ScriptedPermissionProbe::new(Answer::undetermined(), Answer::unavailable());

    let report = probe.report(&context()).expect("probed");

    assert_eq!(report.capture().state(), PermissionState::Unknown);
    assert_eq!(
        report.capture().diagnostic().map(|d| d.category()),
        Some(DiagnosticCategory::PermissionUndetermined)
    );
    assert_eq!(
        report.input().state(),
        PermissionState::Unavailable,
        "a platform with no such authorization says so rather than claiming a grant"
    );
    assert!(!report.input().state().is_granted());
    assert!(!report.input().state().is_refused());
}

#[test]
fn a_probe_obeys_the_operation_context_it_was_given() {
    let probe = ScriptedPermissionProbe::granting();
    let expired = OperationContext::new()
        .with_timeout(Duration::ZERO)
        .expect("representable");

    let error = probe.report(&expired).expect_err("already expired");

    assert_eq!(error.status(), Status::DeadlineExceeded);
    assert!(probe.reads().is_empty());
}

// Requirement: native diagnostics protect desktop content.

#[test]
fn a_failed_probe_reports_a_redacted_category_and_platform_context() {
    let probe = ScriptedPermissionProbe::new(
        Answer::Explained(
            PermissionState::NotGranted,
            mado_pilot_core::RedactedDiagnostic::new(DiagnosticCategory::PermissionDenied)
                .with_platform(mado_pilot_core::PlatformCode::new("osstatus", -25300))
                .with_context("the preflight check refused"),
        ),
        Answer::granted(),
    );

    let outcome = probe
        .probe(PermissionKind::ScreenCapture, &context())
        .expect("probed");
    let diagnostic = outcome.diagnostic().expect("recorded");
    let text = diagnostic.to_string();

    assert_eq!(diagnostic.category(), DiagnosticCategory::PermissionDenied);
    assert_eq!(
        diagnostic.platform().map(|code| code.namespace()),
        Some("osstatus")
    );
    assert!(text.contains("permission_denied"), "{text}");
    assert!(
        !text.contains("password") && !text.contains("Untitled"),
        "a diagnostic carries no desktop content: {text}"
    );
}

// Requirement: published frames do not pin producer progress.

#[test]
fn consumers_that_retain_frames_do_not_stop_capture() {
    let provider = provider();
    let producer = producer(2, 6);
    let target = provider.target();
    let session = provider
        .open(target, &OpenRequest::new(), &context())
        .expect("opened");
    let mut retained = Vec::new();

    for fill in 0..6u8 {
        provider
            .publish_from(&producer, fill)
            .expect("the budget has room");
        retained.push(
            session
                .frame(&FrameRequest::latest(), &context())
                .expect("published"),
        );
        assert_eq!(
            producer.producer_slots_free(),
            producer.pool(),
            "retention never costs producer capacity"
        );
    }

    assert_eq!(retained.len(), 6);
    assert_eq!(producer.detached_slots_free(), 0);
}

#[test]
fn an_exhausted_storage_budget_is_a_typed_bounded_failure() {
    let provider = provider();
    let producer = producer(2, 1);
    let target = provider.target();
    let session = provider
        .open(target, &OpenRequest::new(), &context())
        .expect("opened");
    provider.publish_from(&producer, 1).expect("the first fits");
    let retained = session
        .frame(&FrameRequest::latest(), &context())
        .expect("published");

    let error = provider
        .publish_from(&producer, 2)
        .expect_err("the budget is exhausted");

    assert_eq!(error.status(), Status::LimitExceeded);
    assert_eq!(
        producer.producer_slots_free(),
        producer.pool(),
        "the producer keeps running while the caller holds the budget"
    );
    drop(retained);
    session.close(&context()).expect("closed");
    assert_eq!(producer.detached_slots_free(), producer.detached_budget());
}

// Requirement: native storage is opaque and maps with explicit lifetime.

#[test]
fn a_mapping_that_completes_after_cancellation_commits_nothing() {
    let provider = provider();
    let producer = producer(2, 2);
    let target = provider.target();
    let session = provider
        .open(target, &OpenRequest::new(), &context())
        .expect("opened");
    provider.publish_from(&producer, 0x5A).expect("published");
    let frame = session
        .frame(&FrameRequest::latest(), &context())
        .expect("published");
    producer.set_conversion(Conversion::Slow(Duration::from_millis(30)));
    let expiring = OperationContext::new()
        .with_timeout(Duration::from_millis(5))
        .expect("representable");

    let error = frame
        .map(PixelFormat::Bgra8, &expiring)
        .expect_err("the deadline passed during the conversion");

    assert_eq!(error.status(), Status::DeadlineExceeded);
    producer.set_conversion(Conversion::Immediate);
    let mapping = frame
        .map(PixelFormat::Bgra8, &context())
        .expect("a later mapping still works");
    assert!(mapping.bytes().iter().all(|byte| *byte == 0x5A));
    assert!(
        !mapping.is_shared(),
        "obtaining CPU bytes from native storage is a copy"
    );
}

#[test]
fn a_failed_conversion_leaves_the_frame_usable() {
    let provider = provider();
    let producer = producer(2, 2);
    let target = provider.target();
    let session = provider
        .open(target, &OpenRequest::new(), &context())
        .expect("opened");
    provider.publish_from(&producer, 0x22).expect("published");
    let frame = session
        .frame(&FrameRequest::latest(), &context())
        .expect("published");
    producer.set_conversion(Conversion::Fails(CaptureFault::SourceInvalid));

    let error = frame
        .map(PixelFormat::Bgra8, &context())
        .expect_err("the conversion failed");

    assert_eq!(error.status(), Status::CaptureFailed);
    assert_eq!(
        frame.descriptor().extent(),
        PixelExtent::new(8, 6),
        "a failed mapping changes nothing about the frame"
    );
}

// Requirement: native callback admission and lifecycle are bounded.

#[test]
fn a_terminal_fault_is_ordered_after_the_frames_already_published() {
    let provider = provider();
    let producer = producer(2, 4);
    let target = provider.target();
    let session = provider
        .open(target, &OpenRequest::new(), &context())
        .expect("opened");
    provider.publish_from(&producer, 7).expect("published");
    let published = session
        .frame(&FrameRequest::latest(), &context())
        .expect("published");

    provider.terminate(CaptureFault::SourceInvalid);

    let error = session
        .frame(&FrameRequest::latest(), &context())
        .expect_err("capture ended");
    assert_eq!(error.status(), Status::CaptureFailed);
    assert_eq!(
        published
            .map(PixelFormat::Bgra8, &context())
            .expect("a frame published before the end still maps")
            .stamp(),
        published.stamp()
    );
}

#[test]
fn close_is_repeatable_after_a_terminal_fault() {
    let provider = provider();
    let target = provider.target();
    let session = provider
        .open(target, &OpenRequest::new(), &context())
        .expect("opened");

    provider.terminate(CaptureFault::TargetLost);
    session.close(&context()).expect("close finishes");
    session.close(&context()).expect("close is idempotent");

    assert!(session.is_closed());
    assert_eq!(
        session
            .frame(&FrameRequest::latest(), &context())
            .expect_err("nothing publishes after the end")
            .status(),
        Status::TargetLost
    );
}
