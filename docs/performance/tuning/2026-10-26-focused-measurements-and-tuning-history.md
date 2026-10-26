# 2026-10-26: Focused measurements and tuning history

## Problem and intended outcome

Measurement instructions mixed current methods, historical qualification failures
and ADR-based procedural authority. Contributors need to investigate one bounded
workload without obtaining an unrelated statistical qualification first.

## Change and reason

Use the [performance entrypoint](../../performance.md) and one current method for
[component](../methods/component.md), [actual-consumer](../methods/actual-consumer.md)
and [OCR-watch](../methods/ocr-watch.md) work. Record tuning rationale/effect in
Markdown under ordinary review. Development observations retain empirical misses
without making those misses alone execution failures; explicit qualification and
budget enforcement retain their selected numerical failures. No runtime
algorithm, numerical ceiling or safety limit is adjusted by this governance
change. No CPU-idle threshold or retry-until-quiet procedure is introduced.

## Scope

Documentation and developer measurement tooling, relative to pre-cutover product
source `646369fad6c922ac7b064367817337a18e572d8c`, on the existing Apple Silicon
and Windows x64 lanes. No SDK/ABI, default allocator/thread/backend, native
permission, supported-platform or release-policy expansion is part of this
record. Historical source-bound profiles, receipts and verdicts are not changed.

## Evidence and effect

**Performance outcome: not-measured.** This is a documentation/measurement-authority
change, not a before/after performance experiment. There are no comparable
latency, CPU or memory values and no speedup claim. The method pages describe
the selected smoke/non-enforcing commands and separate their report/exit meaning
from optional qualification; they do not claim those commands were run here.

The [immutable pre-cutover performance ledger](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/performance.md)
is historical evidence of the former organization and retained outcomes, not
current execution authority. Required correctness, identity, finite execution,
owned cleanup and compatibility checks remain current contracts.

## Trade-offs and disposition

**Retained.** Small observations answer narrower questions and may be noisy or
incomplete; they cannot support an all-target performance claim. Numerical
control failures remain failed for qualification while safe diagnostics remain
available. Identity/ownership/unfinished-cleanup barriers still prevent unsafe
execution. This separation makes investigation possible without relabeling
historical failures as passes or granting release approval.
