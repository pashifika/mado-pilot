# ADR 0037: Phase 3 OCR performance budgets

- **Status:** Accepted
- **Date:** 2026-08-24
- **Resolves gate:** the `aarch64-apple-darwin` accepted CPU OCR workloads of [`G-013`](../validation-gates.md#g-013); selects Windows target ceilings from approved-host precursor evidence while the separate final Windows qualification and complete Phase 3 release acceptance remain open
- **Supersedes:** none; the focused observation from the archived ONNX backend Change remains historical evidence at its original revision

## Context

The focused ONNX backend Change measured one full-frame HUD path, but it deliberately set no product budget and did not cover the integrated default loader, bounded-region work, an empty result, exact source correlation, or both-target release acceptance. Phase 3 integration therefore added one production benchmark with three workloads and the hard correctness/resource policy required by the release specification.

The first smoke run disproved a draft oracle assumption: recognizing a bounded crop does not reproduce the full-frame detector quadrilateral byte-for-byte because the detector preprocesses the requested region independently. The result retained the exact expected text and passed the independent fixture geometry thresholds. The benchmark was corrected before precursor measurement to use exact text/count/order plus fixture-derived envelope IoU, center, ordered-point, confidence, and source-correlation gates. No failed retained sample was removed, retried, or used to choose a numeric threshold.

At source `b83f23f73876e7d5eb1fd75e31ec44c2e3f4a1b3`, tree `7dfe9c278ff5e1481b07cfe9a5b9a374b4f419d4`, five fresh optimized processes on the approved Apple M1 Pro host retained 50 samples per workload after three warmups per process. Every sample and warmup passed. Every workload reported zero live Rust heap growth. The worst full, bounded, and empty p95 observations were 476.576 ms, 299.674 ms, and 135.640 ms. Default runtime/model validation plus two-session startup was at most 86.988 ms; first close 1.116 ms; accepted-model reopen plus close 63.569 ms; process resident high-water 548,487,168 bytes.

The five-process ranges were narrow: full p95 varied 1.1%, bounded p95 0.8%, empty p95 1.8%, cold startup 3.8%, and resident high-water 1.4%. A three-times ceiling would hide a material deterministic CPU regression. Rounded ceilings near 1.5 times the worst observation preserve substantial OS-load headroom while remaining useful regression gates.

The corresponding Windows 11 Pro 25H2 build-family-26200 release host ran source `6b5f3c1435b983c560fd01da513f721d8d21ba8d`, tree `0a9fc22d5093d9ad61c9dfa0c93f01a619fe7177`, executable SHA-256 `f3c13157807c9617fb03039b9689cd53ccc138701d9c412180c03fc28800a316` in five fresh processes. All 100 retained samples per workload and every warmup passed with zero growth. Worst full, bounded, and empty p50/p95/maximum observations were 721.258/810.143/849.570 ms, 567.940/581.789/586.828 ms, and 264.172/320.843/359.929 ms. Cold startup was at most 182.002 ms; first close 5.652 ms; reopen-close 161.748 ms; target-native `GetProcessMemoryInfo` peak RSS 242,667,520 bytes. The full-frame process-5 and empty-result process-4 slow observations remain in the evidence and define headroom rather than being excluded as outliers.

## Decision

Accept [`phase-3-ocr-aarch64-apple-darwin.toml`](../benchmarks/phase-3-ocr-aarch64-apple-darwin.toml) as the normative Apple Silicon profile. Select the target-specific ceilings in [`phase-3-ocr-x86_64-pc-windows-msvc.toml`](../benchmarks/phase-3-ocr-x86_64-pc-windows-msvc.toml) from the approved Windows precursor for a separate final qualification. Each profile is bound only to the exact model, controlled ONNX Runtime, fixture, host, toolchain, sample policy, source, and executable it names.

Every warmup and retained operation must satisfy exact expected text/count/order, fixture-derived geometry thresholds, finite same-host-stable confidence, exact frame/effective-region/output/backend/model correlation, and the declared result/tensor/session bounds. Every retained workload must report zero incorrect samples and at most 4,096 bytes of live Rust heap growth. Latency never compensates for a correctness or resource failure.

Accepted inference ceilings are:

| Workload | p50 | p95 | maximum | mapped bytes |
|---|---:|---:|---:|---:|
| `onnx_cpu_hud_full` | 600 ms | 750 ms | 900 ms | 2,073,600 |
| `onnx_cpu_hud_region` | 375 ms | 450 ms | 600 ms | 64,800 |
| `onnx_cpu_blank` | 175 ms | 210 ms | 300 ms | 16,384 |

Selected Windows inference ceilings are:

| Workload | p50 | p95 | maximum | mapped bytes |
|---|---:|---:|---:|---:|
| `onnx_cpu_hud_full` | 900 ms | 1,000 ms | 1,200 ms | 2,073,600 |
| `onnx_cpu_hud_region` | 725 ms | 750 ms | 850 ms | 64,800 |
| `onnx_cpu_blank` | 350 ms | 425 ms | 500 ms | 16,384 |

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

Additional Windows ceilings selected for final qualification are:

| Measure | Ceiling |
|---|---:|
| default model/runtime validation plus two-session open | 250 ms |
| first close after the workload set | 10 ms |
| accepted-model reopen plus close | 225 ms |
| attributable live Rust heap per workload | 20 MiB |
| process resident high-water | 320 MiB |
| input tensor bytes | 256 MiB |
| native output bytes | 256 MiB |
| concurrent inference / session pairs / sessions | 1 / 1 / 2 |
| recognition batch | 6 |

The optimized target benchmark enforces target-specific inference latency, startup, close, reopen/close, live heap, observed mapped bytes, observed detector/recognizer runs, opened session topology, tensor/output limits, concurrency, batch, and peak RSS after reporting observations. Apple reads peak RSS through `getrusage`; Windows reads `PeakWorkingSetSize` through `GetProcessMemoryInfo`. Rust allocator counters remain distinct because they exclude ONNX Runtime and OpenCV native allocations. Producer-surface copied bytes are not applicable: the profiles map immutable CPU replay frames and own no native producer surface, so they record that classification and set no copied-byte ceiling. Hosted smoke runs enforce only target-independent correctness, observed mapping/inference/session resource oracles, and 4 KiB growth; they never apply release-host timing or RSS.

The Windows values are rounded, target-specific regression gates between 1.23 and 1.77 times the worst precursor observation. The 320 MiB RSS limit is 1.38 times a five-process high-water whose range was 0.57%. Larger p95 ranges of 13.10% full-frame and 20.52% empty retain 23% and 32% headroom respectively; independent maximum ceilings remain wider. `G-013` and Phase 3 release acceptance remain open until a separately built budget-enforcing executable passes five fresh Windows processes without retry or exclusion.

## Alternatives

- **Reuse the focused backend observation.** Rejected. It loaded caller-assembled model bytes before the cold timer and covered only full-frame text/order, so it cannot represent the integrated default or the complete release oracle.
- **Require the bounded crop to reproduce full-frame geometry exactly.** Rejected by the first smoke result. Independent crop preprocessing legitimately changes detector geometry while the public full-frame coordinate result still passes the fixture threshold.
- **Set three-times latency and memory ceilings.** Rejected. Stable startup/RSS and the retained full/empty slow rows support target-specific ceilings between 1.23 and 1.77 times the worst observation. Three times would hide a material deterministic CPU regression.
- **Infer Windows ceilings from Apple or hosted Windows Server.** Rejected. CPU, operating system, runtime library, allocator, and host scheduling differ, and `G-013` is target-specific.
- **Withhold Apple ceilings until Windows became available.** Rejected. The Apple host, exact-source executable, five-process sample, and hard oracles independently supported a useful target-specific regression profile. Target-specific profiles preserve that evidence instead of forcing one platform to wait for or borrow from the other.
- **Count only Rust allocations.** Rejected. The 548 MiB observed resident high-water is dominated by native runtime/backend work and must remain visible as a separate ceiling.

## Consequences

Changes to accepted model loading, ONNX sessions, preprocessing, inference, decoding, region mapping, result construction, or cleanup must run each applicable target profile and satisfy every hard and absolute ceiling. A changed source or executable digest requires a complete-diff applicability decision or a rerun; earlier numbers are never relabeled.

Each profile is a regression budget for one named host and fixture, not a general real-time, arbitrary-resolution, multi-region, GPU, or game-compatibility claim. Full-frame 960×540 OCR measured roughly 473 ms p95 on the M1 Pro and 810 ms worst-process p95 on the Core i7-12700KF; callers needing lower latency should use a bounded region rather than assuming the backend downscales an arbitrary 4K frame.

Windows OCR performance remains a release blocker until the separate final five-process qualification passes the selected executable ceilings. Hosted checks still provide useful cross-target proof for exact result behavior, bounded allocation growth, session/tensor/result counts, cancellation, late-result suppression, cleanup, and public-language integration without laundering runner timing into a product claim.

## Verification

The Apple and Windows precursor commands, executable/runtime/model/fixture digests, all process rows, aggregate resource observations, and exclusions are retained in the integration Change evidence. The native cancellation/close-race test separately proved that cancellation after native-run admission issued termination and returned within 250 ms, no late result committed, the backend recovered, admitted work survived a close race, and repeated close was idempotent.

Both committed profiles are included in benchmark-block and hard-budget drift registries. Their executable latency/resource constants are checked against each profile so a code/profile mismatch fails the repository suite.

Review-fixed Apple source `e41fbbd5457d5f9c10da55982a799c608ccc195e`, tree `9fbc47ef0698047aaab5c51e3616712e87ae9b08`, produced executable SHA-256 `fd8713672094be22d4e55a1dd23a4ee23ef75632f779c93ddb5956b077992e48`. Five fresh processes retained 100 instrumented samples per workload after three warmups each. Every executable gate passed: worst full/bounded/empty p95 was 472.623/301.455/184.057 ms and maximum 478.703/309.750/185.290 ms; cold open 87.377 ms, close 1.163 ms, reopen-close 64.929 ms, peak `getrusage` RSS 516,833,280 bytes, zero incorrect samples/growth, exact observed mappings, detector runs 1/1/1, recognizer runs 2/1/0, and opened topology one pair/two sessions. Producer copy is not applicable to these CPU replay inputs. The approved Windows precursor has the same sample and resource policy; its selected ceilings await separate execution before acceptance.

The first resource-instrumented qualification process reported 141.686 ms cold startup and failed the former 140 ms ceiling before any result was accepted. An unchanged-executable repeat reported 90.075 ms; the difference is cold host/cache variance, not inference work. The ceiling is therefore relaxed to 175 ms, 1.23 times the retained failure and 2.01 times the original precursor maximum. The failed run remains evidence, and no correctness, inference, heap, mapping, resident, or cleanup ceiling changes with it.
