# ADR 0100: Grade retained-loop component CPU under frozen sampling plans

- **Status:** Accepted — prospective protocol-4 apparatus and grading contract only
- **Date:** 2026-10-25
- **Resolves gate:** _none_; method, count, numerical-profile and release gates remain required
- **Supersedes:** _none_; for new protocol-4 scopes it replaces the summed-sample
  CPU metric and fixed 20/200 statistical counts of ADRs 0096–0098. Protocol-1/2/3
  evidence and policies are unchanged.

## Context

Protocols 1–3 grade normal CPU as `metrics.cpu_ns`: the sum of process-CPU
deltas around each retained operation. That excludes per-iteration preparation,
public-result oracles and sample storage, which also run on every retained unit.
For short rows, the sum also combines hundreds of reads at the process-CPU source's
representation unit.

The admitted [ADR 0097](0097-apple-observer-on-component-campaign.md) normal
calibration completed all 325 benchmark processes. Its first stored
CPU-repeatability failure was `single-small96x64/cpu_ns`. The independent offline
audit also found CPU-noise failures for `session-owner8-bgra-roi` and
`query-match-rgba-roi`. Separately identified longer diagnostics for those rows
are descriptive; they adopt no count and regrade nothing.

A single fixed count does not fit both small, noisy rows and expensive rows.
Statistical sample execution dominated the failed campaign's wall time.

## Decision

### Protocol identity

The current `template-matching-efficiency` executable accepts only
`--protocol-version 4`, which is its default. Corpus, fixtures, options, oracles,
timing boundaries, Session strata, build-mode split and row coverage are unchanged
from protocol 3: 54 normal rows and 57 instrumented rows. Current binaries cannot
reproduce protocols 1–3. The collector keeps those readers and their original
policies for retained artifacts; its default is protocol 4.

This record introduced policy `template-efficiency-v4-retained-loop-prospective`.
[ADR 0103](0103-component-control-error-allocation.md) specifies its prospective
control/benefit successor for new campaigns; retained policy bindings stay exact.
Manifests and observations declare:

- `sampling_protocol: "mado-component-sampling-v1"`;
- `sampling_limits` with warmups `1..=1000` and samples `1..=40000`;
- `retained_loop_cpu_protocol: "mado-component-retained-loop-cpu-v1"`.

A missing or changed declaration refuses the protocol.

### Retained-loop CPU

Each observed row records `retained_loop_cpu` with exactly these fields:
`protocol`, `source`, `source_unit_ns`, `start_ns`, `end_ns`, `cpu_ns`,
`sample_count`, `reason` and `scope`.

- **Start read:** immediately after the `samples-begin` boundary, before the first
  retained iteration.
- **End read:** after the last retained sample is stored, before `samples-end`.
- **Scope:** `current-process-retained-loop-including-preparation-oracles-and-storage`.
  The bracket covers per-iteration preparation, the measured operation,
  public-result oracles and sample storage on every process thread. It excludes
  setup, warmups, post-warmup memory observation, markers and serialization.
- **Sources:** macOS `getrusage(RUSAGE_SELF)` user plus system time
  (`source_unit_ns: 1000`); Windows `GetProcessTimes` on the current process,
  kernel plus user time (`source_unit_ns: 100`). These are source representation
  units. They are not resolution, accuracy or physical attribution bounds.
- **Validation:** `sample_count` equals the actual retained count.
  `cpu_ns = end_ns - start_ns` with a non-decreasing interval. An unavailable or
  reversed bracket has null `cpu_ns` and a nonempty `reason`. The benchmark exits
  nonzero for such an observed row, and the collector records it as unavailable,
  never zero. When both values exist, the bracket must be at least the contained
  operation sum.
- **Legacy metric:** `metrics.cpu_ns` keeps its exact summed-sample meaning and
  remains reported as a protocol-4 diagnostic. Turnover hit/miss strata keep their
  per-operation CPU as diagnostics. No per-stratum loop bracket is inferred.

### Grading by build mode

Normal protocol-4 rows grade nearest-rank process p95, `ceil(0.95 * N)`, and
`retained_loop_cpu_ns`. Turnover rows also grade each stratum's p95. The
[ADR 0096](0096-component-control-and-comparison-gates.md) formulas still apply:
calibration ceilings and allowances, the 15% noise cap, between-role A/A, order
and drift checks, and per-process ceilings. The CPU clock allowance remains the
maximum observed calibration CPU resolution, but at least the existing 1,000 ns
Apple or 15,625,000 ns Windows accounting floor. It is an engineering allowance,
not an accuracy proof.

Protocol-4 comparison additionally applies the first/second-position and
chronological first/last-three median checks to each role. A role is complete
only when its observed positions follow `AB, BA, AB, BA, AB, BA`.

