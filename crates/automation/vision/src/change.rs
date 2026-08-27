//! Closed visual-change policy selected by the G-005 recorded-sequence gate.
//!
//! A decision from this module has narrow authority: compatible unchanged pixels
//! may skip routine visual analysis, but cannot confirm a template, advance
//! stability, cross a stream/epoch/geometry boundary, or trigger input.

use std::fmt;

use mado_pilot_capture::{CpuMapping, PixelFormat};

/// Stable numeric policy code for conservative analysis of every transition.
pub const ANALYSIS_ALWAYS_POLICY_CODE: u32 = 0;
/// Stable numeric policy code for the accepted exact RGBA8 comparison.
pub const EXACT_RGBA_POLICY_CODE: u32 = 1;

/// One supported product change-detection policy.
///
/// Evaluation-only candidates and arbitrary thresholds are deliberately absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u32)]
pub enum ChangeDetectionPolicy {
    /// Admit every acceptable frame to routine visual analysis.
    AnalysisAlways = ANALYSIS_ALWAYS_POLICY_CODE,
    /// Compare every RGBA8 pixel in a compatible mapped region.
    #[default]
    ExactRgba = EXACT_RGBA_POLICY_CODE,
}

impl ChangeDetectionPolicy {
    /// Returns the stable closed policy code.
    #[must_use]
    pub const fn code(self) -> u32 {
        self as u32
    }

    /// Returns immutable authority and identity facts for this policy.
    #[must_use]
    pub const fn descriptor(self) -> ChangeDetectionDescriptor {
        match self {
            Self::AnalysisAlways => ChangeDetectionDescriptor::analysis_always(),
            Self::ExactRgba => ChangeDetectionDescriptor::exact_rgba(),
        }
    }
}

impl TryFrom<u32> for ChangeDetectionPolicy {
    type Error = UnsupportedChangeDetectionPolicy;

    fn try_from(code: u32) -> Result<Self, Self::Error> {
        match code {
            ANALYSIS_ALWAYS_POLICY_CODE => Ok(Self::AnalysisAlways),
            EXACT_RGBA_POLICY_CODE => Ok(Self::ExactRgba),
            _ => Err(UnsupportedChangeDetectionPolicy { code }),
        }
    }
}

/// A numeric change-detection policy code is not supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnsupportedChangeDetectionPolicy {
    code: u32,
}

impl UnsupportedChangeDetectionPolicy {
    /// Returns the rejected numeric code.
    #[must_use]
    pub const fn code(self) -> u32 {
        self.code
    }
}

impl fmt::Display for UnsupportedChangeDetectionPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "unsupported change-detection policy code {}",
            self.code
        )
    }
}

impl std::error::Error for UnsupportedChangeDetectionPolicy {}

/// Immutable selected-policy identity and authority facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChangeDetectionDescriptor {
    policy: ChangeDetectionPolicy,
    policy_id: &'static str,
    unchanged_may_skip_routine_analysis: bool,
    unchanged_confirms_presence: bool,
    unchanged_advances_consecutive_stability: bool,
    unchanged_creates_duration_stability: bool,
    unchanged_crosses_incompatible_identity_or_geometry: bool,
}

impl ChangeDetectionDescriptor {
    const fn analysis_always() -> Self {
        Self {
            policy: ChangeDetectionPolicy::AnalysisAlways,
            policy_id: "analysis-always-v1",
            unchanged_may_skip_routine_analysis: false,
            unchanged_confirms_presence: false,
            unchanged_advances_consecutive_stability: false,
            unchanged_creates_duration_stability: false,
            unchanged_crosses_incompatible_identity_or_geometry: false,
        }
    }

    const fn exact_rgba() -> Self {
        Self {
            policy: ChangeDetectionPolicy::ExactRgba,
            policy_id: "exact-rgba-v1",
            unchanged_may_skip_routine_analysis: true,
            unchanged_confirms_presence: false,
            unchanged_advances_consecutive_stability: false,
            unchanged_creates_duration_stability: false,
            unchanged_crosses_incompatible_identity_or_geometry: false,
        }
    }

    /// Returns the closed policy.
    #[must_use]
    pub const fn policy(self) -> ChangeDetectionPolicy {
        self.policy
    }

