/*
 * Reports what the C compiler laid the public structures out as.
 *
 * The header is hand-written, so something has to prove it and the Rust
 * definitions agree. This program is one half of that proof: it prints sizes,
 * alignments, and field offsets as the C compiler produced them, and
 * `examples/c-abi-check.rs` compares the output line by line against the same
 * values measured from the Rust `#[repr(C)]` definitions. Two compilers, one
 * comparison; a divergence names the structure and the field.
 *
 * The _Static_asserts below are the part that does not need the comparison:
 * they hold on any conforming target and would fail at compile time.
 *
 * Requires C11 for _Static_assert.
 */

#include <stdio.h>
#include <stddef.h>

#include "madopilot/madopilot.h"

#if defined(_MSC_VER)
#  define MADOPILOT_ALIGNOF(T) __alignof(T)
#else
#  define MADOPILOT_ALIGNOF(T) _Alignof(T)
#endif

#define TYPE(T) \
    printf("type %s size=%zu align=%zu\n", #T, sizeof(T), (size_t)MADOPILOT_ALIGNOF(T))
#define FIELD(T, F) \
    printf("field %s.%s offset=%zu\n", #T, #F, (size_t)offsetof(T, F))
#define HANDLE(T)                                                     \
    printf("handle %s size=%zu align=%zu\n", #T, sizeof(const T*),    \
           (size_t)MADOPILOT_ALIGNOF(const T*))

/* Every versioned structure begins with struct_size. A caller reads that field
 * before it knows anything else about the structure, so its position is the one
 * layout property the whole negotiation rests on. */
#define FIRST_FIELD_IS_STRUCT_SIZE(T) \
    _Static_assert(offsetof(T, struct_size) == 0, #T " begins with struct_size")

#if MADOPILOT_ABI_MINOR >= 2
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_engine_capabilities_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_engine_options_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_diagnostic_batch_info_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_diagnostic_record_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_permission_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_input_capability_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_input_open_request_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_input_descriptor_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_input_event_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_input_request_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_input_receipt_info_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_input_attempt_t);
#define MADOPILOT_FIELD_HAS_TYPE(T, F, P) \
    _Static_assert(_Generic(&((T*)0)->F, P: 1, default: 0), \
                   #T "." #F " has the required public type")
MADOPILOT_FIELD_HAS_TYPE(madopilot_input_receipt_info_t, attempt_count, uint64_t*);
MADOPILOT_FIELD_HAS_TYPE(madopilot_input_receipt_info_t, submitted, uint64_t*);
MADOPILOT_FIELD_HAS_TYPE(madopilot_input_receipt_info_t, last_submitted, uint64_t*);
MADOPILOT_FIELD_HAS_TYPE(madopilot_input_receipt_info_t, cleanup_released, uint64_t*);
MADOPILOT_FIELD_HAS_TYPE(madopilot_input_receipt_info_t, cleanup_owed, uint64_t*);
MADOPILOT_FIELD_HAS_TYPE(madopilot_input_attempt_t, submitted, uint64_t*);
MADOPILOT_FIELD_HAS_TYPE(madopilot_input_attempt_t, last_submitted, uint64_t*);
MADOPILOT_FIELD_HAS_TYPE(madopilot_diagnostic_record_t, region,
                         madopilot_pixel_rect_t*);
#if MADOPILOT_ABI_MINOR >= 4
MADOPILOT_FIELD_HAS_TYPE(madopilot_ocr_profile_options_t, kind, int32_t*);
MADOPILOT_FIELD_HAS_TYPE(madopilot_ocr_zone_scan_request_t, zones,
                         const madopilot_ocr_zone_t**);
MADOPILOT_FIELD_HAS_TYPE(madopilot_ocr_zone_scan_request_t, zone_count, size_t*);
MADOPILOT_FIELD_HAS_TYPE(madopilot_ocr_zone_scan_request_t, zone_stride, size_t*);
MADOPILOT_FIELD_HAS_TYPE(madopilot_ocr_zone_scan_result_info_t, zone_count,
                         uint64_t*);
MADOPILOT_FIELD_HAS_TYPE(madopilot_ocr_zone_scan_result_info_t,
                         unique_candidate_count, uint64_t*);
MADOPILOT_FIELD_HAS_TYPE(madopilot_ocr_zone_scan_result_info_t,
                         membership_count, uint64_t*);
MADOPILOT_FIELD_HAS_TYPE(madopilot_ocr_zone_result_t, region_count, uint64_t*);
MADOPILOT_FIELD_HAS_TYPE(madopilot_diagnostic_record_t, ocr_source_envelope,
                         madopilot_pixel_rect_t*);
MADOPILOT_FIELD_HAS_TYPE(madopilot_diagnostic_record_t, ocr_zone_count,
                         uint64_t*);
MADOPILOT_FIELD_HAS_TYPE(madopilot_diagnostic_record_t,
                         ocr_unique_candidate_count, uint64_t*);
MADOPILOT_FIELD_HAS_TYPE(madopilot_diagnostic_record_t, ocr_membership_count,
                         uint64_t*);
MADOPILOT_FIELD_HAS_TYPE(madopilot_diagnostic_record_t, ocr_result_bytes,
                         uint64_t*);
#endif
#if MADOPILOT_ABI_MINOR >= 5
MADOPILOT_FIELD_HAS_TYPE(madopilot_ocr_provider_options_t, policy, int32_t*);
MADOPILOT_FIELD_HAS_TYPE(madopilot_ocr_provider_descriptor_t, requested_policy, int32_t*);
MADOPILOT_FIELD_HAS_TYPE(madopilot_ocr_provider_descriptor_t, active_provider, int32_t*);
MADOPILOT_FIELD_HAS_TYPE(madopilot_ocr_provider_descriptor_t, initialization_fell_back, uint32_t*);
MADOPILOT_FIELD_HAS_TYPE(madopilot_ocr_provider_descriptor_t, fallback_reason, int32_t*);
#endif
#undef MADOPILOT_FIELD_HAS_TYPE
#endif

FIRST_FIELD_IS_STRUCT_SIZE(madopilot_build_info_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_operation_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_frame_stamp_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_frame_info_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_image_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_target_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_session_info_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_open_request_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_map_request_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_match_options_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_find_request_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_match_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_result_info_t);
#if MADOPILOT_ABI_MINOR >= 3
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_default_ocr_options_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_ocr_request_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_ocr_result_info_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_ocr_region_t);
#endif
#if MADOPILOT_ABI_MINOR >= 4
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_ocr_profile_options_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_ocr_zone_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_ocr_zone_scan_request_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_ocr_zone_scan_result_info_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_ocr_zone_result_t);
#endif
#if MADOPILOT_ABI_MINOR >= 5
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_ocr_provider_options_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_ocr_provider_descriptor_t);
#endif
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_package_info_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_template_info_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_error_detail_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_replay_frame_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_source_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_package_source_t);
FIRST_FIELD_IS_STRUCT_SIZE(madopilot_api_t);

