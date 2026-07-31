//! The input contract suite every input adapter must pass.
//!
//! Written as assertions over the public traits rather than over any
//! implementation, so an adapter passes it for the same reasons a caller can rely
//! on it. Each check panics with a message naming the rule it enforces, because a
//! contract failure is a defect in the adapter and not a value to be handled.
//!
//! # What is here and what is not
//!
//! These are the rules an adapter can be held to without anyone telling it to
//! fail: admission refuses what the descriptor does not advertise, two overlapping
//! sequences each complete without interleaving, a sequence that cannot start
//! delivers nothing, and close stops admission without repeating anything.
//!
//! No check here depends on which of two concurrent sequences the scheduler admits
//! first. That is not a contract rule, and a check that assumed it would fail on a
//! loaded host while naming a rule it never reached. Where a rule genuinely needs
//! contention — a sequence whose deadline passes *while waiting* — it is verified
//! against [`Admission`](mado_pilot_input::Admission) directly, where one thread
//! holds the controller and the same thread observes the refusal.
//!
//! The rules about *failing* part-way — a mechanism that refuses after two events,
//! cleanup that cannot release a modifier — cannot be checked this way, because
//! nothing here can make a working adapter fail on cue. Those are exercised
//! against [`ControlledInput`](crate::ControlledInput), whose behavior a test
//! writes, and a platform adapter meets them through its own fault-injection
//! cases. A suite that pretended to check them would report a pass for a rule it
//! never reached.
//!
//! # Deliverability
//!
//! Every sequence here is bounded, releases whatever it presses, and uses the
//! target the caller names. An adapter that delivers to a real desktop must be
//! given a fixture target of its own: nothing in this suite is safe to point at an
//! application a person is using.

use std::thread;
use std::time::Duration;

use mado_pilot_core::{
    InputDelivery, InputOperationKind, Lifecycle, OperationContext, Status, TargetId,
};
use mado_pilot_input::{
    DeliveryPlan, InputEvent, InputOpenRequest, InputProvider, InputRequest, InputSequence, Key,
    SequenceOutcome,
};

/// How long the sequence in the contention check occupies the controller.
///
/// Long enough that the two sequences genuinely overlap on any host, and nothing
/// in the check depends on which of them the scheduler admits first.
const OCCUPYING_DELAY: Duration = Duration::from_millis(120);

/// Runs every check against `provider` for `target`.
///
/// # Panics
///
/// Panics naming the first rule the adapter does not satisfy.
pub fn run(provider: &dyn InputProvider, target: TargetId) {
    a_described_target_reports_its_own_identity(provider, target);
    a_foreign_target_is_refused(provider, target);
    admission_refuses_what_the_descriptor_does_not_advertise(provider, target);
    an_advertised_sequence_is_delivered_completely(provider, target);
    two_sequences_do_not_interleave_on_one_controller(provider, target);
    a_sequence_that_cannot_start_delivers_nothing(provider, target);
    close_stops_admission_and_is_idempotent(provider, target);
}

/// Returns the context checks that cannot block use.
fn context() -> OperationContext {
    OperationContext::new()
}

/// Returns a keyboard sequence the target's descriptor is asked to support.
fn keystroke() -> InputSequence {
    InputSequence::new(vec![
        InputEvent::KeyPress(Key::Escape),
        InputEvent::KeyRelease(Key::Escape),
    ])
    .expect("a two-event sequence is within every bound")
}

/// Returns a sequence that holds the controller for [`OCCUPYING_DELAY`].
fn occupying() -> InputSequence {
    InputSequence::new(vec![
        InputEvent::Delay(OCCUPYING_DELAY),
        InputEvent::KeyPress(Key::Escape),
        InputEvent::KeyRelease(Key::Escape),
    ])
    .expect("a delay within the event bound")
}

/// Returns the first delivery mechanism the descriptor advertises for keystrokes.
fn keyboard_mechanism(provider: &dyn InputProvider, target: TargetId) -> InputDelivery {
    let descriptor = provider
        .describe(target, &context())
        .expect("input contract: `describe` must report a target this provider issued");
    InputDelivery::ALL
        .into_iter()
        .find(|delivery| {
            descriptor
                .capability()
                .supports(InputOperationKind::Keyboard, *delivery)
        })
        .expect(
            "input contract: this suite needs a target that accepts keyboard input \
             through some mechanism; a capture-only target has no input contract to check",
        )
}

