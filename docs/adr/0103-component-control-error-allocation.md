# ADR 0103: Allocate component control error from the declared 15% tolerance

- **Status:** Accepted — prospective policy design only; method, calibration, control, profile and numerical results remain unaccepted
- **Date:** 2026-10-25
- **Resolves gate:** _none_; method, calibration, control, profile and release gates remain required
- **Supersedes:** _none_; changes the control and benefit allowances for new
  protocol-4 campaigns only. Protocol-1/2/3 and retained artifacts are unchanged.

## Context

[ADR 0100](0100-retained-loop-cpu-and-prospective-sampling.md) carries forward
[ADR 0096](0096-component-control-and-comparison-gates.md)'s empirical allowance
for process ceilings, A/A bias, order/drift and benefit. A six-process range is a
sample, not a confidence interval; using it as the entire control tolerance makes
control strictness depend on the realized calibration spread.

The first protocol-4 unchanged control, `component-owner-p4-v1`, failed on
`A/A retained_loop_cpu_ns drift exceeds frozen control tolerance` under
`template-efficiency-v4-retained-loop-prospective`. Its failure, raw observations,
collector identity and policy remain final; this decision does not regrade them.

## Decision

The collector binds `template-efficiency-v4-retained-loop-control-budget-prospective-r2`
for every new protocol-4 campaign; numerical execution still requires its own
admission. For each graded normal metric independently, wall `p95_ns` and
`retained_loop_cpu_ns`, freeze the following from six calibration processes only:

- `m = median(calibration6)` and `s = max(calibration6) - min(calibration6)`.
- `r` is the maximum observed calibration clock resolution. CPU retains the
  accounting floor of 1,000 ns on Apple and 15,625,000 ns on Windows.
- The empirical allowance is `e = max(s, 2r)`.
- The per-process ceiling remains `max(calibration6) + e`. Neither the control
  nor benefit allowance may enlarge timing or resource ceilings.
- The control allowance is `c = max(e, ceil(m * 15/100))`.
- The benefit allowance is `b = max(e, c)`.

A/A between-role median bias and each control/comparison role's order and drift
must be `<= c`. Order compares first/second launch positions; drift compares the
chronological first/last three processes. Pair order remains
`AB, BA, AB, BA, AB, BA`. Calibration and each later role retain the noise gate
`spread / max(median, 2r) <= 0.15`, using the frozen calibration `r` for later
roles. Passing control differences cannot override a noise or process-ceiling
failure.

Existing improvement rows require
`median(baseline6) - median(candidate6) > b` strictly and jointly on p95 and
retained-loop CPU. Guard rows, including `single-small96x64`, remain non-benefit.
Turnover-stratum p95 keeps independent allowances; no stratum loop CPU is
inferred. Instrumented timing remains descriptive, with resource and exact
work/ownership gates unchanged. Counts, sampling plans and clock derivation stay
as defined by ADR 0100; allowances are never pooled across metrics or counts.

Compute allocation and median-difference decisions exactly in integer
nanoseconds. For sorted calibration values `v0..v5`, `2m = v2 + v3`, so
`ceil(m * 15/100) = ((v2 + v3) * 3 + 39) // 40`. With
`M2(x) = 2 * median(x)`, control differences require
`abs(M2(a) - M2(b)) <= 2c`, and benefit requires
`M2(baseline6) - M2(candidate6) > 2b`. Half-nanosecond medians must not be rounded
before comparison. This exact arithmetic does not replace the existing noise
ratio. Store `empirical_allowances`, `control_allowances` and
`benefit_allowances` separately and recompute them from bound calibration;
caller-edited or rehashed envelope values cannot authorize a profile.

The existing 15% engineering tolerance becomes the declared control-error
allocation, not statistical coverage, a false-failure rate or clock accuracy.
Under the unchanged calibration noise gate,
`c = max(2r, ceil(m * 15/100))`: control tolerance no longer depends on realized
spread at fixed median and clock allowance. Wall p95 bounds cross-process
latency-distribution displacement; retained-loop CPU bounds displacement of
current-process CPU expenditure, including preparation, oracles and storage on
all threads. Neither metric substitutes for the other.

## Alternatives

- **Keep empirical spread as the entire control tolerance:** retains
  sample-dependent strictness without a coverage claim.
- **Regrade or retry the failed control under a wider tolerance:** replaces a
  retained failure rather than establishing a fresh prospective campaign.
- **Fit an allowance to control/comparison values or choose another percentage:**
  breaks the predeclared calibration-only allocation.
- **Use `c` for individual ceilings:** weakens non-regression bounds instead of
  separating control error from them.

## Consequences

Within-role order/drift differences cannot exceed that role's spread. When its
median is no greater than the calibration median, the unchanged noise gate
therefore implies order/drift acceptance under `c`; these checks add no
independent detection in that case. The release-owner decision accepts not
detecting within-role chronological/position shifts at or below `c`.
Retaining these checks does not preserve the finer sensitivity of `e`.

Because `b = c >= e`, the benefit threshold can only rise. The release-owner
decision accepts that an effect at or below `b` on either metric is unclaimable:
it remains a failed benefit gate, not an alternative discharge of either
selected improvement goal.

The policy dictionary and collector identity distinguish this decision from the
previous protocol-4 method. Old policy-bound or collector-bound artifacts must
be refused for new campaigns, not converted or replayed through this policy.
New execution requires fresh source/collector-bound runtime preflight, hash-bound
method review with the identical sampling plan, and host/capacity admission
before six fresh calibration processes. Six fresh unchanged A/A pairs and a
separately accepted numerical-profile review binding the recomputed envelopes
must precede six fresh comparison pairs. Policy admission alone authorizes no
numerical stage.

Both selected goals, both targets and build modes, every required case, Windows
normal image closure and separate actual-consumer evidence remain required under
[ADR 0087](0087-v0-5-0-release-performance-qualification.md). Protocol-1/2/3
readers and arithmetic remain unchanged. No count, numerical result, profile or
performance qualification is accepted here. Defaults, threads, backends and SDK
public contracts do not change.

## Verification

Integration review must check the
[collector](../../tools/template-matching-efficiency.py) and
[`ProtocolFourControlBudgetTests`](../../tools/tests/test_template_matching_efficiency.py)
for unchanged ceilings and noise, separate metric/stratum allocation, exact
half-median boundaries, control equality versus strict joint benefit, rejection
of perturbed stored allowances and old policy/source bindings, and unchanged
protocol-1/2/3 semantics. Independent policy review and integration accept the
reduced sensitivity disclosed above, not a numerical result. The owning Change's
local `numerical-admission-verification-20261023.md` ledger retains the review
and development proof outside the published product documentation. Fresh
execution admissions remain separate prerequisites.
