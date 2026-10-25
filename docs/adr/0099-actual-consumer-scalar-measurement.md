# ADR 0099: Actual-consumer scalar measurement

- **Status:** Accepted — prospective method only; numerical qualification blocked
- **Date:** 2026-10-24
- **Resolves gate:** none
- **Supersedes:** none

## Context

The component benchmark and an actual application invocation measure different
units. The supervised JavaScript consumer executes a workflow in an engine child
and uses a Node import-parser child. A saved-RGBA trial performs fresh capability
and recognition-child work, including real selected OCR initialization. Neither
path produces 200 interchangeable instances of one operation in a process.

The retained consumer source exposes raw host/group and trial-stage observations
only with `MADO_P050_OBSERVATIONS=1`. Observation-off retains different aggregate
fields. Parent delivery/detection times, child-local durations, and sampled RSS
also have different boundaries. Treating these as interchangeable observations
would admit unsupported precision, coverage or performance claims.

The source-bound review retains consumer origin
`5af05259ce0c18e1d464ff0cf45827c6538d21a9` separately from its declared producer
revision and measurement substitutions. Fourteen inspected files match the
retained source packet SHA256
`c0a437cb8e67245aca16574a4c2bbbe12a4b1a5aeaf3e8be5edfb3b8adec08c8`.
This is evidence for the method's boundaries, not current release applicability.

## Decision

Use fresh-invocation scalar cohorts for the actual-consumer lane, with complete
owned-invocation lifetime CPU as a separate non-regression metric. Keep all
component sampling, joint p95/CPU goals and historical outcomes unchanged.

The [consumer method](../performance.md#actual-consumer-scalar-method) defines:

- the twelve fixed cases, dev-normal/observation-on configuration and exact
  distinctions between primary benefit, non-regression and failure diagnostics;
- six baseline calibrations, six balanced unchanged A/A pairs and six balanced
  baseline/candidate pairs for each case, without pooled or replaced samples;
- clock units/resolution, independently graded endpoints, frozen allowances,
  role/order/drift gates and missing-data rules;
- closed, inspectable stages with finite time/output/storage reservations and
  a campaign-level identity/cleanup fence that outlives a phase owner;
- separate logical, Rust, native, resident, growth and final-owner observations.

[ADR 0102](0102-actual-consumer-resident-point-policy.md) specifies the
prospective `consumer-method-review-v2` split between an independently declared
resident checkpoint ceiling and a calibrated comparison allowance. Exact
owner/checkpoint and host-specific resource admission remain required.
Historical methods, failures and unavailable domains are unchanged.

A process-local percentile is not applicable to a singleton consumer unit. Report
same-case cross-process median/range, not a workflow p95. Additional distributions
or operation-local CPU attribution need their own prospective contract; they are
not prerequisites invented by this scalar method.

Observation-on must independently establish normal eligibility and observer cost.
It is not equivalent to observation-off. No instrumented consumer build is
implicitly added to the scalar schedule; separately admitted resource modes have
independent observations and controls. Missing normal counters remain unavailable.

## Alternatives

- **Reuse component 20/200 and p95:** rejected for consumer singletons. Repeated
  heterogeneous host calls or fresh children do not form one process-local
  operation distribution. A new repeated-unit workload would measure another
  contract and require its own reset, ownership and cost review.
- **Use whole-invocation CPU as operation CPU:** rejected. Startup, parser,
  initialization and teardown remain inside the former's declared scope.
- **Use observation-off aggregates for missing on fields:** rejected. Integer-us
  aggregate metrics do not reconstruct raw ns calls, group spans or child marks.
- **One automatically chaining campaign:** rejected. Closed stages retain failures
  and allow result inspection without restarting or extending their deadlines.

## Consequences

This changes no SDK API, supported target, native thread setting, model policy,
consumer checkout or packaging. Application callers still own selected targets
and consumer-created children. Campaign restrictions apply only to its claims.

The method is not a runnable consumer extension to the component collector.
Actual role/image closure, complete process counts, clock/CPU eligibility,
resource coverage, byte-accounted retention and cross-phase admission enforcement
must be established before numerical collection. Current Apple descendant CPU
coverage and Windows accounting resolution/image closure remain unresolved.
A new accepted method does not regrade or complete historical evidence.

## Verification

Independent source and requirement reviews checked observer availability,
parent/child clock boundaries, cleanup timing, RSS coverage, process-count
arithmetic and persistent failure rules. The reviewed source includes the actual
cleanup producer, not only the copied terminal field. Raw packets and findings
remain with the owning `v0-5-0-pre-release-performance-measurement-and-adjustment`
Change; no new native consumer cohort supports this decision.

Documentation checks verify tracked links and the declared schedule arithmetic.
There is no numerical profile or automated consumer enforcement claim. Release
review must reject collection while the explicit admission gates remain open,
and must review prospective source/method changes before new samples.
