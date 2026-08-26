use std::path::Path;

#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
    feature = "cuda-provider"
))]
use std::path::PathBuf;
#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
    feature = "cuda-provider"
))]
use std::sync::Mutex;

#[cfg(any(
    all(
        target_os = "windows",
        target_arch = "x86_64",
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

#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
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

pub(crate) fn prepare(
    requested: OnnxExecutionProviderPolicy,
    provider_root: Option<&Path>,
    runtime_path: &Path,
) -> Result<ProviderPlan, OnnxProviderFallbackReason> {
    let plan = ProviderPlan::resolve(requested)?;
    match plan.candidate() {
        OnnxExecutionProvider::Cpu => {
            if provider_root.is_some()
                && !matches!(
                    plan.requested(),
                    OnnxExecutionProviderPolicy::AutoPreferAccelerator
                )
            {
                Err(OnnxProviderFallbackReason::DependencyUnavailable)
            } else {
                Ok(plan)
            }
        }
        OnnxExecutionProvider::CoreMl => {
            if provider_root.is_some() {
                Err(OnnxProviderFallbackReason::DependencyUnavailable)
            } else {
                Ok(plan)
            }
        }
        OnnxExecutionProvider::Cuda => {
            prepare_cuda_root(
                provider_root.ok_or(OnnxProviderFallbackReason::DependencyUnavailable)?,
                runtime_path,
            )?;
            Ok(plan)
        }
    }
}

pub(crate) fn rollback(provider: OnnxExecutionProvider) {
    if matches!(provider, OnnxExecutionProvider::Cuda) {
        rollback_cuda_root();
    }
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
        OnnxExecutionProvider::Cuda => cfg!(all(target_os = "windows", target_arch = "x86_64")),
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
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        return Ok(OnnxExecutionProvider::Cpu);
    }
    #[allow(unreachable_code)]
    Err(OnnxProviderFallbackReason::UnsupportedTarget)
}

#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
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
    feature = "cuda-provider"
))]
const CUDA_PROVIDER_LIBRARY: &str = "onnxruntime_providers_cuda.dll";

#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
    feature = "cuda-provider"
))]
struct CudaProviderLibraries {
    root: PathBuf,
    _libraries: Vec<libloading::Library>,
}

#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
    feature = "cuda-provider"
))]
static CUDA_PROVIDER_LIBRARIES: Mutex<Option<CudaProviderLibraries>> = Mutex::new(None);

#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
    feature = "cuda-provider"
))]
fn rollback_cuda_root() {
    let loaded = {
        let mut state = CUDA_PROVIDER_LIBRARIES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.take()
    };
    drop(loaded);
}

#[cfg(not(all(
    target_os = "windows",
    target_arch = "x86_64",
    feature = "cuda-provider"
)))]
fn rollback_cuda_root() {}

