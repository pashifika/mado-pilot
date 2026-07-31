//! What a discovered target is, and which operations its provider can perform.
//!
//! A capability is a description, not a promise. It says what the provider is
//! able to attempt, so a caller is refused at admission rather than after an
//! operation has half happened. Focus, authorization, target loss, integrity
//! rules, and operating-system policy can still refuse an operation the
//! capability lists.
//!
//! Nothing here names a platform. A window handle, a display identifier, a
//! virtual-key code, and an event-tap reference are all Adapter implementation
//! details, and none of them appears in a capability.

use std::fmt;

use crate::geometry::CoordinateSpace;
use crate::permission::PermissionKind;

/// What kind of desktop object a discovered target is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum TargetKind {
    /// One application window.
    Window,
    /// One display, including whatever windows happen to be on it.
    Display,
}

impl TargetKind {
    /// Returns a stable lowercase slug, for logs and the C ABI mapping.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            TargetKind::Window => "window",
            TargetKind::Display => "display",
        }
    }
}

impl fmt::Display for TargetKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Whether a provider can attempt one operation on one target.
///
/// `Unknown` is a real answer rather than a gap in the Adapter: some operations
/// cannot be established without performing them, and claiming either support or
/// refusal in that case would be an invention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CapabilitySupport {
    /// The provider can attempt the operation on this target.
    Supported,
    /// The provider cannot perform it on this target at all.
    Unsupported,
    /// The provider cannot establish it without attempting the operation.
    Unknown,
}

impl CapabilitySupport {
    /// Reports whether the operation may be attempted.
    ///
    /// `Unknown` is attemptable: refusing to try would turn an unestablished
    /// answer into a refusal the platform did not make.
    #[must_use]
    pub const fn may_attempt(self) -> bool {
        matches!(
            self,
            CapabilitySupport::Supported | CapabilitySupport::Unknown
        )
    }

    /// Returns a stable lowercase slug, for logs and the C ABI mapping.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            CapabilitySupport::Supported => "supported",
            CapabilitySupport::Unsupported => "unsupported",
            CapabilitySupport::Unknown => "unknown",
        }
    }
}

impl fmt::Display for CapabilitySupport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// What an input operation does, independently of how it is delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum InputOperationKind {
    /// Pointer movement, buttons, and wheel.
    Pointer,
    /// Individual key presses and releases, including modifiers.
    Keyboard,
    /// A run of characters, entered as text rather than as key codes.
    Text,
}

impl InputOperationKind {
    /// Every operation kind version one knows about.
    pub const ALL: [Self; 3] = [
        InputOperationKind::Pointer,
        InputOperationKind::Keyboard,
        InputOperationKind::Text,
    ];

    /// Returns a stable lowercase slug, for logs and the C ABI mapping.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            InputOperationKind::Pointer => "pointer",
            InputOperationKind::Keyboard => "keyboard",
            InputOperationKind::Text => "text",
        }
    }

    const fn index(self) -> u32 {
        match self {
            InputOperationKind::Pointer => 0,
            InputOperationKind::Keyboard => 1,
            InputOperationKind::Text => 2,
        }
    }
}

impl fmt::Display for InputOperationKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// How an input event reaches its target.
///
/// This is a separate axis from what the operation does, because the two do not
/// vary together: a target may accept background keystrokes and no background
/// pointer input, and one that accepts neither may still accept both through the
/// system input path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum InputDelivery {
    /// The operating system's own input path, which affects whatever is focused
    /// and is subject to focus and integrity rules.
    System,
    /// Delivery addressed to the target itself, without activating it.
    BackgroundTarget,
}

impl InputDelivery {
    /// Every delivery mechanism version one knows about.
    pub const ALL: [Self; 2] = [InputDelivery::System, InputDelivery::BackgroundTarget];

    /// Returns a stable lowercase slug, for logs and the C ABI mapping.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            InputDelivery::System => "system",
            InputDelivery::BackgroundTarget => "background_target",
        }
    }

    const fn index(self) -> u32 {
        match self {
            InputDelivery::System => 0,
            InputDelivery::BackgroundTarget => 1,
        }
    }
}

