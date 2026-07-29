# ADR 0003: OpenCV matching profile and public score mapping

- **Status:** Accepted
- **Date:** 2026-07-28
- **Resolves gate:** _none_. This records the Phase 1 backend profile that gate
  `G-013`'s vision workloads and gate `G-007`'s packaging decision will later
  measure and ship.
- **Supersedes:** _none_
- **Amended:** 2026-07-28, after the Phase 1 verification pass aligned backend extraction with the public suppression policy and canonical public-score ordering.

## Context

`mado-pilot-vision` fixes the public score as a finite value inside `0.0..=1.0`.
A backend that reports anything else is refused by the matcher as
`VisionFault::BackendScoreOutOfRange`, a `Status::VisionFailed` — the range is a
contract every backend is normalized to, not a property of a request.

OpenCV's `matchTemplate` offers six methods, and none of them produces that range
directly for the algorithm this product needs:

| Method | Range for 8-bit input | Better score is |
|---|---|---|
| `TM_SQDIFF` | unbounded above | lower |
| `TM_SQDIFF_NORMED` | `0.0` upward, no fixed ceiling | lower |
| `TM_CCORR` | unbounded above | higher |
| `TM_CCORR_NORMED` | `0.0..=1.0` | higher |
| `TM_CCOEFF` | unbounded both ways | higher |
| `TM_CCOEFF_NORMED` | `-1.0..=1.0` | higher |

So the adapter has to pick a method and state how its values become public
scores. Doing that implicitly would put a numeric meaning into the public contract
that no document defines, which is worse than either choice on its own.

Two further decisions turned out to be inseparable from the first, because each
one determines what a score *means* to a caller: what preprocessing the profile
performs, and what counts as one candidate in a dense correlation map.

## Decision

The Phase 1 OpenCV CPU profile is:

1. **Three-channel BGR, alpha dropped.** The adapter declares
   `PixelFormat::Bgra8` as the format the matcher maps a searched region into, and
   converts it to BGR with `COLOR_BGRA2BGR`. Template content is decoded with
   `IMREAD_COLOR`, which yields the same layout whatever the file declared.
2. **`TM_CCOEFF_NORMED`.**
3. **The public score is the raw value clamped to `0.0..=1.0`.** The two ends are
   clamped for different reasons, and only the lower one is a decision: below
   zero, a clamp says that an inverted pattern and an unrelated one are equally
   "not the template". Above one, the clamp is a rounding guard, because
   normalized correlation cannot genuinely exceed one.
4. **A non-finite raw value produces no candidate.** Normalized correlation
   divides by the variance of the template and of the window, so a uniform window
   has no correlation to express. No correlation is not evidence of a match, and
   it is not a backend failure either.
5. **Candidate extraction follows the request's suppression policy.** The adapter
   repeatedly takes the greatest remaining public score, breaking equal public
   scores row-major. `DropOverlapping` suppresses every offset that would overlap
   the selected placement; `KeepAll` removes only the selected offset. Extraction
   stops at the request's result limit, so it emits the exact canonical prefix the
   public matcher can expose without materializing the complete dense map.

The matcher remains authoritative: it validates scores, applies the threshold,
translates region-local candidates, orders canonically, applies the requested
suppression and result limit again, and builds the result envelope. OpenCV's bounded
extraction may reduce work only by applying those same observable prefix rules; it
cannot invent a backend-specific suppression outcome.

## Alternatives

**Rescale the correlation range onto the public range** as `(raw + 1) / 2`. This
is monotone and loses nothing, which is its appeal. Rejected because it changes
what a threshold means: a public score of `0.5` would denote *no correlation*, so
a caller asking for `0.9` would be asking for `0.8` correlation and a caller
reading `0.5` would reasonably conclude "half a match" about a window with no
relationship to the template at all. A public number whose midpoint means
"unrelated" is a worse contract than one whose zero does.

The clamp's cost is bounded and checkable. Every threshold above `0.0` is
unaffected, because no clamped value can reach it. A threshold of exactly `0.0`
already means "report every offset", and it accepts a genuine `0.0` too, so the
clamp does not admit anything such a caller did not ask for. What is lost is the
ability to distinguish `-0.9` from `-0.1` in a public score, and nothing in the
Phase 1 contract can act on that distinction.

**`TM_CCORR_NORMED`, whose range needs no mapping at all.** Rejected because it
does not subtract the mean, so a uniformly bright window correlates highly with
almost any template. Choosing the method that avoids a documented clamp at the
cost of systematic false positives on bright regions would be optimizing the
document rather than the product.

**`TM_SQDIFF_NORMED`, inverted as `1 - value`.** Rejected because its upper bound
is not one: the normalizer is the geometric mean of the two energies, and a bright
template against a dark window drives the value well above one. It would need a
clamp too, and its clamp would be at the *top*, discarding real differences
between genuinely bad matches instead of collapsing values no threshold can
reach.

