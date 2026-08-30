# ADR 0059: No speculative ScreenCaptureKit suspension repair

- **Status:** Accepted
- **Date:** 2026-08-30
- **Resolves gate:** _none_
- **Supersedes:** _none_

## Context

ADR 0057 withholds native template-watch support after the fifth Apple qualification process opened a fresh ScreenCaptureKit producer that reported `SCFrameStatusSuspended` and never delivered a complete frame before the existing deadline. Later green diagnostics established that the failure was intermittent, but they could not replace or relabel the terminal-red process.

PR #61 added private status, callback, lifecycle, ownership, and resource observation plus a comparison-only sample-queue drain seam. Its predeclared single-display experiment ran eight separate processes over `none`, `mapping_only`, `frame_only`, and `both` retainers with the production close order and with one queue-drain sentinel. All eight exact-source processes completed and restored their process baselines. Mapping-only retained no native lease or detached bytes. Frame-bearing variants retained one expected lease and 4,628,480 detached bytes while the fresh producer still completed. No process observed suspended, stopped, missing, unknown, or any other non-complete status, and no callback was refused or left in flight.

The retained result is documented in [ScreenCaptureKit suspension diagnosis](../evidence/macos-sck-suspension-diagnostic.md). It rejects mapping ownership and frame ownership as necessary deterministic causes on the exercised host and topology, but it does not reproduce or explain the historical intermittent suspension.

## Decision

Do not change production ScreenCaptureKit close ordering, detached-frame ownership, stream recovery, deadlines, status reporting, or any public Rust/C/C++ contract without a retained failure signature that selects that change.

Remove the comparison-only `drain_sample_queue` execution policy and gate from current source. Retain only feature-gated bounded status, callback, lifecycle, ownership, and resource observability needed to interpret the frozen diagnostic evidence. Native template-watch support remains withheld under ADR 0057.

## Alternatives

- Make sample-queue drain permanent. Rejected because all unchanged rows completed and no failure-case pair demonstrated that the sentinel changes progress. Permanent drain would add close work and ordering without measured benefit.
- Move detached frames to a smaller lease owner. Rejected because frame-bearing rows completed with their expected lease and bytes. The experiment did not show that retaining session-owned state caused the suspension.
- Add automatic new-epoch recovery for suspension. Rejected because no suspended sequence was observed, so recovery would hide an unclassified framework state and add unmeasured target, cancellation, and stale-result behavior.
- Expose native status or a new typed failure publicly. Rejected because the diagnostic observed only complete frames and therefore established no caller-visible contract that needs representation.
- Extend the deadline, sleep, retry the failed process, or substitute a target. Rejected because each changes the frozen protocol or masks the failure without proving a cause.

## Consequences

- Product capture behavior and released APIs remain unchanged. Integrators do nothing differently.
- Feature-disabled builds retain no suspension diagnostic symbols, counters, transition ring, or comparison close path.
- The private diagnostic feature still incurs fixed counters and bounded snapshot state when explicitly enabled. Those costs are apparatus costs, not product budgets or support evidence.
- The queue-drain comparison cannot be rerun from successor source. Its exact executable, rows, aggregate, hashes, and source revision remain frozen evidence from `f0eab45c3918914098040b96458e5d583bf2a32a`.
- No repair enforcement point exists, so a repair mutation proof and repaired-source performance comparison are not applicable. Adding either later requires new failure evidence and a new decision.
- ADR 0057 remains authoritative. No formal repaired-source host cohort, support promotion, packaging, tag, or release claim follows from the green diagnostic.

## Verification

- The frozen eight-row aggregate at source `f0eab45c3918914098040b96458e5d583bf2a32a` passes the typed `mado_pilot_testkit::sck_suspension_report::validate_aggregate` validator and is summarized in [ScreenCaptureKit suspension diagnosis](../evidence/macos-sck-suspension-diagnostic.md).
- Focused macOS tests enforce status normalization, bounded rings, callback accounting, snapshot consistency, ownership retention, baseline restoration, and native exception containment.
- The production linkage test rejects `_mp_shim_sck_diagnostics_` and any diagnostic close-ordering symbol in a feature-disabled consumer.
- Protected macOS, Windows, repository-policy, and branch-flow checks plus independent concurrency/performance/memory-safety and security/privacy review apply before integration.
- Review must reject any production recovery, drain, owner refactor, deadline change, public status, or support claim that lacks a newer retained failure signature and ADR.
