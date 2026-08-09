# ADR 0020: Phase 2 native performance budgets and evidence gaps

- **Status:** Superseded by ADR 0021
- **Date:** 2026-08-09
- **Resolves gate:** the measured macOS capture, lifecycle, Rust input, and
  process-load workloads of `G-013` from
  [../validation-gates.md](../validation-gates.md). `G-013` remains open for the
  Windows native workloads and for C/C++ common-flow latency and resident memory.
- **Supersedes:** _none_

## Context

Phase 2 adds native capture, lifecycle transitions, native input, and equivalent
Rust, C, and C++ common flows. Hosted CI can build and test deterministic
contracts, but it cannot establish interactive display, permission, GPU/driver,
or input timings. The accepted evidence plan therefore requires revision-bound
runs on one approved host per release target and an explicit gap when a host is
unavailable.

The benchmark harness ran at source commit
`a1faf04505c8471deb4de8c136fddcc7f76105e7`, tree
`a6a3edd6e627eadc9da76785c861136d669e8b05`, on an Apple M1 Pro with 10 CPU
cores and 32 GiB under macOS 26.5.2 build 25F84. The generated fixture was
ad-hoc signed with the approved identifier; Screen Recording and Accessibility
were both granted. Three committed profiles retain the results:

| Profile | Samples after warm-up | Workloads | Result |
|---|---:|---:|---|
| [capture](../benchmarks/phase-2-native-capture-aarch64-apple-darwin.toml) | 200 | 3 | zero rejected samples and zero allocation growth |
| [transitions](../benchmarks/phase-2-native-transitions-aarch64-apple-darwin.toml) | 20 | 4 | zero rejected samples and zero allocation growth |
| [input](../benchmarks/phase-2-native-input-aarch64-apple-darwin.toml) | 50 | 6 | the accepted rerun has zero rejected samples and zero allocation growth |

The commands were:

```text
cargo bench --locked --package mado-pilot --bench native-phase2 -- \
  --workload-set capture --fixture-executable <signed-fixture> \
  --hardware "Apple M1 Pro, 10 CPU cores, 32 GiB" \
  --os-version "macOS 26.5.2 build 25F84" \
  --toolchain "rustc 1.97.1; Apple clang 21.0.0" \
  --gpu-driver "Apple integrated GPU; system driver stack" \
  --display-topology "one built-in 3024x1964 Retina display at scale 2" \
  --permissions-signing "Screen Recording granted; Accessibility granted; generated fixture bundle ad-hoc signed with approved identifier"

# Repeat with --workload-set transitions and --workload-set input, and pass the
# release-built C and C++ example executables for the input set.
```

No approved bare-metal Windows host was available. The
[Windows gap profile](../benchmarks/phase-2-native-x86_64-pc-windows-msvc-evidence-gap.toml)
records zero warm-ups, zero samples, and no substituted numbers. Its fixture hash
is recorded so the eventual run can detect source drift; it is not a measured or
normative profile.

One macOS input run is also deliberately rejected evidence. It recorded one
failed `c_common_flow` sample out of fifty while every other workload passed.
Isolated C-flow and complete-suite reruns did not reproduce the failure, and the
accepted rerun passed, but no root cause was proved. The rejected profile is
retained at
`rasen/changes/phase-2-v0-2-0-exit/evidence/macos-performance-input-a1faf04-failed-attempt.toml`.
A clean rerun is evidence that the workload can pass; it does not explain the
failure or justify erasing it.

## Decision

### Accept only the claims the available host proved

The three macOS profiles are normative for the workloads and measures that carry
budgets. Every measured workload is subject to the two structural hard gates:

- `result_correctness == 0`;
- `allocated_growth_bytes <= 4096`.

The native benchmark calls the same in-process enforcement as the Phase 1
benchmarks. `hard_budget_drift.rs` pins these predicate strings against every
measured profile, and `benchmark_block_drift.rs` pins every committed profile's
benchmark metadata keys against the harness.

Timing ceilings use three times the accepted p95, rounded up to a readable
number. `latest_acquisition` is below a microsecond and therefore uses the
batched `iteration_span_ms` instead of pretending its percentile is independent
of clock granularity. Byte ceilings express the actual frame geometry. Heap
ceilings are selected at a binary boundary above the measured peak while still
rejecting an added accumulation of large frame storage.

