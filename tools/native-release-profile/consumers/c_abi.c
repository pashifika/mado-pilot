/* Released C ABI consumer: exact matching, CPU OCR, interruption and retained ownership.
 * Shared scene: crates/bindings/capi/examples/deterministic-scene.h.
 * Emits content-free outcomes; every failed check exits nonzero after owned cleanup.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#ifdef _WIN32
#include <windows.h>
#endif

#include "deterministic-scene.h"
#include "madopilot/madopilot.h"

/* The accepted blank OCR fixture: a zeroed 64x64 BGRA frame. */
#define BLANK_EXTENT 64u
#define BLANK_BYTES ((size_t)BLANK_EXTENT * (size_t)BLANK_EXTENT * 4u)

static const madopilot_api_t* api;
static madopilot_build_info_t build;
static const char* failed_check;
static madopilot_status_t failed_status;

/* Records the first failed check; later failures never replace it. */
static int fail(const char* check, madopilot_status_t status)
{
    if (failed_check == NULL) {
        failed_check = check;
        failed_status = status;
    }
    return 0;
}

static int require(int condition, const char* check)
{
    return condition ? 1 : fail(check, MADOPILOT_STATUS_OK);
}

static int require_ok(madopilot_status_t status, const char* check)
{
    return status == MADOPILOT_STATUS_OK ? 1 : fail(check, status);
}

static madopilot_str_t borrow(const char* text)
{
    madopilot_str_t view;
    view.data = text;
    view.len = strlen(text);
    return view;
}

static int same_text(madopilot_str_t left, madopilot_str_t right)
{
    return left.len == right.len && left.len != 0 &&
           memcmp(left.data, right.data, left.len) == 0;
}

static int same_stamp(const madopilot_frame_stamp_t* left,
                      const madopilot_frame_stamp_t* right)
{
    return left->stream == right->stream && left->epoch == right->epoch &&
           left->sequence == right->sequence && left->geometry == right->geometry;
}

static int same_rect(const madopilot_pixel_rect_t* rect, int32_t left, int32_t top,
                     int32_t right, int32_t bottom)
{
    return rect->space == MADOPILOT_SPACE_CAPTURE_PIXELS && rect->left == left &&
           rect->top == top && rect->right == right && rect->bottom == bottom;
}

/* A static replay source's first publication: a nonzero stream at epoch 0,
 * sequence 0, geometry 0. */
static int first_publication(const madopilot_frame_stamp_t* stamp)
{
    return stamp->stream != 0 && stamp->epoch == 0 && stamp->sequence == 0 &&
           stamp->geometry == 0;
}

/* A refused call: `expected` was returned, no result was published, and the one
 * owned typed error carries the same status. The error is released here. */
static int refused(madopilot_status_t status, madopilot_error_t** error, const void* output,
                   madopilot_status_t expected, const char* check)
{
    madopilot_error_detail_t detail = {0};
    int typed = 0;
    detail.struct_size = (uint32_t)sizeof(detail);
    if (*error != NULL && api->error_describe(*error, &detail) == MADOPILOT_STATUS_OK) {
        typed = detail.status == status;
    }
    if (!require_ok(api->error_release(*error), check)) {
        typed = 0;
    }
    *error = NULL;
    return require(status == expected && output == NULL && typed, check);
}

/* Returns the final canonical UTF-8 path; the caller frees it. */
static char* controlled_path(const char* value)
{
#ifdef _WIN32
    wchar_t source[32768];
    wchar_t final_path[32768];
    if (MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, value, -1, source, 32768) == 0) {
        return NULL;
    }
    HANDLE handle = CreateFileW(source, 0, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                                NULL, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, NULL);
    if (handle == INVALID_HANDLE_VALUE) return NULL;
    DWORD written = GetFinalPathNameByHandleW(
        handle, final_path, 32768, FILE_NAME_NORMALIZED | VOLUME_NAME_DOS);
    CloseHandle(handle);
    if (written == 0 || written >= 32768) return NULL;
    int bytes = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, final_path, -1, NULL, 0, NULL, NULL);
    if (bytes == 0) return NULL;
    char* canonical = (char*)malloc((size_t)bytes);
    if (canonical == NULL) return NULL;
    if (WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, final_path, -1,
                            canonical, bytes, NULL, NULL) == 0) {
        free(canonical);
        return NULL;
    }
    return canonical;
