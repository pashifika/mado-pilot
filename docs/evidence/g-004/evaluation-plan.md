# G-004 default OCR profile evaluation plan

This plan was frozen at `2026-08-22T13:43:23Z`, before any candidate inference.
Changing the fixture bytes, expected text, candidate set, preprocessing, decoder,
or hard gate after observing a result invalidates the affected rows and requires a
new fixture-profile identifier and complete rerun on both release targets.

## Oracle amendment frozen before v2

The three v1 candidates ran on Apple Silicon after the original freeze. The
PP-OCRv4 and PP-OCRv5 recognizers failed unchanged exact-text gates.
`ppocrv6-multilingual-small` returned exact NFC text, count, order, and accepted
geometry for all 22 regions, but its exact `[OK]` result had confidence `0.76011`.
The PP-OCRv4 and PP-OCRv5 recognizers reported `0.99624` and `0.98939` for the
same exact region. Every value was stable across ten passes.

This disproves the v1 assumption that a universal `0.80` greedy-CTC mean is a
cross-model quality measure. The values are local to different vocabularies and
output distributions and are not calibrated probabilities. Exact expected text
is the quality oracle; confidence remains observable evidence and a future
caller threshold, not a substitute for ground truth.

V1 remains immutable in `fixture-manifest-v1.json` and the target report. Before
any amended candidate run, four local game screenshots were reviewed at
`local_docs/game-screenshots/`. Their artwork and pixels are not committed or
retained in evidence because redistribution rights are not established. They
showed two missing workload classes: dense 24–32 pixel Japanese tooltip lines on
mixed dark content and small labels distributed across a high-resolution light
menu.

V2 was therefore frozen at `2026-08-22T14:02:43Z`. It adds repository-authored
`tooltip.png` and `mission.png` fixtures that reproduce those contrast, density,
scale, and distribution classes without copying source pixels, artwork, names,
or character imagery. The original three PNGs and every original expected row
remain byte-for-byte unchanged. V2 also replaces only the unsupported confidence
rule: each score must be finite, within `[0.0, 1.0]`, and deterministic across
the ten passes. Normalization, geometry, ordering, unexpected-region threshold,
provider, and preprocessing/decoder rules are unchanged.

V2 also separates the detector and recognizer axes. The v1 Apple result proved
only the PP-OCRv6 small recognizer met every exact-text row, while all three DB
detectors found the full fixture. V2 therefore compares that recognizer with the
PP-OCRv4 mobile, PP-OCRv5 mobile, PP-OCRv6 tiny, and PP-OCRv6 small detectors.
All four amended candidates run from fresh processes on both targets. The two
recognizers already rejected by a mandatory Apple row are not viable and are
not run on Windows. This is a new complete qualification matrix, not a rewrite
of v1 output.

## Ordering correction frozen before v3

The first v2 Apple candidate passed exact text, count, geometry, confidence
validity, and stability for all 42 regions. It failed only the new tooltip's
ordering row. The v2 manifest enumerated the entire left card before the right
detail panel, while RapidOCR v3.9.2 orders detector boxes by stable top-edge Y,
groups adjacent top edges less than 10 source pixels apart, then orders each row
by X. The manifest order described file-authoring order rather than the selected
profile's observable ordering contract.

V2 remains immutable in `fixture-manifest-v2.json`; its `tooltip.png` bytes and
failed report retain their original digests. V3 was frozen at
`2026-08-22T14:07:53Z`, before any v3 run. It aligns the card-type label with its
intended detail row and records the tooltip regions in the pinned detector's
explicit row order as `tooltip-v3.png`. All other PNG bytes, expected text,
normalization, geometry tolerances, confidence interpretation, candidates, and
engine settings are unchanged. Every v3 candidate runs from a fresh process on
both targets.

## Pixel-format metadata clarification

Complete pre-commit review found that the candidate record called the input RGB,
while the evaluator used `cv2.imread(..., cv2.IMREAD_COLOR)` and therefore passed
BGR arrays. RapidOCR v3.9.2 detector preprocessing normalizes and transposes that
array without a channel swap, and the recognizer consumes crops from the same BGR
array. The executed profile was BGR on every retained run.

The v3 Apple report keeps candidate-manifest SHA-256
`c77239560b4f93930b19b30cb708c6736151fef3eb9a6fd0bc846e0ab28aa85b`
as its at-run source identity. `candidates.json` now describes the observed BGR
execution and the report binds that clarification to the current candidate-file
digest. No model, evaluator, fixture, preprocessing value, output, timing, or
memory measurement changed, so no run is relabeled or rerun.

## Evaluator hardening frozen before v4

