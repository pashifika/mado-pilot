//! What every entry does before, during, and after its own work.
//!
//! Four rules are implemented once here rather than fifty times in the entries:
//! a panic never crosses into C, an output is in its documented failure state
//! before anything is validated, a size-versioned structure is read and written
//! only within the size its owner declared, and every count, stride, and span is
//! checked before it becomes an address calculation.
//!
//! # What "within the declared size" means for an input
//!
//! A declared size describes a prefix, so it has to end where a prefix can end.
//! Three separate declarations have to agree before a caller's structure is
//! read: the size must land on one of the structure's own field boundaries, it
//! must not exceed the element stride of an array it was read out of, and it
//! must cover every field a presence bit claims is set. A size that breaks any
//! of them describes a structure that cannot exist, and reading it would mix
//! caller bytes with [`Input::defaults`] inside one field — for a pointer field,
//! into an address the caller never passed.
//!
//! The third of those is only as complete as [`Input::PRESENCE`], so the bit a
//! caller sets is read through [`declared!`] rather than out of the flags field
//! directly. That macro will not compile for a bit the table omits, which puts
//! the obligation at the site that honors the bit instead of leaving it to
//! whoever next appends a field.

use std::panic::{self, AssertUnwindSafe};

use crate::error::Fault;
use crate::layout::{LAYOUT, TypeLayout};
use crate::madopilot_error_t;
use crate::status::{MADOPILOT_STATUS_INTERNAL_PANIC, madopilot_status_t};
use crate::types::{
    madopilot_find_request_t, madopilot_map_request_t, madopilot_match_options_t,
    madopilot_open_request_t, madopilot_operation_t,
};

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

/// Builds an input structure's legal-prefix list out of its own fields.
///
/// The offsets come from the compiler rather than from a written table, so a
/// field that moves takes the list with it. What a reader still has to check is
/// that every field is named, which [`check_input_tables`] proves against the
/// same layout report the C probe is diffed against.
macro_rules! prefixes {
    ($ty:ty $(, $field:ident)+ $(,)?) => {
        &[$(std::mem::offset_of!($ty, $field),)+ size_of::<$ty>()]
    };
}

pub(crate) use prefixes;

/// The smallest prefix that covers one field: where it starts, plus how big it
/// is.
///
/// Written as the field's own type rather than as the next field's offset so
/// that appending a field cannot change what an existing presence bit requires.
macro_rules! covers {
    ($ty:ty, $field:ident: $field_ty:ty) => {
        std::mem::offset_of!($ty, $field) + size_of::<$field_ty>()
    };
}

pub(crate) use covers;

/// Whether a presence table declares `bit`.
///
/// A `const fn` so that [`declared!`] can answer the question while the caller
/// of that macro is being compiled.
pub(crate) const fn declares(table: &[(u32, usize)], bit: u32) -> bool {
    let mut index = 0;
    while index < table.len() {
        if table[index].0 == bit {
            return true;
        }
        index += 1;
    }

    false
}

/// Reads one presence bit out of a structure a caller supplied.
///
/// The bit has to appear in that structure's [`Input::PRESENCE`] table, and the
/// requirement is enforced here — where the bit is honored — rather than only
/// where the table is written. A bit the table omits is a bit [`read_input`]
/// cannot refuse for a prefix that stops short of the field it names, so
/// honoring it would apply [`Input::defaults`] under the caller's own claim to
/// have supplied a value. The check is a `const` block, so the omission is a
/// compile error at the site that would have made the mistake.
macro_rules! declared {
    ($value:expr, $ty:ty, $bit:expr $(,)?) => {{
        const {
            // Honoring a bit reads the same table the checks are about, so the
            // structure's tables are held to being consistent here too. This
            // block is written against a named type rather than a type
            // parameter, so it is evaluated when this site is compiled — no
            // caller has to reach the site for the check to run.
            $crate::boundary::check_input_tables::<$ty>();

            assert!(
                $crate::boundary::declares(<$ty as $crate::boundary::Input>::PRESENCE, $bit),
                concat!(
                    stringify!($bit),
                    " is honored for ",
                    stringify!($ty),
                    " but is not in its `Input::PRESENCE` table, so a prefix that stops short of \
                     the field it names would be defaulted instead of refused",
                ),
            );
        }

        // Naming the type binds the table that was checked and the bits that are
        // read to one structure, so the two can never come from different ones.
        let supplied: &$ty = &$value;
        $crate::boundary::Input::presence_bits(supplied) & $bit != 0
    }};
}

