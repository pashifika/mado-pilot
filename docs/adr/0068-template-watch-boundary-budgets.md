# ADR 0068: Accept target-specific template-watch foreign-boundary budgets

- **Status:** Numeric profiles independently accepted; final enforcement pending
- **Date:** 2026-09-07
- **Resolves gate:** none; numeric overhead acceptance is complete, but native resource and final-candidate requirements for `G-013` remain open
- **Supersedes:** the initial unaccepted sixteen-row proposal in this ADR only; ADR 0067 and historical watcher/native budgets remain unchanged

## Context

The ABI 1.6 pull-query implementation needs its own caller-boundary measurements.
The existing sixteen paired Rust-facade/negotiated C-table workloads have two
complete precursor processes per release target, each with 20 warmups and 200
retained samples. All retained consumer oracles passed; all six workloads marked
`caller_zero_allocation_gate` observed zero caller allocation calls. These are
precursors, not final qualification and not independently accepted budgets.

The original sixteen rows do not cover the required full create/cancel/release
lifecycle. The successor benchmark therefore adds `create_cancel_release_rust`
and `create_cancel_release_c`, one complete cycle per sample. Each creates its
own public replay/OpenCV engine, discovers/opens a session, loads/prepares the
package, starts a query, observes the exact settled no-match pending state,
cancels it, checks its `Cancelled` terminal, releases the query, closes/releases
parents, verifies the retained cancellation result, and releases all remaining
result/caller owners before the timed window ends. It records caller allocations
and actual process-live Rust bytes before/after the whole cycle, without sleeping
until a desired heap value appears. This is replay startup-through-close, not
native capture startup. The initial proposal had no precursors for these two
rows. Subsequent extended apparatus precursors are retained separately in the
Change evidence; they do not imply independent numeric acceptance.

### Frozen precursor provenance

All four complete reports belong to source
`9db705be8beff44fb52aa17d42fe9885897077b3`, tree
`624f2e6da362fe82d1fe140a730583245d54bf29`. Paths below are under
`rasen/changes/pull-based-c-and-cpp-template-watch-query/evidence/`.

| Target / report | Raw report SHA-256 |
|---|---|
| Apple `apple-boundary-precursor-02.toml` | `c25a32a2750f8b54c79e0564bbd487e3031ecff1bea3e5483fce446a70fcc5cd` |
| Apple `apple-boundary-precursor-03.toml` | `2d334efe05889b6952f7c2e9fc0ac236b486ed81714148d521a9bb0f8378a146` |
| Windows `windows-prereq-002.zip:boundary-first.stdout.log` | `99170f73dcfa0a98b358ae35dd52837c6858a8044c1a6d2e3bac99f385a69934` |
| Windows `windows-prereq-002.zip:boundary-repeat.stdout.log` | `952fdbfb8d6c14fedd1f0a609712e20a314040593e6e7b75c24446e88a31cf6e` |

Apple executable SHA-256 is
`52480021f4e68eebad6fcf7e6f8125053ea0c43962220e0d9a84ba756cde0f49`;
the original command, source and before/after identity are in
`implementation-result.md`. Windows executable SHA-256 is
`1070371ca87d558efa69f7e585021101627977f6ce5d78c49a98fd9d0c751aed`.
The Windows archive SHA-256 is
`afb88f9f2abd9039203c9b3e7871fb27d31ac8531a171d2f8789c52c425ebd62`;
its `commands.json`, `source-and-headers.json`, `benchmark-before.json`,
`benchmark-after.json`, `environment-provenance.json` and `summary.json` bind
the raw stdout to the source, executable and initialized MSVC/OpenCV host.
Use raw stdout, not the amended `.toml` archive members.

