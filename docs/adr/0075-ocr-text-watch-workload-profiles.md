# ADR 0075: OCR text-watch workload profiles

- **Status:** Accepted
- **Date:** 2026-09-13
- **Resolves gate:** [G-013](../validation-gates.md#g-013) for the fixed Rust OCR controlled and real-CPU startup workloads only; final enforcement and native capture remain unexecuted
- **Supersedes:** none

## Context

The Rust OCR watcher has complete, separately authorized precursors on both
release hosts: three controlled processes and five fresh real-CPU startup
processes per target. Each controlled process contains six workloads with two
warmups and twenty retained samples, plus one controlled startup sample.
The real process contains twelve ordered native-clock/memory stages and one
summary. No process is pooled with another to manufacture a percentile.

The [Apple profile](../benchmarks/ocr-text-watch-aarch64-apple-darwin.toml) and
[Windows profile](../benchmarks/ocr-text-watch-x86_64-pc-windows-msvc.toml) record
exact source/tree, artifact and host applicability and the corresponding worst
process-local observations. Their fixed input, geometry, cadence, process counts,
watchdogs and output bounds are unchanged.

| Target | Controlled execution source | Real-startup execution source | Eligible cohort |
|---|---|---|---|
| Apple M1 Pro, macOS 26.6.2 | `ac0eb23` | `69dd6c6` | controlled3; corrected-pipe real5 |
| Core i7-12700KF, Windows11/base build26200 | `2ff66d4` | `ac0eb23` | new controlled3; real5 |

Windows controlled evidence records25H2/build26200.9445. The earlier real
precursor records Windows11/10.0.26200 without a UBR; the current registry value
must not be attributed backward. Final execution must bind its own complete
current build. This historical precision limit does not make that Windows11 host
an older unsupported OS or change any retained source/image identity.

The earlier Windows controlled failure, Apple `SIGXFSZ` failure, and isolated
loader diagnostic are not samples. Windows's new cohort passed with eight
observed images each time: only canonical OS-system `apphelp.dll` was absent.
Every common hash and all nine declared disk identities still matched. The
[OS-managed presence policy](../ocr-text-watch.md#fixed-real-replay-procedure)
changes no historical result and requires no DLL or OS manipulation.

Private evidence is retained in the Change's 067/081, 078, 096 and 116 records.
116 preserves the new Windows metadata package, complete unit output and raw-file
inventories; full Windows benchmark streams remain on that host. Main independently
reconstructed all399 measurements and2391 ordered endpoints. Apple retained
stdout supplies its own399 measurements and2391 endpoints. Both real cohorts
supply five complete twelve-stage sequences. The tracked profiles carry the
condensed observations, not invented raw-output or allocation claims.

## Decision

Use the target-specific profile ceilings through the source-bound Python runner.
A complete measurement is not numerical qualification: every semantic, identity,
process, dependency and cleanup gate must pass before numerical comparison, and
only a complete separately authorized final target/mode cohort may pass.

### Derivation and scope

- Timing ceilings are twice the worst corresponding precursor process statistic,
  rounded upward to0.001ms. Normal controlled p50/p95 use nearest ranks10/19 from
  exactly twenty retained samples; the independent maximum uses all twenty.
  Controlled startup has one sample. A minimum0.001ms avoids a zero ceiling from
  finite clock resolution.
- Maximum ceilings have a1ms minimum. This applies to short control-path maximums
  and real stage-interval maxima, not to p50/p95. Sub-microsecond observations
  cannot establish an OS scheduling hard-real-time bound; typical-latency and
  whole-workload limits remain independent. This rule is selected before any
  final qualification, not in response to a failed final sample.
- For each cancellation endpoint, first take the larger mapping/inference value
  **within one invocation**, then compute that process's twenty-value p95 and
  maximum. Forty endpoint values are not forty independent samples.
- Native memory ceilings are1.25 times the target-specific maximum, rounded
  upward to1MiB. Include controlled warmups and every real stage. Current RSS,
  process-lifetime peak RSS, Windows private usage and Apple physical footprint
  remain separate; a lifetime peak is not an invocation increment. An unavailable
  platform counterpart is absent, never zero-filled.
- Input-view, cache, retained source/text/index, caller-frame and accessor-view
  ceilings use the fixed fixture's observed maxima, corroborated by the bounded
  frame/result/accessor script. These logical maxima agree on both targets.
  They are neither unique-allocation totals nor bounds on arbitrary callers.
  Cadence-dependent aggregate backend-view traffic remains reported evidence,
  not a fabricated aggregate performance ceiling.
- Physical OCR high-water is at most one. Logical close may still observe that
  one lease; its ceiling is one even though these precursors observed zero.
  Final physical OCR must be zero. Native-session construction counts do not
  establish session destruction or process-global ORT release.
- Stale/dropped-work correctness uses the existing fixed executable assertions:
  exact terminal/source authority, no late confirmation or diagnostic escape,
  finite pending/physical shape, preserved queue age and observable disposition
  accounting. Deliberate supersession and expiry are not required to be zero.
  No unmeasured numeric stale/drop or allocation-growth counter is invented.

Real intervals name their actual clock boundaries. In particular,
`query_terminal_to_logical_close_returned_ms` includes intervening result checks;
it is not isolated API-close latency. Supervisor time includes the Apple image
collector, while Rust intervals and memory exclude it. Fresh processes do not
mean cache-cold storage: ordinary identity hashing may warm files, and no cache
flush, priming run or retry is authorized.

### Enforcement

The OCR profiles retain format version2 and the established
`[[measurement]]` / `[[measurement.budget]]` absolute `at_most` representation.
Each target has seven controlled blocks and one separately scoped real-startup
block. The legacy Rust profile registry and its version1 hard-budget constants
are not reused as a second OCR enforcement authority.

`measurements.py` validates exact invocation/endpoint ordering and completeness.
`budgets.py` validates the closed target-specific metric sets and compares each
process independently. `workloads.py --enforce-budgets` additionally requires an
accepted profile and this accepted ADR, the exact approved host and executable,
canonical source-bound profile bytes and unchanged ADR/source/input/artifact
identities before every process and finally. A numeric failure stops subsequent
processes; a final identity failure cannot be hidden by successful comparisons.
Schema4 records target/mode scope and keeps measurement completeness distinct
from budget acceptance. Measurement-only execution cannot qualify a budget.

These Python changes do not rebuild or alter the pinned Rust producers. Added
comparison runs after child return; controlled internal clocks and native memory
do not include it. Changed source or binaries require reviewed applicability and
new exact execution bindings, not an inferred carry-forward from an old hash.

## Alternatives

- Borrowing old OCR/template ceilings would claim equivalence across different
  workloads, ownership and observer scopes; rejected.
- Pooling processes or mapping/inference endpoints would hide process-local
  tails and change the fixed sample count; rejected.
- Using strict multiples of microscopic maximums without a floor would turn a
  regression profile into an unsupported scheduler guarantee; rejected. P95
  remains sensitive to repeated control-path regressions.
- Parsing Rust `Debug` work records into a new numeric ledger, or rebuilding a
  producer solely to duplicate already emitted endpoints, adds an unnecessary
  protocol or artifact transition; rejected.

## Consequences

Integrators receive no new API, default, provider, native dependency or packaging
change. These are regression ceilings for the named fixtures and hosts, not OCR
throughput or arbitrary-application guarantees. Native WGC/ScreenCaptureKit
acceptance and macOS27.0 verification remain separate.

Changing a ceiling requires a new justified decision before final execution.
Failed, partial, skipped and unexecuted rows remain visible; there is no automatic
retry, sample replacement or promotion of precursor results.

## Verification

The Python suites cover malformed/missing/duplicate endpoint and budget records,
process-local quantiles and independent maxima, paired endpoint reduction,
warmup memory, target-specific missing observations, exact inclusive limits,
first-failure stop, final ADR mutation and measurement-versus-budget status.
The original69-case image-policy suite passed on both native hosts with only
opposite-platform skips. The integrated endpoint/comparator/runner suite passes
98 tests with4 Windows-only skips on Apple. Its initial synthetic `/var` root
failure was corrected in the fixture, without weakening the source-root guard.
Actual CLI admission refuses unaccepted limits before creating an output directory.
All eligible precursor records were consumed offline without rerunning a candidate.

Independent code review found no defects. Independent arithmetic review
reconstructed all808 process-local values and verified all248 target observations,
limits and units. Its Windows OS-provenance finding was corrected and independently
closed without changing measurements or ceilings. The accepted profiles and this
decision are separate from final fixed-cohort enforcement, which has not run.
