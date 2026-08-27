# ADR 0050: Select exact RGBA change detection for compatible mapped regions

- **Status:** Proposed pending identical Windows and Apple Silicon hosted reports and independent review
- **Date:** 2026-08-27
- **Gate:** `G-005`
- **Direction / Slice:** `version-one-delivery` / `phase-4-bounded-template-watch-query`
- **Depends on:** released `v0.3.1` and protected `dev/0.4.0` baseline `dfee89e6542b432324b33395674a973e0e8f136b`

## Context

A later bounded template watcher needs a default rule for deciding whether routine visual analysis may be skipped. That decision cannot infer template presence or stability. It must first prove that a mandatory change is never skipped on a frozen recorded-sequence oracle and must fail safe across stream, epoch, ROI-affecting geometry, transform, descriptor, and frame-order boundaries.

The pre-observation contract is `g-005-evaluation-contract-v1`, SHA-256 `953a6faa3753ae94b1e136b691c5f94875913faae9cbbc6ac277c40fe5fdd641`. It fixed the manifest and expected-row schemas, seven candidate/threshold rows, selection order, metrics, authority limits, aggregate report, privacy boundary, and no-post-observation-rewrite rule before candidate execution. The frozen fixture set is repository-owned Apache-2.0 synthetic RGBA8 data with sixteen frames, seven sequences, and nine adjacent transitions.

Exact frozen identities are:

| Component | SHA-256 |
|---|---|
| fixture manifest | `dea51dc9862373f636870c3593f590fb65cc1489fd2f11a4cbb5842836fa532a` |
| expected rows | `2c082e24628b64fdc23706226311eb59aab2d61351adb3f86aa42f1c2e6648a1` |
| complete fixture checksum listing | `50b14842a0dca9e187166757df6b82ea7b3f2dc21b19440b60ce0b7d25d94943` |
| formatted qualification evaluator source | `9f3f684cd8f418a97c9cbad74165936391934787a396ad29846e427f064a8631` |
| canonical candidate plan | `4b84ee426177f3bcd97e77918f73526629067766b15803aecca291ab53ff037c` |
| canonical formatted report | `12cf52aab777bcfccf75748506a856a0ea4eb6e1435be63b783f2a85353732cf` |

The first execution ran all named loader, oracle, evaluator, discontinuity, privacy, and byte-stability contracts without changing the oracle after observation. It completed with 11 passed and 0 failed.

The first observation remains preserved with evaluator source
`d82314c27f72645fe2a6d42ae50191f7a142e3c5e50970021dd3702968aadef1`
and its complete report in the Change evidence directory. Workspace formatting
then changed only evaluator source layout, not the contract, manifest, expected
rows, candidate plan, decisions, counters, or selected policy. The formatted
source was rerun as a distinct applicability successor and produced the same
decision/counter table; the current canonical report binds the new source digest
instead of relabeling the historical first report.

## Decision

Select `exact-rgba-v1` as the closed default policy for already mapped RGBA8 regions. `analysis-always-v1` remains the supported explicit fail-safe policy. Evaluation-only changed-pixel thresholds and fixed-grid sampling are private and are not runtime options.

The default compares rows without allocation and stops at the first differing RGBA8 pixel. It returns unchanged only when the current mapping is strictly newer and stream id, epoch, geometry revision, effective mapped region, complete descriptor, and transform snapshot are compatible. Unsupported formats, reversed or repeated identity, byte/stride inconsistency, geometry or stream discontinuity, and any checked-arithmetic failure require analysis. Row padding is not pixel content and is not compared.

An unchanged result authorizes only skipping routine visual analysis for that compatible transition. It does not confirm template presence, advance consecutive-observation stability, create duration stability, satisfy a query, mutate a previous result, cross incompatible identity/geometry, or authorize input.

## Candidate result

The predeclared comparison first rejects any candidate with a false skip or typed failure. Among passing heuristics it minimizes admitted analyses, then uses declaration order. Analysis-always is selected only when no heuristic passes.

