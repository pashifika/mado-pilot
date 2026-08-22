# G-004 default OCR profile evidence

This directory holds the fixed candidate/profile inputs and privacy-reviewed
qualification evidence for [`G-004`](../../validation-gates.md#g-004). The gate
remains open: Apple Silicon has one qualifying candidate, but the identical v3
fixture has not yet run on `x86_64-pc-windows-msvc`.

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
| [`report-aarch64-apple-darwin.json`](report-aarch64-apple-darwin.json) | Distilled v1–v3 Apple results; no unexpected recognized text or host path |
| `report-x86_64-pc-windows-msvc.json` | Required final target report; not present until the Windows run is returned |

The current fixture is
[`fixtures/ocr/g-004/fixture-manifest.json`](../../../fixtures/ocr/g-004/fixture-manifest.json),
profile `g-004-japanese-ui-v3`. Its five generated PNGs contain 42 exact regions.
`fixture-manifest-v1.json`, `fixture-manifest-v2.json`, and the original
`tooltip.png` preserve rejected evidence. `SHA256SUMS` covers only current v3
inputs, while `validate.py` separately verifies historical bytes against their
original manifests.

Pre-commit source review corrected one metadata label from RGB to BGR. The
evaluator already used OpenCV BGR arrays and RapidOCR performs no channel swap, so
execution and measurements are unchanged. The Apple report preserves the
at-run candidate-manifest digest and binds the clarification to the corrected
current digest rather than relabeling historical evidence.

## Apple Silicon outcome

All v3 rows used fresh processes, CPU execution only, one intra-op and one
inter-op thread, CPU arena disabled, two warm-up passes, and ten measured passes
over all five images. Every reported gate outcome was stable.

| Candidate | Model bytes | Median / p95 suite | Peak resident | Result |
|---|---:|---:|---:|---|
| `ppocrv4-det-v6-rec-small` | 25,979,900 | 2,651.749 / 2,682.488 ms | 1,000,587,264 B | Pass: all 42 regions |
| `ppocrv5-det-v6-rec-small` | 26,053,959 | 2,559.930 / 2,569.648 ms | 1,014,743,040 B | Reject: one `mission.png` exact-text row and one unexpected region |
| `ppocrv6-det-tiny-rec-small` | 23,064,001 | 2,215.362 / 2,223.786 ms | 759,742,464 B | Reject: one dense-tooltip exact-text row and one unexpected region |
| `ppocrv6-multilingual-small` | 31,163,977 | 3,425.281 / 3,435.967 ms | 997,621,760 B | Reject: one dense-tooltip exact-text row and one unexpected region |

Correctness is a hard gate, so the faster and lower-resident tiny detector cannot
win by performance. `ppocrv4-det-v6-rec-small` advances to Windows because it is
the only candidate that passed text, count, source-relative geometry, ordering,
confidence validity, stability, model identity, vocabulary, and deployment rows.

Peak resident bytes include Python 3.14, OpenCV Python, RapidOCR, ONNX Runtime,
and evaluation/report machinery. They are comparison evidence, not a Rust backend
budget or a support claim. The selected process reached about 954 MiB, so the
implementation Change must measure the native backend, bound model/session/
activation memory, and refuse unsafe loads rather than assuming this disposable
process shape is acceptable.

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

It fails on current fixture/hash/schema drift, historical v1/v2 mutation,
candidate/path/size/digest/vocabulary inconsistency, stale v3 source identity,
changed pass/fail rows, a promoted `G-004` status, or unapproved report payloads.

## Reproduce one target

Create an isolated environment outside product packages:

```sh
python3 -m venv .rasen/changes/phase-3-g004-default-ocr-profile/ephemera/venv
.rasen/changes/phase-3-g004-default-ocr-profile/ephemera/venv/bin/python \
  -m pip install -r docs/evidence/g-004/tool-requirements.txt
```

Provision the five v3 model files beneath an explicit model root using only the
URLs in `candidates.json`:

```text
rapidocr-v3.9.2/ch_PP-OCRv4_det_mobile.onnx
rapidocr-v3.9.2/ch_PP-OCRv5_det_mobile.onnx
rapidocr-v3.9.2/PP-OCRv6_det_tiny.onnx
rapidocr-v3.9.2/PP-OCRv6_det_small.onnx
rapidocr-v3.9.2/PP-OCRv6_rec_small.onnx
```

Run `validate.py` before creating any model session. Then run each v3 candidate in
a fresh process with a ten-minute external deadline:

```sh
python docs/evidence/g-004/evaluate.py \
  --candidate <candidate-id> \
  --model-root <explicit-model-root> \
  --product-revision f3608424dde88f835f35653be8113f7a2009431b \
  --report <sanitized-report.json> \
  --raw-report <ignored-raw-report.json>
```

Candidate IDs are:

```text
ppocrv4-det-v6-rec-small
ppocrv5-det-v6-rec-small
ppocrv6-det-tiny-rec-small
ppocrv6-multilingual-small
```

The Windows run uses Python 3.14.6 and the exact versions in
`tool-requirements.txt`, writes all four sanitized reports outside tracked paths,
and returns them together. Raw reports remain local because they can contain
unexpected recognized text. Only after the selected candidate passes and the
other rows reconcile against Apple may the final Windows report, ADR 0033, and
`G-004` status be accepted.