Independent review found that the v3 evaluator recorded but did not enforce every
version in `tool-requirements.txt`, and verified model bytes/digests without
comparing the loaded ONNX names, types, shapes, provider, or embedded vocabulary
to `candidates.json`. V3 results remain historical observations, but those omitted
hard-gate checks prevent them from being final qualification evidence.

V4 was frozen at `2026-08-22T14:55:37Z`, before any hardened run. The fixture,
candidate model bytes, BGR preprocessing, decoder, oracle, and sample policy are
unchanged. Before session creation it requires Python and every pinned package
version to match. After digest verification and session creation but before
warm-up, it requires CPU-only provider binding, exact ONNX input/output
name/type/shape records, and the exact embedded vocabulary count/digest and
fixture coverage. All four candidates rerun in fresh Apple processes. Windows
qualification must use the same v4 evaluator and source identities.

## Revision and ownership

| Field | Frozen value |
|---|---|
| Released baseline | `f3608424dde88f835f35653be8113f7a2009431b` |
| Product base | `f3608424dde88f835f35653be8113f7a2009431b` (`origin/dev/0.3.0`) |
| Product branch | `chore/phase-3-g004-default-ocr-profile` |
| Direction source | Workstream `version-one-delivery`, Slice `phase-3-onnx-ocr-v0-3-0`, plan Change 1 |
| Fixture profile | `g-004-japanese-ui-v3`; v1 and v2 retained as rejected historical evidence |
| Required targets | `aarch64-apple-darwin`, then `x86_64-pc-windows-msvc` |
| Evidence owner | `docs/evidence/g-004/` and `fixtures/ocr/g-004/` |
| Raw work owner | `.rasen/changes/phase-3-g004-default-ocr-profile/ephemera/`, never a product package or release artifact |

The product repository owns the generator, immutable PNGs, manifest, digests,
distilled target reports, dependency review, and eventual ADR. Font and model
bytes, the Python environment, raw candidate output, and rejected recognized text
remain local ephemera. The Windows operator returns the same sanitized report
shape produced on Apple Silicon; no host-local path enters tracked evidence.

## Language and fixture scope

Version 0.3.0 qualifies horizontal Japanese game UI text: hiragana, katakana,
common kanji represented by the fixture, basic Latin letters, ASCII digits, and
the declared punctuation `.,/%+-[]!>#`. Text is normalized to Unicode NFC and
leading or trailing Unicode whitespace is removed. Internal whitespace, case,
and width are preserved. There is no case folding, width folding, translation,
handwriting, document-layout recovery, or vertical-text support. The profile must
leave room for a later vertical-text profile, but no vertical result is accepted
here.

The repository-owned fixture is generated by
`fixtures/ocr/g-004/generate.py`. It renders only synthetic text and shapes with
Noto Sans JP at weight 400. The input font is pinned to Google Fonts revision
`ec626514f79f831f1ab848a82114a0ce7e2d6372`, SHA-256
`c2f3b4d463500a2ddcd3849cded1fceeb9fd6d1c32e6cbecd568453ba50fc68f`,
and OFL-1.1. The font is not committed; OFL-1.1 explicitly excludes documents
created with the font from the font-license requirement. The generated PNGs and
expected-output manifest are repository contributions under Apache-2.0.

`fixture-manifest.json` fixes each PNG digest, expected NFC UTF-8 region,
source-relative quadrilateral, and order. The PNGs are inputs, not screenshots of
an application or desktop.

## Candidate allowlist

Only these RapidOCR v3.9.2 CPU ONNX pairs are qualified under v2:

| Candidate | Detector | Recognizer | Reason retained before v2 run |
|---|---|---|---|
| `ppocrv4-det-v6-rec-small` | `ch_PP-OCRv4_det_mobile` | `multi_PP-OCRv6_rec_small` | Smallest already-screened detector paired with the only exact-text recognizer |
| `ppocrv5-det-v6-rec-small` | `ch_PP-OCRv5_det_mobile` | `multi_PP-OCRv6_rec_small` | Newer mobile detector comparison without changing recognition |
| `ppocrv6-det-tiny-rec-small` | `multi_PP-OCRv6_det_tiny` | `multi_PP-OCRv6_rec_small` | Smallest same-generation detector/recognizer profile |
| `ppocrv6-multilingual-small` | `multi_PP-OCRv6_det_small` | `multi_PP-OCRv6_rec_small` | Same-generation small quality baseline from v1 |

The v1 screening set was `ppocrv4-japan-mobile`,
`ppocrv5-multilingual-mobile`, and `ppocrv6-multilingual-small`. Its first two
rows remain rejected by exact text; the third is rerun under every v2 gate.

Every URL and digest comes from RapidOCR's tag-pinned `default_models.yaml` at
tag `v3.9.2`, commit `095232a4c94f7f0e6600ba5bba1177010ad696d4`.
Candidate model bytes come only from the manifest's tag-pinned ModelScope paths.
No filename, alternate mirror, or later manifest may substitute for a declared
digest. Orientation classification is disabled because the fixture and v0.3.0
scope are horizontal-only.