#else
    return realpath(value, NULL);
#endif
}

/* One frame of `width` x `height` packed `format` pixels as a memory replay
 * source named `name`. `pixels` is borrowed only for engine construction. */
static void replay_source(madopilot_replay_frame_t* frame, madopilot_source_t* source,
                          const char* name, uint32_t width, uint32_t height,
                          madopilot_pixel_format_t format, const uint8_t* pixels, size_t len)
{
    memset(frame, 0, sizeof(*frame));
    frame->struct_size = (uint32_t)sizeof(*frame);
    frame->width = width;
    frame->height = height;
    frame->format = format;
    frame->continuity = MADOPILOT_CONTINUITY_CONTINUOUS;
    frame->pixels.data = pixels;
    frame->pixels.len = len;

    memset(source, 0, sizeof(*source));
    source->struct_size = (uint32_t)sizeof(*source);
    source->kind = MADOPILOT_SOURCE_REPLAY_MEMORY;
    source->frames = frame;
    source->frame_count = 1;
    source->frame_stride = sizeof(*frame);
    source->target_name = borrow(name);
}

/* Discovers the one replay target and opens it. */
static int open_only_target(const madopilot_engine_t* engine, const madopilot_operation_t* operation,
                            const madopilot_open_request_t* request, madopilot_session_t** session,
                            const char* discover_check, const char* open_check)
{
    madopilot_target_list_t* targets = NULL;
    size_t count = 0;
    madopilot_status_t status;
    int ok = require_ok(api->engine_discover(engine, operation, &targets, NULL), discover_check) &&
             require_ok(api->target_list_count(targets, &count), discover_check) &&
             require(count == 1, discover_check) &&
             require_ok(api->session_open(engine, targets, 0, request, operation, session, NULL),
                        open_check);
    /* Opening copied the identity, so the list is not needed afterwards. */
    status = api->target_list_release(targets);
    return ok && require_ok(status, open_check);
}

/* Both planted copies, each reported once at its planted origin with the patch
 * extent, over the whole scene, under the searched frame's identity. */
static int planted_found(const madopilot_result_t* result, const madopilot_frame_stamp_t* stamp)
{
    madopilot_result_info_t info = {0};
    madopilot_frame_stamp_t searched = {0};
    int seen[2] = {0, 0};
    size_t at;

    info.struct_size = (uint32_t)sizeof(info);
    searched.struct_size = (uint32_t)sizeof(searched);
    if (api->result_describe(result, &info) != MADOPILOT_STATUS_OK ||
        api->result_stamp(result, &searched) != MADOPILOT_STATUS_OK || info.match_count != 2 ||
        !same_stamp(&searched, stamp) ||
        !same_rect(&info.searched, 0, 0, (int32_t)SCENE_WIDTH, (int32_t)SCENE_HEIGHT)) {
        return 0;
    }
    for (at = 0; at < 2; ++at) {
        madopilot_match_t match = {0};
        size_t planted;
        match.struct_size = (uint32_t)sizeof(match);
        if (api->result_match(result, at, &match) != MADOPILOT_STATUS_OK ||
            !same_text(match.template_id, borrow("panel.patch")) ||
            match.bounds.space != MADOPILOT_SPACE_CAPTURE_PIXELS ||
            match.bounds.right - match.bounds.left != (int32_t)PATCH_WIDTH ||
            match.bounds.bottom - match.bounds.top != (int32_t)PATCH_HEIGHT) {
            return 0;
        }
        for (planted = 0; planted < 2; ++planted) {
            if (match.bounds.left == (int32_t)SCENE_PLANTED[planted][0] &&
                match.bounds.top == (int32_t)SCENE_PLANTED[planted][1]) {
                if (seen[planted]) {
                    return 0;
                }
                seen[planted] = 1;
            }
        }
    }
    return seen[0] && seen[1];
}

