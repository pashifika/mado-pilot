# ADR 0098: Collect complete workload-scoped component cohorts

- **Status:** Accepted — collection granularity only
- **Date:** 2026-10-24
- **Resolves gate:** _none_; method, numerical and release gates remain required
- **Supersedes:** _none_; refines ADR 0097 for newly bound workload scopes

## Context

[ADR 0097](0097-apple-observer-on-component-campaign.md) retains the full-corpus
Apple observer-on method and its failed normal calibration. Its 54 normal or 57
instrumented rows need not share one collection invocation to preserve the
complete six-process evidence for each workload. An inspectable unit must remain
a complete workload cohort, not fewer samples or a favorable subset of attempts.

## Decision

Allow `tools/template-matching-efficiency.py` commands `calibrate`, `control` and
`compare` to select one complete workload with `--protocol-version 3 --workload`.
Without `--workload`, retain the exact ordered full mode-specific corpus.
Non-smoke commands accept the flag only once; repeated identical or different
IDs, arbitrary subsets, wrong-mode workloads and selection under protocols 1 or
2 are refused. Existing smoke and historical protocol semantics remain unchanged.

### Scope and reviewed clocks

Bind `plan.workloads` across calibration, independent control, the frozen profile,
the external `component-profile-review` and comparison. Protocol-3 profiles and
profile reviews record `workloads` explicitly; envelopes cover exactly that scope.
Missing, changed or expanded scope, envelope coverage or reviewed limits must
refuse before native launch. Derive and grade only the declared complete cohort;
omitted workloads are not missing attempts within that cohort.

The hash-bound external method review fixes `stage_limits` for `calibration`,
`control` and `comparison` before collection. For a single-workload scope, only
`campaign_timeout_seconds` may differ from the existing full-stage limit: it must
be a positive finite integer no larger than 21,600 seconds for calibration or
43,200 seconds for control/comparison. Case-specific lower absolute stage clocks
come from that accepted source/host/cost review, not a blanket ten-minute ceiling,
a universal 900-second policy or a new CLI timeout. Resolve them from the same
hash-validated review used for method binding. Every other limit must equal
`execution_limits` for the exact selected schedule.
The partial `method_review` binding retains `path`, `sha256` and the validated
`stage_limits`; full-corpus bindings retain `path` and `sha256`.
`plan.execution_limits` must equal the bound limits for its stage.

One live absolute collection deadline covers each invocation; pauses consume it.
No later enlargement, clock reset or spare-time transfer is permitted. The
300-second child, 8-MiB combined output, five-second cleanup and 120-second
per-pass image-binding bounds remain unchanged. Method admission must separately
bound management execution, serialized storage and retention as in ADR 0097;
shorter collection clocks do not establish a whole-owner or disk quota.

### Unchanged evidence and goals

Each selected statistical workload retains 20 warmups and 200 retained units per
process; contract workloads retain one warmup and three units. Calibration uses
one manifest and six baseline processes. Control uses two manifests and six
independent unchanged A/A pairs; comparison uses two manifests and six fresh
baseline/candidate pairs. Both paired stages retain `AB, BA, AB, BA, AB, BA`.
All raw slots, failed attempts, semantic/ownership guards, source/image bindings,
observer eligibility and timing-boundary requirements remain mandatory.

[ADR 0096](0096-component-control-and-comparison-gates.md) noise, A/A bias,
order/drift, frozen clock allowances and per-process timing/resource ceilings
remain unchanged. Both normal selection and repeated-preparation goals still
require each selected improvement row's p95 and summed retained-sample process
CPU benefit to exceed its frozen allowance. Instrumented work/ownership evidence
cannot replace either normal goal. No CPU metric, sampling count, numerical
policy or accepted envelope changes.

### Reporting and qualification

Protocol-3 reports expose `workload_coverage` with `required`, `selected` and
`omitted` workload ID lists and a `complete` boolean, derived from the source's full
mode-specific corpus. `status` describes the declared scope. A partial comparison
may report `status: "passed"`, but `performance_qualified` must remain `false`.
`targets[target]` and the applicable `normal_timing_verdict` or
`heap_and_work_verdict` report `incomplete` for a partial pass. Known failures
remain `failed`; non-applicable verdicts retain their existing unavailable state.
`whole_change_acceptance` remains separate.

A single-workload profile or result cannot qualify the whole target or Change.
Final release still requires every required case, both joint p95/CPU component
goals, per-process/resource non-regression, both targets and build modes, and
separate actual-consumer evidence under
[ADR 0087](0087-v0-5-0-release-performance-qualification.md).

## Alternatives

- Reduce samples or process counts: changes the numerical method instead of
  collection granularity.
- Apply one short timeout to every case: ignores reviewed case cost and retained
  binding/cleanup reserves.
- Stitch partial attempts or automatically aggregate favorable rows: loses the
  complete cohort and exact method/profile scope required for a claim.

## Consequences

Operators admit and inspect each selected workload stage separately. No automatic
execution chain, aggregation, retry, resume or replacement sampling is added.
Neither this decision nor the collector authorizes a native campaign launch.
Current collector/source bindings require fresh applicable review; ADR 0097's
historical tables, failed calibration, source identities and full-corpus rules
remain unchanged. SDK APIs, native settings and consumer responsibilities do not
change. [Performance documentation](../performance.md#workload-scoped-component-cohorts)
and [release notes](../releases/v0.5.0.md#template-matching-measurement) carry the
same scoped-claim boundary.

## Verification

Acceptance requires behavior regressions in
`tools/tests/test_template_matching_efficiency.py` for complete selected
schedules, scope/budget refusal before dispatch, profile/envelope tampering,
partial-pass qualification refusal and unchanged full-corpus/smoke/older-protocol
behavior. Integration verification and concrete case-specific method admission
remain separate; this decision records no new measurement or passing campaign.
