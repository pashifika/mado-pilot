/* Independent public C native qualification consumer. The owned-fixture
 * controller drives stdin/stdout; this program never injects input or focus.
 * Usage: --check | --native --title <exact owned fixture title>
 */
#include <locale.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "madopilot/madopilot.h"
#include "../common/native-watch-protocol.h"

#define OPERATION_NANOS UINT64_C(5000000000)
#define OBSERVATION_NANOS UINT64_C(25000000)
#define INIT(value) do { memset(&(value), 0, sizeof(value)); (value).struct_size = (uint32_t)sizeof(value); } while (0)
#define TRY(expression) do { if (!(expression)) goto cleanup; } while (0)

static const madopilot_api_t *api;
static const char *row = "F1";
static const char *fault = "contract_mismatch";
static madopilot_status_t last_status = MADOPILOT_STATUS_OK;
static madopilot_status_t cleanup_status = MADOPILOT_STATUS_OK;
static int cleanup_failed;
static int check_only;

/* All handles that can retain an engine are gone before final engine_release.
 * Query/result/frame/mapping owners are deliberately separate from this scope. */
typedef struct native_scope {
    madopilot_engine_t *engine;
    madopilot_session_t *session;
    uint64_t target, stream, opened_nanos;
} native_scope;

typedef struct geometry_authority {
    madopilot_frame_stamp_t stamp;
    madopilot_transform_snapshot_t transform;
    mpw_shape shape;
    uint64_t observed_nanos;
} geometry_authority;

typedef struct retained_match {
    madopilot_template_query_result_t *result;
    madopilot_frame_t *frame;
    madopilot_mapping_t *mapping;
    madopilot_template_query_result_info_t info;
    madopilot_match_t match;
    madopilot_image_t image;
    mpw_token token;
    mpw_shape shape;
} retained_match;

static int require(int condition, const char *reason)
{
    if (!condition) fault = reason;
    return condition;
}

static int call(madopilot_status_t status)
{
    last_status = status;
    return require(status == MADOPILOT_STATUS_OK, "public_call_failed");
}

static void released(madopilot_status_t status)
{
    if (status != MADOPILOT_STATUS_OK) {
        cleanup_failed = 1;
        if (cleanup_status == MADOPILOT_STATUS_OK) cleanup_status = status;
    }
}

static madopilot_str_t borrow(const char *text)
{
    madopilot_str_t value = { text, strlen(text) };
    return value;
}

static int text_equal(madopilot_str_t value, const char *expected)
{
    size_t size = strlen(expected);
    return value.len == size && (size == 0 || (value.data != NULL && memcmp(value.data, expected, size) == 0));
}

static int same_text(madopilot_str_t left, madopilot_str_t right)
{
    return left.len == right.len && (left.len == 0 ||
           (left.data != NULL && right.data != NULL && memcmp(left.data, right.data, left.len) == 0));
}

static int bounded(madopilot_operation_t *operation, uint64_t duration)
{
    uint64_t now;
    INIT(*operation);
    if (!call(api->clock_now(&now)) || !require(now <= UINT64_MAX - duration, "clock_overflow")) return 0;
    operation->flags = MADOPILOT_OPERATION_HAS_DEADLINE;
    operation->deadline_nanos = now + duration;
    return 1;
}

static int remaining(uint64_t deadline, madopilot_operation_t *slice)
{
    uint64_t now;
    if (!call(api->clock_now(&now)) || !require(now < deadline, "observation_deadline")) return 0;
    INIT(*slice);
    slice->flags = MADOPILOT_OPERATION_HAS_DEADLINE;
    slice->deadline_nanos = deadline - now < OBSERVATION_NANOS ? deadline : now + OBSERVATION_NANOS;
    return 1;
}

static int fact(const char *key, const char *format, ...)
{
    char numeric[448];
    va_list arguments;
    int size;
    va_start(arguments, format);
    size = vsnprintf(numeric, sizeof(numeric), format, arguments);
    va_end(arguments);
    return require(size > 0 && (size_t)size < sizeof(numeric) && mpw_fact(row, key, numeric), "protocol_failed");
}

static int stamp_equal(const madopilot_frame_stamp_t *left, const madopilot_frame_stamp_t *right)
{
    return left->stream == right->stream && left->epoch == right->epoch &&
           left->sequence == right->sequence && left->geometry == right->geometry;
}

static int stamp_at_least(const madopilot_frame_stamp_t *left, const madopilot_frame_stamp_t *right)
{
    return left->stream == right->stream &&
           (left->epoch > right->epoch || (left->epoch == right->epoch && left->sequence >= right->sequence));
}

static int same_geometry(const madopilot_frame_stamp_t *left, const madopilot_frame_stamp_t *right)
{
    return left->stream == right->stream && left->epoch == right->epoch && left->geometry == right->geometry;
}

static int frame_fact(const char *key, const madopilot_frame_stamp_t *stamp, uint32_t width, uint32_t height)
{
    return fact(key, "%" PRIu64 " %" PRIu64 " %" PRIu64 " %" PRIu64 " %" PRIu32 " %" PRIu32,
                stamp->stream, stamp->epoch, stamp->sequence, stamp->geometry, width, height);
}

static int transform_fact(const madopilot_transform_snapshot_t *transform, uint64_t target)
{
    return fact("transform", "%" PRIu64 " %.17g %.17g %.17g %.17g %.17g %.17g %" PRIu64,
                transform->geometry, transform->desktop_origin_x, transform->desktop_origin_y,
                transform->logical_width, transform->logical_height,
                transform->target_scale_x, transform->target_scale_y, target);
}

static int rectangle_equal(const madopilot_pixel_rect_t *left, const madopilot_pixel_rect_t *right)
{
    return left->space == right->space && left->left == right->left && left->top == right->top &&
           left->right == right->right && left->bottom == right->bottom;
}

static int full_rectangle(const madopilot_pixel_rect_t *rect, uint32_t width, uint32_t height)
{
    return width > 0 && height > 0 && width <= INT32_MAX && height <= INT32_MAX &&
           rect->space == MADOPILOT_SPACE_CAPTURE_PIXELS && rect->left == 0 && rect->top == 0 &&
           rect->right == (int32_t)width && rect->bottom == (int32_t)height;
}

static int transform_valid(const madopilot_transform_snapshot_t *transform,
                           const madopilot_frame_stamp_t *stamp, uint32_t width, uint32_t height)
{
    const uint32_t required = MADOPILOT_TRANSFORM_COVERS_TARGET | MADOPILOT_TRANSFORM_HAS_TARGET_PLACEMENT;
    return transform->struct_size == sizeof(*transform) && transform->flags == required &&
           transform->geometry == stamp->geometry && transform->width == width && transform->height == height &&
           mpw_finite(transform->desktop_origin_x) && mpw_finite(transform->desktop_origin_y) &&
           mpw_finite(transform->logical_width) && transform->logical_width > 0 &&
           mpw_finite(transform->logical_height) && transform->logical_height > 0 &&
           mpw_finite(transform->target_scale_x) && transform->target_scale_x > 0 &&
           mpw_finite(transform->target_scale_y) && transform->target_scale_y > 0 &&
           mpw_finite(transform->desktop_scale_x) && transform->desktop_scale_x > 0 &&
           mpw_finite(transform->desktop_scale_y) && transform->desktop_scale_y > 0;
}

static int transform_equal(const madopilot_transform_snapshot_t *left,
                           const madopilot_transform_snapshot_t *right)
{
    return left->flags == right->flags && left->geometry == right->geometry &&
           left->width == right->width && left->height == right->height &&
           left->desktop_origin_x == right->desktop_origin_x && left->desktop_origin_y == right->desktop_origin_y &&
           left->logical_width == right->logical_width && left->logical_height == right->logical_height &&
           left->target_scale_x == right->target_scale_x && left->target_scale_y == right->target_scale_y &&
           left->desktop_scale_x == right->desktop_scale_x && left->desktop_scale_y == right->desktop_scale_y;
}

