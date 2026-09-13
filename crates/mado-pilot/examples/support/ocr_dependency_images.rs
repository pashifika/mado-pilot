//! Opt-in, private Windows executable-image snapshots for OCR evidence drivers.
//!
//! This observes currently loaded images, including the executable and system DLLs;
//! it neither approves dependencies nor traces images unloaded before observation.
//! The bounded snapshot/path checks reject observed loader races without retrying.
//! Module handles are borrowed identifiers: never close, wrap as owned, or unload them.
//!
//! The explicit report path must have an existing private parent directory. The
//! Python evidence runners create one with Python 3.13+ `mkdir(mode=0o700)`,
//! including its Windows private ACL semantics. Standalone opt-in callers must
//! provide the same protection; this recorder does not inspect or change ACLs.

use std::fmt;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::mem::{size_of, size_of_val};
use std::os::windows::fs::OpenOptionsExt;
use std::path::Path;

use windows::Win32::Foundation::{HANDLE, HMODULE};
use windows::Win32::System::ProcessStatus::{EnumProcessModules, GetModuleFileNameExW};
use windows::Win32::System::Threading::GetCurrentProcess;

#[path = "ocr_image_path.rs"]
pub(crate) mod ocr_image_path;

const REPORT_ENV: &str = "MADO_PILOT_OCR_DEPENDENCY_REPORT";
const MODULE_LIMIT: usize = 256;
const PATH_UNITS: usize = 32_768;
const OUTPUT_LIMIT: usize = 1_048_576;

