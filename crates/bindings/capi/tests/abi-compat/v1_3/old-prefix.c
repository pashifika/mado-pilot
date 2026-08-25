/*
 * The frozen ABI 1.3 C header, compiled against whatever library exists now.
 *
 * This compatibility fixture includes only its own frozen
 * `madopilot/madopilot.h`, never the working header. It preserves the released
 * 648-byte declarations and executes the complete pre-1.3 common flow plus
 * every ABI 1.3 entry. The deterministic package has no OCR model, so the OCR
 * calls exercise their documented initialized refusal paths; the tracked
 * controlled examples separately exercise successful integrated OCR when the
 * reviewed model/runtime prerequisites are available.
 *
 * It is frozen with the header. Do not add later-minor behavior here.
 *
 *   usage: madopilot-abi-compat-v1_3 --package <dir>
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "deterministic-scene.h"
#include "madopilot/madopilot.h"

static int failures = 0;

static madopilot_str_t borrow(const char* text)
{
    madopilot_str_t view;
    view.data = text;
    view.len = strlen(text);
    return view;
}

static int expect(int condition, const char* what)
{
    if (!condition) {
        printf("FAIL: %s\n", what);
        failures += 1;
    }
    return condition;
}

static int expect_ok(madopilot_status_t status, const char* what)
{
    return expect(status == MADOPILOT_STATUS_OK, what);
}

/*
 * The old-prefix path proper.
 *
 * A caller that knows only the mandatory prefix must negotiate successfully and
 * must be told the library's own table size, which is how it discovers that
 * more entries exist than its header declared. A library that refused this, or
 * that reported the caller's size back, would have broken every older header at
 * once.
 */
static int negotiate_at_the_mandatory_prefix(void)
{
    const madopilot_api_t* api = NULL;
    const madopilot_status_t status = madopilot_get_api(
        MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR, MADOPILOT_API_SIZE_INFORMATION, &api);

    if (!expect_ok(status, "negotiating at the mandatory prefix")) {
        return 0;
    }
    if (!expect(api != NULL, "a successful negotiation returns a table")) {
        return 0;
    }
    expect(api->struct_size >= (uint32_t)MADOPILOT_API_SIZE_INFORMATION,
           "the library reports its own table size, not the caller's");
    expect(api->abi_major == MADOPILOT_ABI_MAJOR,
           "the library agrees with the frozen header's ABI major");
    expect(api->abi_minor >= MADOPILOT_ABI_MINOR,
           "the library is at least as new as the frozen header");

    /* Every entry inside the mandatory prefix is reachable at that size. */
    {
        madopilot_str_t text;
        text.data = NULL;
        text.len = 0;
        expect_ok(api->status_text(MADOPILOT_STATUS_OK, &text), "status_text");
        expect(text.len > 0, "a status has a stable slug");
    }

    return 1;
}

/* Refusals the frozen header's own constants describe. */
static void negotiation_refusals(void)
{
    const madopilot_api_t* api = (const madopilot_api_t*)(void*)&failures;
    madopilot_status_t status;

    status = madopilot_get_api(MADOPILOT_ABI_MAJOR + 1u, MADOPILOT_ABI_MINOR,
                               sizeof(madopilot_api_t), &api);
    expect(status == MADOPILOT_STATUS_UNSUPPORTED,
           "a different ABI major is a different library");
    expect(api == NULL, "a refused negotiation nulls its output");

    api = (const madopilot_api_t*)(void*)&failures;
    status = madopilot_get_api(MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR,
                               MADOPILOT_API_SIZE_INFORMATION - 1u, &api);
    expect(status == MADOPILOT_STATUS_INVALID_ARGUMENT,
           "a size below the mandatory prefix is refused");
    expect(api == NULL, "a refused negotiation nulls its output");
}

/*
 * The whole Phase 1 flow, through the frozen header's declarations.
 *
 * Compressed against the C example on purpose: the example is where the flow is
 * explained, and this is where it is proved to still run. The numbers checked
 * are the ones both examples print, so a library that changed an answer fails
 * here as well as there.
 */
