//! Private, single-use, post-registration loader observation; never numerical evidence.
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

use std::cell::UnsafeCell;
use std::ffi::c_void;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::mem::{align_of, size_of};
use std::os::windows::fs::OpenOptionsExt;
use std::path::Path;
use std::ptr::{addr_of, addr_of_mut};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::core::{s, w};

use super::ocr_dependency_images::{self, Image};

const REPORT_ENV: &str = "MADO_PILOT_OCR_LOADER_DIAGNOSTIC_REPORT";
const EVENT_LIMIT: usize = 256;
const PATH_UNITS: usize = 2_048;
const OUTPUT_LIMIT: usize = 1_048_576;
const DRAIN_LIMIT: Duration = Duration::from_secs(1);
const CLOSED: u32 = 1 << 31;
const ADMISSION_OVERFLOW: u32 = 1 << 30;
const WRITERS: u32 = ADMISSION_OVERFLOW - 1;

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

// SDK LDR_DLL_{LOADED,UNLOADED}_NOTIFICATION_DATA have the same layout. Keep
// the two union alternatives explicit, and never copy either aggregate in the
// callback. The ABI below is deliberately restricted to the Windows x64 driver.
#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *const u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NotificationPayload {
    flags: u32,
    full_dll_name: *const UnicodeString,
    base_dll_name: *const UnicodeString,
    dll_base: *mut c_void,
    size_of_image: u32,
}

#[repr(C)]
union NotificationData {
    loaded: NotificationPayload,
    unloaded: NotificationPayload,
}

const _: () = {
    assert!(size_of::<usize>() == 8);
    assert!(size_of::<UnicodeString>() == 16);
    assert!(size_of::<NotificationPayload>() == 40);
    assert!(size_of::<NotificationData>() == 40);
};

type Callback = unsafe extern "system" fn(u32, *const NotificationData, *mut c_void);
type Register = unsafe extern "system" fn(u32, Option<Callback>, *mut c_void, *mut *mut c_void) -> i32;
type Unregister = unsafe extern "system" fn(*mut c_void) -> i32;

struct Slot {
    committed: u32,
    kind: u32,
    base: u64,
    size: u32,
    length: u32,
    path: [u16; PATH_UNITS],
}

impl Slot {
    const fn new() -> Self {
        Self {
            committed: 0,
            kind: 0,
            base: 0,
            size: 0,
            length: 0,
            path: [0; PATH_UNITS],
        }
    }
}

struct Storage {
    gate: AtomicU32,
    observed: AtomicU32,
    overflow: AtomicBool,
    malformed: AtomicBool,
    slots: UnsafeCell<[Slot; EVENT_LIMIT]>,
}

// SAFETY: An admitted writer reserves one unique, never-reused ordinal. Slot
// mutation is exclusive to that writer. Readers first require CLOSED and zero
// admitted writers with an acquire load; writer release-RMWs publish every slot.
// Late callbacks can touch the permanent admission atomic but cannot touch slots.
// Neither this static storage nor synthetic static storage is ever freed/reset.
unsafe impl Sync for Storage {}

impl Storage {
    const fn new() -> Self {
        Self {
            gate: AtomicU32::new(0),
            observed: AtomicU32::new(0),
            overflow: AtomicBool::new(false),
            malformed: AtomicBool::new(false),
            slots: UnsafeCell::new([const { Slot::new() }; EVENT_LIMIT]),
        }
    }

    #[inline(always)]
    fn admit(&self) -> bool {
        let mut state = self.gate.load(Ordering::Relaxed);
        loop {
            if state & CLOSED != 0 {
                return false;
            }
            if state & WRITERS == WRITERS {
                // Close atomically on saturation; never wrap the writer count or
                // publish an overflow flag outside the admission/drain fence.
                match self.gate.compare_exchange_weak(
                    state,
                    state | CLOSED | ADMISSION_OVERFLOW,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => return false,
                    Err(current) => state = current,
                }
            } else {
                match self.gate.compare_exchange_weak(
                    state,
                    state.wrapping_add(1),
                    Ordering::Acquire,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => return true,
                    Err(current) => state = current,
                }
            }
        }
    }

