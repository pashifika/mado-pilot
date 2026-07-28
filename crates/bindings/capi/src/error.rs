//! Structured owned errors.
//!
//! A failing entry always returns its status. It additionally produces an owned
//! error handle when, and only when, the caller asked for one by supplying
//! `out_error`. There is no global, thread-local, or engine-wide last-error
//! slot: the failure belongs to the call that produced it, and a slot would make
//! two threads' failures each other's business.
//!
//! # Why package loading needs more than a status
//!
//! Every other operation in the facade reports
//! [`mado_pilot::Error`](Error) — a status plus diagnostic text that is never
//! required for control flow. Package loading reports
//! [`AssetFault`] instead, which carries *which* rule was broken and *how far*
//! loading had got, and it does so deliberately: a bad content hash and an
//! unsafe entry path are both `AssetInvalid`, and a caller that wanted to tell
//! them apart would otherwise have to read the message. Flattening that into one
//! status at this boundary would throw away detail the Rust layer took care to
//! keep, so [`madopilot_error_detail_t`] carries both alongside the status.

use mado_pilot::{AssetFault, Error, Status};

use crate::boundary::{Out, Versioned};
use crate::handle::opaque;
use crate::status::{
    MADOPILOT_ERROR_CATEGORY_ABI, MADOPILOT_ERROR_CATEGORY_ASSET,
    MADOPILOT_ERROR_CATEGORY_UNSPECIFIED, MADOPILOT_STATUS_INTERNAL,
    MADOPILOT_STATUS_INVALID_ARGUMENT, MADOPILOT_STATUS_OK, code, madopilot_error_category_t,
    madopilot_status_t,
};
use crate::types::{
    MADOPILOT_ASSET_FAULT_UNKNOWN, MADOPILOT_ASSET_STAGE_UNKNOWN, MADOPILOT_ERROR_HAS_ASSET_DETAIL,
    MADOPILOT_ERROR_HAS_BACKEND, asset_fault_code, asset_stage_code, madopilot_asset_fault_t,
    madopilot_asset_stage_t, madopilot_error_detail_t,
};
use crate::view::madopilot_str_t;
use crate::{handle, hooks};

opaque! {
    /// An immutable owned error.
    ///
    /// Released with the module's error release entry. Its message and backend
    /// views are borrowed from it and become invalid at its final release, so a
    /// caller that needs the text afterwards copies it first.
    madopilot_error_t => Fault
}

/// One failure, in the form the boundary reports it.
///
/// Owned rather than borrowed: the values it reports outlive the call that
/// produced them, and a view into a temporary would be the one kind of dangling
/// pointer a caller could not see coming.
#[derive(Debug)]
pub(crate) struct Fault {
    status: madopilot_status_t,
    category: madopilot_error_category_t,
    message: String,
    backend: Option<String>,
    asset: Option<(madopilot_asset_fault_t, madopilot_asset_stage_t)>,
}

impl Fault {
    /// A request this boundary refused: a pointer, size, tag, or conversion.
    pub(crate) fn abi(message: impl Into<String>) -> Self {
        Self::new(
            MADOPILOT_STATUS_INVALID_ARGUMENT,
            MADOPILOT_ERROR_CATEGORY_ABI,
            message,
        )
    }

    /// A session that has closed refusing to start further work.
    pub(crate) fn closed(message: impl Into<String>) -> Self {
        Self::new(
            crate::status::MADOPILOT_STATUS_CLOSED,
            crate::status::MADOPILOT_ERROR_CATEGORY_CAPTURE,
            message,
        )
    }