static int shape_equal(const mpw_shape *left, const mpw_shape *right)
{
    return left->marker_cell_w == right->marker_cell_w && left->marker_cell_h == right->marker_cell_h &&
           left->marker_x == right->marker_x && left->marker_y == right->marker_y &&
           left->token_cell_w == right->token_cell_w && left->token_cell_h == right->token_cell_h &&
           left->token_x == right->token_x && left->token_y == right->token_y;
}

static int describe_frame(madopilot_frame_t *frame, madopilot_frame_stamp_t *stamp, madopilot_frame_info_t *info)
{
    INIT(*stamp); INIT(*info);
    return call(api->frame_stamp(frame, stamp)) && call(api->frame_describe(frame, info)) &&
           require(stamp->flags == 0 && stamp->stream != 0 &&
                   full_rectangle(&info->bounds, info->width, info->height) &&
                   info->space == MADOPILOT_SPACE_CAPTURE_PIXELS &&
                   (info->format == MADOPILOT_PIXEL_FORMAT_RGBA8 || info->format == MADOPILOT_PIXEL_FORMAT_BGRA8) &&
                   info->stride >= (uint64_t)info->width * 4u, "frame_contract_failed");
}

static int map_frame(madopilot_frame_t *frame, const madopilot_operation_t *operation,
                     madopilot_mapping_t **mapping, madopilot_image_t *image,
                     const madopilot_frame_stamp_t *stamp, const madopilot_frame_info_t *info)
{
    madopilot_map_request_t request;
    madopilot_frame_stamp_t mapped;
    INIT(request); INIT(*image); INIT(mapped);
    request.format = MADOPILOT_PIXEL_FORMAT_RGBA8;
    if (!call(api->frame_map(frame, &request, operation, mapping, NULL))) return 0;
    return call(api->mapping_stamp(*mapping, &mapped)) && call(api->mapping_describe(*mapping, image)) &&
           require(stamp_equal(stamp, &mapped) && image->width == info->width && image->height == info->height &&
                   image->format == MADOPILOT_PIXEL_FORMAT_RGBA8 && image->space == MADOPILOT_SPACE_CAPTURE_PIXELS &&
                   full_rectangle(&image->region, image->width, image->height), "mapping_contract_failed");
}

static int close_scope(native_scope *scope)
{
    madopilot_operation_t operation;
    int32_t closed = 0;
    int ok = 1;
    if (scope->session != NULL) {
        if (!bounded(&operation, OPERATION_NANOS)) ok = 0;
        else {
            /* One absolute close budget covers the idempotence observation. */
            released(api->session_close(scope->session, &operation, NULL));
            released(api->session_close(scope->session, &operation, NULL));
            released(api->session_is_closed(scope->session, &closed));
            if (closed != 1) ok = 0;
        }
        released(api->session_release(scope->session));
        scope->session = NULL;
    }
    if (scope->engine != NULL) {
        released(api->engine_release(scope->engine));
        scope->engine = NULL;
    }
    if (!ok) cleanup_failed = 1;
    return ok && !cleanup_failed;
}

static void drop_match(retained_match *held)
{
    released(api->mapping_release(held->mapping)); held->mapping = NULL;
    released(api->frame_release(held->frame)); held->frame = NULL;
    released(api->template_query_result_release(held->result)); held->result = NULL;
}

static int admit(native_scope *scope, const char *title, int report_permissions)
{
    madopilot_source_t source;
    madopilot_operation_t operation;
    madopilot_engine_capabilities_t capabilities;
    madopilot_template_scheduler_descriptor_t scheduler;
    madopilot_target_list_t *targets = NULL;
    madopilot_error_t *error = NULL;
    madopilot_open_request_t open;
    madopilot_session_info_t session_info;
    uint64_t selected_target = 0;
    size_t count = 0, index, selected = 0, matches = 0;
    int permissions[2] = { 0, 0 };
    int ok = 0;
    INIT(source); INIT(capabilities); INIT(scheduler); INIT(open); INIT(session_info);
#if defined(_WIN32)
    source.kind = MADOPILOT_SOURCE_NATIVE_WINDOWS;
#elif defined(__APPLE__)
    source.kind = MADOPILOT_SOURCE_NATIVE_MACOS;
#else
    return require(0, "native_platform_unavailable");
#endif
    TRY(bounded(&operation, OPERATION_NANOS));
    TRY(call(api->engine_create(&source, &operation, &scope->engine, &error)));
    TRY(require(error == NULL && scope->engine != NULL, "admission_contract_failed"));
    TRY(call(api->engine_capabilities(scope->engine, &capabilities)));
    TRY(call(api->engine_template_scheduler_descriptor(scope->engine, &scheduler)));
    TRY(require(scheduler.flags == 0 && scheduler.max_engine_queries == 256 &&
                scheduler.max_active_sessions == 16 && scheduler.max_session_queries == 64 &&
                scheduler.max_in_flight_analyses == 2 && scheduler.latest_pending_frames_per_query == 1 &&
                scheduler.max_mapped_cache_entries == 256 && scheduler.mapped_cache_bytes == UINT64_C(67108864) &&
                scheduler.eligible_queue_expiry_nanos == UINT64_C(30000000000), "scheduler_contract_failed"));
    for (index = 0; index < 2; ++index) {
        madopilot_permission_t permission;
        madopilot_status_t status;
        madopilot_permission_kind_t kind = index == 0 ? MADOPILOT_PERMISSION_KIND_SCREEN_CAPTURE : MADOPILOT_PERMISSION_KIND_INPUT_CONTROL;
        INIT(permission);
        TRY(bounded(&operation, OPERATION_NANOS));
        status = api->engine_permission(scope->engine, kind, &operation, &permission, &error);
#if defined(__APPLE__)
        TRY(call(status));
        TRY(require((capabilities.flags & MADOPILOT_ENGINE_READS_PERMISSIONS) != 0 &&
                    error == NULL && permission.kind == kind && permission.state >= 0 && permission.state <= 3,
                    "permission_contract_failed"));
        permissions[index] = permission.state;
#else
        TRY(require((capabilities.flags & MADOPILOT_ENGINE_READS_PERMISSIONS) == 0 &&
                    status == MADOPILOT_STATUS_UNSUPPORTED && error != NULL, "permission_contract_failed"));
        permissions[index] = MADOPILOT_PERMISSION_STATE_UNAVAILABLE;
#endif
        released(api->error_release(error)); error = NULL;
    }
    if (check_only) {
        printf("CHECK capabilities %" PRIu32 " permissions %d %d scheduler %" PRIu32 " %" PRIu32 "\n",
               capabilities.flags, permissions[0], permissions[1], scheduler.max_engine_queries, scheduler.max_in_flight_analyses);
        ok = 1;
        goto cleanup;
    }
    if (report_permissions) TRY(fact("permissions", "%d %d", permissions[0], permissions[1]));
    /* Input permission is reported independently and is never required here. */
#if defined(__APPLE__)
    TRY(require(permissions[0] == MADOPILOT_PERMISSION_STATE_GRANTED, "capture_permission_not_granted"));
#endif
    TRY(bounded(&operation, OPERATION_NANOS));
    TRY(call(api->engine_discover(scope->engine, &operation, &targets, &error)));
    TRY(require(targets != NULL && error == NULL, "admission_contract_failed"));
    TRY(call(api->target_list_count(targets, &count)));
    TRY(require(count <= 65536, "target_count_exceeded"));
    for (index = 0; index < count; ++index) {
        madopilot_target_t target;
        INIT(target);
        TRY(call(api->target_list_get(targets, index, &target)));
        if ((target.flags & MADOPILOT_TARGET_HAS_KIND) != 0 && target.kind == MADOPILOT_TARGET_KIND_WINDOW && text_equal(target.name, title)) {
            TRY(require(target.capture != MADOPILOT_CAPABILITY_UNSUPPORTED, "target_capability_unavailable"));
            TRY(require(target.target != 0 && (target.flags & MADOPILOT_TARGET_SUPPORTS_PLACEMENT) != 0 &&
                        (target.coordinate_spaces & (1 << MADOPILOT_SPACE_CAPTURE_PIXELS)) != 0,
                        "admission_contract_failed"));
            selected = index; selected_target = target.target; ++matches;
        }
    }
    TRY(require(matches == 1, "owned_target_not_unique"));
    TRY(bounded(&operation, OPERATION_NANOS));
    TRY(call(api->session_open(scope->engine, targets, selected, &open, &operation, &scope->session, &error)));
    TRY(call(api->clock_now(&scope->opened_nanos)));
    TRY(require(scope->session != NULL && error == NULL, "admission_contract_failed"));
    TRY(call(api->session_describe(scope->session, &session_info)));
    TRY(require(session_info.target == selected_target && session_info.stream != 0 && session_info.accepts_input == 0,
                "admission_contract_failed"));
    scope->target = session_info.target; scope->stream = session_info.stream;
    if (report_permissions) TRY(fact("status", "0 0"));
    ok = 1;
cleanup:
    if (error != NULL) {
        /* A typed status/category is enough; never print provider message/views. */
        madopilot_error_detail_t detail;
        INIT(detail);
        if (check_only && api->error_describe(error, &detail) == MADOPILOT_STATUS_OK)
            printf("CHECK refusal %" PRId32 " category %" PRId32 "\n", last_status, detail.category);
        released(api->error_release(error));
    }
    released(api->target_list_release(targets));
    return ok;
}