pub(crate) use declared;

/// A structure that reached [`Input`] through [`inputs!`].
///
/// [`Input`] requires it and nothing but [`inputs!`] implements it, so an
/// implementation written any other way does not compile — whatever way its
/// header happens to be spelled, on one line or several, qualified or not.
///
/// The requirement exists so that the checks an input structure's tables have
/// to pass arrive with the implementation rather than only when something reads
/// the structure. What those checks stand between is a presence bit the
/// boundary cannot refuse and a caller's prefix read as though it covered a
/// field it stops short of.
#[diagnostic::on_unimplemented(
    message = "`{Self}` implements `Input` without registering as one",
    label = "not registered",
    note = "write the implementation inside `inputs!`, which registers the structure and writes the checks its tables have to pass"
)]
pub(crate) trait Registered: Copy {}

/// Implements [`Input`] for each structure named, and registers it.
///
/// Registration is not a second list to keep in step with the first: the
/// implementations are the list. Each invocation emits the implementations it
/// was given, unchanged, and beside each of them a `const` item that runs
/// [`check_input_tables`] for that structure. A `const` item is evaluated
/// wherever it is written — at module scope, inside a function body, in code no
/// test and no caller ever reaches — so implementing the trait and being
/// checked are one act, and no invocation of this macro can be positioned
/// somewhere its checks do not run.
macro_rules! inputs {
    ($(
        $(#[$attr:meta])*
        impl Input for $ty:ty { $($body:tt)* }
    )+) => {
        $(
            impl $crate::boundary::Registered for $ty {}

            $(#[$attr])*
            impl $crate::boundary::Input for $ty { $($body)* }

            const _: () = $crate::boundary::check_input_tables::<$ty>();
        )+
    };
}

pub(crate) use inputs;

/// A size-versioned structure the caller supplies.
pub(crate) trait Input: Registered {
    /// The smallest `struct_size` that describes a usable request.
    const MANDATORY: usize;
    /// The structure's C name, for diagnostics.
    const NAME: &'static str;
    /// Every size a caller's prefix may end at: each field's offset, in
    /// declaration order, and then the whole structure.
    ///
    /// Built with [`prefixes!`]. A `struct_size` between two of these ends
    /// inside a field, which is the one thing a prefix may not do — the field
    /// would be neither covered nor omitted, but half caller bytes and half
    /// [`Input::defaults`].
    const PREFIXES: &'static [usize];
    /// Each presence bit this structure defines, and the smallest `struct_size`
    /// that covers the field the bit names.
    ///
    /// Empty for a structure that defines no presence bits. A bit set for a
    /// field the declared prefix does not reach is refused rather than honored:
    /// honoring it would apply [`Input::defaults`] under the caller's own name
    /// for a deliberate value, which is the difference between "use the
    /// documented default" and "use zero".
    ///
    /// A bit missing from this table is the failure the table exists to
    /// prevent, so nothing may honor a bit without listing it: every read goes
    /// through [`declared!`], which does not compile otherwise.
    const PRESENCE: &'static [(u32, usize)];

    /// Every field at its documented default, for the fields a shorter prefix
    /// omits.
    fn defaults() -> Self;

    /// The presence bits this value carries.
    ///
    /// Zero for a structure whose second field is a discriminant rather than a
    /// bit set; its [`Input::PRESENCE`] table is empty, so the value is never
    /// consulted.
    fn presence_bits(&self) -> u32;
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
/// Rejects null, a misaligned address, a `struct_size` below the structure's
/// mandatory prefix, a `struct_size` that ends inside a field, and a presence
/// bit naming a field the declared size does not cover. Trailing bytes the
/// library does not recognize are ignored.
///
/// # Safety
///
/// `ptr` must point at `struct_size` readable bytes, with its first four bytes
/// set to that size, for the duration of the call.
pub(crate) unsafe fn read_input<S: Input>(ptr: *const S) -> Result<S, Fault> {
    // SAFETY: forwarded unchanged from this function's own contract.
    unsafe { read(ptr, None) }
}

/// Reads one element of a caller-declared array, bounded by its element stride.
///
/// The element's own `struct_size` and the array's element stride are two
/// independent caller declarations of the same extent. An element that claims
/// to be larger than the stride it sits at describes an array that cannot
/// exist, and believing the larger of the two would read past the extent the
/// caller declared for the array as a whole.
///
/// # Errors
///
/// As [`read_input`], and rejects an element whose `struct_size` is above
/// `stride`.
///
/// # Safety
///
/// `ptr` must point at `stride` readable bytes, with its first four bytes set
/// to the element's own size, for the duration of the call.
pub(crate) unsafe fn read_element<S: Input>(ptr: *const S, stride: usize) -> Result<S, Fault> {
    // SAFETY: forwarded unchanged from this function's own contract.
    unsafe { read(ptr, Some(stride)) }
}

/// # Safety
///
/// As [`read_input`], except that a supplied `stride` is the readable extent
/// instead of the structure's own declared size.
unsafe fn read<S: Input>(ptr: *const S, stride: Option<usize>) -> Result<S, Fault> {
    // Nothing at run time: this is where every rule below reads `S`'s tables,
    // so it is where the tables are held to being consistent. Instantiating
    // this function for a structure evaluates the block, which makes an
    // unchecked table a compile error at the first site that would have relied
    // on it — including for an implementation written outside `inputs!`.
    const { check_input_tables::<S>() };

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
    if let Some(stride) = stride
        && declared > stride
    {
        return Err(Fault::abi(format!(
            "a {} element declares {declared} bytes, above the {stride} byte element stride of the array it is read from",
            S::NAME
        )));
    }
    // A size at or above this build's own is a newer caller, whose extra fields
    // are the trailing bytes this build ignores. Only a size inside the
    // structure this build knows has to land on one of its boundaries.
    if declared < size_of::<S>() && !S::PREFIXES.contains(&declared) {
        return Err(Fault::abi(format!(
            "the {} argument declares {declared} bytes, which ends inside a field rather than at a field boundary",
            S::NAME
        )));
    }

    let copied = declared.min(size_of::<S>());
    let mut value = S::defaults();
    // SAFETY: `copied` is at most the caller's declared readable extent and at
    // most `size_of::<S>()`, and `value` is a live local that cannot overlap it.
    unsafe {
        std::ptr::copy_nonoverlapping(ptr.cast::<u8>(), (&raw mut value).cast::<u8>(), copied);
    }

    // Every mandatory prefix reaches at least the second field, so the bits are
    // populated here for every structure that defines any.
    let bits = value.presence_bits();
    for &(bit, required) in S::PRESENCE {
        if bits & bit != 0 && declared < required {
            return Err(Fault::abi(format!(
                "the {} argument sets presence bit {bit:#x} for a field its {declared} byte prefix does not reach, which needs {required} bytes",
                S::NAME
            )));
        }
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

/// Initializes a required owned-handle output and an optional error output, and
/// reports a fault through the error output when that output can carry one.
///
/// The outputs are independent: every valid output is set to null even when the
/// other output is invalid. When both are invalid, the primary output's fault
/// takes precedence.
///
/// A rejected output argument is an invalid argument like any other, so it is
/// described the same way. Saying which output was null or misaligned is the
/// whole diagnostic — `MADOPILOT_STATUS_INVALID_ARGUMENT` alone does not name it
/// — and a caller that passed a usable `out_error` asked for exactly that. Only
/// a caller whose error output is itself unusable gets the status alone, because
/// there is then nowhere to put the message.
///
/// # Errors
///
/// Returns the status to report for a null or misaligned primary output and for
/// a misaligned error output, having already emitted the detail where it could.
///
/// # Safety
///
/// Each output must independently satisfy the contract of
/// [`begin_handle_out`] or [`begin_error_out`].
pub(crate) unsafe fn begin_outputs<T>(
    out_handle: *mut *mut T,
    handle_name: &'static str,
    out_error: *mut *mut madopilot_error_t,
) -> Result<(), madopilot_status_t> {
    // Evaluate both initializers before combining their results so a fault in
    // one output cannot short-circuit initialization of the other.
    // SAFETY: forwarded unchanged from this function's own contract.
    let handle = unsafe { begin_handle_out(out_handle, handle_name) };
    // SAFETY: as above.
    let error = unsafe { begin_error_out(out_error) };

    match (handle, error) {
        (Ok(()), Ok(())) => Ok(()),
        // The error output survived validation and was just cleared, so the
        // fault about the primary output goes through it. A null `out_error`
        // takes this arm too, and `emit` then reports the status only.
        // SAFETY: this arm is reached only when `begin_error_out` accepted
        // `out_error`, which is exactly `emit`'s requirement: null, or an
        // address it has itself just written through.
        (Err(fault), Ok(())) => Err(unsafe { crate::error::emit(out_error, fault) }),
        // Nothing here can carry a message: either the error output is the one
        // that was rejected, or both were.
        (Err(fault), Err(_)) | (Ok(()), Err(fault)) => Err(fault.status()),
    }
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

/// Every structure that owns a family of presence flags in the header.
///
/// A structure carrying presence bits has to appear here, and
/// [`check_input_tables`] will not compile it otherwise. The comparison against
/// the header itself is in this module's tests, where the header can be read as
/// text; this list is what stops a structure from being left out of that
/// comparison, which is a failure the comparison cannot see — a table nobody
/// looks at and a header flag nobody claims look exactly alike from inside a
/// loop over the tables that are listed.
///
/// Each entry names its owner through the type rather than by repeating a
/// string, and the tests hold this list and their own family table to the same
/// membership.
const FAMILY_OWNERS: &[&str] = &[
    <madopilot_operation_t as Input>::NAME,
    <madopilot_open_request_t as Input>::NAME,
    <madopilot_map_request_t as Input>::NAME,
    <madopilot_find_request_t as Input>::NAME,
    <madopilot_match_options_t as Input>::NAME,
];

/// Whether `list` holds `value`.
///
/// A `const fn` so that [`check_input_tables`] can ask the question while the
/// structure whose table it is asking about is being compiled.
const fn lists(list: &[usize], value: usize) -> bool {
    let mut index = 0;
    while index < list.len() {
        if list[index] == value {
            return true;
        }
        index += 1;
    }

    false
}

/// Whether two names are the same name.
///
/// `str` comparison is not available in a `const fn`, and the names being
/// compared are C identifiers, so the bytes are compared directly.
const fn same(left: &str, right: &str) -> bool {
    let (left, right) = (left.as_bytes(), right.as_bytes());
    if left.len() != right.len() {
        return false;
    }

    let mut index = 0;
    while index < left.len() {
        if left[index] != right[index] {
            return false;
        }
        index += 1;
    }

    true
}

/// What the compiler measured `name` as.
///
/// Panics when the layout report does not measure the structure at all, which
/// would leave [`check_input_tables`] comparing a prefix list against nothing.
const fn measured(name: &str) -> &'static TypeLayout {
    let mut index = 0;
    while index < LAYOUT.len() {
        if same(LAYOUT[index].name, name) {
            return &LAYOUT[index];
        }
        index += 1;
    }

    panic!("the layout report does not measure this structure, so its prefix list is unverifiable");
}

/// Runs every check one input structure's tables have to pass, while that
/// structure is being compiled.
///
/// Rust cannot enumerate the implementations of a trait, and const evaluation
/// is what stands in for enumerating them. The checks are written against one
/// structure at a time and run from two places that cannot be avoided: a
/// `const` item [`inputs!`] emits beside each implementation, which is
/// evaluated wherever it was written — module, function body, or code nothing
/// ever reaches — and [`read`], which runs them for every structure it is
/// instantiated for, whoever wrote that structure's implementation. Neither
/// depends on a test being collected, so no implementation can be positioned
/// somewhere its tables go unchecked.
///
/// Every message is a literal because a `const` panic cannot format one. The
/// structure is named by the diagnostic itself, which reports the failure as
/// `check_input_tables::<the structure>`.
pub(crate) const fn check_input_tables<S: Input>() {
    // The caller's declared size is held to the same rule the library's own
    // mandatory prefixes are held to by
    // `tests/layout.rs::every_mandatory_prefix_is_a_real_field_boundary`. That
    // rule is only as good as the boundary list it is checked against, so the
    // list is compared field for field with the layout report the C probe is
    // diffed against.
    let layout = measured(S::NAME);
    assert!(
        S::PREFIXES.len() == layout.fields.len() + 1,
        "the prefix list does not name every one of the structure's fields"
    );

    let mut index = 0;
    while index < layout.fields.len() {
        assert!(
            S::PREFIXES[index] == layout.fields[index].offset,
            "the prefix list does not match the layout report field for field"
        );
        index += 1;
    }
    assert!(
        S::PREFIXES[layout.fields.len()] == layout.size,
        "the prefix list does not end at the size the layout report measured"
    );

    assert!(
        lists(S::PREFIXES, S::MANDATORY),
        "the mandatory prefix ends inside a field"
    );

    assert!(
        S::PRESENCE.is_empty() || owns_a_family(S::NAME),
        "the structure carries presence bits that belong to no family in `FAMILY_OWNERS`, so the header and this table are never compared; add its family there"
    );

    let mut index = 0;
    while index < S::PRESENCE.len() {
        let (_, required) = S::PRESENCE[index];
        assert!(
            lists(S::PREFIXES, required),
            "a presence bit requires a prefix that is not a field boundary"
        );
        assert!(
            required > S::MANDATORY,
            "a presence bit names a field the mandatory prefix already covers, so the bit can never be refused"
        );

        let mut earlier = 0;
        while earlier < index {
            assert!(
                S::PRESENCE[earlier].1 != required,
                "two presence bits require the same prefix, so one of them names the wrong field"
            );
            earlier += 1;
        }

        index += 1;
    }
}

/// Whether [`FAMILY_OWNERS`] lists `name`.
const fn owns_a_family(name: &str) -> bool {
    let mut index = 0;
    while index < FAMILY_OWNERS.len() {
        if same(FAMILY_OWNERS[index], name) {
            return true;
        }
        index += 1;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::{
        FAMILY_OWNERS, Input, madopilot_find_request_t, madopilot_map_request_t,
        madopilot_match_options_t, madopilot_open_request_t, madopilot_operation_t,
    };

    /// The header, read as text rather than compiled.
    ///
    /// `tests/frozen_numbers.rs` reads it the same way and for the same reason:
    /// the ABI a caller compiles against is stated there, so a flag exists for
    /// that caller whether or not this crate accounted for it.
    const HEADER: &str = include_str!("../include/madopilot/madopilot.h");

    /// One family of presence flags, and the structure whose table owns it.
    struct PresenceFamily {
        /// What every flag of the family is named with.
        prefix: &'static str,
        /// The owning structure's C name.
        owner: &'static str,
        /// The owning structure's presence table.
        table: &'static [(u32, usize)],
    }

    /// Which input structure each family of presence flags belongs to.
    ///
    /// A flag whose name matches no family here is not skipped: the test below
    /// fails and names it, so a new family is a decision someone writes down
    /// rather than one the parser makes silently. The same holds in the other
    /// direction, from [`super::FAMILY_OWNERS`], which a structure carrying
    /// presence bits has to appear in to compile at all, and which this list is
    /// held to member for member.
    ///
    /// Each entry names its owner through the type rather than by repeating a
    /// string, which is what ties the family to the table beside it.
    const PRESENCE_FAMILIES: &[PresenceFamily] = &[
        PresenceFamily {
            prefix: "MADOPILOT_OPERATION_HAS_",
            owner: <madopilot_operation_t as Input>::NAME,
            table: <madopilot_operation_t as Input>::PRESENCE,
        },
        PresenceFamily {
            prefix: "MADOPILOT_OPEN_HAS_",
            owner: <madopilot_open_request_t as Input>::NAME,
            table: <madopilot_open_request_t as Input>::PRESENCE,
        },
        PresenceFamily {
            prefix: "MADOPILOT_MAP_HAS_",
            owner: <madopilot_map_request_t as Input>::NAME,
            table: <madopilot_map_request_t as Input>::PRESENCE,
        },
        PresenceFamily {
            prefix: "MADOPILOT_FIND_HAS_",
            owner: <madopilot_find_request_t as Input>::NAME,
            table: <madopilot_find_request_t as Input>::PRESENCE,
        },
        PresenceFamily {
            prefix: "MADOPILOT_MATCH_HAS_",
            owner: <madopilot_match_options_t as Input>::NAME,
            table: <madopilot_match_options_t as Input>::PRESENCE,
        },
    ];

    /// Flags on structures the library writes.
    ///
    /// An output flag reports what the library populated. It makes no claim
    /// about which fields a caller supplied, so it carries no presence
    /// obligation and no input structure owns it.
    const OUTPUT_FLAGS: &[&str] = &[
        "MADOPILOT_IMAGE_SHARED",
        "MADOPILOT_TARGET_SUPPORTS_PLACEMENT",
        "MADOPILOT_ERROR_HAS_ASSET_DETAIL",
        "MADOPILOT_ERROR_HAS_BACKEND",
    ];

    /// The families this module compares are exactly the families the
    /// structures declare.
    ///
    /// [`super::FAMILY_OWNERS`] is what a structure carrying presence bits has
    /// to appear in to compile, and it is checked while that structure is
    /// compiled rather than while a test runs. It is only worth that if it and
    /// the list the comparisons below iterate name the same structures, so the
    /// two are held to each other here.
    #[test]
    fn every_structure_that_declares_a_family_has_one_here() {
        let compared: Vec<&str> = PRESENCE_FAMILIES
            .iter()
            .map(|family| family.owner)
            .collect();

        assert_eq!(
            compared.as_slice(),
            FAMILY_OWNERS,
            "`FAMILY_OWNERS` and `PRESENCE_FAMILIES` name different structures, so a structure can satisfy the one that is checked at compile time and still be compared against the header by nothing"
        );
    }

    /// A presence table carries no bit the header does not declare for that
    /// structure.
    ///
    /// The opposite direction of the test below, and the weaker of the two: a
    /// bit the header does not declare makes the boundary refuse a prefix no
    /// documented caller can ask for, rather than accept one it should refuse.
    /// It is still a table and a header that disagree, and one of them is
    /// wrong.
    #[test]
    fn every_bit_a_presence_table_carries_is_a_flag_the_header_declares() {
        let flags = header_flags();
        for family in PRESENCE_FAMILIES {
            for &(bit, _) in family.table {
                assert!(
                    flags
                        .iter()
                        .any(|&(flag, value)| value == bit && flag.starts_with(family.prefix)),
                    "{}'s presence table carries bit {bit:#x}, which the header declares as no flag of that structure",
                    family.owner
                );
            }
        }
    }

    /// Every flag the header defines, with its value.
    ///
    /// Two filters, because neither alone accounts for every flag. The `Flags`
    /// section catches one that does not follow the `_HAS_` convention, and the
    /// name shape catches a presence bit filed somewhere else in the header —
    /// where the section scan would never look, and where the obligation this
    /// list exists to state is exactly as real.
    fn header_flags() -> Vec<(&'static str, u32)> {
        let mut flags = flags_section();
        for line in HEADER.lines() {
            let Some((name, literal)) = define(line) else {
                continue;
            };
            if !is_presence_name(name) || flags.iter().any(|&(known, _)| known == name) {
                continue;
            }

            flags.push((name, flag_value(name, literal)));
        }

        flags
    }

    /// Every flag the header's `Flags` section defines, in declaration order.
    fn flags_section() -> Vec<(&'static str, u32)> {
        let mut lines = HEADER.lines().skip_while(|line| line.trim() != "* Flags");
        assert!(
            lines.next().is_some(),
            "the header has no `Flags` section banner, so this test would compare against nothing"
        );

        let mut flags = Vec::new();
        for line in lines {
            // The next banner ends the section.
            if line.starts_with("/* ---") {
                break;
            }
            let Some((name, literal)) = define(line) else {
                continue;
            };

            flags.push((name, flag_value(name, literal)));
        }

        assert!(
            !flags.is_empty(),
            "the header's `Flags` section parsed to no flag at all, so every comparison below would be vacuous"
        );

        flags
    }

    /// Splits `#define NAME VALUE` into its name and its value token.
    ///
    /// The indentation the header's platform branches use is accepted, and a
    /// value is the token after the name, which leaves a trailing comment out of
    /// it.
    fn define(line: &'static str) -> Option<(&'static str, Option<&'static str>)> {
        let directive = line
            .trim_start()
            .strip_prefix('#')?
            .trim_start()
            .strip_prefix("define ")?;

        let mut tokens = directive.split_whitespace();
        let name = tokens.next()?;

        Some((name, tokens.next()))
    }

    /// Whether a name follows the convention for a presence bit,
    /// `MADOPILOT_<structure>_HAS_<field>`.
    fn is_presence_name(name: &str) -> bool {
        name.starts_with("MADOPILOT_") && name.contains("_HAS_")
    }

    /// Reads a flag's value, which is an integer literal with an optional width
    /// suffix.
    fn flag_value(name: &str, literal: Option<&str>) -> u32 {
        let literal =
            literal.unwrap_or_else(|| panic!("the header defines `{name}` with no value"));
        let digits = literal.trim_end_matches(['u', 'U', 'l', 'L']);

        digits
            .strip_prefix("0x")
            .map_or_else(|| digits.parse(), |hex| u32::from_str_radix(hex, 16))
            .unwrap_or_else(|_| {
                panic!("the header defines `{name}` as `{literal}`, which is not an integer")
            })
    }

    /// Every flag a caller can set is either an input bit the boundary can
    /// refuse or a flag the library writes.
    ///
    /// A presence table that omits a bit the header declares is the failure
    /// [`super::Input::PRESENCE`] exists to prevent, and it is invisible to a
    /// check that iterates the table: the missing entry is missing from the
    /// iteration too. The header is the independent statement of what bits
    /// exist, so it is what the tables are compared against.
    #[test]
    fn every_flag_the_header_declares_is_an_input_bit_or_an_output_flag() {
        for (name, value) in header_flags() {
            if OUTPUT_FLAGS.contains(&name) {
                continue;
            }

            let family = PRESENCE_FAMILIES
                .iter()
                .find(|family| name.starts_with(family.prefix))
                .unwrap_or_else(|| {
                    panic!(
                        "the header declares `{name}`, which belongs to no known input structure; add its family to `PRESENCE_FAMILIES`, or add the name to `OUTPUT_FLAGS` if the library writes the flag rather than reads it"
                    )
                });

            assert!(
                family.table.iter().any(|&(bit, _)| bit == value),
                "the header declares `{name}` as {value:#x}, which {}'s presence table omits, so a prefix that stops short of the field it names would be defaulted instead of refused",
                family.owner
            );
        }
    }
}
