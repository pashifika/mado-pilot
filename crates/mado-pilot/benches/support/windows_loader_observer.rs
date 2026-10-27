//! Benchmark-only Windows loader observations, ported from the accepted private v2 fixture.
//! Port basis: private v2 fixture SHA-256
//! d38af0f8ad9b7877b523e357b51d7d35bc7c2fd298a16cd0125ee6d817764b8d.
//!
//! This is an observer-on, correlation-only method, not loader backing-object or code-byte
//! identity. No pre-registration history, atomic loader history, final callback drain,
//! observer-off equivalence, residency or zero-overhead claim is made. Native calls can
//! allocate internally. All Rust observer storage used during accounting is fixed and
//! pre-touched before registration; serialization alone allocates, after accounting.
//!
//! Three cuts each take a double census and bridge every module of the selected census.
//! The original cumulative 129-bridge limit is shared: a steady set can contain at most
//! 43 modules across all three cuts. No cache, truncation, retry or identity-by-name exists.
//! Fixed static objects total 3,493,341 bytes; compact observer JSON is bounded by
//! 7,788,672 bytes, below the original 8,388,608-byte cap. These are source-derived
//! layout/output bounds, not measured RSS or allocator footprints. Linker padding,
//! code, the small owner/scalar stack locals and native API costs are excluded.
//! All byte/path/parser/census/bridge buffers are process-static; native scalar
//! results and ModuleInfo use bounded stack locals, never large initializer values.
//!
//! Safety/portability: only x86_64-pc-windows-msvc is supported. The callback and its
//! three original statics remain mapped until process exit; they are never reset after
//! registration and never unregistered. A one-shot owner exclusively accesses the separate
//! observation storage. Callback payloads are immutable after Release/Acquire publication.
//! Untrusted image addresses are read only through ReadProcessMemory, never Rust loads.
//! Final linked-code/unwind inspection and actual Windows verification are still required;
//! preserving source/layout is not a certificate for a newly linked executable.
#![cfg(all(target_os = "windows", target_arch = "x86_64", target_env = "msvc"))]
#![deny(unsafe_op_in_unsafe_fn)]

use std::cell::UnsafeCell;
use std::ffi::{c_char, c_void};
use std::marker::PhantomData;
use std::mem::{MaybeUninit, align_of, offset_of, size_of, size_of_val};
use std::ptr::{null_mut, read_volatile, write_volatile};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use serde_json::{Value, json};

const PROTOCOL: &str = "mado-component-windows-loader-v1";
const METHOD: &str = "windows-ldr-observer-on-warm-v1";
const MAX_MODULES: usize = 128;
const MAX_EVENTS: usize = 256;
const NAME_UNITS: usize = 1024;
const MAX_CENSUSES: usize = 20;
const TOTAL_CENSUS_PATH_UNITS: usize = 131072;
const MAX_IMAGE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_HEADER_BYTES: u64 = 65536;
const MAX_SECTIONS: usize = 96;
const MAX_DEBUG_ENTRIES: usize = 32;
const MAX_TOTAL_SECTIONS: usize = 4096;
const MAX_TOTAL_DEBUG_ENTRIES: usize = 4096;
const MAX_CODEVIEW_BYTES: u32 = 4096;
const MAX_READ_BYTES: u64 = 64 * 1024 * 1024;
const MAX_READ_CALLS: u32 = 32768;
const MAX_BRIDGES: usize = MAX_MODULES + 1;
const MAX_CLEANUP: usize = MAX_BRIDGES * 2;
const MAX_STDOUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_WORKLOAD_ID_BYTES: usize = 256;
// Original allowances, including 20 census pairs of counters and all 129 bridges.
// The unused fixture-case/name allowance and 256 KiB envelope allowance cover the
// three cut labels, indices, completeness flags, extra counters and escaped row ID.
// Per-section 160, debug+CodeView 304 and per-bridge 4096 include maximum-width
// integers, keys, separators and failure/cleanup variants. No arbitrary strings enter
// the report: workload_id is separately bounded, names are counted UTF-16 hex.
const WORST_CASE_STDOUT_BYTES: usize = TOTAL_CENSUS_PATH_UNITS * 4
    + MAX_EVENTS * 2 * NAME_UNITS * 4
    + MAX_BRIDGES * 4 * NAME_UNITS * 4
    + NAME_UNITS * 4
    + MAX_TOTAL_SECTIONS * 160
    + MAX_TOTAL_DEBUG_ENTRIES * (192 + 112)
    + MAX_CENSUSES * MAX_MODULES * 96
    + (MAX_CENSUSES * 2 + 1) * (MAX_EVENTS * 4 + 128)
    + MAX_EVENTS * 256
    + MAX_BRIDGES * 4096
    + 262144;
const _: () = assert!(WORST_CASE_STDOUT_BYTES < MAX_STDOUT_BYTES);
const OVERFLOW: u32 = 1;
const CONTENTION: u32 = 2;
const BAD_DATA: u32 = 4;
const BAD_NAME: u32 = 8;
const COUNTER_WRAP: u32 = 16;

type Handle = *mut c_void;
type FarProc = unsafe extern "system" fn() -> isize;
type Notify = unsafe extern "system" fn(u32, *const c_void, *mut c_void);
type Register = unsafe extern "system" fn(u32, Notify, *mut c_void, *mut Handle) -> i32;

#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *const u16,
}
#[repr(C)]
struct NotificationData {
    flags: u32,
    full: *const UnicodeString,
    basename: *const UnicodeString,
    base: Handle,
    size: u32,
}
#[repr(C)]
struct Payload {
    reason: u32,
    flags: u32,
    base: usize,
    size: u32,
    invalid: u32,
    full_len: u32,
    base_len: u32,
    full: [u16; NAME_UNITS],
    basename: [u16; NAME_UNITS],
}
#[repr(C)]
struct Slot {
    complete: AtomicBool,
    payload: UnsafeCell<Payload>,
}
// SAFETY: reservation assigns exactly one writer per slot, never reused. The
// payload is read only after Acquire observes the writer's Release complete=true.
// There are no later writes to that slot; incomplete slots are never dereferenced.
unsafe impl Sync for Slot {}
impl Slot {
    const fn new() -> Self {
        Self {
            complete: AtomicBool::new(false),
            payload: UnsafeCell::new(Payload {
                reason: 0,
                flags: 0,
                base: 0,
                size: 0,
                invalid: 0,
                full_len: 0,
                base_len: 0,
                full: [0; NAME_UNITS],
                basename: [0; NAME_UNITS],
            }),
        }
    }
}
#[unsafe(no_mangle)]
static P050_WINDOW_SEQUENCE: AtomicU64 = AtomicU64::new(0);
#[unsafe(no_mangle)]
static P050_WINDOW_INCOMPLETE: AtomicU32 = AtomicU32::new(0);
#[unsafe(no_mangle)]
static P050_WINDOW_RECORDS: [Slot; MAX_EVENTS] = [const { Slot::new() }; MAX_EVENTS];

// Returns a copied length, or a refusal sentinel. Volatile scalar operations
// prevent memcpy/vector-copy lowering. Checks are structural, not pointer probes:
// pointer readability/lifetime comes solely from the OS notification contract.
#[inline(always)]
unsafe fn copy_notification_name(source: *const UnicodeString, target: *mut u16) -> u32 {
    if source.is_null() || (source as usize) & 7 != 0 {
        return u32::MAX;
    }
    // SAFETY: non-null aligned OS-owned UNICODE_STRING valid during notification.
    let (length, maximum, buffer) = unsafe {
        (
            read_volatile(&raw const (*source).length),
            read_volatile(&raw const (*source).maximum_length),
            read_volatile(&raw const (*source).buffer),
        )
    };
    if length == 0
        || length & 1 != 0
        || maximum & 1 != 0
        || length > maximum
        || usize::from(length) / 2 > NAME_UNITS
        || buffer.is_null()
        || (buffer as usize) & 1 != 0
        || (buffer as usize) > usize::MAX - usize::from(length)
    {
        return u32::MAX;
    }
    let units = usize::from(length) / 2;
    let mut i = 0;
    while i < units {
        // SAFETY: i < Length/2 <= NAME_UNITS; both buffers remain valid for
        // this callback. Destination belongs exclusively to this reservation.
        let unit = unsafe { read_volatile(buffer.add(i)) };
        if unit == 0 {
            return u32::MAX;
        }
        // SAFETY: same exclusive bounded destination as above.
        unsafe {
            write_volatile(target.add(i), unit);
        }
        i += 1;
    }
    u32::from(length) / 2
}

/// # Safety
/// Only the Windows loader may call this with its documented notification data.
/// This executable and every static above must remain mapped until process exit.
#[unsafe(no_mangle)]
unsafe extern "system" fn p050_window_notification(
    reason: u32,
    data: *const c_void,
    context: *mut c_void,
) {
    let sequence = P050_WINDOW_SEQUENCE.load(Ordering::SeqCst);
    if sequence >= MAX_EVENTS as u64 {
        P050_WINDOW_INCOMPLETE.fetch_or(
            if sequence == u64::MAX {
                OVERFLOW | COUNTER_WRAP
            } else {
                OVERFLOW
            },
            Ordering::SeqCst,
        );
        return;
    }
    // The existing MAX_EVENTS bound fits usize; conversion never truncates.
    let Ok(index) = usize::try_from(sequence) else {
        P050_WINDOW_INCOMPLETE.fetch_or(OVERFLOW, Ordering::SeqCst);
        return;
    };
    // One strong CAS, never a retry/spin/wait. A competing writer makes the
    // observation incomplete, rather than silently assuming loader serialization.
    if P050_WINDOW_SEQUENCE
        .compare_exchange(sequence, sequence + 1, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        P050_WINDOW_INCOMPLETE.fetch_or(CONTENTION, Ordering::SeqCst);
        return;
    }
    // SAFETY: sequence < MAX_EVENTS and unique monotonic CAS reservation.
    let slot = unsafe { &*P050_WINDOW_RECORDS.as_ptr().add(index) };
    let output = slot.payload.get();
    let mut invalid = 0;
    // SAFETY: exclusive slot writer, no reader accesses unpublished payload.
    unsafe {
        (*output).reason = reason;
    }
    if !context.is_null()
        || !(reason == 1 || reason == 2)
        || data.is_null()
        || (data as usize) & 7 != 0
    {
        invalid |= BAD_DATA;
    } else {
        let input = data.cast::<NotificationData>();
        // SAFETY: reason selects OS structures with identical repr(C) layout.
        // Only the callback's OS-owned structure and counted strings are read.
        unsafe {
            (*output).flags = read_volatile(&raw const (*input).flags);
            (*output).base = read_volatile(&raw const (*input).base) as usize;
            (*output).size = read_volatile(&raw const (*input).size);
            if (*output).flags != 0
                || (*output).base == 0
                || (*output).size == 0
                || u64::from((*output).size) > MAX_IMAGE_BYTES
                || (*output).base > usize::MAX - (*output).size as usize
            {
                invalid |= BAD_DATA;
            }
            let full = copy_notification_name(
                read_volatile(&raw const (*input).full),
                (&raw mut (*output).full).cast::<u16>(),
            );
            let basename = copy_notification_name(
                read_volatile(&raw const (*input).basename),
                (&raw mut (*output).basename).cast::<u16>(),
            );
            if full == u32::MAX || basename == u32::MAX {
                invalid |= BAD_NAME;
            }
            // Failed/partially copied names have no valid length, never truncate.
            (*output).full_len = if full == u32::MAX { 0 } else { full };
            (*output).base_len = if basename == u32::MAX { 0 } else { basename };
        }
    }
    // SAFETY: reservation remains exclusively owned until the release below.
    unsafe {
        (*output).invalid = invalid;
    }
    if invalid != 0 {
        P050_WINDOW_INCOMPLETE.fetch_or(invalid, Ordering::SeqCst);
    }
    slot.complete.store(true, Ordering::Release);
}

