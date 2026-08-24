//! The negotiated function table, and the one symbol that hands it out.
//!
//! # Why a table instead of exported functions
//!
//! One exported symbol means one thing to negotiate. A caller that obtained a
//! table has already agreed the ABI major, the minimum minor, and how much of
//! the table it understands, so there is no way to bind to an entry that the
//! agreement did not cover. Fifty exported symbols would each be a separate
//! chance to link against something the negotiation would have refused.
//!
//! # Order is the contract
//!
//! Within an ABI major, a member's position is permanent. Later phases append;
//! nothing is reordered, removed, repurposed, or reserved as a null slot for
//! work that does not exist yet. The complete ABI 1.0 prefix runs from
//! information a caller needs before it has anything else through the dependency
//! order of everything it could then do; ABI 1.2 replaces the unreleased 1.1
//! draft with the native input and bounded diagnostic slice.
//!
//! **The order is frozen for ABI major 1** by ADR 0007 for the complete 1.0
//! prefix and ADR 0023 for the additive 1.2 suffix. No entry moves and none is
//! removed; a later minor appends to the end and raises `MADOPILOT_ABI_MINOR`.
//!
//! # Where the unsafe boundary is
//!
//! Every generated table entry is an `unsafe extern "C" fn` and is the only way
//! to reach the implementation behind it. The implementations are crate-private
//! and take the caller's raw pointers, and the contract that makes dereferencing
//! them sound — valid for the call, retained for the call — is the header's,
//! stated at each declaration and checked here as far as a check is possible:
//! null, alignment, declared size, tags, and arithmetic are all rejected before
//! any address is formed.

use crate::boundary::{Out, Versioned, boundary, prefixes};
use crate::status::{
    MADOPILOT_STATUS_INVALID_ARGUMENT, MADOPILOT_STATUS_OK, MADOPILOT_STATUS_UNSUPPORTED,
    madopilot_status_t,
};
use crate::types::madopilot_build_info_t;
use crate::view::madopilot_str_t;
use crate::{boundary as fence, hooks};

/// The ABI major version this library implements.
///
/// A different major is a different library: the loader names carry it, so an
/// incompatible ABI cannot be loaded by accident.
pub const MADOPILOT_ABI_MAJOR: u32 = 1;

/// The ABI minor version this library implements.
///
/// ABI 1.3 preserves the released ABI 1.0 and 1.2 prefixes and appends one-shot
/// OCR execution plus immutable owned result access.
pub const MADOPILOT_ABI_MINOR: u32 = 3;

/// The library package version, for [`madopilot_build_info_t::library_version`].
const LIBRARY_VERSION: &str = env!("CARGO_PKG_VERSION");