impl fmt::Display for InputDelivery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Which operation-and-delivery combinations a target accepts.
///
/// The supported set is a set of pairs rather than one list of operations and
/// one list of mechanisms. The cross product would claim combinations no Adapter
/// verified: advertising `Keyboard` and `BackgroundTarget` separately says
/// background keystrokes work, and that is a different claim from being able to
/// deliver keystrokes at all.
///
/// A default capability supports nothing, which is what a capture-only target
/// reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct InputCapability {
    /// One bit per (operation, delivery) pair.
    pairs: u8,
    /// One bit per delivery mechanism that needs the target focused.
    focus: u8,
    /// One bit per coordinate space a pointer position may be expressed in.
    spaces: u8,
    permission: Option<PermissionKind>,
}

impl InputCapability {
    /// A target that accepts no input at all.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            pairs: 0,
            focus: 0,
            spaces: 0,
            permission: None,
        }
    }

    /// Adds one verified operation-and-delivery combination.
    #[must_use]
    pub const fn with_pair(mut self, kind: InputOperationKind, delivery: InputDelivery) -> Self {
        self.pairs |= pair_bit(kind, delivery);
        self
    }

    /// Records that `delivery` reaches the target only while it is focused.
    #[must_use]
    pub const fn with_focus_required(mut self, delivery: InputDelivery) -> Self {
        self.focus |= delivery_bit(delivery);
        self
    }

    /// Adds a coordinate space a pointer position may be given in.
    #[must_use]
    pub const fn with_pointer_space(mut self, space: CoordinateSpace) -> Self {
        self.spaces |= space_bit(space);
        self
    }

    /// Records the authorization input delivery ordinarily requires.
    #[must_use]
    pub const fn with_permission(mut self, permission: PermissionKind) -> Self {
        self.permission = Some(permission);
        self
    }

    /// Reports whether `kind` can be delivered through `delivery`.
    #[must_use]
    pub const fn supports(self, kind: InputOperationKind, delivery: InputDelivery) -> bool {
        self.pairs & pair_bit(kind, delivery) != 0
    }

    /// Reports whether any combination is supported.
    #[must_use]
    pub const fn is_available(self) -> bool {
        self.pairs != 0
    }

    /// Reports whether `delivery` requires the target to be focused.
    #[must_use]
    pub const fn requires_focus(self, delivery: InputDelivery) -> bool {
        self.focus & delivery_bit(delivery) != 0
    }

    /// Reports whether a pointer position may be expressed in `space`.
    ///
    /// A coordinate space this build does not know about is not accepted, for
    /// the reason an unknown space is not convertible: a newer caller must not
    /// believe an older library will resolve a position it cannot.
    #[must_use]
    pub const fn accepts_pointer_space(self, space: CoordinateSpace) -> bool {
        self.spaces & space_bit(space) != 0
    }

    /// Returns the authorization input delivery ordinarily requires, if any.
    #[must_use]
    pub const fn permission(self) -> Option<PermissionKind> {
        self.permission
    }
}

const fn pair_bit(kind: InputOperationKind, delivery: InputDelivery) -> u8 {
    // Three operations and two mechanisms occupy six of the eight bits. Every
    // variant is matched by `index`, so a kind or mechanism added later is a
    // compile error in this package rather than a pair that silently reports
    // unsupported.
    1u8 << (kind.index() * 2 + delivery.index())
}

const fn delivery_bit(delivery: InputDelivery) -> u8 {
    1u8 << delivery.index()
}

const fn space_bit(space: CoordinateSpace) -> u8 {
    match space {
        CoordinateSpace::CapturePixels => 1,
        CoordinateSpace::FrameNormalized => 1 << 1,
        CoordinateSpace::TargetNormalized => 1 << 2,
        CoordinateSpace::TargetLogical => 1 << 3,
        CoordinateSpace::DesktopLogical => 1 << 4,
    }
}