/* A successful search that qualified nothing, under the searched frame's identity. */
static int nothing_found(const madopilot_result_t* result, const madopilot_frame_stamp_t* stamp)
{
    madopilot_result_info_t info = {0};
    madopilot_frame_stamp_t searched = {0};
    info.struct_size = (uint32_t)sizeof(info);
    searched.struct_size = (uint32_t)sizeof(searched);
    return api->result_describe(result, &info) == MADOPILOT_STATUS_OK &&
           api->result_stamp(result, &searched) == MADOPILOT_STATUS_OK && info.match_count == 0 &&
           same_stamp(&searched, stamp) &&
           same_rect(&info.searched, 0, 0, (int32_t)SCENE_WIDTH, (int32_t)SCENE_HEIGHT);
}

/* The whole scene mapped back byte for byte, under the frame's identity. */
static int mapping_readable(const madopilot_mapping_t* mapping, const madopilot_frame_stamp_t* stamp,
                            const uint8_t* scene)
{
    madopilot_image_t image = {0};
    madopilot_frame_stamp_t mapped = {0};
    image.struct_size = (uint32_t)sizeof(image);
    mapped.struct_size = (uint32_t)sizeof(mapped);
    return api->mapping_describe(mapping, &image) == MADOPILOT_STATUS_OK &&
           api->mapping_stamp(mapping, &mapped) == MADOPILOT_STATUS_OK &&
           image.width == SCENE_WIDTH && image.height == SCENE_HEIGHT &&
           image.bytes.data != NULL && image.bytes.len == SCENE_BYTES &&
           memcmp(image.bytes.data, scene, SCENE_BYTES) == 0 && same_stamp(&mapped, stamp);
}

/* An empty recognition of exactly the requested region by the build's default
 * OCR identity, under the recognized frame's identity. */
static int empty_recognition(const madopilot_ocr_result_t* result,
                             const madopilot_frame_stamp_t* stamp, int32_t left, int32_t top,
                             int32_t right, int32_t bottom)
{
    madopilot_ocr_result_info_t info = {0};
    info.struct_size = (uint32_t)sizeof(info);
    return api->ocr_result_info(result, &info) == MADOPILOT_STATUS_OK && info.region_count == 0 &&
           same_stamp(&info.source, stamp) &&
           same_rect(&info.effective_region, left, top, right, bottom) &&
           info.output_space == MADOPILOT_SPACE_CAPTURE_PIXELS &&
           same_text(info.backend_id, borrow("onnxruntime-cpu")) &&
           same_text(info.backend_version, borrow("0.4.0+ort-1.29.0-api17")) &&
           same_text(info.model_id, borrow("g-004-rapidocr-ppocrv4-det-v6-rec-small-v1")) &&
           same_text(info.model_version, borrow("rapidocr-3.9.2+095232a4c94f7f0e6600ba5bba1177010ad696d4")) &&
           same_text(info.profile_id, borrow("g-004-rapidocr-ppocrv4-det-v6-rec-small-v1"));
}

/* Fills an operation whose token is already cancelled, then rewrites it into
 * one whose deadline has already passed. Returns 0 when either cannot be built. */
static int cancelled_operation(madopilot_operation_t* operation,
                               madopilot_cancellation_t** cancellation, const char* check)
{
    int32_t cancelled = 0;
    memset(operation, 0, sizeof(*operation));
    operation->struct_size = (uint32_t)sizeof(*operation);
    if (!require_ok(api->cancellation_create(cancellation), check) ||
        !require_ok(api->cancellation_cancel(*cancellation), check) ||
        !require_ok(api->cancellation_is_cancelled(*cancellation, &cancelled), check) ||
        !require(cancelled != 0, check)) {
        return 0;
    }
    operation->cancellation = *cancellation;
    return 1;
}

static int expired_operation(madopilot_operation_t* operation, const char* check)
{
    uint64_t now = 0;
    memset(operation, 0, sizeof(*operation));
    operation->struct_size = (uint32_t)sizeof(*operation);
    if (!require_ok(api->clock_now(&now), check)) {
        return 0;
    }
    operation->flags = MADOPILOT_OPERATION_HAS_DEADLINE;
    operation->deadline_nanos = now;
    return 1;
}

/* Closes twice, confirms the closed state, and confirms a closed session
 * publishes nothing further. */
