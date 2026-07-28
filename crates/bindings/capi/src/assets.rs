//! Asset packages and the templates compiled from them.
//!
//! # Why loading has its own error detail
//!
//! Package loading is the one Phase 1 operation whose Rust contract reports more
//! than a status. The boundary preserves both halves of it — which rule was
//! broken and how far loading had got — in
//! [`madopilot_error_detail_t`](crate::types::madopilot_error_detail_t), because
//! a caller that has to read a message to tell a bad content hash from an unsafe
//! entry path has been given a worse contract than the Rust one it wraps.
//!
//! # A missing template is not an asset failure
//!
//! Asking a loaded package for an identity it never declared is invalid
//! argument, not `MADOPILOT_STATUS_ASSET_INVALID`. A package that loaded is
//! valid; the mistake is the caller's.

use mado_pilot::{AssetPackage, Engine, PreparedTemplate, Status};

use crate::boundary::{self, Input, Out, Versioned};
use crate::engine::{madopilot_engine_t, report};
use crate::error::{Fault, madopilot_error_t};
use crate::handle::opaque;
use crate::operation;
use crate::status::{
    MADOPILOT_ERROR_CATEGORY_ASSET, MADOPILOT_ERROR_CATEGORY_VISION,
    MADOPILOT_STATUS_INVALID_ARGUMENT, MADOPILOT_STATUS_OK, madopilot_status_t,
};
use crate::types::{
    MADOPILOT_PACKAGE_SOURCE_ARCHIVE_BYTES, MADOPILOT_PACKAGE_SOURCE_ARCHIVE_FILE,
    MADOPILOT_PACKAGE_SOURCE_DIRECTORY, MADOPILOT_SPACE_CAPTURE_PIXELS, madopilot_operation_t,
    madopilot_package_info_t, madopilot_package_source_t, madopilot_template_info_t, space_code,
};
use crate::view::{self, madopilot_bytes_t, madopilot_str_t};
use crate::{handle, hooks};

opaque! {
    /// A loaded, validated, immutable asset package.
    ///
    /// Every string a package reports is borrowed from this handle. A template
    /// prepared from it is independent of it: releasing the package does not
    /// disturb a template that is still retained.
    madopilot_package_t => AssetPackage
}

opaque! {
    /// A template compiled for this engine's matching backend.
    ///
    /// Independent of the package it came from and of the engine that compiled
    /// it, and usable from several threads at once while retained.
    madopilot_template_t => PreparedTemplate
}

impl Input for madopilot_package_source_t {
    // Through `path`. The archive-bytes kind supplies its content in the field
    // after it, and a caller that uses that kind says so with a larger size.
    const MANDATORY: usize = 24;
    const NAME: &'static str = "madopilot_package_source_t";

    fn defaults() -> Self {
        Self {
            struct_size: 0,
            kind: MADOPILOT_PACKAGE_SOURCE_DIRECTORY,
            path: madopilot_str_t::empty(),
            archive: madopilot_bytes_t::empty(),
        }
    }
}

impl Versioned for madopilot_package_info_t {
    const MANDATORY: usize = 64;
    const NAME: &'static str = "madopilot_package_info_t";

    fn failure(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            template_count: 0,
            package_id: madopilot_str_t::empty(),
            package_version: madopilot_str_t::empty(),
            license: madopilot_str_t::empty(),
        }
    }
}

impl Versioned for madopilot_template_info_t {
    const MANDATORY: usize = 64;
    const NAME: &'static str = "madopilot_template_info_t";

    fn failure(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            width: 0,
            height: 0,
            min_score: 0.0,
            id: madopilot_str_t::empty(),
            backend: madopilot_str_t::empty(),
            max_results: 0,
            space: MADOPILOT_SPACE_CAPTURE_PIXELS,
        }
    }
}

pub(crate) fn package_load(
    engine: *const madopilot_engine_t,
    source: *const madopilot_package_source_t,
    operation: *const madopilot_operation_t,
    out_package: *mut *mut madopilot_package_t,
    out_error: *mut *mut madopilot_error_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies writable, correctly aligned output addresses.
    if let Err(fault) = unsafe {
        boundary::begin_handle_out(out_package, "out_package")
            .and_then(|()| boundary::begin_error_out(out_error))
    } {
        return fault.status();
    }
    hooks::reach(hooks::Site::Entry);

    // SAFETY: `out_error` was validated above.
    unsafe {
        report(
            out_error,
            run_package_load(engine, source, operation, out_package),
        )
    }
}

