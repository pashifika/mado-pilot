# ADR 0102: Actual-consumer resident checkpoint policy

- **Status:** Accepted — prospective policy design only; context-specific admission remains required
- **Date:** 2026-10-25
- **Resolves gate:** none
- **Supersedes:** none; prospective specialization of ADR 0099's resident metrics

## Context

[ADR 0099](0099-actual-consumer-scalar-measurement.md) separates actual-consumer
scalar cohorts from component performance. A calibration-derived resident
ceiling is a comparison envelope, not an independent resource allocation.
The original Apple `trial-hit-roi` control remains
`FAILED_CONTROL_RESOURCE_ENVELOPE`: four slots ran, slot `03` failed, and slots
`04` through `11` remain not run. A successor method cannot regrade that control
or complete its remaining slots.

The selected endpoint is the actual recognition child's whole-process resident
memory after clean session, engine and bridge release. It includes observer
storage; it is not an OS high-water, allocation-domain, same-owner growth,
final-owner residual or process-tree measurement.

## Decision

For `consumer-method-review-v2`, separate a fixed resident checkpoint ceiling
from a frozen calibration-derived comparison allowance. Bind both before
calibration; never fall back to the v1 formula.

The release owner declares and remains accountable for
`hard_ceiling_bytes=268435456` (256 MiB) and
`comparison_allowance_floor_bytes=1048576` (1 MiB), solely for
`recognition_child_rss_bytes` of Apple `trial-hit-roi` on the approved 32 GiB
host, `dev-normal`, observation-on. Other trial cases need separate admission;
sampled-RSS endpoints, observation-off, other hosts and arbitrary SDK consumers
are excluded.

The fixed ceiling is the owner's chosen allocation of `1/128` of
`34359738368` host physical bytes. That fraction explains the choice, not its
safety or performance: it is neither a sample fit nor a scaling formula, RAM
quota, available-memory or concurrency guarantee. Historical resident values
must not select either amount. Independent policy review and integration admit
these amounts only as this scoped engineering allocation, not host-safety evidence.

The metric contract must name
`native-self-rss-v1 libc::proc_pidinfo(PROC_PIDTASKINFO).pti_resident_size`,
`unit=bytes`, `domain=resident`, the actual recognition-child PID and
`after_trial_owner_release` / `recognition-child-process-point`. Require settled
owner release and `owner_released <= started_ns <= finished_ns` in the child's
`control.started` epoch. The bracket contains one query, not an interval maximum.
Missing initialization, unsettled release, failed/short native results, zero
bytes or missing required endpoints are unavailable, never a zero-byte pass.

Apply these resource rules independently of unchanged timing/CPU rules:

1. Bind matching positive integer amounts, with floor no larger than ceiling,
   in the metric contract and at least one hash-bound accepted `resources`
   evidence record. Require the exact `context_sha256`, source-file bindings,
   `complete=true`, `gate="resources"`, and `resident_policies` keyed exactly by
   selected resident metrics with nonempty justifications. Missing, unbound,
   Boolean, inconsistent or fabricated policies refuse admission.
2. Every invocation, including calibration, must provide positive integral
   resident bytes no larger than the fixed ceiling. A passing median cannot
   cancel a violating row.
3. Six fresh baseline values `B` freeze `s=max(B)-min(B)` and
   `a=max(s,comparison_allowance_floor_bytes)`. Reject `s/median(B)>0.15`.
   Retain the fixed ceiling, floor, minimum, maximum, spread and exact median;
   zero spread and fractional medians are valid. Resident contracts contain no
   `quantum`, `resolution` or `resolution_method`.
4. Control and comparison require all six pairs in `AB, BA, AB, BA, AB, BA`
   order. Each role requires six values, `spread/median<=0.15`, and three values
   in each position. Its absolute first/second-position median difference and
   chronological first/last-three median difference must each be `<=a`.
5. Unchanged A/A requires absolute role-median difference `<=a`; comparison
   requires `median(candidate)<=median(baseline)+a`. Neither stage may enlarge
   the calibration allowance. Resident non-regression is not a benefit and
   cannot replace selected latency/CPU improvement.