/// What a provider can do with one discovered target.
///
/// Capture and input are described separately because they fail separately. A
/// window whose capture is supported and whose input is unavailable is an
/// ordinary target, and version one uses it for exactly what it supports.
/// The target kind is optional because not every provider classifies its
/// targets. A deterministic replay source serves named frame sequences and knows
/// of no window or display behind them, and answering `Window` on its behalf
/// would be an invention a caller could filter on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TargetCapability {
    kind: Option<TargetKind>,
    capture: CapabilitySupport,
    capture_permission: Option<PermissionKind>,
    input: InputCapability,
}

impl TargetCapability {
    /// Describes a capturable target whose kind its provider does not classify.
    ///
    /// This is what a provider that serves prepared frames reports: capture is
    /// what it does, input is not, and there is no desktop object to classify.
    #[must_use]
    pub const fn unclassified() -> Self {
        Self {
            kind: None,
            capture: CapabilitySupport::Supported,
            capture_permission: None,
            input: InputCapability::none(),
        }
    }

    /// Describes a target that can be captured and accepts no input.
    #[must_use]
    pub const fn capture_only(kind: TargetKind) -> Self {
        Self {
            kind: Some(kind),
            capture: CapabilitySupport::Supported,
            capture_permission: None,
            input: InputCapability::none(),
        }
    }

    /// Describes a target with explicit capture and input capabilities.
    #[must_use]
    pub const fn new(kind: TargetKind, capture: CapabilitySupport, input: InputCapability) -> Self {
        Self {
            kind: Some(kind),
            capture,
            capture_permission: None,
            input,
        }
    }

    /// Records the authorization capture ordinarily requires.
    #[must_use]
    pub const fn with_capture_permission(mut self, permission: PermissionKind) -> Self {
        self.capture_permission = Some(permission);
        self
    }

    /// Returns what kind of desktop object the target is, when classified.
    #[must_use]
    pub const fn kind(self) -> Option<TargetKind> {
        self.kind
    }

    /// Returns whether capture may be attempted on the target.
    #[must_use]
    pub const fn capture(self) -> CapabilitySupport {
        self.capture
    }

    /// Returns the authorization capture ordinarily requires, if any.
    ///
    /// `None` means this platform grants no separate capture authorization, not
    /// that capture is authorized.
    #[must_use]
    pub const fn capture_permission(self) -> Option<PermissionKind> {
        self.capture_permission
    }

    /// Returns what input the target accepts.
    #[must_use]
    pub const fn input(self) -> InputCapability {
        self.input
    }
}

