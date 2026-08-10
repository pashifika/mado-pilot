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

use mado_pilot::{
    AssetFault, AssetFaultKind, AssetLimits, AssetPackage, LoadStage, PreparedTemplate, Status,
};

use crate::boundary::{self, Out, Versioned, inputs, prefixes};
use crate::engine::{EngineHandle, madopilot_engine_t, report};
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

inputs! {
    impl Input for madopilot_package_source_t {
        // Through `path`. The archive-bytes kind supplies its content in the field
        // after it, and a caller that uses that kind says so with a larger size.
        const MANDATORY: usize = 24;
        const NAME: &'static str = "madopilot_package_source_t";
        const PREFIXES: &'static [usize] =
            prefixes!(madopilot_package_source_t, struct_size, kind, path, archive);
        const PRESENCE: &'static [(u32, usize)] = &[];

        fn defaults() -> Self {
            Self {
                struct_size: 0,
                kind: MADOPILOT_PACKAGE_SOURCE_DIRECTORY,
                path: madopilot_str_t::empty(),
                archive: madopilot_bytes_t::empty(),
            }
        }

        fn presence_bits(&self) -> u32 {
            // The second field is `kind`, a discriminant rather than a bit set.
            0
        }
    }
}

impl Versioned for madopilot_package_info_t {
    const MANDATORY: usize = 64;
    const NAME: &'static str = "madopilot_package_info_t";
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_package_info_t,
        struct_size,
        flags,
        template_count,
        package_id,
        package_version,
        license,
    );

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
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_template_info_t,
        struct_size,
        flags,
        width,
        height,
        min_score,
        id,
        backend,
        max_results,
        space,
    );

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
    if let Err(status) =
        // SAFETY: the caller supplies writable, correctly aligned output addresses.
        unsafe { boundary::begin_outputs(out_package, "out_package", out_error) }
    {
        return status;
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
    let Some(engine) = (unsafe { handle::borrow::<EngineHandle>(engine) }) else {
        return Err(Fault::abi("`engine` is null"));
    };
    // SAFETY: the caller keeps the operation structure readable for the call.
    let context = unsafe { operation::context(operation) }?;
    context.admit()?;

    // SAFETY: as above, for the source structure and the views it carries.
    let request = unsafe { boundary::read_input::<madopilot_package_source_t>(source) }?;
    // The engine's limits go in because one of the three source kinds is a length
    // the caller declares: the ceiling answers that length before this boundary
    // reads the memory behind it.
    // SAFETY: as above.
    let prepared = unsafe { package_source(&request, engine.limits()) }?;

    let package = match prepared {
        Prepared::Owned(source) => engine.load_package(&source, context.inner()),
        // Lent, not owned: the load reads the caller's view in place and returns
        // a package that owns its own content, so the view has to outlive this
        // call and nothing else.
        Prepared::Borrowed(bytes) => engine.load_archive_bytes(bytes, context.inner()),
    }
    .map_err(Fault::from_asset)?;
    hooks::reach(hooks::Site::AfterTemporary);
    context.commit()?;

    // SAFETY: `out_package` was validated by the entry before any work began.
    unsafe { out_package.write(handle::into_raw(package)) };

    Ok(())
}

/// What one load reads, for the tagged structure a caller supplied.
#[derive(Debug)]
enum Prepared<'a> {
    /// A source the loader opens for itself, and could be asked to open again.
    Owned(mado_pilot::PackageSource),
    /// An archive the caller lends for the duration of one call.
    Borrowed(&'a [u8]),
}