Apple process 01 remains apparatus-invalid because only six report sections
were retained (`apple-boundary-precursor-01-truncated.log`, SHA-256
`4f71ee69e57eb4eb86cf39643b28101d9def816d0f24d0e7da66783a621cbadd`).
The Windows failed initialization remains unchanged in
`windows-prereq-002.zip:prior-failed-attempt-001.zip` (SHA-256
`be03b11a96bfeb1b4d039998876f99c7445a4ddf7a1adce325dc98f356bf8238`).
Neither is a discarded statistical sample or a successful qualification row.

## Decision

Accept complete eighteen-row Apple and Windows profiles before final-candidate
enforcement. Independent decisions are retained as
`evidence/apple-boundary-acceptance-001.json` and
`evidence/windows-boundary-acceptance-001.json` in this Change.
These decisions do not qualify native capture, C++ wrapper overhead, or F9
process resources. Default smoke retains its existing correctness and structural
zero-allocation gates; numeric enforcement remains explicit and fail-closed.

### Accepted derivation and provenance

- For the original sixteen rows, use all applicable source-bound precursor
  processes: the historical pair above and the new pair on each target.
- For the lifecycle pair, use the new source-bound pair only. Apple uses
  `apple-boundary-precursor-005.toml` and `006.toml`, bound by the source,
  build, run-plan, per-run and finalization manifests with suffix `005`.
  Source tree is `1497c5512d01848c886008afb135a16d8341d33d`; executable
  SHA-256 is `af2eca8db77e0428553e18d279a05fe3ab051225b73665374154de272ca415bd`.
  Windows uses `windows-native-candidate-001.zip:boundary-001.stdout.log`
  and `boundary-002.stdout.log`, source tree
  `51d12df8619b123c13e15717f21cb83c7bee8c2c`, executable SHA-256
  `1dd7eada29c09dce953a34c9dbcb7eb09fb5225c1d32f0559518b69e4c39e611`.
- Compute each process's nearest-rank p50/p95/maximum from its complete
  200-element nanosecond array: one-based ranks 100, 190 and 200. Take the
  worst applicable process statistic, multiply by two, and round upward to
  100 ns. Never pool processes or remove outliers.
- Peak and steady live Rust heap use 1.25 times the largest observation,
  rounded upward to 4096 bytes. An observed zero stays zero.
- Apple `003/004` lack a complete durable source/executable/run binding.
  Their earlier blocked decision and original reports remain unchanged;
  the later byte-identical rebuild does not retroactively qualify them.
  To avoid discarding their known tails, apply a separate supplemental
  latency/heap guardband floor uniformly across all eighteen rows, using
  the same two-times/1.25-times rounding. Each final limit is the greater
  of that floor and the source-bound derivation. This is additional
  headroom, not restoration of missing provenance.
- Caller allocation ceilings are the largest source-bound sample's
  `alloc + alloc_zeroed + realloc`, without allowance or deallocation
  subtraction. Every row requires nonpositive post-warmup Rust-heap growth.
  Both lifecycle rows additionally require **each** signed post-release
  live-byte delta to be at most zero; an average cannot hide a leaking sample.
- Retained-frame views must remain exactly 24576 bytes (`96 * 64 * 4`).
  This is readable extent, not an incremental mapping/copy budget.

The initial proposal is preserved byte-for-byte in
`evidence/native-source-candidate-001.zip`, including this ADR and both
original profiles. Windows `query_clone_release_rust` observed 3700 ns
against its unaccepted 2600 ns maximum. Apple `006` exceeded four old maxima:
`pending_poll_c` 4833 > 4000 ns, `first_closed_terminal_poll_rust`
2125 > 900 ns, `retained_terminal_poll_rust` 7792 > 2100 ns, and
`query_clone_release_c` 15000 > 4500 ns. These remain offline proposal
overruns, not discarded samples or final-enforcement results.
The revision applies the same derivation to every row, not just overruns.

### Accepted latency ceilings

Times are **milliseconds per sample**. First closed-terminal and lifecycle
samples contain one observation/cycle; the other rows contain 32 observations.
Do not divide these ceilings by 32.

