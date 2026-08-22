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

/// What an input operation does, independently of how it is submitted.
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

/// How an input event is submitted.
///
/// The route is separate from the operation kind and from the evidence a native
/// API returns. A caller must select a route explicitly; none is a promise that
/// the target application consumed the event or changed state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum InputDelivery {
    /// The operating system input path, affecting the focused system target.
    System,
    /// A message addressed to one exact window.
    WindowMessage,
    /// An event addressed to the process that owns the selected target.
    ProcessDirected,
}

impl InputDelivery {
    /// Every delivery route version one knows about.
    pub const ALL: [Self; 3] = [
        InputDelivery::System,
        InputDelivery::WindowMessage,
        InputDelivery::ProcessDirected,
    ];

    /// Returns the address scope inherent to this route.
    #[must_use]
    pub const fn address_scope(self) -> InputAddressScope {
        match self {
            InputDelivery::System => InputAddressScope::FocusedSystem,
            InputDelivery::WindowMessage => InputAddressScope::ExactWindow,
            InputDelivery::ProcessDirected => InputAddressScope::OwningProcess,
        }
    }

    /// Returns a stable lowercase slug, for diagnostics and the C ABI mapping.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            InputDelivery::System => "system",
            InputDelivery::WindowMessage => "window_message",
            InputDelivery::ProcessDirected => "process_directed",
        }
    }

    const fn index(self) -> usize {
        match self {
            InputDelivery::System => 0,
            InputDelivery::WindowMessage => 1,
            InputDelivery::ProcessDirected => 2,
        }
    }
}

impl fmt::Display for InputDelivery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// What native object or subsystem a route addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum InputAddressScope {
    /// The operating system input stream and whatever target is focused.
    FocusedSystem,
    /// One exact selected window.
    ExactWindow,
    /// The process that owns the selected target.
    OwningProcess,
}

impl InputAddressScope {
    /// Returns a stable lowercase slug, for diagnostics and the C ABI mapping.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            InputAddressScope::FocusedSystem => "focused_system",
            InputAddressScope::ExactWindow => "exact_window",
            InputAddressScope::OwningProcess => "owning_process",
        }
    }
}

impl fmt::Display for InputAddressScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The strongest native submission fact a route can report.
///
/// These variants are independent facts rather than an ordered confidence score.
/// None implies application consumption or visual effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum SubmissionEvidence {
    /// A posting API was invoked and returned without a submission result.
    InvocationOnly,
    /// The system input mechanism reported complete insertion.
    SystemInputAdmission,
    /// The selected target queue accepted the native representation.
    TargetQueueAdmission,
    /// A documented target-specific protocol acknowledged the logical event.
    TargetProtocolAcknowledgement,
}

impl SubmissionEvidence {
    /// Returns a stable lowercase slug, for diagnostics and the C ABI mapping.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            SubmissionEvidence::InvocationOnly => "invocation_only",
            SubmissionEvidence::SystemInputAdmission => "system_input_admission",
            SubmissionEvidence::TargetQueueAdmission => "target_queue_admission",
            SubmissionEvidence::TargetProtocolAcknowledgement => "target_protocol_acknowledgement",
        }
    }
}

impl fmt::Display for SubmissionEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The complete capability metadata for one operation-and-route pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InputRouteCapability {
    operation: InputOperationKind,
    route: InputDelivery,
    support: CapabilitySupport,
    focus_required: bool,
    pointer_spaces: u8,
    permission: Option<PermissionKind>,
    evidence: Option<SubmissionEvidence>,
}

impl InputRouteCapability {
    /// Returns the operation this row describes.
    #[must_use]
    pub const fn operation(self) -> InputOperationKind {
        self.operation
    }

    /// Returns the route this row describes.
    #[must_use]
    pub const fn route(self) -> InputDelivery {
        self.route
    }

    /// Returns the route's address scope.
    #[must_use]
    pub const fn address_scope(self) -> InputAddressScope {
        self.route.address_scope()
    }

    /// Returns whether the pair is supported, unsupported, or attemptable with
    /// unknown application compatibility.
    #[must_use]
    pub const fn support(self) -> CapabilitySupport {
        self.support
    }

    /// Reports whether this pair may be attempted.
    #[must_use]
    pub const fn may_attempt(self) -> bool {
        self.support.may_attempt()
    }

    /// Reports whether this pair requires the selected target to be focused.
    #[must_use]
    pub const fn focus_required(self) -> bool {
        self.focus_required
    }

    /// Reports whether a pointer position may use `space` for this route.
    #[must_use]
    pub const fn accepts_pointer_space(self, space: CoordinateSpace) -> bool {
        self.pointer_spaces & space_bit(space) != 0
    }

    /// Returns the authorization this pair ordinarily requires.
    #[must_use]
    pub const fn permission(self) -> Option<PermissionKind> {
        self.permission
    }