    #[inline(always)]
    fn reserve(&self) -> Option<u32> {
        let mut previous = self.observed.load(Ordering::Relaxed);
        loop {
            if previous == u32::MAX {
                self.overflow.store(true, Ordering::Relaxed);
                return None;
            }
            let ordinal = previous.wrapping_add(1);
            match self.observed.compare_exchange_weak(
                previous,
                ordinal,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Some(ordinal),
                Err(current) => previous = current,
            }
        }
    }

    fn close(&self) {
        self.gate.fetch_or(CLOSED, Ordering::AcqRel);
    }

    fn drained(&self) -> bool {
        let state = self.gate.load(Ordering::Acquire);
        state & CLOSED != 0 && state & WRITERS == 0
    }

    fn drain(&self, limit: Duration) -> bool {
        let started = Instant::now();
        loop {
            if self.drained() {
                return true;
            }
            if started.elapsed() >= limit {
                return false;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn events(&self) -> Result<Vec<Event>, Failure> {
        if !self.drained() {
            return Err(Failure::Rule("callbacks-not-drained"));
        }
        let count = self.observed.load(Ordering::Relaxed).min(EVENT_LIMIT as u32);
        let mut events = Vec::with_capacity(count as usize);
        for index in 0..count as usize {
            // SAFETY: close+acquire-drain above excludes all slot writers,
            // including callbacks entering late. Storage is never reopened.
            let slot = unsafe { &*self.slots.get().cast::<Slot>().add(index) };
            if slot.committed == 0 {
                continue;
            }
            if slot.length == 0 || slot.length as usize > PATH_UNITS {
                self.malformed.store(true, Ordering::Relaxed);
                continue;
            }
            let Ok(path) = ocr_dependency_images::validated_path(&slot.path[..slot.length as usize])
            else {
                self.malformed.store(true, Ordering::Relaxed);
                continue;
            };
            events.push(Event {
                ordinal: index as u32 + 1,
                kind: slot.kind,
                base: slot.base,
                size: slot.size,
                path,
            });
        }
        Ok(events)
    }
}

static STORAGE: Storage = Storage::new();
static STARTED: AtomicBool = AtomicBool::new(false);

// This context and its entire slot arena have process lifetime, regardless of
// registration, unregister, drain, output, panic, or watchdog outcomes.
#[inline(never)]
#[unsafe(no_mangle)]
unsafe extern "system" fn mado_pilot_ocr_loader_notification(
    reason: u32,
    data: *const NotificationData,
    context: *mut c_void,
) {
    // SAFETY: Registration supplies a permanent, aligned Storage context. The
    // synthetic harness supplies the same lifetime. No ownership is transferred.
    let storage = unsafe { &*context.cast::<Storage>() };
    if !storage.admit() {
        return;
    }
    if let Some(ordinal) = storage.reserve() {
        if ordinal > EVENT_LIMIT as u32 {
            storage.overflow.store(true, Ordering::Relaxed);
        } else {
            // SAFETY: The loader supplies borrowed ABI data valid for this call.
            // copy_payload checks null/alignment/length before bounded access;
            // an admitted unique ordinal owns its slot until the release below.
            if !unsafe { copy_payload(storage, ordinal, reason, data) } {
                storage.malformed.store(true, Ordering::Relaxed);
            }
        }
    }
    storage.gate.fetch_sub(1, Ordering::Release);
}

#[inline(always)]
unsafe fn copy_payload(
    storage: &Storage,
    ordinal: u32,
    reason: u32,
    data: *const NotificationData,
) -> bool {
    if (reason != 1 && reason != 2)
        || data.is_null()
        || data.addr() % align_of::<NotificationData>() != 0
    {
        return false;
    }
    // SAFETY: The OS supplies the complete union and referenced counted string
    // for the callback duration. Both union variants have identical layout.
    // Scalar volatile accesses avoid aggregate copies and prevent transforming
    // the UTF16 loop into memcpy/vector runtime calls. No borrowed pointer escapes.
    unsafe {
        let payload = addr_of!((*data).loaded);
        let flags = addr_of!((*payload).flags).read_volatile();
        let name = addr_of!((*payload).full_dll_name).read_volatile();
        let base = addr_of!((*payload).dll_base).read_volatile();
        let size = addr_of!((*payload).size_of_image).read_volatile();
        if flags != 0
            || name.is_null()
            || name.addr() % align_of::<UnicodeString>() != 0
            || base.is_null()
            || size == 0
        {
            return false;
        }
        let bytes = addr_of!((*name).length).read_volatile();
        let maximum = addr_of!((*name).maximum_length).read_volatile();
        let source = addr_of!((*name).buffer).read_volatile();
        if bytes == 0
            || bytes & 1 != 0
            || maximum & 1 != 0
            || maximum < bytes
            || bytes as usize / 2 > PATH_UNITS
            || source.is_null()
            || source.addr() % align_of::<u16>() != 0
        {
            return false;
        }
        let length = bytes as usize / 2;
        let index = ordinal.wrapping_sub(1) as usize;
        let slot = storage.slots.get().cast::<Slot>().add(index);
        let destination = addr_of_mut!((*slot).path).cast::<u16>();
        let mut unit = 0_usize;
        while unit < length {
            destination.add(unit).write_volatile(source.add(unit).read_volatile());
            unit = unit.wrapping_add(1);
        }
        addr_of_mut!((*slot).kind).write_volatile(reason);
        addr_of_mut!((*slot).base).write_volatile(base.addr() as u64);
        addr_of_mut!((*slot).size).write_volatile(size);
        addr_of_mut!((*slot).length).write_volatile(length as u32);
        addr_of_mut!((*slot).committed).write_volatile(1);
    }
    true
}

struct Event {
    ordinal: u32,
    kind: u32,
    base: u64,
    size: u32,
    path: String,
}

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
                register: std::mem::transmute::<unsafe extern "system" fn() -> isize, Register>(register),
                unregister: std::mem::transmute::<unsafe extern "system" fn() -> isize, Unregister>(unregister),
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
        if !file.metadata().map_err(|_| Failure::Rule("trace-metadata"))?.is_file() {
            return Err(Failure::Rule("trace-regular-file"));
        }
        let mut diagnostic = Self {
            report: Report::new(file, OUTPUT_LIMIT)?,
            unregister: None,
            cookie: None,
            stopped: false,
        };
        let initialized = diagnostic.register().and_then(|()| diagnostic.checkpoint("initial"));
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
        let before = STORAGE.observed.load(Ordering::Acquire);
        // The extra two slots preserve the shared helper's conservative missing-
        // terminator boundary while permitting exactly2048 content units.
        let images = ocr_dependency_images::stable_snapshot::<{ PATH_UNITS + 2 }>()
            .map_err(|_| Failure::Rule(match name {
                "initial" => "initial-snapshot-failed",
                _ => "final-snapshot-failed",
            }))?;
        let after = STORAGE.observed.load(Ordering::Acquire);
        self.report.checkpoint(name, before, after, &images)?;
        // Persist the initial checkpoint before workloads: a watchdog/outer kill
        // can leave a visibly incomplete JSON prefix, never a claimed complete run.
        self.report.output.inner.sync_all().map_err(|_| Failure::Rule("trace-sync"))
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
        let result = self.report.finish(&STORAGE, status, drained, failure);
        let synced = self.report.output.inner.sync_all().map_err(|_| Failure::Rule("trace-sync"));
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

struct Bounded<W> {
    inner: W,
    written: usize,
    limit: usize,
    failure: Option<&'static str>,
}

impl<W: Write> Write for Bounded<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.failure.is_some() {
            return Err(io::ErrorKind::Other.into());
        }
        if bytes.len() > self.limit.saturating_sub(self.written) {
            self.failure = Some("trace-output-limit");
            return Err(io::ErrorKind::FileTooLarge.into());
        }
        match self.inner.write(bytes) {
            Ok(0) if !bytes.is_empty() => {
                self.failure = Some("trace-write-zero");
                Err(io::ErrorKind::WriteZero.into())
            }
            Ok(written) => {
                self.written += written;
                Ok(written)
            }
            Err(error) => {
                self.failure = Some("trace-write");
                Err(error)
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.failure.is_some() {
            return Err(io::ErrorKind::Other.into());
        }
        self.inner.flush().inspect_err(|_| self.failure = Some("trace-flush"))
    }
}

struct Report<W> {
    output: Bounded<W>,
    checkpoints: usize,
}

impl<W: Write> Report<W> {
    fn new(inner: W, limit: usize) -> Result<Self, Failure> {
        let mut report = Self {
            output: Bounded { inner, written: 0, limit, failure: None },
            checkpoints: 0,
        };
        report.bytes(b"{\"schema_version\":1,\"scope\":\"post-registration-window\",\"pre_main_history\":false,\"numerical_qualification\":false,\"limits\":{\"events\":256,\"path_utf16_units\":2048,\"output_bytes\":1048576},\"checkpoints\":[")?;
        Ok(report)
    }

    fn failure(&self) -> Failure {
        Failure::Rule(self.output.failure.unwrap_or("trace-write"))
    }

    fn bytes(&mut self, bytes: &[u8]) -> Result<(), Failure> {
        self.output.write_all(bytes).map_err(|_| self.failure())
    }

    fn number(&mut self, number: impl fmt::Display) -> Result<(), Failure> {
        write!(&mut self.output, "{number}").map_err(|_| self.failure())
    }

    fn string(&mut self, value: &str) -> Result<(), Failure> {
        serde_json::to_writer(&mut self.output, value).map_err(|_| self.failure())
    }

    fn checkpoint(&mut self, name: &str, before: u32, after: u32, images: &[Image]) -> Result<(), Failure> {
        if self.checkpoints != 0 {
            self.bytes(b",")?;
        }
        self.bytes(b"{\"name\":")?;
        self.string(name)?;
        self.bytes(b",\"events_before\":")?;
        self.number(before)?;
        self.bytes(b",\"events_after\":")?;
        self.number(after)?;
        self.bytes(b",\"images\":[")?;
        for (index, image) in images.iter().enumerate() {
            if index != 0 {
                self.bytes(b",")?;
            }
            self.bytes(b"{\"base\":")?;
            self.number(image.base)?;
            self.bytes(b",\"path\":")?;
            self.string(&image.path)?;
            self.bytes(b"}")?;
        }
        self.bytes(b"]}")?;
        self.checkpoints += 1;
        Ok(())
    }

    fn finish(
        &mut self,
        storage: &Storage,
        status: Option<i32>,
        drained: bool,
        failure: Option<&'static str>,
    ) -> Result<(), Failure> {
        if self.output.failure.is_some() {
            return Err(self.failure());
        }
        let drained = drained && storage.drained();
        // Do not even inspect committed flags while any admitted writer remains.
        let events = if drained { storage.events()? } else { Vec::new() };
        let gate = storage.gate.load(Ordering::Acquire);
        let closed = gate & CLOSED != 0;
        let overflow = storage.overflow.load(Ordering::Relaxed) || gate & ADMISSION_OVERFLOW != 0;
        let malformed = storage.malformed.load(Ordering::Relaxed);
        let unregistered = status == Some(0);
        let error = failure
            .or(if self.checkpoints == 2 { None } else { Some("checkpoints-incomplete") })
            .or(if closed { None } else { Some("admission-not-closed") })
            .or(if drained { None } else { Some("callbacks-not-drained") })
            .or(if unregistered { None } else { Some("unregister-failed") })
            .or(if overflow { Some("notification-overflow") } else { None })
            .or(if malformed { Some("notification-malformed") } else { None });
        self.bytes(b"],\"events\":[")?;
        for (index, event) in events.iter().enumerate() {
            if index != 0 {
                self.bytes(b",")?;
            }
            self.bytes(b"{\"ordinal\":")?;
            self.number(event.ordinal)?;
            self.bytes(b",\"kind\":")?;
            self.string(if event.kind == 1 { "load" } else { "unload" })?;
            self.bytes(b",\"base\":")?;
            self.number(event.base)?;
            self.bytes(b",\"size\":")?;
            self.number(event.size)?;
            self.bytes(b",\"path\":")?;
            self.string(&event.path)?;
            self.bytes(b"}")?;
        }
        self.bytes(b"],\"observed_events\":")?;
        self.number(storage.observed.load(Ordering::Relaxed))?;
        self.bytes(b",\"overflow\":")?;
        self.number(overflow)?;
        self.bytes(b",\"malformed\":")?;
        self.number(malformed)?;
        self.bytes(b",\"admission_closed\":")?;
        self.number(closed)?;
        self.bytes(b",\"callbacks_drained\":")?;
        self.number(drained)?;
        self.bytes(b",\"unregistered\":")?;
        self.number(unregistered)?;
        self.bytes(b",\"unregister_status\":")?;
        match status {
            Some(status) => self.number(status)?,
            None => self.bytes(b"null")?,
        }
        self.bytes(b",\"error\":")?;
        match error {
            Some(error) => self.string(error)?,
            None => self.bytes(b"null")?,
        }
        self.bytes(b",\"complete\":")?;
        self.number(error.is_none())?;
        self.bytes(b"}\n")?;
        self.output.flush().map_err(|_| self.failure())?;
        match error {
            Some(error) => Err(Failure::Rule(error)),
            None => Ok(()),
        }
    }
}

/// Synthetic-only surface: no registration, process snapshot, module loading or
/// real observation. Each test supplies a distinct process-lifetime static arena.
#[cfg(test)]
pub(crate) mod synthetic {
    use super::*;

    pub(crate) struct Recorder {
        storage: Storage,
    }

    impl Recorder {
        pub(crate) const fn new() -> Self {
            Self { storage: Storage::new() }
        }

        pub(crate) fn notify(&'static self, reason: u32, path: &[u16]) {
            let length = u16::try_from(path.len() * 2).expect("bounded synthetic path");
            self.notify_length(reason, path, length);
        }

        pub(crate) fn notify_length(&'static self, reason: u32, path: &[u16], length: u16) {
            assert!(usize::from(length) <= path.len() * 2);
            let name = UnicodeString {
                length,
                maximum_length: u16::try_from(path.len() * 2).expect("bounded synthetic path"),
                buffer: path.as_ptr(),
            };
            let data = NotificationData {
                loaded: NotificationPayload {
                    flags: 0,
                    full_dll_name: &name,
                    base_dll_name: &name,
                    dll_base: std::ptr::without_provenance_mut(0x1000),
                    size_of_image: 4096,
                },
            };
            // SAFETY: Synthetic borrowed ABI objects and their buffers remain
            // live throughout the call; only the declared bounded length is read.
            // The context is a static arena, exactly as in production.
            unsafe {
                mado_pilot_ocr_loader_notification(
                    reason,
                    &data,
                    addr_of!(self.storage).cast_mut().cast::<c_void>(),
                );
            }
        }

        pub(crate) fn notify_null(&'static self) {
            // SAFETY: The permanent context is valid; a null payload is rejected
            // before dereference by the actual callback payload validation.
            unsafe {
                mado_pilot_ocr_loader_notification(
                    1,
                    std::ptr::null(),
                    addr_of!(self.storage).cast_mut().cast::<c_void>(),
                );
            }
        }

        pub(crate) fn hold_writer(&'static self) -> HeldWriter {
            assert!(self.storage.admit());
            HeldWriter { storage: &self.storage }
        }

        pub(crate) fn close(&self) {
            self.storage.close();
        }

        pub(crate) fn drained(&self) -> bool {
            self.storage.drain(Duration::ZERO)
        }

        pub(crate) fn report(
            &self,
            status: Option<i32>,
            limit: usize,
        ) -> (Vec<u8>, Result<(), Failure>) {
            let mut bytes = Vec::new();
            let result = (|| {
                let mut report = Report::new(&mut bytes, limit)?;
                let image = Image { base: 0x1000, path: "C:\\synthetic\\fixture.exe".to_owned() };
                report.checkpoint("initial", 0, 0, std::slice::from_ref(&image))?;
                let observed = self.storage.observed.load(Ordering::Relaxed);
                report.checkpoint("final", observed, observed, std::slice::from_ref(&image))?;
                report.finish(&self.storage, status, self.storage.drained(), None)
            })();
            (bytes, result)
        }
    }

    pub(crate) struct HeldWriter {
        storage: &'static Storage,
    }

    impl Drop for HeldWriter {
        fn drop(&mut self) {
            self.storage.gate.fetch_sub(1, Ordering::Release);
        }
    }
}