/* The request and nested match options are call-local. Every package/template
 * reference is released before this function returns, including error paths. */
static int start_query(native_scope *scope, const mpw_shape *shape, int probe,
                       madopilot_template_query_t **query)
{
    char path[MPW_LINE_MAX + 1];
    madopilot_package_source_t source;
    madopilot_package_t *package = NULL;
    madopilot_template_t *tmpl = NULL;
    madopilot_template_info_t description;
    madopilot_match_options_t match_options;
    madopilot_template_watch_options_t options;
    madopilot_operation_t operation;
    uint32_t cell_w = probe ? 1u : shape->marker_cell_w;
    uint32_t cell_h = probe ? 1u : shape->marker_cell_h;
    int ok = 0;
    INIT(source); INIT(description); INIT(match_options); INIT(options);
    TRY(require(*query == NULL, "ownership_contract_failed"));
    TRY(require(mpw_asset(cell_w, cell_h, path, sizeof(path)), "protocol_failed"));
    source.kind = MADOPILOT_PACKAGE_SOURCE_DIRECTORY;
    source.path = borrow(path);
    TRY(bounded(&operation, OPERATION_NANOS));
    TRY(call(api->package_load(scope->engine, &source, &operation, &package, NULL)));
    TRY(bounded(&operation, OPERATION_NANOS));
    TRY(call(api->template_prepare_from_package(scope->engine, package, borrow("native.marker"), &operation, &tmpl, NULL)));
    TRY(call(api->template_describe(tmpl, &description)));
    TRY(require(description.width == cell_w * 3u && description.height == cell_h * 2u &&
                description.space == MADOPILOT_SPACE_CAPTURE_PIXELS && text_equal(description.id, "native.marker") &&
                text_equal(description.backend, "opencv-cpu"), "prepared_template_mismatch"));
    match_options.flags = MADOPILOT_MATCH_HAS_MIN_SCORE | MADOPILOT_MATCH_HAS_MAX_RESULTS | MADOPILOT_MATCH_HAS_SUPPRESSION;
    match_options.min_score = probe ? 0.0 : 0.95;
    match_options.max_results = 1;
    match_options.suppression = MADOPILOT_SUPPRESSION_DROP_OVERLAPPING;
    options.match_options = &match_options;
    options.stability_kind = MADOPILOT_TEMPLATE_STABILITY_IMMEDIATE;
    options.change_policy = probe ? MADOPILOT_TEMPLATE_CHANGE_ANALYSIS_ALWAYS : MADOPILOT_TEMPLATE_CHANGE_EXACT_RGBA;
    TRY(bounded(&operation, OPERATION_NANOS));
    TRY(call(api->session_start_template_watch(scope->session, tmpl, &options, &operation, query, NULL)));
    TRY(require(*query != NULL, "ownership_contract_failed"));
    ok = 1;
cleanup:
    released(api->template_release(tmpl));
    released(api->package_release(package));
    memset(path, 0, sizeof(path));
    return ok && !cleanup_failed;
}

static int wait_result(madopilot_template_query_t *query, madopilot_template_query_result_t **result)
{
    madopilot_operation_t operation;
    return bounded(&operation, OPERATION_NANOS) &&
           call(api->template_query_wait(query, &operation, result, NULL)) &&
           require(*result != NULL, "terminal_contract_failed");
}

/* A metadata-only probe has its own explicit authority. It cannot pass F3.
 * There is no public raw-frame transform accessor; this uses the exact retained
 * public result, not a discovery estimate or a controller capture. */
static int bootstrap(native_scope *scope, const madopilot_frame_stamp_t *transition, geometry_authority *geometry)
{
    madopilot_template_query_t *query = NULL;
    madopilot_template_query_result_t *result = NULL;
    madopilot_frame_t *frame = NULL;
    madopilot_template_query_result_info_t info;
    madopilot_frame_info_t frame_info;
    madopilot_frame_stamp_t stamp;
    int ok = 0;
    INIT(info);
    TRY(start_query(scope, NULL, 1, &query));
    TRY(wait_result(query, &result));
    TRY(call(api->template_query_result_info(result, &info)));
    TRY(require(info.outcome == MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED && info.status == MADOPILOT_STATUS_OK &&
                info.target == scope->target && info.match_count == 1 && info.source.stream == scope->stream,
                "bootstrap_not_matched"));
    TRY(call(api->template_query_result_frame(result, &frame)));
    TRY(describe_frame(frame, &stamp, &frame_info));
    TRY(require(stamp_equal(&stamp, &info.source) &&
                transform_valid(&info.transform, &stamp, frame_info.width, frame_info.height) &&
                (transition == NULL || (stamp_at_least(&stamp, transition) && same_geometry(&stamp, transition))),
                "bootstrap_geometry_mismatch"));
    geometry->stamp = stamp;
    geometry->transform = info.transform;
    TRY(require(mpw_geometry(frame_info.width, frame_info.height,
                             info.transform.desktop_origin_x, info.transform.desktop_origin_y,
                             info.transform.logical_width, info.transform.logical_height,
                             info.transform.target_scale_x, info.transform.target_scale_y, &geometry->shape),
                "protocol_failed"));
    TRY(frame_fact("bootstrap", &stamp, frame_info.width, frame_info.height));
    ok = 1;
cleanup:
    released(api->frame_release(frame));
    released(api->template_query_result_release(result));
    released(api->template_query_release(query));
    return ok && !cleanup_failed;
}

/* Each slice uses the original absolute phase deadline. Only slice timeout and
 * pixel mismatch continue observation; no command, token or probe is retried. */
