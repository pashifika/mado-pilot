# ADR 0085: Detector sigmoid roundoff

- **Status:** Accepted; implemented
- **Date:** 2026-10-18
- **Resolves gate:** none; no native or model-quality qualification claim

## Context

A Windows CPU G-004 inference produced a `[1, 1, 736, 3328]` detector map
with one of 2,449,408 samples equal to `f32::from_bits(0x3f800001)`:
`1.0000001192092896`, the next representable value above `1.0`. All other
samples were within `[0, 1]`; none were negative or nonfinite. The exact
`[0, 1]` guard rejected the entire recognition as `MalformedOutput`.

## Decision

Admit finite detector probabilities in `0.0..=1.0_f32.next_up()`. This allows
only the observed one-ULP upper roundoff. Do not clamp or rewrite the tensor.
Keep thresholding, box scoring, geometry, resource bounds, model identities
and public recognition confidence validation unchanged. Negative, nonfinite
and larger out-of-range values remain malformed output.

A general epsilon, symmetric tolerance or unconditional clamp would admit
unobserved faults. Retaining the exact upper bound would continue rejecting
the measured CPU output.

## Verification

A detector regression failed with `MalformedOutput` before the repair and
passed afterward, preserving the detected quadrilateral and order. Boundary
coverage rejects two ULPs above one, a negative subnormal, NaN and infinities.
The backend suite passed 73 tests; the resource-gated native test remained
ignored. A consumer replay of the original recorded image completed OCR
without the numeric fault. Its cropped text still failed the exact-text
oracle; full-frame recognition passed that oracle. The numeric repair does
not promise OCR quality or regrade the original failed run.
