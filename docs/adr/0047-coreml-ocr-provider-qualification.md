# ADR 0047: Reject CoreML OCR provider promotion

- **Status:** Accepted
- **Date:** 2026-08-26
- **Resolves gate:** CoreML candidate portion of `G-006`; CoreML provider budgets remain unassigned under `G-013`
- **Supersedes:** _none_; ADR 0046 policy and every CPU profile remain unchanged

## Context

ADR 0046 fixed the provider policy, placement terminology, quality/resource oracles, process counts, and stop rules before provider qualification. The approved Apple M1 Pro runtime can register CoreML and construct both accepted detector and recognizer sessions without initialization fallback.

The retained debug feasibility run passed the HUD text/count/order and normalized geometry tolerance. Qualification then used optimized exact-source executables, the accepted bounded detector profile, all eight singular and five grouped workloads, the same fixtures and public oracles as CPU, required CoreML, and no retry or exclusion.

## Evidence

The placement-instrumented process proved nonzero CoreML work in both sessions. Redacted unique-node assignment was:

| Session | CoreML nodes | CPU nodes |
|---|---:|---:|
| Detector | 6 | 8 |
| Recognizer | 27 | 44 |

Registration and placement therefore passed. The fixed smoke hard gate did not:

- every warmup and retained sample for `bounded_menu_wide` and `bounded_status_extreme_wide` failed the public quality oracle;
- a diagnostic-only rebuild changed no provider/model/oracle behavior and classified all eight failures as geometry, with no text, structure, confidence-range, or confidence-stability failure;
- the process exceeded the 250 ms active-cancellation return bound and terminated with `Timeout` before report serialization;
- the identical CPU smoke process passed all 13 workload rows and both cancellation paths.

The failed process is retained. It has no qualified p50/p95 report and is not converted into a performance result by discarding failed rows. Raw profiles and output stay private because they contain node names and host paths; revision/executable identities, redacted counts, and output hashes are recorded in the Change evidence.

## Decision

CoreML is not a supported v0.3.1 OCR execution provider. It receives no latency, startup, memory, or automatic-preference budget. On Apple release builds:

- `AutoPreferAccelerator` selects CPU;
- `PreferCoreMl` initializes a fresh CPU pair and reports `QualificationRejected` when CPU construction succeeds;
- `RequireCoreMl` returns provider-unavailable with `QualificationRejected` and publishes no engine;
- existing constructors and explicit CPU policy remain byte- and behavior-compatible.

The target-gated CoreML implementation and private qualification instrumentation may remain in source so a later Change can investigate the geometry/cancellation defects against new evidence. Their presence is not a support claim. Product builds do not enable ORT profiling or write profile files.

## Alternatives

- **Accept because CoreML placement was nonzero.** Rejected: placement proves execution location, not correct public results or bounded cancellation.
- **Relax geometry or cancellation after observing the failure.** Rejected: the oracles and bounds were fixed before execution, CPU passed them, and no platform mechanism justifies a different public contract.
- **Publish CoreML as explicit-only support.** Rejected: a required/manual flag does not make wrong geometry or unbounded cancellation safe.
- **Remove all CoreML code.** Rejected for now: retaining the target-gated implementation preserves reproducible failure evidence without exposing release support.

## Consequences

Apple automatic selection remains CPU and gains no accelerator performance claim. Callers can distinguish policy rejection from missing build capability, missing dependencies, and runtime/provider failure without receiving paths, native messages, device identity, graph names, pixels, or text.

A later CoreML proposal must retain this failed source, identify and fix both geometry and cancellation causes, rerun placement and every fixed hard gate, then execute fresh precursor/final processes before changing the release-qualified decision.
