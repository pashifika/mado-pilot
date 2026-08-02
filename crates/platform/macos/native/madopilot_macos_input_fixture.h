/*
 * MadoPilot macOS input fixture: the window interactive verification targets.
 *
 * This surface is internal to the fixture binary and is compiled into an archive
 * of its own, so nothing in the production Adapter links it. It follows the same
 * rules as the production shim: opaque callbacks, a status return, and no
 * Objective-C type crossing into Rust.
 *
 * AppKit is loaded from its absolute system location rather than linked, exactly
 * as ScreenCaptureKit is in the production shim. A host that cannot provide it
 * reports MP_FIXTURE_UNSUPPORTED instead of failing to load.
 */

#ifndef MADOPILOT_MACOS_INPUT_FIXTURE_H
#define MADOPILOT_MACOS_INPUT_FIXTURE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Status returned by the entry point. Zero is success. */
#define MP_FIXTURE_OK 0u
#define MP_FIXTURE_INVALID_ARGUMENT 1u
#define MP_FIXTURE_UNSUPPORTED 2u
#define MP_FIXTURE_PLATFORM_FAILURE 3u
#define MP_FIXTURE_NATIVE_EXCEPTION 4u

/*
 * What one observed event was.
 *
 * A synthesized text event and a synthesized key press are the same kind of
 * AppKit event at the receiving end, so the fixture reports the key kinds and the
 * UTF-16 unit count beside them rather than inventing a distinction it cannot
 * observe. The units are counted; the characters themselves are never retained,
 * printed, or passed across this boundary.
 */
#define MP_FIXTURE_EVENT_POINTER_MOVE 1u
#define MP_FIXTURE_EVENT_POINTER_PRESS 2u
#define MP_FIXTURE_EVENT_POINTER_RELEASE 3u
#define MP_FIXTURE_EVENT_POINTER_SCROLL 4u
#define MP_FIXTURE_EVENT_KEY_DOWN 5u
#define MP_FIXTURE_EVENT_KEY_UP 6u
#define MP_FIXTURE_EVENT_FLAGS_CHANGED 7u

/* The signing and launch context the fixture window was created in. */
#define MP_FIXTURE_CONTEXT_UNKNOWN 0u
#define MP_FIXTURE_CONTEXT_BUNDLED 1u
#define MP_FIXTURE_CONTEXT_UNBUNDLED 2u

/*
 * Runs the fixture window until the process is terminated.
 *
 * `title` is a NUL-terminated UTF-8 string used verbatim as the window title.
 * `fill` is an 0xRRGGBB colour the window is filled with and nothing else.
 * `ready` is invoked once, after the window is on screen. `sink` is invoked for
 * each observed event on the main thread. Neither callback may let a Rust panic
 * escape.
 */
uint32_t mp_fixture_run(const char *title, uint32_t fill, double width, double height,
                        void *context, void (*ready)(void *context, uint64_t window_number,
                                                     uint32_t launch_context),
                        void (*sink)(void *context, uint32_t kind, uint32_t text_units));

#ifdef __cplusplus
}
#endif

#endif /* MADOPILOT_MACOS_INPUT_FIXTURE_H */
