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

    if (api->struct_size < MADOPILOT_API_SIZE_ENGINE_OCR_PROVIDER_DESCRIPTOR ||
        build.bounded_ocr_model.len == 0u ||
        build.bounded_ocr_profile.len == 0u) {
        fprintf(stderr, "ABI 1.5 OCR provider surface is incomplete\n");
        return 1;
    }
    madopilot_ocr_zone_t zone;
    madopilot_ocr_zone_scan_request_t request;
    memset(&zone, 0, sizeof(zone));
    zone.struct_size = (uint32_t)sizeof(zone);
    zone.region.space = MADOPILOT_SPACE_CAPTURE_PIXELS;
    zone.region.right = 8;
    zone.region.bottom = 8;
    zone.clip_policy = MADOPILOT_CLIP_POLICY_REJECT;
    memset(&request, 0, sizeof(request));
    request.struct_size = (uint32_t)sizeof(request);
    request.model_id = build.bounded_ocr_model;
    request.backend_id = build.default_ocr_backend;
    request.backend_version = build.default_ocr_backend_version;
    request.output_space = MADOPILOT_SPACE_CAPTURE_PIXELS;
    request.zones = &zone;
    request.zone_count = 1;
    request.zone_stride = sizeof(zone);
    if (request.zone_stride != sizeof(madopilot_ocr_zone_t)) {
        return 1;
    }
    madopilot_ocr_provider_options_t provider;
    madopilot_ocr_provider_descriptor_t provider_descriptor;
    memset(&provider, 0, sizeof(provider));
    provider.struct_size = (uint32_t)sizeof(provider);
    provider.policy = MADOPILOT_OCR_PROVIDER_POLICY_CPU;
    memset(&provider_descriptor, 0, sizeof(provider_descriptor));
    provider_descriptor.struct_size = (uint32_t)sizeof(provider_descriptor);
    if (provider.policy != MADOPILOT_OCR_PROVIDER_POLICY_CPU ||
        provider_descriptor.active_provider !=
            MADOPILOT_OCR_EXECUTION_PROVIDER_UNSPECIFIED) {
        return 1;
    }

    printf("MadoPilot::C consumer: abi %u.%u, table %u bytes\n", build.abi_major,
           build.abi_minor, build.table_size);

    return 0;
}
