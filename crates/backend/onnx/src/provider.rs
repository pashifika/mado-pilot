use std::path::Path;

#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
    target_env = "msvc",
    feature = "cuda-provider"
))]
use std::path::PathBuf;
#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
    target_env = "msvc",
    feature = "cuda-provider"
))]
use std::sync::Mutex;

use mado_pilot_core::OperationContext;

#[cfg(any(
    all(
        target_os = "windows",
        target_arch = "x86_64",
        target_env = "msvc",
        feature = "cuda-provider"
    ),
    all(
        target_os = "macos",
        target_arch = "aarch64",
        feature = "coreml-provider"
    )
))]
use ort::ep::ExecutionProvider;
use ort::ep::{CPU, ExecutionProviderDispatch};

use crate::{
    OnnxBackendFault, OnnxExecutionProvider, OnnxExecutionProviderPolicy,
    OnnxProviderFallbackReason,
};

pub(crate) const CUDA_RECOGNIZER_OUTPUT_BYTES: usize = 128 * 1024 * 1024;

pub(crate) const fn recognizer_output_budget(provider: OnnxExecutionProvider) -> usize {
    match provider {
        OnnxExecutionProvider::Cuda => CUDA_RECOGNIZER_OUTPUT_BYTES,
        OnnxExecutionProvider::Cpu | OnnxExecutionProvider::CoreMl => crate::MAX_OUTPUT_BYTES,
    }
}

#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
    target_env = "msvc",
    feature = "cuda-provider"
))]
pub(crate) const CUDA_ARENA_LIMIT_BYTES: usize = 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProviderPlan {
    requested: OnnxExecutionProviderPolicy,
    candidate: OnnxExecutionProvider,
}

impl ProviderPlan {
    pub(crate) fn resolve(
        requested: OnnxExecutionProviderPolicy,
    ) -> Result<Self, OnnxProviderFallbackReason> {
        let candidate = match requested {
            OnnxExecutionProviderPolicy::Cpu => OnnxExecutionProvider::Cpu,
            OnnxExecutionProviderPolicy::PreferCuda | OnnxExecutionProviderPolicy::RequireCuda => {
                OnnxExecutionProvider::Cuda
            }
            OnnxExecutionProviderPolicy::PreferCoreMl
            | OnnxExecutionProviderPolicy::RequireCoreMl => OnnxExecutionProvider::CoreMl,
            OnnxExecutionProviderPolicy::AutoPreferAccelerator => target_accelerator()?,
        };
        if !target_supports(candidate) {
            return Err(OnnxProviderFallbackReason::UnsupportedTarget);
        }
        if !build_supports(candidate) {
            return Err(OnnxProviderFallbackReason::BuildCapabilityUnavailable);
        }
        if !release_supports(candidate) {
            return Err(OnnxProviderFallbackReason::QualificationRejected);
        }
        Ok(Self {
            requested,
            candidate,
        })
    }

    pub(crate) const fn requested(self) -> OnnxExecutionProviderPolicy {
        self.requested
    }

    pub(crate) const fn candidate(self) -> OnnxExecutionProvider {
        self.candidate
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderPreparationFault {
    Terminal(OnnxBackendFault),
    Provider(OnnxProviderFallbackReason),
}

impl From<OnnxBackendFault> for ProviderPreparationFault {
    fn from(fault: OnnxBackendFault) -> Self {
        Self::Terminal(fault)
    }
}

impl From<OnnxProviderFallbackReason> for ProviderPreparationFault {
    fn from(reason: OnnxProviderFallbackReason) -> Self {
        Self::Provider(reason)
    }
}

enum ProviderResources {
    None,
    #[cfg(all(
        target_os = "windows",
        target_arch = "x86_64",
        target_env = "msvc",
        feature = "cuda-provider"
    ))]
    Cuda(CudaProviderLibraries),
}

pub(crate) struct PreparedProvider {
    plan: ProviderPlan,
    resources: ProviderResources,
}

impl std::fmt::Debug for PreparedProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedProvider")
            .field("plan", &self.plan)
            .finish_non_exhaustive()
    }
}

impl PreparedProvider {
    fn new(plan: ProviderPlan, resources: ProviderResources) -> Self {
        Self { plan, resources }
    }

    pub(crate) fn cpu() -> Self {
        Self::new(
            ProviderPlan {
                requested: OnnxExecutionProviderPolicy::Cpu,
                candidate: OnnxExecutionProvider::Cpu,
            },
            ProviderResources::None,
        )
    }

