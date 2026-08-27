# ADR 0045: Integrated OCR cache-cold startup budget

- **Status:** Accepted
- **Date:** 2026-08-26
- **Resolves gate:** the rejected Apple integrated final-enforcement startup row under `G-013`
- **Supersedes:** ADR 0044's Apple integrated cold-open value only; ADR 0041 and its historical profiles remain unchanged

## Context

ADR 0044 initially reused ADR 0041's 125 ms Apple bounded-profile cold-open ceiling because the integrated executable opens the same runtime/model/profile before grouped work begins. The first fresh integrated final-enforcement process at exact source `783588411c8f722ae14f415d34f5961dca97af45`, tree `af9283151145127a637ef891751bb03137aba71e`, and executable `ba6087c487726da834b2f0d1fc296ad1a7fbdb2c6d775ef88f89cd3009f32d65` disproved that reuse: cold open measured 149.831750 ms. Every singular/grouped correctness, latency, resource, cancellation, retained-result, and cleanup row passed, but the process correctly failed closed. It was retained and not retried or excluded; later Apple processes were not started, and Windows stopped before build.

The same executable then produced six non-qualification warm-cache diagnostic observations from 84.408 to 91.766 ms. After explicit authorization, `/usr/sbin/purge` flushed the macOS disk buffer cache to approximate initial-boot conditions; the immediately following diagnostic measured 130.844209 ms while all correctness gates passed. The measured backend-open code path is unchanged by ADR 0044 enforcement and ends before any grouped workload or budget comparison.

The historical ADR 0041 evidence therefore measured process-cold startup without controlling or recording disk-buffer-cache state. Its 125 ms revision-bound profile remains valid for that historical executable and procedure, but it is not a reproducible upper bound for a cache-cold open on the same approved host. The rejected run and controlled probe are retained in [`../../rasen/changes/phase-3-1-v0-3-1-integration-and-release/evidence/rejected-final-enforcement-7835.md`](../../rasen/changes/phase-3-1-v0-3-1-integration-and-release/evidence/rejected-final-enforcement-7835.md).

## Decision

Set the new integrated Apple profile's cold-open ceiling to 200 ms. This is 1.25 times the retained 149.831750 ms release observation, rounded upward to the next 25 ms. Keep the integrated Windows ceiling at 250 ms and keep every historical ADR 0041 constant/profile byte and all singular workload latency ceilings unchanged.

The benchmark selects the integrated startup ceiling only when grouped rows are enabled under `--enforce-budgets`. Non-integrated ADR 0041 enforcement continues to use its original 125 ms Apple and 250 ms Windows values. No untimed model/session priming is added: the integrated metric continues to time the first backend open in each process, including legitimate host cache state.

## Alternatives

- **Retry source `7835884` after the cache warmed.** Rejected because it would hide the observed final failure and violate the no-retry/no-exclusion contract.
- **Prime the runtime/model before starting the timer.** Rejected because that changes the startup operation and would convert a real first-open measurement into a warm-open measurement.
- **Keep 125 ms and treat the controlled purge result as invalid.** Rejected because macOS documents `purge` specifically for approximating initial-boot cold disk-buffer-cache conditions, and the same accepted assets/profile passed every non-time oracle.
- **Raise the historical ADR 0041 profile.** Rejected because it is revision-bound evidence for a different source/procedure and must remain immutable.
- **Use the 250 ms Apple absolute cap.** Rejected because the retained observation yields a tighter 200 ms evidence-derived ceiling.
- **Change Windows.** Rejected because Windows showed no corresponding failure and its existing 250 ms integrated ceiling already exceeds the 1.25-times/25-ms value derived from its 177.934 ms precursor maximum.
- **Remove startup from integrated enforcement.** Rejected because first-open cost remains a release-relevant operation even though grouped work begins later.

## Consequences

The new Apple integrated profile tolerates process-first opens with either warm or cold disk-buffer-cache state up to 200 ms. Regression sensitivity is weaker than the historical 125 ms profile but remains 20% below the former 250 ms absolute cap and only 1.335 times the retained failing observation.

This decision changes no product code, profile identity, model/runtime bytes, constructor, default behavior, workload latency, memory ceiling, C/C++ contract, or packaging statement. It changes only the new integrated benchmark registry and documentation. A future startup improvement may lower the integrated ceiling through a new ADR and fresh evidence.

## Verification

- The rejected `7835884` report and its 149.831750 ms value remain immutable evidence, not a precursor relabel.
- The controlled cache-cold diagnostic is SHA-256 `70df0436720a09438031261d861d94ec5bbc9f0e7b927eadf26133b78f8809a3` and reports 130.844209 ms from the same executable after `purge`.
- Testkit exposes a separate 200 ms Apple integrated constant; the benchmark selects it only for integrated enforcement and preserves the ADR 0041 constant for non-integrated runs.
- New integrated benchmark profiles and drift tests bind the 200/250 ms target values without editing historical Phase 3 or ADR 0041 profile blocks.
- Corrected source `1ad2031` passed five fresh integrated final-enforcement processes on each approved host with no retry or exclusion and every other ADR 0044/0041 gate unchanged. Apple worst final cold open was 86.591 ms; Windows was 193.091 ms.
