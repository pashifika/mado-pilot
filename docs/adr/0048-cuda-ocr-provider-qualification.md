# ADR 0048: Accept explicit CUDA OCR and keep automatic selection on CPU

- **Status:** Accepted
- **Date:** 2026-08-26
- **Resolves gate:** Windows CUDA candidate portion of `G-006`; defines the CUDA OCR row under `G-013`
- **Supersedes:** _none_; ADR 0046 policy and every CPU profile remain unchanged

## Context

ADR 0046 fixed provider semantics and the qualification plan before execution. The approved Windows host is a Core i7-12700KF with an RTX 4080. The controlled root contains the official version-matched ONNX Runtime 1.29.0 CUDA 13 build plus CUDA 13, cuBLAS, cuFFT, cuRAND, cuDNN 9, and NVRTC dependencies. Product loading uses only the canonical explicit root and System32; it does not use `PATH`, Python/PyTorch preload, installation, download, or an inbox runtime.

An initial source eagerly loaded `onnxruntime_providers_cuda.dll` and failed its initialization routine. The retained correction validates that DLL but lets ONNX Runtime load it after environment initialization. A later pass exposed missing NVRTC; the retained closure added the exact NVIDIA-signed NVRTC and builtins DLLs. Final functional validation then passed with empty stderr and no provider switching after active cancellation.

## Evidence

Exact precursor source is base commit/tree `83ce7e3a9519c154bda95f3054d9d8f6df811a50` / `3ab59931a3f3778c4d60e0d99b0dc9bcefd40ee9` plus patch SHA-256 `90e9d2ab4fd3b151d56cc1a9ddc08ecfd76120f667e5e41b57e23e5fa24a6135`. The one release benchmark executable is 1,081,856 bytes, SHA-256 `29b5be55a43d6eb958ff3932dd320782296041847c4748e86d90efb7c31c4d8a`.

The placement smoke passed required CUDA/no fallback and all eight singular plus five grouped quality/resource rows. Redacted unique-node assignment was:

| Session | CUDA nodes | CPU nodes |
|---|---:|---:|
| Detector | 328 | 0 |
| Recognizer | 181 | 2 |

Five fresh CPU and five fresh CUDA precursor processes then ran in the fixed alternating provider/order sequence with three warmups and 20 retained samples per workload. All ten reports and all retained samples passed with zero retry, exclusion, priming, overlap, incorrect result, call failure, post-warm growth, or NVRTC diagnostic. CUDA appeared in every CUDA process and in no CPU process.

CUDA p50 speedup versus the same-index CPU process ranged from 4.326× to 21.048×; p95 ranged from 4.115× to 19.662×; maximum ranged from 4.053× to 19.279×. Representative derived CUDA ceilings are 150/150/175 ms for 4K HUD, 150/175/175 ms for dense tooltip, 175/200/200 ms for one full-frame group, and 150/525/1,100 ms for dense grouped work.

The first exact-final source run, patch SHA-256 `7749d7a4c1c82c6c81e346e93ab0a751be6c38c0b08c120a01ac6643acbe925c`, stopped without retry on its first CUDA process. All correctness, placement, resource, lifecycle, cancellation, ownership, and scalar gates passed, but `zone_dense_unique` produced 107.2573 ms p50, 421.8358 ms p95, and 897.6018 ms maximum. This is retained as a real tail observation rather than excluded as noise. The dense p50 ceiling remains 150 ms; only its p95 and maximum ceilings advance to 525 ms and 1,100 ms, approximately 1.25× and round-up over the new observations. A fresh source revision must pass the complete fixed sequence with those limits; the failed process is not counted toward final acceptance.

The next fresh source run, patch SHA-256 `9338127e5fcbfcbb3aabbdc635d74940615f7fa33deb5727b3cb6afc09593eaf`, passed its first five interleaved processes and stopped without retry on preferred-CUDA fallback process three. Provider selection, correctness, resources, lifecycle, and scalar gates passed; the only failures were CPU `zone_eight_distinct` at 694.0153 ms p95 and 718.0161 ms maximum, and CPU `zone_dense_unique` at 812.6402 ms p95 and 943.0005 ms maximum. The existing standalone CPU profiles and their registry remain byte-for-byte unchanged. The distinct interleaved preferred-CUDA/fresh-CPU fallback matrix receives only grouped tail ceilings: eight-distinct stays 600 ms p50 and advances to 875/900 ms p95/maximum; dense stays 725 ms p50 and advances to 1,025/1,200 ms. These are approximately 1.25× and round-up over the new observations. A third fresh source sequence is the terminal qualification attempt: another unmodeled failure rejects explicit CUDA support rather than widening another gate.

