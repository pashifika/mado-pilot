# ADR 0008: Phase 1 performance budgets

- **Status:** Accepted
- **Date:** 2026-07-28
- **Resolves gate:** the Phase 1 workloads of `G-013` from
  [../validation-gates.md](../validation-gates.md). The gate itself stays open
  for every later phase.
- **Supersedes:** _none_

## Context

`G-013` requires a numeric budget for every workload a phase introduces, backed
by repeatable measurements taken on the release target the workload runs on.
Phase 0 defined the profile and budget format and deliberately set no number.
Phase 1 built the harness and the correctness oracles and still set none,
because a budget needs measurements from both release targets and inventing one
from a single developer machine would be the fiction
[performance.md](../performance.md) exists to avoid.

Both targets have now been measured:

| Target | Host | Toolchain |
|---|---|---|
| `aarch64-apple-darwin` | Apple M1 Pro, 10 cores, 32 GiB, macOS 26.5.2 build 25F84 | Apple clang 21.0.0, CMake 4.4.0, rustc 1.97.1, OpenCV 4.14.0 |
| `x86_64-pc-windows-msvc` | Core i7-12700KF, 20 threads, 32 GiB, Windows 11 Pro 10.0.26200 build 26200 | MSVC 19.37.32824, CMake 3.29.5, rustc 1.97.1, OpenCV 4.14.0 |

Both runs report the same `fixture_sha256`, so the two targets measured the same
bytes. Two hundred samples per workload after twenty warm-up iterations, every
sample checked against its oracle, zero oracle failures on either target.

Five things were not measured before this change and now are: loading the same
package from memory and from an archive as well as from a directory,
`mapped_bytes_per_result`, live-heap peak and growth, one batched span per
workload, and the cost of crossing the C ABI.

## Decision

### Thirteen workloads are measured, in four committed profiles

Two benchmarks, each measured on both release targets:

| Benchmark | Profiles | Covers |
|---|---|---|
| `deterministic-slice` | [aarch64](benchmarks/phase-1-deterministic-slice-aarch64-apple-darwin.toml), [x86_64](benchmarks/phase-1-deterministic-slice-x86_64-pc-windows-msvc.toml) | The eight-operation Rust workflow |
| `c-boundary` | [aarch64](benchmarks/phase-1-c-boundary-aarch64-apple-darwin.toml), [x86_64](benchmarks/phase-1-c-boundary-x86_64-pc-windows-msvc.toml) | What the C ABI costs, against the same work through the facade |

Each file holds one profile, its measurements, three budgets that apply to the
whole file, and the workload-specific budgets.

"Has a budget" can mean either "is covered by a file-level gate" or "carries a
ceiling of its own", and those two readings do not produce the same count. This
record uses one phrasing for both, and every document that cites the number uses
it too: **thirteen workloads are measured, all thirteen are covered by the two
file-level hard gates, eleven carry a per-measurement ceiling, and two are
deliberate unbudgeted controls.** The two controls are `engine_create_rust` and
`match_warm_rust`; the reason they carry none is below, and is recorded in the
profiles beside the measurements themselves.

### The budgets are regression ceilings, and say so

Nothing in Phase 1 has a user-facing latency requirement. A budget presented as
a product requirement would be one, so each numeric ceiling is stated as what it
is: **three times the measured value, rounded up to a readable number.**

Three, not two, because these runs are on developer machines rather than
dedicated hardware, and a ceiling that a background build can trip is a ceiling
that gets raised rather than investigated. Three still fails a structural
regression — an added copy, a lost cache, a second mapping — while surviving a
loaded host. Tightening one requires new evidence and an ADR, exactly as
loosening one does.

### Two hard gates apply to every workload on both targets

- `result_correctness == 0`. A latency number whose output was never checked is
  a timing experiment; a workload whose output is wrong has not produced a slow
  result, it has produced a wrong one.
- `allocated_growth_bytes <= 4096`. Bounded rather than exactly zero, as
  [performance.md](../performance.md) requires, and one page is the bound
  because a leak of even twenty-four bytes per iteration exceeds it over two
  hundred samples while a single rounded-up allocation does not. Both targets
  measured **zero** on all thirteen workloads.

