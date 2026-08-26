# ADR 0049: Bound CUDA OCR output and retain the controlled provider configuration

- **Status:** Accepted
- **Date:** 2026-08-26
- **Resolves gate:** Successor Windows CUDA OCR row under `G-013`
- **Supersedes:** ADR 0048 only for the current pre-release source, measurements, and `zone_empty_4k` maximum; ADR 0048 remains immutable historical evidence

## Context

ADR 0048 accepted explicit Windows CUDA but its terminal executable preceded later provider hardening in the merged `v0.3.1` source. The accepted profile also allowed a six-item 4096-pixel recognizer submission whose estimated native output exceeded the intended bounded working set. This Change requalified the exact merged provider behavior and tested three narrow remediation axes without changing public provider policy.

The baseline was commit `4b70c0a7c2db3be6b85b68aada080aeed91d16be`, tree `fe69d6e575f9f77931da16462d9aa74fe7b84e8f`. The accepted product/apparatus patch is 66,848 bytes, SHA-256 `703c5120275c58b1d3ac7af917e03f6e58ae05a9f138e79b05658d83a26bee1b`. A final 1,610-byte successor patch, SHA-256 `66b03661105a2d629821f52a9355001866ad9b003662fca1d745c11d5ffd9120`, changes only one benchmark tail ceiling and one benchmark-only path-redacting assertion. The complete base-to-successor patch is 67,773 bytes, SHA-256 `50cf0c238e70a9b9a1151f87cfd15984fb8c82ac66ffde79fc13a0ad1207ccfb`; its derived source tree is `896e037d962610c4abd7a4a7b143d1ae9c90f549`.

## Decision

CUDA recognizer submissions have an internal 128 MiB estimated and extracted output limit. CPU and CoreML retain the existing 256 MiB limit, and public `max_output_bytes()` remains the conservative 256 MiB backend fact. The planner forms deterministic maximal batches of at most six in stable width order, checks cancellation before each batch, restores detector order, and publishes no partial result. Ordinary accepted widths still use six-item submissions; six 4096-pixel candidates split into exact batches `[3, 3]`.

Retain the original compile-time CUDA provider configuration: device 0, the 1 GiB arena cap, default arena extension, exhaustive convolution search, and maximum convolution workspace. Add no runtime tuning surface. Retain complete validation and all 16 effective eager preloads from the 18-DLL controlled projection; `onnxruntime_providers_cuda.dll` remains deferred and `onnxruntime.dll` remains the controlled runtime. Add no probing, progressive retry, ambient search, `PATH` mutation, or unloading.

Keep explicit CUDA support, initialization-only preferred fallback, and required-provider failure behavior from ADR 0048. `AutoPreferAccelerator` remains CPU on both release targets. CoreML remains release-rejected under ADR 0047. Rust public APIs, C ABI 1.5, the C++ wrapper, provider descriptors, diagnostics, runtime profile identity, model/profile identity, and package contents do not change.

The successor Windows required-CUDA `zone_empty_4k` budgets are 25/25/50 ms for p50/p95/maximum. The predecessor B1a final process observed 27.0417 ms maximum with zero candidates and recognizer runs. Applying the established 1.25-times rule and rounding upward to the next 25 ms gives 50 ms; every other numeric ceiling remains unchanged.

## Evidence

Five baseline-CUDA, five adaptive-CUDA, and five adaptive-source CPU processes passed the fixed direct-Python sequence. Maximum workload-median adaptive `B/A` was `1.003846588980654`, geometric mean was `0.9723242027853826`, and minimum workload-median CPU-over-adaptive speedup was `4.288338523074435`. Median/worst adaptive CUDA RSS was 1,121,259,520/1,121,615,872 bytes. Adaptive batching is accepted as a memory-safety bound, not as an ordinary-workload RSS reduction claim.

`ArenaExtendStrategy::SameAsRequested` passed performance but reduced median RSS by only 0.016097%, worst RSS by 0.105564%, and host-wide VRAM by 28 MiB; it failed the improvement threshold. Disabling maximum convolution workspace stopped on a real CPU-reference absolute latency failure before a complete comparison. Both candidates were removed.

Five accepted-source memory-stage processes measured provider-preparation current-working-set deltas of 211,079,168..211,476,480 bytes, median 211,193,856 bytes, or 18.8354% of accepted CUDA median RSS. The threshold opened loader investigation. A sealed 18-node PE graph found 85 normal, one delay, and 12 controlled edges with no external imports, but the static provider closure omitted the revision-bound runtime-required NVRTC and NVRTC-builtins DLLs. Static imports therefore cannot prove any strict preload subset sound; the loader remains unchanged. The graph artifact SHA-256 is `933df34182448cbe45c40ecca0306c714aa803fd6557a6c0316a0079ebe8505a`.

