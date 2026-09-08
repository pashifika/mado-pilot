/* Capture-free C ABI double for the actual C/C++ native consumers.
 * Only inputs, clocks, and owned outputs are synthesized here; every readiness,
 * deadline, and result-correlation decision remains in the included consumer. */
#ifndef MADOPILOT_NATIVE_WATCH_CONSUMER_DOUBLE_H
#define MADOPILOT_NATIVE_WATCH_CONSUMER_DOUBLE_H

#include "madopilot/madopilot.h"
#include "../../examples/common/native-watch-protocol.h"

#define MPWT_INIT(value) do { memset(&(value), 0, sizeof(value)); (value).struct_size = (uint32_t)sizeof(value); } while (0)
#define MPWT_WIDTH 96u
#define MPWT_HEIGHT 64u
#define MPWT_STRIDE (MPWT_WIDTH * 4u + 16u)
#define MPWT_START UINT64_C(1000000000)
#define MPWT_END (MPWT_START + UINT64_C(5000000000))

enum mpwt_scene {
    MPWT_ABSENT, MPWT_VISIBLE, MPWT_PARTIAL, MPWT_LOW_CONTRAST,
    MPWT_SEPARATION_32, MPWT_TOLERANCE_8, MPWT_TOLERANCE_9, MPWT_OFFSET_MARKER
};

static struct {
    uint64_t now;
    unsigned acquisitions, maps, visuals, pauses;
    unsigned map_timeouts, acquire_timeouts;
    madopilot_status_t map_failure, describe_failure;
    int broken, expired_operation, mapping_identity_mismatch;
    mpw_shape shape;
    mpw_token token;
    madopilot_frame_stamp_t required, source;
    madopilot_frame_info_t frame_info;
    madopilot_image_t image;
    madopilot_template_query_result_info_t result_info;
    uint8_t pixels[MPWT_HEIGHT * MPWT_STRIDE];
} mpwt;

/* Finite, typed handles: a second acquire while the preceding attempt still
 * owns a frame/mapping is an observable ownership violation in this schedule. */
