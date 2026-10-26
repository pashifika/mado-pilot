# MadoPilot documentation

Find the current rule by what you need to do. Every page linked here is product
documentation and applies to this checkout; none requires the planning repository.

## Understand the design and its contracts

| Page | Use it for |
|---|---|
| [Architecture](architecture.md) | Packages, dependency directions, release targets and OS support, public naming, and the current capture, input, asset, matching, OCR, watcher and diagnostic contracts |
| [C ABI](c-abi.md) | Handle ownership, structure prefixes, statuses, released ABI prefixes and direct C/C++ consumption |
| [Third-party dependencies](third-party-dependencies.md) | License and source policy, OpenCV, ONNX Runtime, model resources and deployment obligations |
| [Decision records](adr/README.md) | Short records of consequential architectural or technical choices and why they were made |

## Use a feature

| Page | Use it for |
|---|---|
| [Native template watching](native-template-watch.md) | Rust template queries over WGC and ScreenCaptureKit sessions |
| [OCR text queries](ocr-text-watch.md) | Rust OCR text-presence queries, model selection and CPU options |
| [OCR model catalog](ocr-model-catalog.md) | Implemented OCR profiles, exact resources and scoped test results |
| [Capture pacing](capture-pacing.md) | Native capture intervals and caller-owned completion cooldown |
| [Standalone Rust input workflow](rust-input-workflow.md) | Building an independent Rust consumer outside this checkout |

## Verify a change

| Page | Use it for |
|---|---|
| [Contributing](../CONTRIBUTING.md) | Setup, branch policy and the required local and CI verification sequence |
| [Validation gates](validation-gates.md) | Version-one decisions, what blocks what, and which checks are mandatory |
| [Windows input](windows-input-verification.md) / [macOS input](macos-input-verification.md) | Input routes, permissions, focus and native verification commands |
| [Windows capture contract tests](windows-capture-contract-tests.md) | WGC ownership tests and their native host requirements |
| [Windows OCR text-watch procedure](ocr-text-watch-windows.md) / [Apple OCR text-watch procedure](ocr-text-watch-macos.md) | Owned-fixture native OCR watcher procedures |

## Measure and tune performance

| Page | Use it for |
|---|---|
| [Performance](performance.md) | Entry point: focused measurement, profile format, and the difference between observations, explicit claims and mandatory safety checks |
| [Measurement methods](performance/methods/) | Component, actual-consumer and OCR-watch measurement definitions |
| [Tuning history](performance/tuning/README.md) | Short records of why a tuning choice or empirical target changed and what it achieved |
| [Benchmark profiles](benchmarks/) | Versioned, scoped workload configurations and targets |

Benchmarks support investigation and tuning. An empirical timing or memory target
is not an enforced runtime limit, and changing one needs a tuning record, not a
decision record. Correctness, resource limits, ownership, privacy, cancellation
and compatibility remain mandatory through their own contracts and tests.

## Read a historical record

| Location | Contents |
|---|---|
| [Evidence](evidence/) | Revision-bound measurements, receipts and reviews behind earlier decisions |
| [Release notes](releases/) | Scope, compatibility and artifacts of each source release |
| [Benchmark profiles](benchmarks/) | Earlier profiles stay byte-identical with their original verdicts |

Historical records keep their original bytes, sources and verdicts; a later
change never regrades them. Records produced before source
`646369fad6c922ac7b064367817337a18e572d8c` may cite decision records by their
former path `docs/adr/NNNN-*.md`. Those numbered records were retired from the
product tree; read them at that source, for example
`https://github.com/pashifika/mado-pilot/tree/646369fad6c922ac7b064367817337a18e572d8c/docs/adr`.
They are historical context only: an old `Accepted` or `Proposed` status grants
no current rule, admission or release authority. The current rule is the one
stated on the pages above.
