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

The `90a8bab` precursor and `c6ff39a` final profiles introduced the movement
phase, but their uniform-background point oracle could accept an immediately
prior fixture placement. Those runs retain their historical source identities
and resource/accounting evidence; they are not budget provenance for the
corrected movement operation.

Corrected source `7c31752bc632a26c4ba61faa0567ac78e2218ea0`, tree
`4e99487e184b3edfcbd62e31299599d2fbe13c7d`, draws one deterministic marker in
each display half and requires both requested-position marker pixels. A
strictly newer frame containing the prior placement therefore fails. Two
unchanged-source precursor runs retained 300 corrected moving pairs apiece with
zero correctness failures. Their moving p50/p95/maximum values were
`41.0535/45.6627/71.4985 ms` and `40.9366/51.1564/71.1795 ms`.

Budget-enforced source `fdcac29` first passed the committed 125/175/225 ms
movement ceilings and every unchanged gate. Fresh review then required the
prior-placement regression to call the same marker-color predicate as production
sampling. Shared-predicate source `f50285a630b07dcf10a675a0e94d34a735aa163c`,
tree `4c2f23f851669932dee304e46d2c947721598549`, reran the complete profile and
passed every latency, correctness, mapping/copy, resource, stale-work, heap,
resident, growth, and cleanup gate. Every supervised process terminated.

## Decision

Accept the repaired dual-4K profile as target-specific regression evidence for
the approved Windows host, exact two-display mixed-DPI topology, signed-origin
seam, repository fixture, and named workload oracles. It is not an
application/game compatibility or real-time guarantee.

Latency ceilings use three times the largest applicable retained observation,
rounded upward to the next readable 25 ms boundary:

The repaired stationary oracle deliberately drains each session's queue floor
and waits for a later callback completed after one shared sample baseline.
Across six repaired runs, the largest arrival observations were 20.0007 ms p50,
40.6720 ms p95, and 58.3643 ms maximum. The 75/150/200 ms ceilings apply the
same three-times, readable-rounding policy to that operation shape.

For corrected marker convergence, the two `7c31752` precursor maxima above
produce raw three-times bounds of 123.1605, 153.4692, and 214.4955 ms. Rounding
upward establishes 125/175/225 ms without treating retries, frame age, or delay
as correctness evidence.

| Workload | p50 ceiling | p95 ceiling | maximum |
|---|---:|---:|---:|
| `dual_display_frame_arrival` | 75 ms | 150 ms | 200 ms |
| `dual_display_callback_copy` | 0.2 ms | 0.5 ms | 1.5 ms |
| `dual_display_moving_seam` | 125 ms | 175 ms | 225 ms |

Every workload also requires zero correctness failures, at most 4,096 bytes of
post-warm-up allocation growth, at most 384 MiB live Rust heap, at most 1 GiB
native process resident high-water memory, exactly 66,355,200 mapped bytes, at
most 199,065,600 callback-copy bytes, at most ten detached textures, one staging
texture, fifteen total producer/detached/staging textures, and a 0.75 stale-work
ratio.

The moving workload retains exactly 300 samples with no warm-up samples. Its
deterministic triangular schedule advances the 1280x720 fixture in 16-pixel
steps between physical X `-960` and `-320`, keeping it across the negative-X
seam. DPI-aware `GetWindowRect` confirms every move. One fixed 16x16 marker
remains in each display half; both independently captured frames must contain
their requested marker center and match their own post-baseline callback record
by stream, epoch, and frame sequence.

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
