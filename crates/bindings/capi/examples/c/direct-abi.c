/*
 * Direct use of the C ABI: negotiate the table, describe the build, and own one
 * cancellation handle from creation to explicit release.
 *
 * Needs only the built library: no model, asset, window, capture, input, or
 * permission.
 */

#include <stdint.h>
#include <stdio.h>

#include <madopilot/madopilot.h>

static int failed(const char* call, madopilot_status_t status)
{
    fprintf(stderr, "direct-abi-c: %s failed with status %d\n", call, (int)status);
    return 1;
}

/* Observes the handle before and after cancelling it. The caller releases it. */
static int observe(const madopilot_api_t* api, const madopilot_cancellation_t* cancellation)
{
    int32_t before = -1;
    int32_t after = -1;
    madopilot_status_t status = api->cancellation_is_cancelled(cancellation, &before);
    if (status != MADOPILOT_STATUS_OK) {
        return failed("cancellation_is_cancelled", status);
    }
    status = api->cancellation_cancel(cancellation);
    if (status != MADOPILOT_STATUS_OK) {
        return failed("cancellation_cancel", status);
    }
    status = api->cancellation_is_cancelled(cancellation, &after);
    if (status != MADOPILOT_STATUS_OK) {
        return failed("cancellation_is_cancelled", status);
    }

    printf("direct-abi-c: is_cancelled before cancel = %d, after cancel = %d\n",
           (int)before, (int)after);
    if (before != 0 || after == 0) {
        fprintf(stderr, "direct-abi-c: expected uncancelled, then cancelled\n");
        return 1;
    }
    return 0;
}

int main(void)
{
    const madopilot_api_t* api = NULL;
    madopilot_status_t status = madopilot_get_api(
        MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR, sizeof(madopilot_api_t), &api);
    if (status != MADOPILOT_STATUS_OK || api == NULL) {
        return failed("madopilot_get_api", status);
    }

    /* Only members inside both this header's table and the library's exist. */
    size_t extent = sizeof(madopilot_api_t);
    if (api->struct_size < extent) {
        extent = api->struct_size;
    }
    if (api->abi_major != MADOPILOT_ABI_MAJOR || api->abi_minor < MADOPILOT_ABI_MINOR ||
        extent < MADOPILOT_API_SIZE_ABI_1_0) {
        fprintf(stderr, "direct-abi-c: abi %u.%u with %u usable bytes is not supported\n",
                (unsigned)api->abi_major, (unsigned)api->abi_minor, (unsigned)extent);
        return 1;
    }

    madopilot_build_info_t build = {0};
    build.struct_size = (uint32_t)sizeof(build);
    status = api->describe_build(&build);
    if (status != MADOPILOT_STATUS_OK) {
        return failed("describe_build", status);
    }
    if (build.abi_major != api->abi_major || build.abi_minor != api->abi_minor ||
        build.table_size != api->struct_size) {
        fprintf(stderr, "direct-abi-c: describe_build disagrees with the negotiated table\n");
        return 1;
    }
    printf("direct-abi-c: abi %u.%u, table %u bytes, using %u bytes, library %.*s\n",
           (unsigned)build.abi_major, (unsigned)build.abi_minor,
           (unsigned)build.table_size, (unsigned)extent,
           (int)build.library_version.len, build.library_version.data);

    madopilot_cancellation_t* cancellation = NULL;
    status = api->cancellation_create(&cancellation);
    if (status != MADOPILOT_STATUS_OK || cancellation == NULL) {
        return failed("cancellation_create", status);
    }

    int result = observe(api, cancellation);

    /* Released on every path once created; a failed release is a failure too. */
    status = api->cancellation_release(cancellation);
    if (status != MADOPILOT_STATUS_OK) {
        return failed("cancellation_release", status);
    }
    printf("direct-abi-c: cancellation_release ok\n");

    return result;
}
