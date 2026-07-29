# ADR 0011: Recoverable stream publication for replay reservations

- **Status:** Proposed
- **Date:** 2026-07-29
- **Resolves gate:** _none_
- **Supersedes:** _none_

## Context

Before this change, the Phase 1 replay Adapter reserved the next finite source
frame before final deadline and cancellation arbitration, then asked
`mado-pilot-capture`'s `StreamState` to publish it. Publication consumes the
owned pixel buffer. Because the existing publication interface returns only an
error on refusal, the replay Adapter deep-copied the head frame before that call
and left the original queued until publication succeeded.

That copy preserves the required transaction but regresses the accepted Phase 1
profile. On the named Windows host at HEAD `f9538fe`, two consecutive full runs
measured `replay_open` p95 at `0.0198 ms` and `0.0157 ms` against the committed
`0.003 ms` ceiling, with `119,062 bytes` at peak in both runs. The committed
no-copy profile records `94,670 bytes`, and the deterministic difference is
approximately one 96×64×4 frame allocation. The same additional allocation is
present on macOS, where allocator behavior keeps latency within its looser
margin.

The ownership change is implemented at source revision `237ac3e`. On the named
Windows host, the full 20-warm-up, 200-sample deterministic-slice run now
measures `replay_open` p50 at `0.000800 ms`, p95 at `0.001000 ms`, and peak live
heap at `94,702 bytes`, with zero correctness failures and zero allocated
growth. The full C-boundary run from the same implementation also satisfies
every existing Windows budget. Both fixture hashes remain unchanged. This
restores the Windows behavior but is not yet acceptance evidence: the matching
Apple Silicon runs have not been produced, so the four committed profiles have
intentionally not been changed.

ADR 0008 requires a crossed regression ceiling to be investigated rather than
accommodated and requires all four Phase 1 profiles to be regenerated together.
The detailed change design and requirements are in
`rasen/changes/phase-1-replay-reservation-copy-tuning/`.

## Decision

`mado-pilot-capture` adds one recoverable publication operation at the
existing stream-publication seam. It returns an immutable published `Frame` on
success, or a `RefusedPublication` owning both the unchanged `Publication` and
its public `Error` on refusal. The existing `StreamState::publish` operation
keeps its signature and behavior and delegates to the recoverable operation,
discarding returned ownership when its caller does not need it.

`mado-pilot-adapter-replay` moves the finite sequence's head frame into its
reservation. A successful publication consumes it once; cancellation, deadline
expiry, or publication refusal moves that exact allocation back to the source
head before the reservation is released. Stream validation remains authoritative
inside `mado-pilot-capture`, and queue and stream locks remain unnested.

The committed Windows `replay_open` ceiling remains `0.003 ms`, the macOS ceiling
remains `0.0025 ms`, and the benchmark fixtures remain unchanged. The decision
is accepted only with regenerated native evidence from both release targets.

## Alternatives

**Change `StreamState::publish` to return recoverable ownership directly.**
Rejected because it changes an existing reviewed public Rust interface and makes
every Adapter handle ownership recovery it may not need. An additive operation
keeps existing callers source- and behavior-compatible.

**Validate in the replay Adapter and then move into ordinary publication.**
Rejected because validation cannot remove a close race or identity exhaustion,
and copying authoritative stream rules into one Adapter would weaken the
publication seam.

**Use shared `Arc` pixel storage throughout `Publication` and `Frame`.** Rejected
because it changes a broad public representation and constrains future native
frame storage to solve one ownership transfer.

**Adopt a different global allocator or a Windows-private heap.** Rejected
because either changes process-wide or platform-specific behavior, introduces
new review and deployment obligations, and retains the unnecessary copy.

**Shrink the benchmark fixture or raise the Windows ceiling.** Rejected because
both accommodate the regression instead of restoring the accepted workload.

## Consequences

- Existing capture Adapters continue using `StreamState::publish` unchanged.
  Adapters that transfer expensive owned storage may opt into recoverable
  publication.
- The capture Rust surface gains one additive method and one owned refusal type;
  the C ABI table, C layouts, and C++ wrapper do not change.
- Publication must complete all fallible validation before moving pixel storage
  into the committed frame. A private validated frame-construction path becomes
  part of the capture implementation, not its public interface.
- Replay reservation rollback becomes ownership-based rather than copy-based.
  The Adapter must keep exact frame order under cancellation, refusal, panic-free
  normal errors, and concurrent advances.
- No dependency, allocator, fixture, release target, or minimum operating-system
  version changes.
- All four Phase 1 profiles must be regenerated from one reviewed source state.
  This costs one native Windows run and one native Apple Silicon run for each
  benchmark, even though the direct regression is in `replay_open`.
- If copy removal does not restore the Windows ceiling, the remaining operation
  admission and stream-state changes must be isolated before this ADR can become
  Accepted; the budget is not relaxed automatically.

## Current verification

At `237ac3e`, the named Windows host passes the workspace architecture check,
formatting, warning-denied clippy, all-target tests, doctests, warning-denied
documentation, dependency policy, and the C/C++ ABI consumer suite. Structural
tests directly prove successful allocation transfer, exact interruption and
refusal rollback, ordered concurrent publication, state-atomic recoverable
refusal, legacy publication compatibility, and pixel-redacted diagnostics.

The Windows deterministic-slice and C-boundary benchmark runs use the committed
fixture hashes, 20 discarded warm-ups, and 200 retained samples. Every workload
reports `result_correctness = 0` and `allocated_growth_bytes = 0`; all latency,
mapped-byte, peak-allocation, and iteration-span budgets pass.

Native Apple Silicon correctness and both full Apple Silicon benchmark runs are
still required. Until those results exist and all four profiles are regenerated
together, this ADR remains Proposed.

## Verification

ADR 0011 becomes Accepted only when the same change includes all of the
following:

- Capture-interface tests showing that closed, malformed, and other refused
  publications return the exact owned allocation and leave current frame,
  cursor, epoch, sequence, geometry, and lifecycle transitions unchanged.
- Compatibility tests showing that existing `StreamState::publish` callers
  receive the same successful frame or public error without ownership handling.
- Replay tests showing allocation identity across a successful advance, exact
  head restoration after cancellation/deadline/refusal, and ordered exactly-once
  publication under concurrent advances.
- Redacted diagnostic tests proving that recoverable refusal formatting excludes
  pixel contents.
- Focused capture/replay tests and the complete repository formatting, lint,
  test, documentation, dependency-policy, and native-target verification
  sequence.
- Regenerated
  `phase-1-deterministic-slice-{aarch64-apple-darwin,x86_64-pc-windows-msvc}`
  and `phase-1-c-boundary-{aarch64-apple-darwin,x86_64-pc-windows-msvc}`
  profiles with unchanged fixture hashes, zero correctness failures, bounded
  allocated growth, Windows `replay_open` p95 at or below `0.003 ms`, and macOS
  p95 at or below `0.0025 ms`.

Until those files and results exist, this record remains Proposed and is not
evidence that the regression is resolved.
