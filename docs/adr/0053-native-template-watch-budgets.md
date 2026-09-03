# ADR 0053: Accept target-specific native template watcher budgets

- **Status:** Accepted
- **Date:** 2026-08-29
- **Resolves gate:** _none_
- **Direction / Slice:** `version-one-delivery` / `phase-4-native-template-watch-qualification`
- **Depends on:** ADR 0050, ADR 0051, ADR 0052, and precursor source `65da97b06d6de80ea7b59b0b16fd8bc022befea7`

## Context

The bounded Rust template watcher had deterministic replay/OpenCV budgets but no accepted platform-native ScreenCaptureKit or WGC profile. Qualification source `65da97b06d6de80ea7b59b0b16fd8bc022befea7`, tree `72b233d8b2e34ceee6452fde6f5b0c7bc2d3fa92`, runs an ordered 24-workload matrix over repository-owned native fixture pixels. Each target retained five fresh sequential optimized processes with three warmups and 20 retained samples for each of 16 sampled workloads. Eight topology, permission, identity, overload, target-loss, close, and progress rows are deliberately single-run hard gates. No process was retried, excluded, overlapped, reordered, or replaced.

Earlier failed apparatus, rejected cohorts, topology interruptions, and deadlock evidence remain tracked and are not relabeled. The exact accepted inputs are:

- Apple reports `apple-precursor-{1..5}-65da97b.toml` and decision record `apple-precursor-accepted-65da97b.md`;
- Windows reports `windows-precursor-{1..5}-65da97b.toml` and decision record `windows-precursor-accepted-65da97b.md`;
- comparison `precursor-comparison-65da97b.md` and exact numeric input `precursor-comparison-65da97b.json`, SHA-256 `387b1fcd379f9d2cc1f7236beafc7e557040ba7c57b259c9e3cb586224f5a944`.

All paths above are under `rasen/changes/phase-4-native-template-watch-qualification/evidence/`. The ten reports contain 240 rows. All source, match, geometry, state, work, lifecycle, ownership, cleanup, privacy, and producer-progress oracles pass; all 160 sampled rows observed zero post-warm retained allocation growth. Apple and Windows have exact shared controlled-work signatures but materially different capture cadence, latency, mapped-byte totals, live heap, and resident memory. A shared numeric profile would erase useful regression signal.

## Decision

Accept independent absolute latency ceilings for the 16 repeatably sampled native watcher workloads. Each p50, p95, and maximum ceiling is twice the worst statistic from the target's five precursor processes, rounded upward to 0.001 ms:

| Workload | Apple p50 ms | Apple p95 ms | Apple max ms | Windows p50 ms | Windows p95 ms | Windows max ms |
|---|---:|---:|---:|---:|---:|---:|
| `window_absent_current` | 275.276 | 281.558 | 285.297 | 11.094 | 22.476 | 23.093 |
| `window_transient_appearance` | 674.163 | 680.820 | 683.207 | 96.437 | 98.592 | 99.746 |
| `window_persistent_appearance` | 408.569 | 415.749 | 417.577 | 138.452 | 179.120 | 198.373 |
| `window_disappearance_reset` | 938.330 | 945.140 | 945.798 | 175.341 | 184.048 | 224.420 |
| `window_strictly_newer` | 407.096 | 415.379 | 418.778 | 138.245 | 191.042 | 193.623 |
| `window_move` | 564.115 | 599.015 | 604.083 | 291.923 | 336.501 | 359.500 |
| `window_resize` | 574.458 | 598.536 | 609.132 | 229.808 | 264.294 | 276.969 |
| `native_high_rate_slow_backend` | 930.280 | 957.199 | 960.783 | 364.648 | 372.833 | 375.238 |
| `two_query_fairness` | 771.217 | 788.633 | 789.227 | 528.941 | 572.854 | 583.992 |
| `two_session_fairness` | 1605.962 | 2051.324 | 2089.810 | 721.416 | 763.108 | 814.845 |
| `exact_coalescing` | 768.886 | 791.611 | 795.228 | 528.717 | 576.195 | 578.013 |
| `unequal_no_coalescing` | 777.984 | 791.200 | 795.291 | 530.719 | 563.713 | 608.426 |
| `stale_generation` | 797.568 | 805.454 | 809.258 | 638.692 | 683.712 | 694.482 |
| `wait_cancel_deadline` | 888.242 | 908.619 | 913.912 | 704.086 | 721.262 | 735.515 |
| `retained_result_mapping` | 4541.716 | 7221.614 | 7343.780 | 462.957 | 569.525 | 578.660 |
| `fresh_session` | 4357.265 | 7209.869 | 7328.993 | 405.649 | 472.422 | 509.762 |

Accept these target-wide resource ceilings:

| Target | Peak resident bytes | Peak live Rust bytes | Post-warm growth bytes |
|---|---:|---:|---:|
| `aarch64-apple-darwin` | 3581935616 | 364232704 | 4096 |
| `x86_64-pc-windows-msvc` | 1379926016 | 205737984 | 4096 |

