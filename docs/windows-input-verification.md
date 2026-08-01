# Windows Input Adapter and Verification

The Windows platform package implements input at the Adapter boundary. Runtime,
public-facade, C ABI, and C++ wiring remain later work, so this is an implemented
Rust platform capability rather than a release-level product claim.

## Capability boundary

Capabilities are operation-and-delivery pairs. A target never acquires a
background capability merely because it accepts the same operation through system
input.

| Discovered target | `System` | `BackgroundTarget` |
|---|---|---|
| Ordinary top-level window | Pointer, keyboard, text; focus required | None |
| Class `MadoPilotInputFixture` | Pointer, keyboard, text; focus required | Pointer, keyboard, text through the acknowledged fixture protocol |
| Display | Pointer only; no focusable target is implied | None |

All five shared pointer spaces are advertised because Windows capture publishes an
authoritative target placement. A request still fails when its selected geometry
policy cannot resolve the named frame:

- `ReprojectCurrent` reads the current extent, placement, and scale.
- `RequireUnchanged` accepts only a source frame retained for that target whose
  geometry fingerprint still equals the live target.
- `UseFrameSnapshot` uses a retained source transform. An old revision that is no
  longer retained is unsupported rather than reconstructed from current DPI.

Windows virtual-screen coordinates are signed physical coordinates. System pointer
moves use `MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK`, preserve half-open target
bounds, and never clamp a far edge into an adjacent display. Primary and secondary
buttons honor `SM_SWAPBUTTON`, and cleanup retains the exact physical mapping used
at press time.

Logical keys are resolved through the target thread's active keyboard layout. A
`Key::Character` that needs implicit modifiers or more than one UTF-16 unit is
unsupported; callers use explicit modifier events or `InputEvent::Text` instead.
Text uses paired `KEYEVENTF_UNICODE` records and the shared 4,096-character bound.

## Focus, integrity, and receipts

Window system input revalidates focus before every irreversible event.
`Preserve` cannot satisfy a focus-requiring system path. `RequireFocused` never
activates a window. `ActivateIfRequired` makes one ordinary
[`SetForegroundWindow`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setforegroundwindow)
attempt and then re-reads the foreground window; Windows may refuse it under its
foreground-lock rules. The Adapter does not attach input queues, start an elevated
helper, or otherwise bypass that policy.

Before system or fixture delivery, the Adapter compares the caller and selected
window process integrity levels. A proven higher target reports `PolicyRefused`.
When
[`SendInput`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput)
returns zero and integrity inspection does not prove UIPI, the Adapter reports the
nonspecific `DeliveryFailed` that the API evidence supports; it does not infer a
UIPI cause from last error.

`delivered` counts complete logical events. A short nonzero native insertion, or a
fixture dispatch that began without returning the expected acknowledgement, is
`Partial` even if no logical event completed. It is never `Unexecuted`, and no
fallback mechanism is tried. [ADR 0015](adr/0015-partial-native-input-effects-and-receipt-accounting.md)
records why this zero-complete partial outcome is required for safe retry behavior.

Pressed buttons and keys belong only to the sequence that successfully pressed
them. On a partial stop they are released newest first under the independent shared
cleanup bound: at most 256 releases and no new release after 250 milliseconds.

## Dedicated fixture

`mado-pilot-windows-input-fixture` creates one controlled top-level window with:

- class `MadoPilotInputFixture`;
- exact title `MadoPilot Input Fixture [<pid>]`;
- a versioned and size-bounded `WM_COPYDATA` vocabulary;
- synchronous acknowledgement for each accepted event;
- at most 256 retained summaries containing only event kind and UTF-16 unit count.

The fixture validates packet length, scalar fields, key and button codes, and UTF-16.
It neither retains nor prints input text. Verification calls
`select_unique_fixture`, which requires a window target, the exact PID-qualified
title, and all three fixture-only background capabilities. Zero matches and
multiple matches both stop before input.

## Automated Windows checks

Run the focused checks from the repository root:

```sh
cargo check --locked -p mado-pilot-platform-windows --all-targets
cargo test --locked -p mado-pilot-input
cargo test --locked -p mado-pilot-platform-windows --lib
cargo test --locked -p mado-pilot-platform-windows --test native_input
```

The native integration test starts the dedicated fixture without activating it,
discovers and selects it exactly once, and sends six acknowledged background
pointer, keyboard, and text events. It never calls system delivery and therefore
does not move the developer's pointer or type into the desktop. Deterministic tests
cover system/background separation, target classes, focus outcomes, signed
mixed-DPI coordinates, integrity-policy receipts, partial events, cleanup, target
loss, cancellation/deadline races, and close.

## Explicit system-input check

The system path is ignored by the default suite because successful keyboard input
requires a real foreground window. Run it only on an interactive Windows desktop:

```sh
cargo test --locked -p mado-pilot-platform-windows --test native_input interactive_system_delivery_targets_only_the_exact_fixture -- --ignored --exact --nocapture --test-threads=1
```

The test opens the PID-qualified fixture and waits 15 seconds. Click that exact
fixture window when prompted. Only after it is foreground does the test use
`RequireFocused` to move the pointer to the fixture center, send Enter down/up, and
send the fixed text `system-probe`. It sends no click. A guard restores the previous
cursor position and foreground window on exit when Windows permits restoration.

If the fixture is not focused or target selection is ambiguous, the test stops
before system input. If focus changes after that authorization, integrity blocks
delivery, or a native record count is short, execution fails with the typed
receipt. Do not replace either failure with `AttachThreadInput`, elevation, or
input intended to defeat foreground policy.

## Redaction review

Production input code emits no event, key, text, window-title, or desktop-content
log. The only title retained by the verifier is its own deterministic fixture title.
Interactive evidence may record capability, event counts, typed faults, and cleanup
counts; it must not record input text or unrelated desktop payload.
