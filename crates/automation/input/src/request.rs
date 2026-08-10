//! One bounded sequence, and the request that asks a controller to deliver it.

use std::fmt;

use mado_pilot_core::{InputCapability, InputDelivery, InputOperationKind, TargetId};

use crate::cleanup::CleanupBudget;
use crate::event::{InputEvent, PressedState};
use crate::fault::InputFault;
use crate::policy::{DeliveryPlan, FocusPolicy, PointerGeometry};

/// How many events one sequence may hold, and what it may hold.
///
/// The limits are part of the descriptor a controller reports, so an Adapter may
/// advertise a smaller ceiling than the contract's own. Nothing may advertise a
/// larger one: the contract ceiling is what bounds the work one admitted sequence
/// can occupy a controller with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SequenceLimits {
    max_events: usize,
}

impl SequenceLimits {
    /// The most events version one accepts in one sequence.
    ///
    /// Two hundred and fifty-six is generous for the sequences automation actually
    /// sends — a chord, a click, a short phrase — and small enough that one
    /// admitted sequence cannot occupy a controller indefinitely. A caller with
    /// more to do sends more sequences, each of which is separately admitted,
    /// separately interruptible, and separately receipted.
    pub const MAX_EVENTS: usize = 256;

    /// The contract's own ceiling.
    #[must_use]
    pub const fn contract() -> Self {
        Self {
            max_events: Self::MAX_EVENTS,
        }
    }

    /// A ceiling an Adapter advertises, clamped to the contract's.
    ///
    /// Clamping rather than refusing, because an Adapter asking for a larger bound
    /// is asking for something the contract does not offer, and the honest response
    /// is the bound that actually applies.
    #[must_use]
    pub const fn at_most(max_events: usize) -> Self {
        Self {
            max_events: if max_events < Self::MAX_EVENTS {
                max_events
            } else {
                Self::MAX_EVENTS
            },
        }
    }

    /// Returns the most events one sequence may hold.
    #[must_use]
    pub const fn max_events(self) -> usize {
        self.max_events
    }
}

impl Default for SequenceLimits {
    fn default() -> Self {
        Self::contract()
    }
}

impl fmt::Display for SequenceLimits {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "at most {} events", self.max_events)
    }
}

/// An ordered, bounded run of input events.
///
/// A sequence is validated when it is built, so a controller that has one has one
/// it can execute rather than one it has to re-check event by event. What it still
/// checks per event is the operation context, because that is what can change while
/// the sequence runs.
#[derive(Debug, Clone, PartialEq)]
pub struct InputSequence {
    events: Vec<InputEvent>,
}

impl InputSequence {
    /// Builds a sequence from `events`, under the contract's own limits.
    ///
    /// # Errors
    ///
    /// As [`InputSequence::within`].
    pub fn new(events: Vec<InputEvent>) -> Result<Self, InputFault> {
        Self::within(events, SequenceLimits::contract())
    }

    /// Builds a sequence from `events`, under `limits`.
    ///
    /// # Errors
    ///
    /// Returns [`InputFault::SequenceOutOfBounds`] for an empty sequence, one
    /// longer than `limits`, or one containing an event outside its own bound. An
    /// empty sequence is refused rather than treated as success, because a caller
    /// that sent nothing wanted something to happen.
    pub fn within(events: Vec<InputEvent>, limits: SequenceLimits) -> Result<Self, InputFault> {
        if events.is_empty() || events.len() > limits.max_events() {
            return Err(InputFault::SequenceOutOfBounds);
        }
        for event in &events {
            event.check()?;
        }
        Ok(Self { events })
    }

    /// Returns the events in order.
    #[must_use]
    pub fn events(&self) -> &[InputEvent] {
        &self.events
    }

    /// Returns how many events the sequence holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Reports whether the sequence holds no events.
    ///
    /// Always false for a sequence that exists; present because a length without
    /// one reads as an oversight.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Returns every operation kind the sequence performs.
    ///
    /// Admission checks each of these against the accepted descriptor, so a
    /// sequence that mixes a supported click with an unsupported keystroke is
    /// refused before the click reaches a native route.
    #[must_use]
    pub fn operation_kinds(&self) -> Vec<InputOperationKind> {
        let mut kinds = Vec::new();
        for kind in self.events.iter().filter_map(InputEvent::operation_kind) {
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
        }
        kinds
    }

