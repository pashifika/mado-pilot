//! Declarative defaults selected when constructing a Windows engine.

use mado_pilot_capture::CapturePacingRequest;

/// Immutable Windows engine configuration without native handles.
///
/// Inheritance leaves the common engine request eligible. Explicit source
/// default bypasses that common request without inventing a native interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowsConfig {
    capture_pacing: CapturePacingRequest,
}

impl WindowsConfig {
    /// Creates an inheriting configuration without consulting the OS.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            capture_pacing: CapturePacingRequest::inherit(),
        }
    }

    /// Replaces this block's entire capture pacing selection.
    #[must_use]
    pub const fn with_capture_pacing(mut self, pacing: CapturePacingRequest) -> Self {
        self.capture_pacing = pacing;
        self
    }

    /// Returns this block's capture pacing selection.
    #[must_use]
    pub const fn capture_pacing(&self) -> CapturePacingRequest {
        self.capture_pacing
    }
}