The final successor built one 1,150,976-byte optimized executable, SHA-256 `8ad2f35de4c66d6c427deb11eeb1137a709a042b690b9064d380378ea4893535`. Five required-CUDA and five preferred-CUDA/missing-root fallback processes ran in exact order with three warmups and 20 retained samples for all eight singular and five grouped workloads. All ten passed with zero retry, exclusion, extra priming, overlap, incorrect result, call failure, post-warm growth, or unexpected stderr diagnostic.

Minimum workload-median p95 CPU-over-CUDA speedup was 3.995×; the minimum individual paired p95 ratio was 3.680×. CUDA median/minimum/maximum RSS was 1,119,711,232/1,118,826,496/1,120,923,648 bytes; fallback was 229,404,672/228,700,160/230,117,376 bytes. The median RSS ratio remains 4.881×, above the fixed 1.5× automatic-selection rule. Host-wide CUDA-minus-fallback VRAM maxima were 663–671 MiB, retained as temporal host observations rather than process attribution.

The extreme probe passed six maximum-width candidates as `[3, 3]`, two native runs, and maximum estimated output 114,954,240 bytes under the 134,217,728-byte limit. Successful native extraction enforced the same limit. Placement reproduced ADR 0048 exactly: detector 328 CUDA / 0 CPU nodes and recognizer 181 CUDA / 2 CPU nodes. The first placement launch is retained as a pre-runtime canonical-path apparatus failure; one authorized canonical-spelling replacement passed without changing source, executable, directory contents, arguments, or product environment.

The sealed final qualification archive is 57,591 bytes, SHA-256 `1f2d97418613126ec0450694554d939c1d890e9c5bb831f84bdce7a1427b9800`. Runtime SHA-256 is `5458c46e26efe64d7b2f960ba6aff97209b454a007af0f93d682ac2570f7541d`; fixture/model/vocabulary identities remain the exact ADR 0048 values, including fixture manifest `a289edb167d45f11f4269cef22ff37d93d2cbe1150201afb9bb3f58439375c4b`, detector `d2a7720d45a54257208b1e13e36a8479894cb74155a5efe29462512d42f49da9`, recognizer `6f327246b50388f3c176ae304bd95767ea6dc0c9ae92153ef8cbe210b3c14884`, and vocabulary `f7aa897ca828a4c7c9e2739c30f9161a33306d532f020bcdb91dcfb664a5507e`.

## Alternatives

- **Use `SameAsRequested` arena extension.** Rejected: it delivered no threshold-sized memory or VRAM improvement.
- **Disable maximum cuDNN workspace.** Rejected: the fixed comparison stopped on a real absolute performance failure and supplied no repeatable memory benefit.
- **Preload only the static PE closure plus NVRTC.** Rejected: known dynamic requirements already prove the static graph incomplete; excluding the remaining controlled DLLs would guess.
- **Make CUDA automatic because it remains faster.** Rejected: median process RSS is 4.881× fallback CPU, above the unchanged 1.5× gate.
- **Keep the inherited 25 ms empty-zone maximum.** Rejected by the retained 27.0417 ms target-native observation; the 50 ms successor follows the existing formula rather than discarding the sample.

## Consequences

Integrators do nothing differently. Explicit Windows CUDA still requires the exact version-matched flat canonical runtime/provider root and caller-supplied model assets; source releases bundle none of them. The internal output bound can increase native recognizer call count only for sufficiently wide batches. Any different recognizer graph, vocabulary class count, width/time-step relation, dependency projection, provider configuration, target, or automatic-selection rule requires new revision-bound qualification.

The loader's measured preparation delta remains temporal attribution. This decision does not claim that preloaded pages exclusively own 211 MiB, that close unloads process-global libraries, or that adaptive batching reduces ordinary steady-state RSS.

## Verification

- Checked width, output arithmetic, adaptive batching, cancellation, ordering, atomic publication, observer drain, and schema/redaction tests live in `mado-pilot-backend-onnx`.
- Current benchmark profiles are the three `phase-3-1-cuda-remediated-*` Windows TOML files; historical ADR 0048 profiles remain byte-identical.
- Testkit drift tests bind all hard predicates and current latency/global ceilings to executable constants.
- `rasen/changes/phase-3-1-onnx-ocr-cuda-performance-and-memory-remediation/evidence/` retains candidate failures, memory attribution, PE closure, direct-runner review, independent reviews, the predecessor C1 stop, apparatus corrections, and the sealed final archive.
- Roll back any future provider/loader/tuning change that fails source identity, provider, quality, output, latency, memory, placement, lifecycle, stderr, or no-retry gates; never convert a stopped run into a passing sample.
