# User-facing performance evaluation

- Date: 2026-10-29
- Decision: retained prospectively
- Scope: component/consumer measurement and source-bound SDK/application
  follow-through; each section retains its own producer and workload

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

## Equivalent released-version work

The formal v0.4.0 baseline is `4148fc3b5b5438f032c7440dad06d4ad271d86cb`;
the selected v0.5.0 source is `e4ede1ba2b91401b9aefe02ff606a0177dd13d8f`.
MadoMata's authored `f61a57f...` dependency reports package0.4.0 but is a later
Git revision, not the formal release baseline.

An identical private public-API probe exercises both exact SDK exports on
Apple Silicon: release/System allocator, Rust1.98.1 and the same OpenCV4.14,
1920×1080 RGBA, a12×10 template, threshold0.9, max8/DropOverlapping. Full and
320×180 ROI searches retain a frame; turnover includes session open, acquisition,
full search, close and caller-owner drop. Every result checks the single planted
match and geometry. Each role/case has two independent processes in reversed
order, with three warmups and twenty retained operations per process.

| Common operation | v0.4.0 process p95 range, ms | Selected v0.5.0 process p95 range, ms |
| --- | ---: | ---: |
| Retained full frame | 94.423500–95.810375 | 90.346541–90.467875 |
| Retained ROI | 3.672750–3.707792 | 3.535875–3.591459 |
| Session turnover | 95.067500–95.318833 | 90.421625–91.690584 |

All twelve invocations pass their semantic and cleanup checks. No latency or
process-peak-RSS regression is observed in these three cases. The full/turnover
process peaks are approximately157–158MiB at v0.4.0 and140–142MiB at the
selected source; ROI peaks are33–37MiB. `/usr/bin/time -l` measures the whole
process peak, including setup/warmup, not cache-only occupancy, a residual or
leak. Twenty operations within one process are not twenty independent launches.
This small descriptive experiment does not qualify all v0.5.0 features,
native capture/input/OCR or Windows.

The source differences guide separate product questions:

| Boundary | Cost question |
| --- | --- |
| Common template matching | Direct RGBA, bounded BGR reuse and selection changes can remove work; retain equal-frame/ROI and lifetime checks |
| OCR model/profile selection | Compare the same model for regression; evaluate each additional model's initialization, recognition and resource needs separately |
| Grouped OCR, eight to sixteen zones | Keep one-to-eight-zone comparisons common; nine-to-sixteen-zone coverage measures added membership/output work, not an equivalent old workload |
| Maintained queries and capture pacing | Judge deadline response, frame age, drops and owner release under the requested behavior, not a synchronous-search or CPU-score surrogate |
| Application startup and trial setup | Separate application validation/backend initialization from the SDK search; do not charge unused OCR setup to template matching |

The common probe source SHA is
`ba0d7508165706f429038e433c2eed69876d5715fb9471b1383f546bc09ce79a`.
Local `ephemera/feature-cost-attribution-v1/common-sdk-results.json`
(`77a857b1485b306c0b943ae121f0dbf826fef4fd0832a0defaa035354dffff6d`)
binds all twelve raw sample/process records and both build receipts. Source
exports and fixture identities remain adjacent. No historical comparison is
regraded, and no budget is widened.

## MadoMata saved-template initialization

At MadoMata `c32c06c`, a Template trial constructed an OCR-profile engine even
though its recognition operation used only template matching. The repair
constructs the matching-only engine for Template selection. App OCR
configuration/profile validation and model/runtime/native-library hash checks
remain; OCR selection still initializes the chosen CPU provider and refuses
fallback. Check or an OCR trial, not a successful Template trial, establishes
OCR initialization.

The repair is merged into MadoMata's development topic in
[PR90](https://github.com/pashifika/mado-mata/pull/90), with separate
implementation/regression and user-contract commits.

The before/after diagnostic uses the same authored SDK pin `f61a57f...`,
optimized builds, driver and saved-frame fixtures. Three pairs in AB/BA/AB order
for each hit/miss × full/ROI case produce twelve pairs, all with correct results and
known child cleanup. The median paired `operation_us` reduction is77.5245ms;
eleven pairs improve. The first hit-ROI pair instead rises436.294→698.652ms.
That adverse observation remains in the result; first-use improvement is not
guaranteed. This is application startup evidence, not a formal v0.4.0/v0.5.0
comparison or a qualified speedup.

Timing is bound to the exact measured executables in local
`paired-smoke-results.json`
(`0ad923b7ad8872f1dfff38541d64e8bb904c4950c385227563ac0dc7c5b0fa67`).
The final backend-guard variant additionally passes348 engine tests and four
real public-controller saved-frame trials, with separate source/executable
records in `final-artifact.json` and `final-smoke-results.json`
(`8cd60cdd6dc1725e61e5e5335f910e99d1e0181e80309cb1751811c7eee07ae2`).
The earlier timing binaries are not relabeled as the final source. Private raw
receipts are not supplied by a Git clone. Windows actual-host verification
remains separate, and the existing numerical failures are unchanged.