    /// Returns the strongest submission evidence this pair can report.
    #[must_use]
    pub const fn evidence(self) -> Option<SubmissionEvidence> {
        self.evidence
    }
}

const INPUT_PAIR_COUNT: usize = InputOperationKind::ALL.len() * InputDelivery::ALL.len();

/// Capability metadata for every operation-and-route pair on one target.
///
/// A default capability marks every pair unsupported. Callers inspect one
/// [`InputRouteCapability`] at a time, so support, scope, focus, permission,
/// coordinate, and evidence facts cannot be accidentally combined across routes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InputCapability {
    support: [CapabilitySupport; INPUT_PAIR_COUNT],
    focus: [bool; INPUT_PAIR_COUNT],
    spaces: [u8; INPUT_PAIR_COUNT],
    permissions: [Option<PermissionKind>; INPUT_PAIR_COUNT],
    evidence: [Option<SubmissionEvidence>; INPUT_PAIR_COUNT],
}

impl InputCapability {
    /// A target that accepts no input at all.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            support: [CapabilitySupport::Unsupported; INPUT_PAIR_COUNT],
            focus: [false; INPUT_PAIR_COUNT],
            spaces: [0; INPUT_PAIR_COUNT],
            permissions: [None; INPUT_PAIR_COUNT],
            evidence: [None; INPUT_PAIR_COUNT],
        }
    }

    /// Configures one operation-and-route pair.
    #[must_use]
    pub const fn with_pair(
        mut self,
        operation: InputOperationKind,
        route: InputDelivery,
        support: CapabilitySupport,
        evidence: SubmissionEvidence,
    ) -> Self {
        let index = pair_index(operation, route);
        self.support[index] = support;
        self.evidence[index] = Some(evidence);
        self
    }

    /// Records that one operation-and-route pair requires focus.
    #[must_use]
    pub const fn with_focus_required(
        mut self,
        operation: InputOperationKind,
        route: InputDelivery,
    ) -> Self {
        self.focus[pair_index(operation, route)] = true;
        self
    }

    /// Adds a coordinate space accepted by pointer input over `route`.
    #[must_use]
    pub const fn with_pointer_space(
        mut self,
        route: InputDelivery,
        space: CoordinateSpace,
    ) -> Self {
        self.spaces[pair_index(InputOperationKind::Pointer, route)] |= space_bit(space);
        self
    }

    /// Records the authorization one operation-and-route pair ordinarily needs.
    #[must_use]
    pub const fn with_permission(
        mut self,
        operation: InputOperationKind,
        route: InputDelivery,
        permission: PermissionKind,
    ) -> Self {
        self.permissions[pair_index(operation, route)] = Some(permission);
        self
    }

    /// Returns the complete metadata for one operation-and-route pair.
    #[must_use]
    pub const fn pair(
        self,
        operation: InputOperationKind,
        route: InputDelivery,
    ) -> InputRouteCapability {
        let index = pair_index(operation, route);
        InputRouteCapability {
            operation,
            route,
            support: self.support[index],
            focus_required: self.focus[index],
            pointer_spaces: self.spaces[index],
            permission: self.permissions[index],
            evidence: self.evidence[index],
        }
    }

    /// Reports whether any pair is supported or attemptable with unknown
    /// application compatibility.
    #[must_use]
    pub const fn is_available(self) -> bool {
        let mut index = 0;
        while index < INPUT_PAIR_COUNT {
            if self.support[index].may_attempt() {
                return true;
            }
            index += 1;
        }
        false
    }
}

impl Default for InputCapability {
    fn default() -> Self {
        Self::none()
    }
}