#[repr(C)]
struct ModuleInfo {
    base: Handle,
    size: u32,
    entry: Handle,
}
#[repr(C)]
#[derive(Default, PartialEq, Eq)]
struct FileId {
    volume: u64,
    id: [u8; 16],
}
#[repr(C)]
#[derive(Default, PartialEq, Eq)]
struct FileBasic {
    creation: i64,
    access: i64,
    write: i64,
    change: i64,
    attributes: u32,
}
#[repr(C)]
#[derive(Default, PartialEq, Eq)]
struct FileStandard {
    allocation: i64,
    eof: i64,
    links: u32,
    delete_pending: u8,
    directory: u8,
}
const _: () = {
    assert!(size_of::<Handle>() == 8 && size_of::<Option<FarProc>>() == 8);
    assert!(size_of::<UnicodeString>() == 16 && offset_of!(UnicodeString, buffer) == 8);
    assert!(size_of::<NotificationData>() == 40 && align_of::<NotificationData>() == 8);
    assert!(offset_of!(NotificationData, full) == 8);
    assert!(offset_of!(NotificationData, basename) == 16);
    assert!(offset_of!(NotificationData, base) == 24);
    assert!(offset_of!(NotificationData, size) == 32);
    assert!(size_of::<Payload>() == 4128 && offset_of!(Slot, payload) == 8);
    assert!(size_of::<Slot>() == 4136 && align_of::<Slot>() == 8);
    assert!(size_of::<ModuleInfo>() == 24 && offset_of!(ModuleInfo, size) == 8);
    assert!(size_of::<FileId>() == 24 && align_of::<FileId>() == 8);
    assert!(size_of::<FileBasic>() == 40 && offset_of!(FileBasic, attributes) == 32);
    assert!(size_of::<FileStandard>() == 24 && offset_of!(FileStandard, directory) == 21);
    assert!(size_of::<AtomicU64>() == 8 && size_of::<AtomicU32>() == 4);
    assert!(size_of::<[Slot; MAX_EVENTS]>() == 1058816);
};
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess() -> Handle;
    fn GetCurrentProcessId() -> u32;
    fn GetLastError() -> u32;
    fn GetModuleHandleW(name: *const u16) -> Handle;
    fn GetProcAddress(module: Handle, name: *const c_char) -> Option<FarProc>;
    fn GetModuleFileNameW(module: Handle, buffer: *mut u16, size: u32) -> u32;
    fn K32EnumProcessModules(
        process: Handle,
        modules: *mut Handle,
        bytes: u32,
        needed: *mut u32,
    ) -> i32;
    fn K32GetModuleInformation(
        process: Handle,
        module: Handle,
        info: *mut ModuleInfo,
        bytes: u32,
    ) -> i32;
    fn K32GetMappedFileNameW(process: Handle, address: Handle, buffer: *mut u16, size: u32) -> u32;
    fn ReadProcessMemory(
        process: Handle,
        address: *const c_void,
        buffer: *mut c_void,
        size: usize,
        read: *mut usize,
    ) -> i32;
    fn CreateFileW(
        path: *const u16,
        access: u32,
        share: u32,
        security: *const c_void,
        disposition: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
    fn GetFinalPathNameByHandleW(file: Handle, buffer: *mut u16, size: u32, flags: u32) -> u32;
    fn GetFileInformationByHandleEx(
        file: Handle,
        class: i32,
        buffer: *mut c_void,
        size: u32,
    ) -> i32;
    fn SetFilePointerEx(file: Handle, offset: i64, result: *mut i64, method: u32) -> i32;
    fn ReadFile(
        file: Handle,
        buffer: *mut c_void,
        bytes: u32,
        read: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn GetFileType(handle: Handle) -> u32;
    fn CloseHandle(handle: Handle) -> i32;
}

// Each discriminant has a fixed report spelling; zero is a valid absent failure.
// Storage contains no references, Vec, String, Option, owner or invalid-zero type.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum Operation {
    None = 0,
    AlreadyInitialized,
    WorkloadIdCapacity,
    InvalidPhase,
    MissingCuts,
    ReadBudget,
    MemoryRange,
    ReadProcessMemory,
    ReadProcessMemoryShort,
    EnumModules,
    ModuleCapacity,
    ModuleNull,
    ModuleInformation,
    ModuleExtent,
    ModuleFileName,
    ModulePathCapacity,
    CensusPathBudget,
    DuplicateModule,
    CensusIncomplete,
    CensusCountCap,
    DoubleCensusMismatch,
    BridgeCountCap,
    BridgeObservationChanged,
    WindowCounterIncomplete,
    SnapshotIncomplete,
    ImageReadRange,
    SetFilePointer,
    FilePosition,
    ReadFile,
    ReadFileShort,
    RvaImageExtent,
    RvaZeroFill,
    RvaAmbiguous,
    RvaUnmapped,
    RvaFileExtent,
    DosSignature,
    PeOffset,
    PeSignature,
    PeHeaderCounts,
    TotalSectionsBudget,
    PeHeaderCap,
    PeArchitecture,
    PeDirectoryCount,
    PeExtent,
    CensusPeSizeMismatch,
    PeSectionExtent,
    DebugExtent,
    TotalDebugBudget,
    CodeviewSize,
    CodeviewNotMapped,
    CodeviewFileExtent,
    CodeviewPointerMismatch,
    CodeviewSignature,
    FileType,
    FileIdInfo,
    FileBasicInfo,
    FileStandardInfo,
    FileState,
    FinalPath,
    FinalPathCapacity,
    MappedPath,
    MappedPathCapacity,
    CreateFile,
    MappedFinalMismatch,
    MemoryFilePeMismatch,
    MappedPathChanged,
    SameHandleChanged,
    FilePostObservation,
    CloseHandle,
    NtdllModule,
    NotificationExport,
    RegisterNotification,
}
impl Operation {
    fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::AlreadyInitialized => "observer_already_initialized",
            Self::WorkloadIdCapacity => "workload_id_capacity",
            Self::InvalidPhase => "observer_phase",
            Self::MissingCuts => "observer_missing_cuts",
            Self::ReadBudget => "read_budget",
            Self::MemoryRange => "memory_range",
            Self::ReadProcessMemory => "ReadProcessMemory",
            Self::ReadProcessMemoryShort => "ReadProcessMemory_short",
            Self::EnumModules => "K32EnumProcessModules",
            Self::ModuleCapacity => "module_capacity_or_shape",
            Self::ModuleNull => "module_null",
            Self::ModuleInformation => "K32GetModuleInformation",
            Self::ModuleExtent => "module_extent",
            Self::ModuleFileName => "GetModuleFileNameW",
            Self::ModulePathCapacity => "module_path_capacity",
            Self::CensusPathBudget => "census_path_budget",
            Self::DuplicateModule => "duplicate_module",
            Self::CensusIncomplete => "census_observation_incomplete_or_changed",
            Self::CensusCountCap => "census_count_cap",
            Self::DoubleCensusMismatch => "boundary_double_census_mismatch",
            Self::BridgeCountCap => "bridge_count_cap",
            Self::BridgeObservationChanged => "bridge_observation_incomplete_or_changed",
            Self::WindowCounterIncomplete => "window_counter_incomplete",
            Self::SnapshotIncomplete => "record_snapshot_incomplete",
            Self::ImageReadRange => "image_read_range",
            Self::SetFilePointer => "SetFilePointerEx",
            Self::FilePosition => "file_position",
            Self::ReadFile => "ReadFile",
            Self::ReadFileShort => "ReadFile_short",
            Self::RvaImageExtent => "rva_image_extent",
            Self::RvaZeroFill => "rva_zero_fill_not_file",
            Self::RvaAmbiguous => "rva_ambiguous",
            Self::RvaUnmapped => "rva_unmapped",
            Self::RvaFileExtent => "rva_file_extent",
            Self::DosSignature => "dos_signature",
            Self::PeOffset => "pe_offset",
            Self::PeSignature => "pe_signature",
            Self::PeHeaderCounts => "pe_header_counts",
            Self::TotalSectionsBudget => "total_sections_budget",
            Self::PeHeaderCap => "pe_header_cap",
            Self::PeArchitecture => "pe_not_amd64_pe32plus",
            Self::PeDirectoryCount => "pe_directory_count",
            Self::PeExtent => "pe_extent",
            Self::CensusPeSizeMismatch => "census_pe_size_mismatch",
            Self::PeSectionExtent => "pe_section_extent",
            Self::DebugExtent => "debug_directory_extent",
            Self::TotalDebugBudget => "total_debug_budget",
            Self::CodeviewSize => "codeview_size",
            Self::CodeviewNotMapped => "codeview_present_not_mapped",
            Self::CodeviewFileExtent => "codeview_file_extent",
            Self::CodeviewPointerMismatch => "codeview_rva_file_pointer_mismatch",
            Self::CodeviewSignature => "codeview_unsupported_signature",
            Self::FileType => "GetFileType_not_disk",
            Self::FileIdInfo => "FileIdInfo",
            Self::FileBasicInfo => "FileBasicInfo",
            Self::FileStandardInfo => "FileStandardInfo",
            Self::FileState => "file_extent_or_state",
            Self::FinalPath => "GetFinalPathNameByHandleW",
            Self::FinalPathCapacity => "final_path_capacity",
            Self::MappedPath => "K32GetMappedFileNameW",
            Self::MappedPathCapacity => "mapped_path_capacity",
            Self::CreateFile => "CreateFileW",
            Self::MappedFinalMismatch => "mapped_final_path_mismatch",
            Self::MemoryFilePeMismatch => "memory_file_pe_mismatch",
            Self::MappedPathChanged => "mapped_path_changed",
            Self::SameHandleChanged => "same_handle_file_changed",
            Self::FilePostObservation => "file_post_observation",
            Self::CloseHandle => "CloseHandle",
            Self::NtdllModule => "GetModuleHandleW_ntdll",
            Self::NotificationExport => "GetProcAddress_LdrRegisterDllNotification",
            Self::RegisterNotification => "LdrRegisterDllNotification",
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Failure {
    operation: Operation,
    code: u32,
    detail: u64,
    has_code: bool,
}
type Result<T, E = Failure> = std::result::Result<T, E>;
fn refuse(operation: Operation, detail: u64) -> Failure {
    Failure {
        operation,
        code: 0,
        detail,
        has_code: false,
    }
}
fn last_error(operation: Operation) -> Failure {
    Failure {
        operation,
        // SAFETY: scalar query; capture before any subsequent native call.
        code: unsafe { GetLastError() },
        detail: 0,
        has_code: true,
    }
}
impl Failure {
    fn present(&self) -> bool {
        self.operation != Operation::None
    }
    fn retain(&mut self, error: Failure) {
        if !self.present() {
            *self = error;
        }
    }
}

#[repr(C)]
struct Budget {
    bytes: u64,
    calls: u32,
    paths: usize,
    sections: usize,
    debug_entries: usize,
}
impl Budget {
    fn read(&mut self, bytes: usize) -> Result<()> {
        if self.calls >= MAX_READ_CALLS || bytes as u64 > MAX_READ_BYTES - self.bytes {
            return Err(refuse(Operation::ReadBudget, bytes as u64));
        }
        self.calls += 1;
        self.bytes += bytes as u64;
        Ok(())
    }
}
#[repr(C)]
struct Name {
    units: [u16; NAME_UNITS],
    len: u32,
}
impl Name {
    fn as_slice(&self) -> &[u16] {
        &self.units[..self.len as usize]
    }
    fn accept(&mut self, count: u32, operation: Operation) -> Result<()> {
        if count as usize >= NAME_UNITS || self.units[..count as usize].contains(&0) {
            return Err(refuse(operation, u64::from(count)));
        }
        self.len = count;
        Ok(())
    }
}
#[repr(C)]
struct Counter {
    first: u64,
    last: u64,
    incomplete: u32,
    pending_len: u32,
    pending: [u64; MAX_EVENTS],
}
impl Counter {
    fn clean(&self) -> bool {
        self.first == self.last
            && self.last <= MAX_EVENTS as u64
            && self.incomplete == 0
            && self.pending_len == 0
    }
    fn pending(&self) -> &[u64] {
        &self.pending[..self.pending_len as usize]
    }
    fn copy_snapshot_from(&mut self, observed: &Self) {
        self.first = observed.first;
        self.last = observed.last;
        self.incomplete = observed.incomplete;
        self.pending_len = observed.pending_len;
        self.pending[..observed.pending_len as usize].copy_from_slice(observed.pending());
    }
}
fn counter(out: &mut Counter) {
    out.first = P050_WINDOW_SEQUENCE.load(Ordering::SeqCst);
    out.pending_len = 0;
    let count = usize::try_from(out.first).map_or(MAX_EVENTS, |n| n.min(MAX_EVENTS));
    for (i, slot) in P050_WINDOW_RECORDS[..count].iter().enumerate() {
        if !slot.complete.load(Ordering::Acquire) {
            out.pending[out.pending_len as usize] = i as u64 + 1;
            out.pending_len += 1;
        }
    }
    out.incomplete = P050_WINDOW_INCOMPLETE.load(Ordering::SeqCst);
    out.last = P050_WINDOW_SEQUENCE.load(Ordering::SeqCst);
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Module {
    base: usize,
    size: u32,
    path_start: u32,
    path_len: u32,
}
impl Module {
    fn path<'a>(&self, paths: &'a [u16; TOTAL_CENSUS_PATH_UNITS]) -> &'a [u16] {
        &paths[self.path_start as usize..(self.path_start + self.path_len) as usize]
    }
}
#[repr(C)]
struct Census {
    before: Counter,
    after: Counter,
    needed: u32,
    module_len: u32,
    modules: [Module; MAX_MODULES],
    error: Failure,
}
impl Census {
    fn modules(&self) -> &[Module] {
        &self.modules[..self.module_len as usize]
    }
}
fn same_modules(a: &Census, b: &Census, paths: &[u16; TOTAL_CENSUS_PATH_UNITS]) -> bool {
    a.module_len == b.module_len
        && a.modules()
            .iter()
            .zip(b.modules())
            .all(|(a, b)| a.base == b.base && a.size == b.size && a.path(paths) == b.path(paths))
}
#[repr(C)]
#[derive(Default, PartialEq, Eq)]
struct Section {
    virtual_size: u32,
    rva: u32,
    raw_size: u32,
    raw: u32,
    characteristics: u32,
}
#[repr(C)]
#[derive(Default, PartialEq, Eq)]
struct DebugEntry {
    kind: u32,
    size: u32,
    rva: u32,
    raw: u32,
    timestamp: u32,
    major: u16,
    minor: u16,
}
#[repr(C)]
#[derive(Default, PartialEq, Eq)]
struct CodeView {
    debug_index: u32,
    guid: [u8; 16],
    age: u32,
}
#[repr(C)]
struct Pe {
    machine: u16,
    magic: u16,
    characteristics: u16,
    subsystem: u16,
    dll_characteristics: u16,
    timestamp: u32,
    entry: u32,
    image_size: u32,
    headers_size: u32,
    checksum: u32,
    preferred_base: u64,
    section_len: u32,
    debug_len: u32,
    codeview_len: u32,
    sections: [Section; MAX_SECTIONS],
    debug: [DebugEntry; MAX_DEBUG_ENTRIES],
    codeview: [CodeView; MAX_DEBUG_ENTRIES],
}
impl Pe {
    fn sections(&self) -> &[Section] {
        &self.sections[..self.section_len as usize]
    }
    fn debug(&self) -> &[DebugEntry] {
        &self.debug[..self.debug_len as usize]
    }
    fn codeview(&self) -> &[CodeView] {
        &self.codeview[..self.codeview_len as usize]
    }
}
fn pe_headers_correlate(memory: &Pe, file: &Pe) -> bool {
    // Exhaustive destructuring makes future fields require an explicit policy.
    // Only preferred_base is excluded. Both raw values are always serialized.
    let Pe {
        machine,
        timestamp,
        characteristics,
        magic,
        entry,
        preferred_base: _,
        image_size,
        headers_size,
        checksum,
        subsystem,
        dll_characteristics,
        section_len,
        debug_len,
        codeview_len,
        sections,
        debug,
        codeview,
    } = memory;
    *machine == file.machine
        && *timestamp == file.timestamp
        && *characteristics == file.characteristics
        && *magic == file.magic
        && *entry == file.entry
        && *image_size == file.image_size
        && *headers_size == file.headers_size
        && *checksum == file.checksum
        && *subsystem == file.subsystem
        && *dll_characteristics == file.dll_characteristics
        && *section_len == file.section_len
        && *debug_len == file.debug_len
        && *codeview_len == file.codeview_len
        && &sections[..*section_len as usize] == file.sections()
        && &debug[..*debug_len as usize] == file.debug()
        && &codeview[..*codeview_len as usize] == file.codeview()
}
#[repr(C)]
struct FileFacts {
    id: FileId,
    basic: FileBasic,
    standard: FileStandard,
    final_path: Name,
}
#[repr(C)]
struct Bridge {
    stage: u32,
    census_index: u32,
    module_index: u32,
    base: usize,
    mapped_path: Name,
    mapped_path_after: Name,
    before: FileFacts,
    after: FileFacts,
    memory_pe: Pe,
    file_pe: Pe,
    error: Failure,
    before_attempted: bool,
    before_complete: bool,
    after_attempted: bool,
    after_complete: bool,
    memory_attempted: bool,
    memory_complete: bool,
    file_attempted: bool,
    file_complete: bool,
    close_attempted: bool,
    close_success: bool,
}
#[repr(C)]
struct Cleanup {
    operation: Operation,
    handle: usize,
    success: bool,
    error: Failure,
}
#[repr(C)]
struct Scratch {
    handles: [Handle; MAX_MODULES],
    name: Name,
    open_path: [u16; NAME_UNITS + 1],
    dos: [u8; 64],
    coff: [u8; 24],
    optional: [u8; 4096],
    section: [u8; 40],
    debug: [u8; 28],
    codeview: [u8; 24],
}
// Tracks an existing immutable counter, not a request to acquire a newer one.
// Zero is valid before the first native counter (e.g. registration failure).
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum SnapshotSource {
    None = 0,
    Initial,
    Begin,
    End,
    CensusAfter,
    Final,
}
#[repr(C)]
struct Storage {
    censuses: [Census; MAX_CENSUSES],
    bridges: [Bridge; MAX_BRIDGES],
    cleanup: [Cleanup; MAX_CLEANUP],
    paths: [u16; TOTAL_CENSUS_PATH_UNITS],
    scratch: Scratch,
    budget: Budget,
    primary: Failure,
    initial_counter: Counter,
    begin_counter: Counter,
    end_counter: Counter,
    final_counter: Counter,
    cuts: [[u32; 2]; 3],
    event_complete: [bool; MAX_EVENTS],
    census_len: u32,
    bridge_len: u32,
    cleanup_len: u32,
    registration_module: usize,
    registration_export: usize,
    cookie: usize,
    registration_status: i32,
    registration_attempted: bool,
    initial_counter_present: bool,
    begin_counter_present: bool,
    end_counter_present: bool,
    end_bridge_counter_present: bool,
    final_counter_present: bool,
    phase: u32,
    snapshot_source: SnapshotSource,
    snapshot_census: u32,
}
struct ExclusiveStorage(UnsafeCell<MaybeUninit<Storage>>);
// SAFETY: only a successful one-shot CLAIMED CAS may form a Storage reference.
// The flag is never released, including failure/drop. No callback refers to STORAGE.
unsafe impl Sync for ExclusiveStorage {}
static STORAGE: ExclusiveStorage = ExclusiveStorage(UnsafeCell::new(MaybeUninit::uninit()));
static CLAIMED: AtomicBool = AtomicBool::new(false);
const CALLBACK_STORAGE_BYTES: usize =
    size_of::<AtomicU64>() + size_of::<AtomicU32>() + size_of::<[Slot; MAX_EVENTS]>();
