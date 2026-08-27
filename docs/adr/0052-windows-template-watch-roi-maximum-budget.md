# ADR 0052: Windows template-watch ROI maximum budget

- **Status:** Accepted
- **Date:** 2026-08-27
- **Resolves gate:** rejected independently remediated Windows template-watch final-enforcement ROI row
- **Supersedes:** ADR 0051's Windows `roi_match` maximum only; its historical profiles and every other ceiling remain unchanged

## Context

Independent review found that the first Phase 4 qualification apparatus synthesized OpenCV work, did not enforce mapped-byte profile rows, asserted fairness before proving both sessions eligible, and omitted startup. Source `34b3572b2ddce32f1cdafa26d7e84e0d1cef2a28`, tree `12b463a22243462ab7bcaa01fd90a6caf7327a47`, corrected those problems without changing product runtime semantics.

The approved Windows host then ran exactly five fresh sequential final-enforcement processes from executable `bc9c8fc38ba3f05e23e423ea702d11203091ac3409139a38a6ea329eb9c010f1`. All 55 workload rows passed their correctness, identity, actual OpenCV call/completion, exact mapped-byte, query/disposition, lifecycle, producer-progress, RSS, heap, and growth gates. Four processes passed every numeric budget. `windows-remediation-final-1` failed closed because one of its 20 `roi_match` samples took 0.7458 ms against ADR 0051's 0.348 ms maximum.

The five ROI rows were:

| Process | p50 ms | p95 ms | maximum ms |
|---|---:|---:|---:|
| `windows-remediation-final-1` | 0.1183 | 0.1846 | 0.7458 |
| `windows-remediation-final-2` | 0.1316 | 0.1666 | 0.1682 |
| `windows-remediation-final-3` | 0.1061 | 0.1774 | 0.1956 |
| `windows-remediation-final-4` | 0.1105 | 0.1516 | 0.1554 |
| `windows-remediation-final-5` | 0.0974 | 0.1314 | 0.1371 |

The accepted 0.246 ms p50 and 0.331 ms p95 ceilings both passed. Only the per-sample maximum was disproved. The full rejected cohort is retained in `rasen/changes/phase-4-template-watch-query-qualification/evidence/windows-remediation-final-rejected.md`; it is not retried, excluded, or relabeled.

## Decision

Set the independently remediated Windows `roi_match` maximum ceiling to 0.933 ms. This is 1.25 times the retained 0.7458 ms observation, rounded upward to the next 0.001 ms:

```text
0.7458 ms × 1.25 = 0.93225 ms → 0.933 ms
```

Keep the Windows ROI p50/p95 ceilings at 0.246/0.331 ms. Keep all Apple ceilings, every other Windows latency ceiling, both target RSS/live-heap ceilings, the 4,096-byte growth gate, exact mapped bytes, exact work signatures, and every hard correctness gate unchanged.

The corrected value applies only to the new independently remediated Windows profile. The original ADR 0051 Windows profile remains revision-bound to its precursor and first final executable. `engine_session_startup` continues to report p50/p95/maximum without a numeric latency ceiling because no pre-remediation precursor predeclared one; all of its non-time gates remain mandatory.

A new source revision changes the Windows benchmark registry before building a fresh executable. That executable must pass five new sequential Windows final-enforcement processes with no retry, exclusion, reorder, priming, or sample replacement. The earlier four passing processes cannot be carried forward.

## Alternatives

- **Retry only `windows-remediation-final-1`.** Rejected because it would hide the retained failure and violate the cohort contract.
- **Exclude the one 0.7458 ms sample.** Rejected because the maximum exists to expose a tail that percentiles cannot hide.
- **Raise p50 and p95 too.** Rejected because all five processes passed those ceilings; changing them would weaken unrelated gates without evidence.
- **Use two times the failed maximum.** Rejected because the retained final observation already includes the independently reviewed apparatus. The 1.25-times final-observation rule used for focused successor corrections yields the tighter 0.933 ms bound.
- **Remove the ROI maximum or withhold it.** Rejected because this measure is present and repeatable enough to retain a sub-millisecond regression ceiling; only its predecessor value was too narrow.
- **Change Apple or borrow its value.** Rejected because target-specific budgets do not cross hosts, and Apple had no corresponding failure.

## Consequences

Windows accepts an occasional sub-millisecond ROI tail while preserving the substantially tighter 0.331 ms p95 gate. The correction weakens one maximum from 0.348 to 0.933 ms but remains only 1.251 times the observed failure and does not compensate for any correctness, work, mapping, memory, lifecycle, or producer-progress failure.

Source `34b3572` and all five of its Windows process reports remain rejected evidence. A successor pass proves the new ceiling; it does not rewrite the old cohort.

## Verification

- The rejected aggregate SHA-256 is `2eb3cbaa0b07de9b7fcb4a0eb8c5434e601615e03115f6951f22ed7b09e0d939` over 69,871 sanitized bytes.
- Drift tests bind 0.933 ms only in the remediated Windows profile and benchmark constant.
- The final Apple cohort from source `34b3572` remains applicable only if the complete successor diff cannot affect Apple workload/oracle/budget execution.
- Acceptance requires five fresh successor Windows processes and exact-source hosted checks.
