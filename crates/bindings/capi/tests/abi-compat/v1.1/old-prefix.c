/*
 * The superseded ABI 1.1 draft header, compiled and linked against the current
 * library. ABI 1.1 was never released: this fixture proves its source still
 * compiles as historical evidence, but negotiation rejects the draft rather
 * than interpreting its suffix as ABI 1.2.
 */

#include <stdint.h>
#include <stdio.h>

#include "madopilot/madopilot.h"

static int failures = 0;

static void expect(int condition, const char* what)
{
    if (!condition) {
        printf("FAIL: %s\n", what);
        failures += 1;
    }
}

static void expect_rejection(uint32_t minimum_minor, size_t extent,
                             madopilot_status_t expected, const char* what)
{
    const madopilot_api_t* api = (const madopilot_api_t*)(uintptr_t)1;
    const madopilot_status_t status =
        madopilot_get_api(MADOPILOT_ABI_MAJOR, minimum_minor, extent, &api);
    expect(status == expected, what);
    expect(api == NULL, "a rejected stale-header negotiation clears its output");
}

int main(void)
{
    printf("superseded header: abi %u.%u, table %zu bytes as the draft declared it\n",
           (unsigned)MADOPILOT_ABI_MAJOR, (unsigned)MADOPILOT_ABI_MINOR,
           sizeof(madopilot_api_t));

    expect_rejection(MADOPILOT_ABI_MINOR, sizeof(madopilot_api_t),
                     MADOPILOT_STATUS_UNSUPPORTED,
                     "minimum minor 1 is explicitly unsupported");
    expect_rejection(MADOPILOT_ABI_MINOR, MADOPILOT_API_SIZE_INFORMATION,
                     MADOPILOT_STATUS_UNSUPPORTED,
                     "a shorter draft prefix cannot make minor 1 valid");
    expect_rejection(0, sizeof(madopilot_api_t),
                     MADOPILOT_STATUS_UNSUPPORTED,
                     "minor 0 cannot claim the superseded suffix extent");
    if (failures != 0) {
        return 1;
    }
    puts("madopilot-abi-compat-v1.1 complete (superseded draft rejected)");
    return 0;
}
