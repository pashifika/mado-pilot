# ADR 0021: Invalidate Phase 2 Native Performance Evidence

- **Status:** Accepted
- **Date:** 2026-08-09
- **Reopens gate:** `G-013` from [../validation-gates.md](../validation-gates.md)
- **Supersedes:** ADR 0020

## Context

Release review found that the three normative macOS Phase 2 profiles named tree
`a6a3edd6e627eadc9da76785c861136d669e8b05`, while the benchmark source had
subsequently changed. The executable whose measurements ADR 0020 accepted was
therefore no longer the executable in the release candidate.

The same review found observable correctness defects in that current benchmark:
`stimulus_to_frame` accepted any declared fixture fill rather than the state
caused by its stimulus, `latest_acquisition` could repeatedly return the same
frame, and the native benchmark did not invoke the harness's hard-budget
predicate enforcement. A separate owned-window replacement regression also
proved that the `SCWindow` retained by `SCContentFilter.includedWindows` is a
snapshot: it continued to report its old frame and `isOnScreen = true` after the
window closed. The input path had therefore been capable of treating stale
snapshot properties as liveness.

After repairing those defects, a non-normative capture probe on the qualified
Apple M1 Pro host retained 200 samples after 20 warm-ups for each workload. All
three workloads reported zero oracle failures and zero allocation growth, but
the corrected `stimulus_to_frame` row reported p95 `348.982708 ms`,
`13,885,440` mapped bytes per result, and stale-work ratio `0.946149704`.
Those values are not proposed budgets: the probe ran from an uncommitted repair
worktree. They demonstrate that ADR 0020's `70 ms`, `4,628,480`-byte, and `0.02`
ceilings do not describe the repaired workload and cannot remain normative.

## Decision

The three macOS Phase 2 profiles recorded by ADR 0020 remain historical measured
evidence but are non-normative for the current source tree. No former ceiling is
relaxed, copied forward, or treated as a release gate. `G-013` is reopened for
all Phase 2 native workloads on macOS as well as the already-open Windows and
C/C++ common-flow evidence.

A replacement profile may become normative only after the benchmark and product
repair are final, one reviewed commit and tree identify the exact executable,
and every applicable workload is rerun on the approved host. That decision
requires new evidence and an ADR; it cannot be made by editing ADR 0020's
numbers in place.

The benchmark continues to fail its process when either structural hard rule is
violated: `result_correctness == 0` and
`allocated_growth_bytes <= 4096`.

## Alternatives

- Keep ADR 0020 normative until a replacement run exists. Rejected: a known
  source mismatch and known false-positive oracles would present historical
  numbers as evidence for code they did not measure.
- Replace only the capture profile. Rejected: all three profiles name one exact
  benchmark tree, input behavior changed with the liveness repair, and release
  evidence must identify one reviewed source revision across the matrix.
- Promote the repair probe and widen the former ceilings. Rejected: the probe
  deliberately lacks a committed source identity, and accepting the first
  post-fix observation would tune budgets to an implementation without review.
- Delete the old profiles. Rejected: retained measurements explain the prior
  decision and make the source drift auditable. Non-normative metadata states
  their current authority without erasing history.

## Consequences

- Phase 2 cannot exit on the macOS performance claims recorded by ADR 0020.
- The three profile files retain their measurements and former budget blocks,
  but carry `normative = false` and an invalidated status.
- Phase 1 regression profiles must also be rerun at the eventual final source
  revision before release acceptance.
- Input now pays for bounded current-window enumeration before irreversible
  delivery. Any optimization must preserve logical `SCWindow` equality and the
  fail-closed replacement regression; stale retained properties are not a valid
  fast path.
- Hosted CI can verify the repaired code, benchmark predicates, profile metadata,
  and deterministic contracts. It still cannot replace interactive native
  performance or display evidence.

## Verification

- `owned_window_replacement_never_retargets_the_retained_filter` passed on the
  qualified host after the fresh logical-window liveness repair and returned
  `TargetLost` before input.
- A complete repaired `capture` benchmark run produced zero oracle failures and
  zero allocation growth; its non-normative results are summarized above.
- `hard_budget_drift.rs` compares both hard predicate strings with every measured
  profile, while the native benchmark calls `enforce_hard_budgets` before
  printing a report.
- `benchmark_block_drift.rs` keeps the harness and committed profile metadata
  shapes synchronized.
- The Phase 2 exit tasks for native measurement, accepted ceilings, and Phase 1
  regression reruns are open again until revision-bound evidence replaces the
  invalidated records.