const FIXED_STORAGE_BYTES: usize =
    size_of::<Storage>() + CALLBACK_STORAGE_BYTES + size_of::<AtomicBool>();

/// Exclusive, non-Send/non-Sync single-row owner; never releases process ownership.
/// Dropping it does not free native callback storage or claim unregister/drain.
#[repr(C)]
pub(super) struct WindowObserver {
    storage: Option<&'static mut Storage>,
    workload_id: &'static str,
    statistical: bool,
    pid: u32,
    local_failure: Failure,
    _thread_bound: PhantomData<*mut ()>,
}
// x64 C layout: storage 8 + workload_id 16 + statistical 1 + padding 3
// + pid 4 + Failure 24 + zero-sized PhantomData 0 = 56, aligned to 8.
const OWNER_BYTES: usize = size_of::<WindowObserver>();

/// Manifest-only provenance: no registration, census, file observation or reset.
pub(super) fn capability() -> Result<Value, String> {
    // SAFETY: null requests the borrowed current executable module handle.
    let base = unsafe { GetModuleHandleW(std::ptr::null()) } as usize;
    if base == 0 {
        // SAFETY: scalar error retrieval immediately after the failed API.
        return Err(format!("GetModuleHandleW(NULL) failed: {}", unsafe {
            GetLastError()
        }));
    }
    fn rva(base: usize, address: usize, bytes: usize) -> Result<u32, String> {
        let delta = address
            .checked_sub(base)
            .ok_or_else(|| "observer address below executable base".to_owned())?;
        let rva = u32::try_from(delta).map_err(|_| "observer RVA exceeds u32".to_owned())?;
        let end = delta
            .checked_add(bytes)
            .ok_or_else(|| "observer RVA range overflow".to_owned())?;
        if end as u64 > u64::from(u32::MAX) + 1 {
            return Err("observer RVA range exceeds image address space".to_owned());
        }
        Ok(rva)
    }
    Ok(json!({
        "protocol": PROTOCOL,
        "callback": {
            "rva": rva(base, p050_window_notification as *const () as usize, 1)?,
            "statics": {
                "sequence": {"rva": rva(base, &raw const P050_WINDOW_SEQUENCE as usize, 8)?, "bytes": 8},
                "incomplete": {"rva": rva(base, &raw const P050_WINDOW_INCOMPLETE as usize, 4)?, "bytes": 4},
                "records": {"rva": rva(base, &raw const P050_WINDOW_RECORDS as usize, 1058816)?, "bytes": 1058816}
            }
        }
    }))
}

