//! Controlled process-global ONNX Runtime loading.

use std::ffi::{CStr, OsStr};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use libloading::{Library, Symbol};

use crate::fault::OnnxBackendFault;

const RUNTIME_VERSION: &str = "1.29.0";
const ENVIRONMENT_NAME: &str = "mado-pilot-onnx-cpu";

#[cfg(target_os = "macos")]
const RUNTIME_FILENAME: &str = "libonnxruntime.1.29.0.dylib";
#[cfg(target_os = "windows")]
const RUNTIME_FILENAME: &str = "onnxruntime.dll";

struct RuntimeRecord {
    canonical_path: PathBuf,
    usable: bool,
    _library: Library,
}

impl std::fmt::Debug for RuntimeRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeRecord")
            .field("usable", &self.usable)
            .finish_non_exhaustive()
    }
}

static RUNTIME: Mutex<Option<RuntimeRecord>> = Mutex::new(None);

/// Loads and initializes the one reviewed runtime boundary.
pub(crate) fn initialize(path: &Path) -> Result<(), OnnxBackendFault> {
    let canonical = validate_path(path)?;
    let mut state = RUNTIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if let Some(record) = state.as_ref() {
        return if record.usable && record.canonical_path == canonical {
            Ok(())
        } else {
            Err(OnnxBackendFault::RuntimeConflict)
        };
    }

    let library = load_library(&canonical)?;
    let api = inspect_api(&library)?;
    if !ort::set_api(api) {
        return Err(OnnxBackendFault::RuntimeConflict);
    }

    // From this point the global API table contains function pointers into this
    // library. Store it before touching any other process-global state so every
    // return path keeps those pointers valid until process exit.
    *state = Some(RuntimeRecord {
        canonical_path: canonical,
        usable: false,
        _library: library,
    });

    if !ort::init()
        .with_name(ENVIRONMENT_NAME)
        .with_telemetry(false)
        .commit()
    {
        return Err(OnnxBackendFault::RuntimeConflict);
    }

    if let Some(record) = state.as_mut() {
        record.usable = true;
    }
    Ok(())
}

fn validate_path(path: &Path) -> Result<PathBuf, OnnxBackendFault> {
    if !path.is_absolute() {
        return Err(OnnxBackendFault::InvalidRuntimePath);
    }
    let canonical =
        std::fs::canonicalize(path).map_err(|_| OnnxBackendFault::RuntimeUnavailable)?;
    if canonical != path || canonical.file_name() != Some(OsStr::new(runtime_filename())) {
        return Err(OnnxBackendFault::InvalidRuntimePath);
    }
    let metadata = canonical
        .metadata()
        .map_err(|_| OnnxBackendFault::RuntimeUnavailable)?;
    if !metadata.is_file() {
        return Err(OnnxBackendFault::InvalidRuntimePath);
    }
    Ok(canonical)
}

const fn runtime_filename() -> &'static str {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        RUNTIME_FILENAME
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        "unsupported-onnx-runtime"
    }
}

#[cfg(target_os = "macos")]
fn load_library(path: &Path) -> Result<Library, OnnxBackendFault> {
    use libloading::os::unix::{Library as UnixLibrary, RTLD_LOCAL, RTLD_NOW};

    // SAFETY: `path` is an absolute canonical regular file with the reviewed
    // target filename. RTLD_NOW resolves every dependency before publication;
    // RTLD_LOCAL prevents this private runtime from becoming ambient state.
    let library = unsafe { UnixLibrary::open(Some(path), RTLD_NOW | RTLD_LOCAL) }
        .map_err(|_| OnnxBackendFault::RuntimeUnavailable)?;
    Ok(library.into())
}

#[cfg(target_os = "windows")]
fn load_library(path: &Path) -> Result<Library, OnnxBackendFault> {
    use libloading::os::windows::{
        LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32, Library as WindowsLibrary,
    };

    // SAFETY: `path` is an absolute canonical regular file. Dependency search is
    // limited to its directory and System32, excluding the current directory,
    // PATH, and other ambient locations.
    let library = unsafe {
        WindowsLibrary::load_with_flags(
            path,
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    }
    .map_err(|_| OnnxBackendFault::RuntimeUnavailable)?;
    Ok(library.into())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn load_library(_path: &Path) -> Result<Library, OnnxBackendFault> {
    Err(OnnxBackendFault::RuntimeUnavailable)
}

fn inspect_api(library: &Library) -> Result<ort::sys::OrtApi, OnnxBackendFault> {
    type GetApiBase = unsafe extern "system" fn() -> *const ort::sys::OrtApiBase;

    // SAFETY: the library is retained by the process-global record after the API
    // is published. The symbol type is the ABI declaration from onnxruntime_c_api.h.
    let getter: Symbol<'_, GetApiBase> = unsafe { library.get(b"OrtGetApiBase\0") }
        .map_err(|_| OnnxBackendFault::RuntimeIncompatible)?;
    // SAFETY: calling this exported function has no arguments and returns the
    // immutable process-global API base owned by the loaded library.
    let base = unsafe { getter() };
    // SAFETY: the exported getter returned this pointer and null is rejected.
    let base = unsafe { base.as_ref() }.ok_or(OnnxBackendFault::RuntimeIncompatible)?;

    // SAFETY: both function pointers are required fields in a non-null API base.
    let version_ptr = unsafe { (base.GetVersionString)() };
    if version_ptr.is_null() {
        return Err(OnnxBackendFault::RuntimeIncompatible);
    }
    // SAFETY: ONNX Runtime owns a process-long NUL-terminated version string.
    let version = unsafe { CStr::from_ptr(version_ptr) }
        .to_str()
        .map_err(|_| OnnxBackendFault::RuntimeIncompatible)?;
    validate_version(version)?;

    // API 17 is compiled into `ort` by the exact `api-17` feature selection.
    if ort::sys::ORT_API_VERSION != 17 {
        return Err(OnnxBackendFault::RuntimeIncompatible);
    }
    // SAFETY: GetApi accepts the declared version and returns an immutable table.
    let api_ptr = unsafe { (base.GetApi)(ort::sys::ORT_API_VERSION) };
    // SAFETY: GetApi returned this pointer for the exact supported version and
    // null is rejected before the table is cloned.
    let api = unsafe { api_ptr.as_ref() }.ok_or(OnnxBackendFault::RuntimeIncompatible)?;
    Ok(api.clone())
}

fn validate_version(version: &str) -> Result<(), OnnxBackendFault> {
    if version == RUNTIME_VERSION {
        Ok(())
    } else {
        Err(OnnxBackendFault::RuntimeIncompatible)
    }
}

#[cfg(test)]
mod tests {
    use super::{validate_path, validate_version};
    use crate::fault::OnnxBackendFault;

    #[test]
    fn relative_runtime_paths_are_rejected_without_searching() {
        assert_eq!(
            validate_path(std::path::Path::new("libonnxruntime.dylib")),
            Err(OnnxBackendFault::InvalidRuntimePath)
        );
    }

    #[test]
    fn any_unreviewed_runtime_version_is_incompatible() {
        assert_eq!(
            validate_version("1.28.0"),
            Err(OnnxBackendFault::RuntimeIncompatible)
        );
        assert_eq!(validate_version("1.29.0"), Ok(()));
    }

    #[test]
    fn missing_controlled_file_is_unavailable_without_fallback() {
        let missing = std::env::temp_dir().join(format!(
            "mado-pilot-onnx-definitely-missing-{}",
            std::process::id()
        ));
        assert_eq!(
            validate_path(&missing),
            Err(OnnxBackendFault::RuntimeUnavailable)
        );
    }
}
