//! Typed discovery requests, and what filtering is allowed to mean.
//!
//! A filter narrows a provider's current result set and nothing else. It cannot
//! reach a target the provider did not list, and it never turns a window title, a
//! process identifier, or a reusable native handle into durable identity: those
//! are metadata an operating system reassigns freely, and a caller that had
//! learned to re-find a target by its title would silently start driving whatever
//! took its place. [`TargetId`](mado_pilot_core::TargetId) is the only thing that
//! says two observations are the same target.

use mado_pilot_core::{InputDelivery, InputOperationKind, TargetKind};

use crate::descriptor::TargetDescription;

/// What a caller is looking for among a provider's current targets.
///
/// A default request matches everything the provider listed, which is the
/// unfiltered discovery a caller already had. Every filter below narrows that
/// set: they combine with and, and none of them widens it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiscoveryRequest {
    kind: Option<TargetKind>,
    require_capture: bool,
    required_input: Option<(InputOperationKind, InputDelivery)>,
    name_contains: Option<String>,
}

impl DiscoveryRequest {
    /// Returns a request that matches every target the provider listed.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Selects only targets the provider classified as `kind`.
    ///
    /// A provider that does not classify its targets matches no kind filter. It
    /// has not said that its targets are windows, and treating an unclassified
    /// target as either kind would answer a question nobody asked it.
    #[must_use]
    pub fn with_kind(mut self, kind: TargetKind) -> Self {
        self.kind = Some(kind);
        self
    }

    /// Selects only targets whose capture may be attempted.
    #[must_use]
    pub const fn requiring_capture(mut self) -> Self {
        self.require_capture = true;
        self
    }

    /// Selects only targets that accept `kind` through `delivery`.
    ///
    /// The pair is required together, because a target that accepts keystrokes
    /// through the system input path has said nothing about accepting them in the
    /// background.
    #[must_use]
    pub const fn requiring_input(
        mut self,
        kind: InputOperationKind,
        delivery: InputDelivery,
    ) -> Self {
        self.required_input = Some((kind, delivery));
        self
    }

    /// Selects only targets whose descriptive name contains `text`.
    ///
    /// This is a filter over mutable metadata and never identity. The comparison
    /// is an exact substring match: case folding depends on locale and on the
    /// Unicode version a host was built against, and a filter whose result
    /// changed with either would not be deterministic.
    #[must_use]
    pub fn with_name_containing(mut self, text: impl Into<String>) -> Self {
        self.name_contains = Some(text.into());
        self
    }

    /// Returns the target kind filter, if any.
    #[must_use]
    pub const fn kind(&self) -> Option<TargetKind> {
        self.kind
    }

    /// Reports whether capture support is required.
    #[must_use]
    pub const fn requires_capture(&self) -> bool {
        self.require_capture
    }

    /// Returns the required input operation and delivery pair, if any.
    #[must_use]
    pub const fn required_input(&self) -> Option<(InputOperationKind, InputDelivery)> {
        self.required_input
    }

    /// Returns the descriptive-name substring filter, if any.
    #[must_use]
    pub fn name_containing(&self) -> Option<&str> {
        self.name_contains.as_deref()
    }

