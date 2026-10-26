# Performance tuning history

Keep one short Markdown record for a coherent tuning decision or empirical-target
change, reviewed with its implementation/profile change in the normal PR.
Use [the template](template.md), named `YYYY-MM-DD-<topic>.md`; link raw output
instead of copying logs. No ADR, Accepted-status parser, separate approval state
or full benchmark campaign is required solely to tune a number.

A record states the problem, change/reason, source/platform/workload scope,
evidence and effect, trade-offs and whether the choice is retained, revised or
reverted. Include old/new parameters or thresholds when they change. Comparable
before/after values need units and method/command identity. Otherwise say
`not-measured`, `not-comparable` or `inconclusive`, and why; make no speedup claim.
One record can cover related parameters rather than every raw sample.

Targets change prospectively. Do not regrade an earlier failure, erase an adverse
measurement or select favorable attempts into an unchanged cohort. Follow-ups
append a dated section or link a successor record; keep the original rationale,
outcome and source-bound evidence attributable. Label any supported extraction
from earlier evidence as historical, not a newly run experiment.

Empirical latency/RSS targets are not runtime allocation/tensor/queue caps,
deadlines, ownership, privacy, compatibility or public guarantees. Those retain
their [product contracts](../../architecture.md) and required verification.
Record severe regressions and cross-metric trade-offs explicitly; a chosen
trade-off is not demonstrated improvement. Missing qualification supplies no
new platform support or release approval.

## Records

| Date | Topic | Observed effect |
| --- | --- | --- |
| 2026-10-26 | [Focused measurements and tuning documentation](2026-10-26-focused-measurements-and-tuning-history.md) | Performance not measured; development observation and optional qualification have explicit separate contracts |
| 2026-10-26 | [OCR workload profile authority](2026-10-26-ocr-watch-profile-authority.md) | Performance not measured; successor profile identity removes Markdown-status admission without changing numerical ceilings |
| 2026-10-26 | [Dense KeepAll selection](2026-10-26-dense-keepall-selection-observation.md) | Independent Apple/Windows instrumented observations: fewer visited offsets and no selector dense copy; 28 more counted allocations, no performance qualification |
| 2026-10-26 | [Bounded KeepAll row heap](2026-10-26-bounded-keepall-row-heap.md) | Direct selector verification reduces dense allocations and counted heap without increasing sparse/empty allocation; preparation and performance qualification remain separate |
| 2026-10-26 | [Preparation reuse](2026-10-26-preparation-reuse-observation.md) | Both targets reduce BGR conversion and live-owner mapping work; cold-ROI allocations increase, with ownership and qualification scopes retained |
| 2026-10-26 | [Focused normal observations](2026-10-26-focused-normal-observations.md) | System-allocator selected-path observations preserve raw timing/CPU, explicit null counters and unqualified scope |

Start a measurement from the [performance guide](../../performance.md).
