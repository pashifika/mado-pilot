/*
 * Shared visual oracle for the platform-native C and C++ examples.
 *
 * The dedicated fixtures start with their platform-specific baseline colour and,
 * in --animate-on-input mode, change to the same deterministic 0xRRGGBB fill.
 * Searching the central half of a canonical BGRA8 mapping keeps the oracle away
 * from window chrome and proves application state independently of the input
 * receipt. This is repository example code, not a public header.
 */
#ifndef MADOPILOT_NATIVE_EXPECTED_CONDITION_H
#define MADOPILOT_NATIVE_EXPECTED_CONDITION_H

#include <stddef.h>
#include <stdint.h>

#define MADOPILOT_EXAMPLE_EXPECTED_FILL_RGB UINT32_C(0x00c45b2e)
#if defined(__APPLE__)
#define MADOPILOT_EXAMPLE_FILL_TOLERANCE 24u
#else
#define MADOPILOT_EXAMPLE_FILL_TOLERANCE 8u
#endif

static int madopilot_example_expected_condition_matches(const uint8_t* pixels,
                                                         size_t length,
                                                         uint64_t stride,
                                                         uint32_t width,
                                                         uint32_t height)
{
    const uint8_t expected[3] = {
        (uint8_t)(MADOPILOT_EXAMPLE_EXPECTED_FILL_RGB & UINT32_C(0xff)),
        (uint8_t)((MADOPILOT_EXAMPLE_EXPECTED_FILL_RGB >> 8) & UINT32_C(0xff)),
        (uint8_t)((MADOPILOT_EXAMPLE_EXPECTED_FILL_RGB >> 16) & UINT32_C(0xff)),
    };
    size_t row_stride;
    size_t required;
    uint32_t row;
    uint32_t column;
    if (pixels == NULL || width < 8u || height < 8u) {
        return 0;
    }
    row_stride = (size_t)stride;
    if (row_stride < (size_t)width * 4u ||
        height > SIZE_MAX / row_stride) {
        return 0;
    }
    required = row_stride * (size_t)height;
    if (length < required) {
        return 0;
    }

    for (row = height / 4u; row < (height * 3u) / 4u; row += 1u) {
        for (column = width / 4u; column < (width * 3u) / 4u; column += 1u) {
            const size_t at = (size_t)row * row_stride + (size_t)column * 4u;
            size_t channel;
            for (channel = 0; channel < 3u; channel += 1u) {
                const uint8_t seen = pixels[at + channel];
                const uint8_t wanted = expected[channel];
                const uint8_t difference = seen > wanted ? seen - wanted : wanted - seen;
                if (difference > MADOPILOT_EXAMPLE_FILL_TOLERANCE) {
                    return 0;
                }
            }
        }
    }
    return 1;
}

#endif /* MADOPILOT_NATIVE_EXPECTED_CONDITION_H */