impl fmt::Display for TargetCapability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            Some(kind) => write!(formatter, "{kind}")?,
            None => formatter.write_str("unclassified")?,
        }
        write!(
            formatter,
            " capture={} input={}",
            self.capture,
            if self.input.is_available() {
                "available"
            } else {
                "none"
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CapabilitySupport, InputCapability, InputDelivery, InputOperationKind, TargetCapability,
        TargetKind,
    };
    use crate::geometry::CoordinateSpace;
    use crate::permission::PermissionKind;

    #[test]
    fn a_capture_only_target_accepts_no_input() {
        let capability = TargetCapability::capture_only(TargetKind::Display);

        assert_eq!(capability.kind(), Some(TargetKind::Display));
        assert_eq!(capability.capture(), CapabilitySupport::Supported);
        assert!(!capability.input().is_available());
        for kind in InputOperationKind::ALL {
            for delivery in InputDelivery::ALL {
                assert!(!capability.input().supports(kind, delivery));
            }
        }
    }

    #[test]
    fn a_supported_pair_does_not_imply_the_cross_product() {
        let input = InputCapability::none()
            .with_pair(
                InputOperationKind::Keyboard,
                InputDelivery::BackgroundTarget,
            )
            .with_pair(InputOperationKind::Pointer, InputDelivery::System);

        assert!(input.supports(
            InputOperationKind::Keyboard,
            InputDelivery::BackgroundTarget
        ));
        assert!(input.supports(InputOperationKind::Pointer, InputDelivery::System));
        assert!(
            !input.supports(InputOperationKind::Pointer, InputDelivery::BackgroundTarget),
            "background pointer input was never advertised"
        );
        assert!(
            !input.supports(InputOperationKind::Keyboard, InputDelivery::System),
            "system keyboard input was never advertised"
        );
        assert!(!input.supports(InputOperationKind::Text, InputDelivery::System));
    }

    #[test]
    fn every_pair_occupies_its_own_bit() {
        let mut all = InputCapability::none();
        for kind in InputOperationKind::ALL {
            for delivery in InputDelivery::ALL {
                all = all.with_pair(kind, delivery);
            }
        }

        for kind in InputOperationKind::ALL {
            for delivery in InputDelivery::ALL {
                assert!(all.supports(kind, delivery), "{kind} over {delivery}");
                let single = InputCapability::none().with_pair(kind, delivery);
                let others = InputOperationKind::ALL.into_iter().flat_map(|other_kind| {
                    InputDelivery::ALL
                        .into_iter()
                        .map(move |other_delivery| (other_kind, other_delivery))
                });
                for (other_kind, other_delivery) in others {
                    assert_eq!(
                        single.supports(other_kind, other_delivery),
                        other_kind == kind && other_delivery == delivery,
                        "{kind}/{delivery} leaked into {other_kind}/{other_delivery}"
                    );
                }
            }
        }
    }

    #[test]
    fn focus_is_recorded_per_delivery_mechanism() {
        let input = InputCapability::none()
            .with_pair(InputOperationKind::Text, InputDelivery::System)
            .with_pair(InputOperationKind::Text, InputDelivery::BackgroundTarget)
            .with_focus_required(InputDelivery::System);

        assert!(input.requires_focus(InputDelivery::System));
        assert!(
            !input.requires_focus(InputDelivery::BackgroundTarget),
            "non-activating delivery is the one that does not need focus"
        );
    }

    #[test]
    fn only_declared_pointer_spaces_are_accepted() {
        let input = InputCapability::none()
            .with_pair(InputOperationKind::Pointer, InputDelivery::System)
            .with_pointer_space(CoordinateSpace::CapturePixels)
            .with_pointer_space(CoordinateSpace::DesktopLogical);

        assert!(input.accepts_pointer_space(CoordinateSpace::CapturePixels));
        assert!(input.accepts_pointer_space(CoordinateSpace::DesktopLogical));
        assert!(!input.accepts_pointer_space(CoordinateSpace::FrameNormalized));
        assert!(!input.accepts_pointer_space(CoordinateSpace::TargetNormalized));
        assert!(!input.accepts_pointer_space(CoordinateSpace::TargetLogical));
    }

    #[test]
    fn an_unknown_capability_may_still_be_attempted() {
        assert!(CapabilitySupport::Supported.may_attempt());
        assert!(CapabilitySupport::Unknown.may_attempt());
        assert!(!CapabilitySupport::Unsupported.may_attempt());
    }

    #[test]
    fn a_capability_records_the_authorizations_each_operation_needs() {
        let capability = TargetCapability::new(
            TargetKind::Window,
            CapabilitySupport::Supported,
            InputCapability::none()
                .with_pair(InputOperationKind::Pointer, InputDelivery::System)
                .with_permission(PermissionKind::InputControl),
        )
        .with_capture_permission(PermissionKind::ScreenCapture);

        assert_eq!(
            capability.capture_permission(),
            Some(PermissionKind::ScreenCapture)
        );
        assert_eq!(
            capability.input().permission(),
            Some(PermissionKind::InputControl)
        );
        let text = capability.to_string();
        assert!(text.contains("window"), "{text}");
        assert!(text.contains("input=available"), "{text}");
    }

    #[test]
    fn an_unclassified_target_is_capturable_and_names_no_desktop_object() {
        let capability = TargetCapability::unclassified();

        assert_eq!(capability.kind(), None);
        assert_eq!(capability.capture(), CapabilitySupport::Supported);
        assert_eq!(capability.capture_permission(), None);
        assert!(!capability.input().is_available());
        assert!(capability.to_string().contains("unclassified"));
    }
}
