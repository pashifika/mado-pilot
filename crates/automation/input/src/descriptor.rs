//! What a target accepts, and the one admission rule every Adapter shares.

use std::fmt;

use mado_pilot_core::{InputCapability, InputDelivery, TargetId};

use crate::event::InputEvent;
use crate::fault::InputFault;
use crate::policy::FocusPolicy;
use crate::request::{InputRequest, SequenceLimits};

/// What one target accepts, as the controller that drives it reports.
///
/// A descriptor is what admission is decided against, which is why it is a value a
/// caller can hold rather than a set of questions it has to ask one at a time: a
/// request that a descriptor refuses is refused before an event is delivered, and
/// the caller can tell why without having tried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDescriptor {
    target: TargetId,
    capability: InputCapability,
    limits: SequenceLimits,
}

impl InputDescriptor {
    /// Describes what `target` accepts.
    #[must_use]
    pub fn new(target: TargetId, capability: InputCapability) -> Self {
        Self {
            target,
            capability,
            limits: SequenceLimits::contract(),
        }
    }

    /// Declares a sequence ceiling below the contract's own.
    #[must_use]
    pub fn with_limits(mut self, limits: SequenceLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Returns the target this describes.
    #[must_use]
    pub const fn target(&self) -> TargetId {
        self.target
    }

    /// Returns which operation and delivery combinations the target accepts.
    #[must_use]
    pub const fn capability(&self) -> InputCapability {
        self.capability
    }

    /// Returns the sequence bounds this controller applies.
    #[must_use]
    pub const fn limits(&self) -> SequenceLimits {
        self.limits
    }

    /// Reports whether the target accepts any input at all.
    #[must_use]
    pub const fn is_available(&self) -> bool {
        self.capability.is_available()
    }

    /// Chooses the delivery mechanism `request` will be executed through.
    ///
    /// This is the admission rule, written once so that two Adapters cannot
    /// disagree about which requests are refusable. It consults only the
    /// descriptor: focus state, authorization, target existence, and the
    /// platform's own policy are runtime facts that a later step still checks and
    /// that this cannot pre-empt.
    ///
    /// The chosen mechanism is the first one in the caller's own order that
    /// supports every operation in the sequence. Later mechanisms are the
    /// caller's permitted fallbacks, and the receipt reports which was used.
    ///
    /// # Errors
    ///
    /// - [`InputFault::ForeignTarget`] when the request names another target.
    /// - [`InputFault::SequenceOutOfBounds`] when the sequence is longer than this
    ///   controller accepts.
    /// - [`InputFault::MissingCoordinateSource`] when the geometry policy needs a
    ///   source frame the request does not carry.
    /// - [`InputFault::UnsupportedCoordinate`] when a pointer position is expressed
    ///   in a space the target does not accept.
    /// - [`InputFault::UnsupportedCombination`] when no permitted mechanism supports
    ///   every operation in the sequence.
    /// - [`InputFault::FocusRequired`] when the chosen mechanism needs the target
    ///   focused and the focus policy will not allow it.
    pub fn admit(&self, request: &InputRequest) -> Result<InputDelivery, InputFault> {
        if request.target() != self.target {
            return Err(InputFault::ForeignTarget);
        }
        if request.sequence().len() > self.limits.max_events() {
            return Err(InputFault::SequenceOutOfBounds);
        }
        request.check()?;

        // Whether the target accepts these operations at all is decided before
        // where a pointer position may be expressed. A capture-only target refuses
        // both, and "no input" is the answer that tells a caller what to do.
        let selected = request
            .delivery()
            .modes()
            .iter()
            .copied()
            .find(|delivery| request.sequence().supported_by(self.capability, *delivery))
            .ok_or(InputFault::UnsupportedCombination)?;

        for event in request.sequence().events() {
            if let InputEvent::PointerMove(position) = event
                && !self.capability.accepts_pointer_space(position.space())
            {
                return Err(InputFault::UnsupportedCoordinate);
            }
        }

        // Whether a `RequireFocused` target is actually focused is a runtime
        // fact, so it is refused later by the controller rather than guessed at
        // here.
        if self.capability.requires_focus(selected) && request.focus() == FocusPolicy::Preserve {
            return Err(InputFault::FocusRequired);
        }

        Ok(selected)
    }
}

impl fmt::Display for InputDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} {} ({})",
            self.target,
            if self.is_available() {
                "input available"
            } else {
                "no input"
            },
            self.limits
        )
    }
}

