//! The input contract suite against the controlled double, plus the scenarios
//! that need a controller told to fail.
//!
//! The suite covers the rules an adapter can be held to unprompted. The scenarios
//! below are the ones the specification states about failing part-way, and they are
//! only reachable through a double: nothing can make a working adapter refuse its
//! third event on cue, and a rule that is never reached is a rule that is not
//! verified.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use mado_pilot_core::{
    CancellationToken, CapabilitySupport, CoordinateSpace, IdentityIssuer, InputCapability,
    InputDelivery, InputOperationKind, Lifecycle, OperationContext, Point, ProviderId, Status,
    SubmissionEvidence, TargetId,
};
use mado_pilot_input::{
    CleanupBudget, CleanupState, DeliveryPlan, InputEvent, InputFault, InputOpenRequest,
    InputProvider, InputRequest, InputRequirement, InputSequence, Key, Modifier, PointerButton,
    PressedState, SequenceOutcome, check_provider_pair,
};
use mado_pilot_testkit::controlled_input::{Behavior, Cleanup, PROVIDER};
use mado_pilot_testkit::{ControlledInput, input_contract};

fn target() -> TargetId {
    IdentityIssuer::new()
        .issue_target(PROVIDER)
        .expect("issued")
}

fn context() -> OperationContext {
    OperationContext::new()
}

fn chord() -> InputSequence {
    InputSequence::new(vec![
        InputEvent::KeyPress(Key::Modifier(Modifier::Control)),
        InputEvent::KeyPress(Key::Character('c')),
        InputEvent::KeyRelease(Key::Character('c')),
        InputEvent::KeyRelease(Key::Modifier(Modifier::Control)),
    ])
    .expect("valid")
}

fn system(target: TargetId, sequence: InputSequence) -> InputRequest {
    InputRequest::new(
        target,
        sequence,
        DeliveryPlan::require(InputDelivery::System),
    )
}

/// Waits until `predicate` holds, failing rather than hanging if it never does.
///
/// A test that needs a sequence to be holding the controller waits for that fact
/// instead of sleeping for however long it is guessed to take. A sleep long enough
/// to be safe on a loaded runner is a slow test, and one short enough to be fast is
/// a test that fails there while naming a rule it never reached.
fn wait_until(what: &str, predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !predicate() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::yield_now();
    }
}

#[test]
fn the_controlled_double_satisfies_the_input_contract() {
    let target = target();

    input_contract::run(&ControlledInput::new(target), target);
}

#[test]
fn a_partially_submitted_route_returns_a_partial_receipt_and_does_not_fallback() {
    let target = target();
    let provider = ControlledInput::with_capability(
        target,
        InputCapability::none()
            .with_pair(
                InputOperationKind::Keyboard,
                InputDelivery::System,
                CapabilitySupport::Supported,
                SubmissionEvidence::SystemInputAdmission,
            )
            .with_pair(
                InputOperationKind::Keyboard,
                InputDelivery::WindowMessage,
                CapabilitySupport::Unknown,
                SubmissionEvidence::TargetQueueAdmission,
            ),
    );
    provider.set_behavior(Behavior::FailAfter {
        submitted: 2,
        fault: InputFault::PolicyRefused,
    });
    let controller = provider
        .open(target, &InputOpenRequest::new(), &context())
        .expect("opened");

    let receipt = controller
        .execute(
            &InputRequest::new(
                target,
                chord(),
                DeliveryPlan::ordered(vec![InputDelivery::System, InputDelivery::WindowMessage])
                    .expect("valid"),
            ),
            &context(),
        )
        .expect("an admitted sequence always produces a receipt");

    assert_eq!(receipt.outcome(), SequenceOutcome::Partial);
    assert_eq!(receipt.submitted(), 2);
    assert_eq!(receipt.fault(), Some(InputFault::PolicyRefused));
    assert_eq!(
        receipt.selected_route(),
        Some(InputDelivery::System),
        "the partially submitted route is terminal"
    );
    assert_eq!(receipt.attempts().len(), 1);
    assert_eq!(receipt.attempts()[0].route(), InputDelivery::System);
    assert_eq!(receipt.attempts()[0].submitted(), 2);
    assert!(
        provider
            .submitted_events()
            .iter()
            .all(|submitted| submitted.route == InputDelivery::System),
        "no event reached the caller's fallback route"
    );
}

#[test]
fn cleanup_that_cannot_release_everything_reports_its_exact_counts() {
    let target = target();
    let provider = ControlledInput::new(target);
    provider.set_behavior(Behavior::FailAfter {
        submitted: 2,
        fault: InputFault::SubmissionFailed,
    });
    provider.set_cleanup(Cleanup::Partial(1));
    let controller = provider
        .open(target, &InputOpenRequest::new(), &context())
        .expect("opened");

    let receipt = controller
        .execute(&system(target, chord()), &context())
        .expect("executed");

    assert_eq!(receipt.cleanup(), CleanupState::Incomplete);
    assert!(receipt.cleanup().may_leave_state_held());
    assert_eq!(receipt.cleanup_released(), 1);
    assert_eq!(receipt.cleanup_owed(), 2);
    assert_eq!(
        provider.released(),
        vec![PressedState::Key(Key::Character('c'))],
        "cleanup released what it could, newest first"
    );
}

