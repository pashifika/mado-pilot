# Benchmark profiles and budgets

Later phases of MadoPilot introduce performance-sensitive behavior: capture,
frame mapping, template matching, OCR, watcher scheduling, and acceleration. Each
of those phases must be able to show that its behavior is both correct and within
an agreed cost before the phase exits.

This document defines the format that evidence takes. Setting a numeric budget
for a workload is gate [`G-013`](validation-gates.md#g-013), which is resolved
per workload and target rather than once. Phase 1 is resolved. ADR 0021
invalidated the three historical macOS Phase 2 native profiles after source and
correctness-oracle drift. ADRs 0024–0032 accept the current target-specific
Phase 2 diagnostic, native input, controlled owning-process, production capture,
transition, and corrected dual-4K profiles while preserving each historical
source identity.

[ADR 0037](adr/0037-phase-3-ocr-performance-budgets.md) accepts target-specific
Apple M1 Pro and Core i7-12700KF default-OCR profiles. Each uses five precursor
processes and a separate five-process post-budget run for cold accepted-model
startup, warm full-frame and bounded-region OCR, empty results, exact source
correlation, allocation growth, observed mapped bytes/inference/session/result
counts, cleanup, close, and target-native resident high-water. A separate native
contract test covers cancellation, late-publication suppression, recovery, and
close races without mixing that instrumentation into latency samples. Hosted
Windows Server smoke enforces hard correctness and growth only; its timing and
resident observations define no release-host budget.

Nothing in this document is itself a measured result. The numbers live in the
profiles under [benchmarks/](benchmarks/), each naming the host it was measured
on, and the example this document references records no measurement at all.

## Where benchmark files live

A phase commits one file per **run** under `docs/benchmarks/`, named
`<phase>-<workload-set>-<target>.toml`. A measured file holds exactly one
`[profile]` and one `[[measurement]]` block per workload, each with its own
budgets, so that a budget is never separated from the conditions under which it
was measured. A run that could not be performed uses one explicit
`[measurements]` gap record with `performed = false` and carries no budget.

One file per run rather than one per workload, because the profile describes the
run and not the workload: a set of eight workloads measured together on one host
shares one fixture hash, one target, one machine, one build, and one sample
count. Eight files would carry eight copies of that and eight chances for them to
disagree. What must never be shared across a file is the *target* — a budget is
valid only for the target in its profile — which is why the target is in the
name.

The format example is
[benchmarks/example-synthetic.toml](benchmarks/example-synthetic.toml). It is a
format demonstration only: it is marked non-normative, it records no measurement,
and its numbers are illustrative.

## Format version

Every file declares `format_version`. The current version is `1`. A change that
alters the meaning of an existing field increments the version and records the
migration in an architecture decision record; adding an optional field does not.

## Benchmark

A file opens with the `[benchmark]` block, which says what was run and what the
file is for. The harness prints this block and a committed profile carries the
same keys with two answers changed, so that a recorded profile is the harness's
output with budgets added rather than a document assembled beside it.

| Field | Required | Meaning |
|---|---|---|
| `id` | yes | The identifier the file is filed under, matching its name. |
| `workload` | yes | One sentence naming what the set of workloads covers. |
| `phase` | yes | The phase that introduced them. |
| `status` | yes | `measured` for a recorded run, `harness-output` for the printer's own output, `format-example` for the format demonstration. |
| `normative` | yes | Whether anything gates on this file. Harness output and the format example do not. |
| `measurements_recorded` | yes | Whether the numbers are readings. False only for a file whose numbers are illustrative. |

`crates/support/testkit/tests/benchmark_block_drift.rs` compares the harness's
keys against every committed profile's, because a key that only one of them
carries is a file no reader can turn into the other.

## Profile

A profile records the conditions that make a measurement reproducible. A budget
without a profile is not evidence, because the same number can pass or fail
depending on the fixture, the host, and the queue policy.

| Field | Required | Meaning |
|---|---|---|
| `fixture` | yes | Tracked path of the recorded frame sequence, template set, or model input the workload replays. |
| `fixture_sha256` | yes | Hash of the fixture, so a silent fixture change invalidates the evidence. |
| `release_target` | yes | Target triple the measurement ran on. A measurement is valid only for its own target. |
| `hardware` | yes | Host machine identification, including CPU, memory, and any accelerator used. |
| `os_version` | yes | Operating-system version string of the host. |
| `build_profile` | yes | Cargo profile and any feature selection that affects code generation. |
| `warmup_iterations` | yes | Iterations discarded before sampling, so first-run costs are separated from steady state. |
| `sample_count` | yes | Number of retained samples. |
| `correctness_oracle` | yes | How each sample's output was checked against the expected result. A latency number without a correctness check is not acceptable evidence. |
| `queue_policy` | yes | Queue depth and drop policy in effect, because latency and stale-work counts are meaningless without it. |
| `notes` | no | Anything else a reader needs in order to reproduce the run. |

## Measures

A budget names one measure. The version-one vocabulary is:

| Measure | Unit | Notes |
|---|---|---|
| `capture_to_result_latency_p50` | milliseconds | End-to-end latency percentile from frame capture to committed result. |
| `capture_to_result_latency_p95` | milliseconds | As above, at the 95th percentile. |
| `peak_memory` | bytes | Peak resident memory during the run. |
| `peak_resident_bytes` | bytes | High-water resident set of the measured native benchmark process or a separately measured child process, reported by the native operating system. Optional because workloads without a native process observation do not emit it. |
| `steady_memory` | bytes | Resident memory in steady state. |
| `mapped_bytes_per_result` | bytes | Frame bytes mapped into CPU memory per produced result, full-frame or region of interest. |
| `stale_work_ratio` | ratio | Share of scheduled work that was dropped, coalesced, superseded, rejected, queue-expired, or discarded as stale. |
| `model_load_time` | milliseconds | Time to load and initialize an OCR model, including provider selection. |
| `startup_time` | milliseconds | Time from process start to a usable session. |
| `result_correctness` | count | Retained samples whose output disagreed with the correctness oracle. A hard gate, never a tuned ceiling. |
| `memory_growth` | bytes | Signed change in resident memory across the sampled run, so a decrease is negative. A hard gate: unbounded growth is a defect, not a slow result, and its predicate bounds growth rather than demanding an exact zero. |
| `latency_p50` | milliseconds | Median of the per-iteration samples for one workload. |
| `latency_p95` | milliseconds | The 95th percentile of the same samples. Distinct from `capture_to_result_latency_p95`, which is end-to-end from capture to committed result rather than one operation. |
| `latency_max` | milliseconds | Slowest retained per-iteration sample for one workload. Frozen scenario bounds use it so one outlier cannot hide behind passing percentiles. |
| `iteration_span_ms` | milliseconds | One clock reading across the whole sampled run, divided by the sample count. It covers everything an iteration does, including the correctness check, so it is an upper bound on the operation rather than a reading of it. Use it where a per-iteration percentile is not expressible; see below. |
| `peak_allocated_bytes` | bytes | High-water mark of live heap bytes during the sampled run, above what was live before the workload's fixture existed. |
| `steady_allocated_bytes` | bytes | Live heap bytes when the sampled run finished, above the same baseline, with the fixture still alive. |
| `allocated_growth_bytes` | bytes | Signed change in live heap bytes across the sampled run alone. A hard gate, on the same terms as `memory_growth`. |
| `copied_bytes_per_result` | bytes | Producer-surface bytes copied while producing one retained sample. The report keeps the largest sample, so unexpected duplicate copies remain visible. |
| `detached_textures_peak` | count | Maximum simultaneously live Adapter-owned detached textures during one workload. |
| `staging_textures_peak` | count | Maximum simultaneously live CPU-readable staging textures during one workload. |
| `gpu_resources_peak` | count | Maximum simultaneously live producer, detached, and staging textures during one workload. |

A phase that needs a measure outside this list adds it here in the same change,
with its unit and its meaning.

### Why some names carry their unit and others do not

The suffix is not decoration and it is not applied evenly, so the rule is worth
stating rather than inferring. A name carries its unit when the quantity would be
ambiguous without it — `iteration_span_ms` is a duration and `_ms` says which
one, `peak_allocated_bytes` counts bytes and `_bytes` separates it from a byte
*rate* — and omits it when the `Unit` column above is the only answer the measure
can have. `latency_p95` is milliseconds because every latency here is.

Four vocabulary names differ from the key a profile records the value under,
which is the one place a reader can be caught out:

| Vocabulary name | Recorded as |
|---|---|
| `latency_p50` | `latency_p50_ms` |
| `latency_p95` | `latency_p95_ms` |
| `latency_max` | `latency_max_ms` |
| `memory_growth` | `allocated_growth_bytes`, when the measure is live heap rather than resident memory |

A budget's `measure` may name either form; committed profiles use the recorded
key everywhere except `latency_p50`, `latency_p95`, and `latency_max`, where
they use the vocabulary name. Renaming to one convention would move every
committed profile, the harness that prints them, and the drift test that
compares the two, so the mapping is documented instead.

### Live heap bytes and resident memory are different measures

`peak_memory`, `steady_memory`, `memory_growth`, and
`peak_resident_bytes` are resident-memory measures.
`peak_allocated_bytes`, `steady_allocated_bytes`, and `allocated_growth_bytes`
are live heap bytes, counted by a global allocator the benchmark installs. They
are separate entries rather than a redefinition of the first group, because a
budget written against one of them does not mean the same thing against the
other and a reader must never have to guess which was measured.

Phase 1 uses the heap measures. Resident memory is read through a different
platform API on each release target, it moves with allocator and
operating-system behaviour that no change to this project can affect, and on a
workload of that size the noise is larger than the signal. Phase 2 adds
`peak_resident_bytes` only for fresh C and C++ child processes whose loaded
native footprint is invisible to the Rust harness allocator; their
`peak_allocated_bytes` remains explicitly harness-side. Live heap growth still
answers whether a repeated in-process operation gives back what it took.

### When a per-iteration latency cannot be measured

An operation faster than the host clock can express reports zero, and a
percentile over zeros is not a budget. On `x86_64-pc-windows-msvc`,
`map_full_frame` measures exactly zero: a mapping whose requested format already
matches the frame's is shared rather than copied, so the operation is a
reference-count increment.

A workload in that position is bounded three ways instead, and a latency ceiling
is not one of them:

- `mapped_bytes_per_result`, which is exact, target-independent, and bounds the
  behaviour that actually costs something;
- `iteration_span_ms`, which recovers a number the clock can express by reading
  it once across hundreds of iterations;
- the hard memory and correctness gates, which apply to every workload.

Recording that a measurement was zero is not the same as recording that a
workload is free. A profile says which of these applies and why.

## Budget kinds

A budget is one of three kinds. A file may contain any mix of them.

### `hard`

A pass or fail requirement that is not a number to be tuned. Correctness and
bounded resource use are hard gates: a run that violates one has produced an
incorrect result, not a slow one. A hard budget states a `requirement` in prose
and a `predicate` that the benchmark harness evaluates.

A hard budget failure fails the phase. It is never relaxed to accommodate a
measurement; the behavior is fixed instead.

Name a hard budget after what it gates — `result_correctness` or `memory_growth` —
rather than after a latency measure. A correctness gate attached to a latency
measure reads as a latency budget and invites a later phase to tune it.

### `absolute`

A numeric ceiling or floor for a measure, with a `unit`, a `limit`, and a
`direction` of `at_most` or `at_least`. Use an absolute budget when the acceptable
cost is a property of the product rather than of a particular machine.

### `relative`

A ratio against a named baseline profile, with `baseline_id` and `max_ratio`. Use a
relative budget when the absolute cost depends on the host but a regression is
still unacceptable — for example, when a change must not make a measure more than
ten percent worse than the recorded baseline on the same target.

A relative budget requires that the baseline it names exists in a tracked
benchmark file for the same release target and fixture hash.

### Which side evaluates a budget

The kind decides that, and the two answers are different because the two kinds
claim different things. A hard budget is a structural property that holds on any
host, so the benchmark harness evaluates its predicate in process on every run —
both the run that produces timings and the reduced run the ordinary test
sequence performs — and a violation fails that run. An absolute or relative
budget is a per-target regression ceiling measured on named hardware, so only a
run on that hardware can evaluate it: whoever performs the run compares it
against the committed profile for the matching release target.

Continuous integration therefore reports correctness and bounded memory on both
release targets and evaluates no timing ceiling. That is deliberate: a hosted
runner's timings vary by more than a regression ceiling is worth, so a ceiling
evaluated there would teach a reader to re-run rather than to investigate.

## Where a budget attaches

A `[[budget]]` at the top level of a file applies to every measurement in it.
A `[[measurement.budget]]` applies to the one measurement it sits under. Use the
first for the gates that are true of the whole run — correctness and bounded
memory — and the second for anything whose number depends on the workload.

A workload that omits a measure's budget is stating that the measure does not
bound it. That is a decision the file should explain, not a gap: a latency
ceiling left out because the operation is faster than the clock says something
different from one left out because nobody looked.

## Rules that apply to every budget

- A budget is set, raised, or lowered only with a measurement recorded in the same
  change, plus an architecture decision record explaining the number.
- A budget is valid only for the release target in its profile. A result from one
  target never satisfies a budget for the other.
- A higher capture rate is not an improvement when it increases stale work,
  increases memory, or produces incorrect results. Correctness and bounded memory
  outrank throughput.
- The correctness oracle is reported alongside every latency and throughput
  number. A benchmark that does not check its output is reported as a timing
  experiment, not as evidence.
- When a run is not performed, the file says so explicitly rather than omitting the
  measurement section.

## Phase 0 status

Phase 0 delivers this format and the synthetic example. It sets no numeric budget,
records no measurement, and makes no performance claim. Each later phase populates
the format for the workloads it introduces, under gate
[`G-013`](validation-gates.md#g-013).

## Phase 1 status

Phase 1 delivers the harness, the correctness oracles, and **the first numeric
budgets this project has set**. The Phase 1 workloads of
[`G-013`](validation-gates.md#g-013) are resolved by
[ADR 0008](adr/0008-phase-1-performance-budgets.md); the gate stays open for
every later phase.

There are two benchmarks, each committed for both release targets:

| Benchmark | Profiles | Covers |
|---|---|---|
| `deterministic-slice` | [aarch64](benchmarks/phase-1-deterministic-slice-aarch64-apple-darwin.toml), [x86_64](benchmarks/phase-1-deterministic-slice-x86_64-pc-windows-msvc.toml) | The eight-operation Rust workflow |
| `c-boundary` | [aarch64](benchmarks/phase-1-c-boundary-aarch64-apple-darwin.toml), [x86_64](benchmarks/phase-1-c-boundary-x86_64-pc-windows-msvc.toml) | What the C ABI costs, against the same work through the facade |

Every profile was measured on a named host, two hundred samples per workload
after twenty warm-up iterations, every sample checked against its oracle, zero
oracle failures anywhere. Each benchmark's two profiles share a fixture hash.

ADR 0011's replay-reservation change is measured in all four profiles above at
the same reviewed executable source state, `8d7973e`. Each benchmark ran twice
on each named native host with the second run recorded. Every budget passes,
fixture hashes match within each benchmark pair, and correctness and allocated
growth are zero everywhere.

On Windows, `replay_open` p95 is `0.000900 ms` under the unchanged `0.003 ms`
ceiling and peak live heap is `94,702 bytes`, down from the regressed `119,062
bytes`. On Apple Silicon, p95 is `0.000750 ms` under the unchanged `0.0025 ms`
ceiling and peak live heap is `95,118 bytes`, back at the pre-copy shape. The
paired C-boundary runs also remain inside every retained ceiling.

The refreshed profiles explain deterministic allocation changes since ADR 0008.
In particular, the archive-file workload now carries the immutable snapshot
copy accepted by ADR 0010, while small asset, vision, and C-boundary allocation
shifts remain far below the existing ceilings. Timing variation is recorded as
measured rather than presented as a deterministic effect.

A refreshed profile keeps the ceilings ADR 0008 set. Each rationale therefore
names the baseline value the ceiling was derived from rather than the current
measurement, so a refresh cannot quietly relax a budget by re-deriving it.

Across those four files, thirteen workloads are measured, all thirteen are
covered by the two file-level hard gates, eleven carry a per-measurement
ceiling, and two are deliberate unbudgeted controls. The two controls are
`engine_create_rust` and `match_warm_rust`: each exists so its C counterpart can
be compared against it in one process, and the Rust workflow's own ceilings are
in the `deterministic-slice` profile for the same target, so a second set here
would be the same claim measured twice and free to disagree with itself.

The scaffolding both share — the sampling loop, the allocation accounting, and
the report — is `mado_pilot_testkit::bench_harness`, so the format described
above has one printer rather than two that can drift.

### The Rust workflow

A bench target of the `mado-pilot` package, at
`crates/mado-pilot/benches/deterministic-slice.rs`, covering eight workloads:

| Workload | What it measures | Correctness oracle |
|---|---|---|
| `replay_open` | Discovering targets and opening a session | The session reports the source's own extent and pixel format |
| `map_full_frame` | Mapping a whole frame to CPU-readable bytes | The mapping covers the whole frame and reports its exact source identity |
| `map_region_of_interest` | Mapping one region of a held frame | The mapping covers the requested region and no more |
| `load_package_directory` | Loading and validating a directory package | The package declares its six tracked templates |
| `load_package_memory` | The same package, described in caller-owned memory | The committed package equals the one the directory commits |
| `load_package_archive` | The same package, as a ZIP archive | As above |
| `prepare_and_match_cold` | Compiling a template and searching with it | Both planted copies are found at their planted offsets, scoring `1.0` within `1e-5` |
| `match_warm` | Searching with an already-compiled template | As above |

The three loading workloads use the `G-014` tiny package rather than the slice
package, because it is the only package tracked in more than one form. Same
bytes, same six templates, three containers: that is what makes the three
numbers comparable, and `mado-pilot-assets` already asserts that all three
commit the same package, so a fixture that drifted fails a test before it
reaches a benchmark.

Every sample is checked against its oracle, including in the run that produces
timings, because a latency number whose output was never checked is a timing
experiment rather than evidence.

The harness has two modes so the oracles can run far more often than the
timings. `cargo test --locked --workspace --all-targets` executes it with three
samples per workload and fails on any oracle violation, which puts the whole
workflow's correctness into the ordinary verification sequence. A timing run is
explicit, and states its host because the program does not guess one:

```sh
cargo bench --locked --package mado-pilot --bench deterministic-slice -- \
    --hardware "Core i7-12700KF, 20 threads, 32 GiB" \
    --os-version "Windows 11 Pro 10.0.26200 build 26200"
```

The release target is the one condition the program does state for itself, and
it selects rather than detects it: `std::env::consts` can report the
architecture and the operating system but not the vendor or the ABI, and a
triple assembled from the parts that are available would be a guess printed
where a measurement condition belongs. Anything that is not one of the two
declared release targets says so.

Turning a run into tracked evidence means adding the budgets and committing it
under `docs/benchmarks/`, in the same change that sets them.

### The C boundary

A bench target of the `mado-pilot-capi` package, at
`crates/bindings/capi/benches/c-boundary.rs`. Each workload that has a Rust
equivalent is measured twice, in one process and one run, so the difference
between a pair is the boundary rather than the conditions.

| Workload | What it measures |
|---|---|
| `negotiate_table` | `madopilot_get_api`, the whole of what a C caller pays before it holds anything |
| `engine_create_c` / `_rust` | Building an engine over the deterministic scene |
| `match_warm_c` / `_rust` | One search with a prepared template, and reading every match back |

The answer is that the boundary costs a fixed amount per entry rather than a
proportion of the work: negotiation does not register on either host's clock,
engine creation costs a sub-microsecond constant more, and a warm match costs
0.1% more on `aarch64-apple-darwin` and 3.4% more on `x86_64-pc-windows-msvc`
across four table calls. What makes the boundary material is therefore the
number of crossings, not the size of the work behind them; see
[ADR 0008](adr/0008-phase-1-performance-budgets.md).

Dynamic loading is not covered. The benchmark links the library, so the
`LoadLibrary` or `dlopen` a real C host performs once at startup is outside
every measured window.

### The finding the budgets are shaped around

On `x86_64-pc-windows-msvc`, `map_full_frame` measures **zero**. The mapping is
shared rather than copied when the requested format already matches the frame's,
so the operation is a reference-count increment, and the Windows monotonic
clock's granularity is coarser than that. Its median is exactly `0.000000` and
its 95th percentile is exactly one 100-nanosecond tick.

That workload therefore has no latency budget on either target. It is bounded by
`mapped_bytes_per_result`, which is exact and target-independent, and by
`iteration_span_ms`, which recovers a number by reading the clock once across
two hundred iterations. `negotiate_table` is bounded the same way for the same
reason, and so is any later operation whose fast path is a pointer copy.

## Phase 2.2 diagnostic performance

The common diagnostic hot paths have one benchmark at
`crates/automation/runtime/benches/diagnostic-overhead.rs`. Its ten workloads
measure one-event input submission, retained-frame acquisition/mapping, and
explicit close/drain with diagnostics `Off`, `Normal`, and `Debug`, plus a
four-slot debug queue under input pressure. Every sample proves its frame,
mapping, receipt, or close result is unchanged, validates retained categories
and order, checks exact loss counts, and records mapped bytes where applicable.

The benchmark's smoke plan runs under
`cargo test --locked --workspace --all-targets`, so both release-target CI jobs
enforce zero oracle failures and bounded allocation growth. A full profile is
run with the target's named host metadata:

```sh
cargo bench --locked --package mado-pilot-runtime \
    --bench diagnostic-overhead -- \
    --hardware "<named host hardware>" \
    --os-version "<named host OS and build>"
```

[ADR 0024](adr/0024-input-diagnostic-performance-budgets.md) accepts the
[aarch64 profile](benchmarks/phase-2-input-diagnostic-overhead-aarch64-apple-darwin.toml).
[ADR 0026](adr/0026-windows-native-and-diagnostic-performance-budgets.md)
replaces the Windows timing gap with the measured
[x86_64 profile](benchmarks/phase-2-input-diagnostic-overhead-x86_64-pc-windows-msvc.toml).
Both profiles retain 200 samples after 20 warmups for every workload, report
zero oracle failures and allocation growth, cap live Rust heap at 32 KiB, and
keep the capture mapping exact at 3,072 bytes.

On the Apple M1 Pro, p95 `Normal` input diagnostics add `0.000042 ms` over
`Off`, `Debug` adds `0.000125 ms`, debug capture/mapping adds `0.000126 ms`,
and debug close/drain adds `0.000209 ms`. On the Windows Core i7-12700KF,
the corresponding post-review differences are `0.000200 ms`, `0.000200 ms`,
`0.000200 ms`, and `0.000300 ms`. Four submissions against capacity four
retain all four normal terminal records, report all eight discarded debug
records, and still return four complete receipts on both targets.

## Phase 2 native performance status

Phase 2's affected [`G-013`](validation-gates.md#g-013) production and
target-specific native profiles are accepted. Windows final-source Phase 1
reruns pass on the exact exit candidate; Apple Silicon runs remain attributed
to `d8336be` and apply by reviewed complete diff. Both keep their unchanged
ceilings.
[ADR 0021](adr/0021-invalidate-phase-2-native-performance-evidence.md)
invalidated the three macOS profiles originally accepted by
[ADR 0020](adr/0020-phase-2-native-performance-budgets.md). ADR 0025 replaced
the macOS input profile, while ADR 0026 accepts the controlled Windows native
profiles:

| Workload set | Target | Profile | Current status |
|---|---|---|---|
| Capture | macOS | [aarch64](benchmarks/phase-2-native-capture-aarch64-apple-darwin.toml) | historical, non-normative after source and oracle repairs |
| Capture | Windows | [x86_64](benchmarks/phase-2-native-capture-x86_64-pc-windows-msvc.toml) | measured and normative under ADR 0026 |
| Transitions | macOS | [aarch64](benchmarks/phase-2-native-transitions-aarch64-apple-darwin.toml) | historical, non-normative because it names the superseded tree |
| Transitions | Windows | [x86_64](benchmarks/phase-2-native-transitions-x86_64-pc-windows-msvc.toml) | measured and normative under ADR 0026 |
| Input and public languages | macOS | [aarch64](benchmarks/phase-2-native-input-aarch64-apple-darwin.toml) | measured and normative under ADR 0025 |
| Input and public languages | Windows | [x86_64](benchmarks/phase-2-native-input-x86_64-pc-windows-msvc.toml) | measured and normative under ADR 0026, extended by ADR 0028 |

The two historical macOS files retain their old samples, environment metadata,
and former budget blocks with `normative = false`; they do not gate current
source. The macOS input requalification at final candidate `dec43d7` retained
300 correct samples with maximum allocation growth 64 bytes under the
4,096-byte hard gate. The harness provisions each C/C++ sample's fresh
approved fixture outside its timed span and retains controller-owned
mode-0500 executable/library pins per workload, so one sample cannot change
the next sample's identity, lifecycle, or visual precondition.

The post-review Windows run is bound to source commit
`6873d4b05a13fd15cb3ffd961892b1153f606d78`, implementation tree
`2483269ee071d14adfe14f829d318a4c59337f85`, on the named Core i7-12700KF /
RTX 4080 host. Its retained 600 capture, 80 transition, and 300
input/public-language samples all satisfy their exact oracles, report zero
allocation growth, and pass the unchanged ADR 0026 ceilings. Capture p95 ranges
from `0.002500 ms` for latest acquisition to `31.546700 ms` for
stimulus-to-frame. Transition p95 ranges from `2.530000 ms` for close to
`112.344000 ms` for first-frame open. Input p95 is `0.366000 ms` for a Rust
receipt, `116.048000 ms` for the Rust common flow, `15.717900 ms` for C process
loading, `15.343200 ms` for C++ process loading, and below `285 ms` for either
public-language common flow.

[ADR 0028](adr/0028-windows-window-message-performance-budgets.md) fixed the
production `WindowMessage` ceilings from the decision-setting `b72a95f`
profile. That run rejected the pre-measurement 64 KiB maximum-sequence
hypothesis by 1,275 bytes and retained a measured 256 KiB regression ceiling.

The post-review input/public-language profile was regenerated at source
`223925d52d24045ddadbc97c751d79d75a94ad7c`, tree
`ae009ae7f8b917ae13c2ebd02cdea92696d009b9`; its
[raw output](evidence/phase-2-performance/native-phase2-input-window-message-223925d.log)
retains 50 samples after five warmups. One-unit submission measured
`0.3134 ms` p50 and `0.5124 ms` p95; positioning plus a two-unit
primary-button event measured `0.7904 ms` p50 and `1.0026 ms` p95. The maximum
256-event sequence measured `73.7495 ms` p50, `78.8125 ms` p95, 66,647 bytes
of aggregate Rust heap, and zero post-warmup growth. Every workload satisfied
its oracle and the unchanged ADR 0026/0028 budgets.

The first Windows transition and language runs were rejected rather than
recorded. They proved benchmark apparatus defects: the resize fixture stopped
publishing before WGC pool recreation stabilized, one workload reused a
1,024-event fixture for 2,050 redacted summaries, child processes lacked the
known Cargo-profile DLL path, and the C++ oracle expected macOS evidence on
Windows. ADR 0026 records the probes and the bounded repairs. Production capture
and input semantics were not changed to make a benchmark pass.

The Phase 1 profiles historically passed all applicable comparisons at their
recorded source revisions. Windows reruns them on the exact exit candidate;
Apple Silicon runs remain attributed to `d8336be` and apply by reviewed complete
diff. Neither target moves its committed ceilings. The accepted Phase 2.2
controlled-stimulus lineage below
supplies the current macOS capture, transition, and owning-process-route
measurements without treating them as comparable to the invalidated
input-stimulus lineage. ADRs 0030, 0031, and 0032 separately accept the complete
macOS, Windows 1280×720, and corrected Windows dual-4K production-capture
lineages.

## Phase 2.2 macOS process-directed and controlled-stimulus lineage

The macOS process-directed route changes what the `native-phase2` bench can
truthfully measure, in two ways. Capture and transition stimulus moves from
focus-dependent product input to acknowledged fixture-private commands, so the
`capture` and `transitions` workload sets keep their CLI switches but start a
new macOS profile lineage: a controlled-stimulus sample is not comparable with
an input-stimulus sample, and the historical files above stay unchanged. And
the route itself gains its own workload sets — `process-directed`,
`process-directed-game-like`, and `process-diagnostics` — covering fixture
command acknowledgement (`fixture_command_acknowledgement`), acknowledged
stimulus to a strictly newer frame (`controlled_stimulus_to_frame`), static
retained-latest and strictly-newer-expiry behavior, discovery/open with
retained process authority (`discovery_open_retained_authority`), the
per-event authority/preflight/post path (`event_authority_preflight_post`),
release cleanup and session close, and diagnostics `Off`/`Normal`/`Debug` and
overflow around process-directed events.

The macOS-only `resize-allocation` workload set is the focused regression seam
for allocation retained across controlled geometry changes. It measures the
fixture resize command separately from `resize_recreation`, uses five warm-ups
and fifty retained samples, and applies the repository hard correctness and
4,096-byte growth gates without defining an independent latency ceiling. It is
non-normative diagnostic evidence and does not replace the complete
`transitions` or production-capture acceptance profiles.

The original five revision-bound profiles were measured on the approved Apple
Silicon host at corrected pre-optimization source commit
`a1eee9c14a0bd9a1ba92a5ceeff53d378c33f426` (implementation tree
`f4a707501748303adcec577df5f18fcd18f13f45`). The controlled-capture,
controlled-transition, and process-diagnostic records remain bound to that
source. Benchmark bodies formerly attributed to `a471c2d` use a visual oracle
absent from its source tree and remain rejected, non-normative evidence.

The authority-timing-sensitive current profiles are measured at final source
`dec43d7b6c91d415f2028e188e89fa289cb9c1c9` (tree
`109f77df9ef9f40b515245ab60a6036822ee7d78`):

- `phase-2-2-process-directed-appkit-aarch64-apple-darwin`;
- `phase-2-2-process-directed-game-like-aarch64-apple-darwin`.

AppKit terminal p95 is `56.466375 ms` under the frozen `106.34 ms` ceiling;
controlled game-like p95 is `56.699333 ms` under `112.18 ms`. Both profiles
record zero correctness failures, one matching fixture event per terminal
sequence, unchanged foreground and physical cursor, and zero post-warm-up
allocation growth in every workload.

The one-read gate is composite rather than a field in either benchmark row.
Eight revision-bound controller, geometry-source, and native seam tests prove
terminal route ordering, source-transform reuse without a Rust live query, one
final ordinary authority call, and zero retained-window authority calls for
cleanup. The measured rows independently prove latency, fixture observation,
foreground/cursor stability, correctness, memory, and executable provenance
without adding a private inventory counter to the timed path.

The read count has a narrow scope. Before optimization, the same terminal
`RequireUnchanged` pointer profile made four fresh inventory reads — route
preflight, Rust live geometry, native preparation, and native final authority —
and measured `212.674625 ms` / `224.368667 ms` p95. The optimized terminal
one-event `RequireUnchanged` or `UseFrameSnapshot` path with default no-focus
behavior and no later fallback has a deterministic one-final-read shape. A
fallback-eligible route makes one early read plus the final read; terminal
`ReprojectCurrent` makes one live-geometry read plus the final read. These are
separate two-read paths. `RequireFocused`, cleanup, and multi-unit sequences are
excluded from the one-read call-count scope.

The committed files under `docs/benchmarks/` carry the exact measurements,
profile metadata, executable hash, and in-process budgets. Every retained
process-directed profile binds its result to one full Git commit id, tree id,
fixture-source SHA-256 digest, and fixture-executable SHA-256 digest. The commit
and tree in its opening provenance comment must exactly match the values in
`[profile].notes`; `benchmark_block_drift` verifies those equalities and digest
shapes for every committed process-directed profile. A newly generated native
report also carries `benchmark_executable_sha256`, which must be retained when
that report becomes a current profile. The
[macOS input verification guide](macos-input-verification.md#current-native-input-performance-evidence)
provides the runnable benchmark commands, and the tuning Change's
[observed report](../rasen/changes/macos-process-directed-performance-tuning/evidence/observed-report.md)
records their outputs and provenance. Game-like samples carry the fixture's
explicit `mode=game-like renderer=opengl` fact, so they establish no result for
another renderer, game, application, input stack, or anti-cheat system.

These passing profiles are controlled-fixture regression evidence, not a
real-time latency promise or evidence of exact-window delivery, application
consumption, `RequireFocused`, `ReprojectCurrent`, or fallback performance.
Timing did not replace topology qualification: independent `single`, exact
two-display non-mirrored `same-scale`, and `mixed-scale` matrices each passed.
All fourteen controlled pair decisions are release-qualified.

## Phase 2 macOS production-capture acceptance

[ADR 0030](adr/0030-macos-production-capture-performance-budgets.md) accepts the
production-capture and production-transition profiles at measured source
`d182300cd8710891ded6cba17184c44d6d58a114`, tree
`c570343d334a5c77415e6a885ef8821c731b0ad5`, on the approved exactly-two-display
mixed-scale Apple Silicon host. The two profiles retain 1,150 samples across
eight workloads with zero correctness failures and zero allocation growth while
enforcing latency, live-heap, mapped-byte, correctness, and growth budgets.

Natural `publication_age`, strictly-newer acquisition, latest acquisition,
BGRA8 mapping, and retained-pressure recovery are recorded separately from
fresh-session startup, controlled resize recreation, and close drain. Mapped
rows carry exactly one 1280x904 BGRA8 frame (`4,628,480` bytes). The deliberate
retained-pressure row reports a stale ratio near `0.835` only after filling the
finite retained budget and proving blocked/resumed publication; ordinary
production publication and acquisition report zero stale work.

The resize investigation did not establish an unbounded general capture leak.
It identified the documented 64-revision macOS source-geometry history growing
in power-of-two allocation steps and briefly retaining capacity 128 because
retirement followed insertion. The implementation now keeps a small history
for fixed-geometry targets, reserves the complete bound only on the first
geometry change, and retires before insertion at the bound. The exact
production resize row changed from `9,856` bytes to zero post-warm-up growth.

ADR 0030's target-specific p50, p95, and maximum ceilings follow the established
three-times-measurement policy. They are regression gates for this fixture,
host, topology, and operation shape, not real-time or application/game
compatibility claims. Production capture uses a 32 MiB peak live Rust heap
ceiling; transitions use 16 MiB. Every row remains subject to zero correctness
failures and at most 4,096 bytes of allocation growth.

## Phase 2 Windows ownership prototype

The G-002 prototype resolves a correctness and ownership gate; it does not set
numeric product budgets. Its accepted result is recorded in
[ADR 0013](adr/0013-windows-capture-frame-detachment.md) and
[evidence/g-002/](evidence/g-002/).

The selected two-frame WGC pool with callback detachment and lease-aware private
textures delivered every 600-frame candidate sample. Across the updated
MSVC/SDK confirmation, the maximum post-warm-up arrival gap was 63.671 ms. That
figure belongs to the rejected `copy-blind-2` control at pool three; the selected
`copy-leased` rows stayed at or below 63.259 ms and the two-4K rows at or below
62.784 ms. The private default-texture peak was 33 under a retained-frame bound
of 40, and every final resource count was zero. Across all accepted workloads, sequence
regressions were zero and maximum sequence stall was 22.613 ms. The lifecycle
suite's maximum callback drain was 0.037 ms, complete close was 527.302 ms, and
complete reset through `StartCapture()` return was 650.355 ms. Those values
demonstrate margin against the
prototype's 500 ms progress and 2 second close/reset correctness gates; they
are measurements, not ceilings for production code.

The required full-frame GPU copy is a consequence of ownership. Each 1280×720
detached matrix row copied 2,654,208,000 bytes and mapped 3,391,488,000 bytes.
Each 600-frame 3840×2160 display row copied 23,887,872,000 bytes and mapped
30,523,392,000 bytes. A later implementation cannot remove that copy by
publishing a WGC surface or by reusing leased content; it may optimize scheduling
or representation only while the ADR's detachment and lifetime tests still pass.

## Phase 2 Windows 1280x720 production-capture acceptance

[ADR 0031](adr/0031-windows-1280-production-capture-performance-budgets.md)
accepts separate 1280×720 capture and transition profiles. Shared-marker capture
source `f50285a`, tree `4c2f23f`, retained 600 samples across four workloads
with zero correctness failures, zero allocation growth, and every unchanged
budget enforced. Repaired transition source `7c31752`, tree `4e99487`, reran all
five lifecycle workloads with zero correctness failures and every unchanged
latency, mapping, memory, growth, and cleanup gate enforced.

The capture profile records arrival, one frame-stamp-correlated callback copy,
latest acquisition, and BGRA8 mapping separately. The shared-marker run reports
one exact 3,686,400-byte callback copy, two detached textures, one staging
texture, five total producer/detached/staging textures, zero steady stale work,
a 7,417,467-byte live Rust heap peak, and a 66,506,752-byte resident peak.
Resource counts are nonzero upper bounds; valid lower counts pass.

ADR 0031 follows the three-times-measurement policy for target-specific p50,
p95, and maximum ceilings. Both profiles use a 32 MiB live Rust heap ceiling,
a 256 MiB native process resident ceiling, zero correctness failures, and at
most 4,096 bytes growth. They are regression gates for the named host, fixture,
topology, and operation shape, not game or real-time guarantees.

## Phase 2 Windows dual-4K production-capture acceptance

[ADR 0032](adr/0032-windows-dual-4k-production-capture-performance-budgets.md)
accepts the corrected mixed-DPI dual-4K profile at shared-predicate source
`f50285a`, tree `4c2f23f`. The stationary pair retains 600 strictly newer
samples per display while sharing each capture/mapping interaction. A distinct
no-warm-up workload retains 300 frame pairs while moving the controlled fixture
across the signed seam.

The final stationary rows report zero correctness failures, zero growth, exact
66,355,200-byte mappings, a 199,065,600-byte six-surface copy interval, seven
detached textures, one staging texture, twelve total resources, a `0.455782313`
stale ratio, 99,582,727 bytes live Rust heap, and 219,213,824 bytes resident.
The moving row reports zero correctness failures, 320 bytes growth, a
six-surface copy interval, six/one/eleven resources, a `0.474145486` stale
ratio, 99,576,732 bytes heap, and 288,911,360 bytes resident.

Two corrected-marker precursor runs establish the moving 125/175/225 ms
p50/p95/maximum ceilings through ADR 0032's three-times/readable-rounding
policy. Both requested marker positions and each frame's coherent post-baseline
stream/epoch/sequence callback record must match under one absolute deadline.
ADR 0032 retains 384 MiB heap, 1 GiB resident, copy, texture, and stale-work
ceilings for all three workloads. Windows production-capture `G-013` is
complete. Windows Phase 1 reruns pass on the exact exit candidate; Apple Silicon
runs remain attributed to `d8336be` and apply by reviewed complete diff.
The complete workload and correctness obligations are in
[windows-capture-contract-tests.md](windows-capture-contract-tests.md).

## Phase 3 accepted CPU OCR performance

The Phase 3 benchmark is
`crates/backend/onnx/benches/onnx-cpu.rs`. It exercises only the accepted
`g-004-rapidocr-ppocrv4-det-v6-rec-small-v1` model through controlled ONNX
Runtime 1.29.0 API 17, CPU provider, one admitted inference, one session pair,
two sessions, one thread per session axis, disabled CPU arena, and recognition
batch ceiling six.

Every process opens through the same default path a Rust/C/C++ caller uses.
Three warmups precede twenty retained samples for each workload. With twenty
samples, nearest-rank p95 is the nineteenth value rather than the maximum; the
independent maximum ceiling still catches a single slow operation:

| Workload | Oracle and accounting |
|---|---|
| `onnx_cpu_hud_full` | exact eight NFC strings/count/order; fixture-derived geometry; finite same-host-stable confidence; exact source/backend/model/effective-region correlation; 2,073,600 observed mapped bytes; one observed detector plus two recognizer tensor runs |
| `onnx_cpu_hud_region` | exact bounded text/count; the same geometry/confidence/source rules; 64,800 observed mapped bytes; one observed detector plus one recognizer tensor run |
| `onnx_cpu_blank` | exact empty result/source correlation; 16,384 observed mapped bytes; one observed detector and zero recognizer tensor runs |

No operation is discarded after execution. A failed warmup aborts immediately;
retained incorrect count must be zero, and every workload must remain within the
shared 4,096-byte live-heap growth gate. The native ignored contract test
separately proves cancellation after native-run admission, termination and
`Cancelled` within 250 ms, no late publication, recovery, close-race safety, and
idempotent repeated close; instrumentation from that proof is not mixed into
latency percentiles.

The first smoke attempt rejected a draft exact-geometry assumption for bounded
recognition. Independent crop preprocessing legitimately produced a different
quadrilateral while passing the pre-existing fixture thresholds, so the
performance oracle was corrected before precursor measurement. The failed
attempt remains recorded; no measured failure was excluded and no target result
selected a threshold.

[`phase-3-ocr-aarch64-apple-darwin.toml`](benchmarks/phase-3-ocr-aarch64-apple-darwin.toml)
is normative under ADR 0037. Five fresh precursor processes at source `b83f23f`,
tree `7dfe9c2`, retained 50 samples per workload. Every warmup/sample passed and
every workload ended with zero heap growth. Worst precursor observations were
476.576/299.674/135.640 ms p95 for full/bounded/empty, 86.988 ms cold open,
1.116 ms first close, 63.569 ms reopen-close, and 548,487,168 bytes RSS.

The earlier post-budget run at source `192f3d2`, tree `45abefc`, remains valid
for latency, correctness, allocation growth, and external RSS observations. An
independent review found that mapped/tensor/session values were synthesized and
the RSS ceiling was manually compared rather than executable; those resource
acceptance claims do not transfer.

Review-fixed source `e41fbbd`, tree `9fbc47e`, instruments the actual mapped
`CpuMapping` bytes, detector/recognizer session accesses, and opened session
topology, and enforces macOS `getrusage` peak RSS in-process. Five fresh
processes retained 100 samples per workload. Worst full/bounded/empty p95 was
472.623/301.455/184.057 ms and maximum 478.703/309.750/185.290 ms; cold open
87.377 ms, close 1.163 ms, reopen-close 64.929 ms, RSS 516,833,280 bytes, and
worst attributable Rust heap 14,149,992 bytes. Every oracle/resource gate passed
with zero growth.

[`phase-3-ocr-x86_64-pc-windows-msvc.toml`](benchmarks/phase-3-ocr-x86_64-pc-windows-msvc.toml)
is the matching normative Windows profile. Precursor source `6b5f3c1`, tree
`0a9fc22`, executable SHA-256
`f3c13157807c9617fb03039b9689cd53ccc138701d9c412180c03fc28800a316`
ran in five fresh processes on the approved Windows 11 Pro 25H2 build-family-26200
Core i7-12700KF host. All 100 retained samples per workload passed with zero
growth. Worst full/bounded/empty p95 was 810.143/581.789/320.843 ms and maximum
849.570/586.828/359.929 ms; cold open was 182.002 ms, first close 5.652 ms,
reopen-close 161.748 ms, and `GetProcessMemoryInfo` peak RSS 242,667,520 bytes.
The full-frame process-5 and empty-result process-4 slow rows remain in the
profile derivation rather than being excluded.

Separate final source `f2d3f29`, tree `b9c8fb5`, executable SHA-256
`f0292ce5fafc106ddff2ffc4ef1066543e780dfa0ca8958eb6784e891d187310`
ran five more fresh processes. Every target budget and hard oracle passed without
retry or exclusion. Worst full/bounded/empty p95 was
717.487/579.638/275.385 ms and maximum 724.006/581.872/290.567 ms; cold open was
177.195 ms, first close 6.081 ms, reopen-close 160.820 ms, and peak RSS
242,810,880 bytes.

The first Apple resource-instrumented process failed the former 140 ms cold
ceiling at 141.686 ms; an unchanged repeat reported 90.075 ms. ADR 0037 retains
the failure and sets 175 ms from both observations. A separate ten-sample run produced one
785.460 ms full-frame outlier, making nearest-rank p95 equal the maximum and
exceeding 750 ms even though the 900 ms maximum held. The final twenty-sample
policy makes p95 the nineteenth sample while the independent maximum remains
enforced; it changes no latency ceiling.

Accepted Apple ceilings are 600/750/900 ms p50/p95/maximum for full-frame,
375/450/600 ms bounded, and 175/210/300 ms empty. Cold open is capped at 175 ms,
first close 2 ms, reopen-close 100 ms, live Rust heap 20 MiB, and resident
high-water 768 MiB.

Accepted Windows ceilings are 900/1,000/1,200 ms p50/p95/maximum for full-frame,
725/750/850 ms bounded, and 350/425/500 ms empty. Cold open is capped at 250 ms,
first close 10 ms, reopen-close 225 ms, live Rust heap 20 MiB, and resident
high-water 320 MiB. These are derived only from the approved Windows desktop,
not Apple or hosted Windows Server.

Both profiles cap input/output tensor bytes at 256 MiB and enforce exact mapped
bytes plus detector/recognizer/session/result counts. Producer-surface copied
bytes are not applicable to immutable CPU replay inputs and have no ceiling.
These are regression ceilings for the named hosts and fixture, not
arbitrary-resolution, 4K, multi-region, real-time, renderer, application, or
game guarantees. Phase 3 default-OCR `G-013` is complete on both release
targets; watcher scheduling and acceleration remain open until later phases
introduce those workloads.

## v0.3.1 bounded-detector candidate performance

`crates/backend/onnx/benches/bounded-detector.rs` compares released native G-004
and the explicit bounded candidate on identical 4K, wide, extreme-wide,
960×540, odd, dense, boundary-region, and 4K blank inputs. Every iteration
checks text/count/order, fixture geometry, same-host confidence, complete source
and profile identity, final detector dimensions/bytes, one direct resize,
detector/recognizer runs, mapped bytes, one-pair/two-session topology,
cancellation, and at most 4,096 post-warm live Rust growth. The bounded
candidate additionally enforces at most 20 MiB attributable live Rust heap.
Native arbitrary-4K peak is recorded as comparator work under its released
256 MiB tensor ceiling; it is not judged by the new profile's peak limit.

ADR 0039 separates three phases. `smoke` is the target-independent hosted gate.
`precursor` runs three warmups and 20 retained samples in five fresh processes,
enforces all hard correctness/resource rules, and records timing/RSS without a
numeric verdict. The current executable refuses `--qualify`;
`enforce-budgets` is added to a fresh executable only after both approved
targets have precursor evidence and a final budget ADR. Result count does not
select a cost class: the 3840×2160 blank workload maps and detects 4K input
despite returning no regions.

The first Apple run at source `2782564`, executable SHA-256
`3a4bb04477b14092c9b3b34153275819684790d072aa1659e44183bafbd1f8b4`,
remains rejected. All correctness/resource rows passed, but the procedure
incorrectly applied the released 64×64 empty-result latency row to the 4K blank
workload, and one 2.697 ms close exceeded the earlier revision's 2 ms final
budget. No failed process was removed or relabeled.

Reviewed rectangular-candidate precursor source `cff5338`, executable SHA-256
`c28257dea60a86fe58d9fe9549670f6615004d3553c2f0bbe0a996a40ef9575d`,
ran five fresh processes per profile on the approved Apple M1 Pro. Every
bounded process passed. The native comparator preserved five false executable
verdicts because the precursor still evaluated its arbitrary-4K peak against
the bounded-only 20 MiB ceiling. Those false verdicts are retained rather than
relabeled. All 1,600 retained native/bounded samples otherwise passed with zero
oracle failures and zero live Rust growth. Worst per-process observations were:

| Workload | Bounded detector | Native detector | Bounded p95 | Native p95 |
|---|---:|---:|---:|---:|
| 4K HUD | 1312×736 / 11,587,584 bytes | 3840×2176 / 100,270,080 bytes | 490.454 ms | 2,347.264 ms |
| Wide menu | 1312×320 / 5,038,080 bytes | 2944×736 / 26,001,408 bytes | 339.375 ms | 787.027 ms |
| Extreme-wide status | 1312×160 / 2,519,040 bytes | 5888×736 / 52,002,816 bytes | 230.009 ms | 1,282.602 ms |
| 960×540 HUD | 1312×736 / 11,587,584 bytes | 1312×736 / 11,587,584 bytes | 479.206 ms | 480.257 ms |
| Dense tooltip | 1312×640 / 10,076,160 bytes | 1472×736 / 13,000,704 bytes | 599.615 ms | 673.754 ms |
| 4K blank | 1312×736 / 11,587,584 bytes | 3840×2176 / 100,270,080 bytes | 242.689 ms | 2,102.128 ms |

Bounded attributable live Rust peak was 12,695,400 bytes and bounded peak RSS
was 586,645,504 bytes. The native comparator's attributable peak reached
108,627,472 bytes under its released 256 MiB tensor ceiling, and its peak RSS
was 2,454,126,592 bytes. Reference-size work is intentionally unchanged. One
odd-size bounded process retained a slower p95 despite equal detector
dimensions, so no general speedup is inferred from the profile name.

The exact Apple process rows and retained native false verdicts are Change
evidence. The matching approved Windows `cff5338` matrix also passed all five
bounded correctness/resource verdicts, but ADR 0039's unchanged formula rejected
the rectangular candidate. Margin-derived p50/p95 ceilings exceeded fixed caps
for 4K HUD, 960×540 HUD, odd HUD, and dense tooltip; dense maximum also exceeded
its cap. Cold derived 275 ms above 250 ms. Observed RSS derived 384 MiB above
320 MiB, although that harness retained all eight source frames and the row is
not reused as one-operation memory evidence. No cap or expected result was
relaxed.

ADR 0040 replaces the unreleased `bounded-v1` tuple. Candidate v2 preserves
1312×736 reference/odd detector pixels, but after an oversized desired detector
first fits the 1312×736 rectangle, it applies a second 6 MiB aspect-preserving
tensor fit when needed. Fixed workload dimensions become 960×512 for 4K,
1024×480 for dense tooltip, and remain 1312×320, 1312×160, 1312×736, and
576×736 for the other declared shapes. The benchmark constructs and drops one
source fixture per workload so those frames are not simultaneously live. OS RSS
is process-lifetime high-water: schema-v3 workload fields are explicitly named
`process_peak_resident_bytes_after_workload`, and only final report-level RSS
sets the process budget.

Exact candidate-v2 source `ce658b3`, executable SHA-256
`dea9cdfbbb66ba75cb490fc3359efa6ca599786726157a520544cf39b38f81eb`,
ran five fresh bounded and five fresh native schema-v3 processes on the approved
Apple M1 Pro with alternating pair order. All ten raw verdicts and all 1,600
retained samples passed with zero oracle failure/growth and complete RSS rows.
Bounded worst p95 was 372.100 ms for 4K HUD, 476.275 ms for reference HUD,
474.544 ms for dense tooltip, and 122.430 ms for 4K blank; peak heap was
12,695,336 bytes and final process RSS was 464,519,168 bytes. Every
formula-derived Apple candidate fits the unchanged caps.

Exact Windows executable
`54a10f73970e24e126b4863853c0949610206bb7135579477883ec669ea0b5ed`
ran the same alternating five bounded/five native schema-v3 matrix on the
approved Core i7-12700KF. All ten raw verdicts and all 1,600 retained samples
passed. Bounded worst p95 was 530.211 ms for 4K HUD, 718.100 ms for reference
HUD, 591.838 ms for dense tooltip, and 244.941 ms for 4K blank; peak heap was
12,695,272 bytes and final process RSS was 228,741,120 bytes. Every
formula-derived Windows candidate fits the unchanged caps.

ADR 0041 accepts the exact target budgets in
`phase-3-1-bounded-ocr-aarch64-apple-darwin.toml` and
`phase-3-1-bounded-ocr-x86_64-pc-windows-msvc.toml`. Strict final source
`33cd36b` rejects missing prerequisites, unknown/duplicate modes, and ambiguous
profiles before work; independent fix review returned no findings.

Apple executable
`7e48921dfeaa7b0f3a4bb33b9e927eea9e50d75422c570adb6443fd4f32cf190`
and Windows executable
`aefdfa9cd6a023049b532f650a5493191994b22b3c07b582097ca1146a58d5e4`
each passed five fresh bounded `--enforce-budgets` processes without retry or
exclusion. Worst final process RSS was 464,666,624 bytes on Apple and
229,089,280 bytes on Windows; all workload latency/heap/growth rows passed.
The explicit profile is qualified for the named target/runtime/model/fixture
boundaries. It remains non-default and is not a real-time or arbitrary-workload
guarantee.

## v0.3.1 integrated grouped-zone qualification

The accepted bounded-v2 singular profiles above remain revision-bound regression
gates. Integrated exact source `180c1b1`, tree `479e410`, added the fixed
one-/three-/eight-zone, dense, and empty rows without changing those profiles.
The approved Apple M1 Pro executable
`6ce1df5bba8bc555fa961af366b0386333e6baeebd7c9483b1be9da39f16c792`
and Windows Core i7-12700KF executable
`b34b99eb7dcb3870edbd768055428be655e1e45ad125400d3b999bfb4da23398`
each ran one native comparator and five fresh alternating 3+20 integrated
processes without retry or exclusion.

Every row passed exact text/count/order/geometry/source/profile,
source-envelope/zones/memberships, detector/recognizer work, mapping, heap,
growth, cancellation, retained-result independence, startup, close, and cleanup
oracles. Both targets reported zero incorrect retained samples, zero call
failures, zero post-warm growth, and identical deterministic resource signatures.
Duplicate, one-pixel-different, adjacent, slight-overlap, and complete-overlap
layouts remain bounded safety cases with no quality or latency support claim.

[ADR 0044](adr/0044-integrated-zone-ocr-target-budgets.md) accepts the
predeclared 1.25-times/25-ms formula results:

| Workload | Apple p50 / p95 / maximum | Windows p50 / p95 / maximum |
|---|---:|---:|
| full-frame one zone | 600 / 600 / 625 ms | 900 / 900 / 900 ms |
| three sparse zones | 375 / 375 / 375 ms | 525 / 525 / 525 ms |
| eight distinct zones | 450 / 450 / 475 ms | 600 / 600 / 600 ms |
| dense unique candidates | 600 / 675 / 700 ms | 725 / 750 / 750 ms |
| empty 4K result | 175 / 175 / 175 ms | 300 / 325 / 325 ms |

ADR 0041 startup/close, 20 MiB attributable live Rust peak, 4 KiB growth,
11,587,584-byte detector tensor, and target final-RSS ceilings remain unchanged.
Active native cancellation-to-return is capped at 25 ms on each target.
Retained one-zone result completion is capped at 625 ms on Apple and 900 ms on
Windows. Each fixed grouped row additionally requires one mapping/resize/detector
run, exact mapped and detector bytes, expected zero/one/two recognizer runs,
exact selected/ignored/unique/membership/result accounting, one cleanup, and no
retained parent resource.

The new integrated benchmark profiles record this separate lineage rather than
editing either ADR 0041 profile. Five fresh `--enforce-budgets` processes from
one final executable on each approved host remain required for release
acceptance. Hosted CI enforces correctness and bounded growth only; its timing
and RSS do not define either profile.
