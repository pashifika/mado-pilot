# ADR 0086: Template-matching preparation reuse and selection work

- **Status:** Accepted
- **Date:** 2026-10-20
- **Resolves gate:** none
- **Supersedes:** none

## Context

The Rust OpenCV path converts each searched region independently and copies its
score matrix before repeated whole-map scans. Live cropped mappings already own
immutable CPU pixels, but frame mapping has no pre-copy reuse. Distinct public
`Frame` constructors can use equal numeric stamps, so stamps alone cannot
identify reusable pixels.

The supported Rust 1.97.1 toolchain rejects
`std::sync::Arc::try_new(0_u8)` with `E0658: use of unstable library feature
allocator_api`. A fallible lazy-cache allocation protocol would therefore need
additional machinery beyond ordinary shared ownership. Fixed-capacity inline
metadata avoids that allocation and initialization protocol.

## Decision

Use bounded opportunistic reuse and consume the request-owned score matrix.
Keep result identity, canonical ordering, operation authority and public
ownership contracts unchanged.

- Capture owns an opaque weak `FrameData` identity. Every `CpuMapping` carries
  it; equal numeric stamps do not equate separate frame instances. The token
  keeps no native payload alive and exposes no address.
- Each frame contains four inline weak mapping slots. Only crop/channel
  conversions consult them; same-format full-frame mapping keeps its direct
  path. Actual mapping owners retain the pixels and reservation. Reuse remains
  `Owned`, not source zero-copy.
- Backend clones share two inline prepared-image slots, retaining at most
  33,554,432 bytes of owned immutable BGR payload. Keys use the mapped pixels'
  frame instance, complete stamp, effective region and descriptor, under the
  fixed BGRA-to-BGR profile. Diagnostic request regions and template identity
  do not identify pixel preparation.
- Cache access uses `try_lock`; contention or poisoned optional state bypasses
  reuse. Conversion, reservations, clocks, allocation and owner destruction
  occur outside guards. Publication deduplicates racing fills. Oversized images
  execute without retention.
- Empty metadata is initialized with its owner, not through a lazy allocator.
  Necessary shared ownership uses ordinary Rust allocation semantics; this
  change does not promise recovery from process-wide allocator failure.
- A caller checkpoint precedes preparation admission. This is not an atomic
  cancellation/publication transaction. Every caller independently checks its
  final result authority; another caller may use already-admitted immutable data.
- Selection mutates only its fresh score matrix. One result uses a linear scan.
  Multiple results use at most one winner/upper bound per row, repairing stale
  rows lazily. The final selected peak needs no suppression or heap reinsertion.
  Finite public scores, inclusive thresholds, signed-zero ties, row-major
  ordering and both suppression modes remain canonical.

### Measurement policy

The opt-in collector fixes 21 first-party rows before optimization: 13
statistical rows with 20 warmups and 200 retained samples, plus eight short
contract rows. Five fresh baseline processes calibrate each row. A separate
alternating five-baseline/five-candidate cohort measures comparison. Failed
attempts, samples and unavailable observations are retained, never retried or
excluded.

For each statistical metric, let spread be the calibration maximum minus
minimum. The allowance is the greater of spread and twice the observed clock
resolution. CPU resolution is at least 1,000 ns on Apple Silicon or 15,625,000 ns
on Windows; these are conservative accounting floors, not measured accuracy.
Spread divided by the greater of the median and twice that resolution must not
exceed 0.15. The per-process ceiling is the calibration maximum plus allowance,
rounded upward. Improvement rows must independently reduce median p95 and CPU
by more than the complete allowance.

Resource ceilings are the calibration maximum plus the greater of spread and
fixed slack: 1 MiB for RSS, 256 KiB for Rust heap. Only non-selection RSS peaks
add one 32 MiB live-cache allowance. Steady-state growth and final-owner residual
use positive endpoint differences, with no per-cycle or live-cache exemption.
Cache-held and evicted-but-live BGR payload are separate; final-owner residual
payload must be zero. Each process must pass; improved medians cannot cancel a
failed process.

Numerical profiles require successful target-specific calibration and review
before candidate timing. Missing or failed target evidence leaves this Change
incomplete. Hosted semantic smoke is not performance qualification. Historical
profiles keep their original source and binary identities.

## Alternatives

- Stamp-only keys cannot distinguish valid equal-stamp frames with different
  pixels. Content hashing adds another whole-image pass and changes identity.
- Strong mapping retention would extend adapter CPU-copy reservations. Retain
  independent BGR images instead; the weak mapping cache helps only while an
  existing pixel owner remains alive.
- Lazy optional metadata adds allocation and initialization states. Inline
  bounded slots are simpler; the tradeoff is fixed metadata in every owner,
  including owners that never use reuse.
- A full-image heap or sort creates avoidable selection storage. Row winners
  bound the index by image height without promising a universal speedup.

## Consequences

No caller migration, facade export or foreign-header change is required.
Independent backend instances each have their own allowance. Session close does
not immediately erase prepared images while a backend remains alive; expired
sources are pruned on access, and final backend ownership releases retained
images. Active evicted owners may outlive a cache entry and are measured
separately. Weak identities may retain control-block storage, not frame payloads.

Ownership and numerical rules are documented in [performance.md](../performance.md)
and the architecture baseline. No capture/input campaign, OCR policy, scheduling,
packaging or platform minimum changes are part of this decision.

## Verification

The instrumented unoptimized baseline is commit
`beb064f0c652648f92df5bfb65aaf05384a18b8d`, tree
`9ed73079aaace231eff1d234a56a86d896139ed2`. Its Apple Silicon executable SHA-256 is
`92aa6bfa4e6b2b909c1f76860fa5e46838356e128a96cf7e97870a078e2f45b7` and corpus SHA-256 is
`100c546d8aab1891abdba3f93818aee65be024f2d09e1cfbd72945e149ba1e28`.
The baseline smoke exercised 21 rows, 21 warmups and 63 retained samples with zero
oracle failures. Collector boundary tests passed 33 cases. These observations do
not qualify an optimization or Windows performance.

The Apple baseline-only calibration retained all 106 manifest/workload attempts
without an identity, cleanup or oracle failure. All 13 statistical rows satisfy
the predeclared noise cap; watcher rows remain non-statistical. The reviewed
numeric profile SHA-256 is
`96294939708e817769a511058433dd688bb1b80cefed4fcf58a9b78e247183d5`.
It was frozen before optimized smoke or comparison timing.

Optimized Apple public smoke exercises all 21 rows with zero oracle failures.
The eight-template cold batches perform one BGR conversion per iteration, live
ROI owners avoid duplicate mapping copies, and dense score-copy bytes are zero.
The existing `deterministic-slice` and `template-watch-query` smoke preserve their
eight- and eleven-workload oracles. These are semantic/work observations, not a
replacement for the frozen target-specific comparison.

The allocation API observation is reproducible with `rustc --edition=2024
--crate-type=lib --emit=metadata -` and stdin
`pub fn probe() { let _ = std::sync::Arc::try_new(0_u8); }`.
The probe exited 1 on Rust 1.97.1 and its temporary output was removed.

Capture reservation/lifetime contracts, canonical differential selection,
prepared-image ownership/privacy contracts and the actual replay benchmark
provide the implementation checks. Numerical acceptance remains governed by
reviewed per-target profiles and retained collector output, not this ADR alone.
