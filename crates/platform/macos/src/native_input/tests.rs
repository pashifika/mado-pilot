//! Delivery-backend rules, checked without posting anything to the host desktop.
//!
//! The commit boundary is exercised through [`SystemCommitSource`], so the cases
//! that decide whether an irreversible event happens — a revoked authorization, a
//! target that moved between preparation and commit, a release that must go out
//! anyway — are reachable without a granted host and without moving the
//! developer's pointer.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use mado_pilot_core::{
    CancellationToken, CoordinateSpace, GeometryRevision, IdentityIssuer, InputDelivery,
    OperationContext, PermissionState, Point, ProviderId, TargetId, TargetKind, TransformSnapshot,
};
use mado_pilot_input::{
    CleanupState, DeliveryPlan, FocusPolicy, GeometryPolicy, InputController, InputDescriptor,
    InputEvent, InputFault, InputRequest, InputSequence, Key, Modifier, PointerButton,
    PointerGeometry, PressedState, SequenceOutcome,
};

use super::{
    CommitGeometry, DriverState, FUNCTION_KEYS, GeometryFingerprint, NativePost, PointerState,
    ProcessCommitSource, SystemButtonState, SystemCommitSource, SystemKeyState, commit_geometry,
    commit_prepared, commit_process, contains_desktop_point, extent_from_points, focus_wait,
    key_flag, modifier_flag, native_button, placement_for, process_permission_denied_fault,
    process_status_fault, release_pending_process, release_process, release_system,
    resolve_key_code, retain_process_press, text_chunks,
};
use crate::input::{
    InputDriver, MacosInputController, PendingTextRelease, SubmissionFailure, input_capability,
};
use crate::shim::{self, ShimStatus};

/// One recorded post, with the flags it would have carried.
#[derive(Debug, Clone, PartialEq)]
struct Posted {
    post: String,
    flags: u32,
}

/// A commit source whose revalidation outcome and post outcome the test writes.
#[derive(Debug, Default)]
struct FakeSource {
    posts: Mutex<Vec<Posted>>,
    revalidation: Mutex<Option<InputFault>>,
    cleanup_authorization: Mutex<Option<InputFault>>,
    current_geometry: Mutex<Option<GeometryFingerprint>>,
    post_failure: Mutex<Option<(ShimStatus, usize)>>,
    revalidations: Mutex<usize>,
    cleanup_authorizations: Mutex<usize>,
}

impl FakeSource {
    fn new() -> Self {
        Self::default()
    }

    fn refusing_revalidation(fault: InputFault) -> Self {
        let source = Self::default();
        *source.revalidation.lock().expect("uncontended") = Some(fault);
        source
    }

    fn failing_post(status: ShimStatus, posted: usize) -> Self {
        let source = Self::default();
        *source.post_failure.lock().expect("uncontended") = Some((status, posted));
        source
    }

    fn with_current_geometry(geometry: GeometryFingerprint) -> Self {
        let source = Self::default();
        *source.current_geometry.lock().expect("uncontended") = Some(geometry);
        source
    }

    fn revoke_cleanup_authorization(&self) {
        *self.cleanup_authorization.lock().expect("uncontended") = Some(InputFault::NotAuthorized);
    }

    fn posts(&self) -> Vec<Posted> {
        self.posts.lock().expect("uncontended").clone()
    }

    fn revalidations(&self) -> usize {
        *self.revalidations.lock().expect("uncontended")
    }

    fn cleanup_authorizations(&self) -> usize {
        *self.cleanup_authorizations.lock().expect("uncontended")
    }
}

impl SystemCommitSource for FakeSource {
    fn revalidate_system_commit(
        &self,
        _focus: FocusPolicy,
        geometry: CommitGeometry,
        _operation: &OperationContext,
    ) -> Result<(), InputFault> {
        *self.revalidations.lock().expect("uncontended") += 1;
        if let Some(fault) = *self.revalidation.lock().expect("uncontended") {
            return Err(fault);
        }
        if let (CommitGeometry::RequireCurrent(expected), Some(current)) = (
            geometry,
            *self.current_geometry.lock().expect("uncontended"),
        ) && current != expected
        {
            return Err(InputFault::GeometryChanged);
        }
        Ok(())
    }

    fn revalidate_cleanup_authorization(&self) -> Result<(), InputFault> {
        *self.cleanup_authorizations.lock().expect("uncontended") += 1;
        match *self.cleanup_authorization.lock().expect("uncontended") {
            Some(fault) => Err(fault),
            None => Ok(()),
        }
    }

    fn post(&self, post: NativePost<'_>, flags: u32) -> Result<(), (ShimStatus, usize)> {
        self.posts.lock().expect("uncontended").push(Posted {
            post: format!("{post:?}"),
            flags,
        });
        match *self.post_failure.lock().expect("uncontended") {
            Some(failure) => Err(failure),
            None => Ok(()),
        }
    }

