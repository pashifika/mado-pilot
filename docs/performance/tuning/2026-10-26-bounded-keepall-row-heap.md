# 2026-10-26: Bounded KeepAll row heap

## Problem and change

The [first dense selection observation](2026-10-26-dense-keepall-selection-observation.md)
reported 54 counted row allocations in the current implementation, versus 26 in
its historical baseline. These whole-row totals include setup and warmup; they
are not selector-only counts or evidence about image preparation.

For `KeepAll` with result limit `k` below map height, retain only the best `k`
row winners in a weakest-first heap, then reuse its vector for greatest-first
selection. Every omitted row has `k` untouched winners ahead of it and cannot
enter the requested prefix. Existing lazy row repair still allows several
results from one retained row. Public scores and row-major tie ordering are
unchanged.

Keep the existing path for a single result, `k >= height`, and
`DropOverlapping`. Overlap suppression can discard several row winners without
emitting them, so the same pruning proof does not apply there. Heap capacity
still grows from actual qualifying rows; empty and sparse maps do not reserve
storage for the full image height. No persistent scratch cache, dependency,
public API/ABI, thread setting or preparation-cache limit changes.

## Direct verification

Native Apple Silicon / Rust 1.97.1 executes exact before/after production selector
sources in one standalone allocation probe. Before is `36f0caf`; exact source
hashes and outcomes are in [the probe record](../../evidence/p050-selector-allocation/apple-direct-probe.json).
The final source differs from the probe snapshot only in `rustfmt` formatting
inside `cfg(test)`; production-prefix bytes are identical. The probe uses the
existing benchmark allocator wrapper over `Accounting` and `System`, with input
allocation and expected-result construction outside the measured call. Each
case uses a 1920×1080 map, limit 16, threshold 0.8 and `KeepAll`.

| Input | Before allocations | After allocations | Before additional counted-heap peak, bytes | After additional counted-heap peak, bytes |
| --- | ---: | ---: | ---: | ---: |
| Dense equal scores | 11 | 4 | 49,152 | 640 |
| Three results in two rows | 2 | 2 | 160 | 160 |
| No qualifying scores | 0 | 0 | 0 | 0 |

Both roles return the exact independent ordered results: 16 dense, three sparse
and zero empty. This supports the bounded allocation mechanism, including vector
reuse, on this compiler. Counted allocator peak is not RSS or native allocation
coverage. No timing or OpenCV matching call is measured by this probe. Full raw
execution and probe sources remain at the private evidence locator; the compact
record is not a standalone reproducibility packet.

The affected OpenCV backend suite passes 62 unit tests, 27 algorithm tests,
one vision-contract integration test and one documentation test in an isolated
Cargo root. Canonical-reference, suppression, cancellation/deadline and
preparation ownership tests remain active.
Independent implementation and observation-scope reviews report no findings.

## Native component observation

On Apple Silicon and Windows x64 MSVC, retained `36f0caf` and freshly
built/attested `9dd20c55` execute five existing selector rows once per role,
with one warmup and three retained units. The
[Apple record](../../evidence/p050-selector-allocation/apple-observation.json)
and [Windows record](../../evidence/p050-selector-allocation/windows-selector-observation.json)
link raw producer output and inspections. All ten row processes and ten
separate manifest processes per target complete with zero oracle failures and
successful cleanup. Before-role inspection precedes the after role. Windows
also passes the same 91-test backend suite and five actual provenance probes.

| Workload | Both targets' row allocations, before → after | Apple peak counted Rust heap, bytes, before → after | Windows peak counted Rust heap, bytes, before → after |
| --- | ---: | ---: | ---: |
| `selection-k16-dense1080p-keepall` | 54 → 26 | 16,639,883 → 16,591,371 | 16,639,282 → 16,590,770 |
| `selection-k16-dense1080p-dropoverlapping` | 62 → 62 | 16,639,891 → 16,639,891 | 16,639,290 → 16,639,290 |
| `selection-k256-small-dense-tied` | 46 → 46 | 67,082 → 67,082 | 66,481 → 66,481 |
| `selection-k256-large-sparse` | 16 → 16 | 66,356,998 → 66,356,998 | 66,356,397 → 66,356,397 |
| `selection-k1-large` | 12 → 12 | 66,356,853 → 66,356,853 | 66,356,252 → 66,356,252 |

Dense KeepAll retains 610,044 visited score offsets and zero score-copy bytes
in both roles. Row counters include setup and warmup; they are not the direct
probe's single-call counters. No normal latency/CPU benefit is inferred.
Candidates use standard Git-checkout attestation. An Apple export-mode attempt
against the checkout is retained separately as a failed source-applicability
attempt, with no numerical row or binary rebuild. Windows retains data-only
publication/packaging failures separately; no native retry follows from them.

Adverse OS-resource observations remain in the raw records: Windows candidate
peak RSS/private-after increase for overlap, small-tied and sparse rows; k1
peak RSS increases while private-after decreases. The sparse operation CPU
delta of 15,625,000 to zero nanoseconds is quantization, not CPU-free work.
Complete lifetime Job CPU with unavailable effective resolution is not a
normal timing benefit or process/image census. Counted Rust post-drop residual
is zero for both selector roles on each target; this is not native leak proof.

## Trade-offs and disposition

Retain the implementation after both-target source-bound verification. Building the
bounded initial heap adds comparisons, and conversion to a greatest-first heap
has `O(k)` work checked for interruption before and after, not inside the standard
library heapify operation. Normal latency/CPU and large-budget interruption
latency are not established by allocation counts. The five existing selector
smoke rows provide changed-path and unchanged-path observations; sparse/empty
KeepAll behavior is separately exercised by the direct probe and contract tests.

Preparation is a [separate both-target observation](2026-10-26-preparation-reuse-observation.md):
the four existing eight-template batch rows compare retained `8cc67f5` with
retained `36f0caf`, not this selector change. Templates are prepared outside each
timed search operation. No image-preparation implementation or
two-entry/33,554,432-byte bound changes in this adjustment.

Actual-consumer work, normal-build benefit and explicit qualification retain
their own evidence requirements. No host-idle admission, automatic calibration,
historical regrading or release approval is introduced.
