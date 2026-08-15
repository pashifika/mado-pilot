# ADR 0029: macOS owning-process input delivery

- **Status:** Accepted — the complete revision-bound native matrix in the
  [observed report](../evidence/phase-2-native/macos-owning-process-qualification.md)
  qualifies all fourteen controlled operation/target/coordinate-space pairs on
  source commit `8dd70810d60c06b298c806ffce16720d0a07e4c2`, source tree
  `1bc47b9cc7caa07f75f7d63f311887124a196a5b`
- **Date:** 2026-08-15
- **Resolves gate:** macOS owning-process `ProcessDirected` publication through
  the recorded native matrix
- **Supersedes:** only ADR 0016's macOS system-only delivery-surface decision;
  its controlled-linkage, non-prompting permission, `System` focus-authority,
  no-private-window-control, and no-implicit-fallback rules remain in force

## Context

The platform-neutral input contract and public C ABI 1.2 already carry
`InputDelivery::ProcessDirected`, `InputAddressScope::OwningProcess`,
`CapabilitySupport::Unknown`, and `SubmissionEvidence::InvocationOnly`
(ADR 0023). Before this decision, the macOS Adapter advertised `System` only
under ADR 0016: macOS has no supported exact-window channel, and `CGEventPost`
reaches whatever is focused.

The rejected `phase-2-2-macos-process-directed-delivery` Change tried to make
public `CGEventPostToPid` safe by admitting a target only while its owning
process had exactly one eligible ordinary window. Its
[observed No-Go report](../../rasen/changes/phase-2-2-macos-process-directed-delivery/evidence/observed-report.md)
proved on 2026-08-14 that starting the required desktop-independent
ScreenCaptureKit stream makes the same target process publish a second
on-screen, layer-zero 66×20 capture-indicator window, so the one-window gate
fails before the production route can be qualified. That result remains valid
for its stricter contract. It also exposes the mismatch that forces this
decision: `CGEventPostToPid` addresses an application process, never one
`SCWindow`, and no window-count admission rule can turn it into exact-window
delivery.

## Decision

macOS additionally advertises input through `ProcessDirected` with
owning-process scope, on these rules:

1. **Process address scope is explicit and caller-owned.** A qualified pair is
   advertised as `ProcessDirected`, `OwningProcess`, `Unknown`, and
   `InvocationOnly`. Posting addresses the current process that owns the
   retained window; routing among that process's windows, responders, queues,
   and handlers is the caller's and target application's responsibility. The
   Adapter never requires the process to have exactly one window and never
   classifies auxiliary or capture-indicator windows to establish authority. A
   caller that requires exact-window consumption rejects the descriptor before
   input.
2. **Retained objects establish authority.** The exact retained logical
   `SCWindow` remains the capture identity, geometry authority, and
   owning-process anchor; the retained original process lifetime is the address
   authority. Numeric PIDs and window numbers narrow lookup only; no PID,
   window number, title, application name, rectangle, or lookalike authorizes
   a replacement, for ordinary events or for cleanup.
3. **One native final commit.** Immediately before each irreversible event, one
   controlled native entry re-establishes retained-window existence, original
   process lifetime and current PID relationship, open/unminimized/on-screen
   state, geometry revision and transform policy, deadline, cancellation, and
   current non-prompting event-post access, then creates and posts the event
   without returning to caller-controlled code in between. macOS offers no
   atomic validate-and-post, so this minimizes rather than eliminates
   target-exit races.
4. **One private event source per sequence.** Each sequence owns one
   `CGEventSourceStatePrivate` source, reused for every ordinary and
   sequence-owned release event and disposed exactly once on every terminal
   path. A nonzero caller activity tag is copied to the source's documented
   `kCGEventSourceUserData` field as non-secret observational metadata; it does
   not participate in authority, admission, posting, or receipt accounting.
   Cleanup releases only state its sequence pressed, through the same route and
   source, and never posts to a replacement process.
5. **Permission and focus are separate.** The public `InputControl` observation
   derives from the non-prompting event-post access check that both macOS
   routes need. Accessibility focus/activation remains a `System`-route
   precondition evaluated only when that route's focus policy requires it; it
   is not a `ProcessDirected` prerequisite. Nothing requests permission,
   presents UI, or opens System Settings.
6. **Receipts report invocation only.** `CGEventPostToPid` returns `void`, so a
   returned call records `InvocationOnly` and the exact invoked prefix, never
   queue admission, exact-window delivery, consumption, or visual success.
   Visual confirmation stays a separate caller operation on strictly newer
   retained frames and never mutates a receipt.
7. **No implicit `System` fallback.** After any possible native effect,
   fallback for the sequence is closed. An explicitly ordered `System` attempt
   may begin only after process-directed preflight proved zero possible effect
   and the System route passes its own focus, geometry, permission, deadline,
   and cancellation checks.
8. **Version-one target scope is unchanged.** Ordinary eligibility remains
   open, unminimized, and on-screen; no off-screen, other-Space, minimized,
   hidden, or closed delivery is added.
9. **Every pair qualifies independently.** Each operation, target class, and
   pointer coordinate-space pair is advertised only after its own mandatory
   native rows pass on the implementation revision while desktop-independent
   capture stays active — including route-wide safety, replacement and
   PID-reuse refusal, foreground preservation, per-pair physical-cursor
   invariance, topology, partial effect, and cleanup rows. An unexecuted row
   leaves its pair unavailable.
