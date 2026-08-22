//! What the Rust compiler actually laid the public structures out as.
//!
//! The header is hand-written, so something has to prove it and the Rust
//! definitions agree. That proof is a comparison between two compilers rather
//! than an assertion in either of them: this module reports what `rustc`
//! produced, `tests/c/madopilot-abi-layout.c` reports what the C compiler
//! produced for the same declarations, and `examples/c-abi-check.rs` fails on
//! the first difference. See
//! `docs/adr/0004-c-header-authorship-and-abi-verification.md`.
//!
//! The report is also the evidence `G-010` froze, taken on both release
//! targets, so it is written in a format that is stable to diff and cheap to
//! paste into a record.

use std::mem::offset_of;

use crate::assets::{madopilot_package_t, madopilot_template_t};
use crate::capture::{madopilot_frame_t, madopilot_mapping_t, madopilot_session_t};
use crate::diagnostic::{madopilot_diagnostic_batch_t, madopilot_diagnostic_reader_t};
use crate::engine::{madopilot_engine_t, madopilot_target_list_t};
use crate::error::madopilot_error_t;
use crate::input::madopilot_input_receipt_t;
use crate::matching::madopilot_result_t;
use crate::operation::madopilot_cancellation_t;
use crate::table::madopilot_api_t;
use crate::types::{
    madopilot_build_info_t, madopilot_diagnostic_batch_info_t, madopilot_diagnostic_record_t,
    madopilot_engine_capabilities_t, madopilot_engine_options_t, madopilot_error_detail_t,
    madopilot_find_request_t, madopilot_frame_info_t, madopilot_frame_stamp_t, madopilot_image_t,
    madopilot_input_attempt_t, madopilot_input_capability_t, madopilot_input_descriptor_t,
    madopilot_input_event_t, madopilot_input_open_request_t, madopilot_input_receipt_info_t,
    madopilot_input_request_t, madopilot_map_request_t, madopilot_match_options_t,
    madopilot_match_t, madopilot_open_request_t, madopilot_operation_t, madopilot_package_info_t,
    madopilot_package_source_t, madopilot_permission_t, madopilot_pixel_rect_t,
    madopilot_replay_frame_t, madopilot_result_info_t, madopilot_session_info_t,
    madopilot_source_t, madopilot_target_t, madopilot_template_info_t,
};
use crate::view::{madopilot_bytes_t, madopilot_str_t};

/// Where one field sits inside its structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldLayout {
    /// The field's C name.
    pub name: &'static str,
    /// Its offset in bytes from the start of the structure.
    pub offset: usize,
}

/// One structure's measured size, alignment, and field offsets.
#[derive(Debug, Clone, Copy)]
pub struct TypeLayout {
    /// The structure's C name.
    pub name: &'static str,
    /// `sizeof` the structure.
    pub size: usize,
    /// `alignof` the structure.
    pub align: usize,
    /// Every field, in declaration order.
    pub fields: &'static [FieldLayout],
}

macro_rules! measure {
    ($ty:ty $(, $field:ident)* $(,)?) => {
        TypeLayout {
            name: stringify!($ty),
            size: size_of::<$ty>(),
            align: align_of::<$ty>(),
            fields: &[$(FieldLayout {
                name: stringify!($field),
                offset: offset_of!($ty, $field),
            }),*],
        }
    };
}

