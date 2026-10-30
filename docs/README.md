# MadoPilot documentation

Find the current rule or procedure by task. Each topic has one canonical guide
below; other documents provide a brief summary and link here or to the specific
guide for details. Every page listed here is product documentation and works
without the `rasen/` planning repository.

## Understand the design and its contracts

| Page | Use it for |
|---|---|
| [Architecture](architecture.md) | Package inventory, dependency allowlist, release targets, [OS support policy](architecture.md#os-support-policy), public naming, and core, capture, input, asset, matching, OCR, watcher, and diagnostic contracts |
| [C ABI](c-abi.md) | Handle ownership, `struct_size` prefix negotiation, statuses, frozen ABI prefixes (`1.0`/`1.2`/`1.3`/`1.4`), and direct C/C++ consumption |
| [Third-party dependencies](third-party-dependencies.md) | License and source policy, OpenCV, ONNX Runtime, OCR model provenance, and deployment obligations |
| [Decision records](adr/README.md) | Short records of consequential architectural or technical choices and their rationale |

## Use a feature

| Page | Use it for |
|---|---|
| [Native template watching](native-template-watch.md) | Rust template presence queries over Windows Graphics Capture (WGC) and ScreenCaptureKit sessions, plus Qualification V2 lanes |
| [OCR text queries](ocr-text-watch.md) | Rust OCR text-presence queries, model selection, CPU execution options, and fixed replay/workload procedures |
| [OCR model catalog](ocr-model-catalog.md) | Implemented OCR model profiles, exact resource digests, catalog schema, and repeatable evaluation rules |
| [Capture pacing](capture-pacing.md) | Native capture interval configuration, caller-owned completion cooldown, and owned-workload verification |
| [Standalone Rust input workflow](rust-input-workflow.md) | Building and running the independent `examples/rust-input-workflow` consumer outside the workspace |

## Verify a change

| Page | Use it for |
|---|---|
| [Contributing](../CONTRIBUTING.md) | Native setup (`tools/setup-native.py`), branch/PR rules, and the 10-step local and CI verification sequence |
| [Validation gates](validation-gates.md) | Version-one `G-001`–`G-014` decision registry, blocking scope, due phase, and canonical evidence links |
| [Windows input](windows-input-verification.md) / [macOS input](macos-input-verification.md) | Input delivery routes, permission preflights, focus/DPI policies, fixture bundling, and opt-in native verification commands |
| [Windows capture contract tests](windows-capture-contract-tests.md) | WGC buffer-pool ownership tests, resource bounds, and native host requirements |
| [Windows OCR text-watch procedure](ocr-text-watch-windows.md) / [Apple OCR text-watch procedure](ocr-text-watch-macos.md) | Owned-fixture native OCR watcher procedures and fixed-cohort verification records |

## Measure and tune performance

| Page | Use it for |
|---|---|
| [Performance](performance.md) | Entry point for focused non-blocking measurements, outcome-driven metrics, TOML profile schema, and hard versus empirical budgets |
| [Measurement methods](performance/methods/) | Component, actual-consumer, and OCR-watch measurement definitions |
| [Tuning history](performance/tuning/README.md) | Concise records explaining why an empirical target or tuning parameter changed and its observed effect |
| [Benchmark profiles](benchmarks/) | Versioned, scoped workload configurations and recorded measurements |

## Read a historical record

| Location | Contents |
|---|---|
| [Evidence](evidence/) | Revision-bound measurements, receipts, and prototype reviews behind earlier decisions |
| [Release notes](releases/) | Scope, compatibility, and artifact boundaries for each tagged source release |
| [Benchmark profiles](benchmarks/) | Frozen profiles and historical measurement verdicts |

Historical records (`docs/evidence/`, `docs/releases/`, `docs/benchmarks/`, and
dated `docs/performance/tuning/2026-*.md`) remain byte-identical with their
original sources and verdicts. Records produced before commit
`646369fad6c922ac7b064367817337a18e572d8c` may cite retired decision records at
`docs/adr/NNNN-*.md` (browsable at that commit in Git history); those archived
records are historical context only and grant no current rule or release
authority.
