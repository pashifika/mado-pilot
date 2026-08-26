# ADR 0044: Integrated zone OCR target budgets

- **Status:** Accepted; Apple integrated cold-open row superseded by ADR 0045
- **Date:** 2026-08-26
- **Resolves gate:** the remaining explicit bounded-profile one-/three-/eight-zone rows under `G-013`
- **Supersedes:** _none_; ADRs 0037–0043, released native G-004, and ADR 0041 singular budgets remain unchanged

## Context

ADRs 0042 and 0043 expose one-to-eight-zone grouped OCR through the explicit ADR 0040 bounded profile, but they intentionally withhold cross-target quality and performance claims. The integration qualification plan fixed the approved hosts, fixtures, native comparator, five fresh 3+20 bounded processes, alternating singular/grouped order, correctness/resource oracles, target caps, and 1.25-times ceiling formula before measurement.

At exact source `180c1b15df1a4c29bd6df14a790b9d18ac831bae` and tree `479e4109d39c76adfb745d717648948d060c79b2`, the approved Apple M1 Pro executable `6ce1df5bba8bc555fa961af366b0386333e6baeebd7c9483b1be9da39f16c792` and Windows Core i7-12700KF executable `b34b99eb7dcb3870edbd768055428be655e1e45ad125400d3b999bfb4da23398` completed the native comparator plus all five bounded/grouped processes without retry or exclusion. Every fixed text, count, order, geometry, confidence, identity, membership, detector/recognizer, mapping, ownership, cancellation, cleanup, heap, growth, and resident gate passed. The complete observations and target differences are retained under [`../../rasen/changes/phase-3-1-v0-3-1-integration-and-release/evidence/`](../../rasen/changes/phase-3-1-v0-3-1-integration-and-release/evidence/).

## Decision

Qualify explicit bounded-profile grouped OCR on both release targets for full-frame one-zone equivalence and the fixed semantically distinct non-overlapping three- and eight-zone layouts. Accept the target-specific budgets below. Duplicate, near-equal, adjacent, and overlapping zones retain their structural safety contract but receive no quality or performance claim; callers own their semantic reconciliation.

### Grouped latency ceilings

| Workload | Apple p50 / p95 / maximum | Windows p50 / p95 / maximum |
|---|---:|---:|
| full-frame one zone | 600 / 600 / 625 ms | 900 / 900 / 900 ms |
| three sparse zones | 375 / 375 / 375 ms | 525 / 525 / 525 ms |
| eight distinct zones | 450 / 450 / 475 ms | 600 / 600 / 600 ms |
| dense unique candidates | 600 / 675 / 700 ms | 725 / 750 / 750 ms |
| empty 4K result | 175 / 175 / 175 ms | 300 / 325 / 325 ms |

Each value is 1.25 times the worst corresponding five-process observation, rounded upward to 25 ms. No Apple value exceeds the predeclared 600/750/900 ms caps and no Windows value exceeds the 900/1,000/1,200 ms caps. ADR 0041 singular ceilings stay byte-for-byte and numerically unchanged.

### Target process and lifecycle ceilings

| Metric | Apple | Windows |
|---|---:|---:|
| cold open | 125 ms (superseded by ADR 0045) | 250 ms |
| first close | 2 ms | 8 ms |
| reopen and close | 100 ms | 200 ms |
| final process resident high-water | 587,202,560 bytes | 301,989,888 bytes |
| active cancellation-to-return after native start | 25 ms | 25 ms |
| retained one-zone result completion | 625 ms | 900 ms |

ADR 0045 supersedes only the Apple integrated cold-open row with a 200 ms cache-cold ceiling after final enforcement disproved 125 ms; it leaves the historical ADR 0041 profile unchanged. Close, resident, 20,971,520-byte attributable live-Rust peak, and 4,096-byte post-warm growth ceilings remain the existing ADR 0041 gates. The cancellation ceiling rounds the worst observed 7.057 ms Apple and 4.506 ms Windows durations upward to the next 25 ms. The retained-result ceilings apply the same 1.25-times/25-ms rule to worst observations of 480.789 ms and 702.946 ms. The separate two-second native-admission wait is only a deterministic harness liveness bound and is not a product latency claim.

### Fixed grouped resource ceilings

Every qualified row maps once, performs one direct detector resize/run, recognizes selected unique candidates once in batches of at most six, and completes cleanup once. Immutable replay frames require no producer-surface copy.