#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
    feature = "cuda-provider"
))]
fn prepare_cuda_root(root: &Path, runtime_path: &Path) -> Result<(), OnnxProviderFallbackReason> {
    use libloading::os::windows::{
        LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32, Library as WindowsLibrary,
    };

    if !root.is_absolute() {
        return Err(OnnxProviderFallbackReason::DependencyUnavailable);
    }
    if runtime_path.parent() != Some(root) {
        return Err(OnnxProviderFallbackReason::DependencyUnavailable);
    }
    let canonical = std::fs::canonicalize(root)
        .map_err(|_| OnnxProviderFallbackReason::DependencyUnavailable)?;
    if canonical != root
        || !canonical
            .metadata()
            .map_err(|_| OnnxProviderFallbackReason::DependencyUnavailable)?
            .is_dir()
    {
        return Err(OnnxProviderFallbackReason::DependencyUnavailable);
    }

    let mut dll_count = 0_usize;
    for entry in std::fs::read_dir(&canonical)
        .map_err(|_| OnnxProviderFallbackReason::DependencyUnavailable)?
    {
        let entry = entry.map_err(|_| OnnxProviderFallbackReason::DependencyUnavailable)?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|_| OnnxProviderFallbackReason::DependencyUnavailable)?;
        let file = std::fs::canonicalize(&path)
            .map_err(|_| OnnxProviderFallbackReason::DependencyUnavailable)?;
        if file != path || !file_type.is_file() || file_type.is_symlink() {
            return Err(OnnxProviderFallbackReason::DependencyUnavailable);
        }
        let is_dll = path
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|extension| extension.eq_ignore_ascii_case("dll"));
        if is_dll {
            let name = path
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .ok_or(OnnxProviderFallbackReason::DependencyUnavailable)?;
            if name != "onnxruntime.dll" && !CUDA_PROVIDER_FILES.contains(&name) {
                return Err(OnnxProviderFallbackReason::DependencyUnavailable);
            }
            dll_count += 1;
        }
    }
    if dll_count != CUDA_PROVIDER_FILES.len() + 1 {
        return Err(OnnxProviderFallbackReason::DependencyUnavailable);
    }

    let mut state = CUDA_PROVIDER_LIBRARIES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(loaded) = state.as_ref() {
        return if loaded.root == canonical {
            Ok(())
        } else {
            Err(OnnxProviderFallbackReason::DependencyUnavailable)
        };
    }

    let mut libraries = Vec::with_capacity(CUDA_PROVIDER_FILES.len() - 1);
    for name in CUDA_PROVIDER_FILES {
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
            return Err(OnnxProviderFallbackReason::DependencyUnavailable);
        }
        if name == CUDA_PROVIDER_LIBRARY {
            // ONNX Runtime loads its provider library only after its own
            // environment exists. Eagerly running that DLL's initialization
            // routine fails even when every controlled dependency is present.
            continue;
        }
        // SAFETY: every path is an absolute canonical regular file directly in
        // the explicit provider root. Dependency search is restricted to that
        // file's directory and System32; loaded libraries remain process-owned.
        let library = unsafe {
            WindowsLibrary::load_with_flags(
                &file,
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        }
        .map_err(|_| OnnxProviderFallbackReason::DependencyUnavailable)?;
        libraries.push(library.into());
    }
    *state = Some(CudaProviderLibraries {
        root: canonical,
        _libraries: libraries,
    });
    Ok(())
}

#[cfg(not(all(
    target_os = "windows",
    target_arch = "x86_64",
    feature = "cuda-provider"
)))]
fn prepare_cuda_root(_root: &Path, _runtime_path: &Path) -> Result<(), OnnxProviderFallbackReason> {
    Err(if target_supports(OnnxExecutionProvider::Cuda) {
        OnnxProviderFallbackReason::BuildCapabilityUnavailable
    } else {
        OnnxProviderFallbackReason::UnsupportedTarget
    })
}

#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
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
    use super::{ProviderPlan, build_supports, release_supports, target_supports};
    use crate::{OnnxExecutionProvider, OnnxExecutionProviderPolicy, OnnxProviderFallbackReason};

    #[test]
    fn cpu_policy_is_available_in_every_build() {
        let plan = ProviderPlan::resolve(OnnxExecutionProviderPolicy::Cpu).unwrap();
        assert_eq!(plan.requested(), OnnxExecutionProviderPolicy::Cpu);
        assert_eq!(plan.candidate(), OnnxExecutionProvider::Cpu);
        assert!(target_supports(OnnxExecutionProvider::Cpu));
        assert!(build_supports(OnnxExecutionProvider::Cpu));
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
        all(target_os = "windows", target_arch = "x86_64")
    ))]
    #[test]
    fn automatic_cpu_selection_ignores_an_unused_cuda_root() {
        let plan = super::prepare(
            OnnxExecutionProviderPolicy::AutoPreferAccelerator,
            Some(std::path::Path::new("/unused-provider-root")),
            std::path::Path::new("/unused-runtime"),
        )
        .expect("automatic policy selects release-qualified CPU without consulting CUDA paths");
        assert_eq!(plan.candidate(), OnnxExecutionProvider::Cpu);
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
