# ADR 0087: v0.5.0 release performance qualification

- **Status:** Accepted; release measurement pending
- **Date:** 2026-10-20
- **Resolves gate:** none
- **Supersedes:** ADR 0086's requirement to finish statistical performance qualification within the individual optimization Change; its implementation contracts and recorded measurement verdicts remain unchanged

## Context

The operator expects further practical-use changes before v0.5.0 and requests
measurement and adjustment against the release candidate rather than repeated
full qualification of each intermediate optimization.

[ADR 0086](0086-template-matching-preparation-reuse-and-selection-work.md)
records successful semantic/work checks, a failed Apple comparison with 99
numerical conditions, and missing Windows performance evidence. Its separate
admission-only diagnostic confirms fewer BGR conversions without establishing
joint latency/CPU benefit. None of those records qualifies the final release.

## Decision

Keep correctness, source identity, cancellation, ownership, finite execution and
resource bounds, affected compatibility checks and required CI at development
integration. Move PR87's remaining statistical calibration, comparison,
non-regression and improvement decisions to the dedicated
`v0-5-0-pre-release-performance-measurement-and-adjustment` Change, executed after
the selected v0.5.0 source has settled and before release performance claims.

- The transferred task identities are PR87 1.3, 1.4, 6.2 and 6.3. Transfer means
  a new execution owner, not a passed or performed measurement. Historical Apple
  failures and Windows `not run` remain attributable to their original scope.
- Freeze the release source/tree, executables, consumer dependency, workloads,
  native loaded-library identities and execution configuration. Relevant later
  changes require affected reruns or a reviewed complete applicability record.
- Review any prospective measurement-method or budget change before collecting
  its candidate acceptance samples. Preserve existing frozen reports, profiles,
  hashes and adverse observations; do not regrade them under the new method.
  Both selected improvement goals and their non-regression obligations carry
  forward unless separately changed by an explicit decision.
- Separate component regression, consumer-shaped replay and actual consumer
  end-to-end observations; normal-build timing and instrumented physical-work
  data; allocation ownership and RSS/high-water. Missing native evidence or an
  inconclusive control remains a named gap, not a pass.
- Direct RGBA/BGRA matching input is a separate implementation Change. It does
  not block PR87 integration; if selected for v0.5.0, integrate it before the
  release candidate's measurements are frozen.

This decision changes no numerical threshold, product behavior, native execution
grant, release target or operating-system minimum. It does not reopen unrelated
accepted evidence automatically, transfer unfinished OCR/C++ Changes, promote
optional Phase 4 Lane C to a mandatory gate, or restore the OCR quality-based
model admission removed by [ADR 0084](0084-ocr-model-choices-and-test-results.md).

## Alternatives

- Complete a new full campaign after each intermediate change: duplicates work
  that relevant later source changes invalidate and does not establish final
  consumer performance.
- Declare the failed campaign passed or widen its ceilings: loses the recorded
  comparison and hides the memory/CPU tradeoff.
- Keep an unowned release TODO: provides no candidate-freeze, execution or
  disposition responsibility. A dedicated Change owns those steps instead.

## Consequences

The optimization's development checklist can finish without claiming performance
qualification. Release preparation owns fresh target-specific measurements and
evidence-supported adjustments or an explicitly reviewed disposition of unmet
claims. No implementation or release is accepted merely because its planning
artifacts are complete. Merge, archive, tag and publication remain separate
operations under [CONTRIBUTING.md](../../CONTRIBUTING.md#release-checklist).

The architecture baseline, performance guide, development release notes and
planning artifacts distinguish integration readiness from release evidence.
Integrators receive no new API or runtime policy from this decision.

## Verification

Review the transfer mapping against all four former open tasks and their new
unchecked execution tasks. Validate the three affected Change artifact sets and
require historical evidence/publication hashes to remain unchanged. Existing
semantic/ownership/compatibility checks remain mandatory; this documentation
update does not rerun them or turn prior CI into native performance evidence.

Release preparation must retain per-target, per-workload, revision-bound
`passed`, `failed`, `not run` or `incomplete` outcomes, including prospective
profile decisions and all campaign attempts. Artifact validation checks the
planning structure; a reviewer checks transfer completeness and claim scope.
No new benchmark budget or release qualification is asserted here.