| Workload | Exact mapped bytes | Detector / tensor bytes | Recognizer runs | Selected / ignored | Unique / memberships | Result semantic bytes |
|---|---:|---:|---:|---:|---:|---:|
| `zone_one_full` | 2,073,600 | 1312×736 / 11,587,584 | 2 | 8 / 0 | 8 / 8 | 1,548 |
| `zone_three_sparse` | 1,479,200 | 1024×480 / 5,898,240 | 1 | 6 / 2 | 6 / 6 | 1,325 |
| `zone_eight_distinct` | 1,479,200 | 1024×480 / 5,898,240 | 2 | 8 / 0 | 8 / 8 | 1,660 |
| `zone_dense_unique` | 4,147,200 | 1024×480 / 5,898,240 | 2 | 11 / 0 | 11 / 11 | 2,022 |
| `zone_empty_4k` | 33,177,600 | 960×512 / 5,898,240 | 0 | 0 / 0 | 0 / 0 | 512 |

The public contract remains bounded by one-to-eight zones, 1,000 unique candidates, 8,000 memberships, and 5,242,880 immutable-result semantic bytes. These safety ceilings are not a claim that every admitted arbitrary or overlapping layout is qualified. Results retain no frame, mapping, tensor, backend/session, lock, producer slot, or parent handle.

## Alternatives

- **Withhold grouped support because Windows is slower.** Rejected because every mandatory row passed unchanged and every observed latency/process value is below its predeclared cap. The target-specific table records the measured difference instead of hiding it.
- **Use one cross-target budget table.** Rejected because ONNX Runtime, OpenCV, allocator, and operating-system behavior differ materially; using the slower table on Apple weakens regression detection, while using Apple values on Windows rejects valid measured behavior.
- **Use the absolute caps directly.** Rejected because the predeclared formula produces tighter evidence-derived ceilings.
- **Raise or replace ADR 0041 singular budgets.** Rejected because the current integrated executable passed every existing singular ceiling; grouped qualification does not justify rewriting historical evidence.
- **Budget only grouped latency.** Rejected because startup, RSS, heap, growth, cancellation, retained ownership, cleanup, mapping, candidate/membership accounting, and detector/recognizer work are equally load-bearing.
- **Qualify duplicate or overlapping layouts from safety tests.** Rejected because structural safety does not establish application semantics, independent detection pixels, or useful performance for those layouts.
- **Use hosted CI timing or infer the missing target.** Rejected because hosted runners are correctness smoke only and both approved target records now exist.

## Consequences

Integrators must continue to select the bounded profile explicitly and provide the controlled model/runtime assets. Existing default constructors and native G-004 behavior do not change. The qualified surface covers the named fixed non-overlapping layouts, not arbitrary application frames, throughput, queueing, scheduling, or real-time behavior.

The benchmark must fail closed when integrated `--enforce-budgets` is selected, require exact source/tree/host/process identity, preserve every singular ADR 0041 gate, enforce every grouped row exactly once, and enforce target-specific cancellation/retained-result limits. New integrated benchmark profiles record the revision-bound measurements and budgets without editing historical Phase 3 or ADR 0041 profile blocks.

Changing a grouped ceiling, host class, fixture/oracle, profile identity, or support boundary requires a new ADR and fresh both-target qualification. A future broader overlap or arbitrary-layout claim requires its own application-owned quality corpus and budgets.

## Verification

- [`../../rasen/changes/phase-3-1-v0-3-1-integration-and-release/evidence/apple-integrated-quality-180c.md`](../../rasen/changes/phase-3-1-v0-3-1-integration-and-release/evidence/apple-integrated-quality-180c.md) and [`windows-integrated-quality-180c.md`](../../rasen/changes/phase-3-1-v0-3-1-integration-and-release/evidence/windows-integrated-quality-180c.md) retain the exact-source native and grouped quality decisions.
- [`apple-integrated-precursor-180c.md`](../../rasen/changes/phase-3-1-v0-3-1-integration-and-release/evidence/apple-integrated-precursor-180c.md) and [`windows-integrated-precursor-180c.md`](../../rasen/changes/phase-3-1-v0-3-1-integration-and-release/evidence/windows-integrated-precursor-180c.md) retain every process maximum, raw report hash, formula input, and provisional target table.
- [`cross-target-integrated-quality-180c.md`](../../rasen/changes/phase-3-1-v0-3-1-integration-and-release/evidence/cross-target-integrated-quality-180c.md) proves zero deterministic resource-signature mismatch and records every target difference.
- The final benchmark and testkit registries enforce the accepted tables, process/lifecycle/resource bounds, exact workload coverage, and fail-closed target selection. Profile drift tests bind the new benchmark TOML blocks to those constants.
- Release acceptance still requires five fresh integrated `--enforce-budgets` processes from one final executable on each approved host, hosted checks, historical profile digest proof, strict Rasen validation, and clean review.
