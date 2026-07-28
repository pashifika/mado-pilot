/*
 * The complete deterministic Phase 1 flow, in C.
 *
 * Negotiate the table, build an absolute deadline and a cancellation handle,
 * supply a deterministic replay source, discover its target, open a session,
 * take a frame and map it, load a tracked asset package, prepare two templates,
 * search that exact frame for both, read the source-correlated results, ask for
 * a template the package does not declare and read the structured error, close,
 * and release every handle in reverse ownership order.
 *
 * This is the C counterpart of
 * crates/mado-pilot/examples/deterministic-slice.rs and answers the same
 * questions with the same numbers. Run the Rust one first if you want to see
 * what the output should say.
 *
 * The program checks every status and every expected outcome and exits non-zero
 * on the first surprise, so it is a test as well as an example. In particular
 * the scene comes from `../deterministic-scene.h`, which is the same integer
 * arithmetic `mado-pilot-testkit` uses: if the two ever drift apart, the
 * template stops being found where it is planted and this program fails. The
 * C++ example includes that same header.
 *
 *   usage: deterministic-slice --package <dir> [--label <text>]
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "deterministic-scene.h"
#include "madopilot/madopilot.h"

/* --------------------------------------------------------------------------
 * Small helpers
 * ----------------------------------------------------------------------- */

static const madopilot_api_t* api = NULL;
static int failures = 0;

static madopilot_str_t borrow(const char* text)
{
    madopilot_str_t view;
    view.data = text;
    view.len = strlen(text);
    return view;
}

static void print_str(const char* label, madopilot_str_t view)
{
    printf("%s%.*s", label, (int)view.len, view.data == NULL ? "" : view.data);
}

static const char* status_name(madopilot_status_t status)
{
    madopilot_str_t text;
    static char buffer[64];
    text.data = NULL;
    text.len = 0;
    if (api != NULL && api->status_text(status, &text) == MADOPILOT_STATUS_OK &&
        text.len < sizeof(buffer)) {
        memcpy(buffer, text.data, text.len);
        buffer[text.len] = '\0';
        return buffer;
    }
    return "unavailable";
}

/* Reports a failed expectation and keeps going, so one run shows every
 * problem rather than only the first. */
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
    if (status != MADOPILOT_STATUS_OK) {
        printf("FAIL: %s returned %s (%d)\n", what, status_name(status), (int)status);
        failures += 1;
        return 0;
    }
    return 1;
}

/* Prints an owned error and releases it. The message is borrowed from the
 * handle, so anything needed afterwards is copied before the release. */
static void report_error(const char* what, madopilot_error_t* error)
{
    madopilot_error_detail_t detail;

    if (error == NULL) {
        printf("  %s: no error detail was produced\n", what);
        return;
    }

    memset(&detail, 0, sizeof(detail));
    detail.struct_size = (uint32_t)sizeof(detail);
    if (api->error_describe(error, &detail) == MADOPILOT_STATUS_OK) {
        printf("  %s: status %s category %d", what, status_name(detail.status),
               (int)detail.category);
        if ((detail.flags & MADOPILOT_ERROR_HAS_ASSET_DETAIL) != 0u) {
            printf(" asset_fault %d stage %d", (int)detail.asset_fault,
                   (int)detail.asset_stage);
        }
        if ((detail.flags & MADOPILOT_ERROR_HAS_BACKEND) != 0u) {
            print_str(" backend ", detail.backend);
        }
        print_str("\n    ", detail.message);
        printf("\n");
    }
    api->error_release(error);
}

