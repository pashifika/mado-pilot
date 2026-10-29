//! Platform-neutral OCR execution-provider selection and observed initialization facts.

use mado_pilot_core::{Error, Result, Status};

use crate::ProviderProfileId;

/// Bounded CPU execution options for each detector and recognizer session.
///
/// One through eight intra-op threads are permitted as a finite resource bound,
/// not a performance or recognition-quality gate. The default is one thread
/// with spinning enabled; callers choose other settings from their measurements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OcrCpuOptions {
    intra_threads: u32,
    allow_spinning: bool,
}

impl OcrCpuOptions {
    /// Selects one through eight intra-op threads per session, with spinning enabled.
    ///
    /// # Errors
    /// Returns [`Status::InvalidArgument`] outside the inclusive range `1..=8`.
    pub fn new(intra_threads: u32) -> Result<Self> {
        if !(1..=8).contains(&intra_threads) {
            return Err(Error::new(
                Status::InvalidArgument,
                "OCR CPU intra-op thread count must be between one and eight",
            ));
        }
        Ok(Self {
            intra_threads,
            allow_spinning: true,
        })
    }

    /// Selects whether intra-op workers may spin briefly before blocking.
    #[must_use]
    pub const fn with_spinning(mut self, allow_spinning: bool) -> Self {
        self.allow_spinning = allow_spinning;
        self
    }

    /// Returns the intra-op thread count for each session.
    #[must_use]
    pub const fn intra_threads(self) -> u32 {
        self.intra_threads
    }

    /// Returns whether intra-op workers may spin before blocking.
    #[must_use]
    pub const fn allow_spinning(self) -> bool {
        self.allow_spinning
    }
}

impl Default for OcrCpuOptions {
    fn default() -> Self {
        Self {
            intra_threads: 1,
            allow_spinning: true,
        }
    }
}

/// Execution provider active for one immutable OCR backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OcrExecutionProvider {
    /// ONNX Runtime's built-in CPU provider.
    Cpu,
    /// NVIDIA CUDA execution provider.
    Cuda,
    /// Apple CoreML execution provider.
    CoreMl,
}

/// Closed initialization-time provider policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OcrExecutionProviderPolicy {
    /// Use CPU and perform no accelerator initialization.
    Cpu,
    /// Prefer the release-qualified target accelerator, otherwise use CPU; an
    /// accepted accelerator initialization failure may also fall back before publication.
    AutoPreferAccelerator,
    /// Prefer CUDA and initialize CPU if it fails before publication.
    PreferCuda,
    /// Require CUDA and publish nothing if it cannot initialize.
    RequireCuda,
    /// Prefer CoreML and initialize CPU if it fails before publication.
    PreferCoreMl,
    /// Require CoreML and publish nothing if it cannot initialize.
    RequireCoreMl,
}

/// Bounded reason a preferred accelerator initialized CPU instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OcrProviderFallbackReason {
    /// The requested accelerator is not defined for the release target.
    UnsupportedTarget,
    /// The binary lacks the applicable provider feature.
    BuildCapabilityUnavailable,
    /// The loaded native runtime does not make the provider available.
    ProviderUnavailable,
    /// One controlled provider dependency is absent or incompatible.
    DependencyUnavailable,
    /// Provider registration failed before session creation.
    RegistrationFailed,
    /// Detector or recognizer session construction failed.
    SessionCreationFailed,
    /// Provider-backed graph or profile validation failed.
    GraphRejected,
    /// Target qualification rejected this provider for the release.
    QualificationRejected,
}

/// Immutable public facts about one engine's OCR provider initialization.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OcrProviderDescriptor {
    requested_policy: OcrExecutionProviderPolicy,
    active_provider: OcrExecutionProvider,
    fallback_reason: Option<OcrProviderFallbackReason>,
    runtime_profile: ProviderProfileId,
}

impl OcrProviderDescriptor {
    /// Creates immutable provider facts owned independently of caller configuration.
    #[must_use]
    pub const fn new(
        requested_policy: OcrExecutionProviderPolicy,
        active_provider: OcrExecutionProvider,
        fallback_reason: Option<OcrProviderFallbackReason>,
        runtime_profile: ProviderProfileId,
    ) -> Self {
        Self {
            requested_policy,
            active_provider,
            fallback_reason,
            runtime_profile,
        }
    }

    /// Returns the caller's requested provider policy.
    #[must_use]
    pub const fn requested_policy(&self) -> OcrExecutionProviderPolicy {
        self.requested_policy
    }

    /// Returns the provider active for detector and recognizer.
    #[must_use]
    pub const fn active_provider(&self) -> OcrExecutionProvider {
        self.active_provider
    }

    /// Returns whether initialization selected CPU after an accelerator failure.
    #[must_use]
    pub const fn initialization_fell_back(&self) -> bool {
        self.fallback_reason.is_some()
    }

    /// Returns the bounded initialization fallback reason, when one occurred.
    #[must_use]
    pub const fn fallback_reason(&self) -> Option<OcrProviderFallbackReason> {
        self.fallback_reason
    }

    /// Returns the active runtime/provider profile identity.
    #[must_use]
    pub const fn runtime_profile(&self) -> &ProviderProfileId {
        &self.runtime_profile
    }
}

#[cfg(test)]
mod tests {
    use mado_pilot_core::Status;

    use super::OcrCpuOptions;

    #[test]
    fn cpu_options_enforce_resource_bounds_without_filtering_unmeasured_counts() {
        for threads in [0, 9, u32::MAX] {
            assert_eq!(
                OcrCpuOptions::new(threads)
                    .expect_err("thread count must remain explicitly bounded")
                    .status(),
                Status::InvalidArgument
            );
        }
        for threads in 1..=8 {
            assert!(OcrCpuOptions::new(threads).is_ok());
        }
    }
}