impl WindowObserver {
    pub(super) fn initialize(workload_id: &'static str, statistical: bool) -> Self {
        // SAFETY: scalar query; no observer registration or pointer access.
        let pid = unsafe { GetCurrentProcessId() };
        let mut owner = Self {
            storage: None,
            workload_id,
            statistical,
            pid,
            local_failure: refuse(Operation::None, 0),
            _thread_bound: PhantomData,
        };
        if CLAIMED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            owner.local_failure = refuse(Operation::AlreadyInitialized, 0);
            return owner;
        }
        // SAFETY: the unreleased one-shot claim excludes every other owner.
        // Storage is repr(C), contains only integer/bool/raw-pointer arrays and
        // Operation/SnapshotSource (both have a valid zero None discriminant).
        // No references/enums with invalid zero or owning values are hidden
        // inside it. Initialize bytes in place, including padding, before forming
        // a typed reference. Volatile writes pre-touch every byte without a large
        // initializer temporary. This is not a working-set residency guarantee.
        let state = unsafe {
            let pointer = (*STORAGE.0.get()).as_mut_ptr();
            let bytes = pointer.cast::<u8>();
            for i in 0..size_of::<Storage>() {
                write_volatile(bytes.add(i), 0);
            }
            &mut *pointer
        };
        for cut in &mut state.cuts {
            *cut = [u32::MAX; 2];
        }
        // No registration has happened in this process owner yet. Only this
        // initialization writes callback storage outside the loader callback.
        P050_WINDOW_SEQUENCE.store(0, Ordering::SeqCst);
        P050_WINDOW_INCOMPLETE.store(0, Ordering::SeqCst);
        for slot in &P050_WINDOW_RECORDS {
            slot.complete.store(false, Ordering::Relaxed);
            // SAFETY: no callback can run before registration; Payload contains
            // only integers/arrays. Never run this after registration or reuse.
            unsafe {
                let bytes = slot.payload.get().cast::<u8>();
                for i in 0..size_of::<Payload>() {
                    write_volatile(bytes.add(i), 0);
                }
            }
        }
        if workload_id.len() > MAX_WORKLOAD_ID_BYTES {
            state.primary = refuse(Operation::WorkloadIdCapacity, workload_id.len() as u64);
        } else if let Err(error) = register(state).and_then(|()| take_cut(state, 0)) {
            state.primary.retain(error);
        } else {
            state.phase = 1;
        }
        if state.primary.present() {
            capture_snapshot(state);
        }
        owner.storage = Some(state);
        owner
    }

    pub(super) fn failed(&self) -> bool {
        self.local_failure.present()
            || self
                .storage
                .as_ref()
                .is_some_and(|state| state.primary.present())
    }

    /// Called before samples-begin/CPU start, after allocation/work accounting starts.
    pub(super) fn begin(&mut self) {
        let Some(state) = self.storage.as_deref_mut() else {
            return;
        };
        if state.primary.present() {
            return;
        }
        if state.phase != 1 {
            state.primary = refuse(Operation::InvalidPhase, u64::from(state.phase));
        } else if let Err(error) = take_cut(state, 1) {
            state.primary.retain(error);
        } else {
            state.phase = 2;
        }
        if state.primary.present() {
            capture_snapshot(state);
        }
    }

    /// Called after CPU end, before samples-end; never inside a retained sample clock.
    pub(super) fn end(&mut self) {
        let Some(state) = self.storage.as_deref_mut() else {
            return;
        };
        if state.primary.present() {
            return;
        }
        if state.phase != 2 {
            state.primary = refuse(Operation::InvalidPhase, u64::from(state.phase));
        } else {
            counter(&mut state.end_counter);
            state.end_counter_present = true;
            state.snapshot_source = SnapshotSource::End;
            if !state.end_counter.clean() {
                state.primary = refuse(Operation::WindowCounterIncomplete, state.end_counter.last);
            } else if let Err(error) = take_cut(state, 2) {
                state.primary.retain(error);
            } else {
                state.phase = 3;
            }
        }
        capture_snapshot(state);
    }

    /// Must be called only after the final allocation/work accounting snapshots.
    /// This serializes the frozen cut, not a newer repaired observation or final drain.
    pub(super) fn into_report(mut self) -> Value {
        if let Some(state) = self.storage.as_deref_mut() {
            if state.phase != 3 {
                state
                    .primary
                    .retain(refuse(Operation::MissingCuts, u64::from(state.phase)));
            }
            // Missing end freezes the last captured cut, never a substitute
            // counter or a newer publication read. Registration failures already
            // captured their snapshot before accounting began.
            if !state.final_counter_present && state.snapshot_source != SnapshotSource::None {
                capture_snapshot(state);
            }
        }
        render(&self)
    }
}

fn register(state: &mut Storage) -> Result<()> {
    let name: [u16; 10] = [110, 116, 100, 108, 108, 46, 100, 108, 108, 0];
    // SAFETY: terminated basename; borrowed system-module handle, no load/free.
    let module = unsafe { GetModuleHandleW(name.as_ptr()) };
    state.registration_module = module as usize;
    if module.is_null() {
        return Err(last_error(Operation::NtdllModule));
    }
    // SAFETY: exact export spelling, live borrowed ntdll module.
    let export = unsafe { GetProcAddress(module, c"LdrRegisterDllNotification".as_ptr()) };
    let export = export.ok_or_else(|| last_error(Operation::NotificationExport))?;
    state.registration_export = export as usize;
    // SAFETY: documented NTSTATUS/ULONG/CALLBACK/PVOID/PVOID* x64 ABI.
    let register: Register = unsafe { std::mem::transmute(export) };
    let mut cookie = null_mut();
    // SAFETY: callback/statics are process-lifetime, null context, writable cookie.
    let status = unsafe { register(0, p050_window_notification, null_mut(), &mut cookie) };
    state.registration_attempted = true;
    state.registration_status = status;
    state.cookie = cookie as usize;
    if status != 0 || cookie.is_null() {
        return Err(Failure {
            operation: Operation::RegisterNotification,
            code: status.cast_unsigned(),
            detail: cookie as usize as u64,
            has_code: true,
        });
    }
    Ok(())
}

fn take_cut(state: &mut Storage, stage: u32) -> Result<()> {
    if state.census_len as usize > MAX_CENSUSES - 2 {
        return Err(refuse(
            Operation::CensusCountCap,
            u64::from(state.census_len),
        ));
    }
    let first = state.census_len;
    for position in 0..2 {
        let index = state.census_len;
        // Reserve first so all partial raw fields survive a native failure.
        state.census_len += 1;
        state.cuts[stage as usize][position] = index;
        census(
            &mut state.censuses[index as usize],
            &mut state.paths,
            &mut state.budget,
            &mut state.scratch,
        );
        state.snapshot_source = SnapshotSource::CensusAfter;
        state.snapshot_census = index;
        let sample = &state.censuses[index as usize];
        if sample.error.present() {
            return Err(sample.error);
        }
    }
    let a = &state.censuses[first as usize];
    let b = &state.censuses[(first + 1) as usize];
    if !same_modules(a, b, &state.paths) || a.after.last != b.before.first {
        return Err(refuse(Operation::DoubleCensusMismatch, u64::from(first)));
    }
    if stage == 2 && state.end_counter.last != a.before.first {
        return Err(refuse(Operation::BridgeObservationChanged, a.before.first));
    }
    for module_index in 0..b.module_len {
        if state.bridge_len as usize == MAX_BRIDGES {
            return Err(refuse(
                Operation::BridgeCountCap,
                u64::from(state.bridge_len),
            ));
        }
        let index = state.bridge_len as usize;
        state.bridge_len += 1;
        let row = &mut state.bridges[index];
        row.stage = stage;
        row.census_index = first + 1;
        row.module_index = module_index;
        bridge(
            &b.modules[module_index as usize],
            &state.paths,
            row,
            &mut state.budget,
            &mut state.scratch,
            &mut state.cleanup,
            &mut state.cleanup_len,
        );
        if row.error.present() {
            return Err(row.error);
        }
    }
    let post = match stage {
        0 => {
            state.initial_counter_present = true;
            state.snapshot_source = SnapshotSource::Initial;
            &mut state.initial_counter
        }
        1 => {
            state.begin_counter_present = true;
            state.snapshot_source = SnapshotSource::Begin;
            &mut state.begin_counter
        }
        _ => {
            state.end_bridge_counter_present = true;
            state.snapshot_source = SnapshotSource::Final;
            &mut state.final_counter
        }
    };
    counter(post);
    if !post.clean() || post.first != b.after.last {
        return Err(refuse(Operation::BridgeObservationChanged, post.last));
    }
    Ok(())
}

fn capture_snapshot(state: &mut Storage) {
    if state.final_counter_present {
        return;
    }
    // Freeze the actual last native cut, including its pending slots. Never take
    // another counter to replace a known failed census/window/post-bridge cut.
    // Only a failure before any counter (e.g. registration) needs a fresh snapshot.
    match state.snapshot_source {
        SnapshotSource::None => counter(&mut state.final_counter),
        SnapshotSource::Initial => state
            .final_counter
            .copy_snapshot_from(&state.initial_counter),
        SnapshotSource::Begin => state.final_counter.copy_snapshot_from(&state.begin_counter),
        SnapshotSource::End => state.final_counter.copy_snapshot_from(&state.end_counter),
        SnapshotSource::CensusAfter => state
            .final_counter
            .copy_snapshot_from(&state.censuses[state.snapshot_census as usize].after),
        SnapshotSource::Final => {}
    }
    state.final_counter_present = true;
    let c = &state.final_counter;
    let mut pending_index = 0;
    let count = usize::try_from(c.first).map_or(MAX_EVENTS, |n| n.min(MAX_EVENTS));
    for (i, slot) in P050_WINDOW_RECORDS[..count].iter().enumerate() {
        // counter() already performed the Acquire. Never re-read completion here
        // or in serialization: a slot pending at this cut may not be healed.
        let complete =
            pending_index == c.pending_len as usize || c.pending[pending_index] != i as u64 + 1;
        if !complete {
            pending_index += 1;
        }
        state.event_complete[i] = complete;
        if complete {
            // SAFETY: the earlier Acquire observed immutable one-time publication.
            let payload = unsafe { &*slot.payload.get() };
            if !published_payload_valid(payload) {
                state
                    .primary
                    .retain(refuse(Operation::SnapshotIncomplete, i as u64 + 1));
            }
        }
    }
    if !c.clean() {
        state
            .primary
            .retain(refuse(Operation::SnapshotIncomplete, c.last));
    }
}

fn published_payload_valid(p: &Payload) -> bool {
    p.invalid == 0
        && (p.reason == 1 || p.reason == 2)
        && p.flags == 0
        && p.base != 0
        && p.size != 0
        && u64::from(p.size) <= MAX_IMAGE_BYTES
        && p.base.checked_add(p.size as usize).is_some()
        && p.full_len > 0
        && p.full_len as usize <= NAME_UNITS
        && p.base_len > 0
        && p.base_len as usize <= NAME_UNITS
        && !p.full[..p.full_len as usize].contains(&0)
        && !p.basename[..p.base_len as usize].contains(&0)
}

fn census(
    out: &mut Census,
    paths: &mut [u16; TOTAL_CENSUS_PATH_UNITS],
    budget: &mut Budget,
    scratch: &mut Scratch,
) {
    counter(&mut out.before);
    let attempt = (|| -> Result<()> {
        // SAFETY: exact fixed output extent; borrowed current-process pseudo-handle.
        if unsafe {
            K32EnumProcessModules(
                GetCurrentProcess(),
                scratch.handles.as_mut_ptr(),
                u32::try_from(size_of_val(&scratch.handles)).map_err(|_| {
                    refuse(
                        Operation::ModuleCapacity,
                        size_of_val(&scratch.handles) as u64,
                    )
                })?,
                &mut out.needed,
            )
        } == 0
        {
            return Err(last_error(Operation::EnumModules));
        }
        if out.needed == 0
            || out.needed as usize > size_of_val(&scratch.handles)
            || !(out.needed as usize).is_multiple_of(size_of::<Handle>())
        {
            return Err(refuse(Operation::ModuleCapacity, u64::from(out.needed)));
        }
        for i in 0..out.needed as usize / size_of::<Handle>() {
            let handle = scratch.handles[i];
            if handle.is_null() {
                return Err(refuse(Operation::ModuleNull, 0));
            }
            let mut info = ModuleInfo {
                base: null_mut(),
                size: 0,
                entry: null_mut(),
            };
            // SAFETY: opaque module value, exact writable native structure.
            if unsafe {
                K32GetModuleInformation(
                    GetCurrentProcess(),
                    handle,
                    &mut info,
                    u32::try_from(size_of::<ModuleInfo>()).map_err(|_| {
                        refuse(Operation::ModuleInformation, size_of::<ModuleInfo>() as u64)
                    })?,
                )
            } == 0
            {
                return Err(last_error(Operation::ModuleInformation));
            }
            if info.base != handle || info.size == 0 || u64::from(info.size) > MAX_IMAGE_BYTES {
                return Err(refuse(Operation::ModuleExtent, u64::from(info.size)));
            }
            // SAFETY: fixed name destination; never dereference the module in Rust.
            let units = unsafe {
                GetModuleFileNameW(
                    handle,
                    scratch.name.units.as_mut_ptr(),
                    u32::try_from(NAME_UNITS)
                        .map_err(|_| refuse(Operation::ModulePathCapacity, NAME_UNITS as u64))?,
                )
            };
            if units == 0 {
                return Err(last_error(Operation::ModuleFileName));
            }
            scratch.name.accept(units, Operation::ModulePathCapacity)?;
            if units as usize > TOTAL_CENSUS_PATH_UNITS - budget.paths {
                return Err(refuse(Operation::CensusPathBudget, u64::from(units)));
            }
            let start = budget.paths;
            budget.paths += units as usize;
            paths[start..budget.paths].copy_from_slice(scratch.name.as_slice());
            out.modules[out.module_len as usize] = Module {
                base: handle as usize,
                size: info.size,
                path_start: u32::try_from(start)
                    .map_err(|_| refuse(Operation::CensusPathBudget, start as u64))?,
                path_len: units,
            };
            out.module_len += 1;
        }
        let modules = &mut out.modules[..out.module_len as usize];
        modules.sort_unstable_by_key(|module| module.base);
        if modules.windows(2).any(|p| p[0].base == p[1].base) {
            return Err(refuse(Operation::DuplicateModule, 0));
        }
        Ok(())
    })();
    counter(&mut out.after);
    if let Err(error) = attempt {
        out.error = error;
    }
    if !out.error.present()
        && (!out.before.clean() || !out.after.clean() || out.before.last != out.after.first)
    {
        out.error = refuse(Operation::CensusIncomplete, out.after.last);
    }
}

