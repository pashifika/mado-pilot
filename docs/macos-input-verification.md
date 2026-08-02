# macOS Input Adapter and Verification

The macOS platform package implements input at the Adapter boundary. Runtime,
public-facade, C ABI, and C++ wiring remain later work, so this is an implemented
Rust platform capability rather than a release-level product claim.

## Capability boundary

Capabilities are operation-and-delivery pairs. macOS publishes exactly one
delivery mechanism, because it has exactly one: `CGEvent` posted at the HID tap,
which reaches whatever is focused.

| Discovered target | `System` | `BackgroundTarget` |
|---|---|---|
| Top-level window | Pointer, keyboard, text; focus required | None |
| Display | Pointer only; no focusable target is implied | None |

There is no fixture class that earns a background capability, and no request may
obtain one. A caller that requires `BackgroundTarget` fails admission with
`UnsupportedCombination` before any event, and a caller that lists it first and
system input second is delivered through system input with the receipt naming the
mechanism that was actually tried. The Adapter never substitutes system input for
background delivery on its own: doing so would focus a window the caller asked not
to disturb.

Every target names `PermissionKind::InputControl` — Accessibility — as the
authorization input needs, separately from the Screen Recording that capture
needs. Naming it is not a claim that it is held.

## Coordinates

All five shared pointer spaces are advertised, because macOS capture publishes an
authoritative per-frame placement. The macOS desktop plane is one continuous space
of points with a top-left origin on the main display and signed coordinates for
anything above or to the left of it — the same plane `CGEvent` accepts, so a
resolved coordinate is posted without rounding. A Retina target's capture pixels
convert through its backing scale, and a point outside the target's own half-open
rectangle is refused rather than clamped into whatever is next to it.

A request still fails when its geometry policy cannot resolve the named frame:

- `ReprojectCurrent` reads the target's live rectangle and backing scale.
- `RequireUnchanged` accepts only a source frame retained for that target whose
  geometry fingerprint still equals the live target.
- `UseFrameSnapshot` uses a retained source transform. A revision that is no
  longer retained is unsupported rather than reconstructed from current geometry.

Geometry is revalidated immediately before every irreversible pointer event, so a
window that moved between resolution and delivery reports `GeometryChanged`
instead of clicking what moved into its place.

## Authorization, focus, and receipts

macOS does not fail a synthesized event from an untrusted process; it discards it
and `CGEventPost` reports nothing either way. The Adapter therefore reads the
Accessibility decision with the non-prompting trust check before every
irreversible event, not once at open. A revocation observed mid-sequence stops
delivery and the receipt carries the count that had already gone out. Nothing here
calls a permission-request API, opens System Settings, or presents any interface;
an unavailable or unreadable state is treated as unauthorized rather than as
permission.

Focus is read from the window server's own front-to-back order: the target is
focused when it is the frontmost window in the ordinary window layer and its
owning process still matches the one discovery recorded. Window names are never
read for this, so it needs no Screen Recording.

- `Preserve` cannot satisfy a focus-requiring system path, so a window request
  using it fails admission.
- `RequireFocused` never activates anything.
- `ActivateIfRequired` asks macOS to activate the *owning application* through
  `NSRunningApplication`, then re-reads the frontmost window for a bounded period
  and reports `FocusRefused` if the intended window did not become frontmost. It
  never claims to have raised one particular window, never passes
  `NSApplicationActivateIgnoringOtherApps`, and never uses the Accessibility API
  to move another application's windows.

A display target has no focus requirement, because nothing about a display is
focusable.

`delivered` counts complete logical events. A text event that reached the target
in part before a later chunk could not be posted is `Partial` even when no logical
event completed, never `Unexecuted`, and no fallback mechanism is tried.
[ADR 0015](adr/0015-partial-native-input-effects-and-receipt-accounting.md)
records why that zero-complete partial outcome is required for safe retry
behavior.

## Keys, modifiers, and text

Everything but a printable character is a fixed hardware key code, transcribed
from the `kVK_` map. macOS defines no key code for F21 through F24, so those are
reported unsupported rather than posted as an undefined code. A `Key::Character`
is resolved through the active keyboard layout with `UCKeyTranslate`, and a
character the layout produces only with modifiers is unsupported: pressing the key
the caller named would deliver a different character. Callers use explicit modifier
events or `InputEvent::Text` instead.

A synthesized event carries exactly the modifiers *this sequence* is holding.
Modifier state is not merged with what the user happens to be holding, so a
sequence asking for a plain keystroke gets one. A release clears its own modifier
on the event that releases it.

Text is posted as `CGEventKeyboardSetUnicodeString` on a key event with code zero,
in chunks of at most sixteen UTF-16 units, and a chunk boundary never splits a
surrogate pair. The shared 4,096-character bound applies.

Scroll uses line units. The platform-neutral convention is positive down and
positive right; Core Graphics counts the opposite way on both axes, and the two
are reconciled in exactly one place inside the shim.

## Sequence-owned state and cleanup