    #[cfg(test)]
    pub(crate) fn for_test(candidate: OnnxExecutionProvider) -> Self {
        let requested = match candidate {
            OnnxExecutionProvider::Cpu => OnnxExecutionProviderPolicy::Cpu,
            OnnxExecutionProvider::Cuda => OnnxExecutionProviderPolicy::PreferCuda,
            OnnxExecutionProvider::CoreMl => OnnxExecutionProviderPolicy::PreferCoreMl,
        };
        Self::new(
            ProviderPlan {
                requested,
                candidate,
            },
            ProviderResources::None,
        )
    }

    pub(crate) const fn candidate(&self) -> OnnxExecutionProvider {
        self.plan.candidate()
    }

    pub(crate) fn commit(&mut self) -> Result<(), OnnxBackendFault> {
        let resources = std::mem::replace(&mut self.resources, ProviderResources::None);
        match resources {
            ProviderResources::None => Ok(()),
            #[cfg(all(
                target_os = "windows",
                target_arch = "x86_64",
                target_env = "msvc",
                feature = "cuda-provider"
            ))]
            ProviderResources::Cuda(libraries) => commit_cuda_libraries(libraries),
        }
    }
}

pub(crate) fn prepare(
    requested: OnnxExecutionProviderPolicy,
    provider_root: Option<&Path>,
    runtime_path: &Path,
    operation: &OperationContext,
) -> Result<PreparedProvider, ProviderPreparationFault> {
    checkpoint(operation)?;
    let plan = ProviderPlan::resolve(requested).map_err(ProviderPreparationFault::Provider)?;
    let resources = match plan.candidate() {
        OnnxExecutionProvider::Cpu => {
            if provider_root.is_some()
                && !matches!(
                    plan.requested(),
                    OnnxExecutionProviderPolicy::AutoPreferAccelerator
                )
            {
                return Err(OnnxProviderFallbackReason::DependencyUnavailable.into());
            }
            ProviderResources::None
        }
        OnnxExecutionProvider::CoreMl => {
            if provider_root.is_some() {
                return Err(OnnxProviderFallbackReason::DependencyUnavailable.into());
            }
            ProviderResources::None
        }
        OnnxExecutionProvider::Cuda => prepare_cuda_root(
            provider_root.ok_or(OnnxProviderFallbackReason::DependencyUnavailable)?,
            runtime_path,
            operation,
        )?,
    };
    checkpoint(operation)?;
    Ok(PreparedProvider::new(plan, resources))
}

pub(crate) fn unavailable_fault(reason: OnnxProviderFallbackReason) -> OnnxBackendFault {
    match reason {
        OnnxProviderFallbackReason::DependencyUnavailable => {
            OnnxBackendFault::ProviderDependencyUnavailable
        }
        OnnxProviderFallbackReason::RegistrationFailed
        | OnnxProviderFallbackReason::SessionCreationFailed
        | OnnxProviderFallbackReason::GraphRejected => {
            OnnxBackendFault::ProviderInitializationFailed
        }
        OnnxProviderFallbackReason::QualificationRejected => {
            OnnxBackendFault::ProviderQualificationRejected
        }
        OnnxProviderFallbackReason::UnsupportedTarget
        | OnnxProviderFallbackReason::BuildCapabilityUnavailable
        | OnnxProviderFallbackReason::ProviderUnavailable => OnnxBackendFault::ProviderUnavailable,
    }
}

pub(crate) fn dispatch(
    provider: OnnxExecutionProvider,
) -> Result<ExecutionProviderDispatch, OnnxProviderFallbackReason> {
    match provider {
        OnnxExecutionProvider::Cpu => Ok(CPU::default()
            .with_arena_allocator(false)
            .build()
            .error_on_failure()),
        OnnxExecutionProvider::Cuda => cuda_dispatch(),
        OnnxExecutionProvider::CoreMl => coreml_dispatch(),
    }
}

const fn target_supports(provider: OnnxExecutionProvider) -> bool {
    match provider {
        OnnxExecutionProvider::Cpu => true,
        OnnxExecutionProvider::Cuda => cfg!(all(
            target_os = "windows",
            target_arch = "x86_64",
            target_env = "msvc"
        )),
        OnnxExecutionProvider::CoreMl => {
            cfg!(all(target_os = "macos", target_arch = "aarch64"))
        }
    }
}

const fn build_supports(provider: OnnxExecutionProvider) -> bool {
    match provider {
        OnnxExecutionProvider::Cpu => true,
        OnnxExecutionProvider::Cuda => cfg!(all(
            target_os = "windows",
            target_arch = "x86_64",
            target_env = "msvc",
            feature = "cuda-provider"
        )),
        OnnxExecutionProvider::CoreMl => cfg!(all(
            target_os = "macos",
            target_arch = "aarch64",
            feature = "coreml-provider"
        )),
    }
}

