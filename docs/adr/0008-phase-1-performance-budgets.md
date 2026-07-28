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

Four things the harness did not measure before this change, and now does:
loading the same package from memory and from an archive as well as from a
directory, `mapped_bytes_per_result`, live-heap peak and growth, and one
batched span per workload.

## Decision

### The eight Phase 1 workloads have budgets, in two committed profiles

[benchmarks/phase-1-deterministic-slice-aarch64-apple-darwin.toml](benchmarks/phase-1-deterministic-slice-aarch64-apple-darwin.toml)
and
[benchmarks/phase-1-deterministic-slice-x86_64-pc-windows-msvc.toml](benchmarks/phase-1-deterministic-slice-x86_64-pc-windows-msvc.toml).
Each holds one profile, eight measurements, three budgets that apply to the
whole file, and twelve workload-specific budgets.

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
  measured **zero** on all eight workloads.

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
  exactly once. This is the measure that would catch a change from sharing to
  copying, which no latency ceiling on a quantised zero ever could.
- `iteration_span_ms` at most 0.00025 (macOS) and 0.0004 (Windows) — one clock
  reading across two hundred iterations rather than two hundred readings, which
  is how a batched timing recovers a number granularity would otherwise swallow.

The same two budgets are written into both files even though only one target has
the problem, so that the two profiles agree about what the workload is allowed
to do rather than describing it differently depending on the clock.

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
  95th percentile on macOS against 1.029 ms on Windows, roughly five times, for
  seven file opens and their metadata. This is why a budget is per target and
  why one target's number never satisfies the other's.
- **Memory loading is within two percent across targets** — 0.0113 against
  0.0121 ms — because no filesystem is involved. The pair is the clearest
  available statement of what the container costs as opposed to what the host's
  filesystem does.
- **Archive loading sits between them** and costs more heap than any other
  workload, which is expected: it inflates entries it has already bounded.
- **Warm matching is not much cheaper than cold** — 0.278 against 0.284 ms on
  macOS — because a 12×10 template is cheap to compile relative to searching a
  96×64 frame. The pair will separate as templates grow, and it is recorded now
  so that the day it does, there is something to compare against.
- **Every workload's `allocated_growth_bytes` is zero on both targets**, which
  is the single most useful line in either file.

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

- **These numbers are now the reference.** A change that crosses one fails the
  phase's performance gate and is investigated rather than accommodated;
  changing a ceiling in either direction requires a re-measurement in the same
  change and an ADR.
- **Both files must be regenerated together.** They share a `fixture_sha256`,
  and a change to any tracked fixture invalidates both. The fixture is pinned by
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
  batched span, and three loading workloads in this change. Its oracles run
  under `cargo test --locked --workspace --all-targets` on both targets and in
  CI, so a harness that stopped checking its output would fail a test rather
  than quietly report faster numbers.