macro_rules! table {
    ($(
        $(#[$doc:meta])*
        $field:ident($($arg:ident: $ty:ty),* $(,)?) => $body:path;
    )*) => {
        /// The immutable ABI-major-one function table.
        ///
        /// Owned by the library, valid for as long as it is loaded, and never
        /// released. Every member returns [`madopilot_status_t`] and reports
        /// values only through validated output parameters.
        #[repr(C)]
        #[derive(Debug)]
        pub struct madopilot_api_t {
            /// `sizeof` this table as the library declares it.
            ///
            /// A caller uses the smaller of this and its own `sizeof` to decide
            /// which members exist.
            pub struct_size: u32,
            /// The ABI major version this table implements.
            pub abi_major: u32,
            /// The ABI minor version this table implements.
            pub abi_minor: u32,
            /// Padding to the natural alignment of the members below. Zero.
            pub reserved: u32,
            $(
                $(#[$doc])*
                pub $field: unsafe extern "C" fn($($ty),*) -> madopilot_status_t,
            )*
        }

        $(
            $(#[$doc])*
            unsafe extern "C" fn $field($($arg: $ty),*) -> madopilot_status_t {
                boundary(|| $body($($arg),*))
            }
        )*

        static TABLE: madopilot_api_t = madopilot_api_t {
            struct_size: MADOPILOT_API_SIZE_CURRENT,
            abi_major: MADOPILOT_ABI_MAJOR,
            abi_minor: MADOPILOT_ABI_MINOR,
            reserved: 0,
            $($field: $field,)*
        };
    };
}

table! {
    /// Reports what this library is and how much table it has.
    describe_build(out_info: *mut madopilot_build_info_t) => describe_build_impl;
    /// Reports the current instant in the library's monotonic clock domain.
    ///
    /// A caller adds to this to build the absolute deadline every operation
    /// structure carries. It is inside the mandatory prefix because without it
    /// a deadline cannot be constructed at all.
    clock_now(out_nanos: *mut u64) => clock_now_impl;
    /// Returns a stable lowercase slug for a status.
    ///
    /// Diagnostic text, borrowed from static storage and valid for the life of
    /// the library. A caller branches on the number, never on this.
    status_text(status: madopilot_status_t, out_text: *mut madopilot_str_t) => status_text_impl;

    /// Creates a cancellation handle that has not been cancelled.
    cancellation_create(
        out_cancellation: *mut *mut crate::operation::madopilot_cancellation_t,
    ) => crate::operation::create;
    /// Adds one owned reference. Null is a no-op.
    cancellation_retain(
        cancellation: *const crate::operation::madopilot_cancellation_t,
    ) => crate::operation::retain;
    /// Drops one owned reference. Null is a no-op.
    cancellation_release(
        cancellation: *mut crate::operation::madopilot_cancellation_t,
    ) => crate::operation::release;
    /// Requests cancellation of every operation carrying this handle.
    cancellation_cancel(
        cancellation: *const crate::operation::madopilot_cancellation_t,
    ) => crate::operation::cancel;
    /// Reports whether cancellation has been requested.
    cancellation_is_cancelled(
        cancellation: *const crate::operation::madopilot_cancellation_t,
        out_cancelled: *mut i32,
    ) => crate::operation::is_cancelled;

    /// Adds one owned reference. Null is a no-op.
    error_retain(error: *const crate::error::madopilot_error_t) => crate::error::retain;
    /// Drops one owned reference. Null is a no-op.
    error_release(error: *mut crate::error::madopilot_error_t) => crate::error::release;
    /// Reports a failure in structured form.
    ///
    /// The message and backend views are borrowed from the error handle.
    error_describe(
        error: *const crate::error::madopilot_error_t,
        out_detail: *mut crate::types::madopilot_error_detail_t,
    ) => crate::error::describe;

    /// Builds an engine over a deterministic source.
    engine_create(
        source: *const crate::types::madopilot_source_t,
        operation: *const crate::types::madopilot_operation_t,
        out_engine: *mut *mut crate::engine::madopilot_engine_t,
        out_error: *mut *mut crate::error::madopilot_error_t,
    ) => crate::engine::create;
    /// Adds one owned reference. Null is a no-op.
    engine_retain(engine: *const crate::engine::madopilot_engine_t) => crate::engine::retain;
    /// Drops one owned reference. Null is a no-op.
    engine_release(engine: *mut crate::engine::madopilot_engine_t) => crate::engine::release;

    /// Loads and validates an asset package.
    package_load(
        engine: *const crate::engine::madopilot_engine_t,
        source: *const crate::types::madopilot_package_source_t,
        operation: *const crate::types::madopilot_operation_t,
        out_package: *mut *mut crate::assets::madopilot_package_t,
        out_error: *mut *mut crate::error::madopilot_error_t,
    ) => crate::assets::package_load;
    /// Adds one owned reference. Null is a no-op.
    package_retain(package: *const crate::assets::madopilot_package_t) => crate::assets::package_retain;
    /// Drops one owned reference. Null is a no-op.
    package_release(package: *mut crate::assets::madopilot_package_t) => crate::assets::package_release;
    /// Reports what a loaded package declares about itself.
    package_describe(
        package: *const crate::assets::madopilot_package_t,
        out_info: *mut crate::types::madopilot_package_info_t,
    ) => crate::assets::package_describe;
    /// Returns the identity of the package's template at `index`.
    package_template_id(
        package: *const crate::assets::madopilot_package_t,
        index: usize,
        out_id: *mut madopilot_str_t,
    ) => crate::assets::package_template_id;
    /// Compiles the template `id` names in `package` for this engine's backend.
    template_prepare_from_package(
        engine: *const crate::engine::madopilot_engine_t,
        package: *const crate::assets::madopilot_package_t,
        id: madopilot_str_t,
        operation: *const crate::types::madopilot_operation_t,
        out_template: *mut *mut crate::assets::madopilot_template_t,
        out_error: *mut *mut crate::error::madopilot_error_t,
    ) => crate::assets::template_prepare_from_package;
    /// Adds one owned reference. Null is a no-op.
    template_retain(
        tmpl: *const crate::assets::madopilot_template_t,
    ) => crate::assets::template_retain;
    /// Drops one owned reference. Null is a no-op.
    template_release(
        tmpl: *mut crate::assets::madopilot_template_t,
    ) => crate::assets::template_release;
    /// Reports what a prepared template is.
    template_describe(
        tmpl: *const crate::assets::madopilot_template_t,
        out_info: *mut crate::types::madopilot_template_info_t,
    ) => crate::assets::template_describe;

    /// Lists the targets this engine's capture adapter can capture.
    engine_discover(
        engine: *const crate::engine::madopilot_engine_t,
        operation: *const crate::types::madopilot_operation_t,
        out_targets: *mut *mut crate::engine::madopilot_target_list_t,
        out_error: *mut *mut crate::error::madopilot_error_t,
    ) => crate::engine::discover;
    /// Adds one owned reference. Null is a no-op.
    target_list_retain(
        targets: *const crate::engine::madopilot_target_list_t,
    ) => crate::engine::target_list_retain;
    /// Drops one owned reference. Null is a no-op.
    target_list_release(
        targets: *mut crate::engine::madopilot_target_list_t,
    ) => crate::engine::target_list_release;
    /// Reports how many targets the list holds.
    target_list_count(
        targets: *const crate::engine::madopilot_target_list_t,
        out_count: *mut usize,
    ) => crate::engine::target_list_count;
    /// Describes the target at `index`.
    ///
    /// Its string views are borrowed from the target list.
    target_list_get(
        targets: *const crate::engine::madopilot_target_list_t,
        index: usize,
        out_target: *mut crate::types::madopilot_target_t,
    ) => crate::engine::target_list_get;

    /// Opens a capture session for the target at `index`.
    ///
    /// The target identity is copied, so the list may be released immediately.
    session_open(
        engine: *const crate::engine::madopilot_engine_t,
        targets: *const crate::engine::madopilot_target_list_t,
        index: usize,
        request: *const crate::types::madopilot_open_request_t,
        operation: *const crate::types::madopilot_operation_t,
        out_session: *mut *mut crate::capture::madopilot_session_t,
        out_error: *mut *mut crate::error::madopilot_error_t,
    ) => crate::capture::session_open;
    /// Adds one owned reference. Null is a no-op.
    session_retain(
        session: *const crate::capture::madopilot_session_t,
    ) => crate::capture::session_retain;
    /// Drops one owned reference. Null is a no-op.
    ///
    /// Releasing a session does not close it, and does not invalidate frames,
    /// mappings, or results the caller still holds.
    session_release(
        session: *mut crate::capture::madopilot_session_t,
    ) => crate::capture::session_release;
    /// Reports what the session accepted.
    session_describe(
        session: *const crate::capture::madopilot_session_t,
        out_info: *mut crate::types::madopilot_session_info_t,
    ) => crate::capture::session_describe;
    /// Closes the session and drains in-flight work under `operation`.
    ///
    /// Idempotent. Work starting after close returns
    /// [`MADOPILOT_STATUS_CLOSED`](crate::status::MADOPILOT_STATUS_CLOSED).
    session_close(
        session: *const crate::capture::madopilot_session_t,
        operation: *const crate::types::madopilot_operation_t,
        out_error: *mut *mut crate::error::madopilot_error_t,
    ) => crate::capture::session_close;
    /// Reports whether the session has finished closing.
    session_is_closed(
        session: *const crate::capture::madopilot_session_t,
        out_closed: *mut i32,
    ) => crate::capture::session_is_closed;

    /// Returns the session's maintained latest frame.
    session_acquire_frame(
        session: *const crate::capture::madopilot_session_t,
        operation: *const crate::types::madopilot_operation_t,
        out_frame: *mut *mut crate::capture::madopilot_frame_t,
        out_error: *mut *mut crate::error::madopilot_error_t,
    ) => crate::capture::session_acquire_frame;
    /// Adds one owned reference. Null is a no-op.
    frame_retain(frame: *const crate::capture::madopilot_frame_t) => crate::capture::frame_retain;
    /// Drops one owned reference. Null is a no-op.
    frame_release(frame: *mut crate::capture::madopilot_frame_t) => crate::capture::frame_release;
    /// Reports the frame's complete source identity.
    frame_stamp(
        frame: *const crate::capture::madopilot_frame_t,
        out_stamp: *mut crate::types::madopilot_frame_stamp_t,
    ) => crate::capture::frame_stamp;
    /// Reports the frame's pixel geometry.
    frame_describe(
        frame: *const crate::capture::madopilot_frame_t,
        out_info: *mut crate::types::madopilot_frame_info_t,
    ) => crate::capture::frame_describe;
    /// Maps the frame, or a region of it, into CPU-readable bytes.
    frame_map(
        frame: *const crate::capture::madopilot_frame_t,
        request: *const crate::types::madopilot_map_request_t,
        operation: *const crate::types::madopilot_operation_t,
        out_mapping: *mut *mut crate::capture::madopilot_mapping_t,
        out_error: *mut *mut crate::error::madopilot_error_t,
    ) => crate::capture::frame_map;
    /// Adds one owned reference. Null is a no-op.
    mapping_retain(
        mapping: *const crate::capture::madopilot_mapping_t,
    ) => crate::capture::mapping_retain;
    /// Drops one owned reference. Null is a no-op.
    ///
    /// At the final release every byte view borrowed from this mapping becomes
    /// invalid and the retained storage is released exactly once.
    mapping_release(
        mapping: *mut crate::capture::madopilot_mapping_t,
    ) => crate::capture::mapping_release;
    /// Reports the mapped image and its borrowed bytes.
    mapping_describe(
        mapping: *const crate::capture::madopilot_mapping_t,
        out_image: *mut crate::types::madopilot_image_t,
    ) => crate::capture::mapping_describe;
    /// Reports the identity of the frame this mapping came from.
    mapping_stamp(
        mapping: *const crate::capture::madopilot_mapping_t,
        out_stamp: *mut crate::types::madopilot_frame_stamp_t,
    ) => crate::capture::mapping_stamp;

    /// Searches one of the session's frames for one prepared template.
    ///
    /// A completed search with no qualifying match succeeds with a count of
    /// zero.
    session_find(
        session: *const crate::capture::madopilot_session_t,
        request: *const crate::types::madopilot_find_request_t,
        operation: *const crate::types::madopilot_operation_t,
        out_result: *mut *mut crate::matching::madopilot_result_t,
        out_error: *mut *mut crate::error::madopilot_error_t,
    ) => crate::matching::session_find;
    /// Adds one owned reference. Null is a no-op.
    result_retain(
        result: *const crate::matching::madopilot_result_t,
    ) => crate::matching::result_retain;
    /// Drops one owned reference. Null is a no-op.
    result_release(
        result: *mut crate::matching::madopilot_result_t,
    ) => crate::matching::result_release;
    /// Reports the count, the searched region, and the backend that answered.
    result_describe(
        result: *const crate::matching::madopilot_result_t,
        out_info: *mut crate::types::madopilot_result_info_t,
    ) => crate::matching::result_describe;
    /// Reports the complete identity of the frame that was searched.
    result_stamp(
        result: *const crate::matching::madopilot_result_t,
        out_stamp: *mut crate::types::madopilot_frame_stamp_t,
    ) => crate::matching::result_stamp;
    /// Reports the options the search actually ran under.
    result_options(
        result: *const crate::matching::madopilot_result_t,
        out_options: *mut crate::types::madopilot_match_options_t,
    ) => crate::matching::result_options;
    /// Describes the match at `index`.
    ///
    /// An index at or beyond the count is invalid argument, and the output is
    /// left in its failure state.
    result_match(
        result: *const crate::matching::madopilot_result_t,
        index: usize,
        out_match: *mut crate::types::madopilot_match_t,
    ) => crate::matching::result_match;

    /// Builds any source kind with explicit engine-wide options.
    engine_create_with_options(
        source: *const crate::types::madopilot_source_t,
        options: *const crate::types::madopilot_engine_options_t,
        operation: *const crate::types::madopilot_operation_t,
        out_engine: *mut *mut crate::engine::madopilot_engine_t,
        out_error: *mut *mut crate::error::madopilot_error_t,
    ) => crate::engine::create_with_options;
    /// Reports engine-wide native input and permission-probe capabilities.
    engine_capabilities(
        engine: *const crate::engine::madopilot_engine_t,
        out_capabilities: *mut crate::types::madopilot_engine_capabilities_t,
    ) => crate::input::engine_capabilities;
    /// Runs one non-prompting permission probe.
    engine_permission(
        engine: *const crate::engine::madopilot_engine_t,
        kind: crate::types::madopilot_permission_kind_t,
        operation: *const crate::types::madopilot_operation_t,
        out_permission: *mut crate::types::madopilot_permission_t,
        out_error: *mut *mut crate::error::madopilot_error_t,
    ) => crate::input::engine_permission;
    /// Reports one operation/route capability pair for a discovered target.
    target_list_input_capability(
        targets: *const crate::engine::madopilot_target_list_t,
        index: usize,
        operation: crate::types::madopilot_input_operation_kind_t,
        delivery: crate::types::madopilot_input_delivery_t,
        out_capability: *mut crate::types::madopilot_input_capability_t,
    ) => crate::input::target_list_input_capability;
    /// Queries one target's input descriptor without opening it.
    engine_input_descriptor(
        engine: *const crate::engine::madopilot_engine_t,
        targets: *const crate::engine::madopilot_target_list_t,
        index: usize,
        operation: *const crate::types::madopilot_operation_t,
        out_descriptor: *mut crate::types::madopilot_input_descriptor_t,
        out_error: *mut *mut crate::error::madopilot_error_t,
    ) => crate::input::engine_input_descriptor;
    /// Opens capture and input together without changing the frozen capture request.
    session_open_with_input(
        engine: *const crate::engine::madopilot_engine_t,
        targets: *const crate::engine::madopilot_target_list_t,
        index: usize,
        request: *const crate::types::madopilot_open_request_t,
        input_request: *const crate::types::madopilot_input_open_request_t,
        operation: *const crate::types::madopilot_operation_t,
        out_session: *mut *mut crate::capture::madopilot_session_t,
        out_error: *mut *mut crate::error::madopilot_error_t,
    ) => crate::capture::session_open_with_input;
    /// Reports the immutable input descriptor accepted by an open session.
    session_input_descriptor(
        session: *const crate::capture::madopilot_session_t,
        out_descriptor: *mut crate::types::madopilot_input_descriptor_t,
    ) => crate::input::session_input_descriptor;
    /// Sends one bounded sequence and returns an immutable owned receipt.
    session_send_input(
        session: *const crate::capture::madopilot_session_t,
        request: *const crate::types::madopilot_input_request_t,
        operation: *const crate::types::madopilot_operation_t,
        out_receipt: *mut *mut crate::input::madopilot_input_receipt_t,
        out_error: *mut *mut crate::error::madopilot_error_t,
    ) => crate::input::session_send_input;
    /// Adds one receipt reference. Null is a no-op.
    input_receipt_retain(
        receipt: *const crate::input::madopilot_input_receipt_t,
    ) => crate::input::receipt_retain;
    /// Drops one receipt reference. Null is a no-op.
    input_receipt_release(
        receipt: *mut crate::input::madopilot_input_receipt_t,
    ) => crate::input::receipt_release;
    /// Reports the fixed fields of an immutable receipt.
    input_receipt_info(
        receipt: *const crate::input::madopilot_input_receipt_t,
        out_info: *mut crate::types::madopilot_input_receipt_info_t,
    ) => crate::input::receipt_info;
    /// Reports the number of immutable route attempts.
    input_receipt_attempt_count(
        receipt: *const crate::input::madopilot_input_receipt_t,
        out_count: *mut usize,
    ) => crate::input::receipt_attempt_count;
    /// Reports one route attempt by index.
    input_receipt_attempt_at(
        receipt: *const crate::input::madopilot_input_receipt_t,
        index: usize,
        out_attempt: *mut crate::types::madopilot_input_attempt_t,
    ) => crate::input::receipt_attempt_at;
    /// Takes the one independent diagnostic reader; null means unavailable.
    engine_take_diagnostic_reader(
        engine: *const crate::engine::madopilot_engine_t,
        out_reader: *mut *mut crate::diagnostic::madopilot_diagnostic_reader_t,
    ) => crate::diagnostic::take_reader;
    /// Adds one diagnostic-reader reference. Null is a no-op.
    diagnostic_reader_retain(
        reader: *const crate::diagnostic::madopilot_diagnostic_reader_t,
    ) => crate::diagnostic::reader_retain;
    /// Drops one diagnostic-reader reference. Null is a no-op.
    diagnostic_reader_release(
        reader: *mut crate::diagnostic::madopilot_diagnostic_reader_t,
    ) => crate::diagnostic::reader_release;
    /// Drains records and losses without producing a diagnostic record itself.
    diagnostic_reader_drain(
        reader: *const crate::diagnostic::madopilot_diagnostic_reader_t,
        out_state: *mut crate::types::madopilot_diagnostic_drain_state_t,
        out_batch: *mut *mut crate::diagnostic::madopilot_diagnostic_batch_t,
    ) => crate::diagnostic::reader_drain;
    /// Adds one diagnostic-batch reference. Null is a no-op.
    diagnostic_batch_retain(
        batch: *const crate::diagnostic::madopilot_diagnostic_batch_t,
    ) => crate::diagnostic::batch_retain;
    /// Drops one diagnostic-batch reference. Null is a no-op.
    diagnostic_batch_release(
        batch: *mut crate::diagnostic::madopilot_diagnostic_batch_t,
    ) => crate::diagnostic::batch_release;
    /// Reports one immutable batch's count and exact losses.
    diagnostic_batch_info(
        batch: *const crate::diagnostic::madopilot_diagnostic_batch_t,
        out_info: *mut crate::types::madopilot_diagnostic_batch_info_t,
    ) => crate::diagnostic::batch_info;
    /// Reports one diagnostic record by index.
    diagnostic_batch_record_at(
        batch: *const crate::diagnostic::madopilot_diagnostic_batch_t,
        index: usize,
        out_record: *mut crate::types::madopilot_diagnostic_record_t,
    ) => crate::diagnostic::batch_record_at;
    /// Recognizes one exact retained frame and returns one immutable owned result.
    session_recognize(
        session: *const crate::capture::madopilot_session_t,
        request: *const crate::types::madopilot_ocr_request_t,
        operation: *const crate::types::madopilot_operation_t,
        out_result: *mut *mut crate::ocr::madopilot_ocr_result_t,
        out_error: *mut *mut crate::error::madopilot_error_t,
    ) => crate::ocr::session_recognize;
    /// Adds one owned OCR result reference. Null is a no-op.
    ocr_result_retain(
        result: *const crate::ocr::madopilot_ocr_result_t,
    ) => crate::ocr::result_retain;
    /// Drops one owned OCR result reference. Null is a no-op.
    ocr_result_release(
        result: *mut crate::ocr::madopilot_ocr_result_t,
    ) => crate::ocr::result_release;
    /// Reports the fixed description of one immutable OCR result.
    ocr_result_info(
        result: *const crate::ocr::madopilot_ocr_result_t,
        out_info: *mut crate::types::madopilot_ocr_result_info_t,
    ) => crate::ocr::result_info;
    /// Reports geometry and confidence for one recognized region.
    ocr_result_region_at(
        result: *const crate::ocr::madopilot_ocr_result_t,
        index: usize,
        out_region: *mut crate::types::madopilot_ocr_region_t,
    ) => crate::ocr::result_region_at;
    /// Reports borrowed normalized text for one recognized region.
    ocr_result_text_at(
        result: *const crate::ocr::madopilot_ocr_result_t,
        index: usize,
        out_text: *mut madopilot_str_t,
    ) => crate::ocr::result_text_at;
}

/// `sizeof` the complete frozen ABI 1.0 function-table prefix.
#[expect(
    clippy::cast_possible_truncation,
    reason = "guarded against the only value that could truncate"
)]
pub const MADOPILOT_API_SIZE_PHASE1: u32 = {
    let size = std::mem::offset_of!(madopilot_api_t, engine_create_with_options);
    assert!(
        size <= u32::MAX as usize,
        "the ABI 1.0 table prefix fits a u32 size"
    );
    size as u32
};

/// `sizeof` the complete frozen ABI 1.2 function-table prefix.
#[expect(
    clippy::cast_possible_truncation,
    reason = "guarded against the only value that could truncate"
)]
pub const MADOPILOT_API_SIZE_1_2: u32 = {
    let size = std::mem::offset_of!(madopilot_api_t, session_recognize);
    assert!(
        size <= u32::MAX as usize,
        "the ABI 1.2 table prefix fits a u32 size"
    );
    size as u32
};

