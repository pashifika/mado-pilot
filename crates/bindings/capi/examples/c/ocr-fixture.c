/* Deterministic one-shot OCR through the released C ABI 1.3 surface. */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "ocr-private-fixture.h"

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
        api->struct_size < MADOPILOT_API_SIZE_OCR_RESULT_TEXT_AT) {
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
    madopilot_diagnostic_reader_t* reader = NULL;
    madopilot_diagnostic_batch_t* batch = NULL;
    madopilot_engine_options_t options = {0};
    options.struct_size = (uint32_t)sizeof(options);
    options.diagnostic_level = MADOPILOT_DIAGNOSTIC_LEVEL_DEBUG;
    options.diagnostic_capacity = 16;
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
    int saw_terminal = 0;
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
        if (record.kind == MADOPILOT_DIAGNOSTIC_KIND_OCR) {
            const uint32_t required =
                MADOPILOT_DIAGNOSTIC_RECORD_HAS_FRAME |
                MADOPILOT_DIAGNOSTIC_RECORD_HAS_SOURCE_SPACE |
                MADOPILOT_DIAGNOSTIC_RECORD_HAS_DESTINATION_SPACE |
                MADOPILOT_DIAGNOSTIC_RECORD_HAS_REGION |
                MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_MODEL_INSTANCE |
                MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_TIMING |
                MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_RESOURCES;
            saw_terminal =
                (record.flags & required) == required &&
                (record.flags & MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_PROFILE) == 0u &&
                record.ocr_profile == MADOPILOT_OCR_DIAGNOSTIC_PROFILE_UNSPECIFIED &&
                record.ocr_outcome == MADOPILOT_OCR_DIAGNOSTIC_OUTCOME_RECOGNIZED &&
                record.ocr_model_instance != 0 && record.result_count == 1 &&
                record.frame.sequence == info.source.sequence;
        }
    }
    if (!saw_admission || !saw_terminal) {
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
    api->ocr_result_release(result);
    result = NULL;
    return 0;

fail:
    if (error != NULL) api->error_release(error);
    if (result != NULL) api->ocr_result_release(result);
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