fn memory_read(
    base: usize,
    limit: u64,
    offset: u64,
    out: &mut [u8],
    budget: &mut Budget,
) -> Result<()> {
    let offset_usize =
        usize::try_from(offset).map_err(|_| refuse(Operation::MemoryRange, offset))?;
    if offset > limit
        || out.len() as u64 > limit - offset
        || base
            .checked_add(offset_usize)
            .and_then(|p| p.checked_add(out.len()))
            .is_none()
    {
        return Err(refuse(Operation::MemoryRange, offset));
    }
    budget.read(out.len())?;
    let mut read = 0;
    // SAFETY: only RPM sees the possibly-unmapped source. Rust owns the exact
    // bounded output slice and never dereferences foreign image addresses.
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            (base + offset_usize) as *const c_void,
            out.as_mut_ptr().cast(),
            out.len(),
            &mut read,
        )
    };
    if ok == 0 {
        let mut error = last_error(Operation::ReadProcessMemory);
        error.detail = read as u64;
        return Err(error);
    }
    if read != out.len() {
        return Err(refuse(Operation::ReadProcessMemoryShort, read as u64));
    }
    Ok(())
}
fn u16le(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn u32le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}
fn u64le(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes([
        b[at],
        b[at + 1],
        b[at + 2],
        b[at + 3],
        b[at + 4],
        b[at + 5],
        b[at + 6],
        b[at + 7],
    ])
}

enum Reader {
    Memory {
        base: usize,
        size: u64,
    },
    File {
        handle: Handle,
        size: u64,
    },
    // Test-only owned bytes exercise the same file-layout parser without FFI.
    #[cfg(test)]
    #[allow(dead_code, reason = "harness = false benchmarks omit test bodies")]
    Bytes(Vec<u8>),
}
impl Reader {
    fn size(&self) -> u64 {
        match self {
            Self::Memory { size, .. } | Self::File { size, .. } => *size,
            #[cfg(test)]
            Self::Bytes(bytes) => bytes.len() as u64,
        }
    }
    fn read(&self, offset: u64, out: &mut [u8], budget: &mut Budget) -> Result<()> {
        if offset > self.size() || out.len() as u64 > self.size() - offset {
            return Err(refuse(Operation::ImageReadRange, offset));
        }
        match self {
            Self::Memory { base, size } => memory_read(*base, *size, offset, out, budget),
            Self::File { handle, .. } => {
                budget.read(out.len())?;
                let file_offset =
                    i64::try_from(offset).map_err(|_| refuse(Operation::ImageReadRange, offset))?;
                let mut position = -1;
                // SAFETY: live synchronous handle, bounded nonnegative offset.
                if unsafe { SetFilePointerEx(*handle, file_offset, &mut position, 0) } == 0 {
                    return Err(last_error(Operation::SetFilePointer));
                }
                if position != file_offset {
                    return Err(refuse(Operation::FilePosition, position.cast_unsigned()));
                }
                let mut read = 0;
                // SAFETY: exact owned destination, synchronous read (no OVERLAPPED).
                if unsafe {
                    ReadFile(
                        *handle,
                        out.as_mut_ptr().cast(),
                        u32::try_from(out.len())
                            .map_err(|_| refuse(Operation::ReadBudget, out.len() as u64))?,
                        &mut read,
                        null_mut(),
                    )
                } == 0
                {
                    let mut error = last_error(Operation::ReadFile);
                    error.detail = u64::from(read);
                    return Err(error);
                }
                if read as usize != out.len() {
                    return Err(refuse(Operation::ReadFileShort, u64::from(read)));
                }
                Ok(())
            }
            #[cfg(test)]
            Self::Bytes(bytes) => {
                budget.read(out.len())?;
                let offset = usize::try_from(offset)
                    .map_err(|_| refuse(Operation::ImageReadRange, offset))?;
                out.copy_from_slice(&bytes[offset..offset + out.len()]);
                Ok(())
            }
        }
    }
    fn rva_offset(&self, rva: u32, bytes: u32, pe: &Pe) -> Result<u64> {
        let start = u64::from(rva);
        let end = start + u64::from(bytes);
        if end > u64::from(pe.image_size) {
            return Err(refuse(Operation::RvaImageExtent, end));
        }
        if matches!(self, Self::Memory { .. }) {
            return Ok(start);
        }
        let mut found = None;
        if end <= u64::from(pe.headers_size) {
            found = Some(start);
        }
        for section in pe.sections() {
            let section_start = u64::from(section.rva);
            let span = u64::from(section.virtual_size.max(section.raw_size));
            if start >= section_start && end <= section_start + span {
                let relative = start - section_start;
                if relative + u64::from(bytes) > u64::from(section.raw_size) {
                    return Err(refuse(Operation::RvaZeroFill, start));
                }
                if found.is_some() {
                    return Err(refuse(Operation::RvaAmbiguous, start));
                }
                found = Some(u64::from(section.raw) + relative);
            }
        }
        let offset = found.ok_or_else(|| refuse(Operation::RvaUnmapped, start))?;
        if offset + u64::from(bytes) > self.size() {
            return Err(refuse(Operation::RvaFileExtent, offset));
        }
        Ok(offset)
    }
}

fn parse_pe(
    reader: &Reader,
    pe: &mut Pe,
    budget: &mut Budget,
    scratch: &mut Scratch,
) -> Result<()> {
    reader.read(0, &mut scratch.dos, budget)?;
    if &scratch.dos[..2] != b"MZ" {
        return Err(refuse(Operation::DosSignature, 0));
    }
    let nt = u64::from(u32le(&scratch.dos, 60));
    if !(64..=MAX_HEADER_BYTES - 24).contains(&nt) {
        return Err(refuse(Operation::PeOffset, nt));
    }
    reader.read(nt, &mut scratch.coff, budget)?;
    if &scratch.coff[..4] != b"PE\0\0" {
        return Err(refuse(Operation::PeSignature, nt));
    }
    let count = usize::from(u16le(&scratch.coff, 6));
    let optional_size = usize::from(u16le(&scratch.coff, 20));
    if count == 0 || count > MAX_SECTIONS || !(112..=4096).contains(&optional_size) {
        return Err(refuse(Operation::PeHeaderCounts, optional_size as u64));
    }
    if count > MAX_TOTAL_SECTIONS - budget.sections {
        return Err(refuse(Operation::TotalSectionsBudget, count as u64));
    }
    budget.sections += count;
    let section_start = nt + 24 + optional_size as u64;
    if section_start + (count * 40) as u64 > MAX_HEADER_BYTES {
        return Err(refuse(Operation::PeHeaderCap, section_start));
    }
    let optional = &mut scratch.optional[..optional_size];
    reader.read(nt + 24, optional, budget)?;
    if u16le(&scratch.coff, 4) != 0x8664 || u16le(optional, 0) != 0x20b {
        return Err(refuse(
            Operation::PeArchitecture,
            u64::from(u16le(&scratch.coff, 4)),
        ));
    }
    let directories = u32le(optional, 108) as usize;
    if directories > (optional_size - 112) / 8 {
        return Err(refuse(Operation::PeDirectoryCount, directories as u64));
    }
    pe.machine = u16le(&scratch.coff, 4);
    pe.timestamp = u32le(&scratch.coff, 8);
    pe.characteristics = u16le(&scratch.coff, 22);
    pe.magic = u16le(optional, 0);
    pe.entry = u32le(optional, 16);
    pe.preferred_base = u64le(optional, 24);
    pe.image_size = u32le(optional, 56);
    pe.headers_size = u32le(optional, 60);
    pe.checksum = u32le(optional, 64);
    pe.subsystem = u16le(optional, 68);
    pe.dll_characteristics = u16le(optional, 70);
    if pe.image_size == 0
        || u64::from(pe.image_size) > MAX_IMAGE_BYTES
        || u64::from(pe.headers_size) < section_start + (count * 40) as u64
        || u64::from(pe.headers_size) > MAX_HEADER_BYTES
        || pe.headers_size > pe.image_size
        || u64::from(pe.headers_size) > reader.size()
    {
        return Err(refuse(Operation::PeExtent, u64::from(pe.image_size)));
    }
    if matches!(reader, Reader::Memory { .. }) && u64::from(pe.image_size) != reader.size() {
        return Err(refuse(Operation::CensusPeSizeMismatch, reader.size()));
    }
    for i in 0..count {
        reader.read(
            section_start + (i * 40) as u64,
            &mut scratch.section,
            budget,
        )?;
        let section = &scratch.section;
        let item = Section {
            virtual_size: u32le(section, 8),
            rva: u32le(section, 12),
            raw_size: u32le(section, 16),
            raw: u32le(section, 20),
            characteristics: u32le(section, 36),
        };
        if u64::from(item.rva) + u64::from(item.virtual_size.max(item.raw_size))
            > u64::from(pe.image_size)
            || (!matches!(reader, Reader::Memory { .. })
                && u64::from(item.raw) + u64::from(item.raw_size) > reader.size())
        {
            return Err(refuse(Operation::PeSectionExtent, i as u64));
        }
        pe.sections[i] = item;
        pe.section_len += 1;
    }
    if directories <= 6 {
        return Ok(());
    }
    let debug_rva = u32le(optional, 112 + 6 * 8);
    let debug_size = u32le(optional, 116 + 6 * 8);
    if debug_rva == 0 && debug_size == 0 {
        return Ok(());
    }
    if debug_rva == 0
        || debug_size == 0
        || !debug_size.is_multiple_of(28)
        || debug_size as usize / 28 > MAX_DEBUG_ENTRIES
    {
        return Err(refuse(Operation::DebugExtent, u64::from(debug_size)));
    }
    let debug_count = debug_size as usize / 28;
    if debug_count > MAX_TOTAL_DEBUG_ENTRIES - budget.debug_entries {
        return Err(refuse(Operation::TotalDebugBudget, debug_count as u64));
    }
    budget.debug_entries += debug_count;
    let debug_offset = reader.rva_offset(debug_rva, debug_size, pe)?;
    for i in 0..debug_count {
        reader.read(debug_offset + (i * 28) as u64, &mut scratch.debug, budget)?;
        let entry = &scratch.debug;
        pe.debug[i] = DebugEntry {
            kind: u32le(entry, 12),
            size: u32le(entry, 16),
            rva: u32le(entry, 20),
            raw: u32le(entry, 24),
            timestamp: u32le(entry, 4),
            major: u16le(entry, 8),
            minor: u16le(entry, 10),
        };
        pe.debug_len += 1;
        let item = &pe.debug[i];
        if item.kind != 2 {
            continue;
        }
        if item.size < 24 || item.size > MAX_CODEVIEW_BYTES {
            return Err(refuse(Operation::CodeviewSize, u64::from(item.size)));
        }
        let offset = if matches!(reader, Reader::Memory { .. }) {
            if item.rva == 0 {
                return Err(refuse(Operation::CodeviewNotMapped, i as u64));
            }
            reader.rva_offset(item.rva, item.size, pe)?
        } else {
            if item.raw == 0 || u64::from(item.raw) + u64::from(item.size) > reader.size() {
                return Err(refuse(Operation::CodeviewFileExtent, i as u64));
            }
            if item.rva != 0 && reader.rva_offset(item.rva, item.size, pe)? != u64::from(item.raw) {
                return Err(refuse(Operation::CodeviewPointerMismatch, i as u64));
            }
            u64::from(item.raw)
        };
        // Original fixture semantics: validate the declared <=4096-byte extent,
        // read only the 24-byte RSDS/GUID/age header. No PDB string/body is claimed.
        reader.read(offset, &mut scratch.codeview, budget)?;
        let cv = &scratch.codeview;
        if &cv[..4] != b"RSDS" {
            return Err(refuse(
                Operation::CodeviewSignature,
                u64::from(u32le(cv, 0)),
            ));
        }
        let output = &mut pe.codeview[pe.codeview_len as usize];
        output.debug_index = pe.debug_len - 1;
        output.guid.copy_from_slice(&cv[4..20]);
        output.age = u32le(cv, 20);
        pe.codeview_len += 1;
    }
    Ok(())
}

