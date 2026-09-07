// A semantic FAIL belongs to the ledger; an incomplete exchange fails the process.
#include <array>
#include <cstdio>
#include <string>

#include "madopilot/madopilot.h"
#include "../../examples/common/native-watch-protocol.h"

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
#define main native_consumer_main
#include "../../examples/cpp/native-template-watch.cpp"
#undef main
#undef mpw_resources
#undef mpw_action
#undef mpw_row
#undef mpw_fact
#undef mpw_library
#undef madopilot_get_api

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
    return passed ? 0 : 1;
}
