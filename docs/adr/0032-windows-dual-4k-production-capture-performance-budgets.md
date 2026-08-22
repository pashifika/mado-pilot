# ADR 0032: Windows dual-4K production capture performance budgets

- **Status:** Accepted
- **Date:** 2026-08-22
- **Resolves gate:** The remaining Windows dual-4K production-capture portion of [`G-013`](../validation-gates.md#g-013)
- **Supersedes:** none; ADR 0031 continues to govern its separate 1280x720 capture and transition profiles

## Context

ADR 0031 accepted the Windows 1280x720 production profiles while explicitly withholding the mixed-DPI dual-4K profile until the approved physical topology became available. The holiday availability recorded by the local launch procedure waived the normal shared-display workday exclusion without changing any benchmark acceptance condition.

The approved host exposed exactly two online non-mirrored 3840x2160 displays: a 144 DPI / 1.5-scale primary at `[0,0,3840,2160)` and a 120 DPI / 1.25-scale secondary at `[-3840,0,0,2160)`. The production fixture straddled their signed-origin seam. Each iteration acquired and mapped one strictly newer frame from both display sessions and derived arrival and callback-copy timing from that one system interaction. Separate timing passes were not used.

Two reviewed precursor runs on source `0208798d9542aaae3a956d3e774c9ce57468bc9d` established repeatability before ceilings were added. Final source `121d41a9eea341b7345a8b0dda4918b1f61ec74e`, tree `7e694a070d1e300642033b56aef499b8238c08ca`, then retained 600 samples per display with every proposed gate enforced in-process. All three runs reported zero correctness failures and no positive allocation growth. Every supervised benchmark and fixture process reached a terminal state.

## Decision

Accept the dual-4K profile as target-specific regression evidence for the approved Windows host, exact two-display mixed-DPI topology, signed-origin seam, repository fixture, and named workload oracles. It is not an application/game compatibility or real-time guarantee.

Latency ceilings use three times the largest applicable final-source or reviewed precursor p50, p95, and maximum, rounded upward with additional clock/scheduling margin:

| Workload | p50 ceiling | p95 ceiling | maximum |
|---|---:|---:|---:|
| `dual_display_frame_arrival` | 20 ms | 100 ms | 150 ms |
| `dual_display_callback_copy` | 0.2 ms | 0.5 ms | 1.5 ms |

The profile also requires:

- zero correctness failures;
- at most 4,096 bytes of post-warm-up allocation growth;
- at most 384 MiB live Rust heap;
- at most 1 GiB native process resident high-water memory;
- exactly 66,355,200 mapped bytes per retained sample, representing two 3840x2160 BGRA8 frames;
- at most 199,065,600 callback-copy bytes per retained sample, representing six 4K producer surfaces;
- at most ten detached textures, one staging texture, and fifteen total producer/detached/staging textures;
- at most 0.75 stale-work ratio while both latest-wins sessions continue finite progress.

The copy and texture ceilings are shape/resource bounds rather than three-times timing derivatives. The three runs copied four or five producer surfaces per retained sample, observed seven or eight detached textures, one staging texture, and twelve or thirteen total textures. The accepted ceilings admit bounded callback scheduling variation while rejecting accumulation beyond the declared dual-session shape.

## Alternatives

- **Reuse ADR 0031's 1280x720 ceilings.** Rejected. Two full 4K mappings, two producer pools, shared retained-byte pressure, and mixed-DPI signed placement have different costs and resource shapes.
- **Run arrival and callback-copy as separate 600-sample workloads.** Rejected. One native interaction already produces both observations; duplication doubles capture, mapping, GPU work, and visible runtime without adding evidence.
- **Freeze the first run's seven-texture peak.** Rejected. The repeat run observed eight with unchanged correctness, copied-byte accounting, cleanup, and growth. The accepted count ceiling includes bounded scheduling margin rather than encoding one callback interleaving.
- **Treat the approximately 0.2 stale ratio as a failure.** Rejected. Mapping two 4K frames is slower than the fixture's 16 ms publication timer, so the latest-wins queues deliberately coalesce work. The retained ratio remains bounded, every returned frame is strictly newer and exact, and producer progress stays finite.
- **Infer physical device removal, TDR, or driver-upgrade results.** Rejected. Those actions were not performed; controlled device-terminal tests remain state-machine evidence only.

## Consequences

Changes to Windows dual-display publication, callback detachment, mixed-DPI placement, signed-origin conversion, mapping, shared retained pressure, or native resource lifetime must run this profile and satisfy its gates. The existing ADR 0026 controlled profiles and ADR 0031 1280x720 profiles retain their source identities, measurements, and ceilings unchanged.

No Rust, C ABI, or C++ public name or layout changes. The remaining Phase 2 `G-013` work is final-source Phase 1 regression evidence, not Windows production capture.

## Verification

The exact commands, topology, source/executable digests, aggregate measurements, final-source applicability, process lifecycle, exclusions, and privacy review are retained in [`windows-dual-4k-production-capture.md`](../evidence/phase-2-native/windows-dual-4k-production-capture.md).

The committed profile is [`phase-2-production-capture-dual-4k-x86_64-pc-windows-msvc.toml`](../benchmarks/phase-2-production-capture-dual-4k-x86_64-pc-windows-msvc.toml). Profile-key drift, hard-budget drift, latency/resource parity tests, workspace verification, and both hosted release-target jobs cover the code and profile structure.
