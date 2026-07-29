//! Deadlines and cancellation, as they cross the boundary.
//!
//! A deadline is an absolute instant in the library's own monotonic domain,
//! never a duration. A duration restarts at every hop, so a caller that passed
//! "500 ms" through three calls would have granted a second and a half; an
//! absolute instant means the same moment everywhere it is carried.
//!
//! The domain's origin is fixed for the life of the loaded library and is not a
//! wall-clock time. A caller reads the current instant from the table's clock
//! entry and adds to it, which is why that entry is inside the mandatory
//! function-table prefix: without it a caller cannot construct a deadline at
//! all.

use std::time::Duration;

use mado_pilot::{CancellationToken, Clock, MonotonicInstant, OperationContext, SystemClock};

use crate::boundary::{covers, declared, inputs, prefixes};
use crate::error::Fault;
use crate::handle::opaque;
use crate::status::MADOPILOT_STATUS_OK;
use crate::types::{MADOPILOT_OPERATION_HAS_DEADLINE, madopilot_operation_t};
use crate::{handle, status};

opaque! {
    /// A shared cancellation flag.
    ///
    /// Every retained reference observes the same flag, so one handle can cancel
    /// several concurrent operations. Cancelling is idempotent and never blocks.
    madopilot_cancellation_t => Cancellation
}

/// The payload behind a cancellation handle.
#[derive(Debug)]
pub(crate) struct Cancellation(CancellationToken);

impl Cancellation {
    fn new() -> Self {
        Self(CancellationToken::new())
    }
}

inputs! {
    impl Input for madopilot_operation_t {
        // Through `flags`. A caller with nothing to say about deadlines or
        // cancellation still has to say that, because an absent operation structure
        // and one that declares neither are different requests.
        const MANDATORY: usize = 8;
        const NAME: &'static str = "madopilot_operation_t";
        const PREFIXES: &'static [usize] = prefixes!(
            madopilot_operation_t,
            struct_size,
            flags,
            deadline_nanos,
            cancellation,
        );
        // `cancellation` carries no presence bit: null is its documented absent
        // value, so a prefix that omits it says the same thing the field would.
        const PRESENCE: &'static [(u32, usize)] = &[(
            MADOPILOT_OPERATION_HAS_DEADLINE,
            covers!(madopilot_operation_t, deadline_nanos: u64),
        )];

        fn defaults() -> Self {
            Self {
                struct_size: 0,
                flags: 0,
                deadline_nanos: 0,
                cancellation: std::ptr::null(),
            }
        }

        fn presence_bits(&self) -> u32 {
            self.flags
        }
    }
}

/// Builds the facade context a caller's operation structure describes.
///
/// # Errors
///
/// Rejects a null or malformed structure, and a `cancellation` that is not a
/// live cancellation handle.
///
/// A deadline is never rejected. Every `uint64_t` nanosecond value the field can
/// hold is representable in the monotonic domain, so there is no unrepresentable
/// case to report — this said there was, and nothing implemented it.
///
/// # Safety
///
/// `operation` must point at a readable structure, and any cancellation handle
/// it names must stay retained for the call.
pub(crate) unsafe fn context(operation: *const madopilot_operation_t) -> Result<Context, Fault> {
    // SAFETY: forwarded unchanged from this function's own contract.
    let request = unsafe { crate::boundary::read_input(operation) }?;

    let mut context = OperationContext::new();

    if declared!(
        request,
        madopilot_operation_t,
        MADOPILOT_OPERATION_HAS_DEADLINE
    ) {
        // Infallible, and worth saying because the comment here used to claim a
        // report that does not exist. `Duration::from_nanos` accepts the whole
        // `u64` range and `MonotonicInstant::from_origin` accepts every duration,
        // so no deadline the field can carry is out of domain and none is
        // clamped. If either ever gains a bound, this is where the refusal goes:
        // a silently nearer deadline expires early, which a caller cannot tell
        // from its own work being slow.
        context = context.with_deadline(MonotonicInstant::from_origin(Duration::from_nanos(
            request.deadline_nanos,
        )));
    }

    if !request.cancellation.is_null() {
        // SAFETY: the caller contract requires the handle to stay retained for
        // the call, and null was excluded above.
        let Some(cancellation) = (unsafe { handle::borrow::<Cancellation>(request.cancellation) })
        else {
            // Unreachable, and it has to be written anyway because `borrow`
            // returns an `Option`. `borrow` refuses exactly one thing — a null
            // pointer — which the branch above already excluded; it cannot tell
            // one handle type from another, because a handle is an opaque
            // pointer with no tag to check. The message here used to claim it
            // could. A caller that passes some other live handle is undefined
            // behaviour by this function's own safety contract, not an error
            // this boundary detects.
            return Err(Fault::abi("`cancellation` is null"));
        };
        context = context.with_cancellation(cancellation.0.clone());
    }

    Ok(Context(context))
}

