//! Shared bounded loader recording and report core; no live adapter or test fixtures.
//!
//! A load notification precedes dynamic linking, not successful initialization.
//! Checkpoint ordinal brackets do not make a snapshot/history atomic. Earlier
//! history and numerical qualification are never asserted by this report.
//!
//! Every callback context has process lifetime. Its scalar/volatile/lock-free
//! source still requires optimized machine-code inspection for imported helpers,
//! memcpy/memset, panic/runtime calls and stack probes before diagnostic execution.

use std::cell::UnsafeCell;
use std::ffi::c_void;
use std::fmt;
use std::io::{self, Write};
use std::mem::{align_of, size_of};
use std::ptr::{addr_of, addr_of_mut};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use super::ocr_image_path;

const EVENT_LIMIT: u32 = 256;
pub(crate) const PATH_UNITS: usize = 2_048;
const CLOSED: u32 = 1 << 31;
const ADMISSION_OVERFLOW: u32 = 1 << 30;
const WRITERS: u32 = ADMISSION_OVERFLOW - 1;

#[derive(Debug)]
pub(crate) enum Failure {
    Rule(&'static str),
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rule(rule) => write!(formatter, "rule={rule}"),
        }
    }
}

impl std::error::Error for Failure {}

// SDK LDR_DLL_{LOADED,UNLOADED}_NOTIFICATION_DATA have the same layout. Keep
// the two union alternatives explicit, and never copy either aggregate in the
// callback. The ABI below is deliberately restricted to the Windows x64 driver.
#[repr(C)]
pub(crate) struct UnicodeString {
    pub(crate) length: u16,
    pub(crate) maximum_length: u16,
    pub(crate) buffer: *const u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct NotificationPayload {
    pub(crate) flags: u32,
    pub(crate) full_dll_name: *const UnicodeString,
    pub(crate) base_dll_name: *const UnicodeString,
    pub(crate) dll_base: *mut c_void,
    pub(crate) size_of_image: u32,
}

#[repr(C)]
pub(crate) union NotificationData {
    pub(crate) loaded: NotificationPayload,
    unloaded: NotificationPayload,
}

const _: () = {
    assert!(size_of::<usize>() == 8);
    assert!(size_of::<UnicodeString>() == 16);
    assert!(size_of::<NotificationPayload>() == 40);
    assert!(size_of::<NotificationData>() == 40);
};

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

pub(crate) struct Storage {
    gate: AtomicU32,
    observed: AtomicU32,
    overflow: AtomicBool,
    malformed: AtomicBool,
    slots: UnsafeCell<[Slot; EVENT_LIMIT as usize]>,
}

// SAFETY: An admitted writer reserves one unique, never-reused ordinal. Slot
// mutation is exclusive to that writer. Readers first require CLOSED and zero
// admitted writers with an acquire load; writer release-RMWs publish every slot.
// Late callbacks can touch the permanent admission atomic but cannot touch slots.
// Neither this static storage nor synthetic static storage is ever freed/reset.
unsafe impl Sync for Storage {}

impl Storage {
    pub(crate) const fn new() -> Self {
        Self {
            gate: AtomicU32::new(0),
            observed: AtomicU32::new(0),
            overflow: AtomicBool::new(false),
            malformed: AtomicBool::new(false),
            slots: UnsafeCell::new([const { Slot::new() }; EVENT_LIMIT as usize]),
        }
    }

