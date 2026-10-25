# ADR 0105: Central-four fresh-trial repeatability

- **Status:** Accepted — prospective method only; no contextual profile or qualification
- **Date:** 2026-10-25
- **Resolves gate:** none
- **Supersedes:** ADR 0104's full-six operation-range repeatability rule for new admission only

## Context

The operator selected another measurement method: remove one minimum and one
maximum before calculation. The method was selected after observing the retained
Apple cohorts. Their original percentage-based calibration remains `NONPASS`;
the v3 comparison remains inconclusive at a baseline range of `102878 us` against
`100000 us`. These observations cannot accept a method selected afterwards.

In the historical calibration and v3 comparison baseline, the excluded maximum
is also the first scheduled invocation (`551859 us` and `579997 us`, respectively).
The retrospective calculation therefore resembles dropping the first row on
those cohorts. The successor is position-independent and governs only fresh
cohorts; no historical verdict or profile is regraded.

Every observation remains a fresh process's first operation, including child
initialization, result delivery and teardown. This is not a warmed or retained
process, startup-free latency, or an attribution of an extreme's cause.

## Decision

Use `consumer-method-review-v4` and `policy="fresh-trial-central-four-v1"`.
For `trial-*` `operation_us` only, apply a central-four range screen to each
complete six-observation calibration and independently to each role in control
and comparison:

1. Keep the original six slots and raw observations. Missing, failed, partial or
   differently sized cohorts cannot be repaired by trimming, replacement or
   selecting successes from a larger pool.
2. Sort by exact value and original schedule order. Remove exactly one occurrence
   at each end: the earliest minimum and latest maximum when values tie. All-equal
   values still exclude two distinct occurrences. The first invocation is not
   automatically excluded.
3. Require the remaining four's range to be `<= max_regression_us`. Excess is
   inconclusive repeatability, not a candidate regression. Equality passes.
4. Report full-six values, minimum, maximum, range and median separately from
   central-four values, range, mean and median. Preserve excluded values and their
   indexes into the role's unchanged schedule-ordered values/slots. Complete slot
   objects and the phase's raw-reference map bind those indexes to source records.

The full-six and central-four medians both equal `(x3+x4)/2` for sorted values
`x1..x6`; comparison continues to use the full-six median. The central-four mean
`(x2+x3+x4+x5)/4` is descriptive only. Reports without a current method review
omit the successor aggregate; a separately labeled historical projection is not
admission or acceptance evidence.

All six balanced pairs remain in `AB,BA,AB,BA,AB,BA` order. Every invocation still
obeys the fixed responsiveness ceiling, correctness, cleanup and resource rules.
Keep all six A/A and comparison pairs, full-six role medians, and the original
position/chronological median checks. No pair is removed or re-paired.

### Sensitivity retained and lost

| Check | Effect of an excluded extreme |
| --- | --- |
| Every invocation's hard ceiling | A value above the ceiling still fails. |
| Central-four range | Its magnitude is absent by design. |
| Full-six median; three-value order/drift medians | Its magnitude beyond the other values is not measured. These checks do not restore tail sensitivity. |
| A/A pair absolute difference | A difference above the allowance still blocks admission. Both partners slowing similarly can remain within it. |
| Comparison pair `candidate-baseline` | A slow candidate maximum or fast baseline minimum can still fail. A slow baseline maximum can mask its paired candidate's increase relative to the other baseline observations. |
| Calibration without pairs | An isolated excluded extreme is constrained by the hard ceiling, not by a pair or median difference. |

Central-four repeatability is intentionally weaker than full-six repeatability.
Passing the other checks does not establish full-tail variability, confidence or
prediction coverage, a steady state, a speedup, or a cause of the excluded values.
A first invocation can still fail its ceiling or paired A/A/comparison check.
No additional compensating gate is introduced.

### Budgets and applicability

Keep `population="fresh-process-first-operation"`, positive exact integer
`hard_ceiling_us` and `max_regression_us`, `max_regression_us <= hard_ceiling_us`,
a ceiling below the process timeout, positive clock metadata and
`2*resolution <= max_regression_us`. Matching accepted context/source-bound
`resources` evidence must carry the same policy, population and budgets with
an explicit user-response/build/observer justification. There are no defaults.

For this new policy, the release owner prospectively selects the same numerical
engineering limits, `hard_ceiling_us=1000000` and `max_regression_us=100000`, only
for Apple `trial-hit-roi` on the named 32 GiB host, `dev-normal`, observation-on
and reviewed loader-on. This is a new policy choice before successor collection,
not automatic inheritance of ADR 0104's admission or a fit to retained samples.
One second remains the authoring-response timescale; 100 ms remains an engineering
deterioration allowance, not an imperceptibility or UI/SLO claim. Other cases,
hosts, targets, builds and observation modes inherit neither amount.

Reject v1/v2/v3 reviews and old profiles for new collection. A successor needs
fresh exact-source/context admission, six-invocation calibration, six-pair
unchanged A/A and independent profile acceptance before candidate collection.
The `consumer-profile-v1` container remains, but its envelope and all bindings
must be recomputed. Campaign identity/cleanup/control fences remain effective.

## Alternatives

- **Full-six range:** stronger tail sensitivity, but not the selected MAX/MIN
  method. Preserve its historical decisions.
- **Drop the first row or warm a process:** changes the positional rule or
  population; neither follows from trimming extrema.
- **Trim pairs, medians, CPU or resources:** weakens independent obligations
  beyond the requested repeatability change.
- **Raise budgets or retry:** changes acceptance based on outcomes instead of
  the prospectively declared contract.

## Consequences

ADR 0102's resident policy, lifetime CPU, component p95/CPU, retained group/query
benefits, source/runtime closure, observer/ownership, resource and bounded-execution
requirements carry into v4 unchanged. No SDK API/ABI, initialization, allocator,
thread, permission, deployment or runtime-default change follows.

ADR 0099 and ADR 0104 remain byte-identical historical records, including their
method names. Current documentation and the Change design/specification name
this successor explicitly. All prior raw rows, source/binary hashes, profiles,
failures and unrun stages retain their original meaning.

## Verification

`tools/tests/test_consumer_performance.py` covers exact tie ordering, all-equal
values, fractional means above floating-point integer precision, slot identity,
completeness, equality/excess boundaries, retained ceiling and pair violations,
comparison masking, unrelated metric policies, and old-review/profile refusal.
The focused module passes101tests on Apple; Windows passes87 with14 explicit
Apple-fixture skips. Both targets pass the synthetic API and two actual offline
CLI contracts: old-review reports preserve all six rows without the successor
aggregate, and old-v3 calibration refuses with zero attempts and six unrun slots.
Every started verification process cleans up. No consumer or benchmark launches.

The Change's `evidence/numerical-admission-verification-20261023.md` section
“Fresh trial central-four successor” and locator085 bind exact sources, reviews,
raw commands/results, preservation checks and the separate historical arithmetic
projection. Actual consumer qualification still requires the admitted stages
above; these implementation proofs supply no numerical profile or release result.
