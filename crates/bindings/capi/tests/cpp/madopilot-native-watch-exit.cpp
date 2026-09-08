// A semantic FAIL belongs to the ledger; an incomplete exchange fails the process.
#include <array>
#include <cstdio>
#include <string>

#include "madopilot/madopilot.h"
#include "../../examples/common/native-watch-protocol.h"
#include "../support/native-watch-consumer-double.h"

namespace exit_test {
struct Row {
    std::string id;
    std::string outcome;
};
std::array<Row, 9> rows;
std::size_t count;
bool refuse_api;
bool reject_row;
bool reject_done;
bool done;
bool consumer_contract;

int row(const char* id, const char* outcome, const char*)
{
    if (count == rows.size()) return 0;
    rows[count++] = {id, outcome};
    return !reject_row;
}
int action(const char* command)
{
    if (std::string(command) != "DONE") return 0;
    done = true;
    return !reject_done;
}
int library(const void*) { return 1; }
int fact(const char*, const char*, const char*) { return 1; }
int resources(mpw_resource_snapshot* out)
{
    *out = {};
    return 1;
}
madopilot_status_t create_engine(const madopilot_source_t*, const madopilot_operation_t*,
                                madopilot_engine_t** engine, madopilot_error_t** error)
{
    *engine = nullptr;
    if (error != nullptr) *error = nullptr;
    return MADOPILOT_STATUS_INTERNAL;
}
} // namespace exit_test

extern "C" madopilot_status_t exit_test_get_api(uint32_t major, uint32_t minor,
                                                size_t size, const madopilot_api_t** out)
{
    if (exit_test::consumer_contract) {
        *out = mpwt_api();
        return MADOPILOT_STATUS_OK;
    }
    if (exit_test::refuse_api) {
        *out = nullptr;
        return MADOPILOT_STATUS_INTERNAL;
    }
    const madopilot_api_t* released = nullptr;
    const auto status = madopilot_get_api(major, minor, size, &released);
    if (status != MADOPILOT_STATUS_OK) return status;
    static madopilot_api_t table;
    table = *released;
    table.engine_create = exit_test::create_engine;
    *out = &table;
    return MADOPILOT_STATUS_OK;
}

#define madopilot_get_api exit_test_get_api
#define mpw_library exit_test::library
#define mpw_fact exit_test::fact
#define mpw_row exit_test::row
#define mpw_action exit_test::action
#define mpw_resources exit_test::resources
#define mpw_visual mpwt_visual
#define mpw_geometry mpwt_geometry
#define mpw_pause mpwt_pause
#define main native_consumer_main
#include "../../examples/cpp/native-template-watch.cpp"
#undef main
#undef mpw_pause
#undef mpw_geometry
#undef mpw_visual
#undef mpw_resources
#undef mpw_action
#undef mpw_row
#undef mpw_fact
#undef mpw_library
#undef madopilot_get_api

static bool readiness_contract()
{
    madopilot::FrameStamp required{};
    required.stream = 1;
    required.sequence = 5;
    madopilot::TemplateQuerySnapshot snapshot{};
    snapshot.flags = MADOPILOT_TEMPLATE_QUERY_HAS_LAST_FRAME;
    snapshot.last_frame = required;
    snapshot.completed = 1;
    snapshot.generation = 2;
    snapshot.pending_count = 1;
    snapshot.in_flight_count = 1;
    if (!pending_ready(snapshot, required, 0, 0)) return false;
    if (pending_ready(snapshot, required, 1, 0)) return false;
    if (pending_ready(snapshot, required, 0, 2)) return false;
    snapshot.completed = 0;
    if (pending_ready(snapshot, required, 0, 0)) return false;
    snapshot.completed = 1;
    snapshot.flags = 0;
    if (pending_ready(snapshot, required, 0, 0)) return false;
    snapshot.flags = MADOPILOT_TEMPLATE_QUERY_HAS_LAST_FRAME;
    required.geometry = 1;
    if (pending_ready(snapshot, required, 0, 0)) return false;
    required.geometry = 0;
    required.sequence = 6;
    if (pending_ready(snapshot, required, 0, 0)) return false;
    snapshot.last_frame.sequence = required.sequence;
    return pending_ready(snapshot, required, 0, 0);
}

