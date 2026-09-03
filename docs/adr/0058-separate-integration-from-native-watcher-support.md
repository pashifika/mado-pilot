# ADR 0058: Separate implementation integration from native watcher support

- **Status:** Accepted
- **Date:** 2026-08-30
- **Resolves gate:** _none_
- **Supersedes:** _none_

## Context

PR #59 implements the native template-watch apparatus and corrective runtime work, and its current candidate passes the applicable local, hosted, policy, and independent-review gates. The frozen qualification still failed: the fifth Apple process ended after a newly opened ScreenCaptureKit stream reported `SCFrameStatusSuspended`. ADR 0057 therefore withholds native watcher support, and the historical failed process cannot be replaced.

Keeping the verified implementation unmerged would make the suspension investigation branch from an older runtime and would conflate source integration with a public support claim. Promoting support to permit the merge would instead erase the failed qualification boundary.

## Decision

Qualification controls support promotion, not integration eligibility. A topic may merge into its `dev/x.y.z` branch when its implemented contracts, protected checks, and reviews pass and tracked documentation continues to state the failed qualification and withheld support.

PR #59 may therefore merge without promoting native WGC or ScreenCaptureKit watcher support. Its Change is archived with result `failed`; suspension observability, diagnosis, and any measured repair proceed in a new Change and topic branch from the integrated `dev/0.4.0` baseline.

## Alternatives

- Keep PR #59 open until a repaired source qualifies. Rejected because the repair would depend on or duplicate an unintegrated, already reviewed runtime and qualification apparatus; it would not improve the validity of the frozen failed evidence.
- Treat protected-check success as native support qualification. Rejected because hosted CI does not execute the approved interactive five-process native matrix and cannot replace Apple process 5.
- Relabel or replace the failed Apple process with later green diagnostics. Rejected by the frozen no-retry and no-replacement protocol; those runs prove intermittence only.
- Implement recovery before integration. Rejected because the current boundary discards non-complete ScreenCaptureKit statuses, so recovery would hide rather than establish the cause.

## Consequences

- The Rust native watcher implementation and qualification apparatus can be reviewed and evolved from `dev/0.4.0`; merging them makes no compatibility or support promise.
- ADR 0057 remains authoritative: native WGC and ScreenCaptureKit watcher support is withheld, and release notes, examples, architecture status, and public claims must retain that boundary.
- The rejected Apple process and every revision-bound benchmark remain unchanged historical evidence.
- The successor Change begins with private, bounded, privacy-safe status and ownership instrumentation. It may alter production behavior or public contracts only after a red-capable experiment identifies the required repair.
- Support can be reconsidered only after a repaired exact source passes fresh uninterrupted five-process cohorts on both approved hosts plus the reviews and protected checks required by ADR 0057.

## Verification

- PR #59's applicable workspace, documentation, policy, Windows, and macOS hosted checks and its independent concurrency and memory-safety reviews must remain green at merge.
- [Phase 4 native template-watch qualification](../evidence/phase-4-native-template-watch-qualification.md) and ADR 0057 retain the Apple terminal-red result and `WITHHELD` support state.
- PR review confirms that the merge description distinguishes integrated implementation, failed qualification, and withheld support.
- The successor Change records its diagnostic source and measurements separately; no successor evidence rewrites PR #59's frozen cohort hashes or outcomes.
