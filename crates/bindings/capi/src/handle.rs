//! Opaque handles and their reference-counted lifecycle.
//!
//! Every owned C handle is an [`Arc`] whose raw pointer has been given a name a
//! C compiler can hold but not look inside. Retain and release are therefore the
//! atomic reference-count operations rather than a lock the boundary invents,
//! which is what makes concurrent retain, const access, and release from several
//! threads safe as long as each thread keeps one live reference of its own.
//!
//! # The rule a caller has to keep
//!
//! A handle passed to a call must stay retained for the whole call. Releasing
//! the final reference concurrently with a call that has not retained one of its
//! own is outside the contract: no boundary check can distinguish it from a
//! valid pointer, because the check would race the release it is trying to
//! detect.
//!
//! # Null
//!
//! Retain and release accept null as a no-op, so a cleanup path can release
//! whatever it has without knowing how far construction got. Every
//! behavior-bearing operation rejects null instead, because "do nothing" is not
//! an answer to a question that has one.

use std::sync::Arc;

/// A payload reachable through an opaque C handle.
///
/// The `Send + Sync` bound is the thread-safety contract in the header made
/// mechanical: a payload that could not be shared across threads could not be
/// given to C, which has no way to express the restriction.
pub(crate) trait Opaque: Sized + Send + Sync {
    /// The opaque C type whose pointer names this payload.
    type C;
}

/// Declares an opaque C handle type and binds it to its Rust payload.
macro_rules! opaque {
    ($(#[$attribute:meta])* $name:ident => $payload:ty) => {
        $(#[$attribute])*
        ///
        /// An opaque handle. Its layout is not part of the ABI and a caller must
        /// not dereference it.
        #[repr(C)]
        #[derive(Debug)]
        pub struct $name {
            _data: [u8; 0],
            _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
        }

        impl $crate::handle::Opaque for $payload {
            type C = $name;
        }
    };
}

pub(crate) use opaque;

/// Moves `value` into a new handle with one reference.
pub(crate) fn into_raw<T: Opaque>(value: T) -> *mut T::C {
    Arc::into_raw(Arc::new(value)).cast::<T::C>().cast_mut()
}

/// Borrows the payload behind `handle` for the duration of a call.
///
/// Returns `None` for null, which every behavior-bearing entry turns into
/// invalid argument.
///
/// # Safety
///
/// `handle` must be null or a pointer this module produced whose reference count
/// the caller keeps above zero for the whole borrow.
pub(crate) unsafe fn borrow<'a, T: Opaque>(handle: *const T::C) -> Option<&'a T> {
    if handle.is_null() {
        return None;
    }

    // SAFETY: the pointer came from `into_raw`, so it points at a live `T`
    // inside an `Arc` allocation, and this function's contract requires the
    // caller to hold a reference for the whole borrow.
    Some(unsafe { &*handle.cast::<T>() })
}

/// Adds one owned reference to `handle`.
///
/// # Safety
///
/// As [`borrow`].
pub(crate) unsafe fn retain<T: Opaque>(handle: *const T::C) {
    if handle.is_null() {
        return;
    }

    // SAFETY: the pointer came from `into_raw` and the caller holds a live
    // reference, so the count is above zero and incrementing it is sound.
    unsafe { Arc::increment_strong_count(handle.cast::<T>()) }
}

/// Drops one owned reference to `handle`, destroying the payload at the last.
///
/// # Safety
///
/// `handle` must be null or a pointer this module produced for which the caller
/// owns a reference that it is giving up, and which no other call is using.
pub(crate) unsafe fn release<T: Opaque>(handle: *const T::C) {
    if handle.is_null() {
        return;
    }

    // SAFETY: the caller owns the reference being dropped, so the count stays
    // consistent, and the allocation is freed by the same `Arc` machinery that
    // created it.
    unsafe { Arc::decrement_strong_count(handle.cast::<T>()) }
}