static int observe_token(native_scope *scope, geometry_authority *geometry, const mpw_token *token)
{
    madopilot_operation_t phase, slice;
    madopilot_frame_t *frame = NULL;
    madopilot_mapping_t *mapping = NULL;
    madopilot_frame_info_t info;
    madopilot_frame_stamp_t stamp;
    madopilot_image_t image;
    int ok = 0;
    TRY(bounded(&phase, OPERATION_NANOS));
    while (remaining(phase.deadline_nanos, &slice)) {
        madopilot_status_t status = api->session_acquire_frame(scope->session, &slice, &frame, NULL);
        if (status == MADOPILOT_STATUS_DEADLINE_EXCEEDED) {
            TRY(require(frame == NULL, "failure_output_not_reset"));
            mpw_pause(); continue;
        }
        TRY(call(status));
        TRY(describe_frame(frame, &stamp, &info));
        TRY(require(same_geometry(&stamp, &geometry->stamp) && stamp_at_least(&stamp, &geometry->stamp) &&
                    info.width == geometry->transform.width && info.height == geometry->transform.height,
                    "unexpected_geometry"));
        TRY(remaining(phase.deadline_nanos, &slice));
        if (!map_frame(frame, &slice, &mapping, &image, &stamp, &info)) {
            if (last_status != MADOPILOT_STATUS_DEADLINE_EXCEEDED) goto cleanup;
            TRY(require(mapping == NULL, "failure_output_not_reset"));
        } else if (mpw_pixels_match_token(image.bytes.data, image.bytes.len, image.stride,
                                          image.width, image.height, &geometry->shape, token)) {
            TRY(call(api->clock_now(&geometry->observed_nanos)));
            TRY(require(geometry->observed_nanos < phase.deadline_nanos, "observation_deadline"));
            geometry->stamp = stamp;
            ok = 1;
        }
        released(api->mapping_release(mapping)); mapping = NULL;
        released(api->frame_release(frame)); frame = NULL;
        if (ok) break;
        mpw_pause();
    }
    if (ok) {
        ok = frame_fact("frame", &geometry->stamp, geometry->transform.width, geometry->transform.height) &&
             transform_fact(&geometry->transform, scope->target);
    }
cleanup:
    released(api->mapping_release(mapping));
    released(api->frame_release(frame));
    return ok && !cleanup_failed;
}

static int wait_geometry_transition(native_scope *scope, const geometry_authority *before,
                                    int movement, madopilot_frame_stamp_t *transition)
{
    madopilot_operation_t phase, slice;
    madopilot_frame_t *frame = NULL;
    madopilot_frame_stamp_t stamp;
    madopilot_frame_info_t info;
    int ok = 0;
    TRY(bounded(&phase, OPERATION_NANOS));
    while (remaining(phase.deadline_nanos, &slice)) {
        madopilot_status_t status = api->session_acquire_frame(scope->session, &slice, &frame, NULL);
        if (status == MADOPILOT_STATUS_DEADLINE_EXCEEDED) {
            TRY(require(frame == NULL, "failure_output_not_reset"));
            mpw_pause(); continue;
        }
        TRY(call(status));
        TRY(describe_frame(frame, &stamp, &info));
        TRY(require(stamp_at_least(&stamp, &before->stamp), "frame_order_mismatch"));
        if (!same_geometry(&stamp, &before->stamp) &&
            (movement || info.width != before->transform.width || info.height != before->transform.height)) {
            *transition = stamp; ok = 1;
        }
        released(api->frame_release(frame)); frame = NULL;
        if (ok) break;
        mpw_pause();
    }
cleanup:
    released(api->frame_release(frame));
    return ok && !cleanup_failed;
}

static int pending_ready(const madopilot_template_query_snapshot_t *snapshot,
                         const madopilot_frame_stamp_t *after, uint64_t completed_before)
{
    return snapshot->completed > completed_before &&
           (snapshot->flags & MADOPILOT_TEMPLATE_QUERY_HAS_LAST_FRAME) != 0 &&
           same_geometry(&snapshot->last_frame, after) && stamp_at_least(&snapshot->last_frame, after);
}

static int pending(madopilot_template_query_t *query, const madopilot_frame_stamp_t *after,
                    uint64_t completed_before, madopilot_template_query_snapshot_t *out)
{
    madopilot_operation_t phase, slice;
    madopilot_template_query_result_t *terminal = NULL;
    int ok = 0;
    TRY(bounded(&phase, OPERATION_NANOS));
    while (remaining(phase.deadline_nanos, &slice)) {
        INIT(*out);
        TRY(call(api->template_query_poll(query, out, &terminal, NULL)));
        TRY(require(out->state == MADOPILOT_TEMPLATE_QUERY_STATE_PENDING && terminal == NULL && out->query_id != 0 &&
                    out->pending_count <= 1 && out->in_flight_count <= 1 && out->confirmed_observations == 0 &&
                    out->confirmed_duration_nanos == 0 && out->failed == 0 && out->queue_expired == 0,
                    "pending_contract_failed"));
        if (pending_ready(out, after, completed_before)) {
            TRY(fact("pending", "%" PRIu64 " %" PRIu32 " %" PRIu64,
                     out->completed, out->confirmed_observations, out->confirmed_duration_nanos));
            ok = 1; break;
        }
        mpw_pause();
    }
cleanup:
    released(api->template_query_result_release(terminal));
    return ok && !cleanup_failed;
}

static int version_view(madopilot_str_t version)
{
    size_t index;
    if (version.data == NULL || version.len == 0 || version.len > 96 ||
        version.data[0] < '0' || version.data[0] > '9') return 0;
    for (index = 0; index < version.len; ++index) {
        unsigned char ch = (unsigned char)version.data[index];
        if (!((ch >= '0' && ch <= '9') || (ch >= 'a' && ch <= 'z') ||
              (ch >= 'A' && ch <= 'Z') || ch == '.' || ch == '-' || ch == '+')) return 0;
    }
    return 1;
}

static int match_facts(const retained_match *held)
{
    return frame_fact("frame", &held->info.source, held->image.width, held->image.height) &&
           transform_fact(&held->info.transform, held->info.target) &&
           fact("match", "%" PRIu64 " %" PRIu64 " %" PRIu64 " %" PRIu32 " %" PRIu64
                " %" PRId32 " %" PRId32 " %" PRId32 " %" PRId32,
                held->info.query_id, held->info.target, held->info.match_count, held->info.confirmed_observations,
                held->info.confirmed_duration_nanos, held->match.bounds.left, held->match.bounds.top,
                held->match.bounds.right, held->match.bounds.bottom) &&
           fact("mapped_view_bytes", "%zu", held->image.bytes.len);
}

