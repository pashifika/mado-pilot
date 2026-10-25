# ADR 0104: Fresh-trial responsiveness and non-regression

- **Status:** Accepted — prospective method and scoped Apple engineering budgets; current-context admission and numerical qualification remain unaccepted
- **Date:** 2026-10-25
- **Resolves gate:** none
- **Supersedes:** none; prospective specialization of ADR 0099's fresh trials

## Context

A fresh saved-RGBA trial includes child startup, OCR resource verification and
initialization, result delivery and teardown, not only template search. Its
whole-operation clock measures first-use responsiveness; it cannot isolate a
matching or retained-preparation benefit.

The retained Apple `trial-hit-roi` calibration has six `operation_us` values,
in invocation order: `551859`, `471323`, `475221`, `470318`, `469603`, `473136`.
Its range is `82256`, median `472229.5`, and exact range/median `164512/944459`,
or `17.418649%` (about `17.42%`). It remains `NONPASS` under its original 15%
rule, with `calibration noise exceeds 0.15`, empty envelopes and no subsequent
A/A, candidate comparison or accepted profile. All six invocations completed
their functional/runtime/cleanup checks; this is not a candidate regression.
The [historical collector][historical-collector] remains bound to its source;
that revision does not identify the separately bound historical consumer build.

## Decision

Use `consumer-method-review-v3` for new actual-consumer admission. For every
`trial-*` case, keep mandatory `operation_us` and replace its positive-benefit
obligation with a fixed responsiveness ceiling and observed non-regression
budget. Reject v1/v2 reviews for new collection; do not reinterpret their receipts.