const fn pair_index(operation: InputOperationKind, route: InputDelivery) -> usize {
    operation.index() as usize * InputDelivery::ALL.len() + route.index()
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
        CapabilitySupport, InputAddressScope, InputCapability, InputDelivery, InputOperationKind,
        SubmissionEvidence, TargetCapability, TargetKind,
    };
    use crate::geometry::CoordinateSpace;
    use crate::permission::PermissionKind;

    #[test]
    fn a_capture_only_target_accepts_no_input() {
        let capability = TargetCapability::capture_only(TargetKind::Display);

        assert_eq!(capability.kind(), Some(TargetKind::Display));
        assert_eq!(capability.capture(), CapabilitySupport::Supported);
        assert!(!capability.input().is_available());
        for operation in InputOperationKind::ALL {
            for route in InputDelivery::ALL {
                assert_eq!(
                    capability.input().pair(operation, route).support(),
                    CapabilitySupport::Unsupported
                );
            }
        }
    }

    #[test]
    fn a_supported_pair_does_not_imply_the_cross_product() {
        let input = InputCapability::none()
            .with_pair(
                InputOperationKind::Keyboard,
                InputDelivery::WindowMessage,
                CapabilitySupport::Supported,
                SubmissionEvidence::TargetProtocolAcknowledgement,
            )
            .with_pair(
                InputOperationKind::Pointer,
                InputDelivery::System,
                CapabilitySupport::Supported,
                SubmissionEvidence::SystemInputAdmission,
            );

        assert_eq!(
            input
                .pair(InputOperationKind::Keyboard, InputDelivery::WindowMessage)
                .support(),
            CapabilitySupport::Supported
        );
        assert_eq!(
            input
                .pair(InputOperationKind::Pointer, InputDelivery::System)
                .support(),
            CapabilitySupport::Supported
        );
        assert_eq!(
            input
                .pair(InputOperationKind::Pointer, InputDelivery::WindowMessage)
                .support(),
            CapabilitySupport::Unsupported,
            "window-message pointer input was never advertised"
        );
        assert_eq!(
            input
                .pair(InputOperationKind::Keyboard, InputDelivery::System)
                .support(),
            CapabilitySupport::Unsupported,
            "system keyboard input was never advertised"
        );
    }

    #[test]
    fn every_pair_has_independent_metadata() {
        for operation in InputOperationKind::ALL {
            for route in InputDelivery::ALL {
                let single = InputCapability::none().with_pair(
                    operation,
                    route,
                    CapabilitySupport::Unknown,
                    SubmissionEvidence::InvocationOnly,
                );
                for other_operation in InputOperationKind::ALL {
                    for other_route in InputDelivery::ALL {
                        assert_eq!(
                            single.pair(other_operation, other_route).may_attempt(),
                            other_operation == operation && other_route == route,
                            "{operation}/{route} leaked into {other_operation}/{other_route}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn unknown_is_attemptable_and_unsupported_is_not() {
        let input = InputCapability::none().with_pair(
            InputOperationKind::Text,
            InputDelivery::ProcessDirected,
            CapabilitySupport::Unknown,
            SubmissionEvidence::InvocationOnly,
        );

        let unknown = input.pair(InputOperationKind::Text, InputDelivery::ProcessDirected);
        assert_eq!(unknown.support(), CapabilitySupport::Unknown);
        assert!(unknown.may_attempt());
        assert_eq!(unknown.address_scope(), InputAddressScope::OwningProcess);
        assert_eq!(unknown.evidence(), Some(SubmissionEvidence::InvocationOnly));
        assert!(
            !input
                .pair(InputOperationKind::Text, InputDelivery::WindowMessage)
                .may_attempt()
        );
    }

    #[test]
    fn focus_permission_and_coordinates_are_pair_local() {
        let input = InputCapability::none()
            .with_pair(
                InputOperationKind::Pointer,
                InputDelivery::System,
                CapabilitySupport::Supported,
                SubmissionEvidence::SystemInputAdmission,
            )
            .with_pair(
                InputOperationKind::Text,
                InputDelivery::System,
                CapabilitySupport::Supported,
                SubmissionEvidence::SystemInputAdmission,
            )
            .with_focus_required(InputOperationKind::Text, InputDelivery::System)
            .with_pointer_space(InputDelivery::System, CoordinateSpace::CapturePixels)
            .with_pointer_space(InputDelivery::System, CoordinateSpace::DesktopLogical)
            .with_permission(
                InputOperationKind::Text,
                InputDelivery::System,
                PermissionKind::InputControl,
            );

        let pointer = input.pair(InputOperationKind::Pointer, InputDelivery::System);
        assert!(!pointer.focus_required());
        assert!(pointer.accepts_pointer_space(CoordinateSpace::CapturePixels));
        assert!(pointer.accepts_pointer_space(CoordinateSpace::DesktopLogical));
        assert!(!pointer.accepts_pointer_space(CoordinateSpace::FrameNormalized));
        assert_eq!(pointer.permission(), None);

        let text = input.pair(InputOperationKind::Text, InputDelivery::System);
        assert!(text.focus_required());
        assert_eq!(text.permission(), Some(PermissionKind::InputControl));
        assert_eq!(text.address_scope(), InputAddressScope::FocusedSystem);
    }

    #[test]
    fn capability_records_capture_and_pair_authorizations_separately() {
        let capability = TargetCapability::new(
            TargetKind::Window,
            CapabilitySupport::Supported,
            InputCapability::none()
                .with_pair(
                    InputOperationKind::Pointer,
                    InputDelivery::System,
                    CapabilitySupport::Supported,
                    SubmissionEvidence::SystemInputAdmission,
                )
                .with_permission(
                    InputOperationKind::Pointer,
                    InputDelivery::System,
                    PermissionKind::InputControl,
                ),
        )
        .with_capture_permission(PermissionKind::ScreenCapture);

        assert_eq!(
            capability.capture_permission(),
            Some(PermissionKind::ScreenCapture)
        );
        assert_eq!(
            capability
                .input()
                .pair(InputOperationKind::Pointer, InputDelivery::System)
                .permission(),
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
