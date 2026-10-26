# 2026-10-26: Dense KeepAll selection observation

## Problem and intended outcome

Observe whether the current selector reduces copied score data and visited
score offsets while preserving the canonical ordered top-16 results for
`selection-k16-dense1080p-keepall`. This is the first focused development
observation under the [component method](../methods/component.md), not a
performance qualification or completion of the p050 release measurement scope.

## Change and reason

Retain the current row-winner selector. No production code, empirical target or
runtime limit changes in this record. The measurement-only baseline successor
ports PR112's smoke exit predicate and its regression tests while preserving the
historical repeated-full-map selector, dense copy and uncached implementation.
The comparison spans whole source revisions; it does not isolate one commit.

## Scope

- Baseline: `8cc67f5a20a1e4726913cf4da6963b5d658c9e84`, successor of retained
  measurement port `d787cb702515646e6126d1af966dfe359b2be2c5`.
- Current and collector: PR112 merge
  `36f0cafb02b5e350ea77a6570e68171d44674655`.
- Native Apple Silicon, macOS 27.0.1, Rust 1.97.1, OpenCV 4.14.0. The three
  observed OpenCV image identities match across roles. TBB reports 10 configured
  threads before and after each process; this selector does not call OpenCV's
  matching kernel.
- Protocol4 instrumented smoke: one independent row process per role, one warmup
  and three retained iterations. Baseline was inspected before current ran.
  Manifest discovery and real source-export attestation are separate processes.
- Identical 1920×1080 score map, template extent 12×10, inclusive threshold 0.8,
  result limit 16 and `KeepAll`. The independent oracle includes nonfinite values,
  clamping and row-major ties, and compares the full ordered result.
- No capture/input, preparation-cache, `matchTemplate`, actual-consumer,
  normal-build, calibration, A/A or full-corpus measurement. Windows results are
  not included in this Apple record.

## Evidence and effect

The [observation manifest](../../evidence/p050-focused-selection/apple-observation.json)
binds exact source/tree, lock, commands, executable hashes, raw-output hashes,
process identities, cleanup and execution limits. Unmodified producer output:
[baseline](../../evidence/p050-focused-selection/apple-baseline-producer.json),
[current](../../evidence/p050-focused-selection/apple-current-producer.json).
Full source-export, attestation and owner receipts remain at the private evidence
locator in the manifest; this compact publication is not the full provenance
packet or a reproducible-build claim.

| Apple observation | Baseline | Current |
| --- | ---: | ---: |
| Oracle failures, warmup plus retained | 0 | 0 |
| Score offsets visited, four calls | 132,710,400 | 610,044 |
| Score bytes copied by selector, four calls | 33,177,600 | 0 |
| Counted Rust allocations, whole row | 26 | 54 |
| Peak counted Rust heap, bytes | 24,885,707 | 16,639,883 |
| Counted Rust heap residual after owner drop, bytes | 0 | 0 |
| Process peak RSS, bytes | 37,388,288 | 29,147,136 |
| RSS residual after owner drop, bytes | 25,116,672 | 16,859,136 |
| Operation p95 from three retained samples, ns | 32,530,875 | 157,208 |
| Retained operation CPU sum, ns | 97,507,000 | 465,000 |
| Retained-loop CPU, ns | 98,061,000 | 948,000 |

Work/allocation counters include setup and warmup; allocation counts are not
per-operation counts. Operation timing excludes scratch restoration and the
reference oracle. Retained-loop CPU includes those costs and sample storage.
RSS is process resident memory, not counted Rust heap or a complete native
allocation account. Three inner samples are not three independent starts.

Both collectors report `execution_status=completed`, `status=passed`, no issues
and `qualification_status=not-requested`. Both row children exit 0 with known
successful cleanup; each smoke owner retains three successful native-call
cleanups, including its processor-identity query, with no false/unknown cleanup.
These counts are not a process-role census. The baseline successor's three
exit-contract tests passed in a separate target. Current normal/instrumented
producer checks on both targets passed in [merged-source CI](https://github.com/pashifika/mado-pilot/actions/runs/40285129179).

## Trade-offs and disposition

Retain the current selector: this fixture shows fewer visited offsets and no
selector dense copy, with correct ordered results and zero final counted-heap
residual. The current row uses **28 more counted Rust allocations**; this cost is
retained in the result, not hidden by the lower peak heap or short observed time.

Do not infer a normal-build speedup, stable latency percentile, complete image
closure, native leak freedom or release/platform qualification. The nonzero RSS
residual is not rewritten as zero when counted Rust owners release. No host-idle
threshold, quiet-host retry or automatic resampling was used.

The next separate engineering question is preparation reuse and its ownership
costs; this score-map-only row cannot answer it. Any allocation tuning needs a
separately scoped observation that also covers sparse/empty rows, rather than
preallocating for dense input based on this one result. The original p050
preparation, consumer and explicit qualification obligations remain open.