static int run_the_flow(const madopilot_api_t* api, const char* package_dir)
{
    madopilot_operation_t operation;
    madopilot_replay_frame_t frame_input;
    madopilot_source_t source;
    madopilot_package_source_t package_source;
    madopilot_open_request_t open_request;
    madopilot_find_request_t find_request;
    madopilot_result_info_t result_info;
    madopilot_engine_t* engine = NULL;
    madopilot_target_list_t* targets = NULL;
    madopilot_session_t* session = NULL;
    madopilot_frame_t* frame = NULL;
    madopilot_package_t* package = NULL;
    madopilot_template_t* prepared = NULL;
    madopilot_result_t* result = NULL;
    madopilot_error_t* error = NULL;
    uint8_t* scene = NULL;
    uint64_t now = 0;
    size_t index = 0;
    int found[2];

    found[0] = 0;
    found[1] = 0;

    expect_ok(api->clock_now(&now), "clock_now");
    memset(&operation, 0, sizeof(operation));
    operation.struct_size = (uint32_t)sizeof(operation);
    operation.flags = MADOPILOT_OPERATION_HAS_DEADLINE;
    operation.deadline_nanos = now + 30ull * 1000ull * 1000ull * 1000ull;

    scene = (uint8_t*)malloc(SCENE_BYTES);
    if (scene == NULL) {
        fprintf(stderr, "could not allocate the scene\n");
        return 0;
    }
    scene_fill_rgba(scene);

    memset(&frame_input, 0, sizeof(frame_input));
    frame_input.struct_size = (uint32_t)sizeof(frame_input);
    frame_input.width = SCENE_WIDTH;
    frame_input.height = SCENE_HEIGHT;
    frame_input.format = MADOPILOT_PIXEL_FORMAT_RGBA8;
    frame_input.continuity = MADOPILOT_CONTINUITY_CONTINUOUS;
    frame_input.pixels.data = scene;
    frame_input.pixels.len = SCENE_BYTES;

    memset(&source, 0, sizeof(source));
    source.struct_size = (uint32_t)sizeof(source);
    source.kind = MADOPILOT_SOURCE_REPLAY_MEMORY;
    source.frames = &frame_input;
    source.frame_count = 1;
    source.frame_stride = sizeof(madopilot_replay_frame_t);
    source.target_name = borrow("panel");

    if (!expect_ok(api->engine_create(&source, &operation, &engine, &error),
                   "engine_create")) {
        api->error_release(error);
        free(scene);
        return 0;
    }
    free(scene);

    expect_ok(api->engine_discover(engine, &operation, &targets, &error),
              "engine_discover");

    memset(&open_request, 0, sizeof(open_request));
    open_request.struct_size = (uint32_t)sizeof(open_request);
    open_request.flags = MADOPILOT_OPEN_HAS_REQUIRED_FORMAT;
    open_request.required_format = MADOPILOT_PIXEL_FORMAT_RGBA8;
    expect_ok(api->session_open(engine, targets, 0, &open_request, &operation,
                                &session, &error),
              "session_open");
    api->target_list_release(targets);
    targets = NULL;

    expect_ok(api->session_acquire_frame(session, &operation, &frame, &error),
              "session_acquire_frame");

    memset(&package_source, 0, sizeof(package_source));
    package_source.struct_size = (uint32_t)sizeof(package_source);
    package_source.kind = MADOPILOT_PACKAGE_SOURCE_DIRECTORY;
    package_source.path = borrow(package_dir);
    expect_ok(api->package_load(engine, &package_source, &operation, &package,
                                &error),
              "package_load");

    expect_ok(api->template_prepare_from_package(engine, package,
                                                 borrow("panel.patch"),
                                                 &operation, &prepared, &error),
              "template_prepare_from_package");

    memset(&find_request, 0, sizeof(find_request));
    find_request.struct_size = (uint32_t)sizeof(find_request);
    find_request.frame = frame;
    find_request.tmpl = prepared;
    expect_ok(api->session_find(session, &find_request, &operation, &result,
                                &error),
              "session_find");

    memset(&result_info, 0, sizeof(result_info));
    result_info.struct_size = (uint32_t)sizeof(result_info);
    if (expect_ok(api->result_describe(result, &result_info), "result_describe")) {
        expect(result_info.match_count == 2,
               "the frozen header still sees both planted copies");
    }

    for (index = 0; index < result_info.match_count; index += 1) {
        madopilot_match_t match;
        size_t planted = 0;

        memset(&match, 0, sizeof(match));
        match.struct_size = (uint32_t)sizeof(match);
        if (!expect_ok(api->result_match(result, index, &match), "result_match")) {
            continue;
        }
        expect(match.score > 0.999 && match.score < 1.001,
               "a planted copy still correlates at one");

        /* Compared as a set: two byte-identical copies differ by far less than
         * the tolerance, so their order is the host's rounding, not an answer. */
        for (planted = 0; planted < 2; planted += 1) {
            if (match.bounds.left == (int32_t)SCENE_PLANTED[planted][0] &&
                match.bounds.top == (int32_t)SCENE_PLANTED[planted][1]) {
                found[planted] = 1;
            }
        }
    }
    expect(found[0] == 1 && found[1] == 1,
           "both planted offsets are still where the frozen header expects");

    {
        madopilot_ocr_result_t* ocr_result =
            (madopilot_ocr_result_t*)(void*)&failures;
        madopilot_engine_t* ocr_engine = (madopilot_engine_t*)(void*)&failures;
        madopilot_ocr_result_info_t ocr_info;
        madopilot_ocr_region_t ocr_region;
        madopilot_str_t ocr_text;

        error = NULL;
        expect(api->session_recognize(session, NULL, &operation, &ocr_result, &error) ==
                   MADOPILOT_STATUS_INVALID_ARGUMENT,
               "ABI 1.3 session_recognize refusal");
        expect(ocr_result == NULL, "session_recognize initializes its owner output");
        expect(error != NULL, "session_recognize returns a typed refusal");
        api->error_release(error);
        error = NULL;

        expect_ok(api->ocr_result_retain(NULL), "ABI 1.3 null OCR retain");
        expect_ok(api->ocr_result_release(NULL), "ABI 1.3 null OCR release");

        memset(&ocr_info, 0xff, sizeof(ocr_info));
        ocr_info.struct_size = (uint32_t)sizeof(ocr_info);
        expect(api->ocr_result_info(NULL, &ocr_info) ==
                   MADOPILOT_STATUS_INVALID_ARGUMENT,
               "ABI 1.3 null OCR info");
        expect(ocr_info.region_count == 0, "OCR info initializes its count");

        memset(&ocr_region, 0xff, sizeof(ocr_region));
        ocr_region.struct_size = (uint32_t)sizeof(ocr_region);
        expect(api->ocr_result_region_at(NULL, 0, &ocr_region) ==
                   MADOPILOT_STATUS_INVALID_ARGUMENT,
               "ABI 1.3 null OCR region");
        expect(ocr_region.confidence == 0.0, "OCR region initializes confidence");

        ocr_text = borrow("sentinel");
        expect(api->ocr_result_text_at(NULL, 0, &ocr_text) ==
                   MADOPILOT_STATUS_INVALID_ARGUMENT,
               "ABI 1.3 null OCR text");
        expect(ocr_text.data == NULL && ocr_text.len == 0,
               "OCR text initializes its borrowed view");

        expect(api->engine_create_with_default_ocr(
                   &source, NULL, NULL, &operation, &ocr_engine, &error) ==
                   MADOPILOT_STATUS_INVALID_ARGUMENT,
               "ABI 1.3 default OCR constructor refusal");
        expect(ocr_engine == NULL, "default OCR initializes its owner output");
        expect(error != NULL, "default OCR returns a typed refusal");
        api->error_release(error);
        error = NULL;
    }

    expect_ok(api->session_close(session, &operation, &error), "session_close");

    api->result_release(result);
    api->template_release(prepared);
    api->package_release(package);
    api->frame_release(frame);
    api->session_release(session);
    api->engine_release(engine);

    return 1;
}