Instrumented protocol-4 timing and CPU are descriptive and set no ceiling or
benefit allowance. Its RSS, Rust-heap, growth and residual ceilings and exact
work/ownership/mechanism checks remain graded. Instrumented evidence cannot set
`performance_qualified` or substitute for a normal goal.

[ADR 0103](0103-component-control-error-allocation.md) specifies the current
prospective protocol-4 policy: empirical allowance for the unchanged individual
ceiling, declared 15% control tolerance and a benefit threshold no smaller than
that tolerance. Noise and sampling remain unchanged. It requires fresh
collector-bound admission and does not regrade earlier protocol-4 artifacts.

### Frozen sampling plans

The benchmark accepts paired `--warmups W --samples N` only with one statistical
`--workload`, never with `--list`, `--smoke` or a contract row. W must be in
`1..=1000` and N in `1..=40000`. Default and `--list` manifests retain the logical
reference counts: 20/200 for statistical rows and 1/3 for contract rows. Smoke
still runs 1/3. Process and row records name the counts actually executed.

Protocol-4 `calibrate`, `control` and `compare` require both
`--sampling-plan PATH` and `--sampling-plan-sha256 SHA`. The plan must meet all of
these conditions:

- UTF-8 JSON of at most 64 KiB whose SHA-256 equals the pinned value;
- an object whose keys are exactly the selected workloads (one `--workload` or the
  full mode corpus), each mapping to `{"warmups": W, "samples": N}`;
- contract rows exactly `{"warmups": 1, "samples": 3}`.

Smoke refuses a plan. The parsed plan, with its path, hash and raw bytes, is
embedded in each campaign plan. The hash-bound method review, calibration,
control, frozen profile and comparison must carry the identical plan; a changed
count refuses before collection.

The parser bounds are not adopted counts, cost proofs or output guarantees. Each
count is a prospective method input chosen by reviewed repeatability and cost
rationale before calibration. The 300-second child, 8-MiB output and five-second
cleanup bounds still fail closed. ADR 0098's case-specific stage clocks still
come from the method review.

The following are not permitted: automatic count selection, adaptive or optional
stopping, resampling, retry, post-result count change, or normalizing CPU across
different counts. A revised count requires fresh calibration and control.

### Unchanged obligations and history

Six baseline calibrations, six unchanged A/A pairs and six fresh comparison pairs
remain required per selected workload. Both normal goals still require joint p95
and CPU benefit above frozen allowances; under protocol 4 the CPU metric is the
retained-loop bracket. Per-process timing/resource ceilings, both targets and
build modes, every required case and separate actual-consumer evidence remain
release requirements under
[ADR 0087](0087-v0-5-0-release-performance-qualification.md).

Protocol-1/2/3 artifacts are not converted or regraded. This includes ADR 0097's
failed normal calibration and its tables. ADR 0097's method acceptance binds its
protocol-3 policy and collector. A protocol-4 campaign needs its own hash-bound
method review (including the sampling plan), a fresh collector-bound preflight,
calibration, control and profile review. Windows normal image closure remains
unresolved. SDK behavior and caller responsibilities do not change.

## Alternatives

- **Raise every row to one larger fixed count:** multiplies the cost of expensive
  rows and still leaves preparation and oracle work outside the CPU metric.
- **Keep summed per-operation CPU with smaller noise rules:** weakens the accepted
  gates instead of measuring the retained work.
- **Normalize CPU per sample or pool processes:** mixes counts and hides the
  per-process distribution that the gates require.
- **Treat source units or observed increments as accuracy:** unsupported by
  either OS API.
- **Increase N until noise passes:** a hidden retry and optional-stopping rule.

## Consequences

New component campaigns name exact per-workload counts before calibration and
retain both CPU observations. Existing protocol-3 preflights and campaigns cannot
be continued with current binaries; their evidence keeps its original meaning.
Hosted semantic smoke and documented commands must use protocol 4.

No count, numerical profile, native protocol-4 result or performance
qualification is accepted here. [Performance documentation](../performance.md#protocol-4-retained-loop-cpu-and-frozen-sampling)
and [release notes](../releases/v0.5.0.md#template-matching-measurement) carry
the same boundary.

## Verification

Acceptance requires behavior regressions in
`tools/tests/test_template_matching_efficiency.py` for:

- invalid bracket arithmetic, sources, units and hidden availability;
- missing capability declarations;
- exact plan hash, coverage and integer bounds;
- refusal of statistical counts on contract rows;
- refusal of changed counts before comparison;
- separate normal retained-loop CPU and instrumented resource grading.

The benchmark must refuse unpaired, out-of-range, smoke/list and contract-row
counts. Hosted CI must exercise protocol-4 semantic smoke in both modes on both
targets. The performance guide records the first direct Apple Silicon development
smoke of selected rows; it is not a sampling-plan campaign. Integration
verification is recorded separately, and this decision records no qualifying
measurement.
