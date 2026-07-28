//! What every entry does before, during, and after its own work.
//!
//! Four rules are implemented once here rather than fifty times in the entries:
//! a panic never crosses into C, an output is in its documented failure state
//! before anything is validated, a size-versioned structure is read and written
//! only within the size its owner declared, and every count, stride, and span is
//! checked before it becomes an address calculation.

use std::panic::{self, AssertUnwindSafe};

use crate::error::Fault;
use crate::status::{MADOPILOT_STATUS_INTERNAL_PANIC, madopilot_status_t};

/// Runs one table entry with a panic containment fence around it.
///
/// A contained panic returns [`MADOPILOT_STATUS_INTERNAL_PANIC`]. Every valid
/// output is already in its failure state by then, because entries initialize
/// outputs first and reach [`crate::hooks::Site::Entry`] only afterwards, and
/// every temporary the unwinding body had allocated is released by the unwind
/// itself.
///
/// Nothing is poisoned. The entries hold no state across calls that a panic
/// could leave half-written: an owned handle is either published through its
/// output or dropped by the unwind, and the facade objects behind the handles
/// are reached through shared references that a panic cannot have mutated.
pub(crate) fn boundary(body: impl FnOnce() -> madopilot_status_t) -> madopilot_status_t {
    // `AssertUnwindSafe` is the honest assertion here rather than a way around
    // the check: the body's captures are raw pointers into caller memory, which
    // carry no unwind-safety information at all, so the property has to be
    // argued from the paragraph above instead of proven by the bound.
    match panic::catch_unwind(AssertUnwindSafe(body)) {
        Ok(status) => status,
        Err(_) => MADOPILOT_STATUS_INTERNAL_PANIC,
    }
}

/// A size-versioned structure the library writes.
pub(crate) trait Versioned: Copy {
    /// The smallest `struct_size` that describes a usable structure.
    const MANDATORY: usize;
    /// The structure's C name, for diagnostics.
    const NAME: &'static str;

    /// The documented failure state, written before anything is validated.
    fn failure(struct_size: u32) -> Self;
}

/// A size-versioned structure the caller supplies.
pub(crate) trait Input: Copy {
    /// The smallest `struct_size` that describes a usable request.
    const MANDATORY: usize;
    /// The structure's C name, for diagnostics.
    const NAME: &'static str;

    /// Every field at its documented default, for the fields a shorter prefix
    /// omits.
    fn defaults() -> Self;
}

/// A validated, failure-initialized output structure.
#[derive(Debug)]
pub(crate) struct Out<S: Versioned> {
    ptr: *mut S,
    size: usize,
}

impl<S: Versioned> Out<S> {
    /// Validates `ptr` and writes the documented failure state through it.
    ///
    /// # Errors
    ///
    /// Rejects null, a misaligned address, and a `struct_size` below the
    /// structure's mandatory prefix. Nothing beyond `struct_size` is read, and
    /// nothing at all is read when the address is rejected.
    ///
    /// # Safety
    ///
    /// `ptr` must be null or point at `struct_size` writable bytes, with its
    /// first four bytes already set to that size, for the duration of the call.
    pub(crate) unsafe fn begin(ptr: *mut S) -> Result<Self, Fault> {
        if ptr.is_null() {
            return Err(Fault::abi(format!("the {} output is null", S::NAME)));
        }
        if !ptr.addr().is_multiple_of(align_of::<S>()) {
            return Err(Fault::abi(format!(
                "the {} output is not aligned to {} bytes",
                S::NAME,
                align_of::<S>()
            )));
        }

        // `struct_size` is the first field of every versioned structure, so it
        // is readable before anything else about the structure is known.
        //
        // SAFETY: the pointer is non-null and correctly aligned for `S`, whose
        // first field is a `u32`, and the caller contract requires the
        // structure to be readable for the call.
        let declared = unsafe { ptr.cast::<u32>().read() } as usize;
        if declared < S::MANDATORY {
            return Err(Fault::abi(format!(
                "the {} output declares {declared} bytes, below its {} byte mandatory prefix",
                S::NAME,
                S::MANDATORY
            )));
        }

        // A caller built against a newer header gets what this library knows.
        // Reporting its own larger size back would claim trailing bytes are
        // populated when they are not.
        let size = declared.min(size_of::<S>());
        let out = Self { ptr, size };
        // SAFETY: `size` is within the caller's declared writable extent.
        unsafe { out.write(S::failure(narrow(size))) };

        Ok(out)
    }

