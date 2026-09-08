/* Exercise the included native consumer without capture or a controller. */
#include "../support/native-watch-consumer-double.h"
#define mpw_fact mpwt_fact
#define mpw_visual mpwt_visual
#define mpw_geometry mpwt_geometry
#define mpw_pause mpwt_pause
#define main native_consumer_main
#include "../../examples/c/native-template-watch.c"
#undef main
#undef mpw_pause
#undef mpw_geometry
#undef mpw_visual
#undef mpw_fact

static int busy_progress_contract(void)
{
    madopilot_template_query_snapshot_t snapshot;
    madopilot_frame_stamp_t required;
    INIT(snapshot); INIT(required);
    required.stream = 1;
    required.sequence = 5;
    snapshot.flags = MADOPILOT_TEMPLATE_QUERY_HAS_LAST_FRAME;
    snapshot.last_frame = required;
    snapshot.completed = 1;
    snapshot.pending_count = 1;
    snapshot.in_flight_count = 1;
    if (!pending_ready(&snapshot, &required, 0)) {
        fputs("completed busy progress was not ready\n", stderr);
        return 1;
    }
    if (pending_ready(&snapshot, &required, 1)) return 2;
    snapshot.completed = 0;
    if (pending_ready(&snapshot, &required, 0)) return 3;
    snapshot.completed = 1;
    snapshot.flags = 0;
    if (pending_ready(&snapshot, &required, 0)) return 4;
    snapshot.flags = MADOPILOT_TEMPLATE_QUERY_HAS_LAST_FRAME;
    required.geometry = 1;
    if (pending_ready(&snapshot, &required, 0)) return 5;
    required.geometry = 0;
    required.sequence = 6;
    if (pending_ready(&snapshot, &required, 0)) return 6;
    /* Authoritative unchanged pixels can advance last_frame without new work. */
    snapshot.last_frame.sequence = required.sequence;
    if (!pending_ready(&snapshot, &required, 0)) return 7;
    return 0;
}

static void reset_consumer(int visible, enum mpwt_scene scene)
{
    mpwt_reset(visible, scene);
    api = mpwt_api();
    fault = "contract_mismatch";
    last_status = cleanup_status = MADOPILOT_STATUS_OK;
    cleanup_failed = 0;
}

static geometry_authority required_geometry(void)
{
    geometry_authority geometry;
    memset(&geometry, 0, sizeof(geometry));
    geometry.stamp = mpwt.required;
    geometry.transform = mpwt.result_info.transform;
    geometry.shape = mpwt.shape;
    return geometry;
}

static int scene_observation(const char *name, int visible, enum mpwt_scene scene, int expected)
{
    native_scope scope;
    geometry_authority geometry;
    int observed, valid;
    reset_consumer(visible, scene);
    memset(&scope, 0, sizeof(scope));
    scope.session = &mpwt_session; scope.target = 23;
    mpwt_session.references = 1;
    geometry = required_geometry();
    observed = observe_token(&scope, &geometry, &mpwt.token);
    (void)api->session_release(scope.session);
    valid = observed == expected && mpwt.visuals == 0 && !mpwt.expired_operation &&
            mpwt_owners_released() && !cleanup_failed;
    if (expected) valid = valid && stamp_equal(&geometry.stamp, &mpwt.source) &&
                                  geometry.observed_nanos < MPWT_END;
    else valid = valid && mpwt.now == MPWT_END && stamp_equal(&geometry.stamp, &mpwt.required);
    if (!valid) fprintf(stderr, "C scene contract failed: %s\n", name);
    return valid;
}