const fn release_supports(provider: OnnxExecutionProvider) -> bool {
    !matches!(provider, OnnxExecutionProvider::CoreMl)
}

fn target_accelerator() -> Result<OnnxExecutionProvider, OnnxProviderFallbackReason> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return Ok(OnnxExecutionProvider::Cpu);
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64", target_env = "msvc"))]
    {
        return Ok(OnnxExecutionProvider::Cpu);
    }
    #[allow(unreachable_code)]
    Err(OnnxProviderFallbackReason::UnsupportedTarget)
}

#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
    target_env = "msvc",
    feature = "cuda-provider"
))]
const CUDA_PROVIDER_FILES: [&str; 17] = [
    "cudart64_13.dll",
    "cublasLt64_13.dll",
    "cublas64_13.dll",
    "curand64_10.dll",
    "cufft64_12.dll",
    "nvrtc-builtins64_130.dll",
    "nvrtc64_130_0.dll",
    "cudnn64_9.dll",
    "cudnn_adv64_9.dll",
    "cudnn_cnn64_9.dll",
    "cudnn_engines_precompiled64_9.dll",
    "cudnn_engines_runtime_compiled64_9.dll",
    "cudnn_graph64_9.dll",
    "cudnn_heuristic64_9.dll",
    "cudnn_ops64_9.dll",
    "onnxruntime_providers_shared.dll",
    "onnxruntime_providers_cuda.dll",
];
#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
    target_env = "msvc",
    feature = "cuda-provider"
))]
const CUDA_PROVIDER_LIBRARY: &str = "onnxruntime_providers_cuda.dll";

#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
    target_env = "msvc",
    feature = "cuda-provider"
))]
struct CudaProviderLibraries {
    root: PathBuf,
    _libraries: Vec<libloading::Library>,
}

#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
    target_env = "msvc",
    feature = "cuda-provider"
))]
static CUDA_PROVIDER_LIBRARIES: Mutex<Option<CudaProviderLibraries>> = Mutex::new(None);

#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
    target_env = "msvc",
    feature = "cuda-provider"
))]
fn commit_cuda_libraries(libraries: CudaProviderLibraries) -> Result<(), OnnxBackendFault> {
    let mut state = CUDA_PROVIDER_LIBRARIES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if state.is_some() {
        return Err(OnnxBackendFault::RuntimeConflict);
    }
    *state = Some(libraries);
    Ok(())
}