/// A validated operation context, and the two checks every entry owes it.
#[derive(Debug)]
pub(crate) struct Context(OperationContext);

impl Context {
    /// Returns the facade context to hand to the operation being performed.
    pub(crate) const fn inner(&self) -> &OperationContext {
        &self.0
    }

    /// Refuses admission when the operation is already over.
    ///
    /// This is the boundary's own before-admission check. Each contract below
    /// arbitrates its own terminal outcome as well, and in the Phase 1 pipeline
    /// an inner one usually observes an interruption first. That is the intent
    /// rather than redundancy: this check is correct in its own right, and it
    /// is what makes an entry that never reaches a contract — because its own
    /// validation failed first — still honor the deadline it was given.
    ///
    /// # Errors
    ///
    /// Returns cancellation or deadline expiry as a fault of the operation
    /// category.
    pub(crate) fn admit(&self) -> Result<(), Fault> {
        self.interruption()
    }

    /// Refuses to commit a result the operation is no longer entitled to.
    ///
    /// # Errors
    ///
    /// As [`Context::admit`]. A value that loses this race is dropped rather
    /// than published, so late work never becomes observable.
    pub(crate) fn commit(&self) -> Result<(), Fault> {
        self.interruption()
    }

    fn interruption(&self) -> Result<(), Fault> {
        match self.0.interruption() {
            None => Ok(()),
            Some(interruption) => Err(Fault::from_error(
                &interruption.into(),
                status::MADOPILOT_ERROR_CATEGORY_OPERATION,
            )),
        }
    }
}

/// Returns the current instant in the library's monotonic domain.
pub(crate) fn now_nanos() -> u64 {
    let elapsed = SystemClock.now().since_origin();

    // A `u64` of nanoseconds is 584 years of process uptime. Saturating is the
    // only honest answer past that, and it makes every deadline built from it
    // already expired rather than wrapping into the past.
    u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX)
}

pub(crate) fn create(out: *mut *mut madopilot_cancellation_t) -> status::madopilot_status_t {
    // SAFETY: the caller supplies a writable, correctly aligned output address.
    if let Err(fault) = unsafe { crate::boundary::begin_handle_out(out, "out_cancellation") } {
        return fault.status();
    }
    crate::hooks::reach(crate::hooks::Site::Entry);

    let created = handle::into_raw(Cancellation::new());
    // SAFETY: `out` was validated above and is writable for the call.
    unsafe { out.write(created) };

    MADOPILOT_STATUS_OK
}

pub(crate) fn retain(cancellation: *const madopilot_cancellation_t) -> status::madopilot_status_t {
    // SAFETY: the handle is null or one this module produced, and the caller
    // holds a live reference for the call.
    unsafe { handle::retain::<Cancellation>(cancellation) }

    MADOPILOT_STATUS_OK
}

pub(crate) fn release(cancellation: *mut madopilot_cancellation_t) -> status::madopilot_status_t {
    // SAFETY: as `retain`, and the caller is giving up the reference it owns.
    unsafe { handle::release::<Cancellation>(cancellation) }

    MADOPILOT_STATUS_OK
}

pub(crate) fn cancel(cancellation: *const madopilot_cancellation_t) -> status::madopilot_status_t {
    crate::hooks::reach(crate::hooks::Site::Entry);

    // SAFETY: the caller keeps the handle retained for the call; null is
    // rejected below because "do nothing" is not an answer to "cancel this".
    let Some(cancellation) = (unsafe { handle::borrow::<Cancellation>(cancellation) }) else {
        return status::MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    cancellation.0.cancel();

    MADOPILOT_STATUS_OK
}

pub(crate) fn is_cancelled(
    cancellation: *const madopilot_cancellation_t,
    out: *mut i32,
) -> status::madopilot_status_t {
    // SAFETY: the caller supplies a writable, correctly aligned output address.
    if let Err(fault) = unsafe { crate::boundary::begin_scalar_out(out, "out_cancelled", 0) } {
        return fault.status();
    }
    crate::hooks::reach(crate::hooks::Site::Entry);

    // SAFETY: the caller keeps the handle retained for the call.
    let Some(cancellation) = (unsafe { handle::borrow::<Cancellation>(cancellation) }) else {
        return status::MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    // SAFETY: `out` was validated above.
    unsafe { crate::boundary::commit_scalar(out, i32::from(cancellation.0.is_cancelled())) };

    MADOPILOT_STATUS_OK
}