int main(int argc, char** argv)
{
    const madopilot_api_t* api = NULL;
    const char* package_dir = NULL;
    int index = 1;

    for (; index + 1 < argc; index += 2) {
        if (strcmp(argv[index], "--package") == 0) {
            package_dir = argv[index + 1];
        }
    }
    if (package_dir == NULL) {
        fprintf(stderr, "usage: %s --package <dir>\n", argv[0]);
        return 2;
    }

    printf("frozen header: abi %u.%u, table %u bytes as v1_3 declared it\n",
           (unsigned)MADOPILOT_ABI_MAJOR, (unsigned)MADOPILOT_ABI_MINOR,
           (unsigned)sizeof(madopilot_api_t));

    if (!negotiate_at_the_mandatory_prefix()) {
        return 1;
    }
    negotiation_refusals();

    if (!expect_ok(madopilot_get_api(MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR,
                                     sizeof(madopilot_api_t), &api),
                   "negotiating at the frozen header's full size")) {
        return 1;
    }
    printf("library table: %u bytes, abi %u.%u\n", (unsigned)api->struct_size,
           (unsigned)api->abi_major, (unsigned)api->abi_minor);

    if (!run_the_flow(api, package_dir)) {
        return 1;
    }

    if (failures != 0) {
        printf("madopilot-abi-compat-v1_3 failed %d check(s)\n", failures);
        return 1;
    }

    printf("madopilot-abi-compat-v1_3 complete\n");
    return 0;
}