namespace {

// The private qualification apparatus is the unit under test. Its public C++
// owners are created through their real factories over the controlled C table.
struct QualificationContract {
    static bool setup(Qualification& q, bool with_query)
    {
        madopilot::Operation op;
        op.deadline(MPWT_END);
        auto created = q.api_.create_engine(madopilot::Source::native_macos(), op);
        if (!created) return false;
        q.engine_ = created.take();
        auto discovered = q.engine_.discover(op);
        if (!discovered) return false;
        auto targets = discovered.take();
        auto opened = q.engine_.open_session(targets, 0, madopilot::OpenRequest(), op);
        if (!opened) return false;
        q.session_ = opened.take();
        q.geometry_.source = mpwt.required;
        q.geometry_.source.sequence = 10; // Bootstrap precedes the absent observation.
        q.geometry_.transform = mpwt.result_info.transform;
        q.geometry_.shape = mpwt.shape;
        q.seen_ = mpwt.required;
        q.target_ = 23;
        q.query_id_ = 71;
        if (!q.backend_version(madopilot::BorrowedStr(mpwt_text("4.14.0")))) return false;
        if (!with_query) return true;
        auto loaded = q.engine_.load_package(madopilot::PackageSource::directory("unused-test-double"), op);
        if (!loaded) return false;
        auto package = loaded.take();
        auto prepared = q.engine_.prepare_from_package(package, "native.marker", op);
        if (!prepared) return false;
        auto marker = prepared.take();
        auto started = q.session_.start_template_watch(marker, madopilot::TemplateWatchOptions(), op);
        if (!started) return false;
        q.query_ = started.take();
        return true;
    }

    static bool scene(const char* name, int visible, mpwt_scene physical, bool expected)
    {
        mpwt_reset(visible, physical);
        auto api = madopilot::Api::load();
        if (!api) return false;
        bool valid = false;
        {
            Qualification q(api.take(), "unused-test-double");
            if (!setup(q, false)) return false;
            const bool observed = q.observe(q.geometry_, mpwt.token, q.seen_);
            valid = observed == expected && mpwt.visuals == 0 && !mpwt.expired_operation;
            if (expected) {
                valid = valid && same_stamp(q.seen_, mpwt.source) && q.observed_at_ < MPWT_END &&
                        q.reason_ == nullptr && q.status_ == MADOPILOT_STATUS_OK;
            } else {
                valid = valid && mpwt.now == MPWT_END && same_stamp(q.seen_, mpwt.required) &&
                        q.reason_ != nullptr;
            }
        }
        valid = valid && mpwt_owners_released();
        if (!valid) std::fprintf(stderr, "C++ scene contract failed: %s\n", name);
        return valid;
    }

    static bool mapping(unsigned scenario)
    {
        mpwt_reset(0, MPWT_ABSENT);
        if (scenario == 0) mpwt.map_timeouts = 1;
        if (scenario == 1) mpwt.map_timeouts = UINT_MAX;
        if (scenario == 2) mpwt.map_failure = MADOPILOT_STATUS_INTERNAL;
        if (scenario == 3) mpwt.acquire_timeouts = 1;
        if (scenario == 4) mpwt.mapping_identity_mismatch = 1;
        // A metadata accessor reporting a deadline is not a map-slice expiry.
        if (scenario == 5) mpwt.describe_failure = MADOPILOT_STATUS_DEADLINE_EXCEEDED;
        auto api = madopilot::Api::load();
        if (!api) return false;
        bool valid = false;
        {
            Qualification q(api.take(), "unused-test-double");
            if (!setup(q, false)) return false;
            const bool observed = q.observe(q.geometry_, mpwt.token, q.seen_);
            valid = mpwt.visuals == 0 && !mpwt.expired_operation;
            if (scenario == 0 || scenario == 3) {
                valid = valid && observed && mpwt.maps == (scenario == 0 ? 2u : 1u) &&
                        mpwt.acquisitions == 2 && same_stamp(q.seen_, mpwt.source) &&
                        q.reason_ == nullptr && q.status_ == MADOPILOT_STATUS_OK;
            } else if (scenario == 1) {
                valid = valid && !observed && mpwt.maps > 1 && mpwt.now == MPWT_END &&
                        same_stamp(q.seen_, mpwt.required) && q.reason_ != nullptr;
            } else {
                valid = valid && !observed && mpwt.maps == 1 && mpwt.acquisitions == 1 &&
                        mpwt.now < MPWT_END && q.reason_ != nullptr;
                if (scenario == 2) valid = valid && q.status_ == MADOPILOT_STATUS_INTERNAL;
                if (scenario == 5) valid = valid && q.status_ == MADOPILOT_STATUS_DEADLINE_EXCEEDED;
            }
        }
        valid = valid && mpwt_owners_released();
        if (!valid) std::fprintf(stderr, "C++ mapping observation contract failed: %u\n", scenario);
        return valid;
    }

