# G-004 default OCR profile evidence

This directory holds the fixed candidate/profile inputs and privacy-reviewed
qualification evidence for [`G-004`](../../validation-gates.md#g-004). The v4
matrix has matching target outcomes, but independent review keeps the gate open
until the evaluator is hardened and rerun on both release targets.

No model or font bytes are committed. No MadoPilot release downloads a model or
claims an OCR backend from this evidence-only Change.

## Tracked artifacts

| File | Purpose |
|---|---|
| [`evaluation-plan.md`](evaluation-plan.md) | Frozen-before-run language, oracle, candidate, engine, privacy, and target rules; preserves rejected v1/v2 amendments |
| [`candidates.json`](candidates.json) | Exact RapidOCR tag/revision, model URLs, byte counts, SHA-256 values, shapes, vocabulary identities, preprocessing/decoder, and controlled paths |
| [`dependency-review.md`](dependency-review.md) | Model, runtime, fixture, font, redistribution, notice, and controlled-host obligations |
| [`tool-requirements.txt`](tool-requirements.txt) | Complete pinned evaluation-only Python environment |
| [`evaluate.py`](evaluate.py) | Bounded one-candidate runner; emits a sanitized report and a separate ignored raw report |
| [`validate.py`](validate.py) | Network-free CI check for current and historical fixture digests, candidate metadata, source identities, result invariants, and privacy fields |
| [`report-aarch64-apple-darwin.json`](report-aarch64-apple-darwin.json) | Distilled v1–v4 Apple results; no unexpected recognized text or host path |
| [`report-x86_64-pc-windows-msvc.json`](report-x86_64-pc-windows-msvc.json) | Distilled final Windows v4 outcomes and open cross-target decision; no unexpected recognized text or host path |

The current fixture is
[`fixtures/ocr/g-004/fixture-manifest.json`](../../../fixtures/ocr/g-004/fixture-manifest.json),
profile `g-004-japanese-ui-v3`. Its five generated PNGs contain 42 exact regions.
`fixture-manifest-v1.json`, `fixture-manifest-v2.json`, and the original
`tooltip.png` preserve rejected evidence. `SHA256SUMS` covers only current v3
inputs, while `validate.py` separately verifies historical bytes against their
original manifests.

Pre-commit source review corrected one metadata label from RGB to BGR. The
evaluator already used OpenCV BGR arrays and RapidOCR performs no channel swap, so
execution was unchanged. The Apple report preserves the v3 at-run
candidate-manifest digest; hardened v4 reruns use the corrected exact-shape
manifest and enforce its complete environment, session, and vocabulary identity.

## Cross-target outcome

### Apple Silicon

All final v4 rows used fresh processes, CPU execution only, one intra-op and one
inter-op thread, CPU arena disabled, two warm-up passes, and ten measured passes
over all five v3 fixture images. Before inference, each row also passed exact
Python/package, ONNX input/output/provider, and embedded-vocabulary checks. Every
reported gate outcome was stable.

| Candidate | Model bytes | Median / p95 suite | Peak resident | Result |
|---|---:|---:|---:|---|
| `ppocrv4-det-v6-rec-small` | 25,979,900 | 2,650.097 / 2,661.246 ms | 1,012,318,208 B | Pass: all 42 regions |
| `ppocrv5-det-v6-rec-small` | 26,053,959 | 2,570.691 / 2,591.098 ms | 1,021,952,000 B | Reject: one `mission.png` exact-text row and one unexpected region |
| `ppocrv6-det-tiny-rec-small` | 23,064,001 | 2,223.747 / 2,233.180 ms | 766,640,128 B | Reject: one dense-tooltip exact-text row and one unexpected region |
| `ppocrv6-multilingual-small` | 31,163,977 | 3,450.667 / 3,470.270 ms | 997,490,688 B | Reject: one dense-tooltip exact-text row and one unexpected region |

Correctness is a hard gate, so the faster and lower-resident tiny detector cannot
win by performance. `ppocrv4-det-v6-rec-small` is
the conditional candidate because it is the only candidate that passed text,
count, source-relative geometry, ordering, confidence validity, stability, model
identity, vocabulary, and deployment rows.

### Windows

The fresh Windows processes used the same declared source, models, fixture,
environment, CPU-only session profile, warm-up count, and measured-pass count.
All candidate-level outcomes and every deterministic image gate field match the
Apple v4 matrix exactly.

| Candidate | Model bytes | Median / p95 suite | Peak resident | Result |
|---|---:|---:|---:|---|
| `ppocrv4-det-v6-rec-small` | 25,979,900 | 2,311.958 / 2,504.966 ms | Unavailable | Pass: all 42 regions |
| `ppocrv5-det-v6-rec-small` | 26,053,959 | 3,102.009 / 3,117.815 ms | Unavailable | Reject: one `mission.png` exact-text row and one unexpected region |
| `ppocrv6-det-tiny-rec-small` | 23,064,001 | 1,669.276 / 1,690.721 ms | Unavailable | Reject: one dense-tooltip exact-text row and one unexpected region |
| `ppocrv6-multilingual-small` | 31,163,977 | 6,567.260 / 6,585.219 ms | Unavailable | Reject: one dense-tooltip exact-text row and one unexpected region |

Peak resident bytes include Python 3.14, OpenCV Python, RapidOCR, ONNX Runtime,
and evaluation/report machinery. The Windows-specific `GetProcessMemoryInfo`
collector returned no value and did not retain its native failure reason, so the
Windows report records `null` rather than inventing a measurement. The Apple
values are comparison evidence, not a Rust backend budget or support claim.

### Independent review outcome

The matching output does not accept a default. Independent review found that v4:

- does not hash the fixture bytes it opens at evaluation time;
- binds installed RapidOCR only by its reported package version, not executed
  code bytes;
- applies the unexpected-region threshold before expected-region matching;
- accepts a private raw-report path outside ignored Change ephemera; and
- does not retain the Windows peak-resident collection failure reason.

Correcting these findings changes the evaluator digest. `G-004` therefore remains
open until every candidate is rerun on both release targets and the replacement
evidence passes independent review.

## Why the real screenshots are not evidence payloads

Four screenshots in ignored `local_docs/game-screenshots/` exposed workload
classes the initial synthetic fixture missed: dense Japanese tooltips over dark
content and small labels across a high-resolution light menu. Redistribution
rights are not established, so no screenshot, crop, digest, title, or recognized
text is tracked. Repository-authored `tooltip-v3.png` and `mission.png` reproduce
only those scale, density, contrast, and layout classes.

## Network-free validation

This command needs only tracked files and Python's standard library; CI runs it in
the repository-policy job:

```sh
python3 docs/evidence/g-004/validate.py
```

It fails on current fixture/hash/schema drift, historical v1/v2 mutation, any
changed frozen v1–v4 Apple or final Windows outcome, candidate/path/size/digest/
vocabulary inconsistency, stale v4 declared source or tool/session identity,
cross-target gate divergence, privacy-schema drift, or a falsely resolved
decision.

## Reproduce one target

Create an isolated environment outside product packages. On POSIX:

```sh
python3 -m venv .rasen/changes/phase-3-g004-default-ocr-profile/ephemera/venv
.rasen/changes/phase-3-g004-default-ocr-profile/ephemera/venv/bin/python \
  -m pip install -r docs/evidence/g-004/tool-requirements.txt
```

On Windows:

```powershell
py -3.14 -m venv .rasen/changes/phase-3-g004-default-ocr-profile/ephemera/venv
.rasen/changes/phase-3-g004-default-ocr-profile/ephemera/venv/Scripts/python.exe `
  -m pip install -r docs/evidence/g-004/tool-requirements.txt
```

Provision the five v4 candidate component files beneath an explicit model root
using only the URLs in `candidates.json`:

```text
rapidocr-v3.9.2/ch_PP-OCRv4_det_mobile.onnx
rapidocr-v3.9.2/ch_PP-OCRv5_det_mobile.onnx
rapidocr-v3.9.2/PP-OCRv6_det_tiny.onnx
rapidocr-v3.9.2/PP-OCRv6_det_small.onnx
rapidocr-v3.9.2/PP-OCRv6_rec_small.onnx
```

Use the same venv interpreter for validation and evaluation. Run
`validate.py` before creating any model session, then run each v4 candidate in a
fresh process with a ten-minute external deadline. On POSIX:

```sh
.rasen/changes/phase-3-g004-default-ocr-profile/ephemera/venv/bin/python \
  docs/evidence/g-004/validate.py
.rasen/changes/phase-3-g004-default-ocr-profile/ephemera/venv/bin/python \
  docs/evidence/g-004/evaluate.py \
  --candidate <candidate-id> \
  --model-root <explicit-model-root> \
  --product-revision f3608424dde88f835f35653be8113f7a2009431b \
  --report <sanitized-report.json> \
  --raw-report <ignored-raw-report.json>
```

On Windows:

```powershell
.rasen/changes/phase-3-g004-default-ocr-profile/ephemera/venv/Scripts/python.exe `
  docs/evidence/g-004/validate.py
.rasen/changes/phase-3-g004-default-ocr-profile/ephemera/venv/Scripts/python.exe `
  docs/evidence/g-004/evaluate.py `
  --candidate <candidate-id> `
  --model-root <explicit-model-root> `
  --product-revision f3608424dde88f835f35653be8113f7a2009431b `
  --report <sanitized-report.json> `
  --raw-report <ignored-raw-report.json>
```

Both report paths must remain under
`.rasen/changes/phase-3-g004-default-ocr-profile/ephemera/`. V4 records that
operator rule but does not enforce it; the corrected evaluator must reject a raw
path outside ignored ephemera before writing or inference.

Candidate IDs are:

```text
ppocrv4-det-v6-rec-small
ppocrv5-det-v6-rec-small
ppocrv6-det-tiny-rec-small
ppocrv6-multilingual-small
```

The recorded Windows run used Python 3.14.6 and the exact versions in
`tool-requirements.txt`, wrote all four sanitized reports outside tracked paths,
and returned them together. Raw reports remain local because they can contain
unexpected recognized text. The tracked final report distills only approved
identity, aggregate, gate, and privacy fields, binds the unchanged Apple report
by SHA-256, and records the unresolved evidence gaps.