#[cfg(test)]
mod tests {
    use super::InputDescriptor;
    use crate::event::{InputEvent, Key, PointerButton};
    use crate::fault::InputFault;
    use crate::policy::{DeliveryPlan, FocusPolicy, GeometryPolicy, PointerGeometry};
    use crate::request::{InputRequest, InputSequence, SequenceLimits};
    use mado_pilot_core::{
        CoordinateSpace, FrameStamp, GeometryRevision, IdentityIssuer, InputCapability,
        InputDelivery, InputOperationKind, Point, ProviderId, StreamCursor, TargetId,
    };

    fn target() -> TargetId {
        IdentityIssuer::new()
            .issue_target(ProviderId::new("fake"))
            .expect("issued")
    }

    fn stamp() -> FrameStamp {
        let issuer = IdentityIssuer::new();
        let mut cursor = StreamCursor::new(issuer.issue_stream().expect("issued"));
        cursor.publish(GeometryRevision::FIRST).expect("published")
    }

    fn point(space: CoordinateSpace) -> Point {
        Point::new(space, 2.0, 3.0).expect("valid")
    }

    fn pointer_capability() -> InputCapability {
        InputCapability::none()
            .with_pair(InputOperationKind::Pointer, InputDelivery::System)
            .with_pointer_space(CoordinateSpace::CapturePixels)
    }

    fn click() -> InputSequence {
        InputSequence::new(vec![
            InputEvent::PointerMove(point(CoordinateSpace::CapturePixels)),
            InputEvent::PointerPress(PointerButton::Primary),
            InputEvent::PointerRelease(PointerButton::Primary),
        ])
        .expect("valid")
    }

    #[test]
    fn admission_selects_the_first_permitted_mechanism_that_supports_the_sequence() {
        let target = target();
        let descriptor = InputDescriptor::new(
            target,
            pointer_capability()
                .with_pair(InputOperationKind::Pointer, InputDelivery::BackgroundTarget),
        );
        let request = InputRequest::new(
            target,
            click(),
            DeliveryPlan::ordered(vec![InputDelivery::BackgroundTarget, InputDelivery::System])
                .expect("valid"),
        );

        assert_eq!(
            descriptor.admit(&request),
            Ok(InputDelivery::BackgroundTarget),
            "the caller's own order decides"
        );
    }

    #[test]
    fn a_permitted_fallback_is_selected_when_the_first_choice_is_unsupported() {
        let target = target();
        let descriptor = InputDescriptor::new(target, pointer_capability());
        let request = InputRequest::new(
            target,
            click(),
            DeliveryPlan::ordered(vec![InputDelivery::BackgroundTarget, InputDelivery::System])
                .expect("valid"),
        );

        assert_eq!(descriptor.admit(&request), Ok(InputDelivery::System));
    }

    #[test]
    fn an_unsupported_pair_fails_admission_before_any_event() {
        let target = target();
        let descriptor = InputDescriptor::new(target, pointer_capability());
        let request = InputRequest::new(
            target,
            click(),
            DeliveryPlan::require(InputDelivery::BackgroundTarget),
        );

        assert_eq!(
            descriptor.admit(&request),
            Err(InputFault::UnsupportedCombination),
            "a required mechanism the target does not accept is refused, not substituted"
        );
    }

