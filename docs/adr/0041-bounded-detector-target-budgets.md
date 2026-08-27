# ADR 0041: Bounded-detector target budgets

- **Status:** Accepted
- **Date:** 2026-08-25
- **Resolves gate:** the v0.3.1 explicit bounded OCR rows under `G-013`
- **Supersedes:** _none_; ADR 0040 profile identity/ceiling and all released Phase 3 budgets remain unchanged

## Context

ADR 0039 fixed the budget formula before precursor measurement. ADR 0040 candidate v2 then ran five fresh bounded and five fresh native schema-v3 processes on each approved target at exact source `ce658b3fcbf4ba2a2e715c8d4b98b6f6a2d68235`, with alternating profile order, three warmups, 20 retained samples per workload/process, no retry or exclusion, and complete source/executable/runtime/model/fixture/RSS identity.

All 20 process reports and all 3,200 retained samples passed text, count, order, geometry, confidence, source, profile, detector dimensions/bytes/runs, recognizer runs, mapping, session topology, cancellation, heap, growth, and cleanup gates. Schema 3 names workload RSS as cumulative process high-water checkpoints and uses only final report RSS for the process budget.

The fixed 1.25-times workload/process formula, 1.5-times close formula, rounding rules, and absolute target caps produce values within every cap. The exact precursor rows remain in the Change evidence. No cap, fixture, oracle, sample, process, or failed row was changed after observation.

## Decision

Accept ADR 0040 candidate v2 and the exact target budgets below for a fresh final executable.

### Apple Silicon

| Workload | p50 / p95 / maximum |
|---|---:|
| 3840×2160 HUD | 475 / 475 / 475 ms |
| 2000×500 wide menu | 425 / 450 / 450 ms |
| 2560×320 extreme-wide status | 300 / 300 / 300 ms |
| 960×540 HUD | 600 / 600 / 600 ms |
| 1001×563 odd HUD | 600 / 600 / 625 ms |
| 1440×720 dense tooltip | 600 / 600 / 600 ms |
| 563×720 mission region | 500 / 500 / 500 ms |
| 3840×2160 blank | 175 / 175 / 175 ms |

Apple process ceilings are cold open 125 ms, first close 2 ms, reopen-close 100 ms, final process RSS 587,202,560 bytes, attributable live Rust peak 20,971,520 bytes, and post-warm growth 4,096 bytes.

### Windows

| Workload | p50 / p95 / maximum |
|---|---:|
| 3840×2160 HUD | 600 / 675 / 700 ms |
| 2000×500 wide menu | 550 / 625 / 625 ms |
| 2560×320 extreme-wide status | 350 / 350 / 375 ms |
| 960×540 HUD | 900 / 900 / 925 ms |
| 1001×563 odd HUD | 900 / 925 / 950 ms |
| 1440×720 dense tooltip | 725 / 750 / 775 ms |
| 563×720 mission region | 600 / 625 / 650 ms |
| 3840×2160 blank | 300 / 325 / 325 ms |

Windows process ceilings are cold open 250 ms, first close 8 ms, reopen-close 200 ms, final process RSS 301,989,888 bytes, attributable live Rust peak 20,971,520 bytes, and post-warm growth 4,096 bytes.

The final executable adds explicit `--enforce-budgets` mode. It parses and validates mode/profile arguments before prerequisite handling, rejects unknown or duplicate selections, and fails closed when either reviewed runtime/model path is missing; only explicit `--smoke` or the debug all-target inventory execution may skip absent native prerequisites. It retains schema 3, the full 3+20 plan, exact identity/resource/cancellation gates, and the same eight workloads. It accepts only the bounded candidate profile, checks every workload appears exactly once in the target budget table, checks p50/p95/maximum and heap, and checks report-level startup/cleanup/RSS plus absolute detector facts. `--precursor` remains available for evidence collection; `--smoke` remains the hosted hard-contract gate.

Separate new benchmark profiles record the final source and measurements:

- `docs/benchmarks/phase-3-1-bounded-ocr-aarch64-apple-darwin.toml`;
- `docs/benchmarks/phase-3-1-bounded-ocr-x86_64-pc-windows-msvc.toml`.

They join drift registries without editing either released Phase 3 profile. Native G-004 remains a comparator under ADR 0037 and receives no candidate-v2 latency/RSS budget.

## Alternatives

- **Use target absolute caps directly.** Rejected because the predeclared formula produces tighter evidence-derived values.
- **Use the smaller value from the two targets for both.** Rejected because ONNX/OpenCV/allocator/runtime behavior is target-specific and existing policy uses separate approved-host profiles.
- **Budget cumulative workload RSS checkpoints.** Rejected because target APIs expose process-lifetime high-water; only final report RSS is a valid process-budget input.
- **Use hosted CI timing/RSS.** Rejected because hosted machines are correctness/resource smoke only, not approved release hosts.
- **Reuse released Phase 3 profile files.** Rejected because candidate v2 has different identity, fixtures, detector dimensions, work, source, and evidence.

## Consequences

The constants and profiles are revision-bound regression ceilings for the named hosts and fixtures, not arbitrary application, scheduling, throughput, multi-region, or real-time guarantees. Source mapping and original-source recognition remain measured obligations even when detector tensors shrink.

The explicit candidate-v2 profile is qualified on both release targets for the named runtime/model/fixture/host boundaries. This is not a default change or an arbitrary application, scheduling, throughput, multi-region, or real-time guarantee.

No released native identity, default constructor, C/C++ surface, dependency, model/runtime byte, provider, model storage, session count, network behavior, or historical Phase 3 evidence changes.

## Verification

- Drift tests compare every committed latency/resource budget to the constants the final benchmark enforces; hard-budget tests compare correctness and growth predicates in both directions.
- Strict final source `33cd36ba248d37a422f120537922fa4caba5c07d` fails closed for missing prerequisites, rejects unknown/duplicate modes, preserves explicit smoke/debug inventory behavior, and passed independent fix review with no findings.
- Apple executable `7e48921dfeaa7b0f3a4bb33b9e927eea9e50d75422c570adb6443fd4f32cf190` passed five fresh bounded enforcement processes.
- Windows executable `aefdfa9cd6a023049b532f650a5493191994b22b3c07b582097ca1146a58d5e4` passed strict preflights and five fresh bounded enforcement processes.
- Hosted macOS and Windows jobs pass bounded/native smoke at the final source.
- Frozen Phase 3 evidence paths remain byte-identical to `dabfc3c27d634e19a073f68aa906712479eb1af2`.