The allowance satisfies `a<=max(0.15*median(B),1048576)`: at low medians the
floor, not the percentage, sets the tolerated difference. The per-row resident
gate is the fixed ceiling only; increases below it can pass subject to role
repeatability, order, drift, bias and non-regression checks. The floor is
engineering comparison slack, not page size, API resolution or a confidence
bound; the 15% rule establishes repeatability, not false-failure probability.

The release-owner decision accepts that per-row sensitivity is the fixed ceiling,
not the old sample-derived ceiling, and that the comparison floor can dominate
the 15% bound at low medians. A context-specific method must acknowledge these
limits without using them to waive other resource or latency/CPU obligations.

## Alternatives

- **Retain a sample-derived absolute ceiling:** conflates comparison allowance
  with an independently declared resource allocation.
- **Widen or resume the failed control:** violates its frozen method and failure
  boundary.
- **Use only the fixed ceiling:** permits resident regressions below that limit
  without the separate comparison checks.
- **Transfer a Windows high-water budget:** changes target, workload and metric
  boundary; it supplies no qualification for this Apple point.

## Consequences

Keep original raw packets, source copies, hashes, accepted methods, calibration,
profiles, failures and lane fences byte-for-byte intact. No regrade, replacement
row, retry or resumed remainder is permitted. A new directory alone cannot
bypass the old fence; campaign management must record a source-bound prospective
applicability/fence disposition while retaining the failed control as final.

A successor campaign requires these separately admitted stages:

1. Freeze a new context and campaign binding the collector, sources, roles,
   binaries, runtime images, host, configuration, fixture and bounded schedule.
   Preserve the existing consumer/producer binaries and Cargo roots. Both roles
   require separately authorized fresh actual loader-on discovery under that
   plan and a closure derived from those origins; old discovery/closure bindings
   cannot be edited or reused for the new execution binding.
2. Rebind and review source/image/process, clock/CPU, observer-cost, resource,
   retention and exclusive-host gates. The resources gate must accept the
   owner's allocation, scope and comparison sensitivity, bind the new host's
   `memory_bytes=34359738368` record by hash, and cite its justification. Admit
   the new context-bound v2 method only after these gates and collector
   integration review are complete.
3. Complete six fresh baseline calibrations, then six fresh balanced unchanged
   A/A pairs, then separately accept a numerical profile before candidate
   collection. Preserve finite stage bounds and inspection between stages;
   automatic chaining is not authorized. A new control failure fences the
   affected target/lane and retains every attempt.

Changes to owner/checkpoint/API, policy amounts, collector arithmetic,
source/binary/runtime identities, workload, target/host, observation mode or
schedule invalidate affected admission and require prospective review and fresh
closure where affected, calibration, control and profile. A different host
requires a new release-owner allocation decision, never automatic rescaling.

This changes no SDK API/ABI, defaults, thread/backend policy or allocator quota.
Unavailable resource domains remain unavailable; semantic/cleanup oracles,
expected deadline failures, per-process timing ceilings and both-target
latency/CPU, ownership and actual-consumer obligations remain unchanged. No
numerical execution, profile or performance qualification is granted.

## Verification

Integration review must bind `tools/consumer-performance.py` and
`tools/native-release-profile/consumer_runtime.py` by hash, not mutable line
numbers. `tools/tests/test_consumer_performance.py` must retain regressions for
policy/context/source matching, v1 refusal, per-row ceilings, missing values,
exact fractional medians, balanced resident checks and candidate non-regression.
Timing/CPU arithmetic and execution bounds remain unchanged. Independent policy
review accepts the scoped engineering design; those tests do not prove host
capacity, context applicability or a numerical result.

The owning release Change retains `policy-and-review.md`,
`api-nomenclature-appendix.md`, `independent-review.md`,
`sensitivity-clarification.md` and `final-review.md` in its local
`consumer-resident-method-v2` evidence group. The local Change ledger
`numerical-admission-verification-20261023.md` records the independent review and
integration disposition; it is not part of the published product documentation.
The [consumer resource rules](../performance.md#closed-execution-and-resource-reservations)
and ADR 0099 carry this contract. Fresh target-specific evidence remains a
separate release gate under
[ADR 0087](0087-v0-5-0-release-performance-qualification.md); no numerical
measurement or profile is accepted by this decision.
