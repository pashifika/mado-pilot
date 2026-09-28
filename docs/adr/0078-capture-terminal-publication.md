# ADR 0078: Commit retained candidates against capture termination

- **Status:** Accepted
- **Date:** 2026-09-28
- **Resolves gate:** none
- **Supersedes:** none

## Context

Capture publication already orders frames and the first terminal fault under
`StreamState`'s mutex. Consumer acquisition previously released that mutex before
its last operation check; a caller clock could record `TargetLost` there and the
call would still return a frame. Runtime OCR only arbitrated explicit close after
backend work. Neither a retained immutable frame nor `is_closed()` closed the
capture-terminal result-publication race.

## Decision

Require `CaptureSession::commit_frame(&Frame, &OperationContext)` and expose it on
runtime `Session` through the Rust facade. `StreamState` checks interruption
outside its mutex, then orders the exact frame's commitment against the stream's
first terminal fault and closing transition under that same mutex. Foreign-stream
frames are refused; older retained frames from the same stream remain eligible.
Every in-tree capture adapter implements the contract without a default fallback.

Runtime acquisition, singular/grouped OCR and template search apply this boundary
after preparing their successful candidate. The monotonic runtime-close check
follows capture commitment without intervening caller work: success proves close
had not started at the earlier capture commitment point; a later close can cause
conservative refusal. Host work and caller clocks never run under a commitment
lock, and commitment adds no frame copy, worker or queue.

A host prepares its bounded candidate before calling `Session::commit_frame` and
uses success as the acceptance linearization point. Terminal-first refuses the
candidate with its original cause; commitment-first permits a historical immutable
value even if termination follows before the call returns. No continuing target
readiness or completed cleanup is promised.

## Alternatives

- Lifecycle polling or a second acquisition: neither atomically orders the
  already prepared candidate against termination; acquisition also changes work.
- A lock guard spanning arbitrary host preparation: blocks terminal handling and
  permits reentrant-clock/backend deadlocks.
- Revoking previously returned frames: breaks immutable retained ownership and
  cannot undo values already observed by a consumer.

## Consequences

Custom Rust `CaptureSession` implementations must add the required method using
their authoritative terminal state. Replay's final consumer refusal does not
rewind a frame already published; prepublication refusal still restores its exact
source reservation. Mapping historical frames and independently completing or
retrying cleanup remain unchanged.

C/C++ synchronous visual operations inherit the runtime fix without adding a C
entry or changing ABI prefixes/layouts. Native adapters share the ordering seam,
but controlled tests do not grant permissions, qualify OS upgrades, or close
native workload/release gates. No dependencies or performance budgets change.

## Verification

The capture and runtime suites exercise terminal-before-commit refusal,
commit-before-terminal retained access, exact-stream identity, cancellation and
deadline refusal, explicit close, and held OCR/template work. The capture suite
includes a reentrant clock that terminates the stream during the last operation
check, reproducing the acquisition failure without sleeps or native authority.

Run the focused tests through the installed-native setup wrapper, then the full
[contributor verification sequence](../../CONTRIBUTING.md#verification):

```sh
python3 tools/setup-native.py -- cargo test --locked -p mado-pilot-capture -p mado-pilot-adapter-replay -p mado-pilot-runtime -p mado-pilot-testkit
```

Actual Windows/macOS capture qualification remains separately authorized,
revision-bound evidence; no controlled result substitutes for it.