    /// Returns the number of bytes this output actually covers.
    pub(crate) const fn declared_size(&self) -> u32 {
        narrow(self.size)
    }

    /// Writes the successful value, within the covered prefix.
    ///
    /// # Safety
    ///
    /// The output must still be the writable structure [`Out::begin`] accepted.
    pub(crate) unsafe fn commit(&self, value: S) {
        // SAFETY: forwarded unchanged from this function's own contract.
        unsafe { self.write(value) }
    }

    /// # Safety
    ///
    /// As [`Out::commit`].
    unsafe fn write(&self, value: S) {
        // SAFETY: `self.size` is at most `size_of::<S>()` and at most the
        // caller's declared size, the source is a live local, and the two
        // ranges cannot overlap because the local is on this frame.
        unsafe {
            std::ptr::copy_nonoverlapping(
                (&raw const value).cast::<u8>(),
                self.ptr.cast::<u8>(),
                self.size,
            );
        }
    }
}

/// Reads a caller-supplied structure, defaulting the fields it omitted.
///
/// # Errors
///
/// Rejects null, a misaligned address, and a `struct_size` below the
/// structure's mandatory prefix. Trailing bytes the library does not recognize
/// are ignored.
///
/// # Safety
///
/// `ptr` must point at `struct_size` readable bytes, with its first four bytes
/// set to that size, for the duration of the call.
pub(crate) unsafe fn read_input<S: Input>(ptr: *const S) -> Result<S, Fault> {
    if ptr.is_null() {
        return Err(Fault::abi(format!("the {} argument is null", S::NAME)));
    }
    if !ptr.addr().is_multiple_of(align_of::<S>()) {
        return Err(Fault::abi(format!(
            "the {} argument is not aligned to {} bytes",
            S::NAME,
            align_of::<S>()
        )));
    }

    // SAFETY: the pointer is non-null and correctly aligned for `S`, whose
    // first field is a `u32`, and the caller contract requires the structure to
    // be readable for the call.
    let declared = unsafe { ptr.cast::<u32>().read() } as usize;
    if declared < S::MANDATORY {
        return Err(Fault::abi(format!(
            "the {} argument declares {declared} bytes, below its {} byte mandatory prefix",
            S::NAME,
            S::MANDATORY
        )));
    }

    let copied = declared.min(size_of::<S>());
    let mut value = S::defaults();
    // SAFETY: `copied` is at most the caller's declared readable extent and at
    // most `size_of::<S>()`, and `value` is a live local that cannot overlap it.
    unsafe {
        std::ptr::copy_nonoverlapping(ptr.cast::<u8>(), (&raw mut value).cast::<u8>(), copied);
    }

    Ok(value)
}

/// Validates an owned-handle output and sets it to its failure state, null.
///
/// # Errors
///
/// Rejects null and a misaligned address.
///
/// # Safety
///
/// `ptr` must be null or a writable, correctly aligned address for the call.
pub(crate) unsafe fn begin_handle_out<T>(
    ptr: *mut *mut T,
    name: &'static str,
) -> Result<(), Fault> {
    if ptr.is_null() {
        return Err(Fault::abi(format!("the {name} output is null")));
    }
    if !ptr.addr().is_multiple_of(align_of::<*mut T>()) {
        return Err(Fault::abi(format!("the {name} output is not aligned")));
    }

    // SAFETY: the pointer is non-null and correctly aligned, and the caller
    // contract requires it to be writable for the call.
    unsafe { ptr.write(std::ptr::null_mut()) };

    Ok(())
}

/// Validates an optional error output and sets it to its failure state, null.
///
/// # Errors
///
/// Rejects a misaligned address. Null is permitted and means the caller wants
/// the status only.
///
/// # Safety
///
/// As [`begin_handle_out`].
pub(crate) unsafe fn begin_error_out<T>(ptr: *mut *mut T) -> Result<(), Fault> {
    if ptr.is_null() {
        return Ok(());
    }

    // SAFETY: forwarded unchanged from this function's own contract.
    unsafe { begin_handle_out(ptr, "out_error") }
}

