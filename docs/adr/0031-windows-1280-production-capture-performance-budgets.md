# ADR 0031: Windows 1280x720 production capture performance budgets

- **Status:** Accepted
- **Date:** 2026-08-22
- **Resolves gate:** The Windows 1280x720 production-capture and production-transition portion of [`G-013`](../validation-gates.md#g-013)
- **Supersedes:** none; ADR 0026 continues to govern its separate controlled `native-phase2` and diagnostic profiles

## Context

ADR 0026 accepted the existing Windows controlled native and diagnostic profiles but deliberately left the production-capture matrix open. Those profiles did not expose callback-copy time, copied bytes, detached/staging/total GPU-resource peaks, native process resident memory, natural timer-driven publication, retained-pressure recovery, or the complete startup/resize/target-loss/close lifecycle at exactly 1280x720.

The successor entry points add those observations without relabeling historical
profiles. The production fixture repaints only a deterministic 16x16 patch on
its 16 ms timer, avoiding whole-window flashing while continuing to drive
natural compositor publication. Workload and fixture lifecycle records go to
stderr outside the measured region and identify setup, warm-up, sampling,
completion, child readiness, and bounded child termination.

The original capture and transition profiles remain bound to source `0208798`.
Pre-landing review then found that the capture harness implemented the
detached/staging/total resource ceilings as equality requirements even though
this ADR declares upper bounds. Repaired source
`c6ff39a9461c128d9a53e4896a34cb65e3c419a3`, tree
`8f2766a9b55c9964f57a096a720ec4a404ad3756`, reran all four capture workloads
against unchanged numeric ceilings. The transition profile remains applicable
at its original revision: the complete intervening diff changes
benchmark-only callback correlation, dual-display movement, the capture
resource predicate, and an opt-in availability apparatus, but no transition
workload, oracle, fixture mode, or accepted limit.

## Decision

Accept the 1280x720 capture and transition profiles as target-specific regression evidence for the approved Windows 11 Pro 25H2 host, exact single-display topology, repository fixture, and named workload oracles. They are not game-compatibility or application-facing real-time guarantees.

Latency ceilings use three times the largest applicable p50, p95, and maximum from final source and its two reviewed same-lineage precursors, rounded upward:

| Capture workload | p50 ceiling | p95 ceiling | maximum |
|---|---:|---:|---:|
| `steady_frame_acquisition` | 75 ms | 150 ms | 200 ms |
| `callback_copy` | 0.5 ms | 1.5 ms | 5 ms |
| `latest_acquisition` | 0.005 ms | 0.015 ms | 0.15 ms |
| `cpu_map_bgra8` | 6 ms | 15 ms | 20 ms |

| Transition workload | p50 ceiling | p95 ceiling | maximum |
|---|---:|---:|---:|
| `open_first_frame` | 350 ms | 350 ms | 350 ms |
| `retained_pressure_resume` | 100 ms | 100 ms | 100 ms |
| `resize_recreation` | 250 ms | 350 ms | 350 ms |
| `target_loss_recovery` | 1,250 ms | 1,250 ms | 1,250 ms |
| `close_drain` | 10 ms | 10 ms | 10 ms |

Both profiles require zero correctness failures, at most 4,096 bytes of
post-warm-up allocation growth, at most 32 MiB of live Rust heap, and at most
256 MiB native process resident high-water memory. Mapped-byte ceilings are
exact for each workload. The capture profile additionally requires exactly
3,686,400 callback-copy bytes and nonzero observations no greater than two
detached textures, one staging texture, and five total
producer/detached/staging textures, plus a 0.02 stale-work ratio. A valid
resource optimization below those three count ceilings passes.

ADR 0032 separately accepts the dual-4K profile from its own mixed-DPI signed-origin evidence. No dual-4K ceiling is inferred from this 1280x720 result, and no ADR 0031 ceiling is changed by that later decision.

## Alternatives

- **Reuse the macOS production ceilings.** Rejected. The capture APIs, copy path, fixture geometry, topology, host, and native resource accounting differ.
- **Promote provisional Windows constants before a final-source run.** Rejected. Unmeasured constants and the nonexistent ADR reference were removed before qualification.
- **Measure dual-display arrival and callback copy in separate 600-sample passes.** Rejected. One capture and mapping operation already yields both timing views; duplicating it doubles GPU work and operator-visible runtime without adding evidence.
- **Log every retained sample.** Rejected. Console I/O would perturb sample latency and post-warm-up allocation accounting. Phase-level progress is emitted immediately before and after measured regions instead.
- **Accept terminal output with corrupted or split metadata.** Rejected. Only two direct argument-vector runs with intact parseable profile metadata contributed to the accepted ceilings.

## Consequences

Changes to Windows 1280x720 natural publication, callback detachment, mapping,
retained progress, session startup, resize recreation, target-loss recovery,
close, or native resource lifetime must run the matching profile and satisfy
these budgets. The release benchmark enforces coherent per-frame callback
records, latency, live-heap, resident-memory, stale-work, exact copied bytes, and
nonzero upper-bounded texture-resource counts in addition to unconditional
correctness and allocation-growth gates.

The existing ADR 0026 profiles and every historical source binding remain
unchanged. No Rust, C ABI, or C++ public name or layout changes.

## Verification

The exact source binding, commands, aggregate measurements, lifecycle observations, and privacy review are retained in [`windows-production-capture.md`](../evidence/phase-2-native/windows-production-capture.md). The committed profiles are:

- [`phase-2-production-capture-1280x720-x86_64-pc-windows-msvc.toml`](../benchmarks/phase-2-production-capture-1280x720-x86_64-pc-windows-msvc.toml);
- [`phase-2-production-transitions-1280x720-x86_64-pc-windows-msvc.toml`](../benchmarks/phase-2-production-transitions-1280x720-x86_64-pc-windows-msvc.toml).

Profile-key drift, hard-budget drift, latency-budget parity, workspace tests, workspace clippy, and both hosted release-target jobs cover the code and profile structure. ADR 0032 records the separately accepted physical dual-4K row.