/// `sizeof` the complete function table this library implements.
#[expect(
    clippy::cast_possible_truncation,
    reason = "guarded against the only value that could truncate"
)]
pub const MADOPILOT_API_SIZE_CURRENT: u32 = {
    let size = size_of::<madopilot_api_t>();
    assert!(size <= u32::MAX as usize, "the table fits a u32 size");
    size as u32
};

/// The table's mandatory prefix: everything through `status_text`.
///
/// A caller that knows less than this cannot report what it loaded and cannot
/// build a deadline, so negotiation refuses it rather than handing back a table
/// it could not use.
///
/// Read from the layout, as the offset of the member after the prefix, which is
/// what a prefix size is. It was written as `16 + 3 * 8`, which assumed the four
/// leading `u32`s pack without padding and that a function pointer is eight
/// bytes; the C header's own definition made the matching assumption. Neither is
/// assumed now, and the number is unchanged on both release targets.
// As `MADOPILOT_API_SIZE_PHASE1`: the assertion below is what makes the
// conversion a fact.
#[expect(
    clippy::cast_possible_truncation,
    reason = "guarded against the only value that could truncate"
)]
pub const MADOPILOT_API_SIZE_INFORMATION: u32 = {
    let offset = std::mem::offset_of!(madopilot_api_t, cancellation_create);
    assert!(offset <= u32::MAX as usize, "a prefix size fits a u32");
    offset as u32
};

