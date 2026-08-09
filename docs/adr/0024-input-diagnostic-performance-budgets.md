# ADR 0024: Diagnostic performance budgets

- **Status:** Accepted
- **Date:** 2026-08-10
- **Resolves gate:** the `aarch64-apple-darwin` diagnostic workload of
  [`G-013`](../validation-gates.md#g-013); the gate remains open for the
  `x86_64-pc-windows-msvc` timing profile and other Phase 2 workloads
- **Supersedes:** _none_

## Context

Phase 2.2 adds an engine-scoped diagnostic stream across capture, mapping,
input, and lifecycle operations. The contract requires diagnostics `Off` to
allocate no queue, enabled emission to remain non-blocking, bounded pressure to
be observable, instrumentation not to change results, and caller-owned draining
to have a measured cost. Those properties need deterministic oracles and a
measured cost; implementation inspection alone cannot establish the overhead.

`crates/automation/runtime/benches/diagnostic-overhead.rs` uses one controlled
capture/input fixture for ten workloads:

1. input submission with diagnostics `Off`;
2. input submission with `Normal`, capacity 64;
3. input submission with `Debug`, capacity 64;
4. four debug input submissions at capacity 4, forcing observable overflow;
5. frame acquisition plus mapping with diagnostics `Off`;
6. the same acquisition and mapping with `Normal`;
7. the same acquisition and mapping with `Debug`;
8. explicit session close with diagnostics `Off`;
9. close plus one drain with `Normal`;
10. close plus one drain with `Debug`.

Input spans time submission and emission. Capture spans acquisition, zero-copy
mapping, and emission, and reports the exact 3,072 mapped bytes. Close spans the
explicit close and one diagnostic drain; construction stays outside the timed
span. Every sample checks its unchanged frame, mapping, receipt, or close
result, exact retained categories and losses, and increasing record sequences.
The smoke plan uses three retained samples in ordinary all-targets verification;
the measured plan uses 200 samples after 20 warmups.

The `aarch64-apple-darwin` run used an Apple M1 Pro with 10 cores and 32 GiB on
macOS 26.5.2 (25F84). Its tracked profile is
[`phase-2-input-diagnostic-overhead-aarch64-apple-darwin.toml`](../benchmarks/phase-2-input-diagnostic-overhead-aarch64-apple-darwin.toml).
No approved Windows timing host was available, so the matching tracked artifact
is an explicit
[evidence gap](../benchmarks/phase-2-input-diagnostic-overhead-x86_64-pc-windows-msvc-evidence-gap.toml),
not a copy of macOS or hosted-CI timings.

## Decision

### Measure disabled, enabled, pressure, and caller drain paths together

The ten workloads share one fixture hash and one process. Each `Off` path is the
control, so enabled-path differences measure diagnostic observation rather than
different capture, mapping, input, or lifecycle work.

The macOS measurements are:

| Workload | p95 | Mapped bytes | Peak live heap | Steady live heap | Growth |
|---|---:|---:|---:|---:|---:|
| `input_submission_diagnostics_off` | 0.000333 ms | 0 B | 6,164 B | 6,099 B | 0 B |
| `input_submission_diagnostics_normal` | 0.000375 ms | 0 B | 16,363 B | 15,595 B | 0 B |
| `input_submission_diagnostics_debug` | 0.000458 ms | 0 B | 16,651 B | 15,595 B | 0 B |
| `input_submission_diagnostic_overflow` | 0.001708 ms for four submissions | 0 B | 8,251 B | 6,955 B | 0 B |
| `capture_mapping_diagnostics_off` | 0.000083 ms | 3,072 B | 5,219 B | 5,219 B | 0 B |
| `capture_mapping_diagnostics_normal` | 0.000084 ms | 3,072 B | 14,715 B | 14,715 B | 0 B |
| `capture_mapping_diagnostics_debug` | 0.000250 ms | 3,072 B | 15,883 B | 14,715 B | 0 B |
| `session_close_drain_diagnostics_off` | 0.000125 ms | 0 B | 1,050 B | 0 B | 0 B |
| `session_close_drain_diagnostics_normal` | 0.000250 ms | 0 B | 11,282 B | 0 B | 0 B |
| `session_close_drain_diagnostics_debug` | 0.000334 ms | 0 B | 11,426 B | 0 B | 0 B |

Normal input diagnostics add 0.000042 ms at p95 over `Off`; Debug adds
0.000125 ms. Debug capture/mapping adds 0.000167 ms while preserving the exact
frame identity and mapped byte count. The separately timed close/drain path
adds 0.000125 ms for `Normal` and 0.000209 ms for `Debug` over close with
diagnostics disabled. A capacity-64 enabled input fixture retains 9,496 B more
steady live heap than `Off`; `Off` exposes no reader and therefore allocates no
diagnostic queue. Pressure preserves all four normal terminal records, reports
all eight discarded debug records, and never changes the four complete
receipts.

### Set regression ceilings from the observed target

The ten macOS p95 ceilings are three times the measured value, rounded upward:

| Workload | p95 ceiling |
|---|---:|
| `input_submission_diagnostics_off` | 0.001 ms |
| `input_submission_diagnostics_normal` | 0.0012 ms |
| `input_submission_diagnostics_debug` | 0.0015 ms |
| `input_submission_diagnostic_overflow` | 0.0052 ms |
| `capture_mapping_diagnostics_off` | 0.0003 ms |
| `capture_mapping_diagnostics_normal` | 0.0003 ms |
| `capture_mapping_diagnostics_debug` | 0.0008 ms |
| `session_close_drain_diagnostics_off` | 0.0004 ms |
| `session_close_drain_diagnostics_normal` | 0.0008 ms |
| `session_close_drain_diagnostics_debug` | 0.0011 ms |

Three times is the same developer-host margin used by ADR 0008: it tolerates
ordinary host variation while still detecting a structural regression. These
are target-specific regression ceilings, not user-facing latency guarantees.
Changing one requires a new measurement and architecture decision.

Three file-level budgets apply to every measured workload:

- `result_correctness == 0`;
- `allocated_growth_bytes <= 4096`;
- `peak_allocated_bytes <= 32768`.

The first two are enforced by the benchmark harness in both smoke and full runs.
The 32 KiB peak ceiling is nearly twice the largest observed 16,651 B fixture
while still bounding queue and per-case storage. Queue capacity itself remains
validated at construction and capped by `MAX_DIAGNOSTIC_CAPACITY`; this budget
measures the concrete fixture rather than replacing that contract ceiling.

### Keep Windows timing explicitly unresolved

Release-target CI compiles and executes the benchmark's correctness and growth
oracles on Windows, but hosted-runner timings do not set or evaluate a Windows
latency ceiling. A named Windows host must run the full command and replace the
gap artifact before the Windows `G-013` diagnostic workload is resolved.

## Alternatives

- **Infer overhead from queue and record sizes.** Rejected. Type sizes do not
  measure synchronization, identity issuance, timestamping, or emission cost.
- **Time only enabled diagnostics.** Rejected. Without the identical `Off`
  control, the input adapter and receipt cost cannot be separated from
  instrumentation overhead.
- **Include drain time in the submission span.** Rejected. Draining is
  caller-owned work and is not on the producer's input hot path. A separate
  close/drain workload times it directly without conflating the two costs.
- **Use hosted Windows CI timings as a profile.** Rejected. CI remains the right
  place for deterministic correctness and bounded-growth gates, but runner
  timing variation is not a stable regression ceiling.

## Consequences

The common diagnostic contract now has a repeatable capture/mapping,
input-submission, overflow, and close/drain benchmark; an accepted macOS
profile; hard correctness and bounded-growth gates on both release-target CI
jobs; and an explicit Windows timing gap. Future changes to diagnostic
recording, runtime orchestration, or queue storage must run this benchmark and
compare the matching target profile. Phase 2 cannot claim complete `G-013`
resolution until the Windows gap and the other open native workload profiles
are replaced with revision-bound measurements.