static int mapping_observation(unsigned scenario)
{
    native_scope scope;
    geometry_authority geometry;
    int observed, valid;
    reset_consumer(0, MPWT_ABSENT);
    memset(&scope, 0, sizeof(scope));
    scope.session = &mpwt_session; scope.target = 23;
    mpwt_session.references = 1;
    geometry = required_geometry();
    if (scenario == 0) mpwt.map_timeouts = 1;
    if (scenario == 1) mpwt.map_timeouts = UINT_MAX;
    if (scenario == 2) mpwt.map_failure = MADOPILOT_STATUS_INTERNAL;
    if (scenario == 3) mpwt.acquire_timeouts = 1;
    if (scenario == 4) mpwt.mapping_identity_mismatch = 1;
    observed = observe_token(&scope, &geometry, &mpwt.token);
    (void)api->session_release(scope.session);
    valid = mpwt.visuals == 0 && !mpwt.expired_operation && mpwt_owners_released() && !cleanup_failed;
    if (scenario == 0) valid = valid && observed && mpwt.maps == 2 && mpwt.acquisitions == 2 &&
                                      stamp_equal(&geometry.stamp, &mpwt.source) && last_status == MADOPILOT_STATUS_OK;
    if (scenario == 1) valid = valid && !observed && mpwt.maps > 1 && mpwt.now == MPWT_END &&
                                      stamp_equal(&geometry.stamp, &mpwt.required);
    if (scenario == 2 || scenario == 4) valid = valid && !observed && mpwt.maps == 1 &&
                                                           mpwt.acquisitions == 1 && mpwt.now < MPWT_END;
    if (scenario == 3) valid = valid && observed && mpwt.maps == 1 && mpwt.acquisitions == 2;
    if (!valid) fprintf(stderr, "C mapping observation contract failed: %u\n", scenario);
    return valid;
}

static int retained_correlation(unsigned scenario)
{
    native_scope scope;
    geometry_authority geometry;
    retained_match held;
    int matched, valid;
    reset_consumer(1, MPWT_VISIBLE);
    memset(&scope, 0, sizeof(scope)); memset(&held, 0, sizeof(held));
    scope.target = 23;
    geometry = required_geometry();
    if (scenario == 1) ++mpwt.result_info.query_id;
    if (scenario == 2) {
        mpwt.source.sequence = mpwt.required.sequence - 1;
        mpwt.result_info.source = mpwt.source;
    }
    if (scenario == 3) mpwt.map_timeouts = 1;
    if (scenario == 4) mpwt.mapping_identity_mismatch = 1;
    if (scenario == 5) mpwt.image.bytes.len = 4;
    if (scenario == 6) ++mpwt.result_info.source.sequence;
    if (scenario == 7) {
        ++mpwt.source.epoch;
        mpwt.result_info.source = mpwt.source;
    }
    mpwt_template_query.references = 1;
    matched = collect_match(&scope, &mpwt_template_query, &geometry, &mpwt.token, &held);
    (void)api->template_query_release(&mpwt_template_query);
    valid = matched == (scenario == 0) && mpwt.acquisitions == 0 && mpwt.visuals == 0;
    if (scenario == 0) {
        valid = valid && held.info.query_id == 71 && stamp_equal(&held.info.source, &mpwt.source) &&
                retained_readable(&held);
        ++mpwt.result_info.query_id;
        valid = valid && !retained_readable(&held);
    }
    if (scenario == 1 || scenario == 2 || scenario == 6 || scenario == 7) valid = valid && mpwt.maps == 0;
    if (scenario == 3) valid = valid && mpwt.maps == 1 && last_status == MADOPILOT_STATUS_DEADLINE_EXCEEDED;
    drop_match(&held);
    valid = valid && mpwt_owners_released() && !cleanup_failed;
    if (!valid) fprintf(stderr, "C retained correlation contract failed: %u\n", scenario);
    return valid;
}

int main(void)
{
    unsigned scenario;
    int passed = busy_progress_contract() == 0;
    passed &= scene_observation("valid_absent", 0, MPWT_ABSENT, 1);
    passed &= scene_observation("valid_visible", 1, MPWT_VISIBLE, 1);
    passed &= scene_observation("absent_token_visible_marker", 0, MPWT_VISIBLE, 0);
    passed &= scene_observation("visible_token_absent_marker", 1, MPWT_ABSENT, 0);
    passed &= scene_observation("partial_marker", 0, MPWT_PARTIAL, 0);
    passed &= scene_observation("ambiguous_low_contrast", 1, MPWT_LOW_CONTRAST, 0);
    passed &= scene_observation("marker_separation_boundary", 1, MPWT_SEPARATION_32, 1);
    passed &= scene_observation("marker_tolerance_boundary", 1, MPWT_TOLERANCE_8, 1);
    passed &= scene_observation("marker_tolerance_exceeded", 1, MPWT_TOLERANCE_9, 0);
    passed &= scene_observation("marker_outside_authoritative_origin", 1, MPWT_OFFSET_MARKER, 0);
    for (scenario = 0; scenario < 5; ++scenario) passed &= mapping_observation(scenario);
    for (scenario = 0; scenario < 8; ++scenario) passed &= retained_correlation(scenario);
    return passed ? 0 : 1;
}
