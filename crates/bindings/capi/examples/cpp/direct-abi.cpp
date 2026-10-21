// Direct use of the C ABI from C++17: the same declarations, statuses, and
// handle lifetime as C. Ownership and error policy belong to the application;
// this sample releases its one handle explicitly.
//
// Needs only the built library: no model, asset, window, capture, input, or
// permission.

#include <algorithm>
#include <cstddef>
#include <cstdint>
#include <cstdio>

#include <madopilot/madopilot.h>

namespace {

int failed(const char* call, madopilot_status_t status)
{
    std::fprintf(stderr, "direct-abi-cpp: %s failed with status %d\n", call,
                 static_cast<int>(status));
    return 1;
}

// Observes the handle before and after cancelling it. The caller releases it.
int observe(const madopilot_api_t& api, const madopilot_cancellation_t* cancellation)
{
    std::int32_t before = -1;
    std::int32_t after = -1;
    if (const auto status = api.cancellation_is_cancelled(cancellation, &before);
        status != MADOPILOT_STATUS_OK) {
        return failed("cancellation_is_cancelled", status);
    }
    if (const auto status = api.cancellation_cancel(cancellation);
        status != MADOPILOT_STATUS_OK) {
        return failed("cancellation_cancel", status);
    }
    if (const auto status = api.cancellation_is_cancelled(cancellation, &after);
        status != MADOPILOT_STATUS_OK) {
        return failed("cancellation_is_cancelled", status);
    }

    std::printf("direct-abi-cpp: is_cancelled before cancel = %d, after cancel = %d\n",
                static_cast<int>(before), static_cast<int>(after));
    if (before != 0 || after == 0) {
        std::fprintf(stderr, "direct-abi-cpp: expected uncancelled, then cancelled\n");
        return 1;
    }
    return 0;
}

} // namespace

int main()
{
    const madopilot_api_t* table = nullptr;
    if (const auto status = madopilot_get_api(MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR,
                                              sizeof(madopilot_api_t), &table);
        status != MADOPILOT_STATUS_OK || table == nullptr) {
        return failed("madopilot_get_api", status);
    }
    const madopilot_api_t& api = *table;

    // Only members inside both this header's table and the library's exist.
    const std::size_t extent =
        std::min<std::size_t>(sizeof(madopilot_api_t), api.struct_size);
    if (api.abi_major != MADOPILOT_ABI_MAJOR || api.abi_minor < MADOPILOT_ABI_MINOR ||
        extent < MADOPILOT_API_SIZE_ABI_1_0) {
        std::fprintf(stderr,
                     "direct-abi-cpp: abi %u.%u with %u usable bytes is not supported\n",
                     static_cast<unsigned>(api.abi_major),
                     static_cast<unsigned>(api.abi_minor), static_cast<unsigned>(extent));
        return 1;
    }

    madopilot_build_info_t build{};
    build.struct_size = static_cast<std::uint32_t>(sizeof(build));
    if (const auto status = api.describe_build(&build); status != MADOPILOT_STATUS_OK) {
        return failed("describe_build", status);
    }
    if (build.abi_major != api.abi_major || build.abi_minor != api.abi_minor ||
        build.table_size != api.struct_size) {
        std::fprintf(stderr,
                     "direct-abi-cpp: describe_build disagrees with the negotiated table\n");
        return 1;
    }
    std::printf("direct-abi-cpp: abi %u.%u, table %u bytes, using %u bytes, library %.*s\n",
                static_cast<unsigned>(build.abi_major),
                static_cast<unsigned>(build.abi_minor),
                static_cast<unsigned>(build.table_size), static_cast<unsigned>(extent),
                static_cast<int>(build.library_version.len), build.library_version.data);

    madopilot_cancellation_t* cancellation = nullptr;
    if (const auto status = api.cancellation_create(&cancellation);
        status != MADOPILOT_STATUS_OK || cancellation == nullptr) {
        return failed("cancellation_create", status);
    }

    const int result = observe(api, cancellation);

    // Released on every path once created; a failed release is a failure too.
    if (const auto status = api.cancellation_release(cancellation);
        status != MADOPILOT_STATUS_OK) {
        return failed("cancellation_release", status);
    }
    std::printf("direct-abi-cpp: cancellation_release ok\n");

    return result;
}
