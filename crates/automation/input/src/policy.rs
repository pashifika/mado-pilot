//! What a caller decides, rather than what the platform decides.
//!
//! Delivery order, focus, and how a pointer position resolves are all caller
//! policy. None of them has a safe default the Adapter could pick: substituting
//! system input for background delivery activates a window the caller asked not to
//! touch, focusing a target steals the user's keyboard, and reprojecting a stale
//! coordinate clicks whatever moved into its place.

use std::fmt;

use mado_pilot_core::{FrameStamp, InputDelivery};

use crate::fault::InputFault;

/// Which delivery mechanisms a controller may use, in the caller's own order.
///
/// A plan with one mechanism permits no fallback: the controller either delivers
/// through it or reports why it could not. A plan with several is an explicit
/// permission to try the next one, and the receipt reports which was used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryPlan {
    modes: Vec<InputDelivery>,
}

impl DeliveryPlan {
    /// Requires exactly `mode`, permitting no substitute.
    #[must_use]
    pub fn require(mode: InputDelivery) -> Self {
        Self { modes: vec![mode] }
    }

    /// Permits `modes`, tried in the order given.
    ///
    /// # Errors
    ///
    /// Returns [`InputFault::InvalidDeliveryPlan`] for an empty list or a repeated
    /// mechanism. A repeat would make the receipt's attempt list ambiguous, and an
    /// empty plan is a request to deliver through nothing.
    pub fn ordered(modes: Vec<InputDelivery>) -> Result<Self, InputFault> {
        if modes.is_empty() {
            return Err(InputFault::InvalidDeliveryPlan);
        }
        for (index, mode) in modes.iter().enumerate() {
            if modes[..index].contains(mode) {
                return Err(InputFault::InvalidDeliveryPlan);
            }
        }
        Ok(Self { modes })
    }

    /// Returns the permitted mechanisms in the caller's order.
    #[must_use]
    pub fn modes(&self) -> &[InputDelivery] {
        &self.modes
    }

    /// Returns the mechanism the controller must try first.
    #[must_use]
    pub fn first(&self) -> InputDelivery {
        // `ordered` refuses an empty list and `require` builds one element, so the
        // plan always has a first mechanism.
        self.modes[0]
    }

    /// Reports whether the caller permitted any substitute for the first choice.
    #[must_use]
    pub fn permits_fallback(&self) -> bool {
        self.modes.len() > 1
    }
}

impl fmt::Display for DeliveryPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, mode) in self.modes.iter().enumerate() {
            if index > 0 {
                formatter.write_str(" then ")?;
            }
            write!(formatter, "{mode}")?;
        }
        Ok(())
    }
}

/// What a controller may do about focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FocusPolicy {
    /// Never change focus. An operation that needs it fails with
    /// [`InputFault::FocusRequired`].
    Preserve,
    /// Require the target to be focused already, and fail rather than focus it.
    RequireFocused,
    /// Focus the target when the selected mechanism needs it.
    ActivateIfRequired,
}

impl FocusPolicy {
    /// Reports whether the controller may activate the target.
    #[must_use]
    pub const fn may_activate(self) -> bool {
        matches!(self, FocusPolicy::ActivateIfRequired)
    }

    /// Returns a stable lowercase slug, for logs and the C ABI mapping.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            FocusPolicy::Preserve => "preserve",
            FocusPolicy::RequireFocused => "require_focused",
            FocusPolicy::ActivateIfRequired => "activate_if_required",
        }
    }
}

impl fmt::Display for FocusPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// How a pointer position taken from a frame resolves at delivery time.
///
/// A position derived from a captured frame describes where something was when
/// that frame was captured. By the time input is delivered the target may have
/// moved, resized, or changed scale, and the three policies are the three honest
/// answers to that: refuse, re-resolve, or trust the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GeometryPolicy {
    /// Refuse when the target moved or resized since the source frame.
    ///
    /// The safe choice for clicking a match: if the window moved, the coordinate no
    /// longer names what was matched.
    RequireUnchanged,
    /// Resolve against the target's authoritative current geometry.
    ReprojectCurrent,
    /// Resolve against the source frame's own retained transform.
    ///
    /// Only where the platform can deliver to those coordinates; a controller that
    /// cannot reports [`InputFault::UnsupportedCombination`].
    UseFrameSnapshot,
}

impl GeometryPolicy {
    /// Reports whether the policy needs the frame its coordinates came from.
    #[must_use]
    pub const fn needs_source_frame(self) -> bool {
        matches!(
            self,
            GeometryPolicy::RequireUnchanged | GeometryPolicy::UseFrameSnapshot
        )
    }

    /// Returns a stable lowercase slug, for logs and the C ABI mapping.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            GeometryPolicy::RequireUnchanged => "require_unchanged",
            GeometryPolicy::ReprojectCurrent => "reproject_current",
            GeometryPolicy::UseFrameSnapshot => "use_frame_snapshot",
        }
    }
}

impl fmt::Display for GeometryPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A geometry policy and the frame the coordinates it governs came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PointerGeometry {
    policy: GeometryPolicy,
    source: Option<FrameStamp>,
}

impl PointerGeometry {
    /// Resolves coordinates against the target's current geometry.
    #[must_use]
    pub const fn reprojected() -> Self {
        Self {
            policy: GeometryPolicy::ReprojectCurrent,
            source: None,
        }
    }