Both are enforced by `mado_pilot_testkit::bench_harness::enforce_hard_budgets`,
which every benchmark target calls unconditionally, so they hold on the run that
produces timings and on the reduced run
`cargo test --locked --workspace --all-targets` performs. A violation therefore
fails a test on both release targets and in CI, rather than becoming a number in
a report nobody compared, and the failure names the workload, the predicate, and
the measurement that crossed it. `crates/support/testkit/tests/hard_budget_drift.rs`
pins the predicate strings the harness enforces against the ones the four
committed profiles state, in both directions, so neither side can drift from the
other without failing a test.

The absolute ceilings in this record are evaluated the other way: each is valid
only for the release target in its profile, so whoever performs a timing run on
that hardware is the one who compares it against the committed file. The rule
behind the split is that a hard budget is a structural property that holds on any
host, while an absolute or relative budget is a per-target regression ceiling
measured on named hardware. [performance.md](../performance.md) states the same
rule for every phase.

How small a leak the growth gate catches depends on which run enforces it. The
twenty-four-bytes-per-iteration figure above is the `--bench` run's, over two
hundred retained samples, and that run is what this decision is set against. The
reduced run retains three, so on that path the same one-page bound is crossed by
a leak of roughly 1,365 bytes an iteration rather than twenty-four: a leak of a
few dozen bytes an iteration survives CI and is caught by the full run before a
profile is recorded. The gate is real on both paths — a leak is a leak — and the
difference is sensitivity rather than reach.

### A faster measurement is not by itself an improvement

[performance.md](../performance.md) states the rule this ADR is bound by: a
higher rate is not an improvement when it increases stale work, increases
memory, or produces incorrect results. The numeric ceilings below are the least
interesting part of these profiles, because a change can pass every one of them
and still be a regression.

What enforces the rule for these workloads is the two hard gates, plus an
assertion in the deterministic-slice benchmark that the mapping workloads take
the paths their recorded byte counts assume: a full-frame mapping in the source
format shares the frame's storage, and a region mapping owns a packed copy. A
change that made a mapping faster by copying rather than sharing fails that
assertion; a change that made matching faster by retaining state between
searches shows up as allocation growth. Neither is caught by a latency number,
which is why removing either gate as redundant would be removing the part that
does the work.

The assertion carries that case because `mapped_bytes_per_result` cannot.
`mapping.bytes().len()` is the same number whether the mapping shared the
frame's storage or copied it, so the measure records what was mapped rather than
how. For the two matching workloads it is derived from the reported searched
region and the backend's bytes per pixel, and is invariant to how many times the
backend mapped. It still bounds a workload that started mapping more than once
per result, which is why the ceilings stay; making it an observed count needs the
vision backend to report mapped bytes where the mapping happens, and that is
deferred to the phase that adds one.

Phase 1 has no queue, so `stale_work_ratio` has nothing to measure and no
profile carries it. The first phase with a watcher or a bounded work queue adds
it, and the rule bites hardest there: that is the phase where a higher capture
rate can genuinely make results worse.

### `peak_allocated_bytes` is capped at half a mebibyte

An absolute, target-independent statement: the entire deterministic slice —
engine, session, package, prepared template, frame, mapping, result — fits in
512 KiB of live heap. The largest measured workload is archive loading, at
193,418 bytes on macOS and 193,212 on Windows, a difference of 0.2 KiB.

### `map_full_frame` gets no latency budget

This is the finding the budget shape had to be built around rather than
discover. On `x86_64-pc-windows-msvc` its median is **exactly 0.000000 ms** and
its 95th percentile is exactly one 100-nanosecond tick. A mapping whose
requested format already matches the frame's is shared rather than copied, so
the operation is a reference-count increment and the host clock cannot resolve
it. A ceiling on that reading would be a budget on the clock's granularity.

It is bounded two other ways instead, both of which say something true:

- `mapped_bytes_per_result` at most 24,576 — 96×64 at four bytes per pixel,
  exactly once. It bounds a change that started mapping the frame more than once
  per result, which no latency ceiling on a quantised zero could. What it does
  not do is separate a shared mapping from a copied one; the benchmark assertion
  described above is what covers that.
