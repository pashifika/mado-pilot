# ADR 0070: Compare the OCR mapping without channel conversion

- **Status:** Accepted
- **Date:** 2026-09-13
- **Resolves gate:** none
- **Supersedes:** none

## Context

The CPU bounded-v2 backend declares `PixelFormat::Bgra8` in
`crates/backend/onnx/src/lib.rs`. The released `ChangeDetector` accepts only
`Rgba8` mappings in `crates/automation/vision/src/change.rs`. Passing the prepared
OCR mapping directly to that comparator always requires analysis, even when
visible pixels are identical. Mapping again as RGBA adds conversion and storage
solely to recover information already present in BGRA.

The Rust OCR watcher design requires one exact ROI mapping and preserves the
released template comparator. Ordinals 0067–0069 belong to the separate frozen
foreign-query work; this decision does not import those changes.

## Decision

The OCR-private scheduler compares compatible BGRA mappings directly under the
existing `ExactRgba` policy's pixel-equality meaning. It checks complete source,
epoch, strictly increasing sequence, geometry revision, transform, ROI and pixel
layout compatibility, then compares visible row bytes outside state locks. Row
padding is not image content. Unsupported or incompatible evidence requires OCR.

The fixed channel permutation `(R,G,B,A) -> (B,G,R,A)` is bijective and
self-inverse. Equality before and after that permutation is equivalent for each
pixel and therefore for every visible row. No second mapping, pixel copy, lossy
conversion, new public policy or change to the released comparator is needed.
Confirmation still requires distinct accepted OCR positives; equal pixels never
advance confirmation or suppress a required confirmation analysis.

## Alternatives

- Map RGBA separately: preserves the old helper but adds avoidable mapping,
  copying and retained storage to every OCR comparison.
- Pass BGRA to the released helper unchanged: silently disables unchanged-frame
  admission, so the default policy no longer avoids redundant inference.
- Broaden the released comparator: unnecessary impact on the already qualified
  template and foreign paths for an OCR-private storage representation.

## Consequences

The runtime owns a small OCR-private visible-row comparison with the same
compatibility and fail-safe rules. It must stay aligned with the public policy's
exact equality semantics, not evolve into a second configurable detector.
Callers retain the same policy selection and backend/profile/provider identity.
Packaging, model bytes, platform floors and foreign surfaces are unchanged.

The cost avoided is structural: a second ROI mapping/conversion. This ADR makes
no measured latency, allocation, RSS or release-support claim. New OCR workload
ceilings remain an independent G-013 gate.

## Verification

The derivation retained with the Change enumerates every ordered byte pair in
each of the four channels: 262,144 cases preserve equality and the self-inverse
property. It executes without OCR, capture, or private inputs:

```sh
python3 rasen/changes/rust-ocr-text-presence-watch/evidence/prove-exact-bgra.py
```

Acceptance additionally requires runtime regressions for compatible unchanged
BGRA, changed visible pixels, ignored row padding, invalid source/layout
authority, and forced consecutive confirmation. The permutation proof alone
does not establish those runtime rules or qualify either native target.