/// The measured layout of every structure the header declares.
///
/// Order matters: the C probe reports the same list in the same order, and the
/// comparison is positional as well as by name.
pub const LAYOUT: &[TypeLayout] = &[
    measure!(madopilot_str_t, data, len),
    measure!(madopilot_bytes_t, data, len),
    measure!(madopilot_pixel_rect_t, space, left, top, right, bottom),
    measure!(madopilot_engine_capabilities_t, struct_size, flags,),
    measure!(
        madopilot_engine_options_t,
        struct_size,
        flags,
        diagnostic_level,
        diagnostic_capacity,
    ),
    measure!(
        madopilot_diagnostic_batch_info_t,
        struct_size,
        flags,
        record_count,
        discarded_normal,
        discarded_debug,
    ),
    measure!(
        madopilot_diagnostic_record_t,
        struct_size,
        flags,
        sequence,
        timestamp_nanos,
        operation_id,
        activity_tag,
        level,
        kind,
        operation,
        status,
        target,
        frame,
        template_identity,
        source_space,
        destination_space,
        region,
        route,
        address_scope,
        evidence,
        input_fault,
        input_outcome,
        cleanup,
        permission_kind,
        permission_state,
        lifecycle,
        search_outcome,
        input_operations,
        partial_native_effect,
        used_fallback,
        reserved,
        requested,
        submitted,
        result_count,
        cleanup_released,
        cleanup_owed,
    ),
    measure!(
        madopilot_permission_t,
        struct_size,
        flags,
        kind,
        state,
        diagnostic_category,
        reserved,
        platform_code,
        platform_namespace,
        context,
    ),
    measure!(
        madopilot_input_capability_t,
        struct_size,
        flags,
        target,
        operation,
        delivery,
        support,
        address_scope,
        permission,
        evidence,
        focus_required,
        pointer_spaces,
        reserved,
    ),
    measure!(
        madopilot_input_open_request_t,
        struct_size,
        flags,
        requirement,
        reserved,
        required_pairs,
        preferred_pairs,
    ),
    measure!(
        madopilot_input_descriptor_t,
        struct_size,
        flags,
        target,
        known_pairs,
        supported_pairs,
        unknown_pairs,
        pointer_spaces,
        max_events,
    ),
    measure!(
        madopilot_input_event_t,
        struct_size,
        kind,
        space,
        button,
        key,
        key_value,
        x,
        y,
        horizontal,
        vertical,
        text,
        delay_nanos,
    ),
    measure!(
        madopilot_input_request_t,
        struct_size,
        flags,
        events,
        event_count,
        event_stride,
        deliveries,
        delivery_count,
        focus_policy,
        geometry_policy,
        source_frame,
        cleanup_max_events,
        reserved,
        cleanup_timeout_nanos,
    ),
    measure!(
        madopilot_input_receipt_info_t,
        struct_size,
        flags,
        target,
        outcome,
        selected_route,
        address_scope,
        attempt_count,
        submitted,
        last_submitted,
        evidence,
        fault,
        cleanup,
        cleanup_released,
        cleanup_owed,
    ),
    measure!(
        madopilot_input_attempt_t,
        struct_size,
        flags,
        route,
        address_scope,
        outcome,
        submitted,
        last_submitted,
        evidence,
        fault,
        reserved,
    ),
    measure!(
        madopilot_build_info_t,
        struct_size,
        flags,
        abi_major,
        abi_minor,
        table_size,
        reserved,
        library_version,
        required_backend,
    ),
    measure!(
        madopilot_operation_t,
        struct_size,
        flags,
        deadline_nanos,
        cancellation,
        activity_tag,
    ),
    measure!(
        madopilot_frame_stamp_t,
        struct_size,
        flags,
        stream,
        epoch,
        sequence,
        geometry,
    ),
    measure!(
        madopilot_frame_info_t,
        struct_size,
        flags,
        width,
        height,
        format,
        space,
        stride,
        bounds,
    ),
    measure!(
        madopilot_image_t,
        struct_size,
        flags,
        width,
        height,
        format,
        space,
        stride,
        bytes,
        region,
    ),
    measure!(
        madopilot_target_t,
        struct_size,
        flags,
        width,
        height,
        format,
        coordinate_spaces,
        name,
        provider,
        target,
        kind,
        capture,
        capture_permission,
        reserved,
    ),
    measure!(
        madopilot_session_info_t,
        struct_size,
        flags,
        stream,
        width,
        height,
        format,
        coordinate_spaces,
        target,
        accepts_input,
        reserved,
    ),
    measure!(
        madopilot_open_request_t,
        struct_size,
        flags,
        required_format,
        preferred_format,
    ),
    measure!(
        madopilot_map_request_t,
        struct_size,
        flags,
        format,
        clip_policy,
        region,
    ),
    measure!(
        madopilot_match_options_t,
        struct_size,
        flags,
        min_score,
        max_results,
        suppression,
    ),
    measure!(
        madopilot_find_request_t,
        struct_size,
        flags,
        frame,
        tmpl,
        options,
        region,
        clip_policy,
    ),
    measure!(
        madopilot_match_t,
        struct_size,
        flags,
        score,
        template_id,
        bounds,
    ),
    measure!(
        madopilot_result_info_t,
        struct_size,
        flags,
        match_count,
        backend_id,
        backend_version,
        searched,
    ),
    measure!(
        madopilot_package_info_t,
        struct_size,
        flags,
        template_count,
        package_id,
        package_version,
        license,
    ),
    measure!(
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
    ),
    measure!(
        madopilot_error_detail_t,
        struct_size,
        flags,
        status,
        category,
        asset_fault,
        asset_stage,
        message,
        backend,
    ),
    measure!(
        madopilot_replay_frame_t,
        struct_size,
        flags,
        width,
        height,
        format,
        continuity,
        pixels,
        captured_at_nanos,
        stride,
    ),
    measure!(
        madopilot_source_t,
        struct_size,
        kind,
        directory,
        frames,
        frame_count,
        frame_stride,
        target_name,
    ),
    measure!(madopilot_package_source_t, struct_size, kind, path, archive,),
    measure!(
        madopilot_api_t,
        struct_size,
        abi_major,
        abi_minor,
        reserved,
        describe_build,
        clock_now,
        status_text,
        cancellation_create,
        cancellation_retain,
        cancellation_release,
        cancellation_cancel,
        cancellation_is_cancelled,
        error_retain,
        error_release,
        error_describe,
        engine_create,
        engine_retain,
        engine_release,
        package_load,
        package_retain,
        package_release,
        package_describe,
        package_template_id,
        template_prepare_from_package,
        template_retain,
        template_release,
        template_describe,
        engine_discover,
        target_list_retain,
        target_list_release,
        target_list_count,
        target_list_get,
        session_open,
        session_retain,
        session_release,
        session_describe,
        session_close,
        session_is_closed,
        session_acquire_frame,
        frame_retain,
        frame_release,
        frame_stamp,
        frame_describe,
        frame_map,
        mapping_retain,
        mapping_release,
        mapping_describe,
        mapping_stamp,
        session_find,
        result_retain,
        result_release,
        result_describe,
        result_stamp,
        result_options,
        result_match,
        engine_create_with_options,
        engine_capabilities,
        engine_permission,
        target_list_input_capability,
        engine_input_descriptor,
        session_open_with_input,
        session_input_descriptor,
        session_send_input,
        input_receipt_retain,
        input_receipt_release,
        input_receipt_info,
        input_receipt_attempt_count,
        input_receipt_attempt_at,
        engine_take_diagnostic_reader,
        diagnostic_reader_retain,
        diagnostic_reader_release,
        diagnostic_reader_drain,
        diagnostic_batch_retain,
        diagnostic_batch_release,
        diagnostic_batch_info,
        diagnostic_batch_record_at,
    ),
];

