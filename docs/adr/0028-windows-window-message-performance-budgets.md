# ADR 0028: Budget Windows window-message delivery

- **Status:** Accepted
- **Date:** 2026-08-10
- **Resolves gate:** _none_
- **Supersedes:** _none_

## Context

ADR 0026 accepted the Windows Phase 2 native profiles before ordinary-window
`WindowMessage` became a production route. This change adds that route to the
same native input harness and measures the public Rust facade against a real
unelevated top-level fixture. A retained sample is correct only when exact-target
pre/post fences pass, the receipt reports `TargetQueueAdmission`, and the
selected fixture reports the expected message families; queue admission alone
is not the oracle.

The change design fixed ceilings before measurement: 1 ms p50 and 2 ms p95 for
one native unit, 6 ms p95 for a two-unit button event, 10 ms for immediate
queue-pressure outcomes, 250 ms for cancellation/deadline cleanup, and 64 KiB
for sequence memory. The last number was a hypothesis rather than an observed
baseline.

## Decision

The Windows native input profile in
[`phase-2-native-input-x86_64-pc-windows-msvc.toml`](../benchmarks/phase-2-native-input-x86_64-pc-windows-msvc.toml)
adds three ordinary-window workloads:

- one pointer move, budgeted at 1 ms p50, 2 ms p95, and 64 KiB peak Rust heap;
- one positioning move plus a two-unit primary-button event, budgeted at 6 ms
  p95 and 64 KiB peak Rust heap; and
- the maximum accepted 256-event sequence, budgeted at 256 KiB peak Rust heap.

The first two retain their fixed pre-measurement ceilings. The maximum-sequence
ceiling replaces the rejected 64 KiB hypothesis. The harness defines
`peak_allocated_bytes` as the complete workload footprint above the
pre-fixture baseline, so it includes the ordinary fixture reader, engine and
controller, request, sequence, receipt, and route state. The final bound profile
measured 66,811 bytes, 1,275 bytes above 64 KiB. Although the sequence's
peak-over-steady increment was only 21,585 bytes, applying that narrower reading
to the published aggregate metric would make the budget false. The replacement
256 KiB ceiling is the next 64 KiB boundary above three times the observed
aggregate peak; it is a regression ceiling, not a user-facing allocation
promise.

The approved native matrix also enforces these wall-clock ceilings directly:

| Row | Ceiling |
|---|---:|
| Queue-full refusal before any native unit | 10 ms |
| Queue-full partial refusal after one native unit | 10 ms |
| Hung-target queue admission | 10 ms |
| Mid-sequence deadline cleanup | 250 ms |
| Mid-sequence cancellation cleanup | 250 ms |

Every timer surrounds the public controller operation or, for externally
triggered cancellation, the interval from cancellation to the joined receipt.
These are approved-host regression checks; they do not promise scheduler latency
on arbitrary Windows hosts.

## Evidence

The retained final raw profile
[`native-phase2-input-window-message-b72a95f.log`](../evidence/phase-2-performance/native-phase2-input-window-message-b72a95f.log)
is bound to source commit `b72a95fa144f3e55855bdf1ba43f833a4b986f91`
and tree `2502e334f254191f873722afc6258d77be9eb02f`. Fifty retained
samples after five warmups produced zero oracle failures:

| Workload | p50 | p95 | Peak Rust heap | Growth |
|---|---:|---:|---:|---:|
| One ordinary pointer unit | 0.2782 ms | 0.3504 ms | 45,227 B | 1,020 B |
| Position plus primary-button event | 0.6866 ms | 0.9568 ms | 45,783 B | 1,180 B |
| Maximum 256-event sequence | 63.8546 ms | 66.6373 ms | 66,811 B | 0 B |

The final failed 64 KiB comparison is therefore
`66,811 > 65,536` by 1,275 bytes. This is evidence against the proposed
ceiling, not evidence of a leak: the retained maximum-sequence workload returned
to zero post-warmup growth.

The earlier
[`native-phase2-input-window-message-fd71a3f.log`](../evidence/phase-2-performance/native-phase2-input-window-message-fd71a3f.log)
first falsified 64 KiB at 66,685 bytes, but its source fields did not include
then-uncommitted C/C++ example changes. It remains rejected provenance. The
later `b8d8de7` profile was accepted before review, then superseded by the
`5aa0c07` rerun when review strengthened the C++ operation oracle and rejected
trailing fixture events. Hosted CI subsequently exposed a stale WGC resize
surface; the production fix in `b72a95f` required the final profile above even
though no input ceiling changed.

The final native-matrix rerun
[`window-message-native-b72a95f.log`](../evidence/phase-2-performance/window-message-native-b72a95f.log)
is bound to the same source commit and tree. It records 3.880 ms for full
refusal, 3.772 ms for partial refusal, 4.771 ms for hung-target queue admission,
23.621 ms for deadline cleanup, and 2.888 ms for cancellation cleanup. All five
rows pass their fixed ceilings. The earlier accepted pressure-only
[`window-message-pressure-cd08dce-rerun.log`](../evidence/phase-2-performance/window-message-pressure-cd08dce-rerun.log)
remains historical evidence for its source revision rather than the final
implementation binding.

Three rejected runs remain visible rather than being averaged into the accepted
one. [`window-message-pressure-fd71a3f.log`](../evidence/phase-2-performance/window-message-pressure-fd71a3f.log)
found that process-lifetime fixture counters included unrelated
`WM_MOUSEMOVE` generated by geometry changes; commit `cd08dce` changed the test
oracle to compare per-request deltas. The immediately preceding
[`window-message-pressure-cd08dce.log`](../evidence/phase-2-performance/window-message-pressure-cd08dce.log)
then rejected external physical-cursor movement during the run. The final-source
[`window-message-native-b72a95f-cursor-rejected.log`](../evidence/phase-2-performance/window-message-native-b72a95f-cursor-rejected.log)
likewise rejected an externally moved cursor before an uncontaminated rerun
passed. Neither cursor failure weakened the invariant.

## Alternatives

- **Keep 64 KiB and subtract the fixture's steady footprint.** Rejected because
  the committed profile exposes aggregate `peak_allocated_bytes`; silently
  changing the meaning for one row would create a second memory metric.
- **Optimize until the aggregate happens to fit 64 KiB.** Rejected because
  the final excess is 1,275 bytes, growth is zero, and no product or safety requirement
  justifies coupling implementation layout to an unmeasured estimate.
- **Rely only on end-to-end latency.** Rejected because queue pressure and
  cleanup are failure-path contracts and need independently named ceilings.

## Consequences

CI can validate the profile shape, hard correctness/growth predicates, native
row assertions, and retained source bindings. The interactive Windows host
remains required to remeasure the native workloads. A later ceiling change must
retain the failing raw profile and explain whether the production contract, the
measurement scope, or the implementation changed.