/// Initializes a required owned-handle output and an optional error output.
///
/// The outputs are independent: every valid output is set to null even when the
/// other output is invalid. When both are invalid, the primary output's fault
/// takes precedence.
///
/// # Errors
///
/// Rejects a null or misaligned primary output and a misaligned error output.
///
/// # Safety
///
/// Each output must independently satisfy the contract of
/// [`begin_handle_out`] or [`begin_error_out`].
pub(crate) unsafe fn begin_handle_and_error_out<T, E>(
    out_handle: *mut *mut T,
    handle_name: &'static str,
    out_error: *mut *mut E,
) -> Result<(), Fault> {
    // Evaluate both initializers before combining their results so a fault in
    // one output cannot short-circuit initialization of the other.
    // SAFETY: forwarded unchanged from this function's own contract.
    let handle = unsafe { begin_handle_out(out_handle, handle_name) };
    // SAFETY: as above.
    let error = unsafe { begin_error_out(out_error) };

    handle.and(error)
}

/// Validates a scalar output and sets it to `failure`.
///
/// # Errors
///
/// Rejects null and a misaligned address.
///
/// # Safety
///
/// As [`begin_handle_out`].
pub(crate) unsafe fn begin_scalar_out<T: Copy>(
    ptr: *mut T,
    name: &'static str,
    failure: T,
) -> Result<(), Fault> {
    if ptr.is_null() {
        return Err(Fault::abi(format!("the {name} output is null")));
    }
    if !ptr.addr().is_multiple_of(align_of::<T>()) {
        return Err(Fault::abi(format!("the {name} output is not aligned")));
    }

    // SAFETY: the pointer is non-null and correctly aligned, and the caller
    // contract requires it to be writable for the call.
    unsafe { ptr.write(failure) };

    Ok(())
}

/// Writes a validated scalar output.
///
/// # Safety
///
/// `ptr` must still be the address [`begin_scalar_out`] accepted.
pub(crate) unsafe fn commit_scalar<T: Copy>(ptr: *mut T, value: T) {
    // SAFETY: forwarded unchanged from this function's own contract.
    unsafe { ptr.write(value) }
}

/// Returns the byte span `count` elements of `stride` occupy.
///
/// # Errors
///
/// Rejects a stride below the element's mandatory prefix, and any product that
/// is not a representable object size. No allocation or address calculation is
/// performed from a value that fails here.
pub(crate) fn span(
    count: usize,
    stride: usize,
    mandatory: usize,
    name: &'static str,
) -> Result<usize, Fault> {
    if count == 0 {
        return Ok(0);
    }
    if stride < mandatory {
        return Err(Fault::abi(format!(
            "`{name}` declares a {stride} byte element stride, below the {mandatory} byte mandatory prefix"
        )));
    }

    let total = count.checked_mul(stride).ok_or_else(|| {
        Fault::abi(format!(
            "`{name}` spans {count} elements of {stride} bytes, which overflows"
        ))
    })?;
    if total > isize::MAX.unsigned_abs() {
        return Err(Fault::abi(format!(
            "`{name}` spans {total} bytes, which is not a representable object size"
        )));
    }

    Ok(total)
}

/// Rejects an index at or beyond `count`.
///
/// # Errors
///
/// Returns invalid argument, which is what an out-of-range indexed accessor
/// reports after initializing its output to the failure state.
pub(crate) fn index_within(index: usize, count: usize, name: &'static str) -> Result<usize, Fault> {
    if index >= count {
        return Err(Fault::abi(format!(
            "`{name}` index {index} is not below the count {count}"
        )));
    }

    Ok(index)
}

/// Narrows a structure size, which is always small enough to fit.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the branch above is the guard against the only value that could truncate"
)]
const fn narrow(size: usize) -> u32 {
    // Every versioned structure is a few dozen bytes, and `Out::begin` clamps to
    // `size_of`, so this cannot saturate for any type this module accepts.
    if size > u32::MAX as usize {
        u32::MAX
    } else {
        size as u32
    }
}
