# ADR 0057: Native Rust template-watch support

- **Status:** Accepted
- **Date:** 2026-08-29
- **Resolves gate:** _none_
- **Supersedes:** _none_

## Context

The Rust template watcher and replay/OpenCV profile were already accepted, but native application support remained withheld. Separate native capture profiles could not prove the integrated maintained-session watcher path: source authority, geometry resets, confirmed-only stability, scheduler work, cancellation, target loss, retained ownership, and cleanup had to pass together on production WGC and ScreenCaptureKit sessions.

ADR 0053 fixed independent Apple Silicon and Windows budgets from exact-source precursor cohorts before enforcement. The final apparatus then ran five fresh budget-enforcing processes per approved host over one identical 24-workload semantic registry. The tracked aggregate is [Phase 4 native template-watch qualification](../evidence/phase-4-native-template-watch-qualification.md).

## Decision

Accept `Session::start_template_watch` as a supported Rust facade operation over maintained Windows WGC and macOS ScreenCaptureKit window/display sessions on the existing qualified platform floors. The support statement is limited to the public Rust query/poll/wait/cancel/result boundary and the invariants proven by the repository native matrices.

Do not infer support for OCR predicates, callbacks or subscriptions, C ABI/C++, automatic input, target activation, arbitrary application/template/ROI compatibility or timing, real-time guarantees, packaging, artifacts, tags, or a `v0.4.0` release.

## Alternatives

- Keep native watcher support withheld after both final matrices passed. Rejected because it would contradict exact-source evidence for the production Rust session boundary and leave implemented behavior undocumented.
- Promote watcher callbacks, C/C++, or automatic input with the Rust boundary. Rejected because those public contracts do not exist and received no qualification.
- Claim general native application compatibility from the repository fixture. Rejected because controlled marker geometry, pixels, topology, and timing cannot establish arbitrary caller content or application behavior.
- Copy one target's latency or resource limits to the other. Rejected by ADR 0053; WGC/D3D11 and ScreenCaptureKit/Core Video retain independent measured ceilings.

## Consequences

- Rust callers may use the existing target-specific facade constructors, open a maintained native session, and start a bounded template query without a caller frame-polling loop.
- macOS callers must grant Screen Recording to the hosting application before capture. MadoPilot remains non-prompting. Windows adds no permission UI, permission probe, or elevation behavior.
- Query and caller-wait deadlines remain separate. Terminal outcomes, exact source correlation, confirmed-only stability, finite scheduling, stale-result rejection, retained ownership, and idempotent close remain mandatory.
- The public example and native watcher documentation must preserve explicit target selection and must not activate targets, inject input, prompt for permissions, or present fixture performance as an application SLA.
- The two revision-bound profiles remain historical precursor records. Final hashes attach to the qualification aggregate rather than replacing earlier source-bound facts.
- C ABI 1.5, the C header, and the C++ wrapper do not change. Existing cross-language checks are regression proof only and are not native watcher API qualification.
- Packaging, crates.io/static artifacts, tags, and release delivery remain separate open work.

## Verification

The decision is enforced by:

- `docs/benchmarks/phase-4-native-template-watch-aarch64-apple-darwin.toml` and `docs/benchmarks/phase-4-native-template-watch-x86_64-pc-windows-msvc.toml`;
- `benchmark_block_drift` and `hard_budget_drift`, which bind workload order, plans, source-shaped identity, hard predicates, and target-specific ceilings;
- deterministic runtime, facade, fixture-protocol, privacy-schema, scheduler, ownership, cancellation, deadline, close, and diagnostics tests;
- five accepted final processes per approved host with no process retry, exclusion, reorder, extra priming, or sample replacement;
- complete-diff applicability from the executed cohort source through the proposed protected merge candidate;
- independent code/concurrency/specification and focused security/privacy/memory-safety review before protected delivery;
- hosted Windows, macOS, repository-policy, and branch-flow checks on the proposed merge candidate.

A future change to capture publication, mapping, watcher admission, matching, stability, lifecycle, diagnostics, fixture authority, workload/profile enforcement, or the public support statement requires affected native rows to rerun or receive reviewed complete-diff applicability. A false skip or stale successful commit invalidates this acceptance regardless of latency.
