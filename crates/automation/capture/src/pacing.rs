//! Atomic native capture pacing selection and truthful configuration reports.

use std::time::Duration;

use mado_pilot_core::{Error, Result, Status};

use crate::fault::CaptureFault;

/// One layer's native minimum-interval selection, including inheritance.
///
/// Resolution replaces the interval and required/preferred policy together.
/// A higher-priority preference may override a lower-priority requirement:
/// these selections are defaults, not hard caps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CapturePacingRequest {
    selected: Option<ResolvedCapturePacing>,
}

impl CapturePacingRequest {
    /// Leaves the lower-priority layer eligible. This is the default.
    #[must_use]
    pub const fn inherit() -> Self {
        Self { selected: None }
    }

    /// Bypasses lower-priority pacing and leaves native cadence untouched.
    ///
    /// Source default does not mean unlimited or a portable numerical FPS.
    #[must_use]
    pub const fn source_default() -> Self {
        ResolvedCapturePacing::source_default().as_request()
    }

    /// Requires a positive native minimum interval for a successful open.
    ///
    /// # Errors
    ///
    /// Returns [`Status::InvalidArgument`] for zero. The selected native adapter
    /// separately validates whether it can represent the positive duration.
    pub fn required(interval: Duration) -> Result<Self> {
        validate_interval(interval)?;
        Ok(ResolvedCapturePacing {
            selection: PacingSelection::Required(interval),
        }
        .as_request())
    }

    /// Prefers a positive native minimum interval, allowing capability absence.
    ///
    /// # Errors
    ///
    /// Returns [`Status::InvalidArgument`] for zero. Native representation errors
    /// remain errors even for a preference; only capability absence can fall back.
    pub fn preferred(interval: Duration) -> Result<Self> {
        validate_interval(interval)?;
        Ok(ResolvedCapturePacing {
            selection: PacingSelection::Preferred(interval),
        }
        .as_request())
    }

    /// Selects this entire request, or `fallback` when this layer inherits.
    #[must_use]
    pub const fn resolve(self, fallback: ResolvedCapturePacing) -> ResolvedCapturePacing {
        match self.selected {
            Some(selected) => selected,
            None => fallback,
        }
    }
}

