# ADR 0026: Windows native and diagnostic performance budgets

- **Status:** Accepted
- **Date:** 2026-08-10
- **Resolves gate:** The Windows diagnostic and former `native-phase2`
  evidence-gap workloads of [`G-013`](../validation-gates.md#g-013); the gate
  remains open for Windows production-capture acceptance, current macOS
  capture/transitions, and final-source Phase 1 reruns
- **Supersedes:** The Windows evidence-gap portions of ADR 0020 and ADR 0024,
  and the remaining-Windows-gap statement in ADR 0025

## Context

The Phase 2.2 candidate had deterministic Windows correctness coverage but no
named-host timing profile. The gap artifacts deliberately contained zero samples
and no budgets. An interactive x86_64 Windows host is now available:

- Intel Core i7-12700KF, 12 cores / 20 threads, 32 GiB;
- Windows 11 Pro 10.0.26200, build 26200;
- NVIDIA GeForce RTX 4080, driver 32.0.15.9186 dated 2026-01-20;
- two 2560x1440 logical displays, primary at `(0,0)` and secondary at
  `(-2560,0)`; the GPU reports 3840x2160 at 120 Hz;
- unelevated interactive local process, with unsigned candidate fixture and
  MSVC C/C++ executables; Windows exposes no separate capture/input permission.

The run used Rust 1.97.1, MSVC 19.44.35228, CMake 3.29.5, and OpenCV 4.14.0. It
is bound to implementation tree
`f02c3e9bc3c08d6faca4f032e6c819376ce5e0db`, based on candidate commit
`5c7c3b5434c4e5279e9dfc23568de5757914b641`.

The first native transition and language runs did not justify recording a
product regression. They exposed four benchmark/fixture defects instead:

1. A resize-only fixture emitted one changed frame. Windows Graphics Capture
   consumes the size-transition frame to recreate its frame pool and may consume
   the first replacement-pool frame while stabilizing placement, leaving no
   later frame for the benchmark oracle. Increasing the deadline from two to ten
   seconds and sending a delayed non-resize stimulus did not create the missing
   post-recreation publication. The fixture now schedules four bounded 16 ms
   repaint ticks after a successful benchmark resize; production capture code is
   unchanged.
2. One retained-pressure sample submits 41 balanced key pairs, or 82 redacted
   fixture event summaries, with the reported retained limit of 40. The harness
   creates one fixture per workload, so five warmups plus twenty samples demanded
   2,050 summaries from a process intentionally capped at 1,024. Raising or
   resetting that privacy/resource bound would weaken the fixture contract. The
   retained-pressure workload now creates one fixture per sample, keeping every
   process below the bound; fixture startup remains outside the measured resume
   latency but is visible in `iteration_span_ms`.
3. The generated Windows C and C++ executables live under
   `target/<profile>/c-abi-check`, while `madopilot.dll` lives in the parent Cargo
   profile directory. Direct benchmark launch exited with loader status 53 before
   printing output. The benchmark now prepends that validated directory to the
   child-only `PATH`, matching the existing C ABI check runner without changing
   process-global state.
4. The C++ oracle expected macOS `InvocationOnly` evidence (`1`) on Windows.
   Direct fixture execution completed and reported Windows
   `TargetProtocolAcknowledgement` evidence (`4`). The oracle is now
   target-specific; accepting either value would hide a route regression.

These failures contradicted the measurement apparatus, not native submission or
capture semantics. Profiles were recorded only after the full retained plans
passed with the repaired apparatus.

## Decision

The four measured Windows profiles under [`../benchmarks/`](../benchmarks/) are
normative regression profiles:

- [`phase-2-input-diagnostic-overhead-x86_64-pc-windows-msvc.toml`](../benchmarks/phase-2-input-diagnostic-overhead-x86_64-pc-windows-msvc.toml);
- [`phase-2-native-capture-x86_64-pc-windows-msvc.toml`](../benchmarks/phase-2-native-capture-x86_64-pc-windows-msvc.toml);
- [`phase-2-native-transitions-x86_64-pc-windows-msvc.toml`](../benchmarks/phase-2-native-transitions-x86_64-pc-windows-msvc.toml);
- [`phase-2-native-input-x86_64-pc-windows-msvc.toml`](../benchmarks/phase-2-native-input-x86_64-pc-windows-msvc.toml).

Every workload requires zero oracle failures and allocation growth at most 4,096
bytes. Every measured workload had zero failures and zero growth. Latency
ceilings are three times measured p95, rounded up:

| Diagnostic workload | Measured p95 (ms) | Ceiling (ms) |
|---|---:|---:|
| input, diagnostics off | 0.000400 | 0.0012 |
| input, normal | 0.000600 | 0.0018 |
| input, debug | 0.000600 | 0.0018 |
| input, four-slot overflow | 0.002400 | 0.0072 |
| capture/map, diagnostics off | 0.000100 | 0.0003 |
| capture/map, normal | 0.000200 | 0.0006 |
| capture/map, debug | 0.000300 | 0.0009 |
| close/drain, diagnostics off | 0.000100 | 0.0003 |
| close/drain, normal | 0.000500 | 0.0015 |
| close/drain, debug | 0.000400 | 0.0012 |

| Native workload | Measured p95 (ms) | Ceiling (ms) |
|---|---:|---:|
| stimulus to frame | 31.875100 | 100 |
| latest acquisition | 0.002700 | 0.009 |
| CPU map BGRA8 | 2.140800 | 8 |
| resize recreation | 98.583200 | 300 |
| open first frame | 110.547200 | 340 |
| retained-pressure resume | 32.328600 | 100 |
| close/drain | 2.436000 | 8 |
| input request/receipt | 0.442400 | 1.4 |
| Rust common flow | 115.182200 | 350 |
| C process load | 15.168000 | 50 |
| C common flow | 277.957900 | 900 |
| C++ process load | 15.812500 | 50 |
| C++ common flow | 279.279500 | 900 |

The diagnostic profile keeps the 32 KiB structural fixture ceiling selected by
ADR 0024; the 15,651-byte measured peak leaves queue and per-case headroom.
Native capture and transitions cap live Rust heap at 16 MiB; native input caps
it at 8 MiB. These native limits exceed three times each profile's largest
measured peak without permitting repeated full-frame accumulation.

Mapped-byte ceilings are exact: 2,322,488 bytes for one 938x619 BGRA8 frame and
3,860,768 bytes for the transition profile's one 1208x799 resized frame. Capture
measured no stale work and permits at most a 0.02 ratio. Deliberate retained
pressure measured 0.5 and is capped at 0.75. C and C++ process-load resident
high-water marks are capped at 48 MiB; their common flows are capped at 192 MiB.

These numbers are regression ceilings for this target, source tree, fixture
hash, and host class. They are not application-facing latency or memory
promises.

## Alternatives

- **Classify the failed resize and language runs as production regressions.**
  Rejected. The resize deadline and callback probes identified an absent fixture
  publication; direct language runs identified loader lookup and an exact
  platform evidence value. Production paths completed once the apparatus stated
  the real preconditions.
- **Increase or reset the 1,024-event fixture cap.** Rejected. That cap is an
  intentional privacy and memory bound. A fresh per-sample process preserves the
  bound and isolates each retained-pressure precondition.
- **Add a retry to production capture after resize.** Rejected. It would mask a
  source-owned fixture that stopped publishing and add product work to satisfy a
  benchmark-only assumption.
- **Accept either C++ evidence value.** Rejected. `InvocationOnly` and
  `TargetProtocolAcknowledgement` are materially different route facts. The
  release target decides the exact oracle.
- **Copy macOS or hosted-CI values.** Rejected. WGC, Windows scheduling, DLL
  loading, target-protocol acknowledgement, GPU/driver behavior, and resident
  process cost are target-specific.

## Consequences

Windows diagnostic, capture, transition, input, C ABI, and C++ changes must rerun
the matching profile and compare these ceilings. Changes to the Windows fixture
or its protocol change the fixture digest and require requalification.

The resize fixture performs up to four extra controlled repaints only after its
benchmark resize command. The retained-pressure benchmark pays one child process
setup per sample; that setup is reported in iteration span but not in the timed
resume latency. Windows language processes receive one validated child-only DLL
search prefix. None of these rules changes a public Rust, C, or C++ contract or
an integrator's deployment behavior.

The two former Windows gap files no longer coexist with measured evidence. The
profile registries now compile all four Windows profiles into benchmark-key and
hard-budget drift tests.

`G-013` remains open for the Windows production-capture callback-copy,
staging/resident, and named 1280×720/dual-4K acceptance profile; revision-bound
macOS capture and transition profiles; and final-source Phase 1 reruns. This ADR
does not infer any of those values from the four measured Windows profiles.

## Verification

The full runs retained 2,980 measured samples: 2,000 diagnostic samples, 600
capture samples, 80 transition samples, and 300 input/public-language samples.
Every sample satisfied its oracle and every workload reported zero allocation
growth. The native runs also enforced hard budgets in-process before printing a
profile.

The exact build and benchmark commands, host inventory, failures, repairs, and
observed output are recorded in the Change's
[`evidence/verification-procedure.md`](../../rasen/changes/phase-2-2-input-submission-observation-contract/evidence/verification-procedure.md)
and
[`evidence/verification-report.md`](../../rasen/changes/phase-2-2-input-submission-observation-contract/evidence/verification-report.md).
Repository enforcement is provided by:

```sh
cargo fmt --all -- --check
cargo test --locked --package mado-pilot-testkit --test benchmark_block_drift
cargo test --locked --package mado-pilot-testkit --test hard_budget_drift
rasen validate phase-2-2-input-submission-observation-contract --strict --no-interactive --json
```

The native measurements themselves require an interactive Windows host and are
not replaced by hosted CI.