| Workload | Apple p50 | Apple p95 | Apple max | Windows p50 | Windows p95 | Windows max |
|---|---:|---:|---:|---:|---:|---:|
| `pending_poll_rust` | 0.001200 | 0.001400 | 0.001500 | 0.002200 | 0.002200 | 0.009200 |
| `pending_poll_c` | 0.003500 | 0.003900 | 0.009700 | 0.003200 | 0.003400 | 0.030800 |
| `first_closed_terminal_poll_rust` | 0.000200 | 0.000500 | 0.004300 | 0.000400 | 0.000400 | 0.014600 |
| `first_closed_terminal_poll_c` | 0.000800 | 0.002200 | 0.074200 | 0.001400 | 0.003000 | 0.029600 |
| `retained_terminal_poll_rust` | 0.001700 | 0.001900 | 0.015600 | 0.002000 | 0.002200 | 0.002400 |
| `retained_terminal_poll_c` | 0.008500 | 0.009200 | 0.040200 | 0.009400 | 0.009600 | 0.033000 |
| `terminal_info_match_read_rust` | 0.001300 | 0.001500 | 0.001700 | 0.001000 | 0.001200 | 0.001200 |
| `terminal_info_match_read_c` | 0.004500 | 0.005000 | 0.005200 | 0.005000 | 0.005200 | 0.012400 |
| `caller_wait_cancelled_rust` | 0.001400 | 0.001600 | 0.006600 | 0.002200 | 0.002400 | 0.003000 |
| `caller_wait_cancelled_c` | 0.008400 | 0.009000 | 0.023300 | 0.011600 | 0.011800 | 0.024400 |
| `query_clone_release_rust` | 0.001400 | 0.001500 | 0.001800 | 0.002800 | 0.002800 | 0.007400 |
| `query_clone_release_c` | 0.004000 | 0.004300 | 0.030000 | 0.003800 | 0.003800 | 0.025600 |
| `result_clone_release_rust` | 0.002500 | 0.002600 | 0.002800 | 0.002400 | 0.002400 | 0.099800 |
| `result_clone_release_c` | 0.008900 | 0.009500 | 0.016700 | 0.010600 | 0.010800 | 0.114600 |
| `exact_frame_after_parents_rust` | 0.048500 | 0.050500 | 0.083800 | 0.047800 | 0.050800 | 0.186200 |
| `exact_frame_after_parents_c` | 0.066000 | 0.069900 | 0.136600 | 0.074400 | 0.089600 | 0.249400 |
| `create_cancel_release_rust` | 1.419400 | 1.587500 | 31.429800 | 2.515600 | 3.036200 | 3.970800 |
| `create_cancel_release_c` | 1.429300 | 1.645600 | 31.570000 | 2.361000 | 2.746000 | 4.030400 |

### Allocation, memory and scope

The original sixteen allocation ceilings remain zero except
`first_closed_terminal_poll_c` (1), `caller_wait_cancelled_c` (64), and
`exact_frame_after_parents_c` (96). Lifecycle ceilings are Rust/C 188/201
on Apple and 226/239 on Windows. These counts cover caller-thread Rust
`GlobalAlloc`, not foreign `malloc` or separately loaded library allocators.

| Workload group | Apple peak / steady bytes | Windows peak / steady bytes |
|---|---:|---:|
| Pending, cancelled caller wait, query clone: Rust | 131072 / 131072 | 131072 / 131072 |
| Pending, cancelled caller wait, query clone: C | 135168 / 135168 | 131072 / 131072 |
| First closed terminal and full lifecycle: Rust and C | 245760 / 0 | 245760 / 0 |
| Retained terminal, info/match read, result clone: Rust | 102400 / 102400 | 98304 / 98304 |
| Retained terminal, info/match read, result clone: C | 102400 / 102400 | 102400 / 102400 |
| Exact frame after parents: Rust and C | 65536 / 65536 | 65536 / 65536 |