int main(int argc, char** argv)
{
    const char* package_path = NULL;
    const char* label = "unlabelled host";
    int index;

    madopilot_status_t status;
    madopilot_build_info_t build;
    madopilot_cancellation_t* cancellation = NULL;
    madopilot_operation_t operation;
    uint64_t now = 0;

    uint8_t* scene = NULL;
    madopilot_replay_frame_t frame_input;
    madopilot_source_t source;

    madopilot_engine_t* engine = NULL;
    madopilot_target_list_t* targets = NULL;
    madopilot_session_t* session = NULL;
    madopilot_frame_t* frame = NULL;
    madopilot_mapping_t* mapping = NULL;
    madopilot_package_t* package = NULL;
    madopilot_template_t* present = NULL;
    madopilot_template_t* absent = NULL;
    madopilot_result_t* found = NULL;
    madopilot_result_t* missing = NULL;
    madopilot_error_t* error = NULL;

    for (index = 1; index < argc; ++index) {
        if (strcmp(argv[index], "--package") == 0 && index + 1 < argc) {
            package_path = argv[++index];
        } else if (strcmp(argv[index], "--label") == 0 && index + 1 < argc) {
            label = argv[++index];
        } else {
            fprintf(stderr, "usage: %s --package <dir> [--label <text>]\n", argv[0]);
            return 2;
        }
    }
    if (package_path == NULL) {
        fprintf(stderr, "usage: %s --package <dir> [--label <text>]\n", argv[0]);
        return 2;
    }

    /* 1. Negotiate the table. Nothing else can be called until this succeeds. */
    status = madopilot_get_api(MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR,
                               sizeof(madopilot_api_t), &api);
    if (status != MADOPILOT_STATUS_OK || api == NULL) {
        fprintf(stderr, "madopilot_get_api failed with %d\n", (int)status);
        return 1;
    }
    printf("host: %s\n", label);

    memset(&build, 0, sizeof(build));
    build.struct_size = (uint32_t)sizeof(build);
    if (!expect_ok(api->describe_build(&build), "describe_build")) {
        return 1;
    }
    printf("abi: %u.%u table %u bytes (header declares %zu)\n", build.abi_major,
           build.abi_minor, build.table_size, sizeof(madopilot_api_t));
    print_str("library: ", build.library_version);
    print_str(" backend ", build.required_backend);
    printf("\n");

    /* 2. Build an absolute deadline and a cancellation handle. The deadline is
     *    an instant in the library's own monotonic domain, not a duration. */
    if (!expect_ok(api->clock_now(&now), "clock_now")) {
        return 1;
    }
    if (!expect_ok(api->cancellation_create(&cancellation), "cancellation_create")) {
        return 1;
    }

    memset(&operation, 0, sizeof(operation));
    operation.struct_size = (uint32_t)sizeof(operation);
    operation.flags = MADOPILOT_OPERATION_HAS_DEADLINE;
    operation.deadline_nanos = now + 30ull * 1000ull * 1000ull * 1000ull;
    operation.cancellation = cancellation;

    /* 3. Supply the deterministic scene as a memory replay source. */
    scene = (uint8_t*)malloc(SCENE_BYTES);
    if (scene == NULL) {
        fprintf(stderr, "could not allocate the scene\n");
        return 1;
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

    status = api->engine_create(&source, &operation, &engine, &error);
    if (!expect_ok(status, "engine_create")) {
        report_error("engine_create", error);
        free(scene);
        return 1;
    }
    /* The pixels were copied, so the caller's storage is its own again. */
    free(scene);
    scene = NULL;

    /* 4. Discover, and open the one target. */
    if (expect_ok(api->engine_discover(engine, &operation, &targets, &error),
                  "engine_discover")) {
        size_t count = 0;
        madopilot_target_t target;

        expect_ok(api->target_list_count(targets, &count), "target_list_count");
        expect(count == 1, "the replay source declares exactly one target");

        memset(&target, 0, sizeof(target));
        target.struct_size = (uint32_t)sizeof(target);
        if (expect_ok(api->target_list_get(targets, 0, &target), "target_list_get")) {
            print_str("target: ", target.name);
            print_str(" from ", target.provider);
            printf(" %ux%u format %d\n", target.width, target.height,
                   (int)target.format);
        }

        /* Out of range is invalid argument, and leaves the output alone. */
        memset(&target, 0, sizeof(target));
        target.struct_size = (uint32_t)sizeof(target);
        expect(api->target_list_get(targets, count, &target) ==
                   MADOPILOT_STATUS_INVALID_ARGUMENT,
               "an out-of-range target index is invalid argument");
        expect(target.width == 0 && target.name.len == 0,
               "a rejected accessor leaves its output in the failure state");
    } else {
        report_error("engine_discover", error);
    }

    {
        madopilot_open_request_t open_request;
        madopilot_session_info_t session_info;

        memset(&open_request, 0, sizeof(open_request));
        open_request.struct_size = (uint32_t)sizeof(open_request);
        open_request.flags = MADOPILOT_OPEN_HAS_REQUIRED_FORMAT;
        open_request.required_format = MADOPILOT_PIXEL_FORMAT_RGBA8;

        status = api->session_open(engine, targets, 0, &open_request, &operation,
                                   &session, &error);
        if (!expect_ok(status, "session_open")) {
            report_error("session_open", error);
            goto cleanup;
        }

        /* Opening copied the identity, so the list is no longer needed. Every
         * borrowed target string dies with it; nothing below uses one. */
        api->target_list_release(targets);
        targets = NULL;

        memset(&session_info, 0, sizeof(session_info));
        session_info.struct_size = (uint32_t)sizeof(session_info);
        if (expect_ok(api->session_describe(session, &session_info),
                      "session_describe")) {
            printf("session: stream %llu %ux%u\n",
                   (unsigned long long)session_info.stream, session_info.width,
                   session_info.height);
        }
    }

    /* 5. Take one frame and hold it. Everything below searches this exact
     *    frame, not whatever the session publishes later. */
    {
        madopilot_frame_stamp_t stamp;
        madopilot_frame_info_t frame_info;

        status = api->session_frame(session, &operation, &frame, &error);
        if (!expect_ok(status, "session_frame")) {
            report_error("session_frame", error);
            goto cleanup;
        }

        memset(&stamp, 0, sizeof(stamp));
        stamp.struct_size = (uint32_t)sizeof(stamp);
        expect_ok(api->frame_stamp(frame, &stamp), "frame_stamp");
        printf("frame: stream %llu epoch %llu sequence %llu geometry %llu\n",
               (unsigned long long)stamp.stream, (unsigned long long)stamp.epoch,
               (unsigned long long)stamp.sequence,
               (unsigned long long)stamp.geometry);
        expect(stamp.epoch == 0 && stamp.sequence == 0 && stamp.geometry == 0,
               "a static image publishes epoch 0, sequence 0, geometry 0");

        memset(&frame_info, 0, sizeof(frame_info));
        frame_info.struct_size = (uint32_t)sizeof(frame_info);
        if (expect_ok(api->frame_describe(frame, &frame_info), "frame_describe")) {
            printf("frame geometry: %ux%u stride %llu bounds [%d, %d) x [%d, %d)\n",
                   frame_info.width, frame_info.height,
                   (unsigned long long)frame_info.stride, frame_info.bounds.left,
                   frame_info.bounds.right, frame_info.bounds.top,
                   frame_info.bounds.bottom);
        }
    }

    /* 6. Map it. The mapped bytes stay readable after the session is gone. */
    {
        madopilot_map_request_t map_request;
        madopilot_image_t image;

        memset(&map_request, 0, sizeof(map_request));
        map_request.struct_size = (uint32_t)sizeof(map_request);
        map_request.format = MADOPILOT_PIXEL_FORMAT_RGBA8;

        status = api->frame_map(frame, &map_request, &operation, &mapping, &error);
        if (!expect_ok(status, "frame_map")) {
            report_error("frame_map", error);
            goto cleanup;
        }

        memset(&image, 0, sizeof(image));
        image.struct_size = (uint32_t)sizeof(image);
        if (expect_ok(api->mapping_describe(mapping, &image), "mapping_describe")) {
            printf("mapped: %ux%u %zu bytes shared %d\n", image.width, image.height,
                   image.bytes.len,
                   (image.flags & MADOPILOT_IMAGE_SHARED) != 0u ? 1 : 0);
            expect(image.bytes.len == SCENE_BYTES,
                   "the whole frame maps to width * height * 4 bytes");
        }
    }

    /* 7. Load the tracked asset package and prepare its templates. */
    {
        madopilot_package_source_t package_source;
        madopilot_package_info_t package_info;
        size_t at;

        memset(&package_source, 0, sizeof(package_source));
        package_source.struct_size = (uint32_t)sizeof(package_source);
        package_source.kind = MADOPILOT_PACKAGE_SOURCE_DIRECTORY;
        package_source.path = borrow(package_path);

        status = api->package_load(engine, &package_source, &operation, &package,
                                   &error);
        if (!expect_ok(status, "package_load")) {
            report_error("package_load", error);
            goto cleanup;
        }

        memset(&package_info, 0, sizeof(package_info));
        package_info.struct_size = (uint32_t)sizeof(package_info);
        if (expect_ok(api->package_describe(package, &package_info),
                      "package_describe")) {
            print_str("package: ", package_info.package_id);
            print_str(" ", package_info.package_version);
            print_str(" under ", package_info.license);
            printf(", %llu templates\n",
                   (unsigned long long)package_info.template_count);
        }
        for (at = 0; at < package_info.template_count; ++at) {
            madopilot_str_t id;
            id.data = NULL;
            id.len = 0;
            if (expect_ok(api->package_template_id(package, at, &id),
                          "package_template_id")) {
                print_str("  declares ", id);
                printf("\n");
            }
        }

        status = api->template_prepare(engine, package, borrow("panel.patch"),
                                       &operation, &present, &error);
        if (!expect_ok(status, "template_prepare(panel.patch)")) {
            report_error("template_prepare", error);
            goto cleanup;
        }
        status = api->template_prepare(engine, package, borrow("panel.absent"),
                                       &operation, &absent, &error);
        if (!expect_ok(status, "template_prepare(panel.absent)")) {
            report_error("template_prepare", error);
            goto cleanup;
        }

        {
            madopilot_template_info_t template_info;
            memset(&template_info, 0, sizeof(template_info));
            template_info.struct_size = (uint32_t)sizeof(template_info);
            if (expect_ok(api->template_describe(present, &template_info),
                          "template_describe")) {
                print_str("template: ", template_info.id);
                printf(" %ux%u min_score %.2f max_results %u\n",
                       template_info.width, template_info.height,
                       template_info.min_score, template_info.max_results);
            }
        }

        /* A package that loaded is valid. Asking it for an identity it never
         * declared is the caller's mistake, so it is invalid argument rather
         * than an asset failure — and the error still says which package rule
         * was involved. */
        {
            madopilot_template_t* nothing = NULL;
            madopilot_error_t* refusal = NULL;
            status = api->template_prepare(engine, package, borrow("panel.absent.typo"),
                                           &operation, &nothing, &refusal);
            expect(status == MADOPILOT_STATUS_INVALID_ARGUMENT,
                   "an undeclared template identity is invalid argument");
            expect(nothing == NULL, "a refused preparation leaves its output null");
            printf("undeclared template:\n");
            report_error("template_prepare", refusal);
        }
    }

    /* 8. Search that exact frame. Two searches, two different answers. */
    {
        madopilot_find_request_t find;
        madopilot_result_info_t info;
        madopilot_frame_stamp_t result_stamp;
        madopilot_match_options_t effective;
        size_t at;

        memset(&find, 0, sizeof(find));
        find.struct_size = (uint32_t)sizeof(find);
        find.frame = frame;
        find.tmpl = present;

        status = api->session_find(session, &find, &operation, &found, &error);
        if (!expect_ok(status, "session_find(panel.patch)")) {
            report_error("session_find", error);
            goto cleanup;
        }

        memset(&info, 0, sizeof(info));
        info.struct_size = (uint32_t)sizeof(info);
        expect_ok(api->result_describe(found, &info), "result_describe");
        print_str("found by ", info.backend_id);
        print_str(" ", info.backend_version);
        printf(": %llu match(es) in [%d, %d) x [%d, %d)\n",
               (unsigned long long)info.match_count, info.searched.left,
               info.searched.right, info.searched.top, info.searched.bottom);
        expect(info.match_count == 2,
               "the patch is planted at two offsets in the scene");

        memset(&effective, 0, sizeof(effective));
        effective.struct_size = (uint32_t)sizeof(effective);
        if (expect_ok(api->result_options(found, &effective), "result_options")) {
            printf("  ran with min_score %.2f max_results %u suppression %d\n",
                   effective.min_score, effective.max_results,
                   (int)effective.suppression);
        }

        for (at = 0; at < info.match_count; ++at) {
            madopilot_match_t match;
            memset(&match, 0, sizeof(match));
            match.struct_size = (uint32_t)sizeof(match);
            if (expect_ok(api->result_match(found, at, &match), "result_match")) {
                print_str("  ", match.template_id);
                printf(" at [%d, %d) x [%d, %d) score %.6f\n", match.bounds.left,
                       match.bounds.right, match.bounds.top, match.bounds.bottom,
                       match.score);
                expect(match.bounds.left == (int32_t)SCENE_PLANTED[0][0] ||
                           match.bounds.left == (int32_t)SCENE_PLANTED[1][0],
                       "a match sits where the fixture plants the patch");
            }
        }

        {
            madopilot_match_t match;
            memset(&match, 0, sizeof(match));
            match.struct_size = (uint32_t)sizeof(match);
            expect(api->result_match(found, info.match_count, &match) ==
                       MADOPILOT_STATUS_INVALID_ARGUMENT,
                   "an out-of-range match index is invalid argument");
            expect(match.score == 0.0 && match.template_id.len == 0,
                   "a rejected accessor leaves its output in the failure state");
        }

        memset(&result_stamp, 0, sizeof(result_stamp));
        result_stamp.struct_size = (uint32_t)sizeof(result_stamp);
        expect_ok(api->result_stamp(found, &result_stamp), "result_stamp");

        /* Nothing found is a successful answer to a well-formed question. */
        find.tmpl = absent;
        status = api->session_find(session, &find, &operation, &missing, &error);
        if (!expect_ok(status, "session_find(panel.absent)")) {
            report_error("session_find", error);
            goto cleanup;
        }
        memset(&info, 0, sizeof(info));
        info.struct_size = (uint32_t)sizeof(info);
        expect_ok(api->result_describe(missing, &info), "result_describe");
        printf("absent template: %llu match(es), which is a successful answer\n",
               (unsigned long long)info.match_count);
        expect(info.match_count == 0, "the absent template is not on this frame");
    }

    /* 9. Close. Twice, because close is idempotent. */
    expect_ok(api->session_close(session, &operation, &error), "session_close");
    expect_ok(api->session_close(session, &operation, &error), "session_close again");
    {
        int32_t closed = 0;
        expect_ok(api->session_is_closed(session, &closed), "session_is_closed");
        expect(closed != 0, "the session reports itself closed");
    }
    {
        madopilot_frame_t* after = NULL;
        madopilot_error_t* refusal = NULL;
        status = api->session_frame(session, &operation, &after, &refusal);
        expect(status == MADOPILOT_STATUS_CLOSED,
               "a closed session publishes nothing further");
        expect(after == NULL, "a refused frame request leaves its output null");
        api->error_release(refusal);
    }

    /* 10. What the caller owns survives the close, and survives the producer. */
    {
        madopilot_image_t image;
        madopilot_frame_stamp_t stamp;

        memset(&image, 0, sizeof(image));
        image.struct_size = (uint32_t)sizeof(image);
        expect_ok(api->mapping_describe(mapping, &image), "mapping_describe after close");
        printf("mapping still readable after close: %zu bytes\n", image.bytes.len);

        memset(&stamp, 0, sizeof(stamp));
        stamp.struct_size = (uint32_t)sizeof(stamp);
        expect_ok(api->result_stamp(found, &stamp), "result_stamp after close");
        printf("result still correlated after close: sequence %llu\n",
               (unsigned long long)stamp.sequence);
    }

cleanup:
    /* Reverse ownership order. Every release accepts null, so this path is the
     * same whether the flow completed or stopped early. */
    api->result_release(missing);
    api->result_release(found);
    api->template_release(absent);
    api->template_release(present);
    api->package_release(package);
    api->mapping_release(mapping);
    api->frame_release(frame);
    api->session_release(session);
    api->target_list_release(targets);
    api->engine_release(engine);
    api->cancellation_release(cancellation);
    free(scene);

    if (failures != 0) {
        printf("%d expectation(s) failed\n", failures);
        return 1;
    }
    printf("deterministic slice complete\n");

    return 0;
}