/// Negotiates the ABI and returns the library's immutable function table.
///
/// This is the only symbol the library exports.
///
/// # Parameters
///
/// - `abi_major`: the ABI major the caller was built against. A different major
///   is refused; it is a different library.
/// - `min_abi_minor`: the oldest minor the caller can work with.
/// - `caller_struct_size`: `sizeof(madopilot_api_t)` as the caller's header
///   declares it. It must be at least
///   [`MADOPILOT_API_SIZE_INFORMATION`].
/// - `out_api`: receives the table, or null on failure.
///
/// # Returns
///
/// [`MADOPILOT_STATUS_OK`], [`MADOPILOT_STATUS_UNSUPPORTED`] for an ABI this
/// library does not implement, or [`MADOPILOT_STATUS_INVALID_ARGUMENT`] for a
/// null output or a size below the mandatory prefix.
///
/// # Safety
///
/// `out_api` must be null or a writable, correctly aligned address for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn madopilot_get_api(
    abi_major: u32,
    min_abi_minor: u32,
    caller_struct_size: usize,
    out_api: *mut *const madopilot_api_t,
) -> madopilot_status_t {
    boundary(|| {
        if out_api.is_null() {
            return MADOPILOT_STATUS_INVALID_ARGUMENT;
        }
        if !out_api
            .addr()
            .is_multiple_of(align_of::<*const madopilot_api_t>())
        {
            return MADOPILOT_STATUS_INVALID_ARGUMENT;
        }
        // SAFETY: the pointer is non-null and correctly aligned, and the
        // caller contract requires it to be writable for the call.
        unsafe { out_api.write(std::ptr::null()) };
        hooks::reach(hooks::Site::Entry);

        if abi_major != MADOPILOT_ABI_MAJOR
            || min_abi_minor > MADOPILOT_ABI_MINOR
            || min_abi_minor == 1
            || (min_abi_minor == 0 && caller_struct_size > MADOPILOT_API_SIZE_PHASE1 as usize)
        {
            return MADOPILOT_STATUS_UNSUPPORTED;
        }
        if caller_struct_size < MADOPILOT_API_SIZE_INFORMATION as usize {
            return MADOPILOT_STATUS_INVALID_ARGUMENT;
        }

        // The caller may know more of the table than this library has, which is
        // not an error: `struct_size` tells it how much of what it knows is
        // actually there, and it uses the smaller of the two.
        //
        // SAFETY: as above.
        unsafe { out_api.write(&raw const TABLE) };

        MADOPILOT_STATUS_OK
    })
}