/// Path-free failure details safe to include in ordinary driver diagnostics.
#[derive(Debug)]
pub(crate) enum Failure {
    Rule(&'static str),
    Windows(&'static str, i32),
    Io(&'static str, io::ErrorKind),
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rule(rule) => write!(formatter, "rule={rule}"),
            Self::Windows(operation, code) => {
                write!(formatter, "operation={operation} hresult={code}")
            }
            Self::Io(operation, kind) => write!(formatter, "operation={operation} kind={kind:?}"),
        }
    }
}

impl std::error::Error for Failure {}

/// An owned, absolute, line-safe image path and an opt-in diagnostic base address.
pub(crate) struct Image {
    #[cfg(feature = "ocr-loader-diagnostic")]
    pub(crate) base: u64,
    pub(crate) path: String,
}

/// Writes one exclusive report only when explicitly requested by the process runner.
///
/// The caller must supply an existing private parent directory (see module docs).
///
/// An absent environment variable is a silent no-op, not dependency evidence. An
/// empty or partial report is retained on failure; only successful child exit plus
/// the runner's independent identity checks can make a report usable as evidence.
pub(crate) fn record_if_requested() -> Result<(), Failure> {
    let Some(destination) = std::env::var_os(REPORT_ENV) else {
        return Ok(());
    };
    let destination = destination
        .into_string()
        .map_err(|_| Failure::Rule("report-path-encoding"))?;
    if !Path::new(&destination).is_absolute()
        || destination.contains(['\0', '\r', '\n'])
        || destination.encode_utf16().take(PATH_UNITS).count() >= PATH_UNITS
    {
        return Err(Failure::Rule("report-path"));
    }

    // The caller supplies the private parent. Deny sharing while writing, never
    // overwrite an earlier report, and include file-setup-loaded DLLs below.
    let mut report = OpenOptions::new()
        .write(true)
        .share_mode(0)
        .create_new(true)
        .open(&destination)
        .map_err(|error| Failure::Io("report-create", error.kind()))?;
    if !report
        .metadata()
        .map_err(|error| Failure::Io("report-metadata", error.kind()))?
        .is_file()
    {
        return Err(Failure::Rule("report-regular-file"));
    }

    let images = stable_snapshot::<PATH_UNITS>()?;

    for image in images {
        report
            .write_all(image.path.as_bytes())
            .map_err(|error| Failure::Io("report-write", error.kind()))?;
        report
            .write_all(b"\n")
            .map_err(|error| Failure::Io("report-write", error.kind()))?;
    }
    report
        .sync_all()
        .map_err(|error| Failure::Io("report-sync", error.kind()))
}

/// Observes a stable, base-ordered inventory with owned paths and borrowed handles.
///
/// The scratch capacity includes the terminator and an ambiguous last slot, so a
/// capacity of 2,050 accepts at most 2,048 UTF-16 content units.
pub(crate) fn stable_snapshot<const PATH_CAPACITY: usize>() -> Result<Vec<Image>, Failure> {
    if !(3..=PATH_UNITS).contains(&PATH_CAPACITY) {
        return Err(Failure::Rule("module-path-capacity"));
    }

    // SAFETY: GetCurrentProcess has no preconditions. Its pseudo-handle is valid
    // for this process and borrowed; it must not be closed or made an owned handle.
    let process = unsafe { GetCurrentProcess() };
    let mut modules = [HMODULE::default(); MODULE_LIMIT];
    let count = snapshot(process, &mut modules)?;
    let modules = &modules[..count];
    let mut path_buffer = [0_u16; PATH_CAPACITY];
    let mut images: Vec<Image> = Vec::with_capacity(count);
    let mut output_bytes = 0_usize;
    for &module in modules {
        let path = module_path(process, module, &mut path_buffer)?;
        let path = ocr_image_path::validated_path(path).map_err(Failure::Rule)?;
        if images.iter().any(|previous| previous.path == path) {
            return Err(Failure::Rule("module-path-duplicate"));
        }
        // Preserve the legacy early line-output rejection before path revalidation.
        // A diagnostic JSON checkpoint cannot fit if its unescaped paths cannot.
        output_bytes = output_bytes
            .checked_add(path.len())
            .and_then(|bytes| bytes.checked_add(1))
            .filter(|bytes| *bytes <= OUTPUT_LIMIT)
            .ok_or(Failure::Rule("report-output-limit"))?;
        images.push(Image {
            #[cfg(feature = "ocr-loader-diagnostic")]
            base: module.0.addr() as u64,
            path,
        });
    }

    let mut verification = [HMODULE::default(); MODULE_LIMIT];
    let verified_count = snapshot(process, &mut verification)?;
    if modules != &verification[..verified_count] {
        return Err(Failure::Rule("module-inventory-changed"));
    }
    // A reused base address can preserve the handle set while changing its path.
    // Reuse the UTF-16 buffer to compare paths without another owned-string copy.
    for (&module, image) in modules.iter().zip(&images) {
        let current = module_path(process, module, &mut path_buffer)?;
        ocr_image_path::validate_line_protocol(current).map_err(Failure::Rule)?;
        if !image.path.encode_utf16().eq(current.iter().copied()) {
            return Err(Failure::Rule("module-path-changed"));
        }
    }
    let verified_count = snapshot(process, &mut verification)?;
    if modules != &verification[..verified_count] {
        return Err(Failure::Rule("module-inventory-changed"));
    }
    Ok(images)
}

fn snapshot(process: HANDLE, modules: &mut [HMODULE; MODULE_LIMIT]) -> Result<usize, Failure> {
    let capacity =
        u32::try_from(size_of_val(modules)).map_err(|_| Failure::Rule("module-capacity"))?;
    let mut needed = 0_u32;
    // SAFETY: process is the current-process pseudo-handle. The initialized,
    // aligned array is exclusively borrowed and capacity is exactly its byte
    // size; needed is a live u32 out-parameter. The API retains neither pointer.
    // Returned HMODULEs are non-owning snapshot values, never FreeLibrary inputs.
    unsafe { EnumProcessModules(process, modules.as_mut_ptr(), capacity, &mut needed) }
        .map_err(|error| Failure::Windows("module-enumerate", error.code().0))?;
    if needed > capacity {
        return Err(Failure::Rule("module-count-limit"));
    }
    let needed = usize::try_from(needed).map_err(|_| Failure::Rule("module-count"))?;
    if needed == 0 || !needed.is_multiple_of(size_of::<HMODULE>()) {
        return Err(Failure::Rule("module-count"));
    }
    let count = needed / size_of::<HMODULE>();
    let modules = &mut modules[..count];
    if modules.iter().any(|module| module.is_invalid()) {
        return Err(Failure::Rule("module-handle-null"));
    }
    modules.sort_unstable_by_key(|module| module.0.addr());
    if modules.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(Failure::Rule("module-handle-duplicate"));
    }
    Ok(count)
}

fn module_path(process: HANDLE, module: HMODULE, buffer: &mut [u16]) -> Result<&[u16], Failure> {
    // A sentinel makes a missing terminator observable even on buffer reuse.
    buffer.fill(u16::MAX);
    // SAFETY: Both handles refer to the current process; module is a non-null,
    // borrowed snapshot identifier, never dereferenced or unloaded by Rust. The
    // API handles loader invalidation as failure. buffer is an exclusive live
    // UTF-16 output slice whose length stable_snapshot bounds; no pointer is retained.
    let length = unsafe { GetModuleFileNameExW(Some(process), Some(module), buffer) };
    if length == 0 {
        return Err(Failure::Windows(
            "module-path",
            windows::core::Error::from_thread().code().0,
        ));
    }
    let length = usize::try_from(length).map_err(|_| Failure::Rule("module-path-length"))?;
    // Reject the ambiguous last-slot boundary too: never accept a truncated path,
    // whether an SDK version reports the terminator or just the copied characters.
    if length >= buffer.len() - 1 || buffer[length] != 0 {
        return Err(Failure::Rule("module-path-limit"));
    }
    Ok(&buffer[..length])
}
