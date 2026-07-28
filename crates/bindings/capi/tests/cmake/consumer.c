/*
 * A C consumer of `MadoPilot::C`, and nothing else.
 *
 * It negotiates the table and reports what it got. That is the whole test: if
 * the target failed to carry the include directory or the library, this file
 * would not compile or would not link, and if the library it linked were not
 * the one that was built, negotiation would refuse it.
 */

#include <stdio.h>
#include <string.h>

#include "madopilot/madopilot.h"

int main(void)
{
    const madopilot_api_t* api = NULL;
    madopilot_build_info_t build;
    madopilot_status_t status;

    status = madopilot_get_api(MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR,
                               sizeof(madopilot_api_t), &api);
    if (status != MADOPILOT_STATUS_OK || api == NULL) {
        fprintf(stderr, "madopilot_get_api failed with %d\n", (int)status);
        return 1;
    }

    memset(&build, 0, sizeof(build));
    build.struct_size = (uint32_t)sizeof(build);
    if (api->describe_build(&build) != MADOPILOT_STATUS_OK) {
        fprintf(stderr, "describe_build failed\n");
        return 1;
    }

    printf("MadoPilot::C consumer: abi %u.%u, table %u bytes\n", build.abi_major,
           build.abi_minor, build.table_size);

    return 0;
}
