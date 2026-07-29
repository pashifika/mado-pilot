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

/// Builds an input structure's legal-prefix list out of its own fields.
///
/// The offsets come from the compiler rather than from a written table, so a
/// field that moves takes the list with it. What a reader still has to check is
/// that every field is named, which
/// `tests::every_input_prefix_list_names_every_field` proves against the same
/// layout report the C probe is diffed against.
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

/// A size-versioned structure the caller supplies.
pub(crate) trait Input: Copy {
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

/// What a check of one input structure is handed: its name, its legal prefixes,
/// its presence table, and its mandatory prefix.
#[cfg(test)]
type InputCheck<'a> = &'a dyn Fn(&'static str, &'static [usize], &'static [(u32, usize)], usize);

/// Runs `check` over every structure a caller supplies.
///
/// Rust cannot enumerate the implementations of a trait, so the list is written
/// out. What keeps a written list from going stale is
/// [`tests::for_every_input_visits_every_input_implementation`], which reads the
/// implementations back out of this crate's own source and requires the two to
/// name the same structures.
#[cfg(test)]
fn for_every_input(check: InputCheck) {
    fn one<S: Input>(check: InputCheck) {
        check(S::NAME, S::PREFIXES, S::PRESENCE, S::MANDATORY);
    }

    one::<crate::types::madopilot_operation_t>(check);
    one::<crate::types::madopilot_source_t>(check);
    one::<crate::types::madopilot_replay_frame_t>(check);
    one::<crate::types::madopilot_package_source_t>(check);
    one::<crate::types::madopilot_open_request_t>(check);
    one::<crate::types::madopilot_map_request_t>(check);
    one::<crate::types::madopilot_find_request_t>(check);
    one::<crate::types::madopilot_match_options_t>(check);
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::PathBuf;

    use crate::layout::{LAYOUT, TypeLayout};

    /// The header, read as text rather than compiled.
    ///
    /// `tests/frozen_numbers.rs` reads it the same way and for the same reason:
    /// the ABI a caller compiles against is stated there, so a flag exists for
    /// that caller whether or not this crate accounted for it.
    const HEADER: &str = include_str!("../include/madopilot/madopilot.h");

    /// Which input structure each family of presence flags belongs to.
    ///
    /// A flag whose name matches no family here is not skipped: the test below
    /// fails and names it, so a new family is a decision someone writes down
    /// rather than one the parser makes silently.
    const PRESENCE_FAMILIES: &[(&str, &str)] = &[
        ("MADOPILOT_OPERATION_HAS_", "madopilot_operation_t"),
        ("MADOPILOT_OPEN_HAS_", "madopilot_open_request_t"),
        ("MADOPILOT_MAP_HAS_", "madopilot_map_request_t"),
        ("MADOPILOT_FIND_HAS_", "madopilot_find_request_t"),
        ("MADOPILOT_MATCH_HAS_", "madopilot_match_options_t"),
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

    fn measured(name: &str) -> &'static TypeLayout {
        LAYOUT
            .iter()
            .find(|layout| layout.name == name)
            .unwrap_or_else(|| panic!("`{name}` is measured by the layout report"))
    }

    /// Every input structure's name and presence table.
    fn presence_tables() -> Vec<(&'static str, &'static [(u32, usize)])> {
        let tables = RefCell::new(Vec::new());
        super::for_every_input(&|name, _, presence, _| tables.borrow_mut().push((name, presence)));

        tables.into_inner()
    }

    /// Every `Input` implementation this crate's source declares, by type name.
    ///
    /// The type name is what `super::for_every_input` is compared against
    /// because [`Input::NAME`] is already tied to it:
    /// `every_input_prefix_list_names_every_field` looks the name up in the
    /// layout report, whose entries are `stringify!` of the measured type.
    fn implemented_inputs() -> Vec<String> {
        let mut names = Vec::new();
        let mut pending = vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")];

        while let Some(directory) = pending.pop() {
            let entries = std::fs::read_dir(&directory)
                .unwrap_or_else(|error| panic!("`{}` is readable: {error}", directory.display()));

            for entry in entries {
                let path = entry
                    .expect("a directory entry under this crate's own source is readable")
                    .path();
                if path.is_dir() {
                    pending.push(path);
                    continue;
                }
                if path.extension().is_none_or(|extension| extension != "rs") {
                    continue;
                }

                let source = std::fs::read_to_string(&path)
                    .unwrap_or_else(|error| panic!("`{}` is readable: {error}", path.display()));
                names.extend(source.lines().filter_map(|line| {
                    line.trim_start()
                        .strip_prefix("impl Input for ")
                        .map(|rest| rest.trim_end_matches('{').trim().to_owned())
                }));
            }
        }
        names.sort_unstable();

        names
    }

    /// Every flag the header's `Flags` section defines, with its value.
    ///
    /// The section is the filter rather than the name, so a flag that does not
    /// follow the `_HAS_` convention is still accounted for. A value is read as
    /// the token after the name, which leaves a trailing comment out of it.
    fn header_flags() -> Vec<(&'static str, u32)> {
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
            let Some(definition) = line.strip_prefix("#define ") else {
                continue;
            };

            let mut tokens = definition.split_whitespace();
            let name = tokens.next().expect("a `#define` names something");
            let literal = tokens
                .next()
                .unwrap_or_else(|| panic!("the header defines `{name}` with no value"));
            let digits = literal.trim_end_matches(['u', 'U', 'l', 'L']);
            let value = digits
                .strip_prefix("0x")
                .map_or_else(|| digits.parse(), |hex| u32::from_str_radix(hex, 16))
                .unwrap_or_else(|_| {
                    panic!("the header defines `{name}` as `{literal}`, which is not an integer")
                });

            flags.push((name, value));
        }

        assert!(
            !flags.is_empty(),
            "the header's `Flags` section parsed to no flag at all, so every comparison below would be vacuous"
        );

        flags
    }

    /// The caller's declared size is held to the same rule the library's own
    /// mandatory prefixes are held to by
    /// `tests/layout.rs::every_mandatory_prefix_is_a_real_field_boundary`. That
    /// rule is only as good as the boundary list it is checked against, so the
    /// list is compared field for field with the layout report the C probe is
    /// diffed against.
    /// Every structure `for_every_input` hands to a check is one the source
    /// implements, and every one the source implements is handed over.
    ///
    /// The three checks below are only as complete as that list, and Rust gives
    /// them no way to notice a ninth implementation. Reading the `impl Input
    /// for` lines back out of `src/` is that way: the list and the source are
    /// two independent statements of the same set, and a new implementation
    /// that reaches only one of them fails here.
    #[test]
    fn for_every_input_visits_every_input_implementation() {
        let implemented = implemented_inputs();
        assert!(
            !implemented.is_empty(),
            "the source names no `Input` implementation at all, so this comparison would be vacuous"
        );

        let mut visited: Vec<String> = presence_tables()
            .into_iter()
            .map(|(name, _)| name.to_owned())
            .collect();
        visited.sort_unstable();

        assert_eq!(
            visited, implemented,
            "`for_every_input` and the `impl Input for` lines under `src/` name different structures, so an implementation is either checked by nothing or checked twice"
        );
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
        let tables = presence_tables();

        for (name, value) in header_flags() {
            if OUTPUT_FLAGS.contains(&name) {
                continue;
            }

            let owner = PRESENCE_FAMILIES
                .iter()
                .find(|(family, _)| name.starts_with(family))
                .unwrap_or_else(|| {
                    panic!(
                        "the header declares `{name}`, which belongs to no known input structure; add its family to `PRESENCE_FAMILIES`, or add the name to `OUTPUT_FLAGS` if the library writes the flag rather than reads it"
                    )
                })
                .1;
            let presence = tables
                .iter()
                .find(|(structure, _)| *structure == owner)
                .unwrap_or_else(|| panic!("`{owner}` implements `Input`"))
                .1;

            assert!(
                presence.iter().any(|&(bit, _)| bit == value),
                "the header declares `{name}` as {value:#x}, which {owner}'s presence table omits, so a prefix that stops short of the field it names would be defaulted instead of refused"
            );
        }
    }

    #[test]
    fn every_presence_table_entry_is_a_flag_the_header_declares() {
        let declared = header_flags();

        for (name, presence) in presence_tables() {
            for &(bit, _) in presence {
                assert!(
                    declared.iter().any(|&(flag, value)| value == bit
                        && PRESENCE_FAMILIES
                            .iter()
                            .any(|(family, owner)| *owner == name && flag.starts_with(family))),
                    "{name}'s presence table carries bit {bit:#x}, which the header declares as no flag of that structure"
                );
            }
        }
    }

    #[test]
    fn every_input_prefix_list_names_every_field() {
        super::for_every_input(&|name, prefixes, _, _| {
            let layout = measured(name);
            let expected: Vec<usize> = layout
                .fields
                .iter()
                .map(|field| field.offset)
                .chain([layout.size])
                .collect();

            assert_eq!(
                prefixes,
                expected.as_slice(),
                "{name} does not list every one of its field offsets"
            );
        });
    }

    #[test]
    fn every_input_mandatory_prefix_is_one_of_its_legal_prefixes() {
        super::for_every_input(&|name, prefixes, _, mandatory| {
            assert!(
                prefixes.contains(&mandatory),
                "{name}'s {mandatory} byte mandatory prefix ends inside a field"
            );
        });
    }

    #[test]
    fn every_presence_bit_needs_a_prefix_that_ends_at_a_field_boundary() {
        super::for_every_input(&|name, prefixes, presence, mandatory| {
            for &(bit, required) in presence {
                assert!(
                    prefixes.contains(&required),
                    "{name}'s presence bit {bit:#x} requires {required} bytes, which is not a field boundary"
                );
                assert!(
                    required > mandatory,
                    "{name}'s presence bit {bit:#x} names a field the mandatory prefix already covers, so the bit can never be refused"
                );
            }
        });
    }

    #[test]
    fn no_two_presence_bits_of_one_structure_name_the_same_field() {
        super::for_every_input(&|name, _, presence, _| {
            for (index, &(bit, required)) in presence.iter().enumerate() {
                assert!(
                    !presence[..index]
                        .iter()
                        .any(|&(_, earlier)| earlier == required),
                    "{name}'s presence bit {bit:#x} requires the same {required} byte prefix as an earlier bit"
                );
            }
        });
    }
}
