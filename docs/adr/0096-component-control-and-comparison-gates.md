# ADR 0096: Gate component controls and comparison cohorts independently

- **Status:** Accepted — apparatus contract only
- **Date:** 2026-10-23
- **Resolves gate:** _none_; source/host method and numerical profile reviews remain required
- **Supersedes:** _none_; introduces prospective numerical policy revisions

## Context

An unchanged A/A cohort must constrain the between-role difference used by the
candidate benefit decision. Stable values within each role alone cannot do so.
Likewise, every process staying below an upper ceiling does not constrain the
spread of faster observations. Both conditions are independent of a favorable
median benefit.

## Decision

Use `template-efficiency-v2-prospective-r2` and
`template-efficiency-v3-session-prospective-r2` for new numerical campaigns.
For every graded primary and turnover-stratum timing metric:

- A/A requires `abs(median(baseline6) - median(control6)) <= frozen_allowance`,
  in addition to each role's existing noise, order, drift and process ceilings.
- Control and comparison require each role independently to satisfy
  `spread / max(median(role6), 2 * frozen_clock_allowance) <= 0.15`.
  Each metric's maximum observed calibration resolution, including the existing
  target CPU floor, is frozen in its envelope. A later role cannot enlarge this
  denominator. It is an engineering allowance, not an accuracy proof.
- A role-local known failure remains failed if another role, metric or clock
  observation is incomplete. Missing values are not zero. Complete metric
  cohorts still undergo their independent checks against frozen allowances.
- Equality passes these two rejection gates. The separate joint normal p95 and
  CPU benefit still requires a difference strictly greater than the allowance.
  A passing benefit does not override any failure or incomplete prerequisite.

No sampling count, launch order, noise cap, calibration formula, resource budget,
retry policy or public workload oracle changes. No sample is removed or replaced.
The format-one historical numerical policy is unchanged.

## Evidence and consequences

Deterministic arithmetic exercises both directions of A/A bias, both comparison
roles, primary and turnover metrics, equality boundaries, role-clock inflation
and unavailable-clock failure retention.
The production grading smoke uses a frozen 2000-ns allowance: unchanged roles at
100000/97000 ns are refused, as is a comparison role with four 50000-ns and two
100000-ns values. Stable controls and comparisons pass those same arithmetic
checks. These are synthetic contract checks, not performance measurements.

Policy and collector identities are bound by method, campaign and profile.
Historical source, raw receipts and outcomes remain immutable and retain their
original collector/policy interpretation. They cannot authorize the revised
policy. New collector-bound runtime preflights and accepted method review must
precede calibration; independent control and numerical review must precede
candidate collection. Frozen executables may be reused under their actual
source identities when compiled inputs have not changed.

This decision accepts no method, numerical envelope or release result. Observer
eligibility, Windows normal image closure, actual-consumer measurement and all
other release obligations remain separate under ADR 0087 and ADR 0094.
