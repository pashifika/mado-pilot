# ADR 0057: Native Rust template-watch support

- **Status:** Rejected
- **Date:** 2026-08-29
- **Resolves gate:** _none_
- **Supersedes:** _none_

## Context

The Rust template watcher and replay/OpenCV profile were already accepted, but native application support remained withheld. Separate native capture profiles could not prove the integrated maintained-session watcher path: source authority, geometry resets, confirmed-only stability, scheduler work, cancellation, target loss, retained ownership, and cleanup had to pass together on production WGC and ScreenCaptureKit sessions.

ADR 0053 fixed independent Apple Silicon and Windows budgets from exact-source precursor cohorts before enforcement. The final apparatus then ran five fresh budget-enforcing processes per approved host over one identical 24-workload semantic registry. The tracked aggregate is [Phase 4 native template-watch qualification](../evidence/phase-4-native-template-watch-qualification.md).

Independent review subsequently invalidated that promotion. The executed cohorts
did not prove a live query's `SchedulerClosed` outcome after engine destruction,
and the Windows topology row remained on one monitor instead of crossing the
approved DPI boundary. A corrected Apple cohort passed 5/5 at `c363826`, but the
same-source Windows cohort and a new cross-target aggregate remain incomplete.
The former acceptance below is therefore withdrawn rather than relabeled onto
the rejected measurements.

## Decision

Withhold native `Session::start_template_watch` support over Windows WGC and
macOS ScreenCaptureKit sessions until corrected same-semantic-source cohorts,
cross-target applicability, privacy/security review, and protected checks all
pass. The Rust APIs and example remain implemented, but implementation is not a
support claim.

Do not infer support for OCR predicates, callbacks or subscriptions, C ABI/C++,
automatic input, target activation, arbitrary application/template/ROI
compatibility or timing, real-time guarantees, packaging, artifacts, tags, or a
`v0.4.0` release.

## Alternatives

- Keep the ADR accepted because every recorded historical row reported success. Rejected because independent review found unexercised lifecycle and topology contracts; zero failures in an incomplete oracle cannot promote support.
- Promote watcher callbacks, C/C++, or automatic input with the Rust boundary. Rejected because those public contracts do not exist and received no qualification.
- Claim general native application compatibility from the repository fixture. Rejected because controlled marker geometry, pixels, topology, and timing cannot establish arbitrary caller content or application behavior.
- Copy one target's latency or resource limits to the other. Rejected by ADR 0053; WGC/D3D11 and ScreenCaptureKit/Core Video retain independent measured ceilings.

## Consequences

- Deterministic replay/OpenCV template queries remain supported under ADRs 0051 and 0052.
- Native WGC and ScreenCaptureKit watcher APIs remain implemented but unqualified for public support.
- The public example and native watcher documentation must state the pending boundary while preserving explicit target selection, non-prompting permissions, no activation or input, and fixture-only performance scope.
- The revision-bound precursor budgets and rejected final measurements remain historical evidence. Their hashes must not be rewritten as current acceptance.
- C ABI 1.5, the C header, and the C++ wrapper do not change. Existing cross-language checks remain regression proof only and are not watcher API qualification.
- Packaging, crates.io/static artifacts, tags, and release delivery remain separate open work.

## Verification

Reconsidering this decision requires:

- five fresh corrected final processes on both approved hosts at one reviewed semantic source, with no retry, exclusion, reorder, extra priming, or sample replacement;
- a Windows topology transition to a monitor different from the authenticated fixture's current monitor with an effective-DPI change;
- a live native query proving immutable `SchedulerClosed` after engine destruction;
- complete workload, budget, lifecycle, cleanup, and privacy aggregates with revision/tree/executable/fixture provenance;
- complete-diff applicability through the proposed protected merge candidate;
- independent code/concurrency/specification and focused security/privacy/memory-safety re-review;
- hosted Windows, macOS, repository-policy, and branch-flow checks on the proposed merge candidate.

Any false skip, stale successful commit, or unsupported reuse of a rejected process
keeps native support withheld regardless of latency.
