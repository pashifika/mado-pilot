# 2026-10-26: OCR workload profile authority

## Problem and intended outcome

The OCR workload runner coupled current numerical enforcement to an ADR filename
and its Markdown `Accepted` status. Current product commands need explicit
machine-readable workload/artifact settings without executing a historical
prose-approval rule.

## Change and reason

Introduce `ocr-text-watch-v3-aarch64-apple-darwin.toml` and
`ocr-text-watch-v3-x86_64-pc-windows-msvc.toml` with `format_version = 3`.
Explicit host and controlled/real executable bindings describe scope;
`review.tuning_record` links this rationale but is not opened or parsed as
execution authority. Existing format-2 files retain their original bytes and
are historical inspection inputs, not current execution profiles.

Numerical limit values are unchanged. Ordinary observation can omit a profile
for a newly identified candidate/host. Supplying a scoped current profile retains
its host/artifact checks; `--enforce-budgets` explicitly requests numerical
non-success on a selected miss. Source/native-image/semantic/cleanup checks stay
independent of numerical results. See the [current OCR method](../methods/ocr-watch.md).

## Scope

Python workload/profile tooling and the two successor target profiles, relative
to source `646369fad6c922ac7b064367817337a18e572d8c`. This does not rebuild the
pinned Rust producers, change workload geometry/cadence/counts or infer that old
binaries match a new checkout. The original source/build/host provenance remains
scoped to its original observations. No new source, host, executable, model or
native capture is qualified by retaining old ceiling values.

## Evidence and effect

**Performance outcome: not-measured.** No new precursor, final enforcement cohort
or before/after latency/RSS experiment is recorded here. There is no speedup or
new numerical qualification claim. This is an authority/format migration.

The [historical ceiling derivation](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0075-ocr-text-watch-workload-profiles.md#derivation-and-scope)
records twice the worst process-local timing (upward to `0.001ms`, with the
separate maximum floor), `1.25` times target memory maxima (upward to `1MiB`),
and fixed-fixture logical extents. It is historical provenance, not current
Markdown admission. Original failures and final cohort outcomes remain unchanged.
The [current method](../methods/ocr-watch.md#historical-outcomes) keeps precursor,
explicit enforcement and native-semantic outcomes distinct.

## Trade-offs and disposition

**Retained.** A prose status no longer serves as an executable oracle; schema,
exact configuration/artifact identity and explicit enforcement carry the runnable
contract. Ordinary observations are descriptive, including missing/noisy data
and target misses. Changing a future empirical limit uses normal review and a
new attributable rationale, not retroactive regrading or a waiver of runtime
resource limits. Product-only operation needs no planning archive.