| Candidate | False skips | Admitted analyses | Skipped analyses | Inspected pixels | Decision |
|---|---:|---:|---:|---:|---|
| `exact-rgba-v1` | 0 | 6 | 3 | 72 | pass / selected |
| `changed-pixel-count-v1/min-2` | 1 | 5 | 4 | 85 | reject |
| `changed-pixel-count-v1/min-4` | 1 | 5 | 4 | 97 | reject |
| `changed-pixel-count-v1/min-8` | 4 | 2 | 7 | 112 | reject |
| `sampled-exact-v1/stride-2` | 1 | 5 | 4 | 28 | reject |
| `sampled-exact-v1/stride-4` | 4 | 2 | 7 | 7 | reject |
| `sampled-exact-v1/stride-8` | 4 | 2 | 7 | 7 | reject |

Every lower-admitted-work candidate skips the frozen one-pixel in-ROI mandatory transition; the stride-4 and stride-8 candidates also skip the appearance/disappearance rows. Their reduced work cannot waive those false negatives.

The canonical target-neutral aggregate is `docs/evidence/g-005/accepted-report.json`. Both hosted release-target jobs must reproduce it byte for byte from the tracked evaluator before this ADR becomes Accepted. A target mismatch or incomplete run remains a rejection; no retry or result borrowing is allowed.

## Alternatives

- **Analysis always.** Safe and remains the fallback, but it admits all nine transitions and discards the proven three compatible unchanged skips.
- **Changed-pixel thresholds.** Rejected because every predeclared threshold skips the mandatory one-pixel change.
- **Fixed-grid sampling.** Rejected because every stride skips at least one mandatory change; lower inspection count is irrelevant after a false skip.
- **Runtime caller thresholds or algorithm names.** Rejected because no arbitrary configuration has recorded-sequence qualification and it would create a new shallow public policy surface.
- **Hash mapped pixels.** Rejected because it adds work and content-derived state without improving the exact-byte decision, and ordinary diagnostics/evidence must not expose per-frame hashes.
- **Build the watcher first and measure end-to-end.** Rejected because queue, timing, backend, and stability effects would obscure whether the change detector itself produced a false skip.

## Consequences

`mado-pilot-vision` owns a closed `ChangeDetectionPolicy`, immutable descriptor facts, and a stateless `ChangeDetector`. The type is `Copy`, contains no worker, lock, queue, callback, or allocation, performs no environment lookup, and adds no facade, runtime, capture-callback, platform, C ABI, or C++ surface. The offline loader, candidates, report generation, and frozen fixtures remain testkit/evidence support and cannot become production dependencies.

This decision qualifies only the exact repository RGBA8 recorded-sequence boundary. It makes no native application, watcher latency, scheduler throughput, OpenCV, OCR, GPU, arbitrary-scene, arbitrary-format, or real-time claim. Numeric watcher budgets remain deferred to `phase-4-template-watch-query-qualification`.

The rollback rule is conservative: any future false skip, fixture/report/policy drift, unsupported descriptor requirement, or source change that invalidates this evidence restores `analysis-always-v1` until a new additive fixture set, evaluator contract, complete both-target run, and reviewed ADR accepts another default. Historical failed reports and this oracle remain unchanged.

## Privacy

The fixture bytes are generated repository data, not captured desktop content. The strict loader reports only closed error kinds and validated synthetic ids. The aggregate report contains transition/candidate ids, document/evaluator digests, decisions, counters, and authority booleans; it excludes pixels, per-frame hashes, fixture/local paths, desktop/window/process data, credentials, template identities, OCR/input text, and free-form decoder/backend/native payloads.

## Verification

- `cargo test --locked -p mado-pilot-testkit --test change_detection_evaluator` passed 11 tests on the first frozen execution and 12 after exact report/runtime descriptor parity was added without changing the evaluator source or oracle.
- `cargo test --locked -p mado-pilot-vision --test change_detection` passed six closed-policy, exact-pixel, sequence-gap, identity/geometry/ROI/format, numeric-code, default, and thread/worker-state contracts.
- Focused Clippy for `mado-pilot-vision` and `mado-pilot-testkit` passed with warnings denied.
- The protected Windows `x86_64-pc-windows-msvc` and macOS `aarch64-apple-darwin` jobs each have a named step that emits and byte-compares the report. Their exact topic revision, run ids, and outcomes are pending.
- Existing workspace dependency-direction, facade, C ABI 1.5 layout, frozen old-header, and C++ tests remain the no-surface-drift gates.
- `rasen/changes/phase-4-change-detection-default/evidence/first-observation.md` retains the preflight identities and complete rejected-candidate table.