    #[inline(always)]
    pub(crate) fn admit(&self) -> bool {
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

    /// Ends one admitted writer's access and publishes its completed writes.
    ///
    /// # Safety
    /// The caller owns one successful, unmatched admission and has finished all
    /// accesses to that admission's slot. It must release that admission once.
    #[inline(always)]
    pub(crate) unsafe fn release(&self) {
        self.gate.fetch_sub(1, Ordering::Release);
    }

    pub(crate) fn observed(&self) -> u32 {
        self.observed.load(Ordering::Acquire)
    }

    pub(crate) fn close(&self) {
        self.gate.fetch_or(CLOSED, Ordering::AcqRel);
    }

    fn drained(&self) -> bool {
        let state = self.gate.load(Ordering::Acquire);
        state & CLOSED != 0 && state & WRITERS == 0
    }

    pub(crate) fn drain(&self, limit: Duration) -> bool {
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
        let count = self.observed.load(Ordering::Relaxed).min(EVENT_LIMIT);
        let mut events = Vec::with_capacity(count as usize);
        for index in 0..count {
            // SAFETY: close+acquire-drain above excludes all slot writers,
            // including callbacks entering late. Storage is never reopened.
            let slot = unsafe { &*self.slots.get().cast::<Slot>().add(index as usize) };
            if slot.committed == 0 {
                continue;
            }
            if slot.length == 0 || slot.length as usize > PATH_UNITS {
                self.malformed.store(true, Ordering::Relaxed);
                continue;
            }
            let Ok(path) = ocr_image_path::validated_path(&slot.path[..slot.length as usize])
            else {
                self.malformed.store(true, Ordering::Relaxed);
                continue;
            };
            events.push(Event {
                ordinal: index + 1,
                kind: slot.kind,
                base: slot.base,
                size: slot.size,
                path,
            });
        }
        Ok(events)
    }
}

// This context and its entire slot arena have process lifetime, regardless of
// registration, unregister, drain, output, panic, or watchdog outcomes.
/// # Safety
/// Context points to process-lifetime Storage. Non-null notification objects and
/// their counted string buffers have valid ABI extents for the entire callback.
#[inline(never)]
#[unsafe(no_mangle)]
pub(crate) unsafe extern "system" fn mado_pilot_ocr_loader_notification(
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
        if ordinal > EVENT_LIMIT {
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
    // SAFETY: This callback owns the successful admission above, has finished
    // every slot access, and releases that admission exactly once.
    unsafe { storage.release() };
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
        || !data.addr().is_multiple_of(align_of::<NotificationData>())
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
            || !name.addr().is_multiple_of(align_of::<UnicodeString>())
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
            || usize::from(bytes) / 2 > PATH_UNITS
            || source.is_null()
            || !source.addr().is_multiple_of(align_of::<u16>())
        {
            return false;
        }
        let length = bytes / 2;
        let index = ordinal.wrapping_sub(1) as usize;
        let slot = storage.slots.get().cast::<Slot>().add(index);
        let destination = addr_of_mut!((*slot).path).cast::<u16>();
        let mut unit = 0_usize;
        while unit < usize::from(length) {
            destination
                .add(unit)
                .write_volatile(source.add(unit).read_volatile());
            unit = unit.wrapping_add(1);
        }
        addr_of_mut!((*slot).kind).write_volatile(reason);
        addr_of_mut!((*slot).base).write_volatile(base.addr() as u64);
        addr_of_mut!((*slot).size).write_volatile(size);
        addr_of_mut!((*slot).length).write_volatile(u32::from(length));
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

pub(crate) struct Bounded<W> {
    // The live adapter needs the owned file for its separate final sync.
    pub(crate) inner: W,
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
        self.inner
            .flush()
            .inspect_err(|_| self.failure = Some("trace-flush"))
    }
}

pub(crate) struct Report<W> {
    pub(crate) output: Bounded<W>,
    checkpoints: usize,
}

impl<W: Write> Report<W> {
    pub(crate) fn new(inner: W, limit: usize) -> Result<Self, Failure> {
        let mut report = Self {
            output: Bounded {
                inner,
                written: 0,
                limit,
                failure: None,
            },
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

    pub(crate) fn checkpoint<'a>(
        &mut self,
        name: &str,
        before: u32,
        after: u32,
        images: impl IntoIterator<Item = (u64, &'a str)>,
    ) -> Result<(), Failure> {
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
        for (index, (base, path)) in images.into_iter().enumerate() {
            if index != 0 {
                self.bytes(b",")?;
            }
            self.bytes(b"{\"base\":")?;
            self.number(base)?;
            self.bytes(b",\"path\":")?;
            self.string(path)?;
            self.bytes(b"}")?;
        }
        self.bytes(b"]}")?;
        self.checkpoints += 1;
        Ok(())
    }

    pub(crate) fn finish(
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
        let events = if drained {
            storage.events()?
        } else {
            Vec::new()
        };
        let gate = storage.gate.load(Ordering::Acquire);
        let closed = gate & CLOSED != 0;
        let overflow = storage.overflow.load(Ordering::Relaxed) || gate & ADMISSION_OVERFLOW != 0;
        let malformed = storage.malformed.load(Ordering::Relaxed);
        let unregistered = status == Some(0);
        let error = failure
            .or(if self.checkpoints == 2 {
                None
            } else {
                Some("checkpoints-incomplete")
            })
            .or(if closed {
                None
            } else {
                Some("admission-not-closed")
            })
            .or(if drained {
                None
            } else {
                Some("callbacks-not-drained")
            })
            .or(if unregistered {
                None
            } else {
                Some("unregister-failed")
            })
            .or(if overflow {
                Some("notification-overflow")
            } else {
                None
            })
            .or(if malformed {
                Some("notification-malformed")
            } else {
                None
            });
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