fn file_facts(handle: Handle, facts: &mut FileFacts) -> Result<()> {
    // SAFETY: live owned file handle; non-disk objects are refused.
    let file_type = unsafe { GetFileType(handle) };
    if file_type != 1 {
        let mut error = if file_type == 0 {
            last_error(Operation::FileType)
        } else {
            refuse(Operation::FileType, 0)
        };
        error.detail = u64::from(file_type);
        return Err(error);
    }
    // SAFETY: documented FileIdInfo/FileBasicInfo/FileStandardInfo layouts and
    // exact writable sizes, same live handle throughout this bridge.
    unsafe {
        if GetFileInformationByHandleEx(
            handle,
            18,
            (&raw mut facts.id).cast(),
            u32::try_from(size_of::<FileId>())
                .map_err(|_| refuse(Operation::FileIdInfo, size_of::<FileId>() as u64))?,
        ) == 0
        {
            return Err(last_error(Operation::FileIdInfo));
        }
        if GetFileInformationByHandleEx(
            handle,
            0,
            (&raw mut facts.basic).cast(),
            u32::try_from(size_of::<FileBasic>())
                .map_err(|_| refuse(Operation::FileBasicInfo, size_of::<FileBasic>() as u64))?,
        ) == 0
        {
            return Err(last_error(Operation::FileBasicInfo));
        }
        if GetFileInformationByHandleEx(
            handle,
            1,
            (&raw mut facts.standard).cast(),
            u32::try_from(size_of::<FileStandard>()).map_err(|_| {
                refuse(
                    Operation::FileStandardInfo,
                    size_of::<FileStandard>() as u64,
                )
            })?,
        ) == 0
        {
            return Err(last_error(Operation::FileStandardInfo));
        }
    }
    if facts.standard.eof <= 0
        || facts.standard.eof.cast_unsigned() > MAX_IMAGE_BYTES
        || facts.standard.directory != 0
        || facts.standard.delete_pending != 0
        || facts.basic.attributes & 0x410 != 0
    {
        return Err(refuse(
            Operation::FileState,
            facts.standard.eof.cast_unsigned(),
        ));
    }
    // SAFETY: VOLUME_NAME_NT (2), normalized-name default; fixed output buffer.
    let count = unsafe {
        GetFinalPathNameByHandleW(
            handle,
            facts.final_path.units.as_mut_ptr(),
            u32::try_from(NAME_UNITS)
                .map_err(|_| refuse(Operation::FinalPathCapacity, NAME_UNITS as u64))?,
            2,
        )
    };
    if count == 0 {
        return Err(last_error(Operation::FinalPath));
    }
    facts.final_path.accept(count, Operation::FinalPathCapacity)
}
fn same_file_facts(a: &FileFacts, b: &FileFacts) -> bool {
    // Last access is retained but may legitimately change because of these reads.
    a.id == b.id
        && a.standard == b.standard
        && a.final_path.as_slice() == b.final_path.as_slice()
        && a.basic.creation == b.basic.creation
        && a.basic.write == b.basic.write
        && a.basic.change == b.basic.change
        && a.basic.attributes == b.basic.attributes
}
fn mapped_path(base: usize, path: &mut Name) -> Result<()> {
    // SAFETY: address is passed only to native query, never dereferenced by Rust.
    let count = unsafe {
        K32GetMappedFileNameW(
            GetCurrentProcess(),
            base as Handle,
            path.units.as_mut_ptr(),
            u32::try_from(NAME_UNITS)
                .map_err(|_| refuse(Operation::MappedPathCapacity, NAME_UNITS as u64))?,
        )
    };
    if count == 0 {
        return Err(last_error(Operation::MappedPath));
    }
    path.accept(count, Operation::MappedPathCapacity)
}
fn bridge(
    module: &Module,
    paths: &[u16; TOTAL_CENSUS_PATH_UNITS],
    row: &mut Bridge,
    budget: &mut Budget,
    scratch: &mut Scratch,
    cleanup: &mut [Cleanup; MAX_CLEANUP],
    cleanup_len: &mut u32,
) {
    row.base = module.base;
    let mut handle = null_mut();
    let result = (|| -> Result<()> {
        mapped_path(module.base, &mut row.mapped_path)?;
        let path = module.path(paths);
        scratch.open_path[..path.len()].copy_from_slice(path);
        scratch.open_path[path.len()] = 0;
        // SAFETY: NUL-terminated census path; read-only, share read/write/delete.
        // This sharing is correlation-only, not the parent's prelaunch file pin.
        handle = unsafe {
            CreateFileW(
                scratch.open_path.as_ptr(),
                0x80000000,
                7,
                std::ptr::null(),
                3,
                0x80,
                null_mut(),
            )
        };
        if handle as isize == -1 {
            handle = null_mut();
            return Err(last_error(Operation::CreateFile));
        }
        row.before_attempted = true;
        file_facts(handle, &mut row.before)?;
        row.before_complete = true;
        if row.mapped_path.as_slice() != row.before.final_path.as_slice() {
            return Err(refuse(Operation::MappedFinalMismatch, module.base as u64));
        }
        row.memory_attempted = true;
        parse_pe(
            &Reader::Memory {
                base: module.base,
                size: u64::from(module.size),
            },
            &mut row.memory_pe,
            budget,
            scratch,
        )?;
        row.memory_complete = true;
        row.file_attempted = true;
        parse_pe(
            &Reader::File {
                handle,
                size: row.before.standard.eof.cast_unsigned(),
            },
            &mut row.file_pe,
            budget,
            scratch,
        )?;
        row.file_complete = true;
        if !pe_headers_correlate(&row.memory_pe, &row.file_pe) {
            return Err(refuse(Operation::MemoryFilePeMismatch, module.base as u64));
        }
        mapped_path(module.base, &mut row.mapped_path_after)?;
        if row.mapped_path_after.as_slice() != row.mapped_path.as_slice() {
            return Err(refuse(Operation::MappedPathChanged, module.base as u64));
        }
        Ok(())
    })();
    if let Err(error) = result {
        row.error = error;
    }
    if !handle.is_null() {
        // Final same-handle facts are attempted even after primary failure. At most
        // two cleanup rows per reserved bridge; the 2*MAX_BRIDGES storage cannot
        // fill before its owning bridge cap. No unrelated operation uses it.
        row.after_attempted = true;
        match file_facts(handle, &mut row.after) {
            Ok(()) => {
                row.after_complete = true;
                if !row.before_complete || !same_file_facts(&row.before, &row.after) {
                    row.error
                        .retain(refuse(Operation::SameHandleChanged, module.base as u64));
                }
            }
            Err(error) => {
                row.error.retain(error);
                cleanup[*cleanup_len as usize] = Cleanup {
                    operation: Operation::FilePostObservation,
                    handle: handle as usize,
                    success: false,
                    error,
                };
                *cleanup_len += 1;
            }
        }
        // SAFETY: exactly the successful CreateFile handle owned above. Explicit
        // single close, no retry/Drop. Failed closes remain independently visible.
        let success = unsafe { CloseHandle(handle) } != 0;
        let error = if success {
            refuse(Operation::None, 0)
        } else {
            last_error(Operation::CloseHandle)
        };
        row.close_attempted = true;
        row.close_success = success;
        row.error.retain(error);
        cleanup[*cleanup_len as usize] = Cleanup {
            operation: Operation::CloseHandle,
            handle: handle as usize,
            success,
            error,
        };
        *cleanup_len += 1;
    }
}