fn run_package_load(
    engine: *const madopilot_engine_t,
    source: *const madopilot_package_source_t,
    operation: *const madopilot_operation_t,
    out_package: *mut *mut madopilot_package_t,
) -> Result<(), Fault> {
    // SAFETY: the caller keeps the engine retained for the call.
    let Some(engine) = (unsafe { handle::borrow::<Engine>(engine) }) else {
        return Err(Fault::abi("`engine` is null"));
    };
    // SAFETY: the caller keeps the operation structure readable for the call.
    let context = unsafe { operation::context(operation) }?;
    context.admit()?;

    // SAFETY: as above, for the source structure and the views it carries.
    let request = unsafe { boundary::read_input::<madopilot_package_source_t>(source) }?;
    // SAFETY: as above.
    let configured = unsafe { package_source(&request) }?;

    let package = engine
        .load_package(&configured, context.inner())
        .map_err(Fault::from_asset)?;
    hooks::reach(hooks::Site::AfterTemporary);
    context.commit()?;

    // SAFETY: `out_package` was validated by the entry before any work began.
    unsafe { out_package.write(handle::into_raw(package)) };

    Ok(())
}

/// # Safety
///
/// Every view the structure carries must be readable for the call.
unsafe fn package_source(
    source: &madopilot_package_source_t,
) -> Result<mado_pilot::PackageSource, Fault> {
    match source.kind {
        MADOPILOT_PACKAGE_SOURCE_DIRECTORY => {
            // SAFETY: forwarded unchanged from this function's own contract.
            let path = unsafe { view::non_empty_string(source.path, "path") }?;
            Ok(mado_pilot::PackageSource::directory(path))
        }
        MADOPILOT_PACKAGE_SOURCE_ARCHIVE_FILE => {
            // SAFETY: as above.
            let path = unsafe { view::non_empty_string(source.path, "path") }?;
            Ok(mado_pilot::PackageSource::archive_file(path))
        }
        MADOPILOT_PACKAGE_SOURCE_ARCHIVE_BYTES => {
            // SAFETY: as above.
            let bytes = unsafe { view::bytes(source.archive, "archive") }?;
            if bytes.is_empty() {
                return Err(Fault::abi("`archive` is empty"));
            }
            Ok(mado_pilot::PackageSource::archive_bytes(bytes.to_vec()))
        }
        other => Err(Fault::abi(format!(
            "unrecognized package source kind {other}"
        ))),
    }
}

pub(crate) fn package_retain(package: *const madopilot_package_t) -> madopilot_status_t {
    // SAFETY: the handle is null or one this module produced, and the caller
    // holds a live reference for the call.
    unsafe { handle::retain::<AssetPackage>(package) }

    MADOPILOT_STATUS_OK
}

pub(crate) fn package_release(package: *mut madopilot_package_t) -> madopilot_status_t {
    // SAFETY: as `package_retain`, and the caller is giving up its reference.
    unsafe { handle::release::<AssetPackage>(package) }

    MADOPILOT_STATUS_OK
}