The [consumer method](../performance.md#fresh-trial-responsiveness-and-non-regression)
defines the exact checks:

- Bind `policy="fresh-trial-responsiveness-v1"` and
  `population="fresh-process-first-operation"` in the operation contract and
  matching context/source-bound accepted `resources` evidence. Require a nonempty
  justification tied to intended user response, build and observation mode.
- Declare positive exact JSON integers `hard_ceiling_us` and `max_regression_us`,
  with `max_regression_us <= hard_ceiling_us` and the ceiling strictly below the
  outer process timeout in microseconds. Reject Boolean, fractional and string
  budgets. Keep positive quantum/resolution and a resolution method; require
  `2*resolution <= max_regression_us`. No default or sample-derived numeric
  budget is admitted.
- Every observed operation must meet the fixed ceiling, including smoke with a
  supplied review and every calibration/control/comparison row. One excess is
  numerical `NONPASS`; a passing median cannot cancel it.
- Retain all six fresh calibration values and their descriptive statistics.
  Freeze the declared ceiling and allowance, not sample-derived replacements.
  A range exceeding `max_regression_us` is inconclusive repeatability, not a
  candidate regression; the operation's former percentage noise gate is absent.
- Keep six complete balanced pairs in `AB, BA, AB, BA, AB, BA` order. Each role's
  range, absolute position-median difference and chronological first/last-three
  median difference must be within the same fixed allowance. A/A additionally
  bounds every absolute paired difference and the absolute role-median difference;
  excess is inconclusive and blocks profile/candidate admission.
- Comparison bounds every paired candidate-minus-baseline difference and the
  candidate-minus-baseline median difference by that allowance. Excess is an
  observed non-regression-budget violation, not causal proof of an SDK regression.
  Retain it even when independent repeatability/order/drift checks are
  inconclusive. Equality passes; a one-microsecond excess fails. Faster values
  need no minimum gain.

Use the same prospectively fixed tolerance for repeatability, controls and
non-regression. These deterministic finite-cohort checks are not confidence or
prediction intervals, clock-accuracy proof or population-wide guarantees. They
cannot justify widening the budget after observation.

The release owner accepts `hard_ceiling_us=1000000` and
`max_regression_us=100000` as prospective engineering limits solely for Apple
`trial-hit-roi`, `fresh-process-first-operation`, on the approved 32 GiB host,
`dev-normal`, `MADO_P050_OBSERVATIONS=1` and the reviewed loader-on configuration.
Other cases, targets, hosts, builds and observation modes inherit neither amount.
These amounts require explicit context/source-bound admission, not defaulting.

[Nielsen's response-time guidance][response-times] associates about one second
with continuity of thought despite a noticeable delay, and about 0.1 seconds
with an instantaneous response. The authoring objective is a fresh saved-frame
trial within that one-second command-response timescale. The owner separately
chooses one 100 ms response timescale as the maximum observed deterioration;
the guidance does not derive a regression or repeatability allowance. Neither
amount is fitted to historical runtimes or taken from a process timeout.

[Apple's responsiveness guidance][apple-responsiveness] instead recommends
roughly `< 100 ms` for synchronous main-thread work responding to a discrete
interaction and reserves time for other event/rendering stages. It does not
justify blocking the UI for one second or allowing a 100 ms regression.
The inclusive bounds here are engineering choices: exactly 100 ms deterioration
may pass, can be noticeable and can be a large percentage at a low baseline.
The same fixed allowance governs repeatability and controls without asserting
a noise distribution or false-failure probability.

The operation includes fresh-child initialization, result delivery and shutdown,
but excludes input decoding/hash and controller construction and observes no
displayed result. These limits are not a user/product SLO, imperceptibility
guarantee or visible-UI measurement; they establish no release/observation-off
equivalence or SDK consumer restriction.

## Alternatives

- **Keep mandatory whole-trial benefit:** demands an improvement in startup-heavy
  work that is not the selected matching/reuse endpoint.
- **Replace 15% with 20%, drop the first row or retry:** fits the method to the
  retained failure and changes its population or evidence.
- **Use only an absolute ceiling:** leaves regressions below that ceiling
  unprotected. Comparative pair and median checks remain required.

## Consequences

The scoped engineering amounts accept no current execution context. Matching
source/context-bound justification and every existing source/build/runtime/
observer/ownership, clock/CPU, resource and bounded-execution gate remain
prerequisites to numerical collection. Fresh calibration, unchanged A/A and
separate profile acceptance remain required before candidate collection.
Synthetic test values are not production defaults or accepted policy.

The existing `consumer-profile-v1` format binds the v3 review and recomputed
trial envelope, including policy, population and exact budgets. Substituted old
reviews or changed fields invalidate admission before target execution. Keep
historical source copies, hashes, observations, failures and campaign fences
unchanged; a new policy or directory cannot bypass a failed control fence.

[ADR 0102](0102-actual-consumer-resident-point-policy.md)'s revision-bound v2
resident policy is carried unchanged by v3. Lifetime CPU admission, other
scalar timing gates, retained group-span and matched-query benefit obligations,
and [ADR 0103](0103-component-control-error-allocation.md)'s component policy
remain unchanged. This changes no SDK API/ABI, consumer initialization, thread
setting, allocator, supported target or runtime default. It grants no native
execution, numerical profile, whole-Change completion or release qualification.

## Verification

For consumer source `4012f4828ba22c859dc3566ad0a71c76a911ef6cd548685160ac9d9033629017`,
the focused contract suite passes 94 tests on Apple. Windows executes the same
94 tests: 80 pass and 14 explicitly skip Apple dyld fixtures. Coverage includes
exact budget/evidence matching, old-method refusal, clock/timeout fit, per-row
ceilings, six-value/pair/order/drift boundaries, profile tampering, verified-byte
reference parsing and unchanged group/query benefit. The historical scalar test
retains the original Apple NONPASS.
Independent method and code reviews cover the prospective policy and failure
preservation. Identity/terminal-bound duration survives a driver's semantic
failure, and complete-pair violations survive missing observations elsewhere.

On each target, the synthetic API smoke exercises fixed-budget boundaries and
coexisting numerical/inconclusive findings. Two actual CLI invocations per
target refuse missing budgets and an old v2 review before any target starts;
each retains all six calibration slots as not-run. All verification processes
report successful cleanup; no consumer or benchmark is launched. These checks
establish neither a user-response budget nor native performance qualification.
Contextual acceptance and fresh source-bound stages remain separate release gates.

[historical-collector]: https://github.com/pashifika/mado-pilot/blob/618c34baaf96736085a041fb99f39a110bf7fa22/tools/consumer-performance.py#L930-L982
[response-times]: https://www.nngroup.com/articles/response-times-3-important-limits/
[apple-responsiveness]: https://developer.apple.com/documentation/xcode/improving-app-responsiveness