10. **The public vocabulary does not change.** Existing Rust, C ABI 1.2, and
    C++ wrapper values and layouts are reused; no enum value, structure field,
    function-table entry, or ABI version is added. The internal macOS shim may
    increment its own versioned, size-checked boundary, which is not the
    public C ABI.

### Supersession scope

This record supersedes the single ADR 0016 decision that no macOS target
advertises `ProcessDirected` for any operation.
Everything else ADR 0016 records stays normative: controlled
loading of AppKit, HIToolbox, and Security.framework from absolute system
paths; non-prompting authorization with no permission request or settings UI;
the `System` route's read-only focus authority and activation limits; the
launch/signature evidence axes; and the rule that no route substitutes for
another without an explicit caller-ordered plan. The rejected one-window No-Go
remains retained, unedited, as the historical result for its stricter contract.

## Alternatives

**Keep the one-window admission gate and classify capture-indicator windows.**
Rejected. It couples input authority to undocumented capture UI behavior, is
spoofable by title/size/position, and still cannot create exact-window routing;
the 2026-08-14 evidence shows active capture defeats the gate structurally.

**Add a new `ApplicationDirected` route or rename `ProcessDirected`.** Rejected
because the existing scope vocabulary already states the owning-process
meaning; a new public route would force an ABI migration without adding any
guarantee.

**Rediscover the target by PID or window number before each event.** Rejected
because both are recycled by the OS and can select a different process lifetime
or logical window; retained-object equality is the only accepted identity.

**Per-event or controller-global event sources.** Rejected: per-event sources
lose coherent sequence modifier/button state and allocate avoidably; one global
source couples concurrent sequence lifecycles and blurs cleanup provenance.

**Set undocumented window-routing hints on events.** Rejected. Public headers
document fields, not routing authority, and a hint would contradict the honest
owning-process contract.

**Use the Accessibility trust check as the sole permission truth for both
routes.** Rejected because it conflates permission to post with the separate
authority the `System` route needs to focus or activate a target.

**Promote a matching newer frame to delivered input, or a visual miss to an
unexecuted receipt.** Rejected: both falsify the observed native boundary, and
a visual miss cannot prove that no other same-process responder reacted.

**Qualify without active capture or via the deprecated Accessibility keyboard
probe.** Rejected because neither exercises the production posting path under
the capture state that invalidated the previous design.

**Fall back to `System` implicitly when process-directed delivery is refused.**
Rejected; it would focus a window the caller asked not to disturb and violate
the existing no-fallback contract after possible effect.

## Consequences

- Callers get a truthful foreground-preserving option and must opt in
  explicitly, accept `Unknown` application compatibility and `InvocationOnly`
  evidence, keep visual confirmation separate, and own any retry policy after
  possible unobserved effect. Callers needing exact-window consumption still
  treat macOS target-directed input as unsupported.
- The retained window stays the capture and geometry authority even though the
  native address is its owning process; a multi-window target process can
  consume events anywhere inside itself, and MadoPilot will not claim
  otherwise.
- The accepted support claim is bound to the fourteen exact pairs in the
  revision-bound observed report. The qualified source advertises each as
  `Unknown`, owning-process scoped, invocation-only, and foreground-preserving.
  Future operation, target-class, coordinate-space, or topology pairs remain
  unavailable until their own mandatory rows pass. A route-wide failure blocks
  every pair; a pair failure blocks only that pair; missing mixed-scale evidence
  is never replaced by same-scale evidence.
- The internal shim gains a size-versioned process-post request and sequence
  source lifecycle, extending the existing layout/version/containment test
  obligations; the released C ABI and its frozen prefixes are unchanged.
- Qualification cost is real: route-wide and per-pair native rows on the
  approved Apple Silicon host across single-display, same-scale, mixed-scale,
  and signed-origin topologies, under sustained active capture, plus the frozen
  pre-measurement performance ceilings.
- Changed together with this decision: the macOS adapter and shim input paths,
  capability mapping, fixture protocol and harness, `docs/architecture.md`,
  `docs/macos-input-verification.md`, `docs/performance.md`, Rust/C/C++ examples
  and migration guidance, and ADR 0016's status line.

## Verification

- The frozen route-wide rows, pair matrix, evidence schema, stop rules, budgets,
  and release-decision rule are reproduced by the runnable commands in the
  [macOS input verification guide](../macos-input-verification.md). The separate
  privacy-reviewed
  [observed report](../evidence/phase-2-native/macos-owning-process-qualification.md)
  binds those commands to the qualified source and artifacts and records
  RW-01–RW-14 passed, all mandatory single-display, same-scale, and mixed-scale
  rows passed, 14 pairs qualified, zero rejected, zero unexecuted, and every
  frozen benchmark gate passed.
- Deterministic controller, native-double, shim layout/version, containment,
  protocol, privacy, linkage, and C/C++ contract suites must cover every
  commit-seam ordering — cancellation, revocation, geometry change, target
  loss, PID reuse, replacement, partial native effect, cleanup, close races —
  and fail if an unqualified pair is advertised, a receipt overstates
  evidence, or a fallback follows possible effect.
- The pre-measurement latency, memory, cleanup, and diagnostic ceilings are
  enforced in-process by the `native-phase2` process-directed workload sets;
  measured profiles become normative budgets only with a committed profile and
  an accepting record, per `docs/performance.md` and ADR 0024/0025 precedent.
- Violations that automation cannot catch — foreground changes, physical-cursor
  movement, privacy leaks in retained evidence — are named stop conditions in
  the procedure and invalidate the run rather than the contract.
