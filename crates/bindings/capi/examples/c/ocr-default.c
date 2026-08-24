/* Integrated default OCR through the released C ABI 1.3 surface. */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <madopilot/madopilot.h>

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
    const char* model_root = NULL;
    const char* runtime_path = NULL;
    for (int index = 1; index + 1 < argc; index += 2) {
        if (strcmp(argv[index], "--model-root") == 0) {
            model_root = argv[index + 1];
        } else if (strcmp(argv[index], "--runtime") == 0) {
            runtime_path = argv[index + 1];
        }
    }
    if (model_root == NULL || runtime_path == NULL) {
        fprintf(stderr, "usage: %s --model-root <dir> --runtime <file>\n", argv[0]);
        return 2;
    }

    const madopilot_api_t* api = NULL;
    if (!ok(madopilot_get_api(MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR,
                              sizeof(madopilot_api_t), &api),
            "madopilot_get_api") ||
        api == NULL ||
        api->struct_size < MADOPILOT_API_SIZE_ENGINE_CREATE_WITH_DEFAULT_OCR) {
        return 1;
    }

    madopilot_build_info_t build = {0};
    build.struct_size = (uint32_t)sizeof(build);
    if (!ok(api->describe_build(&build), "describe_build")) {
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
    source.target_name = borrow("default-ocr-blank");

    madopilot_engine_options_t options = {0};
    options.struct_size = (uint32_t)sizeof(options);
    madopilot_default_ocr_options_t default_ocr = {0};
    default_ocr.struct_size = (uint32_t)sizeof(default_ocr);
    default_ocr.model_root = borrow(model_root);
    default_ocr.runtime_path = borrow(runtime_path);
    madopilot_operation_t operation = {0};
    operation.struct_size = (uint32_t)sizeof(operation);

    madopilot_engine_t* engine = NULL;
    madopilot_target_list_t* targets = NULL;
    madopilot_session_t* session = NULL;
    madopilot_frame_t* frame = NULL;
    madopilot_ocr_result_t* result = NULL;
    madopilot_error_t* error = NULL;

    if (!ok(api->engine_create_with_default_ocr(
                &source, &options, &default_ocr, &operation, &engine, &error),
            "engine_create_with_default_ocr")) {
        goto fail;
    }
    madopilot_engine_capabilities_t capabilities = {0};
    capabilities.struct_size = (uint32_t)sizeof(capabilities);
    if (!ok(api->engine_capabilities(engine, &capabilities), "engine_capabilities") ||
        (capabilities.flags & MADOPILOT_ENGINE_HAS_OCR) == 0u ||
        !ok(api->engine_discover(engine, &operation, &targets, &error),
            "engine_discover")) {
        goto fail;
    }

    madopilot_open_request_t open = {0};
    open.struct_size = (uint32_t)sizeof(open);
    if (!ok(api->session_open(engine, targets, 0, &open, &operation, &session, &error),
            "session_open") ||
        !ok(api->session_acquire_frame(session, &operation, &frame, &error),
            "session_acquire_frame")) {
        goto fail;
    }

    madopilot_ocr_request_t request = {0};
    request.struct_size = (uint32_t)sizeof(request);
    request.frame = frame;
    request.model_id = build.default_ocr_model;
    request.backend_id = build.default_ocr_backend;
    request.backend_version = build.default_ocr_backend_version;
    request.output_space = MADOPILOT_SPACE_CAPTURE_PIXELS;
    request.clip_policy = MADOPILOT_CLIP_POLICY_REJECT;
    if (!ok(api->session_recognize(session, &request, &operation, &result, &error),
            "session_recognize full")) {
        goto fail;
    }

    madopilot_ocr_result_info_t full = {0};
    full.struct_size = (uint32_t)sizeof(full);
    if (!ok(api->ocr_result_info(result, &full), "ocr_result_info full") ||
        full.region_count != 0) {
        goto fail;
    }
    api->ocr_result_release(result);
    result = NULL;

    request.flags = MADOPILOT_OCR_HAS_REGION;
    request.region.space = MADOPILOT_SPACE_CAPTURE_PIXELS;
    request.region.left = 8;
    request.region.top = 8;
    request.region.right = 40;
    request.region.bottom = 40;
    if (!ok(api->session_recognize(session, &request, &operation, &result, &error),
            "session_recognize region")) {
        goto fail;
    }

    madopilot_ocr_result_info_t region = {0};
    region.struct_size = (uint32_t)sizeof(region);
    if (!ok(api->ocr_result_info(result, &region), "ocr_result_info region") ||
        region.region_count != 0 || region.source.sequence != full.source.sequence ||
        region.effective_region.left != 8 || region.effective_region.top != 8 ||
        region.effective_region.right != 40 || region.effective_region.bottom != 40) {
        goto fail;
    }
    if (!ok(api->session_close(session, &operation, &error), "session_close") ||
        !ok(api->session_close(session, &operation, &error), "session_close repeated")) {
        goto fail;
    }

    printf("default-ocr: backend=%.*s model=%.*s full=%llu region=%llu\n",
           (int)build.default_ocr_backend.len, build.default_ocr_backend.data,
           (int)build.default_ocr_model.len, build.default_ocr_model.data,
           (unsigned long long)full.region_count,
           (unsigned long long)region.region_count);

    api->ocr_result_release(result);
    api->frame_release(frame);
    api->session_release(session);
    api->target_list_release(targets);
    api->engine_release(engine);
    return 0;

fail:
    if (error != NULL) api->error_release(error);
    if (result != NULL) api->ocr_result_release(result);
    if (frame != NULL) api->frame_release(frame);
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