    fn classify_post_failure(&self, status: ShimStatus) -> InputFault {
        match status {
            ShimStatus::TargetLost => InputFault::TargetLost,
            ShimStatus::PermissionDenied => InputFault::NotAuthorized,
            _ => InputFault::SubmissionFailed,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct FakeProcessSource {
    result: Result<shim::ProcessPostOutcome, shim::ProcessPostFailure>,
}

impl ProcessCommitSource for FakeProcessSource {
    fn post_process(
        &self,
        _event_source: Option<&shim::ProcessEventSource>,
        _request: shim::ProcessPostRequest<'_>,
        _operation: &OperationContext,
    ) -> Result<shim::ProcessPostOutcome, shim::ProcessPostFailure> {
        self.result
    }
}

#[derive(Debug, Default)]
struct RecordingProcessSource {
    keys: Mutex<Vec<(u16, bool)>>,
    purposes: Mutex<Vec<shim::ProcessPostPurpose>>,
}

impl ProcessCommitSource for RecordingProcessSource {
    fn post_process(
        &self,
        _event_source: Option<&shim::ProcessEventSource>,
        request: shim::ProcessPostRequest<'_>,
        _operation: &OperationContext,
    ) -> Result<shim::ProcessPostOutcome, shim::ProcessPostFailure> {
        let shim::ProcessPostRequest { post, purpose, .. } = request;
        self.purposes.lock().expect("uncontended").push(purpose);
        let shim::ProcessPost::Key { key_code, down } = post else {
            panic!("the cleanup must post the retained key")
        };
        self.keys
            .lock()
            .expect("uncontended")
            .push((key_code, down));
        Ok(shim::ProcessPostOutcome {
            invoked_native_units: 1,
            target_match_count: purpose.expected_target_match_count(),
            authorization: shim::ProcessAuthorizationObservation::Granted,
            geometry: shim::ProcessGeometryObservation::NotApplicable,
        })
    }
}
#[derive(Debug, Default)]
struct UnavailableAfterPartialTextSource {
    purposes: Mutex<Vec<shim::ProcessPostPurpose>>,
}

impl ProcessCommitSource for UnavailableAfterPartialTextSource {
    fn post_process(
        &self,
        _event_source: Option<&shim::ProcessEventSource>,
        request: shim::ProcessPostRequest<'_>,
        _operation: &OperationContext,
    ) -> Result<shim::ProcessPostOutcome, shim::ProcessPostFailure> {
        let shim::ProcessPostRequest { post, purpose, .. } = request;
        self.purposes.lock().expect("uncontended").push(purpose);
        match (purpose, post) {
            (shim::ProcessPostPurpose::Input, shim::ProcessPost::Text(_)) => {
                Err(shim::ProcessPostFailure {
                    status: ShimStatus::PlatformFailure,
                    invoked_native_units: 1,
                    target_match_count: 1,
                    authorization: shim::ProcessAuthorizationObservation::Granted,
                    geometry: shim::ProcessGeometryObservation::NotApplicable,
                })
            }
            (
                shim::ProcessPostPurpose::Release,
                shim::ProcessPost::Key {
                    key_code: 0,
                    down: false,
                },
            ) => Ok(shim::ProcessPostOutcome {
                invoked_native_units: 1,
                target_match_count: 0,
                authorization: shim::ProcessAuthorizationObservation::Granted,
                geometry: shim::ProcessGeometryObservation::NotApplicable,
            }),
            (
                shim::ProcessPostPurpose::Input,
                shim::ProcessPost::Key {
                    key_code: 0,
                    down: false,
                },
            ) => Err(shim::ProcessPostFailure {
                status: ShimStatus::TargetLost,
                invoked_native_units: 0,
                target_match_count: 0,
                authorization: shim::ProcessAuthorizationObservation::Unknown,
                geometry: shim::ProcessGeometryObservation::NotEvaluated,
            }),
            unexpected => panic!("unexpected process post: {unexpected:?}"),
        }
    }
}

#[derive(Debug)]
struct InterruptingProcessSource {
    result: Result<shim::ProcessPostOutcome, shim::ProcessPostFailure>,
    cancellation: CancellationToken,
}

impl ProcessCommitSource for InterruptingProcessSource {
    fn post_process(
        &self,
        _event_source: Option<&shim::ProcessEventSource>,
        _request: shim::ProcessPostRequest<'_>,
        _operation: &OperationContext,
    ) -> Result<shim::ProcessPostOutcome, shim::ProcessPostFailure> {
        self.cancellation.cancel();
        self.result
    }
}

fn fingerprint(origin: (f64, f64), size: (f64, f64), scale: f64) -> GeometryFingerprint {
    let (extent, placement) = placement_for(origin, size, scale).expect("a live rectangle");
    GeometryFingerprint { extent, placement }
}

fn key_post(key_code: u16, down: bool) -> NativePost<'static> {
    NativePost::Key { key_code, down }
}

fn target() -> TargetId {
    IdentityIssuer::new()
        .issue_target(ProviderId::new("macos-native-input"))
        .expect("issued")
}

/// Drives the production native commit and cleanup helpers while making an
/// authorization revocation deterministic between them.
#[derive(Debug, Default)]
struct RevokingNativePathDriver {
    source: FakeSource,
    deliveries: Mutex<usize>,
}

impl InputDriver for RevokingNativePathDriver {
    fn preflight(
        &self,
        delivery: InputDelivery,
        _focus: FocusPolicy,
        _operation: &OperationContext,
    ) -> Result<(), InputFault> {
        if delivery == InputDelivery::System {
            Ok(())
        } else {
            Err(InputFault::UnsupportedCombination)
        }
    }

    fn submit(
        &self,
        _delivery: InputDelivery,
        focus: FocusPolicy,
        event: &InputEvent,
        _geometry: PointerGeometry,
        state: &mut DriverState,
        operation: &OperationContext,
    ) -> Result<(), SubmissionFailure> {
        let mut deliveries = self.deliveries.lock().expect("uncontended");
        let index = *deliveries;
        *deliveries += 1;
        drop(deliveries);

        if index > 0 {
            return Err(SubmissionFailure::before_event(InputFault::NotAuthorized));
        }
        assert_eq!(event, &InputEvent::KeyPress(Key::Modifier(Modifier::Meta)));
        commit_prepared(
            &self.source,
            focus,
            CommitGeometry::NotApplicable,
            operation,
            key_post(0x37, true),
            shim::INPUT_FLAG_META,
        )?;
        state.keys.push(SystemKeyState {
            logical: Key::Modifier(Modifier::Meta),
            key_code: 0x37,
        });
        self.source.revoke_cleanup_authorization();
        Ok(())
    }

    fn release(
        &self,
        delivery: InputDelivery,
        pressed: PressedState,
        state: &mut DriverState,
        operation: &OperationContext,
    ) -> Result<(), InputFault> {
        if delivery != InputDelivery::System {
            return Err(InputFault::UnsupportedCombination);
        }
        release_system(pressed, state, &self.source, operation)
    }
}

#[test]
fn a_focus_observation_never_outlives_the_callers_budget() {
    let bounded = OperationContext::new()
        .with_timeout(Duration::from_millis(30))
        .expect("positive timeout");

    assert!(focus_wait(&bounded) <= Duration::from_millis(30));
    assert_eq!(
        focus_wait(&OperationContext::new()),
        Duration::from_millis(250)
    );
}

#[test]
fn every_fixed_key_resolves_to_a_code_and_no_two_share_one() {
    let keys = [
        Key::Enter,
        Key::Tab,
        Key::Backspace,
        Key::Delete,
        Key::Escape,
        Key::Space,
        Key::ArrowUp,
        Key::ArrowDown,
        Key::ArrowLeft,
        Key::ArrowRight,
        Key::Home,
        Key::End,
        Key::PageUp,
        Key::PageDown,
        Key::Modifier(Modifier::Shift),
        Key::Modifier(Modifier::Control),
        Key::Modifier(Modifier::Alt),
        Key::Modifier(Modifier::Meta),
    ];

    let mut seen = Vec::new();
    for key in keys {
        let code = resolve_key_code(key).unwrap_or_else(|_| panic!("{key} resolves"));
        assert!(
            !seen.contains(&code),
            "{key} shares a hardware code with an earlier key"
        );
        seen.push(code);
    }
    // Backspace and forward delete are different keys, which is the pair a
    // transcription error most easily collapses.
    assert_ne!(
        resolve_key_code(Key::Backspace),
        resolve_key_code(Key::Delete)
    );
}

#[test]
fn function_keys_resolve_only_as_far_as_macos_defines_them() {
    for number in 1..=20u8 {
        let code = resolve_key_code(Key::Function(number)).expect("defined");
        assert_eq!(code, FUNCTION_KEYS[usize::from(number) - 1]);
    }
    for number in [0u8, 21, 24, 25] {
        assert_eq!(
            resolve_key_code(Key::Function(number)),
            Err(InputFault::UnsupportedCombination),
            "F{number} has no macOS key code, so it is refused rather than posted \
             as an undefined one"
        );
    }
}

#[test]
fn a_synthesized_event_carries_only_the_modifiers_this_sequence_is_holding() {
    let mut state = DriverState::default();
    assert_eq!(state.held_flags(), 0);

    state.keys.push(SystemKeyState {
        logical: Key::Modifier(Modifier::Meta),
        key_code: 0x37,
    });
    state.keys.push(SystemKeyState {
        logical: Key::Character('c'),
        key_code: 8,
    });

    assert_eq!(
        state.held_flags(),
        shim::INPUT_FLAG_META,
        "an ordinary key contributes no modifier flag"
    );
    assert_eq!(
        key_flag(Key::Modifier(Modifier::Shift)),
        shim::INPUT_FLAG_SHIFT
    );
    assert_eq!(key_flag(Key::Enter), 0);
    assert_eq!(modifier_flag(Modifier::Control), shim::INPUT_FLAG_CONTROL);
    assert_eq!(modifier_flag(Modifier::Alt), shim::INPUT_FLAG_ALT);
}
#[test]
fn a_partially_posted_process_key_releases_the_exact_resolved_key_code() {
    let logical = Key::Enter;
    let exact_key_code = 0x7F;
    let mut state = DriverState::default();
    let partial = Err(SubmissionFailure::after_native_units(
        InputFault::SubmissionFailed,
        1,
    ));
    let result = retain_process_press(
        &mut state.keys,
        SystemKeyState {
            logical,
            key_code: exact_key_code,
        },
        partial,
    );
    assert!(result.is_err());
    assert_eq!(state.keys.len(), 1);

    let source = RecordingProcessSource::default();
    release_process(
        PressedState::Key(logical),
        &mut state,
        &source,
        &OperationContext::new(),
    )
    .expect("cleanup releases the partially posted key");

    assert!(state.keys.is_empty());
    assert_eq!(
        *source.keys.lock().expect("uncontended"),
        vec![(exact_key_code, false)]
    );
    assert_eq!(
        *source.purposes.lock().expect("uncontended"),
        vec![shim::ProcessPostPurpose::Release]
    );
}
#[test]
fn partial_process_text_cleanup_uses_release_authority_after_target_becomes_unavailable() {
    let source = UnavailableAfterPartialTextSource::default();
    let units = ['x' as u16];
    let failure = commit_process(
        &source,
        None,
        shim::ProcessGeometry::AuthorityOnly,
        &OperationContext::new(),
        NativePost::Text(&units),
        shim::INPUT_FLAG_SHIFT,
    )
    .expect_err("the text down half is the only invoked native unit");
    assert_eq!(failure.invoked_native_units, 1);
    assert!(failure.current_event_may_have_effect);

    release_pending_process(
        &source,
        None,
        PendingTextRelease {
            route: InputDelivery::ProcessDirected,
            flags: shim::INPUT_FLAG_SHIFT,
        },
        &OperationContext::new(),
    )
    .expect(
        "cleanup bypasses ordinary window eligibility after the retained target is unavailable",
    );

    assert_eq!(
        *source.purposes.lock().expect("uncontended"),
        vec![
            shim::ProcessPostPurpose::Input,
            shim::ProcessPostPurpose::Release,
        ]
    );
}

#[test]
fn a_release_clears_its_own_modifier_on_the_event_that_releases_it() {
    let source = FakeSource::new();
    let mut state = DriverState::default();
    state.keys.push(SystemKeyState {
        logical: Key::Modifier(Modifier::Shift),
        key_code: 0x38,
    });
    state.keys.push(SystemKeyState {
        logical: Key::Modifier(Modifier::Control),
        key_code: 0x3B,
    });

    release_system(
        PressedState::Key(Key::Modifier(Modifier::Control)),
        &mut state,
        &source,
        &OperationContext::new(),
    )
    .expect("the release goes out");

    let posted = source.posts();
    assert_eq!(posted.len(), 1);
    assert_eq!(
        posted[0].flags,
        shim::INPUT_FLAG_SHIFT,
        "the modifier still held stays set and the released one does not"
    );
    assert_eq!(state.keys.len(), 1);
}

#[test]
fn a_cleanup_release_is_posted_without_revalidating_focus() {
    // A window that stopped being frontmost is exactly when a held button matters
    // most, so cleanup must not be gated on the focus the request needed.
    let source = FakeSource::new();
    let mut state = DriverState {
        pointer: Some(PointerState {
            desktop: (120.0, 80.0),
            geometry: fingerprint((0.0, 0.0), (640.0, 420.0), 2.0),
        }),
        ..DriverState::default()
    };
    state.buttons.push(SystemButtonState {
        logical: PointerButton::Primary,
        native: shim::INPUT_BUTTON_PRIMARY,
    });

    release_system(
        PressedState::Button(PointerButton::Primary),
        &mut state,
        &source,
        &OperationContext::new(),
    )
    .expect("the release goes out");

    assert_eq!(source.posts().len(), 1);
    assert_eq!(
        source.revalidations(),
        0,
        "cleanup still skips the ordinary focus and geometry gate"
    );
    assert_eq!(source.cleanup_authorizations(), 1);
    assert!(state.buttons.is_empty());
}

#[test]
fn revoked_post_event_access_after_a_submitted_press_leaves_cleanup_truthfully_incomplete() {
    let target = target();
    let driver = Arc::new(RevokingNativePathDriver::default());
    let controller = MacosInputController::with_driver(
        InputDescriptor::new(target, input_capability(TargetKind::Window, true)),
        Arc::clone(&driver) as Arc<dyn InputDriver>,
    );
    let sequence = InputSequence::new(vec![
        InputEvent::KeyPress(Key::Modifier(Modifier::Meta)),
        InputEvent::KeyPress(Key::Enter),
    ])
    .expect("valid sequence");
    let request = InputRequest::new(
        target,
        sequence,
        DeliveryPlan::require(InputDelivery::System),
    )
    .with_focus(FocusPolicy::RequireFocused);

    let receipt = controller
        .execute(&request, &OperationContext::new())
        .expect("possible partial native effect returns a receipt");

    assert_eq!(receipt.outcome(), SequenceOutcome::Partial);
    assert_eq!(receipt.submitted(), 1);
    assert_eq!(receipt.fault(), Some(InputFault::NotAuthorized));
    assert_eq!(receipt.cleanup(), CleanupState::Incomplete);
    assert_eq!(receipt.cleanup_released(), 0);
    assert_eq!(
        driver.source.posts().len(),
        1,
        "only the submitted press was posted"
    );
    assert_eq!(driver.source.cleanup_authorizations(), 1);
    assert_eq!(
        receipt.cleanup_owed(),
        1,
        "the unreleased owned modifier remains truthfully owed"
    );
}

#[test]
fn a_release_with_no_recorded_pointer_is_refused_rather_than_posted_somewhere() {
    let source = FakeSource::new();
    let mut state = DriverState::default();
    state.buttons.push(SystemButtonState {
        logical: PointerButton::Primary,
        native: shim::INPUT_BUTTON_PRIMARY,
    });

    let error = release_system(
        PressedState::Button(PointerButton::Primary),
        &mut state,
        &source,
        &OperationContext::new(),
    )
    .expect_err("a button release carries a location");

    assert_eq!(error, InputFault::UnsupportedCoordinate);
    assert!(source.posts().is_empty());
}

#[test]
fn an_interrupted_cleanup_context_stops_before_the_release_is_posted() {
    let source = FakeSource::new();
    let token = CancellationToken::new();
    token.cancel();
    let mut state = DriverState::default();
    state.keys.push(SystemKeyState {
        logical: Key::Modifier(Modifier::Meta),
        key_code: 0x37,
    });

    let error = release_system(
        PressedState::Key(Key::Modifier(Modifier::Meta)),
        &mut state,
        &source,
        &OperationContext::new().with_cancellation(token),
    )
    .expect_err("an interrupted context posts nothing");

    assert_eq!(error, InputFault::Cancelled);
    assert!(source.posts().is_empty());
}

#[test]
fn a_revoked_authorization_at_the_commit_boundary_stops_before_the_event() {
    let source = FakeSource::refusing_revalidation(InputFault::NotAuthorized);

    let failure = commit_prepared(
        &source,
        FocusPolicy::RequireFocused,
        CommitGeometry::NotApplicable,
        &OperationContext::new(),
        key_post(0x24, true),
        0,
    )
    .expect_err("an unauthorized process posts nothing");

    assert_eq!(failure.fault, InputFault::NotAuthorized);
    assert!(
        !failure.current_event_may_have_effect,
        "macOS discards the event, so nothing reached the target"
    );
    assert!(source.posts().is_empty());
}

#[test]
fn a_geometry_change_between_preparation_and_commit_refuses_the_post() {
    let prepared = fingerprint((0.0, 0.0), (640.0, 420.0), 2.0);
    let moved = fingerprint((50.0, 25.0), (640.0, 420.0), 2.0);
    let source = FakeSource::with_current_geometry(moved);

    let failure = commit_prepared(
        &source,
        FocusPolicy::RequireFocused,
        commit_geometry(GeometryPolicy::RequireUnchanged, prepared).expect("supported policy"),
        &OperationContext::new(),
        NativePost::Pointer {
            action: shim::INPUT_POINTER_PRESS,
            button: shim::INPUT_BUTTON_PRIMARY,
            click_state: shim::INPUT_SINGLE_CLICK,
            location: (10.0, 10.0),
        },
        0,
    )
    .expect_err("the coordinate no longer names what it was resolved against");

    assert_eq!(failure.fault, InputFault::GeometryChanged);
    assert!(source.posts().is_empty());
}

#[test]
fn a_move_after_resolution_distinguishes_snapshot_from_unchanged_geometry() {
    let prepared = fingerprint((0.0, 0.0), (640.0, 420.0), 2.0);
    let moved = fingerprint((50.0, 25.0), (640.0, 420.0), 2.0);
    let post = NativePost::Pointer {
        action: shim::INPUT_POINTER_MOVE,
        button: shim::INPUT_BUTTON_NONE,
        click_state: 0,
        location: (10.0, 10.0),
    };

    let snapshot = FakeSource::with_current_geometry(moved);
    commit_prepared(
        &snapshot,
        FocusPolicy::RequireFocused,
        commit_geometry(GeometryPolicy::UseFrameSnapshot, prepared).expect("supported policy"),
        &OperationContext::new(),
        post,
        0,
    )
    .expect("the retained snapshot remains deliverable after the target moves");
    assert_eq!(snapshot.revalidations(), 1);
    assert_eq!(snapshot.posts().len(), 1);

    let unchanged = FakeSource::with_current_geometry(moved);
    let failure = commit_prepared(
        &unchanged,
        FocusPolicy::RequireFocused,
        commit_geometry(GeometryPolicy::RequireUnchanged, prepared).expect("supported policy"),
        &OperationContext::new(),
        post,
        0,
    )
    .expect_err("the unchanged policy refuses the same move");
    assert_eq!(failure.fault, InputFault::GeometryChanged);
    assert!(unchanged.posts().is_empty());
}

#[test]
fn a_deadline_that_passes_before_the_commit_posts_nothing() {
    let source = FakeSource::new();
    let expiring = OperationContext::new()
        .with_timeout(Duration::from_millis(1))
        .expect("representable");
    std::thread::sleep(Duration::from_millis(5));

    let failure = commit_prepared(
        &source,
        FocusPolicy::RequireFocused,
        CommitGeometry::NotApplicable,
        &expiring,
        key_post(0x24, true),
        0,
    )
    .expect_err("arbitration precedes the irreversible act");

    assert_eq!(failure.fault, InputFault::DeadlineExceeded);
    assert_eq!(source.revalidations(), 0);
    assert!(source.posts().is_empty());
}

#[test]
fn a_post_that_partly_reached_the_target_reports_effect_it_cannot_take_back() {
    let partly = FakeSource::failing_post(ShimStatus::PlatformFailure, 4);
    let units: Vec<u16> = "hello".encode_utf16().collect();

    let failure = commit_prepared(
        &partly,
        FocusPolicy::RequireFocused,
        CommitGeometry::NotApplicable,
        &OperationContext::new(),
        NativePost::Text(&units),
        0,
    )
    .expect_err("the post failed");

    assert!(failure.current_event_may_have_effect);
    assert_eq!(failure.fault, InputFault::SubmissionFailed);
}
#[test]
fn capture_query_denial_does_not_masquerade_as_input_denial() {
    assert_eq!(
        process_permission_denied_fault(PermissionState::Granted),
        InputFault::SubmissionFailed
    );
    assert_eq!(
        process_permission_denied_fault(PermissionState::NotGranted),
        InputFault::NotAuthorized
    );
}

#[test]
fn every_native_process_status_maps_to_an_existing_input_fault() {
    let operation = OperationContext::new();
    let cases = [
        (ShimStatus::TargetLost, InputFault::TargetLost),
        (ShimStatus::PermissionDenied, InputFault::NotAuthorized),
        (ShimStatus::Unsupported, InputFault::UnsupportedCombination),
        (ShimStatus::GeometryChanged, InputFault::GeometryChanged),
        (ShimStatus::Closed, InputFault::ControllerClosed),
        (ShimStatus::TimedOut, InputFault::SubmissionFailed),
        (ShimStatus::InvalidArgument, InputFault::SubmissionFailed),
        (ShimStatus::PlatformFailure, InputFault::SubmissionFailed),
        (ShimStatus::NativeException, InputFault::SubmissionFailed),
        (ShimStatus::Unrecognized(999), InputFault::SubmissionFailed),
    ];

    for (status, expected) in cases {
        assert_eq!(
            process_status_fault(status, &operation),
            expected,
            "{status:?}"
        );
    }
}

#[test]
fn a_post_that_reached_nothing_is_classified_by_what_the_platform_said() {
    let lost = FakeSource::failing_post(ShimStatus::TargetLost, 0);

    let failure = commit_prepared(
        &lost,
        FocusPolicy::RequireFocused,
        CommitGeometry::NotApplicable,
        &OperationContext::new(),
        key_post(0x24, true),
        0,
    )
    .expect_err("the post failed");

    assert!(!failure.current_event_may_have_effect);
    assert_eq!(failure.fault, InputFault::TargetLost);
}

#[test]
fn process_commit_maps_native_counts_to_before_or_during_event_failures() {
    let failure = |status, invoked_native_units, authorization| FakeProcessSource {
        result: Err(shim::ProcessPostFailure {
            status,
            invoked_native_units,
            target_match_count: 1,
            authorization,
            geometry: shim::ProcessGeometryObservation::NotEvaluated,
        }),
    };

    let geometry_changed = commit_process(
        &failure(
            ShimStatus::GeometryChanged,
            0,
            shim::ProcessAuthorizationObservation::Unknown,
        ),
        None,
        shim::ProcessGeometry::AuthorityOnly,
        &OperationContext::new(),
        key_post(0x24, true),
        0,
    )
    .expect_err("geometry changed before posting");
    assert_eq!(geometry_changed.fault, InputFault::GeometryChanged);
    assert!(!geometry_changed.current_event_may_have_effect);

    let revoked = commit_process(
        &failure(
            ShimStatus::PermissionDenied,
            1,
            shim::ProcessAuthorizationObservation::NotGranted,
        ),
        None,
        shim::ProcessGeometry::AuthorityOnly,
        &OperationContext::new(),
        key_post(0x24, true),
        0,
    )
    .expect_err("authorization changed after native effect");
    assert_eq!(revoked.fault, InputFault::NotAuthorized);
    assert!(revoked.current_event_may_have_effect);

    let contradictory_denial = commit_process(
        &failure(
            ShimStatus::PermissionDenied,
            0,
            shim::ProcessAuthorizationObservation::Granted,
        ),
        None,
        shim::ProcessGeometry::AuthorityOnly,
        &OperationContext::new(),
        key_post(0x24, true),
        0,
    )
    .expect_err("the atomic native report controls denial classification");
    assert_eq!(
        contradictory_denial.fault,
        InputFault::SubmissionFailed,
        "a later authorization read must not overwrite the native report"
    );
}

#[test]
fn process_commit_requires_exact_native_unit_and_target_match_counts() {
    let source = |invoked_native_units, target_match_count| FakeProcessSource {
        result: Ok(shim::ProcessPostOutcome {
            invoked_native_units,
            target_match_count,
            authorization: shim::ProcessAuthorizationObservation::Granted,
            geometry: shim::ProcessGeometryObservation::NotApplicable,
        }),
    };

    commit_process(
        &source(1, 1),
        None,
        shim::ProcessGeometry::AuthorityOnly,
        &OperationContext::new(),
        key_post(0x24, true),
        0,
    )
    .expect("one key event and one retained-window match are exact");

    let contradictory = commit_process(
        &source(0, 2),
        None,
        shim::ProcessGeometry::AuthorityOnly,
        &OperationContext::new(),
        key_post(0x24, true),
        0,
    )
    .expect_err("multiple exact retained-window matches are contradictory");
    assert_eq!(contradictory.fault, InputFault::SubmissionFailed);
    assert!(!contradictory.current_event_may_have_effect);

    let units: Vec<u16> = "x".encode_utf16().collect();
    let partial_text = commit_process(
        &source(1, 1),
        None,
        shim::ProcessGeometry::AuthorityOnly,
        &OperationContext::new(),
        NativePost::Text(&units),
        0,
    )
    .expect_err("text requires both balanced native events");
    assert!(partial_text.current_event_may_have_effect);
}

#[test]
fn process_commit_arbitrates_cancellation_again_after_the_blocking_post() {
    let cancellation = CancellationToken::new();
    let context = OperationContext::new().with_cancellation(cancellation.clone());
    let invoked = InterruptingProcessSource {
        result: Ok(shim::ProcessPostOutcome {
            invoked_native_units: 1,
            target_match_count: 1,
            authorization: shim::ProcessAuthorizationObservation::Granted,
            geometry: shim::ProcessGeometryObservation::NotApplicable,
        }),
        cancellation,
    };

    let failure = commit_process(
        &invoked,
        None,
        shim::ProcessGeometry::AuthorityOnly,
        &context,
        key_post(0x24, true),
        0,
    )
    .expect_err("cancellation observed after native effect cannot commit success");

    assert_eq!(failure.fault, InputFault::Cancelled);
    assert!(failure.current_event_may_have_effect);

    let cancellation = CancellationToken::new();
    let context = OperationContext::new().with_cancellation(cancellation.clone());
    let untouched = InterruptingProcessSource {
        result: Err(shim::ProcessPostFailure {
            status: ShimStatus::PlatformFailure,
            invoked_native_units: 0,
            target_match_count: 1,
            authorization: shim::ProcessAuthorizationObservation::Unknown,
            geometry: shim::ProcessGeometryObservation::NotEvaluated,
        }),
        cancellation,
    };

    let failure = commit_process(
        &untouched,
        None,
        shim::ProcessGeometry::AuthorityOnly,
        &context,
        key_post(0x24, true),
        0,
    )
    .expect_err("post-call cancellation still distinguishes an untouched event");

    assert_eq!(failure.fault, InputFault::Cancelled);
    assert!(!failure.current_event_may_have_effect);
}

#[test]
fn text_is_chunked_without_ever_splitting_a_surrogate_pair() {
    let units: Vec<u16> = "x".repeat(40).encode_utf16().collect();
    let chunks = text_chunks(&units);

    assert_eq!(chunks.len(), 3);
    assert!(
        chunks
            .iter()
            .all(|chunk| chunk.len() <= shim::INPUT_MAX_TEXT_CHUNK)
    );
    assert_eq!(chunks.iter().map(|chunk| chunk.len()).sum::<usize>(), 40);

    // Fifteen ASCII characters then an astral one puts a high surrogate exactly
    // on the chunk boundary, which is the case that would deliver half a
    // character followed by half of another.
    let mut mixed = "x".repeat(15);
    mixed.push('\u{1F600}');
    mixed.push('y');
    let units: Vec<u16> = mixed.encode_utf16().collect();
    let chunks = text_chunks(&units);

    assert_eq!(
        chunks[0].len(),
        15,
        "the pair moved wholly into the next post"
    );
    for chunk in &chunks {
        let first = units[chunk.start];
        let last = units[chunk.end - 1];
        assert!(
            !(0xDC00..0xE000).contains(&first),
            "a chunk started on a low surrogate"
        );
        assert!(
            !(0xD800..0xDC00).contains(&last),
            "a chunk ended on a high surrogate"
        );
    }
}

#[test]
fn a_point_outside_the_targets_own_rectangle_is_not_deliverable() {
    let geometry = fingerprint((100.0, 50.0), (640.0, 420.0), 2.0);

    assert!(contains_desktop_point(geometry, (100.0, 50.0)));
    assert!(contains_desktop_point(geometry, (739.0, 469.0)));
    assert!(
        !contains_desktop_point(geometry, (740.0, 300.0)),
        "the far edge belongs to whatever is next to the target"
    );
    assert!(!contains_desktop_point(geometry, (99.0, 300.0)));
    assert!(!contains_desktop_point(geometry, (300.0, 470.0)));
}

#[test]
fn a_retina_capture_pixel_resolves_to_the_point_the_desktop_uses() {
    let (extent, placement) = placement_for((0.0, 0.0), (640.0, 420.0), 2.0).expect("retina");
    let transform = TransformSnapshot::with_target(GeometryRevision::FIRST, extent, placement)
        .expect("the placement covers the frame");

    assert_eq!(extent.width(), 1280);
    assert_eq!(extent.height(), 840);

    let far = Point::new(CoordinateSpace::CapturePixels, 1278.0, 838.0).expect("valid");
    let desktop = transform
        .convert_point(far, CoordinateSpace::DesktopLogical)
        .expect("desktop conversion");

    assert_eq!((desktop.x(), desktop.y()), (639.0, 419.0));
    assert!(contains_desktop_point(
        GeometryFingerprint { extent, placement },
        (desktop.x(), desktop.y())
    ));
}

#[test]
fn a_display_left_of_the_main_one_keeps_its_signed_origin() {
    let (extent, placement) =
        placement_for((-1920.0, -240.0), (1920.0, 1080.0), 1.0).expect("secondary display");
    let transform = TransformSnapshot::with_target(GeometryRevision::FIRST, extent, placement)
        .expect("placement covers the frame");

    let origin = Point::new(CoordinateSpace::CapturePixels, 0.0, 0.0).expect("valid");
    let desktop = transform
        .convert_point(origin, CoordinateSpace::DesktopLogical)
        .expect("desktop conversion");

    assert_eq!((desktop.x(), desktop.y()), (-1920.0, -240.0));
    assert!(contains_desktop_point(
        GeometryFingerprint { extent, placement },
        (-1920.0, -240.0)
    ));
}

#[test]
fn a_rectangle_that_cannot_describe_a_capture_extent_is_refused() {
    assert_eq!(
        extent_from_points((640.0, 420.0), 0.0),
        Err(InputFault::UnsupportedCoordinate)
    );
    assert_eq!(
        extent_from_points((0.0, 420.0), 2.0),
        Err(InputFault::UnsupportedCoordinate)
    );
    assert_eq!(
        extent_from_points((f64::NAN, 420.0), 2.0),
        Err(InputFault::UnsupportedCoordinate)
    );
}

#[test]
fn the_pointer_buttons_map_onto_the_three_the_contract_declares() {
    assert_eq!(
        native_button(PointerButton::Primary),
        Ok(shim::INPUT_BUTTON_PRIMARY)
    );
    assert_eq!(
        native_button(PointerButton::Secondary),
        Ok(shim::INPUT_BUTTON_SECONDARY)
    );
    assert_eq!(
        native_button(PointerButton::Middle),
        Ok(shim::INPUT_BUTTON_MIDDLE)
    );
}

#[test]
fn a_move_while_a_button_is_held_is_prepared_as_a_drag() {
    let mut state = DriverState::default();
    assert_eq!(state.dragging(), None);

    state.buttons.push(SystemButtonState {
        logical: PointerButton::Secondary,
        native: shim::INPUT_BUTTON_SECONDARY,
    });

    assert_eq!(
        state.dragging(),
        Some(PointerButton::Secondary),
        "a move reported as a plain move would leave every drag gesture inert"
    );
    assert_eq!(
        native_button(state.dragging().expect("held")),
        Ok(shim::INPUT_BUTTON_SECONDARY)
    );
}