#[test]
fn cleanup_releases_only_what_the_sequence_itself_pressed() {
    let target = target();
    let provider = ControlledInput::new(target);
    provider.set_behavior(Behavior::FailAfter {
        submitted: 2,
        fault: InputFault::SubmissionFailed,
    });
    let controller = provider
        .open(target, &InputOpenRequest::new(), &context())
        .expect("opened");
    // A press, a move, and then the failure: the move pressed nothing, so cleanup
    // owes one release rather than two.
    let sequence = InputSequence::new(vec![
        InputEvent::PointerPress(PointerButton::Primary),
        InputEvent::PointerMove(Point::new(CoordinateSpace::CapturePixels, 1.0, 1.0).expect("ok")),
        InputEvent::PointerRelease(PointerButton::Primary),
    ])
    .expect("valid");

    let receipt = controller
        .execute(&system(target, sequence), &context())
        .expect("executed");

    assert_eq!(receipt.cleanup_owed(), 1);
    assert_eq!(receipt.cleanup(), CleanupState::Complete);
    assert_eq!(
        provider.released(),
        vec![PressedState::Button(PointerButton::Primary)]
    );
}

#[test]
fn a_cleanup_that_runs_out_of_events_is_not_a_refused_release() {
    let target = target();
    let provider = ControlledInput::new(target);
    provider.set_behavior(Behavior::FailAfter {
        submitted: 2,
        fault: InputFault::SubmissionFailed,
    });
    let controller = provider
        .open(target, &InputOpenRequest::new(), &context())
        .expect("opened");
    // The chord holds two states after two events; the budget allows one release.
    let budget = CleanupBudget::at_most(1, CleanupBudget::MAX_DURATION);

    let receipt = controller
        .execute(
            &system(target, chord()).with_cleanup_budget(budget),
            &context(),
        )
        .expect("executed");

    assert_eq!(
        receipt.cleanup(),
        CleanupState::Exhausted,
        "nothing refused the second release; the budget stopped it"
    );
    assert!(receipt.cleanup().may_leave_state_held());
    assert_eq!(receipt.cleanup_released(), 1);
    assert_eq!(receipt.cleanup_owed(), 2);
    assert_eq!(
        provider.released(),
        vec![PressedState::Key(Key::Character('c'))],
        "the one release the budget allowed went through, newest first"
    );
}

#[test]
fn a_cleanup_whose_time_runs_out_stops_without_refusing_anything() {
    let target = target();
    let provider = ControlledInput::new(target);
    provider.set_behavior(Behavior::FailAfter {
        submitted: 2,
        fault: InputFault::SubmissionFailed,
    });
    let controller = provider
        .open(target, &InputOpenRequest::new(), &context())
        .expect("opened");
    // A budget whose time is already spent: the events are allowed, the clock is not.
    let budget = CleanupBudget::at_most(CleanupBudget::MAX_EVENTS, Duration::ZERO);

    let receipt = controller
        .execute(
            &system(target, chord()).with_cleanup_budget(budget),
            &context(),
        )
        .expect("executed");

    assert_eq!(receipt.cleanup(), CleanupState::Exhausted);
    assert_eq!(receipt.cleanup_released(), 0);
    assert_eq!(receipt.cleanup_owed(), 2);
    assert!(
        provider.released().is_empty(),
        "no release was attempted, so none can have been refused"
    );
}

#[test]
fn cleanup_runs_even_though_the_cancellation_is_what_stopped_the_sequence() {
    let target = target();
    let provider = ControlledInput::new(target);
    let controller = provider
        .open(target, &InputOpenRequest::new(), &context())
        .expect("opened");
    // Press a modifier, then wait. The canceller fires once the press is recorded,
    // so the check before the next irreversible event is the one that fails, and
    // the sequence stops holding exactly the modifier.
    let sequence = InputSequence::new(vec![
        InputEvent::KeyPress(Key::Modifier(Modifier::Control)),
        InputEvent::Delay(Duration::from_millis(60)),
        InputEvent::KeyPress(Key::Character('c')),
        InputEvent::KeyRelease(Key::Character('c')),
        InputEvent::KeyRelease(Key::Modifier(Modifier::Control)),
    ])
    .expect("valid");
    let token = CancellationToken::new();
    let cancelled = OperationContext::new().with_cancellation(token.clone());

    let receipt = thread::scope(|scope| {
        let worker = scope.spawn(|| controller.execute(&system(target, sequence), &cancelled));
        wait_until("the modifier to be pressed", || {
            !provider.submitted_events().is_empty()
        });
        token.cancel();
        worker.join().expect("the sequence finished")
    })
    .expect("an admitted sequence always produces a receipt");

    assert_eq!(
        receipt.outcome(),
        SequenceOutcome::Partial,
        "the cancellation stopped it part-way"
    );
    assert_eq!(receipt.fault(), Some(InputFault::Cancelled));
    assert_eq!(
        receipt.cleanup(),
        CleanupState::Complete,
        "cleanup ran under its own budget rather than the cancelled context that caused it"
    );
    assert_eq!(
        provider.released(),
        vec![PressedState::Key(Key::Modifier(Modifier::Control))],
        "the modifier the sequence pressed did not stay held"
    );
}

