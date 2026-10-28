# User-facing performance evaluation

- Date: 2026-10-29
- Decision: retained prospectively
- Scope: component protocol4 and actual-consumer collectors; no Rust library,
  C ABI, benchmark binary, workload, hardware setting or sample-count change

## Problem and decision

A consumer group had an enclosing primary span, but its individual
recognize/release timings could independently veto calibration. Comparison
repeatability also divided each role's spread by its own median, tightening the
absolute tolerance when a candidate became faster.

Grade the replay invocation, whole group, query wait or full trial operation
appropriate to the case. Keep internal timing visible and structurally checked,
but non-selectable as a numerical gate. Compare both roles with the same
pre-candidate absolute allowance: component `control_allowances[metric]`, scalar
consumer `max(calibration_spread, 2 * frozen_resolution)` in its original unit.

Calibration/A-A relative checks, individual ceilings, order/drift, strict benefit
margins and independent resource/semantic/ownership checks remain. Trial and
resident policies are unchanged. CPU remains diagnostic. New identities are
`template-efficiency-v4-latency-work-resources-v2`,
`consumer-method-review-v6` and `consumer-phase-v4`; old profiles cannot admit a
successor numerical comparison, and historical reports are not regraded.

## External measurement rationale

The reviewed [OBS statistics implementation](https://github.com/obsproject/obs-studio/blob/master/frontend/widgets/OBSBasicStats.cpp)
compares FPS and render time with the configured frame rate/period and reports
render/encode drops separately from CPU and memory. GStreamer's
[latency tracer](https://gstreamer.freedesktop.org/documentation/coretracers/latency.html)
defaults to source-to-sink latency with optional element detail;
[QoS](https://gstreamer.freedesktop.org/documentation/additional/design/qos.html)
tracks clock-relative lateness and processed/dropped work.

This supports measuring the operation the consumer needs, with internal costs
available for diagnosis. The frozen absolute-budget rule is this project's
engineering choice, not an attributed OBS/GStreamer statistical or CI policy.
[ADR0109](../../adr/0109-purpose-driven-performance-metrics.md) records the contract.

## Verification and observed effect

`python3 -m unittest discover -s tools/tests` ran 530 cases in 55.237 seconds:
510 passed and 20 skipped. Regression coverage includes primary versus diagnostic
roles, malformed internal timing, exact absolute-budget boundaries, large integer
budgets, per-invocation ceilings, unchanged calibration/A-A rules, old-profile
refusal and immutable historical-report loading. Independent consumer and
component source reviews reported no actionable findings.

`python3 tools/template-matching-efficiency.py policy --protocol-version 4
--build-mode normal` exited zero and published the current policy ID and distinct
calibration/control/comparison rules.

On Apple Silicon, the actual MadoMata supervised consumer completed one
`group-roi` invocation for each existing frozen role using the new public
collector. A new `prepare` plan reused the persistent campaign without adopting
its old failed calibration or numerical closure. Baseline was inspected before
candidate; both CLI invocations exited zero with known cleanup, correct group
results, `execution_status=completed` and `qualification_status=not-requested`.
Both reports grade only `group_span_ns` while retaining all16 recognize/release
intervals as diagnostics. These are saved-frame functional smokes, not live UI,
new capture, optimized-release timings or a speedup experiment.

The exercised producer identities remain baseline
`beb064f0c652648f92df5bfb65aaf05384a18b8d` and candidate
`ac5fdda72c098342ed728ffa1fbeaea21000c15b`; they are not relabeled as a new source
build. The local receipts under `ephemera/user-facing-evaluation-v1/` bind:

| Receipt | SHA-256 |
| --- | --- |
| `completion.json` | `7c588da7bdda4a9d746426999300af219d0f5e449726931213fde319d7613ea6` |
| `baseline/summary.json` | `ac1f017b97bff5b2e8b43c430955b110a1bfd72a34da461e3d9cf34e29bbacf3` |
| `candidate/summary.json` | `f53084101c41bf2e3f033ce071a51598ab4a7d00053bef38089f95dc1364095f` |

The completion binds collector/helper hashes, exact invocation receipts, ordered
inspection, cleanup and unchanged historical calibration SHA
`ef4084fa544f6959834d89a4e0df2da7146097c2d3b20134cab01286e9df2ea3`.
Private raw receipts are retained locally, not supplied by a Git clone.

## Release and follow-through

A missing speedup claim does not establish a library defect or decide release
readiness. Actual MadoMata behavior, documented bounds, supported-platform/ABI
verification, required CI and known defects retain their independent authority
in the [release checklist](../../../CONTRIBUTING.md#release-checklist).

Windows native verification is not exercised here: the worker is offline during
the user's workday and available after 18:00. Its continuation requires actual
online/fresh execution bindings and applicable functional verification, not an
automatic repeat of failed calibration or comparison cohorts. No new numerical
qualification, energy claim, release tag or publication is established.