static int collect_match(native_scope *scope, madopilot_template_query_t *query,
                          const geometry_authority *geometry, const mpw_token *token, retained_match *held)
{
    madopilot_frame_stamp_t stamp;
    madopilot_frame_info_t frame_info;
    madopilot_operation_t operation;
    madopilot_match_t invalid;
    madopilot_template_query_snapshot_t snapshot;
    madopilot_template_query_result_t *polled = NULL;
    mpw_shape exact_shape;
    int ok = 0;
    TRY(require(held->result == NULL && held->frame == NULL && held->mapping == NULL, "ownership_contract_failed"));
    INIT(snapshot);
    TRY(call(api->template_query_poll(query, &snapshot, &polled, NULL)));
    TRY(require(snapshot.query_id != 0 &&
                ((snapshot.state == MADOPILOT_TEMPLATE_QUERY_STATE_PENDING && polled == NULL) ||
                 (snapshot.state == MADOPILOT_TEMPLATE_QUERY_STATE_TERMINAL && polled != NULL)),
                "terminal_contract_failed"));
    TRY(wait_result(query, &held->result));
    INIT(held->info); INIT(held->match);
    TRY(call(api->template_query_result_info(held->result, &held->info)));
    TRY(require(held->info.outcome == MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED && held->info.status == MADOPILOT_STATUS_OK &&
                held->info.overload == MADOPILOT_TEMPLATE_OVERLOAD_NONE && held->info.query_id == snapshot.query_id &&
                held->info.target == scope->target && held->info.match_count == 1 &&
                held->info.confirmed_observations == 1 && held->info.confirmed_duration_nanos == 0 &&
                text_equal(held->info.template_id, "native.marker") && text_equal(held->info.backend_id, "opencv-cpu") &&
                version_view(held->info.backend_version) && held->info.options.flags ==
                (MADOPILOT_MATCH_HAS_MIN_SCORE | MADOPILOT_MATCH_HAS_MAX_RESULTS | MADOPILOT_MATCH_HAS_SUPPRESSION) &&
                held->info.options.min_score == 0.95 && held->info.options.max_results == 1 &&
                held->info.options.suppression == MADOPILOT_SUPPRESSION_DROP_OVERLAPPING,
                "matched_contract_failed"));
    TRY(call(api->template_query_result_frame(held->result, &held->frame)));
    TRY(describe_frame(held->frame, &stamp, &frame_info));
    TRY(require(stamp_equal(&stamp, &held->info.source) && same_geometry(&stamp, &geometry->stamp) &&
                stamp_at_least(&stamp, &geometry->stamp) &&
                transform_valid(&held->info.transform, &stamp, frame_info.width, frame_info.height) &&
                transform_equal(&held->info.transform, &geometry->transform) &&
                full_rectangle(&held->info.effective_region, frame_info.width, frame_info.height), "match_geometry_mismatch"));
    TRY(require(mpw_geometry(frame_info.width, frame_info.height,
                             held->info.transform.desktop_origin_x, held->info.transform.desktop_origin_y,
                             held->info.transform.logical_width, held->info.transform.logical_height,
                             held->info.transform.target_scale_x, held->info.transform.target_scale_y, &exact_shape) &&
                shape_equal(&exact_shape, &geometry->shape), "match_geometry_mismatch"));
    TRY(call(api->template_query_result_match_at(held->result, 0, &held->match)));
    TRY(require(text_equal(held->match.template_id, "native.marker") && held->match.flags == 0 &&
                mpw_finite(held->match.score) && held->match.score >= held->info.options.min_score && held->match.score <= 1.0 &&
                held->match.bounds.space == MADOPILOT_SPACE_CAPTURE_PIXELS &&
                held->match.bounds.left == exact_shape.marker_x && held->match.bounds.top == exact_shape.marker_y &&
                held->match.bounds.right == exact_shape.marker_x + (int32_t)(exact_shape.marker_cell_w * 3u) &&
                held->match.bounds.bottom == exact_shape.marker_y + (int32_t)(exact_shape.marker_cell_h * 2u),
                "match_bounds_mismatch"));
    memset(&invalid, 0xa5, sizeof(invalid)); invalid.struct_size = (uint32_t)sizeof(invalid);
    TRY(require(api->template_query_result_match_at(held->result, 1, &invalid) == MADOPILOT_STATUS_INVALID_ARGUMENT &&
                invalid.flags == 0 && invalid.score == 0 && invalid.template_id.data == NULL && invalid.template_id.len == 0 &&
                invalid.bounds.space == MADOPILOT_SPACE_CAPTURE_PIXELS && invalid.bounds.left == 0 && invalid.bounds.top == 0 &&
                invalid.bounds.right == 0 && invalid.bounds.bottom == 0, "failure_output_not_reset"));
    TRY(bounded(&operation, OPERATION_NANOS));
    TRY(map_frame(held->frame, &operation, &held->mapping, &held->image, &stamp, &frame_info));
    TRY(require(token->visible == 1 && mpw_pixels_match_token(held->image.bytes.data, held->image.bytes.len,
                held->image.stride, held->image.width, held->image.height, &exact_shape, token), "exact_result_token_mismatch"));
    held->token = *token; held->shape = exact_shape;
    TRY(match_facts(held));
    ok = 1;
cleanup:
    released(api->template_query_result_release(polled));
    return ok && !cleanup_failed;
}

static int same_terminal(const madopilot_template_query_result_info_t *left,
                         const madopilot_template_query_result_info_t *right)
{
    return left->outcome == right->outcome && left->status == right->status && left->overload == right->overload &&
           left->query_id == right->query_id && left->target == right->target && stamp_equal(&left->source, &right->source) &&
           left->match_count == right->match_count && left->confirmed_observations == right->confirmed_observations &&
           left->confirmed_duration_nanos == right->confirmed_duration_nanos &&
           same_text(left->template_id, right->template_id) && same_text(left->backend_id, right->backend_id) &&
           same_text(left->backend_version, right->backend_version) &&
           left->options.flags == right->options.flags && left->options.min_score == right->options.min_score &&
           left->options.max_results == right->options.max_results && left->options.suppression == right->options.suppression &&
           rectangle_equal(&left->effective_region, &right->effective_region) && transform_equal(&left->transform, &right->transform);
}

static int nonmatched(madopilot_template_query_result_t *result, madopilot_template_query_outcome_t outcome)
{
    madopilot_template_query_result_info_t info, cleared;
    madopilot_match_t match;
    madopilot_frame_t *frame = (madopilot_frame_t *)(uintptr_t)1;
    madopilot_error_t *error = (madopilot_error_t *)(uintptr_t)1;
    madopilot_status_t expected = MADOPILOT_STATUS_CLOSED;
    int ok = 0;
    if (outcome == MADOPILOT_TEMPLATE_QUERY_OUTCOME_CANCELLED) expected = MADOPILOT_STATUS_CANCELLED;
    if (outcome == MADOPILOT_TEMPLATE_QUERY_OUTCOME_DEADLINE_EXCEEDED) expected = MADOPILOT_STATUS_DEADLINE_EXCEEDED;
    if (outcome == MADOPILOT_TEMPLATE_QUERY_OUTCOME_TARGET_LOST) expected = MADOPILOT_STATUS_TARGET_LOST;
    INIT(info); INIT(cleared); INIT(cleared.source); INIT(cleared.options); INIT(cleared.transform);
    TRY(call(api->template_query_result_info(result, &info)));
    cleared.outcome = outcome;
    cleared.status = expected;
    cleared.query_id = info.query_id;
    TRY(require(same_terminal(&info, &cleared) && info.struct_size == sizeof(info) &&
                info.source.struct_size == sizeof(info.source) && info.source.flags == 0 &&
                info.options.struct_size == sizeof(info.options) && info.transform.struct_size == sizeof(info.transform) &&
                info.outcome == outcome && info.status == expected && info.query_id != 0 && info.target == 0 &&
                info.overload == MADOPILOT_TEMPLATE_OVERLOAD_NONE && info.match_count == 0 &&
                info.confirmed_observations == 0 && info.confirmed_duration_nanos == 0 &&
                info.template_id.data == NULL && info.template_id.len == 0 &&
                info.backend_id.data == NULL && info.backend_id.len == 0 &&
                info.backend_version.data == NULL && info.backend_version.len == 0 &&
                info.source.stream == 0 && info.source.epoch == 0 && info.source.sequence == 0 && info.source.geometry == 0 &&
                info.transform.flags == 0 && info.transform.width == 0 && info.transform.height == 0,
                "nonmatched_contract_failed"));
    memset(&match, 0xa5, sizeof(match)); match.struct_size = (uint32_t)sizeof(match);
    TRY(require(api->template_query_result_match_at(result, 0, &match) == MADOPILOT_STATUS_INVALID_ARGUMENT &&
                match.flags == 0 && match.score == 0 && match.template_id.data == NULL && match.template_id.len == 0 &&
                match.bounds.space == MADOPILOT_SPACE_CAPTURE_PIXELS && match.bounds.left == 0 && match.bounds.top == 0 &&
                match.bounds.right == 0 && match.bounds.bottom == 0, "failure_output_not_reset"));
    TRY(require(api->template_query_result_frame(result, &frame) == MADOPILOT_STATUS_INVALID_ARGUMENT && frame == NULL,
                "failure_output_not_reset"));
    TRY(require(api->template_query_result_error(result, &error) == MADOPILOT_STATUS_OK && error == NULL,
                "failure_output_not_reset"));
    TRY(fact("status", "0 %" PRId32, outcome));
    ok = 1;
cleanup:
    if (frame != NULL && frame != (madopilot_frame_t *)(uintptr_t)1) released(api->frame_release(frame));
    if (error != NULL && error != (madopilot_error_t *)(uintptr_t)1) released(api->error_release(error));
    return ok && !cleanup_failed;
}

