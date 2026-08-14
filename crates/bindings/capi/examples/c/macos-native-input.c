/*
 * Native macOS C flow.
 *
 * `--check` creates the native engine and reads Screen Recording and Input
 * Control without prompting. Passing one exact full fixture-window title
 * additionally captures that retained window, explicitly requires ABI 1.2
 * process-directed pointer and keyboard pairs, preserves focus, invokes one
 * sequence on the owning process, reports InvocationOnly evidence, and checks
 * the expected visual condition separately on a strictly newer frame. The
 * request lists no System fallback, and no title, captured bytes, or typed text
 * is printed.
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
    (MADOPILOT_INPUT_PAIR_POINTER_PROCESS_DIRECTED | \
     MADOPILOT_INPUT_PAIR_KEYBOARD_PROCESS_DIRECTED)
#define MADOPILOT_EXAMPLE_DELIVERY MADOPILOT_INPUT_DELIVERY_PROCESS_DIRECTED
#define MADOPILOT_EXAMPLE_FOCUS MADOPILOT_FOCUS_PRESERVE
#define MADOPILOT_EXAMPLE_READS_PERMISSIONS 1
#define MADOPILOT_EXAMPLE_ALLOWS_UNKNOWN 1

#include "native-input-common.h"
