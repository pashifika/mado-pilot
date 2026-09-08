/* Pull-only template watch over repository-owned replay pixels.
 * Usage: template-watch-c --package <fixtures/assets/phase1-slice>
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "deterministic-scene.h"
#include "madopilot/madopilot.h"

static int failures = 0;

static int check(madopilot_status_t status, const char* operation)
{
    if (status == MADOPILOT_STATUS_OK) return 1;
    fprintf(stderr, "%s: status %d\n", operation, (int)status);
    ++failures;
    return 0;
}

static int expect(int condition, const char* contract)
{
    if (condition) return 1;
    fprintf(stderr, "contract failed: %s\n", contract);
    ++failures;
    return 0;
}

static madopilot_str_t borrow(const char* value)
{
    madopilot_str_t result = { value, strlen(value) };
    return result;
}

static int bounded(const madopilot_api_t* api, madopilot_operation_t* operation)
{
    uint64_t now = 0;
    memset(operation, 0, sizeof(*operation));
    operation->struct_size = (uint32_t)sizeof(*operation);
    if (!check(api->clock_now(&now), "clock_now")) return 0;
    if (!expect(now <= UINT64_MAX - UINT64_C(10000000000), "representable deadline")) return 0;
    operation->flags = MADOPILOT_OPERATION_HAS_DEADLINE;
    operation->deadline_nanos = now + UINT64_C(10000000000);
    return 1;
}

static int same_stamp(const madopilot_frame_stamp_t* left,
                      const madopilot_frame_stamp_t* right)
{
    return left->stream == right->stream && left->epoch == right->epoch &&
           left->sequence == right->sequence && left->geometry == right->geometry;
}

#define CALL(expression) do { if (!check((expression), #expression)) goto cleanup; } while (0)
#define REQUIRE(condition, contract) do { if (!expect((condition), (contract))) goto cleanup; } while (0)

int main(int argc, char** argv)
{
    const madopilot_api_t* api = NULL;
    madopilot_engine_t* engine = NULL;
    madopilot_target_list_t* targets = NULL;
    madopilot_session_t* session = NULL;
    madopilot_package_t* package = NULL;
    madopilot_template_t* tmpl = NULL;
    madopilot_template_query_t* query = NULL;
    madopilot_template_query_result_t* result = NULL;
    madopilot_template_query_result_t* repeated = NULL;
    madopilot_frame_t* frame = NULL;
    madopilot_mapping_t* mapping = NULL;
    madopilot_error_t* failure = NULL;
    uint8_t pixels[SCENE_BYTES];
    madopilot_operation_t query_operation, wait_operation, close_operation;
    madopilot_replay_frame_t replay;
    madopilot_source_t source;
    madopilot_open_request_t open;
    madopilot_package_source_t package_source;
    madopilot_template_watch_options_t options;
    madopilot_template_query_snapshot_t snapshot;
    madopilot_template_query_result_info_t info;
    madopilot_template_scheduler_descriptor_t scheduler;
    madopilot_frame_stamp_t frame_stamp, mapping_stamp;
    madopilot_map_request_t map;
    madopilot_image_t image;
    size_t index;
    unsigned planted = 0;

    if (argc != 3 || strcmp(argv[1], "--package") != 0) {
        fprintf(stderr, "usage: %s --package <dir>\n", argv[0]);
        return 2;
    }
    if (!check(madopilot_get_api(MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR,
                                sizeof(madopilot_api_t), &api), "madopilot_get_api")) return 1;
    if (!expect(api != NULL && api->struct_size >= MADOPILOT_API_SIZE_TEMPLATE_QUERY_REQUIRED &&
                sizeof(madopilot_api_t) >= MADOPILOT_API_SIZE_TEMPLATE_QUERY_REQUIRED,
                "complete watcher ownership table")) return 1;
    if (!expect(api->clock_now && api->engine_create && api->engine_discover &&
                api->engine_release && api->target_list_release && api->session_open &&
                api->session_close && api->session_release && api->package_load &&
                api->package_release && api->template_prepare_from_package && api->template_release &&
                api->engine_template_scheduler_descriptor && api->session_start_template_watch &&
                api->template_query_retain && api->template_query_release && api->template_query_poll &&
                api->template_query_wait && api->template_query_cancel &&
                api->template_query_result_retain && api->template_query_result_release &&
                api->template_query_result_info && api->template_query_result_match_at &&
                api->template_query_result_frame && api->template_query_result_error &&
                api->frame_stamp && api->frame_map && api->frame_release &&
                api->mapping_stamp && api->mapping_describe && api->mapping_release && api->error_release,
                "required callable entries")) return 1;
    if (!bounded(api, &query_operation)) return 1;

    scene_fill_rgba(pixels);
    memset(&replay, 0, sizeof(replay));
    replay.struct_size = (uint32_t)sizeof(replay);
    replay.width = SCENE_WIDTH;
    replay.height = SCENE_HEIGHT;
    replay.format = MADOPILOT_PIXEL_FORMAT_RGBA8;
    replay.continuity = MADOPILOT_CONTINUITY_CONTINUOUS;
    replay.pixels.data = pixels;
    replay.pixels.len = sizeof(pixels);
    memset(&source, 0, sizeof(source));
    source.struct_size = (uint32_t)sizeof(source);
    source.kind = MADOPILOT_SOURCE_REPLAY_MEMORY;
    source.frames = &replay;
    source.frame_count = 1;
    source.frame_stride = sizeof(replay);
    source.target_name = borrow("template-watch-replay");
    CALL(api->engine_create(&source, &query_operation, &engine, NULL));
    memset(&scheduler, 0, sizeof(scheduler));
    scheduler.struct_size = (uint32_t)sizeof(scheduler);
    CALL(api->engine_template_scheduler_descriptor(engine, &scheduler));
    REQUIRE(scheduler.latest_pending_frames_per_query == 1, "finite latest-wins scheduler");
    CALL(api->engine_discover(engine, &query_operation, &targets, NULL));
    memset(&open, 0, sizeof(open));
    open.struct_size = (uint32_t)sizeof(open);
    CALL(api->session_open(engine, targets, 0, &open, &query_operation, &session, NULL));
    memset(&package_source, 0, sizeof(package_source));
    package_source.struct_size = (uint32_t)sizeof(package_source);
    package_source.kind = MADOPILOT_PACKAGE_SOURCE_DIRECTORY;
    package_source.path = borrow(argv[2]);
    CALL(api->package_load(engine, &package_source, &query_operation, &package, NULL));
    CALL(api->template_prepare_from_package(engine, package, borrow("panel.patch"),
                                            &query_operation, &tmpl, NULL));
    memset(&options, 0, sizeof(options));
    options.struct_size = (uint32_t)sizeof(options);
    options.stability_kind = MADOPILOT_TEMPLATE_STABILITY_IMMEDIATE;
    options.change_policy = MADOPILOT_TEMPLATE_CHANGE_EXACT_RGBA;
    CALL(api->session_start_template_watch(session, tmpl, &options, &query_operation, &query, NULL));
    CALL(api->template_release(tmpl));
    tmpl = NULL;
    CALL(api->package_release(package));
    package = NULL;

    memset(&snapshot, 0, sizeof(snapshot));
    snapshot.struct_size = (uint32_t)sizeof(snapshot);
    CALL(api->template_query_poll(query, &snapshot, &result, NULL));
    if (snapshot.state == MADOPILOT_TEMPLATE_QUERY_STATE_PENDING) {
        REQUIRE(result == NULL, "pending poll owns no terminal result");
        if (!bounded(api, &wait_operation)) goto cleanup;
        CALL(api->template_query_wait(query, &wait_operation, &result, NULL));
    } else {
        REQUIRE(snapshot.state == MADOPILOT_TEMPLATE_QUERY_STATE_TERMINAL && result != NULL,
                "terminal poll owns its result");
    }
    memset(&info, 0, sizeof(info));
    info.struct_size = (uint32_t)sizeof(info);
    CALL(api->template_query_result_info(result, &info));
    if (info.outcome != MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED) {
        CALL(api->template_query_result_error(result, &failure));
        fprintf(stderr, "query terminal: %d, status %d\n", (int)info.outcome, (int)info.status);
        ++failures;
        goto cleanup;
    }
    REQUIRE(info.match_count == 2 && info.confirmed_observations == 1, "confirmed planted matches");
    REQUIRE(info.transform.geometry == info.source.geometry &&
            info.transform.width == SCENE_WIDTH && info.transform.height == SCENE_HEIGHT,
            "exact retained transform");
    for (index = 0; index < 2; ++index) {
        madopilot_match_t found;
        memset(&found, 0, sizeof(found));
        found.struct_size = (uint32_t)sizeof(found);
        CALL(api->template_query_result_match_at(result, index, &found));
        unsigned location;
        REQUIRE(found.bounds.right - found.bounds.left == (int32_t)PATCH_WIDTH &&
                found.bounds.bottom - found.bounds.top == (int32_t)PATCH_HEIGHT &&
                found.score >= 1.0 - 1e-5 && found.score <= 1.0 + 1e-5,
                "match extent and score");
        for (location = 0; location < 2; ++location) {
            if (found.bounds.left == (int32_t)SCENE_PLANTED[location][0] &&
                found.bounds.top == (int32_t)SCENE_PLANTED[location][1]) planted |= 1u << location;
        }
    }
    REQUIRE(planted == 3u, "both planted locations, independent of tie ordering");
    CALL(api->template_query_cancel(query, &repeated, NULL));
    {
        madopilot_template_query_result_info_t winner;
        memset(&winner, 0, sizeof(winner));
        winner.struct_size = (uint32_t)sizeof(winner);
        CALL(api->template_query_result_info(repeated, &winner));
        REQUIRE(winner.outcome == info.outcome && winner.query_id == info.query_id &&
                same_stamp(&winner.source, &info.source) && winner.match_count == info.match_count,
                "cancel preserves the terminal winner");
    }
    CALL(api->template_query_result_release(repeated));
    repeated = NULL;
    CALL(api->template_query_release(query));
    query = NULL;
    if (!bounded(api, &close_operation)) goto cleanup;
    CALL(api->session_close(session, &close_operation, NULL));
    CALL(api->session_release(session));
    session = NULL;
    CALL(api->target_list_release(targets));
    targets = NULL;
    CALL(api->engine_release(engine));
    engine = NULL;

    CALL(api->template_query_result_frame(result, &frame));
    memset(&frame_stamp, 0, sizeof(frame_stamp));
    frame_stamp.struct_size = (uint32_t)sizeof(frame_stamp);
    CALL(api->frame_stamp(frame, &frame_stamp));
    REQUIRE(same_stamp(&info.source, &frame_stamp), "result/frame source correlation after teardown");
    memset(&map, 0, sizeof(map));
    map.struct_size = (uint32_t)sizeof(map);
    map.format = MADOPILOT_PIXEL_FORMAT_RGBA8;
    if (!bounded(api, &wait_operation)) goto cleanup;
    CALL(api->frame_map(frame, &map, &wait_operation, &mapping, NULL));
    CALL(api->frame_release(frame));
    frame = NULL;
    CALL(api->template_query_result_release(result));
    result = NULL;
    memset(&mapping_stamp, 0, sizeof(mapping_stamp));
    mapping_stamp.struct_size = (uint32_t)sizeof(mapping_stamp);
    CALL(api->mapping_stamp(mapping, &mapping_stamp));
    REQUIRE(same_stamp(&frame_stamp, &mapping_stamp), "mapping remains source-correlated");
    memset(&image, 0, sizeof(image));
    image.struct_size = (uint32_t)sizeof(image);
    CALL(api->mapping_describe(mapping, &image));
    REQUIRE(image.bytes.len == sizeof(pixels) && memcmp(image.bytes.data, pixels, sizeof(pixels)) == 0,
            "retained mapping pixels survive every parent");

cleanup:
    check(api->error_release(failure), "error_release");
    check(api->mapping_release(mapping), "mapping_release");
    check(api->frame_release(frame), "frame_release");
    check(api->template_query_result_release(repeated), "template_query_result_release");
    check(api->template_query_result_release(result), "template_query_result_release");
    check(api->template_query_release(query), "template_query_release");
    check(api->template_release(tmpl), "template_release");
    check(api->package_release(package), "package_release");
    if (session != NULL && bounded(api, &close_operation)) check(api->session_close(session, &close_operation, NULL), "session_close");
    check(api->session_release(session), "session_release");
    check(api->target_list_release(targets), "target_list_release");
    check(api->engine_release(engine), "engine_release");
    if (failures != 0) return 1;
    puts("madopilot-c-template-watch complete");
    return 0;
}