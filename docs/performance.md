# Performance measurement and tuning

Use benchmarks to investigate performance questions and guide engineering
decisions while keeping correctness, resource bounds, and ownership contracts
mandatory.

## Choose a method

| Engineering question | Canonical method | Scope and boundary |
|---|---|---|
| What work, latency, or memory does template matching use? | [Component](performance/methods/component.md) | Capture-free Rust/OpenCV rows; separates normal timing from instrumented mechanism counts |
| What does a complete application invocation cost? | [Actual consumer](performance/methods/actual-consumer.md) | Fresh replay, retained-group, query, or saved-RGBA trial invocations |
| What does OCR watch scheduling or real CPU startup cost? | [OCR watch](performance/methods/ocr-watch.md) | Controlled scheduler workloads and 12-stage real ONNX startup |
| Why was an empirical target or tuning parameter changed? | [Tuning history](performance/tuning/README.md) | Concise Markdown rationale, before/after observations, and prospective disposition |

For domain-specific campaign procedures, see
[OCR model comparisons](ocr-model-catalog.md#repeatable-rust-measurements),
[capture pacing](capture-pacing.md#owned-native-verification-apparatus), and
[native release-profile apparatus](evidence/native-release-profiles/README.md#current-apparatus-safeguards).

## Focused development measurements

1. **Start with bounded smoke or non-enforcing observation:** Run component or
   consumer `smoke`, or the OCR non-enforcing runner, to test a hypothesis
   without requiring a prior ADR, full-corpus calibration, or quiet-host
   campaign.
2. **Record complete provenance:** Retain the exact command, source commit/tree,
   binary digests, target/host, workload, settings, and raw output (recording
   unavailable metrics as `null` with a reason rather than `0`).
3. **Evaluate correctness and resource bounds first:** Check oracle pass counts,
   latency, memory, mapped/copied bytes, stale work, and cleanup together.
4. **Document changes in a [tuning record](performance/tuning/template.md):**
   `not-measured`, `not-comparable`, and `inconclusive` are valid outcomes when
   supported by the recorded observations.

## Observation, enforcement and safety

| Scenario | Ordinary development observation | Explicit qualification / `--enforce-budgets` |
|---|---|---|
| Correct, safely cleaned-up run; empirical target missed | Retains the measurement and miss descriptively (`exit 0`) | Returns non-success for the exceeded budget |
| Missing optional OS accounting or noisy host | Retains available data and limits without claiming qualification | Missing required cohort/metric prevents qualification |
| Semantic/oracle failure, identity mismatch, deadline/cancellation failure, or incomplete cleanup | Fails execution; fix the defect before continuing | Fails execution; favorable timing never overrides a contract failure |
| Qualification-control failure with valid identity and cleanup | Keeps labeled diagnostic observations | Retains the failed control and candidate fence without retry or regrading |

Empirical latency and RSS targets ([`G-013`](validation-gates.md#g-013)) are
distinct from enforced runtime safety limits (tensor/queue/allocation ceilings,
deadlines, cancellation, ownership fences, privacy, and ABI compatibility),
which remain mandatory in every run. Release readiness follows the
[release checklist](../CONTRIBUTING.md#release-checklist).

### Choose a product outcome, not a CPU score

As recorded in [ADR 0109](adr/0109-purpose-driven-performance-metrics.md),
performance grading uses user-facing operation outcomes rather than process CPU
counters:

| Surface | Primary outcome | Supporting diagnostic evidence |
|---|---|---|
| Template matching | Normal operation median/p95 and prospective latency non-regression | Instrumented visited scores, mapping/conversion bytes, allocations, and owner release |
| Actual consumer | Whole replay invocation, group/query span, or trial operation latency | Startup/settlement durations, deadline attainment, and resident checkpoints |
| Capture / streaming | Capture-to-result latency, frame age, and deadline attainment | Produced, processed, dropped, coalesced, and superseded frame counts |

Process CPU time remains an optional diagnostic (and Windows component loader
observation remains benchmark-only per
[ADR 0108](adr/0108-windows-component-loader-observation.md)). In baseline vs
candidate comparisons, repeatability uses a pre-candidate absolute allowance.

### Attribute version and feature costs separately

Compare baselines against the exact released tag/commit with identical inputs,
options, model, zone count, build mode, and operation boundary. Evaluate new
capabilities (such as 16-zone grouped OCR versus 8-zone, or continuous watching
versus one-shot search) under their own workload definitions rather than as
regressions on smaller workloads.

## Profiles and changes to targets

[Benchmark profiles](benchmarks/) bind empirical targets to a specific workload,
fixture hash, release target, hardware, OS build, build profile, sample count,
correctness oracle, and queue policy. Adjusting an empirical target or sample
count requires a [tuning record](performance/tuning/README.md) and applies
prospectively without regrading historical runs. Changing an enforced runtime
safety ceiling or public guarantee requires a contract change (and an
[ADR](adr/README.md) when architecturally significant).

### Existing TOML profile vocabulary

In-process Rust benchmark harness profiles (`format_version = 1`, illustrated by
[`benchmarks/example-synthetic.toml`](benchmarks/example-synthetic.toml)) contain
`[benchmark]`, `[profile]`, and `[[measurement]]` tables with three budget kinds:

- `hard`: Mandatory correctness and bounded-allocation predicates (`oracle_failures == 0`,
  `allocated_growth_bytes <= 4096`) enforced on every run.
- `absolute`: Numeric limit, unit, and `at_most` / `at_least` direction.
- `relative`: Ratio (`baseline_id`, `max_ratio`) against a same-target,
  same-fixture baseline.

Metric domains remain distinct:

- `latency_p50_ms`, `latency_p95_ms`, `latency_max_ms`: Per-operation latency
  (distinct from loop-wide `iteration_span_ms`).
- `cpu_ns` vs `retained_loop_cpu_ns`: Operation-summed vs loop-bracketed CPU
  diagnostics.
- `peak_allocated_bytes`, `steady_allocated_bytes`, `allocated_growth_bytes`:
  Counted live Rust heap (distinct from OS resident `peak_memory`, `steady_memory`,
  `memory_growth`, and `peak_resident_bytes`).
- Mapped-view lengths, producer copies, BGR cache payloads, and retained result
  extents are tracked in their own domains and never summed into physical RSS.

## Other maintained Rust benchmarks

To run a focused development smoke on the `bounded-detector` ONNX CPU benchmark
(1 warm-up, 3 samples; requires `MADO_PILOT_ONNX_RUNTIME` and
`MADO_PILOT_G004_MODEL_ROOT`):

```sh
CARGO_TARGET_DIR=target/bounded-ocr-dev \
  python3 tools/setup-native.py -- cargo bench --locked \
  --package mado-pilot-backend-onnx --features benchmark-instrumentation \
  --bench bounded-detector --no-run --message-format=json
"$BOUNDED_BINARY" --smoke --profile=bounded --provider=cpu
```

`--precursor --profile=bounded --provider=cpu` runs 3 warm-ups and 20 samples
with explicit `MADO_PILOT_BOUNDED_SOURCE_REVISION`, `MADO_PILOT_BOUNDED_HOST_ID`,
and `MADO_PILOT_BOUNDED_PROCESS_INDEX` bindings; `--enforce-budgets` adds the
compiled target-specific ceilings (`--qualify` is rejected).

## Historical results and applicability

Historical profiles, receipts, and verdicts remain byte-identical. Pre-cutover
ledger sections are preserved at commit
[`646369fad6c922ac7b064367817337a18e572d8c`](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/performance.md):

| Historical scope | Canonical historical reference |
|---|---|
| Phase 1 replay and C-boundary profiles | [Phase 1 status (`646369f`)](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/performance.md#phase-1-status) |
| Phase 2 diagnostics and native input/capture | [Diagnostic results (`646369f`)](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/performance.md#phase-22-diagnostic-performance) and [native lineage (`646369f`)](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/performance.md#phase-2-native-performance-status) |
| Phase 3 CPU OCR & v0.3.1 bounded/grouped/provider OCR | [CPU OCR (`646369f`)](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/performance.md#phase-3-accepted-cpu-ocr-performance), [bounded detector (`646369f`)](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/performance.md#v031-bounded-detector-candidate-performance), [grouped zones (`646369f`)](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/performance.md#v031-integrated-grouped-zone-qualification), and [providers (`646369f`)](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/performance.md#v031-onnx-execution-provider-qualification) |
| Phase 4 change detection and template watch | [Change detection (`646369f`)](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/performance.md#phase-4-change-detection-correctness-and-cost-boundary) and [replay budgets (`646369f`)](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/performance.md#phase-4-deterministic-template-query-performance) |
| OCR model comparisons | [Per-rule comparison history (`646369f`)](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/performance.md#ocr-model-rule-comparison) and [OCR model catalog](ocr-model-catalog.md#final-comparison-at-cf2763a) |
| Template-matching component & actual consumer | [Component history](performance/methods/component.md#historical-outcomes) and [actual-consumer history](performance/methods/actual-consumer.md#historical-outcomes) |
| OCR watch | [OCR-watch history](performance/methods/ocr-watch.md#historical-outcomes) |
| Current release summary | [v0.5.0 release notes](releases/v0.5.0.md) |