#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
    target_env = "msvc",
    feature = "cuda-provider"
))]
fn prepare_cuda_root(
    root: &Path,
    runtime_path: &Path,
    operation: &OperationContext,
) -> Result<ProviderResources, ProviderPreparationFault> {
    use libloading::os::windows::{
        LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32, Library as WindowsLibrary,
    };

    checkpoint(operation)?;
    if !root.is_absolute() || runtime_path.parent() != Some(root) {
        return Err(OnnxProviderFallbackReason::DependencyUnavailable.into());
    }
    let canonical = std::fs::canonicalize(root)
        .map_err(|_| OnnxProviderFallbackReason::DependencyUnavailable)?;
    if canonical != root
        || !canonical
            .metadata()
            .map_err(|_| OnnxProviderFallbackReason::DependencyUnavailable)?
            .is_dir()
    {
        return Err(OnnxProviderFallbackReason::DependencyUnavailable.into());
    }

    let mut file_count = 0_usize;
    for entry in std::fs::read_dir(&canonical)
        .map_err(|_| OnnxProviderFallbackReason::DependencyUnavailable)?
    {
        checkpoint(operation)?;
        let entry = entry.map_err(|_| OnnxProviderFallbackReason::DependencyUnavailable)?;
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .ok_or(OnnxProviderFallbackReason::DependencyUnavailable)?;
        if name != "onnxruntime.dll" && !CUDA_PROVIDER_FILES.contains(&name) {
            return Err(OnnxProviderFallbackReason::DependencyUnavailable.into());
        }
        let file_type = entry
            .file_type()
            .map_err(|_| OnnxProviderFallbackReason::DependencyUnavailable)?;
        let file = std::fs::canonicalize(&path)
            .map_err(|_| OnnxProviderFallbackReason::DependencyUnavailable)?;
        if file != path || !file_type.is_file() || file_type.is_symlink() {
            return Err(OnnxProviderFallbackReason::DependencyUnavailable.into());
        }
        file_count += 1;
    }
    if file_count != CUDA_PROVIDER_FILES.len() + 1 {
        return Err(OnnxProviderFallbackReason::DependencyUnavailable.into());
    }

    {
        let state = CUDA_PROVIDER_LIBRARIES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(loaded) = state.as_ref() {
            return if loaded.root == canonical {
                Ok(ProviderResources::None)
            } else {
                Err(OnnxProviderFallbackReason::DependencyUnavailable.into())
            };
        }
    }

    let mut libraries = Vec::with_capacity(CUDA_PROVIDER_FILES.len() - 1);
    for name in CUDA_PROVIDER_FILES {
        checkpoint(operation)?;
        let path = canonical.join(name);
        let file = std::fs::canonicalize(&path)
            .map_err(|_| OnnxProviderFallbackReason::DependencyUnavailable)?;
        if file != path
            || file.parent() != Some(canonical.as_path())
            || !file
                .metadata()
                .map_err(|_| OnnxProviderFallbackReason::DependencyUnavailable)?
                .is_file()
        {
            return Err(OnnxProviderFallbackReason::DependencyUnavailable.into());
        }
        if name == CUDA_PROVIDER_LIBRARY {
            // ONNX Runtime loads its provider library only after its own
            // environment exists. Eagerly running that DLL's initialization
            // routine fails even when every controlled dependency is present.
            continue;
        }
        // SAFETY: every path is an absolute canonical regular file directly in
        // the explicit provider root. Dependency search is restricted to that
        // file's directory and System32. Candidate ownership keeps every handle
        // live through session construction; publication commits them globally.
        let library = unsafe {
            WindowsLibrary::load_with_flags(
                &file,
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        }
        .map_err(|_| OnnxProviderFallbackReason::DependencyUnavailable)?;
        libraries.push(library.into());
    }
    checkpoint(operation)?;
    Ok(ProviderResources::Cuda(CudaProviderLibraries {
        root: canonical,
        _libraries: libraries,
    }))
}

#[cfg(not(all(
    target_os = "windows",
    target_arch = "x86_64",
    target_env = "msvc",
    feature = "cuda-provider"
)))]
fn prepare_cuda_root(
    _root: &Path,
    _runtime_path: &Path,
    operation: &OperationContext,
) -> Result<ProviderResources, ProviderPreparationFault> {
    checkpoint(operation)?;
    Err(if target_supports(OnnxExecutionProvider::Cuda) {
        OnnxProviderFallbackReason::BuildCapabilityUnavailable.into()
    } else {
        OnnxProviderFallbackReason::UnsupportedTarget.into()
    })
}

fn checkpoint(operation: &OperationContext) -> Result<(), ProviderPreparationFault> {
    operation.interruption().map_or(Ok(()), |interruption| {
        Err(ProviderPreparationFault::Terminal(interruption.into()))
    })
}

#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
    target_env = "msvc",
    feature = "cuda-provider"
))]
fn cuda_dispatch() -> Result<ExecutionProviderDispatch, OnnxProviderFallbackReason> {
    let provider = ort::ep::CUDA::default()
        .with_device_id(0)
        .with_memory_limit(CUDA_ARENA_LIMIT_BYTES);
    if !provider
        .is_available()
        .map_err(|_| OnnxProviderFallbackReason::ProviderUnavailable)?
    {
        return Err(OnnxProviderFallbackReason::ProviderUnavailable);
    }
    Ok(provider.build().error_on_failure())
}

#[cfg(not(all(
    target_os = "windows",
    target_arch = "x86_64",
    target_env = "msvc",
    feature = "cuda-provider"
)))]
fn cuda_dispatch() -> Result<ExecutionProviderDispatch, OnnxProviderFallbackReason> {
    Err(if target_supports(OnnxExecutionProvider::Cuda) {
        OnnxProviderFallbackReason::BuildCapabilityUnavailable
    } else {
        OnnxProviderFallbackReason::UnsupportedTarget
    })
}

#[cfg(all(
    target_os = "macos",
    target_arch = "aarch64",
    feature = "coreml-provider"
))]
fn coreml_dispatch() -> Result<ExecutionProviderDispatch, OnnxProviderFallbackReason> {
    let provider = ort::ep::CoreML::default()
        .with_compute_units(ort::ep::coreml::ComputeUnits::All)
        .with_subgraphs(false);
    if !provider
        .is_available()
        .map_err(|_| OnnxProviderFallbackReason::ProviderUnavailable)?
    {
        return Err(OnnxProviderFallbackReason::ProviderUnavailable);
    }
    Ok(provider.build().error_on_failure())
}