static int immutable_cancel(madopilot_template_query_t *query, madopilot_template_query_outcome_t outcome)
{
    madopilot_template_query_result_t *first = NULL, *second = NULL, *prior = NULL;
    madopilot_template_query_result_info_t left, right, previous;
    madopilot_template_query_snapshot_t snapshot;
    int ok = 0;
    INIT(left); INIT(right); INIT(previous); INIT(snapshot);
    TRY(call(api->template_query_poll(query, &snapshot, &prior, NULL)));
    if (prior != NULL) {
        TRY(require(snapshot.state == MADOPILOT_TEMPLATE_QUERY_STATE_TERMINAL, "terminal_contract_failed"));
        TRY(call(api->template_query_result_info(prior, &previous)));
        TRY(require(previous.outcome == outcome, "terminal_winner_changed"));
    } else {
        TRY(require(snapshot.state == MADOPILOT_TEMPLATE_QUERY_STATE_PENDING &&
                    outcome == MADOPILOT_TEMPLATE_QUERY_OUTCOME_CANCELLED, "terminal_contract_failed"));
    }
    TRY(call(api->template_query_cancel(query, &first, NULL)));
    TRY(call(api->template_query_result_info(first, &left)));
    TRY(require(left.outcome == outcome && left.query_id == snapshot.query_id &&
                (prior == NULL || same_terminal(&previous, &left)), "terminal_winner_changed"));
    TRY(call(api->template_query_cancel(query, &second, NULL)));
    TRY(call(api->template_query_result_info(second, &right)));
    TRY(require(same_terminal(&left, &right), "terminal_winner_changed"));
    if (outcome != MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED) TRY(nonmatched(second, outcome));
    ok = 1;
cleanup:
    released(api->template_query_result_release(prior));
    released(api->template_query_result_release(second));
    released(api->template_query_result_release(first));
    return ok && !cleanup_failed;
}

static int absent_query(native_scope *scope, geometry_authority *geometry,
                         madopilot_template_query_t **query, madopilot_template_query_snapshot_t *snapshot)
{
    mpw_token token;
    return require(mpw_visual(0, &token), "protocol_failed") && observe_token(scope, geometry, &token) &&
           start_query(scope, &geometry->shape, 0, query) && pending(*query, &geometry->stamp, 0, snapshot);
}

static int geometry_row(native_scope *scope, geometry_authority *geometry, int movement, int *unavailable)
{
    geometry_authority before;
    madopilot_template_query_t *old_query = NULL, *query = NULL;
    madopilot_template_query_snapshot_t prior, newer;
    madopilot_frame_stamp_t transition;
    mpw_token token;
    retained_match held;
    int action, ok = 0;
    memset(&held, 0, sizeof(held));
    *unavailable = 0;
    TRY(absent_query(scope, geometry, &old_query, &prior));
    before = *geometry;
    action = mpw_action(movement ? "MOVE" : "RESIZE");
    TRY(require(action != 0, "protocol_failed"));
    if (action == -1) {
        TRY(immutable_cancel(old_query, MADOPILOT_TEMPLATE_QUERY_OUTCOME_CANCELLED));
        *unavailable = 1; ok = 1; goto cleanup;
    }
    TRY(require(mpw_visual(0, &token), "protocol_failed"));
    TRY(wait_geometry_transition(scope, &before, movement, &transition));
    TRY(bootstrap(scope, &transition, geometry));
    TRY(observe_token(scope, geometry, &token));
    TRY(require(!same_geometry(&before.stamp, &geometry->stamp), "geometry_transition_missing"));
    if (movement) {
        TRY(require(before.transform.desktop_origin_x != geometry->transform.desktop_origin_x ||
                    before.transform.desktop_origin_y != geometry->transform.desktop_origin_y, "movement_not_observed"));
#if defined(_WIN32)
        TRY(require(before.transform.target_scale_x != geometry->transform.target_scale_x ||
                    before.transform.target_scale_y != geometry->transform.target_scale_y, "scale_transition_missing"));
#endif
    } else {
        TRY(require(before.transform.width != geometry->transform.width || before.transform.height != geometry->transform.height,
                    "resize_not_observed"));
    }
    /* A completed no-match analysis on the new absent generation must
     * replace old authority before the old query is cancelled. */
    TRY(pending(old_query, &geometry->stamp, prior.completed, &newer));
    TRY(require(newer.query_id == prior.query_id && newer.generation > prior.generation,
                "pending_generation_mismatch"));
    TRY(immutable_cancel(old_query, MADOPILOT_TEMPLATE_QUERY_OUTCOME_CANCELLED));
    released(api->template_query_release(old_query)); old_query = NULL;
    TRY(start_query(scope, &geometry->shape, 0, &query));
    TRY(pending(query, &geometry->stamp, 0, &newer));
    TRY(require(mpw_visual(1, &token), "protocol_failed"));
    TRY(collect_match(scope, query, geometry, &token, &held));
    TRY(immutable_cancel(query, MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED));
    ok = 1;
cleanup:
    drop_match(&held);
    released(api->template_query_release(query));
    released(api->template_query_release(old_query));
    return ok && !cleanup_failed;
}

static int independent_authority(native_scope *scope, geometry_authority *geometry)
{
    madopilot_template_query_t *query = NULL;
    madopilot_template_query_result_t *result = NULL;
    madopilot_cancellation_t *cancellation = NULL;
    madopilot_operation_t operation;
    madopilot_template_query_snapshot_t snapshot;
    retained_match held;
    mpw_token token;
    madopilot_status_t status;
    int ok = 0;
    memset(&held, 0, sizeof(held));
    TRY(absent_query(scope, geometry, &query, &snapshot));
    /* C references share authority too; intermediate release cannot cancel. */
    TRY(call(api->template_query_retain(query)));
    released(api->template_query_release(query));
    TRY(call(api->cancellation_create(&cancellation)));
    TRY(call(api->cancellation_cancel(cancellation)));
    TRY(bounded(&operation, OPERATION_NANOS));
    operation.cancellation = cancellation;
    status = api->template_query_wait(query, &operation, &result, NULL);
    TRY(require(status == MADOPILOT_STATUS_CANCELLED && result == NULL, "caller_wait_cancel_failed"));
    TRY(fact("status", "%" PRId32 " 0", status));
    released(api->cancellation_release(cancellation)); cancellation = NULL;
    TRY(pending(query, &geometry->stamp, 0, &snapshot));
    TRY(bounded(&operation, 0));
    status = api->template_query_wait(query, &operation, &result, NULL);
    TRY(require(status == MADOPILOT_STATUS_DEADLINE_EXCEEDED && result == NULL, "caller_wait_deadline_failed"));
    TRY(fact("status", "%" PRId32 " 0", status));
    TRY(pending(query, &geometry->stamp, 0, &snapshot));
    TRY(require(mpw_visual(1, &token), "protocol_failed"));
    TRY(collect_match(scope, query, geometry, &token, &held));
    TRY(immutable_cancel(query, MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED));
    drop_match(&held);
    released(api->template_query_release(query)); query = NULL;
    TRY(absent_query(scope, geometry, &query, &snapshot));
    /* The query's original five-second lifetime wins before this new caller
     * wait. No sleep, deadline reset or cancellation manufactures expiry. */
    TRY(wait_result(query, &result));
    TRY(nonmatched(result, MADOPILOT_TEMPLATE_QUERY_OUTCOME_DEADLINE_EXCEEDED));
    TRY(immutable_cancel(query, MADOPILOT_TEMPLATE_QUERY_OUTCOME_DEADLINE_EXCEEDED));
    released(api->template_query_result_release(result)); result = NULL;
    released(api->template_query_release(query)); query = NULL;
    TRY(absent_query(scope, geometry, &query, &snapshot));
    TRY(immutable_cancel(query, MADOPILOT_TEMPLATE_QUERY_OUTCOME_CANCELLED));
    ok = 1;
cleanup:
    drop_match(&held);
    released(api->cancellation_release(cancellation));
    released(api->template_query_result_release(result));
    released(api->template_query_release(query));
    return ok && !cleanup_failed;
}

