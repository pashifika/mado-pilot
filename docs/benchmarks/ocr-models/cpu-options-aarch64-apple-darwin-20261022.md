# OCR CPU options: Apple Silicon comparison, 2026-10-22

Explicit eight-thread, nonspinning execution reduced observed p95 OCR latency for all three models on this corpus. It is an opt-in configuration, not a new default or a universal optimum. Native G-004 used more peak memory; model-specific recognition findings remain visible.

## Scope

- Apple M1 Pro, 10 physical/logical cores, 32 GiB; macOS 27.0.1 build 26A434; `aarch64-apple-darwin`.
- ONNX Runtime 1.29.0/API 17 CPU, OpenCV 4.14.0, Rust 1.97.1; optimized benchmark with `benchmark-instrumentation`. Bench/release stripping was disabled for this comparison.
- The same final executable and workspace source hashes cover both configurations. The build was an uncommitted worktree, not a clean qualification revision.
- Eleven repository-owned images, full-image ROI, singular and grouped calls; one fresh process per model/configuration. Each workload has 2 warmups and 20 measured samples: 2640 measured calls plus 264 warmups. Processes ran serially; no assistant-started build or test ran concurrently. Other host activity was not controlled.
- Baseline: 1 intra-op thread, spinning enabled. Compared option: 8 intra-op threads, spinning disabled. Both detector and recognizer use the selection; inter-op 1, sequential execution and memory patterns disabled remain fixed.
- p95 uses nearest rank. Timings describe the instrumented OCR invocation, not native capture or watcher-notification latency. Startup and RSS have separate scopes below. One host/process per configuration supplies observations, not population confidence intervals.

The [machine-readable report](cpu-options-aarch64-apple-darwin-20261022.json) contains exact model/resource identities, source/binary/runtime hashes, all 132 workload summaries, p50/p95 stage data, allocation observations, lifecycle outcomes and raw-record digests. Raw records and the runnable collector are retained under the Change's `evidence/model-cpu-{default,tuned}-final-20261022/` and `evidence/run-model-tuning-diagnostic.py`; the collector is a maintainer artifact, not an installed command.

## Model identities

- **G-004 native:** `g-004-rapidocr-ppocrv4-det-v6-rec-small-v1`.
- **bounded-v2:** `phase-3-1-rapidocr-ppocrv4-det-v6-rec-small-bounded-v2`.
- **PP-OCRv6 tiny:** `ppocrv6-tiny-det-small-rec-max736-v1`.

## Warm singular OCR p95 (ms)

Each pair compares the one-thread baseline with the explicit eight-thread/nonspinning option. Grouped-call statistics are retained in the JSON report.

| Fixture (pixels) | G-004: 1 thread | G-004: 8 threads | bounded: 1 thread | bounded: 8 threads | PPv6: 1 thread | PPv6: 8 threads |
|---|---:|---:|---:|---:|---:|---:|
| `g-004-japanese-ui-v3/hud` (960×540) | 470.1 | 164.0 | 471.6 | 163.9 | 292.9 | 94.4 |
| `g-004-japanese-ui-v3/menu` (720×480) | 431.5 | 148.7 | 430.3 | 149.7 | 302.0 | 95.3 |
| `g-004-japanese-ui-v3/status` (640×360) | 421.2 | 135.4 | 412.2 | 131.5 | 220.7 | 59.0 |
| `g-004-japanese-ui-v3/tooltip-v3` (1440×720) | 685.0 | 205.9 | 470.9 | 137.2 | 384.0 | 104.0 |
| `g-004-japanese-ui-v3/mission` (1440×720) | 594.9 | 197.8 | 437.2 | 141.5 | 350.4 | 107.0 |
| `compact-ui-v1/dense-tooltip` (640×360) | 573.9 | 192.8 | 569.4 | 196.0 | 293.2 | 97.0 |
| `compact-ui-v1/boundary` (384×192) | 353.5 | 135.6 | 206.4 | 82.8 | 102.3 | 40.4 |
| `compact-ui-v1/decoration` (512×256) | 357.1 | 127.9 | 209.6 | 84.2 | 111.1 | 43.1 |
| `compact-ui-v1/varied-sizes` (736×360) | 418.6 | 140.2 | 266.0 | 87.9 | 193.4 | 57.5 |
| `compact-ui-v1/blank` (320×160) | 266.4 | 100.4 | 119.7 | 49.3 | 9.1 | 5.2 |
| `compact-ui-v1/nonuniform-negative` (448×224) | 267.5 | 95.6 | 148.9 | 59.1 | 49.2 | 21.1 |

