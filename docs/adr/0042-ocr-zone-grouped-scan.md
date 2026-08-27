# ADR 0042: OCR zone-grouped scan

- **Status:** Accepted
- **Date:** 2026-08-25
- **Resolves gate:** _none_; implements Direction Slice `phase-3-1-bounded-zone-ocr-v0-3-1`
- **Supersedes:** _none_; singular OCR and ADRs 0033–0041 remain unchanged

## Context

A caller recognizing three through eight stable game regions with the singular API repeats detector inference and serially occupies the accepted ONNX backend's one inference slot. The merged ADR 0040/0041 bounded profile already maps detector geometry back to the original source and batches recognition, so a grouped operation can share the smallest source envelope without adding a detector or public pipeline abstraction.

Independent per-zone detection, a mosaic, or a mask would change detector pixels and ordering. Duplicating normalized regions into every overlapping group would multiply text and geometry ownership. The predeclared source, fixtures, limits, and pass/fail rules are retained in [`../../rasen/changes/phase-3-1-ocr-zone-grouped-scan/evidence/`](../../rasen/changes/phase-3-1-ocr-zone-grouped-scan/evidence/).

## Decision

`mado-pilot-ocr` adds a separate borrowed zone-scan request accepting exactly one through eight caller-order `OcrZone` entries on one immutable frame, backend/model/profile, output space, and operation context. It resolves and clips every zone before work, refuses the complete request if any effective zone is empty, maps the smallest checked capture-pixel envelope once, and commits one immutable grouped result or no result.

Candidate membership is the exact centroid of the four finite source-envelope-relative quadrilateral points. A candidate belongs to every relative half-open zone satisfying `left <= x < right` and `top <= y < bottom`; there is no epsilon, intersection-area, proximity, priority, or nearest-zone rule. The same bounded helper is used by core and by backend interest filtering, while core remains authoritative.

`BackendRequest` gains an optional borrowed relative-interest view without changing `OcrBackend::recognize`. A backend may ignore the view and remain correct. The integrated `mado-pilot-backend-onnx` path filters detections after detector postprocessing and before perspective crops, performs one detector inference over the shared envelope, recognizes every unique relevant detection once in existing batches, and submits each candidate once with its global detector order. Performance qualification applies to the explicit ADR 0040 bounded profile; the implementation does not change the released default.

Construction uses one unique candidate store, an eight-bit membership mask per temporary candidate, checked `u16` membership indexes ordered by caller group then global detector order, and nine offsets. Empty groups have equal adjacent offsets. Primitive admission and per-candidate limits refuse overflow before allocation; compile-time layout assertions prove the derived aggregate ceilings, while data-dependent mapping bytes are checked at runtime. One admitted `Operation` owns interruption arbitration through the final all-or-nothing commit.

Exact duplicates, nearly equal rectangles, adjacent rectangles, and overlaps remain structurally accepted within the eight-entry bound. Exact coordinates and ordinary membership apply, but callers own deduplication and semantic reconciliation; the library emits no warning and makes no quality, latency, or independence claim for those layouts.

### Implementation proof: derived ceilings are not independent runtime states

The Rust layout and admission products prove that `1,000 × 8 = 8,000`
memberships is the maximum; a putative `8,001`st membership first requires a
`1,001`st candidate and is refused by the raw-candidate ceiling. The same
primitive bounds entail at most 16,384,000 raw text bytes, 4,096,000 normalized
text bytes, 262,144 temporary semantic bytes, 16,000 membership-index bytes,
and 5,242,880 immutable-result semantic bytes. Compile-time assertions bind
those products to the actual Rust layouts so a future layout increase fails the
build instead of adding per-candidate hot-path accounting. Mapping bytes remain
data-dependent; the complete source descriptor is checked because native
conversion may materialize it before cropping, and the returned mapping is
checked again.
This proof replaces the predeclared independent `8,001` membership execution
hypothesis; it does not raise or remove any ceiling.

## Alternatives

- **Run singular OCR once per zone.** Rejected because detector work and inference-slot occupancy scale with zone count.
- **Add `recognize_zones` to every backend.** Rejected because grouping and final membership are platform-neutral contract responsibilities; requiring a second trait method creates two correctness paths.
- **Expose detector and recognizer ports.** Rejected because it leaks the accepted backend's pipeline and session topology into the contract.
- **Build a mosaic, mask, or clustered envelope.** Rejected because it changes detector pixels, geometry, ordering, and quality without measured evidence.
- **Duplicate regions in each group.** Rejected because complete overlap multiplies bounded text and geometry allocations.
- **Deduplicate or reconcile close zones.** Rejected because application semantics are unknowable and fuzzy equality would make exact caller input non-deterministic.

## Consequences

Singular `OcrRequest`, `OcrResult`, backend method signatures, default construction, runtime/facade composition, C ABI 1.3, and C++ remain unchanged. Rust contract callers may opt into grouped scanning directly; later public-surface work may project this frozen behavior without redefining it.

Adding a distant zone can change shared-envelope detector scale and therefore another group's OCR output. Results repeat the exact source envelope and effective caller-order zones so this behavior is observable. Backends that ignore interests remain correct but receive no grouped performance claim.

Retained grouped results own normalized candidates and compact memberships only. They retain no frame, producer slot, mapping, tensor, model allocation, backend buffer, session lock, or backend owner. Diagnostics and observations report only bounded counts, geometry dimensions, resource totals, and opaque identities—never pixels, recognized text, paths, hashes, model bytes, or raw native identifiers.

## Verification

- Request/result and shared contract suites cover one/three/eight zones, clipping, ordering, explicit empty groups, ignored and honored interests, malformed output, every hard limit, terminal races, retained ownership, and safety-only overlap layouts.
- The ONNX native contract uses the fixed tracked HUD replay input to prove one detector run, unique relevant recognition, bounded batching, source-envelope mapping, ignored outside detections, cleanup, busy/close/recovery behavior, and producer progress.
- [`../../rasen/changes/phase-3-1-ocr-zone-grouped-scan/evidence/qualification-plan.md`](../../rasen/changes/phase-3-1-ocr-zone-grouped-scan/evidence/qualification-plan.md) fixes fixtures and pass/fail rules before backend execution; observed contradictions require retained evidence and an ADR update.
- Strict Rasen validation, repository dependency policy, formatting, lints, workspace tests, doctests/rustdoc, native hosted jobs, and independent contract/performance/privacy review gate the protected `dev/0.3.1` pull request.
