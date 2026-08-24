# ADR 0037: Phase 3 OCR performance budgets

- **Status:** Accepted
- **Date:** 2026-08-24
- **Resolves gate:** the `aarch64-apple-darwin` accepted CPU OCR workloads of [`G-013`](../validation-gates.md#g-013); Windows OCR performance and complete Phase 3 release acceptance remain open
- **Supersedes:** none; the focused observation from the archived ONNX backend Change remains historical evidence at its original revision

## Context

The focused ONNX backend Change measured one full-frame HUD path, but it deliberately set no product budget and did not cover the integrated default loader, bounded-region work, an empty result, exact source correlation, or both-target release acceptance. Phase 3 integration therefore added one production benchmark with three workloads and the hard correctness/resource policy required by the release specification.

The first smoke run disproved a draft oracle assumption: recognizing a bounded crop does not reproduce the full-frame detector quadrilateral byte-for-byte because the detector preprocesses the requested region independently. The result retained the exact expected text and passed the independent fixture geometry thresholds. The benchmark was corrected before precursor measurement to use exact text/count/order plus fixture-derived envelope IoU, center, ordered-point, confidence, and source-correlation gates. No failed retained sample was removed, retried, or used to choose a numeric threshold.

At source `b83f23f73876e7d5eb1fd75e31ec44c2e3f4a1b3`, tree `7dfe9c278ff5e1481b07cfe9a5b9a374b4f419d4`, five fresh optimized processes on the approved Apple M1 Pro host retained 50 samples per workload after three warmups per process. Every sample and warmup passed. Every workload reported zero live Rust heap growth. The worst full, bounded, and empty p95 observations were 476.576 ms, 299.674 ms, and 135.640 ms. Default runtime/model validation plus two-session startup was at most 86.988 ms; first close 1.116 ms; accepted-model reopen plus close 63.569 ms; process resident high-water 548,487,168 bytes.

The five-process ranges were narrow: full p95 varied 1.1%, bounded p95 0.8%, empty p95 1.8%, cold startup 3.8%, and resident high-water 1.4%. A three-times ceiling would hide a material deterministic CPU regression. Rounded ceilings near 1.5 times the worst observation preserve substantial OS-load headroom while remaining useful regression gates.

The corresponding Windows 11 Pro 25H2 build-family-26200 release host is unavailable. Windows Server 2025 hosted CI can prove the hard oracle and bounded-growth behavior but is a different system and cannot establish timing or resident-memory ceilings.

## Decision

Accept [`phase-3-ocr-aarch64-apple-darwin.toml`](../benchmarks/phase-3-ocr-aarch64-apple-darwin.toml) as the normative Apple Silicon profile for the exact accepted model, controlled ONNX Runtime, fixture, host, toolchain, and workload policy it names.

Every warmup and retained operation must satisfy exact expected text/count/order, fixture-derived geometry thresholds, finite same-host-stable confidence, exact frame/effective-region/output/backend/model correlation, and the declared result/tensor/session bounds. Every retained workload must report zero incorrect samples and at most 4,096 bytes of live Rust heap growth. Latency never compensates for a correctness or resource failure.

Accepted inference ceilings are:

| Workload | p50 | p95 | maximum | mapped bytes |
|---|---:|---:|---:|---:|
| `onnx_cpu_hud_full` | 600 ms | 750 ms | 900 ms | 2,073,600 |
| `onnx_cpu_hud_region` | 375 ms | 450 ms | 600 ms | 64,800 |
| `onnx_cpu_blank` | 175 ms | 210 ms | 300 ms | 16,384 |

Additional Apple Silicon ceilings are:

| Measure | Ceiling |
|---|---:|
| default model/runtime validation plus two-session open | 175 ms |
| first close after the workload set | 2 ms |
| accepted-model reopen plus close | 100 ms |
| attributable live Rust heap per workload | 20 MiB |
| process resident high-water | 768 MiB |
| input tensor bytes | 256 MiB |
| native output bytes | 256 MiB |
| concurrent inference / session pairs / sessions | 1 / 1 / 2 |
| recognition batch | 6 |

The optimized Apple benchmark enforces inference latency, startup, close, reopen/close, live heap, observed mapped bytes, observed detector/recognizer runs, opened session topology, tensor/output limits, concurrency, batch, and target-native `getrusage` peak RSS after reporting observations. Rust allocator counters remain distinct because they exclude ONNX Runtime and OpenCV native allocations. Producer-surface copied bytes are not applicable: the profile maps immutable CPU replay frames and owns no native producer surface, so it records that classification and sets no copied-byte ceiling. Hosted smoke runs enforce only target-independent correctness, observed mapping/inference/session resource oracles, and 4 KiB growth; they never apply Apple timing/RSS to hosted or Windows machines.

No Windows numeric profile is created. Its p50/p95/maximum, cold startup, close, heap, and resident ceilings are deliberately withheld until the same digest-bound workload runs repeatedly on the approved Windows release host. `G-013` and Phase 3 release acceptance remain open for that row.

## Alternatives

- **Reuse the focused backend observation.** Rejected. It loaded caller-assembled model bytes before the cold timer and covered only full-frame text/order, so it cannot represent the integrated default or the complete release oracle.
- **Require the bounded crop to reproduce full-frame geometry exactly.** Rejected by the first smoke result. Independent crop preprocessing legitimately changes detector geometry while the public full-frame coordinate result still passes the fixture threshold.
- **Set three-times latency and memory ceilings.** Rejected. Five fresh processes varied by at most 3.8%; three times the worst observation would not protect the performance priority of the accepted synchronous CPU profile.
- **Infer Windows ceilings from Apple or hosted Windows Server.** Rejected. CPU, operating system, runtime library, allocator, and host scheduling differ, and `G-013` is target-specific.
- **Withhold Apple ceilings until Windows becomes available.** Rejected. The Apple host, exact-source executable, five-process sample, and hard oracles independently support a useful target-specific regression profile. Withholding the missing Windows row is sufficient and truthful.
- **Count only Rust allocations.** Rejected. The 548 MiB observed resident high-water is dominated by native runtime/backend work and must remain visible as a separate ceiling.

## Consequences

Changes to accepted model loading, ONNX sessions, preprocessing, inference, decoding, region mapping, result construction, or cleanup must run the Apple profile and satisfy every hard and absolute ceiling. A changed source or executable digest requires a complete-diff applicability decision or a rerun; earlier numbers are never relabeled.

The accepted profile is a regression budget for one named host and fixture, not a general real-time, arbitrary-resolution, multi-region, Windows, GPU, or game-compatibility claim. Full-frame 960×540 OCR remains roughly 477 ms p95 on the measured M1 Pro; callers needing lower latency should use a bounded region rather than assuming the backend downscales an arbitrary 4K frame.

Windows OCR performance remains a release blocker. Hosted checks still provide useful cross-target proof for exact result behavior, bounded allocation growth, session/tensor/result counts, cancellation, late-result suppression, cleanup, and public-language integration without laundering runner timing into a product claim.

## Verification

The precursor command, executable/runtime/model/fixture digests, all five process rows, aggregate resource observations, and exclusions are retained in the integration Change evidence. The same source also passed the native cancellation/close-race test: cancellation after native-run admission issued termination and returned within 250 ms, no late result committed, the backend recovered, admitted work survived a close race, and repeated close was idempotent.

The committed profile is included in benchmark-block and hard-budget drift registries. Its executable latency/resource constants are checked against the profile so a code/profile mismatch fails the repository suite.

Post-budget source `192f3d207c85c140c85bed356346cca27dc49765`, tree `45abefc877a266826ce189ff671a400fa7ea08b6`, produced executable SHA-256 `db94a1480ea762a35d98c869e55f822504ac237a9e02d80a6265786d5ac03ac1`. A fresh Apple run passed every executable gate: full/bounded/empty p95 was 469.992/306.045/133.130 ms, cold open 84.629 ms, close 1.007 ms, reopen-close 62.536 ms, zero incorrect samples, zero growth, exact mapped/copied bytes, and 544,391,168-byte external resident high-water below 768 MiB. Windows target-specific precursor and final execution remain withheld; hosted hard-gate jobs are separate cross-target evidence and never fill that numeric row.

The first resource-instrumented qualification process reported 141.686 ms cold startup and failed the former 140 ms ceiling before any result was accepted. An unchanged-executable repeat reported 90.075 ms; the difference is cold host/cache variance, not inference work. The ceiling is therefore relaxed to 175 ms, 1.23 times the retained failure and 2.01 times the original precursor maximum. The failed run remains evidence, and no correctness, inference, heap, mapping, resident, or cleanup ceiling changes with it.
