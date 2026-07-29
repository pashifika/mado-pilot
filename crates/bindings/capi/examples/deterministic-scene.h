/*
 * The deterministic Phase 1 scene, shared by the C and C++ examples.
 *
 * A 96x64 pseudo-random background with a 12x10 patch planted at two offsets and
 * a half-contrast copy at a third. Every value comes from integer arithmetic on
 * the pixel coordinate, so both release targets build the same bytes and neither
 * example needs a tracked image.
 *
 * This is the same arithmetic as `mado_pilot_testkit::match_fixtures`, which
 * produced the tracked template in `fixtures/assets/phase1-slice`. If the two
 * drift apart the template stops being found where it is planted and both
 * examples fail.
 *
 * It lives here rather than inside one example because there are two callers.
 * The Rust copy in the testkit and this one are the only two; a third would be a
 * third place to keep in step.
 *
 * Everything has internal linkage: this header is included by exactly one
 * translation unit per program, and nothing here crosses a link boundary. It is
 * valid C99 and valid C++17, because both examples include it.
 *
 * The names below are unprefixed while the include guard is prefixed, and that
 * stays as it is. This header is not part of what MadoPilot publishes: it is
 * neither installed nor exported, no target adds this directory to a consumer's
 * include path, and the only compilations that see it are the ones `c-abi-check`
 * runs over this repository's own programs. One of those is the frozen ABI
 * fixture `tests/abi-compat/v1/old-prefix.c`, which uses these names and may not
 * be edited, so prefixing them would mean changing a snapshot to improve a name
 * no consumer can collide with. The guard is prefixed because a guard collides
 * across headers a program includes together, which is a different question.
 */

#ifndef MADOPILOT_EXAMPLES_DETERMINISTIC_SCENE_H
#define MADOPILOT_EXAMPLES_DETERMINISTIC_SCENE_H

#include <stddef.h>
#include <stdint.h>

#define SCENE_WIDTH 96u
#define SCENE_HEIGHT 64u
#define PATCH_WIDTH 12u
#define PATCH_HEIGHT 10u

/* Four bytes per pixel, packed rows. */
#define SCENE_BYTES ((size_t)SCENE_WIDTH * (size_t)SCENE_HEIGHT * 4u)

/* Where the patch is planted, exactly. Both examples check the match against
 * these rather than against a literal written twice. */
static const uint32_t SCENE_PLANTED[2][2] = { { 20u, 12u }, { 60u, 40u } };

/* A half-contrast copy, which the default threshold rejects. */
static const uint32_t SCENE_DEGRADED[2] = { 20u, 44u };

static uint8_t scene_byte_of(uint32_t value)
{
    return (uint8_t)(value & 0xffu);
}

static void scene_patch_pixel(uint32_t x, uint32_t y, uint8_t out[3])
{
    if (x == 0u || y == 0u || x + 1u == PATCH_WIDTH || y + 1u == PATCH_HEIGHT) {
        out[0] = 0xffu;
        out[1] = 0xffu;
        out[2] = 0xffu;
        return;
    }
    out[0] = scene_byte_of(x * 20u);
    out[1] = scene_byte_of(y * 24u);
    out[2] = 0x30u;
}

static void scene_background_pixel(uint32_t x, uint32_t y, uint8_t out[3])
{
    uint32_t mixed = x * 2654435761u + y * 2246822519u;
    mixed ^= mixed >> 15;
    out[0] = scene_byte_of(mixed);
    out[1] = scene_byte_of(mixed >> 8);
    out[2] = scene_byte_of(mixed >> 16);
}

static int scene_covers(const uint32_t origin[2], uint32_t x, uint32_t y,
                        uint32_t* out_x, uint32_t* out_y)
{
    if (x < origin[0] || y < origin[1] || x >= origin[0] + PATCH_WIDTH ||
        y >= origin[1] + PATCH_HEIGHT) {
        return 0;
    }
    *out_x = x - origin[0];
    *out_y = y - origin[1];
    return 1;
}

static void scene_pixel(uint32_t x, uint32_t y, uint8_t out[3])
{
    uint32_t px = 0u;
    uint32_t py = 0u;
    size_t index;

    for (index = 0; index < 2; ++index) {
        if (scene_covers(SCENE_PLANTED[index], x, y, &px, &py)) {
            scene_patch_pixel(px, py, out);
            return;
        }
    }

    if (scene_covers(SCENE_DEGRADED, x, y, &px, &py)) {
        uint8_t ideal[3];
        uint8_t noise[3];
        size_t channel;
        scene_patch_pixel(px, py, ideal);
        scene_background_pixel(x, y, noise);
        for (channel = 0; channel < 3; ++channel) {
            out[channel] =
                scene_byte_of(((uint32_t)ideal[channel] + noise[channel]) / 2u);
        }
        return;
    }

    scene_background_pixel(x, y, out);
}

/* Writes SCENE_BYTES of packed RGBA8 into caller-owned storage, so the caller
 * decides how it is allocated. */
static void scene_fill_rgba(uint8_t* pixels)
{
    uint32_t x;
    uint32_t y;

    for (y = 0; y < SCENE_HEIGHT; ++y) {
        for (x = 0; x < SCENE_WIDTH; ++x) {
            uint8_t rgb[3];
            size_t at = ((size_t)y * SCENE_WIDTH + x) * 4u;
            scene_pixel(x, y, rgb);
            pixels[at + 0] = rgb[0];
            pixels[at + 1] = rgb[1];
            pixels[at + 2] = rgb[2];
            pixels[at + 3] = 0xffu;
        }
    }
}

#endif /* MADOPILOT_EXAMPLES_DETERMINISTIC_SCENE_H */