static int closed_idempotently(const madopilot_session_t* session,
                               const madopilot_operation_t* operation, const char* check)
{
    int32_t closed = 0;
    madopilot_frame_t* after = NULL;
    madopilot_error_t* error = NULL;
    madopilot_status_t status;
    int ok;
    if (!require_ok(api->session_close(session, operation, NULL), check) ||
        !require_ok(api->session_close(session, operation, NULL), check) ||
        !require_ok(api->session_is_closed(session, &closed), check) ||
        !require(closed != 0, check)) {
        return 0;
    }
    status = api->session_acquire_frame(session, operation, &after, &error);
    ok = refused(status, &error, after, MADOPILOT_STATUS_CLOSED, check);
    return require_ok(api->frame_release(after), check) && ok;
}

/* The deterministic matching workflow, with its refusal, close, and retention checks. */
static int matching_flow(const char* package_path, int mapping_only)
{
    madopilot_operation_t operation = {0};
    madopilot_operation_t refusing = {0};
    madopilot_frame_stamp_t stamp = {0};
    madopilot_status_t status;
    uint8_t* scene = NULL;
    madopilot_cancellation_t* cancellation = NULL;
    madopilot_engine_t* engine = NULL;
    madopilot_session_t* session = NULL;
    madopilot_frame_t* frame = NULL;
    madopilot_mapping_t* mapping = NULL;
    madopilot_package_t* package = NULL;
    madopilot_template_t* present = NULL;
    madopilot_template_t* absent = NULL;
    madopilot_result_t* found = NULL;
    madopilot_result_t* missing = NULL;
    madopilot_result_t* refused_result = NULL;
    madopilot_error_t* error = NULL;

    operation.struct_size = (uint32_t)sizeof(operation);

    /* 1. An engine over the deterministic scene, and its one open target. */
    {
        madopilot_replay_frame_t supplied;
        madopilot_source_t source;
        madopilot_open_request_t open = {0};
        scene = (uint8_t*)malloc(SCENE_BYTES);
        if (!require(scene != NULL, "matching-engine")) {
            goto cleanup;
        }
        scene_fill_rgba(scene);
        uint8_t* pixels = (uint8_t*)malloc(SCENE_BYTES);
        if (!require(pixels != NULL, "matching-engine")) goto cleanup;
        memcpy(pixels, scene, SCENE_BYTES);
        replay_source(&supplied, &source, "panel", SCENE_WIDTH, SCENE_HEIGHT,
                      MADOPILOT_PIXEL_FORMAT_RGBA8, pixels, SCENE_BYTES);
        status = api->engine_create(&source, &operation, &engine, NULL);
        for (size_t at = 0; at < SCENE_BYTES; ++at) ((volatile uint8_t*)pixels)[at] = 0xa5;
        free(pixels);
        if (!require_ok(status, "matching-engine")) goto cleanup;
        open.struct_size = (uint32_t)sizeof(open);
        open.flags = MADOPILOT_OPEN_HAS_REQUIRED_FORMAT;
        open.required_format = MADOPILOT_PIXEL_FORMAT_RGBA8;
        if (!open_only_target(engine, &operation, &open, &session, "matching-discover",
                              "matching-open")) {
            goto cleanup;
        }
    }

    /* 2. One frame, its complete identity, and a mapping that carries it. */
    {
        madopilot_frame_info_t info = {0};
        madopilot_map_request_t request = {0};
        if (!require_ok(api->session_acquire_frame(session, &operation, &frame, NULL),
                        "matching-frame")) {
            goto cleanup;
        }
        stamp.struct_size = (uint32_t)sizeof(stamp);
        info.struct_size = (uint32_t)sizeof(info);
        if (!require_ok(api->frame_stamp(frame, &stamp), "matching-frame") ||
            !require(first_publication(&stamp), "matching-frame") ||
            !require_ok(api->frame_describe(frame, &info), "matching-frame") ||
            !require(info.width == SCENE_WIDTH && info.height == SCENE_HEIGHT &&
                         same_rect(&info.bounds, 0, 0, (int32_t)SCENE_WIDTH, (int32_t)SCENE_HEIGHT),
                     "matching-frame")) {
            goto cleanup;
        }
        request.struct_size = (uint32_t)sizeof(request);
        request.format = MADOPILOT_PIXEL_FORMAT_RGBA8;
        if (!require_ok(api->frame_map(frame, &request, &operation, &mapping, NULL),
                        "matching-map") ||
            !require(mapping_readable(mapping, &stamp, scene), "matching-map")) {
            goto cleanup;
        }
    }
    if (mapping_only) {
        if (!closed_idempotently(session, &operation, "mapping-close")) goto cleanup;
        goto release_producers;
    }
    status = api->mapping_release(mapping);
    mapping = NULL;
    if (!require_ok(status, "mapping-release")) goto cleanup;

    /* 3. The tracked package and its two templates. */
    {
        madopilot_package_source_t source = {0};
        madopilot_package_info_t info = {0};
        source.struct_size = (uint32_t)sizeof(source);
        source.kind = MADOPILOT_PACKAGE_SOURCE_DIRECTORY;
        source.path = borrow(package_path);
        info.struct_size = (uint32_t)sizeof(info);
        if (!require_ok(api->package_load(engine, &source, &operation, &package, NULL),
                        "package") ||
            !require_ok(api->package_describe(package, &info), "package") ||
            !require(info.template_count == 2, "package") ||
            !require_ok(api->template_prepare_from_package(engine, package, borrow("panel.patch"),
                                                           &operation, &present, NULL),
                        "template-present") ||
            !require_ok(api->template_prepare_from_package(engine, package, borrow("panel.absent"),
                                                           &operation, &absent, NULL),
                        "template-absent")) {
            goto cleanup;
        }
    }

    /* 4. Search that exact frame: two planted copies, nothing for the absent patch. */
    {
        madopilot_find_request_t find = {0};
        find.struct_size = (uint32_t)sizeof(find);
        find.frame = frame;
        find.tmpl = present;
        if (!require_ok(api->session_find(session, &find, &operation, &found, NULL),
                        "find-present") ||
            !require(planted_found(found, &stamp), "find-present")) {
            goto cleanup;
        }
        find.tmpl = absent;
        if (!require_ok(api->session_find(session, &find, &operation, &missing, NULL),
                        "find-absent") ||
            !require(nothing_found(missing, &stamp), "find-absent")) {
            goto cleanup;
        }
        puts("MADO_PROFILE_MATCHING=passed");

        /* 5. Refusals: an already-cancelled token, then an already-passed deadline. */
        find.tmpl = present;
        if (!cancelled_operation(&refusing, &cancellation, "cancellation")) {
            goto cleanup;
        }
        status = api->session_find(session, &find, &refusing, &refused_result, &error);
        if (!refused(status, &error, refused_result, MADOPILOT_STATUS_CANCELLED, "cancellation")) {
            goto cleanup;
        }
        puts("MADO_PROFILE_CANCELLATION=refused");
        if (!expired_operation(&refusing, "deadline")) {
            goto cleanup;
        }
        status = api->session_find(session, &find, &refusing, &refused_result, &error);
        if (!refused(status, &error, refused_result, MADOPILOT_STATUS_DEADLINE_EXCEEDED,
                     "deadline")) {
            goto cleanup;
        }
        puts("MADO_PROFILE_DEADLINE=refused");
        refusing.cancellation = cancellation;
        status = api->session_find(session, &find, &refusing, &refused_result, &error);
        if (!refused(status, &error, refused_result, MADOPILOT_STATUS_CANCELLED,
                     "cancellation-precedence")) goto cleanup;
    }

    /* 6. Close twice; a closed session publishes nothing further. */
    if (!closed_idempotently(session, &operation, "close")) {
        goto cleanup;
    }
    puts("MADO_PROFILE_CLOSE=idempotent");

    /* 7. Release every producer; what the caller owns stays readable and identical. */
release_producers:
    status = api->template_release(present);
    present = NULL;
    if (!require_ok(status, "retained")) {
        goto cleanup;
    }
    status = api->template_release(absent);
    absent = NULL;
    if (!require_ok(status, "retained")) {
        goto cleanup;
    }
    status = api->package_release(package);
    package = NULL;
    if (!require_ok(status, "retained")) {
        goto cleanup;
    }
    status = api->frame_release(frame);
    frame = NULL;
    if (!require_ok(status, "retained")) {
        goto cleanup;
    }
    status = api->session_release(session);
    session = NULL;
    if (!require_ok(status, "retained")) {
        goto cleanup;
    }
    status = api->engine_release(engine);
    engine = NULL;
    if (!require_ok(status, "retained")) {
        goto cleanup;
    }
    if (!require(mapping_only ? mapping_readable(mapping, &stamp, scene) :
                     planted_found(found, &stamp) && nothing_found(missing, &stamp),
                 "retained")) {
        goto cleanup;
    }
    puts(mapping_only ? "MADO_PROFILE_MAPPING=retained" : "MADO_PROFILE_RETAINED=readable");

cleanup:
    /* Reverse ownership order. Every release accepts null, so this path is the
     * same however far the flow got, and every release status still counts. */
    require_ok(api->error_release(error), "release");
    require_ok(api->result_release(refused_result), "release");
    require_ok(api->result_release(missing), "release");
    require_ok(api->result_release(found), "release");
    require_ok(api->template_release(absent), "release");
    require_ok(api->template_release(present), "release");
    require_ok(api->package_release(package), "release");
    require_ok(api->mapping_release(mapping), "release");
    require_ok(api->frame_release(frame), "release");
    if (session != NULL) {
        require_ok(api->session_close(session, &operation, NULL), "release");
    }
    require_ok(api->session_release(session), "release");
    require_ok(api->engine_release(engine), "release");
    require_ok(api->cancellation_release(cancellation), "release");
    free(scene);
    return failed_check == NULL;
}

