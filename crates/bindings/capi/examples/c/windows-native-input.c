/*
 * Native Windows C flow.
 *
 * `--check` creates the native engine and confirms that Windows exposes no
 * permission state this Adapter can read. Passing one exact full fixture-window
 * title additionally selects that unique window, requires the dedicated
 * background-input protocol, captures it, moves the pointer, types "mp",
 * reports the receipt, and closes. It preserves focus and permits no system-
 * input fallback, so an ordinary window is refused before any event exists.
 * No title, captured bytes, or typed text is printed.
 *
 *   windows-native-input --check
 *   windows-native-input "MadoPilot Input Fixture [<pid>]"
 */
#define MADOPILOT_EXAMPLE_NAME "windows-native-input"
#define MADOPILOT_EXAMPLE_SOURCE_KIND MADOPILOT_SOURCE_NATIVE_WINDOWS
#define MADOPILOT_EXAMPLE_REQUIRED_PAIRS \
    (MADOPILOT_INPUT_PAIR_POINTER_BACKGROUND | MADOPILOT_INPUT_PAIR_KEYBOARD_BACKGROUND)
#define MADOPILOT_EXAMPLE_DELIVERY MADOPILOT_INPUT_DELIVERY_BACKGROUND_TARGET
#define MADOPILOT_EXAMPLE_FOCUS MADOPILOT_FOCUS_PRESERVE
#define MADOPILOT_EXAMPLE_READS_PERMISSIONS 0

#include "native-input-common.h"
