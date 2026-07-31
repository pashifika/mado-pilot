//! Dynamically resolved Windows APIs whose exports are newer than the loader
//! floor shared by candidate deployment versions.

use std::ffi::c_void;
use std::mem;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::{ffi::OsString, path::Path};

use windows::Win32::Foundation::{FreeLibrary, HMODULE, HWND, POINT};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows::Win32::System::WinRT::RO_INIT_TYPE;
use windows::core::{
    BOOL, Error, GUID, HRESULT, HSTRING, IInspectable, Interface, PCSTR, PCWSTR, Type,
};

type GetDpiForWindowFn = unsafe extern "system" fn(HWND) -> u32;
type GetDpiForMonitorFn = unsafe extern "system" fn(HMONITOR, i32, *mut u32, *mut u32) -> HRESULT;
type GetScaleFactorForMonitorFn = unsafe extern "system" fn(HMONITOR, *mut i32) -> HRESULT;
type LogicalToPhysicalPointForPerMonitorDpiFn = unsafe extern "system" fn(HWND, *mut POINT) -> BOOL;
type CoIncrementMtaUsageFn = unsafe extern "system" fn(*mut *mut c_void) -> HRESULT;
type RoGetActivationFactoryFn =
    unsafe extern "system" fn(HSTRING, *const GUID, *mut *mut c_void) -> HRESULT;
type RoInitializeFn = unsafe extern "system" fn(RO_INIT_TYPE) -> HRESULT;
type RoUninitializeFn = unsafe extern "system" fn();
type CreateDirect3D11DeviceFromDxgiDeviceFn =
    unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT;

const MDT_EFFECTIVE_DPI: i32 = 0;

pub(crate) fn geometry_api_available() -> bool {
    logical_to_physical_fn().is_some()
        && (scale_factor_for_monitor_fn().is_some() || dpi_for_monitor_fn().is_some())
}

pub(crate) fn winrt_loader_available() -> bool {
    co_increment_mta_usage_fn().is_some()
        && ro_get_activation_factory_fn().is_some()
        && ro_initialize_fn().is_some()
        && ro_uninitialize_fn().is_some()
        && create_direct3d_device_fn().is_some()
}

pub(crate) fn initialize_winrt(init: RO_INIT_TYPE) -> Option<HRESULT> {
    let function = ro_initialize_fn()?;
    // SAFETY: init is a valid WinRT apartment model.
    Some(unsafe { function(init) })
}

pub(crate) fn uninitialize_winrt() {
    if let Some(function) = ro_uninitialize_fn() {
        // SAFETY: the caller pairs this with a successful initialize_winrt call
        // on the same thread.
        unsafe { function() };
    }
}

pub(crate) fn increment_mta_usage() -> bool {
    let Some(function) = co_increment_mta_usage_fn() else {
        return false;
    };
    let mut cookie = std::ptr::null_mut();
    // SAFETY: cookie is a writable output. The returned opaque usage reference
    // is intentionally never decremented, so the process retains it until
    // teardown.
    unsafe { function(&raw mut cookie) }.is_ok()
}

pub(crate) fn activation_factory<T: Interface>(runtime_name: &str) -> windows::core::Result<T> {
    let function = ro_get_activation_factory_fn().ok_or_else(Error::from_thread)?;
    let name = HSTRING::from(runtime_name);
    let mut factory = std::ptr::null_mut();
    // SAFETY: T supplies its IID and factory is writable for one COM interface
    // pointer. HSTRING is passed by its ABI-transparent value representation.
    unsafe { function(mem::transmute_copy(&name), &T::IID, &raw mut factory) }.ok()?;
    // SAFETY: a successful RoGetActivationFactory initialized factory with one
    // owned reference for the requested interface.
    unsafe { Type::from_abi(factory) }
}

pub(crate) fn create_direct3d_device(
    dxgi_device: &IDXGIDevice,
) -> windows::core::Result<IInspectable> {
    let function = create_direct3d_device_fn().ok_or_else(Error::from_thread)?;
    let mut inspectable = std::ptr::null_mut();
    // SAFETY: dxgi_device is a live COM interface and inspectable receives one
    // owned WinRT interface reference on success.
    unsafe { function(Interface::as_raw(dxgi_device), &raw mut inspectable) }.ok()?;
    // SAFETY: the successful native call returned one owned IInspectable.
    unsafe { Type::from_abi(inspectable) }
}