/* The second field is 32 bits wide, so no implicit padding is introduced
 * between it and struct_size. Checking one representative of each of the two
 * shapes is enough; the offset comparison covers the rest. */
_Static_assert(offsetof(madopilot_operation_t, flags) == 4,
               "struct_size is followed immediately by a 32-bit field");
_Static_assert(offsetof(madopilot_source_t, kind) == 4,
               "a tagged structure puts its discriminant in the same slot");

/* The table's mandatory prefix has to be a real prefix of the table. */
_Static_assert(MADOPILOT_API_SIZE_INFORMATION <= sizeof(madopilot_api_t),
               "the mandatory table prefix fits inside the table");

int main(void)
{
    TYPE(madopilot_str_t);
    FIELD(madopilot_str_t, data);
    FIELD(madopilot_str_t, len);

    TYPE(madopilot_bytes_t);
    FIELD(madopilot_bytes_t, data);
    FIELD(madopilot_bytes_t, len);
#if MADOPILOT_ABI_MINOR >= 3
    TYPE(madopilot_ocr_point_t);
    FIELD(madopilot_ocr_point_t, x);
    FIELD(madopilot_ocr_point_t, y);
#endif


    TYPE(madopilot_pixel_rect_t);
    FIELD(madopilot_pixel_rect_t, space);
    FIELD(madopilot_pixel_rect_t, left);
    FIELD(madopilot_pixel_rect_t, top);
    FIELD(madopilot_pixel_rect_t, right);
    FIELD(madopilot_pixel_rect_t, bottom);

#if MADOPILOT_ABI_MINOR >= 2
    TYPE(madopilot_engine_capabilities_t);
    FIELD(madopilot_engine_capabilities_t, struct_size);
    FIELD(madopilot_engine_capabilities_t, flags);

    TYPE(madopilot_engine_options_t);
    FIELD(madopilot_engine_options_t, struct_size);
    FIELD(madopilot_engine_options_t, flags);
    FIELD(madopilot_engine_options_t, diagnostic_level);
    FIELD(madopilot_engine_options_t, diagnostic_capacity);
#if MADOPILOT_ABI_MINOR >= 3
    TYPE(madopilot_default_ocr_options_t);
    FIELD(madopilot_default_ocr_options_t, struct_size);
    FIELD(madopilot_default_ocr_options_t, flags);
    FIELD(madopilot_default_ocr_options_t, model_root);
    FIELD(madopilot_default_ocr_options_t, runtime_path);
#endif
#if MADOPILOT_ABI_MINOR >= 4
    TYPE(madopilot_ocr_profile_options_t);
    FIELD(madopilot_ocr_profile_options_t, struct_size);
    FIELD(madopilot_ocr_profile_options_t, flags);
    FIELD(madopilot_ocr_profile_options_t, kind);
    FIELD(madopilot_ocr_profile_options_t, reserved);
    FIELD(madopilot_ocr_profile_options_t, model_root);
    FIELD(madopilot_ocr_profile_options_t, runtime_path);
#if MADOPILOT_ABI_MINOR >= 5
    TYPE(madopilot_ocr_provider_options_t);
    FIELD(madopilot_ocr_provider_options_t, struct_size);
    FIELD(madopilot_ocr_provider_options_t, flags);
    FIELD(madopilot_ocr_provider_options_t, policy);
    FIELD(madopilot_ocr_provider_options_t, reserved);
    FIELD(madopilot_ocr_provider_options_t, provider_root);

    TYPE(madopilot_ocr_provider_descriptor_t);
    FIELD(madopilot_ocr_provider_descriptor_t, struct_size);
    FIELD(madopilot_ocr_provider_descriptor_t, flags);
    FIELD(madopilot_ocr_provider_descriptor_t, requested_policy);
    FIELD(madopilot_ocr_provider_descriptor_t, active_provider);
    FIELD(madopilot_ocr_provider_descriptor_t, initialization_fell_back);
    FIELD(madopilot_ocr_provider_descriptor_t, fallback_reason);
    FIELD(madopilot_ocr_provider_descriptor_t, runtime_profile);
#endif

    TYPE(madopilot_ocr_engine_descriptor_t);
    FIELD(madopilot_ocr_engine_descriptor_t, struct_size);
    FIELD(madopilot_ocr_engine_descriptor_t, flags);
    FIELD(madopilot_ocr_engine_descriptor_t, backend_id);
    FIELD(madopilot_ocr_engine_descriptor_t, backend_version);
    FIELD(madopilot_ocr_engine_descriptor_t, model_id);
    FIELD(madopilot_ocr_engine_descriptor_t, model_version);
    FIELD(madopilot_ocr_engine_descriptor_t, profile_id);
#endif

    TYPE(madopilot_diagnostic_batch_info_t);
    FIELD(madopilot_diagnostic_batch_info_t, struct_size);
    FIELD(madopilot_diagnostic_batch_info_t, flags);
    FIELD(madopilot_diagnostic_batch_info_t, record_count);
    FIELD(madopilot_diagnostic_batch_info_t, discarded_normal);
    FIELD(madopilot_diagnostic_batch_info_t, discarded_debug);

#if MADOPILOT_ABI_MINOR >= 3
    TYPE(madopilot_ocr_requested_region_t);
    FIELD(madopilot_ocr_requested_region_t, space);
    FIELD(madopilot_ocr_requested_region_t, clip_policy);
    FIELD(madopilot_ocr_requested_region_t, left);
    FIELD(madopilot_ocr_requested_region_t, top);
    FIELD(madopilot_ocr_requested_region_t, right);
    FIELD(madopilot_ocr_requested_region_t, bottom);
#endif

    TYPE(madopilot_diagnostic_record_t);
    FIELD(madopilot_diagnostic_record_t, struct_size);
    FIELD(madopilot_diagnostic_record_t, flags);
    FIELD(madopilot_diagnostic_record_t, sequence);
    FIELD(madopilot_diagnostic_record_t, timestamp_nanos);
    FIELD(madopilot_diagnostic_record_t, operation_id);
    FIELD(madopilot_diagnostic_record_t, activity_tag);
    FIELD(madopilot_diagnostic_record_t, level);
    FIELD(madopilot_diagnostic_record_t, kind);
    FIELD(madopilot_diagnostic_record_t, operation);
    FIELD(madopilot_diagnostic_record_t, status);
    FIELD(madopilot_diagnostic_record_t, target);
    FIELD(madopilot_diagnostic_record_t, frame);
    FIELD(madopilot_diagnostic_record_t, template_identity);
    FIELD(madopilot_diagnostic_record_t, source_space);
    FIELD(madopilot_diagnostic_record_t, destination_space);
    FIELD(madopilot_diagnostic_record_t, region);
    FIELD(madopilot_diagnostic_record_t, route);
    FIELD(madopilot_diagnostic_record_t, address_scope);
    FIELD(madopilot_diagnostic_record_t, evidence);
    FIELD(madopilot_diagnostic_record_t, input_fault);
    FIELD(madopilot_diagnostic_record_t, input_outcome);
    FIELD(madopilot_diagnostic_record_t, cleanup);
    FIELD(madopilot_diagnostic_record_t, permission_kind);
    FIELD(madopilot_diagnostic_record_t, permission_state);
    FIELD(madopilot_diagnostic_record_t, lifecycle);
    FIELD(madopilot_diagnostic_record_t, search_outcome);
    FIELD(madopilot_diagnostic_record_t, input_operations);
    FIELD(madopilot_diagnostic_record_t, partial_native_effect);
    FIELD(madopilot_diagnostic_record_t, used_fallback);
    FIELD(madopilot_diagnostic_record_t, reserved);
    FIELD(madopilot_diagnostic_record_t, requested);
    FIELD(madopilot_diagnostic_record_t, submitted);
    FIELD(madopilot_diagnostic_record_t, result_count);
    FIELD(madopilot_diagnostic_record_t, cleanup_released);
    FIELD(madopilot_diagnostic_record_t, cleanup_owed);
#if MADOPILOT_ABI_MINOR >= 3
    FIELD(madopilot_diagnostic_record_t, ocr_model_instance);
    FIELD(madopilot_diagnostic_record_t, ocr_profile);
    FIELD(madopilot_diagnostic_record_t, ocr_outcome);
    FIELD(madopilot_diagnostic_record_t, ocr_requested_region);
    FIELD(madopilot_diagnostic_record_t, ocr_elapsed_nanos);
    FIELD(madopilot_diagnostic_record_t, ocr_source_pixels);
#if MADOPILOT_ABI_MINOR >= 4
    FIELD(madopilot_diagnostic_record_t, ocr_source_envelope);
    FIELD(madopilot_diagnostic_record_t, ocr_grouped_reserved);
    FIELD(madopilot_diagnostic_record_t, ocr_zone_count);
    FIELD(madopilot_diagnostic_record_t, ocr_unique_candidate_count);
    FIELD(madopilot_diagnostic_record_t, ocr_membership_count);
    FIELD(madopilot_diagnostic_record_t, ocr_result_bytes);
    FIELD(madopilot_diagnostic_record_t, ocr_detector_runs);
    FIELD(madopilot_diagnostic_record_t, ocr_recognizer_runs);
    FIELD(madopilot_diagnostic_record_t, ocr_detector_bytes);
    FIELD(madopilot_diagnostic_record_t, ocr_recognizer_bytes);
#endif
#endif

    TYPE(madopilot_permission_t);
    FIELD(madopilot_permission_t, struct_size);
    FIELD(madopilot_permission_t, flags);
    FIELD(madopilot_permission_t, kind);
    FIELD(madopilot_permission_t, state);
    FIELD(madopilot_permission_t, diagnostic_category);
    FIELD(madopilot_permission_t, reserved);
    FIELD(madopilot_permission_t, platform_code);
    FIELD(madopilot_permission_t, platform_namespace);
    FIELD(madopilot_permission_t, context);

    TYPE(madopilot_input_capability_t);
    FIELD(madopilot_input_capability_t, struct_size);
    FIELD(madopilot_input_capability_t, flags);
    FIELD(madopilot_input_capability_t, target);
    FIELD(madopilot_input_capability_t, operation);
    FIELD(madopilot_input_capability_t, delivery);
    FIELD(madopilot_input_capability_t, support);
    FIELD(madopilot_input_capability_t, address_scope);
    FIELD(madopilot_input_capability_t, permission);
    FIELD(madopilot_input_capability_t, evidence);
    FIELD(madopilot_input_capability_t, focus_required);
    FIELD(madopilot_input_capability_t, pointer_spaces);
    FIELD(madopilot_input_capability_t, reserved);

    TYPE(madopilot_input_open_request_t);
    FIELD(madopilot_input_open_request_t, struct_size);
    FIELD(madopilot_input_open_request_t, flags);
    FIELD(madopilot_input_open_request_t, requirement);
    FIELD(madopilot_input_open_request_t, reserved);
    FIELD(madopilot_input_open_request_t, required_pairs);
    FIELD(madopilot_input_open_request_t, preferred_pairs);

    TYPE(madopilot_input_descriptor_t);
    FIELD(madopilot_input_descriptor_t, struct_size);
    FIELD(madopilot_input_descriptor_t, flags);
    FIELD(madopilot_input_descriptor_t, target);
    FIELD(madopilot_input_descriptor_t, known_pairs);
    FIELD(madopilot_input_descriptor_t, supported_pairs);
    FIELD(madopilot_input_descriptor_t, unknown_pairs);
    FIELD(madopilot_input_descriptor_t, pointer_spaces);
    FIELD(madopilot_input_descriptor_t, max_events);

    TYPE(madopilot_input_event_t);
    FIELD(madopilot_input_event_t, struct_size);
    FIELD(madopilot_input_event_t, kind);
    FIELD(madopilot_input_event_t, space);
    FIELD(madopilot_input_event_t, button);
    FIELD(madopilot_input_event_t, key);
    FIELD(madopilot_input_event_t, key_value);
    FIELD(madopilot_input_event_t, x);
    FIELD(madopilot_input_event_t, y);
    FIELD(madopilot_input_event_t, horizontal);
    FIELD(madopilot_input_event_t, vertical);
    FIELD(madopilot_input_event_t, text);
    FIELD(madopilot_input_event_t, delay_nanos);

    TYPE(madopilot_input_request_t);
    FIELD(madopilot_input_request_t, struct_size);
    FIELD(madopilot_input_request_t, flags);
    FIELD(madopilot_input_request_t, events);
    FIELD(madopilot_input_request_t, event_count);
    FIELD(madopilot_input_request_t, event_stride);
    FIELD(madopilot_input_request_t, deliveries);
    FIELD(madopilot_input_request_t, delivery_count);
    FIELD(madopilot_input_request_t, focus_policy);
    FIELD(madopilot_input_request_t, geometry_policy);
    FIELD(madopilot_input_request_t, source_frame);
    FIELD(madopilot_input_request_t, cleanup_max_events);
    FIELD(madopilot_input_request_t, reserved);
    FIELD(madopilot_input_request_t, cleanup_timeout_nanos);

    TYPE(madopilot_input_receipt_info_t);
    FIELD(madopilot_input_receipt_info_t, struct_size);
    FIELD(madopilot_input_receipt_info_t, flags);
    FIELD(madopilot_input_receipt_info_t, target);
    FIELD(madopilot_input_receipt_info_t, outcome);
    FIELD(madopilot_input_receipt_info_t, selected_route);
    FIELD(madopilot_input_receipt_info_t, address_scope);
    FIELD(madopilot_input_receipt_info_t, attempt_count);
    FIELD(madopilot_input_receipt_info_t, submitted);
    FIELD(madopilot_input_receipt_info_t, last_submitted);
    FIELD(madopilot_input_receipt_info_t, evidence);
    FIELD(madopilot_input_receipt_info_t, fault);
    FIELD(madopilot_input_receipt_info_t, cleanup);
    FIELD(madopilot_input_receipt_info_t, cleanup_released);
    FIELD(madopilot_input_receipt_info_t, cleanup_owed);

    TYPE(madopilot_input_attempt_t);
    FIELD(madopilot_input_attempt_t, struct_size);
    FIELD(madopilot_input_attempt_t, flags);
    FIELD(madopilot_input_attempt_t, route);
    FIELD(madopilot_input_attempt_t, address_scope);
    FIELD(madopilot_input_attempt_t, outcome);
    FIELD(madopilot_input_attempt_t, submitted);
    FIELD(madopilot_input_attempt_t, last_submitted);
    FIELD(madopilot_input_attempt_t, evidence);
    FIELD(madopilot_input_attempt_t, fault);
    FIELD(madopilot_input_attempt_t, reserved);
#endif

    TYPE(madopilot_build_info_t);
    FIELD(madopilot_build_info_t, struct_size);
    FIELD(madopilot_build_info_t, flags);
    FIELD(madopilot_build_info_t, abi_major);
    FIELD(madopilot_build_info_t, abi_minor);
    FIELD(madopilot_build_info_t, table_size);
    FIELD(madopilot_build_info_t, reserved);
    FIELD(madopilot_build_info_t, library_version);
    FIELD(madopilot_build_info_t, required_backend);
#if MADOPILOT_ABI_MINOR >= 3
    FIELD(madopilot_build_info_t, default_ocr_backend);
    FIELD(madopilot_build_info_t, default_ocr_backend_version);
    FIELD(madopilot_build_info_t, default_ocr_runtime_profile);
    FIELD(madopilot_build_info_t, default_ocr_model);
    FIELD(madopilot_build_info_t, default_ocr_model_version);
    FIELD(madopilot_build_info_t, default_ocr_profile);
#endif
#if MADOPILOT_ABI_MINOR >= 4
    FIELD(madopilot_build_info_t, bounded_ocr_model);
    FIELD(madopilot_build_info_t, bounded_ocr_model_version);
    FIELD(madopilot_build_info_t, bounded_ocr_profile);
#endif

    TYPE(madopilot_operation_t);
    FIELD(madopilot_operation_t, struct_size);
    FIELD(madopilot_operation_t, flags);
    FIELD(madopilot_operation_t, deadline_nanos);
    FIELD(madopilot_operation_t, cancellation);
#if MADOPILOT_ABI_MINOR >= 2
    FIELD(madopilot_operation_t, activity_tag);
#endif

    TYPE(madopilot_frame_stamp_t);
    FIELD(madopilot_frame_stamp_t, struct_size);
    FIELD(madopilot_frame_stamp_t, flags);
    FIELD(madopilot_frame_stamp_t, stream);
    FIELD(madopilot_frame_stamp_t, epoch);
    FIELD(madopilot_frame_stamp_t, sequence);
    FIELD(madopilot_frame_stamp_t, geometry);

    TYPE(madopilot_frame_info_t);
    FIELD(madopilot_frame_info_t, struct_size);
    FIELD(madopilot_frame_info_t, flags);
    FIELD(madopilot_frame_info_t, width);
    FIELD(madopilot_frame_info_t, height);
    FIELD(madopilot_frame_info_t, format);
    FIELD(madopilot_frame_info_t, space);
    FIELD(madopilot_frame_info_t, stride);
    FIELD(madopilot_frame_info_t, bounds);

    TYPE(madopilot_image_t);
    FIELD(madopilot_image_t, struct_size);
    FIELD(madopilot_image_t, flags);
    FIELD(madopilot_image_t, width);
    FIELD(madopilot_image_t, height);
    FIELD(madopilot_image_t, format);
    FIELD(madopilot_image_t, space);
    FIELD(madopilot_image_t, stride);
    FIELD(madopilot_image_t, bytes);
    FIELD(madopilot_image_t, region);

    TYPE(madopilot_target_t);
    FIELD(madopilot_target_t, struct_size);
    FIELD(madopilot_target_t, flags);
    FIELD(madopilot_target_t, width);
    FIELD(madopilot_target_t, height);
    FIELD(madopilot_target_t, format);
    FIELD(madopilot_target_t, coordinate_spaces);
    FIELD(madopilot_target_t, name);
    FIELD(madopilot_target_t, provider);
#if MADOPILOT_ABI_MINOR >= 2
    FIELD(madopilot_target_t, target);
    FIELD(madopilot_target_t, kind);
    FIELD(madopilot_target_t, capture);
    FIELD(madopilot_target_t, capture_permission);
    FIELD(madopilot_target_t, reserved);
#endif

    TYPE(madopilot_session_info_t);
    FIELD(madopilot_session_info_t, struct_size);
    FIELD(madopilot_session_info_t, flags);
    FIELD(madopilot_session_info_t, stream);
    FIELD(madopilot_session_info_t, width);
    FIELD(madopilot_session_info_t, height);
    FIELD(madopilot_session_info_t, format);
    FIELD(madopilot_session_info_t, coordinate_spaces);
#if MADOPILOT_ABI_MINOR >= 2
    FIELD(madopilot_session_info_t, target);
    FIELD(madopilot_session_info_t, accepts_input);
    FIELD(madopilot_session_info_t, reserved);
#endif

    TYPE(madopilot_open_request_t);
    FIELD(madopilot_open_request_t, struct_size);
    FIELD(madopilot_open_request_t, flags);
    FIELD(madopilot_open_request_t, required_format);
    FIELD(madopilot_open_request_t, preferred_format);

    TYPE(madopilot_map_request_t);
    FIELD(madopilot_map_request_t, struct_size);
    FIELD(madopilot_map_request_t, flags);
    FIELD(madopilot_map_request_t, format);
    FIELD(madopilot_map_request_t, clip_policy);
    FIELD(madopilot_map_request_t, region);

    TYPE(madopilot_match_options_t);
    FIELD(madopilot_match_options_t, struct_size);
    FIELD(madopilot_match_options_t, flags);
    FIELD(madopilot_match_options_t, min_score);
    FIELD(madopilot_match_options_t, max_results);
    FIELD(madopilot_match_options_t, suppression);

    TYPE(madopilot_find_request_t);
    FIELD(madopilot_find_request_t, struct_size);
    FIELD(madopilot_find_request_t, flags);
    FIELD(madopilot_find_request_t, frame);
    FIELD(madopilot_find_request_t, tmpl);
    FIELD(madopilot_find_request_t, options);
    FIELD(madopilot_find_request_t, region);
    FIELD(madopilot_find_request_t, clip_policy);

    TYPE(madopilot_match_t);
    FIELD(madopilot_match_t, struct_size);
    FIELD(madopilot_match_t, flags);
    FIELD(madopilot_match_t, score);
    FIELD(madopilot_match_t, template_id);
    FIELD(madopilot_match_t, bounds);

    TYPE(madopilot_result_info_t);
    FIELD(madopilot_result_info_t, struct_size);
    FIELD(madopilot_result_info_t, flags);
    FIELD(madopilot_result_info_t, match_count);
    FIELD(madopilot_result_info_t, backend_id);
    FIELD(madopilot_result_info_t, backend_version);
    FIELD(madopilot_result_info_t, searched);
#if MADOPILOT_ABI_MINOR >= 3
    TYPE(madopilot_ocr_request_t);
    FIELD(madopilot_ocr_request_t, struct_size);
    FIELD(madopilot_ocr_request_t, flags);
    FIELD(madopilot_ocr_request_t, frame);
    FIELD(madopilot_ocr_request_t, package);
    FIELD(madopilot_ocr_request_t, model_id);
    FIELD(madopilot_ocr_request_t, backend_id);
    FIELD(madopilot_ocr_request_t, backend_version);
    FIELD(madopilot_ocr_request_t, output_space);
    FIELD(madopilot_ocr_request_t, clip_policy);
    FIELD(madopilot_ocr_request_t, region);
#if MADOPILOT_ABI_MINOR >= 4
    TYPE(madopilot_ocr_zone_t);
    FIELD(madopilot_ocr_zone_t, struct_size);
    FIELD(madopilot_ocr_zone_t, flags);
    FIELD(madopilot_ocr_zone_t, region);
    FIELD(madopilot_ocr_zone_t, clip_policy);

    TYPE(madopilot_ocr_zone_scan_request_t);
    FIELD(madopilot_ocr_zone_scan_request_t, struct_size);
    FIELD(madopilot_ocr_zone_scan_request_t, flags);
    FIELD(madopilot_ocr_zone_scan_request_t, frame);
    FIELD(madopilot_ocr_zone_scan_request_t, package);
    FIELD(madopilot_ocr_zone_scan_request_t, model_id);
    FIELD(madopilot_ocr_zone_scan_request_t, backend_id);
    FIELD(madopilot_ocr_zone_scan_request_t, backend_version);
    FIELD(madopilot_ocr_zone_scan_request_t, output_space);
    FIELD(madopilot_ocr_zone_scan_request_t, reserved);
    FIELD(madopilot_ocr_zone_scan_request_t, zones);
    FIELD(madopilot_ocr_zone_scan_request_t, zone_count);
    FIELD(madopilot_ocr_zone_scan_request_t, zone_stride);
#endif

    TYPE(madopilot_ocr_result_info_t);
    FIELD(madopilot_ocr_result_info_t, struct_size);
    FIELD(madopilot_ocr_result_info_t, flags);
    FIELD(madopilot_ocr_result_info_t, source);
    FIELD(madopilot_ocr_result_info_t, effective_region);
    FIELD(madopilot_ocr_result_info_t, output_space);
    FIELD(madopilot_ocr_result_info_t, reserved);
    FIELD(madopilot_ocr_result_info_t, region_count);
    FIELD(madopilot_ocr_result_info_t, backend_id);
    FIELD(madopilot_ocr_result_info_t, backend_version);
    FIELD(madopilot_ocr_result_info_t, model_id);
    FIELD(madopilot_ocr_result_info_t, model_version);
    FIELD(madopilot_ocr_result_info_t, profile_id);
#if MADOPILOT_ABI_MINOR >= 4
    TYPE(madopilot_ocr_zone_scan_result_info_t);
    FIELD(madopilot_ocr_zone_scan_result_info_t, struct_size);
    FIELD(madopilot_ocr_zone_scan_result_info_t, flags);
    FIELD(madopilot_ocr_zone_scan_result_info_t, source);
    FIELD(madopilot_ocr_zone_scan_result_info_t, source_envelope);
    FIELD(madopilot_ocr_zone_scan_result_info_t, output_space);
    FIELD(madopilot_ocr_zone_scan_result_info_t, zone_count);
    FIELD(madopilot_ocr_zone_scan_result_info_t, unique_candidate_count);
    FIELD(madopilot_ocr_zone_scan_result_info_t, membership_count);
    FIELD(madopilot_ocr_zone_scan_result_info_t, backend_id);
    FIELD(madopilot_ocr_zone_scan_result_info_t, backend_version);
    FIELD(madopilot_ocr_zone_scan_result_info_t, model_id);
    FIELD(madopilot_ocr_zone_scan_result_info_t, model_version);
    FIELD(madopilot_ocr_zone_scan_result_info_t, profile_id);

    TYPE(madopilot_ocr_zone_result_t);
    FIELD(madopilot_ocr_zone_result_t, struct_size);
    FIELD(madopilot_ocr_zone_result_t, flags);
    FIELD(madopilot_ocr_zone_result_t, effective_zone);
    FIELD(madopilot_ocr_zone_result_t, reserved);
    FIELD(madopilot_ocr_zone_result_t, region_count);
#endif

    TYPE(madopilot_ocr_region_t);
    FIELD(madopilot_ocr_region_t, struct_size);
    FIELD(madopilot_ocr_region_t, flags);
    FIELD(madopilot_ocr_region_t, confidence);
    FIELD(madopilot_ocr_region_t, points);
#endif


    TYPE(madopilot_package_info_t);
    FIELD(madopilot_package_info_t, struct_size);
    FIELD(madopilot_package_info_t, flags);
    FIELD(madopilot_package_info_t, template_count);
    FIELD(madopilot_package_info_t, package_id);
    FIELD(madopilot_package_info_t, package_version);
    FIELD(madopilot_package_info_t, license);

    TYPE(madopilot_template_info_t);
    FIELD(madopilot_template_info_t, struct_size);
    FIELD(madopilot_template_info_t, flags);
    FIELD(madopilot_template_info_t, width);
    FIELD(madopilot_template_info_t, height);
    FIELD(madopilot_template_info_t, min_score);
    FIELD(madopilot_template_info_t, id);
    FIELD(madopilot_template_info_t, backend);
    FIELD(madopilot_template_info_t, max_results);
    FIELD(madopilot_template_info_t, space);

    TYPE(madopilot_error_detail_t);
    FIELD(madopilot_error_detail_t, struct_size);
    FIELD(madopilot_error_detail_t, flags);
    FIELD(madopilot_error_detail_t, status);
    FIELD(madopilot_error_detail_t, category);
    FIELD(madopilot_error_detail_t, asset_fault);
    FIELD(madopilot_error_detail_t, asset_stage);
    FIELD(madopilot_error_detail_t, message);
    FIELD(madopilot_error_detail_t, backend);

    TYPE(madopilot_replay_frame_t);
    FIELD(madopilot_replay_frame_t, struct_size);
    FIELD(madopilot_replay_frame_t, flags);
    FIELD(madopilot_replay_frame_t, width);
    FIELD(madopilot_replay_frame_t, height);
    FIELD(madopilot_replay_frame_t, format);
    FIELD(madopilot_replay_frame_t, continuity);
    FIELD(madopilot_replay_frame_t, pixels);
    FIELD(madopilot_replay_frame_t, captured_at_nanos);
    FIELD(madopilot_replay_frame_t, stride);

    TYPE(madopilot_source_t);
    FIELD(madopilot_source_t, struct_size);
    FIELD(madopilot_source_t, kind);
    FIELD(madopilot_source_t, directory);
    FIELD(madopilot_source_t, frames);
    FIELD(madopilot_source_t, frame_count);
    FIELD(madopilot_source_t, frame_stride);
    FIELD(madopilot_source_t, target_name);

    TYPE(madopilot_package_source_t);
    FIELD(madopilot_package_source_t, struct_size);
    FIELD(madopilot_package_source_t, kind);
    FIELD(madopilot_package_source_t, path);
    FIELD(madopilot_package_source_t, archive);

    TYPE(madopilot_api_t);
    FIELD(madopilot_api_t, struct_size);
    FIELD(madopilot_api_t, abi_major);
    FIELD(madopilot_api_t, abi_minor);
    FIELD(madopilot_api_t, reserved);
    FIELD(madopilot_api_t, describe_build);
    FIELD(madopilot_api_t, clock_now);
    FIELD(madopilot_api_t, status_text);
    FIELD(madopilot_api_t, cancellation_create);
    FIELD(madopilot_api_t, cancellation_retain);
    FIELD(madopilot_api_t, cancellation_release);
    FIELD(madopilot_api_t, cancellation_cancel);
    FIELD(madopilot_api_t, cancellation_is_cancelled);
    FIELD(madopilot_api_t, error_retain);
    FIELD(madopilot_api_t, error_release);
    FIELD(madopilot_api_t, error_describe);
    FIELD(madopilot_api_t, engine_create);
    FIELD(madopilot_api_t, engine_retain);
    FIELD(madopilot_api_t, engine_release);
    FIELD(madopilot_api_t, package_load);
    FIELD(madopilot_api_t, package_retain);
    FIELD(madopilot_api_t, package_release);
    FIELD(madopilot_api_t, package_describe);
    FIELD(madopilot_api_t, package_template_id);
    FIELD(madopilot_api_t, template_prepare_from_package);
    FIELD(madopilot_api_t, template_retain);
    FIELD(madopilot_api_t, template_release);
    FIELD(madopilot_api_t, template_describe);
    FIELD(madopilot_api_t, engine_discover);
    FIELD(madopilot_api_t, target_list_retain);
    FIELD(madopilot_api_t, target_list_release);
    FIELD(madopilot_api_t, target_list_count);
    FIELD(madopilot_api_t, target_list_get);
    FIELD(madopilot_api_t, session_open);
    FIELD(madopilot_api_t, session_retain);
    FIELD(madopilot_api_t, session_release);
    FIELD(madopilot_api_t, session_describe);
    FIELD(madopilot_api_t, session_close);
    FIELD(madopilot_api_t, session_is_closed);
    FIELD(madopilot_api_t, session_acquire_frame);
    FIELD(madopilot_api_t, frame_retain);
    FIELD(madopilot_api_t, frame_release);
    FIELD(madopilot_api_t, frame_stamp);
    FIELD(madopilot_api_t, frame_describe);
    FIELD(madopilot_api_t, frame_map);
    FIELD(madopilot_api_t, mapping_retain);
    FIELD(madopilot_api_t, mapping_release);
    FIELD(madopilot_api_t, mapping_describe);
    FIELD(madopilot_api_t, mapping_stamp);
    FIELD(madopilot_api_t, session_find);
    FIELD(madopilot_api_t, result_retain);
    FIELD(madopilot_api_t, result_release);
    FIELD(madopilot_api_t, result_describe);
    FIELD(madopilot_api_t, result_stamp);
    FIELD(madopilot_api_t, result_options);
    FIELD(madopilot_api_t, result_match);
#if MADOPILOT_ABI_MINOR >= 2
    FIELD(madopilot_api_t, engine_create_with_options);
    FIELD(madopilot_api_t, engine_capabilities);
    FIELD(madopilot_api_t, engine_permission);
    FIELD(madopilot_api_t, target_list_input_capability);
    FIELD(madopilot_api_t, engine_input_descriptor);
    FIELD(madopilot_api_t, session_open_with_input);
    FIELD(madopilot_api_t, session_input_descriptor);
    FIELD(madopilot_api_t, session_send_input);
    FIELD(madopilot_api_t, input_receipt_retain);
    FIELD(madopilot_api_t, input_receipt_release);
    FIELD(madopilot_api_t, input_receipt_info);
    FIELD(madopilot_api_t, input_receipt_attempt_count);
    FIELD(madopilot_api_t, input_receipt_attempt_at);
    FIELD(madopilot_api_t, engine_take_diagnostic_reader);
    FIELD(madopilot_api_t, diagnostic_reader_retain);
    FIELD(madopilot_api_t, diagnostic_reader_release);
    FIELD(madopilot_api_t, diagnostic_reader_drain);
    FIELD(madopilot_api_t, diagnostic_batch_retain);
    FIELD(madopilot_api_t, diagnostic_batch_release);
    FIELD(madopilot_api_t, diagnostic_batch_info);
    FIELD(madopilot_api_t, diagnostic_batch_record_at);
#endif
#if MADOPILOT_ABI_MINOR >= 3
    FIELD(madopilot_api_t, session_recognize);
    FIELD(madopilot_api_t, ocr_result_retain);
    FIELD(madopilot_api_t, ocr_result_release);
    FIELD(madopilot_api_t, ocr_result_info);
    FIELD(madopilot_api_t, ocr_result_region_at);
    FIELD(madopilot_api_t, ocr_result_text_at);
    FIELD(madopilot_api_t, engine_create_with_default_ocr);
#endif
#if MADOPILOT_ABI_MINOR >= 4
    FIELD(madopilot_api_t, engine_create_with_ocr_profile);
    FIELD(madopilot_api_t, session_scan_ocr_zones);
    FIELD(madopilot_api_t, ocr_zone_scan_result_retain);
    FIELD(madopilot_api_t, ocr_zone_scan_result_release);
    FIELD(madopilot_api_t, ocr_zone_scan_result_info);
    FIELD(madopilot_api_t, ocr_zone_scan_result_zone_at);
    FIELD(madopilot_api_t, ocr_zone_scan_result_region_at);
    FIELD(madopilot_api_t, ocr_zone_scan_result_text_at);
    FIELD(madopilot_api_t, engine_ocr_descriptor);
#endif
#if MADOPILOT_ABI_MINOR >= 5
    FIELD(madopilot_api_t, engine_create_with_ocr_provider);
    FIELD(madopilot_api_t, engine_ocr_provider_descriptor);
#endif

    HANDLE(madopilot_cancellation_t);
    HANDLE(madopilot_error_t);
    HANDLE(madopilot_engine_t);
    HANDLE(madopilot_target_list_t);
    HANDLE(madopilot_package_t);
    HANDLE(madopilot_template_t);
    HANDLE(madopilot_session_t);
    HANDLE(madopilot_frame_t);
    HANDLE(madopilot_mapping_t);
    HANDLE(madopilot_result_t);
#if MADOPILOT_ABI_MINOR >= 3
    HANDLE(madopilot_ocr_result_t);
#endif
#if MADOPILOT_ABI_MINOR >= 4
    HANDLE(madopilot_ocr_zone_scan_result_t);
#endif
#if MADOPILOT_ABI_MINOR >= 2
    HANDLE(madopilot_input_receipt_t);
    HANDLE(madopilot_diagnostic_reader_t);
    HANDLE(madopilot_diagnostic_batch_t);
#endif

    return 0;
}