    /// Reports whether `description` satisfies every filter in this request.
    ///
    /// Providers use this so that one filter meaning applies to all of them, and
    /// callers may use it to explain a result they did not expect.
    #[must_use]
    pub fn accepts(&self, description: &TargetDescription) -> bool {
        let capability = description.capability();
        if let Some(kind) = self.kind
            && capability.kind() != Some(kind)
        {
            return false;
        }
        if self.require_capture && !capability.capture().may_attempt() {
            return false;
        }
        if let Some((operation, delivery)) = self.required_input
            && !capability.input().supports(operation, delivery)
        {
            return false;
        }
        if let Some(text) = self.name_containing()
            && !description.name().contains(text)
        {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::DiscoveryRequest;
    use crate::descriptor::{CoordinateSupport, PixelFormat, TargetDescription};
    use mado_pilot_core::{
        CapabilitySupport, IdentityIssuer, InputCapability, InputDelivery, InputOperationKind,
        PixelExtent, ProviderId, TargetCapability, TargetId, TargetKind,
    };

    const WINDOWS: ProviderId = ProviderId::new("windows");

    fn target(id: TargetId, name: &str, capability: TargetCapability) -> TargetDescription {
        TargetDescription::new(
            id,
            name,
            PixelExtent::new(64, 48),
            PixelFormat::Bgra8,
            CoordinateSupport::with_target_placement(),
        )
        .with_capability(capability)
    }

    fn ids(count: usize) -> Vec<TargetId> {
        let issuer = IdentityIssuer::new();
        (0..count)
            .map(|_| issuer.issue_target(WINDOWS).expect("issued"))
            .collect()
    }

    #[test]
    fn a_default_request_matches_every_listed_target() {
        let ids = ids(2);
        let classified = target(
            ids[0],
            "Editor",
            TargetCapability::capture_only(TargetKind::Window),
        );
        let unclassified = target(ids[1], "sequence", TargetCapability::unclassified());
        let request = DiscoveryRequest::new();

        assert!(request.accepts(&classified));
        assert!(request.accepts(&unclassified));
    }

    #[test]
    fn a_kind_filter_selects_only_that_classification() {
        let ids = ids(3);
        let window = target(
            ids[0],
            "Editor",
            TargetCapability::capture_only(TargetKind::Window),
        );
        let display = target(
            ids[1],
            "Built-in",
            TargetCapability::capture_only(TargetKind::Display),
        );
        let unclassified = target(ids[2], "sequence", TargetCapability::unclassified());
        let request = DiscoveryRequest::new().with_kind(TargetKind::Window);

        assert!(request.accepts(&window));
        assert!(!request.accepts(&display));
        assert!(
            !request.accepts(&unclassified),
            "a provider that classifies nothing has not claimed a window"
        );
    }

    #[test]
    fn a_capture_filter_keeps_targets_whose_capture_is_only_unknown() {
        let ids = ids(2);
        let unknown = target(
            ids[0],
            "Elevated",
            TargetCapability::new(
                TargetKind::Window,
                CapabilitySupport::Unknown,
                InputCapability::none(),
            ),
        );
        let refused = target(
            ids[1],
            "Protected",
            TargetCapability::new(
                TargetKind::Window,
                CapabilitySupport::Unsupported,
                InputCapability::none(),
            ),
        );
        let request = DiscoveryRequest::new().requiring_capture();

        assert!(request.accepts(&unknown));
        assert!(!request.accepts(&refused));
    }

    #[test]
    fn an_input_filter_requires_the_exact_operation_and_delivery_pair() {
        let ids = ids(1);
        let background_keyboard = target(
            ids[0],
            "Editor",
            TargetCapability::new(
                TargetKind::Window,
                CapabilitySupport::Supported,
                InputCapability::none().with_pair(
                    InputOperationKind::Keyboard,
                    InputDelivery::BackgroundTarget,
                ),
            ),
        );

        assert!(
            DiscoveryRequest::new()
                .requiring_input(
                    InputOperationKind::Keyboard,
                    InputDelivery::BackgroundTarget
                )
                .accepts(&background_keyboard)
        );
        assert!(
            !DiscoveryRequest::new()
                .requiring_input(InputOperationKind::Keyboard, InputDelivery::System)
                .accepts(&background_keyboard),
            "the pair is what was advertised, not the two axes separately"
        );
        assert!(
            !DiscoveryRequest::new()
                .requiring_input(InputOperationKind::Pointer, InputDelivery::BackgroundTarget)
                .accepts(&background_keyboard)
        );
    }

    #[test]
    fn a_name_filter_is_an_exact_substring_over_mutable_metadata() {
        let ids = ids(2);
        let first = target(ids[0], "Notes — Untitled", TargetCapability::unclassified());
        let second = target(ids[1], "notes", TargetCapability::unclassified());
        let request = DiscoveryRequest::new().with_name_containing("Notes");

        assert!(request.accepts(&first));
        assert!(
            !request.accepts(&second),
            "case folding would make the result depend on the host"
        );
        assert_eq!(request.name_containing(), Some("Notes"));
    }

    #[test]
    fn filters_combine_with_and() {
        let ids = ids(1);
        let window = target(
            ids[0],
            "Editor",
            TargetCapability::capture_only(TargetKind::Window),
        );
        let request = DiscoveryRequest::new()
            .with_kind(TargetKind::Window)
            .requiring_capture()
            .with_name_containing("Editor");

        assert!(request.accepts(&window));
        assert!(
            !request
                .clone()
                .with_name_containing("Terminal")
                .accepts(&window),
            "one failing filter refuses the target"
        );
        assert!(
            !request
                .requiring_input(InputOperationKind::Text, InputDelivery::System)
                .accepts(&window)
        );
    }
}
