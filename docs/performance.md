# Performance measurement and tuning

Use measurements to answer an engineering question, then record the reason for
keeping, revising or reverting the change. A benchmark result is not permission
to ignore correctness or resource ownership, and a noisy result is not a reason
to stop safe development.

## Choose a method

| Question | Current method | Boundary |
| --- | --- | --- |
| What work, latency or resources does template matching use? | [Component](performance/methods/component.md) | Capture-free Rust/OpenCV rows; separate normal timing and instrumented mechanism evidence |
| What does one real application invocation cost? | [Actual consumer](performance/methods/actual-consumer.md) | Fresh replay, retained-group, query or saved-RGBA trial invocations; not a component percentile |
| What does OCR watch scheduling or real CPU startup cost? | [OCR watch](performance/methods/ocr-watch.md) | Controlled work and real initialization are separate; native capture uses its own procedure |
| Why was a parameter, empirical target or implementation changed? | [Tuning history](performance/tuning/README.md) | Short Markdown rationale, evidence, effect and limitations |

Other maintained topic guides cover [model comparisons](ocr-model-catalog.md#repeatable-rust-measurements),
[capture pacing](capture-pacing.md#owned-native-verification-apparatus), and
[native release-profile apparatus](evidence/native-release-profiles/README.md#current-apparatus-safeguards).
Use [architecture](architecture.md), [OS support](architecture.md#os-support-policy),
[required verification](../CONTRIBUTING.md) and the
[validation-gate registry](validation-gates.md) for product contracts. Performance
observations do not grant platform support or release approval.

## Focused development measurements

1. Select one question and a finite workload. Use component or consumer `smoke`,
   or the OCR non-enforcing path, before considering a qualification campaign.
2. Retain the command, source/build and executable identities, target/host,
   workload, observation mode, relevant settings, execution bounds and raw output.
   Record unavailable fields with reasons; do not substitute zero.
3. Review correctness, CPU, latency, memory, mapped/copied bytes, stale work and
   cleanup together where observable. A faster result with incorrect output or
   unbounded resources is not an improvement.
4. Record the outcome in a [tuning record](performance/tuning/template.md).
   `not-measured`, `not-comparable` and `inconclusive` are valid outcomes with an
   explanation. A small smoke does not establish a representative distribution.
5. A new hypothesis or parameter can have a separately identified bounded
   experiment after safe cleanup. Keep earlier attempts; do not select favorable
   samples into an allegedly unchanged cohort.

Ordinary measurement requires no accepted ADR, full-corpus schedule,
calibration/A-A/profile approval, prior numerical control pass, CPU-idle
threshold or quiet-host resampling. Optional CPU-idle, transient host-process CPU
and OS-background observations remain descriptive, including when unavailable.
Do not intentionally overlap owned builds, tests or independent timing work with
numerical collection. Host variability alone does not establish OS causality.
For matching components and actual consumers, process CPU is diagnostic rather
than an admission, noise, ceiling or benefit gate. Valid unavailable CPU retains
its reason; malformed accounting still fails validation. Wall latency, physical
work, resources and ownership answer the product question independently.

## Observation, enforcement and safety

| Outcome | Development observation | Explicit qualification or enforcement |
| --- | --- | --- |
| Correct, safely completed measurement; empirical target missed | Keep the value and miss; the miss alone is not a failing execution exit | The selected numerical check returns non-success |
| Missing optional accounting or noisy observations | Retain available data and limitations; no qualification claim | Missing required eligibility or cohort evidence prevents the claim |
| Malformed required input, semantic or identity failure, deadline/cancellation failure, unresolved owned children or failed cleanup | Non-success; resolve unsafe state before further execution | Non-success; favorable numbers cannot override it |
| Numerical qualification-control failure with valid identity and complete cleanup | Separately labeled diagnostics remain available | Keep the failed control and its candidate/profile fence; no retry or regrading |

The methods below retain their opt-in `calibrate`, `control`, `freeze`, `compare`
or `--enforce-budgets` procedures. Choosing them explicitly retains their exact
admission, scope, grading and first-failure rules. They are not prerequisites to
ordinary tuning. A new output directory or observation mode is never a bypass
for an identity, ownership or unfinished-cleanup fence.

Empirical latency/RSS targets differ from enforced tensor/queue/allocation caps,
operation deadlines, cancellation, ownership fences, privacy, ABI compatibility
and public guarantees. Those contracts and their required native/CI checks stay
mandatory. A severe measured regression needs an explicit engineering disposition,
not concealment through a wider threshold or an obligation to rerun until pass.

Use the [release checklist](../CONTRIBUTING.md#release-checklist), not a benchmark
qualification Boolean, for release readiness. A benefit smaller than measurement
uncertainty leaves that benefit claim unestablished; it does not erase functional
improvements or establish a product regression.

### Choose a product outcome, not a CPU score

| Surface | Primary outcome | Supporting evidence |
| --- | --- | --- |
| Matching | Normal operation median/p95 and prospective latency non-regression | Instrumented visited scores, mapping/copy/conversion work, allocations and owner release |
| Actual consumer | Same-clock group/query span or complete trial operation | Startup/settlement, authored deadline outcome and resident checkpoints |
| Capture or streaming | Capture-to-result latency, frame age and deadline attainment | Produced/processed/dropped/coalesced/superseded counts with reasons and bounded queues |

Use the existing surface's apparatus; a capture-free matching row supplies no
capture/FPS/freshness claim. Do not combine different metrics into a score.
Retain process CPU, clock representation/resolution and coverage when observed,
but do not normalize them into cycles or infer work from zero quantized deltas.
Hybrid cores, frequency scaling, scheduling and other OS activity are possible
contributors, not diagnoses from an aggregate CPU counter. Do not change
affinity, turbo/power settings or sample membership to manufacture stability.

Actual-consumer grading uses the whole replay invocation, group/query span or
trial operation, not each internal release/startup interval. In comparisons,
both roles use an absolute repeatability allowance frozen before candidate
collection; candidate speed cannot make that allowance stricter.
The rationale and external measurement precedents are in
[ADR0109](adr/0109-purpose-driven-performance-metrics.md).

## Profiles and changes to targets

[Benchmark profiles](benchmarks/) bind numbers to a workload, fixture hash,
release target, hardware, OS/build, build profile, warmups/sample count,
correctness oracle, queue policy and measurement method. No target's result
qualifies the other target. A source/build/host change needs an applicability
review or new evidence for a claim; it does not rewrite earlier evidence.

An empirical threshold or sample-count adjustment uses ordinary Git review and
a concise tuning record with old/new settings, rationale and observed effect or
missing evidence. A new target applies prospectively, never to regrade a retained
failure. An ADR is needed only when the change makes a consequential architectural
or technical choice; the [ADR guide](adr/README.md) defines that scope. A change
to an enforced runtime limit or public guarantee remains a contract change.

### Existing TOML profile vocabulary

Legacy Rust harness profiles use `format_version = 1`. This is not the version
of every measurement format: component JSON protocols, consumer plans/reviews
and the OCR successor profiles have separate identities described by their
methods. Keep original files and their format interpretation intact. A meaning
change needs a new format identity and migrated callers, not relabeled evidence.

A legacy run file contains `[benchmark]`, one `[profile]`, and one
`[[measurement]]` per workload. `benchmark` records `id`, `workload`, `phase`,
`status`, `normative` and `measurements_recorded`. A not-run file explicitly uses
`[measurements]` with `performed = false` and no budget. The
[synthetic example](benchmarks/example-synthetic.toml) records no measurement and
its illustrative numbers confer no authority.

A top-level `[[budget]]` applies to all measurements; a
`[[measurement.budget]]` applies to that workload only:

- `hard`: a structural correctness/bounded-resource predicate checked by the
  harness. Fix a violation rather than relaxing the invariant.
- `absolute`: a declared numeric limit, unit and `at_most`/`at_least` direction.
- `relative`: a ratio with `baseline_id` and `max_ratio`; the named baseline must
  have the same target and fixture hash.

The profile's metric boundary, not just its budget kind, determines whether it
is an empirical target or a mandatory product invariant. Hosted timings do not
qualify a named release-host ceiling. Required CI and in-process hard predicates
remain required independently of empirical timing.

Use units and domains explicitly:

- `latency_p50`, `latency_p95`, `latency_max` are per-operation milliseconds;
  their stored keys add `_ms`. Capture-to-result percentiles have their own
  end-to-end boundary. `iteration_span_ms` brackets the whole loop including its
  oracle and divides by sample count; it is not an operation percentile.
- `cpu_ns` is summed retained-operation CPU; protocol-4
  `retained_loop_cpu_ns` brackets preparation, operations, oracles and sample
  storage. Neither is automatically owned-process-tree or operation-local CPU.
- `peak_memory`, `steady_memory`, `memory_growth` and `peak_resident_bytes` are
  resident observations. `peak_allocated_bytes`, `steady_allocated_bytes` and
  `allocated_growth_bytes` are counted live Rust heap. Native allocations,
  sampled RSS, OS high-water, private bytes and footprint are separate domains.
- Mapped-view lengths, producer copies, mapping copies, BGR conversion/cache
  payloads and retained source/result/text extents measure different work or
  ownership. Do not sum aliases into physical memory.
- Correctness and growth predicates, stale/disposition counts, tensor/session
  counts and detached/staging/GPU-resource peaks retain the exact oracle and
  scope declared by the producing harness/profile.

A zero from an operation below the clock's representation is not proof of free
work. Retain exact work measures, loop-span observations and correctness/resource
checks rather than inventing a percentile or subtracting clock/polling overhead.

## Other maintained Rust benchmarks

The backend bench target `bounded-detector` implements the existing
`onnx-bounded-cpu` report procedure. For a current CPU bounded-profile
development observation, build separately, retain Cargo's emitted executable
and its identity, then select an explicit mode:

```sh
CARGO_TARGET_DIR=target/bounded-ocr-dev \
  python3 tools/setup-native.py -- cargo bench --locked \
  --package mado-pilot-backend-onnx --features benchmark-instrumentation \
  --bench bounded-detector --no-run --message-format=json
"$BOUNDED_BINARY" --smoke --profile=bounded --provider=cpu
```

Configure the reviewed `MADO_PILOT_ONNX_RUNTIME` and
`MADO_PILOT_G004_MODEL_ROOT` paths using the
[native prerequisites](../CONTRIBUTING.md). Explicit smoke runs one warmup and
three retained samples and checks correctness, arithmetic, privacy-safe
observations, topology, cancellation, cleanup and bounded allocation growth.
With missing runtime/model paths it can explicitly report skipped work; that
is not an executed measurement or numerical qualification.

`--precursor --profile=bounded --provider=cpu` runs three warmups and twenty
samples without a timing/RSS verdict. Non-smoke modes require exact
`MADO_PILOT_BOUNDED_SOURCE_REVISION`, `MADO_PILOT_BOUNDED_HOST_ID` and
`MADO_PILOT_BOUNDED_PROCESS_INDEX` bindings, executable/runtime/model/fixture
identities and all hard oracles; use `MADO_PILOT_BOUNDED_SOURCE_TREE` where the
selected report requires a tree. `--enforce-budgets` explicitly adds the
compiled target-specific workload/startup/close/final-RSS checks; it accepts
the bounded profile only. Both modes refuse absent prerequisites, unknown or
duplicate selections; `--qualify` is refused. Debug inventory is smoke-only.

The existing CPU qualification procedure uses five fresh processes/target and
the same full 3/20 sample policy, without retries or excluded rows. It is not
required for a smoke or an ordinary parameter adjustment. Workload result count
does not select a cost class: 4K blank is still 4K mapping/detector work. Workload
RSS checkpoints are cumulative process high-water; final report RSS is the
process-budget input. Native G-004 is a comparator, not a recipient of bounded
candidate budgets. Integrated grouped zones and non-CPU providers have different
oracles/profiles; a CPU result cannot qualify them. The retained target tables
are in [benchmarks/](benchmarks/), with historical applicability below, not an
ADR-status admission rule. Required runtime tensor/session/ownership limits stay
mandatory; prospective empirical-limit changes use normal review and a tuning
record.

## Historical results and applicability

The pre-cutover performance ledger remains available at an
[immutable source snapshot](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/performance.md).
Its commands, ADR statuses, host selections and qualification rules describe
that source era, not current authority. Original profiles, receipts, source/binary
hashes, values, failures and unrun slots remain unchanged. These links preserve
outcomes without requiring the independent planning repository:

| Historical scope | Retained record and limitation |
| --- | --- |
| Phase 1 replay and C-boundary profiles | [Phase 1 results](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/performance.md#phase-1-status); named hosts/fixtures, not current-source measurements |
| Phase 2 diagnostics and native input/capture | [Diagnostic results](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/performance.md#phase-22-diagnostic-performance) and [native lineage](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/performance.md#phase-2-native-performance-status); retain invalidated profiles and corrected process-directed/production/dual-4K cohorts separately |
| Phase 3 CPU OCR | [CPU OCR results](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/performance.md#phase-3-accepted-cpu-ocr-performance); instrumented resource corrections do not retroactively validate synthesized observations |
| Phase 4 change detection and template watch | [Change-detection oracle](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/performance.md#phase-4-change-detection-correctness-and-cost-boundary) and [replay budgets](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/performance.md#phase-4-deterministic-template-query-performance); replay is not native/application qualification |
| v0.3.1 bounded detector, grouped zones and providers | [Bounded-detector results](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/performance.md#v031-bounded-detector-candidate-performance), [grouped-zone results](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/performance.md#v031-integrated-grouped-zone-qualification) and [provider results](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/performance.md#v031-onnx-execution-provider-qualification); preserve rejected CoreML and predecessor CUDA outcomes beside scoped successors |
| Model comparisons | [Per-rule comparison history](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/performance.md#ocr-model-rule-comparison); numerical success did not erase text/negative-image failures |
| Template-matching component and actual consumer | [Component history](performance/methods/component.md#historical-outcomes) and [consumer history](performance/methods/actual-consumer.md#historical-outcomes); failed, inconclusive and deferred qualification are not passes |
| OCR watch | [OCR history](performance/methods/ocr-watch.md#historical-outcomes); precursor completeness, explicit numerical enforcement and native semantics are different claims |

Current [v0.5.0 status](releases/v0.5.0.md) separately states implemented behavior,
required verification and deferred numerical qualification. This documentation
migration records no speedup and approves no release.