    /// Returns what is still held after the first `submitted` events.
    ///
    /// This is what cleanup releases after a partial failure: state this sequence
    /// pressed and did not release, in reverse order of pressing, so a modifier
    /// pressed first is released last.
    #[must_use]
    pub fn held_after(&self, submitted: usize) -> Vec<PressedState> {
        let mut held: Vec<PressedState> = Vec::new();
        for event in self.events.iter().take(submitted) {
            if let Some(pressed) = event.presses() {
                held.push(pressed);
            }
            if let Some(released) = event.releases()
                && let Some(index) = held.iter().rposition(|state| *state == released)
            {
                held.remove(index);
            }
        }
        held.reverse();
        held
    }

    /// Returns state that may remain held when the current incomplete event may
    /// have had native effect.
    ///
    /// `submitted` counts complete logical events. If the next event is a press,
    /// cleanup must conservatively release it before state held by the complete
    /// prefix. A partial release adds no obligation; releasing an already-released
    /// sequence-owned state remains bounded by the complete-prefix obligation.
    #[must_use]
    pub fn possibly_held_after(
        &self,
        submitted: usize,
        current_event_may_have_effect: bool,
    ) -> Vec<PressedState> {
        let mut held = self.held_after(submitted);
        if current_event_may_have_effect
            && let Some(pressed) = self.events.get(submitted).and_then(InputEvent::presses)
            && !held.contains(&pressed)
        {
            held.insert(0, pressed);
        }
        held
    }

    /// Reports whether every operation is supported or safely attemptable over
    /// `route`.
    #[must_use]
    pub fn may_submit_via(&self, capability: InputCapability, route: InputDelivery) -> bool {
        self.operation_kinds()
            .into_iter()
            .all(|kind| capability.pair(kind, route).may_attempt())
    }
}

/// One ask: deliver this sequence to this target under these policies.
///
/// A typed request rather than a method per primitive. Delivery selection, focus,
/// geometry resolution, admission, partial receipts, and cleanup are the same for
/// every primitive, and a method per primitive would have duplicated all of them
/// five times over.
#[derive(Debug, Clone, PartialEq)]
pub struct InputRequest {
    target: TargetId,
    sequence: InputSequence,
    delivery: DeliveryPlan,
    focus: FocusPolicy,
    pointer_geometry: PointerGeometry,
    cleanup: CleanupBudget,
}

impl InputRequest {
    /// Asks for `sequence` on `target` through `delivery`.
    ///
    /// Focus defaults to [`FocusPolicy::Preserve`] and pointer geometry to
    /// reprojection: the two choices that change nothing the caller did not ask
    /// for. Cleanup defaults to the contract's own bounds. The deadline and
    /// cancellation are not part of the request — they travel in the operation
    /// context, as they do for every other blocking operation.
    #[must_use]
    pub fn new(target: TargetId, sequence: InputSequence, delivery: DeliveryPlan) -> Self {
        Self {
            target,
            sequence,
            delivery,
            focus: FocusPolicy::Preserve,
            pointer_geometry: PointerGeometry::reprojected(),
            cleanup: CleanupBudget::contract(),
        }
    }

    /// Sets what the controller may do about focus.
    #[must_use]
    pub fn with_focus(mut self, focus: FocusPolicy) -> Self {
        self.focus = focus;
        self
    }

    /// Sets how pointer positions resolve.
    #[must_use]
    pub fn with_pointer_geometry(mut self, geometry: PointerGeometry) -> Self {
        self.pointer_geometry = geometry;
        self
    }

    /// Sets the bounds the releases after a partial failure run under.
    ///
    /// Tightening this trades a shorter worst-case hold on the controller for a
    /// greater chance that a release is never attempted, which the receipt then
    /// reports as [`CleanupState::Exhausted`](crate::CleanupState::Exhausted).
    #[must_use]
    pub fn with_cleanup_budget(mut self, cleanup: CleanupBudget) -> Self {
        self.cleanup = cleanup;
        self
    }

    /// Returns the target.
    #[must_use]
    pub const fn target(&self) -> TargetId {
        self.target
    }

    /// Returns the sequence to deliver.
    #[must_use]
    pub const fn sequence(&self) -> &InputSequence {
        &self.sequence
    }

    /// Returns the permitted delivery mechanisms, in order.
    #[must_use]
    pub const fn delivery(&self) -> &DeliveryPlan {
        &self.delivery
    }

    /// Returns the focus policy.
    #[must_use]
    pub const fn focus(&self) -> FocusPolicy {
        self.focus
    }

    /// Returns how pointer positions resolve.
    #[must_use]
    pub const fn pointer_geometry(&self) -> PointerGeometry {
        self.pointer_geometry
    }

