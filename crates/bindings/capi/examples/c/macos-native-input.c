/*
 * Native macOS C flow.
 *
 * `--check` creates the native engine and reads Screen Recording and
 * Accessibility without prompting. Passing one exact full fixture-window title
 * additionally captures that unique window, requires both permissions, opens
 * system input, moves the pointer, types "m", reports the receipt, and waits
 * for a strictly newer frame containing the fixture's expected changed fill.
 * No title, captured bytes, or typed text is printed.
 *
 * Use the repository's dedicated `mado-pilot-macos-input-fixture` with
 * `--animate-on-input`; an ordinary application is not this visual oracle.
 *
 *   macos-native-input --check
 *   macos-native-input "MadoPilot Input Fixture [<pid>]"
 */
#define MADOPILOT_EXAMPLE_NAME "macos-native-input"
#define MADOPILOT_EXAMPLE_SOURCE_KIND MADOPILOT_SOURCE_NATIVE_MACOS
#define MADOPILOT_EXAMPLE_REQUIRED_PAIRS \
    (MADOPILOT_INPUT_PAIR_POINTER_SYSTEM | MADOPILOT_INPUT_PAIR_KEYBOARD_SYSTEM)
#define MADOPILOT_EXAMPLE_DELIVERY MADOPILOT_INPUT_DELIVERY_SYSTEM
#define MADOPILOT_EXAMPLE_FOCUS MADOPILOT_FOCUS_ACTIVATE_IF_REQUIRED
#define MADOPILOT_EXAMPLE_READS_PERMISSIONS 1
#define MADOPILOT_EXAMPLE_ALLOWS_UNKNOWN 0

#include "native-input-common.h"