fn validate_interval(interval: Duration) -> Result<()> {
    if interval.is_zero() {
        return Err(Error::new(
            Status::InvalidArgument,
            "capture pacing interval must be positive",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PacingSelection {
    SourceDefault,
    Required(Duration),
    Preferred(Duration),
}

/// A fully selected native pacing request that cannot contain inheritance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedCapturePacing {
    selection: PacingSelection,
}

impl ResolvedCapturePacing {
    /// Leaves the source's native cadence configuration untouched.
    #[must_use]
    pub const fn source_default() -> Self {
        Self {
            selection: PacingSelection::SourceDefault,
        }
    }

    /// Returns the requested positive interval, with no invented source default.
    #[must_use]
    pub const fn interval(self) -> Option<Duration> {
        match self.selection {
            PacingSelection::SourceDefault => None,
            PacingSelection::Required(interval) | PacingSelection::Preferred(interval) => {
                Some(interval)
            }
        }
    }

    /// Reports whether native pacing is required for a successful open.
    #[must_use]
    pub const fn is_required(self) -> bool {
        matches!(self.selection, PacingSelection::Required(_))
    }

    /// Reports whether capability absence may leave the interval unapplied.
    #[must_use]
    pub const fn is_preferred(self) -> bool {
        matches!(self.selection, PacingSelection::Preferred(_))
    }

    /// Turns this selection into an explicit, non-inheriting request.
    #[must_use]
    pub const fn as_request(self) -> CapturePacingRequest {
        CapturePacingRequest {
            selected: Some(self),
        }
    }
}

/// Why a source cannot apply a requested native capture interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PacingUnsupportedReason {
    /// The source has no native pacing facility, such as pull-driven replay.
    SourceCannotPace,
    /// The native source exists but its optional interval control is unavailable.
    NativeControlUnavailable,
}

/// The native configuration outcome, not measured output FPS or periodicity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CapturePacingOutcome {
    /// Native cadence configuration was left untouched.
    SourceDefault,
    /// The requested minimum was established by native configuration.
    Applied,
    /// Capability absence left a preferred interval unapplied.
    PreferredUnapplied(PacingUnsupportedReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PacingReportState {
    SourceDefault,
    Applied {
        request: ResolvedCapturePacing,
        configured: Duration,
    },
    PreferredUnapplied {
        interval: Duration,
        reason: PacingUnsupportedReason,
    },
}

/// Immutable native pacing configuration accepted by an opened session.
///
/// Factories prevent inherited/source-default applied reports, shorter-than-
/// requested configuration, and successful required-unapplied reports. The
/// report describes configuration, not observed FPS or resource savings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapturePacingReport {
    state: PacingReportState,
}

impl CapturePacingReport {
    /// Reports untouched native cadence without a numerical interval.
    #[must_use]
    pub const fn source_default() -> Self {
        Self {
            state: PacingReportState::SourceDefault,
        }
    }

    /// Reports an established native interval, including upward adjustment.
    ///
    /// # Errors
    ///
    /// Returns [`Status::InvalidArgument`] when `request` selects source default
    /// or `configured` is shorter than its requested positive minimum.
    pub fn applied(request: ResolvedCapturePacing, configured: Duration) -> Result<Self> {
        let Some(interval) = request.interval() else {
            return Err(Error::new(
                Status::InvalidArgument,
                "source-default capture pacing cannot be reported as applied",
            ));
        };
        if configured < interval {
            return Err(Error::new(
                Status::InvalidArgument,
                "configured capture interval is shorter than the requested minimum",
            ));
        }
        Ok(Self {
            state: PacingReportState::Applied {
                request,
                configured,
            },
        })
    }

    /// Reports capability absence for a preference, or untouched source default.
    ///
    /// # Errors
    ///
    /// Returns [`Status::Unsupported`] for a required interval. Permission,
    /// configuration, start, and interruption errors must not use this fallback.
    pub fn unsupported(
        request: ResolvedCapturePacing,
        reason: PacingUnsupportedReason,
    ) -> Result<Self> {
        match request.selection {
            PacingSelection::SourceDefault => Ok(Self::source_default()),
            PacingSelection::Required(_) => Err(CaptureFault::UnsupportedOption.into()),
            PacingSelection::Preferred(interval) => Ok(Self {
                state: PacingReportState::PreferredUnapplied { interval, reason },
            }),
        }
    }

    /// Returns the fully resolved request this report describes.
    #[must_use]
    pub const fn request(self) -> ResolvedCapturePacing {
        match self.state {
            PacingReportState::SourceDefault => ResolvedCapturePacing::source_default(),
            PacingReportState::Applied { request, .. } => request,
            PacingReportState::PreferredUnapplied { interval, .. } => ResolvedCapturePacing {
                selection: PacingSelection::Preferred(interval),
            },
        }
    }

    /// Returns the established native interval only when pacing was applied.
    #[must_use]
    pub const fn configured_interval(self) -> Option<Duration> {
        match self.state {
            PacingReportState::Applied { configured, .. } => Some(configured),
            PacingReportState::SourceDefault | PacingReportState::PreferredUnapplied { .. } => None,
        }
    }

    /// Returns whether native configuration was untouched, applied, or unavailable.
    #[must_use]
    pub const fn outcome(self) -> CapturePacingOutcome {
        match self.state {
            PacingReportState::SourceDefault => CapturePacingOutcome::SourceDefault,
            PacingReportState::Applied { .. } => CapturePacingOutcome::Applied,
            PacingReportState::PreferredUnapplied { reason, .. } => {
                CapturePacingOutcome::PreferredUnapplied(reason)
            }
        }
    }
}