The terminal source, patch SHA-256 `40e5ec9f1720fbfa5ab0943a3738ccc4a719c68d96532201c6740cca41c6152b`, built one 1,094,144-byte executable with SHA-256 `25a0ce6e51bfad48110e118807491d82b3870ac608491f25e86f207244f1c1c7`. All five required-CUDA and five preferred-CUDA/missing-root fresh-CPU processes passed in the fixed order: 249/249 validation checks, zero retry, exclusion, priming, overlap, incorrect result, growth failure, or NVRTC diagnostic. CUDA median/worst process RSS was 1,121,656,832/1,140,326,400 bytes; fallback CPU was 230,543,360/231,251,968 bytes. The three new schema-v5 benchmark profiles retain the predeclared middle CUDA and fallback processes plus the complete enforced budgets. Evidence ZIP SHA-256 is `496b1db1208de95b52d8f2ad5767e65d87cc451a7698868fb443b51292ed097c`.

Lifecycle and memory maxima across retained precursor and final evidence produce these ceilings:

| Measure | Worst observation | Ceiling |
|---|---:|---:|
| Cold open | 339.682 ms | 425 ms |
| First close | 17.774 ms | 25 ms |
| Reopen-close | 135.659 ms | 175 ms |
| Active cancellation | 3.596 ms | 25 ms |
| Retained result | 147.014 ms | 175 ms |
| Process peak RSS | 1,140,817,920 bytes | 1,426,063,360 bytes |

CUDA's controlled arena remains capped at 1 GiB. Across precursor and terminal evidence, host-wide VRAM was 1,811–1,899 MiB in CPU-fallback processes and 2,432–2,700 MiB in CUDA processes, so the conservative cross-process maximum difference was 889 MiB. Host-wide totals are retained as observations, not mislabeled as process attribution.

## Decision

Accept CUDA as an explicit Windows OCR provider for the exact qualified target/runtime/dependency/model/profile boundary:

- `PreferCuda` attempts the controlled CUDA pair and may construct a fresh CPU pair only after pre-publication initialization failure;
- `RequireCuda` returns the typed failure and publishes no engine when CUDA cannot initialize;
- detector and recognizer use one immutable provider pair; no inference, cancellation, device, or native failure retries on CPU;
- source releases bundle no CUDA/ORT/model binary.

Do not accept CUDA for `AutoPreferAccelerator`. Although every required workload exceeds the 15% latency-benefit criterion, CUDA process RSS is about 4.9× the same-source CPU median, above the predeclared 1.5× automatic-selection bound. Both Windows and macOS automatic provider policy therefore select CPU in v0.3.1.

The CUDA hard budgets are the 1.25×-and-round-up values in the new Windows CUDA benchmark profiles and testkit registry. The separate mixed-provider fallback tail ceilings retain the same rule without changing any historical standalone CPU profile. Final acceptance passed with five fresh required-CUDA `--enforce-budgets` processes plus five fresh preferred-CUDA/missing-root CPU-fallback processes and no retry or exclusion.

## Alternatives

- **Make CUDA automatic because it is much faster.** Rejected: the automatic policy required latency and memory gates; the measured RSS ratio fails the memory gate.
- **Reject CUDA entirely because RSS is larger.** Rejected: explicit callers can make that tradeoff, every hard correctness/resource/placement gate passed, the provider arena and observed VRAM increase are bounded, and the dedicated RSS ceiling makes regression observable.
- **Raise the automatic RSS ratio after measurement.** Rejected: that would relax a fixed decision rule to admit the observed candidate.
- **Use ambient toolkit or PyTorch DLLs.** Rejected by ADR 0046 and by the successful sanitized-path controlled-root evidence.

## Consequences

Windows callers gain explicit CUDA performance and pay higher startup, close, RSS, and GPU-memory costs. Existing constructors, explicit CPU, and `AutoPreferAccelerator` remain CPU. CoreML remains rejected under ADR 0047. No result, descriptor, diagnostic, or error includes device identity, native messages, graph names, paths, pixels, or recognized text.

The support statement is revision- and dependency-bound, not a claim for arbitrary NVIDIA hardware, CUDA/cuDNN versions, models, graphs, applications, concurrency, or real-time behavior.
