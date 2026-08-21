# ADR 0016: macOS input delivery surface and focus authority

- **Status:** Superseded by [ADR 0029](0029-macos-process-directed-input.md) for
  the system-only delivery surface; retained for the `System` focus authority,
  non-prompting permission, controlled-linkage, no-private-window-control, and
  no-implicit-fallback rules
- **Date:** 2026-08-02
- **Amended:** 2026-08-10
- **Resolves gate:** _none_
- **Supersedes:** _none_

## Context

`mado-pilot-input` keeps the operation kind and the delivery mechanism as
separate axes. Windows currently exposes focus-dependent `System` delivery and
exact-window `WindowMessage` delivery: ordinary retained windows advertise the
latter as unknown-but-attemptable, while the dedicated fixture advertises it as
supported through an acknowledged protocol. Reading that Adapter alone, the
natural expectation is that each platform supplies some exact-window mechanism.

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
  in HIToolbox, the activation call lives in AppKit, and truthful signature
  inspection lives in Security.framework — frameworks a headless automation
  library should not add as new eager load commands for this feature.

`docs/evidence/g-003/` and [ADR 0012](0012-macos-shim-language-and-containment.md)
already settled that the macOS native boundary is one internal C-callable
Objective-C shim, and that ScreenCaptureKit is loaded from an absolute system path
rather than linked, because Cargo does not propagate a dependency's
`rustc-link-arg` to the final link.

The facade integration exposed one more behavior: opening a ScreenCaptureKit
session for a window creates a small same-process, ordinary-layer auxiliary
window ahead of the selected window. Window-server compositing order therefore
cannot, by itself, answer which application window owns keyboard focus. On the
qualified host, the public Accessibility model continued to report the selected
window as focused and the auxiliary window as unfocused.

## Decision

macOS input advertises **system delivery only**. No macOS target — window,
display, or fixture — advertises `InputDelivery::WindowMessage` or
`InputDelivery::ProcessDirected` for any operation kind, and the Adapter never
substitutes system input for either. A request that requires either route fails
admission before any event.

Four rules follow from that surface and are part of the platform contract:

1. **Authorization is the receipt's truth-source.** The non-prompting
   Accessibility trust check is read again immediately before every irreversible
   event, and an unavailable or unreadable state is treated as unauthorized. A
   revocation observed mid-sequence stops delivery and the receipt reports the
   count already delivered. No permission-request API is called and no settings
   interface is presented.
2. **A current logical-window match establishes liveness before focus.** Input
   queries current shareable content within the caller's remaining budget.
   Window number and owning PID narrow the result, but the current logical
   `SCWindow` must equal the object included by the exact `SCContentFilter`
   retained from discovery. Title and rectangle never re-resolve the target.
   Focus then requires all of the following public observations to agree:
   [`kAXFrontmostAttribute`][ax-frontmost] reports that the owning application is
   active; [`kAXFocusedWindowAttribute`][ax-focused-window] names one of the
   application's [`kAXWindowsAttribute`][ax-windows] elements; and the freshly
   verified frame corresponds one-to-one with that element's
   [`kAXPositionAttribute`][ax-position] and
   [`kAXSizeAttribute`][ax-size]. Shareable content is read again after the
   Accessibility snapshot. A changed frame, missing attribute, unequal logical
   window, or zero or multiple geometry matches establishes no focus and delivers
   nothing. Geometry is only a fail-closed join between live public observations
   after retained logical identity has been established; it never selects a
   replacement. Both native observations are caller-bounded.
   `ActivateIfRequired` activates an application, never a window, then
   repeats that read-back for a bounded period and reports `FocusRefused` when the
   exact retained window is not established as focused.
   `NSApplicationActivateIgnoringOtherApps` is not passed and the Accessibility
   API is never used to raise or otherwise move another application's windows.
3. **AppKit, HIToolbox, and the code-signing Security API are loaded, not newly
   linked**, from absolute system paths on first use, exactly as ScreenCaptureKit
   is. The operation that needed one reports an explicit unavailable/platform
   result where it cannot be loaded, and the Adapter's eager framework list is
   unchanged. The interactive fixture's window is compiled into a second native
   archive that no released artifact links.