    /// An invariant this boundary is responsible for that did not hold.
    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self::new(
            MADOPILOT_STATUS_INTERNAL,
            MADOPILOT_ERROR_CATEGORY_ABI,
            message,
        )
    }

    /// A facade failure, tagged with the subsystem that produced it.
    pub(crate) fn from_error(error: &Error, category: madopilot_error_category_t) -> Self {
        Self::new(code(error.status()), category, error.detail())
    }

    /// A package-loading failure, with the rule and the stage preserved.
    pub(crate) fn from_asset(fault: AssetFault) -> Self {
        let mut error = Self::new(
            code(fault.status()),
            MADOPILOT_ERROR_CATEGORY_ASSET,
            fault.to_string(),
        );
        error.asset = Some((
            asset_fault_code(fault.kind()),
            asset_stage_code(fault.stage()),
        ));

        error
    }

    /// Names the backend this failure came from.
    #[must_use]
    pub(crate) fn with_backend(mut self, backend: impl Into<String>) -> Self {
        self.backend = Some(backend.into());
        self
    }

    fn new(
        status: madopilot_status_t,
        category: madopilot_error_category_t,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status,
            category,
            message: message.into(),
            backend: None,
            asset: None,
        }
    }

    /// Returns the status the failing entry reports.
    pub(crate) const fn status(&self) -> madopilot_status_t {
        self.status
    }

    /// Builds the complete detail record, before any prefix is applied.
    fn detail(&self, struct_size: u32) -> madopilot_error_detail_t {
        let mut flags = 0;
        if self.asset.is_some() {
            flags |= MADOPILOT_ERROR_HAS_ASSET_DETAIL;
        }
        if self.backend.is_some() {
            flags |= MADOPILOT_ERROR_HAS_BACKEND;
        }
        let (asset_fault, asset_stage) = self
            .asset
            .unwrap_or((MADOPILOT_ASSET_FAULT_UNKNOWN, MADOPILOT_ASSET_STAGE_UNKNOWN));

        madopilot_error_detail_t {
            struct_size,
            flags,
            status: self.status,
            category: self.category,
            asset_fault,
            asset_stage,
            message: madopilot_str_t::borrowed(&self.message),
            backend: self
                .backend
                .as_deref()
                .map_or_else(madopilot_str_t::empty, madopilot_str_t::borrowed),
        }
    }
}

impl Versioned for madopilot_error_detail_t {
    // Through `category`: a caller that cannot store the status and the
    // subsystem is not describing an error, it is discarding one.
    const MANDATORY: usize = 16;
    const NAME: &'static str = "madopilot_error_detail_t";

    fn failure(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            status: MADOPILOT_STATUS_INTERNAL,
            category: MADOPILOT_ERROR_CATEGORY_UNSPECIFIED,
            asset_fault: MADOPILOT_ASSET_FAULT_UNKNOWN,
            asset_stage: MADOPILOT_ASSET_STAGE_UNKNOWN,
            message: madopilot_str_t::empty(),
            backend: madopilot_str_t::empty(),
        }
    }
}

/// Reports `fault` through `out_error` and returns its status.
///
/// A caller that passes no error output still gets the complete status, and the
/// library retains nothing.
///
/// # Safety
///
/// `out_error` must be null or a writable address for the call.
pub(crate) unsafe fn emit(
    out_error: *mut *mut madopilot_error_t,
    fault: Fault,
) -> madopilot_status_t {
    let status = fault.status();

    if !out_error.is_null() {
        let handle = handle::into_raw(fault);
        // SAFETY: the caller contract requires a writable address, and the
        // failure-state initialization at entry already wrote null through it.
        unsafe { out_error.write(handle) }
    }

    status
}

pub(crate) fn retain(handle: *const madopilot_error_t) -> madopilot_status_t {
    // SAFETY: the handle is null or one this module produced, and the caller
    // holds a live reference for the call, which is the documented contract.
    unsafe { handle::retain::<Fault>(handle) }

    MADOPILOT_STATUS_OK
}

pub(crate) fn release(handle: *mut madopilot_error_t) -> madopilot_status_t {
    // SAFETY: as `retain`, and the caller is giving up the reference it owns.
    unsafe { handle::release::<Fault>(handle) }

    MADOPILOT_STATUS_OK
}

pub(crate) fn describe(
    error: *const madopilot_error_t,
    out_detail: *mut madopilot_error_detail_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable, correctly aligned output address
    // whose `struct_size` it has already set, which `Out::begin` validates.
    let out = match unsafe { Out::begin(out_detail) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);

    // SAFETY: null is rejected below; otherwise the caller keeps the error
    // retained for the call.
    let Some(fault) = (unsafe { handle::borrow::<Fault>(error) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };

    // SAFETY: `out` was validated above and the detail's borrowed views point
    // into `fault`, which the caller keeps retained.
    unsafe { out.commit(fault.detail(out.declared_size())) }

    MADOPILOT_STATUS_OK
}

/// Maps a facade error into a fault of `category`.
pub(crate) fn facade(category: madopilot_error_category_t) -> impl Fn(Error) -> Fault {
    move |error| Fault::from_error(&error, category)
}

/// The status a facade [`Status`] reports as, for the few places that hold one
/// without an [`Error`] around it.
pub(crate) fn status_code(status: Status) -> madopilot_status_t {
    code(status)
}
