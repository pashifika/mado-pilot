# ADR 0060: Keep current native template-watch support withheld

- **Status:** Accepted
- **Date:** 2026-09-01
- **Resolves gate:** _none_
- **Supersedes:** _none_

## Context

ADR 0057 withheld native Rust template-watch support after the historical two-host qualification was rejected and replacement source `f16591f` ended Apple terminal red. ADR 0059 later repaired reproduced macOS capture-start and private fixture-lifetime defects without assigning either defect as the cause of that historical failure or promoting support.

The integrated current-source candidate `030398ec39f41b21d55f1f8331d07c540ed43863` froze one unchanged 24-workload protocol and the accepted target-specific ADR 0053 budgets before execution. Its first Apple process reached all workloads, then exited 101 with `cleanup_failed`. During `retained_result_mapping`, two fixture finalizations acknowledged control but did not acknowledge stop or finish bounded cleanup. An independent comparison of the diagnostic report against the accepted Apple profile also found `retained_result_mapping` p95 and maximum latency above their fixed limits. The driver retained process 1 as terminal red and did not launch processes 2 through 5.

The approved Windows host passed the corrected topology and fixture preflights, built one formal executable and fixture, and launched the same fail-fast protocol once. Windows process 1 exited 101 after 119.906 seconds with the typed `privacy_violation` gate at `producer_progress_cleanup_privacy`, stage `complete`, and zero report bytes. The extractor retained only bounded failure tokens plus stdout/stderr identities and counts; raw failure text remains ignored. Processes 2 through 5 were not launched, and no retry, replacement, exclusion, overlap, reorder, or extra priming occurred. The exact identities and allowlisted two-target aggregate are recorded in [Native template-watch current-source requalification](../evidence/native-template-watch-current-source-requalification.md).

## Decision

Keep Rust native template watching over Windows WGC and macOS ScreenCaptureKit implemented but unsupported. Each approved host produced an immutable terminal-red process, and either result is independently sufficient to withhold the cross-target boundary. Deterministic replay/OpenCV template watching remains supported under ADRs 0051 and 0052.

Do not retry, replace, or relabel either process 1, and do not launch either recorded suffix. This qualification Change makes no production repair and establishes no causal equivalence with a historical failure. Any repair selected from the current Apple cleanup/latency signature or Windows privacy-gate signature requires a separate corrective Change and fresh affected qualification.

## Alternatives

- Treat the diagnostic report's zero `result_correctness` sum as a green process. Rejected because the process exited nonzero after a cleanup hard gate and independently exceeded two accepted latency limits.
- Retry Apple process 1 or run processes 2 through 5. Rejected because the frozen fail-fast protocol makes a consumed terminal-red index immutable and records the remaining suffix as unlaunched.
- Promote Windows and macOS independently. Rejected because the public Rust native boundary covers both release targets and requires ten ordered green processes under target-specific profiles.
- Attribute the current cleanup failure to the historical ScreenCaptureKit suspension or to ADR 0059's repaired defects. Rejected because the current evidence reports a different observable signature and establishes no causal equivalence.

## Consequences

- Public support tables, examples, architecture, validation gates, and native watcher guidance continue to distinguish implemented native APIs from supported replay/OpenCV behavior.
- The current Apple and Windows results remain bound to source `030398e`; they do not replace or reinterpret ADR 0057, ADR 0059, PR #59, later bounded non-reproduction records, or frozen benchmark sections.
- Exact candidate `030398e` now has one immutable terminal-red process and four unlaunched indices on each approved host. Neither target result is a substitute for the other.
- OCR predicates, callbacks or subscriptions, C ABI/C++, automatic input, target activation, arbitrary application/template/ROI compatibility or timing, real-time guarantees, packaging, artifacts, tags, and `v0.4.0` remain outside the qualified boundary.

## Verification

- The retained Apple and Windows cohorts each record one terminal-red process, exit 101, indices 2 through 5 unlaunched, and zero retries, replacements, exclusions, overlap, reorder, or extra priming.
- Independent Apple accepted-profile comparison retains the two `retained_result_mapping` latency failures. Windows validation retains typed class `privacy_violation`, workload token `producer_progress_cleanup_privacy`, stage token `complete`, and zero report bytes without raw failure text.
- The typed aggregate contains ten ordered slots: two terminal red and eight unlaunched. Its support decision is `WITHHELD`.
- Frozen product and planning evidence inventories remain byte-identical. Documentation updates are outside executable, fixture, workload, oracle, extractor, validator, and budget semantics and cannot turn the red process green.