#[cfg(not(all(
    target_os = "macos",
    target_arch = "aarch64",
    feature = "coreml-provider"
)))]
fn coreml_dispatch() -> Result<ExecutionProviderDispatch, OnnxProviderFallbackReason> {
    Err(if target_supports(OnnxExecutionProvider::CoreMl) {
        OnnxProviderFallbackReason::BuildCapabilityUnavailable
    } else {
        OnnxProviderFallbackReason::UnsupportedTarget
    })
}

#[cfg(test)]
mod tests {
    use mado_pilot_core::{CancellationToken, OperationContext};

    use super::{
        CUDA_RECOGNIZER_OUTPUT_BYTES, ProviderPlan, ProviderPreparationFault, build_supports,
        recognizer_output_budget, release_supports, target_supports,
    };
    use crate::{
        OnnxBackendFault, OnnxExecutionProvider, OnnxExecutionProviderPolicy,
        OnnxProviderFallbackReason,
    };

    #[test]
    fn cpu_policy_is_available_in_every_build() {
        let plan = ProviderPlan::resolve(OnnxExecutionProviderPolicy::Cpu).unwrap();
        assert_eq!(plan.requested(), OnnxExecutionProviderPolicy::Cpu);
        assert_eq!(plan.candidate(), OnnxExecutionProvider::Cpu);
        assert!(target_supports(OnnxExecutionProvider::Cpu));
        assert!(build_supports(OnnxExecutionProvider::Cpu));
    }

    #[test]
    fn recognizer_output_budget_is_narrower_only_for_cuda() {
        assert_eq!(
            recognizer_output_budget(OnnxExecutionProvider::Cuda),
            CUDA_RECOGNIZER_OUTPUT_BYTES
        );
        assert_eq!(
            recognizer_output_budget(OnnxExecutionProvider::Cpu),
            crate::MAX_OUTPUT_BYTES
        );
        assert_eq!(
            recognizer_output_budget(OnnxExecutionProvider::CoreMl),
            crate::MAX_OUTPUT_BYTES
        );
    }

    #[test]
    fn unavailable_target_or_feature_is_a_closed_reason() {
        for (policy, provider) in [
            (
                OnnxExecutionProviderPolicy::PreferCuda,
                OnnxExecutionProvider::Cuda,
            ),
            (
                OnnxExecutionProviderPolicy::PreferCoreMl,
                OnnxExecutionProvider::CoreMl,
            ),
        ] {
            match ProviderPlan::resolve(policy) {
                Ok(plan) => assert_eq!(plan.candidate(), provider),
                Err(reason) => assert!(matches!(
                    reason,
                    OnnxProviderFallbackReason::UnsupportedTarget
                        | OnnxProviderFallbackReason::BuildCapabilityUnavailable
                        | OnnxProviderFallbackReason::QualificationRejected
                )),
            }
        }
    }

    #[test]
    fn automatic_policy_selects_only_a_release_qualified_provider() {
        match ProviderPlan::resolve(OnnxExecutionProviderPolicy::AutoPreferAccelerator) {
            Ok(plan) => assert!(release_supports(plan.candidate())),
            Err(reason) => assert!(matches!(
                reason,
                OnnxProviderFallbackReason::UnsupportedTarget
                    | OnnxProviderFallbackReason::BuildCapabilityUnavailable
            )),
        }
    }

    #[cfg(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "windows", target_arch = "x86_64", target_env = "msvc")
    ))]
    #[test]
    fn automatic_cpu_selection_ignores_an_unused_cuda_root() {
        let plan = super::prepare(
            OnnxExecutionProviderPolicy::AutoPreferAccelerator,
            Some(std::path::Path::new("/unused-provider-root")),
            std::path::Path::new("/unused-runtime"),
            &OperationContext::new(),
        )
        .expect("automatic policy selects release-qualified CPU without consulting CUDA paths");
        assert_eq!(plan.candidate(), OnnxExecutionProvider::Cpu);
    }

    #[test]
    fn preparation_observes_terminal_cancellation_before_provider_work() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let operation = OperationContext::new().with_cancellation(cancellation);
        assert!(matches!(
            super::prepare(
                OnnxExecutionProviderPolicy::Cpu,
                None,
                std::path::Path::new("/unused-runtime"),
                &operation,
            ),
            Err(ProviderPreparationFault::Terminal(
                OnnxBackendFault::Cancelled
            ))
        ));
    }

    #[test]
    fn coreml_is_rejected_after_target_qualification() {
        if target_supports(OnnxExecutionProvider::CoreMl)
            && build_supports(OnnxExecutionProvider::CoreMl)
        {
            assert_eq!(
                ProviderPlan::resolve(OnnxExecutionProviderPolicy::RequireCoreMl),
                Err(OnnxProviderFallbackReason::QualificationRejected)
            );
        }
    }
}
