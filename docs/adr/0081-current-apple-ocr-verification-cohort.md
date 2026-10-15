# ADR 0081: Current Apple OCR verification cohort

- **Status:** Accepted host and measurement procedure; budgets and model admission pending
- **Date:** 2026-10-15
- **Resolves gate:** none; new-model admission under G-013 remains open
- **Supersedes:** ADR 0080's blanket macOS 27 exclusion, for prospective CPU OCR verification only

## Context

The operator selected the current macOS 27 workstation as the baseline for the
remaining OCR model verification. Its observed identity is macOS 27.0.1
(26A434), Apple M1 Pro, 32 GiB, `Pashifika-MBP.lan`. Windows will connect later.
The deployment floor remains macOS 26.5.2; historical macOS 26 qualifications,
capture/input support and hosted CI evidence do not move to this cohort.

The source `10ca533` debug diagnostic remains failed and unqualified. Both
reference and candidate emitted a full-width percent sign for the compact dense
fixture's ASCII percent sign. The candidate also missed or changed tooltip/status
characters and returned an unexpected `●` on the nonuniform negative image.
Its confidence was below 0.5, but the exact-count gate still failed. Host approval
cannot repair those errors or retroactively qualify the run.

The retained optimized build failed loading `serde_derive`: its Mach-O string
pool offset was `2329364` (`0x238b14`, remainder 4 modulo 8). This matches
[Rust issue 157750](https://github.com/rust-lang/rust/issues/157750): stripping
debug information can misalign a dylib. Signature verification had passed.

## Decision

The frozen [`host-policy-v1.json`](../../tools/ocr-models/host-policy-v1.json)
allows the exact current Apple CPU/thread-one cohort. The classifier uses the
precommitted cohort and approval, not an OS-major blacklist or a caller's boolean.
An absent or changed cohort is refused. Windows has no entry until its actual
host and approval are recorded. This is verification authority, not a model or
platform-feature qualification result.

Keep rustc 1.97.1, the deployment floor and benchmark runtime optimization
unchanged. For this Apple benchmark build, pass
`--config 'profile.bench.build-override.strip="none"'` to Cargo. This disables
stripping only for build scripts, procedural macros and their dependencies;
it does not disable benchmark optimization. Build into a fresh dedicated target
directory. Never patch, re-sign or rebuild retained artifacts.

### Reference-only budget precursor

Before a new candidate qualification, run exactly three fresh optimized native
G-004 reference processes on this cohort, serially. Use the existing
`onnx-cpu --model-admission` collector, unchanged G-004 v3 and compact-UI v1
fixtures, CPU threads one, two warmups and twenty samples for each of the 22
workloads. No candidate runs in this precursor. Bind the clean source, executable,
resources, corpus, criteria and approved cohort before launch; retain every raw
observation and independent parent process receipt.

Operational limits are 300 seconds per process, 33,554,432 output bytes per
process, 15 seconds for owned cleanup and 2,000 ms for each admitted interruption
probe. The whole three-process precursor has a 1,200-second bound, including
preflight and validation. Stop subsequent launches on an operational, input
identity, collector or lifecycle failure. Preserve quality failures without
repairing their text or removing observations; this precursor never qualifies a
reference or candidate.

Freeze the following derivation before collecting the precursor. For each
workload, use the largest corresponding metric among all three processes, not a
favorable process or an aggregate that hides one. Compute percentiles with the
existing report validator and retain its per-process summaries.

| Final Apple budget | Derivation from the complete optimized reference precursor |
|---|---|
| Workload p50/p95/max | 1.5 times the respective largest per-process statistic, rounded up to 1 ms |
| Cold open and close | Twice the respective largest process observation, rounded up to 1 ms |
| Process peak RSS | Twice the largest process peak, rounded up to 1 MiB |
| Workload Rust peak and steady bytes | Twice the respective largest observation, rounded up to 4 KiB, with a 4 KiB minimum |
| Workload Rust growth | 4,096 bytes; explicit adoption for this collector scope, not inherited qualification |
| Mapped bytes | Exact source width × height × 4 |
| Detector tensor bytes | Larger of the two declared resize plans' width × height × 3 × 4 |

The 50% latency and 100% initialization/memory headroom are prospective
operating ceilings, not measurements or promises of improvement. They are fixed
before the reference precursor and cannot be widened after seeing a candidate.
Negative growth/steady deltas do not become negative limits. Model and auxiliary
lengths remain exact resource identities, not estimates of native memory.

Required collectors remain `rust-accounting`, `native-peak-rss` and
`onnx-stage-clocks`. Rust accounting includes SDK work plus raw observation
construction/serialization; native peak RSS covers the entire fresh process.
No opaque allocation measurement or native steady-RSS claim is added. Derived
numeric budgets must be reviewed and committed before final qualification; a
missing or inconsistent precursor prevents that freeze.

### Final qualification boundary

Do not spend a candidate attempt while the Windows cohort/policy is unavailable.
Both target reports must name the same implementation revision/tree, and each
policy freeze must be its ancestor. Freeze both target policies before building
that final source. The retained adverse quality evidence requires an explicit
reviewed candidate-attempt disposition; this ADR authorizes the reference-only
precursor, not a candidate retry. No execution identity, oracle, quality gate,
support status, default, provider or watcher scope changes.

## Alternatives

- Keep rejecting macOS 27 despite the operator's decision: blocks the selected
  verification host rather than enforcing reviewed host identity.
- Accept any macOS 27 host or retrofit old runs: loses cohort and historical
  provenance. The exact frozen allowlist remains mandatory.
- Upgrade the compiler, change signatures or use debug timings: unnecessary
  mutation or invalid performance evidence when a build-only override suffices.
- Reuse old workload ceilings or derive limits from the candidate: changes the
  measurement scope or permits post-result admission tuning.

## Verification

The admission tests and actual validator CLI exercise a committed macOS 27
allowlist and reject an unapproved OS change. The isolated optimized catalog CLI
loads the corrected procedural macro and prints the unchanged execution digests.
The reference precursor retains raw samples, quality failures, collector scopes,
process receipts and its exact command/binary binding. It cannot close either
native model-admission gate. Independent Windows evidence and a reviewed final
candidate disposition remain required.