static int retained_readable(retained_match *held)
{
    madopilot_template_query_result_info_t info;
    madopilot_match_t match;
    madopilot_frame_stamp_t stamp, mapped;
    madopilot_frame_info_t frame_info;
    madopilot_image_t image;
    INIT(info); INIT(match); INIT(mapped); INIT(image);
    return call(api->template_query_result_info(held->result, &info)) &&
           call(api->template_query_result_match_at(held->result, 0, &match)) &&
           describe_frame(held->frame, &stamp, &frame_info) && call(api->mapping_stamp(held->mapping, &mapped)) &&
           call(api->mapping_describe(held->mapping, &image)) &&
           require(same_terminal(&held->info, &info) && same_text(held->match.template_id, match.template_id) &&
                   text_equal(held->match.template_id, "native.marker") && text_equal(held->info.backend_id, "opencv-cpu") &&
                   held->match.score == match.score && rectangle_equal(&held->match.bounds, &match.bounds) &&
                   stamp_equal(&stamp, &info.source) && stamp_equal(&mapped, &info.source) &&
                   image.width == held->image.width && image.height == held->image.height && image.stride == held->image.stride &&
                   image.bytes.data == held->image.bytes.data && image.bytes.len == held->image.bytes.len &&
                   mpw_pixels_match_token(held->image.bytes.data, held->image.bytes.len, held->image.stride,
                                          held->image.width, held->image.height, &held->shape, &held->token),
                   "retained_ownership_failed");
}

static int resource_fact(unsigned phase, unsigned index)
{
    mpw_resource_snapshot snapshot;
    return require(mpw_resources(&snapshot), "resource_snapshot_failed") &&
           fact("resource", "%u %u %" PRIu64 " %" PRIu64 " %" PRIu64, phase, index,
                snapshot.private_or_footprint_bytes, snapshot.resident_bytes, snapshot.handle_or_port_count);
}

/* One complete owned lifetime; the initial admission belongs to the caller.
 * These explicit F1 precursor stages exercise the same assertions without
 * contributing a match or ownership pass to any later qualification row. */
static int resource_lifetime(native_scope *scope)
{
    geometry_authority geometry;
    retained_match held;
    madopilot_template_query_t *query = NULL;
    madopilot_template_query_snapshot_t snapshot;
    mpw_token token;
    int ok = 0;
    memset(&geometry, 0, sizeof(geometry));
    memset(&held, 0, sizeof(held));
    TRY(require(mpw_visual(0, &token), "protocol_failed"));
    TRY(bootstrap(scope, NULL, &geometry));
    TRY(observe_token(scope, &geometry, &token));
    TRY(start_query(scope, &geometry.shape, 0, &query));
    TRY(pending(query, &geometry.stamp, 0, &snapshot));
    TRY(require(mpw_visual(1, &token), "protocol_failed"));
    TRY(collect_match(scope, query, &geometry, &token, &held));
    TRY(immutable_cancel(query, MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED));
    released(api->template_query_release(query)); query = NULL;
    TRY(close_scope(scope));
    TRY(retained_readable(&held));
    ok = 1;
cleanup:
    released(api->template_query_release(query));
    drop_match(&held);
    if (!close_scope(scope)) ok = 0;
    return ok && !cleanup_failed;
}

static int resource_prelude(native_scope *scope, const char *title)
{
    unsigned index;
    if (!resource_lifetime(scope) || !resource_fact(0, 0)) return 0;
    for (index = 1; index <= 3; ++index) {
        if (!admit(scope, title, 0) || !resource_lifetime(scope) || !resource_fact(1, index)) return 0;
    }
    return 1;
}

static int retained_ownership(native_scope *scope, const char *title, geometry_authority *geometry, retained_match *held)
{
    mpw_token token;
    TRY(close_scope(scope));
    TRY(retained_readable(held));
    TRY(match_facts(held));
    TRY(admit(scope, title, 0));
    TRY(require(mpw_visual(0, &token), "protocol_failed"));
    /* This is a declared fresh-engine metadata stage, not a replacement
     * observation. Engine-local geometry/stream ordinals are never relabelled
     * or compared across engines; obtain a real new exact-frame transform. */
    TRY(bootstrap(scope, NULL, geometry));
    TRY(observe_token(scope, geometry, &token));
    TRY(retained_readable(held));
    return 1;
cleanup:
    return 0;
}

static int lifecycle_terminals(native_scope *scope, const char *title, geometry_authority *geometry)
{
    madopilot_template_query_t *query = NULL;
    madopilot_template_query_result_t *result = NULL;
    madopilot_template_query_snapshot_t snapshot;
    madopilot_operation_t operation;
    mpw_token token;
    int ok = 0;
    TRY(absent_query(scope, geometry, &query, &snapshot));
    TRY(bounded(&operation, OPERATION_NANOS));
    TRY(call(api->session_close(scope->session, &operation, NULL)));
    TRY(wait_result(query, &result));
    TRY(nonmatched(result, MADOPILOT_TEMPLATE_QUERY_OUTCOME_SESSION_CLOSED));
    TRY(immutable_cancel(query, MADOPILOT_TEMPLATE_QUERY_OUTCOME_SESSION_CLOSED));
    released(api->template_query_result_release(result)); result = NULL;
    released(api->template_query_release(query)); query = NULL;
    TRY(close_scope(scope));
    TRY(admit(scope, title, 0));
    TRY(require(mpw_visual(0, &token), "protocol_failed"));
    TRY(bootstrap(scope, NULL, geometry));
    TRY(observe_token(scope, geometry, &token));
    TRY(start_query(scope, &geometry->shape, 0, &query));
    TRY(pending(query, &geometry->stamp, 0, &snapshot));
    /* No target-list, package or prepared-template reference masks finality. */
    TRY(call(api->engine_release(scope->engine))); scope->engine = NULL;
    TRY(wait_result(query, &result));
    TRY(nonmatched(result, MADOPILOT_TEMPLATE_QUERY_OUTCOME_SCHEDULER_CLOSED));
    TRY(immutable_cancel(query, MADOPILOT_TEMPLATE_QUERY_OUTCOME_SCHEDULER_CLOSED));
    released(api->template_query_result_release(result)); result = NULL;
    released(api->template_query_release(query)); query = NULL;
    TRY(close_scope(scope));
    TRY(admit(scope, title, 0));
    TRY(require(mpw_visual(0, &token), "protocol_failed"));
    TRY(bootstrap(scope, NULL, geometry));
    TRY(observe_token(scope, geometry, &token));
    TRY(start_query(scope, &geometry->shape, 0, &query));
    TRY(pending(query, &geometry->stamp, 0, &snapshot));
    TRY(require(mpw_action("DESTROY") == 1, "protocol_failed"));
    TRY(wait_result(query, &result));
    TRY(nonmatched(result, MADOPILOT_TEMPLATE_QUERY_OUTCOME_TARGET_LOST));
    TRY(immutable_cancel(query, MADOPILOT_TEMPLATE_QUERY_OUTCOME_TARGET_LOST));
    ok = 1;
cleanup:
    released(api->template_query_result_release(result));
    released(api->template_query_release(query));
    return ok && !cleanup_failed;
}

