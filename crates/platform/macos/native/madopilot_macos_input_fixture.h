/*
 * MadoPilot macOS input fixture: the window interactive verification targets.
 *
 * This surface is internal to the fixture binary and is compiled into an archive
 * of its own, so nothing in the production Adapter links it. It follows the same
 * rules as the production shim: opaque callbacks, a status return, and no
 * Objective-C type crossing into Rust.
 *
 * AppKit is loaded from its absolute system location rather than linked, exactly
 * as ScreenCaptureKit is in the production shim. The opt-in game-like renderer
 * likewise loads OpenGL only from its absolute system framework location. A host
 * that cannot provide a requested framework reports MP_FIXTURE_UNSUPPORTED
 * instead of falling back or failing to load.
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

/* Versioned, payload-free commands accepted from the owned test harness. */
#define MP_FIXTURE_CONTROL_VERSION 5u
#define MP_FIXTURE_COMMAND_TRANSITION 1u
#define MP_FIXTURE_COMMAND_REPLACE 2u
#define MP_FIXTURE_COMMAND_MINIMIZE 3u
#define MP_FIXTURE_COMMAND_RESTORE 4u
#define MP_FIXTURE_COMMAND_STOP 5u
#define MP_FIXTURE_COMMAND_YIELD_FOREGROUND 6u
#define MP_FIXTURE_COMMAND_MOVE 7u
#define MP_FIXTURE_COMMAND_RESIZE 8u
#define MP_FIXTURE_COMMAND_OPEN_AUXILIARY 9u
#define MP_FIXTURE_COMMAND_CLOSE_AUXILIARY 10u
#define MP_FIXTURE_COMMAND_CLOSE 11u
#define MP_FIXTURE_COMMAND_MOVE_TO_NEXT_DISPLAY 12u
#define MP_FIXTURE_COMMAND_RESET_EVENTS 13u
#define MP_FIXTURE_COMMAND_READ_EVENTS 14u

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

/* Opt-in deterministic stimuli used only by the native benchmark fixture. */
#define MP_FIXTURE_BEHAVIOR_ANIMATE_ON_KEY_DOWN 1u
#define MP_FIXTURE_BEHAVIOR_RESIZE_ON_KEY_DOWN 2u

/* Mutually exclusive rendering paths selected before the window is created. */
#define MP_FIXTURE_RENDERER_APPKIT_BACKGROUND 0u
#define MP_FIXTURE_RENDERER_OPENGL 1u

/*
 * Runs the fixture window until the process is terminated.
 *
 * `title` is a NUL-terminated UTF-8 string used verbatim as the window title.
 * `fill` and `replacement_fill` are deterministic 0xRRGGBB colours.
 * `renderer` selects the default AppKit background-colour path or the opt-in
 * OpenGL content-view path. `activate` is one when the fixture may take
 * foreground ownership and zero when qualification needs a visible inactive
 * target. `behavior` retains only benchmark compatibility; qualification
 * drives visual transitions through `mp_fixture_control`. `ready` is invoked
 * once after the first window is visible and reports the renderer that actually
 * initialized. `controlled` reports each accepted control nonce with the native
 * status and the exact before/after window numbers. `sink` reports bounded input
 * metadata only.
 * No callback may let a Rust panic escape.
 *
 * `mp_fixture_control` validates the per-run identity and fixed command before
 * enqueuing it on the AppKit main thread. A command nonce greater than the prior
 * nonce executes exactly once; an equal nonce replays the cached result without
 * executing; an older nonce is rejected. A zero return means the command was
 * accepted for asynchronous processing, not that its transition succeeded.
 */
uint32_t mp_fixture_run(const char *title, uint64_t run_nonce, uint32_t fill,
                        uint32_t replacement_fill, uint32_t behavior, uint32_t renderer,
                        uint32_t replacement_delay_ms, double width, double height,
                        uint32_t activate, uint32_t launch_context, uint32_t signature_mode,
                        const uint8_t *signing_identifier, size_t signing_identifier_len,
                        void *context,
                        void (*ready)(void *context, uint64_t window_number,
                                      uint64_t run_nonce, uint32_t renderer,
                                      uint32_t launch_context, uint32_t signature_mode,
                                      const uint8_t *signing_identifier,
                                      size_t signing_identifier_len),
                        void (*replaced)(void *context, uint32_t status,
                                         uint64_t old_window_number,
                                         uint64_t new_window_number),
                        void (*controlled)(void *context, uint64_t nonce,
                                           uint32_t command, uint32_t status,
                                           uint64_t before_window_number,
                                           uint64_t after_window_number),
                        void (*sink)(void *context, uint32_t kind, uint32_t text_units));

uint32_t mp_fixture_control(uint32_t version, uint64_t run_nonce,
                            uint64_t nonce, uint32_t command);

/*
 * Fixture-binary test seam. Attempts only one fixed, deliberately absent
 * absolute framework path and proves the renderer loader refuses it.
 */
uint32_t mp_fixture_test_unsupported_renderer(void);

#ifdef __cplusplus
}
#endif

#endif /* MADOPILOT_MACOS_INPUT_FIXTURE_H */
