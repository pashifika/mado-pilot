# ADR 0032: Windows dual-4K production capture performance budgets

- **Status:** Accepted
- **Date:** 2026-08-22
- **Resolves gate:** The remaining Windows dual-4K production-capture portion of [`G-013`](../validation-gates.md#g-013)
- **Supersedes:** none; ADR 0031 continues to govern its separate 1280x720 capture and transition profiles

## Context

ADR 0031 accepted the Windows 1280x720 production profiles while explicitly
withholding the mixed-DPI dual-4K profile until the approved physical topology
became available. The approved host exposes exactly two online non-mirrored
3840x2160 displays: a 144 DPI / 1.5-scale primary at `[0,0,3840,2160)` and a
120 DPI / 1.25-scale secondary at `[-3840,0,0,2160)`.

The original `121d41a` profile proved stationary timing and resource shape but
pre-landing review found two evidence-integrity defects and one missing row:
callback fields could mix generations, one process-wide callback could be
credited to both sessions, and the required 300-frame moving-seam phase had not
run. That profile remains historical evidence and is not final acceptance.

Repaired source `90a8babb7258bd4d8381fc172033a793a2c2ad14`, tree
`84a167718320f33a814f26f93277d488b1b6e980`, completed two unchanged-source
precursor runs with coherent frame-stamp-bound callback records. Both retained
600 stationary samples per display plus 300 moving-seam samples with zero
correctness failures and bounded allocation growth. The larger moving
observations were 19.5257 ms p50, 21.4397 ms p95, and 24.9960 ms maximum.

Final source `c6ff39a9461c128d9a53e4896a34cb65e3c419a3`, tree
`8f2766a9b55c9964f57a096a720ec4a404ad3756`, added the derived moving-seam
latency gates and reran the complete stationary and moving profile with every
gate enforced in-process. Every supervised benchmark and fixture process
reached a terminal state.

## Decision

Accept the repaired dual-4K profile as target-specific regression evidence for
the approved Windows host, exact two-display mixed-DPI topology, signed-origin
seam, repository fixture, and named workload oracles. It is not an
application/game compatibility or real-time guarantee.

Latency ceilings use three times the largest applicable retained observation,
rounded upward with scheduling margin:

The repaired oracle deliberately drains each session's queue floor and waits for
a later callback completed after one shared sample baseline. Across six repaired
runs, the largest arrival observations were 20.0007 ms p50, 40.6720 ms p95, and
58.3643 ms maximum. The 75/150/200 ms ceilings apply the same three-times,
readable-rounding policy to that corrected operation shape; they do not excuse a
failed sample.

| Workload | p50 ceiling | p95 ceiling | maximum |
|---|---:|---:|---:|
| `dual_display_frame_arrival` | 75 ms | 150 ms | 200 ms |
| `dual_display_callback_copy` | 0.2 ms | 0.5 ms | 1.5 ms |
| `dual_display_moving_seam` | 60 ms | 75 ms | 75 ms |

Every workload also requires zero correctness failures, at most 4,096 bytes of
post-warm-up allocation growth, at most 384 MiB live Rust heap, at most 1 GiB
native process resident high-water memory, exactly 66,355,200 mapped bytes, at
most 199,065,600 callback-copy bytes, at most ten detached textures, one staging
texture, fifteen total producer/detached/staging textures, and a 0.75 stale-work
ratio.

The moving workload retains exactly 300 samples with no warm-up samples. Its
deterministic triangular schedule advances the 1280x720 fixture in 16-pixel
steps between physical X `-960` and `-320`, keeping it across the negative-X
seam. Each move is confirmed through DPI-aware `GetWindowRect`; one declared
content point remains inside each display half. Both acquired frames must match
their own post-baseline callback record by stream, epoch, and frame sequence.

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
