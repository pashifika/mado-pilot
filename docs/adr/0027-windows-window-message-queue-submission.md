# ADR 0027: Permit explicit Windows window-message queue submission

- **Status:** Accepted
- **Date:** 2026-08-10
- **Resolves gate:** _none_
- **Supersedes:** ADR 0022 for the ordinary-window product decision; ADR 0022 remains the historical application-acknowledgement qualification

## Context

ADR 0022 rejected ordinary-window background input because the former public
contract treated application delivery as the success threshold. That conclusion
remains correct: a successful `PostMessageW` proves only queue admission, a
generic window-procedure return is not application acknowledgement, and legacy
messages do not imply Raw Input, DirectInput, XInput, asynchronous-state, raw-HID,
helper, hook, or anti-cheat compatibility.

ADR 0023 subsequently introduced a different public contract. `WindowMessage` is
an explicit delivery route, `CapabilitySupport::Unknown` identifies a safely
attemptable route without consumer proof, `TargetQueueAdmission` identifies the
strongest ordinary-target evidence, and visual progress is a separate
strictly-newer-frame observation. The old system-only decision therefore no
longer answers the current contract.

The implementation plan also assumed that checking a retained window immediately
before posting could prove that no message ever reaches a recycled same-value
`HWND`. Win32 supplies no atomic generation check. Microsoft warns that a foreign
window can be destroyed after `IsWindow` and that the handle can be recycled.
`PostMessageW` accepts an `HWND` but no owner-generation token or compare-and-post
predicate:

- <https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-iswindow>
- <https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-postmessagew>

A production contract that promised generation-atomic exclusion would therefore
be unverifiable and false even if a bounded native stress run happened to observe
no wrong-window message.

## Decision

Ordinary live top-level Windows targets may expose pointer, keyboard, and text
through explicit `WindowMessage` with exact-window address scope,
`CapabilitySupport::Unknown`, focus preservation, and at most
`TargetQueueAdmission` evidence. Display targets, child targets, and targets for
which production authority cannot be revalidated do not expose the route. The
exact `MadoPilotInputFixture` retains its separate supported, versioned
`TargetProtocolAcknowledgement` contract on the same public route.

Before and after every normal or cleanup `PostMessageW`, the Windows Adapter
compares the retained `HWND`, owner process creation identity, owning thread, root
relationship, class, provider identity, and capture-item liveness under the
operation bound. Mutable title and geometry never select a target. Only an
unchanged preflight reaches the post. An accepted post followed by mismatch or
unavailable authority is a possible native effect: the current logical event is
settled conservatively, fallback closes, cleanup may use only the same fenced
route, and the sequence stops.

This pre/post fence is the strongest observable authority Win32 permits; it is
not an atomic handle-generation guarantee. Documentation and diagnostics must
state that residual ABA reuse risk rather than claiming that native tests prove
its absence.

Ordinary production delivery uses asynchronous `PostMessageW` only. It never uses
`SendMessageTimeoutW`, focus activation, cursor movement, `SendInput`,
`BlockInput`, queue attachment, a hook, helper injection, elevation, message
filter changes, broadcast, or thread messages. Ordered fallback is caller-owned
and can advance only after a separately reported, retry-safe no-effect refusal.

## Alternatives

- **Keep ordinary targets system-only.** Rejected for the new contract. ADR 0023
  can truthfully represent unknown compatibility, queue admission, partial native
  effect, and separate visual verification without calling any of them
  application delivery.
- **Promise that immediate preflight prevents every reused-handle delivery.**
  Rejected because Win32 has a documented time-of-check/time-of-use interval and
  no generation-bearing post API.
- **Hold an owner process handle and infer that the `HWND` cannot be reused.**
  Rejected. The handle distinguishes owner lifetime but does not lock the window
  object or make `PostMessageW` conditional on that process identity.
- **Use a synchronous call to close the identity interval.** Rejected.
  `SendMessageTimeoutW` still accepts only `HWND`, can take effect after timeout,
  and a generic `LRESULT` has no consumption semantics.
- **Whitelist class, title, geometry, or prior visual success.** Rejected. Those
  facts are descriptive and can be duplicated or changed; none grants current
  target authority or stable consumer compatibility.
- **Activate the target or fall back to `SendInput` internally.** Rejected as a
  different route that violates explicit delivery selection and foreground/
  physical-cursor preservation.

## Consequences

- Callers must explicitly select or order `WindowMessage`; it is never an
  optimistic system-input replacement.
- A complete ordinary receipt can coexist with no application or visual change.
  Callers that need effect evidence must evaluate a strictly newer frame.
- Queue pressure, target loss, cancellation, deadline, partial UTF-16/native
  representation, cleanup, and fallback closure remain visible at logical-event
  granularity.
- A native handle-reuse stress pass is safety evidence for the tested run, not a
  proof of an unavailable atomic Win32 property. The residual race and every
  unavailable topology/integrity row remain in the support limitations.
- ADR 0022 retains its measurements, negative consumer matrix, fixture-only
  acknowledgement reasoning, synchronous late-effect evidence, and historical
  no-go conclusion for the stronger application-acknowledged contract. Only its
  system-only consequence is superseded.
- The Windows package gains no DLL, service, hook, helper, privilege manifest,
  injected component, probe feature, or new external dependency.

## Verification

- Deterministic authority tests cover sibling/child metadata, reparenting,
  replacement, owner restart, pre/post mismatch, cancellation, deadline, and
  unavailable observations without native posting.
- A fake post source covers queue success/refusal, immediate error capture,
  partial multi-unit events, indeterminate post-fence outcomes, fallback closure,
  cleanup accounting, terminal races, and repeated close.
- Native tests use the public route and fail on any observed sibling,
  replacement, restarted-owner, foreground, wrong-process, or physical-cursor
  effect. They include bounded handle-reuse stress and record the residual API
  limitation rather than translating a pass into atomic proof.
- Descriptor, facade, ABI 1.2 C, C++ RAII, example, diagnostic privacy, package,
  and stale-vocabulary checks enforce the public distinction between ordinary
  queue admission and fixture acknowledgement. Frozen ABI 1.0 remains unchanged
  and minor 1 remains rejected.
- The revision-bound procedure and mandatory host/topology rows are tracked in
  [`verification-procedure.md`](../../rasen/changes/phase-2-2-windows-window-message-delivery/evidence/verification-procedure.md).