pub(crate) fn window_dpi(hwnd: HWND) -> Option<u32> {
    let function = dpi_for_window_fn()?;
    // SAFETY: hwnd is an opaque window handle from current enumeration.
    let dpi = unsafe { function(hwnd) };
    (dpi > 0).then_some(dpi)
}

pub(crate) fn logical_to_physical(hwnd: HWND, point: &mut POINT) -> bool {
    let Some(function) = logical_to_physical_fn() else {
        return false;
    };
    // SAFETY: point is writable and hwnd is an opaque current window handle.
    unsafe { function(hwnd, point).as_bool() }
}

pub(crate) fn monitor_scale(monitor: HMONITOR) -> Option<f64> {
    if let Some(function) = scale_factor_for_monitor_fn() {
        let mut percent = 0;
        // SAFETY: percent is writable and monitor came from current enumeration.
        if unsafe { function(monitor, &raw mut percent) }.is_ok() && percent > 0 {
            return Some(f64::from(percent) / 100.0);
        }
    }

    let function = dpi_for_monitor_fn()?;
    let mut dpi_x = 0;
    let mut dpi_y = 0;
    // SAFETY: both outputs are writable and monitor is a current handle.
    unsafe { function(monitor, MDT_EFFECTIVE_DPI, &raw mut dpi_x, &raw mut dpi_y) }
        .ok()
        .ok()?;
    (dpi_x > 0 && dpi_y > 0).then_some((f64::from(dpi_x) + f64::from(dpi_y)) / 192.0)
}

fn dpi_for_window_fn() -> Option<GetDpiForWindowFn> {
    static FUNCTION: OnceLock<Option<GetDpiForWindowFn>> = OnceLock::new();
    *FUNCTION.get_or_init(|| load_user32(b"GetDpiForWindow\0"))
}

fn logical_to_physical_fn() -> Option<LogicalToPhysicalPointForPerMonitorDpiFn> {
    static FUNCTION: OnceLock<Option<LogicalToPhysicalPointForPerMonitorDpiFn>> = OnceLock::new();
    *FUNCTION.get_or_init(|| load_user32(b"LogicalToPhysicalPointForPerMonitorDPI\0"))
}

fn dpi_for_monitor_fn() -> Option<GetDpiForMonitorFn> {
    static FUNCTION: OnceLock<Option<GetDpiForMonitorFn>> = OnceLock::new();
    *FUNCTION.get_or_init(|| load_shcore(b"GetDpiForMonitor\0"))
}

fn scale_factor_for_monitor_fn() -> Option<GetScaleFactorForMonitorFn> {
    static FUNCTION: OnceLock<Option<GetScaleFactorForMonitorFn>> = OnceLock::new();
    *FUNCTION.get_or_init(|| load_shcore(b"GetScaleFactorForMonitor\0"))
}

fn co_increment_mta_usage_fn() -> Option<CoIncrementMtaUsageFn> {
    static FUNCTION: OnceLock<Option<CoIncrementMtaUsageFn>> = OnceLock::new();
    *FUNCTION.get_or_init(|| load_combase(b"CoIncrementMTAUsage\0"))
}

fn ro_get_activation_factory_fn() -> Option<RoGetActivationFactoryFn> {
    static FUNCTION: OnceLock<Option<RoGetActivationFactoryFn>> = OnceLock::new();
    *FUNCTION.get_or_init(|| load_combase(b"RoGetActivationFactory\0"))
}

fn ro_initialize_fn() -> Option<RoInitializeFn> {
    static FUNCTION: OnceLock<Option<RoInitializeFn>> = OnceLock::new();
    *FUNCTION.get_or_init(|| load_combase(b"RoInitialize\0"))
}

fn ro_uninitialize_fn() -> Option<RoUninitializeFn> {
    static FUNCTION: OnceLock<Option<RoUninitializeFn>> = OnceLock::new();
    *FUNCTION.get_or_init(|| load_combase(b"RoUninitialize\0"))
}

fn create_direct3d_device_fn() -> Option<CreateDirect3D11DeviceFromDxgiDeviceFn> {
    static FUNCTION: OnceLock<Option<CreateDirect3D11DeviceFromDxgiDeviceFn>> = OnceLock::new();
    *FUNCTION.get_or_init(|| load_d3d11(b"CreateDirect3D11DeviceFromDXGIDevice\0"))
}

