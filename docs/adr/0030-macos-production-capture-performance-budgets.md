# ADR 0030: macOS production capture performance budgets

- **Status:** Accepted
- **Date:** 2026-08-21
- **Resolves gate:** the `aarch64-apple-darwin` production-capture and production-transition workloads of [`G-013`](../validation-gates.md#g-013); the cross-platform gate state remains tracked in the registry
- **Supersedes:** none; ADR 0021 remains historical and ADR 0029 continues to govern its separate controlled-stimulus lineage

## Context

The accepted Phase 2.2 macOS profiles prove controlled fixture stimulus and owning-process delivery, but they do not measure the release candidate's natural production capture publication, retained-pressure behavior, fresh-session startup, resize recreation, or close drain. The exit candidate added distinct `production-capture` and `production-transitions` workload sets rather than relabeling historical profiles.

Initial runs exposed two independent findings. Repeated fixture launches caused an invalid-source apparatus failure and were replaced with one authenticated fixture lifetime. The transition row then retained 9,856 Rust heap bytes because the existing 64-revision `GeometryLedger` grew its `VecDeque` lazily and pushed a 65th entry before retirement, allowing capacity 128. Focused samples produced the exact capacity sequence 1,408, 4,224, 9,856, and 21,120 bytes. The fix keeps a small history for fixed-geometry targets, reserves the 64-entry bound only on the first geometry change, and retires before insertion at the bound.

A post-budget final-source rerun exposed one further apparatus lifetime: the retained-pressure fixture was started before four unrelated long workloads and could sit idle for about three minutes before use. Moving its one-time creation into that workload's untimed setup preserves one fixture across all pressure samples without overlapping unrelated fixture lifetimes or changing the measured product operation.

Final measured source `d182300cd8710891ded6cba17184c44d6d58a114`, tree `c570343d334a5c77415e6a885ef8821c731b0ad5`, retained 1,150 production samples with zero correctness failures and zero allocation growth while enforcing every accepted latency, live-heap, mapped-byte, correctness, and growth budget. The tracked profiles are [`phase-2-production-capture-aarch64-apple-darwin.toml`](../benchmarks/phase-2-production-capture-aarch64-apple-darwin.toml) and [`phase-2-production-transitions-aarch64-apple-darwin.toml`](../benchmarks/phase-2-production-transitions-aarch64-apple-darwin.toml).

A pre-landing review found that the accepted live-heap and mapped-byte limits were recorded but not executable, and that resize accepted any changed capture extent. The corrected harness binds the resource constants to both profiles and requires the independently captured frame to carry the fixture's exact next frame-authoritative target geometry. The first exact-source capture rerun was rejected when one `cpu_map_bgra8` sample exceeded the unchanged 10 ms hard maximum (`13.930042 ms`); two subsequent unchanged-source runs passed every ceiling, with maxima `0.462750 ms` and `0.456000 ms`. The rejected run remains part of the review record rather than being relabeled as acceptance evidence.

## Decision

Accept both profiles as target-specific regression evidence for the approved Apple Silicon macOS 26.5.2 host, exact two-display mixed-scale topology, authenticated repository fixture, and named workload oracles. They are not game-compatibility or real-time guarantees.

Latency ceilings use three times the largest applicable measured p50, p95, and maximum from the committed final-source result and its reviewed same-lineage precursor, rounded upward:

| Workload | p50 ceiling | p95 ceiling | maximum |
|---|---:|---:|---:|
| `publication_age` | 5 ms | 15 ms | 50 ms |
| `steady_frame_acquisition` | 75 ms | 150 ms | 250 ms |
| `latest_acquisition` | 1 ms | 1 ms | 1 ms |
| `cpu_map_bgra8` | 1 ms | 2 ms | 10 ms |
| `retained_pressure_resume` | 10 ms | 50 ms | 75 ms |
| `open_first_frame` | 350 ms | 350 ms | 400 ms |
| `resize_recreation` | 175 ms | 250 ms | 300 ms |
| `close_drain` | 250 ms | 300 ms | 300 ms |

Every workload also requires zero correctness failures and at most 4,096 bytes of allocation growth. Production capture peak live Rust heap is capped at 32 MiB; production transitions at 16 MiB. Any row that maps the 1280x904 fixture is capped at 4,628,480 bytes per result. The retained-pressure stale ratio remains evidence of deliberately filling finite storage and is not a steady-capture ceiling.

## Alternatives

- **Remove resize because many games keep fixed geometry.** Rejected. Fixed-geometry sessions retain their small history after the fix, while the public contract still covers display movement, scale changes, fullscreen/windowed changes, and source-frame coordinate safety. Removing resize would require a separate public support and typed-failure decision.
- **Preallocate 64 entries on the first frame.** Rejected. It passes the benchmark but charges roughly 11 KiB to every active stream even when a game or application never changes geometry.
- **Increase warm-ups until lazy history is full.** Rejected. It hides the avoidable 64-to-128 allocation and makes the measurement policy depend on an implementation capacity.
- **Raise the 4,096-byte growth gate.** Rejected. Conditional bounded reservation removes growth without weakening the existing cross-profile hard gate.
- **Copy the historical native or controlled-profile ceilings.** Rejected. Those profiles name different sources, stimulus, topology, and oracles; they remain revision-bound evidence.
- **Withhold production latency ceilings.** Rejected. The final exact-source run and reviewed same-lineage precursor cover every named workload, and the established three-times policy yields conservative host-regression bounds without claiming application-wide latency.

## Consequences

The macOS production capture and transition workload sets become normative regression profiles. Changes to macOS capture publication, geometry history, frame mapping, retained-pressure recovery, session startup, reconfiguration, or close must run the matching set and satisfy these budgets.

Fixed-geometry sessions do not reserve the complete geometry history. A stream pays the bounded allocation once when its first distinct geometry revision is observed, after which repeated transitions reuse capacity and the oldest of 64 revisions retires before insertion.

No Rust, C ABI, or C++ public name or layout changes. Windows behavior and budgets are not inferred; cross-platform Phase 2 and release status remain governed by the current `G-013` registry and exit review.

## Verification

The exact commands, approved metadata, aggregate results, and privacy review are retained in [`macos-production-capture.md`](../evidence/phase-2-native/macos-production-capture.md). On the accepted source:

- `production-capture`: 20 warm-ups, 200 samples for five workloads, zero correctness failures and growth; mapped rows each reported 4,628,480 bytes;
- `production-transitions`: 5 warm-ups, 50 samples for three workloads, zero correctness failures and growth;
- focused `resize-allocation`: fixture-command and production resize controls both reported zero growth after the fix; owner-retention mutations failed at 9,856 or 21,120 bytes;
- `cargo test --locked -p mado-pilot-platform-macos`: 297 passed, 14 ignored;
- benchmark profile drift, hard-budget drift, and latency-budget parity tests include both new profiles.

The release benchmark enforces latency, live-heap, and applicable mapped-byte ceilings before accepting output and applies hard correctness/allocation gates unconditionally. Profile-parity tests bind every executable ADR 0030 constant to the recorded budgets. Historical profile files and their revision-bound sections are unchanged.