Peak resident limits are 1.25 times the worst target-native process high-water rounded upward to 1 MiB. Peak live Rust limits are 1.25 times the worst target-native high-water rounded upward to 4 KiB. Growth remains the predeclared fixed 4 KiB hard gate rather than a value inferred from the observed zero growth.

The following boundaries are intentionally non-numeric or exact rather than inferred ceilings:

| Boundary | Decision |
|---|---|
| single-run gate latency | Withhold p50/p95/maximum ceilings for `environment_identity`, `window_topology_scale`, `display_current_newer`, `permission_availability`, `queue_expiry_overload`, `native_stop_target_loss`, `session_engine_close`, and `producer_progress_cleanup_privacy`; one observation per process is insufficient for a distribution. |
| startup through close | Accept the sampled `fresh_session` latency row above. Withhold a numeric startup or close ceiling from the single-run identity and close gates; continue enforcing their exact lifecycle, ownership, terminal-outcome, and cleanup oracles. |
| mapped/copied bytes | Withhold a cadence-dependent aggregate ceiling. Enforce exact checked mapping accounting and workload-specific source/ROI/geometry invariants before latency, and retain target-native aggregates in the report. Apple and Windows values are not interchangeable. |
| work disposition | Accept exact controlled signatures: fairness and exact coalescing each complete 40 queries from 40 backend admissions plus 40 coalesced dispositions; unequal requests complete 40 queries from 80 admissions with zero coalescing; overload completes 32 queries from two admissions plus 30 queue expirations. For cadence-dependent rows, enforce conservation, finite latest-wins, zero silent query loss, zero unexpected rejection/failure, and `stale_discards == work_superseded`; withhold fixed aggregate count ceilings. |
| producer progress | Enforce source/epoch/sequence/geometry monotonicity, progress while results are retained, bounded cleanup, and zero active backend work. Withhold a publication-rate ceiling because ScreenCaptureKit is continuous and WGC is change-driven. |

Every semantic, resource, and progress gate runs before numeric latency acceptance. Faster output cannot compensate for a false match, stale commit, silent query loss, incorrect mapping, starvation, target-lifecycle error, ownership violation, unbounded retention, privacy leak, or producer stall.

## Alternatives

- **One shared target profile.** Rejected. Apple and Windows differ by capture model and by more than 2.5 times in accepted peak resident memory. A shared ceiling would either reject a correct target or weaken the other target's regression boundary.
- **Turn every observed aggregate into a ceiling.** Rejected. Publication, supersession, stale-discard, mapped-byte, and backend counts legitimately follow target capture cadence. The contract is the exact conservation and terminal-outcome relation, not equality between ScreenCaptureKit and WGC.
- **Infer distributions from single-run hard gates.** Rejected. Five single observations do not establish per-process p50/p95/maximum behavior. Those rows remain stronger semantic gates without a misleading timing promise.
- **Borrow deterministic replay/OpenCV budgets from ADR 0051.** Rejected. Native capture owns permission, topology, geometry, target-loss, producer, and native-frame lifetime costs absent from replay.
- **Select or relax ceilings after final enforcement.** Rejected. That would convert a final regression into a post-hoc budget. Any relaxation requires new precursor evidence and an ADR amendment before rerunning final acceptance.

## Consequences

The final qualification executable and profiles must select the exact Apple or Windows latency and resource values above from the compile target. They must not expose a runtime tuning option or claim a real-time guarantee. Integrators receive no new API behavior; these are repository regression boundaries for the approved host classes, native fixture, topology procedures, and named workloads.

Final acceptance still requires five fresh sequential no-rebuild processes per target after the budget implementation is committed. A failed process is retained and fails the cohort; retry, exclusion, sample replacement, profile relaxation, source drift, fixture drift, host drift, or topology drift cannot repair it.

This ADR does not qualify OCR predicates, callbacks, C/C++, automatic input, arbitrary applications/templates/ROIs, packaging, or a release. Adding a target, changing fixture geometry, changing the workload contract, or widening any accepted ceiling requires additive evidence and an amended or successor ADR.

## Verification

- `native-template-watch --enforce-budgets` must fail hard semantic/resource/progress gates before selecting the compile target's latency table.
- A strict target profile must carry the 24 ordered workloads, 16 sampled latency rows, eight explicitly non-numeric gate rows, resource ceilings, exact workload plan, source/fixture/backend/host provenance, and every exact controlled-work rule in this ADR.
- Drift tests must compare the benchmark's enforced tables and hard predicates with both committed target profiles. Historical benchmark evidence remains byte-identical; native profiles and registry rows are additive.
- Five fresh final processes on each approved target rerun the unchanged committed executable with three warmups and 20 samples, sequentially and without retry, exclusion, overlap, or replacement.
- Precursor derivation is independently reproducible from `precursor-comparison-65da97b.json`; its accepted SHA-256 is recorded above.