A candidate is viable only when its two model files total at most 64 MiB, both
SHA-256 values match before session creation, the recognition model embeds a
`character` vocabulary containing every fixture code point, and ONNX Runtime can
load both files with `CPUExecutionProvider` alone. Model bytes are controlled
host-provided inputs: MadoPilot will not bundle them, download them, search ambient
paths, or infer identity from a filename.

## Fixed inference profile

The evaluation uses RapidOCR 3.9.2 with ONNX Runtime 1.29.0. The exact Python
environment is recorded in `tool-requirements.txt`. Each candidate runs in a
fresh process with:

- detector: DB postprocessing; OpenCV-decoded BGR input stays in BGR order and
  becomes three planar float32 channels, with side length limited to 736 under
  `limit_type=min`, dimensions rounded to multiples of 32, per-channel mean
  `[0.5, 0.5, 0.5]` and standard deviation `[0.5, 0.5, 0.5]`, threshold `0.3`,
  box threshold `0.5`, at most 1,000 candidates, unclip ratio `1.6`, dilation
  enabled, and fast scoring;
- recognizer: BGR order remains unchanged; dynamic-width `[3, 48, 320]` nominal
  input, batch size 6, aspect-preserving resize and right padding,
  model-embedded vocabulary, greedy CTC argmax, adjacent duplicate removal,
  blank-token removal, and concatenation;
- confidence: arithmetic mean of retained non-blank token maxima after duplicate
  removal, with each token rounded to five decimals and the mean rounded to five
  decimals, matching RapidOCR v3.9.2 `CTCLabelDecode`;
- engine: CPU execution provider only, graph optimizations enabled, CPU memory
  arena disabled, one intra-op thread, one inter-op thread, no classifier, no
  CoreML, DirectML, CUDA, CANN, OpenVINO, Paddle, or Torch provider;
- input bounds: the five committed fixture PNGs only, each below 2,000 pixels on
  either side; no network after model provisioning.

Each fresh candidate process has a ten-minute external deadline. It loads and
validates no file above 64 MiB, performs two unreported warm-up passes, then ten
measured passes over the five images. Initialization time, per-pass end-to-end
median/p95/maximum, total model bytes, and process peak resident bytes are retained
as tie-breakers. They are not Phase 3 product budgets: a correctness failure can
never be traded for lower latency or memory.

## Hard quality gates

Every candidate must satisfy every row independently on each release target:

| Gate | Required result |
|---|---|
| Model identity | Both file digests, ONNX input/output shapes, and embedded vocabulary digest match the candidate record before inference |
| Text | Every expected region appears exactly once with exact NFC text after trimming leading and trailing Unicode whitespace |
| Region count | Exactly the manifest region count; no unexpected region at confidence `>= 0.5` |
| Geometry | Matched source-relative quadrilateral has IoU `>= 0.50`, absolute center delta X `<= 0.025`, and absolute center delta Y `<= 0.025` |
| Ordering | Matched region identifiers occur in exact manifest order, which binds RapidOCR v3.9.2's stable Y / adjacent-10-pixel row / X sort |
| Confidence | Every matched region reports a finite decoder value in `[0.0, 1.0]`; confidence is recorded but has no universal hard floor |
| Stability | All ten measured passes return identical normalized text, count, order, geometry gate, and exact confidence values |
| Cross-target | Windows and Apple Silicon have identical normalized text, count, order, and gate outcomes; confidence values, timing, and resident memory may differ |
| Privacy | Tracked evidence contains only approved expected fixture text, aggregate metrics, model/profile identities, geometry, digests, licenses, and typed outcomes |

A missing row, exception, timeout, digest drift, vocabulary omission, target-only
success, incompatible license, or unsupported controlled deployment rejects the
candidate. V2 removes one disproved cross-model interpretation; it does not
weaken or rewrite any observed-output oracle.

## Privacy allowlist

Tracked reports may contain product/upstream revisions, release target, operating
system version/build, CPU architecture and logical-core count, physical memory,
Python/RapidOCR/ONNX Runtime/OpenCV/Pillow versions, model identifiers/digests/
byte counts/shapes/vocabulary digest and count, fixture identifiers/digests,
aggregate latency and resident memory, expected fixture text and geometry, gate
counts, and typed failure categories.

Reports exclude user and machine names, home or model paths, environment variables,
credentials, network addresses, hardware serials, unrelated application metadata,
desktop pixels, non-fixture images, raw tensors, and unexpected recognized text.
Raw output stays in ignored Rasen ephemera and is deleted under the local retention
policy after the accepted decision is preserved.