impl Versioned for madopilot_build_info_t {
    // Through `table_size`: what a caller checks before it trusts anything else.
    const MANDATORY: usize = 20;
    const NAME: &'static str = "madopilot_build_info_t";
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_build_info_t,
        struct_size,
        flags,
        abi_major,
        abi_minor,
        table_size,
        reserved,
        library_version,
        required_backend,
    );
    const ZEROED_PADDING: &'static [(usize, usize)] = &[];

    fn failure(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            abi_major: 0,
            abi_minor: 0,
            table_size: 0,
            reserved: 0,
            library_version: madopilot_str_t::empty(),
            required_backend: madopilot_str_t::empty(),
        }
    }
}

fn describe_build_impl(out_info: *mut madopilot_build_info_t) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable output structure whose
    // `struct_size` it has set.
    let out = match unsafe { Out::begin(out_info) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);

    // SAFETY: `out` was validated above, and both views borrow from static
    // storage that lives as long as the library.
    unsafe {
        out.commit(madopilot_build_info_t {
            struct_size: out.declared_size(),
            flags: 0,
            abi_major: MADOPILOT_ABI_MAJOR,
            abi_minor: MADOPILOT_ABI_MINOR,
            table_size: MADOPILOT_API_SIZE_CURRENT,
            reserved: 0,
            library_version: madopilot_str_t::borrowed(LIBRARY_VERSION),
            required_backend: madopilot_str_t::borrowed(mado_pilot::REQUIRED_BACKEND),
        });
    }

    MADOPILOT_STATUS_OK
}