/// The description names the target it was asked about.
///
/// # Panics
///
/// Panics when the descriptor describes another target.
pub fn a_described_target_reports_its_own_identity(provider: &dyn InputProvider, target: TargetId) {
    let descriptor = provider
        .describe(target, &context())
        .expect("input contract: `describe` must report a target this provider issued");

    assert_eq!(
        descriptor.target(),
        target,
        "input contract: a descriptor must describe the target it was asked about"
    );
}

/// A target from another engine or provider is refused.
///
/// # Panics
///
/// Panics when a foreign identity is accepted.
pub fn a_foreign_target_is_refused(provider: &dyn InputProvider, target: TargetId) {
    let foreign = mado_pilot_core::IdentityIssuer::new()
        .issue_target(provider.provider())
        .expect("issued");
    assert_ne!(
        foreign, target,
        "the foreign identity must be a different one"
    );

    let error = provider
        .describe(foreign, &context())
        .expect_err("input contract: a target from another engine must be refused");

    assert_eq!(
        error.status(),
        Status::InvalidArgument,
        "input contract: a foreign target is a caller mistake, not a platform failure"
    );
}

/// A combination the descriptor does not advertise fails before any delivery.
///
/// # Panics
///
/// Panics when an unadvertised combination is admitted, or when the failure is not
/// an unsupported outcome.
pub fn admission_refuses_what_the_descriptor_does_not_advertise(
    provider: &dyn InputProvider,
    target: TargetId,
) {
    let descriptor = provider
        .describe(target, &context())
        .expect("input contract: `describe` must report a target this provider issued");
    let Some(unadvertised) = InputDelivery::ALL.into_iter().find(|delivery| {
        !descriptor
            .capability()
            .supports(InputOperationKind::Keyboard, *delivery)
    }) else {
        // Every mechanism accepts keystrokes, so there is no unadvertised
        // combination to refuse. Nothing is wrong; the rule has no case here.
        return;
    };
    let controller = provider
        .open(target, &InputOpenRequest::new(), &context())
        .expect("input contract: opening an optional-input controller must succeed");

    let error = controller
        .execute(
            &InputRequest::new(target, keystroke(), DeliveryPlan::require(unadvertised)),
            &context(),
        )
        .expect_err(
            "input contract: a required mechanism the descriptor does not advertise must be \
             refused rather than substituted",
        );

    assert_eq!(
        error.status(),
        Status::Unsupported,
        "input contract: an unadvertised combination is unsupported"
    );
    controller.close(&context()).expect("close");
}

/// An advertised sequence produces a complete receipt naming its mechanism.
///
/// # Panics
///
/// Panics when the receipt is not complete, when it counts the wrong number of
/// events, or when it names a mechanism the request did not permit.
pub fn an_advertised_sequence_is_delivered_completely(
    provider: &dyn InputProvider,
    target: TargetId,
) {
    let mechanism = keyboard_mechanism(provider, target);
    let controller = provider
        .open(target, &InputOpenRequest::new(), &context())
        .expect("input contract: opening an optional-input controller must succeed");
    let sequence = keystroke();
    let expected = sequence.len();

    let receipt = controller
        .execute(
            &InputRequest::new(target, sequence, DeliveryPlan::require(mechanism)),
            &context(),
        )
        .expect("input contract: an advertised sequence must be admitted");

    assert_eq!(
        receipt.outcome(),
        SequenceOutcome::Complete,
        "input contract: an advertised sequence with no injected failure must complete"
    );
    assert_eq!(
        receipt.delivered(),
        expected,
        "input contract: a complete receipt counts every event"
    );
    assert_eq!(
        receipt.delivery(),
        Some(mechanism),
        "input contract: the receipt names the mechanism the caller permitted"
    );
    assert_eq!(
        receipt.target(),
        target,
        "input contract: the receipt names the target it was addressed to"
    );
    controller.close(&context()).expect("close");
}