    /// Refuses delivery when the target changed since `source`.
    #[must_use]
    pub const fn require_unchanged_since(source: FrameStamp) -> Self {
        Self {
            policy: GeometryPolicy::RequireUnchanged,
            source: Some(source),
        }
    }

    /// Resolves coordinates against the retained transform of `source`.
    #[must_use]
    pub const fn from_frame_snapshot(source: FrameStamp) -> Self {
        Self {
            policy: GeometryPolicy::UseFrameSnapshot,
            source: Some(source),
        }
    }

    /// Returns the policy.
    #[must_use]
    pub const fn policy(self) -> GeometryPolicy {
        self.policy
    }

    /// Returns the source frame, when the policy has one.
    #[must_use]
    pub const fn source(self) -> Option<FrameStamp> {
        self.source
    }

    /// Checks that the policy has the source frame it needs.
    ///
    /// # Errors
    ///
    /// Returns [`InputFault::MissingCoordinateSource`] when a policy that resolves
    /// against a frame was given none. The constructors above cannot produce that
    /// state; a request assembled field by field over the C ABI can.
    pub fn check(self) -> Result<(), InputFault> {
        if self.policy.needs_source_frame() && self.source.is_none() {
            return Err(InputFault::MissingCoordinateSource);
        }
        Ok(())
    }
}

impl Default for PointerGeometry {
    /// Reprojecting is the default, because it is the only policy that needs no
    /// source frame and therefore the only one a caller with no frame can mean.
    fn default() -> Self {
        Self::reprojected()
    }
}

impl fmt::Display for PointerGeometry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.source {
            Some(source) => write!(formatter, "{} from {source}", self.policy),
            None => write!(formatter, "{}", self.policy),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DeliveryPlan, FocusPolicy, GeometryPolicy, PointerGeometry};
    use crate::fault::InputFault;
    use mado_pilot_core::{
        FrameSequence, FrameStamp, GeometryRevision, IdentityIssuer, InputDelivery, StreamCursor,
    };

    fn stamp() -> FrameStamp {
        let issuer = IdentityIssuer::new();
        let mut cursor = StreamCursor::new(issuer.issue_stream().expect("issued"));
        cursor.publish(GeometryRevision::FIRST).expect("published")
    }

    #[test]
    fn a_required_mechanism_permits_no_substitute() {
        let plan = DeliveryPlan::require(InputDelivery::BackgroundTarget);

        assert_eq!(plan.modes(), [InputDelivery::BackgroundTarget]);
        assert_eq!(plan.first(), InputDelivery::BackgroundTarget);
        assert!(
            !plan.permits_fallback(),
            "a caller that asked for background delivery did not ask for system input"
        );
    }

    #[test]
    fn an_ordered_plan_keeps_the_callers_order() {
        let plan =
            DeliveryPlan::ordered(vec![InputDelivery::BackgroundTarget, InputDelivery::System])
                .expect("valid");

        assert_eq!(plan.first(), InputDelivery::BackgroundTarget);
        assert!(plan.permits_fallback());
        assert_eq!(plan.to_string(), "background_target then system");
    }

    #[test]
    fn an_empty_or_repeating_plan_is_refused() {
        assert_eq!(
            DeliveryPlan::ordered(Vec::new()),
            Err(InputFault::InvalidDeliveryPlan)
        );
        assert_eq!(
            DeliveryPlan::ordered(vec![InputDelivery::System, InputDelivery::System]),
            Err(InputFault::InvalidDeliveryPlan)
        );
    }

    #[test]
    fn only_one_focus_policy_may_activate_the_target() {
        assert!(FocusPolicy::ActivateIfRequired.may_activate());
        assert!(!FocusPolicy::Preserve.may_activate());
        assert!(!FocusPolicy::RequireFocused.may_activate());
    }

    #[test]
    fn the_policies_that_resolve_against_a_frame_need_one() {
        assert!(GeometryPolicy::RequireUnchanged.needs_source_frame());
        assert!(GeometryPolicy::UseFrameSnapshot.needs_source_frame());
        assert!(!GeometryPolicy::ReprojectCurrent.needs_source_frame());
    }

    #[test]
    fn a_frame_bound_policy_without_its_frame_is_refused() {
        let source = stamp();

        assert_eq!(PointerGeometry::reprojected().check(), Ok(()));
        assert_eq!(
            PointerGeometry::require_unchanged_since(source).check(),
            Ok(())
        );
        assert_eq!(
            PointerGeometry {
                policy: GeometryPolicy::RequireUnchanged,
                source: None,
            }
            .check(),
            Err(InputFault::MissingCoordinateSource)
        );
    }

    #[test]
    fn a_pointer_geometry_reports_the_frame_it_resolves_against() {
        let source = stamp();
        let geometry = PointerGeometry::from_frame_snapshot(source);

        assert_eq!(geometry.policy(), GeometryPolicy::UseFrameSnapshot);
        assert_eq!(geometry.source(), Some(source));
        assert_eq!(
            source.sequence(),
            FrameSequence::FIRST,
            "the stamp travels whole, not as a sequence number"
        );
        assert_eq!(PointerGeometry::default(), PointerGeometry::reprojected());
    }
}
