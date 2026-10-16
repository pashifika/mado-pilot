# ADR 0082: Current Windows OCR reference precursor

- **Status:** Accepted host and prospective procedure; numerical budgets and model admission pending
- **Date:** 2026-10-15
- **Resolves gate:** none; new-model admission under G-013 remains open
- **Supersedes:** ADR 0081's Windows readiness deferral only

## Context

The operator connected `win-worker` and required ordinary branch operations in
its existing project root, not a new worktree. Readiness at source
`50f258d7d2bd6676a6b4a817d865cedada433d27` confirms the same-root fast-forward,
Windows 11 Pro 26H2 build 26300.9550, Core i7-12700KF and 32 GiB. Its complete
32,420-byte packet is retained by the Change with SHA-256
`be51371eb07a0313bccd20b579460f2c3a6affdd3c432aebd09e016112e868b8`.
No build, native load, model initialization or inference occurred in readiness.

The two legacy model resources match the catalog. The three candidate paths are
absent from the inspected controlled root. OpenCV 4.14.0, MSVC and ONNX Runtime
1.29.0 metadata are present; actual runtime API-17/CPU compatibility is not yet
verified. A header declaring API 29 does not prove or disprove `GetApi(17)`.

## Decision

Approve only the prospective CPU/thread-one cohort below through
[`host-policy-v1.json`](../../tools/ocr-models/host-policy-v1.json):

| Binding field | Value |
|---|---|
| `cohort_id` | `windows-i7-12700kf-32g-26300-ocr-cpu-v1` |
| `target` | `x86_64-pc-windows-msvc` |
| `os` | `Windows-11-10.0.26300-SP0` |
| `cpu` | `Intel64 Family 6 Model 151 Stepping 2, GenuineIntel` |
| `host_id` | `PASHIFIKA-PC` |
| `runtime` | `1.29.0-api17` |
| `provider` | `cpu` |
| `intra_op`, `inter_op` | `1`, `1` |
| `approval` | `ADR-0082` |

The existing runner's exact OS/CPU strings are binding fields. Edition, UBR and
marketing hardware names remain recorded provenance, consistent with ADR 0061;
no second host-matching implementation is introduced. Historical Windows 25H2
and Apple evidence, deployment floors, capture/input and watcher support do not
change. Host approval never substitutes for successful runtime or model gates.

### Build and metadata preparation

Use the same clean checkout and a fresh dedicated `CARGO_TARGET_DIR`, `TEMP`,
`TMP` and report directory under `.rasen/`. Do not rebuild retained artifacts.
Select installed x64 MSVC 14.44.35207 and SDK 10.0.26100.0 in a fresh
noninteractive Developer PowerShell child, with explicit OpenCV/libclang roots.
Omit profiles and PSReadLine; preserve existing history and audit logs while
retaining command/output evidence. Do not install, upgrade or change global settings.
The standard `tools/setup-native.py -- ...` wrapper may compile/load its native
prerequisite probes during this stage. It is not a model probe.

Build the optimized `onnx-cpu` benchmark once with `--offline --locked`,
`--features benchmark-instrumentation` and `--no-run`. Windows does not require
the Apple procedural-macro strip override. Bound the build child to 1,800 seconds,
16 MiB captured output and 30 seconds for owned cleanup; bound the complete
preparation stage to 2,100 seconds. Retain a failed build without automatic retry.
Only metadata-mode execution (`--model-admission --describe-profile`) follows a
successful build, with 30-second command, 4 MiB output and five-second cleanup bounds. Record
actual executable digest, embedded source/tree/compiler, OpenCV, original output
and parent receipts. This stage never initializes models or runs inference.

### Reference-only budget precursor

Before launch, freeze the actual clean source/tree, optimized executable,
runtime/resources, host policy, prepared fixtures, collector script and exact
commands. The runtime DLL must retain the inspected SHA-256
`69d8e6d3879a3b4001cdc74c8ed9ccc7e7f799a5b847059738323404519ec471`.
A separate reference-only orchestration invokes the existing Rust
`--model-admission` collector and shared bounded process runner. The ordinary
`tools/ocr-models/run.py run`, including `--diagnostic`, runs both profiles and
is not this precursor.

Run exactly three fresh, serial native G-004 processes, using the unchanged
11-image G-004 v3/compact-UI v1 corpus and all 22 singular/grouped workloads.
Retain two warmups and twenty measured observations per workload/process.
CPU intra/inter-op threads are one. Required collectors are `rust-accounting`,
`native-peak-rss` and `onnx-stage-clocks`; Windows native peak is
`GetProcessMemoryInfo`'s `PeakWorkingSetSize`, not opaque heap or steady RSS.

Adopt prospective operational bounds of 300 seconds and 33,554,432 output bytes
per process, 15 seconds for owned cleanup, 2,000 ms for each admitted interruption
probe and 1,200 seconds for the whole precursor. Before each launch require the
full process/cleanup allowance plus 30 seconds for validation to remain.
Stop later launches on operational, source/input, collector or lifecycle failure.
Retain every quality failure without rewriting or removing it; quality failure
does not hide the reference's resource observations. An existing attempt is not
rerun. No candidate process or resource acquisition belongs to this precursor.

Independently apply ADR 0081's predeclared derivation to the complete Windows
observations: largest per-process p50/p95/max times 1.5, rounded up to 1 ms;
largest cold-open/close times two, rounded up to 1 ms; largest native peak times
two, rounded up to 1 MiB; largest workload Rust peak/steady times two, rounded up
to 4 KiB with a 4 KiB minimum; Rust growth ceiling 4,096 bytes. Mapped bytes are
exact source width × height × 4; detector bytes are the larger of the two declared
resize plans' width × height × 3 × 4. Use the existing percentile implementation.
These are prospective headroom rules, not copied Apple measurements or ceilings.

Review and commit the resulting Windows numerical budgets before building a
common final source for both targets. Missing or inconsistent observations block
that freeze. The precursor is always `qualification_eligible: false`; it cannot
rate the reference, admit the candidate or close either final native gate.

## Alternatives

- Reuse Apple's numbers or old Windows workload budgets: wrong cohort or scope.
- Use `--diagnostic` as reference-only: it also attempts the candidate.
- Acquire missing candidate assets before reference calibration: unnecessary for
  this stage and not a remedy for the preserved adverse quality evidence.
- Create another worktree or change old binaries: violates the operator's root
  constraint or loses provenance. Dedicated output directories are sufficient.

## Consequences

Candidate assets, final runtime observations, both numerical policies and a
reviewed candidate-attempt disposition remain required for final qualification.
The old failed candidate and reference rows stay nonpassing; no oracle, model ID,
support flag, default, provider or watcher scope changes. A failed precursor
requires a new explicit prospective decision, not a widened limit or hidden retry.

## Verification

The retained readiness packet's digest and six source-file hashes were checked
against `50f258d`; its five resource declarations match the source catalog.
The actual validator CLI accepted the new Windows entry and unchanged Apple
entry, and refused an unapproved Windows host and OS string. These four checks
used explicitly synthetic reports and budgets, not native measurements. Apple
budget bytes, catalog and criteria remained unchanged. Build, runtime loadability,
collectors and precursor outcomes require their own original execution receipts;
readiness and policy smoke are not that proof.