    /// Returns the reviewed policy identity used by evidence and diagnostics.
    #[must_use]
    pub const fn policy_id(self) -> &'static str {
        self.policy_id
    }

    /// Reports whether unchanged can skip routine analysis.
    #[must_use]
    pub const fn unchanged_may_skip_routine_analysis(self) -> bool {
        self.unchanged_may_skip_routine_analysis
    }

    /// Reports whether unchanged alone confirms template presence.
    #[must_use]
    pub const fn unchanged_confirms_presence(self) -> bool {
        self.unchanged_confirms_presence
    }

    /// Reports whether unchanged alone advances consecutive stability.
    #[must_use]
    pub const fn unchanged_advances_consecutive_stability(self) -> bool {
        self.unchanged_advances_consecutive_stability
    }

    /// Reports whether unchanged alone creates duration stability.
    #[must_use]
    pub const fn unchanged_creates_duration_stability(self) -> bool {
        self.unchanged_creates_duration_stability
    }

    /// Reports whether unchanged can cross incompatible identity or geometry.
    #[must_use]
    pub const fn unchanged_crosses_incompatible_identity_or_geometry(self) -> bool {
        self.unchanged_crosses_incompatible_identity_or_geometry
    }
}

/// The reviewed G-005 default descriptor.
pub const DEFAULT_CHANGE_DETECTION_DESCRIPTOR: ChangeDetectionDescriptor =
    ChangeDetectionDescriptor::exact_rgba();

/// A bounded decision with no authority beyond routine-analysis admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeDecision {
    /// Run routine visual analysis for the current frame.
    AnalysisRequired,
    /// Compatible mapped RGBA8 pixels are exactly unchanged.
    Unchanged,
}

/// Stateless executor for one closed change-detection policy.
///
/// The value is `Copy`, contains no worker, lock, queue, callback, or allocation,
/// and performs no environment lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ChangeDetector {
    policy: ChangeDetectionPolicy,
}

impl ChangeDetector {
    /// Creates a detector for a supported closed policy.
    #[must_use]
    pub const fn new(policy: ChangeDetectionPolicy) -> Self {
        Self { policy }
    }

    /// Creates the reviewed G-005 default detector.
    #[must_use]
    pub const fn selected_default() -> Self {
        Self::new(ChangeDetectionPolicy::ExactRgba)
    }

    /// Returns the detector's immutable descriptor.
    #[must_use]
    pub const fn descriptor(self) -> ChangeDetectionDescriptor {
        self.policy.descriptor()
    }

    /// Compares two already mapped regions under the detector's policy.
    ///
    /// Any unsupported format, stale/reversed frame, identity discontinuity,
    /// geometry/transform/region mismatch, descriptor mismatch, or malformed byte
    /// extent fails safe to [`ChangeDecision::AnalysisRequired`]. Only exact
    /// equality of every RGBA8 pixel in a compatible newer mapping returns
    /// [`ChangeDecision::Unchanged`]. Row padding is not pixel content and is not
    /// compared.
    #[must_use]
    pub fn compare(self, previous: &CpuMapping, current: &CpuMapping) -> ChangeDecision {
        if self.policy == ChangeDetectionPolicy::AnalysisAlways
            || !mappings_are_compatible(previous, current)
        {
            return ChangeDecision::AnalysisRequired;
        }

        let descriptor = previous.descriptor();
        let row_bytes = descriptor.row_bytes();
        let Ok(height) = usize::try_from(descriptor.extent().height()) else {
            return ChangeDecision::AnalysisRequired;
        };
        if row_bytes == 0
            || previous.bytes().len() != descriptor.byte_len()
            || current.bytes().len() != descriptor.byte_len()
        {
            return ChangeDecision::AnalysisRequired;
        }

        for row in 0..height {
            let Some(start) = row.checked_mul(descriptor.stride()) else {
                return ChangeDecision::AnalysisRequired;
            };
            let Some(end) = start.checked_add(row_bytes) else {
                return ChangeDecision::AnalysisRequired;
            };
            let Some(previous_row) = previous.bytes().get(start..end) else {
                return ChangeDecision::AnalysisRequired;
            };
            let Some(current_row) = current.bytes().get(start..end) else {
                return ChangeDecision::AnalysisRequired;
            };
            if previous_row != current_row {
                return ChangeDecision::AnalysisRequired;
            }
        }
        ChangeDecision::Unchanged
    }
}

fn mappings_are_compatible(previous: &CpuMapping, current: &CpuMapping) -> bool {
    let previous_stamp = previous.stamp();
    let current_stamp = current.stamp();
    previous_stamp.stream() == current_stamp.stream()
        && previous_stamp.epoch() == current_stamp.epoch()
        && previous_stamp.sequence() < current_stamp.sequence()
        && previous_stamp.geometry() == current_stamp.geometry()
        && previous.region() == current.region()
        && previous.descriptor() == current.descriptor()
        && previous.descriptor().format() == PixelFormat::Rgba8
        && previous.transform() == current.transform()
}
