# macOS production-capture acceptance

## Scope and source

This record accepts only the repository fixture's production capture and transition workloads on the approved Apple Silicon host. It is bound to:

| Fact | Value |
|---|---|
| Commit | `7ba689c6496030af38ded5d3af9b9fd1d6234d29` |
| Tree | `42b80aa2318f428728b733cd68ab42e6b8863251` |
| Benchmark executable SHA-256 | `b3655f657d20157335cffdea18c32c2a4a6c74b549802afc5dea17097221c2c3` |
| Fixture source SHA-256 | `c5576c5290003c723f1d3797ab1c6032e935a9e04ab42d50ce5dc9108bc029ea` |
| Fixture executable SHA-256 | `40a33a5ceefff9ff01afef52b694799d2a1af91cc355a9d1020dda2a68891374` |
| Host | Apple M1 Pro, 10 cores, 32 GiB |
| OS / SDK | macOS 26.5.2 (`25F84`) / SDK 26.5 |
| Deployment target | macOS 26.5.2 |
| Displays | exactly two online non-mirrored displays; signed-origin 3840x2160 1x; main 2560x1440 logical / 5120x2880 backing 2x |
| Permissions / signing | Screen Recording and event-post access granted; fixture structurally ad-hoc signed |

No result transfers to another application, renderer, game, display topology, operating-system version, input stack, or anti-cheat system.

## Production transitions

Five warm-ups and fifty retained samples per workload:

| Workload | Correctness failures | p50 | p95 | max | Mapped bytes | Peak Rust heap | Growth |
|---|---:|---:|---:|---:|---:|---:|---:|
| `open_first_frame` | 0 | 97.821750 ms | 104.117541 ms | 105.085916 ms | 4,628,480 B | 4,636,342 B | 0 B |
| `resize_recreation` | 0 | 52.336333 ms | 70.281000 ms | 93.493458 ms | 0 B | 19,172 B | 0 B |
| `close_drain` | 0 | 72.071958 ms | 85.032791 ms | 98.274000 ms | 0 B | 7,638 B | 0 B |

The private resize acknowledgement is stimulus only. Acceptance requires the independently captured frame to advance epoch and geometry revision and to carry the expected extent and frame-authoritative transform. The earlier 9,856-byte result was traced to geometry-history capacity growth; after the bounded conditional reservation repair this exact row reports zero growth.

## Production capture

Twenty warm-ups and two hundred retained samples per workload:

| Workload | Correctness failures | p50 | p95 | max | Mapped bytes | Stale ratio | Peak Rust heap | Growth |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `publication_age` | 0 | 0.815167 ms | 2.487750 ms | 7.812750 ms | 4,628,480 B | 0 | 9,300,901 B | 0 B |
| `steady_frame_acquisition` | 0 | 18.107250 ms | 43.526083 ms | 61.952250 ms | 4,628,480 B | 0 | 9,300,853 B | 0 B |
| `latest_acquisition` | 0 | 0.000750 ms | 0.000875 ms | 0.003000 ms | 4,628,480 B | 0 | 9,300,789 B | 0 B |
| `cpu_map_bgra8` | 0 | 0.189417 ms | 0.453625 ms | 4.760209 ms | 4,628,480 B | n/a | 9,300,789 B | 0 B |
| `retained_pressure_resume` | 0 | 2.828166 ms | 6.448000 ms | 18.139042 ms | 0 B | 0.834299917 | 4,671,989 B | 0 B |

The retained-pressure stale ratio is the expected consequence of filling the finite retained budget, proving one publication cannot commit, releasing one slot, and observing resumed publication with a sequence gap. It is not a steady-capture loss rate.

The final source creates the retained-pressure fixture once in that workload's
untimed setup. An intermediate post-budget run started it before four unrelated
long workloads, left it idle for about three minutes, and later failed the
confirmation-session close; that run emitted no profile and contributes no
measurement. Scoping the fixture to its workload preserves one fixture across
all pressure samples while avoiding overlapping unrelated fixture lifetimes.

## Procedure

After building, assembling, signing, and strictly verifying the repository fixture as documented in [`macos-input-verification.md`](../../macos-input-verification.md), run each workload set with the exact source and approved metadata:

```sh
cargo bench --locked -p mado-pilot --bench native-phase2 -- \
  --workload-set production-transitions \
  --fixture-executable "$MADO_PILOT_MACOS_FIXTURE_EXECUTABLE" \
  --source-revision 7ba689c6496030af38ded5d3af9b9fd1d6234d29 \
  --source-tree 42b80aa2318f428728b733cd68ab42e6b8863251 \
  --toolchain "rustc 1.97.1; cargo 1.97.1; Apple clang 21.0.0; macOS SDK 26.5" \
  --gpu-driver "Apple integrated GPU; system driver stack" \
  --hardware "Apple M1 Pro, 10 cores, 32 GiB" \
  --os-version "macOS 26.5.2 (25F84)" \
  --deployment-target "macOS 26.5.2" \
  --display-topology "exactly two online non-mirrored displays; signed-origin 3840x2160 1x; main 2560x1440 logical / 5120x2880 backing 2x; mixed-scale connection method unrestricted" \
  --permissions-signing "Screen Recording granted; event-post access granted; target bundle structurally ad-hoc signed"

# Repeat with --workload-set production-capture and identical metadata.
```

The tracked profiles retain the complete output and ADR 0030 budgets. Profile-drift tests bind their benchmark keys, hard predicates, and latency budgets to the harness.

## Privacy review

This record contains approved source and executable digests, aggregate timings, correctness/resource counts, permission/signing classifications, and non-sensitive host/topology metadata. It contains no captured pixels or hashes, recognized or injected text, credentials, PIDs, window titles, raw display/window identifiers, user paths, or unrelated desktop data.