/* The accepted CPU blank-frame OCR workflow, with the same refusal, close, and
 * retention checks. */
static int ocr_flow(const char* model_root, const char* runtime_path)
{
    madopilot_operation_t operation = {0};
    madopilot_operation_t refusing = {0};
    madopilot_frame_stamp_t stamp = {0};
    madopilot_ocr_request_t request = {0};
    madopilot_status_t status;
    uint8_t* blank = NULL;
    char* controlled_root = NULL;
    char* controlled_runtime = NULL;
    madopilot_cancellation_t* cancellation = NULL;
    madopilot_engine_t* engine = NULL;
    madopilot_session_t* session = NULL;
    madopilot_frame_t* frame = NULL;
    madopilot_ocr_result_t* full = NULL;
    madopilot_ocr_result_t* bounded = NULL;
    madopilot_ocr_result_t* refused_result = NULL;
    madopilot_error_t* error = NULL;

    operation.struct_size = (uint32_t)sizeof(operation);

    /* 1. An engine with the accepted default CPU OCR over one blank frame. */
    {
        madopilot_replay_frame_t supplied;
        madopilot_source_t source;
        madopilot_engine_options_t options = {0};
        madopilot_default_ocr_options_t default_ocr = {0};
        madopilot_engine_capabilities_t capabilities = {0};
        madopilot_open_request_t open = {0};
        controlled_root = controlled_path(model_root);
        controlled_runtime = controlled_path(runtime_path);
        blank = (uint8_t*)calloc(BLANK_BYTES, 1);
        if (!require(controlled_root != NULL && controlled_runtime != NULL && blank != NULL,
                     "ocr-prerequisite")) {
            goto cleanup;
        }
        replay_source(&supplied, &source, "default-ocr-blank", BLANK_EXTENT, BLANK_EXTENT,
                      MADOPILOT_PIXEL_FORMAT_BGRA8, blank, BLANK_BYTES);
        supplied.stride = (uint64_t)BLANK_EXTENT * 4u;
        options.struct_size = (uint32_t)sizeof(options);
        default_ocr.struct_size = (uint32_t)sizeof(default_ocr);
        default_ocr.model_root = borrow(controlled_root);
        default_ocr.runtime_path = borrow(controlled_runtime);
        if (!require_ok(api->engine_create_with_default_ocr(&source, &options, &default_ocr,
                                                            &operation, &engine, NULL),
                        "ocr-engine")) {
            goto cleanup;
        }
        for (size_t at = 0; at < BLANK_BYTES; ++at) ((volatile uint8_t*)blank)[at] = 0xa5;
        free(blank);
        blank = NULL;
        madopilot_ocr_provider_descriptor_t provider = {0};
        provider.struct_size = (uint32_t)sizeof(provider);
        if (!require_ok(api->engine_ocr_provider_descriptor(engine, &provider), "ocr-provider") ||
            !require(provider.active_provider == MADOPILOT_OCR_EXECUTION_PROVIDER_CPU &&
                     provider.requested_policy == MADOPILOT_OCR_PROVIDER_POLICY_CPU &&
                     provider.initialization_fell_back == 0 &&
                     same_text(provider.runtime_profile, borrow("onnxruntime-1.29.0-api17-cpu")),
                     "ocr-provider")) {
            goto cleanup;
        }
        capabilities.struct_size = (uint32_t)sizeof(capabilities);
        if (!require_ok(api->engine_capabilities(engine, &capabilities), "ocr-engine") ||
            !require((capabilities.flags & MADOPILOT_ENGINE_HAS_OCR) != 0u, "ocr-engine")) {
            goto cleanup;
        }
        open.struct_size = (uint32_t)sizeof(open);
        if (!open_only_target(engine, &operation, &open, &session, "ocr-discover", "ocr-open")) {
            goto cleanup;
        }
    }

    /* 2. One frame and its identity. */
    if (!require_ok(api->session_acquire_frame(session, &operation, &frame, NULL), "ocr-frame")) {
        goto cleanup;
    }
    stamp.struct_size = (uint32_t)sizeof(stamp);
    if (!require_ok(api->frame_stamp(frame, &stamp), "ocr-frame") ||
        !require(first_publication(&stamp), "ocr-frame")) {
        goto cleanup;
    }

    /* 3. Recognize the whole frame and one region: a blank frame reads as nothing. */
    request.struct_size = (uint32_t)sizeof(request);
    request.frame = frame;
    request.model_id = build.default_ocr_model;
    request.backend_id = build.default_ocr_backend;
    request.backend_version = build.default_ocr_backend_version;
    request.output_space = MADOPILOT_SPACE_CAPTURE_PIXELS;
    request.clip_policy = MADOPILOT_CLIP_POLICY_REJECT;
    if (!require_ok(api->session_recognize(session, &request, &operation, &full, NULL), "ocr-full") ||
        !require(empty_recognition(full, &stamp, 0, 0, (int32_t)BLANK_EXTENT, (int32_t)BLANK_EXTENT),
                 "ocr-full")) {
        goto cleanup;
    }
    request.flags = MADOPILOT_OCR_HAS_REGION;
    request.region.space = MADOPILOT_SPACE_CAPTURE_PIXELS;
    request.region.left = 8;
    request.region.top = 8;
    request.region.right = 40;
    request.region.bottom = 40;
    if (!require_ok(api->session_recognize(session, &request, &operation, &bounded, NULL),
                    "ocr-region") ||
        !require(empty_recognition(bounded, &stamp, 8, 8, 40, 40), "ocr-region")) {
        goto cleanup;
    }

    /* 4. Refusals on recognition. */
    if (!cancelled_operation(&refusing, &cancellation, "ocr-cancellation")) {
        goto cleanup;
    }
    status = api->session_recognize(session, &request, &refusing, &refused_result, &error);
    if (!refused(status, &error, refused_result, MADOPILOT_STATUS_CANCELLED, "ocr-cancellation")) {
        goto cleanup;
    }
    if (!expired_operation(&refusing, "ocr-deadline")) {
        goto cleanup;
    }
    status = api->session_recognize(session, &request, &refusing, &refused_result, &error);
    if (!refused(status, &error, refused_result, MADOPILOT_STATUS_DEADLINE_EXCEEDED,
                 "ocr-deadline")) {
        goto cleanup;
    }
    refusing.cancellation = cancellation;
    status = api->session_recognize(session, &request, &refusing, &refused_result, &error);
    if (!refused(status, &error, refused_result, MADOPILOT_STATUS_CANCELLED,
                 "ocr-cancellation-precedence")) goto cleanup;

    /* 5. Close twice; a closed session publishes nothing further. */
    if (!closed_idempotently(session, &operation, "ocr-close")) {
        goto cleanup;
    }

    /* 6. Release every producer; both results stay readable and identical. */
    status = api->frame_release(frame);
    frame = NULL;
    if (!require_ok(status, "ocr-retained")) {
        goto cleanup;
    }
    status = api->session_release(session);
    session = NULL;
    if (!require_ok(status, "ocr-retained")) {
        goto cleanup;
    }
    status = api->engine_release(engine);
    engine = NULL;
    if (!require_ok(status, "ocr-retained")) {
        goto cleanup;
    }
    if (!require(empty_recognition(full, &stamp, 0, 0, (int32_t)BLANK_EXTENT, (int32_t)BLANK_EXTENT) &&
                     empty_recognition(bounded, &stamp, 8, 8, 40, 40),
                 "ocr-retained")) {
        goto cleanup;
    }
    puts("MADO_PROFILE_OCR=passed");

cleanup:
    require_ok(api->error_release(error), "release");
    require_ok(api->ocr_result_release(refused_result), "release");
    require_ok(api->ocr_result_release(bounded), "release");
    require_ok(api->ocr_result_release(full), "release");
    require_ok(api->frame_release(frame), "release");
    if (session != NULL) {
        require_ok(api->session_close(session, &operation, NULL), "release");
    }
    require_ok(api->session_release(session), "release");
    require_ok(api->engine_release(engine), "release");
    require_ok(api->cancellation_release(cancellation), "release");
    free(blank);
    free(controlled_runtime);
    free(controlled_root);
    return failed_check == NULL;
}

