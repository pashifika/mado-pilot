# ADR 0022: Keep Ordinary Windows Background Input System-Only

- **Status:** Superseded by ADR 0027 for the ordinary-window product decision; retained for the historical application-acknowledgement qualification
- **Date:** 2026-08-09
- **Resolves gate:** Phase 2.1 Windows ordinary background-input qualification
- **Supersedes:** _none_

## Context

The Windows Adapter exposes pointer, keyboard, and text delivery to ordinary
windows through the focus-dependent `System` mechanism. Only the dedicated
`MadoPilotInputFixture` also advertises `BackgroundTarget`; that path uses a
versioned `WM_COPYDATA` protocol and acknowledges each accepted event. Phase 2.1
asked whether ordinary windows could safely gain a background capability by
sending documented legacy input messages to one inactive `HWND`.

`PostMessageW` and `SendMessageTimeoutW` do not provide equivalent evidence.
A successful post proves only queue admission. A successful bounded send proves
only that the selected window procedure returned; its `LRESULT` is not a generic
application-consumption acknowledgement. A timed-out send can still take effect.
Raw Input and asynchronous state polling are separate consumer models and do not
become legacy-message consumers merely because the target owns a message loop.

A revision-bound, qualification-only executable exercised these distinctions on
the approved Windows 11 Pro 25H2 physical host. It retained exact owner lifetime,
process creation time, thread, root relationship, class and current metadata,
integrity, deadline, cancellation, and one exact `HWND` before every call. An
owned unrelated process remained foreground, and every row checked the real
cursor and bounded counters for the foreground, sibling, child, replacement,
and restarted-owner fixtures.

The matrix made 212 classified operation calls: 106 admitted posts and 106
bounded dispatch returns. Instrumented ordinary and game-legacy consumers
produced 148 exact legacy observations. Raw Input and state-polling modes
produced 64 observation timeouts with zero corresponding consumer change, even
though their window procedures received the legacy messages. Queue pressure
refused the qualified post with `ERROR_NOT_ENOUGH_QUOTA`. A hung target returned
`ERROR_TIMEOUT`, then observed the message late without changing the committed
terminal result.

The probe could make those distinctions only through application-private
fixture counters. Production discovery has no stable public predicate that
proves an arbitrary non-fixture window consumes a proposed keyboard, text, or
pointer legacy-message sequence at descriptor time and immediately before the
call. Class, title, geometry, PID, transport success, or prior observation do not
supply that predicate.

The frozen gate therefore failed its truthful-receipt and public-eligibility
gates. Exact handle reuse, higher-integrity/UIPI, single-display, deliberately
mixed-DPI, and hosted-CI rows also remained unexecuted and independently prevent
a go result. The full source, executable, host, row, and raw-log bindings are in
[the qualification evidence](../evidence/phase-2-native/windows-background-input-qualification.md).

## Decision

Ordinary Windows targets remain system-only. The Adapter will not expose an
ordinary-window `BackgroundTarget` pair or ship a `PostMessageW` /
`SendMessageTimeoutW` input path for them.

The existing `MadoPilotInputFixture` background capability remains unchanged.
Its acknowledgement is an explicit application-private contract and is not
precedent for uninstrumented windows.

Queue admission, bounded dispatch return, exact fixture observation, application
consumption, timeout, and possible partial effect remain distinct facts.
`InputReceipt.delivered` continues to count only complete logical events. A
message call without an application acknowledgement cannot be promoted to
`delivered`; a call that may have taken effect remains non-retry-safe and must be
reported conservatively under the existing partial-effect rules.

Legacy-message observation does not imply Raw Input, DirectInput, XInput,
asynchronous key-state polling, raw-HID, anti-cheat, or injected-helper
compatibility. No fixture result may be generalized to those consumers or to a
real third-party application.

The qualification probe and its no-go-only fixture modes are disposable. They
must not remain in production packages, exports, examples, capability
descriptors, or runtime feature flags after this decision is recorded. Only the
bounded, privacy-reviewed evidence summary and this ADR are retained.

## Alternatives

- **Advertise background support after a successful post.** Rejected because the
  return proves queue admission, not target observation or application
  consumption. Queue admission can also fail under the documented finite quota.
- **Treat a successful synchronous dispatch as delivered.** Rejected because a
  generic window-procedure return has no acknowledgement semantics. The timeout
  row also proved that a return failure can precede a real late effect.
- **Add admission and dispatch fields to `InputReceipt`.** Rejected as a solution
  to this gate. Richer transport accounting would still not provide the missing
  public consumer-eligibility predicate or prove application consumption.
- **Whitelist applications, classes, titles, or geometry.** Rejected because
  metadata is neither exact target authority nor a stable public compatibility
  contract. Duplicate and replacement rows demonstrated the retargeting risk.
- **Offer an opt-in legacy-message mode for every ordinary window.** Rejected
  because caller optimism does not make an unknown consumer eligible and would
  advertise a capability the Adapter cannot validate before the irreversible
  call.
- **Switch focus, use `SendInput`, attach target input queues, install hooks, or
  inject a helper.** Rejected as different system-input or invasive mechanisms
  that violate the qualification safety boundary and the headless
  foreground-preservation requirement.
- **Retain the probe behind a private feature flag.** Rejected because dormant
  no-go code creates an unreviewed production path and obscures the supported
  capability surface.

## Consequences

- Ordinary window descriptors continue to expose pointer, keyboard, and text
  only through `System`, with focus required.
- Display descriptors remain pointer-only through `System`.
- The exact `MadoPilotInputFixture` class continues to expose its existing
  acknowledged pointer, keyboard, and text `BackgroundTarget` pairs.
- No facade, C ABI, C++ wrapper, capability table, example, package, or support
  statement gains ordinary Windows background input.
- The blocked production phase closes with no follow-up `input-control` proposal,
  alias, compatibility shim, fallback, or optimistic availability claim.
- No product latency or throughput budget is created for the rejected path.
  Finite queue, timeout, observation, memory, worker, and cleanup bounds remain
  correctness requirements for any future investigation.
- Reopening the question requires a new Change and new evidence. At minimum, a
  stable public eligibility contract and an application acknowledgement must
  exist before transport mechanics can be reconsidered; rerunning this private
  fixture matrix alone is insufficient.

## Verification

- The approved-host matrix completed with zero wrong-window or foreground
  observations and no real-cursor movement in the accepted run.
- Exact sibling, child, duplicate-metadata, reparent, replacement, owner-exit,
  cancellation, deadline, queue-pressure, hung-target, late-observation,
  partial-close, and bounded-cleanup cases produced their recorded outcomes.
- Deterministic model and protocol tests passed; Windows Clippy accepted the
  qualification revision with warnings denied.
- The final cleanup removes the qualification binary and verifies that ordinary
  production descriptors remain system-only while the acknowledged fixture
  capability remains unchanged.