4. **Bundle launch and code signature are separate evidence axes.** The first is
   `Bundled`, `Unbundled`, or `Unknown`; the second is `Unsigned`, `Invalid`,
   structurally valid `AdHoc`, structurally valid `CertificateBacked`, or
   `PlatformFailure`. The shim uses dynamically loaded public
   `SecCodeCopySelf`, `SecCodeCheckValidity`, and
   `SecCodeCopySigningInformation`, including `kSecCodeInfoFlags`,
   `kSecCodeInfoIdentifier`, and `kSecCodeSignatureAdhoc`. A signing identifier
   is exposed only to deliberate fixture evidence; ordinary diagnostics select a
   reviewed static literal naming the two classifications and never interpolate
   the identifier.

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
Matching by mutable title and rectangle to *select or raise* a window would
replace the retained target authority and remains rejected. The accepted
read-only focus check is narrower: a fresh shareable-content snapshot must
contain a logical `SCWindow` equal to the retained object, its exact current
geometry joins one bounded focus snapshot, and every ambiguous join refuses
input.

**Keep treating the first ordinary-layer Window Server entry as focused.**
Rejected after the facade's simultaneous capture-and-input run showed a
ScreenCaptureKit-associated same-process auxiliary window in that layer. Ignoring
it by size, title, or owner alone would also ignore real application windows and
could send keyboard input to the wrong target.

**Treat an active owning application as sufficient.** Rejected because another
window of that application may hold focus. Application activation remains only a
request; exact window focus still requires the read-back above.

**Link AppKit, Carbon, or Security.framework eagerly for this feature.** Rejected
because it would add load commands to every binary that links the Adapter,
including a headless one, and would change what `tests/linkage.rs` asserts. The
controlled-loading arrangement was already in place for ScreenCaptureKit and
costs one `dlopen` per process and framework used.

**Require a named certificate identity for the OSS fixture.** Rejected because it
would make the reproducible project check depend on a developer's keychain. The
documented fixture mode uses `codesign --sign -`: an ad-hoc seal with a stable
signing identifier and no certificate identity. Certificate-backed builds remain
a distinct reported mode and need their own evidence.

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
  certainty focuses the target itself and uses `RequireFocused`; both policies
  use the same exact, read-only focus authority.
- Focus observation now makes bounded shareable-content and Accessibility
  queries as well as the per-event trust check. This adds no permission: system
  keyboard delivery already requires Accessibility. An application that does not
  publish enough public window geometry is refused rather than guessed.
- The internal shim surface first moved from 2 to 3 when it gained input and from
  3 to 4 when liveness accepted the retained selection and authorization split
  launch/signature axes. It moves from 4 to 5 for the retained-target focus query
  and its caller-bounded native waits. It is internal, is not the public C ABI,
  and is not covered by the ABI compatibility policy; the layout test
  asserts both sides agree.
- The fixture is verified interactively rather than automatically, because macOS
  cannot deliver without focusing. Its ad-hoc bundle signature is assembled and
  structurally verified automatically, but that does not prove a TCC decision,
  Gatekeeper acceptance, or successful input. `docs/architecture.md` states that
  gap rather than averaging it away.
- Changed in the same Change: `crates/platform/macos/` (input, native input,
  fixture protocol, fixture binary, shim surface, build script, bundle metadata,
  tests), `docs/architecture.md`, `docs/macos-input-verification.md`,
  `docs/third-party-dependencies.md`, `CONTRIBUTING.md`, and `README.md`.

[ax-frontmost]: https://developer.apple.com/documentation/applicationservices/kaxfrontmostattribute
[ax-focused-window]: https://developer.apple.com/documentation/applicationservices/kaxfocusedwindowattribute
[ax-windows]: https://developer.apple.com/documentation/applicationservices/kaxwindowsattribute
[ax-position]: https://developer.apple.com/documentation/applicationservices/kaxpositionattribute
[ax-size]: https://developer.apple.com/documentation/applicationservices/kaxsizeattribute