/// Prepares what a caller's tagged structure describes, under `limits`.
///
/// Two of the three kinds name something the loader opens itself, and both are
/// measured before anything is read. The archive-bytes kind is caller memory, and
/// it stays caller memory: the loader reads the view in place for the length of
/// one call, and the package it commits owns each template's content in its own
/// allocation, so nothing here needs an owned copy of the archive. That is the
/// point rather than an optimisation. Such a copy would be sized by the caller's
/// own declared length, up to whatever the configured source ceiling admits, and
/// the reference-counted representation a retained source needs cannot be
/// allocated fallibly on stable Rust — so the one failure mode a C boundary must
/// never have, a host terminated instead of given a status, would be the only
/// answer available when that allocation could not be satisfied. A view that is
/// only read does not raise the question.
///
/// Which leaves three questions about one view, in this order: is the view a
/// shape a view may have, is the length one this engine will read, and what do
/// the bytes say. The first is a malformed request and the released ABI fixes its
/// status and category, so it cannot be answered second — a null pointer carrying
/// a length is that refusal whether the length is one byte or a gigabyte. The
/// second is the configured source ceiling, answered on the declared length while
/// the caller's memory is still untouched; the loader applies the same ceiling to
/// what it reads, so this one is early rather than sufficient, and a caller that
/// tightened [`AssetLimits::with_max_total_compressed_bytes`] tightened both. The
/// third is the only one that reads the caller's memory.
///
/// [`AssetLimits::with_max_total_compressed_bytes`]: mado_pilot::AssetLimits::with_max_total_compressed_bytes
///
/// # Safety
///
/// Every view the structure carries must be readable and unmodified for the
/// call.
unsafe fn package_source<'a>(
    source: &'a madopilot_package_source_t,
    limits: AssetLimits,
) -> Result<Prepared<'a>, Fault> {
    match source.kind {
        MADOPILOT_PACKAGE_SOURCE_DIRECTORY => {
            // SAFETY: forwarded unchanged from this function's own contract.
            let path = unsafe { view::non_empty_string(source.path, "path") }?;
            Ok(Prepared::Owned(mado_pilot::PackageSource::directory(path)))
        }
        MADOPILOT_PACKAGE_SOURCE_ARCHIVE_FILE => {
            // SAFETY: as above.
            let path = unsafe { view::non_empty_string(source.path, "path") }?;
            Ok(Prepared::Owned(mado_pilot::PackageSource::archive_file(
                path,
            )))
        }
        MADOPILOT_PACKAGE_SOURCE_ARCHIVE_BYTES => {
            // The view's shape first, because a null pointer carrying a length is
            // a malformed request whatever that length is, and the released ABI
            // fixes that refusal. Then the length against the ceiling, which is
            // the check that has to happen while the caller's bytes are still
            // untouched. Only then the range itself.
            let declared =
                u64::try_from(view::byte_len(source.archive, "archive")?).map_err(|_| {
                    Fault::internal("an archive length inside the address space exceeds `u64`")
                })?;
            if declared > limits.max_total_compressed_bytes() {
                return Err(Fault::from_asset(AssetFault::new(
                    AssetFaultKind::ArchiveLimit,
                    LoadStage::Source,
                )));
            }

            // SAFETY: as above.
            let bytes = unsafe { view::bytes(source.archive, "archive") }?;
            if bytes.is_empty() {
                return Err(Fault::abi("`archive` is empty"));
            }
            Ok(Prepared::Borrowed(bytes))
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
    if let Err(status) =
        // SAFETY: the caller supplies writable, correctly aligned output addresses.
        unsafe { boundary::begin_outputs(out_template, "out_template", out_error) }
    {
        return status;
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
    let Some(engine) = (unsafe { handle::borrow::<EngineHandle>(engine) }) else {
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

#[cfg(test)]
mod tests {
    use mado_pilot::AssetLimits;

    use super::{
        MADOPILOT_PACKAGE_SOURCE_ARCHIVE_BYTES, Prepared, madopilot_bytes_t,
        madopilot_package_source_t, madopilot_str_t, package_source,
    };
    use crate::status::{MADOPILOT_STATUS_INVALID_ARGUMENT, MADOPILOT_STATUS_LIMIT_EXCEEDED};

    /// An archive-bytes source over `archive`, as a caller would declare it.
    fn source(archive: madopilot_bytes_t) -> madopilot_package_source_t {
        madopilot_package_source_t {
            struct_size: crate::layout::struct_size::<madopilot_package_source_t>(),
            kind: MADOPILOT_PACKAGE_SOURCE_ARCHIVE_BYTES,
            path: madopilot_str_t::empty(),
            archive,
        }
    }

    fn limits(ceiling: u64) -> AssetLimits {
        AssetLimits::ceiling()
            .with_max_total_compressed_bytes(ceiling)
            .expect("below the implementation ceiling")
    }

    /// The ceiling, against a buffer that is entirely readable.
    ///
    /// Configured small rather than exercised at 256 MiB, because what is being
    /// checked is the *order* — the refusal arrives on the declared length, before
    /// the view behind it is read at all.
    #[test]
    fn a_declared_length_above_the_configured_ceiling_is_refused_before_the_view_is_read() {
        let buffer = [0xffu8; 8];
        let view = madopilot_bytes_t {
            data: buffer.as_ptr(),
            len: buffer.len(),
        };

        // Bound to a local rather than passed as a temporary: what the boundary
        // returns borrows the structure it read, which is the compiler's half of
        // the rule that a lent archive may not outlive the call.
        let declared = source(view);

        // SAFETY: the view describes `buffer`, a live local, for the call.
        let refused = unsafe { package_source(&declared, limits(4)) }
            .expect_err("eight bytes are above a four byte ceiling");
        assert_eq!(refused.status(), MADOPILOT_STATUS_LIMIT_EXCEEDED);

        // SAFETY: as above. Exactly at the ceiling is a caller that fits, which
        // is what says the comparison is not off by one in the safe direction.
        let accepted = unsafe { package_source(&declared, limits(8)) }
            .expect("eight bytes are within an eight byte ceiling");
        match accepted {
            // The caller's own memory, not a copy of it. Asserting the address
            // rather than the contents is the point: an equal copy would pass a
            // comparison of bytes and would be the allocation this boundary does
            // not make.
            Prepared::Borrowed(bytes) => {
                assert!(std::ptr::eq(bytes.as_ptr(), buffer.as_ptr()));
                assert_eq!(bytes.len(), buffer.len());
            }
            Prepared::Owned(_) => panic!("an archive view is read in place, never owned"),
        }
    }

    /// A malformed view is malformed whatever it declares.
    ///
    /// The shape rules are frozen for ABI major 1 as invalid-argument refusals of
    /// the boundary's own category, so the ceiling may not answer first: a null
    /// pointer carrying a length is not a caller asking for too much memory, it is
    /// a caller describing a view that cannot exist.
    #[test]
    fn a_null_view_carrying_a_length_is_refused_as_malformed_rather_than_as_a_limit() {
        let view = madopilot_bytes_t {
            data: std::ptr::null(),
            len: usize::try_from(AssetLimits::MAX_TOTAL_COMPRESSED_BYTES + 1)
                .expect("the ceiling fits an object size on both release targets"),
        };

        let declared = source(view);

        // SAFETY: nothing reads the view: the pointer is refused on its shape.
        let refused = unsafe { package_source(&declared, limits(4)) }
            .expect_err("a null pointer with a length is not a view");
        assert_eq!(refused.status(), MADOPILOT_STATUS_INVALID_ARGUMENT);
    }

    /// An empty view is a caller with nothing to load.
    ///
    /// Refused here rather than by the archive stages, because a zero-length
    /// archive is a request this boundary can answer without reading anything.
    #[test]
    fn an_empty_archive_view_is_refused_as_malformed() {
        let view = madopilot_bytes_t {
            data: std::ptr::null(),
            len: 0,
        };

        let declared = source(view);

        // SAFETY: the one view a null pointer may describe is the empty one, and
        // no slice is formed over it.
        let refused = unsafe { package_source(&declared, limits(8)) }
            .expect_err("an empty view carries no archive");
        assert_eq!(refused.status(), MADOPILOT_STATUS_INVALID_ARGUMENT);
    }
}
