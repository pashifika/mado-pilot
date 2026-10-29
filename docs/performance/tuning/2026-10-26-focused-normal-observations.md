# 2026-10-26: Focused normal-mode observations

## Question and source roles

Observe the selected paths with the system allocator and no matching/mapping
heap/work instrumentation. This follows the
[bounded selector adjustment](2026-10-26-bounded-keepall-row-heap.md) and
[preparation allocation reconciliation](2026-10-26-preparation-reuse-observation.md#source-reconciliation-2026-10-26).
No product code, fixture, native setting, cache bound or empirical threshold
changes for this observation.

Two comparisons are deliberately separate:

- Selector: retained source `36f0cafb02b5e350ea77a6570e68171d44674655` versus
  `c3050a805280c4315fba854d6984a2645b0ca0ba`, selecting
  `selection-k16-dense1080p-keepall`. The successor is the tested `9dd20c55`
  implementation plus documentation/evidence; PR114 merged as `7daec84c`, but
  the measured executable is not relabeled as that merge.
- Preparation: `8cc67f5a20a1e4726913cf4da6963b5d658c9e84` versus `36f0caf`,
  selecting `batch8-cold-roi320x180`. One operation is eight public searches;
  this does not isolate template compilation or the selector adjustment.

Three separate fresh normal Cargo roots use `template-matching-timing`, actual
Cargo JSON executable paths and real normal-mode attestations. The two retained
source exports use export attestation; the clean c305 checkout uses standard
checkout attestation. All instrumented artifacts remain immutable. There is no
later Cargo invocation in a frozen normal target.

Each role runs once through protocol-4 normal `smoke --workload`, one warmup and
three retained units, with a separate manifest process. Before-role inspection
precedes after-role launch, and the selector pair precedes preparation. Native
children retain90s/8MiB output/5s cleanup limits inside finite owned stages.
No host-idle admission, replacement sample, calibration, A/A or qualification
is selected. These are development observations, not accepted performance
profiles or a statistical speedup verdict.

## Apple Silicon observations

The [Apple record](../../evidence/p050-focused-normal/apple-observation.json)
binds all three builds/attestations, executable hashes, four unmodified raw
producer outputs and inspections. Every selected row has zero oracle failures,
correct warmup/retained results and successful cleanup. All26 unobserved
heap/work/owner fields are null, not zero. Actual OpenCV endpoints report TBB/10
before and after; that is not worker census or transitive image closure.

| Comparison | Before retained elapsed ns, in order | After retained elapsed ns, in order | Before retained-loop CPU ns | After retained-loop CPU ns |
| --- | --- | --- | ---: | ---: |
| Dense KeepAll | 134875 / 127083 / 129125 | 129875 / 124583 / 123542 | 824000 | 815000 |
| Cold ROI, eight searches | 28826792 / 28781041 / 28897875 | 28424708 / 28519250 / 28349916 | 463647000 | 472592000 |

Cold-ROI operation elapsed values are lower in this one process pair, while
retained-loop CPU is higher. The CPU bracket includes iteration preparation,
oracles and sample storage, not just the eight-search operation. Both facts
remain visible; the wall-time direction does not establish a joint CPU benefit.
The raw outputs retain operation CPU and separate RSS/footprint observations.
Post-drop RSS is not a Rust/native ownership or leak oracle.

All three Apple builds and attestations complete successfully. A post-build
data inspector's field-path error is retained separately; it reads the existing
successful cleanup receipt without rebuilding or rerunning a native command.

## Windows x64 MSVC observations

The [Windows record](../../evidence/p050-focused-normal/windows-observation.json)
binds three fresh builds, three actual attestations with15 provenance probes,
four raw selected rows and four manifest children. All ten inner/outer phases
complete successfully with known cleanup and restored ErrorMode. Every row
preserves exact result oracles, the system allocator and26 explicit null
counters. Actual OpenCV endpoints report ms-concurrency/20 before and after.

| Comparison | Before retained elapsed ns, in order | After retained elapsed ns, in order | Before retained-loop CPU ns | After retained-loop CPU ns |
| --- | --- | --- | ---: | ---: |
| Dense KeepAll | 195900 / 191800 / 194000 | 198800 / 191200 / 191200 | 0 | 0 |
| Cold ROI, eight searches | 25762600 / 20769300 / 21204900 | 20722200 / 20615900 / 21192200 | 78125000 | 62500000 |

Zero selector CPU is a quantized observation, not CPU-free work or a benefit.
Final Job CPU is complete within its owned-job scope, with unavailable effective
resolution; source units and1/0/0 accounting counts do not establish accuracy,
PID-role census or image closure. Cold-ROI post-drop RSS increases from14,114,816
to14,123,008 bytes in this pair. All other raw resource points remain available,
without selecting favorable metrics or comparing them across targets.

All352 transported packet payload hashes are verified. Fifteen environment-
bearing private originals remain local with explicit references and redacted
projections; this does not verify omitted original bytes. No native retry,
test rerun or additional workload is performed.

## Disposition and limits

Retain the selector implementation and bounded preparation design. Normal mode
preserves the selected public-result oracles with accounting absent. One fresh
process per role and three units do not establish repeatability, causal OS
resource differences, robust p95/CPU improvement or qualification. Do not pool
targets, substitute unavailable counters or infer application benefit from a
capture-free component row. Actual-consumer and explicit qualification claims
retain their own evidence requirements.