Pressed buttons and keys belong only to the sequence that successfully pressed
them. On a partial stop they are released newest first under the independent shared
cleanup bound: at most 256 releases and no new release after 250 milliseconds.
Cleanup runs under a fresh context derived from the request's clock with no
cancellation, because the request's own interruption is usually why cleanup is
running.

Cleanup deliberately does **not** revalidate focus or geometry. A window that
stopped being frontmost is exactly when a held button matters most. It also never
claims that external keyboard or pointer state was restored: a receipt reports
`Incomplete` for a release the platform refused and `Exhausted` for one that was
never attempted, and the two leave a caller with different options.

## Dedicated fixture

`mado-pilot-macos-input-fixture` creates one controlled window with:

- the exact title `MadoPilot Input Fixture [<pid>]`;
- one fixed fill colour and no other content, so a captured frame of it contains
  nothing from the user's desktop;
- a bounded report of at most 256 observed events, each printed as its kind and
  UTF-16 unit count and never its characters;
- the stable bundle identifier `dev.mado-pilot.macos-input-fixture` when it is run
  from a bundle.

macOS has no background input channel, so the fixture acknowledges nothing and
there is no protocol to version: everything it observes arrived as ordinary system
input. That makes selection the fail-closed step. `select_unique_fixture` requires
a window target, the exact process-qualified title, all three operations over
system delivery, no background delivery, and exactly one match. Zero matches and
several matches both stop before input. A check then confirms the selection against
the window's own deterministic content — `frame_is_fixture_content` requires the
sampled region to be one flat colour within tolerance of the declared fill —
because an application window is not one flat colour.

### Bundling the fixture

Running the bare executable is supported and reports itself as unbundled, which is
what the permission evidence should then say. To give it the stable application
identity macOS records authorization against:

```sh
cargo build --locked -p mado-pilot-platform-macos --bin mado-pilot-macos-input-fixture
APP=target/debug/MadoPilotInputFixture.app
mkdir -p "$APP/Contents/MacOS"
cp crates/platform/macos/bundle/Info.plist "$APP/Contents/Info.plist"
cp target/debug/mado-pilot-macos-input-fixture "$APP/Contents/MacOS/"
```

## Automated macOS checks

Run the focused checks from the repository root:

```sh
cargo check --locked -p mado-pilot-platform-macos --all-targets
cargo test --locked -p mado-pilot-input
cargo test --locked -p mado-pilot-platform-macos --lib
cargo test --locked -p mado-pilot-platform-macos --test native_input
```

Deterministic tests cover the capability matrix, focus outcomes, Retina and signed
multi-display mapping, partial sequences, cleanup completeness and exhaustion,
target loss, cancellation and deadline races, and close. They run against the
driver seam rather than the desktop, because a live host cannot be made to revoke
an authorization or refuse a release on cue.

The native integration test **delivers nothing**. It exercises discovery, the
input provider surface, and the refusals that precede any event. Starting the
fixture window takes focus and is therefore opt-in:

```sh
MADO_PILOT_MACOS_FIXTURE=1 cargo test --locked \
  -p mado-pilot-platform-macos --test native_input
```

## Explicit system-input check

Run this only on an interactive Apple Silicon desktop with Screen Recording and
Accessibility granted to the process that launches `cargo test`:

```sh
cargo test --locked -p mado-pilot-platform-macos --test native_input interactive_system_delivery_targets_only_the_exact_fixture -- --ignored --exact --nocapture --test-threads=1
```

The check starts the fixture, selects it exactly once, and waits 15 seconds. Click
that exact fixture window when prompted; the check probes with `RequireFocused`,
which activates nothing, and every probe before it is focused delivers zero
events. Only after the fixture is frontmost does it send Enter down and up and the
fixed text `system-probe`. It sends no click and no pointer movement.

If the fixture is not focused in time, selection is ambiguous, or Accessibility is
absent, the check stops before system input. Do not replace either failure with a
permission request, a settings prompt, or an activation intended to take focus from
the user.

## Redaction review

Production input code emits no event, key, text, window-title, or desktop-content
log. The fixture prints only its own deterministic title, its process and window
numbers, its launch context, and per-event kind and unit counts. The Objective-C
fixture reads an event's characters solely to take their length and never copies
them out of that block. Interactive evidence may record capability, event counts,
typed faults, and cleanup counts; it must not record input text or unrelated
desktop payload.

## Frameworks

Input adds no crate and no eager framework. `CGEvent`, `CGWindowList`, and the
Accessibility trust check come from frameworks the build script already declares.
AppKit — for application activation — and HIToolbox — for the keyboard-layout
lookup — are opened from their absolute system paths on first use, exactly as
ScreenCaptureKit is, so a headless library carries a load command for neither and
the operation that needed one reports `Unsupported` where it is unavailable.
`crates/platform/macos/tests/linkage.rs` asserts the eager list is unchanged. The
fixture's window is compiled into a separate archive that no released artifact
links.
