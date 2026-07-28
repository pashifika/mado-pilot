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
//! A view is always borrowed and never owned. Each one names, in the
//! declaration that returns it, the handle whose retention keeps it readable.
//! When that handle reaches its final release the view becomes invalid, and the
//! caller is responsible for having copied anything it still needs.

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

/// Resolves `view` into borrowed bytes.
///
/// # Errors
///
/// Rejects a null pointer with a nonzero length before reading anything.
///
/// # Safety
///
/// `view.data` must point to `view.len` readable bytes that stay valid and
/// unmodified for the duration of the call.
pub(crate) unsafe fn bytes<'a>(
    view: madopilot_bytes_t,
    field: &'static str,
) -> Result<&'a [u8], Fault> {
    if view.data.is_null() {
        if view.len == 0 {
            return Ok(&[]);
        }
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

    // SAFETY: the pointer is non-null, the length is a representable object
    // size, and the caller contract documented on this function requires the
    // range to be readable and unmodified for the call.
    Ok(unsafe { slice::from_raw_parts(view.data, view.len) })
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