fn failure_json(error: &Failure) -> Value {
    if !error.present() {
        return Value::Null;
    }
    json!({
        "operation": error.operation.name(),
        "raw_code": if error.has_code { Some(error.code) } else { None },
        "detail": error.detail,
    })
}
fn wide_hex(units: &[u16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(units.len() * 4);
    for &unit in units {
        for shift in [12, 8, 4, 0] {
            result.push(HEX[((unit >> shift) & 15) as usize] as char);
        }
    }
    result
}
fn bytes_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 15) as usize] as char);
    }
    result
}
fn counter_json(counter: &Counter) -> Value {
    json!({
        "first": counter.first, "last": counter.last,
        "incomplete_bits": counter.incomplete, "pending_sequences": counter.pending(),
        "clean": counter.clean(),
    })
}
fn facts_json(facts: &FileFacts, attempted: bool) -> Value {
    if !attempted {
        return Value::Null;
    }
    json!({
        "volume_serial": facts.id.volume, "file_id_hex": bytes_hex(&facts.id.id),
        "final_nt_path_utf16_hex": wide_hex(facts.final_path.as_slice()),
        "creation_time": facts.basic.creation, "last_access_time": facts.basic.access,
        "last_write_time": facts.basic.write, "change_time": facts.basic.change,
        "attributes": facts.basic.attributes, "allocation_size": facts.standard.allocation,
        "file_size": facts.standard.eof, "links": facts.standard.links,
        "delete_pending": facts.standard.delete_pending, "directory": facts.standard.directory,
    })
}
fn pe_json(pe: &Pe, attempted: bool) -> Value {
    if !attempted {
        return Value::Null;
    }
    let sections: Vec<Value> = pe
        .sections()
        .iter()
        .map(|s| {
            json!({
                "virtual_size": s.virtual_size, "rva": s.rva, "raw_size": s.raw_size,
                "raw_offset": s.raw, "characteristics": s.characteristics,
            })
        })
        .collect();
    let debug: Vec<Value> = pe
        .debug()
        .iter()
        .map(|d| {
            json!({
                "type": d.kind, "size": d.size, "rva": d.rva, "raw_offset": d.raw,
                "timestamp": d.timestamp, "major": d.major, "minor": d.minor,
            })
        })
        .collect();
    let codeview: Vec<Value> = pe
        .codeview()
        .iter()
        .map(|c| {
            json!({
                "debug_index": c.debug_index, "guid_raw_hex": bytes_hex(&c.guid), "age": c.age,
            })
        })
        .collect();
    json!({
        "machine": pe.machine, "timestamp": pe.timestamp,
        "characteristics": pe.characteristics, "magic": pe.magic,
        "entry_rva": pe.entry, "preferred_base": pe.preferred_base,
        "image_size": pe.image_size, "headers_size": pe.headers_size,
        "checksum": pe.checksum, "subsystem": pe.subsystem,
        "dll_characteristics": pe.dll_characteristics, "sections": sections,
        "debug_entries": debug,
        "codeview_status": if pe.codeview_len == 0 { "ABSENT" } else { "PRESENT_RSDS" },
        "codeview": codeview,
    })
}
fn census_json(census: &Census, paths: &[u16; TOTAL_CENSUS_PATH_UNITS]) -> Value {
    let modules: Vec<Value> = census.modules().iter().map(|module| json!({
        "base": module.base, "size": module.size, "path_utf16_hex": wide_hex(module.path(paths)),
    })).collect();
    json!({
        "before": counter_json(&census.before), "after": counter_json(&census.after),
        "needed_bytes": census.needed, "failure": failure_json(&census.error), "modules": modules,
    })
}
fn bridge_json(bridge: &Bridge) -> Value {
    json!({
        "stage": (["initial", "begin", "end"][bridge.stage as usize]),
        "census_index": bridge.census_index, "module_index": bridge.module_index,
        "base": bridge.base,
        "mapped_nt_path_utf16_hex": wide_hex(bridge.mapped_path.as_slice()),
        "mapped_nt_path_after_utf16_hex": wide_hex(bridge.mapped_path_after.as_slice()),
        "file_before": facts_json(&bridge.before, bridge.before_attempted),
        "file_after": facts_json(&bridge.after, bridge.after_attempted),
        "file_before_complete": bridge.before_complete,
        "file_after_complete": bridge.after_complete,
        "memory_pe": pe_json(&bridge.memory_pe, bridge.memory_attempted),
        "file_pe": pe_json(&bridge.file_pe, bridge.file_attempted),
        "memory_pe_complete": bridge.memory_complete, "file_pe_complete": bridge.file_complete,
        "failure": failure_json(&bridge.error),
        "close_success": if bridge.close_attempted { Some(bridge.close_success) } else { None },
    })
}
fn cut_json(cut: [u32; 2]) -> Value {
    json!(cut.map(|i| if i == u32::MAX { None } else { Some(i) }))
}
fn event_json(index: usize, complete: bool) -> Value {
    if !complete {
        return json!({"sequence": index + 1, "complete": false});
    }
    // SAFETY: complete comes only from the frozen snapshot's successful Acquire.
    // The callback never mutates a published slot and storage lasts until exit.
    let p = unsafe { &*P050_WINDOW_RECORDS[index].payload.get() };
    json!({
        "sequence": index + 1, "complete": true, "reason": p.reason, "flags": p.flags,
        "base": p.base, "size": p.size, "invalid_bits": p.invalid,
        "full_name_utf16_hex": wide_hex(&p.full[..p.full_len as usize]),
        "base_name_utf16_hex": wide_hex(&p.basename[..p.base_len as usize]),
    })
}
fn window_unchanged(state: &Storage) -> bool {
    if state.phase != 3
        || !state.begin_counter_present
        || !state.end_counter_present
        || !state.begin_counter.clean()
        || !state.end_counter.clean()
        || state.begin_counter.last != state.end_counter.first
    {
        return false;
    }
    let begin = state.cuts[1][1];
    let end = state.cuts[2][0];
    begin != u32::MAX
        && end != u32::MAX
        && same_modules(
            &state.censuses[begin as usize],
            &state.censuses[end as usize],
            &state.paths,
        )
}
fn limits_json() -> Value {
    json!({
        "modules": MAX_MODULES, "events": MAX_EVENTS, "name_utf16_units": NAME_UNITS,
        "censuses": MAX_CENSUSES, "total_census_path_units": TOTAL_CENSUS_PATH_UNITS,
        "image_file_bytes": MAX_IMAGE_BYTES, "header_bytes": MAX_HEADER_BYTES,
        "sections": MAX_SECTIONS, "debug_entries": MAX_DEBUG_ENTRIES,
        "total_sections": MAX_TOTAL_SECTIONS, "total_debug_entries": MAX_TOTAL_DEBUG_ENTRIES,
        "codeview_bytes": MAX_CODEVIEW_BYTES, "read_bytes": MAX_READ_BYTES, "read_calls": MAX_READ_CALLS,
        "bridges": MAX_BRIDGES, "stdout_bytes": MAX_STDOUT_BYTES,
        "worst_case_stdout_bytes": WORST_CASE_STDOUT_BYTES,
        "cleanup_records": MAX_CLEANUP, "workload_id_bytes": MAX_WORKLOAD_ID_BYTES,
        "fixed_storage_bytes": FIXED_STORAGE_BYTES, "observation_storage_bytes": size_of::<Storage>(),
        "callback_storage_bytes": CALLBACK_STORAGE_BYTES, "owner_bytes": OWNER_BYTES,
        "storage_bytes_scope": "sum_of_static_objects_excludes_linker_padding_code_and_native_cost",
    })
}
fn render(owner: &WindowObserver) -> Value {
    let state = owner.storage.as_deref();
    let primary = state.map_or(&owner.local_failure, |state| &state.primary);
    let censuses: Vec<Value> = state.map_or_else(Vec::new, |state| {
        state.censuses[..state.census_len as usize]
            .iter()
            .map(|c| census_json(c, &state.paths))
            .collect()
    });
    let bridges: Vec<Value> = state.map_or_else(Vec::new, |state| {
        state.bridges[..state.bridge_len as usize]
            .iter()
            .map(bridge_json)
            .collect()
    });
    let cleanup: Vec<Value> = state.map_or_else(Vec::new, |state| {
        state.cleanup[..state.cleanup_len as usize]
            .iter()
            .map(|c| {
                json!({
                    "operation": c.operation.name(), "handle": c.handle,
                    "success": c.success, "failure": failure_json(&c.error),
                })
            })
            .collect()
    });
    let failed_close_handles: Vec<usize> = state.map_or_else(Vec::new, |state| {
        state.cleanup[..state.cleanup_len as usize]
            .iter()
            .filter(|c| c.operation == Operation::CloseHandle && !c.success)
            .map(|c| c.handle)
            .collect()
    });
    let cleanup_ok = state.is_some_and(|state| {
        state.cleanup[..state.cleanup_len as usize]
            .iter()
            .all(|c| c.success)
    });
    let records_complete = state.is_some_and(|state| {
        state.final_counter_present
            && state.final_counter.clean()
            && state.event_complete[..usize::try_from(state.final_counter.first)
                .map_or(MAX_EVENTS, |n| n.min(MAX_EVENTS))]
                .iter()
                .all(|c| *c)
    });
    let complete = !primary.present()
        && records_complete
        && cleanup_ok
        && state.is_some_and(|state| state.phase == 3);
    let events: Vec<Value> = state.map_or_else(Vec::new, |state| {
        if !state.final_counter_present {
            return Vec::new();
        }
        // Keep reservations that raced the snapshot as explicit pending events,
        // not newer publication reads. snapshot_n still refers to first only.
        let count =
            usize::try_from(state.final_counter.last).map_or(MAX_EVENTS, |n| n.min(MAX_EVENTS));
        (0..count)
            .map(|i| event_json(i, state.event_complete[i]))
            .collect()
    });
    let registration = state.map_or_else(
        || json!({"module": 0, "export": 0, "ntstatus": null, "cookie": 0, "unregister_called": false}),
        |state| json!({
            "module": state.registration_module, "export": state.registration_export,
            "ntstatus": if state.registration_attempted { Some(state.registration_status) } else { None },
            "cookie": state.cookie, "unregister_called": false,
        }),
    );
    let cuts = state.map_or_else(
        || json!({"initial": [null, null], "begin": [null, null], "end": [null, null]}),
        |state| json!({
            "initial": cut_json(state.cuts[0]), "begin": cut_json(state.cuts[1]), "end": cut_json(state.cuts[2]),
        }),
    );
    json!({
        "protocol": PROTOCOL, "method": METHOD, "pid": owner.pid,
        "workload_id": if owner.workload_id.len() <= MAX_WORKLOAD_ID_BYTES { Some(owner.workload_id) } else { None },
        "statistical": owner.statistical, "complete": complete,
        "window_unchanged": state.is_some_and(window_unchanged),
        "primary_failure": failure_json(primary), "registration": registration, "cuts": cuts,
        "censuses": censuses, "bridges": bridges, "cleanup": cleanup,
        "cut_counters": {
            "initial": state.filter(|s| s.initial_counter_present).map(|s| counter_json(&s.initial_counter)),
            "begin": state.filter(|s| s.begin_counter_present).map(|s| counter_json(&s.begin_counter)),
            "end": state.filter(|s| s.end_bridge_counter_present).map(|s| counter_json(&s.final_counter)),
        },
        "window_counter": {
            "begin": state.filter(|s| s.begin_counter_present).map(|s| counter_json(&s.begin_counter)),
            "end": state.filter(|s| s.end_counter_present).map(|s| counter_json(&s.end_counter)),
        },
        "final_counter": state.filter(|s| s.final_counter_present).map(|s| counter_json(&s.final_counter)),
        "snapshot_n": state.filter(|s| s.final_counter_present).map(|s| s.final_counter.first),
        "events": events, "records_complete": records_complete,
        "records_complete_scope": "snapshot_n_only",
        "final_owned_cleanup": {
            "ok": cleanup_ok, "remaining_owned_dll": 0, "failed_close_handles": failed_close_handles,
        },
        "limits": limits_json(),
        "read_bytes": state.map_or(0, |s| s.budget.bytes),
        "read_calls": state.map_or(0, |s| s.budget.calls),
        "census_path_units": state.map_or(0, |s| s.budget.paths),
        "total_sections_reserved": state.map_or(0, |s| s.budget.sections),
        "total_debug_entries_reserved": state.map_or(0, |s| s.budget.debug_entries),
        "qualification": false,
        "claim": "conditional_observed_window_and_path_header_correlation_only",
        "backing_object_proof": false, "code_bytes_proof": false,
        "path_comparison": "exact_utf16_no_normalization",
        "owned_cleanup_scope": "file_handles_only",
        "callback_lifetime": "process_exit_no_unregister_no_drain",
        "outer_process_cleanup": "supervisor_only", "serialization_exit_observed": false,
    })
}

// Source-derived x64 repr(C) arithmetic, not an executed layout/working-set claim.
// These assertions deliberately fail a future build if a field changes the bound.
// Static object bytes: 2,434,512 observer + 1,058,828 callback + 1 one-shot flag.
// JSON bound: 7,788,672 < 8,388,608 bytes, compact encoding, observer block only.
const _: () = {
    assert!(OWNER_BYTES == 56 && align_of::<WindowObserver>() == 8);
    assert!(offset_of!(WindowObserver, storage) == 0);
    assert!(offset_of!(WindowObserver, workload_id) == 8);
    assert!(offset_of!(WindowObserver, statistical) == 24);
    assert!(offset_of!(WindowObserver, pid) == 28);
    assert!(offset_of!(WindowObserver, local_failure) == 32);
    assert!(offset_of!(WindowObserver, _thread_bound) == 56);
    assert!(size_of::<Failure>() == 24);
    assert!(size_of::<Counter>() == 2072);
    assert!(size_of::<Name>() == 2052);
    assert!(size_of::<Module>() == 24);
    assert!(size_of::<Census>() == 7248);
    assert!(size_of::<Pe>() == 3512);
    assert!(size_of::<FileFacts>() == 2144);
    assert!(size_of::<Bridge>() == 15480);
    assert!(size_of::<Cleanup>() == 48);
    assert!(size_of::<Scratch>() == 9408);
    assert!(size_of::<Storage>() == 2434512);
    assert!(FIXED_STORAGE_BYTES == 3493341);
    assert!(WORST_CASE_STDOUT_BYTES == 7788672);
};