- `iteration_span_ms` at most 0.0006 (macOS) and 0.0004 (Windows) — one clock
  reading across two hundred iterations rather than two hundred readings, which
  is how a batched timing recovers a number granularity would otherwise swallow.
  Both follow the same rule as every other ceiling here, three times the value
  that target measured, rounded up: 0.000192 on `aarch64-apple-darwin` and
  0.000103 on `x86_64-pc-windows-msvc`. The committed profiles are where those
  measurements and their derivations are recorded.

The same two budgets are written into both files even though only one target has
the problem, so that the two profiles agree about what the workload is allowed
to do rather than describing it differently depending on the clock.

### The C ABI is not a material cost, and that is now measured

Task 9.2 asks for "any material C ABI startup overhead". Whether it is material
was the question, and answering it by reasoning about function calls would have
been an assertion. Each workload that has a Rust equivalent is measured twice,
in one process, one build, one run, so the difference between a pair is the
boundary rather than the conditions.

| | aarch64-apple-darwin | x86_64-pc-windows-msvc |
|---|---|---|
| `negotiate_table` | below the clock's resolution | below the clock's resolution |
| `engine_create`, C − Rust | +0.12 µs | +0.30 µs |
| `match_warm`, C − Rust | +0.25 µs (0.1%) | +9.0 µs (3.4%) |

Negotiation is what a C caller pays before it holds anything, happens once, and
does not register on either host's monotonic clock. Engine creation costs a
fixed sub-microsecond amount more through the table — a large *proportion* of a
very small number, which is why the absolute figure is the one that matters.

The warm-match row is the one worth reading carefully. The C path is four table
entries: `session_find`, `result_describe`, and one `result_match` per match,
each validating a size-versioned structure at the boundary. The Rust path is one
call and a slice read. So the cost is per crossing rather than per unit of work,
and a caller that reads many matches out of one result pays it many times. Nine
microseconds against a 267-microsecond search is not material; the same four
crossings against an operation a hundred times cheaper would be.

That is the finding, and it is a property of the boundary rather than of this
workload: **the C ABI costs a fixed amount per entry, so what makes it material
is the number of crossings, not the size of the work behind them.** A later
phase that adds a per-frame entry point should measure it rather than assume
this result transfers.

Budgets are set on the C workloads only. The Rust halves carry none, and the
profiles say why: they are the control that makes the C number interpretable,
and the Rust workflow's own ceilings are in the `deterministic-slice` profile
for the same target. A second set here would be the same claim measured twice
and free to disagree with itself.

`negotiate_table` gets the same treatment as `map_full_frame` for the same
reason — its median is zero on both targets — so it is bounded by
`iteration_span_ms` rather than by a percentile.

What this does **not** measure is dynamic loading. The benchmark links the
library, so the `LoadLibrary` or `dlopen` a real C host performs once at startup
is outside every window. That is stated in the profiles rather than left for a
reader to infer, because "C ABI startup overhead" could reasonably be read to
include it.

### Live heap, not resident memory

`peak_memory`, `steady_memory`, and `memory_growth` remain defined as resident
memory and remain unused. Phase 1 uses three new measures —
`peak_allocated_bytes`, `steady_allocated_bytes`, `allocated_growth_bytes` —
counted by a global allocator the benchmark installs. Resident memory is read
through a different platform API on each target, moves with allocator and
operating-system behaviour no change to this project can affect, and on a
workload this size is more noise than signal. The new measures are added to the
vocabulary rather than redefining the old ones, because a budget written against
one does not mean the same thing against the other.

A later phase that holds native GPU textures or loads an OCR model will need
resident memory as well: those costs are not on the heap this counts.

### One profile file per run, not per workload

[performance.md](../performance.md) said one file per workload. Eight workloads
measured together on one host share one fixture hash, one target, one machine,
one build, and one sample count, so eight files would carry eight copies of the
profile and eight chances for them to disagree. The naming rule is now
`<phase>-<workload-set>-<target>.toml`, and the target stays in the name because
that is the one thing that must never be shared across a file.

## What the measurements showed

Worth recording because the numbers, not the budgets, are the evidence:

