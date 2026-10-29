/* Deterministic singular and grouped OCR through the ABI 1.4 C surface. */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "ocr-private-fixture.h"
#include "windows-utf8-entry.h"

static int ok(madopilot_status_t status, const char* call)
{
    if (status == MADOPILOT_STATUS_OK) {
        return 1;
    }
    fprintf(stderr, "%s failed with status %d\n", call, (int)status);
    return 0;
}

static madopilot_str_t borrow(const char* text)
{
    madopilot_str_t view;
    view.data = text;
    view.len = strlen(text);
    return view;
}

int main(int argc, char** argv)
{
    const char* package_path = NULL;
    for (int index = 1; index + 1 < argc; index += 2) {
        if (strcmp(argv[index], "--package") == 0) {
            package_path = argv[index + 1];
        }
    }
    if (package_path == NULL) {
        fprintf(stderr, "usage: %s --package <dir>\n", argv[0]);
        return 2;
    }

    const madopilot_api_t* api = NULL;
    if (!ok(madopilot_get_api(MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR,
                              sizeof(madopilot_api_t), &api),
            "madopilot_get_api") ||
        api == NULL ||
        api->struct_size < MADOPILOT_API_SIZE_OCR_ZONE_SCAN_RESULT_TEXT_AT) {
        return 1;
    }

    uint8_t pixels[32u * 24u * 4u];
    memset(pixels, 0x40, sizeof(pixels));
    madopilot_replay_frame_t supplied = {0};
    supplied.struct_size = (uint32_t)sizeof(supplied);
    supplied.width = 32;
    supplied.height = 24;
    supplied.format = MADOPILOT_PIXEL_FORMAT_RGBA8;
    supplied.continuity = MADOPILOT_CONTINUITY_CONTINUOUS;
    supplied.pixels.data = pixels;
    supplied.pixels.len = sizeof(pixels);
    supplied.stride = 32u * 4u;

    madopilot_source_t source = {0};
    source.struct_size = (uint32_t)sizeof(source);
    source.kind = MADOPILOT_SOURCE_REPLAY_MEMORY;
    source.frames = &supplied;
    source.frame_count = 1;
    source.frame_stride = sizeof(supplied);
    source.target_name = borrow("ocr-panel");

    madopilot_operation_t operation = {0};
    operation.struct_size = (uint32_t)sizeof(operation);
    madopilot_engine_t* engine = NULL;
    madopilot_target_list_t* targets = NULL;
    madopilot_session_t* session = NULL;
    madopilot_frame_t* frame = NULL;
    madopilot_package_t* package = NULL;
    madopilot_ocr_result_t* result = NULL;
    madopilot_ocr_zone_scan_result_t* grouped_result = NULL;
    madopilot_ocr_zone_scan_result_t* sixteen_result = NULL;
    madopilot_ocr_zone_scan_result_t* refused_result = NULL;
    madopilot_diagnostic_reader_t* reader = NULL;
    madopilot_diagnostic_batch_t* batch = NULL;
    madopilot_engine_options_t options = {0};
    options.struct_size = (uint32_t)sizeof(options);
    options.diagnostic_level = MADOPILOT_DIAGNOSTIC_LEVEL_DEBUG;
    options.diagnostic_capacity = 32;
    madopilot_error_t* error = NULL;

    if (!ok(madopilot_fixture_engine_create(
                &source, &options, &operation, &engine, &error),
            "madopilot_fixture_engine_create") ||
        !ok(api->engine_discover(engine, &operation, &targets, &error),
            "engine_discover")) {
        goto fail;
    }
    if (!ok(api->engine_take_diagnostic_reader(engine, &reader),
            "engine_take_diagnostic_reader") ||
        reader == NULL) {
        goto fail;
    }

    madopilot_open_request_t open = {0};
    open.struct_size = (uint32_t)sizeof(open);
    if (!ok(api->session_open(engine, targets, 0, &open, &operation, &session, &error),
            "session_open")) {
        goto fail;
    }
    api->target_list_release(targets);
    targets = NULL;
    if (!ok(api->session_acquire_frame(session, &operation, &frame, &error),
            "session_acquire_frame")) {
        goto fail;
    }

    madopilot_package_source_t package_source = {0};
    package_source.struct_size = (uint32_t)sizeof(package_source);
    package_source.kind = MADOPILOT_PACKAGE_SOURCE_DIRECTORY;
    package_source.path = borrow(package_path);
    if (!ok(api->package_load(engine, &package_source, &operation, &package, &error),
            "package_load")) {
        goto fail;
    }

    madopilot_ocr_request_t request = {0};
    request.struct_size = (uint32_t)sizeof(request);
    request.frame = frame;
    request.package = package;
    request.model_id = borrow(MADOPILOT_FIXTURE_OCR_MODEL_ID);
    request.backend_id = borrow(MADOPILOT_FIXTURE_OCR_BACKEND_ID);
    request.backend_version = borrow(MADOPILOT_FIXTURE_OCR_BACKEND_VERSION);
    request.output_space = MADOPILOT_SPACE_CAPTURE_PIXELS;
    request.clip_policy = MADOPILOT_CLIP_POLICY_REJECT;
    if (!ok(api->session_recognize(session, &request, &operation, &result, &error),
            "session_recognize")) {
        goto fail;
    }

    madopilot_ocr_zone_t zones[3] = {0};
    const int32_t edges[3][4] = {
        {0, 0, 16, 12}, {24, 0, 32, 12}, {8, 0, 24, 12}};
    for (size_t index = 0; index < 3; ++index) {
        zones[index].struct_size = (uint32_t)sizeof(zones[index]);
        zones[index].region.space = MADOPILOT_SPACE_CAPTURE_PIXELS;
        zones[index].region.left = edges[index][0];
        zones[index].region.top = edges[index][1];
        zones[index].region.right = edges[index][2];
        zones[index].region.bottom = edges[index][3];
        zones[index].clip_policy = MADOPILOT_CLIP_POLICY_REJECT;
    }
    madopilot_ocr_zone_scan_request_t grouped_request = {0};
    grouped_request.struct_size = (uint32_t)sizeof(grouped_request);
    grouped_request.frame = frame;
    grouped_request.package = package;
    grouped_request.model_id = request.model_id;
    grouped_request.backend_id = request.backend_id;
    grouped_request.backend_version = request.backend_version;
    grouped_request.output_space = MADOPILOT_SPACE_CAPTURE_PIXELS;
    grouped_request.zones = zones;
    grouped_request.zone_count = 3;
    grouped_request.zone_stride = sizeof(zones[0]);
    if (!ok(api->session_scan_ocr_zones(
                session, &grouped_request, &operation, &grouped_result, &error),
            "session_scan_ocr_zones")) {
        goto fail;
    }

    madopilot_ocr_zone_t expanded_zones[17] = {0};
    for (size_t index = 0; index < 17; ++index) {
        expanded_zones[index].struct_size = (uint32_t)sizeof(expanded_zones[index]);
        expanded_zones[index].region.space = MADOPILOT_SPACE_CAPTURE_PIXELS;
        expanded_zones[index].region.left = (int32_t)(index % 4);
        expanded_zones[index].region.top = (int32_t)(index / 4);
        expanded_zones[index].region.right = 32;
        expanded_zones[index].region.bottom = 24;
        expanded_zones[index].clip_policy = MADOPILOT_CLIP_POLICY_REJECT;
    }
    grouped_request.zones = expanded_zones;
    grouped_request.zone_count = 16;
    if (!ok(api->session_scan_ocr_zones(
                session, &grouped_request, &operation, &sixteen_result, &error),
            "session_scan_ocr_zones sixteen")) {
        goto fail;
    }
    grouped_request.zone_count = 17;
    const madopilot_status_t refused_status = api->session_scan_ocr_zones(
        session, &grouped_request, &operation, &refused_result, &error);
    if (refused_status != MADOPILOT_STATUS_INVALID_ARGUMENT ||
        refused_result != NULL) {
        fprintf(stderr, "seventeen OCR zones were not refused atomically\n");
        goto fail;
    }
    if (error != NULL) {
        api->error_release(error);
        error = NULL;
    }

    if (!ok(api->session_close(session, &operation, &error), "session_close")) {
        goto fail;
    }
    api->frame_release(frame);
    frame = NULL;
    api->package_release(package);
    package = NULL;
    api->session_release(session);
    session = NULL;
    api->engine_release(engine);
    engine = NULL;

    madopilot_ocr_result_info_t info = {0};
    info.struct_size = (uint32_t)sizeof(info);
    madopilot_ocr_region_t region = {0};
    region.struct_size = (uint32_t)sizeof(region);
    madopilot_str_t text = {0};
    if (!ok(api->ocr_result_info(result, &info), "ocr_result_info") ||
        info.region_count != 1 ||
        !ok(api->ocr_result_region_at(result, 0, &region), "ocr_result_region_at") ||
        !ok(api->ocr_result_text_at(result, 0, &text), "ocr_result_text_at")) {
        goto fail;
    }
    const char expected_text[] = "\xe9\xad\x94\xe5\xb0\x8e\xe5\xa3\xab A-7";
    const double expected_points[4][2] = {
        {1.0, 2.0}, {20.0, 2.0}, {20.0, 9.0}, {1.0, 9.0}};
    if (text.len != sizeof(expected_text) - 1 ||
        memcmp(text.data, expected_text, sizeof(expected_text) - 1) != 0 ||
        region.confidence != 0.91) {
        goto fail;
    }
    for (size_t point = 0; point < 4; ++point) {
        if (region.points[point].x != expected_points[point][0] ||
            region.points[point].y != expected_points[point][1]) {
            goto fail;
        }
    }

    madopilot_ocr_zone_scan_result_info_t grouped_info = {0};
    grouped_info.struct_size = (uint32_t)sizeof(grouped_info);
    madopilot_ocr_zone_result_t grouped_first = {0};
    madopilot_ocr_zone_result_t grouped_empty = {0};
    madopilot_ocr_zone_result_t grouped_overlap = {0};
    grouped_first.struct_size = (uint32_t)sizeof(grouped_first);
    grouped_empty.struct_size = (uint32_t)sizeof(grouped_empty);
    grouped_overlap.struct_size = (uint32_t)sizeof(grouped_overlap);
    madopilot_ocr_region_t grouped_region = {0};
    grouped_region.struct_size = (uint32_t)sizeof(grouped_region);
    madopilot_str_t grouped_text = {0};
    madopilot_str_t overlap_text = {0};
    if (!ok(api->ocr_zone_scan_result_info(grouped_result, &grouped_info),
            "ocr_zone_scan_result_info") ||
        grouped_info.zone_count != 3 ||
        grouped_info.unique_candidate_count != 1 ||
        grouped_info.membership_count != 2 ||
        grouped_info.source.sequence != info.source.sequence ||
        !ok(api->ocr_zone_scan_result_zone_at(
                grouped_result, 0, &grouped_first),
            "ocr_zone_scan_result_zone_at first") ||
        !ok(api->ocr_zone_scan_result_zone_at(
                grouped_result, 1, &grouped_empty),
            "ocr_zone_scan_result_zone_at empty") ||
        !ok(api->ocr_zone_scan_result_zone_at(
                grouped_result, 2, &grouped_overlap),
            "ocr_zone_scan_result_zone_at overlap") ||
        grouped_first.region_count != 1 || grouped_empty.region_count != 0 ||
        grouped_overlap.region_count != 1 ||
        !ok(api->ocr_zone_scan_result_region_at(
                grouped_result, 0, 0, &grouped_region),
            "ocr_zone_scan_result_region_at") ||
        grouped_region.confidence != region.confidence ||
        grouped_region.points[0].x != region.points[0].x ||
        grouped_region.points[0].y != region.points[0].y ||
        !ok(api->ocr_zone_scan_result_text_at(
                grouped_result, 0, 0, &grouped_text),
            "ocr_zone_scan_result_text_at first") ||
        !ok(api->ocr_zone_scan_result_text_at(
                grouped_result, 2, 0, &overlap_text),
            "ocr_zone_scan_result_text_at overlap") ||
        grouped_text.len != text.len ||
        memcmp(grouped_text.data, text.data, text.len) != 0 ||
        grouped_text.data != overlap_text.data ||
        grouped_text.len != overlap_text.len) {
        goto fail;
    }

    madopilot_ocr_zone_scan_result_info_t sixteen_info = {0};
    sixteen_info.struct_size = (uint32_t)sizeof(sixteen_info);
    if (!ok(api->ocr_zone_scan_result_info(sixteen_result, &sixteen_info),
            "ocr_zone_scan_result_info sixteen") ||
        sixteen_info.zone_count != 16 ||
        sixteen_info.unique_candidate_count != 1 ||
        sixteen_info.membership_count != 16 ||
        sixteen_info.source.stream != info.source.stream ||
        sixteen_info.source.epoch != info.source.epoch ||
        sixteen_info.source.sequence != info.source.sequence ||
        sixteen_info.source.geometry != info.source.geometry ||
        sixteen_info.output_space != MADOPILOT_SPACE_CAPTURE_PIXELS ||
        sixteen_info.source_envelope.space != MADOPILOT_SPACE_CAPTURE_PIXELS ||
        sixteen_info.source_envelope.left != 0 ||
        sixteen_info.source_envelope.top != 0 ||
        sixteen_info.source_envelope.right != 32 ||
        sixteen_info.source_envelope.bottom != 24) {
        goto fail;
    }
    madopilot_ocr_zone_result_t last_group = {0};
    madopilot_str_t shared_text = {0};
    for (size_t index = 0; index < 16; ++index) {
        last_group.struct_size = (uint32_t)sizeof(last_group);
        madopilot_ocr_region_t member = {0};
        member.struct_size = (uint32_t)sizeof(member);
        madopilot_str_t member_text = {0};
        if (!ok(api->ocr_zone_scan_result_zone_at(sixteen_result, index, &last_group),
                "ocr_zone_scan_result_zone_at sixteen") ||
            last_group.region_count != 1 ||
            last_group.effective_zone.space != expanded_zones[index].region.space ||
            last_group.effective_zone.left != expanded_zones[index].region.left ||
            last_group.effective_zone.top != expanded_zones[index].region.top ||
            last_group.effective_zone.right != expanded_zones[index].region.right ||
            last_group.effective_zone.bottom != expanded_zones[index].region.bottom ||
            !ok(api->ocr_zone_scan_result_region_at(sixteen_result, index, 0, &member),
                "ocr_zone_scan_result_region_at sixteen") ||
            member.confidence != region.confidence ||
            !ok(api->ocr_zone_scan_result_text_at(sixteen_result, index, 0, &member_text),
                "ocr_zone_scan_result_text_at sixteen") ||
            member_text.len != text.len ||
            memcmp(member_text.data, text.data, text.len) != 0) {
            goto fail;
        }
        for (size_t point = 0; point < 4; ++point) {
            if (member.points[point].x != region.points[point].x ||
                member.points[point].y != region.points[point].y) {
                goto fail;
            }
        }
        if (index == 0) shared_text = member_text;
        if (member_text.data != shared_text.data || member_text.len != shared_text.len) {
            goto fail;
        }
    }

    madopilot_diagnostic_drain_state_t drain_state =
        MADOPILOT_DIAGNOSTIC_DRAIN_OPEN_EMPTY;
    if (!ok(api->diagnostic_reader_drain(reader, &drain_state, &batch),
            "diagnostic_reader_drain") ||
        drain_state != MADOPILOT_DIAGNOSTIC_DRAIN_BATCH || batch == NULL) {
        goto fail;
    }
    madopilot_diagnostic_batch_info_t batch_info = {0};
    batch_info.struct_size = (uint32_t)sizeof(batch_info);
    if (!ok(api->diagnostic_batch_info(batch, &batch_info),
            "diagnostic_batch_info")) {
        goto fail;
    }
    int saw_admission = 0;
    int saw_singular_terminal = 0;
    int saw_grouped_terminal = 0;
    for (size_t index = 0; index < (size_t)batch_info.record_count; ++index) {
        madopilot_diagnostic_record_t record = {0};
        record.struct_size = (uint32_t)sizeof(record);
        if (!ok(api->diagnostic_batch_record_at(batch, index, &record),
                "diagnostic_batch_record_at")) {
            goto fail;
        }
        if (record.kind == MADOPILOT_DIAGNOSTIC_KIND_OPERATION_STARTED &&
            record.operation == MADOPILOT_DIAGNOSTIC_OPERATION_OCR_RECOGNITION) {
            saw_admission = 1;
        }
        if (record.kind == MADOPILOT_DIAGNOSTIC_KIND_OCR &&
            record.result_count == 1) {
            const uint32_t required =
                MADOPILOT_DIAGNOSTIC_RECORD_HAS_FRAME |
                MADOPILOT_DIAGNOSTIC_RECORD_HAS_SOURCE_SPACE |
                MADOPILOT_DIAGNOSTIC_RECORD_HAS_DESTINATION_SPACE |
                MADOPILOT_DIAGNOSTIC_RECORD_HAS_REGION |
                MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_MODEL_INSTANCE |
                MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_TIMING |
                MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_RESOURCES;
            saw_singular_terminal =
                (record.flags & required) == required &&
                (record.flags & MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_PROFILE) == 0u &&
                (record.flags & MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_ZONE_COUNT) == 0u &&
                record.ocr_profile == MADOPILOT_OCR_DIAGNOSTIC_PROFILE_UNSPECIFIED &&
                record.ocr_outcome == MADOPILOT_OCR_DIAGNOSTIC_OUTCOME_RECOGNIZED &&
                record.ocr_model_instance != 0 &&
                record.frame.sequence == info.source.sequence;
        } else if (record.kind == MADOPILOT_DIAGNOSTIC_KIND_OCR &&
                   record.result_count == 2) {
            const uint32_t required =
                MADOPILOT_DIAGNOSTIC_RECORD_HAS_FRAME |
                MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_SOURCE_ENVELOPE |
                MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_ZONE_COUNT |
                MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_RESULT_COUNTS;
            const uint32_t forbidden =
                MADOPILOT_DIAGNOSTIC_RECORD_HAS_REGION |
                MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_REQUESTED_REGION |
                MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_RESULT_BYTES |
                MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_BACKEND_WORK;
            saw_grouped_terminal =
                (record.flags & required) == required &&
                (record.flags & forbidden) == 0u &&
                record.ocr_zone_count == 3 &&
                record.ocr_unique_candidate_count == 1 &&
                record.ocr_membership_count == 2 &&
                record.ocr_source_envelope.left == 0 &&
                record.ocr_source_envelope.top == 0 &&
                record.ocr_source_envelope.right == 32 &&
                record.ocr_source_envelope.bottom == 12;
        }
    }
    if (!saw_admission || !saw_singular_terminal || !saw_grouped_terminal) {
        fprintf(stderr, "OCR diagnostics were incomplete\n");
        goto fail;
    }
    api->diagnostic_batch_release(batch);
    batch = NULL;
    api->diagnostic_reader_release(reader);
    reader = NULL;

    printf("ocr: sequence=%llu text=", (unsigned long long)info.source.sequence);
    fwrite(text.data, 1, text.len, stdout);
    printf(" confidence=%.5f\n", region.confidence);
    printf("ocr-zones: groups=%llu unique=%llu memberships=%llu last-index=15 "
           "last-memberships=%llu sequence=%llu refused17=%d\n",
           (unsigned long long)sixteen_info.zone_count,
           (unsigned long long)sixteen_info.unique_candidate_count,
           (unsigned long long)sixteen_info.membership_count,
           (unsigned long long)last_group.region_count,
           (unsigned long long)sixteen_info.source.sequence, (int)refused_status);
    api->ocr_zone_scan_result_release(sixteen_result);
    sixteen_result = NULL;
    api->ocr_result_release(result);
    result = NULL;
    api->ocr_zone_scan_result_release(grouped_result);
    grouped_result = NULL;
    return 0;

fail:
    if (error != NULL) api->error_release(error);
    if (result != NULL) api->ocr_result_release(result);
    if (grouped_result != NULL) api->ocr_zone_scan_result_release(grouped_result);
    if (sixteen_result != NULL) api->ocr_zone_scan_result_release(sixteen_result);
    if (refused_result != NULL) api->ocr_zone_scan_result_release(refused_result);
    if (batch != NULL) api->diagnostic_batch_release(batch);
    if (reader != NULL) api->diagnostic_reader_release(reader);
    if (frame != NULL) api->frame_release(frame);
    if (package != NULL) api->package_release(package);
    if (session != NULL) {
        madopilot_error_t* close_error = NULL;
        api->session_close(session, &operation, &close_error);
        if (close_error != NULL) api->error_release(close_error);
        api->session_release(session);
    }
    if (targets != NULL) api->target_list_release(targets);
    if (engine != NULL) api->engine_release(engine);
    return 1;
}