fn clock_now_impl(out_nanos: *mut u64) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable, correctly aligned output address.
    if let Err(fault) = unsafe { fence::begin_scalar_out(out_nanos, "out_nanos", 0_u64) } {
        return fault.status();
    }
    hooks::reach(hooks::Site::Entry);

    // SAFETY: `out_nanos` was validated above.
    unsafe { fence::commit_scalar(out_nanos, crate::operation::now_nanos()) };

    MADOPILOT_STATUS_OK
}

fn status_text_impl(
    status: madopilot_status_t,
    out_text: *mut madopilot_str_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable, correctly aligned output address.
    let prepared =
        unsafe { fence::begin_scalar_out(out_text, "out_text", madopilot_str_t::empty()) };
    if let Err(fault) = prepared {
        return fault.status();
    }
    hooks::reach(hooks::Site::Entry);

    // SAFETY: `out_text` was validated above, and the view borrows from static
    // storage that lives as long as the library.
    unsafe {
        fence::commit_scalar(
            out_text,
            madopilot_str_t::borrowed(crate::status::text(status)),
        );
    }

    MADOPILOT_STATUS_OK
}

#[cfg(test)]
mod tests {
    use std::ptr;

    use crate::layout::struct_size;
    use mado_pilot::PixelFormat;
    use mado_pilot_testkit::match_fixtures;

