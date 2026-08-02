# ADR 0016: macOS input delivery surface and focus authority

- **Status:** Accepted
- **Date:** 2026-08-02
- **Resolves gate:** _none_
- **Supersedes:** _none_

## Context

`mado-pilot-input` keeps the operation kind and the delivery mechanism as separate
axes, and `mado-pilot-platform-windows` fills both: a target accepts pointer,
keyboard, and text over `InputDelivery::System`, and the dedicated fixture class
additionally accepts all three over `InputDelivery::BackgroundTarget` through an
acknowledged window-message protocol. Reading that Adapter alone, the natural
expectation is that each platform supplies some background mechanism and that a
fixture earns it.

macOS supplies neither. `CGEventPost` injects into the window server's own event
stream and is delivered to whatever is focused; there is no supported per-window
channel an unfocused process may post to, and no fixture the project controls can
create one. Three further macOS behaviors force decisions that Windows did not:

- The operating system does not fail a synthesized event from a process without
  Accessibility. It discards it, and `CGEventPost` returns nothing either way, so
  no return value distinguishes a delivered event from a dropped one.
- Raising one specific window of another application requires the Accessibility
  API and a private identifier to correlate an `AXUIElement` with a `CGWindowID`.
  Activating the owning *application* needs neither.
- The keyboard-layout lookup that maps a printable character to a key code lives
  in HIToolbox, and the activation call lives in AppKit — two frameworks a
  headless automation library should not carry a load command for.

`docs/evidence/g-003/` and [ADR 0012](0012-macos-shim-language-and-containment.md)
already settled that the macOS native boundary is one internal C-callable
Objective-C shim, and that ScreenCaptureKit is loaded from an absolute system path
rather than linked, because Cargo does not propagate a dependency's
`rustc-link-arg` to the final link.

## Decision

macOS input advertises **system delivery only**. No macOS target — window,
display, or fixture — advertises `InputDelivery::BackgroundTarget` for any
operation kind, and the Adapter never substitutes system input for it. A request
that requires background delivery fails admission before any event.

Three rules follow from that surface and are part of the platform contract:

1. **Authorization is the receipt's truth-source.** The non-prompting
   Accessibility trust check is read again immediately before every irreversible
   event, and an unavailable or unreadable state is treated as unauthorized. A
   revocation observed mid-sequence stops delivery and the receipt reports the
   count already delivered. No permission-request API is called and no settings
   interface is presented.
2. **`ActivateIfRequired` activates an application, never a window.** It sends
   `NSRunningApplication.activateWithOptions:` with
   `NSApplicationActivateAllWindows` only, re-reads the frontmost ordinary-layer
   window for a bounded period, and reports `FocusRefused` when the intended
   window did not become frontmost.
   `NSApplicationActivateIgnoringOtherApps` is not passed and the Accessibility
   API is not used to move another application's windows.
3. **AppKit and HIToolbox are loaded, not linked**, from absolute system paths on
   first use, exactly as ScreenCaptureKit is. The operation that needed one
   reports `Unsupported` where it is unavailable, and the Adapter's eager
   framework list is unchanged. The interactive fixture's window is compiled into
   a second native archive that no released artifact links.

## Alternatives

**Advertise background delivery for a macOS fixture class.** Rejected because
nothing could implement it. A fixture could receive a Mach or socket message, but
that would test a private channel this Adapter invented rather than the input path
a caller gets. Advertising a capability whose implementation is a test-only
side-channel makes the descriptor a claim about the fixture, not about macOS.

**Fall back to system input when background delivery is requested.** Rejected for
the reason `mado-pilot-input` states in its own contract: substituting system
input focuses a window the caller explicitly asked not to disturb and injects into
whatever is focused instead. A caller that asked for background delivery did not
ask for that.

**Probe Accessibility once at open instead of per event.** Rejected because macOS
revokes it while a process is running, and because the platform gives no delivery
failure to fall back on. A per-open probe would let a sequence report `Complete`
for events the window server discarded. The cost accepted is one trust check per
irreversible event.

**Raise the specific target window through the Accessibility API.** Rejected
because correlating an `AXUIElement` with the `CGWindowID` this Adapter's
identities are built on needs `_AXUIElementGetWindow`, which is not public API.
Matching by title and rectangle instead would make focus depend on mutable window
metadata, which is exactly what target selection elsewhere in this project refuses
to do. Application-level activation with a read-back is weaker but honest.

**Link AppKit and Carbon eagerly.** Rejected because it would add load commands
for the desktop UI framework and for Carbon to every binary that links the
Adapter, including a headless one, and would change what `tests/linkage.rs`
asserts. The controlled-loading arrangement was already in place for
ScreenCaptureKit and costs one `dlopen` per process.

## Consequences

- Integrators targeting macOS cannot ask for background input and must expect
  system delivery to require focus. A cross-platform caller that wants background
  delivery on Windows and something on macOS must decide explicitly whether it
  permits system input as a fallback; the delivery plan is where it says so.
- The receipt's honesty on macOS rests on a permission check rather than on a
  platform error. If a future macOS release reports delivery failures directly,
  the per-event probe becomes a redundancy rather than the only signal, and the
  rule above can be relaxed with evidence.
- `ActivateIfRequired` is best-effort by construction. A caller that needs
  certainty focuses the target itself and uses `RequireFocused`.
- The internal shim surface version moves from 2 to 3 and gains an input section.
  It is internal, is not the public C ABI, and is not covered by the ABI
  compatibility policy; the layout test asserts both sides agree.
- The fixture is verified interactively rather than automatically, because macOS
  cannot deliver without focusing. `docs/architecture.md` states that gap rather
  than averaging it away.
- Changed in the same Change: `crates/platform/macos/` (input, native input,
  fixture protocol, fixture binary, shim surface, build script, bundle metadata,
  tests), `docs/architecture.md`, `docs/macos-input-verification.md`,
  `docs/third-party-dependencies.md`, `CONTRIBUTING.md`, and `README.md`.
