//! Private, single-use, post-registration loader observation; never numerical evidence.
//!
//! Live registration, native snapshots, and the report file stay in this adapter;
//! the shared recording core contains no live API or synthetic-test helpers.
//!
//! Registration follows driver/report/API initialization. Earlier history is unobserved.
//! A load notification precedes dynamic linking: it does not prove successful DLL
//! initialization. Checkpoint ordinal brackets do not make a snapshot/history atomic.
//! The caller supplies a precreated private report directory; no ACL is inspected or
//! changed here. No module is loaded, pinned, owned, or queried from the callback.
//!
//! The callback's scalar/volatile/lock-free source must additionally pass optimized
//! codegen inspection for imported calls, memcpy, panic paths and stack probes before
//! any diagnostic execution. Source inspection is not that codegen evidence.

use std::ffi::c_void;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::path::Path;
use std::ptr::addr_of;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::core::{s, w};

use super::ocr_dependency_images;
use super::ocr_loader_recording::{
    self, NotificationData, PATH_UNITS, Report, Storage, mado_pilot_ocr_loader_notification,
};

const REPORT_ENV: &str = "MADO_PILOT_OCR_LOADER_DIAGNOSTIC_REPORT";
const OUTPUT_LIMIT: usize = 1_048_576;
const DRAIN_LIMIT: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub(crate) enum Failure {
    Rule(&'static str),
    NtStatus(&'static str, i32),
}

impl Failure {
    fn rule(&self) -> &'static str {
        match self {
            Self::Rule(rule) | Self::NtStatus(rule, _) => rule,
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rule(rule) => write!(formatter, "rule={rule}"),
            Self::NtStatus(rule, status) => {
                write!(formatter, "rule={rule} ntstatus={status}")
            }
        }
    }
}

impl std::error::Error for Failure {}

impl From<ocr_loader_recording::Failure> for Failure {
    fn from(error: ocr_loader_recording::Failure) -> Self {
        match error {
            ocr_loader_recording::Failure::Rule(rule) => Self::Rule(rule),
        }
    }
}

type Callback = unsafe extern "system" fn(u32, *const NotificationData, *mut c_void);
type Register =
    unsafe extern "system" fn(u32, Option<Callback>, *mut c_void, *mut *mut c_void) -> i32;
type Unregister = unsafe extern "system" fn(*mut c_void) -> i32;

static STORAGE: Storage = Storage::new();
static STARTED: AtomicBool = AtomicBool::new(false);

struct Api {
    register: Register,
    unregister: Unregister,
}

impl Api {
    fn resolve() -> Result<Self, Failure> {
        // SAFETY: ntdll is already resident. GetModuleHandleW adds no reference;
        // this borrowed handle is never freed, pinned, or made an owner.
        let module = unsafe { GetModuleHandleW(w!("ntdll.dll")) }
            .map_err(|_| Failure::Rule("ntdll-unavailable"))?;
        // SAFETY: Constant, terminated export names and the borrowed ntdll handle
        // satisfy GetProcAddress. No candidate/observed module is forced in.
        let register = unsafe { GetProcAddress(module, s!("LdrRegisterDllNotification")) }
            .ok_or(Failure::Rule("register-api-unavailable"))?;
        // SAFETY: Same already-loaded module and constant export-name contract.
        let unregister = unsafe { GetProcAddress(module, s!("LdrUnregisterDllNotification")) }
            .ok_or(Failure::Rule("unregister-api-unavailable"))?;
        // SAFETY: The named exports have the documented NTAPI signatures above;
        // NTSTATUS is signed i32, not HRESULT. Windows x64 ABI sizes are checked.
        Ok(unsafe {
            Self {
                register: std::mem::transmute::<unsafe extern "system" fn() -> isize, Register>(
                    register,
                ),
                unregister: std::mem::transmute::<unsafe extern "system" fn() -> isize, Unregister>(
                    unregister,
                ),
            }
        })
    }
}

pub(crate) struct Diagnostic {
    report: Report<File>,
    unregister: Option<Unregister>,
    cookie: Option<*mut c_void>,
    stopped: bool,
}

impl Diagnostic {
    pub(crate) fn begin() -> Result<Self, Failure> {
        if STARTED.swap(true, Ordering::AcqRel) {
            return Err(Failure::Rule("diagnostic-single-use"));
        }
        let destination = std::env::var_os(REPORT_ENV)
            .ok_or(Failure::Rule("trace-path-required"))?
            .into_string()
            .map_err(|_| Failure::Rule("trace-path-encoding"))?;
        if !Path::new(&destination).is_absolute()
            || destination.contains(['\0', '\r', '\n'])
            || destination.encode_utf16().take(PATH_UNITS + 1).count() > PATH_UNITS
        {
            return Err(Failure::Rule("trace-path"));
        }
        // The caller creates and protects the parent; never create it, alter its
        // permissions, overwrite a prior report, or allow sharing during writing.
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(0)
            .open(destination)
            .map_err(|_| Failure::Rule("trace-create"))?;
        if !file
            .metadata()
            .map_err(|_| Failure::Rule("trace-metadata"))?
            .is_file()
        {
            return Err(Failure::Rule("trace-regular-file"));
        }
        let mut diagnostic = Self {
            report: Report::new(file, OUTPUT_LIMIT)?,
            unregister: None,
            cookie: None,
            stopped: false,
        };
        let initialized = diagnostic
            .register()
            .and_then(|()| diagnostic.checkpoint("initial"));
        if let Err(error) = initialized {
            let _ = diagnostic.stop(Some(error.rule()));
            return Err(error);
        }
        Ok(diagnostic)
    }

    fn register(&mut self) -> Result<(), Failure> {
        let api = Api::resolve()?;
        self.unregister = Some(api.unregister);
        let mut cookie = std::ptr::null_mut();
        // SAFETY: Signature resolved from ntdll. Flags are zero; callback/context
        // remain valid for process lifetime, even if the API returns failure or
        // leaves cleanup unresolved. The cookie out-parameter is live for this call.
        let status = unsafe {
            (api.register)(
                0,
                Some(mado_pilot_ocr_loader_notification),
                addr_of!(STORAGE).cast_mut().cast::<c_void>(),
                &mut cookie,
            )
        };
        if status != 0 {
            return Err(Failure::NtStatus("register-failed", status));
        }
        if cookie.is_null() {
            return Err(Failure::Rule("register-cookie-null"));
        }
        self.cookie = Some(cookie);
        Ok(())
    }

    fn checkpoint(&mut self, name: &'static str) -> Result<(), Failure> {
        let before = STORAGE.observed();
        // The extra two slots preserve the shared helper's conservative missing-
        // terminator boundary while permitting exactly2048 content units.
        let images =
            ocr_dependency_images::stable_snapshot::<{ PATH_UNITS + 2 }>().map_err(|_| {
                Failure::Rule(match name {
                    "initial" => "initial-snapshot-failed",
                    _ => "final-snapshot-failed",
                })
            })?;
        let after = STORAGE.observed();
        self.report.checkpoint(
            name,
            before,
            after,
            images.iter().map(|image| (image.base, image.path.as_str())),
        )?;
        // Persist the initial checkpoint before workloads: a watchdog/outer kill
        // can leave a visibly incomplete JSON prefix, never a claimed complete run.
        self.report
            .output
            .inner
            .sync_all()
            .map_err(|_| Failure::Rule("trace-sync"))
    }

    pub(crate) fn finish(mut self, failure: Option<&'static str>) -> Result<(), Failure> {
        let checkpoint = self.checkpoint("final");
        let failure = failure.or_else(|| checkpoint.as_ref().err().map(Failure::rule));
        self.stop(failure)
    }

    fn stop(&mut self, failure: Option<&'static str>) -> Result<(), Failure> {
        if self.stopped {
            return Err(Failure::Rule("diagnostic-already-stopped"));
        }
        self.stopped = true;
        STORAGE.close();
        let status = match (self.unregister, self.cookie.take()) {
            (Some(unregister), Some(cookie)) => {
                // SAFETY: Exactly the cookie returned by successful registration,
                // consumed once. Success is NOT treated as callback drain.
                Some(unsafe { unregister(cookie) })
            }
            _ => None,
        };
        let drained = STORAGE.drain(DRAIN_LIMIT);
        let result = self
            .report
            .finish(&STORAGE, status, drained, failure)
            .map_err(Failure::from);
        let synced = self
            .report
            .output
            .inner
            .sync_all()
            .map_err(|_| Failure::Rule("trace-sync"));
        result.and(synced)
    }
}

impl Drop for Diagnostic {
    fn drop(&mut self) {
        if !self.stopped {
            // No final checkpoint is fabricated when unwinding before the end-
            // image writer. Never retry unregister/output or free callback storage.
            let _ = self.stop(Some("unfinished-diagnostic"));
        }
    }
}
