/* Explicit bounded OCR profile and grouped zones through C ABI 1.4. */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <madopilot/madopilot.h>

static madopilot_str_t borrow(const char* text)
{
    madopilot_str_t view;
    view.data = text;
    view.len = strlen(text);
    return view;
}

static int ok(madopilot_status_t status, const char* call)
{
    if (status == MADOPILOT_STATUS_OK) return 1;
    fprintf(stderr, "%s failed with status %d\n", call, (int)status);
    return 0;
}

static madopilot_ocr_zone_t zone(int32_t left, int32_t top,
                                 int32_t right, int32_t bottom)
{
    madopilot_ocr_zone_t value = {0};
    value.struct_size = (uint32_t)sizeof(value);
    value.region.space = MADOPILOT_SPACE_CAPTURE_PIXELS;
    value.region.left = left;
    value.region.top = top;
    value.region.right = right;
    value.region.bottom = bottom;
    value.clip_policy = MADOPILOT_CLIP_POLICY_REJECT;
    return value;
}

int main(int argc, char** argv)
{
    const char* model_root = NULL;
    const char* runtime_path = NULL;
    for (int index = 1; index + 1 < argc; index += 2) {
        if (strcmp(argv[index], "--model-root") == 0) {
            model_root = argv[index + 1];
        } else if (strcmp(argv[index], "--runtime") == 0) {
            runtime_path = argv[index + 1];
        }
    }
    if (model_root == NULL) model_root = getenv("MADO_PILOT_G004_MODEL_ROOT");
    if (runtime_path == NULL) runtime_path = getenv("MADO_PILOT_ONNX_RUNTIME");
    if (model_root == NULL || runtime_path == NULL) {
        fprintf(stderr, "profile OCR prerequisites are not configured; skipping\n");
        return 77;
    }

    const madopilot_api_t* api = NULL;
    if (!ok(madopilot_get_api(MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR,
                              sizeof(madopilot_api_t), &api),
            "madopilot_get_api") ||
        api == NULL ||
        api->struct_size < MADOPILOT_API_SIZE_OCR_ZONE_SCAN_RESULT_TEXT_AT) {
        return 1;
    }

    uint8_t pixels[64u * 64u * 4u] = {0};
    madopilot_replay_frame_t supplied = {0};
    supplied.struct_size = (uint32_t)sizeof(supplied);
    supplied.width = 64;
    supplied.height = 64;
    supplied.format = MADOPILOT_PIXEL_FORMAT_BGRA8;
    supplied.continuity = MADOPILOT_CONTINUITY_CONTINUOUS;
    supplied.pixels.data = pixels;
    supplied.pixels.len = sizeof(pixels);
    supplied.stride = 64u * 4u;

    madopilot_source_t source = {0};
    source.struct_size = (uint32_t)sizeof(source);
    source.kind = MADOPILOT_SOURCE_REPLAY_MEMORY;
    source.frames = &supplied;
    source.frame_count = 1;
    source.frame_stride = sizeof(supplied);
    source.target_name = borrow("bounded-profile-blank");

    madopilot_engine_options_t options = {0};
    options.struct_size = (uint32_t)sizeof(options);
    madopilot_ocr_profile_options_t profile = {0};
    profile.struct_size = (uint32_t)sizeof(profile);
    profile.kind = MADOPILOT_OCR_PROFILE_BOUNDED_DETECTOR;
    profile.model_root = borrow(model_root);
    profile.runtime_path = borrow(runtime_path);
    madopilot_operation_t operation = {0};
    operation.struct_size = (uint32_t)sizeof(operation);

    madopilot_engine_t* engine = NULL;
    madopilot_target_list_t* targets = NULL;
    madopilot_session_t* session = NULL;
    madopilot_frame_t* frame = NULL;
    madopilot_ocr_zone_scan_result_t* result = NULL;
    madopilot_error_t* error = NULL;
    int success = 0;

    if (!ok(api->engine_create_with_ocr_profile(
                &source, &options, &profile, &operation, &engine, &error),
            "engine_create_with_ocr_profile")) goto cleanup;

    madopilot_build_info_t build = {0};
    build.struct_size = (uint32_t)sizeof(build);
    if (!ok(api->describe_build(&build), "describe_build") ||
        !ok(api->engine_discover(engine, &operation, &targets, &error),
            "engine_discover")) goto cleanup;

    madopilot_open_request_t open = {0};
    open.struct_size = (uint32_t)sizeof(open);
    if (!ok(api->session_open(engine, targets, 0, &open, &operation,
                              &session, &error), "session_open") ||
        !ok(api->session_acquire_frame(session, &operation, &frame, &error),
            "session_acquire_frame")) goto cleanup;

    madopilot_ocr_zone_t zones[3];
    zones[0] = zone(0, 0, 24, 24);
    zones[1] = zone(40, 0, 64, 24);
    zones[2] = zone(0, 40, 24, 64);
    madopilot_ocr_zone_scan_request_t request = {0};
    request.struct_size = (uint32_t)sizeof(request);
    request.frame = frame;
    request.model_id = build.bounded_ocr_model;
    request.backend_id = build.default_ocr_backend;
    request.backend_version = build.default_ocr_backend_version;
    request.output_space = MADOPILOT_SPACE_CAPTURE_PIXELS;
    request.zones = zones;
    request.zone_count = 3;
    request.zone_stride = sizeof(zones[0]);
    if (!ok(api->session_scan_ocr_zones(session, &request, &operation,
                                        &result, &error),
            "session_scan_ocr_zones")) goto cleanup;

    madopilot_ocr_zone_scan_result_info_t info = {0};
    info.struct_size = (uint32_t)sizeof(info);
    if (!ok(api->ocr_zone_scan_result_info(result, &info),
            "ocr_zone_scan_result_info") || info.zone_count != 3u ||
        info.unique_candidate_count != 0u || info.membership_count != 0u) {
        goto cleanup;
    }

    /* Parents close in reverse creation order; the result remains independent. */
    if (!ok(api->session_close(session, &operation, &error), "session_close")) {
        goto cleanup;
    }
    api->frame_release(frame); frame = NULL;
    api->session_release(session); session = NULL;
    api->target_list_release(targets); targets = NULL;
    api->engine_release(engine); engine = NULL;

    for (size_t zone_index = 0; zone_index < (size_t)info.zone_count; ++zone_index) {
        madopilot_ocr_zone_result_t group = {0};
        group.struct_size = (uint32_t)sizeof(group);
        if (!ok(api->ocr_zone_scan_result_zone_at(result, zone_index, &group),
                "ocr_zone_scan_result_zone_at")) goto cleanup;
        for (size_t region_index = 0;
             region_index < (size_t)group.region_count; ++region_index) {
            madopilot_str_t text = {0};
            if (!ok(api->ocr_zone_scan_result_text_at(
                        result, zone_index, region_index, &text),
                    "ocr_zone_scan_result_text_at")) goto cleanup;
            printf("%.*s\n", (int)text.len, text.data);
        }
    }
    success = 1;

cleanup:
    if (error != NULL) api->error_release(error);
    if (result != NULL) api->ocr_zone_scan_result_release(result);
    if (frame != NULL) api->frame_release(frame);
    if (session != NULL) {
        madopilot_error_t* close_error = NULL;
        api->session_close(session, &operation, &close_error);
        if (close_error != NULL) api->error_release(close_error);
        api->session_release(session);
    }
    if (targets != NULL) api->target_list_release(targets);
    if (engine != NULL) api->engine_release(engine);
    return success ? 0 : 1;
}