/* Prints the terminal line: 0 when every check held, 1 otherwise. */
static int finish(void)
{
    if (failed_check == NULL) {
        puts("MADO_PROFILE_RESULT=passed");
        return 0;
    }
    printf("MADO_PROFILE_FAILURE=%s\n", failed_check);
    if (failed_status != MADOPILOT_STATUS_OK) {
        madopilot_str_t slug = {NULL, 0};
        if (api != NULL && api->status_text(failed_status, &slug) == MADOPILOT_STATUS_OK &&
            slug.data != NULL) {
            printf("MADO_PROFILE_STATUS=%.*s\n", (int)slug.len, slug.data);
        } else {
            printf("MADO_PROFILE_STATUS=%d\n", (int)failed_status);
        }
    }
    puts("MADO_PROFILE_RESULT=failed");
    return 1;
}

int main(int argc, char** argv)
{
    const char* package = NULL;
    const char* model_root = NULL;
    const char* runtime = NULL;
    madopilot_status_t status;
    int index;

    /* Every observation reaches the log even if the library aborts afterwards. */
    setvbuf(stdout, NULL, _IONBF, 0);

    for (index = 1; index + 1 < argc; index += 2) {
        if (strcmp(argv[index], "--package") == 0 && package == NULL) {
            package = argv[index + 1];
        } else if (strcmp(argv[index], "--model-root") == 0 && model_root == NULL) {
            model_root = argv[index + 1];
        } else if (strcmp(argv[index], "--runtime") == 0 && runtime == NULL) {
            runtime = argv[index + 1];
        } else {
            break;
        }
    }
    if (index != argc || package == NULL || model_root == NULL || runtime == NULL) {
        fputs("usage: --package <dir> --model-root <dir> --runtime <file>\n", stderr);
        fail("usage", MADOPILOT_STATUS_OK);
        return finish();
    }

    /* Negotiate the table; nothing else can be called until this succeeds. */
    status = madopilot_get_api(MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR, sizeof(madopilot_api_t),
                               &api);
    if (!require_ok(status, "negotiate") || !require(api != NULL, "negotiate")) {
        api = NULL;
        return finish();
    }
    if (!require(api->struct_size >= MADOPILOT_API_SIZE_ENGINE_OCR_PROVIDER_DESCRIPTOR,
                 "negotiate")) {
        return finish();
    }
    build.struct_size = (uint32_t)sizeof(build);
    if (!require_ok(api->describe_build(&build), "build")) {
        return finish();
    }
    puts("MADO_PROFILE_CONSUMER=c-abi");

    if (matching_flow(package, 0) && matching_flow(package, 1)) {
        ocr_flow(model_root, runtime);
    }
    return finish();
}
