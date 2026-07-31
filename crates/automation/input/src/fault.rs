//! Input rules that could not be satisfied.

use std::fmt;

use mado_pilot_core::{Error, Interruption, Status};

/// An input rule that could not be satisfied.
///
/// Typed so a test — and a caller — can assert which rule was broken rather than
/// reading a message. It converts into the public [`Error`] at the package
/// boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum InputFault {
    /// The target identity was issued by another engine or another provider.
    ForeignTarget,
    /// No target with that identity is known to this provider.
    UnknownTarget,
    /// The target this provider issued the identity for no longer exists.
    TargetLost,
    /// The capture and input providers wired together are different providers.
    ///
    /// One provider's target identity means nothing to another, so a pairing that
    /// was allowed would deliver input to whatever happened to share an ordinal.
    ProviderMismatch,
    /// The requested operation and delivery combination is not in the accepted
    /// descriptor.
    UnsupportedCombination,
    /// The request declares no delivery mechanism, or names one twice.
    InvalidDeliveryPlan,
    /// Every allowed delivery mechanism refused the sequence.
    DeliveryUnavailable,
    /// The sequence is empty, longer than the accepted limit, or contains an
    /// event that exceeds its own bound.
    SequenceOutOfBounds,
    /// A pointer position is expressed in a coordinate space the target does not
    /// accept.
    UnsupportedCoordinate,
    /// The geometry policy needs the frame its coordinates came from, and the
    /// request named none.
    MissingCoordinateSource,
    /// The target moved or resized since the frame the coordinates came from, and
    /// the geometry policy refuses that.
    GeometryChanged,
    /// The operation needs the target focused, and the focus policy does not allow
    /// focusing it.
    FocusRequired,
    /// The operating system refused to focus the target.
    FocusRefused,
    /// The operating system withheld authorization for input.
    NotAuthorized,
    /// The operating system's own policy refused the delivery: integrity level,
    /// interface privilege isolation, or a comparable restriction.
    PolicyRefused,
    /// The controller is closed or closing, so it accepts no new sequence.
    ControllerClosed,
    /// The operation was cancelled before the next event.
    ///
    /// A receipt needs the two interruptions as faults of its own: a sequence that
    /// stopped part-way reports how far it got, and returning the interruption as
    /// the operation's error instead would discard that count.
    Cancelled,
    /// The operation's deadline passed before the next event.
    DeadlineExceeded,
    /// The platform reported a failure that none of the above explains.
    DeliveryFailed,
}

impl InputFault {
    /// Returns the public status this fault reports as.
    #[must_use]
    pub const fn status(self) -> Status {
        match self {
            InputFault::ForeignTarget
            | InputFault::UnknownTarget
            | InputFault::ProviderMismatch
            | InputFault::InvalidDeliveryPlan
            | InputFault::SequenceOutOfBounds
            | InputFault::MissingCoordinateSource => Status::InvalidArgument,
            InputFault::UnsupportedCombination
            | InputFault::UnsupportedCoordinate
            | InputFault::DeliveryUnavailable
            | InputFault::FocusRequired => Status::Unsupported,
            InputFault::TargetLost => Status::TargetLost,
            InputFault::ControllerClosed => Status::Closed,
            InputFault::Cancelled => Status::Cancelled,
            InputFault::DeadlineExceeded => Status::DeadlineExceeded,
            InputFault::GeometryChanged
            | InputFault::FocusRefused
            | InputFault::NotAuthorized
            | InputFault::PolicyRefused
            | InputFault::DeliveryFailed => Status::InputFailed,
        }
    }

    const fn detail(self) -> &'static str {
        match self {
            InputFault::ForeignTarget => "target identity was not issued by this provider",
            InputFault::UnknownTarget => "no such target",
            InputFault::TargetLost => "target no longer exists",
            InputFault::ProviderMismatch => "capture and input providers do not match",
            InputFault::UnsupportedCombination => {
                "requested operation and delivery combination is not supported"
            }
            InputFault::InvalidDeliveryPlan => "delivery plan is empty or repeats a mechanism",
            InputFault::DeliveryUnavailable => "no allowed delivery mechanism was available",
            InputFault::SequenceOutOfBounds => "input sequence exceeds its declared bounds",
            InputFault::UnsupportedCoordinate => {
                "pointer coordinate space is not accepted by this target"
            }
            InputFault::MissingCoordinateSource => {
                "geometry policy requires the source frame of its coordinates"
            }
            InputFault::GeometryChanged => "target geometry changed since the source frame",
            InputFault::FocusRequired => "operation requires focus the focus policy withholds",
            InputFault::FocusRefused => "the operating system refused to focus the target",
            InputFault::NotAuthorized => "input control is not authorized",
            InputFault::PolicyRefused => "operating-system policy refused the delivery",
            InputFault::ControllerClosed => "input controller is closed",
            InputFault::Cancelled => "operation was cancelled",
            InputFault::DeadlineExceeded => "operation deadline passed",
            InputFault::DeliveryFailed => "input delivery failed",
        }
    }
}

impl From<Interruption> for InputFault {
    fn from(interruption: Interruption) -> Self {
        match interruption {
            Interruption::Cancelled => InputFault::Cancelled,
            Interruption::DeadlineExceeded => InputFault::DeadlineExceeded,
        }
    }
}

impl fmt::Display for InputFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.detail())
    }
}

impl std::error::Error for InputFault {}

impl From<InputFault> for Error {
    fn from(fault: InputFault) -> Self {
        Error::new(fault.status(), fault.detail())
    }
}

#[cfg(test)]
mod tests {
    use super::InputFault;
    use mado_pilot_core::{Error, Status};

    #[test]
    fn faults_map_to_public_statuses() {
        assert_eq!(InputFault::ForeignTarget.status(), Status::InvalidArgument);
        assert_eq!(
            InputFault::UnsupportedCombination.status(),
            Status::Unsupported
        );
        assert_eq!(InputFault::TargetLost.status(), Status::TargetLost);
        assert_eq!(InputFault::ControllerClosed.status(), Status::Closed);
        assert_eq!(InputFault::NotAuthorized.status(), Status::InputFailed);
    }

    #[test]
    fn a_provider_mismatch_is_a_caller_mistake_rather_than_a_platform_failure() {
        assert_eq!(
            InputFault::ProviderMismatch.status(),
            Status::InvalidArgument,
            "no platform was asked anything yet"
        );
    }

    #[test]
    fn an_interruption_becomes_the_fault_a_receipt_records() {
        use mado_pilot_core::Interruption;

        assert_eq!(
            InputFault::from(Interruption::Cancelled),
            InputFault::Cancelled
        );
        assert_eq!(
            InputFault::from(Interruption::DeadlineExceeded),
            InputFault::DeadlineExceeded
        );
        assert_eq!(InputFault::Cancelled.status(), Status::Cancelled);
        assert_eq!(
            InputFault::DeadlineExceeded.status(),
            Status::DeadlineExceeded
        );
    }

    #[test]
    fn a_fault_converts_into_the_public_error() {
        let error: Error = InputFault::FocusRefused.into();

        assert_eq!(error.status(), Status::InputFailed);
        assert!(!error.detail().is_empty());
    }
}