    use super::*;
    use crate::engine::madopilot_engine_t;
    use crate::hooks::{Site, armed};
    use crate::status::MADOPILOT_STATUS_INTERNAL_PANIC;
    use crate::types::{
        MADOPILOT_CONTINUITY_CONTINUOUS, MADOPILOT_PIXEL_FORMAT_RGBA8,
        MADOPILOT_SOURCE_REPLAY_MEMORY, madopilot_operation_t, madopilot_replay_frame_t,
        madopilot_source_t,
    };
    use crate::view::{madopilot_bytes_t, madopilot_str_t};

    /// Negotiates the table the way a C caller would.
    fn table() -> &'static madopilot_api_t {
        let mut api: *const madopilot_api_t = ptr::null();
        // SAFETY: `api` is a live, writable, correctly aligned local.
        let status = unsafe {
            madopilot_get_api(
                MADOPILOT_ABI_MAJOR,
                MADOPILOT_ABI_MINOR,
                size_of::<madopilot_api_t>(),
                &raw mut api,
            )
        };
        assert_eq!(status, MADOPILOT_STATUS_OK);

        // SAFETY: negotiation succeeded, so `api` names the library's static
        // table, which lives as long as the library does.
        unsafe { api.as_ref() }.expect("a negotiated table is never null")
    }

    fn operation() -> madopilot_operation_t {
        madopilot_operation_t {
            struct_size: struct_size::<madopilot_operation_t>(),
            flags: 0,
            deadline_nanos: 0,
            cancellation: ptr::null(),
            activity_tag: 0,
        }
    }

    #[test]
    fn a_panic_before_any_output_is_contained() {
        let api = table();
        let mut info = madopilot_build_info_t::failure(struct_size::<madopilot_build_info_t>());

        let status = armed(Site::Entry, || {
            // SAFETY: `info` is a live, writable, correctly aligned local whose
            // `struct_size` is set.
            unsafe { (api.describe_build)(&raw mut info) }
        });

        assert_eq!(status, MADOPILOT_STATUS_INTERNAL_PANIC);
        assert_eq!(info.abi_major, 0, "the output stays in its failure state");
        assert_eq!(info.table_size, 0);
        assert!(info.library_version.data.is_null());
    }

    #[test]
    fn the_library_stays_usable_after_a_contained_panic() {
        let api = table();
        let mut info = madopilot_build_info_t::failure(struct_size::<madopilot_build_info_t>());

        // SAFETY: as above.
        let panicked = armed(Site::Entry, || unsafe {
            (api.describe_build)(&raw mut info)
        });
        assert_eq!(panicked, MADOPILOT_STATUS_INTERNAL_PANIC);

        // SAFETY: as above.
        let recovered = unsafe { (api.describe_build)(&raw mut info) };
        assert_eq!(recovered, MADOPILOT_STATUS_OK);
        assert_eq!(info.abi_major, MADOPILOT_ABI_MAJOR);
        assert_eq!(info.table_size, MADOPILOT_API_SIZE_CURRENT);
    }

    #[test]
    fn a_panic_after_temporary_allocation_exposes_no_handle() {
        let api = table();
        let pixels = match_fixtures::scene_pixels(PixelFormat::Rgba8);
        let frame = madopilot_replay_frame_t {
            struct_size: struct_size::<madopilot_replay_frame_t>(),
            flags: 0,
            width: match_fixtures::SCENE.width(),
            height: match_fixtures::SCENE.height(),
            format: MADOPILOT_PIXEL_FORMAT_RGBA8,
            continuity: MADOPILOT_CONTINUITY_CONTINUOUS,
            pixels: madopilot_bytes_t::borrowed(&pixels),
            captured_at_nanos: 0,
            stride: 0,
        };
        let source = madopilot_source_t {
            struct_size: struct_size::<madopilot_source_t>(),
            kind: MADOPILOT_SOURCE_REPLAY_MEMORY,
            directory: madopilot_str_t::empty(),
            frames: &raw const frame,
            frame_count: 1,
            frame_stride: size_of::<madopilot_replay_frame_t>(),
            target_name: madopilot_str_t::empty(),
        };
        let operation = operation();
        let mut engine: *mut madopilot_engine_t = ptr::null_mut();
        let mut error: *mut crate::error::madopilot_error_t = ptr::null_mut();

        // The engine has been constructed by the time this site is reached, so
        // the unwind is what has to release it.
        let status = armed(Site::AfterTemporary, || {
            // SAFETY: every pointer is a live local that outlives the call, and
            // the pixel view borrows a live `Vec`.
            unsafe {
                (api.engine_create)(
                    &raw const source,
                    &raw const operation,
                    &raw mut engine,
                    &raw mut error,
                )
            }
        });

        assert_eq!(status, MADOPILOT_STATUS_INTERNAL_PANIC);
        assert!(engine.is_null(), "no partial handle is exposed");
        assert!(error.is_null(), "a contained panic produces no owned error");

        // The same request succeeds afterwards, which is what "poison only what
        // cannot be proven safe" means in practice: nothing was poisoned.
        // SAFETY: as above.
        let recovered = unsafe {
            (api.engine_create)(
                &raw const source,
                &raw const operation,
                &raw mut engine,
                &raw mut error,
            )
        };
        assert_eq!(recovered, MADOPILOT_STATUS_OK);
        assert!(!engine.is_null());
        // SAFETY: the engine was produced by this table and is owned here.
        unsafe { (api.engine_release)(engine) };
    }
}
