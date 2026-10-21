//! Actual loaded OpenCV image paths, never a hash-only runtime-path lookup.

use std::path::PathBuf;

use mado_pilot::ContentDigest;
use serde_json::{Value, json};

pub(super) fn observe() -> Result<Value, String> {
    let mut paths: Vec<_> = loaded_paths()?
        .into_iter()
        .filter(|path| {
            path.file_name().is_some_and(|name| {
                name.to_string_lossy()
                    .to_ascii_lowercase()
                    .contains("opencv")
            })
        })
        .map(|path| path.canonicalize().map_err(|error| error.to_string()))
        .collect::<Result<_, _>>()?;
    paths.sort();
    paths.dedup();
    if !paths.iter().any(|path| {
        path.file_name().is_some_and(|name| {
            let name = name.to_string_lossy().to_ascii_lowercase();
            name.contains("opencv_core") || name.contains("opencv_world")
        })
    }) {
        return Err("loaded OpenCV core/world image path unavailable".into());
    }
    let mut images = Vec::with_capacity(paths.len());
    for path in paths {
        let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
        images.push(json!({"path": path, "sha256": ContentDigest::of(&bytes).to_string()}));
    }
    Ok(
        json!({"method": if cfg!(target_os = "macos") { "dyld-loaded-image-paths" } else { "K32EnumProcessModules-GetModuleFileNameW" }, "images": images}),
    )
}

#[cfg(target_os = "macos")]
fn loaded_paths() -> Result<Vec<PathBuf>, String> {
    use std::ffi::{CStr, c_char};

    unsafe extern "C" {
        fn _dyld_image_count() -> u32;
        fn _dyld_get_image_name(index: u32) -> *const c_char;
    }
    // SAFETY: dyld returns the current process's loaded-image count; no unloads
    // are initiated by this single-threaded post-search identity observation.
    let count = unsafe { _dyld_image_count() };
    if count > 4096 {
        return Err("loaded image enumeration exceeds diagnostic bound".into());
    }
    let mut paths = Vec::with_capacity(count as usize);
    for index in 0..count {
        // SAFETY: index is bounded by the observed count. dyld owns each returned
        // NUL-terminated image name; copy it immediately without retaining pointers.
        let pointer = unsafe { _dyld_get_image_name(index) };
        if pointer.is_null() {
            return Err("loaded image disappeared during identity observation".into());
        }
        // SAFETY: the non-null dyld image name is NUL-terminated and remains loaded.
        let name = unsafe { CStr::from_ptr(pointer) }
            .to_str()
            .map_err(|error| error.to_string())?;
        paths.push(PathBuf::from(name));
    }
    Ok(paths)
}

#[cfg(windows)]
fn loaded_paths() -> Result<Vec<PathBuf>, String> {
    use std::ffi::{OsString, c_void};
    use std::os::windows::ffi::OsStringExt;

    use std::mem::{size_of, size_of_val};

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn K32EnumProcessModules(
            process: *mut c_void,
            modules: *mut *mut c_void,
            bytes: u32,
            needed: *mut u32,
        ) -> i32;
        fn GetModuleFileNameW(module: *mut c_void, name: *mut u16, length: u32) -> u32;
    }
    let mut modules = [std::ptr::null_mut(); 4096];
    let mut needed = 0_u32;
    // SAFETY: the current-process pseudo-handle needs no close. Both output
    // buffers are writable for their exact supplied lengths; API writes needed.
    let ok = unsafe {
        K32EnumProcessModules(
            GetCurrentProcess(),
            modules.as_mut_ptr(),
            u32::try_from(size_of_val(&modules)).expect("bounded module array"),
            &mut needed,
        )
    };
    if ok == 0
        || needed as usize > size_of_val(&modules)
        || !(needed as usize).is_multiple_of(size_of::<*mut c_void>())
    {
        return Err("loaded module enumeration failed or exceeded diagnostic bound".into());
    }
    let mut paths = Vec::new();
    for &module in &modules[..needed as usize / size_of::<*mut c_void>()] {
        let mut name = [0_u16; 32768];
        // SAFETY: module was enumerated from this process, and name is writable
        // for the supplied capacity. No module unload is initiated by this probe.
        let length = unsafe {
            GetModuleFileNameW(
                module,
                name.as_mut_ptr(),
                u32::try_from(name.len()).expect("bounded module path"),
            )
        } as usize;
        if length == 0 || length >= name.len() {
            return Err("loaded module path unavailable or truncated".into());
        }
        paths.push(PathBuf::from(OsString::from_wide(&name[..length])));
    }
    Ok(paths)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn loaded_paths() -> Result<Vec<PathBuf>, String> {
    Err("actual loaded runtime identity requires a supported native target".into())
}