/// Two overlapping sequences each complete, without interleaving.
///
/// Deliberately order-free. Which sequence the scheduler admits first is not a
/// contract rule, and a check that assumed an order would fail on a loaded host
/// while naming a rule it never reached. What the contract does require is that
/// both are admitted eventually and that each reports its own events — a
/// controller that let them interleave could not.
///
/// # Panics
///
/// Panics when either sequence fails to be admitted, or when either receipt counts
/// the other's events.
pub fn two_sequences_do_not_interleave_on_one_controller(
    provider: &dyn InputProvider,
    target: TargetId,
) {
    let mechanism = keyboard_mechanism(provider, target);
    let controller = provider
        .open(target, &InputOpenRequest::new(), &context())
        .expect("input contract: opening an optional-input controller must succeed");
    let generous = OperationContext::new()
        .with_timeout(OCCUPYING_DELAY.saturating_mul(8))
        .expect("representable");
    let occupying_events = occupying().len();
    let keystroke_events = keystroke().len();
    assert_ne!(
        occupying_events, keystroke_events,
        "the two sequences must differ in length for their receipts to be told apart"
    );

    let (first, second) = thread::scope(|scope| {
        let occupier = scope.spawn(|| {
            controller.execute(
                &InputRequest::new(target, occupying(), DeliveryPlan::require(mechanism)),
                &generous,
            )
        });
        let waiting = controller.execute(
            &InputRequest::new(target, keystroke(), DeliveryPlan::require(mechanism)),
            &generous,
        );
        (
            occupier.join().expect("the occupying sequence finished"),
            waiting,
        )
    });

    let first = first.expect("input contract: the occupying sequence must be admitted");
    let second = second.expect(
        "input contract: a sequence that waits for the controller within its deadline must \
         be admitted once the controller is free",
    );

    assert_eq!(
        first.outcome(),
        SequenceOutcome::Complete,
        "input contract: a sequence with a generous deadline must finish"
    );
    assert_eq!(
        second.outcome(),
        SequenceOutcome::Complete,
        "input contract: a sequence with a generous deadline must finish"
    );
    assert_eq!(
        first.delivered(),
        occupying_events,
        "input contract: each receipt counts its own sequence, not what overlapped it"
    );
    assert_eq!(
        second.delivered(),
        keystroke_events,
        "input contract: each receipt counts its own sequence, not what overlapped it"
    );
    controller.close(&context()).expect("close");
}

/// A sequence that cannot start delivers nothing.
///
/// The deterministic half of the rule about a sequence that never acquires the
/// controller: an operation that is already past its deadline cannot start,
/// whatever else is or is not running. The contended half is exercised where it can
/// be made deterministic — against the admission gate itself, where one thread
/// holds the controller and the same thread observes the refusal.
///
/// # Panics
///
/// Panics when the sequence delivers an event, or reports anything but an
/// unexecuted outcome or an expired operation.
pub fn a_sequence_that_cannot_start_delivers_nothing(
    provider: &dyn InputProvider,
    target: TargetId,
) {
    let mechanism = keyboard_mechanism(provider, target);
    let controller = provider
        .open(target, &InputOpenRequest::new(), &context())
        .expect("input contract: opening an optional-input controller must succeed");
    let expired = OperationContext::new()
        .with_timeout(Duration::ZERO)
        .expect("representable");

    let outcome = controller.execute(
        &InputRequest::new(target, keystroke(), DeliveryPlan::require(mechanism)),
        &expired,
    );

    match outcome {
        // Either shape is truthful: an adapter may report the interruption as the
        // operation's error, or as an unexecuted receipt. What it may not do is
        // deliver an event.
        Err(error) => assert_eq!(
            error.status(),
            Status::DeadlineExceeded,
            "input contract: a sequence that never ran reports why it did not"
        ),
        Ok(receipt) => {
            assert_eq!(
                receipt.outcome(),
                SequenceOutcome::Unexecuted,
                "input contract: a sequence that could not start delivered nothing"
            );
            assert_eq!(
                receipt.delivered(),
                0,
                "input contract: an unexecuted receipt counts no events"
            );
            assert_eq!(
                receipt.delivery(),
                None,
                "input contract: nothing was delivered, so no mechanism delivered it"
            );
        }
    }
    controller.close(&context()).expect("close");
}

/// Close stops admission, and a retried close changes nothing.
///
/// # Panics
///
/// Panics when a closed controller admits a sequence, when close is not
/// idempotent, or when the lifecycle does not reach closed.
pub fn close_stops_admission_and_is_idempotent(provider: &dyn InputProvider, target: TargetId) {
    let mechanism = keyboard_mechanism(provider, target);
    let controller = provider
        .open(target, &InputOpenRequest::new(), &context())
        .expect("input contract: opening an optional-input controller must succeed");

    controller
        .close(&context())
        .expect("input contract: close must succeed for an idle controller");
    assert_eq!(
        controller.lifecycle(),
        Lifecycle::Closed,
        "input contract: a drained close reaches the closed state"
    );
    assert!(
        controller.is_closed(),
        "input contract: the derived answer must agree with the lifecycle"
    );
    controller
        .close(&context())
        .expect("input contract: close must be idempotent");

    let error = controller
        .execute(
            &InputRequest::new(target, keystroke(), DeliveryPlan::require(mechanism)),
            &context(),
        )
        .expect_err("input contract: a closed controller must admit nothing");

    assert_eq!(
        error.status(),
        Status::Closed,
        "input contract: a closed controller reports that it is closed"
    );
}