static int negotiate(void)
{
    madopilot_status_t status = madopilot_get_api(MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR, sizeof(madopilot_api_t), &api);
    if (!call(status)) return 0;
    if (!require(api != NULL && api->abi_major == 1 && api->abi_minor >= 6 &&
                 api->struct_size >= MADOPILOT_API_SIZE_TEMPLATE_QUERY_REQUIRED &&
                 sizeof(madopilot_api_t) >= MADOPILOT_API_SIZE_TEMPLATE_QUERY_REQUIRED, "abi_extent_unavailable")) return 0;
    return require(api->clock_now && api->engine_create && api->engine_release && api->engine_capabilities &&
        api->engine_permission && api->engine_template_scheduler_descriptor && api->engine_discover &&
        api->target_list_count && api->target_list_get && api->target_list_release && api->session_open &&
        api->session_describe && api->session_close && api->session_is_closed && api->session_release &&
        api->session_acquire_frame && api->package_load && api->package_release && api->template_prepare_from_package &&
        api->template_describe && api->template_release && api->session_start_template_watch &&
        api->template_query_retain && api->template_query_release && api->template_query_poll &&
        api->template_query_wait && api->template_query_cancel && api->template_query_result_retain &&
        api->template_query_result_release && api->template_query_result_info && api->template_query_result_match_at &&
        api->template_query_result_frame && api->template_query_result_error && api->frame_retain && api->frame_release &&
        api->frame_describe && api->frame_stamp && api->frame_map && api->mapping_describe && api->mapping_stamp &&
        api->mapping_release && api->cancellation_create && api->cancellation_cancel && api->cancellation_release &&
        api->error_release && api->error_describe, "abi_lifecycle_unavailable");
}

int main(int argc, char **argv)
{
    const char *rows[] = { "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9" };
    native_scope scope;
    geometry_authority geometry;
    retained_match held;
    madopilot_template_query_t *query = NULL;
    madopilot_template_query_snapshot_t snapshot;
    mpw_token token;
    uint64_t ready_nanos = 0;
    size_t completed = 0, index;
    int ok = 0, unavailable = 0, negotiated = 0, resource_ok = 0;
    const char *failed_reason;
    madopilot_status_t failed_status;
    memset(&scope, 0, sizeof(scope)); memset(&geometry, 0, sizeof(geometry)); memset(&held, 0, sizeof(held));
    if (setlocale(LC_ALL, "C") == NULL) return 2;
    check_only = argc == 2 && strcmp(argv[1], "--check") == 0;
    if (!check_only && (argc != 4 || strcmp(argv[1], "--native") != 0 || strcmp(argv[2], "--title") != 0 ||
                        strlen(argv[3]) > 1024 || !mpw_utf8_payload(argv[3]))) {
        fputs("usage: native-template-watch-c --check | --native --title <owned fixture title>\n", stderr);
        return 2;
    }
    if (!check_only && !mpw_library((const void *)(uintptr_t)&madopilot_get_api)) return 1;
    TRY(negotiate()); negotiated = 1;
    if (check_only) {
        printf("CHECK abi %" PRIu32 " %" PRIu32 " table %" PRIu32 "\n", api->abi_major, api->abi_minor, api->struct_size);
        ok = admit(&scope, NULL, 1);
        goto cleanup;
    }
    TRY(admit(&scope, argv[3], 1));
    TRY(resource_prelude(&scope, argv[3]));
    TRY(admit(&scope, argv[3], 0));
    TRY(require(mpw_row(row, "PASS", "admission_observed"), "protocol_failed")); ++completed;
    row = rows[completed];
    TRY(require(mpw_visual(0, &token), "protocol_failed"));
    TRY(bootstrap(&scope, NULL, &geometry));
    TRY(observe_token(&scope, &geometry, &token));
    ready_nanos = geometry.observed_nanos;
    TRY(require(ready_nanos >= scope.opened_nanos, "clock_overflow"));
    TRY(fact("startup_nanos", "%" PRIu64, ready_nanos - scope.opened_nanos));
    TRY(start_query(&scope, &geometry.shape, 0, &query));
    TRY(pending(query, &geometry.stamp, 0, &snapshot));
    TRY(require(mpw_row(row, "PASS", "absent_completed_pending"), "protocol_failed")); ++completed;
    row = rows[completed];
    TRY(require(mpw_visual(1, &token), "protocol_failed"));
    TRY(collect_match(&scope, query, &geometry, &token, &held));
    TRY(immutable_cancel(query, MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED));
    released(api->template_query_release(query)); query = NULL;
    TRY(require(mpw_row(row, "PASS", "exact_match_correlated"), "protocol_failed")); ++completed;
    row = rows[completed];
    TRY(geometry_row(&scope, &geometry, 0, &unavailable));
    TRY(require(mpw_row(row, unavailable ? "UNEXECUTED" : "PASS", unavailable ? "resize_unavailable" : "resize_correlated"), "protocol_failed")); ++completed;
    row = rows[completed];
    TRY(geometry_row(&scope, &geometry, 1, &unavailable));
    TRY(require(mpw_row(row, unavailable ? "UNEXECUTED" : "PASS", unavailable ? "topology_unavailable" : "movement_correlated"), "protocol_failed")); ++completed;
    row = rows[completed];
    TRY(independent_authority(&scope, &geometry));
    TRY(require(mpw_row(row, "PASS", "independent_authority"), "protocol_failed")); ++completed;
    row = rows[completed];
    TRY(retained_ownership(&scope, argv[3], &geometry, &held));
    TRY(require(mpw_row(row, "PASS", "retained_ownership"), "protocol_failed")); ++completed;
    drop_match(&held);
    row = rows[completed];
    TRY(lifecycle_terminals(&scope, argv[3], &geometry));
    TRY(require(mpw_row(row, "PASS", "lifecycle_terminals"), "protocol_failed")); ++completed;
    ok = 1;
cleanup:
    failed_reason = fault; failed_status = last_status;
    if (negotiated) {
        released(api->template_query_release(query));
        drop_match(&held);
        (void)close_scope(&scope);
    }
    if (check_only) {
        printf("CHECK status %" PRId32 " cleanup %d native_rows 0\n", failed_status, !cleanup_failed);
        return ok && !cleanup_failed ? 0 : 1;
    }
    if (!ok && completed < 8) {
        row = rows[completed];
        (void)fact("status", "%" PRId32 " 0", failed_status);
        if (!mpw_row(row, strcmp(failed_reason, "capture_permission_not_granted") == 0 ? "UNEXECUTED" :
                         strcmp(failed_reason, "resource_snapshot_failed") == 0 ? "INFRA" :
                         strcmp(failed_reason, "target_capability_unavailable") == 0 ||
                         (strcmp(failed_reason, "public_call_failed") == 0 && failed_status == MADOPILOT_STATUS_UNSUPPORTED) ?
                         "UNSUPPORTED" : "FAIL", failed_reason)) return 1;
        ++completed;
    }
    for (index = completed; index < 8; ++index) {
        if (!mpw_row(rows[index], "UNEXECUTED", "prior_row_failed")) return 1;
    }
    row = "F9";
    resource_ok = resource_fact(2, 4);
    if (!fact("status", "%" PRId32 " 0", cleanup_status) ||
        !mpw_row(row, !resource_ok ? "INFRA" : cleanup_failed ? "FAIL" : "PASS",
                 !resource_ok ? "resource_snapshot_failed" : cleanup_failed ? "consumer_cleanup_failed" : "consumer_cleanup") ||
        mpw_action("DONE") != 1) return 1;
    return 0;
}
