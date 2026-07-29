//! Borrowed pointer-length views.
//!
//! Every string and byte range that crosses the boundary is an explicit pointer
//! and length. Nothing is NUL-terminated, because a length the caller states is
//! a length the library can validate, and a terminator it must search for is
//! not.
//!
//! These two structures carry no `struct_size`. They are the boundary's
//! primitives rather than extensible records: a later field would change what a
//! view *is*, and every structure that contains one would have to change with
//! it. A later phase that needs more than a pointer and a length adds a
//! different type.
//!
//! # Ownership
//!
//! A view is always borrowed and never owned. For a library-returned output view,
//! its declaration names the handle whose retention keeps it readable; the view
//! becomes invalid at that handle's final release. For a caller-supplied input
//! view, the caller keeps its bytes readable and unmodified for the call, and the
//! library retains no reference after the call returns.

use std::ffi::c_char;
use std::slice;

use crate::error::Fault;

/// A borrowed UTF-8 string view.
///
/// `data` may be null only when `len` is zero, which is the empty string. The
/// bytes are UTF-8 and are **not** NUL-terminated.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_str_t {
    /// The first byte, or null for an empty view.
    pub data: *const c_char,
    /// The length in bytes, excluding any terminator.
    pub len: usize,
}

/// A borrowed byte view.
///
/// `data` may be null only when `len` is zero.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct madopilot_bytes_t {
    /// The first byte, or null for an empty view.
    pub data: *const u8,
    /// The length in bytes.
    pub len: usize,
}

impl madopilot_str_t {
    /// Returns the empty view, which is what a failed or absent string is.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            data: std::ptr::null(),
            len: 0,
        }
    }

    /// Borrows `value` for as long as the caller keeps its owner alive.
    #[must_use]
    pub(crate) const fn borrowed(value: &str) -> Self {
        Self {
            data: value.as_ptr().cast::<c_char>(),
            len: value.len(),
        }
    }
}

impl madopilot_bytes_t {
    /// Returns the empty view.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            data: std::ptr::null(),
            len: 0,
        }
    }

    /// Borrows `value` for as long as the caller keeps its owner alive.
    #[must_use]
    pub(crate) const fn borrowed(value: &[u8]) -> Self {
        Self {
            data: value.as_ptr(),
            len: value.len(),
        }
    }
}

/// Validates a byte view's pointer-length shape and returns the length it
/// describes.
///
/// The shape and nothing else, so this reads no byte the view points at. A null
/// pointer carrying a length and a length that cannot be an object size are both
/// malformed requests — the refusal ADR 0007 freezes as
/// `MADOPILOT_STATUS_INVALID_ARGUMENT` with `MADOPILOT_ERROR_CATEGORY_ABI` — and
/// they are refusals about the declaration rather than about the content.
///
/// Separated from [`bytes`] so an entry that has to decide something *from* the
/// length can decide it before a slice over the range exists: a view that becomes
/// owned storage is sized by its own declaration, and the ceiling on that
/// allocation has to be applied before the allocation, without that taking
/// precedence over the shape rules the released ABI already fixed.
///
/// # Errors
///
/// Rejects a null pointer with a nonzero length, and a length above
/// `isize::MAX`.
pub(crate) fn byte_len(view: madopilot_bytes_t, field: &'static str) -> Result<usize, Fault> {
    if view.data.is_null() && view.len != 0 {
        return Err(Fault::abi(format!(
            "`{field}` is a null pointer with length {}",
            view.len
        )));
    }
    // A length that cannot be an object size cannot describe one, and rejecting
    // it here is what keeps every later offset and stride calculation inside the
    // range the address space can represent.
    if view.len > isize::MAX.unsigned_abs() {
        return Err(Fault::abi(format!(
            "`{field}` declares {} bytes, which is not a representable object size",
            view.len
        )));
    }

    Ok(view.len)
}

/// Resolves `view` into borrowed bytes.
///
/// # Errors
///
/// As [`byte_len`], whose checks this performs before reading anything.
///
/// # Safety
///
/// `view.data` must point to `view.len` readable bytes that stay valid and
/// unmodified for the duration of the call.
pub(crate) unsafe fn bytes<'a>(
    view: madopilot_bytes_t,
    field: &'static str,
) -> Result<&'a [u8], Fault> {
    let len = byte_len(view, field)?;
    if view.data.is_null() {
        // The one view a null pointer may describe, which `byte_len` has just
        // proved is this one: no pointer, no bytes, and no slice to form.
        return Ok(&[]);
    }

    // SAFETY: the pointer is non-null, the length is a representable object
    // size, and the caller contract documented on this function requires the
    // range to be readable and unmodified for the call.
    Ok(unsafe { slice::from_raw_parts(view.data, len) })
}

/// Resolves `view` into a borrowed UTF-8 string.
///
/// # Errors
///
/// Rejects a null pointer with a nonzero length, and bytes that are not UTF-8.
///
/// # Safety
///
/// As [`bytes`].
pub(crate) unsafe fn string<'a>(
    view: madopilot_str_t,
    field: &'static str,
) -> Result<&'a str, Fault> {
    let raw = madopilot_bytes_t {
        data: view.data.cast::<u8>(),
        len: view.len,
    };
    // SAFETY: forwarded unchanged from this function's own safety contract.
    let bytes = unsafe { bytes(raw, field) }?;

    std::str::from_utf8(bytes).map_err(|error| {
        Fault::abi(format!(
            "`{field}` is not UTF-8: invalid byte at offset {}",
            error.valid_up_to()
        ))
    })
}

/// Resolves `view` into a borrowed string that must not be empty.
///
/// # Errors
///
/// As [`string`], and rejects an empty view.
///
/// # Safety
///
/// As [`bytes`].
pub(crate) unsafe fn non_empty_string<'a>(
    view: madopilot_str_t,
    field: &'static str,
) -> Result<&'a str, Fault> {
    // SAFETY: forwarded unchanged from this function's own safety contract.
    let value = unsafe { string(view, field) }?;
    if value.is_empty() {
        return Err(Fault::abi(format!("`{field}` must not be empty")));
    }

    Ok(value)
}