**Greyscale rather than BGR.** Rejected because colour is discriminative in
exactly this product's domain: two controls of the same luminance and different
hue are two controls. Greyscale is also *more* preprocessing than BGR, not less —
dropping alpha is the minimum that makes a captured frame and a decoded template
comparable, and a captured frame's alpha channel is not part of what a user sees.
Multi-channel `matchTemplate` sums over channels, so the cost is three passes
rather than one, which Phase 1 has no evidence to trade correctness for.

**Honour a template's alpha as a mask.** Deferred, not rejected. OpenCV's masked
`matchTemplate` supports it, but a mask changes what the normalizer covers and so
changes what a score means, which needs its own fixtures and its own recorded
decision.

**Report every offset above the threshold instead of peaks.** Rejected because
the public suppression rule cannot repair a dense map afterwards: it drops a
candidate overlapping a canonically *earlier* survivor, so the survivor of one
match's correlation hill would be the hill's top-left pixel rather than its peak.
Reporting peaks yields the set the public rules would keep, without materializing
thousands of candidates for one match.

## Consequences

Callers get a score whose zero means "not the template" and whose one means
"exact", with no scaling to undo, and a threshold that means what it reads. In
exchange:

- **Negative correlation is not observable** through any Phase 1 public surface.
  A caller that needs to detect an inverted pattern cannot express it, and adding
  it later means a new option rather than a changed score, because changing the
  mapping would move every existing threshold.
- **`Suppression::KeepAll` preserves overlapping placements.**
  `Suppression::DropOverlapping` keeps the canonical peak and suppresses its
  overlap window. The two policies therefore remain observably distinct for this
  backend, as the public contract requires.
- **A caller's result limit bounds the adapter's work.** Candidate extraction
  performs at most one scan of the score map per requested result. A caller asking
  for `u32::MAX` results is asking for at most one scan per offset; the loop still
  terminates because every scan removes at least the selected offset.
- **Colour-blind matching is not available.** A template that should match
  regardless of hue cannot be expressed in Phase 1.
- **Scores are not bit-reproducible across offsets.** OpenCV normalizes through
  integral images and computes the correlation numerator over the whole region at
  once, so a score carries rounding from arithmetic involving the rest of the
  scene. Two byte-identical copies of one patch in one image were measured at
  `1.0` and `1.0 - 3.6e-7`, and which of the two differs depends on what else the
  scene contains. Fixtures therefore compare scores against a tolerance, and never
  assert an ordering between two candidates whose scores differ by less than it.

  Notably, this is *not* mainly a cross-target effect. Across the two release
  targets the fixtures agree on every outcome, count, identity, rectangle, and
  ordering, and on sixteen of seventeen scores bit-for-bit; the single difference
  is five `f32` units in the last place. The tolerance exists because one host
  varies against itself, not because the hosts vary against each other.
- **OpenCV writes its own diagnostics to standard error** for malformed image
  content, outside MadoPilot's diagnostic surface. The adapter does not silence
  the library globally, because a host may be using OpenCV for its own work and an
  adapter is not the right place to change process-wide logging. An integrator
  who needs it quiet sets OpenCV's log level.

The dependency obligations this brings are recorded in
[third-party-dependencies.md](../third-party-dependencies.md): OpenCV 4 is a
development prerequisite in Phase 1, no release bundling is claimed while `G-007`
is open, and an absent library is a process-load failure rather than an actionable
status. Closing that gap needs deferred or weak dynamic loading, which belongs
with `G-007`'s controlled library search paths rather than with `G-008`, whose
scope is static-link feasibility.

## Verification

- `crates/backend/opencv/src/candidates.rs` unit-tests the score mapping and bounded
  extraction directly, including the negative half, the rounding guard, the
  non-finite offset, equal-public-score row-major ties, `KeepAll`, the overlap
  window, and the result limit. These need no OpenCV installation, so the decisions
  in them are checkable without one.
- `crates/backend/opencv/tests/vision_contract.rs` runs
  `mado-pilot-testkit`'s backend-independent suite against this adapter unchanged.
  A profile that broke score validation, candidate bounds, or the three
  successful-empty outcomes fails there.
- `crates/backend/opencv/tests/algorithm.rs` asserts exact bounds, exact result
  counts, canonical ordering where scores separate, full-frame coordinates from a
  region and from a clipped region, and the absent template's margin below the
  fixture threshold.
- The measured scores behind the tolerance are recorded in
  [../evidence/vision-opencv/](../evidence/vision-opencv/), produced on both
  release targets by `cargo run --release --package mado-pilot-backend-opencv
  --example match-report`, which searches the same fixtures the tests use.
