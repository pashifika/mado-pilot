# Benchmark profiles and budgets

Later phases of MadoPilot introduce performance-sensitive behavior: capture,
frame mapping, template matching, OCR, watcher scheduling, and acceleration. Each
of those phases must be able to show that its behavior is both correct and within
an agreed cost before the phase exits.

This document defines the format that evidence takes. Setting a numeric budget
for a workload is gate [`G-013`](validation-gates.md#g-013), which is resolved
per workload and target rather than once. Phase 1 is resolved. ADR 0021
invalidated the three historical macOS Phase 2 native profiles after source and
correctness-oracle drift. [ADR 0024](adr/0024-input-diagnostic-performance-budgets.md)
now accepts the macOS diagnostic slice, and
[ADR 0025](adr/0025-macos-native-input-performance-budgets.md) accepts the
revision-bound macOS native input and public-language profile. The remaining
native workload and target gaps stay open.

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
| `peak_resident_bytes` | bytes | High-water resident set of a measured child process, reported by the native operating system after that owned child exits. Optional because in-process workloads do not create one. |
| `steady_memory` | bytes | Resident memory in steady state. |
| `mapped_bytes_per_result` | bytes | Frame bytes mapped into CPU memory per produced result, full-frame or region of interest. |
| `stale_work_ratio` | ratio | Share of scheduled work that was dropped, coalesced, superseded, rejected, queue-expired, or discarded as stale. |
| `model_load_time` | milliseconds | Time to load and initialize an OCR model, including provider selection. |
| `startup_time` | milliseconds | Time from process start to a usable session. |
| `result_correctness` | count | Retained samples whose output disagreed with the correctness oracle. A hard gate, never a tuned ceiling. |
| `memory_growth` | bytes | Signed change in resident memory across the sampled run, so a decrease is negative. A hard gate: unbounded growth is a defect, not a slow result, and its predicate bounds growth rather than demanding an exact zero. |
| `latency_p50` | milliseconds | Median of the per-iteration samples for one workload. |
| `latency_p95` | milliseconds | The 95th percentile of the same samples. Distinct from `capture_to_result_latency_p95`, which is end-to-end from capture to committed result rather than one operation. |
| `iteration_span_ms` | milliseconds | One clock reading across the whole sampled run, divided by the sample count. It covers everything an iteration does, including the correctness check, so it is an upper bound on the operation rather than a reading of it. Use it where a per-iteration percentile is not expressible; see below. |
| `peak_allocated_bytes` | bytes | High-water mark of live heap bytes during the sampled run, above what was live before the workload's fixture existed. |
| `steady_allocated_bytes` | bytes | Live heap bytes when the sampled run finished, above the same baseline, with the fixture still alive. |
| `allocated_growth_bytes` | bytes | Signed change in live heap bytes across the sampled run alone. A hard gate, on the same terms as `memory_growth`. |

A phase that needs a measure outside this list adds it here in the same change,
with its unit and its meaning.

### Why some names carry their unit and others do not

The suffix is not decoration and it is not applied evenly, so the rule is worth
stating rather than inferring. A name carries its unit when the quantity would be
ambiguous without it — `iteration_span_ms` is a duration and `_ms` says which
one, `peak_allocated_bytes` counts bytes and `_bytes` separates it from a byte
*rate* — and omits it when the `Unit` column above is the only answer the measure
can have. `latency_p95` is milliseconds because every latency here is.

Three vocabulary names differ from the key a profile records the value under,
which is the one place a reader can be caught out:

| Vocabulary name | Recorded as |
|---|---|
| `latency_p50` | `latency_p50_ms` |
| `latency_p95` | `latency_p95_ms` |
| `memory_growth` | `allocated_growth_bytes`, when the measure is live heap rather than resident memory |

A budget's `measure` may name either form; committed profiles use the recorded
key everywhere except `latency_p50` and `latency_p95`, where they use the
vocabulary name. Renaming to one convention would move every committed profile,
the harness that prints them, and the drift test that compares the two, so the
mapping is documented instead.

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

Phase 2's affected [`G-013`](validation-gates.md#g-013) workloads are partially
resolved. [ADR 0021](adr/0021-invalidate-phase-2-native-performance-evidence.md)
invalidated the three macOS profiles originally accepted by
[ADR 0020](adr/0020-phase-2-native-performance-budgets.md). ADR 0025 replaced
the macOS input profile; ADR 0026 now replaces every Windows native gap:

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
source. The macOS input requalification retained 300 correct samples with zero
allocation growth after replacing stale six-event/two-key-pair expectations and
giving every public-language sample an independent visual precondition.

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

[ADR 0028](adr/0028-windows-window-message-performance-budgets.md) requalified
the Windows input profile at source commit
`fd71a3f9e08d1eada50b53cf4b47d830e92b60d2`, tree
`0267816de8c18a98bacf66c7ab455c8a18143650`, and added production
`WindowMessage` rows. One-unit submission measured `0.1999 ms` p50 and
`0.3060 ms` p95; positioning plus a two-unit primary-button event measured
`0.7913 ms` p95. The maximum 256-event sequence measured 66,685 bytes of
aggregate Rust heap with zero post-warmup growth. That result rejected the
pre-measurement 64 KiB hypothesis by 1,149 bytes, so ADR 0028 records the failed
comparison and sets a 256 KiB regression ceiling rather than changing production
code to fit an estimate. The final native rerun kept full/partial queue refusal
below `0.7 ms`, hung queue admission below `0.3 ms`, and
deadline/cancellation cleanup below `22 ms` against their fixed 10 ms and 250 ms
ceilings.

The first Windows transition and language runs were rejected rather than
recorded. They proved benchmark apparatus defects: the resize fixture stopped
publishing before WGC pool recreation stabilized, one workload reused a
1,024-event fixture for 2,050 redacted summaries, child processes lacked the
known Cargo-profile DLL path, and the C++ oracle expected macOS evidence on
Windows. ADR 0026 records the probes and the bounded repairs. Production capture
and input semantics were not changed to make a benchmark pass.

The Phase 1 profiles historically passed all applicable comparisons at their
recorded source revisions. Release acceptance still requires rerunning them at
the eventual final Phase 2 revision; their committed Phase 1 ceilings do not
move. Current macOS capture and transition profiles also remain required before
Phase 2 exit.

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

The production Windows capture portion of
[`G-013`](validation-gates.md#g-013) remains open independently of the
`native-phase2` profile accepted by ADR 0026. Its acceptance profile must budget
capture arrival, callback-copy p95, mapped and copied bytes, detached/staging
and resident memory, drops and stale work under pressure, session startup,
resize recreation, close drain, and reset recovery at 1280×720 and on the named
dual-4K topology. The complete workload and correctness obligations are in
[windows-capture-contract-tests.md](windows-capture-contract-tests.md).
