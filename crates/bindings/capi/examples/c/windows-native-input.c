/*
 * Native Windows C flow.
 *
 * `--check` creates the native engine and confirms that Windows exposes no
 * permission state this Adapter can read. Passing one exact full window title
 * additionally selects that unique window, requires explicit exact-window
 * `WindowMessage`, captures it, moves the pointer, types "m", reports the
 * receipt, and waits for a strictly newer frame containing the expected fill.
 * It preserves focus and permits no system-input fallback.
 *
 * Ordinary windows expose this route as unknown-but-attemptable and report
 * target-queue admission; the dedicated `MadoPilotInputFixture` alone reports
 * supported compatibility and protocol acknowledgement. A complete receipt is
 * not proof that an arbitrary application consumed the posted messages.
 *
 * Start `mado-pilot-windows-window-message-fixture -- --title-token=example`
 * for the ordinary visual oracle used below.
 *
 *   windows-native-input --check
 *   windows-native-input "MadoPilot Ordinary WindowMessage Fixture [example]"
 */
#define MADOPILOT_EXAMPLE_NAME "windows-native-input"
#define MADOPILOT_EXAMPLE_SOURCE_KIND MADOPILOT_SOURCE_NATIVE_WINDOWS
#define MADOPILOT_EXAMPLE_REQUIRED_PAIRS \
    (MADOPILOT_INPUT_PAIR_POINTER_WINDOW_MESSAGE | \
     MADOPILOT_INPUT_PAIR_KEYBOARD_WINDOW_MESSAGE)
#define MADOPILOT_EXAMPLE_DELIVERY MADOPILOT_INPUT_DELIVERY_WINDOW_MESSAGE
#define MADOPILOT_EXAMPLE_FOCUS MADOPILOT_FOCUS_PRESERVE
#define MADOPILOT_EXAMPLE_READS_PERMISSIONS 0
#define MADOPILOT_EXAMPLE_ALLOWS_UNKNOWN 1

#include "native-input-common.h"