## Whole-process memory and backend startup

RSS is the peak of the entire fresh benchmark process across initialization, all fixtures/modes and lifecycle work—not HUD-only, tensor-only or a live application memory budget. Cold open is one backend-constructor observation per process; host file caches were not cleared.

| Model | RSS, 1 thread (MiB) | RSS, 8 threads (MiB) | Cold open, 1 thread (ms) | Cold open, 8 threads (ms) |
|---|---:|---:|---:|---:|
| G-004 native | 488.8 | 564.5 | 81.6 | 81.5 |
| bounded-v2 | 419.4 | 396.8 | 80.4 | 81.0 |
| PP-OCRv6 tiny | 253.4 | 244.4 | 68.4 | 67.0 |

Do not infer a general memory reduction from these single-process peaks. In particular, G-004 trades higher peak RSS for lower latency.

## Recognition and lifecycle outcomes

Per-workload recognition finding counts were identical before and after CPU tuning; this does not claim bit-identical floating-point outputs or accuracy outside the corpus. Counts below are findings across 20 repeated samples in each of two modes, not distinct failing images.

| Model | Observed recognition findings per configuration |
|---|---|
| G-004 native | No `quality()` findings on the 11 images. |
| bounded-v2 | `nonuniform-negative`: 40 `region_count` and 40 `unexpected_detection` findings. |
| PP-OCRv6 tiny | `status`: 40 `text_order`; `tooltip-v3`: 120 `text_order`; `nonuniform-negative`: 40 `region_count` findings. |

All 2640 measured calls returned `Ok`; vocabulary checks passed in all six processes. Cancellation, deadline, closed-backend refusal, repeated close and initialization cancellation produced their expected statuses. The diagnostic process deadline was 600 s, cleanup deadline 10 s and lifecycle-observation bound 1000 ms; these are not substitutes for ADR 0075's historical numerical ceilings.

## Choosing a configuration

- Native inference occupied approximately 96–97% of baseline HUD time; preprocessing occupied less than 1%. CPU execution options target that measured bottleneck instead of adding speculative preprocessing complexity. Native inference still accounts for approximately 88–91% after tuning.
- The earlier HUD exploration compared 1/2/4/8-thread configurations and selected spinning alternatives using separately pinned prototype binaries. Eight/nonspinning had the lowest tested HUD p95; the final same-binary comparison above covers 1/spinning versus 8/nonspinning over the full corpus.
- G-004 had no recognition findings in this fixed corpus but used the most peak memory. bounded-v2 reduced work on several images while retaining a negative-image recognition failure. PP-OCRv6 tiny was lower-latency/lower-memory here, with text/order and region-count findings that the application must assess.
- Choose from application images, ROI, confidence requirements, latency and available CPU/memory—not from an aggregate fastest-model badge. More workers can contend with the application; spinning changes worker wait behavior. Competing-application load and power consumption were not measured.
- Counts 3/5/6/7 remain supported but unmeasured. Windows, accelerators and native capture are unmeasured in this comparison. No current result inherits another platform's qualification.

Use [`OcrProfileConfig::with_cpu_options`](../../ocr-text-watch.md#cpu-execution-options) to opt in. The default remains 1 thread with spinning enabled. Model choice is not filtered by these findings; resource bounds, identity validation, cancellation and cleanup remain required. Historical failed/unexecuted qualification records are unchanged.