#[cfg(test)]
mod tests {
    use super::*;

    // cfg(test) is also set for the harness-free benchmark. Only its omitted
    // test bodies leave these helpers unused; the exit-contract harness uses them.

    #[allow(dead_code, reason = "harness = false benchmarks omit test bodies")]
    fn image() -> Vec<u8> {
        let mut bytes = vec![0u8; 1024];
        bytes[..2].copy_from_slice(b"MZ");
        put32(&mut bytes, 60, 64);
        bytes[64..68].copy_from_slice(b"PE\0\0");
        put16(&mut bytes, 68, 0x8664);
        put16(&mut bytes, 70, 1);
        put32(&mut bytes, 72, 0x12345678);
        put16(&mut bytes, 84, 240);
        put16(&mut bytes, 86, 0x2022);
        put16(&mut bytes, 88, 0x20b);
        put32(&mut bytes, 104, 0x1000);
        bytes[112..120].copy_from_slice(&0x180000000u64.to_le_bytes());
        put32(&mut bytes, 144, 8192);
        put32(&mut bytes, 148, 512);
        put32(&mut bytes, 152, 7);
        put16(&mut bytes, 156, 3);
        put16(&mut bytes, 158, 0x8160);
        put32(&mut bytes, 196, 16);
        put32(&mut bytes, 248, 0x1000);
        put32(&mut bytes, 252, 28);
        put32(&mut bytes, 336, 512);
        put32(&mut bytes, 340, 0x1000);
        put32(&mut bytes, 344, 512);
        put32(&mut bytes, 348, 512);
        put32(&mut bytes, 364, 0x60000020);
        put32(&mut bytes, 516, 0x12345678);
        put32(&mut bytes, 524, 2);
        put32(&mut bytes, 528, 24);
        put32(&mut bytes, 532, 0x101c);
        put32(&mut bytes, 536, 540);
        bytes[540..544].copy_from_slice(b"RSDS");
        bytes[544..560].copy_from_slice(&[1; 16]);
        put32(&mut bytes, 560, 3);
        bytes
    }
    #[allow(dead_code, reason = "harness = false benchmarks omit test bodies")]
    fn put16(bytes: &mut [u8], at: usize, value: u16) {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
    #[allow(dead_code, reason = "harness = false benchmarks omit test bodies")]
    fn put32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    #[allow(dead_code, reason = "harness = false benchmarks omit test bodies")]
    fn parse_test(bytes: Vec<u8>) -> (Pe, Budget, Result<()>) {
        // SAFETY: these three bounded POD types contain only zero-valid scalar
        // fields, raw pointers and arrays. No process storage or FFI is used.
        let (mut pe, mut budget, mut scratch): (Pe, Budget, Scratch) =
            unsafe { (std::mem::zeroed(), std::mem::zeroed(), std::mem::zeroed()) };
        let result = parse_pe(&Reader::Bytes(bytes), &mut pe, &mut budget, &mut scratch);
        (pe, budget, result)
    }
    #[allow(dead_code, reason = "harness = false benchmarks omit test bodies")]
    fn parsed() -> Pe {
        let (pe, _, result) = parse_test(image());
        assert!(result.is_ok());
        pe
    }

    #[test]
    fn only_preferred_base_is_excluded_from_correlation() {
        let memory = parsed();
        let mut file = parsed();
        file.preferred_base += 0x10000;
        assert!(pe_headers_correlate(&memory, &file));
        assert_ne!(
            pe_json(&memory, true)["preferred_base"],
            pe_json(&file, true)["preferred_base"]
        );
        let changes: [fn(&mut Pe); 13] = [
            |p| p.machine ^= 1,
            |p| p.timestamp ^= 1,
            |p| p.characteristics ^= 1,
            |p| p.magic ^= 1,
            |p| p.entry ^= 1,
            |p| p.image_size ^= 1,
            |p| p.headers_size ^= 1,
            |p| p.checksum ^= 1,
            |p| p.subsystem ^= 1,
            |p| p.dll_characteristics ^= 1,
            |p| p.sections[0].characteristics ^= 1,
            |p| p.debug[0].timestamp ^= 1,
            |p| p.codeview[0].age ^= 1,
        ];
        for change in changes {
            let mut file = parsed();
            change(&mut file);
            assert!(!pe_headers_correlate(&memory, &file));
        }
    }

    #[test]
    fn parser_retains_partial_debug_on_codeview_pointer_mismatch() {
        let mut bytes = image();
        put32(&mut bytes, 536, 544);
        let (pe, budget, result) = parse_test(bytes);
        assert!(matches!(
            result,
            Err(Failure {
                operation: Operation::CodeviewPointerMismatch,
                ..
            })
        ));
        assert_eq!(pe.section_len, 1);
        assert_eq!(pe.debug_len, 1);
        assert_eq!(pe.debug[0].raw, 544);
        assert_eq!(pe.codeview_len, 0);
        assert_eq!(budget.debug_entries, 1);
    }

    #[test]
    fn parser_rejects_unsupported_signature_and_declared_extent() {
        let mut bytes = image();
        bytes[540..544].copy_from_slice(b"NB10");
        let (_, _, result) = parse_test(bytes);
        assert!(matches!(
            result,
            Err(Failure {
                operation: Operation::CodeviewSignature,
                ..
            })
        ));
        let mut bytes = image();
        put32(&mut bytes, 528, MAX_CODEVIEW_BYTES + 1);
        let (_, _, result) = parse_test(bytes);
        assert!(matches!(
            result,
            Err(Failure {
                operation: Operation::CodeviewSize,
                ..
            })
        ));
        let mut bytes = image();
        put32(&mut bytes, 196, 17);
        let (_, _, result) = parse_test(bytes);
        assert!(matches!(
            result,
            Err(Failure {
                operation: Operation::PeDirectoryCount,
                ..
            })
        ));
    }

    #[test]
    fn file_rva_refuses_overlapping_sections_and_zero_fill() {
        let mut pe = parsed();
        pe.sections[1] = Section {
            virtual_size: 512,
            rva: 0x1000,
            raw_size: 512,
            raw: 512,
            characteristics: 0,
        };
        pe.section_len = 2;
        let reader = Reader::Bytes(image());
        assert!(matches!(
            reader.rva_offset(0x1000, 28, &pe),
            Err(Failure {
                operation: Operation::RvaAmbiguous,
                ..
            })
        ));
        pe.section_len = 1;
        pe.sections[0].raw_size = 16;
        assert!(matches!(
            reader.rva_offset(0x1000, 28, &pe),
            Err(Failure {
                operation: Operation::RvaZeroFill,
                ..
            })
        ));
    }

    #[test]
    fn cumulative_read_budget_refuses_without_reserving_more() {
        let mut budget = Budget {
            bytes: MAX_READ_BYTES,
            calls: 1,
            paths: 0,
            sections: 0,
            debug_entries: 0,
        };
        assert!(budget.read(1).is_err());
        assert_eq!(budget.calls, 1);
        budget.bytes = 0;
        budget.calls = MAX_READ_CALLS;
        assert!(budget.read(1).is_err());
        assert_eq!(budget.bytes, 0);
    }

    #[test]
    fn pending_torn_and_overflow_counters_are_never_clean() {
        // SAFETY: integer-only fixed counter; no callback/static access.
        let mut c: Counter = unsafe { std::mem::zeroed() };
        assert!(c.clean());
        c.first = 1;
        c.last = 1;
        c.pending[0] = 1;
        c.pending_len = 1;
        assert!(!c.clean());
        assert_eq!(counter_json(&c)["pending_sequences"], json!([1]));
        c.pending_len = 0;
        c.last = 2;
        assert!(!c.clean());
        c.first = 2;
        c.incomplete = OVERFLOW;
        assert!(!c.clean());
    }

    #[test]
    fn published_record_rejects_bad_reason_lengths_and_invalid_bits() {
        // SAFETY: integer-only local payload, no callback or static is invoked.
        let mut p: Payload = unsafe { std::mem::zeroed() };
        p.reason = 1;
        p.base = 0x10000;
        p.size = 4096;
        p.full_len = 1;
        p.base_len = 1;
        p.full[0] = b'a' as u16;
        p.basename[0] = b'a' as u16;
        assert!(published_payload_valid(&p));
        p.reason = 3;
        assert!(!published_payload_valid(&p));
        p.reason = 1;
        p.full_len = NAME_UNITS as u32 + 1;
        assert!(!published_payload_valid(&p));
        p.full_len = 1;
        p.invalid = BAD_NAME;
        assert!(!published_payload_valid(&p));
    }

    #[test]
    fn sticky_first_failure_is_not_replaced_by_cleanup_failure() {
        let mut failure = refuse(Operation::MemoryFilePeMismatch, 17);
        failure.retain(refuse(Operation::CloseHandle, 23));
        assert_eq!(
            failure_json(&failure),
            json!({
                "operation": "memory_file_pe_mismatch", "raw_code": null, "detail": 17,
            })
        );
    }

    #[test]
    fn missing_end_retains_the_last_initial_or_begin_cut() {
        for (phase, source) in [(1, SnapshotSource::Initial), (2, SnapshotSource::Begin)] {
            // SAFETY: the same zero-valid storage layout as initialize(), but
            // allocated directly on the test heap, never process-static storage.
            // No registration, native API or callback is invoked by this test.
            let mut allocation = Box::<Storage>::new_uninit();
            let pointer = allocation.as_mut_ptr();
            unsafe {
                pointer.write_bytes(0, 1);
            }
            let mut state = unsafe { allocation.assume_init() };
            state.phase = phase;
            state.snapshot_source = source;
            state.initial_counter_present = true;
            state.begin_counter_present = phase == 2;
            state.cuts = [[0, 1], [u32::MAX; 2], [u32::MAX; 2]];
            let pointer = Box::into_raw(state);
            let owner = WindowObserver {
                // SAFETY: only this owner borrows the allocation. into_report()
                // consumes it and returns owned JSON, after which it is reclaimed.
                storage: Some(unsafe { &mut *pointer }),
                workload_id: "missing-end-regression",
                statistical: true,
                pid: 1,
                local_failure: refuse(Operation::None, 0),
                _thread_bound: PhantomData,
            };
            let report = owner.into_report();
            // SAFETY: no reference to storage survives the consumed owner.
            unsafe {
                drop(Box::from_raw(pointer));
            }
            assert_eq!(
                report["primary_failure"],
                json!({
                    "operation": "observer_missing_cuts", "raw_code": null, "detail": phase,
                })
            );
            assert_eq!(
                report["final_counter"],
                json!({
                    "first": 0, "last": 0, "incomplete_bits": 0,
                    "pending_sequences": [], "clean": true,
                })
            );
            assert_eq!(report["snapshot_n"], 0);
            assert_eq!(report["complete"], false);
            assert_eq!(report["cuts"]["end"], json!([null, null]));
        }
    }

    #[test]
    fn failure_snapshot_preserves_pending_even_after_a_later_clean_observation() {
        // SAFETY: local integer-only counters; this test never invokes a callback,
        // native API, process storage or registration.
        let (mut cut, mut frozen): (Counter, Counter) =
            unsafe { (std::mem::zeroed(), std::mem::zeroed()) };
        cut.first = 2;
        cut.last = 2;
        cut.pending[0] = 2;
        cut.pending_len = 1;
        frozen.copy_snapshot_from(&cut);
        // A later clean counter must not replace the copied failure cut.
        cut.pending_len = 0;
        assert!(cut.clean());
        assert!(!frozen.clean());
        assert_eq!(counter_json(&frozen)["pending_sequences"], json!([2]));
        assert_eq!(frozen.first, 2);
        assert_eq!(frozen.last, 2);
    }
}