pub(crate) fn package_describe(
    package: *const madopilot_package_t,
    out_info: *mut madopilot_package_info_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable output structure whose
    // `struct_size` it has set.
    let out = match unsafe { Out::begin(out_info) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);

    // SAFETY: the caller keeps the handle retained for the call.
    let Some(package) = (unsafe { handle::borrow::<AssetPackage>(package) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };

    let manifest = package.manifest();
    let count = u64::try_from(package.template_count()).unwrap_or(u64::MAX);
    // SAFETY: `out` was validated above, and every view borrows from the
    // package the caller keeps retained.
    unsafe {
        out.commit(madopilot_package_info_t {
            struct_size: out.declared_size(),
            flags: 0,
            template_count: count,
            package_id: madopilot_str_t::borrowed(manifest.package_id()),
            package_version: madopilot_str_t::borrowed(manifest.package_version()),
            license: madopilot_str_t::borrowed(manifest.license()),
        });
    }

    MADOPILOT_STATUS_OK
}

pub(crate) fn package_template_id(
    package: *const madopilot_package_t,
    index: usize,
    out_id: *mut madopilot_str_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable, correctly aligned output address.
    let prepared =
        unsafe { boundary::begin_scalar_out(out_id, "out_id", madopilot_str_t::empty()) };
    if let Err(fault) = prepared {
        return fault.status();
    }
    hooks::reach(hooks::Site::Entry);

    // SAFETY: the caller keeps the handle retained for the call.
    let Some(package) = (unsafe { handle::borrow::<AssetPackage>(package) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };

    match boundary::index_within(index, package.template_count(), "template") {
        Ok(index) => {
            let Some(id) = package.template_ids().nth(index) else {
                return MADOPILOT_STATUS_INVALID_ARGUMENT;
            };
            // SAFETY: `out_id` was validated above, and the view borrows from
            // the package the caller keeps retained.
            unsafe { boundary::commit_scalar(out_id, madopilot_str_t::borrowed(id.as_str())) };
            MADOPILOT_STATUS_OK
        }
        Err(fault) => fault.status(),
    }
}

pub(crate) fn template_prepare_from_package(
    engine: *const madopilot_engine_t,
    package: *const madopilot_package_t,
    id: madopilot_str_t,
    operation: *const madopilot_operation_t,
    out_template: *mut *mut madopilot_template_t,
    out_error: *mut *mut madopilot_error_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies writable, correctly aligned output addresses.
    if let Err(fault) = unsafe {
        boundary::begin_handle_out(out_template, "out_template")
            .and_then(|()| boundary::begin_error_out(out_error))
    } {
        return fault.status();
    }
    hooks::reach(hooks::Site::Entry);

    // SAFETY: `out_error` was validated above.
    unsafe {
        report(
            out_error,
            run_template_prepare(engine, package, id, operation, out_template),
        )
    }
}

fn run_template_prepare(
    engine: *const madopilot_engine_t,
    package: *const madopilot_package_t,
    id: madopilot_str_t,
    operation: *const madopilot_operation_t,
    out_template: *mut *mut madopilot_template_t,
) -> Result<(), Fault> {
    // SAFETY: the caller keeps both handles retained for the call.
    let Some(engine) = (unsafe { handle::borrow::<Engine>(engine) }) else {
        return Err(Fault::abi("`engine` is null"));
    };
    // SAFETY: as above.
    let Some(package) = (unsafe { handle::borrow::<AssetPackage>(package) }) else {
        return Err(Fault::abi("`package` is null"));
    };
    // SAFETY: the caller keeps the identity view readable for the call.
    let id = unsafe { view::non_empty_string(id, "id") }?;
    // SAFETY: the caller keeps the operation structure readable for the call.
    let context = unsafe { operation::context(operation) }?;
    context.admit()?;

    // Resolved here rather than through `Engine::prepare_from_package`, which
    // flattens the asset layer's typed fault into a plain error. Resolution is
    // the only step of that method that can produce one, and a C caller that
    // loses it is left with a status shared by every other malformed request —
    // the reason `MADOPILOT_ASSET_FAULT_UNKNOWN_TEMPLATE` existed with nothing
    // able to produce it. A Rust caller recovers the same detail the same way,
    // by asking the package; see `AssetPackage::resolve_template`.
    //
    // A package that loaded is valid, so an identity it never declared is still
    // invalid argument rather than an asset failure. The kind and the stage say
    // which mistake it was; the status says whose.
    // No checkpoint between the two: resolution is a lookup in a committed
    // package, not work worth interrupting, and the entry already admitted.
    let source = package.resolve_template(id).map_err(Fault::from_asset)?;

    let prepared = engine
        .prepare_template(&source, context.inner())
        .map_err(|error| {
            let category = if error.status() == Status::InvalidArgument {
                MADOPILOT_ERROR_CATEGORY_ASSET
            } else {
                MADOPILOT_ERROR_CATEGORY_VISION
            };
            Fault::from_error(&error, category).with_backend(engine.backend().id())
        })?;
    hooks::reach(hooks::Site::AfterTemporary);
    context.commit()?;

    // SAFETY: `out_template` was validated by the entry before any work began.
    unsafe { out_template.write(handle::into_raw(prepared)) };

    Ok(())
}

pub(crate) fn template_retain(template: *const madopilot_template_t) -> madopilot_status_t {
    // SAFETY: the handle is null or one this module produced, and the caller
    // holds a live reference for the call.
    unsafe { handle::retain::<PreparedTemplate>(template) }

    MADOPILOT_STATUS_OK
}

pub(crate) fn template_release(template: *mut madopilot_template_t) -> madopilot_status_t {
    // SAFETY: as `template_retain`, and the caller is giving up its reference.
    unsafe { handle::release::<PreparedTemplate>(template) }

    MADOPILOT_STATUS_OK
}

pub(crate) fn template_describe(
    template: *const madopilot_template_t,
    out_info: *mut madopilot_template_info_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable output structure whose
    // `struct_size` it has set.
    let out = match unsafe { Out::begin(out_info) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);

    // SAFETY: the caller keeps the handle retained for the call.
    let Some(template) = (unsafe { handle::borrow::<PreparedTemplate>(template) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };

    let defaults = template.defaults();
    // SAFETY: `out` was validated above, and both views borrow from the
    // template the caller keeps retained.
    unsafe {
        out.commit(madopilot_template_info_t {
            struct_size: out.declared_size(),
            flags: 0,
            width: template.extent().width(),
            height: template.extent().height(),
            min_score: defaults.min_score(),
            id: madopilot_str_t::borrowed(template.id().as_str()),
            backend: madopilot_str_t::borrowed(template.backend().as_str()),
            max_results: defaults.max_results(),
            // Phase 1 prepares templates from packages, whose declared geometry
            // the loader has already refused unless it is capture pixels.
            space: space_code(mado_pilot::CoordinateSpace::CapturePixels),
        });
    }

    MADOPILOT_STATUS_OK
}