    /// Returns the bounds cleanup runs under after a partial failure.
    #[must_use]
    pub const fn cleanup_budget(&self) -> CleanupBudget {
        self.cleanup
    }

    /// Checks the request against itself, before any target is consulted.
    ///
    /// # Errors
    ///
    /// Returns [`InputFault::MissingCoordinateSource`] when the geometry policy
    /// needs a source frame the request does not carry.
    pub fn check(&self) -> Result<(), InputFault> {
        if self
            .sequence
            .operation_kinds()
            .contains(&InputOperationKind::Pointer)
        {
            self.pointer_geometry.check()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{InputRequest, InputSequence, SequenceLimits};
    use crate::event::{InputEvent, Key, Modifier, PointerButton, PressedState};
    use crate::fault::InputFault;
    use crate::policy::{DeliveryPlan, FocusPolicy, GeometryPolicy, PointerGeometry};
    use mado_pilot_core::{
        CapabilitySupport, CoordinateSpace, IdentityIssuer, InputCapability, InputDelivery,
        InputOperationKind, Point, ProviderId, SubmissionEvidence, TargetId,
    };

    fn target() -> TargetId {
        IdentityIssuer::new()
            .issue_target(ProviderId::new("fake"))
            .expect("issued")
    }

    fn point() -> Point {
        Point::new(CoordinateSpace::CapturePixels, 3.0, 5.0).expect("valid")
    }

    fn click() -> InputSequence {
        InputSequence::new(vec![
            InputEvent::PointerMove(point()),
            InputEvent::PointerPress(PointerButton::Primary),
            InputEvent::PointerRelease(PointerButton::Primary),
        ])
        .expect("valid")
    }

    #[test]
    fn an_empty_sequence_is_refused() {
        assert_eq!(
            InputSequence::new(Vec::new()),
            Err(InputFault::SequenceOutOfBounds)
        );
    }

    #[test]
    fn a_sequence_longer_than_its_limit_is_refused() {
        let events = vec![InputEvent::KeyPress(Key::Space); SequenceLimits::MAX_EVENTS];

        assert!(InputSequence::new(events.clone()).is_ok());
        let mut too_many = events;
        too_many.push(InputEvent::KeyPress(Key::Space));
        assert_eq!(
            InputSequence::new(too_many),
            Err(InputFault::SequenceOutOfBounds)
        );
    }

    #[test]
    fn an_adapter_limit_below_the_contract_applies() {
        let limits = SequenceLimits::at_most(2);

        assert_eq!(limits.max_events(), 2);
        assert_eq!(
            InputSequence::within(
                vec![
                    InputEvent::KeyPress(Key::Space),
                    InputEvent::KeyRelease(Key::Space),
                    InputEvent::KeyPress(Key::Tab),
                ],
                limits
            ),
            Err(InputFault::SequenceOutOfBounds)
        );
    }

    #[test]
    fn an_adapter_cannot_advertise_a_larger_bound_than_the_contract() {
        assert_eq!(
            SequenceLimits::at_most(SequenceLimits::MAX_EVENTS * 4).max_events(),
            SequenceLimits::MAX_EVENTS
        );
    }

    #[test]
    fn a_sequence_with_an_out_of_bound_event_is_refused_whole() {
        assert_eq!(
            InputSequence::new(vec![
                InputEvent::KeyPress(Key::Space),
                InputEvent::Delay(InputEvent::MAX_DELAY + Duration::from_secs(1)),
            ]),
            Err(InputFault::SequenceOutOfBounds)
        );
    }

    #[test]
    fn a_sequence_reports_every_operation_kind_it_performs_once() {
        let sequence = InputSequence::new(vec![
            InputEvent::PointerMove(point()),
            InputEvent::PointerPress(PointerButton::Primary),
            InputEvent::Delay(Duration::from_millis(10)),
            InputEvent::Text("hi".to_owned()),
        ])
        .expect("valid");

        assert_eq!(
            sequence.operation_kinds(),
            vec![InputOperationKind::Pointer, InputOperationKind::Text]
        );
    }

    #[test]
    fn a_mixed_sequence_needs_every_kind_supported_by_one_mechanism() {
        let sequence = InputSequence::new(vec![
            InputEvent::PointerPress(PointerButton::Primary),
            InputEvent::KeyPress(Key::Enter),
        ])
        .expect("valid");
        let pointer_only = InputCapability::none().with_pair(
            InputOperationKind::Pointer,
            InputDelivery::System,
            CapabilitySupport::Supported,
            SubmissionEvidence::SystemInputAdmission,
        );
        let both = pointer_only.with_pair(
            InputOperationKind::Keyboard,
            InputDelivery::System,
            CapabilitySupport::Supported,
            SubmissionEvidence::SystemInputAdmission,
        );

        assert!(!sequence.may_submit_via(pointer_only, InputDelivery::System));
        assert!(sequence.may_submit_via(both, InputDelivery::System));
        assert!(
            !sequence.may_submit_via(both, InputDelivery::WindowMessage),
            "support is per mechanism"
        );
    }

    #[test]
    fn held_state_is_what_was_pressed_and_not_released() {
        let sequence = InputSequence::new(vec![
            InputEvent::KeyPress(Key::Modifier(Modifier::Control)),
            InputEvent::KeyPress(Key::Character('c')),
            InputEvent::KeyRelease(Key::Character('c')),
            InputEvent::PointerPress(PointerButton::Primary),
        ])
        .expect("valid");

        assert_eq!(sequence.held_after(0), Vec::new());
        assert_eq!(
            sequence.held_after(2),
            vec![
                PressedState::Key(Key::Character('c')),
                PressedState::Key(Key::Modifier(Modifier::Control)),
            ],
            "cleanup releases in reverse order, so the modifier goes last"
        );
        assert_eq!(
            sequence.held_after(3),
            vec![PressedState::Key(Key::Modifier(Modifier::Control))]
        );
        assert_eq!(
            sequence.held_after(4),
            vec![
                PressedState::Button(PointerButton::Primary),
                PressedState::Key(Key::Modifier(Modifier::Control)),
            ]
        );
    }

    #[test]
    fn a_partial_press_adds_a_conservative_cleanup_obligation() {
        let sequence = InputSequence::new(vec![
            InputEvent::KeyPress(Key::Modifier(Modifier::Control)),
            InputEvent::PointerPress(PointerButton::Primary),
        ])
        .expect("valid");

        assert_eq!(
            sequence.possibly_held_after(1, true),
            vec![
                PressedState::Button(PointerButton::Primary),
                PressedState::Key(Key::Modifier(Modifier::Control)),
            ]
        );
        assert_eq!(
            sequence.possibly_held_after(1, false),
            vec![PressedState::Key(Key::Modifier(Modifier::Control))]
        );
    }

    #[test]
    fn a_fully_released_sequence_leaves_nothing_for_cleanup() {
        assert_eq!(click().held_after(3), Vec::new());
    }

    #[test]
    fn a_request_defaults_to_changing_nothing_the_caller_did_not_ask_for() {
        let request = InputRequest::new(
            target(),
            click(),
            DeliveryPlan::require(InputDelivery::System),
        );

        assert_eq!(request.focus(), FocusPolicy::Preserve);
        assert_eq!(
            request.pointer_geometry().policy(),
            GeometryPolicy::ReprojectCurrent
        );
        assert_eq!(request.check(), Ok(()));
    }

    #[test]
    fn a_pointer_request_without_the_frame_its_policy_needs_is_refused() {
        let request = InputRequest::new(
            target(),
            click(),
            DeliveryPlan::require(InputDelivery::System),
        )
        .with_pointer_geometry(PointerGeometry::reprojected());

        assert_eq!(request.check(), Ok(()));

        let keyboard_only = InputRequest::new(
            target(),
            InputSequence::new(vec![InputEvent::KeyPress(Key::Enter)]).expect("valid"),
            DeliveryPlan::require(InputDelivery::System),
        );
        assert_eq!(
            keyboard_only.check(),
            Ok(()),
            "a sequence with no pointer event needs no pointer geometry"
        );
    }

    #[test]
    fn a_request_defaults_to_the_contracts_own_cleanup_bounds() {
        use crate::cleanup::CleanupBudget;

        let request = InputRequest::new(
            target(),
            click(),
            DeliveryPlan::require(InputDelivery::System),
        );

        assert_eq!(request.cleanup_budget(), CleanupBudget::contract());
        assert_eq!(
            request
                .with_cleanup_budget(CleanupBudget::at_most(1, Duration::from_millis(5)))
                .cleanup_budget()
                .max_events(),
            1
        );
    }

    #[test]
    fn a_request_keeps_the_policies_it_was_given() {
        let request = InputRequest::new(
            target(),
            click(),
            DeliveryPlan::ordered(vec![InputDelivery::WindowMessage, InputDelivery::System])
                .expect("valid"),
        )
        .with_focus(FocusPolicy::ActivateIfRequired);

        assert!(request.delivery().permits_fallback());
        assert_eq!(request.focus(), FocusPolicy::ActivateIfRequired);
        assert_eq!(request.sequence().len(), 3);
    }
}