#define MPWT_OWNER(name) \
    struct madopilot_##name##_t { unsigned references; }; \
    static madopilot_##name##_t mpwt_##name; \
    static inline int mpwt_##name##_live(const madopilot_##name##_t *owner) { \
        if (owner != &mpwt_##name || owner->references == 0) { mpwt.broken = 1; return 0; } \
        return 1; \
    } \
    static inline madopilot_status_t mpwt_##name##_retain(const madopilot_##name##_t *owner) { \
        if (!mpwt_##name##_live(owner)) return MADOPILOT_STATUS_INVALID_ARGUMENT; \
        ++((madopilot_##name##_t *)owner)->references; \
        return MADOPILOT_STATUS_OK; \
    } \
    static inline madopilot_status_t mpwt_##name##_release(madopilot_##name##_t *owner) { \
        if (owner != NULL) { \
            if (!mpwt_##name##_live(owner)) return MADOPILOT_STATUS_INVALID_ARGUMENT; \
            --owner->references; \
        } \
        return MADOPILOT_STATUS_OK; \
    }
MPWT_OWNER(engine)
MPWT_OWNER(target_list)
MPWT_OWNER(session)
MPWT_OWNER(package)
MPWT_OWNER(template)
MPWT_OWNER(template_query)
MPWT_OWNER(template_query_result)
MPWT_OWNER(frame)
MPWT_OWNER(mapping)
MPWT_OWNER(error)
#undef MPWT_OWNER
#define MPWT_REQUIRE_OWNER(name, owner) \
    do { if (!mpwt_##name##_live(owner)) return MADOPILOT_STATUS_INVALID_ARGUMENT; } while (0)

static inline int mpwt_owners_released(void)
{
    return !mpwt.broken && mpwt_engine.references == 0 && mpwt_target_list.references == 0 &&
           mpwt_session.references == 0 && mpwt_package.references == 0 && mpwt_template.references == 0 &&
           mpwt_template_query.references == 0 && mpwt_template_query_result.references == 0 &&
           mpwt_frame.references == 0 && mpwt_mapping.references == 0 && mpwt_error.references == 0;
}

static inline madopilot_str_t mpwt_text(const char *text)
{
    madopilot_str_t out = { text, strlen(text) };
    return out;
}

static inline void mpwt_fill(uint32_t x, uint32_t y, uint32_t width, uint32_t height,
                             uint8_t red, uint8_t green, uint8_t blue)
{
    uint32_t row, column;
    for (row = y; row < y + height; ++row) {
        for (column = x; column < x + width; ++column) {
            uint8_t *pixel = mpwt.pixels + (size_t)row * MPWT_STRIDE + (size_t)column * 4u;
            pixel[0] = red; pixel[1] = green; pixel[2] = blue; pixel[3] = 255;
        }
    }
}

static inline void mpwt_scene_pixels(enum mpwt_scene scene)
{
    size_t index;
    mpwt_fill(0, 0, MPWT_WIDTH, MPWT_HEIGHT, 48, 48, 48);
    if (scene != MPWT_ABSENT) {
        /* Literal fixture marker layout: primary/secondary/primary,
         * secondary/primary/primary. These are test pixels, not a predicate. */
        static const int primary[6] = { 1, 0, 1, 0, 1, 1 };
        for (index = 0; index < 6; ++index) {
            uint8_t red = primary[index] ? 40 : 208;
            uint8_t green = primary[index] ? 72 : 184;
            uint8_t blue = primary[index] ? 104 : 24;
            if (scene == MPWT_LOW_CONTRAST && !primary[index]) { red = 71; green = 72; blue = 104; }
            if (scene == MPWT_SEPARATION_32 && !primary[index]) { red = 72; green = 72; blue = 104; }
            if (index == 5) {
                if (scene == MPWT_PARTIAL) { red = 48; green = 48; blue = 48; }
                if (scene == MPWT_TOLERANCE_8) red += 8;
                if (scene == MPWT_TOLERANCE_9) red += 9;
            }
            mpwt_fill((uint32_t)mpwt.shape.marker_x + (uint32_t)(index % 3u) * mpwt.shape.marker_cell_w +
                          (scene == MPWT_OFFSET_MARKER ? 16u : 0u),
                      (uint32_t)mpwt.shape.marker_y + (uint32_t)(index / 3u) * mpwt.shape.marker_cell_h,
                      mpwt.shape.marker_cell_w, mpwt.shape.marker_cell_h, red, green, blue);
        }
    }
    for (index = 0; index < 90; ++index) {
        int primary = mpwt.token.cells[index] == '1';
        mpwt_fill((uint32_t)mpwt.shape.token_x + (uint32_t)(index % 10u) * mpwt.shape.token_cell_w,
                  (uint32_t)mpwt.shape.token_y + (uint32_t)(index / 10u) * mpwt.shape.token_cell_h,
                  mpwt.shape.token_cell_w, mpwt.shape.token_cell_h,
                  primary ? 220 : 20, primary ? 180 : 60, primary ? 36 : 200);
    }
}

static inline void mpwt_reset(int visible, enum mpwt_scene scene)
{
    /* Literal VisualToken(0x55aa33cc) vectors, including sentinels, inverse,
     * marker bit, and checksum from testkit/src/visual_token.rs. No codec copy. */
    static const char absent[] = "101100100100110011110011000101010110101010110011000011001110101010010101010101000010111010";
    static const char present[] = "101100100100110011110011000101010110101010110011000011001110101010010101011100000010111010";
    int leaked = !mpwt_owners_released();
    memset(&mpwt, 0, sizeof(mpwt));
    mpwt.broken = leaked;
    mpwt.now = MPWT_START;
    mpwt.shape.marker_cell_w = 4; mpwt.shape.marker_cell_h = 3;
    mpwt.shape.marker_x = 7; mpwt.shape.marker_y = 8;
    mpwt.shape.token_cell_w = 3; mpwt.shape.token_cell_h = 2;
    mpwt.shape.token_x = 40; mpwt.shape.token_y = 20;
    mpwt.token.value = UINT32_C(0x55aa33cc);
    mpwt.token.visible = visible;
    memcpy(mpwt.token.cells, visible ? present : absent, sizeof(mpwt.token.cells));
    MPWT_INIT(mpwt.required);
    mpwt.required.stream = 11; mpwt.required.epoch = 2;
    mpwt.required.sequence = 40; mpwt.required.geometry = 7;
    mpwt.source = mpwt.required; mpwt.source.sequence = 41;
    MPWT_INIT(mpwt.frame_info);
    mpwt.frame_info.width = MPWT_WIDTH; mpwt.frame_info.height = MPWT_HEIGHT;
    mpwt.frame_info.stride = MPWT_STRIDE;
    mpwt.frame_info.format = MADOPILOT_PIXEL_FORMAT_RGBA8;
    mpwt.frame_info.space = MADOPILOT_SPACE_CAPTURE_PIXELS;
    mpwt.frame_info.bounds.space = MADOPILOT_SPACE_CAPTURE_PIXELS;
    mpwt.frame_info.bounds.right = MPWT_WIDTH; mpwt.frame_info.bounds.bottom = MPWT_HEIGHT;
    MPWT_INIT(mpwt.image);
    mpwt.image.width = MPWT_WIDTH; mpwt.image.height = MPWT_HEIGHT;
    mpwt.image.stride = MPWT_STRIDE;
    mpwt.image.format = MADOPILOT_PIXEL_FORMAT_RGBA8;
    mpwt.image.space = MADOPILOT_SPACE_CAPTURE_PIXELS;
    mpwt.image.region = mpwt.frame_info.bounds;
    mpwt.image.bytes.data = mpwt.pixels; mpwt.image.bytes.len = sizeof(mpwt.pixels);
    MPWT_INIT(mpwt.result_info);
    mpwt.result_info.outcome = MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED;
    mpwt.result_info.status = MADOPILOT_STATUS_OK;
    mpwt.result_info.overload = MADOPILOT_TEMPLATE_OVERLOAD_NONE;
    mpwt.result_info.query_id = 71; mpwt.result_info.target = 23;
    mpwt.result_info.source = mpwt.source;
    mpwt.result_info.match_count = 1; mpwt.result_info.confirmed_observations = 1;
    mpwt.result_info.template_id = mpwt_text("native.marker");
    mpwt.result_info.backend_id = mpwt_text("opencv-cpu");
    mpwt.result_info.backend_version = mpwt_text("4.14.0");
    MPWT_INIT(mpwt.result_info.options);
    mpwt.result_info.options.flags = MADOPILOT_MATCH_HAS_MIN_SCORE | MADOPILOT_MATCH_HAS_MAX_RESULTS |
                                     MADOPILOT_MATCH_HAS_SUPPRESSION;
    mpwt.result_info.options.min_score = 0.95;
    mpwt.result_info.options.max_results = 1;
    mpwt.result_info.options.suppression = MADOPILOT_SUPPRESSION_DROP_OVERLAPPING;
    mpwt.result_info.effective_region = mpwt.frame_info.bounds;
    MPWT_INIT(mpwt.result_info.transform);
    mpwt.result_info.transform.flags = MADOPILOT_TRANSFORM_COVERS_TARGET | MADOPILOT_TRANSFORM_HAS_TARGET_PLACEMENT;
    mpwt.result_info.transform.geometry = mpwt.source.geometry;
    mpwt.result_info.transform.width = MPWT_WIDTH; mpwt.result_info.transform.height = MPWT_HEIGHT;
    mpwt.result_info.transform.logical_width = MPWT_WIDTH; mpwt.result_info.transform.logical_height = MPWT_HEIGHT;
    mpwt.result_info.transform.target_scale_x = 1; mpwt.result_info.transform.target_scale_y = 1;
    mpwt.result_info.transform.desktop_scale_x = 1; mpwt.result_info.transform.desktop_scale_y = 1;
    mpwt_scene_pixels(scene);
}

static inline int mpwt_fact(const char *row_id, const char *key, const char *values)
{
    (void)row_id; (void)key; (void)values;
    return 1;
}

static inline int mpwt_visual(int visible, mpw_token *out)
{
    ++mpwt.visuals;
    if (visible != mpwt.token.visible) { mpwt.broken = 1; return 0; }
    *out = mpwt.token;
    return 1;
}

static inline int mpwt_geometry(uint32_t width, uint32_t height, double x, double y,
                                double logical_w, double logical_h, double scale_x, double scale_y,
                                mpw_shape *out)
{
    (void)x; (void)y; (void)logical_w; (void)logical_h; (void)scale_x; (void)scale_y;
    if (width != MPWT_WIDTH || height != MPWT_HEIGHT) { mpwt.broken = 1; return 0; }
    *out = mpwt.shape;
    return 1;
}

static inline void mpwt_pause(void)
{
    ++mpwt.pauses;
    mpwt.now = mpwt.now + UINT64_C(5000000) < MPWT_END ? mpwt.now + UINT64_C(5000000) : MPWT_END;
}

static inline madopilot_status_t mpwt_clock(uint64_t *out)
{
    *out = mpwt.now;
    return MADOPILOT_STATUS_OK;
}

static inline int mpwt_deadline(const madopilot_operation_t *operation)
{
    if (operation->flags != MADOPILOT_OPERATION_HAS_DEADLINE || operation->deadline_nanos > MPWT_END ||
        mpwt.now >= MPWT_END || operation->deadline_nanos <= mpwt.now) {
        mpwt.expired_operation = 1;
        return 0; /* Also bounds a broken consumer that restarts its deadline. */
    }
    return 1;
}

static inline madopilot_status_t mpwt_acquire(const madopilot_session_t *session,
                                             const madopilot_operation_t *operation,
                                             madopilot_frame_t **out, madopilot_error_t **error)
{
    *out = NULL; if (error != NULL) *error = NULL;
    MPWT_REQUIRE_OWNER(session, session);
    ++mpwt.acquisitions;
    if (!mpwt_deadline(operation)) return MADOPILOT_STATUS_INTERNAL;
    if (mpwt_frame.references != 0 || mpwt_mapping.references != 0) {
        mpwt.broken = 1; return MADOPILOT_STATUS_INTERNAL;
    }
    if (mpwt.acquire_timeouts != 0) {
        --mpwt.acquire_timeouts; mpwt.now = operation->deadline_nanos;
        return MADOPILOT_STATUS_DEADLINE_EXCEEDED;
    }
    mpwt_frame.references = 1; *out = &mpwt_frame;
    return MADOPILOT_STATUS_OK;
}

static inline madopilot_status_t mpwt_frame_stamp(const madopilot_frame_t *frame, madopilot_frame_stamp_t *out)
{
    memset(out, 0, sizeof(*out));
    MPWT_REQUIRE_OWNER(frame, frame);
    *out = mpwt.source; return MADOPILOT_STATUS_OK;
}
static inline madopilot_status_t mpwt_frame_describe(const madopilot_frame_t *frame, madopilot_frame_info_t *out)
{
    memset(out, 0, sizeof(*out));
    MPWT_REQUIRE_OWNER(frame, frame);
    *out = mpwt.frame_info; return MADOPILOT_STATUS_OK;
}
static inline madopilot_status_t mpwt_map(const madopilot_frame_t *frame, const madopilot_map_request_t *request,
                                         const madopilot_operation_t *operation, madopilot_mapping_t **out,
                                         madopilot_error_t **error)
{
    (void)request;
    *out = NULL; if (error != NULL) *error = NULL;
    MPWT_REQUIRE_OWNER(frame, frame);
    ++mpwt.maps;
    if (!mpwt_deadline(operation)) return MADOPILOT_STATUS_INTERNAL;
    if (mpwt.map_failure != MADOPILOT_STATUS_OK) return mpwt.map_failure;
    if (mpwt.map_timeouts != 0) {
        --mpwt.map_timeouts; mpwt.now = operation->deadline_nanos;
        return MADOPILOT_STATUS_DEADLINE_EXCEEDED;
    }
    ++mpwt_mapping.references; *out = &mpwt_mapping;
    return MADOPILOT_STATUS_OK;
}
static inline madopilot_status_t mpwt_mapping_stamp(const madopilot_mapping_t *mapping, madopilot_frame_stamp_t *out)
{
    memset(out, 0, sizeof(*out));
    MPWT_REQUIRE_OWNER(mapping, mapping);
    *out = mpwt.source;
    if (mpwt.mapping_identity_mismatch) ++out->sequence;
    return MADOPILOT_STATUS_OK;
}
static inline madopilot_status_t mpwt_mapping_describe(const madopilot_mapping_t *mapping, madopilot_image_t *out)
{
    memset(out, 0, sizeof(*out));
    MPWT_REQUIRE_OWNER(mapping, mapping);
    if (mpwt.describe_failure != MADOPILOT_STATUS_OK) return mpwt.describe_failure;
    *out = mpwt.image; return MADOPILOT_STATUS_OK;
}
static inline madopilot_status_t mpwt_result_info(const madopilot_template_query_result_t *result,
                                                 madopilot_template_query_result_info_t *out)
{
    memset(out, 0, sizeof(*out));
    MPWT_REQUIRE_OWNER(template_query_result, result);
    *out = mpwt.result_info; return MADOPILOT_STATUS_OK;
}
static inline madopilot_status_t mpwt_result_match(const madopilot_template_query_result_t *result,
                                                  size_t index, madopilot_match_t *out)
{
    memset(out, 0, sizeof(*out));
    MPWT_REQUIRE_OWNER(template_query_result, result);
    if (index != 0) return MADOPILOT_STATUS_INVALID_ARGUMENT;
    MPWT_INIT(*out);
    out->bounds.space = MADOPILOT_SPACE_CAPTURE_PIXELS;
    out->score = 0.99; out->template_id = mpwt_text("native.marker");
    out->bounds.left = mpwt.shape.marker_x; out->bounds.top = mpwt.shape.marker_y;
    out->bounds.right = mpwt.shape.marker_x + (int32_t)(3u * mpwt.shape.marker_cell_w);
    out->bounds.bottom = mpwt.shape.marker_y + (int32_t)(2u * mpwt.shape.marker_cell_h);
    return MADOPILOT_STATUS_OK;
}
static inline madopilot_status_t mpwt_result_frame(const madopilot_template_query_result_t *result, madopilot_frame_t **out)
{
    *out = NULL;
    MPWT_REQUIRE_OWNER(template_query_result, result);
    ++mpwt_frame.references; *out = &mpwt_frame; return MADOPILOT_STATUS_OK;
}
static inline madopilot_status_t mpwt_result_error(const madopilot_template_query_result_t *result, madopilot_error_t **out)
{
    *out = NULL;
    MPWT_REQUIRE_OWNER(template_query_result, result);
    return MADOPILOT_STATUS_OK;
}
static inline madopilot_status_t mpwt_error_describe(const madopilot_error_t *error, madopilot_error_detail_t *out)
{
    memset(out, 0, sizeof(*out));
    MPWT_REQUIRE_OWNER(error, error);
    MPWT_INIT(*out); return MADOPILOT_STATUS_INTERNAL;
}
static inline madopilot_status_t mpwt_poll(const madopilot_template_query_t *query,
                                          madopilot_template_query_snapshot_t *out,
                                          madopilot_template_query_result_t **result, madopilot_error_t **error)
{
    memset(out, 0, sizeof(*out)); *result = NULL; if (error != NULL) *error = NULL;
    MPWT_REQUIRE_OWNER(template_query, query);
    MPWT_INIT(*out);
    out->state = MADOPILOT_TEMPLATE_QUERY_STATE_PENDING; out->query_id = 71;
    out->flags = MADOPILOT_TEMPLATE_QUERY_HAS_LAST_FRAME; out->last_frame = mpwt.required;
    out->completed = 1; out->generation = 2;
    out->pending_count = 1; out->in_flight_count = 1;
    return MADOPILOT_STATUS_OK;
}
static inline madopilot_status_t mpwt_wait(const madopilot_template_query_t *query,
                                          const madopilot_operation_t *operation,
                                          madopilot_template_query_result_t **out, madopilot_error_t **error)
{
    (void)operation;
    *out = NULL; if (error != NULL) *error = NULL;
    MPWT_REQUIRE_OWNER(template_query, query);
    ++mpwt_template_query_result.references; *out = &mpwt_template_query_result;
    return MADOPILOT_STATUS_OK;
}
static inline madopilot_status_t mpwt_cancel(const madopilot_template_query_t *query,
                                            madopilot_template_query_result_t **out, madopilot_error_t **error)
{
    return mpwt_wait(query, NULL, out, error);
}

static inline madopilot_status_t mpwt_create(const madopilot_source_t *source, const madopilot_operation_t *operation,
                                            madopilot_engine_t **out, madopilot_error_t **error)
{
    (void)source; (void)operation;
    ++mpwt_engine.references; *out = &mpwt_engine; if (error != NULL) *error = NULL;
    return MADOPILOT_STATUS_OK;
}
static inline madopilot_status_t mpwt_discover(const madopilot_engine_t *engine, const madopilot_operation_t *operation,
                                              madopilot_target_list_t **out, madopilot_error_t **error)
{
    (void)operation;
    *out = NULL; if (error != NULL) *error = NULL;
    MPWT_REQUIRE_OWNER(engine, engine);
    ++mpwt_target_list.references; *out = &mpwt_target_list;
    return MADOPILOT_STATUS_OK;
}
static inline madopilot_status_t mpwt_open(const madopilot_engine_t *engine, const madopilot_target_list_t *targets,
                                          size_t index, const madopilot_open_request_t *request,
                                          const madopilot_operation_t *operation, madopilot_session_t **out,
                                          madopilot_error_t **error)
{
    (void)index; (void)request; (void)operation;
    *out = NULL; if (error != NULL) *error = NULL;
    MPWT_REQUIRE_OWNER(engine, engine);
    MPWT_REQUIRE_OWNER(target_list, targets);
    ++mpwt_session.references; *out = &mpwt_session;
    return MADOPILOT_STATUS_OK;
}
static inline madopilot_status_t mpwt_load_package(const madopilot_engine_t *engine,
                                                  const madopilot_package_source_t *source,
                                                  const madopilot_operation_t *operation,
                                                  madopilot_package_t **out, madopilot_error_t **error)
{
    (void)source; (void)operation;
    *out = NULL; if (error != NULL) *error = NULL;
    MPWT_REQUIRE_OWNER(engine, engine);
    ++mpwt_package.references; *out = &mpwt_package;
    return MADOPILOT_STATUS_OK;
}
static inline madopilot_status_t mpwt_prepare(const madopilot_engine_t *engine, const madopilot_package_t *package,
                                             madopilot_str_t id, const madopilot_operation_t *operation,
                                             madopilot_template_t **out, madopilot_error_t **error)
{
    (void)id; (void)operation;
    *out = NULL; if (error != NULL) *error = NULL;
    MPWT_REQUIRE_OWNER(engine, engine);
    MPWT_REQUIRE_OWNER(package, package);
    ++mpwt_template.references; *out = &mpwt_template;
    return MADOPILOT_STATUS_OK;
}
static inline madopilot_status_t mpwt_start(const madopilot_session_t *session, const madopilot_template_t *marker,
                                           const madopilot_template_watch_options_t *options,
                                           const madopilot_operation_t *operation,
                                           madopilot_template_query_t **out, madopilot_error_t **error)
{
    (void)options; (void)operation;
    *out = NULL; if (error != NULL) *error = NULL;
    MPWT_REQUIRE_OWNER(session, session);
    MPWT_REQUIRE_OWNER(template, marker);
    ++mpwt_template_query.references; *out = &mpwt_template_query;
    return MADOPILOT_STATUS_OK;
}

static inline const madopilot_api_t *mpwt_api(void)
{
    static madopilot_api_t table;
    memset(&table, 0, sizeof(table));
    table.struct_size = (uint32_t)sizeof(table); table.abi_major = 1; table.abi_minor = 6;
    table.clock_now = mpwt_clock;
#define MPWT_LIFECYCLE(name) table.name##_retain = mpwt_##name##_retain; table.name##_release = mpwt_##name##_release
    MPWT_LIFECYCLE(engine); MPWT_LIFECYCLE(target_list); MPWT_LIFECYCLE(session);
    MPWT_LIFECYCLE(package); MPWT_LIFECYCLE(template); MPWT_LIFECYCLE(template_query);
    MPWT_LIFECYCLE(template_query_result); MPWT_LIFECYCLE(frame); MPWT_LIFECYCLE(mapping); MPWT_LIFECYCLE(error);
#undef MPWT_LIFECYCLE
    table.engine_create = mpwt_create; table.engine_discover = mpwt_discover; table.session_open = mpwt_open;
    table.package_load = mpwt_load_package; table.template_prepare_from_package = mpwt_prepare;
    table.session_start_template_watch = mpwt_start;
    table.session_acquire_frame = mpwt_acquire;
    table.frame_stamp = mpwt_frame_stamp; table.frame_describe = mpwt_frame_describe; table.frame_map = mpwt_map;
    table.mapping_stamp = mpwt_mapping_stamp; table.mapping_describe = mpwt_mapping_describe;
    table.error_describe = mpwt_error_describe;
    table.template_query_poll = mpwt_poll; table.template_query_wait = mpwt_wait; table.template_query_cancel = mpwt_cancel;
    table.template_query_result_info = mpwt_result_info; table.template_query_result_match_at = mpwt_result_match;
    table.template_query_result_frame = mpwt_result_frame; table.template_query_result_error = mpwt_result_error;
    return &table;
}

#endif
