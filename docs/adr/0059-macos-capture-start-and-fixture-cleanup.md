# ADR 0059: Make macOS capture start resumable and fixture cleanup finite

- **Status:** Accepted
- **Date:** 2026-09-01
- **Resolves gate:** _none_
- **Supersedes:** _none_

## Context

A deterministic macOS capture scenario gave one open operation a ten-second budget while native start completion arrived after the shim's first two-second wait slice. The shim had already accepted the one allowed start request, but Rust converted the internal slice expiry into terminal `CaptureFailed` instead of rejoining that request under the caller's remaining deadline.

A separate deterministic fixture scenario held `NSRunningApplication.isTerminated` stale after the exact PID lifetime had ended. Cleanup trusted that advisory property, sent a termination request to a dead lifetime, and could recursively schedule another global-queue reaper. The first repaired focused campaign then exposed a transient process-registry gap immediately after a successful workspace launch callback, proving that launch-time registry visibility cannot outrank the callback's retained application identity.

These are reproduced internal lifecycle defects. They do not establish the cause of the archived Apple `CaptureFailed` process, and fixing them does not replace its revision-bound evidence or promote native watcher support. The measurements and exact source identities are recorded in [macOS capture-start and fixture-cleanup remediation](../evidence/macos-capture-start-fixture-cleanup-remediation.md).

## Decision

`mado-pilot-platform-macos` admits exactly one native capture-start submission per session through an `idle` / `pending` / `settled(status)` gate. Wait expiry is an internal bounded slice only: Rust checkpoints the original operation and rejoins the pending or cached result without resubmitting. A successful start records `started` before settlement, and close or drop joins start before submitting at most one stop.

Private fixture containment owns the callback-returned application, PID, and launch time, then revalidates that exact lifetime before every later observation or termination request. A successful launch callback whose workspace registry entry is not yet visible is `unknown`, not lost: unknown lifetime cannot authorize retry or termination. The fixture lookup never consults advisory `isTerminated`. Synchronous containment waits at most one second for exact registry equality; once that identity has been observed, its later registry absence is definitive loss. Containment then sends at most one graceful and one forced request. It attempts this bounded synchronous containment before allocating delayed-cleanup state; any remaining cleanup is observation-only on one serial queue at 100 ms intervals, bounded to twenty observations, with saturating scheduled, active, completed, and exhausted counters exposed only by the private fixture feature. Failure to allocate the delayed record is reported as scheduled and exhausted rather than losing cleanup debt silently.

## Alternatives

- Increase the two-second native wait or add a sleep. Rejected because either substitutes a machine-dependent delay for the caller's explicit deadline and still cannot make an accepted asynchronous request terminal.
- Retry `startCaptureWithCompletionHandler:` after a slice expiry. Rejected because it can submit the same irreversible native transition more than once and split close authority across completions.
- Recover from `SCFrameStatusSuspended`, inflate the public benchmark deadline, or reopen streams. Rejected because the archived suspension is not reproducible or causally tied to either defect; recovery would change product behavior without a red-capable contract.
- Use `isTerminated` as process-lifetime authority and recursively dispatch reapers. Rejected because the property is observably stale and recursive scheduling has no finite ownership or exhaustion accounting.
- Require immediate workspace-registry visibility after the launch callback. Rejected because three fresh processes reproduced a valid callback before that registry view caught up. Treating that gap as death can launch a replacement while the accepted application still lives; treating it as termination authority can message an unproven lifetime.

## Consequences

- Integrators, public Rust callers, and C or C++ consumers change nothing. The private fixture lifetime contract advances the internal shim surface to version 20; no released API, ABI table entry, support statement, retry policy, or normal-build symbol is added.
- A no-deadline caller may keep joining a genuinely pending native start; finite-deadline and cancellable callers retain their original authority and are checked between slices.
- Close and drop preserve a successful late start long enough to stop it once. Native start failures remain normalized statuses and are cached rather than retaining framework error objects.
- Fixture containment may report private exhaustion instead of scheduling indefinitely. Exact identity failure refuses further termination rather than acting on a possible replacement process. Delayed-cleanup allocation failure remains observable after one bounded synchronous containment attempt.
- The one-display focused campaign verifies these lifecycle repairs only. Mixed-scale topology and the uninterrupted cross-host cohorts required by ADR 0057 remain separate support-qualification obligations.
- The archived Apple failure and every frozen benchmark section remain unchanged; this ADR neither assigns that failure a cause nor replaces it with later green diagnostics.

## Verification

- Deterministic native scenarios cover multi-slice and no-deadline success, finite deadline, cancellation, start submission and completion failures, simultaneous joiners, close/drop races, late completion, advisory termination stale in either direction, PID reuse, probe failure, delayed exact death, bounded exhaustion, overlapping releases, transient launch registration, and delayed-cleanup allocation failure on both launch abandonment and handle release.
- The private fixture report asserts zero active or exhausted cleanup and conserved scheduled/completed counts after every focused process. The feature-disabled linkage suite rejects the private counter symbol from release builds.
- Source `843d0143668f9bdbe49482b9a11ebdc15289efbc`, tree `47712d7d10f470d888123c45215241be14aa409c`, passed twenty predeclared fresh sequential `retained_result_mapping` processes with no retry or replacement. All twenty exited zero, conserved backend and cleanup accounting, retained no fixture PID, and reported no correctness, allocation-growth, query, work, or typed-operation failure.
- Exact commands, artifact hashes, the initial campaign failures, accepted process outcomes, privacy limits, and historical boundary are retained in [macOS capture-start and fixture-cleanup remediation](../evidence/macos-capture-start-fixture-cleanup-remediation.md).
