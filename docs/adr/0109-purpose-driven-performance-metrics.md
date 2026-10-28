# ADR 0109: Grade product outcomes, retain CPU as a diagnostic

- Status: Accepted
- Date: 2026-10-28
- Current contract: [Performance measurement](../performance.md#choose-a-product-outcome-not-a-cpu-score), [component method](../performance/methods/component.md), [actual-consumer method](../performance/methods/actual-consumer.md)

## Context

MadoPilot measures visual automation, not processor scores. Useful outcomes are
correct results within an operation's time budget, less physical preparation or
selection work, bounded memory and timely owner release. Process CPU includes
execution outside the primary operation and depends on accounting resolution,
scheduling and hardware. Hybrid cores and frequency scaling can affect execution;
a CPU spread alone does not identify which mechanism caused it.

The source-bound Windows `single-small96x64` comparison under the previous
joint p95/CPU policy completed all six pairs with known cleanup. Its sole failed
check was candidate retained-loop CPU range/median `2/9`, exceeding `15%`.
That is a retained result under its original policy, not proof of an operation
latency regression or a reason to keep rerunning the same cohort.

Comparable tools separate product outcomes from CPU diagnostics:
[OBS statistics](https://github.com/obsproject/obs-studio/blob/master/frontend/widgets/OBSBasicStats.cpp)
show FPS, render time, skipped/lagged frames, CPU and memory separately;
[GStreamer latency tracing](https://gstreamer.freedesktop.org/documentation/coretracers/latency.html)
measures source-to-sink and element latency, while
[QoS](https://gstreamer.freedesktop.org/documentation/additional/design/qos.html)
tracks timing and processed/dropped work.
[scrcpy](https://github.com/Genymobile/scrcpy/blob/master/doc/video.md) documents
content-dependent frame rate and the latency cost of buffering. These are
measurement precedents, not claims about those projects' complete CI policy.

## Decision and rationale

Current matching methods grade normal operation latency and independently check
physical work, resources, semantics and ownership. Actual consumers grade the
whole replay invocation, same-clock group/query span or full trial operation,
with independently selected resident checkpoints. Internal phase/call durations
remain structurally validated diagnostics, not additional timing gates.
CPU values, coverage, resolution and missing reasons remain visible diagnostics;
CPU availability, variation, ceilings and benefit do not gate these claims.
Malformed accounting remains invalid data. Partial CPU never becomes complete
accounting or zero CPU.

Use the pre-candidate frozen absolute timing allowance for comparison-role
repeatability. Dividing each role's spread by its own median would tighten the
budget merely because the candidate is faster. Calibration and unchanged A/A
retain their relative repeatability rules; individual latency/resource ceilings,
order/drift and strict benefit margins remain independent.

Keep fixed schedules, source/runtime identity and finite owned cleanup. Do not
change affinity, turbo/power settings, raise a threshold after a result, or
repeat until passing. Internal timing and CPU remain useful for investigation
without defining whether the enclosing operation is usable. No new capture
apparatus or composite performance score is introduced.

## Consequences

Use new component policy/report and consumer review/report identities. Historical
cohorts, profiles, raw artifacts and verdicts remain immutable. A new numerical
claim needs fresh applicable calibration/control/profile evidence; safe smoke
requires no numerical qualification. Old collector hashes cannot be replaced in
retained receipts. Source/build changes retain their own applicability and native
verification requirements.

Latency acceptance is no energy-efficiency, CPU-saving or full-native-memory
claim. Capture freshness/deadline/drop observations remain specific to their
actual capture surface. Regression coverage distinguishes valid noisy/unavailable
CPU from malformed data and continues rejecting real latency, resource, identity
and ownership violations. Required CI and release authority remain separate.
