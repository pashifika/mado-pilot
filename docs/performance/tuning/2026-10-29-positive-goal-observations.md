# 2026-10-29: Independent selection and preparation observations

## Question and decision

Retain the current selector and preparation implementation. This record adds
source-bound observations, not production changes or wider empirical ceilings.
It asks whether dense KeepAll selection and cold full-frame preparation meet
their separate latency/work goals under the [component method](../methods/component.md).
CPU remains an untrimmed diagnostic; latency, resource, semantics, provenance
and ownership checks remain independent requirements.

The Apple selected normal comparison passes. Apple preparation's same-baseline
A/A does not pass its frozen latency ceiling, so no preparation candidate
comparison follows. The Windows full instrumented corpus passes both roles;
its counters are descriptive, not normal timing. Prior turnover failures and
full-target/actual-consumer qualification gaps remain open.

## Comparable sources and methods

- Baseline: `2b3572440394cfa64b742880fa9e92a760b5021a`, a measurement-only
  apparatus successor preserving the historical repeated-full-map selector,
  dense copy and uncached preparation rules.
- Candidate and collector: `2cb8d4f576e696152ceafab507d28b61eb2d62f6`.
  The comparison spans whole source roles, not one isolated optimization.
- `66260dc6a8580ae4037b6994e7166af3c07095ac` is a docs-only applicability
  successor, not the measured executable revision. Original artifacts,
  attestations and runtime evidence remain separately identified.
- Selection uses20 warmups/200 samples per process. Preparation uses20/100,
  prospectively selected from whole-command cost, not candidate results.
  Calibration is six baseline observations; independent A/A and comparison
  each use six balanced AB/BA pairs. Manifest processes are additional.
- Normal mode uses the normal allocator, not work-accounting instrumentation.
  Selection times the production selector on the canonical1920×1080 dense
  score map with top16/KeepAll; scratch restoration and reference oracle are
  outside that clock. Preparation times eight sequential public Matcher
  searches per sample. Fixture/backend/key creation and teardown are excluded:
  this is not isolated template compilation or whole application startup.
- No CPU-idle threshold, quiet-host retry, affinity, boost or power setting is
  introduced. Failed cohorts are retained without replacement or regrading.

## Apple normal selection

Native `aarch64-apple-darwin` completes one calibration, one independent A/A,
one profile freeze and one comparison:35 benchmark attempts including five
manifest processes. All actual child/CLI/outer boundaries have known cleanup.

The [public projection](../../evidence/p050-positive-goals/apple-selection.json)
contains all12 comparison process scalars, exact executable/source/method/
profile/report hashes and numerical allowances. Full raw/started/owner/runtime
receipts remain in the private p050 evidence tree, not in this compact file.

| Median of six process observations | Baseline | Candidate |
| --- | ---: | ---: |
| Operation p95, ns | 33,378,458.5 | 138,229 |
| Peak RSS, bytes | 37,494,784 | 29,253,632 |
| Endpoint RSS growth, bytes | 24,576 | 32,768 |
| Endpoint RSS residual, bytes | 25,182,208 | 16,875,520 |
| Retained-loop CPU diagnostic, ns | 6,581,706,500 | 59,354,000 |

The selected comparison passes strict benefit, all individual latency/resource
ceilings, repeatability, order and drift. The frozen benefit allowance is
5,008,904ns. The RSS-growth increase remains visible; lower peak RSS does not
offset another metric. RSS growth/residual covers the complete measured process,
including retained harness storage, not only the cache.

Stock `status=passed` and `execution_status=completed` coexist with
`performance_qualified=false` and `normal_timing_verdict=incomplete`. This is a
selected production-selector benefit, not full corpus/platform qualification,
all selection fixtures, `matchTemplate`, capture or UI latency.

Separate instrumented1/3 observations include four iterations and setup/warmup.
Both roles allocate26 times; copied score bytes fall33,177,600→0, visited offsets
132,710,400→610,044 and peak counted Rust heap24,885,707→16,591,371B.
After-drop counted heap is1531B in both roles, an absolute value rather than a
zero-residual or complete native-leak claim. These are not the normal200-sample
counters.

## Apple preparation control

The [preparation projection](../../evidence/p050-positive-goals/apple-preparation.json)
retains six calibration and12 A/A process scalars. Calibration passes with
p95 range/median0.665569506%. Independent A/A completes all14 attempts with known
cleanup but has a complete stock numerical FAIL:

| Same-baseline A/A observation | p95, ns | Frozen ceiling, ns | Excess, ns |
| --- | ---: | ---: | ---: |
| Baseline process4 | 747,807,792 | 747,455,333 | 352,459 |

The ceiling is the calibration maximum742,540,458 plus its spread4,914,875.
This is failure to meet the frozen method using the same baseline in both
roles, not demonstrated candidate regression or an identified OS cause. No
profile, candidate comparison, retry or threshold adjustment follows. Later
stock aggregate checks after the first refusal are not represented as passed.

## Windows full instrumented corpus

Native `x86_64-pc-windows-msvc` runs each original instrumented role once:
one manifest plus57 workload rows per role,116 records/114 workload rows total.
Baseline inspection and retention precede current admission. Both stock reports
pass all authored semantic/own-format checks; all native records exit0 with
known cleanup. Each role preserves its own19 format-contract leaf differences.
No additional build, attestation, targeted duplicate smoke or held-window
observation is part of this corpus run.

The [Windows projection](../../evidence/p050-positive-goals/windows-instrumented.json)
retains both selected rows' scalar/resource/CPU observations and packet identity.
The receiver verifies40 delivered payloads, the12 unchanged source-publication
payloads, actual CLI/terminal/guard records and114 report summaries. The232
attempt/started originals remain Windows-local references, not receiver-byte-
verified raw data. Stock semantic evaluation, peer inspection and receiver
reference/arithmetic checks are distinct evidence layers.

| Instrumented1/3 work, four iterations including setup/warmup | Baseline | Candidate |
| --- | ---: | ---: |
| Selection allocations | 26 | 26 |
| Selection score-copy bytes | 33,177,600 | 0 |
| Selection visited offsets | 132,710,400 | 610,044 |
| Cold-full allocations | 486 | 435 |
| Cold-full BGR conversions | 32 | 4 |
| Cold-full mapping copies | 32 | 0 |
| Cold-full score-copy bytes | 261,700,992 | 0 |

Both final owner observations are correct. Resource and CPU scalars are retained
separately in the projection; physical work reductions do not substitute for
normal latency, independent resource limits or full native allocation coverage.
No Windows normal numerical result is claimed by this instrumented record.

Adverse cold-full observations are retained beside the reduced work:

| Cold-full diagnostic/resource observation | Baseline | Candidate |
| --- | ---: | ---: |
| Retained-loop CPU, ns | 1,828,125,000 | 2,171,875,000 |
| Private commit after retained samples, bytes | 26,955,776 | 33,021,952 |

## Verification and limits

Independent retained-data reviews find no issue in the Apple selection evidence
or Windows corpus reconciliation. The Apple comparison's raw quantiles, integer
allowances and all35 attempt/started bindings are independently checked; the
Windows review preserves its explicit omitted-raw limits. No native run is
repeated for review. Public projections exclude private host/environment values
and binaries. Existing turnover failures, unsupported consumer admission and
uncompleted full qualification remain unchanged.
