# G-004 default OCR profile evidence

This directory holds the fixed candidate/profile inputs and privacy-reviewed
qualification evidence for [`G-004`](../../validation-gates.md#g-004). The v5
matrix has matching release-target outcomes under the corrected evaluator
identity, and final independent review accepts the immutable profile decision.

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
| [`report-aarch64-apple-darwin-v5.json`](report-aarch64-apple-darwin-v5.json) | Current Apple v5 outcomes after independent patch review; bound by the Windows cross-target report |
| [`report-x86_64-pc-windows-msvc-v5.json`](report-x86_64-pc-windows-msvc-v5.json) | Accepted Windows v5 outcomes, Apple binding, cross-target reconciliation, and final evidence-review record |

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

## Current v5 qualification status

Independent patch review passed before any v5 run. Four fresh processes on each
release target then used evaluator SHA-256
`780f6cccf9679bc63aeaf6829b90769032246cbcfa29746b8012865294530249`,
candidate metadata SHA-256
`4b5aa66d3a7c390211219c794e35ee685701a9cd23c0f24f0d62047280199ff7`,
and RapidOCR code SHA-256
`753f75e387f6b6d128cc644b209fb76dde04cb735de06e411d643826f0a4a5aa`.
Every consumed fixture byte count/digest matched its manifest row, every outcome
was stable across ten measured passes, and raw output remained in private
ephemera.

### Apple Silicon v5

| Candidate | Model bytes | Median / p95 suite | Peak resident | Result |
|---|---:|---:|---:|---|
| `ppocrv4-det-v6-rec-small` | 25,979,900 | 2,622.916 / 2,632.851 ms | 986,087,424 B | Pass: all 42 regions |
| `ppocrv5-det-v6-rec-small` | 26,053,959 | 2,527.167 / 2,538.604 ms | 993,312,768 B | Reject: one `mission.png` exact-text row and one admitted unexpected region |
| `ppocrv6-det-tiny-rec-small` | 23,064,001 | 2,195.475 / 2,211.013 ms | 733,937,664 B | Reject: one dense-tooltip exact-text row and one admitted unexpected region |
| `ppocrv6-multilingual-small` | 31,163,977 | 3,445.286 / 3,589.109 ms | 999,129,088 B | Reject: one dense-tooltip exact-text row and one admitted unexpected region |

### Windows v5

| Candidate | Model bytes | Median / p95 suite | Peak resident | Result |
|---|---:|---:|---:|---|
| `ppocrv4-det-v6-rec-small` | 25,979,900 | 2,207.743 / 2,250.868 ms | 553,406,464 B | Pass: all 42 regions |
| `ppocrv5-det-v6-rec-small` | 26,053,959 | 3,000.408 / 3,021.903 ms | 547,180,544 B | Reject: one `mission.png` exact-text row and one admitted unexpected region |
| `ppocrv6-det-tiny-rec-small` | 23,064,001 | 1,609.076 / 1,616.285 ms | 492,269,568 B | Reject: one dense-tooltip exact-text row and one admitted unexpected region |
| `ppocrv6-multilingual-small` | 31,163,977 | 6,404.375 / 6,426.121 ms | 558,338,048 B | Reject: one dense-tooltip exact-text row and one admitted unexpected region |

The candidate-level outcomes and every deterministic image gate field match
across targets. Tool/code/model/fixture identities also match; timing, confidence
values, and typed resident measurements differ only in the fields the frozen plan
permits to vary. Final independent review verified those bindings, required the
validator to recompute every tracked v5 source hash, and accepted
`ppocrv4-det-v6-rec-small` as the only passing immutable profile. `G-004` is
resolved; implementation and support remain separate later Changes.

## Historical v4 cross-target outcome

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

### Historical v4 independent review outcome

The matching output does not accept a default. Independent review found that v4:

- does not hash the fixture bytes it opens at evaluation time;
- binds installed RapidOCR only by its reported package version, not executed
  code bytes;
- applies the unexpected-region threshold before expected-region matching;
- accepts a private raw-report path outside ignored Change ephemera; and
- does not retain the Windows peak-resident collection failure reason.

Correcting these findings changed the evaluator digest and kept `G-004` open
until every candidate was rerun on both release targets and the replacement
evidence passed independent review.

Evaluator v5 addresses all five findings without changing the frozen quality
oracle or candidate set. Its source hash and new report filenames are recorded in
`candidates.json`. Independent patch review passed before any v5 run; both target
reruns are complete. Final evidence review verified the consolidated reports and
validator, closed two review findings, and accepted the profile.

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
vocabulary inconsistency, stale v4 evidence identity, cross-target gate
divergence, privacy-schema drift, final-review or accepted-decision drift, v5
evaluator-source or frozen Apple/Windows outcome drift, or incomplete binding
between the two v5 target reports.

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

Provision the five v5 candidate component files beneath an explicit model root
using only the URLs in `candidates.json`:

```text
rapidocr-v3.9.2/ch_PP-OCRv4_det_mobile.onnx
rapidocr-v3.9.2/ch_PP-OCRv5_det_mobile.onnx
rapidocr-v3.9.2/PP-OCRv6_det_tiny.onnx
rapidocr-v3.9.2/PP-OCRv6_det_small.onnx
rapidocr-v3.9.2/PP-OCRv6_rec_small.onnx
```

Use the same venv interpreter for validation and evaluation. Independent v5 patch
review has passed. Run `validate.py` before creating any model session, then run
each candidate in a fresh process with a ten-minute external deadline. On POSIX:

```sh
.rasen/changes/phase-3-g004-default-ocr-profile/ephemera/venv/bin/python \
  docs/evidence/g-004/validate.py
.rasen/changes/phase-3-g004-default-ocr-profile/ephemera/venv/bin/python \
  docs/evidence/g-004/evaluate.py \
  --candidate <candidate-id> \
  --model-root <explicit-model-root> \
  --product-revision f3608424dde88f835f35653be8113f7a2009431b \
  --report <sanitized-report.json> \
  --raw-report .rasen/changes/phase-3-g004-default-ocr-profile/ephemera/raw-v5/<candidate-id>.json
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
  --raw-report .rasen/changes/phase-3-g004-default-ocr-profile/ephemera/raw-v5/<candidate-id>.json
```

The evaluator resolves both paths before inference. The raw path must be distinct
from the sanitized report and remain under
`.rasen/changes/phase-3-g004-default-ocr-profile/ephemera/`; other raw
destinations are rejected. The sanitized report may be returned through
`local_docs/` and is distilled into the reserved tracked v5 report only after
review.

Candidate IDs are:

```text
ppocrv4-det-v6-rec-small
ppocrv5-det-v6-rec-small
ppocrv6-det-tiny-rec-small
ppocrv6-multilingual-small
```

The historical Windows v4 run used Python 3.14.6 and the exact versions in
`tool-requirements.txt`, wrote all four sanitized reports outside tracked paths,
and returned them together. Its tracked report remains an immutable audit record
bound to the unchanged Apple v4 report. New v5 raw reports remain private because
they can contain unexpected recognized text; only approved identity, aggregate,
gate, privacy, consumed-fixture, code-digest, and typed resident fields may enter
the reserved tracked v5 reports.