Heap metrics cover process-wide live Rust allocations, including fixture,
report storage and concurrent workers. They are not RSS, native heap, GPU
memory or incremental result-only storage. Lifecycle latency includes replay
startup through final-owner release, not native capture readiness. Operation
contexts remain finite authority, not backend preemption or a hard wall-clock
guarantee for final-owner drop.

Machine-readable profiles retain the old observations separately from current
accepted limits and new precursor identities:

- [Apple profile](../benchmarks/phase-5-template-watch-boundary-aarch64-apple-darwin.toml)
- [Windows profile](../benchmarks/phase-5-template-watch-boundary-x86_64-pc-windows-msvc.toml)

The process counts do not establish confidence intervals or guarantee future
tail behavior. In particular, Apple's retained approximately 15.7 ms lifecycle
tails are included, not clipped. Only subsequent predeclared final enforcement
can demonstrate that the exact final candidate meets these fixed ceilings.

## Alternatives

- Reuse Rust/native watcher or Phase 1 heap limits: rejected; those measure
  different paths, cadence, baselines and host profiles.
- Accept the original sixteen rows as a full lifecycle profile: rejected;
  clone/release is not create/cancel/final-release, and untimed setup is not
  startup-through-close. The two new measured paths need new precursors.
- Derive a C-minus-Rust or C/Rust ratio gate: rejected for this proposal;
  clock granularity and separately scheduled windows make subtraction or a
  near-zero denominator unstable. Both absolute sides remain visible.
- Parse arbitrary TOML limits at runtime: rejected; existing compiled
  `LatencyBudget` conventions suffice without a dependency or predicate
  language. Known profile ids, exact document pins and review govern changes.
- Enable proposed limits with a CLI override or tune after final failures:
  rejected; either would bypass independent pre-candidate acceptance.

## Consequences

Both compiled profiles cover all eighteen rows, with explicit per-sample
lifecycle deltas and exact profile-document byte pins. Runtime, adapter,
public API/ABI, frozen headers and signed applications are unchanged by
numeric acceptance. The independent review references and current profile
statuses must agree with the compiled limits.

Changes to a measured path or oracle require new source/executable identities
and affected precursor/review work. Historical reports and superseded
proposals are not rewritten. Neither the native controller's steady-state
scope nor native F9 resource ceilings is accepted by this decision.

## Verification

`template_watch_boundary/budgets.rs` checks exact declared release target,
profile id, operator-stated hardware/OS, release/debug/features, fixture/scene
hashes, sample schedule, batch size, authority constants and profile document
bytes before numeric measurement. Unknown, duplicated, missing-value or
`--enforce-budgets=false` arguments fail instead of silently selecting smoke.
`--budget-profile` selects a compiled id, not a file, and requires enforcement.
Missing/duplicate/extra workloads, missing caller samples and incompatible
mapping/lifecycle metrics fail; empty observations never become zero.
Existing hard gates run before numeric comparisons. Reports are emitted before
numeric comparisons so an overrun preserves the actual observations.

The eighteen-row smoke and source-bound precursors were executed; exact
commands, output, hashes and exits are retained in Change evidence. The
pre-acceptance refusal is preserved in
`evidence/apple-boundary-enforcement-refusal-001.json`.

After profile integration, verify strict CLI parsing, wrong-target and
wrong-host/build refusal, profile-byte drift, complete metric coverage, and
the unconditional correctness/zero-allocation gates in an isolated target root.
Do not rebuild any pinned precursor executable.

Final qualification remains pending: pin a clean final source candidate and
one release executable per target, then run five fresh sequential full
enforcement processes per target with the exact host arguments and
`--bench --enforce-budgets --budget-profile
phase-5-template-watch-boundary-<target>`. Preserve every attempted process and
full raw output. No retry, replacement, exclusion or ceiling change may rescue
a final overrun. These final runs have **not** been executed.
