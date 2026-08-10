//! What a target accepts, and the one admission rule every Adapter shares.

use std::fmt;

use mado_pilot_core::{InputCapability, InputDelivery, SubmissionEvidence, TargetId};

use crate::event::InputEvent;
use crate::fault::InputFault;
use crate::policy::FocusPolicy;
use crate::request::{InputRequest, SequenceLimits};

/// What one target accepts, as the controller that drives it reports.
///
/// A descriptor is what admission is decided against, which is why it is a value a
/// caller can hold rather than a set of questions it has to ask one at a time: a
/// request that a descriptor refuses never reaches a native route, and the caller
/// can tell why without having tried.
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

    /// Validates route-independent request invariants.
    ///
    /// Route-specific capability, focus, coordinate, and evidence decisions are
    /// made by [`InputDescriptor::preflight_route`] so every visited route can be
    /// represented by an immutable attempt record.
    ///
    /// # Errors
    ///
    /// Returns [`InputFault::ForeignTarget`],
    /// [`InputFault::SequenceOutOfBounds`], or another request-structure fault
    /// before any route or native operation is attempted.
    pub fn validate(&self, request: &InputRequest) -> Result<(), InputFault> {
        if request.target() != self.target {
            return Err(InputFault::ForeignTarget);
        }
        if request.sequence().len() > self.limits.max_events() {
            return Err(InputFault::SequenceOutOfBounds);
        }
        request.check()
    }

    /// Evaluates one caller-selected route without native effect.
    ///
    /// `Unknown` capability support remains attemptable. The returned evidence is
    /// the common submission threshold for every operation in the sequence.
    ///
    /// # Errors
    ///
    /// Returns a route-local refusal when a pair is unsupported, pointer
    /// coordinates are not accepted, focus policy withholds required focus, or
    /// the descriptor has inconsistent evidence for operations sharing a route.
    pub fn preflight_route(
        &self,
        request: &InputRequest,
        route: InputDelivery,
    ) -> Result<SubmissionEvidence, InputFault> {
        self.validate(request)?;
        let operations = request.sequence().operation_kinds();
        let mut evidence = None;
        for operation in operations.iter().copied() {
            let pair = self.capability.pair(operation, route);
            if !pair.may_attempt() {
                return Err(InputFault::UnsupportedCombination);
            }
            let pair_evidence = pair.evidence().ok_or(InputFault::UnsupportedCombination)?;
            if evidence.is_some_and(|current| current != pair_evidence) {
                return Err(InputFault::UnsupportedCombination);
            }
            evidence = Some(pair_evidence);
            if pair.focus_required() && request.focus() == FocusPolicy::Preserve {
                return Err(InputFault::FocusRequired);
            }
        }

        if operations.contains(&mado_pilot_core::InputOperationKind::Pointer) {
            let pointer = self
                .capability
                .pair(mado_pilot_core::InputOperationKind::Pointer, route);
            for event in request.sequence().events() {
                if let InputEvent::PointerMove(position) = event
                    && !pointer.accepts_pointer_space(position.space())
                {
                    return Err(InputFault::UnsupportedCoordinate);
                }
            }
        }

        // A delay-only sequence performs no input operation, but still needs an
        // explicit route. Use that route's first attemptable pair as its threshold
        // rather than inventing evidence.
        if evidence.is_none() {
            evidence = mado_pilot_core::InputOperationKind::ALL
                .into_iter()
                .find_map(|operation| {
                    let pair = self.capability.pair(operation, route);
                    pair.may_attempt().then(|| pair.evidence()).flatten()
                });
        }

        evidence.ok_or(InputFault::UnsupportedCombination)
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
        CapabilitySupport, CoordinateSpace, FrameStamp, GeometryRevision, IdentityIssuer,
        InputCapability, InputDelivery, InputOperationKind, Point, ProviderId, StreamCursor,
        SubmissionEvidence, TargetId,
    };
    use std::time::Duration;

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
            .with_pair(
                InputOperationKind::Pointer,
                InputDelivery::System,
                CapabilitySupport::Supported,
                SubmissionEvidence::SystemInputAdmission,
            )
            .with_pointer_space(InputDelivery::System, CoordinateSpace::CapturePixels)
    }

    fn click() -> InputSequence {
        InputSequence::new(vec![
            InputEvent::PointerMove(point(CoordinateSpace::CapturePixels)),
            InputEvent::PointerPress(PointerButton::Primary),
            InputEvent::PointerRelease(PointerButton::Primary),
        ])
        .expect("valid")
    }

    fn request(target: TargetId, sequence: InputSequence, route: InputDelivery) -> InputRequest {
        InputRequest::new(target, sequence, DeliveryPlan::require(route))
    }

    #[test]
    fn delay_only_uses_selected_routes_first_attemptable_pair_evidence() {
        let target = target();
        let descriptor = InputDescriptor::new(
            target,
            InputCapability::none().with_pair(
                InputOperationKind::Keyboard,
                InputDelivery::WindowMessage,
                CapabilitySupport::Supported,
                SubmissionEvidence::TargetProtocolAcknowledgement,
            ),
        );
        let delay =
            InputSequence::new(vec![InputEvent::Delay(Duration::from_millis(1))]).expect("valid");

        assert_eq!(
            descriptor.preflight_route(
                &request(target, delay, InputDelivery::WindowMessage),
                InputDelivery::WindowMessage,
            ),
            Ok(SubmissionEvidence::TargetProtocolAcknowledgement)
        );
    }

    #[test]
    fn delay_only_refuses_selected_route_without_an_attemptable_pair() {
        let target = target();
        let descriptor = InputDescriptor::new(target, pointer_capability());
        let delay =
            InputSequence::new(vec![InputEvent::Delay(Duration::from_millis(1))]).expect("valid");

        assert_eq!(
            descriptor.preflight_route(
                &request(target, delay, InputDelivery::WindowMessage),
                InputDelivery::WindowMessage,
            ),
            Err(InputFault::UnsupportedCombination)
        );
    }

    #[test]
    fn unknown_window_route_is_attemptable_with_declared_evidence() {
        let target = target();
        let descriptor = InputDescriptor::new(
            target,
            pointer_capability()
                .with_pair(
                    InputOperationKind::Pointer,
                    InputDelivery::WindowMessage,
                    CapabilitySupport::Unknown,
                    SubmissionEvidence::TargetQueueAdmission,
                )
                .with_pointer_space(InputDelivery::WindowMessage, CoordinateSpace::CapturePixels),
        );

        assert_eq!(
            descriptor.preflight_route(
                &request(target, click(), InputDelivery::WindowMessage),
                InputDelivery::WindowMessage,
            ),
            Ok(SubmissionEvidence::TargetQueueAdmission)
        );
    }

    #[test]
    fn supported_fixture_route_preserves_protocol_acknowledgement() {
        let target = target();
        let descriptor = InputDescriptor::new(
            target,
            pointer_capability()
                .with_pair(
                    InputOperationKind::Pointer,
                    InputDelivery::WindowMessage,
                    CapabilitySupport::Supported,
                    SubmissionEvidence::TargetProtocolAcknowledgement,
                )
                .with_pointer_space(InputDelivery::WindowMessage, CoordinateSpace::CapturePixels),
        );

        assert_eq!(
            descriptor.preflight_route(
                &request(target, click(), InputDelivery::WindowMessage),
                InputDelivery::WindowMessage,
            ),
            Ok(SubmissionEvidence::TargetProtocolAcknowledgement)
        );
    }

    #[test]
    fn unsupported_pair_is_a_route_local_preflight_refusal() {
        let target = target();
        let descriptor = InputDescriptor::new(target, pointer_capability());

        assert_eq!(
            descriptor.preflight_route(
                &request(target, click(), InputDelivery::WindowMessage),
                InputDelivery::WindowMessage,
            ),
            Err(InputFault::UnsupportedCombination)
        );
    }

    #[test]
    fn every_operation_needs_one_route_with_consistent_evidence() {
        let target = target();
        let descriptor = InputDescriptor::new(target, pointer_capability());
        let mixed = InputSequence::new(vec![
            InputEvent::PointerPress(PointerButton::Primary),
            InputEvent::KeyPress(Key::Enter),
        ])
        .expect("valid");

        assert_eq!(
            descriptor.preflight_route(
                &request(target, mixed, InputDelivery::System),
                InputDelivery::System,
            ),
            Err(InputFault::UnsupportedCombination)
        );
    }

    #[test]
    fn pointer_coordinates_are_checked_per_route() {
        let target = target();
        let descriptor = InputDescriptor::new(target, pointer_capability());
        let desktop = InputSequence::new(vec![InputEvent::PointerMove(point(
            CoordinateSpace::DesktopLogical,
        ))])
        .expect("valid");

        assert_eq!(
            descriptor.preflight_route(
                &request(target, desktop, InputDelivery::System),
                InputDelivery::System,
            ),
            Err(InputFault::UnsupportedCoordinate)
        );
    }

    #[test]
    fn focus_policy_is_checked_for_the_selected_pair() {
        let target = target();
        let capability = pointer_capability()
            .with_focus_required(InputOperationKind::Pointer, InputDelivery::System);
        let descriptor = InputDescriptor::new(target, capability);
        let preserving = request(target, click(), InputDelivery::System);

        assert_eq!(
            descriptor.preflight_route(&preserving, InputDelivery::System),
            Err(InputFault::FocusRequired)
        );
        assert_eq!(
            descriptor.preflight_route(
                &preserving
                    .clone()
                    .with_focus(FocusPolicy::ActivateIfRequired),
                InputDelivery::System,
            ),
            Ok(SubmissionEvidence::SystemInputAdmission)
        );
        assert_eq!(
            descriptor.preflight_route(
                &preserving.with_focus(FocusPolicy::RequireFocused),
                InputDelivery::System,
            ),
            Ok(SubmissionEvidence::SystemInputAdmission)
        );
    }

    #[test]
    fn route_independent_validation_precedes_preflight() {
        let selected = target();
        let descriptor = InputDescriptor::new(selected, pointer_capability())
            .with_limits(SequenceLimits::at_most(2));
        let foreign = request(target(), click(), InputDelivery::System);
        let too_long = request(selected, click(), InputDelivery::System);
        let unchanged = request(selected, click(), InputDelivery::System)
            .with_pointer_geometry(PointerGeometry::require_unchanged_since(stamp()));

        assert_eq!(
            descriptor.validate(&foreign),
            Err(InputFault::ForeignTarget)
        );
        assert_eq!(
            descriptor.validate(&too_long),
            Err(InputFault::SequenceOutOfBounds)
        );
        assert_eq!(
            unchanged.pointer_geometry().policy(),
            GeometryPolicy::RequireUnchanged
        );
    }

    #[test]
    fn capture_only_target_has_no_attemptable_route() {
        let target = target();
        let descriptor = InputDescriptor::new(target, InputCapability::none());

        assert!(!descriptor.is_available());
        assert_eq!(
            descriptor.preflight_route(
                &request(target, click(), InputDelivery::System),
                InputDelivery::System,
            ),
            Err(InputFault::UnsupportedCombination)
        );
    }
}