macro_rules! handle {
    ($ty:ty) => {
        TypeLayout {
            name: stringify!($ty),
            size: size_of::<*const $ty>(),
            align: align_of::<*const $ty>(),
            fields: &[],
        }
    };
}

/// Every opaque handle, whose pointer size and alignment are all a caller sees.
///
/// The check that matters is that each one is a thin pointer: an opaque type a C
/// caller could not size must not become a fat pointer on the Rust side.
pub const HANDLE_POINTERS: &[TypeLayout] = &[
    handle!(madopilot_cancellation_t),
    handle!(madopilot_error_t),
    handle!(madopilot_engine_t),
    handle!(madopilot_target_list_t),
    handle!(madopilot_package_t),
    handle!(madopilot_template_t),
    handle!(madopilot_session_t),
    handle!(madopilot_frame_t),
    handle!(madopilot_mapping_t),
    handle!(madopilot_result_t),
    handle!(madopilot_input_receipt_t),
    handle!(madopilot_diagnostic_reader_t),
    handle!(madopilot_diagnostic_batch_t),
];

/// `sizeof T`, in the width every `struct_size` field is declared with.
///
/// A C caller writes `sizeof(T)`; this is what a Rust caller emulating one
/// writes instead, so the conversion is checked in one place rather than at
/// every call site.
///
/// # Panics
///
/// Panics for a structure larger than `u32::MAX`, which no public structure is.
#[must_use]
pub fn struct_size<T>() -> u32 {
    u32::try_from(size_of::<T>()).expect("a public structure is smaller than 4 GiB")
}

/// Renders the report in the line format the C probe also prints.
///
/// One `type` line per structure followed by one `field` line per field, so a
/// difference is a single-line diff naming the structure and the field.
#[must_use]
pub fn report() -> String {
    let mut lines = String::new();
    for layout in LAYOUT {
        lines.push_str(&format!(
            "type {} size={} align={}\n",
            layout.name, layout.size, layout.align
        ));
        for field in layout.fields {
            lines.push_str(&format!(
                "field {}.{} offset={}\n",
                layout.name, field.name, field.offset
            ));
        }
    }
    for layout in HANDLE_POINTERS {
        lines.push_str(&format!(
            "handle {} size={} align={}\n",
            layout.name, layout.size, layout.align
        ));
    }

    lines
}