- **Directory loading is the widest gap between the targets**: 0.187 ms at the
  95th percentile on macOS against 0.981 ms on Windows, roughly five times, for
  seven file opens and their metadata. This is why a budget is per target and
  why one target's number never satisfies the other's.
- **Memory loading is within ten percent across targets** — 0.0113 against
  0.0124 ms — because no filesystem is involved. Set beside the directory pair,
  it separates what the container costs from what the host's filesystem does,
  and says that essentially all of that five-times gap is the filesystem.
- **Archive loading sits between them** — 0.072 against 0.277 ms — and costs
  more heap than any other workload, which is expected: it inflates entries it
  has already bounded.
- **Warm matching is barely cheaper than cold** — 0.265 against 0.280 ms on
  macOS, 0.278 against 0.305 on Windows — because a 12×10 template is cheap to
  compile relative to searching a 96×64 frame. The pair will separate as
  templates grow, and it is recorded now so that the day it does, there is
  something to compare against.
- **Every workload's `allocated_growth_bytes` is zero on both targets**, across
  both benchmarks and all thirteen workloads, which is the single most useful
  line in any of the four files.

## Alternatives

**Set no budgets and exit Phase 1 with the harness only.** `G-013` blocks the
exit of each phase that introduces a workload, so this is not available. It is
also the option that costs most later: the first regression would arrive with
nothing to compare against, and the measurements to compare with would have to
be taken from a tree that already contained the regression.

**Use relative budgets against these runs as baselines**, rather than absolute
ceilings. The format supports it and it is the better long-run shape. Rejected
for now because a relative budget requires a tracked baseline for the same
target and fixture hash, and these files *are* that baseline — a baseline whose
budget is a ratio against itself says nothing. A later phase that re-measures
the same workloads should add relative budgets against these files, which is
what they exist to enable.

**Tighten the ceilings to 1.5× and treat a flap as a signal.** Rejected: these
runs are on machines that also run editors and browsers, and a ceiling that
fails for that reason teaches a reader to re-run rather than to investigate,
which is worse than a loose ceiling honestly labelled.

**Measure resident memory as `performance.md` originally specified.** Costed
above. It would also mean platform-specific code in a portable crate's benchmark
for a number that says less than the one the counting allocator produces.

**Give `map_full_frame` a latency ceiling anyway**, set from the macOS number
where the clock can resolve it. Rejected: a budget is valid only for the target
in its profile, and writing one for the target where the measurement happens to
work while omitting it where it does not would make the pair of files disagree
about what the workload is.

## Consequences

- **These numbers are now the reference, and what a crossing costs depends on
  the budget's kind.** Crossing a hard gate fails the benchmark binary, so it
  fails `cargo test --locked --workspace --all-targets` on both release targets
  and in CI. Crossing an absolute ceiling fails the comparison the operator
  performs against the committed profile after a timing run on that target's
  hardware; nothing in CI reports it, because nothing in CI runs a timing. Either
  way the crossing is investigated rather than accommodated, and changing a
  ceiling in either direction requires a re-measurement in the same change and
  an ADR.
- **The four files must be regenerated together.** Each benchmark's pair shares
  a `fixture_sha256`, and a change to any tracked fixture invalidates them. The fixture is pinned by
  `SHA256SUMS` in two directories, each enforced by a test that checks both
  directions — every listed file matches, and every file is listed.
- **Re-measuring needs the Windows host.** The macOS numbers can be regenerated
  on the development machine; the Windows ones cannot, and hosted CI is too
  variable to substitute. That is a real cost of per-target budgets and is the
  reason CI runs correctness rather than timings.
- **`G-013` stays open.** Only the Phase 1 workloads are resolved. Capture, OCR,
  watcher scheduling, and acceleration each introduce workloads that need their
  own measurements and their own record.
- **The harness is now the thing to keep honest.** It grew a global allocator, a
  batched span, three loading workloads, and a second benchmark in this change.
  The scaffolding those two share lives in `mado-pilot-testkit`, so the profile
  format has one printer rather than two that can drift apart. Its oracles run
  under `cargo test --locked --workspace --all-targets` on both targets and in
  CI, so a harness that stopped checking its output would fail a test rather
  than quietly report faster numbers.