fn load_user32<T: Copy>(name: &'static [u8]) -> Option<T> {
    // SAFETY: this process-lifetime module reference is intentionally retained;
    // every requested symbol has a signature checked at its typed call site.
    let module = load_system_module("user32.dll")?;
    load_symbol(module, name)
}

fn load_shcore<T: Copy>(name: &'static [u8]) -> Option<T> {
    // SAFETY: see load_user32. Retaining the module keeps cached function
    // pointers valid for the process lifetime.
    let module = load_system_module("shcore.dll")?;
    load_symbol(module, name)
}

fn load_combase<T: Copy>(name: &'static [u8]) -> Option<T> {
    // SAFETY: see load_user32.
    let module = load_system_module("combase.dll")?;
    load_symbol(module, name)
}

fn load_d3d11<T: Copy>(name: &'static [u8]) -> Option<T> {
    // SAFETY: see load_user32.
    let module = load_system_module("d3d11.dll")?;
    load_symbol(module, name)
}

fn load_system_module(name: &str) -> Option<HMODULE> {
    let path = system_directory()?.join(name);
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: wide is a NUL-terminated absolute path retained for the call. The
    // search flag also confines this system module's dependent DLL resolution.
    let module =
        unsafe { LoadLibraryExW(PCWSTR(wide.as_ptr()), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }
            .ok()?;
    if module_is_in_system_directory(module) {
        Some(module)
    } else {
        // SAFETY: module is the owned reference returned by LoadLibraryExW.
        let _released = unsafe { FreeLibrary(module) };
        None
    }
}

fn module_is_in_system_directory(module: HMODULE) -> bool {
    let Some(module_path) = module_path(module) else {
        return false;
    };
    let Some(system_directory) = system_directory() else {
        return false;
    };
    module_path
        .parent()
        .is_some_and(|parent| paths_equal_ignore_ascii_case(parent, system_directory))
}

fn module_path(module: HMODULE) -> Option<PathBuf> {
    let mut buffer = vec![0u16; 260];
    loop {
        // SAFETY: buffer is writable and module is a live loader handle.
        let written = unsafe { GetModuleFileNameW(Some(module), &mut buffer) };
        let written = usize::try_from(written).ok()?;
        if written == 0 {
            return None;
        }
        if written < buffer.len() {
            return Some(PathBuf::from(OsString::from_wide(&buffer[..written])));
        }
        buffer.resize(buffer.len().checked_mul(2)?, 0);
    }
}

fn system_directory() -> Option<&'static PathBuf> {
    static DIRECTORY: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIRECTORY.get_or_init(read_system_directory).as_ref()
}

fn read_system_directory() -> Option<PathBuf> {
    let mut buffer = vec![0u16; 260];
    loop {
        // SAFETY: buffer is writable for its complete length.
        let written = unsafe { GetSystemDirectoryW(Some(&mut buffer)) };
        let written = usize::try_from(written).ok()?;
        if written == 0 {
            return None;
        }
        if written < buffer.len() {
            return Some(PathBuf::from(OsString::from_wide(&buffer[..written])));
        }
        buffer.resize(written.checked_add(1)?, 0);
    }
}

fn paths_equal_ignore_ascii_case(left: &Path, right: &Path) -> bool {
    left.as_os_str()
        .to_string_lossy()
        .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
}

fn load_symbol<T: Copy>(module: HMODULE, name: &'static [u8]) -> Option<T> {
    // SAFETY: name is NUL-terminated and module is a successfully loaded system
    // DLL. The typed wrappers above supply the matching ABI signature.
    let function = unsafe { GetProcAddress(module, PCSTR(name.as_ptr())) }?;
    // SAFETY: FARPROC and the requested system function pointer have the same
    // pointer representation on supported 64-bit Windows targets.
    Some(unsafe { mem::transmute_copy(&function) })
}

#[cfg(test)]
mod tests {
    use super::{load_system_module, module_is_in_system_directory};

    #[test]
    fn optional_modules_resolve_only_from_the_system_directory() {
        for name in ["user32.dll", "shcore.dll", "combase.dll", "d3d11.dll"] {
            let module = load_system_module(name).expect("system module");
            assert!(module_is_in_system_directory(module));
        }
    }
}
