//! Platform-neutral OCR execution-provider selection and observed initialization facts.

use crate::ProviderProfileId;

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
