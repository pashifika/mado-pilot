# ADR 0071: Scope OCR workload measurements to observable byte views

- **Status:** Accepted
- **Date:** 2026-09-13
- **Resolves gate:** none; G-013 numerical acceptance remains open
- **Supersedes:** none

## Context

The OCR watcher acceptance contract requires workload-specific timing, memory,
mapped bytes, retained bytes, disposition accounting and relevant startup cost.
The first prospective runner additionally required an unavailable total physical
mapping/allocation ledger. That field was always absent, so even complete
observable evidence could never satisfy measurement completeness.

A `CpuMapping` exposes its layout and bytes, not a physical allocation identity.
Full same-format CPU maps may share storage; crop/conversion can allocate.
Summing views cannot establish unique allocations. ONNX and platform resources
also have lifetimes that are not represented by those view extents.

The backend already supplies nondefault initialization-stage observation, and
the benchmark already samples target-native process memory. These seams are
sufficient for scoped measurements without a new product API or allocator layer.

## Decision

Keep these quantities separate:

| Measurement | Meaning |
|---|---|
| Backend-input mapped bytes | Sum of actual CPU view lengths arriving at the controlled backend boundary, including a mapped request rejected as Busy; the prior admitted-only counter remains distinct |
| Retained-accessor mapped bytes | Sum of actual mapping descriptor lengths returned to caller-retained frame access |
| Mapping cache high-water | Runtime-reported logical cache extent, not unique native allocations |
| Result/source/text/index and separate frame extents | Logical retained ownership extents; aliases are not additional physical storage |
| Process memory | Native RSS/peak plus Apple footprint or Windows private bytes at required checkpoints, including retirement |

No byte-view sum is labeled total physical memory. OS memory observations cover
process-native residency independently; sampled high-water and OS process peak
are not interchangeable, and neither attributes every native allocation.

Real cold startup uses a separate nondefault `ocr-text-watch-qualification`
feature forwarding the existing ONNX `benchmark-instrumentation` feature. The
normal facade and example add no observation callback or memory sampling. The
qualified example still uses the existing public CPU profile constructor.

The existing four ordered native-ready hooks establish runtime/provider and
one detector/recognizer construction. Observing exactly one of each session-ready
hook through query teardown proves no additional construction in that interval;
it is not a live-session count after destruction. Record engine/session readiness,
query terminal, logical close, physical OCR zero, parent release and retained-owner
release against one process-local monotonic origin. The supervisor independently
records whole-process termination and cleanup. ORT process-global residency may
remain until process exit.

Fixed structured rows are checked for order, count, field types and required
observations. Missing memory, absent qualification instrumentation, duplicate
stages or malformed measurements are non-passes. Measured values may be complete
without satisfying a numerical budget: these are independent decisions. First
process failures retain precedence over the missing telemetry they cause.

## Alternatives

- Require every native allocation's identity and lifetime: not the accepted
  contract and unavailable through current portable APIs; adds intrusive,
  platform-specific accounting solely to satisfy an invented gate.
- Treat summed mapping descriptors as allocations: false under shared mappings
  and separately retained aliases.
- Leave real startup as supervisor wall time only: cannot distinguish runtime,
  model construction, first query, logical close and physical retirement.
- Add default runtime counters or another OCR constructor: unnecessary public
  surface and steady-state cost for qualification-only observations.

## Consequences and verification

Prospective profile format2 and workload report schema3 name these scopes.
The fixed input, workload order, warmup/sample counts, process limits and
unaccepted numerical ceilings do not change. Controlled timings remain controlled
work, not measured real OCR throughput. The original unmeasured/failure records
remain revision-bound evidence.

Before use, model-free checks must reject missing/duplicate/out-of-order rows,
retain process-failure precedence, and prove valid observations cannot promote
unaccepted numerical qualification. The executable semantic workloads and
qualification-only stage recorder must also be exercised without claiming model
or native support. Actual CPU replay, native capture, separately authorized
precursors and an accepting `ocr-text-watch-workload-profiles` ADR remain required.
This ADR is not that numerical accepting record.
