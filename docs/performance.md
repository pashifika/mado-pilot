# Benchmark profiles and budgets

Later phases of MadoPilot introduce performance-sensitive behavior: capture,
frame mapping, template matching, OCR, watcher scheduling, and acceleration. Each
of those phases must be able to show that its behavior is both correct and within
an agreed cost before the phase exits.

This document defines the format that evidence takes. It deliberately assigns no
numeric product budget, because no representative workload exists yet: a number
invented now would be fiction that later evidence has to argue against. Setting a
numeric budget is gate [`G-013`](validation-gates.md#g-013).

Phase 0 ran no runtime benchmark. Nothing in this document, and nothing in the
example it references, is a measured result.

## Where benchmark files live

A phase commits one file per workload under `docs/benchmarks/`, named
`<phase>-<workload>.toml`. Each file contains one profile and its budgets, so that
a budget is never separated from the conditions under which it was measured.

The format example is
[benchmarks/example-synthetic.toml](benchmarks/example-synthetic.toml). It is a
format demonstration only: it is marked non-normative, it records no measurement,
and its numbers are illustrative.

## Format version

Every file declares `format_version`. The current version is `1`. A change that
alters the meaning of an existing field increments the version and records the
migration in an architecture decision record; adding an optional field does not.

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
| `steady_memory` | bytes | Resident memory in steady state. |
| `mapped_bytes_per_result` | bytes | Frame bytes mapped into CPU memory per produced result, full-frame or region of interest. |
| `stale_work_ratio` | ratio | Share of scheduled work that was dropped, coalesced, superseded, rejected, queue-expired, or discarded as stale. |
| `model_load_time` | milliseconds | Time to load and initialize an OCR model, including provider selection. |
| `startup_time` | milliseconds | Time from process start to a usable session. |
| `result_correctness` | count | Retained samples whose output disagreed with the correctness oracle. A hard gate, never a tuned ceiling. |
| `memory_growth` | bytes | Signed change in resident memory across the sampled run, so a decrease is negative. A hard gate: unbounded growth is a defect, not a slow result, and its predicate bounds growth rather than demanding an exact zero. |

A phase that needs a measure outside this list adds it here in the same change,
with its unit and its meaning.

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

Phase 1 delivers the harness and the correctness oracles for the deterministic
Rust workflow, and **still sets no numeric budget**. `G-013` stays open: a budget
needs measurements from both release targets, taken on hosts whose CPU and
operating-system build are stated, and inventing a number from one developer
machine would be the fiction this document exists to avoid.

The harness is a bench target of the `mado-pilot` package, at
`crates/mado-pilot/benches/deterministic-slice.rs`. It covers six workloads:

| Workload | What it measures | Correctness oracle |
|---|---|---|
| `replay_open` | Discovering targets and opening a session | The session reports the source's own extent and pixel format |
| `map_full_frame` | Mapping a whole frame to CPU-readable bytes | The mapping covers the whole frame and reports its exact source identity |
| `map_region_of_interest` | Mapping one region of a held frame | The mapping covers the requested region and no more |
| `load_package` | Loading and validating a directory package | The package declares both tracked templates |
| `prepare_and_match_cold` | Compiling a template and searching with it | Both planted copies are found at their planted offsets, scoring `1.0` within `1e-5` |
| `match_warm` | Searching with an already-compiled template | As above |

Every sample is checked against its oracle, including in the run that produces
timings, because a latency number whose output was never checked is a timing
experiment rather than evidence.

The harness has two modes so the oracles can run far more often than the
timings. `cargo test --locked --workspace --all-targets` executes it with three
samples per workload and fails on any oracle violation, which puts the whole
workflow's correctness into the ordinary verification sequence. A timing run is
explicit, and labels its host because the program does not guess one:

```sh
cargo bench --locked --package mado-pilot --bench deterministic-slice -- \
    --label "Windows 11 Pro 25H2, Core i7-12700KF"
```

It prints a profile-shaped report with measurements and no budget. Turning one
into tracked evidence means recording the exact target triple and the fixture
hashes alongside it, in a file under `docs/benchmarks/`, in the same change that
sets the budget it supports.

The harness has been run on both release targets with every oracle satisfied.
That produced one finding a later budget has to plan around rather than
discover: on `x86_64-pc-windows-msvc`, `map_full_frame` measures **zero**. The
mapping is shared rather than copied when the requested format already matches
the frame's, so the operation is a reference-count increment, and the Windows
monotonic clock's granularity is coarser than that. A budget for that workload
therefore cannot be a latency ceiling taken one iteration at a time. It needs a
measure that does not vanish — `mapped_bytes_per_result` bounds the same
behavior, and a batched timing over many iterations recovers a number — and the
same is true of any later operation whose fast path is a pointer copy.