| Workload | Accepted target-specific ceilings |
|---|---|
| `stimulus_to_frame` | p95 70 ms; 4,628,480 mapped bytes; stale-work ratio 0.02 |
| `latest_acquisition` | batched span 0.0006 ms |
| `cpu_map_bgra8` | p95 1 ms; 4,628,480 mapped bytes |
| `resize_recreation` | p95 170 ms |
| `open_first_frame` | p95 350 ms; 7,504,640 mapped bytes |
| `retained_pressure_resume` | p95 50 ms; stale-work ratio 0.95 |
| `close_drain` | p95 100 ms |
| `input_request_receipt` | p95 50 ms |
| `rust_common_flow` | p95 450 ms; 4,628,480 mapped bytes |
| `c_process_load`, `cpp_process_load` | p95 750 ms; 192 MiB child peak resident set |
| `c_common_flow`, `cpp_common_flow` | 4,628,480 mapped bytes; latency and resident-memory ceilings withheld |

The capture profile caps tracked live heap at 32 MiB. The transition and input
profiles each cap it at 16 MiB. `peak_allocated_bytes` for a child-process
workload sees only the Rust harness; `peak_resident_bytes` is separately read
from the operating system after that owned child exits and is therefore the
measure used for C/C++ process memory.

The stale-work budgets are not throughput targets. `stimulus_to_frame` permits a
small latest-wins scheduling gap, while `retained_pressure_resume` deliberately
fills finite storage and expects most attempted publications to be rejected
before one released slot makes progress. A higher capture rate is not an
improvement if either ratio crosses its ceiling, mapped bytes grow, storage
accumulates, or an oracle fails.

### Withhold two classes of claim

No Windows number is inferred from macOS, hosted CI, or the earlier G-002
ownership prototype. All Windows Phase 2 native performance workloads remain
open until all three workload sets run on one approved bare-metal Windows host
at the same reviewed source revision.

The C and C++ common-flow latency and resident-memory ceilings also remain open.
Although only the C row rejected a sample, both wrappers traverse the same C ABI
and must receive one synchronized decision. Their correctness and mapped-byte
budgets remain active. Acceptance requires either a proved cause with a
regression test or repeated clean revision-bound runs under a predeclared sample
plan that makes the unexplained failure decision explicit.

### Preserve the Phase 1 baseline

The Phase 1 deterministic and C-boundary macOS profiles were rerun at the same
source revision. All 55 applicable hard and absolute budget comparisons passed.
Those results are retained in the Change evidence directory; the Phase 1
profiles and ceilings do not move.

## Alternatives

- **Copy the macOS ceilings to Windows.** Rejected: native startup, capture,
  display, input, resident memory, and driver behavior are target-specific. A
  cross-target number would be invented evidence.
- **Use hosted CI timings for Windows.** Rejected: CI does not exercise the
  required interactive desktop, display topology, input acknowledgement, or
  stable GPU/driver context.
- **Accept the clean input rerun and discard the failed run.** Rejected: the hard
  correctness gate failed once, and no evidence establishes that the cause was
  outside the product.
- **Withhold every macOS ceiling until Windows is available.** Rejected: that
  would discard independently reproducible evidence. `G-013` is resolved per
  workload and target, so an explicit partial resolution is more truthful.
- **Treat child-process Rust allocation counts as C/C++ memory.** Rejected: they
  count harness allocations, not the loaded native process. Resident high-water
  marks name the actual observation and its operating-system dependence.

## Consequences

- macOS regressions for the accepted workloads can be reviewed against committed
  numbers now; they are not product latency promises and do not transfer to a
  different host or target.
- Phase 2 cannot exit while the Windows profile and the two common-flow ceilings
  remain open. The source release record must keep those gaps visible.
- The input profile is normative but explicitly partial: hard correctness and
  mapped-byte rules cover every row, while comments beside the C/C++ common
  flows identify the missing ceilings.
- A future profile refresh never moves a ceiling implicitly. Tightening,
  relaxing, or accepting the withheld ceilings requires evidence and an ADR.
- `peak_resident_bytes` joins the benchmark vocabulary without redefining Phase
  1's allocator-based measures.

## Verification

- `cargo test --locked --package mado-pilot-testkit --test hard_budget_drift
  --test benchmark_block_drift` checks profile/harness predicate and metadata
  drift.
- `cargo bench --locked --package mado-pilot --bench native-phase2 -- <profile
  arguments>` enforces correctness and growth while producing the target report.
- The three measured profiles under [../benchmarks/](../benchmarks/) carry exact
  values and budgets; the Windows file explicitly carries no measurements.
- `phase-1-deterministic-rerun-a1faf04.toml` and
  `phase-1-c-boundary-rerun-a1faf04.toml` in the Change evidence directory record
  the synchronized macOS regression reruns.
- Native evidence review must inspect both the accepted and failed-attempt input
  files. This historical-failure obligation cannot be inferred by an automated
  parser from the accepted profile alone.