    static bool correlate(unsigned scenario)
    {
        mpwt_reset(1, MPWT_VISIBLE);
        if (scenario == 1) ++mpwt.result_info.query_id; // Wrong, nonzero, otherwise self-consistent.
        if (scenario == 2) {
            mpwt.source.sequence = mpwt.required.sequence - 1;
            mpwt.result_info.source = mpwt.source; // Newer than bootstrap, older than observation.
        }
        if (scenario == 3) mpwt.map_timeouts = 1;
        if (scenario == 4) mpwt.mapping_identity_mismatch = 1;
        if (scenario == 5) mpwt.image.bytes.len = 4;
        if (scenario == 6) ++mpwt.result_info.source.sequence;
        if (scenario == 7) {
            ++mpwt.source.epoch;
            mpwt.result_info.source = mpwt.source;
        }
        auto api = madopilot::Api::load();
        if (!api) return false;
        bool valid = false;
        {
            Qualification q(api.take(), "unused-test-double");
            if (!setup(q, true)) return false;
            const bool matched = q.first_match();
            valid = matched == (scenario == 0) && mpwt.acquisitions == 0 && mpwt.visuals == 1;
            if (scenario == 0) {
                valid = valid && q.retained_.info.query_id == 71 && q.verify_retained() &&
                        same_stamp(q.retained_.info.source, mpwt.source);
                ++mpwt.result_info.query_id;
                valid = valid && !q.verify_retained();
            }
            if (scenario == 1 || scenario == 2 || scenario == 6 || scenario == 7) valid = valid && mpwt.maps == 0;
            if (scenario == 3) valid = valid && mpwt.maps == 1 && q.status_ == MADOPILOT_STATUS_DEADLINE_EXCEEDED;
        }
        valid = valid && mpwt_owners_released();
        if (!valid) std::fprintf(stderr, "C++ retained correlation contract failed: %u\n", scenario);
        return valid;
    }

    static bool run()
    {
        exit_test::consumer_contract = true;
        bool passed = true;
        passed &= scene("valid_absent", 0, MPWT_ABSENT, true);
        passed &= scene("valid_visible", 1, MPWT_VISIBLE, true);
        passed &= scene("absent_token_visible_marker", 0, MPWT_VISIBLE, false);
        passed &= scene("visible_token_absent_marker", 1, MPWT_ABSENT, false);
        passed &= scene("partial_marker", 0, MPWT_PARTIAL, false);
        passed &= scene("ambiguous_low_contrast", 1, MPWT_LOW_CONTRAST, false);
        passed &= scene("marker_separation_boundary", 1, MPWT_SEPARATION_32, true);
        passed &= scene("marker_tolerance_boundary", 1, MPWT_TOLERANCE_8, true);
        passed &= scene("marker_tolerance_exceeded", 1, MPWT_TOLERANCE_9, false);
        passed &= scene("marker_outside_authoritative_origin", 1, MPWT_OFFSET_MARKER, false);
        for (unsigned scenario = 0; scenario < 6; ++scenario) passed &= mapping(scenario);
        for (unsigned scenario = 0; scenario < 8; ++scenario) passed &= correlate(scenario);
        return passed;
    }
};

} // namespace

int main()
{
    char name[] = "native-consumer";
    char native[] = "--native";
    char title_option[] = "--title";
    char title[] = "unused-no-target-opened";
    char* arguments[] = {name, native, title_option, title};
    bool passed = true;
    for (unsigned scenario = 0; scenario < 4; ++scenario) {
        exit_test::count = 0;
        exit_test::done = false;
        exit_test::refuse_api = scenario == 1;
        exit_test::reject_done = scenario == 2;
        exit_test::reject_row = scenario == 3;
        const int exit = native_consumer_main(4, arguments);
        const bool complete = scenario < 3;
        bool valid = (exit == 0) == (scenario < 2) &&
                     exit_test::count == (complete ? 9 : 1) &&
                     exit_test::done == complete &&
                     exit_test::rows[0].id == "F1" && exit_test::rows[0].outcome == "FAIL";
        if (complete) {
            for (std::size_t index = 1; index < 8; ++index) {
                valid = valid && exit_test::rows[index].id == "F" + std::to_string(index + 1) &&
                        exit_test::rows[index].outcome == "UNEXECUTED";
            }
            valid = valid && exit_test::rows[8].id == "F9" && exit_test::rows[8].outcome == "PASS";
        }
        if (!valid) std::fprintf(stderr, "native exit scenario %u failed: exit=%d rows=%zu done=%d\n",
                                 scenario, exit, exit_test::count, exit_test::done);
        passed = passed && valid;
    }
    passed &= QualificationContract::run();
    if (!readiness_contract()) {
        std::fputs("native busy progress readiness contract failed\n", stderr);
        return 1;
    }
    return passed ? 0 : 1;
}
