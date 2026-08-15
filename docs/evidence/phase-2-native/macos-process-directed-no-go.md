# Phase 2.2 macOS process-directed qualification no-go

This record closes the Phase 2.2 attempt to add macOS `ProcessDirected` input.
The candidate did not satisfy its frozen one-window authority rule while the
selected window was under active ScreenCaptureKit capture. macOS therefore
continues to expose only the previously qualified `System` input route.

## Decision

The result is **No-Go** as of 2026-08-14. No pointer, keyboard, text, or display
operation publishes a macOS `ProcessDirected` capability. The rejected product,
facade, ABI, wrapper, fixture, example, benchmark, and support-documentation
changes are not part of the release tree.

The route would have addressed the process that currently owns a selected window,
not that exact window. Its version-one contract therefore required a fresh public
observation immediately before every irreversible event proving all of the
following:

- the retained logical `SCWindow` is still current and eligible;
- the original owning-process lifetime and current PID still agree;
- exactly one eligible ordinary window belongs to that process; and
- current geometry and non-prompting authorization remain valid.

Zero, multiple, unavailable, or ambiguous eligible-window observations had to
refuse before posting. This reduced wrong-window risk without claiming that
`CGEventPostToPid` was an exact-window API.

## Reviewed candidate and host

| Fact | Value |
|---|---|
| Release base | `ffb1823b68ba632b4fc8e7725361ea4596e220f0` |
| Rejected feature review | [PR #34](https://github.com/pashifika/mado-pilot/pull/34), with its positive qualification claim bound to `b1059cf6239042107bd62373eb65211117beaab9` |
| Accepted candidate revision | None; the earlier passing claim did not enforce the frozen one-window rule |
| Target | `aarch64-apple-darwin` |
| Host | Apple M1 Pro, 10 CPU cores, 32 GiB |
| Operating system | macOS 26.5.2 (`25F84`) |
| SDK / compiler | macOS SDK 26.5 / Apple Clang 21.0.0 (`clang-2100.1.1.101`) |
| Rust | 1.97.1 (`8bab26f4f`) |
| Display topology | Three online, non-mirrored displays: one effective 1× display and two effective 2× displays, including a signed-origin 1×/2× seam |
| Authorization | Screen Recording and Accessibility granted to the process running qualification |
| Fixture | Generated `MadoPilotInputFixture.app`, bundled and structurally valid ad hoc |

PR #34's recorded matrix permitted additional same-process windows and therefore
qualified a weaker contract than the frozen design. Review restored the required
one-window admission predicate before publication and reran the blocking
active-capture row. The result below invalidates the earlier positive matrix; its
sample counts and performance profiles are not release evidence.

## Blocking observation

Before target capture started, the inactive fixture exposed one eligible ordinary
window. Its qualification-only process-directed descriptor was `Unknown`, as
expected.

Starting the required desktop-independent ScreenCaptureKit stream preserved the
retained logical window and owning-process lifetime but published another finite,
on-screen, layer-zero window for the same process. The fresh candidate set became:

| Public observation | Retained fixture window | Additional window |
|---|---:|---:|
| Same owning process | yes | yes |
| On screen | yes | yes |
| Window layer | 0 | 0 |
| Logical extent | 640×452 | 66×20 |
| Position relative to retained window | `(0, 0)` | `(+6, +6)` |
| Retained logical identity | yes | no |

The additional window appeared only after capture began. Its timing, placement,
and appearance were consistent with a ScreenCaptureKit capture status/privacy
indicator, but the public `SCWindow` observations contained no provenance field
that could establish that classification as authority. The No-Go does not depend
on proving which component created it: the required public observation contained
two eligible ordinary windows either way.

Strict authority consequently changed the candidate descriptor from `Unknown` to
`Unsupported` and refused before calling `CGEventPostToPid`. No process-directed
native unit was invoked and the fixture observed no input event. Because the
required source-frame → production input → strictly newer expected-state frame
flow could not begin, both the one-window admission and independent visual-oracle
gates failed globally.

| Operation pair | Decision | Reason |
|---|---|---|
| Window pointer / `ProcessDirected` | Do not ship | Active capture violates one-window admission before posting |
| Window keyboard / `ProcessDirected` | Do not ship | Active capture violates one-window admission before posting |
| Window text / `ProcessDirected` | Do not ship | Active capture violates one-window admission before posting |
| Display / `ProcessDirected` | Unsupported | A display has no owning-process address |

The remaining native matrix rows were not executed after the global stop. Passing
system-input, inactive pre-capture, single-display, same-scale, or mixed-scale
observations cannot substitute for this failed end-to-end row.

## Why metadata exclusion is unsafe

Here, “spoofable” means “not an authority credential,” not merely “malicious code
might choose the same title.” The title, rectangle, visibility, layer, and owning
PID exposed for an ordinary application window are mutable observations. Public
ScreenCaptureKit metadata did not attest that the 66×20 window was operating-system
UI or otherwise unable to receive an event.

A title/size/position exception could therefore remove a real possible receiver
from the admission count. Concrete same-process cases include a modal reconnect
or confirmation dialog, a launcher or anti-cheat status surface, an AppKit panel,
or an in-game overlay. A benign application can expose such a second top-level
window coincidentally; an adversarial application can reproduce every proposed
filter deliberately.

The caller still selected one retained window and correlated capture results with
that window. `CGEventPostToPid` addresses its PID rather than its retained
`SCWindow`, and its void return does not identify which application window or
responder consumed the event. If a second same-process dialog becomes key, an
`Enter`, `Escape`, or text sequence can affect that dialog while MadoPilot reports
only process-level invocation. Removing the dialog from the eligibility count by
heuristic does not remove it from the process's event routing.

## Rejected alternatives

- **Ignore the additional window by title, size, location, appearance, or owner.**
  Rejected because none proves provenance, legitimate application UI can match,
  and OS version, localization, display scale, or placement can change the
  capture-associated window.
- **Ignore every window first observed after capture starts.** Rejected because a
  target-created dialog or overlay could then bypass one-window admission.
- **Treat the retained window and process lifetime as sufficient.** Rejected
  because they prove where the process post is addressed, not which same-process
  window or responder receives it.
- **Stop capture before posting and restart afterward.** Rejected because the
  source frame and strictly newer confirmation frame would no longer belong to
  one maintained retained stream; the workaround breaks the qualification
  contract it is intended to prove.
- **Post first and infer success from a later frame.** Rejected because a wrong
  irreversible event may already have affected another window, and an unchanged
  or changed target frame cannot identify the actual consumer.
- **Fall back to `System`.** Rejected because the caller explicitly selected a
  different route and the input contract forbids implicit fallback after any
  possible native effect.
- **Use private window identifiers, helper injection, an event tap, or a
  fixture-only side channel.** Rejected as a different product/security contract,
  not evidence for a public macOS process-post route.

## Reproduction

The controlled candidate used the generated, signed fixture bundle and ran the
blocking row serially:

```sh
MADO_PILOT_MACOS_FIXTURE_EXECUTABLE="$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
MADO_PILOT_MACOS_QUALIFICATION_TOPOLOGY=mixed-scale \
  cargo test --locked -p mado-pilot-platform-macos \
  --features private-fixture --test native_input \
  process_directed_delivery_qualifies_default_and_game_like_renderers -- \
  --ignored --exact --nocapture --test-threads=1
```

Observed result: the row failed before input because active capture changed
process-directed admission from `Unknown` to `Unsupported`. The bounded raw output
had SHA-256
`16ddeeb33b2334be27b8d469b661b3a2b04807e3da0b8ffe9975ea7b6cc52975`.
It remains session-local because native identifiers excluded by the evidence
privacy rule appeared in the diagnostic trace.

A temporary trace used public ScreenCaptureKit fields only to establish candidate
count and relative geometry. A temporary
`presenterOverlayPrivacyAlertSetting = Never` experiment did not remove the extra
window. Both diagnostics and experiment were removed from product code.

## Consequences and reopening criteria

The architecture and released interfaces remain unchanged: macOS advertises
`System` input only, requires focus for window input, and never substitutes it for
`ProcessDirected`. No permission prompt, Settings navigation, event tap, cursor or
focus mutation, private API, helper injection, or unrelated-user input was added
by this decision.

A future proposal must start as a separate Change. It needs either a supported
public mechanism that provides unforgeable event-destination authority, or an
explicitly different cooperative-application contract whose requirements and
consumption evidence are visible to callers. A title/geometry exception or a
repeat of the invalidated relaxed matrix is not requalification.

## Privacy and applicability

This record retains no pixels, captured-image hashes, input payloads, recognized
text, credentials, raw display identifiers, process identifiers, native window
identifiers, unrelated application identity, or signing identifier. Geometry is
limited to the controlled fixture and expressed relatively where it identifies
the blocking relationship.

The observation is bound to the host and OS above. It does not claim every macOS
release creates the same auxiliary window. It is sufficient for this release
because the candidate had to pass every mandatory row on the declared release
host, and one failed global row blocks publication rather than being averaged
with other results.
