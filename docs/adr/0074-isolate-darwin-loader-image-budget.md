# ADR 0074: Isolate the Darwin loader-image budget

- **Status:** Accepted
- **Date:** 2026-09-13
- **Resolves gate:** _none_; OCR workload budgets remain open
- **Supersedes:** ADR 0072's process-wide file limit and exec-only launcher topology, not its historical evidence or numeric report/output limits

## Context

On source `ac0eb2312ec13cb332c0dfd777d2768f4f9f4b47`, Apple replay and the
three-process controlled precursor completed. Cold-startup process4 then emitted
a valid result, complete measurements and its exact approved image set, but
terminated with `SIGXFSZ`. Its image report was87,293 bytes, below1MiB. Process5
was not started. The triggering write and path for that native process could
not be identified from retained OS logs; no cache pathname is claimed.

The launcher imposed `RLIMIT_FSIZE=1MiB` on every regular-file write, not only
loader diagnostics. A model-free child with native signal disposition reproduced
the same `SIGXFSZ` while writing an unrelated2MiB file; its image report remained
below the budget. The global limit therefore changes native behavior outside the
evidence channel and can turn successful runtime work into a process failure.

## Decision

Keep a small collector process around the native candidate. Pass one anonymous
pipe across exec and direct both dyld diagnostics and the child-side report
setting to that pipe. The collector writes an exclusive private0600 file in
64KiB chunks, never beyond1MiB. Saturation, incomplete observation or collection
failure remains nonpass; stdout and stderr retain their separate supervisor
budget.

Do not impose a new process-wide file-size limit. Inherited OS limits are
unchanged, never raised. This is not a sandbox for arbitrary native file writes;
model/asset validation and developer-owned prerequisite policy remain separate.

The candidate stays inside the supervisor-owned process group. The collector
preserves native exit status/signals, closes its pipe endpoints, and terminates
and reaps its own child on collection failure. Outer deadlines and bounded
process-tree cleanup remain authoritative. No shared supervisor, Rust API,
backend selection or native input/capture implementation changes.

## Measurement scope

Rust `Instant`, RSS, physical-footprint and process-peak rows describe the native
candidate, not its Python collector or the outer Python driver. Whole-process
supervisor timing includes collector startup, native execution, draining and
owned cleanup. The collector has a Python baseline plus bounded read/write
buffers; its RSS is not silently included in or claimed by native-process rows.

The1MiB image budget, ordinary-output budgets, image-count rules and first-failure
stop policy do not change. Exact dependency manifests still require independent
approval. Earlier outcomes remain bound to their original source and collector;
a corrected cohort cannot replace or merge away its predecessor's failed row.

## Verification and alternatives

Owned model-free checks establish real dyld capture, ordinary-output separation,
report saturation refusal, unchanged inherited limits, an unrelated2MiB write,
native-signal propagation and supervised timeout cleanup. This validates the
collector contract, not OCR numerical or native-capture qualification.

Raising the global file limit would preserve the wrong coupling. Ignoring the
signal or replacing a failed sample would hide the failure. Adding a Rust dyld
callback/FFI lifetime boundary is unnecessary for this private evidence channel.
