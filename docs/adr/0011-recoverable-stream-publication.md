# ADR 0011: Recoverable stream publication for replay reservations

- **Status:** Accepted
- **Date:** 2026-07-30
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

The ownership change and its accessor coverage are implemented at reviewed
executable source revision `8d7973e`. On the named Windows host, the second of
two consecutive full 20-warm-up, 200-sample deterministic-slice runs measures
`replay_open` p50 at `0.000800 ms`, p95 at `0.000900 ms`, and peak live heap at
`94,702 bytes`, with zero correctness failures and zero allocated growth. The
paired C-boundary run satisfies every existing Windows budget.

The same two benchmarks also ran twice on the named Apple Silicon host at
`8d7973e`, with the second runs recorded. There `replay_open` p50 and p95 are
both `0.000750 ms`, peak live heap is `95,118 bytes`, and every correctness,
growth, latency, mapped-byte, and peak-allocation budget passes. Both benchmark
pairs retain matching fixture hashes.

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
remains `0.0025 ms`, and the benchmark fixtures remain unchanged. The four
profiles carry regenerated native evidence from both release targets at the
same reviewed source state, so this decision is accepted.

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
- Performance acceptance requires all four Phase 1 profiles to be regenerated
  from one reviewed source state. This costs one native Windows run and one
  native Apple Silicon run for each benchmark, even though the direct regression
  is in `replay_open`; this decision's evidence has paid that cost.
- Copy removal restores the Windows ceiling without relaxing any budget. A
  future regression must again be isolated rather than accommodated by moving
  the ceiling.

## Current verification

The named Windows host passes the workspace architecture check, formatting,
warning-denied clippy, all-target tests, doctests, warning-denied documentation,
dependency policy, and the C/C++ ABI consumer suite. Native Apple Silicon
verification at `8d7973e` passes the same eight-step sequence: 680 tests pass,
one platform-specific test is ignored, and the frozen v1 header and both CMake
consumers pass.

Structural evidence is linked directly to the implementation tests:

- [`stream.rs`](../../crates/automation/capture/src/stream.rs) covers closed,
  malformed, and inconsistent-geometry ownership recovery; state-atomic
  refusal; legacy `publish` compatibility; and pixel-redacted refusal
  diagnostics.
- [`provider.rs`](../../crates/adapter/replay/src/provider.rs) covers successful
  allocation transfer, exact interruption and refusal rollback, caller-clock
  lock discipline, and ordered exactly-once concurrent publication.
- [`capture_contract.rs`](../../crates/adapter/replay/tests/capture_contract.rs)
  runs the shared capture contract against both memory and directory replay
  sources.

The synchronized acceptance profiles are:

- [`phase-1-deterministic-slice-x86_64-pc-windows-msvc.toml`](../benchmarks/phase-1-deterministic-slice-x86_64-pc-windows-msvc.toml)
  and
  [`phase-1-c-boundary-x86_64-pc-windows-msvc.toml`](../benchmarks/phase-1-c-boundary-x86_64-pc-windows-msvc.toml);
- [`phase-1-deterministic-slice-aarch64-apple-darwin.toml`](../benchmarks/phase-1-deterministic-slice-aarch64-apple-darwin.toml)
  and
  [`phase-1-c-boundary-aarch64-apple-darwin.toml`](../benchmarks/phase-1-c-boundary-aarch64-apple-darwin.toml).

Each benchmark ran twice on each named host with the committed fixture hashes,
20 discarded warm-ups, and 200 retained samples; the second run is recorded.
Every workload reports `result_correctness = 0` and
`allocated_growth_bytes = 0`, and all latency, mapped-byte, peak-allocation, and
iteration-span budgets pass.

Two macOS measurements moved enough to record explicitly. `load_package_archive`
peak live heap rose to `201,987 bytes`, which is the accepted cost of ADR 0010's
single in-memory archive copy and not this change. `load_package_directory`
latency rose from `0.186625 ms` to `0.302500 ms`, reproduced in both runs and
inside its retained `0.6 ms` ceiling; no reviewed change touches directory
loading, so the standing background load on that host is the candidate
explanation. Neither ceiling was re-derived: a refreshed profile keeps the
ceilings ADR 0008 set.

On Windows, `load_package_archive` peak live heap similarly rose from `193,212`
to `201,787 bytes`; the tracked archive's 8,449 bytes plus allocation rounding
explain the deterministic delta. The C-boundary profile records small
deterministic allocation shifts from the intervening C ABI validation work. No
ceiling was re-derived, no fixture hash changed, and no budget was relaxed.

## Verification

ADR 0011 is accepted because the same change includes all of the following:

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

These conditions are satisfied by the linked structural tests, native
verification, and four synchronized profiles. This record is therefore evidence
that the replay reservation regression is resolved without changing its budgets.