    #[test]
    fn a_sequence_needing_two_operations_needs_both_from_one_mechanism() {
        let target = target();
        let descriptor = InputDescriptor::new(target, pointer_capability());
        let request = InputRequest::new(
            target,
            InputSequence::new(vec![
                InputEvent::PointerPress(PointerButton::Primary),
                InputEvent::KeyPress(Key::Enter),
            ])
            .expect("valid"),
            DeliveryPlan::require(InputDelivery::System),
        );

        assert_eq!(
            descriptor.admit(&request),
            Err(InputFault::UnsupportedCombination)
        );
    }

    #[test]
    fn a_request_for_another_target_is_refused() {
        let descriptor = InputDescriptor::new(target(), pointer_capability());
        let request = InputRequest::new(
            target(),
            click(),
            DeliveryPlan::require(InputDelivery::System),
        );

        assert_eq!(descriptor.admit(&request), Err(InputFault::ForeignTarget));
    }

    #[test]
    fn a_pointer_space_the_target_does_not_accept_is_refused() {
        let target = target();
        let descriptor = InputDescriptor::new(target, pointer_capability());
        let request = InputRequest::new(
            target,
            InputSequence::new(vec![InputEvent::PointerMove(point(
                CoordinateSpace::DesktopLogical,
            ))])
            .expect("valid"),
            DeliveryPlan::require(InputDelivery::System),
        );

        assert_eq!(
            descriptor.admit(&request),
            Err(InputFault::UnsupportedCoordinate)
        );
    }

    #[test]
    fn a_sequence_longer_than_the_controller_accepts_is_refused() {
        let target = target();
        let descriptor = InputDescriptor::new(target, pointer_capability())
            .with_limits(SequenceLimits::at_most(2));
        let request = InputRequest::new(
            target,
            click(),
            DeliveryPlan::require(InputDelivery::System),
        );

        assert_eq!(
            descriptor.admit(&request),
            Err(InputFault::SequenceOutOfBounds)
        );
        assert_eq!(descriptor.limits().max_events(), 2);
    }

    #[test]
    fn a_geometry_policy_without_its_source_frame_is_refused() {
        let target = target();
        let descriptor = InputDescriptor::new(target, pointer_capability());
        let unchanged = InputRequest::new(
            target,
            click(),
            DeliveryPlan::require(InputDelivery::System),
        )
        .with_pointer_geometry(PointerGeometry::require_unchanged_since(stamp()));

        assert_eq!(descriptor.admit(&unchanged), Ok(InputDelivery::System));
        assert_eq!(
            unchanged.pointer_geometry().policy(),
            GeometryPolicy::RequireUnchanged
        );
    }

    #[test]
    fn a_mechanism_needing_focus_is_refused_when_focus_is_preserved() {
        let target = target();
        let descriptor = InputDescriptor::new(
            target,
            pointer_capability().with_focus_required(InputDelivery::System),
        );
        let preserving = InputRequest::new(
            target,
            click(),
            DeliveryPlan::require(InputDelivery::System),
        );

        assert_eq!(
            descriptor.admit(&preserving),
            Err(InputFault::FocusRequired)
        );
        assert_eq!(
            descriptor.admit(
                &preserving
                    .clone()
                    .with_focus(FocusPolicy::ActivateIfRequired)
            ),
            Ok(InputDelivery::System)
        );
        assert_eq!(
            descriptor.admit(&preserving.with_focus(FocusPolicy::RequireFocused)),
            Ok(InputDelivery::System),
            "whether the target is focused is a runtime fact, refused later"
        );
    }

    #[test]
    fn a_capture_only_target_admits_nothing() {
        let target = target();
        let descriptor = InputDescriptor::new(target, InputCapability::none());
        let request = InputRequest::new(
            target,
            click(),
            DeliveryPlan::require(InputDelivery::System),
        );

        assert!(!descriptor.is_available());
        assert_eq!(
            descriptor.admit(&request),
            Err(InputFault::UnsupportedCombination)
        );
    }
}