#[test]
fn a_failed_cleanup_still_reports_what_it_owed() {
    let target = target();
    let provider = ControlledInput::new(target);
    provider.set_behavior(Behavior::FailAfter {
        submitted: 1,
        fault: InputFault::TargetLost,
    });
    provider.set_cleanup(Cleanup::Fails);
    let controller = provider
        .open(target, &InputOpenRequest::new(), &context())
        .expect("opened");

    let receipt = controller
        .execute(&system(target, chord()), &context())
        .expect("executed");

    assert_eq!(receipt.cleanup(), CleanupState::Incomplete);
    assert_eq!(receipt.cleanup_released(), 0);
    assert_eq!(receipt.cleanup_owed(), 1);
    assert!(provider.released().is_empty());
}

#[test]
fn close_racing_a_sequence_leaves_one_truthful_receipt_and_no_later_event() {
    let target = target();
    let provider = ControlledInput::new(target);
    let controller = provider
        .open(target, &InputOpenRequest::new(), &context())
        .expect("opened");
    let occupying = InputSequence::new(vec![
        InputEvent::Delay(Duration::from_millis(80)),
        InputEvent::KeyPress(Key::Escape),
        InputEvent::KeyRelease(Key::Escape),
    ])
    .expect("valid");
    let generous = OperationContext::new()
        .with_timeout(Duration::from_secs(5))
        .expect("representable");

    let (running, closed) = thread::scope(|scope| {
        let worker = scope.spawn(|| controller.execute(&system(target, occupying), &generous));
        wait_until("the sequence to hold the controller", || {
            provider.executing() == 1
        });
        let closed = controller.close(&generous);
        (worker.join().expect("the sequence finished"), closed)
    });

    let receipt = running.expect("the admitted sequence produced its receipt");
    closed.expect("close drains rather than failing");
    assert_eq!(
        receipt.outcome(),
        SequenceOutcome::Complete,
        "a sequence already admitted when close began finishes and reports once"
    );
    assert_eq!(controller.lifecycle(), Lifecycle::Closed);

    let submitted_before = provider.submitted_events().len();
    let error = controller
        .execute(&system(target, chord()), &context())
        .expect_err("a closed controller admits nothing");
    assert_eq!(error.status(), Status::Closed);
    assert_eq!(
        provider.submitted_events().len(),
        submitted_before,
        "no ordinary event begins after close"
    );
}

#[test]
fn required_input_refuses_a_target_that_accepts_none() {
    let target = target();
    let provider = ControlledInput::with_capability(target, InputCapability::none());

    let optional = provider.open(target, &InputOpenRequest::new(), &context());
    let required = provider.open(
        target,
        &InputOpenRequest::new().with_requirement(InputRequirement::Required),
        &context(),
    );

    assert!(
        optional.is_ok(),
        "optional input opens capture-only rather than failing"
    );
    assert_eq!(
        required
            .expect_err("required input is unavailable")
            .status(),
        Status::Unsupported
    );
}

#[test]
fn a_required_combination_absent_from_the_capability_refuses_the_open() {
    let target = target();
    let provider = ControlledInput::with_capability(
        target,
        InputCapability::none().with_pair(
            InputOperationKind::Keyboard,
            InputDelivery::System,
            CapabilitySupport::Supported,
            SubmissionEvidence::SystemInputAdmission,
        ),
    );

    let refused = provider.open(
        target,
        &InputOpenRequest::new()
            .requiring(InputOperationKind::Keyboard, InputDelivery::WindowMessage),
        &context(),
    );

    assert_eq!(
        refused
            .expect_err("the exact-window keyboard route was never advertised")
            .status(),
        Status::Unsupported
    );
}

#[test]
fn an_input_provider_is_only_wired_to_its_own_capture_provider() {
    let provider = ControlledInput::new(target());

    assert!(check_provider_pair(provider.provider(), PROVIDER).is_ok());
    assert_eq!(
        check_provider_pair(ProviderId::new("replay"), provider.provider())
            .expect_err("a mismatch is refused")
            .status(),
        Status::InvalidArgument
    );
}

#[test]
fn a_controller_survives_being_shared_across_threads() {
    let target = target();
    let provider = Arc::new(ControlledInput::new(target));
    let controller = provider
        .open(target, &InputOpenRequest::new(), &context())
        .expect("opened");
    let generous = OperationContext::new()
        .with_timeout(Duration::from_secs(5))
        .expect("representable");

    thread::scope(|scope| {
        for _ in 0..4 {
            let controller = Arc::clone(&controller);
            let generous = generous.clone();
            scope.spawn(move || {
                controller
                    .execute(&system(target, chord()), &generous)
                    .expect("executed");
            });
        }
    });

    assert_eq!(
        provider.submitted_events().len(),
        16,
        "four sequences of four events each, none of them interleaved"
    );
}
