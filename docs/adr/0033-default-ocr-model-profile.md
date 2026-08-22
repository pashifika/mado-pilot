# ADR 0033: Default OCR model profile

- **Status:** Proposed; `G-004` remains open pending Windows qualification
- **Date:** 2026-08-22
- **Resolves gate:** [`G-004`](../validation-gates.md#g-004) only after the selected v3 profile passes `x86_64-pc-windows-msvc` and this record is accepted
- **Supersedes:** none

## Context

Phase 3 needs one immutable CPU ONNX profile before an OCR contract or backend can
be implemented. A filename or loadable model is insufficient: language coverage,
model bytes, preprocessing, decoder, confidence meaning, license, deployment,
and identical-input behavior must be fixed independently of implementation.

The v1 fixture compared RapidOCR PP-OCRv4 Japanese mobile, PP-OCRv5 multilingual
mobile, and PP-OCRv6 multilingual small pairs. PP-OCRv4 and PP-OCRv5 failed exact
NFC text on Apple Silicon. PP-OCRv6 returned all expected text but exposed that a
universal `0.80` greedy-CTC mean was not a calibrated cross-model quality measure:
the same exact region scored `0.76011`, `0.98939`, and `0.99624` under different
recognizers. V2 retained exact text/count/geometry/order as the oracle, made
confidence a finite deterministic observation, and added synthetic dense-tooltip
and high-resolution-menu workloads derived from private real-game layout classes.
Its first run exposed a manifest authoring-order defect. V3 preserved every
historical digest and bound order to RapidOCR v3.9.2's stable Y, adjacent-10-pixel
row, then X sort.

On Apple Silicon, only the PP-OCRv4 mobile detector paired with the PP-OCRv6 small
recognizer passed all 42 v3 regions. The PP-OCRv5 detector missed one mission row;
the smaller/faster PP-OCRv6 tiny detector and PP-OCRv6 small detector each missed
one dense-tooltip row. All failures were stable over ten passes. The selected
Apple process used 25,979,900 model bytes, a 2,651.749 ms median five-image suite,
and 1,000,587,264 peak resident bytes in the evaluation-only Python/RapidOCR/
OpenCV/ONNX Runtime process. Those observations select among candidates; they are
not a Rust backend budget or support claim.

The required Windows row has not run. Therefore this ADR is proposed and `G-004`
remains open.

## Decision

Conditionally select profile `g-004-rapidocr-ppocrv4-det-v6-rec-small-v1` for the
Windows qualification row. It becomes the Phase 3 default only if the exact v3
fixture passes unchanged on `x86_64-pc-windows-msvc` and this ADR is accepted.
Until then no default identifier, backend wiring, or OCR support statement is
available.

The immutable candidate is:

- detector `ch_PP-OCRv4_det_mobile.onnx`, 4,745,517 bytes, SHA-256
  `d2a7720d45a54257208b1e13e36a8479894cb74155a5efe29462512d42f49da9`;
- recognizer `PP-OCRv6_rec_small.onnx`, 21,234,383 bytes, SHA-256
  `6f327246b50388f3c176ae304bd95767ea6dc0c9ae92153ef8cbe210b3c14884`;
- RapidOCR source/metadata version 3.9.2, Git revision
  `095232a4c94f7f0e6600ba5bba1177010ad696d4`, with the recognizer's
  18,708-entry embedded vocabulary SHA-256
  `f7aa897ca828a4c7c9e2739c30f9161a33306d532f020bcdb91dcfb664a5507e`;
- horizontal Japanese, basic Latin, ASCII digits, and the declared UI symbols;
  NFC normalization with leading/trailing whitespace trim, internal whitespace,
  case, and width preserved; vertical text unsupported by this profile;
- OpenCV-order BGR planar float32 input with no channel swap; DB detection at
  side limit 736, `limit_type=min`, dimensions rounded to 32, mean/std
  `[0.5, 0.5, 0.5]`, threshold `0.3`, box
  threshold `0.5`, maximum 1,000 candidates, unclip `1.6`, dilation enabled, fast
  scoring; orientation classification disabled;
- recognition shape `[3, 48, 320]` with dynamic width, aspect-preserving resize,
  right padding, batch size 6, embedded vocabulary, and greedy CTC duplicate/blank
  removal; confidence is the mean of retained token maxima using RapidOCR's
  five-decimal rounding and is not a calibrated probability;
- CPU execution only for v0.3.0 qualification; accelerator selection remains
  `G-006` work.

Deployment is controlled host-provided. The caller selects an explicit model root;
files occur only at
`rapidocr-v3.9.2/ch_PP-OCRv4_det_mobile.onnx` and
`rapidocr-v3.9.2/PP-OCRv6_rec_small.onnx` beneath it. The implementation must
validate safe relative paths, exact byte counts, and SHA-256 before session
creation. It must not search ambient paths, infer identity from filenames,
download a model, or substitute a later tag. Absence or mismatch produces a typed,
actionable backend-unavailable outcome naming the relative path and expected
digest.

Model bytes remain outside the repository and release. RapidOCR/PaddleOCR model
provenance is Apache-2.0; ONNX Runtime 1.29.0 used for qualification is MIT. Native
runtime bundling remains open under `G-007`.

## Alternatives

- **PP-OCRv4 Japanese mobile pair.** Rejected by unchanged exact-text rows despite
  the smallest 14,498,852-byte pair and lower Apple suite time.
- **PP-OCRv5 multilingual mobile pair.** Rejected by unchanged case/width exact-
  text rows. Case-folding or width-folding after observing output would erase UI
  distinctions the profile intentionally preserves.
- **PP-OCRv6 tiny detector plus small recognizer.** Rejected on Apple by one dense-
  tooltip exact-text row. Its lower 759,742,464-byte evaluation peak and faster
  suite cannot waive correctness.
- **PP-OCRv6 small detector plus small recognizer.** Rejected on the same dense-
  tooltip row and was slower/larger than the qualifying hybrid.
- **Treat decoder confidence `>= 0.80` as quality.** Rejected by evidence: exact
  ground-truth text received materially different model-local means. Confidence
  remains observable and caller-usable but cannot replace the exact fixture.
- **Commit or download model bytes.** Rejected. Controlled provisioning plus
  pre-session digest verification is smaller, network-free at runtime, and avoids
  silently changing a default when an upstream tag moves.
- **Use private game screenshots as public fixtures.** Rejected because
  redistribution rights are unresolved. They informed only synthetic workload
  classes; no pixel, crop, digest, title, or output is tracked.

## Consequences

The Windows operator must run all four v3 candidates with the same fixture and
tool versions; only an unchanged pass for the conditionally selected profile can
advance this ADR. A target-specific failure, missing row, digest difference, or
source drift keeps `G-004` open.

If accepted, later OCR contract/backend Changes consume this profile by reference
and cannot silently change detector, recognizer, vocabulary, preprocessing,
decoder, normalization, order, confidence semantics, deployment, or digests. A
change requires new cross-target evidence and an ADR that supersedes this one.

The implementation must prioritize bounded native memory and session lifetime.
The approximately 954 MiB Apple evaluation-process peak is not accepted as a
product ceiling. The backend Change must establish repeatable native load/
inference profiles, finite image/model/session bounds, allocation and resident
budgets, and failure before unsafe allocation. No OCR API or support claim is
created by this proposed record.

## Verification

[`../evidence/g-004/evaluation-plan.md`](../evidence/g-004/evaluation-plan.md)
fixes the oracle and amendments. [`../evidence/g-004/candidates.json`](../evidence/g-004/candidates.json)
fixes every component identity and controlled path.
[`../evidence/g-004/report-aarch64-apple-darwin.json`](../evidence/g-004/report-aarch64-apple-darwin.json)
retains sanitized Apple outcomes; the required Windows report is absent.
[`../evidence/g-004/dependency-review.md`](../evidence/g-004/dependency-review.md)
records licenses and deployment obligations.

`python3 docs/evidence/g-004/validate.py` runs in repository-policy CI and fails on
current or historical digest drift, profile/path/size/vocabulary inconsistency,
stale v3 source identity, changed Apple pass/fail rows, privacy payloads, or a
prematurely promoted gate status. Acceptance additionally requires independent
review of both target reports and exact merge-revision checks through the
protected pull request to `dev/0.3.0`.
